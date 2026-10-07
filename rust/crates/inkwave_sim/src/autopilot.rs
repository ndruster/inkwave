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
                for i in 0..n {
                    let ctx = BotCtx {
                        actors: &snap,
                        nav: &nav,
                        paint: &m.paint,
                        world: &sim,
                        t: &tuning.player,
                        w: &tuning.spritzer,
                        mate_goals: &mate_goals,
                    };
                    inputs[i] = bots[i].step(FIXED_DT, &mut m.actors[i], &ctx);
                }
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
        m.step(FIXED_DT, &inputs, &sim, &tuning.player, &tuning.spritzer);

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
