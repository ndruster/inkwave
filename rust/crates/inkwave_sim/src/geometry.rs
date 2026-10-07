//! Raw stage geometry: oriented boxes extracted from an upstream layout into
//! `assets/maps/<stage>.json` (schema `inkwave.stage_layout.v1`).
//!
//! Only data here — no renderer, no collision solving (Task 4 builds the
//! collision world from these primitives). Coordinates are metres, Y-up.
//! See `assets/maps/stage-layout.schema.md` for field semantics.

use std::collections::HashMap;

use serde::Deserialize;

use crate::tuning::Source;

/// A three-component vector in world space ([x, y, z]).
pub type Vec3 = [f32; 3];

/// One collision/paint block. The `kind` tag ("box"/"obox"/"ramp") mirrors the
/// upstream mapkit/map layout defs; shared flags sit alongside the geometry.
///
/// Deserialised via [`FlatBrush`]: serde's internally-tagged enums cannot carry
/// a `flatten`ed struct inside their variants, so the flat representation is
/// parsed first and validated/split into the enum with `TryFrom`.
#[derive(Debug, Clone, PartialEq)]
pub enum Brush {
    /// Axis-aligned box.
    Box {
        min: Vec3,
        max: Vec3,
        common: BrushCommon,
    },
    /// Box rotated about the Y axis (`rot_y` in degrees).
    Obox {
        center: Vec3,
        size: Vec3,
        rot_y: f32,
        common: BrushCommon,
    },
    /// Tilted thick slab whose top surface runs `low` -> `high`.
    Ramp {
        low: Vec3,
        high: Vec3,
        width: f32,
        thickness: f32,
        thin: bool,
        common: BrushCommon,
    },
}

/// Flat on-wire representation of one primitive: geometry fields are optional
/// (each `kind` uses its own subset) and shared flags are flattened in.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatBrush {
    kind: String,
    min: Option<Vec3>,
    max: Option<Vec3>,
    center: Option<Vec3>,
    size: Option<Vec3>,
    rot_y: Option<f32>,
    low: Option<Vec3>,
    high: Option<Vec3>,
    width: Option<f32>,
    thickness: Option<f32>,
    #[serde(default)]
    thin: bool,
    #[serde(flatten)]
    common: BrushCommon,
}

impl TryFrom<FlatBrush> for Brush {
    type Error = String;

    fn try_from(f: FlatBrush) -> Result<Self, Self::Error> {
        let common = f.common;
        let need = |present: bool, field: &str| -> Result<(), String> {
            if present {
                Ok(())
            } else {
                Err(format!("{} missing `{field}`", f.kind))
            }
        };
        match f.kind.as_str() {
            "box" => {
                need(f.min.is_some(), "min")?;
                need(f.max.is_some(), "max")?;
                Ok(Brush::Box {
                    min: f.min.unwrap(),
                    max: f.max.unwrap(),
                    common,
                })
            }
            "obox" => {
                need(f.center.is_some(), "center")?;
                need(f.size.is_some(), "size")?;
                need(f.rot_y.is_some(), "rotY")?;
                Ok(Brush::Obox {
                    center: f.center.unwrap(),
                    size: f.size.unwrap(),
                    rot_y: f.rot_y.unwrap(),
                    common,
                })
            }
            "ramp" => {
                need(f.low.is_some(), "low")?;
                need(f.high.is_some(), "high")?;
                need(f.width.is_some(), "width")?;
                need(f.thickness.is_some(), "thickness")?;
                Ok(Brush::Ramp {
                    low: f.low.unwrap(),
                    high: f.high.unwrap(),
                    width: f.width.unwrap(),
                    thickness: f.thickness.unwrap(),
                    thin: f.thin,
                    common,
                })
            }
            other => Err(format!("unknown brush kind `{other}`")),
        }
    }
}

impl<'de> Deserialize<'de> for Brush {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Brush::try_from(FlatBrush::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Brush {
    /// Shared flags for every brush kind.
    pub fn common(&self) -> &BrushCommon {
        match self {
            Brush::Box { common, .. } | Brush::Obox { common, .. } | Brush::Ramp { common, .. } => {
                common
            }
        }
    }

    pub fn is_box(&self) -> bool {
        matches!(self, Brush::Box { .. })
    }
    pub fn is_obox(&self) -> bool {
        matches!(self, Brush::Obox { .. })
    }
    pub fn is_ramp(&self) -> bool {
        matches!(self, Brush::Ramp { .. })
    }
}

/// Wall decal placement (mural id + world-space normal it sticks to).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Mural {
    pub id: u32,
    pub n: Vec3,
}

/// Flags common to every primitive; defaults in the extractor match
/// `src/world/level.js` `_addBlock`, so nothing is re-derived here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushCommon {
    pub tag: Option<String>,
    pub color: String,
    pub pattern: u16,
    /// Faces accept ink (also false for grates/rails; roof/perch *tops* are
    /// excluded later at face build time).
    pub paint: bool,
    pub solid: bool,
    /// Kids walk on grates; squids/shots/ink pass through; never paintable.
    pub grate: bool,
    /// Railing: a collision-only grate subtype (the visible rail is a prop).
    pub rail: bool,
    /// Off-limits top: landing slides the actor off, top is not paintable.
    pub roof: bool,
    /// Standable but never paintable top.
    pub perch: bool,
    /// Bot navigation must not route along this top.
    pub no_nav: bool,
    /// Collision-only primitive (prop collider or rail): no faces generated.
    pub hidden: bool,
    pub bevel: Option<f32>,
    /// World-space normals that never accept ink.
    pub no_paint: Vec<Vec3>,
    pub mural: Vec<Mural>,
    /// Octagon-platform source marker `[cx, cz, circumradius]`.
    pub oct: Option<Vec3>,
}

/// Playfield bounds (XZ rectangle).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
}

/// Per-stage static info.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub id: String,
    pub bounds: Bounds,
    /// One spawn pad `[x, y, z]` per team (Alpha -Z, Bravo +Z).
    pub spawn_pads: Vec<Vec3>,
    pub spawn_barrier: f32,
    pub water_y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimitiveCounts {
    #[serde(rename = "box")]
    pub boxes: u32,
    pub obox: u32,
    pub ramp: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCounts {
    pub single: u32,
    pub half: u32,
    #[serde(rename = "halfMirrored")]
    pub half_mirrored: u32,
}

/// Node-side statistics echoed by the extractor for cross-checks (TR-3.3).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutMeta {
    pub primitive_counts: PrimitiveCounts,
    pub source_counts: SourceCounts,
    pub bounds: Bounds,
    pub spawn_pads: Vec<Vec3>,
    pub spawn_barrier: f32,
    pub water_y: f32,
    pub surface_slots: HashMap<String, u16>,
}

/// A full extracted stage layout document.
#[derive(Debug, Clone, Deserialize)]
pub struct StageLayout {
    pub schema: String,
    pub source: Source,
    pub stage: StageInfo,
    /// Positional: index == upstream block id.
    pub primitives: Vec<Brush>,
    pub meta: LayoutMeta,
}

/// Parse a stage layout document from JSON.
pub fn layout_from_str(s: &str) -> Result<StageLayout, serde_json::Error> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> StageLayout {
        crate::embedded_tidewater()
    }

    // TR-3.3: Tidewater stage info matches the node-side stats.
    #[test]
    fn stage_info() {
        let l = layout();
        assert_eq!(l.schema, "inkwave.stage_layout.v1");
        assert_eq!(l.source.path, "src/world/stages/tidewater/layout.js");
        assert_eq!(l.source.commit.len(), 40);
        assert_eq!(l.stage.id, "tidewater");
        let b = l.stage.bounds;
        assert_eq!(
            b,
            Bounds {
                min_x: -28.0,
                max_x: 28.0,
                min_z: -47.0,
                max_z: 47.0
            }
        );
        assert_eq!(
            l.stage.spawn_pads,
            vec![[0.0, 2.4, -41.8], [0.0, 2.4, 41.8]]
        );
        assert_eq!(l.stage.spawn_barrier, 4.2);
        assert_eq!(l.stage.water_y, -1.6);
        // meta must echo the same stage info.
        assert_eq!(l.meta.bounds, b);
        assert_eq!(l.meta.spawn_pads, l.stage.spawn_pads);
        assert_eq!(l.meta.spawn_barrier, l.stage.spawn_barrier);
        assert_eq!(l.meta.water_y, l.stage.water_y);
        assert_eq!(l.meta.surface_slots.get("herringbone"), Some(&28));
        assert_eq!(l.meta.surface_slots.get("terrazzo"), Some(&29));
        assert_eq!(l.meta.surface_slots.get("stucco"), Some(&30));
    }

    // TR-3.3: parsed primitive counts equal both the meta counts and the
    // frozen node-side numbers for the baseline commit.
    #[test]
    fn primitive_counts_match_meta() {
        let l = layout();
        let c = l.meta.primitive_counts;
        let mut boxes = 0u32;
        let mut oboxes = 0u32;
        let mut ramps = 0u32;
        for p in &l.primitives {
            match p {
                Brush::Box { .. } => boxes += 1,
                Brush::Obox { .. } => oboxes += 1,
                Brush::Ramp { .. } => ramps += 1,
            }
        }
        assert_eq!(boxes, c.boxes);
        assert_eq!(oboxes, c.obox);
        assert_eq!(ramps, c.ramp);
        assert_eq!(l.primitives.len() as u32, c.total);
        // frozen baseline (3e9b550): 99 single + 130 half + 130 mirrored.
        assert_eq!(
            c,
            PrimitiveCounts {
                boxes: 81,
                obox: 262,
                ramp: 16,
                total: 359
            }
        );
        let sc = l.meta.source_counts;
        assert_eq!(
            sc,
            SourceCounts {
                single: 99,
                half: 130,
                half_mirrored: 130
            }
        );
        assert_eq!(sc.single + sc.half + sc.half_mirrored, c.total);
    }

    // The expanded layout is mirror-symmetric: primitives after single+half
    // are the level.js mirrorDef of the half slice, in the same order.
    #[test]
    fn half_slice_is_mirrored_verbatim() {
        let l = layout();
        let n = l.primitives.len();
        let single = l.meta.source_counts.single as usize;
        let half = l.meta.source_counts.half as usize;
        assert_eq!(n, single + 2 * half);
        for i in 0..half {
            let a = &l.primitives[single + i];
            let m = &l.primitives[single + half + i];
            assert!(mirror_brush(a, m), "primitive {i} mirror mismatch");
        }
        // single block (Jubilee terrace) sits on the central axis: first
        // primitive is an obox ring slab near +x, small positive z.
        if let Brush::Obox { center, .. } = &l.primitives[0] {
            assert!(center[1] > 0.0);
        } else {
            panic!("primitive 0 must be the terrace ring obox");
        }
    }

    // Rail semantics baked by the extractor: rail => grate + hidden + !paint.
    #[test]
    fn rail_and_grate_flags() {
        let l = layout();
        let rails = l.primitives.iter().filter(|p| p.common().rail).count();
        assert!(rails > 10, "tidewater has many rail blocks, got {rails}");
        for p in l.primitives.iter().filter(|p| p.common().rail) {
            let c = p.common();
            assert!(c.grate);
            assert!(c.hidden);
            assert!(!c.paint);
        }
        // no tidewater primitive is non-solid
        assert!(l.primitives.iter().all(|p| p.common().solid));
    }

    // Spot-check known mirrored geometry (Town Hall loggia + west flight).
    #[test]
    fn known_mirrored_primitives_exist() {
        let l = layout();
        // flight-w ramp in Alpha half: low [-10.8,-0.2,-37.2] -> high [-4.8,2.4,-37.2]
        let alpha_flight = l.primitives.iter().any(|p| {
            matches!(p, Brush::Ramp { low, high, width, common, .. }
                if *low == [-10.8, -0.2, -37.2]
                    && *high == [-4.8, 2.4, -37.2]
                    && *width == 2.4
                    && common.tag.as_deref() == Some("flight-w"))
        });
        // mirrored in Bravo half (x,z negated).
        let bravo_flight = l.primitives.iter().any(|p| {
            matches!(p, Brush::Ramp { low, high, .. }
                if *low == [10.8, -0.2, 37.2] && *high == [4.8, 2.4, 37.2])
        });
        assert!(alpha_flight, "Alpha west flight ramp missing");
        assert!(bravo_flight, "mirrored west flight ramp missing");
        // townhall-wall box mirrored to +Z
        let wall = l.primitives.iter().any(|p| {
            matches!(p, Brush::Box { min, max, common, .. }
                if *min == [-22.0, -1.2, 45.4]
                    && *max == [9.5, 5.2, 46.0]
                    && common.tag.as_deref() == Some("townhall-wall"))
        });
        assert!(wall, "mirrored townhall-wall box missing");
    }

    // --- helpers ---

    fn mirror_brush(a: &Brush, b: &Brush) -> bool {
        if std::mem::discriminant(a) != std::mem::discriminant(b) {
            return false;
        }
        let ca = a.common();
        let cb = b.common();
        if ca.tag != cb.tag
            || ca.pattern != cb.pattern
            || ca.paint != cb.paint
            || ca.solid != cb.solid
            || ca.grate != cb.grate
            || ca.rail != cb.rail
            || ca.roof != cb.roof
            || ca.perch != cb.perch
            || ca.no_nav != cb.no_nav
            || ca.hidden != cb.hidden
        {
            return false;
        }
        match (a, b) {
            (
                Brush::Box {
                    min: mn0, max: mx0, ..
                },
                Brush::Box {
                    min: mn1, max: mx1, ..
                },
            ) => {
                // mirrorDef: y components keep their own side (min.y, max.y).
                mn1 == &[-mx0[0], mn0[1], -mx0[2]] && mx1 == &[-mn0[0], mx0[1], -mn0[2]]
            }
            (
                Brush::Obox {
                    center: c0,
                    size: s0,
                    rot_y: r0,
                    ..
                },
                Brush::Obox {
                    center: c1,
                    size: s1,
                    rot_y: r1,
                    ..
                },
            ) => c1 == &neg_xz(c0) && s0 == s1 && (r0 - r1).abs() < 1e-6,
            (
                Brush::Ramp {
                    low: l0,
                    high: h0,
                    width: w0,
                    thickness: t0,
                    thin: q0,
                    ..
                },
                Brush::Ramp {
                    low: l1,
                    high: h1,
                    width: w1,
                    thickness: t1,
                    thin: q1,
                    ..
                },
            ) => l1 == &neg_xz(l0) && h1 == &neg_xz(h0) && w0 == w1 && t0 == t1 && q0 == q1,
            _ => false,
        }
    }

    fn neg_xz(p: &Vec3) -> Vec3 {
        [-p[0], p[1], -p[2]]
    }
}
