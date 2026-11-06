//! Texlib data document (M4 Task 52 batch A, spec.md D-T52-1 / TR-52.1).
//!
//! Parses `rust/assets/texlib.json` (schema `inkwave.texlib.v1`), extracted by
//! `rust/tools/extract/extract-texlib.mjs` from
//! `src/world/texlib.js` (25 MATERIALS layers + the 21 stage layers appended
//! at L1167), `src/world/levelMaterial.js` (the 50-slot table L79-91) and
//! `src/world/inkShading.js` (INK_RIPPLES L22).
//!
//! [`slot_uniforms`] reproduces the `uTL` / `uTLt` / `uTLs` construction
//! (levelMaterial.js L96-99) so the WGSL side gets the same three vec4 tables
//! the JS shader receives. The GLSL bodies themselves are hand-ported in
//! batches B/C/D against the source line anchors.
//!
//! Parsing goes through the sim crate's public [`inkwave_sim::net::wire::JVal`]
//! (order-preserving JSON) — the inkwave crate carries no serde dependency of
//! its own (Task 52 discipline: no new deps, sim zero changes).

use inkwave_sim::net::wire::{JVal, parse_json};

/// The extracted document, compiled into the binary (≡ the `TUNING_JSON`
/// pattern in `inkwave_sim/src/lib.rs`).
pub const TEXLIB_JSON: &str = include_str!("../../../../assets/texlib.json");

/// One MATERIALS layer (texlib.js entry meta; prep/surf GLSL not carried).
#[derive(Debug, Clone, PartialEq)]
pub struct TexLayer {
    /// Layer name; stage layers are `'<stage>:<name>'` (texlib.js L1167).
    pub name: String,
    /// Uber-program group, `m.group || 0` (texlib.js L1367).
    pub group: u32,
    pub detail: f32,
    pub scale: f32,
    /// `tint` — absent ⇒ true (JS `M[n].tint === false` only fires on an
    /// explicit `false`, levelMaterial.js L97).
    pub tint: bool,
    /// `mask` — premultiplied paint mask tint mode (L97 `? 2`).
    pub mask: bool,
    /// `stair` — ramp-frame sampling params (levelMaterial.js L99), absent ⇒
    /// `[0,0,0,0]`.
    pub stair: Option<[f32; 4]>,
    pub alpha: bool,
    /// Anti-tiling mode: 0 PLAIN / 1 GRID / 2 HEX (texlib.js L85-86).
    pub mode: u32,
    /// Allowed grid transforms bitmask (mirror-u / mirror-v / transpose).
    pub sym: u32,
    /// Height range in metres mapped to orm.a (texlib.js L205).
    pub hr: [f32; 2],
    /// AO strength into the cavity bake (texlib.js L190 `uAO`).
    pub ao: f32,
}

/// The 50-slot pattern table (levelMaterial.js L79-91), resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotTables {
    /// Slot → layer name (`map`, L79-85; slot 49 = TL_SIDE).
    pub map: Vec<String>,
    /// Per-slot vertical-face remap, `onWall[i] ?? i` (L89-91/L97).
    pub on_wall: Vec<u32>,
    /// Per-slot top-face remap, `onTop[i] ?? i` (L90-91/L97).
    pub on_top: Vec<u32>,
}

/// Wet-ink layer constants (inkShading.js L22; levelMaterial.js L47-50/L62).
#[derive(Debug, Clone, PartialEq)]
pub struct InkConsts {
    /// `INK_RIPPLES` (inkShading.js L22).
    pub ripples: u32,
    /// `uInkGlow` default (levelMaterial.js L50).
    pub glow: f32,
    /// `uTeamA` hex (levelMaterial.js L47).
    pub team_a: String,
    /// `uTeamB` hex (levelMaterial.js L48).
    pub team_b: String,
    /// `uPpm` default — atlas texels per metre (levelMaterial.js L62).
    pub ppm: f32,
}

/// Dusk street-lamp pools (levelMaterial.js L11).
#[derive(Debug, Clone, PartialEq)]
pub struct LampConsts {
    /// `MAX_LAMPS`.
    pub max: u32,
    /// `LAMP_R` (m).
    pub radius: f32,
    /// `LAMP_HEX`.
    pub hex: String,
    /// `LAMP_I` (× night factor k, L35).
    pub intensity: f32,
}

/// Material baseline (levelMaterial.js L40-41). `clearcoat` is approximated
/// on the Rust side (spec D-T52-7②: Bevy StandardMaterial has no clearcoat
/// channel here); the values are carried for the approximation + Task 55.
#[derive(Debug, Clone, PartialEq)]
pub struct MatConsts {
    pub roughness: f32,
    pub metalness: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
}

/// Bake geometry (texlib.js L1332 / L4 header / GEN_MAIN L185 2×2 sub-sample).
#[derive(Debug, Clone, PartialEq)]
pub struct TexConsts {
    /// `createTextureLibrary` default size.
    pub size_high: u32,
    /// Low-quality size (texlib.js L4 `// 256 for low quality`).
    pub size_low: u32,
    /// Sub-samples per axis (2×2 = 4 taps per texel).
    pub sub: u32,
}

/// The whole `texlib.json` document.
#[derive(Debug, Clone, PartialEq)]
pub struct TexLibDoc {
    pub schema: String,
    /// Upstream commit the artifact was extracted from.
    pub commit: String,
    pub layers: Vec<TexLayer>,
    pub slots: SlotTables,
    pub ink: InkConsts,
    pub lamps: LampConsts,
    pub mat: MatConsts,
    pub tex: TexConsts,
}

/// The three per-slot uniform tables, ≡ levelMaterial.js L96-99.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotUniforms {
    /// `uTL[i] = (layerIdx, 1/scale, mode, sym)` (L96).
    pub u_tl: Vec<[f32; 4]>,
    /// `uTLt[i] = (tintMode, normalStrength, onWall, onTop)` (L97).
    pub u_tl_t: Vec<[f32; 4]>,
    /// `uTLs[i] = stair || [0,0,0,0]` (L99).
    pub u_tl_s: Vec<[f32; 4]>,
}

fn f32_of(v: &JVal, key: &str) -> Option<f32> {
    v.field(key)?.as_f64().map(|x| x as f32)
}
fn u32_of(v: &JVal, key: &str) -> Option<u32> {
    v.field(key)?.as_f64().map(|x| x as u32)
}
fn str_of<'a>(v: &'a JVal, key: &str) -> Option<&'a str> {
    v.field(key)?.as_str()
}
/// `bool` with the JS-side default for an absent key.
fn bool_of(v: &JVal, key: &str, default: bool) -> Option<bool> {
    match v.field(key) {
        None => Some(default),
        Some(x) => x.as_bool(),
    }
}
fn u32_of_or(v: &JVal, key: &str, default: u32) -> Option<u32> {
    match v.field(key) {
        None => Some(default),
        Some(_) => u32_of(v, key),
    }
}
fn f32_arr2(v: &JVal, key: &str) -> Option<[f32; 2]> {
    let a = v.field(key)?.as_array()?;
    if a.len() != 2 {
        return None;
    }
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32])
}
/// `Option`-of-`Option`: outer = malformed, inner = key absent (stair).
type OptStair = Option<Option<[f32; 4]>>;

fn f32_arr4(v: &JVal, key: &str) -> OptStair {
    match v.field(key) {
        None => Some(None),
        Some(x) => {
            let a = x.as_array()?;
            if a.len() != 4 {
                return None;
            }
            let mut out = [0.0f32; 4];
            for (i, e) in a.iter().enumerate() {
                out[i] = e.as_f64()? as f32;
            }
            Some(Some(out))
        }
    }
}
fn u32_vec(v: &JVal, key: &str) -> Option<Vec<u32>> {
    v.field(key)?
        .as_array()?
        .iter()
        .map(|e| e.as_f64().map(|x| x as u32))
        .collect()
}
fn str_vec(v: &JVal, key: &str) -> Option<Vec<String>> {
    v.field(key)?
        .as_array()?
        .iter()
        .map(|e| e.as_str().map(String::from))
        .collect()
}

/// Parse a `texlib.json` document; `None` on any malformed field.
#[must_use]
pub fn texlib_from_str(s: &str) -> Option<TexLibDoc> {
    let root = parse_json(s)?;
    let schema = str_of(&root, "schema")?.to_string();
    if schema != "inkwave.texlib.v1" {
        return None;
    }
    let commit = str_of(root.field("source")?, "commit")?.to_string();
    let layers = root
        .field("layers")?
        .as_array()?
        .iter()
        .map(|l| {
            Some(TexLayer {
                name: str_of(l, "name")?.to_string(),
                group: u32_of_or(l, "group", 0)?,
                detail: f32_of(l, "detail")?,
                scale: f32_of(l, "scale")?,
                tint: bool_of(l, "tint", true)?,
                mask: bool_of(l, "mask", false)?,
                stair: f32_arr4(l, "stair")?,
                alpha: bool_of(l, "alpha", false)?,
                mode: u32_of(l, "mode")?,
                sym: u32_of(l, "sym")?,
                hr: f32_arr2(l, "hr")?,
                ao: f32_of(l, "ao")?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let slots_val = root.field("slots")?;
    let slots = SlotTables {
        map: str_vec(slots_val, "map")?,
        on_wall: u32_vec(slots_val, "onWall")?,
        on_top: u32_vec(slots_val, "onTop")?,
    };
    let ink_val = root.field("ink")?;
    let ink = InkConsts {
        ripples: u32_of(ink_val, "ripples")?,
        glow: f32_of(ink_val, "glow")?,
        team_a: str_of(ink_val, "teamA")?.to_string(),
        team_b: str_of(ink_val, "teamB")?.to_string(),
        ppm: f32_of(ink_val, "ppm")?,
    };
    let lamp_val = root.field("lamps")?;
    let lamps = LampConsts {
        max: u32_of(lamp_val, "max")?,
        radius: f32_of(lamp_val, "radius")?,
        hex: str_of(lamp_val, "hex")?.to_string(),
        intensity: f32_of(lamp_val, "intensity")?,
    };
    let mat_val = root.field("mat")?;
    let mat = MatConsts {
        roughness: f32_of(mat_val, "roughness")?,
        metalness: f32_of(mat_val, "metalness")?,
        clearcoat: f32_of(mat_val, "clearcoat")?,
        clearcoat_roughness: f32_of(mat_val, "clearcoatRoughness")?,
    };
    let tex_val = root.field("tex")?;
    let tex = TexConsts {
        size_high: u32_of(tex_val, "sizeHigh")?,
        size_low: u32_of(tex_val, "sizeLow")?,
        sub: u32_of(tex_val, "sub")?,
    };
    Some(TexLibDoc {
        schema,
        commit,
        layers,
        slots,
        ink,
        lamps,
        mat,
        tex,
    })
}

/// The embedded document; panics only if the artifact is malformed (the
/// extraction pipeline is schema-tested in CI, ≡ `embedded_tuning`).
#[must_use]
pub fn embedded_texlib() -> TexLibDoc {
    texlib_from_str(TEXLIB_JSON).expect("assets/texlib.json must be a valid texlib document")
}

/// `lib.layers` name → layer index (the JS `L` object, levelMaterial.js L78;
/// `L[n] ?? 0` / `L.gel ?? -1` fallbacks are the caller's).
#[must_use]
pub fn layer_index(doc: &TexLibDoc, name: &str) -> Option<usize> {
    doc.layers.iter().position(|l| l.name == name)
}

/// Build the three uniform tables ≡ levelMaterial.js L96-99.
#[must_use]
pub fn slot_uniforms(doc: &TexLibDoc) -> SlotUniforms {
    let mut u_tl = Vec::with_capacity(doc.slots.map.len());
    let mut u_tl_t = Vec::with_capacity(doc.slots.map.len());
    let mut u_tl_s = Vec::with_capacity(doc.slots.map.len());
    for (i, n) in doc.slots.map.iter().enumerate() {
        let m = doc.layers.iter().find(|l| &l.name == n);
        // L96: (L[n] ?? 0, 1/((M[n] && M[n].scale) || 4), (M[n] && M[n].mode) ?? 1,
        //      (M[n] && M[n].sym) ?? 7)
        let idx = layer_index(doc, n).unwrap_or(0) as f32;
        let scale = m.map_or(0.0, |l| l.scale);
        let inv_scale = 1.0 / if scale == 0.0 { 4.0 } else { scale };
        let mode = m.map_or(1, |l| l.mode) as f32;
        let sym = m.map_or(7, |l| l.sym) as f32;
        u_tl.push([idx, inv_scale, mode, sym]);
        // L97: (mask ? 2 : tint === false ? 0 : 1, n === 'grate' ? 0.6 : 1.0,
        //      onWall[i] ?? i, onTop[i] ?? i)
        let tint_mode = match m {
            Some(l) if l.mask => 2.0,
            Some(l) if !l.tint => 0.0,
            _ => 1.0,
        };
        let nrm = if n == "grate" { 0.6 } else { 1.0 };
        u_tl_t.push([
            tint_mode,
            nrm,
            doc.slots.on_wall[i] as f32,
            doc.slots.on_top[i] as f32,
        ]);
        // L99: (...(M[n] && M[n].stair) || [0, 0, 0, 0])
        u_tl_s.push(m.and_then(|l| l.stair).unwrap_or([0.0; 4]));
    }
    SlotUniforms {
        u_tl,
        u_tl_t,
        u_tl_s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> TexLibDoc {
        embedded_texlib()
    }

    #[test]
    fn texlib_json_parses_with_expected_shape() {
        let d = doc();
        assert_eq!(d.schema, "inkwave.texlib.v1");
        assert_eq!(d.layers.len(), 46, "25 base + 21 stage layers");
        assert_eq!(d.slots.map.len(), 50);
        assert_eq!(d.slots.on_wall.len(), 50);
        assert_eq!(d.slots.on_top.len(), 50);
    }

    /// The 25 MATERIALS entries, field-by-field against the texlib.js line
    /// anchors (batch A pin: an upstream retune of any layer fails here).
    #[test]
    #[allow(clippy::type_complexity)] // flat pin table mirrors the JSON field list
    fn base_layer_meta_pins_texlib_js() {
        let d = doc();
        #[rustfmt::skip]
        let expect: [(&str, u32, f32, f32, bool, bool, bool, u32, u32, [f32; 2], f32, Option<[f32; 4]>); 25] = [
            // name, group, detail, scale, tint, mask, alpha, mode, sym, hr, ao, stair  (≡ texlib.js line)
            ("concrete",     0, 1.0,  4.0,  true,  false, false, 1, 7, [-0.004,   0.0008], 0.5,  None),                       // L213
            ("pavers",       0, 0.9,  4.0,  true,  false, false, 1, 7, [-0.008,   0.001],  0.55, None),                       // L238
            ("tiles",        0, 0.15, 2.0,  true,  false, false, 1, 7, [-0.003,   0.0006], 0.5,  None),                       // L263
            ("asphalt",      0, 0.4,  2.0,  true,  false, false, 2, 7, [-0.0014,  0.0008], 0.45, None),                       // L285
            ("boardwalk",    0, 0.3,  2.0,  false, false, false, 1, 1, [-0.015,   0.0012], 0.4,  None),                       // L303
            ("metalpanel",   0, 0.2,  2.0,  true,  false, false, 1, 3, [-0.006,   0.003],  0.45, None),                       // L349
            ("corrugated",   0, 0.2,  2.4,  true,  false, false, 0, 0, [-0.03,    0.001],  0.3,  None),                       // L379
            ("hazard",       0, 0.5,  2.0,  false, false, false, 0, 0, [-0.0004,  0.0004], 0.3,  None),                       // L400
            ("grate",        0, 0.3,  1.2,  false, false, true,  1, 3, [-0.009,   0.0006], 0.35, None),                       // L422
            ("brick",        0, 0.7,  2.0,  true,  false, false, 1, 1, [-0.007,   0.0008], 0.5,  None),                       // L449
            ("rubber",       0, 0.35, 2.0,  true,  false, false, 2, 7, [-0.0007,  0.0007], 0.35, None),                       // L477
            ("glasstile",    0, 0.0,  1.2,  true,  false, false, 1, 7, [-0.004,   0.0018], 0.45, None),                       // L490
            ("planks",       1, 0.35, 2.24, false, false, false, 1, 1, [-0.022,   0.0016], 0.4,  None),                       // L518
            ("hullpaint",    1, 0.25, 3.2,  true,  true,  false, 1, 1, [-0.004,   0.003],  0.3,  None),                       // L615
            ("nonslip",      1, 0.5,  2.4,  true,  true,  false, 1, 7, [-0.012,   0.0015], 0.4,  None),                       // L691
            ("gelcoat",      1, 0.1,  2.4,  true,  true,  false, 1, 1, [-0.0016,  0.0005], 0.3,  None),                       // L738
            ("yard",         1, 0.9,  3.2,  true,  true,  false, 1, 7, [-0.01,    0.0012], 0.5,  None),                       // L768
            ("weatherboard", 1, 0.3,  2.4,  true,  true,  false, 1, 1, [-0.016,   0.0006], 0.55, None),                       // L828
            ("render",       1, 0.8,  2.4,  true,  true,  false, 2, 7, [-0.0016,  0.0012], 0.4,  None),                       // L894
            ("treads",       2, 0.25, 1.12, true,  true,  false, 1, 1, [-0.1,     0.002],  0.1,  Some([0.28, 4.0, 0.5, 1.0])),  // L924
            ("stonestep",    2, 0.6,  1.92, true,  true,  false, 1, 1, [-0.115,   0.002],  0.1,  Some([0.32, 6.0, 0.5, 2.0])),  // L984
            ("rampboard",    2, 0.35, 1.6,  false, false, false, 1, 1, [-0.022,   0.03],   0.3,  Some([0.4,  4.0, 0.45, 3.0])), // L1041
            ("gangdeck",     2, 0.3,  1.2,  false, false, false, 1, 1, [-0.004,   0.02],   0.3,  Some([0.3,  4.0, 0.4,  4.0])), // L1097
            ("detail",       0, 0.0,  0.5,  true,  false, false, 0, 0, [-0.0003,  0.0002], 0.0,  None),                       // L1138
            ("gel",          0, 0.0,  1.0,  true,  false, false, 0, 0, [-0.006,   0.006],  0.0,  None),                       // L1154
        ];
        for (i, (name, g, det, sc, tint, mask, alpha, mode, sym, hr, ao, stair)) in
            expect.into_iter().enumerate()
        {
            let l = &d.layers[i];
            assert_eq!(
                (
                    l.name.as_str(),
                    l.group,
                    l.detail,
                    l.scale,
                    l.tint,
                    l.mask,
                    l.alpha,
                    l.mode,
                    l.sym,
                    l.hr,
                    l.ao,
                    l.stair
                ),
                (
                    name, g, det, sc, tint, mask, alpha, mode, sym, hr, ao, stair
                ),
                "layer {i} (≡ texlib.js {name})"
            );
        }
    }

    /// The stage append (texlib.js L1167 + stages/surfaces.js L14-24):
    /// names `'<stage>:<name>'`, groups 3 + registry index, in slot order.
    #[test]
    fn stage_layers_appended_in_registry_order() {
        let d = doc();
        let stage: Vec<&str> = d.layers[25..].iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            stage,
            [
                "tidewater:herringbone",
                "tidewater:terrazzo",
                "tidewater:stucco",
                "kelpline:tarmac",
                "kelpline:quay",
                "kelpline:chequer",
                "saltpan:saltcrust",
                "saltpan:drymud",
                "saltpan:saltboard",
                "crossmarket:setts",
                "crossmarket:ashlar",
                "crossmarket:tramway",
                "lockgate:setts",
                "lockgate:engbrick",
                "lockgate:hoofsteps",
                "terraces:calce",
                "terraces:cotto",
                "terraces:pebble",
                "cargo:tarmac",
                "cargo:quay",
                "cargo:chequer",
            ]
        );
        for (k, group) in (3..=9u32).enumerate() {
            for l in &d.layers[25 + k * 3..25 + k * 3 + 3] {
                assert_eq!(l.group, group, "{} group (stages/surfaces.js L23)", l.name);
            }
        }
        // spot: lockgate:setts ≡ stages/lockgate/surfaces.js L22-23.
        let s = &d.layers[25 + 12];
        assert_eq!(s.name, "lockgate:setts");
        assert_eq!(
            (
                s.detail, s.scale, s.tint, s.mask, s.alpha, s.mode, s.sym, s.hr, s.ao
            ),
            (0.6, 2.0, true, true, false, 1, 1, [-0.016, 0.006], 0.55)
        );
    }

    /// The full 50-slot `map` literal (levelMaterial.js L79-85, the stage
    /// fill L84 and the trailing TL_SIDE).
    #[test]
    fn slot_map_pins_level_material_literal() {
        let d = doc();
        let base = [
            "concrete",
            "pavers",
            "tiles",
            "concrete",
            "rubber",
            "corrugated",
            "boardwalk",
            "metalpanel",
            "tiles",
            "concrete",
            "asphalt",
            "metalpanel",
            "grate",
            "brick",
            "rubber",
            "glasstile",
            "pavers",
            "planks",
            "hullpaint",
            "nonslip",
            "gelcoat",
            "yard",
            "weatherboard",
            "render",
            "treads",
            "stonestep",
            "rampboard",
            "gangdeck",
        ];
        for (i, n) in base.iter().enumerate() {
            assert_eq!(d.slots.map[i], *n, "slot {i}");
        }
        // 28..48: the stage slots (every stage owns its three, none unused).
        for (k, n) in d.slots.map[28..49].iter().enumerate() {
            assert_eq!(n, &d.layers[25 + k].name, "stage slot {}", 28 + k);
        }
        assert_eq!(d.slots.map[49], "concrete", "TL_SIDE (L85)");
    }

    /// onWall/onTop resolution: base table (L89-90) + the STAGE_SURFACES
    /// loop (L91), identity where unset.
    #[test]
    fn on_wall_on_top_resolution() {
        let d = doc();
        let side = (d.slots.map.len() - 1) as u32; // 49
        // base (L89): ramp/asphalt/yard sides → concrete; car-deck edge →
        // hull plating; stair/ramp sides → stringer/skirting/steel.
        for (i, w) in [
            (4usize, side),
            (10, side),
            (21, side),
            (19, 18),
            (24, 18),
            (25, 23),
            (26, 17),
            (27, 11),
        ] {
            assert_eq!(d.slots.on_wall[i], w, "onWall[{i}] (L89)");
        }
        assert_eq!(
            d.slots.on_top[20], 17,
            "gelcoat hulls get a planked deck on top (L90)"
        );
        // identity on the untouched shared slots
        for i in [
            0usize, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 23,
        ] {
            assert_eq!(d.slots.on_wall[i], i as u32, "onWall[{i}] identity");
        }
        for i in 0..28usize {
            if i != 20 {
                assert_eq!(d.slots.on_top[i], i as u32, "onTop[{i}] identity");
            }
        }
        // stage loop (L91): lockgate setts → engbrick on walls (surfaces.js L21)
        assert_eq!(d.slots.on_wall[40], 41, "lockgate:setts onWall");
        assert_eq!(d.slots.on_wall[41], 41);
        assert_eq!(d.slots.on_wall[42], 41);
        // every resolved target is a valid slot
        for i in 0..50 {
            assert!((d.slots.on_wall[i] as usize) < 50);
            assert!((d.slots.on_top[i] as usize) < 50);
        }
    }

    /// uTL/uTLt/uTLs ≡ levelMaterial.js L96-99 construction, spot-pinned per
    /// construction branch (index / scale-default / mode / sym / tint 0-1-2 /
    /// grate normal strength / stair table).
    #[test]
    fn slot_uniform_tables_match_construction() {
        let d = doc();
        let u = slot_uniforms(&d);
        assert_eq!(u.u_tl.len(), 50);
        // slot 0 concrete → (0, 1/4, GRID, 7)
        assert_eq!(u.u_tl[0], [0.0, 0.25, 1.0, 7.0]);
        // slot 12 grate → layer 8, 1/1.2, GRID, sym 3
        assert_eq!(u.u_tl[12], [8.0, 1.0 / 1.2, 1.0, 3.0]);
        // slot 24 treads → layer 19, 1/1.12, GRID, sym 1
        assert_eq!(u.u_tl[24], [19.0, 1.0 / 1.12, 1.0, 1.0]);
        // slot 28 tidewater:herringbone → the first stage layer (25)
        assert_eq!(u.u_tl[28][0], 25.0);
        // slot 49 SIDE concrete → layer 0
        assert_eq!(u.u_tl[49][0], 0.0);
        // tint modes (L97): mask → 2, tint===false → 0, else 1
        assert_eq!(u.u_tl_t[6][0], 0.0, "boardwalk tint false");
        assert_eq!(u.u_tl_t[12][0], 0.0, "grate tint false");
        assert_eq!(u.u_tl_t[18][0], 2.0, "hullpaint mask");
        assert_eq!(u.u_tl_t[4][0], 1.0, "rubber plain tint");
        // grate normal strength 0.6 (L97), everything else 1.0
        assert_eq!(u.u_tl_t[12][1], 0.6);
        assert_eq!(u.u_tl_t[0][1], 1.0);
        // onWall/onTop wired through (L97 z/w)
        assert_eq!(u.u_tl_t[4][2], 49.0);
        assert_eq!(
            u.u_tl_t[20],
            [2.0, 1.0, 20.0, 17.0],
            "gelcoat: mask, top→planks (no wall override)"
        );
        assert_eq!(
            u.u_tl_t[21],
            [2.0, 1.0, 49.0, 21.0],
            "yard: mask, wall→SIDE (L89 21: SIDE)"
        );
        assert_eq!(u.u_tl_t[40], [2.0, 1.0, 41.0, 40.0], "lockgate:setts");
        // stair table (L99): treads/stonestep/rampboard/gangdeck, else zeros
        assert_eq!(u.u_tl_s[24], [0.28, 4.0, 0.5, 1.0]);
        assert_eq!(u.u_tl_s[25], [0.32, 6.0, 0.5, 2.0]);
        assert_eq!(u.u_tl_s[26], [0.4, 4.0, 0.45, 3.0]);
        assert_eq!(u.u_tl_s[27], [0.3, 4.0, 0.4, 4.0]);
        assert_eq!(u.u_tl_s[0], [0.0; 4]);
        assert_eq!(u.u_tl_s[40], [0.0; 4]);
    }

    /// inkShading.js L22 / levelMaterial.js L11/L40-41/L47-50/L62 /
    /// texlib.js L4/L1332/L185 constants.
    #[test]
    fn extracted_constants_pin_js_sources() {
        let d = doc();
        assert_eq!(d.ink.ripples, 24, "INK_RIPPLES (inkShading.js L22)");
        assert_eq!(d.ink.glow, 0.06, "uInkGlow (levelMaterial.js L50)");
        assert_eq!(d.ink.team_a, "#ff8a14");
        assert_eq!(d.ink.team_b, "#2f5bff");
        assert_eq!(d.ink.ppm, 20.0, "uPpm default (L62)");
        assert_eq!(d.lamps.max, 12);
        assert_eq!(d.lamps.radius, 10.0);
        assert_eq!(d.lamps.hex, "#ffc48a");
        assert_eq!(d.lamps.intensity, 16.0);
        assert_eq!(d.mat.roughness, 0.82);
        assert_eq!(d.mat.metalness, 0.0);
        assert_eq!(d.mat.clearcoat, 1.0);
        assert_eq!(d.mat.clearcoat_roughness, 0.08);
        assert_eq!(d.tex.size_high, 512);
        assert_eq!(d.tex.size_low, 256);
        assert_eq!(d.tex.sub, 2);
        // uGel resolves to the gel layer index (L100 `L.gel ?? -1`)
        assert_eq!(layer_index(&d, "gel"), Some(24));
    }
}
