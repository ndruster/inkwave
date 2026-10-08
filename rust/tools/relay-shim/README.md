# relay-shim — TEST-ONLY native relay shim

> **This is not the server.** It is a throwaway, native re-implementation of
> the room semantics of `server/src/index.js` (the Cloudflare Worker +
> Durable Object relay), used only to run Rust↔Rust integration tests inside
> the sandbox, where `wrangler dev` and the deployed relay may be unreachable.
> It is never deployed, never referenced by the game build, and the upstream
> `server/` tree stays byte-for-byte untouched.

## What it is

A plain `ws://` WebSocket server (tungstenite, no TLS) that speaks PROTO v1
exactly as the Worker does:

- `GET /room/<CODE>?name=&create=1&v=1` upgrade → a room.
- Join ladder: `Please refresh…` / `Room code taken` / `Room not found` /
  `Room is full` / `Match in progress`, then `{"t":"err",…}` + `close(4000)`.
- `welcome` (roster incl. self, `host` = oldest), `join` fan-out, `leave`
  broadcast with re-elected host, empty-room auto-unlock.
- `b|<p>` → `m|<from>|<p>` to everyone else; `s|<to>|<p>` → `m|<from>|<p>` to
  one member (sender included). Payloads are forwarded raw, never parsed.
- `{"t":"ping","c":n}` → `{"t":"pong","c":n}`; `{"t":"lock","v"}` host-only.
- Bare `"ping"` → `"pong"` (runtime auto-response / liveness).
- Hygiene: `ORIGIN_OK` allow-list, `CODE` shape, `MSG_MAX`, `RATE`/burst
  cutoff, silence sweep (match/lobby thresholds). All tunable via
  `ShimConfig` (`ShimConfig::relaxed()` for fast tests).

See `src/lib.rs` for the line-by-line mapping to `server/src/index.js` and the
documented deviations (deterministic id mint, Unicode-scalar name slice).

## Use as a library (tests / harness)

```rust
let shim = relay_shim::Shim::spawn(relay_shim::ShimConfig::relaxed()).unwrap();
let transport = inkwave::net::NativeTransport::new(shim.url.clone());
```

## Run the binary

```sh
cargo run -p relay-shim            # ws://127.0.0.1:8787
INKWAVE_RELAY=ws://127.0.0.1:8787 cargo run -p inkwave   # point the client at it
```

Ctrl-C stops it. `Drop` closes the accept/sweep threads.
