//! Task 7 tests: Spritzer fire loop, projectile flight, hits and paint.

use super::*;
use crate::actor::{Actor, ActorInput, FIXED_DT, FireGate, InkQuery, SimWorld};
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
use crate::paint::PaintGrid;
use crate::tuning::PlayerTuning;
use glam::{Vec3, vec3};

const DT: f32 = FIXED_DT;
const MM: f32 = 1e-3;

fn flags() -> BrushCommon {
    BrushCommon {
        tag: None,
        color: "#dddddd".into(),
        pattern: 0,
        paint: true,
        solid: true,
        grate: false,
        rail: false,
        roof: false,
        perch: false,
        no_nav: false,
        hidden: false,
        bevel: None,
        no_paint: Vec::new(),
        mural: Vec::<Mural>::new(),
        oct: None,
    }
}

fn bx(min: [f32; 3], max: [f32; 3]) -> Brush {
    Brush::Box {
        min,
        max,
        common: flags(),
    }
}

const B: Bounds = Bounds {
    min_x: -30.0,
    max_x: 30.0,
    min_z: -30.0,
    max_z: 30.0,
};

/// Flat 20×20 m deck at y = 0 (turf), spawn pads off to the sides.
struct Fix {
    world: CollisionWorld,
    pads: [Vec3; 2],
    t: PlayerTuning,
    w: Spritzer,
}

fn fix() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.spritzer,
    }
}

impl Fix {
    fn sim(&self) -> SimWorld<'_> {
        SimWorld {
            collision: &self.world,
            spawn_pads: self.pads,
            spawn_barrier: 4.2,
        }
    }
}

struct ZeroInk;
impl InkQuery for ZeroInk {
    fn sample(&self, _face: i32, _u: f32, _v: f32) -> u8 {
        0
    }
}

fn idle() -> ActorInput {
    ActorInput::default()
}

fn hold_fire() -> ActorInput {
    ActorInput {
        fire: true,
        ..ActorInput::default()
    }
}

/// A two-actor scenario wired through the full step pipeline (actor physics →
/// weapon runner → projectile sim), so the TRs exercise the real integration.
struct Sim {
    f: Fix,
    actors: Vec<Actor>,
    runners: Vec<WeaponRunner>,
    paint: PaintGrid,
    ps: ProjectileSim,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let f = fix();
        let mut shooter = Actor::new(0, 0, &f.t);
        shooter.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t); // aim +Z
        let mut victim = Actor::new(1, 4, &f.t);
        victim.spawn_at(vec3(0.0, 0.05, 3.0), std::f32::consts::PI, &f.sim(), &f.t);
        victim.set_invuln(0.0);
        let paint = PaintGrid::new(&f.world);
        Self {
            f,
            actors: vec![shooter, victim],
            runners: vec![WeaponRunner::new(), WeaponRunner::new()],
            paint,
            ps: ProjectileSim::new(seed),
        }
    }

    /// One full fixed step: both actors step (slot 0 uses `inp`, slot 1 idles),
    /// slot 0's runner fires, then the projectile sim advances.
    fn step(&mut self, inp: &ActorInput) {
        let zero = ZeroInk;
        // Split the actor array so the runner can take `&mut actors[0]` while
        // `actors[1]` steps independently; the split borrows end before the
        // projectile step re-borrows the whole slice.
        {
            let sim = self.f.sim();
            let [s, v] = self.actors.as_mut_slice() else {
                unreachable!("two actors");
            };
            s.step(DT, inp, &sim, &zero, &self.f.t);
            v.step(DT, &idle(), &sim, &zero, &self.f.t);
            let gate = s.fire_gate;
            self.runners[0].update(DT, &gate, s, &self.f.w, &mut self.ps);
        }
        self.ps.step(
            DT,
            &self.f.world,
            &mut self.actors,
            &mut self.paint,
            &self.f.t,
        );
    }
}

// ------------------------------------------------------------------ TR-7.1
// Close-range Spritzer: 3 hits = 108 damage → splat + respawn; the next shot
// inside the post-respawn invulnerability window deals no damage.
#[test]
fn tr7_1_three_hits_kill_and_invuln_blocks_the_fourth() {
    let mut s = Sim::new(0x1234_5678);
    let mut hits = 0usize;
    let mut killed_step = None;
    for step in 0..40 {
        s.step(&hold_fire());
        for e in s.ps.drain_events() {
            if let SimEvent::Hit {
                victim_slot,
                damage,
                killed,
                ..
            } = e
            {
                assert_eq!(victim_slot, 4);
                assert_eq!(damage, 36.0);
                if killed {
                    hits += 1;
                    killed_step = Some(step);
                } else {
                    hits += 1;
                }
            }
        }
        if killed_step.is_some() {
            break;
        }
    }
    assert_eq!(hits, 3, "exactly three hits to splat (3 × 36 ≥ 100)");
    assert!(killed_step.is_some(), "third hit must splat the victim");
    assert!(!s.actors[1].alive);
    assert_eq!(s.actors[1].hp, 0.0);
    assert_eq!(s.actors[1].deaths, 1);
    assert!(
        s.actors[1].respawn_timer > 0.0 && s.actors[1].respawn_timer <= s.f.t.respawn_time,
        "respawn timer armed"
    );

    // Wait out the respawn (5.5 s) without firing.
    for _ in 0..(s.f.t.respawn_time as usize * 60 + 30) {
        s.step(&idle());
        let _ = s.ps.drain_events();
    }
    assert!(s.actors[1].alive, "victim respawned after the timer");
    assert!(
        s.actors[1].invuln() > 0.0,
        "spawn invulnerability active after respawn"
    );

    // Put the victim back in the line of fire and take more shots during the
    // invulnerable window: hits land but deal no damage.
    s.actors[1].pos = vec3(0.0, 0.05, 3.0);
    s.actors[1].vel = Vec3::ZERO;
    s.runners[0].reset();
    let hp_before = s.actors[1].hp;
    let mut blocked_hits = 0usize;
    for _ in 0..12 {
        s.step(&hold_fire());
        for e in s.ps.drain_events() {
            if let SimEvent::Hit { killed, damage, .. } = e {
                assert!(!killed, "invulnerable victim must not be killed");
                assert_eq!(damage, 36.0);
                blocked_hits += 1;
            }
        }
    }
    assert!(blocked_hits >= 1, "shots still land as (blocked) hits");
    assert_eq!(s.actors[1].hp, hp_before, "no damage inside invuln");
    assert!(s.actors[1].alive);
    println!(
        "TR-7.1: 3 hits kill at step {killed_step:?}; {blocked_hits} blocked hits during invuln"
    );
}

// ------------------------------------------------------------------ TR-7.2
// Ink gating: below `inkPerShot` the gun clicks empty; sustained fire from a
// full tank produces exactly floor(100 / 0.95) = 105 rounds.
#[test]
fn tr7_2_ink_gating_and_shots_to_empty() {
    let f = fix();
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(7);
    let mut runner = WeaponRunner::new();
    assert_eq!(a.ink, f.t.ink_max);
    assert_eq!(f.w.ink_per_shot, 0.95);

    let mut fired = 0usize;
    for _ in 0..12 * 60 {
        runner.update(
            DT,
            &FireGate {
                fire: true,
                ..Default::default()
            },
            &mut a,
            &f.w,
            &mut ps,
        );
        fired += ps
            .drain_events()
            .into_iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
    }
    let want = (f.t.ink_max / f.w.ink_per_shot).floor() as usize; // 105
    assert_eq!(fired, want, "shots until the tank is dry");
    assert!(a.ink < f.w.ink_per_shot, "reservoir below one shot");
    assert_eq!(
        ps.list.len(),
        want,
        "one round per shot, none expired (no step)"
    );

    // Below the cost: no round leaves the muzzle, an empty-click fires.
    let mut low = 0usize;
    for _ in 0..60 {
        runner.update(
            DT,
            &FireGate {
                fire: true,
                ..Default::default()
            },
            &mut a,
            &f.w,
            &mut ps,
        );
        low += ps
            .drain_events()
            .into_iter()
            .filter(|e| matches!(e, SimEvent::LowInk { .. }))
            .count();
    }
    assert_eq!(ps.list.len(), want, "no extra round while below inkPerShot");
    assert!(low >= 1, "empty-click event emitted");
    println!(
        "TR-7.2: {fired} shots, ink left = {:.2}, lowink clicks = {low}",
        a.ink
    );
}

// ------------------------------------------------------------------ TR-7.3
// Ballistics: dead straight until `straightTime`, then the upstream gravity
// integrator; the round expires at `range / projSpeed`.
//
// NOTE (spec discrepancy): TR-7.3 text says "g=25", but upstream
// `fireShooter` hardcodes `grav: 28` for shooter rounds (weapons.js L1034);
// 25 is the *player* gravity. The port follows upstream (AC-6 equivalence is
// the gate); the reference integrator here uses 28 accordingly.
#[test]
fn tr7_3_straight_phase_then_parabola_and_range_cutoff() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 5.0, 0.0), 0.0, &f.sim(), &f.t); // level shot, high above the deck
    a.aim_pitch = 0.0;
    let mut ps = ProjectileSim::new(99);
    ps.fire_shooter(&a, &f.w, 0.0, None); // spread suppressed for the trace
    assert_eq!(ps.list.len(), 1);
    let start = ps.list[0].pos;
    assert!((start.y - (5.0 + MUZZLE_EYE_Y)).abs() < MM);

    // Reference integrator (JS `_step` order: gravity/drag then position).
    // The gravity/drag values are hardcoded from upstream `fireShooter`
    // (weapons.js L1034: grav 28, drag 0.8), NOT the implementation's
    // constants — so a wrong constant in weapon.rs fails this cross-check.
    const REF_GRAV: f32 = 28.0;
    const REF_DRAG: f32 = 0.8;
    let mut ref_pos = start;
    let mut ref_vel = vec3(0.0, 0.0, f.w.proj_speed);
    let straight = f.w.straight_time;
    let mut age = 0.0f32;
    let mut max_err = 0.0f32;
    let mut straight_steps = 0usize;
    let mut gone_at = None;
    for step in 0..60 {
        let (y0, z0) = (ref_pos.y, ref_pos.z);
        age += DT;
        if age > straight {
            ref_vel.y -= REF_GRAV * DT;
            let k = 1.0 - REF_DRAG * DT;
            ref_vel *= k;
        }
        ref_pos += ref_vel * DT;
        if age <= straight {
            assert!(
                (ref_pos.y - y0).abs() < MM && (ref_pos.z - z0 - f.w.proj_speed * DT).abs() < MM,
                "straight phase must be exactly linear at step {step}"
            );
            straight_steps += 1;
        }
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t);
        if let Some(p) = ps.list.first() {
            max_err = max_err.max(p.pos.distance(ref_pos));
            if age <= straight {
                assert!(
                    (p.pos.y - start.y).abs() < MM,
                    "no drop before straightTime ends (step {step})"
                );
            }
        } else {
            gone_at = Some(step);
            break;
        }
    }
    // The round dies on the first step whose accumulated age exceeds
    // `life = range / projSpeed` (0.3676 s): floor(0.3676 / DT) = 22 full
    // steps survive, so it's gone on the 23rd `ps.step` (0-based index 22).
    let cutoff = (f.w.range / f.w.proj_speed / DT).floor() as usize; // 22
    println!(
        "TR-7.3: straight {straight_steps} steps ({:.3}s), expired at step {gone_at:?} (cutoff {cutoff}), max err vs g={REF_GRAV} reference = {:.5} m",
        straight_steps as f32 * DT,
        max_err
    );
    assert!((7..=8).contains(&straight_steps), "0.13 s ≈ 8 steps");
    assert!(
        max_err < 1e-2,
        "flight within 1 cm of the reference: {max_err}"
    );
    assert_eq!(
        gone_at,
        Some(cutoff),
        "expiry at the life cap range/projSpeed = 0.3676 s (drag pulls the actual \
         travel to ~11.8 m, not the full 12.5 m)"
    );
    assert!(ps.list.is_empty());
}

// ------------------------------------------------------------------ TR-7.4
// A round that lands on a face stamps ink the Task 6 grid can answer.
#[test]
fn tr7_4_impact_paints_the_face_queryable_by_the_grid() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    a.aim_pitch = -std::f32::consts::FRAC_PI_2; // straight down
    let mut ps = ProjectileSim::new(5);
    assert_eq!(pg.coverage()[0], 0.0);

    ps.fire_shooter(&a, &f.w, 0.0, None);
    let mut impacts = Vec::new();
    for _ in 0..30 {
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t);
        for e in ps.drain_events() {
            if let SimEvent::Impact {
                pos,
                team,
                radius,
                victim_slot,
                ..
            } = e
            {
                assert_eq!(victim_slot, None, "deck hit is not a body hit");
                impacts.push((pos, team, radius));
            }
        }
        if ps.list.is_empty() {
            break;
        }
    }
    assert_eq!(impacts.len(), 1, "exactly one world impact");
    let (pos, team, _r) = impacts[0];
    assert_eq!(team, 0);
    assert!(pos.y.abs() < 0.2, "impact on the deck: {pos}");

    let after = pg.coverage();
    assert!(
        after[0] > 0.0,
        "impact must claim turf, coverage = {after:?}"
    );
    println!(
        "TR-7.4: coverage team0 = {:.5} ({:.2} m² of {:.2})",
        after[0],
        after[0] * pg.turf_area,
        pg.turf_area
    );

    // The grid answers ownership right at the impact point.
    let face = f
        .world
        .faces
        .iter()
        .find(|fc| fc.n.y > 0.9 && fc.turf)
        .expect("deck top face");
    assert_eq!(
        pg.sample_world(&f.world, face.id as i32, vec3(pos.x, 0.0, pos.z)),
        1,
        "impact point must read as Alpha ink"
    );
    // A far corner is still clean.
    assert_eq!(
        pg.sample_world(&f.world, face.id as i32, vec3(9.0, 0.0, 9.0)),
        0
    );
}

// ------------------------------------------------------------------ units

#[test]
fn rng_is_deterministic_and_bounded() {
    let mut a = Rng::new(42);
    let mut b = Rng::new(42);
    let xs: Vec<f32> = (0..8).map(|_| a.next_f32()).collect();
    let ys: Vec<f32> = (0..8).map(|_| b.next_f32()).collect();
    assert_eq!(xs, ys);
    assert!(xs.iter().all(|v| (0.0..1.0).contains(v)));

    // Spread cone: the deflected direction stays inside the stated angle.
    let mut r = Rng::new(1);
    for _ in 0..50 {
        let d = r.spread(Vec3::Z, 5.5);
        let ang = d.angle_between(Vec3::Z).to_degrees();
        assert!(ang <= 5.5 + 1e-2, "spread {ang} exceeds the cone");
    }
    assert_eq!(r.spread(Vec3::Z, 0.0), Vec3::Z);
}

#[test]
fn capsule_distance_matches_analytic_values() {
    let base = Vec3::ZERO;
    // Point level with the axis → pure horizontal distance.
    assert!((point_capsule_dist(vec3(1.0, 0.7, 0.0), base, 0.38, 1.45) - 1.0).abs() < MM);
    // Below the bottom cap → distance to the sphere at (0, r, 0).
    let d = point_capsule_dist(vec3(0.0, -1.0, 0.0), base, 0.38, 1.45);
    assert!((d - (1.0 + 0.38)).abs() < MM);
    // A segment straight through the middle hits near t = 0.5, dist ≈ 0. The
    // 6-sample + ternary refine (JS `segmentCapsuleDist`) converges to ~3 mm
    // on a 2 m segment — far inside the hit threshold (≈ 0.5 m), so the
    // tolerance reflects the algorithm's resolution, not a port error.
    let res = segment_capsule_dist(vec3(-1.0, 0.7, 0.0), vec3(1.0, 0.7, 0.0), base, 0.38, 1.45);
    assert!(res.dist < 0.01 && (res.t - 0.5).abs() < 0.05, "{res:?}");
}

#[test]
fn ballistic_is_a_noop_outside_its_envelope() {
    let m = Vec3::ZERO;
    let dir = Vec3::Z;
    // Too close (hd < 1.5): unchanged.
    assert_eq!(
        ballistic(m, dir, vec3(0.0, 0.0, 1.0), 34.0, 0.13, 28.0, 0.8, 12.5),
        dir
    );
    // Beyond maxDist: unchanged.
    assert_eq!(
        ballistic(m, dir, vec3(0.0, 0.0, 20.0), 34.0, 0.13, 28.0, 0.8, 12.5),
        dir
    );
    // A target 6 m ahead at muzzle height needs a small upward correction; the
    // result is a unit vector pointing forward.
    let out = ballistic(m, dir, vec3(0.0, 0.0, 6.0), 34.0, 0.13, 28.0, 0.8, 12.5);
    assert!((out.length() - 1.0).abs() < 1e-3);
    assert!(out.z > 0.9);
}

#[test]
fn runner_bloom_and_spread_track_upstream_formulas() {
    let f = fix();
    let w = &f.w;
    let mut r = WeaponRunner::new();
    // First shot: base × spreadFirst.
    let s0 = r.spread_deg(true, w);
    assert!((s0 - w.spread_ground * SPREAD_FIRST).abs() < MM);
    // Air cone is wider than the ground cone.
    assert!(r.spread_deg(false, w) > s0);
    // Fully bloomed: base × 1.
    r.bloom = 1.0;
    assert!((r.spread_deg(true, w) - w.spread_ground).abs() < MM);
    // Decay: released trigger recovers over bloomRecover seconds.
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(3);
    r.update(DT, &FireGate::default(), &mut a, w, &mut ps);
    assert!(r.bloom < 1.0);
}

#[test]
fn runner_move_speed_uses_firing_penalty() {
    let f = fix();
    let mut r = WeaponRunner::new();
    assert_eq!(r.move_speed(&f.t, &f.w), f.t.run_speed);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(3);
    r.update(
        DT,
        &FireGate {
            fire: true,
            ..Default::default()
        },
        &mut a,
        &f.w,
        &mut ps,
    );
    assert_eq!(r.move_speed(&f.t, &f.w), f.w.move_speed_firing);
}
