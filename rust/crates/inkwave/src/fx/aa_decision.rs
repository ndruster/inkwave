//! M6 Task 72 / G-4 — the written SMAA/GTAO 选型落地裁决 (decision), as a
//! headless-anchorable pure-data module. No camera component is wired: this is
//! the「继续登记」branch of G-4, with the exact justification recorded here and
//! in REVIEW_CHECKLIST (D-M6-10).
//!
//! Read at implementation time (2026-11-21) from Bevy 0.19.1's vendored source:
//! - **SMAA built-in EXISTS**: `bevy_anti_alias::smaa::{Smaa, SmaaPlugin}`
//!   (`bevy_anti_alias-0.19.1/src/smaa/mod.rs` L80-99), registered by the
//!   default `AntiAliasPlugin` (`src/lib.rs` L27-31), with embedded
//!   `SMAAAreaLUT.ktx2` / `SMAASearchLUT.ktx2` behind the `smaa_luts` feature
//!   (L291-315). Without that feature Bevy uses a `lut_placeholder`.
//! - **GTAO has NO built-in**: registry search finds only `bevy_pbr::ssao`
//!   (`ssao.wgsl` / `spatial_denoise.wgsl`), not GTAO.
//! - **Our camera has no `Msaa` component** (`world.rs` search: zero hits) and
//!   this port never builds a depth/normal prepass (zero `DepthPrepass`/
//!   `NormalPrepass` hits); `postprocess.json smaa.gate = "msaa>0 && !multisampled"`.
//!
//! Decision:
//! - GTAO → continue「登记用户机复核」(needs custom pass + depth/normal prepass,
//!   no Bevy built-in, visuals untestable in sandbox).
//! - SMAA → continue「登记用户机复核」rather than wire the `Smaa` component:
//!   adding a second built-in fullscreen effect into the camera pipeline that
//!   ALSO carries Bloom + our Task 71 custom post-pass is an unvalidated GPU
//!   change (sandbox has no GPU); and Bevy's default `lut_placeholder` differs
//!   from JS upstream's real area/search textures, so visual parity cannot be
//!   claimed. The decision GATE itself (tier × post_enabled × multisampled) is
//!   pure data and fully anchored here — wiring flips to「接线」only after the
//!   user-machine review signs off (D-M6-10).

use crate::fx::post_process::PassCombo;

/// Result of the G-4 decision for one tier/frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AaDecision {
    /// Whether Bevy's built-in `Smaa` *would* be enabled by the extracted gate.
    pub smaa_gate: bool,
    /// Whether SMAA is actually wired (false in M6 — user-machine review item).
    pub smaa_wired: bool,
    /// Whether GTAO is wired (false — no built-in, prepass missing).
    pub gtao_wired: bool,
}

/// Evaluate the extracted `smaa.gate = "msaa>0 && !multisampled"` against a
/// resolved [`PassCombo`]. `multisampled` is the camera's actual MSAA state;
/// this port's camera is never multisampled, so callers pass `false`.
#[must_use]
pub fn decide(combo: &PassCombo, multisampled: bool) -> AaDecision {
    AaDecision {
        // `combo.smaa` is derived as `msaa > 0` (post_process.rs L170), so the
        // third term is redundant with the first; both are kept to make the
        // extracted gate "msaa>0 && !multisampled" explicit (review P2-2).
        smaa_gate: combo.msaa > 0 && !multisampled && combo.smaa,
        smaa_wired: false, // D-M6-10: user-machine review before wiring
        gtao_wired: false, // no Bevy built-in + missing depth/normal prepass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::post_process::effective_combo;

    #[test]
    fn task72_aa_decision_low_is_all_off() {
        let d = decide(&effective_combo("low", true), false);
        assert!(!d.smaa_gate, "low has msaa=0 → no SMAA gate");
        assert!(!d.smaa_wired && !d.gtao_wired, "M6 wires neither built-in");
    }

    #[test]
    fn task72_aa_decision_high_gate_but_not_wired() {
        let d = decide(&effective_combo("high", true), false);
        assert!(d.smaa_gate, "high msaa=4, not multisampled → gate true");
        assert!(
            !d.smaa_wired,
            "decision is 继续登记 until user-machine review"
        );
    }

    #[test]
    fn task72_aa_decision_multisampled_disables_smaa() {
        let d = decide(&effective_combo("ultra", true), true);
        assert!(!d.smaa_gate, "!multisampled term of the extracted gate");
    }

    #[test]
    fn task72_aa_decision_post_off_forces_all_off() {
        let d = decide(&effective_combo("ultra", false), false);
        assert!(!d.smaa_gate, "post_enabled=false 保底");
    }
}
