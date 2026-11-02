//! `IMPL.crab` (specials.js L1908-2043) — Crab Rig: an ink-proof crab tank
//! (gatling + cannon + roll); damage hits the hull (`hurt`, L2014-2026 —
//! live here), the hull breaks at 0 hp (`end('broken')`, L2012, reported by
//! [`crate::special::dispatch_tick`]).

use crate::actor::{Actor, ActorInput, FireGate, SpecialActive, angle_diff};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::special::specials;
use crate::special::world;
use crate::weapon::{ProjectileSim, Rng};
use glam::{Vec3, vec3};

/// JS L1912: the initial cannon cooldown (`ccd: 0.4`) — hardcoded (D-T35-4).
const START_CCD: f32 = 0.4;
/// JS L2040: the dismount hop (`vel.y = 5`) — hardcoded (D-T35-4).
const END_HOP_VEL_Y: f32 = 5.0;
/// JS L1927: the gatling muzzle fallback `(0, 0.7, 1.0)` (the prop anchor
/// does not exist in the sim) — hardcoded (D-T35-4).
const GUN_MUZZLE: Vec3 = vec3(0.0, 0.7, 1.0);
/// JS L1937: the cannon muzzle fallback `(0.5, 1.1, -0.2)` — hardcoded.
const CANNON_MUZZLE: Vec3 = vec3(0.5, 1.1, -0.2);
/// JS L1956: the roll follows velocity above `0.8` m/s — hardcoded.
const ROLL_HS_MIN: f32 = 0.8;
/// JS L1957: the rolled hull turn rate (`8` rad/s) — hardcoded.
const ROLL_TURN: f32 = 8.0;
/// JS L1962: the gun pitch clamp `(-0.4, 0.5)` — hardcoded.
const PITCH_MIN: f32 = -0.4;
/// JS L1962: the gun pitch clamp upper.
const PITCH_MAX: f32 = 0.5;
/// JS L1965: the fire-pose hold (`firing = 0.3`) — hardcoded.
const FIRE_HOLD: f32 = 0.3;
/// JS L1967: the catch-up burst cap (`guard++ < 3`) — hardcoded.
const GUARD_MAX: usize = 3;
/// JS L1980: the cannon lead pitch `clamp(aimPitch + 0.45, 0.1, 1.0)`.
const CANNON_PITCH_BIAS: f32 = 0.45;
/// JS L1980: the cannon lead pitch clamp lower.
const CANNON_PITCH_MIN: f32 = 0.1;
/// JS L1980: the cannon lead pitch clamp upper.
const CANNON_PITCH_MAX: f32 = 1.0;
/// JS L2003: the roll spin rate divisor (`hs / 0.95`) — visual, hardcoded.
const ROLL_ANG_DIV: f32 = 0.95;
/// JS L2005: the rolled footprint gate (`hs > 2`) — hardcoded.
const ROLL_PAINT_HS: f32 = 2.0;
/// JS L2005: the footprint period (`legT > 0.08`) — hardcoded.
const LEG_PERIOD: f32 = 0.08;
/// JS L2007: the walking leg timer rate (`1 + hs * 2.2`) — visual.
const LEG_RATE_SPAN: f32 = 2.2;
/// JS L2005: the footprint splat `pos + 0.2y`, radius `0.9` — hardcoded.
const FOOTPRINT_UP: f32 = 0.2;
/// JS L2005: the footprint splat radius.
const FOOTPRINT_R: f32 = 0.9;

/// The ghost-side net snapshot (JS `s.net`, L2134: `{roll, firing, hull}`).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Net {
    /// JS `N.roll` (L2134).
    pub roll: bool,
    /// JS `N.firing` (L2135).
    pub firing: bool,
    /// JS `N.hull` (L2134): the owner's hull yaw, quantised to a byte.
    pub hull: f32,
}

/// Kind state (JS `s.hp`, `s.hull`, `s.roll`, `s.gcd`, `s.ccd`, `s.subWas`,
/// `s.legT`, `s.rollAng`, `s.net`, L1911-1912 / L2134).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.hp` (L1911): hull hit points (damage is filtered through
    /// [`hurt`], never the actor's own `hp`).
    pub hp: f32,
    /// JS `s.hull` (L1911): the hull yaw (the body facing).
    pub hull: f32,
    /// JS `s.roll` (L1912): rolled into the armoured ball.
    pub roll: bool,
    /// JS `s.gcd` (L1912): gatling cooldown.
    pub gcd: f32,
    /// JS `s.ccd` (L1912): cannon cooldown.
    pub ccd: f32,
    /// JS `s.subWas` (L1912): the cannon-press edge tracker.
    pub sub_was: bool,
    /// JS `s.legT` (L1912): leg animation timer — out of sim scope.
    pub leg_t: f32,
    /// JS `s.rollAng` (L1912): accumulated roll angle — visual.
    pub roll_ang: f32,
    /// JS `s.net` (L2134): the ghost-side snapshot (`None` on the owner).
    pub net: Option<Net>,
}

/// JS `IMPL.crab.start` (L1909-1943).
pub fn start(a: &mut crate::actor::Actor, s: &mut SpecialActive) {
    let d = &specials().crab;
    s.no_squid = true;
    s.ink_proof = true;
    s.speed = Some(d.speed);
    s.face_yaw = Some(a.yaw); // JS L1911: `faceYaw: a.yaw`
    a.form = crate::actor::Form::Kid; // JS L1913
    s.state = crate::special::KindState::Crab(State {
        hp: d.hp as f32,
        hull: a.yaw,
        ccd: START_CCD,
        ..State::default()
    });
    // JS L1914: `a.hitR = 1.0; a.hitH = 2.2` — the sim has no enlarged hitbox
    // (shots test the actor's feet/body probes); batch 4 reviews whether the
    // crab's bigger profile needs one.
}

/// JS `IMPL.crab.hurt` (L2014-2026): front/above hits reach the rider,
/// everything else is absorbed by the hull (roll armour on top).
/// `attacker_pos` is `None` for self/environmental damage — JS's
/// `attacker !== a` guard is equivalent: a co-located self hit gives
/// `front = 0`, `not above` → hull absorbs (same outcome).
pub fn hurt(a: &mut crate::actor::Actor, amount: f32, attacker_pos: Option<Vec3>) -> f32 {
    let apos = a.pos;
    let Some(s) = a.special_active.as_mut() else {
        return amount;
    };
    let crate::special::KindState::Crab(st) = &mut s.state else {
        return amount;
    };
    let mut to_rider = false;
    if let Some(ap) = attacker_pos
        && !st.roll
    {
        // JS L2018: `rl = Math.hypot(rx, rz) || 1`.
        let rx = ap.x - apos.x;
        let rz = ap.z - apos.z;
        let rl = rx.hypot(rz);
        let rl = if rl == 0.0 { 1.0 } else { rl };
        let front = (rx * st.hull.sin() + rz * st.hull.cos()) / rl;
        // JS L2020: from behind or from above.
        to_rider = front < -0.45 || ap.y > apos.y + 2.4;
    }
    if to_rider {
        return amount;
    }
    let armor = if st.roll {
        specials().crab.roll_armor
    } else {
        1.0
    };
    st.hp -= amount * armor; // JS L2023
    0.0 // JS L2026
}

/// JS `IMPL.crab.local(a, s, p)` (L1990-1993): a point in the hull frame →
/// world.
#[must_use]
fn local(a: &Actor, hull: f32, p: Vec3) -> Vec3 {
    let (c, sn) = (hull.cos(), hull.sin());
    vec3(
        a.pos.x + p.x * c + p.z * sn,
        a.pos.y + p.y,
        a.pos.z - p.x * sn + p.z * c,
    )
}

/// JS `IMPL.crab.weapon` (L1944-1988): roll toggle (raw `intent.squid`,
/// L1947), hull steering, the gatling burst (`winp.fire`, L1964) and the
/// cannon shot (`winp.sub` edge, L1976). Always returns `true` (JS L1960 /
/// L1987 — the crab owns the trigger).
#[allow(clippy::too_many_arguments)]
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    input: &ActorInput,
    gate: &mut FireGate,
    sys: &mut crate::special::SpecialSystem,
    rng: &mut Rng,
    ps: &mut ProjectileSim,
) -> bool {
    let d = &specials().crab;
    let crate::special::KindState::Crab(st) = &mut s.state else {
        return true;
    };
    // JS L1948-1953: swim = roll into an armoured ball (fast, can't shoot).
    let roll = input.squid;
    if roll != st.roll {
        st.roll = roll;
        s.speed = Some(if roll { d.roll_speed } else { d.speed }); // JS L1950
    }
    // JS L1955-1959: the hull turns slowly toward the aim.
    let hs = (a.vel.x * a.vel.x + a.vel.z * a.vel.z).sqrt();
    let want = if roll {
        if hs > ROLL_HS_MIN {
            a.vel.x.atan2(a.vel.z)
        } else {
            st.hull
        }
    } else {
        a.aim_yaw
    };
    let rate = (if roll { ROLL_TURN } else { d.turn_rate }) * dt;
    st.hull += angle_diff(st.hull, want).clamp(-rate, rate);
    s.face_yaw = Some(st.hull);
    if roll {
        return true; // JS L1960
    }
    /// JS L1970: the `Math.PI / 180` deg→rad factor of the gun spread.
    const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
    st.gcd -= dt;
    st.ccd -= dt;
    let pitch = a.aim_pitch.clamp(PITCH_MIN, PITCH_MAX); // JS L1962
    let cp = pitch.cos();
    if gate.fire {
        s.firing = FIRE_HOLD; // JS L1965
        let mut guard = 0usize;
        while st.gcd <= 0.0 && guard < GUARD_MAX {
            // JS L1967-1974: one gatling round per catch-up slot.
            guard += 1;
            st.gcd += d.gun_interval;
            let m = local(a, st.hull, GUN_MUZZLE);
            let sp = (rng.next_f32() - 0.5) * 2.0 * d.gun_spread * DEG_TO_RAD;
            let sp2 = (rng.next_f32() - 0.5) * d.gun_spread * DEG_TO_RAD;
            let dir = vec3(
                (st.hull + sp).sin() * cp,
                (pitch + sp2).sin(),
                (st.hull + sp).cos() * cp,
            );
            ps.fire_crab_gatling(a, d, m, dir);
        }
    } else if st.gcd < 0.0 {
        st.gcd = 0.0; // JS L1975
    }
    // JS L1976-1986: the mortar shell on the sub press edge.
    let sub_press = gate.sub && !st.sub_was;
    st.sub_was = gate.sub;
    if sub_press && st.ccd <= 0.0 {
        st.ccd = d.cannon_gap;
        let m = local(a, st.hull, CANNON_MUZZLE);
        let lp = (a.aim_pitch + CANNON_PITCH_BIAS).clamp(CANNON_PITCH_MIN, CANNON_PITCH_MAX);
        let vel =
            vec3(st.hull.sin() * lp.cos(), lp.sin(), st.hull.cos() * lp.cos()) * d.cannon_speed;
        let obj = world::make_shell(a.slot, a.team, false, m, vel);
        let data = world::v3(m)
            .into_iter()
            .chain(world::v3(vel))
            .collect::<Vec<_>>();
        crate::special::spawn(a, sys, obj, &data);
    }
    true // JS L1987
}

/// JS `IMPL.crab.tick` (L1994-2013): the roll spin / leg timers and the
/// rolled footprint paint (raised as [`Actor::sp_crab_paint`] — the match
/// drains it with the paint grid; the ghost tick runs it too (JS L2118) and
/// replication drains the request muted). The hull break (JS L2012 `s.hp <= 0
/// → end('broken')`) is reported by [`crate::special::dispatch_tick`].
pub fn tick(a: &mut Actor, s: &mut SpecialActive, dt: f32) {
    let crate::special::KindState::Crab(st) = &mut s.state else {
        return;
    };
    let hs = (a.vel.x * a.vel.x + a.vel.z * a.vel.z).sqrt();
    if st.roll {
        st.roll_ang += (hs / ROLL_ANG_DIV) * dt; // JS L2003 (visual spin)
        if a.grounded && hs > ROLL_PAINT_HS {
            st.leg_t += dt;
            if st.leg_t > LEG_PERIOD {
                st.leg_t = 0.0;
                a.sp_crab_paint = true; // JS L2005
            }
        }
    } else {
        st.leg_t += dt * (1.0 + hs * LEG_RATE_SPAN); // JS L2007 (visual legs)
    }
}

/// The drain for [`Actor::sp_crab_paint`] (JS L2005): one seed draw +
/// the footprint splat, turf credited without charging the gauge. `muted`
/// is the JS `ghostMute` window (registry.js L73-77) around the ghost tick:
/// the `paint` helper evaluates the seed `Math.random()` at the call site
/// (specials.js L53) *before* `PaintSystem.splat` returns 0 under the mute
/// (paint.js L470), so the draw is consumed even when the splat is dropped
/// (same convention as [`world::paint_splat`]).
pub fn paint_footprint(
    a: &mut Actor,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    muted: bool,
) {
    let seed = rng.next_f32();
    if muted {
        return;
    }
    let pos = a.pos;
    let area = paint.splat(
        collision,
        vec3(pos.x, pos.y + FOOTPRINT_UP, pos.z),
        FOOTPRINT_R,
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

/// JS `IMPL.crab.end` (L2028-2042): the dismount hop.
pub fn end(a: &mut crate::actor::Actor, _reason: &str) {
    if a.alive {
        // JS L2040: `a.vel.y = 5; a.grounded = false`.
        a.vel.y = END_HOP_VEL_Y;
        a.grounded = false;
    }
    // JS L2031: `a.hitR/hitH = undefined` — no sim-side hitbox to restore.
}
