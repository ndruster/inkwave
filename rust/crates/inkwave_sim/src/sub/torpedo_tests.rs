//! TR-34.1 / TR-34.3: per-kind behaviour tests for the Tide Torpedo kit.
//!
//! The JS sources are authoritative: `src/config.js` (the `torpedo` row)
//! and `src/game/kits/torpedo.js`. Expected values are anchored to those
//! lines.

use super::SubKind;
use super::tests::{Scene, hits};
use super::torpedo::TorpedoState;
use crate::actor::{FIXED_DT, FireGate};
use crate::weapon::{FoeScan, SimEvent, WeaponKind, WeaponRunner, throw_velocity};
use glam::{Vec3, vec3};

const DT: f32 = FIXED_DT;

/// Deploy the torpedo from the thrower (JS `use` L244-258 via `G.subs.use`).
fn deploy(s: &mut Scene) -> Vec<SimEvent> {
    s.actors[0].sub = SubKind::Torpedo;
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
    s.subs.drain_events()
}

fn kit_events(evs: &[SimEvent], op: f32) -> Vec<Vec<f32>> {
    evs.iter()
        .filter_map(|e| match e {
            SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == "torpedo" && data[0] == op => {
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
            } if k == "torpedo" => Some(data.clone()),
            _ => None,
        })
        .collect()
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

// TR-34.1 — the tuning table values the torpedo port is anchored to.
#[test]
fn tr34_1_torpedo_tuning_values() {
    // JS config.js L382-391 (`torpedo` row).
    let s = Scene::new();
    assert_eq!(super::ink_cost(SubKind::Torpedo, &s.sb), 65.0);
    let t = &s.sb.torpedo;
    assert_eq!(t.throw_speed, 13.5);
    assert_eq!(t.feet_paint, 1.25);
    assert_eq!(t.arm_time, 0.12);
    assert_eq!(t.lock_range, 6.5);
    assert_eq!(t.fly_life, 2.5);
    assert_eq!(t.unfold_time, 0.5);
    assert_eq!(t.launch_speed0, 2.2);
    assert_eq!(t.launch_speed, 9.5);
    assert_eq!(t.launch_accel, 8.0);
    assert_eq!(t.turn_rate, 1.9);
    assert_eq!(t.launch_life, 2.8);
    assert_eq!(t.hp, 20);
    assert_eq!(t.radius, 2.4);
    assert_eq!(t.core_radius, 1.2);
    assert_eq!(t.damage_max, 60.0);
    assert_eq!(t.damage_min, 35.0);
    assert_eq!(t.drops, 10);
    assert_eq!(t.drop_damage, 12.0);
    assert_eq!(t.drop_hits, 3);
}

// TR-34.3 — the throw: 1.35 m up on the shared arc, the `[0, …]` record
// and the feet patch (JS torpedo.js L244-258).
#[test]
fn tr34_1_torpedo_throw_and_feet_paint() {
    let mut s = Scene::new();
    let evs = deploy(&mut s);
    let spawn = &kit_events(&evs, 0.0)[0];
    assert_eq!(spawn.len(), 8); // `[0, gid, pos×3, vel×3]` (JS L247)
    let it = &s.subs.torpedo[0];
    assert_eq!(it.pos.y, 0.05 + 1.35);
    let want = throw_velocity(&s.actors[0], s.sb.torpedo.throw_speed);
    assert!((it.vel - want).length() < 1e-5);
    assert_eq!(it.state, TorpedoState::Fly);
    assert_eq!(it.hp, 20.0); // JS L264 (`hp: sub.hp`), config L387
    // JS L249-250: the throw inks a patch under the thrower's feet.
    assert!(
        evs.iter().any(|e| matches!(e, SimEvent::Turf { .. })),
        "the throw credits the feet patch"
    );
}

// TR-34.3 — one out at a time: `blocked()` refuses the second throw and
// the runner leaves the ink untouched (JS torpedo.js L733, weapons.js
// L145).
#[test]
fn tr34_3_torpedo_one_out() {
    let mut s = Scene::new();
    assert!(!s.subs.kit_blocked(SubKind::Torpedo, 0));
    deploy(&mut s);
    assert!(s.subs.kit_blocked(SubKind::Torpedo, 0));
    // The match layer feeds `blocked()` into the runner; with the gate the
    // release neither arms the throw nor pays the ink (JS weapons.js
    // L145-147: the `inkCost` charge is inside the `!blocked` branch).
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
    // The blocked release dropped the aim (JS weapons.js L143-146: the
    // release edge consumes `aimingSub` either way). A fresh press + an
    // unblocked release does arm the throw and pays the ink.
    runner_frame(&mut r, &mut s, &w, &hold, 3.0 * DT, false);
    runner_frame(&mut r, &mut s, &w, &rel, 4.0 * DT, false);
    assert!(r.pending_sub, "an unblocked release arms the throw");
    assert_eq!(s.actors[0].ink, ink0 - 65.0, "the throw pays `inkCost`");
}

// TR-34.1 — the lock chain: arm after `armTime`, the `[3, …]` lock record,
// the 0.5 s unfold, then the launch ramp and the 10 Hz swim reports.
#[test]
fn tr34_1_torpedo_locks_unfolds_launches() {
    // JS config.js L386: `armTime: 0.12, lockRange: 6.5, unfoldTime: 0.5,
    // launchSpeed0: 2.2, launchAccel: 8`. JS torpedo.js L353-359 (fly →
    // `findTarget` → `startLock`), L374-384 (`[3, gid, foe, pos×3]`),
    // L386-401 (unfold), L325-328 (the `[4, …]` swim report).
    let mut s = Scene::new();
    s.add_enemy_at(0.0, 7.0); // inside `lockRange` along the throw line
    deploy(&mut s);
    let mut lock_rec = None;
    let mut unfolded_at = None;
    let mut launched_at = None;
    let mut speed0 = None;
    for _ in 0..60 * 3 {
        let evs = s.step();
        if let Some(r) = kit_events(&evs, 3.0).first() {
            assert_eq!(r.len(), 6);
            assert_eq!(r[2], 1.0, "the record names the locked foe");
            lock_rec = Some(r.clone());
            unfolded_at = Some(s.time);
        }
        if s.subs
            .torpedo
            .first()
            .is_some_and(|it| it.state == TorpedoState::Launch)
            && launched_at.is_none()
        {
            launched_at = Some(s.time);
            speed0 = Some(s.subs.torpedo[0].speed);
        }
        if launched_at.is_some() {
            break;
        }
    }
    let _ = lock_rec.expect("the torpedo locks onto the foe");
    let unfolded_at = unfolded_at.expect("the lock enters the unfold");
    // The lock fires on the first frame past `armTime` (JS L353).
    assert!(
        (unfolded_at - 0.12).abs() <= 2.0 * DT,
        "the lock waits for armTime ({unfolded_at})"
    );
    let launched_at = launched_at.expect("the unfold finishes and launches");
    assert!(
        (launched_at - unfolded_at - 0.5).abs() <= 2.0 * DT,
        "the unfold lasts `unfoldTime` ({launched_at} - {unfolded_at})"
    );
    assert_eq!(speed0, Some(2.2), "the launch starts at `launchSpeed0`");
    // Ramp: `speed = min(launchSpeed, speed + accel·h)` (JS L417-419).
    for _ in 0..18 {
        s.step();
    }
    let it = &s.subs.torpedo[0];
    assert!(
        (it.speed - (2.2 + 8.0 * 0.3)).abs() < 0.2,
        "the launch ramps at 8 m/s², got {}",
        it.speed
    );
    // The owner's swim reports: `[4, gid, pos×3, dir×3, speed]`, 10 a
    // second while launched (JS L325-328).
    let mut saw_swim = false;
    for _ in 0..30 {
        let evs = s.step();
        if kit_events(&evs, 4.0).iter().any(|r| r.len() == 9) {
            saw_swim = true;
            break;
        }
    }
    assert!(saw_swim, "the launch sends 10 Hz swim reports");
}

// TR-34.1 — the locked burst: the touched foe takes `damageMax` (60), the
// full record flag is set, ten droplets fly out and the shared per-foe cap
// (3 hits × 12) bounds the follow-up damage at 96 total.
#[test]
fn tr34_1_torpedo_burst_core_damage_and_drops() {
    // JS config.js L388-390; torpedo.js L477-513 (burst), L506 (the linear
    // `dmax→dmin` falloff), L539-551 (`spawnDrops`), L584-587 (the
    // `dropHits` cap).
    let mut s = Scene::new();
    s.add_enemy_at(0.0, 5.0);
    deploy(&mut s);
    let mut burst_rec = None;
    let mut burst_hits: Vec<(usize, f32)> = Vec::new();
    for _ in 0..60 * 4 {
        let evs = s.step();
        if let Some(r) = kit_events(&evs, 1.0).first() {
            assert_eq!(r.len(), 6);
            burst_rec = Some(r.clone());
            burst_hits = hits(&evs);
            break;
        }
    }
    let rec = burst_rec.expect("the torpedo reaches its foe and bursts");
    assert_eq!(rec[5], 1.0, "a locked burst is the full one");
    // The foe whose body triggered the burst is a point-blank hit (JS
    // L503-504: `hitFoe` → distance 0 → `damageMax`).
    assert_eq!(
        burst_hits
            .iter()
            .find(|(slot, _)| *slot == 1)
            .map(|(_, d)| *d),
        Some(60.0),
        "the touched foe takes the core damage"
    );
    // The burst throws ten droplets (JS L513 + config L390); the foe's body
    // is right at the burst centre, so several already died on it in the
    // burst frame (`tickDrops` runs in the same pass, JS L552-570). The
    // survivors carry the droplet spec.
    assert!(!s.subs.torpedo_drops.is_empty(), "droplets fly out");
    for d in &s.subs.torpedo_drops {
        assert_eq!(d.dmg, 12.0);
        assert_eq!(d.cap, 3);
        assert_eq!(d.paint, 0.65);
        assert!(!d.ghost);
    }
    // The droplets: at most three land on the same foe (the burst group's
    // shared `dropHits` counter), each for 12.
    let mut drop_hits = 0usize;
    for _ in 0..60 * 4 {
        let evs = s.step();
        for (slot, dmg) in hits(&evs) {
            if slot == 1 {
                assert_eq!(dmg, 12.0, "a droplet hit is `dropDamage`");
                drop_hits += 1;
            }
        }
        if s.subs.torpedo_drops.is_empty() {
            break;
        }
    }
    assert!(
        drop_hits <= 3,
        "the shared cap allows three droplet hits, got {drop_hits}"
    );
}

// TR-34.1 — a lethal blast that catches a locked torpedo bursts it *full*
// in mid-air: the `[1, …, 1]` record and all ten droplets.
#[test]
fn tr34_1_torpedo_triggered_burst_throws_ten_drops() {
    // JS torpedo.js L313-316 (the `triggered` burst at the head of `tick`),
    // L666-675 / `damageArea` (a lethal blast catches a locked torpedo),
    // L513 (`spawnDrops` on the full burst), config L390 (`drops: 10`).
    let mut s = Scene::new();
    s.add_enemy_at(0.0, 7.0); // locks, but stays far from the hover point
    deploy(&mut s);
    for _ in 0..60 * 2 {
        s.step();
        if s.subs
            .torpedo
            .first()
            .is_some_and(|it| it.state == TorpedoState::Unfold)
        {
            break;
        }
    }
    let p = s.subs.torpedo[0].pos;
    s.subs.damage_area(p, 3.0, 60.0, 1, false);
    let evs = s.step();
    let rec = &kit_events(&evs, 1.0)[0];
    assert_eq!(rec[5], 1.0, "the caught torpedo bursts full");
    assert!(
        s.subs.torpedo.is_empty(),
        "the burst torpedo is spliced out"
    );
    assert!(
        end_events(&evs).is_empty(),
        "a burst suppresses the `[2, …]` end record"
    );
    assert_eq!(s.subs.torpedo_drops.len(), 10, "`drops: 10` (L390)");
    for d in &s.subs.torpedo_drops {
        assert_eq!(d.dmg, 12.0);
        assert_eq!(d.cap, 3);
        assert_eq!(d.paint, 0.65);
    }
    // The distant foe is outside the 2.4 m burst radius: nothing hurt.
    assert!(
        hits(&evs).is_empty(),
        "the mid-air burst misses the distant foe"
    );
}

// TR-34.1 — a torpedo that never locks bursts *small* on its first floor
// touch: the `[1, …]` record carries `full = 0` and no droplets fly.
#[test]
fn tr34_1_torpedo_uncontested_bursts_small() {
    // JS torpedo.js L349-351 (`fly`: a surface touch bursts `false`),
    // L481 (`full` rides the record), L513 (`spawnDrops` only when full).
    let mut s = Scene::new();
    deploy(&mut s);
    let mut rec = None;
    for _ in 0..60 * 4 {
        let evs = s.step();
        if let Some(r) = kit_events(&evs, 1.0).first() {
            rec = Some(r.clone());
            break;
        }
    }
    let rec = rec.expect("the arc lands and bursts");
    assert_eq!(rec[5], 0.0, "an unlocked burst is the small one");
    assert!(
        s.subs.torpedo_drops.is_empty(),
        "the small burst throws no droplets"
    );
    assert!(
        end_events(&s.subs.drain_events()).is_empty(),
        "a burst suppresses the `[2, …]` end record"
    );
}

// TR-34.3 — shoot-down: 20 damage pops it mid-flight, the splice sends the
// `[2, gid, shot]` end record and nothing bursts or hurts.
#[test]
fn tr34_3_torpedo_shoot_down() {
    // JS config.js L387 (`hp: 20`); torpedo.js L529-534 (`hurt` →
    // `destroy`), L331-334 (the dead splice's `[2, gid, 1]`).
    let mut s = Scene::new();
    deploy(&mut s);
    s.step();
    let p = s.subs.torpedo[0].pos;
    let gid = s.subs.torpedo[0].gid;
    // One 20-damage shot through the body: `hp` 20 → 0 (JS `hurt` L530).
    assert!(
        s.subs.block_shot(
            p - vec3(1.0, 0.0, 0.0),
            p + vec3(1.0, 0.0, 0.0),
            1,
            20.0,
            false,
            &s.sb
        ),
        "the shot is absorbed by the torpedo"
    );
    let it = &s.subs.torpedo[0];
    assert_eq!(it.state, TorpedoState::Dead);
    assert!(it.shot);
    // A second shot finds nothing live to absorb it.
    assert!(!s.subs.block_shot(
        p - vec3(1.0, 0.0, 0.0),
        p + vec3(1.0, 0.0, 0.0),
        1,
        20.0,
        false,
        &s.sb
    ));
    let evs = s.step();
    let ends = end_events(&evs);
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0],
        vec![2.0, gid as f32, 1.0],
        "the end record flags `shot`"
    );
    assert!(
        kit_events(&evs, 1.0).is_empty(),
        "a shot-down torpedo never bursts"
    );
    assert!(hits(&evs).is_empty(), "no damage rides a shot-down torpedo");
    assert!(s.subs.torpedo.is_empty());
}
