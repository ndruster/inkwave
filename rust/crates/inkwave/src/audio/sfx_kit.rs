//! M5 Task 60 — kit-weapon SFX cue table, extracted from the live upstream
//! registry by `rust/tools/extract/extract-kit-sfx.mjs` (`kit-sfx.json`, 53
//! names, `source.commit` pinned). The 148 base cues stay in `sfx_table.rs`;
//! this module only adds the kit entries and the event→cue rules.
//!
//! `SfxGate` is reused unchanged: `sfx_table::meta` falls back to
//! `kit_meta`, so gate.play(name, …) applies each kit cue's extracted gain /
//! max / jitter / minGap exactly like the JS engine.
//!
//! Functions (`build`/`loop`) are code, not data: synthesiser definitions
//! stay in `sfx_table.rs`; this file only carries the extracted metadata.

use super::sfx_table::SfxMeta;
use inkwave_sim::net::wire::{JVal, parse_json};
use std::sync::OnceLock;

const KIT_JSON: &str = include_str!("../../../../assets/audio/kit-sfx.json");

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KitCue {
    pub name: &'static str,
    pub gain: f64,
    pub max: usize,
    pub jitter: f64,
    pub min_gap: f64,
    pub reverb: f64,
}

fn cues() -> &'static [KitCue] {
    static CUES: OnceLock<Vec<KitCue>> = OnceLock::new();
    CUES.get_or_init(|| {
        let root = parse_json(KIT_JSON).expect("kit-sfx.json parses");
        let list = root
            .field("cues")
            .and_then(JVal::as_array)
            .expect("kit-sfx.json cues array");
        list.iter()
            .map(|c| {
                let num = |k: &str| -> f64 { c.field(k).and_then(JVal::as_f64).unwrap_or(0.0) };
                KitCue {
                    // `JVal::as_str` borrows the parse-owned string; leak the
                    // tiny name into the 'static table (bounded: 53 names).
                    name: Box::leak(
                        c.field("name")
                            .and_then(JVal::as_str)
                            .expect("kit cue name")
                            .to_string()
                            .into_boxed_str(),
                    ),
                    gain: num("gain"),
                    max: num("max") as usize,
                    jitter: num("jitter"),
                    min_gap: num("minGap"),
                    reverb: num("reverb"),
                }
            })
            .collect()
    })
}

pub fn kit_names() -> impl Iterator<Item = &'static str> {
    cues().iter().map(|c| c.name)
}

#[must_use]
pub fn kit_cue(name: &str) -> Option<&'static KitCue> {
    cues().iter().find(|c| c.name == name)
}

/// `SfxGate` uses this when a cue is not in the base 148 table — no change to
/// the gate itself (TR-60 "reuse SfxGate").
#[must_use]
pub fn kit_meta(name: &str) -> Option<SfxMeta> {
    kit_cue(name).map(|c| SfxMeta {
        gain: c.gain,
        max: c.max.max(1),
        jitter: c.jitter,
        min_gap: c.min_gap,
        one_shot: None,
        is_loop: false,
    })
}

/// ≡ `main.js` L543-546 footstep cue selection by SURFACE (not speed):
/// `surface === 1 ? step_ink : surface === 2 ? step_enemy : step_dry`.
/// These three names exist in the base 148-cue table (they really bake).
/// `FootstepEvent` has no surface field (M4 data model), so callers pass 0
/// (= dry) — registered as D-T60-2.
#[must_use]
pub fn footstep_cue(surface: u8) -> &'static str {
    match surface {
        1 => "step_ink",
        2 => "step_enemy",
        _ => "step_dry",
    }
}

/// ≡ `main.js` L544: local 0.7 / remote 0.45 × `min(1, 0.45 + speed / 8)`.
#[must_use]
pub fn footstep_volume(local: bool, speed: f32) -> f64 {
    let base = if local { 0.7 } else { 0.45 };
    base * (0.45 + f64::from(speed.max(0.0)) / 8.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracted_table_has_all_53_kit_cues() {
        let names: Vec<_> = kit_names().collect();
        assert_eq!(names.len(), 53);
        assert_eq!(
            names
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            53
        );
        assert!(names.contains(&"bow_draw"));
        assert!(names.contains(&"brolly_open"));
        assert!(names.contains(&"blade_charge"));
    }

    #[test]
    fn kit_meta_feeds_the_gate_with_the_extracted_values() {
        let c = kit_cue("blade_charge").expect("blade_charge extracted");
        let m = kit_meta("blade_charge").expect("kit meta");
        assert_eq!(m.gain, c.gain);
        assert_eq!(m.max, 2);
        assert!((m.jitter - 0.0).abs() < 1e-9);
        assert!(m.min_gap >= 0.0);
        assert_eq!(kit_meta("shot"), None, "base cues stay in sfx_table.rs");
    }

    #[test]
    fn gate_accepts_a_kit_cue_without_new_code() {
        use super::super::sfx_table::SfxGate;
        let mut gate = SfxGate::new(0);
        let d = gate.play("bow_draw", 0.0, None, 0.0, 1.0, 1.0, 0.5);
        assert!(d.is_some(), "kit cue plays through the unchanged gate");
        assert_eq!(gate.played, 1);
        let c = kit_cue("bow_draw").unwrap();
        assert!(c.gain > 0.0 && c.max > 0);
    }

    #[test]
    fn footstep_cue_matches_upstream_surface_rule() {
        assert_eq!(footstep_cue(0), "step_dry");
        assert_eq!(footstep_cue(1), "step_ink");
        assert_eq!(footstep_cue(2), "step_enemy");
        // All three cues are in the real base table (they can really bake).
        for name in ["step_dry", "step_ink", "step_enemy"] {
            assert!(
                super::super::sfx_table::bake(name).is_some(),
                "{name} bakes"
            );
        }
        assert_eq!(footstep_volume(true, 4.0), 0.7 * 0.95);
        assert_eq!(footstep_volume(false, 100.0), 0.45);
    }
}
