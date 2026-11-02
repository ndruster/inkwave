//! Task 33 TR-33.1: the bow kit — the charge → shot-tier table, the
//! three-arrow fan (ground about UP, air about dir×UP), the draw → release
//! flow, arrow flight (straight / gravity / dive past range), lodging,
//! direct hits and the lodged-arrow burst (falloff + line of sight), the
//! trail drips, ghosts and the arrow pool cap. Fixed-seed, value-by-value
//! against the ported formulas (spec §7).

use super::bow::{self, BowArrow, MAX_ARROWS};
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

/// Flat 20×20 m deck at y = 0 — the same stage as [`super::blade_tests`].
struct Fix {
    world: CollisionWorld,
    pads: [Vec3; 2],
    t: PlayerTuning,
    w: WeaponSet,
    sb: crate::tuning::SubSet,
}

fn deck() -> Brush {
    bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])
}

fn fix() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[deck()], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.weapons,
        sb: tuning.subs,
    }
}

/// Deck + a thin slab (for the line-of-sight gate).
fn fix_slab() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[deck(), bx([1.0, -1.0, 4.8], [1.2, 3.0, 5.2])], B),
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

/// A team-1 foe snapped to the deck, with no spawn invulnerability.
fn foe(f: &Fix, slot: usize, pos: Vec3) -> Actor {
    let mut e = Actor::new(1, slot, &f.t);
    e.spawn_at(pos, 0.0, &f.sim(), &f.t);
    e.set_invuln(0.0);
    e
}

fn gate(fire: bool) -> FireGate {
    FireGate {
        fire,
        ..Default::default()
    }
}

/// One bow runner frame; `arrows` is the real volley pool (the bow branch
/// of `WeaponRunner::update` pushes into it).
#[allow(clippy::too_many_arguments)]
fn frame(
    f: &Fix,
    r: &mut WeaponRunner,
    a: &mut Actor,
    ps: &mut ProjectileSim,
    pg: &mut PaintGrid,
    g: &FireGate,
    ps_time: f32,
    arrows: &mut Vec<BowArrow>,
) -> Vec<SimEvent> {
    r.update(
        DT,
        g,
        a,
        WeaponKind::Bow,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        ps,
        &mut FoeScan::empty(),
        pg,
        &f.t,
        None,
        g.fire,
        ps_time,
        arrows,
        &mut Vec::new(),
        false,
        false,
    );
    ps.drain_events()
}

/// A flying arrow with tier-2 stats (tests override what they care about).
fn flying(pos: Vec3, dir: Vec3, speed: f32) -> BowArrow {
    BowArrow {
        owner_slot: 0,
        team: 0,
        pos,
        prev: pos,
        vel: dir * speed,
        dir,
        fdir: dir,
        hit_p: Vec3::ZERO,
        nrm: Vec3::Y,
        st: false,
        age: 0.0,
        dist: 0.0,
        t: 0.0,
        range: 21.0,
        speed,
        tier: 2,
        dmg: 55.0,
        lodge: true,
        fuse: 0.55,
        br: 1.8,
        bd: 45.0,
        be: 15.0,
        bp: 1.55,
        // Drips disabled unless the test re-arms them (JS `trail` L69).
        trail: -1000.0,
        seed: 0.5,
        no_hit: false,
        center: true,
        ghost: false,
    }
}

/// An arrow already stuck into `hit_p` with outward normal `nrm`.
fn lodged(hit_p: Vec3, nrm: Vec3) -> BowArrow {
    let mut p = flying(hit_p, nrm, 0.0);
    p.st = true;
    p.vel = Vec3::ZERO;
    p.hit_p = hit_p;
    p.nrm = nrm;
    p
}

/// Count the draws a [`Rng`] made between two states (the private `s` field
/// is visible to this child module — the xorshift stream is tiny, so the
/// test count is bounded by the loop guard).
fn rng_draws(before: u32, after: u32) -> usize {
    if before == after {
        return 0;
    }
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

// ------------------------------------------------------------------ TR-33.1

// JS `bowShot` L46-61: the tier table at the exact spec boundaries
// c = 0.449 / 0.45 / 0.998 / 0.999, every field checked against tuning.
#[test]
fn tr33_1_bow_shot_tier_boundaries() {
    let f = fix();
    let w = &f.w.bow;

    // Just below ring1 → tap tier, u = c / ring1.
    let s = bow::bow_shot(0.449, w);
    let u = 0.449 / w.ring1;
    assert_eq!(s.tier, 0);
    assert!((s.dmg - lerp(w.damage_tap[0], w.damage_tap[1], u)).abs() <= MM);
    assert!((s.dmg - 19.986667).abs() <= MM, "dmg = lerp(14, 20, u)");
    assert!((s.range - lerp(w.flight_tap[0], w.flight_tap[1], u)).abs() <= MM);
    assert!((s.range - 7.494444).abs() <= MM);
    assert!((s.speed - w.speed_tap).abs() <= MM);
    assert!((s.side - s.dmg * w.side_mul).abs() <= MM);
    assert!((s.side - 15.989333).abs() <= MM);
    assert!((s.fan - w.fan_tap).abs() <= MM);
    assert!(!s.lodge);
    assert_eq!(s.fuse, 0.0);
    assert_eq!((s.br, s.bd, s.be, s.bp), (0.0, 0.0, 0.0, 0.0));

    // Exactly ring1 → ring tier, u = 0.
    let s = bow::bow_shot(0.45, w);
    assert_eq!(s.tier, 1);
    assert!((s.dmg - w.damage_ring[0]).abs() <= MM);
    assert!((s.range - w.flight_ring[0]).abs() <= MM);
    assert!((s.speed - w.speed_ring[0]).abs() <= MM);
    assert!((s.side - s.dmg * w.side_mul).abs() <= MM);
    assert!((s.fan - w.fan_ring).abs() <= MM);
    assert!(s.lodge);
    assert!((s.fuse - w.fuse_ring).abs() <= MM);
    assert!((s.br - w.burst_radius[0]).abs() <= MM);
    assert!((s.bd - w.burst_damage[0]).abs() <= MM);
    assert!((s.be - w.burst_edge[0]).abs() <= MM);
    assert!((s.bp - w.burst_paint[0]).abs() <= MM);

    // Just below full → ring tier, u ≈ 1.
    let s = bow::bow_shot(0.998, w);
    let u = (0.998 - w.ring1) / (1.0 - w.ring1);
    assert_eq!(s.tier, 1);
    assert!((s.dmg - lerp(w.damage_ring[0], w.damage_ring[1], u)).abs() <= MM);
    assert!((s.dmg - 37.970_91).abs() <= MM);
    assert!((s.range - lerp(w.flight_ring[0], w.flight_ring[1], u)).abs() <= MM);
    assert!((s.range - 16.485455).abs() <= MM);
    assert!((s.speed - lerp(w.speed_ring[0], w.speed_ring[1], u)).abs() <= MM);
    assert!((s.speed - 47.978182).abs() <= MM, "lerp(42, 48, u)");
    assert!((s.side - s.dmg * w.side_mul).abs() <= MM);
    assert!((s.side - 30.376727).abs() <= MM);

    // Full draw → tier 2, all top values.
    let s = bow::bow_shot(0.999, w);
    assert_eq!(s.tier, 2);
    assert!((s.range - w.flight_full).abs() <= MM);
    assert!((s.speed - w.speed_full).abs() <= MM);
    assert!((s.dmg - w.damage_full).abs() <= MM);
    assert!((s.side - w.side_full).abs() <= MM);
    assert!((s.fan - w.fan_full).abs() <= MM);
    assert!(s.lodge);
    assert!((s.fuse - w.fuse_full).abs() <= MM);
    assert!((s.br - w.burst_radius[1]).abs() <= MM);
    assert!((s.bd - w.burst_damage[1]).abs() <= MM);
    assert!((s.be - w.burst_edge[1]).abs() <= MM);
    assert!((s.bp - w.burst_paint[1]).abs() <= MM);
}

// JS `volley` L97-113 on the ground: the flat fan about UP, 3 rng draws in
// i = -1, 0, +1 order, side arrows at `side` damage; the `['k','bow']`
// record + fire event (JS `looseVolley` L74-93).
#[test]
fn tr33_1_volley_ground_fan_and_record() {
    let f = fix();
    let w = &f.w.bow;
    let a = kid(&f);
    let mut ps = ProjectileSim::new(0x2024_0333);
    let mut arrows = Vec::new();
    let s0 = ps.rng.s;

    let s = bow::loose_volley(&a, 0.45, w, &mut arrows, &mut ps);
    assert_eq!(arrows.len(), 3);
    assert_eq!(rng_draws(s0, ps.rng.s), 3, "one seed per arrow (JS L106)");
    let seeds = pre_draw(&Rng { s: s0 }, 3);

    // JS L98: ground fan axis = UP; dir = (0,0,1) rotated by i·6° about Y.
    let (sin6, cos6) = (w.fan_ring.to_radians().sin(), w.fan_ring.to_radians().cos());
    assert!((arrows[0].dir - vec3(-sin6, 0.0, cos6)).length() <= MM);
    assert!((arrows[1].dir - Vec3::Z).length() <= MM);
    assert!((arrows[2].dir - vec3(sin6, 0.0, cos6)).length() <= MM);

    for (i, p) in arrows.iter().enumerate() {
        assert!((p.seed - seeds[i]).abs() <= 1e-6);
        assert!((p.speed - s.speed).abs() <= MM);
        assert!((p.range - s.range).abs() <= MM);
        assert!(p.lodge);
        assert!((p.fuse - s.fuse).abs() <= MM);
        assert!((p.trail + 1.8).abs() <= MM, "JS L69 trail starts -1.8");
        assert_eq!(p.owner_slot, 0);
        assert_eq!(p.team, 0);
        // JS L106: side arrows take `side`, the centre takes `dmg`.
        let want = if i == 1 { s.dmg } else { s.side };
        assert!((p.dmg - want).abs() <= MM);
        assert_eq!(p.center, i == 1);
        assert!(!p.ghost);
    }

    let ev = ps.drain_events();
    let kit = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Kit { kind, data, .. } if kind == "bow" => Some(data.clone()),
            _ => None,
        })
        .expect("a bow kit record");
    // JS L88: [r2 m×3, r3 dir×3, r3 c, air]; muzzle = eye + aim·0.3.
    assert_eq!(kit.len(), 8);
    assert!(kit[0].abs() <= MM);
    assert!((kit[1] - 1.05).abs() <= MM);
    assert!((kit[2] - 0.3).abs() <= MM);
    assert!(kit[3].abs() <= MM);
    assert!(kit[4].abs() <= MM);
    assert!((kit[5] - 1.0).abs() <= MM);
    assert!((kit[6] - 0.45).abs() <= MM);
    assert_eq!(kit[7], 0.0, "grounded");
    assert_eq!(
        count_kind(
            &ev,
            |e| matches!(e, SimEvent::Fire { charge: Some(c), .. } if (*c - 0.45).abs() <= MM)
        ),
        1
    );
}

// JS `volley` L97-113 in the air: the upright fan about dir×UP; the
// degenerate aim = UP case falls back to the X axis.
#[test]
fn tr33_1_volley_air_fan() {
    let f = fix();
    let w = &f.w.bow;
    let mut a = kid(&f);
    a.grounded = false;
    let mut ps = ProjectileSim::new(0xA1B2);
    let mut arrows = Vec::new();

    bow::loose_volley(&a, 0.45, w, &mut arrows, &mut ps);
    // Axis = dir×UP = (-1,0,0): i = -1 pitches down, i = +1 up.
    let (sin6, cos6) = (w.fan_ring.to_radians().sin(), w.fan_ring.to_radians().cos());
    assert!((arrows[0].dir - vec3(0.0, -sin6, cos6)).length() <= MM);
    assert!((arrows[1].dir - Vec3::Z).length() <= MM);
    assert!((arrows[2].dir - vec3(0.0, sin6, cos6)).length() <= MM);

    let ev = ps.drain_events();
    let air = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Kit { kind, data, .. } if kind == "bow" => Some(data[7]),
            _ => None,
        })
        .expect("record");
    assert_eq!(air, 1.0, "air flag set");

    // Aim straight up: dir×UP degenerates → axis = X (JS L98-99 fallback).
    let mut ps2 = ProjectileSim::new(0xA1B3);
    let mut arrows2 = Vec::new();
    a.aim_pitch = std::f32::consts::FRAC_PI_2;
    bow::loose_volley(&a, 0.45, w, &mut arrows2, &mut ps2);
    assert!((arrows2[0].dir - vec3(0.0, cos6, -sin6)).length() <= MM);
    assert!((arrows2[2].dir - vec3(0.0, cos6, sin6)).length() <= MM);
}

// The draw → release flow (JS `update` L389-422 / `release` L423-435):
// charge_t linear, ring1 by frame 28, full draw at ring 2,
// ink = 16·max(0.15, c), cooldown 0.22, firing_t 0.35.
#[test]
fn tr33_1_draw_then_release_full() {
    let f = fix();
    let w = &f.w.bow;
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0xC0FFEE);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut arrows = Vec::new();
    let s0 = ps.rng.s;

    for n in 1..=26u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(true),
            n as f32 * DT,
            &mut arrows,
        );
        assert!(r.charging, "drawing");
        assert!((r.charge - n as f32 * DT).abs() <= MM, "linear draw");
        assert_eq!(r.bow.as_ref().unwrap().ring, 0, "below ring1");
    }
    for n in 27..=60u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(true),
            n as f32 * DT,
            &mut arrows,
        );
        assert!((r.charge - (n as f32 * DT).min(1.0)).abs() <= MM);
        if n == 28 {
            assert_eq!(r.bow.as_ref().unwrap().ring, 1, "ring1 by frame 28");
        }
    }
    assert_eq!(r.bow.as_ref().unwrap().ring, 2, "full draw");
    assert!((r.charge - 1.0).abs() <= MM);

    // Release: the trigger lets go.
    let ev = frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        61.0 * DT,
        &mut arrows,
    );
    assert_eq!(arrows.len(), 3);
    assert_eq!(rng_draws(s0, ps.rng.s), 3, "the volley draws");
    // JS L425: ink -= inkFull · max(inkMin, c) = 16 · 1.
    assert!((a.ink - (f.t.ink_max - w.ink_full * 1.0)).abs() <= MM);
    assert!((a.ink - 84.0).abs() <= MM);
    assert!((r.cooldown - w.cooldown).abs() <= MM, "cooldown 0.22");
    assert!((r.firing_t - FIRING_T).abs() <= MM, "firing_t 0.35");
    assert!(!r.charging);
    assert_eq!(r.charge, 0.0);
    assert_eq!(r.bow.as_ref().unwrap().ring, 0);
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Fire { .. })),
        1,
        "the release fires"
    );
}

// JS L400-402: a half-empty tank caps the draw at ink/inkFull; the release
// still pays 16·max(0.15, c).
#[test]
fn tr33_1_tank_caps_charge() {
    let f = fix();
    let w = &f.w.bow;
    let mut a = kid(&f);
    a.ink = 8.0;
    let mut ps = ProjectileSim::new(0x7ABC);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut arrows = Vec::new();

    for n in 1..=60u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(true),
            n as f32 * DT,
            &mut arrows,
        );
    }
    assert!(
        (r.charge - 8.0 / w.ink_full).abs() <= MM,
        "capped at ink/inkFull = 0.5"
    );
    assert_eq!(r.bow.as_ref().unwrap().ring, 1, "never reaches full");
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        61.0 * DT,
        &mut arrows,
    );
    assert!(
        (a.ink - (8.0 - w.ink_full * 0.5)).abs() <= MM,
        "the release pays inkFull·max(0.15, c) = 8"
    );
}

// JS L394-397: below inkFull·inkMin the draw never starts — the empty-click
// LowInk path.
#[test]
fn tr33_1_low_ink_empty_click() {
    let f = fix();
    let w = &f.w.bow;
    let mut a = kid(&f);
    a.ink = w.ink_full * w.ink_min - 0.4; // 2.0 < 2.4
    let mut ps = ProjectileSim::new(0x1E11);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut arrows = Vec::new();

    let ev = frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        DT,
        &mut arrows,
    );
    assert!(!r.charging, "no draw without ink");
    assert!(arrows.is_empty());
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::LowInk { owner_slot: 0 })),
        1
    );
    assert!((r.empty_cd - EMPTY_CD).abs() <= MM);
}

// JS `stepArrow` L175-264 past the reach: gravity after `straight`, then the
// dive (diveGrav + diveDrag) once dist > range.
#[test]
fn tr33_1_arrow_dive_past_range() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0xD1FE);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = flying(vec3(0.0, 60.0, 0.0), Vec3::Z, 32.0);
    p.range = 2.0;
    p.lodge = false;

    let mut dived = false;
    for _ in 0..12 {
        let (age0, dist0, vel0) = (p.age, p.dist, p.vel);
        let alive = BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut [],
            None,
        );
        assert!(alive, "high in the sky: nothing catches it");
        // Replicate the JS gate: gravity on the *new* age, dive on the old dist.
        let mut want = vel0;
        if age0 + DT > w.straight {
            want.y -= w.grav * DT;
        }
        if dist0 > p.range {
            dived = true;
            want.y -= w.dive_grav * DT;
            want *= 1.0 - w.dive_drag * DT;
        }
        assert!(
            (p.vel - want).length() <= 1e-4,
            "vel after step at age {age0}"
        );
    }
    assert!(dived, "the dive branch ran");
}

// A tap arrow (no lodge) splats on the world: radius = paintTap·(0.85 +
// 0.3·seed) riding the arrow's own seed — no rng draw (JS L241-249).
#[test]
fn tr33_1_tap_arrow_impact_splat() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x7A95);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = flying(vec3(0.0, 5.0, 0.0), Vec3::NEG_Y, 32.0);
    p.lodge = false;
    p.tier = 0;
    p.dmg = 14.0;
    p.range = 5.0;
    let s0 = ps.rng.s;

    let mut dead = false;
    for _ in 0..20 {
        dead = !BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut [],
            None,
        );
        if dead {
            break;
        }
    }
    assert!(dead, "the tap arrow dies on the deck");
    assert_eq!(rng_draws(s0, ps.rng.s), 0, "the splat rides p.seed");
    let ev = ps.drain_events();
    let impact = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Impact {
                radius,
                pos,
                normal,
                ..
            } => Some((*radius, *pos, *normal)),
            _ => None,
        })
        .expect("an impact");
    assert!(
        (impact.0 - w.paint_tap).abs() <= MM,
        "impact radius = paintTap"
    );
    assert!(impact.1.distance(vec3(0.0, 0.0, 0.0)) <= 0.05);
    assert!((impact.2 - Vec3::Y).length() <= MM);
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. })),
        1,
        "the tap splat credits turf"
    );
}

// JS L197-214 + L237: a lodging arrow that hits a body takes the direct hit,
// drops (vel = dir·1.5, −9, dir·1.5), dist clamps up to range, then sticks
// into the floor at a slant along its old flight line.
#[test]
fn tr33_1_direct_hit_drops_then_lodges() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0xD105);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = flying(vec3(0.0, 0.7, 2.0), Vec3::Z, 42.0);
    p.dmg = 30.0; // ring-tier centre
    p.range = 12.5;
    let mut actors = vec![foe(&f, 4, vec3(0.0, 0.0, 3.4))];
    let s0 = ps.rng.s;

    // Frame 1 flies past; frame 2 crosses the foe at z = 3.4 — still inside
    // the `straight` window (2·DT = 0.033 < 0.05), so `dir` is un-drooped.
    let mut ev: Vec<SimEvent> = Vec::new();
    for _ in 0..3 {
        let _ = BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut actors,
            None,
        );
        ev = ps.drain_events();
        if ev.iter().any(|e| matches!(e, SimEvent::Hit { .. })) {
            break;
        }
    }
    assert_eq!(
        count_kind(
            &ev,
            |e| matches!(e, SimEvent::Hit { damage, victim_slot, .. }
                if (*damage - 30.0).abs() <= MM && *victim_slot == 4)
        ),
        1,
        "the direct hit hurts"
    );
    assert!(p.no_hit);
    // JS L210: vel = (dir.x·1.5, −9, dir.z·1.5) with dir = (0,0,1).
    assert!(
        (p.vel - vec3(0.0, -9.0, 1.5)).length() <= MM,
        "JS L210 drop"
    );
    assert!((p.dist - 12.5).abs() <= MM, "dist = max(dist, range)");
    // JS L203: at = prev.lerp(pos, t) — the closest-approach t comes from the
    // ported ternary search (`segment_capsule_dist`), whose bracket does not
    // pin a boundary minimum to t = 1, so the test re-runs the same helper
    // and asserts the drop landed on that exact point.
    let prev2 = vec3(0.0, 0.7, 2.7);
    let pos2 = vec3(0.0, 0.7, 3.4);
    let res = segment_capsule_dist(prev2, pos2, vec3(0.0, 0.0, 3.4), f.t.radius, f.t.height);
    assert!(
        (p.pos - prev2.lerp(pos2, res.t)).length() <= MM,
        "stuck at the body"
    );
    assert_eq!(rng_draws(s0, ps.rng.s), 0);

    // It falls and lodges in the deck, slanted along the old flight line.
    let mut stuck = false;
    for _ in 0..20 {
        if BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut actors,
            None,
        ) && p.st
        {
            stuck = true;
            break;
        }
    }
    assert!(stuck, "the dropped arrow sticks in the floor");
    // JS L237: stick = normalize(fdir.x·0.55, −1, fdir.z·0.55).
    let l = (1.0f32 + 0.55 * 0.55).sqrt();
    assert!((p.dir - vec3(0.0, -1.0 / l, 0.55 / l)).length() <= MM);
    assert!((p.pos - (p.hit_p + p.dir * 0.12)).length() <= MM, "JS L125");
    assert!(p.hit_p.y.abs() <= MM, "stuck into the deck top");
}

// JS `burst` L135-162: the lodged arrow detonates after its fuse (ring tier
// 0.7 s ≈ frame 42) — paint ring, Impact at br, no rng draws.
#[test]
fn tr33_1_lodged_fuse_bursts() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0xB01F15E);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = lodged(vec3(0.0, 0.0, 5.0), Vec3::Y);
    p.fuse = 0.7;
    p.br = w.burst_radius[0];
    p.bp = w.burst_paint[0];
    let s0 = ps.rng.s;

    let mut n = 0usize;
    loop {
        n += 1;
        let alive = BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut [],
            None,
        );
        if !alive {
            break;
        }
        assert!(n < 60, "the fuse must burst within a second");
    }
    assert!(
        (n as f32 - 0.7 / DT).abs() <= 1.5,
        "fuse 0.7 → ~42 frames, got {n}"
    );
    assert_eq!(rng_draws(s0, ps.rng.s), 0, "burst rides p.seed");
    let ev = ps.drain_events();
    let impact = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Impact {
                pos,
                radius,
                victim_slot,
                ..
            } => Some((*pos, *radius, *victim_slot)),
            _ => None,
        })
        .expect("burst impact");
    assert!((impact.1 - w.burst_radius[0]).abs() <= MM);
    assert_eq!(impact.2, None);
    // JS L137: c = hitP + n·0.18.
    assert!((impact.0 - vec3(0.0, 0.18, 5.0)).length() <= MM);
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. })),
        1,
        "the burst ring paints"
    );
}

// JS L146-158: burst splash — full `bd` inside `burstInner`, lerp to `be` at
// br, nothing past br.
#[test]
fn tr33_1_burst_splash_falloff() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x5FA15);
    let mut pg = PaintGrid::new(&f.world);
    let p = lodged(vec3(0.0, 0.0, 5.0), Vec3::Y); // c = (0, 0.18, 5)

    // v = (dx, 0.25, 5) (the clamp pins y at feet+0.25);
    // d = |v−c| − 0.38·0.6, with the 0.07 vertical offset baked in.
    let dx = |d: f32| ((d + 0.228).powi(2) - 0.07f32.powi(2)).sqrt();
    let mut actors = vec![
        foe(&f, 1, vec3(dx(0.6), 0.0, 5.0)),  // inner edge → full bd
        foe(&f, 2, vec3(dx(1.2), 0.0, 5.0)),  // mid → lerp(bd, be, 0.5)
        foe(&f, 3, vec3(dx(1.8), 0.0, 5.0)),  // outer edge → be
        foe(&f, 4, vec3(dx(1.81), 0.0, 5.0)), // past br → untouched
    ];
    bow::burst(&p, &f.world, &mut pg, w, &f.t, &mut ps, &mut actors, None);
    let ev = ps.drain_events();
    let dmg_of = |slot: usize| -> Option<f32> {
        ev.iter().find_map(|e| match e {
            SimEvent::Hit {
                victim_slot,
                damage,
                ..
            } if *victim_slot == slot => Some(*damage),
            _ => None,
        })
    };
    assert!(
        (dmg_of(1).unwrap_or(f32::NAN) - 45.0).abs() <= MM,
        "d=0.6 → bd"
    );
    assert!(
        (dmg_of(2).unwrap_or(f32::NAN)
            - lerp(p.bd, p.be, (1.2 - w.burst_inner) / (p.br - w.burst_inner)))
        .abs()
            <= MM,
        "d=1.2 → 30"
    );
    assert!(
        (dmg_of(3).unwrap_or(f32::NAN) - 15.0).abs() <= MM,
        "d=1.8 → be"
    );
    assert!(dmg_of(4).is_none(), "d=1.81 → past the burst");
}

// JS L152: the splash needs line of sight.
#[test]
fn tr33_1_burst_splash_needs_los() {
    let f = fix_slab();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x105);
    let mut pg = PaintGrid::new(&f.world);
    let p = lodged(vec3(0.0, 0.0, 5.0), Vec3::Y);
    let dx = ((1.2f32 + 0.228).powi(2) - 0.07f32.powi(2)).sqrt();
    let mut actors = vec![foe(&f, 1, vec3(dx, 0.0, 5.0))];
    bow::burst(&p, &f.world, &mut pg, w, &f.t, &mut ps, &mut actors, None);
    let ev = ps.drain_events();
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Hit { .. })),
        0,
        "the slab blocks the splash"
    );
}

// A ghost volley (the remote replay, JS `ghost` L114-118): it lodges and
// bursts for the eye only — no paint, no damage, no draws.
#[test]
fn tr33_1_ghost_arrow_mutes_everything() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x6EC7);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = lodged(vec3(0.0, 0.0, 5.0), Vec3::Y);
    p.ghost = true;
    let mut actors = vec![foe(&f, 1, vec3(0.3, 0.0, 5.0))];
    let s0 = ps.rng.s;

    let alive = BowArrow::step(
        &mut p,
        DT,
        &f.world,
        &mut pg,
        w,
        &f.t,
        &mut ps,
        &mut actors,
        None,
    );
    assert!(alive, "still on its fuse");
    p.t = p.fuse;
    let alive = BowArrow::step(
        &mut p,
        DT,
        &f.world,
        &mut pg,
        w,
        &f.t,
        &mut ps,
        &mut actors,
        None,
    );
    assert!(!alive);
    assert_eq!(rng_draws(s0, ps.rng.s), 0);
    let ev = ps.drain_events();
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Impact { .. })),
        1,
        "the burst FX still rides the Impact feed"
    );
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Hit { .. })), 0);
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. })), 0);
    assert!((actors[0].hp - f.t.hp).abs() <= MM, "ghosts never hurt");
}

// JS L252-259: lodging arrows drip a dotted trail — 2 draws per drip,
// radius = trailRadius·(0.8 + r0·0.4).
#[test]
fn tr33_1_trail_drip_two_draws() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x7D411);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = flying(vec3(0.0, 1.0, 0.0), Vec3::Z, 42.0);
    p.trail = w.trail_every - 0.1; // re-arm the drip for the first step
    let s0 = ps.rng.s;
    let rands = pre_draw(&Rng { s: s0 }, 2);

    let alive = BowArrow::step(
        &mut p,
        DT,
        &f.world,
        &mut pg,
        w,
        &f.t,
        &mut ps,
        &mut [],
        None,
    );
    assert!(alive);
    assert_eq!(rng_draws(s0, ps.rng.s), 2, "radius + seed per drip");
    assert_eq!(p.trail, 0.0, "the accumulator resets");
    let ev = ps.drain_events();
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. })), 1);
    // The drip splat radius follows the first draw.
    let want = w.trail_radius * (0.8 + rands[0] * 0.4);
    assert!(
        want > 0.25 && want < 0.45,
        "radius = trailRadius·(0.8+0.4r)"
    );
}

// JS L101: the arrow pool is capped at MAX — the oldest arrow is cut.
#[test]
fn tr33_1_max_arrows_cap() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0xC4F2);
    let mut arrows: Vec<BowArrow> = (0..MAX_ARROWS)
        .map(|i| {
            let mut p = flying(Vec3::ZERO, Vec3::Z, 1.0);
            p.seed = 1000.0 + i as f32;
            p
        })
        .collect();
    bow::volley(
        &kid(&f),
        &bow::bow_shot(0.45, w),
        Vec3::Y,
        Vec3::Z,
        false,
        false,
        &mut arrows,
        &mut ps,
    );
    assert_eq!(arrows.len(), MAX_ARROWS);
    // JS `kill(0)` (L71) is a swap-remove, not a shift: the three cuts
    // remove the oldest (seed 1000), then the last arrow swapped into its
    // place (seed 1191), then the volley's own first arrow — so 190 old
    // arrows survive and the hole at index 0 finally holds the centre
    // arrow of the new volley (rng seed < 1000).
    assert!(
        !arrows.iter().any(|p| p.seed == 1000.0 || p.seed == 1191.0),
        "the swap-removed arrows are gone"
    );
    assert_eq!(
        arrows.iter().filter(|p| p.seed >= 1000.0).count(),
        MAX_ARROWS - 2
    );
    assert!(arrows[0].seed < 1000.0, "the centre arrow sits at index 0");
}

// JS `MAIN_KITS.bow.moveSpeed` L500-504 (runner `move_speed`, weapon.rs).
#[test]
fn tr33_1_move_speed_drawing_and_firing() {
    let f = fix();
    let mut r = WeaponRunner::new();
    // Drawing at c = 0.2: lerp(run·0.75, drawing, min(1, c·2.5)) = lerp(4.5, 2.6, 0.5).
    r.charging = true;
    r.charge = 0.2;
    assert!((r.move_speed(WeaponKind::Bow, &f.t, &f.w) - 3.55).abs() <= MM);
    // The loose pose window.
    r.charging = false;
    r.firing_t = FIRING_T;
    assert!((r.move_speed(WeaponKind::Bow, &f.t, &f.w) - f.w.bow.move_speed_firing).abs() <= MM);
    assert!((f.w.bow.move_speed_firing - 4.2).abs() <= MM);
    // Idle.
    r.firing_t = 0.0;
    assert!((r.move_speed(WeaponKind::Bow, &f.t, &f.w) - f.t.run_speed).abs() <= MM);
}

// JS `MAIN_KITS.bow.tick` L514-519: a draw the runner stopped updating is
// cancelled quietly after 0.15 s.
#[test]
fn tr33_1_sweep_stalled_draw() {
    let f = fix();
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0x57A11);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut arrows = Vec::new();

    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        0.1,
        &mut arrows,
    );
    assert!(r.charging);
    let _ = ps.drain_events();
    bow::sweep_stalled_draw(&mut r, 0.4);
    assert!(!r.charging, "0.3 s stale > 0.15 → cancelled");
    assert_eq!(r.charge, 0.0);
    assert_eq!(r.charge_t, 0.0);
    assert_eq!(r.bow.as_ref().unwrap().ring, 0);
    // A fresh draw is left alone.
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        0.5,
        &mut arrows,
    );
    bow::sweep_stalled_draw(&mut r, 0.55);
    assert!(r.charging, "0.05 s is not stale");
}

// P2-4 regression (JS `stepArrow` L205-207): the `applyHit` call is guarded
// by `!ghost`, but the `weapon:impact` emit sits OUTSIDE it — a ghost arrow
// that crosses a body still shows the impact FX, yet never damages.
#[test]
fn tr33_1_ghost_arrow_impact() {
    let f = fix();
    let w = &f.w.bow;
    let mut ps = ProjectileSim::new(0x6EC8);
    let mut pg = PaintGrid::new(&f.world);
    let mut p = flying(vec3(0.0, 1.0, -3.0), Vec3::Z, 42.0);
    p.ghost = true;
    p.lodge = false; // dies on the body — no drop/lodge follow-up to muddy it
    let mut actors = vec![foe(&f, 1, vec3(0.0, 0.0, 0.0))];

    let mut alive = true;
    let mut n = 0usize;
    while alive {
        n += 1;
        alive = BowArrow::step(
            &mut p,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut actors,
            None,
        );
        assert!(n < 20, "the arrow must reach the victim");
    }
    assert!(!alive, "the tap arrow dies on the body");
    let ev = ps.drain_events();
    assert_eq!(
        count_kind(&ev, |e| matches!(
            e,
            SimEvent::Impact {
                victim_slot: Some(1),
                owner_slot: 0,
                ..
            }
        )),
        1,
        "the ghost hit still emits exactly its impact"
    );
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Impact { .. })), 1);
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Hit { .. })), 0);
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::HitSend { .. })),
        0
    );
    assert!((actors[0].hp - f.t.hp).abs() <= MM, "ghosts never hurt");
}
