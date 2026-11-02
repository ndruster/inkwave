//! Task 32 TR-32.1 / TR-32.2: the main-weapon batch three — roller drum +
//! flick, brush drag + swipe and the blade kit (tap crescent, heavy wave,
//! melee bounds, ghost waves). Fixed-seed, value-by-value against the ported
//! formulas (spec §8).

use super::blade::{BladeWave, Swing};
use super::*;
use crate::actor::{Actor, FIXED_DT, FireGate, SimWorld};
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

/// Flat 20×20 m deck at y = 0 — the same stage as [`super::brolly_tests`].
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

/// A kid at the deck origin (`spawn_at` snaps it to the deck top, y = 0.0),
/// aiming +Z (yaw 0, pitch 0).
fn kid(f: &Fix) -> Actor {
    let mut a = Actor::new(0, 0, &f.t);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.t);
    a
}

fn gate(fire: bool) -> FireGate {
    FireGate {
        fire,
        ..Default::default()
    }
}

fn gate_press() -> FireGate {
    FireGate {
        fire: true,
        pressed: true,
        ..Default::default()
    }
}

/// One runner frame with an empty foe scan (fire-path-only tests).
#[allow(clippy::too_many_arguments)]
fn frame(
    f: &Fix,
    r: &mut WeaponRunner,
    a: &mut Actor,
    kind: WeaponKind,
    ps: &mut ProjectileSim,
    pg: &mut PaintGrid,
    g: &FireGate,
    move_dir: Vec3,
) -> Vec<SimEvent> {
    r.update(
        DT,
        g,
        a,
        kind,
        &f.w,
        &f.sb,
        move_dir,
        &f.world,
        ps,
        &mut FoeScan::empty(),
        pg,
        &f.t,
        None,
        g.fire,
        0.0,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        false,
    );
    ps.drain_events()
}

/// Count the draws a [`Rng`] made between two states (the private `s` field
/// is visible to this child module — the xorshift stream is tiny, so the
/// test count is bounded by the loop guard).
fn rng_draws(before: u32, after: u32) -> usize {
    let mut r = Rng { s: before };
    for n in 1..=4096 {
        if r.next_u32() == after {
            return n;
        }
    }
    panic!("rng state not reached within 4096 draws");
}

/// Advance a clone of `rng` by `n` draws and return the values.
fn pre_draw(rng: &Rng, n: usize) -> Vec<f32> {
    let mut r = *rng;
    (0..n).map(|_| r.next_f32()).collect()
}

fn count_kind(events: &[SimEvent], is: impl Fn(&SimEvent) -> bool) -> usize {
    events.iter().filter(|e| is(e)).count()
}

// ------------------------------------------------------------------ TR-32.1

// The roller drum: one 3-splat group per ≥0.28 m of travel, 3 rng draws per
// group, ink = 1.1 × moved, and the Impact radius = rollWidth / 2.
#[test]
fn tr32_1_roller_drag_paint_ink_and_rng() {
    let f = fix();
    let rw = &f.w.roller;
    let mut a = kid(&f);
    a.grounded = true;
    a.vel = vec3(0.0, 0.0, rw.roll_speed);
    let mut ps = ProjectileSim::new(0x2024_0332);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let s0 = ps.rng.s;

    let mut paints = 0usize;
    let mut ink_at_last_paint = a.ink;
    for _frame_i in 0..40 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Roller,
            &mut ps,
            &mut pg,
            &gate(true),
            Vec3::ZERO,
        );
        let impact = ev.iter().find_map(|e| match e {
            SimEvent::Impact { radius, area, .. } => Some((*radius, *area)),
            _ => None,
        });
        if let Some((radius, area)) = impact {
            paints += 1;
            assert!(
                (radius - rw.roll_width / 2.0).abs() <= MM,
                "radius = rollWidth/2"
            );
            assert!(area > 0.0, "the drum must cover ground");
            let moved = rw.roll_speed * DT * 4.0; // one group per 4 frames
            assert!(
                moved >= 0.28 - MM && rw.roll_speed * DT * 3.0 < 0.28,
                "cadence gate"
            );
            let want = (ink_at_last_paint - rw.roll_ink_per_meter * moved).max(0.0);
            assert!((a.ink - want).abs() <= 1e-4, "ink = 1.1 × moved");
            ink_at_last_paint = a.ink;
        } else {
            assert!(
                (a.ink - ink_at_last_paint).abs() <= MM,
                "a sub-0.28 m frame neither paints nor spends ink"
            );
        }
        a.pos += vec3(0.0, 0.0, rw.roll_speed * DT);
    }
    // The drum starts counting displacement on the frame it enters the roll
    // state, so the first group lands on frame 5 and then every 4 frames.
    assert_eq!(paints, 9, "40 frames at 4.4 m/s = 9 drum groups");
    assert_eq!(
        rng_draws(s0, ps.rng.s),
        paints * 3,
        "exactly one seed per splat"
    );
}

// The brush drag: one splat per ≥0.22 m, ink = 0.32 × moved, radius =
// brushWidth × 0.45, Impact radius = brushWidth / 2.
#[test]
fn tr32_1_brush_drag_paint_and_ink() {
    let f = fix();
    let bw = &f.w.brush;
    let mut a = kid(&f);
    a.grounded = true;
    a.vel = vec3(0.0, 0.0, bw.brush_speed);
    let mut ps = ProjectileSim::new(0xB00B5);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let s0 = ps.rng.s;

    let mut paints = 0usize;
    let mut ink_at_last = a.ink;
    for _ in 0..30 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Brush,
            &mut ps,
            &mut pg,
            &gate(true),
            vec3(0.0, 0.0, 1.0),
        );
        let impact = ev.iter().find_map(|e| match e {
            SimEvent::Impact { radius, area, .. } => Some((*radius, *area)),
            _ => None,
        });
        if let Some((radius, area)) = impact {
            paints += 1;
            assert!((radius - bw.brush_width / 2.0).abs() <= MM);
            assert!(area > 0.0);
            let moved = bw.brush_speed * DT * 2.0;
            let want = (ink_at_last - bw.brush_ink_per_meter * moved).max(0.0);
            assert!((a.ink - want).abs() <= 1e-4, "ink = 0.32 × moved");
            ink_at_last = a.ink;
        }
        a.pos += vec3(0.0, 0.0, bw.brush_speed * DT);
    }
    // Displacement starts from the frame the drag state is entered → the
    // first line lands on frame 3, then every 2 frames.
    assert_eq!(paints, 14, "30 frames at 7.4 m/s = 14 bristle lines");
    assert_eq!(rng_draws(s0, ps.rng.s), paints, "one seed per splat");
}

// The brush swipe: 0.04 s wind-up → 5 drops, exact per-drop values from the
// seeded stream (RNG order [speed, radius, seed, cu]), `swipeSide` alternating
// (the flip happens on the throw, so the first swipe uses side = −1).
#[test]
fn tr32_1_brush_swipe_values_and_alternation() {
    let f = fix();
    let bw = &f.w.brush;
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(0x511CE);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let s0 = ps.rng.s;

    // Standing still + held fire → the swipe throw on frame 1.
    let mut released = None;
    for i in 1..=10 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Brush,
            &mut ps,
            &mut pg,
            &gate(true),
            Vec3::ZERO,
        );
        if count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })) == 1 {
            released = Some(i);
            break;
        }
    }
    assert_eq!(released, Some(4), "wind-up 0.04 s → release on frame 4");
    assert_eq!(ps.list.len(), bw.swipe_drops as usize);
    assert!((a.ink - (f.t.ink_max - bw.swipe_ink)).abs() <= MM);
    assert!(
        (r.cooldown - bw.swipe_interval).abs() <= MM,
        "cooldown = 0.16"
    );
    assert!((r.firing_t - 0.25).abs() <= MM, "firing_t literal");
    let side = -1.0_f32; // swipeSide flipped from +1 on the throw
    let up = 0.08_f32; // clamp(0,−0.3,0.3)·0.5+0.08
    let arc = bw.swipe_spread_deg * std::f32::consts::PI / 180.0 * 1.6;
    let rands = pre_draw(&Rng { s: s0 }, bw.swipe_drops as usize * 4);
    for (i, p) in ps.list.iter().enumerate() {
        let t = i as f32 / 4.0;
        let ang = -(side * (t - 0.5) * arc); // yaw 0, ang = yaw − side·(t−0.5)·arc
        let ox = ang.sin();
        let oz = ang.cos();
        let (r1, r2, r3, r4) = (
            rands[i * 4],
            rands[i * 4 + 1],
            rands[i * 4 + 2],
            rands[i * 4 + 3],
        );
        let sp = bw.swipe_speed * (0.8 + r1 * 0.3);
        let cu = (up + (r4 - 0.5) * 0.08).cos();
        let tx = -oz * side;
        let tz = ox * side;
        let hx = ox * 0.8 + tx * 0.45 + 0.0 * 0.25; // fx = sin(0) = 0
        let hz = oz * 0.8 + tz * 0.45 + 1.0 * 0.25; // fz = cos(0) = 1
        let hl = hx.hypot(hz);
        assert!((p.pos.x - ox * 0.85).abs() <= MM, "drop {i} pos.x");
        // `spawn_at` snaps the kid to the deck top (y = 0), so the drop rides
        // a.pos.y + 0.45.
        assert!((p.pos.y - 0.45).abs() <= MM, "drop {i} pos.y");
        assert!((p.pos.z - oz * 0.85).abs() <= MM, "drop {i} pos.z");
        assert!((p.vel.x - hx / hl * cu * sp).abs() <= MM, "drop {i} vel.x");
        assert!((p.vel.y - up.sin() * sp).abs() <= MM, "drop {i} vel.y");
        assert!((p.vel.z - hz / hl * cu * sp).abs() <= MM, "drop {i} vel.z");
        assert!(
            (p.radius - (0.55 + r2 * 0.2)).abs() <= MM,
            "drop {i} radius"
        );
        assert!((p.seed - r3).abs() <= MM, "drop {i} seed");
        assert_eq!(p.life, 1.0);
        assert_eq!(p.size, 0.1);
        assert_eq!(p.grav, 26.0);
        assert_eq!(p.drag, 0.6);
        assert_eq!(p.straight, 0.0);
        assert_eq!(p.delay, 0.0);
        assert!((p.damage - bw.swipe_damage_near).abs() <= MM);
        assert_eq!(p.dmg_far, Some(bw.swipe_damage_far));
        assert_eq!(p.trail, 0.0);
        assert_eq!(p.trail_every, 2.2);
        assert_eq!(p.trail_radius, 0.32);
    }
    assert_eq!(rng_draws(s0, ps.rng.s), bw.swipe_drops as usize * 4);

    // Second swipe after the cooldown: the side flips back to +1.
    for _ in 0..12 {
        let _ = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Brush,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
    }
    ps.list.clear();
    for i in 1..=10 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Brush,
            &mut ps,
            &mut pg,
            &gate(true),
            Vec3::ZERO,
        );
        if count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })) == 1 {
            assert_eq!(i, 4, "same wind-up");
            break;
        }
    }
    let ang0 = -(1.0 * (0.0 - 0.5) * arc); // i = 0, side = +1
    assert!(
        (ps.list[0].pos.z - ang0.cos() * 0.85).abs() <= MM,
        "the second swipe mirrors (side = +1)"
    );
}

// The blade tap: 7 crescent drops, exact per-drop values (RNG order
// [speed, radius, seed, cu]), sweep-order delays, volley group, Fire event.
#[test]
fn tr32_1_blade_tap_crescent_values() {
    let f = fix();
    let bl = &f.w.blade;
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(0xB1ADE);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let s0 = ps.rng.s;

    let ev1 = frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    assert!(ev1.is_empty(), "the tap itself emits nothing on frame 1");
    assert!((a.ink - (f.t.ink_max - bl.tap_ink)).abs() <= MM);
    assert_eq!(ps.list.len(), 0, "drops leave on the 0.028 s release");
    // The swing is created at the end of the press frame (`run_swing` runs
    // before the press handling), so it first advances on frame 2 — the
    // crescent leaves when t crosses 0.028, on frame 3.
    let mut ev_fire = Vec::new();
    let mut fire_frame = 0;
    for i in 2..=6 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
        if count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })) == 1 {
            ev_fire = ev;
            fire_frame = i;
            break;
        }
    }
    assert_eq!(fire_frame, 3, "release at swing t ≥ 0.028 → frame 3");

    let up = 0.06_f32; // clamp(0,−0.4,0.45)·0.75+0.06
    let arc = bl.tap_spread_deg * std::f32::consts::PI / 180.0;
    let side = 1.0_f32; // kit side −1 flips to +1 on the first tap
    let rands = pre_draw(&Rng { s: s0 }, 7 * 4);
    assert_eq!(ps.list.len(), 7);
    for (i, p) in ps.list.iter().enumerate() {
        let t = i as f32 / 6.0;
        let ang = side * (t - 0.5) * arc;
        let ox = ang.sin();
        let oz = ang.cos();
        let mid = 1.0 - (t - 0.5).abs() * 2.0;
        let (r1, r2, r3, r4) = (
            rands[i * 4],
            rands[i * 4 + 1],
            rands[i * 4 + 2],
            rands[i * 4 + 3],
        );
        let sp = bl.tap_speed * (0.8 + 0.24 * mid) * (0.95 + r1 * 0.1);
        let cu = (up + (r4 - 0.5) * 0.05).cos();
        let tx = -oz * side;
        let tz = ox * side;
        let hx = ox + tx * 0.12;
        let hz = oz + tz * 0.12;
        let hl = hx.hypot(hz);
        assert!((p.pos.x - ox * 0.8).abs() <= MM, "drop {i} pos.x");
        assert!(
            (p.pos.y - (0.98 + (0.5 - t) * 0.12)).abs() <= MM,
            "drop {i} pos.y"
        ); // a.pos.y = 0
        assert!((p.pos.z - oz * 0.8).abs() <= MM, "drop {i} pos.z");
        assert!((p.vel.x - hx / hl * cu * sp).abs() <= MM, "drop {i} vel.x");
        assert!((p.vel.y - up.sin() * sp).abs() <= MM, "drop {i} vel.y");
        assert!((p.vel.z - hz / hl * cu * sp).abs() <= MM, "drop {i} vel.z");
        assert!((p.radius - bl.tap_drop_radius * (0.85 + r2 * 0.3)).abs() <= MM);
        assert!((p.seed - r3).abs() <= MM);
        assert!((p.delay - t * 0.018).abs() <= MM, "sweep-order delay");
        assert_eq!(p.life, 1.2);
        assert_eq!(p.straight, bl.tap_straight);
        assert_eq!(p.size, 0.13);
        assert_eq!(p.grav, 24.0);
        assert_eq!(p.drag, 0.5);
        assert_eq!(p.trail, -0.2);
        assert_eq!(p.trail_every, 1.25);
        assert_eq!(p.trail_radius, 0.3);
        assert!((p.damage - bl.tap_damage_near).abs() <= MM);
        assert_eq!(p.dmg_far, Some(bl.tap_damage_far));
        assert_eq!(p.volley, Some(0), "one volley group per cut");
    }
    assert_eq!(rng_draws(s0, ps.rng.s), 28);
    if let Some(SimEvent::Fire { muzzle, dir, .. }) =
        ev_fire.iter().find(|e| matches!(e, SimEvent::Fire { .. }))
    {
        assert!(
            (muzzle.x - 0.0).abs() <= MM
                && (muzzle.y - 0.98).abs() <= MM
                && (muzzle.z - 0.8).abs() <= MM
        );
        let want = vec3(0.0, up.sin(), 1.0).normalize();
        assert!((dir - want).length() <= MM);
    } else {
        unreachable!();
    }
}

// The heavy cut: full charge → release → overhead swing → the wave handoff
// with the Kit record (r2 pos / r3 dir) and the lunge.
#[test]
fn tr32_1_blade_heavy_wave_kit_record() {
    let f = fix();
    let bl = &f.w.blade;
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(0xC0FFEE);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();

    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    let mut frames = 0;
    while r.charge < 1.0 && frames < 200 {
        frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate(true),
            Vec3::ZERO,
        );
        frames += 1;
    }
    assert!(r.charge >= 1.0, "the charge completes while held");
    // Release → heavy.
    let _ = frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate(false),
        Vec3::ZERO,
    );
    let ink_after_tap = f.t.ink_max - bl.tap_ink;
    assert!(
        (a.ink - (ink_after_tap - bl.heavy_ink).max(0.0)).abs() <= MM,
        "heavy spends 9 ink"
    );
    let kit = r.blade.as_ref().expect("kit");
    assert!(kit.swing.as_ref().expect("swing").heavy);
    assert!(
        (kit.lunge - bl.lunge_time).abs() <= MM,
        "grounded safe lunge"
    );
    assert_eq!(kit.cd, bl.heavy_interval);
    assert!((r.firing_t - 0.5).abs() <= MM);

    // The wave spawns at swing t ≥ 0.05 with the quantised Kit record.
    let mut kit_data = None;
    for _ in 0..6 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
        if let Some(e) = ev.iter().find(|e| matches!(e, SimEvent::Kit { .. })) {
            if let SimEvent::Kit { kind, data, .. } = e {
                assert_eq!(kind, "blade");
                kit_data = Some(data.clone());
            }
            break;
        }
    }
    let d = kit_data.expect("the heavy cut emits the blade Kit record");
    assert_eq!(d.len(), 6);
    // pos = (x + sin(yaw)·0.55, y + waveHeight·0.52, z + cos(yaw)·0.55), r2.
    assert_eq!(d[0], 0.0);
    assert!(
        (d[1] - (0.0 + bl.wave_height * 0.52).mul_add(100.0, 0.5).trunc() / 100.0).abs() <= 1e-4
    );
    assert_eq!(d[2], 0.55);
    // pitch 0 → dir = (sin 0, 0, cos 0), r3.
    assert_eq!(&d[3..], &[0.0, 0.0, 1.0]);
    let wave = r.blade_wave.as_ref().expect("the match takes the wave");
    assert!((wave.pos.z - 0.55).abs() <= MM);
    assert_eq!(wave.dir, vec3(0.0, 0.0, 1.0));
    assert!(!wave.ghost);
}

// The charged wave: the stripe cadence over 12 m (paintAcc pre-armed 0.3,
// 0.7 m every) and the spent-wave ground spatter, then the dying fade.
#[test]
fn tr32_1_blade_wave_stripes_and_end() {
    let f = fix();
    let bl = &f.w.blade;
    let mut ps = ProjectileSim::new(0x1234);
    let mut pg = PaintGrid::new(&f.world);
    let s0 = ps.rng.s;
    let mut wave = BladeWave::new(0, 0, vec3(0.0, 0.934, -6.0), vec3(0.0, 0.0, 1.0), false);

    let mut turfs = 0usize;
    let mut impacts = 0usize;
    let mut end_frame = None;
    for i in 1..=40 {
        let alive = wave.step(DT, &f.world, &mut pg, bl, &f.t, &mut ps, &mut [], None);
        let ev = ps.drain_events();
        turfs += count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. }));
        impacts += count_kind(&ev, |e| matches!(e, SimEvent::Impact { .. }));
        if wave.dying >= 0.0 && end_frame.is_none() {
            end_frame = Some(i);
            assert!((wave.dying - 0.14).abs() <= MM, "the burst fade window");
        }
        if !alive {
            assert_eq!(end_frame.unwrap() + 9, i, "dying 0.14 s → 9 fade frames");
            break;
        }
    }
    // Stripes on frames 1,3,…,27 (14) + the spent-wave spatter on frame 28.
    assert_eq!(end_frame, Some(28), "12 m at 26 m/s → frame 28");
    assert_eq!(turfs, 15, "14 stripes + 1 end spatter");
    assert_eq!(impacts, 0, "the spent wave slumps — turf credit, no impact");
    // 2 draws per stripe (radius, seed) + 1 end seed.
    assert_eq!(rng_draws(s0, ps.rng.s), 29);
    assert!(wave.dist >= bl.wave_range);
}

// A wave that meets a wall bursts: the Impact carries `waveEndRadius` 1.05.
#[test]
fn tr32_1_blade_wave_wall_burst_impact() {
    let f = fix();
    let bl = &f.w.blade;
    // The plain deck has no wall — the +X wave would fly off the edge. Add
    // the east wall so the segment test breaks the wave at x = 10.
    let w2 = CollisionWorld::from_brushes(
        &[
            bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
            bx([10.0, -1.0, -10.0], [11.0, 3.0, 10.0]),
        ],
        B,
    );
    let mut ps = ProjectileSim::new(0x99);
    let mut pg = PaintGrid::new(&w2);
    let mut wave = BladeWave::new(0, 0, vec3(0.0, 0.934, 0.0), vec3(1.0, 0.0, 0.0), false);

    let mut impact = None;
    for _ in 0..40 {
        let alive = wave.step(DT, &w2, &mut pg, bl, &f.t, &mut ps, &mut [], None);
        for e in ps.drain_events() {
            if let SimEvent::Impact { radius, .. } = e {
                impact = Some((radius, wave.dying));
            }
        }
        if impact.is_some() {
            assert!(alive, "a stopped wave lives through its fade");
            break;
        }
    }
    let (radius, dying) = impact.expect("the east deck wall breaks the wave");
    assert!(
        (radius - bl.wave_end_radius).abs() <= MM,
        "end burst radius 1.05"
    );
    assert!((dying - 0.14).abs() <= MM);
}

// A wave that flies off the deck paints nothing over the void: the stripe
// rng draws sit behind the `g.hit` gate (P0 review) and the spent-wave end
// spatter raycasts too — a miss draws nothing and credits nothing — while
// the paint accumulator keeps resetting on every threshold crossing.
#[test]
fn tr32_1_blade_wave_over_void_draws_nothing() {
    let f = fix();
    let bl = &f.w.blade;
    // Deck spans z ∈ [−10, 0] only; the +Z wave leaves the edge at z = 0.
    let w2 = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 0.0])], B);
    let mut ps = ProjectileSim::new(0x2E10);
    let mut pg = PaintGrid::new(&w2);
    let s0 = ps.rng.s;
    let mut wave = BladeWave::new(0, 0, vec3(0.0, 0.934, -6.0), vec3(0.0, 0.0, 1.0), false);

    let mut turfs = 0usize;
    let mut impacts = 0usize;
    let mut end_frame = None;
    for i in 1..=40 {
        let alive = wave.step(DT, &w2, &mut pg, bl, &f.t, &mut ps, &mut [], None);
        let ev = ps.drain_events();
        turfs += count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. }));
        impacts += count_kind(&ev, |e| matches!(e, SimEvent::Impact { .. }));
        if wave.dying >= 0.0 && end_frame.is_none() {
            end_frame = Some(i);
        }
        if !alive {
            assert_eq!(end_frame.unwrap() + 9, i, "the fade window is unchanged");
            break;
        }
    }
    // Stripes land on frames 1,3,…,13 while the wave is still over the deck
    // (7 groups); from frame 15 the raycast misses and nothing is credited.
    assert_eq!(end_frame, Some(28), "12 m at 26 m/s → frame 28");
    assert_eq!(turfs, 7, "only the on-deck stripes credit turf");
    assert_eq!(impacts, 0);
    // 2 draws per on-deck stripe — and, unlike the full-deck run's 29, the
    // spent-wave end over the void adds no seed draw at all.
    assert_eq!(rng_draws(s0, ps.rng.s), 14, "zero draws across the void");
    // The accumulator still resets on every crossing over the void, so it
    // can never run away past the threshold.
    assert!(
        wave.paint_acc < bl.wave_paint_every,
        "paint_acc keeps resetting over the void"
    );
}

// ------------------------------------------------------------------ TR-32.2

// The roller flick: pressed edge only, 0.22 s wind-up (14 frames), the
// 9-drop sheet with exact values, cooldown 0.40 / firing_t 0.25 / recover
// 0.18; a low-ink press clicks empty (0.45 lockout) and keeps rolling.
#[test]
fn tr32_2_roller_flick_timing_and_low_ink() {
    let f = fix();
    let rw = &f.w.roller;
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(0xF11C);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let s0 = ps.rng.s;

    // Held fire without a press never winds up.
    for _ in 0..20 {
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Roller,
            &mut ps,
            &mut pg,
            &gate(true),
            Vec3::ZERO,
        );
        assert!(count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })) == 0);
    }
    assert_eq!(ps.list.len(), 0);

    // Low ink: the press clicks empty and falls through to rolling.
    a.ink = rw.flick_ink - 0.5;
    let ev = frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Roller,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::LowInk { .. })), 1);
    assert!((r.empty_cd - EMPTY_CD).abs() <= MM, "0.45 click lockout");
    assert_eq!(r.flick, -1.0, "no wind-up without ink");
    assert!(
        (a.ink - (rw.flick_ink - 0.5)).abs() <= MM,
        "the empty click spends no ink"
    );
    a.ink = f.t.ink_max;

    // The real flick.
    let ev = frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Roller,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    assert!(ev.is_empty());
    assert_eq!(r.flick, 0.0);
    assert!((a.ink - (f.t.ink_max - rw.flick_ink)).abs() <= MM);
    let mut release = 0;
    loop {
        release += 1;
        let ev = frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Roller,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
        if count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })) == 1 {
            break;
        }
        assert!(release < 30, "the wind-up must complete");
    }
    assert_eq!(release, 14, "0.22 s / 1/60 → 14 wind-up frames");
    assert!(
        (r.cooldown - (rw.flick_interval - rw.flick_windup)).abs() <= MM,
        "0.62−0.22"
    );
    assert!((r.firing_t - 0.25).abs() <= MM);
    assert!((r.flick_recover - 0.18).abs() <= MM);
    assert_eq!(ps.list.len(), rw.flick_drops as usize);

    // Exact sheet values. RNG order per drop: [ang, speed, radius, seed, cu].
    let up = 0.32_f32; // clamp(0,−0.2,0.5)+0.32
    let rands = pre_draw(&Rng { s: s0 }, 9 * 5);
    for (i, p) in ps.list.iter().enumerate() {
        let t = (i as f32 / 8.0) * 2.0 - 1.0;
        let (ra, rs, rr, rseed, rcu) = (
            rands[i * 5],
            rands[i * 5 + 1],
            rands[i * 5 + 2],
            rands[i * 5 + 3],
            rands[i * 5 + 4],
        );
        let ang =
            t * rw.flick_spread_deg * (std::f32::consts::PI / 180.0) * 0.5 + (ra - 0.5) * 0.05;
        let sp = rw.flick_speed * (0.82 + 0.28 * (1.0 - t.abs()) + rs * 0.08);
        let cu = (up + (rcu - 0.5) * 0.12).cos();
        assert!(
            (p.pos.x - 0.0).abs() <= MM
                && (p.pos.y - 1.3).abs() <= MM
                && (p.pos.z - 0.6).abs() <= MM
        );
        assert!((p.vel.x - ang.sin() * cu * sp).abs() <= MM);
        assert!((p.vel.y - up.sin() * sp).abs() <= MM);
        assert!((p.vel.z - ang.cos() * cu * sp).abs() <= MM);
        assert!((p.radius - (0.85 + rr * 0.3)).abs() <= MM);
        assert!((p.seed - rseed).abs() <= MM);
        assert_eq!(p.life, 1.4);
        assert_eq!(p.size, 0.15);
        assert_eq!(p.grav, 26.0);
        assert_eq!(p.drag, 0.4);
        assert!((p.damage - rw.flick_damage_near).abs() <= MM);
        assert_eq!(p.dmg_far, Some(rw.flick_damage_far));
    }
    assert_eq!(rng_draws(s0, ps.rng.s), 45);
}

// The brush "moving" gate: hs > 2.0 and |move_dir| > 0.3 are strict; a
// standing kid firing swipes instead of dragging.
#[test]
fn tr32_2_brush_moving_boundary() {
    let f = fix();
    let bw = &f.w.brush;

    // hs exactly 2.0 → not moving → a standing shot swipes (ink −2.2).
    let mut a = kid(&f);
    a.grounded = true;
    a.vel = vec3(0.0, 0.0, 2.0);
    let mut ps = ProjectileSim::new(1);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Brush,
        &mut ps,
        &mut pg,
        &gate(true),
        vec3(0.0, 0.0, 1.0),
    );
    assert_eq!(r.flick, 0.0, "hs = 2.0 is not moving → the swipe throw");
    assert!((a.ink - (f.t.ink_max - bw.swipe_ink)).abs() <= MM);

    // hs 2.01 but |move_dir| exactly 0.3 → still not moving → swipe.
    let mut a2 = kid(&f);
    a2.grounded = true;
    a2.vel = vec3(0.0, 0.0, 2.01);
    let mut ps2 = ProjectileSim::new(2);
    let mut pg2 = PaintGrid::new(&f.world);
    let mut r2 = WeaponRunner::new();
    frame(
        &f,
        &mut r2,
        &mut a2,
        WeaponKind::Brush,
        &mut ps2,
        &mut pg2,
        &gate(true),
        vec3(0.0, 0.0, 0.3),
    );
    assert_eq!(r2.flick, 0.0, "|move_dir| = 0.3 is not moving");

    // hs 2.01 + |move_dir| 0.31 → moving → the drag state machine, no swipe.
    let mut a3 = kid(&f);
    a3.grounded = true;
    a3.vel = vec3(0.0, 0.0, 7.4);
    let mut ps3 = ProjectileSim::new(3);
    let mut pg3 = PaintGrid::new(&f.world);
    let mut r3 = WeaponRunner::new();
    frame(
        &f,
        &mut r3,
        &mut a3,
        WeaponKind::Brush,
        &mut ps3,
        &mut pg3,
        &gate(true),
        vec3(0.0, 0.0, 0.31),
    );
    assert_eq!(r3.flick, -1.0, "moving → brushing, no swipe");
    assert!(r3.rolling);
    assert!(
        (a3.ink - f.t.ink_max).abs() <= MM,
        "no swipe ink while dragging"
    );
}

// Drop fall-off (JS L1603): damage lerps near → far over 7 m from `start`,
// before the below-falloff. 0 / 3.5 / 7 / 10 m → 125 / 77.5 / 30 / 30.
#[test]
fn tr32_2_drop_dmg_far_lerp() {
    let f = fix();
    let rw = &f.w.roller;
    let mut owner = ShieldOwner {
        runners: &mut [],
        launched: &mut [],
    };

    let mut hit_at = |z: f32, seg_len: f32| -> f32 {
        let mut ps = ProjectileSim::new(7);
        let mut pg = PaintGrid::new(&f.world);
        ps.list.push(Projectile {
            pos: vec3(0.0, 1.0, 0.0),
            prev: vec3(0.0, 1.0, 0.0),
            start: vec3(0.0, 1.0, 0.0),
            vel: vec3(0.0, 0.0, seg_len / DT),
            team: 0,
            owner_slot: 0,
            age: 0.0,
            life: 1.4,
            straight: 99.0,
            radius: 0.9,
            size: 0.15,
            damage: rw.flick_damage_near,
            grav: 0.0,
            drag: 0.0,
            trail: 0.0,
            trail_every: 0.0,
            trail_radius: 0.45,
            seed: 0.5,
            blast: false,
            pellet: None,
            burst: None,
            ghost: false,
            below_falloff: 0.0,
            below_falloff_max: 0.0,
            volley: None,
            dmg_far: Some(rw.flick_damage_far),
            delay: 0.0,
            sp: None,
        });
        let mut v = Actor::new(1, 1, &f.t);
        v.spawn_at(vec3(0.0, 0.05, z), 0.0, &f.sim(), &f.t);
        v.set_invuln(0.0);
        v.vel = Vec3::ZERO;
        ps.step_masked(
            DT,
            &f.world,
            &mut [v],
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
        ps.drain_events()
            .iter()
            .find_map(|e| match e {
                SimEvent::Hit { damage, .. } => Some(*damage),
                _ => None,
            })
            .unwrap_or(f32::NAN)
    };

    // The capsule sampler steps t in 1/6 increments and the quick-reject
    // gate measures against the *moved* round, so each segment is chosen so
    // a sample lands on the victim axis. The ternary refine keeps a ~0.4
    // damage wobble (JS `Physics.segmentCapsuleDist` behaves the same).
    assert!((hit_at(0.0, 1.0) - 125.0).abs() <= 0.5, "0 m → near");
    assert!(
        (hit_at(3.5, 4.2) - 77.5).abs() <= 0.5,
        "3.5 m → mid (5/6 of 4.2)"
    );
    assert!(
        (hit_at(7.0, 8.4) - 30.0).abs() <= 0.5,
        "7 m → far (5/6 of 8.4)"
    );
    assert!(
        (hit_at(10.0, 10.0) - 30.0).abs() <= 0.5,
        "beyond 7 m stays far"
    );
}

// The delay gate (JS `_step` L1571): a still-delayed round counts down only —
// no age, no integration, no hit test. The blade crescent staggers 0.018 s.
#[test]
fn tr32_2_blade_drop_delay_freezes_the_first_frames() {
    let f = fix();
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(0xDD);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate(false),
        Vec3::ZERO,
    );
    // The swing advances only from frame 2 — the crescent leaves on frame 3.
    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate(false),
        Vec3::ZERO,
    );
    assert_eq!(ps.list.len(), 7);
    let frozen = ps.list[6].pos;
    ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    assert_eq!(ps.list[6].age, 0.0, "delay 0.018 > dt → frozen");
    assert_eq!(ps.list[6].pos, frozen, "a frozen round does not move");
    assert!(ps.list[0].age > 0.0, "delay 0 steps immediately");
    ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w, &f.sb, None);
    assert!(
        ps.list[6].age > 0.0,
        "the last drop launches once its delay clears"
    );
}

// The roller drum crush: gated on roll speed (hs > 1.0), the fwd/lat box
// bounds, and the 0.5 s per-victim window.
#[test]
fn tr32_2_roller_melee_gates_and_window() {
    let f = fix();

    let crush = |vx: f32, vz: f32, vx_lat: f32| -> usize {
        let mut actors = vec![kid(&f), {
            let mut v = Actor::new(1, 4, &f.t);
            v.spawn_at(vec3(vx_lat, 0.05, vz), 0.0, &f.sim(), &f.t);
            v.set_invuln(0.0);
            v
        }];
        actors[0].grounded = true;
        actors[0].vel = vec3(0.0, 0.0, vx);
        let mut ps = ProjectileSim::new(5);
        let mut pg = PaintGrid::new(&f.world);
        let mut r = WeaponRunner::new();
        let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
        r.update(
            DT,
            &gate(true),
            me,
            WeaponKind::Roller,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut foes,
            &mut pg,
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        ps.drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Hit { .. }))
            .count()
    };

    assert_eq!(
        crush(1.0, 0.5, 0.0),
        0,
        "hs = 1.0 is not rolling fast enough"
    );
    assert_eq!(crush(1.01, 0.5, 0.0), 1, "hs > 1.0 crushes for 140");
    assert_eq!(crush(1.01, -0.2, 0.0), 0, "fwd = −0.2 is outside");
    assert_eq!(crush(1.01, -0.19, 0.0), 1, "fwd > −0.2");
    assert_eq!(crush(1.01, 1.35, 0.0), 0, "fwd = 1.35 is outside");
    assert_eq!(crush(1.01, 1.34, 0.0), 1, "fwd < 1.35");
    assert_eq!(crush(1.01, 0.5, 1.3), 0, "lat = width/2+0.35 is outside");
    assert_eq!(crush(1.01, 0.5, 1.29), 1, "lat < width/2+0.35");

    // The 0.5 s window: the same victim is not re-crushed until it decays.
    let mut actors = vec![kid(&f), {
        let mut v = Actor::new(1, 4, &f.t);
        v.spawn_at(vec3(0.0, 0.05, 0.5), 0.0, &f.sim(), &f.t);
        v.set_invuln(0.0);
        v
    }];
    actors[0].grounded = true;
    actors[0].vel = vec3(0.0, 0.0, 1.01);
    let mut ps = ProjectileSim::new(6);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut hits = 0usize;
    for i in 0..40 {
        // The drum crush (140) one-shots the kid — keep the victim standing
        // so the 0.5 s window, not death, gates the re-crush.
        actors[1].alive = true;
        actors[1].hp = f.t.hp;
        let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
        r.update(
            DT,
            &gate(true),
            me,
            WeaponKind::Roller,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut foes,
            &mut pg,
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        hits += ps
            .drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Hit { .. }))
            .count();
        if i == 5 {
            assert_eq!(hits, 1, "the window keeps the victim alive for 0.5 s");
        }
    }
    assert_eq!(hits, 2, "the window decays after 30 frames → one re-crush");
}

// The brush melee: no speed gate, the fwd < 1.2 box and the 0.35 s window.
#[test]
fn tr32_2_brush_melee_box_and_cd() {
    let f = fix();
    let bw = &f.w.brush;
    let mut actors = vec![kid(&f), {
        let mut v = Actor::new(1, 4, &f.t);
        v.spawn_at(vec3(0.0, 0.05, 1.19), 0.0, &f.sim(), &f.t);
        v.set_invuln(0.0);
        v
    }];
    actors[0].grounded = true;
    actors[0].vel = vec3(0.0, 0.0, 7.4);
    let mut ps = ProjectileSim::new(8);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut hits = 0usize;
    for i in 0..40 {
        let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
        r.update(
            DT,
            &gate(true),
            me,
            WeaponKind::Brush,
            &f.w,
            &f.sb,
            vec3(0.0, 0.0, 1.0),
            &f.world,
            &mut ps,
            &mut foes,
            &mut pg,
            &f.t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        let ev = ps.drain_events();
        for e in &ev {
            if let SimEvent::Hit { damage, .. } = e {
                assert!((*damage - bw.brush_damage).abs() <= MM, "30 per brush hit");
                hits += 1;
            }
        }
        let _ = i;
    }
    assert_eq!(hits, 2, "0.35 s window → re-brush after 21 frames");

    // fwd = 1.2 is outside the box.
    let mut actors2 = vec![kid(&f), {
        let mut v = Actor::new(1, 4, &f.t);
        v.spawn_at(vec3(0.0, 0.05, 1.2), 0.0, &f.sim(), &f.t);
        v.set_invuln(0.0);
        v
    }];
    actors2[0].grounded = true;
    actors2[0].vel = vec3(0.0, 0.0, 7.4);
    let mut ps2 = ProjectileSim::new(9);
    let mut pg2 = PaintGrid::new(&f.world);
    let mut r2 = WeaponRunner::new();
    let (me, mut foes) = FoeScan::split_around(&mut actors2, 0);
    r2.update(
        DT,
        &gate(true),
        me,
        WeaponKind::Brush,
        &f.w,
        &f.sb,
        vec3(0.0, 0.0, 1.0),
        &f.world,
        &mut ps2,
        &mut foes,
        &mut pg2,
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
        ps2.drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Hit { .. }))
            .count(),
        0,
        "fwd = 1.2 is outside"
    );
}

// The blade melee box: reach + hr exact, the arc half-angle boundary, the
// face-hug exemption, LOS occlusion and one hit per swing.
#[test]
fn tr32_2_blade_melee_bounds_arc_los_and_once() {
    let f = fix();
    let bl = &f.w.blade;
    let hr = f.t.radius;
    let half = bl.melee_arc_deg * (std::f32::consts::PI / 180.0) * 0.5;

    let cut = |world: &CollisionWorld, vpos: Vec3| -> usize {
        let mut actors = vec![kid(&f), {
            let mut v = Actor::new(1, 4, &f.t);
            v.pos = vpos;
            v
        }];
        actors[0].grounded = true;
        let mut ps = ProjectileSim::new(11);
        let mut pg = PaintGrid::new(world);
        let mut r = WeaponRunner::new();
        let mut hits = 0usize;
        // The swing is born on the press frame; its melee window opens on
        // frame 2 (t crosses 0.01).
        for i in 0..2 {
            let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
            r.update(
                DT,
                &if i == 0 { gate_press() } else { gate(false) },
                me,
                WeaponKind::Blade,
                &f.w,
                &f.sb,
                Vec3::ZERO,
                world,
                &mut ps,
                &mut foes,
                &mut pg,
                &f.t,
                None,
                i == 0,
                0.0,
                &mut Vec::new(),
                &mut Vec::new(),
                false,
                false,
            );
            hits += ps
                .drain_events()
                .iter()
                .filter(|e| matches!(e, SimEvent::Hit { damage, .. } if (*damage - bl.tap_melee).abs() <= MM))
                .count();
        }
        hits
    };

    // Reach boundary: d = reach + hr exactly is inside (`d > reach+hr` skips).
    assert_eq!(cut(&f.world, vec3(0.0, 0.05, bl.reach + hr)), 1);
    assert_eq!(cut(&f.world, vec3(0.0, 0.05, bl.reach + hr + 0.01)), 0);
    // Arc boundary: 75° ± ε.
    let d = 1.0;
    let inside = half - 1e-4;
    let outside = half + 1e-4;
    assert_eq!(
        cut(&f.world, vec3(inside.sin() * d, 0.05, inside.cos() * d)),
        1,
        "ang < half"
    );
    assert_eq!(
        cut(&f.world, vec3(outside.sin() * d, 0.05, outside.cos() * d)),
        0,
        "ang > half"
    );
    // Face hug: behind the yaw but within hr + 0.4 and fwd > −0.15 → exempt.
    assert_eq!(
        cut(&f.world, vec3(0.0, 0.05, -0.1)),
        1,
        "the face-hug exemption"
    );

    // LOS occlusion: a thin wall between the eye and the victim.
    let w2 = CollisionWorld::from_brushes(
        &[
            bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
            bx([-0.1, 0.0, 1.0], [0.1, 2.0, 1.1]),
        ],
        B,
    );
    assert_eq!(cut(&w2, vec3(0.0, 0.05, 1.3)), 0, "the wall blocks the cut");

    // One hit per swing: the window spans ~7 frames, the victim is cut once.
    let mut actors = vec![kid(&f), {
        let mut v = Actor::new(1, 4, &f.t);
        v.pos = vec3(0.0, 0.05, 0.8);
        v
    }];
    actors[0].grounded = true;
    let mut ps = ProjectileSim::new(12);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut hits = 0usize;
    for i in 0..12 {
        let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
        r.update(
            DT,
            &if i == 0 { gate_press() } else { gate(false) },
            me,
            WeaponKind::Blade,
            &f.w,
            &f.sb,
            Vec3::ZERO,
            &f.world,
            &mut ps,
            &mut foes,
            &mut pg,
            &f.t,
            None,
            i == 0,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        // Count only the melee hits (35): the crescent drops fired on frame 2
        // carry their own damage values and must not confuse the tally.
        hits += ps
            .drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Hit { damage, .. } if (*damage - bl.tap_melee).abs() <= MM))
            .count();
    }
    assert_eq!(hits, 1, "the swing's hit set pierces each victim once");
}

// The heavy lunge: three ground samples along the dash must stay above the
// sea — at the deck edge the cut lands without the lunge.
#[test]
fn tr32_2_blade_lunge_safe_samples() {
    let f = fix();
    let bl = &f.w.blade;

    let charge_heavy = |a: &mut Actor| -> f32 {
        let mut ps = ProjectileSim::new(13);
        let mut pg = PaintGrid::new(&f.world);
        let mut r = WeaponRunner::new();
        frame(
            &f,
            &mut r,
            a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate_press(),
            Vec3::ZERO,
        );
        let mut n = 0;
        while r.charge < 1.0 && n < 200 {
            frame(
                &f,
                &mut r,
                a,
                WeaponKind::Blade,
                &mut ps,
                &mut pg,
                &gate(true),
                Vec3::ZERO,
            );
            n += 1;
        }
        frame(
            &f,
            &mut r,
            a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
        r.blade.as_ref().expect("kit").lunge
    };

    // Mid-deck facing +Z: all three samples land on the deck → lunge.
    let mut a = kid(&f);
    a.grounded = true;
    assert!((charge_heavy(&mut a) - bl.lunge_time).abs() <= MM);

    // 9.5 m from the east wall facing +X: the k = 1.4 sample (10.97 m) is
    // over the sea → no lunge.
    let mut a2 = Actor::new(0, 0, &f.t);
    a2.spawn_at(
        vec3(9.5, 0.05, 0.0),
        std::f32::consts::FRAC_PI_2,
        &f.sim(),
        &f.t,
    );
    a2.grounded = true;
    assert_eq!(charge_heavy(&mut a2), 0.0, "lungeSafe rejects the ledge");
}

// A ghost wave replays the motion and the rng draws, but never paints and
// never hurts, and dies on the same 12 m range.
#[test]
fn tr32_2_ghost_wave_is_silent() {
    let f = fix();
    let bl = &f.w.blade;
    let mut victim = Actor::new(1, 4, &f.t);
    victim.pos = vec3(0.0, 0.05, 3.0);
    victim.grounded = true;
    let hp0 = victim.hp;
    let mut ps = ProjectileSim::new(0x6057);
    let mut pg = PaintGrid::new(&f.world);
    let s0 = ps.rng.s;
    let mut wave = BladeWave::new(0, 0, vec3(0.0, 0.934, -6.0), vec3(0.0, 0.0, 1.0), true);

    let mut events = 0usize;
    let mut removed_at = None;
    let mut vs = vec![victim];
    for i in 1..=40 {
        let alive = wave.step(DT, &f.world, &mut pg, bl, &f.t, &mut ps, &mut vs, None);
        events += ps.drain_events().len();
        if !alive {
            removed_at = Some(i);
            break;
        }
    }
    assert_eq!(
        events, 0,
        "a ghost emits nothing (no Turf, no Impact, no Hit)"
    );
    assert_eq!(vs[0].hp, hp0, "a ghost hurts nobody");
    assert_eq!(
        removed_at,
        Some(37),
        "the same 28-frame run + 9 fade frames"
    );
    // The draws still happen (JS `mute` suppresses effects, not calls).
    assert_eq!(rng_draws(s0, ps.rng.s), 29);
}

// The swing struct sanity: the kit's `Swing` rides the runner between frames
// and the runner reports busy while it cuts.
#[test]
fn tr32_2_blade_busy_and_swing_window() {
    let f = fix();
    let mut a = kid(&f);
    a.grounded = true;
    let mut ps = ProjectileSim::new(21);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    assert!(!r.busy());
    frame(
        &f,
        &mut r,
        &mut a,
        WeaponKind::Blade,
        &mut ps,
        &mut pg,
        &gate_press(),
        Vec3::ZERO,
    );
    assert!(r.busy(), "a live swing is busy");
    let kit = r.blade.as_ref().expect("kit");
    let sw: &Swing = kit.swing.as_ref().expect("swing");
    // `run_swing` advances before the press handling, so the swing born on
    // this frame still carries t = 0.
    assert!(!sw.heavy && sw.side == 1 && sw.t.abs() <= MM);
    // The swing lives until t > 0.4, then the kit idles.
    for _ in 0..26 {
        frame(
            &f,
            &mut r,
            &mut a,
            WeaponKind::Blade,
            &mut ps,
            &mut pg,
            &gate(false),
            Vec3::ZERO,
        );
    }
    assert!(
        r.blade.as_ref().unwrap().swing.is_none(),
        "the swing expires at 0.4 s"
    );
    assert!(!r.busy());
}
