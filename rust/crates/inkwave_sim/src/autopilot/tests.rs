//! Task 9 autopilot tests: the TR-9.1/9.2/9.3 evidence runs. Kept short
//! (12 s match clock) so the suite stays fast; the CLI produces the full
//! 90 s evidence JSON.

use super::*;
use crate::collision::CollisionWorld;

struct Env {
    layout: crate::geometry::StageLayout,
    collision: CollisionWorld,
    tuning: crate::tuning::Tuning,
}

fn env() -> Env {
    Env {
        layout: crate::embedded_tidewater(),
        collision: CollisionWorld::tidewater(),
        tuning: crate::embedded_tuning(),
    }
}

/// The deterministic report core (frame timing is wall-clock noise; it is
/// `null` on the CLI stdout for exactly this reason).
fn core_json(r: &Report) -> String {
    let mut c = r.clone();
    c.frame_times = None;
    serde_json::to_string(&c).unwrap()
}

impl Env {
    fn run(&self, duration: f32, seed: u64, autopilot: bool) -> Report {
        super::run(
            duration,
            seed,
            autopilot,
            &self.layout,
            &self.collision,
            &self.tuning,
        )
    }
}

// ------------------------------------------------------------------ TR-9.1
#[test]
fn tr9_1_full_run_no_panic_all_bots_active_and_both_teams_paint() {
    let e = env();
    let r = e.run(12.0, 0xABCD_1234, true);
    assert!(r.settled, "the run must reach End inside the step budget");
    let res = r.result.expect("End carries a result");
    assert_eq!(r.bots.len(), 8);
    for b in &r.bots {
        assert!(
            b.stats.dist > 5.0,
            "bot {}/{} moved: {}",
            b.team,
            b.slot,
            b.stats.dist
        );
        assert!(b.stats.shots > 0, "bot {}/{} fired", b.team, b.slot);
    }
    assert!(
        res.coverage[0] > 0.0 && res.coverage[1] > 0.0,
        "both teams painted: {:?}",
        res.coverage
    );
    assert!(
        res.coverage.iter().all(|c| *c >= 0.0 && *c <= 1.0),
        "coverage is a valid fraction: {:?}",
        res.coverage
    );
    assert!(
        (res.coverage[0] + res.coverage[1]) < 1.0,
        "teams cannot cover more than the field"
    );
    println!(
        "TR-9.1: settled, cov={:.4}/{:.4}, kills={}, turf={:.0}/{:.0} m2",
        res.coverage[0], res.coverage[1], r.kills, r.turf_m2[0], r.turf_m2[1]
    );
}

// ------------------------------------------------------------------ TR-9.2
#[test]
fn tr9_2_same_seed_is_bit_identical() {
    let e = env();
    let a = core_json(&e.run(12.0, 777, true));
    let b = core_json(&e.run(12.0, 777, true));
    assert_eq!(a, b, "same seed must replay byte-for-byte");
    // a different seed must diverge (the run is not frozen)
    let c = core_json(&e.run(12.0, 778, true));
    assert_ne!(a, c, "different seeds must diverge");
    println!("TR-9.2: {} bytes identical across runs", a.len());
}

// ------------------------------------------------------------------ TR-9.3
#[test]
fn tr9_3_no_bot_stalls_past_the_watchdog_window() {
    let e = env();
    let r = e.run(12.0, 0x5EED_900D, true);
    // The displacement watchdog fires at 1.5 s windows; TR-9.3 forbids a bot
    // being pinned to one spot for > 5 s. Report the worst observed window.
    assert!(
        r.max_stall < 5.0,
        "no >5 s stall: worst {:.2}s, {} watchdog firings",
        r.max_stall,
        r.stalls
    );
    for b in &r.bots {
        assert!(
            b.stats.max_stall < 5.0,
            "bot {}/{} stalled {:.2}s",
            b.team,
            b.slot,
            b.stats.max_stall
        );
    }
    println!(
        "TR-9.3: max_stall={:.2}s, watchdog firings={}",
        r.max_stall, r.stalls
    );
}

#[test]
fn idle_control_run_makes_no_progress() {
    // autopilot off = the Task 8 control path: intro, clock, no shots.
    let e = env();
    let r = e.run(6.0, 42, false);
    assert!(r.settled);
    assert_eq!(r.kills, 0);
    assert!(
        r.bots
            .iter()
            .all(|b| b.stats.shots == 0 && b.stats.dist < 1.0)
    );
    assert_eq!(r.turf_m2, [0.0, 0.0]);
}

// ---------------------------------------------------- Task 44 batch B
/// TR-44.1 (smoke): one full Boss Battle through the driver.
fn boss_run(seed: u64, duration: f32, kill_at: Option<f32>) -> Report {
    let e = env();
    super::run_boss(
        seed,
        &e.layout,
        &e.collision,
        &e.tuning,
        BossRunOpts {
            duration,
            difficulty: "normal",
            kill_at,
        },
    )
}

#[test]
fn run_boss_lose_branch() {
    // Clock runs out with the boss alive: finish 3.2 s, winner 1.
    let r = boss_run(0xB05_1234, 40.0, None);
    assert!(r.settled, "the boss run must reach End inside the budget");
    let res = r.result.clone().expect("boss match settles with a result");
    let b = res
        .boss
        .clone()
        .expect("boss outcome folded into the result");
    assert_eq!(b.name, "HULLBREAKER");
    assert!(!b.win);
    assert_eq!(res.winner, 1, "survived clock = squad loses");
    assert!(b.hp > 0.0, "boss survived: hp {}", b.hp);
    assert_eq!(b.max_hp, 93_600.0, "round(18000 × 5.2 × 1)");
    assert_eq!(res.points, [0.0, 0.0], "JS boss result has no points line");
    assert!(r.boss.as_ref() == Some(&b));
    // TR-44.1 reconciliation: Σ stats_by_nid boss damage + surviving HP
    // accounts for the full pool (0.51 tolerance = the result's `Math.round`).
    assert!(
        (r.boss_dmg + b.hp - b.max_hp).abs() <= 0.51,
        "bossDmg reconciliation: dmg {} + hp {} vs maxHp {}",
        r.boss_dmg,
        b.hp,
        b.max_hp
    );
    // TR-44.1 determinism: same seed twice → byte-identical settlement.
    let r2 = boss_run(0xB05_1234, 40.0, None);
    assert_eq!(
        serde_json::to_string(&r.result).unwrap(),
        serde_json::to_string(&r2.result).unwrap()
    );
    assert_eq!(r.boss_dmg, r2.boss_dmg);
    assert_eq!(r.weak_hits, r2.weak_hits);
    println!(
        "TR-44.1 lose: hp={:.0}/{:.0} t={:.1}",
        b.hp, b.max_hp, b.time
    );
}

#[test]
fn run_boss_win_branch() {
    // Lethal hit injected at elapsed 20 s: finish 5.2 s, winner 0.
    let r = boss_run(0xB05_9999, 40.0, Some(20.0));
    assert!(r.settled);
    let res = r.result.clone().expect("boss match settles with a result");
    let b = res
        .boss
        .clone()
        .expect("boss outcome folded into the result");
    assert!(b.win, "the injected kill must land");
    assert_eq!(res.winner, 0);
    assert_eq!(b.hp, 0.0);
    assert_eq!(b.max_hp, 93_600.0);
    // JS killT = duration - time at the kill ≈ 40 - 20 = 20.0 (round1).
    assert!(
        (b.time - 20.0).abs() < 0.1,
        "kill clock should be ~20.0: {}",
        b.time
    );
    // TR-44.1 reconciliation: the boss died, so every HP point was accounted
    // — Σ stats_by_nid boss_dmg equals the full pool (the injected lethal
    // routes through `apply_damage`, so it lands in the same table).
    assert!(
        (r.boss_dmg - b.max_hp).abs() <= 0.51,
        "win-branch dmg reconciliation: {} vs {}",
        r.boss_dmg,
        b.max_hp
    );
    println!("TR-44.1 win: t={:.1} phase={}", b.time, b.phase);
}
