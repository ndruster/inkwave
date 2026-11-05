//! Task 36 — Zone Control rules engine (verbatim port of `src/game/zones.js`).
//!
//! A match with [`MatchMode::Zones`] owns one [`ZoneControl`]
//! (JS `match.zones`, match.js L96-99 / L128-131). The host runs the rules
//! and records every decision on its event timeline; guests are followers
//! that replay them and only predict the count between snapshots
//! (JS zones.js L1-5).
//!
//! Mapping (line numbers are the JS source):
//!   - constants L47-53            -> `WARN_GAP`/`MIN_STINT`/`FILL_T`/`WIPE_T`
//!     + `zt.final_centre`/`zt.warn` (the `??` fallbacks are not ported:
//!       tuning always carries the values)
//!   - `mirrorZone` L56            -> [`mirror_def`] (`partsOf` L55 is folded
//!     into `ZoneDef::parts`, geometry normalisation)
//!   - `insidePoly` L58-65         -> [`inside_poly`]
//!   - `placeholderZones` L68-73   -> [`placeholder_zones`] (JS `void B` L72:
//!     `level.bounds` is unused, only `spawnPads[0]` is needed)
//!   - `zoneCells` L77-103         -> [`zone_cells`] (JS `f.atlas` ->
//!     [`Face::paintable`](crate::collision::Face::paintable))
//!   - ctor L106-140               -> [`ZoneControl::new`]
//!   - `follower` L143 / `_net` L144 -> [`ZoneControl::follower`] +
//!     [`ZoneControl::net_out`] drained by [`ZoneControl::drain_net`] (the
//!     batch-3 network layer records onto the timeline, JS `G.netm.recZone`)
//!   - `total`/`losing`/`_swapDelay`/`_schedule` L148-157 -> [`ZoneControl::total`]
//!     / [`ZoneControl::losing`] / [`ZoneControl::swap_delay`] / [`ZoneControl::schedule`]
//!   - `_flood` L161-167           -> [`flood_zone`] (free fn; the JS
//!     `z.flood` presentation field is not stored — spec §7)
//!   - `_sample` L170-190          -> [`ZoneControl::_sample`]
//!   - `_zoneOwner` L192-197       -> [`ZoneControl::_zone_owner`]
//!   - `_contest` L200-206         -> [`contest`] (free fn; returns the warn event)
//!   - `_setOwner` L208-223        -> [`ZoneControl::_set_owner`] (`['zp']`
//!     lands before `['zo']` like JS L215/L221)
//!   - `_penalise` L228-236        -> [`ZoneControl::_penalise`]
//!   - `_tick` L238-249            -> [`ZoneControl::_tick`]
//!   - `_fillSpecials` L252-263    -> [`ZoneControl::_fill_specials`] (JS
//!     `a.remote` L258 is the [`Match::step_masked`](crate::match_::Match::step_masked)
//!     ownership mask; the `special:ready` edge rides the actor event bus)
//!   - `update` L266-289           -> [`ZoneControl::update`] (JS `return
//!     this._end(...)` surfaces as `Some((winner, reason))`; the caller runs
//!     [`Match::end_zones`](crate::match_::Match::end_zones))
//!   - `_swap` L291-303            -> [`ZoneControl::_swap`]
//!   - `_finalLock` L306-312       -> [`ZoneControl::_final_lock`]
//!   - `timeUp` L315-333           -> [`ZoneControl::time_up`] (`Some` = end
//!     now, `None` = overtime began — JS true/false)
//!   - `_overtime` L338-355        -> [`ZoneControl::_overtime`]
//!   - `_tiebreak` L357-361        -> [`ZoneControl::_tiebreak`] (`Math.random`
//!     -> the seeded [`Rng`] stream, sim determinism)
//!   - `_end` L363-369             -> [`ZoneControl::_end`]
//!   - `netEvent` L372-413         -> [`ZoneControl::net_event`]
//!   - `state` L416-424            -> [`ZoneControl::state`]
//!   - `inZone` (match.js L286-297) -> [`in_zone`]
//!
//! Events (JS `emit('zones:*')`, zones.js L31-36) land on
//! [`ZoneControl::events_out`]; `Match` drains them into
//! [`MatchEvent::Zone`](crate::match_::MatchEvent::Zone). The HUD does not
//! consume them yet (batch 3+).

use serde::{Deserialize, Serialize};

use crate::actor::Actor;
use crate::actor::ActorEvent;
use crate::actor::FIXED_DT;
use crate::collision::CollisionWorld;
use crate::geometry::{ZoneDef, ZoneLayout};
use crate::net::wire::{JVal, dec, js_round, r3};
use crate::paint::{FloodEase, PaintGrid};
use crate::tuning::ZonesTuning;
use crate::weapon::Rng;
use glam::Vec3;

#[cfg(test)]
mod tests;

/// JS L47-48: `FINAL = ZONES.finalCentre ?? 30` and `WARN = ZONES.warn ?? 0.3`
/// — the `??` fallback branches are not ported (tuning always carries the
/// values); they read `zt.final_centre` / `zt.warn` at every use site.
/// JS L49: s between two warnings for one zone.
const WARN_GAP: f32 = 4.0;
/// JS L50: a rotation never leaves an objective live for less than this
/// before the lock.
const MIN_STINT: f32 = 10.0;
/// JS L51: capture flood — the front's run time (s), eased out.
const FILL_T: f32 = 0.6;
/// JS L53: new-objective wipe — in step with ZoneMarks' reveal sweep (linear).
const WIPE_T: f32 = 0.7;

/// JS L52: `r3` over an f32 sim value, through the shortest-decimal `dec`
/// re-interpretation so the wire bytes match JS exactly.
fn jf(v: f32) -> JVal {
    JVal::Num(r3(dec(v)))
}

/// An integer wire slot (JS pushes plain integers; `Num` keeps them
/// fraction-free through [`crate::net::wire::num`]).
fn jn(v: i64) -> JVal {
    JVal::Num(v as f64)
}

/// JS `+x.toFixed(1)` (zones.js L421) — round to one decimal.
fn r1(x: f32) -> f64 {
    js_round(dec(x) * 10.0) / 10.0
}

/// JS `insidePoly` (L58-65): crossing-number test on the XZ plane.
#[must_use]
pub fn inside_poly(poly: &[[f32; 2]], x: f32, z: f32) -> bool {
    let mut inside = false;
    let n = poly.len();
    // JS `j = poly.length - 1`: an empty outline never enters the loop
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let [xi, zi] = poly[i];
        let [xj, zj] = poly[j];
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// JS `inZone` (match.js L286-297): a point on (or just above) a zone —
/// inside one outline, `y0 - 1` … `y1 + 2.5` (`??` defaults not ported).
#[must_use]
pub fn in_zone(def: &ZoneDef, p: Vec3) -> bool {
    let y0 = def.y0.unwrap_or(-2.0);
    let y1 = def.y1.unwrap_or(6.0);
    if p.y < y0 - 1.0 || p.y > y1 + 2.5 {
        return false;
    }
    def.parts().iter().any(|poly| inside_poly(poly, p.x, p.z))
}

/// JS `mirrorZone` (L56): the 180° mirror `[-x, -z]` of every outline part
/// (`partsOf` L55 is already normalised into [`ZoneDef::parts`]).
fn mirror_def(z: &ZoneDef) -> ZoneDef {
    ZoneDef::new(
        z.parts()
            .iter()
            .map(|p| p.iter().map(|[x, zz]| [-*x, -*zz]).collect())
            .collect(),
        z.y0,
        z.y1,
    )
}

/// JS `placeholderZones` (L68-73): a 10 × 10 m centre at mid, side zones a
/// third of the way to each spawn. `level.bounds` is unused in JS (`void B`,
/// L72) so only `spawnPads[0]` is needed.
fn placeholder_zones(spawn_pad0: Vec3) -> (Vec<ZoneDef>, ZoneDef) {
    let sq = |cx: f32, cz: f32, h: f32| {
        vec![
            [cx - h, cz - h],
            [cx + h, cz - h],
            [cx + h, cz + h],
            [cx - h, cz + h],
        ]
    };
    (
        vec![ZoneDef::new(vec![sq(0.0, 0.0, 5.0)], Some(-2.0), Some(6.0))],
        ZoneDef::new(
            vec![sq(spawn_pad0.x * 0.4, spawn_pad0.z * 0.45, 4.0)],
            Some(-2.0),
            Some(6.0),
        ),
    )
}

/// JS `zoneCells` (L77-103): the paint-grid cells of a zone — every live
/// turf cell whose centre lies inside the outline and within `[y0, y1]` —
/// plus the flood centre / reach. Cell centres are the exact face-plane
/// points (JS lifts nothing here, unlike `_initGrid`'s `+0.06`); the global
/// `u32` cell index matches [`PaintGrid::flood`]'s region cells (batch 1).
/// Returns `(cells, center, reach)`.
fn zone_cells(
    def: &ZoneDef,
    paint: &PaintGrid,
    world: &CollisionWorld,
) -> (Vec<u32>, [f32; 3], f32) {
    // JS L80: `zone.y0 ?? -2` / `zone.y1 ?? 6`
    let y0 = def.y0.unwrap_or(-2.0);
    let y1 = def.y1.unwrap_or(6.0);
    let parts = def.parts();
    // JS L82-83: outline bounding box
    let (mut x0, mut x1, mut z0, mut z1) = (
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    );
    for p in parts {
        for &[x, z] in p {
            x0 = x0.min(x);
            x1 = x1.max(x);
            z0 = z0.min(z);
            z1 = z1.max(z);
        }
    }
    let mut cells: Vec<u32> = Vec::new();
    let (mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32);
    for f in &world.faces {
        // JS L85: `!f.turf || !f.atlas` skip (`atlas` -> `paintable`)
        if !f.turf || !f.paintable {
            continue;
        }
        let Some((nu, nv, cu, cv, off)) = paint.face_meta(f.id as usize) else {
            continue;
        };
        // JS L86-94
        for j in 0..nv {
            for i in 0..nu {
                let s = (i as f32 + 0.5) * cu;
                let t = (j as f32 + 0.5) * cv;
                let wx = f.origin.x + f.u.x * s + f.v.x * t;
                let wy = f.origin.y + f.u.y * s + f.v.y * t;
                let wz = f.origin.z + f.u.z * s + f.v.z * t;
                if wx < x0 || wx > x1 || wz < z0 || wz > z1 || wy < y0 || wy > y1 {
                    continue;
                }
                if !parts.iter().any(|p| inside_poly(p, wx, wz)) {
                    continue;
                }
                // JS L91: `f.grid + j * f.nu + i`
                let k = off + (j as usize) * (nu as usize) + i as usize;
                // JS L92: `P.dead && P.dead[k]`
                if paint.cell_dead(k) {
                    continue;
                }
                cells.push(k as u32);
                cx += wx;
                cy += wy;
                cz += wz;
            }
        }
    }
    // JS L96-97: mean centre (`Math.max(1, cells.length)`)
    let n = cells.len().max(1) as f32;
    let center = [cx / n, cy / n, cz / n];
    // JS L98-100: reach — the farthest outline corner from the centre (XZ)
    let mut reach = 0.0f32;
    for p in parts {
        for &[x, z] in p {
            reach = reach.max((x - center[0]).hypot(z - center[2]));
        }
    }
    (cells, center, reach)
}

/// Match mode selector (JS `match.mode`, match.js L20; the practice /
/// attract modes stay out of scope for the sim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MatchMode {
    /// Turf War (the default every existing call site runs).
    #[default]
    Turf,
    /// Zone Control (this module).
    Zones,
    /// Boss Battle (Task 44; JS `match.mode === 'boss'`, match.js L20 — one
    /// squad on team 0 against HULLBREAKER, `bossMode` driven by the caller
    /// through [`crate::boss::BossMode`]).
    Boss,
}

/// JS `d.kind` (L112-113): `'center'` | `'side'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    Center,
    Side,
}

/// JS `this.objectives[i]` (L117-121): one operational group of zones.
#[derive(Debug, Clone)]
pub struct ObjectiveState {
    /// JS `id`: `'center'` / `'sideA'` / `'sideB'`.
    pub name: &'static str,
    /// Indices into [`ZoneControl::zones`] (JS holds the zone objects).
    pub zones: Vec<usize>,
    /// JS `home`: -1 centre, 0 Alpha's side, 1 Bravo's side.
    pub home: i8,
}

/// JS `this.zones[i]` (L114): one zone with its cell set and live marks.
#[derive(Debug, Clone)]
pub struct ZoneState {
    pub id: usize,
    pub kind: ZoneKind,
    pub home: i8,
    /// Zone holder: -1 nobody, 0/1 team (JS `owner`).
    pub owner: i8,
    /// Ink share of each team over the zone's live turf cells.
    pub share: [f32; 2],
    /// JS `hold`: wipe gate — sampled shares are ignored until `clock >= hold`.
    pub hold: f32,
    pub warn_armed: bool,
    pub warn_t: f32,
    /// Global paint-grid cell ids (JS `cells`, the `Int32Array`).
    pub cells: Vec<u32>,
    /// Cell-mean centre (JS `center` `[x, y, z]`).
    pub center: [f32; 3],
    /// Farthest outline corner distance in XZ (JS `reach`).
    pub reach: f32,
    pub def: ZoneDef,
}

/// JS `emit('zones:*')` payloads (L31-36), drained by `Match` into
/// [`MatchEvent::Zone`](crate::match_::MatchEvent::Zone).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ZoneEvent {
    /// JS `zones:zone` (L196).
    Zone { zone: usize, owner: i8 },
    /// JS `zones:control` (L222 / L300 / L383).
    Control {
        owner: i8,
        prev: i8,
        objective: String,
    },
    /// JS `zones:penalty` (L235 / L390).
    Penalty {
        team: usize,
        penalty: f32,
        total: f32,
        start: f32,
        end: f32,
    },
    /// JS `zones:active` (L302 / L311 / L397) — a rotation; `final_lock` is
    /// the last-30-s lock, `moved: false` when only the lock is announced.
    Active {
        objective: String,
        zones: Vec<usize>,
        final_lock: bool,
        moved: bool,
    },
    /// JS `zones:contest` (L205).
    Contest { zone: usize, holder: i8, share: f32 },
    /// JS `zones:overtime` (L321 / L328 / L400).
    Overtime { losing: i8 },
    /// JS `zones:end` (L367).
    End {
        winner: usize,
        reason: String,
        counts: [f32; 2],
        penalty: [f32; 2],
    },
}

/// JS `this.log` entry (L234 / L389): one penalty record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ZonePenaltyLog {
    /// Elapsed match time (`duration - time`, JS `t`).
    pub t: f32,
    pub team: usize,
    pub start: f32,
    pub end: f32,
    pub penalty: f32,
    pub next: f32,
}

/// JS zones-mode `_judge` result (match.js L254-257): the settlement line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneOutcome {
    pub winner: usize,
    pub reason: Option<String>,
    /// JS `counts`: `[Math.ceil(count0), Math.ceil(count1)]`.
    pub counts: [i64; 2],
    pub penalty: [f32; 2],
    pub overtime: bool,
    pub log: Vec<ZonePenaltyLog>,
}

/// JS `state()` per-zone entry (L420).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneSnapZone {
    pub id: usize,
    pub owner: i8,
    /// JS `+s.toFixed(3)` — three-decimal rounded shares.
    pub share: [f64; 2],
    pub cells: usize,
    pub center: [f32; 3],
}

/// JS `state()` snapshot (L416-424) for the HUD / results / tests.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneSnapshot {
    pub active: String,
    pub owner: i8,
    pub last_owner: i8,
    /// JS `count.map(Math.ceil)`.
    pub count: [i64; 2],
    pub penalty: [f32; 2],
    /// JS `[Math.ceil(total0), Math.ceil(total1)]`.
    pub total: [i64; 2],
    pub zones: Vec<ZoneSnapZone>,
    /// JS `final || !isFinite ? 0 : +nextSwap.toFixed(1)`.
    pub next_swap: f64,
    pub final_lock: bool,
    pub overtime: bool,
    pub overtime_t: f64,
    pub neutral_t: f64,
    pub winner: Option<usize>,
    pub reason: Option<String>,
    pub placeholder: bool,
}

/// JS `_flood` (L161-167) as a free function (borrow shape: the zone is
/// mutably borrowed while `Match`'s fields stay split). The JS `z.flood`
/// presentation field (the ZoneMarks front) is not stored — spec §7.
fn flood_zone(
    z: &mut ZoneState,
    team: i8,
    clock: f32,
    paint: &mut PaintGrid,
    world: &CollisionWorld,
) {
    // JS L162-163: fill = a real team; wipes (-1) run longer, wider, linear.
    let fill = team == 0 || team == 1;
    let dur = if fill { FILL_T } else { WIPE_T };
    let reach = if fill { z.reach + 0.4 } else { z.reach + 3.5 };
    let ease = if fill {
        FloodEase::Out
    } else {
        FloodEase::Linear
    };
    // JS L164 (z.flood) is presentation; L165: a wipe gates re-capture.
    if !fill {
        z.hold = clock + dur + 0.05;
    }
    // JS L166 `G.paint.flood(z.region, team, {center, …})` — the batch-1
    // `PaintGrid::flood` takes the flat cell list + XZ centre; `region_id`
    // is the zone id so a new flood replaces the running one (paint.js L721).
    paint.flood(
        world,
        &z.cells,
        (z.center[0], z.center[2]),
        if fill { team } else { -1 },
        reach,
        dur,
        ease,
        z.id as u32,
    );
}

/// JS `_contest` (L200-206) as a free function: warn once per held zone,
/// re-armed when the other team is pushed back below `WARN - 0.08`.
fn contest(z: &mut ZoneState, warn: f32, clock: f32) -> Option<ZoneEvent> {
    // JS L201 only ever runs on a held zone (`owner === -1` short-circuits at
    // L183/L178 before the call); callers guard this — pinned for debug builds.
    debug_assert!(z.owner >= 0, "contest on an unheld zone");
    let o = z.share[1 - z.owner as usize];
    if o < warn - 0.08 {
        z.warn_armed = true;
        return None;
    }
    if o < warn || !z.warn_armed || clock - z.warn_t < WARN_GAP {
        return None;
    }
    z.warn_armed = false;
    z.warn_t = clock;
    Some(ZoneEvent::Contest {
        zone: z.id,
        holder: z.owner,
        share: o,
    })
}

/// The Zone Control rules engine (JS `class ZoneControl`).
#[derive(Debug, Clone)]
pub struct ZoneControl {
    /// JS `this.zones` (L114): `[center…, sideA (home 0), sideB (home 1)]`.
    pub zones: Vec<ZoneState>,
    /// JS `this.objectives` (L117-121).
    pub objectives: [ObjectiveState; 3],
    /// JS `this.active` — index into [`ZoneControl::objectives`].
    pub active: usize,
    /// JS `owner` (L123): holder of the operational objective, -1 nobody.
    pub owner: i8,
    /// JS `lastOwner` (L124): survives neutral spells (penalty bookkeeping).
    pub last_owner: i8,
    /// JS `neutralT` (L125): time since the objective last went neutral.
    pub neutral_t: f32,
    /// JS `count` (L126): each team's countdown from `ZONES.count`.
    pub count: [f32; 2],
    /// JS `penalty` (L127): the counted-off lock (not part of the score).
    pub penalty: [f32; 2],
    /// JS `start` (L128): count + penalty when the team took the objective.
    start: [f32; 2],
    /// JS `tieEnd` (L129): the tie point caught during the stint.
    tie_end: [Option<f32>; 2],
    /// JS `holdT` (L130): current unbroken hold, s.
    pub hold_t: [f32; 2],
    /// JS `clock` (L131): s since the match started playing.
    pub clock: f32,
    /// JS `final` (L132): centre-only from here on. `final` is a Rust
    /// keyword — `final_lock` throughout.
    pub final_lock: bool,
    /// JS `nextSwap` (L156): s until the next rotation (`INFINITY` = none).
    pub next_swap: f32,
    sample_t: f32,
    snap_t: f32,
    /// JS `overtime` / `overtimeT` (L135).
    pub overtime: bool,
    pub overtime_t: f32,
    /// JS `otLosing` (L136): the team behind when overtime began (-1 sudden death).
    pub ot_losing: i8,
    /// JS `winner` / `reason` (L137).
    pub winner: Option<usize>,
    pub reason: Option<String>,
    /// JS `log` (L138).
    pub log: Vec<ZonePenaltyLog>,
    /// JS `placeholder` (L110): the stage had no zone layout.
    pub placeholder: bool,
    /// JS `get follower` (L143): `match.follower` — synced by `Match::step`
    /// before every update (the replication layer sets it after construction).
    pub follower: bool,
    /// JS `_net` (L144) `G.netm.recZone(e)`: the batch-3 network layer drains
    /// these records onto the event timeline.
    pub net_out: Vec<JVal>,
    /// JS `emit('zones:*')` — drained by `Match` into `MatchEvent::Zone`.
    pub events_out: Vec<ZoneEvent>,
    /// The rules' numbers (JS reads `ZONES` live; the sim carries the slice).
    zt: ZonesTuning,
}

impl ZoneControl {
    /// JS ctor (L106-140). `time` is `match.time` at construction (the clock
    /// still at full length); `spawn_pad0` is `level.spawnPads[0]` (the
    /// placeholder side-zone anchor, L69/L71).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        paint: &PaintGrid,
        world: &CollisionWorld,
        layout: Option<&ZoneLayout>,
        spawn_pad0: Vec3,
        zt: &ZonesTuning,
        time: f32,
        follower: bool,
        rng: &mut Rng,
    ) -> Self {
        // JS L108-109: `layout.zones || placeholderZones(level)`
        let (placeholder, center_defs, side_def) = match layout {
            Some(l) => (false, l.center.clone(), l.side.clone()),
            None => {
                let (c, s) = placeholder_zones(spawn_pad0);
                (true, c, s)
            }
        };
        // JS L112-113: `[center…, sideA (home 0), mirror sideB (home 1)]`
        let mut defs: Vec<(ZoneDef, ZoneKind, i8)> = Vec::new();
        for z in &center_defs {
            defs.push((z.clone(), ZoneKind::Center, -1));
        }
        defs.push((side_def.clone(), ZoneKind::Side, 0));
        defs.push((mirror_def(&side_def), ZoneKind::Side, 1));
        // JS L114: cells / centre / reach per zone
        let zones: Vec<ZoneState> = defs
            .into_iter()
            .enumerate()
            .map(|(i, (def, kind, home))| {
                let (cells, center, reach) = zone_cells(&def, paint, world);
                ZoneState {
                    id: i,
                    kind,
                    home,
                    owner: -1,
                    share: [0.0, 0.0],
                    hold: 0.0,
                    warn_armed: true,
                    warn_t: -99.0,
                    cells,
                    center,
                    reach,
                    def,
                }
            })
            .collect();
        // JS L115-121: objectives — the centre (1 or 2 zones) + one per side
        let nc = center_defs.len();
        let objectives = [
            ObjectiveState {
                name: "center",
                zones: (0..nc).collect(),
                home: -1,
            },
            ObjectiveState {
                name: "sideA",
                zones: vec![nc],
                home: 0,
            },
            ObjectiveState {
                name: "sideB",
                zones: vec![nc + 1],
                home: 1,
            },
        ];
        let mut z = Self {
            zones,
            objectives,
            active: 0, // JS L122
            owner: -1,
            last_owner: -1,
            neutral_t: 0.0,
            count: [zt.count as f32; 2],
            penalty: [0.0, 0.0],
            start: [zt.count as f32; 2],
            tie_end: [None, None],
            hold_t: [0.0, 0.0],
            clock: 0.0,
            // JS L132: `match.time <= FINAL` — a match this short never rotates
            final_lock: time <= zt.final_centre as f32,
            next_swap: 0.0,
            sample_t: 0.0, // JS L134
            snap_t: 0.0,   // JS L139
            overtime: false,
            overtime_t: 0.0,
            ot_losing: -1,
            winner: None,
            reason: None,
            log: Vec::new(),
            placeholder,
            follower,
            net_out: Vec::new(),
            events_out: Vec::new(),
            zt: *zt,
        };
        z.schedule(time, rng); // JS L133
        z
    }

    /// JS `_net` (L144): the host records; a follower replays instead.
    fn _net(&mut self, e: JVal) {
        if !self.follower {
            self.net_out.push(e);
        }
    }

    /// Drain the pending `['z…']` records (JS `recZone` sink; batch 3 ships
    /// them on the event timeline).
    pub fn drain_net(&mut self) -> Vec<JVal> {
        std::mem::take(&mut self.net_out)
    }

    /// Drain the pending `zones:*` events.
    pub fn drain_events(&mut self) -> Vec<ZoneEvent> {
        std::mem::take(&mut self.events_out)
    }

    /// Is `p` inside the operational objective's zones (JS `inZone` over
    /// `Z.active.zones`, match.js L139)?
    #[must_use]
    pub fn point_in_active(&self, p: Vec3) -> bool {
        self.objectives[self.active]
            .zones
            .iter()
            .any(|&zi| in_zone(&self.zones[zi].def, p))
    }

    /// JS `total` (L148): count + penalty — what the team still has to count
    /// off. NOT the score (that is [`ZoneControl::losing`]).
    #[must_use]
    pub fn total(&self, t: usize) -> f32 {
        self.count[t] + self.penalty[t]
    }

    /// JS `losing` (L150): the team behind (higher count; penalty excluded),
    /// -1 when level.
    #[must_use]
    pub fn losing(&self) -> i8 {
        let a = self.count[0];
        let b = self.count[1];
        if a == b {
            -1
        } else if a > b {
            0
        } else {
            1
        }
    }

    /// JS `_swapDelay` (L151): `rotateMin + random() * (rotateMax - rotateMin)`.
    #[must_use]
    pub fn swap_delay(&self, rng: &mut Rng) -> f32 {
        self.zt.rotate_min as f32
            + rng.next_f32() * (self.zt.rotate_max - self.zt.rotate_min) as f32
    }

    /// JS `_schedule` (L154-157): no rotation that would land inside the last
    /// `FINAL` s (+ a short stint): from the centre there are none left
    /// (`INFINITY`); from a side zone the final lock itself takes it back.
    pub fn schedule(&mut self, time: f32, rng: &mut Rng) {
        let d = self.swap_delay(rng);
        let left = time - self.zt.final_centre as f32;
        self.next_swap = if left - d >= MIN_STINT {
            d
        } else if self.active == 0 {
            f32::INFINITY
        } else {
            (left + 0.5).max(0.5)
        };
    }

    /// JS `_sample` (L170-190): coverage of the operational zones + control.
    /// `elapsed` is `duration - time` for the penalty log (JS reads
    /// `this.match` live).
    fn _sample(&mut self, paint: &mut PaintGrid, world: &CollisionWorld, elapsed: f32) {
        let zids: Vec<usize> = self.objectives[self.active].zones.clone();
        for zid in zids {
            // JS L173-177: share over the zone's cells (grid value 1 = team0,
            // 2 = team1). The grid borrow re-opens each iteration so a flood
            // below can take `&mut paint`.
            let (a, b, n) = {
                let grid = paint.grid();
                let c = &self.zones[zid].cells;
                let mut a = 0usize;
                let mut b = 0usize;
                for &k in c {
                    match grid[k as usize] {
                        1 => a += 1,
                        2 => b += 1,
                        _ => {}
                    }
                }
                (a, b, c.len().max(1))
            };
            {
                let z = &mut self.zones[zid];
                z.share[0] = a as f32 / n as f32;
                z.share[1] = b as f32 / n as f32;
            }
            if self.follower {
                // JS L178: control is the host's call; followers only warn
                if self.zones[zid].owner >= 0 {
                    let ev = contest(&mut self.zones[zid], self.zt.warn, self.clock);
                    if let Some(e) = ev {
                        self.events_out.push(e);
                    }
                }
                continue;
            }
            if self.clock < self.zones[zid].hold {
                continue; // JS L179: still being wiped for its turn
            }
            let owner = self.zones[zid].owner;
            if owner == -1 {
                // JS L181: take at `control` share
                let t: i8 = if self.zones[zid].share[0] >= self.zt.control {
                    0
                } else if self.zones[zid].share[1] >= self.zt.control {
                    1
                } else {
                    -1
                };
                if t >= 0 {
                    self._zone_owner(zid, t, paint, world);
                }
            } else if self.zones[zid].share[1 - owner as usize] >= self.zt.contest {
                // JS L183-184: neutralise at the `contest` share
                self._zone_owner(zid, -1, paint, world);
            } else {
                let ev = contest(&mut self.zones[zid], self.zt.warn, self.clock);
                if let Some(e) = ev {
                    self.events_out.push(e);
                }
            }
        }
        if self.follower {
            return; // JS L187
        }
        // JS L188-189: the objective is held when a team holds all of its zones
        let zids: Vec<usize> = self.objectives[self.active].zones.clone();
        let o: i8 = if zids.iter().all(|&i| self.zones[i].owner == 0) {
            0
        } else if zids.iter().all(|&i| self.zones[i].owner == 1) {
            1
        } else {
            -1
        };
        if o != self.owner {
            self._set_owner(o, elapsed);
        }
    }

    /// JS `_zoneOwner` (L192-197): a zone taken (flooded with the taker's
    /// ink) or neutralised.
    fn _zone_owner(&mut self, zid: usize, t: i8, paint: &mut PaintGrid, world: &CollisionWorld) {
        self._net(JVal::Arr(vec![
            JVal::Str("zz".to_string()),
            jn(zid as i64),
            jn(t as i64),
        ]));
        let clock = self.clock;
        {
            let z = &mut self.zones[zid];
            z.owner = t;
            if t >= 0 {
                z.warn_armed = true;
                z.warn_t = -99.0;
                flood_zone(z, t, clock, paint, world);
            }
        }
        self.events_out.push(ZoneEvent::Zone {
            zone: zid,
            owner: t,
        });
    }

    /// JS `_setOwner` (L208-223): the objective changed hands. The penalty
    /// record (`['zp']`, L215) lands before `['zo']` (L221) so a follower
    /// plays them in the same order.
    fn _set_owner(&mut self, o: i8, elapsed: f32) {
        let prev = self.owner;
        self.owner = o;
        if o == -1 {
            self.neutral_t = 0.0;
        } else {
            let other = 1 - o;
            // JS L214-215: the other team loses it to us -> its penalty
            if self.last_owner == other {
                self._penalise(other as usize, elapsed);
            }
            // JS L216-217: a capture off the other team starts our stint
            if self.last_owner != o {
                let t = self.total(o as usize);
                self.start[o as usize] = t;
                self.tie_end[o as usize] = None;
            }
            self.last_owner = o;
        }
        self.hold_t = [0.0, 0.0];
        self._net(JVal::Arr(vec![
            JVal::Str("zo".to_string()),
            jn(o as i64),
            // Task 47 follow-up: the zone clock at record time (field 2) —
            // stamped BEFORE this frame's `_tick`, so the follower's catch-up
            // adds one dt (see [`ZoneControl::catch_up`]).
            jf(self.clock),
        ]));
        self.events_out.push(ZoneEvent::Control {
            owner: o,
            prev,
            objective: self.objectives[self.active].name.to_string(),
        });
    }

    /// JS `_penalise` (L228-236): `ROUND(K × (start − end)) + (start == count
    /// ? 1 : 0)` — `Math.round` half-up via [`js_round`].
    fn _penalise(&mut self, t: usize, elapsed: f32) {
        let start = self.start[t];
        // JS L230: past a tie point, progress beyond it is not penalised
        let end = match self.tie_end[t] {
            Some(te) => te.max(self.total(t)),
            None => self.total(t),
        };
        // JS L231: all-integer maths in f64; the sim runs f32 (deviation
        // note: same values at these magnitudes).
        let p = js_round(dec(self.zt.penalty_k * (start - end))).max(0.0) as f32
            + if start == self.zt.count as f32 {
                1.0
            } else {
                0.0
            };
        self._net(JVal::Arr(vec![
            JVal::Str("zp".to_string()),
            jn(t as i64),
            jn(p as i64),
            jf(start),
            jf(end),
        ]));
        self.penalty[t] += p;
        let total = self.total(t);
        self.log.push(ZonePenaltyLog {
            t: elapsed,
            team: t,
            start,
            end,
            penalty: p,
            next: total,
        });
        self.events_out.push(ZoneEvent::Penalty {
            team: t,
            penalty: p,
            total,
            start,
            end,
        });
    }

    /// JS `_tick` (L238-249): count down the holder — penalty first, then the
    /// count; record the tie point when the behind team catches up.
    fn _tick(&mut self, t: usize, dt: f32) {
        let home = self.objectives[self.active].home;
        // JS L240: centre rate / own-half rate / enemy-half rate
        let rate = if home < 0 {
            self.zt.rate_center
        } else if home == t as i8 {
            self.zt.rate_home
        } else {
            self.zt.rate_away
        };
        let mut d = rate * dt;
        let before = self.count[t];
        let opp = self.count[1 - t];
        let from_pen = self.penalty[t].min(d);
        self.penalty[t] -= from_pen;
        d -= from_pen;
        self.count[t] = (self.count[t] - d).max(0.0);
        // JS L246-248: the tie point
        if self.tie_end[t].is_none() && before > opp && self.count[t] <= opp {
            self.tie_end[t] = Some(opp);
        }
    }

    /// JS `_tick` (L238-243): the centre / own-half / enemy-half countdown
    /// rate for team `t` on the active objective.
    fn z_rate(&self, t: usize) -> f32 {
        let home = self.objectives[self.active].home;
        if home < 0 {
            self.zt.rate_center
        } else if home == t as i8 {
            self.zt.rate_home
        } else {
            self.zt.rate_away
        }
    }

    /// Task 47 follow-up (deviation from JS, zones.js L272 / L401-407): the
    /// follower adopted a `['zs']` / `['zo']` value stamped at the host's
    /// record frame, but the host kept counting down through the delivery
    /// delay (one tick interval plus relay slack). Advance the holder's
    /// countdown by that delay exactly as [`Self::_tick`] would, so the
    /// replayed penalty matches the host instead of drifting stale until the
    /// next correction. `lag <= 0` (a clock that has not caught up yet) is a
    /// no-op; the penalty floors at 0 like `_tick`'s `min(d)`.
    fn catch_up(&mut self, lag: f32) {
        if self.owner < 0 || lag <= 0.0 {
            return;
        }
        let t = self.owner as usize;
        let d = self.z_rate(t) * lag;
        let from_pen = self.penalty[t].min(d);
        self.penalty[t] -= from_pen;
        self.count[t] = (self.count[t] - (d - from_pen)).max(0.0);
    }

    /// JS `_fillSpecials` (L252-263): the disadvantaged team's gauges fill.
    /// `remote` is the [`Match::step_masked`](crate::match_::Match::step_masked)
    /// ownership mask (JS `a.remote`, L258); the `special:ready` edge rides
    /// the actor event bus like `Actor::add_turf`.
    fn _fill_specials(&mut self, dt: f32, actors: &mut [Actor], remote: Option<&[bool]>) {
        let (team, rate) = if self.owner >= 0 {
            (1 - self.owner, self.zt.gauge_held)
        } else {
            (self.losing(), self.zt.gauge_neutral)
        };
        if team < 0 {
            return;
        }
        for (i, a) in actors.iter_mut().enumerate() {
            if a.team != team as usize || !a.alive || a.special_active.is_some() {
                continue;
            }
            if remote.is_some_and(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let was = a.special_ready();
            a.special = a.special_cost.min(a.special + rate * dt);
            if !was && a.special_ready() {
                a.push_event(ActorEvent::SpecialReady);
            }
        }
    }

    /// JS `update` (L266-289): the per-frame rules while the match plays
    /// (and through overtime). `Some((winner, reason))` = the rules ended the
    /// match this frame (JS `return this._end(...)` → `match.endZones`).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        time: f32,
        duration: f32,
        paint: &mut PaintGrid,
        world: &CollisionWorld,
        actors: &mut [Actor],
        remote: Option<&[bool]>,
        rng: &mut Rng,
    ) -> Option<(usize, String)> {
        if self.winner.is_some() {
            return None; // JS L267
        }
        self.clock += dt;
        self.sample_t -= dt;
        if self.sample_t <= 0.0 {
            self.sample_t = 1.0 / self.zt.sample_hz as f32;
            self._sample(paint, world, duration - time);
        }
        if self.owner >= 0 {
            let o = self.owner as usize;
            self._tick(o, dt);
            self.hold_t[o] += dt;
            if !self.follower && self.count[o] <= 0.0 && self.penalty[o] <= 0.0 {
                return self._end(o, "knockout"); // JS L274
            }
        } else {
            self.neutral_t += dt;
        }
        self._fill_specials(dt, actors, remote);
        if self.follower {
            // JS L277: a guest only predicts the swap clock / overtime timer
            if self.overtime {
                self.overtime_t += dt;
            } else if !self.final_lock {
                self.next_swap -= dt;
            }
            return None;
        }
        self.snap_t -= dt;
        if self.snap_t <= 0.0 {
            // JS L278-282: the count snapshot twice a second
            self.snap_t = 0.5;
            self._net(JVal::Arr(vec![
                JVal::Str("zs".to_string()),
                jf(self.count[0]),
                jf(self.count[1]),
                jf(self.penalty[0]),
                jf(self.penalty[1]),
                if self.next_swap.is_finite() {
                    jf(self.next_swap)
                } else {
                    jn(-1)
                },
                jf(self.overtime_t),
                jf(self.neutral_t),
                jf(self.hold_t[0]),
                jf(self.hold_t[1]),
                // Task 47 follow-up: the zone clock at record time (field 10,
                // AFTER this frame's `_tick`) — see [`ZoneControl::catch_up`].
                jf(self.clock),
            ]));
        }
        // JS L283-288: rotation — none in overtime, none once the last FINAL
        // s have started (this frame's tick reaching them -> the lock)
        if self.overtime {
            return self._overtime(dt, rng);
        }
        if !self.final_lock {
            if time - dt <= self.zt.final_centre as f32 {
                self._final_lock(time, rng, paint, world);
            } else {
                self.next_swap -= dt;
                if self.next_swap <= 0.0 {
                    self._swap(None, false, time, rng, paint, world);
                }
            }
        }
        None
    }

    /// JS `_swap` (L291-303): rotate the operational objective (to `to`, or
    /// centre <-> a random side). The new objective starts neutral on bare
    /// floor (wiped as it is revealed).
    fn _swap(
        &mut self,
        to: Option<usize>,
        final_lock: bool,
        time: f32,
        rng: &mut Rng,
        paint: &mut PaintGrid,
        world: &CollisionWorld,
    ) {
        // JS L293: `to || (active === o[0] ? (random < 0.5 ? o[1] : o[2]) : o[0])`
        let next = match to {
            Some(i) => i,
            None => {
                if self.active == 0 {
                    if rng.next_f32() < 0.5 { 1 } else { 2 }
                } else {
                    0
                }
            }
        };
        self._net(JVal::Arr(vec![
            JVal::Str("za".to_string()),
            jn(next as i64),
            jn(i64::from(final_lock)),
        ]));
        let old: Vec<usize> = self.objectives[self.active].zones.clone();
        for zid in old {
            self.zones[zid].owner = -1; // JS L295
        }
        self.active = next;
        // JS L297-298: the new objective's zones wipe to bare floor
        let clock = self.clock;
        let new: Vec<usize> = self.objectives[next].zones.clone();
        for zid in new {
            let z = &mut self.zones[zid];
            z.owner = -1;
            z.warn_armed = true;
            flood_zone(z, -1, clock, paint, world);
        }
        // JS L299: the lock kills the rotation clock; a normal swap reschedules
        if final_lock {
            self.next_swap = 0.0;
        } else {
            self.schedule(time, rng);
        }
        if self.owner != -1 {
            // JS L300: the objective went neutral with the move
            self.owner = -1;
            self.neutral_t = 0.0;
            self.events_out.push(ZoneEvent::Control {
                owner: -1,
                prev: self.last_owner,
                objective: self.objectives[next].name.to_string(),
            });
        }
        self.sample_t = 0.0; // JS L301: re-read control from the ink right away
        let zids: Vec<usize> = self.objectives[next].zones.clone();
        self.events_out.push(ZoneEvent::Active {
            objective: self.objectives[next].name.to_string(),
            zones: zids,
            final_lock,
            moved: true,
        });
    }

    /// JS `_finalLock` (L306-312): the last `FINAL` s — the centre only from
    /// here to the end (overtime included).
    fn _final_lock(
        &mut self,
        time: f32,
        rng: &mut Rng,
        paint: &mut PaintGrid,
        world: &CollisionWorld,
    ) {
        self.final_lock = true;
        self.next_swap = 0.0;
        if self.active != 0 {
            self._swap(Some(0), true, time, rng, paint, world);
        } else {
            self._net(JVal::Arr(vec![JVal::Str("zf".to_string())]));
            let zids: Vec<usize> = self.objectives[0].zones.clone();
            self.events_out.push(ZoneEvent::Active {
                objective: "center".to_string(),
                zones: zids,
                final_lock: true,
                moved: false,
            });
        }
    }

    /// JS `timeUp` (L315-333): the horn. `Some((winner, reason))` = the match
    /// ends now (JS `true`); `None` = overtime has begun (JS `false`).
    pub fn time_up(&mut self) -> Option<(usize, String)> {
        let l = self.losing();
        // JS L317: the team that only just lost the objective gets the grace
        let grace = self.owner == -1
            && self.last_owner == l
            && self.neutral_t < self.zt.overtime_grace as f32;
        if l >= 0 && (self.owner == l || grace) {
            self.overtime = true;
            self.overtime_t = 0.0;
            self.ot_losing = l;
            self._net(JVal::Arr(vec![JVal::Str("zt".to_string()), jn(l as i64)]));
            self.events_out.push(ZoneEvent::Overtime { losing: l });
            return None;
        }
        if l < 0 {
            // JS L324-330: level at the horn — sudden death
            self.overtime = true;
            self.overtime_t = 0.0;
            self._net(JVal::Arr(vec![JVal::Str("zt".to_string()), jn(-1)]));
            self.events_out.push(ZoneEvent::Overtime { losing: -1 });
            return None;
        }
        self._end((1 - l) as usize, "time")
    }

    /// JS `_overtime` (L338-355): the overtime end conditions (comeback /
    /// retake / neutralised / cap).
    fn _overtime(&mut self, dt: f32, rng: &mut Rng) -> Option<(usize, String)> {
        self.overtime_t += dt;
        // JS L340-349: the team behind at the horn stays "L"; sudden death
        // picks L the moment someone falls behind
        let mut l = self.ot_losing;
        if l < 0 {
            l = self.losing();
            if l < 0 {
                if self.overtime_t >= self.zt.overtime_max as f32 {
                    return self._end(self._tiebreak(rng), "overtime-cap");
                }
                return None;
            }
        }
        let w = 1 - l;
        if self.count[l as usize] < self.count[w as usize] {
            return self._end(l as usize, "comeback");
        }
        if self.owner == w {
            return self._end(w as usize, "retake");
        }
        if self.owner != l && self.neutral_t >= self.zt.overtime_grace as f32 {
            return self._end(w as usize, "neutralised");
        }
        if self.overtime_t >= self.zt.overtime_max as f32 {
            return self._end(w as usize, "overtime-cap");
        }
        None
    }

    /// JS `_tiebreak` (L357-361): the summed share of the active zones; a dead
    /// heat flips a seeded coin.
    fn _tiebreak(&self, rng: &mut Rng) -> usize {
        let mut s = [0.0f32; 2];
        for &zid in &self.objectives[self.active].zones {
            s[0] += self.zones[zid].share[0];
            s[1] += self.zones[zid].share[1];
        }
        if s[0] == s[1] {
            if rng.next_f32() < 0.5 { 0 } else { 1 }
        } else if s[0] > s[1] {
            0
        } else {
            1
        }
    }

    /// JS `_end` (L363-369): the decision. `Some((winner, reason))` on the
    /// first call (the caller runs `Match::end_zones`, JS `match.endZones`);
    /// `None` when already decided (the JS winner guard).
    fn _end(&mut self, winner: usize, reason: &str) -> Option<(usize, String)> {
        if self.winner.is_some() {
            return None;
        }
        // JS L365: the final numbers, exact
        self._net(JVal::Arr(vec![
            JVal::Str("ze".to_string()),
            jn(winner as i64),
            JVal::Str(reason.to_string()),
            jf(self.count[0]),
            jf(self.count[1]),
            jf(self.penalty[0]),
            jf(self.penalty[1]),
        ]));
        self.winner = Some(winner);
        self.reason = Some(reason.to_string());
        self.events_out.push(ZoneEvent::End {
            winner,
            reason: reason.to_string(),
            counts: self.count,
            penalty: self.penalty,
        });
        Some((winner, reason.to_string()))
    }

    /// JS `netEvent` (L372-413): a follower replays the host's records (the
    /// batch-3 timeline, `TickEvent::Zone`). `Some((winner, reason))` when a
    /// `['ze']` decides the match (JS `_end` → `match.endZones`).
    #[allow(clippy::too_many_arguments)]
    pub fn net_event(
        &mut self,
        e: &JVal,
        paint: &mut PaintGrid,
        world: &CollisionWorld,
        time: f32,
        duration: f32,
        rng: &mut Rng,
    ) -> Option<(usize, String)> {
        // JS L373: non-arrays, hosts and decided matches ignore the stream
        let arr = match e {
            JVal::Arr(a) => a,
            _ => return None,
        };
        if !self.follower || self.winner.is_some() {
            return None;
        }
        let tag = match arr.first() {
            Some(JVal::Str(s)) => s.as_str(),
            _ => return None,
        };
        let n = |i: usize| arr.get(i).and_then(JVal::as_f64).unwrap_or(0.0);
        match tag {
            "zz" => {
                // JS L375
                let zid = n(1) as usize;
                if zid < self.zones.len() {
                    let t = n(2) as i8;
                    self._zone_owner(zid, t, paint, world);
                }
            }
            "zo" => {
                // JS L376-385
                let o = n(1) as i8;
                let prev = self.owner;
                if o != prev {
                    self.owner = o;
                    if o == -1 {
                        self.neutral_t = 0.0;
                    } else {
                        if self.last_owner != o {
                            let t = self.total(o as usize);
                            self.start[o as usize] = t;
                            self.tie_end[o as usize] = None;
                        }
                        self.last_owner = o;
                    }
                    self.hold_t = [0.0, 0.0];
                    self.events_out.push(ZoneEvent::Control {
                        owner: o,
                        prev,
                        objective: self.objectives[self.active].name.to_string(),
                    });
                    // Task 47 follow-up: the host's `_tick` ran from its own
                    // capture frame (recorded BEFORE that tick — hence the
                    // extra dt) through ours; our prediction started with the
                    // replay, so catch the new holder's countdown up.
                    if o >= 0 && arr.len() > 2 {
                        let lag = self.clock - n(2) as f32 + FIXED_DT;
                        self.catch_up(lag);
                    }
                }
            }
            "zp" => {
                // JS L386-391
                let t = n(1) as usize;
                let p = n(2) as f32;
                let start = n(3) as f32;
                let end = n(4) as f32;
                self.penalty[t] += p;
                let total = self.total(t);
                self.log.push(ZonePenaltyLog {
                    t: duration - time,
                    team: t,
                    start,
                    end,
                    penalty: p,
                    next: total,
                });
                self.events_out.push(ZoneEvent::Penalty {
                    team: t,
                    penalty: p,
                    total,
                    start,
                    end,
                });
            }
            "za" => {
                // JS L393
                let next = n(1) as usize;
                if next < self.objectives.len() {
                    let fin = n(2) != 0.0;
                    if fin {
                        self.final_lock = true;
                    }
                    self._swap(Some(next), fin, time, rng, paint, world);
                }
            }
            "zf" => {
                // JS L394-399
                self.final_lock = true;
                self.next_swap = 0.0;
                let zids: Vec<usize> = self.objectives[0].zones.clone();
                self.events_out.push(ZoneEvent::Active {
                    objective: "center".to_string(),
                    zones: zids,
                    final_lock: true,
                    moved: false,
                });
            }
            "zt" => {
                // JS L400
                let l = n(1) as i8;
                self.overtime = true;
                self.overtime_t = 0.0;
                self.ot_losing = l;
                self.events_out.push(ZoneEvent::Overtime { losing: l });
            }
            "zs" => {
                // JS L401-407: the count snapshot corrects the prediction
                self.count = [n(1) as f32, n(2) as f32];
                self.penalty = [n(3) as f32, n(4) as f32];
                let ns = n(5) as f32;
                self.next_swap = if ns < 0.0 { f32::INFINITY } else { ns };
                self.overtime_t = n(6) as f32;
                self.neutral_t = n(7) as f32;
                self.hold_t = [n(8) as f32, n(9) as f32];
                // Task 47 follow-up: the snapshot is stamped at the host's
                // record frame (field 10, AFTER that frame's `_tick`); the
                // host kept counting down until this replay landed, so catch
                // the holder's countdown up by the delivery delay.
                if arr.len() > 10 {
                    let lag = self.clock - n(10) as f32;
                    self.catch_up(lag);
                }
            }
            "ze" => {
                // JS L408-411
                if arr.len() > 3 {
                    self.count = [n(3) as f32, n(4) as f32];
                    self.penalty = [n(5) as f32, n(6) as f32];
                }
                let w = n(1) as usize;
                let reason = arr
                    .get(2)
                    .and_then(JVal::as_str)
                    .unwrap_or_default()
                    .to_string();
                return self._end(w, &reason);
            }
            _ => {}
        }
        None
    }

    /// JS `state` (L416-424): the HUD / results / tests snapshot.
    #[must_use]
    pub fn state(&self) -> ZoneSnapshot {
        let zids: Vec<usize> = self.objectives[self.active].zones.clone();
        ZoneSnapshot {
            active: self.objectives[self.active].name.to_string(),
            owner: self.owner,
            last_owner: self.last_owner,
            count: [self.count[0].ceil() as i64, self.count[1].ceil() as i64],
            penalty: self.penalty,
            total: [self.total(0).ceil() as i64, self.total(1).ceil() as i64],
            zones: zids
                .iter()
                .map(|&i| {
                    let z = &self.zones[i];
                    ZoneSnapZone {
                        id: z.id,
                        owner: z.owner,
                        share: [r3(dec(z.share[0])), r3(dec(z.share[1]))],
                        cells: z.cells.len(),
                        center: z.center,
                    }
                })
                .collect(),
            next_swap: if self.final_lock || !self.next_swap.is_finite() {
                0.0
            } else {
                r1(self.next_swap)
            },
            final_lock: self.final_lock,
            overtime: self.overtime,
            overtime_t: r1(self.overtime_t),
            neutral_t: r1(self.neutral_t),
            winner: self.winner,
            reason: self.reason.clone(),
            placeholder: self.placeholder,
        }
    }
}
