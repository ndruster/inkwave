//! TR-34.1 / TR-34.2 / TR-34.3: per-kind behaviour tests for the Whirl
//! Boomerang kit.
//!
//! The JS sources are authoritative: `src/config.js` (the `boomerang` row)
//! and `src/game/kits/boomerang.js`. Expected values are anchored to those
//! lines.

use super::SubKind;
use super::boomerang::BoomerangState;
use super::tests::{Scene, hits};
use crate::actor::{FIXED_DT, FireGate};
use crate::weapon::{FoeScan, SimEvent, WeaponKind, WeaponRunner};
use glam::Vec3;

const DT: f32 = FIXED_DT;

fn kit_events(evs: &[SimEvent], op: f32) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubSpawn { sub_kind, data, .. }
                if sub_kind == "boomerang" && data[0] == op =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

fn end_events(evs: &[SimEvent]) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubEnd {
                sub_kind: Some(k),
                data,
                ..
            } if k == "boomerang" => Some(data.clone()),
            _ => None,
        })
        .collect()
}

/// JS `blast`'s falloff (boomerang.js L452-453): `lerp(dmgMin, dmgMax, k²)`
/// over the chest distance, recomputed from a record's centre.
fn blast_damage(c: [f32; 3], foe: [f32; 3], radius: f32, dmax: f32, dmin: f32) -> f32 {
    let d = ((c[0] - foe[0]).powi(2) + (c[1] - foe[1]).powi(2) + (c[2] - foe[2]).powi(2)).sqrt();
    let k = 1.0 - ((d - 0.8) / (radius - 0.8)).clamp(0.0, 1.0);
    dmin + (dmax - dmin) * (k * k)
}

fn runner_frame(
    r: &mut WeaponRunner,
    s: &mut Scene,
    w: &crate::tuning::WeaponSet,
    gate: &FireGate,
    time: f32,
    sub_blocked: bool,
) {
    r.update(
        DT,
        gate,
        &mut s.actors[0],
        WeaponKind::Shooter,
        w,
        &s.sb,
        Vec3::ZERO,
        &s.world,
        &mut s.ps,
        &mut FoeScan::empty(),
        &mut s.paint,
        &s.t,
        None,
        false,
        time,
        &mut Vec::new(),
        &mut Vec::new(),
        false,
        sub_blocked,
    );
}

// TR-34.1 — the tuning table values the boomerang port is anchored to.
#[test]
fn tr34_1_boomerang_tuning_values() {
    // JS config.js L397-405 (`boomerang` row).
    let s = Scene::new();
    assert_eq!(super::ink_cost(SubKind::Boomerang, &s.sb), 60.0);
    let t = &s.sb.boomerang;
    assert_eq!(t.throw_speed, 16.0);
    assert_eq!(t.range, 9.0);
    assert_eq!(t.out_time, 0.6);
    assert_eq!(t.hover, 1.75);
    assert_eq!(t.hover_radius, 1.35);
    assert_eq!(t.tick_rate, 8);
    assert_eq!(t.tick_damage, 9.0);
    assert_eq!(t.hover_paint, 1.0);
    assert_eq!(t.return_speed, 17.0);
    assert_eq!(t.orbit, 2.5);
    assert_eq!(t.orbit_radius, 1.5);
    assert_eq!(t.orbit_spin, 7.0);
    assert_eq!(t.orbit_damage, 6.0);
    assert_eq!(t.orbit_hit_cd, 0.3);
    assert_eq!(t.radius, 2.4);
    assert_eq!(t.damage_max, 100.0);
    assert_eq!(t.damage_min, 30.0);
    assert_eq!(t.paint_radius, 2.2);
    assert_eq!(t.contact, 0.3);
    assert_eq!(t.hit_fuse, 0.6);
    assert_eq!(t.hit_radius, 3.1);
    assert_eq!(t.hit_damage_max, 180.0);
    assert_eq!(t.hit_damage_min, 35.0);
    assert_eq!(t.hit_paint_radius, 2.7);
}

// TR-34.1 — the throw plan: chest-high flat release, the full-range plan
// with no wall, the eased outbound duration and the `[0, gid, start×3,
// dir×3]` record (JS `plan` L145-152, `planFrom` L154-165, `use` L175-182,
// `spawn` L185-200).
#[test]
fn tr34_1_boomerang_throw_plan_and_record() {
    let mut s = Scene::new();
    let evs = s.deploy(SubKind::Boomerang);
    let it = &s.subs.boomerang[0];
    let gid = it.gid;
    let rec = &kit_events(&evs, 0.0)[0];
    assert_eq!(rec.len(), 8); // JS L178
    assert_eq!(rec[1], gid as f32);
    // JS L149: start = (x + sin(yaw)·0.3, y + 1.35, z + cos(yaw)·0.3).
    assert!((rec[2] - 0.0).abs() < 1e-6);
    assert!((rec[3] - 1.4).abs() < 1e-6); // 0.05 + 1.35
    assert!((rec[4] - 0.3).abs() < 1e-6);
    // JS L147-148: pitch clamped to [-0.5, 0.25]; a level aim throws flat.
    assert!((rec[5] - 0.0).abs() < 1e-6);
    assert!((rec[6] - 0.0).abs() < 1e-6);
    assert!((rec[7] - 1.0).abs() < 1e-6);
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SimEvent::SubUse {
                kind: SubKind::Boomerang,
                ..
            }
        )),
        "JS L180: emit('sub:use')"
    );
    // JS L158-160: nothing blocks the 9.4 m ray → the full `range`.
    assert!(!it.plan.wall);
    assert_eq!(it.plan.len, 9.0);
    assert!((it.plan.end - glam::vec3(0.0, 1.4, 9.3)).length() < 1e-5);
    // JS L193: `outT = max(0.18, outTime * sqrt(len / range))`.
    assert!((it.out_t - 0.6).abs() < 1e-6);
    assert_eq!(it.state, BoomerangState::Out);
    assert!(!it.ghost);
    assert!(!it.blasted);
    assert_eq!(it.owner_slot, 0);
    assert_eq!(it.team, 0);
}

// TR-34.1 — the empty-flight lifecycle: out → hover → back → orbit → the
// small end burst, with the `[1, gid, pos×3, 0]` record and no `[2]` end
// record (blasted suppresses it; JS `tick` L239-241, `out` L249-266,
// `hover` L268-281, `startBack`/`back` L312-342, `orbit` L344-369,
// `burst` L428).
#[test]
fn tr34_1_boomerang_lifecycle_phases_and_end_burst() {
    let mut s = Scene::new();
    let mut all = s.deploy(SubKind::Boomerang);
    // JS L193/L252: the 0.6 s eased outbound flight.
    for _ in 0..34 {
        all.extend(s.step());
    }
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Out);
    for _ in 0..8 {
        all.extend(s.step());
    }
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Hover);
    // JS L280: hover runs `hover` = 1.75 s, then `startBack` (frame 141).
    for _ in 0..96 {
        all.extend(s.step());
    }
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Hover);
    for _ in 0..8 {
        all.extend(s.step());
    }
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Back);
    // JS L340: it enters orbit within `orbitRadius + 0.35` of the chest.
    let mut reached_orbit = false;
    for _ in 0..200 {
        all.extend(s.step());
        if s.subs.boomerang[0].state == BoomerangState::Orbit {
            reached_orbit = true;
            break;
        }
    }
    assert!(reached_orbit, "the boomerang must swing home");
    // JS L368: `orbit` 2.5 s of circling, then the small burst.
    let mut gone = false;
    for _ in 0..250 {
        all.extend(s.step());
        if s.subs.boomerang.is_empty() {
            gone = true;
            break;
        }
    }
    assert!(gone, "the end burst removes it");
    let big = kit_events(&all, 1.0);
    assert_eq!(big.len(), 1);
    assert_eq!(big[0].len(), 6);
    assert_eq!(big[0][5], 0.0); // JS L428: the end burst is `big = false`
    assert!(
        end_events(&all).is_empty(),
        "JS L239: `blasted` suppresses the [2, gid] record"
    );
    assert!(hits(&all).is_empty());
    assert!(
        all.iter().any(|e| matches!(e, SimEvent::Turf { .. })),
        "the streaks and drops credit turf"
    );
    assert!(s.paint.coverage()[0] > 0.0);
}

// TR-34.1 — the hover shred: 9 damage per 1/8 s tick inside `hoverRadius`
// with LOS, the ink drops under it, and a foe off the flight line so the
// boomerang never arms (JS `shred` L283-309, `hover` L268-281).
#[test]
fn tr34_1_boomerang_hover_shred_kills() {
    let mut s = Scene::new();
    s.add_enemy_at(-1.1, 9.3); // beside the hover point, clear of the path
    let mut all = s.deploy(SubKind::Boomerang);
    let mut saw_back = false;
    let mut ever_armed = false;
    for _ in 0..180 {
        all.extend(s.step());
        let st = s.subs.boomerang[0].state;
        if st == BoomerangState::Armed {
            ever_armed = true;
        }
        if st == BoomerangState::Back {
            saw_back = true;
            break;
        }
    }
    assert!(saw_back, "hover runs out and it whirls back");
    assert!(!ever_armed, "a foe off the swept path never arms it");
    let h = hits(&all);
    assert_eq!(h.len(), 12, "12 shred ticks of 9 kill the 100 hp foe");
    assert!(h.iter().all(|(slot, d)| *slot == 1 && *d == 9.0));
    assert!(!s.actors[1].alive);
    assert!(s.paint.coverage()[0] > 0.0, "JS L298-307: hover ink drops");
}

// TR-34.1 — contact: a foe in the swept path stops it dead, takes the
// 9-damage latch hit, the `[3, gid, pos×3]` arm record, then the full
// Splat-Bomb blast on the 0.6 s fuse with the `[1, …, big]` record and no
// `[2]` end record (JS `contact` L371-384, `arm`/`armed` L386-404,
// `blast` L429-457).
#[test]
fn tr34_1_boomerang_contact_arms_and_big_blast() {
    let mut s = Scene::new();
    s.add_enemy(4.0); // on the flight line
    s.add_enemy(5.6); // just off it — only the blast reaches it
    let mut all = s.deploy(SubKind::Boomerang);
    let mut armed = false;
    for _ in 0..60 {
        all.extend(s.step());
        if s.subs.boomerang[0].state == BoomerangState::Armed {
            armed = true;
            break;
        }
    }
    assert!(armed, "the foe on the path arms it");
    let arm_rec = &kit_events(&all, 3.0)[0];
    assert_eq!(arm_rec.len(), 5); // JS L380
    // JS L418-426: an armed boomerang rides out its own path (no fizzle).
    super::boomerang::owner_splatted(&mut s.subs.boomerang, 0);
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Armed);
    // `armed` never moves it, so the last live position is the blast centre
    // (the records themselves are `r3`-quantised, boomerang.js L9).
    let mut centre = Vec3::ZERO;
    for _ in 0..60 {
        if let Some(it) = s.subs.boomerang.first() {
            centre = it.pos;
        }
        all.extend(s.step());
        if s.subs.boomerang.is_empty() {
            break;
        }
    }
    assert!(s.subs.boomerang.is_empty());
    let blast_rec = &kit_events(&all, 1.0)[0];
    assert_eq!(blast_rec[5], 1.0); // JS L399: the fuse blast is `big`
    // The fuse does not move it: the blast centre is the arm position.
    assert_eq!(&blast_rec[2..5], &arm_rec[2..5]);
    let c = [centre.x, centre.y, centre.z];
    for (got, want) in blast_rec[2..5].iter().zip(c.iter()) {
        assert!(
            (got - want).abs() <= 0.001,
            "the record is `r3` of the centre"
        );
    }
    assert!(end_events(&all).is_empty());

    let h = hits(&all);
    let foe1: Vec<f32> = h.iter().filter(|(s, _)| *s == 1).map(|(_, d)| *d).collect();
    assert_eq!(foe1[0], 9.0, "JS L379: the latch hit is `tickDamage`");
    assert!(
        foe1[..foe1.len() - 1].iter().all(|d| *d == 9.0),
        "armed shred"
    );
    assert!(foe1.len() >= 5, "the 0.6 s fuse lands several shred ticks");
    let last1 = *foe1.last().unwrap();
    assert!(last1 > 170.0 && last1 <= 180.0);
    let want1 = blast_damage(c, [0.0, 0.8, 4.0], 3.1, 180.0, 35.0);
    assert!((last1 - want1).abs() < 1e-4, "JS L452-453 falloff");
    let foe2: Vec<(usize, f32)> = h.iter().cloned().filter(|(s, _)| *s == 2).collect();
    assert_eq!(foe2.len(), 1, "the blast reaches it exactly once");
    let want2 = blast_damage(c, [0.0, 0.8, 5.6], 3.1, 180.0, 35.0);
    assert!((foe2[0].1 - want2).abs() < 1e-4);
    assert!(s.actors[2].alive, "the rim hit leaves it standing");
    assert!(!s.actors[1].alive, "the latch victim dies to the blast");
    assert!(s.paint.coverage()[0] > 0.0);
}

// TR-34.1 — the orbit graze: 6 damage per foe per 0.3 s cooldown while it
// circles the thrower, then the small end burst (JS `orbit` L344-369).
#[test]
fn tr34_1_boomerang_orbit_graze() {
    let mut s = Scene::new();
    s.add_enemy_at(-1.5, 0.0); // on the orbit circle, behind the thrower
    let mut all = s.deploy(SubKind::Boomerang);
    let mut gone = false;
    for _ in 0..420 {
        all.extend(s.step());
        if s.subs.boomerang.is_empty() {
            gone = true;
            break;
        }
    }
    assert!(gone);
    let h = hits(&all);
    let foe1: Vec<f32> = h.iter().filter(|(s, _)| *s == 1).map(|(_, d)| *d).collect();
    assert_eq!(foe1[0], 6.0, "JS L366: the graze is `orbitDamage`");
    assert!(
        foe1.iter().filter(|d| **d == 6.0).count() >= 2,
        "the circle sweeps the foe at least twice in 2.5 s"
    );
    for d in &foe1 {
        assert!(*d == 6.0 || (*d >= 30.0 && *d <= 100.0), "graze or burst");
    }
    assert!(kit_events(&all, 1.0).len() == 1);
    assert!(
        s.paint.coverage()[0] > 0.0,
        "JS L358-364: the orbit ink ring"
    );
}

// TR-34.3 — one out at a time: `blocked()` refuses the second throw and
// the runner leaves the ink untouched (JS boomerang.js L603, weapons.js
// L145).
#[test]
fn tr34_3_boomerang_one_out() {
    let mut s = Scene::new();
    assert!(!s.subs.kit_blocked(SubKind::Boomerang, 0));
    s.deploy(SubKind::Boomerang);
    assert!(s.subs.kit_blocked(SubKind::Boomerang, 0));
    // The match layer feeds `blocked()` into the runner; with the gate the
    // release neither arms the throw nor pays the ink (JS weapons.js
    // L145-147).
    let mut r = WeaponRunner::new();
    let w = crate::embedded_tuning().weapons;
    let ink0 = s.actors[0].ink;
    let hold = FireGate {
        sub: true,
        ..Default::default()
    };
    runner_frame(&mut r, &mut s, &w, &hold, DT, true);
    let rel = FireGate {
        sub_released: true,
        ..Default::default()
    };
    runner_frame(&mut r, &mut s, &w, &rel, 2.0 * DT, true);
    assert!(!r.pending_sub, "the second throw is refused");
    assert_eq!(s.actors[0].ink, ink0, "the refused throw costs no ink");
    // A fresh press + an unblocked release arms the throw and pays the ink.
    runner_frame(&mut r, &mut s, &w, &hold, 3.0 * DT, false);
    runner_frame(&mut r, &mut s, &w, &rel, 4.0 * DT, false);
    assert!(r.pending_sub, "an unblocked release arms the throw");
    assert_eq!(s.actors[0].ink, ink0 - 60.0, "the throw pays `inkCost`");
}

// TR-34.1 — a splatted thrower fizzles it: it drops out of the air and
// pops a little ink, no damage, and the `[2, gid]` end record fires
// because it never blasted (JS `'splatted'` L418-426, `fizzle` L406-417,
// `tick` L239-241).
#[test]
fn tr34_1_boomerang_owner_splatted_fizzles() {
    let mut s = Scene::new();
    s.add_enemy_at(-1.1, 9.3); // close enough to shred if it were live
    let mut all = s.deploy(SubKind::Boomerang);
    let gid = s.subs.boomerang[0].gid;
    for _ in 0..40 {
        all.extend(s.step()); // into the hover
    }
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Hover);
    // JS L418-426: the `'splatted'` listener fizzles a live throw.
    super::boomerang::owner_splatted(&mut s.subs.boomerang, 0);
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Fizzle);
    let mut gone = false;
    let mut later = Vec::new();
    for _ in 0..60 {
        later.extend(s.step());
        if s.subs.boomerang.is_empty() {
            gone = true;
            break;
        }
    }
    all.extend(later.iter().cloned());
    assert!(gone, "the drop-and-pop finishes well inside 0.7 s");
    let ends = end_events(&all);
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0], vec![2.0, gid as f32]); // JS L239: not blasted
    assert!(kit_events(&all, 1.0).is_empty(), "no burst record");
    assert!(
        hits(&later).is_empty(),
        "a fizzling boomerang hurts nobody (only the pre-splat hover shred lands)"
    );
    assert!(s.actors[1].alive);
    assert!(s.paint.coverage()[0] > 0.0, "JS L412: the pop is ink");
}

// TR-34.2 — a ghost replays the flight but decides nothing: no records,
// no damage, no ink, no turf; duplicate `(src_nid, gid)` throws drop; the
// arm and big-blast ops only drive the replay state (JS `ghost` L201-215,
// `netGhost` mute wrap, netmatch.js L490-491).
#[test]
fn tr34_2_boomerang_ghost_replays_but_decides_nothing() {
    let mut s = Scene::new();
    s.add_enemy_at(-8.0, -8.0); // actor 1 is the remote owner (team 1)
    let nids = [1u8, 2]; // actor 0 → nid 1, actor 1 → nid 2
    let ghost_rec = |s: &mut Scene, data: &[f32]| {
        s.subs.net_ghost(
            1,
            2,
            "boomerang",
            data,
            &nids,
            &s.world,
            &mut s.actors,
            &mut s.paint,
            &mut s.ps,
            &s.t,
            &s.sb,
            None,
            s.time,
        );
        s.subs.drain_events()
    };
    // op 0: a level +z throw from the hand.
    let evs = ghost_rec(&mut s, &[0.0, 7.0, 0.0, 1.4, 0.3, 0.0, 0.0, 1.0]);
    assert!(kit_events(&evs, 0.0).is_empty(), "ghosts echo no records");
    assert!(s.kit_ghosted(SubKind::Boomerang));
    let it = &s.subs.boomerang[0];
    assert_eq!(it.gid, 7);
    assert_eq!(it.src_nid, Some(2));
    assert_eq!(it.team, 1);
    // Duplicate (src_nid, gid) is dropped (JS L203, D-T34-5).
    ghost_rec(&mut s, &[0.0, 7.0, 0.0, 1.4, 0.3, 0.0, 0.0, 1.0]);
    assert_eq!(s.kit_count(SubKind::Boomerang), 1);

    let mut all = Vec::new();
    for _ in 0..20 {
        all.extend(s.step());
    }
    let it = &s.subs.boomerang[0];
    assert_eq!(it.state, BoomerangState::Out);
    assert!(it.pos.z > 5.0, "the ghost flies its own eased path");
    assert!(hits(&all).is_empty());
    assert!(!all.iter().any(|e| matches!(e, SimEvent::Turf { .. })));
    assert!(
        kit_events(&all, 3.0).is_empty(),
        "ghosts skip `contact` (L371)"
    );
    assert_eq!(s.paint.coverage()[0], 0.0);
    assert_eq!(s.paint.coverage()[1], 0.0);

    // op 3: the owner's screen armed it — the replay follows, hurtlessly.
    let evs = ghost_rec(&mut s, &[3.0, 7.0, 0.0, 1.4, 5.0]);
    assert!(kit_events(&evs, 3.0).is_empty());
    assert_eq!(s.subs.boomerang[0].state, BoomerangState::Armed);
    // op 1 with `big`: the full blast replays muted — no echo, no damage.
    let evs = ghost_rec(&mut s, &[1.0, 7.0, 0.0, 1.4, 5.0, 1.0]);
    assert!(kit_events(&evs, 1.0).is_empty());
    assert!(hits(&evs).is_empty());
    assert!(s.subs.boomerang[0].blasted);
    let evs = s.step();
    assert!(s.subs.boomerang.is_empty(), "the blast dead-splices it");
    assert!(
        end_events(&evs).is_empty(),
        "ghosts splice without [2, gid]"
    );
    assert!(hits(&evs).is_empty());
    assert_eq!(s.paint.coverage()[0], 0.0);
    assert_eq!(s.paint.coverage()[1], 0.0);
}
