//! TR-34.1 / TR-34.3: per-kind behaviour tests for the Shaker kit sub.
//!
//! The JS sources are authoritative: `src/config.js` (the `shaker` row) and
//! `src/game/kits/shaker.js`. Expected values are anchored to those lines.

use super::SubKind;
use super::tests::{Scene, hits};
use crate::actor::{FIXED_DT, FireGate};
use crate::weapon::{FoeScan, SimEvent, WeaponKind, WeaponRunner, throw_velocity};
use glam::Vec3;

const DT: f32 = FIXED_DT;

/// Deploy the shaker from the thrower at a given charge level (JS `use`
/// L264-275 via `G.subs.use`, the runner's `pendingKitLevel`).
fn deploy(s: &mut Scene, level: u32) -> Vec<SimEvent> {
    s.actors[0].sub = SubKind::Shaker;
    s.subs.use_sub(
        0,
        level,
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
            SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == "shaker" && data[0] == op => {
                Some(data.clone())
            }
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
        false,
        None,
    );
}

// TR-34.1 — ink cost and the throw geometry.
#[test]
fn tr34_1_shaker_ink_cost_and_throw() {
    // JS config.js L366: `inkCost: 60`.
    let s = Scene::new();
    assert_eq!(super::ink_cost(SubKind::Shaker, &s.sb), 60.0);
    // JS shaker.js L269-272: released 1.35 m up on the shared
    // `throwVelocity` arc at `throwSpeed` (config L366: 13), recorded as
    // `[0, gid, pos×3, vel×3, level]`.
    assert_eq!(s.sb.shaker.throw_speed, 13.0);
    let mut s = Scene::new();
    let evs = deploy(&mut s, 1);
    let spawn = &kit_events(&evs, 0.0)[0];
    assert_eq!(spawn[0], 0.0);
    assert_eq!(spawn[8], 1.0); // the charge level rides the throw record
    let it = &s.subs.shaker[0];
    assert!((it.pos.y - (0.05 + 1.35)).abs() < 1e-5);
    let want = throw_velocity(&s.actors[0], s.sb.shaker.throw_speed);
    assert!((it.vel - want).length() < 1e-5);
    assert_eq!(it.left, 1); // JS L266: `left = level`
}

// TR-34.1 — the charge level selects the blast count (1..=3).
#[test]
fn tr34_1_shaker_level_sets_blast_count() {
    // JS shaker.js L266 (`it.left = it.level = level`) and L406-457 (one
    // blast per `left`).
    for level in [1u32, 2, 3] {
        let mut s = Scene::new();
        deploy(&mut s, level);
        assert_eq!(s.subs.shaker[0].level, level);
        assert_eq!(s.subs.shaker[0].left, level);
    }
}

// TR-34.1 — the hold charge: plain holds fill at 1/chargeTime, the level
// thresholds sit at meter 0.5 and 1.
#[test]
fn tr34_1_shaker_hold_charge_levels() {
    // JS shaker.js L192-222 (`hold`): `meter += dt / chargeTime` per frame
    // at rest (the budget cap `min(0.3, budget + maxRate·dt)` never binds
    // without the boost terms), `level = meter >= 1 ? 3 : meter >= 0.5 ? 2
    // : 1` (L212). config.js L366-367: `chargeTime: 2.7`, `maxRate: 1.6`.
    let s = Scene::new();
    assert_eq!(s.sb.shaker.charge_time, 2.7);
    assert_eq!(s.sb.shaker.max_rate, 1.6);
    // The runner counts a hold from the second press frame (the first only
    // raises `aimingSub`) and once more on the release frame, so `holds`
    // press frames + the release frame fill `holds · DT / 2.7` of meter.
    let cases: [(usize, u32); 3] = [(10, 1), (85, 2), (170, 3)];
    for (holds, want) in cases {
        let mut s = Scene::new();
        s.actors[0].sub = SubKind::Shaker;
        let mut r = WeaponRunner::new();
        let w = crate::embedded_tuning().weapons;
        let mut time = 0.0f32;
        let ink0 = s.actors[0].ink;
        let hold = FireGate {
            sub: true,
            ..Default::default()
        };
        for _ in 0..holds {
            time += DT;
            runner_frame(&mut r, &mut s, &w, &hold, time);
        }
        let rel = FireGate {
            sub_released: true,
            ..Default::default()
        };
        time += DT;
        runner_frame(&mut r, &mut s, &w, &rel, time);
        assert!(r.pending_sub, "release after {holds} holds");
        assert_eq!(r.pending_kit_level, want, "level after {holds} holds");
        // JS weapons.js L147: the release pays `inkCost` (60).
        assert_eq!(s.actors[0].ink, ink0 - 60.0);
    }
}

// TR-34.1 — fuse 0.5 after the first floor contact, gap 0.42 between
// blasts, three blasts for a level-3 throw.
#[test]
fn tr34_1_shaker_fuse_and_blast_cadence() {
    // JS config.js L366-370: `fuse: 0.5`, `gap: 0.42`. JS shaker.js
    // L436-439: the fuse / `next` countdowns run every frame once armed;
    // L488-525: each blast re-arms `it.next = s.gap`.
    let mut s = Scene::new();
    assert_eq!(s.sb.shaker.fuse, 0.5);
    assert_eq!(s.sb.shaker.gap, 0.42);
    deploy(&mut s, 3);
    let mut blasts: Vec<f32> = Vec::new();
    let mut armed_at = None;
    for _ in 0..60 * 12 {
        let evs = s.step();
        if armed_at.is_none() && s.subs.shaker.first().is_some_and(|it| it.armed) {
            armed_at = Some(s.time);
        }
        if !kit_events(&evs, 1.0).is_empty() {
            blasts.push(s.time);
        }
        if s.subs.shaker.is_empty() {
            break;
        }
    }
    assert_eq!(blasts.len(), 3, "a level-3 throw fires three blasts");
    let armed_at = armed_at.expect("the can arms on its first floor touch");
    assert!(
        (blasts[0] - armed_at - 0.5).abs() <= 2.0 * DT,
        "the first blast lands on the 0.5 s fuse ({} vs armed {armed_at})",
        blasts[0]
    );
    for w in blasts.windows(2) {
        assert!(
            (w[1] - w[0] - 0.42).abs() <= 2.0 * DT,
            "blast gap {} should be the 0.42 gap",
            w[1] - w[0]
        );
    }
}

// TR-34.1 — per-blast damage: the 0.8 m core takes `damageMax` (110),
// nothing lands beyond `radius` (2.3).
#[test]
fn tr34_1_shaker_blast_damage_and_radius() {
    // JS config.js L366-370. JS shaker.js L497-505: `k = 1 - clamp((d -
    // 0.8) / (radius - 0.8), 0, 1)`, `dmg = lerp(damageMin, damageMax,
    // k·k)` from the blast centre to the foe's chest (kid: +0.7).
    let mut s = Scene::new();
    assert_eq!(s.sb.shaker.radius, 2.3);
    assert_eq!(s.sb.shaker.damage_max, 110.0);
    assert_eq!(s.sb.shaker.damage_min, 30.0);
    deploy(&mut s, 1);
    // Wait for the first floor contact, then pin the can: the JS landing
    // bounce (`contactFloor` L459-464) walks it ~1.7 m forward during the
    // 0.5 s fuse, which would make the blast geometry unpredictable.
    // Zeroing the horizontal velocity on the arming frame parks it at the
    // armed point — a fixture stabilisation; `blast()` is untouched.
    let mut land = None;
    for _ in 0..60 * 6 {
        s.step();
        if let Some(it) = s.subs.shaker.first_mut()
            && it.armed
        {
            it.vel.x = 0.0;
            it.vel.z = 0.0;
            land = Some(it.pos);
            break;
        }
    }
    let land = land.expect("the can finds the deck");
    s.add_enemy_at(land.x, land.z); // slot 1: inside the 0.8 m core
    s.add_enemy_at(land.x + 1.5, land.z); // slot 2: mid falloff
    s.add_enemy_at(land.x + 3.0, land.z); // slot 3: outside the radius
    let mut blast_c = None;
    let mut core_hit = None;
    let mut mid_hit = None;
    let mut far_hits = 0usize;
    for _ in 0..60 * 6 {
        let evs = s.step();
        if let Some(rec) = kit_events(&evs, 1.0).first() {
            blast_c = Some(Vec3::new(rec[2], rec[3], rec[4]));
        }
        for (slot, dmg) in hits(&evs) {
            match slot {
                1 => {
                    let _ = core_hit.get_or_insert(dmg);
                }
                2 => {
                    let _ = mid_hit.get_or_insert(dmg);
                }
                _ => far_hits += 1,
            }
        }
        if s.subs.shaker.is_empty() {
            break;
        }
    }
    let c = blast_c.expect("the level-1 throw fires its single blast");
    assert!(
        (c - land).length() < 0.02,
        "the pinned can blasts at its armed point ({c} vs {land})"
    );
    // The chest sits 0.75 - 0.19 = 0.56 m above the resting centre: inside
    // the 0.8 m core, so the blast is a point-blank `damageMax`.
    assert_eq!(core_hit, Some(110.0), "the core takes damageMax");
    // Mid falloff: `d = sqrt(1.5^2 + 0.56^2)`, `k = 1 - (d - 0.8) / 1.5`,
    // `dmg = lerp(30, 110, k·k)` — compared against the *recorded* centre
    // (r2-quantised), so allow the quantisation slop.
    let d = (Vec3::new(land.x + 1.5, 0.05 + 0.7, land.z) - c).length();
    let k = 1.0 - ((d - 0.8) / (2.3 - 0.8)).clamp(0.0, 1.0);
    let want = 30.0 + (110.0 - 30.0) * k * k;
    let got = mid_hit.expect("the mid-range foe is hit");
    assert!(
        (got - want).abs() < 1.0,
        "mid falloff: got {got}, formula says {want} (d {d})"
    );
    assert_eq!(far_hits, 0, "nothing lands beyond the blast radius");
}

// TR-34.1 — the travel trail drips ink every `trailEvery` metres.
#[test]
fn tr34_1_shaker_trail_drips_every_038_m() {
    // JS config.js L366-370: `trailEvery: 0.38`. JS shaker.js L428-432:
    // the drip fires while airborne once the accumulator passes the step.
    // JS `drip` (L480-485) hands each droplet to the projectile stream
    // (`spawnDrop`), so the drips are counted as `ps.list` entries, not as
    // turf events (the splat credit rides the drops' own floor touch, which
    // the fixture's un-stepped projectile sim never runs).
    let mut s = Scene::new();
    assert_eq!(s.sb.shaker.trail_every, 0.38);
    deploy(&mut s, 1);
    let mut dist = 0.0f32;
    let mut blasted = false;
    for _ in 0..60 * 4 {
        let evs = s.step();
        if !kit_events(&evs, 1.0).is_empty() {
            blasted = true;
            break;
        }
        if let Some(it) = s.subs.shaker.first()
            && it.ground == 0.0
        {
            let hs = it.vel.x.hypot(it.vel.z);
            if hs > 1.5 {
                dist += hs * DT;
            }
        }
    }
    assert!(blasted, "the can goes off before the trail window ends");
    // The accumulator drips exactly once per `trailEvery` metres of airborne
    // horizontal travel (JS L430: `while trail >= trailEvery`).
    let drips = s.ps.list.len();
    assert_eq!(
        drips,
        (dist / 0.38).floor() as usize,
        "one drip per 0.38 m of flight"
    );
    // Sanity: the ~7.4 m flight really lays a trail.
    assert!(drips >= 15, "the airborne can drips its trail, got {drips}");
}
