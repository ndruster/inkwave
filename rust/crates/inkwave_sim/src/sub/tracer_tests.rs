//! TR-34.1 / TR-34.2: per-kind behaviour tests for the Tracer Bolt kit.
//!
//! The JS sources are authoritative: `src/config.js` (the `tracer` row)
//! and `src/game/kits/tracer.js`. Expected values are anchored to those
//! lines.

use super::SubKind;
use super::tests::{Scene, hits};
use super::tracer::TracerState;
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
use crate::weapon::SimEvent;
use glam::vec3;

fn kit_events(evs: &[SimEvent], op: f32) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == "tracer" && data[0] == op => {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

/// Step until the first bolt leaves the `fly` state (or `max` frames).
fn step_until_end(s: &mut Scene, max: usize) -> Vec<SimEvent> {
    let mut evs = Vec::new();
    for _ in 0..max {
        evs.extend(s.step());
        if s.subs.tracer.iter().any(|b| !b.state.live()) {
            break;
        }
    }
    evs
}

// TR-34.1 — the tuning table values the tracer port is anchored to.
#[test]
fn tr34_1_tracer_tuning_values() {
    // JS config.js L392-396 (`tracer` row).
    let s = Scene::new();
    assert_eq!(super::ink_cost(SubKind::Tracer, &s.sb), 40.0);
    let t = &s.sb.tracer;
    assert_eq!(t.speed, 40.0);
    assert_eq!(t.range, 32.0);
    assert_eq!(t.pitch_down, 9.0);
    assert_eq!(t.floor_exit, 0.3);
    assert_eq!(t.max_bounces, 12);
    assert_eq!(t.size, 0.16);
    assert_eq!(t.puddle_radius, 1.35);
    assert_eq!(t.direct_damage, 35.0);
    assert_eq!(t.direct_mark, 9.0);
    assert_eq!(t.trail_life, 1.0);
    assert_eq!(t.trail_radius, 0.42);
    assert_eq!(t.trail_damage, 22.0);
    assert_eq!(t.trail_mark, 3.0);
}

// TR-34.1 — the fire geometry: chest-low release, the aim line lowered by
// `pitchDown`, and the `[0, gid, from×3, dir×3]` record (JS `launch`
// L168-180, `use` L193-200).
#[test]
fn tr34_1_tracer_fire_geometry_and_record() {
    let mut s = Scene::new();
    let evs = s.deploy(SubKind::Tracer);
    let rec = &kit_events(&evs, 0.0)[0];
    assert_eq!(rec.len(), 8); // JS L197
    // JS L170-171: from = (pos + fwd·0.3, y + 1.2) — yaw 0 ⇒ +z.
    assert!((rec[2] - 0.0).abs() < 1e-6);
    assert!((rec[3] - 1.25).abs() < 1e-6); // 0.05 + 1.2
    assert!((rec[4] - 0.3).abs() < 1e-6);
    // JS L177: pitch = asin(aimDir.y) − 9° ⇒ −9° with a level aim.
    assert!((rec[5] - 0.0).abs() < 1e-6);
    assert!((rec[6] - -0.156).abs() < 1e-6); // r3(sin −9°)
    assert!((rec[7] - 0.988).abs() < 1e-6); // r3(cos 9°)
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SimEvent::SubUse {
                kind: SubKind::Tracer,
                ..
            }
        )),
        "JS L199: emit('sub:use')"
    );
    let b = &s.subs.tracer[0];
    assert_eq!(b.state, TracerState::Fly);
    assert_eq!(b.left, 32.0); // JS L209: `left: sub.range`
    assert_eq!(b.pts.len(), 1); // JS L213: `addPoint(b)` seeds the trail
    assert!(!b.ghost);
    assert_eq!(b.owner_slot, 0);
    assert_eq!(b.team, 0);
    // The live dir is the un-quantised unit vector.
    assert!((b.dir - vec3(0.0, -0.156_434_47, 0.987_688_34)).length() < 1e-5);
}

// TR-34.1 — the floor bounce: the skim flattening (`rebound` L182-191),
// the puddle splat and its turf credit (`bounce` L307-326, `credit` L164).
#[test]
fn tr34_1_tracer_floor_bounce_skims_and_puddles() {
    let mut s = Scene::new();
    let evs = s.deploy(SubKind::Tracer);
    assert!(s.paint.coverage()[0] == 0.0);
    let mut all = evs;
    let mut bounced = false;
    for _ in 0..60 {
        all.extend(s.step());
        if s.subs.tracer[0].bounces > 0 {
            bounced = true;
            break;
        }
    }
    assert!(bounced, "the −9° shot must reach the deck (~8 m out)");
    let b = &s.subs.tracer[0];
    assert_eq!(b.bounces, 1);
    // JS L184-186: mirror off (0,1,0), then `dir.y *= floorExit` and
    // renormalise: 0.15643·0.3 / |·| ≈ 0.04746 (the dn ≥ 0.04 guard holds).
    assert!(
        (b.dir.y - 0.047_457).abs() < 1e-3,
        "skim exit, got {}",
        b.dir.y
    );
    assert!(b.dir.z > 0.99);
    // JS L314: the puddle splat credits turf (unmuted, real bolt).
    assert!(
        all.iter().any(|e| matches!(e, SimEvent::Turf { .. })),
        "bounce puddle credits turf"
    );
    assert!(s.paint.coverage()[0] > 0.0, "the puddle paints the deck");
}

// TR-34.1 — the direct hit: 35 damage, the 9 s mark, the `[1, …]` end
// record with the foe index (JS `advance` L280-292, `directHit` L327-339,
// `end` L340-346).
#[test]
fn tr34_1_tracer_direct_hit_and_end_record() {
    let mut s = Scene::new();
    s.add_enemy(1.0); // just past the muzzle: the head hits before the trail
    let evs = s.deploy(SubKind::Tracer);
    assert!(kit_events(&evs, 1.0).is_empty());
    let all = step_until_end(&mut s, 60);
    let b = &s.subs.tracer[0];
    assert_eq!(b.state, TracerState::Hit);
    assert_eq!(b.direct, Some(1));
    assert_eq!(b.hit_set, vec![1]);
    // JS L329-330: `applyHit(owner, e, 35)` on the local foe.
    assert_eq!(hits(&all), vec![(1, 35.0)]);
    // JS L331: `track(e, team, 9)` (subs.js L537-542).
    assert_eq!(s.actors[1].status.track, 9.0);
    assert_eq!(s.actors[1].status.track_team, Some(0));
    assert_eq!(s.actors[1].hp, 65.0);
    // JS L342: `[1, gid, r3(pos×3), foe]` — the sim carries the actor idx.
    let rec = &kit_events(&all, 1.0)[0];
    assert_eq!(rec.len(), 6);
    assert_eq!(rec[1], b.gid as f32);
    assert_eq!(rec[5], 1.0);
    // The bolt is dropped once the trail fully fades (JS L259, trailLife 1).
    for _ in 0..120 {
        s.step();
    }
    assert!(s.subs.tracer.is_empty(), "faded trail drops the bolt");
}

// TR-34.1 — the trail touch: a foe beside the path (outside the direct
// band 0.38+0.16, inside the trail reach 0.38+0.42) takes 22 and the 3 s
// mark, once per bolt (JS `trailTouch` L348-374).
#[test]
fn tr34_1_tracer_trail_touch_marks_once() {
    let mut s = Scene::new();
    s.add_enemy_at(0.65, 3.0); // 0.54 < 0.65 < 0.80
    s.deploy(SubKind::Tracer);
    let mut all = Vec::new();
    let mut trail_hit = false;
    for _ in 0..60 {
        all.extend(s.step());
        if hits(&all).iter().any(|(v, d)| *v == 1 && *d == 22.0) {
            trail_hit = true;
            break;
        }
    }
    assert!(trail_hit, "the foe touching the live trail is hit");
    assert_eq!(hits(&all), vec![(1, 22.0)]);
    assert_eq!(s.actors[1].status.track, 3.0); // JS L366: trailMark 3
    assert_eq!(s.actors[1].hp, 78.0);
    assert!(
        s.subs.tracer[0].state.live(),
        "a trail touch does not end the bolt"
    );
    // Once per bolt (JS L352 `hitSet.has(e)`): let it fly on and re-scan.
    let before = all.len();
    for _ in 0..30 {
        all.extend(s.step());
    }
    let fresh: Vec<(usize, f32)> = hits(&all[before..]);
    assert!(
        fresh.iter().all(|(v, _)| *v != 1),
        "no second trail hit on the same foe: {fresh:?}"
    );
}

// TR-34.1 — the range budget: a level flight (aim +9° cancels `pitchDown`)
// ends in `range` with no end record (JS L303, `end` L341).
#[test]
fn tr34_1_tracer_range_exhausts_without_record() {
    let mut s = Scene::new();
    s.actors[0].aim_pitch = 9.0 * std::f32::consts::PI / 180.0; // pitch → 0
    s.deploy(SubKind::Tracer);
    let all = step_until_end(&mut s, 120);
    let b = &s.subs.tracer[0];
    assert_eq!(b.state, TracerState::Range);
    assert!(b.left <= 1e-4);
    assert!(b.pos.y > 1.0, "no gravity: it stays at release height");
    assert!(
        kit_events(&all, 1.0).is_empty(),
        "range ends silently (JS L341)"
    );
    assert!(hits(&all).is_empty());
}

// TR-34.1 — the bounce budget: a 1 m corridor ricochets until
// `bounces > maxBounces` ends it in `bounces` (JS L325, `advance` L275).
#[test]
fn tr34_1_tracer_max_bounces_terminates() {
    let mut s = Scene::new();
    // Rebuild the deck with two paintable walls 1 m apart along z.
    let flags = BrushCommon {
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
    };
    let deck = Brush::Box {
        min: [-10.0, -1.0, -10.0],
        max: [10.0, 0.0, 10.0],
        common: flags.clone(),
    };
    let wall = |min: [f32; 3], max: [f32; 3]| Brush::Box {
        min,
        max,
        common: flags.clone(),
    };
    let b = Bounds {
        min_x: -30.0,
        max_x: 30.0,
        min_z: -30.0,
        max_z: 30.0,
    };
    s.world = CollisionWorld::from_brushes(
        &[
            deck,
            wall([0.5, 0.0, -1.0], [1.5, 5.0, 30.0]),
            wall([-1.5, 0.0, -1.0], [-0.5, 5.0, 30.0]),
        ],
        b,
    );
    s.paint = crate::paint::PaintGrid::new(&s.world);
    s.actors[0].aim_yaw = std::f32::consts::FRAC_PI_4;
    s.actors[0].aim_pitch = 9.0 * std::f32::consts::PI / 180.0; // level dir
    s.deploy(SubKind::Tracer);
    let all = step_until_end(&mut s, 120);
    let bolt = &s.subs.tracer[0];
    assert_eq!(bolt.state, TracerState::Bounces);
    assert_eq!(bolt.bounces, 13); // JS L325: `bounces > 12` ends on the 13th
    assert!(bolt.left > 10.0, "the range budget was not the limiter");
    assert!(kit_events(&all, 1.0).is_empty());
}

// TR-34.2 — a ghost bolt replays the flight but decides nothing: no
// puddles, no turf, no damage (its trail still marks a local foe, JS
// L366 has no mute guard), duplicate gids drop, and the end record only
// flips the state (JS `ghost` L219-234, `netGhost` mute wrap).
#[test]
fn tr34_2_tracer_ghost_flies_but_decides_nothing() {
    let mut s = Scene::new();
    s.add_enemy(10.0); // the ghost's owner (team 1) stands away
    let nids = [1u8, 2]; // actor 0 → nid 1, actor 1 → nid 2
    let ghost_rec = |s: &mut Scene, data: &[f32]| {
        s.subs.net_ghost(
            1,
            2,
            "tracer",
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
    // op 0: spawn a ghost aimed level-low at the thrower (it descends ~17°
    // so it ricochets off the deck on the way — muted).
    let evs = ghost_rec(&mut s, &[0.0, 5.0, 0.0, 1.25, 8.0, 0.0, -0.3, -0.95]);
    assert!(kit_events(&evs, 0.0).is_empty(), "ghosts echo no records");
    assert!(s.kit_ghosted(SubKind::Tracer));
    // Duplicate (src_nid, gid) is dropped (JS L223).
    ghost_rec(&mut s, &[0.0, 5.0, 0.0, 1.25, 8.0, 0.0, -0.3, -0.95]);
    assert_eq!(s.kit_count(SubKind::Tracer), 1);

    let mut all = Vec::new();
    for _ in 0..20 {
        all.extend(s.step());
    }
    let b = &s.subs.tracer[0];
    assert!(b.state.live(), "the ghost flies its own path");
    assert!(b.bounces >= 1, "the ghost ricochets too (JS L251)");
    assert!(b.pos.z < 8.0 - 12.0, "it advanced at speed 40");
    assert!(
        !all.iter().any(|e| matches!(e, SimEvent::Turf { .. })),
        "muted bounce puddles never credit (JS ghostMute)"
    );
    assert_eq!(s.paint.coverage()[1], 0.0, "a ghost paints nothing");
    assert_eq!(
        hits(&all),
        Vec::<(usize, f32)>::new(),
        "a ghost hurts nobody"
    );
    assert_eq!(s.actors[0].hp, 100.0);
    // Its trail still marks the thrower it passes (JS L366: no mute guard).
    assert_eq!(s.actors[0].status.track, 3.0);
    assert_eq!(s.actors[0].status.track_team, Some(1));

    // op 1 with the thrower's nid: state flips to `hit`, no damage lands.
    let evs = ghost_rec(&mut s, &[1.0, 5.0, 0.0, 1.25, 2.0, 1.0]);
    assert!(
        kit_events(&evs, 1.0).is_empty(),
        "ghosts echo no end records"
    );
    let b = &s.subs.tracer[0];
    assert_eq!(b.state, TracerState::Hit);
    assert_eq!(s.actors[0].hp, 100.0);
    assert_eq!(hits(&evs), Vec::<(usize, f32)>::new());
    // op 1 with foe −1 (nobody here): plain `hit` end (JS L233).
    ghost_rec(&mut s, &[0.0, 6.0, 0.0, 1.25, 8.0, 0.0, 0.0, -1.0]);
    let evs = ghost_rec(&mut s, &[1.0, 6.0, 0.0, 1.25, 4.0, -1.0]);
    assert!(kit_events(&evs, 1.0).is_empty());
    let g = s
        .subs
        .tracer
        .iter()
        .find(|x| x.gid == 6)
        .expect("second ghost");
    assert_eq!(g.state, TracerState::Hit);
    assert_eq!(g.pos, vec3(0.0, 1.25, 4.0)); // JS L232: pos snapped to the record
}
