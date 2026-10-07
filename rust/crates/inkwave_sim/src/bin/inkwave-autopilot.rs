//! Headless autopilot CLI (Task 9): runs a full Tidewater match with easy
//! bots and prints the deterministic JSON report.
//!
//! ```text
//! cargo run -p inkwave_sim --bin inkwave-autopilot -- \
//!     --duration 90 --seed 12345 --autopilot --out report.json
//! ```
//!
//! Flags: `--duration <s>` (match clock, default 90), `--seed <u64>`
//! (default 1), `--autopilot` (bot brains on; default off = idle input),
//! `--out <path>` (also write the full report, frame timing included).
//! stdout carries the deterministic core (`frame_times: null`) so two runs
//! with the same seed diff clean (TR-9.2). The wall-clock step-timing summary
//! (TR-16.1) is printed to **stderr** as `p50/p95 FPS` so stdout stays
//! byte-for-byte diffable.
use inkwave_sim::autopilot::{self, Report};

fn main() {
    let mut duration = 90.0f32;
    let mut seed = 1u64;
    let mut autopilot = false;
    let mut out: Option<std::path::PathBuf> = None;

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--duration" => {
                if let Some(v) = iter.next() {
                    duration = v.parse().expect("--duration wants a number");
                }
            }
            "--seed" => {
                if let Some(v) = iter.next() {
                    seed = v.parse().expect("--seed wants an integer");
                }
            }
            "--autopilot" => autopilot = true,
            "--out" => {
                if let Some(v) = iter.next() {
                    out = Some(v.into());
                }
            }
            other => panic!("unknown flag `{other}` (see --help comment)"),
        }
    }

    let layout = inkwave_sim::embedded_tidewater();
    let collision = inkwave_sim::collision::CollisionWorld::from_layout(&layout);
    let tuning = inkwave_sim::embedded_tuning();
    let report = autopilot::run(duration, seed, autopilot, &layout, &collision, &tuning);

    if let Some(path) = &out {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap())
            .expect("--out must be writable");
    }
    // TR-16.1: step-rate summary on stderr (nondeterministic by nature; kept
    // off stdout so TR-9.2 diffs stay clean). These are fixed-step SIM rates
    // (1000 / step_ms), NOT render FPS — the render loop must stay under this.
    if let Some(ft) = &report.frame_times {
        eprintln!(
            "[autopilot] sim steps={} mean={:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms \
             | sim p50 FPS={:.1} sim p95 FPS={:.1}",
            ft.steps,
            ft.mean_ms,
            ft.p50_ms,
            ft.p95_ms,
            ft.max_ms,
            if ft.p50_ms > 0.0 {
                1000.0 / ft.p50_ms
            } else {
                0.0
            },
            if ft.p95_ms > 0.0 {
                1000.0 / ft.p95_ms
            } else {
                0.0
            },
        );
    }
    let mut stdout_report: Report = report;
    stdout_report.frame_times = None;
    println!("{}", serde_json::to_string(&stdout_report).unwrap());
}
