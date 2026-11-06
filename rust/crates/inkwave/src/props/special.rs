//! Task 53 (D-T53-4) — the special-prop kit: the baked crab / jetpack /
//! kraken / missile / speaker geometry (extract-special.mjs →
//! `assets/props/special.json` + `special.bin.gz`) decoded into a
//! kind → parts table over one shared [`Archive`].
//!
//! This batch delivers the data + loader only; the gameplay consumers
//! (Boss Battle actors, power-ups) arrive in Task 54/57 and read
//! [`SpecialKit::parts`] + the referenced templates directly. The manifest's
//! `extras` anchors (muzzle/hatch/leg pivots, `ballR`, …) ride as the raw
//! [`JVal`] tree so the geometry contract stays typed while the anchor
//! vocabulary is not frozen here.
// The gameplay consumers land with Task 54/57; until then only the tests
// read this module (fx/mod.rs precedent).
#![allow(dead_code)]
use std::collections::HashMap;

use inkwave_sim::net::wire::{JVal, parse_json};

use super::archive::{Archive, SPECIAL_MANIFEST, assets_dir, decode_gz};

/// One baked part of a special kind: the template row in the archive plus the
/// manifest's bookkeeping (name/bucket/tris/verts).
#[derive(Debug, Clone, PartialEq)]
pub struct SpecialPart {
    /// Part name as authored in JS (`body`, `legs0.body`, …).
    pub name: String,
    /// Bucket name (`gloss` / `glow` / `paint`).
    pub bucket: String,
    /// Template index into [`Archive::templates`].
    pub tid: usize,
    /// Triangle count of the template.
    pub tris: usize,
    /// Vertex count of the template.
    pub verts: usize,
}

/// The parsed `special.json` manifest.
#[derive(Debug, Clone)]
pub struct SpecialManifest {
    /// kind id (`crab`, …) → parts in bake order.
    pub kinds: Vec<(String, Vec<SpecialPart>)>,
    /// Raw `kinds.<kind>.extras` anchor tree (Task 54/57 consumers).
    pub extras: HashMap<String, JVal>,
    pub templates: usize,
    pub verts: usize,
    pub tris: usize,
    pub bin_sha256: String,
}

/// Manifest + decoded geometry archive for the special props.
#[derive(Debug, Clone)]
pub struct SpecialKit {
    pub manifest: SpecialManifest,
    /// `None` when `special.bin.gz` is absent (silent degradation, D-T53-6⑦).
    pub archive: Option<Archive>,
}

fn num(v: Option<&JVal>) -> Option<f64> {
    v?.as_f64()
}

/// Parse the embedded `special.json`; `None` on any shape mismatch.
#[must_use]
pub fn parse_special_manifest(json: &str) -> Option<SpecialManifest> {
    let root = parse_json(json)?;
    let meta = root.field("meta")?;
    let kinds_v = root.field("kinds")?;
    let JVal::Obj(kind_rows) = kinds_v else {
        return None;
    };
    let mut kinds = Vec::with_capacity(kind_rows.len());
    let mut extras = HashMap::new();
    for (kind, spec) in kind_rows {
        let parts_v = spec.field("parts")?.as_array()?;
        let mut parts = Vec::with_capacity(parts_v.len());
        for p in parts_v {
            parts.push(SpecialPart {
                name: p.field("name")?.as_str()?.to_string(),
                bucket: p.field("bucket")?.as_str()?.to_string(),
                tid: num(p.field("tid"))? as usize,
                tris: num(p.field("tris"))? as usize,
                verts: num(p.field("verts"))? as usize,
            });
        }
        if let Some(e) = spec.field("extras") {
            extras.insert(kind.clone(), e.clone());
        }
        kinds.push((kind.clone(), parts));
    }
    Some(SpecialManifest {
        kinds,
        extras,
        templates: num(meta.field("templates"))? as usize,
        verts: num(meta.field("verts"))? as usize,
        tris: num(meta.field("tris"))? as usize,
        bin_sha256: meta.field("binSha256")?.as_str()?.to_string(),
    })
}

/// Load + decode `assets/props/special.bin.gz`; `None` when absent.
#[must_use]
pub fn load_special_archive() -> Option<Archive> {
    let gz = std::fs::read(assets_dir().join("special.bin.gz")).ok()?;
    decode_gz(&gz)
}

impl SpecialKit {
    /// Embedded manifest + runtime archive (the archive is optional; the
    /// manifest always parses — the tests pin that).
    #[must_use]
    pub fn load() -> Option<SpecialKit> {
        Some(SpecialKit {
            manifest: parse_special_manifest(SPECIAL_MANIFEST)?,
            archive: load_special_archive(),
        })
    }

    /// The parts of one kind (`crab`, `jetpack`, `kraken`, `missile`,
    /// `speaker`).
    #[must_use]
    pub fn parts(&self, kind: &str) -> Option<&[SpecialPart]> {
        self.manifest
            .kinds
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, p)| p.as_slice())
    }
}
