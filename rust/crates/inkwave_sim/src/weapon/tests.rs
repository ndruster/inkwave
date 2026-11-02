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
        only_in: None,
        not_in: None,
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
    sb: crate::tuning::SubSet,
}

fn fix() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.weapons,
        sb: tuning.subs,
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
            s.step(DT, inp, &sim, &zero, &self.f.t, &self.f.sb, None, None);
            v.step(DT, &idle(), &sim, &zero, &self.f.t, &self.f.sb, None, None);
            let gate = s.fire_gate;
            self.runners[0].update(
                DT,
                &gate,
                s,
                WeaponKind::Shooter,
                &self.f.w,
                &self.f.sb,
                Vec3::ZERO,
                &self.f.world,
                &mut self.ps,
                &mut FoeScan::empty(),
                &mut self.paint,
                &self.f.t,
                None,
                false,
                0.0,
                &mut Vec::new(),
                &mut Vec::new(),
                false,
                false,
            );
        }
        self.ps.step(
            DT,
            &self.f.world,
            &mut self.actors,
            &mut self.paint,
            &self.f.t,
            &self.f.w,
            &self.f.sb,
            None,
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
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
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
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
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
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
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
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
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
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
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
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
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
        trail_start: -2.5,
        burst: None,
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
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
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
        trail_start: -(2.5 - 0.8),
        burst: None,
    };
    let mut ps = ProjectileSim::new(777);
    ps.fire_round(&a, &spec, 0.0, None);

    let mut drips: Vec<(f32, f32)> = Vec::new(); // (z, radius) per ink splat
    let mut impacts = 0usize;
    let mut turf = 0usize;
    for _ in 0..40 {
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
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
        ps2.step(DT, &f.world, &mut [], &mut pg2, &f.t, &f.w, &f.sb, None);
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
    // Task 30 batch one: twins / blaster / brolly joined the wired set.
    assert!(WeaponKind::Blaster.is_wired());
    assert!(WeaponKind::Twins.is_wired());
    assert!(WeaponKind::Brolly.is_wired());
    // Task 31 batch two: charger / spinner / bucket joined the wired set.
    assert!(WeaponKind::Charger.is_wired());
    assert!(WeaponKind::Spinner.is_wired());
    assert!(WeaponKind::Bucket.is_wired());
    // Task 32 batch three: roller / brush / blade joined the wired set.
    assert!(WeaponKind::Roller.is_wired());
    assert!(WeaponKind::Brush.is_wired());
    assert!(WeaponKind::Blade.is_wired());
    // Task 33 batch four: bow / mitts joined the wired set — all 12 kinds
    // now dispatch for real.
    assert!(WeaponKind::Bow.is_wired());
    assert!(WeaponKind::Mitts.is_wired());

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
    // Task 33: the bow / mitts kits join the firing window (bow.js L446 /
    // mitts.js L910 `moveSpeedFiring`).
    assert_eq!(
        r.move_speed(WeaponKind::Bow, &f.t, &f.w),
        f.w.bow.move_speed_firing,
        "bow firing"
    );
    assert_eq!(
        r.move_speed(WeaponKind::Mitts, &f.t, &f.w),
        f.w.mitts.move_speed_firing,
        "mitts firing"
    );
    // Task 32: the blade kit owns its curve (blade.js L683-689) — with the
    // pose window active the kit `moveSpeedFiring` applies.
    assert_eq!(
        r.move_speed(WeaponKind::Blade, &f.t, &f.w),
        f.w.blade.move_speed_firing
    );
    // Brolly kit semantics (weapons.js L664 / brolly.js): the raised canopy
    // is the slowest stance, firing next, plain run otherwise.
    r.firing_t = 0.0;
    assert_eq!(r.move_speed(WeaponKind::Brolly, &f.t, &f.w), f.t.run_speed);
    r.firing_t = 0.1;
    assert_eq!(
        r.move_speed(WeaponKind::Brolly, &f.t, &f.w),
        f.w.brolly.move_speed_firing,
        "brolly firing"
    );
    r.firing_t = 0.0;
    r.ensure_brolly(&f.w).open = 0.5;
    assert_eq!(
        r.move_speed(WeaponKind::Brolly, &f.t, &f.w),
        f.w.brolly.move_speed_shield,
        "brolly shield up (open > 0.2)"
    );
    r.brolly = None;
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
        WeaponKind::Bow,
        WeaponKind::Blade,
    ] {
        assert_eq!(r.spread_deg(true, k, &f.w), 0.0, "spread {k:?}");
    }
    // Task 33: the mitts punch cone reads the def (mitts.js L139 `spread`).
    assert_eq!(
        r.spread_deg(true, WeaponKind::Mitts, &f.w),
        f.w.mitts.punch_spread,
        "spread Mitts"
    );
    // Brolly cone (brolly.js fireBlast): ground vs air spread from the def.
    assert!((r.spread_deg(true, WeaponKind::Brolly, &f.w) - f.w.brolly.spread_deg).abs() < MM);
    assert!((r.spread_deg(false, WeaponKind::Brolly, &f.w) - f.w.brolly.spread_air).abs() < MM);

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
    // round, no event. (Task 30 wired the blaster; the charger stands in as
    // the still-unwired arm — it lands with Task 31.)
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
            WeaponKind::Charger,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
    }
    assert_eq!(a.ink, ink0, "charger arm must not consume ink yet");
    assert!(
        ps.list.is_empty(),
        "charger arm fires nothing until Task 31"
    );
    assert!(ps.drain_events().is_empty(), "no events from a no-op arm");
}

// ============================================================ Task 30: twins
// TR-30.1 — cadence / ink / bloom / damage, fixed-seed value assertions.
#[test]
fn tr30_1_twins_cadence_ink_and_bloom() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(0xABCD_0001);
    let mut r = WeaponRunner::new();
    let gate = FireGate {
        fire: true,
        ..Default::default()
    };
    let mut shots = 0usize;
    for frame in 0..60 {
        r.update(
            DT,
            &gate,
            &mut a,
            WeaponKind::Twins,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        for e in ps.drain_events() {
            if let SimEvent::Fire { .. } = e {
                shots += 1;
            }
        }
        // One shot lands on frame 0 (cooldown 0) and every 0.085 s after.
        if frame == 0 {
            assert_eq!(shots, 1);
            // D-4: the twins bloom step is the hardcoded +0.25 (JS L511).
            assert!((r.spread_deg(true, WeaponKind::Twins, &f.w) - 6.0 * 0.625).abs() < MM);
        }
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    assert_eq!(shots, 12, "60 frames at fireInterval 0.085 = 12 shots");
    assert!(
        (a.ink - (f.t.ink_max - 12.0 * f.w.twins.ink_per_shot)).abs() < MM,
        "ink spent: 12 × 0.85"
    );
    // Four shots saturate the bloom → the full ground / air cones (6° / 11°).
    assert!((r.spread_deg(true, WeaponKind::Twins, &f.w) - f.w.twins.spread_ground).abs() < MM);
    assert!((r.spread_deg(false, WeaponKind::Twins, &f.w) - f.w.twins.spread_air).abs() < MM);
    // Firing pose slows the kid to `moveSpeedFiring` (5.2).
    assert!((r.move_speed(WeaponKind::Twins, &f.t, &f.w) - f.w.twins.move_speed_firing).abs() < MM);
    // Hands alternate: the two per-hand clocks were both reset.
    assert!(r.since_hand[0] < 1.0 && r.since_hand[1] < 1.0);
}

// TR-30.1 — a twin round does `damage: 30` on a body hit.
#[test]
fn tr30_1_twins_hit_damage_30() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut victim = Actor::new(1, 4, &f.t);
    victim.spawn_at(vec3(0.0, 0.05, 3.0), std::f32::consts::PI, &f.sim(), &f.t);
    victim.set_invuln(0.0);
    let mut actors = vec![a.clone(), victim];
    let mut ps = ProjectileSim::new(99);
    ps.fire_twin(&actors[0], &f.w.twins, 0.0, -1, &f.world);
    let mut hits = Vec::new();
    for _ in 0..30 {
        ps.step(DT, &f.world, &mut actors, &mut pg, &f.t, &f.w, &f.sb, None);
        hits.extend(ps.drain_events().into_iter().filter_map(|e| match e {
            SimEvent::Hit {
                victim_slot,
                damage,
                ..
            } => Some((victim_slot, damage)),
            _ => None,
        }));
        if !hits.is_empty() {
            break;
        }
    }
    assert_eq!(hits, vec![(4usize, f.w.twins.damage)], "one 30-damage hit");
    assert!(actors[1].alive, "one twin round does not splat");
}

// TR-30.1 — the planted turret: post-roll stance fires the `turret*` group.
#[test]
fn tr30_1_twins_roll_plants_the_turret() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(7);
    let mut r = WeaponRunner::new();
    let gate_fire = FireGate {
        fire: true,
        ..Default::default()
    };
    let gate_idle = FireGate::default();
    // One normal shot first (hand-alternating group).
    r.update(
        DT,
        &gate_fire,
        &mut a,
        WeaponKind::Twins,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    let _ = ps.drain_events();
    let normal = ps.list.last().unwrap();
    assert!(
        (normal.vel.length() - f.w.twins.proj_speed).abs() < MM,
        "dual-mode round flies at projSpeed 31"
    );
    ps.list.clear();
    // Jump-tap while firing + moving → the dodge roll.
    let move_dir = vec3(0.0, 0.0, 1.0);
    assert!(
        r.try_dodge(WeaponKind::Twins, &mut a, &f.w, move_dir, true),
        "roll starts"
    );
    assert!((a.ink - (f.t.ink_max - f.w.twins.ink_per_shot - f.w.twins.roll_ink)).abs() < MM);
    // The roll velocity curve: full v0 at the start, 0.4·v0 at the end
    // (JS L552-554: v = dodgeDir · v0(1 − 0.6u), v0 = rollDist/(0.7·rollTime)).
    let v0 = f.w.twins.roll_dist / (0.7 * f.w.twins.roll_time);
    let v = r.dodge_velocity(WeaponKind::Twins, &f.w).unwrap();
    assert!((v.length() - v0).abs() < MM);
    for _ in 0..9 {
        r.update(
            DT,
            &gate_fire,
            &mut a,
            WeaponKind::Twins,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    let v_mid = r.dodge_velocity(WeaponKind::Twins, &f.w).unwrap();
    assert!((v_mid.length() - v0 * 0.7).abs() < 0.05);
    // Finish the roll (0.3 s) with the trigger held → planted.
    for _ in 0..9 {
        r.update(
            DT,
            &gate_fire,
            &mut a,
            WeaponKind::Twins,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    assert_eq!(r.dodge_t, 0.0);
    assert!(r.turret, "roll ends planted while the trigger is held");
    assert!(
        (r.spread_deg(true, WeaponKind::Twins, &f.w) - f.w.twins.turret_spread).abs() < MM,
        "planted cone is turretSpread 1.6 (JS _spreadDeg L103)"
    );
    // The planted shot: both guns at once (`side = 0`) — the turret group.
    let n_before = ps.list.len();
    r.update(
        DT,
        &gate_fire,
        &mut a,
        WeaponKind::Twins,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    let planted = ps.list.last().unwrap();
    assert!(ps.list.len() > n_before);
    assert!(
        (planted.vel.length() - f.w.twins.turret_proj_speed).abs() < MM,
        "turret round flies at 36"
    );
    assert!(planted.straight >= f.w.twins.turret_straight - MM);
    // Walking un-plants the stance.
    r.update(
        DT,
        &gate_fire,
        &mut a,
        WeaponKind::Twins,
        &f.w,
        &f.sb,
        vec3(1.0, 0.0, 0.0),
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    assert!(!r.turret, "walking un-plants");
    // Releasing the trigger also un-plants.
    r.turret = true;
    r.update(
        DT,
        &gate_idle,
        &mut a,
        WeaponKind::Twins,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    assert!(!r.turret, "releasing the trigger un-plants");
}

// TR-30.1 — the roll chain: two charges, then the lockout, then the refill.
#[test]
fn tr30_1_twins_roll_chain_lockout_and_refill() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(3);
    let mut r = WeaponRunner::new();
    let gate = FireGate {
        fire: true,
        ..Default::default()
    };
    let md = vec3(1.0, 0.0, 0.0);
    // Roll 1.
    assert!(r.try_dodge(WeaponKind::Twins, &mut a, &f.w, md, true));
    assert_eq!(r.rolls_used, 1);
    // A tap mid-roll queues the chain link.
    assert!(r.try_dodge(WeaponKind::Twins, &mut a, &f.w, md, true));
    assert!(r.roll_queued);
    // Roll 1 ends → the queued roll chains straight on (JS L499 re-checks the
    // live intent, so the move direction must still be held).
    for _ in 0..18 {
        r.update(
            DT,
            &gate,
            &mut a,
            WeaponKind::Twins,
            &f.w,
            &f.sb,
            md,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    assert!(r.dodge_t > 0.0, "the queued roll chained on");
    assert_eq!(r.rolls_used, 0, "both charges spent");
    assert!(
        (r.roll_lock - f.w.twins.roll_lockout).abs() < 0.05,
        "chain lockout armed"
    );
    // Locked out: no third roll.
    assert!(!r.try_dodge(WeaponKind::Twins, &mut a, &f.w, md, true));
    // Idle past the lockout + the reset window → the chain refills.
    for _ in 0..300 {
        r.update(
            DT,
            &FireGate::default(),
            &mut a,
            WeaponKind::Twins,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    assert_eq!(r.rolls_used, 0);
    assert!(r.roll_lock <= 0.0);
    assert!(
        r.try_dodge(WeaponKind::Twins, &mut a, &f.w, md, true),
        "chain refilled"
    );
    // A dry tank refuses the roll.
    a.ink = 0.0;
    r.roll_lock = 0.0;
    r.rolls_used = 0;
    r.dodge_t = 0.0;
    assert!(
        !r.try_dodge(WeaponKind::Twins, &mut a, &f.w, md, true),
        "needs rollInk"
    );
}

// =========================================================== Task 30: blaster
// TR-30.1 — cadence / ink and the blast round's own physics fields.
#[test]
fn tr30_1_blaster_cadence_ink_and_fields() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(0x0005_1507);
    let mut r = WeaponRunner::new();
    let gate = FireGate {
        fire: true,
        ..Default::default()
    };
    let mut shots = 0usize;
    for _ in 0..60 {
        r.update(
            DT,
            &gate,
            &mut a,
            WeaponKind::Blaster,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        shots += ps
            .drain_events()
            .into_iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    }
    assert_eq!(shots, 2, "fireInterval 0.78 → two shots in a second");
    assert!((a.ink - (f.t.ink_max - 18.0)).abs() < MM, "inkPerShot 9");
    assert!(
        (r.move_speed(WeaponKind::Blaster, &f.t, &f.w) - f.w.blaster.move_speed_firing).abs() < MM
    );

    // The blast round's fields (JS `fireBlaster` L1173-1192).
    let mut ps2 = ProjectileSim::new(1);
    ps2.fire_blaster(&a, &f.w.blaster, 0.0);
    let p = &ps2.list[0];
    assert!(p.blast, "blaster rounds are blasts");
    assert!(p.pellet.is_none());
    assert_eq!(p.damage, f.w.blaster.direct_damage);
    assert_eq!(p.straight, 99.0);
    assert_eq!(p.grav, 0.0);
    assert_eq!(p.drag, 0.0);
    assert_eq!(p.size, 0.26);
    assert_eq!(p.trail, -1.5);
    assert_eq!(p.trail_every, 2.2);
    assert_eq!(p.trail_radius, 0.45);
    assert!(
        (p.life - f.w.blaster.range / f.w.blaster.proj_speed).abs() < MM,
        "life = range / projSpeed"
    );
}

// TR-30.1 — a direct hit splats outright (125 ≥ 100 hp) and the burst still
// detonates at the body.
#[test]
fn tr30_1_blaster_direct_hit_kills() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut victim = Actor::new(1, 4, &f.t);
    victim.spawn_at(vec3(0.0, 0.05, 3.0), std::f32::consts::PI, &f.sim(), &f.t);
    victim.set_invuln(0.0);
    let mut actors = vec![a.clone(), victim];
    let mut ps = ProjectileSim::new(11);
    ps.fire_blaster(&actors[0], &f.w.blaster, 0.0);
    let mut hit = None;
    let mut burst = None;
    for _ in 0..30 {
        ps.step(DT, &f.world, &mut actors, &mut pg, &f.t, &f.w, &f.sb, None);
        for e in ps.drain_events() {
            if let SimEvent::Hit {
                victim_slot,
                damage,
                killed,
                ..
            } = e
            {
                hit = Some((victim_slot, damage, killed));
            }
            if let SimEvent::Impact { radius, .. } = e {
                burst = Some(radius);
            }
        }
        if hit.is_some() {
            break;
        }
    }
    assert_eq!(
        hit,
        Some((4usize, f.w.blaster.direct_damage, true)),
        "direct hit = 125 = splat"
    );
    assert_eq!(
        burst,
        Some(f.w.blaster.burst_radius),
        "the blast detonated (burstRadius)"
    );
    assert!(!actors[1].alive);
}

// TR-30.1 — the mid-air burst: a blast that expires at `range` detonates and
// splash-damages a nearby enemy with `lerp(splashMax, splashMin, d/splashR)`.
#[test]
fn tr30_1_blaster_airburst_splash_falloff() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    // Off the shot line (x = 1.5) so the round never touches the capsule, but
    // inside the burst radius when the blast expires at full range.
    let mut victim = Actor::new(1, 4, &f.t);
    victim.spawn_at(vec3(1.5, 0.05, 9.5), std::f32::consts::PI, &f.sim(), &f.t);
    victim.set_invuln(0.0);
    let mut actors = vec![a.clone(), victim];
    let mut ps = ProjectileSim::new(0xB1A57);
    ps.fire_blaster(&actors[0], &f.w.blaster, 0.0);
    let mut burst_pos = None;
    let mut hit = None;
    for _ in 0..45 {
        ps.step(DT, &f.world, &mut actors, &mut pg, &f.t, &f.w, &f.sb, None);
        for e in ps.drain_events() {
            match e {
                SimEvent::Impact { pos, radius, .. } => {
                    assert_eq!(radius, f.w.blaster.burst_radius);
                    burst_pos = Some(pos);
                }
                SimEvent::Hit {
                    victim_slot,
                    damage,
                    ..
                } => {
                    assert_eq!(victim_slot, 4);
                    hit = Some(damage);
                }
                _ => {}
            }
        }
    }
    let at = burst_pos.expect("the blast burst in mid-air at range");
    assert!(at.y > 0.5, "air-burst, not a ground impact");
    let d = actors[1].pos + vec3(0.0, 0.7, 0.0);
    let dist = d.distance(at);
    assert!(
        dist < f.w.blaster.splash_radius,
        "victim inside the splash cone"
    );
    let expect = f.w.blaster.splash_damage_max
        + (f.w.blaster.splash_damage_min - f.w.blaster.splash_damage_max)
            * (dist / f.w.blaster.splash_radius);
    assert!(
        (hit.expect("splash damage landed") - expect).abs() < MM,
        "splash = lerp(70, 30, d/splashRadius)"
    );
    assert!(actors[1].alive, "splash alone does not splat");
}

// TR-30.1 — a blast that detonates on the deck paints under the burst.
#[test]
fn tr30_1_blaster_burst_paints_the_deck() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    a.aim_pitch = -0.6; // dig the shot into the deck a couple of metres out
    let mut ps = ProjectileSim::new(0x9E37);
    ps.fire_blaster(&a, &f.w.blaster, 0.0);
    let mut painted = 0.0f32;
    for _ in 0..30 {
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
        for e in ps.drain_events() {
            if let SimEvent::Impact { area, radius, .. } = e {
                assert_eq!(radius, f.w.blaster.burst_radius);
                painted += area;
            }
        }
    }
    assert!(painted > 0.0, "the under-burst splat claimed turf");
    assert!(pg.coverage()[0] > 0.0, "team ink on the deck");
}

// ---- Task 31: charger / spinner / bucket (TR-31.1 / TR-31.2) ----

/// Drive one runner frame with the trigger held.
fn hold(
    r: &mut WeaponRunner,
    a: &mut Actor,
    k: WeaponKind,
    f: &Fix,
    ps: &mut ProjectileSim,
    n: usize,
) {
    for _ in 0..n {
        r.update(
            DT,
            &FireGate {
                fire: true,
                ..Default::default()
            },
            a,
            k,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            ps,
            &mut FoeScan::empty(),
            &mut PaintGrid::new(&f.world),
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
    }
}

/// Drive one runner frame with the trigger released.
fn release(r: &mut WeaponRunner, a: &mut Actor, k: WeaponKind, f: &Fix, ps: &mut ProjectileSim) {
    r.update(
        DT,
        &FireGate::default(),
        a,
        k,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
}

#[test]
fn tr31_1_charger_charge_curve_and_release_timing() {
    let f = fix();
    let cw = &f.w.charger;
    assert_eq!(cw.charge_time, 1.0, "JS config: chargeTime 1.0");
    assert_eq!(cw.ink_full, 18.0, "JS config: inkFull 18");

    // S-curve (weapons.js L192-196): `t < 0.2 ? t·1.25 : 0.25 + (t−0.2)·0.9375`
    // on the `dt / chargeTime` clock — every frame value checked.
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(11);
    let mut r = WeaponRunner::new();
    for n in 1..=60 {
        hold(&mut r, &mut a, WeaponKind::Charger, &f, &mut ps, 1);
        let t = n as f32 * DT;
        let want = if t < 0.2 {
            t * 1.25
        } else {
            0.25 + (t - 0.2) * 0.9375
        };
        assert!(
            (r.charge - want).abs() < MM,
            "frame {n}: charge {} want {want}",
            r.charge
        );
    }
    assert!(r.charge >= 1.0 - MM, "full charge at t = chargeTime");
    assert!(r.charge_dinged, "the full-charge latch fired");

    // Release (JS L204-215): `c = max(0.12, charge)`, ink −= inkFull·c,
    // cooldown 0.28, the hitscan deferred to the match drain (D-T31-1).
    release(&mut r, &mut a, WeaponKind::Charger, &f, &mut ps);
    assert_eq!(r.charger_release, Some(1.0));
    assert!(
        (a.ink - (f.t.ink_max - 18.0)).abs() < MM,
        "full charge costs inkFull"
    );
    assert_eq!(r.cooldown, 0.28);
    assert_eq!(r.firing_t, FIRING_T);
    assert_eq!(r.charge, 0.0);
    assert!(!r.charging);

    // A one-frame tap still shoots: the 0.12 floor (JS L205).
    let mut a2 = Actor::new(0, 0, &f.t);
    a2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps2 = ProjectileSim::new(11);
    let mut r2 = WeaponRunner::new();
    hold(&mut r2, &mut a2, WeaponKind::Charger, &f, &mut ps2, 1);
    assert!(
        (r2.charge - DT * 1.25).abs() < MM,
        "tap charge on the steep leg"
    );
    release(&mut r2, &mut a2, WeaponKind::Charger, &f, &mut ps2);
    assert_eq!(r2.charger_release, Some(0.12), "tap floors at 0.12");
    assert!((a2.ink - (f.t.ink_max - 18.0 * 0.12)).abs() < MM);

    // Half charge: t = 0.5 → 0.25 + 0.3·0.9375 = 0.53125.
    let mut a3 = Actor::new(0, 0, &f.t);
    a3.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps3 = ProjectileSim::new(11);
    let mut r3 = WeaponRunner::new();
    hold(&mut r3, &mut a3, WeaponKind::Charger, &f, &mut ps3, 30);
    assert!((r3.charge - 0.53125).abs() < MM);
    release(&mut r3, &mut a3, WeaponKind::Charger, &f, &mut ps3);
    assert!((r3.charger_release.unwrap() - 0.53125).abs() < MM);
    assert!((a3.ink - (f.t.ink_max - 18.0 * 0.53125)).abs() < MM);

    // Below 20 % of inkFull a charge cannot start (JS L188): empty-click
    // lockout + LowInk, no charging state.
    let mut a4 = Actor::new(0, 0, &f.t);
    a4.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    a4.ink = 3.0; // < 18·0.2 = 3.6
    let mut ps4 = ProjectileSim::new(11);
    let mut r4 = WeaponRunner::new();
    hold(&mut r4, &mut a4, WeaponKind::Charger, &f, &mut ps4, 1);
    assert!(!r4.charging, "the 20 % gate blocks the spin");
    assert_eq!(r4.empty_cd, EMPTY_CD);
    assert!(
        ps4.drain_events()
            .iter()
            .any(|e| matches!(e, SimEvent::LowInk { .. })),
        "empty click emits LowInk"
    );
}

#[test]
fn tr31_1_charger_hitscan_damage_and_events() {
    let f = fix();
    let cw = &f.w.charger;
    let mk = || {
        let mut s = Actor::new(0, 0, &f.t);
        s.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
        let mut v = Actor::new(1, 1, &f.t);
        v.spawn_at(vec3(0.0, 0.05, 6.0), 0.0, &f.sim(), &f.t);
        v.set_invuln(0.0);
        (s, v)
    };

    // Half charge: dmg = lerp(40, 160·0.62, 0.5) = 69.6, range 19; the beam
    // stops on the victim (muzzle z = 0.3 → len ≈ 5.7).
    let (s, v) = mk();
    let mut actors = vec![s, v];
    let mut pg = PaintGrid::new(&f.world);
    let mut ps = ProjectileSim::new(5);
    let m = ProjectileSim::muzzle(&actors[0]);
    ps.fire_charger(
        0,
        0,
        m,
        vec3(0.0, 0.0, 1.0),
        0.5,
        cw,
        &f.world,
        &mut actors,
        &mut pg,
        &f.t,
        &f.sb,
        None,
        None,
        None,
    );
    let evs = ps.drain_events();
    let hit = evs
        .iter()
        .find_map(|e| match e {
            SimEvent::Hit { damage, killed, .. } => Some((*damage, *killed)),
            _ => None,
        })
        .expect("the beam hurts the victim");
    assert!((hit.0 - 69.6).abs() < MM, "half-charge damage");
    assert!(!hit.1);
    assert!(
        (actors[1].hp - (f.t.hp - 69.6)).abs() < MM,
        "hp after the beam"
    );
    let fire = evs
        .iter()
        .find_map(|e| match e {
            SimEvent::Fire { charge, len, .. } => Some((*charge, *len)),
            _ => None,
        })
        .expect("charger fire event");
    assert_eq!(fire.0, Some(0.5), "JS L1281: charge rides the payload");
    let len = fire.1.expect("len rides the payload");
    // The beam stops at the segment/capsule closest-approach point. The
    // solver (JS `segmentCapsuleDist`: 6 samples + 8 golden-section steps
    // over the full 19 m segment) converges to the true minimum (5.7 m)
    // with a ~0.03 m residual — same numbers a JS run produces.
    assert!((len - 5.7).abs() < 0.05, "beam stops on the capsule: {len}");
    let imp = evs
        .iter()
        .find_map(|e| match e {
            SimEvent::Impact {
                radius,
                victim_slot,
                ..
            } => Some((*radius, *victim_slot)),
            _ => None,
        })
        .expect("charger impact");
    assert!(
        (imp.0 - 1.2 * (0.6 + 0.4 * 0.5)).abs() < MM,
        "terminal radius"
    );
    assert_eq!(imp.1, Some(1), "the victim is credited to the impact");

    // Full charge: damageMax 160 splats through 100 hp (JS L1243-1244).
    let (s, v) = mk();
    let mut actors = vec![s, v];
    let mut ps = ProjectileSim::new(5);
    let m = ProjectileSim::muzzle(&actors[0]);
    ps.fire_charger(
        0,
        0,
        m,
        vec3(0.0, 0.0, 1.0),
        1.0,
        cw,
        &f.world,
        &mut actors,
        &mut pg,
        &f.t,
        &f.sb,
        None,
        None,
        None,
    );
    let killed = ps
        .drain_events()
        .iter()
        .any(|e| matches!(e, SimEvent::Hit { killed: true, .. }));
    assert!(killed, "full charge splats");

    // No victim, no world hit: the beam runs the full lerp(rangeMin,
    // rangeMax, 0.5) = 19 m and the line splats paint the deck.
    let (s, _) = mk();
    let mut actors = vec![s];
    let mut pg2 = PaintGrid::new(&f.world);
    let mut ps = ProjectileSim::new(5);
    let m = ProjectileSim::muzzle(&actors[0]);
    ps.fire_charger(
        0,
        0,
        m,
        vec3(0.0, 0.0, 1.0),
        0.5,
        cw,
        &f.world,
        &mut actors,
        &mut pg2,
        &f.t,
        &f.sb,
        None,
        None,
        None,
    );
    let len = ps
        .drain_events()
        .iter()
        .find_map(|e| match e {
            SimEvent::Fire { len, .. } => *len,
            _ => None,
        })
        .expect("fire event");
    assert!((len - 19.0).abs() < MM, "empty beam runs the charged range");
    assert!(pg2.coverage()[0] > 0.0, "the line splats ink the deck");
}

#[test]
fn tr31_1_spinner_linear_charge_and_burst_stream() {
    let f = fix();
    let sp = &f.w.spinner;
    assert_eq!(sp.ink_full, 24.0, "JS config: inkFull 24");
    assert_eq!(sp.fire_interval, 0.05, "JS config: fireInterval 0.05");

    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(13);
    let mut r = WeaponRunner::new();

    // Linear charge (JS L469-470): `charge = min(ink/inkFull, t)`, no S-curve.
    for n in 1..=30 {
        hold(&mut r, &mut a, WeaponKind::Spinner, &f, &mut ps, 1);
        assert!((r.charge - n as f32 * DT).abs() < MM, "frame {n}");
    }
    assert!(
        !r.streaming(),
        "streaming is the retired splatling's (L381-426)"
    );

    // Release (JS L478-485): c = max(0.15, 0.5), ink −= 24·c, burstT =
    // lerp(0.35, 1.8, c) = 1.075, streamCd = 0.
    release(&mut r, &mut a, WeaponKind::Spinner, &f, &mut ps);
    assert!(
        (r.burst_t - 1.075).abs() < MM,
        "burst length follows the charge"
    );
    assert!((r.burst_power - 0.5).abs() < MM);
    assert!((a.ink - (f.t.ink_max - 12.0)).abs() < MM);
    assert_eq!(r.stream_cd, 0.0);
    assert_eq!(r.charge, 0.0, "the ch slot ships 0 during the burst");

    // Stream cadence (JS L452-461): one round per `fireInterval`, burst ends
    // at burstT ≤ 0 → K = ceil(1.075/DT) = 65 frames, shots =
    // floor(K·DT/0.05) + 1 = 22.
    let mut shots = 0usize;
    let mut per_frame_max = 0usize;
    let mut frames = 0usize;
    while r.burst_t > 0.0 && frames < 200 {
        let before = ps.list.len();
        release(&mut r, &mut a, WeaponKind::Spinner, &f, &mut ps);
        let made = ps.list.len() - before;
        per_frame_max = per_frame_max.max(made);
        shots += ps
            .drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
        frames += 1;
    }
    assert_eq!(frames, 65, "burstT 1.075 spans ceil(1.075/DT) frames");
    assert_eq!(shots, 22, "floor(65·DT/0.05) + 1 rounds");
    assert_eq!(
        per_frame_max, 1,
        "interval > DT: never more than one per frame"
    );
    assert_eq!(r.cooldown, 0.2, "burst tail cooldown (JS L458)");
    assert_eq!(ps.list.len(), 22, "no round stepped out");

    // Round fields (fireStream L1338-1342): power 0.5 lerps speed/straight/
    // range between the min/max, size 0.12, life 1.3 capped by range/speed.
    let p = &ps.list[0];
    assert!((p.vel.length() - 32.5).abs() < MM, "lerp(25, 40, 0.5)");
    assert!((p.straight - 0.165).abs() < MM);
    assert_eq!(p.size, 0.12);
    assert!(
        (p.life - (15.5f32 / 32.5).min(1.3)).abs() < MM,
        "range/speed cap"
    );
    assert_eq!(p.grav, PROJ_GRAV);
    assert_eq!(p.drag, PROJ_DRAG);
    assert_eq!(p.trail_every, sp.trail_every);
    assert_eq!(p.trail, -(2.2 - sp.trail_every));
}

#[test]
fn tr31_2_bucket_windup_and_blob_taper() {
    let f = fix();
    let bw = &f.w.bucket;
    assert_eq!(bw.windup, 0.12, "JS config: windup 0.12");
    assert_eq!(bw.blobs, 5, "JS config: blobs 5");

    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps = ProjectileSim::new(17);
    let mut r = WeaponRunner::new();

    // A tap starts the throw (JS L441 `inp.fire || inp.firePressed`); the
    // flick winds up for `windup` and the volley leaves (L434-438).
    r.update(
        DT,
        &FireGate {
            pressed: true,
            ..Default::default()
        },
        &mut a,
        WeaponKind::Bucket,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        &mut ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    assert_eq!(r.flick, 0.0, "the throw wound up on the tap");
    assert!((a.ink - (f.t.ink_max - bw.ink_per_shot)).abs() < MM);
    for _ in 2..9 {
        release(&mut r, &mut a, WeaponKind::Bucket, &f, &mut ps);
    }
    assert!(ps.list.is_empty(), "nothing leaves before the windup ends");
    release(&mut r, &mut a, WeaponKind::Bucket, &f, &mut ps); // frame 9: 8·DT ≥ 0.12
    assert_eq!(r.flick, -1.0, "the windup completed");
    assert_eq!(ps.list.len(), 5, "five blobs per throw (JS L1317)");
    assert_eq!(r.cooldown, bw.fire_interval - bw.windup, "JS L438");
    assert_eq!(r.firing_t, FIRING_T);
    let fires = ps
        .drain_events()
        .iter()
        .filter(|e| matches!(e, SimEvent::Fire { .. }))
        .count();
    assert_eq!(fires, 1, "one weapon:fire per throw (JS L1330)");

    // Per-blob taper (L1318-1324): speed 1−i·0.075, size 0.22−i·0.02,
    // radius 1−i·0.08, start pulled back i·0.12 along the throw dir.
    let m = ProjectileSim::muzzle(&a);
    for (i, p) in ps.list.iter().enumerate() {
        let fi = i as f32;
        assert!(
            (p.vel.length() - bw.throw_speed * (1.0 - fi * 0.075)).abs() < MM,
            "blob {i} speed"
        );
        assert_eq!(p.size, 0.22 - fi * 0.02);
        assert!((p.radius - bw.impact_radius * (1.0 - fi * 0.08)).abs() < MM);
        assert_eq!(p.volley, Some(0), "one throw group");
        assert_eq!(p.below_falloff, bw.below_falloff);
        assert_eq!(p.below_falloff_max, bw.below_falloff_max);
        assert_eq!(p.life, 2.2);
        assert_eq!(p.straight, 0.0);
        assert_eq!(p.grav, bw.gravity);
        assert_eq!(p.drag, bw.drag);
        assert_eq!(p.trail_every, 1.6);
        assert_eq!(p.trail, 0.0);
        assert!(
            (m.distance(p.start) - fi * 0.12).abs() < MM,
            "blob {i} start offset"
        );
    }

    // Horizontal aim + lob 0.3 (L1312-1313): every blob leaves at pitch
    // 0.3 ± the 0.04/2 jitter, heading ± 0.05/2.
    for p in &ps.list {
        let pitch = p.vel.y.atan2(p.vel.x.hypot(p.vel.z));
        assert!(
            (0.28 - MM..0.32 + MM).contains(&pitch),
            "pitch {pitch} outside 0.3 ± 0.02"
        );
        let yaw = p.vel.x.atan2(p.vel.z);
        assert!(yaw.abs() <= 0.025 + MM, "yaw {yaw} outside ± 0.025");
    }

    // Fixed seed ⇒ identical volley; a different seed diverges (3 draws ×
    // 5 blobs: side → pitch → seed).
    let mut a2 = Actor::new(0, 0, &f.t);
    a2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    let mut ps2 = ProjectileSim::new(17);
    ps2.fire_slosh(&a2, bw, None);
    let same = ps
        .list
        .iter()
        .zip(ps2.list.iter())
        .all(|(x, y)| x.vel == y.vel && x.seed == y.seed);
    assert!(same, "the same seed replays the same spread");
    let mut ps3 = ProjectileSim::new(18);
    ps3.fire_slosh(&a2, bw, None);
    assert!(
        ps3.list[0].vel != ps.list[0].vel || ps3.list[4].vel != ps.list[4].vel,
        "a different seed spreads differently"
    );
}

#[test]
fn tr31_2_lob_aim_solves_the_arc() {
    let f = fix();
    let bw = &f.w.bucket;
    let from = vec3(0.0, 1.5, 0.0);
    let target = vec3(0.0, 0.0, 6.0);

    // In reach (hd = 6 ∈ [1, 9.5]): the solved pitch lands the arc on the
    // target — replay the integrator and check the crossing height.
    let mut dir = vec3(0.0, 0.0, 1.0);
    assert!(ProjectileSim::lob_aim(
        from,
        &mut dir,
        target,
        bw.throw_speed,
        bw.gravity,
        bw.drag,
        bw.reach
    ));
    let mut vh = dir.x.hypot(dir.z) * bw.throw_speed;
    let mut vy = dir.y * bw.throw_speed;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut land_y = f32::NAN;
    for _ in 0..600 {
        let (px, py) = (x, y);
        vy -= bw.gravity * SIM_DT;
        let k = 1.0 - bw.drag * SIM_DT;
        vh *= k;
        vy *= k;
        x += vh * SIM_DT;
        y += vy * SIM_DT;
        if x >= 6.0 {
            // Same crossing interpolation as the solver's objective fn.
            land_y = py + (y - py) * ((6.0 - px) / (x - px).max(1e-6));
            break;
        }
    }
    assert!(x >= 6.0, "the solved arc reaches the crosshair");
    assert!(
        (from.y + land_y - target.y).abs() < 0.05,
        "the arc lands on the target: y = {}",
        from.y + land_y
    );
    assert!(
        dir.x.abs() < MM && dir.z > 0.9,
        "the heading survives the solve"
    );

    // Out of reach / too close: no solve, dir untouched (JS L998).
    let mut d2 = vec3(0.0, 0.0, 1.0);
    assert!(
        !ProjectileSim::lob_aim(
            from,
            &mut d2,
            vec3(0.0, 0.0, 12.0),
            bw.throw_speed,
            bw.gravity,
            bw.drag,
            bw.reach
        ),
        "hd 12 > reach 9.5"
    );
    assert_eq!(d2, vec3(0.0, 0.0, 1.0), "the default dir survives");
    assert!(
        !ProjectileSim::lob_aim(
            from,
            &mut d2,
            vec3(0.0, 0.0, 0.5),
            bw.throw_speed,
            bw.gravity,
            bw.drag,
            bw.reach
        ),
        "hd 0.5 < 1"
    );
}

#[test]
fn tr31_2_volley_passthrough_and_below_falloff() {
    let f = fix();
    let bw = &f.w.bucket;
    let mk_blob = |i: usize, start_y: f32| Projectile {
        pos: vec3(0.0, 1.0, 2.5 + i as f32 * 0.1),
        prev: vec3(0.0, 1.0, 2.5 + i as f32 * 0.1),
        start: vec3(0.0, start_y, 0.0),
        vel: vec3(0.0, 0.0, 60.0),
        team: 0,
        owner_slot: 0,
        age: 0.0,
        life: 2.2,
        straight: 0.0,
        radius: bw.impact_radius,
        size: 0.15,
        damage: bw.damage,
        grav: bw.gravity,
        drag: bw.drag,
        trail: 0.0,
        trail_every: 0.0,
        trail_radius: 0.42,
        seed: 0.5,
        blast: false,
        pellet: None,
        burst: None,
        ghost: false,
        below_falloff: bw.below_falloff,
        below_falloff_max: bw.below_falloff_max,
        volley: Some(0),
        dmg_far: None,
        delay: 0.0,
        sp: None,
    };

    // Two blobs of one throw across the same victim: the first hurts, the
    // second passes through (JS L1599) and stays alive.
    let mut victim = Actor::new(1, 1, &f.t);
    victim.spawn_at(vec3(0.0, 0.05, 3.0), 0.0, &f.sim(), &f.t);
    let mut pg = PaintGrid::new(&f.world);
    let mut ps = ProjectileSim::new(3);
    ps.list.push(mk_blob(0, 1.0));
    ps.list.push(mk_blob(1, 1.0));
    ps.volley_alive.insert(0, 2);
    let mut owner = ShieldOwner {
        runners: &mut [],
        launched: &mut [],
    };
    ps.step_masked(
        DT,
        &f.world,
        &mut [victim.clone()],
        &mut pg,
        &f.t,
        &f.w,
        &f.sb,
        None,
        None,
        &[],
        &mut owner,
        None,
    );
    let hits: Vec<f32> = ps
        .drain_events()
        .iter()
        .filter_map(|e| match e {
            SimEvent::Hit { damage, .. } => Some(*damage),
            _ => None,
        })
        .collect();
    assert_eq!(hits.len(), 1, "one hit per throw per victim");
    assert!((hits[0] - 55.0).abs() < MM, "level target takes full 55");
    assert_eq!(ps.list.len(), 1, "the second blob sailed through");
    assert!(ps.volley_hits.contains_key(&(0, 0)), "the group remembers");

    // The group's record dies with its last blob (JS shares the Set object).
    for _ in 0..200 {
        ps.step_masked(
            DT,
            &f.world,
            &mut [],
            &mut pg,
            &f.t,
            &f.w,
            &f.sb,
            None,
            None,
            &[],
            &mut owner,
            None,
        );
        let _ = ps.drain_events();
        if ps.list.is_empty() {
            break;
        }
    }
    assert!(ps.list.is_empty(), "the survivor expires on life");
    assert!(
        ps.volley_hits.is_empty() && ps.volley_alive.is_empty(),
        "the group was cleaned"
    );

    // Below-falloff (JS L1605): target 1.7 m under the throw start →
    // 55·(1 − clamp(1.7·0.12, 0, 0.35)) = 55·0.796 = 43.78.
    let mut victim2 = Actor::new(1, 1, &f.t);
    victim2.spawn_at(vec3(0.0, 0.05, 3.0), 0.0, &f.sim(), &f.t);
    let mut ps2 = ProjectileSim::new(3);
    ps2.list.push(mk_blob(0, 3.0));
    ps2.step_masked(
        DT,
        &f.world,
        &mut [victim2.clone()],
        &mut pg,
        &f.t,
        &f.w,
        &f.sb,
        None,
        None,
        &[],
        &mut owner,
        None,
    );
    let dmg = ps2
        .drain_events()
        .into_iter()
        .find_map(|e| match e {
            SimEvent::Hit { damage, .. } => Some(damage),
            _ => None,
        })
        .expect("the raised blob still connects");
    assert!((dmg - 43.78).abs() < MM, "downward throw loses 20.4 %");
}
