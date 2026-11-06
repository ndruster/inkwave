//! Procedural texture library port (M4 Task 52, spec.md D-T52-1..6).
//!
//! Batch A: [`schema`] — the extracted `texlib.json` document (layer meta,
//! slot tables, ink/lamp/material constants) + the `uTL`/`uTLt`/`uTLs`
//! construction ≡ `src/world/levelMaterial.js` L96-99.
//! Batches B/C add the CPU bake core and the 46-layer surf programs;
//! batch E adds the on-demand bake resource.

pub mod schema;
// Batch C: 46 layer surf programs (25 base in `materials`, 21 stage in
// `materials_stage`). Stubs until the ports land.
pub mod materials;
pub mod materials_stage;

// Batch B: the CPU bake core (consumed by batch E's `res`).
pub mod bake;
pub mod r#gen; // `gen` is a reserved keyword in edition 2024 → raw ident

// Batch E: the on-demand bake resource (`TexLibBake` + upload poll).
pub mod res;
