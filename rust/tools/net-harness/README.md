# net-harness — TEST-ONLY multi-client driver

**This is a test tool. It is not part of the game, never shipped, never
deployed.** It exists so the Rust client network stack can be exercised
headless inside the sandbox (Task 19, spec FR-M2-4).

## What it drives

Two modes, one scenario (4 clients join a room, exchange payloads, the host
locks, a stranger is refused, one client leaves, survivors re-elect):

- **fake mode** — `FakeRelay`/`FakeTransport` on a virtual clock with the
  seeded `?netlag/&netjitter/&netspike` SIM. Fully deterministic: the same
  seed replays byte-identical outcomes.
- **shim mode** — real `tungstenite` sockets against the test-only
  `relay-shim`. Four clients speak the PROTO v1 wire raw; a fifth feeds
  frames through the production `inkwave_sim::net::transport::RxState`
  handler (the exact path the native/wasm transports run), so the report
  cross-checks the shim against the client stack.

Each mode collects a per-client state snapshot (id, host, roster, payload
order) and prints a consistency report; the integration tests in
`tests/acceptance.rs` assert on the same results.

## Commands

```sh
cargo run -p net-harness            # both modes, report to stdout (exit 1 on inconsistency)
cargo run -p net-harness -- fake    # fake mode only
cargo run -p net-harness -- shim    # shim mode only
cargo test -p net-harness           # acceptance tests (TR-19 harness)
```

## Lag injection

`run_fake(Some(LinkSim { lag_ms, jitter_ms, spike_p }), seed)` applies the
SIM per endpoint; `heartbeat_samples` drives the 2 s `ping`→`pong`
heartbeat and the RTT EMA end-to-end (Task 18 review minors ②/⑥).
