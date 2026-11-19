//! M5 Task 65 — the post-processing chain DATA PLANE, ported from
//! `src/core/renderer.js`, `src/config.js` and `styles/hud.css`.
//!
//! All parameters come from the pinned `assets/postprocess.json`, independently
//! re-parsed by `tools/postprocess-crosscheck.mjs`. This module owns:
//!   * the 12-uniform GradeShader chain (incl. `uHurtColor`, D-T65-2),
//!   * the four-tier pass on/off derivation (low = all off保底),
//!   * GTAO + denoise constants, SMAA gate, tilt-shift CSS data,
//!   * the pass order `render_hdr → gtao → bloom → grade → screenfx → output → smaa`.
//!
//! D-T65-1: tilt-shift is a DOM/CSS overlay upstream, not a GPU pass — we hold
//! its data only. D-T65-3: GTAO/SMAA/grade GPU pass bodies cannot be validated in
//! the sandbox; only parameter/switch data is tested, visuals are a user-machine
//! REVIEW_CHECKLIST item. Bevy has built-in Bloom (already wired); GTAO/SMAA
//! would need custom fullscreen passes and are intentionally not faked here.

use std::sync::OnceLock;

use inkwave_sim::net::wire::{JVal, parse_json};

const DOC: &str = include_str!("../../../../assets/postprocess.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("postprocess.json parses"))
}

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("postprocess.json: missing field {key}"))
}

fn num(v: &JVal, key: &str) -> f64 {
    field(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("postprocess.json: missing numeric field {key}"))
}

fn array<'a>(v: &'a JVal, key: &str) -> &'a [JVal] {
    field(v, key)
        .as_array()
        .unwrap_or_else(|| panic!("postprocess.json: missing array field {key}"))
}

fn triple(v: &JVal, key: &str) -> [f64; 3] {
    let a = array(v, key);
    [
        a[0].as_f64().unwrap(),
        a[1].as_f64().unwrap(),
        a[2].as_f64().unwrap(),
    ]
}

/// GradeShader's 12 uniforms (renderer.js L17-32), all extracted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeUniforms {
    pub sat: f64,
    pub vib: f64,
    pub contrast: f64,
    pub shadow_tint: [f64; 3],
    pub high_tint: [f64; 3],
    pub lift: f64,
    pub exposure: f64,
    pub vignette: f64,
    pub hurt: f64,
    pub hurt_color: [f64; 3],
    pub flash: f64,
    pub aspect: f64,
}

pub fn grade_defaults() -> GradeUniforms {
    let g = field(doc(), "grade_defaults");
    GradeUniforms {
        sat: num(g, "sat"),
        vib: num(g, "vib"),
        contrast: num(g, "contrast"),
        shadow_tint: triple(g, "shadow_tint"),
        high_tint: triple(g, "high_tint"),
        lift: num(g, "lift"),
        exposure: num(g, "exposure"),
        vignette: num(g, "vignette"),
        hurt: num(g, "hurt"),
        hurt_color: triple(g, "hurt_color"),
        flash: num(g, "flash"),
        aspect: num(g, "aspect"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GtaoSpec {
    pub blend_intensity: f64,
    pub radius: f64,
    pub distance_exponent: f64,
    pub thickness: f64,
    pub scale: f64,
    pub samples: usize,
    pub distance_falloff: f64,
    pub half_res_apple_gpu: bool,
    pub denoise_luma_phi: f64,
    pub denoise_depth_phi: f64,
    pub denoise_normal_phi: f64,
    pub denoise_radius: f64,
    pub denoise_rings: usize,
    pub denoise_samples: usize,
}

pub fn gtao() -> GtaoSpec {
    let a = field(doc(), "gtao");
    let d = field(a, "denoise");
    GtaoSpec {
        blend_intensity: num(a, "blend_intensity"),
        radius: num(a, "radius"),
        distance_exponent: num(a, "distance_exponent"),
        thickness: num(a, "thickness"),
        scale: num(a, "scale"),
        samples: num(a, "samples") as usize,
        distance_falloff: num(a, "distance_falloff"),
        half_res_apple_gpu: field(a, "half_res_apple_gpu").as_bool().unwrap_or(false),
        denoise_luma_phi: num(d, "luma_phi"),
        denoise_depth_phi: num(d, "depth_phi"),
        denoise_normal_phi: num(d, "normal_phi"),
        denoise_radius: num(d, "radius"),
        denoise_rings: num(d, "rings") as usize,
        denoise_samples: num(d, "samples") as usize,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TiltShiftSpec {
    pub blur_px: f64,
    pub saturate: f64,
    pub top_height_pct: f64,
    pub bottom_height_pct: f64,
}

pub fn tilt_shift() -> TiltShiftSpec {
    let t = field(doc(), "tilt_shift");
    TiltShiftSpec {
        blur_px: num(t, "blur_px"),
        saturate: num(t, "saturate"),
        top_height_pct: num(t, "top_height_pct"),
        bottom_height_pct: num(t, "bottom_height_pct"),
    }
}

/// Per-tier post-processing pass switches derived from `QUALITY` (config.js
/// L585-591): `gtao ⇔ ao`, `smaa ⇔ msaa>0` (SMAA stands in where MSAA skipped),
/// `grade ⇔ post_enabled` (the low-tier保底 gate), `tilt_shift ⇔ diorama DOM`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassCombo {
    pub msaa: usize,
    pub bloom: bool,
    pub gtao: bool,
    pub smaa: bool,
}

pub fn pass_combo(tier: &str) -> PassCombo {
    let q = field(field(doc(), "quality"), tier);
    let msaa = num(q, "msaa") as usize;
    let bloom = field(q, "bloom").as_bool().unwrap_or(false);
    let ao = field(q, "ao").as_bool().unwrap_or(false);
    PassCombo {
        msaa,
        bloom,
        gtao: ao,
        // JS: SMAA only stands in when MSAA was skipped (Apple GPU / msaa=0
        // with the settings still requesting edges). Data-plane derivation:
        // medium/high/ultra request msaa>0; the actual SMAA fallback is a
        // GPU-only decision registered in D-T65-3.
        smaa: msaa > 0,
    }
}

/// Apply the low-tier保底: `post_enabled=false` forces every pass off,
/// mirroring `EnvParams.post_enabled` and the screenfx composite gate.
#[must_use]
pub fn effective_combo(tier: &str, post_enabled: bool) -> PassCombo {
    if !post_enabled {
        return PassCombo {
            msaa: 0,
            bloom: false,
            gtao: false,
            smaa: false,
        };
    }
    pass_combo(tier)
}

pub fn pass_order() -> &'static [String] {
    static VAL: OnceLock<Vec<String>> = OnceLock::new();
    VAL.get_or_init(|| {
        array(doc(), "pass_order")
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect()
    })
}

pub fn source_commit() -> &'static str {
    static VAL: OnceLock<String> = OnceLock::new();
    VAL.get_or_init(|| {
        field(field(doc(), "source"), "commit")
            .as_str()
            .unwrap_or("")
            .to_string()
    })
    .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task65_grade_chain_is_12_uniforms_and_pinned() {
        let g = grade_defaults();
        assert!((g.sat - 1.08).abs() < 1e-9);
        assert!((g.vib - 0.12).abs() < 1e-9);
        assert!((g.contrast - 1.07).abs() < 1e-9);
        assert_eq!(g.shadow_tint, [0.975, 0.99, 1.035]);
        assert_eq!(g.high_tint, [1.025, 1.0, 0.972]);
        assert_eq!(g.lift, 0.0);
        assert_eq!(g.exposure, 1.0);
        assert!((g.vignette - 0.22).abs() < 1e-9);
        assert_eq!(g.hurt, 0.0);
        // D-T65-2: the 12th uniform previously missing from environment.rs.
        assert_eq!(g.hurt_color, [1.0, 0.2, 0.3]);
        assert_eq!(g.flash, 0.0);
        assert!((g.aspect - 1.7).abs() < 1e-9);
        // Cross-check the Grade struct reads the same hurt_color from
        // environment.json (same upstream uniform).
        let env_grade = crate::environment::Grade::defaults();
        for i in 0..3 {
            assert!((env_grade.hurt_color[i] as f64 - g.hurt_color[i]).abs() < 1e-6);
        }
        assert_eq!(source_commit().len(), 40);
    }

    #[test]
    fn task65_quality_tier_switches_and_low_all_off() {
        assert_eq!(
            pass_combo("low"),
            PassCombo {
                msaa: 0,
                bloom: false,
                gtao: false,
                smaa: false
            }
        );
        assert_eq!(
            pass_combo("medium"),
            PassCombo {
                msaa: 2,
                bloom: true,
                gtao: false,
                smaa: true
            }
        );
        assert_eq!(
            pass_combo("high"),
            PassCombo {
                msaa: 4,
                bloom: true,
                gtao: true,
                smaa: true
            }
        );
        assert_eq!(pass_combo("ultra"), pass_combo("high"));
        // post_enabled=false forces all off at EVERY tier (the low保底 path).
        for tier in ["low", "medium", "high", "ultra"] {
            let c = effective_combo(tier, false);
            assert_eq!(
                c.msaa + usize::from(c.bloom) + usize::from(c.gtao) + usize::from(c.smaa),
                0,
                "{tier} with post off must have all passes off"
            );
        }
        assert_eq!(effective_combo("low", true), pass_combo("low"));
    }

    #[test]
    fn task65_gtao_smaa_tilt_params_are_pinned() {
        let a = gtao();
        assert!((a.blend_intensity - 0.8).abs() < 1e-9);
        assert!((a.radius - 1.1).abs() < 1e-9);
        assert!((a.distance_exponent - 1.6).abs() < 1e-9);
        assert_eq!(a.thickness, 1.0);
        assert_eq!(a.scale, 1.5);
        assert_eq!(a.samples, 12);
        assert_eq!(a.distance_falloff, 1.0);
        assert!(a.half_res_apple_gpu);
        assert_eq!(
            (
                a.denoise_luma_phi,
                a.denoise_depth_phi,
                a.denoise_normal_phi
            ),
            (10.0, 2.0, 3.0)
        );
        assert_eq!(
            (a.denoise_radius, a.denoise_rings, a.denoise_samples),
            (6.0, 2, 16)
        );
        let t = tilt_shift();
        assert_eq!(
            (t.blur_px, t.saturate, t.top_height_pct, t.bottom_height_pct),
            (6.0, 1.12, 28.0, 22.0)
        );
    }

    #[test]
    fn task65_pass_order_matches_renderer_composer() {
        assert_eq!(
            pass_order().join(","),
            "render_hdr,gtao,bloom,grade_vignette,screenfx,output,smaa"
        );
        // grade precedes screenfx (Task 64 composite), both before output.
        let grade = pass_order()
            .iter()
            .position(|s| s == "grade_vignette")
            .unwrap();
        let fx = pass_order().iter().position(|s| s == "screenfx").unwrap();
        let out = pass_order().iter().position(|s| s == "output").unwrap();
        assert!(grade < fx && fx < out, "grade → screenfx → output ordering");
    }
}
