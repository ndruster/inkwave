//! `IMPL.stamp` (specials.js L1598-1714) — Mega Stamp: a heavy body that
//! swings (front guard deflects damage — live in [`crate::special`]
//! `filter_damage`), smashes, flips and throws. The swing ladder is batch 4:
//! [`move_hook`] (body-yaw slew), [`weapon`] (swing / flip / throw),
//! [`tick_full`] (flip windows + guard sweep) and [`slam`] (drained by the
//! match from [`Actor::sp_stamp_slam`]).

use crate::actor::{Actor, ActorInput, FireGate, SpecialActive, angle_diff};
use crate::collision::CollisionWorld;
use crate::net::KitVal;
use crate::paint::{PaintGrid, SplatOpts};
use crate::special::specials;
use crate::special::world;
use crate::sub::{SubKind, SubState, SubSystem};
use crate::tuning::PlayerTuning;
use crate::weapon::{ProjectileSim, Rng, SimEvent, apply_hit};
use glam::{Vec3, vec3};

/// JS L1607: the "moving" threshold that picks the faster turn rate
/// (`hs > 1.5 ? turnRate : turnRateStill`) — hardcoded (D-T35-4).
const MOVE_HS_THRESH: f32 = 1.5;
/// JS L1628: the throw origin (`pos.y + 1.5`) — hardcoded.
const THROW_FROM_UP: f32 = 1.5;
/// JS L1637: the flip cooldown padding (`cd = flipTime + 0.1`) — hardcoded.
const FLIP_CD_EXTRA: f32 = 0.1;
/// JS L1642: the grounded swing wind-up (`pending = 0.15`) — hardcoded.
const PENDING_SWING: f32 = 0.15;
/// JS L1646: the firing-pose hold (`s.firing = 0.5`) — hardcoded.
const FIRING_HOLD: f32 = 0.5;
/// JS L1662: the back-smash window (`t >= flipTime * 0.3`) — hardcoded.
const FLIP_BACK_FRAC: f32 = 0.3;
/// JS L1663: the front-smash window (`t >= flipTime * 0.75`) — hardcoded.
const FLIP_FRONT_FRAC: f32 = 0.75;
/// JS L1673: the guard sweep test height (`pos.y + 1.0`) — hardcoded.
const GUARD_AY: f32 = 1.0;
/// JS L1677: the shot vertical tolerance (`|p.y - ay| > 2`) — hardcoded.
const SHOT_Y_TOL: f32 = 2.0;
/// JS L1697: the slam ground probe origin (`pos.y + 1.2`) — hardcoded.
const SLAM_PROBE_UP: f32 = 1.2;
/// JS L1697: the slam ground probe length (`raycast … DOWN 3`).
const SLAM_PROBE_DIST: f32 = 3.0;
/// JS L1699-1700: the slam paint lifts (`at.y + 0.2`) — hardcoded.
const PAINT_LIFT: f32 = 0.2;
/// JS L1699: the main slam splat radius (`radius * 0.95`) — hardcoded.
const PAINT_MAIN_FRAC: f32 = 0.95;
/// JS L1700: the forward splat offset (`at + f * radius * 0.7`) — hardcoded.
const PAINT_FWD_FRAC: f32 = 0.7;
/// JS L1700: the forward splat radius (`radius * 0.6`) — hardcoded.
const PAINT_FWD_R_FRAC: f32 = 0.6;
/// JS L1707: the slam vertical gate (`|e.y - at.y| > 1.8`) — hardcoded.
const SLAM_DY_MAX: f32 = 1.8;
/// JS L1708: the slam LOS origin lift (`at + 0.5y`) — hardcoded.
const SLAM_LOS_FROM_UP: f32 = 0.5;
/// JS L1708: the slam LOS target lift (`e.pos + 0.6y`) — hardcoded.
const SLAM_LOS_TARGET_UP: f32 = 0.6;
/// JS L1711: the slam's device damage (`damageArea(at, radius, 80, team)`)
/// — hardcoded.
const SLAM_SUB_DMG: f32 = 80.0;

/// JS `s.flip` (L1600/L1637): an in-flight airborne flip, with the two smash
/// windows already fired (`back` first, then `front`).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Flip {
    /// JS `fl.t` (L1637): seconds since the flip started.
    pub t: f32,
    /// JS `fl.back` (L1637): the behind smash has fired.
    pub back: bool,
    /// JS `fl.front` (L1637): the in-front smash has fired.
    pub front: bool,
}

/// Kind state (JS `s.cd`, `s.pending`, `s.subWas`, `s.guard`, `s.flip`,
/// `s.bodyYaw`, L1600).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.cd` (L1600): seconds until the next swing.
    pub cd: f32,
    /// JS `s.pending` (L1600): `-1` none, else the queued swing timer.
    pub pending: f32,
    /// JS `s.subWas` (L1600): the throw-press edge tracker.
    pub sub_was: bool,
    /// JS `s.guard` (L1600): seconds of front-guard left (the deflect gate).
    pub guard: f32,
    /// JS `s.flip` (L1600): an airborne flip is in progress.
    pub flip: Option<Flip>,
    /// JS `s.bodyYaw` (L1600): the body facing (turns slower than the aim).
    pub body_yaw: f32,
}

/// JS `stampFwd(s)` (specials.js L56): the body facing unit vector.
#[must_use]
fn stamp_fwd(body_yaw: f32) -> Vec3 {
    vec3(body_yaw.sin(), 0.0, body_yaw.cos())
}

/// JS `IMPL.stamp.start` (L1599-1602).
pub fn start(a: &mut crate::actor::Actor, s: &mut SpecialActive) {
    s.speed = Some(specials().stamp.move_speed);
    s.no_squid = true;
    s.face_yaw = Some(a.yaw); // JS L1600: `faceYaw: a.yaw`
    s.state = crate::special::KindState::Stamp(State {
        pending: -1.0,
        guard: 0.0,
        body_yaw: a.yaw,
        ..State::default()
    });
    // JS L1601: `this._swapWeapon(a, 'sp_stamp')` — the sim has no held-weapon
    // model; the runner is reset on use via `peek_special_use` (deviation).
}

/// JS `IMPL.stamp.move` (L1605-1618): the body-yaw slew toward the aim
/// (faster when stopped), `faceYaw = bodyYaw`, and the along/strafe input
/// filter (sideways shuffle). The actor's move hook rewrites the fresh
/// per-frame intent copy. Deviation: JS's `mvIn`/`mvOut` compound guard
/// (L1611-1613) is unnecessary — the sim hands [`move_hook`] a new copy of
/// the intent every frame (see the actor step), so the filter never re-fires
/// on its own output.
pub fn move_hook(a: &Actor, s: &mut SpecialActive, dt: f32, intent: &mut ActorInput) {
    let d = &specials().stamp;
    let crate::special::KindState::Stamp(st) = &mut s.state else {
        return;
    };
    let hs = a.vel.x.hypot(a.vel.z);
    let rate = (if hs > MOVE_HS_THRESH {
        d.turn_rate
    } else {
        d.turn_rate_still
    }) * dt;
    st.body_yaw += angle_diff(st.body_yaw, a.aim_yaw).clamp(-rate, rate);
    s.face_yaw = Some(st.body_yaw); // JS L1609
    let f = stamp_fwd(st.body_yaw);
    let mv = &mut intent.move_dir;
    let along = mv.x * f.x + mv.z * f.z; // JS L1614
    mv.x = f.x * along + (mv.x - f.x * along) * d.strafe; // JS L1615
    mv.z = f.z * along + (mv.z - f.z * along) * d.strafe; // JS L1616
}

/// JS `IMPL.stamp.weapon` (L1619-1657): the throw (sub press — ends the
/// special), the swing / airborne flip (`winp.fire`), and the pending-swing
/// resolution (queued into [`Actor::sp_stamp_slam`], drained by the match
/// after the runner, plus the instant lunge). Always returns `true` (JS
/// L1631/L1656 — the stamp owns the trigger).
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    gate: &FireGate,
    sys: &mut crate::special::SpecialSystem,
) -> (bool, Option<&'static str>) {
    let d = &specials().stamp;
    let crate::special::KindState::Stamp(st) = &mut s.state else {
        return (true, None);
    };
    st.cd -= dt; // JS L1621
    let sub_press = gate.sub && !st.sub_was; // JS L1622
    st.sub_was = gate.sub;
    if sub_press {
        // JS L1623-1632: throw it — a long-range blast, and the special is
        // over. The `throw` anim trigger (L1627) is presentation.
        let m = ProjectileSim::muzzle(a); // JS L1625
        let dir = ProjectileSim::aim_from(a, m, None); // JS L1626
        let from = vec3(a.pos.x, a.pos.y + THROW_FROM_UP, a.pos.z); // JS L1628
        let obj = world::make_tstamp(a.slot, a.team, false, from, dir);
        let data = world::v3(from)
            .into_iter()
            .chain(world::v3(dir))
            .collect::<Vec<_>>();
        crate::special::spawn(a, sys, obj, &data); // JS L1629
        return (true, Some("throw")); // JS L1630-1631
    }
    if gate.fire && st.cd <= 0.0 && st.flip.is_none() {
        // JS L1633-1647.
        if !a.grounded {
            // JS L1634-1640: mid-air flip — behind first, then a long smash
            // in front. The `roll` trigger and the swing sound are
            // presentation.
            st.flip = Some(Flip::default());
            st.guard = d.flip_time;
            st.cd = d.flip_time + FLIP_CD_EXTRA;
        } else {
            st.pending = PENDING_SWING; // JS L1642
            st.cd = d.interval;
            st.guard = d.guard_time;
        }
        s.firing = FIRING_HOLD; // JS L1646
    }
    if st.pending >= 0.0 {
        st.pending -= dt; // JS L1648-1649
        if st.pending < 0.0 {
            // JS L1650-1653: the smash lands (queued — the match drains it
            // after the runner, same frame, positions unchanged), plus the
            // lunge shove along the body facing.
            a.sp_stamp_slam.push((d.radius, d.reach));
            let f = stamp_fwd(st.body_yaw);
            a.vel.x += f.x * d.lunge;
            a.vel.z += f.z * d.lunge;
        }
    }
    (true, None) // JS L1656
}

/// JS `IMPL.stamp.tick` (L1658-1670): the flip's two smash windows (queued
/// into [`Actor::sp_stamp_slam`]) and the guard decay + sweep. The match
/// calls this before [`crate::special::tick`] (JS order: weapon → tick).
/// The front-smash `shake` (JS L1663) is presentation.
pub fn tick_full(a: &mut Actor, dt: f32, ps: &mut ProjectileSim, subs: &mut SubSystem) {
    let d = &specials().stamp;
    let mut sweep = false;
    let mut slams: Vec<(f32, f32)> = Vec::new();
    {
        let Some(s) = a.special_active.as_mut() else {
            return;
        };
        if s.ghost || s.ended || s.kind != "stamp" {
            return;
        }
        let crate::special::KindState::Stamp(st) = &mut s.state else {
            return;
        };
        if let Some(fl) = &mut st.flip {
            fl.t += dt; // JS L1661
            if !fl.back && fl.t >= d.flip_time * FLIP_BACK_FRAC {
                fl.back = true;
                slams.push((d.flip_back_radius, -d.flip_back_reach)); // JS L1662
            }
            if !fl.front && fl.t >= d.flip_time * FLIP_FRONT_FRAC {
                fl.front = true;
                slams.push((d.flip_radius, d.flip_reach)); // JS L1663
            }
            if fl.t >= d.flip_time {
                st.flip = None; // JS L1664
            }
        }
        if st.guard > 0.0 {
            st.guard -= dt; // JS L1667
            sweep = true;
        }
    }
    a.sp_stamp_slam.extend(slams);
    if sweep {
        guard_sweep(a, ps, subs); // JS L1668
    }
}

/// JS `IMPL.stamp.guardSweep` (L1672-1693): the swing's guard knocks enemy
/// shots out of the air, defuses enemy bombs in front, and smashes the
/// smashable sub devices (JS `STAMP_SMASHES`, L60). The burst/shield-hit FX
/// (L1680/L1686) are presentation. Deviation: JS `subs._destroy(it)` (L1691)
/// is immediate removal; the sim marks `SubState::Dead`, which the sub update
/// splices at the end of its pass (same outcome, one frame later for the
/// device's own step).
pub fn guard_sweep(a: &Actor, ps: &mut ProjectileSim, subs: &mut SubSystem) {
    let d = &specials().stamp;
    let ay = a.pos.y + GUARD_AY; // JS L1673
    // JS L1675-1681: enemy shots in the front arc, reverse order (the sim's
    // `remove_shot` swap-removes, so descending indices visit each once).
    for i in (0..ps.list.len()).rev() {
        let p = &ps.list[i];
        if p.team == a.team || (p.pos.y - ay).abs() > SHOT_Y_TOL {
            continue; // JS L1677
        }
        let dh = (p.pos.x - a.pos.x).hypot(p.pos.z - a.pos.z);
        if dh > d.deflect_r || !crate::special::stamp_front(a, p.pos, d.deflect_arc) {
            continue; // JS L1678
        }
        ps.remove_shot(i); // JS L1679
    }
    // JS L1682-1687: enemy splat bombs in front (JS iterates a copy; the sim
    // `defuse_bomb` swap-removes, so descending indices are equivalent).
    let target = vec3(a.pos.x, ay, a.pos.z);
    for i in (0..ps.bombs.len()).rev() {
        let b = &ps.bombs[i];
        if b.team == a.team || b.kind != "bomb" {
            continue; // JS L1683
        }
        if b.pos.distance(target) > d.bomb_clear_r
            || !crate::special::stamp_front(a, b.pos, d.deflect_arc)
        {
            continue; // JS L1684
        }
        ps.defuse_bomb(i); // JS L1685
    }
    // JS L1688-1692: the smashable sub devices in front.
    for it in subs.items.iter_mut() {
        if it.team == a.team
            || it.state == SubState::Dead
            || !matches!(
                it.kind,
                SubKind::Bomb
                    | SubKind::Sticky
                    | SubKind::Burst
                    | SubKind::Seeker
                    | SubKind::Mist
                    | SubKind::Mine
            )
        {
            continue; // JS L1689
        }
        if it.pos.distance(target) > d.bomb_clear_r
            || !crate::special::stamp_front(a, it.pos, d.deflect_arc)
        {
            continue; // JS L1690
        }
        it.state = SubState::Dead; // JS L1691
    }
}

/// JS `IMPL.stamp.slam(a, s, radius, reach)` (L1694-1712): the smash — a
/// ground probe under the target point, two team splats, the `['4','sl'…]`
/// record, and flat `def.damage` to every enemy in the splash disc with line
/// of sight (the stamp runs its own loop, not [`crate::special::blast`]),
/// plus the device damage. The match drains [`Actor::sp_stamp_slam`] into
/// this after the actor step. The explosion FX / sound / rumble (L1702-1703)
/// are presentation.
#[allow(clippy::too_many_arguments)]
pub fn slam(
    actors: &mut [Actor],
    idx: usize,
    radius: f32,
    reach: f32,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    ps: &mut ProjectileSim,
    subs: &mut SubSystem,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    let d = &specials().stamp;
    let (team, owner_slot, pos, body_yaw) = {
        let a = &actors[idx];
        let yaw = a
            .special_active
            .as_ref()
            .and_then(|s| match &s.state {
                crate::special::KindState::Stamp(st) => Some(st.body_yaw),
                _ => None,
            })
            .unwrap_or(a.yaw);
        (a.team, a.slot, a.pos, yaw)
    };
    // JS L1695-1698: the target point, probed down onto the floor.
    let f = stamp_fwd(body_yaw);
    let c = pos + f * reach;
    let g = collision.raycast(
        vec3(c.x, pos.y + SLAM_PROBE_UP, c.z),
        Vec3::NEG_Y,
        SLAM_PROBE_DIST,
        true,
    );
    let at = if g.hit {
        g.point
    } else {
        vec3(c.x, pos.y, c.z)
    };
    // JS L1699-1700: the two splats (two seed draws, call order).
    let seed = rng.next_f32();
    let mut area = paint.splat(
        collision,
        vec3(at.x, at.y + PAINT_LIFT, at.z),
        radius * PAINT_MAIN_FRAC,
        team,
        &SplatOpts {
            seed,
            ..SplatOpts::default()
        },
    );
    let fwd = at + f * (radius * PAINT_FWD_FRAC);
    let seed = rng.next_f32();
    area += paint.splat(
        collision,
        vec3(fwd.x, at.y + PAINT_LIFT, fwd.z),
        radius * PAINT_FWD_R_FRAC,
        team,
        &SplatOpts {
            seed,
            ..SplatOpts::default()
        },
    );
    actors[idx].add_turf_no_special(area);
    // JS L1701: `rec(a, [4, 'sl', ...v3(at), r2(radius)])`.
    let mut rec = vec![KitVal::Num(4.0), KitVal::Str("sl".to_string())];
    rec.extend(world::v3(at).into_iter().map(KitVal::Num));
    rec.push(KitVal::Num(world::r2(radius)));
    actors[idx].sp_records.push(rec);
    // JS L1704-1710: the damage disc (XZ radius + the player's own radius,
    // a 1.8 m vertical gate, LOS from `at + 0.5y` to `e + 0.6y`).
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue; // JS L1705
        }
        let dh = (e.pos.x - at.x).hypot(e.pos.z - at.z);
        if dh > radius + t.radius || (e.pos.y - at.y).abs() > SLAM_DY_MAX {
            continue; // JS L1707
        }
        let ep = vec3(e.pos.x, e.pos.y + SLAM_LOS_TARGET_UP, e.pos.z);
        if !collision.los(vec3(at.x, at.y + SLAM_LOS_FROM_UP, at.z), ep) {
            continue; // JS L1708
        }
        hits.push((ei, d.damage)); // JS L1709
    }
    for (ei, dmg) in hits {
        // JS L1709 `G.projectiles.applyHit(a, e, s.def.damage, 'stamp')`:
        // remote victims route `'send'` like [`crate::special::slam_impact`].
        let (vs, vt) = (actors[ei].slot, actors[ei].team);
        let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
        if send {
            ps.push_event(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(&mut actors[ei], dmg, Some((team, owner_slot)), Some(pos), t);
            ps.push_event(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }
    subs.damage_area(at, radius, SLAM_SUB_DMG, team, false); // JS L1711
}

/// JS `IMPL.stamp.end` (L1713): `this._restoreWeapon(a)` — the sim has no
/// held-weapon model (see [`start`]), so this is a no-op.
pub fn end(_a: &mut crate::actor::Actor, _reason: &str) {}
