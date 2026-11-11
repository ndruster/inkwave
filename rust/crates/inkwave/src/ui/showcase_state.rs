//! Task 56 batch A — 3D lobby / showcase STATE layer (pure data, no GPU).
//!
//! The extracted `rust/assets/lobbyset.json` (produced by
//! `rust/tools/extract/extract-lobbyset.mjs` from the READ-ONLY JS tree) holds
//! the alley layout, 8 lobby marks, camera, per-tier quality flags, pedestal
//! constants, mode sets and emote→dance table. This module ports the
//! `Showcase` state machine: loadout→display cues, hub/lobby/full-frame
//! switching, and the room code / ready / roster mapping from the existing
//! `ui::online::project()` view.
//!
//! Batch C wires these into Bevy; the 3D alley's materials, reflections,
//! PMREM lighting and cross-dissolves are GPU-review items (NFR-M4-5).
#![allow(dead_code)] // CONTRACTS-style API surface: remaining consumers are M5

use std::sync::OnceLock;

use bevy::prelude::Resource;
use inkwave_sim::net::wire::{JVal, parse_json};

use super::online::{OnlineScreen, OnlineView, RosterRow};

const DOC: &str = include_str!("../../../../assets/lobbyset.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("lobbyset.json parses"))
}

fn field(v: &'static JVal, key: &str) -> &'static JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("lobbyset.json: missing field {key}"))
}

fn f32_field(v: &'static JVal, key: &str) -> f32 {
    v.field(key)
        .and_then(JVal::as_f64)
        .unwrap_or_else(|| panic!("lobbyset.json: missing numeric field {key}")) as f32
}

fn bool_field(v: &'static JVal, key: &str) -> bool {
    v.field(key)
        .and_then(JVal::as_bool)
        .unwrap_or_else(|| panic!("lobbyset.json: missing boolean field {key}"))
}

fn str_field(v: &'static JVal, key: &str) -> &'static str {
    v.field(key)
        .and_then(JVal::as_str)
        .unwrap_or_else(|| panic!("lobbyset.json: missing string field {key}"))
}

fn array(v: &'static JVal, key: &str) -> &'static [JVal] {
    v.field(key)
        .and_then(JVal::as_array)
        .unwrap_or_else(|| panic!("lobbyset.json: missing array field {key}"))
}

fn pos3(v: &'static JVal, key: &str) -> [f32; 3] {
    let a = array(v, key);
    [
        a[0].as_f64().expect("x") as f32,
        a[1].as_f64().expect("y") as f32,
        a[2].as_f64().expect("z") as f32,
    ]
}

// ------------------------------------------------------------------ extracted data

/// One of the eight lineup marks (lobbySet.js L67): world position, yaw and
/// team (0 = your team on the asphalt, 1 = rivals on the dock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub team: u8,
}

#[must_use]
pub fn spots() -> Vec<Spot> {
    array(doc(), "spots")
        .iter()
        .map(|s| Spot {
            pos: pos3(s, "pos"),
            yaw: f32_field(s, "yaw"),
            team: f32_field(s, "team") as u8,
        })
        .collect()
}

/// `lobbySet.js` QUALITY row (L53-57): low disables reflection / steam /
/// extra lights — the Rust low tier keeps the minimal Bevy UI lobby.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LobbyQuality {
    pub name: &'static str,
    pub reflection: f32,
    pub shadow: u32,
    pub steam: bool,
    pub extra_lights: bool,
}

#[must_use]
pub fn lobby_quality(name: &str) -> Option<LobbyQuality> {
    let row = field(doc(), "quality").field(name)?;
    Some(LobbyQuality {
        name: match name {
            "low" => "low",
            "medium" => "medium",
            "high" => "high",
            _ => "high",
        },
        reflection: f32_field(row, "refl"),
        shadow: f32_field(row, "shadow") as u32,
        steam: bool_field(row, "steam"),
        extra_lights: bool_field(row, "extraLights"),
    })
}

/// `LOB_EMOTES` row (showcase.js L73-77) → dance clip + variant/hop.
#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub name: String,
    pub dance: String,
    pub variant: f32,
    pub duration: f32,
    pub hop: f32,
}

#[must_use]
pub fn emote(name: &str) -> Option<Emote> {
    let e = field(doc(), "emotes").field(name)?;
    Some(Emote {
        name: name.to_string(),
        dance: str_field(e, "dance").to_string(),
        variant: f32_field(e, "v"),
        duration: f32_field(e, "dur"),
        hop: f32_field(e, "hop"),
    })
}

/// Pedestal / podium constants (showcase.js L49-71).
#[derive(Clone, Debug, PartialEq)]
pub struct Pedestal {
    pub char_h: f32,
    pub load_focus_y: f32,
    pub plate_h: f32,
    pub set_fade: f32,
    pub glide: f32,
    pub hub_id: String,
    pub results_slots: usize,
    pub slot_r: f32,
    pub slot_top: f32,
    pub swim_v: f32,
}

#[must_use]
pub fn pedestal() -> Pedestal {
    let p = field(doc(), "pedestal");
    let swim = field(p, "SWIM");
    let slots = array(p, "SLOTS");
    Pedestal {
        char_h: f32_field(p, "CHAR_H"),
        load_focus_y: f32_field(p, "LOAD_FOCUS_Y"),
        plate_h: f32_field(p, "PLATE_H"),
        set_fade: f32_field(p, "SET_FADE"),
        glide: f32_field(p, "GLIDE"),
        hub_id: str_field(p, "HUB_ID").to_string(),
        results_slots: slots.len(),
        slot_r: f32_field(&slots[0], "R"),
        slot_top: f32_field(&slots[0], "top"),
        swim_v: f32_field(swim, "v"),
    }
}

/// Mode sets (showcase.js L60-63).
#[must_use]
pub fn mode_set(name: &str) -> Vec<&'static str> {
    array(field(doc(), "modes"), name)
        .iter()
        .filter_map(JVal::as_str)
        .collect()
}

/// `lobbySet.js` L44 / L70 camera poses (extracted, never hand-copied):
/// the hub uses the closer `hubCamera`, the line-up uses `CAM`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub pos: [f32; 3],
    pub target: [f32; 3],
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

#[must_use]
pub fn camera_pose(mode: ShowcaseMode) -> CameraPose {
    let key = if mode == ShowcaseMode::Hub {
        "hubCamera"
    } else {
        "cam"
    };
    let c = field(doc(), key);
    CameraPose {
        pos: pos3(c, "pos"),
        target: pos3(c, "target"),
        fov: f32_field(c, "fov"),
        near: f32_field(c, "near"),
        far: f32_field(c, "far"),
    }
}

/// Alley shell dimensions (`lobbySet.js` L38-43). The placeholder Bevy set
/// builds its floor, side walls and dock from these extracted values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlleyDims {
    pub x_l: f32,
    pub x_r: f32,
    pub z_near: f32,
    pub z_mouth: f32,
    pub h_l: f32,
    pub h_r: f32,
    /// dock `[x0, x1, z0, z1, h]` (lobbySet.js L40).
    pub dock: [f32; 5],
}

/// `LobbySet.hubSpot` (lobbySet.js L68): your kid's mark outside a room.
#[must_use]
pub fn hub_spot() -> ([f32; 3], f32) {
    let h = field(doc(), "hubSpot");
    (pos3(h, "pos"), f32_field(h, "yaw"))
}

#[must_use]
pub fn alley_dims() -> AlleyDims {
    let a = field(doc(), "alley");
    let d = field(a, "dock");
    AlleyDims {
        x_l: f32_field(a, "xL"),
        x_r: f32_field(a, "xR"),
        z_near: f32_field(a, "zNear"),
        z_mouth: f32_field(a, "zMouth"),
        h_l: f32_field(a, "hL"),
        h_r: f32_field(a, "hR"),
        dock: [
            f32_field(d, "x0"),
            f32_field(d, "x1"),
            f32_field(d, "z0"),
            f32_field(d, "z1"),
            f32_field(d, "h"),
        ],
    }
}

// ------------------------------------------------------------------ state machine

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShowcaseMode {
    #[default]
    Hidden,
    Loadout,
    Locker,
    Results,
    Hub,
    Lobby,
}

impl ShowcaseMode {
    /// Studio overlays drawn over the live world (showcase.js OVERLAY).
    #[must_use]
    pub fn is_overlay(self) -> bool {
        matches!(self, Self::Loadout | Self::Locker | Self::Results)
    }

    /// Full-frame LobbySet modes (showcase.js SET_MODES / fullFrame L1570-1573).
    #[must_use]
    pub fn is_full_frame(self) -> bool {
        matches!(self, Self::Hub | Self::Lobby)
    }

    /// One squidkid on the ink pedestal (showcase.js PEDESTAL).
    #[must_use]
    pub fn is_pedestal(self) -> bool {
        matches!(self, Self::Loadout | Self::Locker)
    }
}

/// One-shot display cue (showcase.js `_showPedestal` L1146-1178).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShowcaseCue {
    #[default]
    None,
    /// weapon changed → hop + twirl + mid-air weapon spawn + splash.
    HopTwirl,
    /// a single look part changed → pop (rotate/squash).
    Pop,
    /// preset / random look → dip into the ink then burst out.
    Dip,
    /// both weapon + look changed → full character swap (16-frame warmup).
    SwapChar,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShowcaseMember {
    pub slot: usize,
    pub name: String,
    pub team: u8,
    pub ready: bool,
    pub host: bool,
    pub you: bool,
    pub emote: Option<String>,
}

/// Runtime showcase state. `reduced` = low quality tier, where the 3D LobbySet
/// is skipped and the existing minimal Bevy UI lobby stays (tasks.md Task 56
/// Notes + props/mod.rs Low precedent).
#[derive(Resource, Clone, Debug)]
pub struct ShowcaseState {
    pub mode: ShowcaseMode,
    pub weapon: String,
    pub style: String,
    pub room_code: String,
    pub members: Vec<ShowcaseMember>,
    pub ready_on: bool,
    pub quick: bool,
    pub reduced: bool,
    pub full_frame: bool,
    pub last_cue: ShowcaseCue,
    pub replay_count: u32,
}

impl Default for ShowcaseState {
    fn default() -> Self {
        Self {
            mode: ShowcaseMode::Hidden,
            weapon: "shooter".to_string(),
            style: String::new(),
            room_code: String::new(),
            members: Vec::new(),
            ready_on: false,
            quick: false,
            reduced: false,
            full_frame: false,
            last_cue: ShowcaseCue::None,
            replay_count: 0,
        }
    }
}

impl ShowcaseState {
    /// loadout → display: compare the current weapon/style with the new ones
    /// and emit the same one-shot cue the JS `_showPedestal` picks.
    pub fn apply_loadout(&mut self, weapon: &str, style: &str, preset: bool) -> ShowcaseCue {
        let w_changed = weapon != self.weapon;
        let s_changed = style != self.style;
        let cue = match (w_changed, s_changed) {
            (true, true) => ShowcaseCue::SwapChar,
            (true, false) => ShowcaseCue::HopTwirl,
            (false, true) if preset => ShowcaseCue::Dip,
            (false, true) => ShowcaseCue::Pop,
            (false, false) => ShowcaseCue::None,
        };
        if w_changed {
            self.weapon = weapon.to_string();
        }
        if s_changed {
            self.style = style.to_string();
        }
        if cue != ShowcaseCue::None {
            // `last_cue` means "most recent one-shot", not "this frame's
            // result": `sync_from_online_view` calls this every frame and most
            // frames produce None. Overwriting with None would erase the cue
            // before the render layer's `replay_count` diff can consume it
            // (review P1-2).
            self.replay_count += 1;
            self.last_cue = cue;
        }
        cue
    }

    /// Consume the existing online projection (ui/online.rs `project()`), the
    /// single source of truth for room code / roster / ready state.
    pub fn from_online_view(view: &OnlineView, tier: &str) -> Self {
        let mut s = Self::default();
        s.mode = match view.screen {
            OnlineScreen::Hub => ShowcaseMode::Hub,
            OnlineScreen::Lobby => ShowcaseMode::Lobby,
            OnlineScreen::MatchLoading | OnlineScreen::Match => ShowcaseMode::Hidden,
        };
        s.room_code = view
            .lobby
            .code_tiles
            .chars()
            .filter(|c| *c != '•' && *c != ' ')
            .take(5)
            .collect();
        s.members = rows_to_members(&view.lobby.rows);
        s.ready_on = view.lobby.ready_on;
        s.weapon = view.lobby.weapon.clone();
        s.quick = view.screen == OnlineScreen::MatchLoading;
        s.reduced = tier.eq_ignore_ascii_case("low");
        s.full_frame = s.mode.is_full_frame() && !s.reduced && !s.quick;
        s
    }

    /// Frame-by-frame Bevy sync: consume the very same `OnlineView` the 2D
    /// lobby panel reads, but keep the one-shot cue / emote state that
    /// `project()` does not carry (≡ `updateLobby` L1739-1754 patching only
    /// roster fields). Returns the cue produced when the loadout weapon
    /// changes, so the render layer can replay its hop/twirl/pop.
    pub fn sync_from_online_view(&mut self, view: &OnlineView, tier: &str) -> ShowcaseCue {
        let next = Self::from_online_view(view, tier);
        // Keep the current look (the online view only carries the weapon), so
        // a roster refresh cannot spuriously fire a `SwapChar` cue.
        let style = self.style.clone();
        let cue = self.apply_loadout(&next.weapon, &style, false);
        self.mode = next.mode;
        self.quick = next.quick;
        self.reduced = next.reduced;
        self.refresh_frame();
        self.room_code = next.room_code;
        self.ready_on = next.ready_on;
        let prev = std::mem::take(&mut self.members);
        let mut rows = next.members;
        for m in &mut rows {
            if let Some(old) = prev.iter().find(|p| p.slot == m.slot && p.name == m.name) {
                m.emote = old.emote.clone();
            }
        }
        self.members = rows;
        cue
    }

    pub fn enter_hub(&mut self) {
        self.mode = ShowcaseMode::Hub;
        self.refresh_frame();
    }

    pub fn enter_lobby(&mut self, quick: bool) {
        self.mode = ShowcaseMode::Lobby;
        self.quick = quick;
        self.refresh_frame();
    }

    pub fn show_loadout(&mut self, weapon: &str, style: &str, preset: bool) -> ShowcaseCue {
        self.mode = ShowcaseMode::Loadout;
        self.refresh_frame();
        self.apply_loadout(weapon, style, preset)
    }

    pub fn show_locker(&mut self) {
        self.mode = ShowcaseMode::Locker;
        self.refresh_frame();
    }

    pub fn show_results(&mut self) {
        self.mode = ShowcaseMode::Results;
        self.refresh_frame();
    }

    pub fn hide(&mut self) {
        self.mode = ShowcaseMode::Hidden;
        self.quick = false;
        self.refresh_frame();
    }

    pub fn leave_lobby(&mut self) {
        self.members.clear();
        self.room_code.clear();
        self.ready_on = false;
        self.enter_hub();
    }

    /// `lobbyEmote` (showcase.js L1999): assign a dance cue to a roster slot.
    /// Returns the extracted emote so the render layer can drive CharAnim.
    pub fn emote(&mut self, slot: usize, name: &str) -> Option<Emote> {
        let e = emote(name)?;
        if let Some(m) = self.members.iter_mut().find(|m| m.slot == slot) {
            m.emote = Some(name.to_string());
        }
        Some(e)
    }

    fn refresh_frame(&mut self) {
        self.full_frame = self.mode.is_full_frame() && !self.reduced && !self.quick;
    }
}

/// Team-order slots: your team (0..3) on the asphalt, rivals (4..7) on the
/// dock — the 8 `LobbySet.spots` order (lobbySet.js L67).
#[must_use]
pub fn rows_to_members(rows: &[RosterRow]) -> Vec<ShowcaseMember> {
    let mut front = 0usize;
    let mut back = 4usize;
    rows.iter()
        .map(|r| {
            let slot = if r.team == 1 {
                let s = back.min(7);
                back = back.saturating_add(1);
                s
            } else {
                let s = front.min(3);
                front = front.saturating_add(1);
                s
            };
            ShowcaseMember {
                slot,
                name: r.name.clone(),
                team: r.team,
                ready: r.ready,
                host: r.host,
                you: r.you,
                emote: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::online::{HubView, LobbyView};

    fn row(name: &str, team: u8, ready: bool) -> RosterRow {
        RosterRow {
            name: name.to_string(),
            team,
            ready,
            host: false,
            ping: 0,
            you: name == "me",
            sig: name.to_string(),
        }
    }

    fn view(screen: OnlineScreen, code: &str, rows: Vec<RosterRow>, ready: bool) -> OnlineView {
        let mut cells = [' '; 5];
        for (i, c) in code.chars().take(5).enumerate() {
            cells[i] = c;
        }
        OnlineView {
            screen,
            hub: HubView {
                create_enabled: true,
                join_label: "JOIN",
                join_ready: false,
                cells,
                hint: String::new(),
                err_title: None,
                err_text: None,
                create_status: None,
                create_err: false,
            },
            lobby: LobbyView {
                code_tiles: code.to_string(),
                rows,
                status_txt: String::new(),
                ready_is_start: false,
                ready_on: ready,
                ready_sub: "",
                start_enabled: false,
                start_sub: String::new(),
                settings_enabled: false,
                duration: 180,
                bots: true,
                difficulty: "normal".to_string(),
                mode: "turf".to_string(),
                weapon: "spritzer".to_string(),
                sub: None,
                special: None,
                count: 8,
                leave_text: None,
                banner: None,
            },
        }
    }

    #[test]
    fn extracted_lobbyset_anchors() {
        let s = spots();
        assert_eq!(s.len(), 8);
        assert_eq!(s[0].team, 0);
        assert_eq!(s[4].team, 1);
        assert!(s.iter().all(|p| p.pos.iter().all(|v| v.is_finite())));
        assert_eq!(f32_field(field(doc(), "cam"), "fov"), 32.0);
        assert_eq!(pos3(field(doc(), "cam"), "pos"), [-2.45, 2.25, 10.4]);
        let low = lobby_quality("low").unwrap();
        assert_eq!(low.reflection, 0.0);
        assert_eq!(low.shadow, 1024);
        assert!(!low.steam && !low.extra_lights);
        assert_eq!(lobby_quality("high").unwrap().reflection, 0.5);
        assert!(lobby_quality("potato").is_none());
        let p = pedestal();
        assert_eq!(p.char_h, 1.62);
        assert_eq!(p.plate_h, 1.64);
        assert_eq!(p.results_slots, 4);
        assert_eq!(p.swim_v, 10.0);
        assert_eq!(mode_set("pedestal"), vec!["loadout", "locker"]);
        assert_eq!(mode_set("set"), vec!["hub", "lobby"]);
    }

    #[test]
    fn emote_table_maps_dances() {
        let b = emote("booyah").expect("booyah");
        assert_eq!(b.dance, "victory");
        assert_eq!(b.duration, 2.9);
        assert_eq!(b.hop, 0.32);
        assert_eq!(emote("wave").unwrap().variant, 2.0);
        assert!(emote("unknown").is_none());
    }

    #[test]
    fn loadout_change_cues_follow_the_js_branches() {
        // Seed the look so the first call is a genuine no-change frame; the
        // default style is empty, which would otherwise be a `Pop` edge.
        let mut s = ShowcaseState {
            style: "look0".to_string(),
            ..Default::default()
        };
        assert_eq!(
            s.apply_loadout("shooter", "look0", false),
            ShowcaseCue::None
        );
        assert_eq!(
            s.apply_loadout("roller", "look0", false),
            ShowcaseCue::HopTwirl
        );
        assert_eq!(s.apply_loadout("roller", "look1", false), ShowcaseCue::Pop);
        assert_eq!(s.apply_loadout("roller", "look2", true), ShowcaseCue::Dip);
        assert_eq!(s.apply_loadout("bow", "look3", true), ShowcaseCue::SwapChar);
        assert_eq!(s.replay_count, 4);
        assert_eq!(s.weapon, "bow");
        assert_eq!(s.style, "look3");
    }

    #[test]
    fn mode_transitions_and_full_frame() {
        let mut s = ShowcaseState::default();
        s.enter_hub();
        assert_eq!(s.mode, ShowcaseMode::Hub);
        assert!(s.full_frame && !s.reduced);
        s.enter_lobby(false);
        assert_eq!(s.mode, ShowcaseMode::Lobby);
        assert!(s.full_frame);
        s.enter_lobby(true);
        assert!(!s.full_frame, "launch overlay drops full-frame");
        s.show_loadout("shooter", "x", false);
        assert_eq!(s.mode, ShowcaseMode::Loadout);
        assert!(s.mode.is_overlay() && s.mode.is_pedestal() && !s.full_frame);
        s.hide();
        assert_eq!(s.mode, ShowcaseMode::Hidden);
        assert!(!s.quick && !s.full_frame);
    }

    #[test]
    fn online_view_maps_code_ready_weapon_and_roster() {
        let rows = vec![
            row("me", 0, true),
            row("a", 0, false),
            row("b", 0, true),
            row("c", 0, false),
            row("d", 1, false),
            row("e", 1, true),
            row("f", 1, false),
            row("g", 1, true),
        ];
        let v = view(OnlineScreen::Lobby, "AB3CD", rows, true);
        let mut s = ShowcaseState::from_online_view(&v, "high");
        assert_eq!(s.mode, ShowcaseMode::Lobby);
        assert_eq!(s.room_code, "AB3CD");
        assert_eq!(s.weapon, "spritzer");
        assert!(s.ready_on && s.full_frame && !s.reduced);
        assert_eq!(s.members.len(), 8);
        assert_eq!(s.members[0].slot, 0);
        assert_eq!(s.members[4].slot, 4);
        assert!(s.members[0].you && s.members[0].ready);
        assert!(s.emote(0, "booyah").is_some());
        assert_eq!(s.members[0].emote.as_deref(), Some("booyah"));
    }

    #[test]
    fn low_tier_keeps_the_minimal_lobby_ui() {
        let v = view(OnlineScreen::Hub, "•••••", vec![], false);
        let s = ShowcaseState::from_online_view(&v, "low");
        assert!(s.reduced);
        assert!(!s.full_frame, "no 3D set at low quality");
        assert_eq!(s.room_code, "", "bullet padding is not a room code");
        assert_eq!(s.mode, ShowcaseMode::Hub);
    }

    #[test]
    fn scene_switch_sequence_is_idempotent() {
        let mut s = ShowcaseState::default();
        for _ in 0..8 {
            s.enter_hub();
            s.enter_lobby(false);
            s.show_locker();
            s.show_results();
            s.hide();
        }
        assert_eq!(s.mode, ShowcaseMode::Hidden);
        assert!(!s.full_frame);
        s.enter_lobby(false);
        s.leave_lobby();
        assert_eq!(s.mode, ShowcaseMode::Hub);
        assert!(s.members.is_empty() && s.room_code.is_empty() && !s.ready_on);
    }

    #[test]
    fn extracted_camera_poses_and_alley_dims() {
        let hub = camera_pose(ShowcaseMode::Hub);
        assert_eq!(hub.pos, [-1.2, 1.1, 5.6]);
        assert_eq!(hub.target, [0.39, 1.0, 0.86]);
        assert_eq!((hub.fov, hub.near, hub.far), (30.0, 0.1, 140.0));
        let lobby = camera_pose(ShowcaseMode::Lobby);
        assert_eq!(lobby.pos, [-2.45, 2.25, 10.4]);
        assert_eq!(lobby.target, [-0.95, 1.2, -1.7]);
        assert_eq!((lobby.fov, lobby.near, lobby.far), (32.0, 0.1, 140.0));
        // Overlay/pedestal modes still fall back to the line-up camera.
        assert_eq!(camera_pose(ShowcaseMode::Loadout), lobby);
        let a = alley_dims();
        assert_eq!(
            [a.x_l, a.x_r, a.z_near, a.z_mouth],
            [-4.0, 4.0, 14.0, -34.0]
        );
        assert_eq!((a.h_l, a.h_r), (13.2, 15.6));
        assert_eq!(a.dock, [-1.35, 4.0, -10.5, -1.9, 1.0]);
        let p = pedestal();
        assert_eq!((p.slot_r, p.slot_top), (0.7, 0.72));
        assert_eq!(hub_spot(), ([1.7, 0.0, 0.9], -0.45));
    }

    #[test]
    fn frame_sync_keeps_emotes_and_emits_weapon_cues() {
        let rows = vec![
            row("me", 0, true),
            row("a", 0, false),
            row("b", 0, true),
            row("c", 0, false),
            row("d", 1, false),
            row("e", 1, true),
            row("f", 1, false),
            row("g", 1, true),
        ];
        let mut s = ShowcaseState::from_online_view(
            &view(OnlineScreen::Lobby, "AB3CD", rows.clone(), true),
            "high",
        );
        assert_eq!(
            s.sync_from_online_view(
                &view(OnlineScreen::Lobby, "AB3CD", rows.clone(), true),
                "high"
            ),
            ShowcaseCue::None
        );
        assert!(s.emote(0, "wave").is_some());
        // Roster refresh must not wipe an in-flight emote or replay counter.
        assert_eq!(
            s.sync_from_online_view(
                &view(OnlineScreen::Lobby, "AB3CD", rows.clone(), false),
                "high"
            ),
            ShowcaseCue::None
        );
        assert_eq!(s.members[0].emote.as_deref(), Some("wave"));
        assert!(!s.ready_on);
        // A weapon change is the only view field that drives a display cue.
        let mut changed = view(OnlineScreen::Lobby, "AB3CD", rows, true);
        changed.lobby.weapon = "roller".to_string();
        assert_eq!(
            s.sync_from_online_view(&changed, "high"),
            ShowcaseCue::HopTwirl
        );
        assert_eq!(s.weapon, "roller");
        assert_eq!(s.replay_count, 1);
        // Low tier drops the 3D set every frame without touching cues.
        assert_eq!(s.sync_from_online_view(&changed, "low"), ShowcaseCue::None);
        assert!(s.reduced && !s.full_frame);
        assert_eq!(s.replay_count, 1, "no-change frames emit no extra cue");
        assert_eq!(
            s.last_cue,
            ShowcaseCue::HopTwirl,
            "last_cue survives a no-change frame (review P1-2)"
        );
    }
}
