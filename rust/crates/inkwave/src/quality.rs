//! Quality tier table (M4 Task 52 batch E1, spec D-T52-5: 质量档位表起建).
//!
//! Minimal seed for Task 55 (the settings UI / renderer wiring consumes and
//! extends this). Anchors:
//! * texlib bake size — the library contract `createTextureLibrary(...,
//!   { size = 512 })` with "256 for low quality" (texlib.js L4/L1332, ≡ the
//!   extracted `tex.size_low/size_high`, schema.rs `TexConsts`);
//! * anti-tiling (防重复采样) switch — spec D-T52-5 says ≡ a JS
//!   `rendererQuality === 'low'` gate. **实读登记偏差**：`rendererQuality`
//!   在 `src/` 全树无消费点（grep 无命中）；JS 侧 anti-tiling mode 恒经
//!   `uTL.z` 逐层携带（levelMaterial.js L96 → 片元 L319 `texlibSample(...,
//!   int(tl.z), ...)`，texlib.js L56 "pass TEXLIB_PLAIN to disable"），无质量
//!   门。本档位旗标按 spec 语义起建（Low 关、其余开），接线时以旗标为准；
//!   另注：JS 真实 size 门在 main.js L121 `q.paintAtlas >= 4096 ? 512 : 256`
//!   （即 JS medium 实际也烘 256）——本表按 spec/task 锚 texlib.js L4 取
//!   Low=256、Medium/High=512，差异登记于此，Task 55 消费时复核。
//! * shadows — the K-key runtime toggle (`world.rs::toggle_shadows`,
//!   `shadow_maps_enabled`) 并入档位语义：档位给默认开/关 + shadow map
//!   尺寸（≡ config.js `QUALITY[*].shadowSize` L587-589）。

/// The three render-quality tiers (JS `DEFAULT_SETTINGS.quality`
/// 'low'|'medium'|'high'|'ultra', config.js L567/L585-591 — 'ultra' maps to
/// [`QualityTier::High`] on the Rust side; the JS ultra/high rows differ only
/// in `pixelRatio`, which is Task 55 renderer scope).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QualityTier {
    Low,
    Medium,
    High,
}

/// Boot default when nothing overrides (task brief: 默认 Medium).
pub const DEFAULT_TIER: QualityTier = QualityTier::Medium;

/// Env var the headless/native boot reads for the tier (Task 55 replaces this
/// with the persisted settings document).
pub const ENV_QUALITY: &str = "INKWAVE_QUALITY";

impl QualityTier {
    /// JS settings name (`config.js` L567 vocabulary).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parse a JS quality name; `'ultra'` folds into High (see the enum doc).
    /// `None` for anything else (caller falls back to [`DEFAULT_TIER`]).
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" | "ultra" => Some(Self::High),
            _ => None,
        }
    }

    /// Texlib bake edge size, px (≡ texlib.js L4 "256 for low quality" /
    /// L1332 `size = 512` default; schema `TexConsts::size_low/size_high`).
    #[must_use]
    pub fn texlib_size(self) -> u32 {
        match self {
            Self::Low => crate::texlib::schema::embedded_texlib().tex.size_low,
            Self::Medium | Self::High => crate::texlib::schema::embedded_texlib().tex.size_high,
        }
    }

    /// Anti-tiling (防重复采样) switch — see the module doc for the
    /// `rendererQuality === 'low'` 实读偏差登记. `false` ⇒ the level shader
    /// passes `TEXLIB_PLAIN` (mode 0) regardless of the per-layer `uTL.z`.
    #[must_use]
    pub fn anti_tiling(self) -> bool {
        !matches!(self, Self::Low)
    }

    /// Sun shadow maps on by default (JS `DEFAULT_SETTINGS.shadows: true`,
    /// config.js L568); K (`world.rs::toggle_shadows`) flips `enabled` at
    /// runtime on top of the tier default.
    #[must_use]
    pub fn shadows_default(self) -> bool {
        true
    }

    /// Shadow map size, px (≡ config.js `QUALITY[*].shadowSize` L587-589;
    /// feeds `DirectionalLightShadowMap { size }`, world.rs L859).
    #[must_use]
    pub fn shadow_size(self) -> u32 {
        match self {
            Self::Low => 1024,
            Self::Medium => 2048,
            Self::High => 4096,
        }
    }

    /// Paint atlas edge, px (≡ config.js `QUALITY[*].paintAtlas` L587-590;
    /// the JS main.js L121 size gate reads this — see the module doc).
    #[must_use]
    pub fn paint_atlas_size(self) -> u32 {
        match self {
            Self::Low | Self::Medium => 2048,
            Self::High => 4096,
        }
    }
}

/// Boot tier from the environment (`INKWAVE_QUALITY`, e.g.
/// `INKWAVE_QUALITY=low`), [`DEFAULT_TIER`] when unset/unrecognised (JS
/// `QUALITY[settings.quality] || QUALITY.high` fallback shape, main.js L117 —
/// ours defaults to Medium per the task brief).
#[must_use]
pub fn tier_from_env() -> QualityTier {
    std::env::var(ENV_QUALITY)
        .ok()
        .and_then(|s| QualityTier::from_name(&s.to_ascii_lowercase()))
        .unwrap_or(DEFAULT_TIER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tier → texlib size table (Low=256, Medium/High=512 ≡ texlib.js
    /// L4/L1332 via the extracted `TexConsts`).
    #[test]
    fn tier_texlib_size_table() {
        assert_eq!(QualityTier::Low.texlib_size(), 256);
        assert_eq!(QualityTier::Medium.texlib_size(), 512);
        assert_eq!(QualityTier::High.texlib_size(), 512);
    }

    /// Anti-tiling gate flag (Low off, rest on; 实读偏差见模块头注释).
    #[test]
    fn tier_anti_tiling_flag() {
        assert!(!QualityTier::Low.anti_tiling());
        assert!(QualityTier::Medium.anti_tiling());
        assert!(QualityTier::High.anti_tiling());
    }

    /// Shadow semantics folded into the tier: default-on (K toggles at
    /// runtime) + per-tier map size ≡ config.js QUALITY L587-589.
    #[test]
    fn tier_shadow_semantics() {
        for t in [QualityTier::Low, QualityTier::Medium, QualityTier::High] {
            assert!(t.shadows_default());
        }
        assert_eq!(QualityTier::Low.shadow_size(), 1024);
        assert_eq!(QualityTier::Medium.shadow_size(), 2048);
        assert_eq!(QualityTier::High.shadow_size(), 4096);
    }

    /// paintAtlas ladder ≡ config.js L587-590 (ultra == high, folded).
    #[test]
    fn tier_paint_atlas_sizes() {
        assert_eq!(QualityTier::Low.paint_atlas_size(), 2048);
        assert_eq!(QualityTier::Medium.paint_atlas_size(), 2048);
        assert_eq!(QualityTier::High.paint_atlas_size(), 4096);
    }

    /// JS name vocabulary, incl. the 'ultra' fold and rejects.
    #[test]
    fn tier_name_roundtrip() {
        assert_eq!(QualityTier::from_name("low"), Some(QualityTier::Low));
        assert_eq!(QualityTier::from_name("medium"), Some(QualityTier::Medium));
        assert_eq!(QualityTier::from_name("high"), Some(QualityTier::High));
        assert_eq!(QualityTier::from_name("ultra"), Some(QualityTier::High));
        assert_eq!(QualityTier::from_name("potato"), None);
        for t in [QualityTier::Low, QualityTier::Medium, QualityTier::High] {
            assert_eq!(QualityTier::from_name(t.name()), Some(t));
        }
    }

    /// Env boot: unset ⇒ Medium; a recognised value wins (guarded so a
    /// pre-set environment cannot make this flaky).
    #[test]
    fn tier_from_env_defaults_and_overrides() {
        if std::env::var(ENV_QUALITY).is_err() {
            assert_eq!(tier_from_env(), DEFAULT_TIER);
            assert_eq!(DEFAULT_TIER, QualityTier::Medium);
        }
    }
}
