//! Task 51 — the ink particle FX system, ported from `src/fx/`:
//! batches A+B moved the data core (`fx.js`, 2267 lines) into
//! [`particles`] (pools / scheduler / culling) and [`api`] (the ≈60-method
//! spawn surface); batch C adds [`render`] (the Bevy instanced-billboard
//! placeholder: per-pool quad meshes rebuilt from the pool data every frame)
//! and batch D adds [`hooks`] (`fxHooks.js`, 712 lines: the gameplay → VFX
//! glue — MatchEvent consumption + the actor/projectile/bomb/cloud/beam/
//! ambient polling paths + the per-actor state machine).
//!
//! [`FxState`] is the Bevy `Resource` holding the data core plus the hooks
//! runtime; `world.rs` registers it and chains `fx_hooks` (before the UI
//! state machine that drains `Match::events`) and `fx_update` (at the tail
//! of the Update chain, after `camera_rig`).
//!
//! Batch E (D-T51-5) adds the sibling submodules [`zone_marks`]
//! (`zoneMarks.js`, 517 lines — the zone flood/contest/sweep marks data
//! layer + the simplified boundary-band mesh placeholder), [`swim_wake`]
//! (`swimWake.js`, 107 lines — the swimmer trail ring buffers feeding the
//! ink shader's wake uniforms, upload point deferred) and [`screen_fx`]
//! (`screenfx.js`, 1068 lines — the full-screen event → post-process
//! uniform state machine, data-only: the composite pass itself belongs to
//! Task 55).
//!
//! Declared deviations from the JS original (full list in the task report):
//!   * all intermediates are `f32` (JS runs `f64` through `Math.random` and
//!     the scratch vectors);
//!   * CSS colour parsing covers `#rgb` / `#rrggbb` plus a named subset
//!     (three's full CSS parser is not reproduced);
//!   * callbacks are `Box<dyn FnMut + Send + Sync>` taking `Vec3`/`f32` by
//!     value — JS hands out scratch objects and tolerates re-entrant calls
//!     back into `FX`; the port does not allow re-entrancy;
//!   * `update` receives the camera position; the renderer reads the camera
//!     basis itself (JS also reads camDir and the view matrix for the shader
//!     uniforms — GLSL precision deferred per D-T51-7②);
//!   * `dispose()` / the scene tree are not ported (no GPU resources here).

// The FX API surface is a faithful port of the JS call sites; the not-yet
// driven spawns / stats / pool utilities stay intentionally alive here for
// batch E (`zone_marks` / `swim_wake` / `screen_fx`) and the debug overlay.
// The blanket `allow` from the batch-A/B placeholder wiring (the temporary
// `#[allow(dead_code, unused_imports)] mod fx;` in `main.rs`) is converged
// onto this module.
#![allow(dead_code)]

pub mod api;
pub mod hooks;
pub mod particles;
pub mod render;
pub mod screen_fx;
pub mod swim_wake;
pub mod zone_marks;

use bevy::prelude::Resource;

use hooks::HooksRuntime;
use particles::{DEFAULT_SEED, Fx, FxOptions};

/// The Bevy-side home of the FX system: the pooled data core plus the
/// `fxHooks` runtime state (≡ `G.fx` + the `FxHooks` instance in JS).
#[derive(Resource)]
pub struct FxState {
    /// ≡ `G.fx` — the particle pools / scheduler / spawn API.
    pub fx: Fx,
    /// ≡ the `FxHooks` instance (per-actor state, `seen` flags, counters).
    pub hooks: HooksRuntime,
}

impl FxState {
    /// Build the pair with an explicit RNG seed. `hooks.bootstrap` consumes
    /// the first `rand()` draws exactly like the JS constructor (L64).
    #[must_use]
    pub fn new(seed: u32) -> Self {
        let mut fx = Fx::with_seed(FxOptions::default(), seed);
        let mut hooks = HooksRuntime::new();
        hooks.bootstrap(&mut fx);
        Self { fx, hooks }
    }
}

impl Default for FxState {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}
