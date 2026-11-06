//! Task 53 decode round-trip tests (D-T53-2): every stage archive must decode
//! to exactly what its embedded manifest promises (template/vertex/triangle
//! counts, bucket table, per-placement instance data), and the special archive
//! must decode with an empty instance table.

use super::archive::{SPECIAL_MANIFEST, STAGE_IDS, load_stage_archive, stage_manifest};
use inkwave_sim::net::wire::{JVal, parse_json};

fn totals(archive: &super::archive::Archive) -> (u64, u64) {
    let mut verts = 0u64;
    let mut tris = 0u64;
    for t in &archive.templates {
        verts += t.positions.len() as u64;
        tris += (t.indices.len() / 3) as u64;
    }
    (verts, tris)
}

#[test]
fn every_stage_archive_decodes_to_its_manifest() {
    for stage in STAGE_IDS {
        let manifest = stage_manifest(stage)
            .unwrap_or_else(|| panic!("{stage}: embedded manifest must parse"));
        assert_eq!(manifest.meta.stage, stage, "{stage}: meta.stage");
        let archive = load_stage_archive(stage)
            .unwrap_or_else(|| panic!("{stage}: assets/props/{stage}.bin.gz must decode"));

        assert_eq!(
            archive.templates.len() as u64,
            manifest.meta.templates,
            "{stage}: template count"
        );
        let (verts, tris) = totals(&archive);
        assert_eq!(verts, manifest.meta.verts, "{stage}: vertex total");
        assert_eq!(tris, manifest.meta.tris, "{stage}: triangle total");
        assert_eq!(
            archive.buckets, manifest.meta.buckets,
            "{stage}: bucket table"
        );

        let inst_expected: u64 = manifest.placements.iter().map(|p| p.inst_count).sum();
        assert_eq!(
            archive.instances.len() as u64,
            inst_expected,
            "{stage}: instance count"
        );
        assert_eq!(
            archive.spins.len() as u64,
            manifest.counts.spin,
            "{stage}: spins"
        );
        assert_eq!(
            archive.blinks.len() as u64,
            manifest.counts.blink,
            "{stage}: blinks"
        );
        // decor: pads bake as 2 instances + one mesh each (>= decorMeshes is
        // the contract; flags:cloth multiplies per team instance).
        assert!(
            archive.decor.len() as u64 >= manifest.counts.decor_meshes,
            "{stage}: decor instances"
        );

        // Per-placement round-trip: the archive's instance rows at
        // [inst_start, +inst_count) must carry the manifest's placement
        // transform within the quantisation band (1 mm / f32).
        for p in &manifest.placements {
            for k in p.inst_start..p.inst_start + p.inst_count {
                let inst = archive
                    .instances
                    .get(k as usize)
                    .unwrap_or_else(|| panic!("{stage}: placement {} instance {k} missing", p.i));
                assert_eq!(
                    inst.mode_flags, p.mode_flags,
                    "{stage}: placement {} modeFlags",
                    p.i
                );
                assert!(
                    (inst.rot_y - p.rot_y as f32).abs() < 1e-5,
                    "{stage}: placement {} rotY",
                    p.i
                );
                assert!(
                    (inst.scale - p.scale as f32).abs() < 1e-5,
                    "{stage}: placement {} scale",
                    p.i
                );
                for (a, b) in inst.pos.iter().zip(p.pos) {
                    assert!(
                        (a - b as f32).abs() <= 1.5e-3,
                        "{stage}: placement {} pos",
                        p.i
                    );
                }
            }
        }
    }
}

#[test]
fn special_archive_decodes_with_empty_tables() {
    let root = parse_json(SPECIAL_MANIFEST).expect("special manifest parses");
    let meta = root.field("meta").expect("meta");
    let templates = meta
        .field("templates")
        .and_then(|v| v.as_f64())
        .expect("templates") as usize;
    let gz = std::fs::read(super::archive::assets_dir().join("special.bin.gz"))
        .expect("special.bin.gz present");
    let archive = super::archive::decode_gz(&gz).expect("special archive decodes");
    assert_eq!(archive.templates.len(), templates, "special template count");
    assert!(archive.instances.is_empty(), "special instances empty");
    assert!(archive.spins.is_empty() && archive.blinks.is_empty() && archive.decor.is_empty());
    // every kind part references a real template; buckets are the three roles.
    let kinds = root.field("kinds").expect("kinds");
    let mut seen = 0usize;
    let JVal::Obj(kind_rows) = kinds else {
        panic!("kinds object")
    };
    for (_kind, spec) in kind_rows {
        for part in spec
            .field("parts")
            .and_then(|v| v.as_array())
            .expect("parts")
        {
            let tid = part.field("tid").and_then(|v| v.as_f64()).expect("tid") as usize;
            assert!(tid < templates, "special part tid in range");
            let bucket = part
                .field("bucket")
                .and_then(|v| v.as_str())
                .expect("bucket");
            assert!(
                matches!(bucket, "gloss" | "paint" | "glow"),
                "special bucket role: {bucket}"
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 35, "special part rows (5 kinds)");
}

#[test]
fn mode_filter_matches_variants_js() {
    // variants.js L6 expanded to the props.schema.md 0..4 vocabulary.
    use super::archive::mode_passes;
    assert!(mode_passes(0, "turf") && mode_passes(0, "zones"));
    assert!(mode_passes(1, "turf") && !mode_passes(1, "zones")); // onlyIn turf
    assert!(mode_passes(2, "zones") && !mode_passes(2, "turf")); // onlyIn zones
    assert!(mode_passes(3, "zones") && !mode_passes(3, "turf")); // notIn turf
    assert!(mode_passes(4, "turf") && !mode_passes(4, "zones")); // notIn zones
    assert!(!mode_passes(9, "turf"));
}

#[test]
fn decode_rejects_corrupt_archives() {
    use super::archive::decode;
    assert!(decode(&[]).is_none());
    let mut good = Vec::new();
    {
        // minimal valid archive: 0 buckets, 0 of every table.
        good.extend_from_slice(&0x3150_4B49u32.to_le_bytes());
        good.extend_from_slice(&1u16.to_le_bytes());
        good.extend_from_slice(&0u16.to_le_bytes());
        for _ in 0..5 {
            good.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    assert!(decode(&good).is_some());
    // wrong magic
    let mut bad = good.clone();
    bad[0] ^= 0xff;
    assert!(decode(&bad).is_none());
    // truncated
    assert!(decode(&good[..good.len() - 2]).is_none());
    // trailing garbage
    let mut bad = good.clone();
    bad.push(0);
    assert!(decode(&bad).is_none());
}

#[test]
fn manifest_lamps_and_murals_are_sane() {
    for stage in STAGE_IDS {
        let m = stage_manifest(stage).unwrap();
        assert!(m.lamps.len() <= 12, "{stage}: lamps ≤ 12 (setLevelLamps)");
        for l in &m.lamps {
            assert!(l[1] > 0.0, "{stage}: lamp y above ground");
        }
        // 12 mural slots: 4 base rows + up to 8 stage rows (murals.js L60-71).
        assert!(
            !m.murals.is_empty() && m.murals.len() <= 12,
            "{stage}: mural rows"
        );
        for row in &m.murals {
            assert!(
                row.rect[1] > 0.0 && row.rect[3] > 0.0,
                "{stage}: mural rect"
            );
            // The 4 base rows are placeholders (murals.js L64: fx [0,0],
            // place [0,-8,0,0]); only stage rows carry a real face placement
            // (place = [x0, xLen, y0, yLen], xLen < 0 = strip repeat).
            if row.fx != [0.0, 0.0] {
                assert!(
                    row.place[3] > 0.0 && row.place[1] != 0.0,
                    "{stage}: mural place rows"
                );
            }
        }
    }
}

#[test]
fn manifest_meta_counts_are_self_consistent() {
    // placements/tris/verts must agree with the archive tables (the
    // round-trip test covers the archive; here: manifest-internal sanity).
    for stage in STAGE_IDS {
        let m = stage_manifest(stage).unwrap();
        assert_eq!(
            m.placements.len() as u64,
            m.meta.placements,
            "{stage}: placement rows"
        );
        let tris: u64 = m.placements.iter().map(|p| p.tris).sum();
        // placement tris include spin/blink estimates; they may exceed the
        // template tris (blink counts 84 each) but never fall below the
        // static bake by more than the blink/spin allowance.
        assert!(
            tris + m.counts.blink * 84 + m.counts.spin * 400 >= m.meta.tris,
            "{stage}: tris sanity"
        );
        assert!(m.meta.bin_sha256.len() == 64, "{stage}: sha256 hex");
    }
}

// D-T53-4: the special kit loader — kind tables map 1:1 onto archive
// templates (counts, vertex/triangle agreement, bucket roles).
#[test]
fn special_kit_parts_map_to_templates() {
    let kit = super::special::SpecialKit::load().expect("special kit loads");
    let archive = kit.archive.as_ref().expect("special.bin.gz decodes");
    assert_eq!(
        kit.manifest
            .kinds
            .iter()
            .map(|(k, p)| (k.as_str(), p.len()))
            .collect::<Vec<_>>(),
        vec![
            ("crab", 22),
            ("jetpack", 3),
            ("kraken", 2),
            ("missile", 3),
            ("speaker", 5),
        ],
        "kind → part counts"
    );
    assert_eq!(kit.manifest.templates, 35);
    assert_eq!(kit.manifest.verts, 40_200);
    assert_eq!(kit.manifest.tris, 82_010);
    let mut verts = 0usize;
    let mut tris = 0usize;
    for (kind, parts) in &kit.manifest.kinds {
        assert_eq!(
            kit.parts(kind).unwrap().len(),
            parts.len(),
            "{kind}: parts()"
        );
        for p in parts {
            let t = archive
                .templates
                .get(p.tid)
                .unwrap_or_else(|| panic!("{kind}/{}: tid {} out of range", p.name, p.tid));
            assert_eq!(t.positions.len(), p.verts, "{kind}/{}: verts", p.name);
            assert_eq!(t.indices.len() / 3, p.tris, "{kind}/{}: tris", p.name);
            assert!(
                archive.buckets.iter().any(|b| b.name == p.bucket),
                "{kind}/{}: bucket {bucket} in archive table",
                p.name,
                bucket = p.bucket
            );
            verts += p.verts;
            tris += p.tris;
        }
    }
    assert_eq!(verts, kit.manifest.verts, "part verts sum to meta");
    assert_eq!(tris, kit.manifest.tris, "part tris sum to meta");
    assert_eq!(kit.manifest.bin_sha256.len(), 64, "special bin sha256 hex");
    assert!(
        kit.manifest.extras.contains_key("crab"),
        "extras anchors ride the manifest"
    );
}

// D-T53-5: the mapThumb port is byte-exact against the node-generated goldens
// (rust/tools/extract/gen-mapthumb-golden.mjs, 8 stages × 3 themes).
#[test]
fn mapthumb_goldens_match_byte_for_byte() {
    for stage in STAGE_IDS {
        for theme in ["day", "sunset", "golden"] {
            let path = super::archive::assets_dir()
                .join("thumb")
                .join(format!("{stage}-{theme}.svg"));
            let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!("{stage}/{theme}: golden {} missing ({e})", path.display())
            });
            let got = super::mapthumb::embedded_thumb_svg(stage, theme, ["#18c7e8", "#ff4a5a"]);
            if got != golden {
                // first divergent byte window for the failure message
                let diff = got
                    .bytes()
                    .zip(golden.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(got.len().min(golden.len()));
                let ctx = |s: &str| {
                    let b: Vec<u8> = s.bytes().skip(diff.saturating_sub(40)).take(120).collect();
                    String::from_utf8_lossy(&b).into_owned()
                };
                panic!(
                    "{stage}/{theme}: byte {diff} differs\n rust: {}\n js:   {}",
                    ctx(&got),
                    ctx(&golden)
                );
            }
        }
    }
}

/// Diagnostic (run with `INKWAVE_THUMB_AUDIT=1 cargo test … --ignored`):
/// writes every divergent fixture to `/tmp/thumb-rust` instead of stopping at
/// the first one — used once per upstream bump to size the golden drift.
#[test]
#[ignore]
fn mapthumb_golden_audit() {
    let mut total = 0usize;
    for stage in STAGE_IDS {
        for theme in ["day", "sunset", "golden"] {
            let path = super::archive::assets_dir()
                .join("thumb")
                .join(format!("{stage}-{theme}.svg"));
            let golden = std::fs::read_to_string(&path).expect("golden present");
            let got = super::mapthumb::embedded_thumb_svg(stage, theme, ["#18c7e8", "#ff4a5a"]);
            if got != golden {
                let out = std::path::Path::new("/tmp/thumb-rust");
                std::fs::create_dir_all(out).unwrap();
                std::fs::write(out.join(format!("{stage}-{theme}.svg")), &got).unwrap();
                total += 1;
                println!(
                    "{stage}/{theme}: rust {} bytes vs js {}",
                    got.len(),
                    golden.len()
                );
            }
        }
    }
    println!("mapthumb audit: {total} divergent fixtures written to /tmp/thumb-rust");
}

// D-T53-4: mural reconciliation — every mural id the stage layout stamps on a
// face has a manifest placement row, and the manifest's stage rows are the
// atlas slots 4…11 (rows 0-3 are the shared base strips, murals.js L64;
// tidewater's rows 6/10 are drawn but placed on no block — the JS runtime
// never samples them either, so the contract is layout ⊆ manifest).
#[test]
fn mural_rows_reconcile_with_layout_markers() {
    for stage in STAGE_IDS {
        let m = stage_manifest(stage).unwrap();
        let layout = inkwave_sim::embedded_layout(stage);
        let laid: std::collections::BTreeSet<u64> = layout
            .primitives
            .iter()
            .flat_map(|p| p.common().mural.iter())
            .map(|mu| mu.id as u64)
            .collect();
        let rows: std::collections::BTreeSet<u64> = m.murals.iter().map(|r| r.id).collect();
        for id in &laid {
            assert!(
                rows.contains(id),
                "{stage}: layout mural {id} has no manifest row"
            );
            assert!(
                *id >= 4 && *id <= 11,
                "{stage}: mural id {id} outside the stage range"
            );
        }
        let base: Vec<u64> = m
            .murals
            .iter()
            .filter(|r| r.fx == [0.0, 0.0])
            .map(|r| r.id)
            .collect();
        assert_eq!(base, vec![0, 1, 2, 3], "{stage}: base strip rows");
        assert!(
            m.murals.iter().all(|r| r.id <= 11),
            "{stage}: manifest mural ids in 0..=11"
        );
    }
}

// Review P0-1: ECMA `toFixed` breaks an exact tie by picking the larger n
// AFTER stripping the sign, so a negative tie rounds away from zero. The
// current 24 mapthumb goldens contain no negative odd/2^(f+1) coordinate, so
// the byte-identical check cannot catch a regression here — pinned directly.
#[test]
fn fmt_fixed_negative_tie_matches_tofixed() {
    use super::mapthumb::fmt_fixed;
    assert_eq!(fmt_fixed(-0.25, 1), "-0.3"); // (-0.25).toFixed(1)
    assert_eq!(fmt_fixed(0.25, 1), "0.3");
    assert_eq!(fmt_fixed(-0.75, 1), "-0.8");
    assert_eq!(fmt_fixed(-2.625, 2), "-2.63"); // (-2.625).toFixed(2)
    assert_eq!(fmt_fixed(-0.0625, 3), "-0.063");
    assert_eq!(fmt_fixed(0.125, 2), "0.13");
    // non-tie values keep the ordinary rounding path
    assert_eq!(fmt_fixed(-0.24, 1), "-0.2");
    assert_eq!(fmt_fixed(-0.26, 1), "-0.3");
    assert_eq!(fmt_fixed(-0.251, 1), "-0.3");
}

// Review P1-2: the blink bulb must wind outward; a reversed winding renders
// the far hemisphere's inner wall under Bevy's default back-face culling.
// Signed volume via the divergence theorem (Three SphereGeometry(1,8,6) ≈
// +3.51861; our mirrored u-parameterisation needs its own diagonals).
#[test]
fn blink_sphere_winds_outward() {
    let (positions, _, _, indices) = super::blink_sphere_data();
    assert_eq!(indices.len() % 3, 0);
    let mut vol = 0f64;
    for tri in indices.chunks(3) {
        let p = |k: usize| positions[tri[k] as usize];
        let (a, b, c) = (p(0), p(1), p(2));
        let cross = [
            f64::from(b[1] * c[2] - b[2] * c[1]),
            f64::from(b[2] * c[0] - b[0] * c[2]),
            f64::from(b[0] * c[1] - b[1] * c[0]),
        ];
        vol += f64::from(a[0]) * cross[0] + f64::from(a[1]) * cross[1] + f64::from(a[2]) * cross[2];
    }
    vol /= 6.0;
    // Three SphereGeometry(1, 8, 6) ≈ 3.51861; the sign is what is pinned.
    assert!(vol > 0.0, "blink sphere winds inward: signed volume {vol}");
    assert!(
        vol > 3.4 && vol < 3.7,
        "unexpected blink sphere volume {vol}"
    );
}
