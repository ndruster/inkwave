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
use inkwave_sim::match_::{MatchEvent, MatchResult, Phase};
use inkwave_sim::tuning::Tuning;

use crate::actors::team_color;
use crate::ink_render::DemoSim;
use crate::input::PlayerControls;

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
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Menu,
            duration: DEFAULT_DURATION,
        }
    }
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

/// Button command handled by `ui_state_machine` on click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
/// deviation).
#[must_use]
pub fn results_text(res: &MatchResult, names: &[String; 2]) -> (String, String, String) {
    (
        format!("{} WINS!", names[res.winner].to_uppercase()),
        pct(res.coverage[0]),
        pct(res.coverage[1]),
    )
}

/// Per-actor results rows: tag, splats, deaths, turf m² (no trailing newline).
#[must_use]
pub fn stats_rows(demo: &DemoSim) -> String {
    let mut s = String::new();
    for (i, a) in demo.m.actors.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(&format!(
            "{}  splats {}  deaths {}  turf {:.0}m2",
            slot_tag(a.team, a.slot),
            a.splats,
            a.deaths,
            a.turf
        ));
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
    commands.spawn((label(text, 20.0, Color::WHITE), ChildOf(btn)));
}

/// Build all four screen trees (only one is displayed at a time).
pub fn build_ui(mut commands: Commands) {
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
    button(&mut commands, res_col, "PLAY AGAIN", Action::Restart);
    button(&mut commands, res_col, "MAIN MENU", Action::Menu);
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
            st.duration = d;
            demo.replay(d as f32);
            st.screen = Screen::Hud;
        }
        Action::Resume => st.screen = Screen::Hud,
        Action::Restart => {
            let d = st.duration;
            demo.replay(d as f32);
            st.screen = Screen::Hud;
        }
        Action::Menu => st.screen = Screen::Menu,
        Action::Quit => *quit = true,
    }
    pc.paused = st.screen != Screen::Hud;
}

/// State machine: pause edge / Enter shortcuts / button clicks, plus the
/// event drain feeding the kill feed and the results transition.
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
) {
    let prev = st.screen;
    let mut quit = false;

    // Esc/P edge from `map_input` (TR-12.1): Hud <-> Pause.
    if std::mem::take(&mut pc.pause_edge) {
        match st.screen {
            Screen::Hud => st.screen = Screen::Pause,
            Screen::Pause => st.screen = Screen::Hud,
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
                apply_action(&mut st, &mut pc, Action::Restart, &mut demo, &mut quit)
            }
            Screen::Hud => {}
        }
    }
    // Button clicks.
    for ev in clicks.read() {
        if ev.event.button != PointerButton::Primary {
            continue;
        }
        if let Some(a) = actions.get(ev.entity).ok().copied() {
            apply_action(&mut st, &mut pc, a.0, &mut demo, &mut quit);
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

    if on_hud || on_results {
        for (role, mut text, mut color) in labels.iter_mut() {
            // (new text, new colour) — `None` colour keeps the current one.
            let (t, c): (String, Option<Color>) = match role.0 {
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
                "res_title" if on_results => demo.m.result.map_or((String::new(), None), |r| {
                    let (title, _, _) = results_text(&r, &names);
                    (title, Some(team_color(r.winner)))
                }),
                "res_a" if on_results => (
                    demo.m.result.map_or(String::new(), |r| pct(r.coverage[0])),
                    None,
                ),
                "res_b" if on_results => (
                    demo.m.result.map_or(String::new(), |r| pct(r.coverage[1])),
                    None,
                ),
                "res_stats" if on_results => (stats_rows(&demo), None),
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
            "res_a_bar" if on_results => demo.m.result.map_or(0.0, |r| r.coverage[0] * 100.0),
            "res_b_bar" if on_results => demo.m.result.map_or(0.0, |r| r.coverage[1] * 100.0),
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
        };
        let names = ["Alpha".to_string(), "Bravo".to_string()];
        let (t, a, b) = results_text(&r, &names);
        assert_eq!(t, "ALPHA WINS!");
        assert_eq!(a, "61.3%");
        assert_eq!(b, "38.7%");
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
        .insert_resource(demo)
        .insert_resource(atlas)
        .insert_resource(PlayerControls {
            paused: true,
            ..default()
        })
        .init_resource::<UiState>()
        .init_resource::<Feed>()
        .init_resource::<HitFlash>()
        .add_systems(Startup, build_ui)
        .add_systems(Update, (ui_state_machine, ui_update).chain());
        app
    }
}
