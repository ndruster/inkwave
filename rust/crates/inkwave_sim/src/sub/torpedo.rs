//! Tide Torpedo kit sub — the sim half of `src/game/kits/torpedo.js` (851
//! lines), batch two of Task 34 (spec §5).
//!
//! Thrown like a bomb (the throw inks a patch under the thrower's feet).
//! In flight it locks onto a foe in sight, unfolds, then swims at it and
//! bursts (wide paint + a droplet spray that ink and hit for 12, three per
//! foe). A torpedo that touches anything before locking bursts like a plain
//! bomb. Enemy fire shoots it down (20 hp); a locked torpedo treats lethal
//! blasts like a Drip Curtain (they set it off at once). One live per
//! owner: a second throw is refused.
//!
//! Keep in lockstep with upstream on sync:
//!   - `use` (L244-258)         -> [`use_torpedo`]
//!   - `spawn` (L261-277)       -> [`spawn`]
//!   - `ghost` (L279-305)       -> [`ghost`]
//!   - `tick` (L313-335)        -> [`tick`]
//!   - `fly` (L337-359)         -> [`fly`]
//!   - `findTarget` (L361-373)  -> [`find_target`]
//!   - `startLock` (L374-384)   -> [`start_lock`]
//!   - `unfold` (L386-401)      -> [`unfold`]
//!   - `launch` (L403-426)      -> [`launch`] / `swim` (L428-438) -> [`swim`]
//!   - `touchFoe` (L440-449)    -> [`touch_foe`]
//!   - `touchDevice` (L451-456) -> [`touch_device`]
//!   - `lethalContact` (L458-476) -> [`lethal_contact`]
//!   - `burst` (L478-513)       -> [`burst`]
//!   - `destroy` (L515-524) / `fizzle` (L525-528) / `hurt` (L529-536)
//!   - `spawnDrops` (L539-551)  -> [`spawn_drops`]
//!   - `tickDrops` (L552-570) / `dropStep` (L572-609) -> [`tick_drops`] / [`drop_step`]
//!   - `blockShot` (L666-675) / `blockRay` (L676-690) / `damageArea`
//!     (L692-700) / `areaHit` (L701-705) -> the `SubSystem` block hooks
//!   - `netHurt` (L726) / `KIT_GHOSTS.torpedoArea.netHurt` (L707-709)
//!     -> [`net_hurt`] / [`net_area_hurt`]
//!   - `blocked` (L733)         -> [`blocked`]
//!
//! Presentation-only branches dropped per the M3 range rule (spec §0): the
//! prop rig (`buildTorpedo` / `buildRig` / `orient` / `poseRig` / `finAt`),
//! the lock ring, the instanced drop mesh (`dropsMesh` / `dropMesh`), the
//! whirr / audio, `nearCam`, `G.fx`, the `sub:use` / `sub:arm` /
//! `sub:destroyed` / `bomb:explode` / `shake` emits (not in the netmatch
//! `FORWARD` list), `flash` / `glowI` / `fin` / `propOpen` / `propA` /
//! `roll` / `ringT` / `hover`'s visual use, the boss splash and the bot
//! layer (`threatOf` / the `bot` block / `THROWN`). The `roll` init
//! `Math.random()` (L270) is a mesh rotation draw and is not consumed.
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-9 is lifted: kit kinds now deploy through this module.
//!   - `sp` (Bomb Barrage, L268) is always false (D-T34-3): credit is the
//!     plain turf event.
//!   - `findTarget` drops the `!e.superJumpState` term (no super jump in
//!     the sim, D-T34-2).
//!   - `touchDevice`'s `G.specials?.shotHit` probe is Task 35 (no specials
//!     in the sim); only the sub-device probe is ported.
//!   - The lock record (`[3, gid, …]`) carries the *target's actor-list
//!     index* where JS carries `tgt.nid` (same pattern as waddle, D-T34-5);
//!     the replication layer translates idx -> nid outbound and nid -> idx
//!     inbound.
//!   - A lethal blast that catches a locked torpedo on the *public*
//!     `damage_area` fan (a splat-bomb blast during the runner's update, or
//!     a remote `torpedoArea` net hit) sets the `triggered` flag instead of
//!     bursting inline — that path has no paint / actor context; the burst
//!     lands at the head of the next `tick`, before the torpedo substeps
//!     (same frame for the bomb case, one frame late for the net case).
//!     The in-sim paths (`lethalContact`, a kit blast's `damageArea`) burst
//!     inline exactly like JS.
//!   - The feet-paint block filter (`b.roof || b.rail || b.perch`, L250)
//!     is dropped with the batch-one placement precedent (the sim raycast
//!     already excludes those surfaces).
//!   - `hitBase` / `chest` fold `e.smoothY` in JS; the sim uses the raw
//!     `e.pos` like the batch-one `_fly` capsule probes (sub.rs L1145-1149).

use glam::{Vec3, vec3};

use crate::actor::Form;
use crate::paint::SplatOpts;
use crate::weapon::{Bomb, SimEvent, segment_capsule_dist, throw_velocity};

use super::{Ctx, DOWN, GRAV, TAU, clamp, lerp, r2};

/// JS `HIT_R` (L36): the shot-down / contact radius of the torpedo (m).
const HIT_R: f32 = 0.3;
/// JS `MAX_DROPS` (L220): the module-wide droplet cap.
const MAX_DROPS: usize = 96;

/// JS `it.state` (the `tick` switch, torpedo.js L318-321).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TorpedoState {
    /// `fly` — the throw's gravity arc.
    Fly,
    /// `unfold` — locked, hovering while the fins swing out.
    Unfold,
    /// `launch` — swimming at the foe.
    Launch,
    /// `dead` — removed at the end of this update pass.
    Dead,
}

impl TorpedoState {
    /// JS `live(t)` (L242).
    #[must_use]
    pub fn live(self) -> bool {
        self != Self::Dead
    }
}

/// JS `t.net` (L297): the owner's last swim report for a ghost.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct TorpedoNet {
    /// JS `N.p` — the reported position.
    pub p: Vec3,
    /// JS `N.d` — the reported heading (unit).
    pub d: Vec3,
    /// JS `N.s` — the reported speed.
    pub s: f32,
    /// JS `N.age` — dead-reckoning time since the report.
    pub age: f32,
}

/// JS `t` (the `spawn` object, L264-274) minus the mesh / ring / audio /
/// presentation fields (`sub` reads the tuning table, `sp` is D-T34-3,
/// `flash` / `glowI` / `fin` / `propOpen` / `propA` / `roll` / `ringT` are
/// visual, `thr` is the bot layer).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TorpedoItem {
    /// JS `it.owner` — actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`).
    pub owner_slot: usize,
    /// The owner's team at spawn (JS `a.team`; `it.team` mirrors it).
    pub owner_team: usize,
    /// JS `it.team`.
    pub team: usize,
    /// JS `it.pos`.
    pub pos: Vec3,
    /// JS `it.prev` — the swept-step start for the capsule probes.
    pub prev: Vec3,
    /// JS `it.vel` — the flight velocity (the `fly` arc).
    pub vel: Vec3,
    /// JS `it.dir` — the swim heading (unit).
    pub dir: Vec3,
    /// JS `it.speed`.
    pub speed: f32,
    /// JS `it.state`.
    pub state: TorpedoState,
    /// JS `it.t` — the per-state timer.
    pub t: f32,
    /// JS `it.age` — the total life.
    pub age: f32,
    /// JS `it.hp` — the shoot-down health (`sub.hp`).
    pub hp: f32,
    /// JS `it.target` — the locked foe's actor-list index.
    pub target: Option<usize>,
    /// JS `it.hover` — the spot the lock stopped it at (the unfold bob).
    pub hover: Vec3,
    /// JS `it.hitN` — the surface normal it touched (the burst LOS origin).
    pub hit_n: Option<Vec3>,
    /// JS `it.hitFoe` — the foe whose body triggered the burst (damage 0
    /// distance, LOS-exempt).
    pub hit_foe: Option<usize>,
    /// JS `it.ghost`.
    pub ghost: bool,
    /// JS `it.gid` (D-T34-5: ghosts keep the wire form, keyed with
    /// `src_nid`).
    pub gid: u32,
    /// The remote owner's `nid` for a ghost (D-T34-5).
    pub src_nid: Option<u8>,
    /// JS `it.net` — the ghost's dead-reckoned swim report.
    pub net: Option<TorpedoNet>,
    /// JS `it.sendT` — the owner's next swim-report countdown.
    pub send_t: f32,
    /// JS `it.burst` — ended with a blast (suppresses the end record).
    pub burst: bool,
    /// JS `it.shot` — shot down (the end record's `shot` flag).
    pub shot: bool,
    /// Set by the public `damage_area` fan when a lethal blast catches a
    /// locked torpedo; [`tick`] bursts it at the head of the next pass (see
    /// the module notes).
    pub triggered: bool,
}

impl TorpedoItem {
    /// JS `live(t)` (L242).
    #[must_use]
    pub fn live(&self) -> bool {
        self.state.live()
    }

    /// JS `locked(t)` (L241).
    #[must_use]
    pub fn locked(&self) -> bool {
        matches!(self.state, TorpedoState::Unfold | TorpedoState::Launch)
    }
}

/// JS `drops[k]` (the `spawnDrops` object, L545-549) minus the instanced
/// mesh fields; `rec` rides [`Ctx::drop_recs`] by `rec_idx`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TorpedoDrop {
    /// JS `d.pos`.
    pub pos: Vec3,
    /// JS `d.prev`.
    pub prev: Vec3,
    /// JS `d.vel`.
    pub vel: Vec3,
    /// JS `d.age`.
    pub age: f32,
    /// JS `d.owner` — actor-list index.
    pub owner_idx: usize,
    /// The owner's identity slot (JS `a.slot`).
    pub owner_slot: usize,
    /// JS `d.team`.
    pub team: usize,
    /// JS `d.rec` — the burst group's shared per-foe hit counter.
    pub rec_idx: usize,
    /// JS `d.cap` (`sub.dropHits`).
    pub cap: u32,
    /// JS `d.paint` (`sub.dropPaint`).
    pub paint: f32,
    /// JS `d.dmg` (`sub.dropDamage`).
    pub dmg: f32,
    /// JS `d.size`.
    pub size: f32,
    /// JS `d.ghost` — the owner's ghost drops paint / hit nothing.
    pub ghost: bool,
}

/// JS `use(subs, a, sub)` (L244-258): the throw plus the feet patch.
pub(crate) fn use_torpedo(ctx: &mut Ctx, actor_idx: usize) {
    let a = &ctx.actors[actor_idx];
    let (slot, team) = (a.slot, a.team);
    let speed = ctx.sb.torpedo.throw_speed;
    let feet_paint = ctx.sb.torpedo.feet_paint;
    // JS L245-246: released 1.35 m up on the shared `throwVelocity` arc.
    let pos = vec3(a.pos.x, a.pos.y + 1.35, a.pos.z);
    let vel = throw_velocity(a, speed);
    let gid = ctx.gid_next(slot);
    spawn(ctx, actor_idx, slot, team, pos, vel, false, gid, None);
    // JS L247: `netRec(a, 'torpedo', [0, gid, pos×3, vel×3])`.
    ctx.events.push(SimEvent::SubSpawn {
        owner_slot: slot,
        team,
        sub_kind: "torpedo".to_string(),
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
        kind: super::SubKind::Torpedo,
    });
    // JS L249-250: the throw inks a patch under the thrower's feet.
    let a_pos = ctx.actors[actor_idx].pos;
    let g = ctx
        .world
        .raycast(vec3(a_pos.x, a_pos.y + 0.4, a_pos.z), DOWN, 2.5, false);
    if g.hit {
        let seed = ctx.rng.next_f32();
        let area = ctx.paint.splat(
            ctx.world,
            g.point + g.normal * 0.1,
            feet_paint,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        ctx.events.push(SimEvent::Turf {
            owner_slot: slot,
            team,
            area,
        });
    }
}

/// JS `spawn(subs, a, sub, pos, vel, ghost, gid)` (L261-277) minus the rig
/// / audio / `roll` (presentation; its draw is not consumed).
#[allow(clippy::too_many_arguments)]
fn spawn(
    ctx: &mut Ctx,
    owner_idx: usize,
    owner_slot: usize,
    owner_team: usize,
    pos: Vec3,
    vel: Vec3,
    ghost: bool,
    gid: u32,
    src_nid: Option<u8>,
) {
    let hp = ctx.sb.torpedo.hp as f32;
    let dir = vel.normalize_or_zero();
    ctx.torpedo.push(TorpedoItem {
        owner_idx,
        owner_slot,
        owner_team,
        team: owner_team,
        pos,
        prev: pos,
        vel,
        dir,
        speed: 0.0,
        state: TorpedoState::Fly,
        t: 0.0,
        age: 0.0,
        hp,
        target: None,
        hover: Vec3::ZERO,
        hit_n: None,
        hit_foe: None,
        ghost,
        gid,
        src_nid,
        net: None,
        send_t: 0.0,
        burst: false,
        shot: false,
        triggered: false,
    });
}

/// JS `ghost(a, d)` (L279-305): a remote throw / lock / swim / burst / end
/// record. The netmatch replay wraps `'k'` records in `mute++`
/// (netmatch.js L490-491), so the burst op runs muted.
pub(crate) fn ghost(ctx: &mut Ctx, actor_idx: usize, nid: u8, data: &[f32], nids: &[u8]) {
    let (op, gid) = (data[0], data[1] as u32);
    if op == 0.0 {
        // JS L282 dedups on the globally-unique `gid`; the sim keys ghosts
        // on `(src_nid, gid)` (D-T34-5).
        if ctx
            .torpedo
            .iter()
            .any(|x| x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        if data.len() < 8 {
            return;
        }
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        let pos = vec3(data[2], data[3], data[4]);
        let vel = vec3(data[5], data[6], data[7]);
        spawn(ctx, actor_idx, slot, team, pos, vel, true, gid, Some(nid));
        return;
    }
    let idx = ctx
        .torpedo
        .iter()
        .position(|x| x.ghost && x.src_nid == Some(nid) && x.gid == gid && x.live());
    let Some(i) = idx else { return };
    match op {
        3.0 => {
            if data.len() < 6 {
                return;
            }
            ctx.torpedo[i].pos = vec3(data[3], data[4], data[5]);
            if ctx.torpedo[i].state == TorpedoState::Fly {
                // JS L291: `startLock(t, byNid(d[2]))` — the record carries
                // the wire nid; translate it to an actor index (waddle
                // pattern).
                let tgt = nid_to_idx(nids, data[2] as i32);
                start_lock(ctx, i, tgt);
            }
        }
        4.0 => {
            if data.len() < 9 {
                return;
            }
            if ctx.torpedo[i].state == TorpedoState::Fly {
                ctx.torpedo[i].pos = vec3(data[2], data[3], data[4]);
                start_lock(ctx, i, None);
            }
            if ctx.torpedo[i].state == TorpedoState::Unfold {
                ctx.torpedo[i].state = TorpedoState::Launch;
                ctx.torpedo[i].t = 0.0;
            }
            ctx.torpedo[i].speed = data[8];
            ctx.torpedo[i].net = Some(TorpedoNet {
                p: vec3(data[2], data[3], data[4]),
                d: vec3(data[5], data[6], data[7]).normalize_or_zero(),
                s: data[8],
                age: 0.0,
            });
        }
        1.0 => {
            if data.len() < 6 {
                return;
            }
            ctx.torpedo[i].pos = vec3(data[2], data[3], data[4]);
            burst(ctx, i, data[5] != 0.0, true);
        }
        2.0 => {
            // JS L297: `d[2] ? destroy(t) : fizzle(t)` — both end it; the
            // `shot` flag only feeds the owner's own end record.
            if data.len() < 3 {
                return;
            }
            if data[2] != 0.0 {
                destroy(ctx, i);
            } else {
                fizzle(ctx, i);
            }
        }
        _ => {}
    }
}

/// JS `byNid(n)` (L278) for a ghost record: `-1` is "no target".
fn nid_to_idx(nids: &[u8], nid: i32) -> Option<usize> {
    if nid < 0 {
        return None;
    }
    nids.iter().position(|&n| n as i32 == nid)
}

/// JS `startLock(t, e)` (L374-384): the `[3, …]` record, the hover stop.
fn start_lock(ctx: &mut Ctx, i: usize, tgt: Option<usize>) {
    if !ctx.torpedo[i].ghost {
        let (slot, team, gid) = (
            ctx.torpedo[i].owner_slot,
            ctx.torpedo[i].team,
            ctx.torpedo[i].gid,
        );
        let p = ctx.torpedo[i].pos;
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "torpedo".to_string(),
            data: vec![
                3.0,
                gid as f32,
                tgt.map_or(-1.0, |ei| ei as f32),
                r2(p.x),
                r2(p.y),
                r2(p.z),
            ],
        });
    }
    ctx.torpedo[i].state = TorpedoState::Unfold;
    ctx.torpedo[i].t = 0.0;
    ctx.torpedo[i].target = tgt;
    ctx.torpedo[i].vel = Vec3::ZERO;
    ctx.torpedo[i].hover = ctx.torpedo[i].pos;
}

/// JS `tick(dt)` (L313-335): the reverse walk with the substepped flight,
/// the lethal-contact probe, the owner's 10 Hz swim report, the ghost
/// age-out and the dead splice (`[2, gid, shot]` for a local torpedo that
/// ended without bursting).
pub(crate) fn tick(ctx: &mut Ctx, dt: f32) {
    let mut i = ctx.torpedo.len();
    while i > 0 {
        i -= 1;
        if ctx.torpedo[i].live() && ctx.torpedo[i].triggered {
            // A lethal blast caught it on the public fan (a splat-bomb
            // blast this frame, or a remote `torpedoArea` hit): burst now,
            // before it substeps (see the module notes).
            ctx.torpedo[i].triggered = false;
            burst(ctx, i, true, false);
        }
        if ctx.torpedo[i].live() {
            ctx.torpedo[i].age += dt;
            ctx.torpedo[i].t += dt;
            // JS L317: substep the flight at ~60 Hz.
            let n = (dt / (1.0 / 60.0) - 1e-6).ceil().max(1.0) as usize;
            let h = dt / n as f32;
            let muted = ctx.torpedo[i].ghost;
            for _ in 0..n {
                if !ctx.torpedo[i].live() {
                    break;
                }
                match ctx.torpedo[i].state {
                    TorpedoState::Fly => fly(ctx, i, h),
                    TorpedoState::Unfold => unfold(ctx, i, h),
                    TorpedoState::Launch => launch(ctx, i, h, muted),
                    TorpedoState::Dead => break,
                }
            }
            if ctx.torpedo[i].live() && ctx.torpedo[i].locked() {
                lethal_contact(ctx, i);
            }
            if !ctx.torpedo[i].ghost
                && ctx.torpedo[i].live()
                && ctx.torpedo[i].state == TorpedoState::Launch
            {
                // JS L325-328: the owner's swim report, 10 a second.
                ctx.torpedo[i].send_t -= dt;
                if ctx.torpedo[i].send_t <= 0.0 {
                    ctx.torpedo[i].send_t = 0.1;
                    let (slot, team, gid) = (
                        ctx.torpedo[i].owner_slot,
                        ctx.torpedo[i].team,
                        ctx.torpedo[i].gid,
                    );
                    let p = ctx.torpedo[i].pos;
                    let d = ctx.torpedo[i].dir;
                    let sp = ctx.torpedo[i].speed;
                    ctx.events.push(SimEvent::SubSpawn {
                        owner_slot: slot,
                        team,
                        sub_kind: "torpedo".to_string(),
                        data: vec![
                            4.0,
                            gid as f32,
                            r2(p.x),
                            r2(p.y),
                            r2(p.z),
                            r2(d.x),
                            r2(d.y),
                            r2(d.z),
                            r2(sp),
                        ],
                    });
                }
            }
            if ctx.torpedo[i].ghost && ctx.torpedo[i].age > 15.0 {
                // its owner left (JS L330)
                ctx.torpedo[i].state = TorpedoState::Dead;
            }
        }
        if !ctx.torpedo[i].live() {
            if !ctx.torpedo[i].ghost && !ctx.torpedo[i].burst {
                let (slot, team, gid) = (
                    ctx.torpedo[i].owner_slot,
                    ctx.torpedo[i].team,
                    ctx.torpedo[i].gid,
                );
                let shot = if ctx.torpedo[i].shot { 1.0 } else { 0.0 };
                ctx.events.push(SimEvent::SubEnd {
                    owner_slot: slot,
                    team,
                    sub_kind: Some("torpedo".to_string()),
                    data: vec![2.0, gid as f32, shot],
                });
            }
            ctx.torpedo.remove(i);
        }
    }
    tick_drops(ctx, dt);
}

// ---- in flight: gravity arc; locks onto a foe in range, bursts small on
// touching anything first (JS `fly`, L337-359).
fn fly(ctx: &mut Ctx, i: usize, h: f32) {
    let s = &ctx.sb.torpedo;
    let (arm_time, lock_range, fly_life) = (s.arm_time, s.lock_range, s.fly_life);
    let water = ctx.t.water_y;
    ctx.torpedo[i].prev = ctx.torpedo[i].pos;
    ctx.torpedo[i].vel.y -= GRAV * h;
    let vel = ctx.torpedo[i].vel;
    ctx.torpedo[i].pos += vel * h;
    if ctx.torpedo[i].vel.length_squared() > 1e-4 {
        ctx.torpedo[i].dir = ctx.torpedo[i].vel.normalize();
    }
    if ctx.torpedo[i].ghost {
        // JS L344-349: flies the arc, stops where it touches and waits for
        // its owner's word.
        let prev = ctx.torpedo[i].prev;
        let pos = ctx.torpedo[i].pos;
        let w = ctx.world.segment(prev, pos, false);
        if w.hit {
            ctx.torpedo[i].pos = w.point + w.normal * 0.14;
            ctx.torpedo[i].vel = Vec3::ZERO;
        }
        if ctx.torpedo[i].pos.y < water - 1.8 {
            fizzle(ctx, i);
        }
        return;
    }
    if touch_foe(ctx, i) {
        burst(ctx, i, false, false);
        return;
    }
    if touch_device(ctx, i) {
        burst(ctx, i, false, false);
        return;
    }
    let prev = ctx.torpedo[i].prev;
    let pos = ctx.torpedo[i].pos;
    let w = ctx.world.segment(prev, pos, false);
    if w.hit {
        ctx.torpedo[i].pos = w.point + w.normal * 0.14;
        ctx.torpedo[i].hit_n = Some(w.normal);
        burst(ctx, i, false, false);
        return;
    }
    if ctx.torpedo[i].pos.y < water - 1.8 {
        fizzle(ctx, i);
        return;
    }
    if ctx.torpedo[i].age >= arm_time {
        let p = ctx.torpedo[i].pos;
        if let Some(tg) = find_target(ctx, i, p, lock_range) {
            start_lock(ctx, i, Some(tg));
            return;
        }
    }
    if ctx.torpedo[i].age > fly_life {
        burst(ctx, i, false, false);
    }
}

/// JS `findTarget(t, range)` (L361-373): the nearest foe in sight. A squid
/// hidden in its own ink stays hidden unless its team is tracked.
fn find_target(ctx: &Ctx, i: usize, from: Vec3, range: f32) -> Option<usize> {
    let team = ctx.torpedo[i].team;
    let mut best: Option<usize> = None;
    let mut bd = range;
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        if e.form == Form::Squid
            && e.submerged
            && !(e.status.track > 0.0 && e.status.track_team == Some(team))
        {
            continue;
        }
        let c = chest(e.pos, e.form);
        let d = c.distance(from);
        if d >= bd || !ctx.world.los(from, c) {
            continue;
        }
        bd = d;
        best = Some(ei);
    }
    best
}

/// JS `chest(e, out)` (L40): the aim point on a foe's body.
fn chest(pos: Vec3, form: Form) -> Vec3 {
    vec3(
        pos.x,
        pos.y + if form == Form::Squid { 0.3 } else { 0.8 },
        pos.z,
    )
}

// ---- locked: hovers, the unfold bob, turns to face the foe (JS `unfold`,
// L386-401).
fn unfold(ctx: &mut Ctx, i: usize, h: f32) {
    let s = &ctx.sb.torpedo;
    let (lock_range, unfold_time, launch_speed0) = (s.lock_range, s.unfold_time, s.launch_speed0);
    if !ctx.torpedo[i].ghost {
        let tgt_alive = ctx.torpedo[i]
            .target
            .is_some_and(|ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
        if !tgt_alive {
            let p = ctx.torpedo[i].pos;
            let team_pos = p;
            let nt = find_target(ctx, i, team_pos, lock_range * 1.4);
            if nt.is_some() {
                ctx.torpedo[i].target = nt;
            }
        }
    }
    let k = clamp(ctx.torpedo[i].t / unfold_time, 0.0, 1.0);
    let tgt = ctx.torpedo[i]
        .target
        .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
    if let Some(ei) = tgt {
        let e_pos = ctx.actors[ei].pos;
        let e_form = ctx.actors[ei].form;
        let v = (chest(e_pos, e_form) - ctx.torpedo[i].pos).normalize_or_zero();
        let dir = ctx.torpedo[i].dir;
        ctx.torpedo[i].dir = dir.lerp(v, 1.0 - (-9.0 * h).exp()).normalize_or_zero();
    }
    // JS L395: the hover bob — the two shake draws are consumed even at
    // zero amplitude (JS evaluates `Math.random()` before multiplying).
    let shake = if k > 0.6 { (k - 0.6) * 0.03 } else { 0.0 };
    let r1 = ctx.rng.next_f32();
    let r2v = ctx.rng.next_f32();
    let hover = ctx.torpedo[i].hover;
    ctx.torpedo[i].pos = vec3(
        hover.x + (r1 - 0.5) * shake,
        hover.y + (ctx.torpedo[i].t * 13.0).sin() * 0.025 + 0.06 * back_out(k, 1.5) * 0.4,
        hover.z + (r2v - 0.5) * shake,
    );
    if ctx.torpedo[i].t >= unfold_time {
        ctx.torpedo[i].state = TorpedoState::Launch;
        ctx.torpedo[i].t = 0.0;
        ctx.torpedo[i].speed = launch_speed0;
    }
}

/// JS `backOut` (L39).
fn back_out(t: f32, s: f32) -> f32 {
    let t = clamp(t, 0.0, 1.0) - 1.0;
    1.0 + t * t * ((s + 1.0) * t + s)
}

// ---- launched: swims at the foe, speeding up, homing gently; bursts on a
// foe, a wall or the floor (JS `launch`, L403-426).
fn launch(ctx: &mut Ctx, i: usize, h: f32, muted: bool) {
    if ctx.torpedo[i].ghost {
        swim(ctx, i, h);
        return;
    }
    let s = &ctx.sb.torpedo;
    let (launch_speed, launch_accel, turn_rate, launch_life) =
        (s.launch_speed, s.launch_accel, s.turn_rate, s.launch_life);
    let water = ctx.t.water_y;
    ctx.torpedo[i].speed = launch_speed.min(ctx.torpedo[i].speed + launch_accel * h);
    let tgt = ctx.torpedo[i]
        .target
        .filter(|&ei| ctx.actors.get(ei).is_some_and(|e| e.alive));
    if let Some(ei) = tgt {
        let e_pos = ctx.actors[ei].pos;
        let e_form = ctx.actors[ei].form;
        let mut v = chest(e_pos, e_form) - ctx.torpedo[i].pos;
        if v.length_squared() > 1e-4 {
            v = v.normalize();
            let dir = ctx.torpedo[i].dir;
            let ang = (dir.dot(v)).clamp(-1.0, 1.0).acos();
            let max = turn_rate * h;
            if ang > 1e-4 {
                let mut axis = dir.cross(v);
                if axis.length_squared() < 1e-8 {
                    axis = vec3(0.0, 1.0, 0.0);
                } else {
                    axis = axis.normalize();
                }
                let ang = ang.min(max);
                // Rodrigues rotation of `dir` about `axis` by `ang`.
                let d = ctx.torpedo[i].dir;
                let rot = d * ang.cos()
                    + axis.cross(d) * ang.sin()
                    + axis * axis.dot(d) * (1.0 - ang.cos());
                ctx.torpedo[i].dir = rot.normalize_or_zero();
            }
        }
    }
    ctx.torpedo[i].prev = ctx.torpedo[i].pos;
    let dir = ctx.torpedo[i].dir;
    let sp = ctx.torpedo[i].speed;
    ctx.torpedo[i].pos += dir * (sp * h);
    if touch_foe(ctx, i) {
        burst(ctx, i, true, false);
        return;
    }
    if touch_device(ctx, i) {
        burst(ctx, i, true, false);
        return;
    }
    let prev = ctx.torpedo[i].prev;
    let pos = ctx.torpedo[i].pos;
    let w = ctx.world.segment(prev, pos, false);
    if w.hit {
        ctx.torpedo[i].pos = w.point + w.normal * 0.16;
        ctx.torpedo[i].hit_n = Some(w.normal);
        burst(ctx, i, true, false);
        return;
    }
    if ctx.torpedo[i].pos.y < water - 0.2 {
        fizzle(ctx, i);
        return;
    }
    if ctx.torpedo[i].t > launch_life {
        burst(ctx, i, true, muted);
    }
}

/// JS `swim(t, h)` (L428-438): a ghost's dead-reckoned swim, eased onto its
/// owner's last report.
fn swim(ctx: &mut Ctx, i: usize, h: f32) {
    ctx.torpedo[i].prev = ctx.torpedo[i].pos;
    let Some(n) = ctx.torpedo[i].net else {
        // JS L430: no report yet — accelerate on the owner's tuning.
        let s = &ctx.sb.torpedo;
        let (launch_speed, launch_accel) = (s.launch_speed, s.launch_accel);
        ctx.torpedo[i].speed = launch_speed.min(ctx.torpedo[i].speed + launch_accel * h);
        let dir = ctx.torpedo[i].dir;
        let sp = ctx.torpedo[i].speed;
        ctx.torpedo[i].pos += dir * sp * h;
        return;
    };
    ctx.torpedo[i].net.as_mut().unwrap().age += h;
    let est = n.p + n.d * (n.s * n.age);
    let dir = ctx.torpedo[i].dir;
    let sp = ctx.torpedo[i].speed;
    let p = ctx.torpedo[i].pos + dir * (sp * h);
    ctx.torpedo[i].pos = p.lerp(est, 1.0 - (-18.0 * h).exp());
    ctx.torpedo[i].dir = dir.lerp(n.d, 1.0 - (-12.0 * h).exp()).normalize_or_zero();
    ctx.torpedo[i].speed = n.s;
}

/// JS `touchFoe(t)` (L440-449): any foe's body along this step.
fn touch_foe(ctx: &mut Ctx, i: usize) -> bool {
    let team = ctx.torpedo[i].team;
    let prev = ctx.torpedo[i].prev;
    let pos = ctx.torpedo[i].pos;
    let radius = ctx.t.radius;
    let mut found: Option<(usize, f32)> = None;
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        if (e.pos.x - pos.x).abs() > 3.0 || (e.pos.z - pos.z).abs() > 3.0 {
            continue;
        }
        let h = if e.form == Form::Squid {
            ctx.t.squid_height
        } else {
            ctx.t.height
        };
        let res = segment_capsule_dist(prev, pos, e.pos, radius, h);
        if res.dist < radius + 0.16 {
            found = Some((ei, res.t));
            break;
        }
    }
    if let Some((ei, t)) = found {
        let p = ctx.torpedo[i].pos;
        ctx.torpedo[i].pos = ctx.torpedo[i].prev.lerp(p, t);
        ctx.torpedo[i].hit_foe = Some(ei);
        return true;
    }
    false
}

/// JS `touchDevice(t)` (L451-456): enemy curtains / sprinklers / beacons
/// and the kit devices count as surfaces (the probe does no damage).
/// `G.specials?.shotHit` is Task 35.
fn touch_device(ctx: &mut Ctx, i: usize) -> bool {
    let prev = ctx.torpedo[i].prev;
    let pos = ctx.torpedo[i].pos;
    let team = ctx.torpedo[i].team;
    ctx.probe_block_shot(prev, pos, team, 0.0, false)
}

/// JS `lethalContact(t)` (L458-476): a locked torpedo treats lethal bombs
/// like a Drip Curtain — an enemy splat bomb that touches it goes off right
/// there (its blast then sets the torpedo off too), and an enemy sticky /
/// skitter charge that touches it detonates on the spot. Only this client's
/// own live devices: a remote player's are set off on its owner's screen.
fn lethal_contact(ctx: &mut Ctx, i: usize) {
    let team = ctx.torpedo[i].team;
    let pos = ctx.torpedo[i].pos;
    // ---- enemy splat bombs (JS L462-467)
    let bomb_hit = ctx.projectiles.bombs.iter().position(|b| {
        b.kind == "bomb" && !b.ghost && b.team != team && b.pos.distance(pos) <= 0.62
    });
    if let Some(bi) = bomb_hit {
        let b: Bomb = ctx.projectiles.bombs[bi].clone();
        ctx.projectiles.bombs.swap_remove(bi);
        explode_bomb_fan(ctx, &b);
        if ctx.torpedo[i].triggered {
            ctx.torpedo[i].triggered = false;
            burst(ctx, i, true, false);
        }
        if !ctx.torpedo[i].live() {
            return;
        }
    }
    // ---- enemy sticky / skitter charges in flight (JS L468-475)
    let mut j = 0;
    while j < ctx.items.len() {
        let it = &ctx.items[j];
        let hit = it.state == super::SubState::Fly
            && !it.ghost
            && it.team != team
            && matches!(it.kind, super::SubKind::Sticky | super::SubKind::Seeker)
            && it.pos.distance(pos) <= 0.62;
        if !hit {
            j += 1;
            continue;
        }
        let (r, dmax, dmin, pr) = super::blast_params(it.kind, ctx.sb);
        let p = ctx.items[j].pos;
        ctx.blast(j, p, r, dmax, dmin, pr, super::UP, false);
        ctx.items[j].state = super::SubState::Dead;
        if !ctx.torpedo[i].live() {
            return;
        }
        // JS `for (const it of S.items)` keeps walking the (unchanged)
        // list; the dead item stays until the update splice.
        j += 1;
    }
}

/// Run the splat bomb's full detonation with the sub-device fan-out wired
/// back into this frame's kit state (JS `P._explodeBomb` L1488's
/// `G.subs.damageArea` call). The fan marks a caught locked torpedo rather
/// than bursting it inline (no paint / actor context on that path);
/// [`lethal_contact`] bursts it right after.
fn explode_bomb_fan(ctx: &mut Ctx, b: &Bomb) {
    let Ctx {
        items,
        events,
        waddle,
        torpedo,
        tracer,
        boomerang,
        ..
    } = ctx;
    let mut fan = |c: Vec3, radius: f32, dmg: f32, team: usize, muted: bool| {
        super::damage_area_fan_plain(
            items, events, waddle, torpedo, tracer, boomerang, c, radius, dmg, team, muted,
        );
    };
    ctx.projectiles.explode_bomb(
        b, ctx.world, ctx.actors, ctx.paint, ctx.t, ctx.sb, &mut fan, ctx.remote,
    );
}

/// JS `burst(t, full)` (L478-513): the `[1, gid, pos, full]` record, the
/// ground-anchored splash (wide + five satellites when full, a small one
/// otherwise), the turf credit, the core-rim damage with LOS,
/// `damageArea(60)` and the droplet spray. The `explosion` / audio /
/// `shake` / `bomb:explode` / rumble / boss lines are presentation or out
/// of scope.
fn burst(ctx: &mut Ctx, i: usize, full: bool, muted: bool) {
    if !ctx.torpedo[i].live() {
        return;
    }
    if !ctx.torpedo[i].ghost {
        let (slot, team, gid) = (
            ctx.torpedo[i].owner_slot,
            ctx.torpedo[i].team,
            ctx.torpedo[i].gid,
        );
        let p = ctx.torpedo[i].pos;
        ctx.events.push(SimEvent::SubSpawn {
            owner_slot: slot,
            team,
            sub_kind: "torpedo".to_string(),
            data: vec![
                1.0,
                gid as f32,
                r2(p.x),
                r2(p.y),
                r2(p.z),
                if full { 1.0 } else { 0.0 },
            ],
        });
    }
    ctx.torpedo[i].state = TorpedoState::Dead;
    ctx.torpedo[i].burst = true;
    let s = &ctx.sb.torpedo;
    let (radius, core_radius, dmax, dmin, paint_radius, fallback_paint, drops) = (
        s.radius,
        s.core_radius,
        s.damage_max,
        s.damage_min,
        s.paint_radius,
        s.fallback_paint,
        s.drops,
    );
    let c = ctx.torpedo[i].pos;
    // JS L485-486: paint on the ground under a mid-air burst, else where
    // it touched.
    let g = ctx
        .world
        .raycast(vec3(c.x, c.y + 0.2, c.z), DOWN, 2.6, false);
    let pc = if g.hit { g.point + g.normal * 0.2 } else { c };
    let team = ctx.torpedo[i].team;
    let mut area = 0.0;
    if full {
        let seed = ctx.rng.next_f32();
        if !muted {
            area += ctx.paint.splat(
                ctx.world,
                pc,
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
        for _ in 0..5 {
            let ang = ctx.rng.next_f32() * TAU;
            let rr = paint_radius * (0.55 + ctx.rng.next_f32() * 0.45);
            let rad = 0.55 + ctx.rng.next_f32() * 0.45;
            let sd = ctx.rng.next_f32();
            if muted {
                continue;
            }
            let p = vec3(pc.x + ang.cos() * rr, pc.y + 0.3, pc.z + ang.sin() * rr);
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
    } else {
        let seed = ctx.rng.next_f32();
        if !muted {
            area += ctx.paint.splat(
                ctx.world,
                pc,
                fallback_paint,
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
    let (os, ot) = (ctx.torpedo[i].owner_slot, ctx.torpedo[i].owner_team);
    ctx.events.push(SimEvent::Turf {
        owner_slot: os,
        team: ot,
        area,
    });
    // ---- damage (JS L503-512): the foe it touched is a point-blank hit;
    // LOS from the touched surface (or straight up) otherwise.
    let hit_n = ctx.torpedo[i].hit_n.unwrap_or(super::UP);
    let los_from = c + hit_n * 0.25;
    let hit_foe = ctx.torpedo[i].hit_foe;
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in ctx.actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue;
        }
        let v = chest(e.pos, e.form);
        let d = if Some(ei) == hit_foe {
            0.0
        } else {
            v.distance(c)
        };
        if d > radius {
            continue;
        }
        if d > 0.0 && !ctx.world.los(los_from, v) {
            continue;
        }
        let dmg = if d <= core_radius {
            dmax
        } else {
            lerp(dmax, dmin, (d - core_radius) / (radius - core_radius))
        };
        hits.push((ei, dmg));
    }
    let (ghost, oi, os) = (
        ctx.torpedo[i].ghost,
        ctx.torpedo[i].owner_idx,
        ctx.torpedo[i].owner_slot,
    );
    for (ei, dmg) in hits {
        ctx.hit_routed(ghost, oi, os, team, ei, dmg);
    }
    ctx.damage_area(c, radius, 60.0, team, muted);
    if full {
        spawn_drops(ctx, i, c, drops);
    }
}

/// JS `destroy(t)` (L515-524): shot down — gone without a blast.
fn destroy(ctx: &mut Ctx, i: usize) {
    if !ctx.torpedo[i].live() {
        return;
    }
    ctx.torpedo[i].state = TorpedoState::Dead;
    ctx.torpedo[i].shot = true;
}

/// JS `fizzle(t)` (L525-528): sank / aged out — gone without a blast.
fn fizzle(ctx: &mut Ctx, i: usize) {
    if !ctx.torpedo[i].live() {
        return;
    }
    ctx.torpedo[i].state = TorpedoState::Dead;
}

/// JS `hurt(t, dmg)` (L529-536): enemy fire shoots it down. `muted`
/// mirrors the JS `netMuted()` guard; a ghost's hit rides
/// [`SimEvent::DeviceHit`] to its owner (JS `netHurt`, L532).
pub(crate) fn hurt(
    torpedo: &mut [TorpedoItem],
    events: &mut Vec<SimEvent>,
    i: usize,
    dmg: f32,
    muted: bool,
) {
    let it = &torpedo[i];
    // JS L530 `!(dmg > 0)` — NaN-safe negation.
    if !it.live() || dmg <= 0.0 || dmg.is_nan() || muted {
        return;
    }
    if it.ghost {
        events.push(SimEvent::DeviceHit {
            owner_slot: it.owner_slot,
            team: it.owner_team,
            gid: it.gid,
            kind: "torpedo".to_string(),
            d: r2(dmg),
        });
        return;
    }
    torpedo[i].hp -= dmg;
    if torpedo[i].hp <= 0.0 {
        torpedo[i].state = TorpedoState::Dead;
        torpedo[i].shot = true;
    }
}

// ---- droplets: fly out of a locked burst, ink where they land, 12 a hit,
// three hits per foe at most (JS `spawnDrops`, L539-551).
fn spawn_drops(ctx: &mut Ctx, i: usize, c: Vec3, count: u32) {
    let s = &ctx.sb.torpedo;
    let (drop_damage, drop_hits, drop_paint) = (s.drop_damage, s.drop_hits, s.drop_paint);
    let (owner_idx, owner_slot, team, ghost) = (
        ctx.torpedo[i].owner_idx,
        ctx.torpedo[i].owner_slot,
        ctx.torpedo[i].team,
        ctx.torpedo[i].ghost,
    );
    let rec_idx = ctx.drop_recs.len();
    ctx.drop_recs.push(Vec::new());
    let base = ctx.rng.next_f32() * TAU;
    let n = count as usize;
    for k in 0..n {
        if ctx.torpedo_drops.len() >= MAX_DROPS {
            break;
        }
        let a = base + (k as f32 / n as f32) * TAU + (ctx.rng.next_f32() - 0.5) * 0.4;
        let hs = 3.2 + ctx.rng.next_f32() * 3.4;
        let vy = 3.2 + ctx.rng.next_f32() * 3.2;
        let size = 0.075 + ctx.rng.next_f32() * 0.035;
        let p = vec3(c.x + a.cos() * 0.22, c.y + 0.15, c.z + a.sin() * 0.22);
        ctx.torpedo_drops.push(TorpedoDrop {
            pos: p,
            prev: p,
            vel: vec3(a.cos() * hs, vy, a.sin() * hs),
            age: 0.0,
            owner_idx,
            owner_slot,
            team,
            rec_idx,
            cap: drop_hits,
            paint: drop_paint,
            dmg: drop_damage,
            size,
            ghost,
        });
    }
}

/// JS `tickDrops(dt)` (L552-570): the reverse walk with the swap-with-last
/// removal (JS L556).
fn tick_drops(ctx: &mut Ctx, dt: f32) {
    let mut i = ctx.torpedo_drops.len();
    while i > 0 {
        i -= 1;
        let muted = ctx.torpedo_drops[i].ghost;
        let dead = drop_step(ctx, i, dt, muted);
        if dead {
            ctx.torpedo_drops.swap_remove(i);
        }
    }
}

/// JS `dropStep(d, dt)` (L572-609): one droplet's step — true when it's
/// done.
fn drop_step(ctx: &mut Ctx, j: usize, dt: f32, muted: bool) -> bool {
    let water = ctx.t.water_y;
    ctx.torpedo_drops[j].age += dt;
    ctx.torpedo_drops[j].prev = ctx.torpedo_drops[j].pos;
    ctx.torpedo_drops[j].vel.y -= 21.0 * dt;
    let vel = ctx.torpedo_drops[j].vel;
    ctx.torpedo_drops[j].pos += vel * dt;
    let mut dead = ctx.torpedo_drops[j].age > 2.4 || ctx.torpedo_drops[j].pos.y < water - 1.0;
    if !dead {
        let team = ctx.torpedo_drops[j].team;
        let prev = ctx.torpedo_drops[j].prev;
        let pos = ctx.torpedo_drops[j].pos;
        let size = ctx.torpedo_drops[j].size;
        let radius = ctx.t.radius;
        let mut found: Option<(usize, f32)> = None;
        for (ei, e) in ctx.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            if (e.pos.x - pos.x).abs() > 2.5 || (e.pos.z - pos.z).abs() > 2.5 {
                continue;
            }
            let h = if e.form == Form::Squid {
                ctx.t.squid_height
            } else {
                ctx.t.height
            };
            let res = segment_capsule_dist(prev, pos, e.pos, radius, h);
            if res.dist < radius + size {
                found = Some((ei, res.t));
                break;
            }
        }
        if let Some((ei, t)) = found {
            // JS L584-587: the shared burst-group cap (three hits per foe).
            let (rec_idx, cap, dmg, ghost, oi, os, team) = (
                ctx.torpedo_drops[j].rec_idx,
                ctx.torpedo_drops[j].cap,
                ctx.torpedo_drops[j].dmg,
                ctx.torpedo_drops[j].ghost,
                ctx.torpedo_drops[j].owner_idx,
                ctx.torpedo_drops[j].owner_slot,
                ctx.torpedo_drops[j].team,
            );
            let rec = &mut ctx.drop_recs[rec_idx];
            let k = rec.iter().position(|(x, _)| *x == ei).map(|p| rec[p].1);
            let k = k.unwrap_or(0);
            if k < cap {
                if let Some(p) = rec.iter_mut().find(|(x, _)| *x == ei) {
                    p.1 += 1;
                } else {
                    rec.push((ei, 1));
                }
                ctx.hit_routed(ghost, oi, os, team, ei, dmg);
            }
            let pos = ctx.torpedo_drops[j].pos;
            ctx.torpedo_drops[j].pos = ctx.torpedo_drops[j].prev.lerp(pos, t);
            dead = true;
        }
    }
    if !dead {
        let prev = ctx.torpedo_drops[j].prev;
        let pos = ctx.torpedo_drops[j].pos;
        let team = ctx.torpedo_drops[j].team;
        let dmg = ctx.torpedo_drops[j].dmg;
        if ctx.probe_block_shot(prev, pos, team, dmg, muted) {
            dead = true;
        }
    }
    if !dead {
        let prev = ctx.torpedo_drops[j].prev;
        let pos = ctx.torpedo_drops[j].pos;
        // JS L594: `G.physics.segment(d.prev, d.pos, _hit, true)` — grates
        // count for droplets too.
        let w = ctx.world.segment(prev, pos, true);
        if w.hit {
            let (paint, team, ghost, os, ot) = (
                ctx.torpedo_drops[j].paint,
                ctx.torpedo_drops[j].team,
                ctx.torpedo_drops[j].ghost,
                ctx.torpedo_drops[j].owner_slot,
                ctx.torpedo_drops[j].team,
            );
            let vel = ctx.torpedo_drops[j].vel;
            // JS L596: radius draw then seed draw (positional args first).
            let rad = paint * (0.8 + ctx.rng.next_f32() * 0.4);
            let seed = ctx.rng.next_f32();
            let area = if muted {
                0.0
            } else {
                ctx.paint.splat(
                    ctx.world,
                    w.point + w.normal * 0.1,
                    rad,
                    team,
                    &SplatOpts {
                        seed,
                        stretch: Some(vel.normalize_or_zero()),
                        stretch_amt: Some(0.5),
                        kind: None,
                    },
                )
            };
            if area > 0.0 && !ghost {
                ctx.events.push(SimEvent::Turf {
                    owner_slot: os,
                    team: ot,
                    area,
                });
            }
            dead = true;
        }
    }
    dead
}

/// JS `closestOnSeg` (L661-665) + `blockShot` (L666-675): enemy shots shoot
/// it down (in any live state, the shot is absorbed).
pub(crate) fn block_shot(
    torpedo: &mut [TorpedoItem],
    events: &mut Vec<SimEvent>,
    prev: Vec3,
    pos: Vec3,
    team: usize,
    dmg: f32,
    muted: bool,
) -> bool {
    for (i, it) in torpedo.iter().enumerate() {
        if !it.live() || it.team == team {
            continue;
        }
        if (it.pos.x - pos.x).abs() > 4.0 || (it.pos.z - pos.z).abs() > 4.0 {
            continue;
        }
        if closest_on_seg(prev, pos, it.pos).distance(it.pos) > HIT_R {
            continue;
        }
        hurt(torpedo, events, i, dmg, muted);
        return true;
    }
    false
}

/// JS `closestOnSeg(a, b, p)` (L661-665).
fn closest_on_seg(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let v = b - a;
    let l2 = v.length_squared();
    let k = if l2 > 1e-9 {
        clamp((p - a).dot(v) / l2, 0.0, 1.0)
    } else {
        0.0
    };
    a + v * k
}

/// JS `blockRay` (L676-690): a charger beam shoots down the nearest enemy
/// torpedo on its path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn block_ray(
    torpedo: &mut [TorpedoItem],
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
    for (i, it) in torpedo.iter().enumerate() {
        if !it.live() || it.team == team {
            continue;
        }
        let v = it.pos - from;
        let tc = v.dot(dir);
        if tc <= 0.0 || tc > best + HIT_R {
            continue;
        }
        let d2 = v.length_squared() - tc * tc;
        if d2 > HIT_R * HIT_R {
            continue;
        }
        let tt = (tc - (HIT_R * HIT_R - d2).sqrt()).max(0.05);
        if tt < best {
            best = tt;
            hit = Some(i);
        }
    }
    if let Some(i) = hit {
        hurt(torpedo, events, i, dmg, muted);
    }
    best
}

/// JS `damageArea` (L692-700) + `areaHit` (L701-705) from the in-sim blast
/// context: locked torpedoes caught by a lethal blast go off at once (the
/// Drip Curtain rule), anything else takes the damage.
pub(crate) fn damage_area_ctx(ctx: &mut Ctx, c: Vec3, radius: f32, dmg: f32, team: usize) {
    let mut i = 0;
    while i < ctx.torpedo.len() {
        let hit = ctx.torpedo[i].live()
            && ctx.torpedo[i].team != team
            && ctx.torpedo[i].pos.distance(c) <= radius + 0.3;
        if !hit {
            i += 1;
            continue;
        }
        if ctx.torpedo[i].ghost {
            // JS L697: `netHurt(t.owner, 'torpedoArea', t.gid, dmg)`.
            let it = &ctx.torpedo[i];
            ctx.events.push(SimEvent::DeviceHit {
                owner_slot: it.owner_slot,
                team: it.owner_team,
                gid: it.gid,
                kind: "torpedoArea".to_string(),
                d: r2(dmg),
            });
            i += 1;
            continue;
        }
        if ctx.torpedo[i].locked() && dmg >= 50.0 {
            burst(ctx, i, true, false);
        } else {
            hurt(ctx.torpedo, ctx.events, i, dmg, false);
        }
        i += 1;
    }
}

/// The public-fan variant of [`damage_area_ctx`]: a caught locked torpedo
/// sets `triggered` instead of bursting (no paint / actor context on that
/// path — see the module notes).
pub(crate) fn damage_area_plain(
    torpedo: &mut [TorpedoItem],
    events: &mut Vec<SimEvent>,
    c: Vec3,
    radius: f32,
    dmg: f32,
    team: usize,
) {
    let mut trigger: Vec<usize> = Vec::new();
    let mut hurt_idx: Vec<usize> = Vec::new();
    for (i, it) in torpedo.iter().enumerate() {
        if !it.live() || it.team == team || it.pos.distance(c) > radius + 0.3 {
            continue;
        }
        if it.ghost {
            events.push(SimEvent::DeviceHit {
                owner_slot: it.owner_slot,
                team: it.owner_team,
                gid: it.gid,
                kind: "torpedoArea".to_string(),
                d: r2(dmg),
            });
            continue;
        }
        if it.locked() && dmg >= 50.0 {
            trigger.push(i);
        } else {
            hurt_idx.push(i);
        }
    }
    for i in trigger {
        torpedo[i].triggered = true;
    }
    for i in hurt_idx {
        hurt(torpedo, events, i, dmg, false);
    }
}

/// JS `SUB_KITS.torpedo.netHurt(gid, dmg)` (L726): the owner's side of a
/// shot-down hit made on its torpedo on another screen.
pub(crate) fn net_hurt(
    torpedo: &mut [TorpedoItem],
    events: &mut Vec<SimEvent>,
    gid: u32,
    dmg: f32,
) {
    let idx = torpedo
        .iter()
        .position(|x| !x.ghost && x.gid == gid && x.live());
    if let Some(i) = idx {
        hurt(torpedo, events, i, dmg, false);
    }
}

/// JS `KIT_GHOSTS.torpedoArea.netHurt(gid, dmg)` (L707-709): the owner's
/// side of a blast that caught its torpedo on another screen.
pub(crate) fn net_area_hurt(
    torpedo: &mut [TorpedoItem],
    events: &mut Vec<SimEvent>,
    gid: u32,
    dmg: f32,
) {
    let idx = torpedo
        .iter()
        .position(|x| !x.ghost && x.gid == gid && x.live());
    let Some(i) = idx else { return };
    if torpedo[i].locked() && dmg >= 50.0 {
        torpedo[i].triggered = true;
    } else {
        hurt(torpedo, events, i, dmg, false);
    }
}

/// JS `SUB_KITS.torpedo.blocked(a)` (L733): one live torpedo per owner — a
/// second throw is refused.
#[must_use]
pub fn blocked(torpedo: &[TorpedoItem], owner_idx: usize) -> bool {
    torpedo.iter().any(|t| t.owner_idx == owner_idx && t.live())
}
