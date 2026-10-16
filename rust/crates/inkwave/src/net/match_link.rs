//! Task 24 — the app-side match link: session events → a live [`Match`],
//! the replication controller driving it, and the "starting → load, results
//! → back to lobby" seams (JS `main.js` `startNetMatch` L770-807,
//! `netMatchGo` L816-821, `netMatchEnd` L823-842, `netMatchAborted` L844-846
//! and `session.js` `_begin`/`_go`/`_launch`/`endMatch`).
//!
//! Three resources keep the Bevy borrows disjoint (plan risk R1):
//! [`NetMatchState`] (the replication controller + match bookkeeping),
//! [`NetEvents`] (session events staged by `ui_state_machine`, consumed
//! here) and the existing [`OnlineSession`] / [`DemoSim`]. `net_tick` runs
//! after `ui_state_machine` (which pumps the session) and before
//! `ink_upload` (which only replays the paint stream onto the atlas while a
//! net match is live).
//!
//! Declared deviations (Task 24 plan):
//!  - ③ JS builds the match in `init` and enters `intro` on `go`; the Rust
//!    builder enters `intro` at construction and `net_tick` does not step
//!    until `go` (`go_pending`), then re-enters `intro` on every client like
//!    JS `netMatchGo` → `m.start()` (main.js L816-821). Guests additionally
//!    re-enter on the host's `{k:'st'}` through the replication layer.
//!  - ④ bot brains always use `tuning.difficulty.easy` (the Tuning table
//!    ships one difficulty in M1).
//!  - ⑤ the match seed is `fnv1a(cfg.id)` (JS uses `Math.random()`; the Rust
//!    sim's determinism contract wants a stable seed).

use bevy::prelude::*;
use inkwave_sim::actor::{ActorInput, FIXED_DT, SimWorld};
use inkwave_sim::bot::Bot;
use inkwave_sim::match_::{Match, MatchEvent};
use inkwave_sim::net::matchbuild::{build_net_match, fnv1a, local_roster_index};
use inkwave_sim::net::replication::{Replication, ReplicationEvent};
use inkwave_sim::net::session::{MatchPhase, SessionEvent, StartCfg, map_no_bots};

use crate::ink_render::DemoSim;
use crate::input::PlayerControls;
use crate::net::OnlineSession;
use crate::nettest::NetTest;
use crate::ui::online::room_gone_toast;
use crate::ui::{Screen, UiState};

/// Session events staged by `ui_state_machine` for `net_tick` to consume
/// (the pump happens there so the connect `settle` and screen sync stay in
/// the same frame; `poll_events` is drained exactly once — plan risk R1).
#[derive(Resource, Default)]
pub struct NetEvents(pub Vec<SessionEvent>);

/// The live net match (JS `G.netm` + `G.match` while `G.mode === 'match'`).
#[derive(Resource, Default)]
pub struct NetMatchState {
    /// The replication controller for the current match (`G.netm`, JS L308).
    pub rep: Option<Replication>,
    /// Roster index of the local player's actor (JS `m.local`), the slot
    /// `net_tick` writes the keyboard/mouse intent into.
    pub local_idx: usize,
    /// The `{k:'start',…}` config of the current match (`this._startCfg`).
    pub cfg: Option<StartCfg>,
    /// Host: `{k:'res'}` sent once after the judge (JS match.js L258/271).
    pub result_sent: bool,
    /// Host: `{k:'end'}` + `endMatch` already fired (main.js L383/L972).
    pub end_sent: bool,
    /// Match built, waiting for the `go` handshake (JS: state `init`).
    pub go_pending: bool,
    /// Match seed (deviation ⑤: `fnv1a(cfg.id)`).
    pub seed: u64,
    /// Per-actor inputs for the masked step (parallel to `m.actors`).
    inputs: Vec<ActorInput>,
    /// Fixed-step accumulator for the net match.
    acc: f32,
    /// Host countdown on the results screen (JS `_netEndT`, main.js L972:
    /// 12 s, then `sendEnd` + `netMatchEnd`). `0.0` = not armed.
    end_timer: f64,
    /// A `Host` migration event was seen and the next `Leave` carries the
    /// re-election flag (JS `hostChanged`, session.js L156-163: `onLeave`
    /// runs with the flag, the `host` emit follows).
    host_changed: bool,
}

// Task 25: the builder trio (`build_net_match` / `local_roster_index` /
// `fnv1a`) moved verbatim to `inkwave_sim::net::matchbuild` so the
// test-only net-harness assembles the exact same match from the same code.

/// `Match{Start}` handler — JS `startNetMatch` (main.js L770-807) minus the
/// render seams (the Rust app keeps the world; the actors swap via
/// `sync_actor_count`), plus the `NetMatch` constructor (session.js L308) and
/// the post-load `ready` report (`_begin` L316-317 → `mark_loaded`).
fn start_net_match(
    os: &mut OnlineSession,
    nms: &mut NetMatchState,
    demo: &mut DemoSim,
    st: &mut UiState,
    autopilot: bool,
) {
    let (Some(cfg), Some(my_id), Some(host_id)) = (
        os.sess.start_cfg().cloned(),
        os.sess.my_id().map(str::to_string),
        os.sess.host_id().map(str::to_string),
    ) else {
        return;
    };
    let seed = fnv1a(&cfg.id);
    let sim = SimWorld::new(&demo.world, &demo.layout);
    let mut built = build_net_match(
        &cfg,
        seed,
        &sim,
        &demo.tuning.player,
        &demo.tuning.match_config,
    );
    let mut rep = Replication::new(&cfg, &my_id, &host_id);
    rep.bind_match(&mut built);
    // JS match.js L112: every bot roster entry gets a brain. On the host the
    // bot slots are owned (non-remote) so `attach_bot` takes; on a guest they
    // are remote and it is a no-op — exactly like JS (bots run host-side).
    // Deviation ④: the one Tuning difficulty.
    for (i, r) in cfg.roster.iter().enumerate() {
        if r.bot {
            rep.attach_bot(i, &built.actors[i], &demo.tuning.difficulty.easy, seed);
        }
    }
    // JS netmatch.js L79: humans-only stage — owners who left while the
    // match was loading (no NetMatch yet to hear the leave frame) drop now.
    // `s._members` is the relay-authoritative set (session.js L31/L100-101).
    let members: Vec<String> = os.sess.member_ids();
    rep.prune_loading_leavers(&mut built, &members);
    // JS `match.js` L116 resolves `m.local` over the *final* actor array —
    // i.e. `cfg.roster` minus the entries prune actually dropped. The drop
    // is gated on the humans-only stage (netmatch.js L79 / replication.rs
    // `prune_loading_leavers`), so the enumeration must use the same gate:
    // on a bots stage nobody is dropped and no owner may be skipped.
    let no_bots = map_no_bots(&cfg.map);
    nms.local_idx = local_roster_index(&cfg.roster, &my_id, &members, no_bots);
    // Task 26 test seam: `?autopilot` ≡ JS `match.js` L112
    // `if (mine && (r.bot || o.autopilot))` — the local human slot gets a
    // brain too (bots already have one from the loop above; `attach_bot`
    // no-ops on remote slots, replication.rs L318-321, so a guest's own
    // owned local slot is the only one this can take).
    if autopilot {
        rep.attach_bot(
            nms.local_idx,
            &built.actors[nms.local_idx],
            &demo.tuning.difficulty.easy,
            seed,
        );
    }
    demo.m = built;
    nms.cfg = Some(cfg);
    nms.seed = seed;
    nms.inputs = vec![ActorInput::default(); demo.m.actors.len()];
    nms.acc = 0.0;
    nms.go_pending = true;
    nms.result_sent = false;
    nms.end_sent = false;
    nms.end_timer = 0.0;
    nms.host_changed = false;
    nms.rep = Some(rep);
    // Camera + local-player wiring follow the net slot (JS `rig.follow(m.local)`).
    demo.player_slot = nms.local_idx;
    // A fresh match replaced the paint grid: the grid-version rollback in
    // the atlas drain wipes the bitmap for the new match (as `replay` does).
    st.net_back_pressed = false;
    os.sess.mark_loaded();
}

/// Tear the net match down and restore a fresh offline demo roster (4v4) so
/// leaving online mode returns the app to its M1 baseline. JS `netMatchEnd`
/// (main.js L823-842): dispose the NetMatch, `endMatch`, back to the lobby.
fn teardown_net_match(nms: &mut NetMatchState, demo: &mut DemoSim, st: &mut UiState) {
    if nms.rep.is_none() && nms.cfg.is_none() && !nms.go_pending {
        return;
    }
    nms.rep = None;
    nms.cfg = None;
    nms.go_pending = false;
    nms.result_sent = false;
    nms.end_sent = false;
    nms.end_timer = 0.0;
    nms.host_changed = false;
    nms.local_idx = 0;
    restore_offline_demo(demo);
    if matches!(st.screen, Screen::Hud | Screen::Results | Screen::Pause) {
        st.screen = Screen::OnlineLobby;
    }
}

/// Rebuild the offline 4v4 demo (the roster `DemoSim::new` would have built).
fn restore_offline_demo(demo: &mut DemoSim) {
    let sim = SimWorld::new(&demo.world, &demo.layout);
    let seed = demo.m.seed;
    let m = Match::new(
        90.0,
        seed,
        &sim,
        &demo.tuning.player,
        &demo.tuning.match_config,
    );
    demo.bots = m
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| Bot::new(a, &demo.tuning.difficulty.easy, seed, i))
        .collect();
    demo.mate_goals = vec![None; demo.bots.len()];
    demo.inputs = vec![ActorInput::default(); m.actors.len()];
    demo.acc = 0.0;
    demo.splats = 0;
    demo.player_slot = 0;
    demo.m = m;
}

/// The net-match pump (plan Step 4): dispatch the session events staged by
/// `ui_state_machine`, run the replication substeps, and fire the host's
/// result / end sends.
#[allow(clippy::too_many_arguments)]
pub fn net_tick(
    time: Res<Time>,
    mut os: NonSendMut<OnlineSession>,
    mut nms: ResMut<NetMatchState>,
    mut demo: ResMut<DemoSim>,
    mut net_ev: ResMut<NetEvents>,
    mut st: ResMut<UiState>,
    pc: Res<PlayerControls>,
    nettest: Res<NetTest>,
) {
    // Task 26 test seam: with `?autopilot` the local slot is bot-driven, so
    // no player controller writes into it (≡ match.js L118 `!o.autopilot`).
    let autopilot = nettest.cfg.as_ref().is_some_and(|c| c.autopilot);
    let os = &mut *os;
    let nms = &mut *nms;
    let demo = &mut *demo;
    let events = std::mem::take(&mut net_ev.0);
    if !st.net_mode {
        // Pause → MAIN MENU mid-match leaves the room and drops `net_mode`
        // (JS disposes the NetMatch on the game switch, main.js L823-842):
        // tear the orphan down now instead of pumping a net match behind the
        // offline menu. `teardown_net_match` no-ops when nothing is live.
        teardown_net_match(nms, demo, &mut st);
        return; // fully offline: nothing to pump
    }
    let dt = f64::from(time.delta_secs().min(0.1));

    // ---- 1. session events (JS `_emit` subscriptions the app wires up) ----
    for ev in events {
        match ev {
            SessionEvent::Match { phase } => match phase {
                MatchPhase::Start => start_net_match(os, nms, demo, &mut st, autopilot),
                MatchPhase::Go => {
                    // JS `_launch` → `netMatchGo` (main.js L816-821): every
                    // client starts its match at the host's word. Deviation ③:
                    // the Rust match was built in `intro`, so this re-enters
                    // `intro` (state_t reset) instead of leaving `init`; the
                    // host's `{k:'st'}` broadcast re-aligns the guests.
                    if nms.rep.is_some() {
                        nms.go_pending = false;
                        demo.m.set_phase(inkwave_sim::match_::Phase::Intro);
                        nms.acc = 0.0;
                    }
                }
                MatchPhase::End => teardown_net_match(nms, demo, &mut st),
                MatchPhase::Aborted => {
                    // JS `netMatchAborted` (main.js L844-846): back to the
                    // menus with the reason as a toast. The Rust hub keeps
                    // online mode (the room is gone; the hub is the door).
                    let reason = os.sess.error().unwrap_or("").to_string();
                    teardown_net_match(nms, demo, &mut st);
                    if !reason.is_empty() {
                        os.flags.banner = Some(room_gone_toast(&reason));
                    }
                    st.screen = Screen::OnlineHub;
                }
            },
            SessionEvent::Host { host_id } => {
                // session.js L156-157: the re-elected host lands *before*
                // `onLeave` runs (replication.rs L200-202 contract).
                if let Some(rep) = &mut nms.rep {
                    rep.set_host(&host_id);
                    nms.host_changed = true;
                }
            }
            SessionEvent::Leave { player, .. } => {
                let before = demo.m.actors.len();
                let mut shifted: Option<usize> = None;
                if let Some(rep) = &mut nms.rep {
                    let sim = SimWorld::new(&demo.world, &demo.layout);
                    rep.on_leave(
                        &player.id,
                        nms.host_changed,
                        &mut demo.m,
                        &demo.tuning.difficulty.easy,
                        nms.seed,
                        &sim,
                        &demo.tuning.player,
                    );
                    // The humans-only stage dropped the leaver's actor
                    // (`remove_at` inside `on_leave`): JS keeps `m.local` as
                    // an object ref (match.js L116), the Rust port tracks a
                    // slot — re-resolve it against the post-leave owners.
                    if demo.m.actors.len() != before {
                        shifted = os.sess.my_id().and_then(|my| rep.local_slot(my));
                    }
                }
                if let Some(i) = shifted {
                    nms.local_idx = i;
                    demo.player_slot = i;
                }
                if nms.inputs.len() != demo.m.actors.len() {
                    nms.inputs
                        .resize(demo.m.actors.len(), ActorInput::default());
                }
                nms.host_changed = false;
            }
            SessionEvent::GameMessage { from, payload } => {
                let end = if let Some(rep) = &mut nms.rep {
                    rep.on_message(&from, &payload, &mut demo.m, &demo.tuning.player)
                } else {
                    None
                };
                if end == Some(ReplicationEvent::MatchEnd) {
                    // JS netmatch.js L218 `case 'end'` → `netMatchEnd()`:
                    // the host already sent `{k:'end'}`; leave the match.
                    if let (Some(rep), Some(tr)) = (&mut nms.rep, os.sess.transport_mut()) {
                        rep.send_end(tr); // host-only no-op for guests
                    }
                    os.sess.end_match();
                    teardown_net_match(nms, demo, &mut st);
                }
            }
            // `State` / `Lobby` / `Error` / `Join` / `Emote` are consumed by
            // the UI layer (`ui_state_machine` settle + `project` facts).
            _ => {}
        }
    }

    // ---- 2. fixed-step replication substeps (plan Step 4.3) ----
    if let Some(mut rep) = nms.rep.take() {
        if !nms.go_pending {
            let sim = SimWorld::new(&demo.world, &demo.layout);
            let t = &demo.tuning;
            // JS main.js L872: an online match keeps running under the pause
            // menu — `pc.paused` does NOT gate the net step.
            nms.acc += dt as f32;
            let mut steps = 0;
            while nms.acc >= FIXED_DT && steps < 4 {
                nms.acc -= FIXED_DT;
                steps += 1;
                // Bot brains (owned + adopted actors, match.js L222-224).
                rep.drive_bots(
                    &mut demo.m,
                    &demo.nav,
                    &sim,
                    &t.player,
                    &t.spritzer,
                    &mut nms.inputs,
                );
                // Local player input into `inputs[local_idx]` (JS
                // PlayerController; zeroed outside `playing`, match.js
                // L220-226).
                if !autopilot {
                    if demo.m.phase == inkwave_sim::match_::Phase::Active {
                        if let Some(inp) = nms.inputs.get_mut(nms.local_idx) {
                            *inp = pc.intent;
                        }
                        if let Some(a) = demo.m.actors.get_mut(nms.local_idx) {
                            a.aim_yaw = pc.yaw;
                            a.aim_pitch = pc.pitch;
                        }
                    } else if let Some(inp) = nms.inputs.get_mut(nms.local_idx) {
                        *inp = ActorInput::default();
                    }
                }
                demo.m.step_masked(
                    FIXED_DT,
                    &nms.inputs,
                    Some(rep.remote_mask()),
                    &sim,
                    &t.player,
                    &t.spritzer,
                );
                let Some(tr) = os.sess.transport_mut() else {
                    break; // link gone; the session will emit `Aborted`
                };
                let rest: Vec<MatchEvent> =
                    rep.update(FIXED_DT, &mut demo.m, &sim, &t.player, &t.spritzer, tr);
                // Un-consumed bus events (splat / turf / countdown / …) feed
                // the HUD + audio drains like the offline match's own bus.
                demo.m.events.extend(rest);
                // Host settles the match (JS match.js L258/271 `sendResult`
                // inside `_judge`, right before `setState('judge')`).
                if rep.is_host() && demo.m.result.is_some() && !nms.result_sent {
                    if let Some(tr) = os.sess.transport_mut() {
                        rep.send_result(&demo.m, tr);
                    }
                    nms.result_sent = true;
                }
            }
        }
        // ---- 3. host results-dwell: 12 s auto-end + BACK TO LOBBY ----
        // JS main.js L972 (`_netEndT`) and L383 (`netBackToLobby`).
        if rep.is_host() && !nms.end_sent && st.screen == Screen::Results {
            if nms.end_timer == 0.0 {
                nms.end_timer = 12.0;
            }
            nms.end_timer -= dt;
            if nms.end_timer <= 0.0 {
                if let Some(tr) = os.sess.transport_mut() {
                    rep.send_end(tr);
                }
                nms.end_sent = true;
                os.sess.end_match();
            }
        }
        if st.net_back_pressed {
            // JS `netBackToLobby` (main.js L383): host on results → clear the
            // timer, `sendEnd()`, `netMatchEnd()` now. Guests stay (their
            // button is a local display; they leave on the host's `{k:'end'}`).
            st.net_back_pressed = false;
            if rep.is_host() && st.screen == Screen::OnlineLobby && !nms.end_sent {
                if let Some(tr) = os.sess.transport_mut() {
                    rep.send_end(tr);
                }
                nms.end_sent = true;
                os.sess.end_match();
            }
        }
        nms.rep = Some(rep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::online::{OnlineFlags, SessionCmd, TeamSel, code_type, project, run_cmd};
    use inkwave_sim::collision::CollisionWorld;
    use inkwave_sim::match_::{Phase, SPAWN_RING_R};
    use inkwave_sim::net::session::{NetSession, Profile, RosterEntry, SessionState};
    use inkwave_sim::net::transport::{FakeRelay, FakeTransport};

    type Sess = NetSession<FakeTransport, Box<dyn FnMut() -> FakeTransport>>;

    /// The session.rs test-helper pattern (L1951-1984): shared virtual clock.
    fn sess(relay: &FakeRelay, seed: u64) -> Sess {
        let r = relay.clone();
        NetSession::new(
            Profile::default(),
            seed,
            Box::new(move || r.transport(None, seed)),
        )
    }

    fn step(s: &mut Sess, dt_ms: f64) {
        if let Some(tr) = s.transport_mut() {
            tr.advance(dt_ms);
        }
        s.update(dt_ms / 1000.0);
    }

    fn step2(a: &mut Sess, b: &mut Sess, dt_ms: f64) {
        let mut moved = false;
        if let Some(tr) = a.transport_mut() {
            tr.advance(dt_ms);
            moved = true;
        }
        if let Some(tr) = b.transport_mut() {
            tr.advance(if moved { 0.0 } else { dt_ms });
        }
        a.update(dt_ms / 1000.0);
        b.update(dt_ms / 1000.0);
    }

    fn roster_entry(nid: u8, owner: &str, bot: bool, team: u8, slot: u8) -> RosterEntry {
        RosterEntry {
            nid,
            owner: owner.into(),
            bot,
            team,
            slot,
            name: format!("p{nid}"),
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
        }
    }

    fn cfg_with(roster: Vec<RosterEntry>, host: &str) -> StartCfg {
        StartCfg {
            roster,
            map: "tidewater".into(),
            time: "day".into(),
            duration: 90,
            difficulty: "normal".into(),
            palette: 0,
            mode: "turf".into(),
            host: host.into(),
            id: "m-42".into(),
        }
    }

    // -- TR-24.1: the roster-order build contract -----------------------------

    #[test]
    fn tr24_build_net_match_roster_order() {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let sim = SimWorld::new(&demo.world, &demo.layout);
        let cfg = cfg_with(
            vec![
                roster_entry(0, "AAAA", false, 0, 0),
                roster_entry(1, "BBBB", false, 1, 0),
                roster_entry(2, "AAAA", true, 0, 1),
                roster_entry(3, "BBBB", true, 1, 1),
            ],
            "AAAA",
        );
        let m = build_net_match(
            &cfg,
            42,
            &sim,
            &demo.tuning.player,
            &demo.tuning.match_config,
        );
        // One actor per roster entry, in roster order (the parallel-array
        // contract of the replication layer, match.js `_setupRoster` L107).
        assert_eq!(m.actors.len(), 4);
        assert_eq!(m.runners.len(), 4);
        for (i, r) in cfg.roster.iter().enumerate() {
            assert_eq!(m.actors[i].team, r.team as usize, "actor {i} team");
            assert_eq!(m.actors[i].slot, r.slot as usize, "actor {i} slot");
            assert_eq!(m.actors[i].invuln, 0.0, "intro is the protection");
        }
        // `place_on_pad` (match.js L85-91): ring placement facing the arena.
        for (i, r) in cfg.roster.iter().enumerate() {
            let pad = sim.spawn_pads[r.team as usize];
            let d = (m.actors[i].pos - pad).length();
            assert!(
                (d - SPAWN_RING_R).abs() < 1e-3,
                "actor {i} sits on the pad ring: {d}"
            );
            let yaw = if r.team == 0 {
                0.0
            } else {
                std::f32::consts::PI
            };
            assert!((m.actors[i].yaw - yaw).abs() < 1e-6);
        }
        // `duration: max(1)` guard (the sim clock refuses a zero-length match).
        assert_eq!(m.duration, 90.0);
        let mut short = cfg.clone();
        short.duration = 0;
        let m0 = build_net_match(
            &short,
            42,
            &sim,
            &demo.tuning.player,
            &demo.tuning.match_config,
        );
        assert_eq!(m0.duration, 1.0);
    }

    // -- TR-24.2: two-client lobby → match → results → lobby flow --------------

    #[test]
    fn tr24_lobby_flow_two_clients() {
        let relay = FakeRelay::new();
        let mut h = sess(&relay, 11);
        let mut hf = OnlineFlags::default();
        run_cmd(&mut h, &mut hf, SessionCmd::Create);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        let code = h.code().expect("code").to_string();
        let mut g = sess(&relay, 22);
        let mut gf = OnlineFlags::default();
        for c in code.chars() {
            assert!(code_type(c, &mut gf.code_draft, &mut gf.caret).is_none());
        }
        run_cmd(&mut g, &mut gf, SessionCmd::Join);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Lobby
                && g.lobby().players.len() == 2
                && h.lobby().players.len() == 2
            {
                break;
            }
        }
        assert_eq!(h.lobby().players.len(), 2);

        // Team switch + ready-up through the pure command layer.
        run_cmd(&mut g, &mut gf, SessionCmd::SetTeam(TeamSel::One));
        run_cmd(&mut g, &mut gf, SessionCmd::ToggleReady);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.lobby().players.iter().any(|p| !p.you && p.ready) {
                break;
            }
        }
        let facts = crate::ui::online::OnlineFacts {
            state: h.state(),
            flags: &hf,
            is_host: true,
            code: Some(&code),
            lobby: h.lobby(),
            can_start: h.can_start(),
            start_block: h.start_block(),
            error: None,
        };
        assert!(project(&facts).lobby.start_enabled);

        // START ⇒ both clients get `Match{Start}` with the same cfg.
        run_cmd(&mut h, &mut hf, SessionCmd::Start);
        let mut got_start = [false; 2];
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            for (i, s) in [&mut h, &mut g].iter_mut().enumerate() {
                for ev in s.poll_events() {
                    if matches!(
                        ev,
                        SessionEvent::Match {
                            phase: MatchPhase::Start
                        }
                    ) {
                        got_start[i] = true;
                    }
                }
            }
            if got_start == [true; 2] {
                break;
            }
        }
        assert_eq!(got_start, [true; 2], "both sides see the start");
        let cfg = h.start_cfg().expect("host cfg").clone();
        assert_eq!(g.start_cfg(), Some(&cfg), "the guest got the same cfg");

        // Build both sides exactly like `start_net_match` does (deviation ③:
        // no stepping until `go`).
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let sim = SimWorld::new(&demo.world, &demo.layout);
        let (mut hm, mut hrep, hli) = build_side(&h, &cfg, &demo, &sim);
        let (mut gm, mut grep, gli) = build_side(&g, &cfg, &demo, &sim);
        assert_eq!(hm.actors.len(), gm.actors.len());
        assert_eq!(hm.actors.len(), cfg.roster.len(), "2 humans + 6 bots");
        assert_ne!(hli, gli, "the two humans hold different slots");
        // Host owns its human + all bots; the guest owns only its own.
        assert_eq!(hrep.remote_mask().iter().filter(|b| **b).count(), 1);
        assert_eq!(
            grep.remote_mask().iter().filter(|b| **b).count(),
            cfg.roster.len() - 1
        );

        // mark_loaded ⇒ the ready handshake ⇒ `Match{Go}` on both.
        h.mark_loaded();
        g.mark_loaded();
        let mut got_go = [false; 2];
        for _ in 0..200 {
            step2(&mut h, &mut g, 1.0);
            for (i, s) in [&mut h, &mut g].iter_mut().enumerate() {
                for ev in s.poll_events() {
                    if matches!(
                        ev,
                        SessionEvent::Match {
                            phase: MatchPhase::Go
                        }
                    ) {
                        got_go[i] = true;
                    }
                }
            }
            if got_go == [true; 2] {
                break;
            }
        }
        assert_eq!(got_go, [true; 2], "the go handshake completes");
        // JS `netMatchGo` → `m.start()` (main.js L816-821): both re-enter.
        hm.set_phase(Phase::Intro);
        gm.set_phase(Phase::Intro);

        // N fixed substeps: host-authoritative sim + guest replication.
        let tp = &demo.tuning.player;
        let sp = &demo.tuning.spritzer;
        let mut hinputs = vec![ActorInput::default(); hm.actors.len()];
        let mut ginputs = vec![ActorInput::default(); gm.actors.len()];
        // INTRO_TIME is 4.2 s (match_.rs L70): 300 substeps clear it.
        for _ in 0..300 {
            hrep.drive_bots(&mut hm, &demo.nav, &sim, tp, sp, &mut hinputs);
            hm.step_masked(FIXED_DT, &hinputs, Some(hrep.remote_mask()), &sim, tp, sp);
            let rest = hrep.update(FIXED_DT, &mut hm, &sim, tp, sp, h.transport_mut().unwrap());
            hm.events.extend(rest);
            grep.drive_bots(&mut gm, &demo.nav, &sim, tp, sp, &mut ginputs);
            gm.step_masked(FIXED_DT, &ginputs, Some(grep.remote_mask()), &sim, tp, sp);
            let rest = grep.update(FIXED_DT, &mut gm, &sim, tp, sp, g.transport_mut().unwrap());
            gm.events.extend(rest);
            step2(&mut h, &mut g, FIXED_DT as f64 * 1000.0);
            for ev in h.poll_events() {
                if let SessionEvent::GameMessage { from, payload } = ev {
                    hrep.on_message(&from, &payload, &mut hm, tp);
                }
            }
            for ev in g.poll_events() {
                if let SessionEvent::GameMessage { from, payload } = ev {
                    grep.on_message(&from, &payload, &mut gm, tp);
                }
            }
        }
        // The host's state broadcast drove the guest through the intro too.
        assert_eq!(hm.phase, Phase::Active, "host ran the intro clock");
        assert_eq!(gm.phase, Phase::Active, "guest followed the host's state");
        // The idle host human is replicated onto the guest's copy.
        let d = hm.actors[hli].pos.distance(gm.actors[hli].pos);
        assert!(d < 0.5, "guest tracks the host-owned actor: {d}");

        // Host endgame: force the finish window, the judge settles, the
        // result rides the replication bus to the guest (JS match.js L258/271).
        hm.phase = Phase::Finish;
        hm.state_t = 1000.0;
        hm.step_masked(FIXED_DT, &hinputs, Some(hrep.remote_mask()), &sim, tp, sp);
        assert!(hm.result.is_some(), "the host judged the match");
        hrep.send_result(&hm, h.transport_mut().unwrap());
        let mut guest_result = false;
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            for ev in g.poll_events() {
                if let SessionEvent::GameMessage { from, payload } = ev {
                    grep.on_message(&from, &payload, &mut gm, tp);
                    if gm.result.is_some() {
                        guest_result = true;
                    }
                }
            }
            if guest_result {
                break;
            }
        }
        assert!(guest_result, "the guest got the host's result");
        let hr = hm.result.expect("host result");
        let gr = gm.result.expect("guest result");
        assert_eq!(gr.coverage, hr.coverage);
        assert_eq!(gr.winner, hr.winner);

        // `{k:'end'}` ⇒ both rooms return to the lobby (JS netmatch L218 +
        // `netMatchEnd` main.js L823-842).
        hrep.send_end(h.transport_mut().unwrap());
        h.end_match();
        let mut back = false;
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            for ev in g.poll_events() {
                let SessionEvent::GameMessage { from, payload } = ev else {
                    continue;
                };
                if grep.on_message(&from, &payload, &mut gm, tp) == Some(ReplicationEvent::MatchEnd)
                {
                    g.end_match();
                    back = true;
                }
            }
            if back {
                break;
            }
        }
        assert!(back, "the guest saw the host's end");
        assert_eq!(h.state(), SessionState::Lobby);
        assert_eq!(g.state(), SessionState::Lobby);
        assert_eq!(h.lobby().players.len(), 2, "the room survives the match");
    }

    /// The `start_net_match` build sequence (JS `startNetMatch` L770-807 +
    /// the `NetMatch` ctor, session.js L308) on plain data, for the flow test.
    fn build_side(
        s: &Sess,
        cfg: &StartCfg,
        demo: &DemoSim,
        sim: &SimWorld,
    ) -> (Match, Replication, usize) {
        let my = s.my_id().expect("my_id").to_string();
        let host = s.host_id().expect("host_id").to_string();
        let seed = fnv1a(&cfg.id);
        let mut m = build_net_match(
            cfg,
            seed,
            sim,
            &demo.tuning.player,
            &demo.tuning.match_config,
        );
        let mut rep = Replication::new(cfg, &my, &host);
        rep.bind_match(&mut m);
        for (i, r) in cfg.roster.iter().enumerate() {
            if r.bot {
                rep.attach_bot(i, &m.actors[i], &demo.tuning.difficulty.easy, seed);
            }
        }
        let members: Vec<String> = s.member_ids();
        rep.prune_loading_leavers(&mut m, &members);
        let local_idx = local_roster_index(&cfg.roster, &my, &members, map_no_bots(&cfg.map));
        (m, rep, local_idx)
    }

    // -- Review M-1: the local slot follows prune's gate ----------------------

    #[test]
    fn tr24_local_roster_index_follows_prune_gate() {
        // Roster: an absent human "CCCC" first, then my "BBBB", then a bot.
        let roster = vec![
            roster_entry(0, "CCCC", false, 0, 0),
            roster_entry(1, "BBBB", false, 1, 0),
            roster_entry(2, "BBBB", true, 1, 1),
        ];
        let members = vec!["BBBB".to_string()];
        // Bots stage: `prune_loading_leavers` keeps CCCC's actor (the drop
        // is gated on the humans-only stage, netmatch.js L79), so the final
        // array keeps the offset — local is index 1, not 0.
        assert_eq!(local_roster_index(&roster, "BBBB", &members, false), 1);
        // Humans-only stage: prune drops CCCC, the array shifts — local 0.
        assert_eq!(local_roster_index(&roster, "BBBB", &members, true), 0);
        // A bot entry is never the local player even when owned (L116).
        let roster2 = vec![
            roster_entry(0, "BBBB", true, 0, 0),
            roster_entry(1, "BBBB", false, 0, 1),
        ];
        assert_eq!(local_roster_index(&roster2, "BBBB", &members, false), 1);
        // Own actor is never pruned even when absent from members (the host
        // itself is always a member, but the `r.owner == my` arm guards it).
        assert_eq!(local_roster_index(&roster, "CCCC", &[], true), 0);
    }

    // -- Review m-8: `local_slot` re-resolves after a leave removal -----------

    #[test]
    fn tr24_local_slot_follows_leave_removal() {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let sim = SimWorld::new(&demo.world, &demo.layout);
        let mut cfg = cfg_with(
            vec![
                roster_entry(0, "CCCC", false, 0, 0),
                roster_entry(1, "BBBB", false, 1, 0),
            ],
            "CCCC",
        );
        cfg.map = "cargo".into(); // humans-only stage: `on_leave` prunes
        let mut m = build_net_match(
            &cfg,
            1,
            &sim,
            &demo.tuning.player,
            &demo.tuning.match_config,
        );
        let mut rep = Replication::new(&cfg, "BBBB", "CCCC");
        rep.bind_match(&mut m);
        assert_eq!(rep.local_slot("BBBB"), Some(1));
        rep.on_leave(
            "CCCC",
            false,
            &mut m,
            &demo.tuning.difficulty.easy,
            1,
            &sim,
            &demo.tuning.player,
        );
        assert_eq!(m.actors.len(), 1, "the leaving owner's actor is dropped");
        assert_eq!(rep.local_slot("BBBB"), Some(0), "the slot shifts");
        assert_eq!(rep.local_slot("ZZZZ"), None, "absent ids resolve to None");
    }

    // -- TR-24.2: teardown restores the offline baseline -----------------------

    #[test]
    fn tr24_match_end_returns_lobby() {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let sim = SimWorld::new(&demo.world, &demo.layout);
        let cfg = cfg_with(
            vec![
                roster_entry(0, "AAAA", false, 0, 0),
                roster_entry(1, "BBBB", false, 1, 0),
            ],
            "AAAA",
        );
        let mut m = build_net_match(
            &cfg,
            1,
            &sim,
            &demo.tuning.player,
            &demo.tuning.match_config,
        );
        let rep = Replication::new(&cfg, "AAAA", "AAAA");
        rep.bind_match(&mut m);
        let mut nms = NetMatchState {
            rep: Some(rep),
            cfg: Some(cfg),
            seed: 1,
            inputs: vec![ActorInput::default(); m.actors.len()],
            ..Default::default()
        };
        demo.m = m;
        demo.player_slot = 0;
        let mut st = UiState {
            net_mode: true,
            screen: Screen::Results,
            ..Default::default()
        };

        teardown_net_match(&mut nms, &mut demo, &mut st);
        assert!(nms.rep.is_none() && nms.cfg.is_none() && !nms.go_pending);
        // The offline 4v4 demo comes back (JS `netMatchEnd` re-shows the
        // menus' own match; main.js L823-842).
        assert_eq!(demo.m.actors.len(), 8, "offline roster restored");
        assert_eq!(demo.player_slot, 0);
        assert_eq!(st.screen, Screen::OnlineLobby);

        // Idempotent: a second `Match{End}` does not rebuild anything.
        teardown_net_match(&mut nms, &mut demo, &mut st);
        assert_eq!(demo.m.actors.len(), 8);
        assert_eq!(st.screen, Screen::OnlineLobby);
    }
}
