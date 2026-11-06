//! Task 53 (D-T53-2) — the `IKP1` props archive decoder + stage manifests.
//!
//! Binary format: `rust/assets/props/props.schema.md` (little-endian, gzip).
//! Manifests ride `include_str!` (all eight stages + special stay well under
//! 1 MB) so the data contract is compile-time checked and the cross-check
//! tests can run without the `.bin.gz` files; the geometry archives load at
//! runtime (D-T53-6⑦: 22 MB gz across eight stages cannot ride the wasm
//! binary). A missing/unreadable archive degrades silently — ≡ main.js
//! L232-240 wrapping the whole prop build in a try/catch ("[props] failed").
//!
//! Parsing style follows the texlib precedent (texlib/schema.rs L15): the
//! inkwave crate carries no serde dependency, so manifests are read through
//! the sim's order-preserving JSON reader (`inkwave_sim::net::wire::JVal`).

use std::path::PathBuf;

use flate2::read::GzDecoder;
use inkwave_sim::net::wire::{JVal, parse_json};
use std::io::Read;

/// Archive magic: 'I', 'K', 'P', '1' packed LE (props.schema.md).
pub const MAGIC: u32 = 0x3150_4B49;
/// Archive version this decoder accepts.
pub const VERSION: u16 = 1;

/// One material bucket (JS `PropKit._buckets` key; props.js L3129-3141).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub name: String,
    /// The `~ns` suffix: the bucket never casts shadows (props.js L3079).
    pub ns: bool,
}

/// One baked geometry template (welded + quantised, props.schema.md).
#[derive(Debug, Clone)]
pub struct Template {
    pub bucket: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// A placement instance: template + the begin-matrix components
/// (≡ `Builder.begin(pos, rotY, scale)`, props.js L1140-1183).
#[derive(Debug, Clone, Copy)]
pub struct Instance {
    pub template: u32,
    pub bucket: u8,
    pub pos: [f32; 3],
    pub rot_y: f32,
    pub scale: f32,
    /// 0 always | 1 onlyIn turf | 2 onlyIn zones | 3 notIn turf | 4 notIn zones
    /// (variants.js L6; ≡ main.js L235 mode filter).
    pub mode_flags: u8,
}

/// A spinner record — WORLD base matrix, ≡ `PropKit._spin` entries;
/// animation `M(t) = base × rot(axis, t·speed + phase)` (props.js L3270-3275).
#[derive(Debug, Clone, Copy)]
pub struct Spin {
    pub template: u32,
    pub bucket: u8,
    /// Row-major 3×4 (translation in `base[0..3][3]`).
    pub base: [[f32; 4]; 3],
    pub speed: f32,
    pub phase: f32,
    /// 0 = +Y, 1 = +Z (fan).
    pub axis: u8,
}

/// A blink-light record — WORLD bulb, ≡ `PropKit._blink` entries; colour
/// pulse ≡ props.js L3279-3284.
#[derive(Debug, Clone, Copy)]
pub struct Blink {
    pub pos: [f32; 3],
    pub scale: f32,
    pub color: [f32; 3],
    pub rate: f32,
    pub phase: f32,
    pub lo: f32,
    pub hi: f32,
}

/// A decor instance (spawn pads / lamps / palms / flags; world-baked, the
/// position rides per-instance only for the pad bases).
#[derive(Debug, Clone, Copy)]
pub struct DecorInst {
    pub template: u32,
    pub bucket: u8,
    pub pos: [f32; 3],
    pub scale: f32,
}

/// The decoded archive.
#[derive(Debug, Clone, Default)]
pub struct Archive {
    pub buckets: Vec<Bucket>,
    pub templates: Vec<Template>,
    pub instances: Vec<Instance>,
    pub spins: Vec<Spin>,
    pub blinks: Vec<Blink>,
    pub decor: Vec<DecorInst>,
}

/// Bounds a corrupt header cannot push past (per-element sanity caps).
const MAX_VERTS_PER_TEMPLATE: u32 = 4_000_000;
const MAX_IDX_PER_TEMPLATE: u32 = 12_000_000;
const MAX_RECORDS: u32 = 200_000;

/// Little-endian cursor with checked reads (never panics on bad input).
struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.i.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        let s = &self.b[self.i..end];
        self.i = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    /// Vertex position: i32 millimetres.
    fn mm(&mut self) -> Option<f32> {
        Some(self.i32()? as f32 / 1000.0)
    }
    /// Vertex normal: i16 × 1000.
    fn nor(&mut self) -> Option<f32> {
        Some(self.i16()? as f32 / 1000.0)
    }
    /// Vertex colour channel: u8 → linear 0..1.
    fn col8(&mut self) -> Option<f32> {
        Some(self.u8()? as f32 / 255.0)
    }
}

/// Decode an `IKP1` archive (props.schema.md); `None` on any malformed byte.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Archive> {
    let mut r = Rd::new(bytes);
    if r.u32()? != MAGIC {
        return None;
    }
    if r.u16()? != VERSION {
        return None;
    }
    let bucket_count = r.u16()? as usize;
    let mut buckets = Vec::with_capacity(bucket_count);
    for _ in 0..bucket_count {
        let len = r.u8()? as usize;
        let name = String::from_utf8(r.take(len)?.to_vec()).ok()?;
        let ns = r.u8()? != 0;
        buckets.push(Bucket { name, ns });
    }

    let template_count = r.u32()?;
    if template_count > MAX_RECORDS {
        return None;
    }
    let mut templates = Vec::with_capacity(template_count as usize);
    for _ in 0..template_count {
        let vc = r.u32()?;
        let ic = r.u32()?;
        if vc > MAX_VERTS_PER_TEMPLATE || ic > MAX_IDX_PER_TEMPLATE {
            return None;
        }
        let mut positions = Vec::with_capacity(vc as usize);
        let mut normals = Vec::with_capacity(vc as usize);
        let mut colors = Vec::with_capacity(vc as usize);
        for _ in 0..vc {
            let p = [r.mm()?, r.mm()?, r.mm()?];
            let n = [r.nor()?, r.nor()?, r.nor()?];
            let c = [r.col8()?, r.col8()?, r.col8()?, 1.0];
            positions.push(p);
            normals.push(n);
            colors.push(c);
        }
        let mut indices = Vec::with_capacity(ic as usize);
        for _ in 0..ic {
            let j = r.u32()?;
            if j >= vc {
                return None;
            }
            indices.push(j);
        }
        // The bucket is implied by every instance that references the
        // template (templates are bucket-scoped by construction); recover it
        // from the first referencing record after the tables are read.
        templates.push(Template {
            bucket: u8::MAX,
            positions,
            normals,
            colors,
            indices,
        });
    }

    let inst_count = r.u32()?;
    if inst_count > MAX_RECORDS {
        return None;
    }
    let mut instances = Vec::with_capacity(inst_count as usize);
    for _ in 0..inst_count {
        let template = r.u32()?;
        let bucket = r.u8()?;
        let pos = [r.mm()?, r.mm()?, r.mm()?];
        let rot_y = r.f32()?;
        let scale = r.f32()?;
        let mode_flags = r.u8()?;
        if template >= templates.len() as u32 || bucket as usize >= buckets.len() {
            return None;
        }
        instances.push(Instance {
            template,
            bucket,
            pos,
            rot_y,
            scale,
            mode_flags,
        });
    }

    let spin_count = r.u32()?;
    if spin_count > MAX_RECORDS {
        return None;
    }
    let mut spins = Vec::with_capacity(spin_count as usize);
    for _ in 0..spin_count {
        let template = r.u32()?;
        let bucket = r.u8()?;
        let mut base = [[0.0f32; 4]; 3];
        for row in base.iter_mut() {
            for v in row.iter_mut() {
                *v = r.f32()?;
            }
        }
        let speed = r.f32()?;
        let phase = r.f32()?;
        let axis = r.u8()?;
        if template >= templates.len() as u32 || bucket as usize >= buckets.len() || axis > 1 {
            return None;
        }
        spins.push(Spin {
            template,
            bucket,
            base,
            speed,
            phase,
            axis,
        });
    }

    let blink_count = r.u32()?;
    if blink_count > MAX_RECORDS {
        return None;
    }
    let mut blinks = Vec::with_capacity(blink_count as usize);
    for _ in 0..blink_count {
        let pos = [r.mm()?, r.mm()?, r.mm()?];
        let scale = r.f32()?;
        let color = [r.col8()?, r.col8()?, r.col8()?];
        let rate = r.f32()?;
        let phase = r.f32()?;
        let lo = r.f32()?;
        let hi = r.f32()?;
        blinks.push(Blink {
            pos,
            scale,
            color,
            rate,
            phase,
            lo,
            hi,
        });
    }

    let decor_count = r.u32()?;
    if decor_count > MAX_RECORDS {
        return None;
    }
    let mut decor = Vec::with_capacity(decor_count as usize);
    for _ in 0..decor_count {
        let template = r.u32()?;
        let bucket = r.u8()?;
        let pos = [r.mm()?, r.mm()?, r.mm()?];
        let scale = r.f32()?;
        if template >= templates.len() as u32 || bucket as usize >= buckets.len() {
            return None;
        }
        decor.push(DecorInst {
            template,
            bucket,
            pos,
            scale,
        });
    }

    if r.i != bytes.len() {
        return None; // trailing garbage ⇒ corrupt archive
    }
    // Recover each template's bucket from its first referencing record
    // (instances, then spins, then decor — every template is referenced).
    for t in &mut templates {
        t.bucket = u8::MAX;
    }
    for rec in instances
        .iter()
        .map(|i| (i.template, i.bucket))
        .chain(spins.iter().map(|s| (s.template, s.bucket)))
        .chain(decor.iter().map(|d| (d.template, d.bucket)))
    {
        let t = &mut templates[rec.0 as usize];
        if t.bucket == u8::MAX {
            t.bucket = rec.1;
        } else if t.bucket != rec.1 {
            return None; // a template shared across buckets would break the contract
        }
    }
    Some(Archive {
        buckets,
        templates,
        instances,
        spins,
        blinks,
        decor,
    })
}

/// Gunzip + decode a `.bin.gz` payload.
#[must_use]
pub fn decode_gz(gz: &[u8]) -> Option<Archive> {
    let mut raw = Vec::new();
    GzDecoder::new(gz).read_to_end(&mut raw).ok()?;
    decode(&raw)
}

/// The runtime archive directory: `INKWAVE_ASSETS` (env, points at
/// `rust/assets`) or the in-repo default resolved from the crate manifest.
/// wasm has no filesystem — the lobby wiring (Task 57) fetches the same
/// `assets/props/*.gz` through the trunk `copy-dir` route there.
#[must_use]
pub fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("INKWAVE_ASSETS") {
        return PathBuf::from(dir).join("props");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/props")
}

/// Load + decode `<stage>.bin.gz`; `None` when the file is absent (silent
/// degradation ≡ the JS try/catch, D-T53-6⑦).
#[must_use]
pub fn load_stage_archive(stage: &str) -> Option<Archive> {
    let path = assets_dir().join(format!("{stage}.bin.gz"));
    let gz = std::fs::read(&path).ok()?;
    decode_gz(&gz)
}

// ------------------------------------------------------------------ manifests

/// The eight extractable stage ids (maps.js L121 order).
#[allow(dead_code)] // golden/round-trip test driver; the Task 57 lobby picker iterates it for real
pub const STAGE_IDS: [&str; 8] = [
    "tidewater",
    "kelpline",
    "halyard",
    "saltpan",
    "crossmarket",
    "lockgate",
    "terraces",
    "cargo",
];

const MANIFEST_TIDEWATER: &str = include_str!("../../../../assets/props/tidewater.json");
const MANIFEST_KELPLINE: &str = include_str!("../../../../assets/props/kelpline.json");
const MANIFEST_HALYARD: &str = include_str!("../../../../assets/props/halyard.json");
const MANIFEST_SALTPAN: &str = include_str!("../../../../assets/props/saltpan.json");
const MANIFEST_CROSSMARKET: &str = include_str!("../../../../assets/props/crossmarket.json");
const MANIFEST_LOCKGATE: &str = include_str!("../../../../assets/props/lockgate.json");
const MANIFEST_TERRACES: &str = include_str!("../../../../assets/props/terraces.json");
const MANIFEST_CARGO: &str = include_str!("../../../../assets/props/cargo.json");
/// The special-prop manifest (D-T53-4; consumers = Task 54/57).
#[allow(dead_code)] // read through `special::parse_special_manifest` in tests until Task 54
pub const SPECIAL_MANIFEST: &str = include_str!("../../../../assets/props/special.json");

/// The embedded manifest JSON for one stage id.
#[must_use]
pub fn embedded_manifest(stage: &str) -> Option<&'static str> {
    Some(match stage {
        "tidewater" => MANIFEST_TIDEWATER,
        "kelpline" => MANIFEST_KELPLINE,
        "halyard" => MANIFEST_HALYARD,
        "saltpan" => MANIFEST_SALTPAN,
        "crossmarket" => MANIFEST_CROSSMARKET,
        "lockgate" => MANIFEST_LOCKGATE,
        "terraces" => MANIFEST_TERRACES,
        "cargo" => MANIFEST_CARGO,
        _ => return None,
    })
}

/// `meta` block (props.schema.md).
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub stage: String,
    pub placements: u64,
    pub templates: u64,
    pub verts: u64,
    pub tris: u64,
    pub bin: String,
    pub bin_bytes: u64,
    pub raw_bytes: u64,
    pub bin_sha256: String,
    pub buckets: Vec<Bucket>,
}

/// One placement row (manifest `placements[]`) — the cross-check anchor set.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementRow {
    pub i: u64,
    pub type_: String,
    pub pos: [f64; 3],
    pub rot_y: f64,
    pub scale: f64,
    pub mode_flags: u8,
    pub inst_start: u64,
    pub inst_count: u64,
    pub tris: u64,
    pub bbox: [[f64; 3]; 2],
}

/// `counts` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub spin: u64,
    pub blink: u64,
    pub flags: u64,
    pub banners: u64,
    pub decor_meshes: u64,
}

/// One mural placement row (`murals[]`).
#[derive(Debug, Clone, PartialEq)]
pub struct MuralRow {
    pub id: u64,
    pub rect: [f64; 4],
    pub place: [f64; 4],
    pub fx: [f64; 2],
}

/// A parsed stage manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub meta: Meta,
    pub placements: Vec<PlacementRow>,
    pub lamps: Vec<[f32; 3]>,
    pub murals: Vec<MuralRow>,
    pub counts: Counts,
}

fn arr3(v: &JVal) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}
fn arr4(v: &JVal) -> Option<[f64; 4]> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    Some([
        a[0].as_f64()?,
        a[1].as_f64()?,
        a[2].as_f64()?,
        a[3].as_f64()?,
    ])
}
fn arr2(v: &JVal) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    if a.len() != 2 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?])
}
fn num(v: Option<&JVal>) -> Option<f64> {
    v?.as_f64()
}
fn uint(v: Option<&JVal>) -> Option<u64> {
    num(v).map(|x| x as u64)
}

/// Parse one stage manifest; `None` on any shape mismatch.
#[must_use]
pub fn parse_manifest(json: &str) -> Option<Manifest> {
    let root = parse_json(json)?;
    let meta_v = root.field("meta")?;
    let buckets = meta_v
        .field("buckets")?
        .as_array()?
        .iter()
        .map(|b| {
            Some(Bucket {
                name: b.field("name")?.as_str()?.to_string(),
                ns: b.field("ns")?.as_f64()? != 0.0,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let meta = Meta {
        stage: meta_v.field("stage")?.as_str()?.to_string(),
        placements: uint(meta_v.field("placements"))?,
        templates: uint(meta_v.field("templates"))?,
        verts: uint(meta_v.field("verts"))?,
        tris: uint(meta_v.field("tris"))?,
        bin: meta_v.field("bin")?.as_str()?.to_string(),
        bin_bytes: uint(meta_v.field("binBytes"))?,
        raw_bytes: uint(meta_v.field("rawBytes"))?,
        bin_sha256: meta_v.field("binSha256")?.as_str()?.to_string(),
        buckets,
    };
    let placements = root
        .field("placements")?
        .as_array()?
        .iter()
        .map(|p| {
            Some(PlacementRow {
                i: uint(p.field("i"))?,
                type_: p.field("type")?.as_str()?.to_string(),
                pos: arr3(p.field("pos")?)?,
                rot_y: num(p.field("rotY"))?,
                scale: num(p.field("scale"))?,
                mode_flags: num(p.field("modeFlags"))? as u8,
                inst_start: uint(p.field("instStart"))?,
                inst_count: uint(p.field("instCount"))?,
                tris: uint(p.field("tris"))?,
                bbox: {
                    let bb = p.field("bbox")?.as_array()?;
                    [arr3(&bb[0])?, arr3(&bb[1])?]
                },
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let lamps = root
        .field("lamps")?
        .as_array()?
        .iter()
        .map(|l| {
            let [x, y, z] = arr3(l)?;
            Some([x as f32, y as f32, z as f32])
        })
        .collect::<Option<Vec<_>>>()?;
    let murals = root
        .field("murals")?
        .as_array()?
        .iter()
        .map(|m| {
            Some(MuralRow {
                id: uint(m.field("id"))?,
                rect: arr4(m.field("rect")?)?,
                place: arr4(m.field("place")?)?,
                fx: arr2(m.field("fx")?)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let c = root.field("counts")?;
    let counts = Counts {
        spin: uint(c.field("spin"))?,
        blink: uint(c.field("blink"))?,
        flags: uint(c.field("flags"))?,
        banners: uint(c.field("banners"))?,
        decor_meshes: uint(c.field("decorMeshes"))?,
    };
    Some(Manifest {
        meta,
        placements,
        lamps,
        murals,
        counts,
    })
}

/// The embedded + parsed manifest for one stage (panic-free: `None` on a
/// malformed embedded document, which the decode tests assert against).
#[must_use]
pub fn stage_manifest(stage: &str) -> Option<Manifest> {
    parse_manifest(embedded_manifest(stage)?)
}

/// Does a `modeFlags` record belong to `mode`? ≡ variants.js L6 expanded to
/// the 0..4 vocabulary (props.schema.md).
#[must_use]
pub fn mode_passes(mode_flags: u8, mode: &str) -> bool {
    match mode_flags {
        0 => true,
        1 => mode == "turf",
        2 => mode == "zones",
        3 => mode != "turf",
        4 => mode != "zones",
        _ => false,
    }
}
