//! Bow kit: the Tideline Bow draw → three-arrow volley → lodged-arrow
//! bursts (M3 Task 33).
//!
//! Faithful port of the sim-side subset of `src/game/kits/bow.js`:
//!   - `kitOf` L40 (sim fields)            -> [`BowKit`]
//!   - `bowShot` L46-61                    -> [`bow_shot`]
//!   - `looseVolley` L74-93                -> [`loose_volley`]
//!   - `volley` L97-113                    -> [`volley`]
//!   - `ghost` L114-118                    -> the `TickEvent::Kit` replay
//!     (`net/replication.rs`, Task 33 §6)
//!   - `lodge` L122-132                    -> [`lodge`]
//!   - `burst` L135-162                    -> [`burst`] (devices L159 /
//!     boss L160 are not ported — no subs/specials/boss layers)
//!   - `stepArrow` L175-264                -> [`BowArrow::step`] (the boss
//!     sphere L216-224 and the device/special block L226-231 are not ported)
//!   - `update` L389-422                   -> [`update`]
//!   - `release` L423-435                  -> [`release`]
//!   - `MAIN_KITS.bow.tick` L506-521       -> [`sweep_stalled_draw`] +
//!     [`BowArrow::step`] driven from the match loop (Task 33 §5)
//!   - `MAIN_KITS.bow.moveSpeed` L500-504  -> `WeaponRunner::move_speed`
//!     (weapon.rs)
//!   - `MAIN_KITS.bow.reset` L499          -> `WeaponRunner::reset` (`bow: None`)
//!
//! Intentional deviations from JS (recorded for review):
//!   - The draw state (`charging` / `charge` / `chargeT`) lives on the
//!     *runner* exactly like JS (weapons.js L40-41 — the bow writes
//!     `runner.charging`, not a kit copy), so the `ch` tick slot mirrors it
//!     for free and `busy()` (L62) counts the draw as busy through the
//!     runner's own `charging` term. The kit only carries `ring` / `stall` /
//!     `lastC` / `seen`.
//!   - `a.aimPoint` is always `None` in the sim, so `_ballistic` (L78) keeps
//!     the aim line as-is — the same result JS produces when the correction
//!     can't resolve (spec D-T32 note, carried over).
//!   - The bot fan nudge (`botYaw`, L83-85), the bot draw/release policy
//!     (L417-420, `bot` L451-491) and the `predictLeap`-style planning are
//!     Task 39 (spec D-T33-1); the structure that reads them is kept where
//!     it costs nothing.
//!   - Presentation (`chargeLoop` audio, `recoil`, rumble, FX, `ticks`
//!     countdown L180-184, `fireFacing`) is dropped (D-7); the sim consumes
//!     no rng for it.
//!   - `credit(owner, area)` (L39) collapses to the plain turf credit — the
//!     `specialActive` split is Task 35.
//!   - The per-shot `Math.random()` arrow seeds ride [`ProjectileSim::rng`]
//!     in JS's draw order: 3 per volley (`volley` L106), 2 per trail drip
//!     (`stepArrow` L257), 0 elsewhere.

use glam::{Quat, Vec3, vec3};

use crate::actor::{Actor, FireGate, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{Bow, PlayerTuning};

use super::{
    DEG, ProjectileSim, SimEvent, WeaponRunner, apply_hit, clamp_f, lerp, segment_capsule_dist,
};

/// Arrows alive at once, flying + lodged (JS `MAX` L28).
pub const MAX_ARROWS: usize = 192;
/// Added to the victim's capsule radius on the arrow hit test (JS
/// `ARROW_R` L29).
pub const ARROW_R: f32 = 0.11;
/// A flying arrow gives up after this long, seconds (JS `LIFE` L31).
pub const ARROW_LIFE: f32 = 3.0;

/// Per-runner kit state (JS `kitOf` L40; the `bot*` fields are Task 39, the
/// draw itself lives on the runner — see module docs). `seen` rides the
/// projectile stream's clock (JS `G.time`); the stalled-draw sweep
/// (`tick` L514-519) reads it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BowKit {
    /// Rings announced: 0 none, 1 ring 1 reached, 2 full draw (JS `ring`;
    /// the `bow:ring` emits are HUD/audio-only, the field is kept for the
    /// state parity).
    pub ring: u8,
    /// JS `stall` — seconds the draw has stopped growing (bot release gate,
    /// Task 39; kept for state parity).
    pub stall: f32,
    /// JS `lastC` — last frame's charge (the stall test).
    pub last_c: f32,
    /// JS `kit.seen` — the last runner-update timestamp (L391).
    pub seen: f32,
}

impl Default for BowKit {
    fn default() -> Self {
        Self::new()
    }
}

impl BowKit {
    /// JS `kitOf` fresh-kit initialiser (the sim fields).
    #[must_use]
    pub fn new() -> Self {
        Self {
            ring: 0,
            stall: 0.0,
            last_c: 0.0,
            seen: 0.0,
        }
    }
}

/// The charge → shot-tier table (JS `bowShot` L46-61): tier 0 tap (< ring1,
/// no lodge), tier 1 ring (lodges + bursts), tier 2 full draw (top everything).
#[derive(Debug, Clone, Copy)]
pub struct BowShot {
    pub tier: u8,
    pub range: f32,
    pub speed: f32,
    pub dmg: f32,
    pub side: f32,
    pub fan: f32,
    pub lodge: bool,
    pub fuse: f32,
    pub br: f32,
    pub bd: f32,
    pub be: f32,
    pub bp: f32,
}

/// JS `bowShot(c)` (L46-61).
#[must_use]
pub fn bow_shot(c: f32, w: &Bow) -> BowShot {
    if c >= 0.999 {
        BowShot {
            tier: 2,
            range: w.flight_full,
            speed: w.speed_full,
            dmg: w.damage_full,
            side: w.side_full,
            fan: w.fan_full,
            lodge: true,
            fuse: w.fuse_full,
            br: w.burst_radius[1],
            bd: w.burst_damage[1],
            be: w.burst_edge[1],
            bp: w.burst_paint[1],
        }
    } else if c >= w.ring1 {
        let u = (c - w.ring1) / (1.0 - w.ring1);
        let d = lerp(w.damage_ring[0], w.damage_ring[1], u);
        BowShot {
            tier: 1,
            range: lerp(w.flight_ring[0], w.flight_ring[1], u),
            speed: lerp(w.speed_ring[0], w.speed_ring[1], u),
            dmg: d,
            side: d * w.side_mul,
            fan: w.fan_ring,
            lodge: true,
            fuse: w.fuse_ring,
            br: w.burst_radius[0],
            bd: w.burst_damage[0],
            be: w.burst_edge[0],
            bp: w.burst_paint[0],
        }
    } else {
        let u = clamp_f(c / w.ring1, 0.0, 1.0);
        let d = lerp(w.damage_tap[0], w.damage_tap[1], u);
        BowShot {
            tier: 0,
            range: lerp(w.flight_tap[0], w.flight_tap[1], u),
            speed: w.speed_tap,
            dmg: d,
            side: d * w.side_mul,
            fan: w.fan_tap,
            lodge: false,
            fuse: 0.0,
            br: 0.0,
            bd: 0.0,
            be: 0.0,
            bp: 0.0,
        }
    }
}

/// One arrow of a volley (JS `newArrow` record L68-69 + the `Object.assign`
/// field set L105-106, minus the mesh/FX fields). World object: the match
/// owns the `arrows` list and steps it reverse-order every frame, like the
/// blade waves.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BowArrow {
    pub owner_slot: usize,
    pub team: usize,
    pub pos: Vec3,
    pub prev: Vec3,
    pub vel: Vec3,
    pub dir: Vec3,
    /// Flight direction at the direct hit (JS `fdir`): the dropped arrow
    /// sticks in at a slant along it (L237).
    pub fdir: Vec3,
    /// Surface point the lodged arrow is stuck into (JS `hitP`).
    pub hit_p: Vec3,
    pub nrm: Vec3,
    /// Lodged flag (JS `st`: 0 flying, 1 lodged).
    pub st: bool,
    pub age: f32,
    pub dist: f32,
    /// Lodged fuse clock (JS `t`).
    pub t: f32,
    pub range: f32,
    pub speed: f32,
    pub tier: u8,
    pub dmg: f32,
    pub lodge: bool,
    pub fuse: f32,
    pub br: f32,
    pub bd: f32,
    pub be: f32,
    pub bp: f32,
    /// Trail-drip distance accumulator (JS `trail`, starts `-1.8` so the
    /// first drip lands behind the archer).
    pub trail: f32,
    pub seed: f32,
    /// Already took its direct body hit (JS `noHit`): the arrow drops.
    pub no_hit: bool,
    /// Centre arrow of the fan (JS `center` — presentation pitch only, kept
    /// for the record parity).
    pub center: bool,
    /// A remote player's arrow replayed from the `['k', …]` record (JS
    /// `ghost` L114-118): it flies, lodges and bursts for the eye only —
    /// never paints, never hurts.
    #[serde(default)]
    pub ghost: bool,
}

/// JS `volley(a, S, m, dir, air, ghost)` L97-113: the three arrows of a
/// volley — a flat fan about the vertical on the ground, an upright one
/// about the aim's right axis in the air. Consumes 3 rng draws (JS L106
/// `Math.random()` per arrow, in `i = -1, 0, 1` order).
#[allow(clippy::too_many_arguments)]
pub fn volley(
    a: &Actor,
    s: &BowShot,
    m: Vec3,
    dir: Vec3,
    air: bool,
    ghost: bool,
    arrows: &mut Vec<BowArrow>,
    ps: &mut ProjectileSim,
) {
    // JS L98: the fan axis.
    let axis = if air {
        let ax = dir.cross(Vec3::Y);
        if ax.length_squared() < 1e-4 {
            Vec3::X
        } else {
            ax.normalize()
        }
    } else {
        Vec3::Y
    };
    for i in -1i32..=1 {
        // JS L101: the pool is capped — `kill(0)` (L71) swaps the last
        // arrow into the hole and pops; the list order is not preserved.
        if arrows.len() >= MAX_ARROWS {
            arrows.swap_remove(0);
        }
        let d = if i == 0 {
            dir
        } else {
            Quat::from_axis_angle(axis, (i as f32) * s.fan * DEG).mul_vec3(dir)
        };
        let seed = ps.rng.next_f32();
        arrows.push(BowArrow {
            owner_slot: a.slot,
            team: a.team,
            pos: m,
            prev: m,
            vel: d * s.speed,
            dir: d,
            fdir: d,
            hit_p: Vec3::ZERO,
            nrm: Vec3::Y,
            st: false,
            age: 0.0,
            dist: 0.0,
            t: 0.0,
            range: s.range,
            speed: s.speed,
            tier: s.tier,
            dmg: if i != 0 { s.side } else { s.dmg },
            lodge: s.lodge,
            fuse: s.fuse,
            br: s.br,
            bd: s.bd,
            be: s.be,
            bp: s.bp,
            trail: -1.8,
            seed,
            no_hit: false,
            center: i == 0,
            ghost,
        });
    }
}

/// JS `looseVolley(a, c)` L74-93 (the weapons-side release): the muzzle,
/// the aim, the three arrows, the `['k','bow']` record and the fire event.
pub fn loose_volley(
    a: &Actor,
    c: f32,
    w: &Bow,
    arrows: &mut Vec<BowArrow>,
    ps: &mut ProjectileSim,
) -> BowShot {
    let s = bow_shot(c, w);
    let m = ProjectileSim::muzzle(a);
    let dir = ProjectileSim::aim_from(a, m, None);
    // JS L78 `_ballistic(m, dir, a.aimPoint, …)`: `aim_point` is always None
    // in the sim — the aim line is kept as-is (module docs).
    let air = !a.grounded;
    volley(a, &s, m, dir, air, false, arrows, ps);
    let q2 = |v: f32| (v * 100.0).round() / 100.0;
    let q3 = |v: f32| (v * 1000.0).round() / 1000.0;
    // JS L88 `netRec(a, 'bow', [r2 m×3, r3 dir×3, r3 c, air])`.
    ps.push_event(SimEvent::Kit {
        owner_slot: a.slot,
        team: a.team,
        kind: "bow".to_string(),
        data: vec![
            q2(m.x),
            q2(m.y),
            q2(m.z),
            q3(dir.x),
            q3(dir.y),
            q3(dir.z),
            q3(c),
            if air { 1.0 } else { 0.0 },
        ],
    });
    // JS L90 `emit('weapon:fire', { …, charge: c })` — the `['p']`-record
    // feed, like the blade's spawnWave Fire (Task 32 §7).
    ps.push_event(SimEvent::Fire {
        owner_slot: a.slot,
        team: a.team,
        muzzle: m,
        dir,
        charge: Some(c),
        len: None,
    });
    s
}

/// JS `lodge(p, point, n, d)` L122-132: stick the arrow into a surface.
/// The splat takes no rng draw (JS rides the arrow's own `seed`); its turf
/// credit rides [`SimEvent::Turf`] to the match drain, like the blade
/// waves' stripes.
#[allow(clippy::too_many_arguments)]
fn lodge(
    p: &mut BowArrow,
    point: Vec3,
    n: Vec3,
    d: Vec3,
    w: &Bow,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    ps: &mut ProjectileSim,
) {
    p.st = true;
    p.t = 0.0;
    p.hit_p = point;
    p.nrm = n;
    p.dir = d.normalize_or(d);
    // JS L125: tip buried, the shaft sticks out behind it.
    p.pos = p.hit_p + p.dir * 0.12;
    if p.ghost {
        return;
    }
    let at = p.hit_p + p.nrm * 0.08;
    let area = paint.splat(
        world,
        at,
        w.paint_stick,
        p.team,
        &SplatOpts {
            seed: p.seed,
            stretch: Some(p.dir),
            stretch_amt: Some(0.5),
            kind: None,
        },
    );
    if area > 0.0 {
        ps.push_event(SimEvent::Turf {
            owner_slot: p.owner_slot,
            team: p.team,
            area,
        });
    }
}

/// JS `burst(p)` L135-162: the lodged arrow detonates — paint ring, splash
/// damage with falloff + line of sight, the impact event. Takes no rng
/// draws (JS rides `p.seed` throughout, L138-142).
#[allow(clippy::too_many_arguments)]
pub fn burst(
    p: &BowArrow,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    w: &Bow,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    actors: &mut [Actor],
    remote: Option<&[bool]>,
) {
    let c = p.hit_p + p.nrm * 0.18;
    if !p.ghost {
        let mut area = paint.splat(
            world,
            p.hit_p + p.nrm * 0.1,
            p.bp,
            p.team,
            &SplatOpts {
                seed: p.seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        for i in 0..2 {
            let ang = p.seed * 20.0 + (i as f32) * 2.7;
            let r = p.bp * 0.55;
            let pt = vec3(c.x + ang.cos() * r, c.y + 0.1, c.z + ang.sin() * r);
            area += paint.splat(
                world,
                pt,
                p.bp * 0.45,
                p.team,
                &SplatOpts {
                    seed: p.seed + i as f32,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            );
        }
        if area > 0.0 {
            ps.push_event(SimEvent::Turf {
                owner_slot: p.owner_slot,
                team: p.team,
                area,
            });
        }
        // JS L146-158: splash damage — nearest point of the victim's body
        // axis, distance past 60 % of the capsule radius, line of sight.
        let inner = w.burst_inner;
        for si in 0..actors.len() {
            let e = &actors[si];
            if e.team == p.team || !e.alive {
                continue;
            }
            let h = if e.form == Form::Squid {
                t.squid_height
            } else {
                t.height
            };
            let hr = t.radius;
            let by = e.pos.y + e.smooth_y();
            let v = vec3(
                e.pos.x,
                clamp_f(c.y, by + 0.25, by + (0.3f32).max(h - 0.25)),
                e.pos.z,
            );
            let d = (v.distance(c) - hr * 0.6).max(0.0);
            if d > p.br || !world.los(c, v) {
                continue;
            }
            let dmg = if d <= inner {
                p.bd
            } else {
                lerp(p.bd, p.be, (d - inner) / (p.br - inner).max(0.01))
            };
            route_hit(p, si, dmg, t, ps, actors, remote);
        }
    }
    // JS `burstFx` L170 `emit('weapon:impact', …)` — pushed for ghosts too
    // (spec §1: ghost → Impact only).
    ps.push_event(SimEvent::Impact {
        pos: c,
        normal: p.nrm,
        team: p.team,
        radius: p.br,
        victim_slot: None,
        owner_slot: p.owner_slot,
        area: 0.0,
    });
}

/// Route a direct-hit through the remote mask like [`ProjectileSim`]'s body
/// hits (JS `applyHit` → `'send'` for proxy victims).
fn route_hit(
    p: &BowArrow,
    si: usize,
    dmg: f32,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    actors: &mut [Actor],
    remote: Option<&[bool]>,
) {
    let (victim_slot, victim_team) = (actors[si].slot, actors[si].team);
    let send = remote.is_some_and(|m| m.get(si).copied().unwrap_or(false));
    if send {
        ps.push_event(SimEvent::HitSend {
            attacker_slot: p.owner_slot,
            victim_slot,
            victim_team,
            team: p.team,
            damage: dmg,
        });
    } else {
        let atk_pos = super::actor_pos_by_slot(actors, p.owner_slot);
        let outcome = apply_hit(
            &mut actors[si],
            dmg,
            Some((p.team, p.owner_slot)),
            atk_pos,
            t,
        );
        ps.push_event(SimEvent::Hit {
            attacker_slot: p.owner_slot,
            victim_slot,
            team: p.team,
            damage: dmg,
            killed: outcome.killed,
        });
    }
}

impl BowArrow {
    /// JS `stepArrow(p, i, dt)` (L175-264). Returns `false` when the arrow
    /// is spent and the match must remove it (JS `kill` L71).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        p: &mut Self,
        dt: f32,
        world: &CollisionWorld,
        paint: &mut PaintGrid,
        w: &Bow,
        t: &PlayerTuning,
        ps: &mut ProjectileSim,
        actors: &mut [Actor],
        remote: Option<&[bool]>,
    ) -> bool {
        if p.st {
            // Lodged: fuse down, then burst (the `ticks` countdown is audio).
            p.t += dt;
            if p.t >= p.fuse {
                burst(p, world, paint, w, t, ps, actors, remote);
                return false;
            }
            return true;
        }
        p.age += dt;
        p.prev = p.pos;
        if p.age > w.straight {
            p.vel.y -= w.grav * dt;
        }
        if p.dist > p.range {
            // Past the reach the arrow noses down (JS L191).
            p.vel.y -= w.dive_grav * dt;
            p.vel *= 1.0 - w.dive_drag * dt;
        }
        p.pos += p.vel * dt;
        let sp = p.vel.length();
        p.dist += sp * dt;
        if sp > 0.01 {
            p.dir = p.vel / sp;
        }
        // JS L197-214: actors — a victim takes the direct hit; a lodging
        // arrow then drops and sticks into the floor at their feet.
        if !p.no_hit {
            for si in 0..actors.len() {
                let e = &actors[si];
                if e.team == p.team || !e.alive {
                    continue;
                }
                let h = if e.form == Form::Squid {
                    t.squid_height
                } else {
                    t.height
                };
                let hr = t.radius;
                if (e.pos.x - p.pos.x).abs() > 3.0 + hr || (e.pos.z - p.pos.z).abs() > 3.0 + hr {
                    continue;
                }
                let base = vec3(e.pos.x, e.pos.y + e.smooth_y(), e.pos.z);
                let res = segment_capsule_dist(p.prev, p.pos, base, hr, h);
                if res.dist < hr * 0.95 + ARROW_R {
                    let at = p.prev.lerp(p.pos, res.t);
                    if !p.ghost {
                        route_hit(p, si, p.dmg, t, ps, actors, remote);
                    }
                    // JS L207: the `weapon:impact` emit is *outside* the
                    // `!ghost` guard — a ghost arrow's hit still shows.
                    ps.push_event(SimEvent::Impact {
                        pos: at,
                        normal: -p.dir,
                        team: p.team,
                        radius: 0.3,
                        victim_slot: Some(actors[si].slot),
                        owner_slot: p.owner_slot,
                        area: 0.0,
                    });
                    if !p.lodge {
                        return false;
                    }
                    p.no_hit = true;
                    p.fdir = p.dir;
                    p.pos = at;
                    p.prev = at;
                    p.vel = vec3(p.dir.x * 1.5, -9.0, p.dir.z * 1.5);
                    p.dist = p.dist.max(p.range);
                    return true;
                }
            }
        }
        // JS L216-231: boss spheres / enemy devices block arrows — not
        // ported (no boss/subs/specials layers, Tasks 34-36).
        // JS L233-249: the world.
        let hit = world.segment(p.prev, p.pos, true);
        if hit.hit {
            if p.lodge {
                // A dropped arrow (after a direct hit) sticks in at a slant
                // along its old flight line (JS L237).
                let stick = if p.no_hit {
                    vec3(p.fdir.x * 0.55, -1.0, p.fdir.z * 0.55).normalize_or(Vec3::Y)
                } else {
                    p.dir
                };
                lodge(p, hit.point, hit.normal, stick, w, world, paint, ps);
                return true;
            }
            if !p.ghost {
                let at = hit.point + hit.normal * 0.12;
                let area = paint.splat(
                    world,
                    at,
                    w.paint_tap * (0.85 + 0.3 * p.seed),
                    p.team,
                    &SplatOpts {
                        seed: p.seed,
                        stretch: Some(p.dir),
                        stretch_amt: Some(0.7),
                        kind: None,
                    },
                );
                if area > 0.0 {
                    ps.push_event(SimEvent::Turf {
                        owner_slot: p.owner_slot,
                        team: p.team,
                        area,
                    });
                }
                ps.push_event(SimEvent::Impact {
                    pos: hit.point,
                    normal: hit.normal,
                    team: p.team,
                    radius: w.paint_tap,
                    victim_slot: None,
                    owner_slot: p.owner_slot,
                    area: 0.0,
                });
            }
            return false;
        }
        // JS L252-259: trail drips — lodging arrows only, a dotted ink line
        // under the flight. 2 rng draws per drip (radius, seed).
        if p.lodge && !p.no_hit {
            p.trail += sp * dt;
            if p.trail > w.trail_every {
                p.trail = 0.0;
                let g = world.raycast(p.pos, Vec3::NEG_Y, 4.0, true);
                if g.hit && !p.ghost {
                    let radius = w.trail_radius * (0.8 + ps.rng.next_f32() * 0.4);
                    let seed = ps.rng.next_f32();
                    let area = paint.splat(
                        world,
                        g.point + g.normal * 0.1,
                        radius,
                        p.team,
                        &SplatOpts {
                            seed,
                            stretch: None,
                            stretch_amt: None,
                            kind: None,
                        },
                    );
                    if area > 0.0 {
                        ps.push_event(SimEvent::Turf {
                            owner_slot: p.owner_slot,
                            team: p.team,
                            area,
                        });
                    }
                }
            }
        }
        // JS L260-263: life / sea expiry.
        if p.age > ARROW_LIFE || p.pos.y < t.water_y - 1.2 {
            return false;
        }
        true
    }
}

/// JS `MAIN_KITS.bow.update` (L389-422): the draw. `ps_time` is
/// [`ProjectileSim::time`] — the kit's `G.time` stand-in.
#[allow(clippy::too_many_arguments)]
pub fn update(
    r: &mut WeaponRunner,
    dt: f32,
    gate: &FireGate,
    a: &mut Actor,
    w: &Bow,
    arrows: &mut Vec<BowArrow>,
    ps: &mut ProjectileSim,
    ps_time: f32,
) {
    let mut k = r.bow.take().map(|b| *b).unwrap_or_default();
    k.seen = ps_time;
    if gate.fire && r.cooldown <= 0.0 {
        if !r.charging {
            if a.ink < w.ink_full * w.ink_min {
                r.empty_click(ps, a);
                r.bow = Some(Box::new(k));
                return;
            }
            r.charging = true;
            r.charge = 0.0;
            r.charge_t = 0.0;
            k.ring = 0;
            k.stall = 0.0;
            k.last_c = 0.0;
        }
        // JS L400-402: the draw builds linearly; the tank caps it.
        let max_c = clamp_f(a.ink / w.ink_full, 0.0, 1.0);
        r.charge_t = (r.charge_t + dt / w.charge_time).min(1.0);
        r.charge = max_c.min(r.charge_t);
        if k.ring < 1 && r.charge >= w.ring1 {
            k.ring = 1;
        }
        if k.ring < 2 && r.charge >= 0.999 {
            k.ring = 2;
        }
        // JS L419-420: the bot stall release is Task 39 (no `a.bot` in the
        // sim) — the counters are kept for state parity.
        k.stall = if r.charge > k.last_c + 1e-4 {
            0.0
        } else {
            k.stall + dt
        };
        k.last_c = r.charge;
    } else if r.charging {
        release(r, &mut k, w, a, arrows, ps);
    }
    r.bow = Some(Box::new(k));
}

/// JS `release(runner, w, k)` L423-435: loose the volley, pay the ink.
pub fn release(
    r: &mut WeaponRunner,
    k: &mut BowKit,
    w: &Bow,
    a: &mut Actor,
    arrows: &mut Vec<BowArrow>,
    ps: &mut ProjectileSim,
) {
    r.charging = false;
    let c = r.charge;
    a.ink = (a.ink - w.ink_full * w.ink_min.max(c)).max(0.0);
    a.note_fired();
    loose_volley(a, c, w, arrows, ps);
    r.charge = 0.0;
    r.charge_t = 0.0;
    k.ring = 0;
    k.stall = 0.0;
    r.firing_t = super::FIRING_T;
    r.cooldown = w.cooldown;
}

/// JS `MAIN_KITS.bow.tick` L514-519: a draw left hanging (the runner
/// stopped updating — e.g. a special took the trigger, Task 35) is
/// cancelled quietly. The match calls this for every bow runner each frame.
pub fn sweep_stalled_draw(r: &mut WeaponRunner, ps_time: f32) {
    let Some(kit) = r.bow.as_deref_mut() else {
        return;
    };
    if r.charging && ps_time - kit.seen > 0.15 {
        r.charging = false;
        r.charge = 0.0;
        r.charge_t = 0.0;
        kit.ring = 0;
    }
}
