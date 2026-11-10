//! Task 54 batch C1 (D-T54-3) — the `IKC1` character archive decoder, the
//! `char.json` manifest, the material-bucket table and the rig entity tree.
//!
//! Binary format authority: `rust/assets/char/char.schema.md` (magic `IKC1`,
//! pos i32 mm / nor i16 ×1000 / col u8, welded on the 1e-3 grid). The decoder
//! mirrors the props precedent line-for-line (`≡ props/archive.rs L###`):
//! same cursor, same sanity caps, same silent-degradation loader, same
//! gzip handling — the inkwave crate carries no serde, so the manifest rides
//! `include_str!` and is read through the sim's order-preserving JSON reader
//! (`inkwave_sim::net::wire::JVal`, ≡ props/archive.rs L11-13).
//!
//! Scope: decode + manifest + buckets + `spawn_char_rig` (bone parent-child
//! tree, tier-filtered part meshes, weapon frame nodes). The pose→Transform
//! bridge, per-character material instances and the actors.rs swap landed in
//! batch C2 (D-T54-3) and are wired from `world.rs`; the archive is baked into
//! both native and wasm builds so no runtime filesystem is needed.
//!
//! Known deviation (pinned by `twins_far_mirror_is_a_negative_determinant`):
//! the two `weapon:twins:far*:@L` part matrices carry det = −1 (the JS mirror
//! `pivotL`); Bevy `Transform` cannot represent mirroring, so those entities
//! get the decomposed (un-mirrored) transform plus the raw matrix in
//! [`CharPartMatrix`] for C2 to resolve.
#![allow(dead_code)] // Task 54 C1 scaffold: the whole rig API is wired in batch C2 (D-T54-3)

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use flate2::read::GzDecoder;
use inkwave_sim::net::wire::{JVal, parse_json};

// ---------------------------------------------------------------- IKC1 decode

/// Archive magic: 'I', 'K', 'C', '1' packed LE (≡ char.schema.md L17).
pub const MAGIC: u32 = 0x3143_4B49;
/// Archive version this decoder accepts (≡ char.schema.md L18).
pub const VERSION: u32 = 1;

/// Bounds a corrupt header cannot push past (≡ props/archive.rs L107-109).
const MAX_VERTS_PER_TEMPLATE: u32 = 4_000_000;
const MAX_IDX_PER_TEMPLATE: u32 = 12_000_000;
const MAX_RECORDS: u32 = 200_000;

/// Little-endian cursor with checked reads (≡ props/archive.rs L111-160;
/// never panics on bad input).
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
    /// Vertex position: i32 millimetres (≡ char.schema.md L24).
    fn mm(&mut self) -> Option<f32> {
        Some(self.i32()? as f32 / 1000.0)
    }
    /// Vertex normal: i16 × 1000 (≡ char.schema.md L25).
    fn nor(&mut self) -> Option<f32> {
        Some(self.i16()? as f32 / 1000.0)
    }
    /// Vertex colour channel: u8 → linear 0..1 (≡ char.schema.md L26).
    fn col8(&mut self) -> Option<f32> {
        Some(self.u8()? as f32 / 255.0)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
}

/// One baked geometry template, welded + quantised (≡ char.schema.md L21-29;
/// field shape ≡ props/archive.rs `Template` L35-42 minus the bucket — the
/// char archive carries no bucket table, materials ride the manifest
/// `parts[].matClass`).
#[derive(Debug, Clone)]
pub struct CharTemplate {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// One part instance: template + bone mount + part id + the bone-local
/// rest matrix (≡ char.schema.md L30-36: `u32 templateId, u16 boneId,
/// u32 partId, f32 m[12]` packed LE, row-major 3×4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharInstance {
    pub template: u32,
    /// Index into `rig.boneNames ∪ rig.pseudoBones` (≡ char.schema.md L32,
    /// L45-51: 0..87 real bones, then `@kid`, `@model`).
    pub bone: u16,
    /// Index into the manifest `parts[].id` (sorted part keys).
    pub part: u32,
    /// Bone-local 3×4 row-major: [r0 r1 r2 t0 / r3 r4 r5 t1 / r6 r7 r8 t2].
    pub matrix: [f32; 12],
}

/// The decoded character archive.
#[derive(Debug, Clone, Default)]
pub struct CharArchive {
    pub templates: Vec<CharTemplate>,
    pub instances: Vec<CharInstance>,
}

/// Decode an `IKC1` archive (≡ char.schema.md L16-36); `None` on any
/// malformed byte (checked-cursor style ≡ props/archive.rs L162-352).
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<CharArchive> {
    let mut r = Rd::new(bytes);
    if r.u32()? != MAGIC {
        return None;
    }
    if r.u32()? != VERSION {
        return None;
    }
    let template_count = r.u32()?;
    let instance_count = r.u32()?;
    if template_count > MAX_RECORDS || instance_count > MAX_RECORDS {
        return None;
    }

    let mut templates = Vec::with_capacity(template_count as usize);
    for _ in 0..template_count {
        let vc = r.u32()?;
        let ic = r.u32()?;
        let tri = r.u32()?; // ≡ char.schema.md L22: triCount = indexCount/3
        if vc > MAX_VERTS_PER_TEMPLATE || ic > MAX_IDX_PER_TEMPLATE || ic % 3 != 0 || tri != ic / 3
        {
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
        templates.push(CharTemplate {
            positions,
            normals,
            colors,
            indices,
        });
    }

    let mut instances = Vec::with_capacity(instance_count as usize);
    for _ in 0..instance_count {
        let template = r.u32()?;
        let bone = r.u16()?;
        let part = r.u32()?;
        if template >= templates.len() as u32 || part >= MAX_RECORDS {
            return None;
        }
        let mut matrix = [0.0f32; 12];
        for v in matrix.iter_mut() {
            *v = r.f32()?;
        }
        instances.push(CharInstance {
            template,
            bone,
            part,
            matrix,
        });
    }

    if r.i != bytes.len() {
        return None; // trailing garbage ⇒ corrupt archive (≡ props L323-325)
    }
    Some(CharArchive {
        templates,
        instances,
    })
}

/// Gunzip + decode a `.bin.gz` payload (≡ props/archive.rs L354-360).
#[must_use]
pub fn decode_gz(gz: &[u8]) -> Option<CharArchive> {
    let mut raw = Vec::new();
    GzDecoder::new(gz).read_to_end(&mut raw).ok()?;
    decode(&raw)
}

/// The runtime archive directory: `INKWAVE_ASSETS` (env, points at
/// `rust/assets`) or the in-repo default resolved from the crate manifest
/// (≡ props/archive.rs L362-372, `char` instead of `props`).
#[must_use]
pub fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("INKWAVE_ASSETS") {
        return PathBuf::from(dir).join("char");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/char")
}

/// Load + decode `char.bin.gz`.
///
/// Native honours `INKWAVE_ASSETS` (an explicit asset root for the
/// extract/verify tools). Every other build — including **wasm32**, which has
/// neither a filesystem nor a Trunk fetch hook — decodes the 4.6 MB archive
/// baked in at compile time (D-T54-5⑨, review P0-1). Embedding is what keeps
/// the character rig present in the browser build: reading from disk there
/// returned `None`, and both `actors.rs` and `bridge.rs` then spawned zero
/// actors. Props stay on the Task 57 fetch path (22 MB, too large to bake).
#[must_use]
pub fn load_archive() -> Option<CharArchive> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(dir) = std::env::var("INKWAVE_ASSETS") {
        let gz = std::fs::read(PathBuf::from(dir).join("char/char.bin.gz")).ok()?;
        return decode_gz(&gz);
    }
    const GZ: &[u8] = include_bytes!("../../../../assets/char/char.bin.gz");
    decode_gz(GZ)
}

// ------------------------------------------------------------------ manifest

/// The embedded manifest (≡ props/archive.rs L398-408: manifests ride
/// `include_str!`, the cross-check tests need no `.bin.gz` on disk).
const MANIFEST: &str = include_str!("../../../../assets/char/char.json");

/// `meta.counts` (≡ char.schema.md L79-82).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharCounts {
    pub templates: u64,
    pub verts: u64,
    pub tris: u64,
    pub instances: u64,
    pub parts: u64,
    pub bones: u64,
    pub weapon_kinds: u64,
    pub tiers: u64,
    pub hair_styles: u64,
}

/// `meta` block subset (≡ char.schema.md L77-86).
#[derive(Debug, Clone, PartialEq)]
pub struct CharMeta {
    pub bin: String,
    pub bin_bytes: u64,
    pub raw_bytes: u64,
    pub bin_sha256: String,
    pub counts: CharCounts,
}

/// One `parts[]` row (≡ char.schema.md L88: `{key,id,tier,bone,matClass,
/// shadow,template,verts,tris,matrix,extra}`; `extra` is C2 scope — the bomb
/// grip/inHandL frames and tank fill constants).
#[derive(Debug, Clone, PartialEq)]
pub struct PartRow {
    pub key: String,
    pub id: u32,
    /// `hero|game|far` for kid rows; `near|far` for weapon/bomb rows
    /// (weapon slots carry the tier in the key, char.schema.md L58-71).
    pub tier: String,
    pub bone: String,
    /// One of the 12 classes ≡ char.schema.md L73-74.
    pub mat_class: String,
    pub shadow: bool,
    pub template: u32,
    pub verts: u64,
    pub tris: u64,
    /// Bone-local 3×4 row-major, same layout as the binary (f64 in JSON).
    pub matrix: [f64; 12],
}

/// `rig` block subset the tree needs (≡ char.schema.md L83-87; `rest` /
/// `boneInverses` / `hairRest` consumers already live in `tables::Rig`).
#[derive(Debug, Clone, PartialEq)]
pub struct RigTable {
    pub bone_names: Vec<String>,
    /// Parent index per bone, ordered by `bone_names`; `None` = hangs off
    /// the kid node (≡ tables.rs L353).
    pub bone_parent: Vec<Option<usize>>,
    pub bone_index: HashMap<String, usize>,
    /// `["@kid", "@model"]` appended after the real bones (schema L45-51).
    pub pseudo_bones: Vec<String>,
    /// Rest position per bone (kid space), ordered by `bone_names`.
    pub rest: Vec<[f64; 3]>,
    pub aim_pivot: [f64; 3],
    pub fist_offset: [f64; 3],
    pub grip_hole_l: [f64; 3],
    pub hair_max: usize,
    pub hair_segs: usize,
}

/// A pos+quat frame (≡ char.schema.md L90-93: `off`/`inHand`/`handR`/…).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub pos: [f64; 3],
    pub quat: [f64; 4],
}

/// A grip spec `{pos,handZ,handY}` (≡ char.schema.md L91).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripSpec {
    pub pos: [f64; 3],
    pub hand_z: [f64; 3],
    pub hand_y: [f64; 3],
}

/// A weapon `frames` block, exposed as-is for the C2 pose bridge
/// (≡ char.schema.md L89-95; optional members per kind — `drumAt`/`drumR`
/// roller, `spinAt` spinner/zooka, `left`+`inHandL` dualies/mitts).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponFrames {
    /// FIST_OFFSET / GRIP_HOLE_L bone-local pos.
    pub pivot: [f64; 3],
    /// pos+quat = `def.inHand` (≡ schema L90).
    pub off: Frame,
    pub in_hand: Frame,
    pub in_hand_l: Option<Frame>,
    pub hand_r: Frame,
    pub hand_l: Frame,
    pub grip_r: GripSpec,
    pub grip_l: GripSpec,
    pub muzzle: [f64; 3],
    pub twirl: [f64; 3],
    pub drum_at: Option<[f64; 3]>,
    pub spin_at: Option<[f64; 3]>,
    pub drum_r: Option<f64>,
    pub draw_len: Option<f64>,
    pub dual: bool,
    pub mirror_dual: bool,
    /// `{pivot, off}` for the dual left instance (≡ schema L93); `pivot` is
    /// a position-only frame (GRIP_HOLE_L bone-local).
    pub left: Option<([f64; 3], Frame)>,
}

/// A weapon part lamp spec (≡ char.schema.md L94:
/// `lamp{color,emissive,intensity,roughness}` — the only colour constants the
/// manifest carries, so the material table reads them here, never hand-copies).
#[derive(Debug, Clone, PartialEq)]
pub struct LampSpec {
    pub color: String,
    pub emissive: String,
    pub intensity: f64,
    pub roughness: Option<f64>,
}

/// `weapons.<kind>.parts.<name>` (≡ char.schema.md L94:
/// `{pivot,mat,lamp}`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPartSpec {
    pub pivot: [f64; 3],
    pub mat: String,
    pub lamp: Option<LampSpec>,
}

/// `weapons.<kind>` (≡ char.schema.md L89-96).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponDef {
    pub frames: WeaponFrames,
    /// Part-key references (`weapon:<kind>:<slot>[@L]`).
    pub instances: Vec<String>,
    /// Named sub-parts in JSON order.
    pub parts: Vec<(String, WeaponPartSpec)>,
}

/// The parsed `char.json` manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct CharManifest {
    pub meta: CharMeta,
    pub rig: RigTable,
    pub parts: Vec<PartRow>,
    /// BTreeMap for deterministic iteration (JS object insertion order is
    /// already sorted here; BTreeMap pins it regardless).
    pub weapons: BTreeMap<String, WeaponDef>,
    /// `weaponKindList.kinds` (11 shooter family + kit, ≡ schema L97).
    pub weapon_kinds: Vec<String>,
    /// `weaponKindList.specialWeaponKinds` (4, ≡ schema L97).
    pub special_weapon_kinds: Vec<String>,
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

fn arr12(v: &JVal) -> Option<[f64; 12]> {
    let a = v.as_array()?;
    if a.len() != 12 {
        return None;
    }
    let mut out = [0.0f64; 12];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64()?;
    }
    Some(out)
}

fn num(v: Option<&JVal>) -> Option<f64> {
    v?.as_f64()
}

fn uint(v: Option<&JVal>) -> Option<u64> {
    num(v).map(|x| x as u64)
}

fn frame(v: &JVal) -> Option<Frame> {
    Some(Frame {
        pos: arr3(v.field("pos")?)?,
        quat: arr4(v.field("quat")?)?,
    })
}

fn grip(v: &JVal) -> Option<GripSpec> {
    Some(GripSpec {
        pos: arr3(v.field("pos")?)?,
        hand_z: arr3(v.field("handZ")?)?,
        hand_y: arr3(v.field("handY")?)?,
    })
}

fn lamp(v: &JVal) -> Option<LampSpec> {
    Some(LampSpec {
        color: v.field("color")?.as_str()?.to_string(),
        emissive: v.field("emissive")?.as_str()?.to_string(),
        intensity: num(v.field("intensity"))?,
        roughness: num(v.field("roughness")),
    })
}

fn weapon_frames(fr: &JVal) -> Option<WeaponFrames> {
    let left = fr
        .field("left")
        .and_then(|l| Some((arr3(l.field("pivot")?)?, frame(l.field("off")?)?)));
    Some(WeaponFrames {
        pivot: arr3(fr.field("pivot")?)?,
        off: frame(fr.field("off")?)?,
        in_hand: frame(fr.field("inHand")?)?,
        in_hand_l: fr.field("inHandL").and_then(frame),
        hand_r: frame(fr.field("handR")?)?,
        hand_l: frame(fr.field("handL")?)?,
        grip_r: grip(fr.field("gripR")?)?,
        grip_l: grip(fr.field("gripL")?)?,
        muzzle: arr3(fr.field("muzzle")?)?,
        twirl: arr3(fr.field("twirl")?)?,
        drum_at: fr.field("drumAt").and_then(arr3),
        spin_at: fr.field("spinAt").and_then(arr3),
        drum_r: num(fr.field("drumR")),
        draw_len: num(fr.field("drawLen")),
        dual: fr.field("dual").and_then(JVal::as_bool).unwrap_or(false),
        mirror_dual: fr
            .field("mirrorDual")
            .and_then(JVal::as_bool)
            .unwrap_or(false),
        left,
    })
}

/// Parse the `char.json` manifest; `None` on any shape mismatch
/// (≡ props/archive.rs `parse_manifest` L519-605 style).
#[must_use]
pub fn parse_manifest(json: &str) -> Option<CharManifest> {
    let root = parse_json(json)?;
    let meta_v = root.field("meta")?;
    let c = meta_v.field("counts")?;
    let meta = CharMeta {
        bin: meta_v.field("bin")?.as_str()?.to_string(),
        bin_bytes: uint(meta_v.field("binBytes"))?,
        raw_bytes: uint(meta_v.field("rawBytes"))?,
        bin_sha256: meta_v.field("binSha256")?.as_str()?.to_string(),
        counts: CharCounts {
            templates: uint(c.field("templates"))?,
            verts: uint(c.field("verts"))?,
            tris: uint(c.field("tris"))?,
            instances: uint(c.field("instances"))?,
            parts: uint(c.field("parts"))?,
            bones: uint(c.field("bones"))?,
            weapon_kinds: uint(c.field("weaponKinds"))?,
            tiers: uint(c.field("tiers"))?,
            hair_styles: uint(c.field("hairStyles"))?,
        },
    };

    let rig_v = root.field("rig")?;
    let bone_names: Vec<String> = rig_v
        .field("boneNames")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    let mut bone_index = HashMap::with_capacity(bone_names.len());
    for (i, n) in bone_names.iter().enumerate() {
        bone_index.insert(n.clone(), i);
    }
    // Cross-check the manifest's own `boneIndex` (≡ char.schema.md L83).
    let JVal::Obj(bi_pairs) = rig_v.field("boneIndex")? else {
        return None;
    };
    for (name, idx) in bi_pairs {
        if bone_index.get(name).copied() != idx.as_f64().map(|x| x as usize) {
            return None;
        }
    }
    let JVal::Obj(bp_pairs) = rig_v.field("boneParent")? else {
        return None;
    };
    let mut bone_parent: Vec<Option<usize>> = vec![None; bone_names.len()];
    for (name, parent) in bp_pairs {
        let i = *bone_index.get(name)?;
        match parent {
            JVal::Null => {}
            JVal::Str(p) => {
                bone_parent[i] = Some(*bone_index.get(p)?);
            }
            _ => return None,
        }
    }
    let JVal::Obj(rest_pairs) = rig_v.field("rest")? else {
        return None;
    };
    let mut rest = vec![[0.0f64; 3]; bone_names.len()];
    for (name, pos) in rest_pairs {
        let i = *bone_index.get(name)?;
        rest[i] = arr3(pos)?;
    }
    let rig = RigTable {
        bone_names,
        bone_parent,
        bone_index,
        pseudo_bones: rig_v
            .field("pseudoBones")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()?,
        rest,
        aim_pivot: arr3(rig_v.field("aimPivot")?)?,
        fist_offset: arr3(rig_v.field("fistOffset")?)?,
        grip_hole_l: arr3(rig_v.field("gripHoleL")?)?,
        hair_max: uint(rig_v.field("hairMax"))? as usize,
        hair_segs: uint(rig_v.field("hairSegs"))? as usize,
    };

    let parts: Vec<PartRow> = root
        .field("parts")?
        .as_array()?
        .iter()
        .map(|p| {
            Some(PartRow {
                key: p.field("key")?.as_str()?.to_string(),
                id: uint(p.field("id"))? as u32,
                tier: p.field("tier")?.as_str()?.to_string(),
                bone: p.field("bone")?.as_str()?.to_string(),
                mat_class: p.field("matClass")?.as_str()?.to_string(),
                shadow: p.field("shadow")?.as_bool()?,
                template: uint(p.field("template"))? as u32,
                verts: uint(p.field("verts"))?,
                tris: uint(p.field("tris"))?,
                matrix: arr12(p.field("matrix")?)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let weapons_v = root.field("weapons")?;
    let JVal::Obj(weapon_rows) = weapons_v else {
        return None;
    };
    let mut weapons = BTreeMap::new();
    for (kind, w) in weapon_rows {
        let fr = weapon_frames(w.field("frames")?)?;
        let instances: Vec<String> = w
            .field("instances")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()?;
        let mut parts_vec = Vec::new();
        if let Some(JVal::Obj(pp)) = w.field("parts") {
            for (name, pv) in pp {
                parts_vec.push((
                    name.clone(),
                    WeaponPartSpec {
                        pivot: arr3(pv.field("pivot")?)?,
                        mat: pv.field("mat")?.as_str()?.to_string(),
                        lamp: match pv.field("lamp") {
                            Some(l) if !matches!(l, JVal::Null) => Some(lamp(l)?),
                            _ => None,
                        },
                    },
                ));
            }
        }
        weapons.insert(
            kind.clone(),
            WeaponDef {
                frames: fr,
                instances,
                parts: parts_vec,
            },
        );
    }

    let wkl = root.field("weaponKindList")?;
    let str_list = |v: Option<&JVal>| -> Option<Vec<String>> {
        let arr = v?.as_array()?;
        arr.iter().map(|x| x.as_str().map(str::to_string)).collect()
    };

    Some(CharManifest {
        meta,
        rig,
        parts,
        weapons,
        weapon_kinds: str_list(wkl.field("kinds"))?,
        special_weapon_kinds: str_list(wkl.field("specialWeaponKinds"))?,
    })
}

/// The embedded + parsed manifest (`None` on a malformed embedded document,
/// which the decode tests assert against ≡ props/archive.rs L607-612).
#[must_use]
pub fn manifest() -> Option<CharManifest> {
    parse_manifest(MANIFEST)
}

// -------------------------------------------------------------- material buckets

/// The StandardMaterial buckets the 12 `matClass` values fold into
/// (≡ char.schema.md L73-74; the task's six character buckets plus the four
/// weapon/tank classes — `dark` folds into [`MatBucket::Squid`] and `lamp`
/// into [`MatBucket::Glow`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatBucket {
    Skin,
    Cloth,
    Hair,
    Eye,
    Squid,
    Glow,
    Plastic,
    Ink,
    Glass,
    Fill,
}

/// `matClass` → bucket (≡ char.schema.md L73-74); `None` on an unknown class.
#[must_use]
pub fn mat_bucket(cls: &str) -> Option<MatBucket> {
    Some(match cls {
        "skin" => MatBucket::Skin,
        "cloth" => MatBucket::Cloth,
        "hair" => MatBucket::Hair,
        "eye" => MatBucket::Eye,
        "squid" | "dark" => MatBucket::Squid,
        "glow" | "lamp" => MatBucket::Glow,
        "plastic" => MatBucket::Plastic,
        "ink" => MatBucket::Ink,
        "glass" => MatBucket::Glass,
        "fill" => MatBucket::Fill,
        _ => return None,
    })
}

/// Bucket → StandardMaterial approximation (D-T54-5①: the withRim / clearcoat
/// / pattern / uHurt shader face is NOT ported). The hue rides the baked
/// vertex colour (extraction quantised the style-0 `col` per vertex), so —
/// exactly like the props buckets (≡ props/mod.rs L43-113) — `base_color`
/// stays white and no constant is hand-copied; only the unlit flag is
/// structural (`glow`/`lamp` draw as MeshBasic in JS).
#[must_use]
pub fn bucket_material(b: MatBucket) -> StandardMaterial {
    match b {
        MatBucket::Glow => StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        },
        _ => StandardMaterial {
            base_color: Color::WHITE,
            ..default()
        },
    }
}

/// Per-part lamp material from the manifest `lamp{color,emissive,intensity,
/// roughness}` spec (≡ char.schema.md L94; hex parsing reuses the minimap
/// `hexColor` reader, ≡ ui/minimap.rs L76-108). C2 clones these per character
/// for the `_updateMaterials` CPU pass.
#[must_use]
pub fn lamp_material(lamp: &LampSpec) -> StandardMaterial {
    use crate::ui::minimap::hex_rgb;
    let c = hex_rgb(&lamp.color);
    let e = hex_rgb(&lamp.emissive);
    let i = lamp.intensity as f32;
    StandardMaterial {
        base_color: Color::srgb(c[0] / 255.0, c[1] / 255.0, c[2] / 255.0),
        emissive: LinearRgba::new(e[0] / 255.0 * i, e[1] / 255.0 * i, e[2] / 255.0 * i, 1.0),
        perceptual_roughness: lamp.roughness.map_or(0.5, |r| r as f32),
        ..default()
    }
}

// ------------------------------------------------------------------ rig tree

// The tree items below are read by the C2 pose bridge / actors.rs swap
// (D-T54-3), mirroring `#[allow(dead_code)] // wired in Task 52` on
// props' `spawn_archive` (≡ props/mod.rs L223).

/// Marker: the character root (the engine writes its position/rotation,
/// ≡ actor.js L144-153; C2 swaps this into `actors.rs`).
#[derive(Component)]
pub struct CharRig;

/// Marker: a bone entity; `index` is the `rig.boneNames` slot the batch-B
/// pose output (`apply`) writes its local Transform into.
#[derive(Component)]
pub struct CharBone {
    pub index: u16,
}

/// Marker: the kid Group node (skinned parts mount here at rest,
/// ≡ char.schema.md L48-51).
#[derive(Component)]
pub struct CharKid;

/// Marker: the model Group node (squid parts, rigid, ≡ schema L51).
#[derive(Component)]
pub struct CharModel;

/// Marker: a part mesh entity; `part` is the manifest `parts[].id`.
#[derive(Component)]
pub struct CharPart {
    pub part: u32,
}

/// The raw bone-local 3×4 row-major matrix of a part instance
/// (≡ char.schema.md L34-36). Kept beside the decomposed `Transform` because
/// Bevy `Transform` cannot represent the two negative-determinant twins
/// mirror rows (module header deviation).
#[derive(Component)]
pub struct CharPartMatrix(pub [f32; 12]);

/// Which weapon frame node a marker entity is (≡ char.schema.md L89-93:
/// the static `pivot → off → muzzle` chain under `handR`, plus the dual
/// left chain under `handL`). C2 attaches the weapon meshes /
/// `animateWeapon` outputs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponNodeKind {
    PivotR,
    OffR,
    Muzzle,
    PivotL,
    OffL,
}

/// Marker: a weapon frame node.
#[derive(Component)]
pub struct CharWeaponNode(pub WeaponNodeKind);

/// What one spawn of the rig renders: the LOD tier of the kid rows, the
/// held weapon kind (`None` = bare hands) and whether the bomb sub is shown.
#[derive(Debug, Clone, Copy)]
pub struct RigSpawnOpts<'a> {
    /// `hero | game | far` (kid rows; weapon `near` rows render at hero/game,
    /// `far` rows at far — ≡ char.schema.md L58-71).
    pub tier: &'a str,
    pub weapon: Option<&'a str>,
    pub bomb: bool,
}

/// One node of the pure rig tree (parent index always precedes the child).
#[derive(Debug, Clone)]
pub struct RigNode {
    pub parent: Option<usize>,
    pub kind: RigNodeKind,
    pub transform: Transform,
}

/// Node kinds of [`RigNode`].
#[derive(Debug, Clone, PartialEq)]
pub enum RigNodeKind {
    Root,
    Kid,
    Model,
    Bone(u16),
    Part {
        part: u32,
        template: u32,
        mat_class: String,
        matrix: [f32; 12],
    },
    Weapon(WeaponNodeKind),
}

/// Decompose a row-major 3×4 bone-local matrix into a `Transform`
/// (≡ char.schema.md L34-36; three.js `Matrix4.decompose` semantics).
fn matrix_transform(m: &[f32; 12]) -> Transform {
    let mat = Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        Vec4::new(m[3], m[7], m[11], 1.0),
    );
    Transform::from_matrix(mat)
}

/// Does a part row belong to this spawn's visible set?
fn row_visible(row: &PartRow, opts: &RigSpawnOpts) -> bool {
    let weapon_prefix = opts.weapon.map(|k| format!("weapon:{k}:"));
    let is_weapon = row.key.starts_with("weapon:");
    let is_bomb = row.key.starts_with("bomb:");
    if row.tier == opts.tier {
        // far-tier kid rows + far-tier weapon rows of the held kind only.
        (!is_weapon || weapon_prefix.is_some_and(|p| row.key.starts_with(&p)))
            && (!is_bomb || opts.bomb)
    } else if row.tier == "near" && opts.tier != "far" {
        if is_bomb {
            opts.bomb
        } else if is_weapon {
            weapon_prefix.is_some_and(|p| row.key.starts_with(&p))
        } else {
            true // kid rows never carry `near`
        }
    } else {
        false
    }
}

/// Build the pure rig entity tree (bones + tier-filtered part meshes +
/// weapon frame nodes) without touching Bevy resources. Node order is a
/// topological parent-before-child order; index 0 is the root.
#[must_use]
pub fn build_rig_tree(
    archive: &CharArchive,
    manifest: &CharManifest,
    opts: &RigSpawnOpts,
) -> Vec<RigNode> {
    let mut nodes = vec![
        RigNode {
            parent: None,
            kind: RigNodeKind::Root,
            transform: Transform::default(),
        },
        RigNode {
            parent: Some(0),
            kind: RigNodeKind::Kid,
            transform: Transform::default(),
        },
        RigNode {
            parent: Some(0),
            kind: RigNodeKind::Model,
            transform: Transform::default(),
        },
    ];
    // Bone parent-child tree (≡ char.schema.md L83; `None` parent hangs the
    // bone off the kid node ≡ tables.rs L353). Rest positions ride until C2
    // overwrites them with the pose output. Bones occupy node slots
    // 3..3+bone_count, so a parent's node index is `3 + parent` regardless
    // of declaration order.
    for i in 0..manifest.rig.bone_names.len() {
        let parent = manifest.rig.bone_parent[i].map_or(1, |p| 3 + p);
        nodes.push(RigNode {
            parent: Some(parent),
            kind: RigNodeKind::Bone(i as u16),
            transform: Transform::from_translation(Vec3::from(
                manifest.rig.rest[i].map(|x| x as f32),
            )),
        });
    }
    // Part meshes: archive instances in binary order, tier-filtered through
    // the manifest row (≡ char.schema.md L39-41 cross-check contract).
    for inst in &archive.instances {
        let Some(row) = manifest.parts.get(inst.part as usize) else {
            continue; // decode already bounds-checked; the tree stays total
        };
        if !row_visible(row, opts) {
            continue;
        }
        // boneId: 0..87 real bones, then the pseudo mounts appended in the
        // manifest order (≡ char.schema.md L45-51).
        let parent = match usize::from(inst.bone).checked_sub(manifest.rig.bone_names.len()) {
            None => 3 + usize::from(inst.bone),
            Some(k) => match manifest.rig.pseudo_bones.get(k).map(String::as_str) {
                Some("@kid") => 1,
                Some("@model") => 2,
                // Out-of-range mount: park it on the root (decode cannot
                // see the manifest; the cross-check test pins real data).
                _ => 0,
            },
        };
        nodes.push(RigNode {
            parent: Some(parent),
            kind: RigNodeKind::Part {
                part: inst.part,
                template: inst.template,
                mat_class: row.mat_class.clone(),
                matrix: inst.matrix,
            },
            transform: matrix_transform(&inst.matrix),
        });
    }
    // Weapon frame nodes (≡ char.schema.md L89-93): pivot → off → muzzle
    // under handR, the dual left chain under handL. Empty transforms-only
    // nodes; C2 attaches meshes + `animateWeapon` outputs.
    if let Some(kind) = opts.weapon
        && let Some(def) = manifest.weapons.get(kind)
    {
        let (Some(hr), Some(hl)) = (
            manifest.rig.bone_index.get("handR"),
            manifest.rig.bone_index.get("handL"),
        ) else {
            return nodes;
        };
        let fr = &def.frames;
        let pivot_r = nodes.len();
        nodes.push(RigNode {
            parent: Some(3 + *hr),
            kind: RigNodeKind::Weapon(WeaponNodeKind::PivotR),
            transform: Transform::from_translation(Vec3::from(fr.pivot.map(|x| x as f32))),
        });
        let off_r = nodes.len();
        let off_q = fr.off.quat.map(|x| x as f32);
        nodes.push(RigNode {
            parent: Some(pivot_r),
            kind: RigNodeKind::Weapon(WeaponNodeKind::OffR),
            transform: Transform {
                translation: Vec3::from(fr.off.pos.map(|x| x as f32)),
                rotation: Quat::from_xyzw(off_q[0], off_q[1], off_q[2], off_q[3]),
                ..default()
            },
        });
        nodes.push(RigNode {
            parent: Some(off_r),
            kind: RigNodeKind::Weapon(WeaponNodeKind::Muzzle),
            transform: Transform::from_translation(Vec3::from(fr.muzzle.map(|x| x as f32))),
        });
        if let Some((pivot_l, off_l)) = fr.left {
            let pivot_ln = nodes.len();
            nodes.push(RigNode {
                parent: Some(3 + *hl),
                kind: RigNodeKind::Weapon(WeaponNodeKind::PivotL),
                transform: Transform::from_translation(Vec3::from(pivot_l.map(|x| x as f32))),
            });
            let off_lq = off_l.quat.map(|x| x as f32);
            nodes.push(RigNode {
                parent: Some(pivot_ln),
                kind: RigNodeKind::Weapon(WeaponNodeKind::OffL),
                transform: Transform {
                    translation: Vec3::from(off_l.pos.map(|x| x as f32)),
                    rotation: Quat::from_xyzw(off_lq[0], off_lq[1], off_lq[2], off_lq[3]),
                    ..default()
                },
            });
        }
    }
    nodes
}

/// One template → one mesh (POSITION/NORMAL/COLOR_0 + u32 indices,
/// ≡ props/mod.rs L115-123).
fn template_mesh(t: &CharTemplate) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, t.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, t.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, t.colors.clone());
    mesh.insert_indices(Indices::U32(t.indices.clone()));
    mesh
}

/// What [`spawn_char_rig`] built (smoke + budget anchor).
#[derive(Debug)]
pub struct BuiltRig {
    pub root: Entity,
    /// Real bones + the two pseudo mounts (≡ manifest `meta.counts.bones`).
    pub rig_entries: usize,
    pub parts: usize,
    pub weapon_nodes: usize,
}

/// Spawn the rig tree (≡ `props/mod.rs` `spawn_archive` L224-300 precedent:
/// shared template meshes + shared bucket materials, per-node transform).
/// Wiring into `actors.rs` / `world.rs` is batch C2 (D-T54-3).
pub fn spawn_char_rig(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    archive: &CharArchive,
    manifest: &CharManifest,
    opts: &RigSpawnOpts,
) -> BuiltRig {
    let mut mesh_of: Vec<Handle<Mesh>> = Vec::with_capacity(archive.templates.len());
    for t in &archive.templates {
        mesh_of.push(meshes.add(template_mesh(t)));
    }
    let mut mat_of: HashMap<MatBucket, Handle<StandardMaterial>> = HashMap::new();
    let mut bucket_mat = |bucket: MatBucket, materials: &mut Assets<StandardMaterial>| {
        mat_of
            .entry(bucket)
            .or_insert_with_key(|b| materials.add(bucket_material(*b)))
            .clone()
    };

    let nodes = build_rig_tree(archive, manifest, opts);
    let mut entities: Vec<Entity> = Vec::with_capacity(nodes.len());
    let mut built = BuiltRig {
        root: Entity::PLACEHOLDER,
        rig_entries: 0,
        parts: 0,
        weapon_nodes: 0,
    };
    for node in &nodes {
        let parent = node.parent.map(|p| entities[p]);
        let spawn = match &node.kind {
            RigNodeKind::Root => commands.spawn((CharRig, node.transform)),
            RigNodeKind::Kid => {
                built.rig_entries += 1;
                commands.spawn((CharKid, node.transform))
            }
            RigNodeKind::Model => {
                built.rig_entries += 1;
                commands.spawn((CharModel, node.transform))
            }
            RigNodeKind::Bone(i) => {
                built.rig_entries += 1;
                commands.spawn((CharBone { index: *i }, node.transform))
            }
            RigNodeKind::Part {
                part,
                template,
                mat_class,
                matrix,
            } => {
                built.parts += 1;
                let bucket = mat_bucket(mat_class).unwrap_or(MatBucket::Plastic);
                let mat = bucket_mat(bucket, materials);
                commands.spawn((
                    Mesh3d(mesh_of[*template as usize].clone()),
                    MeshMaterial3d(mat),
                    CharPart { part: *part },
                    CharPartMatrix(*matrix),
                    node.transform,
                ))
            }
            RigNodeKind::Weapon(k) => {
                built.weapon_nodes += 1;
                commands.spawn((CharWeaponNode(*k), node.transform))
            }
        };
        let mut spawn = spawn;
        if let Some(p) = parent {
            spawn.insert(ChildOf(p));
        }
        entities.push(spawn.id());
    }
    built.root = entities[0];
    built
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// P0-1 guard: the bytes compiled into the wasm build must be the same as
    /// the extracted archive, and must decode (a stale `char.bin.gz` would
    /// otherwise be silently baked in).
    #[test]
    fn embedded_archive_is_byte_identical_to_disk_and_decodes() {
        const GZ: &[u8] = include_bytes!("../../../../assets/char/char.bin.gz");
        let disk = std::fs::read(assets_dir().join("char.bin.gz")).expect("char.bin.gz on disk");
        assert_eq!(
            GZ,
            disk.as_slice(),
            "re-run node tools/extract/extract-character.mjs"
        );
        let a = decode_gz(GZ).expect("embedded archive decodes");
        assert!(!a.templates.is_empty());
        assert!(!a.instances.is_empty());
    }

    fn manifest() -> CharManifest {
        super::manifest().expect("char.json manifest parses")
    }

    fn archive() -> CharArchive {
        load_archive().expect("char.bin.gz decodes")
    }

    // Decode-count reconciliation against the manifest (≡ props/tests.rs
    // `every_stage_archive_decodes_to_its_manifest` L19-98 precedent).
    #[test]
    fn char_archive_decodes_to_manifest_counts() {
        let m = manifest();
        let a = archive();
        assert_eq!(
            a.templates.len() as u64,
            m.meta.counts.templates,
            "templates"
        );
        assert_eq!(
            a.instances.len() as u64,
            m.meta.counts.instances,
            "instances"
        );
        let mut verts = 0u64;
        let mut tris = 0u64;
        for t in &a.templates {
            verts += t.positions.len() as u64;
            tris += (t.indices.len() / 3) as u64;
        }
        assert_eq!(verts, m.meta.counts.verts, "vertex total");
        assert_eq!(tris, m.meta.counts.tris, "triangle total");
        // Manifest-level counts (task anchor): parts=196, rig 87+2=89,
        // weapons=17, kind list=13 (+4 special).
        assert_eq!(m.parts.len(), 196, "parts rows");
        assert_eq!(m.rig.bone_names.len(), 87, "real bones");
        assert_eq!(
            m.rig.pseudo_bones,
            vec!["@kid".to_string(), "@model".to_string()]
        );
        assert_eq!(
            m.rig.bone_names.len() + m.rig.pseudo_bones.len(),
            m.meta.counts.bones as usize,
            "rig entries = bones meta count"
        );
        assert_eq!(m.weapons.len(), 17, "weapon kinds in manifest");
        assert_eq!(m.weapon_kinds.len(), 13, "weaponKindList.kinds");
        assert_eq!(m.special_weapon_kinds.len(), 4, "specialWeaponKinds");
        assert_eq!(m.meta.bin, "char.bin.gz");
        assert_eq!(m.meta.bin_sha256.len(), 64, "sha256 hex");
    }

    // Per-row cross-check (≡ char.schema.md L39-41: the manifest carries the
    // same id/template/matrix/bone for the archive instances).
    #[test]
    fn manifest_rows_reconcile_with_archive_instances() {
        let m = manifest();
        let a = archive();
        assert_eq!(a.instances.len(), m.parts.len());
        let mut by_part = vec![None; m.parts.len()];
        for inst in &a.instances {
            let slot = by_part
                .get_mut(inst.part as usize)
                .unwrap_or_else(|| panic!("partId {} out of range", inst.part));
            assert!(
                slot.replace(*inst).is_none(),
                "duplicate partId {}",
                inst.part
            );
        }
        let mut names: Vec<String> = m.rig.bone_names.clone();
        names.extend(m.rig.pseudo_bones.iter().cloned());
        for (row, inst) in m.parts.iter().zip(by_part.iter()) {
            let inst = inst.unwrap_or_else(|| panic!("part {} has no instance", row.key));
            assert_eq!(inst.template, row.template, "{}: template", row.key);
            assert_eq!(
                names[usize::from(inst.bone)],
                row.bone,
                "{}: bone name",
                row.key
            );
            let t = &a.templates[inst.template as usize];
            assert_eq!(t.positions.len() as u64, row.verts, "{}: verts", row.key);
            assert_eq!((t.indices.len() / 3) as u64, row.tris, "{}: tris", row.key);
            for (b, f) in inst.matrix.iter().zip(row.matrix.iter()) {
                assert!(
                    (*b - *f as f32).abs() <= 1e-6,
                    "{}: matrix component",
                    row.key
                );
            }
        }
    }

    // Draw-call budget (CONTRACTS §1 ≤45/角色): every tier × every weapon
    // kind, plus the bomb sub, counted as mesh entities in the built tree.
    #[test]
    fn rig_tree_draw_call_budget_per_tier() {
        let m = manifest();
        let a = archive();
        let mut worst = (0usize, String::new());
        for tier in ["hero", "game", "far"] {
            for kind in m.weapons.keys() {
                for bomb in [false, true] {
                    let opts = RigSpawnOpts {
                        tier,
                        weapon: Some(kind),
                        bomb,
                    };
                    let nodes = build_rig_tree(&a, &m, &opts);
                    let parts = nodes
                        .iter()
                        .filter(|n| matches!(n.kind, RigNodeKind::Part { .. }))
                        .count();
                    if parts > worst.0 {
                        worst = (parts, format!("{tier}/{kind}/bomb={bomb}"));
                    }
                    assert!(
                        parts <= 45,
                        "{tier}/{kind}/bomb={bomb}: {parts} draw calls > 45"
                    );
                    // kid rows are always present at their own tier
                    assert!(parts >= 16, "{tier}/{kind}: kid rows missing");
                }
            }
        }
        // Bare hands still renders the kid body.
        let nodes = build_rig_tree(
            &a,
            &m,
            &RigSpawnOpts {
                tier: "hero",
                weapon: None,
                bomb: false,
            },
        );
        let parts = nodes
            .iter()
            .filter(|n| matches!(n.kind, RigNodeKind::Part { .. }))
            .count();
        assert_eq!(parts, 16, "hero bare hands = kid rows only");
    }

    // Tree shape: 89 rig entries (87 bones + @kid + @model), parent indices
    // precede children, bones hang off the kid node at rest.
    #[test]
    fn rig_tree_has_the_manifest_bone_hierarchy() {
        let m = manifest();
        let a = archive();
        let nodes = build_rig_tree(
            &a,
            &m,
            &RigSpawnOpts {
                tier: "hero",
                weapon: Some("shooter"),
                bomb: false,
            },
        );
        let mut bones = 0;
        let mut mounts = 0;
        for (i, n) in nodes.iter().enumerate() {
            if let Some(p) = n.parent {
                assert!(p < i, "node {i}: parent must precede");
            }
            match &n.kind {
                RigNodeKind::Bone(_) => {
                    bones += 1;
                    // hips is the only root bone and hangs off the kid node
                    // (≡ tables.rs L353; char.json boneParent hips = null)
                }
                RigNodeKind::Kid | RigNodeKind::Model => mounts += 1,
                _ => {}
            }
        }
        assert_eq!(bones + mounts, 89, "rig entries");
        // Weapon chain: pivot → off → muzzle under handR (+ dual left pair
        // for kinds that carry `left`).
        let weapon_nodes: Vec<_> = nodes
            .iter()
            .filter_map(|n| match &n.kind {
                RigNodeKind::Weapon(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert_eq!(
            weapon_nodes,
            vec![
                WeaponNodeKind::PivotR,
                WeaponNodeKind::OffR,
                WeaponNodeKind::Muzzle
            ],
            "shooter frame chain"
        );
        let nodes = build_rig_tree(
            &a,
            &m,
            &RigSpawnOpts {
                tier: "hero",
                weapon: Some("dualies"),
                bomb: false,
            },
        );
        let weapon_nodes: Vec<_> = nodes
            .iter()
            .filter_map(|n| match &n.kind {
                RigNodeKind::Weapon(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert_eq!(
            weapon_nodes,
            vec![
                WeaponNodeKind::PivotR,
                WeaponNodeKind::OffR,
                WeaponNodeKind::Muzzle,
                WeaponNodeKind::PivotL,
                WeaponNodeKind::OffL
            ],
            "dualies frame chains"
        );
    }

    // Determinism: the same gz bytes decode to field-identical archives
    // (TR-3.4 discipline ≡ char.schema.md L5-7).
    #[test]
    fn decode_is_deterministic() {
        let gz = std::fs::read(assets_dir().join("char.bin.gz")).expect("char.bin.gz present");
        let a = decode_gz(&gz).expect("first decode");
        let b = decode_gz(&gz).expect("second decode");
        assert_eq!(a.instances, b.instances, "instances identical");
        assert_eq!(a.templates.len(), b.templates.len());
        for (x, y) in a.templates.iter().zip(&b.templates) {
            assert_eq!(x.positions, y.positions, "positions");
            assert_eq!(x.normals, y.normals, "normals");
            assert_eq!(x.colors, y.colors, "colors");
            assert_eq!(x.indices, y.indices, "indices");
        }
    }

    // Corrupt-input rejection, minimal-valid-archive style
    // (≡ props/tests.rs `decode_rejects_corrupt_archives` L154-179).
    #[test]
    fn decode_rejects_corrupt_archives() {
        let mut good = Vec::new();
        good.extend_from_slice(&MAGIC.to_le_bytes());
        good.extend_from_slice(&VERSION.to_le_bytes());
        good.extend_from_slice(&0u32.to_le_bytes()); // templates
        good.extend_from_slice(&0u32.to_le_bytes()); // instances
        assert!(decode(&good).is_some());
        let mut bad = good.clone();
        bad[0] ^= 0xff; // wrong magic
        assert!(decode(&bad).is_none());
        assert!(decode(&good[..good.len() - 2]).is_none()); // truncated
        let mut bad = good.clone();
        bad.push(0); // trailing garbage
        assert!(decode(&bad).is_none());
        // triCount must equal indexCount/3 (≡ char.schema.md L22)
        let mut bad = good.clone();
        bad.splice(8..12, 1u32.to_le_bytes()); // templateCount = 1
        bad.extend_from_slice(&1u32.to_le_bytes()); // vc
        bad.extend_from_slice(&3u32.to_le_bytes()); // ic
        bad.extend_from_slice(&2u32.to_le_bytes()); // tri (wrong)
        assert!(decode(&bad).is_none());
    }

    // Manifest sanity: every matClass buckets, ids are the sorted 0..195,
    // tier vocabulary matches the schema (kid hero|game|far, weapon near|far).
    #[test]
    fn manifest_mat_classes_all_bucket_and_ids_are_dense() {
        let m = manifest();
        for row in &m.parts {
            assert!(
                mat_bucket(&row.mat_class).is_some(),
                "{}: matClass {}",
                row.key,
                row.mat_class
            );
            assert!(
                matches!(row.tier.as_str(), "hero" | "game" | "far" | "near"),
                "{}: tier {}",
                row.key,
                row.tier
            );
        }
        let mut ids: Vec<u32> = m.parts.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            (0..m.parts.len() as u32).collect::<Vec<_>>(),
            "dense ids"
        );
        // The schema's 12 matClass values all appear (char.schema.md L73-74).
        let classes: std::collections::BTreeSet<&str> =
            m.parts.iter().map(|p| p.mat_class.as_str()).collect();
        assert_eq!(
            classes.len(),
            12,
            "all 12 schema matClasses present: {classes:?}"
        );
    }

    // The twins mirror rows carry det = −1 matrices; Bevy Transform cannot
    // represent mirroring, so the raw matrix must ride in CharPartMatrix and
    // the deviation stays visible (module header).
    #[test]
    fn twins_far_mirror_is_a_negative_determinant() {
        let m = manifest();
        let rows: Vec<&PartRow> = m
            .parts
            .iter()
            .filter(|p| p.key.starts_with("weapon:twins:") && p.key.ends_with("@L"))
            .collect();
        assert_eq!(rows.len(), 2, "twins far @L rows");
        for row in &rows {
            let mm = row.matrix;
            let det = mm[0] * (mm[5] * mm[10] - mm[6] * mm[9])
                - mm[1] * (mm[4] * mm[10] - mm[6] * mm[8])
                + mm[2] * (mm[4] * mm[9] - mm[5] * mm[8]);
            assert!(det < 0.0, "{}: mirror det {det}", row.key);
        }
    }

    // The lamp colour constants must come from the manifest, never be
    // hand-copied: every lamp spec parses to a finite material.
    #[test]
    fn lamp_specs_parse_from_manifest() {
        let m = manifest();
        let mut seen = 0;
        for (kind, def) in &m.weapons {
            for (name, spec) in &def.parts {
                if let Some(lamp) = &spec.lamp {
                    let mat = lamp_material(lamp);
                    let c = mat.base_color.to_linear();
                    assert!(c.red.is_finite() && c.green.is_finite() && c.blue.is_finite());
                    let e = mat.emissive.to_f32_array();
                    assert!(
                        e.iter().all(|x| x.is_finite()),
                        "{kind}/{name}: emissive finite"
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen >= 4, "lamp specs present in the manifest (saw {seen})");
    }
}
