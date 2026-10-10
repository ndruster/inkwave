//! Task 20 — the session state machine: rooms, the lobby and match
//! orchestration (`src/net/session.js`, contract `docs/NET.md`).
//!
//! [`NetSession`] owns one [`Transport`] and walks
//! `offline → connecting → lobby → starting → match` (plus `error`), exactly
//! like the JS `NetSession` class:
//!
//! - **Rooms** — [`NetSession::create`] mints 5-character codes from
//!   `CODE_CHARS` (session.js L15, no 0/O/1/I/W/A/S/D) and retries ≤ 4 times
//!   while the relay answers `Room code taken` (L73-79); [`NetSession::join`]
//!   uppercases/strips the code and requires ≥ 4 chars (L82-85). The relay's
//!   own refusals (`Room not found` / `Room is full` / `Match in progress` /
//!   `Could not connect`) surface verbatim as [`SessionEvent::Error`]
//!   (`_fail`, L126-131).
//! - **Lobby ownership** — the host (relay-elected oldest member) owns the
//!   lobby: guests send `{k:'me'}` to the host (`_applyMe`, L216-233), the
//!   host validates (≤ 4 a side, known weapon ids) and broadcasts the whole
//!   lobby `{k:'lobby'}` (`_wireLobby` field set, L186-189).
//!   [`fix_teams`] mirrors L173-179 line by line: honour explicit team
//!   requests while keeping ≤ 4 a side, then place everyone still on `auto`
//!   on the smaller side.
//! - **Start handshake** — [`NetSession::start`] builds the roster (nid
//!   order: team 0 humans then bots, then team 1; bots fill to 4 a side,
//!   L271-290), locks the room and broadcasts `{k:'start',…}` (L293-295);
//!   every client reports `{k:'ready'}` once its match is loaded and the host
//!   answers `{k:'go'}` — with the 12 s fallback so one slow load cannot
//!   stall everyone (L320-335), and a member that left while loading counts
//!   as ready (L324).
//! - **Relay membership** — `join`/`leave` control frames update the member
//!   map, migrate the host (the `host` field on `leave`), and the host
//!   re-broadcasts the lobby (L145-166).
//! - **Ping** — while in the lobby, every 2 s the host records its own RTT
//!   into the lobby and guests send `{k:'me',ping}` to the host (L381-390).
//!
//! # M2 scope deviations (documented, per tasks.md Notes)
//!
//! - **Turf War only.** `mode` is restricted: [`NetSession::set_settings`]
//!   refuses a non-turf mode (the host never *sends* one), and a guest that
//!   receives a `start` cfg with `mode != "turf"` fails and leaves. The JS
//!   zones/boss branches (`ZONES.duration`, `bossFallbackMap`, one-squad
//!   roster) are unreachable here.
//! - **`style`/`sub`/`special` pass through uninterpreted** — the Rust port
//!   only models the Spritzer placeholder, so `style` is an opaque [`JVal`]
//!   and, unlike JS `randomStyle()`, locally-created players and roster bots
//!   carry `style: null`. `sub`/`special` are validated against the JS id
//!   tables (`subOf`/`specialOf`, L18) but never interpreted.
//! - **`name.slice(0,16)`** counts Unicode scalars, not UTF-16 code units
//!   (same deviation as the Task 19 shim).
//! - **Bot roster shuffle**: JS uses `Array.sort(() => Math.random()-0.5)`
//!   (not a uniform shuffle); Rust uses a seeded Fisher-Yates over the same
//!   tables — Rust↔Rust same-version play only needs the *shape*, and the
//!   RNG is deterministic (NFR-M2-3).
//! - **Async launch seams.** JS `_begin` awaits `launchLobby()`/
//!   `startNetMatch()` before reporting ready. The Rust app layer (Task 24)
//!   builds the match when [`SessionEvent::Match { phase: Start }]` arrives
//!   and then calls [`NetSession::mark_loaded`], preserving the JS ordering.
//!   `match?.onMessage` / `onLeave` / `netMatchAborted` are surfaced as
//!   [`SessionEvent::GameMessage`] / [`SessionEvent::Leave`] /
//!   [`SessionEvent::Match { phase: Aborted }]` for the replication layer
//!   (Task 21) to consume.
//!
//! All timing (2 s ping, 12 s go fallback) is driven by
//! [`NetSession::update(dt_seconds)]`, which also drains the transport — so
//! the whole machine runs deterministically against
//! [`crate::net::transport::FakeTransport`] (TR-20.1/2/3).

use std::collections::{HashMap, HashSet};

use super::SessionId;
use super::transport::{Transport, TransportEvent, rand01};
use super::wire::{self, JVal};

// ---------------------------------------------------------------------------
// JS config tables mirrored here (src/config.js, read-only provenance)
// ---------------------------------------------------------------------------

/// `CODE_CHARS` (session.js L15) — room-code alphabet: no 0/O/1/I (misread)
/// and no W/A/S/D (menu cursor keys), 28 chars ≈ 17 M codes.
pub const CODE_CHARS: &[u8] = b"BCEFGHJKLMNPQRTUVXYZ23456789";

/// `TEAM = 4` — a side's size cap (session.js L16).
pub const TEAM_SIZE: usize = 4;

/// `maxPlayers: TEAM * 2` (session.js L62).
pub const MAX_PLAYERS: usize = TEAM_SIZE * 2;

/// `MATCH.defaultDuration = 180` (config.js L474) — the lobby's default
/// match length in seconds.
pub const DEFAULT_DURATION: i64 = 180;

/// `TEAM_PALETTES.length` (config.js L8-14) — the palette index bound
/// checked by `setSettings` (session.js L243).
pub const PALETTE_COUNT: usize = 5;

/// `Object.keys(WEAPONS)` (config.js L303-405 region; the 15 weapon ids) —
/// the validity table behind `WEAPONS[x] ? x : 'shooter'` (L19, L67, L169,
/// L201, L220).
pub const WEAPON_IDS: &[&str] = &[
    "shooter",
    "roller",
    "charger",
    "blaster",
    "bucket",
    "spinner",
    "twins",
    "brush",
    "brolly",
    "bow",
    "blade",
    "mitts",
    "dualies",
    "slosher",
    "splatling",
];

/// `WEAPON_ORDER` (config.js L303) — the 12 weapons bots are dealt from
/// (roster generation, session.js L277).
pub const WEAPON_ORDER: &[&str] = &[
    "shooter", "twins", "brolly", "blaster", "spinner", "charger", "bow", "roller", "brush",
    "blade", "mitts", "bucket",
];

/// `Object.keys(SUBS)` == `SUB_ORDER` (config.js L407) — the `subOf`
/// validity table (session.js L18).
pub const SUB_IDS: &[&str] = &[
    "bomb",
    "sticky",
    "burst",
    "shaker",
    "seeker",
    "waddle",
    "torpedo",
    "tracer",
    "boomerang",
    "scan",
    "curtain",
    "sprinkler",
    "mine",
    "beacon",
    "mist",
];

/// `Object.keys(SPECIALS)` == `SPECIAL_ORDER` (config.js L469) — the
/// `specialOf` validity table (session.js L18).
pub const SPECIAL_IDS: &[&str] = &[
    "slam",
    "storm",
    "barrage",
    "barrage_sticky",
    "barrage_burst",
    "barrage_seeker",
    "barrage_mist",
    "bubbler",
    "sonar",
    "strike",
    "zooka",
    "wail",
    "kraken",
    "blower",
    "jetpack",
    "stamp",
    "booyah",
    "zipcaster",
    "crab",
];

/// `BOT_NAMES` (config.js L547-550) — 20 bot display names, popped off the
/// shuffled copy during roster generation (session.js L273/L287).
pub const BOT_NAMES: &[&str] = &[
    "Squiddo", "Blotch", "Marlo", "Inky Vee", "Pip", "Coral", "Riptide", "Nori", "Suki", "Zest",
    "Kelp", "Drip", "Tako", "Sprinkle", "Bubbles", "Moxie", "Juno", "Wasabi", "Fizz", "Loop",
];

/// `MAPS` ids with their `noBots` flag (config.js L515-524). M2 is Turf
/// War only, so `noBoss`/`onlineOnly` are not modelled; `noBots` drives the
/// lobby default, `setSettings` and `startBlock` (session.js L62/L249/L270,
/// `noBotsStartBlock` config.js L539-545).
pub const MAP_IDS: &[&str] = &[
    "tidewater",
    "kelpline",
    "halyard",
    "saltpan",
    "crossmarket",
    "lockgate",
    "terraces",
    "cargo",
];

/// `mapNoBots(id)` (config.js L532): only `cargo` is humans-only.
#[must_use]
pub fn map_no_bots(id: &str) -> bool {
    id == "cargo"
}

fn map_exists(id: &str) -> bool {
    MAP_IDS.contains(&id)
}

fn weapon_ok(id: &str) -> bool {
    WEAPON_IDS.contains(&id)
}

/// `subOf` / `specialOf` (session.js L18): a known id, or `null`.
fn sub_of(id: Option<&String>) -> Option<String> {
    id.filter(|s| SUB_IDS.contains(&s.as_str())).cloned()
}

fn special_of(id: Option<&String>) -> Option<String> {
    id.filter(|s| SPECIAL_IDS.contains(&s.as_str())).cloned()
}

/// `String.prototype.slice(0, 16)` over Unicode scalars (deviation noted in
/// the module docs).
fn js_slice16(s: &str) -> String {
    s.chars().take(16).collect()
}

// ---------------------------------------------------------------------------
// State & wire types
// ---------------------------------------------------------------------------

/// `this.state` (session.js L22): the machine's six states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Offline,
    Connecting,
    Lobby,
    Starting,
    Match,
    Error,
}

/// A player's lobby team request — `'auto' | 0 | 1` (session.js L169/L206).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamSel {
    Auto,
    Zero,
    One,
}

impl TeamSel {
    fn to_jval(self) -> JVal {
        match self {
            TeamSel::Auto => JVal::Str("auto".into()),
            TeamSel::Zero => JVal::Num(0.0),
            TeamSel::One => JVal::Num(1.0),
        }
    }

    /// JS keeps a number that is not `0`/`1` verbatim (a broken state the
    /// host never emits after `_fixTeams` L176-177); the Rust wire models
    /// only `auto`/`0`/`1`, so anything else decodes to `Auto` (M2
    /// deviation, unreachable under Rust↔Rust).
    fn from_jval(v: &JVal) -> Self {
        match v {
            JVal::Num(n) if *n == 0.0 => TeamSel::Zero,
            JVal::Num(n) if *n == 1.0 => TeamSel::One,
            _ => TeamSel::Auto,
        }
    }
}

/// One lobby player — the `_newPlayer` shape (session.js L168-170) plus the
/// local-only `you`/`host` marks `_pushLobby` writes (L191-194). `host` is
/// part of the wire (`_fixTeams` L178 sets it before `_wireLobby`… actually
/// `_wireLobby` does NOT include it — the 10 mapped keys are the contract).
#[derive(Debug, Clone, PartialEq)]
pub struct LobbyPlayer {
    pub id: SessionId,
    pub name: String,
    pub team: TeamSel,
    pub weapon: String,
    pub sub: Option<String>,
    pub special: Option<String>,
    /// Opaque passthrough (deviation: locally created players get `null`).
    pub style: Option<JVal>,
    pub ready: bool,
    pub ping: i64,
    /// `p.id === hostId` (session.js L169/L178/L192).
    pub host: bool,
    /// `p.id === myId` — local view only (`_pushLobby` L192), never encoded.
    pub you: bool,
}

/// `this.lobby` — `_blankLobby` (session.js L56-63) with M2's turf-only
/// `mode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Lobby {
    pub map: String,
    pub time: String,
    pub duration: i64,
    pub bots: bool,
    pub difficulty: String,
    pub palette: usize,
    pub mode: String,
    pub players: Vec<LobbyPlayer>,
    pub max_players: usize,
}

impl Default for Lobby {
    /// `_blankLobby()` with the JS defaults (L61-62): the first MAPS entry,
    /// `day`, [`DEFAULT_DURATION`], bots on unless the stage forbids them,
    /// `normal`, palette 0, `turf`, no players.
    fn default() -> Self {
        Self {
            map: MAP_IDS[0].to_string(),
            time: "day".to_string(),
            duration: DEFAULT_DURATION,
            bots: !map_no_bots(MAP_IDS[0]),
            difficulty: "normal".to_string(),
            palette: 0,
            mode: "turf".to_string(),
            players: Vec::new(),
            max_players: MAX_PLAYERS,
        }
    }
}

/// One roster entry — the `start()` push shape (session.js L279/L287).
#[derive(Debug, Clone, PartialEq)]
pub struct RosterEntry {
    pub nid: u8,
    pub owner: SessionId,
    pub bot: bool,
    pub team: u8,
    pub slot: u8,
    pub name: String,
    pub weapon: String,
    pub sub: Option<String>,
    pub special: Option<String>,
    pub style: Option<JVal>,
}

/// The `{k:'start',…}` match config (session.js L292). `id` is the match
/// instance id (`Math.random().toString(36).slice(2,8)`) that `ready`/`go`
/// are stamped with.
#[derive(Debug, Clone, PartialEq)]
pub struct StartCfg {
    pub roster: Vec<RosterEntry>,
    pub map: String,
    pub time: String,
    pub duration: i64,
    pub difficulty: String,
    pub palette: usize,
    pub mode: String,
    pub host: SessionId,
    pub id: String,
}

/// The local player's identity sent on connect — `_profile()` (session.js
/// L65-68). The app fills this from its own state; the session sanitises it
/// exactly like JS (`name` default `'Player'`, weapon fallback `'shooter'`).
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub weapon: String,
    pub sub: Option<String>,
    pub special: Option<String>,
    pub style: Option<JVal>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Player".to_string(),
            weapon: "shooter".to_string(),
            sub: None,
            special: None,
            style: None,
        }
    }
}

impl Profile {
    /// `_profile()` L67: clamp the name, validate the weapon, `subOf`/
    /// `specialOf` the loadout, pass `style` through.
    fn sanitized(&self) -> Profile {
        Profile {
            name: js_slice16(if self.name.is_empty() {
                "Player"
            } else {
                &self.name
            }),
            weapon: if weapon_ok(&self.weapon) {
                self.weapon.clone()
            } else {
                "shooter".to_string()
            },
            sub: sub_of(self.sub.as_ref()),
            special: special_of(self.special.as_ref()),
            style: self.style.clone(),
        }
    }
}

/// `setMe(ch)`'s argument (session.js L197-214). `None` = the JS `undefined`
/// ("leave the field alone"); `Some(None)` = an explicit `null` (only
/// meaningful for `sub`/`special`, which `subOf` maps to `null` anyway).
#[derive(Debug, Clone, Default)]
pub struct MeChange {
    pub name: Option<String>,
    pub weapon: Option<String>,
    pub sub: Option<Option<String>>,
    pub special: Option<Option<String>>,
    pub style: Option<JVal>,
    pub ready: Option<bool>,
    pub team: Option<TeamSel>,
    /// Host-side bookkeeping only (`_applyMe` L225): guests send the ping
    /// via the lobby timer, never through [`NetSession::set_me`].
    pub ping: Option<i64>,
}

/// `setSettings(s)`'s argument (session.js L235-251). `mode` is validated to
/// `turf` only (M2 deviation); `None` fields are left unchanged.
#[derive(Debug, Clone, Default)]
pub struct SettingsPatch {
    pub map: Option<String>,
    pub time: Option<String>,
    pub duration: Option<f64>,
    pub bots: Option<bool>,
    pub difficulty: Option<String>,
    pub palette: Option<f64>,
    pub mode: Option<String>,
}

// ---------------------------------------------------------------------------
// Session events (the JS `_emit` subscription surface, queue-shaped)
// ---------------------------------------------------------------------------

/// One drained session event — the union of the JS `on(ev, fn)` channels
/// (`state` / `lobby` / `error` / `join` / `leave` / `host` / `emote` /
/// `match`) plus the match passthrough the replication layer (Task 21)
/// consumes. The app polls these like [`TransportEvent`]s.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// `_setState` (L50-54).
    State { state: SessionState },
    /// `_emit('lobby')` — a snapshot of the local lobby view.
    Lobby { lobby: Lobby },
    /// `_emit('error')` (L130/L140).
    Error { message: String },
    /// `_emit('join')` — a player appeared in the lobby who wasn't there
    /// before (guest side, L367).
    Join { player: LobbyPlayer },
    /// `_emit('leave')` (L163) — `player` is the lobby view if known, else
    /// the relay membership entry (L154).
    Leave { player: LobbyPlayer, reason: String },
    /// `_emit('host')` — host migration (L161).
    Host { host_id: SessionId },
    /// `_emit('emote')` (L263/L372).
    Emote { id: SessionId, name: String },
    /// `_emit('match', {phase})` (L304/L350) + the JS `netMatchAborted`
    /// seam (L141) as `Aborted`.
    Match { phase: MatchPhase },
    /// A game payload the session does not consume (ticks, hits, …) — the
    /// `default:` arm of `_message` forwarding to `match.onMessage` (L376).
    GameMessage { from: SessionId, payload: String },
}

/// The `phase` values of [`SessionEvent::Match`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    /// `{phase:'start'}` (L304) — the app builds the match, then calls
    /// [`NetSession::mark_loaded`].
    Start,
    /// The `go` handshake completed — `_launch` (L337-341).
    Go,
    /// `{phase:'end'}` — `endMatch` (L350).
    End,
    /// A connection loss while `starting`/`match` — the JS
    /// `G.game.netMatchAborted(this.error)` seam (L141).
    Aborted,
}

// ---------------------------------------------------------------------------
// Wire encoders (byte-exact against the Task 17 fixtures)
// ---------------------------------------------------------------------------

fn jstr(s: &str) -> JVal {
    JVal::Str(s.to_string())
}
fn jnum(v: i64) -> JVal {
    JVal::Num(v as f64)
}
fn jbool(v: bool) -> JVal {
    JVal::Bool(v)
}
fn jopt_str(v: &Option<String>) -> JVal {
    match v {
        Some(s) => jstr(s),
        None => JVal::Null,
    }
}
fn jopt_style(v: &Option<JVal>) -> JVal {
    v.clone().unwrap_or(JVal::Null)
}
/// JS `x != null ? !!x : <skip>` for a wire value: `null`/absent maps to
/// `None` (leave the field alone), anything else to its truthiness (JS
/// `!!0 === false`, `!!"" === false`).
fn js_truthy_opt(v: &JVal) -> Option<bool> {
    match v {
        JVal::Null => None,
        JVal::Bool(b) => Some(*b),
        JVal::Num(n) => Some(*n != 0.0 && !n.is_nan()),
        JVal::Str(s) => Some(!s.is_empty()),
        JVal::Arr(_) | JVal::Obj(_) => Some(true),
    }
}

/// Ordered JSON object from `&str` keys (JVal stores `String` keys).
fn jobj(pairs: Vec<(&str, JVal)>) -> JVal {
    JVal::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `_wireLobby` (session.js L186-189) — the exact field set and order.
#[must_use]
pub fn lobby_to_jval(l: &Lobby) -> JVal {
    jobj(vec![
        ("map", jstr(&l.map)),
        ("time", jstr(&l.time)),
        ("duration", jnum(l.duration)),
        ("bots", jbool(l.bots)),
        ("difficulty", jstr(&l.difficulty)),
        ("palette", jnum(l.palette as i64)),
        ("mode", jstr(&l.mode)),
        (
            "players",
            JVal::Arr(l.players.iter().map(player_to_jval).collect()),
        ),
    ])
}

/// The per-player mapping of `_wireLobby` L188.
#[must_use]
pub fn player_to_jval(p: &LobbyPlayer) -> JVal {
    jobj(vec![
        ("id", jstr(&p.id)),
        ("name", jstr(&p.name)),
        ("team", p.team.to_jval()),
        ("weapon", jstr(&p.weapon)),
        ("sub", jopt_str(&p.sub)),
        ("special", jopt_str(&p.special)),
        ("style", jopt_style(&p.style)),
        ("ready", jbool(p.ready)),
        ("ping", jnum(p.ping)),
    ])
}

/// `{k:'lobby', l}` (session.js L183).
#[must_use]
pub fn encode_lobby(l: &Lobby) -> String {
    jobj(vec![("k", jstr("lobby")), ("l", lobby_to_jval(l))]).to_wire()
}

/// The `{k:'start'}` cfg (session.js L292 key order).
#[must_use]
pub fn encode_start(cfg: &StartCfg) -> String {
    let roster = cfg
        .roster
        .iter()
        .map(|r| {
            jobj(vec![
                ("nid", jnum(r.nid as i64)),
                ("owner", jstr(&r.owner)),
                ("bot", jbool(r.bot)),
                ("team", jnum(r.team as i64)),
                ("slot", jnum(r.slot as i64)),
                ("name", jstr(&r.name)),
                ("weapon", jstr(&r.weapon)),
                ("sub", jopt_str(&r.sub)),
                ("special", jopt_str(&r.special)),
                ("style", jopt_style(&r.style)),
            ])
        })
        .collect();
    jobj(vec![
        ("k", jstr("start")),
        ("roster", JVal::Arr(roster)),
        ("map", jstr(&cfg.map)),
        ("time", jstr(&cfg.time)),
        ("duration", jnum(cfg.duration)),
        ("difficulty", jstr(&cfg.difficulty)),
        ("palette", jnum(cfg.palette as i64)),
        ("mode", jstr(&cfg.mode)),
        ("host", jstr(&cfg.host)),
        ("id", jstr(&cfg.id)),
    ])
    .to_wire()
}

/// `{k:'ready', id}` / `{k:'go', id}` (session.js L317/L333).
#[must_use]
pub fn encode_ready(id: &str) -> String {
    jobj(vec![("k", jstr("ready")), ("id", jstr(id))]).to_wire()
}

#[must_use]
pub fn encode_go(id: &str) -> String {
    jobj(vec![("k", jstr("go")), ("id", jstr(id))]).to_wire()
}

/// `{k:'emote', n}` (session.js L262).
#[must_use]
pub fn encode_emote(name: &str) -> String {
    jobj(vec![("k", jstr("emote")), ("n", jstr(name))]).to_wire()
}

/// `{k:'me', …}` — the present fields in the JS insertion order
/// (`setMe` L200-206: name, weapon, sub, special, style, ready, team; the
/// connect-time `me` L110 and the ping `me` L388 are subsets of the same
/// canonical order, with `ping` last as in the fixture). Takes the same
/// [`MeChange`] the lobby applies: `None` = key absent, `Some(None)` for
/// `sub`/`special` = explicit `null`.
#[must_use]
pub fn encode_me(o: &MeChange) -> String {
    let mut pairs = vec![("k", jstr("me"))];
    if let Some(v) = &o.name {
        pairs.push(("name", jstr(v)));
    }
    if let Some(v) = &o.weapon {
        pairs.push(("weapon", jstr(v)));
    }
    if let Some(v) = &o.sub {
        pairs.push(("sub", jopt_str(v)));
    }
    if let Some(v) = &o.special {
        pairs.push(("special", jopt_str(v)));
    }
    if let Some(v) = &o.style {
        pairs.push(("style", v.clone()));
    }
    if let Some(v) = o.ready {
        pairs.push(("ready", jbool(v)));
    }
    if let Some(v) = o.team {
        pairs.push(("team", v.to_jval()));
    }
    if let Some(v) = &o.ping {
        pairs.push(("ping", jnum(*v)));
    }
    jobj(pairs).to_wire()
}

/// Parse a `start` payload back into a [`StartCfg`] (the guest side of
/// `_message` L373). Missing/typed-wrong fields fall back like JS
/// (`undefined` would simply render wrong; we require the contract shape).
fn decode_start(v: &JVal) -> Option<StartCfg> {
    let roster = v.field("roster")?.as_array()?;
    let mut out = Vec::with_capacity(roster.len());
    for r in roster {
        out.push(RosterEntry {
            nid: r.field("nid")?.as_f64()? as u8,
            owner: r.field("owner")?.as_str()?.to_string(),
            bot: r.field("bot")?.as_bool()?,
            team: r.field("team")?.as_f64()? as u8,
            slot: r.field("slot")?.as_f64()? as u8,
            name: r.field("name")?.as_str()?.to_string(),
            weapon: r.field("weapon")?.as_str()?.to_string(),
            sub: match r.field("sub") {
                Some(JVal::Str(s)) => Some(s.clone()),
                _ => None,
            },
            special: match r.field("special") {
                Some(JVal::Str(s)) => Some(s.clone()),
                _ => None,
            },
            style: r.field("style").filter(|s| **s != JVal::Null).cloned(),
        });
    }
    Some(StartCfg {
        roster: out,
        map: v.field("map")?.as_str()?.to_string(),
        time: v.field("time")?.as_str()?.to_string(),
        duration: v.field("duration")?.as_f64()? as i64,
        difficulty: v.field("difficulty")?.as_str()?.to_string(),
        palette: v.field("palette")?.as_f64()? as usize,
        mode: v.field("mode")?.as_str()?.to_string(),
        host: v.field("host")?.as_str()?.to_string(),
        id: v.field("id")?.as_str()?.to_string(),
    })
}

/// Parse a `lobby` payload into a [`Lobby`] (guest side of `_message`
/// L358-370). `host`/`you` are re-marked by the caller.
fn decode_lobby(v: &JVal) -> Option<Lobby> {
    let players = v.field("players")?.as_array()?;
    let mut out = Vec::with_capacity(players.len());
    for p in players {
        out.push(LobbyPlayer {
            id: p.field("id")?.as_str()?.to_string(),
            name: p.field("name")?.as_str()?.to_string(),
            team: p
                .field("team")
                .map(TeamSel::from_jval)
                .unwrap_or(TeamSel::Auto),
            weapon: p.field("weapon")?.as_str()?.to_string(),
            sub: match p.field("sub") {
                Some(JVal::Str(s)) => Some(s.clone()),
                _ => None,
            },
            special: match p.field("special") {
                Some(JVal::Str(s)) => Some(s.clone()),
                _ => None,
            },
            style: p.field("style").filter(|s| **s != JVal::Null).cloned(),
            ready: p.field("ready").and_then(JVal::as_bool).unwrap_or(false),
            ping: p.field("ping").and_then(JVal::as_f64).unwrap_or(0.0) as i64,
            host: false,
            you: false,
        });
    }
    Some(Lobby {
        map: v.field("map")?.as_str()?.to_string(),
        time: v.field("time")?.as_str()?.to_string(),
        duration: v.field("duration")?.as_f64()? as i64,
        bots: v.field("bots").and_then(JVal::as_bool).unwrap_or(false),
        difficulty: v.field("difficulty")?.as_str()?.to_string(),
        // Placeholder: the caller re-reads `l.palette` from the raw JVal with
        // the `Number.isInteger` test (L363); this zero is never used.
        palette: 0,
        mode: v.field("mode")?.as_str()?.to_string(),
        players: out,
        max_players: MAX_PLAYERS,
    })
}

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

/// `_fixTeams` (session.js L173-179), verbatim: honour explicit team
/// requests while keeping ≤ [`TEAM_SIZE`] a side, then place everyone still
/// on `auto` on the smaller side, then re-mark hosts.
pub fn fix_teams(players: &mut [LobbyPlayer], host_id: Option<&str>) {
    let mut count = [0usize, 0];
    for p in players.iter_mut() {
        let req = match p.team {
            TeamSel::Zero => Some(0),
            TeamSel::One => Some(1),
            TeamSel::Auto => None,
        };
        if let Some(t) = req {
            if count[t] < TEAM_SIZE {
                count[t] += 1;
            } else {
                p.team = TeamSel::Auto;
            }
        }
    }
    for p in players.iter_mut() {
        if p.team == TeamSel::Auto {
            let t = if count[0] <= count[1] { 0 } else { 1 };
            p.team = if t == 0 { TeamSel::Zero } else { TeamSel::One };
            count[t] += 1;
        }
    }
    for p in players.iter_mut() {
        p.host = Some(p.id.as_str()) == host_id;
    }
}

/// A `create`/`join` in flight (drives the `Room code taken` retry loop,
/// session.js L72-79).
#[derive(Debug, Clone)]
struct Pending {
    code: String,
    name: String,
    create: bool,
    tries: u8,
}

/// The session state machine — the Rust `NetSession` (see module docs).
/// `T` is the transport (fake in tests, native/wasm in the app) and `F` a
/// factory for it: JS builds a fresh `Transport` per connect attempt
/// (`_connect` L91), so retries need one too.
pub struct NetSession<T: Transport, F: FnMut() -> T> {
    factory: F,
    tr: Option<T>,
    state: SessionState,
    code: Option<String>,
    my_id: Option<SessionId>,
    host_id: Option<SessionId>,
    error: Option<String>,
    lobby: Lobby,
    profile: Profile,
    /// `this._members` (L31): relay membership, authoritative for who is
    /// connected (id → name).
    members: HashMap<SessionId, String>,
    pending: Option<Pending>,
    /// `this._startCfg` (L32) + `this._ready` (L301).
    start_cfg: Option<StartCfg>,
    ready: HashSet<SessionId>,
    /// `this._goT` (L327): the 12 s go-fallback countdown, seconds.
    go_t: Option<f64>,
    /// `this._botsPref` (L33): host's fill-with-bots preference.
    bots_pref: Option<bool>,
    /// `this._pingT` (L383): the 2 s lobby ping countdown, seconds (JS
    /// starts undefined ⇒ the first `update` fires immediately).
    ping_t: f64,
    /// Deterministic `Math.random()` stand-in (NFR-M2-3).
    rng: u64,
    events: Vec<SessionEvent>,
}

impl<T: Transport, F: FnMut() -> T> NetSession<T, F> {
    #[must_use]
    pub fn new(profile: Profile, rng_seed: u64, factory: F) -> Self {
        Self {
            factory,
            tr: None,
            state: SessionState::Offline,
            code: None,
            my_id: None,
            host_id: None,
            error: None,
            lobby: Lobby::default(),
            profile,
            members: HashMap::new(),
            pending: None,
            start_cfg: None,
            ready: HashSet::new(),
            go_t: None,
            bots_pref: None,
            ping_t: 0.0,
            rng: rng_seed | 1,
            events: Vec::new(),
        }
    }

    /// JS re-reads `G.game.profile` on every `_connect` (L95); the Rust seam
    /// is this setter — the app layer (Task 24) calls it when its profile
    /// changes, and the next `create`/`join` picks it up via `_profile()`.
    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
    }

    /// `get isHost` (session.js L36).
    #[must_use]
    pub fn is_host(&self) -> bool {
        self.my_id.is_some() && self.my_id == self.host_id
    }

    /// `get active` (L37): in a match. (Rust has no `this.match` yet — Task
    /// 21 adds it; `state == Match` is the same predicate for now.)
    #[must_use]
    pub fn active(&self) -> bool {
        self.state == SessionState::Match
    }

    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
    }
    /// The room code once the connect settled (L97).
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
    #[must_use]
    pub fn my_id(&self) -> Option<&str> {
        self.my_id.as_deref()
    }
    #[must_use]
    pub fn host_id(&self) -> Option<&str> {
        self.host_id.as_deref()
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn lobby(&self) -> &Lobby {
        &self.lobby
    }
    #[must_use]
    pub fn start_cfg(&self) -> Option<&StartCfg> {
        self.start_cfg.as_ref()
    }

    /// The owned transport (the app drives native/wasm I/O; tests use this
    /// to call [`crate::net::transport::FakeTransport::advance`]).
    #[must_use]
    pub fn transport_mut(&mut self) -> Option<&mut T> {
        self.tr.as_mut()
    }

    /// Drain pending session events (oldest first).
    pub fn poll_events(&mut self) -> Vec<SessionEvent> {
        std::mem::take(&mut self.events)
    }

    fn set_state(&mut self, s: SessionState) {
        if self.state == s {
            return;
        }
        self.state = s;
        self.events.push(SessionEvent::State { state: s });
    }

    fn gen_code(&mut self) -> String {
        // L74: five alphabet chars, `(Math.random() * 28) | 0` each.
        (0..5)
            .map(|_| {
                let i =
                    (rand01(&mut self.rng) * CODE_CHARS.len() as f64) as usize % CODE_CHARS.len();
                CODE_CHARS[i] as char
            })
            .collect()
    }

    /// `Math.random().toString(36).slice(2, 8)` (L292): six lowercase
    /// base36 chars.
    fn base36_id(&mut self) -> String {
        const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        (0..6)
            .map(|_| D[(rand01(&mut self.rng) * 36.0) as usize % 36] as char)
            .collect()
    }

    // ------------------------------------------------------------------ rooms

    /// `create(name)` (session.js L71-79): mint a code, connect with
    /// `create=1`, and retry (≤ 4 attempts total) while the relay answers
    /// `Room code taken`. The settled code surfaces via
    /// [`SessionEvent::State`] → `Lobby`; read it back with [`Self::code`].
    pub fn create(&mut self, name: Option<&str>) {
        let profile = self.profile.sanitized();
        let name = name.map(js_slice16).unwrap_or(profile.name.clone());
        // `tries` counts attempts made; the JS loop allows 4 (L73).
        self.pending = Some(Pending {
            code: String::new(),
            name,
            create: true,
            tries: 1,
        });
        self.retry_connect();
    }

    /// `join(code, name)` (L81-85): uppercase, strip non-alphanumerics,
    /// require ≥ 4 chars, then connect (no create-retry).
    pub fn join(&mut self, code: &str, name: Option<&str>) {
        let profile = self.profile.sanitized();
        let name = name.map(js_slice16).unwrap_or(profile.name.clone());
        let code: String = code
            .to_uppercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if code.len() < 4 {
            let e = "Room not found".to_string();
            self.error = Some(e.clone());
            if let Some(tr) = self.tr.as_mut() {
                tr.close();
            }
            self.tr = None;
            self.set_state(SessionState::Error);
            self.events.push(SessionEvent::Error { message: e });
            return;
        }
        self.pending = Some(Pending {
            code,
            name,
            create: false,
            tries: 1,
        });
        self.retry_connect();
    }

    /// `_connect(code, name, create)` (L87-112) for the current `pending`.
    fn retry_connect(&mut self) {
        let Some(p) = self.pending.clone() else {
            return;
        };
        // `leave(true)` first (L88).
        self.leave(true);
        self.error = None;
        let code = if p.create {
            self.gen_code()
        } else {
            p.code.clone()
        };
        self.pending = Some(Pending {
            code: code.clone(),
            name: p.name.clone(),
            create: p.create,
            tries: p.tries,
        });
        self.set_state(SessionState::Connecting);
        let mut tr = (self.factory)();
        tr.connect(&code, &p.name, p.create);
        self.tr = Some(tr);
    }

    /// `leave(silent)` (L114-124).
    pub fn leave(&mut self, silent: bool) {
        // match?.dispose() — Task 21.
        if let Some(mut tr) = self.tr.take() {
            tr.close();
        }
        let was = self.state;
        self.code = None;
        self.my_id = None;
        self.host_id = None;
        self.members.clear();
        self.lobby = Lobby::default();
        self.start_cfg = None;
        self.ready.clear();
        self.go_t = None;
        self.bots_pref = None;
        self.pending = None;
        if !silent && was != SessionState::Offline {
            self.set_state(SessionState::Offline);
            let lobby = self.lobby.clone();
            self.events.push(SessionEvent::Lobby { lobby });
        } else {
            self.state = SessionState::Offline;
        }
    }

    /// `_fail(e)` (L126-131).
    fn fail(&mut self, e: &str) {
        self.error = Some(e.to_string());
        if let Some(mut tr) = self.tr.take() {
            tr.close();
        }
        self.pending = None;
        self.set_state(SessionState::Error);
        self.events.push(SessionEvent::Error {
            message: self.error.clone().unwrap_or_default(),
        });
    }

    /// `_closed(reason)` (L133-142).
    fn closed(&mut self, reason: &str) {
        let in_match = self.state == SessionState::Match || self.state == SessionState::Starting;
        self.error = if reason == "bye" {
            None
        } else {
            Some("Lost connection to the room".to_string())
        };
        self.tr = None;
        self.code = None;
        self.pending = None;
        let state = if self.error.is_some() {
            SessionState::Error
        } else {
            SessionState::Offline
        };
        self.set_state(state);
        if let Some(e) = self.error.clone() {
            self.events.push(SessionEvent::Error { message: e });
        }
        if in_match {
            // JS calls `netMatchAborted(this.error)` for any in-match close,
            // including the clean `bye` (L141) — the argument is only the
            // error text, not a condition.
            self.events.push(SessionEvent::Match {
                phase: MatchPhase::Aborted,
            });
        }
    }

    // ------------------------------------------------------------------ lobby plumbing

    /// `_newPlayer(id, name, o)` (L168-170). `o` is the loadout patch
    /// (empty for join frames).
    fn new_player(&self, id: SessionId, name: &str, o: &MeChange) -> LobbyPlayer {
        LobbyPlayer {
            id: id.clone(),
            name: js_slice16(if name.is_empty() { "Player" } else { name }),
            team: TeamSel::Auto,
            weapon: match &o.weapon {
                Some(w) if weapon_ok(w) => w.clone(),
                _ => "shooter".to_string(),
            },
            sub: sub_of(o.sub.as_ref().and_then(|v| v.as_ref())),
            special: special_of(o.special.as_ref().and_then(|v| v.as_ref())),
            // Deviation: JS `randomStyle()`; Rust passes through / null.
            style: o.style.clone(),
            ready: false,
            ping: 0,
            host: Some(&id) == self.host_id.as_ref(),
            you: Some(&id) == self.my_id.as_ref(),
        }
    }

    /// `_fixTeams` (L173-179), verbatim: honour explicit team requests
    /// while keeping ≤ [`TEAM_SIZE`] a side, then place everyone still on
    /// `auto` on the smaller side, then re-mark hosts.
    fn fix_teams(&mut self) {
        fix_teams(&mut self.lobby.players, self.host_id.as_deref());
    }

    /// `_broadcastLobby` (L181-185).
    fn broadcast_lobby(&mut self) {
        if !self.is_host() {
            return;
        }
        if let Some(tr) = self.tr.as_mut() {
            let json = encode_lobby(&self.lobby);
            tr.broadcast(&json);
        }
        self.push_lobby();
    }

    /// `_pushLobby` (L191-194).
    fn push_lobby(&mut self) {
        for p in self.lobby.players.iter_mut() {
            p.you = Some(p.id.as_str()) == self.my_id.as_deref();
            p.host = Some(p.id.as_str()) == self.host_id.as_deref();
        }
        let lobby = self.lobby.clone();
        self.events.push(SessionEvent::Lobby { lobby });
    }

    // ------------------------------------------------------------------ lobby actions

    /// `setMe(ch)` (L197-214).
    pub fn set_me(&mut self, ch: MeChange) {
        if self.tr.is_none() || self.my_id.is_none() {
            return;
        }
        // Sanitise like JS `setMe` does (L200-206).
        let o = MeChange {
            name: ch.name.map(|v| js_slice16(&v)),
            weapon: ch.weapon.filter(|w| weapon_ok(w)),
            sub: ch.sub.map(|v| v.filter(|s| SUB_IDS.contains(&s.as_str()))),
            special: ch
                .special
                .map(|v| v.filter(|s| SPECIAL_IDS.contains(&s.as_str()))),
            style: ch.style,
            // JS `!!ch.ready` (L205): a `Some(false)` stays false; the
            // `!= null` guard is the `Option` itself.
            ready: ch.ready,
            team: ch.team,
            ping: None,
        };
        if self.is_host() {
            self.apply_me(&self.my_id.clone().expect("checked above"), &o);
            return;
        }
        // Optimistic local echo for the fields the host won't refuse (L210-211).
        let my_id = self.my_id.clone().expect("checked above");
        if let Some(me) = self.lobby.players.iter_mut().find(|p| p.id == my_id) {
            if let Some(v) = &o.name {
                me.name = v.clone();
            }
            if let Some(v) = &o.weapon {
                me.weapon = v.clone();
            }
            if let Some(v) = &o.sub {
                me.sub = v.clone();
            }
            if let Some(v) = &o.special {
                me.special = v.clone();
            }
            if let Some(v) = &o.style {
                me.style = Some(v.clone());
            }
            if let Some(v) = o.ready {
                me.ready = v;
            }
            self.push_lobby();
        }
        let json = encode_me(&o);
        if let (Some(tr), Some(host)) = (self.tr.as_mut(), self.host_id.clone()) {
            tr.send_to(&host, &json);
        }
    }

    /// `_applyMe(id, o)` (L216-233) — host side, from a `me` payload.
    fn apply_me_jval(&mut self, id: &str, d: &JVal) {
        let o = MeChange {
            name: d
                .field("name")
                .and_then(JVal::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            weapon: d
                .field("weapon")
                .and_then(JVal::as_str)
                .filter(|s| weapon_ok(s))
                .map(|s| s.to_string()),
            sub: d.field("sub").map(|v| match v {
                JVal::Str(s) if SUB_IDS.contains(&s.as_str()) => Some(s.clone()),
                _ => None,
            }),
            special: d.field("special").map(|v| match v {
                JVal::Str(s) if SPECIAL_IDS.contains(&s.as_str()) => Some(s.clone()),
                _ => None,
            }),
            style: d.field("style").filter(|v| **v != JVal::Null).cloned(),
            // JS `if (o.ready != null) p.ready = !!o.ready` (L224) — null is
            // skipped; everything else takes JS truthiness (`0`/`""`/`NaN`
            // are false, not true).
            ready: d.field("ready").and_then(js_truthy_opt),
            team: d.field("team").and_then(|v| match v {
                JVal::Str(s) if s == "auto" => Some(TeamSel::Auto),
                JVal::Num(n) if *n == 0.0 => Some(TeamSel::Zero),
                JVal::Num(n) if *n == 1.0 => Some(TeamSel::One),
                _ => None,
            }),
            // `ping` rides the same frame (L225): `Math.round(o.ping)` when
            // non-null, merged into the same apply pass.
            ping: d
                .field("ping")
                .and_then(JVal::as_f64)
                .map(|p| wire::js_round(p) as i64),
        };
        self.apply_me(id, &o);
    }

    fn apply_me(&mut self, id: &str, o: &MeChange) {
        if !self.lobby.players.iter().any(|x| x.id == id) {
            return;
        }
        // Pre-compute the team cap outside the mutable borrow (L228 counts
        // the *others* already on the requested side).
        let team_ok = match o.team {
            Some(t @ (TeamSel::Zero | TeamSel::One)) => {
                let n = self
                    .lobby
                    .players
                    .iter()
                    .filter(|x| x.id != id && x.team == t)
                    .count();
                (n < TEAM_SIZE).then_some(t)
            }
            Some(TeamSel::Auto) => Some(TeamSel::Auto),
            None => None,
        };
        if let Some(p) = self.lobby.players.iter_mut().find(|x| x.id == id) {
            // JS `if (o.name)` (L219) — an empty name is skipped.
            if let Some(v) = o.name.as_ref().filter(|s| !s.is_empty()) {
                p.name = v.clone();
            }
            if let Some(v) = &o.weapon {
                p.weapon = v.clone();
            }
            if let Some(v) = &o.sub {
                p.sub = v.clone();
            }
            if let Some(v) = &o.special {
                p.special = v.clone();
            }
            if let Some(v) = &o.style {
                p.style = Some(v.clone());
            }
            if let Some(v) = o.ready {
                p.ready = v;
            }
            if let Some(v) = o.ping {
                p.ping = v;
            }
            if let Some(t) = team_ok {
                p.team = t;
            }
        }
        self.fix_teams();
        self.broadcast_lobby();
    }

    /// `setSettings(s)` (L235-251). M2 deviation: `mode` only accepts
    /// `'turf'` (the host refuses to send zones/boss).
    pub fn set_settings(&mut self, s: SettingsPatch) {
        if !self.is_host() {
            return;
        }
        let l = &mut self.lobby;
        if let Some(m) = &s.map
            && map_exists(m)
        {
            l.map = m.clone();
        }
        if matches!(s.time.as_deref(), Some("day") | Some("dusk")) {
            l.time = s.time.clone().expect("matched");
        }
        if let Some(d) = s.duration.filter(|d| *d != 0.0 && !d.is_nan()) {
            // `if (s.duration)` is a truthiness test (L240) — 0/NaN skip the
            // field; otherwise `Math.max(60, Math.min(600, +s.duration | 0))`
            // with `| 0` truncating toward zero.
            let d = (d as i64).clamp(60, 600);
            l.duration = d;
        }
        if let Some(b) = s.bots {
            self.bots_pref = Some(b);
        }
        if matches!(
            s.difficulty.as_deref(),
            Some("easy") | Some("normal") | Some("hard")
        ) {
            l.difficulty = s.difficulty.clone().expect("matched");
        }
        if let Some(p) = s.palette {
            // `Number.isInteger && 0 <= p < TEAM_PALETTES.length` (L243).
            if p.fract() == 0.0 && p >= 0.0 && (p as usize) < PALETTE_COUNT {
                l.palette = p as usize;
            }
        }
        if s.mode.as_deref() == Some("turf") {
            l.mode = "turf".to_string();
        }
        // Stage rules (L249): a noBots stage forces bots off; the host's
        // own preference comes back on the next stage. (The boss-mode map
        // fallback of L248 is unreachable in turf-only M2.)
        let pref = self.bots_pref;
        l.bots = if map_no_bots(&l.map) {
            false
        } else {
            pref.unwrap_or(l.bots)
        };
        self.broadcast_lobby();
    }

    /// `canStart()` (L253-256).
    #[must_use]
    pub fn can_start(&self) -> bool {
        if !self.is_host() || self.state != SessionState::Lobby {
            return false;
        }
        let my_id = self.my_id.as_deref();
        self.start_block().is_none()
            && self
                .lobby
                .players
                .iter()
                .all(|p| p.ready || Some(p.id.as_str()) == my_id)
    }

    /// `startBlock()` = `noBotsStartBlock(lobby)` (L258; config.js
    /// L539-545): why a humans-only room cannot start regardless of
    /// ready-ups.
    #[must_use]
    pub fn start_block(&self) -> Option<&'static str> {
        if !map_no_bots(&self.lobby.map) {
            return None;
        }
        let ps = &self.lobby.players;
        if ps.len() < 2 {
            return Some("Needs 2+ players — no bots on this stage");
        }
        if !ps.iter().any(|p| p.team == TeamSel::Zero) || !ps.iter().any(|p| p.team == TeamSel::One)
        {
            return Some("Needs a player on each team");
        }
        None
    }

    /// `emote(name)` (L260-264).
    pub fn emote(&mut self, name: &str) {
        if self.tr.is_none() || self.my_id.is_none() {
            return;
        }
        let json = encode_emote(&js_slice16(name));
        if let Some(tr) = self.tr.as_mut() {
            tr.broadcast(&json);
        }
        self.events.push(SessionEvent::Emote {
            id: self.my_id.clone().expect("checked above"),
            name: name.to_string(),
        });
    }

    // ------------------------------------------------------------------ match orchestration

    /// `start()` (L267-297) — host only, turf mode only.
    pub fn start(&mut self) -> bool {
        if !self.is_host()
            || self.state != SessionState::Lobby
            || self.tr.is_none()
            || self.start_block().is_some()
        {
            return false;
        }
        // Snapshot the lobby so the RNG-driven build below can take
        // `&mut self` freely (JS reads `this.lobby` field-by-field).
        let (players, bots, map, time, duration, difficulty, palette) = {
            let l = &self.lobby;
            (
                l.players.clone(),
                l.bots && !map_no_bots(&l.map),
                l.map.clone(),
                l.time.clone(),
                l.duration,
                l.difficulty.clone(),
                l.palette,
            )
        };
        let host = self.my_id.clone().expect("is_host");
        let mut roster: Vec<RosterEntry> = Vec::new();
        let mut nid = 0u8;
        let mut names = self.shuffled(BOT_NAMES);
        for team in 0..2u8 {
            // L277: the weapon shuffle happens *per team* — each side draws
            // its own order (one shared shuffle would collapse both teams'
            // bot weapon sets).
            let weapons = self.shuffled(WEAPON_ORDER);
            let humans: Vec<&LobbyPlayer> = players
                .iter()
                .filter(|p| {
                    if team == 0 {
                        p.team == TeamSel::Zero
                    } else {
                        p.team == TeamSel::One
                    }
                })
                .collect();
            let mut slot = 0u8;
            for p in humans {
                roster.push(RosterEntry {
                    nid,
                    owner: p.id.clone(),
                    bot: false,
                    team,
                    slot,
                    name: p.name.clone(),
                    weapon: p.weapon.clone(),
                    sub: sub_of(p.sub.as_ref()),
                    special: special_of(p.special.as_ref()),
                    style: p.style.clone(),
                });
                nid += 1;
                slot += 1;
            }
            if bots {
                while (slot as usize) < TEAM_SIZE {
                    // `weapons.find(w => !used.has(w)) || weapons[slot % len]`
                    // (L282-283).
                    let used: Vec<String> = roster
                        .iter()
                        .filter(|r| r.team == team)
                        .map(|r| r.weapon.clone())
                        .collect();
                    let wpn = weapons
                        .iter()
                        .find(|w| !used.iter().any(|u| u == *w))
                        .unwrap_or(&weapons[slot as usize % weapons.len()])
                        .to_string();
                    // L285-286: a random sub/special about half the time.
                    let sub = if rand01(&mut self.rng) < 0.5 {
                        None
                    } else {
                        Some(
                            SUB_IDS[(rand01(&mut self.rng) * SUB_IDS.len() as f64) as usize
                                % SUB_IDS.len()]
                            .to_string(),
                        )
                    };
                    let special = if rand01(&mut self.rng) < 0.5 {
                        None
                    } else {
                        Some(
                            SPECIAL_IDS[(rand01(&mut self.rng) * SPECIAL_IDS.len() as f64)
                                as usize
                                % SPECIAL_IDS.len()]
                            .to_string(),
                        )
                    };
                    let name = names.pop().unwrap_or("Bot").to_string();
                    roster.push(RosterEntry {
                        nid,
                        owner: host.clone(),
                        bot: true,
                        team,
                        slot,
                        name,
                        weapon: wpn,
                        sub,
                        special,
                        // Deviation: JS `randomStyle()`.
                        style: None,
                    });
                    nid += 1;
                    slot += 1;
                }
            }
        }
        let id = self.base36_id();
        // Turf-only: the zones/boss duration + mode branches of L292 are
        // unreachable.
        let cfg = StartCfg {
            roster,
            map,
            time,
            duration,
            difficulty,
            palette,
            mode: "turf".to_string(),
            host,
            id,
        };
        if let Some(tr) = self.tr.as_mut() {
            tr.lock(true);
            let json = encode_start(&cfg);
            tr.broadcast(&json);
        }
        self.begin(cfg);
        true
    }

    fn shuffled<'a>(&mut self, items: &[&'a str]) -> Vec<&'a str> {
        let mut v = items.to_vec();
        for i in (1..v.len()).rev() {
            let j = (rand01(&mut self.rng) * (i + 1) as f64) as usize % (i + 1);
            v.swap(i, j);
        }
        v
    }

    /// `_begin(cfg)` (L299-318) minus the async launch seams: the app builds
    /// the match when [`SessionEvent::Match { phase: Start }`] arrives and
    /// then calls [`Self::mark_loaded`] (JS sends `ready` only after
    /// `startNetMatch` resolves, L310-317).
    fn begin(&mut self, cfg: StartCfg) {
        self.start_cfg = Some(cfg);
        self.ready.clear();
        self.go_t = None;
        for p in self.lobby.players.iter_mut() {
            p.ready = false;
        }
        self.set_state(SessionState::Starting);
        self.events.push(SessionEvent::Match {
            phase: MatchPhase::Start,
        });
    }

    /// The app's seam: the match finished loading (both host and guests).
    pub fn mark_loaded(&mut self) {
        if self.state != SessionState::Starting || self.start_cfg.is_none() {
            return;
        }
        if self.is_host() {
            let my_id = self.my_id.clone().expect("is_host");
            self.mark_ready(&my_id);
        } else if let (Some(tr), Some(host), Some(cfg)) = (
            self.tr.as_mut(),
            self.host_id.clone(),
            self.start_cfg.clone(),
        ) {
            tr.send_to(&host, &encode_ready(&cfg.id));
        }
    }

    /// `_markReady(id)` (L320-328).
    fn mark_ready(&mut self, id: &str) {
        if !self.is_host() || self.state != SessionState::Starting || self.start_cfg.is_none() {
            return;
        }
        self.ready.insert(id.to_string());
        let humans: Vec<SessionId> = {
            let cfg = self.start_cfg.as_ref().expect("checked");
            let mut v: Vec<SessionId> = Vec::new();
            for r in &cfg.roster {
                if !r.bot && !v.contains(&r.owner) {
                    v.push(r.owner.clone());
                }
            }
            v
        };
        // L324: a human that left while loading counts as ready.
        for h in &humans {
            if !self.members.contains_key(h) && Some(h.as_str()) != self.my_id.as_deref() {
                self.ready.insert(h.clone());
            }
        }
        let all = humans.iter().all(|h| self.ready.contains(h));
        if all {
            self.go();
        } else if self.go_t.is_none() {
            self.go_t = Some(12.0);
        }
    }

    /// `_go()` (L330-335).
    fn go(&mut self) {
        self.go_t = None;
        if self.state != SessionState::Starting {
            return;
        }
        if let (Some(tr), Some(cfg)) = (self.tr.as_mut(), self.start_cfg.clone()) {
            tr.broadcast(&encode_go(&cfg.id));
        }
        self.launch();
    }

    /// `_launch()` (L337-341).
    fn launch(&mut self) {
        self.set_state(SessionState::Match);
        self.events.push(SessionEvent::Match {
            phase: MatchPhase::Go,
        });
    }

    /// `endMatch()` (L344-352): the results were shown, everyone back to
    /// the lobby (the room stays).
    pub fn end_match(&mut self) {
        self.start_cfg = None;
        self.ready.clear();
        self.go_t = None;
        if self.tr.is_none() {
            return;
        }
        if self.is_host() {
            if let Some(tr) = self.tr.as_mut() {
                tr.lock(false);
            }
            for p in self.lobby.players.iter_mut() {
                p.ready = false;
            }
            self.broadcast_lobby();
        }
        self.set_state(SessionState::Lobby);
        self.events.push(SessionEvent::Match {
            phase: MatchPhase::End,
        });
        self.push_lobby();
    }

    // ------------------------------------------------------------------ incoming

    /// `_control(o)` (L145-166) for join/leave relay frames.
    fn on_control(&mut self, ev: TransportEvent) {
        match ev {
            TransportEvent::Join { id, name } => {
                self.members.insert(id.clone(), name.clone());
                if self.is_host() {
                    let p = self.new_player(id, &name, &MeChange::default());
                    self.lobby.players.push(p);
                    self.fix_teams();
                    self.broadcast_lobby();
                }
            }
            TransportEvent::Leave { id, host } => {
                let gone = self
                    .lobby
                    .players
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .unwrap_or_else(|| LobbyPlayer {
                        id: id.clone(),
                        name: self
                            .members
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(|| "Player".into()),
                        team: TeamSel::Auto,
                        weapon: "shooter".to_string(),
                        sub: None,
                        special: None,
                        style: None,
                        ready: false,
                        ping: 0,
                        host: false,
                        you: false,
                    });
                self.members.remove(&id);
                let host_changed = Some(host.as_str()) != self.host_id.as_deref();
                self.host_id = Some(host.clone());
                self.lobby.players.retain(|p| p.id != id);
                for p in self.lobby.players.iter_mut() {
                    p.host = p.id == host;
                }
                // match?.onLeave(id, hostChanged) — Task 21 consumes the
                // Leave/Host events.
                if host_changed {
                    self.events.push(SessionEvent::Host { host_id: host });
                }
                if self.is_host() {
                    self.fix_teams();
                    self.broadcast_lobby();
                }
                self.events.push(SessionEvent::Leave {
                    player: gone,
                    reason: "left".to_string(),
                });
                self.push_lobby();
            }
            _ => {}
        }
    }

    /// `_message(from, d)` (L355-378).
    fn on_message(&mut self, from: SessionId, payload: String) {
        let Some(v) = wire::parse_json(&payload) else {
            return;
        };
        if !matches!(v, JVal::Obj(_)) {
            return;
        }
        match v.field("k").and_then(JVal::as_str) {
            Some("lobby") => {
                if Some(from.as_str()) != self.host_id.as_deref() {
                    return;
                }
                let Some(l) = v.field("l").and_then(decode_lobby) else {
                    return;
                };
                // L362-364: field-by-field apply; palette only when an
                // integer; M2 coerces a non-turf mode to turf (deviation:
                // JS keeps zones/boss).
                self.lobby.map = l.map;
                self.lobby.time = l.time;
                self.lobby.duration = l.duration;
                self.lobby.bots = l.bots;
                self.lobby.difficulty = l.difficulty;
                if let Some(p) = v
                    .field("l")
                    .and_then(|x| x.field("palette"))
                    .and_then(JVal::as_f64)
                    && p.fract() == 0.0
                {
                    // L363 is `Number.isInteger(l.palette)` alone. A negative
                    // integer saturates to 0 through `as usize` (JS keeps it
                    // verbatim) — only a hostile host can send one.
                    self.lobby.palette = p as usize;
                }
                self.lobby.mode = "turf".to_string();
                let prev: HashSet<String> =
                    self.lobby.players.iter().map(|p| p.id.clone()).collect();
                let host_id = self.host_id.clone();
                self.lobby.players = l
                    .players
                    .into_iter()
                    .map(|mut p| {
                        p.host = Some(p.id.as_str()) == host_id.as_deref();
                        p
                    })
                    .collect();
                for p in &self.lobby.players {
                    if !prev.contains(&p.id) {
                        self.events.push(SessionEvent::Join { player: p.clone() });
                    }
                }
                self.push_lobby();
            }
            Some("me") => {
                if self.is_host() {
                    self.apply_me_jval(&from, &v);
                }
            }
            Some("emote") => {
                let name = v
                    .field("n")
                    .and_then(JVal::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.events.push(SessionEvent::Emote { id: from, name });
            }
            Some("start") => {
                if Some(from.as_str()) != self.host_id.as_deref()
                    || self.state != SessionState::Lobby
                {
                    return;
                }
                let Some(cfg) = decode_start(&v) else {
                    return;
                };
                // M2: a non-turf cfg is refused — error + leave.
                if cfg.mode != "turf" {
                    self.fail("Unsupported game mode");
                    self.leave(false);
                    return;
                }
                self.begin(cfg);
            }
            Some("ready") => {
                let same = self
                    .start_cfg
                    .as_ref()
                    .is_some_and(|c| v.field("id").and_then(JVal::as_str) == Some(c.id.as_str()));
                if self.is_host() && same {
                    self.mark_ready(&from);
                }
            }
            Some("go") => {
                if Some(from.as_str()) == self.host_id.as_deref()
                    && self.state == SessionState::Starting
                {
                    self.launch();
                }
            }
            _ => {
                // `default: match?.onMessage` (L376) — outside a match JS
                // ignores the payload; the replication layer (Task 21)
                // consumes these once the match exists.
                if self.state == SessionState::Match || self.state == SessionState::Starting {
                    self.events
                        .push(SessionEvent::GameMessage { from, payload });
                }
            }
        }
    }

    /// Drain the transport, run the lobby ping timer and the 12 s go
    /// fallback. `dt` is in **seconds** (session.js `update(dt)`).
    pub fn update(&mut self, dt: f64) {
        // 1. Transport events (control + payloads).
        let mut drained = Vec::new();
        if let Some(tr) = self.tr.as_mut() {
            drained = tr.poll_events();
        }
        for ev in drained {
            match ev {
                TransportEvent::Welcome { id, host, members } => {
                    self.on_welcome(id, host, members);
                }
                TransportEvent::Error { e } => {
                    // `create`'s retry loop (L73-79): exactly 4 attempts,
                    // and only `Room code taken` re-mints — anything else
                    // fails immediately (JS `break`).
                    //
                    // After an Error the batch is stale either way: JS
                    // `_fail`/`_connect` close the old transport, whose
                    // nulled `onclose` never fires (transport.js L75/L108),
                    // so the paired `Closed` must not reach `_closed`.
                    let retry = self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.create && p.tries < 4 && e == "Room code taken");
                    if retry {
                        if let Some(p) = self.pending.as_mut() {
                            p.tries += 1;
                        }
                        self.retry_connect();
                    } else {
                        self.fail(&e);
                    }
                    break;
                }
                TransportEvent::Closed { reason } => self.closed(&reason),
                ev @ (TransportEvent::Join { .. } | TransportEvent::Leave { .. }) => {
                    self.on_control(ev)
                }
                TransportEvent::Message { from, payload } => self.on_message(from, payload),
            }
        }
        // 2. Lobby ping cadence (L381-390).
        if self.tr.is_some() && self.state == SessionState::Lobby {
            self.ping_t -= dt;
            if self.ping_t <= 0.0 {
                self.ping_t = 2.0;
                let ping = self.tr.as_ref().map_or(0.0, |t| t.rtt());
                let ping = wire::js_round(ping) as i64;
                if self.is_host() {
                    let changed = self
                        .lobby
                        .players
                        .iter_mut()
                        .find(|p| Some(p.id.as_str()) == self.my_id.as_deref())
                        .is_some_and(|me| {
                            if me.ping != ping {
                                me.ping = ping;
                                true
                            } else {
                                false
                            }
                        });
                    if changed {
                        self.broadcast_lobby();
                    }
                } else if let (Some(tr), Some(host)) = (self.tr.as_mut(), self.host_id.clone()) {
                    let o = MeChange {
                        ping: Some(ping),
                        ..Default::default()
                    };
                    tr.send_to(&host, &encode_me(&o));
                }
            }
        }
        // 3. The 12 s go fallback (L327).
        if let Some(t) = self.go_t.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.go();
            }
        }
    }

    /// The welcome arm of `_connect` (L96-111).
    fn on_welcome(&mut self, id: SessionId, host: SessionId, members: Vec<(SessionId, String)>) {
        let Some(p) = self.pending.take() else {
            return;
        };
        self.code = Some(p.code.clone());
        self.my_id = Some(id);
        self.host_id = Some(host);
        self.members.clear();
        for (mid, name) in members {
            self.members.insert(mid, name);
        }
        self.lobby = Lobby::default();
        self.bots_pref = if self.is_host() { Some(true) } else { None };
        if self.is_host() {
            let profile = self.profile.sanitized();
            let me = self.new_player(
                self.my_id.clone().expect("welcome"),
                &p.name,
                &MeChange {
                    weapon: Some(profile.weapon.clone()),
                    sub: Some(profile.sub.clone()),
                    special: Some(profile.special.clone()),
                    style: profile.style.clone(),
                    ..Default::default()
                },
            );
            self.lobby.players = vec![me];
            self.fix_teams();
        }
        self.set_state(SessionState::Lobby);
        // Tell the host who we are (the host already knows itself, L110).
        // `sub`/`special`/`style` are always present on this frame (JS
        // serialises `null`, not an absent key).
        if !self.is_host() {
            let profile = self.profile.sanitized();
            // `sub`/`special`/`style` keys are always present (JS serialises
            // `null`, not an absent key): `Some(None)` / `Some(JVal::Null)`.
            let o = MeChange {
                name: Some(p.name.clone()),
                weapon: Some(profile.weapon.clone()),
                sub: Some(profile.sub.clone()),
                special: Some(profile.special.clone()),
                style: Some(profile.style.clone().unwrap_or(JVal::Null)),
                ..Default::default()
            };
            let json = encode_me(&o);
            if let (Some(tr), Some(host)) = (self.tr.as_mut(), self.host_id.clone()) {
                tr.send_to(&host, &json);
            }
        }
        self.push_lobby();
    }
}

#[cfg(test)]
mod tests {
    use super::super::transport::{FakeRelay, FakeTransport, LinkSim, Transport, rand01};
    use super::*;

    /// Task 17 gold-standard fixtures (`rust/assets/net-fixtures.json`).
    const FIXTURES: &str = include_str!("../../../../assets/net-fixtures.json");

    fn fixture_msg(name: &str) -> String {
        let f: serde_json::Value = serde_json::from_str(FIXTURES).expect("fixtures parse");
        f["fixtures"]["msgs"][name]
            .as_str()
            .expect("fixture msg is a string")
            .to_string()
    }

    type Sess = NetSession<FakeTransport, Box<dyn FnMut() -> FakeTransport>>;

    fn sess(relay: &FakeRelay, seed: u64, sim: Option<LinkSim>) -> Sess {
        let r = relay.clone();
        NetSession::new(
            Profile::default(),
            seed,
            Box::new(move || r.transport(sim, seed)),
        )
    }

    /// Move the shared virtual clock by `dt_ms` (through `s`'s transport, if
    /// any) and tick the session. `update` takes **seconds**.
    fn step(s: &mut Sess, dt_ms: f64) {
        if let Some(tr) = s.transport_mut() {
            tr.advance(dt_ms);
        }
        s.update(dt_ms / 1000.0);
    }

    /// Drive two sessions sharing one relay: the clock moves exactly once per
    /// tick; both endpoints are pumped, then both sessions drain.
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

    fn drain(s: &mut Sess) -> Vec<SessionEvent> {
        s.poll_events()
    }

    fn has_event(evts: &[SessionEvent], want: &SessionEvent) -> bool {
        evts.contains(want)
    }

    /// Host `create` + guest `join` until both lobbies show two players;
    /// returns them with all pending events drained.
    fn host_and_guest(relay: &FakeRelay) -> (Sess, Sess, String) {
        let mut h = sess(relay, 11, None);
        h.create(None);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Lobby, "host reached lobby");
        let code = h.code().expect("settled code").to_string();
        drain(&mut h);
        let mut g = sess(relay, 22, None);
        g.join(&code, None);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Lobby
                && g.lobby().players.len() == 2
                && h.lobby().players.len() == 2
            {
                break;
            }
        }
        assert_eq!(h.lobby().players.len(), 2, "host sees both");
        assert_eq!(g.lobby().players.len(), 2, "guest sees the host lobby");
        drain(&mut h);
        drain(&mut g);
        (h, g, code)
    }

    // -- TR-20.1: create / join / leave and error paths ----------------------

    #[test]
    fn create_mints_valid_code_and_reaches_lobby() {
        let relay = FakeRelay::new();
        let mut h = sess(&relay, 11, None);
        h.create(None);
        assert_eq!(h.state(), SessionState::Connecting);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        let code = h.code().expect("code settles on welcome").to_string();
        assert_eq!(code.len(), 5);
        for c in code.bytes() {
            assert!(CODE_CHARS.contains(&c), "code char {c:?} in alphabet");
        }
        assert!(h.is_host());
        assert_eq!(h.lobby().players.len(), 1);
        let me = &h.lobby().players[0];
        assert_eq!(me.name, "Player");
        assert_eq!(me.weapon, "shooter");
        assert!(me.host && me.you);
        assert_eq!(me.team, TeamSel::Zero, "solo auto lands on team 0");
        let evts = drain(&mut h);
        assert!(has_event(
            &evts,
            &SessionEvent::State {
                state: SessionState::Lobby
            }
        ));
    }

    #[test]
    fn create_retries_with_a_new_code_when_taken() {
        // Occupy the *first* code the session's seeded RNG would mint, so the
        // relay answers `Room code taken` (session.js L73-79) and attempt 2
        // must re-mint. `gen_code` draws 5 `rand01` values per code off
        // `seed | 1` — replay it to find the collision.
        let mut st = 11u64 | 1;
        let mint = |st: &mut u64| -> String {
            (0..5)
                .map(|_| {
                    CODE_CHARS[((rand01(st) * CODE_CHARS.len() as f64) as usize) % CODE_CHARS.len()]
                        as char
                })
                .collect()
        };
        let first = mint(&mut st);
        let second = mint(&mut st);
        let relay = FakeRelay::new();
        let mut squat = relay.transport(None, 999);
        squat.connect(&first, "squatter", true);
        squat.advance(0.0);
        assert!(squat.is_open(), "squatter owns the first code");

        let mut h = sess(&relay, 11, None);
        h.create(None); // attempt 1 mints `first`; the err frame is in flight
        squat.close(); // the room frees before the first advance
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Lobby, "retry succeeded");
        assert_eq!(h.code().unwrap(), second, "second minted code wins");
        assert_ne!(h.code().unwrap(), first);
        // The retry never surfaced an error (JS swallows `Room code taken`
        // inside the loop; only the 5th failure would `_fail`).
        assert!(h.error().is_none());
    }

    #[test]
    fn join_sanitises_the_code_and_surfaces_relay_refusals() {
        let relay = FakeRelay::new();
        // < 4 cleaned chars: local refusal, no transport at all (L83).
        let mut g = sess(&relay, 22, None);
        g.join("ab!", None);
        assert_eq!(g.state(), SessionState::Error);
        assert_eq!(g.error(), Some("Room not found"));
        assert!(g.transport_mut().is_none());
        let evts = drain(&mut g);
        assert!(has_event(
            &evts,
            &SessionEvent::Error {
                message: "Room not found".into()
            }
        ));
        // ≥ 4 cleaned chars: uppercase + strip, then the relay's own text.
        let mut g2 = sess(&relay, 33, None);
        g2.join(" ab-12 ", None); // → "AB12"
        assert_eq!(g2.state(), SessionState::Connecting);
        for _ in 0..50 {
            step(&mut g2, 1.0);
            if g2.state() == SessionState::Error {
                break;
            }
        }
        assert_eq!(g2.state(), SessionState::Error);
        assert_eq!(g2.error(), Some("Room not found"));
    }

    #[test]
    fn join_a_locked_room_gets_match_in_progress() {
        let relay = FakeRelay::new();
        let (mut h, mut g, code) = host_and_guest(&relay);
        assert!(h.start(), "host starts");
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Starting {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Starting);
        // The `lock` frame reached the relay (session.js L294).
        let mut late = sess(&relay, 44, None);
        late.join(&code, None);
        for _ in 0..50 {
            step(&mut late, 1.0);
            if late.state() == SessionState::Error {
                break;
            }
        }
        assert_eq!(late.error(), Some("Match in progress"));
    }

    #[test]
    fn leave_resets_the_local_view_and_peers_see_it() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        g.leave(false);
        assert_eq!(g.state(), SessionState::Offline);
        assert!(g.code().is_none() && g.my_id().is_none());
        assert_eq!(g.lobby().players.len(), 0);
        let evts = drain(&mut g);
        assert!(has_event(
            &evts,
            &SessionEvent::State {
                state: SessionState::Offline
            }
        ));
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.lobby().players.len() == 1 {
                break;
            }
        }
        assert_eq!(h.lobby().players.len(), 1, "host dropped the guest");
        let evts = drain(&mut h);
        assert!(
            evts.iter().any(|e| matches!(e, SessionEvent::Leave { .. })),
            "leave event surfaced: {evts:?}"
        );
    }

    #[test]
    fn connection_loss_surfaces_lost_connection() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        g.transport_mut()
            .expect("transport")
            .simulate_closed("relay sweep");
        for _ in 0..50 {
            step(&mut g, 1.0);
            if g.state() == SessionState::Error {
                break;
            }
        }
        assert_eq!(g.error(), Some("Lost connection to the room"));
        assert_eq!(g.state(), SessionState::Error);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.lobby().players.len() == 1 {
                break;
            }
        }
        assert_eq!(h.lobby().players.len(), 1, "host saw the drop");
    }

    // -- TR-20.2: lobby authority, teams, settings, wire bytes ---------------

    #[test]
    fn guest_me_roundtrips_through_the_host() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        let guest_id = g.my_id().expect("guest id").to_string();
        g.set_me(MeChange {
            weapon: Some("roller".into()),
            sub: Some(Some("bomb".into())),
            special: Some(Some("zooka".into())),
            ready: Some(true),
            ..Default::default()
        });
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            let hp = h.lobby().players.iter().find(|p| p.id == guest_id);
            if hp.is_some_and(|p| p.weapon == "roller" && p.ready) {
                break;
            }
        }
        let hp = h
            .lobby()
            .players
            .iter()
            .find(|p| p.id == guest_id)
            .expect("host applied me");
        assert_eq!(hp.weapon, "roller");
        assert_eq!(hp.sub.as_deref(), Some("bomb"));
        assert_eq!(hp.special.as_deref(), Some("zooka"));
        assert!(hp.ready);
        assert_eq!(hp.team, TeamSel::One, "guest auto-balanced to team 1");
        // The broadcast comes back to the guest; the optimistic echo survives.
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            let gp = g.lobby().players.iter().find(|p| p.id == guest_id);
            if gp.is_some_and(|p| p.weapon == "roller") {
                break;
            }
        }
        let gp = g
            .lobby()
            .players
            .iter()
            .find(|p| p.id == guest_id)
            .expect("guest entry");
        assert_eq!(gp.weapon, "roller");
        assert!(gp.you, "local view marks itself");
    }

    #[test]
    fn set_me_rejects_unknown_weapon_locally() {
        let relay = FakeRelay::new();
        let (mut h, _g, _) = host_and_guest(&relay);
        let host_id = h.my_id().expect("host id").to_string();
        h.set_me(MeChange {
            weapon: Some("laser".into()),
            ..Default::default()
        });
        let p = h
            .lobby()
            .players
            .iter()
            .find(|x| x.id == host_id)
            .expect("host entry");
        assert_eq!(p.weapon, "shooter", "bad weapon id is dropped (L201)");
    }

    #[test]
    fn fix_teams_caps_honours_requests_and_balances() {
        // 8 players: five explicitly on team 0 — the 5th is demoted to auto
        // and lands on the smaller side (_fixTeams L176-177).
        let mk = |id: &str, t: TeamSel| LobbyPlayer {
            id: id.into(),
            name: id.into(),
            team: t,
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
            ready: false,
            ping: 0,
            host: false,
            you: false,
        };
        let mut ps: Vec<LobbyPlayer> = vec![
            mk("a", TeamSel::Zero),
            mk("b", TeamSel::Zero),
            mk("c", TeamSel::Zero),
            mk("d", TeamSel::Zero),
            mk("e", TeamSel::Zero),
            mk("f", TeamSel::Auto),
            mk("g", TeamSel::One),
            mk("h", TeamSel::Auto),
        ];
        fix_teams(&mut ps, Some("a"));
        let team = |id: &str| ps.iter().find(|p| p.id == id).unwrap().team;
        assert_eq!(team("a"), TeamSel::Zero);
        assert_eq!(team("d"), TeamSel::Zero);
        assert_eq!(
            team("e"),
            TeamSel::One,
            "5th explicit Zero overflows the cap → auto → smaller side"
        );
        assert_eq!(team("f"), TeamSel::One, "auto fills the smaller side");
        assert_eq!(team("g"), TeamSel::One);
        assert_eq!(
            team("h"),
            TeamSel::One,
            "smaller side keeps absorbing autos"
        );
        let zeros = ps.iter().filter(|p| p.team == TeamSel::Zero).count();
        let ones = ps.iter().filter(|p| p.team == TeamSel::One).count();
        assert_eq!((zeros, ones), (4, 4));
        assert!(ps.iter().find(|p| p.id == "a").unwrap().host);
        assert!(!ps.iter().find(|p| p.id == "b").unwrap().host);
        // All-auto alternates 0,1,0,1 (count[0] <= count[1] tie → 0 first).
        let mut solo: Vec<LobbyPlayer> = (0..4)
            .map(|i| mk(&format!("s{i}"), TeamSel::Auto))
            .collect();
        fix_teams(&mut solo, None);
        assert_eq!(
            solo.iter().map(|p| p.team).collect::<Vec<_>>(),
            vec![TeamSel::Zero, TeamSel::One, TeamSel::Zero, TeamSel::One]
        );
    }

    #[test]
    fn set_settings_clamps_and_validates_like_js() {
        let relay = FakeRelay::new();
        let (mut h, mut _g, _) = host_and_guest(&relay);
        h.set_settings(SettingsPatch {
            duration: Some(30.0),
            ..Default::default()
        });
        assert_eq!(h.lobby().duration, 60, "Math.max(60, …) (L240)");
        h.set_settings(SettingsPatch {
            duration: Some(999.0),
            ..Default::default()
        });
        assert_eq!(h.lobby().duration, 600);
        h.set_settings(SettingsPatch {
            duration: Some(120.7),
            ..Default::default()
        });
        assert_eq!(h.lobby().duration, 120, "`| 0` truncates");
        h.set_settings(SettingsPatch {
            map: Some("nope".into()),
            time: Some("night".into()),
            difficulty: Some("hell".into()),
            mode: Some("zones".into()),
            palette: Some(1.5),
            ..Default::default()
        });
        assert_eq!(h.lobby().map, "tidewater", "unknown map ignored (L237)");
        assert_eq!(h.lobby().time, "day");
        assert_eq!(h.lobby().difficulty, "normal");
        assert_eq!(h.lobby().mode, "turf", "M2: non-turf mode refused");
        assert_eq!(h.lobby().palette, 0, "non-integer palette ignored (L243)");
        h.set_settings(SettingsPatch {
            palette: Some(7.0),
            ..Default::default()
        });
        assert_eq!(h.lobby().palette, 0, "out of range ignored");
        h.set_settings(SettingsPatch {
            palette: Some(3.0),
            ..Default::default()
        });
        assert_eq!(h.lobby().palette, 3);
        // Stage rule (L249): cargo forces bots off; the pref returns with
        // the stage.
        h.set_settings(SettingsPatch {
            map: Some("cargo".into()),
            ..Default::default()
        });
        assert!(!h.lobby().bots, "noBots stage");
        h.set_settings(SettingsPatch {
            bots: Some(false),
            ..Default::default()
        });
        h.set_settings(SettingsPatch {
            map: Some("kelpline".into()),
            ..Default::default()
        });
        assert!(
            !h.lobby().bots,
            "explicit pref false survives the stage swap"
        );
        h.set_settings(SettingsPatch {
            bots: Some(true),
            ..Default::default()
        });
        assert!(h.lobby().bots);
        // Guests cannot change settings (setSettings is host-only, L236).
        let guest_before = h.lobby().clone();
        _g.set_settings(SettingsPatch {
            map: Some("saltpan".into()),
            ..Default::default()
        });
        assert_eq!(h.lobby(), &guest_before);
    }

    #[test]
    fn wire_encoders_are_byte_exact_against_fixtures() {
        assert_eq!(
            encode_me(&MeChange {
                name: Some("ab".into()),
                weapon: Some("shooter".into()),
                sub: Some(None),
                special: Some(None),
                style: Some(JVal::Null),
                ready: None,
                team: None,
                ping: Some(42),
            }),
            fixture_msg("me")
        );
        assert_eq!(
            encode_ready("abc123"),
            fixture_msg("ready"),
            "ready carries the match id"
        );
        assert_eq!(encode_go("abc123"), fixture_msg("go"));
        assert_eq!(encode_emote("booyah"), fixture_msg("emote"));

        let lp = |id: &str, name: &str, team: TeamSel, weapon: &str, ready: bool, ping: i64| {
            LobbyPlayer {
                id: id.into(),
                name: name.into(),
                team,
                weapon: weapon.into(),
                sub: None,
                special: None,
                style: None,
                ready,
                ping,
                host: false,
                you: false,
            }
        };
        let l1 = Lobby {
            map: "tidewater".into(),
            time: "day".into(),
            duration: 180,
            bots: true,
            difficulty: "normal".into(),
            palette: 0,
            mode: "turf".into(),
            players: vec![lp("A1B2", "ab", TeamSel::Zero, "shooter", true, 42)],
            max_players: MAX_PLAYERS,
        };
        assert_eq!(encode_lobby(&l1), fixture_msg("lobby"));
        let l2 = Lobby {
            map: "tidewater".into(),
            time: "dusk".into(),
            duration: 120,
            bots: false,
            difficulty: "hard".into(),
            palette: 2,
            mode: "turf".into(),
            players: vec![
                lp("A1B2", "ab", TeamSel::Zero, "shooter", true, 15),
                lp("K3XZ", "cd", TeamSel::One, "spritzer", false, 88),
            ],
            max_players: MAX_PLAYERS,
        };
        assert_eq!(encode_lobby(&l2), fixture_msg("lobby2"));

        let re = |nid: u8, owner: &str, bot: bool, team: u8, slot: u8, name: &str, weapon: &str| {
            RosterEntry {
                nid,
                owner: owner.into(),
                bot,
                team,
                slot,
                name: name.into(),
                weapon: weapon.into(),
                sub: None,
                special: None,
                style: None,
            }
        };
        let s1 = StartCfg {
            roster: vec![
                re(0, "A1B2", false, 0, 0, "ab", "shooter"),
                re(1, "A1B2", true, 0, 1, "Bot", "shooter"),
            ],
            map: "tidewater".into(),
            time: "day".into(),
            duration: 180,
            difficulty: "normal".into(),
            palette: 0,
            mode: "turf".into(),
            host: "A1B2".into(),
            id: "abc123".into(),
        };
        assert_eq!(encode_start(&s1), fixture_msg("start"));
        let s2 = StartCfg {
            roster: vec![
                re(0, "A1B2", false, 0, 0, "ab", "shooter"),
                re(1, "K3XZ", false, 0, 1, "cd", "spritzer"),
                re(2, "A1B2", true, 0, 2, "B0", "shooter"),
                re(3, "A1B2", true, 0, 3, "B1", "shooter"),
                re(4, "A1B2", true, 1, 0, "B2", "shooter"),
                re(5, "A1B2", true, 1, 1, "B3", "shooter"),
                re(6, "A1B2", true, 1, 2, "B4", "shooter"),
                re(7, "A1B2", true, 1, 3, "B5", "shooter"),
            ],
            id: "xyz789".into(),
            ..s1.clone()
        };
        assert_eq!(encode_start(&s2), fixture_msg("start2"));
    }

    #[test]
    fn emote_broadcasts_and_echoes_locally() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        let host_id = h.my_id().expect("host id").to_string();
        h.emote("booyah");
        let evts = drain(&mut h);
        assert!(
            has_event(
                &evts,
                &SessionEvent::Emote {
                    id: host_id.clone(),
                    name: "booyah".into()
                }
            ),
            "sender sees its own emote (L263): {evts:?}"
        );
        let mut seen = Vec::new();
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            seen.extend(drain(&mut g));
            if seen.iter().any(|e| matches!(e, SessionEvent::Emote { .. })) {
                break;
            }
        }
        assert!(
            has_event(
                &seen,
                &SessionEvent::Emote {
                    id: host_id,
                    name: "booyah".into()
                }
            ),
            "peer receives the emote (L372)"
        );
    }

    // -- TR-20.3: start → ready → go ----------------------------------------

    #[test]
    fn start_ready_go_full_sequence() {
        let relay = FakeRelay::new();
        let (mut h, mut g, code) = host_and_guest(&relay);
        assert!(!h.can_start(), "guest is not ready yet (L255)");
        g.set_me(MeChange {
            ready: Some(true),
            ..Default::default()
        });
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.can_start() {
                break;
            }
        }
        assert!(h.can_start(), "all humans ready ⇒ start allowed");

        let host_id = h.my_id().expect("host").to_string();
        let guest_id = g.my_id().expect("guest").to_string();
        assert!(h.start());
        assert_eq!(h.state(), SessionState::Starting);
        let evts = drain(&mut h);
        assert!(has_event(
            &evts,
            &SessionEvent::Match {
                phase: MatchPhase::Start
            }
        ));
        h.mark_loaded(); // host is instantly "loaded"; reports ready (L316)

        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Starting {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Starting, "guest began from cfg");
        drain(&mut g);
        g.mark_loaded(); // guest sends {k:'ready', id} (L317)

        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.state() == SessionState::Match {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Match, "go fired on last ready");
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Match {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Match, "guest launched on go");
        let gevts = drain(&mut g);
        assert!(has_event(
            &gevts,
            &SessionEvent::Match {
                phase: MatchPhase::Go
            }
        ));

        // Roster shape (session.js L271-290): team 0 humans+bots then team 1,
        // 4 a side with bots filling, nid 0..7, bots owned by the host.
        let cfg = h.start_cfg().expect("cfg kept").clone();
        assert_eq!(cfg.roster.len(), MAX_PLAYERS, "bots fill to 4+4");
        assert_eq!(cfg.mode, "turf");
        assert_eq!(cfg.host, host_id);
        assert_eq!(cfg.duration, 180);
        assert_eq!(cfg.id.len(), 6, "base36 match id");
        for (i, r) in cfg.roster.iter().enumerate() {
            assert_eq!(r.nid as usize, i);
            assert_eq!(r.team, (i / 4) as u8);
            assert_eq!(r.slot, (i % 4) as u8);
            if r.bot {
                assert_eq!(r.owner, host_id, "bots belong to the host");
                assert!(BOT_NAMES.contains(&r.name.as_str()), "bot name");
            }
        }
        assert_eq!(cfg.roster[0].owner, host_id);
        assert!(!cfg.roster[0].bot);
        assert_eq!(cfg.roster[4].owner, guest_id);
        assert!(!cfg.roster[4].bot);
        // Weapons unique within a team (L282 `used` set).
        for team in 0..2u8 {
            let ws: Vec<&str> = cfg
                .roster
                .iter()
                .filter(|r| r.team == team)
                .map(|r| r.weapon.as_str())
                .collect();
            let uniq: HashSet<&str> = ws.iter().copied().collect();
            assert_eq!(ws.len(), uniq.len(), "team {team} weapons: {ws:?}");
        }
        // The guest decoded the *same* cfg byte-for-byte (fixture fidelity is
        // pinned by `wire_encoders_are_byte_exact_against_fixtures`).
        assert_eq!(g.start_cfg(), Some(&cfg), "guest cfg == host cfg");
        // The relay is locked while the match loads/plays.
        let mut late = sess(&relay, 55, None);
        late.join(&code, None);
        for _ in 0..50 {
            step(&mut late, 1.0);
            if late.state() != SessionState::Connecting {
                break;
            }
        }
        assert_eq!(late.error(), Some("Match in progress"));

        // endMatch: host unlocks, everyone returns to the lobby (L344-352).
        h.end_match();
        assert_eq!(h.state(), SessionState::Lobby);
        g.end_match();
        assert_eq!(g.state(), SessionState::Lobby);
        let hevts = drain(&mut h);
        assert!(has_event(
            &hevts,
            &SessionEvent::Match {
                phase: MatchPhase::End
            }
        ));
        let mut late2 = sess(&relay, 66, None);
        late2.join(&code, None);
        for _ in 0..50 {
            step2(&mut h, &mut late2, 1.0);
            if late2.state() == SessionState::Lobby {
                break;
            }
        }
        assert_eq!(
            late2.state(),
            SessionState::Lobby,
            "lock released for joins"
        );
    }

    #[test]
    fn go_fallback_fires_at_12s() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        h.start();
        h.mark_loaded(); // host ready; guest never loads
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Starting {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Starting);
        drain(&mut h);
        drain(&mut g);
        // 11.99 s: the timer has not expired (L327). The Starting-wait loop
        // consumed a few ms of `go_t`, so leave a generous margin.
        step2(&mut h, &mut g, 11_990.0);
        assert_eq!(h.state(), SessionState::Starting, "not before 12 s");
        assert_eq!(g.state(), SessionState::Starting);
        // +15 ms: the fallback go releases everyone (L330-334).
        step2(&mut h, &mut g, 15.0);
        assert_eq!(h.state(), SessionState::Match, "12 s fallback go");
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Match {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Match, "guest got the fallback go");
    }

    #[test]
    fn member_dropped_while_loading_counts_as_ready() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        h.start(); // host does NOT mark_loaded yet
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Starting {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Starting);
        drain(&mut h);
        drain(&mut g);
        g.transport_mut()
            .expect("transport")
            .simulate_closed("sweep");
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if h.lobby().players.len() == 1 {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Starting, "host still loading");
        // The host finishes loading *after* the drop: the missing human counts
        // as ready, so go fires immediately (L324) — no 12 s wait.
        h.mark_loaded();
        assert_eq!(h.state(), SessionState::Match);
        let evts = drain(&mut h);
        assert!(has_event(
            &evts,
            &SessionEvent::Match {
                phase: MatchPhase::Go
            }
        ));
    }

    #[test]
    fn guest_refuses_a_non_turf_start() {
        // A legacy/malicious host broadcasts the fixture `start` with
        // mode=zones. M2: the guest fails with the player-facing text and
        // leaves (module docs, turf-only deviation).
        let relay = FakeRelay::new();
        let mut boss = relay.transport(None, 7);
        boss.connect("BCDE2", "boss", true);
        boss.advance(0.0);
        let mut g = sess(&relay, 22, None);
        g.join("BCDE2", None);
        for _ in 0..50 {
            step(&mut g, 1.0);
            if g.state() == SessionState::Lobby {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Lobby);
        drain(&mut g);
        let zones = fixture_msg("start").replace("\"mode\":\"turf\"", "\"mode\":\"zones\"");
        boss.broadcast(&zones);
        for _ in 0..50 {
            step(&mut g, 1.0);
            if g.state() != SessionState::Lobby {
                break;
            }
        }
        assert_eq!(g.error(), Some("Unsupported game mode"));
        assert_eq!(g.state(), SessionState::Offline, "error then leave");
        assert!(g.transport_mut().is_none());
    }

    #[test]
    fn lobby_ping_flows_to_the_host_and_back() {
        // 40 ms one-way lag: the heartbeat pong is SIM-held on the way back,
        // so rtt = 40 (transport.js L53/L69). Stepping 1 ms at a time keeps
        // the pong delivery exact (a coarse step would inflate the sample).
        let relay = FakeRelay::new();
        let sim = Some(LinkSim {
            lag_ms: 40.0,
            jitter_ms: 0.0,
            spike_p: 0.0,
        });
        let mut h = sess(&relay, 11, sim);
        h.create(None);
        let mut code = String::new();
        for _ in 0..200 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                code = h.code().expect("code").to_string();
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Lobby);
        drain(&mut h);
        let mut g = sess(&relay, 22, sim);
        g.join(&code, None);
        for _ in 0..400 {
            step2(&mut h, &mut g, 1.0);
            if g.lobby().players.len() == 2 && h.lobby().players.len() == 2 {
                break;
            }
        }
        assert_eq!(h.lobby().players.len(), 2);
        drain(&mut h);
        drain(&mut g);
        // The 2 s lobby timer (L381-390): the host writes its own rtt into
        // the lobby, guests report via {k:'me',ping}.
        let guest_id = g.my_id().expect("guest").to_string();
        let mut g_ping = 0i64;
        let mut h_ping = 0i64;
        for _ in 0..6000 {
            step2(&mut h, &mut g, 1.0);
            h_ping = h
                .lobby()
                .players
                .iter()
                .find(|p| p.id == guest_id)
                .map_or(0, |p| p.ping);
            g_ping = g
                .lobby()
                .players
                .iter()
                .find(|p| p.host)
                .map_or(0, |p| p.ping);
            if h_ping == 40 && g_ping == 40 {
                break;
            }
        }
        assert_eq!(h_ping, 40, "guest rtt surfaces on the host lobby");
        assert_eq!(g_ping, 40, "host rtt surfaces on the guest lobby");
    }

    #[test]
    fn host_migration_on_leave() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        let guest_id = g.my_id().expect("guest").to_string();
        h.leave(false);
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.is_host() {
                break;
            }
        }
        assert!(g.is_host(), "oldest member elected (server L139-141)");
        let evts = drain(&mut g);
        assert!(
            has_event(
                &evts,
                &SessionEvent::Host {
                    host_id: guest_id.clone()
                }
            ),
            "host event surfaced (L161): {evts:?}"
        );
        assert!(
            evts.iter().any(|e| matches!(e, SessionEvent::Leave { .. })),
            "leave event surfaced: {evts:?}"
        );
        // The new host re-broadcasts; its own entry is marked host.
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.lobby().players.len() == 1 {
                break;
            }
        }
        assert_eq!(g.lobby().players.len(), 1);
        assert!(g.lobby().players[0].host);
        assert_eq!(g.lobby().players[0].id, guest_id);
    }

    #[test]
    fn game_messages_only_forward_during_a_match() {
        let relay = FakeRelay::new();
        let (mut h, mut g, _) = host_and_guest(&relay);
        // Lobby phase: `default: match?.onMessage` — no match yet ⇒ dropped.
        h.transport_mut()
            .expect("tr")
            .broadcast("{\"k\":\"t\",\"ts\":1}");
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
        }
        let evts = drain(&mut g);
        assert!(
            !evts
                .iter()
                .any(|e| matches!(e, SessionEvent::GameMessage { .. })),
            "lobby payloads are ignored: {evts:?}"
        );
        // Drive both into the match, then a tick payload must surface.
        h.start();
        h.mark_loaded();
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Starting {
                break;
            }
        }
        g.mark_loaded();
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            if g.state() == SessionState::Match {
                break;
            }
        }
        assert_eq!(g.state(), SessionState::Match);
        drain(&mut h);
        drain(&mut g);
        let host_id = h.my_id().expect("host").to_string();
        h.transport_mut()
            .expect("tr")
            .broadcast("{\"k\":\"t\",\"ts\":2}");
        let mut seen = Vec::new();
        for _ in 0..50 {
            step2(&mut h, &mut g, 1.0);
            seen.extend(drain(&mut g));
            if seen
                .iter()
                .any(|e| matches!(e, SessionEvent::GameMessage { .. }))
            {
                break;
            }
        }
        assert!(
            has_event(
                &seen,
                &SessionEvent::GameMessage {
                    from: host_id,
                    payload: "{\"k\":\"t\",\"ts\":2}".into()
                }
            ),
            "match payloads reach Task 21: {seen:?}"
        );
    }

    #[test]
    fn no_bots_stage_gates_the_start() {
        let relay = FakeRelay::new();
        let (mut h, _g, _) = host_and_guest(&relay);
        // Solo on a humans-only stage blocks (config.js L539-545); with the
        // guest split across both teams there is no block, and `start()`
        // proceeds without bots (session.js L270).
        h.set_settings(SettingsPatch {
            map: Some("cargo".into()),
            ..Default::default()
        });
        assert_eq!(h.start_block(), None, "two humans, one a side");
        assert!(h.start());
        let cfg = h.start_cfg().expect("cfg");
        assert_eq!(cfg.roster.len(), 2, "cargo fills no bots");
        assert!(!cfg.roster.iter().any(|r| r.bot));
    }

    #[test]
    fn create_exhausts_four_retries_and_fails() {
        // session.js L73-78: every attempt is refused `Room code taken`, so
        // after 4 tries the loop ends and `_fail(lastErr)` surfaces the text.
        // FakeRelay is single-room and ignores the code, so one open squatter
        // makes *every* mint collide.
        let relay = FakeRelay::new();
        let mut squat = relay.transport(None, 999);
        squat.connect("XXXXX", "squatter", true);
        squat.advance(0.0);
        assert!(squat.is_open());

        let mut h = sess(&relay, 11, None);
        h.create(None);
        for _ in 0..200 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Error {
                break;
            }
        }
        assert_eq!(h.state(), SessionState::Error);
        assert_eq!(h.error(), Some("Room code taken"));
        assert!(h.transport_mut().is_none(), "fail closes the transport");
        let evts = drain(&mut h);
        assert!(has_event(
            &evts,
            &SessionEvent::Error {
                message: "Room code taken".into()
            }
        ));
    }

    #[test]
    fn join_a_full_room_gets_room_is_full() {
        // The fail ladder's `open >= ROOM_MAX` arm (server/src/index.js L72):
        // the first raw client creates, seven more join, the eighth open slot
        // makes the session's join refuse.
        let relay = FakeRelay::new();
        let mut fillers = Vec::new();
        for i in 0..8u64 {
            let mut t = relay.transport(None, 100 + i);
            t.connect("FULL01", &format!("f{i}"), i == 0);
            t.advance(0.0);
            assert!(t.is_open(), "filler {i} joined");
            fillers.push(t);
        }
        let mut g = sess(&relay, 22, None);
        g.join("FULL01", None);
        for _ in 0..50 {
            step(&mut g, 1.0);
            if g.state() == SessionState::Error {
                break;
            }
        }
        assert_eq!(g.error(), Some("Room is full"));
        let _ = fillers;
    }

    #[test]
    fn solo_on_a_no_bots_stage_blocks_the_start() {
        // `startBlock()` (L258, config.js L539-545): a humans-only stage with
        // fewer than 2 players refuses the start regardless of ready-ups, and
        // `start()` (L268) returns false without locking or broadcasting.
        let relay = FakeRelay::new();
        let mut h = sess(&relay, 11, None);
        h.create(None);
        for _ in 0..50 {
            step(&mut h, 1.0);
            if h.state() == SessionState::Lobby {
                break;
            }
        }
        h.set_settings(SettingsPatch {
            map: Some("cargo".into()),
            ..Default::default()
        });
        assert_eq!(
            h.start_block(),
            Some("Needs 2+ players — no bots on this stage")
        );
        assert!(!h.can_start(), "blocked room cannot start");
        assert!(!h.start(), "start() refuses");
        assert_eq!(h.state(), SessionState::Lobby, "no lock, no cfg");
        assert!(h.start_cfg().is_none());
    }
}
