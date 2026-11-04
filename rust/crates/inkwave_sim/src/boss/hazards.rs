//! HULLBREAKER's attacks as geometry — a line-for-line port of the sim half
//! of `src/boss/bossHazards.js` (L1-352). The `HazardFX` render section
//! (L357-600) is Task 46 and deliberately absent here.
//!
//! Every hazard is a pure function of its move record `{ id, t0, s,
//! d: [tele, act, rec], p }` and the boss clock, so every client draws the
//! same rings, circles, lanes and beams at the same moment (JS L1-8):
//! - damage to a squidkid is applied victim-side, from what that screen
//!   shows ([`Boss::hz_hurt_local`], JS L119-187);
//! - ink is painted by the host only ([`Boss::hz_paint`], JS L233-289) — the
//!   sim records [`BossEvent::Splat`](super::BossEvent::Splat) instead of
//!   touching the paint grid;
//! - [`Boss::threat`] (JS L294-349) is what bots read to step out of circles,
//!   hop rings and take cover (consumed by Task 47).
//!
//! The `hz` bookkeeping (`hits` / `cache` / `prevBt` / `dead`, JS L70-78)
//! lives on the [`Boss`] entity (Task 42 hoisting precedent, D-T43 design
//! §2); the logic is `impl Boss` below. Records stay [`JVal`] end to end —
//! same shape as the `['bm']` wire payload, no parallel typed struct.
//!
//! Determinism: JS numbers are f64, so every hazard expression is evaluated
//! in f64 in the JS operation order (`// JS bossHazards.js L<line>` anchors
//! below); `Math.round` is the shared [`js_round`]; the reach array is f32
//! (JS `Float32Array`, L205). All constants ride `tuning.boss` — the
//! `0.35` / `1.2` reach probe steps (L209) and the paint-beat literals
//! (`2.2`, `0.06`, `0.09`, `0.07`, …) are JS literals, not extracted.

use std::f64::consts::{PI, TAU};

use glam::{Vec3, vec3};

use crate::actor::Actor;
use crate::net::wire::{self, JVal};
use crate::tuning::{BossHazardTuning, PlayerTuning};

use super::{Boss, BossEvent, BossSquad, HzCache, clamp, js_round};

// ---------------------------------------------------------------------------
// Move identity + record accessors (design §2: the record is a JVal)
// ---------------------------------------------------------------------------

/// The six moves (`MOVES` keys, bossHazards.js L17-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoveId {
    Slam,
    Barrage,
    Sweep,
    Charge,
    Crablets,
    Frenzy,
}

impl MoveId {
    /// The record's `id` string → a [`MoveId`]; unknown ids → `None` (JS's
    /// `switch` simply matches no case).
    #[must_use]
    pub fn parse_id(s: &str) -> Option<Self> {
        match s {
            "slam" => Some(Self::Slam),
            "barrage" => Some(Self::Barrage),
            "sweep" => Some(Self::Sweep),
            "charge" => Some(Self::Charge),
            "crablets" => Some(Self::Crablets),
            "frenzy" => Some(Self::Frenzy),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slam => "slam",
            Self::Barrage => "barrage",
            Self::Sweep => "sweep",
            Self::Charge => "charge",
            Self::Crablets => "crablets",
            Self::Frenzy => "frenzy",
        }
    }
}

/// `m.t0` (boss.js L436 `typeof m.t0 === 'number'`); malformed → `0`.
#[must_use]
pub fn rec_t0(m: &JVal) -> f64 {
    m.field("t0").and_then(JVal::as_f64).unwrap_or(0.0)
}

/// `m.id` → [`MoveId`].
#[must_use]
pub fn rec_id(m: &JVal) -> Option<MoveId> {
    MoveId::parse_id(m.field("id").and_then(JVal::as_str)?)
}

/// `m.d[i]` — the `[tele, act, rec]` phase durations (bossHazards.js L38).
#[must_use]
pub fn rec_d(m: &JVal, i: usize) -> f64 {
    m.field("d")
        .and_then(JVal::as_array)
        .and_then(|a| a.get(i))
        .and_then(JVal::as_f64)
        .unwrap_or(0.0)
}

/// `m.p[key]` as a number.
#[must_use]
pub fn rec_p_f64(m: &JVal, key: &str) -> f64 {
    m.field("p")
        .and_then(|p| p.field(key))
        .and_then(JVal::as_f64)
        .unwrap_or(0.0)
}

/// `m.p.rings` — the slam ring delays (bossBrain.js L239, bossHazards.js
/// L103/L133/L249).
#[must_use]
pub fn rec_rings(m: &JVal) -> Vec<f64> {
    m.field("p")
        .and_then(|p| p.field("rings"))
        .and_then(JVal::as_array)
        .map(|a| a.iter().filter_map(JVal::as_f64).collect())
        .unwrap_or_default()
}

/// `m.p.b` — the barrage barrels `[x, y, z, throwTime]` (bossBrain.js L256,
/// bossHazards.js L93/L145/L261/L318).
#[must_use]
pub fn rec_pts(m: &JVal) -> Vec<[f64; 4]> {
    m.field("p")
        .and_then(|p| p.field("b"))
        .and_then(JVal::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let a = r.as_array()?;
                    Some([
                        a.first()?.as_f64()?,
                        a.get(1)?.as_f64()?,
                        a.get(2)?.as_f64()?,
                        a.get(3)?.as_f64()?,
                    ])
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `m.p.n` — the crablets count (bossBrain.js L279-280).
#[must_use]
pub fn rec_n(m: &JVal) -> f64 {
    rec_p_f64(m, "n")
}

/// `m.p.spin` — the frenzy spin flag (bossBrain.js L286).
#[must_use]
pub fn rec_spin(m: &JVal) -> f64 {
    rec_p_f64(m, "spin")
}

// ---------------------------------------------------------------------------
// Hazard geometry — pure functions of the record + the boss clock
// ---------------------------------------------------------------------------

/// JS `moveTimes(m)` (bossHazards.js L38).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveTimes {
    pub tele: f64,
    pub act: f64,
    pub rec: f64,
    pub end: f64,
}

#[must_use]
pub fn move_times(m: &JVal) -> MoveTimes {
    let (tele, act, rec) = (rec_d(m, 0), rec_d(m, 1), rec_d(m, 2));
    MoveTimes {
        tele,
        act,
        rec,
        end: tele + act + rec,
    }
}

/// JS `movePhaseAt(m, u)` (bossHazards.js L39-46): the phase at move-local
/// time `u`, `None` before the move / after the record.
#[must_use]
pub fn move_phase_at(m: &JVal, u: f64) -> Option<&'static str> {
    let t = move_times(m);
    if u < 0.0 {
        return None; // JS L41
    }
    if u < t.tele {
        return Some("tele"); // JS L42
    }
    if u < t.tele + t.act {
        return Some("act"); // JS L43
    }
    if u < t.end {
        return Some("rec"); // JS L44
    }
    None // JS L45
}

/// JS `chargeDist(p, u, tele)` (bossHazards.js L48-54): distance along the
/// lane at time `u` (accelerates over `chargeRamp`, then full speed), capped
/// at the lane length `l`.
#[must_use]
pub fn charge_dist(hz: &BossHazardTuning, v: f64, l: f64, u: f64, tele: f64) -> f64 {
    let t = u - tele; // JS L49
    if t <= 0.0 {
        return 0.0; // JS L50
    }
    let a = v / f64::from(hz.charge_ramp); // JS L51
    let r = f64::from(hz.charge_ramp);
    let s = if t < r {
        0.5 * a * t * t // JS L52
    } else {
        0.5 * a * r * r + v * (t - r)
    };
    l.min(s) // JS L53 `Math.min(p.L, s)`
}

/// JS `chargeTime(p)` (bossHazards.js L55): when the charge arrives.
#[must_use]
pub fn charge_time(hz: &BossHazardTuning, v: f64, l: f64) -> f64 {
    let r = f64::from(hz.charge_ramp);
    let s0 = 0.5 * v * r; // JS L55
    if l <= s0 {
        ((2.0 * l * r) / v).sqrt()
    } else {
        r + (l - s0) / v
    }
}

/// JS `barrelPos(p, b, k, out)` (bossHazards.js L57-60): a lobbed barrel in
/// flight (`k` 0..1 of its flight) — a high arc from the shell `s` to its
/// circle `b`. Returns the point instead of writing into `out`.
#[must_use]
pub fn barrel_pos(s: [f64; 3], b: &[f64; 4], k: f64) -> [f64; 3] {
    // JS L58: `peak = 5 + 0.35 * Math.hypot(b[0] - p.sx, b[2] - p.sz)`.
    let peak = 5.0 + 0.35 * (b[0] - s[0]).hypot(b[2] - s[2]);
    // JS L59.
    [
        s[0] + (b[0] - s[0]) * k,
        s[1] + (b[1] - s[1]) * k + peak * 4.0 * k * (1.0 - k),
        s[2] + (b[2] - s[2]) * k,
    ]
}

/// JS `beamAngle(m, u)` (bossHazards.js L61): the sweep beam heading —
/// `smoothstep`-eased from `a0` to `a1` across the active phase.
#[must_use]
pub fn beam_angle(m: &JVal, u: f64) -> f64 {
    let a0 = rec_p_f64(m, "a0");
    let a1 = rec_p_f64(m, "a1");
    // JS L61: `k = clamp((u - m.d[0]) / m.d[1], 0, 1)`.
    let k = clamp((u - rec_d(m, 0)) / rec_d(m, 1), 0.0, 1.0);
    a0 + (a1 - a0) * smoothstep(0.0, 1.0, k)
}

/// JS `ringStart(m, k)` (bossHazards.js L62): when slam ring `k` launches.
#[must_use]
pub fn ring_start(m: &JVal, k: usize) -> f64 {
    rec_d(m, 0) + rec_rings(m).get(k).copied().unwrap_or(0.0)
}

/// JS `ringRadius(m, k, u)` (bossHazards.js L63): slam ring `k`'s radius at
/// move-local time `u`.
#[must_use]
pub fn ring_radius(hz: &BossHazardTuning, m: &JVal, k: usize, u: f64) -> f64 {
    f64::from(hz.ring_r0) + f64::from(hz.ring_speed) * (u - ring_start(m, k))
}

/// JS `hDist(p, x, z)` / `hDist3(x0, z0, x1, z1)` (bossHazards.js L354-355):
/// planar distance.
#[must_use]
fn h_dist(x0: f64, z0: f64, x1: f64, z1: f64) -> f64 {
    (x0 - x1).hypot(z0 - z1)
}

/// JS `smoothstep(a, b, x)` (core/ctx.js L30) in f64.
#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------
// threat() — what bots read (bossHazards.js L294-349; Task 47 consumes)
// ---------------------------------------------------------------------------

/// JS `threat()`'s return object (bossHazards.js L292-293).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threat {
    /// 0..1 danger level.
    pub level: f64,
    /// Unit escape direction (JS L346-347, normalised at the end).
    pub ax: f64,
    pub az: f64,
    /// Seconds until a slam ring reaches here, `-1` when none (JS L296).
    pub ring_in: f64,
    /// A beam / spray that cover beats (JS L296/L331/L342).
    pub cover: bool,
    /// The sweep beam (diving under your own ink beats it too, JS L296/L331).
    pub beam: bool,
    /// A charge is coming down this lane (JS L296/L338).
    pub lane: bool,
}

impl Default for Threat {
    fn default() -> Self {
        // JS L296.
        Self {
            level: 0.0,
            ax: 0.0,
            az: 0.0,
            ring_in: -1.0,
            cover: false,
            beam: false,
            lane: false,
        }
    }
}

/// JS `push(lvl, dx, dz)` (bossHazards.js L297-301).
fn push(out: &mut Threat, lvl: f64, dx: f64, dz: f64) {
    // JS L298: `Math.hypot(dx, dz) || 1` — 0 and NaN both fall back.
    let l = dx.hypot(dz);
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    out.ax += (dx / l) * lvl; // JS L299
    out.az += (dz / l) * lvl;
    out.level = out.level.max(lvl); // JS L300
}

// ---------------------------------------------------------------------------
// impl Boss — the hazard bookkeeping + victim-side damage + host paint
// ---------------------------------------------------------------------------

impl<'a> Boss<'a> {
    /// JS `add(m)` (bossHazards.js L80-85): idempotent by `t0` (net replays,
    /// host migration); the move is logged for the audit.
    pub fn hz_add(&mut self, m: JVal) -> bool {
        let t0 = rec_t0(&m);
        if self.moves.iter().any(|x| rec_t0(x) == t0) {
            return false; // JS L81
        }
        // JS L83: `this.boss.log.moves.push([m.t0, m.id])`.
        let id = m
            .field("id")
            .and_then(JVal::as_str)
            .unwrap_or_default()
            .to_string();
        self.log.moves.push((t0, id));
        self.moves.push(m); // JS L82
        true
    }

    /// JS `current(bt)` (bossHazards.js L86): the move playing at boss time
    /// `bt` (latest record wins).
    #[must_use]
    pub fn hz_current(&self, bt: f32) -> Option<&JVal> {
        let bt = f64::from(bt);
        self.moves.iter().rev().find(|m| {
            let t0 = rec_t0(m);
            bt >= t0 && bt < t0 + move_times(m).end
        })
    }

    /// JS `fizzle(bt)` (bossHazards.js L88-98): defeat / time's up — every
    /// pending hazard stops landing. The in-flight barrels' fx burst
    /// (L91-97) is render-side (Task 46).
    pub fn hz_fizzle(&mut self, bt: f32) {
        if self.hz_dead >= 0.0 {
            return; // JS L89
        }
        self.hz_dead = bt; // JS L90
    }

    /// JS `_live(m, bt)` (bossHazards.js L101-105): hazards still able to
    /// hurt at boss time `bt` (a move stays listed until its last barrel /
    /// ring is spent).
    #[must_use]
    pub fn hz_live(&self, m: &JVal, bt: f32) -> bool {
        let t0 = rec_t0(m);
        let u = f64::from(bt) - t0; // JS L102
        let t = move_times(m);
        // JS L103: slam rings outlive the record.
        let extra = if rec_id(m) == Some(MoveId::Slam) {
            let hz = self.tb.hz;
            (f64::from(hz.ring_max) - f64::from(hz.ring_r0)) / f64::from(hz.ring_speed)
                + rec_rings(m).last().copied().unwrap_or(0.0)
        } else {
            0.0
        };
        // JS L104.
        u >= 0.0
            && u < t.end + extra + 0.2
            && (self.hz_dead < 0.0 || f64::from(bt) < f64::from(self.hz_dead))
    }

    /// JS `update(dt, bt, { live })` (bossHazards.js L107-116): prune spent
    /// records and their one-shot keys, then the victim-side damage slice.
    /// `fx.draw` (L115) is render-side (Task 46).
    pub fn hz_update(&mut self, dt: f32, bt: f32, live: bool, squad: &mut BossSquad<'_>) {
        // JS L108-111: walk backwards, splice at `end + 4`, drop the keys.
        let btf = f64::from(bt);
        let mut i = self.moves.len();
        while i > 0 {
            i -= 1;
            let stale = {
                let m = &self.moves[i];
                btf - rec_t0(m) > move_times(m).end + 4.0
            };
            if stale {
                let t0 = rec_t0(&self.moves[i]);
                self.moves.remove(i);
                // JS L110: `k.startsWith(m.t0 + '|')`.
                let prefix = format!("{}|", wire::num(t0));
                self.hz_hits.retain(|k| !k.starts_with(&prefix));
                // The JS WeakMap drops the record's geometry cache with it.
                self.hz_cache.remove(&t0.to_bits());
            }
        }
        let prev = self.hz_prev_bt; // JS L112
        if live {
            self.hz_hurt_local(prev, bt, dt, squad); // JS L113
        }
        self.hz_prev_bt = bt; // JS L114
    }

    /// JS `_hurtLocal(bt0, bt1, dt)` (bossHazards.js L119-187): victim-side
    /// damage — whoever owns a squidkid applies what that screen shows.
    /// `remote` actors are owned by their own clients (L122).
    pub fn hz_hurt_local(&mut self, bt0: f32, bt1: f32, dt: f32, squad: &mut BossSquad<'_>) {
        let mul = f64::from(self.dmg_mul); // JS L120
        let boss_pos = self.pos;
        let hz = self.tb.hz;
        let safe_pad = f64::from(self.tb.reach_step); // JS L35 `SAFE_PAD`
        let player_radius = f64::from(squad.player.radius); // JS L155 `PLAYER.radius`
        // JS iterates `hz.moves` while mutating `hits` / the geometry cache;
        // the port clones the records so `self` stays free for those.
        let moves = self.moves.clone();
        for idx in 0..squad.actors.len() {
            // JS L122: `a.remote || !a.alive || a.team === 1`.
            if squad.remote[idx] || !squad.actors[idx].alive || squad.actors[idx].team == 1 {
                continue;
            }
            let a = &squad.actors[idx];
            // JS L123: spawn-pad squidkids are out of reach of every hazard.
            if self.nav.in_pad(a.pos.x, a.pos.z, safe_pad as f32) {
                continue;
            }
            let nid = squad.ids[idx]; // JS L127 `a.nid ?? a.slot`
            let (apx, apy, apz) = (f64::from(a.pos.x), f64::from(a.pos.y), f64::from(a.pos.z));
            let submerged = a.submerged; // JS L151
            for m in &moves {
                if !self.hz_live(m, bt1) {
                    continue; // JS L125
                }
                let t0 = rec_t0(m);
                let num_t0 = wire::num(t0); // JS L127 `${m.t0}|…`
                let u0 = f64::from(bt0) - t0; // JS L126
                let u1 = f64::from(bt1) - t0;
                let tele = rec_d(m, 0); // JS L129
                let Some(id) = rec_id(m) else {
                    continue; // JS: no `switch` case matches
                };
                match id {
                    MoveId::Slam => {
                        let px = rec_p_f64(m, "x");
                        let py = rec_p_f64(m, "y");
                        let pz = rec_p_f64(m, "z");
                        let rings = rec_rings(m);
                        // JS L132: the impact circle, once, on the tele→act
                        // crossing (the band can't be skipped by a slow
                        // frame).
                        if u0 < tele
                            && u1 >= tele
                            && h_dist(apx, apz, px, pz) < f64::from(hz.slam_r) + 0.35
                            && apy < py + 2.0
                            && self.hz_once(&num_t0, "impact", 0, nid)
                        {
                            deal(squad, idx, f64::from(hz.slam_dmg) * mul, boss_pos); // JS L128/L132
                        }
                        for (k, &delay) in rings.iter().enumerate() {
                            let start = tele + delay; // JS L62 ringStart
                            if u1 < start {
                                continue; // JS L134
                            }
                            // JS L135: the swept band between the frame's
                            // start and end radius (no tunnelling).
                            let r0 = f64::from(hz.ring_r0)
                                + f64::from(hz.ring_speed) * (u0.max(start) - start);
                            let r1 =
                                f64::from(hz.ring_r0) + f64::from(hz.ring_speed) * (u1 - start);
                            if r0 > f64::from(hz.ring_max) {
                                continue; // JS L136
                            }
                            let d = h_dist(apx, apz, px, pz); // JS L137
                            // JS L139-140: the band, the wave hugging the
                            // floor (a hop clears it), the reach clip.
                            if d > r0 - f64::from(hz.ring_w) - 0.25
                                && d < r1 + f64::from(hz.ring_w) + 0.25
                                && apy < py + f64::from(hz.ring_h)
                                && apy > py - 0.8
                                && d < f64::from(self.hz_reach_at(
                                    m,
                                    px,
                                    pz,
                                    apx,
                                    apz,
                                    f64::from(hz.ring_max),
                                )) + 0.3
                                && self.hz_once(&num_t0, "ring", k as i64, nid)
                            {
                                deal(squad, idx, f64::from(hz.ring_dmg) * mul, boss_pos);
                            }
                        }
                    }
                    MoveId::Barrage => {
                        // JS L145-148: the landing circle, once per barrel.
                        for (i, b) in rec_pts(m).iter().enumerate() {
                            let land = b[3] + f64::from(hz.barrel_flight);
                            if u0 < land
                                && u1 >= land
                                && h_dist(apx, apz, b[0], b[2]) < f64::from(hz.barrel_r) + 0.35
                                && (apy - b[1]).abs() < 2.2
                                && self.hz_once(&num_t0, "barrel", i as i64, nid)
                            {
                                deal(squad, idx, f64::from(hz.barrel_dmg) * mul, boss_pos);
                            }
                        }
                    }
                    MoveId::Sweep => {
                        let act = rec_d(m, 1);
                        // JS L151: diving under your own ink ducks the beam.
                        if u1 < tele || u1 > tele + act || submerged {
                            continue;
                        }
                        let ang = beam_angle(m, u1); // JS L152
                        let len = f64::from(self.hz_beam_len(m, ang));
                        let (p_ox, p_oy) = (rec_p_f64(m, "ox"), rec_p_f64(m, "oy"));
                        let (p_oz, p_y) = (rec_p_f64(m, "oz"), rec_p_f64(m, "y"));
                        let (sx, sz) = (ang.sin(), ang.cos()); // JS L153
                        let dx = apx - p_ox;
                        let dz = apz - p_oz;
                        let along = dx * sx + dz * sz; // JS L154
                        let lat = (dx * sz - dz * sx).abs();
                        // JS L155-156: the lane slice, cover-checked.
                        if along > 1.5
                            && along < len + 0.4
                            && lat < f64::from(hz.beam_w) + player_radius
                            && apy < p_y + 2.4
                            && self.world.collision.los(
                                vec3(p_ox as f32, p_oy as f32, p_oz as f32),
                                vec3(apx as f32, (apy + 0.9) as f32, apz as f32),
                            )
                        {
                            deal(
                                squad,
                                idx,
                                f64::from(hz.beam_dps) * mul * f64::from(dt),
                                boss_pos,
                            );
                        }
                    }
                    MoveId::Charge => {
                        let act = rec_d(m, 1);
                        if u1 < tele || u1 > tele + act {
                            continue; // JS L160
                        }
                        let (p_x, p_y, p_z) =
                            (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                        let (p_yaw, p_v, p_l) =
                            (rec_p_f64(m, "yaw"), rec_p_f64(m, "v"), rec_p_f64(m, "L"));
                        let s = charge_dist(&hz, p_v, p_l, u1, tele); // JS L161
                        let (sy, cy) = (p_yaw.sin(), p_yaw.cos());
                        let bx = p_x + sy * s;
                        let bz = p_z + cy * s;
                        // JS L162-176: the two WALL_BODY circles (bossNav.js
                        // L10) along the lane.
                        for c in self.tb.nav.wall_body {
                            let cx = bx + sy * f64::from(c.z);
                            let cz = bz + cy * f64::from(c.z);
                            if (apx - cx).hypot(apz - cz) < f64::from(c.r) + 0.35 && apy < p_y + 3.5
                            {
                                // JS L165-167.
                                if self.hz_once(&num_t0, "ram", 0, nid) {
                                    // JS L169-171: bowled aside, away from
                                    // the lane, and popped up.
                                    let side = if (apx - bx) * cy - (apz - bz) * sy >= 0.0 {
                                        1.0
                                    } else {
                                        -1.0
                                    };
                                    let a = &mut squad.actors[idx];
                                    a.vel.x =
                                        (f64::from(a.vel.x) + cy * side * 9.0 + sy * 5.0) as f32;
                                    a.vel.z =
                                        (f64::from(a.vel.z) + -sy * side * 9.0 + cy * 5.0) as f32;
                                    a.vel.y = 7.0;
                                    a.grounded = false;
                                    deal(squad, idx, f64::from(hz.charge_dmg) * mul, boss_pos); // JS L172
                                }
                                break; // JS L174
                            }
                        }
                    }
                    MoveId::Crablets => {
                        // JS L130: no `crablets` case — crablets hurt through
                        // `crabBurst` (L189-194).
                    }
                    MoveId::Frenzy => {
                        let act = rec_d(m, 1);
                        if u1 < tele || u1 > tele + act {
                            continue; // JS L180
                        }
                        let (p_x, p_y, p_z) =
                            (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                        // JS L181: the spray, cover-checked.
                        if h_dist(apx, apz, p_x, p_z) < f64::from(hz.frenzy_r)
                            && apy < p_y + 3.0
                            && self.world.collision.los(
                                vec3(p_x as f32, (p_y + 1.6) as f32, p_z as f32),
                                vec3(apx as f32, (apy + 0.8) as f32, apz as f32),
                            )
                        {
                            deal(
                                squad,
                                idx,
                                f64::from(hz.frenzy_dps) * mul * f64::from(dt),
                                boss_pos,
                            );
                        }
                    }
                }
            }
        }
    }

    /// JS `once(kind, i, dmg)` key check (bossHazards.js L127-128): `true`
    /// when this one-shot hit has not landed yet (the key is consumed). The
    /// key is `${t0}|${kind}|${i}|${nid}` with `t0` in JS number form.
    fn hz_once(&mut self, num_t0: &str, kind: &str, i: i64, nid: i32) -> bool {
        let key = format!("{num_t0}|{kind}|{i}|{nid}");
        self.hz_hits.insert(key)
    }

    /// JS `crabBurst(x, y, z, id)` (bossHazards.js L189-194): a crablet
    /// burst — every local squidkid nearby takes the hit (the `id` parameter
    /// is unused in JS).
    pub fn hz_crab_burst(&mut self, x: f32, y: f32, z: f32, squad: &mut BossSquad<'_>) {
        let boss_pos = self.pos;
        let hz = self.tb.hz;
        let mul = f64::from(self.dmg_mul); // JS L192
        let safe_pad = f64::from(self.tb.reach_step); // JS L35 `SAFE_PAD`
        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
        for idx in 0..squad.actors.len() {
            // JS L191.
            if squad.remote[idx]
                || !squad.actors[idx].alive
                || squad.actors[idx].team == 1
                || self.nav.in_pad(
                    squad.actors[idx].pos.x,
                    squad.actors[idx].pos.z,
                    safe_pad as f32,
                )
            {
                continue;
            }
            let a = &squad.actors[idx];
            // JS L192.
            if (f64::from(a.pos.x) - fx).hypot(f64::from(a.pos.z) - fz) < f64::from(hz.crab_r)
                && (f64::from(a.pos.y) - fy).abs() < 1.6
            {
                deal(squad, idx, f64::from(hz.crab_dmg) * mul, boss_pos);
            }
        }
    }

    /// JS `_c(m)` (bossHazards.js L201): the per-move derived-geometry cache,
    /// keyed by `t0` (JS's WeakMap on the record).
    fn hz_cache(&mut self, t0: f64) -> &mut HzCache {
        self.hz_cache.entry(t0.to_bits()).or_default()
    }

    /// JS `reach(m, x, z, R)` (bossHazards.js L202-213): how far a ground
    /// hazard centred on `(x, z)` spreads along each of `REACH_N` headings
    /// before its floor ends — cached on the move (the stage doesn't change).
    pub fn hz_reach(&mut self, m: &JVal, x: f64, z: f64, r: f64) -> Vec<f32> {
        let t0 = rec_t0(m);
        if let Some(reach) = &self.hz_cache(t0).reach {
            return reach.clone(); // JS L204
        }
        let out = self.hz_reach_calc(x, z, r);
        self.hz_cache(t0).reach = Some(out.clone()); // JS L212
        out
    }

    /// The pure probe loop of [`Self::hz_reach`] (JS bossHazards.js L206-210),
    /// without the per-move cache — read-only so the bot threat read
    /// ([`Self::threat`], which bots hold as `&Boss`) can use it. Values are
    /// identical; only the caching differs (perf-only deviation, D-T43-9
    /// precedent).
    fn hz_reach_calc(&self, x: f64, z: f64, r: f64) -> Vec<f32> {
        let n = self.tb.reach_n as usize; // JS L36 `REACH_N`
        let mut out = vec![0.0f32; n];
        for (k, o) in out.iter_mut().enumerate() {
            // JS L207.
            let a = ((k as f64 + 0.5) / n as f64) * TAU;
            let (sx, sz) = (a.sin(), a.cos());
            let mut rr = 0.0f64;
            // JS L209: step 0.35 m until the floor ends (a wall, a drop, a
            // pad — `kindAt !== 0`), never inside the first 1.2 m.
            while rr < r
                && (rr < 1.2 || self.nav.kind_at((x + sx * rr) as f32, (z + sz * rr) as f32) == 0)
            {
                rr += 0.35;
            }
            *o = rr.min(r) as f32; // JS L210 (Float32Array store)
        }
        out
    }

    /// JS `reachAt(m, x, z, px, pz, R)` (bossHazards.js L214-217): the reach
    /// along the heading from `(x, z)` to `(px, pz)`.
    pub fn hz_reach_at(&mut self, m: &JVal, x: f64, z: f64, px: f64, pz: f64, r: f64) -> f32 {
        let k = self.reach_bucket(x, z, px, pz);
        self.hz_reach(m, x, z, r)[k] // JS L216
    }

    /// Read-only [`Self::hz_reach_at`] (JS bossHazards.js L214-217): same
    /// heading bucket, but recomputes the reach through
    /// [`Self::hz_reach_calc`] instead of the per-move cache, so it works on
    /// `&self`. Used by [`Self::threat`]; values identical.
    fn hz_reach_at_ro(&self, x: f64, z: f64, px: f64, pz: f64, r: f64) -> f32 {
        let k = self.reach_bucket(x, z, px, pz);
        self.hz_reach_calc(x, z, r)[k] // JS L216
    }

    /// The heading-bucket index of JS L215: `floor(atan2(px-x, pz-z) / TAU * N)`.
    fn reach_bucket(&self, x: f64, z: f64, px: f64, pz: f64) -> usize {
        let n = self.tb.reach_n as usize;
        // JS L215.
        let a = (px - x).atan2(pz - z);
        let a = if a < 0.0 { a + TAU } else { a };
        (a / TAU * n as f64).floor() as usize % n
    }

    /// JS `beamLen(m, ang)` (bossHazards.js L220-230): where the sweep beam
    /// stops — the first wall along it, cached per move + angle bucket
    /// (`Math.round(ang * 90)`; walls don't move).
    pub fn hz_beam_len(&mut self, m: &JVal, ang: f64) -> f32 {
        let t0 = rec_t0(m);
        let key = js_round(ang * 90.0) as i32; // JS L221
        if let Some(&l) = self.hz_cache(t0).beam.get(&key) {
            return l; // JS L222-223
        }
        let beam_len = f64::from(self.tb.hz.beam_len);
        // JS L224-226: raycast from the emitter, `skipGrates = true`.
        let a = f64::from(key) / 90.0;
        let h = self.world.collision.raycast(
            vec3(
                rec_p_f64(m, "ox") as f32,
                (rec_p_f64(m, "y") + 1.0) as f32,
                rec_p_f64(m, "oz") as f32,
            ),
            vec3(a.sin() as f32, 0.0, a.cos() as f32),
            beam_len as f32,
            true,
        );
        let l = if h.hit { h.dist } else { beam_len as f32 };
        self.hz_cache(t0).beam.insert(key, l); // JS L227
        l
    }

    /// JS `paint(bt0, bt1)` (bossHazards.js L233-289): the host's ink for
    /// the boss-time slice just advanced. Host gating is the caller's
    /// (boss.js L108, Task 43 wiring); the sim records
    /// [`BossEvent::Splat`] with `team: 1` (the boss's ink) instead of
    /// touching the paint grid, and the JS `Math.random()` seeds draw from
    /// the seeded boss rng.
    pub fn hz_paint(&mut self, bt0: f32, bt1: f32, player: &PlayerTuning) {
        let hz = self.tb.hz;
        let moves = self.moves.clone();
        for m in &moves {
            if !self.hz_live(m, bt1) {
                continue; // JS L244
            }
            let t0 = rec_t0(m);
            let u0 = f64::from(bt0) - t0; // JS L245
            let u1 = f64::from(bt1) - t0;
            let tele = rec_d(m, 0);
            let act = rec_d(m, 1);
            match rec_id(m) {
                Some(MoveId::Slam) => {
                    let (px, py, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                    // JS L248: the slam mark + its six satellite blobs.
                    if u0 < tele && u1 >= tele {
                        self.hz_splat(px, py, pz, 2.8, player);
                        for i in 0..7 {
                            let a = f64::from(i) * 0.9;
                            self.hz_splat(px + a.cos() * 2.6, py, pz + a.sin() * 2.6, 1.3, player);
                        }
                    }
                    // JS L249-258: the ring fronts, discretised every 2.2 m
                    // of expansion.
                    let rings = rec_rings(m);
                    for (k, &delay) in rings.iter().enumerate() {
                        let start = tele + delay;
                        for (j, _t) in paint_steps(u0, u1, start, 2.2 / f64::from(hz.ring_speed)) {
                            let r = f64::from(hz.ring_r0) + 2.2 * (j + 1.0); // JS L250
                            if r > f64::from(hz.ring_max) - 1.0 {
                                continue; // JS L251
                            }
                            let n = js_round(r * 1.4) as i64; // JS L252
                            for i in 0..n {
                                // JS L253-256.
                                let a = (i as f64 / n as f64) * TAU + (k as f64) * 0.4 + j;
                                let x = px + a.cos() * r;
                                let z = pz + a.sin() * r;
                                if r < f64::from(self.hz_reach_at(
                                    m,
                                    px,
                                    pz,
                                    x,
                                    z,
                                    f64::from(hz.ring_max),
                                )) {
                                    self.hz_splat(x, py, z, 0.75, player); // JS L256
                                }
                            }
                        }
                    }
                }
                Some(MoveId::Barrage) => {
                    // JS L261-264: each landing circle + four satellites.
                    for b in rec_pts(m) {
                        let land = b[3] + f64::from(hz.barrel_flight);
                        if u0 < land && u1 >= land {
                            self.hz_splat(b[0], b[1], b[2], 2.2, player);
                            for i in 0..4 {
                                let a = f64::from(i) * 1.6 + b[0];
                                self.hz_splat(
                                    b[0] + a.cos() * 1.8,
                                    b[1],
                                    b[2] + a.sin() * 1.8,
                                    1.0,
                                    player,
                                );
                            }
                        }
                    }
                }
                Some(MoveId::Sweep) => {
                    // JS L266-272: the beam tip every 0.06 s, every other
                    // beat also a mid-fan blob.
                    let (p_ox, p_oz, p_y) =
                        (rec_p_f64(m, "ox"), rec_p_f64(m, "oz"), rec_p_f64(m, "y"));
                    for (k, t) in paint_steps(u0, u1.min(tele + act), tele, 0.06) {
                        let ang = beam_angle(m, t); // JS L268
                        let l = f64::from(self.hz_beam_len(m, ang));
                        self.hz_splat(
                            p_ox + ang.sin() * l,
                            p_y,
                            p_oz + ang.cos() * l,
                            1.05,
                            player,
                        ); // JS L269
                        if (k as i64) % 2 != 0 {
                            // JS L270.
                            let f = 0.45 + 0.4 * ((k * 0.37) % 1.0);
                            self.hz_splat(
                                p_ox + ang.sin() * l * f,
                                p_y,
                                p_oz + ang.cos() * l * f,
                                0.8,
                                player,
                            );
                        }
                    }
                }
                Some(MoveId::Charge) => {
                    // JS L273-278: the scuff trail, alternating sides.
                    let (p_x, p_y, p_z) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                    let (p_yaw, p_v, p_l) =
                        (rec_p_f64(m, "yaw"), rec_p_f64(m, "v"), rec_p_f64(m, "L"));
                    for (k, t) in paint_steps(u0, u1.min(tele + act), tele, 0.09) {
                        let s = charge_dist(&hz, p_v, p_l, t, tele); // JS L274
                        let (sx, sz) = (p_yaw.sin(), p_yaw.cos());
                        let side = if (k as i64) % 2 != 0 { 1.3 } else { -1.3 };
                        self.hz_splat(
                            p_x + sx * (s - 1.0) + sz * side,
                            p_y,
                            p_z + sz * (s - 1.0) - sx * side,
                            1.0,
                            player,
                        ); // JS L276
                    }
                }
                Some(MoveId::Frenzy) => {
                    // JS L279-286: two jets per 0.07 s beat, golden-ratio
                    // spread.
                    let (p_x, p_y, p_z, rot0) = (
                        rec_p_f64(m, "x"),
                        rec_p_f64(m, "y"),
                        rec_p_f64(m, "z"),
                        rec_p_f64(m, "rot0"),
                    );
                    for (k, _t) in paint_steps(u0, u1.min(tele + act), tele, 0.07) {
                        for j in 0..2 {
                            let fj = f64::from(j);
                            // JS L282.
                            let a = rot0 + (k * 0.07) * 2.6 + fj * PI + (k % 3.0) * 0.7;
                            let r = f64::from(hz.frenzy_r)
                                * (0.35 + 0.6 * ((k * 0.618 + fj * 0.3) % 1.0));
                            self.hz_splat(p_x + a.sin() * r, p_y, p_z + a.cos() * r, 0.9, player); // JS L283
                        }
                    }
                }
                Some(MoveId::Crablets) | None => {
                    // JS L246: no `crablets` paint case.
                }
            }
        }
    }

    /// JS `splat(x, y, z, r)` (bossHazards.js L236-241): the ground probe
    /// (DOWN 4 m, `skipGrates = true`), the pad / water rejects, then the
    /// paint point at `hit + normal · 0.08`.
    fn hz_splat(&mut self, x: f64, y: f64, z: f64, r: f64, player: &PlayerTuning) {
        if self.nav.in_pad(x as f32, z as f32, 0.3) {
            return; // JS L237
        }
        // JS L238: `_v.set(x, y + 1.5, z)` DOWN 4, skipGrates = true.
        let g = self.world.collision.raycast(
            vec3(x as f32, (y + 1.5) as f32, z as f32),
            Vec3::NEG_Y,
            4.0,
            true,
        );
        // JS L239: `!g.hit || g.point.y < PLAYER.waterY`.
        if !g.hit || f64::from(g.point.y) < f64::from(player.water_y) {
            return;
        }
        // JS L240: `_v3.copy(g.point).addScaledVector(g.normal, 0.08)`,
        // `{ seed: Math.random() }` → the seeded boss rng.
        let pos = g.point + g.normal * 0.08;
        let seed = self.rng.next_f32();
        self.events.push(BossEvent::Splat {
            pos,
            radius: r as f32,
            team: 1,
            seed,
        });
    }

    /// JS `threat(x, y, z, horizon = 1.2)` (bossHazards.js L294-349): what
    /// bots read to step out of circles, hop rings and take cover.
    ///
    /// Takes `&self` (the JS is a pure read of the move records): the bot
    /// brain holds the boss immutably through [`crate::bot::BotCtx::boss`].
    /// The slam-ring reach probe therefore recomputes without the per-move
    /// cache ([`Self::hz_reach_at_ro`]) — perf-only, values identical.
    pub fn threat(&self, x: f64, y: f64, z: f64, horizon: f64) -> Threat {
        let mut out = Threat::default(); // JS L295-296
        let bt = f64::from(self.bt); // JS L295
        // JS L302.
        if self.nav.in_pad(x as f32, z as f32, self.tb.reach_step) {
            return out;
        }
        let hz = self.tb.hz;
        let yaw = f64::from(self.yaw); // JS L331: `this.boss.yaw`
        let moves = self.moves.clone();
        for m in &moves {
            if !self.hz_live(m, self.bt) {
                continue; // JS L304
            }
            let t0 = rec_t0(m);
            let u = bt - t0; // JS L305
            let tele = rec_d(m, 0);
            let act = rec_d(m, 1);
            match rec_id(m) {
                Some(MoveId::Slam) => {
                    let (px, py, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                    // JS L308: the wind-up circle.
                    if u < tele + 0.1 && h_dist(x, z, px, pz) < f64::from(hz.slam_r) + 1.2 {
                        push(&mut out, 1.0, x - px, z - pz);
                    }
                    let d = h_dist(x, z, px, pz); // JS L309
                    let rings = rec_rings(m);
                    for &delay in rings.iter() {
                        // JS L310-311: `r = ringRadius(m, k, u)` with
                        // ringStart = tele + rings[k].
                        let r =
                            f64::from(hz.ring_r0) + f64::from(hz.ring_speed) * (u - (tele + delay));
                        let t_in = (d - f64::from(hz.ring_w) - r) / f64::from(hz.ring_speed);
                        // JS L312.
                        if d > f64::from(self.hz_reach_at_ro(px, pz, x, z, f64::from(hz.ring_max)))
                            + 0.3
                            || (y - py).abs() > 0.8
                        {
                            continue;
                        }
                        // JS L313.
                        if r < f64::from(hz.ring_max)
                            && t_in > -0.1
                            && t_in < horizon
                            && (out.ring_in < 0.0 || t_in < out.ring_in)
                        {
                            out.ring_in = t_in.max(0.0);
                        }
                    }
                }
                Some(MoveId::Barrage) => {
                    // JS L318-321: barrels about to land near here.
                    for b in rec_pts(m) {
                        let land = b[3] + f64::from(hz.barrel_flight);
                        if u < land + 0.05
                            && u > b[3] - 0.6
                            && h_dist(x, z, b[0], b[2]) < f64::from(hz.barrel_r) + 1.0
                        {
                            push(&mut out, 0.9, x - b[0], z - b[2]);
                        }
                    }
                }
                Some(MoveId::Sweep) => {
                    if u > tele + act {
                        continue; // JS L324
                    }
                    let (p_ox, p_oz) = (rec_p_f64(m, "ox"), rec_p_f64(m, "oz"));
                    let d = h_dist(x, z, p_ox, p_oz); // JS L325
                    if d > f64::from(hz.beam_len) + 1.5 {
                        continue; // JS L326
                    }
                    // JS L328-330: inside the part of the fan the beam has
                    // still to cross.
                    let ang = (x - p_ox).atan2(z - p_oz);
                    let a0 = rec_p_f64(m, "a0");
                    let a1 = rec_p_f64(m, "a1");
                    let from = if u < tele { a0 } else { beam_angle(m, u) };
                    let lo = from.min(a1) - 0.2;
                    let hi = from.max(a1) + 0.2;
                    let mut a = ang;
                    while a < lo - PI {
                        a += TAU;
                    }
                    while a > hi + PI {
                        a -= TAU;
                    }
                    if a >= lo && a <= hi {
                        // JS L331: cover beats the beam; escape across the
                        // boss's facing.
                        out.cover = true;
                        out.beam = true;
                        push(&mut out, 0.8, -yaw.sin(), -yaw.cos());
                    }
                }
                Some(MoveId::Charge) => {
                    if u > tele + act {
                        continue; // JS L335
                    }
                    let (p_x, p_z, p_yaw) =
                        (rec_p_f64(m, "x"), rec_p_f64(m, "z"), rec_p_f64(m, "yaw"));
                    let (p_v, p_l) = (rec_p_f64(m, "v"), rec_p_f64(m, "L"));
                    // JS L336.
                    let s = charge_dist(&hz, p_v, p_l, u.max(tele), tele);
                    let (sx, sz) = (p_yaw.sin(), p_yaw.cos());
                    let dx = x - p_x; // JS L337
                    let dz = z - p_z;
                    let along = dx * sx + dz * sz;
                    let lat = dx * sz - dz * sx;
                    // JS L338: `Math.sign(lat || 1)` — 0 / NaN both → 1.
                    if along > s - 3.0 && along < p_l + 5.0 && lat.abs() < 3.6 {
                        out.lane = true;
                        let sgn = if lat == 0.0 || lat.is_nan() {
                            1.0
                        } else {
                            lat.signum()
                        };
                        push(&mut out, 1.0, sz * sgn, -sx * sgn);
                    }
                }
                Some(MoveId::Crablets) => {
                    // JS L306: no `crablets` case.
                }
                Some(MoveId::Frenzy) => {
                    // JS L341-343.
                    let (px, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "z"));
                    if u < tele + act && h_dist(x, z, px, pz) < f64::from(hz.frenzy_r) + 2.0 {
                        out.cover = true;
                        push(&mut out, 0.9, x - px, z - pz);
                    }
                }
                None => {}
            }
        }
        // JS L346-347: normalise the escape direction.
        let l = out.ax.hypot(out.az);
        if l > 1e-3 {
            out.ax /= l;
            out.az /= l;
        }
        out
    }
}

/// JS `_damage(a, dmg)` (bossHazards.js L195) with the D-T43 pseudo-attacker:
/// the boss has no team-1 entity in the match, so the hit is credited to
/// `(team 1, slot 0)` from the boss position.
fn deal(squad: &mut BossSquad<'_>, idx: usize, dmg: f64, boss_pos: Vec3) {
    if dmg > 0.0 {
        let a: &mut Actor = &mut squad.actors[idx];
        a.damage(dmg as f32, Some((1, 0)), Some(boss_pos), squad.player);
    }
}

/// JS `steps(u0, u1, start, every, fn)` (bossHazards.js L242): the discrete
/// beats `(start + k·every]` crossed by the `(u0, u1]` time slice, as
/// `(k, start + k·every)` pairs.
fn paint_steps(u0: f64, u1: f64, start: f64, every: f64) -> Vec<(f64, f64)> {
    let k0 = ((u0 - start) / every).floor(); // JS L242
    let k1 = ((u1 - start) / every).floor();
    let mut out = Vec::new();
    let mut k = (k0 + 1.0).max(0.0);
    while k <= k1 {
        out.push((k, start + k * every));
        k += 1.0;
    }
    out
}

// ---------------------------------------------------------------------------
// Smoke tests (full TR-43.1 / TR-43.2 harnesses are Task 43 batch D)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Actor, SimWorld};
    use crate::boss::{BossEvent, BossSquad};
    use crate::collision::CollisionWorld;
    use glam::vec3;

    use super::super::{BossConfig, nav::BossNav};

    fn with_boss<T>(f: impl FnOnce(&mut Boss<'_>) -> T) -> T {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tuning = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tuning.boss.nav);
        let mut boss = Boss::new(
            &world,
            &nav,
            &tuning.boss,
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: true,
                seed: 7,
            },
        );
        f(&mut boss)
    }

    fn num(v: f64) -> JVal {
        JVal::Num(v)
    }

    fn slam_rec(t0: f64, mark: [f64; 3]) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str("slam".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(1.0), num(1.9), num(1.0)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("x".into(), num(mark[0])),
                    ("y".into(), num(mark[1])),
                    ("z".into(), num(mark[2])),
                    ("rings".into(), JVal::Arr(vec![num(0.0), num(0.6)])),
                ]),
            ),
        ])
    }

    // JS L80-85: `add` is idempotent by t0 and logs the move.
    #[test]
    fn hz_add_idempotent_and_logged() {
        with_boss(|boss| {
            assert!(boss.hz_add(slam_rec(10.0, [0.0, 0.0, 0.0])));
            assert!(!boss.hz_add(slam_rec(10.0, [0.0, 0.0, 0.0]))); // JS L81
            assert!(boss.hz_add(slam_rec(11.0, [0.0, 0.0, 0.0])));
            assert_eq!(boss.moves.len(), 2);
            assert_eq!(boss.log.moves.len(), 2);
            assert_eq!(boss.log.moves[0], (10.0, "slam".to_string()));
        });
    }

    // JS L39-46: the phase windows.
    #[test]
    fn move_phase_at_windows() {
        let m = slam_rec(0.0, [0.0, 0.0, 0.0]); // d = [1.0, 1.9, 1.0], end 3.9
        assert_eq!(move_phase_at(&m, -0.5), None);
        assert_eq!(move_phase_at(&m, 0.0), Some("tele"));
        assert_eq!(move_phase_at(&m, 0.999), Some("tele"));
        assert_eq!(move_phase_at(&m, 1.0), Some("act"));
        assert_eq!(move_phase_at(&m, 2.899), Some("act"));
        assert_eq!(move_phase_at(&m, 2.9), Some("rec"));
        assert_eq!(move_phase_at(&m, 3.899), Some("rec"));
        assert_eq!(move_phase_at(&m, 3.9), None);
        assert_eq!(move_times(&m).end, 3.9);
    }

    // JS L57-60: the arc starts at the shell, ends at the circle.
    #[test]
    fn barrel_pos_endpoints() {
        let s = [1.0, 5.0, -2.0];
        let b = [4.0, 0.0, 6.0, 1.2];
        let p0 = barrel_pos(s, &b, 0.0);
        assert_eq!(p0, s);
        let p1 = barrel_pos(s, &b, 1.0);
        assert!(
            (p1[0] - b[0]).abs() < 1e-12
                && (p1[1] - b[1]).abs() < 1e-12
                && (p1[2] - b[2]).abs() < 1e-12
        );
        // Mid-flight: peak 5 + 0.35·hypot(3, 8), arc term peak·4·k·(1−k).
        let k = 0.5;
        let peak = 5.0 + 0.35 * 3.0f64.hypot(8.0);
        let pm = barrel_pos(s, &b, k);
        assert!((pm[1] - (s[1] + (b[1] - s[1]) * k + peak * 4.0 * k * (1.0 - k))).abs() < 1e-12);
    }

    // JS L48-55: the ramp then full speed, capped at the lane length.
    #[test]
    fn charge_dist_ramp_and_cap() {
        let hz = crate::embedded_tuning().boss.hz;
        let v = f64::from(hz.charge_speed[1]);
        let ramp = f64::from(hz.charge_ramp);
        assert_eq!(charge_dist(&hz, v, 30.0, 0.5, 1.0), 0.0); // JS L50
        // Half through the ramp: 0.5·a·t².
        let t = ramp * 0.5;
        let a = v / ramp;
        let s = charge_dist(&hz, v, 30.0, 1.0 + t, 1.0);
        assert!((s - 0.5 * a * t * t).abs() < 1e-9);
        // Past the ramp: linear, then capped at L.
        let s = charge_dist(&hz, v, 30.0, 1.0 + ramp + 1.0, 1.0);
        assert!((s - (0.5 * v * ramp + v * 1.0)).abs() < 1e-9);
        assert_eq!(charge_dist(&hz, v, 30.0, 100.0, 1.0), 30.0); // JS L53
        // chargeTime: beyond the ramp distance, r + (L − s0)/v (JS L55).
        let s0 = 0.5 * v * ramp;
        let l = s0 + 10.0;
        assert!((charge_time(&hz, v, l) - (ramp + 10.0 / v)).abs() < 1e-9);
        // Within the ramp: sqrt(2·L·r / v).
        assert!((charge_time(&hz, v, s0) - ((2.0 * s0 * ramp) / v).sqrt()).abs() < 1e-9);
    }

    // JS L294-349: a fresh boss threatens nothing; a slam wind-up pushes the
    // point directly away from the mark and reports the ring arrival.
    #[test]
    fn threat_basic() {
        with_boss(|boss| {
            boss.bt = 5.0;
            let t = boss.threat(0.0, 0.0, 0.0, 1.2);
            assert_eq!(t, Threat::default()); // no moves
            assert_eq!(t.ring_in, -1.0);

            // A slam about to land at (0, 0, 8): tele 1.0, u = 0.5 < tele + 0.1.
            boss.hz_add(slam_rec(4.5, [0.0, 0.0, 8.0]));
            let t = boss.threat(0.0, 0.0, 4.0, 1.2);
            assert_eq!(t.level, 1.0);
            assert!((t.az - -1.0).abs() < 1e-9); // escape away from the mark (−z)
            // Ring 0 launches at tele; its front reaches d = 4 at tIn =
            // (4 − ringW − r)/ringSpeed with r = ringR0 + ringSpeed·(u − tele).
            let r = f64::from(boss.tb.hz.ring_r0) + f64::from(boss.tb.hz.ring_speed) * (0.5 - 1.0);
            let expect =
                (4.0 - f64::from(boss.tb.hz.ring_w) - r) / f64::from(boss.tb.hz.ring_speed);
            assert!(
                (t.ring_in - expect).abs() < 1e-9,
                "ring_in {} vs {expect}",
                t.ring_in
            );
        });
    }

    // JS L221: the beam cache bucket is `Math.round(ang * 90)`.
    #[test]
    fn beam_len_bucket_is_js_round() {
        assert_eq!(js_round(0.4 * 90.0), 36.0);
        assert_eq!(js_round(-0.4 * 90.0), -36.0);
        assert_eq!(js_round(0.5), 1.0); // ties toward +∞, like Math.round
    }

    // -----------------------------------------------------------------------
    // TR-43.1 / TR-43.2 behaviour tests (Task 43 batch D)
    // -----------------------------------------------------------------------

    /// Static team-0 squad fixture (same shape as the brain harness).
    struct Kit {
        actors: Vec<Actor>,
        ids: Vec<i32>,
        remote: Vec<bool>,
        player: crate::tuning::PlayerTuning,
    }

    impl Kit {
        fn new() -> Self {
            Self {
                actors: Vec::new(),
                ids: Vec::new(),
                remote: Vec::new(),
                player: crate::embedded_tuning().player,
            }
        }
        fn add(&mut self, x: f32, y: f32, z: f32, id: i32) {
            self.add_r(x, y, z, id, false);
        }
        fn add_r(&mut self, x: f32, y: f32, z: f32, id: i32, remote: bool) {
            let mut a = Actor::new(0, self.actors.len(), &self.player);
            a.pos = vec3(x, y, z);
            self.actors.push(a);
            self.ids.push(id);
            self.remote.push(remote);
        }
        fn squad(&mut self) -> BossSquad<'_> {
            BossSquad {
                actors: &mut self.actors,
                ids: &self.ids,
                remote: &self.remote,
                player: &self.player,
            }
        }
    }

    fn with_squad<T>(sim: bool, seed: u64, f: impl FnOnce(&mut Boss<'_>, &mut Kit) -> T) -> T {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tuning = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tuning.boss.nav);
        let mut boss = Boss::new(
            &world,
            &nav,
            &tuning.boss,
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim,
                seed,
            },
        );
        let mut kit = Kit::new();
        f(&mut boss, &mut kit)
    }

    // Flat parameter list mirrors the JS record fields one-for-one (fidelity).
    #[allow(clippy::too_many_arguments)]
    fn sweep_rec(
        t0: f64,
        ox: f64,
        oy: f64,
        oz: f64,
        y: f64,
        a0: f64,
        a1: f64,
        tele: f64,
        act: f64,
    ) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str("sweep".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(tele), num(act), num(0.9)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("ox".into(), num(ox)),
                    ("oy".into(), num(oy)),
                    ("oz".into(), num(oz)),
                    ("y".into(), num(y)),
                    ("a0".into(), num(a0)),
                    ("a1".into(), num(a1)),
                    ("c".into(), num(0.0)),
                ]),
            ),
        ])
    }

    // Flat parameter list mirrors the JS record fields one-for-one (fidelity).
    #[allow(clippy::too_many_arguments)]
    fn charge_rec(
        t0: f64,
        x: f64,
        y: f64,
        z: f64,
        yaw: f64,
        l: f64,
        v: f64,
        tele: f64,
        act: f64,
    ) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str("charge".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(tele), num(act), num(4.2)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("x".into(), num(x)),
                    ("y".into(), num(y)),
                    ("z".into(), num(z)),
                    ("yaw".into(), num(yaw)),
                    ("L".into(), num(l)),
                    ("wall".into(), num(1.0)),
                    ("v".into(), num(v)),
                    ("stun".into(), num(1.0)),
                ]),
            ),
        ])
    }

    fn frenzy_rec(t0: f64, x: f64, y: f64, z: f64) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str("frenzy".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(1.2), num(3.0), num(1.7)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("x".into(), num(x)),
                    ("y".into(), num(y)),
                    ("z".into(), num(z)),
                    ("rot0".into(), num(0.5)),
                    ("spin".into(), num(1.0)),
                    ("stun".into(), num(1.0)),
                ]),
            ),
        ])
    }

    /// A slam record without rings: only the impact + the tele paint fire.
    fn slam_rec_no_rings(t0: f64, c: [f64; 3]) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str("slam".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(1.0), num(1.9), num(1.0)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("x".into(), num(c[0])),
                    ("y".into(), num(c[1])),
                    ("z".into(), num(c[2])),
                    ("rings".into(), JVal::Arr(vec![])),
                ]),
            ),
        ])
    }

    /// A heading from the home emitter where the beam runs long and the
    /// eye-line is clear (Tidewater-measured, not assumed).
    fn open_beam_heading(boss: &mut Boss<'_>, hx: f64, hy: f64, hz: f64) -> f64 {
        let probe = sweep_rec(-5.0, hx, hy + 2.3, hz, hy, 0.0, 0.0, 0.5, 1.0);
        for k in 0..48 {
            let a = f64::from(k) / 48.0 * TAU;
            if f64::from(boss.hz_beam_len(&probe, a)) < 7.0 {
                continue;
            }
            let (ax, az) = (hx + a.sin() * 6.0, hz + a.cos() * 6.0);
            let ay = f64::from(boss.world.collision.ground_height(
                ax as f32,
                az as f32,
                (hy + 2.0) as f32,
                false,
            ));
            if !ay.is_finite() || (ay - hy).abs() > 0.7 {
                continue;
            }
            if boss.world.collision.los(
                vec3(hx as f32, (hy + 2.3) as f32, hz as f32),
                vec3(ax as f32, (ay + 0.9) as f32, az as f32),
            ) {
                return a;
            }
        }
        panic!("no clear beam heading from the Tidewater home emitter");
    }

    fn splat_count(boss: &mut Boss<'_>) -> usize {
        boss.drain_events()
            .into_iter()
            .filter(|e| matches!(e, BossEvent::Splat { .. }))
            .count()
    }

    // -- geometry reference values ------------------------------------------

    #[test]
    fn ring_radius_is_linear_in_u() {
        // JS bossHazards.js L62-63: ringStart = tele + rings[k];
        // ringRadius = ringR0 + ringSpeed·(u − ringStart).
        let hz = crate::embedded_tuning().boss.hz;
        let m = slam_rec(0.0, [0.0, 0.0, 0.0]); // d = [1.0, 1.9, 1.0]
        let tele = rec_d(&m, 0);
        assert_eq!(ring_start(&m, 0), tele);
        assert_eq!(ring_start(&m, 1), tele + 0.6); // p.rings = [0, 0.6]
        assert_eq!(ring_radius(&hz, &m, 0, tele), f64::from(hz.ring_r0));
        let speed = f64::from(hz.ring_speed);
        assert!(
            (ring_radius(&hz, &m, 0, tele + 1.0) - (f64::from(hz.ring_r0) + speed)).abs() < 1e-9
        );
        let u = tele + 0.37;
        assert!(
            (ring_radius(&hz, &m, 0, u + 0.13) - ring_radius(&hz, &m, 0, u) - speed * 0.13).abs()
                < 1e-9
        );
    }

    #[test]
    fn beam_angle_endpoints_and_smoothstep_monotonic() {
        // JS bossHazards.js L61: a0 at u = tele, a1 at tele + act, smoothstep
        // in between, clamped outside.
        let (a0, a1) = (-1.0f64, 1.0);
        let m = sweep_rec(0.0, 0.0, 0.0, 0.0, 0.0, a0, a1, 1.0, 2.0);
        assert_eq!(beam_angle(&m, 1.0), a0);
        assert_eq!(beam_angle(&m, 3.0), a1);
        assert_eq!(beam_angle(&m, 0.5), a0); // k clamps to 0
        assert_eq!(beam_angle(&m, 4.0), a1); // k clamps to 1
        assert!(beam_angle(&m, 2.0).abs() < 1e-12); // smoothstep(0.5) = 0.5
        let mut prev = beam_angle(&m, 1.0);
        for i in 1..=20 {
            let cur = beam_angle(&m, 1.0 + 2.0 * f64::from(i) / 20.0);
            assert!(cur >= prev, "sweep angle must be monotonic");
            prev = cur;
        }
    }

    #[test]
    fn charge_time_round_trips_through_charge_dist() {
        // JS bossHazards.js L48-55: chargeTime is when chargeDist reaches L.
        let hz = crate::embedded_tuning().boss.hz;
        for ph in 1..=3usize {
            let v = f64::from(hz.charge_speed[ph]);
            let s0 = 0.5 * v * f64::from(hz.charge_ramp);
            for l in [3.0, s0, s0 + 10.0, 34.0] {
                let t = charge_time(&hz, v, l);
                let back = charge_dist(&hz, v, l, t + 1.0, 1.0);
                assert!((back - l).abs() < 1e-9, "ph{ph} L{l}: reached {back}");
            }
        }
    }

    #[test]
    fn reach_prunes_at_walls_on_tidewater() {
        // JS bossHazards.js L202-213: the reach walk stops at `kindAt !== 0`.
        with_squad(true, 7, |boss, _kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let r = f64::from(boss.tb.hz.ring_max);
            let m = slam_rec(0.0, [hx, hy, hz]);
            let reach = boss.hz_reach(&m, hx, hz, r);
            assert_eq!(reach.len(), boss.tb.reach_n as usize); // JS L36 REACH_N
            assert!(reach.iter().all(|&v| v >= 1.2 && f64::from(v) <= r + 1e-6));
            // Measured on Tidewater: some headings run the full ringMax,
            // others stop early at geometry.
            assert!(
                reach.iter().any(|&v| f64::from(v) >= r - 0.35),
                "no open heading"
            );
            assert!(reach.iter().any(|&v| f64::from(v) < r), "no pruned heading");
            // reachAt picks the same bucket as the array (JS L214-217).
            for k in [0usize, 13, 48, 95] {
                let a = (k as f64 + 0.5) / f64::from(boss.tb.reach_n) * TAU;
                let px = hx + a.sin() * 5.0;
                let pz = hz + a.cos() * 5.0;
                assert_eq!(
                    f64::from(boss.hz_reach_at(&m, hx, hz, px, pz, r)),
                    f64::from(reach[k])
                );
            }
        });
    }

    #[test]
    fn beam_len_truncates_at_walls_and_reaches_full_when_open() {
        // JS bossHazards.js L220-230: horizontal raycast, capped at beamLen.
        with_squad(true, 7, |boss, _kit| {
            let beam = f64::from(boss.tb.hz.beam_len);
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let mut candidates: Vec<(f64, f64, f64)> = vec![(hx, hy, hz)];
            for i in boss.nav.plan_ids() {
                let (x, z) = boss.nav.xz(*i);
                candidates.push((
                    f64::from(x),
                    f64::from(boss.nav.floor_at(x, z)),
                    f64::from(z),
                ));
            }
            for (i, (ox, oy, oz)) in candidates.iter().enumerate() {
                let m = sweep_rec(i as f64, *ox, *oy, *oz, *oy, 0.0, 1.0, 0.5, 1.0);
                let mut full = false;
                let mut cut = false;
                for k in 0..96 {
                    let a = (f64::from(k) + 0.5) / 96.0 * TAU;
                    let l = f64::from(boss.hz_beam_len(&m, a));
                    assert!(l <= beam + 1e-3, "beam {l} beyond HZ.beamLen {beam}");
                    if l >= beam - 1e-3 {
                        full = true;
                    }
                    if l < beam - 0.5 {
                        cut = true;
                    }
                }
                if full && cut {
                    return; // both behaviours measured on Tidewater
                }
            }
            panic!("no Tidewater emitter shows both a truncated and a full-length beam");
        });
    }

    // -- damage semantics (JS bossHazards.js L119-194) -----------------------

    #[test]
    fn slam_impact_hits_once_per_actor() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            kit.add(hx as f32, hy as f32, hz as f32, 100);
            boss.hz_add(slam_rec(0.0, [hx, hy, hz]));
            let mul = f64::from(boss.dmg_mul);
            let want = f64::from(boss.tb.hz.slam_dmg) * mul;
            let hp0 = f64::from(kit.actors[0].hp);
            // JS L132: the impact fires on the tele crossing; JS L127-128:
            // the once key stops repeats.
            boss.hz_hurt_local(0.5, 1.2, 1.0 / 60.0, &mut kit.squad());
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - want).abs() < 1e-3,
                "impact damage"
            );
            boss.hz_hurt_local(0.5, 1.2, 1.0 / 60.0, &mut kit.squad());
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - want).abs() < 1e-3,
                "impact must not repeat"
            );
        });
    }

    #[test]
    fn slam_ring_band_sweeps_at_low_framerate() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let r = f64::from(boss.tb.hz.ring_max);
            let m = slam_rec(0.0, [hx, hy, hz]);
            boss.hz_add(m.clone());
            // An open heading so the reach clip (JS L137) can't hide the band.
            let reach = boss.hz_reach(&m, hx, hz, r);
            let k = (0..reach.len())
                .find(|&i| f64::from(reach[i]) >= 9.0)
                .expect("open heading");
            let a = (k as f64 + 0.5) / f64::from(boss.tb.reach_n) * TAU;
            let d = 8.0f64;
            kit.add(
                (hx + a.sin() * d) as f32,
                hy as f32,
                (hz + a.cos() * d) as f32,
                100,
            );
            // One 10 fps frame crossing r 2.4 → 11.9: the band test
            // (JS L135-140) still catches the kid at d = 8.
            boss.hz_hurt_local(1.0, 2.0, 0.1, &mut kit.squad());
            let mul = f64::from(boss.dmg_mul);
            let want = f64::from(boss.tb.hz.ring_dmg) * mul;
            assert!(
                (100.0 - f64::from(kit.actors[0].hp) - want).abs() < 1e-2,
                "ring band hit"
            );
        });
    }

    #[test]
    fn safe_pad_actors_are_immune() {
        with_squad(true, 7, |boss, kit| {
            // JS L123: kids inside `inPad(x, z, SAFE_PAD)` take nothing.
            let (px, pz) = boss.nav.pad_center(0);
            let py = f64::from(boss.nav.floor_at(px, pz));
            kit.add(px, py as f32, pz, 100);
            boss.hz_add(slam_rec(0.0, [f64::from(px), py, f64::from(pz)]));
            let hp0 = f64::from(kit.actors[0].hp);
            boss.hz_hurt_local(0.5, 1.2, 1.0 / 60.0, &mut kit.squad());
            boss.hz_hurt_local(1.0, 2.0, 0.1, &mut kit.squad());
            assert_eq!(
                f64::from(kit.actors[0].hp),
                hp0,
                "spawn-pad kid took damage"
            );
        });
    }

    #[test]
    fn submerged_kids_duck_the_sweep_beam() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let a = open_beam_heading(boss, hx, hy, hz);
            boss.hz_add(sweep_rec(0.0, hx, hy + 2.3, hz, hy, a, a, 0.5, 1.0));
            let (ax, az) = (hx + a.sin() * 6.0, hz + a.cos() * 6.0);
            kit.add(ax as f32, hy as f32, az as f32, 100);
            kit.actors[0].submerged = true;
            let hp0 = f64::from(kit.actors[0].hp);
            boss.hz_hurt_local(0.8, 0.9, 1.0 / 60.0, &mut kit.squad());
            assert_eq!(
                f64::from(kit.actors[0].hp),
                hp0,
                "JS L151: submerged ducks the beam"
            );
            kit.actors[0].submerged = false;
            boss.hz_hurt_local(0.9, 1.0, 1.0 / 60.0, &mut kit.squad());
            assert!(
                f64::from(kit.actors[0].hp) < hp0,
                "the same kid must burn out of the water"
            );
        });
    }

    #[test]
    fn sweep_beam_damage_is_dps_times_dt() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let a = open_beam_heading(boss, hx, hy, hz);
            boss.hz_add(sweep_rec(0.0, hx, hy + 2.3, hz, hy, a, a, 0.5, 1.0));
            let (ax, az) = (hx + a.sin() * 6.0, hz + a.cos() * 6.0);
            kit.add(ax as f32, hy as f32, az as f32, 100);
            let dps = f64::from(boss.tb.hz.beam_dps) * f64::from(boss.dmg_mul);
            let hp0 = f64::from(kit.actors[0].hp);
            boss.hz_hurt_local(0.75, 0.85, 0.1, &mut kit.squad());
            let d1 = hp0 - f64::from(kit.actors[0].hp);
            assert!((d1 - dps * 0.1).abs() < 1e-3, "JS L157: beamDps·mul·dt");
            let hp1 = f64::from(kit.actors[0].hp);
            boss.hz_hurt_local(0.85, 1.05, 0.2, &mut kit.squad());
            let d2 = hp1 - f64::from(kit.actors[0].hp);
            assert!((d2 - dps * 0.2).abs() < 1e-3, "double dt doubles the burn");
            assert!(
                (d2 - 2.0 * d1).abs() < 1e-3,
                "continuous, no once key on the beam"
            );
        });
    }

    #[test]
    fn remote_actors_are_never_hurt() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let a = open_beam_heading(boss, hx, hy, hz);
            boss.hz_add(sweep_rec(0.0, hx, hy + 2.3, hz, hy, a, a, 0.5, 1.0));
            // JS L120: `remote[i]` actors skip every local hurt branch.
            kit.add_r(
                (hx + a.sin() * 6.0) as f32,
                hy as f32,
                (hz + a.cos() * 6.0) as f32,
                100,
                true,
            );
            kit.add(
                (hx + a.sin() * 5.0) as f32,
                hy as f32,
                (hz + a.cos() * 5.0) as f32,
                101,
            );
            let hp_r0 = f64::from(kit.actors[0].hp);
            let hp_l0 = f64::from(kit.actors[1].hp);
            boss.hz_hurt_local(0.8, 0.9, 1.0 / 60.0, &mut kit.squad());
            assert_eq!(
                f64::from(kit.actors[0].hp),
                hp_r0,
                "remote kid must be untouched"
            );
            assert!(
                f64::from(kit.actors[1].hp) < hp_l0,
                "local control kid must burn"
            );
        });
    }

    #[test]
    fn sweep_beam_stops_where_the_wall_blocks_the_eye_line() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let probe = sweep_rec(-5.0, hx, hy + 2.3, hz, hy, 0.0, 0.0, 0.5, 1.0);
            // JS L156: damage needs `los(emitter eye → kid chest)`. Scan the
            // ring for a heading the eye-line can't cross; prefer one the
            // beam itself would reach (isolating the LOS branch).
            let mut isolated: Option<(f64, f64, f64, f64)> = None;
            let mut blocked: Option<(f64, f64, f64, f64)> = None;
            'scan: for ri in 0..6 {
                let d = 4.0 + f64::from(ri);
                for k in 0..64 {
                    let a = f64::from(k) / 64.0 * TAU;
                    let (ax, az) = (hx + a.sin() * d, hz + a.cos() * d);
                    let ay = f64::from(boss.world.collision.ground_height(
                        ax as f32,
                        az as f32,
                        (hy + 2.0) as f32,
                        false,
                    ));
                    if !ay.is_finite() || (ay - hy).abs() > 1.5 {
                        continue;
                    }
                    if boss.world.collision.los(
                        vec3(hx as f32, (hy + 2.3) as f32, hz as f32),
                        vec3(ax as f32, (ay + 0.9) as f32, az as f32),
                    ) {
                        continue;
                    }
                    let ang = (ax - hx).atan2(az - hz);
                    let len = f64::from(boss.hz_beam_len(&probe, ang));
                    blocked = Some((ax, ay, az, ang));
                    if d > 1.5 && d < len + 0.4 {
                        isolated = Some((ax, ay, az, ang));
                        break 'scan;
                    }
                }
            }
            let (ax, ay, az, ang) = blocked.expect("Tidewater: nothing blocks the home eye-line");
            let m = sweep_rec(0.0, hx, hy + 2.3, hz, hy, ang, ang, 0.5, 1.0);
            if isolated.is_some() {
                // The lane test (JS L153-154) would pass: only LOS stops it.
                let len = f64::from(boss.hz_beam_len(&m, ang));
                let d = ((ax - hx).powi(2) + (az - hz).powi(2)).sqrt();
                assert!(
                    d < len + 0.4 && d > 1.5,
                    "isolated LOS case must be in the lane"
                );
            }
            boss.hz_add(m);
            kit.add(ax as f32, ay as f32, az as f32, 100);
            let hp0 = f64::from(kit.actors[0].hp);
            boss.hz_hurt_local(0.8, 0.9, 1.0 / 60.0, &mut kit.squad());
            assert_eq!(
                f64::from(kit.actors[0].hp),
                hp0,
                "JS L156: no damage behind the wall"
            );
        });
    }

    #[test]
    fn charge_ram_hits_once_and_bowls_the_kid_over() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let hz_t = boss.tb.hz;
            let v = f64::from(hz_t.charge_speed[1]);
            let l = 10.0f64;
            let tele = 1.15f64;
            let act = charge_time(&hz_t, v, l);
            boss.hz_add(charge_rec(0.0, hx, hy, hz, 0.0, l, v, tele, act));
            // JS L162-167: a WALL_BODY circle at the bow (yaw 0 → +z lane).
            let c0 = boss.tb.nav.wall_body[0];
            kit.add(hx as f32, hy as f32, (hz + l + f64::from(c0.z)) as f32, 100);
            let hp0 = f64::from(kit.actors[0].hp);
            let u1 = tele + act;
            boss.hz_hurt_local((u1 - 0.05) as f32, u1 as f32, 1.0 / 60.0, &mut kit.squad());
            let mul = f64::from(boss.dmg_mul);
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - f64::from(hz_t.charge_dmg) * mul).abs() < 1e-3,
                "ram damage"
            );
            assert_eq!(kit.actors[0].vel.y, 7.0); // JS L169: knocked up
            assert!(!kit.actors[0].grounded); // JS L171
            boss.hz_hurt_local((u1 - 0.05) as f32, u1 as f32, 1.0 / 60.0, &mut kit.squad());
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - f64::from(hz_t.charge_dmg) * mul).abs() < 1e-3,
                "ram once"
            );
        });
    }

    // -- threat() per move (JS bossHazards.js L294-349) ----------------------

    #[test]
    fn threat_slam_inside_and_outside_the_circle() {
        with_squad(true, 7, |boss, _kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let r = f64::from(boss.tb.hz.ring_max);
            let m = slam_rec(0.0, [hx, hy, hz]);
            boss.hz_add(m.clone());
            boss.bt = 0.5f32; // u = 0.5 < tele
            // JS L308-313: inside the wind-up circle — full push, ring ETA.
            let t = boss.threat(hx, hy, hz + 2.0, 1.2);
            assert!(t.level > 0.0, "inside the slam circle");
            assert!(t.ring_in >= 0.0, "ring ETA: {t:?}");
            // Outside the circle but within reach: the ring is still more than
            // the horizon away → tIn > 1.2 → ringIn stays −1 (JS L312).
            let reach = boss.hz_reach(&m, hx, hz, r);
            let k = (0..reach.len())
                .find(|&i| f64::from(reach[i]) >= 12.6)
                .expect("open heading");
            let a = (k as f64 + 0.5) / f64::from(boss.tb.reach_n) * TAU;
            let t2 = boss.threat(hx + a.sin() * 12.0, hy, hz + a.cos() * 12.0, 1.2);
            assert_eq!(t2.level, 0.0);
            assert_eq!(t2.ring_in, -1.0);
        });
    }

    #[test]
    fn threat_charge_reports_the_lane_and_escapes_across_it() {
        with_squad(true, 7, |boss, _kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let hz_t = boss.tb.hz;
            let v = f64::from(hz_t.charge_speed[1]);
            let l = 10.0f64;
            let tele = 1.15f64;
            let act = charge_time(&hz_t, v, l);
            boss.hz_add(charge_rec(0.0, hx, hy, hz, 0.0, l, v, tele, act));
            boss.bt = (tele + act) as f32;
            // JS L338: |lat| < 3.6 on the axis; the escape push is
            // (sz·sgn(lat), −sx·sgn(lat)) → +x for a kid east of a +z lane.
            let t = boss.threat(hx + 2.0, hy, hz + 10.0, 1.2);
            assert!(t.lane, "kid on the charge lane");
            assert!(
                t.ax > 0.99 && t.az.abs() < 0.01,
                "escape across the lane: {t:?}"
            );
            let t2 = boss.threat(hx + 5.0, hy, hz + 10.0, 1.2);
            assert!(!t2.lane, "off-lane kid needs no lane warning");
        });
    }

    #[test]
    fn threat_frenzy_inside_the_spin_circle_covers() {
        with_squad(true, 7, |boss, _kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            boss.hz_add(frenzy_rec(0.0, hx, hy, hz));
            boss.bt = 2.0f32; // u = 2.0 < tele + act = 4.2
            // JS L341-343: inside `frenzyR + 2` during the spin → cover.
            let t = boss.threat(hx, hy, hz + f64::from(boss.tb.hz.frenzy_r) - 0.5, 1.2);
            assert!(t.cover);
            assert!(t.level > 0.0);
            let t2 = boss.threat(hx, hy, hz + f64::from(boss.tb.hz.frenzy_r) + 3.0, 1.2);
            assert!(!t2.cover);
        });
    }

    // -- TR-43.2: host/guest parity and paint --------------------------------

    #[test]
    fn host_and_guest_agree_on_current_move_and_geometry() {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tb = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tb.boss.nav);
        let mut host = Boss::new(
            &world,
            &nav,
            &tb.boss,
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: true,
                seed: 7,
            },
        );
        let mut guest = Boss::new(
            &world,
            &nav,
            &tb.boss,
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: false,
                seed: 7,
            },
        );
        let (hx, hy, hz) = (
            f64::from(host.pos.x),
            f64::from(host.pos.y),
            f64::from(host.pos.z),
        );
        let rec = slam_rec(0.0, [hx, hy, hz]);
        host.hz_add(rec.clone());
        guest.on_move(rec.clone()); // JS boss.js L444-452: the guest receives the record
        let r = f64::from(tb.boss.hz.ring_max);
        let reach = host.hz_reach(&rec, hx, hz, r);
        let k = (0..reach.len())
            .find(|&i| f64::from(reach[i]) >= 9.0)
            .expect("open heading");
        let a = (k as f64 + 0.5) / f64::from(tb.boss.reach_n) * TAU;
        let pos = vec3(
            (hx + a.sin() * 8.0) as f32,
            hy as f32,
            (hz + a.cos() * 8.0) as f32,
        );
        let mut hk = Kit::new();
        let mut gk = Kit::new();
        hk.add(pos.x, pos.y, pos.z, 100);
        gk.add(pos.x, pos.y, pos.z, 100);
        let hp_h0 = f64::from(hk.actors[0].hp);
        let hp_g0 = f64::from(gk.actors[0].hp);
        let dt = 1.0f32 / 60.0;
        let mut bt = 0.0f32;
        for _ in 0..240 {
            bt += dt;
            host.bt = bt;
            guest.bt = bt;
            host.hz_update(dt, bt, true, &mut hk.squad()); // JS L107 live
            guest.hz_update(dt, bt, false, &mut gk.squad()); // guest: live=false
            let hm = host.hz_current(bt);
            let gm = guest.hz_current(bt);
            assert_eq!(hm.cloned(), gm.cloned(), "current move diverged at bt {bt}");
            if let Some(m) = hm {
                let u = f64::from(bt) - rec_t0(m);
                let gmm = gm.unwrap();
                for kk in 0..rec_rings(m).len() {
                    assert_eq!(
                        ring_radius(&tb.boss.hz, m, kk, u),
                        ring_radius(&tb.boss.hz, gmm, kk, u)
                    );
                }
            }
        }
        assert!(
            f64::from(hk.actors[0].hp) < hp_h0,
            "host-side kid must take ring damage"
        );
        assert_eq!(
            f64::from(gk.actors[0].hp),
            hp_g0,
            "guest must not apply damage (live=false)"
        );
        assert!(
            !guest
                .drain_events()
                .iter()
                .any(|e| matches!(e, BossEvent::Splat { .. })),
            "guest produced a Splat"
        );
    }

    #[test]
    fn paint_is_host_only_and_team1() {
        with_squad(true, 7, |boss, kit| {
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            boss.hz_add(slam_rec(0.0, [hx, hy, hz]));
            // JS bossHazards.js L233-289: the host paints across the tele beat.
            boss.hz_paint(0.5, 1.2, &kit.player);
            let ev = boss.drain_events();
            let splats: Vec<_> = ev
                .iter()
                .filter(|e| matches!(e, BossEvent::Splat { .. }))
                .collect();
            assert!(!splats.is_empty(), "host slam paint");
            for s in &splats {
                if let BossEvent::Splat { team, .. } = s {
                    assert_eq!(*team, 1, "boss paint is team 1");
                }
            }
        });
    }

    #[test]
    fn paint_rejects_spawn_pads() {
        with_squad(true, 7, |boss, kit| {
            // JS L237: `inPad(x, z, 0.3)` kills the splat. Tidewater pads have
            // r = spawnBarrier 4.2, so the mark + all 7 satellites are rejected.
            let (px, pz) = boss.nav.pad_center(0);
            let py = f64::from(boss.nav.floor_at(px, pz));
            boss.hz_add(slam_rec_no_rings(0.0, [f64::from(px), py, f64::from(pz)]));
            boss.hz_paint(0.5, 1.2, &kit.player);
            assert_eq!(splat_count(boss), 0, "pad centre must stay clean");
        });
    }

    #[test]
    fn paint_rejects_points_at_or_below_water() {
        with_squad(true, 7, |boss, kit| {
            // JS L239: no ground hit (open sea) → no splat.
            let bx = f64::from(boss.world.collision.bounds.max_x);
            let (hx, hy, hz) = (
                f64::from(boss.pos.x),
                f64::from(boss.pos.y),
                f64::from(boss.pos.z),
            );
            let control = slam_rec_no_rings(0.0, [hx, hy, hz]);
            boss.hz_add(control);
            boss.hz_paint(0.5, 1.2, &kit.player);
            assert!(splat_count(boss) > 0, "control paint on solid ground");
            let sea = slam_rec_no_rings(10.0, [bx + 5.0, 0.0, 0.0]);
            boss.hz_add(sea);
            boss.hz_paint(10.5, 11.2, &kit.player);
            assert_eq!(splat_count(boss), 0, "open sea must stay clean");
        });
    }
}
