//! Brolly kit: pellet-blast shotgun with a deployable canopy shield
//! (M3 Task 30).
//!
//! Faithful port of the sim-side subset of `src/game/kits/brolly.js`:
//!   - `kitOf` L48-61                    -> [`BrollyKit::new`]
//!   - `pelletDamage` L64-68             -> [`super::PelletMark::pellet_damage`]
//!   - `fireBlast` L79-119               -> [`fire_blast`]
//!   - `update` L122-159                 -> [`update`] (trigger half)
//!   - `tick` runner maintenance L393-411 -> [`update`] (maintenance tail)
//!   - `heldPose` / `refreshShields` L172-190 -> [`held_shield`]
//!   - `hitShield` / `breakShield` L203-239  -> [`hit_kit`]
//!   - `launch` / `addLaunched` L258-288     -> [`launch`] / [`LaunchedCanopy::new`]
//!   - `updateLaunched` L306-367         -> [`LaunchedCanopy::update`]
//!   - `blockActors` L369-386            -> [`LaunchedCanopy::block_actors`]
//!   - `netState` / `netApply` L636-649  -> [`super::WeaponRunner::brolly_net_state`] /
//!     [`super::WeaponRunner::net_apply_brolly`]
//!
//! Intentional deviations from JS (recorded for review):
//!   - The bot-hold branch (JS L128/L135 `bb`) is dropped — bot brolly logic
//!     is Task 39; `shieldMode == canopy` for now.
//!   - Presentation-only state (`flash`, `shake`, `sndT`, `fxT`, mesh / FX /
//!     audio / rumble calls, the per-pellet `vis` jitter draw) is dropped
//!     (spec D-7); the sim consumes no rng for it.
//!   - `G.time` -> [`super::ProjectileSim::time`] as the kit clock.
//!   - The `tick` pre-scans (shield block L426-438, pellet scoring L443-476)
//!     run inside the core [`super::ProjectileSim::step_one`] pass instead of
//!     as a separate pre-pass: `predict` (L249-255) replays the exact
//!     integrator, so the swept segments and hit points are identical.
//!   - The launched canopy's turf credit (JS `o.addTurf(area)` L337) rides the
//!     return value to the match layer.

use std::sync::OnceLock;

use glam::{Vec3, vec3};

use crate::actor::{Actor, FireGate, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatKind, SplatOpts};
use crate::tuning::{Brolly, PlayerTuning, WeaponSet};

use super::{
    DEG, EMPTY_CD, FIRING_T, PelletMark, ProjectileSim, Rng, RoundSpec, SimEvent, WeaponRunner,
    clamp_f,
};

/// Held-canopy shield disc: centre offsets and radius (brolly.js L41).
const HELD_UP: f32 = 0.9;
const HELD_FWD: f32 = 0.62;
/// Held-canopy shield radius (JS `HELD.r` L41).
pub const HELD_R: f32 = 0.8;
/// Aim-pitch clamp of the held pose (JS `HELD.pitchMin/pitchMax` L41).
const PITCH_MIN: f32 = -0.5;
const PITCH_MAX: f32 = 0.45;
/// Launched-canopy crown height and blocking radius (JS L43).
const LAUNCH_H: f32 = 0.76;
/// Launched-canopy blocking radius (JS `LAUNCH_R` L43).
pub const LAUNCH_R: f32 = 0.8;

/// Dome depth (crown → rim plane), `brolly-model.js` L23-26:
/// `RIM_R / sin(52°) * (1 − cos(52°))` ≈ 0.3414. The launched blocking
/// plane sits half this behind the crown (JS L343, L378).
#[must_use]
pub fn canopy_depth() -> f32 {
    let th = 52.0_f32 * DEG;
    (0.7 / th.sin()) * (1.0 - th.cos())
}

/// Pellet cone pattern (JS `PATTERN` L75-77): centre, an inner ring of 4 at
/// 0.42 (phase-offset 0.4), an outer ring of 6 at 0.9.
fn pattern() -> &'static [(f32, f32)] {
    static P: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    P.get_or_init(|| {
        let mut v = vec![(0.0_f32, 0.0_f32)];
        for i in 0..4 {
            v.push((0.42, (i as f32 / 4.0) * std::f32::consts::TAU + 0.4));
        }
        for i in 0..6 {
            v.push((0.9, (i as f32 / 6.0) * std::f32::consts::TAU));
        }
        v
    })
}

/// Canopy lifecycle (JS `k.state`: `'ready' / 'broken' / 'launched'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CanopyState {
    /// Intact (or regrowing into intact).
    Ready,
    /// Shot apart; regrows after `regrowTime`.
    Broken,
    /// Detached as a launched wall; regrows after `regrowTime`.
    Launched,
}

/// Per-runner kit state (JS `runner.kit`, `kitOf` L48-61; the shield pose
/// fields mirror `k.sh` L56).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrollyKit {
    pub hp: f32,
    pub hp_max: f32,
    pub state: CanopyState,
    pub regrow_t: f32,
    pub regrow_k: f32,
    pub grow: f32,
    pub open: f32,
    pub opening: bool,
    pub want: bool,
    pub press_t: f32,
    pub open_t: f32,
    pub launch_k: f32,
    pub shot: bool,
    pub prev_fire: bool,
    pub lock_release: bool,
    pub last_hit: f32,
    pub last_upd: f32,
    /// Held-shield pose cache (JS `k.sh.C/N/Cp/Np/prev`).
    pub shield_c: Vec3,
    pub shield_n: Vec3,
    pub shield_cp: Vec3,
    pub shield_np: Vec3,
    pub shield_prev: bool,
}

impl BrollyKit {
    /// JS `kitOf` fresh-kit initialiser (L52-57).
    #[must_use]
    pub fn new(hp_max: f32) -> Self {
        Self {
            hp: hp_max,
            hp_max,
            state: CanopyState::Ready,
            regrow_t: 0.0,
            regrow_k: 0.0,
            grow: 1.0,
            open: 0.0,
            opening: false,
            want: false,
            press_t: 0.0,
            open_t: 0.0,
            launch_k: 0.0,
            shot: false,
            prev_fire: false,
            lock_release: false,
            last_hit: -9.0,
            last_upd: 0.0,
            shield_c: Vec3::ZERO,
            shield_n: Vec3::ZERO,
            shield_cp: Vec3::ZERO,
            shield_np: Vec3::ZERO,
            shield_prev: false,
        }
    }
}

/// A launched canopy sliding the ground (JS `addLaunched` record L276-288,
/// mesh/FX fields dropped; the shield half mirrors `c.sh` L283).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchedCanopy {
    pub team: usize,
    /// Owner identity slot (JS `c.owner`; the match resolves it for turf).
    pub slot: usize,
    pub pos: Vec3,
    pub dir: Vec3,
    pub vy: f32,
    pub t: f32,
    pub life: f32,
    pub hp: f32,
    pub hp_max: f32,
    pub h0: f32,
    pub shoving: bool,
    pub alive: bool,
    pub fade: f32,
    /// JS `c.ghost` (brolly.js L280): a remote player's canopy replayed from
    /// the `['k', …]` record (`ghost(a, d)`, L650-653). It slides, blocks
    /// shots and holds players back like the owner's, but never paints —
    /// JS wraps its step in `nm.mute++/--` (L416-419) so the splat call
    /// returns 0 (paint.js L470); the owner's stripe arrives as its own `'s'`
    /// records instead.
    #[serde(default)]
    pub ghost: bool,
    pub paint_acc: f32,
    pub last_hit: f32,
    /// This frame's shield pose (JS `c.sh.C/N`), snapshotted to `shield_*`
    /// at the end of the frame (JS `snapShields` L192).
    pub cur_c: Vec3,
    pub cur_n: Vec3,
    pub shield_cp: Vec3,
    pub shield_np: Vec3,
    pub shield_prev: bool,
}

impl LaunchedCanopy {
    /// JS `addLaunched` (L276-288) without the mesh.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        team: usize,
        slot: usize,
        pos: Vec3,
        dir: Vec3,
        hp: f32,
        hp_max: f32,
        life: f32,
        h0: f32,
        ghost: bool,
    ) -> Self {
        Self {
            team,
            slot,
            pos,
            dir,
            vy: 0.0,
            t: 0.0,
            life,
            hp,
            hp_max,
            h0,
            shoving: false,
            alive: true,
            fade: 0.0,
            ghost,
            paint_acc: 0.0,
            last_hit: -9.0,
            cur_c: Vec3::ZERO,
            cur_n: dir,
            shield_cp: Vec3::ZERO,
            shield_np: dir,
            shield_prev: false,
        }
    }

    /// JS `updateLaunched(c, dt)` (L306-367): slide, ground follow, sea death,
    /// the ink stripe and the blocking-plane pose. Returns the m² the stripe
    /// claimed and whether the canopy is still live (`false` = removed,
    /// JS L313/L420).
    #[must_use]
    pub fn update(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        bw: &Brolly,
        rng: &mut Rng,
    ) -> (f32, bool) {
        self.t += dt;
        if self.alive && self.t >= self.life {
            self.alive = false;
        }
        let mut area = 0.0;
        if !self.alive {
            // JS L310-313: fade out (collapse fast when shot apart).
            self.fade += dt / if self.hp <= 0.0 { 0.18 } else { 0.45 };
        } else {
            // JS L315-317: slide eases in, slows while shoving an enemy.
            let sp = bw.launch_speed
                * (if self.shoving { 0.6 } else { 1.0 })
                * (0.35 + self.t * 3.0).min(1.0);
            self.shoving = false;
            let step = sp * dt;
            let mut blocked = false;
            for hgt in [0.3_f32, 1.0] {
                if world
                    .raycast(self.pos + vec3(0.0, hgt, 0.0), self.dir, 0.12 + step, true)
                    .hit
                {
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                self.pos += self.dir * step;
                // JS L327-329: step up small lips, follow slopes down, fall
                // off edges (gravity 24 like the bombs, L329).
                let g = world.raycast(
                    vec3(self.pos.x, self.pos.y + 0.6, self.pos.z),
                    -Vec3::Y,
                    1.5,
                    true,
                );
                if g.hit
                    && g.point.y > self.pos.y - 0.12 - (0.0 - self.vy).max(0.0) * dt - 0.02
                    && self.vy <= 0.01
                {
                    self.pos.y = g.point.y;
                    self.vy = 0.0;
                } else {
                    self.vy -= 24.0 * dt;
                    self.pos.y += self.vy * dt;
                    if g.hit && self.pos.y < g.point.y {
                        self.pos.y = g.point.y;
                        self.vy = 0.0;
                    }
                }
                // JS L330: rolling into the sea destroys it outright.
                if self.pos.y < t.water_y - 0.4 {
                    self.alive = false;
                    self.hp = 0.0;
                }
                // JS L332-338: the ink stripe, every 0.32 m while grounded.
                // A ghost canopy (JS L416-419 `nm.mute++`) still consumes
                // the seed — JS evaluates `Math.random()` into the opts
                // before `splat` returns 0 under mute (paint.js L470) — but
                // paints nothing: the owner's stripe arrives as `'s'`
                // records instead.
                self.paint_acc += step;
                if self.paint_acc >= 0.32 && self.vy == 0.0 {
                    self.paint_acc = 0.0;
                    let seed = rng.next_f32();
                    if !self.ghost {
                        area = paint.splat(
                            world,
                            vec3(self.pos.x, self.pos.y + 0.3, self.pos.z),
                            bw.launch_paint,
                            self.team,
                            &SplatOpts {
                                seed,
                                stretch: Some(self.dir),
                                stretch_amt: None,
                                kind: Some(SplatKind::Roll),
                            },
                        );
                    }
                }
            }
        }
        // JS L342-344: the blocking plane runs even while fading.
        let h_now = LAUNCH_H + (self.h0 - LAUNCH_H) * (1.0 - self.t / 0.22).max(0.0);
        self.cur_c =
            vec3(self.pos.x, self.pos.y + h_now, self.pos.z) - self.dir * (canopy_depth() * 0.5);
        self.cur_n = self.dir;
        (area, self.fade < 1.0)
    }

    /// JS `blockActors(c)` (L369-386): enemy players can't pass the wall —
    /// pushed back out to the side they're on; allies walk through. `radius`
    /// is `PLAYER.radius` (JS L371/L377).
    pub fn block_actors(&mut self, actors: &mut [Actor], radius: f32) {
        if !self.alive {
            return;
        }
        let n = self.dir;
        let (tx, tz) = (-n.z, n.x);
        let rr = radius + 0.18;
        let half_depth = canopy_depth() * 0.5;
        for e in actors.iter_mut() {
            if !e.alive || e.team == self.team {
                continue;
            }
            let dx = e.pos.x - self.pos.x;
            let dz = e.pos.z - self.pos.z;
            let dy = e.pos.y - self.pos.y;
            if dy < -1.2 || dy > 1.6 {
                continue;
            }
            let along = dx * tx + dz * tz;
            if along.abs() > LAUNCH_R + radius * 0.6 {
                continue;
            }
            let across = dx * n.x + dz * n.z + half_depth;
            if across.abs() >= rr {
                continue;
            }
            let side = if across >= 0.0 { 1.0 } else { -1.0 };
            let push = side * rr - across;
            e.pos.x += n.x * push;
            e.pos.z += n.z * push;
            let vn = e.vel.x * n.x + e.vel.z * n.z;
            if vn * side < 0.0 {
                e.vel.x -= n.x * vn;
                e.vel.z -= n.z * vn;
            }
            if side > 0.0 {
                // JS L384: shoving an enemy along slows the slide next frame.
                self.shoving = true;
            }
        }
    }
}

/// JS `update(r, dt, inp, w)` (L122-159) + the `tick` runner maintenance
/// (L393-411). The bot-hold branch (L128) is dropped (Task 39), so
/// `shieldMode == canopy`.
#[allow(clippy::too_many_arguments)]
pub fn update(
    r: &mut WeaponRunner,
    dt: f32,
    gate: &FireGate,
    a: &mut Actor,
    w: &WeaponSet,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
) {
    let bw = &w.brolly;
    // JS `kitOf(r)`: the sim creates the kit lazily on first use; `take`
    // frees the runner borrow while the trigger logic runs.
    let mut k = r
        .brolly
        .take()
        .unwrap_or_else(|| Box::new(BrollyKit::new(bw.canopy_hp as f32)));
    let time = ps.time;
    // JS L397: the maintenance idle-close keys off the *previous* stamp —
    // true when the runner did not update for > 0.1 s (dead actor).
    let stale = time - k.last_upd > 0.1;
    k.last_upd = time;
    let fire = gate.fire;
    let press = fire && !k.prev_fire;
    k.prev_fire = fire;
    if press {
        k.press_t = 0.0;
        k.shot = false;
    }
    if fire {
        k.press_t += dt;
    } else {
        k.lock_release = false;
    }
    let canopy = k.state == CanopyState::Ready && k.grow > 0.6 && !k.lock_release;
    let shield_mode = canopy;
    let mut want = false;
    if fire && !k.lock_release {
        if shield_mode {
            // JS L140: tap = one blast; hold past `openDelay` = unfold.
            if !k.shot && r.cooldown <= 0.0 && k.open < 0.05 && k.press_t <= bw.open_delay + 0.06 {
                fire_blast(r, a, bw, ps);
                k.shot = true;
            }
            want = k.press_t >= bw.open_delay;
        } else if r.cooldown <= 0.0 {
            // JS L142: no canopy (broken / launched) — repeat blasts.
            fire_blast(r, a, bw, ps);
        }
    }
    if !fire && r.cooldown < 0.0 {
        r.cooldown = 0.0;
    }
    // JS L146-148: unfold / fold.
    let was = k.open;
    if want {
        k.open = (k.open + dt / bw.open_time).min(1.0);
    } else {
        k.open = (k.open - dt / bw.close_time).max(0.0);
    }
    k.opening = k.open > was || (k.open >= 1.0 && k.opening);
    // JS L152: an open canopy holds the firing pose (fireFacing dropped, D-7).
    if k.open > 0.0 {
        a.note_fired();
        r.firing_t = r.firing_t.max(0.2);
    }
    // JS L154-157: held fully open → launch.
    if k.open >= 1.0 && want {
        k.open_t += dt;
        if k.open_t >= bw.launch_hold {
            launch(r, &mut k, a, bw, world, ps);
        }
    } else {
        k.open_t = 0.0;
    }
    k.launch_k = if k.state == CanopyState::Ready {
        clamp_f(k.open_t / bw.launch_hold, 0.0, 1.0)
    } else {
        0.0
    };
    k.want = want;

    // ---- JS `tick` maintenance (L397-411): idle-close, regrow, canopy regen.
    if stale || !a.alive || a.form != Form::Kid {
        k.want = false;
        k.open = (k.open - dt / bw.close_time).max(0.0);
        k.open_t = 0.0;
    }
    if k.state != CanopyState::Ready {
        k.regrow_t -= dt;
        k.regrow_k = clamp_f(1.0 - k.regrow_t / bw.regrow_time, 0.0, 1.0);
        if k.regrow_t <= 0.0 {
            k.state = CanopyState::Ready;
            k.hp = k.hp_max;
            k.grow = 0.001;
            k.regrow_k = 1.0;
        }
    } else {
        if k.grow < 1.0 {
            k.grow = (k.grow + dt / 0.32).min(1.0); // JS L408 literal
        }
        if k.open <= 0.0 && time - k.last_hit > bw.canopy_regen_delay {
            k.hp = k.hp_max.min(k.hp + bw.canopy_regen * dt);
        }
    }
    r.brolly = Some(k);
}

/// JS `fireBlast(r, w)` (L79-119): the pellet cone. Returns whether the
/// blast actually fired (JS's `false` = dry).
fn fire_blast(r: &mut WeaponRunner, a: &mut Actor, bw: &Brolly, ps: &mut ProjectileSim) -> bool {
    if a.ink < bw.ink_per_shot {
        // JS L81: `_empty()` + a 0.2 s retry clamp.
        if r.empty_cd <= 0.0 {
            r.empty_cd = EMPTY_CD;
            ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
        }
        r.cooldown = r.cooldown.max(0.2);
        return false;
    }
    a.ink -= bw.ink_per_shot;
    a.note_fired(); // JS L82 `a.lastFire = 0`
    r.firing_t = FIRING_T; // JS L83
    let m = ProjectileSim::muzzle(a);
    let dir0 = ProjectileSim::aim_from(a, m, None);
    // JS L86 `_ballistic`: no crosshair world point in the sim → no
    // correction (same no-op path as `fire_round`'s `None` branch).
    let mut rr = dir0.cross(Vec3::Y); // JS L87
    if rr.length_squared() < 1e-6 {
        rr = Vec3::X;
    }
    rr = rr.normalize();
    let uu = rr.cross(dir0).normalize(); // JS L88
    let deg = if a.grounded {
        bw.spread_deg
    } else {
        bw.spread_air
    };
    let cone = (deg * DEG).tan();
    let turn = ps.rng.next_f32() * std::f32::consts::TAU; // JS L89
    let n = (bw.pellets as usize).min(pattern().len()); // JS L90
    ps.blast_seq += 1; // JS L91 `BLAST++`
    let blast = ps.blast_seq;
    for i in 0..n {
        let (pr, pa) = pattern()[i];
        // JS L93-96: pattern point + jitter, projected on the cone basis.
        let ang = pa + turn;
        let jx = (ps.rng.next_f32() - 0.5) * 0.18;
        let jy = (ps.rng.next_f32() - 0.5) * 0.18;
        let ox = (pr * ang.cos() + jx) * cone;
        let oy = (pr * ang.sin() + jy) * cone * 0.72;
        let d = (dir0 + rr * ox + uu * oy).normalize();
        // JS L99: one speed for the whole blast (± a hair); per-pellet paint.
        let speed = bw.proj_speed * (0.985 + ps.rng.next_f32() * 0.03);
        let radius = bw.pellet_paint * (0.85 + ps.rng.next_f32() * 0.3);
        // JS L101 drops the presentation `vis` jitter draw (D-7).
        let spec = RoundSpec {
            proj_speed: speed,
            straight_time: bw.straight_time,
            range: bw.range,
            grav: bw.pellet_grav,
            drag: bw.pellet_drag,
            size: bw.pellet_size,
            life: bw.pellet_life,
            impact_radius: radius,
            damage: 0.0,        // core pass never applies it; the kit scores (L99/L1608)
            trail_every: 0.0,   // JS L100 `trailEvery: 0`
            trail_radius: 0.42, // `fireCustom` default (L1204) — unused
            trail_start: -1.5,
        };
        // JS L103: `p.life = w.pelletLife` overrides the fireCustom default.
        ps.push_round(
            a,
            &spec,
            m,
            d,
            bw.pellet_life,
            false,
            Some(PelletMark {
                blast,
                falloff_start: bw.falloff_start,
                falloff_end: bw.falloff_end,
                dmg_near: bw.pellet_damage,
                dmg_far: bw.pellet_damage_far,
            }),
            false, // JS L113: ONE `weapon:fire` per blast, not per pellet
        );
    }
    ps.push_event(SimEvent::Fire {
        owner_slot: a.slot,
        team: a.team,
        muzzle: m,
        dir: dir0,
    });
    r.cooldown = bw.fire_interval; // JS L116
    true
}

/// JS `launch(r, k, w)` (L258-273): detach the canopy as a sliding wall.
fn launch(
    r: &mut WeaponRunner,
    k: &mut BrollyKit,
    a: &Actor,
    bw: &Brolly,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
) {
    let dir = vec3(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
    // JS L262: settle onto the ground just ahead of the owner.
    let origin = vec3(a.pos.x + dir.x * 0.9, a.pos.y + 1.2, a.pos.z + dir.z * 0.9);
    let g = world.raycast(origin, -Vec3::Y, 3.5, true);
    let pos = if g.hit {
        g.point
    } else {
        vec3(origin.x, a.pos.y, origin.z)
    };
    let hp = k.hp.max(bw.canopy_hp as f32 * 0.5); // JS L264
    let h0 = a.visual_y() + HELD_UP - pos.y;
    // JS L266 `netRec`: the wire payload with the r2/r3/round quantisation
    // (the net layer re-reads it from the event — Task 30 §4).
    let q2 = |v: f32| (v * 100.0).round() / 100.0;
    let q3 = |v: f32| (v * 1000.0).round() / 1000.0;
    ps.push_event(SimEvent::Kit {
        owner_slot: a.slot,
        team: a.team,
        kind: "brolly".to_string(),
        data: vec![
            q2(pos.x),
            q2(pos.y),
            q2(pos.z),
            q3(dir.x),
            q3(dir.z),
            hp.round(),
            q2(h0),
        ],
    });
    // JS L268: the held canopy is gone until it regrows.
    k.state = CanopyState::Launched;
    k.regrow_t = bw.regrow_time;
    k.regrow_k = 0.0;
    k.open = 0.0;
    k.grow = 0.0;
    k.open_t = 0.0;
    k.launch_k = 0.0;
    k.lock_release = true;
    // JS L265: the owner's own canopy (`ghost = false`); the remote replay
    // path (`MAIN_KITS.brolly.ghost`, L650-653) passes `true` (Task 30 §4).
    r.brolly_launch = Some(LaunchedCanopy::new(
        a.team,
        a.slot,
        pos,
        dir,
        hp,
        bw.canopy_hp as f32,
        bw.launch_life,
        h0,
        false,
    ));
}

/// JS `heldPose` + the `refreshShields` gate (L172-190): the held canopy
/// blocks only when it is a live kid's ready shield, open enough and grown
/// enough. Returns `(centre, normal)` for the frame's [`super::Shield`].
#[must_use]
pub fn held_shield(kit: &BrollyKit, a: &Actor) -> Option<(Vec3, Vec3)> {
    if !a.alive
        || a.form != Form::Kid
        || kit.state != CanopyState::Ready
        || kit.open < 0.55
        || kit.grow < 0.9
    {
        return None;
    }
    let p = clamp_f(a.aim_pitch, PITCH_MIN, PITCH_MAX);
    let cp = p.cos();
    let n = vec3(a.aim_yaw.sin() * cp, p.sin(), a.aim_yaw.cos() * cp);
    let c = vec3(a.pos.x, a.visual_y() + HELD_UP, a.pos.z) + n * HELD_FWD;
    Some((c, n))
}

/// JS `hitShield` + `breakShield` held branch (L203-239): a shot blocked by
/// a held canopy drains kit hp; at 0 the canopy breaks and regrows after
/// `regrow_time` (flash / shake / sound dropped, D-7).
pub fn hit_kit(k: &mut BrollyKit, dmg: f32, time: f32, regrow_time: f32) {
    // JS L205: `if (!s.hp || !(dmg > 0)) return` — hp already gone, or a
    // non-positive damage number (NaN never occurs in the sim).
    if k.hp <= 0.0 || dmg <= 0.0 {
        return;
    }
    k.hp -= dmg;
    k.last_hit = time;
    if k.hp <= 0.0 {
        k.hp = 0.0;
        k.state = CanopyState::Broken;
        k.regrow_t = regrow_time;
        k.regrow_k = 0.0;
        k.open = 0.0;
        k.grow = 0.0;
        k.open_t = 0.0;
        k.lock_release = true;
    }
}
