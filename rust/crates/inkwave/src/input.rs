//! Task 12 — player input wiring (keyboard + mouse → `ActorInput`).
//!
//! Key mapping mirrors upstream `src/game/player.js` L102-126:
//!   W/A/S/D + arrows  move (camera-relative)
//!   Space             jump
//!   Shift             squid form
//!   mouse left        fire
//!   mouse right / E   sub (dive through ink)
//!   F / Q             special
//!   Esc / P           pause (TR-12.1; upstream main.js L437 accepts both —
//!                     the browser consumes Esc while pointer-locked, so P is
//!                     the reliable web key. Native window close still exits.)
//!   G                 toggle mouse-look grab (M1 stand-in for the web
//!                     pointer lock; `CursorGrabMode::Confined` keeps the
//!                     cursor inside the window like a locked pointer does)
//!
//! Look parameters read the upstream defaults (settings L563-566):
//! `sensitivity = 1.0` (0.2..3), horizontal `fov = 82` (65..100).

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use inkwave_sim::actor::ActorInput;

/// Upstream default mouse multiplier (settings.js `sensitivity: 1.0`).
pub const DEFAULT_SENSITIVITY: f32 = 1.0;
/// Base rad/texel at sensitivity 1.0 (cameraRig.js look rate, kept from Task 10).
const LOOK_RATE: f32 = 0.0021;

/// User-tunable look settings (defaults = upstream; Task 13 menu edits these).
#[derive(Resource)]
pub struct LookSettings {
    /// Mouse multiplier (upstream 0.2..3).
    pub sensitivity: f32,
    /// Horizontal FOV in degrees at the 16:9 reference (upstream 65..100).
    pub fov_h: f32,
}

impl Default for LookSettings {
    fn default() -> Self {
        Self {
            sensitivity: DEFAULT_SENSITIVITY,
            fov_h: crate::world::BASE_FOV_H,
        }
    }
}

/// Live controller state for the local player (slot 0).
#[derive(Resource, Default)]
pub struct PlayerControls {
    /// Latest mapped intent (written every frame by [`map_input`]).
    pub intent: ActorInput,
    /// View yaw/pitch driven by mouse look (shared with the camera rig).
    pub yaw: f32,
    pub pitch: f32,
    /// Mouse-look grab on/off (G toggles; upstream web uses pointer lock).
    pub look_enabled: bool,
    /// Match frozen (owned by the UI state machine, Task 13: `screen != Hud`).
    pub paused: bool,
    /// Esc/P was pressed this frame; the UI state machine consumes it to
    /// flip Hud <-> Pause (Task 13). `map_input` only raises the edge.
    pub pause_edge: bool,
}

/// Camera-relative move basis (player.js L118-120):
/// forward = (sin y, 0, cos y), right = (-cos y, 0, sin y).
#[must_use]
pub fn move_basis(yaw: f32) -> (Vec3, Vec3) {
    let (s, c) = (yaw.sin(), yaw.cos());
    (Vec3::new(s, 0.0, c), Vec3::new(-c, 0.0, s))
}

/// Compose the world-space move dir from WASD/arrow bits (player.js L102-108:
/// magnitude > 1 is normalised, matching the JS stick clamp).
#[must_use]
pub fn move_dir(
    fwd_held: bool,
    back_held: bool,
    left_held: bool,
    right_held: bool,
    yaw: f32,
) -> Vec3 {
    let (f, r) = move_basis(yaw);
    let mut m = Vec3::ZERO;
    if fwd_held {
        m += f;
    }
    if back_held {
        m -= f;
    }
    if right_held {
        m += r;
    }
    if left_held {
        m -= r;
    }
    if m.length() > 1.0 {
        m = m.normalize();
    }
    m
}

fn any_key(keys: &ButtonInput<KeyCode>, codes: &[KeyCode]) -> bool {
    codes.iter().any(|k| keys.pressed(*k))
}

/// One frame of input: mouse look + key/mouse → `PlayerControls.intent`.
pub fn map_input(
    mut pc: ResMut<PlayerControls>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    settings: Res<LookSettings>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // Esc / P raise the pause edge (TR-12.1 / Task 13): the UI state machine
    // owns the Hud <-> Pause flip. The OS window close button still exits via
    // Bevy's default `AppExit` on window close.
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyP) {
        pc.pause_edge = true;
    }
    // G toggles mouse-look grab (M1 stand-in for web pointer lock).
    if keys.just_pressed(KeyCode::KeyG) {
        pc.look_enabled = !pc.look_enabled;
        if let Ok(mut c) = cursor.single_mut() {
            if pc.look_enabled {
                c.grab_mode = CursorGrabMode::Confined;
                c.visible = false;
            } else {
                c.grab_mode = CursorGrabMode::None;
                c.visible = true;
            }
        }
    }

    if pc.look_enabled {
        let k = LOOK_RATE * settings.sensitivity;
        for m in motion.read() {
            pc.yaw -= m.delta.x * k;
            pc.pitch = (pc.pitch - m.delta.y * k).clamp(-1.05, 1.15);
        }
    } else {
        motion.clear();
    }

    // player.js L102-105: WASD + arrows are equivalent.
    let fwd = keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp);
    let back = keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown);
    let left = keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft);
    let right = keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight);

    // sim uses glam 0.30, Bevy re-exports 0.32 — bridge via arrays.
    let md = move_dir(fwd, back, left, right, pc.yaw);
    pc.intent = ActorInput {
        move_dir: glam::Vec3::from_array(md.to_array()),
        jump: keys.pressed(KeyCode::Space),
        squid: any_key(&keys, &[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        fire: mouse.pressed(MouseButton::Left),
        sub: mouse.pressed(MouseButton::Right) || keys.pressed(KeyCode::KeyE),
        special: keys.pressed(KeyCode::KeyF) || keys.pressed(KeyCode::KeyQ),
    };
}

/// Scroll wheel nudges sensitivity (0.2..3, upstream settings range).
pub fn tune_sensitivity(mut settings: ResMut<LookSettings>, mut wheel: MessageReader<MouseWheel>) {
    for w in wheel.read() {
        settings.sensitivity = (settings.sensitivity + w.y * 0.1).clamp(0.2, 3.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_basis_matches_js_forward_right() {
        // player.js L118: forward = (sy, 0, cy); yaw 0 → +Z.
        let (f, r) = move_basis(0.0);
        assert_eq!(f, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(r, Vec3::new(-1.0, 0.0, 0.0));
        // yaw 90° → forward +X, right +Z? (sin=1, cos=0) f=(1,0,0), r=(0,0,1)
        let (f, r) = move_basis(std::f32::consts::FRAC_PI_2);
        assert!(f.distance(Vec3::X) < 1e-6);
        assert!(r.distance(Vec3::Z) < 1e-6);
    }

    #[test]
    fn diagonal_is_normalised_like_the_js_stick() {
        let m = move_dir(true, false, false, true, 0.0);
        assert!((m.length() - 1.0).abs() < 1e-5, "len {}", m.length());
        let single = move_dir(true, false, false, false, 0.0);
        assert!((single.length() - 1.0).abs() < 1e-5);
        let idle = move_dir(false, false, false, false, 0.7);
        assert_eq!(idle, Vec3::ZERO);
    }

    #[test]
    fn move_dir_decomposes_to_camera_axes() {
        // yaw 0: W → +Z, D → -X (right = (-cos,0,sin) = (-1,0,0)).
        let w = move_dir(true, false, false, false, 0.0);
        assert!(w.distance(Vec3::Z) < 1e-6);
        let d = move_dir(false, false, false, true, 0.0);
        assert!(d.distance(Vec3::NEG_X) < 1e-6);
        // S cancels W.
        let c = move_dir(true, true, false, false, 1.23);
        assert_eq!(c, Vec3::ZERO);
    }
}
