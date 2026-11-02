//! `IMPL.kraken` (specials.js L1410-1469) — Kraken: an ink-proof trampling
//! form; LMB jump-attacks, landings blast, movement paints a trail. Batch 2:
//! [`weapon`] owns the trigger (through [`crate::special::weapon_hook`]) and
//! [`tick`] runs the hop / landing-blast / trail ladder with the match's
//! context (roster, paint, projectiles, subs). The prop mesh (`propMesh`,
//! L1416-1419), the sounds (L1420/L1432/L1438/L1446), the FX / shake / rumble
//! (L1421/L1445/L1447/L1450) and the body pose/squash (L1457-1462) are
//! presentation. `GHOST.kraken` (L2056-2065) has only a visual `tick`, so a
//! ghost start falls back to `IMPL.kraken.start` (L282) and the ghost never
//! runs [`tick`] (the match only calls it for non-ghost actives).

use glam::{Vec3, vec3};

use crate::actor::{Actor, ActorEvent, SpecialActive};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::special::specials;
use crate::sub::SubSystem;
use crate::tuning::PlayerTuning;
use crate::weapon::{ProjectileSim, Rng};

/// JS L1412: the ground acceleration override (`accel: 45`) — hardcoded in
/// JS, not tuning (D-T35-4).
const ACCEL: f32 = 45.0;
/// JS L1431: the air-borne jump-attack dive speed (`min(vel.y, -16)`) —
/// hardcoded in JS (D-T35-4).
const DIVE_VEL: f32 = 16.0;
/// JS L1438: the hop gate (`vel.y > 3`) — hardcoded in JS (D-T35-4).
const HOP_GATE: f32 = 3.0;
/// JS L1454-1455: the trail distance threshold — hardcoded in JS (D-T35-4).
const TRAIL_STEP: f32 = 0.5;
/// JS L1444: the landing splat height offset — hardcoded in JS (D-T35-4).
const LAND_PAINT_UP: f32 = 0.3;
/// JS L1449: the blast centre offset — hardcoded in JS (D-T35-4).
const BLAST_UP: f32 = 0.4;
/// JS L1455: the trail splat height offset — hardcoded in JS (D-T35-4).
const TRAIL_UP: f32 = 0.2;

/// Kind state (JS `s.cd`, `s.attack`, `s.trail`, `s.air`, L1412).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.cd` (L1412): seconds until the next jump-attack.
    pub cd: f32,
    /// JS `s.attack` (L1412/1426): a jump-attack is airborne.
    pub attack: bool,
    /// JS `s.trail` (L1412/1454): accumulated distance for the paint trail.
    pub trail: f32,
    /// JS `s.air` (L1412/1440): grounded→airborne edge tracking.
    pub air: bool,
}

/// JS `IMPL.kraken.start` (L1411-1422): the behaviour flags (L1412) + the
/// forced kid form (L1413). `a.weaponRunner.reset()` (L1414) is the match's
/// half (the runner lives on the match, not the actor) — run on the live
/// `special:use` peek. The prop / mesh / FX (L1415-1421) are presentation.
pub fn start(a: &mut Actor, s: &mut SpecialActive) {
    s.no_squid = true;
    s.ink_proof = true;
    s.speed = Some(specials().kraken.speed);
    s.accel = Some(ACCEL); // JS L1412 (D-T35-4)
    s.face_move = true;
    a.form = crate::actor::Form::Kid; // JS L1413
    s.state = crate::special::KindState::Kraken(State::default());
}

/// JS `IMPL.kraken.weapon(a, s, dt, inp)` (L1423-1435): the jump-attack owns
/// the trigger (always returns `true`, so the weapon runner is skipped —
/// [`crate::special::weapon_hook`]). The `cd` decays even when nothing fires
/// (L1424).
pub fn weapon(a: &mut Actor, s: &mut SpecialActive, dt: f32) -> bool {
    let Some(st) = (match &mut s.state {
        crate::special::KindState::Kraken(st) => Some(st),
        _ => None,
    }) else {
        return true;
    };
    let d = &specials().kraken;
    st.cd -= dt; // JS L1424
    if a.fire_gate.pressed && st.cd <= 0.0 {
        // JS L1425-1426: `inp.firePressed && s.cd <= 0`.
        st.cd = d.cooldown;
        st.attack = true;
        if a.grounded {
            // JS L1428-1430: a hop forward along the aim (`_fwd`, L652 =
            // `(sin(aimYaw), 0, cos(aimYaw))`) at half `attackFwd`, plus the
            // full `attackVel` vertical launch.
            let f = Vec3::new(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
            a.vel.x += f.x * d.attack_fwd * 0.5;
            a.vel.z += f.z * d.attack_fwd * 0.5;
            a.vel.y = d.attack_vel;
            a.grounded = false;
        } else {
            // JS L1431: dive onto whoever is below.
            a.vel.y = a.vel.y.min(-DIVE_VEL);
        }
    }
    true // JS L1434
}

/// JS `IMPL.kraken.tick(a, s, dt)` (L1436-1463): the hop boost (L1438), the
/// landing blast (L1439-1451) and the movement ink trail (L1452-1456). The
/// match runs this for the *live* kraken only, with the roster / paint /
/// projectile / sub context. The visual body pose (L1457-1462) is
/// presentation.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    rng: &mut Rng,
    ps: &mut ProjectileSim,
    actors: &mut [Actor],
    idx: usize,
    dt: f32,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    subs: &mut SubSystem,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    let d = &specials().kraken;
    // Phase 1: the state machine on the actor's own fields (disjoint field
    // borrows — `s.state` alongside `a.vel` / `a.grounded` / `a.pos`).
    let (team, slot, slam, trail) = {
        let a = &mut actors[idx];
        let Some(s) = a.special_active.as_mut() else {
            return;
        };
        if s.ghost || s.ended || s.kind != "kraken" {
            return;
        }
        let Some(st) = (match &mut s.state {
            crate::special::KindState::Kraken(st) => Some(st),
            _ => None,
        }) else {
            return;
        };
        // JS L1438: a rising hop that is not a jump-attack gets the big
        // `hopVel` boost on the airborne edge.
        if !a.grounded && !st.air && a.vel.y > HOP_GATE && !st.attack {
            a.vel.y = a.vel.y.max(d.hop_vel);
        }
        let landed = st.air && a.grounded; // JS L1439
        st.air = !a.grounded; // JS L1440
        let mut slam = None;
        if landed && st.attack {
            // JS L1441-1451: the jump-attack landed.
            st.attack = false;
            slam = Some(a.pos);
        }
        let mut trail = false;
        if a.grounded {
            // JS L1453-1456: the trail accrues ground speed.
            st.trail += a.vel.x.hypot(a.vel.z) * dt;
            if st.trail > TRAIL_STEP {
                st.trail = 0.0;
                trail = true;
            }
        }
        (a.team, a.slot, slam, trail)
    };
    // Phase 2: paint / events / blast through the whole roster.
    if let Some(c) = slam {
        // JS L1444: `paint(a, c + 0.3y, radius * 0.8, team)` — one seed
        // draw, credited through `addTurfNoSpecial`.
        let seed = rng.next_f32();
        let area = paint.splat(
            collision,
            vec3(c.x, c.y + LAND_PAINT_UP, c.z),
            d.radius * 0.8,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        actors[idx].add_turf_no_special(area);
        // JS L1448: `emit('special:slam', { actor, pos, radius })` — the
        // FORWARD bus marker (netmatch.js L47).
        actors[idx].push_event(ActorEvent::SpecialSlam {
            pos: c,
            radius: d.radius,
        });
        // JS L1449: `blast(a, team, c + 0.4y, radius, damage, damage * 0.6,
        // 'kraken', radius * 0.7)`.
        crate::special::blast(
            ps.events_mut(),
            subs,
            actors,
            collision,
            slot,
            team,
            vec3(c.x, c.y + BLAST_UP, c.z),
            d.radius,
            d.damage,
            d.damage * 0.6,
            d.radius * 0.7,
            remote,
            t,
            false,
        );
    }
    if trail {
        // JS L1455: `paint(a, a.pos + 0.2y, paintRadius, team)` — one seed
        // draw.
        let p = actors[idx].pos;
        let seed = rng.next_f32();
        let area = paint.splat(
            collision,
            vec3(p.x, p.y + TRAIL_UP, p.z),
            d.paint_radius,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        actors[idx].add_turf_no_special(area);
    }
}

/// JS `IMPL.kraken.end` (L1464-1468): mesh removal + the exit explosion are
/// presentation.
pub fn end(_a: &mut Actor, _reason: &str) {}
