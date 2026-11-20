//! M6 Task 73 — the `showFps` setting consumer (data plane). The real FPS
//! text node is `LabelRole("fps")` in the HUD (ui/mod.rs); this module holds
//! the pure display logic + resource so it is headless-anchorable. The actual
//! glyph rendering is a user-machine review item (D-M6-11).

use bevy::prelude::Resource;

/// Smoothed FPS display state. `text` is the exact string written to the
/// `LabelRole("fps")` node each frame (empty when `show_fps=false`).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct FpsDisplay {
    /// Exponential moving average of instantaneous FPS.
    pub smoothed: f64,
    /// Last resolved label text (anchor surface + real node source).
    pub text: String,
}

impl Default for FpsDisplay {
    fn default() -> Self {
        Self {
            smoothed: 60.0,
            text: String::new(),
        }
    }
}

impl FpsDisplay {
    /// EMA smoothing factor: JS HUD uses a ~0.5 s window; at 60 fps that is
    /// an α of ≈0.1 (upstream `_updFps`). Kept as a named constant (no magic).
    pub const SMOOTH_ALPHA: f64 = 0.1;

    /// Advance the EMA with this frame's delta and return the display text.
    /// `show_fps=false` → empty string (label hidden), no smoothing work.
    pub fn update(&mut self, delta_secs: f64, show_fps: bool) -> String {
        if !show_fps || !delta_secs.is_finite() || delta_secs <= 0.0 {
            self.text.clear();
            return self.text.clone();
        }
        let inst = 1.0 / delta_secs;
        self.smoothed += Self::SMOOTH_ALPHA * (inst - self.smoothed);
        self.text = fps_text(true, self.smoothed);
        self.text.clone()
    }
}

/// Pure text resolution: `show_fps=false` always yields "" (hidden label),
/// otherwise the rounded smoothed FPS string. Used by both the Bevy system and
/// the headless anchors (TR-73.1).
#[must_use]
pub fn fps_text(show_fps: bool, smoothed: f64) -> String {
    if !show_fps {
        String::new()
    } else {
        format!("{} FPS", smoothed.round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task73_fps_text_setting_gates_visibility() {
        assert_eq!(fps_text(false, 59.6), "", "showFps=off hides the label");
        assert_eq!(fps_text(true, 59.6), "60 FPS");
        let mut d = FpsDisplay::default();
        assert_eq!(d.update(1.0 / 30.0, false), "", "off → no text");
        let t = d.update(1.0 / 60.0, true);
        assert!(t.ends_with(" FPS"));
    }

    #[test]
    fn task73_fps_ema_is_stable_and_rejects_bad_delta() {
        let mut d = FpsDisplay::default();
        let before = d.smoothed;
        assert_eq!(d.update(0.0, true), "", "zero delta → no text");
        assert_eq!(d.update(f64::NAN, true), "", "NaN delta → no text");
        assert_eq!(d.smoothed, before, "bad deltas never perturb the EMA");
    }
}
