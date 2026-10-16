//! Task 26 — the test-only net smoke seam (TR-26.1/TR-26.2).
//!
//! The JS app's smoke harness drives its client through the `__G.net` API
//! (`tools/net-test.mjs` L73-87) and the `?skipTitle&autopilot` query seams;
//! the Rust port has no JS API surface and its Online UI's Create/Start
//! buttons are mouse-only (`ui/mod.rs` L408/L752/L769 — the keyboard covers
//! hub code entry, Enter-join and ready only). This module adds the
//! equivalent *query-gated* test seam so a headless browser client can run a
//! full room flow unattended:
//!
//! ```text
//! ?net=create        auto EnterHub + SessionCmd::Create (≡ net-test L73),
//!                    then logs `[nettest] code <CODE>` once `sess.code()`
//!                    settles (the script feeds it to the guest).
//! ?netjoin=CODE      auto EnterHub + Join with CODE (≡ net-test L77).
//! ?netready          first `SessionState::Lobby` as a guest ⇒ ToggleReady
//!                    (≡ net-test L80; the host never readies — its
//!                    ToggleReady is `tryStart`, online.rs L235-236).
//! ?netdur=<N>        host on the lobby ⇒ SetDuration(N) once (the existing
//!                    clamped `hostSet` path, ≡ the JS UI's duration buttons;
//!                    declared deviation from the design note: no direct
//!                    `lobby.duration` write bypassing the ≥60 clamp).
//! ?netstart          host with `can_start()` ⇒ SessionCmd::Start once
//!                    (≡ net-test L81-87).
//! ?autopilot         the local human slot gets a bot brain and the player
//!                    input overwrite is skipped (≡ match.js L112
//!                    `mine && (r.bot || o.autopilot)` + L118 no controller),
//!                    so an unattended client still moves and paints.
//! ?netprobe          every 2 s during the match, log
//!                    `[nettest] probe t=.. cov=.. rem=<idx:(x,y,z);..>` —
//!                    the smoke script's movement/coverage evidence.
//! ```
//!
//! Zero behaviour when no seam parameter is present (`cfg == None`), and the
//! seam never reads native CLI args (native has no config — the native
//! client's smoke run is a REVIEW_CHECKLIST item, tasks.md Task 26).

use bevy::prelude::*;
use inkwave_sim::net::session::SessionState;

use crate::ink_render::DemoSim;
use crate::net::OnlineSession;
use crate::net::match_link::NetMatchState;
use crate::ui::online::{SessionCmd, run_cmd};
use crate::ui::{Screen, UiState};

/// One parsed smoke config (every knob defaults off).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetTestCfg {
    pub create: bool,
    pub join: Option<String>,
    pub ready: bool,
    pub start: bool,
    pub dur: Option<f64>,
    pub autopilot: bool,
    pub probe: bool,
}

/// Parse a page query string (`?a=b&c`) into the smoke config. Pure and
/// platform-free so the unit tests run on the native host. `None` = no seam
/// parameter present (the seam stays inert). Unknown keys are ignored; a
/// malformed value for a *known* key disqualifies only that knob.
#[must_use]
pub fn parse_query(search: &str) -> Option<NetTestCfg> {
    let mut cfg = NetTestCfg::default();
    let mut seen = false;
    for kv in search.trim_start_matches('?').split('&') {
        let (k, v) = match kv.split_once('=') {
            Some(p) => p,
            None => (kv, ""),
        };
        match k {
            "net" if v == "create" => cfg.create = true,
            "netjoin" => {
                let code: String = v.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                if code.len() != 5 {
                    continue;
                }
                cfg.join = Some(code);
            }
            "netready" => cfg.ready = true,
            "netstart" => cfg.start = true,
            "netdur" => {
                let Ok(n) = v.parse::<f64>() else { continue };
                cfg.dur = Some(n);
            }
            "autopilot" => cfg.autopilot = true,
            "netprobe" => cfg.probe = true,
            _ => continue,
        }
        seen = true;
    }
    seen.then_some(cfg)
}

/// The live seam state (one-shot latches per command + the probe timer).
#[derive(Resource)]
pub struct NetTest {
    pub cfg: Option<NetTestCfg>,
    code_logged: bool,
    lobby_logged: bool,
    boot_logged: bool,
    err_logged: bool,
    hub_done: bool,
    readied: bool,
    dur_sent: bool,
    started: bool,
    probe_acc: f32,
}

impl NetTest {
    /// Read the seam config from the page location. Native builds have no
    /// seam (always `None`); wasm parses `location.search()`.
    #[must_use]
    pub fn from_location() -> Self {
        Self {
            cfg: parse_location_query(),
            code_logged: false,
            lobby_logged: false,
            boot_logged: false,
            err_logged: false,
            hub_done: false,
            readied: false,
            dur_sent: false,
            started: false,
            probe_acc: 0.0,
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_location_query() -> Option<NetTestCfg> {
    let search = web_sys::window()?.location().search().unwrap_or_default();
    parse_query(&search)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_location_query() -> Option<NetTestCfg> {
    // Native has no page URL and no CLI seam (the native client's smoke run
    // is a REVIEW_CHECKLIST item): an empty query is inert.
    parse_query("")
}

/// Evidence lines must reach the browser console: `println!` is a no-op on
/// `wasm32-unknown-unknown` (no stdout), and the CDP smoke probe reads the
/// console. Native keeps `println!` (the seam is inert there anyway).
pub fn console_log(s: &str) {
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(s));
    #[cfg(not(target_arch = "wasm32"))]
    println!("{s}");
}

/// The seam state machine, run after `online_ui_update` (the session pump)
/// so each frame sees the settled state. Every action is one-shot latched.
pub fn nettest_update(
    time: Res<Time>,
    mut st: ResMut<UiState>,
    mut os: NonSendMut<OnlineSession>,
    nms: Res<NetMatchState>,
    demo: Res<DemoSim>,
    mut nt: ResMut<NetTest>,
) {
    let Some(cfg) = nt.cfg.clone() else { return };
    // Deref the `NonSendMut` once so `run_cmd` can take the disjoint
    // `sess`/`flags` borrows (ui/mod.rs L1002 pattern).
    let os = &mut *os;
    if !nt.boot_logged {
        nt.boot_logged = true;
        console_log(&format!("[nettest] boot {cfg:?} screen={:?}", st.screen));
    }
    if let Some(err) = os.sess.error()
        && !nt.err_logged
    {
        nt.err_logged = true;
        console_log(&format!("[nettest] err {err}"));
    }

    // 1. boot action: Menu ⇒ hub (the `EnterHub` arm, ui/mod.rs L1339-1341)
    // then create/join (≡ net-test.mjs L73/L77).
    if !nt.hub_done && st.screen == Screen::Menu {
        nt.hub_done = true;
        st.net_mode = true;
        st.screen = Screen::OnlineHub;
        if cfg.create {
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Create);
        } else if let Some(code) = &cfg.join {
            os.flags.code_draft.clone_from(code);
            os.flags.caret = 5;
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Join);
        }
    }

    // 2. the minted room code is the script's join input (≡ net-test L73's
    // `create()` return value, which the harness reads out of the page).
    if cfg.create
        && !nt.code_logged
        && let Some(code) = os.sess.code()
    {
        nt.code_logged = true;
        console_log(&format!("[nettest] code {code}"));
    }

    // 2b. the lobby view is the welcome/join evidence: reaching `Lobby` means
    // this client got the relay's `welcome` (≡ transport.js L43 `done`) and
    // the roster broadcast; the member count proves the cross-client join
    // landed (the smoke script pairs this with the JS lobby's player list).
    if !nt.lobby_logged && os.sess.state() == SessionState::Lobby {
        nt.lobby_logged = true;
        console_log(&format!(
            "[nettest] lobby {} members {}",
            if os.sess.is_host() { "host" } else { "guest" },
            os.sess.member_ids().len()
        ));
    }

    // 3. guest ready once the lobby settles (≡ net-test L80).
    if cfg.ready && !nt.readied && os.sess.state() == SessionState::Lobby && !os.sess.is_host() {
        nt.readied = true;
        run_cmd(&mut os.sess, &mut os.flags, SessionCmd::ToggleReady);
    }

    // 4. host duration patch on the lobby (the clamped `hostSet` path).
    if let Some(d) = cfg.dur
        && !nt.dur_sent
        && os.sess.state() == SessionState::Lobby
        && os.sess.is_host()
    {
        nt.dur_sent = true;
        run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetDuration(d));
    }

    // 5. host start when the room allows it (≡ net-test L81-87).
    if cfg.start
        && !nt.started
        && os.sess.state() == SessionState::Lobby
        && os.sess.is_host()
        && os.sess.can_start()
    {
        nt.started = true;
        run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Start);
    }

    // 6. probe lines during the match — the movement/coverage evidence the
    // smoke script asserts on. Remote slots per the replication mask
    // (`remote_mask`, replication.rs L331-333); coverage ≡ `P.coverage()`
    // (net-test.mjs L144 / match_e2e.rs L141-143).
    if cfg.probe && st.screen == Screen::Hud {
        nt.probe_acc += time.delta_secs();
        if nt.probe_acc >= 2.0 {
            nt.probe_acc = 0.0;
            let cov = demo.m.paint.coverage();
            let mut line = format!(
                "[nettest] probe t={:.1} cov={:.4}/{:.4} rem=",
                demo.m.time, cov[0], cov[1]
            );
            if let Some(rep) = &nms.rep {
                let mask = rep.remote_mask();
                for (j, a) in demo.m.actors.iter().enumerate() {
                    if mask.get(j).copied().unwrap_or(false) {
                        line.push_str(&format!(
                            "{j}:({:.2},{:.2},{:.2});",
                            a.pos.x, a.pos.y, a.pos.z
                        ));
                    }
                }
            }
            console_log(&line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr26_parse_no_seam_params_is_none() {
        assert_eq!(parse_query(""), None);
        assert_eq!(parse_query("?relay=ws://x&map=sunset"), None);
        // a known key with a malformed value does not arm the seam.
        assert_eq!(parse_query("?netdur=abc"), None);
        assert_eq!(parse_query("?netjoin=toolong"), None);
    }

    #[test]
    fn tr26_parse_full_host_config() {
        let c =
            parse_query("?relay=w&net=create&netdur=60&netstart&autopilot&netprobe").expect("cfg");
        assert!(c.create && c.start && c.autopilot && c.probe);
        assert_eq!(c.dur, Some(60.0));
        assert_eq!(c.join, None);
        assert!(!c.ready);
    }

    #[test]
    fn tr26_parse_join_keeps_case_and_shape() {
        let c = parse_query("?netjoin=aB2-3x&netready&netprobe").expect("cfg");
        // the relay mints the exact code; the seam must not re-case it
        // (the `[nettest] code` line feeds this path verbatim).
        assert_eq!(c.join.as_deref(), Some("aB23x"));
        assert!(c.ready && c.probe);
        assert!(!c.create);
    }

    #[test]
    fn tr26_parse_only_net_create_counts() {
        assert_eq!(parse_query("?net=join"), None);
        assert_eq!(
            parse_query("?net=create"),
            Some(NetTestCfg {
                create: true,
                ..Default::default()
            })
        );
    }
}
