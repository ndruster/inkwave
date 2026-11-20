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

use bevy::math::{Mat4, Vec3, Vec4};
use bevy::prelude::Resource;
use inkwave_sim::net::SessionId;
use inkwave_sim::net::session::{Lobby, TeamSel};
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

// ------------------------------------------------------------------ Task 67 materials

/// `groundMaterial()` planar-reflection constants (lobbySet-mats.js L175/L290-292).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WetReflectionSpec {
    pub env_map_intensity: f32,
    pub fresnel_f0: f32,
    pub fresnel_scale: f32,
    pub fresnel_pow: f32,
    pub k_damp: f32,
    pub k_ink: f32,
    pub water_mix: [f32; 2],
}

#[must_use]
pub fn wet_reflection() -> WetReflectionSpec {
    let r = field(doc(), "materials");
    let r = field(r, "reflection");
    let wm = array(r, "water_mix");
    WetReflectionSpec {
        env_map_intensity: f32_field(r, "env_map_intensity"),
        fresnel_f0: f32_field(r, "fresnel_f0"),
        fresnel_scale: f32_field(r, "fresnel_scale"),
        fresnel_pow: f32_field(r, "fresnel_pow"),
        k_damp: f32_field(r, "k_damp"),
        k_ink: f32_field(r, "k_ink"),
        water_mix: [
            wm[0].as_f64().unwrap() as f32,
            wm[1].as_f64().unwrap() as f32,
        ],
    }
}

/// `neonMaterial()` constants (lobbySet-mats.js L17/L321): hot-core color,
/// intensity vec4 (team / sign / buzz / halo) and the two team hex colors.
#[derive(Clone, Debug, PartialEq)]
pub struct NeonSpec {
    pub core_color: [f32; 3],
    pub intensity: [f32; 4],
    pub team_a: &'static str,
    pub team_b: &'static str,
}

#[must_use]
pub fn neon() -> NeonSpec {
    let n = field(field(doc(), "materials"), "neon");
    let c = array(n, "core_color");
    let i = array(n, "intensity");
    NeonSpec {
        core_color: [
            c[0].as_f64().unwrap() as f32,
            c[1].as_f64().unwrap() as f32,
            c[2].as_f64().unwrap() as f32,
        ],
        intensity: [
            i[0].as_f64().unwrap() as f32,
            i[1].as_f64().unwrap() as f32,
            i[2].as_f64().unwrap() as f32,
            i[3].as_f64().unwrap() as f32,
        ],
        team_a: str_field(n, "team_a"),
        team_b: str_field(n, "team_b"),
    }
}

/// `steamMaterial()` / `_fx()` constants (lobbySet-mats.js L424-441,
/// lobbySet.js L589-591): crossed cards, color, scrolling noise and alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct SteamSpec {
    pub cards: u32,
    pub card_height: f32,
    pub x: f32,
    pub z: f32,
    pub seed_step: f32,
    pub color: [f32; 3],
    pub time_scale: f32,
    pub noise_scale: [f32; 2],
    pub alpha_edge: [f32; 2],
    pub alpha_scale: f32,
}

#[must_use]
pub fn steam() -> SteamSpec {
    let s = field(field(doc(), "materials"), "steam");
    let col = array(s, "color");
    let ns = array(s, "noise_scale");
    let ae = array(s, "alpha_edge");
    SteamSpec {
        cards: f32_field(s, "cards") as u32,
        card_height: f32_field(s, "card_height"),
        x: f32_field(s, "x"),
        z: f32_field(s, "z"),
        seed_step: f32_field(s, "seed_step"),
        color: [
            col[0].as_f64().unwrap() as f32,
            col[1].as_f64().unwrap() as f32,
            col[2].as_f64().unwrap() as f32,
        ],
        time_scale: f32_field(s, "time_scale"),
        noise_scale: [
            ns[0].as_f64().unwrap() as f32,
            ns[1].as_f64().unwrap() as f32,
        ],
        alpha_edge: [
            ae[0].as_f64().unwrap() as f32,
            ae[1].as_f64().unwrap() as f32,
        ],
        alpha_scale: f32_field(s, "alpha_scale"),
    }
}

/// Lobby-local PMREM `_bakeEnv()/_rebakeEnv()` constants (lobbySet.js L156/L160/L73).
/// Distinct from the world PMREM (env_bake.rs): size 128 / envIntensity 0.6.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyPmremSpec {
    pub sigma: f32,
    pub near: f32,
    pub far: f32,
    pub size: u32,
    pub position: [f32; 3],
    pub team_normalize: f32,
    pub environment_intensity: f32,
    pub env_card_k: EnvCardK,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvCardK {
    pub door: f32,
    pub neon: f32,
    pub bulbs: f32,
    pub vending: f32,
}

#[must_use]
pub fn lobby_pmrem() -> LobbyPmremSpec {
    let p = field(field(doc(), "materials"), "pmrem");
    let k = field(p, "env_card_k");
    LobbyPmremSpec {
        sigma: f32_field(p, "sigma"),
        near: f32_field(p, "near"),
        far: f32_field(p, "far"),
        size: f32_field(p, "size") as u32,
        position: pos3(p, "position"),
        team_normalize: f32_field(p, "team_normalize"),
        environment_intensity: f32_field(p, "environment_intensity"),
        env_card_k: EnvCardK {
            door: f32_field(k, "door"),
            neon: f32_field(k, "neon"),
            bulbs: f32_field(k, "bulbs"),
            vending: f32_field(k, "vending"),
        },
    }
}

/// `LOB_EMOTES` row (showcase.js L73-77) → dance clip + variant/hop.
#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub name: String,
    pub dance: String,
    pub variant: f32,
    pub duration: f32,
    pub hop: f32,
    /// `showcase.js` L74 `t0`: optional start offset (flex = 2.86 s).
    pub t0: Option<f32>,
    /// `showcase.js` L74 `trig`: optional one-shot (`flex` = `hairflip`,
    /// handled by `CharAnim::trigger`).
    pub trig: Option<String>,
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
        t0: e.field("t0").and_then(JVal::as_f64).map(|v| v as f32),
        trig: e.field("trig").and_then(JVal::as_str).map(str::to_string),
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

// ------------------------------------------------- M6 Task 72: live inputs

/// M6 Task 72 — runtime-measured lobby projection inputs (D-T62-2 / D-M6-7).
/// Upstream `showcase.js` measures these from the DOM every frame:
/// `_measureLobby`(L2677-2694) → `R.l/r/t/b` safe rect, and `_lobFrame`(L2670)
/// → `L.kFront = |sx - vx| * 0.5 * W` (px/metre at your mark). Neither is a
/// JSON field, so the native sandbox uses the honest defaults below (which
/// reproduce the previous `None` / `1.0` fallback); wasm overwrites them with
/// measured values in the DOM sink. Kept as a Resource for headless anchoring.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LobbyMeasure {
    /// Safe rectangle (left, right, top, bottom) in CSS px.
    pub safe: (f32, f32, f32, f32),
    /// Pixels per metre at your mark; JS initial `kFront: 150` (L1596).
    pub k_front: f32,
}

impl Default for LobbyMeasure {
    fn default() -> Self {
        // Review P1-2: a degenerate rect (0,0,0,0) would cull EVERY plate
        // (x>24 && x<-24). Native has no measured DOM, so default to the
        // "unmeasured" sentinel → projection passes `None` (≡ JS `!U`, no
        // culling). See `safe_opt()`.
        Self {
            safe: (0.0, 0.0, 0.0, 0.0),
            k_front: 150.0,
        }
    }
}

impl LobbyMeasure {
    /// The safe rect for projection. `None` (≡ JS `!U`, no culling) when the
    /// native sandbox has no measured DOM — the degenerate default sentinel.
    /// Wasm overwrites `safe` with the real `_measureLobby` rectangle.
    #[must_use]
    pub fn safe_opt(&self) -> Option<(f32, f32, f32, f32)> {
        let (l, r, t, b) = self.safe;
        if r > l && b > t {
            Some((l, r, t, b))
        } else {
            None
        }
    }

    /// Build a full-window safe rect from a window size (explicit measurement
    /// fallback; edge insets 24/20/30 then apply, matching JS `U` present).
    #[must_use]
    pub fn full_window(width: f32, height: f32) -> Self {
        Self {
            safe: (0.0, width, 0.0, height),
            k_front: 150.0,
        }
    }
}

/// M6 Task 72 — the hub⇄room cross-dissolve state machine, ported from
/// `showcase.js` `_updateSet`(L2093-2121) + `render()`(L3055-3071). `k` ramps
/// 0↔1 at `dt / SET_FADE`; `covered` forces the composite opacity to 1 (skip
/// the eInOut fade) when nothing was drawn under the full-frame set. Pure
/// data (no GPU / no DOM), headless-testable.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SetFade {
    /// Cross-dissolve progress in [0, 1] (JS `L.k`).
    pub k: f32,
    /// Time since the set went off-screen (JS `L.gone`, release after 1.5 s).
    pub gone: f32,
    /// LobbySet resources/prewarm finished (JS `L.ready`).
    pub ready: bool,
    /// True when nothing was drawn under the full-frame set this frame.
    pub covered: bool,
}

impl Default for SetFade {
    fn default() -> Self {
        Self {
            k: 0.0,
            gone: 0.0,
            ready: false,
            covered: false,
        }
    }
}

impl SetFade {
    /// Advance the dissolve by `dt` seconds. `on` = mode ∈ {hub, lobby}.
    /// SET_FADE comes from `pedestal().set_fade` (lobbyset.json, zero hand-copy).
    pub fn advance(&mut self, dt: f64, on: bool, set_fade: f32) {
        let step = (dt.max(0.0) / set_fade.max(1e-4) as f64) as f32;
        self.k = if on && self.ready {
            (self.k + step).min(1.0)
        } else {
            (self.k - step).max(0.0)
        };
        if !on && self.k <= 0.0 {
            self.gone += dt as f32;
        } else {
            self.gone = 0.0;
        }
    }

    /// Composition opacity ≡ `covered ? 1 : eInOut(k)` (showcase.js L3069).
    /// `eInOut`(L37) is the quintic smoothstep `t³(t(6t−15)+10)`.
    #[must_use]
    pub fn opacity(&self) -> f32 {
        if self.covered {
            return 1.0;
        }
        let t = self.k.clamp(0.0, 1.0) as f64;
        (t * t * t * (t * (t * 6.0 - 15.0) + 10.0)) as f32
    }

    /// Whether the LobbySet can be released (JS `L.gone > 1.5`, L2103).
    #[must_use]
    pub fn should_release(&self) -> bool {
        self.gone > 1.5
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

/// Task 62: DOM nametag projection ≡ `showcase.js` L2056-2090
/// `lobbyAnchor` / `lobbySlotAnchor` / `_lobProject`. Pure math (no GPU):
/// `x/y` are pixel coordinates, `behind` is the WebGL NDC-z test, `s` is the
/// right-vector projected scale, and `vis` is cleared when the tag falls
/// outside the measured UI safe rect (inset 24/24/20/30 px, ≡ L2086-2088).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LobbyTag {
    pub x: f32,
    pub y: f32,
    pub behind: bool,
    pub s: f32,
    pub vis: bool,
    pub row: usize,
    pub slot: usize,
}

/// Task 62: one projected DOM plate's DATA (Task 64 owns the actual DOM node).
/// `free=true` is an empty lineup mark (`lobbySlotAnchor`, L2065-2073).
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyNameplate {
    pub tag: LobbyTag,
    pub name: String,
    pub team: u8,
    pub ready: bool,
    pub host: bool,
    pub you: bool,
    pub emote: Option<String>,
    pub free: bool,
}

/// Bevy-side data plane consumed by the Task 64 DOM overlay bridge. Keeping
/// it as a resource (not UI nodes) makes the whole projection headless-testable
/// and never draws a second copy over the layer-1 lobby camera.
#[derive(Resource, Default, Debug)]
pub struct LobbyNameplates(pub Vec<LobbyNameplate>);

fn project_ndc(p: Vec3, view: Mat4, proj: Mat4) -> Vec3 {
    let clip = proj * view * Vec4::new(p.x, p.y, p.z, 1.0);
    Vec3::new(clip.x, clip.y, clip.z) / clip.w
}

/// `ui` is `(l, r, t, b)` in pixels for the measured safe rect; `None` means
/// no UI rect test.
/// `k_front` ≡ `lob.kFront` (the lobby approach blend; callers pass the live
/// value, the `max(1, …)` guard is ported verbatim).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn project_lobby_tag(
    p: Vec3,
    pose: CameraPose,
    width: f32,
    height: f32,
    ui: Option<(f32, f32, f32, f32)>,
    k_front: f32,
    vis: bool,
    row: usize,
    slot: usize,
) -> LobbyTag {
    let eye = Vec3::from_array(pose.pos);
    let target = Vec3::from_array(pose.target);
    // glam 0.30+ `look_to_rh` requires a normalized direction (it no longer
    // normalizes internally); passing the raw eye→target vector scales the
    // translation term and pushes every NDC z past the far plane.
    let dir = (target - eye).normalize();
    let view = Mat4::look_to_rh(eye, dir, Vec3::Y);
    let proj = Mat4::perspective_rh(pose.fov.to_radians(), width / height, pose.near, pose.far);
    let ndc = project_ndc(p, view, proj);
    // `_d.setFromMatrixColumn(cam.matrixWorld, 0)` = camera right in world.
    let right = dir.cross(Vec3::Y).normalize();
    let shifted = project_ndc(p + right, view, proj);
    let x = (ndc.x + 1.0) * 0.5 * width;
    let y = (1.0 - ndc.y) * 0.5 * height;
    let s = (shifted.x - ndc.x).abs() * 0.5 * width / k_front.max(1.0);
    let inside =
        ui.is_none_or(|(l, r, t, b)| x > l + 24.0 && x < r - 24.0 && y > t + 20.0 && y < b + 30.0);
    LobbyTag {
        x,
        y,
        behind: ndc.z > 1.0,
        s,
        vis: inside && vis,
        row,
        slot,
    }
}

/// `lobbyAnchor` height ≡ showcase.js L2060:
/// `pos.y + M.y + plateH * M.sy` (PLATE_H from the extracted pedestal table).
#[must_use]
pub fn member_anchor(pos: [f32; 3], y: f32, sy: f32) -> Vec3 {
    let plate_h = pedestal().plate_h;
    Vec3::new(pos[0], pos[1] + y + plate_h * sy, pos[2])
}

/// `lobbySlotAnchor` height ≡ showcase.js L2072: `S.pos.y + PLATE_H - 0.35`.
#[must_use]
pub fn slot_anchor(pos: [f32; 3]) -> Vec3 {
    Vec3::new(pos[0], pos[1] + pedestal().plate_h - 0.35, pos[2])
}

/// Full lobby plate set: occupied marks project from the member anchor, empty
/// marks from `lobbySlotAnchor` (`showcase.js` L2056-2073). The eight rows are
/// always emitted in `spots()` order so a DOM consumer can key them by
/// `(row, slot)` without rebuilding its nodes on roster refresh.
#[must_use]
pub fn project_lobby_nameplates(
    state: &ShowcaseState,
    width: f32,
    height: f32,
    ui: Option<(f32, f32, f32, f32)>,
    k_front: f32,
) -> Vec<LobbyNameplate> {
    if state.mode != ShowcaseMode::Lobby || !state.full_frame {
        return Vec::new();
    }
    let pose = camera_pose(state.mode);
    spots()
        .into_iter()
        .enumerate()
        .map(|(slot, spot)| {
            let row = spot.team as usize;
            match state.members.iter().find(|m| m.slot == slot) {
                Some(m) => LobbyNameplate {
                    tag: project_lobby_tag(
                        member_anchor(spot.pos, 0.0, 1.0),
                        pose,
                        width,
                        height,
                        ui,
                        k_front,
                        true,
                        row,
                        slot % 4,
                    ),
                    name: m.name.clone(),
                    team: m.team,
                    ready: m.ready,
                    host: m.host,
                    you: m.you,
                    emote: m.emote.clone(),
                    free: false,
                },
                None => LobbyNameplate {
                    tag: project_lobby_tag(
                        slot_anchor(spot.pos),
                        pose,
                        width,
                        height,
                        ui,
                        k_front,
                        true,
                        row,
                        slot % 4,
                    ),
                    name: String::new(),
                    team: spot.team,
                    ready: false,
                    host: false,
                    you: false,
                    emote: None,
                    free: true,
                },
            }
        })
        .collect()
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
    /// Incremented by every `lobbyEmote` delivery so replaying the same name
    /// remains a new one-shot (JS `showcase.js` L2002-2005 pre-empts the old queue).
    pub emote_serial: u32,
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
                m.emote_serial = old.emote_serial;
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
            m.emote_serial = m.emote_serial.wrapping_add(1);
        }
        Some(e)
    }

    /// Consume a live `SessionEvent::Emote` (session.js L263/L372). The relay
    /// id is mapped to the SAME team-ordered slot `rows_to_members` produced,
    /// so the 3D preview and DOM plate both follow the player who sent it.
    pub fn apply_emote_event(&mut self, lobby: &Lobby, id: &SessionId, name: &str) -> bool {
        if self.mode != ShowcaseMode::Lobby || !self.full_frame {
            return false;
        }
        let Some(slot) = slot_for_player(lobby, id) else {
            return false;
        };
        self.emote(slot, name).is_some()
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
                emote_serial: 0,
            }
        })
        .collect()
}

/// Team-ordered lobby-player → lineup slot, identical to `rows_to_members`
/// (project() preserves lobby player order). Kept here so a network emote event
/// can target the same slot without mutating `inkwave_sim`.
#[must_use]
pub fn slot_for_player(lobby: &Lobby, id: &SessionId) -> Option<usize> {
    let mut front = 0usize;
    let mut back = 4usize;
    for p in &lobby.players {
        let slot = if p.team == TeamSel::One {
            let s = back.min(7);
            back = back.saturating_add(1);
            s
        } else {
            let s = front.min(3);
            front = front.saturating_add(1);
            s
        };
        if &p.id == id {
            return Some(slot);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::online::{HubView, LobbyView};

    /// TR-72.1: cross-dissolve k ramps at dt/SET_FADE (showcase.js L2097),
    /// eInOut is the quintic smoothstep (L37), covered bypasses the fade (L3069).
    #[test]
    fn task72_set_fade_ramp_and_covered() {
        let sf = pedestal().set_fade;
        let mut f = SetFade {
            ready: true,
            ..Default::default()
        };
        // on + ready → k rises to 1 over exactly set_fade seconds.
        f.advance(sf as f64, true, sf);
        assert!((f.k - 1.0).abs() < 1e-5, "one SET_FADE step reaches k=1");
        assert!((f.opacity() - 1.0).abs() < 1e-5);
        // off → k falls back to 0, then gone accumulates for release.
        f.advance(sf as f64, false, sf);
        assert!(f.k.abs() < 1e-5);
        f.advance(1.6, false, sf);
        assert!(f.should_release(), "gone > 1.5 releases the LobbySet");
        // covered forces opacity 1 regardless of k.
        let c = SetFade {
            covered: true,
            ..Default::default()
        };
        assert_eq!(c.opacity(), 1.0);
        // eInOut(0.5) == 0.5 (quintic smoothstep is symmetric).
        let h = SetFade {
            k: 0.5,
            ..Default::default()
        };
        assert!((h.opacity() - 0.5).abs() < 1e-5);
    }

    /// TR-72.1: LobbyMeasure defaults mirror the JS initial kFront:150 and a
    /// full-window safe rect; the measured struct feeds pure projection.
    #[test]
    fn task72_lobby_measure_defaults_match_js() {
        let m = LobbyMeasure::default();
        assert_eq!(m.k_front, 150.0, "JS kFront initial (showcase.js L1596)");
        // Review P1-2: unmeasured default must mean "no culling" (≡ JS !U).
        assert_eq!(
            m.safe_opt(),
            None,
            "native unmeasured → no safe-rect culling"
        );
        let full = LobbyMeasure::full_window(1920.0, 1080.0);
        assert_eq!(full.safe, (0.0, 1920.0, 0.0, 1080.0));
        assert_eq!(full.safe_opt(), Some((0.0, 1920.0, 0.0, 1080.0)));
    }

    /// TR-62.1: `flex` is the only emote with a start offset and a trigger
    /// (`lobbyset.json` ← showcase.js L74). Both must survive extraction.
    #[test]
    fn task62_emote_t0_and_trigger_are_extracted() {
        let flex = emote("flex").expect("flex emote");
        assert_eq!(flex.dance, "victory");
        assert_eq!(flex.t0, Some(2.86));
        assert_eq!(flex.trig.as_deref(), Some("hairflip"));
        assert_eq!(flex.hop, 0.12);
        let wave = emote("wave").expect("wave emote");
        assert_eq!(wave.t0, None);
        assert_eq!(wave.trig, None);
        assert_eq!(wave.variant, 2.0);
    }

    /// The anchor heights are JS literals (`PLATE_H - 0.35` / `plateH*sy`),
    /// and the projection is checked against the camera's own look-at target:
    /// it must land at screen centre, with the right-shifted point to its
    /// right, a behind-camera point flagged, and UI occlusion honoured.
    #[test]
    fn task62_nametag_projection_matches_js_rules() {
        let pose = camera_pose(ShowcaseMode::Lobby);
        let target = Vec3::from_array(pose.target);
        let center = project_lobby_tag(target, pose, 1920.0, 1080.0, None, 1.0, true, 1, 2);
        assert!((center.x - 960.0).abs() < 0.5, "look-at target → centre x");
        assert!((center.y - 540.0).abs() < 0.5, "look-at target → centre y");
        assert!(!center.behind && center.vis);
        assert_eq!((center.row, center.slot), (1, 2));
        let shifted = project_lobby_tag(
            target + Vec3::X,
            pose,
            1920.0,
            1080.0,
            None,
            1.0,
            true,
            0,
            0,
        );
        assert!(
            shifted.x > center.x,
            "camera-right shift increases screen x"
        );
        assert!(shifted.s > 0.0, "scale is derived from the right vector");
        let scaled = project_lobby_tag(
            target + Vec3::X,
            pose,
            1920.0,
            1080.0,
            None,
            2.0,
            true,
            0,
            0,
        );
        assert!(
            (scaled.s - shifted.s * 0.5).abs() < 1e-3,
            "kFront divides the projected scale (JS max(1, kFront))"
        );
        // A point behind the eye is flagged (WebGL NDC z > 1).
        let behind = project_lobby_tag(
            Vec3::from_array(pose.pos) + (Vec3::from_array(pose.pos) - target),
            pose,
            1920.0,
            1080.0,
            None,
            1.0,
            true,
            0,
            0,
        );
        assert!(behind.behind);
        // The measured UI rect is a SAFE area (l+24,r-24,t+20,b+30): the
        // centre stays visible inside it and is hidden outside it.
        let safe = project_lobby_tag(
            target,
            pose,
            1920.0,
            1080.0,
            Some((900.0, 1020.0, 500.0, 580.0)),
            1.0,
            true,
            0,
            0,
        );
        assert!(safe.vis, "centre falls inside the inset UI safe rect");
        let outside = project_lobby_tag(
            target,
            pose,
            1920.0,
            1080.0,
            Some((0.0, 100.0, 0.0, 100.0)),
            1.0,
            true,
            0,
            0,
        );
        assert!(!outside.vis, "centre outside the UI safe rect is hidden");
        // Anchors use the extracted plate height.
        let ph = pedestal().plate_h;
        assert_eq!(
            slot_anchor([1.0, 2.0, 3.0]),
            Vec3::new(1.0, 2.0 + ph - 0.35, 3.0)
        );
        assert_eq!(
            member_anchor([1.0, 2.0, 3.0], 0.5, 2.0),
            Vec3::new(1.0, 2.5 + ph * 2.0, 3.0)
        );
    }

    /// TR-62.1: the plate data plane follows the roster, keeps ready/host/you
    /// flags and always emits all eight marks (empty slots are free markers).
    #[test]
    fn task62_nameplates_follow_roster_and_flags() {
        let mut v = view(
            OnlineScreen::Lobby,
            "AB3CD",
            vec![row("me", 0, true), row("rival", 1, false)],
            true,
        );
        v.lobby.rows[0].host = true;
        let mut s = ShowcaseState::from_online_view(&v, "high");
        assert!(s.emote(0, "wave").is_some());
        let plates = project_lobby_nameplates(&s, 1920.0, 1080.0, None, 1.0);
        assert_eq!(plates.len(), 8);
        assert_eq!(plates[0].name, "me");
        assert!(plates[0].ready && plates[0].host && plates[0].you && !plates[0].free);
        assert_eq!(plates[0].emote.as_deref(), Some("wave"));
        assert_eq!((plates[0].tag.row, plates[0].tag.slot), (0, 0));
        assert!(plates[1].free && plates[1].name.is_empty());
        assert_eq!((plates[1].tag.row, plates[1].tag.slot), (0, 1));
        assert_eq!(plates[4].name, "rival");
        assert_eq!((plates[4].tag.row, plates[4].tag.slot), (1, 0));
        assert!(
            plates
                .iter()
                .all(|p| p.tag.x.is_finite() && p.tag.y.is_finite())
        );

        let empty = ShowcaseState::from_online_view(
            &view(OnlineScreen::Lobby, "AB3CD", vec![], false),
            "high",
        );
        let empty_plates = project_lobby_nameplates(&empty, 1920.0, 1080.0, None, 1.0);
        assert_eq!(empty_plates.len(), 8);
        assert!(empty_plates.iter().all(|p| p.free));

        // Overlay/low-tier modes never leak stale plates to the DOM bridge.
        let mut overlay = s.clone();
        overlay.mode = ShowcaseMode::Loadout;
        overlay.refresh_frame();
        assert!(project_lobby_nameplates(&overlay, 1920.0, 1080.0, None, 1.0).is_empty());
    }

    /// TR-62.1 P0 guard: a real `SessionEvent::Emote { id, name }` must reach
    /// the same team-ordered slot the roster projected, and a repeat delivery
    /// must produce a new serial (same-name replay).
    #[test]
    fn task62_emote_event_maps_peer_id_to_slot_and_replays() {
        let player = |id: &str, team: TeamSel, you: bool| inkwave_sim::net::session::LobbyPlayer {
            id: id.to_string(),
            name: id.to_string(),
            team,
            weapon: "shooter".to_string(),
            sub: None,
            special: None,
            style: None,
            ready: false,
            ping: 0,
            host: false,
            you,
        };
        let lobby = Lobby {
            players: vec![
                player("p0", TeamSel::Zero, true),
                player("p1", TeamSel::One, false),
            ],
            ..Lobby::default()
        };
        let v = view(
            OnlineScreen::Lobby,
            "AB3CD",
            vec![row("p0", 0, true), row("p1", 1, false)],
            true,
        );
        let mut s = ShowcaseState::from_online_view(&v, "high");
        assert!(s.apply_emote_event(&lobby, &"p1".to_string(), "wave"));
        assert_eq!(
            s.members
                .iter()
                .find(|m| m.slot == 4)
                .unwrap()
                .emote
                .as_deref(),
            Some("wave")
        );
        assert_eq!(
            s.members.iter().find(|m| m.slot == 4).unwrap().emote_serial,
            1
        );
        assert!(s.apply_emote_event(&lobby, &"p1".to_string(), "wave"));
        assert_eq!(
            s.members.iter().find(|m| m.slot == 4).unwrap().emote_serial,
            2
        );
        assert!(!s.apply_emote_event(&lobby, &"ghost".to_string(), "wave"));
        assert!(!s.apply_emote_event(&lobby, &"p1".to_string(), "unknown"));

        s.mode = ShowcaseMode::Hub;
        s.refresh_frame();
        assert!(!s.apply_emote_event(&lobby, &"p1".to_string(), "wave"));
    }

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
    fn task67_material_params_match_upstream_literals() {
        // Values pinned from the READ-ONLY lobbySet-mats.js / lobbySet.js; the
        // crosscheck re-parses them independently (242 checks).
        let r = wet_reflection();
        assert_eq!(
            (
                r.env_map_intensity,
                r.fresnel_f0,
                r.fresnel_scale,
                r.fresnel_pow
            ),
            (1.0, 0.02, 0.98, 5.0)
        );
        assert_eq!((r.k_damp, r.k_ink, r.water_mix), (0.5, 0.5, [0.8, 1.0]));

        let n = neon();
        assert_eq!(n.core_color, [0.1, 1.0, 0.35]);
        assert_eq!(n.intensity, [7.0, 7.0, 1.0, 5.0]);
        assert_eq!((n.team_a, n.team_b), ("#ff8a14", "#2f5bff"));

        let s = steam();
        assert_eq!((s.cards, s.card_height, s.seed_step), (3, 3.4, 1.7));
        assert_eq!((s.x, s.z), (-2.8, -6.0));
        assert_eq!(s.color, [0.2, 0.19, 0.21]);
        assert_eq!(s.time_scale, 0.35);
        assert_eq!(
            (s.noise_scale, s.alpha_edge, s.alpha_scale),
            ([1.6, 3.0], [0.4, 0.85], 0.6)
        );

        let p = lobby_pmrem();
        assert_eq!((p.sigma, p.near, p.far, p.size), (0.04, 0.1, 100.0, 128));
        assert_eq!(p.position, [0.8, 1.4, -1.2]);
        assert_eq!((p.team_normalize, p.environment_intensity), (4.0, 0.6));
        assert_eq!(
            (
                p.env_card_k.door,
                p.env_card_k.neon,
                p.env_card_k.bulbs,
                p.env_card_k.vending
            ),
            (5.0, 4.0, 3.0, 2.5)
        );
    }

    #[test]
    fn task67_low_tier_keeps_reflection_and_steam_off() {
        // D-T67-1 / Task 61 low保底: reflection=0 and steam=false must hold so
        // the headless / low GPU path never pays for the reflection pass.
        let low = lobby_quality("low").unwrap();
        assert_eq!(low.reflection, 0.0);
        assert!(!low.steam);
        let medium = lobby_quality("medium").unwrap();
        assert_eq!(medium.reflection, 0.34);
        assert!(medium.steam);
        let high = lobby_quality("high").unwrap();
        assert_eq!(high.reflection, 0.5);
        assert!(high.steam);
        // Lobby PMREM is distinct from the world PMREM (D-T67-2).
        assert_eq!(lobby_pmrem().size, 128);
        assert_eq!(crate::env_bake::pmrem().size, 256);
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
