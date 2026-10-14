//! Task 18 — the real WebSocket transports for the `inkwave` app crate.
//!
//! Frame semantics live in `inkwave_sim::net::transport` (shared `RxState` +
//! the deterministic `FakeTransport`); this module only owns I/O:
//! - native: [`native_transport::NativeTransport`] (tungstenite, one thread
//!   per link, Bevy main thread never blocks);
//! - wasm: [`wasm_transport::WasmTransport`] (web-sys `WebSocket`, driven by
//!   browser events).
//!
//! Task 24 wires the session layer in: [`AppTransport`] picks the platform
//! transport, [`RelayBase`] carries the resolved relay URL, and
//! [`OnlineSession`] owns the [`NetSession`] state machine the online UI
//! drives (mirrors `src/net/session.js` as the JS `Net.session` singleton).

#[cfg(not(target_arch = "wasm32"))]
pub mod native_transport;
#[cfg(target_arch = "wasm32")]
pub mod wasm_transport;

pub mod match_link;

use bevy::prelude::Resource;
use inkwave_sim::net::session::{NetSession, Profile};

use crate::ui::online::OnlineFlags;

#[cfg(not(target_arch = "wasm32"))]
use native_transport::NativeTransport as PlatformTransport;
#[cfg(target_arch = "wasm32")]
use wasm_transport::WasmTransport as PlatformTransport;

/// The transport the app's online stack runs on (native tungstenite /
/// browser web-sys — same [`inkwave_sim::net::transport::Transport`] API).
pub type AppTransport = PlatformTransport;

/// Resolved relay base URL (`ws://…` / `wss://…`). Inserted once at boot from
/// `--relay=` / `INKWAVE_RELAY` / `INKWAVE_HOST` (native) or `location` +
/// `?relay=` (wasm) — the JS `relayURL()` seam (transport.js L20-42).
#[derive(Resource, Clone, Debug)]
pub struct RelayBase(pub String);

/// Resolve the relay base for the current platform. On native a CLI
/// `--relay=` override wins (then env, then PROD —
/// [`native_transport::NativeTransport::resolve_relay_base`]); on wasm the
/// page location decides and the CLI override is meaningless.
#[must_use]
pub fn default_relay_base(cli_override: Option<&str>) -> RelayBase {
    #[cfg(not(target_arch = "wasm32"))]
    {
        RelayBase(native_transport::NativeTransport::resolve_relay_base(
            cli_override,
        ))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = cli_override;
        RelayBase(wasm_transport::relay_url_from_location())
    }
}

/// Deterministic stand-in for the JS `Math.random()` room-code mint
/// (session.js L74; NFR-M2-3). Fixed seed: the code only has to be unique
/// per retry attempt, not unguessable, and the retry loop is bounded.
const SESSION_RNG_SEED: u64 = 20_261_014;

/// Session-layer resource: the one `NetSession` the online UI talks to
/// (JS `Net.session`, session.js L12) plus the pure-layer UI flags
/// ([`OnlineFlags`] — busy / code draft / banners, the JS screen-local `st`).
///
/// Deviation note (Task 24 plan): stored via `insert_non_send_resource` and
/// accessed with the `NonSend`/`NonSendMut` system params — the platform
/// transports cannot satisfy Bevy's `Resource: Send + Sync` bound (native
/// `Receiver` is `!Sync`; wasm `Rc` is `!Send`). The Bevy main thread owns
/// the session, so non-send storage is exactly the right semantics.
pub struct OnlineSession {
    pub sess: NetSession<AppTransport, Box<dyn FnMut() -> AppTransport>>,
    pub flags: OnlineFlags,
}

impl OnlineSession {
    /// Build the session for `relay_base`. The transport factory mints a
    /// fresh platform transport per connect attempt (the JS `_connect`
    /// rebuilds `this.tr`, session.js L95-100).
    #[must_use]
    pub fn new(relay_base: &str) -> Self {
        let base = relay_base.to_string();
        let factory: Box<dyn FnMut() -> AppTransport> =
            Box::new(move || AppTransport::new(base.clone()));
        Self {
            sess: NetSession::new(Profile::default(), SESSION_RNG_SEED, factory),
            flags: OnlineFlags {
                code_draft: "     ".to_string(),
                ..Default::default()
            },
        }
    }
}
