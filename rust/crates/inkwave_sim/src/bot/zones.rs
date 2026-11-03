//! Zone Control team plan (spec Task 38 batch 3): the shared per-team brain
//! of every bot in a zones match — the zone cells mapped to standable nav
//! nodes, the live ink re-scan, the per-team role deal and the retake waves.
//! Faithful port of the `ZonePlan` class of `src/game/bots.js` (L70-268).
//!
//! The JS plan hangs off the `ZoneControl` instance through a `WeakMap`
//! (bots.js L70); the sim stores it as [`crate::match_::Match::zone_plan`]
//! and [`ZonePlan::tick`] is driven once per frame by the replication /
//! autopilot layer before the bots step, with the same 0.33 s scan gate
//! (JS L175) riding `ZoneControl::clock` (JS `G.time` — the zones clock keeps
//! the same phase; recorded deviation).
//!
//! Upstream mapping (bots.js):
//!   - `zonePlan()` / `new ZonePlan(Z)` + `_build`  -> [`ZonePlan::build`]
//!   - `tick` (ink re-scan, waves, role re-deal)    -> [`ZonePlan::tick`]
//!   - `_wave` / `_assign`                          -> private helpers
//!   - `isActive` / `roleOf` / `onZone` / `onActive`
//!     / `hotspot` / `enemyDir` / `waveOf`          -> same-named methods
//!   - `lobPitch` (L39-48)                          -> [`lob_pitch`]
//!
//! Intentional deviations from JS (recorded for review):
//!   - **actor identity**: JS keys `roles` / `zoneOf` Maps on the actor
//!     objects; the sim indexes by roster position (parallel to
//!     [`crate::actor::Actor`]), so a splat/respawn keeps one bot's identity
//!     and remote (human) actors simply never appear in `is_bot`.
//!   - **`reach`**: JS's zone object also carries a `reach` (the rules
//!     engine's corner distance); the plan's own reachable-cell count keeps
//!     a distinct name here (`ZoneInfo::reach`) — the JS local `reach`
//!     counter (L136) of the same value.
//!   - **`hotspot` return**: JS hands back a nav node object; the sim
//!     returns the node's `Vec3` (the only fields the strike retarget reads).
//!   - **`spNext` mutation from the brains**: JS mutates `P.spNext[t]` from
//!     `_zoneSpecial` through the shared plan; the sim keeps it in a relaxed
//!     [`AtomicU32`] (f32 bits) so `BotCtx` can hold the plan immutably while
//!     a bot still chains the push's special gate (single-threaded, same
//!     sequential semantics; atomics keep the plan `Sync` for the Bevy
//!     render resource, where a `Cell` would not).
//!   - **`_zoneJump` / `zJumpAt` are cut** (module docs of `bot`): the sim
//!     has no super jumps, so the post-rotation "jump to the zone" arm is
//!     absent by construction.

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, SimWorld};
use crate::match_::zones::ZoneControl;
use crate::nav::NavGraph;
use crate::paint::PaintGrid;
use crate::weapon::WeaponKind;

/// JS `PAINTERS` (bots.js L34) minus the retired slosher: the close-range
/// bodies that always paint (and re-ink) rather than perch or guard.
#[must_use]
pub fn is_painter_kind(kind: WeaponKind) -> bool {
    matches!(
        kind,
        WeaponKind::Roller | WeaponKind::Brush | WeaponKind::Bucket
    )
}

/// JS `lobPitch` (bots.js L39-48): the aim pitch that lobs a thrown sub
/// (weapons.js `throwVelocity`: pitch + 0.28, +1.5 up, from 1.35 m, gravity
/// 24) `d` metres out to a spot `dy` above our feet; `None` when out of
/// reach. The JS `for (p = -0.5; p <= 0.75; p += 0.025)` ladder is walked by
/// index (f32 accumulation would drift off the JS double grid).
#[must_use]
pub fn lob_pitch(d: f32, dy: f32, speed: f32) -> Option<f32> {
    let mut best = f32::NAN;
    let mut be = f32::INFINITY;
    for k in 0..=50 {
        let p = -0.5 + k as f32 * 0.025;
        let tp = (p + 0.28).clamp(-0.3, 1.1);
        let vx = tp.cos() * speed;
        let vy = tp.sin() * speed + 1.5;
        let disc = vy * vy + 48.0 * (1.35 - dy);
        if disc < 0.0 {
            continue;
        }
        let e = ((vx * (vy + disc.sqrt())) / 24.0 - d).abs();
        if e < be {
            be = e;
            best = p;
        }
    }
    if be < 1.2 { Some(best) } else { None }
}

/// JS role strings (bots.js L257-261): the per-bot job on the objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneRole {
    /// JS `'paint'` — ink the zone's neediest patches.
    Paint,
    /// JS `'guard'` — hold the enemy-facing edge and re-ink.
    Guard,
    /// JS `'watch'` — a long-range perch with a sightline over the zone.
    Watch,
    /// JS `'push'` — press forward past the zone while we hold it.
    Push,
    /// JS `'stage'` — regroup at the staging spot until the push (retake).
    Stage,
}

/// JS `out.set(a, { role, zone })` (L263): one bot's assignment.
#[derive(Debug, Clone, Copy)]
pub struct RoleAssign {
    pub role: ZoneRole,
    /// Zone id (== index into [`ZoneControl::zones`]).
    pub zone: usize,
}

/// JS `waves[t]` (L83): the team's retake wave state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveState {
    /// JS `'free'` — we hold it / it's neutral.
    Free,
    /// JS `'stage'` — regrouping outside the zone.
    Stage,
    /// JS `'push'` — going in together.
    Push,
}

impl WaveState {
    fn label(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Stage => "stage",
            Self::Push => "push",
        }
    }
}

/// JS `waves[t]` entry (L83).
#[derive(Debug, Clone, Copy)]
pub struct Wave {
    pub state: WaveState,
    /// The zone the wave is going for; `-1` while free (JS `zone: -1`).
    pub zone: isize,
    /// JS `t0`: when the current state began.
    pub t0: f32,
    pub pushes: u32,
}

/// JS `I.stage[t]` (L131): the staging spot for team `t` at this zone.
#[derive(Debug, Clone)]
pub struct StageSpot {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// JS `spots`: the ring nodes near the best spot to spread across.
    pub spots: Vec<usize>,
}

/// JS `this.info[zi]` (L90-134): one zone's prebuilt plan data.
#[derive(Debug, Clone)]
pub struct ZoneInfo {
    /// JS `cellNode`: per zone cell, the local node index or -1.
    pub cell_node: Vec<i32>,
    /// JS `nodes`: standable nav node ids (the local-index space).
    pub nodes: Vec<usize>,
    /// JS `nb`: neighbourhoods (≤ 2.3 m, same level) per local node.
    pub nb: Vec<Vec<usize>>,
    /// JS `reach` (L102): reachable-cell count (≠ `ZoneState::reach`).
    pub reach: usize,
    /// JS `R`: farthest cell corner from the centre, min 2.
    pub r: f32,
    /// JS `ring`: nav nodes on / by the zone (guard + watch spots).
    pub ring: Vec<usize>,
    /// JS `far`: nav nodes 3-18 m out (push spots).
    pub far: Vec<usize>,
    /// JS `parts`: the zone outline part(s) for point-in-polygon tests.
    pub parts: Vec<Vec<[f32; 2]>>,
    /// JS `y0` / `y1` (defaults -2 / 6).
    pub y0: f32,
    pub y1: f32,
    pub stage: [Option<StageSpot>; 2],
    /// JS `need[t][li]`: per-node ink need for team t.
    pub need: [Vec<f32>; 2],
    pub need_sum: [f32; 2],
    /// JS `ink[t][li]`: per-node cell count painted by team t.
    pub ink: [Vec<u16>; 2],
}

/// The shared Zone Control plan (JS `class ZonePlan`).
#[derive(Debug)]
pub struct ZonePlan {
    /// Per zone id.
    pub info: Vec<ZoneInfo>,
    /// JS `roles[t]`: per roster index, `None` for non-bots / unassigned.
    roles: [Vec<Option<RoleAssign>>; 2],
    waves: [Wave; 2],
    /// JS `spNext[t]` — relaxed atomic (f32 bits) so the brains can chain it
    /// through `&ZonePlan` while the plan stays `Sync` (see the module docs).
    sp_next: [AtomicU32; 2],
    /// JS `this.focus` (L246): the two-zone hysteresis pick per team.
    focus: [Option<usize>; 2],
    /// JS `this.t` / `this.roleT` (L79): the scan / role-deal clocks.
    t: f32,
    role_t: f32,
    /// JS `this.sig` (L79).
    sig: String,
}

impl ZonePlan {
    /// JS `zonePlan()` ctor (L69-79) + `_build` (L83-134): build the plan once per
    /// [`ZoneControl`] (the sim clears it on restart / zones-mode change).
    #[must_use]
    pub fn build(zctl: &ZoneControl, nav: &NavGraph, paint: &PaintGrid, world: &SimWorld) -> Self {
        // JS L71: turf + atlas faces sorted by their grid offset (the sim's
        // `face_meta(f.id).4` is the JS `f.grid` base index).
        let mut faces: Vec<usize> = world
            .collision
            .faces
            .iter()
            .filter(|f| f.turf && paint.face_meta(f.id as usize).is_some())
            .map(|f| f.id as usize)
            .collect();
        faces.sort_by_key(|&fid| paint.face_meta(fid).map_or(0, |m| m.4));
        let starts: Vec<usize> = faces
            .iter()
            .map(|&fid| paint.face_meta(fid).map_or(0, |m| m.4))
            .collect();
        let info = zctl
            .zones
            .iter()
            .map(|z| build_info(z, nav, paint, world, &faces, &starts))
            .collect();
        Self {
            info,
            roles: [Vec::new(), Vec::new()],
            waves: [
                Wave {
                    state: WaveState::Free,
                    zone: -1,
                    t0: 0.0,
                    pushes: 0,
                },
                Wave {
                    state: WaveState::Free,
                    zone: -1,
                    t0: 0.0,
                    pushes: 0,
                },
            ],
            sp_next: [AtomicU32::new(0), AtomicU32::new(0)],
            focus: [None, None],
            t: -1.0,
            role_t: -1.0,
            sig: String::new(),
        }
    }

    /// JS `P.tick()` (L173-195): the ~3×/s ink re-scan, the wave machine and
    /// the role re-deal on a rotation / change of hands / every 2 s.
    pub fn tick(
        &mut self,
        zctl: &ZoneControl,
        paint: &PaintGrid,
        world: &SimWorld,
        actors: &[Actor],
        kinds: &[WeaponKind],
        is_bot: &[bool],
    ) {
        let now = zctl.clock;
        // JS L175 (the 0.33 s scan gate; `now < this.t` covers a clock reset)
        if self.t >= 0.0 && now - self.t < 0.33 && now >= self.t {
            return;
        }
        self.t = now;
        let grid = paint.grid();
        let active = &zctl.objectives[zctl.active].zones;
        for &zi in active {
            let z = &zctl.zones[zi];
            let i = &mut self.info[zi];
            for k in 0..i.nodes.len() {
                i.need[0][k] = 0.0;
                i.need[1][k] = 0.0;
                i.ink[0][k] = 0;
                i.ink[1][k] = 0;
            }
            let (mut s0, mut s1) = (0.0f32, 0.0f32);
            for (ci, &cell) in z.cells.iter().enumerate() {
                let li = i.cell_node[ci];
                if li < 0 {
                    continue;
                }
                let l = li as usize;
                let g = grid.get(cell as usize).copied().unwrap_or(0);
                if g == 1 {
                    i.ink[0][l] += 1;
                } else if g == 2 {
                    i.ink[1][l] += 1;
                }
                if g != 1 {
                    let v = if g == 2 { 1.5 } else { 1.0 };
                    i.need[0][l] += v;
                    s0 += v;
                }
                if g != 2 {
                    let v = if g == 1 { 1.5 } else { 1.0 };
                    i.need[1][l] += v;
                    s1 += v;
                }
            }
            i.need_sum[0] = s0;
            i.need_sum[1] = s1;
        }
        self.wave(0, zctl, actors, is_bot, now);
        self.wave(1, zctl, actors, is_bot, now);
        // JS L193: rotation / change of hands / wave flip re-deal the roles
        let owners: Vec<String> = active
            .iter()
            .map(|&zi| zctl.zones[zi].owner.to_string())
            .collect();
        let sig = format!(
            "{}:{}:{}:{}{}",
            zctl.objectives[zctl.active].name,
            zctl.owner,
            owners.join(","),
            self.waves[0].state.label(),
            self.waves[1].state.label()
        );
        if sig != self.sig || now - self.role_t > 2.0 || now < self.role_t {
            self.sig = sig;
            self.role_t = now;
            self.assign(0, zctl, world, actors, kinds, is_bot);
            self.assign(1, zctl, world, actors, kinds, is_bot);
        }
    }

    /// JS `waveOf(t)` (L86).
    #[must_use]
    pub fn wave_of(&self, t: usize) -> &Wave {
        &self.waves[t]
    }

    /// JS `P.spNext[t]` read / write (L219 / L1213 / L1227).
    #[must_use]
    pub fn sp_next(&self, t: usize) -> f32 {
        f32::from_bits(self.sp_next[t].load(Ordering::Relaxed))
    }

    pub fn set_sp_next(&self, t: usize, v: f32) {
        self.sp_next[t].store(v.to_bits(), Ordering::Relaxed);
    }

    /// JS `isActive(zi)` (L136).
    #[must_use]
    pub fn is_active(&self, zi: usize, zctl: &ZoneControl) -> bool {
        zctl.objectives[zctl.active].zones.contains(&zi)
    }

    /// JS `roleOf(a)` (L137): the bot's assignment; the default is paint on
    /// the first active zone (a bot the plan has never dealt).
    #[must_use]
    pub fn role_of(&self, actor_idx: usize, team: usize, zctl: &ZoneControl) -> RoleAssign {
        self.roles[team]
            .get(actor_idx)
            .copied()
            .flatten()
            .unwrap_or(RoleAssign {
                role: ZoneRole::Paint,
                zone: zctl.objectives[zctl.active].zones[0],
            })
    }

    /// JS `enemyDir(zone, t)` (L139-142): the unit xz direction from a zone
    /// toward team `t`'s enemy spawn.
    #[must_use]
    pub fn enemy_dir(
        &self,
        zone: &crate::match_::zones::ZoneState,
        t: usize,
        world: &SimWorld,
    ) -> [f32; 2] {
        let p = world.spawn_pads[1 - t];
        let dx = p.x - zone.center[0];
        let dz = p.z - zone.center[2];
        let l = dx.hypot(dz);
        let l = if l == 0.0 { 1.0 } else { l };
        [dx / l, dz / l]
    }

    /// JS `onZone(zi, p)` (L144-147): standing on (or right at the edge of)
    /// zone `zi`?
    #[must_use]
    pub fn on_zone(&self, zi: usize, pos: glam::Vec3, zctl: &ZoneControl) -> bool {
        let i = &self.info[zi];
        let c = zctl.zones[zi].center;
        (pos.x - c[0]).hypot(pos.z - c[2]) < i.r + 1.5 && pos.y > i.y0 - 1.5 && pos.y < i.y1 + 2.5
    }

    /// JS `onActive(x, y, z)` (L149-163): is the point on a live zone? (the
    /// ray-cast point-in-polygon over each outline part).
    #[must_use]
    pub fn on_active(&self, x: f32, y: f32, z: f32, zctl: &ZoneControl) -> bool {
        for &zi in &zctl.objectives[zctl.active].zones {
            let i = &self.info[zi];
            if y < i.y0 - 1.5 || y > i.y1 + 2.5 {
                continue;
            }
            for poly in &i.parts {
                let mut ins = false;
                let n = poly.len();
                let mut j = n.wrapping_sub(1);
                for k in 0..n {
                    let [xi, zi_] = poly[k];
                    let [xj, zj] = poly[j];
                    if (zi_ > z) != (zj > z) && x < ((xj - xi) * (z - zi_)) / (zj - zi_) + xi {
                        ins = !ins;
                    }
                    j = k;
                }
                if ins {
                    return true;
                }
            }
        }
        false
    }

    /// JS `hotspot(zi, t)` (L165-171): the most-needed spot of zone `zi` for
    /// team `t` (a Vortex Strike's landing point); `None` when the zone has
    /// no mapped nodes.
    #[must_use]
    pub fn hotspot(
        &self,
        zi: isize,
        t: usize,
        zctl: &ZoneControl,
        nav: &NavGraph,
    ) -> Option<glam::Vec3> {
        let mut zi = zi;
        if zi < 0 || !self.is_active(zi as usize, zctl) {
            zi = zctl.objectives[zctl.active].zones[0] as isize;
        }
        let i = &self.info[zi as usize];
        let need = &i.need[t];
        let mut bv = -1.0f32;
        let mut bi: Option<usize> = None;
        for li in 0..i.nodes.len() {
            let mut v = 0.0f32;
            for &k in &i.nb[li] {
                v += need[k];
            }
            if v > bv {
                bv = v;
                bi = Some(li);
            }
        }
        bi.map(|li| nav.nodes[i.nodes[li]].pos)
    }

    /// JS `_wave(t, now)` (L198-222): the retake wave machine (only while the
    /// other team holds the objective).
    fn wave(&mut self, t: usize, zctl: &ZoneControl, actors: &[Actor], is_bot: &[bool], now: f32) {
        let w = &mut self.waves[t];
        if zctl.owner != (1 - t) as i8 {
            w.state = WaveState::Free;
            w.zone = -1;
            return;
        }
        if w.state == WaveState::Free {
            // JS L202-204: go for one zone — on a two-zone centre the one
            // they hold least firmly (reduce keeps the *first* minimum, like
            // JS's `<` comparator).
            let zs = &zctl.objectives[zctl.active].zones;
            let mut pick = zs[0];
            for &zi in zs {
                if zctl.zones[zi].share[1 - t] < zctl.zones[pick].share[1 - t] {
                    pick = zi;
                }
            }
            w.zone = pick as isize;
            w.state = WaveState::Stage;
            w.t0 = now;
        }
        let zi = w.zone as usize;
        // The staging / centre data is read through a raw copy so `self`
        // stays mutably borrowable for the counters below.
        let st = self.info[zi].stage[t].as_ref().map(|s| (s.x, s.z));
        let c = zctl.zones[zi].center;
        let r = self.info[zi].r;
        let mut alive = 0u32;
        let mut gathered = 0u32;
        let mut sp = false;
        for (i, a) in actors.iter().enumerate() {
            if a.team != t || !is_bot.get(i).copied().unwrap_or(false) || !a.alive {
                continue;
            }
            alive += 1;
            let near = st.is_some_and(|(sx, sz)| (a.pos.x - sx).hypot(a.pos.z - sz) < 8.0)
                || (a.pos.x - c[0]).hypot(a.pos.z - c[2]) < r + 2.0;
            if near {
                gathered += 1;
            }
            if a.special_ready() && (a.pos.x - c[0]).hypot(a.pos.z - c[2]) < r + 18.0 {
                sp = true;
            }
        }
        let w = &mut self.waves[t];
        if w.state == WaveState::Stage {
            let waited = now - w.t0;
            if (alive >= 3 && gathered >= 3)
                || (sp && gathered >= 2)
                || (gathered >= alive && alive > 0 && waited > 7.0)
                || waited > 15.0
            {
                w.state = WaveState::Push;
                w.t0 = now;
                w.pushes += 1;
                self.sp_next[t].store(now.to_bits(), Ordering::Relaxed);
            }
        } else if w.state == WaveState::Push && alive <= 1 && now - w.t0 > 4.0 {
            // wiped out: regroup
            w.state = WaveState::Stage;
            w.t0 = now;
        }
    }

    /// JS `_assign(t)` (L224-267): deal the roles / zone picks for team `t`.
    #[allow(clippy::too_many_arguments)]
    fn assign(
        &mut self,
        t: usize,
        zctl: &ZoneControl,
        world: &SimWorld,
        actors: &[Actor],
        kinds: &[WeaponKind],
        is_bot: &[bool],
    ) {
        let act = &zctl.objectives[zctl.active].zones;
        let prev = std::mem::take(&mut self.roles[t]);
        let mut out = vec![None; actors.len()];
        // JS L226: the team's bots in roster order (dead ones included).
        let bots: Vec<usize> = (0..actors.len())
            .filter(|&i| actors[i].team == t && is_bot.get(i).copied().unwrap_or(false))
            .collect();
        let hold_all = zctl.owner == t as i8;
        // JS L229: 0 mid · 1 painter · 2 long range (chargers always keep a
        // perch; spinners / splatlings only while we hold — else they ink).
        let class_of = |i: usize| -> u8 {
            let k = kinds.get(i).copied().unwrap_or(WeaponKind::Shooter);
            if k == WeaponKind::Charger || (k.is_long() && hold_all) {
                2
            } else if is_painter_kind(k) {
                1
            } else {
                0
            }
        };
        // JS L231-234: pushers, only while we hold the whole objective.
        let mut n_push = 0usize;
        if hold_all && bots.len() >= 3 {
            n_push = if bots.len() >= 4 && act.iter().all(|&z| zctl.zones[z].share[t] >= 0.9) {
                2
            } else {
                1
            };
        }
        // JS L233-234: prev-pushers first, then mid-range kits, then slot
        // (`Array.prototype.sort` is stable; the sim sorts by the same key).
        let mut cands: Vec<(bool, u8, usize, usize)> = bots
            .iter()
            .filter(|&&i| class_of(i) != 2)
            .map(|&i| {
                (
                    prev.get(i)
                        .copied()
                        .flatten()
                        .is_some_and(|p| p.role == ZoneRole::Push),
                    class_of(i),
                    actors[i].slot,
                    i,
                )
            })
            .collect();
        cands.sort_by(|a, b| (!a.0).cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut pushers: Vec<usize> = cands.iter().take(n_push).map(|c| c.3).collect();
        pushers.sort_unstable();
        // JS L238-254: the zone per bot on a two-zone centre.
        let mut zone_of = vec![usize::MAX; actors.len()];
        if act.len() == 1 {
            for &i in &bots {
                zone_of[i] = act[0];
            }
        } else {
            let (z0, z1) = (act[0], act[1]);
            let (mut w0, mut w1) = (
                if zctl.zones[z0].owner == t as i8 {
                    1
                } else {
                    3
                },
                if zctl.zones[z1].owner == t as i8 {
                    1
                } else {
                    3
                },
            );
            if w0 == 3 && w1 == 3 {
                let pf = self.focus[t];
                let f0 = zctl.zones[z0].share[t] + if pf == Some(z0) { 0.1 } else { 0.0 };
                let f1 = zctl.zones[z1].share[t] + if pf == Some(z1) { 0.1 } else { 0.0 };
                if f0 >= f1 {
                    w1 = 1;
                    self.focus[t] = Some(z0);
                } else {
                    w0 = 1;
                    self.focus[t] = Some(z1);
                }
            }
            // JS `Math.round` (L248) — the wire's half-up round.
            let cap0 =
                (crate::net::wire::js_round((bots.len() as f64 * w0 as f64) / (w0 + w1) as f64))
                    as usize;
            let dd = |i: usize, zi: usize| -> f32 {
                let a = &actors[i];
                let p = if a.alive { a.pos } else { world.spawn_pads[t] };
                let c = zctl.zones[zi].center;
                (p.x - c[0]).hypot(p.z - c[2])
            };
            let key = |i: usize| -> f32 {
                dd(i, z0) - dd(i, z1)
                    + match prev.get(i).copied().flatten().map(|p| p.zone) {
                        Some(z) if z == z0 => -5.0,
                        Some(z) if z == z1 => 5.0,
                        _ => 0.0,
                    }
            };
            let mut order = bots.clone();
            order.sort_by(|&x, &y| {
                key(x)
                    .partial_cmp(&key(y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for (idx, &i) in order.iter().enumerate() {
                zone_of[i] = if idx < cap0 { z0 } else { z1 };
            }
        }
        let w_state = self.waves[t].state;
        let w_zone = self.waves[t].zone;
        for &i in &bots {
            let a = &actors[i];
            let cls = class_of(i);
            let mut role = if pushers.contains(&i) {
                ZoneRole::Push
            } else if cls == 2 {
                ZoneRole::Watch
            } else if hold_all && cls != 1 {
                ZoneRole::Guard
            } else {
                ZoneRole::Paint
            };
            let mut zone = zone_of[i];
            if w_state != WaveState::Free {
                // JS L258-262: locked out — everyone goes for the one zone;
                // off it, wait at the staging spot until the push.
                zone = w_zone as usize;
                if role != ZoneRole::Watch
                    && w_state == WaveState::Stage
                    && !(a.alive && self.on_zone(zone, a.pos, zctl))
                {
                    role = ZoneRole::Stage;
                }
            }
            out[i] = Some(RoleAssign { role, zone });
        }
        self.roles[t] = out;
    }
}

/// JS `_build(z, faces, starts)` (L83-134): map the zone's paint cells to
/// standable nav nodes and precompute the ring / far / staging data.
fn build_info(
    z: &crate::match_::zones::ZoneState,
    nav: &NavGraph,
    paint: &PaintGrid,
    world: &SimWorld,
    faces: &[usize],
    starts: &[usize],
) -> ZoneInfo {
    let [cx, cy, cz] = z.center;
    let n = z.cells.len();
    let mut cell_node = vec![-1i32; n];
    let mut nodes: Vec<usize> = Vec::new();
    let mut idx: Vec<(usize, usize)> = Vec::new(); // id -> local (small, linear like JS Map)
    let mut r = 2.0f32;
    for (i, &cell) in z.cells.iter().enumerate() {
        if starts.is_empty() {
            break;
        }
        let k = cell as usize;
        // JS L89-91: binary search the face whose grid base <= k (the
        // extractor guarantees every zone cell lies on a turf atlas face).
        let mut lo = 0usize;
        let mut hi = starts.len() - 1;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if starts[mid] <= k {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let fid = faces[lo];
        let (nu, _nv, cu, cv, off) = paint.face_meta(fid).expect("turf face has a grid");
        let loc = (k - off) as u32;
        let s = ((loc % nu) as f32 + 0.5) * cu;
        let tt = ((loc / nu) as f32 + 0.5) * cv;
        let f = &world.collision.faces[fid];
        let p = f.origin + f.u * s + f.v * tt;
        r = r.max((p.x - cx).hypot(p.z - cz));
        let Some(id) = nav.nearest(p, 1.0, false) else {
            continue;
        };
        let nd = nav.nodes[id].pos;
        // JS L97: nowhere to stand near it
        if (nd.x - p.x).hypot(nd.z - p.z) > 2.2 || (nd.y - p.y).abs() > 1.8 {
            continue;
        }
        let li = match idx.iter().position(|&(a, _)| a == id) {
            Some(l) => l,
            None => {
                let l = nodes.len();
                idx.push((id, l));
                nodes.push(id);
                l
            }
        };
        cell_node[i] = li as i32;
    }
    let reach = cell_node.iter().filter(|&&v| v >= 0).count();
    // JS L103-108: neighbourhoods (≤ 2.3 m, same level) per local node.
    let nb: Vec<Vec<usize>> = nodes
        .iter()
        .map(|&id| {
            let a = nav.nodes[id].pos;
            nodes
                .iter()
                .enumerate()
                .filter(|(_, jd)| {
                    let b = nav.nodes[**jd].pos;
                    (a.x - b.x).abs() < 2.4
                        && (a.z - b.z).abs() < 2.4
                        && (a.y - b.y).abs() < 1.0
                        && (a.x - b.x).hypot(a.z - b.z) <= 2.3
                })
                .map(|(j, _)| j)
                .collect()
        })
        .collect();
    // JS L109-115: ring (on / by the zone) and far (3-18 m out) nav nodes,
    // skipping the spawn-barrier nodes (`q.zone >= 0`).
    let mut ring: Vec<usize> = Vec::new();
    let mut far: Vec<usize> = Vec::new();
    for &id in &nav.valid_ids {
        let q = &nav.nodes[id];
        if q.zone >= 0 {
            continue;
        }
        let d = (q.pos.x - cx).hypot(q.pos.z - cz);
        if d <= r + 16.0 {
            ring.push(id);
        }
        if d >= r + 3.0 && d <= r + 18.0 {
            far.push(id);
        }
    }
    // JS L118-131: staging spots — a patch of ground 3-9 m outside the zone
    // on each team's own side (toward its spawn).
    let stage: [Option<StageSpot>; 2] = [0, 1].map(|t| {
        let p = world.spawn_pads[t];
        let ex = p.x - cx;
        let ez = p.z - cz;
        // JS L120: `Math.hypot(ex, ez) || 1` — only the exact-zero case
        // substitutes 1 (a sub-1 m pad offset keeps its true length).
        let el = ex.hypot(ez);
        let el = if el == 0.0 { 1.0 } else { el };
        let mut best: Option<usize> = None;
        let mut bs = f32::NEG_INFINITY;
        for &id in &ring {
            let q = &nav.nodes[id];
            let dx = q.pos.x - cx;
            let dz = q.pos.z - cz;
            let d = dx.hypot(dz);
            if d < r + 3.0 || d > r + 9.0 || q.wet == 2 {
                continue;
            }
            let sc = ((dx * ex + dz * ez) / (d * el)) * 3.0
                - (d - (r + 5.0)).abs() * 0.3
                - (q.pos.y - cy).abs() * 0.35
                - if q.wet != 0 { 1.0 } else { 0.0 };
            if sc > bs {
                bs = sc;
                best = Some(id);
            }
        }
        let best = best?;
        let b = nav.nodes[best].pos;
        let mut spots: Vec<usize> = ring
            .iter()
            .copied()
            .filter(|&id| {
                let q = &nav.nodes[id];
                (q.pos.x - b.x).hypot(q.pos.z - b.z) < 3.2
                    && (q.pos.y - b.y).abs() < 1.2
                    && q.wet != 2
            })
            .collect();
        if spots.is_empty() {
            spots.push(best);
        }
        Some(StageSpot {
            x: b.x,
            y: b.y,
            z: b.z,
            spots,
        })
    });
    let nl = nodes.len();
    // JS L132: `far: far.length ? far : ring.slice()` reads the *original*
    // ring (the `nodes.slice()` fallback only lands on `ring` itself in the
    // same object literal), so an empty ring leaves `far` empty too —
    // unreachable on real maps, mirrored exactly.
    let far = if far.is_empty() { ring.clone() } else { far };
    let ring = if ring.is_empty() { nodes.clone() } else { ring };
    ZoneInfo {
        cell_node,
        ring,
        far,
        nodes,
        nb,
        reach,
        r,
        parts: z.def.parts().to_vec(),
        y0: z.def.y0.unwrap_or(-2.0),
        y1: z.def.y1.unwrap_or(6.0),
        stage,
        need: [vec![0.0; nl], vec![0.0; nl]],
        need_sum: [0.0, 0.0],
        ink: [vec![0; nl], vec![0; nl]],
    }
}
