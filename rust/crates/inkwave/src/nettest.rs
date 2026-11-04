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
//! ?netweapon=<id>    Task 39 §3: on the first lobby frame, latch
//!                    `SessionCmd::SetWeapon` (≡ JS `setMe({weapon})`,
//!                    session.js L200-206). Id validity is the sim's job
//!                    (`WEAPONS[x]` guard — an unknown id keeps the current
//!                    weapon), the seam never re-validates.
//! ?netsub=<id|none>  `setMe({sub})` (L202 `subOf`); the literal `none`
//!                    maps to the explicit clear `Some(None)`, aligned with
//!                    the lobby cycle UI's NONE stop.
//! ?netspecial=<id|none>  `setMe({special})` (L203 `specialOf`), same
//!                    `none` rule.
//! ?netmode=turf|zones|boss  host-only `hostSet({mode})` (session.js L244;
//!                    Task 45: `boss` passes through — the session accepts
//!                    the mode and the guest boss cfg, D-T44-8). Any other
//!                    value disqualifies the knob.
//!                    → `[nettest] loadout w=<id> sub=<id|-> sp=<id|->
//!                    mode=<turf|zones|boss>` is logged once after the latch,
//!                    reading the session's *effective* values (the lobby
//!                    `me` row + `lob.mode`), so a sim-dropped invalid id
//!                    shows the old weapon and the script can reconcile.
//!                    → `[nettest] result w=<0|1> cov=<a>/<b>` once the
//!                    local match settles (host-judged or the host's `res`
//!                    on a guest); a zones match appends
//!                    ` zone=<c0>/<c1> reason=<id|-> pen=<p0>/<p1> ot=<0|1>`
//!                    (the `ZoneOutcome` fields, counts already `ceil`'d,
//!                    penalty one decimal — the script's literal contract).
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
    /// Task 39 §3: `?netweapon=<id>` — validity is the sim's (`WEAPONS[x]`
    /// guard, session.js L201: an unknown id keeps the current weapon).
    pub weapon: Option<String>,
    /// `?netsub=<id|none>`: `Some(None)` = the literal `none` (explicit
    /// clear, ≡ the cycle UI's NONE stop); `Some(Some(id))` = equip.
    pub sub: Option<Option<String>>,
    /// `?netspecial=<id|none>`, same `none` rule.
    pub special: Option<Option<String>>,
    /// `?netmode=turf|zones|boss` (host-only; anything else disqualifies).
    pub mode: Option<String>,
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
            // Task 39 §3: loadout ids are the sim's to validate (the
            // `WEAPONS[x]` / `subOf` / `specialOf` guards, session.js
            // L200-206) — the seam only rejects empty values; the literal
            // `none` on sub/special maps to the explicit clear.
            "netweapon" => {
                if v.is_empty() {
                    continue;
                }
                cfg.weapon = Some(v.to_string());
            }
            "netsub" => {
                if v.is_empty() {
                    continue;
                }
                cfg.sub = Some(if v == "none" {
                    None
                } else {
                    Some(v.to_string())
                });
            }
            "netspecial" => {
                if v.is_empty() {
                    continue;
                }
                cfg.special = Some(if v == "none" {
                    None
                } else {
                    Some(v.to_string())
                });
            }
            "netmode" => {
                // Task 45: boss passes through too (session.js L244 `hostSet`
                // accepts the three modes; the session already lets a guest
                // join a boss room without rejecting — D-T44-8). Task 46
                // batch B wired the entity driving: the app layer now owns
                // the `BossMode` across frames (D-T45-9 resolved by batch A's
                // owned refactor — `start_net_match` constructs it, `net_tick`
                // feeds `step_masked` / `drive_bots` / `rep.update`).
                if v != "turf" && v != "zones" && v != "boss" {
                    continue;
                }
                cfg.mode = Some(v.to_string());
            }
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
    /// Task 39 §3: the one-shot loadout/mode latch + the settled-result line.
    loadout_done: bool,
    result_done: bool,
    /// Task 39 §4: sandbox join retry — under the software renderer the JS
    /// host's `welcome` can exceed the transport's 8 s handshake budget
    /// (`CONNECT_TIMEOUT_MS`, transport.rs L51 ≡ transport.js L48) while the
    /// room itself stays alive; re-issue the join a few times instead of
    /// failing the scenario. Test-seam only, no sim/JS behaviour change.
    join_attempts: u8,
    retry_acc: f32,
    /// Task 39 §4 (resilience): the symmetric *create* retry for the host
    /// tab — the first connect can blow the same 8 s budget while the wasm
    /// main thread is blocked compiling PBR shaders under SwiftShader,
    /// leaving the session in `Error` with no room minted (`settle` releases
    /// `busy`, online.rs L317-319, so a re-issued `Create` runs).
    create_attempts: u8,
    create_acc: f32,
    /// Task 46 batch B (D-T45-11): one-shot latch for the guest boss-B
    /// arrival probe line.
    boss_b_logged: bool,
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
            loadout_done: false,
            result_done: false,
            join_attempts: 0,
            retry_acc: 0.0,
            create_attempts: 0,
            create_acc: 0.0,
            boss_b_logged: false,
        }
    }
}

/// Task 46 batch B (D-T45-11): the guest boss-B arrival condition — a
/// follower boss (`sim == false`, JS `bossMode.js` L23 `sim: !match.follower`)
/// whose net buffer has at least one snapshot from the host's tick `B` field
/// (`rep.update` → `sample_boss`, replication.rs L2020-2028). Returns the
/// buffered count on first arrival. Pure so the gate is unit-testable without
/// a live session.
#[must_use]
fn boss_b_arrival(bm: Option<&inkwave_sim::boss::BossMode>, is_host: bool) -> Option<usize> {
    let bm = bm?;
    if is_host || bm.boss.sim {
        return None;
    }
    (!bm.boss.net.buf.is_empty()).then_some(bm.boss.net.buf.len())
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

    // 1a. Task 39 §4 (resilience) sandbox create retry: the host's first
    // connect timing out against the 8 s handshake budget (SwiftShader
    // shader compilation blocking the wasm main thread) lands the session
    // in `Error` before any room is minted — re-issue Create (up to 3
    // times, 5 s apart), exactly symmetric to 1b. Test-seam only.
    if cfg.create && nt.hub_done && os.sess.state() == SessionState::Error && nt.create_attempts < 3
    {
        nt.create_acc += time.delta_secs();
        if nt.create_acc >= 5.0 {
            nt.create_acc = 0.0;
            nt.create_attempts += 1;
            console_log(&format!("[nettest] create retry {}", nt.create_attempts));
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Create);
        }
    }

    // 1b. Task 39 §4 sandbox join retry: a `welcome` slower than the 8 s
    // handshake budget lands the session in `Error` while the room stays
    // alive — re-issue the join (up to 3 times, 5 s apart). Test-seam only.
    if cfg.join.is_some()
        && nt.hub_done
        && os.sess.state() == SessionState::Error
        && nt.join_attempts < 3
    {
        nt.retry_acc += time.delta_secs();
        if nt.retry_acc >= 5.0 {
            nt.retry_acc = 0.0;
            nt.join_attempts += 1;
            console_log(&format!("[nettest] join retry {}", nt.join_attempts));
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

    // 2c. Task 39 §3: latch the loadout picks + host mode once the lobby
    // lands, *before* ready / start, so the `start()` cfg roster carries
    // them (JS `setMe` L200-206 self-select; `hostSet({mode})` L244 — the
    // host-only / Lobby-state guards live in `run_cmd`). The confirmation
    // line reads the session's effective values: an id the sim dropped
    // (invalid weapon → current kept; invalid sub/special → cleared) shows
    // the settled state, which is exactly what the script reconciles on.
    if !nt.loadout_done
        && os.sess.state() == SessionState::Lobby
        && (cfg.weapon.is_some()
            || cfg.sub.is_some()
            || cfg.special.is_some()
            || cfg.mode.is_some())
    {
        nt.loadout_done = true;
        if let Some(w) = &cfg.weapon {
            run_cmd(
                &mut os.sess,
                &mut os.flags,
                SessionCmd::SetWeapon(w.clone()),
            );
        }
        if let Some(s) = cfg.sub.clone() {
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetSub(s));
        }
        if let Some(sp) = cfg.special.clone() {
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetSpecial(sp));
        }
        if let Some(m) = &cfg.mode {
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetMode(m.clone()));
        }
        let lob = os.sess.lobby();
        let me = lob.players.iter().find(|p| p.you);
        console_log(&format!(
            "[nettest] loadout w={} sub={} sp={} mode={}",
            me.map(|p| p.weapon.as_str()).unwrap_or("shooter"),
            me.and_then(|p| p.sub.as_deref()).unwrap_or("-"),
            me.and_then(|p| p.special.as_deref()).unwrap_or("-"),
            lob.mode,
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
            // Task 39 §3: `sub=` = the local deploy counters (non-bomb
            // `Match::sub_fired`, JS subs.js L71 `use`, + bomb
            // `bombs_thrown`, weapons.js L151); `sp=` = Σ `specials_used`
            // (the existing `stats.specials++`, JS actor.js L840 /
            // specials.js L254 start). Inserted between `cov=` and `rem=`
            // so the script's `rem=(.*)` capture keeps its meaning.
            let sub_n = demo.m.sub_fired + demo.m.projectiles.bombs_thrown;
            let sp_n: u32 = demo.m.actors.iter().map(|a| a.specials_used).sum();
            let mut line = format!(
                "[nettest] probe t={:.1} cov={:.4}/{:.4} sub={} sp={} rem=",
                demo.m.time, cov[0], cov[1], sub_n, sp_n
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

    // 6b. Task 46 batch B (D-T45-11): guest boss-B arrival probe. Once the
    // follower boss has buffered the host's first tick `B` snapshot
    // (`rep.update` → `sample_boss` → `boss.net.buf`, replication.rs
    // L2020-2028 ≡ JS netmatch guest `boss.net` feed), log the count once.
    // Literal contract (the smoke script greps this):
    //   `[nettest] boss B recv=<n>`
    if !nt.boss_b_logged
        && let Some(rep) = &nms.rep
        && let Some(n) = boss_b_arrival(demo.bm.lock().unwrap().as_ref(), rep.is_host())
    {
        nt.boss_b_logged = true;
        console_log(&format!("[nettest] boss B recv={n}"));
    }

    // 7. Task 39 §3: the settled-result line, once. `Match::result` is the
    // end state (the host judges locally, a guest gets the host's `{k:'res'}`
    // — replication drives the same field, netmatch.js L652-660). Literal
    // contract (the script parses this verbatim):
    //   turf:  `[nettest] result w=<0|1> cov=<a>/<b>`
    //   zones: `[nettest] result w=<0|1> cov=<a>/<b> zone=<c0>/<c1>
    //           reason=<id|-> pen=<p0>/<p1> ot=<0|1>`
    // `w` is the sim's seeded tie-break winner, never -1 (the -1 slot stays
    // reserved for a result-less End, structurally unreachable here).
    if !nt.result_done
        && let Some(res) = demo.m.result.as_ref()
    {
        nt.result_done = true;
        let mut line = format!(
            "[nettest] result w={} cov={:.4}/{:.4}",
            res.winner, res.coverage[0], res.coverage[1]
        );
        if let Some(z) = &res.zones {
            line.push_str(&format!(
                " zone={}/{} reason={} pen={:.1}/{:.1} ot={}",
                z.counts[0],
                z.counts[1],
                z.reason.as_deref().unwrap_or("-"),
                z.penalty[0],
                z.penalty[1],
                u8::from(z.overtime)
            ));
        }
        console_log(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr39_parse_loadout_knobs() {
        // Task 39 §3: the four new keys, the `none` → `Some(None)` clear
        // mapping (the cycle UI's NONE stop), and the `netmode` value gate.
        let c = parse_query(
            "?netjoin=ABCDE&netweapon=brolly&netsub=none&netspecial=stamp&netmode=zones",
        )
        .expect("join makes it Some");
        assert_eq!(c.weapon.as_deref(), Some("brolly"));
        assert_eq!(c.sub, Some(None), "`netsub=none` maps to the clear");
        assert_eq!(c.special, Some(Some("stamp".to_string())));
        assert_eq!(c.mode.as_deref(), Some("zones"));
        // turf is the other accepted value; an unknown id is NOT screened
        // here (the sim's `WEAPONS[x]` / `subOf` guards own validity).
        let c = parse_query("?net=create&netmode=turf&netweapon=weird&netsub=bomb").expect("some");
        assert_eq!(c.mode.as_deref(), Some("turf"));
        assert_eq!(c.weapon.as_deref(), Some("weird"));
        assert_eq!(c.sub, Some(Some("bomb".to_string())));
        assert_eq!(c.special, None, "absent knob stays None");
        // Task 45: `boss` passes through (the session accepts it, D-T44-8).
        // A bad `netmode` value disqualifies only that knob; empty values
        // drop their own knob; no loadout keys ⇒ all fields None and the
        // cfg still exists for other keys.
        let c = parse_query("?net=create&netmode=boss&netweapon=&netsub=").expect("some");
        assert_eq!(c.mode.as_deref(), Some("boss"));
        assert_eq!((c.weapon, c.sub), (None, None));
        assert_eq!(
            parse_query("?net=create&netmode=duel&netweapon=&netsub=")
                .map(|c| (c.mode, c.weapon, c.sub)),
            Some((None, None, None))
        );
        let c = parse_query("?net=create&netweapon=charger").expect("some");
        assert_eq!(c.weapon.as_deref(), Some("charger"));
        assert_eq!(c.sub, None);
        assert_eq!(c.special, None);
        assert_eq!(c.mode, None);
    }

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

    #[test]
    fn tr46_boss_b_arrival_gates_on_the_follower_role() {
        // Task 46 batch B (D-T45-11): only a guest-side follower boss with a
        // non-empty net buffer counts as "B arrived". The host boss
        // (`sim == true`, JS bossMode.js L23 `sim: !match.follower`) never
        // triggers the probe.
        use inkwave_sim::actor::SimWorld;
        use inkwave_sim::boss::BossMode;
        use inkwave_sim::boss::nav::BossNav;
        use inkwave_sim::collision::CollisionWorld;
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let sim = SimWorld::new(&world, &layout);
        let tb = inkwave_sim::embedded_tuning();
        let mk = |sim_boss: bool| {
            BossMode::new(
                &sim,
                BossNav::new(&sim, &tb.boss.nav),
                tb.boss.clone(),
                5.2,
                "normal",
                sim_boss,
                1,
            )
        };
        let host = mk(true);
        let follower = mk(false);
        assert!(boss_b_arrival(None, false).is_none(), "turf: no boss");
        assert!(
            boss_b_arrival(Some(&host), true).is_none(),
            "host never probes"
        );
        assert!(
            boss_b_arrival(Some(&host), false).is_none(),
            "a sim boss never follows snapshots"
        );
        assert!(
            boss_b_arrival(Some(&follower), false).is_none(),
            "empty net buffer = no arrival yet"
        );
    }
}
