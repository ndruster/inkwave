//! Whirl Boomerang — the `boomerang` kit sub (Task 34 batch two).
//!
//! Faithful port of `src/game/kits/boomerang.js` (710 lines): a flat throw
//! along the aim (pitch clamped to `[-0.5, 0.25]`), flying out to the plan's
//! end point — `range` away, or short of a wall — easing to a stop. It then
//! hovers, shredding close foes with rapid ticks and inking the floor under
//! it, whirls back to the thrower, circles them for `orbit` seconds grazing
//! foes it touches, and bursts beside them like a small Splat Bomb. A foe in
//! its path (out or back) stops it dead: it arms in place and goes off like
//! a full Splat Bomb instead, without returning. One out at a time
//! ([`blocked`]). A splatted thrower fizzles it: it drops and pops in a
//! little splash of ink, no damage.
//!
//! Keep in lockstep with upstream on sync:
//!   - `plan` (L145-152)        -> [`plan`]
//!   - `planFrom` (L154-165)    -> [`plan_from`]
//!   - `use` (L175-182)         -> [`use_boomerang`]
//!   - `spawn` (L185-200)       -> [`spawn`]
//!   - `ghost` (L201-215)       -> [`ghost`]
//!   - `tick` (L223-241)        -> [`tick`]
//!   - `out` (L249-266)         -> [`out`]
//!   - `hover` (L268-281)       -> [`hover`]
//!   - `shred` (L283-309)       -> [`shred`]
//!   - `startBack` (L312-320)   -> [`start_back`]
//!   - `back` (L322-342)        -> [`back`]
//!   - `orbit` (L344-369)       -> [`orbit`]
//!   - `contact` (L371-384)     -> [`contact`]
//!   - `arm` (L386-395)         -> [`arm`]
//!   - `armed` (L396-404)       -> [`armed`]
//!   - `fizzle` (L406-417)      -> [`fizzle`]
//!   - `'splatted'` (L418-426)  -> [`owner_splatted`]
//!   - `burst` (L428)           -> [`burst`]
//!   - `blast` (L429-457)       -> [`blast`]
//!   - `streak` (L459-466)      -> [`streak`]
//!   - `blocked` (L603)         -> [`blocked`]
//!
//! Presentation-only branches are dropped per the M3 range rule (spec §0):
//! the prop mesh / spin-blur disc (`makeMesh`, `proto`, `draw`), the `spinA`
//! / `spinW` / `fxT` / `beepT` / `loop` / `loopName` / `mesh` fields, the
//! audio, `G.fx`, `nearCam`, the aim guide (`hold` / `guideTick`, L536-568),
//! the `bot` brain, the icon / SFX blocks and the never-read `victim` /
//! `ground` fields. `emit('sub:use', …)` rides [`SimEvent::SubUse`];
//! `applyHit` rides [`Ctx::hit_routed`]; `sub:arm` / `bomb:explode` /
//! `shake` are not in the netmatch `FORWARD` list and never reach the sim.
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-3: `it.sp` (`a.specialActive`, L191) is always false in the
//!     sim (specials are Task 35), so the turf credit is always the plain
//!     `addTurf`.
//!   - D-T34-5: ghosts key on `(src_nid, gid)` (JS dedups on the globally
//!     unique `gid` alone, L203/L207); the replication layer rewrites the
//!     wire gid.
//!   - `e.smoothY` on the foe capsule base is ignored — the batch-one
//!     precedent (capsules use `e.pos`).
//!   - The end record `[1, gid, pos×3, big]` and the arm record
//!     `[3, gid, pos×3]` use `r3` (boomerang.js L9), not the batch-one `r2`.
//!   - `ghostMute` (registry.js L73-77) raises the paint mute and, through
//!     it, `_hurt`'s `netMuted()` guard: the port threads `muted = it.ghost`
//!     into the splats and the device damage instead. Ghost hits drop
//!     through [`Ctx::hit_routed`]'s ghost flag.
//!   - The `contact` foe scan keeps JS's `e.invuln > 0` skip (L374); the
//!     `shred` / `orbit` / `blast` scans have no such guard upstream and
//!     gained none here.

use glam::{Vec3, vec3};

use crate::actor::Form;
use crate::paint::SplatOpts;
use crate::weapon::{SimEvent, point_capsule_dist, segment_capsule_dist};

use super::{Ctx, DOWN, TAU, clamp, lerp};

/// JS `r3` (L9): the three-decimal quantisation this kit's records use.
fn r3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

/// JS `UP` (L27) — only the fizzle's burst normal (presentation) used it;
/// kept for the `shred`-free blast path is not needed. Unused constant
/// removed; the module uses [`super::DOWN`] only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
/// JS `it.state` (the `tick` switch, L231-238). `dead` is the removal
/// sentinel (L239-241).
pub enum BoomerangState {
    /// `out` — the flat throw easing to a stop at the plan's end.
    Out,
    /// `hover` — spinning in place, shredding and inking.
    Hover,
    /// `back` — the curving homing flight to the thrower.
    Back,
    /// `orbit` — circling the thrower.
    Orbit,
    /// `armed` — latched onto a foe: full-blast fuse.
    Armed,
    /// `fizzle` — the thrower was splatted: dropping to pop.
    Fizzle,
    /// `dead` — removed at the end of this update pass.
    Dead,
}

/// JS `newPlan()` (L166): the shared throw plan (outbound flight target).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct BoomerangPlan {
    /// JS `P.dir` — the flat throw direction.
    pub dir: Vec3,
    /// JS `P.start` — the release point.
    pub start: Vec3,
    /// JS `P.end` — the hover point (start + dir·len, floor-lifted).
    pub end: Vec3,
    /// JS `P.wallPoint` — the wall hit position (valid when `wall`).
    pub wall_point: Vec3,
    /// JS `P.wallNormal` — the wall hit normal (valid when `wall`).
    pub wall_normal: Vec3,
    /// JS `P.len` — the outbound distance.
    pub len: f32,
    /// JS `P.wall` — the ray stopped short of a wall.
    pub wall: bool,
}

/// One live boomerang (JS `it`, `spawn` L185-200 minus the mesh / audio /
/// `spin` / `spinW` / `fxT` / `beepT` / `loop` / `victim` presentation and
/// never-read fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoomerangItem {
    /// JS `it.owner` — actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`).
    pub owner_slot: usize,
    /// The owner's team at spawn (JS `a.team`; `it.team` mirrors it).
    pub owner_team: usize,
    /// JS `it.team`.
    pub team: usize,
    /// JS `it.state`.
    pub state: BoomerangState,
    /// JS `it.t` — the per-state timer (the tick adds it before the switch).
    pub t: f32,
    /// JS `it.age`.
    pub age: f32,
    /// JS `it.pos`.
    pub pos: Vec3,
    /// JS `it.prev`.
    pub prev: Vec3,
    /// JS `it.vel` — the flight velocity (drives the fizzle drop and the
    /// `back` homing; the bank / trail FX are dropped).
    pub vel: Vec3,
    /// JS `it.plan`.
    pub plan: BoomerangPlan,
    /// JS `it.outT` — the outbound duration (`max(0.18, outTime *
    /// sqrt(len / range))`, L193).
    pub out_t: f32,
    /// JS `it.trail` — the streak distance accumulator (L460).
    pub trail: f32,
    /// JS `it.tickT` — the shred tick countdown (L290).
    pub tick_t: f32,
    /// JS `it.paintT` — the hover / orbit paint countdown (L301/L362).
    pub paint_t: f32,
    /// JS `it.hitCd` — per-foe orbit-graze cooldowns (actor index → time).
    pub hit_cd: Vec<(usize, f32)>,
    /// JS `it.loop`'s name — presentation-only; dropped.
    /// JS `it.fromWall` (L196): the wall splat on arrival.
    pub from_wall: bool,
    /// JS `it.orbitA` — the orbit angle (set at the `back` → `orbit`
    /// transition, L338).
    pub orbit_a: f32,
    /// JS `it.from` — the orbit ease-in start position (L339).
    pub orbit_from: Vec3,
    /// JS `it.ghost`.
    pub ghost: bool,
    /// JS `it.gid` (D-T34-5: ghosts keep the wire form, keyed with
    /// `src_nid`).
    pub gid: u32,
    /// The remote owner's `nid` for a ghost (D-T34-5).
    pub src_nid: Option<u8>,
    /// JS `it.blasted` — it went off (suppresses the `[2, gid]` end
    /// record, L239).
    pub blasted: bool,
}

/// JS `chest(e, out)` (L170): the aim / homing chest point.
fn chest(pos: Vec3, form: Form) -> Vec3 {
    vec3(
        pos.x,
        pos.y + if form == Form::Squid { 0.3 } else { 0.75 },
        pos.z,
    )
}

/// JS `bodyH(e)` (L173): the foe capsule height.
fn body_h(form: Form, squid_h: f32, h: f32) -> f32 {
    if form == Form::Squid { squid_h } else { h }
}

/// JS `credit(it, area)` (L171): turf for the thrower when the splat landed
/// ink. Ghosts run muted (area 0), so the `area > 0` guard covers both.
fn credit(ctx: &mut Ctx, i: usize, area: f32) {
    if area > 0.0 {
        let (slot, team) = (ctx.boomerang[i].owner_slot, ctx.boomerang[i].team);
        ctx.events.push(SimEvent::Turf {
            owner_slot: slot,
            team,
            area,
        });
    }
}

/// JS `plan(a, s, out)` (L145-152): the flat throw from the hand along the
/// aim, pitch clamped.
fn plan(ctx: &Ctx, actor_idx: usize) -> BoomerangPlan {
    let a = &ctx.actors[actor_idx];
    let pitch = clamp(a.aim_pitch, -0.5, 0.25);
    let cp = pitch.cos();
    let (sy, cy) = (a.aim_yaw.sin(), a.aim_yaw.cos());
    BoomerangPlan {
        dir: vec3(sy * cp, pitch.sin(), cy * cp),
        start: vec3(a.pos.x + sy * 0.3, a.pos.y + 1.35, a.pos.z + cy * 0.3),
        end: Vec3::ZERO,
        wall_point: Vec3::ZERO,
        wall_normal: Vec3::ZERO,
        len: 0.0,
        wall: false,
    }
}

/// JS `planFrom(s, out)` (L154-165): complete the plan from its start and
/// direction — stop short of a wall, lift off the floor.
fn plan_from(ctx: &Ctx, plan: &mut BoomerangPlan) {
    let range = ctx.sb.boomerang.range;
    let h = ctx.world.raycast(plan.start, plan.dir, range + 0.4, false);
    plan.wall = h.hit;
    plan.len = if h.hit {
        (h.dist - 0.4).max(0.3)
    } else {
        range
    };
    if h.hit {
        plan.wall_point = h.point;
        plan.wall_normal = h.normal;
    }
    plan.end = plan.start + plan.dir * plan.len;
    let gy = ctx
        .world
        .ground_height(plan.end.x, plan.end.z, plan.end.y + 0.3, false);
    if gy > f32::NEG_INFINITY && plan.end.y - gy < 0.55 {
        plan.end.y = gy + 0.55;
    }
}

/// JS `use(subs, a, sub)` (L175-182): the throw.
pub(crate) fn use_boomerang(ctx: &mut Ctx, actor_idx: usize) {
    let a = &ctx.actors[actor_idx];
    let (slot, team) = (a.slot, a.team);
    let mut plan = plan(ctx, actor_idx);
    plan_from(ctx, &mut plan);
    let gid = ctx.gid_next(slot);
    spawn(ctx, actor_idx, slot, team, plan, false, gid, None);
    // JS L178: `netRec(a, 'boomerang', [0, gid, start×3, dir×3])` — the kit
    // record layout has no kind string at `d[2]` (unlike `subs`).
    ctx.events.push(SimEvent::SubSpawn {
        owner_slot: slot,
        team,
        sub_kind: "boomerang".to_string(),
        data: vec![
            0.0,
            gid as f32,
            r3(plan.start.x),
            r3(plan.start.y),
            r3(plan.start.z),
            r3(plan.dir.x),
            r3(plan.dir.y),
            r3(plan.dir.z),
        ],
    });
    ctx.events.push(SimEvent::SubUse {
        owner_slot: slot,
        team,
        kind: super::SubKind::Boomerang,
    });
}

/// JS `spawn(subs, a, s, P, ghost, gid)` (L185-200) minus the mesh / audio;
/// the `spinA` / `spinW` / `fxT` / `beepT` / `loop` fields are presentation.
#[allow(clippy::too_many_arguments)]
fn spawn(
    ctx: &mut Ctx,
    owner_idx: usize,
    owner_slot: usize,
    owner_team: usize,
    plan: BoomerangPlan,
    ghost: bool,
    gid: u32,
    src_nid: Option<u8>,
) {
    let s = &ctx.sb.boomerang;
    let out_t = (s.out_time * (plan.len / s.range).sqrt()).max(0.18);
    ctx.boomerang.push(BoomerangItem {
        owner_idx,
        owner_slot,
        owner_team,
        team: owner_team,
        state: BoomerangState::Out,
        t: 0.0,
        age: 0.0,
        pos: plan.start,
        prev: plan.start,
        vel: Vec3::ZERO,
        plan,
        out_t,
        trail: 0.0,
        tick_t: 0.0,
        paint_t: 0.0,
        hit_cd: Vec::new(),
        from_wall: plan.wall,
        orbit_a: 0.0,
        orbit_from: Vec3::ZERO,
        ghost,
        gid,
        src_nid,
        blasted: false,
    });
}

/// JS `ghost(a, d)` (L201-215): a remote throw / arm / burst / fizzle
/// record. The netmatch replay wraps `'k'` records in `mute++`
/// (netmatch.js L490-491), so the ending ops run muted: paint draws, no
/// splats, no hits — the owner's screen decides.
pub(crate) fn ghost(ctx: &mut Ctx, actor_idx: usize, nid: u8, data: &[f32]) {
    let (op, gid) = (data[0], data[1] as u32);
    if op == 0.0 {
        if data.len() < 8 {
            return;
        }
        // JS L203 dedups on the globally-unique `gid`; the sim keys ghosts
        // on `(src_nid, gid)` (D-T34-5).
        if ctx
            .boomerang
            .iter()
            .any(|x| x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        let mut plan = BoomerangPlan {
            dir: vec3(data[5], data[6], data[7]).normalize_or_zero(),
            start: vec3(data[2], data[3], data[4]),
            end: Vec3::ZERO,
            wall_point: Vec3::ZERO,
            wall_normal: Vec3::ZERO,
            len: 0.0,
            wall: false,
        };
        plan_from(ctx, &mut plan);
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        spawn(ctx, actor_idx, slot, team, plan, true, gid, Some(nid));
        return;
    }
    let idx = ctx.boomerang.iter().position(|x| {
        x.ghost && x.src_nid == Some(nid) && x.gid == gid && x.state != BoomerangState::Dead
    });
    let Some(i) = idx else { return };
    if op == 3.0 {
        if data.len() < 5 {
            return;
        }
        ctx.boomerang[i].prev = ctx.boomerang[i].pos;
        ctx.boomerang[i].pos = vec3(data[2], data[3], data[4]);
        arm(ctx, i);
    } else if op == 1.0 {
        if data.len() < 6 {
            return;
        }
        ctx.boomerang[i].pos = vec3(data[2], data[3], data[4]);
        if data[5] != 0.0 {
            let s = &ctx.sb.boomerang;
            let (r, dmax, dmin, pr) = (
                s.hit_radius,
                s.hit_damage_max,
                s.hit_damage_min,
                s.hit_paint_radius,
            );
            blast(ctx, i, r, dmax, dmin, pr, true, true);
        } else {
            burst(ctx, i, true);
        }
    } else if op == 2.0 && ctx.boomerang[i].state != BoomerangState::Fizzle {
        ctx.boomerang[i].state = BoomerangState::Fizzle;
        ctx.boomerang[i].t = 0.0;
        ctx.boomerang[i].vel *= 0.25;
    }
}

/// JS `tick(dt)` (L223-241): the reverse walk, the per-item ghost mute wrap
/// and the dead splice with the `[2, gid]` end record.
pub(crate) fn tick(ctx: &mut Ctx, dt: f32) {
    let mut i = ctx.boomerang.len();
    while i > 0 {
        i -= 1;
        ctx.boomerang[i].t += dt;
        ctx.boomerang[i].age += dt;
        ctx.boomerang[i].prev = ctx.boomerang[i].pos;
        let muted = ctx.boomerang[i].ghost; // JS `ghostMute` wrap (L230)
        let state = ctx.boomerang[i].state;
        match state {
            BoomerangState::Out => out(ctx, i, dt),
            BoomerangState::Hover => hover(ctx, i, dt, muted),
            BoomerangState::Back => back(ctx, i, dt),
            BoomerangState::Orbit => orbit(ctx, i, dt, muted),
            BoomerangState::Armed => armed(ctx, i, dt, muted),
            BoomerangState::Fizzle => fizzle(ctx, i, dt, muted),
            BoomerangState::Dead => {}
        }
        if ctx.boomerang[i].ghost && ctx.boomerang[i].age > 25.0 {
            ctx.boomerang[i].state = BoomerangState::Dead; // (its owner left)
        }
        if ctx.boomerang[i].state == BoomerangState::Dead {
            if !ctx.boomerang[i].ghost && !ctx.boomerang[i].blasted {
                let (slot, team, gid) = (
                    ctx.boomerang[i].owner_slot,
                    ctx.boomerang[i].owner_team,
                    ctx.boomerang[i].gid,
                );
                ctx.events.push(SimEvent::SubEnd {
                    owner_slot: slot,
                    team,
                    sub_kind: Some("boomerang".to_string()),
                    data: vec![2.0, gid as f32],
                });
            }
            ctx.boomerang.remove(i);
        }
    }
}

/// JS `out(it, dt)` (L249-266): ease from the throw speed to a stop at the
/// plan's end; a foe in the way arms it.
fn out(ctx: &mut Ctx, i: usize, _dt: f32) {
    let (start, dir, len, out_t) = {
        let it = &ctx.boomerang[i];
        (it.plan.start, it.plan.dir, it.plan.len, it.out_t)
    };
    let u = clamp(ctx.boomerang[i].t / out_t, 0.0, 1.0);
    let s = len * (1.0 - (1.0 - u) * (1.0 - u));
    ctx.boomerang[i].pos = start + dir * s;
    ctx.boomerang[i].vel = dir * ((2.0 * len / out_t) * (1.0 - u));
    if !ctx.boomerang[i].ghost && contact(ctx, i) {
        return;
    }
    streak(ctx, i, 0.29, ctx.boomerang[i].ghost);
    if u >= 1.0 {
        if ctx.boomerang[i].from_wall {
            // stopped at a wall: a splat of ink on it where it struck (L260).
            let (wp, wn, team, ghost) = (
                ctx.boomerang[i].plan.wall_point,
                ctx.boomerang[i].plan.wall_normal,
                ctx.boomerang[i].team,
                ctx.boomerang[i].ghost,
            );
            let seed = ctx.rng.next_f32();
            let area = if ghost {
                0.0
            } else {
                ctx.paint.splat(
                    ctx.world,
                    wp + wn * 0.1,
                    0.65,
                    team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                )
            };
            credit(ctx, i, area);
        }
        ctx.boomerang[i].state = BoomerangState::Hover;
        ctx.boomerang[i].t = 0.0;
        ctx.boomerang[i].tick_t = 0.0;
        ctx.boomerang[i].paint_t = 0.0;
    }
}

/// JS `hover(it, dt)` (L268-281): spin in place, shred what's close, ink
/// the floor; settle to a useful height over it.
fn hover(ctx: &mut Ctx, i: usize, dt: f32, muted: bool) {
    ctx.boomerang[i].vel = Vec3::ZERO;
    let pos = ctx.boomerang[i].pos;
    let gy = ctx.world.ground_height(pos.x, pos.z, pos.y + 0.2, false);
    if gy > f32::NEG_INFINITY && pos.y - gy < 4.5 {
        let h = pos.y - gy;
        let want = if h > 1.6 {
            gy + 1.1
        } else if h < 0.6 {
            gy + 0.8
        } else {
            pos.y
        };
        ctx.boomerang[i].pos.y = lerp(pos.y, want, 1.0 - (-2.5 * dt).exp());
    }
    shred(ctx, i, dt, 1.0, muted);
    if ctx.boomerang[i].t >= ctx.sb.boomerang.hover {
        start_back(ctx, i);
    }
}

/// JS `shred(it, dt, paintK)` (L283-309): the rapid tick damage to foes in
/// `hoverRadius` with a clear line, the device `damageArea`, and the ink
/// drops under it while `paintK > 0`.
fn shred(ctx: &mut Ctx, i: usize, dt: f32, paint_k: f32, muted: bool) {
    let s = &ctx.sb.boomerang;
    let (tick_rate, tick_damage, hover_radius) = (s.tick_rate, s.tick_damage, s.hover_radius);
    ctx.boomerang[i].tick_t -= dt;
    if ctx.boomerang[i].tick_t <= 0.0 {
        ctx.boomerang[i].tick_t += 1.0 / tick_rate as f32;
        let (pos, team, ghost, oi, os) = (
            ctx.boomerang[i].pos,
            ctx.boomerang[i].team,
            ctx.boomerang[i].ghost,
            ctx.boomerang[i].owner_idx,
            ctx.boomerang[i].owner_slot,
        );
        let (radius, squid_h, h) = (ctx.t.radius, ctx.t.squid_height, ctx.t.height);
        let mut hits: Vec<usize> = Vec::new();
        for (ei, e) in ctx.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            let bh = body_h(e.form, squid_h, h);
            if point_capsule_dist(pos, e.pos, radius, bh) > hover_radius {
                continue;
            }
            let c = chest(e.pos, e.form);
            if !ctx.world.los(pos, c) {
                continue;
            }
            hits.push(ei);
        }
        for ei in hits {
            ctx.hit_routed(ghost, oi, os, team, ei, tick_damage);
        }
        ctx.damage_area(pos, hover_radius, tick_damage, team, muted);
    }
    ctx.boomerang[i].paint_t -= dt;
    if ctx.boomerang[i].paint_t <= 0.0 && paint_k > 0.0 {
        ctx.boomerang[i].paint_t = 0.2;
        let pos = ctx.boomerang[i].pos;
        let g = ctx.world.raycast(pos, DOWN, 3.4, false);
        if g.hit {
            // JS L303-306 draws: `a`, `r`, the radius draw, then `seed`.
            let a = ctx.rng.next_f32() * TAU;
            let r = ctx.rng.next_f32() * 0.45;
            let rad = ctx.sb.boomerang.hover_paint * paint_k * (0.8 + ctx.rng.next_f32() * 0.4);
            let seed = ctx.rng.next_f32();
            let p = g.point + vec3(a.cos() * r, 0.12, a.sin() * r);
            let team = ctx.boomerang[i].team;
            let area = if muted {
                0.0
            } else {
                ctx.paint.splat(
                    ctx.world,
                    p,
                    rad,
                    team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                )
            };
            credit(ctx, i, area);
        }
    }
}

/// JS `startBack(it)` (L312-320): kick out to one side so it swings home.
fn start_back(ctx: &mut Ctx, i: usize) {
    ctx.boomerang[i].state = BoomerangState::Back;
    ctx.boomerang[i].t = 0.0;
    let (oi, pos) = (ctx.boomerang[i].owner_idx, ctx.boomerang[i].pos);
    let (opos, form) = (ctx.actors[oi].pos, ctx.actors[oi].form);
    let mut v = chest(opos, form) - pos;
    v.y = 0.0;
    if v.length_squared() < 1e-4 {
        v = vec3(0.0, 0.0, 1.0);
    }
    let v = v.normalize();
    ctx.boomerang[i].vel = vec3(-v.z, 0.6, v.x) * 5.5;
}

/// JS `back(it, dt)` (L322-342): the curving homing flight (through
/// anything); a foe in the way arms it.
fn back(ctx: &mut Ctx, i: usize, dt: f32) {
    let s = &ctx.sb.boomerang;
    let (return_speed, orbit_radius) = (s.return_speed, s.orbit_radius);
    let (oi, t, pos) = (
        ctx.boomerang[i].owner_idx,
        ctx.boomerang[i].t,
        ctx.boomerang[i].pos,
    );
    let (opos, form) = (ctx.actors[oi].pos, ctx.actors[oi].form);
    let mut tgt = chest(opos, form);
    tgt.y += 0.2;
    let sp = return_speed.min(4.0 + t * 34.0);
    let mut v = tgt - pos;
    let d = v.length();
    v *= sp / d.max(1e-3);
    ctx.boomerang[i].vel = ctx.boomerang[i].vel.lerp(v, 1.0 - (-5.0 * dt).exp());
    let bv = ctx.boomerang[i].vel;
    ctx.boomerang[i].pos += bv * dt;
    if !ctx.boomerang[i].ghost && contact(ctx, i) {
        return;
    }
    streak(ctx, i, 0.22, ctx.boomerang[i].ghost);
    if ctx.boomerang[i].pos.distance(tgt) < orbit_radius + 0.35 {
        ctx.boomerang[i].state = BoomerangState::Orbit;
        ctx.boomerang[i].t = 0.0;
        let pos = ctx.boomerang[i].pos;
        ctx.boomerang[i].orbit_a = (pos.z - opos.z).atan2(pos.x - opos.x);
        ctx.boomerang[i].orbit_from = pos;
        ctx.boomerang[i].paint_t = 0.0;
    } else if ctx.boomerang[i].t > 3.0 && !ctx.boomerang[i].ghost {
        burst(ctx, i, false); // couldn't catch up: goes off where it is (L341)
    }
}

/// JS `orbit(it, dt)` (L344-369): circle the thrower, graze foes, ink a
/// ring; then the small burst.
fn orbit(ctx: &mut Ctx, i: usize, dt: f32, muted: bool) {
    let s = &ctx.sb.boomerang;
    let (orbit_spin, orbit_radius, orbit_time, orbit_damage, orbit_hit_cd) = (
        s.orbit_spin,
        s.orbit_radius,
        s.orbit,
        s.orbit_damage,
        s.orbit_hit_cd,
    );
    ctx.boomerang[i].orbit_a += orbit_spin * dt;
    let oi = ctx.boomerang[i].owner_idx;
    let (opos, form, t) = (ctx.actors[oi].pos, ctx.actors[oi].form, ctx.boomerang[i].t);
    let h = if form == Form::Squid { 0.55 } else { 0.95 };
    let a = ctx.boomerang[i].orbit_a;
    let target = vec3(
        opos.x + a.cos() * orbit_radius,
        opos.y + h + 0.08 * (t * 9.0).sin(),
        opos.z + a.sin() * orbit_radius,
    );
    let k = clamp(t / 0.18, 0.0, 1.0);
    let from = ctx.boomerang[i].orbit_from;
    ctx.boomerang[i].pos = from.lerp(target, k * k * (3.0 - 2.0 * k));
    ctx.boomerang[i].vel = vec3(-a.sin(), 0.0, a.cos()) * (orbit_spin * orbit_radius);
    let (team, ghost, oi, os, pos) = (
        ctx.boomerang[i].team,
        ctx.boomerang[i].ghost,
        ctx.boomerang[i].owner_idx,
        ctx.boomerang[i].owner_slot,
        ctx.boomerang[i].pos,
    );
    let (radius, squid_h, hh) = (ctx.t.radius, ctx.t.squid_height, ctx.t.height);
    let time = ctx.time;
    let mut hits: Vec<usize> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        let bh = body_h(e.form, squid_h, hh);
        if point_capsule_dist(pos, e.pos, radius, bh) > radius + 0.4 {
            continue;
        }
        let last = ctx.boomerang[i]
            .hit_cd
            .iter()
            .find(|(k, _)| *k == ei)
            .map_or(-9.0, |(_, v)| *v);
        if time - last < orbit_hit_cd {
            continue;
        }
        hits.push(ei);
    }
    for ei in hits {
        if let Some(p) = ctx.boomerang[i].hit_cd.iter_mut().find(|(k, _)| *k == ei) {
            p.1 = time;
        } else {
            ctx.boomerang[i].hit_cd.push((ei, time));
        }
        ctx.hit_routed(ghost, oi, os, team, ei, orbit_damage);
    }
    ctx.boomerang[i].paint_t -= dt;
    if ctx.boomerang[i].paint_t <= 0.0 {
        ctx.boomerang[i].paint_t = 0.1;
        let pos = ctx.boomerang[i].pos;
        let g = ctx.world.raycast(pos, DOWN, 2.5, false);
        if g.hit {
            let seed = ctx.rng.next_f32();
            let team = ctx.boomerang[i].team;
            let area = if muted {
                0.0
            } else {
                ctx.paint.splat(
                    ctx.world,
                    vec3(g.point.x, g.point.y + 0.1, g.point.z),
                    0.55,
                    team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                )
            };
            credit(ctx, i, area);
        }
    }
    if ctx.boomerang[i].t >= orbit_time && !ctx.boomerang[i].ghost {
        burst(ctx, i, false); // (a ghost circles on until its owner's record)
    }
}

/// JS `contact(it)` (L371-384): a foe in the swept path stops it dead and
/// arms it (real throws only — the caller guards `!it.ghost`).
fn contact(ctx: &mut Ctx, i: usize) -> bool {
    let s = &ctx.sb.boomerang;
    let (contact_r, tick_damage) = (s.contact, s.tick_damage);
    let (prev, pos, team) = (
        ctx.boomerang[i].prev,
        ctx.boomerang[i].pos,
        ctx.boomerang[i].team,
    );
    let (radius, squid_h, h) = (ctx.t.radius, ctx.t.squid_height, ctx.t.height);
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive || e.invuln > 0.0 {
            continue;
        }
        let bh = body_h(e.form, squid_h, h);
        let res = segment_capsule_dist(prev, pos, e.pos, radius, bh);
        if res.dist > radius + contact_r {
            continue;
        }
        ctx.boomerang[i].pos = prev.lerp(pos, res.t);
        let (ghost, oi, os, gid) = (
            ctx.boomerang[i].ghost,
            ctx.boomerang[i].owner_idx,
            ctx.boomerang[i].owner_slot,
            ctx.boomerang[i].gid,
        );
        ctx.hit_routed(ghost, oi, os, team, ei, tick_damage);
        // JS L380: `netRec(it.owner, 'boomerang', [3, gid, pos×3])`.
        let pos = ctx.boomerang[i].pos;
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: os,
            team,
            sub_kind: "boomerang".to_string(),
            data: vec![3.0, gid as f32, r3(pos.x), r3(pos.y), r3(pos.z)],
        });
        arm(ctx, i);
        return true;
    }
    false
}

/// JS `arm(it, e)` (L386-395): stop dead and run the full-blast fuse. The
/// never-read `victim` / audio / `sub:arm` emit are dropped.
fn arm(ctx: &mut Ctx, i: usize) {
    let tick_rate = ctx.sb.boomerang.tick_rate;
    ctx.boomerang[i].state = BoomerangState::Armed;
    ctx.boomerang[i].t = 0.0;
    ctx.boomerang[i].tick_t = 1.0 / tick_rate as f32;
    ctx.boomerang[i].paint_t = 0.0;
    ctx.boomerang[i].vel = Vec3::ZERO;
}

/// JS `armed(it, dt)` (L396-404): keep shredding (no paint) until the fuse.
fn armed(ctx: &mut Ctx, i: usize, dt: f32, muted: bool) {
    shred(ctx, i, dt, 0.0, muted);
    if ctx.boomerang[i].t >= ctx.sb.boomerang.hit_fuse && !ctx.boomerang[i].ghost {
        let s = &ctx.sb.boomerang;
        let (r, dmax, dmin, pr) = (
            s.hit_radius,
            s.hit_damage_max,
            s.hit_damage_min,
            s.hit_paint_radius,
        );
        blast(ctx, i, r, dmax, dmin, pr, true, false);
    }
}

/// JS `fizzle(it, dt)` (L406-417): drop out of the air and pop (ink, no
/// damage).
fn fizzle(ctx: &mut Ctx, i: usize, dt: f32, muted: bool) {
    {
        let it = &mut ctx.boomerang[i];
        it.vel.y -= 20.0 * dt;
        it.vel.x *= 1.0 - 2.0 * dt;
        it.vel.z *= 1.0 - 2.0 * dt;
        it.pos += it.vel * dt;
    }
    let (prev, pos, t) = (
        ctx.boomerang[i].prev,
        ctx.boomerang[i].pos,
        ctx.boomerang[i].t,
    );
    let g = ctx.world.segment(prev, pos, false);
    if g.hit || t > 0.7 || pos.y < ctx.t.water_y - 1.0 {
        let c = if g.hit { g.point } else { pos };
        if pos.y > ctx.t.water_y {
            let seed = ctx.rng.next_f32();
            let team = ctx.boomerang[i].team;
            let area = if muted {
                0.0
            } else {
                ctx.paint.splat(
                    ctx.world,
                    vec3(c.x, c.y + 0.15, c.z),
                    0.8,
                    team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                )
            };
            credit(ctx, i, area);
        }
        ctx.boomerang[i].state = BoomerangState::Dead;
    }
}

/// JS `on('splatted')` (L418-426): a splatted thrower fizzles its live
/// boomerang (armed / fizzling ones ride out their own path).
pub(crate) fn owner_splatted(boomerang: &mut [BoomerangItem], owner_idx: usize) {
    for it in boomerang.iter_mut() {
        if it.owner_idx != owner_idx
            || matches!(
                it.state,
                BoomerangState::Dead | BoomerangState::Armed | BoomerangState::Fizzle
            )
        {
            continue;
        }
        it.state = BoomerangState::Fizzle;
        it.t = 0.0;
        it.vel *= 0.25;
    }
}

/// JS `burst(it)` (L428): the small Splat-Bomb pop.
fn burst(ctx: &mut Ctx, i: usize, muted: bool) {
    let s = &ctx.sb.boomerang;
    let (r, dmax, dmin, pr) = (s.radius, s.damage_max, s.damage_min, s.paint_radius);
    blast(ctx, i, r, dmax, dmin, pr, false, muted);
}

/// JS `blast(it, radius, dmgMax, dmgMin, paintR, big)` (L429-457): the
/// record, the ink fan, the falloff damage and the device damage.
#[allow(clippy::too_many_arguments)]
fn blast(
    ctx: &mut Ctx,
    i: usize,
    radius: f32,
    dmg_max: f32,
    dmg_min: f32,
    paint_r: f32,
    big: bool,
    muted: bool,
) {
    let c = ctx.boomerang[i].pos;
    let ghost = ctx.boomerang[i].ghost;
    if !ghost {
        // JS L431: `netRec(it.owner, 'boomerang', [1, gid, pos×3, big])`.
        let (os, team, gid) = (
            ctx.boomerang[i].owner_slot,
            ctx.boomerang[i].team,
            ctx.boomerang[i].gid,
        );
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: os,
            team,
            sub_kind: "boomerang".to_string(),
            data: vec![
                1.0,
                gid as f32,
                r3(c.x),
                r3(c.y),
                r3(c.z),
                if big { 1.0 } else { 0.0 },
            ],
        });
    }
    ctx.boomerang[i].blasted = true;
    // ink: on the floor under it (the burst is in the air) plus around the
    // burst itself (L435-441). RNG order: centre `seed`, then per ring drop
    // `(a, rr, radius, seed)`, then the optional air splat `seed`.
    let g = ctx.world.raycast(c, DOWN, 3.2, false);
    let pc = if g.hit {
        vec3(g.point.x, g.point.y + 0.2, g.point.z)
    } else {
        c
    };
    let team = ctx.boomerang[i].team;
    let mut area = 0.0;
    let seed = ctx.rng.next_f32();
    if !muted {
        area += ctx.paint.splat(
            ctx.world,
            pc,
            paint_r,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    let n = if big { 5 } else { 4 };
    for _ in 0..n {
        let a = ctx.rng.next_f32() * TAU;
        let rr = paint_r * (0.55 + ctx.rng.next_f32() * 0.45);
        let rad = 0.5 + ctx.rng.next_f32() * 0.4;
        let s2 = ctx.rng.next_f32();
        if muted {
            continue;
        }
        area += ctx.paint.splat(
            ctx.world,
            vec3(pc.x + a.cos() * rr, pc.y + 0.4, pc.z + a.sin() * rr),
            rad,
            team,
            &SplatOpts {
                seed: s2,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    if !g.hit || c.y - g.point.y > 1.2 {
        let seed = ctx.rng.next_f32();
        if !muted {
            area += ctx.paint.splat(
                ctx.world,
                c,
                paint_r * 0.6,
                team,
                &SplatOpts {
                    seed,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            );
        }
    }
    credit(ctx, i, area);
    let (radius, team, ghost, oi, os) = (
        radius,
        ctx.boomerang[i].team,
        ctx.boomerang[i].ghost,
        ctx.boomerang[i].owner_idx,
        ctx.boomerang[i].owner_slot,
    );
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        let ch = chest(e.pos, e.form);
        let d = ch.distance(c);
        if d > radius || !ctx.world.los(c, ch) {
            continue;
        }
        let k = 1.0 - clamp((d - 0.8) / (radius - 0.8), 0.0, 1.0);
        hits.push((ei, lerp(dmg_min, dmg_max, k * k)));
    }
    for (ei, dmg) in hits {
        ctx.hit_routed(ghost, oi, os, team, ei, dmg);
    }
    ctx.damage_area(c, radius, if big { 60.0 } else { 35.0 }, team, muted);
    ctx.boomerang[i].state = BoomerangState::Dead;
}

/// JS `streak(it, r)` (L459-466): a light ink streak on the floor under the
/// flight (thinner on the way back).
fn streak(ctx: &mut Ctx, i: usize, r: f32, ghost: bool) {
    {
        let (pos, prev) = (ctx.boomerang[i].pos, ctx.boomerang[i].prev);
        ctx.boomerang[i].trail += pos.distance(prev);
    }
    if ctx.boomerang[i].trail < 0.8 {
        return;
    }
    ctx.boomerang[i].trail = 0.0;
    let pos = ctx.boomerang[i].pos;
    let g = ctx.world.raycast(pos, DOWN, 2.6, false);
    if g.hit {
        // JS L464: radius draw then seed draw (positional args first).
        let rad = r * (0.85 + ctx.rng.next_f32() * 0.3);
        let seed = ctx.rng.next_f32();
        let team = ctx.boomerang[i].team;
        let area = if ghost {
            0.0
        } else {
            ctx.paint.splat(
                ctx.world,
                vec3(g.point.x, g.point.y + 0.1, g.point.z),
                rad,
                team,
                &SplatOpts {
                    seed,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            )
        };
        credit(ctx, i, area);
    }
}

/// JS `SUB_KITS.boomerang.blocked(a)` (L603) via `active` (L169): one live
/// boomerang per owner — a second throw is refused.
#[must_use]
pub fn blocked(boomerang: &[BoomerangItem], owner_idx: usize) -> bool {
    boomerang
        .iter()
        .any(|b| b.owner_idx == owner_idx && b.state != BoomerangState::Dead)
}
