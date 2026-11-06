//! Task 53 (D-T53-2) — the props runtime: decode the `IKP1` archive for the
//! current stage and spawn the set dressing (≡ `PropKit.build()` +
//! `Decor`, props.js L3199-3241 / decor.js).
//!
//! Layout follows the boss_render precedent (a module directory with a
//! sibling `archive.rs`). The archive carries welded, quantised geometry
//! templates + per-placement instances; here every template becomes one
//! `Mesh` (POSITION/NORMAL/COLOR_0) and every bucket one `StandardMaterial`
//! (≡ props.js L3129-3141 — the canvas atlas is not ported, so `paint`/
//! `gloss`/`fence` lose their `map` and `blob` is not drawn at all; clearcoat
//! is approximated as roughness ≡ D-T52-7②). Instances spawn as entities
//! sharing the template mesh + bucket material (D-T53-2: shared mesh handle,
//! per-instance transform), filtered by the match mode exactly like
//! main.js L235 (`inMode`). The Low quality tier skips dressing entirely —
//! a Rust-side decision (spec D-T53-2): JS main.js L228-241 has no such gate,
//! it only passes `quality` into `PropKit` (geometry segments).
//!
//! Spinners and blink lights spawn as static records in this batch; their
//! per-frame animation (≡ props.js update L3265-3300) and the `uLamps`
//! upload ride `anim.rs` (batch C, D-T53-3).
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

pub mod anim;
pub mod archive;
pub mod mapthumb;
pub mod special;
#[cfg(test)]
mod tests;

use crate::quality::tier_from_env;
use crate::world::LayoutRes;
use archive::{Archive, Template, load_stage_archive, mode_passes, stage_manifest};

/// The match variant the app currently plays (the lobby picker is Task 57;
/// until then the session is home turf, ≡ DEFAULT_SETTINGS.mode).
pub const PROPS_MODE: &str = "turf";

/// Marker: a static prop / decor instance entity (never animated).
#[derive(Component)]
pub struct PropInstance;

/// The bucket → material table (≡ props.js L3129-3141; the `ns` flag has no
/// per-entity shadow API in Bevy 0.19 and is accepted but ignored).
fn bucket_material(name: &str, _ns: bool) -> StandardMaterial {
    match name {
        // MeshStandardMaterial { map: atlas } — map not ported (D-T53-6②):
        // pure vertex colour on the same roughness/metalness.
        "paint" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.6,
            metallic: 0.0,
            ..default()
        },
        // clearcoat 0.65 approximated as a lower roughness (D-T52-7②).
        "gloss" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.36,
            metallic: 0.0,
            ..default()
        },
        "metal" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.33,
            metallic: 0.7,
            ..default()
        },
        "wood" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.74,
            metallic: 0.0,
            ..default()
        },
        "rubber" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.86,
            metallic: 0.0,
            ..default()
        },
        // side: DoubleSide (props.js L3135).
        "foliage" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.7,
            metallic: 0.0,
            double_sided: true,
            ..default()
        },
        // MeshBasicMaterial unlit — Bevy 0.19 `unlit` flag is the exact
        // equivalent (base colour × vertex colour, no lighting).
        "glow" => StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        },
        // chain-texture alphaTest not ported: plain double-sided metal.
        "fence" => StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.4,
            metallic: 0.5,
            double_sided: true,
            ..default()
        },
        // decor lamp glass (decor.js bulbMat emissive) — emissive shade.
        "glass" => StandardMaterial {
            base_color: Color::WHITE,
            emissive: LinearRgba::new(0.55, 0.42, 0.2, 1.0),
            perceptual_roughness: 0.3,
            metallic: 0.0,
            ..default()
        },
        _ => StandardMaterial::from(Color::srgb(0.8, 0.8, 0.8)),
    }
}

/// One template → one mesh (POSITION/NORMAL/COLOR_0 + u32 indices).
fn template_mesh(t: &Template) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, t.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, t.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, t.colors.clone());
    mesh.insert_indices(Indices::U32(t.indices.clone()));
    mesh
}

/// The blink bulb geometry: `G.sph(1, 8, 6, false)` (props.js L307) — a unit
/// UV sphere, 8 width × 6 height segments, per-instance scale from the record.
fn blink_sphere() -> Mesh {
    let (positions, normals, uvs, indices) = blink_sphere_data();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Vertex/index tables split out so the winding test (review P1-2) can read
/// them without the Bevy attribute API.
#[allow(clippy::type_complexity)]
fn blink_sphere_data() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    const W: usize = 8; // width segments
    const H: usize = 6; // height segments
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((W + 1) * (H + 1));
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(positions.len());
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(positions.len());
    for iy in 0..=H {
        let v = iy as f32 / H as f32;
        let theta = v * std::f32::consts::PI;
        for ix in 0..=W {
            let u = ix as f32 / W as f32;
            let phi = u * std::f32::consts::TAU;
            let x = -phi.sin() * theta.sin();
            let y = theta.cos();
            let z = phi.cos() * theta.sin();
            positions.push([x, y, z]);
            normals.push([x, y, z]);
            uvs.push([u, 1.0 - v]);
        }
    }
    let mut indices: Vec<u32> = Vec::with_capacity(W * H * 6);
    for iy in 0..H {
        for ix in 0..W {
            let a = (iy * (W + 1) + ix) as u32;
            let b = a + 1;
            let c = a + W as u32 + 1;
            let d = c + 1;
            // Winding: our u-parameterisation (x=-sinφ, z=+cosφ) is a MIRROR
            // of Three SphereGeometry's (x=-cosφ, z=+sinφ) — ix increases in
            // the negative-φ direction — so Three's outward (a,b,d)/(b,c,d)
            // pattern does NOT port by renaming (review's suggested (b,a,d)/
            // (a,c,d) measures signed volume -3.51861 = inward). The outward
            // diagonals for our vertex layout are (a,b,d)/(b,d,c), pinned by
            // the `blink_sphere_winds_outward` test (≡ +3.51861).
            if iy != 0 {
                indices.extend([a, b, d]);
            }
            if iy != H - 1 {
                indices.extend([b, d, c]);
            }
        }
    }
    (positions, normals, uvs, indices)
}

/// Build a Bevy `Transform` from an instance record (≡ `Builder.begin`:
/// compose(pos, rotY, scale³)).
fn instance_transform(pos: [f32; 3], rot_y: f32, scale: f32) -> Transform {
    Transform {
        translation: Vec3::from(pos),
        rotation: Quat::from_rotation_y(rot_y),
        scale: Vec3::splat(scale),
    }
}

/// Runtime summary of the loaded archive (smoke + cross-check anchor).
#[derive(Resource, Default)]
pub struct PropsState {
    /// The stage id the archive was loaded for (empty = nothing loaded).
    pub stage: String,
    pub instances: usize,
    pub spins: usize,
    pub blinks: usize,
    pub decor: usize,
    /// Final `uLamps` world positions (manifest `lamps` ≡ setLevelLamps,
    /// levelMaterial.js L28-36) — consumed by `level_render` (D-T53-3).
    pub lamps: Vec<[f32; 3]>,
    /// Night factor `k` (≡ `setLevelLamps`'s `uNight`): 0 = day/golden
    /// default theme (lamps off), 1 = sunset. The theme picker (Task 55/57)
    /// sets this; until then the session runs the JS default `'day'`.
    pub night_k: f32,
    /// Mural placement rows for the current stage (D-T53-5).
    pub murals: Vec<archive::MuralRow>,
}

/// Archive → (meshes, materials) + the spawn of every mode-passing instance.
/// Returns the counts actually spawned (smoke assertion + `PropsState`).
pub struct BuiltProps {
    pub instances: usize,
    pub spins: usize,
    pub blinks: usize,
    pub decor: usize,
}

/// Spawn the whole archive (instances + decor + spin/blink records).
pub fn spawn_archive(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    archive: &Archive,
    mode: &str,
    sphere: Handle<Mesh>,
) -> BuiltProps {
    let mut mesh_of: Vec<Handle<Mesh>> = Vec::with_capacity(archive.templates.len());
    for t in &archive.templates {
        mesh_of.push(meshes.add(template_mesh(t)));
    }
    let mut mat_of: Vec<Handle<StandardMaterial>> = Vec::with_capacity(archive.buckets.len());
    for b in &archive.buckets {
        mat_of.push(materials.add(bucket_material(&b.name, b.ns)));
    }

    let mut built = BuiltProps {
        instances: 0,
        spins: 0,
        blinks: 0,
        decor: 0,
    };
    // NOTE: the `~ns` buckets (castShadow=false in JS) have no per-entity
    // shadow control in Bevy 0.19 — they cast anyway (declared visual
    // approximation, REVIEW_CHECKLIST).
    for inst in &archive.instances {
        if !mode_passes(inst.mode_flags, mode) {
            continue;
        }
        built.instances += 1;
        commands.spawn((
            Mesh3d(mesh_of[inst.template as usize].clone()),
            MeshMaterial3d(mat_of[inst.bucket as usize].clone()),
            instance_transform(inst.pos, inst.rot_y, inst.scale),
            PropInstance,
        ));
    }
    for d in &archive.decor {
        built.decor += 1;
        commands.spawn((
            Mesh3d(mesh_of[d.template as usize].clone()),
            MeshMaterial3d(mat_of[d.bucket as usize].clone()),
            Transform::from_translation(Vec3::from(d.pos)).with_scale(Vec3::splat(d.scale)),
            PropInstance,
        ));
    }
    // Spinners + blink lights: animated by `anim.rs` (D-T53-3) every frame.
    for s in &archive.spins {
        built.spins += 1;
        commands.spawn((
            Mesh3d(mesh_of[s.template as usize].clone()),
            MeshMaterial3d(mat_of[s.bucket as usize].clone()),
            anim::SpinEntity::from_record(*s),
            PropInstance,
        ));
    }
    for b in &archive.blinks {
        built.blinks += 1;
        // JS draws bulbs with an unlit MeshBasicMaterial whose colour pulses
        // (props.js L3137) — one `unlit` StandardMaterial per bulb, animated
        // by `anim::props_blink_anim`.
        let mat = materials.add(StandardMaterial {
            base_color: Color::linear_rgb(b.color[0], b.color[1], b.color[2]),
            unlit: true,
            ..default()
        });
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(Vec3::from(b.pos)).with_scale(Vec3::splat(b.scale)),
            anim::BlinkEntity::from_record(*b),
        ));
    }
    built
}

/// Startup: load + decode + spawn the current stage's archive. Any missing
/// file / unknown stage / Low tier degrades silently (≡ main.js L237-242).
fn props_startup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    layout: Res<LayoutRes>,
    mut state: ResMut<PropsState>,
) {
    if tier_from_env() == crate::quality::QualityTier::Low {
        return; // main.js L229: Low generates no props
    }
    let stage = layout.0.stage.id.as_str();
    let Some(manifest) = stage_manifest(stage) else {
        return; // unknown stage id: nothing to load
    };
    let Some(archive) = load_stage_archive(stage) else {
        return; // archive absent (e.g. wasm without the copy-dir fetch): silent skip
    };
    let sphere = meshes.add(blink_sphere());
    let built = spawn_archive(
        &mut commands,
        &mut meshes,
        &mut materials,
        &archive,
        PROPS_MODE,
        sphere,
    );
    state.stage = stage.to_string();
    state.instances = built.instances;
    state.spins = built.spins;
    state.blinks = built.blinks;
    state.decor = built.decor;
    state.lamps = manifest.lamps.clone();
    state.murals = manifest.murals.clone();
    info!(
        "[props] stage={stage} instances={}/{}/{}/{} (spins/blinks/decor) lamps={} murals={}",
        built.instances,
        built.spins,
        built.blinks,
        built.decor,
        manifest.lamps.len(),
        manifest.murals.len()
    );
}

/// The props plugin (added after `WorldPlugin` in main.rs).
pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropsState>()
            .add_systems(Startup, props_startup)
            .add_systems(
                Update,
                (anim::props_spin_anim, anim::props_blink_anim).chain(),
            );
    }
}
