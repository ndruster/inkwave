//! Task 21/22 — authoritative sharding, host duties and the playback layer
//! (replication + [`super::playback`]).
//!
//! Faithful port of the *ownership / routing / host-control* half of
//! `src/net/netmatch.js`; the playback half (per-sender clocks, Hermite
//! interpolation, corrections — Task 22) lives in [`super::playback`] and is
//! driven through [`Replication::playback`]. The event timeline (Task 23) is
//! the remaining seam (`out`, the per-peer `events` queue).
//!
//! Mapping (line numbers are the JS source):
//!   - ownership mask `a.remote = r.owner !== me` (match.js L110) ->
//!     [`Replication::remote`] fed to [`crate::match_::Match::step_masked`]
//!   - `_sendTick` (L190-203, 20 Hz, host clock snapshot `c` every 0.5 s) ->
//!     [`Replication::update`]
//!   - `sendHit` (L163-167) / `_hit` (L630-636) -> the `MatchEvent::HitSend`
//!     drain in [`Replication::update`] and the `'hit'` case of
//!     [`Replication::on_message`]
//!   - `onMessage` switch (L206-221) -> [`Replication::on_message`]
//!   - `_hostClock` / `_hostState` / `_result` / `sendResult` / `sendEnd`
//!     (L639-666) -> the `'t'` / `'st'` / `'res'` / `'end'` cases and
//!     [`Replication::send_result`] / [`Replication::send_end`]
//!   - `onLeave` / `_adopt` / `_remove` (L671-707, match.js L169-182) ->
//!     [`Replication::on_leave`]
//!   - bot driving for owned + adopted actors (match.js L222-224) ->
//!     [`Replication::drive_bots`] (the autopilot snapshot pattern, Task 9)
//!   - `_tick` clock bookkeeping / `_advance` / `_pathAt` / `_sample` /
//!     `applyRemote` / `hermite` / `quantile` (L229-346, L349-428, L761-786)
//!     -> [`super::playback`], driven by [`Replication::update`] (frame
//!     order: `now` → advance → sample → play events → tick timer → record
//!     ink + event bus → apply — everything recorded after the send rides
//!     the *next* tick, like JS `Match.update` running after
//!     `NetMatch.update`) and [`Replication::on_tick`]
//!   - event timeline recording / replay (Task 23): local splats, shots,
//!     triggers and `splatted` / `respawn` events are recorded onto the
//!     outgoing tick `e` (JS `_rec` / `recSplat` / `recProj` /
//!     `_onLocalEvent`, L107-143) by [`Replication::record_ink`] and the bus
//!     drain; received events play on the sender timeline (JS `_playEvents` /
//!     `_play` / `_playEvent` / `_remoteSplat` / `_remoteRespawn`,
//!     L447-619) via [`Replication::play_events`]
//!
//! Scope and deviations (recorded for review):
//!   - **Direct time source**: JS stamps ticks with `now() =
//!     performance.now()/1000`; the sim has no wall clock, so the replicator
//!     accumulates its own logical seconds (`now += dt`, f64 — JS numbers
//!     are f64 end to end) and feeds it to the playback clock. Rust↔Rust
//!     only — both ends use the same rule, and only time *differences*
//!     (`o = now − ts`, `target = now − off − delay`) ever matter.
//!   - **Proxy state comes off the playback timeline** (Task 22): received
//!     samples enter the per-actor buffer; [`super::playback::Playback::apply`]
//!     writes position (path + correction), velocity, facing, flags,
//!     hp/ink/invuln/turf onto the proxy [`Actor`] each frame. JS app-layer
//!     fields the sim does not model (`hurtFlash`, `aimDir`/`aimPoint`,
//!     climbing `wallN`, special/super-jump/kit pose state, FX/audio) are
//!     skipped — see the playback module docs for the line-by-line list.
//!   - **Death / respawn are event-driven** (Task 23, closing the Task 22
//!     ALIVE-edge deviation): the victim's owner records `'ev splatted'` /
//!     `'ev respawn'` on its timeline (JS `_onLocalEvent`, L139-143) and every
//!     receiver runs [`super::playback::Playback::remote_splat`] /
//!     [`super::playback::Playback::remote_respawn`] when the event matures
//!     (JS `_remoteSplat` / `_remoteRespawn`, L603-619). [`super::playback::Playback::apply`]
//!     no longer touches the ALIVE edge. A player who dies and respawns
//!     between two samples is now handled exactly like JS (the events land in
//!     order on the sender timeline).
//!   - **`w:'shot'`**: the JS shooter round reports `p.weaponId || p.wid ||
//!     p.type` (weapons.js L1608) and the M1 Spritzer has no `weaponId`/`wid`,
//!     so the wire id is `'shot'` — the victim's owner only forwards it to
//!     FX/audio (M1 has no kit damage modifiers reading `w`).
//!   - **`dh` / `bhit`**: M2 is turf-only with no devices/boss — received as
//!     ignored stubs (JS L210-215), per the spec's 收口要求.
//!   - **Adopted-bot difficulty**: JS `new BotBrain(a, cfg.difficulty ||
//!     'normal')` (L698); the embedded tuning ships only `easy`
//!     ([`crate::tuning::DifficultySet`]), so adopted bots always get `easy`.
//!   - **`spawnPending` / `handoff`**: implemented in [`super::playback`]
//!     (JS L679 `buf.length = 0; handoff = true`, L704 mid-respawn adopt
//!     finishes the respawn locally). [`Replication::on_leave`] hands the
//!     buffer clear + glide flag to [`super::playback::Playback::handoff`];
//!     [`Replication::adopt`] continues the teleport counter from the
//!     *shown* `tp` (JS `a.net.tp`, L702) via
//!     [`super::playback::Playback::adopt_reset`].
//!   - **Guest result `points`/`elapsed`**: the wire `{k:'res'}` carries only
//!     `cov/win/st` (L652-654); the Rust [`crate::match_::MatchResult`] adds
//!     the spec score line, so guests rebuild `points` from the stat rows
//!     (`Σ turf × pointsPerM2`) and `elapsed` from the duration (the clock is
//!     hard-set to 0, L660).
//!   - **`{k:'own'}`**: reserved stub on both sides (JS `_ownership` L709 is
//!     empty, the host never sends it) — received and ignored.
//!   - **`net.tp` initial value**: JS seeds `a.net.tp = -1` (L85) and copies
//!     `a.netTp = a.net.tp || 0` (L702), so a never-respawned actor reports
//!     `-1` (falsy → 0) until its first teleport; the Rust [`Actor`] starts
//!     `net_tp` at 0 and bumps it on respawn. The counter only gates the
//!     Task 22 path-cut, so the absolute base is irrelevant — Rust↔Rust both
//!     ends agree.
//!   - **HitSend timing**: JS calls `sendHit` inline the frame the projectile
//!     lands (weapons.js L1608 path); the Rust sim surfaces the same hit as a
//!     [`crate::match_::MatchEvent::HitSend`] drained by the *next*
//!     [`Replication::update`], so the `{k:'hit'}` frame can leave up to one
//!     frame (1/60 s) later. The victim's owner applies it on arrival either
//!     way.
//!   - **`Match::step` runs before [`Replication::update`]** (app/test loop;
//!     JS runs `NetMatch.update` before `Match.update`): a ghost round
//!     dequeued by `play_events` starts integrating one frame later than JS,
//!     and replayed splats land on the grid after this frame's local step
//!     instead of before it. Both ends of Rust↔Rust agree on the rule, so
//!     grids stay identical; only the presentation start of a ghost is a
//!     frame late.
//!   - **Trigger recording scope**: JS records *every* `character.trigger`
//!     of a local actor (L90-93); the sim only observes the two triggers
//!     with gameplay meaning — `'shoot'` (the fire bus record) and
//!     `'spawn'` (the respawn bus record). Jump/land/dodge/special triggers
//!     are character animation the M1 Spritzer sim does not model, so they
//!     never fire and are never recorded.
//!   - **Kill confirmation**: JS `_remoteSplat` emits a local
//!     `{killed:true, weaponId}` `'hit'` when the attacker is the local
//!     player (L618) — HUD/FX feedback. The sim has no consumer for it; the
//!     attribution itself (`attacker.stats.splats++`, L613) is ported.
//!   - **Render drain wiring**: [`crate::paint::PaintGrid::drain_ink`] may
//!     run every frame (the app renderer does), but it must run *after*
//!     [`Replication::update`] — a drain between `Match::step` and the
//!     update would remove the local splats before `record_ink` sees them
//!     (the seq cursor is drain-safe once recorded; the entries themselves
//!     must still exist when the recorder runs). The
//!     `regression_recording_survives_render_draining` test pins it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, ActorInput, FIXED_DT, SimWorld};
use crate::bot::{Bot, BotCtx};
use crate::match_::{Match, MatchEvent, MatchResult, Phase, ZoneOutcome, find_actor};
use crate::nav::NavGraph;
use crate::net::playback::Playback;
use crate::net::session::{StartCfg, map_no_bots};
use crate::net::transport::Transport;
use crate::net::wire::{
    self, JVal, decode_direct, decode_host, decode_tick, encode_direct, encode_host, encode_tick,
    parse_json,
};
use crate::net::{
    ActorSample, ActorStatsRow, BombRecord, DirectMsg, EventRecord, ForwardEvent, HostMsg,
    KitRecord, KitVal, ProjectileRecord, SessionId, SplatRecord, Tick, TickEvent, TimedEvent,
    TrigData, TriggerRecord, ZoneRecord, ZonesResult,
};
use crate::paint::{SplatKind, SplatOpts};
use crate::tuning::{Difficulty, PlayerTuning, Spritzer, SubSet, WeaponSet};
use crate::weapon::blade::BladeWave;
use crate::weapon::brolly::LaunchedCanopy;
use crate::weapon::{PROJ_DRAG, PROJ_GRAV, PROJ_SIZE, SHOT_LIFE, WeaponKind};
use crate::weapon::{bow, mitts};
use glam::{Vec3, vec3};

/// JS `TICK = 1/20` (netmatch.js L30): owners stream every 1/20 s.
pub const TICK_DT: f32 = 1.0 / 20.0;
/// JS host clock snapshot period (`clockT = 0.5`, L199).
pub const CLOCK_PERIOD: f32 = 0.5;

/// JS `match.state` strings (match.js) ↔ [`Phase`]. The wire carries the JS
/// spelling (`{k:'st', s:'playing'}`, netmatch.js L80); `End` is JS `judge`.
#[must_use]
pub fn phase_str(p: Phase) -> &'static str {
    match p {
        Phase::Init => "init",
        Phase::Intro => "intro",
        Phase::Active => "playing",
        Phase::Finish => "finish",
        Phase::End => "judge",
    }
}

/// [`Phase`] from a wire state string (`None` for unknown strings — JS's
/// `setState` would accept them, the sim keeps the enum closed).
#[must_use]
pub fn phase_of(s: &str) -> Option<Phase> {
    Some(match s {
        "init" => Phase::Init,
        "intro" => Phase::Intro,
        "playing" => Phase::Active,
        "finish" => Phase::Finish,
        "judge" => Phase::End,
        _ => return None,
    })
}

/// Something the app layer must react to (JS `G.game?.netMatchEnd?.()`,
/// netmatch.js L218 — the host's `{k:'end'}` returns everyone to the lobby).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationEvent {
    /// The host ended the match — call
    /// [`crate::net::session::NetSession::end_match`] and tear down.
    MatchEnd,
}

/// The replication controller for one client of one match: who owns which
/// squidkid, the 20 Hz tick stream, the hit channel, the host's clock/state/
/// result broadcasts, and leave-time adoption / host migration.
///
/// All roster-parallel vectors ([`Replication::nids`], [`Replication::owners`],
/// [`Replication::remote`], …) are indexed by the *match actor index* — the
/// roster order the [`Match`] was built from (match.js `_setupRoster` L107-113
/// pushes actors in roster order, so the arrays stay aligned;
/// [`Replication::on_leave`] prunes them together with [`Match::remove_actor`]).
pub struct Replication {
    /// Our relay session id (`this.myId`, netmatch.js L63).
    pub my_id: SessionId,
    /// The current host (`this.s.hostId`). The app updates this from the
    /// session's `Host` / `Leave` events **before** calling
    /// [`Replication::on_leave`] (JS session.js L153-157 re-elects, then
    /// `netm.onLeave` runs).
    pub host_id: SessionId,
    /// Roster-order actor nids (JS `a.nid`, match.js L110).
    pub nids: Vec<u8>,
    /// Roster-order owner session ids (JS `a.owner`).
    pub owners: Vec<SessionId>,
    /// The ownership mask (JS `a.remote`, match.js L110) — pass to
    /// [`Match::step_masked`], or read via [`Replication::remote_mask`].
    pub remote: Vec<bool>,
    /// `true` once a leave made this client the host (drove
    /// `m.follower = false`, L681). Introspection only.
    pub adopted_host: bool,
    /// Adopted / roster bots by actor index (JS `a.bot`, match.js L112).
    /// Only ever `Some` for *owned* actors — the host runs the bots. Roster
    /// bots need [`Replication::attach_bot`] once the actor exists; adopted
    /// bots get their brain inside [`Replication::on_leave`].
    bots: Vec<Option<Bot>>,
    /// The playback layer (Task 22): per-sender clocks, per-actor sample
    /// buffers, Hermite paths and corrections. Roster-parallel like the
    /// arrays above (pruned together in [`Replication::remove_at`]).
    playback: Playback,
    /// Outgoing event timeline (JS `this.out`, L107) — filled by the local
    /// recorder (Task 23) and shipped non-empty as the tick's `e`.
    pub out: Vec<TimedEvent>,
    /// The [`crate::paint::PaintGrid::ink_seq`] value this client has
    /// recorded through: only stream entries with a higher `seq` that were
    /// not painted under `applying` go onto `out`. The Rust stand-in of
    /// JS's `applying` loop guard (`recSplat`, L110: a splat replayed from
    /// a peer's timeline is never re-recorded and never re-broadcast). The
    /// counter is monotonic per grid, so the render layer's `drain_ink`
    /// cannot hide a local splat; `Match::restart`'s fresh grid resets it.
    recorded_ink: usize,
    /// JS `tickT` (L57, starts 0 → the first frame sends).
    tick_t: f32,
    /// JS `clockT` (L61, starts 0 → the first host tick carries `c`).
    clock_t: f32,
    /// Logical sender clock, seconds (see module docs).
    now: f64,
    /// The humans-only stage flag (JS `mapNoBots(this.cfg.map)`, L673).
    no_bots: bool,
    /// Queued `{k:'dh', kind:'sp'}` bubble hurts awaiting the owner-side
    /// world registry (Task 35 batch 3): `on_message` has no collision /
    /// sub-system handles, so the hit rides to the next [`Replication::update`]
    /// where the full [`crate::special::world::WorldCtx`] exists. JS applies
    /// `netHurt` on packet arrival (between frames) — the deferral is at
    /// most one frame (recorded deviation).
    pending_sp_hurt: Vec<(u32, f32)>,
}

impl Replication {
    /// Bind to a started match: the roster decides ownership — JS
    /// `_setupRoster` (match.js L104-113). Roster bots (owned, `bot: true`)
    /// still need [`Replication::attach_bot`] once the [`Match`] actors
    /// exist (JS builds the brain right after the actor, match.js L112);
    /// [`Replication::bind_match`] applies the host/guest flag.
    #[must_use]
    pub fn new(cfg: &StartCfg, my_id: &str, host_id: &str) -> Self {
        let n = cfg.roster.len();
        Self {
            my_id: my_id.to_string(),
            host_id: host_id.to_string(),
            nids: cfg.roster.iter().map(|r| r.nid).collect(),
            owners: cfg.roster.iter().map(|r| r.owner.clone()).collect(),
            remote: cfg.roster.iter().map(|r| r.owner != my_id).collect(),
            adopted_host: false,
            bots: (0..n).map(|_| None).collect(),
            playback: Playback::new(n, TICK_DT as f64),
            out: Vec::new(),
            recorded_ink: 0,
            tick_t: 0.0,
            clock_t: 0.0,
            now: 0.0,
            no_bots: map_no_bots(&cfg.map),
            pending_sp_hurt: Vec::new(),
        }
    }

    /// Apply the roster to a freshly built [`Match`]: the host/guest flag
    /// (JS `this.follower = !o.host`, match.js L106).
    pub fn bind_match(&self, m: &mut Match) {
        m.follower = !self.is_host();
        // Zone Control mirrors the flag every `Match::step` (match_.rs
        // L761), but `play_events` runs *before* the first step, so seed it
        // here: a guest's first-frame zone records must pass `net_event`'s
        // follower gate (zones.rs L1223, JS zones.js L373).
        if let Some(z) = m.zones.as_mut() {
            z.follower = m.follower;
        }
    }

    /// JS `bind` L79: on a humans-only stage, an actor whose owner left
    /// *while the match was loading* (before a NetMatch existed to hear the
    /// leave frame) is dropped now. The app passes the session's current
    /// member ids (`s._members`, session.js L31/L100-101); own actors are never
    /// pruned. No-op on stages that keep bots.
    pub fn prune_loading_leavers(&mut self, m: &mut Match, members: &[SessionId]) {
        if !self.no_bots {
            return;
        }
        // Iterate high→low: pruning shifts the parallel arrays.
        for i in (0..self.owners.len()).rev() {
            if self.owners[i] != self.my_id && !members.iter().any(|x| *x == self.owners[i]) {
                self.remove_at(m, i);
            }
        }
    }

    /// The live slot of the local player's actor (JS resolves `m.local` as an
    /// object ref, match.js L116 — the Rust port tracks an index, so after a
    /// `remove_at` shift it must re-resolve). Only meaningful on a
    /// humans-only stage (where `prune_loading_leavers` / `on_leave` drop
    /// actors): bots never exist there, so the first owner match is the
    /// local human.
    #[must_use]
    pub fn local_slot(&self, my_id: &str) -> Option<usize> {
        self.owners.iter().position(|o| o == my_id)
    }

    /// Drop actor `i` from the match and every roster-parallel array
    /// (JS `_remove`, L686-690).
    fn remove_at(&mut self, m: &mut Match, i: usize) {
        m.remove_actor(i);
        self.nids.remove(i);
        self.owners.remove(i);
        self.remote.remove(i);
        self.bots.remove(i);
        self.playback.remove_at(i);
    }

    /// Attach the brain for roster bot `idx` (JS match.js L112). Only valid
    /// for owned, non-remote actors — the host never follows its own bots.
    pub fn attach_bot(&mut self, idx: usize, a: &Actor, diff: &Difficulty, seed: u64) {
        if idx < self.nids.len() && !self.remote[idx] {
            self.bots[idx] = Some(Bot::new(a, diff, seed, idx));
        }
    }

    #[must_use]
    pub fn is_host(&self) -> bool {
        self.my_id == self.host_id
    }

    /// The [`Match::step_masked`] mask.
    #[must_use]
    pub fn remote_mask(&self) -> &[bool] {
        &self.remote
    }

    #[must_use]
    pub fn owner_of(&self, idx: usize) -> Option<&str> {
        self.owners.get(idx).map(String::as_str)
    }

    #[must_use]
    pub fn nid_of(&self, idx: usize) -> Option<u8> {
        self.nids.get(idx).copied()
    }

    /// Roster index by `nid` (JS `byNid`, netmatch.js L74).
    #[must_use]
    pub fn idx_of_nid(&self, nid: u8) -> Option<usize> {
        self.nids.iter().position(|&n| n == nid)
    }

    /// The last *shown* teleport counter for an actor (JS `a.net.tp`, the
    /// playback path's current `tp`).
    #[must_use]
    pub fn last_tp(&self, idx: usize) -> u32 {
        self.playback.tp_shown(idx)
    }

    /// The playback layer (read-only view for tests / the app: buffer depth,
    /// shown `tp`, proxy visibility, [`Playback::proxy_shown`]).
    #[must_use]
    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// The app's host view changed (session `Host` / `Leave` events).
    pub fn set_host(&mut self, host_id: &str) {
        self.host_id = host_id.to_string();
    }

    /// One 60 Hz frame of replication (JS `NetMatch.update`, L172-188, then
    /// the `Match.update` → `applyRemote` consumption, match.js L228):
    /// advance every sender's playback clock, sample every remote actor's
    /// path, **replay the due event-timeline records** (JS `_playEvents`,
    /// L179 — before the tick send, exactly like JS), run the 20 Hz tick
    /// timer (owners stream their actors + the locally recorded timeline;
    /// the host rides its `c` clock snapshot every 0.5 s), record the local
    /// splats (Task 23) and drain the match event bus — the host
    /// re-broadcasts phase changes (`{k:'st'}`, L80), every client routes
    /// proxy hits to the victim's owner (`{k:'hit'}`, L163-167) and records
    /// the local shots / triggers / deaths / respawns onto the outgoing
    /// timeline (Task 23) — then apply the sampled timeline onto the
    /// proxies.
    ///
    /// Returns the events the replicator did *not* consume (Splat / Turf /
    /// Countdown / OneMinute — HUD consumers; Fire / Respawn / HitSend are
    /// recorded / routed here).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        m: &mut Match,
        sim: &SimWorld,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
        tr: &mut impl Transport,
    ) -> Vec<MatchEvent> {
        self.now += dt as f64;

        // ---- queued ghost-bubble hurts (Task 35 batch 3, JS `netHurt`
        // L325-326 on the owner's screen): the owner-side registry lives in
        // `m.specials` and the hurt needs the full world context, so it
        // lands here at the first frame boundary after the message.
        if !self.pending_sp_hurt.is_empty() {
            let pending = std::mem::take(&mut self.pending_sp_hurt);
            for (gid, d) in pending {
                let time = m.projectiles.time;
                let mut ctx = crate::special::world::WorldCtx {
                    actors: &mut m.actors,
                    collision: sim.collision,
                    paint: &mut m.paint,
                    rng: &mut m.rng,
                    subs: &mut m.subs,
                    sb,
                    t,
                    events: m.projectiles.events_mut(),
                    remote: Some(&self.remote).map(|v| &**v),
                    time,
                };
                crate::special::net_hurt_obj(&mut m.specials, gid, d, &mut ctx);
            }
        }

        // ---- playback clocks + path sampling (JS L176-177, before the
        // tick send so the frame's samples ride the *next* packet's
        // timeline, exactly like `update`'s ordering).
        let dt64 = dt as f64;
        let gravity = t.gravity as f64;
        self.playback.advance_all(self.now, dt64);
        {
            let playback = &mut self.playback;
            let owners = &self.owners;
            for (i, &rem) in self.remote.iter().enumerate() {
                if rem {
                    playback.sample(i, &owners[i], dt64, gravity);
                }
            }
        }

        // ---- event timeline replay (JS `_playEvents`, L179: after
        // sampling, before the tick send — so a due splat paints into this
        // frame's grid and a due death already hides the proxy before its
        // sample is applied below).
        self.play_events(m, sim, t, w, sb);

        // ---- outgoing tick (L182-187: `tickT -= dt; if (<=0) { += TICK;
        // if (<0) = TICK; _sendTick() }`).
        self.tick_t -= dt;
        if self.tick_t <= 0.0 {
            self.tick_t += TICK_DT;
            if self.tick_t < 0.0 {
                self.tick_t = TICK_DT;
            }
            self.send_tick(m, tr);
        }

        // ---- Zone Control: the host's rules decisions + count snapshots
        // (JS `recZone`, netmatch.js L148, fed by `zones.js _net` L144).
        // JS records them inside `Match.update` — `zones.update` (match.js
        // L197-198) runs *before* the paint flush that produces the splat
        // records — so the drain lands before `record_ink` below: both ride
        // the next tick, zone records first. Followers never own the rules
        // (JS `recZone`'s `isHost` guard; `ZoneControl::_net` L623).
        if self.is_host()
            && let Some(z) = m.zones.as_mut()
        {
            for e in z.drain_net() {
                self.rec(TickEvent::Zone(ZoneRecord { payload: e }));
            }
        }

        // ---- record the local splats painted since the last frame (JS
        // `recSplat` fires during `Match.update`, which runs *after*
        // `NetMatch.update`'s `_sendTick` — so the record rides the next
        // tick, exactly like the bus records below).
        self.record_ink(m);

        // ---- event bus (JS listens on `match:state` L80, `splatted` /
        // `respawn` / `weapon:fire` through the FORWARD subscription
        // L77/L139-143, and calls `sendHit` inline, L163-167; the sim
        // surfaces all of these as bus events drained here).
        let drained = std::mem::take(&mut m.events);
        let mut rest = Vec::new();
        for e in drained {
            match e {
                MatchEvent::Phase(p) => {
                    if self.is_host() {
                        let msg = HostMsg::State {
                            phase: phase_str(p).to_string(),
                            t: Some(m.time),
                        };
                        tr.broadcast(&encode_host(&msg));
                    }
                    rest.push(MatchEvent::Phase(p));
                }
                MatchEvent::HitSend {
                    attacker,
                    victim,
                    damage,
                } => {
                    self.send_hit(m, t, tr, attacker, victim, damage);
                }
                MatchEvent::Fire {
                    owner,
                    muzzle,
                    dir,
                    charge,
                    len,
                } => {
                    // JS order inside one shot: `recProj` (weapons.js
                    // `_push` → netmatch L116) → `emit('weapon:fire')` →
                    // `_onLocalEvent` (L139) → `character.trigger('shoot')`
                    // (L179, after `fireShooter` returns).
                    self.record_fire(m, &w.shooter, owner, muzzle, dir, charge, len);
                }
                MatchEvent::Respawn { team, slot } => {
                    // JS `respawn()` (actor.js L172-175): `trigger('spawn')`
                    // fires *before* `emit('respawn')`, so the timeline
                    // carries `tr spawn` ahead of `ev respawn`.
                    if let Some(i) = find_actor(&m.actors, team, slot)
                        && !self.remote[i]
                    {
                        let nid = self.nids[i];
                        self.rec(TickEvent::Trigger(TriggerRecord {
                            nid,
                            name: "spawn".to_string(),
                            data: TrigData::Zero,
                        }));
                    }
                    self.record_event(m, ForwardEvent::Respawn, team, slot);
                    rest.push(MatchEvent::Respawn { team, slot });
                }
                MatchEvent::Splat {
                    victim_team,
                    victim_slot,
                    attacker,
                    cause,
                } => {
                    // JS `_onLocalEvent` (L139-143): forwarded only for
                    // *local* victims — a proxy's death is decided by its
                    // owner and arrives on the owner's timeline instead.
                    if let Some(vi) = find_actor(&m.actors, victim_team, victim_slot)
                        && !self.remote[vi]
                    {
                        self.record_splat_event(m, vi, attacker, cause);
                    }
                    rest.push(MatchEvent::Splat {
                        victim_team,
                        victim_slot,
                        attacker,
                        cause,
                    });
                }
                MatchEvent::SpecialUse { owner, id } => {
                    // JS `_onLocalEvent` (L139-143) over the FORWARD list
                    // (L47): `['ev', 'special:use', packEvent({actor, id})]`
                    // — `packEvent` maps the actor to `{n: nid}` and passes
                    // the string through (L803-813). Local actors only; the
                    // receiver's `play_event` ignores the name (FX-only).
                    if let Some(i) = find_actor(&m.actors, owner.0, owner.1)
                        && !self.remote[i]
                    {
                        self.rec(TickEvent::Event(EventRecord {
                            name: ForwardEvent::SpecialUse,
                            payload: JVal::Obj(vec![
                                (
                                    "actor".to_string(),
                                    JVal::Obj(vec![(
                                        "n".to_string(),
                                        JVal::Num(f64::from(self.nids[i])),
                                    )]),
                                ),
                                ("id".to_string(), JVal::Str(id.clone())),
                            ]),
                        }));
                    }
                    rest.push(MatchEvent::SpecialUse { owner, id });
                }
                MatchEvent::SpecialSlam { owner, pos, radius } => {
                    // JS `['ev', 'special:slam', packEvent({actor, pos,
                    // radius})]` (actor.js L914): vector → `[r2, r2, r2]`,
                    // number → `r3` (netmatch.js L803-813).
                    if let Some(i) = find_actor(&m.actors, owner.0, owner.1)
                        && !self.remote[i]
                    {
                        self.rec(TickEvent::Event(EventRecord {
                            name: ForwardEvent::SpecialSlam,
                            payload: JVal::Obj(vec![
                                (
                                    "actor".to_string(),
                                    JVal::Obj(vec![(
                                        "n".to_string(),
                                        JVal::Num(f64::from(self.nids[i])),
                                    )]),
                                ),
                                (
                                    "pos".to_string(),
                                    JVal::Arr(vec![
                                        JVal::Num(wire::r2(f64::from(pos.x))),
                                        JVal::Num(wire::r2(f64::from(pos.y))),
                                        JVal::Num(wire::r2(f64::from(pos.z))),
                                    ]),
                                ),
                                ("radius".to_string(), JVal::Num(wire::r3(f64::from(radius)))),
                            ]),
                        }));
                    }
                    rest.push(MatchEvent::SpecialSlam { owner, pos, radius });
                }
                other => rest.push(other),
            }
        }

        // ---- kit world-object spawns (JS `recKit`, L134-137): the owner
        // records `['k', nid, kind, data]` for its *local* actors only — a
        // proxy never launches (its runner is masked), and the guard mirrors
        // JS `if (!a || a.remote …) return`.
        for spawn in std::mem::take(&mut m.kit_out) {
            if let Some(i) = find_actor(&m.actors, spawn.owner.0, spawn.owner.1)
                && !self.remote[i]
            {
                let mut data = spawn.data;
                if matches!(
                    spawn.kind.as_str(),
                    "subs" | "shaker" | "waddle" | "torpedo" | "tracer" | "boomerang"
                ) && data.len() >= 2
                {
                    // D-T34-5: the sim's owner-side gid (`slot * 100_000 +
                    // off`) is rewritten into the wire gid (`nid * 100_000 +
                    // off`) so receivers key their ghosts on
                    // `(src_nid, gid)` — JS `netId` packs the owner's `nid`
                    // straight in (registry.js L64-65).
                    data[1] =
                        (u32::from(self.nids[i]) * 100_000 + (data[1] as u32) % 100_000) as f32;
                    // The sim pushes the *target's actor index* into the
                    // record slots JS fills with `tgt?.nid ?? -1` (waddle.js
                    // L381 / L469, torpedo.js L333, tracer.js L342); rewrite
                    // the index into that actor's peer nid (−1 / −2 sentinels
                    // stay).
                    let idx_slot = match (spawn.kind.as_str(), data[0] as i32) {
                        ("waddle", 3) | ("torpedo", 3) => Some(2),
                        ("waddle", 4) => Some(6),
                        ("tracer", 1) => Some(5),
                        _ => None,
                    };
                    if let Some(k) = idx_slot
                        && let Some(&v) = data.get(k)
                    {
                        let vi = v as i32;
                        if vi >= 0 && (vi as usize) < self.nids.len() {
                            data[k] = f32::from(self.nids[vi as usize]);
                        }
                    }
                }
                self.rec(TickEvent::Kit(KitRecord {
                    nid: self.nids[i],
                    kind: spawn.kind,
                    sub_kind: spawn.sub_kind,
                    data: data.into_iter().map(KitVal::Num).collect(),
                }));
            }
        }

        // ---- specials-channel records (JS specials.js `rec(a, d)` →
        // `netRec(a, 'sp', d)`, L42): drain each *local* actor's per-frame
        // queue into `['k', nid, 'sp', data]` timeline records — the exact
        // shape `recKit` (netmatch.js L134-137) writes. Op-2 (world-object
        // spawn) records carry the gid the sim stamped in slot form
        // (`slot * 100_000 + seq`); rewrite it into the wire form
        // (`nid * 100_000 + seq`) so receivers key ghosts on `(src_nid,
        // gid)` like JS `netId` (registry.js L64-65) — D-T34-5 pattern.
        for (i, a) in m.actors.iter_mut().enumerate() {
            if self.remote[i] || a.sp_records.is_empty() {
                continue;
            }
            let nid = self.nids[i];
            for mut rec in a.sp_records.drain(..) {
                let op = rec.first().and_then(KitVal::as_f32).unwrap_or(-1.0) as i32;
                if op == 2
                    && let Some(KitVal::Num(g)) = rec.get_mut(2)
                {
                    *g = (u32::from(nid) * 100_000 + (*g as u32) % 100_000) as f32;
                }
                self.rec(TickEvent::Kit(KitRecord {
                    nid,
                    kind: "sp".to_string(),
                    sub_kind: None,
                    data: rec,
                }));
            }
        }

        // ---- splat-bomb throws (JS `recBomb`, netmatch.js L124-127):
        // `['b', nid, kind, r2×6]` on the owner's timeline, local actors
        // only — the same `!a.remote` guard as `recKit`.
        for spawn in std::mem::take(&mut m.bomb_out) {
            if let Some(i) = find_actor(&m.actors, spawn.owner.0, spawn.owner.1)
                && !self.remote[i]
            {
                self.rec(TickEvent::Bomb(BombRecord {
                    nid: self.nids[i],
                    kind: spawn.kind,
                    pos: [spawn.pos.x, spawn.pos.y, spawn.pos.z],
                    vel: [spawn.vel.x, spawn.vel.y, spawn.vel.z],
                }));
            }
        }

        // ---- ghost-device hits (JS `sendDevHit`, netmatch.js L157-159):
        // point-to-point `{k:'dh', kind:'subs', id, d}` to the device
        // owner's screen. The ghost's `gid` already rides the wire form
        // (D-T34-5); a ghost whose owner is *ours* cannot exist (ghosts are
        // only built from remote records), so the self-target is dropped.
        // JS `netHurt` (registry.js L68) guards the send with `dmg > 0`;
        // this chokepoint is its Rust counterpart — a zero-damage hit never
        // leaves the owner's screen (the curtain fade is applied locally
        // before the event is queued, so the guard does not touch it).
        for hit in std::mem::take(&mut m.dh_out) {
            if let Some(i) = find_actor(&m.actors, hit.owner.0, hit.owner.1)
                && hit.d > 0.0
                && self.owners[i] != self.my_id
            {
                let msg = DirectMsg::DeviceHit {
                    kind: hit.kind.clone(),
                    id: hit.gid,
                    d: hit.d,
                };
                tr.send_to(&self.owners[i], &encode_direct(&msg));
            }
        }

        // ---- apply the sampled path + correction onto the proxies (JS
        // `applyRemote`, L349-428, run by `Match.update` after
        // `NetMatch.update` in the same frame).
        {
            let playback = &mut self.playback;
            for (i, &rem) in self.remote.iter().enumerate() {
                if rem {
                    playback.apply(i, &mut m.actors[i], dt);
                    // JS `SpecialSystem.update` → `_ghostTick` (specials.js
                    // L533-570 / L287-298): every frame advances a proxy's
                    // *ghost* active (timer, firing decay, duration expiry,
                    // death end) regardless of the playback ready / alive
                    // gates above — the sim runs it right after `applyRemote`
                    // (JS runs it later in the same frame, one sub-frame of
                    // skew, harmless for the sim).
                    crate::special::ghost_tick(&mut m.actors[i], dt);
                    // JS L296 → `GHOST.crab.tick` L2118 → `IMPL.crab.tick`
                    // L2005: the ghost's rolled footprint raises the paint
                    // request under `ghostMute` — drain it muted: the seed
                    // draw still happens (specials.js L53 evaluates the
                    // `Math.random()` argument before paint.js L470's mute
                    // return), the splat / turf do not.
                    if m.actors[i].sp_crab_paint {
                        m.actors[i].sp_crab_paint = false;
                        crate::special::crab::paint_footprint(
                            &mut m.actors[i],
                            sim.collision,
                            &mut m.paint,
                            &mut m.rng,
                            true,
                        );
                    }
                    // JS `SpecialSystem.end` L373: a ghost's clean `time` end
                    // routes its proxy marker through `returning()` /
                    // `finish()` — drain the action raised by `ghost_tick`.
                    crate::special::drain_marker_action(&mut m.actors[i], &mut m.specials);
                    // JS `applyRemote` L398: `MAIN_KITS[kind]?.netApply?.(wr,
                    // S.ks | 0, dt)` — rebuild the remote brolly canopy pose
                    // so the held-shield assembly (match_.rs `refreshShields`,
                    // brolly.js L178-190) turns the proxy's umbrella into a
                    // shield that blocks LOCAL shots. The `!alive` / `!ready`
                    // early-outs of JS L352/L354 gate the call identically.
                    if m.kinds[i] == WeaponKind::Brolly
                        && m.actors[i].alive
                        && let Some(ks) = playback.cur_ks(i)
                    {
                        m.runners[i].net_apply_brolly(ks, w);
                    }
                }
            }
        }

        rest
    }

    /// JS `_rec` (L107): prefix the record with this client's logical
    /// sender-clock (`r3(now())`) and queue it onto the outgoing timeline.
    fn rec(&mut self, kind: TickEvent) {
        self.out.push(TimedEvent {
            ts: wire::r3(self.now) as f32,
            kind,
        });
    }

    /// JS `recSplat` (L109-114) for every splat the local sim painted since
    /// the last frame. The replayed (remote) splats are *not* re-recorded:
    /// `play_events` paints under [`crate::paint::PaintGrid::applying`] (the
    /// verbatim JS L467-473 guard), which stamps
    /// [`crate::paint::InkSplat::replayed`] on the pushed entry, and the
    /// cursor skips it. The cursor keys on the grid's monotonic
    /// [`crate::paint::PaintGrid::ink_seq`], so the render layer's
    /// [`crate::paint::PaintGrid::drain_ink`] can never hide a local splat,
    /// and a rebuilt grid (`Match::restart`) resets the cursor.
    fn record_ink(&mut self, m: &Match) {
        let seq = m.paint.ink_seq;
        if seq < self.recorded_ink {
            self.recorded_ink = 0; // grid rebuilt (Match::restart)
        }
        let stream = m.paint.ink_stream();
        let start = stream.partition_point(|s| s.seq <= self.recorded_ink);
        for s in &stream[start..] {
            if !s.replayed {
                self.rec(TickEvent::Splat(SplatRecord::from_ink_splat(s)));
            }
        }
        self.recorded_ink = seq;
    }

    /// JS `recProj` (L116-122) + `_onLocalEvent('weapon:fire')` (L139-143) +
    /// the `character.trigger('shoot')` wrapper (L90-93, weapons.js L179):
    /// one local shot leaves three timeline records, in that order. The
    /// projectile's own fields are rebuilt from the [`Spritzer`] tuning —
    /// the M1 Spritzer round is fully determined by it (`ptype: 'shot'`, no
    /// `wid`, no `delay`, `life = min(range/speed, 1.2)` like the local
    /// `fire_shooter`).
    /// Task 31: a charger release (`charge: Some`) is pure hitscan — JS
    /// `fireCharger` never calls `_push`, so no `['p']` record is made; the
    /// `charge`/`len` numbers ride the `['ev'] payload (JS L1281,
    /// `packEvent` keeps numbers via `r3`) and the remote draws the ghost
    /// beam from them.
    #[allow(clippy::too_many_arguments)]
    fn record_fire(
        &mut self,
        m: &Match,
        w: &Spritzer,
        owner: (usize, usize),
        muzzle: Vec3,
        dir: Vec3,
        charge: Option<f32>,
        len: Option<f32>,
    ) {
        let Some(i) = find_actor(&m.actors, owner.0, owner.1) else {
            return;
        };
        if self.remote[i] {
            return; // JS `recProj`: `if (!o || o.remote …) return`
        }
        let nid = self.nids[i];
        if charge.is_none() {
            let speed = w.proj_speed;
            self.rec(TickEvent::Proj(ProjectileRecord {
                nid,
                ptype: "shot".to_string(),
                wid: None,
                pos: [muzzle.x, muzzle.y, muzzle.z],
                vel: [dir.x * speed, dir.y * speed, dir.z * speed],
                delay: 0.0,
                life: (w.range / w.proj_speed).min(SHOT_LIFE),
                straight: w.straight_time,
                radius: w.impact_radius,
                size: PROJ_SIZE,
                grav: PROJ_GRAV,
                drag: PROJ_DRAG,
                trail_every: w.trail_every,
                head: false,
                vis: 0.1,
                tail0: 0.8,
                tail_k: 1.3,
                wob: 0.035,
                wob_f: 26.0,
                nose: 0.3,
                sats: 3.0,
            }));
        }
        // `packEvent({actor, weapon, muzzle, dir})` (L803-813): actor →
        // `{n}`, weapon id string passes through, Vector3 → `[r2 ×3]`.
        // The id is the *shooter's own* weapon (`m.kinds[i]`, JS
        // `emit('weapon:fire', { weapon: w.id })`), not the Spritzer
        // default — the remote dispatches `WEAPONS[e.weapon]` and only the
        // charger branch consumes `charge`/`len`.
        let mut payload = vec![
            (
                "actor".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(nid)))]),
            ),
            (
                "weapon".to_string(),
                JVal::Str(m.kinds[i].id_str().to_string()),
            ),
            (
                "muzzle".to_string(),
                JVal::Arr(vec![
                    JVal::Num(wire::r2(f64::from(muzzle.x))),
                    JVal::Num(wire::r2(f64::from(muzzle.y))),
                    JVal::Num(wire::r2(f64::from(muzzle.z))),
                ]),
            ),
            (
                "dir".to_string(),
                JVal::Arr(vec![
                    JVal::Num(wire::r2(f64::from(dir.x))),
                    JVal::Num(wire::r2(f64::from(dir.y))),
                    JVal::Num(wire::r2(f64::from(dir.z))),
                ]),
            ),
        ];
        // JS L1281: `fireCharger` appends `charge`/`len` after `dir` (the
        // object-literal insertion order `packEvent` preserves); every other
        // weapon's payload stops at `dir`.
        if let Some(c) = charge {
            payload.push(("charge".to_string(), JVal::Num(wire::r3(f64::from(c)))));
        }
        if let Some(l) = len {
            payload.push(("len".to_string(), JVal::Num(wire::r3(f64::from(l)))));
        }
        self.rec(TickEvent::Event(EventRecord {
            name: ForwardEvent::WeaponFire,
            payload: JVal::Obj(payload),
        }));
        self.rec(TickEvent::Trigger(TriggerRecord {
            nid,
            name: "shoot".to_string(),
            data: TrigData::Zero,
        }));
    }

    /// JS `_onLocalEvent('splatted')` → `packEvent({victim, attacker,
    /// cause})` (L139-143, L803-813). Key order is the JS object-literal
    /// insertion order (`actor.js` L233): `victim`, `attacker`, `cause`.
    /// `attacker` is `null` for water deaths — `packEvent` drops nulls, so
    /// the key is simply absent (JS L807-810 skips `v == null`).
    fn record_splat_event(
        &mut self,
        m: &Match,
        vi: usize,
        attacker: Option<(usize, usize)>,
        cause: crate::actor::SplatCause,
    ) {
        let mut pairs = vec![(
            "victim".to_string(),
            JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[vi])))]),
        )];
        if let Some((at, aslot)) = attacker
            && let Some(ai) = find_actor(&m.actors, at, aslot)
        {
            pairs.push((
                "attacker".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[ai])))]),
            ));
        }
        pairs.push((
            "cause".to_string(),
            JVal::Str(
                match cause {
                    crate::actor::SplatCause::Weapon => "weapon",
                    crate::actor::SplatCause::Water => "water",
                }
                .to_string(),
            ),
        ));
        self.rec(TickEvent::Event(EventRecord {
            name: ForwardEvent::Splatted,
            payload: JVal::Obj(pairs),
        }));
    }

    /// JS `_onLocalEvent` for a named event whose payload is
    /// `{actor: <local actor>}` (`respawn`, actor.js L175 → `{n: nid}`).
    fn record_event(&mut self, m: &Match, name: ForwardEvent, team: usize, slot: usize) {
        let Some(i) = find_actor(&m.actors, team, slot) else {
            return;
        };
        if self.remote[i] {
            return;
        }
        self.rec(TickEvent::Event(EventRecord {
            name,
            payload: JVal::Obj(vec![(
                "actor".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[i])))]),
            )]),
        }));
    }

    /// JS `_playEvents` / `_play` (L452-498): dequeue every sender's due
    /// timeline records and apply them in sender-clock order. `'s'` splats
    /// paint through [`crate::paint::PaintGrid::splat`] under
    /// [`crate::paint::PaintGrid::applying`] — the verbatim JS guard
    /// (L467-473) that keeps them out of the recorder; `'p'` rounds spawn
    /// visual-only ghosts; `'tr'` triggers are consumed (their JS consumers
    /// `_netTrig` / `_trigSideEffects`, L481-482, are character
    /// presentation the sim does not model); `'ev'` dispatches `splatted` /
    /// `respawn` into the proxy state machines and ignores the FX-only
    /// names (`weapon:fire` presentation is the `'p'` ghost + audio, out of
    /// sim scope).
    fn play_events(
        &mut self,
        m: &mut Match,
        sim: &SimWorld,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
    ) {
        let due = self.playback.due_events();
        if due.is_empty() {
            return;
        }
        for (_from, events) in due {
            for ev in events {
                match ev.kind {
                    TickEvent::Splat(s) => {
                        let opts = SplatOpts {
                            seed: s.seed,
                            // JS L468: the stretch vector exists only when
                            // any component is non-zero; `stretchAmt` rides
                            // it (L471), defaulting like `recSplat`'s `?? 1`.
                            stretch: if s.stretch[0] != 0.0
                                || s.stretch[1] != 0.0
                                || s.stretch[2] != 0.0
                            {
                                Some(vec3(s.stretch[0], s.stretch[1], s.stretch[2]))
                            } else {
                                None
                            },
                            stretch_amt: if s.stretch[0] != 0.0
                                || s.stretch[1] != 0.0
                                || s.stretch[2] != 0.0
                            {
                                Some(s.stretch_amt)
                            } else {
                                None
                            },
                            kind: s.kind.as_deref().and_then(SplatKind::from_name),
                        };
                        // JS L467/L473: `applying` wraps the replayed paint
                        // so `recSplat` never re-broadcasts it.
                        m.paint.applying = true;
                        m.paint.splat(
                            sim.collision,
                            vec3(s.x, s.y, s.z),
                            s.radius,
                            s.team as usize,
                            &opts,
                        );
                        m.paint.applying = false;
                    }
                    TickEvent::Proj(p) => {
                        // JS `_play` 'p' (L476): `byNid.get(e[2])`, then
                        // `ghostProjectile` — no `remote` guard (a ghost of
                        // your own round can arrive from a peer you adopt
                        // mid-flight; JS lets it die silently).
                        let Some(i) = self.idx_of_nid(p.nid) else {
                            continue;
                        };
                        let owner_team = m.actors[i].team;
                        m.projectiles.list.push(crate::weapon::Projectile {
                            pos: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            prev: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            start: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            vel: vec3(p.vel[0], p.vel[1], p.vel[2]),
                            team: owner_team,
                            owner_slot: m.actors[i].slot,
                            age: 0.0,
                            life: p.life,
                            straight: p.straight,
                            radius: p.radius,
                            // JS L868/L870: `size` comes from the record
                            // (e[15]) — the ghost's own hit/paint scale.
                            size: p.size,
                            // JS `ghostProjectile` L870: `damage: 0` — the
                            // ghost never hurts; its motion comes from the
                            // record's own `grav` / `drag`.
                            damage: 0.0,
                            grav: p.grav,
                            drag: p.drag,
                            trail: -1.5,
                            trail_every: p.trail_every,
                            trail_radius: 0.45,
                            // JS L871: `seed: Math.random()` — the ghost's
                            // paint is muted, so the seed only feeds this
                            // client's own rng stream.
                            seed: m.projectiles.rng.next_f32(),
                            ghost: true,
                            // Task 30 §4 (net layer) realises blast/pellet
                            // ghosts; the sim-side ghost stays a plain round.
                            blast: false,
                            pellet: None,
                            // The `['p']` record carries no burst override.
                            burst: None,
                            // JS `ghostProjectile` (netmatch.js L868-871)
                            // carries neither `belowFalloff` nor `volley` —
                            // the `['p']` record has no such fields and the
                            // ghost never hurts (`damage: 0`).
                            below_falloff: 0.0,
                            below_falloff_max: 0.0,
                            volley: None,
                            // Task 32: the `['p']` record carries no drop
                            // falloff / delay — a ghost round is a plain one.
                            dmg_far: None,
                            delay: 0.0,
                            // JS `_new` (L851) leaves `sp` undefined even
                            // for ghosts — the L1570 lazy stamp reads this
                            // screen's actor on the first update tick.
                            sp: None,
                        });
                    }
                    TickEvent::Trigger(_) => {
                        // JS L478-483: the L480 guard gates `_netTrig` +
                        // `_trigSideEffects` — character animation and loop
                        // presentation. The sim has no character layer to
                        // animate and Spritzer has no charge/spin/roll
                        // loops, so the record is consumed here; the guard's
                        // outcome cannot touch sim state either way.
                    }
                    TickEvent::Event(e) => {
                        self.play_event(m, t, &e);
                    }
                    TickEvent::Kit(k) => {
                        // JS `_play` 'k' (L486-492): resolve the owner,
                        // require a *remote* actor, hand the payload to the
                        // kit's `ghost` — brolly spawns a solid sliding ghost
                        // canopy (`brolly.js ghost` L650-653 → `addLaunched`
                        // with `ghost = true`, L276-288: blocks shots and
                        // holds players, never paints — the JS `mute++`
                        // wrapper L490 is the `ghost` flag in
                        // `LaunchedCanopy::update`). Kinds without a sim-side
                        // kit layer are ignored verbatim (JS `!K || !K.ghost`
                        // breaks — the decode-tolerance Non-Goal clause).
                        let Some(i) = self.idx_of_nid(k.nid) else {
                            continue;
                        };
                        if !self.remote[i] {
                            continue;
                        }
                        // Numeric view of the mixed payload (kit records are
                        // all-`Num` on the wire; the `sp` channel's strings
                        // are read from `k.data` directly).
                        let dnum: Vec<f32> = k.data.iter().map(|v| v.num_or(0.0)).collect();
                        match k.kind.as_str() {
                            // JS `ghost` guards `Array.isArray(d)` (L651); a
                            // short payload would pack `undefined` → NaN —
                            // the sim drops the malformed record instead.
                            "brolly" => {
                                if k.data.len() < 7 {
                                    continue;
                                }
                                let d = &dnum;
                                // JS L652: `_gd.set(d[3], 0, d[4]).normalize()`.
                                let dir = vec3(d[3], 0.0, d[4]).normalize_or(Vec3::Z);
                                m.launched.push(LaunchedCanopy::new(
                                    m.actors[i].team,
                                    m.actors[i].slot,
                                    vec3(d[0], d[1], d[2]),
                                    dir,
                                    d[5],
                                    w.brolly.canopy_hp as f32,
                                    w.brolly.launch_life,
                                    d[6],
                                    true,
                                ));
                            }
                            // JS `blade.js ghost(a, d)` L267-270: a remote
                            // player's ink wave — it moves and dies for the
                            // eye only (Task 32 §7).
                            "blade" => {
                                if k.data.len() < 6 {
                                    continue;
                                }
                                let d = &dnum;
                                let pos = vec3(d[0], d[1], d[2]);
                                // JS L269: `_gd.set(d[3], d[4], d[5]).normalize()`.
                                let dir = vec3(d[3], d[4], d[5]).normalize_or(Vec3::Z);
                                m.waves.push(BladeWave::new(
                                    m.actors[i].slot,
                                    m.actors[i].team,
                                    pos,
                                    dir,
                                    true,
                                ));
                            }
                            // JS `bow.js ghost(a, d)` L300-305: `[m×3, dir×3,
                            // c, air]` → the three-arrow volley with the
                            // `ghost` flag (arrows neither paint nor hurt).
                            "bow" => {
                                if k.data.len() < 8 {
                                    continue;
                                }
                                let d = &dnum;
                                let s = bow::bow_shot(d[6], &w.bow);
                                let pos = vec3(d[0], d[1], d[2]);
                                // JS L303: `_gd.set(d[3], d[4], d[5]).normalize()`.
                                let dir = vec3(d[3], d[4], d[5]).normalize_or(Vec3::Z);
                                bow::volley(
                                    &m.actors[i],
                                    &s,
                                    pos,
                                    dir,
                                    d[7] != 0.0,
                                    true,
                                    &mut m.arrows,
                                    &mut m.projectiles,
                                );
                            }
                            // JS `mitts.js ghost(a, d)` L863-870: only the
                            // `0` (fist spawn) variant has a sim-side object;
                            // `1` (land splash) / `2` (cling tick) are FX the
                            // sim does not model — consumed and ignored.
                            "mitts" => {
                                let d = &dnum;
                                if d.len() < 8 || d[0] != 0.0 {
                                    continue;
                                }
                                let pos = vec3(d[1], d[2], d[3]);
                                // JS L866: `spawnFist(…, _dir.set(d[4], d[5], d[6]),
                                // true)` — the sender's aim dir is copied as-is
                                // (`spawnFist` L132), never re-normalised here.
                                let dir = vec3(d[4], d[5], d[6]);
                                m.fists.push(mitts::MittsFist {
                                    owner_slot: m.actors[i].slot,
                                    team: m.actors[i].team,
                                    pos,
                                    prev: pos,
                                    vel: dir * w.mitts.fist_speed,
                                    dir,
                                    age: 0.0,
                                    life: w.mitts.fist_range / w.mitts.fist_speed,
                                    hand: d[7] as i32,
                                    seed: m.projectiles.rng.next_f32(),
                                    trail: -0.8,
                                    ghost: true,
                                });
                            }
                            // JS `SUBS.netGhost(a, d)` (subs.js L192-212,
                            // reached from `_play` 'k' L486-492): the throw /
                            // place / end ghost of a remote player's device.
                            // The wire `data[2]` kind string was lifted to
                            // `k.sub_kind` at decode (the numeric vector
                            // keeps the JS index layout); kit kinds and the
                            // `bomb` kind are ignored inside `net_ghost`
                            // (D-T34-9 / the bomb rides `['b']`).
                            "subs" => {
                                let Some(sk) = k.sub_kind.as_deref() else {
                                    continue;
                                };
                                let ps_time = m.projectiles.time;
                                m.subs.net_ghost(
                                    i,
                                    k.nid,
                                    sk,
                                    &dnum,
                                    &self.nids,
                                    sim.collision,
                                    &mut m.actors,
                                    &mut m.paint,
                                    &mut m.projectiles,
                                    t,
                                    sb,
                                    Some(&self.remote).map(|v| &**v),
                                    ps_time,
                                );
                            }
                            // JS `_play` 'k' → `KIT_GHOSTS.sp.ghost` =
                            // `specials.js netGhost(a, d)` (L267-276): the
                            // owner's special lifecycle records drive the
                            // proxy's ghost active and world-object list.
                            // The payload is mixed `KitVal` (op / gid numbers
                            // plus the string kind / reason slots).
                            "sp" => {
                                let ps_time = m.projectiles.time;
                                let mut ctx = crate::special::world::WorldCtx {
                                    actors: &mut m.actors,
                                    collision: sim.collision,
                                    paint: &mut m.paint,
                                    rng: &mut m.rng,
                                    subs: &mut m.subs,
                                    sb,
                                    t,
                                    events: m.projectiles.events_mut(),
                                    remote: Some(&self.remote).map(|v| &**v),
                                    time: ps_time,
                                };
                                crate::special::net_ghost(i, &k.data, &mut m.specials, &mut ctx);
                                // JS specials.js L2051: `GHOST.sonar.start`
                                // *is* `IMPL.sonar.start` — a ghost cast
                                // reveals this screen's enemies too ("the
                                // reveal is everyone's"). The `revealed`
                                // flag on the kind state makes it fire once
                                // per cast even if the record replays.
                                let needs_reveal =
                                    m.actors[i].special_active.as_ref().is_some_and(|s| {
                                        s.ghost && s.kind == "sonar" && !s.sonar_revealed()
                                    });
                                if needs_reveal {
                                    crate::special::sonar::apply_reveal(i, &mut m.actors);
                                }
                            }
                            // Batch two: the five kit subs ride `['k', nid,
                            // <kit id>, [op, gid, …]]` (kits/registry.js) —
                            // the wire kind IS the kit id and there is no
                            // `sub_kind` payload. `net_ghost` routes on the
                            // kind and translates the wire nids in the lock
                            // / walk / beam records via `self.nids`.
                            "shaker" | "waddle" | "torpedo" | "tracer" | "boomerang" => {
                                let ps_time = m.projectiles.time;
                                m.subs.net_ghost(
                                    i,
                                    k.nid,
                                    &k.kind,
                                    &dnum,
                                    &self.nids,
                                    sim.collision,
                                    &mut m.actors,
                                    &mut m.paint,
                                    &mut m.projectiles,
                                    t,
                                    sb,
                                    Some(&self.remote).map(|v| &**v),
                                    ps_time,
                                );
                            }
                            _ => {}
                        }
                    }
                    // JS `_play` 'b' (L477): `ghostBomb(a, e[3..9])` — the
                    // visual copy of a remote player's splat bomb. JS has no
                    // remote guard on `'b'` (unlike `'k'`): the record lands
                    // on the owner's timeline, and a sender never replays
                    // its own.
                    TickEvent::Bomb(b) => {
                        // JS `ghostBomb` L879 branches on `kind === 'storm'`
                        // (ghost throwStorm + ghost cloud); both kinds ride
                        // the same record shape and `ghost_bomb` builds the
                        // matching bomb (Task 35 batch 2, D-T34-1 retired).
                        if b.kind != "bomb" && b.kind != "storm" {
                            continue;
                        }
                        let Some(i) = self.idx_of_nid(b.nid) else {
                            continue;
                        };
                        m.projectiles.ghost_bomb(
                            b.kind,
                            vec3(b.pos[0], b.pos[1], b.pos[2]),
                            vec3(b.vel[0], b.vel[1], b.vel[2]),
                            m.actors[i].slot,
                            m.actors[i].team,
                            // JS `ghostBomb` L879 replays `throwBomb(a)` with
                            // this screen's actor — `sp` reads its live
                            // `specialActive` (weapons.js L1461).
                            m.actors[i].special_active.is_some(),
                        );
                    }
                    // JS `_play` 'z' (L495): `G.match.zones?.netEvent(e[1])`
                    // — the host's rules records replay on the follower's
                    // own ZoneControl (Task 36 batch 3). A `['ze']` that
                    // decides the match mirrors JS `_end` → `match.endZones`
                    // (zones.js L367-368); the guest flood paints into the
                    // local grid exactly like the host's did.
                    TickEvent::Zone(z) => {
                        let Some(zc) = m.zones.as_mut() else { continue };
                        if let Some((winner, reason)) = zc.net_event(
                            &z.payload,
                            &mut m.paint,
                            sim.collision,
                            m.time,
                            m.duration,
                            &mut m.rng,
                        ) {
                            m.end_zones(winner, &reason);
                        }
                    }
                    // Task 41 batch 2: the boss codec decodes `bm`/`bc`, but
                    // the Rust sim does not run a boss (Boss Battle sim is a
                    // later batch) — the records are acknowledged and dropped,
                    // mirroring JS's `this.match?.boss?.…` no-op when no boss
                    // sim exists (netmatch.js L494/L496).
                    TickEvent::BossMove(_) | TickEvent::BossCrabBurst { .. } => {}
                }
            }
        }
    }

    /// JS `_playEvent` (L573-601): unpack the payload, resolve
    /// `e.actor || e.victim`, require a *remote* actor, then dispatch by
    /// name. `splatted` / `respawn` drive the proxy state machines and
    /// *early-return* (L580-581) — they never reach JS's trailing
    /// `emit(name, e)` (L600); the remaining FORWARD names are FX/audio
    /// (out of sim scope) and that trailing emit feeds the app bus the sim
    /// does not have.
    fn play_event(&mut self, m: &mut Match, t: &PlayerTuning, e: &EventRecord) {
        let payload = match &e.payload {
            JVal::Obj(pairs) => pairs,
            _ => return,
        };
        let nid_of = |key: &str| -> Option<u8> {
            payload.iter().find(|(k, _)| k == key).and_then(|(_, v)| {
                let JVal::Obj(inner) = v else { return None };
                inner
                    .iter()
                    .find(|(k, _)| k == "n")
                    .and_then(|(_, n)| n.as_f64())
                    .map(|n| n as u8)
            })
        };
        // `const a = e.actor || e.victim` (L576) — JS unpacks the payload
        // first, so an `actor` key whose nid is unknown resolves to `null`
        // and *falls back* to `victim`. Resolve-then-fallback, not
        // key-then-lookup. respawn carries `actor`, splatted carries
        // `victim`.
        let resolve = |key: &str| nid_of(key).and_then(|n| self.idx_of_nid(n));
        let Some(ai) = resolve("actor").or_else(|| resolve("victim")) else {
            return;
        };
        if !self.remote[ai] {
            return; // JS L577: our own events never replay through us
        }
        match e.name {
            ForwardEvent::Splatted => {
                // JS L580 → `_remoteSplat(e.victim, e.attacker, e.cause)`
                // (L603-619). The `!victim` half of the L604 guard is the
                // nid resolution above (the payload's `victim` is the same
                // actor as `e.victim` for a splat record).
                if self
                    .playback
                    .remote_splat(ai, &mut m.actors[ai], &mut m.runners[ai], t)
                    && let Some(at_nid) = nid_of("attacker")
                    && let Some(ti) = self.idx_of_nid(at_nid)
                {
                    // JS L613: `attacker.stats.splats++` — *any* attacker,
                    // local or proxy. The kill itself is only decided on
                    // the victim's owner (where the victim is local and
                    // this event never replays), so each client counts the
                    // credit exactly once.
                    m.actors[ti].splats += 1;
                }
            }
            ForwardEvent::Respawn => {
                // JS L581 → `_remoteRespawn(a)` (L623-627).
                self.playback.remote_respawn(ai, &mut m.actors[ai], t);
            }
            // FX/audio-only forwards (jump / superjump / special / dodge /
            // weapon:fire — the ghost round itself rode the `'p'` record).
            _ => {}
        }
    }

    /// JS `_sendTick` (L190-203): pack every **owned** actor
    /// (`if (!x.remote)`, L192), stamp with the sender clock, ride the
    /// pending event timeline, and for the host the 0.5 s clock snapshot
    /// `c = [state, r2(time)]` (L197-200).
    fn send_tick(&mut self, m: &Match, tr: &mut impl Transport) {
        let ts = wire::r3(self.now) as f32;
        let mut actors = Vec::new();
        for (i, (a, wr)) in m.actors.iter().zip(&m.runners).enumerate() {
            if self.remote[i] {
                continue;
            }
            actors.push((
                self.nids[i],
                ActorSample::from_actor(a, wr, m.kinds[i], a.net_tp, ts),
            ));
        }
        let clock = if self.is_host() {
            self.clock_t -= TICK_DT;
            if self.clock_t <= 0.0 {
                self.clock_t = CLOCK_PERIOD;
                Some((phase_str(m.phase), m.time))
            } else {
                None
            }
        } else {
            None
        };
        let tick = Tick {
            ts,
            actors,
            events: std::mem::take(&mut self.out),
            // The Rust host does not run a boss sim yet (Task 41 batch 2 is
            // codec + fixtures); non-boss ticks never grow the `B` key.
            b: None,
        };
        tr.broadcast(&encode_tick(&tick, clock));
    }

    /// JS `sendHit` (L163-167): `{k:'hit', v, a, d: r2(dmg), w}` to the
    /// victim's owner. `victim.owner === myId` cannot happen through the
    /// mask (a proxy is never ours), but JS guards it anyway — we apply the
    /// damage locally instead (the `'local'` route, weapons.js L1552).
    fn send_hit(
        &self,
        m: &mut Match,
        t: &PlayerTuning,
        tr: &mut impl Transport,
        attacker: (usize, usize),
        victim: (usize, usize),
        damage: f32,
    ) {
        let (Some(vi), Some(ai)) = (
            find_actor(&m.actors, victim.0, victim.1),
            find_actor(&m.actors, attacker.0, attacker.1),
        ) else {
            return;
        };
        let d = wire::r2(wire::dec(damage)) as f32;
        if self.owners[vi] == self.my_id {
            // JS reads the attacker's *current* position in the victim's
            // `damageTaken` hook — on this machine the attacker is a proxy,
            // so its interpolated pos is the faithful stand-in (Task 33 §5).
            let ap = m.actors[ai].pos;
            m.actors[vi].damage(d, Some(attacker), Some(ap), t);
            return;
        }
        let msg = DirectMsg::Hit {
            v: self.nids[vi],
            a: self.nids[ai],
            d,
            w: Some("shot".to_string()),
        };
        tr.send_to(&self.owners[vi], &encode_direct(&msg));
    }

    /// One inbound relay payload (JS `onMessage`, L206-221). `from` is the
    /// sender's session id as the transport reports it.
    pub fn on_message(
        &mut self,
        from: &str,
        payload: &str,
        m: &mut Match,
        t: &PlayerTuning,
    ) -> Option<ReplicationEvent> {
        let v = parse_json(payload)?;
        let k = v.field("k").and_then(JVal::as_str).unwrap_or("");
        match k {
            "t" => self.on_tick(from, &v, m),
            // The victim's owner applies the damage right away — health must
            // be current, so hits never wait on the playback timeline
            // (netmatch.js L162 comment, `_hit` L630-636).
            "hit" => {
                if let Some(DirectMsg::Hit {
                    v: vn, a: an, d, ..
                }) = decode_direct(&v)
                {
                    self.apply_hit(m, t, vn, an, d);
                }
            }
            // JS L210-211: `netHurt` on the device owner's side — `subs`
            // routes to `SUBS.netHurt`, the kit kinds to their own lists
            // (waddle.js L691, torpedo.js `netHurt` / `netHurtArea`).
            // D-T34-5: the wire gid (`src_nid * 100_000 + off`) is
            // converted back to the owner-side local gid
            // (`slot * 100_000 + off`) — the sim keys devices on the
            // roster slot, not the session nid.
            "dh" => {
                if let Some(DirectMsg::DeviceHit { kind, id, d }) = decode_direct(&v)
                    && matches!(
                        kind.as_str(),
                        "subs" | "waddle" | "torpedo" | "torpedoArea" | "sp"
                    )
                {
                    let src_nid = (id / 100_000) as u8;
                    let mut off = id % 100_000;
                    // D-T34-5: the outbound rewrite is `gid % 100_000`, so a
                    // local `off` of 100_000 rides the wire as 0 — map it
                    // back before rebuilding the owner-side gid, or such a
                    // hit would silently miss its device.
                    if off == 0 {
                        off = 100_000;
                    }
                    if let Some(i) = self.idx_of_nid(src_nid) {
                        let gid = (m.actors[i].slot as u32) * 100_000 + off;
                        if kind == "sp" {
                            // Task 35 batch 3: a ghost bubble hurt on its
                            // owner's registry — queued for the next
                            // `update` (the registry needs the full world
                            // context, which `on_message` does not carry).
                            self.pending_sp_hurt.push((gid, d));
                        } else {
                            m.subs.net_hurt(&kind, gid, d);
                        }
                    }
                }
            }
            // M2 turf-only: no boss — ignored stub (JS L215 runs
            // `boss.remoteHit`; spec 收口).
            "bhit" => {}
            "st" => {
                if from == self.host_id
                    && let Some(HostMsg::State { phase, t: time }) = decode_host(&v)
                {
                    self.host_state(m, &phase, time);
                }
            }
            "res" => {
                if from == self.host_id
                    && let Some(HostMsg::Result {
                        coverage,
                        winner,
                        stats,
                        zones,
                        ..
                    }) = decode_host(&v)
                {
                    self.host_result(m, coverage, winner, &stats, zones.as_ref());
                }
            }
            "end" => {
                if from == self.host_id {
                    return Some(ReplicationEvent::MatchEnd);
                }
            }
            // `{k:'own'}`: JS `_ownership` is an empty reserved stub (L709)
            // and the host never sends it — receive and ignore.
            "own" => {}
            _ => {}
        }
        None
    }

    /// JS `_tick` (L229-263): feed the sender's playback clock (offset
    /// floor, the late/gap windows, `want` — [`Playback::on_tick`]), route
    /// the actor samples into their buffers (the `!a || !a.remote || a.owner
    /// !== from` guard, L248, and the monotonic-`t` guard + 40-capacity
    /// trim, L251-253), queue the tick's events on the sender's timeline
    /// (L256, played by Task 23), and steer the guest clock off the host's
    /// `c` (L257).
    fn on_tick(&mut self, from: &str, v: &JVal, m: &mut Match) {
        let (tick, clock) = decode_tick(v);
        self.playback.on_tick(from, tick.ts as f64, self.now);
        self.playback.push_events(from, tick.events);
        if let Some((state, time)) = clock
            && from == self.host_id
        {
            self.host_clock(m, &state, time);
        }
        for (nid, s) in &tick.actors {
            let Some(i) = self.idx_of_nid(*nid) else {
                continue;
            };
            if !self.remote[i] || self.owners[i] != from {
                continue;
            }
            self.playback.push_sample(i, s);
        }
    }

    /// JS `_hostClock` (L639-643): guests only, both sides `playing`, off by
    /// more than 0.2 s → nudge halfway. The clock keeps running locally
    /// between nudges (the sim's `step_masked` decrements it like JS's).
    fn host_clock(&self, m: &mut Match, state: &str, time: f32) {
        if self.is_host() || m.phase != Phase::Active || state != "playing" {
            return;
        }
        if (m.time - time).abs() > 0.2 {
            m.time += (time - m.time) * 0.5;
        }
    }

    /// JS `_hostState` (L644-649): guests only; hard-set the clock **only
    /// when the frame carries a numeric `t`** (`typeof d.t === 'number'`,
    /// L647); re-enter the phase on change, never *into* `judge` (that comes
    /// with `res`).
    fn host_state(&self, m: &mut Match, state: &str, time: Option<f32>) {
        if self.is_host() {
            return;
        }
        if let Some(time) = time {
            m.time = time;
        }
        let Some(p) = phase_of(state) else { return };
        if p != m.phase && p != Phase::End {
            m.set_phase(p);
        }
    }

    /// JS `_result` (L656-665): guests only; per-nid stat overwrite; the
    /// turf clock is pinned to 0; the host's count becomes the local result;
    /// straight into `judge` (bypassing the `st` guard — this *is* the judge
    /// transition). With `zones` the settlement is built verbatim from the
    /// wire extras (JS L662: `counts: d.zc, penalty: d.zp, reason: d.zr,
    /// overtime: !!d.zo, log: d.zl || []`) — the host's count, never the
    /// guest's replayed rules state.
    fn host_result(
        &self,
        m: &mut Match,
        coverage: [f32; 2],
        winner: u8,
        stats: &[ActorStatsRow],
        zones: Option<&ZonesResult>,
    ) {
        if self.is_host() {
            return;
        }
        for row in stats {
            if let Some(i) = self.idx_of_nid(row.nid) {
                let a = &mut m.actors[i];
                a.turf = row.turf as f32;
                a.splats = row.splats;
                a.deaths = row.deaths;
            }
        }
        // The host's stat rows are the authority for the whole point line
        // (a guest only simulates its own actors — see module docs).
        let mut points = [0.0f32; 2];
        for row in stats {
            if let Some(i) = self.idx_of_nid(row.nid) {
                points[m.actors[i].team] += row.turf as f32;
            }
        }
        points[0] *= m.points_per_m2;
        points[1] *= m.points_per_m2;
        m.time = 0.0;
        m.result = Some(MatchResult {
            coverage,
            winner: winner as usize,
            points: if zones.is_some() {
                [0.0, 0.0] // JS zones result has no points line (match.js L256-257)
            } else {
                points
            },
            elapsed: m.duration,
            zones: zones.map(|z| ZoneOutcome {
                winner: winner as usize,
                reason: z.reason.clone(),
                counts: z.counts,
                penalty: z.penalty,
                overtime: z.overtime,
                log: z.log.clone(),
            }),
            boss: None,
        });
        m.set_phase(Phase::End);
    }

    /// JS `_hit` (L630-636) + `applyHit` local route (weapons.js L1545-1552):
    /// the victim must exist, be ours (`!remote`), be alive; the attacker
    /// must exist and be on the other team; then `victim.damage(d, attacker)`.
    /// The `w` payload only drives JS FX/audio (M1 has no kit modifiers).
    fn apply_hit(&self, m: &mut Match, t: &PlayerTuning, victim_nid: u8, atk_nid: u8, dmg: f32) {
        let (Some(vi), Some(ai)) = (self.idx_of_nid(victim_nid), self.idx_of_nid(atk_nid)) else {
            return;
        };
        if self.remote[vi] || !m.actors[vi].alive || m.actors[ai].team == m.actors[vi].team {
            return;
        }
        let atk = (m.actors[ai].team, m.actors[ai].slot);
        let ap = m.actors[ai].pos;
        m.actors[vi].damage(dmg, Some(atk), Some(ap), t);
    }

    /// JS `sendResult` (L650-655): the host's final count, broadcast. The JS
    /// call site is `_judge` (match.js L271) — the app calls this when the
    /// host's match settles; the stat rows ride in roster order. A zones
    /// result carries the `zc/zp/zr/zo/zl` extras (JS L652-653).
    pub fn send_result(&self, m: &Match, tr: &mut impl Transport) {
        if !self.is_host() {
            return;
        }
        let Some(r) = &m.result else { return };
        let stats = m
            .actors
            .iter()
            .enumerate()
            .map(|(i, a)| ActorStatsRow {
                nid: self.nids[i],
                turf: wire::round_int(a.turf) as i32,
                splats: a.splats,
                deaths: a.deaths,
                // No boss sim in the Rust host yet (Task 41 batch 2 is codec
                // + fixtures): JS L654 writes `bossDmg || 0` / `weakHits || 0`
                // for turf rows, so 0/0 reproduces the pre-Task-41 bytes.
                boss_dmg: 0,
                weak_hits: 0,
            })
            .collect();
        let msg = HostMsg::Result {
            coverage: r.coverage,
            winner: r.winner as u8,
            stats,
            zones: r.zones.as_ref().map(|z| ZonesResult {
                counts: z.counts,
                penalty: z.penalty,
                reason: z.reason.clone(),
                overtime: z.overtime,
                log: z.log.clone(),
            }),
            bo: None,
        };
        tr.broadcast(&encode_host(&msg));
    }

    /// JS `sendEnd` (L666): results shown, everyone back to the lobby.
    pub fn send_end(&self, tr: &mut impl Transport) {
        if self.is_host() {
            tr.broadcast(&encode_host(&HostMsg::End));
        }
    }

    /// JS `onLeave` (L671-684): a member dropped mid-match. Their squidkids
    /// carry on as bots owned by the (possibly new) host — or, on a
    /// humans-only stage, are removed outright (`_remove`, L686-690).
    /// `host_changed` is the leave frame's re-election flag; the app must
    /// have applied [`Replication::set_host`] first. `sim` / `t` drive the
    /// mid-respawn adopt branch (JS L704 `a.respawn()`).
    #[allow(clippy::too_many_arguments)] // JS `onLeave(id, hostChanged)` + the match context it threads
    pub fn on_leave(
        &mut self,
        id: &str,
        host_changed: bool,
        m: &mut Match,
        diff: &Difficulty,
        seed: u64,
        sim: &SimWorld,
        t: &PlayerTuning,
    ) {
        let new_owner = self.host_id.clone();
        // Iterate high→low: the noBots path prunes the parallel arrays.
        for i in (0..self.owners.len()).rev() {
            if self.owners[i] != id {
                continue;
            }
            if self.no_bots {
                self.remove_at(m, i);
                continue;
            }
            // JS L677-679: the squidkid now belongs to the host.
            self.owners[i] = new_owner.clone();
            if new_owner == self.my_id {
                self.adopt(i, m, diff, seed, sim, t);
            } else {
                // Same squidkid, new sender: drop the stale buffer and glide
                // onto the new path (`buf.length = 0; handoff = true`, L679).
                self.playback.handoff(i);
            }
        }
        // JS L681: the new host stops following and re-arms its clock feed.
        if host_changed && self.is_host() {
            m.follower = false;
            self.clock_t = 0.0;
            self.adopted_host = true;
        }
    }

    /// JS `_adopt` (L692-707): continue from exactly where it was drawn —
    /// no pop. `remote = false`, a bot brain takes over, the weapon runner
    /// resets, and the teleport counter continues from what everyone has
    /// seen (`a.netTp = a.net.tp || 0`, L702 — the *shown* counter,
    /// [`Playback::adopt_reset`]). JS L704: a proxy mid-respawn finishes the
    /// respawn locally (the pad drop, the `net_tp` bump); otherwise the
    /// actor simply keeps its drawn position (L705, `a.pos` already holds
    /// path + correction from [`Playback::apply`]).
    fn adopt(
        &mut self,
        i: usize,
        m: &mut Match,
        diff: &Difficulty,
        seed: u64,
        sim: &SimWorld,
        t: &PlayerTuning,
    ) {
        self.remote[i] = false;
        let pending = self.playback.spawn_pending(i);
        let tp = self.playback.adopt_reset(i);
        m.actors[i].net_tp = tp;
        if m.actors[i].alive && pending {
            m.actors[i].respawn(sim, t);
        }
        self.bots[i] = Some(Bot::new(&m.actors[i], diff, seed, i));
        m.runners[i].reset();
        // JS zeroes the intent and clears superJump/specialActive — the sim
        // actor has no intent field (inputs come per-frame) and neither
        // state exists in M1.
    }

    /// Step every owned bot brain (JS `if (a.bot) a.bot.update(dt)`,
    /// match.js L222-224, plus the L224 intent zeroing outside `playing`),
    /// writing into `inputs` for [`Match::step_masked`]. The autopilot
    /// snapshot pattern (Task 9): brains read a roster snapshot taken before
    /// any bot steps.
    #[allow(clippy::too_many_arguments)] // Task 38: the weapon set + range table the brains read
    pub fn drive_bots(
        &mut self,
        m: &mut Match,
        nav: &NavGraph,
        world: &SimWorld,
        t: &PlayerTuning,
        ws: &WeaponSet,
        ranges: &HashMap<String, f32>,
        inputs: &mut [ActorInput],
    ) {
        if !self.bots.iter().any(|b| b.is_some()) {
            return;
        }
        let live = m.phase == Phase::Active;
        let snap = m.actors.clone();
        let mut mate_goals = vec![None; m.actors.len()];
        for (i, b) in self.bots.iter().enumerate() {
            if let Some(b) = b {
                mate_goals[i] = b.goal_node();
            }
        }
        // Remote slots have no local brain — their target memory reports
        // `None` (blade targetBias deviation, module docs).
        let mate_minds = self
            .bots
            .iter()
            .map(|b| b.as_ref().map(|bb| bb.mind()))
            .collect::<Vec<_>>();
        // Task 38 batch 3: drive the shared Zone Control plan once per frame
        // before the brains (JS `zonePlan().tick()` rides the match update);
        // only locally-driven bots appear in `is_bot`.
        let mut zone_plan = m.zone_plan.take();
        if let Some(zctl) = m.zones.as_ref() {
            let mut p = zone_plan
                .take()
                .unwrap_or_else(|| crate::bot::zones::ZonePlan::build(zctl, nav, &m.paint, world));
            let is_bot = self
                .bots
                .iter()
                .enumerate()
                .map(|(i, b)| b.is_some() && !self.remote[i])
                .collect::<Vec<_>>();
            p.tick(zctl, &m.paint, world, &snap, &m.kinds, &is_bot);
            zone_plan = Some(p);
        }
        for (i, slot) in self.bots.iter_mut().enumerate() {
            // Only owned actors ever hold a brain (module docs).
            if self.remote[i] {
                continue;
            }
            let Some(b) = slot else {
                continue;
            };
            if live {
                let ctx = BotCtx {
                    actors: &snap,
                    nav,
                    paint: &m.paint,
                    world,
                    t,
                    weapons: ws,
                    weapon_range: ranges,
                    kinds: &m.kinds,
                    runners: &m.runners,
                    mate_goals: &mate_goals,
                    subs: &m.subs,
                    mate_minds: &mate_minds,
                    zones: zone_plan.as_ref(),
                    zctl: m.zones.as_ref(),
                    boss: None,
                };
                inputs[i] = b.step(FIXED_DT, &mut m.actors[i], &ctx);
            } else {
                inputs[i] = b.hold(&m.actors[i]);
            }
        }
        m.zone_plan = zone_plan;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;
    use crate::geometry::StageLayout;
    use crate::match_::{FINISH_TIME, INTRO_TIME};
    use crate::net::session::RosterEntry;
    use crate::net::transport::{FakeRelay, FakeTransport, TransportEvent};
    use crate::tuning::Tuning;
    use crate::weapon::WeaponRunner;
    use crate::weapon::brolly::CanopyState;
    use glam::vec3;

    struct Fix {
        layout: StageLayout,
        world: CollisionWorld,
        tuning: Tuning,
    }

    fn fix() -> Fix {
        Fix {
            layout: crate::embedded_tidewater(),
            world: CollisionWorld::tidewater(),
            tuning: crate::embedded_tuning(),
        }
    }

    impl Fix {
        fn sim(&self) -> SimWorld<'_> {
            SimWorld::new(&self.world, &self.layout)
        }
        fn nav(&self) -> NavGraph {
            NavGraph::new(&self.world, &self.layout, &self.tuning.player)
        }
    }

    fn entry(nid: u8, owner: &str, bot: bool, team: u8, slot: u8) -> RosterEntry {
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

    fn cfg(roster: Vec<RosterEntry>, map: &str, host: &str) -> StartCfg {
        StartCfg {
            roster,
            map: map.into(),
            time: "day".into(),
            duration: 120,
            difficulty: "normal".into(),
            palette: 0,
            mode: "turf".into(),
            host: host.into(),
            id: "m1".into(),
        }
    }

    /// 1v1 + one host-owned bot, actors in roster order:
    /// `[H(nid0, team0), G(nid1, team1), B(nid2, team1 slot1)]`.
    fn roster111(h: &str, g: &str) -> Vec<RosterEntry> {
        vec![
            entry(0, h, false, 0, 0),
            entry(1, g, false, 1, 0),
            entry(2, h, true, 1, 1),
        ]
    }

    /// A 3-actor match matching [`roster111`]: [`Match::with_roster`] with
    /// team_size 1 plus the bot appended on the team-1 ring.
    fn match111(f: &Fix, seed: u64, duration: f32) -> Match {
        let sim = f.sim();
        let t = &f.tuning.player;
        let mut m = Match::with_roster(duration, seed, &sim, t, &f.tuning.match_config, 1);
        let mut a = Actor::new(1, 1, t);
        let pad = sim.spawn_pads[1];
        let ang = (1.0 / 4.0) * std::f32::consts::TAU + 0.6;
        a.spawn_at(
            vec3(pad.x + ang.cos() * 1.2, pad.y, pad.z + ang.sin() * 1.2),
            std::f32::consts::PI,
            &sim,
            t,
        );
        a.invuln = 0.0;
        m.actors.push(a);
        m.runners.push(WeaponRunner::new());
        m.kinds.push(crate::weapon::WeaponKind::Shooter);
        m
    }

    /// A [`Transport`] that records framed sends (envelope-split), so the
    /// tick-content checks need no relay semantics.
    #[derive(Default)]
    struct Rec {
        out: Vec<(Option<String>, String)>,
    }
    impl Rec {
        /// The first recorded send whose JSON carries message key `k`.
        fn find(&self, k: &str) -> &(Option<String>, String) {
            self.out
                .iter()
                .find(|(_, p)| {
                    parse_json(p)
                        .and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                        == Some(k.to_string())
                })
                .unwrap_or_else(|| panic!("a {k} frame was recorded"))
        }
    }
    impl Transport for Rec {
        fn connect(&mut self, _code: &str, _name: &str, _create: bool) {}
        fn poll_events(&mut self) -> Vec<TransportEvent> {
            Vec::new()
        }
        fn send_raw(&mut self, s: &str) -> bool {
            if let Some(rest) = s.strip_prefix("b|") {
                self.out.push((None, rest.to_string()));
            } else if let Some(rest) = s.strip_prefix("s|")
                && let Some((to, body)) = rest.split_once('|')
            {
                self.out.push((Some(to.to_string()), body.to_string()));
            } else {
                self.out.push((None, s.to_string()));
            }
            true
        }
        fn is_open(&self) -> bool {
            true
        }
        fn rtt(&self) -> f64 {
            0.0
        }
        fn close(&mut self) {}
    }

    fn ep(relay: &FakeRelay, seed: u64) -> FakeTransport {
        relay.transport(None, seed)
    }

    fn drain_msgs(tr: &mut FakeTransport) -> Vec<(SessionId, String)> {
        tr.poll_events()
            .into_iter()
            .filter_map(|e| match e {
                TransportEvent::Message { from, payload } => Some((from, payload)),
                _ => None,
            })
            .collect()
    }

    /// Both endpoints on one relay, handshaked, all chatter drained.
    fn pair(relay: &FakeRelay) -> (FakeTransport, FakeTransport) {
        let mut a = ep(relay, 1);
        let mut b = ep(relay, 2);
        a.connect("ROOM", "a", true);
        b.connect("ROOM", "b", false);
        for _ in 0..20 {
            a.advance(1.0);
            b.advance(0.0);
        }
        drain_msgs(&mut a);
        drain_msgs(&mut b);
        (a, b)
    }

    // ---------------------------------------------------------------- phase map
    #[test]
    fn phase_strings_match_js_states() {
        // netmatch.js L80/L648: the wire carries match.js state names.
        assert_eq!(phase_str(Phase::Init), "init");
        assert_eq!(phase_str(Phase::Intro), "intro");
        assert_eq!(phase_str(Phase::Active), "playing");
        assert_eq!(phase_str(Phase::Finish), "finish");
        assert_eq!(phase_str(Phase::End), "judge");
        for p in [
            Phase::Init,
            Phase::Intro,
            Phase::Active,
            Phase::Finish,
            Phase::End,
        ] {
            assert_eq!(phase_of(phase_str(p)), Some(p));
        }
        assert_eq!(phase_of("nonsense"), None);
    }

    // ------------------------------------------------------------------ TR-21.1
    #[test]
    fn tr21_1_owner_proxy_split_and_nid_routing() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);

        let mut mh = match111(&f, 7, 60.0);
        let mut mg = match111(&f, 7, 60.0);
        let mut rh = Replication::new(&cfg, h, h);
        let mut rg = Replication::new(&cfg, g, h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 7);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        // Mask: each client owns exactly its roster actors; the host owns
        // the bot; `follower` only on the guest (match.js L106).
        assert_eq!(rh.remote_mask(), [false, true, false].as_slice());
        assert_eq!(rg.remote_mask(), [true, false, true].as_slice());
        assert!(!mh.follower);
        assert!(mg.follower, "guests never call time / judge");

        // Run both to Active.
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        // A proxy runs no physics and no input: shove the guest's own actor
        // with input and confirm the proxy of host-actor-0 never moves.
        let pinned = mg.actors[0].pos;
        let mut inputs = vec![ActorInput::default(); 3];
        inputs[0].move_dir = vec3(0.0, 0.0, 1.0);
        inputs[1].move_dir = vec3(1.0, 0.0, 0.0);
        for _ in 0..30 {
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
        }
        assert_eq!(
            mg.actors[0].pos, pinned,
            "a proxy must not integrate input/physics"
        );
        assert!(
            mg.actors[1].pos != pinned,
            "the owned actor must run the sim"
        );

        // Tick content: the host streams only its owned actors (L192) and
        // rides the clock snapshot (L197-200).
        let mut rec = Rec::default();
        rh.update(TICK_DT, &mut mh, &sim, t, w, sb, &mut rec);
        let (to, payload) = rec.find("t").clone();
        assert_eq!(to, None, "ticks are broadcast");
        let v = parse_json(&payload).unwrap();
        let (tick, clock) = decode_tick(&v);
        assert_eq!(tick.actors.len(), 2, "owned actors only: nid0 + bot nid2");
        assert_eq!(tick.actors[0].0, 0);
        assert_eq!(tick.actors[1].0, 2);
        assert!(clock.is_some(), "the host tick carries `c`");

        // A spoofed tick from the wrong sender must not move the proxy
        // (L248 `a.owner !== from`), and a non-newer ts is dropped (L251).
        let before = mg.actors[0].pos;
        let mut fake = tick.clone();
        fake.ts += 1.0;
        for (_, s) in &mut fake.actors {
            s.x += 50.0;
        }
        rg.on_message("IMPOSTOR", &encode_tick(&fake, None), &mut mg, t);
        assert_eq!(mg.actors[0].pos, before, "owner mismatch drops the sample");

        let mut good = fake.clone();
        good.ts += 1.0;
        rg.on_message(h, &encode_tick(&good, None), &mut mg, t);
        // Task 22: the sample lands in the playback buffer — advance the
        // clock one frame so the path is sampled and applied (tr sits
        // behind the newest sample → the Early hold shows it verbatim).
        rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
        assert_eq!(
            mg.actors[0].pos.x, good.actors[0].1.x,
            "the owner's word lands"
        );

        // Same ts again (a reordered retransmission): rejected.
        let mut stale = good.clone();
        stale.ts += 1.0;
        for (_, s) in &mut stale.actors {
            s.x += 100.0;
        }
        // `stale` reuses `good`'s sample timestamps? No — decode stamps
        // `t` from the tick ts, so bump nothing: same ts as `good`.
        stale.ts = good.ts;
        rg.on_message(h, &encode_tick(&stale, None), &mut mg, t);
        rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
        assert_eq!(
            mg.actors[0].pos.x, good.actors[0].1.x,
            "a non-newer sample is dropped (L251)"
        );
    }

    // ------------------------------------------------------------------ TR-21.2
    #[test]
    fn tr21_2_hit_channel_end_to_end() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        // Real relay ids — `send_to` routes by them (FakeRelay.route).
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);

        let mut mh = match111(&f, 0xABCD_1234, 120.0);
        let mut mg = match111(&f, 0xABCD_1234, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        // Both to Active, then pin the victim 3 m down the shooter's aim
        // line (team0 faces +Z from its pad).
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);
        let pad0 = sim.spawn_pads[0];
        let victim_p = vec3(pad0.x, pad0.y, pad0.z + 3.0);
        mh.actors[1].invuln = 0.0;
        mg.actors[1].invuln = 0.0;
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = pad0;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
            a.invuln = 0.0;
        }

        // Hold the trigger on the host until the round hits the proxy.
        let hp_before = mh.actors[1].hp;
        let mut inputs = vec![ActorInput::default(); 3];
        let mut sent = None;
        for _ in 0..(10.0 * 60.0) as usize {
            inputs[0].fire = true;
            // keep the pinned geometry: the guest's ticks would otherwise
            // re-draw the proxy at the guest's own (unmoved) sim position.
            mh.actors[1].pos = victim_p;
            mh.actors[0].pos = pad0;
            mh.actors[0].aim_yaw = 0.0;
            mh.actors[0].aim_pitch = 0.0;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let hit = mh.events.iter().find_map(|e| match e {
                MatchEvent::HitSend { damage, victim, .. } => Some((*victim, *damage)),
                _ => None,
            });
            if let Some((victim, damage)) = hit {
                assert_eq!(victim, (1, 0), "the victim is the guest's actor");
                assert!(damage > 0.0);
                // The proxy took NO local damage — JS 'send' route.
                assert_eq!(mh.actors[1].hp, hp_before, "proxy hp untouched");
                sent = Some(damage);
            }
            // rh.update consumes the HitSend and routes `{k:'hit'}` to G.
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if sent.is_some() {
                assert!(
                    mg.actors[1].hp < hp_before,
                    "the owner applied the damage immediately"
                );
                break;
            }
        }
        assert!(sent.is_some(), "a held trigger must hit the proxy");

        // The owner's chain: keep firing until the hits splat the victim →
        // respawn timer → respawn, tp bumped; the shooter's proxy follows
        // through the owner's ticks. Once the victim is down, stop firing
        // and stop pinning so the respawn is clean.
        let mut saw_respawn = false;
        let mut splatted = false;
        for _ in 0..(t.respawn_time * 60.0) as usize + 600 {
            inputs[0].fire = !splatted;
            if !splatted {
                mh.actors[1].pos = victim_p;
                mh.actors[0].pos = pad0;
                mh.actors[0].aim_yaw = 0.0;
                mh.actors[0].aim_pitch = 0.0;
            }
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if !mg.actors[1].alive {
                splatted = true;
            }
            if splatted && mg.actors[1].alive && mg.actors[1].net_tp >= 1 {
                saw_respawn = true;
                break;
            }
        }
        assert!(saw_respawn, "the victim respawned on the owner");
        assert!(
            mg.actors[1].deaths >= 1,
            "the hit splatted the victim: {:?}",
            mg.kills.last()
        );
        let mut followed = false;
        for _ in 0..120 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            // The guest's respawned actor-1 sample arrives at the HOST inbox.
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            if mh.actors[1].alive && rh.last_tp(1) == 1 && mh.actors[1].hp > 0.0 {
                followed = true;
                break;
            }
        }
        assert!(
            followed,
            "the tp bump and respawn state reached the shooter"
        );
    }

    // ------------------------------------------------------------------ TR-21.3
    #[test]
    fn tr21_3_guest_leave_actor_adopted_as_bot() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh = match111(&f, 5, 120.0);
        let mut rh = Replication::new(&cfg, h, h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 5);
        rh.bind_match(&mut mh);
        assert!(rh.remote_mask()[1], "the guest actor starts as a proxy");

        let nav = f.nav();
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            let mut inputs = vec![ActorInput::default(); 3];
            rh.drive_bots(
                &mut mh,
                &nav,
                &sim,
                t,
                w,
                &f.tuning.weapon_range,
                &mut inputs,
            );
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
        }
        // The leave: the host stays host, so `host_changed = false`.
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 5, &sim, t);
        assert!(
            !rh.remote_mask()[1],
            "the left player's squidkid is adopted"
        );
        assert_eq!(rh.owner_of(1), Some(h));
        assert_eq!(rh.nid_of(1), Some(1), "the ownership mapping keeps the nid");

        // It now simulates: the pinned proxy position must move under the
        // adopted bot.
        let before = mh.actors[1].pos;
        for _ in 0..120 {
            let mut inputs = vec![ActorInput::default(); 3];
            rh.drive_bots(
                &mut mh,
                &nav,
                &sim,
                t,
                w,
                &f.tuning.weapon_range,
                &mut inputs,
            );
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
        }
        assert_ne!(mh.actors[1].pos, before, "the adopted bot lives again");
    }

    #[test]
    fn tr21_3_host_migration_completes_the_match() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let mut obs = ep(&relay, 3);
        obs.connect("ROOM", "obs", false);
        for _ in 0..20 {
            trh.advance(1.0);
            trg.advance(0.0);
            obs.advance(0.0);
        }
        drain_msgs(&mut trh);
        drain_msgs(&mut trg);
        drain_msgs(&mut obs);
        // Real relay ids: the leave/re-election frames and `from == host_id`
        // checks compare against what the relay mints.
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let dur = 6.0;
        let mut mh = match111(&f, 9, dur);
        let mut mg = match111(&f, 9, dur);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 9);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        let nav = f.nav();
        // Intro out on both.
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            let mut ih = vec![ActorInput::default(); 3];
            rh.drive_bots(&mut mh, &nav, &sim, t, w, &f.tuning.weapon_range, &mut ih);
            mh.step_masked(FIXED_DT, &ih, Some(rh.remote_mask()), &sim, t, w, sb, None);
            let mut ig = vec![ActorInput::default(); 3];
            ig[1].move_dir = vec3(0.0, 0.0, -1.0);
            mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        assert_eq!(mg.phase, Phase::Active);

        // The host leaves: the relay re-elects the guest (oldest member).
        trh.simulate_closed("bye");
        let mut host_changed = false;
        let mut left = String::new();
        for _ in 0..20 {
            trg.advance(1.0);
            obs.advance(0.0);
            for e in trg.poll_events() {
                if let TransportEvent::Leave { id, host } = e {
                    left = id;
                    host_changed = host == g;
                }
            }
            if host_changed {
                break;
            }
        }
        assert!(host_changed, "the guest is the new host");
        assert_eq!(left, h);
        rg.set_host(&g);
        rg.on_leave(&h, true, &mut mg, &f.tuning.difficulty.easy, 9, &sim, t);

        // Ownership mapping: the old host's actors (his player + his bot)
        // now belong to the new host and are adopted; the guest's own actor
        // stays local; `follower` clears (L681).
        assert_eq!(rg.owner_of(0), Some(g.as_str()));
        assert_eq!(rg.owner_of(2), Some(g.as_str()));
        assert!(!rg.remote_mask()[0] && !rg.remote_mask()[2], "adopted");
        assert!(!rg.remote_mask()[1], "still mine");
        assert!(!mg.follower, "the new host calls time and judges");

        // Play out the rest of the short match: the new host drives all
        // three actors and settles with a result.
        let max = ((dur + FINISH_TIME) * 60.0) as usize + 120;
        for _ in 0..max {
            let mut ig = vec![ActorInput::default(); 3];
            rg.drive_bots(&mut mg, &nav, &sim, t, w, &f.tuning.weapon_range, &mut ig);
            mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            drain_msgs(&mut trg);
            trg.advance(16.0);
            obs.advance(0.0);
            if mg.phase == Phase::End {
                break;
            }
        }
        assert_eq!(mg.phase, Phase::End, "the migrated host finishes");
        let r = mg.result.as_ref().expect("settled with a result");
        assert!(
            (r.elapsed - dur).abs() < 0.1,
            "clock ran to zero: {}",
            r.elapsed
        );

        // The result broadcast + end frame reach an observer (the same
        // frames every guest applies).
        let mut m3 = match111(&f, 9, dur);
        let mut r3 = Replication::new(&cfg, "OBS", &g);
        r3.bind_match(&mut m3);
        m3.phase = Phase::Finish;
        rg.send_result(&mg, &mut trg);
        trg.advance(16.0);
        obs.advance(0.0);
        let msgs = drain_msgs(&mut obs);
        let res = msgs
            .iter()
            .find(|(_, p)| {
                parse_json(p).and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                    == Some("res".into())
            })
            .expect("sendResult broadcasts");
        assert_eq!(res.0, g);
        r3.on_message(&res.0, &res.1, &mut m3, t);
        assert_eq!(m3.phase, Phase::End, "res carries the guest to judge");
        assert_eq!(
            m3.result.as_ref().expect("result").winner,
            r.winner,
            "the host's count"
        );
        assert_eq!(m3.time, 0.0);

        // `{k:'end'}` → the app seam.
        rg.send_end(&mut trg);
        trg.advance(16.0);
        obs.advance(0.0);
        let end = drain_msgs(&mut obs)
            .into_iter()
            .find(|(_, p)| {
                parse_json(p).and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                    == Some("end".into())
            })
            .expect("sendEnd broadcasts");
        assert_eq!(
            r3.on_message(&end.0, &end.1, &mut m3, t),
            Some(ReplicationEvent::MatchEnd)
        );
    }

    #[test]
    fn tr21_3_no_bots_stage_removes_leavers() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        // Cargo Terminal: humans-only (config.js L524).
        let cfg = cfg(roster111(h, g), "cargo", h);
        let mut mh = match111(&f, 4, 30.0);
        let mut rh = Replication::new(&cfg, h, h);
        rh.bind_match(&mut mh);
        assert_eq!(mh.actors.len(), 3);
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 4, &sim, t);
        // JS `_remove` (L686-690): the squidkid is gone from every screen.
        assert_eq!(mh.actors.len(), 2, "the leaver's actor is removed");
        assert_eq!(rh.nid_of(0), Some(0));
        assert_eq!(rh.nid_of(1), Some(2), "the bot shifted down with it");
        assert_eq!(rh.remote_mask().len(), 2);
        // The match still runs.
        let nav = f.nav();
        let mut inputs = vec![ActorInput::default(); 2];
        for _ in 0..60 {
            rh.drive_bots(
                &mut mh,
                &nav,
                &sim,
                t,
                w,
                &f.tuning.weapon_range,
                &mut inputs,
            );
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
        }
        assert_eq!(mh.actors.len(), 2);
    }

    #[test]
    fn guest_clock_steers_and_follower_gates_hold() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let mut mg = match111(&f, 3, 120.0);
        let cfg = cfg(roster111("HOST", "GUEST"), "tidewater", "HOST");
        let mut rg = Replication::new(&cfg, "GUEST", "HOST");
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Active);
        mg.time = 100.0;
        // |diff| <= 0.2: untouched (L642).
        rg.on_message(
            "HOST",
            &encode_tick(
                &Tick {
                    ts: 1.0,
                    actors: vec![],
                    events: vec![],
                    b: None,
                },
                Some(("playing", 100.1)),
            ),
            &mut mg,
            t,
        );
        assert_eq!(mg.time, 100.0);
        // |diff| > 0.2: nudge halfway (L642).
        rg.on_message(
            "HOST",
            &encode_tick(
                &Tick {
                    ts: 2.0,
                    actors: vec![],
                    events: vec![],
                    b: None,
                },
                Some(("playing", 102.0)),
            ),
            &mut mg,
            t,
        );
        assert!((mg.time - 101.0).abs() < 1e-3, "halfway nudge: {}", mg.time);
        // `st` hard-sets the clock and re-enters the phase (L647-648)…
        rg.on_message(
            "HOST",
            &encode_host(&HostMsg::State {
                phase: "finish".into(),
                t: Some(0.0),
            }),
            &mut mg,
            t,
        );
        assert_eq!(mg.phase, Phase::Finish);
        assert_eq!(mg.time, 0.0);
        // …but a follower never judges on its own (L216): Finish + follower
        // must not settle even past the window.
        for _ in 0..(FINISH_TIME * 60.0) as usize + 10 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Finish, "the follower waits for res");
        assert!(mg.result.is_none());
        // `judge` via `st` is ignored (L648) — only `res` settles it.
        rg.on_message(
            "HOST",
            &encode_host(&HostMsg::State {
                phase: "judge".into(),
                t: Some(0.0),
            }),
            &mut mg,
            t,
        );
        assert_eq!(mg.phase, Phase::Finish);
    }

    #[test]
    fn hit_wire_shape_matches_js() {
        // sendHit frame (netmatch.js L165): {k:'hit', v, a, d, w} with d
        // already r2'd; our channel uses w:'shot' (weapons.js L1608 fallback).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let cfg = cfg(roster111("H", "G"), "tidewater", "H");
        let mut m = match111(&f, 1, 30.0);
        let mut r = Replication::new(&cfg, "H", "H");
        r.bind_match(&mut m);
        let mut rec = Rec::default();
        m.events.push(MatchEvent::HitSend {
            attacker: (0, 0),
            victim: (1, 0),
            damage: 33.333,
        });
        let rest = r.update(FIXED_DT, &mut m, &sim, t, w, sb, &mut rec);
        assert!(
            !rest.iter().any(|e| matches!(e, MatchEvent::HitSend { .. })),
            "HitSend is consumed"
        );
        let (to, payload) = rec.find("hit");
        assert_eq!(
            to.as_deref(),
            Some("G"),
            "point-to-point to the victim owner"
        );
        let v = parse_json(payload).unwrap();
        assert_eq!(v.field("k").and_then(JVal::as_str), Some("hit"));
        assert_eq!(v.field("v").and_then(JVal::as_f64), Some(1.0));
        assert_eq!(v.field("a").and_then(JVal::as_f64), Some(0.0));
        assert_eq!(v.field("d").and_then(JVal::as_f64), Some(33.33));
        assert_eq!(v.field("w").and_then(JVal::as_str), Some("shot"));
    }

    #[test]
    fn st_frame_without_t_leaves_the_clock_alone() {
        // Review M-4: JS `_hostState` L647 guards the hard-set with
        // `typeof d.t === 'number'` — a frame missing `t` must not zero the
        // guest clock (and `encode_host` drops the key, like JS
        // `JSON.stringify` dropping `undefined`).
        let f = fix();
        let mut mg = match111(&f, 3, 120.0);
        let cfg = cfg(roster111("HOST", "GUEST"), "tidewater", "HOST");
        let mut rg = Replication::new(&cfg, "GUEST", "HOST");
        rg.bind_match(&mut mg);
        mg.time = 42.0;
        let frame = encode_host(&HostMsg::State {
            phase: "playing".into(),
            t: None,
        });
        assert!(!frame.contains("\"t\""), "{frame}");
        rg.on_message("HOST", &frame, &mut mg, &f.tuning.player);
        assert_eq!(mg.time, 42.0, "a missing t must not zero the clock");
        assert_eq!(mg.phase, Phase::Active, "the state is still re-entered");
    }

    #[test]
    fn no_bots_bind_prunes_leavers() {
        // Review M-2: JS `bind` L79 — on a humans-only stage, actors whose
        // owner left *while the match was loading* are dropped at bind.
        let f = fix();
        let (h, g) = ("HOST", "GUEST");
        let cargo_cfg = cfg(roster111(h, g), "cargo", h);
        let mut mh = match111(&f, 4, 30.0);
        let mut rh = Replication::new(&cargo_cfg, h, h);
        rh.bind_match(&mut mh);
        // `g` left during the load — the session no longer lists them.
        rh.prune_loading_leavers(&mut mh, &[h.to_string()]);
        assert_eq!(mh.actors.len(), 2, "the leaver's actor is gone at bind");
        assert_eq!(rh.nid_of(1), Some(2), "the bot shifted down with it");
        assert_eq!(rh.remote_mask().len(), 2);
        // A stage that keeps bots is untouched (JS only prunes mapNoBots).
        let tide_cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh2 = match111(&f, 5, 30.0);
        let mut rh2 = Replication::new(&tide_cfg, h, h);
        rh2.bind_match(&mut mh2);
        rh2.prune_loading_leavers(&mut mh2, &[h.to_string()]);
        assert_eq!(mh2.actors.len(), 3, "bot stages keep everyone");
    }

    /// NFR-M2-2 / TR-22.4: the **playback layer's** CPU overhead must not
    /// push the native frame time more than 20 % over the bare sim step.
    /// Three arms over both endpoints of a 1v1+bot match (600 frames, 120
    /// warm-up, p50 of the per-frame wall time):
    ///   - `bare` — `step_masked` only (the M1 frame);
    ///   - `playback` — bare + the Task 22 layer driven directly
    ///     (`on_tick`/`push_sample` at 20 Hz from `ActorSample::from_actor`,
    ///     `advance_all`/`sample`/`apply` every frame), no JSON;
    ///   - `full` — bare + the whole [`Replication`] overlay including the
    ///     tick encode/decode (reported, not gated — the wire codec is
    ///     Task 21's surface and dominates its cost).
    ///
    /// Ignored by default (wall-clock, release-profile gate): run with
    /// `cargo test --release -p inkwave_sim -- --ignored`. Takes the min p50
    /// over five interleaved repeats per arm — a shared box throws
    /// scheduler hiccups (one run read +132 %), and best-case is the robust
    /// estimator for CPU overhead.
    #[test]
    #[ignore]
    fn nfr_m2_2_playback_overlay_stays_under_20_percent() {
        use std::time::Instant;

        use crate::net::playback::Playback;

        /// A transport that keeps the framed payloads for direct delivery
        /// to the peer's `on_message` (no relay latency inside the timed
        /// loop — we measure the CPU overlay, not the fake network).
        struct Tap {
            out: Vec<String>,
        }
        impl Transport for Tap {
            fn connect(&mut self, _c: &str, _n: &str, _r: bool) {}
            fn poll_events(&mut self) -> Vec<TransportEvent> {
                Vec::new()
            }
            fn send_raw(&mut self, s: &str) -> bool {
                if let Some(rest) = s.strip_prefix("b|") {
                    self.out.push(rest.to_string());
                } else if let Some(rest) = s.strip_prefix("s|")
                    && let Some((_, body)) = rest.split_once('|')
                {
                    self.out.push(body.to_string());
                }
                true
            }
            fn is_open(&self) -> bool {
                true
            }
            fn rtt(&self) -> f64 {
                0.0
            }
            fn close(&mut self) {}
        }

        fn p50(mut v: Vec<f64>) -> f64 {
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        }

        fn run(arm: u8) -> (f64, usize) {
            let f = fix();
            let sim = f.sim();
            let t = &f.tuning.player;
            let w = &f.tuning.weapons;
            let sb = &f.tuning.subs;
            let (h, g) = ("HOST", "GUEST");
            let cfg = cfg(roster111(h, g), "tidewater", h);
            let mut mh = match111(&f, 11, 30.0);
            let mut mg = match111(&f, 11, 30.0);
            let mut rh = Replication::new(&cfg, h, h);
            let mut rg = Replication::new(&cfg, g, h);
            rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 11);
            rh.bind_match(&mut mh);
            rg.bind_match(&mut mg);
            let nav = f.nav();
            let mut th = Tap { out: Vec::new() };
            let mut tg = Tap { out: Vec::new() };

            // `playback` arm: the Task 22 layer per client, driven by hand
            // (the same call sequence `Replication::update` runs, minus the
            // wire codec).
            let mut pbh = Playback::new(3, 1.0 / 20.0);
            let mut pbg = Playback::new(3, 1.0 / 20.0);
            let mut now = 0.0f64;
            let dt64 = FIXED_DT as f64;
            let gravity = t.gravity as f64;
            let mut tick_t = 0.0f32;

            const FRAMES: usize = 600;
            const WARM: usize = 120;
            let mut times: Vec<f64> = Vec::with_capacity(FRAMES - WARM);
            for frame in 0..FRAMES {
                let t0 = Instant::now();
                let mut ih = vec![ActorInput::default(); 3];
                rh.drive_bots(&mut mh, &nav, &sim, t, w, &f.tuning.weapon_range, &mut ih);
                mh.step_masked(FIXED_DT, &ih, Some(rh.remote_mask()), &sim, t, w, sb, None);
                let ig = vec![ActorInput::default(); 3];
                mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w, sb, None);
                match arm {
                    0 => {}
                    1 => {
                        now += dt64;
                        tick_t -= FIXED_DT;
                        if tick_t <= 0.0 {
                            tick_t += 1.0 / 20.0;
                            // owners stream their actors (JS `_sendTick`):
                            // the host's tick (host-owned samples) feeds the
                            // guest's playback under sender `h`, and vice
                            // versa — so `pbh.sample(_, g)` / `pbg.sample(_,
                            // h)` below find a live clock.
                            pbh.on_tick(g, now, now);
                            pbg.on_tick(h, now, now);
                            for (i, &rem) in rh.remote.iter().enumerate() {
                                if !rem {
                                    let s = ActorSample::from_actor(
                                        &mh.actors[i],
                                        &mh.runners[i],
                                        mh.kinds[i],
                                        mh.actors[i].net_tp,
                                        now as f32,
                                    );
                                    pbg.push_sample(i, &s);
                                }
                            }
                            for (i, &rem) in rg.remote.iter().enumerate() {
                                if !rem {
                                    let s = ActorSample::from_actor(
                                        &mg.actors[i],
                                        &mg.runners[i],
                                        mg.kinds[i],
                                        mg.actors[i].net_tp,
                                        now as f32,
                                    );
                                    pbh.push_sample(i, &s);
                                }
                            }
                        }
                        pbh.advance_all(now, dt64);
                        pbg.advance_all(now, dt64);
                        for (i, &rem) in rh.remote.iter().enumerate() {
                            if rem {
                                pbh.sample(i, g, dt64, gravity);
                            }
                        }
                        for (i, &rem) in rg.remote.iter().enumerate() {
                            if rem {
                                pbg.sample(i, h, dt64, gravity);
                            }
                        }
                        for (i, &rem) in rh.remote.iter().enumerate() {
                            if rem {
                                pbh.apply(i, &mut mh.actors[i], FIXED_DT);
                            }
                        }
                        for (i, &rem) in rg.remote.iter().enumerate() {
                            if rem {
                                pbg.apply(i, &mut mg.actors[i], FIXED_DT);
                            }
                        }
                    }
                    _ => {
                        let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut th);
                        let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut tg);
                        for p in th.out.drain(..) {
                            rg.on_message(h, &p, &mut mg, t);
                        }
                        for p in tg.out.drain(..) {
                            rh.on_message(g, &p, &mut mh, t);
                        }
                    }
                }
                if frame >= WARM {
                    times.push(t0.elapsed().as_secs_f64() * 1e6);
                }
            }
            // The proxy shown-count: the timing arms are only meaningful if
            // the overlay actually interpolates and displays them. The
            // `full` arm drives the clients' internal playback, the
            // `playback` arm the hand-built ones.
            let mut shown = 0;
            for (i, &rem) in rh.remote.iter().enumerate() {
                let live = if arm == 2 {
                    rh.playback().proxy_shown(i, mh.actors[i].alive)
                } else {
                    pbh.proxy_shown(i, mh.actors[i].alive)
                };
                if rem && live {
                    shown += 1;
                }
            }
            for (i, &rem) in rg.remote.iter().enumerate() {
                let live = if arm == 2 {
                    rg.playback().proxy_shown(i, mg.actors[i].alive)
                } else {
                    pbg.proxy_shown(i, mg.actors[i].alive)
                };
                if rem && live {
                    shown += 1;
                }
            }
            (p50(times), shown)
        }

        // Wall-clock arms on a shared box are noisy (one observed run read
        // +132 % from a scheduler hiccup). Take the **min** p50 over five
        // interleaved repeats per arm — best-case is the robust estimator
        // for CPU overhead — and check the shown-count from the last repeat.
        let mut base = f64::INFINITY;
        let mut pb = f64::INFINITY;
        let mut full = f64::INFINITY;
        let (mut base_shown, mut pb_shown, mut full_shown) = (0, 0, 0);
        for _ in 0..5 {
            let (v, s) = run(0);
            base = f64::min(base, v);
            base_shown = s;
            let (v, s) = run(1);
            pb = f64::min(pb, v);
            pb_shown = s;
            let (v, s) = run(2);
            full = f64::min(full, v);
            full_shown = s;
        }
        assert_eq!(pb_shown, 3, "the playback arm interpolates every proxy");
        assert_eq!(full_shown, 3, "the full overlay interpolates every proxy");
        assert_eq!(base_shown, 0, "the bare arm drives none");
        let gain = (pb - base) / base;
        eprintln!(
            "[nfr-m2-2] bare p50 = {base:.1} µs, +playback = {pb:.1} µs ({:+.1}%), +full overlay = {full:.1} µs ({:+.1}%)",
            gain * 100.0,
            (full - base) / base * 100.0
        );
        assert!(gain < 0.20, "playback gain {:+.1}% ≥ 20%", gain * 100.0);
    }

    // ------------------------------------------------------------------ TR-23.1
    #[test]
    fn tr23_1_splat_replay_matches_grid_and_never_loops_back() {
        // A sprays ink → B replays the timeline: byte-identical coverage
        // grids, and B re-broadcasts nothing (JS `applying` guard, L110 —
        // Rust: `PaintGrid::applying` + the seq-keyed `recorded_ink`
        // cursor).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 21, 120.0);
        let mut mg = match111(&f, 21, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);
        assert!(
            mh.paint.grid().iter().all(|&v| v == 0),
            "both grids start ink-free"
        );

        // A's splat at an r2-clean centre (the wire quantization is a no-op,
        // so a faithful replay must land on byte-identical cells).
        let pad0 = sim.spawn_pads[0];
        let c = vec3(
            wire::r2(pad0.x as f64) as f32,
            (wire::r2(pad0.y as f64) + 0.3) as f32,
            wire::r2(pad0.z as f64) as f32,
        );
        let opts = SplatOpts {
            seed: 0.42,
            stretch: None,
            stretch_amt: None,
            kind: None,
        };
        mh.paint.splat(sim.collision, c, 2.0, 0, &opts);
        assert!(
            mh.paint.grid().iter().any(|&v| v != 0),
            "the host splat painted cells"
        );

        // Run until B's grid catches up. B's own outgoing ticks must never
        // carry a splat record for the replayed ink (loopback).
        let mut guest_splat_records = 0;
        let mut settled = false;
        for _ in 0..600 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            // Host inbox: B's ticks — scan them for splat records *before*
            // routing (the loopback check).
            for (from, p) in drain_msgs(&mut trh) {
                if from == g
                    && let Some(v) = parse_json(&p)
                    && v.field("k").and_then(JVal::as_str) == Some("t")
                {
                    let (tick, _) = decode_tick(&v);
                    guest_splat_records += tick
                        .events
                        .iter()
                        .filter(|e| matches!(e.kind, TickEvent::Splat(_)))
                        .count();
                }
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if mg.paint.grid() == mh.paint.grid() {
                settled = true;
                break;
            }
        }
        assert!(settled, "B replays A's ink: grids byte-identical");
        assert_eq!(
            guest_splat_records, 0,
            "a replayed splat is never re-broadcast (JS `applying`)"
        );
    }

    // ------------------------------------------- M-1 regression (review fix)
    #[test]
    fn regression_recording_survives_render_draining() {
        // The app render layer calls [`crate::paint::PaintGrid::drain_ink`]
        // every frame (ink_render.rs L218). The seq-keyed cursor must still
        // record the local splat (M-1 regression: the old index cursor saw
        // an empty stream and silently dropped every local splat once the
        // renderer drained). Contract: the drain runs *after*
        // [`Replication::update`] in the same frame.
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 21, 120.0);
        let mut mg = match111(&f, 21, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        let pad0 = sim.spawn_pads[0];
        let c = vec3(
            wire::r2(pad0.x as f64) as f32,
            (wire::r2(pad0.y as f64) + 0.3) as f32,
            wire::r2(pad0.z as f64) as f32,
        );
        let opts = SplatOpts {
            seed: 0.42,
            stretch: None,
            stretch_amt: None,
            kind: None,
        };
        mh.paint.splat(sim.collision, c, 2.0, 0, &opts);

        let mut guest_splat_records = 0;
        let mut settled = false;
        for _ in 0..600 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            // Render layer drains *after* the replicator recorded / replayed.
            mh.paint.drain_ink();
            mg.paint.drain_ink();
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                if from == g
                    && let Some(v) = parse_json(&p)
                    && v.field("k").and_then(JVal::as_str) == Some("t")
                {
                    let (tick, _) = decode_tick(&v);
                    guest_splat_records += tick
                        .events
                        .iter()
                        .filter(|e| matches!(e.kind, TickEvent::Splat(_)))
                        .count();
                }
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if mg.paint.grid() == mh.paint.grid() {
                settled = true;
                break;
            }
        }
        assert!(
            settled,
            "a drained stream does not drop the local splat record"
        );
        assert_eq!(
            guest_splat_records, 0,
            "replayed ink is still never re-broadcast under draining"
        );

        // Discriminate the seq cursor itself: the guest now splats locally
        // *after* having replayed. The old index cursor — advanced past the
        // replayed entries and then reset by the drain — would drop this
        // record and the host's grid would never converge.
        let pad1 = sim.spawn_pads[1];
        let c2 = vec3(
            wire::r2(pad1.x as f64) as f32,
            (wire::r2(pad1.y as f64) + 0.3) as f32,
            wire::r2(pad1.z as f64) as f32,
        );
        mg.paint.splat(sim.collision, c2, 2.0, 1, &opts);
        let mut converged = false;
        for _ in 0..600 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            mh.paint.drain_ink();
            mg.paint.drain_ink();
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if mg.paint.grid() == mh.paint.grid() {
                converged = true;
                break;
            }
        }
        assert!(
            converged,
            "a local splat after replay is recorded despite the render drain"
        );
    }

    // ------------------------------------------------------------------ TR-23.2
    #[test]
    fn tr23_2_ghost_round_never_damages() {
        // One host round decides exactly one hit; the `'hit'` channel applies
        // it once on the owner. The guest's ghost copy flies the same path
        // and must not damage its own victim a second time (JS
        // `ghostProjectile` `damage: 0`, weapons.js L871 + the `dmg > 0`
        // guard, L1608).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 22, 120.0);
        let mut mg = match111(&f, 22, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        let pad0 = sim.spawn_pads[0];
        let victim_p = vec3(pad0.x, pad0.y, pad0.z + 3.0);
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = pad0;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
            a.invuln = 0.0;
        }
        for m in [&mut mh.actors[1], &mut mg.actors[1]] {
            m.pos = victim_p;
            m.invuln = 0.0;
        }
        let hp_before = mg.actors[1].hp;

        // Exactly one round: the trigger is held for a single frame.
        let mut inputs = vec![ActorInput::default(); 3];
        let mut dmg = None;
        let mut saw_ghost = false;
        for frame in 0..(1.5 * 60.0) as usize {
            inputs[0].fire = frame == 0;
            // keep the pinned geometry (proxies redraw from samples)
            mh.actors[1].pos = victim_p;
            mg.actors[1].pos = victim_p;
            mh.actors[0].pos = pad0;
            mh.actors[0].aim_yaw = 0.0;
            mh.actors[0].aim_pitch = 0.0;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            if dmg.is_none() {
                dmg = mh.events.iter().find_map(|e| match e {
                    MatchEvent::HitSend { damage, .. } => Some(*damage),
                    _ => None,
                });
            }
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            if !saw_ghost {
                saw_ghost = mg
                    .projectiles
                    .list
                    .iter()
                    .any(|p| p.ghost && p.damage == 0.0);
            }
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        let d = dmg.expect("the host round hit the proxy");
        assert!(saw_ghost, "the 'p' record spawned a ghost round on B");
        assert!(mg.actors[1].alive, "one round does not splat the victim");
        // The owner applied the round's damage exactly once; the guest's
        // ghost copy must not hurt its own victim a second time (a double
        // application would land near `hp_before - 2*d`, far below the
        // tolerance band — hp regen over the test window is a few points).
        let lost = hp_before - mg.actors[1].hp;
        assert!(
            lost > 0.0 && lost < d + 6.0,
            "exactly one damage application — the ghost never hurts: lost {lost} vs d {d}"
        );
        assert!(
            mh.projectiles.list.is_empty() && mg.projectiles.list.is_empty(),
            "both rounds die on expiry"
        );
    }

    #[test]
    fn tr23_2_ghost_never_paints_and_splats_land_on_the_sender_clock() {
        // A steep round into the turf: the host paints (its impact splat is
        // recorded as `'s'`), the guest's ghost flies the same path and must
        // add nothing of its own — and the replayed splat matures on the
        // playback timeline (~delay behind the sender clock), not the moment
        // the packet arrives (JS `_playEvents` gates on `p.tr`, L452-462).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 23, 120.0);
        let mut mg = match111(&f, 23, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        let pad0 = sim.spawn_pads[0];
        let stand = vec3(pad0.x, pad0.y, pad0.z + 2.0);
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = stand;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
        }

        let mut inputs = vec![ActorInput::default(); 3];
        let mut host_at = None;
        let mut guest_at = None;
        let mut saw_ghost = false;
        for frame in 0..150 {
            inputs[0].fire = frame == 0;
            mh.actors[0].pos = stand;
            mh.actors[0].aim_pitch = -1.4;
            mg.actors[0].pos = stand;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            if host_at.is_none() && !mh.paint.ink_stream().is_empty() {
                host_at = Some(frame);
            }
            if !saw_ghost {
                saw_ghost = mg.projectiles.list.iter().any(|p| p.ghost);
            }
            if guest_at.is_none() && !mg.paint.ink_stream().is_empty() {
                guest_at = Some(frame);
            }
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        let hf = host_at.expect("the host round painted the turf");
        let gf = guest_at.expect("the splat replayed onto B");
        assert!(
            gf > hf,
            "the replay waits for the playback clock (arrived frame {hf}, played {gf})"
        );
        assert!(
            gf - hf >= 3,
            "the splat lands ~sender-clock + playback delay, not on packet arrival: {gf} - {hf}"
        );
        assert!(saw_ghost, "the 'p' record spawned a ghost on B");
        assert_eq!(
            mg.paint.ink_stream().len(),
            mh.paint.ink_stream().len(),
            "the ghost painted nothing extra — B's stream is pure replay"
        );
        assert!(
            mh.projectiles.list.is_empty() && mg.projectiles.list.is_empty(),
            "the ghost disappears on expiry"
        );
    }

    // ------------------------------------------------------------------ TR-30.2
    /// Latest decoded actor sample per nid across every recorded `t` frame.
    fn latest_samples(rec: &Rec) -> Vec<(u8, ActorSample)> {
        let mut out: Vec<(u8, ActorSample)> = Vec::new();
        for (_, payload) in &rec.out {
            let Some(v) = parse_json(payload) else {
                continue;
            };
            if v.field("k").and_then(JVal::as_str) != Some("t") {
                continue;
            }
            let (tick, _) = decode_tick(&v);
            for (nid, s) in tick.actors {
                match out.iter_mut().find(|(n, _)| *n == nid) {
                    Some(row) => row.1 = s,
                    None => out.push((nid, s)),
                }
            }
        }
        out
    }

    /// Every decoded actor sample of every recorded `t` frame.
    fn all_samples(rec: &Rec) -> Vec<(u8, ActorSample)> {
        let mut out = Vec::new();
        for (_, payload) in &rec.out {
            if let Some(v) = parse_json(payload)
                && v.field("k").and_then(JVal::as_str) == Some("t")
            {
                let (tick, _) = decode_tick(&v);
                out.extend(tick.actors);
            }
        }
        out
    }

    #[test]
    fn tr30_2_pack_slots_three_loadouts() {
        // JS `packActor` (netmatch.js L723-751) slots `ch` (s[14], L748),
        // `lock` (s[20], L750) and `ks` (s[21], `MAIN_KITS[kind].netState`,
        // L750) for the Task 30 batch (twins / blaster / brolly). Spec D-5:
        // the twins dodge never appears in `lock` or in the `F` word.
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh = match111(&f, 30, 120.0);
        let mut mg = match111(&f, 30, 120.0);
        // D-2 per-actor loadout: actor0 twins, actor1 blaster, actor2 brolly.
        mh.kinds = vec![WeaponKind::Twins, WeaponKind::Blaster, WeaponKind::Brolly];
        mg.kinds = mh.kinds.clone();
        let mut rh = Replication::new(&cfg, h, h);
        let mut rg = Replication::new(&cfg, g, h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        let mut rech = Rec::default();
        let mut recg = Rec::default();
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        // Hold twins fire + move (dodge gate, weapons.js L533-548), tap the
        // jump at frame 10 → dodge roll; hold the blaster (auto) and the
        // brolly (tap-blast then unfold).
        let mut inputs = vec![ActorInput::default(); 3];
        let mut saw_roll = false;
        for frame in 0..30 {
            inputs[0].fire = true;
            inputs[0].move_dir = vec3(1.0, 0.0, 0.0);
            inputs[0].jump = frame == 10;
            inputs[1].fire = true;
            inputs[2].fire = true;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            // `dodge_vel` is read *before* `try_dodge` (JS order, match_.rs
            // L505-509), so the roll shows from the frame after the tap.
            if (10..28).contains(&frame) && mh.actors[0].dodge_vel.is_some() {
                saw_roll = true;
            }
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut rech);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut recg);
        }
        assert!(
            saw_roll,
            "the twins jump-tap started a dodge roll (spec D-5)"
        );

        let hs = latest_samples(&rech);
        let s0 = &hs.iter().find(|(n, _)| *n == 0).expect("twins sample").1;
        let s2 = &hs.iter().find(|(n, _)| *n == 2).expect("brolly sample").1;
        // Brolly unfolded: state Ready, open=1, grow=1, hp=450/450,
        // regrowK=0 → ks = 15<<2 | 7<<6 | 31<<9 = 16380 (brolly.js L636-641).
        assert_eq!(s2.ks, 16380, "brolly open pose (spec D-1)");
        assert_eq!(s2.ch, 0.0, "brolly is not streaming (Task 31)");
        assert_eq!(s2.lock, 0.0, "lockT is a dualies field (spec D-5)");
        assert_eq!(s0.ks, 0, "twins has no kit netState (spec D-1)");
        assert_eq!(s0.ch, 0.0, "twins is not a charge weapon");
        assert_eq!(
            s0.lock, 0.0,
            "twins roll lives in dodgeT/rollLock, not lockT"
        );
        let gs = latest_samples(&recg);
        let s1 = &gs.iter().find(|(n, _)| *n == 1).expect("blaster sample").1;
        assert_eq!(s1.ks, 0, "blaster has no kit netState");
        assert_eq!(s1.ch, 0.0, "blaster is auto, not streaming/charging");
        assert_eq!(s1.lock, 0.0);

        // No wire sample ever carries `F.dodge` (JS L735 reads `wr.dodge`,
        // the dualies object — spec D-5 keeps it 0 for twins).
        for (nid, s) in all_samples(&rech) {
            assert!(!s.flag(crate::net::flags::DODGE), "nid{nid} packed F.dodge");
            assert_eq!(s.lock, 0.0, "nid{nid} packed a lock timer");
        }

        // Release the brolly: the canopy folds (close_time 0.1 s) back to the
        // fresh Ready pose open=0, grow=1, hp full → ks = 7<<6 | 31<<9 = 16320.
        inputs[2].fire = false;
        for _ in 0..15 {
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut rech);
        }
        let s2 = &latest_samples(&rech)
            .into_iter()
            .find(|(n, _)| *n == 2)
            .expect("brolly sample")
            .1;
        assert_eq!(s2.ks, 16320, "brolly folded pose");
    }

    #[test]
    fn tr30_2_kit_record_ghost_blocks_local_shots() {
        // JS→Rust shape: a received `['k', nid, 'brolly', [x,y,z,dx,dz,hp,h0]]`
        // (recKit, netmatch.js L134-137) spawns a ghost canopy on the
        // receiver (JS `ghost` L650-653 → `addLaunched(ghost=true)` L276-288):
        // it slides, blocks the local player's shots and never paints (the
        // `mute++` wrapper L490 / paint.js L470). Unknown kinds and short
        // payloads are ignored (decode tolerance).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mg = match111(&f, 31, 120.0);
        let mut rg = Replication::new(&cfg, g, h);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Active);
        let mut rec = Rec::default();

        // Tolerance first: an unknown kit kind (JS `!K || !K.ghost` breaks)
        // and a truncated payload (JS would pack NaN) must spawn nothing.
        let junk = |kind: &str, data: Vec<f32>, ts: f32| Tick {
            ts,
            actors: Vec::new(),
            events: vec![TimedEvent {
                ts,
                kind: TickEvent::Kit(KitRecord {
                    nid: 0,
                    kind: kind.into(),
                    data: data.into_iter().map(KitVal::Num).collect(),
                    sub_kind: None,
                }),
            }],
            b: None,
        };
        rg.on_message(
            h,
            &encode_tick(&junk("mitts", vec![0.0; 7], 0.5), None),
            &mut mg,
            t,
        );
        rg.on_message(
            h,
            &encode_tick(&junk("brolly", vec![1.0, 2.0, 3.0], 0.5), None),
            &mut mg,
            t,
        );
        for _ in 0..30 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
        }
        assert!(
            mg.launched.is_empty(),
            "unknown / short 'k' records spawn nothing (decode tolerance)"
        );

        // The real record: a canopy at `pad0` (owner nid0 = the guest's
        // proxy, team0), its normal facing back at the local shooter pinned
        // 2.5 m behind it.
        let base = sim.spawn_pads[0];
        let shooter = vec3(base.x, base.y, base.z - 2.5);
        let hp0 = w.brolly.canopy_hp as f32;
        let proxy_hp = mg.actors[0].hp;
        let live = Tick {
            ts: 2.0,
            actors: Vec::new(),
            events: vec![TimedEvent {
                ts: 2.0,
                kind: TickEvent::Kit(KitRecord {
                    nid: 0,
                    kind: "brolly".into(),
                    data: vec![base.x, base.y, base.z, 0.0, -1.0, hp0, 1.2]
                        .into_iter()
                        .map(KitVal::Num)
                        .collect(),
                    sub_kind: None,
                }),
            }],
            b: None,
        };
        rg.on_message(h, &encode_tick(&live, None), &mut mg, t);

        let mut inputs = vec![ActorInput::default(); 3];
        let mut spawned = None;
        let mut fired = false;
        let mut min_hp = hp0;
        for frame in 0..120 {
            mg.actors[1].pos = shooter;
            mg.actors[1].yaw = 0.0;
            mg.actors[1].aim_yaw = 0.0;
            mg.actors[1].aim_pitch = 0.0;
            mg.actors[1].invuln = 0.0;
            inputs[1].fire = spawned.is_some() && !fired;
            if inputs[1].fire {
                fired = true;
            }
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
            if spawned.is_none() && !mg.launched.is_empty() {
                let c = &mg.launched[0];
                spawned = Some((frame, c.ghost, c.team, c.hp, c.pos, c.dir));
            }
            if let Some(c) = mg.launched.first() {
                min_hp = min_hp.min(c.hp);
            }
        }
        let (frame, ghost, team, hp_at_spawn, pos, dir) =
            spawned.expect("the 'k' record spawned a ghost canopy");
        assert!(
            frame > 0,
            "the ghost matures on the sender timeline (JS `p.tr`, L452-462)"
        );
        assert!(ghost, "the replayed canopy is a ghost (JS `mute++`, L490)");
        assert_eq!(team, 0, "the canopy carries the owner's team");
        assert_eq!(hp_at_spawn, hp0, "addLaunched seeds hp from the record");
        assert!((pos - vec3(base.x, base.y, base.z)).length() < 1e-3);
        assert!((dir - vec3(0.0, 0.0, -1.0)).length() < 1e-3);

        // The shot died on the canopy, not on the proxy behind it: the
        // canopy lost hp, no team-1 round survives, the proxy is untouched
        // and the ghost painted nothing (value 1 = team0 ink).
        assert!(
            min_hp < hp0 && min_hp > 0.0,
            "the blocked round drained the ghost canopy: {min_hp} of {hp0}"
        );
        assert_eq!(
            mg.actors[0].hp, proxy_hp,
            "the round never reached the proxy"
        );
        assert!(
            mg.projectiles.list.iter().all(|p| p.team != 1),
            "the round died on the shield"
        );
        assert!(
            mg.paint.grid().iter().all(|&v| v != 1),
            "a ghost canopy never paints (JS `mute`)"
        );
    }

    #[test]
    fn tr30_2_brolly_pose_converges_rust_to_rust() {
        // Rust→Rust, same seed: the host owns the brolly (actor0); the
        // canopy pose reaches the guest's proxy through the `ks` slot
        // (packActor L750 → unpackActor L754 → applyRemote `netApply` L398),
        // and the rebuilt held shield blocks the guest's own shots (item B).
        // The ghost round on the host drains the *real* kit hp, which flows
        // back through `ks`; a break and the full regrow cycle converge on
        // the same slot.
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 32, 120.0);
        let mut mg = match111(&f, 32, 120.0);
        mh.kinds = vec![WeaponKind::Brolly, WeaponKind::Shooter, WeaponKind::Shooter];
        mg.kinds = mh.kinds.clone();
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        // Geometry: host actor0 at `base` aiming +Z; the guest's own actor
        // 2.5 m ahead aiming back (re-pinned every frame, tr23_2 pattern).
        let base = sim.spawn_pads[0];
        let front = vec3(base.x, base.y, base.z + 2.5);
        let host_hp0 = mh.actors[0].hp;
        let mut inputs = vec![ActorInput::default(); 3];
        let mut frame = 0usize;
        // Phase A: the host holds fire (tap-blast at frame 0, then unfold);
        // the guest fires one round at frame 30, ~15 frames after `open=1`.
        while frame < 60 {
            inputs[0].fire = true;
            inputs[1].fire = frame == 30;
            for m in [&mut mh.actors[0], &mut mg.actors[0]] {
                m.pos = base;
                m.yaw = 0.0;
                m.aim_yaw = 0.0;
                m.aim_pitch = 0.0;
                m.invuln = 0.0;
            }
            for m in [&mut mh.actors[1], &mut mg.actors[1]] {
                m.pos = front;
                m.yaw = std::f32::consts::PI;
                m.aim_yaw = std::f32::consts::PI;
                m.aim_pitch = 0.0;
                m.invuln = 0.0;
            }
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            frame += 1;
        }
        // The guest's round died on the proxy's rebuilt held shield: the
        // host player is untouched. Its ghost copy flew the same path on the
        // host and drained the *real* canopy: 450 − 10 (JS `projDamage`
        // `p.damage || 10`, brolly.js L244) = 440, quantised into `ks` as
        // 30/31 → both ends agree on 15<<2 | 7<<6 | 30<<9 = 15868.
        let kit = mh.runners[0].brolly.as_ref().expect("host brolly kit");
        assert_eq!(kit.state, CanopyState::Ready);
        assert!(
            (kit.hp - 440.0).abs() < 1e-3,
            "the ghost round drained the real canopy: {}",
            kit.hp
        );
        assert_eq!(mh.actors[0].hp, host_hp0, "the blocked round never hurt");
        assert!(
            mg.projectiles.list.iter().all(|p| p.team != 1),
            "the guest round died on the proxy shield"
        );
        let hk = mh.runners[0].brolly_net_state();
        let gk = mg.runners[0].brolly_net_state();
        assert_eq!(hk, 15868, "host pose word (brolly.js L636-641)");
        assert_eq!(gk, hk, "the guest proxy converged through `ks`");

        // Phase B: break the host canopy (direct `hit_kit` — the break
        // mechanism itself is TR-30.1's scope; here only its propagation).
        inputs[0].fire = false;
        inputs[1].fire = false;
        crate::weapon::brolly::hit_kit(
            mh.runners[0].brolly.as_mut().expect("host kit"),
            500.0,
            mh.projectiles.time,
            w.brolly.regrow_time,
        );
        while frame < 90 {
            mh.actors[0].pos = base;
            mh.actors[0].aim_yaw = 0.0;
            mg.actors[1].pos = front;
            mg.actors[1].aim_yaw = std::f32::consts::PI;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            frame += 1;
        }
        // `regrow_k` starts filling the moment the canopy breaks (JS `tick`
        // L401-405), so compare the semantic bits + convergence, not a fixed
        // word: state Broken, hp 0, open/grow 0.
        let hk = mh.runners[0].brolly_net_state();
        let gk = mg.runners[0].brolly_net_state();
        assert_eq!(hk & 3, 1, "Broken state bits (brolly.js L636-641)");
        assert_eq!((hk >> 9) & 31, 0, "hp 0 bits");
        assert_eq!(hk & 0x1FF, 1, "open/grow folded on break");
        assert_eq!(gk, hk, "the break converges through `ks`");

        // Phase C: the 5 s regrow. At completion the host kit is Ready,
        // hp full, grow→1, regrowK=1 → ks = 7<<6 | 31<<9 | 7<<14 = 131008,
        // and the proxy's netApply→re-quantise round-trips exactly.
        while frame < 420 {
            mh.actors[0].pos = base;
            mh.actors[0].aim_yaw = 0.0;
            mg.actors[1].pos = front;
            mg.actors[1].aim_yaw = std::f32::consts::PI;
            mh.step_masked(
                FIXED_DT,
                &inputs,
                Some(rh.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            mg.step_masked(
                FIXED_DT,
                &inputs,
                Some(rg.remote_mask()),
                &sim,
                t,
                w,
                sb,
                None,
            );
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            frame += 1;
        }
        let hk = mh.runners[0].brolly_net_state();
        let gk = mg.runners[0].brolly_net_state();
        assert_eq!(hk, 131008, "regrown pose word (brolly.js L636-641)");
        assert_eq!(gk, hk, "the regrow converges through `ks`");
    }

    // ---------------------------------------------------------------- TR-34.2
    #[test]
    fn tr34_2_dh_owner_applies_the_ghost_hit_non_owner_ignores() {
        // JS `sendDevHit`/`netHurt` (netmatch.js L157-159, subs.js
        // L214-217) over the wire: a hit on a ghost device rides
        // `{k:'dh', kind:'subs', id, d}` to the owner's screen; the owner's
        // real copy takes the damage, every other screen drops it.
        use crate::sub::SubKind;
        use crate::weapon::SimEvent;

        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh = match111(&f, 41, 120.0);
        let mut mg = match111(&f, 41, 120.0);
        let mut rh = Replication::new(&cfg, h, h);
        let mut rg = Replication::new(&cfg, g, h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        // The host deploys a hop beacon locally: slot 0 mints gid 1
        // (D-T34-5) and the spawn record carries the plant payload.
        mh.actors[0].sub = SubKind::Beacon;
        mh.subs.use_sub(
            0,
            1,
            sim.collision,
            &mut mh.actors,
            &mut mh.paint,
            &mut mh.projectiles,
            t,
            sb,
            Some(rh.remote_mask()),
            0.0,
        );
        let evs = mh.subs.drain_events();
        let spawn = evs
            .iter()
            .find_map(|e| match e {
                SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == "beacon" => {
                    Some(data.clone())
                }
                _ => None,
            })
            .expect("the beacon ['k','subs',…] spawn record");
        assert_eq!(spawn[1], 1.0, "owner-side gid");
        assert_eq!(mh.subs.items[0].hp, sb.beacon.hp as f32);

        // The guest replays it as a ghost (the `['k', nid, 'subs', …]` arm,
        // subs.js `netGhost` L192-212).
        mg.subs.net_ghost(
            0,
            0,
            "beacon",
            &spawn,
            &rg.nids,
            sim.collision,
            &mut mg.actors,
            &mut mg.paint,
            &mut mg.projectiles,
            t,
            sb,
            Some(rg.remote_mask()),
            0.0,
        );
        assert_eq!(mg.subs.items.len(), 1);
        assert!(mg.subs.items[0].ghost);
        assert_eq!(mg.subs.items[0].gid, 1, "slot 0 and nid 0 pack the same id");
        let hp0 = sb.beacon.hp as f32;

        // A guest round dies on the ghost capsule: `block_shot` keeps the
        // ghost hp (its owner decides, subs.js L597-607) and emits the
        // DeviceHit that the `dh_out` drain sends to the owner.
        let bp = mg.subs.items[0].pos;
        let blocked = mg.subs.block_shot(
            bp + vec3(0.0, 0.5, -1.0),
            bp + vec3(0.0, 0.5, -0.2),
            1,
            30.0,
            false,
            sb,
        );
        assert!(blocked, "the beacon capsule stops the round");
        let dev = mg
            .subs
            .drain_events()
            .into_iter()
            .find_map(|e| match e {
                SimEvent::DeviceHit { gid, d, .. } => Some((gid, d)),
                _ => None,
            })
            .expect("the ghost hit reports a DeviceHit");
        assert_eq!(dev, (1, 30.0));

        // The wire `'dh'` frame, delivered to both screens: the guest's
        // ghost copy ignores it (`net_hurt` only matches real items), the
        // host's real beacon takes the 30.
        let msg = encode_direct(&DirectMsg::DeviceHit {
            kind: "subs".into(),
            id: dev.0,
            d: dev.1,
        });
        rg.on_message(h, &msg, &mut mg, t);
        assert_eq!(
            mg.subs.items[0].hp, hp0,
            "the non-owner screen ignores the 'dh'"
        );
        rh.on_message(g, &msg, &mut mh, t);
        assert_eq!(mh.subs.items[0].hp, 20.0, "the owner applied the hit");

        // A second hit kills it: the owner's splice emits the `[2, gid]`
        // end record (subs.js L164-167).
        let msg2 = encode_direct(&DirectMsg::DeviceHit {
            kind: "subs".into(),
            id: 1,
            d: 30.0,
        });
        rh.on_message(g, &msg2, &mut mh, t);
        mh.subs.update(
            FIXED_DT,
            sim.collision,
            &mut mh.actors,
            &mut mh.paint,
            &mut mh.projectiles,
            t,
            sb,
            Some(rh.remote_mask()),
            0.0,
        );
        let ends: Vec<Vec<f32>> = mh
            .subs
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                SimEvent::SubEnd { data, .. } => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(ends, vec![vec![2.0, 1.0]]);
        assert!(mh.subs.items.is_empty());
    }

    #[test]
    fn tr34_2_bomb_ghost_record_is_presentation_only() {
        // JS `_play` 'b' (netmatch.js L477 → `ghostBomb`, weapons.js
        // L877-885): a remote player's splat bomb flies and pops on this
        // screen under the mute wrap — it paints nothing and hurts nothing;
        // the owner's screen decides the damage.
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mg = match111(&f, 43, 120.0);
        let mut rg = Replication::new(&cfg, g, h);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Active);

        // The victim stands exactly under the record's drop point (nid 0 is
        // the host — the guest's remote proxy 0).
        let base = sim.spawn_pads[0];
        mg.actors[1].pos = base;
        mg.actors[1].set_invuln(0.0);
        let hp0 = mg.actors[1].hp;
        let cov0 = mg.paint.coverage();

        let tick = Tick {
            ts: 2.0,
            actors: Vec::new(),
            events: vec![TimedEvent {
                ts: 2.0,
                kind: TickEvent::Bomb(BombRecord {
                    nid: 0,
                    kind: "bomb".into(),
                    pos: [base.x, base.y + 1.2, base.z],
                    vel: [0.0, 0.0, 0.0],
                }),
            }],
            b: None,
        };
        rg.on_message(h, &encode_tick(&tick, None), &mut mg, t);

        let mut rec = Rec::default();
        let mut saw_ghost = None;
        for _ in 0..400 {
            mg.actors[1].pos = base;
            mg.actors[1].invuln = 0.0;
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
            if saw_ghost.is_none() && !mg.projectiles.bombs.is_empty() {
                saw_ghost = Some(mg.projectiles.bombs[0].ghost);
            }
        }
        assert_eq!(
            saw_ghost,
            Some(true),
            "the ['b'] record replays as a ghost bomb"
        );
        assert!(mg.projectiles.bombs.is_empty(), "the ghost popped and left");
        assert_eq!(mg.actors[1].hp, hp0, "a ghost bomb hurts nothing");
        assert_eq!(
            mg.paint.coverage(),
            cov0,
            "a ghost bomb paints nothing (JS `mute`)"
        );
        assert!(mg.actors[1].alive);
    }

    #[test]
    fn tr34_2_torpedo_ghost_locks_launches_but_decides_nothing() {
        // JS `SUB_KITS.torpedo.ghost` (torpedo.js L270-300): the
        // `['k', nid, 'torpedo', [0|3|4|1, …]]` ops fly, lock, launch and
        // burst on this screen as pure presentation. The `[3]` lock record
        // carries the foe's wire `nid` (JS `byNid` L278) — the ghost
        // resolves it to the *local* actor list — but every damage, paint
        // and end decision belongs to the owner's screen (D-T34-5).
        use crate::sub::torpedo::TorpedoState;
        use crate::weapon::SimEvent;

        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mg = match111(&f, 44, 120.0);
        let mut rg = Replication::new(&cfg, g, h);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Active);
        let mut rec = Rec::default();

        let kit = |data: Vec<f32>, ts: f32| Tick {
            ts,
            actors: Vec::new(),
            events: vec![TimedEvent {
                ts,
                kind: TickEvent::Kit(KitRecord {
                    nid: 0,
                    kind: "torpedo".into(),
                    data: data.into_iter().map(KitVal::Num).collect(),
                    sub_kind: None,
                }),
            }],
            b: None,
        };
        let echo = |evs: Vec<SimEvent>| -> bool {
            evs.iter().any(|e| match e {
                SimEvent::SubSpawn { sub_kind, .. } => sub_kind == "torpedo",
                SimEvent::SubEnd { sub_kind, .. } => *sub_kind == Some("torpedo".to_string()),
                _ => false,
            })
        };

        // The guest's own player (actor 1, team 1) stands in the throw
        // line: a real torpedo locks and bursts on it, the ghost may not.
        let base = sim.spawn_pads[0];
        let foe = vec3(base.x, base.y, base.z + 5.0);
        let hp0 = mg.actors[1].hp;
        let mut echo_seen = false;
        macro_rules! step_frame {
            () => {{
                mg.actors[1].pos = foe;
                mg.actors[1].invuln = 0.0;
                mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
                let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
                if echo(mg.subs.drain_events()) {
                    echo_seen = true;
                }
            }};
        }

        // op0: the throw record `[0, gid, pos×3, vel×3]` (JS L281-286).
        let p0 = vec3(base.x, base.y + 1.35, base.z + 0.3);
        rg.on_message(
            h,
            &encode_tick(
                &kit(vec![0.0, 1.0, p0.x, p0.y, p0.z, 0.0, 3.0, 9.0], 2.0),
                None,
            ),
            &mut mg,
            t,
        );
        let mut spawned = None;
        for frame in 0..60 {
            step_frame!();
            if let Some(it) = mg.subs.torpedo_items().first() {
                spawned = Some((frame, it.ghost, it.team, it.src_nid, it.gid, it.state));
                break;
            }
        }
        let (frame, ghost, team, src, gid, state) =
            spawned.expect("the ['k',…,[0,…]] record ghosts a torpedo");
        assert!(frame > 0, "the ghost matures on the sender timeline");
        assert!(ghost, "the replayed torpedo is a ghost (JS `mute` wrap)");
        assert_eq!((team, src, gid), (0, Some(0), 1), "owner nid 0 → proxy 0");
        assert_eq!(state, TorpedoState::Fly, "op0 ghosts into the arc");

        // op3: the lock record `[3, gid, foe_nid, pos×3]` — nid 1 is the
        // guest's own player, so `byNid` must resolve to actor index 1.
        for _ in 0..10 {
            step_frame!();
        }
        let p = mg.subs.torpedo_items()[0].pos;
        rg.on_message(
            h,
            &encode_tick(&kit(vec![3.0, 1.0, 1.0, p.x, p.y, p.z], 2.5), None),
            &mut mg,
            t,
        );
        let mut saw = None;
        for _ in 0..60 {
            step_frame!();
            if let Some(it) = mg.subs.torpedo_items().first()
                && it.state == TorpedoState::Unfold
            {
                saw = Some((it.target, it.pos));
                break;
            }
        }
        let (tgt, pos_at_lock) = saw.expect("op3 start_locks the ghost (Fly → Unfold)");
        assert_eq!(
            tgt,
            Some(1),
            "the record's foe nid 1 resolves to the local actor index (JS `byNid`)"
        );
        assert!(
            (pos_at_lock - p).length() < 0.2,
            "the lock record places the ghost at its owner's report"
        );

        // The ghost unfolds on its own timer and launches.
        let mut launched = false;
        for _ in 0..120 {
            step_frame!();
            if mg
                .subs
                .torpedo_items()
                .first()
                .is_some_and(|i| i.state == TorpedoState::Launch)
            {
                launched = true;
                break;
            }
        }
        assert!(launched, "the ghost unfolds into Launch (JS `unfold` L386)");

        // op4: the swim report `[4, gid, pos×3, dir×3, speed]` seeds the
        // dead-reckoning; the ghost swims the owner's line.
        let p = mg.subs.torpedo_items()[0].pos;
        rg.on_message(
            h,
            &encode_tick(
                &kit(vec![4.0, 1.0, p.x, p.y, p.z, 0.0, 0.0, 1.0, 14.0], 3.0),
                None,
            ),
            &mut mg,
            t,
        );
        for _ in 0..10 {
            step_frame!();
        }
        let z0 = mg.subs.torpedo_items()[0].pos.z;
        for _ in 0..40 {
            step_frame!();
        }
        let it = &mg.subs.torpedo_items()[0];
        assert!(it.net.is_some(), "op4 seeds the ghost's swim report");
        assert_eq!(it.speed, 14.0, "the ghost swims at the reported speed");
        assert!(
            it.pos.z > z0 + 0.5,
            "the ghost chases down the owner's line: z {} → {}",
            z0,
            it.pos.z
        );
        assert_eq!(mg.actors[1].hp, hp0, "the ghost swim hurts nobody");

        // op1: the full-burst record `[1, gid, pos×3, 1]` — presentation
        // only: muted paint, routed-away hits, muted droplets.
        let p = mg.subs.torpedo_items()[0].pos;
        rg.on_message(
            h,
            &encode_tick(&kit(vec![1.0, 1.0, p.x, p.y, p.z, 1.0], 4.0), None),
            &mut mg,
            t,
        );
        for _ in 0..30 {
            step_frame!();
            if mg.subs.torpedo_items().is_empty() {
                break;
            }
        }
        assert!(
            mg.subs.torpedo_items().is_empty(),
            "the [1,…,1] record bursts the ghost away"
        );
        assert_eq!(
            mg.actors[1].hp, hp0,
            "the ghost burst decides nothing: the local foe is untouched"
        );
        assert!(
            mg.paint.grid().iter().all(|&v| v != 1),
            "a ghost torpedo never paints (JS `mute`)"
        );
        assert!(
            !echo_seen,
            "the ghost emitted no SubSpawn/SubEnd echoes of its own"
        );
    }

    #[test]
    fn tr34_2_waddle_dh_kills_the_real_one_and_broadcasts_the_end() {
        // The full round trip (netmatch.js L157-159 `sendDevHit` +
        // waddle.js `netHurt` L691 / `blockShot` L720): the host throws a
        // real waddle (`'k'` op0 → guest ghosts it); a guest round dies on
        // the ghost capsule, which keeps its own hp and rides `'dh'` to the
        // owner; the owner's real waddle takes the damage, pops at 0 hp,
        // and its `[2, gid, popped]` end record (waddle.js L706-712)
        // travels back as `'k'` op2 to remove the ghost.
        use crate::sub::SubKind;
        use crate::sub::waddle::WaddleState;
        use crate::weapon::SimEvent;

        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 45, 120.0);
        let mut mg = match111(&f, 45, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mh.phase, Phase::Active);

        let base = sim.spawn_pads[0];
        let far_a = vec3(base.x + 20.0, base.y, base.z + 20.0);
        let far_b = vec3(base.x - 20.0, base.y, base.z - 20.0);
        let echo = |evs: Vec<SimEvent>| -> bool {
            evs.iter().any(|e| match e {
                SimEvent::SubSpawn { sub_kind, .. } => sub_kind == "waddle",
                SimEvent::SubEnd { sub_kind, .. } => *sub_kind == Some("waddle".to_string()),
                _ => false,
            })
        };
        let mut echo_seen = false;
        macro_rules! pair_step {
            () => {{
                // Pin the host-side foes far from the throw: the real
                // waddle must land, sense nobody and sit on its 1.1 s fuse
                // (config.js L378) while the `'dh'` exchange runs.
                mh.actors[1].pos = far_a;
                mh.actors[2].pos = far_b;
                mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
                mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
                let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
                let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut trg);
                trh.advance(16.0);
                trg.advance(0.0);
                for (from, p) in drain_msgs(&mut trh) {
                    rh.on_message(&from, &p, &mut mh, t);
                }
                for (from, p) in drain_msgs(&mut trg) {
                    rg.on_message(&from, &p, &mut mg, t);
                }
                if echo(mg.subs.drain_events()) {
                    echo_seen = true;
                }
            }};
        }

        // The host throws: slot 0 mints gid 1 (D-T34-5), the spawn record
        // is `[0, gid, pos×3, vel×3]` (waddle.js L269).
        mh.actors[0].ink = 100.0;
        mh.actors[0].sub = SubKind::Waddle;
        mh.subs.use_sub(
            0,
            1,
            sim.collision,
            &mut mh.actors,
            &mut mh.paint,
            &mut mh.projectiles,
            t,
            sb,
            Some(rh.remote_mask()),
            0.0,
        );
        let hp_full = sb.waddle.hp as f32;
        let g_hp0 = mg.actors[1].hp;

        let mut landed = None;
        for _ in 0..240 {
            pair_step!();
            let host = mh.subs.waddle_items().first().map(|i| i.state);
            let guest = mg
                .subs
                .waddle_items()
                .first()
                .map(|i| (i.ghost, i.team, i.src_nid, i.gid, i.hp, i.state));
            if host.is_some_and(|s| s != WaddleState::Fly)
                && guest.is_some_and(|x| x.5 != WaddleState::Fly)
            {
                landed = Some(guest.unwrap());
                break;
            }
        }
        let (ghost, team, src, gid, ghp, state) =
            landed.expect("the throw round-tripped and both copies landed");
        assert!(ghost, "the guest replays the throw as a ghost");
        assert_eq!(
            (team, src, gid),
            (0, Some(0), 1),
            "the ghost keys on (src_nid, gid)"
        );
        assert_eq!(ghp, hp_full, "the ghost lands at full hp");
        assert_ne!(state, WaddleState::Fly);

        // A guest round dies on the ghost capsule: blocked, hp kept,
        // DeviceHit emitted (waddle.js `hurt` L608 → `netHurt` L691).
        // `blockShot` compares `segmentCapsuleDist` (distance to the
        // capsule CORE axis, physics.js L306) against 0.2, so the ray
        // must end strictly inside 0.2 of the axis, at core height
        // [0.17, 0.23] (radius 0.17, height 0.4).
        let gp = mg.subs.waddle_items()[0].pos;
        let blocked = mg.subs.block_shot(
            gp + vec3(0.0, 0.2, -1.0),
            gp + vec3(0.0, 0.2, -0.1),
            1,
            10.0,
            false,
            sb,
        );
        assert!(blocked, "the ghost capsule absorbs the round");
        // NB: do NOT drain the queued `DeviceHit` here — `Match::update`
        // (match_.rs L1135-1147) must see it to route `dh_out` → `'dh'`.
        // The exact tuple (gid 1, kind "waddle", d 10) is verified
        // end-to-end below: the host's `net_hurt` only matches that gid
        // and only that damage lands.
        assert_eq!(
            mg.subs.waddle_items()[0].hp,
            hp_full,
            "the ghost keeps its own hp — the owner decides"
        );

        // The `'dh'` rides the relay to the owner: the guest's own copy
        // ignores it, the host's real waddle takes the 10.
        pair_step!();
        assert_eq!(
            mh.subs.waddle_items()[0].hp,
            hp_full - 10.0,
            "the owner applied the 'dh' damage"
        );
        assert_eq!(
            mg.subs.waddle_items()[0].hp,
            hp_full,
            "the non-owner screen ignores the 'dh'"
        );

        // A lethal 30 pops the real one: `hurt` → `pop` → the dead splice
        // emits `[2, gid, 1]` (waddle.js L706-712, `why == 'popped'`).
        let gp = mg.subs.waddle_items()[0].pos;
        assert!(
            mg.subs.block_shot(
                gp + vec3(0.0, 0.2, -1.0),
                gp + vec3(0.0, 0.2, -0.1),
                1,
                30.0,
                false,
                sb,
            ),
            "the second round dies on the ghost again"
        );
        pair_step!(); // routes the lethal 'dh' to the host: hp 20 − 30 ≤ 0
        assert_eq!(
            mh.subs.waddle_items()[0].hp,
            hp_full - 40.0,
            "the owner applied the second hit"
        );
        mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w, sb, None);
        let end = mh
            .kit_out
            .iter()
            .find(|k| k.kind == "waddle" && k.data.first() == Some(&2.0))
            .expect("the pop splices the [2, gid, popped] end record");
        assert_eq!(end.data, vec![2.0, 1.0, 1.0], "popped = 1 (JS `why`)");
        // `rh.update` must run *before* the next `step_masked` (which
        // clears `kit_out`, match_.rs L468) so the end record rides the
        // broadcast tick. It then matures on the guest only once
        // `rg.update` advances the playback clock, so drive a few full
        // round-trip frames (the ghost's 40 s age-out is nowhere near —
        // removal here can only be the op2 splice).
        let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, sb, &mut trh);
        trh.advance(16.0);
        trg.advance(0.0);
        for (from, p) in drain_msgs(&mut trh) {
            rh.on_message(&from, &p, &mut mh, t);
        }
        for (from, p) in drain_msgs(&mut trg) {
            rg.on_message(&from, &p, &mut mg, t);
        }
        for _ in 0..60 {
            if mg.subs.waddle_items().is_empty() {
                break;
            }
            pair_step!();
        }
        assert!(
            mh.subs.waddle_items().is_empty(),
            "the popped waddle is gone on the owner's screen"
        );
        assert!(
            mg.subs.waddle_items().is_empty(),
            "the end record removed the guest's ghost"
        );

        assert_eq!(
            mg.actors[1].hp, g_hp0,
            "the guest's own player is untouched"
        );
        assert!(
            mg.paint.grid().iter().all(|&v| v != 1),
            "the ghost waddle never painted"
        );
        assert!(
            mh.paint.grid().iter().all(|&v| v != 1),
            "the real waddle popped before it could blast"
        );
        assert!(
            !echo_seen,
            "the guest ghost emitted no SubSpawn/SubEnd echoes"
        );
    }

    #[test]
    fn tr34_2_boomerang_ghost_never_bursts_on_a_local_foe() {
        // JS `kits/boomerang.js` `ghost` (L200-221) under the `'k'` mute
        // wrap: the replay rides Out → Hover → Back → Orbit around its
        // owner's proxy, but every burst / arm / shred decision is guarded
        // on `!ghost` (boomerang.js L249-369) — a local foe in the flight
        // line and under the hover takes nothing, and only the owner's
        // `[1, …]` record ends it.
        use crate::sub::boomerang::BoomerangState;
        use crate::weapon::SimEvent;

        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mg = match111(&f, 46, 120.0);
        let mut rg = Replication::new(&cfg, g, h);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
        }
        assert_eq!(mg.phase, Phase::Active);
        let mut rec = Rec::default();

        let kit = |data: Vec<f32>, ts: f32| Tick {
            ts,
            actors: Vec::new(),
            events: vec![TimedEvent {
                ts,
                kind: TickEvent::Kit(KitRecord {
                    nid: 0,
                    kind: "boomerang".into(),
                    data: data.into_iter().map(KitVal::Num).collect(),
                    sub_kind: None,
                }),
            }],
            b: None,
        };
        let echo = |evs: Vec<SimEvent>| -> bool {
            evs.iter().any(|e| match e {
                SimEvent::SubSpawn { sub_kind, .. } => sub_kind == "boomerang",
                SimEvent::SubEnd { sub_kind, .. } => *sub_kind == Some("boomerang".to_string()),
                _ => false,
            })
        };

        let base = sim.spawn_pads[0];
        let mut foe = vec3(base.x, base.y, base.z + 4.5);
        let hp0 = mg.actors[1].hp;
        let mut echo_seen = false;
        let mut saw_armed = false;
        macro_rules! step_frame {
            () => {{
                mg.actors[1].pos = foe;
                mg.actors[1].invuln = 0.0;
                mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w, sb, None);
                let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, sb, &mut rec);
                let evs = mg.subs.drain_events();
                if echo(evs.clone()) {
                    echo_seen = true;
                }
                if let Some(it) = mg.subs.boomerang_items().first()
                    && it.state == BoomerangState::Armed
                {
                    saw_armed = true;
                }
            }};
        }

        // op0: `[0, gid, start×3, dir×3]` — the ghost re-plans the raycast
        // on this world (boomerang.js L205-210 `planFrom`).
        let st = vec3(base.x, base.y + 1.35, base.z + 0.3);
        rg.on_message(
            h,
            &encode_tick(
                &kit(vec![0.0, 1.0, st.x, st.y, st.z, 0.0, 0.0, 1.0], 2.0),
                None,
            ),
            &mut mg,
            t,
        );
        let mut spawned = None;
        for frame in 0..60 {
            step_frame!();
            if let Some(it) = mg.subs.boomerang_items().first() {
                spawned = Some((frame, it.ghost, it.team, it.src_nid, it.gid, it.state));
                break;
            }
        }
        let (frame, ghost, team, src, gid, state) =
            spawned.expect("the ['k',…,[0,…]] record ghosts a boomerang");
        assert!(frame > 0, "the ghost matures on the sender timeline");
        assert!(ghost, "the replayed throw is a ghost (JS `mute` wrap)");
        assert_eq!((team, src, gid), (0, Some(0), 1), "owner nid 0 → proxy 0");
        assert_eq!(state, BoomerangState::Out);

        // Out rides through the pinned foe (a real throw would ARM there,
        // boomerang.js L257) and settles into Hover.
        let mut hover = None;
        for _ in 0..150 {
            step_frame!();
            match mg.subs.boomerang_items().first() {
                Some(it) if it.state == BoomerangState::Hover => {
                    hover = Some(it.pos);
                    break;
                }
                None => break,
                _ => {}
            }
        }
        let hover = hover.expect("the ghost rides Out into Hover");

        // Park the foe right under the hover: the 8 Hz shred (L283-309)
        // would tick-damage and ink a real one's spot.
        foe = hover + vec3(0.0, -0.4, -0.8);
        let mut saw_back = false;
        let mut saw_orbit = false;
        for _ in 0..200 {
            step_frame!();
            if let Some(it) = mg.subs.boomerang_items().first() {
                match it.state {
                    BoomerangState::Back => saw_back = true,
                    BoomerangState::Orbit => {
                        saw_orbit = true;
                        break;
                    }
                    _ => {}
                }
            } else {
                break;
            }
        }
        assert!(saw_back, "the ghost starts back after its hover");
        assert!(
            saw_orbit,
            "the ghost homes onto its owner's proxy and orbits"
        );

        // Orbit past `orbitTime` (2.5 s): a real one bursts, the ghost just
        // keeps circling (L368 `!it.ghost` guard).
        let mut orbit_frames = 0;
        for _ in 0..220 {
            step_frame!();
            match mg.subs.boomerang_items().first() {
                Some(it) if it.state == BoomerangState::Orbit => orbit_frames += 1,
                Some(_) => break,
                None => break,
            }
            if orbit_frames > 160 {
                break;
            }
        }
        assert!(
            orbit_frames > 150,
            "the ghost orbits past its burst timer, unburst: {orbit_frames} frames"
        );
        assert_eq!(mg.actors[1].hp, hp0, "the ghost shredded / grazed nobody");

        // The owner's `[1, …, 0]` burst record ends it — muted.
        let p = mg.subs.boomerang_items()[0].pos;
        rg.on_message(
            h,
            &encode_tick(&kit(vec![1.0, 1.0, p.x, p.y, p.z, 0.0], 6.0), None),
            &mut mg,
            t,
        );
        for _ in 0..30 {
            step_frame!();
            if mg.subs.boomerang_items().is_empty() {
                break;
            }
        }
        assert!(
            mg.subs.boomerang_items().is_empty(),
            "the [1,…,0] record bursts the ghost away"
        );
        assert_eq!(mg.actors[1].hp, hp0, "the ghost burst decides nothing");
        assert!(!saw_armed, "a local foe never arms a ghost");
        assert!(
            mg.paint.grid().iter().all(|&v| v != 1),
            "a ghost boomerang never paints (JS `mute`)"
        );
        assert!(
            !echo_seen,
            "the ghost emitted no SubSpawn/SubEnd echoes of its own"
        );
    }
}
