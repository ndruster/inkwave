//! Task 10 — Bevy level-world rendering for the Rust port.
//!
//! Turns an [`inkwave_sim::geometry::StageLayout`] into batched render meshes
//! (one draw call per pattern-style × block colour), plus the daytime
//! environment (gradient sky dome, sun with toggleable cascaded shadows,
//! translucent sea at `water_y`, team-tinted spawn-barrier cylinders) and a
//! third-person camera rig with mouse look and collision boom-shrink.
//!
//! Faithful to upstream `src/world/level.js` (`_addBlock` block frames and the
//! `hidden` cull rule) and `src/world/environment.js` (day theme sun/sky
//! colours). M1 approximates the texlib uber-shaders with solid block colours
//! plus simple procedural stripe/grid textures.

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

use crate::ink_render::{InkAtlasRes, InkExt, InkImage, InkMaterial};

/// Sky clear colour matching `renderer.js` setClearColor (0x9fd8f0).
const SKY: Color = Color::srgb(0.624, 0.847, 0.941);
/// Fog colour: `horizon.lerp(skyMid, 0.15)` (environment.js L3039; the raw
/// horizon #d4ecfa itself only appears in the sky gradient, see `sky_image`).
const FOG_COLOR: Color = Color::srgb(0.760, 0.886, 0.976);
/// Day fog range (environment.js L52 `fog: [25, 900]`).
const FOG_NEAR: f32 = 25.0;
const FOG_FAR: f32 = 900.0;
/// Sun colour (`environment.js` day `sunColor` #fff0dc).
const SUN_COLOR: Color = Color::srgb(1.0, 0.941, 0.863);
/// Sun azimuth / elevation (deg) for the day theme (`environment.js` L41).
const SUN_AZ: f32 = 222.0;
const SUN_EL: f32 = 39.0;
/// Horizontal field of view (settings.fov default, cameraRig.js L180).
/// cameraRig.js L188 converts it with the FIXED 16/9 reference aspect,
/// independent of the window size; [`REF_ASPECT`] is that constant.
pub(crate) const BASE_FOV_H: f32 = 82.0;
const REF_ASPECT: f32 = 16.0 / 9.0;
/// Default boom length (cameraRig.js `dist`).
const BOOM: f32 = 4.5;
/// Pivot height above the feet (cameraRig.js `hgt`).
const PIVOT_H: f32 = 1.85;
/// Texlib tile scale in metres (`surfaces.js` scale: 2.4).
const TEX_SCALE: f32 = 2.4;

/// Procedural texture styles the M1 material set approximates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Style {
    Flat,
    Concrete,
    Plank,
    Tile,
    Brick,
    Ashlar,
    Metal,
    Hazard,
    Grate,
}

/// Map an upstream PATTERN id (mapkit.js L9-13) to an M1 texture style.
fn style_for(pattern: u16) -> Style {
    match pattern {
        0 | 8 => Style::Flat,
        3 | 9 | 10 | 14 | 18 | 19 | 20 | 21 | 25 => Style::Concrete,
        1 | 6 | 17 | 22 | 24 | 26 | 27 => Style::Plank,
        2 | 15 | 29 => Style::Tile,
        13 | 16 | 28 => Style::Brick,
        23 | 30 => Style::Ashlar,
        5 | 7 | 11 => Style::Metal,
        4 => Style::Hazard,
        12 => Style::Grate,
        _ => Style::Flat,
    }
}

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

/// Append one oriented box (24 verts / 36 tris) to `batch`. UVs are projected
/// from the world position onto the face's tangent axes (world metres /
/// [`TEX_SCALE`]).
///
/// With `ink = Some((world, atlas, brush_index))` the block's faces are looked
/// up through the collision world (`blocks[bid].faces[axis * 2 + sign]`, the
/// same table as [`FACES`] below) and each vertex gets UV1 = its position in
/// that face's atlas rect; non-paintable / culled faces get `(0, 0)`.
fn emit_box(batch: &mut Batch, f: &Frame, ink: Option<(&CollisionWorld, &InkAtlas, usize)>) {
    // (local normal axis, sign, u-axis, v-axis) chosen so that in the
    // right-handed frame (axes[0] × axes[1] = axes[2]) the tangent cross
    // axes[ua] × axes[va] equals the outward normal sign·axes[na] for every
    // face (negative faces swap u/v) — then the CCW quad order below always
    // faces outward.
    const FACES: [(usize, f32, usize, usize); 6] = [
        (0, 1.0, 1, 2),
        (0, -1.0, 2, 1),
        (1, 1.0, 2, 0),
        (1, -1.0, 0, 2),
        (2, 1.0, 0, 1),
        (2, -1.0, 1, 0),
    ];
    for (na, sign, ua, va) in FACES {
        let normal = f.axes[na] * sign;
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
        for (cu, cv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let mut local = Vec3::ZERO;
            local[na] = sign * f.half[na];
            local[ua] = cu * f.half[ua];
            local[va] = cv * f.half[va];
            let world = f.center + f.axes[0] * local.x + f.axes[1] * local.y + f.axes[2] * local.z;
            batch.positions.push(world.to_array());
            batch.normals.push(normal.to_array());
            let du = world - f.center;
            batch.uvs.push([
                du.dot(f.axes[ua]) / TEX_SCALE,
                du.dot(f.axes[va]) / TEX_SCALE,
            ]);
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

/// Generate a small procedural tile texture (grayscale; tinted by base colour).
fn pattern_image(style: Style) -> Image {
    const N: u32 = 64;
    let mut px = vec![255u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let v: u8 = match style {
                Style::Flat => 255,
                Style::Concrete => 236 + ((x * 7 + y * 13) % 5) as u8 * 3,
                Style::Plank => {
                    let seam = (y % 16 == 0) as u8;
                    let grain = ((x * 3 + y) % 11) as u8;
                    235 - seam * 45 - grain * 2
                }
                Style::Tile => {
                    let grout = (x % 32 == 0 || y % 32 == 0) as u8;
                    240 - grout * 40
                }
                Style::Brick => {
                    let row = y / 8;
                    let off = (row % 2) * 8;
                    let mortar = (y % 8 == 0 || (x + off) % 16 == 0) as u8;
                    232 - mortar * 48
                }
                Style::Ashlar => {
                    let row = y / 16;
                    let off = (row % 2) * 16;
                    let bed = (y % 16 == 0) as u8;
                    let perp = ((x + off) % 32 == 0) as u8;
                    238 - bed * 40 - perp * 24
                }
                Style::Metal => {
                    let seam = (x % 16 == 0) as u8;
                    236 - seam * 36
                }
                Style::Hazard => {
                    let stripe = ((x + y) / 8 % 2 == 0) as u8;
                    240 - stripe * 150
                }
                Style::Grate => {
                    let hole = (x % 8 < 2 && y % 8 < 2) as u8;
                    230 - hole * 160
                }
            };
            let i = ((y * N + x) * 4) as usize;
            px[i] = v;
            px[i + 1] = v;
            px[i + 2] = v;
            px[i + 3] = 255;
        }
    }
    Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    )
}

/// Vertical gradient for the sky dome (zenith #1d6fdc → mid #5aa8f2 → horizon
/// #d4ecfa, `environment.js` day theme). The gradient is compressed into the
/// upper hemisphere (v 0..0.5 → zenith→mid→horizon), so the horizon tint
/// reaches eye level like the upstream shader; the lower half (hidden by the
/// sea/ground) stays horizon-coloured.
fn sky_image() -> Image {
    const W: u32 = 4;
    const H: u32 = 128;
    let mut px = vec![0u8; (W * H * 4) as usize];
    let zenith = [29u8, 111, 220];
    let mid = [90u8, 168, 242];
    let hor = [212u8, 236, 250];
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
        batch.uvs.push([x / TEX_SCALE, z / TEX_SCALE]);
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
        app.add_plugins(crate::ink_render::InkPlugin)
            .insert_resource(ClearColor(SKY))
            .insert_resource(LayoutRes(layout))
            .insert_resource(SimWorld(world))
            .insert_resource(atlas)
            .insert_resource(demo)
            .insert_resource(pc)
            .init_resource::<CamRig>()
            .init_resource::<crate::input::LookSettings>()
            .init_resource::<crate::ui::UiState>()
            .init_resource::<crate::ui::Feed>()
            .init_resource::<crate::ui::HitFlash>();
        // Task 14: procedural SFX (silent degradation when no audio device).
        crate::audio::add_audio(app);
        app.add_systems(
            Startup,
            (
                build_level_world,
                crate::actors::spawn_actors,
                crate::ui::build_ui,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                crate::input::map_input,
                // Before the UI state machine: it drains `Match::events`.
                crate::audio::audio_sfx,
                crate::ui::ui_state_machine,
                crate::input::tune_sensitivity,
                crate::ink_render::ink_upload,
                crate::actors::sync_actors,
                crate::ui::ui_update,
                crate::ui::ui_button_style,
                crate::audio::sfx_gc,
                toggle_shadows,
                camera_rig,
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ink_materials: ResMut<Assets<InkMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let l = &layout.0;
    let ink_tex = images.add(atlas_res.make_image());
    commands.insert_resource(InkImage(ink_tex.clone()));

    // ---- batched primitive geometry (one draw call per style × colour) ----
    let mut batches: HashMap<(Style, String), Batch> = HashMap::new();
    let mut visible = 0u32;
    let mut hidden = 0u32;
    for (bid, brush) in l.primitives.iter().enumerate() {
        let c = brush.common();
        if c.hidden {
            hidden += 1;
            continue;
        }
        visible += 1;
        emit_box(
            batches
                .entry((style_for(c.pattern), c.color.clone()))
                .or_default(),
            &frame_of(brush),
            Some((&sim.0, &atlas_res.atlas, bid)),
        );
    }
    let mut style_tex: HashMap<Style, Handle<Image>> = HashMap::new();
    let mut draw_calls = 0u32;
    let mut sorted: Vec<_> = batches.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for ((style, color), batch) in sorted {
        if batch.positions.is_empty() {
            continue;
        }
        let tex = style_tex
            .entry(style)
            .or_insert_with(|| images.add(pattern_image(style)))
            .clone();
        let mesh = meshes.add(to_mesh(&batch));
        let mat = ink_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: hex_color(&color),
                base_color_texture: Some(tex),
                perceptual_roughness: 0.85,
                ..default()
            },
            extension: InkExt {
                ink_atlas: ink_tex.clone(),
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

    // Ambient sky fill (day hemiSky #b4d0ff). M1 uses a single ambient term;
    // upstream also has a hemiGround #dcc3a0×2.2 bounce (environment.js L3036).
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.706, 0.816, 1.0),
        brightness: 1500.0,
        affects_lightmapped_meshes: false,
    });

    // Sun: direction from (SUN_AZ, SUN_EL) — environment.js L2996:
    // uSunDir = (cos(el)·cos(az), sin(el), cos(el)·sin(az)).
    // A Bevy directional light shines along its transform's -Z, so -Z = the
    // travel direction (away from the sun, i.e. -sun_dir).
    let az = SUN_AZ.to_radians();
    let el = SUN_EL.to_radians();
    let sun_dir = Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin()).normalize();
    commands.spawn((
        SunLight,
        DirectionalLight {
            color: SUN_COLOR,
            illuminance: 12000.0,
            shadow_maps_enabled: true,
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
    commands.insert_resource(DirectionalLightShadowMap { size: 2048 });

    // Sky dome: inside-out gradient sphere (unlit).
    let sky_tex = images.add(sky_image());
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
        base_color: Color::srgba(0.039, 0.31, 0.541, 0.78), // day seaDeep #0a4f8a
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
    commands.spawn((
        Camera3d::default(),
        Projection::from(PerspectiveProjection {
            // cameraRig.js L188: vertical FOV from the horizontal 82° at the
            // fixed 16/9 reference; Bevy derives `aspect` from the window.
            fov: vfov(BASE_FOV_H, REF_ASPECT).to_radians(),
            near: 0.15,
            far: 6500.0,
            ..default()
        }),
        DistanceFog {
            color: FOG_COLOR,
            falloff: FogFalloff::Linear {
                start: FOG_NEAR,
                end: FOG_FAR,
            },
            ..default()
        },
        Transform::from_translation(Vec3::new(0.0, 4.0, -50.0)),
    ));

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

/// `K` toggles sun shadows (spec: 阴影可开关).
fn toggle_shadows(
    keys: Res<ButtonInput<KeyCode>>,
    mut sun: Query<&mut DirectionalLight, With<SunLight>>,
) {
    if keys.just_pressed(KeyCode::KeyK) {
        for mut l in sun.iter_mut() {
            l.shadow_maps_enabled = !l.shadow_maps_enabled;
            println!(
                "[inkwave] shadows {}",
                if l.shadow_maps_enabled { "ON" } else { "OFF" }
            );
        }
    }
}

/// Third-person follow of the local player (slot 0): the pivot tracks the
/// actor's visual position, look comes from `PlayerControls`, and the boom
/// shrinks when a wall blocks the line of sight (cameraRig.js follow).
fn camera_rig(
    mut rig: ResMut<CamRig>,
    sim: Res<SimWorld>,
    pc: Res<crate::input::PlayerControls>,
    demo: Res<crate::ink_render::DemoSim>,
    settings: Res<crate::input::LookSettings>,
    time: Res<Time>,
    mut q: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let (yaw, pitch) = (pc.yaw, pc.pitch);
    // follow the player's feet + visual offset, damped (upstream pivot lerp)
    if let Some(a) = demo.m.actors.first() {
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
    let cam_pos = pivot - look * rig.boom + Vec3::Y * 0.15;

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
    fn emit_box_uvs_scale_by_tex_scale() {
        let l = sample();
        let mut batch = Batch::default();
        // 4.8 m box → ±2.4 half → u spans 4.8/2.4 = 2 tiles: corners at ±1.0
        emit_box(
            &mut batch,
            &frame_of(&box_brush(&l, [0.0, 0.0, 0.0], [4.8, 4.8, 4.8])),
            None,
        );
        let us: Vec<f32> = batch.uvs.iter().map(|u| u[0]).collect();
        assert!(us.iter().any(|v| (*v - 1.0).abs() < 1e-5));
        assert!(us.iter().any(|v| (*v + 1.0).abs() < 1e-5));
    }

    #[test]
    fn style_for_covers_pattern_table() {
        assert_eq!(style_for(0), Style::Flat);
        assert_eq!(style_for(29), Style::Tile); // herringbone slot
        assert_eq!(style_for(28), Style::Brick);
        assert_eq!(style_for(30), Style::Ashlar); // stucco slot
        assert_eq!(style_for(4), Style::Hazard);
        assert_eq!(style_for(99), Style::Flat);
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
    fn embedded_tidewater_batches_to_expected_styles() {
        let l = sample();
        let mut keys = std::collections::HashSet::new();
        let mut visible = 0u32;
        let mut hidden = 0u32;
        for b in &l.primitives {
            let c = b.common();
            if c.hidden {
                hidden += 1;
                continue;
            }
            visible += 1;
            keys.insert((style_for(c.pattern), c.color.clone()));
        }
        // TR-10.1: visible primitives collapse into 15 style×colour batches.
        assert_eq!(keys.len(), 15);
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
            keys.len()
        );
    }
}
