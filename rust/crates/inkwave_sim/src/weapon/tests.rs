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
    w: WeaponSet,
}

fn fix() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.weapons,
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
            self.runners[0].update(DT, &gate, s, WeaponKind::Shooter, &self.f.w, &mut self.ps);
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
    assert_eq!(f.w.shooter.ink_per_shot, 0.95);

    let mut fired = 0usize;
    for _ in 0..12 * 60 {
        runner.update(
            DT,
            &FireGate {
                fire: true,
                ..Default::default()
            },
            &mut a,
            WeaponKind::Shooter,
            &f.w,
            &mut ps,
        );
        fired += ps
            .drain_events()
            .into_iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
    }
    let want = (f.t.ink_max / f.w.shooter.ink_per_shot).floor() as usize; // 105
    assert_eq!(fired, want, "shots until the tank is dry");
    assert!(a.ink < f.w.shooter.ink_per_shot, "reservoir below one shot");
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
            WeaponKind::Shooter,
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
    ps.fire_shooter(&a, &f.w.shooter, 0.0, None); // spread suppressed for the trace
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
    let mut ref_vel = vec3(0.0, 0.0, f.w.shooter.proj_speed);
    let straight = f.w.shooter.straight_time;
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
                (ref_pos.y - y0).abs() < MM
                    && (ref_pos.z - z0 - f.w.shooter.proj_speed * DT).abs() < MM,
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
    let cutoff = (f.w.shooter.range / f.w.shooter.proj_speed / DT).floor() as usize; // 22
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

    ps.fire_shooter(&a, &f.w.shooter, 0.0, None);
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
    let s0 = r.spread_deg(true, WeaponKind::Shooter, w);
    assert!((s0 - w.shooter.spread_ground * SPREAD_FIRST).abs() < MM);
    // Air cone is wider than the ground cone.
    assert!(r.spread_deg(false, WeaponKind::Shooter, w) > s0);
    // Fully bloomed: base × 1.
    r.bloom = 1.0;
    assert!((r.spread_deg(true, WeaponKind::Shooter, w) - w.shooter.spread_ground).abs() < MM);
    // Decay: released trigger recovers over bloomRecover seconds.
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(3);
    r.update(
        DT,
        &FireGate::default(),
        &mut a,
        WeaponKind::Shooter,
        w,
        &mut ps,
    );
    assert!(r.bloom < 1.0);
}

#[test]
fn runner_move_speed_uses_firing_penalty() {
    let f = fix();
    let mut r = WeaponRunner::new();
    assert_eq!(r.move_speed(WeaponKind::Shooter, &f.t, &f.w), f.t.run_speed);
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
        WeaponKind::Shooter,
        &f.w,
        &mut ps,
    );
    assert_eq!(
        r.move_speed(WeaponKind::Shooter, &f.t, &f.w),
        f.w.shooter.move_speed_firing
    );
}

// ------------------------------------------------------------------ TR-29.2
// Generalized projectile system (Task 29): the `RoundSpec` paths — straight /
// parabola / spread / trail-drip — each pinned to a fixed seed.

/// A mid-air deck-free launch: actor at `y`, level aim (+Z), custom spec.
fn airborne_actor(f: &Fix, y: f32) -> Actor {
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, y, 0.0), 0.0, &f.sim(), &f.t);
    a.aim_pitch = 0.0;
    a
}

#[test]
fn tr29_2_fire_round_straight_then_parabola_and_life_cutoff() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let a = airborne_actor(&f, 8.0); // high above the deck: no world hit
    let spec = RoundSpec {
        proj_speed: 20.0,
        straight_time: 0.15,
        range: 10.4, // life = min(10.4/20, 1.0) = 0.52 s
        grav: 20.0,
        drag: 0.0,
        size: 0.2,
        life: 1.0,
        impact_radius: 0.5,
        damage: 0.0,
        trail_every: 0.0,
        trail_radius: 0.0,
    };
    let mut ps = ProjectileSim::new(1234);
    ps.fire_round(&a, &spec, 0.0, None);
    assert_eq!(ps.list.len(), 1);
    let life = (spec.range / spec.proj_speed).min(spec.life);
    assert!((life - 0.52).abs() < 1e-5, "life = range/speed, capped");
    assert_eq!(ps.list[0].size, spec.size, "per-round hit radius");
    let start = ps.list[0].pos;

    // Reference integrator: straight until `straight_time`, then pure
    // parabola (drag 0). Gravity only after the straight phase.
    let mut ref_pos = start;
    let mut ref_vel = vec3(0.0, 0.0, spec.proj_speed);
    let mut age = 0.0f32;
    let mut max_err = 0.0f32;
    let mut gone_at = None;
    for step in 0..60 {
        age += DT;
        if age > spec.straight_time {
            ref_vel.y -= spec.grav * DT;
        }
        ref_pos += ref_vel * DT;
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t);
        if let Some(p) = ps.list.first() {
            max_err = max_err.max(p.pos.distance(ref_pos));
            if age <= spec.straight_time {
                assert!((p.pos.y - start.y).abs() < MM, "no drop in straight phase");
            }
        } else {
            gone_at = Some(step);
            break;
        }
    }
    // Dies on the first step whose age exceeds life = 0.52 s: floor(0.52/DT)
    // = 31 surviving steps, gone at index 31 ((32 × DT = 0.533 > 0.52)).
    assert_eq!(gone_at, Some(31), "expiry at the life cap");
    assert!(
        max_err < 1e-2,
        "generalized flight within 1 cm of the reference: {max_err}"
    );
    assert_eq!(pg.coverage()[0], 0.0, "no trail spec ⇒ no drips");
}

#[test]
fn tr29_2_spread_cone_is_bounded_and_seed_deterministic() {
    let f = fix();
    let a = airborne_actor(&f, 8.0);
    let spec = RoundSpec::shooter(&f.w.shooter);
    let spread = 6.0f32;

    let fire = |seed: u64| -> Vec<Vec3> {
        let mut ps = ProjectileSim::new(seed);
        let mut vels = Vec::new();
        for _ in 0..8 {
            ps.fire_round(&a, &spec, spread, None);
        }
        for p in &ps.list {
            vels.push(p.vel);
        }
        vels
    };

    let vels = fire(42);
    assert_eq!(vels.len(), 8);
    let mut max_ang = 0.0f32;
    for v in &vels {
        let ang = v.normalize().angle_between(Vec3::Z).to_degrees();
        assert!(ang <= spread + 1e-2, "cone breach: {ang}° > {spread}°");
        max_ang = max_ang.max(ang);
    }
    assert!(max_ang > 0.5, "spread must actually deflect: {max_ang}°");

    // Same seed ⇒ byte-identical deflection stream (AC-6 determinism).
    assert_eq!(fire(42), vels, "fixed seed must replay exactly");
    assert_ne!(fire(43), vels, "a different seed must differ");
}

#[test]
fn tr29_2_trail_drips_paint_the_deck_under_a_fixed_seed() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let a = airborne_actor(&f, 2.0); // muzzle ≈ 3.05 m: inside the 4 m drip ray
    let spec = RoundSpec {
        proj_speed: 20.0,
        straight_time: 0.2,
        range: 8.0, // life = 0.4 s ⇒ ≈ 7.5 m of travel
        grav: 28.0,
        drag: 0.8,
        size: PROJ_SIZE,
        life: 1.2,
        impact_radius: 0.5,
        damage: 0.0,
        trail_every: 0.8,
        trail_radius: 0.3,
    };
    let mut ps = ProjectileSim::new(777);
    ps.fire_round(&a, &spec, 0.0, None);

    let mut drips: Vec<(f32, f32)> = Vec::new(); // (z, radius) per ink splat
    let mut impacts = 0usize;
    let mut turf = 0usize;
    for _ in 0..40 {
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t);
        for e in ps.drain_events() {
            match e {
                SimEvent::Impact { .. } => impacts += 1,
                SimEvent::Turf { .. } => turf += 1,
                _ => {}
            }
        }
        for s in pg.drain_ink() {
            drips.push((s.center.z, s.radius));
        }
        if ps.list.is_empty() {
            break;
        }
    }
    // The round expires in mid-air (life cap), never hits the deck.
    assert_eq!(impacts, 0, "no world impact — the drips are the only paint");
    assert!(drips.len() >= 5, "expected ≈7 drips, got {}", drips.len());
    assert!(turf >= 3, "drips credit turf via `Turf` events: {turf}");
    assert!(
        drips.windows(2).all(|w| w[0].0 < w[1].0),
        "drips advance along +Z"
    );
    assert!(
        drips
            .iter()
            .all(|(_, r)| (0.3 * 0.8 - 1e-3..=0.3 * 1.2 + 1e-3).contains(r)),
        "drip radius = trailRadius × (0.8 + rand·0.4): {drips:?}"
    );
    assert!(pg.coverage()[0] > 0.0, "the trail paints turf");

    // Determinism: same seed ⇒ same drip sequence.
    let mut pg2 = PaintGrid::new(&f.world);
    let mut ps2 = ProjectileSim::new(777);
    ps2.fire_round(&a, &spec, 0.0, None);
    let mut drips2: Vec<(f32, f32)> = Vec::new();
    for _ in 0..40 {
        ps2.step(DT, &f.world, &mut [], &mut pg2, &f.t);
        let _ = ps2.drain_events();
        for s in pg2.drain_ink() {
            drips2.push((s.center.z, s.radius));
        }
        if ps2.list.is_empty() {
            break;
        }
    }
    assert_eq!(drips, drips2, "fixed seed replays the drip train exactly");
}

#[test]
fn tr29_2_kind_table_and_dispatch_skeleton() {
    let f = fix();
    let t = crate::embedded_tuning();

    // The enum covers the 12 roster kinds in WEAPON_ORDER order; retired
    // kinds never parse (TR-28.2).
    assert_eq!(WeaponKind::ALL.len(), 12);
    assert_eq!(t.weapon_order.len(), 12);
    for (id, k) in t.weapon_order.iter().zip(WeaponKind::ALL) {
        assert_eq!(WeaponKind::from_kind_str(id), Some(k), "order drift: {id}");
    }
    assert_eq!(WeaponKind::from_kind_str("dualies"), None);
    assert!(WeaponKind::Shooter.is_wired());
    assert!(!WeaponKind::Blaster.is_wired());

    // Move-speed table (weapons.js L63-92): idle is always the run speed,
    // the firing states read the def, the kit kinds stay at run speed.
    let mut r = WeaponRunner::new();
    for k in WeaponKind::ALL {
        assert_eq!(r.move_speed(k, &f.t, &f.w), f.t.run_speed, "idle {k:?}");
    }
    r.firing_t = 0.1;
    assert_eq!(
        r.move_speed(WeaponKind::Shooter, &f.t, &f.w),
        f.w.shooter.move_speed_firing
    );
    assert_eq!(
        r.move_speed(WeaponKind::Blaster, &f.t, &f.w),
        f.w.blaster.move_speed_firing
    );
    assert_eq!(
        r.move_speed(WeaponKind::Twins, &f.t, &f.w),
        f.w.twins.move_speed_firing
    );
    assert_eq!(
        r.move_speed(WeaponKind::Brush, &f.t, &f.w),
        f.w.brush.move_speed_firing
    );
    for k in [
        WeaponKind::Brolly,
        WeaponKind::Bow,
        WeaponKind::Blade,
        WeaponKind::Mitts,
    ] {
        assert_eq!(r.move_speed(k, &f.t, &f.w), f.t.run_speed, "kit {k:?}");
    }
    r.firing_t = 0.0;
    r.charging = true; // charge 0 → the lerp floor
    assert!((r.move_speed(WeaponKind::Charger, &f.t, &f.w) - f.t.run_speed * 0.7).abs() < MM);
    assert!(
        (r.move_speed(WeaponKind::Spinner, &f.t, &f.w) - f.t.run_speed * 0.75).abs() < MM,
        "spinner charge floor = run×0.75"
    );
    r.charging = false;
    r.burst_t = 0.1;
    assert_eq!(
        r.move_speed(WeaponKind::Spinner, &f.t, &f.w),
        f.w.spinner.move_speed_firing
    );
    r.burst_t = 0.0;
    r.flick = 0.0;
    assert!(
        (r.move_speed(WeaponKind::Bucket, &f.t, &f.w) - f.w.bucket.move_speed_firing * 0.7).abs()
            < MM,
        "bucket flick = firing×0.7 (JS L71)"
    );
    // Roller flick lerp: at `flick = 0` (t = 0) the blend floor is the plain
    // firing speed; at the full windup it reaches ×0.45 (JS L86).
    assert!(
        (r.move_speed(WeaponKind::Roller, &f.t, &f.w) - f.w.roller.move_speed_firing).abs() < MM,
        "roller flick start (t = 0)"
    );
    r.flick = f.w.roller.flick_windup;
    assert!(
        (r.move_speed(WeaponKind::Roller, &f.t, &f.w) - f.w.roller.move_speed_firing * 0.45).abs()
            < MM,
        "roller flick full windup → ×0.45 (JS L86)"
    );

    // Spread table (weapons.js L94-107).
    let r = WeaponRunner::new();
    assert!((r.spread_deg(true, WeaponKind::Blaster, &f.w) - 1.2).abs() < MM);
    assert!((r.spread_deg(false, WeaponKind::Blaster, &f.w) - 4.0).abs() < MM);
    assert!((r.spread_deg(true, WeaponKind::Spinner, &f.w) - f.w.spinner.spread_ground).abs() < MM);
    assert!((r.spread_deg(false, WeaponKind::Spinner, &f.w) - f.w.spinner.spread_air).abs() < MM);
    assert!(
        (r.spread_deg(true, WeaponKind::Twins, &f.w) - f.w.twins.spread_ground * 0.5).abs() < MM
    );
    for k in [
        WeaponKind::Roller,
        WeaponKind::Charger,
        WeaponKind::Bucket,
        WeaponKind::Brush,
        WeaponKind::Brolly,
        WeaponKind::Bow,
        WeaponKind::Blade,
        WeaponKind::Mitts,
    ] {
        assert_eq!(r.spread_deg(true, k, &f.w), 0.0, "spread {k:?}");
    }

    // busy() / MAX_ROUNDS scaffold.
    let mut r = WeaponRunner::new();
    assert!(!r.busy());
    r.charging = true;
    assert!(r.busy());
    r.charging = false;
    r.lock_t = 0.1;
    assert!(r.busy());
    assert_eq!(MAX_ROUNDS, 700);

    // Unwired kinds: the dispatch arm is a strict no-op — no ink spent, no
    // round, no event (the blaster ammo path lands with Task 30).
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(5);
    let mut br = WeaponRunner::new();
    let ink0 = a.ink;
    for _ in 0..30 {
        br.update(
            DT,
            &FireGate {
                fire: true,
                ..Default::default()
            },
            &mut a,
            WeaponKind::Blaster,
            &f.w,
            &mut ps,
        );
    }
    assert_eq!(a.ink, ink0, "blaster arm must not consume ink yet");
    assert!(
        ps.list.is_empty(),
        "blaster arm fires nothing until Task 30"
    );
    assert!(ps.drain_events().is_empty(), "no events from a no-op arm");
}
