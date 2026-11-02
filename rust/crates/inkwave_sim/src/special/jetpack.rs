//! `IMPL.jetpack` (specials.js L1512-1595) — Ink Jet: a body-owned flight
//! form (boost up, hover, fire ink shots); ends with a super-jump return to
//! the `origin` marker: the sim routes the marker through
//! [`crate::special::end`] (JS L374) + the actor's `sp_marker` /
//! `sp_marker_action` flags.

use crate::actor::{Actor, ActorInput, SimWorld, SpecialActive};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::PlayerTuning;
use crate::weapon::Rng;
use glam::{Vec3, vec3};

/// JS L1514: the initial boost cooldown (`cd: 0.3`) — hardcoded (D-T35-4).
const START_CD: f32 = 0.3;
/// JS L1516: the launch impulse (`a.vel.y = 7`) — hardcoded (D-T35-4).
const LAUNCH_VEL_Y: f32 = 7.0;
/// JS L1537: the ground probe above the actor (`pos.y + 0.5`) — hardcoded.
const HOVER_PROBE_UP: f32 = 0.5;
/// JS L1538: the hover floor never drops below `origin.y - 1` — hardcoded.
const FLOOR_DROP: f32 = 1.0;
/// JS L1539: the hover bob `sin(t * 2.2) * 0.12` — hardcoded.
const WOBBLE_FREQ: f32 = 2.2;
/// JS L1539: the hover bob amplitude.
const WOBBLE_AMP: f32 = 0.12;
/// JS L1544: the boost burst length (`boostT = 0.35`) — hardcoded.
const BOOST_TIME: f32 = 0.35;
/// JS L1552: the hover velocity gain (`(wantY - y) * 3`) — hardcoded.
const HOVER_GAIN: f32 = 3.0;
/// JS L1552: the hover velocity clamp `(-5, 6)` — hardcoded.
const HOVER_VY_MIN: f32 = -5.0;
/// JS L1552: the hover velocity clamp upper.
const HOVER_VY_MAX: f32 = 6.0;
/// JS L1553: the descend blend starts above `wantY + 1` — hardcoded.
const DESCEND_MARGIN: f32 = 1.0;
/// JS L1553: the descend / ascend blend rates (`2.5 : 6`) — hardcoded.
const HOVER_DESCEND: f32 = 2.5;
/// JS L1553: the ascend blend rate.
const HOVER_ASCEND: f32 = 6.0;
/// JS L1554: gravity fraction while boosting (`0.35`) — hardcoded.
const BOOST_GRAVITY_FRAC: f32 = 0.35;
/// JS L1563: the fire-pose hold (`firing = 0.4`) — hardcoded.
const FIRE_HOLD: f32 = 0.4;
/// JS L1576: the exhaust paint period (`paintT = 0.28`) — hardcoded.
const PAINT_PERIOD: f32 = 0.28;

/// Kind state (JS `s.origin`, `s.cd`, `s.paintT`, `s.boostCd`, `s.boostT`,
/// `s.jumpWas`, L1514).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.origin` (L1514): the special's start position (the `jumpBack`
    /// target).
    pub origin: Vec3,
    /// JS `s.cd` (L1514): seconds until the next boost.
    pub cd: f32,
    /// JS `s.paintT` (L1514): trail-paint accumulator.
    pub paint_t: f32,
    /// JS `s.boostCd` / `s.boostT` (L1514): the boost ladder.
    pub boost_cd: f32,
    /// JS `s.boostT` (L1514).
    pub boost_t: f32,
    /// JS `s.jumpWas` (L1514): the jump-edge tracker.
    pub jump_was: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            cd: 0.0,
            paint_t: 0.0,
            boost_cd: 0.0,
            boost_t: 0.0,
            jump_was: false,
        }
    }
}

/// JS `IMPL.jetpack.start` (L1513-1517).
pub fn start(a: &mut Actor, s: &mut SpecialActive) {
    s.body = true;
    s.jump_back = true; // JS L1514
    a.grounded = false;
    a.vel.y = LAUNCH_VEL_Y; // JS L1516 (D-T35-4)
    a.sp_marker = Some("jetpack".to_string()); // JS L1515: `ReturnMarker`
    s.state = crate::special::KindState::Jetpack(State {
        origin: a.pos,
        cd: START_CD,
        jump_was: true,
        ..State::default()
    });
    // JS L1517 `_swapWeapon(a, 'sp_jetgun')`: the sim never swaps the runner
    // weapon — the jetpack's `weapon_hook` arm always returns `true` (skip
    // the runner), so the held weapon id is irrelevant (deviation: the real
    // `sp_jetgun` stats are never consulted; the blast is hardcoded in the
    // jetpack ladder, JS L1566-1567).
}

/// JS `IMPL.jetpack.body` (L1530-1587): hover steering + boost + fire +
/// exhaust paint. The fire / paint mechanics ride the `sp_jet_fire` /
/// `sp_jet_paint` flags (the match drains them after the step).
pub fn body(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    world: &SimWorld,
    t: &PlayerTuning,
    input: &ActorInput,
) {
    let d = &crate::special::specials().jetpack;
    let crate::special::KindState::Jetpack(st) = &mut s.state else {
        return;
    };
    // JS L1531-1535: horizontal steer toward the stick at hover speed.
    let mv = input.move_dir;
    let tvx = mv.x * d.speed;
    let tvz = mv.z * d.speed;
    let ex = tvx - a.vel.x;
    let ez = tvz - a.vel.z;
    let el = (ex * ex + ez * ez).sqrt(); // JS Math.hypot
    let r = d.accel * dt;
    if el <= r {
        a.vel.x = tvx;
        a.vel.z = tvz;
    } else {
        a.vel.x += ex / el * r;
        a.vel.z += ez / el * r;
    }
    // JS L1537-1539: hover height over whatever is below (never lower than
    // above the take-off point).
    let gy = world
        .collision
        .ground_height(a.pos.x, a.pos.z, a.pos.y + HOVER_PROBE_UP, false);
    let floor = if gy == f32::NEG_INFINITY {
        st.origin.y
    } else {
        gy.max(st.origin.y - FLOOR_DROP)
    };
    let want_y = floor + d.height + (s.t * WOBBLE_FREQ).sin() * WOBBLE_AMP;
    // JS L1541-1550: jump = a boost burst.
    st.boost_cd -= dt;
    st.boost_t -= dt;
    let jump = input.jump;
    if jump && !st.jump_was && st.boost_cd <= 0.0 {
        st.boost_cd = d.boost_gap;
        st.boost_t = BOOST_TIME;
        a.vel.y = d.boost;
    }
    st.jump_was = jump;
    // JS L1551-1554: hover settle vs the reduced-gravity boost arc.
    if st.boost_t <= 0.0 {
        let vy = ((want_y - a.pos.y) * HOVER_GAIN).clamp(HOVER_VY_MIN, HOVER_VY_MAX);
        let blend = if a.pos.y > want_y + DESCEND_MARGIN {
            HOVER_DESCEND
        } else {
            HOVER_ASCEND
        };
        a.vel.y += (vy - a.vel.y) * 1.0_f32.min(dt * blend);
    } else {
        a.vel.y -= t.gravity * BOOST_GRAVITY_FRAC * dt;
    }
    // JS L1555-1559: integrate + resolve, airborne, face the aim.
    let py = a.pos.y;
    a.pos += a.vel * dt;
    a.body_resolve(false, py, false, world, t);
    a.grounded = false;
    s.face_yaw = Some(a.aim_yaw);
    // JS L1561-1572: fire (the blast itself is the match's drain).
    st.cd -= dt;
    if input.fire && st.cd <= 0.0 {
        st.cd = d.interval;
        s.firing = FIRE_HOLD;
        a.sp_jet_fire = true;
    }
    // JS L1574-1579: exhaust ink below (drain: the seed is drawn only when
    // the ground probe lands, JS L1577-1578).
    st.paint_t -= dt;
    if st.paint_t <= 0.0 {
        st.paint_t = PAINT_PERIOD;
        a.sp_jet_paint = true;
    }
}

/// JS L1577-1578: the exhaust trail splat under the hover (the match drains
/// [`Actor::sp_jet_paint`] into this — the seed is drawn only when the ground
/// probe lands, JS L1577-1578, and it never charges the gauge).
pub fn paint_trail(
    a: &mut Actor,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
) {
    const PROBE_DOWN: f32 = 9.0; // JS L1577: `groundBelow(a.pos, 9)` (D-T35-4)
    const LIFT: f32 = 0.2; // JS L1578: `g.setY(g.y + 0.2)`
    const R: f32 = 1.0; // JS L1578: paint radius `1.0`
    if let Some(g) = crate::special::world::ground_below(collision, a.pos, PROBE_DOWN) {
        let seed = rng.next_f32();
        let area = paint.splat(
            collision,
            vec3(g.x, g.y + LIFT, g.z),
            R,
            a.team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        a.add_turf_no_special(area);
    }
}

/// JS `IMPL.jetpack` has no `tick` (body-owned).
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.jetpack.end` (L1588-1594): audio / pack visuals +
/// `_restoreWeapon` — presentation only in the sim (the runner stays
/// untouched; the `weapon_hook` arm just stops skipping it).
pub fn end(_a: &mut Actor, _reason: &str) {}
