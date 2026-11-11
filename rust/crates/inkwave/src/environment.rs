//! Task 55 batch A — environment / post-process / quality DATA layer.
//!
//! Pure data, no Bevy and no GPU: everything is read at runtime from the
//! extracted `rust/assets/environment.json` (produced by
//! `rust/tools/extract/extract-environment.mjs` from the READ-ONLY JS tree).
//! No number is hand-copied into Rust; the extractor slices and evaluates the
//! real `THEMES`, `BLOOM`, `GradeShader.uniforms`, `QUALITY`,
//! `DEFAULT_SETTINGS`, `TIMES` and `mapTheme()` source blocks.
//!
//! Batch B wires these into `world.rs` (lights / fog / clear colour / shadow
//! size) and the quality tier table; batch C registers the visual-only items
//! in `REVIEW_CHECKLIST` for a real-GPU review (NFR-M4-5).
#![allow(dead_code)] // batch B/C consumers land progressively

use std::sync::OnceLock;

use inkwave_sim::net::wire::{JVal, parse_json};

const DOC: &str = include_str!("../../../assets/environment.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("environment.json parses"))
}

fn field(v: &'static JVal, key: &str) -> &'static JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("environment.json: missing field {key}"))
}

fn f32_field(v: &'static JVal, key: &str) -> f32 {
    v.field(key)
        .and_then(JVal::as_f64)
        .unwrap_or_else(|| panic!("environment.json: missing numeric field {key}")) as f32
}

fn opt_f32(v: &'static JVal, key: &str) -> Option<f32> {
    v.field(key).and_then(JVal::as_f64).map(|x| x as f32)
}

fn bool_field(v: &'static JVal, key: &str) -> bool {
    v.field(key)
        .and_then(JVal::as_bool)
        .unwrap_or_else(|| panic!("environment.json: missing boolean field {key}"))
}

/// `#rrggbb` → sRGB 0..1. The JS side hands hex strings to THREE.Color;
/// conversion to linear space is a renderer concern (registered deviation).
fn hex_rgb(s: &str) -> [f32; 3] {
    let t = s.strip_prefix('#').unwrap_or(s);
    assert_eq!(t.len(), 6, "environment.json: expected #rrggbb, got {s}");
    let chan =
        |a: usize, b: usize| u8::from_str_radix(&t[a..b], 16).expect("hex colour") as f32 / 255.0;
    [chan(0, 2), chan(2, 4), chan(4, 6)]
}

fn rgb_field(v: &'static JVal, key: &str) -> [f32; 3] {
    hex_rgb(
        v.field(key)
            .and_then(JVal::as_str)
            .unwrap_or_else(|| panic!("environment.json: missing colour field {key}")),
    )
}

fn nums(v: &'static JVal, key: &str) -> Vec<f32> {
    v.field(key)
        .and_then(JVal::as_array)
        .unwrap_or_else(|| panic!("environment.json: missing array field {key}"))
        .iter()
        .map(|x| x.as_f64().expect("numeric array entry") as f32)
        .collect()
}

fn triple(v: &'static JVal, key: &str) -> [f32; 3] {
    let a = nums(v, key);
    assert_eq!(a.len(), 3, "expected a 3-vector for {key}");
    [a[0], a[1], a[2]]
}

// ------------------------------------------------------------------ grade

/// `GradeShader.uniforms` defaults + the per-theme overlay (renderer.js L202-215
/// writes the theme's grade over the default every frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    pub exposure: f32,
    pub sat: f32,
    pub vib: f32,
    pub contrast: f32,
    pub lift: f32,
    pub vignette: f32,
    pub shadow_tint: [f32; 3],
    pub high_tint: [f32; 3],
    pub hurt: f32,
    pub flash: f32,
    pub aspect: f32,
    pub bloom: [f32; 3],
}

impl Grade {
    /// Renderer defaults (renderer.js L17-32 + `BLOOM` L15).
    #[must_use]
    pub fn defaults() -> Self {
        let d = field(doc(), "gradeDefaults");
        let b = triple_from(field(doc(), "bloomDefault"));
        Self {
            exposure: f32_field(d, "uExposure"),
            sat: f32_field(d, "uSat"),
            vib: f32_field(d, "uVib"),
            contrast: f32_field(d, "uContrast"),
            lift: f32_field(d, "uLift"),
            vignette: f32_field(d, "uVignette"),
            shadow_tint: triple(d, "uShadowTint"),
            high_tint: triple(d, "uHighTint"),
            hurt: f32_field(d, "uHurt"),
            flash: f32_field(d, "uFlash"),
            aspect: f32_field(d, "uAspect"),
            bloom: [b[0], b[1], b[2]],
        }
    }

    /// Theme overlay: absent fields keep the renderer default (only `sunset`
    /// carries a `bloom` triple).
    #[must_use]
    fn overlay(base: Self, theme: &'static JVal) -> Self {
        Self {
            exposure: opt_f32(theme, "uExposure").unwrap_or(base.exposure),
            sat: opt_f32(theme, "uSat").unwrap_or(base.sat),
            vib: opt_f32(theme, "uVib").unwrap_or(base.vib),
            contrast: opt_f32(theme, "uContrast").unwrap_or(base.contrast),
            lift: opt_f32(theme, "uLift").unwrap_or(base.lift),
            vignette: opt_f32(theme, "uVignette").unwrap_or(base.vignette),
            shadow_tint: theme
                .field("uShadowTint")
                .map(triple_from)
                .unwrap_or(base.shadow_tint),
            high_tint: theme
                .field("uHighTint")
                .map(triple_from)
                .unwrap_or(base.high_tint),
            bloom: theme.field("bloom").map(triple_from).unwrap_or(base.bloom),
            ..base
        }
    }
}

/// `triple()` needs a field key; a raw array value uses this instead.
fn triple_from(v: &JVal) -> [f32; 3] {
    let a = v
        .as_array()
        .unwrap_or_else(|| panic!("environment.json: expected a numeric array"));
    assert_eq!(a.len(), 3, "expected a 3-vector");
    [
        a[0].as_f64().expect("number") as f32,
        a[1].as_f64().expect("number") as f32,
        a[2].as_f64().expect("number") as f32,
    ]
}

// ------------------------------------------------------------------ themes

/// One `THEMES` row, kept as the extracted JVal so every field (including
/// those only a GPU pass consumes) stays data-driven.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: &'static str,
    v: &'static JVal,
}

impl Theme {
    #[must_use]
    pub fn get(name: &str) -> Option<Self> {
        field(doc(), "themes").field(name).map(|v| Self {
            name: match name {
                "day" => "day",
                "sunset" => "sunset",
                "golden" => "golden",
                _ => "day",
            },
            v,
        })
    }

    #[must_use]
    pub fn num(&self, key: &str) -> f32 {
        f32_field(self.v, key)
    }

    #[must_use]
    pub fn opt_num(&self, key: &str) -> Option<f32> {
        opt_f32(self.v, key)
    }

    #[must_use]
    pub fn rgb(&self, key: &str) -> [f32; 3] {
        rgb_field(self.v, key)
    }

    #[must_use]
    pub fn array(&self, key: &str) -> Vec<f32> {
        nums(self.v, key)
    }

    #[must_use]
    pub fn fog(&self) -> [f32; 2] {
        let a = self.array("fog");
        [a[0], a[1]]
    }

    #[must_use]
    pub fn night(&self) -> f32 {
        self.num("night")
    }

    #[must_use]
    pub fn grade(&self) -> Grade {
        Grade::overlay(Grade::defaults(), field(self.v, "grade"))
    }
}

/// Theme names present in `THEMES` (order = extraction order).
#[must_use]
pub fn theme_names() -> Vec<&'static str> {
    match field(doc(), "themes") {
        JVal::Obj(entries) => entries.iter().map(|(k, _)| k.as_str()).collect(),
        _ => panic!("environment.json: themes must be an object"),
    }
}

/// `mapTheme(map, time)` evaluated by the extractor against the real JS
/// config (config.js L513): unknown maps/times fall back to `day`.
#[must_use]
pub fn map_theme(map: &str, time: &str) -> &'static str {
    field(doc(), "mapThemes")
        .field(map)
        .and_then(|row| row.field(time))
        .and_then(JVal::as_str)
        .unwrap_or("day")
}

// ------------------------------------------------------------------ quality

/// One `QUALITY` row (config.js L585-591). `ultra` is kept distinct from
/// `high` here — they differ only in `pixel_ratio`; `quality.rs::QualityTier`
/// still folds them for the three-tier internals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualityPreset {
    pub name: &'static str,
    pub pixel_ratio: f32,
    pub shadow_size: u32,
    pub msaa: u32,
    pub bloom: bool,
    pub ao: bool,
    pub paint_atlas: u32,
    pub particles: f32,
}

pub const TIER_NAMES: [&str; 4] = ["low", "medium", "high", "ultra"];

impl QualityPreset {
    #[must_use]
    pub fn get(name: &str) -> Option<Self> {
        let row = field(doc(), "quality").field(name)?;
        Some(Self {
            name: TIER_NAMES.iter().copied().find(|n| *n == name)?,
            pixel_ratio: f32_field(row, "pixelRatio"),
            shadow_size: f32_field(row, "shadowSize") as u32,
            msaa: f32_field(row, "msaa") as u32,
            bloom: bool_field(row, "bloom"),
            ao: bool_field(row, "ao"),
            paint_atlas: f32_field(row, "paintAtlas") as u32,
            particles: f32_field(row, "particles"),
        })
    }

    #[must_use]
    pub fn all() -> [Self; 4] {
        TIER_NAMES.map(|n| Self::get(n).expect("quality tier present"))
    }

    /// Low + no GPU: every optional post-process off (tasks.md L335 保底路径).
    #[must_use]
    pub fn post_processing_enabled(self) -> bool {
        self.bloom || self.ao || self.msaa > 0
    }
}

/// `DEFAULT_SETTINGS` boot values (config.js L562-582), data-driven.
#[must_use]
pub fn default_settings() -> (String, bool, bool) {
    let s = field(doc(), "defaultSettings");
    (
        s.field("quality")
            .and_then(JVal::as_str)
            .unwrap_or("high")
            .to_string(),
        bool_field(s, "shadows"),
        bool_field(s, "bloom"),
    )
}

// ------------------------------------------------------------------ batch B

use bevy::prelude::{Color, Resource};

/// Extracted sRGB hex → Bevy colour. The JS side feeds hex strings to
/// THREE.Color; no linear-space conversion is done here (D-T55-①).
#[must_use]
pub fn srgb_color(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Everything the render world needs for one (theme, quality tier) pair.
/// Pure data — no GPU handles — so every combination is headless-testable
/// (TR-55.1) and the values come only from `environment.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvParams {
    pub theme: &'static str,
    pub tier: &'static str,
    pub sun_color: [f32; 3],
    pub sun_az: f32,
    pub sun_el: f32,
    pub sun_intensity: f32,
    pub hemi_sky: [f32; 3],
    pub hemi_ground: [f32; 3],
    pub hemi_intensity: f32,
    pub zenith: [f32; 3],
    pub sky_mid: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    pub fog: [f32; 2],
    /// `horizon.lerp(skyMid, 0.15)` (environment.js L3039).
    pub fog_color: [f32; 3],
    pub sea_deep: [f32; 3],
    pub night: f32,
    pub shadow_size: u32,
    pub msaa: u32,
    pub pixel_ratio: f32,
    pub bloom: bool,
    pub ao: bool,
    pub post_enabled: bool,
    pub particles: f32,
    /// Task 54 D-T54-5⑪: quality tier → character rig LOD (档间几何直切).
    pub char_tier: &'static str,
    pub grade: Grade,
}

/// Resolve a `(theme, tier)` pair. Unknown names fall back to `day` / `high`
/// (≡ JS `QUALITY[settings.quality] || QUALITY.high`, main.js L117).
#[must_use]
pub fn env_params(theme_name: &str, tier_name: &str) -> EnvParams {
    let theme: &'static str = match theme_name {
        "sunset" => "sunset",
        "golden" => "golden",
        _ => "day",
    };
    let tier: &'static str = TIER_NAMES
        .iter()
        .copied()
        .find(|n| *n == tier_name)
        .unwrap_or("high");
    let t = Theme::get(theme).expect("theme present in environment.json");
    let q = QualityPreset::get(tier).expect("tier present in environment.json");
    let (_, def_shadows, def_bloom) = default_settings();
    let horizon = t.rgb("horizon");
    let sky_mid = t.rgb("skyMid");
    let bloom = q.bloom && def_bloom;
    let ao = q.ao;
    let _ = def_shadows; // the K-key runtime toggle owns the shadow flag
    EnvParams {
        theme,
        tier,
        sun_color: t.rgb("sunColor"),
        sun_az: t.num("sunAz"),
        sun_el: t.num("sunEl"),
        sun_intensity: t.num("sunIntensity"),
        hemi_sky: t.rgb("hemiSky"),
        hemi_ground: t.rgb("hemiGround"),
        hemi_intensity: t.num("hemiIntensity"),
        zenith: t.rgb("zenith"),
        sky_mid,
        horizon,
        ground: t.rgb("ground"),
        fog: t.fog(),
        fog_color: lerp3(horizon, sky_mid, 0.15),
        sea_deep: t.rgb("seaDeep"),
        night: t.night(),
        shadow_size: q.shadow_size,
        msaa: q.msaa,
        pixel_ratio: q.pixel_ratio,
        bloom,
        ao,
        // Low tier + all optional passes off = the SwiftShader 保底可玩路径.
        post_enabled: bloom || ao || q.msaa > 0,
        particles: q.particles,
        char_tier: match tier {
            "low" => "far",
            "medium" => "game",
            _ => "hero",
        },
        grade: t.grade(),
    }
}

/// Runtime (theme, tier, shadows) state. `K` toggles shadows, `L` cycles the
/// quality tier, `T` cycles the theme (the JS settings UI is M5 scope).
#[derive(Resource, Clone, Debug)]
pub struct EnvState {
    pub params: EnvParams,
    pub shadows: bool,
}

impl Default for EnvState {
    fn default() -> Self {
        Self::from_boot()
    }
}

impl EnvState {
    #[must_use]
    pub fn from_boot() -> Self {
        let raw_tier = std::env::var(crate::quality::ENV_QUALITY)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let tier: &'static str = match raw_tier.as_str() {
            "low" | "medium" | "high" | "ultra" => TIER_NAMES
                .iter()
                .copied()
                .find(|n| *n == raw_tier)
                .unwrap_or("high"),
            _ => crate::quality::tier_from_env().name(),
        };
        let raw_theme = std::env::var("INKWAVE_THEME")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let theme = match raw_theme.as_str() {
            "sunset" | "dusk" => "sunset",
            "golden" => "golden",
            _ => "day",
        };
        Self {
            params: env_params(theme, tier),
            shadows: default_settings().1,
        }
    }

    pub fn cycle_tier(&mut self) {
        let i = TIER_NAMES
            .iter()
            .position(|n| *n == self.params.tier)
            .unwrap_or(2);
        let next = TIER_NAMES[(i + 1) % TIER_NAMES.len()];
        self.params = env_params(self.params.theme, next);
    }

    pub fn cycle_theme(&mut self) {
        let order = ["day", "sunset", "golden"];
        let i = order
            .iter()
            .position(|n| *n == self.params.theme)
            .unwrap_or(0);
        let next = order[(i + 1) % order.len()];
        self.params = env_params(next, self.params.tier);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_are_extracted_and_anchor_key_values() {
        assert_eq!(theme_names().len(), 3);
        for n in ["day", "sunset", "golden"] {
            assert!(Theme::get(n).is_some(), "missing theme {n}");
        }
        let day = Theme::get("day").unwrap();
        let sunset = Theme::get("sunset").unwrap();
        let golden = Theme::get("golden").unwrap();
        assert_eq!(day.num("sunIntensity"), 3.3);
        assert_eq!(day.num("sunAz"), 222.0);
        assert_eq!(sunset.num("sunEl"), 15.0);
        assert_eq!(sunset.night(), 1.0);
        assert_eq!(golden.num("sunEl"), 28.0);
        assert_eq!(golden.opt_num("shafts"), Some(1.0));
        assert_eq!(day.fog(), [25.0, 900.0]);
        assert_eq!(sunset.fog(), [25.0, 800.0]);
        assert_eq!(golden.fog(), [30.0, 1000.0]);
        // '#fff0dc' → sRGB triple, no hand-written constants in the Rust tree.
        assert!((day.rgb("sunColor")[0] - 1.0).abs() < 1e-6);
        assert!((day.rgb("sunColor")[1] - 240.0 / 255.0).abs() < 1e-6);
        assert!((sunset.rgb("zenith")[2] - 0x68 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn grade_defaults_and_theme_overlay() {
        let d = Grade::defaults();
        assert_eq!(d.sat, 1.08);
        assert_eq!(d.vib, 0.12);
        assert_eq!(d.contrast, 1.07);
        assert_eq!(d.vignette, 0.22);
        assert_eq!(d.aspect, 1.7);
        assert_eq!(d.shadow_tint, [0.975, 0.99, 1.035]);
        assert_eq!(d.high_tint, [1.025, 1.0, 0.972]);
        assert_eq!(d.bloom, [0.28, 0.45, 2.4]);

        let day = Theme::get("day").unwrap().grade();
        assert_eq!(day.exposure, 0.9);
        assert_eq!(day.sat, 1.1);
        assert_eq!(
            day.bloom, d.bloom,
            "day inherits the renderer bloom default"
        );
        let dusk = Theme::get("sunset").unwrap().grade();
        assert_eq!(dusk.exposure, 1.0);
        assert_eq!(dusk.vignette, 0.28);
        assert_eq!(dusk.bloom, [0.4, 0.55, 1.7], "sunset overrides bloom");
        assert_eq!(dusk.aspect, d.aspect, "unset fields keep the default");
    }

    #[test]
    fn quality_presets_match_the_js_table() {
        let all = QualityPreset::all();
        assert_eq!(all[0].name, "low");
        assert_eq!(all[0].pixel_ratio, 0.75);
        assert_eq!(all[0].shadow_size, 1024);
        assert_eq!(all[0].msaa, 0);
        assert!(!all[0].bloom && !all[0].ao);
        assert!(!all[0].post_processing_enabled(), "Low = 保底全关路径");
        assert_eq!(all[1].particles, 0.7);
        assert!(all[1].bloom && !all[1].ao);
        assert_eq!(all[2].shadow_size, 4096);
        assert!(all[2].ao && all[2].bloom);
        assert_eq!(all[2].paint_atlas, 4096);
        assert_eq!(all[3].pixel_ratio, 2.0);
        assert_eq!(all[3].msaa, 4);
        assert!(QualityPreset::get("potato").is_none());
    }

    #[test]
    fn map_theme_uses_the_real_js_fallback_chain() {
        assert_eq!(map_theme("tidewater", "day"), "day");
        assert_eq!(map_theme("tidewater", "dusk"), "sunset");
        assert_eq!(map_theme("halyard", "day"), "golden");
        assert_eq!(map_theme("halyard", "dusk"), "sunset");
        assert_eq!(map_theme("crossmarket", "day"), "golden");
        assert_eq!(map_theme("cargo", "dusk"), "sunset");
        assert_eq!(map_theme("nonexistent", "dusk"), "day");
        assert_eq!(map_theme("tidewater", "night"), "day");
        // every resolved name must exist in THEMES
        for m in ["tidewater", "halyard", "cargo"] {
            for t in ["day", "dusk"] {
                assert!(Theme::get(map_theme(m, t)).is_some());
            }
        }
    }

    #[test]
    fn default_settings_are_data_driven() {
        assert_eq!(default_settings(), ("high".to_string(), true, true));
    }

    /// TR-55.1: every tier × theme combination constructs without panic and
    /// yields finite numbers (the headless/low-GPU 保底 path).
    #[test]
    fn all_tier_theme_combinations_are_finite() {
        for q in QualityPreset::all() {
            for n in theme_names() {
                let th = Theme::get(n).expect("theme");
                let g = th.grade();
                let vals = [
                    th.num("sunAz"),
                    th.num("sunEl"),
                    th.num("sunIntensity"),
                    th.night(),
                    th.fog()[0],
                    th.fog()[1],
                    g.exposure,
                    g.sat,
                    g.bloom[0],
                    q.pixel_ratio,
                    q.particles,
                ];
                assert!(
                    vals.iter().all(|v| v.is_finite()),
                    "{} × {} produced a non-finite value",
                    q.name,
                    n
                );
                assert!(q.shadow_size > 0 && q.paint_atlas > 0);
            }
        }
    }

    /// TR-55.1 (batch B): the resolved render parameters for the two extreme
    /// (theme, tier) pairs — day/high and sunset/low.
    #[test]
    fn env_params_anchor_day_high_and_low_sunset() {
        let day = env_params("day", "high");
        assert_eq!(day.theme, "day");
        assert_eq!(day.tier, "high");
        assert_eq!(day.sun_az, 222.0);
        assert_eq!(day.sun_el, 39.0);
        assert_eq!(day.sun_intensity, 3.3);
        assert_eq!(day.fog, [25.0, 900.0]);
        assert_eq!(day.shadow_size, 4096);
        assert_eq!(day.msaa, 4);
        assert!(day.bloom && day.ao && day.post_enabled);
        assert_eq!(day.char_tier, "hero");
        assert_eq!(day.grade.exposure, 0.9);
        assert!((day.sun_color[0] - 1.0).abs() < 1e-6);

        let dusk = env_params("sunset", "low");
        assert_eq!(dusk.theme, "sunset");
        assert_eq!(dusk.night, 1.0);
        assert_eq!(dusk.fog, [25.0, 800.0]);
        assert_eq!(dusk.shadow_size, 1024);
        assert_eq!(dusk.msaa, 0);
        assert!(
            !dusk.bloom && !dusk.ao && !dusk.post_enabled,
            "Low = 全关保底"
        );
        assert_eq!(dusk.char_tier, "far");
        assert_eq!(dusk.grade.bloom, [0.4, 0.55, 1.7]);
        assert_eq!(dusk.grade.vignette, 0.28);
    }

    /// `setTheme()` L3039: fog colour = horizon.lerp(skyMid, 0.15).
    #[test]
    fn fog_color_is_the_js_horizon_sky_mid_lerp() {
        let p = env_params("day", "medium");
        assert_eq!(p.fog_color, lerp3(p.horizon, p.sky_mid, 0.15));
        let expected = 212.0 / 255.0 + (90.0 / 255.0 - 212.0 / 255.0) * 0.15;
        assert!((p.fog_color[0] - expected).abs() < 1e-6);
    }

    /// Every tier × theme pair resolves to finite, coherent render parameters
    /// (the headless / wasm 不 panic 保底, TR-55.1).
    #[test]
    fn all_env_param_combinations_are_finite_and_coherent() {
        for tier in TIER_NAMES {
            for theme in theme_names() {
                let p = env_params(theme, tier);
                assert_eq!(p.theme, theme);
                assert_eq!(p.tier, tier);
                assert!(matches!(p.char_tier, "far" | "game" | "hero"));
                assert_eq!(p.post_enabled, p.bloom || p.ao || p.msaa > 0);
                assert_eq!(p.bloom, QualityPreset::get(tier).unwrap().bloom);
                assert_eq!(p.shadow_size, QualityPreset::get(tier).unwrap().shadow_size);
                assert!(p.shadow_size > 0 && p.pixel_ratio > 0.0);
                let finite = [
                    p.sun_az,
                    p.sun_el,
                    p.sun_intensity,
                    p.hemi_intensity,
                    p.fog[0],
                    p.fog[1],
                    p.fog_color[0],
                    p.sea_deep[0],
                    p.night,
                    p.particles,
                    p.grade.exposure,
                    p.grade.bloom[0],
                ];
                assert!(finite.iter().all(|v| v.is_finite()), "{tier} × {theme}");
            }
        }
    }

    #[test]
    fn env_state_cycles_tier_and_theme() {
        let mut s = EnvState {
            params: env_params("day", "high"),
            shadows: true,
        };
        s.cycle_tier();
        assert_eq!(s.params.tier, "ultra");
        assert_eq!(s.params.char_tier, "hero");
        s.cycle_tier();
        assert_eq!(s.params.tier, "low");
        assert_eq!(s.params.char_tier, "far");
        s.cycle_theme();
        assert_eq!(s.params.theme, "sunset");
        assert_eq!(s.params.fog, [25.0, 800.0]);
        s.cycle_theme();
        assert_eq!(s.params.theme, "golden");
        s.cycle_theme();
        assert_eq!(s.params.theme, "day");
        s.shadows = false;
        assert!(!s.shadows);
    }
}
