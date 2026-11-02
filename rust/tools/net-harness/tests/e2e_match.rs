//! Task 25 — TR-25.1 / TR-25.2 / TR-25.3: full-stack end-to-end matches on
//! the virtual clock (1 host + 2 guests, bots padded to 8, shortened
//! duration). TEST-ONLY. The scenario driver and its consistency checks live
//! in `net_harness::match_e2e`; these tests pin the scenario matrix and
//! assert the reports.

use net_harness::match_e2e::MatchOpts;
use net_harness::{DropAt, LinkSim, run_match};

/// TR-25.1: a lagless full match settles consistently across all three ends.
#[test]
fn tr25_full_match_three_clients() {
    let rep = run_match(&MatchOpts::plain(3, 42));
    rep.print();
    assert_eq!(rep.clients.len(), 3);
    assert!(rep.clients.iter().all(|c| c.live));
    assert!(rep.ok(), "every TR-25.1 check passes");
}

/// TR-25.1 determinism: the same seed replays to the identical final paint
/// grid and the identical settled result on every client.
#[test]
fn tr25_full_match_deterministic() {
    let a = run_match(&MatchOpts::plain(3, 77));
    let b = run_match(&MatchOpts::plain(3, 77));
    assert!(a.ok() && b.ok());
    for (x, y) in a.clients.iter().zip(b.clients.iter()) {
        assert_eq!(x.grid_fnv, y.grid_fnv, "final paint grid identical");
        assert_eq!(x.result, y.result, "settled result identical");
    }
}

/// TR-25.2: the `?netlag=40&netjitter=30&netspike=0.01` SIM still settles
/// consistently, and the kink / path-error / teleport stats meet the numeric
/// gates calibrated by the Task 25 review (spec Open Question 3): p95
/// smoothness per client + teleport counts. p99/max spike outliers are the
/// scenario's own input (`netspike=0.01`) — reported, not gated, like the JS
/// `net-test.mjs` which only prints them.
#[test]
fn tr25_lag_scenario_stats() {
    let mut opts = MatchOpts::plain(3, 99);
    opts.sim = Some(LinkSim {
        lag_ms: 40.0,
        jitter_ms: 30.0,
        spike_p: 0.01,
    });
    opts.stats = true;
    let rep = run_match(&opts);
    rep.print();
    assert!(rep.ok(), "the lagged match settles consistently");
    let stats = rep.stats.as_ref().expect("stats collected");
    assert_eq!(stats.per_client.len(), 3);
    for c in &stats.per_client {
        assert!(c.frames > 0, "client {} recorded remote frames", c.client);
        assert!(
            c.kink.p95 <= 5.0,
            "c{} kink p95 {} cm",
            c.client,
            c.kink.p95
        );
        assert!(
            c.turn_kink.p95 <= 3.0,
            "c{} turn-kink p95 {} °",
            c.client,
            c.turn_kink.p95
        );
        assert!(
            c.path_err.p95 <= 30.0,
            "c{} path-error p95 {} cm",
            c.client,
            c.path_err.p95
        );
        assert!(c.teleports <= 2, "c{} teleports {}", c.client, c.teleports);
    }
    let tele: usize = stats.per_client.iter().map(|c| c.teleports).sum();
    assert!(tele <= 5, "total teleports {tele}");
}

/// TR-25.3 host drop: the host's socket dies mid-match; the relay re-elects,
/// a guest adopts authority, and the match runs to settlement.
#[test]
fn tr25_host_migration() {
    let mut opts = MatchOpts::plain(3, 1234);
    opts.drop = Some(DropAt {
        client: 0,
        at_s: 5.0,
    });
    let rep = run_match(&opts);
    rep.print();
    assert!(!rep.clients[0].live, "the host dropped out");
    assert!(
        rep.clients.iter().any(|c| c.live && c.adopted_host),
        "a live client adopted the host role"
    );
    assert!(rep.ok(), "the migrated match settled consistently");
}

/// Task 36 batch 3 — Zone Control online: the host runs the rules and
/// records every decision, the guests replay them; the decision state must
/// agree on every frame and the settled zones outcome must be identical
/// across ends. 120 s of match clock exercises ≥1 rotation (rotateMin..
/// rotateMax = 30..60 s inside the 90 s pre-final window, zones.js L154-157),
/// the last-30-s lock (`zf`) and the horn; the turf-oriented bots never hold
/// a zone to the `control` share, so the level horn goes to sudden death
/// (`zt -1`) and the test-shortened overtime cap (`overtime_max` 300 → 20 s
/// through the builder's `zones_tuning` seam) ends it on the `ze` record —
/// the tiebreak winner rides the wire to every guest.
#[test]
fn tr36_zones_match_three_clients() {
    let mut opts = MatchOpts::plain(3, 4242);
    opts.mode = "zones".to_string();
    opts.duration = 120.0;
    let rep = run_match(&opts);
    rep.print();
    assert!(rep.clients.iter().all(|c| c.live));
    assert!(rep.ok(), "the zones match settles consistently");
}

/// TR-25.3 guest drop: a guest's socket dies mid-match; its actor is adopted
/// as a bot on the host and keeps playing to settlement.
#[test]
fn tr25_guest_dropped_to_bot() {
    let mut opts = MatchOpts::plain(3, 555);
    opts.drop = Some(DropAt {
        client: 2,
        at_s: 5.0,
    });
    let rep = run_match(&opts);
    rep.print();
    assert!(!rep.clients[2].live, "the guest dropped out");
    assert_eq!(
        rep.dropped_actor_moved,
        Some(true),
        "the adopted actor kept moving on the host"
    );
    assert!(rep.ok(), "the drop-out match settled consistently");
}
