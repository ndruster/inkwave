//! Task 30 TR-30.1: the brolly kit — pellet cone and falloff, the canopy
//! lifecycle (tap → open → launch → break → regrow) and the launched wall.

use super::brolly::{
    BrollyKit, CanopyState, HELD_R, LaunchedCanopy, canopy_depth, held_shield, hit_kit,
};
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

/// Flat 20×20 m deck at y = 0 — the same stage as [`super::tests`].
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

/// A kid at the deck origin, aiming +Z.
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

/// One runner-driven frame (the real dispatch path, so the common
/// `cooldown -= dt` decay runs too).
fn frame(
    f: &Fix,
    r: &mut WeaponRunner,
    a: &mut Actor,
    ps: &mut ProjectileSim,
    fire: bool,
) -> Vec<SimEvent> {
    r.update(
        DT,
        &gate(fire),
        a,
        WeaponKind::Brolly,
        &f.w,
        Vec3::ZERO,
        &f.world,
        ps,
        &mut FoeScan::empty(),
        &mut PaintGrid::new(&f.world),
        &f.t,
        None,
        false,
        0.0,
    );
    ps.drain_events()
}

// TR-30.1 — tap cadence, ink cost and the 11-pellet cone geometry.
#[test]
fn tr30_1_brolly_cadence_ink_and_pellet_cone() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0xB00);
    let mut r = WeaponRunner::new();
    let mut blasts = 0usize;
    let mut cone_checked = false;
    for cycle in 0..5 {
        let ev = frame(&f, &mut r, &mut a, &mut ps, true);
        blasts += ev
            .iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
        if cycle == 0 {
            // One blast = 11 pellets, kit-scored (core damage 0), no world
            // blast flag, no trail drips; ONE fire event for the whole blast.
            assert_eq!(ps.list.len(), f.w.brolly.pellets as usize);
            let dir0 = a.aim_dir();
            for p in ps.list.iter() {
                let pm = p.pellet.expect("pellet mark");
                assert_eq!(pm.blast, 1, "first blast id");
                assert!(!p.blast, "pellets are not blaster blasts");
                assert_eq!(p.damage, 0.0, "the kit scores pellets, not the core");
                assert_eq!(p.trail_every, 0.0, "no trail drips (JS L100)");
                assert_eq!(p.size, f.w.brolly.pellet_size);
                assert_eq!(p.life, f.w.brolly.pellet_life);
                let v = p.vel.normalize();
                assert!(v.dot(dir0) > 0.98, "inside the 8° cone");
                let speed = p.vel.length();
                assert!(
                    (speed - f.w.brolly.proj_speed).abs() <= f.w.brolly.proj_speed * 0.015 + MM,
                    "one blast speed ±1.5%"
                );
            }
            cone_checked = true;
        }
        for _ in 0..28 {
            let _ = frame(&f, &mut r, &mut a, &mut ps, false);
            ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w);
        }
    }
    assert!(cone_checked);
    assert_eq!(
        blasts, 5,
        "one tap = one blast, cooldown 0.45 fits the cycle"
    );
    assert!(
        (a.ink - (f.t.ink_max - 5.0 * f.w.brolly.ink_per_shot)).abs() < MM,
        "inkPerShot 5.5 × 5"
    );
    assert_eq!(ps.blast_seq, 5, "blast ids increment");
}

// TR-30.1 — `pelletDamage`: flat 8 to 2.8 m, lerp to 2.5 at 6.5 m.
#[test]
fn tr30_1_pellet_damage_falloff() {
    let f = fix();
    let bw = &f.w.brolly;
    let m = PelletMark {
        blast: 0,
        falloff_start: bw.falloff_start,
        falloff_end: bw.falloff_end,
        dmg_near: bw.pellet_damage,
        dmg_far: bw.pellet_damage_far,
    };
    assert_eq!(m.pellet_damage(0.0), 8.0);
    assert_eq!(m.pellet_damage(2.8), 8.0);
    assert!(
        (m.pellet_damage(4.65) - 5.25).abs() < MM,
        "mid-falloff lerp"
    );
    assert_eq!(m.pellet_damage(6.5), 2.5);
    assert_eq!(m.pellet_damage(9.0), 2.5, "clamped past falloffEnd");
}

// TR-30.1 — hold the trigger: tap blast once, unfold, then launch the canopy
// as a sliding wall (`SimEvent::Kit` + the runner's launch handoff).
#[test]
fn tr30_1_brolly_open_then_launch() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(0x1A0C);
    let mut r = WeaponRunner::new();
    let mut fire_events = 0usize;
    let mut kit_events = Vec::new();
    let mut launched_at = None;
    for frame_no in 0..100 {
        let ev = frame(&f, &mut r, &mut a, &mut ps, true);
        fire_events += ev
            .iter()
            .filter(|e| matches!(e, SimEvent::Fire { .. }))
            .count();
        kit_events.extend(ev.into_iter().filter_map(|e| match e {
            SimEvent::Kit { kind, data, .. } => Some((kind, data)),
            _ => None,
        }));
        if r.brolly_launch.is_some() && launched_at.is_none() {
            launched_at = Some(frame_no);
        }
        ps.step(DT, &f.world, &mut [], &mut pg, &f.t, &f.w);
        // openDelay 0.12 + openTime 0.14 + launchHold 0.9 ≈ 1.15 s: nothing
        // has launched yet at 1.0 s.
        if frame_no == 59 {
            assert!(r.brolly_launch.is_none(), "no launch before ~1.15 s");
            let k = r.brolly.as_ref().unwrap();
            assert_eq!(k.open, 1.0, "canopy fully open");
            assert!(
                k.launch_k > 0.0 && k.launch_k < 1.0,
                "launch charge visible"
            );
        }
        if launched_at.is_some() {
            break;
        }
    }
    let at = launched_at.expect("the held canopy launches");
    assert!((60..80).contains(&at), "launch at ~1.15 s (frame {at})");
    assert_eq!(
        fire_events, 1,
        "the tap blast fires once, the shot guard holds"
    );
    assert_eq!(kit_events.len(), 1);
    let (kind, data) = &kit_events[0];
    assert_eq!(kind, "brolly");
    assert_eq!(data.len(), 7, "[x, y, z, dx, dz, hp, h0]");
    assert_eq!(data[5], 450.0, "full canopy hp");
    let c = r.brolly_launch.take().unwrap();
    assert_eq!(c.team, 0);
    assert_eq!(c.slot, 0);
    assert!((c.pos.y - 0.0).abs() < MM, "settled onto the deck");
    assert!((c.dir.z - 1.0).abs() < MM, "slides along the aim (+Z)");
    assert_eq!(c.hp, 450.0);
    let k = r.brolly.as_ref().unwrap();
    assert_eq!(k.state, CanopyState::Launched);
    assert_eq!(k.open, 0.0);
    assert!(k.lock_release, "no re-open until the trigger is released");
    assert!(
        (k.regrow_t - f.w.brolly.regrow_time).abs() < 0.05,
        "regrow armed (the same frame's decay included)"
    );
}

// TR-30.1 — the held canopy blocks a round and drains kit hp; a broken
// canopy regrows after `regrowTime`.
#[test]
fn tr30_1_held_canopy_blocks_breaks_regrows() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut a = kid(&f);
    let mut ps = ProjectileSim::new(42);
    let mut r = WeaponRunner::new();
    // Open the canopy for real (hold past openDelay), stop before launch
    // (`closeTime` 0.1 would fold it again if released).
    for _ in 0..13 {
        let _ = frame(&f, &mut r, &mut a, &mut ps, true);
    }
    {
        let k = r.brolly.as_mut().unwrap();
        assert!(k.open >= 0.55 && k.grow >= 0.9, "shield pose live");
        assert_eq!(k.state, CanopyState::Ready);
    }
    let (c, n) = held_shield(r.brolly.as_ref().unwrap(), &a).expect("live held shield");
    assert!((n - Vec3::Z).length() < MM, "normal along the aim");
    assert!(
        (c - vec3(0.0, a.visual_y() + 0.9, 0.62)).length() < MM,
        "HELD_UP / HELD_FWD pose"
    );
    let shields = vec![Shield {
        team: 0,
        c,
        n,
        r: HELD_R,
        cp: c,
        np: n,
        regrow_time: f.w.brolly.regrow_time,
        target: ShieldTarget::Held(0),
    }];
    // An enemy round crossing the swept disc this frame (the tap blast's own
    // pellets are team-0 and pass their own shield — cleared for focus).
    ps.list.clear();
    ps.list.push(Projectile {
        pos: c + n * 0.8,
        prev: c + n * 1.4,
        start: c + n * 1.4,
        vel: -n * 60.0,
        team: 1,
        owner_slot: 4,
        age: 0.0,
        life: 1.2,
        straight: 0.1,
        radius: 0.78,
        size: 0.15,
        damage: 36.0,
        grav: 28.0,
        drag: 0.8,
        trail: -2.5,
        trail_every: 1.1,
        trail_radius: 0.4,
        seed: 0.5,
        blast: false,
        pellet: None,
        ghost: false,
        below_falloff: 0.0,
        below_falloff_max: 0.0,
        volley: None,
        dmg_far: None,
        delay: 0.0,
    });
    let mut launched: Vec<LaunchedCanopy> = Vec::new();
    let mut owner = ShieldOwner {
        runners: std::slice::from_mut(&mut r),
        launched: &mut launched,
    };
    ps.step_masked(
        DT,
        &f.world,
        &mut [],
        &mut pg,
        &f.t,
        &f.w,
        None,
        &shields,
        &mut owner,
    );
    assert!(ps.list.is_empty(), "the round is eaten by the canopy");
    assert!(
        (r.brolly.as_ref().unwrap().hp - (450.0 - 36.0)).abs() < MM,
        "hitShield drains the shot damage"
    );
    // Break it outright, then idle out the regrow.
    {
        let k = r.brolly.as_mut().unwrap();
        hit_kit(k, 500.0, ps.time, f.w.brolly.regrow_time);
        assert_eq!(k.state, CanopyState::Broken);
        assert_eq!(k.hp, 0.0);
        assert!((k.regrow_t - f.w.brolly.regrow_time).abs() < MM);
    }
    assert!(
        held_shield(r.brolly.as_ref().unwrap(), &a).is_none(),
        "broken = no shield"
    );
    for _ in 0..(f.w.brolly.regrow_time as usize * 60 + 5) {
        let _ = frame(&f, &mut r, &mut a, &mut ps, false);
    }
    let k = r.brolly.as_ref().unwrap();
    assert_eq!(k.state, CanopyState::Ready, "regrown after regrowTime");
    assert_eq!(k.hp, k.hp_max);
    assert!(
        k.grow > 0.0 && k.grow < 0.5,
        "regrows small and re-inflates from 0.001"
    );
}

// TR-30.1 — the launched wall: slides, lays the 0.32 m ink stripe and shoves
// enemies out of its plane (allies pass through).
#[test]
fn tr30_1_launched_canopy_slides_paints_blocks() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let mut rng = Rng::new(0x51DE);
    let mut c = LaunchedCanopy::new(
        0,
        0,
        vec3(0.0, 0.0, 0.0),
        Vec3::Z,
        450.0,
        450.0,
        f.w.brolly.launch_life,
        1.0,
        false,
    );
    let mut area = 0.0;
    for _ in 0..120 {
        let (a, keep) = c.update(DT, &f.world, &mut pg, &f.t, &f.w.brolly, &mut rng);
        area += a;
        assert!(keep, "alive for the whole 2 s");
    }
    assert!(c.pos.z > 3.0, "slid at launchSpeed 2.6");
    assert!(area > 0.0, "the ink stripe claimed turf");
    assert!(pg.coverage()[0] > 0.0, "team ink under the wall");
    assert!(
        (c.cur_n - Vec3::Z).length() < MM,
        "blocking normal = travel dir"
    );

    // Blocking: an enemy in the plane is pushed out front and flags shoving;
    // an ally at the same spot is ignored.
    let mut actors = vec![
        {
            let mut e = Actor::new(1, 4, &f.t);
            e.spawn_at(
                vec3(0.0, 0.05, c.pos.z + 0.1),
                std::f32::consts::PI,
                &f.sim(),
                &f.t,
            );
            e
        },
        {
            let mut e = Actor::new(0, 1, &f.t);
            e.spawn_at(
                vec3(0.5, 0.05, c.pos.z + 0.1),
                std::f32::consts::PI,
                &f.sim(),
                &f.t,
            );
            e
        },
    ];
    let z0 = actors[0].pos.z;
    c.block_actors(&mut actors, f.t.radius);
    assert!(
        actors[0].pos.z > z0,
        "enemy pushed out in front of the wall"
    );
    assert!(c.shoving, "shoving an enemy slows the next slide");
    assert_eq!(actors[1].pos.z, z0 - 0.0, "allies walk through");
}

// TR-30.1 — the pellet merge: a point-blank blast lands as exactly ONE hit
// (11 × 8 = 88) after the 1/30 s window.
#[test]
fn tr30_1_pellet_blast_merges_into_one_hit() {
    let f = fix();
    let mut pg = PaintGrid::new(&f.world);
    let a = kid(&f);
    let mut victim = Actor::new(1, 4, &f.t);
    victim.spawn_at(vec3(0.0, 0.05, 1.0), std::f32::consts::PI, &f.sim(), &f.t);
    victim.set_invuln(0.0);
    let mut actors = vec![a, victim];
    let mut ps = ProjectileSim::new(0x9E5E);
    let mut r = WeaponRunner::new();
    let _ = frame(&f, &mut r, &mut actors[0], &mut ps, true);
    let _ = frame(&f, &mut r, &mut actors[0], &mut ps, false);
    let mut hits = Vec::new();
    for _ in 0..10 {
        ps.step(DT, &f.world, &mut actors, &mut pg, &f.t, &f.w);
        hits.extend(ps.drain_events().into_iter().filter_map(|e| match e {
            SimEvent::Hit {
                victim_slot,
                damage,
                killed,
                ..
            } => Some((victim_slot, damage, killed)),
            _ => None,
        }));
    }
    let total: f32 = hits.iter().map(|(_, d, _)| d).sum();
    assert_eq!(hits.len(), 1, "one merged hit per blast per victim");
    assert_eq!(hits[0].0, 4);
    assert!(
        (total - 88.0).abs() < 11.0 * MM,
        "11 pellets × pelletDamage(<2.8 m) = 88, got {total}"
    );
    assert!(!hits[0].2, "88 < 100 hp: the victim survives the merge");
    assert!(
        (actors[1].hp - (f.t.hp - 88.0)).abs() < 11.0 * MM,
        "the merged damage applied once"
    );
}

// BrollyKit::new sanity (the JS `kitOf` initialiser).
#[test]
fn tr30_1_brolly_kit_init() {
    let k = BrollyKit::new(450.0);
    assert_eq!(k.state, CanopyState::Ready);
    assert_eq!(k.hp, 450.0);
    assert_eq!(k.grow, 1.0);
    assert_eq!(k.open, 0.0);
    assert_eq!(k.last_hit, -9.0);
    assert!(
        canopy_depth() > 0.3 && canopy_depth() < 0.35,
        "dome depth ≈ 0.3414"
    );
}
