//! Waddle Bomb kit sub — the sim half of `src/game/kits/waddle.js` (768
//! lines), batch two of Task 34 (spec §5).
//!
//! Thrown like the other subs; on landing it *senses* for a foe inside its
//! circle, locks on one, wakes, then walks after it and explodes on contact
//! (or when its fuse / life / travel budget runs out). Enemy fire pops it.
//!
//! Keep in lockstep with upstream on sync:
//!   - `use` (L264-272)         -> [`use_waddle`]
//!   - `spawn` (L277-288)       -> [`spawn`]
//!   - `ghost` (L290-311)       -> [`ghost`]
//!   - `scan` (L316-326)        -> [`scan`]
//!   - `fly` (L328-345) / `land` (L348-366) -> [`fly`] / [`land`]
//!   - `sense` (L368-378)       -> [`sense`]
//!   - `lock` (L380-389)        -> [`lock`]
//!   - `wake` (L391-401)        -> [`wake`]
//!   - `walk` (L427-472) / `follow` (L474-484) -> [`walk`] / [`follow`]
//!   - `stepWalk` (L486-508) / `fallStep` (L509-527) -> [`step_walk`] / [`fall_step`]
//!   - `blast` (L573-603)       -> [`blast`]
//!   - `hurt` (L605-612) / `pop` (L614-622) -> [`hurt`] / [`pop`]
//!   - `tick` (L693-712)        -> [`tick`]
//!   - `blockShot` (L720-728) / `blockRay` (L729-741) / `damageArea`
//!     (L742-748) -> the `SubSystem` block hooks fan into [`hurt`]
//!   - `netHurt` (L691)         -> [`SubSystem::net_hurt`]
//!
//! Presentation-only branches dropped per the M3 range rule (spec §0): the
//! prop mesh / `makeMesh` / `makeRing`, `MID` / `HIP` / `STRIDE`, `look`
//! (L485-501), `ringFx`, `previewFx`, `blinkLamp`, `noise` / `hush`, audio,
//! `near`, `G.fx`, `shake` / rumble, the boss splash and the `sub:land` /
//! `sub:lock` / `sub:destroyed` / `sub:end` emits (not in the netmatch
//! `FORWARD` list). The `sense` jitter `Math.random()` (L374) is a mesh
//! rotation draw and is not consumed (determinism rule). The bot layer
//! (`botLob` L255-262, `threatOf`, the `bot` block) is out of scope.
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-9 is lifted: kit kinds now deploy through this module.
//!   - D-T34-7: the sim's `Match` owns no nav graph (`G.nav`), so `walk`
//!     runs JS's own `!nav` branch — `repath` is a no-op, `goal` aims
//!     straight at the foe, and the nav-node step-up hop never starts. The
//!     wall-slide, ledge-drop (`air` / `fallStep`), stuck / life / travel
//!     give-ups and the `[4, …]` walk records all stay.
//!   - `sp` (Bomb Barrage, L281) is always false (D-T34-3): credit is the
//!     plain turf event.
//!   - `alive` (L314) drops the `!e.superJumpState` term (no super jump in
//!     the sim, D-T34-2).
//!   - The lock / walk records carry the *target's actor-list index* where
//!     JS carries `tgt.nid` (the sim has no nid table); the replication
//!     layer rewrites idx → wire nid outbound and nid → idx inbound
//!     (same pattern as the D-T34-5 gid rewrite).

use glam::{Vec3, vec3};

use crate::actor::Form;
use crate::paint::SplatOpts;
use crate::weapon::{SimEvent, segment_capsule_dist, throw_velocity};

use super::{Ctx, GRAV, TAU, clamp, lerp, r2};

/// JS `it.state` (the `tick` switch, waddle.js L697-704).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WaddleState {
    /// `fly` — in the air after the throw.
    Fly,
    /// `sense` — landed, circle open, looking for a foe.
    Sense,
    /// `wake` — locked on, the little wake-up pause.
    Wake,
    /// `walk` — chasing its foe.
    Walk,
    /// `dead` — removed at the end of this update pass.
    Dead,
}

/// JS `it.net` (L303): the owner's last reported walk pose.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct WaddleNet {
    /// JS `N.x` — the owner's reported position.
    pub x: f32,
    /// JS `N.y`.
    pub y: f32,
    /// JS `N.z`.
    pub z: f32,
    /// JS `N.h` — the reported heading.
    pub h: f32,
}

/// One live waddle bomb (JS `spawn` L277-288 minus the mesh / ring / audio
/// / `spin` / `phase` / `blink` presentation fields and the never-set
/// `path` / `pi` / `hop` nav fields, D-T34-7).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WaddleItem {
    /// JS `it.owner` — actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`).
    pub owner_slot: usize,
    /// The owner's team at spawn (JS `a.team`; `it.team` mirrors it).
    pub owner_team: usize,
    /// JS `it.team`.
    pub team: usize,
    /// JS `it.pos` — the bomb's centre.
    pub pos: Vec3,
    /// JS `it.vel` — flight velocity; the ledge-drop carry while `air`.
    pub vel: Vec3,
    /// JS `it.state`.
    pub state: WaddleState,
    /// JS `it.t` — the per-state timer (the tick adds it before the switch).
    pub t: f32,
    /// JS `it.age`.
    pub age: f32,
    /// JS `it.hp` — enemy fire pops it when this reaches 0.
    pub hp: f32,
    /// JS `it.heading` — the facing / walk direction.
    pub heading: f32,
    /// JS `it.target` — the locked foe (actor-list index).
    pub target: Option<usize>,
    /// JS `it.travel` — ground distance walked since the lock.
    pub travel: f32,
    /// JS `it.walkT` — time spent walking.
    pub walk_t: f32,
    /// JS `it.lostT` — time since its foe died.
    pub lost_t: f32,
    /// JS `it.air` — falling off a ledge.
    pub air: bool,
    /// JS `it.prog` — the stuck-detection window (`t` / `x` / `z`).
    pub prog_t: f32,
    /// JS `it.prog.x`.
    pub prog_x: f32,
    /// JS `it.prog.z`.
    pub prog_z: f32,
    /// JS `it.sendT` — the walk-record countdown.
    pub send_t: f32,
    /// JS `it.net` — the ghost's last reported pose.
    pub net: Option<WaddleNet>,
    /// JS `it.blasted` — it went off (suppresses the `[2, gid]` end record).
    pub blasted: bool,
    /// JS `it.why` — the blast / end reason (`'popped'` drives the end
    /// record's flag).
    pub why: String,
    /// JS `it.ghost`.
    pub ghost: bool,
    /// JS `it.gid` (D-T34-5: ghosts keep the wire form, keyed with
    /// `src_nid`).
    pub gid: u32,
    /// The remote owner's `nid` for a ghost (D-T34-5).
    pub src_nid: Option<u8>,
}

/// JS `angleDiff(a, b)`: signed shortest angular difference `b − a`.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > std::f32::consts::PI {
        d -= TAU;
    } else if d < -std::f32::consts::PI {
        d += TAU;
    }
    d
}

/// JS `use(subs, a, sub)` (L264-272): the throw.
pub(crate) fn use_waddle(ctx: &mut Ctx, actor_idx: usize) {
    let a = &ctx.actors[actor_idx];
    let (slot, team) = (a.slot, a.team);
    let speed = ctx.sb.waddle.throw_speed;
    // JS L266-267: released 1.35 m up on the shared `throwVelocity` arc.
    let pos = vec3(a.pos.x, a.pos.y + 1.35, a.pos.z);
    let vel = throw_velocity(a, speed);
    let heading = a.aim_yaw;
    let gid = ctx.gid_next(slot);
    spawn(
        ctx, actor_idx, slot, team, pos, vel, heading, false, gid, None,
    );
    // JS L269: `netRec(a, 'waddle', [0, gid, pos×3, vel×3])`.
    ctx.events.push(SimEvent::SubSpawn {
        owner_slot: slot,
        team,
        sub_kind: "waddle".to_string(),
        data: vec![
            0.0,
            gid as f32,
            r2(pos.x),
            r2(pos.y),
            r2(pos.z),
            r2(vel.x),
            r2(vel.y),
            r2(vel.z),
        ],
    });
    ctx.events.push(SimEvent::SubUse {
        owner_slot: slot,
        team,
        kind: super::SubKind::Waddle,
    });
}

/// JS `spawn(a, sub, pos, vel, ghost, gid)` (L277-288) minus the mesh /
/// ring / `spin` / `phase` presentation fields.
#[allow(clippy::too_many_arguments)]
fn spawn(
    ctx: &mut Ctx,
    owner_idx: usize,
    owner_slot: usize,
    owner_team: usize,
    pos: Vec3,
    vel: Vec3,
    heading: f32,
    ghost: bool,
    gid: u32,
    src_nid: Option<u8>,
) {
    let hp = ctx.sb.waddle.hp as f32;
    ctx.waddle.push(WaddleItem {
        owner_idx,
        owner_slot,
        owner_team,
        team: owner_team,
        pos,
        vel,
        state: WaddleState::Fly,
        t: 0.0,
        age: 0.0,
        hp,
        heading,
        target: None,
        travel: 0.0,
        walk_t: 0.0,
        lost_t: 0.0,
        air: false,
        prog_t: 0.0,
        prog_x: 0.0,
        prog_z: 0.0,
        send_t: 0.0,
        net: None,
        blasted: false,
        why: String::new(),
        ghost,
        gid,
        src_nid,
    });
}

/// JS `ghost(a, d)` (L290-311): a remote throw / lock / walk / blast / end
/// record. The netmatch replay wraps `'k'` records in `mute++`, so the
/// blast op runs muted: paint draws, no splats, no hits. `nids` is the
/// roster nid table used to resolve the records' foe `nid` back to an
/// actor-list index (JS `byNid`, L289).
pub(crate) fn ghost(ctx: &mut Ctx, actor_idx: usize, nid: u8, data: &[f32], nids: &[u8]) {
    let (op, gid) = (data[0], data[1] as u32);
    // JS `byNid(d)` (L289): `-1` (and unknown nids) → no foe.
    let by_nid = |want: f32| -> Option<usize> {
        let want = want as i32;
        if want < 0 {
            return None;
        }
        nids.iter().position(|&n| i32::from(n) == want)
    };
    if op == 0.0 {
        // JS L293 dedups on the globally-unique `gid`; the sim keys ghosts
        // on `(src_nid, gid)` (D-T34-5).
        if ctx
            .waddle
            .iter()
            .any(|x| x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        if data.len() < 8 {
            return;
        }
        let a = &ctx.actors[actor_idx];
        let (slot, team, heading) = (a.slot, a.team, a.aim_yaw);
        let pos = vec3(data[2], data[3], data[4]);
        let vel = vec3(data[5], data[6], data[7]);
        spawn(
            ctx,
            actor_idx,
            slot,
            team,
            pos,
            vel,
            heading,
            true,
            gid,
            Some(nid),
        );
        return;
    }
    let idx = ctx.waddle.iter().position(|x| {
        x.ghost && x.src_nid == Some(nid) && x.gid == gid && x.state != WaddleState::Dead
    });
    let Some(i) = idx else { return };
    if op == 3.0 || op == 4.0 {
        let (x, y, z) = if op == 3.0 {
            if data.len() < 6 {
                return;
            }
            (data[3], data[4], data[5])
        } else {
            if data.len() < 7 {
                return;
            }
            (data[2], data[3], data[4])
        };
        if ctx.waddle[i].state == WaddleState::Fly {
            ctx.waddle[i].pos = vec3(x, y, z);
            land(ctx, i, vec3(x, y, z));
        }
        if op == 3.0 {
            ctx.waddle[i].pos = vec3(x, y, z);
            if ctx.waddle[i].state == WaddleState::Sense {
                let tgt = by_nid(data[2]);
                lock(ctx, i, tgt);
            }
            return;
        }
        if ctx.waddle[i].state == WaddleState::Sense {
            let tgt = by_nid(data[6]);
            lock(ctx, i, tgt);
        }
        ctx.waddle[i].net = Some(WaddleNet {
            x,
            y,
            z,
            h: data[5],
        });
        let t = by_nid(data[6]);
        if t.is_some() {
            ctx.waddle[i].target = t;
        }
    } else if op == 1.0 {
        if data.len() < 5 {
            return;
        }
        ctx.waddle[i].pos = vec3(data[2], data[3], data[4]);
        ctx.waddle[i].why = "net".to_string();
        blast(ctx, i, true);
    } else if op == 2.0 {
        let popped = data.get(2).copied().unwrap_or(0.0) != 0.0;
        if popped {
            pop(ctx, i);
        } else {
            ctx.waddle[i].state = WaddleState::Dead;
        }
    }
}

/// JS `scan(it, R)` (L316-326): the nearest foe inside the horizontal
/// circle, within `senseUp` above / below.
fn scan(ctx: &Ctx, i: usize, r: f32) -> Option<usize> {
    let it = &ctx.waddle[i];
    let sense_up = ctx.sb.waddle.sense_up;
    let mut best: Option<usize> = None;
    let mut bd = f32::INFINITY;
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == it.team || !e.alive {
            continue;
        }
        let d = (e.pos.x - it.pos.x).hypot(e.pos.z - it.pos.z);
        if d > r || (e.pos.y - it.pos.y).abs() > sense_up {
            continue;
        }
        if d < bd {
            bd = d;
            best = Some(ei);
        }
    }
    best
}

/// JS `fly(it, dt)` (L328-345): swept flight; a floor lands it, walls and
/// ceilings bounce it, deep water sinks it (`plop`, no blast).
fn fly(ctx: &mut Ctx, i: usize, dt: f32) {
    ctx.waddle[i].vel.y -= GRAV * dt;
    let prev = ctx.waddle[i].pos;
    let vel = ctx.waddle[i].vel;
    ctx.waddle[i].pos += vel * dt;
    let h = ctx.world.segment(prev, ctx.waddle[i].pos, false);
    if h.hit {
        let n = h.normal;
        if n.y > 0.6 {
            land(ctx, i, h.point);
            return;
        }
        ctx.waddle[i].pos = h.point + n * 0.15;
        let vn = ctx.waddle[i].vel.dot(n);
        ctx.waddle[i].vel += n * (-vn * 1.45);
        ctx.waddle[i].vel *= 0.55;
    }
    if ctx.waddle[i].pos.y < ctx.t.water_y - 1.8 {
        // JS `plop` (L346): sank, no blast.
        ctx.waddle[i].why = "sea".to_string();
        ctx.waddle[i].state = WaddleState::Dead;
    }
}

/// JS `land(it, h)` (L348-366) minus the ring / audio / `sub:land` emit.
fn land(ctx: &mut Ctx, i: usize, point: Vec3) {
    ctx.waddle[i].pos = point;
    ctx.waddle[i].vel = Vec3::ZERO;
    ctx.waddle[i].state = WaddleState::Sense;
    ctx.waddle[i].t = 0.0;
}

/// JS `sense(it, dt)` (L368-378): the circle opens over 0.3 s; a foe inside
/// locks it, else the fuse blows it in place.
fn sense(ctx: &mut Ctx, i: usize) {
    let (r0, fuse) = (ctx.sb.waddle.sense_radius, ctx.sb.waddle.fuse);
    if !ctx.waddle[i].ghost {
        let r = r0 * (1.0f32).min(ctx.waddle[i].t / 0.3 + 0.35);
        if let Some(tgt) = scan(ctx, i, r) {
            lock(ctx, i, Some(tgt));
            return;
        }
    }
    let t = ctx.waddle[i].t;
    if t >= fuse && !ctx.waddle[i].ghost {
        ctx.waddle[i].why = "fuse".to_string();
        blast(ctx, i, false);
    }
}

/// JS `lock(it, tgt)` (L380-389): the `[3, gid, foe, pos]` record, then the
/// wake-up pause. JS's `it.heading = it.m.outer.rotation.y` is a mesh
/// read-back — the sim keeps the current heading.
fn lock(ctx: &mut Ctx, i: usize, tgt: Option<usize>) {
    if !ctx.waddle[i].ghost {
        let (slot, team, gid) = (
            ctx.waddle[i].owner_slot,
            ctx.waddle[i].team,
            ctx.waddle[i].gid,
        );
        let p = ctx.waddle[i].pos;
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "waddle".to_string(),
            data: vec![
                3.0,
                gid as f32,
                tgt.map_or(-1.0, |x| x as f32),
                r2(p.x),
                r2(p.y),
                r2(p.z),
            ],
        });
    }
    ctx.waddle[i].target = tgt;
    ctx.waddle[i].state = WaddleState::Wake;
    ctx.waddle[i].t = 0.0;
}

/// JS `wake(it, dt)` (L391-401): steer toward the foe (or the ghost's
/// reported heading), then off it goes after 0.32 s.
fn wake(ctx: &mut Ctx, i: usize, dt: f32) {
    let it = &ctx.waddle[i];
    let tgt = it
        .target
        .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
    let want = if let Some(ei) = tgt {
        let e = &ctx.actors[ei];
        Some((e.pos.x - it.pos.x, e.pos.z - it.pos.z))
    } else {
        it.net.map(|n| (n.h, 0.0))
    };
    if let Some(w) = want {
        let heading = if tgt.is_some() {
            angle_diff(it.heading, w.0.atan2(w.1))
        } else {
            angle_diff(it.heading, w.0)
        };
        let h = it.heading;
        ctx.waddle[i].heading = h + clamp(heading, -14.0 * dt, 14.0 * dt);
    }
    if ctx.waddle[i].t >= 0.32 {
        ctx.waddle[i].state = WaddleState::Walk;
        ctx.waddle[i].t = 0.0;
        let (x, z) = (ctx.waddle[i].pos.x, ctx.waddle[i].pos.z);
        ctx.waddle[i].prog_t = 0.0;
        ctx.waddle[i].prog_x = x;
        ctx.waddle[i].prog_z = z;
    }
}

/// JS `walk(it, dt)` (L427-472; ghosts take `follow`, L474-484). D-T34-7:
/// no nav graph, so this is JS's `!nav` branch — direct steering, no
/// repath / node hop.
fn walk(ctx: &mut Ctx, i: usize, dt: f32) {
    if ctx.waddle[i].ghost {
        follow(ctx, i, dt);
        return;
    }
    let s = &ctx.sb.waddle;
    let (sense_radius, trigger_dist, speed, turn_rate) =
        (s.sense_radius, s.trigger_dist, s.speed, s.turn_rate);
    let (life, max_travel, radius, dmax, dmin, paint_radius) = (
        s.life,
        s.max_travel,
        s.radius,
        s.damage_max,
        s.damage_min,
        s.paint_radius,
    );
    let _ = (radius, dmax, dmin, paint_radius); // used by blast, not here
    ctx.waddle[i].walk_t += dt;
    // lost its foe: the nearest other foe in range, else it goes off (L431-435)
    let alive_tgt = ctx.waddle[i]
        .target
        .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
    if alive_tgt.is_none() {
        let nt = scan(ctx, i, sense_radius);
        if let Some(nt) = nt {
            ctx.waddle[i].target = Some(nt);
            ctx.waddle[i].lost_t = 0.0;
        } else {
            ctx.waddle[i].lost_t += dt;
            if ctx.waddle[i].lost_t > 0.5 {
                ctx.waddle[i].why = "lost".to_string();
                blast(ctx, i, false);
                return;
            }
        }
    }
    let team = ctx.waddle[i].team;
    let mut c = ctx.waddle[i].pos;
    c.y += 0.3;
    // reached *any* foe → boom (L438-443)
    for e in ctx.actors.iter() {
        if e.team == team || !e.alive {
            continue;
        }
        let mut v = e.pos;
        v.y += if e.form == Form::Squid { 0.25 } else { 0.7 };
        if v.distance(c) < trigger_dist {
            ctx.waddle[i].why = "reached".to_string();
            blast(ctx, i, false);
            return;
        }
    }
    if ctx.waddle[i].walk_t > life || ctx.waddle[i].travel > max_travel {
        ctx.waddle[i].why = if ctx.waddle[i].travel > max_travel {
            "travel".to_string()
        } else {
            "life".to_string()
        };
        blast(ctx, i, false);
        return;
    }
    let mut moved = 0.0;
    let tgt = ctx.waddle[i]
        .target
        .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
    if ctx.waddle[i].air {
        moved = fall_step(ctx, i, dt);
    } else if let Some(ei) = tgt {
        let e = &ctx.actors[ei];
        let (tx, tz) = (e.pos.x, e.pos.z);
        let (px, pz) = (ctx.waddle[i].pos.x, ctx.waddle[i].pos.z);
        let heading = ctx.waddle[i].heading;
        let turn = angle_diff(heading, (tx - px).atan2(tz - pz));
        ctx.waddle[i].heading = heading + clamp(turn, -turn_rate * dt, turn_rate * dt);
        let spd = speed * if turn.abs() > 1.3 { 0.45 } else { 1.0 };
        moved = step_walk(ctx, i, dt, spd);
    }
    if ctx.waddle[i].state == WaddleState::Dead {
        return;
    }
    ctx.waddle[i].travel += moved;
    // stuck (wall / water's edge with no way round): give up (L461-466)
    ctx.waddle[i].prog_t += dt;
    if ctx.waddle[i].prog_t > 1.4 {
        let pd = (ctx.waddle[i].pos.x - ctx.waddle[i].prog_x)
            .hypot(ctx.waddle[i].pos.z - ctx.waddle[i].prog_z);
        if pd < 0.4 && !ctx.waddle[i].air {
            ctx.waddle[i].why = "stuck".to_string();
            blast(ctx, i, false);
            return;
        }
        ctx.waddle[i].prog_t = 0.0;
        ctx.waddle[i].prog_x = ctx.waddle[i].pos.x;
        ctx.waddle[i].prog_z = ctx.waddle[i].pos.z;
    }
    // online: where it is, for the ghosts on other screens (L467-471)
    ctx.waddle[i].send_t -= dt;
    if ctx.waddle[i].send_t <= 0.0 {
        ctx.waddle[i].send_t = 0.1;
        let (slot, team, gid) = (
            ctx.waddle[i].owner_slot,
            ctx.waddle[i].team,
            ctx.waddle[i].gid,
        );
        let p = ctx.waddle[i].pos;
        let h = ctx.waddle[i].heading;
        let t = ctx.waddle[i]
            .target
            .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive))
            .map_or(-1.0, |x| x as f32);
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "waddle".to_string(),
            data: vec![4.0, gid as f32, r2(p.x), r2(p.y), r2(p.z), r2(h), t],
        });
    }
}

/// JS `follow(it, dt)` (L474-484): a ghost eases onto its owner's reported
/// spot and heading.
fn follow(ctx: &mut Ctx, i: usize, dt: f32) {
    ctx.waddle[i].walk_t += dt;
    let Some(n) = ctx.waddle[i].net else { return };
    let k = 1.0 - (-14.0 * dt).exp();
    let pos = ctx.waddle[i].pos;
    ctx.waddle[i].pos = vec3(
        pos.x + (n.x - pos.x) * k,
        pos.y + (n.y - pos.y) * k,
        pos.z + (n.z - pos.z) * k,
    );
    let h = ctx.waddle[i].heading;
    ctx.waddle[i].heading = h + angle_diff(h, n.h) * k;
}

/// JS `stepWalk(it, dt, spd)` (L486-508): one step on the flat — slide
/// along walls, snap onto the ground (steps up to 0.45 m), ledge-drop into
/// `air`, never onto open water.
fn step_walk(ctx: &mut Ctx, i: usize, dt: f32, spd: f32) -> f32 {
    let heading = ctx.waddle[i].heading;
    let (hx, hz) = (heading.sin(), heading.cos());
    let step = spd * dt;
    let mut mx = hx * step;
    let mut mz = hz * step;
    let mut p = ctx.waddle[i].pos;
    p.y += 0.42;
    let w = ctx.world.raycast(p, vec3(hx, 0.0, hz), step + 0.2, false);
    if w.hit && w.normal.y.abs() < 0.5 {
        let n = w.normal;
        let dn = mx * n.x + mz * n.z;
        mx -= dn * n.x;
        mz -= dn * n.z;
        let l = mx.hypot(mz);
        if l > 1e-5 {
            let w2 = ctx
                .world
                .raycast(p, vec3(mx / l, 0.0, mz / l), l + 0.2, false);
            if w2.hit && w2.normal.y.abs() < 0.5 {
                mx = 0.0;
                mz = 0.0;
            }
        }
    }
    let (nx, nz) = (ctx.waddle[i].pos.x + mx, ctx.waddle[i].pos.z + mz);
    let moved = mx.hypot(mz);
    if moved < 1e-6 {
        return 0.0;
    }
    let gy = ctx
        .world
        .ground_height(nx, nz, ctx.waddle[i].pos.y + 0.45, false);
    if !gy.is_finite() {
        return 0.0; // the sea / nothing ahead: stay on the deck
    }
    if gy < ctx.waddle[i].pos.y - 0.4 {
        // off a ledge: drop down (L502-505)
        ctx.waddle[i].air = true;
        ctx.waddle[i].vel = vec3(mx / dt, 0.6, mz / dt);
        ctx.waddle[i].pos.x = nx;
        ctx.waddle[i].pos.z = nz;
        return moved;
    }
    ctx.waddle[i].pos = vec3(nx, gy, nz);
    moved
}

/// JS `fallStep(it, dt)` (L509-527): the ledge drop — gravity, wall stop,
/// land on the ground, sink in water.
fn fall_step(ctx: &mut Ctx, i: usize, dt: f32) -> f32 {
    let (x0, z0) = (ctx.waddle[i].pos.x, ctx.waddle[i].pos.z);
    let y0 = ctx.waddle[i].pos.y;
    ctx.waddle[i].vel.y -= GRAV * dt;
    let hl = ctx.waddle[i].vel.x.hypot(ctx.waddle[i].vel.z);
    if hl > 1e-4 {
        let mut p = ctx.waddle[i].pos;
        p.y += 0.3;
        let d = vec3(ctx.waddle[i].vel.x / hl, 0.0, ctx.waddle[i].vel.z / hl);
        let w = ctx.world.raycast(p, d, hl * dt + 0.15, false);
        if w.hit && w.normal.y.abs() < 0.5 {
            ctx.waddle[i].vel.x = 0.0;
            ctx.waddle[i].vel.z = 0.0;
        }
    }
    let vel = ctx.waddle[i].vel;
    ctx.waddle[i].pos += vel * dt;
    let gy = ctx
        .world
        .ground_height(ctx.waddle[i].pos.x, ctx.waddle[i].pos.z, y0 + 0.05, false);
    if gy.is_finite() && ctx.waddle[i].pos.y <= gy {
        ctx.waddle[i].pos.y = gy;
        ctx.waddle[i].air = false;
        ctx.waddle[i].vel = Vec3::ZERO;
    }
    if ctx.waddle[i].pos.y < ctx.t.water_y - 1.2 {
        // JS L525: sank while falling, no blast.
        ctx.waddle[i].why = "fell".to_string();
        ctx.waddle[i].state = WaddleState::Dead;
        return 0.0;
    }
    (ctx.waddle[i].pos.x - x0).hypot(ctx.waddle[i].pos.z - z0)
}

/// JS `blast(it)` (L573-603): the `[1, gid, pos]` record, the floor + four
/// satellite splats (RNG order `[seed, (ang, r, size, seed) × 4]`), the
/// turf credit, the Splat-Bomb-sized damage with LOS, `damageArea(60)`.
/// The `explosion` / audio / `shake` / `bomb:explode` / rumble / boss lines
/// are presentation or out of scope.
fn blast(ctx: &mut Ctx, i: usize, muted: bool) {
    if ctx.waddle[i].state == WaddleState::Dead {
        return;
    }
    let s = &ctx.sb.waddle;
    let (radius, dmax, dmin, paint_radius) = (s.radius, s.damage_max, s.damage_min, s.paint_radius);
    if !ctx.waddle[i].ghost {
        let (slot, team, gid) = (
            ctx.waddle[i].owner_slot,
            ctx.waddle[i].team,
            ctx.waddle[i].gid,
        );
        let p = ctx.waddle[i].pos;
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "waddle".to_string(),
            data: vec![1.0, gid as f32, r2(p.x), r2(p.y), r2(p.z)],
        });
    }
    ctx.waddle[i].blasted = true;
    let mut c = ctx.waddle[i].pos;
    c.y += 0.25;
    // ---- ink: the floor under it + four satellite splats (JS L580-584)
    let team = ctx.waddle[i].team;
    let mut p0 = ctx.waddle[i].pos;
    p0.y += 0.2;
    let mut area = 0.0;
    let seed = ctx.rng.next_f32();
    if !muted {
        area += ctx.paint.splat(
            ctx.world,
            p0,
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
    for _ in 0..4 {
        let ang = ctx.rng.next_f32() * TAU;
        let r = paint_radius * (0.55 + ctx.rng.next_f32() * 0.45);
        let rad = 0.5 + ctx.rng.next_f32() * 0.4;
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
    let (os, ot) = (ctx.waddle[i].owner_slot, ctx.waddle[i].owner_team);
    ctx.events.push(SimEvent::Turf {
        owner_slot: os,
        team: ot,
        area,
        no_special: false,
    });
    // ---- damage: line of sight from just above the bomb (JS L594-601)
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
        ctx.waddle[i].ghost,
        ctx.waddle[i].owner_idx,
        ctx.waddle[i].owner_slot,
    );
    for (ei, dmg) in hits {
        ctx.hit_routed(ghost, oi, os, team, ei, dmg);
    }
    ctx.damage_area(c, radius, 60.0, team, muted);
    ctx.waddle[i].state = WaddleState::Dead;
}

/// JS `hurt(it, dmg)` (L605-612): enemy fire / blasts pop it. `muted`
/// mirrors the JS `netMuted()` guard; a ghost's hit rides
/// [`SimEvent::DeviceHit`] to its owner (JS `netHurt`, L609).
pub(crate) fn hurt(
    waddle: &mut [WaddleItem],
    events: &mut Vec<SimEvent>,
    i: usize,
    dmg: f32,
    muted: bool,
) {
    let it = &waddle[i];
    if it.state == WaddleState::Dead || it.state == WaddleState::Fly || muted {
        return;
    }
    if !it.ghost {
        waddle[i].hp -= dmg;
    } else {
        events.push(SimEvent::DeviceHit {
            owner_slot: it.owner_slot,
            team: it.owner_team,
            gid: it.gid,
            kind: "waddle".to_string(),
            d: r2(dmg),
        });
        return;
    }
    if waddle[i].hp > 0.0 {
        return;
    }
    pop_plain(waddle, i);
}

/// JS `pop(it)` (L614-622) minus the burst / audio / `sub:destroyed` emit.
fn pop_plain(waddle: &mut [WaddleItem], i: usize) {
    waddle[i].why = "popped".to_string();
    waddle[i].state = WaddleState::Dead;
}

/// JS `pop(it)` from a ghost's `[2, gid, 1]` end record.
fn pop(ctx: &mut Ctx, i: usize) {
    pop_plain(ctx.waddle, i);
}

/// JS `SUB_KITS.waddle.tick` (L693-712): the reverse walk, the per-state
/// timers, the ghost age-out and the dead splice (`[2, gid, popped]` for a
/// local bomb that ended without blasting).
pub(crate) fn tick(ctx: &mut Ctx, dt: f32) {
    let mut i = ctx.waddle.len();
    while i > 0 {
        i -= 1;
        ctx.waddle[i].age += dt;
        ctx.waddle[i].t += dt;
        match ctx.waddle[i].state {
            WaddleState::Fly => fly(ctx, i, dt),
            WaddleState::Sense => sense(ctx, i),
            WaddleState::Wake => wake(ctx, i, dt),
            WaddleState::Walk => walk(ctx, i, dt),
            WaddleState::Dead => {}
        }
        if ctx.waddle[i].ghost && ctx.waddle[i].age > 40.0 {
            // its owner left mid-chase (JS L705)
            ctx.waddle[i].state = WaddleState::Dead;
        }
        if ctx.waddle[i].state == WaddleState::Dead {
            if !ctx.waddle[i].ghost && !ctx.waddle[i].blasted {
                let (slot, team, gid) = (
                    ctx.waddle[i].owner_slot,
                    ctx.waddle[i].team,
                    ctx.waddle[i].gid,
                );
                let popped = if ctx.waddle[i].why == "popped" {
                    1.0
                } else {
                    0.0
                };
                ctx.events.push(SimEvent::SubEnd {
                    owner_slot: slot,
                    team,
                    sub_kind: Some("waddle".to_string()),
                    data: vec![2.0, gid as f32, popped],
                });
            }
            ctx.waddle.remove(i);
        }
    }
}

/// JS `blockShot` (L720-728): enemy shots pop it (the shot is absorbed).
/// The capsule is `it.pos` + radius 0.17, height 0.4 (JS `mesh` bounds).
pub(crate) fn block_shot(
    waddle: &mut [WaddleItem],
    events: &mut Vec<SimEvent>,
    prev: Vec3,
    pos: Vec3,
    team: usize,
    dmg: f32,
    muted: bool,
) -> bool {
    for (i, it) in waddle.iter().enumerate() {
        if it.team == team || it.state == WaddleState::Fly || it.state == WaddleState::Dead {
            continue;
        }
        let res = segment_capsule_dist(prev, pos, it.pos, 0.17, 0.4);
        if res.dist < 0.2 {
            hurt(waddle, events, i, dmg, muted);
            return true;
        }
    }
    false
}

/// JS `blockRay` (L729-741): a charger beam pops the nearest enemy waddle
/// on its path (sphere radius 0.24 at `pos + 0.2 y`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn block_ray(
    waddle: &mut [WaddleItem],
    events: &mut Vec<SimEvent>,
    from: Vec3,
    dir: Vec3,
    len: f32,
    team: usize,
    dmg: f32,
    muted: bool,
) -> f32 {
    let mut best = len;
    let mut hit: Option<usize> = None;
    for (i, it) in waddle.iter().enumerate() {
        if it.team == team || it.state == WaddleState::Fly || it.state == WaddleState::Dead {
            continue;
        }
        let mut c = it.pos;
        c.y += 0.2;
        let t = (c - from).dot(dir);
        if t <= 0.0 || t >= best {
            continue;
        }
        let p = from + dir * t;
        if p.distance(c) < 0.24 {
            best = t;
            hit = Some(i);
        }
    }
    if let Some(i) = hit {
        hurt(waddle, events, i, dmg, muted);
    }
    best
}

/// JS `damageArea` (L742-748): an enemy blast pops nearby waddles.
pub(crate) fn damage_area(
    waddle: &mut [WaddleItem],
    events: &mut Vec<SimEvent>,
    c: Vec3,
    radius: f32,
    dmg: f32,
    team: usize,
    muted: bool,
) {
    let hits: Vec<usize> = waddle
        .iter()
        .enumerate()
        .filter(|(_, it)| {
            it.team != team
                && it.state != WaddleState::Fly
                && it.state != WaddleState::Dead
                && it.pos.distance(c) < radius + 0.3
        })
        .map(|(i, _)| i)
        .collect();
    for i in hits {
        hurt(waddle, events, i, dmg, muted);
    }
}

/// JS `netHurt(gid, dmg)` (L691): the owner's side of a hit made on its
/// waddle on another screen.
pub(crate) fn net_hurt(waddle: &mut [WaddleItem], events: &mut Vec<SimEvent>, gid: u32, dmg: f32) {
    let idx = waddle
        .iter()
        .position(|x| !x.ghost && x.gid == gid && x.state != WaddleState::Dead);
    if let Some(i) = idx {
        hurt(waddle, events, i, dmg, false);
    }
}
