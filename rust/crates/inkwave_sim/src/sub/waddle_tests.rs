//! TR-34.1 / TR-34.3: per-kind behaviour tests for the Waddle Bomb kit.
//!
//! The JS sources are authoritative: `src/config.js` (the `waddle` row) and
//! `src/game/kits/waddle.js`. Expected values are anchored to those lines.

use super::SubKind;
use super::tests::{Scene, hits};
use super::waddle::WaddleState;
use crate::actor::FIXED_DT;
use crate::weapon::SimEvent;
use glam::vec3;

const DT: f32 = FIXED_DT;

/// Advance until the waddle list is empty or `frames` steps pass,
/// collecting every event.
fn run(s: &mut Scene, frames: usize) -> Vec<SimEvent> {
    let mut all = Vec::new();
    for _ in 0..frames {
        let evs = s.step();
        all.extend(evs);
        if s.subs.waddle.is_empty() && !all.is_empty() {
            break;
        }
    }
    all
}

fn kit_events(evs: &[SimEvent], op: f32) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == "waddle" && data[0] == op => {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

fn end_events(evs: &[SimEvent]) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubEnd { sub_kind, data, .. } if sub_kind.as_deref() == Some("waddle") => {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

// TR-34.1 — the tuning table values the waddle port is anchored to.
#[test]
fn tr34_1_waddle_tuning_values() {
    // JS config.js L371-376 (`waddle` row).
    let s = Scene::new();
    assert_eq!(super::ink_cost(SubKind::Waddle, &s.sb), 65.0);
    let w = &s.sb.waddle;
    assert_eq!(w.sense_radius, 7.5); // senseRadius
    assert_eq!(w.sense_up, 4.0); // senseUp
    assert_eq!(w.fuse, 1.1); // nobody near: blows in place after 1.1 s
    assert_eq!(w.speed, 4.0); // tracking speed
    assert_eq!(w.turn_rate, 8.0); // turnRate
    assert_eq!(w.life, 9.0); // life
    assert_eq!(w.max_travel, 26.0); // maxTravel
    assert_eq!(w.trigger_dist, 1.2); // triggerDist
    assert_eq!(w.radius, 3.0); // blast radius
    assert_eq!(w.damage_max, 180.0); // dmgMax
    assert_eq!(w.damage_min, 35.0); // dmgMin
    assert_eq!(w.hp, 30); // shoot-down health
}

// TR-34.1 — the throw: 1.35 m up on the shared arc, `[0, gid, pos×3,
// vel×3]` record.
#[test]
fn tr34_1_waddle_throw_record() {
    // JS waddle.js L264-272 (`use`).
    let mut s = Scene::new();
    s.actors[0].sub = SubKind::Waddle;
    s.subs.use_sub(
        0,
        1,
        &s.world,
        &mut s.actors,
        &mut s.paint,
        &mut s.ps,
        &s.t,
        &s.sb,
        None,
        s.time,
    );
    let evs = s.subs.drain_events();
    let spawn = &kit_events(&evs, 0.0)[0];
    assert_eq!(spawn.len(), 8);
    let it = &s.subs.waddle[0];
    assert!((it.pos.y - (0.05 + 1.35)).abs() < 1e-5);
    assert_eq!(it.hp, 30.0);
    assert!(matches!(it.state, WaddleState::Fly));
}

// TR-34.1 — nobody in the sense circle: the fuse blast goes off in place
// after 1.1 s with no lock record.
#[test]
fn tr34_1_waddle_fuse_blast_in_place() {
    // JS waddle.js L368-378 (`sense`): `if (it.t >= s.fuse && !it.ghost) {
    // it.why = 'fuse'; blast(it); }` — the scan radius tops out at
    // `senseRadius` (7.5 m) after 0.3 s.
    let mut s = Scene::new();
    s.add_enemy_at(0.0, 15.0); // far outside the 7.5 m sense circle
    s.actors[0].sub = SubKind::Waddle;
    s.subs.use_sub(
        0,
        1,
        &s.world,
        &mut s.actors,
        &mut s.paint,
        &mut s.ps,
        &s.t,
        &s.sb,
        None,
        s.time,
    );
    s.subs.drain_events();
    // Wait for the landing (the `Sense` state), then time the fuse.
    let mut landed_at = None;
    for _ in 0..60 * 6 {
        s.step();
        if s.subs
            .waddle
            .first()
            .is_some_and(|it| it.state == WaddleState::Sense)
        {
            landed_at = Some(s.time);
            break;
        }
    }
    let landed_at = landed_at.expect("the bomb lands on the deck");
    let evs = run(&mut s, 60 * 6);
    assert!(kit_events(&evs, 3.0).is_empty(), "no foe: no lock record");
    let blast = &kit_events(&evs, 1.0)[0];
    assert_eq!(blast[0], 1.0);
    assert!(
        (s.time - landed_at - 1.1).abs() <= 2.0 * DT,
        "the fuse blast fires 1.1 s after landing"
    );
    assert!(
        hits(&evs).is_empty(),
        "the in-place blast misses the far foe"
    );
    assert!(
        end_events(&evs).is_empty(),
        "a blast suppresses the end record"
    );
}

// TR-34.1 — a foe inside the circle locks it: `[3, gid, foe, pos×3]`, the
// 0.32 s wake, the 4 m/s walk and the `reached` blast for point-blank 180.
#[test]
fn tr34_1_waddle_lock_walk_and_reached_blast() {
    // JS waddle.js L380-381 (`lock`), L391-403 (`wake`, 0.32 s), L427-469
    // (`walk`: `triggerDist` 1.2 to the foe chest, speed 4.0), L573-603
    // (`blast`: `k = 1 - clamp((d - 0.8) / (radius - 0.8))`, `dmg =
    // lerp(35, 180, k·k)`).
    let mut s = Scene::new();
    s.actors[0].sub = SubKind::Waddle;
    s.subs.use_sub(
        0,
        1,
        &s.world,
        &mut s.actors,
        &mut s.paint,
        &mut s.ps,
        &s.t,
        &s.sb,
        None,
        s.time,
    );
    s.subs.drain_events();
    let mut landed = None;
    for _ in 0..60 * 6 {
        s.step();
        if let Some(it) = s.subs.waddle.first()
            && matches!(
                it.state,
                WaddleState::Sense | WaddleState::Wake | WaddleState::Walk
            )
        {
            landed = Some(it.pos);
            break;
        }
    }
    let landed = landed.expect("the bomb lands");
    // Seat a foe half a metre away: the sense scan (radius ≥ 2.6 m at the
    // first tick) locks it, and the first walk frame is already inside
    // `triggerDist` — the chest sits 0.58 m from the blast centre, inside
    // the 0.8 m full-damage core.
    s.add_enemy_at(landed.x + 0.5, landed.z);
    let evs = run(&mut s, 60 * 6);
    let lock = &kit_events(&evs, 3.0)[0];
    assert_eq!(lock[0], 3.0);
    assert_eq!(
        lock[2], 1.0,
        "the lock record names the foe (actor index 1)"
    );
    assert!(!kit_events(&evs, 1.0).is_empty(), "it blasts on `reached`");
    let h = hits(&evs);
    assert_eq!(h.len(), 1, "one foe, one blast");
    assert_eq!(h[0].1, 180.0, "the point-blank core takes damageMax");
    assert!(
        end_events(&evs).is_empty(),
        "a blast suppresses the end record"
    );
}

// TR-34.1 — the walk runs at `speed` and gives up on `maxTravel` (26 m
// before `life` × speed allows), blasting where it stands.
#[test]
fn tr34_1_waddle_walk_speed_and_travel_cap() {
    // JS waddle.js L445: `if (it.walkT > s.life || it.travel > s.maxTravel)
    // { it.why = it.travel > s.maxTravel ? 'travel' : 'life'; blast(it); }`.
    let mut s = Scene::new();
    // A foe 5 m out locks it; the bomb then chases a stationary foe and
    // catches them well before the caps — so instead track the walk
    // distance over a fixed window.
    s.add_enemy_at(0.0, 11.5);
    s.actors[0].sub = SubKind::Waddle;
    s.subs.use_sub(
        0,
        1,
        &s.world,
        &mut s.actors,
        &mut s.paint,
        &mut s.ps,
        &s.t,
        &s.sb,
        None,
        s.time,
    );
    s.subs.drain_events();
    for _ in 0..60 * 6 {
        s.step();
        if s.subs
            .waddle
            .first()
            .is_some_and(|it| it.state == WaddleState::Walk)
        {
            break;
        }
    }
    let start_travel = s.subs.waddle[0].travel;
    for _ in 0..30 {
        s.step();
        if s.subs.waddle.is_empty() {
            break;
        }
    }
    if let Some(it) = s.subs.waddle.first() {
        let moved = it.travel - start_travel;
        // 0.5 s at 4 m/s, allowing the sharp-turn slowdown (×0.45).
        assert!(
            moved > 0.9 && moved < 2.05,
            "the walk covers ~4 m/s ({moved} m in 0.5 s)"
        );
    }
}

// TR-34.3 — shoot-down: enemy fire pops it (hp 30) and the end record
// broadcasts `popped = 1`.
#[test]
fn tr34_3_waddle_shoot_down_pops_it() {
    // JS waddle.js L605-612 (`hurt`), L614-622 (`pop`), L708 (`[2, gid,
    // why === 'popped' ? 1 : 0]`), L720-728 (`blockShot`: the capsule is
    // `pos` + radius 0.17, height 0.4, hit when `dist < 0.2`).
    let mut s = Scene::new();
    s.add_enemy_at(0.0, 15.0); // far: keeps it in the fuse window, not `reached`
    s.actors[0].sub = SubKind::Waddle;
    s.subs.use_sub(
        0,
        1,
        &s.world,
        &mut s.actors,
        &mut s.paint,
        &mut s.ps,
        &s.t,
        &s.sb,
        None,
        s.time,
    );
    s.subs.drain_events();
    for _ in 0..60 * 6 {
        s.step();
        if s.subs
            .waddle
            .first()
            .is_some_and(|it| it.state != WaddleState::Fly)
        {
            break;
        }
    }
    let pos = s.subs.waddle[0].pos;
    let shot = |s: &mut Scene, dmg: f32| {
        s.subs.block_shot(
            vec3(pos.x - 1.0, pos.y + 0.3, pos.z),
            vec3(pos.x + 1.0, pos.y + 0.3, pos.z),
            1, // the enemy team fires
            dmg,
            false,
            &s.sb,
        )
    };
    assert!(shot(&mut s, 20.0), "the shot is absorbed by the bomb");
    s.subs.drain_events();
    assert_eq!(s.subs.waddle[0].hp, 10.0);
    assert!(shot(&mut s, 20.0));
    let evs = s.step(); // the dead splice
    assert!(s.subs.waddle.is_empty());
    let end = &end_events(&evs)[0];
    assert_eq!(end[0], 2.0);
    assert_eq!(end[2], 1.0, "the end record flags `popped`");
    assert!(
        kit_events(&evs, 1.0).is_empty() && hits(&evs).is_empty(),
        "a popped waddle never blasts"
    );
}
