//! Task 9 autopilot: drives a full [`Match`] with [`Bot`] brains (or idle
//! input) and emits the headless-run JSON report the TR-9 acceptance needs.
//!
//! The loop mirrors the JS `main.js` fixed-step order: every 60 Hz frame the
//! bots read a *snapshot* of the roster (the JS brains read `G.actors` live;
//! the sim hands `Bot::step` a mutable actor, so the shared context is a
//! clone), produce [`ActorInput`]s, and the match steps once with them.
//! Everything is seeded: the match RNG, each bot's private stream, and the
//! projectile stream — two runs with the same seed are bit-identical (TR-9.2).
//!
//! Deviations from JS (recorded for review):
//!   - **Roster snapshot**: JS `BotBrain.update` reads the live actor array;
//!     here the snapshot is taken once per frame before any bot steps, so a
//!     bot sees the previous frame's roster when it is not its own turn. The
//!     JS bots also run sequentially with live reads; the observable spread
//!     is one frame (16 ms), absorbed by the bot think-tick jitter.
//!   - **`autopilot = false`**: every slot gets idle input (the CLI switch
//!     exists for the TR-8-style control run; the spec's "1 autopilot + 7
//!     bot" is satisfied by the all-bot mode, which strictly covers it).
//!   - **Frame timing**: wall-clock per step, native runs only, and it is
//!     inherently nondeterministic. [`Report::frame_times`] is therefore an
//!     `Option`: the CLI prints the deterministic core (timing `null`) on
//!     stdout so TR-9.2 can diff two runs byte-for-byte, and writes the full
//!     document (timing included) to `--out` when given.
//!   - **Intro / Finish freeze**: JS `BotBrain.update` L342 early-exits with
//!     zeroed intents whenever the match is not `playing`; the brains' timers
//!     and RNG streams stay frozen there. [`crate::bot::Bot::hold`] mirrors
//!     that for the Intro/Finish windows (the dead-plan-clear still runs, the
//!     rest of the update does not).

use serde::{Deserialize, Serialize};

use crate::actor::{ActorInput, FIXED_DT, SimWorld};
use crate::bot::{Bot, BotCtx, BotStats};
use crate::collision::CollisionWorld;
use crate::geometry::StageLayout;
use crate::match_::{FINISH_TIME, INTRO_TIME, Match, Phase};
use crate::nav::NavGraph;
use crate::tuning::Tuning;

/// Per-bot counters in the report (identity + [`BotStats`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BotReport {
    pub team: usize,
    pub slot: usize,
    #[serde(flatten)]
    pub stats: BotStats,
}

/// Fixed-step wall-clock timing summary (native runs only, ms per step).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FrameTimes {
    pub steps: u32,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

/// The headless-run JSON document (TR-9.1 evidence).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub seed: u64,
    pub duration: f32,
    pub map: String,
    pub autopilot: bool,
    /// Match settlement (null when the run never reached `End`).
    pub result: Option<crate::match_::MatchResult>,
    /// Total splats logged by the match.
    pub kills: u32,
    /// Inked area per team, m² (`coverage × turf_area`).
    pub turf_m2: [f32; 2],
    pub bots: Vec<BotReport>,
    /// Wall-clock step timing (native `--out` documents only; `null` in the
    /// deterministic stdout report, see the module docs).
    pub frame_times: Option<FrameTimes>,
    /// TR-9.3: longest stall window across all bots, s.
    pub max_stall: f32,
    /// TR-9.3: total displacement-watchdog firings across all bots.
    pub stalls: u32,
    /// Whether the run settled before the step budget expired.
    pub settled: bool,
    /// Boss Battle settlement when the run drove a Boss Battle (Task 44;
    /// `None` for turf / zones runs).
    #[serde(default)]
    pub boss: Option<crate::boss::BossOutcome>,
    /// Aggregated boss damage dealt by all attackers (TR-44.1 reconciliation:
    /// `boss_dmg + boss.hp ≈ max_hp` on lose, `boss_dmg ≈ max_hp` on win —
    /// tolerance 0.51 for the `Math.round` on `hp`).
    #[serde(default)]
    pub boss_dmg: f32,
    /// Aggregated weak hits on the boss (TR-44.1 reconciliation).
    #[serde(default)]
    pub weak_hits: u32,
}

/// Run one full match. `duration` is the match clock; the intro/finish
/// windows add themselves on top (same as [`Match::new`] semantics).
#[must_use]
pub fn run(
    duration: f32,
    seed: u64,
    autopilot: bool,
    layout: &StageLayout,
    collision: &CollisionWorld,
    tuning: &Tuning,
) -> Report {
    let sim = SimWorld::new(collision, layout);
    let nav = NavGraph::new(collision, layout, &tuning.player);
    let mut m = Match::new(duration, seed, &sim, &tuning.player, &tuning.match_config);
    let n = m.actors.len();
    let mut bots: Vec<Bot> = m
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| Bot::new(a, &tuning.difficulty.easy, seed, i))
        .collect();
    let mut inputs = vec![ActorInput::default(); n];
    let mut mate_goals = vec![None; n];
    #[cfg(not(target_arch = "wasm32"))]
    let mut ft_samples: Vec<f64> = Vec::new();

    // One extra second of budget beyond the expected settle point.
    let max_steps = ((INTRO_TIME + duration + FINISH_TIME) * 60.0) as usize + 60;
    let mut settled = false;
    for _ in 0..max_steps {
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = std::time::Instant::now();

        if autopilot {
            if m.phase == Phase::Active {
                let snap = m.actors.clone();
                for (i, b) in bots.iter().enumerate() {
                    mate_goals[i] = b.goal_node();
                }
                let mate_minds = bots.iter().map(|b| Some(b.mind())).collect::<Vec<_>>();
                // Task 38 batch 3: drive the shared Zone Control plan (JS
                // `zonePlan().tick()` rides the match update; every actor is
                // a bot here).
                let mut zone_plan = m.zone_plan.take();
                if let Some(zctl) = m.zones.as_ref() {
                    let mut p = zone_plan.take().unwrap_or_else(|| {
                        crate::bot::zones::ZonePlan::build(zctl, &nav, &m.paint, &sim)
                    });
                    let is_bot = vec![true; n];
                    p.tick(zctl, &m.paint, &sim, &snap, &m.kinds, &is_bot);
                    zone_plan = Some(p);
                }
                for i in 0..n {
                    let ctx = BotCtx {
                        actors: &snap,
                        nav: &nav,
                        paint: &m.paint,
                        world: &sim,
                        t: &tuning.player,
                        weapons: &tuning.weapons,
                        weapon_range: &tuning.weapon_range,
                        kinds: &m.kinds,
                        runners: &m.runners,
                        mate_goals: &mate_goals,
                        subs: &m.subs,
                        mate_minds: &mate_minds,
                        zones: zone_plan.as_ref(),
                        zctl: m.zones.as_ref(),
                        boss: None,
                    };
                    inputs[i] = bots[i].step(FIXED_DT, &mut m.actors[i], &ctx);
                }
                m.zone_plan = zone_plan;
            } else {
                // JS `BotBrain.update` L342: outside `playing` the brains
                // early-exit with zeroed intents — timers and RNG stay frozen.
                for i in 0..n {
                    inputs[i] = bots[i].hold(&m.actors[i]);
                }
            }
        } else {
            for inp in inputs.iter_mut() {
                *inp = ActorInput::default();
            }
        }
        m.step(
            FIXED_DT,
            &inputs,
            &sim,
            &tuning.player,
            &tuning.weapons,
            &tuning.subs,
            None,
        );

        #[cfg(not(target_arch = "wasm32"))]
        ft_samples.push(t0.elapsed().as_secs_f64() * 1e3);

        if m.phase == Phase::End {
            settled = true;
            break;
        }
    }

    let cov = m.paint.coverage();
    let area = m.paint.turf_area;
    let bots_rep: Vec<BotReport> = bots
        .iter()
        .map(|b| BotReport {
            team: b.team,
            slot: b.slot,
            stats: b.stats,
        })
        .collect();
    let max_stall = bots
        .iter()
        .map(|b| b.stats.max_stall)
        .fold(0.0f32, f32::max);
    let stalls: u32 = bots.iter().map(|b| b.stats.stalls).sum();

    #[cfg(not(target_arch = "wasm32"))]
    let frame_times = Some(summarize(&ft_samples));
    #[cfg(target_arch = "wasm32")]
    let frame_times = None;

    Report {
        seed,
        duration,
        map: layout.stage.id.clone(),
        autopilot,
        result: m.result,
        kills: m.kills.len() as u32,
        turf_m2: [cov[0] * area, cov[1] * area],
        bots: bots_rep,
        frame_times,
        max_stall,
        stalls,
        settled,
        boss: None,
        boss_dmg: 0.0,
        weak_hits: 0,
    }
}

/// Driver knobs for [`run_boss`] (Task 44 batch B smoke driver — the JS
/// equivalent is the `startMatch({mode:'boss', …})` opts, match.js L19-21).
#[derive(Debug, Clone, Copy)]
pub struct BossRunOpts<'d> {
    /// Match clock; `<= 0` takes `BOSS_MODE.duration` (JS match.js L21).
    pub duration: f32,
    /// Difficulty string driving the HP / damage tables (JS
    /// `opts.difficulty`, bossMode.js L22-23; `""` = `'normal'`).
    pub difficulty: &'d str,
    /// Test hook for the win branch: once the elapsed clock reaches this,
    /// inject a lethal hit on the boss (JS routes it through the weapon hit
    /// path; the sim driver applies it directly). `None` = the squad never
    /// lands the kill.
    pub kill_at: Option<f32>,
}

/// Run one full Boss Battle with bot brains (the offline analogue of
/// `startMatch({mode:'boss'})`, match.js L55-100 / bossMode.js). The match
/// carries the [`crate::tuning::BossModeRules`] slice; the [`crate::boss::BossMode`]
/// wrapper is owned here in the driver (Task 44 §2) and handed to
/// [`Match::step`] every frame, with the bots targeting it through
/// [`BotCtx::boss`].
#[must_use]
pub fn run_boss(
    seed: u64,
    layout: &StageLayout,
    collision: &CollisionWorld,
    tuning: &Tuning,
    opts: BossRunOpts<'_>,
) -> Report {
    let sim = SimWorld::new(collision, layout);
    let nav = NavGraph::new(collision, layout, &tuning.player);
    let bnav = crate::boss::nav::BossNav::new(&sim, &tuning.boss.nav);
    let rules = tuning.boss.mode;
    let squad = rules.squad.max(1) as usize;
    // JS bossMode.js L21: `units = squadUnits(team-0 actors)` — slot 0 counts
    // as the local human, the rest as bots.
    let units = crate::boss::squad_units(1, squad - 1, &tuning.boss.squad_units);
    let duration = if opts.duration <= 0.0 {
        rules.duration as f32
    } else {
        opts.duration
    };
    let mut m = Match::with_roster_mode(
        duration,
        seed,
        &sim,
        &tuning.player,
        &tuning.match_config,
        squad,
        crate::match_::MatchMode::Boss,
        None,
        None,
        Some(rules),
    );
    // JS L23: `sim: !match.follower` — the offline driver is always the host.
    let mut bm = crate::boss::BossMode::new(
        &sim,
        &bnav,
        &tuning.boss,
        units,
        opts.difficulty,
        true,
        seed,
    );
    let n = m.actors.len();
    let mut bots: Vec<Bot> = m
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| Bot::new(a, &tuning.difficulty.easy, seed, i))
        .collect();
    let mut inputs = vec![ActorInput::default(); n];
    let mut mate_goals = vec![None; n];
    #[cfg(not(target_arch = "wasm32"))]
    let mut ft_samples: Vec<f64> = Vec::new();

    // Budget: boss intro + clock + the longer of the two finish windows.
    let max_steps =
        ((rules.intro + duration + rules.finish_win.max(rules.finish_lose)) * 60.0) as usize + 60;
    let mut settled = false;
    for _ in 0..max_steps {
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = std::time::Instant::now();

        if m.phase == Phase::Active {
            let snap = m.actors.clone();
            for (i, b) in bots.iter().enumerate() {
                mate_goals[i] = b.goal_node();
            }
            let mate_minds = bots.iter().map(|b| Some(b.mind())).collect::<Vec<_>>();
            for i in 0..n {
                let ctx = BotCtx {
                    actors: &snap,
                    nav: &nav,
                    paint: &m.paint,
                    world: &sim,
                    t: &tuning.player,
                    weapons: &tuning.weapons,
                    weapon_range: &tuning.weapon_range,
                    kinds: &m.kinds,
                    runners: &m.runners,
                    mate_goals: &mate_goals,
                    subs: &m.subs,
                    mate_minds: &mate_minds,
                    zones: None,
                    zctl: None,
                    // JS `G.boss` (bots.js L345): the brains fight the boss.
                    boss: Some(&bm.boss),
                };
                inputs[i] = bots[i].step(FIXED_DT, &mut m.actors[i], &ctx);
            }
        } else {
            // JS `BotBrain.update` L342: brains early-exit outside `playing`.
            for i in 0..n {
                inputs[i] = bots[i].hold(&m.actors[i]);
            }
        }

        let ka = opts.kill_at.unwrap_or(f32::INFINITY);
        if m.phase == Phase::Active && !bm.boss.dead && m.elapsed() >= ka {
            // The kill rides the regular damage path so `killer` /
            // `boss_kill` bookkeeping matches JS `onDefeat`.
            bm.boss.state = m.phase;
            let hp = bm.boss.hp;
            bm.boss.apply_damage(
                Some(crate::boss::BossAttacker {
                    nid: 0,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                hp,
                false,
                None,
            );
        }
        m.step(
            FIXED_DT,
            &inputs,
            &sim,
            &tuning.player,
            &tuning.weapons,
            &tuning.subs,
            Some(&mut bm),
        );

        #[cfg(not(target_arch = "wasm32"))]
        ft_samples.push(t0.elapsed().as_secs_f64() * 1e3);

        if m.phase == Phase::End {
            settled = true;
            break;
        }
    }

    let cov = m.paint.coverage();
    let area = m.paint.turf_area;
    let bots_rep: Vec<BotReport> = bots
        .iter()
        .map(|b| BotReport {
            team: b.team,
            slot: b.slot,
            stats: b.stats,
        })
        .collect();
    let max_stall = bots
        .iter()
        .map(|b| b.stats.max_stall)
        .fold(0.0f32, f32::max);
    let stalls: u32 = bots.iter().map(|b| b.stats.stalls).sum();

    #[cfg(not(target_arch = "wasm32"))]
    let frame_times = Some(summarize(&ft_samples));
    #[cfg(target_arch = "wasm32")]
    let frame_times = None;

    let boss = m.result.as_ref().and_then(|r| r.boss.clone());
    // TR-44.1 reconciliation: Σ stats_by_nid.{boss_dmg,weak_hits}.
    let boss_dmg: f32 = bm.boss.stats_by_nid.values().map(|s| s.boss_dmg).sum();
    let weak_hits: u32 = bm.boss.stats_by_nid.values().map(|s| s.weak_hits).sum();
    Report {
        seed,
        duration,
        map: layout.stage.id.clone(),
        autopilot: true,
        result: m.result,
        kills: m.kills.len() as u32,
        turf_m2: [cov[0] * area, cov[1] * area],
        bots: bots_rep,
        frame_times,
        max_stall,
        stalls,
        settled,
        boss,
        boss_dmg,
        weak_hits,
    }
}

/// Mean / percentiles / max over the per-step wall times (ms).
#[cfg(not(target_arch = "wasm32"))]
fn summarize(samples: &[f64]) -> FrameTimes {
    if samples.is_empty() {
        return FrameTimes::default();
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let pct = |q: f64| s[((samples.len() as f64 - 1.0) * q).round() as usize];
    FrameTimes {
        steps: samples.len() as u32,
        mean_ms: mean,
        p50_ms: pct(0.5),
        p95_ms: pct(0.95),
        max_ms: *s.last().unwrap(),
    }
}

#[cfg(test)]
mod tests;
