//! Task 59 — settings state, persistence and boot application.
//!
//! Upstream JS (`src/main.js` L41-42, L59, L347-355, L398-406):
//! `DEFAULT_SETTINGS` + `localStorage['inkwave.settings']`, merged as
//! `{...DEFAULT_SETTINGS, ...saved}`; `_setSettings` writes immediately and
//! dispatches quality / audio / palette changes. The defaults table and the
//! four QUALITY presets are already extracted into `assets/environment.json`
//! (`extract-environment.mjs` slices `config.js` L561-591, source.commit
//! pinned) — this module reads them through `include_str!` so no number is
//! hand-copied (NFR-M5-6): a missing extracted key is a hard error, never a
//! silent literal fallback.
//!
//! Persistence carriers (deviation D-T59-1, registered in REVIEW_CHECKLIST):
//!   - native: a JSON file (default `$HOME/.inkwave/settings.json`, override
//!     `INKWAVE_SETTINGS_PATH`);
//!   - wasm32: `window.localStorage['inkwave.rust.settings']` (D-T59-4: a
//!     **separate key** from the JS one, so fields the Rust port does not own
//!     — `padSensitivity`, `colorblind`, `minimap`, `matchLength`, `lastMode`,
//!     `difficulty`, `rumble`, `aimAssistMouse`, `fpsCap` and the runtime
//!     `fullscreen`/`stageTimes`/`seenWeapons` keys — are never clobbered).
//!
//! There is no serde in this crate; decoding uses a tolerant per-key scanner
//! (a single unparseable value, e.g. `1e999`, must not discard the whole
//! document — JS `loadJSON` keeps the other keys) and encoding is a
//! fixed-field writer.
//!
//! `INKWAVE_QUALITY` / `INKWAVE_THEME` remain **startup overrides** (≡ the
//! README §7 headless matrix): they live in `boot_*` fields, are applied by
//! consumers, and are deliberately NEVER written into the document. A user
//! explicitly cycling the quality / theme row clears the corresponding boot
//! override (an explicit action wins over a startup flag).

use bevy::prelude::Resource;
use inkwave_sim::net::wire::{JVal, parse_json};

const DOC: &str = include_str!("../../../assets/environment.json");
/// D-T59-4: separate from the JS `inkwave.settings` key (see module docs).
#[cfg(target_arch = "wasm32")]
pub const STORAGE_KEY: &str = "inkwave.rust.settings";
/// Native-only boot override for the config file path.
#[cfg(not(target_arch = "wasm32"))]
pub const PATH_ENV: &str = "INKWAVE_SETTINGS_PATH";
/// ≡ menus.js L1976-2000 `_slider` (the four sliders wired here use step 0.05).
pub const SLIDER_STEP: f64 = 0.05;
/// Key lists pinned to the extracted table by `quality_names_match_table`.
pub const QUALITY_NAMES: [&str; 4] = ["low", "medium", "high", "ultra"];
pub const THEME_NAMES: [&str; 3] = ["day", "sunset", "golden"];

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Settings {
    pub quality: String,
    /// Rust-side extension (D-T59-2): the JS settings screen has no theme row.
    pub theme: String,
    pub shadows: bool,
    pub bloom: bool,
    /// ≡ audio.js L165-172 `setVolumes`: linear sliders → `taper(v) = v^1.5`
    /// gains with τ = 0.04 s smoothing.
    pub master: f64,
    pub music: f64,
    pub sfx: f64,
    pub sensitivity: f64,
    /// Stored/persisted but not applied to the camera rig (D-T59-3).
    pub fov: f64,
    pub invert_y: bool,
    pub aim_assist: f64,
    pub show_fps: bool,
    pub camera_shake: f64,
    /// Startup-only overrides — excluded from `to_json` (P0-1 fix).
    pub boot_quality: Option<String>,
    pub boot_theme: Option<String>,
    /// Optional per-instance storage path override (test seam for the REAL
    /// `save()` chain; never serialized, no process-global state).
    pub storage_path: Option<std::path::PathBuf>,
}

fn defaults_json() -> &'static JVal {
    // `parse_json` borrows nothing; the document is a compile-time constant,
    // so a `OnceLock<JVal>` keeps the owned subtree alive for the process.
    static DOC_ROOT: std::sync::OnceLock<JVal> = std::sync::OnceLock::new();
    DOC_ROOT.get_or_init(|| {
        let root = parse_json(DOC).expect("environment.json parses");
        root.field("defaultSettings")
            .expect("environment.json defaultSettings")
            .clone()
    })
}

/// Extracted values are mandatory: fail fast instead of silently hand-copying
/// a default (NFR-M5-6).
fn req_num(d: &JVal, k: &str) -> f64 {
    d.field(k)
        .and_then(JVal::as_f64)
        .unwrap_or_else(|| panic!("environment.json defaultSettings.{k} missing / not a number"))
}
fn req_bool(d: &JVal, k: &str) -> bool {
    d.field(k)
        .and_then(JVal::as_bool)
        .unwrap_or_else(|| panic!("environment.json defaultSettings.{k} missing / not a bool"))
}
fn req_text(d: &JVal, k: &str) -> String {
    d.field(k)
        .and_then(JVal::as_str)
        .unwrap_or_else(|| panic!("environment.json defaultSettings.{k} missing / not a string"))
        .to_string()
}

/// Tolerant per-key scanner (P0-3 fix): `serde_json` rejects an overflowing
/// literal such as `1e999` for the WHOLE document, while JS `JSON.parse`
/// yields `Infinity` and keeps the other keys. Extract only our own scalar
/// keys as raw text, then let `sanitize()` clamp/reject each independently.
fn field_raw(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let i = src.find(&needle)? + needle.len();
    let after = &src[i..];
    let colon = after.find(':')? + 1;
    let value = after[colon..].trim_start();
    let end = value.find([',', '}', '\n', '\r']).unwrap_or(value.len());
    Some(value[..end].trim().to_string())
}
fn scan_num(src: &str, key: &str, d: f64) -> f64 {
    field_raw(src, key)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(d)
}
fn scan_bool(src: &str, key: &str, d: bool) -> bool {
    field_raw(src, key)
        .and_then(|v| match v.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .unwrap_or(d)
}
fn scan_text(src: &str, key: &str, d: &str) -> String {
    field_raw(src, key)
        .and_then(|v| {
            let t = v.trim_matches('"').trim_matches('\'').to_string();
            (!t.is_empty()).then_some(t)
        })
        .unwrap_or_else(|| d.to_string())
}

impl Default for Settings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Settings {
    #[must_use]
    pub fn defaults() -> Self {
        let d = defaults_json();
        Self {
            quality: req_text(d, "quality"),
            theme: "day".to_string(),
            shadows: req_bool(d, "shadows"),
            bloom: req_bool(d, "bloom"),
            master: req_num(d, "master"),
            music: req_num(d, "music"),
            sfx: req_num(d, "sfx"),
            sensitivity: req_num(d, "sensitivity"),
            fov: req_num(d, "fov"),
            invert_y: req_bool(d, "invertY"),
            aim_assist: req_num(d, "aimAssist"),
            show_fps: req_bool(d, "showFps"),
            camera_shake: req_num(d, "cameraShake"),
            boot_quality: None,
            boot_theme: None,
            storage_path: None,
        }
    }

    /// ≡ `{...DEFAULT_SETTINGS, ...saved}`: unknown / malformed keys keep
    /// their extracted default; one bad key cannot discard the rest (P0-3).
    #[must_use]
    pub fn from_json(src: &str) -> Self {
        let mut s = Self::defaults();
        s.quality = scan_text(src, "quality", &s.quality);
        s.theme = scan_text(src, "theme", &s.theme);
        s.shadows = scan_bool(src, "shadows", s.shadows);
        s.bloom = scan_bool(src, "bloom", s.bloom);
        s.master = scan_num(src, "master", s.master);
        s.music = scan_num(src, "music", s.music);
        s.sfx = scan_num(src, "sfx", s.sfx);
        s.sensitivity = scan_num(src, "sensitivity", s.sensitivity);
        s.fov = scan_num(src, "fov", s.fov);
        s.invert_y = scan_bool(src, "invertY", s.invert_y);
        s.aim_assist = scan_num(src, "aimAssist", s.aim_assist);
        s.show_fps = scan_bool(src, "showFps", s.show_fps);
        s.camera_shake = scan_num(src, "cameraShake", s.camera_shake);
        s.sanitize();
        s
    }

    /// Fixed-field writer — `boot_*` are startup-only and intentionally absent.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut s = self.clone();
        s.sanitize();
        format!(
            concat!(
                "{{\"quality\":\"{}\",\"theme\":\"{}\",\"shadows\":{},\"bloom\":{},",
                "\"master\":{:.4},\"music\":{:.4},\"sfx\":{:.4},",
                "\"sensitivity\":{:.4},\"fov\":{:.4},\"invertY\":{},",
                "\"aimAssist\":{:.4},\"showFps\":{},\"cameraShake\":{:.4}}}"
            ),
            s.quality,
            s.theme,
            s.shadows,
            s.bloom,
            s.master,
            s.music,
            s.sfx,
            s.sensitivity,
            s.fov,
            s.invert_y,
            s.aim_assist,
            s.show_fps,
            s.camera_shake,
        )
    }

    /// Clamp every slider to its JS range, whitelist the enums and replace
    /// non-finite numbers with the extracted default (≡ `_slider` /
    /// `setVolumes` guards).
    pub fn sanitize(&mut self) {
        let d = Self::defaults();
        let clamp_num = |v: f64, lo: f64, hi: f64, fallback: f64| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                fallback
            }
        };
        self.master = clamp_num(self.master, 0.0, 1.0, d.master);
        self.music = clamp_num(self.music, 0.0, 1.0, d.music);
        self.sfx = clamp_num(self.sfx, 0.0, 1.0, d.sfx);
        self.sensitivity = clamp_num(self.sensitivity, 0.2, 3.0, d.sensitivity);
        self.fov = clamp_num(self.fov, 65.0, 100.0, d.fov);
        self.aim_assist = clamp_num(self.aim_assist, 0.0, 1.0, d.aim_assist);
        self.camera_shake = clamp_num(self.camera_shake, 0.0, 1.0, d.camera_shake);
        if !QUALITY_NAMES.contains(&self.quality.as_str()) {
            self.quality = d.quality;
        }
        if !THEME_NAMES.contains(&self.theme.as_str()) {
            self.theme = "day".to_string();
        }
    }

    /// Consumers use the effective value: a startup override wins until the
    /// user explicitly changes that row (`apply_settings_row` clears it).
    #[must_use]
    pub fn effective_quality(&self) -> &str {
        self.boot_quality.as_deref().unwrap_or(&self.quality)
    }
    #[must_use]
    pub fn effective_theme(&self) -> &str {
        self.boot_theme.as_deref().unwrap_or(&self.theme)
    }

    /// ≡ `_slider`: step a 0.05-grid slider by ±1 and stay in range.
    pub fn step(&mut self, field: Slider, dir: i8) {
        let (v, lo, hi) = match field {
            Slider::Master => (&mut self.master, 0.0, 1.0),
            Slider::Music => (&mut self.music, 0.0, 1.0),
            Slider::Sfx => (&mut self.sfx, 0.0, 1.0),
            Slider::Sensitivity => (&mut self.sensitivity, 0.2, 3.0),
        };
        let grid = ((*v / SLIDER_STEP).round() + f64::from(dir)) * SLIDER_STEP;
        *v = grid.clamp(lo, hi);
        self.sanitize();
    }

    pub fn cycle_quality(&mut self) {
        self.boot_quality = None; // explicit user action wins over the startup flag
        let i = QUALITY_NAMES
            .iter()
            .position(|n| *n == self.effective_quality())
            .unwrap_or(2);
        self.quality = QUALITY_NAMES[(i + 1) % QUALITY_NAMES.len()].to_string();
    }

    pub fn cycle_theme(&mut self) {
        self.boot_theme = None;
        let i = THEME_NAMES
            .iter()
            .position(|n| *n == self.effective_theme())
            .unwrap_or(0);
        self.theme = THEME_NAMES[(i + 1) % THEME_NAMES.len()].to_string();
    }

    /// Startup overrides live in `boot_*` (never persisted). The two
    /// `Option<&str>` arguments make the pure rule testable without touching
    /// process-global env state.
    pub fn apply_env_overrides_from(&mut self, quality: Option<&str>, theme: Option<&str>) {
        self.boot_quality = quality
            .map(|q| q.to_ascii_lowercase())
            .filter(|q| QUALITY_NAMES.contains(&q.as_str()));
        self.boot_theme = theme.map(|t| match t.to_ascii_lowercase().as_str() {
            // ≡ EnvState::from_boot L444-448 (`dusk` is a JS alias).
            "sunset" | "dusk" => "sunset".to_string(),
            "golden" => "golden".to_string(),
            _ => "day".to_string(),
        });
    }

    fn apply_env_overrides(&mut self) {
        let q = std::env::var(crate::quality::ENV_QUALITY).ok();
        let t = std::env::var("INKWAVE_THEME").ok();
        self.apply_env_overrides_from(q.as_deref(), t.as_deref());
    }

    /// Boot load: defaults ← saved document ← startup env overrides.
    #[must_use]
    pub fn load() -> Self {
        let mut s = match read_storage() {
            Some(raw) => Self::from_json(&raw),
            None => Self::defaults(),
        };
        s.apply_env_overrides();
        s
    }

    /// Persist immediately (≡ main.js L400 `saveJSON(...)`); `boot_*` never
    /// enter the document (P0-1).
    pub fn save(&self) {
        let json = self.to_json();
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Per-instance path override (test seam) on native only.
            if let Some(path) = &self.storage_path {
                write_at(path, &json);
            } else {
                write_storage(&json);
            }
        }
        #[cfg(target_arch = "wasm32")]
        write_storage(&json);
    }

    /// Test seam: explicit-path round trip WITHOUT process env overrides, so
    /// the tests are deterministic on machines that set INKWAVE_THEME etc.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    #[must_use]
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => Self::from_json(&raw),
            Err(_) => Self::defaults(),
        }
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn save_to(&self, path: &std::path::Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.to_json());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slider {
    Master,
    Music,
    Sfx,
    Sensitivity,
}

// ------------------------------------------------------------------ storage

#[cfg(not(target_arch = "wasm32"))]
fn settings_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var(PATH_ENV) {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".inkwave")
        .join("settings.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn read_storage() -> Option<String> {
    std::fs::read_to_string(settings_path()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_storage(json: &str) {
    write_at(&settings_path(), json);
}

#[cfg(not(target_arch = "wasm32"))]
fn write_at(path: &std::path::Path, json: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}

#[cfg(target_arch = "wasm32")]
fn read_storage() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()
}

#[cfg(target_arch = "wasm32")]
fn write_storage(json: &str) {
    if let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = st.set_item(STORAGE_KEY, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_extracted_table() {
        let d = defaults_json();
        let s = Settings::defaults();
        assert_eq!(s.master, req_num(d, "master"));
        assert_eq!(s.music, req_num(d, "music"));
        assert_eq!(s.sfx, req_num(d, "sfx"));
        assert_eq!(s.quality, req_text(d, "quality"));
        assert_eq!(s.sensitivity, req_num(d, "sensitivity"));
        assert_eq!(s.fov, req_num(d, "fov"));
        assert_eq!(s.theme, "day");
        assert!(s.boot_quality.is_none() && s.boot_theme.is_none());
    }

    /// The hardcoded key lists must track the extracted quality/theme tables
    /// (NFR-M5-6: no silent drift).
    #[test]
    fn quality_names_match_table() {
        let root = parse_json(DOC).expect("environment.json parses");
        let q = root.field("quality").expect("quality table");
        for name in QUALITY_NAMES {
            assert!(q.field(name).is_some(), "quality.{name} missing upstream");
        }
        for name in THEME_NAMES {
            assert!(
                DOC.contains(&format!("\"{name}\"")),
                "theme {name} missing upstream"
            );
        }
    }

    #[test]
    fn from_json_merges_defaults_and_clamps() {
        let s =
            Settings::from_json("{\"master\":2.5,\"music\":-1,\"quality\":\"bogus\",\"sfx\":0.25}");
        assert_eq!(s.master, 1.0);
        assert_eq!(s.music, 0.0);
        let d = Settings::defaults();
        assert_eq!(s.quality, d.quality);
        assert_eq!(s.sfx, 0.25);
        assert_eq!(s.bloom, d.bloom); // untouched keys keep the extracted default
    }

    /// P0-3: a single overflowing literal must NOT discard the other keys,
    /// and `sanitize()` (not a whole-document parse failure) supplies the
    /// fallback for the bad key.
    #[test]
    fn non_finite_value_does_not_discard_the_document() {
        let s = Settings::from_json("{\"master\":0.3,\"sfx\":1e999,\"sensitivity\":null}");
        let d = Settings::defaults();
        assert_eq!(s.master, 0.3, "the good key survives");
        assert_eq!(s.sfx, d.sfx);
        assert_eq!(s.sensitivity, d.sensitivity);
    }

    #[test]
    fn slider_steps_match_the_js_005_grid() {
        let mut s = Settings::defaults();
        s.master = 0.83;
        s.step(Slider::Master, 1);
        // 0.83 snaps to 0.85's grid neighbour, then one +0.05 step → 0.90.
        assert!((s.master - 0.9).abs() < 1e-9);
        s.master = 1.0;
        s.step(Slider::Master, 1);
        assert_eq!(s.master, 1.0);
        s.master = 0.0;
        s.step(Slider::Master, -1);
        assert_eq!(s.master, 0.0);
        s.sensitivity = 0.2;
        s.step(Slider::Sensitivity, -1);
        assert_eq!(s.sensitivity, 0.2);
    }

    /// P0-1: startup overrides win for consumers, are absent from the saved
    /// document, and do not leak into a later boot.
    #[test]
    fn env_overrides_win_and_are_never_persisted() {
        let mut s = Settings::defaults();
        s.quality = "ultra".into();
        s.theme = "golden".into();
        s.apply_env_overrides_from(Some("LOW"), Some("dusk"));
        assert_eq!(s.effective_quality(), "low");
        assert_eq!(s.effective_theme(), "sunset");
        assert_eq!(
            crate::environment::EnvState::from_settings(&s).params.tier,
            "low"
        );
        let saved = s.to_json();
        assert!(!saved.contains("boot_"));
        let reloaded = Settings::from_json(&saved);
        assert_eq!(reloaded.quality, "ultra");
        assert_eq!(reloaded.theme, "golden");
        assert!(reloaded.boot_quality.is_none() && reloaded.boot_theme.is_none());
        assert_eq!(reloaded.effective_quality(), "ultra");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn settings_roundtrip_write_read_apply() {
        let path = std::env::temp_dir().join(format!(
            "inkwave-task59-settings-{}.json",
            std::process::id()
        ));
        let mut s = Settings::defaults();
        s.quality = "low".into();
        s.theme = "sunset".into();
        s.master = 0.35;
        s.music = 0.45;
        s.sfx = 0.55;
        s.sensitivity = 1.45;
        s.bloom = false;
        s.shadows = false;
        s.save_to(&path);
        let got = Settings::load_from(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, s, "write → read → apply must be identity");
    }

    /// The REAL `save()` / `load()` chain (the one `settings_clicks` calls),
    /// redirected to a private test path instead of mutating process env.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_load_chain_persists_without_boot_overrides() {
        let path =
            std::env::temp_dir().join(format!("inkwave-task59-chain-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut s = Settings::defaults();
        s.master = 0.45;
        s.quality = "medium".into();
        s.storage_path = Some(path.clone());
        s.apply_env_overrides_from(Some("ultra"), Some("golden"));
        s.save();
        let raw = std::fs::read_to_string(&path).expect("save() wrote the document");
        let got = Settings::from_json(&raw);
        let _ = std::fs::remove_file(&path);
        assert_eq!(got.master, 0.45);
        assert_eq!(got.quality, "medium", "boot override was not persisted");
        assert!(got.boot_quality.is_none());
        assert_eq!(
            crate::environment::EnvState::from_settings(&s).params.tier,
            "ultra"
        );
    }

    #[test]
    fn low_tier_keeps_the_headless_playable_path() {
        let mut s = Settings::defaults();
        s.quality = "low".into();
        s.bloom = false;
        s.shadows = false;
        let env = crate::environment::EnvState::from_settings(&s);
        assert_eq!(env.params.tier, "low");
        assert!(
            !env.params.post_enabled,
            "low tier must disable all post FX"
        );
        assert!(!env.shadows);
    }

    #[test]
    fn audio_buses_start_from_the_saved_sliders_via_taper() {
        let mut s = Settings::defaults();
        s.master = 0.25;
        s.music = 1.0;
        s.sfx = 0.0;
        let vols = crate::audio::music::AudioVolumes::from_settings(&s);
        assert_eq!(vols.master.value_at(0.0), crate::audio::music::taper(0.25));
        assert_eq!(vols.music.value_at(0.0), crate::audio::music::taper(1.0));
        assert_eq!(vols.sfx.value_at(0.0), crate::audio::music::taper(0.0));
    }
}
