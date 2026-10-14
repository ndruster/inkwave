//! Task 24 — the pure online-lobby mapping layer (TR-24.1).
//!
//! No Bevy, no GPU, no clock: [`run_cmd`] turns a UI [`SessionCmd`] into
//! [`NetSession`] calls, [`project`] turns the session facts into a flat
//! [`OnlineView`] the Bevy screen tree renders verbatim. Every string and
//! gate mirrors `src/ui/menus.js` — `_scr_online` (L2401-2772), `_scr_lobby`
//! (L2774-3600) and the app-level `_onNetState` / `_roomGone` (L2359-2379) —
//! with the JS line numbers quoted at each rule.
//!
//! Declared deviations (Task 24 plan):
//! - ⑥ the name input is omitted: the session Profile stays at its default
//!   `'Player'` (JS `this._profile().name` with the same empty-name fallback,
//!   session.js `_profile()` L67);
//! - ⑦ leave confirmation is a two-step banner instead of the JS three-text
//!   modal (L3026-3039) — the same texts, shown inline;
//! - ⑧ the gamepad code-entry mode (`st.input === 'pad'`, the ghost letter
//!   at L2516 and the `↑↓ pick a letter · A to confirm` hint at L2526) is
//!   omitted: the Rust hub has no gamepad focus path in M2, so the keyboard
//!   hint branch of L2526 is the only one projected;
//! - the hub `mode` state machine (idle/entry/connecting/error, L2404) lives
//!   in the Bevy screen layer; this module exposes the data it renders.

use inkwave_sim::net::session::{Lobby, MeChange, NetSession, SessionState, SettingsPatch};
use inkwave_sim::net::transport::Transport;

/// Re-exported so the Bevy screen tree can build team buttons without naming
/// the sim crate directly.
pub use inkwave_sim::net::session::TeamSel;

/// Room-code alphabet (`CODE_ABC`, menus.js L37; session.js L14/L74 mints
/// codes from the same set: no `O`/`0`, no `I`/`1`, no `W`/`A`/`S`/`D`).
pub const CODE_ABC: &[u8; 28] = b"BCEFGHJKLMNPQRTUVXYZ23456789";

/// `JOIN_ERR` (menus.js L49-54): the six known join failures as
/// (title, text) pairs.
#[must_use]
pub fn error_copy(msg: &str) -> (&'static str, &'static str) {
    match msg {
        "Room not found" => (
            "ROOM NOT FOUND",
            "No room uses that code. Double-check it with your friend — rooms close when everyone leaves.",
        ),
        "Room is full" => (
            "ROOM IS FULL",
            "All 8 spots are taken. Ask the host to make space, or open a room of your own.",
        ),
        "Match in progress" => (
            "MATCH IN PROGRESS",
            "They are mid-match right now. Try again in a few minutes — the room reopens after the results.",
        ),
        "Could not connect" => (
            "CAN\u{2019}T CONNECT",
            "The INKWAVE servers didn\u{2019}t answer. Check your connection, then try again.",
        ),
        "Room code taken" => (
            "TRY AGAIN",
            "That room code was just taken. Give it another go.",
        ),
        "Lost connection to the room" => (
            "CONNECTION LOST",
            "The link to the room dropped. Check your connection and join again.",
        ),
        _ => ("COULDN\u{2019}T JOIN", "Something went wrong. Try again."),
    }
}

/// `showError` fallback text (menus.js L2608): the raw message when it is
/// not in `JOIN_ERR`, the `text` field when it is.
#[must_use]
pub fn error_detail(msg: &str) -> String {
    if matches!(
        msg,
        "Room not found"
            | "Room is full"
            | "Match in progress"
            | "Could not connect"
            | "Room code taken"
            | "Lost connection to the room"
    ) {
        error_copy(msg).1.to_string()
    } else if msg.is_empty() {
        "Something went wrong. Try again.".to_string()
    } else {
        msg.to_string()
    }
}

/// `_roomGone` toast (menus.js L2378): `Title-case — first sentence.` for a
/// known error, the raw message otherwise.
#[must_use]
pub fn room_gone_toast(msg: &str) -> String {
    let (title, text) = error_copy(msg);
    if title == "COULDN\u{2019}T JOIN" {
        // unknown error: the JS `: msg` arm (L2378 falls through to the raw
        // message only when JOIN_ERR has no entry)
        return msg.to_string();
    }
    let mut t = title.to_lowercase();
    t.replace_range(0..1, &title[0..1].to_uppercase());
    let first = text.split('.').next().unwrap_or(text);
    format!("{t} — {first}.")
}

/// The hub code-entry hint for an invalid character (`type()`, menus.js
/// L2544): the four branches in the JS test order.
#[must_use]
pub fn code_char_feedback(c: char) -> &'static str {
    if c == 'O' || c == '0' {
        "No O or 0 in room codes — try the letter next to it"
    } else if c == 'I' || c == '1' {
        "No I or 1 in room codes"
    } else if matches!(c, 'W' | 'A' | 'S' | 'D') {
        "Room codes never use W, A, S or D"
    } else {
        "Letters and numbers only"
    }
}

/// `type()` (menus.js L2539-2555) on the draft: a valid `CODE_ABC` character
/// lands at the caret and advances it (caret clamps at 4, L2552); an invalid
/// one returns its hint instead. `draft` is the fixed 5-cell string with
/// spaces for empty cells, `caret` the JS `st.caret`.
pub fn code_type(c: char, draft: &mut String, caret: &mut usize) -> Option<&'static str> {
    if !CODE_ABC.contains(&(c as u8)) {
        return Some(code_char_feedback(c));
    }
    let i = (*caret).min(4);
    let mut cells: Vec<char> = draft.chars().collect();
    cells.resize(5, ' ');
    cells[i] = c;
    *draft = cells.into_iter().collect();
    if *caret < 4 {
        *caret += 1;
    }
    None
}

/// `backspace()` (menus.js L2556-2564): clear the caret cell, stepping left
/// first when it is already empty. Returns whether a cell was cleared (JS
/// plays the pop vs the error sfx).
pub fn code_backspace(draft: &mut String, caret: &mut usize) -> bool {
    let i = (*caret).min(4);
    let mut cells: Vec<char> = draft.chars().collect();
    cells.resize(5, ' ');
    if cells[i] == ' ' && i > 0 {
        *caret = i - 1;
    }
    let had = cells[*caret] != ' ';
    cells[*caret] = ' ';
    *draft = cells.into_iter().collect();
    had
}

/// `full()` (menus.js L2509): five characters typed.
#[must_use]
pub fn code_full(draft: &str) -> bool {
    draft.len() == 5 && !draft.contains(' ')
}

/// `firstEmpty()` (menus.js L2508): the first empty cell index, clamped to 4.
#[must_use]
pub fn first_empty(draft: &str) -> usize {
    draft.chars().position(|c| c == ' ').unwrap_or(4)
}

/// `type()` L2554's arming condition for the 320 ms auto-join (`st.autoT`):
/// the draft is full, the caret sits at the last cell, and the character
/// just typed is the one in that cell (i.e. it landed there — re-typing the
/// last cell re-arms, moving the caret left without filling does not).
#[must_use]
pub fn auto_join_armed(draft: &str, caret: usize, c: char) -> bool {
    code_full(draft) && caret == 4 && draft.chars().nth(4) == Some(c)
}

/// `listNames` (menus.js L3073): `A & B` for up to two names, else the first
/// two plus `+N`.
#[must_use]
pub fn list_names(names: &[&str]) -> String {
    if names.len() <= 2 {
        names.join(" & ")
    } else {
        format!("{}, {} +{}", names[0], names[1], names.len() - 2)
    }
}

/// `tryStart` toast (menus.js L3062-3066): the start block wins, else the
/// waiting list, else the flat "Not ready to start yet".
#[must_use]
pub fn waiting_banner(block: Option<&str>, waiting: &[&str]) -> String {
    if let Some(b) = block {
        return b.to_string();
    }
    if waiting.is_empty() {
        "Not ready to start yet".to_string()
    } else {
        format!("Waiting for {} to ready up", list_names(waiting))
    }
}

/// `askLeave` texts (menus.js L3028-3033, deviation ⑦: inline banner).
#[must_use]
pub fn leave_text(is_host: bool, others: usize, first_other: Option<&str>, code: &str) -> String {
    let heir = if is_host && others > 0 {
        first_other
    } else {
        None
    };
    match heir {
        Some(h) => format!(
            "You\u{2019}re the host — {h} takes over the room. You can come back with the code {code}."
        ),
        None if others > 0 => {
            format!("You can rejoin any time with the code {code} while the room is open.")
        }
        None => "You\u{2019}re the last one here — the room closes when you leave.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// One online UI intent, dispatched by the Bevy screen layer and applied by
/// [`run_cmd`] (the JS click/accept handlers).
#[derive(Debug, Clone, PartialEq)]
pub enum SessionCmd {
    /// `doCreate` (menus.js L2657-2684).
    Create,
    /// `doJoin` (L2617-2647) with the current draft.
    Join,
    /// `cancelConnect` (L2648-2656).
    Cancel,
    /// `leaveRoom` (L3040-3046).
    Leave,
    /// `toggleReady` (L3048-3058): host ⇒ `tryStart`.
    ToggleReady,
    /// `requestTeam` (L3075-3083).
    SetTeam(TeamSel),
    /// `hostSet({duration})` (L2841/L2944-2950).
    SetDuration(f64),
    /// `hostSet({bots})` (L2848).
    SetBots(bool),
    /// `hostSet({difficulty})` (L2850).
    SetDifficulty(String),
    /// `tryStart` (L3060-3072).
    Start,
}

/// UI-side mutable flags [`run_cmd`] owns (JS screen-local `st`/`S`): kept
/// beside the session so the pure layer stays testable without Bevy.
#[derive(Debug, Default)]
pub struct OnlineFlags {
    /// `st.busy` (L2404): a connect is in flight; blocks re-entry.
    pub busy: bool,
    /// Which connect owns `busy`: only a *join* turns the JOIN button into
    /// CANCEL (`st.mode === 'connecting'`, L2522/L2439); a create keeps the
    /// button as JOIN and swallows clicks (`doJoin` busy guard L2618).
    pub joining: bool,
    /// `st.code` joined with empty cells as spaces (L2404).
    pub code_draft: String,
    /// caret index into `code_draft` (`st.caret`).
    pub caret: usize,
    /// `hintEl.textContent` override (invalid char, "N more to go",
    /// "Cancelled"); `None` = the default hint.
    pub hint: Option<String>,
    /// lobby one-shot toast (waiting-for / start block).
    pub banner: Option<String>,
    /// deviation ⑦: first LEAVE press armed the confirm banner.
    pub leave_confirm: bool,
    /// `createStatus.textContent` (doCreate L2665/L2680): the create card's
    /// own status line — `'Opening a room'` while busy, the rejection text on
    /// a failed create. Never the `hintEl` line (render L2525-2526 only
    /// rewrites the hint in modes idle/entry).
    pub create_status: Option<String>,
    /// `create.classList.is-err` (L2678): the create card shows red.
    pub create_err: bool,
    /// `st.mode === 'error'` (showError L2613): the jstat join-error panel is
    /// up. Only a failed *join* sets it — a failed create keeps the mode and
    /// writes `create_status` instead (doCreate catch L2673-2682).
    pub err_panel: bool,
}

/// The create card's rejection text (doCreate catch L2680):
/// ``E ? `${E.title} — ${E.text}` : (e && e.message) || 'Couldn’t open a room'``.
#[must_use]
pub fn create_error_text(msg: &str) -> String {
    let (title, text) = error_copy(msg);
    if title == "COULDN\u{2019}T JOIN" {
        if msg.is_empty() {
            "Couldn\u{2019}t open a room".to_string()
        } else {
            msg.to_string()
        }
    } else {
        format!("{title} \u{2014} {text}")
    }
}

impl OnlineFlags {
    /// Clear the in-flight state when the session reaches a settled screen
    /// (Lobby/Starting/Match ⇒ join/create resolved; Error ⇒ rejected —
    /// JS `tok === st.token` arms, L2637/L2643/L2669/L2674). `err` is the
    /// session error when the settle lands on `SessionState::Error`.
    pub fn settle(&mut self, err: Option<&str>) {
        let was_create = self.busy && !self.joining;
        self.busy = false;
        match err {
            // doJoin catch L2642-2646 → showError L2607-2616: the jstat
            // panel, `st.caret = 4` (L2613), and hintEl untouched — it keeps
            // the "Connecting to room X…" text doJoin wrote at L2632
            // (render L2525-2526 never rewrites it in mode 'error').
            Some(_) if self.joining => {
                self.err_panel = true;
                self.caret = 4;
                let code: String = self.code_draft.chars().filter(|c| *c != ' ').collect();
                self.hint = Some(format!("Connecting to room {code}\u{2026}"));
            }
            // doCreate catch L2673-2682: the create card takes the text,
            // the mode/hint line stay as they were (no `render()` call).
            Some(msg) if was_create => {
                self.create_status = Some(create_error_text(msg));
                self.create_err = true;
            }
            _ => {
                self.hint = None;
            }
        }
        self.joining = false;
    }
}

/// Apply one command to the session, mirroring the JS handler's guards.
/// Generic over the transport so the FakeTransport integration tests drive
/// the exact same code path as the app (TR-24.1).
pub fn run_cmd<T: Transport, F: FnMut() -> T>(
    s: &mut NetSession<T, F>,
    fl: &mut OnlineFlags,
    cmd: SessionCmd,
) {
    // The lobby toast is one-shot: any new interaction clears it (JS toasts
    // expire on their own timer; the banner flag lives until the next press).
    fl.banner = None;
    match cmd {
        SessionCmd::Create => {
            if fl.busy {
                return; // `if (st.busy) return` L2658
            }
            fl.busy = true;
            fl.joining = false; // create keeps the JOIN label (L2522)
            // L2660/L2665: the create card sheds `is-err` and shows its own
            // status line (not the hint line — doCreate never touches hintEl
            // or calls render, L2657-2666).
            fl.create_err = false;
            fl.create_status = Some("Opening a room".to_string());
            s.create(None); // name ⇒ Profile default (deviation ⑥)
        }
        SessionCmd::Join => {
            if fl.busy {
                return; // L2618
            }
            let code: String = fl.code_draft.chars().filter(|c| *c != ' ').collect();
            if code.len() < 5 {
                // L2621-2625: `st.caret = firstEmpty()` then the count hint.
                fl.caret = first_empty(&fl.code_draft);
                let n = 5 - code.len();
                fl.hint = Some(format!(
                    "{n} more character{} to go",
                    if n > 1 { "s" } else { "" }
                ));
                return;
            }
            fl.busy = true;
            fl.joining = true; // `setMode('connecting')` L2633 ⇒ CANCEL label
            // L2631: the join attempt sheds the error panel.
            fl.err_panel = false;
            s.join(&code, None);
        }
        SessionCmd::Cancel => {
            // cancelConnect L2648-2656: token++, busy off, `net.leave()`;
            // L2651 clears the create card's status line.
            fl.busy = false;
            fl.joining = false;
            fl.create_status = None;
            fl.hint = Some("Cancelled".to_string());
            s.leave(false);
        }
        SessionCmd::Leave => {
            // leaveRoom L3040-3046
            fl.busy = false;
            fl.joining = false;
            fl.leave_confirm = false;
            s.leave(false);
        }
        SessionCmd::ToggleReady => {
            // toggleReady L3051: `if (!me || S.launching) return` — the
            // launch countdown owns the room once the session left Lobby.
            if s.state() != SessionState::Lobby {
                return;
            }
            // toggleReady L3048-3049: the host's READY? button is START!
            if s.is_host() {
                run_cmd(s, fl, SessionCmd::Start);
                return;
            }
            let me_ready = s.lobby().players.iter().find(|p| p.you).map(|p| p.ready);
            if let Some(r) = me_ready {
                // L3052-3053: optimistic flip sent through setMe
                s.set_me(MeChange {
                    ready: Some(!r),
                    ..Default::default()
                });
            }
        }
        SessionCmd::SetTeam(v) => {
            // requestTeam L3080: `if (!me || S.launching) return`.
            if s.state() != SessionState::Lobby {
                return;
            }
            if s.lobby().players.iter().any(|p| p.you) {
                // requestTeam L3081
                s.set_me(MeChange {
                    team: Some(v),
                    ..Default::default()
                });
            }
        }
        SessionCmd::SetDuration(d) => {
            // hostSet L2944-2946: host-only, ignored while launching
            if s.is_host() && s.state() == SessionState::Lobby {
                s.set_settings(SettingsPatch {
                    duration: Some(d),
                    ..Default::default()
                });
            }
        }
        SessionCmd::SetBots(b) => {
            if s.is_host() && s.state() == SessionState::Lobby {
                s.set_settings(SettingsPatch {
                    bots: Some(b),
                    ..Default::default()
                });
            }
        }
        SessionCmd::SetDifficulty(d) => {
            if s.is_host() && s.state() == SessionState::Lobby {
                s.set_settings(SettingsPatch {
                    difficulty: Some(d),
                    ..Default::default()
                });
            }
        }
        SessionCmd::Start => {
            // tryStart L3060-3071: host-only (`S.launching` is the
            // post-start countdown, i.e. the session having left Lobby); on
            // !canStart the toast lists the blockers (the banner is consumed
            // by `project`).
            if !s.is_host() || s.state() != SessionState::Lobby {
                return;
            }
            if !s.can_start() {
                let waiting: Vec<&str> = s
                    .lobby()
                    .players
                    .iter()
                    .filter(|p| !p.you && !p.ready && !p.host)
                    .map(|p| p.name.as_str())
                    .collect();
                fl.banner = Some(waiting_banner(s.start_block(), &waiting));
                return;
            }
            s.start();
        }
    }
}

// ---------------------------------------------------------------------------
// projection
// ---------------------------------------------------------------------------

/// Which screen the online flow is on (JS `this.current` + `_onNetState`
/// L2359-2368).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineScreen {
    /// `online` hub (menus.js `_scr_online`).
    Hub,
    /// `lobby` (`_scr_lobby`).
    Lobby,
    /// `starting`: the lobby's launch overlay (`launchLobby`, L2362).
    MatchLoading,
    /// `match`: the HUD takes the screen (the online screens are hidden).
    Match,
}

/// Everything [`project`] reads: the session snapshot plus the UI flags.
pub struct OnlineFacts<'a> {
    pub state: SessionState,
    pub flags: &'a OnlineFlags,
    pub is_host: bool,
    pub code: Option<&'a str>,
    pub lobby: &'a Lobby,
    pub can_start: bool,
    pub start_block: Option<&'static str>,
    pub error: Option<&'a str>,
}

/// One lobby nameplate row (JS `open[k]` + `sig`, menus.js L2922-2928,
/// L3231). `sig` is the change-detection key the screen layer compares
/// before rewriting the labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterRow {
    pub name: String,
    /// `teamOf(p)` (L2785): everything not team 1 displays as ALPHA.
    pub team: u8,
    pub ready: bool,
    pub host: bool,
    pub ping: i64,
    pub you: bool,
    pub sig: String,
}

/// The hub projection (`render()`, menus.js L2510-2527).
#[derive(Debug, Clone, PartialEq)]
pub struct HubView {
    /// CREATE enabled: `!st.busy` (L2658).
    pub create_enabled: bool,
    /// JOIN button label: `'CANCEL'` while connecting else `'JOIN'` (L2522).
    pub join_label: &'static str,
    /// JOIN "ready" highlight: `full() && mode !== 'connecting'` (L2520).
    pub join_ready: bool,
    /// the five code cells (empty = space).
    pub cells: [char; 5],
    /// `hintEl.textContent` (L2525-2526 + overrides).
    pub hint: String,
    /// `jstat` error panel (L2607-2616): title + text while `mode ===
    /// 'error'`.
    pub err_title: Option<String>,
    pub err_text: Option<String>,
    /// `createStatus.textContent` (L2665/L2680) + `create.is-err` (L2678):
    /// the create card's own status line, red on rejection.
    pub create_status: Option<String>,
    pub create_err: bool,
}

/// The lobby projection (`render`/`renderStatus`/`renderBar`).
#[derive(Debug, Clone, PartialEq)]
pub struct LobbyView {
    /// room code tiles: `code.padEnd(5,'•')` (L2781, up to 8).
    pub code_tiles: String,
    pub rows: Vec<RosterRow>,
    /// `statusTxt` (L3286-3293).
    pub status_txt: String,
    /// the READY button: host ⇒ `'START!'` semantics via `ready_is_start`
    /// (toggleReady L3048-3049); guest label `'READY?'`/`'READY!'` (L3354).
    pub ready_is_start: bool,
    pub ready_on: bool,
    pub ready_sub: &'static str,
    /// START enabled: `host && canStart` (renderBar L3356-3357).
    pub start_enabled: bool,
    /// `startSub` (L3361).
    pub start_sub: String,
    /// host-only settings edits (hostSet guard L2945).
    pub settings_enabled: bool,
    /// current lobby settings the settings row shows (`LEN …  BOTS …  SKILL
    /// …`, L2841/L2848/L2850).
    pub duration: i64,
    pub bots: bool,
    pub difficulty: String,
    /// `SQUIDKIDS n/8` count (L2815).
    pub count: usize,
    /// two-step leave banner (deviation ⑦) when armed.
    pub leave_text: Option<String>,
    /// lobby toast from `run_cmd` (waiting-for / block).
    pub banner: Option<String>,
}

/// The full projection the screen tree renders.
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineView {
    pub screen: OnlineScreen,
    pub hub: HubView,
    pub lobby: LobbyView,
}

/// Map the session facts onto the view. Pure: same facts ⇒ same view.
#[must_use]
pub fn project(f: &OnlineFacts<'_>) -> OnlineView {
    let screen = match f.state {
        SessionState::Offline | SessionState::Connecting | SessionState::Error => OnlineScreen::Hub,
        SessionState::Lobby => OnlineScreen::Lobby,
        SessionState::Starting => OnlineScreen::MatchLoading,
        SessionState::Match => OnlineScreen::Match,
    };

    // ---- hub (menus.js _scr_online render(), L2510-2527)
    let draft = &f.flags.code_draft;
    let mut cells = [' '; 5];
    for (i, c) in draft.chars().take(5).enumerate() {
        cells[i] = c;
    }
    let full = code_full(draft);
    let hint = if let Some(h) = &f.flags.hint {
        h.clone()
    } else if f.flags.joining {
        // `hintEl.textContent = 'Connecting to room ${code}…'` (L2632)
        format!(
            "Connecting to room {}…",
            draft.chars().filter(|c| *c != ' ').collect::<String>()
        )
    } else if draft.is_empty() || draft.chars().all(|c| c == ' ') {
        "Room codes are 5 letters & numbers".to_string()
    } else if full {
        "Press JOIN (or Enter) to hop in".to_string()
    } else {
        "Type or paste the code".to_string()
    };
    // The jstat panel only shows for a *join* rejection (showError L2607:
    // `mode = 'error'`); a create failure lives on the create card instead
    // (doCreate catch L2673-2682).
    let (err_title, err_text) = match (f.state, f.error) {
        (SessionState::Error, Some(e)) if f.flags.err_panel => {
            let (t, _) = error_copy(e);
            (Some(t.to_string()), Some(error_detail(e)))
        }
        _ => (None, None),
    };
    let hub = HubView {
        create_enabled: !f.flags.busy,
        // `st.mode === 'connecting'` is exactly the in-flight join (L2522).
        join_label: if f.flags.joining { "CANCEL" } else { "JOIN" },
        join_ready: full && !f.flags.joining,
        cells,
        hint,
        err_title,
        err_text,
        create_status: f.flags.create_status.clone(),
        create_err: f.flags.create_err,
    };

    // ---- lobby (menus.js _scr_lobby)
    let lob = f.lobby;
    let players = &lob.players;
    let me = players.iter().find(|p| p.you);
    // room code tiles L2781: uppercase, padded with •, ≤8 (the askLeave
    // texts interpolate this same padded `code`, L3032-3033)
    let code_big = f.code.unwrap_or("").to_uppercase();
    let mut tiles = code_big.clone();
    while tiles.chars().count() < 5 {
        tiles.push('•');
    }
    // roster rows (L3231 sig: name|host|ready|weapon|team|you — weapon kept
    // out of the row data, it only feeds the sig)
    let rows: Vec<RosterRow> = players
        .iter()
        .map(|p| RosterRow {
            name: p.name.clone(),
            team: if p.team == TeamSel::One { 1 } else { 0 },
            ready: p.ready,
            host: p.host,
            ping: p.ping,
            you: p.you,
            sig: format!(
                "{}|{}|{}|{}|{}|{}",
                p.name,
                p.host,
                p.ready,
                p.weapon,
                p.team == TeamSel::One,
                p.you
            ),
        })
        .collect();
    // statusTxt (L3286-3292): waiting = !ready && !host
    let waiting: Vec<&str> = players
        .iter()
        .filter(|p| !p.ready && !p.host)
        .map(|p| p.name.as_str())
        .collect();
    let host_p = players.iter().find(|p| p.host);
    let status_txt = if players.len() >= lob.max_players {
        if waiting.is_empty() {
            "Room full · everyone\u{2019}s ready!".to_string()
        } else {
            format!("Room full · {} not ready", waiting.len())
        }
    } else if players.len() <= 1 {
        "Share the code to fill the room".to_string()
    } else if waiting.is_empty() {
        if f.is_host {
            "Everyone\u{2019}s ready — start when you like!".to_string()
        } else {
            format!(
                "Everyone\u{2019}s ready — waiting for {}",
                host_p.map(|p| p.name.as_str()).unwrap_or("the host")
            )
        }
    } else {
        format!(
            "{} of {} ready",
            players.len() - waiting.len(),
            players.len()
        )
    };
    // renderBar (L3352-3361)
    let ready_on = me.map(|p| p.ready).unwrap_or(false);
    let ok = f.is_host && f.can_start;
    let start_waiting: Vec<&str> = players
        .iter()
        .filter(|p| !p.you && !p.ready && !p.host)
        .map(|p| p.name.as_str())
        .collect();
    let humans = players.len();
    let start_sub = if ok {
        if humans <= 1 && lob.bots {
            "Just you and the bots".to_string()
        } else if humans <= 1 {
            "Nobody to play against yet!".to_string()
        } else {
            "Everyone\u{2019}s ready — let\u{2019}s ink!".to_string()
        }
    } else if let Some(b) = f.start_block {
        b.to_string()
    } else if !start_waiting.is_empty() {
        format!("Waiting for {}", list_names(&start_waiting))
    } else {
        "Getting ready…".to_string()
    };
    let leave_text = if f.flags.leave_confirm {
        let others: Vec<&str> = players
            .iter()
            .filter(|p| !p.you)
            .map(|p| p.name.as_str())
            .collect();
        Some(leave_text(
            f.is_host,
            others.len(),
            others.first().copied(),
            &tiles.chars().take(8).collect::<String>(),
        ))
    } else {
        None
    };
    let lobby = LobbyView {
        code_tiles: tiles.chars().take(8).collect(),
        rows,
        status_txt,
        ready_is_start: f.is_host,
        ready_on,
        ready_sub: if ready_on {
            "Press again to cancel"
        } else {
            "Let the host know you\u{2019}re set"
        },
        start_enabled: ok,
        start_sub,
        settings_enabled: f.is_host,
        duration: lob.duration,
        bots: lob.bots,
        difficulty: lob.difficulty.clone(),
        count: humans,
        leave_text,
        banner: f.flags.banner.clone(),
    };

    OnlineView { screen, hub, lobby }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwave_sim::net::session::{LobbyPlayer, Profile};
    use inkwave_sim::net::transport::{FakeRelay, FakeTransport};

    type Sess = NetSession<FakeTransport, Box<dyn FnMut() -> FakeTransport>>;

    /// session.rs test helper pattern (L1951-1960): one endpoint on a shared
    /// [`FakeRelay`] with a deterministic code/roster RNG seed.
    fn sess(relay: &FakeRelay, seed: u64) -> Sess {
        let r = relay.clone();
        NetSession::new(
            Profile::default(),
            seed,
            Box::new(move || r.transport(None, seed)),
        )
    }

    /// Move the shared virtual clock and tick one session (session.rs L1964).
    fn step(s: &mut Sess, dt_ms: f64) {
        if let Some(tr) = s.transport_mut() {
            tr.advance(dt_ms);
        }
        s.update(dt_ms / 1000.0);
    }

    /// Drive two sessions sharing one relay; the clock moves exactly once per
    /// tick (session.rs L1973-1984).
    fn step2(a: &mut Sess, b: &mut Sess, dt_ms: f64) {
        let mut moved = false;
        if let Some(tr) = a.transport_mut() {
            tr.advance(dt_ms);
            moved = true;
        }
        if let Some(tr) = b.transport_mut() {
            tr.advance(if moved { 0.0 } else { dt_ms });
        }
        a.update(dt_ms / 1000.0);
        b.update(dt_ms / 1000.0);
    }

    fn lp(id: &str, name: &str, host: bool, you: bool, ready: bool, team: TeamSel) -> LobbyPlayer {
        LobbyPlayer {
            id: id.into(),
            name: name.into(),
            team,
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
            ready,
            ping: 42,
            host,
            you,
        }
    }

    fn facts<'a>(
        state: SessionState,
        flags: &'a OnlineFlags,
        lobby: &'a Lobby,
        is_host: bool,
        code: Option<&'a str>,
        can_start: bool,
        error: Option<&'a str>,
    ) -> OnlineFacts<'a> {
        OnlineFacts {
            state,
            flags,
            is_host,
            code,
            lobby,
            can_start,
            start_block: None,
            error,
        }
    }

    // -- TR-24.1: error copy mirrors JOIN_ERR (menus.js L49-54) --------------

    #[test]
    fn tr24_error_copy_matches_errmap() {
        // The six known join failures, exact titles (menus.js L49-54).
        assert_eq!(error_copy("Room not found").0, "ROOM NOT FOUND");
        assert_eq!(error_copy("Room is full").0, "ROOM IS FULL");
        assert_eq!(error_copy("Match in progress").0, "MATCH IN PROGRESS");
        assert_eq!(error_copy("Could not connect").0, "CAN\u{2019}T CONNECT");
        assert_eq!(error_copy("Room code taken").0, "TRY AGAIN");
        assert_eq!(
            error_copy("Lost connection to the room").0,
            "CONNECTION LOST"
        );
        // The fallback pair (L2608 `else`).
        assert_eq!(
            error_copy("anything else"),
            ("COULDN\u{2019}T JOIN", "Something went wrong. Try again.")
        );
        // showError: known ⇒ its own text, empty ⇒ fallback, unknown ⇒ raw.
        assert_eq!(error_detail("Room is full"), error_copy("Room is full").1);
        assert_eq!(error_detail(""), "Something went wrong. Try again.");
        assert_eq!(error_detail("weird"), "weird");
        // _roomGone toast (L2378): `Title-case — first sentence.` / raw.
        assert_eq!(
            room_gone_toast("Room not found"),
            "Room not found — No room uses that code."
        );
        assert_eq!(
            room_gone_toast("Room is full"),
            "Room is full — All 8 spots are taken."
        );
        assert_eq!(room_gone_toast("custom text"), "custom text");
    }

    // -- TR-24.1: code entry (menus.js L2539-2564) ----------------------------

    #[test]
    fn tr24_code_char_feedback() {
        // The four branches in the JS test order (type(), L2544).
        for c in ['O', '0'] {
            assert_eq!(
                code_char_feedback(c),
                "No O or 0 in room codes — try the letter next to it"
            );
        }
        for c in ['I', '1'] {
            assert_eq!(code_char_feedback(c), "No I or 1 in room codes");
        }
        for c in ['W', 'A', 'S', 'D'] {
            assert_eq!(code_char_feedback(c), "Room codes never use W, A, S or D");
        }
        assert_eq!(code_char_feedback('%'), "Letters and numbers only");
        assert_eq!(code_char_feedback('a'), "Letters and numbers only");

        // type(): a valid char lands at the caret and advances it (L2552).
        let mut draft = "     ".to_string();
        let mut caret = 0usize;
        assert_eq!(
            code_type('O', &mut draft, &mut caret),
            Some(code_char_feedback('O'))
        );
        assert_eq!(draft, "     ", "invalid char never writes a cell");
        assert!(code_type('B', &mut draft, &mut caret).is_none());
        assert_eq!(draft, "B    ");
        assert_eq!(caret, 1);
        // Caret clamps at 4: typing past the last cell overwrites it (L2552).
        caret = 4;
        assert!(code_type('C', &mut draft, &mut caret).is_none());
        assert_eq!(draft, "B   C");
        assert_eq!(caret, 4, "caret stops at the last cell");
        assert!(!code_full(&draft));
        assert!(code_full("BCEFG"));

        // backspace(): empty cell steps left first, then clears (L2556-2564).
        let mut draft = "BCE  ".to_string();
        let mut caret = 2;
        assert!(code_backspace(&mut draft, &mut caret), "E cleared");
        assert_eq!(draft, "BC   ");
        assert_eq!(caret, 2);
        // Caret on an empty cell: step left once, then clear that cell.
        caret = 2;
        assert!(
            code_backspace(&mut draft, &mut caret),
            "C cleared after step-left"
        );
        assert_eq!(caret, 1);
        assert_eq!(draft, "B    ");
        // Stepping left onto another empty cell: nothing cleared (error sfx).
        caret = 4;
        assert!(
            !code_backspace(&mut draft, &mut caret),
            "empty→empty is the error-sfx arm"
        );
        assert_eq!(caret, 3);
        assert_eq!(draft, "B    ");
        // All empty: nothing was cleared (JS plays the error sfx).
        let mut draft = "     ".to_string();
        let mut caret = 0;
        assert!(!code_backspace(&mut draft, &mut caret));
    }

    // -- TR-24.1: name lists / banners / leave texts --------------------------

    #[test]
    fn tr24_list_names_and_waiting_banner() {
        // listNames (L3073): ≤2 join " & ", else first two + "+N".
        assert_eq!(list_names(&[]), "");
        assert_eq!(list_names(&["A"]), "A");
        assert_eq!(list_names(&["A", "B"]), "A & B");
        assert_eq!(list_names(&["A", "B", "C", "D"]), "A, B +2");
        // tryStart toast (L3062-3066): block wins, then the list, then flat.
        assert_eq!(
            waiting_banner(Some("Needs 2+ players"), &["X"]),
            "Needs 2+ players"
        );
        assert_eq!(waiting_banner(None, &[]), "Not ready to start yet");
        assert_eq!(
            waiting_banner(None, &["Zoe"]),
            "Waiting for Zoe to ready up"
        );
        assert_eq!(
            waiting_banner(None, &["Zoe", "Kai", "Moe"]),
            "Waiting for Zoe, Kai +1 to ready up"
        );
    }

    #[test]
    fn tr24_leave_confirm_two_step() {
        // askLeave texts (L3028-3033), shown inline as the two-step banner.
        assert_eq!(
            leave_text(true, 1, Some("Zoe"), "BCEFG"),
            "You\u{2019}re the host — Zoe takes over the room. \
             You can come back with the code BCEFG."
        );
        assert_eq!(
            leave_text(false, 1, Some("Zoe"), "BCEFG"),
            "You can rejoin any time with the code BCEFG while the room is open."
        );
        assert_eq!(
            leave_text(true, 0, None, "BCEFG"),
            "You\u{2019}re the last one here — the room closes when you leave."
        );
    }

    // -- TR-24.1: hub projection ---------------------------------------------

    #[test]
    fn tr24_project_hub_gating() {
        let lobby = Lobby::default();
        // Idle: CREATE live, JOIN label, default hint, no error panel.
        let flags = OnlineFlags::default();
        let v = project(&facts(
            SessionState::Offline,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert_eq!(v.screen, OnlineScreen::Hub);
        assert!(v.hub.create_enabled);
        assert_eq!(v.hub.join_label, "JOIN");
        assert!(!v.hub.join_ready);
        assert_eq!(v.hub.hint, "Room codes are 5 letters & numbers");
        assert!(v.hub.err_title.is_none() && v.hub.err_text.is_none());

        // Create in flight: CREATE disabled, label stays JOIN (L2522); the
        // "Opening a room" status lives on the create card (L2665), the hint
        // line is untouched by doCreate (render L2525-26 keeps the default).
        let flags = OnlineFlags {
            busy: true,
            create_status: Some("Opening a room".to_string()),
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Connecting,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert!(!v.hub.create_enabled);
        assert_eq!(v.hub.join_label, "JOIN");
        assert_eq!(v.hub.hint, "Room codes are 5 letters & numbers");
        assert_eq!(v.hub.create_status.as_deref(), Some("Opening a room"));
        assert!(!v.hub.create_err);

        // Join in flight: CANCEL label + the connecting hint (L2632).
        let flags = OnlineFlags {
            busy: true,
            joining: true,
            code_draft: "BCEFG".into(),
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Connecting,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert_eq!(v.hub.join_label, "CANCEL");
        assert!(!v.hub.join_ready, "full() && mode !== connecting (L2520)");
        assert_eq!(v.hub.hint, "Connecting to room BCEFG…");

        // Full draft, idle: the press-join hint + join_ready; cells split.
        let flags = OnlineFlags {
            code_draft: "BCE  ".into(),
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Offline,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert_eq!(v.hub.cells, ['B', 'C', 'E', ' ', ' ']);
        assert_eq!(v.hub.hint, "Type or paste the code");
        let flags = OnlineFlags {
            code_draft: "BCEFG".into(),
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Offline,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert!(v.hub.join_ready);
        assert_eq!(v.hub.hint, "Press JOIN (or Enter) to hop in");

        // A hint override wins over every default (invalid char, Cancelled).
        let flags = OnlineFlags {
            hint: Some("Cancelled".into()),
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Offline,
            &flags,
            &lobby,
            false,
            None,
            false,
            None,
        ));
        assert_eq!(v.hub.hint, "Cancelled");

        // Error state with the jstat panel armed by a *join* rejection
        // (showError L2607-2616: `mode = 'error'` + jstat `is-on`).
        let flags = OnlineFlags {
            err_panel: true,
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Error,
            &flags,
            &lobby,
            false,
            None,
            false,
            Some("Room is full"),
        ));
        assert_eq!(v.hub.err_title.as_deref(), Some("ROOM IS FULL"));
        assert_eq!(
            v.hub.err_text.as_deref(),
            Some(error_copy("Room is full").1)
        );
        // The same Error without the panel (a create failure routes to the
        // create card instead, doCreate catch L2673-2682) shows no jstat.
        let flags = OnlineFlags {
            create_status: Some("ROOM IS FULL — All 8 spots are taken.".into()),
            create_err: true,
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Error,
            &flags,
            &lobby,
            false,
            None,
            false,
            Some("Room is full"),
        ));
        assert!(v.hub.err_title.is_none() && v.hub.err_text.is_none());
        assert!(v.hub.create_err);
        assert_eq!(
            v.hub.create_status.as_deref(),
            Some("ROOM IS FULL — All 8 spots are taken.")
        );

        // Screen mapping (L2359-2368).
        let flags = OnlineFlags::default();
        assert_eq!(
            project(&facts(
                SessionState::Lobby,
                &flags,
                &lobby,
                true,
                Some("BCEFG"),
                false,
                None
            ))
            .screen,
            OnlineScreen::Lobby
        );
        assert_eq!(
            project(&facts(
                SessionState::Starting,
                &flags,
                &lobby,
                true,
                Some("BCEFG"),
                false,
                None
            ))
            .screen,
            OnlineScreen::MatchLoading
        );
        assert_eq!(
            project(&facts(
                SessionState::Match,
                &flags,
                &lobby,
                true,
                Some("BCEFG"),
                false,
                None
            ))
            .screen,
            OnlineScreen::Match
        );
    }

    // -- TR-24.1: lobby projection -------------------------------------------

    #[test]
    fn tr24_project_start_enable_needs_all_ready() {
        // Host room, guest not ready: START disabled, waiting sub + count.
        let lobby = Lobby {
            players: vec![
                lp("AAAA", "Host", true, true, true, TeamSel::Zero),
                lp("BBBB", "Guest", false, false, false, TeamSel::One),
            ],
            ..Default::default()
        };
        let flags = OnlineFlags::default();
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &lobby,
            true,
            Some("bcefg"),
            false,
            None,
        ));
        assert!(!v.lobby.start_enabled);
        assert_eq!(v.lobby.start_sub, "Waiting for Guest");
        assert_eq!(v.lobby.status_txt, "1 of 2 ready");
        assert_eq!(v.lobby.code_tiles, "BCEFG");
        assert!(
            v.lobby.ready_is_start,
            "the host's READY? is START! (L3048)"
        );
        assert!(v.lobby.settings_enabled);
        assert_eq!(v.lobby.count, 2);
        assert_eq!(v.lobby.rows.len(), 2);
        assert_eq!(v.lobby.rows[1].team, 1);

        // All ready: enabled, both status flavors (host / guest view).
        let lobby2 = Lobby {
            players: vec![
                lp("AAAA", "Host", true, true, true, TeamSel::Zero),
                lp("BBBB", "Guest", false, false, true, TeamSel::One),
            ],
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &lobby2,
            true,
            Some("BCEFG"),
            true,
            None,
        ));
        assert!(v.lobby.start_enabled);
        assert_eq!(
            v.lobby.start_sub,
            "Everyone\u{2019}s ready — let\u{2019}s ink!"
        );
        assert_eq!(
            v.lobby.status_txt,
            "Everyone\u{2019}s ready — start when you like!"
        );
        // Guest view: same room, `you` on the guest row (`project` finds
        // `me` via `you`, JS `meP()`); all-ready ⇒ "Press again to cancel".
        let lobby3 = Lobby {
            players: vec![
                lp("AAAA", "Host", true, false, true, TeamSel::Zero),
                lp("BBBB", "Guest", false, true, true, TeamSel::One),
            ],
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &lobby3,
            false,
            Some("BCEFG"),
            false,
            None,
        ));
        assert_eq!(
            v.lobby.status_txt,
            "Everyone\u{2019}s ready — waiting for Host"
        );
        assert!(!v.lobby.ready_is_start);
        assert!(!v.lobby.settings_enabled);
        assert!(!v.lobby.start_enabled);
        assert!(v.lobby.ready_on);
        assert_eq!(v.lobby.ready_sub, "Press again to cancel");

        // Solo: the share-the-code status; bots on ⇒ "Just you and the bots"
        // (renderBar L3356-3357).
        let solo = Lobby {
            players: vec![lp("AAAA", "Host", true, true, true, TeamSel::Zero)],
            bots: true,
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &solo,
            true,
            Some("BCEFG"),
            true,
            None,
        ));
        assert_eq!(v.lobby.status_txt, "Share the code to fill the room");
        assert_eq!(v.lobby.start_sub, "Just you and the bots");
        // Bots off: the "nobody to play against" warning instead (L3358).
        let solo2 = Lobby {
            bots: false,
            ..solo.clone()
        };
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &solo2,
            true,
            Some("BCEFG"),
            true,
            None,
        ));
        assert_eq!(v.lobby.start_sub, "Nobody to play against yet!");

        // The two-step leave banner only when armed (deviation ⑦).
        let flags = OnlineFlags {
            leave_confirm: true,
            ..Default::default()
        };
        let v = project(&facts(
            SessionState::Lobby,
            &flags,
            &lobby2,
            true,
            Some("BCEFG"),
            true,
            None,
        ));
        assert_eq!(
            v.lobby.leave_text.as_deref(),
            Some(leave_text(true, 1, Some("Guest"), "BCEFG").as_str())
        );
    }

    // -- TR-24.1: run_cmd drives the real session (FakeTransport) -------------

    #[test]
    fn tr24_run_cmd_maps_to_session() {
        let relay = FakeRelay::new();
        let mut h = sess(&relay, 11);
        let mut hf = OnlineFlags::default();

        // Create: busy set, joining NOT set (the JOIN label stays, L2522),
        // the session starts connecting; a second press is swallowed (L2658).
        run_cmd(&mut h, &mut hf, SessionCmd::Create);
        assert!(hf.busy && !hf.joining);
        assert_eq!(h.state(), SessionState::Connecting);
        run_cmd(&mut h, &mut hf, SessionCmd::Create);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Lobby);
        assert!(h.is_host());
        let code = h.code().expect("code").to_string();
        hf.settle(None);

        // Join with a short draft: the "N more characters to go" hint, no
        // connect started (L2621-2625).
        let mut g = sess(&relay, 22);
        let mut gf = OnlineFlags {
            code_draft: code[..3].to_string() + "  ",
            ..Default::default()
        };
        run_cmd(&mut g, &mut gf, SessionCmd::Join);
        assert!(!gf.busy, "the short join never connects");
        assert_eq!(gf.hint.as_deref(), Some("2 more characters to go"));
        // Review m-3: `doJoin` L2621 sets `st.caret = firstEmpty()` before
        // the count hint (the draft is 3 chars + 2 empty cells).
        assert_eq!(gf.caret, 3, "caret lands on the first empty cell");
        // A join while busy is swallowed too (L2618).
        gf.busy = true;
        run_cmd(&mut g, &mut gf, SessionCmd::Join);
        assert_eq!(g.state(), SessionState::Offline);
        gf.busy = false;

        // Full code: joining + busy, then the lobby with both players.
        gf.code_draft = code.clone();
        run_cmd(&mut g, &mut gf, SessionCmd::Join);
        assert!(gf.busy && gf.joining);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Lobby
                && g.lobby().players.len() == 2
                && h.lobby().players.len() == 2
            {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Lobby);
        assert_eq!(h.lobby().players.len(), 2);
        gf.settle(None);

        // Cancel: busy cleared, "Cancelled" hint, the session left (L2648).
        let mut c = sess(&relay, 33);
        let mut cf = OnlineFlags {
            code_draft: code.clone(),
            ..Default::default()
        };
        run_cmd(&mut c, &mut cf, SessionCmd::Join);
        run_cmd(&mut c, &mut cf, SessionCmd::Cancel);
        assert!(!cf.busy && !cf.joining);
        assert_eq!(cf.hint.as_deref(), Some("Cancelled"));
        assert_eq!(c.state(), SessionState::Offline);

        // Leave: clears the confirm flag and the busy state (L3040).
        let mut lf = OnlineFlags {
            busy: true,
            leave_confirm: true,
            ..Default::default()
        };
        run_cmd(&mut g, &mut lf, SessionCmd::Leave);
        assert!(!lf.busy && !lf.leave_confirm);
    }

    #[test]
    fn tr24_start_block_banner() {
        let relay = FakeRelay::new();
        let mut h = sess(&relay, 11);
        let mut hf = OnlineFlags::default();
        run_cmd(&mut h, &mut hf, SessionCmd::Create);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        let code = h.code().expect("code").to_string();
        let mut g = sess(&relay, 22);
        let mut gf = OnlineFlags {
            code_draft: code.clone(),
            ..Default::default()
        };
        run_cmd(&mut g, &mut gf, SessionCmd::Join);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Lobby
                && g.lobby().players.len() == 2
                && h.lobby().players.len() == 2
            {
                break;
            }
        }

        // Guest START is a no-op (tryStart host guard, L3060).
        run_cmd(&mut g, &mut gf, SessionCmd::Start);
        assert_eq!(g.state(), SessionState::Lobby);
        assert!(gf.banner.is_none());

        // Host START with an unready guest: the waiting-for toast (L3062-3066),
        // and `project` surfaces it as the lobby banner.
        run_cmd(&mut h, &mut hf, SessionCmd::Start);
        assert_eq!(
            h.state(),
            SessionState::Lobby,
            "blocked start stays in lobby"
        );
        assert_eq!(
            hf.banner.as_deref(),
            Some("Waiting for Player to ready up"),
            "the guest profile defaults to 'Player' (deviation ⑥)"
        );
        let v = project(&facts(
            SessionState::Lobby,
            &hf,
            h.lobby(),
            true,
            Some(&code),
            h.can_start(),
            None,
        ));
        assert_eq!(
            v.lobby.banner.as_deref(),
            Some("Waiting for Player to ready up")
        );

        // Guest readies through ToggleReady (optimistic set_me, L3052); the
        // host's ToggleReady is START (L3048) ⇒ the session launches.
        run_cmd(&mut g, &mut gf, SessionCmd::ToggleReady);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.lobby().players.iter().any(|p| !p.you && p.ready) {
                break;
            }
        }
        assert!(h.can_start());
        run_cmd(&mut h, &mut hf, SessionCmd::ToggleReady);
        assert_eq!(h.state(), SessionState::Starting);

        // Review m-4: the launch countdown (`S.launching`, L2945/L3051/
        // L3061/L3080 — the session having left Lobby) blocks every lobby
        // command: a second Start is a no-op.
        run_cmd(&mut h, &mut hf, SessionCmd::Start);
        assert_eq!(h.state(), SessionState::Starting, "no double launch");
        run_cmd(&mut h, &mut hf, SessionCmd::SetBots(false));
        run_cmd(&mut h, &mut hf, SessionCmd::SetDuration(60.0));
        assert_eq!(h.state(), SessionState::Starting);
    }

    // -- Review fixes: settle routing, firstEmpty, auto-join arming ----------

    #[test]
    fn tr24_settle_error_routing() {
        // Join rejection: showError L2607-2616 — jstat panel, `st.caret = 4`
        // (L2613), and the "Connecting to room X…" hint stays (render
        // L2525-2526 never rewrites hintEl in mode 'error').
        let mut f = OnlineFlags {
            busy: true,
            joining: true,
            code_draft: "BCEFG".into(),
            caret: 4,
            ..Default::default()
        };
        f.settle(Some("Room is full"));
        assert!(!f.busy && !f.joining);
        assert!(f.err_panel);
        assert_eq!(f.caret, 4);
        assert_eq!(f.hint.as_deref(), Some("Connecting to room BCEFG\u{2026}"));
        // The projected hub: JOIN label back, jstat panel on (mode 'error'
        // is not 'connecting', L2520/L2522).
        let lobby = Lobby::default();
        let v = project(&facts(
            SessionState::Error,
            &f,
            &lobby,
            false,
            None,
            false,
            Some("Room is full"),
        ));
        assert_eq!(v.hub.join_label, "JOIN");
        assert!(v.hub.join_ready);
        assert_eq!(v.hub.err_title.as_deref(), Some("ROOM IS FULL"));

        // Create rejection: doCreate catch L2673-2682 — the create card
        // takes the `${title} — ${text}` copy; the hint line and jstat are
        // untouched (no `showError`, no `setMode`).
        let mut f = OnlineFlags {
            busy: true,
            create_status: Some("Opening a room".into()),
            ..Default::default()
        };
        f.settle(Some("Room is full"));
        assert!(!f.busy && !f.joining);
        assert!(!f.err_panel);
        assert!(f.create_err);
        assert_eq!(
            f.create_status.as_deref(),
            Some(format!("ROOM IS FULL \u{2014} {}", error_copy("Room is full").1).as_str())
        );
        assert!(f.hint.is_none());
        // Unknown failure: the raw message (L2680 `(e && e.message)`).
        let mut f = OnlineFlags {
            busy: true,
            ..Default::default()
        };
        f.settle(Some("Server on fire"));
        assert_eq!(f.create_status.as_deref(), Some("Server on fire"));
        // Empty message: the "Couldn't open a room" fallback (L2680).
        let mut f = OnlineFlags {
            busy: true,
            ..Default::default()
        };
        f.settle(Some(""));
        assert_eq!(
            f.create_status.as_deref(),
            Some("Couldn\u{2019}t open a room")
        );

        // Success: busy clears, the hint returns to the projected default.
        let mut f = OnlineFlags {
            busy: true,
            joining: true,
            code_draft: "BCEFG".into(),
            ..Default::default()
        };
        f.settle(None);
        assert!(!f.busy && !f.joining && !f.err_panel);
        assert!(f.hint.is_none());
    }

    #[test]
    fn tr24_first_empty_and_auto_join_armed() {
        // `firstEmpty()` L2508: first empty cell, clamped to 4.
        assert_eq!(first_empty("     "), 0);
        assert_eq!(first_empty("BC   "), 2);
        assert_eq!(first_empty("BCEFG"), 4);
        // `type()` L2554 arming: full draft + caret on the last cell + the
        // typed char is the one that landed there.
        assert!(auto_join_armed("BCEF2", 4, '2'));
        assert!(
            !auto_join_armed("BCEF2", 4, 'G'),
            "the char must be the one typed"
        );
        assert!(
            !auto_join_armed("BCEF2", 3, '2'),
            "caret must sit on cell 4"
        );
        assert!(!auto_join_armed("BCE  ", 4, 'E'), "draft not full");
        // The real path: the fifth valid char at caret 4 arms.
        let mut draft = "BCEF ".to_string();
        let mut caret = 4;
        assert!(code_type('2', &mut draft, &mut caret).is_none());
        assert!(auto_join_armed(&draft, caret, '2'));
    }
}
