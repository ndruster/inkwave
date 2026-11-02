//! `inkwave_sim` — headless gameplay simulation for the INKWAVE Rust port.
//!
//! This crate must stay free of any renderer/window dependency (Bevy, wgpu,
//! winit): the Bevy app in the `inkwave` crate drives it, unit tests run it
//! natively without a GPU, and the future PROTO-v1 network layer (see
//! `rust/PORT_MAP.md`, module `net`) will serialize its state directly.
//!
//! Data flow (Task 3): node ESM extractors under `rust/tools/extract` read the
//! upstream JS tree and emit deterministic JSON into `rust/assets/`; the JSON
//! is embedded here at compile time, so deleting an artifact fails the build
//! (TR-3.1) and the Rust constants can never silently drift from the pipeline.

pub mod actor;
pub mod autopilot;
pub mod bot;
pub mod collision;
pub mod geometry;
// `match` is a Rust keyword; the module keeps the PORT_MAP name via a trailing
// underscore (`inkwave_sim::match_`).
pub mod ink_atlas;
pub mod match_;
pub mod nav;
// Task 15 — PROTO v1 boundary types (docs + shapes only, no transport).
pub mod net;
pub mod paint;
pub mod special;
pub mod sub;
pub mod tuning;
pub mod weapon;

/// Crate-wide semantic version, mirrored from the workspace `Cargo.toml`.
pub const SIM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Extracted gameplay tuning (`rust/assets/tuning.json`).
pub const TUNING_JSON: &str = include_str!("../../../assets/tuning.json");

/// Extracted Tidewater layout (`rust/assets/maps/tidewater.json`).
pub const TIDEWATER_JSON: &str = include_str!("../../../assets/maps/tidewater.json");

/// Extracted Saltpan Basin layout (`rust/assets/maps/saltpan.json`).
pub const SALTPAN_JSON: &str = include_str!("../../../assets/maps/saltpan.json");

/// Extracted Crossroads Market layout (`rust/assets/maps/crossmarket.json`).
pub const CROSSMARKET_JSON: &str = include_str!("../../../assets/maps/crossmarket.json");

/// Tuning compiled into the binary; panics only if the extracted artifact is
/// malformed (the pipeline output is schema-tested in CI).
#[must_use]
pub fn embedded_tuning() -> tuning::Tuning {
    tuning::tuning_from_str(TUNING_JSON)
        .expect("assets/tuning.json must be a valid tuning document")
}

/// Tidewater layout compiled into the binary.
#[must_use]
pub fn embedded_tidewater() -> geometry::StageLayout {
    geometry::layout_from_str(TIDEWATER_JSON)
        .expect("assets/maps/tidewater.json must be a valid stage layout document")
}

/// Saltpan layout compiled into the binary.
#[must_use]
pub fn embedded_saltpan() -> geometry::StageLayout {
    geometry::layout_from_str(SALTPAN_JSON)
        .expect("assets/maps/saltpan.json must be a valid stage layout document")
}

/// Crossmarket layout compiled into the binary.
#[must_use]
pub fn embedded_crossmarket() -> geometry::StageLayout {
    geometry::layout_from_str(CROSSMARKET_JSON)
        .expect("assets/maps/crossmarket.json must be a valid stage layout document")
}

/// The embedded layout for a stage id (Task 37: the extracted stages).
///
/// # Panics
/// Panics when `id` is not one of the embedded stage layouts.
#[must_use]
pub fn embedded_layout(id: &str) -> geometry::StageLayout {
    match id {
        "tidewater" => embedded_tidewater(),
        "saltpan" => embedded_saltpan(),
        "crossmarket" => embedded_crossmarket(),
        other => panic!("no embedded stage layout for id `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::SIM_VERSION;

    #[test]
    fn scaffold_smoke() {
        assert!(SIM_VERSION.contains("m1"));
    }
}
