//! Task 19 net-harness CLI — prints the consistency report.
//!
//! `cargo run -p net-harness [fake|shim|all]`. TEST-ONLY; see `lib.rs` for
//! what the two modes drive and assert.

use net_harness::{LinkSim, run_fake, run_shim};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("all");
    let mut any_fail = false;

    if mode == "all" || mode == "fake" {
        let rep = run_fake(
            Some(LinkSim {
                lag_ms: 30.0,
                jitter_ms: 20.0,
                spike_p: 0.05,
            }),
            42,
        );
        rep.print();
        any_fail |= !rep.ok();
    }
    if mode == "all" || mode == "shim" {
        let rep = run_shim();
        rep.print();
        any_fail |= !rep.ok();
    }
    if any_fail {
        std::process::exit(1);
    }
}
