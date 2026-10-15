//! Task 25 — full-stack end-to-end match scenarios (TR-25.1 / TR-25.2 /
//! TR-25.3). TEST-ONLY, like the rest of this crate.
//!
//! Where [`crate::run_fake`] stops at the *session* layer (rosters, payloads,
//! lock, leave), this drives the whole client stack the way the Bevy app
//! does: `FakeRelay` + N `NetSession`s + one [`Replication`] + [`Match`] per
//! client, stepped in lockstep on the shared virtual clock. The driving
//! sequence is the verbatim `tr24_lobby_flow_two_clients` pattern
//! (`crates/inkwave/src/net/match_link.rs` L611-818, now N-client) and the
//! event dispatch mirrors the app's `net_tick` contract (`Host` →
//! `set_host` + flag, `Leave` → `on_leave(flag)`, `GameMessage` →
//! `on_message`, `MatchEnd` → `end_match`).
//!
//! Scenario knobs (tasks.md Task 25):
//!  - TR-25.1: ≥3 headless clients (1 host + 2 guests, bots pad the roster
//!    to 8), shortened duration; assert every client's turf coverage agrees
//!    within 0.5 pp and the settled result equals the host's `res`.
//!  - TR-25.2: rerun with the `?netlag=40&netjitter=30&netspike=0.01` SIM
//!    (`LinkSim`) and collect the smoothness stats — kink / path error /
//!    teleports — ported verbatim from `tools/net-test.mjs` (line anchors
//!    on each formula). Numeric thresholds are set by the Task 25 review
//!    against JS semantics (spec Open Question 3), not here.
//!  - TR-25.3: mid-match drop-out — host (migration) or guest (actor
//!    adopted as a bot) — run to settlement.
//!
//! JS `net-test.mjs` records *rendered* frames on a wall clock; the harness
//! records every fixed 60 Hz substep on the virtual clock
//! (`frame × 1000/60` ms). The kink steady-frame filter (6–45 ms) passes
//! every frame at a constant 16.67 ms step, so the formulas keep their
//! meaning; the review calibrates against that difference.
//!
//! Declared deviation (review A6): the app holds the results screen for the
//! JS `_netEndT` 12 s countdown (`main.js` L972, `net_tick`'s `end_timer`)
//! before `sendEnd`; the harness sends `end` right after settlement — the
//! handshake frames themselves are identical.

use std::collections::HashMap;

use inkwave_sim::actor::{ActorInput, FIXED_DT, SimWorld};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::match_::{FINISH_TIME, INTRO_TIME, Match, MatchResult, Phase};
use inkwave_sim::net::matchbuild::{build_net_match, fnv1a, local_roster_index};
use inkwave_sim::net::replication::{Replication, ReplicationEvent};
use inkwave_sim::net::session::{
    MatchPhase, MeChange, NetSession, Profile, SessionEvent, SessionState, SettingsPatch, StartCfg,
    map_no_bots,
};
use inkwave_sim::net::transport::{FakeRelay, FakeTransport, LinkSim, Transport};
use inkwave_sim::tuning::{PlayerTuning, Tuning};
use inkwave_sim::{embedded_tidewater, embedded_tuning};

/// The session type of one harness client (the tr24 pattern,
/// `match_link.rs` L418-428: shared virtual clock through one `FakeRelay`).
pub type HSess = NetSession<FakeTransport, Box<dyn FnMut() -> FakeTransport>>;

fn hsess(relay: &FakeRelay, sim: Option<LinkSim>, seed: u64) -> HSess {
    let r = relay.clone();
    NetSession::new(
        Profile::default(),
        seed,
        Box::new(move || r.transport(sim, seed)),
    )
}

/// Advance the shared virtual clock by `dt_ms` (the first endpoint drives
/// the relay clock, the rest read at the same instant — tr24 `step2`,
/// `match_link.rs` L437-448), pump every session and return the drained
/// events with their client index.
fn drain(sess: &mut [HSess], dt_ms: f64) -> Vec<(usize, SessionEvent)> {
    for (i, s) in sess.iter_mut().enumerate() {
        if let Some(tr) = s.transport_mut() {
            tr.advance(if i == 0 { dt_ms } else { 0.0 });
        }
    }
    for s in sess.iter_mut() {
        s.update(dt_ms / 1000.0);
    }
    let mut out = Vec::new();
    for (i, s) in sess.iter_mut().enumerate() {
        out.extend(s.poll_events().into_iter().map(|e| (i, e)));
    }
    out
}

// ---------------------------------------------------------------------------
// Scenario knobs
// ---------------------------------------------------------------------------

/// A mid-match drop-out: client `client` closes its socket at `at_s` seconds
/// of match time (≡ `net-test.mjs` `--leave host|guest` with `DROP=kill`,
/// the browser process killed so the relay sees the socket drop, L118-125).
#[derive(Debug, Clone, Copy)]
pub struct DropAt {
    pub client: usize,
    pub at_s: f64,
}

/// Everything that varies between the TR-25 scenarios.
#[derive(Debug, Clone)]
pub struct MatchOpts {
    /// Human clients (1 host + N-1 guests); bots pad the roster to 8.
    pub clients: usize,
    /// The link SIM (`None` = lagless).
    pub sim: Option<LinkSim>,
    /// Scenario seed (per-endpoint RNG seeds derive from it — `run_fake`
    /// convention, lib.rs L419-420).
    pub seed: u64,
    /// Match clock length in seconds (test-only shortening, ≡ the `duration
    /// = 600 / 40` override in `net-test.mjs` L85).
    pub duration: f64,
    pub drop: Option<DropAt>,
    /// Collect the kink / path-error smoothness stats (TR-25.2).
    pub stats: bool,
}

impl MatchOpts {
    #[must_use]
    pub fn plain(clients: usize, seed: u64) -> Self {
        Self {
            clients,
            sim: None,
            seed,
            duration: 15.0,
            drop: None,
            stats: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-client view + the consistency report
// ---------------------------------------------------------------------------

/// What one client ended the match believing.
#[derive(Debug, Clone)]
pub struct ClientView {
    pub name: String,
    /// Whether this client survived to the settlement (dropped ones are
    /// excluded from every consistency check).
    pub live: bool,
    /// `paint.coverage()` at the settlement snapshot (`net-test.mjs` L144
    /// `P.coverage()` — fractions per team).
    pub coverage: [f32; 2],
    /// FNV over the final paint grid bytes (determinism diffing).
    pub grid_fnv: u64,
    /// The settled result (`{k:'res'}` on guests, judged locally on the host).
    pub result: Option<MatchResult>,
    /// Session state after the end handshake.
    pub state: SessionState,
    /// The link RTT at the end (transport EMA, ms).
    pub rtt: f64,
    /// Host-migration adoption flag (`Replication::adopted_host`).
    pub adopted_host: bool,
}

/// The outcome of one `run_match`, assertable and printable (the CLI prints
/// it; the tests assert on the same values — the `Report` convention).
#[derive(Debug)]
pub struct MatchReport {
    pub clients: Vec<ClientView>,
    pub checks: Vec<(&'static str, bool)>,
    pub stats: Option<SmoothStats>,
    /// Substeps actually run.
    pub frames: usize,
    /// Guest-drop scenario (TR-25.3): did the orphaned actor keep playing on
    /// the live host after `on_leave` adopted it as a bot (`None` otherwise).
    pub dropped_actor_moved: Option<bool>,
}

impl MatchReport {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|(_, v)| *v)
    }

    pub fn print(&self) {
        println!("=== net-harness match report ===");
        for (i, c) in self.clients.iter().enumerate() {
            println!(
                "  client[{i}] {:<6} live={} cov={:.2}%/{:.2}% winner={:?} state={:?} rtt={:.1}ms adopted={}",
                c.name,
                c.live,
                f64::from(c.coverage[0]) * 100.0,
                f64::from(c.coverage[1]) * 100.0,
                c.result.map(|r| r.winner),
                c.state,
                c.rtt,
                c.adopted_host,
            );
        }
        println!("  frames: {}", self.frames);
        for (name, pass) in &self.checks {
            println!("  [{}] {name}", if *pass { "PASS" } else { "FAIL" });
        }
        println!(
            "  overall: {}",
            if self.ok() {
                "CONSISTENT"
            } else {
                "INCONSISTENT"
            }
        );
        if let Some(moved) = self.dropped_actor_moved {
            println!("  dropped actor kept playing on the host: {moved}");
        }
        if let Some(s) = &self.stats {
            s.print();
        }
    }
}

// ---------------------------------------------------------------------------
// Smoothness statistics (TR-25.2) — verbatim ports of tools/net-test.mjs
// ---------------------------------------------------------------------------

const MS_PER_FRAME: f64 = 1000.0 / 60.0;

/// One recorded actor frame. Field set ≡ `net-test.mjs` L100-104 minus the
/// render-only entries (visible/flags/debug); `sj` (super jump) is always
/// false — the M1 Rust sim has no super-jump state (replication.rs L1210-1212
/// deviation), so the JS `!sj` guards are structurally satisfied.
#[derive(Clone, Copy)]
struct Frame {
    t: f64,
    x: f64,
    y: f64,
    z: f64,
    yaw: f64,
    alive: bool,
    tp: u32,
    remote: bool,
}

/// `bpct/pct` (net-test.mjs L242): sorted, `floor(p·n)` clamped to the last.
#[must_use]
fn pct(arr: &[f64], p: f64) -> f64 {
    if arr.is_empty() {
        return 0.0;
    }
    let mut s = arr.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[((p * s.len() as f64) as usize).min(s.len() - 1)]
}

/// One percentile summary (`bdist`, net-test.mjs L258, extended with p95 per
/// the Task 25 spec wording).
#[derive(Debug, Clone, Copy)]
pub struct Dist {
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Dist {
    #[must_use]
    fn of(arr: &[f64], unit: f64) -> Self {
        Self {
            p50: pct(arr, 0.5) * unit,
            p90: pct(arr, 0.9) * unit,
            p95: pct(arr, 0.95) * unit,
            p99: pct(arr, 0.99) * unit,
            max: arr.iter().fold(0.0f64, |a, &b| f64::max(a, b)) * unit,
        }
    }

    fn fmt_line(&self, name: &str, unit: &str) -> String {
        format!(
            "{name} p50 {:.1} · p90 {:.1} · p95 {:.1} · p99 {:.1} · max {:.1} {unit}",
            self.p50, self.p90, self.p95, self.p99, self.max
        )
    }
}

/// Frame-to-frame "kink" (net-test.mjs L244-257): how far this frame's step
/// breaks from the previous step, scaled to the frame times; steady frames
/// only (a renderer hitch is not the netcode).
fn kink_seq(tr: &[Frame], out_p: &mut Vec<f64>, out_y: &mut Vec<f64>) {
    for k in 2..tr.len() {
        let (a, b, c) = (tr[k - 2], tr[k - 1], tr[k]);
        // L247: dead frames or a respawn (tp change) between a and c break
        // the comparison; the sj terms are always false (see `Frame`).
        if !a.alive || !b.alive || !c.alive || a.tp != c.tp {
            continue;
        }
        let t1 = b.t - a.t;
        let t2 = c.t - b.t;
        // L249: steady frames only.
        if t1 < 6.0 || t2 < 6.0 || t1 > 45.0 || t2 > 45.0 {
            continue;
        }
        let s = t2 / t1;
        let dx = (c.x - b.x) - (b.x - a.x) * s;
        let dz = (c.z - b.z) - (b.z - a.z) * s;
        let dy = ((c.y - b.y) - (b.y - a.y) * s) * 0.5;
        out_p.push((dx * dx + dz * dz + dy * dy).sqrt());
        // L254-255: turn kink on the wrapped yaw deltas.
        let w = |x: f64| x.sin().atan2(x.cos());
        out_y.push((w(c.yaw - b.yaw) - w(b.yaw - a.yaw) * s).abs());
    }
}

/// `sample` (net-test.mjs L233-241): the owner's position at wall time `t`
/// (linear between its frames); a respawn / dead / super-jump bracket is
/// skipped.
fn sample(tr: &[Frame], t: f64) -> Option<(f64, f64, f64)> {
    if tr.len() < 2 || t <= tr[0].t || t >= tr[tr.len() - 1].t {
        return None;
    }
    let hi = tr.partition_point(|f| f.t <= t);
    if hi == 0 || hi >= tr.len() {
        return None;
    }
    let (a, b) = (tr[hi - 1], tr[hi]);
    if b.tp != a.tp || !a.alive || !b.alive {
        return None;
    }
    let k = (t - a.t) / (b.t - a.t);
    Some((
        a.x + (b.x - a.x) * k,
        a.y + (b.y - a.y) * k,
        a.z + (b.z - a.z) * k,
    ))
}

/// The per-2 s delay fit + path error (net-test.mjs L280-291): the adaptive
/// delay moves, so fit `lag ∈ [40, 400] ms` (step 5) against the owner's
/// track per window, then measure the distance from the owner's path at
/// that delay.
fn fit_path_error(tr: &[Frame], tru: &[Frame], lags: &mut Vec<f64>, errs: &mut Vec<f64>) {
    // L278: an owner track this client never saw move enough is unmeasurable.
    if tru.len() < 30 {
        return;
    }
    let t_end = tr[tr.len() - 1].t;
    let mut w0 = tr[0].t;
    while w0 < t_end {
        let seg: Vec<Frame> = tr
            .iter()
            .filter(|f| f.t >= w0 && f.t < w0 + 2000.0 && f.alive)
            .copied()
            .collect();
        if seg.len() >= 30 {
            let mut best: Option<(f64, f64)> = None;
            let mut lag = 40.0;
            while lag <= 400.0 {
                let (mut e2, mut n) = (0.0, 0usize);
                for f in &seg {
                    if let Some(p) = sample(tru, f.t - lag) {
                        e2 += (f.x - p.0) * (f.x - p.0)
                            + (f.z - p.2) * (f.z - p.2)
                            + (f.y - p.1) * (f.y - p.1);
                        n += 1;
                    }
                }
                if n > 20 && best.is_none_or(|(_, be)| (e2 / (n as f64)) < be) {
                    best = Some((lag, e2 / n as f64));
                }
                lag += 5.0;
            }
            if let Some((bl, _)) = best {
                lags.push(bl);
                for f in &seg {
                    if let Some(p) = sample(tru, f.t - bl) {
                        errs.push(
                            ((f.x - p.0) * (f.x - p.0)
                                + (f.z - p.2) * (f.z - p.2)
                                + (f.y - p.1) * (f.y - p.1))
                                .sqrt(),
                        );
                    }
                }
            }
        }
        w0 += 2000.0;
    }
}

/// Teleported frames (net-test.mjs L275): a > 1.2 m jump between consecutive
/// alive frames outside a respawn / super jump.
#[must_use]
fn teleports(tr: &[Frame]) -> usize {
    let mut n = 0;
    for k in 1..tr.len() {
        let (a, b) = (tr[k - 1], tr[k]);
        if a.alive && b.alive && a.tp == b.tp {
            let d = ((b.x - a.x) * (b.x - a.x) + (b.z - a.z) * (b.z - a.z)).sqrt();
            if d > 1.2 {
                n += 1;
            }
        }
    }
    n
}

/// Smoothness summary for one observing client.
#[derive(Debug)]
pub struct ClientStats {
    pub client: usize,
    pub remote_actors: usize,
    pub frames: usize,
    pub teleports: usize,
    pub kink: Dist,
    pub turn_kink: Dist,
    pub lags: Dist,
    pub path_err: Dist,
}

/// The full TR-25.2 statistics bundle.
#[derive(Debug)]
pub struct SmoothStats {
    /// The owners' own frames baseline (net-test.mjs L259-261).
    pub owner_kink: Dist,
    pub owner_turn_kink: Dist,
    pub per_client: Vec<ClientStats>,
}

impl SmoothStats {
    pub fn print(&self) {
        println!("  --- smoothness (net-test.mjs 口径, 虚拟 60 Hz 帧)");
        println!("    {}", self.owner_kink.fmt_line("owners' own kink", "cm"));
        println!(
            "    {}",
            self.owner_turn_kink.fmt_line("owners' own turn-kink", "°")
        );
        for c in &self.per_client {
            println!(
                "    c{} · {} remote squidkids · {} frames · teleports {}",
                c.client, c.remote_actors, c.frames, c.teleports
            );
            println!("      {}", c.kink.fmt_line("kink", "cm"));
            println!("      {}", c.turn_kink.fmt_line("turn-kink", "°"));
            println!(
                "      {}   delay p50 {:.0} ms",
                c.path_err.fmt_line("path error", "cm"),
                c.lags.p50
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The scenario driver
// ---------------------------------------------------------------------------

/// One drained event through the app `net_tick` dispatch contract
/// (`match_link.rs` L252-305): `Host` → `set_host` + flag, `Leave` →
/// `on_leave(flag)` + slot re-resolve, `GameMessage` → `on_message`,
/// `MatchEnd` → `end_match`.
#[allow(clippy::too_many_arguments)] // the per-client state the app threads through `net_tick`
fn dispatch(
    i: usize,
    ev: SessionEvent,
    sess: &mut [HSess],
    rep: &mut [Replication],
    m: &mut [Match],
    inputs: &mut [Vec<ActorInput>],
    local_idx: &mut [usize],
    host_changed: &mut [bool],
    gone: &[bool],
    sim: &SimWorld,
    tuning: &Tuning,
    seed: u64,
) {
    match ev {
        SessionEvent::Host { host_id } => {
            if gone[i] {
                return;
            }
            // session.js L156-157: the re-elected host lands *before*
            // `onLeave` runs (replication.rs L200-202 contract).
            rep[i].set_host(&host_id);
            host_changed[i] = true;
        }
        SessionEvent::Leave { player, .. } => {
            if gone[i] {
                return;
            }
            let before = m[i].actors.len();
            rep[i].on_leave(
                &player.id,
                host_changed[i],
                &mut m[i],
                &tuning.difficulty.easy,
                seed,
                sim,
                &tuning.player,
            );
            host_changed[i] = false;
            // Review m-8 (`match_link.rs` L260-290): a humans-only prune
            // shifts the local slot; re-resolve and resize the input row.
            if m[i].actors.len() != before
                && let Some(li) = rep[i].local_slot(sess[i].my_id().unwrap_or_default())
            {
                local_idx[i] = li;
                inputs[i].resize(m[i].actors.len(), ActorInput::default());
            }
        }
        SessionEvent::GameMessage { from, payload } => {
            if gone[i] {
                return;
            }
            if rep[i].on_message(&from, &payload, &mut m[i], &tuning.player)
                == Some(ReplicationEvent::MatchEnd)
            {
                // JS netmatch.js L218 `case 'end'` → `netMatchEnd()`.
                sess[i].end_match();
            }
        }
        _ => {}
    }
}

/// Run one full-stack match scenario and return the consistency report.
/// Panics only on a broken harness precondition (the scenario's own
/// failures land in `checks`).
pub fn run_match(opts: &MatchOpts) -> MatchReport {
    let n = opts.clients;
    let tuning = embedded_tuning();
    let tp = &tuning.player;
    let sp = &tuning.spritzer;
    let layout = embedded_tidewater();
    let world = CollisionWorld::from_layout(&layout);
    let sim = SimWorld::new(&world, &layout);
    let nav = inkwave_sim::nav::NavGraph::new(&world, &layout, tp);

    // -- lobby: create, join, ready, shortened duration, start ---------------
    let relay = FakeRelay::new();
    let mut sess: Vec<HSess> = (0..n)
        .map(|i| {
            hsess(
                &relay,
                opts.sim,
                opts.seed.wrapping_mul(31).wrapping_add(i as u64 + 1),
            )
        })
        .collect();
    sess[0].create(Some("Host"));
    for _ in 0..50 {
        let _ = drain(&mut sess, 10.0);
        if sess[0].state() == SessionState::Lobby {
            break;
        }
    }
    let code = sess[0].code().expect("room code").to_string();
    for (i, s) in sess.iter_mut().enumerate().skip(1) {
        s.join(&code, Some(&format!("P{}", i + 1)));
    }
    for _ in 0..200 {
        let _ = drain(&mut sess, 10.0);
        if sess.iter().all(|s| s.lobby().players.len() == n) {
            break;
        }
    }
    assert_eq!(
        sess[0].lobby().players.len(),
        n,
        "every client reached the lobby"
    );
    for s in sess.iter_mut().skip(1) {
        s.set_me(MeChange {
            ready: Some(true),
            ..Default::default()
        });
    }
    sess[0].set_settings(SettingsPatch {
        duration: Some(opts.duration),
        ..Default::default()
    });
    for _ in 0..200 {
        let _ = drain(&mut sess, 10.0);
        if sess[0].can_start() {
            break;
        }
    }
    assert!(sess[0].can_start(), "the room is ready to launch");
    assert!(sess[0].start(), "the host starts the match");

    // -- `Match{Start}` on every client, then build each side -----------------
    let mut got_start = vec![false; n];
    for _ in 0..200 {
        for (i, ev) in drain(&mut sess, 10.0) {
            if matches!(
                ev,
                SessionEvent::Match {
                    phase: MatchPhase::Start
                }
            ) {
                got_start[i] = true;
            }
        }
        if got_start.iter().all(|g| *g) {
            break;
        }
    }
    assert!(got_start.iter().all(|g| *g), "every client saw the start");
    let mut cfg = sess[0].start_cfg().expect("host cfg").clone();
    // Test-only shortening ≡ net-test.mjs L85 (`duration = 600 / 40`): the
    // lobby path clamps to ≥ 60 (session.js L241, ported at session.rs
    // L1284-1290), so the harness overrides the *local* cfg every side
    // builds from — the same seam the JS harness uses, not a lobby patch.
    cfg.duration = opts.duration.max(1.0) as i64;
    assert_eq!(cfg.roster.len(), 8, "bots pad the roster to 8");
    let seed = fnv1a(&cfg.id);

    let mut m: Vec<Match> = Vec::new();
    let mut rep: Vec<Replication> = Vec::new();
    let mut local_idx: Vec<usize> = Vec::new();
    for s in &mut sess {
        let (mm, rr, li) = build_side(&cfg, s, &sim, tp, &tuning);
        m.push(mm);
        rep.push(rr);
        local_idx.push(li);
    }
    let mut inputs: Vec<Vec<ActorInput>> = m
        .iter()
        .map(|mm| vec![ActorInput::default(); mm.actors.len()])
        .collect();
    let mut host_changed = vec![false; n];
    let mut result_sent = vec![false; n];
    let mut gone = vec![false; n];

    // -- `mark_loaded` ⇒ the go handshake ⇒ `Match{Go}` -----------------------
    for s in &mut sess {
        s.mark_loaded();
    }
    let mut got_go = vec![false; n];
    for _ in 0..400 {
        for (i, ev) in drain(&mut sess, 10.0) {
            if matches!(
                ev,
                SessionEvent::Match {
                    phase: MatchPhase::Go
                }
            ) {
                got_go[i] = true;
            }
        }
        if got_go.iter().all(|g| *g) {
            break;
        }
    }
    assert!(got_go.iter().all(|g| *g), "the go handshake completed");
    // Deviation ③ (Task 24 plan): the Rust match enters `intro` at
    // construction; `netMatchGo` re-enters it on every client (main.js
    // L816-821), exactly like the tr24 flow test.
    for mm in &mut m {
        mm.set_phase(Phase::Intro);
    }

    // -- the lockstep match loop ----------------------------------------------
    let max_frames =
        ((f64::from(INTRO_TIME) + opts.duration + f64::from(FINISH_TIME)) * 60.0) as usize + 900;
    let drop_frame = opts.drop.map(|d| (d.at_s * 60.0) as usize);
    // Stats buffers: `own[j]` = frames of actor `j` from its owning client;
    // `remote[c][j]` = client `c`'s rendered frames of a non-owned actor
    // (≡ `own` / `tracks` in net-test.mjs L224-268).
    let mut own: Vec<Vec<Frame>> = Vec::new();
    let mut remote: Vec<HashMap<usize, Vec<Frame>>> = vec![HashMap::new(); n];
    let mut frame = 0usize;
    let mut host_settled = false;
    // Guest drop: snapshot the abandoned actor on the live host at the drop
    // frame; settlement checks it kept moving after the bot adoption.
    // Index identity `local_idx[guest]` == the host's slot for that actor
    // relies on the bots stage (`map_no_bots == false`): `on_leave` adopts
    // (mutates `owners`, replication.rs L1158-1174) instead of `remove_at`,
    // so no left-shift can happen (review A5).
    let mut drop_track: Option<(usize, usize, (f32, f32, f32))> = None;
    while frame < max_frames && !host_settled {
        // The drop-out fires once the match clock has run `at_s` seconds.
        if let Some(d) = opts.drop
            && !gone[d.client]
            && frame >= drop_frame.unwrap_or(usize::MAX)
            && m[d.client].phase == Phase::Active
        {
            if let Some(tr) = sess[d.client].transport_mut() {
                tr.close();
            }
            gone[d.client] = true;
            if d.client != 0
                && let Some(h) = (0..n).find(|x| !gone[*x] && rep[*x].is_host())
            {
                let li = local_idx[d.client];
                if let Some(a) = m[h].actors.get(li) {
                    drop_track = Some((h, li, (a.pos.x, a.pos.y, a.pos.z)));
                }
            }
        }
        for i in 0..n {
            if gone[i] {
                continue;
            }
            rep[i].drive_bots(&mut m[i], &nav, &sim, tp, sp, &mut inputs[i]);
            m[i].step_masked(
                FIXED_DT,
                &inputs[i],
                Some(rep[i].remote_mask()),
                &sim,
                tp,
                sp,
            );
            let rest = rep[i].update(
                FIXED_DT,
                &mut m[i],
                &sim,
                tp,
                sp,
                sess[i].transport_mut().expect("live transport"),
            );
            m[i].events.extend(rest);
            if opts.stats {
                let t_ms = frame as f64 * MS_PER_FRAME;
                for (j, a) in m[i].actors.iter().enumerate() {
                    let f = Frame {
                        t: t_ms,
                        x: f64::from(a.pos.x),
                        y: f64::from(a.pos.y),
                        z: f64::from(a.pos.z),
                        yaw: f64::from(a.yaw),
                        alive: a.alive,
                        tp: a.net_tp,
                        remote: rep[i].remote_mask()[j],
                    };
                    if f.remote {
                        remote[i].entry(j).or_default().push(f);
                    } else {
                        while own.len() <= j {
                            own.push(Vec::new());
                        }
                        own[j].push(f);
                    }
                }
            }
        }
        for (i, ev) in drain(&mut sess, f64::from(FIXED_DT) * 1000.0) {
            dispatch(
                i,
                ev,
                &mut sess,
                &mut rep,
                &mut m,
                &mut inputs,
                &mut local_idx,
                &mut host_changed,
                &gone,
                &sim,
                &tuning,
                seed,
            );
        }
        // The authoritative clock is the live host's: once it judges, the
        // settlement bus takes over.
        if let Some(h) = (0..n).find(|i| !gone[*i] && rep[*i].is_host()) {
            host_settled = m[h].result.is_some();
        }
        frame += 1;
    }

    // -- settlement: the host's `res` rides the bus to every guest ------------
    let host_i = (0..n)
        .find(|i| !gone[*i] && rep[*i].is_host())
        .expect("a live host");
    if m[host_i].result.is_some() && !result_sent[host_i] {
        rep[host_i].send_result(&m[host_i], sess[host_i].transport_mut().unwrap());
        result_sent[host_i] = true;
    }
    for _ in 0..2000 {
        let all_res = m
            .iter()
            .enumerate()
            .all(|(i, mm)| gone[i] || mm.result.is_some());
        if all_res {
            break;
        }
        for (i, ev) in drain(&mut sess, 10.0) {
            if let SessionEvent::GameMessage { from, payload } = ev
                && !gone[i]
            {
                rep[i].on_message(&from, &payload, &mut m[i], tp);
            }
        }
    }

    // The settlement snapshot (≡ net-test.mjs L142-150 `views`).
    let mut views: Vec<ClientView> = Vec::new();
    for i in 0..n {
        views.push(ClientView {
            name: format!("c{i}"),
            live: !gone[i],
            coverage: m[i].paint.coverage(),
            grid_fnv: fnv1a_bytes(m[i].paint.grid()),
            result: m[i].result,
            state: sess[i].state(),
            rtt: sess[i].transport_mut().map_or(0.0, |t| t.rtt()),
            adopted_host: rep[i].adopted_host,
        });
    }

    // -- end handshake: host `end` ⇒ everyone back in the lobby ---------------
    rep[host_i].send_end(sess[host_i].transport_mut().unwrap());
    sess[host_i].end_match();
    for _ in 0..2000 {
        for (i, ev) in drain(&mut sess, 10.0) {
            if let SessionEvent::GameMessage { from, payload } = ev
                && !gone[i]
                && rep[i].on_message(&from, &payload, &mut m[i], tp)
                    == Some(ReplicationEvent::MatchEnd)
            {
                sess[i].end_match();
            }
        }
        if (0..n).all(|i| gone[i] || sess[i].state() == SessionState::Lobby) {
            break;
        }
    }
    for (i, v) in views.iter_mut().enumerate() {
        v.state = sess[i].state();
    }

    // -- checks ----------------------------------------------------------------
    let live: Vec<usize> = (0..n).filter(|i| !gone[*i]).collect();
    let mut checks: Vec<(&'static str, bool)> = Vec::new();

    let all_judged = live.iter().all(|i| m[*i].result.is_some());
    checks.push(("every live client settled a result", all_judged));

    // Coverage agreement: pairwise, per team, < 0.5 percentage points
    // (tasks.md TR-25.1; the JS report prints `cov*100` to 2 decimals,
    // net-test.mjs L154).
    let mut cov_spread = 0.0f32;
    for &a in &live {
        for &b in &live {
            let ca = m[a].paint.coverage();
            let cb = m[b].paint.coverage();
            cov_spread = cov_spread
                .max((ca[0] - cb[0]).abs())
                .max((ca[1] - cb[1]).abs());
        }
    }
    checks.push((
        "paint coverage agrees across ends (<0.5pp)",
        cov_spread < 0.005,
    ));

    // Results identical to the host's `res` (tasks.md TR-25.1). The wire
    // `{k:'res'}` is authoritative for `coverage` / `winner` / the stat rows
    // and the clock (netmatch.js L652-660), so those match *exactly*;
    // `points` is rebuilt from the integer-quantized turf rows — the
    // documented guest-side deviation (replication.rs L80-84). The rounding
    // bound: `wire::round_int` per actor (replication.rs L1119) is ±0.5 m²,
    // × `pointsPerM2 = 1` (tuning.json L24) over 4 actors per team ⇒ the
    // per-team rebuild error is < 2.0 points (review A3).
    let host_res = m[host_i].result.expect("host judged");
    let res_ok = live.iter().all(|i| {
        m[*i].result.is_some_and(|r| {
            r.winner == host_res.winner
                && r.coverage == host_res.coverage
                && r.elapsed == host_res.elapsed
                && (r.points[0] - host_res.points[0]).abs() < 2.0 + 1e-3
                && (r.points[1] - host_res.points[1]).abs() < 2.0 + 1e-3
        })
    });
    checks.push(("every live result equals the host res", res_ok));

    let lobby_ok = live.iter().all(|i| sess[*i].state() == SessionState::Lobby);
    checks.push(("every live session returned to the lobby", lobby_ok));

    let painted = host_res.coverage[0] + host_res.coverage[1] > 0.01;
    checks.push(("the bots actually painted turf", painted));

    // Guest drop: the abandoned actor must still be in the host's roster
    // (adoption, not removal) and must have kept moving as a bot.
    let dropped_actor_moved = drop_track.map(|(h, li, p0)| {
        m[h].actors.get(li).is_some_and(|a| {
            ((a.pos.x - p0.0) * (a.pos.x - p0.0)
                + (a.pos.y - p0.1) * (a.pos.y - p0.1)
                + (a.pos.z - p0.2) * (a.pos.z - p0.2))
                .sqrt()
                > 0.05
        })
    });
    if opts.drop.is_some() {
        let migrated = match opts.drop {
            Some(d) if d.client == 0 => live.iter().any(|i| rep[*i].adopted_host),
            Some(_) => dropped_actor_moved == Some(true),
            None => true,
        };
        checks.push((
            "the drop-out scenario settled cleanly",
            migrated && all_judged,
        ));
    }

    let stats = opts.stats.then(|| compute_stats(&own, &remote));
    MatchReport {
        clients: views,
        checks,
        stats,
        frames: frame,
        dropped_actor_moved,
    }
}

/// The `start_net_match` build sequence (JS `startNetMatch` main.js L770-807
/// plus the `NetMatch` ctor session.js L308) — verbatim the tr24 `build_side`
/// helper (`match_link.rs` L822-849), now shared through
/// [`inkwave_sim::net::matchbuild`].
fn build_side(
    cfg: &StartCfg,
    s: &mut HSess,
    sim: &SimWorld,
    tp: &PlayerTuning,
    tuning: &Tuning,
) -> (Match, Replication, usize) {
    let my = s.my_id().expect("my_id").to_string();
    let host = s.host_id().expect("host_id").to_string();
    let seed = fnv1a(&cfg.id);
    let mut m = build_net_match(cfg, seed, sim, tp, &tuning.match_config);
    let mut r = Replication::new(cfg, &my, &host);
    r.bind_match(&mut m);
    for (i, e) in cfg.roster.iter().enumerate() {
        if e.bot {
            r.attach_bot(i, &m.actors[i], &tuning.difficulty.easy, seed);
        }
    }
    let members: Vec<String> = s.member_ids();
    r.prune_loading_leavers(&mut m, &members);
    let li = local_roster_index(&cfg.roster, &my, &members, map_no_bots(&cfg.map));
    (m, r, li)
}

/// FNV-1a over the grid bytes (determinism diffing without shipping the
/// whole grid in the report).
fn fnv1a_bytes(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Assemble the TR-25.2 statistics from the recorded frames.
fn compute_stats(own: &[Vec<Frame>], remote: &[HashMap<usize, Vec<Frame>>]) -> SmoothStats {
    // Owners' own baseline (net-test.mjs L259-261).
    let mut own_p = Vec::new();
    let mut own_y = Vec::new();
    for tr in own {
        kink_seq(tr, &mut own_p, &mut own_y);
    }
    let mut per_client = Vec::new();
    for (c, tracks) in remote.iter().enumerate() {
        // JS pools these per client as `P` / `Y` (net-test.mjs L262-267).
        let (mut p, mut y, mut lags, mut errs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut tele = 0usize;
        let mut frames = 0usize;
        // Deterministic iteration (HashMap order would not change the
        // pooled distributions, but keeps the per-client line stable).
        let mut keys: Vec<usize> = tracks.keys().copied().collect();
        keys.sort_unstable();
        for j in keys {
            let tr = &tracks[&j];
            frames += tr.len();
            kink_seq(tr, &mut p, &mut y);
            tele += teleports(tr);
            let tru = own.get(j).map(Vec::as_slice).unwrap_or(&[]);
            fit_path_error(tr, tru, &mut lags, &mut errs);
        }
        per_client.push(ClientStats {
            client: c,
            remote_actors: tracks.len(),
            frames,
            teleports: tele,
            kink: Dist::of(&p, 100.0),
            turn_kink: Dist::of(&y, 57.3),
            lags: Dist::of(&lags, 1.0),
            path_err: Dist::of(&errs, 100.0),
        });
    }
    SmoothStats {
        owner_kink: Dist::of(&own_p, 100.0),
        owner_turn_kink: Dist::of(&own_y, 57.3),
        per_client,
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the ported formulas
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(t: f64, x: f64, z: f64) -> Frame {
        Frame {
            t,
            x,
            y: 0.0,
            z,
            yaw: 0.0,
            alive: true,
            tp: 0,
            remote: false,
        }
    }

    #[test]
    fn pct_matches_the_js_selection_rule() {
        // net-test.mjs L242: sorted, `floor(p·n)` clamped to the last index.
        let a: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert_eq!(pct(&a, 0.5), 50.0);
        assert_eq!(pct(&a, 0.95), 95.0);
        assert_eq!(pct(&a, 0.99), 99.0);
        assert_eq!(pct(&[], 0.5), 0.0);
    }

    #[test]
    fn kink_is_zero_on_a_steady_line_and_spikes_on_a_snap() {
        // Constant velocity ⇒ every step matches the previous one ⇒ kink 0.
        let straight: Vec<Frame> = (0..10)
            .map(|k| fr(k as f64 * MS_PER_FRAME, k as f64, 0.0))
            .collect();
        let mut p = Vec::new();
        let mut y = Vec::new();
        kink_seq(&straight, &mut p, &mut y);
        assert!(!p.is_empty());
        assert!(p.iter().all(|v| *v < 1e-9), "steady line: {p:?}");

        // One frame teleports sideways ⇒ a large kink at that corner.
        let mut snap: Vec<Frame> = straight.clone();
        snap[5].x += 0.5;
        let mut p2 = Vec::new();
        kink_seq(&snap, &mut p2, &mut y);
        assert!(p2.iter().any(|v| *v > 0.3), "snap kink: {p2:?}");
    }

    #[test]
    fn kink_skips_dead_and_respawned_brackets() {
        let mut tr: Vec<Frame> = (0..6)
            .map(|k| fr(k as f64 * MS_PER_FRAME, k as f64, 0.0))
            .collect();
        tr[3].tp = 1; // respawn between a and c
        let mut p = Vec::new();
        let mut y = Vec::new();
        kink_seq(&tr, &mut p, &mut y);
        // The windows whose a/c straddle the tp change are skipped (JS only
        // compares `a[6] !== c[6]`, L247).
        assert_eq!(p.len(), 2, "only the clean windows survive: {p:?}");
        tr[3].alive = false;
        let mut p2 = Vec::new();
        kink_seq(&tr, &mut p2, &mut y);
        assert!(p2.iter().all(|v| *v < 1e-9));
    }

    #[test]
    fn sample_interpolates_and_guards_brackets() {
        let tr: Vec<Frame> = (0..4).map(|k| fr(k as f64 * 16.0, k as f64, 0.0)).collect();
        let p = sample(&tr, 24.0).expect("inside");
        assert!((p.0 - 1.5).abs() < 1e-9);
        assert!(sample(&tr, -1.0).is_none(), "before the track");
        assert!(sample(&tr, 50.0).is_none(), "after the track");
    }

    #[test]
    fn teleport_counts_only_clean_big_jumps() {
        let tr = vec![fr(0.0, 0.0, 0.0), fr(16.0, 2.0, 0.0)];
        assert_eq!(teleports(&tr), 1);
        let respawn = vec![fr(0.0, 0.0, 0.0), {
            let mut f = fr(16.0, 2.0, 0.0);
            f.tp = 1;
            f
        }];
        assert_eq!(teleports(&respawn), 0);
    }
}
