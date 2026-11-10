//! Task 54 batch B1: the JS character spring-pose runtime ported to pure Rust.
//!
//! `src/game/character.js` (READ-ONLY reference) drives the procedural, layered
//! character animation: root tracking → stepping → pose layers → application.
//! This module is the headless data core — no Bevy scene graph, no three.js:
//! [`anim::CharAnim`] holds all per-character animation state and runs the same
//! maths the JS does, [`apply`] produces per-node LOCAL transforms, and the
//! pinned test diffs those local pos/quat values (plus the composed weapon
//! frame transforms) against the reference fixture recorded by
//! `rust/tools/extract/char-pose-reference.mjs`, one value per frame.
//!
//! Batch B2 landed the face / fingers / jiggle / gaze / micro-expression /
//! dance / one-shot layers and the pure-data material pass. Only five
//! render-side or Non-Goal methods stay patched to no-ops on BOTH sides
//! (`_poseLocker`, `_attendMenu`, `_pickLook`, `_squidBlink`,
//! `_updateMaterials`); the fixture header `meta.b2Noops` is authoritative.
//! `bridge.rs` applies `update_materials` to cloned Bevy materials.
//!
//! Every helper mirrors a `character.js` line range (`≡ character.js L####`);
//! all tables load at runtime from `rust/assets/char/char-poses.json` +
//! `char.json` (batch A extraction) — nothing is hand-copied.
#[cfg(test)]
mod pinned;

pub mod anim;
pub mod apply;
pub mod bridge;
pub mod build;
pub mod feet;
pub mod hair;
pub mod math;
pub mod rig;
pub mod squid;
pub mod tables;
pub mod weapon;
