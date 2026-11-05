//! Task 13 — menu / HUD / results screens (bevy_ui).
//!
//! Screen state machine (upstream `SCREENS` main / pause / results + the
//! in-match HUD, menus.js L29-33):
//!   Menu ──Play──▶ Hud ◀──Esc/P──▶ Pause
//!                    │                │ Restart / Menu
//!                    └──match End──▶ Results ──Again / Menu
//!
//! `Screen::Menu` starts the app (TR-13.1 full chain); every non-Hud screen
//! freezes the sim through `PlayerControls.paused` (the Task 12 pause path).
//! HUD numbers come straight from the sim: clock `Match::time` (upstream
//! `fmtTime` ceil, ui-util.js L203-207), turf `PaintGrid::coverage` ×100
//! (main.js L990 passes `cov*100`), ink tank `Actor::ink / ink_max`, kill
//! feed from `MatchEvent::Splat` (hud.js feed). Results mirror `hud.judge`
//! (hud.js L530-551): winner line `{NAME} WINS!` + coverage percents.
//! Durations 90/180 s (config.js `MATCH.durations`, default 180).

use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::picking::events::{Click, Pointer};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use inkwave_sim::actor::{ActorInput, SplatCause};
use inkwave_sim::boss::BossEvent;
use inkwave_sim::match_::{MatchEvent, MatchResult, Phase, ZoneOutcome};
use inkwave_sim::tuning::Tuning;

use crate::actors::team_color;
use crate::audio::{BossAudioDirector, BossAudioFrame};
use crate::ink_render::DemoSim;
use crate::input::PlayerControls;
use crate::net::OnlineSession;
use crate::net::match_link::NetEvents;
use inkwave_sim::net::session::{SPECIAL_IDS, SUB_IDS, SessionEvent, SessionState, WEAPON_ORDER};

pub mod boss_hud;
pub mod minimap;
pub mod minimap_ui;
pub mod online;

use self::boss_hud::{BOSS_NAME, BossHudFrame, BossHudOut, BossHudState};
use self::online::{SessionCmd, run_cmd};

/// Which screen is live (exactly one at a time).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    /// Main menu; the sim sits frozen at the intro.
    Menu,
    /// Live match HUD.
    Hud,
    /// Pause overlay (Esc/P during play, upstream main.js L437).
    Pause,
    /// Results (match reached `Phase::End`).
    Results,
    /// Online hub (Task 24; menus.js `online` / `_scr_online` L2401).
    OnlineHub,
    /// Lobby (Task 24; menus.js `lobby` / `_scr_lobby` L2774).
    OnlineLobby,
}

/// Upstream default match length (config.js `MATCH.defaultDuration`).
/// The spec's "Play 直接开始 180s" is the Enter shortcut on the main menu;
/// the two buttons pick 90/180 explicitly.
const DEFAULT_DURATION: u32 = 180;

/// The UI state machine resource. `screen != Hud` ⇒ `pc.paused`.
#[derive(Resource)]
pub struct UiState {
    pub screen: Screen,
    /// Selected match length (upstream `MATCH.defaultDuration` = 180).
    pub duration: u32,
    /// Task 24: the online flow owns the screen — offline Play/Restart are
    /// refused and the Results buttons switch semantics (JS screens
    /// `online`/`lobby` + `main.js` `startNetMatch` hiding the menu).
    pub net_mode: bool,
    /// Task 24: the host pressed BACK TO LOBBY on the net Results screen
    /// (JS `netBackToLobby`, main.js L383); `net_tick` consumes the flag to
    /// fire `sendEnd` + `endMatch` before the screen tears down.
    pub net_back_pressed: bool,
    /// Task 24: `st.autoT` (menus.js L2554) — the 320 ms auto-join countdown
    /// armed when the fifth character lands in the last cell. `0.0` = no
    /// timer pending.
    pub auto_join: f64,
    /// Task 46 batch B (D-T44-5): the offline Play-menu mode selector. JS has
    /// no offline boss entry (boss only ever starts through `netmatch.js`
    /// L164-171); this is the Rust-side placeholder cycle (turf → boss —
    /// offline zones is not wired into `DemoSim`, Task 38 batch 3).
    pub play_mode: PlayMode,
}

/// Offline Play-menu match mode (Task 46 batch B).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PlayMode {
    #[default]
    Turf,
    Boss,
}

impl PlayMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Turf => "TURF",
            Self::Boss => "BOSS",
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Menu,
            duration: DEFAULT_DURATION,
            net_mode: false,
            net_back_pressed: false,
            auto_join: 0.0,
            play_mode: PlayMode::default(),
        }
    }
}

/// Task 46 batch C2 — the boss HUD + bossAudio state machines (≡ hud-boss.js
/// / bossAudio.js bus subscriptions). `hud` is `Some` only while a boss match
/// is live (`DemoSim::bm`); `out` caches the render descriptor for
/// `ui_update`, `audio` the trigger-log director.
#[derive(Resource, Default)]
pub struct BossHudRes {
    pub hud: Option<BossHudState>,
    pub out: BossHudOut,
    pub audio: BossAudioDirector,
}

/// Root panel of one screen; `ui_update` toggles its `Display`.
#[derive(Component)]
pub(crate) struct ScreenRoot(Screen);

/// Dynamic label refreshed by `ui_update` every frame.
#[derive(Component)]
pub(crate) struct LabelRole(&'static str);

/// Dynamic bar (width percentage) refreshed by `ui_update`.
#[derive(Component)]
pub(crate) struct BarRole(&'static str);

/// Crosshair arm (flashes on a local kill).
#[derive(Component)]
pub(crate) struct Crosshair;

/// Button command handled by `ui_state_machine` on click. (No `Eq`: the
/// Task 24 `Online(OnlineAction)` arm carries an `f64` duration.)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Start a fresh match of N seconds (upstream `MATCH.durations`).
    Play(u32),
    Resume,
    Restart,
    Menu,
    /// Quit the app (spec: main menu / pause "退出"; wasm has no window
    /// close so the button is hidden there, see `build_ui`).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Quit,
    /// Task 24: an online-flow button; mapped onto a `SessionCmd` / screen
    /// hop by `ui_state_machine`.
    Online(OnlineAction),
    /// Task 24: Results "BACK TO LOBBY" (JS `netMatchEnd` brings the lobby
    /// back, main.js L769-846).
    NetBackToLobby,
    /// Task 46 batch B (D-T44-5): cycle the offline Play-menu mode
    /// (turf → boss). JS has no offline boss entry; the online lobby's MODE
    /// button (`lob_mode`) is the interaction this mirrors.
    ModeCycle,
}

/// The online screens' button intents (menus.js `_scr_online` /
/// `_scr_lobby` click handlers). `Duration(f64)` keeps this off `Eq`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OnlineAction {
    /// Main-menu ONLINE card → hub screen.
    EnterHub,
    /// `doCreate` (L2657).
    Create,
    /// `doJoin` (L2617) / CANCEL while connecting (L2439).
    Join,
    /// BACK to the main menu (hub only).
    HubBack,
    /// `toggleReady` (L3048): host ⇒ `tryStart`.
    Ready,
    /// `requestTeam` (L3075).
    Team(online::TeamSel),
    /// `hostSet({duration})` (L2841).
    Duration(f64),
    /// `hostSet({bots})` toggle (L2848).
    Bots,
    /// `hostSet({difficulty})` cycle (L2850).
    Difficulty,
    /// Task 39: `setMe({weapon})` (session.js L200-206) — cycle the local
    /// player's weapon over `WEAPON_ORDER`. JS has no lobby loadout UI (the
    /// loadout screen drives the same call); the Lobby-state / `me`-row
    /// guards live in `online::run_cmd`.
    WeaponCycle,
    /// Task 39: `setMe({sub})` (session.js L202) — cycle over `SUB_IDS` with
    /// a NONE stop after the last entry.
    SubCycle,
    /// Task 39: `setMe({special})` (session.js L203) — cycle over
    /// `SPECIAL_IDS` with a NONE stop.
    SpecialCycle,
    /// Task 39: host `hostSet({mode})` toggle (session.js L244; Task 44
    /// batch D cycles turf → zones → boss → turf). Host-only guard in
    /// `run_cmd`.
    ModeToggle,
    /// `askLeave` two-step (L3026, deviation ⑦).
    Leave,
}

#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct UiAction(Action);

/// Kill-feed lines: (expiry time, text), newest first, 5 s dwell (upstream
/// hud.js:460 uses 4.2 s; rounded up for M1).
#[derive(Resource, Default)]
pub struct Feed {
    pub lines: Vec<(f32, String)>,
}

/// Clock expiry of the crosshair kill flash.
#[derive(Resource, Default)]
pub struct HitFlash {
    pub until: f32,
}

const PANEL_BG: Color = Color::srgba(0.02, 0.03, 0.05, 0.72);
const BTN_BG: Color = Color::srgba(0.10, 0.12, 0.16, 0.95);
const BTN_BG_HOVER: Color = Color::srgba(0.18, 0.21, 0.27, 0.95);
const BTN_BG_PRESS: Color = Color::srgba(0.30, 0.34, 0.42, 0.95);
const DIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.45);
const AMBER: Color = Color::srgb(1.0, 0.82, 0.10);
const RED: Color = Color::srgb(1.0, 0.36, 0.36);

/// Upstream `fmtTime` (ui-util.js L203-207): ceil to whole seconds, M:SS.
#[must_use]
pub fn fmt_clock(s: f32) -> String {
    let t = (s - 1e-6).max(0.0).ceil() as i64;
    format!("{}:{:02}", t / 60, t % 60)
}

/// Timer colour states (hud.js L1175-1178: `is-final` ≤ 10 s, `is-last`
/// ≤ 60 s).
#[must_use]
pub fn timer_color(t: f32) -> Color {
    if t <= 10.001 {
        RED
    } else if t <= 60.001 {
        AMBER
    } else {
        Color::WHITE
    }
}

/// `A1` / `B3` roster tag (team letter + 1-based slot; the sim has no names).
#[must_use]
pub fn slot_tag(team: usize, slot: usize) -> String {
    format!("{}{}", if team == 0 { 'A' } else { 'B' }, slot + 1)
}

/// One kill-feed line from a match event (upstream kill card: victim +
/// attacker; water deaths credit nobody, match.js L184 / actor.js L384).
#[must_use]
pub fn splat_text(ev: &MatchEvent) -> Option<String> {
    match ev {
        MatchEvent::Splat {
            victim_team,
            victim_slot,
            attacker,
            cause,
        } => {
            let v = slot_tag(*victim_team, *victim_slot);
            Some(match attacker {
                Some((at, as_)) => format!("{} > {}", slot_tag(*at, *as_), v),
                None if *cause == SplatCause::Water => format!("{v} fell in"),
                None => format!("{v} splatted"),
            })
        }
        _ => None,
    }
}

/// Coverage percent string (upstream `pct`, one decimal).
#[must_use]
pub fn pct(f: f32) -> String {
    format!("{:.1}%", (f * 100.0).min(100.0))
}

/// Kill-feed line for slot `i` (empty when the feed is shorter).
#[must_use]
fn feed_line(feed: &Feed, i: usize) -> String {
    feed.lines
        .get(i)
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
}

/// Team display names (config.js `TEAM_NAMES`; the embedded palette wins).
#[must_use]
pub fn team_names(t: &Tuning) -> [String; 2] {
    t.team_palettes
        .first()
        .map(|p| p.names.clone())
        .unwrap_or_else(|| ["Alpha".to_string(), "Bravo".to_string()])
}

/// Results headline + the two percent lines (hud.js L541/550-551). The sim's
/// `judge` breaks ties with the seeded RNG so `winner` is always 0/1 — no
/// "IT'S A TIE!" branch here (upstream hud.js L541 has one; recorded sim
/// deviation). Task 36 batch 3: a zones result runs the `_judgeZones` mirror
/// below instead (hud.js L1096-1126).
#[must_use]
pub fn results_text(res: &MatchResult, names: &[String; 2]) -> (String, String, String) {
    if let Some(b) = &res.boss {
        // Task 46 batch B/C2: Boss Battle settlement (JS `bossMode.js` L44-52
        // + `main.js` L960 `boss: {name, defeated, time, hpLeft, phase}` —
        // the summary lines mirror that record; the per-player bossDmg /
        // weakHits columns live in `stats_rows`, JS main.js L961 + menus.js
        // L4031-4040).
        let title = if b.win { "VICTORY!" } else { "DEFEAT" };
        return (
            title.to_string(),
            format!(
                "{} {:.0}/{:.0} HP · PHASE {}",
                b.name, b.hp, b.max_hp, b.phase
            ),
            format!("TIME {:.1}s", b.time),
        );
    }
    if let Some(z) = &res.zones {
        return zones_text(res.winner, z, names);
    }
    (
        format!("{} WINS!", names[res.winner].to_uppercase()),
        pct(res.coverage[0]),
        pct(res.coverage[1]),
    )
}

/// JS `_judgeZones` text mirror (hud.js L1096-1126, the static end state of
/// the reveal): the headline is the winner line (L1115) with the reason sub
/// of L1114 appended (`knockout` rides the KO stamp there — shown as
/// `KNOCKOUT!`); the two side lines are the final counts with the `whole()`
/// rounding (L1099: `max(0, ceil(v - 1e-6))`) and the penalty caption
/// (L1108-1110).
fn zones_text(winner: usize, z: &ZoneOutcome, names: &[String; 2]) -> (String, String, String) {
    let sub = match z.reason.as_deref() {
        Some("knockout") => " · KNOCKOUT!",
        Some("comeback") => " · OVERTIME COMEBACK!",
        Some("retake") => " · RETAKEN IN OVERTIME",
        Some("neutralised") => " · HELD ON THROUGH OVERTIME",
        Some("overtime-cap") => " · OVERTIME LIMIT",
        _ if z.overtime => " · OVERTIME",
        _ => "",
    };
    // JS `whole` (L1099): ceil minus a 1e-6 epsilon, floored at 0.
    let whole = |v: f64| (v - 1e-6).ceil().max(0.0) as i64;
    let line = |t: usize| {
        let cnt = whole(z.counts[t] as f64);
        let pen = whole(f64::from(z.penalty[t]));
        if pen > 0 {
            format!("{cnt}  COUNT LEFT · +{pen} PENALTY")
        } else {
            format!("{cnt}  COUNT LEFT")
        }
    };
    (
        format!("{} WINS!{sub}", names[winner].to_uppercase()),
        line(0),
        line(1),
    )
}

/// Results bar width in percent: turf coverage; a zones result runs the
/// `_judgeZones` fill (hud.js L1136: `p = (100 - cnt) / 100`) — the
/// counted-off progress of each team's final count.
fn res_bar(res: &MatchResult, t: usize) -> f32 {
    match &res.zones {
        Some(z) => (100.0 - z.counts[t].clamp(0, 100) as f32).clamp(0.0, 100.0),
        None => res.coverage[t] * 100.0,
    }
}

/// Per-actor results rows: tag, splats, deaths, turf m² (no trailing
/// newline). Task 46 batch C2: in a boss match the rows mirror the JS boss
/// results table (main.js L961 `damage/weakHits/splats`, menus.js L4031-4040
/// DAMAGE / weak hits / splats(crablets) / deaths / TURF) fed from
/// `Boss::stats_by_nid` — offline the squad `ids` are roster indices
/// (match_.rs L1432), so the key is the actor slot — plus a totals line.
#[must_use]
pub fn stats_rows(demo: &DemoSim) -> String {
    let bm = demo.bm.lock().unwrap();
    let boss = bm.as_ref();
    let mut s = String::new();
    for (i, a) in demo.m.actors.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        match boss {
            Some(b) => {
                let st = b.boss.stats_by_nid.get(&(a.slot as i32));
                s.push_str(&format!(
                    "{}  dmg {:.0}  weak {}  splats {}  deaths {}  turf {:.0}m2",
                    slot_tag(a.team, a.slot),
                    f64::from(st.map_or(0.0, |x| x.boss_dmg)),
                    st.map_or(0, |x| x.weak_hits),
                    st.map_or(0, |x| x.splats),
                    a.deaths,
                    a.turf,
                ));
            }
            None => {
                s.push_str(&format!(
                    "{}  splats {}  deaths {}  turf {:.0}m2",
                    slot_tag(a.team, a.slot),
                    a.splats,
                    a.deaths,
                    a.turf
                ));
            }
        }
    }
    if let Some(b) = boss {
        // Totals over the per-attacker table (JS sums the DAMAGE / weak-hit
        // columns across the roster).
        let (dmg, weak) = b.boss.stats_by_nid.values().fold((0.0f32, 0u32), |acc, x| {
            (acc.0 + x.boss_dmg, acc.1 + x.weak_hits)
        });
        s.push_str(&format!("\nBOSS DMG {:.0}  WEAK {weak}", f64::from(dmg)));
    }
    s
}

// ---------------------------------------------------------------- build

fn full_panel(screen: Screen, bg: Color) -> (Node, BackgroundColor, ScreenRoot) {
    (
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(14.0),
            ..default()
        },
        BackgroundColor(bg),
        ScreenRoot(screen),
    )
}

fn col(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(gap),
        ..default()
    }
}

fn row(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(gap),
        ..default()
    }
}

fn label(text: impl Into<String>, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(text.into()),
        TextFont::from_font_size(size),
        TextColor(color),
    )
}

/// Spawn a button with a centred label child under `parent`.
fn button(commands: &mut Commands, parent: Entity, text: &str, action: Action) {
    button_dyn(commands, parent, text, action, None);
}

/// [`button`] with an optional [`LabelRole`] on the label so `ui_update`
/// (or `online_ui_update`) can rewrite it (JOIN/CANCEL, READY?/START!).
fn button_dyn(
    commands: &mut Commands,
    parent: Entity,
    text: &str,
    action: Action,
    role: Option<&'static str>,
) {
    let btn = commands
        .spawn((
            Button,
            UiAction(action),
            Node {
                width: Val::Px(240.0),
                height: Val::Px(52.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_BG),
            ChildOf(parent),
        ))
        .id();
    let lbl = commands
        .spawn((label(text, 20.0, Color::WHITE), ChildOf(btn)))
        .id();
    if let Some(r) = role {
        commands.entity(lbl).insert(LabelRole(r));
    }
}

/// Compact button for the lobby settings rows (JS `_seg`/`_toggle` are much
/// smaller than the full-width menu buttons).
fn small_button(commands: &mut Commands, parent: Entity, text: &str, action: Action) {
    let btn = commands
        .spawn((
            Button,
            UiAction(action),
            Node {
                width: Val::Px(110.0),
                height: Val::Px(36.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_BG),
            ChildOf(parent),
        ))
        .id();
    commands.spawn((label(text, 15.0, Color::WHITE), ChildOf(btn)));
}

/// Compact button with a [`LabelRole`] on the label so `online_ui_update` can
/// rewrite its text (Task 39 lobby loadout / mode row).
fn small_button_dyn(
    commands: &mut Commands,
    parent: Entity,
    text: &str,
    action: Action,
    role: &'static str,
) {
    let btn = commands
        .spawn((
            Button,
            UiAction(action),
            Node {
                width: Val::Px(150.0),
                height: Val::Px(36.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_BG),
            ChildOf(parent),
        ))
        .id();
    commands.spawn((
        label(text, 15.0, Color::WHITE),
        LabelRole(role),
        ChildOf(btn),
    ));
}

/// Build all four screen trees (only one is displayed at a time).
pub fn build_ui(mut commands: Commands) {
    // Task 46 batch C2: the boss HUD / bossAudio state machines live with
    // the UI (registered here so both the real app and the headless test
    // apps get them).
    commands.init_resource::<BossHudRes>();
    // Task 46: placeholder defeat quake (boss.js L182) — same registration
    // surface as `BossHudRes` so test apps get it too.
    commands.init_resource::<crate::boss_render::BossShake>();
    // ================= main menu =================
    let menu = commands.spawn(full_panel(Screen::Menu, PANEL_BG)).id();
    let menu_col = commands.spawn((col(18.0), ChildOf(menu))).id();
    commands.spawn((label("INKWAVE", 64.0, Color::WHITE), ChildOf(menu_col)));
    commands.spawn((
        label(
            "Rust port M1 - 4v4 turf war",
            18.0,
            Color::srgba(1.0, 1.0, 1.0, 0.7),
        ),
        ChildOf(menu_col),
    ));
    button(&mut commands, menu_col, "PLAY 90s", Action::Play(90));
    button(&mut commands, menu_col, "PLAY 180s", Action::Play(180));
    // Task 46 batch B (D-T44-5): offline mode selector (turf → boss). The
    // dynamic label is refreshed by `ui_update`'s "menu_mode" role.
    small_button_dyn(
        &mut commands,
        menu_col,
        "MODE TURF",
        Action::ModeCycle,
        "menu_mode",
    );
    // Task 24: the ONLINE entry (JS main menu's ONLINE button, menus.js
    // L2407 "the main menu's ONLINE button is --b too").
    button(
        &mut commands,
        menu_col,
        "ONLINE",
        Action::Online(OnlineAction::EnterHub),
    );
    // Spec: main menu "退出". A browser tab has no window to close, so the
    // button only exists on native.
    #[cfg(not(target_arch = "wasm32"))]
    button(&mut commands, menu_col, "QUIT", Action::Quit);
    commands.spawn((
        label(
            "WASD move - Shift squid - Space jump - LMB fire - RMB/E sub - F/Q special - G mouse-look - Esc/P pause",
            15.0,
            DIM,
        ),
        ChildOf(menu_col),
    ));

    // ================= live HUD =================
    let hud = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            ScreenRoot(Screen::Hud),
        ))
        .id();

    // timer: top centre (upstream `_updTimer` states).
    let timer_panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            ChildOf(hud),
        ))
        .id();
    commands.spawn((
        (
            LabelRole("timer"),
            Text::new("3:00"),
            TextFont::from_font_size(34.0),
            TextColor(Color::WHITE),
        ),
        ChildOf(timer_panel),
    ));

    // Task 46 batch C2 — boss HUD placeholder档 (≡ hud-boss.js `_mount`):
    // the bar block under the timer, the callout pill, the title card, the
    // phase banner and the ending word. Non-boss matches leave every role
    // empty / zero-width (ui_update guards on `BossHudRes`).
    let boss_panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(52.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
            ChildOf(hud),
        ))
        .id();
    commands.spawn((
        (
            LabelRole("boss_bar"),
            Text::new(""),
            TextFont::from_font_size(18.0),
            TextColor(Color::WHITE),
        ),
        ChildOf(boss_panel),
    ));
    let boss_bar_bg = commands
        .spawn((
            Node {
                width: Val::Px(360.0),
                height: Val::Px(16.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            ChildOf(boss_panel),
        ))
        .id();
    // JS `_writeBar`: the white chip lag rides behind the coloured fill.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(0.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        BarRole("boss_chip_bar"),
        ChildOf(boss_bar_bg),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(0.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.85, 0.25, 0.2)),
        BarRole("boss_hp_bar"),
        ChildOf(boss_bar_bg),
    ));
    // JS `_stun` is-open meter (hud-boss.js L237 / L321).
    let boss_stun_bg = commands
        .spawn((
            Node {
                width: Val::Px(200.0),
                height: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            ChildOf(boss_panel),
        ))
        .id();
    commands.spawn((
        Node {
            width: Val::Percent(0.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgb(1.0, 0.8, 0.2)),
        BarRole("boss_stun_bar"),
        ChildOf(boss_stun_bg),
    ));
    commands.spawn((
        (
            LabelRole("boss_open"),
            Text::new(""),
            TextFont::from_font_size(22.0),
            TextColor(Color::srgb(1.0, 0.8, 0.2)),
        ),
        ChildOf(boss_panel),
    ));
    // JS callout pill (hud-boss.js L183-205) — centred upper third.
    commands.spawn((
        (
            LabelRole("boss_call"),
            Text::new(""),
            TextFont::from_font_size(30.0),
            TextColor(Color::srgb(1.0, 0.9, 0.3)),
        ),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(30.0),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        ChildOf(hud),
    ));
    // JS `_titleCard` (L444-461) / `_banner` (L463-472) / `_ending` (L474-494)
    // — stacked centred overlays.
    for (role, size, top) in [
        ("boss_title", 44.0, 18.0),
        ("boss_banner", 34.0, 34.0),
        ("boss_ending", 52.0, 46.0),
    ] {
        commands.spawn((
            (
                LabelRole(role),
                Text::new(""),
                TextFont::from_font_size(size),
                TextColor(Color::WHITE),
            ),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(top),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            ChildOf(hud),
        ));
    }

    // turf percent + bar, top-left / top-right (upstream zone-control-free
    // turf HUD: two team counters with the leader highlighted).
    for (team, side_left) in [(0usize, true), (1, false)] {
        let p = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(10.0),
                    left: if side_left { Val::Px(16.0) } else { Val::Auto },
                    right: if side_left { Val::Auto } else { Val::Px(16.0) },
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
                ChildOf(hud),
            ))
            .id();
        let role_txt = if team == 0 { "cov_a" } else { "cov_b" };
        let role_bar = if team == 0 { "cov_a_bar" } else { "cov_b_bar" };
        commands.spawn((
            (
                LabelRole(role_txt),
                Text::new("0.0%"),
                TextFont::from_font_size(22.0),
                TextColor(team_color(team)),
            ),
            ChildOf(p),
        ));
        let bar_bg = commands
            .spawn((
                Node {
                    width: Val::Px(240.0),
                    height: Val::Px(14.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                ChildOf(p),
            ))
            .id();
        commands.spawn((
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(team_color(team)),
            BarRole(role_bar),
            ChildOf(bar_bg),
        ));
    }

    // ink tank: bottom-left bar (upstream sloshing canvas tank -> M1 bar).
    let ink_panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(16.0),
                left: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            },
            ChildOf(hud),
        ))
        .id();
    commands.spawn((label("INK", 14.0, DIM), ChildOf(ink_panel)));
    let ink_bg = commands
        .spawn((
            Node {
                width: Val::Px(240.0),
                height: Val::Px(18.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            ChildOf(ink_panel),
        ))
        .id();
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(team_color(0)),
        BarRole("ink_bar"),
        ChildOf(ink_bg),
    ));

    // crosshair: centred plus (upstream per-weapon reticle -> M1 minimal).
    let xh = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                ..default()
            },
            ChildOf(hud),
        ))
        .id();
    for (w, h, x, y) in [(2.0, 22.0, -1.0, -11.0), (22.0, 2.0, -11.0, -1.0)] {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x),
                top: Val::Px(y),
                width: Val::Px(w),
                height: Val::Px(h),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            Crosshair,
            ChildOf(xh),
        ));
    }

    // kill feed: bottom-right, four slots.
    let feed_panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(16.0),
                right: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::End,
                row_gap: Val::Px(4.0),
                ..default()
            },
            ChildOf(hud),
        ))
        .id();
    for i in 0..4 {
        commands.spawn((
            (
                LabelRole(match i {
                    0 => "feed0",
                    1 => "feed1",
                    2 => "feed2",
                    _ => "feed3",
                }),
                Text::new(""),
                TextFont::from_font_size(16.0),
                TextColor(Color::WHITE),
            ),
            ChildOf(feed_panel),
        ));
    }

    // ================= pause overlay =================
    let pause = commands.spawn(full_panel(Screen::Pause, PANEL_BG)).id();
    let pause_col = commands.spawn((col(14.0), ChildOf(pause))).id();
    commands.spawn((label("PAUSED", 44.0, Color::WHITE), ChildOf(pause_col)));
    button(&mut commands, pause_col, "RESUME", Action::Resume);
    button(&mut commands, pause_col, "RESTART", Action::Restart);
    button(&mut commands, pause_col, "MAIN MENU", Action::Menu);
    #[cfg(not(target_arch = "wasm32"))]
    button(&mut commands, pause_col, "QUIT", Action::Quit);

    // ================= results =================
    let res = commands.spawn(full_panel(Screen::Results, PANEL_BG)).id();
    let res_col = commands.spawn((col(12.0), ChildOf(res))).id();
    commands.spawn((
        (
            LabelRole("res_title"),
            Text::new(""),
            TextFont::from_font_size(44.0),
            TextColor(Color::WHITE),
        ),
        ChildOf(res_col),
    ));
    // coverage bar pair (upstream judge side-by-side percents; M1 draws two
    // independent percent bars growing from the row edges instead of the
    // upstream single-track share bars — same numbers, simpler layout).
    let cov_row = commands
        .spawn((
            (
                Node {
                    width: Val::Px(520.0),
                    height: Val::Px(18.0),
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            ),
            ChildOf(res_col),
        ))
        .id();
    commands.spawn((
        Node {
            width: Val::Percent(0.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(team_color(0)),
        BarRole("res_a_bar"),
        ChildOf(cov_row),
    ));
    commands.spawn((
        Node {
            width: Val::Percent(0.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(team_color(1)),
        BarRole("res_b_bar"),
        ChildOf(cov_row),
    ));
    let pct_row = commands.spawn((row(24.0), ChildOf(res_col))).id();
    for role in ["res_a", "res_b"] {
        commands.spawn((
            (
                LabelRole(if role == "res_a" { "res_a" } else { "res_b" }),
                Text::new("0.0%"),
                TextFont::from_font_size(24.0),
                TextColor(if role == "res_a" {
                    team_color(0)
                } else {
                    team_color(1)
                }),
            ),
            ChildOf(pct_row),
        ));
    }
    commands.spawn((
        (
            LabelRole("res_stats"),
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        ),
        ChildOf(res_col),
    ));
    button_dyn(
        &mut commands,
        res_col,
        "PLAY AGAIN",
        Action::Restart,
        Some("res_again"),
    );
    button(&mut commands, res_col, "MAIN MENU", Action::Menu);

    // ================= online hub (Task 24, `_scr_online`) =================
    let hub = commands.spawn(full_panel(Screen::OnlineHub, PANEL_BG)).id();
    let hub_col = commands.spawn((col(14.0), ChildOf(hub))).id();
    commands.spawn((label("ONLINE", 44.0, Color::WHITE), ChildOf(hub_col)));
    commands.spawn((
        label("Private rooms · 4 v 4 · up to 8 squidkids", 16.0, DIM),
        ChildOf(hub_col),
    ));
    // the five code cells (L2424-2433: one box per character)
    let code_row = commands.spawn((row(10.0), ChildOf(hub_col))).id();
    for i in 0..5 {
        let role = match i {
            0 => "hub_c0",
            1 => "hub_c1",
            2 => "hub_c2",
            3 => "hub_c3",
            _ => "hub_c4",
        };
        commands.spawn((
            (
                LabelRole(role),
                Text::new(" "),
                TextFont::from_font_size(40.0),
                TextColor(Color::WHITE),
            ),
            ChildOf(code_row),
        ));
    }
    commands.spawn((
        (
            LabelRole("hub_hint"),
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(DIM),
        ),
        ChildOf(hub_col),
    ));
    commands.spawn((
        (
            LabelRole("hub_err"),
            Text::new(""),
            TextFont::from_font_size(18.0),
            TextColor(RED),
        ),
        ChildOf(hub_col),
    ));
    button(
        &mut commands,
        hub_col,
        "CREATE ROOM",
        Action::Online(OnlineAction::Create),
    );
    // `createStatus` (doCreate L2665/L2680): the create card's own line —
    // "Opening a room" while in flight, the rejection text on failure.
    commands.spawn((
        (
            LabelRole("hub_create"),
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(DIM),
        ),
        ChildOf(hub_col),
    ));
    button_dyn(
        &mut commands,
        hub_col,
        "JOIN",
        Action::Online(OnlineAction::Join),
        Some("hub_join"),
    );
    button(
        &mut commands,
        hub_col,
        "BACK",
        Action::Online(OnlineAction::HubBack),
    );
    commands.spawn((
        label(
            "A-Z / 0-9 type the code · Backspace edit · Enter join · Esc back",
            14.0,
            DIM,
        ),
        ChildOf(hub_col),
    ));

    // ================= lobby (Task 24, `_scr_lobby`) =================
    let lob = commands
        .spawn(full_panel(Screen::OnlineLobby, PANEL_BG))
        .id();
    let lob_col = commands.spawn((col(10.0), ChildOf(lob))).id();
    // room code big tiles (L2793)
    commands.spawn((
        (
            LabelRole("lob_code"),
            Text::new(""),
            TextFont::from_font_size(34.0),
            TextColor(AMBER),
        ),
        ChildOf(lob_col),
    ));
    commands.spawn((
        (
            LabelRole("lob_count"),
            Text::new("SQUIDKIDS 1/8"),
            TextFont::from_font_size(16.0),
            TextColor(DIM),
        ),
        ChildOf(lob_col),
    ));
    // eight fixed roster slots (L2922-2928 `open[k]`)
    for i in 0..8 {
        let role = match i {
            0 => "lob_row0",
            1 => "lob_row1",
            2 => "lob_row2",
            3 => "lob_row3",
            4 => "lob_row4",
            5 => "lob_row5",
            6 => "lob_row6",
            _ => "lob_row7",
        };
        commands.spawn((
            (
                LabelRole(role),
                Text::new("OPEN"),
                TextFont::from_font_size(18.0),
                TextColor(Color::WHITE),
            ),
            ChildOf(lob_col),
        ));
    }
    commands.spawn((
        (
            LabelRole("lob_status"),
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(DIM),
        ),
        ChildOf(lob_col),
    ));
    // team row (L2901: ALPHA | AUTO | BRAVO)
    let team_row = commands.spawn((row(8.0), ChildOf(lob_col))).id();
    small_button(
        &mut commands,
        team_row,
        "ALPHA",
        Action::Online(OnlineAction::Team(online::TeamSel::Zero)),
    );
    small_button(
        &mut commands,
        team_row,
        "AUTO",
        Action::Online(OnlineAction::Team(online::TeamSel::Auto)),
    );
    small_button(
        &mut commands,
        team_row,
        "BRAVO",
        Action::Online(OnlineAction::Team(online::TeamSel::One)),
    );
    // READY?/READY!/START! (L3354: the host's button reads START!)
    button_dyn(
        &mut commands,
        lob_col,
        "READY?",
        Action::Online(OnlineAction::Ready),
        Some("lob_ready"),
    );
    // host settings row (L2841/L2848/L2850)
    let set_row = commands.spawn((row(8.0), ChildOf(lob_col))).id();
    small_button(
        &mut commands,
        set_row,
        "LEN 90",
        Action::Online(OnlineAction::Duration(90.0)),
    );
    small_button(
        &mut commands,
        set_row,
        "LEN 180",
        Action::Online(OnlineAction::Duration(180.0)),
    );
    small_button(
        &mut commands,
        set_row,
        "BOTS",
        Action::Online(OnlineAction::Bots),
    );
    small_button(
        &mut commands,
        set_row,
        "SKILL",
        Action::Online(OnlineAction::Difficulty),
    );
    // Task 39: own loadout cycle + host mode toggle (JS has no lobby
    // loadout/mode UI — these buttons drive the same `setMe` / `hostSet`
    // session calls the loadout screen and host controls make).
    let load_row = commands.spawn((row(8.0), ChildOf(lob_col))).id();
    small_button_dyn(
        &mut commands,
        load_row,
        "WPN",
        Action::Online(OnlineAction::WeaponCycle),
        "lob_weapon",
    );
    small_button_dyn(
        &mut commands,
        load_row,
        "SUB",
        Action::Online(OnlineAction::SubCycle),
        "lob_sub",
    );
    small_button_dyn(
        &mut commands,
        load_row,
        "SPC",
        Action::Online(OnlineAction::SpecialCycle),
        "lob_special",
    );
    small_button_dyn(
        &mut commands,
        load_row,
        "MODE",
        Action::Online(OnlineAction::ModeToggle),
        "lob_mode",
    );
    commands.spawn((
        (
            LabelRole("lob_settings"),
            Text::new(""),
            TextFont::from_font_size(15.0),
            TextColor(DIM),
        ),
        ChildOf(lob_col),
    ));
    button(
        &mut commands,
        lob_col,
        "LEAVE",
        Action::Online(OnlineAction::Leave),
    );
    // banners: the lobby toast (waiting-for / start block) and the two-step
    // leave confirm (deviation ⑦).
    commands.spawn((
        (
            LabelRole("lob_banner"),
            Text::new(""),
            TextFont::from_font_size(18.0),
            TextColor(AMBER),
        ),
        ChildOf(lob_col),
    ));
    commands.spawn((
        (
            LabelRole("lob_leave"),
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(RED),
        ),
        ChildOf(lob_col),
    ));
}

// ---------------------------------------------------------------- logic

/// Apply one UI action (pure transition; also drives the Task 12 pause flag).
/// `quit` is set when the action asks the app to exit (native QUIT button).
pub fn apply_action(
    st: &mut UiState,
    pc: &mut PlayerControls,
    a: Action,
    demo: &mut DemoSim,
    quit: &mut bool,
) {
    match a {
        Action::Play(d) => {
            if st.net_mode {
                // Task 24: the online flow owns the sim; offline Play is
                // refused (JS hides the menu during a net match).
                return;
            }
            st.duration = d;
            match st.play_mode {
                // Task 46: boss clock = BOSS_MODE.duration (JS match.js L21
                // — the lobby time is ignored); difficulty defaults to
                // "normal" (JS bossMode.js L23 `o.difficulty || 'normal'`),
                // there is no offline difficulty selector.
                PlayMode::Boss => demo.replay_boss(0.0, "normal"),
                PlayMode::Turf => demo.replay(d as f32),
            }
            st.screen = Screen::Hud;
        }
        Action::Resume => st.screen = Screen::Hud,
        Action::Restart => {
            if st.net_mode {
                return; // the net Results button is remapped to
                // `NetBackToLobby` by `ui_state_machine`
            }
            let d = st.duration;
            match st.play_mode {
                PlayMode::Boss => demo.replay_boss(0.0, "normal"),
                PlayMode::Turf => demo.replay(d as f32),
            }
            st.screen = Screen::Hud;
        }
        Action::Menu => st.screen = Screen::Menu,
        Action::Quit => *quit = true,
        // Task 46: the offline mode cycle (turf → boss).
        Action::ModeCycle => {
            st.play_mode = match st.play_mode {
                PlayMode::Turf => PlayMode::Boss,
                PlayMode::Boss => PlayMode::Turf,
            };
        }
        // Task 24: the online buttons drive the session resource, which this
        // pure helper has no access to; `ui_state_machine` dispatches them
        // through `handle_online_action` instead.
        Action::Online(_) => {}
        // Results "BACK TO LOBBY" (JS `netMatchEnd` re-shows the lobby,
        // main.js L769-846).
        Action::NetBackToLobby => st.screen = Screen::OnlineLobby,
    }
    pc.paused = st.screen != Screen::Hud;
}

/// State machine: pause edge / Enter shortcuts / button clicks, plus the
/// event drain feeding the kill feed and the results transition.
///
/// Task 24: in online mode (`st.net_mode`) the same machine also pumps the
/// [`OnlineSession`], keeps the screen in step with the session state (JS
/// `_onNetState` L2359-2368) and routes the hub/lobby keys + buttons.
#[allow(clippy::too_many_arguments)]
pub fn ui_state_machine(
    mut st: ResMut<UiState>,
    mut pc: ResMut<PlayerControls>,
    mut demo: ResMut<DemoSim>,
    keys: Res<ButtonInput<KeyCode>>,
    mut clicks: MessageReader<Pointer<Click>>,
    actions: Query<&UiAction>,
    mut feed: ResMut<Feed>,
    mut flash: ResMut<HitFlash>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut exits: MessageWriter<AppExit>,
    time: Res<Time>,
    mut os: NonSendMut<OnlineSession>,
    mut net_ev: ResMut<NetEvents>,
    mut boss: ResMut<BossHudRes>,
    mut shake: ResMut<crate::boss_render::BossShake>,
) {
    let prev = st.screen;
    let mut quit = false;
    // Deref the `NonSendMut` once so the online paths can take disjoint
    // field borrows (`&mut os.sess` + `&mut os.flags`) behind `run_cmd`.
    let os = &mut *os;

    if st.net_mode {
        // Pump the session (JS `Net` is socket-callback driven; the Rust
        // `NetSession` needs `update` + `poll_events` every frame) and
        // settle the in-flight flags when a connect resolves or is rejected
        // (`tok === st.token` arms, menus.js L2637/L2643/L2669/L2674).
        os.sess.update(f64::from(time.delta_secs().min(0.1)));
        // `poll_events` is drained exactly once here (plan risk R1); the
        // match-driving events are staged for `net_tick`, which runs later
        // in the chain and owns the replication controller + transport.
        for ev in os.sess.poll_events() {
            if let SessionEvent::State { state } = ev
                && state != SessionState::Connecting
            {
                // A rejection carries the session error so `settle` can route
                // it (join ⇒ jstat panel L2607-2616, create ⇒ create card
                // L2673-2682, mid-match ⇒ the `_roomGone` toast arm).
                let err = if state == SessionState::Error {
                    os.sess.error().map(str::to_string)
                } else {
                    None
                };
                os.flags.settle(err.as_deref());
            }
            net_ev.0.push(ev);
        }
        // Screen follows the session (JS `_onNetState` L2359-2368 +
        // `leaveRoom` → `show('online')` L3045): hub ↔ lobby ↔ in-match.
        match online::project(&online_facts(os)).screen {
            online::OnlineScreen::Hub => {
                if st.screen == Screen::OnlineLobby {
                    st.screen = Screen::OnlineHub;
                }
            }
            online::OnlineScreen::Lobby | online::OnlineScreen::MatchLoading => {
                if st.screen == Screen::OnlineHub {
                    st.screen = Screen::OnlineLobby;
                }
            }
            online::OnlineScreen::Match => {
                if matches!(st.screen, Screen::OnlineHub | Screen::OnlineLobby) {
                    st.screen = Screen::Hud;
                }
            }
        }
        // `st.autoT` (menus.js L2554): the 320 ms auto-join fires only while
        // the hub is still in entry mode with a full code — the JS callback
        // guard `st.alive && st.mode === 'entry' && full()`. Cancel does not
        // clear the timer (JS quirk: it re-checks and re-joins, L2554).
        if st.screen == Screen::OnlineHub && st.auto_join > 0.0 {
            st.auto_join -= f64::from(time.delta_secs());
            if st.auto_join <= 0.0 {
                st.auto_join = 0.0;
                if !os.flags.busy && online::code_full(&os.flags.code_draft) {
                    run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Join);
                }
            }
        }
    }

    // Esc/P edge from `map_input` (TR-12.1): Hud <-> Pause.
    if std::mem::take(&mut pc.pause_edge) {
        match st.screen {
            Screen::Hud => st.screen = Screen::Pause,
            Screen::Pause => st.screen = Screen::Hud,
            // Task 24: Esc on the hub leaves online mode; while connecting
            // it cancels the connect instead (onKey L2725).
            Screen::OnlineHub => {
                if os.flags.busy {
                    run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Cancel);
                } else {
                    // Leaving the hub destroys the JS screen (`st.alive`
                    // permanently drops the pending auto-join, L2554).
                    st.auto_join = 0.0;
                    st.net_mode = false;
                    st.screen = Screen::Menu;
                }
            }
            // Esc on the lobby is the two-step leave (askLeave L3026,
            // deviation ⑦); ignored while launching (L3534 `!S.launching`).
            Screen::OnlineLobby if os.sess.state() == SessionState::Lobby => {
                if os.flags.leave_confirm {
                    run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Leave);
                } else {
                    os.flags.leave_confirm = true;
                }
            }
            _ => {}
        }
        println!("[inkwave] ui {:?}", st.screen);
    }
    // Enter shortcuts (keyboard path for TR-13.1 without the mouse).
    if keys.just_pressed(KeyCode::Enter) {
        match st.screen {
            // Spec: main-menu Play starts the default 180 s directly (not the
            // last picked duration).
            Screen::Menu => apply_action(
                &mut st,
                &mut pc,
                Action::Play(DEFAULT_DURATION),
                &mut demo,
                &mut quit,
            ),
            Screen::Pause => apply_action(&mut st, &mut pc, Action::Resume, &mut demo, &mut quit),
            Screen::Results => {
                if st.net_mode {
                    // net Results: Enter = BACK TO LOBBY (`netMatchEnd`).
                    // JS `netBackToLobby` (main.js L383) is host-only — a
                    // guest stays until the host's `{k:'end'}` arrives.
                    if os.sess.is_host() {
                        st.net_back_pressed = true;
                        apply_action(
                            &mut st,
                            &mut pc,
                            Action::NetBackToLobby,
                            &mut demo,
                            &mut quit,
                        );
                    }
                } else {
                    apply_action(&mut st, &mut pc, Action::Restart, &mut demo, &mut quit);
                }
            }
            Screen::Hud => {}
            // Task 24: Enter on the hub joins (onKey L2733 `doJoin`), on the
            // lobby it toggles ready (the READY? button's accept).
            // `doJoin` opens with `clearTimeout(st.autoT)` (L2619) even on
            // the short-code early return.
            Screen::OnlineHub => {
                st.auto_join = 0.0;
                run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Join);
            }
            Screen::OnlineLobby => run_cmd(&mut os.sess, &mut os.flags, SessionCmd::ToggleReady),
        }
    }
    // Task 24: R toggles ready on the lobby (keyboard ready shortcut).
    if st.screen == Screen::OnlineLobby && keys.just_pressed(KeyCode::KeyR) {
        run_cmd(&mut os.sess, &mut os.flags, SessionCmd::ToggleReady);
    }
    // Task 24: hub code entry (onKey L2722-2745): while connecting only
    // Escape/Backspace get through (cancelConnect); otherwise alphabet keys
    // type and Backspace edits.
    if st.screen == Screen::OnlineHub {
        if os.flags.busy {
            if keys.just_pressed(KeyCode::Backspace) {
                run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Cancel);
            }
        } else {
            for kc in keys.get_just_pressed() {
                if let Some(c) = key_to_code_char(*kc) {
                    // `type()` opens with `clearError()` (L2540): the jstat
                    // panel drops and the hint line returns to its default
                    // (render L2526) unless the char is rejected.
                    os.flags.err_panel = false;
                    let fb = online::code_type(c, &mut os.flags.code_draft, &mut os.flags.caret);
                    // `type()` L2554: the fifth valid char landing in the last
                    // cell (re-)arms the 320 ms auto-join (`st.autoT`); an
                    // invalid char early-returns at L2546 and never touches
                    // the timer (JS quirk kept verbatim).
                    if fb.is_none()
                        && online::auto_join_armed(&os.flags.code_draft, os.flags.caret, c)
                    {
                        st.auto_join = 0.32;
                    }
                    os.flags.hint = fb.map(str::to_string);
                }
            }
            if keys.just_pressed(KeyCode::Backspace) {
                // `backspace()` opens with `clearError()` too (L2558) and
                // cancels the pending auto-join (L2559).
                os.flags.err_panel = false;
                st.auto_join = 0.0;
                online::code_backspace(&mut os.flags.code_draft, &mut os.flags.caret);
                os.flags.hint = None;
            }
        }
    }
    // Button clicks.
    for ev in clicks.read() {
        if ev.event.button != PointerButton::Primary {
            continue;
        }
        if let Some(a) = actions.get(ev.entity).ok().copied() {
            match a.0 {
                // Task 24: online buttons drive the session, not the
                // offline screen transitions.
                Action::Online(oa) => handle_online_action(&mut st, oa, &mut *os),
                // In online mode MAIN MENU leaves the room first (leaveRoom
                // L3040-3046) and the Results PLAY AGAIN reads BACK TO
                // LOBBY (label rewritten by `ui_update`).
                Action::Menu if st.net_mode => {
                    run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Leave);
                    st.net_mode = false;
                    st.screen = Screen::Menu;
                }
                Action::Restart if st.net_mode && st.screen == Screen::Results => {
                    // Host-only, exactly like JS `netBackToLobby` L383.
                    if os.sess.is_host() {
                        st.net_back_pressed = true;
                        apply_action(
                            &mut st,
                            &mut pc,
                            Action::NetBackToLobby,
                            &mut demo,
                            &mut quit,
                        );
                    }
                }
                other => apply_action(&mut st, &mut pc, other, &mut demo, &mut quit),
            }
            println!("[inkwave] ui click {:?} -> {:?}", a.0, st.screen);
        }
    }

    // Match end -> results (replaces the old auto-restart demo loop).
    if st.screen == Screen::Hud && demo.m.phase == Phase::End {
        st.screen = Screen::Results;
        println!("[inkwave] ui results");
    }

    // Drain match events into the feed + kill flash.
    let now = time.elapsed_secs();
    for ev in demo.m.events.drain(..) {
        if let Some(line) = splat_text(&ev) {
            feed.lines.insert(0, (now + 5.0, line));
        }
        if let MatchEvent::Splat {
            attacker: Some((0, 0)),
            ..
        } = ev
        {
            flash.until = now + 0.15;
        }
    }
    feed.lines.retain(|(exp, _)| *exp > now);
    feed.lines.truncate(4);

    // Task 46 batch C2 — drain the boss event queue into the pure state
    // machines (≡ the hud-boss.js / bossAudio.js bus subscriptions). Offline
    // the stepper fills `DemoSim::boss_events`; `net_tick` mirrors the client
    // queue in. The move callouts / audio loops ride the live `boss.st`
    // phase edges (the sim raises one `Move` per move record, not per phase).
    {
        // Take first: the `bm` mutex guard would keep `demo` immutably
        // borrowed and block the queue drain.
        let events: Vec<BossEvent> = std::mem::take(&mut demo.boss_events);
        // JS boss.js L182: defeat → `emit('shake', {amount: 0.9})` — the
        // placeholder quake consumed by `camera_rig` (Task 51 replaces it
        // with directional screenfx).
        if events.iter().any(|e| matches!(e, BossEvent::Defeat { .. })) {
            shake.amount = 0.9;
        }
        let bm = demo.bm.lock().unwrap();
        let boss_live = bm.is_some();
        if boss_live != boss.hud.is_some() {
            // JS `setMode(true, boss)` (hud-boss.js L100-117) + the app's
            // `bossAudio.begin()` (bossMode.js L13).
            boss.hud = bm
                .as_ref()
                .map(|m| BossHudState::new(m.boss.max_hp, m.boss.hp));
            boss.audio = BossAudioDirector::default();
            if boss_live {
                boss.audio.begin();
            }
            boss.out = BossHudOut::default();
        }
        if let (Some(hud), Some(bm)) = (boss.hud.as_mut(), bm.as_ref()) {
            let b = &bm.boss;
            for ev in &events {
                hud.on_event(ev);
            }
            let dt = f64::from(time.delta_secs().min(0.1));
            hud.update(
                dt,
                &BossHudFrame {
                    hp: b.hp,
                    max_hp: b.max_hp,
                    dead: b.dead,
                    stunned: b.stunned,
                    invuln: b.invuln,
                    move_id: b.st.move_id.as_deref(),
                    move_phase: b.st.move_phase.as_deref(),
                    hud_visible: st.screen == Screen::Hud,
                    finished: demo.m.phase == Phase::End,
                },
            );
            boss.out = hud.out();
            boss.audio.update(
                dt,
                &events,
                &BossAudioFrame {
                    move_id: b.st.move_id.as_deref(),
                    move_phase: b.st.move_phase.as_deref(),
                    playing: demo.m.phase == Phase::Active,
                    ended: demo.m.phase == Phase::End,
                },
            );
        }
    }

    pc.paused = st.screen != Screen::Hud;

    // Leaving the Hud releases the grabbed cursor so menu buttons are
    // clickable (upstream pointer-lock releases on pause too).
    if prev == Screen::Hud && st.screen != Screen::Hud {
        pc.look_enabled = false;
        if let Ok(mut c) = cursor.single_mut() {
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
    }

    // Entering the Hud re-acquires mouse look (upstream re-locks the pointer
    // when play resumes). Clearing the intent drops the click that pressed
    // RESUME/PLAY — `map_input` already ran this frame and would otherwise
    // fire one shot on the resume frame.
    if st.screen == Screen::Hud && prev != Screen::Hud {
        pc.look_enabled = true;
        pc.intent = ActorInput::default();
        if let Ok(mut c) = cursor.single_mut() {
            c.grab_mode = CursorGrabMode::Confined;
            c.visible = false;
        }
    }

    if quit {
        exits.write(AppExit::Success);
    }
}

/// Snapshot the session + UI flags into the pure layer's [`online::OnlineFacts`]
/// (the arguments of [`online::project`] / the screen sync).
fn online_facts(os: &OnlineSession) -> online::OnlineFacts<'_> {
    online::OnlineFacts {
        state: os.sess.state(),
        flags: &os.flags,
        is_host: os.sess.is_host(),
        code: os.sess.code(),
        lobby: os.sess.lobby(),
        can_start: os.sess.can_start(),
        start_block: os.sess.start_block(),
        error: os.sess.error(),
    }
}

/// Bevy `KeyCode` → a hub code-entry character (menus.js onKey L2728:
/// `/^[a-z0-9]$/i`; `online::code_type` then rejects the letters `CODE_ABC`
/// skips, L2544). Manual mapping — no IME layer in bevy input.
fn key_to_code_char(k: KeyCode) -> Option<char> {
    use KeyCode::*;
    Some(match k {
        KeyA => 'A',
        KeyB => 'B',
        KeyC => 'C',
        KeyD => 'D',
        KeyE => 'E',
        KeyF => 'F',
        KeyG => 'G',
        KeyH => 'H',
        KeyI => 'I',
        KeyJ => 'J',
        KeyK => 'K',
        KeyL => 'L',
        KeyM => 'M',
        KeyN => 'N',
        KeyO => 'O',
        KeyP => 'P',
        KeyQ => 'Q',
        KeyR => 'R',
        KeyS => 'S',
        KeyT => 'T',
        KeyU => 'U',
        KeyV => 'V',
        KeyW => 'W',
        KeyX => 'X',
        KeyY => 'Y',
        KeyZ => 'Z',
        Digit0 => '0',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        _ => return None,
    })
}

/// Cycle over a mandatory id table (Task 39 weapon button): an id outside the
/// table (e.g. `dualies` — valid in JS `WEAPONS`, not in the 12-entry
/// `WEAPON_ORDER` bots are dealt from) jumps to the table head; a known id
/// moves to the next entry, wrapping.
#[must_use]
fn next_in_cycle(current: &str, table: &[&str]) -> String {
    match table.iter().position(|x| *x == current) {
        Some(i) => table[(i + 1) % table.len()].to_string(),
        None => table[0].to_string(),
    }
}

/// Cycle an optional slot (sub / special) over its table with a NONE stop:
/// `None` picks the head; the last entry — or an id outside the table, which
/// JS `subOf`/`specialOf` (session.js L18) would already have nulled — goes
/// back to `None` (clears the slot).
#[must_use]
fn next_option_cycle(current: Option<&str>, table: &[&str]) -> Option<String> {
    match current {
        None => Some(table[0].to_string()),
        Some(c) => match table.iter().position(|x| *x == c) {
            Some(i) if i + 1 < table.len() => Some(table[i + 1].to_string()),
            _ => None,
        },
    }
}

/// Route one online button to the session / screen flags (the JS click
/// handlers in `_scr_online` / `_scr_lobby`; the screen-local `st` lives in
/// [`online::OnlineFlags`]).
fn handle_online_action(st: &mut UiState, oa: OnlineAction, os: &mut OnlineSession) {
    match oa {
        // Main-menu ONLINE card → hub (`show('online')`, menus.js L2407).
        OnlineAction::EnterHub => {
            st.net_mode = true;
            st.screen = Screen::OnlineHub;
        }
        OnlineAction::HubBack => {
            // Leaving the hub destroys the JS screen (`st.alive = false`
            // permanently drops the pending auto-join, L2554 callback).
            st.auto_join = 0.0;
            st.net_mode = false;
            st.screen = Screen::Menu;
        }
        OnlineAction::Create => run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Create),
        OnlineAction::Join => {
            // joinBtn click (L2439): connecting ⇒ `cancelConnect`, else
            // `doJoin` (whose busy guard swallows a create in flight).
            let cmd = if os.flags.joining {
                SessionCmd::Cancel
            } else {
                // `doJoin` opens with `clearTimeout(st.autoT)` (L2619)
                // regardless of the trigger source.
                st.auto_join = 0.0;
                SessionCmd::Join
            };
            run_cmd(&mut os.sess, &mut os.flags, cmd);
        }
        OnlineAction::Ready => run_cmd(&mut os.sess, &mut os.flags, SessionCmd::ToggleReady),
        OnlineAction::Team(t) => run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetTeam(t)),
        OnlineAction::Duration(d) => {
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetDuration(d))
        }
        OnlineAction::Bots => {
            // botTgl flip (L2848): `hostSet({ bots: v })` with the toggled
            // value.
            let v = !os.sess.lobby().bots;
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetBots(v));
        }
        OnlineAction::Difficulty => {
            // diffSeg cycle over the `DIFFICULTY` insertion order
            // (config.js L503-508): easy → normal → hard → easy.
            let next = match os.sess.lobby().difficulty.as_str() {
                "easy" => "normal",
                "normal" => "hard",
                _ => "easy",
            };
            run_cmd(
                &mut os.sess,
                &mut os.flags,
                SessionCmd::SetDifficulty(next.to_string()),
            );
        }
        OnlineAction::WeaponCycle => {
            // Task 39: `setMe({weapon})` over `WEAPON_ORDER`; the current
            // value comes from the local roster row (`you`, `_pushLobby`
            // L192). Lobby-state / `me`-row guards live in `run_cmd`.
            let cur = os
                .sess
                .lobby()
                .players
                .iter()
                .find(|p| p.you)
                .map(|p| p.weapon.as_str())
                .unwrap_or("shooter");
            let next = next_in_cycle(cur, WEAPON_ORDER);
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetWeapon(next));
        }
        OnlineAction::SubCycle => {
            let cur = os
                .sess
                .lobby()
                .players
                .iter()
                .find(|p| p.you)
                .and_then(|p| p.sub.as_deref());
            let next = next_option_cycle(cur, SUB_IDS);
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetSub(next));
        }
        OnlineAction::SpecialCycle => {
            let cur = os
                .sess
                .lobby()
                .players
                .iter()
                .find(|p| p.you)
                .and_then(|p| p.special.as_deref());
            let next = next_option_cycle(cur, SPECIAL_IDS);
            run_cmd(&mut os.sess, &mut os.flags, SessionCmd::SetSpecial(next));
        }
        OnlineAction::ModeToggle => {
            // `hostSet({mode})` (session.js L244): Task 44 batch D cycles
            // turf → zones → boss → turf. Host-only guard in `run_cmd`.
            let next = match os.sess.lobby().mode.as_str() {
                "turf" => "zones",
                "zones" => "boss",
                _ => "turf",
            };
            run_cmd(
                &mut os.sess,
                &mut os.flags,
                SessionCmd::SetMode(next.to_string()),
            );
        }
        OnlineAction::Leave => {
            // askLeave two-step (L3026-3046, deviation ⑦): first press arms
            // the confirm banner, second leaves the room.
            if os.flags.leave_confirm {
                run_cmd(&mut os.sess, &mut os.flags, SessionCmd::Leave);
            } else {
                os.flags.leave_confirm = true;
            }
        }
    }
}

/// Task 24: per-frame hub/lobby label refresh (equality-guarded writes, the
/// same no-dirty-layout pattern as `ui_update`).
#[allow(clippy::too_many_lines)]
pub fn online_ui_update(
    st: Res<UiState>,
    os: NonSend<OnlineSession>,
    mut labels: Query<(&LabelRole, &mut Text, &mut TextColor)>,
) {
    if !st.net_mode {
        return;
    }
    let view = online::project(&online_facts(&os));
    let hub = &view.hub;
    let lob = &view.lobby;
    let loading = view.screen == online::OnlineScreen::MatchLoading;
    for (role, mut text, mut color) in labels.iter_mut() {
        // (new text, new colour) — `None` colour keeps the current one.
        let (t, c): (String, Option<Color>) = match role.0 {
            "hub_c0" => (hub.cells[0].to_string(), None),
            "hub_c1" => (hub.cells[1].to_string(), None),
            "hub_c2" => (hub.cells[2].to_string(), None),
            "hub_c3" => (hub.cells[3].to_string(), None),
            "hub_c4" => (hub.cells[4].to_string(), None),
            "hub_hint" => (hub.hint.clone(), None),
            "hub_err" => match (&hub.err_title, &hub.err_text) {
                (Some(ti), Some(te)) => (format!("{ti} — {te}"), Some(RED)),
                _ => (String::new(), None),
            },
            "hub_create" => match &hub.create_status {
                Some(s) => (
                    s.clone(),
                    if hub.create_err { Some(RED) } else { Some(DIM) },
                ),
                None => (String::new(), Some(DIM)),
            },
            "hub_join" => (hub.join_label.to_string(), None),
            "lob_code" => (lob.code_tiles.clone(), None),
            "lob_count" => (format!("SQUIDKIDS {}/8", lob.count), None),
            "lob_row0" => roster_row(lob, 0),
            "lob_row1" => roster_row(lob, 1),
            "lob_row2" => roster_row(lob, 2),
            "lob_row3" => roster_row(lob, 3),
            "lob_row4" => roster_row(lob, 4),
            "lob_row5" => roster_row(lob, 5),
            "lob_row6" => roster_row(lob, 6),
            "lob_row7" => roster_row(lob, 7),
            "lob_status" => (lob.status_txt.clone(), None),
            // renderBar L3354-3355: the host's button reads START!
            "lob_ready" => (
                if lob.ready_is_start {
                    "START!"
                } else if lob.ready_on {
                    "READY!"
                } else {
                    "READY?"
                }
                .to_string(),
                None,
            ),
            "lob_settings" => (
                format!(
                    "LEN {} · BOTS {} · SKILL {}",
                    lob.duration,
                    if lob.bots { "ON" } else { "OFF" },
                    lob.difficulty.to_uppercase()
                ),
                None,
            ),
            // Task 39: the loadout / mode buttons read back the projected
            // own-row values (uppercase ids, NONE for an empty slot).
            "lob_weapon" => (format!("WPN {}", lob.weapon.to_uppercase()), None),
            "lob_sub" => (
                format!(
                    "SUB {}",
                    lob.sub.as_deref().unwrap_or("NONE").to_uppercase()
                ),
                None,
            ),
            "lob_special" => (
                format!(
                    "SPC {}",
                    lob.special.as_deref().unwrap_or("NONE").to_uppercase()
                ),
                None,
            ),
            "lob_mode" => (format!("MODE {}", lob.mode.to_uppercase()), None),
            // `launchLobby` overlay (L2362) while `starting`; else the
            // one-shot toast from `run_cmd`.
            "lob_banner" => (
                if loading {
                    "Starting the match…".to_string()
                } else {
                    lob.banner.clone().unwrap_or_default()
                },
                None,
            ),
            "lob_leave" => (lob.leave_text.clone().unwrap_or_default(), None),
            _ => continue,
        };
        if text.0 != t {
            text.0 = t;
        }
        if let Some(c) = c
            && color.0 != c
        {
            color.0 = c;
        }
    }
}

/// One roster slot line (JS nameplate `open[k]`, menus.js L2922-2928):
/// `OPEN` for an empty slot, else `★ name (you) · TEAM · READY?/READY!`
/// tinted by team.
fn roster_row(lob: &online::LobbyView, k: usize) -> (String, Option<Color>) {
    match lob.rows.get(k) {
        None => ("OPEN".to_string(), Some(DIM)),
        Some(r) => {
            let mut s = String::new();
            if r.host {
                s.push_str("★ ");
            }
            s.push_str(&r.name);
            if r.you {
                s.push_str(" (you)");
            }
            s.push_str(if r.team == 1 {
                " · BRAVO"
            } else {
                " · ALPHA"
            });
            s.push_str(if r.ready { " · READY!" } else { " · READY?" });
            (s, Some(team_color(r.team as usize)))
        }
    }
}

/// Per-frame HUD refresh: screen display switch + all dynamic labels/bars.
/// Every write is guarded by an equality check (upstream hud.js caches the
/// last string and only touches the DOM on change) so unchanged frames don't
/// mark `Text`/`Node` dirty and retrigger text measurement + layout.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn ui_update(
    st: Res<UiState>,
    demo: Res<DemoSim>,
    feed: Res<Feed>,
    flash: Res<HitFlash>,
    time: Res<Time>,
    // `ScreenRoot` and `BarRole` never co-exist on one entity; the `Without`
    // filters make the two `&mut Node` queries provably disjoint.
    mut roots: Query<(&ScreenRoot, &mut Node), Without<BarRole>>,
    mut labels: Query<(&LabelRole, &mut Text, &mut TextColor)>,
    mut bars: Query<(&BarRole, &mut Node), Without<ScreenRoot>>,
    mut cross: Query<(&Crosshair, &mut BackgroundColor)>,
    boss: Res<BossHudRes>,
) {
    for (root, mut node) in roots.iter_mut() {
        let want = if st.screen == root.0 {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
    }

    let on_hud = st.screen == Screen::Hud;
    let on_results = st.screen == Screen::Results;
    let cov = demo.m.paint.coverage();
    let names = team_names(&demo.tuning);
    let ink_frac = demo.m.actors.first().map_or(0.0, |a| {
        (a.ink / demo.tuning.player.ink_max).clamp(0.0, 1.0)
    });
    let now = time.elapsed_secs();

    let on_menu = st.screen == Screen::Menu;

    // Task 46 batch C2 — boss HUD placeholder档 (≡ the hud-boss.js writes).
    // Non-boss matches leave `BossHudRes::out` at the all-off default, so
    // every role clears.
    let bo = &boss.out;
    let (boss_bar_text, boss_open_text, boss_call_text, boss_banner_text) = if bo.bar_visible {
        let mut bar = format!("{BOSS_NAME} {}% · P{}", bo.pct, bo.phase);
        if bo.low {
            bar.push_str(" · LOW");
        }
        if bo.shield {
            bar.push_str(" · SHIELD");
        }
        if bo.enraged {
            bar.push_str(" · ENRAGED");
        }
        (
            bar,
            if bo.open {
                "OPEN!".to_string()
            } else {
                String::new()
            },
            bo.callout.clone().unwrap_or_default(),
            match bo.banner {
                Some(big) => format!("{big} — {}", bo.banner_sub.unwrap_or_default()),
                None => String::new(),
            },
        )
    } else {
        (String::new(), String::new(), String::new(), String::new())
    };
    let (boss_hp_w, boss_chip_w, boss_stun_w) = if bo.bar_visible {
        (
            bo.shown * 100.0,
            bo.chip * 100.0,
            if bo.open { bo.open_frac * 100.0 } else { 0.0 },
        )
    } else {
        (0.0, 0.0, 0.0)
    };

    if on_hud || on_results || on_menu {
        for (role, mut text, mut color) in labels.iter_mut() {
            // (new text, new colour) — `None` colour keeps the current one.
            let (t, c): (String, Option<Color>) = match role.0 {
                // Task 46: the offline mode selector label.
                "menu_mode" if on_menu => (format!("MODE {}", st.play_mode.label()), None),
                "timer" if on_hud => (fmt_clock(demo.m.time), Some(timer_color(demo.m.time))),
                "cov_a" if on_hud => (
                    pct(cov[0]),
                    // Leader highlight; ties count as A leading (spec adds
                    // this element; upstream HUD has no tie convention).
                    Some(if cov[0] >= cov[1] { team_color(0) } else { DIM }),
                ),
                "cov_b" if on_hud => (
                    pct(cov[1]),
                    Some(if cov[1] > cov[0] { team_color(1) } else { DIM }),
                ),
                "feed0" if on_hud => (feed_line(&feed, 0), None),
                "feed1" if on_hud => (feed_line(&feed, 1), None),
                "feed2" if on_hud => (feed_line(&feed, 2), None),
                "feed3" if on_hud => (feed_line(&feed, 3), None),
                "res_title" if on_results => {
                    demo.m.result.as_ref().map_or((String::new(), None), |r| {
                        let (title, _, _) = results_text(r, &names);
                        (title, Some(team_color(r.winner)))
                    })
                }
                "res_a" if on_results => (
                    demo.m.result.as_ref().map_or(String::new(), |r| {
                        let (_, a, _) = results_text(r, &names);
                        a
                    }),
                    None,
                ),
                "res_b" if on_results => (
                    demo.m.result.as_ref().map_or(String::new(), |r| {
                        let (_, _, b) = results_text(r, &names);
                        b
                    }),
                    None,
                ),
                "res_stats" if on_results => (stats_rows(&demo), None),
                // Task 46 batch C2 — boss HUD roles (JS bar text / callout /
                // title card / banner / ending word).
                "boss_bar" if on_hud => (
                    boss_bar_text.clone(),
                    Some(if bo.low { RED } else { Color::WHITE }),
                ),
                "boss_open" if on_hud => (boss_open_text.clone(), None),
                "boss_call" if on_hud => (boss_call_text.clone(), None),
                "boss_title" if on_hud => (
                    if bo.title_card {
                        BOSS_NAME.to_string()
                    } else {
                        String::new()
                    },
                    None,
                ),
                "boss_banner" if on_hud => (boss_banner_text.clone(), None),
                "boss_ending" if on_hud => (bo.ending.unwrap_or_default().to_string(), None),
                // Task 24: in online mode the same button is BACK TO LOBBY
                // (JS `netMatchEnd` re-shows the lobby; the click is
                // remapped in `ui_state_machine`).
                "res_again" if on_results => (
                    if st.net_mode {
                        "BACK TO LOBBY".to_string()
                    } else {
                        "PLAY AGAIN".to_string()
                    },
                    None,
                ),
                _ => continue,
            };
            if text.0 != t {
                text.0 = t;
            }
            if let Some(c) = c
                && color.0 != c
            {
                color.0 = c;
            }
        }
    }

    for (role, mut node) in bars.iter_mut() {
        let p = match role.0 {
            "cov_a_bar" if on_hud => cov[0] * 100.0,
            "cov_b_bar" if on_hud => cov[1] * 100.0,
            "ink_bar" if on_hud => ink_frac * 100.0,
            "res_a_bar" if on_results => demo.m.result.as_ref().map_or(0.0, |r| res_bar(r, 0)),
            "res_b_bar" if on_results => demo.m.result.as_ref().map_or(0.0, |r| res_bar(r, 1)),
            "boss_hp_bar" if on_hud => boss_hp_w,
            "boss_chip_bar" if on_hud => boss_chip_w,
            "boss_stun_bar" if on_hud => boss_stun_w,
            _ => continue,
        };
        let w = Val::Percent(p.min(100.0));
        if node.width != w {
            node.width = w;
        }
    }

    let hot = now < flash.until;
    let arm = if hot {
        AMBER
    } else {
        Color::srgba(1.0, 1.0, 1.0, 0.85)
    };
    for (_, mut bg) in cross.iter_mut() {
        if bg.0 != arm {
            bg.0 = arm;
        }
    }
}

/// Hover/press tint for menu buttons (upstream menu button states).
#[allow(clippy::type_complexity)]
pub fn ui_button_style(
    mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>,
) {
    for (inter, mut bg) in q.iter_mut() {
        bg.0 = match inter {
            Interaction::Pressed => BTN_BG_PRESS,
            Interaction::Hovered => BTN_BG_HOVER,
            Interaction::None => BTN_BG,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_clock_matches_upstream_ceil() {
        // ui-util.js L203-207: ceil, so 65.2 s shows as 1:06.
        assert_eq!(fmt_clock(180.0), "3:00");
        assert_eq!(fmt_clock(65.2), "1:06");
        assert_eq!(fmt_clock(9.001), "0:10");
        assert_eq!(fmt_clock(0.0), "0:00");
        assert_eq!(fmt_clock(-3.0), "0:00");
    }

    #[test]
    fn timer_color_states() {
        assert_eq!(timer_color(120.0), Color::WHITE);
        assert_eq!(timer_color(30.0), AMBER); // last minute
        assert_eq!(timer_color(8.0), RED); // final countdown
    }

    #[test]
    fn splat_text_covers_all_credit_cases() {
        let ev = MatchEvent::Splat {
            victim_team: 1,
            victim_slot: 2,
            attacker: Some((0, 0)),
            cause: SplatCause::Weapon,
        };
        assert_eq!(splat_text(&ev).as_deref(), Some("A1 > B3"));
        let water = MatchEvent::Splat {
            victim_team: 0,
            victim_slot: 0,
            attacker: None,
            cause: SplatCause::Water,
        };
        assert_eq!(splat_text(&water).as_deref(), Some("A1 fell in"));
        assert!(splat_text(&MatchEvent::OneMinute).is_none());
    }

    #[test]
    fn results_text_uses_winner_and_percents() {
        let r = MatchResult {
            coverage: [0.6132, 0.3868],
            winner: 0,
            points: [0.0; 2],
            elapsed: 180.0,
            zones: None,
            boss: None,
        };
        let names = ["Alpha".to_string(), "Bravo".to_string()];
        let (t, a, b) = results_text(&r, &names);
        assert_eq!(t, "ALPHA WINS!");
        assert_eq!(a, "61.3%");
        assert_eq!(b, "38.7%");
    }

    #[test]
    fn results_text_shows_the_zones_final_count() {
        // JS `_judgeZones` (hud.js L1096-1126): winner headline + the reason
        // sub (L1114), the `whole()`'d counts and the penalty caption.
        let names = ["Alpha".to_string(), "Bravo".to_string()];
        let r = MatchResult {
            coverage: [0.159, 0.248],
            winner: 1,
            points: [0.0; 2],
            elapsed: 140.0,
            zones: Some(ZoneOutcome {
                winner: 1,
                reason: Some("overtime-cap".to_string()),
                counts: [100, 87],
                penalty: [0.0, 3.0],
                overtime: true,
                log: Vec::new(),
            }),
            boss: None,
        };
        let (t, a, b) = results_text(&r, &names);
        assert_eq!(t, "BRAVO WINS! · OVERTIME LIMIT");
        assert_eq!(a, "100  COUNT LEFT");
        assert_eq!(b, "87  COUNT LEFT · +3 PENALTY");
        // The zones bar is the counted-off fill (hud.js L1136), not turf.
        assert_eq!(res_bar(&r, 0), 0.0);
        assert_eq!(res_bar(&r, 1), 13.0);
        let turf = MatchResult {
            zones: None,
            ..r.clone()
        };
        assert!(
            (res_bar(&turf, 0) - 15.9).abs() < 1e-4,
            "turf keeps the coverage bar"
        );
    }

    #[test]
    fn results_text_boss_branch_shows_outcome_hp_and_time() {
        // Task 46 batch B/C2: Boss settlement (JS bossMode.js L44-52:
        // `win = boss.dead`; main.js L960 `boss: {name, time, hpLeft, phase}`
        // — batch C2 adds the PHASE column; the per-player bossDmg / weakHits
        // columns ride `stats_rows`).
        let names = ["Alpha".to_string(), "Bravo".to_string()];
        let mk = |win: bool, time: f32, hp: f32| MatchResult {
            coverage: [0.0; 2],
            winner: if win { 0 } else { 1 },
            points: [0.0; 2],
            elapsed: 180.0,
            zones: None,
            boss: Some(inkwave_sim::boss::BossOutcome {
                name: "HULLBREAKER".to_string(),
                win,
                time,
                hp,
                max_hp: 120_000.0,
                phase: 2,
            }),
        };
        let (t, a, b) = results_text(&mk(true, 84.3, 0.0), &names);
        assert_eq!(t, "VICTORY!");
        assert_eq!(a, "HULLBREAKER 0/120000 HP · PHASE 2");
        assert_eq!(b, "TIME 84.3s");
        let (t, a, b) = results_text(&mk(false, 95.7, 41_234.0), &names);
        assert_eq!(t, "DEFEAT");
        assert_eq!(a, "HULLBREAKER 41234/120000 HP · PHASE 2");
        assert_eq!(b, "TIME 95.7s");
    }

    /// A live-enough `BossMode` for the presentation tests (the nettest.rs
    /// L610-628 construction pattern).
    fn mk_boss_mode() -> inkwave_sim::boss::BossMode {
        use inkwave_sim::actor::SimWorld;
        use inkwave_sim::boss::BossMode;
        use inkwave_sim::boss::nav::BossNav;
        use inkwave_sim::collision::CollisionWorld;
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let sim = SimWorld::new(&world, &layout);
        let tb = inkwave_sim::embedded_tuning();
        BossMode::new(
            &sim,
            BossNav::new(&sim, &tb.boss.nav),
            tb.boss.clone(),
            5.2,
            "normal",
            true,
            1,
        )
    }

    /// Task 46 batch C2: the boss results table rides `Boss::stats_by_nid`
    /// (JS main.js L961 damage/weakHits + menus.js L4031-4040 columns) with a
    /// totals line; turf matches keep the classic rows.
    #[test]
    fn stats_rows_boss_mode_shows_dmg_and_weak_columns() {
        use inkwave_sim::boss::AttackerStats;
        use inkwave_sim::collision::CollisionWorld;
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout, world, 20261004);

        // Turf rows first (no bm).
        let turf = stats_rows(&demo);
        assert!(turf.contains("splats"), "{turf}");
        assert!(!turf.contains("dmg"));

        // A boss match: seed the per-attacker table (offline squad ids are
        // roster indices, so nid == slot).
        let mut bm = mk_boss_mode();
        bm.boss.stats_by_nid.insert(
            0,
            AttackerStats {
                boss_dmg: 12_345.6,
                weak_hits: 7,
                splats: 3,
            },
        );
        bm.boss.stats_by_nid.insert(
            1,
            AttackerStats {
                boss_dmg: 4_000.0,
                weak_hits: 1,
                splats: 0,
            },
        );
        *demo.bm.lock().unwrap() = Some(bm);
        let rows = stats_rows(&demo);
        assert!(rows.contains("dmg 12346"), "{rows}");
        assert!(rows.contains("weak 7"), "{rows}");
        assert!(rows.contains("splats 3"), "{rows}");
        assert!(
            rows.contains("BOSS DMG 16346  WEAK 8"),
            "totals line: {rows}"
        );
    }

    /// Headless (TR-46.1): a boss match's drained events drive the state
    /// machines through the real systems without panicking, and the HUD
    /// labels ride the descriptor.
    #[test]
    fn boss_hud_headless_drain_updates_labels() {
        let mut app = ui_test_app();
        app.update(); // startup: build_ui inits BossHudRes
        // Enter a Hud screen and arm a boss match.
        app.world_mut().resource_mut::<UiState>().screen = Screen::Hud;
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            let mut bm = mk_boss_mode();
            bm.boss.hp = bm.boss.max_hp * 0.5;
            let hp = bm.boss.hp;
            *demo.bm.lock().unwrap() = Some(bm);
            demo.boss_events.push(BossEvent::Phase { phase: 2 });
            demo.boss_events.push(BossEvent::Stunned { dur: 3.0 });
            demo.boss_events.push(BossEvent::Hp { hp });
        }
        app.update();
        {
            let boss = app.world().resource::<BossHudRes>();
            assert!(boss.hud.is_some(), "bm live → state built");
            assert_eq!(boss.out.banner, Some("PHASE 2!"));
            assert!(boss.out.open, "stun window armed");
            assert!(!boss.audio.cues.is_empty(), "audio director fed");
        }
        // Defeat: SUNK! then the bar drops 1.3 s later.
        app.world_mut()
            .resource_mut::<DemoSim>()
            .boss_events
            .push(BossEvent::Defeat { killer: 0 });
        app.update();
        assert_eq!(
            app.world().resource::<BossHudRes>().out.ending,
            Some("SUNK!")
        );
        let label_text = |app: &mut App, want: &str| -> String {
            app.world_mut()
                .query::<(&LabelRole, &Text)>()
                .iter(app.world())
                .find(|(r, _)| r.0 == want)
                .map(|(_, t)| t.0.clone())
                .unwrap_or_default()
        };
        assert_eq!(label_text(&mut app, "boss_ending"), "SUNK!");
        // The 1.3 s is-gone timer is pinned in the boss_hud unit tests (the
        // headless loop's wall-time dt is not controllable here).
        assert!(app.world().resource::<BossHudRes>().out.bar_visible);
    }

    #[test]
    fn apply_action_transitions_and_freeze() {
        use inkwave_sim::collision::CollisionWorld;
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let mut st = UiState::default();
        let mut pc = PlayerControls::default();
        assert_eq!(st.screen, Screen::Menu);
        // The menu freezes the sim (WorldPlugin boots paused; mirror that
        // here through the state machine's own invariant).
        let mut quit = false;
        apply_action(&mut st, &mut pc, Action::Menu, &mut demo, &mut quit);
        assert!(pc.paused, "menu is frozen");

        apply_action(&mut st, &mut pc, Action::Play(90), &mut demo, &mut quit);
        assert_eq!(st.screen, Screen::Hud);
        assert!(!pc.paused);
        assert_eq!(demo.m.duration, 90.0, "chosen duration applied");
        assert_eq!(demo.m.phase, Phase::Intro, "fresh match");

        apply_action(&mut st, &mut pc, Action::Menu, &mut demo, &mut quit);
        assert_eq!(st.screen, Screen::Menu);
        assert!(pc.paused);

        apply_action(&mut st, &mut pc, Action::Quit, &mut demo, &mut quit);
        assert!(quit, "QUIT asks the app to exit");
    }

    /// TR-24.1: in net mode the offline Play/Restart transitions are refused
    /// and the Results button becomes BACK TO LOBBY (JS `netBackToLobby`
    /// remap, main.js L383/L872).
    #[test]
    fn tr24_net_mode_results_buttons() {
        use inkwave_sim::collision::CollisionWorld;
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut demo = DemoSim::new(layout, world, 20261004);
        let mut st = UiState::default();
        let mut pc = PlayerControls::default();
        let mut quit = false;
        st.net_mode = true;
        st.screen = Screen::Results;

        // Restart on net results must NOT restart the sim: `ui_state_machine`
        // remaps the button to `NetBackToLobby` (host-gated there).
        apply_action(&mut st, &mut pc, Action::Restart, &mut demo, &mut quit);
        assert_eq!(st.screen, Screen::Results);
        // Offline Play is refused while online (the flow owns the sim).
        apply_action(&mut st, &mut pc, Action::Play(90), &mut demo, &mut quit);
        assert_eq!(st.screen, Screen::Results);

        // The remapped button lands on the lobby screen.
        apply_action(
            &mut st,
            &mut pc,
            Action::NetBackToLobby,
            &mut demo,
            &mut quit,
        );
        assert_eq!(st.screen, Screen::OnlineLobby);
        assert!(pc.paused, "the sim stays frozen off the Hud");

        // Offline mode keeps the classic Restart behaviour.
        st.net_mode = false;
        st.screen = Screen::Results;
        apply_action(&mut st, &mut pc, Action::Restart, &mut demo, &mut quit);
        assert_eq!(st.screen, Screen::Hud);
        assert!(!pc.paused);
    }

    /// TR-13.1 (headless half): the screen roots exist, only the live screen
    /// is displayed, and the timer label tracks the sim clock.
    #[test]
    fn screens_switch_display_and_timer_follows_clock() {
        let mut app = ui_test_app();
        app.update();

        let displayed = |app: &mut App, want: Screen| -> usize {
            app.world_mut()
                .query::<(&ScreenRoot, &Node)>()
                .iter(app.world())
                .filter(|(r, n)| r.0 == want && n.display == Display::Flex)
                .count()
        };
        // Menu screen visible, the others hidden.
        assert_eq!(displayed(&mut app, Screen::Menu), 1);
        assert_eq!(displayed(&mut app, Screen::Hud), 0);

        // Play -> Hud displayed, menu hidden; timer shows the fresh clock.
        // Drive the action helper through a SystemState (three &mut resources
        // at once can't be grabbed one-by-one off the world).
        {
            use bevy::ecs::system::SystemState;
            let mut ss =
                SystemState::<(ResMut<UiState>, ResMut<PlayerControls>, ResMut<DemoSim>)>::new(
                    app.world_mut(),
                );
            let (mut st, mut pc, mut demo) = ss.get_mut(app.world_mut()).unwrap();
            let mut quit = false;
            apply_action(&mut st, &mut pc, Action::Play(90), &mut demo, &mut quit);
        }
        app.update();
        assert_eq!(displayed(&mut app, Screen::Hud), 1);
        assert_eq!(displayed(&mut app, Screen::Menu), 0);

        let timer = label_text(&mut app, "timer");
        assert_eq!(timer.as_deref(), Some("1:30"), "90 s clock");

        // The results screen appears when the match ends.
        {
            let w = app.world_mut();
            w.resource_mut::<DemoSim>().m.phase = Phase::End;
            w.resource_mut::<DemoSim>().m.result = Some(MatchResult {
                coverage: [0.6, 0.4],
                winner: 0,
                points: [0.0; 2],
                elapsed: 90.0,
                zones: None,
                boss: None,
            });
        }
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Results);
        // TR-13.1 second half: results -> Restart -> live Hud again.
        {
            use bevy::ecs::system::SystemState;
            let mut ss =
                SystemState::<(ResMut<UiState>, ResMut<PlayerControls>, ResMut<DemoSim>)>::new(
                    app.world_mut(),
                );
            let (mut st, mut pc, mut demo) = ss.get_mut(app.world_mut()).unwrap();
            let mut quit = false;
            apply_action(&mut st, &mut pc, Action::Restart, &mut demo, &mut quit);
        }
        app.update();
        assert_eq!(displayed(&mut app, Screen::Hud), 1);
        assert_eq!(displayed(&mut app, Screen::Results), 0);
        assert!(!app.world().resource::<PlayerControls>().paused);
        let timer = label_text(&mut app, "timer");
        assert_eq!(timer.as_deref(), Some("1:30"), "restarted clock");
    }

    /// TR-13.1/TR-13.2: the pause edge flips Hud <-> Pause through the state
    /// machine, the freeze invariant holds on every screen, and re-entering
    /// the Hud re-acquires mouse look.
    #[test]
    fn pause_edge_flips_hud_and_pause() {
        let mut app = ui_test_app();
        app.update();
        // Start a match through the state machine path (Play action applied
        // directly, then one update to settle the screens).
        {
            use bevy::ecs::system::SystemState;
            let mut ss =
                SystemState::<(ResMut<UiState>, ResMut<PlayerControls>, ResMut<DemoSim>)>::new(
                    app.world_mut(),
                );
            let (mut st, mut pc, mut demo) = ss.get_mut(app.world_mut()).unwrap();
            let mut quit = false;
            apply_action(&mut st, &mut pc, Action::Play(90), &mut demo, &mut quit);
        }
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Hud);

        // Raise the Esc/P edge -> Pause, frozen, cursor look released.
        app.world_mut().resource_mut::<PlayerControls>().pause_edge = true;
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Pause);
        let pc = app.world().resource::<PlayerControls>();
        assert!(pc.paused, "pause freezes the sim");
        assert!(!pc.look_enabled, "pause releases mouse look");
        {
            use bevy::window::CursorGrabMode;
            let cur = app
                .world_mut()
                .query::<&CursorOptions>()
                .single(app.world())
                .unwrap();
            assert!(
                matches!(cur.grab_mode, CursorGrabMode::None) && cur.visible,
                "OS cursor released for the menu"
            );
        }

        // Edge again -> back to Hud, unfrozen, look re-acquired.
        app.world_mut().resource_mut::<PlayerControls>().pause_edge = true;
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Hud);
        let pc = app.world().resource::<PlayerControls>();
        assert!(!pc.paused);
        assert!(pc.look_enabled, "resume re-acquires mouse look");
        {
            use bevy::window::CursorGrabMode;
            let cur = app
                .world_mut()
                .query::<&CursorOptions>()
                .single(app.world())
                .unwrap();
            assert!(
                matches!(cur.grab_mode, CursorGrabMode::Confined) && !cur.visible,
                "resume re-grabs the cursor"
            );
        }

        // Edge on the Hud flips to Pause; a stale edge on Menu is consumed
        // without changing the screen.
        app.world_mut().resource_mut::<PlayerControls>().pause_edge = true;
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Pause);
        {
            use bevy::ecs::system::SystemState;
            let mut ss = SystemState::<ResMut<UiState>>::new(app.world_mut());
            ss.get_mut(app.world_mut()).unwrap().screen = Screen::Menu;
        }
        app.world_mut().resource_mut::<PlayerControls>().pause_edge = true;
        app.update();
        assert_eq!(app.world().resource::<UiState>().screen, Screen::Menu);
        assert!(!app.world().resource::<PlayerControls>().pause_edge);
    }

    /// TR-13.2: match events drain into the kill feed (newest first, expired
    /// lines dropped, max four slots) and a local kill arms the crosshair
    /// flash; the feed labels mirror the feed.
    #[test]
    fn feed_drains_expires_and_mirrors_labels() {
        let mut app = ui_test_app();
        app.update();
        // Start a match so the Hud labels update.
        {
            use bevy::ecs::system::SystemState;
            let mut ss =
                SystemState::<(ResMut<UiState>, ResMut<PlayerControls>, ResMut<DemoSim>)>::new(
                    app.world_mut(),
                );
            let (mut st, mut pc, mut demo) = ss.get_mut(app.world_mut()).unwrap();
            let mut quit = false;
            apply_action(&mut st, &mut pc, Action::Play(90), &mut demo, &mut quit);
        }
        app.update();

        // Queue six splat events + one already-expired-style set: five bots
        // kills and one local kill (attacker slot 0).
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            for i in 0..5u8 {
                demo.m.events.push(MatchEvent::Splat {
                    victim_team: 1,
                    victim_slot: (i % 4) as usize,
                    attacker: Some((0, 0)),
                    cause: SplatCause::Weapon,
                });
            }
            demo.m.events.push(MatchEvent::Splat {
                victim_team: 0,
                victim_slot: 0,
                attacker: None,
                cause: SplatCause::Water,
            });
        }
        app.update();
        let feed = app.world().resource::<Feed>();
        assert_eq!(feed.lines.len(), 4, "truncate(4)");
        assert_eq!(feed.lines[0].1, "A1 fell in", "newest first");
        assert!(
            app.world().resource::<HitFlash>().until > 0.0,
            "local kill flashed"
        );
        assert_eq!(label_text(&mut app, "feed0").as_deref(), Some("A1 fell in"));

        // Expire everything: rewrite the feed with past expiries (the app
        // clock can't be advanced in place) and let the state machine drop
        // them on the next drain.
        {
            let now = app.world().resource::<Time>().elapsed_secs();
            let mut feed = app.world_mut().resource_mut::<Feed>();
            for (e, _) in feed.lines.iter_mut() {
                *e = now - 0.1;
            }
        }
        app.update();
        assert!(
            app.world().resource::<Feed>().lines.is_empty(),
            "expired dropped"
        );
        assert_eq!(label_text(&mut app, "feed0").as_deref(), Some(""));
    }

    #[test]
    fn tr39_cycle_helpers() {
        // Weapon: known id → next entry with wrap; an id outside the 12-entry
        // `WEAPON_ORDER` (e.g. `dualies`, valid in JS `WEAPONS` but never
        // dealt there) jumps back to the table head.
        assert_eq!(next_in_cycle("shooter", WEAPON_ORDER), "twins");
        assert_eq!(next_in_cycle("bucket", WEAPON_ORDER), "shooter");
        assert_eq!(next_in_cycle("dualies", WEAPON_ORDER), "shooter");
        // Optional slots: NONE → head, head → next, last entry → NONE, and an
        // off-table id (JS `subOf` would have nulled it) → NONE.
        assert_eq!(next_option_cycle(None, SUB_IDS).as_deref(), Some("bomb"));
        assert_eq!(
            next_option_cycle(Some("bomb"), SUB_IDS).as_deref(),
            Some("sticky")
        );
        assert_eq!(
            next_option_cycle(SUB_IDS.last().copied(), SUB_IDS),
            None,
            "the NONE stop sits after the last entry"
        );
        assert_eq!(next_option_cycle(Some("nope"), SUB_IDS), None);
        assert_eq!(
            next_option_cycle(None, SPECIAL_IDS).as_deref(),
            Some("slam")
        );
        assert_eq!(
            next_option_cycle(SPECIAL_IDS.last().copied(), SPECIAL_IDS),
            None
        );
    }

    fn label_text(app: &mut App, role: &str) -> Option<String> {
        app.world_mut()
            .query::<(&LabelRole, &Text)>()
            .iter(app.world())
            .find(|(r, _)| r.0 == role)
            .map(|(_, t)| t.0.clone())
    }

    /// Shared Bevy app with the UI systems wired (no rendering).
    fn ui_test_app() -> App {
        use crate::ink_render::InkAtlasRes;
        use bevy::asset::{AssetApp, AssetPlugin};
        use bevy::image::{ImagePlugin, TextureAtlasLayout};
        use bevy::input::InputPlugin;
        use bevy::picking::DefaultPickingPlugins;
        use bevy::text::TextPlugin;
        use bevy::transform::TransformPlugin;
        use bevy::ui::UiPlugin;
        use bevy::window::WindowPlugin;
        use inkwave_sim::collision::CollisionWorld;

        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout, world.clone(), 20261004);
        let atlas = InkAtlasRes::new(&world);

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            TransformPlugin,
            InputPlugin,
            TextPlugin,
            UiPlugin,
            WindowPlugin::default(),
            DefaultPickingPlugins,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        // bevy_ui's image-node system needs this asset collection registered
        // (normally via TextureAtlasPlugin, which we don't add wholesale).
        .init_asset::<TextureAtlasLayout>()
        .init_resource::<NetEvents>()
        .insert_resource(demo)
        .insert_resource(atlas)
        .insert_resource(PlayerControls {
            paused: true,
            ..default()
        })
        .init_resource::<UiState>()
        .init_resource::<Feed>()
        .init_resource::<HitFlash>()
        // Task 24: the state machine pumps the online session; the test app
        // never enters `net_mode`, so the transport thread stays idle.
        .insert_non_send(crate::net::OnlineSession::new("ws://127.0.0.1:8787"))
        .add_systems(Startup, build_ui)
        .add_systems(
            Update,
            (ui_state_machine, ui_update, online_ui_update).chain(),
        );
        app
    }
}
