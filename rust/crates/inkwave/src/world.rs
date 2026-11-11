//! Task 10 — Bevy level-world rendering for the Rust port.
//!
//! Turns an [`inkwave_sim::geometry::StageLayout`] into batched render meshes
//! (one draw call per pattern slot × block colour × face-orientation group —
//! Task 52 D2), plus the daytime environment (gradient sky dome, sun with
//! toggleable cascaded shadows, translucent sea at `water_y`, team-tinted
//! spawn-barrier cylinders) and a third-person camera rig with mouse look and
//! collision boom-shrink.
//!
//! Faithful to upstream `src/world/level.js` (`_addBlock` block frames and the
//! `hidden` cull rule) and `src/world/environment.js` (day theme sun/sky
//! colours). Task 52 D2 replaces the M1 solid-colour approximation with the
//! ported texlib uber-shader (`level.wgsl` via [`crate::level_render::LevelExt`]).

use std::collections::HashMap;

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, GlobalAmbientLight};
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::geometry::{Brush, StageLayout};
use inkwave_sim::ink_atlas::InkAtlas;

use crate::environment::{EnvState, srgb_color};
use crate::ink_render::{InkAtlasRes, InkImage};
use crate::level_render::{LevelExt, LevelMaterial, add_texlib_placeholders};
use bevy::post_process::bloom::Bloom;

// Task 55: sky / sun / fog / sea / shadow-size constants are no longer
// hand-written here. `EnvParams` reads THEMES + QUALITY from the extracted
// `rust/assets/environment.json` (see `crate::environment`).
/// Horizontal field of view (settings.fov default, cameraRig.js L180).
/// cameraRig.js L188 converts it with the FIXED 16/9 reference aspect,
/// independent of the window size; [`REF_ASPECT`] is that constant.
pub(crate) const BASE_FOV_H: f32 = 82.0;
const REF_ASPECT: f32 = 16.0 / 9.0;
/// World camera clip planes. `camera_rig` only rewrites FOV every frame, so
/// the Task 56 lobby override restores these explicitly when it steps out
/// (review P1-1: near/far would otherwise stay at the alley's 0.1/140).
pub(crate) const CAM_NEAR: f32 = 0.15;
pub(crate) const CAM_FAR: f32 = 6500.0;
/// Default boom length (cameraRig.js `dist`).
const BOOM: f32 = 4.5;
/// Pivot height above the feet (cameraRig.js `hgt`).
const PIVOT_H: f32 = 1.85;

/// Parse a `#rrggbb` sRGB hex string into a [`Color`].
fn hex_color(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    let byte = |i: usize| -> f32 {
        u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(221) as f32 / 255.0
    };
    Color::srgb(byte(0), byte(1), byte(2))
}

/// World-space frame for one primitive (centre, half-extents, axes as world
/// directions). Mirrors `level.js` `_addBlock`.
struct Frame {
    center: Vec3,
    half: Vec3,
    axes: [Vec3; 3],
}

fn frame_of(brush: &Brush) -> Frame {
    match brush {
        Brush::Box { min, max, .. } => {
            let min = Vec3::from(*min);
            let max = Vec3::from(*max);
            Frame {
                center: (min + max) * 0.5,
                half: (max - min) * 0.5,
                axes: [Vec3::X, Vec3::Y, Vec3::Z],
            }
        }
        Brush::Obox {
            center,
            size,
            rot_y,
            ..
        } => {
            // level.js L67: axes = [(c,0,-s), Y, (s,0,c)] for rotY degrees.
            let a = rot_y.to_radians();
            let (c, s) = (a.cos(), a.sin());
            Frame {
                center: Vec3::from(*center),
                half: Vec3::from(*size) * 0.5,
                axes: [Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c)],
            }
        }
        Brush::Ramp {
            low,
            high,
            width,
            thickness,
            thin,
            ..
        } => {
            // level.js L72-94: tilted slab, top runs low→high, underside to floor.
            let l0 = Vec3::from(*low);
            let h0 = Vec3::from(*high);
            let d = h0 - l0;
            let len = d.length();
            let s = if len > 1e-6 { d / len } else { Vec3::X };
            let flat = Vec3::new(s.x, 0.0, s.z).normalize_or_zero();
            // level.js L77: side = UP × flat (left-handed cross, ported verbatim).
            let mut side = Vec3::Y.cross(flat).normalize_or_zero();
            let mut n = s.cross(side).normalize_or_zero();
            if n.y < 0.0 {
                n = -n;
                side = -side;
            }
            let rise = h0.y - l0.y;
            let thick = if *thin {
                *thickness
            } else {
                thickness.max(rise * n.y + 0.35)
            };
            let ext = 0.6; // extend under the floor at the low end
            let a = l0 - s * ext;
            let center = (a + h0) * 0.5 - n * (thick / 2.0);
            // level.js L92-93: keep the frame right-handed (side × n = s).
            let mut axes = [side, n, s];
            if side.cross(n).dot(s) < 0.0 {
                axes[0] = -axes[0];
            }
            Frame {
                center,
                half: Vec3::new(*width / 2.0, thick / 2.0, (len + ext) / 2.0),
                axes,
            }
        }
    }
}

/// Vertex data accumulated for one render batch.
#[derive(Default)]
struct Batch {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// UV1 = ink-atlas rect coords (zero when the batch has no ink context).
    uvs_b: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

/// Box face table: (normal axis, outward sign). The index order ≡ the
/// collision layout `faces[k * 2 + (sign > 0 ? 0 : 1)]` (level.js
/// `_buildFaces` L202). The per-face (u, v) axes are NOT fixed here — they
/// follow the JS frame rule in [`face_frame`] (wall v = world up, top/bottom
/// u = world X), which the shader's edge / drip / stair math depends on.
const FACES: [(usize, f32); 6] = [
    (0, 1.0),
    (0, -1.0),
    (1, 1.0),
    (1, -1.0),
    (2, 1.0),
    (2, -1.0),
];

/// Pick the face's in-plane axes and signed tangent vectors ≡ level.js
/// `_buildFaces` L170-182 verbatim:
/// * wall (`|n.y| < 0.5`): `vi` = the in-plane axis most aligned with world
///   up, `v` = that axis flipped to `v.y >= 0`; `ui` = the other axis.
/// * top/bottom: `ui` = the in-plane axis most aligned with world X, `vi` =
///   the other, `v` flipped to `+z` when `v.z < 0 && |v.z| > 0.3`.
/// * `u = v × n` (JS L182, `u × v = n`) — the handedness fixes the pattern
///   mirror, so the sign of `u` must match JS exactly.
///
/// For the axis-aligned frame this yields the JS table:
/// (+X): u=-Z v=+Y · (-X): u=+Z v=+Y · (+Y): u=-X v=+Z · (-Y): u=+X v=+Z ·
/// (+Z): u=+X v=+Y · (-Z): u=-X v=+Y.
fn face_frame(axes: &[Vec3; 3], na: usize, sign: f32) -> (usize, usize, Vec3, Vec3) {
    let n = axes[na] * sign;
    let others: [usize; 2] = match na {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let (ui, vi) = if n.y.abs() < 0.5 {
        let vi = if axes[others[0]].y.abs() > axes[others[1]].y.abs() {
            others[0]
        } else {
            others[1]
        };
        let ui = if others[0] == vi {
            others[1]
        } else {
            others[0]
        };
        (ui, vi)
    } else {
        let ui = if axes[others[0]].x.abs() >= axes[others[1]].x.abs() {
            others[0]
        } else {
            others[1]
        };
        let vi = if others[0] == ui {
            others[1]
        } else {
            others[0]
        };
        (ui, vi)
    };
    let mut v = axes[vi];
    if n.y.abs() < 0.5 {
        if v.y < 0.0 {
            v = -v;
        }
    } else if v.z < 0.0 && v.z.abs() > 0.3 {
        v = -v;
    }
    let u = v.cross(n);
    (ui, vi, u, v)
}

/// Map an upstream PATTERN id to its texlib slot (levelMaterial.js L79-85
/// table is 50 slots wide, slot 49 = TL_SIDE). The layout's pattern id ≡ the
/// JS `b.pattern` ≡ the shader's `u_pattern`, so no style fallback table is
/// needed any more.
fn pattern_slot(pattern: u16) -> u16 {
    pattern.min(49)
}

/// Quantised face-orientation identity of one box face — everything the
/// `level.wgsl` `BatchParams` (deviation ③) needs, constant inside a batch.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct OrientGroup {
    /// The block frame axes, each component × 64 rounded (orientation
    /// signature: recovers `u_tan` and the wall test from the key).
    sig: [i8; 9],
    /// `FACES` index (0..6): picks the (na, sign) pair; the (u, v) frame is
    /// then re-derived from `sig` with [`face_frame`] (≡ JS `_buildFaces`).
    face: u8,
    /// Face size along u/v in metres as exact `f32` bits (stored bit-wise so
    /// the key stays `Eq`/`Hash`/`Ord`). The shader's edge distances
    /// (`ef ≡ L745`, coping, drip grooves) need the precise face size, so it
    /// must be part of the key, not just the batch.
    su: u32,
    sv: u32,
    /// ≡ JS `f.atlas ? 1 : 0` (level.js L266): the face owns a paint-atlas
    /// rect, so ink can wet it (`wetBase`).
    wet: bool,
    /// ≡ JS `groundedBottom` (L198-201): wall faces only.
    grounded: bool,
}

/// Batch key: pattern slot × block colour × face-orientation group.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct BatchKey {
    pattern: u16,
    color: String,
    orient: OrientGroup,
}

impl BatchKey {
    /// The frame axes recovered from the quantised signature.
    fn axes(&self) -> [Vec3; 3] {
        let s = &self.orient.sig;
        std::array::from_fn(|i| {
            Vec3::new(s[i * 3] as f32, s[i * 3 + 1] as f32, s[i * 3 + 2] as f32) / 64.0
        })
    }

    /// Deviation ③: the per-batch `BatchParams` re-derived from the key
    /// (≡ JS `fdat`/`fflag`/`ftan` vertex attributes, constant per box face).
    fn batch_params(&self) -> crate::level_render::BatchParams {
        let (na, sign) = FACES[self.orient.face as usize];
        let axes = self.axes();
        let normal_y = axes[na].y * sign;
        let (_ui, _vi, u, _v) = face_frame(&axes, na, sign);
        crate::level_render::BatchParams {
            pattern: self.pattern as f32,
            wet_base: self.orient.wet as u32 as f32,
            flags_x: (normal_y.abs() < 0.3) as u32 as f32,
            flags_y: self.orient.grounded as u32 as f32,
            mur: -1.0,
            pad: [0.0; 3],
            fsz: Vec4::new(
                f32::from_bits(self.orient.su),
                f32::from_bits(self.orient.sv),
                0.0,
                0.0,
            ),
            // ≡ JS `faceTan = f.u` (level.js L268) — the signed tangent.
            tan: (u, 0.0).into(),
        }
    }
}

/// Append one box face (4 verts / 2 tris) to `batch`. UV0 = the face-local
/// metre coords ≡ JS `faceUv` (cu, cv from the face origin, 0..su × 0..sv —
/// the shader's edge distances `min(fu, fsz - fu)` rely on that convention).
///
/// With `ink = Some((world, atlas, brush_index))` the block's face is looked
/// up through the collision world (`blocks[bid].faces[na * 2 + sign]`, the
/// same table as [`FACES`]) and each vertex gets UV1 = its position in that
/// face's atlas rect; non-paintable / culled faces get `(0, 0)`.
fn emit_face(
    batch: &mut Batch,
    f: &Frame,
    face: usize,
    ink: Option<(&CollisionWorld, &InkAtlas, usize)>,
) {
    let (na, sign) = FACES[face];
    let normal = f.axes[na] * sign;
    let (ui, vi, u, v) = face_frame(&f.axes, na, sign);
    let quad_start = batch.positions.len() as u32;
    // collision face id for this box face (FACES order == faces[k*2+sign])
    let face_atlas = ink.map(|(world, atlas, bid)| {
        let fid = world.blocks[bid].faces[na * 2 + if sign > 0.0 { 0 } else { 1 }];
        if fid < 0 {
            return None;
        }
        let fa = atlas.face_atlas(fid as u32)?;
        Some((fa, &world.faces[fid as usize], atlas.size()))
    });
    // u × v = n (JS L182), so this corner order is CCW about the outward
    // normal, and the (-1, -1) corner is the JS face origin (L184).
    for (cu, cv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let world = f.center + normal * f.half[na] + u * (cu * f.half[ui]) + v * (cv * f.half[vi]);
        batch.positions.push(world.to_array());
        batch.normals.push(normal.to_array());
        batch
            .uvs
            .push([(cu + 1.0) * f.half[ui], (cv + 1.0) * f.half[vi]]);
        if ink.is_some() {
            let uv_b = match face_atlas.flatten() {
                Some((fa, face, size)) => {
                    // glam 0.30 (sim) vs 0.32 (Bevy): bridge via arrays
                    let o = Vec3::from_array(face.origin.to_array());
                    let uu = Vec3::from_array(face.u.to_array());
                    let vv = Vec3::from_array(face.v.to_array());
                    let rel = world - o;
                    fa.uv_at(rel.dot(uu), rel.dot(vv), size)
                }
                None => (0.0, 0.0),
            };
            batch.uvs_b.push([uv_b.0, uv_b.1]);
        }
    }
    batch.indices.extend_from_slice(&[
        quad_start,
        quad_start + 1,
        quad_start + 2,
        quad_start,
        quad_start + 2,
        quad_start + 3,
    ]);
}

/// Append one whole oriented box (all six faces) to a single `batch` — the
/// M1 path, kept for tests; the renderer routes faces through
/// [`build_level_batches`] so each box face lands in its orientation batch.
#[cfg(test)]
fn emit_box(batch: &mut Batch, f: &Frame, ink: Option<(&CollisionWorld, &InkAtlas, usize)>) {
    for face in 0..6 {
        emit_face(batch, f, face, ink);
    }
}

/// Build the render batches: one entry per (pattern slot, block colour,
/// face-orientation group), sorted by key. Pure data (no GPU assets), so the
/// tests can assert the batching directly.
fn build_level_batches(
    l: &StageLayout,
    world: &CollisionWorld,
    atlas: &InkAtlas,
) -> Vec<(BatchKey, Batch)> {
    let mut batches: HashMap<BatchKey, Batch> = HashMap::new();
    let mut scratch: Vec<u32> = Vec::new();
    for (bid, brush) in l.primitives.iter().enumerate() {
        let c = brush.common();
        if c.hidden {
            continue;
        }
        let f = frame_of(brush);
        for (face, &(na, sign)) in FACES.iter().enumerate() {
            let normal = f.axes[na] * sign;
            let (ui, vi, _u, v) = face_frame(&f.axes, na, sign);
            // ≡ JS face.wall (level.js L187)
            let wall = normal.y.abs() < 0.3;
            // ≡ JS `f.atlas ? 1 : 0` (L266): a paint-atlas rect exists for
            // this face (culled / non-paintable faces have none).
            let fid = world.blocks[bid].faces[na * 2 + if sign > 0.0 { 0 } else { 1 }];
            let wet = fid >= 0 && atlas.face_atlas(fid as u32).is_some();
            // ≡ JS groundedBottom (L198-201): probe the face's bottom edge
            // centre — origin + u·su/2 + v·(-0.06) + n·0.06, i.e. the face
            // centre of the v = 0 edge pushed 6 cm out along n. `v` is the
            // signed JS face v (world-up aligned on walls).
            let mut grounded = false;
            if wall {
                let probe = f.center + normal * (f.half[na] + 0.06) - v * (f.half[vi] + 0.06);
                grounded = world.point_inside(
                    glam::Vec3::from_array(probe.to_array()),
                    0.0,
                    bid as i32,
                    &mut scratch,
                ) || probe.y < 0.02;
            }
            let sig =
                std::array::from_fn(|i| (f.axes[i / 3].to_array()[i % 3] * 64.0).round() as i8);
            let key = BatchKey {
                pattern: pattern_slot(c.pattern),
                color: c.color.clone(),
                orient: OrientGroup {
                    sig,
                    face: face as u8,
                    su: (2.0 * f.half[ui]).to_bits(),
                    sv: (2.0 * f.half[vi]).to_bits(),
                    wet,
                    grounded,
                },
            };
            emit_face(
                batches.entry(key).or_default(),
                &f,
                face,
                Some((world, atlas, bid)),
            );
        }
    }
    let mut sorted: Vec<_> = batches.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
}

/// Build a [`Mesh`] from accumulated batch data.
fn to_mesh(batch: &Batch) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, batch.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, batch.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs.clone());
    if !batch.uvs_b.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, batch.uvs_b.clone());
    }
    mesh.insert_indices(Indices::U32(batch.indices.clone()));
    mesh
}

/// Vertical gradient for the sky dome (theme zenith → skyMid → horizon,
/// `environment.js` THEMES). The gradient is compressed into the upper
/// hemisphere (v 0..0.5 → zenith→mid→horizon), so the horizon tint reaches
/// eye level like the upstream shader; the lower half (hidden by the
/// sea/ground) stays horizon-coloured.
fn sky_image(p: &crate::environment::EnvParams) -> Image {
    const W: u32 = 4;
    const H: u32 = 128;
    let mut px = vec![0u8; (W * H * 4) as usize];
    let to255 = |c: [f32; 3]| {
        [
            (c[0] * 255.0).round() as u8,
            (c[1] * 255.0).round() as u8,
            (c[2] * 255.0).round() as u8,
        ]
    };
    let zenith = to255(p.zenith);
    let mid = to255(p.sky_mid);
    let hor = to255(p.horizon);
    for y in 0..H {
        let t = (y as f32 / (H - 1) as f32 * 2.0).min(1.0); // 0 = top of the texture
        let (a, b, k) = if t < 0.5 {
            (zenith, mid, t / 0.5)
        } else {
            (mid, hor, (t - 0.5) / 0.5)
        };
        let c = [
            a[0] as f32 + (b[0] as f32 - a[0] as f32) * k,
            a[1] as f32 + (b[1] as f32 - a[1] as f32) * k,
            a[2] as f32 + (b[2] as f32 - a[2] as f32) * k,
        ];
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            px[i] = c[0] as u8;
            px[i + 1] = c[1] as u8;
            px[i + 2] = c[2] as u8;
            px[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    )
}

/// Append a lat/long sphere (dome) to `batch`; `v` runs top→bottom for the
/// sky gradient texture.
fn emit_sphere(batch: &mut Batch, center: Vec3, radius: f32, sectors: u32, stacks: u32) {
    let vertex = |batch: &mut Batch, i: u32, j: u32| {
        let theta = std::f32::consts::PI * i as f32 / stacks as f32; // 0..pi
        let phi = std::f32::consts::TAU * j as f32 / sectors as f32;
        let p = Vec3::new(
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        );
        batch.positions.push((center + p * radius).to_array());
        batch.normals.push(p.to_array());
        batch
            .uvs
            .push([j as f32 / sectors as f32, i as f32 / stacks as f32]);
    };
    for i in 0..stacks {
        for j in 0..sectors {
            let a = batch.positions.len() as u32;
            vertex(batch, i, j);
            vertex(batch, i + 1, j);
            vertex(batch, i + 1, j + 1);
            vertex(batch, i, j + 1);
            // CCW seen from outside (theta increases downward, phi CCW in x-z).
            batch
                .indices
                .extend_from_slice(&[a, a + 2, a + 1, a, a + 3, a + 2]);
        }
    }
}

/// Append a horizontal quad (normal +Y) to `batch`.
fn emit_quad(batch: &mut Batch, center: Vec3, hx: f32, hz: f32) {
    let a = batch.positions.len() as u32;
    for (x, z) in [(-hx, -hz), (-hx, hz), (hx, hz), (hx, -hz)] {
        batch
            .positions
            .push((center + Vec3::new(x, 0.0, z)).to_array());
        batch.normals.push([0.0, 1.0, 0.0]);
        batch.uvs.push([x, z]);
    }
    batch
        .indices
        .extend_from_slice(&[a, a + 1, a + 2, a, a + 2, a + 3]);
}

/// Append an open cylinder shell (normals outward) to `batch`.
fn emit_cylinder_shell(batch: &mut Batch, center: Vec3, radius: f32, half_h: f32, seg: u32) {
    for j in 0..seg {
        let a = batch.positions.len() as u32;
        let a0 = std::f32::consts::TAU * j as f32 / seg as f32;
        let a1 = std::f32::consts::TAU * (j + 1) as f32 / seg as f32;
        for (ang, u) in [
            (a0, j as f32 / seg as f32),
            (a1, (j + 1) as f32 / seg as f32),
        ] {
            let dir = Vec3::new(ang.cos(), 0.0, ang.sin());
            for h in [-half_h, half_h] {
                batch
                    .positions
                    .push((center + dir * radius + Vec3::Y * h).to_array());
                batch.normals.push(dir.to_array());
                batch.uvs.push([u, (h + half_h) / (2.0 * half_h)]);
            }
        }
        // quad order: (a0,-h) (a0,+h) (a1,+h) (a1,-h) → CCW seen from outside
        batch
            .indices
            .extend_from_slice(&[a, a + 1, a + 2, a, a + 2, a + 3]);
    }
}

/// Marker: batched level geometry (casts + receives shadows).
#[derive(Component)]
struct LevelGeometry;

/// Marker: the sun entity (shadow toggle target).
#[derive(Component)]
struct SunLight;

/// The loaded stage layout.
#[derive(Resource)]
pub struct LayoutRes(pub StageLayout);

/// Camera rig state (Task 12: the pivot follows the player actor, yaw/pitch
/// live in `PlayerControls`; only the collision boom spring remains here).
#[derive(Resource)]
struct CamRig {
    boom: f32,
    /// Exponential smoothing of the follow position (upstream rig lerps the
    /// pivot; M1 damps toward the player's visual position).
    pivot: Vec3,
}

impl Default for CamRig {
    fn default() -> Self {
        Self {
            boom: BOOM,
            pivot: Vec3::new(0.0, 2.4, -41.8),
        }
    }
}

/// Collision world backing the camera boom probe.
#[derive(Resource)]
struct SimWorld(CollisionWorld);

/// Plugin wiring the level renderer + environment + camera rig.
pub struct WorldPlugin {
    pub layout: StageLayout,
}

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        let layout = self.layout.clone();
        // One CollisionWorld shared by the atlas packing, the demo match and
        // the camera probe (face ids must agree across all three views).
        let world = CollisionWorld::from_layout(&layout);
        let atlas = InkAtlasRes::new(&world);
        let demo = crate::ink_render::DemoSim::new(layout.clone(), world.clone(), 20261004);
        // The app boots on the main menu: the sim stays frozen until Play
        // (Task 13), so `PlayerControls` starts paused.
        let pc = crate::input::PlayerControls {
            paused: true,
            ..default()
        };
        // Task 55: boot theme/tier from INKWAVE_THEME / INKWAVE_QUALITY; the
        // extracted data drives clear colour, lights, fog and shadow size.
        let boot = EnvState::default();
        let boot_horizon = srgb_color(boot.params.horizon);
        app.add_plugins((
            crate::ink_render::InkPlugin,
            // Task 52 D2: the level uber-shader material.
            crate::level_render::LevelPlugin,
        ))
        .insert_resource(boot)
        .insert_resource(ClearColor(boot_horizon))
        .insert_resource(LayoutRes(layout))
        .insert_resource(SimWorld(world))
        .insert_resource(atlas)
        .insert_resource(demo)
        .insert_resource(pc)
        .init_resource::<CamRig>()
        .init_resource::<crate::input::LookSettings>()
        .init_resource::<crate::ui::UiState>()
        .init_resource::<crate::ui::Feed>()
        .init_resource::<crate::ui::HitFlash>()
        // Task 24: the net-match link resources (plan Step 4).
        .init_resource::<crate::net::match_link::NetMatchState>()
        .init_resource::<crate::net::match_link::NetEvents>()
        // Task 51: the FX data core + hooks runtime and the batched
        // quad renderer over the pools.
        .insert_resource(crate::fx::FxState::default())
        .init_resource::<crate::fx::render::FxRenderState>()
        // Task 51 batch E: the zone marks / swim wake / screen-fx data
        // layers (zoneMarks.js / swimWake.js / screenfx.js).
        .init_resource::<crate::fx::zone_marks::ZoneMarksState>()
        .init_resource::<crate::fx::swim_wake::SwimWakeState>()
        .init_resource::<crate::fx::screen_fx::ScreenFxState>()
        // Task 52 batch E2: the ripple pool (paint.js L551-587) fed by the
        // splat stream + capture floods, and the flood-edge tracker.
        .init_resource::<crate::ink_ripple::RippleTable>()
        .init_resource::<crate::level_render::RippleFeed>()
        // Task 54 batch C2: per-slot CharAnim + rig binding state.
        .init_resource::<crate::char::bridge::CharBridge>()
        // Task 56 batch C: showcase state machine + build-once lobby set.
        .init_resource::<crate::ui::showcase_state::ShowcaseState>()
        .init_resource::<crate::ui::showcase_scene::ShowcaseScene>();
        // Task 24: the online session. Stored as a *non-send* resource — the
        // platform transport is not `Send + Sync` (native `Receiver`, wasm
        // `Rc`), and the Bevy main thread owns it. `RelayBase` is inserted by
        // `main.rs` before this plugin (the headless branch builds neither).
        let relay = app
            .world()
            .get_resource::<crate::net::RelayBase>()
            .map(|r| r.0.clone())
            .unwrap_or_default();
        app.world_mut()
            .insert_non_send(crate::net::OnlineSession::new(&relay));
        // Task 14: procedural SFX (silent degradation when no audio device).
        crate::audio::add_audio(app);
        app.add_systems(
            Startup,
            (
                build_level_world,
                crate::actors::spawn_actors,
                crate::ui::build_ui,
                // Task 49 批次 C: the minimap tree hangs under the HUD root,
                // so it spawns after `build_ui` applied the screen trees.
                crate::ui::minimap_ui::minimap_startup,
            )
                .chain(),
        )
        .add_systems(
            Update,
            // The chain is split into two nested groups (Bevy's `.chain()`
            // impl covers tuples up to 20 systems; the full ordered list is
            // 22). The outer `.chain()` keeps group 1 entirely before group 2.
            (
                (
                    crate::input::map_input,
                    // Before the UI state machine: it drains `Match::events`.
                    crate::audio::audio_sfx,
                    // gameplay → VFX glue ≡ the JS `hooks.update(dt)` call:
                    // reads `Match::events` with `.iter()` only, so it must
                    // run before the UI state machine drains them (Task 51 D).
                    crate::fx::hooks::fx_hooks,
                    // Task 51 batch E: same read-only `Match::events` rule —
                    // all three run before the UI drain.
                    crate::fx::zone_marks::zone_marks_update,
                    crate::fx::swim_wake::swim_wake_update,
                    crate::fx::screen_fx::screen_fx_update,
                    crate::audio::music::music_scenes,
                    crate::audio::music::music_update,
                    crate::ui::ui_state_machine,
                    // Task 24: consumes the staged session events and steps the
                    // replication controller before the atlas drain (`ink_upload`)
                    // and before the visual-tree rebuild/sync.
                    crate::net::match_link::net_tick,
                    crate::input::tune_sensitivity,
                    crate::ink_render::ink_upload,
                    // Task 52 batch E2: drain the texlib bake worker and swap
                    // the placeholder arrays in place (cheap poll; inert once
                    // the result has landed).
                    crate::texlib::res::texlib_bake_poll,
                    crate::actors::sync_actor_count,
                    crate::actors::sync_actors,
                    crate::ui::ui_update,
                    // Task 49 批次 C: raster + compose first, then the box
                    // spring / layout / dot writes read the same frame's state.
                    crate::ui::minimap_ui::minimap_update,
                    crate::ui::minimap_ui::minimap_ui,
                    crate::ui::online_ui_update,
                )
                    .chain(),
                (
                    // Task 54 batch C2: the pose bridge — CharAnim state →
                    // rig tree writes; runs right after the root sync group
                    // (group 1 is at the `chain()` tuple cap, so it heads
                    // group 2 — still before every consumer below).
                    crate::char::bridge::char_pose_bridge,
                    // Task 56 batch C: sync the showcase state from the very
                    // same OnlineView the 2D panel consumed in group 1, then
                    // build/gate the placeholder alley set from it.
                    crate::ui::showcase_scene::showcase_sync,
                    crate::ui::showcase_scene::showcase_scene_update,
                    // Task 50 batch D: plays the cues `ui_update` recorded this
                    // frame (runs after the director, before the voice GC).
                    crate::audio::boss_audio::boss_audio_playback,
                    // Task 26: the test-only smoke seam runs after the session
                    // pump so every frame sees the settled state (inert without
                    // seam query params; `None` config ⇒ immediate return).
                    crate::nettest::nettest_update,
                    crate::ui::ui_button_style,
                    crate::audio::sfx_gc,
                    environment_controls,
                    camera_rig,
                    // Task 56 batch C: while the alley set is full-frame the
                    // extracted hub/line-up pose wins over `camera_rig`; on
                    // exit this system stops writing and the rig reclaims it.
                    crate::ui::showcase_scene::showcase_camera_update,
                    // Task 52 batch E2: ripple / wake / clock uniform wiring
                    // into every level material; runs right after `camera_rig`
                    // so `setView` sees this frame's camera position.
                    crate::level_render::level_material_wiring,
                    // ≡ the JS `fx.update(dt, camera)` main-loop call plus the
                    // per-frame instance-buffer rebuild (Task 51 C): runs at the
                    // chain tail so `camera_rig`'s camera basis is current.
                    crate::fx::render::fx_update,
                )
                    .chain(),
            )
                .chain(),
        );
    }
}

/// Boom length for the next frame given a line-of-sight probe result
/// (cameraRig.js `cameraProbe` + `boom.step`): shrink toward the pivot when a
/// wall blocks the line of sight, clamped to `[0.45, BOOM]`. Upstream is a
/// critically-damped spring with fast-in / slow-out rates (22+26·deep vs 3.6,
/// L381); M1 approximates that with frame-rate-independent exponential damping
/// at the same rates.
fn boom_next(current: f32, hit: bool, hit_dist: f32, dt: f32) -> f32 {
    let clear = if hit { hit_dist - 0.3 } else { BOOM };
    let target = BOOM.min(clear.max(0.45));
    let k = if target < current { 22.0 } else { 3.6 };
    let a = 1.0 - (-k * dt).exp();
    current + (target - current) * a
}

/// Vertical FOV (deg) from a horizontal FOV at a reference aspect (cameraRig.js L188).
fn vfov(h_deg: f32, aspect: f32) -> f32 {
    2.0 * ((h_deg * 0.5).to_radians().tan() / aspect)
        .atan()
        .to_degrees()
}

/// Build all level geometry, environment, lights and the camera.
#[allow(clippy::too_many_arguments)]
fn build_level_world(
    mut commands: Commands,
    layout: Res<LayoutRes>,
    sim: Res<SimWorld>,
    atlas_res: Res<InkAtlasRes>,
    env: Res<EnvState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut level_materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let l = &layout.0;
    let p = &env.params;
    let ink_tex = images.add(atlas_res.make_image());
    commands.insert_resource(InkImage(ink_tex.clone()));

    // ---- texlib placeholder textures + the shared level extension ----
    // Batch E replaces the placeholder array contents with the baked layers;
    // until then the shader path runs on neutral constants.
    let doc = crate::texlib::schema::embedded_texlib();
    let texlib_images = add_texlib_placeholders(&doc, &mut images);
    commands.insert_resource(texlib_images.clone());
    // Batch E2: bake the slot-referenced layers on a worker thread at the
    // quality tier's size (Low 256 / Med·High 512); `texlib_bake_poll` swaps
    // the placeholder contents once the chain lands. The headless branch
    // never adds `WorldPlugin`, so this startup system — and with it the
    // bake — is render-world gated by construction.
    let tier = crate::quality::tier_from_env();
    commands.insert_resource(crate::texlib::res::start_bake(&doc, tier.texlib_size()));
    let base_ext = LevelExt::new_shared(
        ink_tex.clone(),
        &texlib_images,
        &doc,
        atlas_res.atlas.size(),
    );

    // ---- batched primitive geometry (one draw call per pattern slot ×
    // colour × face-orientation group) ----
    let mut visible = 0u32;
    let mut hidden = 0u32;
    for brush in &l.primitives {
        if brush.common().hidden {
            hidden += 1;
        } else {
            visible += 1;
        }
    }
    let batches = build_level_batches(l, &sim.0, &atlas_res.atlas);
    let mut draw_calls = 0u32;
    for (key, batch) in batches {
        if batch.positions.is_empty() {
            continue;
        }
        let mesh = meshes.add(to_mesh(&batch));
        let mat = level_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: hex_color(&key.color),
                // ≡ levelMaterial.js L40 material baseline (doc.mat.roughness).
                perceptual_roughness: doc.mat.roughness,
                ..default()
            },
            extension: LevelExt {
                batch: key.batch_params(),
                ..base_ext.clone()
            },
        });
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            LevelGeometry,
            Visibility::default(),
        ));
        draw_calls += 1;
    }

    // ---- environment ----
    let b = &l.stage.bounds;
    let cx = (b.min_x + b.max_x) / 2.0;
    let cz = (b.min_z + b.max_z) / 2.0;

    // Ambient sky fill: theme hemiSky (environment.js L3036 also has a
    // hemiGround bounce × hemiGroundK; M1 keeps the single ambient term).
    commands.insert_resource(GlobalAmbientLight {
        color: srgb_color(p.hemi_sky),
        brightness: 1500.0 * (p.hemi_intensity / 0.45).max(0.1),
        affects_lightmapped_meshes: false,
    });

    // Sun: direction from the theme's (sunAz, sunEl) — environment.js L2996:
    // uSunDir = (cos(el)·cos(az), sin(el), cos(el)·sin(az)). A Bevy
    // directional light shines along its transform's -Z, so -Z = the travel
    // direction (away from the sun, i.e. -sun_dir).
    let az = p.sun_az.to_radians();
    let el = p.sun_el.to_radians();
    let sun_dir = Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin()).normalize();
    commands.spawn((
        SunLight,
        DirectionalLight {
            color: srgb_color(p.sun_color),
            illuminance: 12000.0 * (p.sun_intensity / 3.3).max(0.1),
            shadow_maps_enabled: env.shadows,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            first_cascade_far_bound: 14.0,
            maximum_distance: 180.0,
            ..default()
        }
        .build(),
        Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, -sun_dir)),
    ));
    commands.insert_resource(DirectionalLightShadowMap {
        size: p.shadow_size as usize,
    });

    // Sky dome: inside-out gradient sphere (unlit).
    let sky_tex = images.add(sky_image(p));
    let mut dome = Batch::default();
    emit_sphere(&mut dome, Vec3::new(cx, 0.0, cz), 1200.0, 24, 16);
    let dome_mesh = meshes.add(to_mesh(&dome));
    let dome_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(sky_tex),
        unlit: true,
        cull_mode: Some(Face::Front),
        fog_enabled: false,
        ..default()
    });
    commands.spawn((Mesh3d(dome_mesh), MeshMaterial3d(dome_mat)));

    // Sea: translucent plane at water_y, well past the arena bounds.
    let sea_span = (b.max_x - b.min_x).max(b.max_z - b.min_z) * 4.0;
    let mut sea = Batch::default();
    emit_quad(
        &mut sea,
        Vec3::new(cx, l.stage.water_y, cz),
        sea_span,
        sea_span,
    );
    let sea_mesh = meshes.add(to_mesh(&sea));
    let sea_mat = materials.add(StandardMaterial {
        // theme seaDeep (#0a4f8a for day, #1a2c5e at dusk).
        base_color: Color::srgba(p.sea_deep[0], p.sea_deep[1], p.sea_deep[2], 0.78),
        perceptual_roughness: 0.12,
        alpha_mode: AlphaMode::Blend,
        fog_enabled: false, // upstream seaMat: fog false (environment.js L1774)
        ..default()
    });
    commands.spawn((Mesh3d(sea_mesh), MeshMaterial3d(sea_mat)));

    // Spawn barriers: translucent team-tinted cylinders at each pad
    // (decor.js `_buildPads`: pad group sits at the spawn point, barrier is a
    // 2.6 m open cylinder at local y = 1.3 → world centre pad.y + 1.3).
    let barrier_cols = [
        Color::srgba(1.0, 0.541, 0.078, 0.30), // Alpha  #ff8a14
        Color::srgba(0.184, 0.357, 1.0, 0.30), // Bravo  #2f5bff
    ];
    for (i, pad) in l.stage.spawn_pads.iter().enumerate() {
        let mut shell = Batch::default();
        emit_cylinder_shell(
            &mut shell,
            Vec3::new(pad[0], pad[1] + 1.3, pad[2]),
            l.stage.spawn_barrier,
            1.3,
            48,
        );
        let mesh = meshes.add(to_mesh(&shell));
        let mat = materials.add(StandardMaterial {
            base_color: barrier_cols[i % 2],
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            ..default()
        });
        commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat)));
    }

    // ---- camera ----
    // Fog: theme `fog near/far` + `horizon.lerp(skyMid, 0.15)`
    // (environment.js L3039-3040). Bloom: Bevy's built-in post-process
    // replaces the JS UnrealBloomPass for the tier's on/off gate; GTAO /
    // grade / SMAA / tilt-shift remain GPU-review items (D-T55-③).
    let cam = commands
        .spawn((
            Camera3d::default(),
            Projection::from(PerspectiveProjection {
                // cameraRig.js L188: vertical FOV from the horizontal 82° at
                // the fixed 16/9 reference; Bevy derives `aspect` from the window.
                fov: vfov(BASE_FOV_H, REF_ASPECT).to_radians(),
                near: CAM_NEAR,
                far: CAM_FAR,
                ..default()
            }),
            DistanceFog {
                color: srgb_color(p.fog_color),
                falloff: FogFalloff::Linear {
                    start: p.fog[0],
                    end: p.fog[1],
                },
                ..default()
            },
            Transform::from_translation(Vec3::new(0.0, 4.0, -50.0)),
        ))
        .id();
    if p.bloom {
        commands.entity(cam).insert(Bloom {
            intensity: p.grade.bloom[0],
            ..default()
        });
    }

    // TR-10.1 entity-count log (cross-check against layout meta).
    println!(
        "[inkwave] level build: primitives total={} visible={} hidden(collision-only)={} batches(draw-calls)={} meta_counts={:?}",
        l.primitives.len(),
        visible,
        hidden,
        draw_calls,
        l.meta.primitive_counts
    );
}

/// Task 55 runtime environment controls: `K` shadows, `L` quality tier,
/// `T` theme. Changes apply to the live render resources (clear colour,
/// ambient, sun, shadow-map size, fog, camera bloom). The sky dome texture
/// and already-spawned character rigs keep their boot values until the level
/// is rebuilt — D-T55-⑦ (the JS settings UI itself is M5 scope).
#[allow(clippy::too_many_arguments)]
fn environment_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<EnvState>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut sun: Query<&mut DirectionalLight>,
    mut fog: Query<&mut DistanceFog>,
    cameras: Query<Entity, With<Camera3d>>,
    mut commands: Commands,
) {
    let mut changed = false;
    if keys.just_pressed(KeyCode::KeyK) {
        state.shadows = !state.shadows;
        changed = true;
    }
    if keys.just_pressed(KeyCode::KeyL) {
        state.cycle_tier();
        changed = true;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        state.cycle_theme();
        changed = true;
    }
    if !changed {
        return;
    }
    let p = &state.params;
    clear.0 = srgb_color(p.horizon);
    ambient.color = srgb_color(p.hemi_sky);
    ambient.brightness = 1500.0 * (p.hemi_intensity / 0.45).max(0.1);
    shadow_map.size = p.shadow_size as usize;
    for mut l in &mut sun {
        l.color = srgb_color(p.sun_color);
        l.illuminance = 12000.0 * (p.sun_intensity / 3.3).max(0.1);
        l.shadow_maps_enabled = state.shadows;
    }
    for mut f in &mut fog {
        f.color = srgb_color(p.fog_color);
        f.falloff = FogFalloff::Linear {
            start: p.fog[0],
            end: p.fog[1],
        };
    }
    for e in &cameras {
        if p.bloom {
            commands.entity(e).insert(Bloom {
                intensity: p.grade.bloom[0],
                ..default()
            });
        } else {
            commands.entity(e).remove::<Bloom>();
        }
    }
    println!(
        "[inkwave] env: theme={} tier={} shadows={} bloom={} ao={} char_lod={} post={}",
        p.theme, p.tier, state.shadows, p.bloom, p.ao, p.char_tier, p.post_enabled
    );
}

/// Third-person follow of the local player (`demo.player_slot` — slot 0
/// offline, the net roster's local index in a net match, JS
/// `rig.follow(m.local)`): the pivot tracks the actor's visual position,
/// look comes from `PlayerControls`, and the boom shrinks when a wall blocks
/// the line of sight (cameraRig.js follow).
#[allow(clippy::too_many_arguments)] // Bevy system injection (Task 46 adds the shake read).
fn camera_rig(
    mut rig: ResMut<CamRig>,
    sim: Res<SimWorld>,
    pc: Res<crate::input::PlayerControls>,
    demo: Res<crate::ink_render::DemoSim>,
    settings: Res<crate::input::LookSettings>,
    shake: Res<crate::boss_render::BossShake>,
    time: Res<Time>,
    mut q: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let (yaw, pitch) = (pc.yaw, pc.pitch);
    // follow the player's feet + visual offset, damped (upstream pivot lerp)
    if let Some(a) = demo.m.actors.get(demo.player_slot) {
        let target = Vec3::from_array(a.pos.to_array()) + Vec3::Y * a.smooth_y();
        let k = 1.0 - (-18.0 * time.delta_secs()).exp();
        rig.pivot = rig.pivot.lerp(target, k);
    }

    let pivot = rig.pivot + Vec3::Y * PIVOT_H;
    let cp = pitch.cos();
    let look = Vec3::new(yaw.sin() * cp, pitch.sin(), yaw.cos() * cp);
    // Boom probe (cameraRig.js cameraProbe): shrink toward the pivot when a
    // wall blocks the line of sight; never past 0.45 m (L384). M1 approximates
    // the upstream 0.62 m multi-ray cylinder probe (physics.js L283-301) with
    // a single centre ray. sim uses glam 0.30, Bevy re-exports 0.32; bridge
    // via arrays.
    let hit = sim.0.raycast(
        glam::Vec3::from_array(pivot.to_array()),
        glam::Vec3::from_array((-look).to_array()),
        BOOM + 0.3,
        true,
    );
    rig.boom = boom_next(rig.boom, hit.hit, hit.dist, time.delta_secs());
    let mut cam_pos = pivot - look * rig.boom + Vec3::Y * 0.15;
    // Task 46 placeholder quake (JS boss.js L182 `emit('shake', {amount:
    // 0.9})` on defeat): high-frequency camera jitter scaled by the decaying
    // amount. Directional screenfx is Task 51.
    if shake.amount > 0.0 {
        let t = time.elapsed_secs();
        cam_pos += Vec3::new((t * 41.0).sin(), (t * 53.0).sin() * 0.7, (t * 37.0).cos())
            * (shake.amount * 0.08);
    }

    // FOV from the user setting (horizontal at the 16/9 reference,
    // cameraRig.js L188); Bevy recomputes `aspect` from the window each frame.
    for (mut tf, mut proj) in q.iter_mut() {
        tf.translation = cam_pos;
        // `look` is the view direction (camera sits at pivot - look·boom, so it
        // faces along +look); look_to points the camera -Z onto `look`.
        tf.look_to(Dir3::new_unchecked(look), Vec3::Y);
        if let Projection::Perspective(ref mut p) = *proj {
            p.fov = vfov(settings.fov_h, REF_ASPECT).to_radians();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwave_sim::geometry::BrushCommon;

    fn sample() -> StageLayout {
        inkwave_sim::embedded_tidewater()
    }

    fn base_common(l: &StageLayout) -> BrushCommon {
        l.primitives
            .iter()
            .find(|b| matches!(b, Brush::Box { .. }))
            .unwrap()
            .common()
            .clone()
    }

    fn box_brush(l: &StageLayout, min: [f32; 3], max: [f32; 3]) -> Brush {
        Brush::Box {
            min,
            max,
            common: base_common(l),
        }
    }

    #[test]
    fn frame_of_box_is_axis_aligned() {
        let l = sample();
        let f = frame_of(&box_brush(&l, [-2.0, 0.0, 1.0], [2.0, 3.0, 5.0]));
        assert_eq!(f.center, Vec3::new(0.0, 1.5, 3.0));
        assert_eq!(f.half, Vec3::new(2.0, 1.5, 2.0));
        assert_eq!(f.axes, [Vec3::X, Vec3::Y, Vec3::Z]);
    }

    #[test]
    fn frame_of_obox_rotates_axes_by_rot_y() {
        let l = sample();
        let b = Brush::Obox {
            center: [1.0, 2.0, 3.0],
            size: [4.0, 2.0, 2.0],
            rot_y: 90.0,
            common: base_common(&l),
        };
        let f = frame_of(&b);
        assert_eq!(f.center, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(f.half, Vec3::new(2.0, 1.0, 1.0));
        // axes = (c,0,-s), Y, (s,0,c) with c=0, s=1
        assert!(f.axes[0].distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-6);
        assert!(f.axes[2].distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-6);
        // right-handed: axes0 × axes1 = axes2
        assert!(f.axes[0].cross(f.axes[1]).dot(f.axes[2]) > 0.99);
    }

    #[test]
    fn frame_of_ramp_is_right_handed_and_spans_low_to_high() {
        let l = sample();
        let b = Brush::Ramp {
            low: [0.0, 0.0, 0.0],
            high: [0.0, 2.0, 4.0],
            width: 3.0,
            thickness: 0.3,
            thin: false,
            common: base_common(&l),
        };
        let f = frame_of(&b);
        // len = sqrt(20), ext = 0.6 → z half = (len+0.6)/2
        let len = (20.0f32).sqrt();
        assert!((f.half.z - (len + 0.6) / 2.0).abs() < 1e-5);
        assert!((f.half.x - 1.5).abs() < 1e-6);
        // frame right-handed after the flip guard
        assert!(f.axes[0].cross(f.axes[1]).dot(f.axes[2]) > 0.99);
        // axes[2] ≈ the low→high direction
        let d = Vec3::new(0.0, 2.0, 4.0).normalize();
        assert!(f.axes[2].dot(d) > 0.99);
    }

    #[test]
    fn emit_box_makes_24_verts_36_indices() {
        let l = sample();
        let mut batch = Batch::default();
        emit_box(
            &mut batch,
            &frame_of(&box_brush(&l, [-1.0, 0.0, 0.0], [1.0, 2.0, 1.0])),
            None,
        );
        assert_eq!(batch.positions.len(), 24);
        assert_eq!(batch.normals.len(), 24);
        assert_eq!(batch.uvs.len(), 24);
        assert_eq!(batch.indices.len(), 36);
        // outward normals: one per face axis pair
        for n in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ] {
            assert!(batch.normals.iter().any(|m| m[..] == n[..]));
        }
    }

    #[test]
    fn emit_box_uvs_are_face_metres_zero_to_size() {
        let l = sample();
        let mut batch = Batch::default();
        // 4.8 m box → face-local UV0 runs 0..4.8 on both axes (≡ JS faceUv,
        // pure metres; the shader divides by the uTL 1/scale table).
        emit_box(
            &mut batch,
            &frame_of(&box_brush(&l, [0.0, 0.0, 0.0], [4.8, 4.8, 4.8])),
            None,
        );
        let us: Vec<f32> = batch.uvs.iter().map(|u| u[0]).collect();
        assert!(us.contains(&0.0));
        assert!(us.iter().any(|v| (*v - 4.8).abs() < 1e-5));
        assert!(
            !us.iter().any(|v| *v < -1e-6),
            "UV0 must start at the face origin"
        );
    }

    #[test]
    fn pattern_slot_clamps_to_texlib_table() {
        // The layout pattern ≡ the texlib slot id (levelMaterial.js L79-85,
        // 50 slots; slot 49 = TL_SIDE).
        assert_eq!(pattern_slot(0), 0);
        assert_eq!(pattern_slot(29), 29); // herringbone slot
        assert_eq!(pattern_slot(30), 30);
        assert_eq!(pattern_slot(49), 49);
        assert_eq!(pattern_slot(99), 49);
    }

    #[test]
    fn hex_color_parses() {
        assert_eq!(
            hex_color("#ff8a14"),
            Color::srgb(1.0, 138.0 / 255.0, 20.0 / 255.0)
        );
        assert_eq!(
            hex_color("#2f5bff"),
            Color::srgb(47.0 / 255.0, 91.0 / 255.0, 1.0)
        );
    }

    #[test]
    fn boom_next_shrinks_on_wall_and_clamps() {
        let dt = 1.0 / 60.0;
        // No obstruction: eases toward the full boom length (slow-out rate 3.6).
        let b = boom_next(2.0, false, 99.0, dt);
        let a_out = 1.0 - (-3.6 * dt).exp();
        assert!((b - (2.0 + (BOOM - 2.0) * a_out)).abs() < 1e-5);
        // Wall 1.2 m behind the pivot: target = 1.2 - 0.3 = 0.9 m (fast-in 22).
        let b = boom_next(BOOM, true, 1.2, dt);
        let a_in = 1.0 - (-22.0 * dt).exp();
        assert!((b - (BOOM + (0.9 - BOOM) * a_in)).abs() < 1e-5);
        // Very close wall clamps the target at the 0.45 m minimum (L384).
        let b = boom_next(BOOM, true, 0.1, dt);
        assert!((b - (BOOM + (0.45 - BOOM) * a_in)).abs() < 1e-5);
    }

    #[test]
    fn emit_sphere_is_wound_outward_for_front_cull() {
        // The sky dome is rendered with `cull_mode: Front`, so its triangles
        // must be CCW seen from OUTSIDE (front faces point outward); a camera
        // inside then sees the back faces. Verify one equator triangle's
        // geometric normal points away from the centre.
        let mut batch = Batch::default();
        emit_sphere(&mut batch, Vec3::ZERO, 10.0, 24, 16);
        // Skip the degenerate pole quads; sample a mid-stack quad (i=5).
        let sectors = 24u32;
        let quad = 5 * sectors; // i=5
        let a = (quad * 4) as usize;
        let p0 = Vec3::from_array(batch.positions[a]);
        let p1 = Vec3::from_array(batch.positions[a + 2]);
        let p2 = Vec3::from_array(batch.positions[a + 1]);
        let geo = (p1 - p0).cross(p2 - p0).normalize();
        let outward = p0.normalize();
        assert!(
            geo.dot(outward) > 0.0,
            "sphere winding must face outward (dot={})",
            geo.dot(outward)
        );
    }

    #[test]
    fn vfov_matches_camera_rig_formula() {
        // cameraRig.js L188: v = 2*atan(tan(82/2 deg)/(16/9)) in deg ≈ 52.11
        let v = vfov(82.0, 16.0 / 9.0);
        assert!((v - 52.11).abs() < 0.05, "vfov={v}");
    }

    #[test]
    fn embedded_tidewater_batches_to_expected_keys() {
        let l = sample();
        let world = CollisionWorld::from_layout(&l);
        let atlas = InkAtlas::new(&world);
        let batches = build_level_batches(&l, &world, &atlas);
        // Every batch pattern is a clamped visible-primitive pattern.
        let visible_patterns: std::collections::HashSet<u16> = l
            .primitives
            .iter()
            .filter(|b| !b.common().hidden)
            .map(|b| pattern_slot(b.common().pattern))
            .collect();
        for (key, _) in &batches {
            assert!(
                visible_patterns.contains(&key.pattern),
                "pattern {}",
                key.pattern
            );
        }
        // TR-10.1 (re-keyed for D2): the visible primitives collapse into the
        // pinned (pattern slot × colour × orientation) batch count. The face
        // size lives in the key (the shader's edge-distance math needs the
        // exact fsz), so distinct box dimensions cannot share a batch —
        // tidewater's hand-sized geometry makes this far larger than the M1
        // 15 style×colour batches.
        assert_eq!(batches.len(), 1174, "batch count");
        let mut visible = 0u32;
        let mut hidden = 0u32;
        for b in &l.primitives {
            if b.common().hidden {
                hidden += 1;
            } else {
                visible += 1;
            }
        }
        // Total matches the layout meta primitive count; hidden are the
        // collision-only props culled from the render (level.js `_addBlock`).
        assert_eq!(
            visible + hidden,
            l.primitives.len() as u32,
            "TR-10.1: visible+hidden == total"
        );
        println!(
            "TR-10.1 counts: total={} visible={} hidden={} batches={}",
            l.primitives.len(),
            visible,
            hidden,
            batches.len()
        );
    }

    /// ≡ level.js `_buildFaces` L170-182 on the axis-aligned frame: the
    /// six-face (ui, vi, u, v) table pinned verbatim (JS source values as
    /// comments). Wall faces get v = world up, u = horizontal with the JS
    /// handedness (u × v = n); top/bottom get u = world X, v = world +Z.
    #[test]
    fn face_frame_matches_js_table() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        // JS _buildFaces for (k, sign) → (ui, vi, u, v):
        //   (+X) k0 s+1: vi=Y(1) ui=Z(2)  u=Y×X=-Z  v=+Y
        //   (-X) k0 s-1: vi=Y(1) ui=Z(2)  u=Y×-X=+Z  v=+Y
        //   (+Y) k1 s+1: ui=X(0) vi=Z(2)  u=Z×Y=-X  v=+Z
        //   (-Y) k1 s-1: ui=X(0) vi=Z(2)  u=Z×-Y=+X  v=+Z
        //   (+Z) k2 s+1: vi=Y(1) ui=X(0)  u=Y×Z=+X  v=+Y
        //   (-Z) k2 s-1: vi=Y(1) ui=X(0)  u=Y×-Z=-X  v=+Y
        let expect: [(usize, usize, Vec3, Vec3); 6] = [
            (2, 1, Vec3::NEG_Z, Vec3::Y),
            (2, 1, Vec3::Z, Vec3::Y),
            (0, 2, Vec3::NEG_X, Vec3::Z),
            (0, 2, Vec3::X, Vec3::Z),
            (0, 1, Vec3::X, Vec3::Y),
            (0, 1, Vec3::NEG_X, Vec3::Y),
        ];
        for (face, &(na, sign)) in FACES.iter().enumerate() {
            let (ui, vi, u, v) = face_frame(&axes, na, sign);
            let (eu, ev, eu_vec, ev_vec) = expect[face];
            assert_eq!((ui, vi), (eu, ev), "face {face} (ui, vi)");
            assert_eq!(u, eu_vec, "face {face} u (JS mirror handedness)");
            assert_eq!(v, ev_vec, "face {face} v");
            // wall faces: v ≡ world up exactly
            if matches!(face, 0 | 1 | 4 | 5) {
                assert_eq!(v, Vec3::Y, "face {face} wall v = world up");
                assert_eq!(u.y, 0.0, "face {face} wall u horizontal");
            }
            // JS L182: u × v = n
            assert_eq!(u.cross(v), axes[na] * sign, "face {face} handedness");
        }
    }

    /// One floating paintable box → six orientation batches: wall flags per
    /// face, wet ≡ paint-atlas rect (bottom face excluded), grounded false
    /// while airborne, and `u_tan` ≡ the JS signed face u.
    #[test]
    fn single_box_makes_six_orientation_batches() {
        let mut l = sample();
        let mut c = base_common(&l);
        c.pattern = 3;
        c.paint = true;
        c.color = "#abcdef".to_string();
        l.primitives = vec![Brush::Box {
            min: [0.0, 10.0, 0.0],
            max: [2.0, 13.0, 2.0],
            common: c,
        }];
        let world = CollisionWorld::from_layout(&l);
        let atlas = InkAtlas::new(&world);
        let batches = build_level_batches(&l, &world, &atlas);
        assert_eq!(batches.len(), 6, "one batch per box face");
        // JS _buildFaces (ui, vi) and the signed u per FACES index (see
        // `face_frame_matches_js_table`).
        let ui_of = [2usize, 2, 0, 0, 0, 0];
        let vi_of = [1usize, 1, 2, 2, 1, 1];
        let tan_of = [
            Vec3::NEG_Z,
            Vec3::Z,
            Vec3::NEG_X,
            Vec3::X,
            Vec3::X,
            Vec3::NEG_X,
        ];
        let half = [1.0f32, 1.5, 1.0]; // 2×3×2 box
        let mut seen = [false; 6];
        for (key, batch) in &batches {
            assert_eq!(key.pattern, 3);
            assert_eq!(key.color, "#abcdef");
            assert_eq!(batch.positions.len(), 4);
            let face = key.orient.face as usize;
            seen[face] = true;
            let o = &key.orient;
            let bp = key.batch_params();
            // wall faces: ±X (0,1) and ±Z (4,5)
            let wall = matches!(o.face, 0 | 1 | 4 | 5);
            assert_eq!(bp.flags_x, wall as u32 as f32, "face {face}");
            // airborne: no wall face stands on anything
            assert!(!o.grounded, "face {face}");
            // the -Y face (index 3) is not paintable → no atlas rect → dry;
            // the other five own one.
            assert_eq!(o.wet, o.face != 3, "face {face}");
            // tan ≡ the JS signed u (wall tangents stay horizontal)
            assert_eq!(bp.tan.truncate(), tan_of[face], "face {face}");
            assert_eq!(bp.tan.y, 0.0, "face {face} tan horizontal");
            // su/sv ≡ the JS face extents 2·h[ui] / 2·h[vi]
            assert_eq!(bp.fsz.x, 2.0 * half[ui_of[face]], "face {face}");
            assert_eq!(bp.fsz.y, 2.0 * half[vi_of[face]], "face {face}");
        }
        assert!(seen.iter().all(|s| *s), "all six faces batched");
    }

    /// A box resting on the floor plane grounds its four wall faces
    /// (≡ JS groundedBottom probe L198-201); the top face stays ungrounded.
    #[test]
    fn grounded_probe_fires_for_walls_at_the_floor() {
        let mut l = sample();
        let mut c = base_common(&l);
        c.pattern = 0;
        c.paint = true;
        c.color = "#ffffff".to_string();
        l.primitives = vec![Brush::Box {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 3.0, 2.0],
            common: c,
        }];
        let world = CollisionWorld::from_layout(&l);
        let atlas = InkAtlas::new(&world);
        let batches = build_level_batches(&l, &world, &atlas);
        let grounded: Vec<u8> = batches
            .iter()
            .filter(|(k, _)| k.orient.grounded)
            .map(|(k, _)| k.orient.face)
            .collect();
        assert_eq!(
            grounded,
            vec![0, 1, 4, 5],
            "wall faces (±X, ±Z) grounded at y≈-0.06 < 0.02"
        );
    }

    /// Every embedded batch's `BatchParams` re-derives identically from its
    /// key (pattern / wet gate / flags / face size / mur sentinel).
    #[test]
    fn batch_params_rederive_from_keys() {
        let l = sample();
        let world = CollisionWorld::from_layout(&l);
        let atlas = InkAtlas::new(&world);
        let batches = build_level_batches(&l, &world, &atlas);
        assert!(!batches.is_empty());
        for (key, _) in &batches {
            let bp = key.batch_params();
            assert_eq!(bp.pattern, key.pattern as f32);
            assert_eq!(bp.wet_base, key.orient.wet as u32 as f32);
            assert_eq!(bp.flags_y, key.orient.grounded as u32 as f32);
            assert_eq!(bp.mur, -1.0);
            assert_eq!(bp.fsz.x, f32::from_bits(key.orient.su));
            assert_eq!(bp.fsz.y, f32::from_bits(key.orient.sv));
            assert!(bp.fsz.x > 0.0 && bp.fsz.y > 0.0, "face sizes positive");
            // flags_x ≡ the wall test re-derived from the quantised normal;
            // tan ≡ the JS signed face u re-derived from the key.
            let (na, sign) = FACES[key.orient.face as usize];
            let axes = key.axes();
            let normal_y = axes[na].y * sign;
            let (_ui, _vi, u, _v) = face_frame(&axes, na, sign);
            assert_eq!(bp.flags_x, (normal_y.abs() < 0.3) as u32 as f32);
            assert_eq!(bp.tan.truncate(), u);
        }
    }
}
