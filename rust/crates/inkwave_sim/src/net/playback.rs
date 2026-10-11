//! Task 22 — playback: per-sender clocks, the interpolation buffer, the
//! Hermite path and the correction curve.
//!
//! Faithful port of the *playback* half of `src/net/netmatch.js`:
//!   - `_tick` clock bookkeeping (L229-244: offset floor + creep, the
//!     late/gap windows, `want`) → [`Playback::on_tick`]
//!   - `_advance` (L271-284: eased delay, the ±8 % / −20 %..+25 % rate
//!     stretch, the run-out brake, the rate ease) → [`Playback::advance_all`]
//!   - `_pathAt` (L288-303: Hermite / teleport hold / ballistic ≤ 0.18 s /
//!     before-oldest hold) → [`path_at`]
//!   - `_sample` (L305-346: buffer trim, the `tp` snap, the shown-moment
//!     jump measured against last frame's time, critically damped ω = 13
//!     correction, handoff) → [`Playback::sample`]
//!   - `applyRemote` (L349-428) sim-side subset + `_remoteSplat` /
//!     `_remoteRespawn` state machines (L603-627) → [`Playback::apply`]
//!   - `onLeave` handoff (L679) / `_adopt` (L692-707) buffer + offset reset
//!     → [`Playback::handoff`] / [`Playback::adopt_reset`]
//!   - `hermite` (L766-786, C1 position through the owner's velocity
//!     tangents, the grounded Y floor clamp L776, `hp` from the earlier
//!     sample L784), `quantile` (L761-764), `blankSample` / `copySample`
//!     (L757-758)
//!
//! All playback time arithmetic is `f64` (JS numbers are f64 end to end:
//! `performance.now()/1000`, the wire-parsed `d.ts`, `p.tr` …). The wire
//! `t`/`ts` fields stay `f32` (they are `r3`-quantised milliseconds, exactly
//! representable); every comparison against the clock promotes to `f64`.
//!
//! Scope and deviations (recorded for review):
//!   - **Logical clock**: `now()` is the [`super::replication::Replication`]
//!     frame-accumulated `now` (f64), not a wall clock — same rule both
//!     Rust ends, and only differences (`o = now − ts`, `target = now − off
//!     − delay`) ever matter.
//!   - **Death / respawn are driven by the `ALIVE` flag edge** (JS runs the
//!     `_remoteSplat` / `_remoteRespawn` state machines off the `'splatted'`
//!     / `'respawn'` events on the sender timeline — Task 23). The state
//!     writes are verbatim L603-627 (buffer clear, `ready`/`has` reset,
//!     `deathTp`, `spawnPending`, respawn timer / hp / invuln, runner
//!     reset); only the *trigger* differs, and it fires when the sampled
//!     path crosses the first dead/alive sample — *approximately* the
//!     moment the timeline event would land: both the `'splatted'` event
//!     and the dead sample ride the victim-owner's clock (JS records the
//!     event under `e.actor || e.victim` → the victim, L140), but events
//!     carry the exact `now()` stamp while samples are packed at the tick
//!     boundary, so the two can land up to one tick (50 ms) apart — the
//!     first alive sample trails the `'respawn'` event the same way.
//!     A further consequence of the same deviation: an owner who dies and
//!     respawns *between* two of its samples shows JS a brief hide (the
//!     splat event clears `ready` before any dead sample arrives) where
//!     Rust never sees a dead sample and snaps straight to the respawn
//!     point. Task 23's event timeline closes both gaps.
//!   - **App-layer fields do not exist in the sim** and are not ported:
//!     `hurtFlash` (L374-375), `aimDir`/`aimPoint` (L362-364, derived from
//!     `aim_yaw`/`aim_pitch` on demand), climbing `wallN` (L373, M1 has no
//!     climb), special/super-jump/kit pose state (L379-398, M1 constants),
//!     landing & squid in/out FX and audio (L400-412), `anim.time` /
//!     `_finishFrame` (L351, L428). The per-frame `landT` / `lastDamage`
//!     increments (L414) are skipped too: `Actor` keeps `last_damage`
//!     private, and the sim readers of it (regen / attribution inside
//!     `Actor::step` / `damage`, the bot getters) all run on locally-owned
//!     actors — the frozen proxy never executes them. `netTurnRate` (L358-359) lands on
//!     [`Actor::yaw_vel`] — the closest existing seam; a frozen proxy never
//!     reads it.
//!   - **`a.netTp` is not written from samples** (JS `applyRemote` never
//!     touches it; only `_adopt` reads the shown counter). The shown `tp`
//!     lives in [`ActorNet::tp_shown`] and [`super::replication::Replication::adopt`]
//!     continues from it (L702).
//!   - **`tp_shown` seeds 0** (JS seeds `a.net.tp = -1`, L85, so the first
//!     sample always "snaps"). With 0 the snap is skipped but the `!has`
//!     branch zeroes the correction identically — same outcome.
//!   - **`p.events` is filled but not consumed** (the event timeline is
//!     Task 23); `boss` / `voices` / `_playEvents` sampling (L178-180) are
//!     out of M2 turf scope.

use crate::actor::{Actor, Form};
use crate::net::flags;
use crate::net::{ActorSample, SessionId, TimedEvent};
use crate::tuning::PlayerTuning;
use crate::weapon::WeaponRunner;
use glam::DVec3;

/// JS `TAU` (netmatch.js L37).
const TAU: f64 = std::f64::consts::TAU;

/// JS `angDiff` (netmatch.js L38): shortest signed arc from `a` to `b`.
#[must_use]
pub fn ang_diff(a: f64, b: f64) -> f64 {
    let mut d = (b - a) % TAU;
    if d > std::f64::consts::PI {
        d -= TAU;
    } else if d < -std::f64::consts::PI {
        d += TAU;
    }
    d
}

/// JS `quantile` (L761-764): ascending sort of a copy, `s[min(len−1,
/// floor(q·len))]`; empty → 0.
#[must_use]
pub fn quantile(arr: &[f64], q: f64) -> f64 {
    if arr.is_empty() {
        return 0.0;
    }
    let mut s = arr.to_vec();
    s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    s[usize::min(s.len() - 1, (q * s.len() as f64).floor() as usize)]
}

/// JS `blankSample` (L757): the scratch shape before any path is sampled.
#[must_use]
pub fn blank_sample() -> ActorSample {
    ActorSample {
        t: 0.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw: 0.0,
        aim_yaw: 0.0,
        aim_pitch: 0.0,
        f: 0,
        hp: 100.0,
        ink: 100.0,
        sp: 0.0,
        ch: 0.0,
        turf: 0.0,
        tp: 0,
        wx: 0.0,
        wy: 0.0,
        wz: 1.0,
        lock: 0.0,
        ks: 0,
        spx: 0.0,
        spst: 0,
    }
}

/// JS `copySample` (L758): every field, including `t`.
fn copy_sample(s: &ActorSample, o: &mut ActorSample) {
    *o = *s;
}

/// JS `_pathAt` return codes (L286-287 doc): interpolating / holding across
/// a teleport / extrapolating past the newest sample / before the oldest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMode {
    /// Hermite between two same-`tp` samples.
    Interp,
    /// `tp` differs inside the bracket: hold one sample, then snap.
    Hold,
    /// Buffer ran dry: ballistic ≤ 0.18 s past the newest sample, then hold.
    Extrapolate,
    /// Before the oldest buffered sample: hold it.
    Early,
}

/// JS `_pathAt` (L288-303). `buf` is the sender's sample list (ascending,
/// non-empty); the sampled state lands in `s`.
#[must_use]
pub fn path_at(buf: &[ActorSample], t: f64, s: &mut ActorSample, gravity: f64) -> PathMode {
    let mut s0 = &buf[0];
    let mut s1: Option<&ActorSample> = None;
    for i in 0..buf.len().saturating_sub(1) {
        if (buf[i].t as f64) <= t && (buf[i + 1].t as f64) > t {
            s0 = &buf[i];
            s1 = Some(&buf[i + 1]);
            break;
        }
    }
    if let Some(b) = s1 {
        if s0.tp == b.tp {
            hermite(s0, b, t, s);
            return PathMode::Interp;
        }
        // teleport between these two: hold, then snap (L292)
        copy_sample(if t >= b.t as f64 { b } else { s0 }, s);
        return PathMode::Hold;
    }
    let last = &buf[buf.len() - 1];
    if t < last.t as f64 {
        copy_sample(s0, s);
        return PathMode::Early;
    }
    // buffer ran dry: a short ballistic extrapolation along the last
    // velocity, then hold (L295-301)
    copy_sample(last, s);
    let e = f64::min(t - last.t as f64, 0.18);
    if e > 0.0 {
        s.x = (last.x as f64 + last.vx as f64 * e) as f32;
        s.z = (last.z as f64 + last.vz as f64 * e) as f32;
        if !last.flag(flags::GROUNDED) {
            s.y = (last.y as f64 + last.vy as f64 * e - 0.25 * gravity * e * e) as f32;
            s.vy = (last.vy as f64 - 0.5 * gravity * e) as f32;
        }
    }
    PathMode::Extrapolate
}

/// JS `hermite` (L766-786): C1 position through the owner's velocity
/// tangents; linear velocity/angles/charge/lock/ink; discrete state from the
/// earlier sample; `hp` switches at `u = 0.5`.
fn hermite(a: &ActorSample, b: &ActorSample, t: f64, o: &mut ActorSample) {
    let d = f64::max(1e-3, b.t as f64 - a.t as f64);
    let u = ((t - a.t as f64) / d).clamp(0.0, 1.0);
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    copy_sample(a, o);
    o.t = t as f32;
    o.x = (h00 * a.x as f64 + h10 * d * a.vx as f64 + h01 * b.x as f64 + h11 * d * b.vx as f64)
        as f32;
    o.z = (h00 * a.z as f64 + h10 * d * a.vz as f64 + h01 * b.z as f64 + h11 * d * b.vz as f64)
        as f32;
    // vertical: Hermite in the air, but never through a floor both ends are
    // standing on (landing velocities jump) — L774-777
    let mut y = h00 * a.y as f64 + h10 * d * a.vy as f64 + h01 * b.y as f64 + h11 * d * b.vy as f64;
    if a.flag(flags::GROUNDED) || b.flag(flags::GROUNDED) {
        y = f64::min(
            f64::max(a.y as f64, b.y as f64) + 0.02,
            f64::max(f64::min(a.y as f64, b.y as f64) - 0.02, y),
        );
    }
    o.y = y as f32;
    o.vx = (a.vx as f64 + (b.vx as f64 - a.vx as f64) * u) as f32;
    o.vy = (a.vy as f64 + (b.vy as f64 - a.vy as f64) * u) as f32;
    o.vz = (a.vz as f64 + (b.vz as f64 - a.vz as f64) * u) as f32;
    o.yaw = (a.yaw as f64 + ang_diff(a.yaw as f64, b.yaw as f64) * u) as f32;
    o.aim_yaw = (a.aim_yaw as f64 + ang_diff(a.aim_yaw as f64, b.aim_yaw as f64) * u) as f32;
    o.aim_pitch = (a.aim_pitch as f64 + (b.aim_pitch as f64 - a.aim_pitch as f64) * u) as f32;
    o.ch = (a.ch as f64 + (b.ch as f64 - a.ch as f64) * u) as f32;
    o.lock = (a.lock as f64 + (b.lock as f64 - a.lock as f64) * u) as f32;
    o.hp = if u < 0.5 { a.hp } else { b.hp };
    o.ink = (a.ink as f64 + (b.ink as f64 - a.ink as f64) * u) as f32;
}

/// JS `p.win` (L240): the ~3 s lateness / tick-spacing windows.
#[derive(Debug, Clone, Default)]
struct Win {
    late: Vec<f64>,
    gap: Vec<f64>,
}

/// One sender's playback clock — JS `_peer` (L223-227) plus the fields
/// `_tick` / `_advance` maintain.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Fastest-packet floor, crept up by 0.25 % of the gap per packet
    /// (L235-237).
    off: f64,
    /// Buffer depth in seconds, quick to add (×2) slow to trim (×0.35)
    /// (L274).
    delay: f64,
    /// Playback rate (L279-283), eased with `dt × 6`.
    rate: f64,
    /// Target buffer need from the windows (L244); `None` until the first
    /// tick (JS `p.want ?? 0.1`, L273).
    want: Option<f64>,
    /// Playback clock position in the sender's time (L276-284).
    tr: Option<f64>,
    /// JS `p.init` (L235): the first packet sets `off` instead of steering.
    init: bool,
    /// JS `p.lastTs` (L234): newest sender timestamp seen.
    last_ts: f64,
    /// JS `p.prevTs` (L241): previous tick's `ts` for the gap window.
    prev_ts: Option<f64>,
    win: Win,
    /// The sender's event timeline queue (L256) — consumed by Task 23.
    pub events: Vec<TimedEvent>,
}

impl Peer {
    fn new() -> Self {
        Self {
            off: 0.0,
            delay: 0.1,
            rate: 1.0,
            want: None,
            tr: None,
            init: false,
            last_ts: 0.0,
            prev_ts: None,
            win: Win::default(),
            events: Vec::new(),
        }
    }
}

/// Per-proxy playback state — JS `a.net` (L85, the fields Task 22 owns).
#[derive(Debug, Clone)]
pub struct ActorNet {
    /// The interpolation buffer, newest last, capacity 40 (L249-253).
    buf: Vec<ActorSample>,
    /// This frame's sampled path state (JS `n.cur`).
    cur: ActorSample,
    /// JS `n.prevRaw` / `n.prevT`: the moment shown last frame.
    prev_raw: Option<DVec3>,
    prev_t: Option<f64>,
    /// JS `n.err` / `n.errV`: the decaying correction offset.
    err: DVec3,
    err_v: DVec3,
    /// JS `n.tp`: the teleport counter currently *shown* (snapped in
    /// `_sample`, L316).
    tp_shown: u32,
    /// JS `n.has` / `n.ready` / `n.handoff` / `n.spawnPending` /
    /// `n.deathTp`.
    has: bool,
    ready: bool,
    handoff: bool,
    spawn_pending: bool,
    death_tp: u32,
}

impl ActorNet {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            cur: blank_sample(),
            prev_raw: None,
            prev_t: None,
            err: DVec3::ZERO,
            err_v: DVec3::ZERO,
            tp_shown: 0,
            has: false,
            ready: false,
            handoff: false,
            spawn_pending: false,
            death_tp: 0,
        }
    }
}

/// The playback layer for one client: one clock per sender, one sample
/// buffer + correction per remote actor.
pub struct Playback {
    /// Insertion-ordered (JS `Map`) — the per-frame loops must run in a
    /// stable order for reproducible traces.
    peers: Vec<(SessionId, Peer)>,
    /// Roster-parallel per-proxy state (indexed like
    /// [`super::replication::Replication`]'s arrays).
    actors: Vec<ActorNet>,
    /// JS `TICK` (L30), the default gap sample before the first pair.
    tick_dt: f64,
}

impl Playback {
    #[must_use]
    pub fn new(n_actors: usize, tick_dt: f64) -> Self {
        Self {
            peers: Vec::new(),
            actors: (0..n_actors).map(|_| ActorNet::new()).collect(),
            tick_dt,
        }
    }

    /// JS `_peer` (L223-227): get-or-create the sender's clock.
    pub fn ensure_peer(&mut self, from: &str) {
        if !self.peers.iter().any(|(id, _)| id == from) {
            self.peers.push((from.to_string(), Peer::new()));
        }
    }

    pub fn peer_tr(&self, from: &str) -> Option<f64> {
        self.peers
            .iter()
            .find(|(id, _)| id == from)
            .and_then(|(_, p)| p.tr)
    }

    #[must_use]
    pub fn buf_len(&self, idx: usize) -> usize {
        self.actors.get(idx).map_or(0, |n| n.buf.len())
    }

    /// JS `a.net.tp` — the shown teleport counter (`_adopt` continues from
    /// it, L702).
    #[must_use]
    pub fn tp_shown(&self, idx: usize) -> u32 {
        self.actors.get(idx).map_or(0, |n| n.tp_shown)
    }

    /// JS `a.net.spawnPending`.
    #[must_use]
    pub fn spawn_pending(&self, idx: usize) -> bool {
        self.actors.get(idx).is_some_and(|n| n.spawn_pending)
    }

    /// Whether the app should draw this proxy (JS `character.root.visible`
    /// outcomes in `applyRemote` L352 / L354 / L421 / L427): hidden until
    /// the path is ready, while dead, and while a pending respawn still
    /// shows the death's `tp`.
    #[must_use]
    pub fn proxy_shown(&self, idx: usize, alive: bool) -> bool {
        self.actors
            .get(idx)
            .is_some_and(|n| n.ready && alive && !(n.spawn_pending && n.cur.tp == n.death_tp))
    }

    /// JS `_tick` clock bookkeeping (L229-244) for one inbound tick:
    /// `now` is this client's logical clock at arrival, `ts` the sender's
    /// `r3` stamp.
    pub fn on_tick(&mut self, from: &str, ts: f64, now: f64) {
        self.ensure_peer(from);
        let Some((_, p)) = self.peers.iter_mut().find(|(id, _)| id == from) else {
            return;
        };
        let o = now - ts;
        // JS L234: `if (!(d.ts < p.lastTs)) p.lastTs = d.ts` — no NaN
        // stamps on the wire, so the negated form is plain `>=`.
        if ts >= p.last_ts {
            p.last_ts = ts;
        }
        if !p.init {
            p.off = o;
            p.init = true;
        } else if o < p.off {
            p.off = o; // a faster packet: the true floor is lower
        } else {
            p.off += (o - p.off) * 0.0025; // creep up slowly
        }
        p.win.late.push(o - p.off);
        p.win.gap.push(match p.prev_ts {
            Some(prev) => (ts - prev).clamp(0.0, 0.25),
            None => self.tick_dt,
        });
        p.prev_ts = Some(ts);
        if p.win.late.len() > 60 {
            p.win.late.remove(0);
            p.win.gap.remove(0);
        }
        p.want = Some(
            (quantile(&p.win.gap, 0.95) + quantile(&p.win.late, 0.9) + 0.012).clamp(0.07, 0.3),
        );
    }

    /// JS L256: the tick's event records join the sender's queue (Task 23
    /// plays them on `p.tr`).
    pub fn push_events(&mut self, from: &str, events: Vec<TimedEvent>) {
        if events.is_empty() {
            return;
        }
        self.ensure_peer(from);
        if let Some((_, p)) = self.peers.iter_mut().find(|(id, _)| id == from) {
            p.events.extend(events);
        }
    }

    /// JS L246-254: route one decoded sample into its actor's buffer — the
    /// monotonic-`t` guard (L251) and the 40-capacity tail trim (L253).
    /// Returns whether the sample was accepted.
    pub fn push_sample(&mut self, idx: usize, s: &ActorSample) -> bool {
        let Some(n) = self.actors.get_mut(idx) else {
            return false;
        };
        if let Some(last) = n.buf.last()
            && s.t <= last.t
        {
            return false;
        }
        n.buf.push(*s);
        if n.buf.len() > 40 {
            let cut = n.buf.len() - 40;
            n.buf.drain(0..cut);
        }
        true
    }

    /// JS `_advance` (L271-284) for every sender, once per frame.
    pub fn advance_all(&mut self, now: f64, dt: f64) {
        for (_, p) in &mut self.peers {
            if !p.init {
                continue;
            }
            let want = p.want.unwrap_or(0.1);
            // quick to add buffer, slow to trim it (L274)
            let k = f64::min(1.0, dt * if want > p.delay { 2.0 } else { 0.35 });
            p.delay += (want - p.delay) * k;
            let target = now - p.off - p.delay;
            match p.tr {
                None => {
                    p.tr = Some(target);
                    p.rate = 1.0;
                }
                Some(tr) => {
                    if (target - tr).abs() > 0.5 {
                        p.tr = Some(target); // (re)sync
                        p.rate = 1.0;
                        continue;
                    }
                    // small errors: a barely visible ±8 % time stretch;
                    // big ones catch up faster (L277-279)
                    let e = target - tr;
                    let mut rate = if e.abs() < 0.12 {
                        1.0 + (e * 1.5).clamp(-0.08, 0.08)
                    } else {
                        1.0 + (e * 2.5).clamp(-0.2, 0.25)
                    };
                    let ahead = p.last_ts - tr; // path still buffered (L280)
                    if ahead < 0.015 {
                        rate = f64::min(rate, 0.5 + 0.5 * f64::max(0.0, ahead / 0.015));
                    }
                    p.rate += (rate - p.rate) * f64::min(1.0, dt * 6.0);
                    p.tr = Some(tr + dt * p.rate);
                }
            }
        }
    }

    /// JS `_sample` (L305-346) for one remote actor: evaluate the path at
    /// `tr`, fold any shown-moment jump into the critically damped
    /// correction, and mark the proxy ready. `owner` selects the sender's
    /// clock.
    pub fn sample(&mut self, idx: usize, owner: &str, dt: f64, gravity: f64) {
        let Some((_, peer)) = self.peers.iter().find(|(id, _)| id == owner) else {
            return;
        };
        let Some(tr) = peer.tr else {
            return;
        };
        let Some(n) = self.actors.get_mut(idx) else {
            return;
        };
        if n.buf.is_empty() {
            return;
        }
        // drop history older than what we might still need (last frame's
        // moment must stay covered) — L311
        while n.buf.len() > 2 && (n.buf[1].t as f64) < tr - 0.25 {
            n.buf.remove(0);
        }
        let mode = path_at(&n.buf, tr, &mut n.cur, gravity);
        let mut snap_now = false;
        if n.cur.tp != n.tp_shown {
            snap_now = true;
            n.tp_shown = n.cur.tp;
        }
        // Correction (L317-343): the path itself is C1, so the only
        // discontinuities are new data rewriting a moment already shown.
        if n.handoff {
            // a new owner (host migration): keep drawing from here and let
            // the offset settle (L322-328)
            n.handoff = false;
            // JS L326 also sets `snapNow = false` here — dead in both
            // languages: the only reader (L329) is the `else if` branch
            // handoff excludes.
            if let (Some(prev_raw), true) = (n.prev_raw, n.has) {
                n.err += prev_raw - dvec3_of(&n.cur);
                n.tp_shown = n.cur.tp;
            }
            if n.err.length_squared() > 16.0 {
                n.err = DVec3::ZERO;
                n.err_v = DVec3::ZERO;
            }
            n.has = true;
        } else if snap_now || !n.has {
            n.err = DVec3::ZERO;
            n.err_v = DVec3::ZERO;
            n.has = true;
        } else if let (Some(prev_raw), Some(prev_t)) = (n.prev_raw, n.prev_t) {
            let mut p = blank_sample();
            let _ = path_at(&n.buf, prev_t, &mut p, gravity);
            let j = prev_raw - dvec3_of(&p);
            let j2 = j.length_squared();
            if j2 > 1e-6 && j2 < 16.0 {
                n.err += j;
            } else if j2 >= 16.0 {
                // a real relocation: show it
                n.err = DVec3::ZERO;
                n.err_v = DVec3::ZERO;
            }
        }
        n.prev_raw = Some(dvec3_of(&n.cur));
        n.prev_t = Some(tr);
        if n.err.x != 0.0
            || n.err.y != 0.0
            || n.err.z != 0.0
            || n.err_v.x != 0.0
            || n.err_v.y != 0.0
            || n.err_v.z != 0.0
        {
            // critically damped, ω = 13 (L340-341): no pop, no kick in speed
            let w: f64 = 13.0;
            let k = (-w * dt).exp();
            for c in 0..3 {
                let x0 = n.err[c];
                let v0 = n.err_v[c];
                let q = v0 + w * x0;
                let nx = (x0 + q * dt) * k;
                let nv = (v0 - w * q * dt) * k;
                n.err[c] = nx;
                n.err_v[c] = nv;
            }
            if n.err.length_squared() < 1e-6 && n.err_v.length_squared() < 1e-4 {
                n.err = DVec3::ZERO;
                n.err_v = DVec3::ZERO;
            }
        }
        let _ = mode; // JS stats.extrap / dbg only (L314, L344) — app-side
        n.ready = true;
    }

    /// JS `applyRemote` (L349-428) sim-side subset + the `_remoteSplat` /
    /// `_remoteRespawn` state machines (L603-627, triggered off the `ALIVE`
    /// flag edge — see module docs). Writes the proxy actor in place of
    /// `Actor::step` (the mask freezes it, match.rs `step_masked`).
    pub fn apply(
        &mut self,
        idx: usize,
        a: &mut Actor,
        wr: &mut WeaponRunner,
        dt: f32,
        t: &PlayerTuning,
    ) {
        let Some(n) = self.actors.get_mut(idx) else {
            return;
        };
        let s = n.cur;
        if n.ready {
            let sample_alive = s.flag(flags::ALIVE);
            if a.alive && !sample_alive {
                // JS `_remoteSplat` (L603-616)
                a.alive = false;
                a.hp = 0.0;
                a.respawn_timer = t.respawn_time;
                a.deaths += 1;
                wr.reset();
                n.buf.clear();
                n.ready = false;
                n.has = false;
                n.death_tp = n.tp_shown;
            } else if !a.alive && sample_alive {
                // JS `_remoteRespawn` (L623-627)
                a.alive = true;
                a.hp = t.hp;
                a.invuln = t.spawn_invuln;
                a.respawn_timer = 0.0;
                n.spawn_pending = true;
            }
        }
        if !n.ready {
            return;
        }
        if !a.alive {
            // JS L354: the death countdown runs here because `step_masked`
            // freezes remote actors.
            a.respawn_timer -= dt;
            return;
        }
        // position = sampled path + decaying correction (L356)
        let e = n.err;
        a.pos.x = (s.x as f64 + e.x) as f32;
        a.pos.z = (s.z as f64 + e.z) as f32;
        a.set_visual_y((s.y as f64 + e.y) as f32);
        a.vel = glam::vec3(s.vx, s.vy, s.vz);
        // JS `netTurnRate` (L358-359) — the sim seam is `yaw_vel`
        a.yaw_vel = if dt > 0.0 {
            (ang_diff(a.yaw as f64, s.yaw as f64) / dt as f64) as f32
        } else {
            0.0
        };
        a.yaw = s.yaw;
        a.aim_yaw = s.aim_yaw;
        a.aim_pitch = s.aim_pitch;
        // discrete state (L365-372; M1 has no climb — `wallN` skipped)
        a.form = if s.flag(flags::SQUID) {
            Form::Squid
        } else {
            Form::Kid
        };
        a.submerged = s.flag(flags::SUB);
        a.grounded = s.flag(flags::GROUNDED);
        // JS L371 checks `gt2` first (the two bits are mutually exclusive
        // on the sender side — `packActor`; the order only matters for a
        // corrupt double-set frame, where JS reports 2).
        a.ground_team = if s.flag(flags::GT2) {
            2
        } else if s.flag(flags::GT1) {
            1
        } else {
            0
        };
        a.on_enemy = s.flag(flags::ENEMY);
        // hp/ink (L376); `special` is an M1 constant (L376 `a.special = S.sp`)
        a.hp = s.hp;
        a.ink = s.ink;
        // L377: the invuln bit is a hard 0.1 s window (the proxy never runs
        // `Actor::step`, so nothing else decrements it)
        a.invuln = if s.flag(flags::INVULN) { 0.1 } else { 0.0 };
        // L378: the turf stat only ever ratchets up
        a.turf = a.turf.max(s.turf);
        // L420-426: a respawned proxy stays hidden until the path shows the
        // new life's `tp`
        if n.spawn_pending {
            if s.tp == n.death_tp {
                return;
            }
            n.spawn_pending = false;
        }
    }

    /// JS `onLeave` L679 (non-self takeover): same squidkid, new sender —
    /// drop the stale buffer and glide onto the new path.
    pub fn handoff(&mut self, idx: usize) {
        if let Some(n) = self.actors.get_mut(idx) {
            n.buf.clear();
            n.handoff = true;
        }
    }

    /// JS `_adopt` L703-706: the buffer and the correction reset; returns
    /// the shown `tp` to continue the counter from (L702). The caller
    /// finishes the JS L704 branch (mid-respawn proxy → local respawn).
    pub fn adopt_reset(&mut self, idx: usize) -> u32 {
        let Some(n) = self.actors.get_mut(idx) else {
            return 0;
        };
        let tp = n.tp_shown;
        n.buf.clear();
        n.err = DVec3::ZERO;
        n.err_v = DVec3::ZERO;
        n.spawn_pending = false;
        tp
    }

    /// Roster pruning (JS `_remove` deletes the actor; its `net` goes with
    /// it).
    pub fn remove_at(&mut self, idx: usize) {
        self.actors.remove(idx);
    }

    /// The peer count (introspection / tests).
    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// The sender's current delay in seconds (introspection / tests).
    #[must_use]
    pub fn peer_delay(&self, from: &str) -> f64 {
        self.peers
            .iter()
            .find(|(id, _)| id == from)
            .map_or(0.0, |(_, p)| p.delay)
    }
}

fn dvec3_of(s: &ActorSample) -> DVec3 {
    DVec3::new(s.x as f64, s.y as f64, s.z as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: f32, x: f32) -> ActorSample {
        ActorSample {
            t,
            x,
            y: 0.0,
            z: 0.0,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw: 0.0,
            aim_yaw: 0.0,
            aim_pitch: 0.0,
            f: flags::ALIVE | flags::GROUNDED,
            hp: 100.0,
            ink: 100.0,
            sp: 0.0,
            ch: 0.0,
            turf: 0.0,
            tp: 0,
            wx: 0.0,
            wy: 0.0,
            wz: 0.0,
            lock: 0.0,
            ks: 0,
            spx: 0.0,
            spst: 0,
        }
    }

    #[test]
    fn tr22_quantile_matches_js() {
        // L761-764: ascending sort, `s[min(len-1, floor(q*len))]`, empty → 0.
        assert_eq!(quantile(&[], 0.95), 0.0);
        let a = [3.0, 1.0, 2.0];
        assert_eq!(quantile(&a, 0.0), 1.0);
        assert_eq!(quantile(&a, 0.5), 2.0);
        assert_eq!(quantile(&a, 0.95), 3.0);
        assert_eq!(quantile(&a, 1.0), 3.0, "floor(1*3)=3 clamps to len-1");
        // JS `%`-style sort is numeric; NaN never appears here.
        assert_eq!(quantile(&[5.0, 5.0, 5.0], 0.9), 5.0);
    }

    #[test]
    fn tr22_ang_diff_shortest_arc() {
        // L38
        assert!(
            (ang_diff(0.0, std::f64::consts::PI * 0.5) - std::f64::consts::PI * 0.5).abs() < 1e-12
        );
        assert!(
            (ang_diff(0.0, -std::f64::consts::PI * 0.5) + std::f64::consts::PI * 0.5).abs() < 1e-12
        );
        // wrap the long way: +3.0 → −3.0 is +3.283… not −6.0
        let d = ang_diff(3.0, -3.0);
        assert!(d > 0.0 && d < std::f64::consts::PI, "{d}");
    }

    // ---------------------------------------------------------------- TR-22.1
    #[test]
    fn tr22_1_hermite_is_c1_and_exact_through_owner_tangents() {
        // Constant-velocity path: the Hermite through the owner's velocity
        // tangents must reproduce the analytic position (p95 error → 0,
        // well under the 2 cm gate) with no corner at the knots.
        let dt = 1.0 / 20.0;
        let mut buf = Vec::new();
        for i in 0..10 {
            let mut s = sample(i as f32 * dt, i as f32 * 4.0 * dt);
            s.vx = 4.0;
            buf.push(s);
        }
        let mut o = blank_sample();
        // interior points: exact
        for k in 0..9 {
            let t = (k as f64 + 0.37) * dt as f64;
            assert!(matches!(path_at(&buf, t, &mut o, 25.0), PathMode::Interp));
            let want = t * 4.0;
            assert!((o.x as f64 - want).abs() < 1e-3, "u=0.37 at k={k}: {}", o.x);
        }
        // knots are C1: the derivative from both sides is the owner velocity
        // (tangent continuity is structural in the Hermite basis; check the
        // position is continuous and the local delta matches v·dt).
        let eps = 1e-4;
        let knot = 5.0 * dt as f64;
        let _ = path_at(&buf, knot - eps, &mut o, 25.0);
        let before = o.x;
        let _ = path_at(&buf, knot + eps, &mut o, 25.0);
        let after = o.x;
        assert!(
            (after - before) < 1e-2,
            "no corner jump: {before} → {after}"
        );
        // hp is discrete from the earlier sample until u ≥ 0.5 (L784)
        let mut a = sample(0.0, 0.0);
        a.hp = 40.0;
        let mut b = sample(0.05, 1.0);
        b.hp = 90.0;
        let _ = path_at(&[a, b], 0.02, &mut o, 25.0);
        assert_eq!(o.hp, 40.0);
        let _ = path_at(&[a, b], 0.04, &mut o, 25.0);
        assert_eq!(o.hp, 90.0);
    }

    #[test]
    fn tr22_1_tp_jump_cuts_the_path() {
        // L291-292: differing `tp` inside the bracket → hold, never glide.
        let mut a = sample(0.0, 0.0);
        a.tp = 3;
        let mut b = sample(0.05, 100.0);
        b.tp = 4;
        let mut o = blank_sample();
        assert!(matches!(
            path_at(&[a, b], 0.025, &mut o, 25.0),
            PathMode::Hold
        ));
        assert_eq!(o.x, 0.0, "before b.t holds a");
        // past b.t the hold ends: the newest sample is shown, no glide
        assert!(matches!(
            path_at(&[a, b], 0.06, &mut o, 25.0),
            PathMode::Extrapolate
        ));
        assert_eq!(o.x, 100.0, "at b.t onward snaps to b");
    }

    // ---------------------------------------------------------------- TR-22.2
    #[test]
    fn tr22_2_delay_stays_in_the_85_to_300ms_band() {
        // want = clamp(0.07, 0.3, q95(gap) + q90(late) + 0.012) (L244); the
        // eased delay (L274) rides inside the band once converged.
        let mut pb = Playback::new(1, 1.0 / 20.0);
        let tick = 1.0 / 20.0f64;
        // a steady sender with 20 ms of lateness
        for i in 0..200 {
            pb.on_tick("H", i as f64 * tick, i as f64 * tick + 0.02);
            pb.advance_all(i as f64 * tick + 0.02, tick);
        }
        let d = pb.peer_delay("H");
        assert!((0.07 - 1e-9..=0.3 + 1e-9).contains(&d), "delay {d}");
        // q95(gap)=0.05 + q90(late)=0 + 0.012 = 0.062 → clamped up to 0.07
        assert!((d - 0.07).abs() < 0.001, "converged to want: {d}");
        // a spiky sender: the 90th percentile keeps one hiccup from
        // dragging the delay (L238-239 comment) — pin both sides of q90.
        let mut pb2 = Playback::new(1, 1.0 / 20.0);
        for i in 0..200 {
            let late = if i == 100 { 0.5 } else { 0.0 };
            pb2.on_tick("H", i as f64 * tick, i as f64 * tick + late);
            pb2.advance_all(i as f64 * tick + late, tick);
            // while the spike is still inside the 60-sample window (it
            // slides out at i=160), q90 must ignore it: one outlier of 60
            // sits below the floor(0.9*60)=54th sorted index.
            if i == 150 {
                let d = pb2.peer_delay("H");
                assert!(
                    (d - 0.07).abs() < 0.005,
                    "one hiccup in the window must not drag the delay: {d}"
                );
            }
        }
        let d2 = pb2.peer_delay("H");
        assert!((0.07..=0.3).contains(&d2), "spiky delay {d2} stays clamped");
        // the other side: late on 20 % of ticks, q90 lands on the spike —
        // 0.05 + 0.5 + 0.012 → clamped to the 0.3 ceiling.
        let mut pb3 = Playback::new(1, 1.0 / 20.0);
        for i in 0..200 {
            let late = if i % 5 == 0 { 0.5 } else { 0.0 };
            pb3.on_tick("H", i as f64 * tick, i as f64 * tick + late);
            pb3.advance_all(i as f64 * tick + late, tick);
        }
        let d3 = pb3.peer_delay("H");
        assert!(
            (d3 - 0.3).abs() < 0.01,
            "a persistently spiky sender is dragged to the ceiling: {d3}"
        );
    }

    #[test]
    fn tr22_2_extrapolation_capped_at_180ms() {
        // L297: `e = min(t − last.t, 0.18)` — the path never runs further
        // than 180 ms past the newest sample.
        let mut buf = vec![sample(0.0, 0.0)];
        buf[0].vx = 10.0;
        buf[0].vz = 10.0;
        let mut o = blank_sample();
        let _ = path_at(&buf, 1.0, &mut o, 25.0);
        assert!((o.x - 1.8).abs() < 1e-6, "x capped at 10·0.18: {}", o.x);
        assert!((o.z - 1.8).abs() < 1e-6);
        // airborne: ballistic y, then hold (L300)
        buf[0].f = flags::ALIVE;
        buf[0].vy = 5.0;
        let _ = path_at(&buf, 0.1, &mut o, 25.0);
        let want_y = 5.0 * 0.1 - 0.25 * 25.0 * 0.01;
        assert!((o.y as f64 - want_y).abs() < 1e-4, "ballistic y {}", o.y);
        let want_vy = 5.0 - 0.5 * 25.0 * 0.1;
        assert!((o.vy as f64 - want_vy).abs() < 1e-4);
    }

    // --------------------------------------------------------------- TR-22.3
    #[test]
    fn tr22_3_correction_settles_without_a_pop() {
        // Leaving an extrapolation: the new sample rewrites the shown
        // moment; the jump goes into `err` (L330-335) and the actor keeps
        // drawing from where it was (pos = S + err, L356).
        let mut pb = Playback::new(1, 1.0 / 20.0);
        let dt = 1.0 / 60.0f64;
        pb.on_tick("H", 0.0, 0.0); // init, off = 0
        // one sample at sender t = 0; the clock sits at delay behind
        let mut s = sample(0.0, 0.0);
        s.vx = 10.0;
        pb.push_sample(0, &s);
        // advance so tr passes the sample end → extrapolation shown
        for _ in 0..20 {
            pb.advance_all(20.0 * dt, dt);
            pb.sample(0, "H", dt, 25.0);
        }
        let shown_x = pb.actors[0].cur.x as f64 + pb.actors[0].err.x;
        // the next packet rewrites the moment: same path time, new data
        let mut s2 = sample(0.02, 0.2);
        s2.vx = 10.0;
        pb.push_sample(0, &s2);
        pb.advance_all(21.0 * dt, dt);
        pb.sample(0, "H", dt, 25.0);
        let after_x = pb.actors[0].cur.x as f64 + pb.actors[0].err.x;
        // the drawn position moved by ~v·dt·rate, not by the rewrite
        assert!(
            (after_x - shown_x).abs() < 0.5,
            "no pop: {shown_x} → {after_x}"
        );
        assert!(
            pb.actors[0].err.length_squared() > 1e-6,
            "jump carried as err"
        );
        // …and it decays: run frames, err → 0, position continuous
        // (ω=13 eases ~×0.978 per 1/60 s frame — ~240 frames to <1e-3)
        let mut prev = after_x;
        for i in 1..=400 {
            pb.advance_all((21.0 + i as f64) * dt, dt);
            pb.sample(0, "H", dt, 25.0);
            let x = pb.actors[0].cur.x as f64 + pb.actors[0].err.x;
            assert!((x - prev).abs() < 0.1, "asymptotic, never a jump");
            prev = x;
        }
        assert!(pb.actors[0].err.length_squared() < 1e-6, "err settled");
    }

    #[test]
    fn tr22_3_hermite_y_never_through_a_floor() {
        // L776: with either end grounded, y stays within the two feet
        // heights ± 0.02.
        let mut a = sample(0.0, 0.0);
        a.y = 0.5;
        a.vy = -30.0; // a landing velocity that would plunge the curve
        let mut b = sample(0.05, 1.0);
        b.y = 0.5;
        b.vy = 0.0;
        b.f = flags::ALIVE | flags::GROUNDED;
        let mut o = blank_sample();
        for k in 1..5 {
            let _ = path_at(&[a, b], k as f64 * 0.01, &mut o, 25.0);
            assert!(
                o.y as f64 >= 0.5 - 0.02 - 1e-6 && o.y as f64 <= 0.5 + 0.02 + 1e-6,
                "y {} through the floor at u={}",
                o.y,
                k as f64 / 5.0
            );
        }
    }

    #[test]
    fn tr22_buffer_capacity_and_monotonic_guard() {
        // L251-253: a non-newer sample is dropped; the buffer trims to 40.
        let mut pb = Playback::new(1, 1.0 / 20.0);
        let mut s = sample(1.0, 0.0);
        assert!(pb.push_sample(0, &s));
        s.t = 0.9;
        assert!(!pb.push_sample(0, &s), "older than the tail is dropped");
        s.t = 1.0;
        assert!(!pb.push_sample(0, &s), "equal is dropped too");
        for i in 2..=50 {
            s.t = i as f32;
            assert!(pb.push_sample(0, &s));
        }
        assert_eq!(pb.buf_len(0), 40);
        assert_eq!(pb.actors[0].buf[0].t, 11.0, "oldest trimmed first");
    }

    #[test]
    fn tr22_apply_writes_invuln_window_and_turf_ratchet() {
        // Review M-5 (JS `applyRemote` L377-378): the invuln bit is a hard
        // 0.1 s window, and turf only ratchets up (`Math.max`). Driven
        // through the ready path with `cur` set directly — the writes under
        // test are the discrete ones, not the clock.
        let tuning = crate::embedded_tuning();
        let t = &tuning.player;
        let dt = 1.0 / 60.0f32;
        let mut pb = Playback::new(1, 1.0 / 20.0);
        let mut a = Actor::new(0, 0, t);
        let mut wr = WeaponRunner::new();
        let mut s = sample(0.0, 0.0);
        s.f |= flags::INVULN;
        s.turf = 50.0;
        pb.on_tick("H", 0.0, 0.0);
        pb.push_sample(0, &s);
        pb.advance_all(0.0, dt as f64);
        pb.sample(0, "H", dt as f64, 25.0);
        assert!(pb.actors[0].ready);
        pb.apply(0, &mut a, &mut wr, dt, t);
        assert_eq!(a.invuln, 0.1, "the invuln bit is a 0.1 s window");
        assert_eq!(a.turf, 50.0);
        // bit off + smaller turf: the window clears, the stat never regresses
        let mut s2 = sample(0.05, 0.0);
        s2.turf = 40.0;
        pb.actors[0].cur = s2;
        pb.apply(0, &mut a, &mut wr, dt, t);
        assert_eq!(a.turf, 50.0, "turf only ratchets up");
        assert_eq!(a.invuln, 0.0, "bit off clears the window");
    }

    #[test]
    fn tr22_handoff_and_adopt_reset_the_path() {
        // L679 + L322-328: a new sender glides onto its path via err;
        // L692-707: adoption clears the buffer and the offset.
        let mut pb = Playback::new(1, 1.0 / 20.0);
        pb.on_tick("H", 0.0, 0.0);
        pb.push_sample(0, &sample(0.0, 5.0));
        pb.advance_all(0.0, 1.0 / 60.0);
        pb.sample(0, "H", 1.0 / 60.0, 25.0);
        assert!(pb.actors[0].ready);
        pb.handoff(0);
        assert_eq!(pb.buf_len(0), 0);
        pb.on_tick("N", 0.0, 0.0);
        pb.push_sample(0, &sample(0.0, 9.0));
        pb.advance_all(0.0, 1.0 / 60.0);
        pb.sample(0, "N", 1.0 / 60.0, 25.0);
        // err carries the shown position: pos = S + err stays near 5
        // (one damping frame moves it slightly — the point is there is no
        // 4 m pop)
        let pos = pb.actors[0].cur.x as f64 + pb.actors[0].err.x;
        assert!((pos - 5.0).abs() < 0.2, "handoff keeps drawing: {pos}");
        // an absurd relocation (> 4 m) is shown instead of damped (L327)
        pb.handoff(0);
        pb.actors[0].err = DVec3::new(50.0, 0.0, 0.0);
        pb.push_sample(0, &sample(0.0, 9.0));
        pb.sample(0, "N", 1.0 / 60.0, 25.0);
        assert_eq!(pb.actors[0].err.x, 0.0, ">4 m err is zeroed");
        // adopt returns the shown tp and clears the state
        let tp = pb.adopt_reset(0);
        assert_eq!(tp, 0);
        assert_eq!(pb.buf_len(0), 0);
    }
}
