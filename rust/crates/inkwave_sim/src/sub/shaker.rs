//! Shaker Bomb kit sub — the sim half of `src/game/kits/shaker.js` (618
//! lines), batch two of Task 34 (spec §5).
//!
//! Hold the sub button to charge (`hold` L192-222 rides the weapon runner
//! as [`crate::weapon::ShakerCharge`]); release throws the can, which pops
//! once per charge level and hops forward between pops.
//!
//! Keep in lockstep with upstream on sync:
//!   - `hold` (L192-222)        -> [`crate::weapon::WeaponRunner::shaker_hold`]
//!   - `drop` / `'splatted'` (L254-262, L577) -> runner `reset` + the
//!     end-of-frame stale sweep (see the module notes on deviations)
//!   - `use` (L264-275)         -> [`use_shaker`]
//!   - `spawn` (L277-288)       -> [`spawn`]
//!   - `ghost` (L290-301)       -> [`ghost`]
//!   - `update` (L406-457)      -> [`step`]
//!   - `contactFloor` (L459-464)-> [`contact_floor`]
//!   - `arm` (L466-471)         -> [`arm`]
//!   - `drip` (L480-485)        -> [`drip`]
//!   - `blast` (L488-525)       -> [`blast`]
//!   - `tick` (L596-608)        -> [`tick`]
//!
//! Presentation-only branches dropped per the M3 range rule (spec §0): the
//! prop / lamp / hand meshes, `clink` / audio / rumble, `handFx`, `pips`,
//! the danger ring (`groundUnder` / `gp` / `gn` / `gOk` — only the ring
//! reads them), `lamps`, `dispose`, the `sub:charge` / `bomb:arm` /
//! `bomb:explode` / `shake` emits (not in the netmatch `FORWARD` list) and
//! the bot AI (`newState`'s `botLevel` / `botMax`, `botHold`, `botLob`,
//! `waggles`, the `bot` registration block — the sim's bot layer owns
//! aiming). `spin` / the mesh tumble randoms are visual and their draws are
//! not consumed (determinism rule).
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-9 is lifted: kit kinds now deploy through this module.
//!   - The stale-charge sweep (`tick` L606 over `G.actors`) runs per-runner
//!     at the end of [`crate::weapon::WeaponRunner::update`] instead (the
//!     sim has no global actor list there); the `'splatted'` drop (L577)
//!     lands in the runner `reset` on the dead-actor branch (one-frame
//!     early vs JS's event timing — the charge never throws either way).
//!   - `hold`'s `paid` gate drops the Bomb Barrage branch (D-T34-3/D-T34-6:
//!     no specials in the sim), and `wag` is always 0 (no waggle input).

use glam::{Vec3, vec3};

use crate::actor::Form;
use crate::paint::SplatOpts;
use crate::weapon::{DropSpec, SimEvent, throw_velocity};

use super::{Ctx, DOWN, GRAV, TAU, clamp, lerp, r2};

/// JS `RAD` (L34): the world contact radius the can's centre keeps off a
/// surface.
const RAD: f32 = 0.19;

/// One live shaker can (JS `spawn` L277-288 minus the mesh / `spin` /
/// `gp` / `gn` / `gOk` presentation fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShakerItem {
    /// JS `it.owner` — actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`).
    pub owner_slot: usize,
    /// The owner's team at spawn (JS `a.team`; `it.team` mirrors it).
    pub owner_team: usize,
    /// JS `it.team`.
    pub team: usize,
    /// JS `it.level` — the charge level the throw carried (1..=3).
    pub level: u32,
    /// JS `it.left` — blasts still owed.
    pub left: u32,
    /// JS `it.blasts` — blasts fired so far.
    pub blasts: u32,
    /// JS `it.pos` — the can's centre.
    pub pos: Vec3,
    /// JS `it.vel`.
    pub vel: Vec3,
    /// JS `it.dir` — the horizontal hop direction (wall-bounce mirrored).
    pub dir: Vec3,
    /// JS `it.age`.
    pub age: f32,
    /// JS `it.fuse` — the first-blast countdown; `-1` is the JS "off"
    /// sentinel (kept as a plain `f32` like JS).
    pub fuse: f32,
    /// JS `it.next` — the between-blasts countdown (`gap`).
    pub next: f32,
    /// JS `it.trail` — the trail-drip distance accumulator.
    pub trail: f32,
    /// JS `it.ground` — the resting-contact window timer.
    pub ground: f32,
    /// JS `it.armed` — found a floor (or aged out) and started its fuse.
    pub armed: bool,
    /// JS `it.state === 'dead'`.
    pub dead: bool,
    /// JS `it.ghost`.
    pub ghost: bool,
    /// JS `it.gid` (D-T34-5: ghosts keep the wire form, keyed with
    /// `src_nid`).
    pub gid: u32,
    /// The remote owner's `nid` for a ghost (D-T34-5).
    pub src_nid: Option<u8>,
}

/// JS `use(subs, a, sub)` (L264-275): the throw on release. `level` is the
/// runner's fresh charge (`st && G.time - st.t1 < 0.25 ? st.level : 1`,
/// L265 — computed by the runner before the call).
pub(crate) fn use_shaker(ctx: &mut Ctx, actor_idx: usize, level: u32) {
    let a = &ctx.actors[actor_idx];
    let (slot, team) = (a.slot, a.team);
    let speed = ctx.sb.shaker.throw_speed;
    // JS L269-270: released 1.35 m up on the shared `throwVelocity` arc.
    let pos = vec3(a.pos.x, a.pos.y + 1.35, a.pos.z);
    let vel = throw_velocity(a, speed);
    let gid = ctx.gid_next(slot);
    spawn(
        ctx, actor_idx, slot, team, pos, vel, level, false, gid, None,
    );
    // JS L272: `netRec(a, 'shaker', [0, gid, pos×3, vel×3, level])` — the
    // kit record layout has no kind string at `d[2]` (unlike `subs`).
    ctx.events.push(SimEvent::SubSpawn {
        owner_slot: slot,
        team,
        sub_kind: "shaker".to_string(),
        data: vec![
            0.0,
            gid as f32,
            r2(pos.x),
            r2(pos.y),
            r2(pos.z),
            r2(vel.x),
            r2(vel.y),
            r2(vel.z),
            level as f32,
        ],
    });
    ctx.events.push(SimEvent::SubUse {
        owner_slot: slot,
        team,
        kind: super::SubKind::Shaker,
    });
}

/// JS `spawn(a, sub, pos, vel, level, ghost, gid)` (L277-288) minus the
/// mesh / tumble / `spin` (presentation; its draws are not consumed).
#[allow(clippy::too_many_arguments)]
fn spawn(
    ctx: &mut Ctx,
    owner_idx: usize,
    owner_slot: usize,
    owner_team: usize,
    pos: Vec3,
    vel: Vec3,
    level: u32,
    ghost: bool,
    gid: u32,
    src_nid: Option<u8>,
) {
    let mut dir = vec3(vel.x, 0.0, vel.z);
    if dir.length_squared() < 1e-6 {
        // JS L278: degenerate horizontal → the owner's aim yaw.
        let a = &ctx.actors[owner_idx];
        dir = vec3(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
    }
    let dir = dir.normalize();
    ctx.shaker.push(ShakerItem {
        owner_idx,
        owner_slot,
        owner_team,
        team: owner_team,
        level,
        left: level,
        blasts: 0,
        pos,
        vel,
        dir,
        age: 0.0,
        fuse: -1.0,
        next: 0.0,
        trail: 0.0,
        ground: 0.0,
        armed: false,
        dead: false,
        ghost,
        gid,
        src_nid,
    });
}

/// JS `ghost(a, d)` (L290-301): a remote throw / blast / end record. The
/// netmatch replay wraps `'k'` records in `mute++` (netmatch.js L490-491),
/// so the blast op runs muted: paint draws, no splats, no hits.
pub(crate) fn ghost(ctx: &mut Ctx, actor_idx: usize, nid: u8, data: &[f32]) {
    let (op, gid) = (data[0], data[1] as u32);
    if op == 0.0 {
        // JS L293 dedups on the globally-unique `gid`; the sim keys ghosts
        // on `(src_nid, gid)` (D-T34-5).
        if ctx
            .shaker
            .iter()
            .any(|x| x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        if data.len() < 9 {
            return;
        }
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        let pos = vec3(data[2], data[3], data[4]);
        let vel = vec3(data[5], data[6], data[7]);
        // JS L294: `clamp(d[8] | 0, 1, 3)`.
        let level = (data[8] as i32).clamp(1, 3) as u32;
        spawn(
            ctx,
            actor_idx,
            slot,
            team,
            pos,
            vel,
            level,
            true,
            gid,
            Some(nid),
        );
        return;
    }
    let idx = ctx
        .shaker
        .iter()
        .position(|x| x.ghost && x.src_nid == Some(nid) && x.gid == gid && !x.dead);
    let Some(i) = idx else { return };
    if op == 1.0 {
        if data.len() < 5 {
            return;
        }
        // JS L299: jump the can to the owner's blast spot and pop it.
        ctx.shaker[i].pos = vec3(data[2], data[3], data[4]);
        ctx.shaker[i].fuse = -1.0;
        ctx.shaker[i].next = 0.0;
        blast(ctx, i, true);
    } else if op == 2.0 {
        ctx.shaker[i].dead = true;
    }
}

/// JS `SUB_KITS.shaker.tick` (L596-608): the reverse walk with the ghost
/// mute wrap and the dead splice (the `[2, gid]` end record only for a
/// sunk / abandoned local can that still owed blasts). The stale-charge
/// sweep (L606) is runner-side (see the module docs).
pub(crate) fn tick(ctx: &mut Ctx, dt: f32) {
    let mut i = ctx.shaker.len();
    while i > 0 {
        i -= 1;
        let muted = ctx.shaker[i].ghost;
        step(ctx, i, dt, muted);
        if ctx.shaker[i].dead {
            if !ctx.shaker[i].ghost && ctx.shaker[i].left > 0 {
                let (slot, team, gid) = (
                    ctx.shaker[i].owner_slot,
                    ctx.shaker[i].team,
                    ctx.shaker[i].gid,
                );
                ctx.events.push(SimEvent::SubEnd {
                    owner_slot: slot,
                    team,
                    sub_kind: Some("shaker".to_string()),
                    data: vec![2.0, gid as f32],
                });
            }
            ctx.shaker.remove(i);
        }
    }
}

/// JS `update(it, dt)` (L406-457): swept flight, resting contact, the
/// thin trail, and the fuse / gap blast chain. The mesh pose block
/// (L493-505, JS "look") is presentation.
fn step(ctx: &mut Ctx, i: usize, dt: f32, muted: bool) {
    let trail_every = ctx.sb.shaker.trail_every;
    ctx.shaker[i].age += dt;
    // ---- motion: gravity, swept centre vs the world (JS L410-421)
    ctx.shaker[i].vel.y -= GRAV * dt;
    let prev = ctx.shaker[i].pos;
    let vel = ctx.shaker[i].vel;
    ctx.shaker[i].pos += vel * dt;
    let mut floor = false;
    let h = ctx.world.segment(prev, ctx.shaker[i].pos, false);
    if h.hit {
        let n = h.normal;
        ctx.shaker[i].pos = h.point + n * RAD;
        if n.y > 0.6 {
            floor = contact_floor(ctx, i, n, dt);
        } else {
            let vn = ctx.shaker[i].vel.dot(n);
            if vn < 0.0 {
                ctx.shaker[i].vel += n * (-vn * 1.5);
                ctx.shaker[i].vel *= 0.62;
            }
            // hops carry on away from the wall it bounced off (JS L419)
            let mut dir = ctx.shaker[i].dir;
            let dn = dir.x * n.x + dir.z * n.z;
            if dn < 0.0 && n.y.abs() < 0.6 {
                dir.x -= 2.0 * dn * n.x;
                dir.z -= 2.0 * dn * n.z;
                dir.y = 0.0;
                ctx.shaker[i].dir = dir.normalize();
            }
        }
    }
    if !floor {
        // resting probe under the centre (JS L423-425)
        let pos = ctx.shaker[i].pos;
        let g = ctx.world.raycast(pos, DOWN, RAD, false);
        if g.hit && g.normal.y > 0.6 {
            ctx.shaker[i].pos.y += RAD - g.dist;
            floor = contact_floor(ctx, i, g.normal, dt);
        }
    }
    ctx.shaker[i].ground = if floor {
        0.12
    } else {
        (ctx.shaker[i].ground - dt).max(0.0)
    };
    // ---- thin trail: shaken-out droplets while airborne (JS L428-432;
    // a ghost's drops arrive as its owner's records)
    let vel = ctx.shaker[i].vel;
    let hs = vel.x.hypot(vel.z);
    if ctx.shaker[i].ground == 0.0 && hs > 1.5 {
        ctx.shaker[i].trail += hs * dt;
        while ctx.shaker[i].trail >= trail_every {
            ctx.shaker[i].trail -= trail_every;
            if !ctx.shaker[i].ghost {
                drip(ctx, i);
            }
        }
    }
    // ---- fuse (first landing) → blast → hop → blast … (JS L436-439; a
    // ghost holds at the brink until its owner's blast record arrives)
    if ctx.shaker[i].fuse >= 0.0 {
        ctx.shaker[i].fuse -= dt;
        if ctx.shaker[i].fuse <= 0.0 {
            if ctx.shaker[i].ghost {
                ctx.shaker[i].fuse = 1e-3;
            } else {
                ctx.shaker[i].fuse = -1.0;
                blast(ctx, i, muted);
            }
        }
    } else if ctx.shaker[i].next > 0.0 {
        ctx.shaker[i].next -= dt;
        if ctx.shaker[i].next <= 0.0 {
            if ctx.shaker[i].ghost {
                ctx.shaker[i].next = 1e-3;
            } else {
                blast(ctx, i, muted);
            }
        }
    }
    if ctx.shaker[i].ghost && ctx.shaker[i].age > 20.0 {
        // its owner left mid-throw (JS L440)
        ctx.shaker[i].dead = true;
    }
    if ctx.shaker[i].dead {
        return;
    }
    if ctx.shaker[i].pos.y < ctx.t.water_y - 1.8 {
        // sank: no blast (JS L442)
        ctx.shaker[i].dead = true;
        return;
    }
    if !ctx.shaker[i].armed && ctx.shaker[i].age > 6.0 {
        // never found a floor (wedged): go anyway (JS L443)
        arm(ctx, i);
    }
}

/// JS `contactFloor` (L459-464): the dull landing bounce, or the resting
/// slide to a stop; arms the can on first contact.
fn contact_floor(ctx: &mut Ctx, i: usize, n: Vec3, dt: f32) -> bool {
    let vn = ctx.shaker[i].vel.dot(n);
    if vn < -2.2 {
        ctx.shaker[i].vel += n * (-vn);
        ctx.shaker[i].vel *= 0.45;
        ctx.shaker[i].vel += n * (-vn * 0.3);
    } else if vn < 0.0 {
        ctx.shaker[i].vel += n * (-vn);
        let f = (-6.0 * dt).exp();
        ctx.shaker[i].vel.x *= f;
        ctx.shaker[i].vel.z *= f;
    }
    if !ctx.shaker[i].armed {
        arm(ctx, i);
    }
    true
}

/// JS `arm` (L466-471) minus the audio / `bomb:arm` emit / `groundUnder`
/// (danger-ring presentation).
fn arm(ctx: &mut Ctx, i: usize) {
    ctx.shaker[i].armed = true;
    ctx.shaker[i].fuse = ctx.sb.shaker.fuse;
}

/// JS `drip` (L480-485): one trail droplet shaken from the vent. RNG order
/// `[vx, vy, vz, radius, seed]` (JS evaluates the args then `spawnDrop`'s
/// own seed draw, weapons.js L1420-1426; the `vis` draw is presentation).
fn drip(ctx: &mut Ctx, i: usize) {
    let trail_radius = ctx.sb.shaker.trail_radius;
    let mut p = ctx.shaker[i].pos;
    p.y -= 0.1;
    let vel = ctx.shaker[i].vel;
    let r1 = ctx.rng.next_f32();
    let r2 = ctx.rng.next_f32();
    let r3 = ctx.rng.next_f32();
    let r4 = ctx.rng.next_f32();
    let seed = ctx.rng.next_f32();
    let v = vec3(
        vel.x * 0.2 + (r1 - 0.5) * 0.5,
        -2.2 - r2,
        vel.z * 0.2 + (r3 - 0.5) * 0.5,
    );
    let spec = DropSpec {
        life: 1.2,
        straight: 0.0,
        size: 0.045,
        radius: trail_radius * (0.85 + r4 * 0.3),
        damage: 0.0,
        dmg_far: 0.0,
        trail_start: 0.0,
        trail_every: 0.0,
        trail_radius: 0.3,
        grav: 22.0,
        drag: 0.4,
        delay: 0.0,
    };
    let owner_idx = ctx.shaker[i].owner_idx;
    ctx.projectiles
        .push_drop(&ctx.actors[owner_idx], &spec, p, v, seed);
}

/// JS `blast` (L488-525): the `[1, gid, pos]` record, the floor + satellite
/// splats (RNG order `[seed, (ang, r, size, seed) × 3]`), the turf credit,
/// the smaller-Splat-Bomb damage with LOS, `damageArea(40)` and the hop to
/// the next pop. The `explosion` / audio / `shake` / `bomb:explode` /
/// rumble / boss-splash lines are presentation or out of scope.
fn blast(ctx: &mut Ctx, i: usize, muted: bool) {
    let s = &ctx.sb.shaker;
    let (radius, dmax, dmin, paint_radius, gap, hop_speed, hop_up) = (
        s.radius,
        s.damage_max,
        s.damage_min,
        s.paint_radius,
        s.gap,
        s.hop_speed,
        s.hop_up,
    );
    let c = ctx.shaker[i].pos;
    if !ctx.shaker[i].ghost {
        let (slot, team, gid) = (
            ctx.shaker[i].owner_slot,
            ctx.shaker[i].team,
            ctx.shaker[i].gid,
        );
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "shaker".to_string(),
            data: vec![1.0, gid as f32, r2(c.x), r2(c.y), r2(c.z)],
        });
    }
    ctx.shaker[i].blasts += 1;
    ctx.shaker[i].left -= 1;
    // ---- ink: the floor it stands on + three satellite splats (JS L495-502)
    let team = ctx.shaker[i].team;
    let g = ctx
        .world
        .raycast(vec3(c.x, c.y + 0.2, c.z), DOWN, 3.5, false);
    let center = if g.hit { g.point + g.normal * 0.1 } else { c };
    let mut area = 0.0;
    let seed = ctx.rng.next_f32();
    if !muted {
        area += ctx.paint.splat(
            ctx.world,
            center,
            paint_radius,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    for _ in 0..3 {
        let ang = ctx.rng.next_f32() * TAU;
        let r = paint_radius * (0.55 + ctx.rng.next_f32() * 0.4);
        let rad = 0.45 + ctx.rng.next_f32() * 0.3;
        let sd = ctx.rng.next_f32();
        if muted {
            continue;
        }
        let p = vec3(c.x + ang.cos() * r, c.y + 0.3, c.z + ang.sin() * r);
        area += ctx.paint.splat(
            ctx.world,
            p,
            rad,
            team,
            &SplatOpts {
                seed: sd,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    let (os, ot) = (ctx.shaker[i].owner_slot, ctx.shaker[i].owner_team);
    ctx.events.push(SimEvent::Turf {
        owner_slot: os,
        team: ot,
        area,
    });
    // ---- damage: line of sight from just above the can (JS L514-522)
    let los_from = vec3(c.x, c.y + 0.3, c.z);
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        let mut v = e.pos;
        v.y += if e.form == Form::Squid { 0.3 } else { 0.7 };
        let d = v.distance(c);
        if d > radius || !ctx.world.los(los_from, v) {
            continue;
        }
        let k = 1.0 - clamp((d - 0.8) / (radius - 0.8), 0.0, 1.0);
        hits.push((ei, lerp(dmin, dmax, k * k)));
    }
    let (ghost, oi, os) = (
        ctx.shaker[i].ghost,
        ctx.shaker[i].owner_idx,
        ctx.shaker[i].owner_slot,
    );
    for (ei, dmg) in hits {
        ctx.hit_routed(ghost, oi, os, team, ei, dmg);
    }
    ctx.damage_area(c, radius, 40.0, team, muted);
    if ctx.shaker[i].left > 0 {
        // the blast kicks it forward and up; the next one goes off `gap`
        // later (JS L523-524)
        let dir = ctx.shaker[i].dir;
        ctx.shaker[i].vel = vec3(dir.x * hop_speed, hop_up, dir.z * hop_speed);
        ctx.shaker[i].next = gap;
        ctx.shaker[i].ground = 0.0;
    } else {
        ctx.shaker[i].dead = true;
    }
}
