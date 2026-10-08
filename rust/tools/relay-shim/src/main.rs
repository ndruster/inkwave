//! TEST-ONLY relay shim binary.
//!
//! NOT a deployment target and NOT a replacement for `server/` — see
//! `src/lib.rs` and `README.md`. Start it when you want the Rust client to
//! talk to a local PROTO v1 relay (e.g. `INKWAVE_RELAY=ws://127.0.0.1:8787`).
//!
//! Usage: `relay-shim [port]` (default `8787`, the `wrangler dev` port the
//! client's localhost probe expects). Ctrl-C stops it.

use std::io::Write as _;

use relay_shim::{Shim, ShimConfig};

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8787);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let shim = match Shim::bind(addr, ShimConfig::default()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("relay-shim: bind {addr} failed: {e}");
            std::process::exit(1);
        }
    };
    println!("relay-shim (TEST ONLY) listening on {}", shim.url);
    println!("  production hygiene: MSG_MAX=65536 RATE=90 sweep match/lobby = 20s/150s");
    let _ = std::io::stdout().flush();
    // Park until interrupted; `Drop` stops the accept/sweep threads.
    loop {
        std::thread::park();
    }
}
