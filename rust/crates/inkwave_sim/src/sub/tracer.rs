//! Tracer Bolt — the `tracer` kit sub (Task 34 batch two).
//!
//! Faithful port of `src/game/kits/tracer.js` (553 lines): a fast,
//! gravity-less bolt fired a few degrees *below* the aim line. It
//! ricochets off every surface except grates (it flies straight through
//! those), leaving a puddle at each bounce; off a floor the exit angle is
//! flattened so it skims low. It flies until its total travel reaches
//! `range` or it hits a foe; the lingering trail damages and marks a foe
//! who touches it (once per bolt, a direct hit taking the place of the
//! trail hit).
//!
//! Keep in lockstep with upstream on sync:
//!   - `use` (L193-200)          -> [`use_tracer`]
//!   - `launch` (L168-180)       -> the launch block of [`use_tracer`]
//!   - `spawn` (L204-218)        -> [`spawn`]
//!   - `ghost` (L219-234)        -> [`ghost`]
//!   - `addPoint` (L236-241)     -> [`add_point`]
//!   - `tick` (L243-267)         -> [`tick`]
//!   - `advance` (L270-306)      -> [`advance`]
//!   - `bounce` (L307-326)       -> [`bounce`]
//!   - `rebound` (L182-191)      -> [`rebound`]
//!   - `directHit` (L327-339)    -> [`direct_hit`]
//!   - `end` (L340-346)          -> [`end`]
//!   - `trailTouch` (L348-374)   -> [`trail_touch`]
//!   - `credit` (L164)           -> the turf credit in [`bounce`]
//!
//! Presentation-only branches are dropped per the M3 range rule (spec §0):
//! the bolt-head mesh, the trail ribbon (`RIB_CAP` still bounds the point
//! list, which the sim's `trailTouch` reads), `path`, `poseHead`,
//! `drawRibbon`, `dispose`, audio, `G.fx`, `nearCam`, the aim guide
//! (`hold` / `guideTick`, L397-454) and the `bot` / `noArc` registry
//! fields. `emit('sub:use', …)` rides [`SimEvent::SubUse`]; `emit('hit')`
//! rides [`SimEvent::Hit`] through [`Ctx::hit_routed`]; `tracer:hit` /
//! `weapon:impact` / `sub:destroyed` are not in the netmatch `FORWARD`
//! list and never reach the sim.
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-3: `it.sp` (`a.specialActive`, L208) is always false in the
//!     sim (specials are Task 35), so the turf credit is always the plain
//!     `addTurf`.
//!   - `a.aimPoint` (L174) is the local client's crosshair world position
//!     (player.js L254-286) — input-side data the sim does not carry. The
//!     launcher therefore uses `aim_dir()` directly, exactly as
//!     [`crate::weapon::ProjectileSim::aim_from`] falls back when the point
//!     is missing (match_.rs L717 passes `None`); the
//!     `d > 2 / dot >= 0` guard and renormalisation are no-ops on a unit
//!     `aimDir`, so the resulting pitch/yaw rebuild (L177-179) is the
//!     identity.
//!   - `e.smoothY` on the foe capsule base (L32) is ignored — the batch-one
//!     precedent (capsules use `e.pos`).
//!   - The bounce puddle skips `roof` / `perch` blocks (L312): the sim's
//!     [`Hit`] carries no block metadata (batch-one precedent).
//!   - `G.specials?.shotHit` (L294) is Task 35 and not probed.
//!   - The end record `[1, gid, pos×3, foe]` carries the *actor-list index*
//!     where JS carries the foe `nid` (L342); the replication layer
//!     translates idx ↔ nid on the wire (same convention as the waddle /
//!     torpedo lock records).
//!   - `ghostMute` (registry.js L73-77) raises the paint mute and, through
//!     it, `_hurt`'s `netMuted()` guard (subs.js L598): the port threads
//!     `muted = it.ghost` into the splat and the device probe instead. The
//!     trail's `track` has no mute guard in JS (L331/L366) and is applied
//!     by ghosts too.

use glam::{Vec3, vec3};

use crate::actor::Form;
use crate::collision::Hit;
use crate::paint::SplatOpts;
use crate::weapon::segment_capsule_dist;

use super::{Ctx, clamp};

/// JS `DEG` (L26).
const DEG: f32 = std::f32::consts::PI / 180.0;
/// JS `RIB_CAP` (L105): the trail point list bound.
const RIB_CAP: usize = 160;

/// JS `b.state` (the `end` reasons, tracer.js L343): `fly` is live; the
/// rest are terminal (the trail fades, then the bolt is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TracerState {
    /// `fly` — moving along its path.
    Fly,
    /// `hit` — ended on a foe (direct or via the end record).
    Hit,
    /// `blocked` — stopped by a device.
    Blocked,
    /// `range` — travel exhausted.
    Range,
    /// `water` — fell below the water line.
    Water,
    /// `bounces` — bounce budget exhausted.
    Bounces,
}

impl TracerState {
    /// JS `b.state === 'fly'`.
    #[must_use]
    pub fn live(self) -> bool {
        self == Self::Fly
    }
}

/// JS `{ p, b }` trail point (L240): world position + the `clock` stamp.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct TrailPoint {
    /// JS `p`.
    pub p: Vec3,
    /// JS `b` — the sim's `ctx.time` at append.
    pub b: f32,
}

/// One live bolt (JS `b`, the duck-typed objects of `bolts`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TracerItem {
    /// JS `b.owner` — resolved to the actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`) for records / turf.
    pub owner_slot: usize,
    /// The owner's team (JS `a.team`; `b.team` mirrors it).
    pub owner_team: usize,
    /// JS `b.team`.
    pub team: usize,
    /// JS `b.pos`.
    pub pos: Vec3,
    /// JS `b.dir` — unit flight direction.
    pub dir: Vec3,
    /// JS `b.left` — remaining travel budget.
    pub left: f32,
    /// JS `b.travel` — distance covered (presentation only, kept for parity).
    pub travel: f32,
    /// JS `b.bounces`.
    pub bounces: u32,
    /// JS `b.state`.
    pub state: TracerState,
    /// JS `b.hitSet` — foes this bolt has already damaged (the trail's
    /// once-per-bolt rule; a direct hit also lands here).
    pub hit_set: Vec<usize>,
    /// JS `b.pts` — the fading trail.
    pub pts: Vec<TrailPoint>,
    /// JS `b.age`.
    pub age: f32,
    /// JS `b.direct` — the foe of the direct hit (end record payload).
    pub direct: Option<usize>,
    /// JS `b.ghost`.
    pub ghost: bool,
    /// JS `b.gid`.
    pub gid: u32,
    /// The wire `nid` that minted this ghost (D-T34-5 ghost keying).
    pub src_nid: Option<u8>,
}

impl TracerItem {
    /// JS `b.state === 'fly'`.
    #[must_use]
    pub fn live(&self) -> bool {
        self.state.live()
    }
}

/// JS `r3` (L17): the three-decimal quantisation the tracer records use
/// (the batch-one subs use [`r2`]; tracer.js keeps its own helper).
fn r3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

/// JS `use(subs, a, sub)` (L193-200) with `launch` (L168-180) inlined:
/// the chest-height release, the aim line lowered, the `[0, …]` record.
pub(crate) fn use_tracer(ctx: &mut Ctx, actor_idx: usize) {
    let a = &ctx.actors[actor_idx];
    let (slot, team) = (a.slot, a.team);
    let pitch_down = ctx.sb.tracer.pitch_down;
    // JS L169-170: chest point (the `smoothY` bob is presentation-side input,
    // ignored per the batch-one capsule precedent).
    let chest = vec3(a.pos.x, a.pos.y + 1.0, a.pos.z);
    let fx = a.aim_yaw.sin();
    let fz = a.aim_yaw.cos();
    let mut from = vec3(a.pos.x + fx * 0.3, a.pos.y + 1.2, a.pos.z + fz * 0.3);
    if !ctx.world.los(chest, from) {
        from = chest; // JS L173: never launch inside a wall
    }
    // JS L174-176: the crosshair line, lowered. The sim has no `aimPoint`
    // (deviation note in the module doc) — `aim_dir()` is the fallback the
    // JS guard takes whenever the point is nearer than 2 m or behind.
    let adir = a.aim_dir();
    let pitch = clamp(
        adir.y.clamp(-1.0, 1.0).asin() - pitch_down * DEG,
        -1.5,
        1.45,
    );
    let yaw = adir.x.atan2(adir.z);
    let cp = pitch.cos();
    let dir = vec3(yaw.sin() * cp, pitch.sin(), yaw.cos() * cp);

    let gid = ctx.gid_next(slot);
    spawn(ctx, actor_idx, slot, team, from, dir, false, gid, None);
    // JS L197: `netRec(a, 'tracer', [0, gid, from×3, dir×3])` (r3-quantised).
    ctx.events.push(crate::weapon::SimEvent::SubSpawn {
        owner_slot: slot,
        team,
        sub_kind: "tracer".to_string(),
        data: vec![
            0.0,
            gid as f32,
            r3(from.x),
            r3(from.y),
            r3(from.z),
            r3(dir.x),
            r3(dir.y),
            r3(dir.z),
        ],
    });
    ctx.events.push(crate::weapon::SimEvent::SubUse {
        owner_slot: slot,
        team,
        kind: super::SubKind::Tracer,
    });
}

/// JS `spawn(subs, a, sub, from, dir, ghost, gid)` (L204-218) minus the
/// head / ribbon / audio. The `addPoint` inside JS `spawn` seeds the trail
/// at the launch point.
#[allow(clippy::too_many_arguments)]
fn spawn(
    ctx: &mut Ctx,
    owner_idx: usize,
    owner_slot: usize,
    owner_team: usize,
    from: Vec3,
    dir: Vec3,
    ghost: bool,
    gid: u32,
    src_nid: Option<u8>,
) {
    let range = ctx.sb.tracer.range;
    ctx.tracer.push(TracerItem {
        owner_idx,
        owner_slot,
        owner_team,
        team: owner_team,
        pos: from,
        dir,
        left: range,
        travel: 0.0,
        bounces: 0,
        state: TracerState::Fly,
        hit_set: Vec::new(),
        pts: vec![TrailPoint {
            p: from,
            b: ctx.time,
        }],
        age: 0.0,
        direct: None,
        ghost,
        gid,
        src_nid,
    });
}

/// JS `ghost(a, d)` (L219-234). The netmatch replay wraps `'k'` records in
/// `mute++`, so the roster nid table resolves the end record's foe back to
/// an actor-list index.
pub(crate) fn ghost(ctx: &mut Ctx, actor_idx: usize, nid: u8, data: &[f32], nids: &[u8]) {
    if data.len() < 2 {
        return;
    }
    let op = data[0];
    let gid = data[1] as u32;
    if op == 0.0 {
        if data.len() < 8 {
            return;
        }
        // JS L223: `bolts.some(x => x.gid === gid)` — the sim keys ghosts on
        // (src_nid, gid) (D-T34-5).
        if ctx
            .tracer
            .iter()
            .any(|x| x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        let from = vec3(data[2], data[3], data[4]);
        let dir = vec3(data[5], data[6], data[7]).normalize();
        spawn(ctx, actor_idx, slot, team, from, dir, true, gid, Some(nid));
        return;
    }
    if op != 1.0 || data.len() < 6 {
        return; // JS L230: unknown ops are ignored
    }
    let Some(i) = ctx
        .tracer
        .iter()
        .position(|x| x.ghost && x.src_nid == Some(nid) && x.gid == gid && x.state.live())
    else {
        return;
    };
    ctx.tracer[i].pos = vec3(data[2], data[3], data[4]);
    add_point(ctx, i);
    // JS L232: `d[5] >= 0 ? byNid(d[5]) : null` (-1 nobody, -2 a device).
    let foe = if data[5] >= 0.0 {
        nid_to_idx(nids, data[5] as i32)
    } else {
        None
    };
    if let Some(ei) = foe {
        direct_hit(ctx, i, ei);
    } else {
        let why = if data[5] == -2.0 {
            TracerState::Blocked
        } else {
            TracerState::Hit
        };
        end(ctx, i, why);
    }
}

/// JS `G.actors.find(x => x.nid === n)` for a ghost record.
fn nid_to_idx(nids: &[u8], nid: i32) -> Option<usize> {
    if nid < 0 {
        return None;
    }
    nids.iter().position(|&x| x as i32 == nid)
}

/// JS `addPoint(b)` (L236-241).
fn add_point(ctx: &mut Ctx, i: usize) {
    let pos = ctx.tracer[i].pos;
    let clock = ctx.time;
    let pts = &mut ctx.tracer[i].pts;
    if let Some(last) = pts.last_mut()
        && last.p.distance_squared(pos) < 1e-6
    {
        last.b = clock; // JS L238: refresh the stamp, no new point
        return;
    }
    if pts.len() >= RIB_CAP {
        pts.remove(0);
    }
    pts.push(TrailPoint { p: pos, b: clock });
}

/// JS `tick(dt)` (L243-267) minus `clock += dt` (the sim carries
/// `ctx.time`), the guide, the ribbon / head pose and the hum.
pub(crate) fn tick(ctx: &mut Ctx, dt: f32) {
    let speed = ctx.sb.tracer.speed;
    let trail_life = ctx.sb.tracer.trail_life;
    let mut i = ctx.tracer.len();
    while i > 0 {
        i -= 1;
        ctx.tracer[i].age += dt;
        if ctx.tracer[i].state.live() {
            let muted = ctx.tracer[i].ghost; // JS `ghostMute` wrap (L251)
            advance(ctx, i, speed * dt, muted);
            add_point(ctx, i);
            if ctx.tracer[i].ghost && ctx.tracer[i].age > 4.0 {
                // JS L253: a ghost that never heard its end record.
                end(ctx, i, TracerState::Range);
            }
        }
        // JS L257: drop fully faded points (keep one for the next segment).
        {
            let pts = &mut ctx.tracer[i].pts;
            while pts.len() > 1 && ctx.time - pts[1].b > trail_life {
                pts.remove(0);
            }
        }
        trail_touch(ctx, i);
        let faded = {
            let pts = &ctx.tracer[i].pts;
            pts.is_empty() || ctx.time - pts[pts.len() - 1].b > trail_life
        };
        if !ctx.tracer[i].state.live() && faded {
            // JS L259: `dispose(b); bolts.splice(i, 1)` — no end record and
            // no `sub:destroyed` (not forwarded); the trail just goes.
            ctx.tracer.remove(i);
        }
    }
}

/// JS `advance(b, dist)` (L270-306): bounces, direct hits, devices, range,
/// water. `muted` mirrors the `ghostMute` wrap — the puddle is skipped and
/// the device probe does no damage, but a device still stops the bolt (its
/// owner's copy takes the hit through the owner's own unmuted pass).
fn advance(ctx: &mut Ctx, i: usize, dist: f32, muted: bool) {
    let mut remain = dist;
    let mut guard = 0;
    while remain > 1e-5 && ctx.tracer[i].state.live() && guard < 24 {
        guard += 1;
        let pos = ctx.tracer[i].pos;
        let dir = ctx.tracer[i].dir;
        let left = ctx.tracer[i].left;
        let step = remain.min(left);
        // JS L275: grates are flown straight through.
        let w = ctx.world.raycast(pos, dir, step, true);
        let seg = if w.hit { w.dist } else { step };
        let endp = pos + dir * seg;
        // JS L280-286: the earliest foe on this stretch (a ghost's foe hit
        // is its owner's call — the end record decides).
        if !ctx.tracer[i].ghost {
            let team = ctx.tracer[i].team;
            let size = ctx.sb.tracer.size;
            let radius = ctx.t.radius;
            let mut best: Option<usize> = None;
            let mut bt = 2.0f32;
            for (ei, e) in ctx.actors.iter().enumerate() {
                if e.team == team || !e.alive {
                    continue;
                }
                if (e.pos.x - pos.x).abs().max((e.pos.z - pos.z).abs()) > seg + 3.0 {
                    continue;
                }
                let h = if e.form == Form::Squid {
                    ctx.t.squid_height
                } else {
                    ctx.t.height
                };
                let res = segment_capsule_dist(pos, endp, e.pos, radius, h);
                if res.dist < radius + size && res.t < bt {
                    bt = res.t;
                    best = Some(ei);
                }
            }
            if let Some(ei) = best {
                ctx.tracer[i].pos = pos.lerp(endp, bt);
                add_point(ctx, i);
                direct_hit(ctx, i, ei);
                return;
            }
        }
        // JS L294: enemy curtains / shields / devices stop it (and take the
        // direct damage). `G.specials?.shotHit` is Task 35.
        let team = ctx.tracer[i].team;
        let dd = ctx.sb.tracer.direct_damage;
        if ctx.probe_block_shot(pos, endp, team, dd, muted) {
            ctx.tracer[i].pos = endp;
            add_point(ctx, i);
            end(ctx, i, TracerState::Blocked);
            return;
        }
        ctx.tracer[i].pos = endp;
        ctx.tracer[i].left -= seg;
        ctx.tracer[i].travel += seg;
        remain -= seg;
        if w.hit {
            bounce(ctx, i, &w, muted);
        }
        if !ctx.tracer[i].state.live() {
            return;
        }
        if ctx.tracer[i].left <= 1e-4 {
            add_point(ctx, i);
            end(ctx, i, TracerState::Range);
            return;
        }
        if ctx.tracer[i].pos.y < ctx.t.water_y - 0.3 {
            add_point(ctx, i);
            end(ctx, i, TracerState::Water);
            return;
        }
    }
}

/// JS `bounce(b, w)` (L307-326) minus the fx / audio / `weapon:impact`
/// emit: a puddle at the strike point, the rebound, the bounce budget.
fn bounce(ctx: &mut Ctx, i: usize, w: &Hit, muted: bool) {
    add_point(ctx, i);
    let dir = ctx.tracer[i].dir;
    // JS L313: the smear runs along the flat part of the incoming path.
    let mut stretch = vec3(dir.x, 0.0, dir.z);
    if stretch.length_squared() < 1e-4 {
        stretch = vec3(0.0, 0.0, 1.0);
    }
    let stretch = stretch.normalize();
    let seed = ctx.rng.next_f32();
    let team = ctx.tracer[i].team;
    let puddle = ctx.sb.tracer.puddle_radius;
    // JS L314: the roof / perch filter is dropped (no block metadata).
    let area = if muted {
        0.0
    } else {
        ctx.paint.splat(
            ctx.world,
            w.point + w.normal * 0.1,
            puddle,
            team,
            &SplatOpts {
                seed,
                stretch: Some(stretch),
                stretch_amt: Some(0.35),
                kind: None,
            },
        )
    };
    // JS `credit(b, area)` (L164): zero-area splats never credit.
    if area != 0.0 {
        ctx.events.push(crate::weapon::SimEvent::Turf {
            owner_slot: ctx.tracer[i].owner_slot,
            team: ctx.tracer[i].owner_team,
            area,
            no_special: false,
        });
    }
    let n = w.normal;
    let floor_exit = ctx.sb.tracer.floor_exit;
    ctx.tracer[i].dir = rebound(ctx.tracer[i].dir, n, floor_exit);
    ctx.tracer[i].pos = w.point + n * 0.03;
    ctx.tracer[i].bounces += 1;
    if ctx.tracer[i].bounces > ctx.sb.tracer.max_bounces {
        end(ctx, i, TracerState::Bounces);
    }
}

/// JS `rebound(dir, n, sub)` (L182-191): mirror off the surface; a floor
/// flattens the exit so the bolt skims.
fn rebound(mut dir: Vec3, n: Vec3, floor_exit: f32) -> Vec3 {
    dir -= n * (2.0 * dir.dot(n));
    if n.y > 0.6 {
        dir.y *= floor_exit;
        dir = dir.normalize();
        let dn = dir.dot(n);
        if dn < 0.04 {
            dir += n * (0.04 - dn); // sloped floor: never back into it
            dir = dir.normalize();
        }
    }
    dir.normalize()
}

/// JS `directHit(b, e)` (L327-339): the once-per-bolt mark, the routed
/// damage, the long mark, and the end.
fn direct_hit(ctx: &mut Ctx, i: usize, ei: usize) {
    ctx.tracer[i].hit_set.push(ei);
    ctx.tracer[i].direct = Some(ei);
    let (ghost, oi, os, team) = (
        ctx.tracer[i].ghost,
        ctx.tracer[i].owner_idx,
        ctx.tracer[i].owner_slot,
        ctx.tracer[i].team,
    );
    let dmg = ctx.sb.tracer.direct_damage;
    ctx.hit_routed(ghost, oi, os, team, ei, dmg);
    // JS L331: `track` has no mute guard — a ghost's direct record still
    // marks the local foe.
    let mark = ctx.sb.tracer.direct_mark;
    ctx.track(ei, team, mark);
    end(ctx, i, TracerState::Hit);
}

/// JS `end(b, why)` (L340-346): the owner's `[1, …]` record for a real
/// bolt that ended on a foe or a device, then the state flip.
fn end(ctx: &mut Ctx, i: usize, why: TracerState) {
    if !ctx.tracer[i].state.live() {
        return;
    }
    if !ctx.tracer[i].ghost && (why == TracerState::Hit || why == TracerState::Blocked) {
        let gid = ctx.tracer[i].gid;
        let pos = ctx.tracer[i].pos;
        let (owner_slot, team) = (ctx.tracer[i].owner_slot, ctx.tracer[i].team);
        // JS L342: `why === 'hit' ? b.direct?.nid ?? -1 : -2` — the sim
        // carries the actor-list index; replication maps it to the wire nid.
        let foe = if why == TracerState::Hit {
            ctx.tracer[i].direct.map_or(-1.0, |x| x as f32)
        } else {
            -2.0
        };
        ctx.events.push(crate::weapon::SimEvent::SubSpawn {
            owner_slot,
            team,
            sub_kind: "tracer".to_string(),
            data: vec![1.0, gid as f32, r3(pos.x), r3(pos.y), r3(pos.z), foe],
        });
    }
    ctx.tracer[i].state = why;
}

/// JS `trailTouch(b)` (L348-374): a foe touching the live trail is damaged
/// and marked (short), once per bolt.
fn trail_touch(ctx: &mut Ctx, i: usize) {
    let n = ctx.tracer[i].pts.len();
    if n < 2 {
        return;
    }
    let team = ctx.tracer[i].team;
    let life = ctx.sb.tracer.trail_life * 0.92;
    let reach = ctx.t.radius + ctx.sb.tracer.trail_radius;
    let radius = ctx.t.radius;
    let mut hits: Vec<usize> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive || ctx.tracer[i].hit_set.contains(&ei) {
            continue;
        }
        let h = if e.form == Form::Squid {
            ctx.t.squid_height
        } else {
            ctx.t.height
        };
        let base = e.pos;
        for k in 0..n - 1 {
            let a = ctx.tracer[i].pts[k];
            let c = ctx.tracer[i].pts[k + 1];
            if ctx.time - a.b > life {
                continue;
            }
            let minx = a.p.x.min(c.p.x) - reach;
            let maxx = a.p.x.max(c.p.x) + reach;
            let minz = a.p.z.min(c.p.z) - reach;
            let maxz = a.p.z.max(c.p.z) + reach;
            if base.x < minx || base.x > maxx || base.z < minz || base.z > maxz {
                continue;
            }
            let res = segment_capsule_dist(a.p, c.p, base, radius, h);
            if res.dist >= reach {
                continue;
            }
            hits.push(ei);
            break; // JS L371: next foe, this bolt already caught this one
        }
    }
    let (ghost, oi, os) = (
        ctx.tracer[i].ghost,
        ctx.tracer[i].owner_idx,
        ctx.tracer[i].owner_slot,
    );
    let dmg = ctx.sb.tracer.trail_damage;
    let mark = ctx.sb.tracer.trail_mark;
    for ei in hits {
        ctx.tracer[i].hit_set.push(ei);
        ctx.hit_routed(ghost, oi, os, team, ei, dmg);
        ctx.track(ei, team, mark);
    }
}
