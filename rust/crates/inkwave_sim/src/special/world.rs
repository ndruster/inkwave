//! The specials world-object registry (JS `SpecialSystem.world`, specials.js
//! L566; `_ghostObj` L306-322, `_spawn` L300-305) and the batch-2 object
//! mechanics. The storm cloud (JS `Projectiles.clouds`, weapons.js L1539,
//! stepped by `_updateClouds` L1760-1795) is the first payload kind; batches
//! 3/4 add the missile / bubble / twister / speaker / orb / return-marker /
//! thrown-stamp / shell bodies to [`ObjectBody`] and their step arms to
//! [`step_special_objects`].

use glam::{Vec3, vec3};

use crate::actor::{Actor, Form};
use crate::collision::CollisionWorld;
use crate::match_::find_actor;
use crate::net::KitVal;
use crate::paint::{PaintGrid, SplatOpts};
use crate::sub::SubSystem;
use crate::tuning::{PlayerTuning, SubSet};
use crate::weapon::{Rng, SimEvent, apply_hit, segment_capsule_dist};

const TAU: f32 = std::f32::consts::TAU;

/// Per-kind object payload (JS stores the kind-specific fields loose on the
/// object literal). `None` = a batch-1 registry entry / ghost outline block
/// with no sim-side mechanics.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum ObjectBody {
    #[default]
    None,
    /// JS `Projectiles.clouds` entry (weapons.js L1539): a storm cloud.
    Cloud(Cloud),
    /// JS `Missile` (specials.js L776-820): the strike's ballistic arc.
    Missile(Missile),
    /// JS `Tornado` (specials.js L822-874): the strike's landing vortex.
    Tornado(Tornado),
    /// JS `Twister` (specials.js L877-931): a zooka volley.
    Twister(Twister),
    /// JS `Speaker` (specials.js L934-1019): the howl box.
    Speaker(Speaker),
    /// JS `Bubble` (specials.js L1022-1101): a blower bubble.
    Bubble(Bubble),
    /// JS `Orb` (specials.js L1193-1242): the thrown cheer orb.
    Orb(Orb),
    /// JS `ReturnMarker` (specials.js L688-773): the jetpack / zipline home
    /// pad marker (never rides the wire — replayed from the ghost records).
    Marker(Marker),
    /// JS `ThrownStamp` (specials.js L1104-1153): the Mega Stamp throw.
    TStamp(TStamp),
    /// JS `Shell` (specials.js L1156-1190): the Crab Rig mortar shell.
    Shell(Shell),
}

/// Everything the batch-3 object mechanics need beyond the registry itself
/// (the JS globals `G.*` at the specials.js call sites). `events` is the
/// projectile event queue split out of [`crate::weapon::ProjectileSim`] so
/// the weapon-step bubble hooks can borrow `ps.rng` alongside it.
pub struct WorldCtx<'a> {
    pub actors: &'a mut [Actor],
    pub collision: &'a CollisionWorld,
    pub paint: &'a mut PaintGrid,
    pub rng: &'a mut Rng,
    pub subs: &'a mut SubSystem,
    pub sb: &'a SubSet,
    pub t: &'a PlayerTuning,
    pub events: &'a mut Vec<SimEvent>,
    pub remote: Option<&'a [bool]>,
    pub time: f32,
}

/// JS `r2` (specials.js L39): the record quantiser.
#[must_use]
pub fn r2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// JS `v3` (specials.js L40): a record triple.
#[must_use]
pub fn v3(v: Vec3) -> [f32; 3] {
    [r2(v.x), r2(v.y), r2(v.z)]
}

/// JS `groundBelow(p, maxDown)` (specials.js L48-51): a down raycast from
/// `p.y + 0.3` (the `0.3` / default `30` are hardcoded in JS, D-T35-4).
#[must_use]
pub fn ground_below(collision: &CollisionWorld, p: Vec3, max_down: f32) -> Option<Vec3> {
    const PROBE_UP: f32 = 0.3; // JS L49
    let g = collision.raycast(
        vec3(p.x, p.y + PROBE_UP, p.z),
        Vec3::NEG_Y,
        max_down,
        true, // JS `_hit2, true`: skip grates
    );
    g.hit.then_some(g.point)
}

/// JS `paint(owner, pos, r, team)` (specials.js L53): one seed draw is
/// consumed *always* (JS evaluates `Math.random()` for the splat argument
/// even under the ghost mute, where the splat itself returns 0) and the
/// claimed area credits turf without charging the gauge.
pub fn paint_splat(
    ctx: &mut WorldCtx,
    owner: Option<usize>,
    pos: Vec3,
    r: f32,
    team: usize,
    muted: bool,
) {
    let seed = ctx.rng.next_f32();
    if muted {
        return; // JS `netMuted()` inside `PaintSystem.splat` (paint.js L470)
    }
    let area = ctx.paint.splat(
        ctx.collision,
        pos,
        r,
        team,
        &SplatOpts {
            seed,
            stretch: None,
            stretch_amt: None,
            kind: None,
        },
    );
    if let Some(oi) = owner {
        ctx.actors[oi].add_turf_no_special(area);
    }
}

/// JS `tickDamage(owner, e, dmg, weaponId)` (specials.js L81-85): continuous
/// damage is *each client's own* — remote victims are skipped (their owner
/// decides), the call is a direct `e.damage` (no `applyHit` routing, no
/// per-frame hit events).
fn tick_damage(ctx: &mut WorldCtx, owner: usize, ei: usize, dmg: f32) {
    let (team, slot) = (ctx.actors[owner].team, ctx.actors[owner].slot);
    let owner_pos = ctx.actors[owner].pos;
    let e = &mut ctx.actors[ei];
    if !e.alive || e.team == team {
        return; // JS L82
    }
    if ctx
        .remote
        .is_some_and(|m| m.get(ei).copied().unwrap_or(false))
    {
        return; // JS L82: `e.remote`
    }
    // JS L83-84: `e.damage(...)`; the `killed → emit('hit')` is FX-only (the
    // splat rides the actor event bus, same as the cloud dps ladder).
    e.damage(dmg, Some((team, slot)), Some(owner_pos), ctx.t);
}

/// JS `Missile` (specials.js L776-820). No RNG draws.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Missile {
    pub owner_slot: usize,
    /// JS `m.t`.
    pub t: f32,
    /// JS `m.flight` (`SPECIALS.strike.flight`).
    pub flight: f32,
    /// JS `m.from` (`a.pos + 1.6y`, L779 — the `1.6` is hardcoded, D-T35-4).
    pub from: Vec3,
    /// JS `m.to`: the ground point under the aim target.
    pub to: Vec3,
}

/// JS `Tornado` (specials.js L822-874).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Tornado {
    pub owner_slot: usize,
    pub t: f32,
    pub dur: f32,
    pub radius: f32,
    /// JS `tn.paintT`: the 0.1 s churn window countdown.
    pub paint_t: f32,
}

/// JS `Twister` (specials.js L877-931).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Twister {
    pub owner_slot: usize,
    /// JS `tw.vel` (`dir * d.speed`).
    pub vel: Vec3,
    pub t: f32,
    /// JS `tw.life` (`range / speed`).
    pub life: f32,
    /// JS `tw.hit`: actors already pierced this volley (actor indices).
    pub hit: Vec<usize>,
    /// JS `tw.paintAcc`: metres travelled since the last floor drip.
    pub paint_acc: f32,
}

/// JS `Speaker` (specials.js L934-1019). `phase`: 0 charge, 1 blast, 2 fade.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Speaker {
    pub owner_slot: usize,
    pub t: f32,
    pub phase: u8,
    /// JS `sp.dir`: the clamped-pitch beam direction (unit).
    pub dir: Vec3,
    /// JS `sp.mouth`: the beam origin (the box's cone tip).
    pub mouth: Vec3,
    pub range: f32,
    pub radius: f32,
}

/// JS `Bubble` (specials.js L1022-1101).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Bubble {
    pub owner_slot: usize,
    pub t: f32,
    /// JS `b.life`: countdown while released (`SPECIALS.blower.life`).
    pub life: f32,
    pub r: f32,
    pub vel: Vec3,
    /// JS `b.charge`: own-team ink soaked (pops at `popDamage`).
    pub charge: f32,
    /// JS `b.held`: still on the blower's muzzle.
    pub held: bool,
    /// JS `b.ph` = `Math.random() * 6` (ctor, L1027): the bob phase.
    pub ph: f32,
}

/// JS `Orb` (specials.js L1193-1242). `phase`: 0 fly, 1 fuse.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Orb {
    pub owner_slot: usize,
    pub vel: Vec3,
    pub t: f32,
    pub phase: u8,
    pub prev: Vec3,
    /// JS `d.fuse` (`SPECIALS.booyah.fuse`).
    pub fuse: f32,
}

/// JS `ReturnMarker` (specials.js L688-773) minus the presentation (rings,
/// pillar, badge canvas). `back` / `done` / `fade` are the JS L691 flags; the
/// `superjump:land` / `splatted` listeners (L709-710) are driven through
/// [`crate::special::drain_marker_action`] and the super-jump landing drain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Marker {
    /// JS `m.owner` identity slot.
    pub owner_slot: usize,
    /// JS `m.icon` (`'jetpack'` / `'zipcaster'`).
    pub icon: String,
    /// JS `m.t`.
    pub t: f32,
    /// JS `m.done`.
    pub done: bool,
    /// JS `m.back` (set by `returning()`, L714).
    pub back: bool,
    /// JS `m.fade` (presentation-only opacity, drives removal at `<= 0`).
    pub fade: f32,
}

/// JS `ThrownStamp` (specials.js L1104-1153). No RNG draws in the ctor.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct TStamp {
    pub owner_slot: usize,
    /// JS `s.vel` (L1108: `dir * throwSpeed`, then `vel.y += 2.5`).
    pub vel: Vec3,
    /// JS `s.prev` (the swept-segment start, L1118).
    pub prev: Vec3,
    pub t: f32,
}

/// JS `Shell` (specials.js L1156-1190). No RNG draws in the ctor.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Shell {
    pub owner_slot: usize,
    pub vel: Vec3,
    pub prev: Vec3,
    pub t: f32,
}

/// JS storm cloud `{ owner, team, t, dur, dir, rainT, groundY, ghost }`
/// (weapons.js L1539 + the `ghost` flag stamped at L1729/L1740).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Cloud {
    /// JS `c.owner` identity slot (the team rides [`WorldObject::team`]).
    pub owner_slot: usize,
    /// JS `c.t`: seconds since spawn.
    pub t: f32,
    /// JS `c.dur` (`SPECIALS.storm.duration`).
    pub dur: f32,
    /// JS `c.dir` (`b.dir` — the throw heading, unit XZ): the drift vector.
    pub dir: Vec3,
    /// JS `c.rainT`: countdown to the next rain drop (fires every 0.045 s).
    pub rain_t: f32,
    /// JS `c.groundY`: the raycast floor height the cloud spawned over
    /// (presentation anchor; the rain raycasts run from the cloud itself).
    pub ground_y: f32,
}

/// One live special world object (JS `Missile` / `Twister` / `Speaker` /
/// `Bubble` / `ThrownStamp` / `Shell` / `Orb` / `StormCloud` / `ReturnMarker`
/// / `Zipline`). `kind` is the wire tag (`'mi'`, `'tw'`, …, specials.js
/// L311-317), the sim-side owner kind (`'cloud'`, …) or the ghost tag.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldObject {
    /// JS `w.kind` — the visual/wire tag.
    pub kind: String,
    /// JS `w.gid` (registry.js `netId`, L65): `0` = not yet on the wire
    /// (`_spawn` L303 only records when `gid` is truthy).
    pub gid: u32,
    /// JS `w.team`.
    pub team: usize,
    /// JS `w.pos` (the ghost spawn point from the record payload; the cloud's
    /// live position — JS advances `c.group.position` in place, L1768).
    pub pos: Vec3,
    /// JS `w.ghost` (L320): a remote proxy's copy — visual only.
    pub ghost: bool,
    /// JS `w.dead`: popped / expired objects stay listed until pruned.
    pub dead: bool,
    /// The kind-specific payload (JS loose fields).
    #[serde(default)]
    pub body: ObjectBody,
}

impl WorldObject {
    /// JS `_ghostObj` (L306-322): a ghost visual object built from the
    /// `['k', nid, 'sp', 2, kind, gid, x, y, z, …]` record payload.
    #[must_use]
    pub fn ghost(kind: &str, gid: u32, team: usize, pos: Vec3) -> Self {
        Self {
            kind: kind.to_string(),
            gid,
            team,
            pos,
            ghost: true,
            dead: false,
            body: ObjectBody::None,
        }
    }

    /// An owner-side object (`_spawn`, L300-305) — `gid` stamped by
    /// [`crate::special::spawn`].
    #[must_use]
    pub fn new(kind: &str, team: usize, pos: Vec3) -> Self {
        Self {
            kind: kind.to_string(),
            gid: 0,
            team,
            pos,
            ghost: false,
            dead: false,
            body: ObjectBody::None,
        }
    }
}

/// A storm cloud waiting to enter the world (JS `_spawnCloud` L1509-1541,
/// called inline from `_updateBombs` L1729/L1740): the spent storm bomb's
/// raw integrated position + heading. The match converts these right after
/// the projectile step, so the fresh cloud steps in the same frame (JS
/// `_updateBombs` runs before `_updateClouds` inside one `update`).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CloudSpawn {
    pub owner_slot: usize,
    pub team: usize,
    /// JS `b.pos` — the bomb's position when it landed / timed out.
    pub pos: Vec3,
    /// JS `b.dir` — the throw heading.
    pub dir: Vec3,
    /// JS `b.ghost` (stamped onto the cloud at L1729/L1740).
    pub ghost: bool,
}

/// JS `_spawnCloud` geometry (weapons.js L1511-L1535): raycast down 12 m from
/// `b.pos + 0.5y` for the floor, then hang the cloud at `groundY + 4.6`. The
/// `0.5` / `12` / `4.6` triple is hardcoded in JS (D-T35-4); the 62
/// presentation `Math.random()` draws of the puff meshes (L1522-L1532) are
/// dropped like the bomb `spin` draws (weapon module docs).
#[must_use]
pub fn make_cloud(collision: &CollisionWorld, spawn: &CloudSpawn, dur: f32) -> WorldObject {
    const CLOUD_PROBE_UP: f32 = 0.5; // JS L1511
    const CLOUD_PROBE_DOWN: f32 = 12.0; // JS L1511
    const CLOUD_HANG: f32 = 4.6; // JS L1535
    let g = collision.raycast(
        vec3(spawn.pos.x, spawn.pos.y + CLOUD_PROBE_UP, spawn.pos.z),
        Vec3::NEG_Y,
        CLOUD_PROBE_DOWN,
        false,
    );
    let ground_y = if g.hit { g.point.y } else { spawn.pos.y };
    WorldObject {
        kind: "cloud".to_string(),
        gid: 0,
        team: spawn.team,
        pos: vec3(spawn.pos.x, ground_y + CLOUD_HANG, spawn.pos.z),
        ghost: spawn.ghost,
        dead: false,
        body: ObjectBody::Cloud(Cloud {
            owner_slot: spawn.owner_slot,
            t: 0.0,
            dur,
            dir: spawn.dir,
            rain_t: 0.0,
            ground_y,
        }),
    }
}

/// JS `_updateClouds(dt)` (weapons.js L1760-1795) for every cloud in the
/// registry, reverse order (JS splices in place). Drift runs unconditionally;
/// the rain + dps ladder only while `t < dur - 0.3`; the cloud dies at
/// `t >= dur`. The `0.045` rain period, `0.45`/`0.35` splat pair, `-0.8`
/// rain start, `12` ray length, `0.1` normal push, `-0.6` LOS origin and
/// `+1.2` LOS target are hardcoded in JS (D-T35-4).
#[allow(clippy::too_many_arguments)]
pub fn step_clouds(
    objects: &mut Vec<WorldObject>,
    dt: f32,
    actors: &mut [Actor],
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    const RAIN_EVERY: f32 = 0.045; // JS L1775
    const RAIN_START_DROP: f32 = 0.8; // JS L1777 (`y - 0.8`)
    const RAY_DOWN: f32 = 12.0; // JS L1778
    const NORMAL_PUSH: f32 = 0.1; // JS L1779
    const LOS_TARGET_UP: f32 = 1.2; // JS L1786 (`e.pos.y + 1.2`)
    const LOS_FROM_DOWN: f32 = 0.6; // JS L1787 (`c.y - 0.6`)
    const CLOUD_TAIL: f32 = 0.3; // JS L1771/L1793 (`dur - 0.3` quiet window)
    let sp = &crate::special::specials().storm;
    let mut i = objects.len();
    while i > 0 {
        i -= 1;
        // `Cloud` is `Copy`: work on a local snapshot and write it back, so
        // the object's position reads below never alias the payload borrow.
        let mut cloud = match &objects[i].body {
            ObjectBody::Cloud(c) => *c,
            _ => continue,
        };
        let ghost = objects[i].ghost;
        let team = objects[i].team;
        cloud.t += dt;
        objects[i].pos += cloud.dir * (sp.drift_speed * dt); // JS L1768
        if cloud.t < cloud.dur - CLOUD_TAIL {
            // JS L1773-1780: the rain ladder. Every tick consumes the two
            // scatter draws even for a ghost (the splat + its two draws are
            // the `!c.ghost` half of the L1779 guard).
            cloud.rain_t -= dt;
            while cloud.rain_t <= 0.0 {
                cloud.rain_t += RAIN_EVERY;
                let a = rng.next_f32() * std::f32::consts::TAU;
                let r = rng.next_f32().sqrt() * sp.radius;
                let v = vec3(
                    objects[i].pos.x + a.cos() * r,
                    objects[i].pos.y - RAIN_START_DROP,
                    objects[i].pos.z + a.sin() * r,
                );
                let g = collision.raycast(v, Vec3::NEG_Y, RAY_DOWN, false);
                if g.hit && !ghost {
                    let rad = 0.45 + rng.next_f32() * 0.35;
                    let seed = rng.next_f32();
                    let area = paint.splat(
                        collision,
                        g.point + g.normal * NORMAL_PUSH,
                        rad,
                        team,
                        &SplatOpts {
                            seed,
                            stretch: None,
                            stretch_amt: None,
                            kind: None,
                        },
                    );
                    // JS L1779: `c.owner.addTurfNoSpecial(...)` — special ink
                    // counts as turf but never charges the gauge (spec §2).
                    if let Some(oi) = find_actor(actors, team, cloud.owner_slot) {
                        actors[oi].add_turf_no_special(area);
                    }
                }
            }
            // JS L1782-1791: the dps ladder. Runs for ghost clouds too — the
            // cloud hurts *this screen's* local actors wherever it lives
            // ("online: each client rains on its own actors", L1783); remote
            // victims are skipped (their owner's cloud decides there). The
            // owner's position is snapshotted before the loop (JS reads
            // `c.owner.pos` live; the owner shares the cloud's team and is
            // never a victim here, so the value cannot change mid-loop).
            let owner_pos = find_actor(actors, team, cloud.owner_slot).map(|oi| actors[oi].pos);
            for (ei, e) in actors.iter_mut().enumerate() {
                if e.team == team || !e.alive {
                    continue;
                }
                if remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false)) {
                    continue; // JS L1783: `e.remote` — each client rains on its own actors
                }
                let dx = e.pos.x - objects[i].pos.x;
                let dz = e.pos.z - objects[i].pos.z;
                if dx * dx + dz * dz > sp.radius * sp.radius || e.pos.y > objects[i].pos.y {
                    continue;
                }
                let from = vec3(e.pos.x, objects[i].pos.y - LOS_FROM_DOWN, e.pos.z);
                let to = vec3(e.pos.x, e.pos.y + LOS_TARGET_UP, e.pos.z);
                if !collision.los(from, to) {
                    continue;
                }
                // JS L1789: `e.damage(sp.dps * dt, c.owner, 'storm')` — a
                // direct damage call (no `applyHit` routing: the victim must
                // be local by the L1783 guard). The `killed → emit('hit')`
                // of L1790 is FX-only (the splat rides the actor event bus).
                e.damage(sp.dps * dt, Some((team, cloud.owner_slot)), owner_pos, t);
            }
        }
        if cloud.t >= cloud.dur {
            objects.swap_remove(i); // JS L1793: `clouds.splice(i, 1)`
        } else if let ObjectBody::Cloud(c) = &mut objects[i].body {
            *c = cloud;
        }
    }
}

/// JS `Projectiles.update` tail + `SpecialSystem.world` objects: step every
/// registered object body. Batch 2: clouds only; batches 3/4 add the missile
/// / bubble / twister arms here.
#[allow(clippy::too_many_arguments)]
pub fn step_objects(
    objects: &mut Vec<WorldObject>,
    dt: f32,
    actors: &mut [Actor],
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    step_clouds(objects, dt, actors, collision, paint, rng, remote, t);
}

// ---------------------------------------------------------------------------
// batch-3 constructors (JS class ctors: `Missile` L777 / `Tornado` L823 /
// `Twister` L878 / `Speaker` L935 / `Bubble` L1023 / `Orb` L1194). The object
// `kind` carries the wire tag (`'mi'` / `'tw'` / `'sk'` / `'bu'` / `'or'`,
// specials.js L311-317); `'tornado'` never rides the wire (the missile pushes
// it directly, L812).
// ---------------------------------------------------------------------------

/// JS `new Missile(sys, a, to)` (L777-790): `from = a.pos + 1.6y` is baked in
/// by the caller (the `1.6` is hardcoded, D-T35-4). No RNG draws.
#[must_use]
pub fn make_missile(
    owner_slot: usize,
    team: usize,
    ghost: bool,
    from: Vec3,
    to: Vec3,
) -> WorldObject {
    let d = &crate::special::specials().strike;
    WorldObject {
        kind: "mi".to_string(),
        gid: 0,
        team,
        pos: from,
        ghost,
        dead: false,
        body: ObjectBody::Missile(Missile {
            owner_slot,
            t: 0.0,
            flight: d.flight,
            from,
            to,
        }),
    }
}

/// JS `new Tornado(sys, owner, team, pos)` (L823-839) minus the ctor paint
/// (the caller runs [`paint_splat`] with the right mute context).
#[must_use]
pub fn make_tornado(owner_slot: usize, team: usize, ghost: bool, pos: Vec3) -> WorldObject {
    let d = &crate::special::specials().strike;
    WorldObject {
        kind: "tornado".to_string(),
        gid: 0,
        team,
        pos,
        ghost,
        dead: false,
        body: ObjectBody::Tornado(Tornado {
            owner_slot,
            t: 0.0,
            dur: d.duration,
            radius: d.radius,
            paint_t: 0.0,
        }),
    }
}

/// JS `new Twister(sys, a, from, dir)` (L878-887). No RNG draws.
#[must_use]
pub fn make_twister(
    owner_slot: usize,
    team: usize,
    ghost: bool,
    pos: Vec3,
    dir: Vec3,
) -> WorldObject {
    let d = &crate::special::specials().zooka;
    WorldObject {
        kind: "tw".to_string(),
        gid: 0,
        team,
        pos,
        ghost,
        dead: false,
        body: ObjectBody::Twister(Twister {
            owner_slot,
            vel: dir * d.speed,
            t: 0.0,
            life: d.range / d.speed,
            hit: Vec::new(),
            paint_acc: 0.0,
        }),
    }
}

/// JS `new Speaker(sys, a, pos, dir)` (L935-972): the mouth is the cone tip
/// `(0, 0.8, 0.5)` rotated by `rotY = atan2(dir.x, dir.z)` about `pos`
/// (L941/L956/L960 — the fallback prop geometry, D-T35-4); the beam direction
/// re-clamps the pitch to `asin(clamp(dir.y, -0.3, 0.3))` (L962-963). No RNG.
#[must_use]
pub fn make_speaker(
    owner_slot: usize,
    team: usize,
    ghost: bool,
    pos: Vec3,
    dir: Vec3,
) -> WorldObject {
    let d = &crate::special::specials().wail;
    let nd = dir.normalize();
    let rot_y = nd.x.atan2(nd.z);
    let mouth = vec3(
        pos.x + 0.5 * rot_y.sin(),
        pos.y + 0.8,
        pos.z + 0.5 * rot_y.cos(),
    );
    let pitch = nd.y.clamp(-0.3, 0.3).asin();
    let beam = vec3(
        rot_y.sin() * pitch.cos(),
        pitch.sin(),
        rot_y.cos() * pitch.cos(),
    );
    WorldObject {
        kind: "sk".to_string(),
        gid: 0,
        team,
        pos,
        ghost,
        dead: false,
        body: ObjectBody::Speaker(Speaker {
            owner_slot,
            t: 0.0,
            phase: 0,
            dir: beam,
            mouth,
            range: d.range,
            radius: d.radius,
        }),
    }
}

/// JS `new Bubble(sys, a)` (L1023-1031): one draw (`ph = random() * 6`). The
/// muzzle hook owns `pos` / `r` while `held`.
#[must_use]
pub fn make_bubble(owner_slot: usize, team: usize, ghost: bool, rng: &mut Rng) -> WorldObject {
    let d = &crate::special::specials().blower;
    WorldObject {
        kind: "bu".to_string(),
        gid: 0,
        team,
        pos: Vec3::ZERO,
        ghost,
        dead: false,
        body: ObjectBody::Bubble(Bubble {
            owner_slot,
            t: 0.0,
            life: d.life,
            r: d.r_min,
            vel: Vec3::ZERO,
            charge: 0.0,
            held: true,
            ph: rng.next_f32() * 6.0,
        }),
    }
}

/// JS `Bubble.release(dir)` (L1032-1037): the `0.25` lift is hardcoded
/// (D-T35-4).
pub fn bubble_release(b: &mut Bubble, f: Vec3) {
    let d = &crate::special::specials().blower;
    b.held = false;
    let mut v = vec3(f.x, 0.0, f.z).normalize_or_zero() * d.drift;
    v.y = 0.25;
    b.vel = v;
}

/// JS `new Orb(sys, a, from, vel)` (L1194-1202). No RNG draws.
#[must_use]
pub fn make_orb(owner_slot: usize, team: usize, ghost: bool, from: Vec3, vel: Vec3) -> WorldObject {
    let d = &crate::special::specials().booyah;
    WorldObject {
        kind: "or".to_string(),
        gid: 0,
        team,
        pos: from,
        ghost,
        dead: false,
        body: ObjectBody::Orb(Orb {
            owner_slot,
            vel,
            t: 0.0,
            phase: 0,
            prev: from,
            fuse: d.fuse,
        }),
    }
}

/// JS `new ReturnMarker(sys, a, pos, icon)` (L689-691): the sim keeps only
/// the lifecycle flags. Never recorded on the wire (JS pushes it directly to
/// `sys.world`, L1515/L1775 — no `gid`, no `rec`); the ghost copy is rebuilt
/// from the owner's start record.
#[must_use]
pub fn make_marker(
    owner_slot: usize,
    team: usize,
    icon: &str,
    pos: Vec3,
    ghost: bool,
) -> WorldObject {
    WorldObject {
        kind: "return".to_string(),
        gid: 0,
        team,
        pos,
        ghost,
        dead: false,
        body: ObjectBody::Marker(Marker {
            owner_slot,
            icon: icon.to_string(),
            t: 0.0,
            done: false,
            back: false,
            fade: 1.0,
        }),
    }
}

/// JS `new ThrownStamp(sys, a, from, dir)` (L1105-1108): `vel = dir *
/// throwSpeed` then `vel.y += 2.5` (the `2.5` lift is hardcoded, D-T35-4).
#[must_use]
pub fn make_tstamp(
    owner_slot: usize,
    team: usize,
    ghost: bool,
    from: Vec3,
    dir: Vec3,
) -> WorldObject {
    const THROW_LIFT: f32 = 2.5; // JS L1108
    let d = &crate::special::specials().stamp;
    let mut vel = dir * d.throw_speed;
    vel.y += THROW_LIFT;
    WorldObject {
        kind: "ts".to_string(),
        gid: 0,
        team,
        pos: from,
        ghost,
        dead: false,
        body: ObjectBody::TStamp(TStamp {
            owner_slot,
            vel,
            prev: from,
            t: 0.0,
        }),
    }
}

/// JS `new Shell(sys, a, from, vel)` (L1157-1159). No RNG draws.
#[must_use]
pub fn make_shell(
    owner_slot: usize,
    team: usize,
    ghost: bool,
    from: Vec3,
    vel: Vec3,
) -> WorldObject {
    WorldObject {
        kind: "sh".to_string(),
        gid: 0,
        team,
        pos: from,
        ghost,
        dead: false,
        body: ObjectBody::Shell(Shell {
            owner_slot,
            vel,
            prev: from,
            t: 0.0,
        }),
    }
}

/// The most recent live return marker of `owner_slot` (JS `s.marker` — the
/// owner keeps one per active special; `rposition` skips an older marker
/// still fading out when a new special started before the old copy removed
/// itself).
#[must_use]
pub fn find_marker(objects: &[WorldObject], owner_slot: usize) -> Option<usize> {
    objects.iter().rposition(|w| {
        matches!(
            &w.body,
            ObjectBody::Marker(m) if m.owner_slot == owner_slot && !w.dead
        )
    })
}

// ---------------------------------------------------------------------------
// bubble hit hooks (JS `shotHit` / `rayHit` / `areaHit` / `_bubbleHit`,
// L442-483) — called from the projectile step and the twister's own update.
// `muted` is JS's `netMuted()` at the call site (a ghost object's hooks drop
// the bubble effects but still report the geometry hit).
// ---------------------------------------------------------------------------

/// JS `shotHit(prev, pos, team, dmg, owner)` (L442-449): the first bubble
/// whose capsule the segment grazes absorbs the shot — the *geometry* hit
/// returns `true` even when `_bubbleHit`'s effects are muted.
#[allow(clippy::too_many_arguments)]
pub fn shot_hit(
    prev: Vec3,
    pos: Vec3,
    team: usize,
    dmg: f32,
    owner: Option<usize>,
    objects: &mut [WorldObject],
    ctx: &mut WorldCtx,
    muted: bool,
) -> bool {
    for wi in 0..objects.len() {
        let b = match &objects[wi].body {
            ObjectBody::Bubble(b) => *b,
            _ => continue,
        };
        if objects[wi].dead {
            continue; // JS L444
        }
        let wp = objects[wi].pos;
        let res = segment_capsule_dist(
            prev,
            pos,
            vec3(wp.x, wp.y - b.r * 0.5, wp.z),
            b.r * 0.5,
            b.r,
        );
        if res.dist < b.r * 0.95 {
            bubble_hit(wi, team, dmg, owner, objects, ctx, muted);
            return true;
        }
    }
    false
}

/// JS `rayHit(from, dir, len, team, dmg, owner)` (L450-464): shortens the
/// beam to the near edge of the closest bubble; the `0.1` floor is JS's own
/// (L460, D-T35-4). `dir` is unit.
#[allow(clippy::too_many_arguments)]
pub fn ray_hit(
    from: Vec3,
    dir: Vec3,
    len: f32,
    team: usize,
    dmg: f32,
    owner: Option<usize>,
    objects: &mut [WorldObject],
    ctx: &mut WorldCtx,
    muted: bool,
) -> f32 {
    let mut best = len;
    let mut hit_b: Option<usize> = None;
    for (wi, o) in objects.iter().enumerate() {
        let b = match &o.body {
            ObjectBody::Bubble(b) => *b,
            _ => continue,
        };
        if o.dead {
            continue; // JS L453
        }
        let v = o.pos - from;
        let t = v.dot(dir);
        if t <= 0.0 || t > best {
            continue; // JS L456
        }
        let d2 = v.length_squared() - t * t;
        if d2 > b.r * b.r {
            continue; // JS L458
        }
        let tt = t - (b.r * b.r - d2).sqrt();
        if tt < best {
            best = 0.1f32.max(tt); // JS L460 (the quirk: `best` floors at 0.1
            // while the comparison runs on raw `tt`)
            hit_b = Some(wi);
        }
    }
    if let Some(wi) = hit_b {
        bubble_hit(wi, team, dmg, owner, objects, ctx, muted);
    }
    best
}

/// JS `areaHit(c, radius, dmg, team, owner)` (L465-470): every bubble in the
/// blast takes `dmg * 1.5` (no early exit).
#[allow(clippy::too_many_arguments)]
pub fn area_hit(
    c: Vec3,
    radius: f32,
    dmg: f32,
    team: usize,
    owner: Option<usize>,
    objects: &mut [WorldObject],
    ctx: &mut WorldCtx,
    muted: bool,
) {
    let mut hits: Vec<usize> = Vec::new();
    for (wi, o) in objects.iter().enumerate() {
        let b = match &o.body {
            ObjectBody::Bubble(b) => *b,
            _ => continue,
        };
        if o.dead {
            continue; // JS L467
        }
        if o.pos.distance(c) < radius + b.r * 0.8 {
            hits.push(wi);
        }
    }
    for wi in hits {
        bubble_hit(wi, team, dmg * 1.5, owner, objects, ctx, muted);
    }
}

/// JS `_bubbleHit(w, team, dmg, owner)` (L473-483). A ghost bubble reports the
/// hit to its owner over `netHurt('sp', gid, …)` (negative `d` = same-team
/// ink; the `d > 0` gate in `netHurt`, registry.js L68, drops it on the wire).
/// Own-team ink charges a released bubble until it pops; enemy ink soaks and
/// shrinks it (floor `rMin * 0.75`, JS L482).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bubble_hit(
    wi: usize,
    team: usize,
    dmg: f32,
    owner: Option<usize>,
    objects: &mut [WorldObject],
    ctx: &mut WorldCtx,
    muted: bool,
) {
    if objects[wi].dead || muted {
        return; // JS L474
    }
    let b = match &objects[wi].body {
        ObjectBody::Bubble(b) => *b,
        _ => return,
    };
    let wt = objects[wi].team;
    if objects[wi].ghost {
        // JS L476
        if !(team == wt && b.held) {
            ctx.events.push(SimEvent::DeviceHit {
                owner_slot: b.owner_slot,
                team: wt,
                kind: "sp".to_string(),
                gid: objects[wi].gid,
                d: if team == wt { -dmg } else { dmg },
            });
        }
        return;
    }
    let d = &crate::special::specials().blower;
    if team == wt {
        if b.held {
            return; // JS L479
        }
        let mut b2 = b;
        b2.charge += dmg;
        objects[wi].body = ObjectBody::Bubble(b2);
        if b2.charge >= d.pop_damage {
            bubble_explode(wi, owner, objects, ctx);
        }
    } else {
        let mut b2 = b;
        b2.r = (d.r_min * 0.75).max(b2.r - dmg * 0.004);
        objects[wi].body = ObjectBody::Bubble(b2);
    }
}

/// JS `Bubble.explode(by)` (L1081-1099): records `[3, gid, 'x']` for the other
/// screens, churns 6 + 1 floor splats, then the `blast` (attacker = the
/// same-team shooter when one exists, else the bubble's owner). Always runs
/// *unmuted* (JS: `_bubbleHit` already returned under `netMuted`, and the
/// ghost-side `netEvent` route is not wrapped in `ghostMute` — the paint and
/// device damage are real; player damage drops on the remote attacker).
pub fn bubble_explode(
    wi: usize,
    by: Option<usize>,
    objects: &mut [WorldObject],
    ctx: &mut WorldCtx,
) {
    if objects[wi].dead {
        return; // JS L1082
    }
    objects[wi].dead = true;
    let b = match &objects[wi].body {
        ObjectBody::Bubble(b) => *b,
        _ => return,
    };
    let team = objects[wi].team;
    let owner_idx = find_actor(ctx.actors, team, b.owner_slot);
    if !objects[wi].ghost && objects[wi].gid != 0 {
        // JS L1084: `rec(this.owner, [3, this.gid, 'x'])`
        if let Some(oi) = owner_idx {
            ctx.actors[oi].sp_records.push(vec![
                KitVal::Num(3.0),
                KitVal::Num(objects[wi].gid as f32),
                KitVal::Str("x".to_string()),
            ]);
        }
    }
    let d = &crate::special::specials().blower;
    let r_blast = b.r * d.blast_mul;
    let c = objects[wi].pos;
    // JS L1087: `const owner = by && by.team === this.team ? by : this.owner`
    let attacker_idx = match by {
        Some(bi) if ctx.actors[bi].team == team => Some(bi),
        _ => owner_idx,
    };
    let attacker_slot = attacker_idx
        .map(|ai| ctx.actors[ai].slot)
        .unwrap_or(b.owner_slot);
    for k in 0..6 {
        // JS L1088-1092: 4 draws per landed drip, 2 per miss.
        let ang = (k as f32 / 6.0) * TAU + ctx.rng.next_f32() * 0.4;
        let rr = r_blast * (0.35 + ctx.rng.next_f32() * 0.5);
        let g = ground_below(
            ctx.collision,
            vec3(c.x + ang.cos() * rr, c.y + 1.0, c.z + ang.sin() * rr),
            6.0,
        );
        if let Some(g) = g {
            let rad = 1.2 + ctx.rng.next_f32() * 0.6;
            paint_splat(
                ctx,
                attacker_idx,
                vec3(g.x, g.y + 0.2, g.z),
                rad,
                team,
                false,
            );
        }
    }
    if let Some(g0) = ground_below(ctx.collision, c, 6.0) {
        paint_splat(
            ctx,
            attacker_idx,
            vec3(g0.x, g0.y + 0.2, g0.z),
            r_blast * 0.55,
            team,
            false,
        );
    }
    crate::special::blast(
        ctx.events,
        ctx.subs,
        ctx.actors,
        ctx.collision,
        attacker_slot,
        team,
        c,
        r_blast,
        d.damage_max,
        d.damage_min,
        r_blast * 0.45,
        ctx.remote,
        ctx.t,
        false,
    );
}

/// JS `Orb._blast()` (L1225-1240): one floor splat + 12 ring drips + the
/// `blast` (the `0.3` / `0.72` / `0.2` / `2` / `8` / `0.3` / `0.45` / `1.3` /
/// `0.7` constants are JS's, D-T35-4).
fn orb_blast(objects: &mut [WorldObject], i: usize, o: &Orb, ctx: &mut WorldCtx) {
    let d = &crate::special::specials().booyah;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    let c = objects[i].pos;
    let owner_idx = find_actor(ctx.actors, team, o.owner_slot);
    let g = ground_below(ctx.collision, c, 6.0);
    let base = g.unwrap_or(c) + vec3(0.0, 0.3, 0.0);
    paint_splat(ctx, owner_idx, base, d.radius * 0.72, team, ghost);
    for k in 0..12 {
        let ang = (k as f32 / 12.0) * TAU + ctx.rng.next_f32() * 0.3;
        let rr = d.radius * (0.5 + ctx.rng.next_f32() * 0.45);
        let gg = ground_below(
            ctx.collision,
            vec3(c.x + ang.cos() * rr, c.y + 2.0, c.z + ang.sin() * rr),
            8.0,
        );
        if let Some(gg) = gg {
            let rad = 1.3 + ctx.rng.next_f32() * 0.7;
            paint_splat(
                ctx,
                owner_idx,
                vec3(gg.x, gg.y + 0.2, gg.z),
                rad,
                team,
                ghost,
            );
        }
    }
    crate::special::blast(
        ctx.events,
        ctx.subs,
        ctx.actors,
        ctx.collision,
        o.owner_slot,
        team,
        c,
        d.radius,
        d.damage_max,
        d.damage_min,
        d.kill_radius,
        ctx.remote,
        ctx.t,
        ghost,
    );
}

// ---------------------------------------------------------------------------
// per-object step arms (JS `update(dt)` on each class)
// ---------------------------------------------------------------------------

/// JS `Missile.update` (L791-818): climb `u² * 30` over the first 35 % of the
/// flight, then dive `34(1-u)² + 0.3` onto the target; at `k >= 1` a (ghost of
/// a) tornado lands at `to` — pushed directly, never recorded. No RNG draws.
fn step_missile(
    objects: &mut Vec<WorldObject>,
    i: usize,
    mut m: Missile,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    let d = &crate::special::specials().strike;
    m.t += dt;
    let k = m.t / m.flight;
    let ghost = objects[i].ghost;
    let team = objects[i].team;
    if k < 0.35 {
        let u = k / 0.35;
        objects[i].pos = vec3(m.from.x, m.from.y + u * u * 30.0, m.from.z);
    } else {
        let u = (k - 0.35) / 0.65;
        let o = 1.0 - u;
        objects[i].pos = vec3(m.to.x, m.to.y + 34.0 * o * o + 0.3, m.to.z);
    }
    if k >= 1.0 {
        // JS L809-815: `new Tornado(...)` (ctor paint at `to + 0.4y`,
        // `radius * 0.7`) then `sys.world.push(tn)` — no gid, no record.
        let owner_idx = find_actor(ctx.actors, team, m.owner_slot);
        paint_splat(
            ctx,
            owner_idx,
            vec3(m.to.x, m.to.y + 0.4, m.to.z),
            d.radius * 0.7,
            team,
            ghost,
        );
        objects.push(make_tornado(m.owner_slot, team, ghost, m.to));
        return false;
    }
    objects[i].body = ObjectBody::Missile(m);
    true
}

/// JS `Tornado.update` (L840-868): the 0.1 s churn window (2 drips, 4 draws
/// per landed drip / 2 per miss), then hurt + drag enemies inside the column.
/// The drag applies to *remote* proxies too (JS L864 runs outside `tickDamage`
/// — only the damage is per-client).
fn step_tornado(
    objects: &mut [WorldObject],
    i: usize,
    mut tn: Tornado,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    let d = &crate::special::specials().strike;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    let pos = objects[i].pos;
    tn.t += dt;
    let fade = ((tn.dur - tn.t) / 0.6).clamp(0.0, 1.0);
    let owner_idx = find_actor(ctx.actors, team, tn.owner_slot);
    tn.paint_t -= dt;
    if tn.paint_t <= 0.0 {
        tn.paint_t = 0.1;
        for _k in 0..2 {
            let ang = ctx.rng.next_f32() * TAU;
            let r = ctx.rng.next_f32().sqrt() * tn.radius;
            let g = ground_below(
                ctx.collision,
                vec3(pos.x + ang.cos() * r, pos.y + 3.0, pos.z + ang.sin() * r),
                8.0,
            );
            if let Some(g) = g {
                let rad = 0.9 + ctx.rng.next_f32() * 0.8;
                paint_splat(ctx, owner_idx, vec3(g.x, g.y + 0.2, g.z), rad, team, ghost);
            }
        }
    }
    // JS L859-865: hurt (and gently drag) enemies inside the column.
    let mut victims: Vec<(usize, f32, f32, f32)> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        let dx = pos.x - e.pos.x;
        let dz = pos.z - e.pos.z;
        let dh = dx.hypot(dz);
        if dh > tn.radius || e.pos.y < pos.y - 1.5 || e.pos.y > pos.y + 7.0 {
            continue;
        }
        victims.push((ei, dx, dz, dh));
    }
    for (ei, dx, dz, dh) in victims {
        if let Some(oi) = owner_idx {
            tick_damage(ctx, oi, ei, d.dps * dt * fade);
        }
        if dh > 0.3 {
            ctx.actors[ei].vel.x += (dx / dh) * d.pull * dt;
            ctx.actors[ei].vel.z += (dz / dh) * d.pull * dt;
        }
    }
    objects[i].body = ObjectBody::Tornado(tn);
    tn.t < tn.dur
}

/// JS `Twister._burst(p, n, soft)` (L926-929): one paint at `p + n * 0.15`
/// (radius 1.6, or 1.1 when `soft`). The FX burst is `near()`-gated (no draws).
fn twister_burst(
    ctx: &mut WorldCtx,
    owner_slot: usize,
    team: usize,
    ghost: bool,
    p: Vec3,
    n: Vec3,
    soft: bool,
) {
    let rad = if soft { 1.1 } else { 1.6 };
    let owner_idx = find_actor(ctx.actors, team, owner_slot);
    paint_splat(ctx, owner_idx, p + n * 0.15, rad, team, ghost);
}

/// JS `Twister.update` (L888-925): fly straight; walls / sub shields / bubbles
/// stop it; players pierce once; a floor drip every `paintEvery` metres; dies
/// past `life`.
fn step_twister(
    objects: &mut [WorldObject],
    i: usize,
    mut tw: Twister,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    let d = &crate::special::specials().zooka;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    tw.t += dt;
    let prev = objects[i].pos;
    objects[i].pos = prev + tw.vel * dt;
    let pos = objects[i].pos;
    let owner_idx = find_actor(ctx.actors, team, tw.owner_slot);
    // JS L894-897: walls end it, then sub shields, then bubbles.
    let seg = ctx.collision.segment(prev, pos, true);
    if seg.hit {
        twister_burst(
            ctx,
            tw.owner_slot,
            team,
            ghost,
            seg.point,
            seg.normal,
            false,
        );
        return false;
    }
    if ctx.subs.block_shot(prev, pos, team, 120.0, ghost, ctx.sb) {
        twister_burst(ctx, tw.owner_slot, team, ghost, pos, Vec3::Y, false);
        return false;
    }
    if shot_hit(prev, pos, team, 120.0, owner_idx, objects, ctx, ghost) {
        twister_burst(ctx, tw.owner_slot, team, ghost, pos, Vec3::Y, false);
        return false;
    }
    // JS L899-911: players inside the column (the capsule runs along the
    // horizontal segment at `prev.y`; `e.hitR` does not exist in the sim, so
    // `PLAYER.radius` is always used).
    let attacker_remote = owner_idx.is_some_and(|oi| {
        ctx.remote
            .is_some_and(|m| m.get(oi).copied().unwrap_or(false))
    });
    let mut newly: Vec<usize> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive || tw.hit.contains(&ei) {
            continue;
        }
        let top = e.pos.y + if e.form == Form::Squid { 0.6 } else { 1.6 };
        if top < pos.y - d.height * 0.5 || e.pos.y > pos.y + d.height * 0.5 {
            continue;
        }
        let res = segment_capsule_dist(
            vec3(prev.x, prev.y, prev.z),
            vec3(pos.x, prev.y, pos.z),
            vec3(e.pos.x, prev.y - 0.5, e.pos.z),
            0.01,
            1.0,
        );
        if res.dist < d.radius + ctx.t.radius {
            newly.push(ei);
        }
    }
    for ei in newly {
        tw.hit.push(ei); // JS L907 runs before `applyHit`
        if attacker_remote {
            continue; // JS `shouldApplyHit`: a ghost's hits drop on this screen
        }
        let (vs, vt) = (ctx.actors[ei].slot, ctx.actors[ei].team);
        let send = ctx
            .remote
            .is_some_and(|m| m.get(ei).copied().unwrap_or(false));
        if send {
            ctx.events.push(SimEvent::HitSend {
                attacker_slot: tw.owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: d.damage,
            });
        } else {
            let atk_pos = owner_idx.map(|oi| ctx.actors[oi].pos);
            let outcome = apply_hit(
                &mut ctx.actors[ei],
                d.damage,
                Some((team, tw.owner_slot)),
                atk_pos,
                ctx.t,
            );
            ctx.events.push(SimEvent::Hit {
                attacker_slot: tw.owner_slot,
                victim_slot: vs,
                team,
                damage: d.damage,
                killed: outcome.killed,
            });
        }
    }
    // JS L913-918: a line of ink under the path (2 draws, only when the floor
    // is found; the counter resets even on a miss).
    tw.paint_acc += tw.vel.length() * dt;
    if tw.paint_acc > d.paint_every {
        tw.paint_acc = 0.0;
        if let Some(g) = ground_below(ctx.collision, pos, 3.5) {
            let rad = d.paint_radius * (0.85 + ctx.rng.next_f32() * 0.3);
            paint_splat(ctx, owner_idx, vec3(g.x, g.y + 0.15, g.z), rad, team, ghost);
        }
    }
    if tw.t > tw.life {
        twister_burst(ctx, tw.owner_slot, team, ghost, pos, Vec3::Y, true);
        return false;
    }
    objects[i].body = ObjectBody::Twister(tw);
    true
}

/// JS `Speaker.update` (L973-1017): charge `d.charge` s, blast `d.blast` s
/// (through walls — no LOS), fade 0.6 s. The beam test uses the mouth + the
/// clamped-pitch direction baked by [`make_speaker`]. No RNG.
fn step_speaker(
    objects: &mut [WorldObject],
    i: usize,
    mut sp: Speaker,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    let d = &crate::special::specials().wail;
    let team = objects[i].team;
    sp.t += dt;
    let keep = if sp.phase == 0 {
        if sp.t >= d.charge {
            sp.phase = 1;
            sp.t = 0.0;
        }
        true
    } else if sp.phase == 1 {
        // JS L998-1006: everyone in the beam, walls or not.
        let owner_idx = find_actor(ctx.actors, team, sp.owner_slot);
        let mut victims: Vec<usize> = Vec::new();
        for (ei, e) in ctx.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            let mut v = e.pos;
            v.y += 0.8;
            v -= sp.mouth;
            let along = v.dot(sp.dir);
            if along < -0.5 || along > sp.range {
                continue;
            }
            let off = (v - sp.dir * along).length();
            if off < sp.radius + 0.45 {
                victims.push(ei);
            }
        }
        if let Some(oi) = owner_idx {
            for ei in victims {
                tick_damage(ctx, oi, ei, d.dps * dt);
            }
        }
        if sp.t >= d.blast {
            sp.phase = 2;
            sp.t = 0.0;
        }
        true
    } else {
        sp.t / 0.6 < 1.0 // JS L1012-1016
    };
    objects[i].body = ObjectBody::Speaker(sp);
    keep
}

/// JS `Bubble.update` (L1038-1073): drag + bob + deck support + wall stop,
/// enemies cannot pass through (the push applies to *remote* proxies too —
/// JS has no `e.remote` guard here), life expiry pops it.
fn step_bubble(
    objects: &mut [WorldObject],
    i: usize,
    mut b: Bubble,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    b.t += dt;
    if !b.held {
        b.life -= dt;
        b.vel *= (-0.45 * dt).exp();
        b.vel.y += (b.t * 1.7 + b.ph).sin() * 0.15 * dt;
        let old = objects[i].pos;
        objects[i].pos += b.vel * dt;
        // JS L1049-1050: don't sink into the deck (`groundHeight`, no grate
        // skip — JS L1049 passes no flag).
        let pos = objects[i].pos;
        let g = ctx
            .collision
            .ground_height(pos.x, pos.z, pos.y + 1.0, false);
        if g != f32::NEG_INFINITY && pos.y - b.r * 0.85 < g {
            objects[i].pos.y = g + b.r * 0.85;
            b.vel.y = b.vel.y.max(0.0);
        }
        // JS L1051-1052: stop at walls (the ray runs from the pre-move pos).
        let dir = b.vel.normalize_or_zero();
        let w = ctx
            .collision
            .raycast(old, dir, b.r * 0.8 + b.vel.length() * dt, true);
        if w.hit && w.normal.y.abs() < 0.6 {
            objects[i].pos = old;
            b.vel = vec3(0.0, b.vel.y, 0.0);
        }
        // JS L1054-1065: enemies can't pass through.
        let pos = objects[i].pos;
        let mut push: Vec<(usize, f32, f32, f32)> = Vec::new();
        for (ei, e) in ctx.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            let vx = e.pos.x - pos.x;
            let vz = e.pos.z - pos.z;
            let dy = e.pos.y + 0.8 - pos.y;
            let r_ring = (b.r * b.r - dy * dy).max(0.0).sqrt() + ctx.t.radius;
            let dh = vx.hypot(vz);
            if dh < r_ring && dh > 1e-3 {
                push.push((ei, vx / dh, vz / dh, r_ring));
            }
        }
        for (ei, ux, uz, r_ring) in push {
            ctx.actors[ei].pos.x = pos.x + ux * r_ring;
            ctx.actors[ei].pos.z = pos.z + uz * r_ring;
            let vn = ctx.actors[ei].vel.x * ux + ctx.actors[ei].vel.z * uz;
            if vn < 0.0 {
                ctx.actors[ei].vel.x -= ux * vn;
                ctx.actors[ei].vel.z -= uz * vn;
            }
        }
        if b.life <= 0.0 {
            // JS L1066 + `_pop` L1074-1078: dead + one floor splat.
            objects[i].dead = true;
            let pos = objects[i].pos;
            let owner_idx = find_actor(ctx.actors, team, b.owner_slot);
            paint_splat(
                ctx,
                owner_idx,
                vec3(pos.x, pos.y - b.r * 0.5, pos.z),
                b.r * 0.6,
                team,
                ghost,
            );
            objects[i].body = ObjectBody::Bubble(b);
            return false;
        }
    }
    objects[i].body = ObjectBody::Bubble(b);
    !objects[i].dead
}

/// JS `Orb.update` (L1203-1224): ballistic fly (gravity `24`, JS L1209,
/// D-T35-4), stick to the first surface hit, fuse `d.fuse` then blast; a
/// 4 s miss or a drop under the sea removes it silently.
fn step_orb(
    objects: &mut [WorldObject],
    i: usize,
    mut o: Orb,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    o.t += dt;
    if o.phase == 0 {
        o.prev = objects[i].pos;
        o.vel.y -= 24.0 * dt;
        objects[i].pos += o.vel * dt;
        let seg = ctx.collision.segment(o.prev, objects[i].pos, true);
        if seg.hit {
            objects[i].pos = seg.point + seg.normal * 0.5;
            o.phase = 1;
            o.t = 0.0;
        } else if o.t > 4.0 || objects[i].pos.y < ctx.t.water_y - 2.0 {
            return false; // JS L1213: silent removal
        }
    } else if o.t / o.fuse >= 1.0 {
        orb_blast(objects, i, &o, ctx);
        return false;
    }
    objects[i].body = ObjectBody::Orb(o);
    true
}

/// JS `ReturnMarker.update` (L745-748) minus the presentation: the fade
/// ladder (`dt * 3`, JS L747) and the 30 s safety cap (JS L748) are the only
/// sim-relevant lines.
fn step_marker(objects: &mut [WorldObject], i: usize, mut mk: Marker, dt: f32) -> bool {
    const FADE_RATE: f32 = 3.0; // JS L747
    const LIFE_CAP: f32 = 30.0; // JS L748
    mk.t += dt;
    if mk.done {
        mk.fade -= dt * FADE_RATE;
        if mk.fade <= 0.0 {
            return false; // JS L747: `if (this.fade <= 0) return false`
        }
    }
    if mk.t > LIFE_CAP {
        mk.done = true;
    }
    objects[i].body = ObjectBody::Marker(mk);
    true
}

/// JS `ThrownStamp.update` (L1116-1136): ballistic sweep — surface segment,
/// then the enemy capsule scan, then the bubble `shotHit`; boom / 3 s / sea
/// removal. The `12` gravity, `0.3` normal push, `0.7`/`1.6` probe capsule,
/// `0.8` hit distance, `200` bubble damage and `3` life are hardcoded in JS
/// (D-T35-4).
fn step_tstamp(
    objects: &mut [WorldObject],
    i: usize,
    mut s: TStamp,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    const GRAV: f32 = 12.0; // JS L1119
    const SEG_PUSH: f32 = 0.3; // JS L1123
    const CAP_R: f32 = 0.7; // JS L1126
    const CAP_H: f32 = 1.6; // JS L1126
    const CAP_DIST: f32 = 0.8; // JS L1127
    const SHOT_DMG: f32 = 200.0; // JS L1129
    const LIFE: f32 = 3.0; // JS L1132
    s.t += dt;
    s.prev = objects[i].pos;
    s.vel.y -= GRAV * dt;
    objects[i].pos += s.vel * dt;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    let pos = objects[i].pos;
    let mut at: Option<Vec3> = None;
    let seg = ctx.collision.segment(s.prev, pos, true);
    if seg.hit {
        at = Some(seg.point + seg.normal * SEG_PUSH);
    }
    if at.is_none() {
        for e in ctx.actors.iter() {
            if e.team == team || !e.alive {
                continue; // JS L1125
            }
            let res = segment_capsule_dist(s.prev, pos, e.pos, CAP_R, CAP_H);
            if res.dist < CAP_DIST {
                at = Some(s.prev.lerp(pos, res.t));
                break;
            }
        }
    }
    let owner_idx = find_actor(ctx.actors, team, s.owner_slot);
    if at.is_none() && shot_hit(s.prev, pos, team, SHOT_DMG, owner_idx, objects, ctx, ghost) {
        at = Some(pos);
    }
    if at.is_some() || s.t > LIFE || pos.y < ctx.t.water_y - 2.0 {
        if let Some(c) = at {
            tstamp_boom(objects, i, s.owner_slot, c, ctx);
        }
        return false;
    }
    objects[i].body = ObjectBody::TStamp(s);
    true
}

/// JS `ThrownStamp._boom(c)` (L1138-1150): one floor splat (`* 0.8`), 4 ring
/// drips (`0.9` r) and the blast (kill radius `1.2`) — all hardcoded in JS
/// (D-T35-4). RNG order: floor seed, then per drip `ang`, `rr`, seed.
fn tstamp_boom(objects: &[WorldObject], i: usize, owner_slot: usize, c: Vec3, ctx: &mut WorldCtx) {
    const BOOM_PROBE: f32 = 4.0; // JS L1140 `groundBelow(c, 4)`
    const FLOOR_LIFT: f32 = 0.2; // JS L1141
    const DRIP_Y: f32 = 0.5; // JS L1144
    const DRIP_R: f32 = 0.9; // JS L1144
    const KILL_R: f32 = 1.2; // JS L1150
    let d = &crate::special::specials().stamp;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    let owner_idx = find_actor(ctx.actors, team, owner_slot);
    let g = ground_below(ctx.collision, c, BOOM_PROBE);
    let base = g.unwrap_or(c);
    paint_splat(
        ctx,
        owner_idx,
        vec3(base.x, base.y + FLOOR_LIFT, base.z),
        d.throw_radius * 0.8,
        team,
        ghost,
    );
    for _k in 0..4 {
        let ang = ctx.rng.next_f32() * TAU;
        let rr = d.throw_radius * (0.5 + ctx.rng.next_f32() * 0.4);
        paint_splat(
            ctx,
            owner_idx,
            vec3(c.x + ang.cos() * rr, c.y + DRIP_Y, c.z + ang.sin() * rr),
            DRIP_R,
            team,
            ghost,
        );
    }
    crate::special::blast(
        ctx.events,
        ctx.subs,
        ctx.actors,
        ctx.collision,
        owner_slot,
        team,
        c,
        d.throw_radius,
        d.throw_damage_max,
        d.throw_damage_min,
        KILL_R,
        ctx.remote,
        ctx.t,
        ghost,
    );
}

/// JS `Shell.update` (L1163-1187): mortar sweep — surface segment, enemy
/// capsule scan, bubble `shotHit`; on hit one splat + the cannon blast, else
/// keep while `t < 4` above the sea. The `20` gravity, `0.25` push / margin,
/// `150` bubble damage, `0.75` splat factor, `0.8` kill radius and `4` life
/// are hardcoded in JS (D-T35-4). JS probes `e.hitR || PLAYER.radius`
/// (L1173-1174) — the sim has no per-actor hitbox override, so
/// [`PlayerTuning::radius`] / [`PlayerTuning::height`] always apply.
fn step_shell(
    objects: &mut [WorldObject],
    i: usize,
    mut sh: Shell,
    dt: f32,
    ctx: &mut WorldCtx,
) -> bool {
    const GRAV: f32 = 20.0; // JS L1166
    const SEG_PUSH: f32 = 0.25; // JS L1170
    const HIT_MARGIN: f32 = 0.25; // JS L1174
    const SHOT_DMG: f32 = 150.0; // JS L1176
    const FLOOR_LIFT: f32 = 0.1; // JS L1180
    const KILL_R: f32 = 0.8; // JS L1184
    const LIFE: f32 = 4.0; // JS L1187
    sh.t += dt;
    sh.prev = objects[i].pos;
    sh.vel.y -= GRAV * dt;
    objects[i].pos += sh.vel * dt;
    let team = objects[i].team;
    let ghost = objects[i].ghost;
    let pos = objects[i].pos;
    let mut at: Option<Vec3> = None;
    let seg = ctx.collision.segment(sh.prev, pos, true);
    if seg.hit {
        at = Some(seg.point + seg.normal * SEG_PUSH);
    }
    if at.is_none() {
        for e in ctx.actors.iter() {
            if e.team == team || !e.alive {
                continue; // JS L1172
            }
            let res = segment_capsule_dist(sh.prev, pos, e.pos, ctx.t.radius, ctx.t.height);
            if res.dist < ctx.t.radius + HIT_MARGIN {
                at = Some(sh.prev.lerp(pos, res.t));
                break;
            }
        }
    }
    let owner_idx = find_actor(ctx.actors, team, sh.owner_slot);
    if at.is_none() && shot_hit(sh.prev, pos, team, SHOT_DMG, owner_idx, objects, ctx, ghost) {
        at = Some(pos);
    }
    if let Some(a) = at {
        let d = &crate::special::specials().crab;
        paint_splat(
            ctx,
            owner_idx,
            vec3(a.x, a.y + FLOOR_LIFT, a.z),
            d.cannon_radius * 0.75,
            team,
            ghost,
        );
        crate::special::blast(
            ctx.events,
            ctx.subs,
            ctx.actors,
            ctx.collision,
            sh.owner_slot,
            team,
            a,
            d.cannon_radius,
            d.cannon_damage_max,
            d.cannon_damage_min,
            KILL_R,
            ctx.remote,
            ctx.t,
            ghost,
        );
        return false;
    }
    objects[i].body = ObjectBody::Shell(sh);
    sh.t < LIFE && pos.y > ctx.t.water_y - 2.0
}

/// JS `SpecialSystem.update` tail (L566-569) for the batch-3 / batch-4
/// bodies: step every non-cloud object in reverse order; a body whose
/// `update` returns false — or that is `dead` — is pruned (a bubble killed
/// by the weapon step still integrates one last frame first, exactly like
/// JS).
pub fn step_special_objects(objects: &mut Vec<WorldObject>, dt: f32, ctx: &mut WorldCtx) {
    let mut i = objects.len();
    while i > 0 {
        i -= 1;
        let body = std::mem::replace(&mut objects[i].body, ObjectBody::None);
        let keep = match body {
            ObjectBody::Missile(b) => step_missile(objects, i, b, dt, ctx),
            ObjectBody::Tornado(b) => step_tornado(objects, i, b, dt, ctx),
            ObjectBody::Twister(b) => step_twister(objects, i, b, dt, ctx),
            ObjectBody::Speaker(b) => step_speaker(objects, i, b, dt, ctx),
            ObjectBody::Bubble(b) => step_bubble(objects, i, b, dt, ctx),
            ObjectBody::Orb(b) => step_orb(objects, i, b, dt, ctx),
            ObjectBody::Marker(b) => step_marker(objects, i, b, dt),
            ObjectBody::TStamp(b) => step_tstamp(objects, i, b, dt, ctx),
            ObjectBody::Shell(b) => step_shell(objects, i, b, dt, ctx),
            other => {
                objects[i].body = other;
                continue;
            }
        };
        if !keep || objects[i].dead {
            objects.swap_remove(i);
        }
    }
}
