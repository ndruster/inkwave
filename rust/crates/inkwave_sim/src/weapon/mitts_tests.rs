//! Task 33 TR-33.2: the mitts kit — punch cadence / hand alternation, the
//! fist flight (range, direct hit, splash falloff + LOS, wall burst), the
//! charged leap (velocity endpoints, push-off, ledge cancel), the wall cling
//! (enter / drain / four exit causes / hang freeze), the land splash (glove
//! smash, damage ring, 25 draws) and the armour snapshot (guard arc, leap
//! armour + grace). Fixed-seed, value-by-value against the ported formulas
//! (spec §7).

use super::mitts::{self, MittsFist, MittsKit};
use super::*;
use crate::actor::{Actor, ActorInput, FIXED_DT, FireGate, Form, KitArmor, NoInk, SimWorld};
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
use crate::paint::PaintGrid;
use crate::tuning::{Mitts, PlayerTuning};
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

/// Flat 20×20 m deck at y = 0; [`fix_wall`] adds a clingable face at x = 3
/// (y ∈ [-1, 3], z ∈ [-2, 2]) and [`fix_slab`] a thin LOS blocker.
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

fn fix_wall() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[deck(), bx([3.0, -1.0, -2.0], [4.0, 3.0, 2.0])], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.weapons,
        sb: tuning.subs,
    }
}

fn fix_slab() -> Fix {
    let tuning = crate::embedded_tuning();
    Fix {
        world: CollisionWorld::from_brushes(&[deck(), bx([0.5, -1.0, 1.9], [0.7, 3.0, 2.1])], B),
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

/// A kid at the deck origin, aiming +Z (yaw 0, pitch 0).
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

/// One mitts runner frame with the real fist pool and the raw jump hold.
#[allow(clippy::too_many_arguments)]
fn frame(
    f: &Fix,
    r: &mut WeaponRunner,
    a: &mut Actor,
    ps: &mut ProjectileSim,
    pg: &mut PaintGrid,
    g: &FireGate,
    jump_held: bool,
    ps_time: f32,
    fists: &mut Vec<MittsFist>,
    foes: &mut FoeScan<'_>,
) -> Vec<SimEvent> {
    r.update(
        DT,
        g,
        a,
        WeaponKind::Mitts,
        &f.w,
        &f.sb,
        Vec3::ZERO,
        &f.world,
        ps,
        foes,
        pg,
        &f.t,
        None,
        g.fire,
        ps_time,
        &mut Vec::new(),
        fists,
        jump_held,
        false,
    );
    ps.drain_events()
}

/// A fist in flight toward +Z at chest height (JS `spawnFist` L128-136).
fn fist(f: &Fix, pos: Vec3, dir: Vec3) -> MittsFist {
    let w = &f.w.mitts;
    MittsFist {
        owner_slot: 0,
        team: 0,
        pos,
        prev: pos,
        vel: dir * w.fist_speed,
        dir,
        age: 0.0,
        life: w.fist_range / w.fist_speed,
        hand: 0,
        seed: 0.5,
        trail: -100.0, // drips off unless re-armed
        ghost: false,
    }
}

/// A fresh kit with the leap-ink floor wired (JS `K()` L48-67).
fn kit(w: &Mitts) -> MittsKit {
    let mut k = MittsKit::new();
    k.ink_min = w.leap_ink_min;
    k
}

/// Count the draws a [`Rng`] made between two states.
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

fn hits_of(ev: &[SimEvent]) -> Vec<(usize, f32)> {
    ev.iter()
        .filter_map(|e| match e {
            SimEvent::Hit {
                victim_slot,
                damage,
                ..
            } => Some((*victim_slot, *damage)),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ TR-33.2

// JS `leapVelocity` L275-287: sqrt(v²) linear endpoints, the angle clamp
// [18°, 60°] around 40° + pitch·0.55, and the vy cap at 13.
#[test]
fn tr33_2_leap_velocity_endpoints() {
    let f = fix();
    let w = &f.w.mitts;
    let a = kid(&f);
    let k = kit(w);
    let (s40, c40) = (40f32.to_radians().sin(), 40f32.to_radians().cos());

    let v0 = mitts::leap_velocity(&a, &k, 0.0, w);
    assert!((v0.y - w.leap_speed_min * s40).abs() <= MM);
    assert!((v0.y - 7.006389).abs() <= MM);
    assert!((v0.z - w.leap_speed_min * c40).abs() <= MM);
    assert!((v0.z - 8.349882).abs() <= MM);
    assert!(v0.x.abs() <= MM);

    let v1 = mitts::leap_velocity(&a, &k, 1.0, w);
    assert!((v1.y - w.leap_speed_max * s40).abs() <= MM, "18.6·sin40");
    assert!((v1.y - 11.955857).abs() <= MM);
    assert!((v1.z - w.leap_speed_max * c40).abs() <= MM);
    assert!((v1.z - 14.248427).abs() <= MM);

    // c = 0.5: v = sqrt(lerp(v_min², v_max², 0.5)) — the linear-in-v² ramp.
    let v =
        ((w.leap_speed_min * w.leap_speed_min + w.leap_speed_max * w.leap_speed_max) * 0.5).sqrt();
    let vh = mitts::leap_velocity(&a, &k, 0.5, w);
    assert!((vh.y - v * s40).abs() <= MM, "15.244·sin40");
    assert!((vh.z - v * c40).abs() <= MM);

    // Pitch up 1 rad → clamped to 60°, and the vy cap pins it at 13.
    let mut up = kid(&f);
    up.aim_pitch = 1.0;
    let vu = mitts::leap_velocity(&up, &k, 1.0, w);
    assert!(
        (vu.y - w.leap_vy_max).abs() <= MM,
        "vy = min(18.6·sin60, 13)"
    );
    assert!((vu.z - w.leap_speed_max * 60f32.to_radians().cos()).abs() <= MM);
    assert!((vu.z - 9.3).abs() <= MM);

    // Pitch down 1.5 rad → clamped to the 18° floor.
    let mut dn = kid(&f);
    dn.aim_pitch = -1.5;
    let vd = mitts::leap_velocity(&dn, &k, 0.0, w);
    assert!((vd.y - w.leap_speed_min * 18f32.to_radians().sin()).abs() <= MM);
    assert!((vd.y - 3.368285).abs() <= MM);
    assert!((vd.z - w.leap_speed_min * 18f32.to_radians().cos()).abs() <= MM);
    assert!((vd.z - 10.366518).abs() <= MM);
}

// JS L282-285: a cling leap pushing off the wall gets its heading nudged so
// the dot with the wall normal is at least 0.35.
#[test]
fn tr33_2_leap_wall_exit_guard() {
    let f = fix();
    let w = &f.w.mitts;
    let mut k = kit(w);
    k.cling = true;
    k.cling_n = Vec3::X;
    let vh = w.leap_speed_min * 40f32.to_radians().cos();

    // yaw = asin(0.3): dn = 0.3 < 0.35 → nudged and renormalised.
    let mut a = kid(&f);
    a.aim_yaw = 0.3f32.asin();
    let v = mitts::leap_velocity(&a, &k, 0.0, w);
    let dn = 0.3f32;
    let dx = dn + 0.35 - dn; // 0.35
    let dz = (1.0 - dn * dn).sqrt();
    let l = dx.hypot(dz);
    assert!((v.x - dx / l * vh).abs() <= MM, "JS L284 renormalise");
    assert!((v.z - dz / l * vh).abs() <= MM);
    assert!((v.y - 7.006389).abs() <= MM);

    // yaw = asin(0.4): dn = 0.4 ≥ 0.35 → untouched.
    a.aim_yaw = 0.4f32.asin();
    let v = mitts::leap_velocity(&a, &k, 0.0, w);
    assert!((v.x - 0.4 * vh).abs() <= MM);
    assert!((v.z - (1.0f32 - 0.16).sqrt() * vh).abs() <= MM);
}

// JS `update` L632-645: 0.12 s per punch → 9 fists in 60 frames, hands
// alternating from the right glove (side flips 1 → 0 first), 3 draws per
// punch (2 spread + 1 seed), ink 1.6 per punch.
#[test]
fn tr33_2_punch_cadence_hands_and_rng() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0x94CC4);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut fists = Vec::new();
    let s0 = ps.rng.s;
    let rands = pre_draw(&Rng { s: s0 }, 27);

    for n in 1..=60u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(true),
            false,
            n as f32 * DT,
            &mut fists,
            &mut FoeScan::empty(),
        );
    }
    assert_eq!(fists.len(), 9, "frames 1,8,15,…,57");
    assert_eq!(rng_draws(s0, ps.rng.s), 27, "3 draws per punch");
    assert!((a.ink - (f.t.ink_max - 9.0 * w.ink_per_punch)).abs() <= MM);
    assert!((a.ink - 85.6).abs() <= MM);
    assert!((r.firing_t - FIRING_T).abs() <= MM);

    let want = [0, 1, 0, 1, 0, 1, 0, 1, 0];
    for (i, fh) in fists.iter().enumerate() {
        assert_eq!(fh.hand, want[i], "JS L641 side flip");
        assert!(
            (fh.seed - rands[i * 3 + 2]).abs() <= 1e-6,
            "seed after the cone"
        );
        assert!((fh.life - w.fist_range / w.fist_speed).abs() <= MM);
        assert!((fh.trail + 0.8).abs() <= MM, "JS L134 trail starts -0.8");
        assert!((fh.vel.length() - w.fist_speed).abs() <= MM);
    }

    // The glove muzzle: hand 0 is the −X side at yaw 0 (JS L118-122),
    // pushed 0.35 along the aim (L617).
    let m0 = mitts::glove_muzzle(&kid(&f), 0, &f.world);
    assert!((m0 - vec3(-0.16, 1.05, 0.35)).length() <= MM);
    let m1 = mitts::glove_muzzle(&kid(&f), 1, &f.world);
    assert!((m1 - vec3(0.16, 1.05, 0.35)).length() <= MM);
}

// JS L637: the per-frame catch-up loop fires at most 2 fists (`guard < 2`).
#[test]
fn tr33_2_punch_backfill_capped_at_two() {
    let f = fix();
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0xBA211);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut fists = Vec::new();

    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        false,
        DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    assert_eq!(fists.len(), 1);
    // A deeply negative shot clock: the loop still stops after 2.
    r.mitts.as_mut().unwrap().cd = -1.0;
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        false,
        2.0 * DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    assert_eq!(fists.len(), 3, "1 + 2, never 3 catch-ups");
}

// JS L641: from the wall only the right glove is free (hand stays 0).
#[test]
fn tr33_2_cling_punches_right_hand_only() {
    let f = fix_wall();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    a.pos = vec3(2.95, 1.5, 0.0);
    a.grounded = false;
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.cling = true;
    k.cling_n = Vec3::NEG_X;
    k.side = 0;
    k.cd = -1.0;
    r.mitts = Some(Box::new(k));
    let mut ps = ProjectileSim::new(0xC116);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();

    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        false,
        DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    assert_eq!(fists.len(), 2);
    assert_eq!(fists[0].hand, 0);
    assert_eq!(fists[1].hand, 0, "cling pins side to 0 (JS L641)");
    let _ = ps.drain_events();
}

// JS L639-640: below `inkPerPunch` the punch is an empty click, no fist.
#[test]
fn tr33_2_low_ink_punch_empty_click() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    a.ink = w.ink_per_punch - 0.6; // 1.0 < 1.6
    let mut ps = ProjectileSim::new(0x1E11);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut fists = Vec::new();

    let ev = frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        false,
        DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    assert!(fists.is_empty());
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::LowInk { owner_slot: 0 })),
        1
    );
    assert!((r.empty_cd - EMPTY_CD).abs() <= MM);
}

// JS `stepFist` L197-246: the fist lives `range / speed` seconds (4.6/28 ≈
// 9.86 frames) then bursts in the air onto the ground under it — no draws.
#[test]
fn tr33_2_fist_life_bursts_in_air() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0x11FE);
    let mut pg = PaintGrid::new(&f.world);
    let mut fh = fist(&f, vec3(0.0, 1.0, 0.0), Vec3::Z);
    let s0 = ps.rng.s;

    let mut alive = true;
    let mut n = 0usize;
    while alive {
        n += 1;
        alive = MittsFist::step(
            &mut fh,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut FoeScan::empty(),
            None,
        );
        assert!(n < 30, "the fist must die at its range");
    }
    assert_eq!(n, 10, "life 0.1643 s → frame 10");
    assert_eq!(rng_draws(s0, ps.rng.s), 0, "the air burst draws nothing");
    let ev = ps.drain_events();
    let impact = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Impact {
                radius,
                victim_slot,
                ..
            } => Some((*radius, *victim_slot)),
            _ => None,
        })
        .expect("burst impact");
    assert!(
        (impact.0 - w.splash_radius).abs() <= MM,
        "radius = splashRadius"
    );
    assert_eq!(impact.1, None);
    assert_eq!(count_kind(&ev, |e| matches!(e, SimEvent::Turf { .. })), 1);
}

// JS L205-217: the first enemy capsule takes the direct hit (`punchDamage`)
// and the burst; the hit point rides the Impact `victim_slot`.
#[test]
fn tr33_2_fist_direct_hit() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0xD18E);
    let mut pg = PaintGrid::new(&f.world);
    let mut fh = fist(&f, vec3(0.0, 1.0, 0.0), Vec3::Z);
    let mut actors = vec![foe(&f, 4, vec3(0.0, 0.0, 2.2))];
    let s0 = ps.rng.s;

    let mut alive = true;
    let mut ev = Vec::new();
    while alive {
        alive = MittsFist::step(
            &mut fh,
            DT,
            &f.world,
            &mut pg,
            w,
            &f.t,
            &mut ps,
            &mut FoeScan::all(&mut actors),
            None,
        );
        ev = ps.drain_events();
        if !alive {
            break;
        }
    }
    assert!(!alive, "the fist dies on the body");
    assert_eq!(hits_of(&ev), vec![(4, w.punch_damage)]);
    assert!((w.punch_damage - 38.0).abs() <= MM);
    assert_eq!(
        count_kind(&ev, |e| matches!(
            e,
            SimEvent::Impact {
                victim_slot: Some(4),
                ..
            }
        )),
        1
    );
    assert_eq!(rng_draws(s0, ps.rng.s), 0);
    assert!((actors[0].hp - (f.t.hp - 38.0)).abs() <= MM);
}

// JS `burstFist` L164-170: the splash is `lerp(22, 10, d/1.4)` to everyone
// near *but the one it hit*, gated by line of sight.
#[test]
fn tr33_2_fist_splash_falloff_and_direct_excluded() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0x5FA54);
    let mut pg = PaintGrid::new(&f.world);
    let fh = fist(&f, Vec3::ZERO, Vec3::Z);
    let mut actors = vec![
        foe(&f, 1, vec3(0.0, 0.0, 2.0)), // the one it hit — excluded
        foe(&f, 2, vec3(0.7, 0.0, 2.0)), // d = sqrt(0.58) → 15.472
        foe(&f, 3, vec3(1.5, 0.0, 2.0)), // d = 1.5297 > 1.4 → none
    ];
    // c = (0, 1, 2); v2 = (x, 0.7, 2) for a kid on the deck.
    let d = (0.7f32 * 0.7 + 0.3 * 0.3).sqrt();
    let want = lerp(w.splash_max, w.splash_min, d / w.splash_radius);
    burst_at(
        &f,
        &fh,
        vec3(0.0, 1.0, 2.0),
        Some(0),
        &mut ps,
        &mut pg,
        &mut actors,
    );
    let ev = ps.drain_events();
    let hits = hits_of(&ev);
    assert_eq!(hits.len(), 1, "the direct victim takes no splash");
    assert_eq!(hits[0].0, 2);
    assert!((hits[0].1 - want).abs() <= MM, "d=0.7616 → 15.472");
    assert!((want - 15.472195).abs() <= MM);
    assert!(
        (actors[0].hp - f.t.hp).abs() <= MM,
        "direct foe untouched here"
    );
    assert!(
        (actors[2].hp - f.t.hp).abs() <= MM,
        "past the splash radius"
    );
}

#[test]
fn tr33_2_fist_splash_needs_los() {
    let f = fix_slab();
    let mut ps = ProjectileSim::new(0x105);
    let mut pg = PaintGrid::new(&f.world);
    let fh = fist(&f, Vec3::ZERO, Vec3::Z);
    let mut actors = vec![
        foe(&f, 1, vec3(0.0, 0.0, 2.0)),
        foe(&f, 2, vec3(1.2, 0.0, 2.0)), // d = 1.2369 < 1.4, but the slab blocks
    ];
    burst_at(
        &f,
        &fh,
        vec3(0.0, 1.0, 2.0),
        Some(0),
        &mut ps,
        &mut pg,
        &mut actors,
    );
    let ev = ps.drain_events();
    assert!(hits_of(&ev).is_empty(), "LOS gates the splash");
}

// JS L158: a fist bursting on a surface (the `normal` branch) splats with
// 1 rng draw (radius = fistPaint·(0.85 + 0.3·r0)).
#[test]
fn tr33_2_fist_wall_burst_one_draw() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0xAF11);
    let mut pg = PaintGrid::new(&f.world);
    let fh = fist(&f, Vec3::ZERO, Vec3::Z);
    let s0 = ps.rng.s;
    let r0 = pre_draw(&Rng { s: s0 }, 1)[0];
    let mut actors = vec![];
    mitts::burst_fist(
        &fh,
        vec3(4.0, 1.0, 4.0),
        None,
        Some(Vec3::NEG_Z),
        &f.world,
        &mut pg,
        w,
        &f.t,
        &mut ps,
        &mut FoeScan::all(&mut actors),
        None,
    );
    assert_eq!(rng_draws(s0, ps.rng.s), 1, "one jitter draw");
    let want = w.fist_paint * (0.85 + 0.3 * r0);
    assert!(want > 0.8 && want < 1.15, "radius = fistPaint·(0.85+0.3r)");
    assert!(
        (w.fist_paint * 0.85 - 0.8).abs() <= 0.05,
        "fistPaint 0.85 keeps the band tight"
    );
}

/// Drive [`mitts::burst_fist`] over the full-list scan (the match's step).
#[allow(clippy::too_many_arguments)]
fn burst_at(
    f: &Fix,
    fh: &MittsFist,
    at: Vec3,
    direct: Option<usize>,
    ps: &mut ProjectileSim,
    pg: &mut PaintGrid,
    actors: &mut [Actor],
) {
    mitts::burst_fist(
        fh,
        at,
        direct,
        None,
        &f.world,
        pg,
        &f.w.mitts,
        &f.t,
        ps,
        &mut FoeScan::all(actors),
        None,
    );
}

// JS `MAIN_KITS.mitts.jump` L650-660: the press is swallowed while
// charging / leaping / clinging (with a fire intent it starts the crouch
// from the wall), and on the ground a fire+jump press starts the charge.
#[test]
fn tr33_2_jump_hook_swallows_and_starts() {
    let f = fix();
    let w = &f.w.mitts;

    let mut a = kid(&f);
    let mut k = kit(w);
    k.charging = true;
    assert!(k.jump(&mut a, false), "charging swallows the press");
    k.charging = false;
    k.leaping = true;
    assert!(k.jump(&mut a, false), "leaping swallows the press");

    // Clinging: no fire → hop off the wall (JS `dropCling('jump')` L484).
    let mut k2 = kit(w);
    k2.cling = true;
    k2.cling_n = Vec3::NEG_X;
    let mut a2 = kid(&f);
    assert!(k2.jump(&mut a2, false));
    assert!(!k2.cling);
    assert!((a2.vel - vec3(-3.4, 4.6, 0.0)).length() <= MM);

    // Clinging + fire intent → the wall crouch starts instead.
    let mut k3 = kit(w);
    k3.cling = true;
    let mut a3 = kid(&f);
    assert!(k3.jump(&mut a3, true));
    assert!(k3.charging, "fire+jump off the wall charges");
    assert!(k3.cling);

    // Ground: no fire intent → a plain jump (not swallowed).
    let mut k4 = kit(w);
    let mut a4 = kid(&f);
    assert!(!k4.jump(&mut a4, false));
    assert!(!k4.charging);

    // Ground + fire + enough ink → the crouch; too little ink → not.
    let mut k5 = kit(w);
    let mut a5 = kid(&f);
    assert!(k5.jump(&mut a5, true));
    assert!(k5.charging);
    let mut k6 = kit(w);
    let mut a6 = kid(&f);
    a6.ink = w.leap_ink_min - 0.1;
    assert!(!k6.jump(&mut a6, true), "under leapInkMin the jump is free");
}

// actor.js L311-313 via `Actor::step`: a fire+jump press becomes the crouch,
// never a jump; without the kit the same press jumps.
#[test]
fn tr33_2_step_jump_swallowed_by_charge() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    assert!(a.grounded, "spawned on the deck");
    let mut k = kit(w);
    let input = ActorInput {
        jump: true,
        fire: true,
        ..Default::default()
    };
    a.step(
        DT,
        &input,
        &f.sim(),
        &NoInk,
        &f.t,
        &f.sb,
        None,
        Some(&mut k),
    );
    assert!(k.charging, "the press became a crouch");
    assert!(a.vel.y.abs() <= MM, "no jump velocity");
    assert!(!a.grounded || a.vel.y <= MM);

    // Without the kit's ink floor the same press jumps normally.
    let mut b = kid(&f);
    let mut k2 = kit(w);
    k2.ink_min = 1000.0;
    b.step(
        DT,
        &input,
        &f.sim(),
        &NoInk,
        &f.t,
        &f.sb,
        None,
        Some(&mut k2),
    );
    assert!(!k2.charging);
    // The jump sets vel.y = jumpVel, then `_integrate` spends one gravity
    // step in the same frame (actor.js jump → integrate).
    assert!(
        (b.vel.y - (f.t.jump_vel - f.t.gravity * DT)).abs() <= MM,
        "jumpVel 8.4 minus one gravity step"
    );
}

// The leap charge → release (JS `update` L556-579 / `releaseLeap` L368-390):
// charge_t linear to full at 0.45 s, the release pays lerp(9,15,c) ink,
// launches with leapVelocity, pushes 1 rng draw and records `[2,…]`.
#[test]
fn tr33_2_leap_charge_release() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    assert!(k.jump(&mut a, true), "fire+jump starts the crouch");
    r.mitts = Some(Box::new(k));
    let mut ps = ProjectileSim::new(0x1E4F);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();
    let s0 = ps.rng.s;

    for n in 1..=27u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(false),
            true,
            n as f32 * DT,
            &mut fists,
            &mut FoeScan::empty(),
        );
        let k = r.mitts.as_ref().unwrap();
        assert!(k.charging);
        assert!((k.charge - (n as f32 * DT / w.leap_charge_time).min(1.0)).abs() <= MM);
        assert!((r.charge - k.charge).abs() <= MM, "runner mirror");
    }
    assert!(r.mitts.as_ref().unwrap().full);
    let _ = ps.drain_events();

    // Let go of the jump key → the pounce.
    let ev = frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        28.0 * DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let k = r.mitts.as_ref().unwrap();
    assert!(!k.charging && k.leaping);
    assert!(
        (a.ink - (f.t.ink_max - w.leap_ink_max)).abs() <= MM,
        "ink 85"
    );
    assert!((a.vel - vec3(0.0, 11.955857, 14.248427)).length() <= MM);
    assert!((k.leap_h - 14.248427).abs() <= MM);
    assert_eq!(rng_draws(s0, ps.rng.s), 1, "the push-off splat seed");
    assert_eq!(r.charge, 0.0);
    let kit_ev = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Kit { kind, data, .. } if kind == "mitts" => Some(data.clone()),
            _ => None,
        })
        .expect("leap record");
    assert_eq!(kit_ev[0], 2.0);
    assert!((kit_ev[4] - 1.0).abs() <= MM, "r2 charge");
    assert_eq!(&kit_ev[5..8], &[0.0, 0.0, 0.0], "not from a wall");
}

// JS L572: walked off a ledge mid-charge — the crouch dies after 0.12 s air.
#[test]
fn tr33_2_ledge_cancel_charge() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.charging = true;
    r.mitts = Some(Box::new(k));
    a.grounded = false;
    let mut ps = ProjectileSim::new(0x1E62);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();

    for n in 1..=7u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(false),
            true,
            n as f32 * DT,
            &mut fists,
            &mut FoeScan::empty(),
        );
        assert!(r.mitts.as_ref().unwrap().charging, "air {n} < 0.12");
    }
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        true,
        8.0 * DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let k = r.mitts.as_ref().unwrap();
    assert!(!k.charging, "0.133 s off the ground cancels");
    assert_eq!(k.charge, 0.0);
    assert_eq!(r.charge, 0.0);
}

// JS L549-556: the body was owned by something else for > 0.25 s — the kit
// resets itself (charge cancelled, leap dropped).
#[test]
fn tr33_2_stale_seen_resets() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.charging = true;
    k.leaping = true;
    k.seen = 0.1;
    r.mitts = Some(Box::new(k));
    let mut ps = ProjectileSim::new(0x57A1E);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();

    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        true,
        0.4,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let k = r.mitts.as_ref().unwrap();
    assert!(!k.charging, "0.3 s stale > 0.25 → reset");
    assert!(!k.leaping);
    assert_eq!(k.charge, 0.0);
}

// JS `startCling` L463-477 via the leap wall contact (L590-594): into > 1.5
// and a clingable wall → hang on, vel zeroed, 25 splash draws, and the cling
// drain starts the same frame.
#[test]
fn tr33_2_cling_enter_and_drain() {
    let f = fix_wall();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    a.pos = vec3(2.95, 1.5, 0.0);
    a.grounded = false;
    a.set_invuln(0.0); // start_cling's splash arms exactly landInvuln
    a.contacts.wall = true;
    a.contacts.wall_normal = Vec3::NEG_X;
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.leaping = true;
    k.leap_v = vec3(8.0, 0.0, 0.0);
    k.leap_t = 0.1;
    r.mitts = Some(Box::new(k));
    let mut ps = ProjectileSim::new(0xC119);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();
    let s0 = ps.rng.s;

    let ev = frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.1,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let k = r.mitts.as_ref().unwrap();
    assert!(k.cling && k.hang, "stuck to the wall");
    assert!(!k.leaping);
    assert!((k.cling_n - Vec3::NEG_X).length() <= MM);
    // JS L469-470: clingPt = pos − n·radius, y = pos.y + 1.
    assert!((k.cling_pt - vec3(3.33, 2.5, 0.0)).length() <= MM);
    assert!(a.vel.length() <= MM, "hang zeroes the velocity");
    assert_eq!(rng_draws(s0, ps.rng.s), 25, "the wall hit's land splash");
    assert!(
        (a.invuln() - w.land_invuln).abs() <= MM,
        "0.5 s untouchable"
    );
    // The cling block ran the same frame: one frame of drain.
    assert!((a.ink - (f.t.ink_max - w.cling_drain * DT)).abs() <= MM);
    let kit_ev = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Kit { kind, data, .. } if kind == "mitts" => Some(data.clone()),
            _ => None,
        })
        .expect("splash record");
    assert_eq!(kit_ev[0], 1.0);
    assert!((kit_ev[1] - 3.33).abs() <= MM);
    assert!((kit_ev[4] + 1.0).abs() <= MM, "n.x");
    assert_eq!(
        count_kind(
            &ev,
            |e| matches!(e, SimEvent::Impact { radius, .. } if (*radius - w.land_radius).abs() <= MM)
        ),
        1
    );
}

// JS L591: `into ≤ 1.5` (a glancing hit) → slid, physics takes over.
#[test]
fn tr33_2_cling_enter_rejects_glancing_and_floor() {
    let f = fix_wall();
    let w = &f.w.mitts;
    // Too shallow: into = 1.4999.
    let mut a = kid(&f);
    a.pos = vec3(2.95, 1.5, 0.0);
    a.grounded = false;
    a.contacts.wall = true;
    a.contacts.wall_normal = Vec3::NEG_X;
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.leaping = true;
    k.leap_v = vec3(1.4999, 0.0, 0.0);
    k.leap_t = 0.1;
    r.mitts = Some(Box::new(k));
    let mut ps = ProjectileSim::new(0xC1A2);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.1,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let k = r.mitts.as_ref().unwrap();
    assert!(!k.cling);
    assert!(k.slid, "JS L594: physics takes it from here");

    // A floor under the feet → not clingable (JS `clingableAt` L351-352).
    let mut b = kid(&f);
    b.pos = vec3(2.95, 0.05, 0.0);
    b.grounded = false;
    b.contacts.wall = true;
    b.contacts.wall_normal = Vec3::NEG_X;
    let mut r2 = WeaponRunner::new();
    let mut k2 = kit(w);
    k2.leaping = true;
    k2.leap_v = vec3(8.0, 0.0, 0.0);
    k2.leap_t = 0.1;
    r2.mitts = Some(Box::new(k2));
    frame(
        &f,
        &mut r2,
        &mut b,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.2,
        &mut fists,
        &mut FoeScan::empty(),
    );
    assert!(!r2.mitts.as_ref().unwrap().cling, "floor blocks the cling");
}

/// One clinging kid at the wall with a live kit on the runner.
fn clinging(f: &Fix, ink: f32) -> (Actor, WeaponRunner) {
    let w = &f.w.mitts;
    let mut a = kid(f);
    a.pos = vec3(2.95, 1.5, 0.0);
    a.grounded = false;
    a.ink = ink;
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.cling = true;
    k.hang = true;
    k.cling_n = Vec3::NEG_X;
    k.cling_pt = vec3(3.33, 2.5, 0.0);
    r.mitts = Some(Box::new(k));
    (a, r)
}

// JS `dropCling` L479-490: the four exit causes — jump hops (3.4, 4.6),
// empty / lost push a little off (1.2, 0.5), squid just lets go.
#[test]
fn tr33_2_cling_exit_causes() {
    let f = fix_wall();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0xC113);
    let mut pg = PaintGrid::new(&f.world);
    let mut fists = Vec::new();

    // 'jump' — the hook hops off the wall.
    let (mut a, mut r) = clinging(&f, 100.0);
    let k = r.mitts.as_mut().unwrap();
    assert!(k.jump(&mut a, false));
    assert!((a.vel - vec3(-3.4, 4.6, 0.0)).length() <= MM);

    // 'empty' — the 8/s drain runs the tank out.
    let (mut a, mut r) = clinging(&f, 4.0);
    let mut dropped = false;
    for n in 1..=35u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(false),
            false,
            n as f32 * DT,
            &mut fists,
            &mut FoeScan::empty(),
        );
        let _ = ps.drain_events();
        if !r.mitts.as_ref().unwrap().cling {
            dropped = true;
            break;
        }
    }
    assert!(dropped, "4 ink drains at 8/s → 0 within 31 frames");
    assert!(a.ink.abs() <= MM);
    assert!(
        (a.vel - vec3(-1.2, 0.5, 0.0)).length() <= MM,
        "'empty' push"
    );

    // 'squid' — no velocity kick.
    let (mut a, mut r) = clinging(&f, 100.0);
    a.form = Form::Squid;
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        40.0 * DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    assert!(!r.mitts.as_ref().unwrap().cling);
    assert!(a.vel.length() <= MM, "'squid' lets go softly");

    // 'lost' — the wall is gone.
    let (mut a, mut r) = clinging(&f, 100.0);
    a.pos.x = 5.0;
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        41.0 * DT,
        &mut fists,
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    assert!(!r.mitts.as_ref().unwrap().cling);
    assert!((a.vel - vec3(-1.2, 0.5, 0.0)).length() <= MM, "'lost' push");
    assert!((w.cling_drain - 8.0).abs() <= MM);
}

// actor.js L502-507: a clinging kid hangs frozen — velocity zeroed, no
// gravity, no ground following.
#[test]
fn tr33_2_hang_freezes_velocity() {
    let f = fix_wall();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    a.pos = vec3(2.95, 1.5, 0.0);
    a.grounded = true; // would otherwise follow the deck
    a.vel = vec3(1.0, 2.0, 3.0);
    let mut k = kit(w);
    k.cling = true;
    k.hang = true;
    let p0 = a.pos;
    a.step(
        DT,
        &ActorInput::default(),
        &f.sim(),
        &NoInk,
        &f.t,
        &f.sb,
        None,
        Some(&mut k),
    );
    assert!(a.vel.length() <= MM, "hang zeroes the velocity");
    assert!(!a.grounded, "hang drops the ground flag");
    assert!((a.pos - p0).length() <= MM, "no integration while hanging");
}

// JS `landSplash` L426-461 driven by the leap landing (L584-588): glove
// smash 40 + the damage ring lerp(100, 35) with LOS, 25 draws, 0.5 s
// invulnerability, the `[1,…]` record and the 2.8 m Impact.
#[test]
fn tr33_2_land_splash_damage_and_rng() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0x1A4D);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.leaping = true;
    r.mitts = Some(Box::new(k));
    let s0 = ps.rng.s;

    // c = at + n·0.35 = (0, 0.35, 0); v2 = (dx, 0.7, 0) → d = sqrt(dx²+0.1225).
    let dx = |d: f32| (d * d - 0.35f32 * 0.35).sqrt();
    let mut actors = vec![
        kid(&f),
        foe(&f, 1, vec3(dx(0.6), 0.0, 0.0)),  // glove + 100
        foe(&f, 2, vec3(dx(1.7), 0.0, 0.0)),  // 67.5, no glove
        foe(&f, 3, vec3(dx(2.8), 0.0, 0.0)),  // 35
        foe(&f, 4, vec3(dx(2.81), 0.0, 0.0)), // past the ring
    ];
    actors[0].pos = vec3(0.0, 0.0, 0.0);
    actors[0].grounded = true;
    actors[0].set_invuln(0.0); // the landing arms exactly landInvuln
    let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
    let ev = frame(
        &f,
        &mut r,
        me,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.1,
        &mut Vec::new(),
        &mut foes,
    );
    assert_eq!(rng_draws(s0, ps.rng.s), 25, "1 centre + 6 blobs × 4 draws");
    assert!((me.invuln() - w.land_invuln).abs() <= MM);
    let k = r.mitts.as_ref().unwrap();
    assert!(!k.leaping && k.land_t <= MM, "landed, grace clock armed");

    let hits = hits_of(&ev);
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0], (1, w.glove_damage), "glove smash first");
    assert_eq!(hits[1].0, 1);
    assert!((hits[1].1 - w.land_damage_max).abs() <= MM);
    assert_eq!(hits[2].0, 2);
    assert!((hits[2].1 - 67.5).abs() <= MM, "lerp(100,35,0.5) at d=1.7");
    assert_eq!(hits[3].0, 3);
    assert!((hits[3].1 - w.land_damage_min).abs() <= MM, "35 at the rim");
    assert_eq!(
        count_kind(&ev, |e| matches!(
            e,
            SimEvent::Hit {
                killed: true,
                victim_slot: 1,
                ..
            }
        )),
        1,
        "40 + 100 splats the glove victim"
    );
    assert!((actors[2].hp - (f.t.hp - 67.5)).abs() <= MM);
    assert!((actors[3].hp - (f.t.hp - 35.0)).abs() <= MM);
    assert!((actors[4].hp - f.t.hp).abs() <= MM, "d=2.81 untouched");

    let kit_ev = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Kit { kind, data, .. } if kind == "mitts" => Some(data.clone()),
            _ => None,
        })
        .expect("land record");
    assert_eq!(kit_ev, vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    assert_eq!(
        count_kind(&ev, |e| matches!(e, SimEvent::Impact { radius, pos, .. }
                if (*radius - w.land_radius).abs() <= MM
                    && (pos - vec3(0.0, 0.35, 0.0)).length() <= MM)),
        1
    );
}

#[test]
fn tr33_2_land_splash_needs_los() {
    // A slab straddling the +X line at chest height: the foe sits at ring
    // distance d = 1.7 (past the 1.48 glove reach) but out of sight.
    let tuning = crate::embedded_tuning();
    let f = Fix {
        world: CollisionWorld::from_brushes(&[deck(), bx([0.5, -1.0, -0.2], [0.7, 3.0, 0.2])], B),
        pads: [vec3(0.0, 0.0, -20.0), vec3(0.0, 0.0, 20.0)],
        t: tuning.player,
        w: tuning.weapons,
        sb: tuning.subs,
    };
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0x1A4E);
    let mut pg = PaintGrid::new(&f.world);
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.leaping = true;
    r.mitts = Some(Box::new(k));

    // c = at + n·0.35 = (0, 0.35, 0); v2 = (x, 0.7, 0) → d = sqrt(x²+0.1225).
    let dx = (1.7f32 * 1.7 - 0.35 * 0.35).sqrt(); // 1.6636 > glove reach 1.48
    let mut actors = vec![kid(&f), foe(&f, 1, vec3(dx, 0.0, 0.0))];
    actors[0].pos = vec3(0.0, 0.0, 0.0);
    actors[0].grounded = true;
    let (me, mut foes) = FoeScan::split_around(&mut actors, 0);
    let ev = frame(
        &f,
        &mut r,
        me,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.1,
        &mut Vec::new(),
        &mut foes,
    );
    assert!(hits_of(&ev).is_empty(), "LOS gates the damage ring");
    assert!(
        (1.7f32 * 1.7 - 0.35 * 0.35).sqrt() > w.glove_radius + f.t.radius,
        "the foe is out of glove reach, so only the ring could hurt it"
    );
}

// JS `damageTaken` L888-901 via the [`KitArmor`] snapshot: the guard arc is
// ±70° about the aim heading, halving damage inside it.
#[test]
fn tr33_2_guard_arc_halves_frontal_damage() {
    let f = fix();
    let w = &f.w.mitts;
    let mut a = kid(&f);
    a.set_invuln(0.0);
    let mut r = WeaponRunner::new();
    let mut ps = ProjectileSim::new(0x64A4D);
    let mut pg = PaintGrid::new(&f.world);

    // One punch arms the pose window; the next frame (trigger off) still
    // guards: firingT 0.333 > 0.35 − guardAfter(0.25).
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(true),
        false,
        DT,
        &mut Vec::new(),
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        2.0 * DT,
        &mut Vec::new(),
        &mut FoeScan::empty(),
    );
    let snap = a.kit_armor.expect("snapshot written");
    assert!(snap.guard_up);
    assert!(
        (snap.guard_arc_cos - w.guard_arc.to_radians().cos()).abs() <= MM,
        "cos of the 70° arc"
    );

    let dmg = |v: &mut Actor, ap: Option<Vec3>| {
        v.hp = f.t.hp;
        v.damage(10.0, Some((1, 4)), ap, &f.t);
        f.t.hp - v.hp
    };
    assert!(
        (dmg(&mut a, Some(vec3(0.0, 0.7, 5.0))) - 5.0).abs() <= MM,
        "straight ahead ×0.5"
    );
    let a69 = 69f32.to_radians();
    assert!(
        (dmg(&mut a, Some(vec3(a69.sin() * 5.0, 0.7, a69.cos() * 5.0))) - 5.0).abs() <= MM,
        "69° inside the arc"
    );
    let a71 = 71f32.to_radians();
    assert!(
        (dmg(&mut a, Some(vec3(a71.sin() * 5.0, 0.7, a71.cos() * 5.0))) - 10.0).abs() <= MM,
        "71° outside"
    );
    assert!(
        (dmg(&mut a, Some(vec3(0.0, 0.7, -5.0))) - 10.0).abs() <= MM,
        "behind"
    );
    assert!(
        (dmg(&mut a, None) - 10.0).abs() <= MM,
        "no attacker position → full"
    );

    // The window closes: guardAfter 0.25 s after the last punch pose.
    for n in 3..=20u32 {
        frame(
            &f,
            &mut r,
            &mut a,
            &mut ps,
            &mut pg,
            &gate(false),
            false,
            n as f32 * DT,
            &mut Vec::new(),
            &mut FoeScan::empty(),
        );
        let _ = ps.drain_events();
    }
    assert!(!a.kit_armor.unwrap().guard_up, "firingT decayed below 0.1");
    assert!((dmg(&mut a, Some(vec3(0.0, 0.7, 5.0))) - 10.0).abs() <= MM);
}

// JS L884-886: airborne from a leap, or within the 0.15 s grace after
// landing, incoming damage is scaled by `leapArmor` 0.35 — never stacked
// with the guard arc.
#[test]
fn tr33_2_leap_armor_and_grace() {
    let f = fix();
    let w = &f.w.mitts;
    let mut ps = ProjectileSim::new(0xA4A4);
    let mut pg = PaintGrid::new(&f.world);

    // Airborne from a leap: ×leapArmor, and it wins over the guard arc.
    let mut a = kid(&f);
    a.set_invuln(0.0);
    a.grounded = false;
    let mut r = WeaponRunner::new();
    let mut k = kit(w);
    k.leaping = true;
    r.mitts = Some(Box::new(k));
    r.firing_t = FIRING_T; // the guard would also be up — never both
    frame(
        &f,
        &mut r,
        &mut a,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.1,
        &mut Vec::new(),
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    let snap = a.kit_armor.expect("snapshot written");
    assert!(snap.leaping && snap.guard_up, "both windows open");
    a.damage(100.0, Some((1, 4)), Some(vec3(0.0, 0.7, 5.0)), &f.t);
    assert!(
        (a.hp - (f.t.hp - 100.0 * w.leap_armor)).abs() <= MM,
        "×0.35, not ×0.5"
    );

    // The landing grace window: land_t < leapArmorGrace still ×0.35.
    let mut a2 = kid(&f);
    a2.set_invuln(0.0);
    let mut r2 = WeaponRunner::new();
    let mut k2 = kit(w);
    k2.land_t = 0.05;
    r2.mitts = Some(Box::new(k2));
    frame(
        &f,
        &mut r2,
        &mut a2,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.2,
        &mut Vec::new(),
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    let snap2 = a2.kit_armor.unwrap();
    assert!(!snap2.leaping && snap2.land_t < w.leap_armor_grace);
    a2.damage(100.0, Some((1, 4)), Some(vec3(0.0, 0.7, 5.0)), &f.t);
    assert!(
        (a2.hp - (f.t.hp - 100.0 * w.leap_armor)).abs() <= MM,
        "grace ×0.35"
    );

    // Past both windows: the guard arc halves instead (no armour stacking).
    let mut a3 = kid(&f);
    a3.set_invuln(0.0);
    let mut r3 = WeaponRunner::new();
    r3.mitts = Some(Box::new(kit(w))); // fresh kit: land_t 9.0
    r3.firing_t = FIRING_T;
    frame(
        &f,
        &mut r3,
        &mut a3,
        &mut ps,
        &mut pg,
        &gate(false),
        false,
        0.3,
        &mut Vec::new(),
        &mut FoeScan::empty(),
    );
    let _ = ps.drain_events();
    let snap3 = a3.kit_armor.unwrap();
    assert!(!snap3.leaping && snap3.land_t >= w.leap_armor_grace && snap3.guard_up);
    a3.damage(100.0, Some((1, 4)), Some(vec3(0.0, 0.7, 5.0)), &f.t);
    assert!(
        (a3.hp - (f.t.hp - 100.0 * w.guard_armor)).abs() <= MM,
        "guard ×0.5"
    );
}

// JS `MAIN_KITS.mitts.moveSpeed` (mitts.js L905-912, weapon.rs L927-940):
// the pounce holds its launch speed above the air floor, the crouch crawls,
// the post-land hop doubles it, the punch window slows, else run speed.
#[test]
fn tr33_2_move_speed_arms() {
    let f = fix();
    let w = &f.w.mitts;
    let mut r = WeaponRunner::new();

    // Leaping: max(leapH, airMinSpeed).
    let mut k = kit(w);
    k.leaping = true;
    k.leap_h = 20.0;
    r.mitts = Some(Box::new(k));
    assert!((r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - 20.0).abs() <= MM);
    r.mitts.as_mut().unwrap().leap_h = 1.0;
    assert!(
        (r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - f.t.air_min_speed).abs() <= MM,
        "air floor 4.6"
    );

    // Crouch crawl.
    let mut k = kit(w);
    k.charging = true;
    r.mitts = Some(Box::new(k));
    assert!(
        (r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - w.move_speed_charging).abs() <= MM,
        "1.1"
    );

    // Post-land hop: land_t < 0.25 → charging × 2.
    let mut k = kit(w);
    k.land_t = 0.1;
    r.mitts = Some(Box::new(k));
    assert!(
        (r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - w.move_speed_charging * 2.0).abs() <= MM,
        "2.2"
    );

    // Punch window.
    let mut k = kit(w);
    k.land_t = 9.0;
    r.mitts = Some(Box::new(k));
    r.firing_t = FIRING_T;
    assert!(
        (r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - w.move_speed_firing).abs() <= MM,
        "5.4"
    );

    // Otherwise the plain run speed.
    r.firing_t = 0.0;
    assert!((r.move_speed(WeaponKind::Mitts, &f.t, &f.w) - f.t.run_speed).abs() <= MM);
}

// P0-1 regression (JS `stepFist` L212 / `burstFist` L169 `applyHit(f.owner,
// e, …)`): the guard arc is judged from the *owner's live actor position*
// looked up in the roster, not from the hit/burst point. A fist landing on
// the victim's front while the owner stands behind it must NOT trigger the
// guard; the same fist with the owner in front must.
#[test]
fn tr33_2_guard_arc_uses_owner_position() {
    let f = fix();
    let w = &f.w.mitts;
    let guard = KitArmor {
        leaping: false,
        land_t: 9.0,
        guard_up: true,
        leap_armor: 0.35,
        leap_grace: 0.15,
        guard_armor: 0.65,
        guard_arc_cos: 70f32.to_radians().cos(),
    };

    let mut owner = kid(&f); // team 0, slot 0 — the fist's owner
    owner.pos = vec3(-4.0, 0.05, 0.0); // behind the victim's back (−X)
    let mut victim = foe(&f, 1, vec3(0.0, 0.05, 0.0));
    victim.aim_yaw = std::f32::consts::FRAC_PI_2; // facing +X
    victim.kit_armor = Some(guard);
    let mut actors = vec![owner, victim];

    let mut ps = ProjectileSim::new(0x94D1);
    let mut pg = PaintGrid::new(&f.world);

    // Fly a fist down the +X axis onto the victim's front; run to the hit.
    let fly = |actors: &mut [Actor], ps: &mut ProjectileSim, pg: &mut PaintGrid| {
        let mut fh = fist(&f, vec3(2.0, 1.0, 0.0), Vec3::NEG_X);
        fh.prev = vec3(2.5, 1.0, 0.0);
        let mut alive = true;
        let mut n = 0usize;
        while alive {
            n += 1;
            alive = MittsFist::step(
                &mut fh,
                DT,
                &f.world,
                pg,
                w,
                &f.t,
                ps,
                &mut FoeScan::all(actors),
                None,
            );
            assert!(n < 30, "the fist must resolve within its range");
        }
        assert!(!alive, "the fist dies on the body");
        let _ = ps.drain_events();
    };

    // Owner behind: dot = −1 → outside the arc → full `punchDamage`, even
    // though the hit point sits on the victim's front (+X) side.
    fly(&mut actors, &mut ps, &mut pg);
    assert!(
        (f.t.hp - actors[1].hp - w.punch_damage).abs() <= MM,
        "guard arc keyed on the owner position: full damage"
    );

    // Owner in front of the aim heading (+X, inside the ±70° arc): the same
    // hit is scaled by `guardArmor`. The snapshot survives the non-lethal hit.
    actors[0].pos = vec3(4.0, 0.05, 0.0);
    actors[1].hp = f.t.hp;
    fly(&mut actors, &mut ps, &mut pg);
    assert!(
        (f.t.hp - actors[1].hp - w.punch_damage * guard.guard_armor).abs() <= MM,
        "owner inside the arc → ×0.65"
    );
    assert!(actors[1].kit_armor.is_some(), "snapshot still in place");
}
