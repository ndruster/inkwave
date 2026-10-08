//! TR-19 harness acceptance tests (tasks.md Task 19: "多客户端无头对局
//! 驱动器——起 shim、拉起 N 个 in-process 客户端会话、注入 lag 场景、
//! 收集各端状态快照输出一致性报告").
//!
//! The scenario library lives in `net_harness::lib`; these tests assert the
//! consistency reports directly (the CLI prints the same thing).

use net_harness::{LinkSim, heartbeat_samples, run_fake, run_shim};

#[test]
fn fake_scenario_lagless_is_consistent() {
    let rep = run_fake(None, 1);
    rep.print();
    assert!(rep.ok(), "fake lagless: {:?}", rep.checks);
    // Lagless link: the relay answers every ping at the same virtual
    // instant, so the heartbeat EMA must be exactly 0 everywhere.
    assert!(rep.rtts.iter().all(|r| *r == 0.0));
}

#[test]
fn fake_scenario_lag_jitter_spike_is_consistent_and_deterministic() {
    let sim = Some(LinkSim {
        lag_ms: 40.0,
        jitter_ms: 25.0,
        spike_p: 0.3,
    });
    let a = run_fake(sim, 0xC0FFEE);
    let b = run_fake(sim, 0xC0FFEE);
    assert!(a.ok(), "fake lag: {:?}", a.checks);
    // Same seed ⇒ byte-identical scenario outcomes (NFR-M2-3 determinism).
    assert_eq!(a.clients, b.clients);
    assert_eq!(a.rtts, b.rtts);
    // The EMA of samples each ≥ lag_ms is itself ≥ lag_ms.
    assert!(a.rtts.iter().all(|r| *r >= 40.0));
}

#[test]
fn shim_scenario_is_consistent() {
    let rep = run_shim();
    rep.print();
    assert!(rep.ok(), "shim: {:?}", rep.checks);
}

#[test]
fn heartbeat_cadence_and_rtt_end_to_end() {
    // Task 18 minor ②: the fake relay now answers the runtime `ping`→`pong`,
    // so the 2 s cadence and the RTT EMA are observable end-to-end.
    // 300 ticks × 10 ms = 3 s ⇒ at least two pings (first on welcome, next
    // at +2000 ms) and an EMA of 0 on a lagless link.
    let (rtts, bytes) = heartbeat_samples(None, 300);
    assert!(rtts.iter().all(|r| *r == 0.0), "lagless rtt: {rtts:?}");
    assert!(bytes >= 8, "two heartbeats of 4 bytes: {bytes}");

    // Constant 50 ms lag, no jitter/spike ⇒ every sample is exactly 50 once
    // the first pong lands (the first few ticks still read 0).
    let sim = Some(LinkSim {
        lag_ms: 50.0,
        jitter_ms: 0.0,
        spike_p: 0.0,
    });
    let (rtts, _) = heartbeat_samples(sim, 300);
    assert!(rtts[..5].iter().all(|r| *r == 0.0), "pre-pong: {rtts:?}");
    assert!(
        rtts[10..].iter().all(|r| (r - 50.0).abs() < 1e-9),
        "constant-lag rtt EMA: {:?}",
        &rtts[10..]
    );
}
