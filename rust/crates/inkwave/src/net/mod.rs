//! Task 18 — the real WebSocket transports for the `inkwave` app crate.
//!
//! Frame semantics live in `inkwave_sim::net::transport` (shared `RxState` +
//! the deterministic `FakeTransport`); this module only owns I/O:
//! - native: [`native_transport::NativeTransport`] (tungstenite, one thread
//!   per link, Bevy main thread never blocks);
//! - wasm: [`wasm_transport::WasmTransport`] (web-sys `WebSocket`, driven by
//!   browser events).
// The transports are the Task 18 deliverable; nothing in the game loop drives
// them until the session layer (Task 19) wires them in.
#![allow(dead_code)]

#[cfg(not(target_arch = "wasm32"))]
pub mod native_transport;
#[cfg(target_arch = "wasm32")]
pub mod wasm_transport;
