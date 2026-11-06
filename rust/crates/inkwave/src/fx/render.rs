//! Task 51 batch C — the Bevy renderer over the FX data core.
//!
//! JS drives the pools through three GPU impostor materials (`fx.js`
//! L663-707: instanced billboards + the ring/shell/beam/mesh planes + the
//! additive glow pass + the `fx.wgsl`/`ink.wgsl`-adjacent shaders). The Rust
//! placeholder tier rebuilds one batched quad mesh per pool every frame from
//! the authoritative pool data (`Fx::drops` … `Fx::pillars`) and uploads it
//! through the same `Mesh` + `StandardMaterial` path as the boss renderer
//! (`boss_render.rs` L169-176 precedent).
//!
//! Declared placeholder simplifications (vs the JS shader tiers):
//!   * one unlit vertex-coloured blended quad per particle — no per-pool
//!     GLSL impostor shading (sphere / streak / bubble / ghost / feather
//!     shapes), no soft-particle depth fade, no additive glow pass; the
//!     GLSL precision question stays deferred per D-T51-7②;
//!   * billboards use the camera right/up basis; rings / marks / storm
//!     puddles use the entry normal's orthonormal plane basis; beams /
//!     pillars stretch along their `up` axis camera-facing;
//!   * alpha is the pool's own `alpha` × the JS fade-in / fade-out ramps
//!     where the data plane carries them (sprites / rings / marks);
//!   * the GPU dust motes (`fx.js` L1152 `motes` — a pure shader particle
//!     field with no CPU positions) are **not** drawn here: the data plane
//!     carries nothing to iterate, so the motes surface is deferred to the
//!     batch-E shader work (declared gap).
//!
//! The mesh entities are spawned lazily on the first frame with a camera
//! present; headless play never builds `WorldPlugin`, so nothing spawns.

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
// The pools store glam 0.30 vectors (the sim's version); Bevy's prelude glob
// would otherwise shadow them with glam 0.32. Bridge at the camera edge.
use glam::Vec3;

use super::FxState;
use super::particles::{Fx, Sprite, basis, ease_out};

/// pool slot order for the entity / handle arrays.
const POOLS: [&str; 8] = [
    "fx_drops",
    "fx_puffs",
    "fx_glows",
    "fx_rings",
    "fx_shells",
    "fx_beams",
    "fx_marks",
    "fx_pillars",
];

/// The renderer-side state: one batched mesh entity per pool plus the
/// shared unlit material.
#[derive(Resource, Default)]
pub struct FxRenderState {
    entities: Option<[Entity; 8]>,
    handles: Vec<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

impl FxRenderState {
    fn ensure_entities(
        &mut self,
        commands: &mut Commands,
        meshes: &mut ResMut<Assets<Mesh>>,
        materials: &mut ResMut<Assets<StandardMaterial>>,
    ) {
        if self.entities.is_some() {
            return;
        }
        let material = self.material.get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                ..default()
            })
        });
        self.handles.clear();
        let mut ids = [Entity::PLACEHOLDER; 8];
        for (slot, _name) in POOLS.iter().enumerate() {
            let handle = meshes.add(empty_mesh());
            let entity = commands
                .spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material.clone()),
                    Visibility::default(),
                    Transform::default(),
                ))
                .id();
            ids[slot] = entity;
            self.handles.push(handle);
        }
        self.entities = Some(ids);
    }

    /// Rebuild one pool's batched quad mesh from the data plane.
    fn upload(&mut self, fx: &Fx, cam: Vec3, right: Vec3, up: Vec3, meshes: &mut Assets<Mesh>) {
        if self.entities.is_none() {
            return;
        }
        let mut buf = QuadBuf::new();
        // ---- droplets ≡ the JS drop impostor billboards (fx.js L726-865).
        // Streak stretch is a shader effect the placeholder tier skips.
        for d in &fx.drops {
            let s = d.size * 1.5;
            buf.quad(d.pos, right, up, s, s, d.col, 1.0);
        }
        self.write(0, &buf, meshes);

        // ---- puffs & glows ≡ the sprite pools (L867-935): size rides the
        // size0→size1 curve over the JS ease-out, alpha the fade ramps.
        buf.clear();
        for s in &fx.puffs {
            sprite_quad(&mut buf, s, right, up);
        }
        self.write(1, &buf, meshes);
        buf.clear();
        for s in &fx.glows {
            sprite_quad(&mut buf, s, right, up);
        }
        self.write(2, &buf, meshes);

        // ---- rings ≡ the ring plane (L938-962): a flat quad in the entry
        // normal's plane, radius grow0→radius over the ease-out.
        buf.clear();
        for r in &fx.rings {
            let t = (r.age / r.life.max(1e-4)).clamp(0.0, 1.0);
            let rad = r.grow0 + (r.radius - r.grow0) * ease_out(t);
            let (ax, ay) = basis(r.normal);
            let fade = ring_alpha(r);
            buf.quad(r.pos, ax, ay, rad, rad, r.col, r.alpha * fade);
        }
        self.write(3, &buf, meshes);

        // ---- shells ≡ the ink shells (L1001+): billboard at the expanded
        // radius.
        buf.clear();
        for s in &fx.shells {
            let rad = s.r0 + (s.r1 - s.r0) * s.expand.clamp(0.0, 1.0);
            let t = (s.age / s.life.max(1e-4)).clamp(0.0, 1.0);
            buf.quad(s.pos, right, up, rad, rad, s.col, s.alpha * (1.0 - t));
        }
        self.write(4, &buf, meshes);

        // ---- beams ≡ the charger beam columns (L1080+): camera-facing
        // quads stretched along Y (the Rust `Beam` entry carries no `up`
        // axis — every spawn API column is vertical; deviation).
        buf.clear();
        for b in &fx.beams {
            let t = (b.age / b.life.max(1e-4)).clamp(0.0, 1.0);
            let c = b.pos + Vec3::Y * (b.height * 0.5);
            let side = Vec3::Y.cross(cam - c);
            let side = if side.length_squared() < 1e-6 {
                right
            } else {
                side.normalize()
            };
            buf.quad(
                c,
                side,
                Vec3::Y,
                b.radius * 2.0,
                b.height * 0.5,
                b.col,
                1.0 - t,
            );
        }
        self.write(5, &buf, meshes);

        // ---- marks ≡ the immediate marks (L966-969): flat quads in the
        // normal plane; `Mark` carries no age/life — its `t` is the pool's
        // own fade parameter.
        buf.clear();
        for m in &fx.marks {
            let (ax, ay) = basis(m.normal);
            let fade = 1.0 - m.t.clamp(0.0, 1.0);
            buf.quad(m.pos, ax, ay, m.radius, m.radius, m.col, m.alpha * fade);
        }
        self.write(6, &buf, meshes);

        // ---- pillars ≡ the ink pillars (L1140+); `Pillar` is a live
        // column whose `alpha` is driven by the pool owner each frame.
        buf.clear();
        for p in &fx.pillars {
            let c = p.pos + Vec3::Y * (p.height * 0.5);
            let side = Vec3::Y.cross(cam - c);
            let side = if side.length_squared() < 1e-6 {
                right
            } else {
                side.normalize()
            };
            buf.quad(
                c,
                side,
                Vec3::Y,
                p.radius * 2.0,
                p.height * 0.5,
                p.col,
                p.alpha,
            );
        }
        self.write(7, &buf, meshes);
        // motes (GPU dust, fx.js L1152): no CPU positions in the data plane
        // → deferred to batch E (declared gap, see module docs).
    }

    fn write(&self, pool: usize, buf: &QuadBuf, meshes: &mut Assets<Mesh>) {
        let Some(handle) = self.handles.get(pool) else {
            return;
        };
        if let Some(mut mesh) = meshes.get_mut(handle) {
            *mesh = buf.build();
        }
    }
}

/// Sprite fade shape ≡ the JS sprite shader's fade-in / fade-out ramps
/// (fx.js L884-894 X-layout).
fn sprite_alpha(s: &Sprite) -> f32 {
    let fin = (s.age / s.fade_in.max(1e-4)).min(1.0);
    let fout = ((s.life - s.age) / s.fade_out.max(1e-4)).min(1.0);
    s.alpha * fin.max(0.0) * fout.max(0.0)
}

fn ring_alpha(r: &super::particles::Ring) -> f32 {
    1.0 - (r.age / r.life.max(1e-4)).clamp(0.0, 1.0)
}

fn sprite_quad(buf: &mut QuadBuf, s: &Sprite, right: Vec3, up: Vec3) {
    let t = (s.age / s.life.max(1e-4)).clamp(0.0, 1.0);
    let size = s.size0 + (s.size1 - s.size0) * ease_out(t);
    buf.quad(s.pos, right, up, size, size, s.col, sprite_alpha(s));
}

/// Accumulates camera-aligned quads into one batched triangle-list mesh.
struct QuadBuf {
    pos: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl QuadBuf {
    fn new() -> Self {
        Self {
            pos: Vec::new(),
            col: Vec::new(),
            idx: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.pos.clear();
        self.col.clear();
        self.idx.clear();
    }

    /// Push a centered quad spanned by `ax`/`ay` with half-extents `hw`/`hh`.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors the vertex writer
    fn quad(&mut self, c: Vec3, ax: Vec3, ay: Vec3, hw: f32, hh: f32, col: Vec3, alpha: f32) {
        if !(hw.is_finite() && hh.is_finite() && alpha.is_finite()) {
            return;
        }
        let base = self.pos.len() as u32;
        let corners = [
            c - ax * hw - ay * hh,
            c + ax * hw - ay * hh,
            c + ax * hw + ay * hh,
            c - ax * hw + ay * hh,
        ];
        for v in corners.iter() {
            self.pos.push(v.to_array());
            self.col.push([col.x, col.y, col.z, alpha]);
        }
        self.idx
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn build(&self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
        let normals: Vec<[f32; 3]> = if self.pos.is_empty() {
            Vec::new()
        } else {
            // one normal per vertex — the unlit material ignores it; the
            // attribute exists to satisfy the standard mesh layout.
            vec![[0.0, 0.0, 1.0]; self.pos.len()]
        };
        let uvs: Vec<[f32; 2]> = if self.pos.is_empty() {
            Vec::new()
        } else {
            let mut v = Vec::with_capacity(self.pos.len());
            for _ in 0..self.pos.len() / 4 {
                v.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
            }
            v
        };
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col.clone());
        mesh.insert_indices(Indices::U32(self.idx.clone()));
        mesh
    }
}

fn empty_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// The frame driver ≡ the JS `fx.update(dt, camera)` call in the main loop,
/// plus the per-frame instance-buffer rebuild the JS GPU upload does. Runs
/// at the tail of the Update chain (after `camera_rig`, so the camera basis
/// is current).
pub fn fx_update(
    time: Res<Time>,
    cams: Query<&GlobalTransform, With<Camera>>,
    mut fs: ResMut<FxState>,
    mut rs: ResMut<FxRenderState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();
    let cam = match cams.single().ok() {
        Some(g) => {
            let (_s, r, t) = g.to_scale_rotation_translation();
            Some((
                glam::Vec3::from_array(t.to_array()),
                glam::Vec3::from_array((r * bevy::math::Vec3::X).to_array()),
                glam::Vec3::from_array((r * bevy::math::Vec3::Y).to_array()),
            ))
        }
        None => None,
    };
    // ≡ FX.update (fx.js L2206-2261) — pool physics, scheduler, culling.
    fs.fx.update(dt, cam.as_ref().map(|c| c.0));
    if cam.is_some() {
        rs.ensure_entities(&mut commands, &mut meshes, &mut materials);
        if let Some((pos, right, up)) = cam {
            rs.upload(&fs.fx, pos, right, up, &mut meshes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadbuf_builds_a_valid_indexed_batch() {
        let mut buf = QuadBuf::new();
        buf.quad(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            1.0,
            1.0,
            Vec3::new(1.0, 0.5, 0.0),
            0.8,
        );
        buf.quad(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::X,
            Vec3::Y,
            0.5,
            0.5,
            Vec3::new(0.0, 0.4, 1.0),
            1.0,
        );
        let mesh = buf.build();
        assert_eq!(mesh.count_vertices(), 8);
        assert_eq!(mesh.indices().unwrap().len(), 12);
        // degenerate quads (NaN) are dropped, not uploaded
        let mut bad = QuadBuf::new();
        bad.quad(Vec3::ZERO, Vec3::X, Vec3::Y, f32::NAN, 1.0, Vec3::ZERO, 1.0);
        assert_eq!(bad.build().count_vertices(), 0);
    }

    #[test]
    fn sprite_alpha_ramps_in_and_out() {
        let mut s = Sprite {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            col: Vec3::ONE,
            size0: 0.1,
            size1: 1.0,
            age: 0.0,
            life: 1.0,
            alpha: 0.9,
            rot: 0.0,
            rot_vel: 0.0,
            drag: 0.0,
            buoy: 0.0,
            kind: 0.0,
            fade_in: 0.1,
            seed: 0.0,
            wobble: 0.0,
            fade_out: 0.2,
        };
        assert!(sprite_alpha(&s) < 1e-3, "faded in at birth");
        s.age = 0.5;
        assert!((sprite_alpha(&s) - 0.9).abs() < 1e-4, "full alpha mid-life");
        s.age = 0.95;
        assert!(sprite_alpha(&s) < 0.3, "fading out near death");
    }
}
