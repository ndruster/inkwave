//! Blade kit: Brine Cutlass quick/heavy cuts and the charged ink wave
//! (M3 Task 32).
//!
//! Faithful port of the sim-side subset of `src/game/kits/blade.js`:
//!   - `st()` L38-44                       -> [`BladeKit::new`]
//!   - `cancelCharge` L49-53               -> [`cancel_charge`]
//!   - `tap` L72-86                        -> [`tap`]
//!   - `heavy` L87-102                     -> [`heavy`]
//!   - `lungeSafe` L103-111                -> [`lunge_safe`] (the bot ledge
//!     branch is restored by Task 47 batch D)
//!   - `runSwing` L114-127                 -> [`run_swing`]
//!   - `fireDrops` L131-178                -> [`fire_drops`]
//!   - `groundStroke` L181-194             -> [`ground_stroke`]
//!   - `melee` L199-242                    -> [`melee`] (boss L232-235 and the
//!     device `damageArea` L237-241 are not ported — no boss/devices layers)
//!   - `spawnWave` L247-258                -> [`spawn_wave`]
//!   - `addWave` L261-265                  -> [`BladeWave::new`]
//!   - `ghost` L267-270                    -> the `TickEvent::Kit` replay
//!     (`net/replication.rs`, Task 32 §7)
//!   - `endWave` L272-295                  -> [`BladeWave::end_wave`]
//!   - `updateWaves`/`stepWave` L296-372   -> [`BladeWave::step`] (reverse
//!     iteration lives in the match loop, like the launched canopies)
//!   - `MAIN_KITS.blade.update` L624-678   -> [`update`]
//!   - `busy` L681 / `moveSpeed` L683-689  -> `WeaponRunner::busy` /
//!     `WeaponRunner::move_speed` (weapon.rs)
//!
//! Intentional deviations from JS (recorded for review):
//!   - `cutYaw` (L65-71): the sim's `aimPoint` is always `None` (spec §1),
//!     so the cut heading is the aim yaw — `a.yaw` (the JS fallback branch).
//!   - `coreDriven` / `botHolds` (L74-78, L640-642): `auto` stays `false`
//!     (the sim has no climb-aim override); the `botHolds` fire rescue is
//!     restored by Task 47 batch D via [`Actor::bl_hold`].
//!   - `raw` (L630) reads the *ungated* intent fire flag — the sim passes
//!     the raw input through (spec D-T32-2).
//!   - Presentation state (`flashT`, `taps`, `heavies`, `sp`, hums, FX,
//!     audio, rumble, `fireFacing`, `a.character.trigger`) is dropped (D-7);
//!     the sim consumes no rng for it.
//!   - The special-credit split (`credit(a, area, sp)` L195) collapses to
//!     the plain turf credit — specials are Task 33.
//!   - JS `rollHits`-style per-swing victim sets key on actor objects; the
//!     sim keys [`Swing::hits`] / [`BladeWave::hits`] on roster indices.
//!   - Ghost waves (the remote replay) draw their rng values like the
//!     owner's but skip the rasterisation and every event — the JS `mute`
//!     wrapper (L299-301) suppresses both the turf credit and the net
//!     records; the owner's stripe arrives as its own paint records
//!     instead (same rule as `LaunchedCanopy::ghost`, Task 30).

use std::collections::BTreeSet;

use glam::{Vec3, vec3};

use crate::actor::{Actor, FireGate, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{Blade, PlayerTuning};

use super::{
    DEG, DropSpec, FoeScan, ProjectileSim, SimEvent, WeaponRunner, apply_hit, clamp_f, lerp,
};

/// Per-runner kit state (JS `st()` L38-44; the presentation fields
/// `flashT` / `taps` / `heavies` are dropped). `last_t` rides the projectile
/// stream's clock (JS `G.time`); a fresh kit starts at 0 — the first frame's
/// idle check only re-applies the default state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BladeKit {
    pub held: bool,
    pub hold_t: f32,
    pub charge: f32,
    pub charging: bool,
    pub full: bool,
    pub store: f32,
    pub cd: f32,
    pub queued: f32,
    /// JS `side: -1` — the *next* quick cut flips to `-side` (L80).
    pub side: i32,
    pub swing: Option<Swing>,
    pub lunge: f32,
    pub lx: f32,
    pub lz: f32,
    pub lv: f32,
    pub last_t: f32,
    pub since: f32,
    /// JS `K.lowWarned` (L663): the once-per-hold empty-ink warning.
    pub low_warned: bool,
}

impl Default for BladeKit {
    fn default() -> Self {
        Self::new()
    }
}

impl BladeKit {
    /// JS `st()` fresh-kit initialiser.
    #[must_use]
    pub fn new() -> Self {
        Self {
            held: false,
            hold_t: 0.0,
            charge: 0.0,
            charging: false,
            full: false,
            store: 0.0,
            cd: 0.0,
            queued: 0.0,
            side: -1,
            swing: None,
            lunge: 0.0,
            lx: 0.0,
            lz: 0.0,
            lv: 0.0,
            last_t: 0.0,
            since: 99.0,
            low_warned: false,
        }
    }
}

/// The swing in progress (JS `K.swing`, L82/L94; the `sp` special flag and
/// the one-shot `dev` device hit are not ported).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Swing {
    pub heavy: bool,
    pub t: f32,
    pub side: i32,
    pub yaw: f32,
    pub pitch: f32,
    /// Roster indices already cut by this swing (JS `hits: new Set()`).
    pub hits: BTreeSet<usize>,
    pub fired: bool,
    pub stroke: bool,
}

/// A charged ink wave crossing the arena (JS `addWave` record L263 without
/// the mesh/FX fields). World object: the match owns the list and steps it
/// reverse-order every frame, like the launched canopies.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BladeWave {
    /// Owner identity slot (JS `W.owner`; the match resolves it for turf).
    pub owner_slot: usize,
    pub team: usize,
    pub pos: Vec3,
    pub prev: Vec3,
    pub dir: Vec3,
    pub dist: f32,
    pub t: f32,
    /// Roster indices already pierced (JS `W.hits`).
    pub hits: BTreeSet<usize>,
    /// JS `paintAcc: 0.3` — pre-armed so the first stripe lands early.
    pub paint_acc: f32,
    /// `-1` while alive; the burst fade window (JS `dying: 0.14`).
    pub dying: f32,
    /// A remote player's wave replayed from the `['k', …]` record (JS
    /// `ghost` L267-270): it moves and dies for the eye only, never paints
    /// and never hurts.
    #[serde(default)]
    pub ghost: bool,
}

impl BladeWave {
    /// JS `addWave` (L261-265) without the mesh.
    #[must_use]
    pub fn new(owner_slot: usize, team: usize, pos: Vec3, dir: Vec3, ghost: bool) -> Self {
        Self {
            owner_slot,
            team,
            pos,
            prev: pos,
            dir,
            dist: 0.0,
            t: 0.0,
            hits: BTreeSet::new(),
            paint_acc: 0.3,
            dying: -1.0,
            ghost,
        }
    }

    /// JS `stepWave(W, i, dt)` (L304-372). Returns `false` when the wave is
    /// spent and the match must remove it (JS `WAVES.splice` L311).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        paint: &mut PaintGrid,
        w: &Blade,
        t: &PlayerTuning,
        ps: &mut ProjectileSim,
        actors: &mut [Actor],
        remote: Option<&[bool]>,
    ) -> bool {
        self.t += dt;
        if self.dying >= 0.0 {
            self.dying -= dt;
            return self.dying > 0.0;
        }
        let step = w.wave_speed * dt;
        self.prev = self.pos;
        self.pos += self.dir * step;
        self.dist += step;
        // JS L320-338: enemies inside the crescent's slab, each pierced once.
        let hx = self.dir.x;
        let hz = self.dir.z;
        // JS `Math.hypot(hx, hz) || 1`.
        let hl = {
            let v = hx.hypot(hz);
            if v == 0.0 { 1.0 } else { v }
        };
        let ux = hx / hl;
        let uz = hz / hl;
        let dmg = lerp(
            w.wave_damage,
            w.wave_damage_far,
            (self.dist / w.wave_range).clamp(0.0, 1.0),
        );
        let mut hits: Vec<usize> = Vec::new();
        for (si, e) in actors.iter().enumerate() {
            if e.team == self.team || !e.alive || self.hits.contains(&si) {
                continue;
            }
            let hr = t.radius;
            let h = if e.form == Form::Squid {
                t.squid_height
            } else {
                t.height
            };
            let ex = e.pos.x - self.prev.x;
            let ez = e.pos.z - self.prev.z;
            let along = ex * ux + ez * uz;
            if along < -hr || along > step * hl + hr {
                continue;
            }
            if (ex * uz - ez * ux).abs() > w.wave_width * 0.5 + hr {
                continue;
            }
            let cy = self.prev.y
                + (self.pos.y - self.prev.y) * (along / (step * hl).max(1e-4)).clamp(0.0, 1.0);
            let y0 = e.pos.y + e.smooth_y();
            if y0 + h < cy - w.wave_height * 0.55 || y0 > cy + w.wave_height * 0.45 {
                continue;
            }
            hits.push(si);
        }
        for si in hits {
            self.hits.insert(si);
            if self.ghost {
                continue;
            }
            let (victim_slot, victim_team) = (actors[si].slot, actors[si].team);
            let send = remote.is_some_and(|m| m.get(si).copied().unwrap_or(false));
            if send {
                ps.push_event(SimEvent::HitSend {
                    attacker_slot: self.owner_slot,
                    victim_slot,
                    victim_team,
                    team: self.team,
                    damage: dmg,
                });
            } else {
                let atk_pos = super::actor_pos_by_slot(actors, self.owner_slot);
                let outcome = apply_hit(
                    &mut actors[si],
                    dmg,
                    Some((self.team, self.owner_slot)),
                    atk_pos,
                    t,
                );
                ps.push_event(SimEvent::Hit {
                    attacker_slot: self.owner_slot,
                    victim_slot,
                    team: self.team,
                    damage: dmg,
                    killed: outcome.killed,
                });
            }
        }
        // JS L344-351: walls break it (the shield / device pre-passes are not
        // ported — no subs/specials layers, spec §5).
        let mut stop = false;
        let mut end: Option<(Vec3, Vec3)> = None;
        let hit = world.segment(self.prev, self.pos, true);
        if hit.hit {
            self.pos = hit.point;
            end = Some((hit.point, hit.normal));
            stop = true;
        }
        // JS L353-362: a straight stripe of ink along its path. The rng
        // draws (radius jitter, then seed) sit inside the `g.hit` gate — JS
        // evaluates them as `splat` arguments only after the raycast hits —
        // and a ghost still draws them (the `mute` wrapper suppresses the
        // splat's effect, not the calls).
        self.paint_acc += step;
        if !stop && self.paint_acc >= w.wave_paint_every {
            self.paint_acc = 0.0;
            let g = world.raycast(self.pos, -Vec3::Y, w.wave_height * 0.52 + 1.4, true);
            let (rad, seed) = if g.hit {
                (0.9 + ps.rng.next_f32() * 0.2, ps.rng.next_f32())
            } else {
                (0.0, 0.0)
            };
            if g.hit && !self.ghost {
                let area = paint.splat(
                    world,
                    g.point + g.normal * 0.08,
                    w.wave_paint_radius * rad,
                    self.team,
                    &SplatOpts {
                        seed,
                        stretch: Some(vec3(ux, 0.0, uz)),
                        stretch_amt: Some(0.85),
                        kind: None,
                    },
                );
                if area > 0.0 {
                    ps.push_event(SimEvent::Turf {
                        owner_slot: self.owner_slot,
                        team: self.team,
                        area,
                        no_special: false,
                    });
                }
            }
        }
        if !stop && self.dist >= w.wave_range {
            self.end_wave(None, world, paint, w, ps);
        }
        if let Some((at, normal)) = end {
            self.end_wave(Some((at, normal)), world, paint, w, ps);
        }
        // JS only splices a wave when its `dying` fade runs out (L311) —
        // a stopped wave lives on in the fade window.
        true
    }

    /// JS `endWave(W, at, normal)` (L272-295): start the burst fade, splat
    /// the end mark and credit the owner. `at = Some` is the wall burst (JS
    /// also emits `weapon:impact` — the [`SimEvent::Impact`] payload);
    /// `None` is the spent wave slumping into a ground spatter (JS credits
    /// without an impact emit — [`SimEvent::Turf`]).
    fn end_wave(
        &mut self,
        at: Option<(Vec3, Vec3)>,
        world: &CollisionWorld,
        paint: &mut PaintGrid,
        w: &Blade,
        ps: &mut ProjectileSim,
    ) {
        self.dying = 0.14;
        match at {
            Some((point, normal)) => {
                let seed = ps.rng.next_f32();
                if self.ghost {
                    return;
                }
                let area = paint.splat(
                    world,
                    point + normal * 0.1,
                    w.wave_end_radius,
                    self.team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                );
                ps.push_event(SimEvent::Impact {
                    pos: point,
                    normal,
                    team: self.team,
                    radius: w.wave_end_radius,
                    victim_slot: None,
                    owner_slot: self.owner_slot,
                    area,
                    no_special: false,
                });
            }
            None => {
                let g = world.raycast(self.pos, -Vec3::Y, 3.0, true);
                if !g.hit {
                    return;
                }
                let seed = ps.rng.next_f32();
                if self.ghost {
                    return;
                }
                let stretch = vec3(self.dir.x, 0.0, self.dir.z).normalize_or_zero();
                let area = paint.splat(
                    world,
                    g.point + g.normal * 0.1,
                    w.wave_end_radius * 0.85,
                    self.team,
                    &SplatOpts {
                        seed,
                        stretch: Some(stretch),
                        stretch_amt: Some(0.6),
                        kind: None,
                    },
                );
                if area > 0.0 {
                    ps.push_event(SimEvent::Turf {
                        owner_slot: self.owner_slot,
                        team: self.team,
                        area,
                        no_special: false,
                    });
                }
            }
        }
    }
}

/// JS `cancelCharge` (L49-53).
fn cancel_charge(r: &mut WeaponRunner, k: &mut BladeKit) {
    k.charging = false;
    k.charge = 0.0;
    k.full = false;
    k.store = 0.0;
    r.charge = 0.0;
}

/// JS `tap` (L72-86): the quick cut. `a.fireFacing` / audio / rumble are
/// presentation-only and dropped.
fn tap(r: &mut WeaponRunner, k: &mut BladeKit, w: &Blade, a: &mut Actor, ps: &mut ProjectileSim) {
    if a.ink < w.tap_ink {
        r.empty_click(ps, a);
        k.cd = 0.18; // JS L74 literal
        return;
    }
    a.ink -= w.tap_ink;
    a.note_fired();
    k.side = -k.side;
    k.cd = w.tap_interval;
    k.since = 0.0;
    k.swing = Some(Swing {
        heavy: false,
        t: 0.0,
        side: k.side,
        yaw: a.yaw, // `cutYaw` fallback (spec §1: sim `aimPoint` is None)
        pitch: a.aim_pitch,
        hits: BTreeSet::new(),
        fired: false,
        stroke: false,
    });
    r.firing_t = 0.4;
}

/// JS `lungeSafe` (L103-111): the overhead cut's short lunge probe (used by
/// `heavy` L96 and the bot tactics' cut-range test, L542).
pub(crate) fn lunge_safe(
    a: &Actor,
    fx: f32,
    fz: f32,
    w: &Blade,
    world: &CollisionWorld,
    t: &PlayerTuning,
) -> bool {
    // JS L103-111: three samples along the lunge must not stand in the sea,
    // and a bot never lunges off a ledge (Task 47 batch D — JS `a.bot &&
    // !a.isLocal`: every brain-driven actor is non-local in the sim). JS
    // `groundHeight(x, z, yMax)` has no grate flag — the sim passes
    // `skip_grates = false`.
    for k in [0.5_f32, 1.0, 1.4] {
        let gy = world.ground_height(
            a.pos.x + fx * w.lunge_dist * k,
            a.pos.z + fz * w.lunge_dist * k,
            a.pos.y + 0.6,
            false,
        );
        if gy <= t.water_y + 0.25 {
            return false;
        }
        if a.is_bot && gy < a.pos.y - 1.6 {
            return false;
        }
    }
    true
}

fn heavy(
    r: &mut WeaponRunner,
    k: &mut BladeKit,
    w: &Blade,
    a: &mut Actor,
    world: &CollisionWorld,
    t: &PlayerTuning,
) {
    a.ink = (a.ink - w.heavy_ink).max(0.0);
    a.note_fired();
    k.cd = w.heavy_interval;
    k.since = 0.0;
    k.swing = Some(Swing {
        heavy: true,
        t: 0.0,
        side: 1,
        yaw: a.yaw,
        pitch: a.aim_pitch,
        hits: BTreeSet::new(),
        fired: false,
        stroke: false,
    });
    r.firing_t = 0.5;
    let fx = a.yaw.sin();
    let fz = a.yaw.cos();
    k.lunge = 0.0;
    if a.grounded && lunge_safe(a, fx, fz, w, world, t) {
        k.lunge = w.lunge_time;
        k.lx = fx;
        k.lz = fz;
        k.lv = w.lunge_dist / (w.lunge_time * 0.75);
    }
}

/// JS `runSwing` (L114-127): the swing in progress.
#[allow(clippy::too_many_arguments)]
fn run_swing(
    r: &mut WeaponRunner,
    k: &mut BladeKit,
    sw: &mut Swing,
    dt: f32,
    w: &Blade,
    a: &mut Actor,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    paint: &mut PaintGrid,
    t: &PlayerTuning,
    remote: Option<&[bool]>,
) {
    sw.t += dt;
    if !sw.fired && sw.t >= if sw.heavy { 0.05 } else { 0.028 } {
        sw.fired = true;
        if sw.heavy {
            spawn_wave(r, sw, w, a, world, ps);
        } else {
            fire_drops(sw, w, a, world, ps);
        }
    }
    if sw.heavy && !sw.stroke && sw.t >= 0.075 {
        sw.stroke = true;
        ground_stroke(sw, w, a, world, paint, ps);
    }
    // JS L120-121: the melee window — a frame that crosses into it cuts.
    let t1 = if sw.heavy { 0.16 } else { 0.12 };
    if sw.t >= 0.01 && sw.t - dt <= t1 {
        melee(sw, w, a, world, ps, foes, t, remote);
    }
    if k.lunge > 0.0 {
        k.lunge -= dt;
        if k.lunge > 0.0 && a.alive {
            let v = k.lv * (0.55 + 0.45 * (k.lunge / (w.lunge_time * 0.5)).min(1.0));
            a.vel.x = k.lx * v;
            a.vel.z = k.lz * v;
        }
    }
}

/// JS `fireDrops` (L131-178): the crescent of droplets flung across the
/// sweep. One droplet per victim per cut rides the Task 31 volley group
/// (`volley = { hit: new Set() }` L137). RNG order per drop:
/// [speed, radius, seed, cu] — the `vis` draw (L152) is presentation-only
/// and not consumed (Task 29 rule).
fn fire_drops(sw: &Swing, w: &Blade, a: &Actor, world: &CollisionWorld, ps: &mut ProjectileSim) {
    let yaw = sw.yaw;
    let side = sw.side as f32;
    // JS L133: the *live* aim pitch, not the swing-captured one.
    let pitch = a.aim_pitch;
    let steep = pitch < -0.55;
    let up = if steep {
        pitch * 0.85
    } else {
        clamp_f(pitch, -0.4, 0.45) * 0.75 + 0.06
    };
    let n = w.tap_drops as usize;
    let group = ps.volley_next;
    ps.volley_next += 1;
    ps.volley_alive.insert(group, n);
    let arc = w.tap_spread_deg * DEG;
    for i in 0..n {
        let fi = i as f32;
        let t = if n > 1 { fi / (n as f32 - 1.0) } else { 0.5 };
        let off = side * (t - 0.5) * arc;
        let ang = yaw + off;
        let ox = ang.sin();
        let oz = ang.cos();
        let mid = 1.0 - (t - 0.5).abs() * 2.0;
        let sp = w.tap_speed
            * (0.8 + 0.24 * mid)
            * (0.95 + ps.rng.next_f32() * 0.1)
            * (if steep { 0.45 } else { 1.0 });
        let radius = w.tap_drop_radius * (0.85 + ps.rng.next_f32() * 0.3);
        let seed = ps.rng.next_f32();
        let cu = (up + (ps.rng.next_f32() - 0.5) * 0.05).cos();
        // JS L156-159: leaves from the blade's path (a diagonal cut); never
        // from inside a wall the kid is hugging.
        let r0 = if steep { 0.55 } else { 0.8 };
        let mut pos = vec3(
            a.pos.x + ox * r0,
            a.pos.y + (if steep { 0.7 } else { 0.98 }) + (0.5 - t) * 0.12,
            a.pos.z + oz * r0,
        );
        let body = vec3(a.pos.x, pos.y, a.pos.z);
        if !world.los(body, pos) {
            pos = body + vec3(ox, 0.0, oz) * 0.12;
        }
        let tx = -oz * side;
        let tz = ox * side;
        let hx = ox + tx * 0.12;
        let hz = oz + tz * 0.12;
        let hl = hx.hypot(hz);
        let vel = vec3(hx / hl * cu * sp, up.sin() * sp, hz / hl * cu * sp);
        ps.push_drop(
            a,
            &DropSpec {
                life: 1.2,
                straight: if steep { 0.0 } else { w.tap_straight },
                size: 0.13, // JS L149 literal
                radius,
                damage: w.tap_damage_near,
                dmg_far: w.tap_damage_far,
                trail_start: -0.2, // JS L149 `trail: -0.2`
                trail_every: 1.25,
                trail_radius: 0.3,
                grav: 24.0, // JS L150 literal
                drag: 0.5,
                delay: t * 0.018, // JS L151: sweep-order release
            },
            pos,
            vel,
            seed,
        );
        if let Some(p) = ps.list.last_mut() {
            p.volley = Some(group);
        }
    }
    ps.push_event(SimEvent::Fire {
        owner_slot: a.slot,
        team: a.team,
        muzzle: vec3(
            a.pos.x + yaw.sin() * 0.8,
            a.pos.y + 0.98,
            a.pos.z + yaw.cos() * 0.8,
        ),
        dir: vec3(yaw.sin(), up.sin(), yaw.cos()).normalize_or_zero(),
        charge: None,
        len: None,
    });
}

/// JS `groundStroke` (L181-194): the blade meets the ground in front at the
/// bottom of the overhead cut. JS credits once (L193) — one [`SimEvent::Turf`]
/// with the summed area (no `weapon:impact` emit in JS).
fn ground_stroke(
    sw: &Swing,
    w: &Blade,
    a: &Actor,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    ps: &mut ProjectileSim,
) {
    let fx = sw.yaw.sin();
    let fz = sw.yaw.cos();
    let stretch = vec3(fx, 0.0, fz);
    let mut area = 0.0;
    for d in [0.75_f32, 1.35] {
        let g = world.raycast(
            vec3(a.pos.x + fx * d, a.pos.y + 0.8, a.pos.z + fz * d),
            -Vec3::Y,
            2.2,
            true,
        );
        if !g.hit {
            continue;
        }
        let seed = ps.rng.next_f32();
        area += paint.splat(
            world,
            g.point + g.normal * 0.08,
            w.stroke_radius,
            a.team,
            &SplatOpts {
                seed,
                stretch: Some(stretch),
                stretch_amt: Some(0.9),
                kind: None,
            },
        );
    }
    if area > 0.0 {
        ps.push_event(SimEvent::Turf {
            owner_slot: a.slot,
            team: a.team,
            area,
            no_special: false,
        });
    }
}

/// JS `melee` (L199-242): the blade as a hitbox. Scan first, apply after
/// (the FoeScan borrow shape, same as the roller / brush melee). The boss
/// (L232-235) and device `damageArea` (L237-241) branches are not ported.
#[allow(clippy::too_many_arguments)]
fn melee(
    sw: &mut Swing,
    w: &Blade,
    a: &Actor,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    t: &PlayerTuning,
    remote: Option<&[bool]>,
) {
    let half = (if sw.heavy {
        w.heavy_arc_deg
    } else {
        w.melee_arc_deg
    }) * DEG
        * 0.5;
    let fx = sw.yaw.sin();
    let fz = sw.yaw.cos();
    let dmg = if sw.heavy { w.heavy_melee } else { w.tap_melee };
    let o = vec3(a.pos.x, a.pos.y + 1.0, a.pos.z);
    let mut hits: Vec<usize> = Vec::new();
    for (ei, e) in foes.iter() {
        if e.team == a.team || !e.alive || sw.hits.contains(&ei) {
            continue;
        }
        let hr = t.radius;
        let h = if e.form == Form::Squid {
            t.squid_height
        } else {
            t.height
        };
        let dx = e.pos.x - a.pos.x;
        let dz = e.pos.z - a.pos.z;
        let d = dx.hypot(dz);
        if d > w.reach + hr {
            continue;
        }
        let ey = e.pos.y + e.smooth_y() - a.pos.y;
        if ey > (if sw.heavy { 1.9 } else { 1.55 })
            || ey + h < (if sw.heavy { -0.9 } else { -0.35 })
        {
            continue;
        }
        let fwd = dx * fx + dz * fz;
        let lat = (dx * fz - dz * fx).abs();
        let ang = lat.atan2(fwd);
        if ang > half && !(d < hr + 0.4 && fwd > -0.15) {
            continue;
        }
        let target = vec3(
            e.pos.x,
            e.pos.y + e.smooth_y() + (h * 0.6).min(0.85),
            e.pos.z,
        );
        if !world.los(o, target) {
            continue;
        }
        hits.push(ei);
    }
    let (a_slot, a_team, a_pos) = (a.slot, a.team, a.pos);
    for ei in hits {
        sw.hits.insert(ei);
        let Some(v) = foes.get_mut(ei) else {
            continue;
        };
        let (victim_slot, victim_team) = (v.slot, v.team);
        let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
        if send {
            ps.push_event(SimEvent::HitSend {
                attacker_slot: a_slot,
                victim_slot,
                victim_team,
                team: a_team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(v, dmg, Some((a_team, a_slot)), Some(a_pos), t);
            ps.push_event(SimEvent::Hit {
                attacker_slot: a_slot,
                victim_slot,
                team: a_team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }
}

/// JS `spawnWave` (L247-258): the charged wave leaves the blade. The world
/// handoff rides [`WeaponRunner::blade_wave`] (the match takes it into its
/// registry, like the brolly launch); the remote replay rides the
/// [`SimEvent::Kit`] channel with the JS `r2`/`r3` quantisation.
fn spawn_wave(
    r: &mut WeaponRunner,
    sw: &Swing,
    w: &Blade,
    a: &Actor,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
) {
    let pitch = if sw.pitch.abs() < 0.3 {
        sw.pitch * 0.4
    } else {
        clamp_f(sw.pitch, -0.45, 0.35)
    };
    let cp = pitch.cos();
    let dir = vec3(sw.yaw.sin() * cp, pitch.sin(), sw.yaw.cos() * cp);
    let mut pos = vec3(
        a.pos.x + sw.yaw.sin() * 0.55,
        a.pos.y + w.wave_height * 0.52,
        a.pos.z + sw.yaw.cos() * 0.55,
    );
    // JS L252-254: starting inside a wall (hugging cover) — born at the body.
    let body = vec3(a.pos.x, pos.y, a.pos.z);
    if !world.los(body, pos) {
        pos = body;
    }
    r.blade_wave = Some(BladeWave::new(a.slot, a.team, pos, dir, false));
    let q2 = |v: f32| (v * 100.0).round() / 100.0;
    let q3 = |v: f32| (v * 1000.0).round() / 1000.0;
    ps.push_event(SimEvent::Kit {
        owner_slot: a.slot,
        team: a.team,
        kind: "blade".to_string(),
        data: vec![
            q2(pos.x),
            q2(pos.y),
            q2(pos.z),
            q3(dir.x),
            q3(dir.y),
            q3(dir.z),
        ],
    });
    ps.push_event(SimEvent::Fire {
        owner_slot: a.slot,
        team: a.team,
        muzzle: pos,
        dir,
        charge: None,
        len: None,
    });
}

/// JS `MAIN_KITS.blade.update` (L624-678). `intent_fire` is the raw
/// (ungated) trigger flag (JS `a.intent.fire` L630, spec D-T32-2); `gate`
/// carries the emerge-suppressed `fire` / `pressed` the JS `winp` mirrors.
/// `ps_time` is the projectile stream clock (JS `G.time`).
#[allow(clippy::too_many_arguments)]
pub fn update(
    r: &mut WeaponRunner,
    dt: f32,
    gate: &FireGate,
    intent_fire: bool,
    a: &mut Actor,
    w: &Blade,
    world: &CollisionWorld,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    paint: &mut PaintGrid,
    t: &PlayerTuning,
    remote: Option<&[bool]>,
    ps_time: f32,
) {
    let mut k = r.blade.take().map(|b| *b).unwrap_or_default();
    // JS L626: resumed after a special took the trigger — an idle gap
    // drops the whole held state.
    if ps_time - k.last_t > 0.3 {
        cancel_charge(r, &mut k);
        k.held = false;
        k.swing = None;
        k.lunge = 0.0;
        k.queued = 0.0;
    }
    k.last_t = ps_time;
    k.cd -= dt;
    k.since += dt;
    k.queued = (k.queued - dt).max(0.0);
    if let Some(mut sw) = k.swing.take() {
        run_swing(
            r, &mut k, &mut sw, dt, w, a, world, ps, foes, paint, t, remote,
        );
        if sw.t <= 0.4 {
            k.swing = Some(sw);
        }
    }
    let raw = intent_fire;
    let can_act = a.form != Form::Squid && a.kid_t() >= t.emerge_delay && a.alive;
    if !can_act {
        // JS L632-638: squid / emerging — a charge keeps while the trigger
        // stays held (briefly), and is lost once it's let go.
        if k.charging {
            k.store += dt;
            a.note_fired();
            if !raw || k.store > w.charge_store {
                cancel_charge(r, &mut k);
            }
        }
        k.held = k.held && raw;
        r.charge = k.charge;
        r.blade = Some(Box::new(k));
        return;
    }
    k.store = 0.0;
    let mut fire = gate.fire;
    // JS L642: a bot holding a charge keeps the trigger alive even when the
    // emerge gate dropped it this frame (`botHolds`: `a.bot._blHold > G.time
    // - 0.3`, Task 47 batch D). The `coreDriven` auto branch (L640) stays
    // false — the sim's climb / refill override never drives the trigger.
    if !fire && k.charging && a.bl_hold > ps_time - 0.3 {
        fire = true;
    }
    // JS L644: press edge (the `coreDriven` auto branch is Task 39 — spec
    // D-T32-3).
    if gate.pressed {
        if k.cd <= 0.0 {
            tap(r, &mut k, w, a, ps);
        } else {
            k.queued = 0.16; // JS literal
        }
        k.held = true;
        k.hold_t = 0.0;
    } else if k.queued > 0.0 && k.cd <= 0.0 && !k.charging {
        k.queued = 0.0;
        tap(r, &mut k, w, a, ps);
    }
    if fire && k.held {
        k.hold_t += dt;
        // JS L651-654: held past the cut, the blade rises and charges.
        if !k.charging && k.hold_t >= w.charge_delay && !k.swing.as_ref().is_some_and(|s| s.heavy) {
            k.charging = true;
            k.charge = 0.0;
            k.full = false;
        }
        if k.charging {
            // JS L656: not enough ink for the heavy cut — it never completes.
            let cap: f32 = if a.ink >= w.heavy_ink { 1.0 } else { 0.94 };
            k.charge = cap.min(k.charge + dt / w.charge_time);
            if k.charge >= 1.0 && !k.full {
                k.full = true;
            }
            if cap < 1.0 && k.charge >= cap && !k.low_warned {
                k.low_warned = true;
                r.empty_click(ps, a);
            }
            // JS L665: `fireFacing` is presentation; `lastFire = 0` keeps
            // the idle refill away mid-charge.
            r.firing_t = r.firing_t.max(0.3);
            a.note_fired();
        }
    } else if k.held && !fire {
        // JS L667-676: release.
        k.held = false;
        k.low_warned = false;
        if k.charging {
            let c = k.charge;
            cancel_charge(r, &mut k);
            if c >= 1.0 {
                heavy(r, &mut k, w, a, world, t);
            } else if c >= 0.3 {
                if k.cd <= 0.04 {
                    k.cd = 0.0;
                    tap(r, &mut k, w, a, ps);
                } else {
                    k.queued = 0.16;
                }
            }
        }
    }
    // JS L677: the HUD meter mirrors the kit charge (the runner's own
    // `charging` flag deliberately stays false — a charge is not busy).
    r.charge = k.charge;
    r.blade = Some(Box::new(k));
}
