//! Task 52 batch D2 — wire the ported `level.wgsl` uber-shader as a
//! `MaterialExtension` over `StandardMaterial` (spec.md D-T52-4).
//!
//! `LevelExt` mirrors the WGSL binding contract of `level.wgsl` L84-185
//! (declared order ≡ binding number): the ink atlas reuses the `InkImage`
//! handle from `ink_render` (20/21, same slot as the legacy `InkExt`), the
//! texlib array textures (22/23/24) share the single sampler declared on the
//! albedo field (25), the mural atlas (26/27) is a 1×1 white placeholder
//! (deviation ⑤ data-inactive), and bindings 28-40 are the uniform tables
//! (`uTL`/`uTLt`/`uTLs` from [`crate::texlib::schema::slot_uniforms`]; the
//! ripple / wake pools are fed every frame by [`level_material_wiring`]
//! since batch E2, the lamp / mural pools stay zero — Tasks 53/55).
//!
//! `LevelParams` (41) carries the per-frame material scalars/colours with the
//! JS defaults taken from the texlib document (never hand-copied); `BatchParams`
//! (42) carries deviation ③ (per-vertex faceData/faceFlags/faceTan as
//! per-batch constants) and is built by `world` from the batch key.

use std::sync::OnceLock;

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;

use crate::texlib::schema::{TexLibDoc, slot_uniforms};

/// Shader handle registered by [`LevelPlugin`] (same precedent as the
/// `INK_SHADER` static in `ink_render.rs`).
static LEVEL_SHADER: OnceLock<Handle<Shader>> = OnceLock::new();

// ---------------------------------------------------------------- uniforms

/// ≡ `level.wgsl` `struct LevelParams` (binding 41): 16 scalars (64 B) then
/// six vec4-padded vec3 slots (96 B) = 160 B, matching the WGSL uniform
/// layout member-for-member.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct LevelParams {
    /// ≡ uInkClock (inkShading.js L26; default 0).
    pub ink_clock: f32,
    /// ≡ uTime (levelMaterial.js L49; default 0).
    pub time: f32,
    /// ≡ uPpm (L62; default `doc.ink.ppm`).
    pub ppm: f32,
    /// ≡ uAtlasSize (L61); the shader derives uTexel = 1/this.
    pub atlas_px: f32,
    /// ≡ uInkGlow (L50; default `doc.ink.glow`).
    pub ink_glow: f32,
    /// ≡ uGel (L64; `L.gel ?? -1` sentinel = off, deviation ⑤).
    pub gel: f32,
    /// ≡ uTexSize (L95; default `doc.tex.size_high`).
    pub tex_size: f32,
    /// ≡ uAO (L58; deviation ④ default 0 = inactive).
    pub ao: f32,
    /// ≡ uLampN (L60; default 0).
    pub lamp_n: i32,
    /// ≡ uSeeOn (L55; default 0).
    pub see_on: f32,
    /// ≡ uSeeFeet (L56).
    pub see_feet: f32,
    /// ≡ uSeeA2C (L57).
    pub see_a2c: f32,
    /// ≡ GRATE define (L715) as a runtime flag (default 0).
    pub grate: f32,
    /// WGSL `_pad0..2` (keeps the vec3 block at offset 64).
    pub pad: [f32; 3],
    /// ≡ uTeamA (L47).
    pub team_a: Vec4,
    /// ≡ uTeamB (L48).
    pub team_b: Vec4,
    /// ≡ uLampCol = LAMP_HEX × LAMP_I × k (L11/L60).
    pub lamp_col: Vec4,
    /// ≡ uSeeB player chest (L54).
    pub see_b: Vec4,
    /// ≡ uSeeA camera position (L53).
    pub see_a: Vec4,
    /// ≡ directionalLights[0].direction (inkShading L132-136).
    pub sun_dir: Vec4,
}

/// ≡ `level.wgsl` `struct BatchParams` (binding 42, deviation ③): 8 scalars,
/// `u_fsz` (vec2 + pad) and `u_tan` (vec3 + pad) = 64 B.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct BatchParams {
    /// ≡ vFaceData.x (pattern slot id).
    pub pattern: f32,
    /// ≡ vFaceData.y (ink gate, `> 0.5` in INK_COLOR).
    pub wet_base: f32,
    /// ≡ vFaceFlags.x (wall face).
    pub flags_x: f32,
    /// ≡ vFaceFlags.y (grounded bottom edge).
    pub flags_y: f32,
    /// ≡ vFaceFlags.z (mural id, < 0 = none; ⑤ default -1).
    pub mur: f32,
    /// WGSL `_pad0..2`.
    pub pad: [f32; 3],
    /// ≡ vFaceData.zw (face size in metres) + pad.
    pub fsz: Vec4,
    /// ≡ vFaceTan (frame-space u axis) + pad.
    pub tan: Vec4,
}

impl Default for BatchParams {
    fn default() -> Self {
        Self {
            pattern: 0.0,
            wet_base: 0.0,
            flags_x: 0.0,
            flags_y: 0.0,
            mur: -1.0,
            pad: [0.0; 3],
            fsz: Vec4::ZERO,
            tan: Vec4::ZERO,
        }
    }
}

impl LevelParams {
    /// The JS defaults, every value sourced from the texlib document (never
    /// hand-copied): glow/ppm/team colours from `doc.ink`, tex_size from
    /// `doc.tex`, lamp colour `LAMP_HEX × LAMP_I` from `doc.lamps`, and the
    /// sun fallback `normalize(-0.41, 0.83, -0.38)` (level.wgsl L36-37, the
    /// same `#else` branch as inkShading.js L132-136).
    #[must_use]
    pub fn defaults(doc: &TexLibDoc, atlas_px: u32) -> Self {
        Self {
            ink_clock: 0.0,
            time: 0.0,
            ppm: doc.ink.ppm,
            atlas_px: atlas_px as f32,
            ink_glow: doc.ink.glow,
            gel: -1.0,
            tex_size: doc.tex.size_high as f32,
            ao: 0.0,
            lamp_n: 0,
            see_on: 0.0,
            see_feet: 0.0,
            see_a2c: 0.0,
            grate: 0.0,
            pad: [0.0; 3],
            team_a: hex_vec4(&doc.ink.team_a, 1.0),
            team_b: hex_vec4(&doc.ink.team_b, 1.0),
            lamp_col: hex_vec4(&doc.lamps.hex, doc.lamps.intensity),
            see_b: Vec4::ZERO,
            see_a: Vec4::ZERO,
            sun_dir: (Vec3::new(-0.41, 0.83, -0.38).normalize(), 0.0).into(),
        }
    }
}

/// `#rrggbb` → linear sRGB components × `scale` (w = 0; the WGSL slots are
/// vec3 + pad). Same byte parsing as `world::hex_color`.
fn hex_vec4(s: &str, scale: f32) -> Vec4 {
    let h = s.trim_start_matches('#');
    let byte = |i: usize| -> f32 {
        u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(221) as f32 / 255.0
    };
    Vec4::new(byte(0) * scale, byte(1) * scale, byte(2) * scale, 0.0)
}

// ---------------------------------------------------------------- extension

/// The level material extension: every binding of the `level.wgsl` contract
/// (group 3 slots 20-42, above `StandardMaterial`'s own — the same precedent
/// as `InkExt` at 20/21 in `ink_render.rs`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct LevelExt {
    /// The shared ink atlas (RGBA8), the same `InkImage` handle the upload
    /// system keeps live.
    #[texture(20)]
    #[sampler(21)]
    pub ink_atlas: Handle<Image>,
    /// Texlib albedo array (≡ tAlbedo). Binding 25's sampler is declared on
    /// this field and shared by all three array textures (WGSL `texlib_sampler`).
    #[texture(22, dimension = "2d_array")]
    #[sampler(25)]
    pub t_albedo: Handle<Image>,
    /// Texlib normal array (≡ tNormal).
    #[texture(23, dimension = "2d_array")]
    pub t_normal: Handle<Image>,
    /// Texlib ORM array (≡ tOrm).
    #[texture(24, dimension = "2d_array")]
    pub t_orm: Handle<Image>,
    /// Mural atlas (≡ uMural; ⑤ 1×1 white placeholder, data-inactive).
    #[texture(26)]
    #[sampler(27)]
    pub mural_atlas: Handle<Image>,
    /// ≡ uTL (levelMaterial.js L96/L208): (layer, 1/scale, mode, sym).
    #[uniform(28)]
    pub u_tl: [Vec4; 50],
    /// ≡ uTLt (L97/L209): (tintMode, nrm, onWall, onTop).
    #[uniform(29)]
    pub u_tl_t: [Vec4; 50],
    /// ≡ uTLs (L99/L210): stair params.
    #[uniform(30)]
    pub u_tl_s: [Vec4; 50],
    /// ≡ uRip (inkShading.js L51): fed per frame by [`level_material_wiring`].
    #[uniform(31)]
    pub u_rip: [Vec4; 24],
    /// ≡ uRipP (L52).
    #[uniform(32)]
    pub u_rip_p: [Vec4; 24],
    /// ≡ uWake (levelMaterial.js L174): fed per frame (D-T51-7⑦ 收口).
    #[uniform(33)]
    pub u_wake: [Vec4; 48],
    /// ≡ uWakeB (L175).
    #[uniform(34)]
    pub u_wake_b: [Vec4; 4],
    /// ≡ uSwimH (L176).
    #[uniform(35)]
    pub u_swim_h: [Vec4; 4],
    /// ≡ uSwimF (L177).
    #[uniform(36)]
    pub u_swim_f: [Vec4; 4],
    /// ≡ uLamps (L163): `array<vec3<f32>,12>` — the WGSL uniform stride is 16
    /// per element, so the Rust side packs xyz = lamp and w = padding. Zero
    /// until Task 53/55.
    #[uniform(37)]
    pub u_lamps: [Vec4; 12],
    /// ≡ uMurA (L213): zero (⑤).
    #[uniform(38)]
    pub u_mur_a: [Vec4; 12],
    /// ≡ uMurB (L214).
    #[uniform(39)]
    pub u_mur_b: [Vec4; 12],
    /// ≡ uMurC (L215, JS vec2[12] vec4-padded).
    #[uniform(40)]
    pub u_mur_c: [Vec4; 12],
    /// Per-frame material scalars/colours (binding 41).
    #[uniform(41)]
    pub params: LevelParams,
    /// Deviation ③ per-batch face data (binding 42).
    #[uniform(42)]
    pub batch: BatchParams,
}

/// Level-shaded standard material.
pub type LevelMaterial = ExtendedMaterial<StandardMaterial, LevelExt>;

impl MaterialExtension for LevelExt {
    fn fragment_shader() -> ShaderRef {
        LEVEL_SHADER
            .get()
            .cloned()
            .map(ShaderRef::Handle)
            .unwrap_or(ShaderRef::Default)
    }
}

impl LevelExt {
    /// The shared (batch-independent) extension: the three `uTL` tables built
    /// from the texlib document ≡ levelMaterial.js L96-99, the JS default
    /// `LevelParams`, and zero-filled batch-E pools. Every level material is
    /// a clone of this with a per-batch [`BatchParams`].
    #[must_use]
    pub fn new_shared(
        ink_atlas: Handle<Image>,
        texlib: &TexLibImages,
        doc: &TexLibDoc,
        atlas_px: u32,
    ) -> Self {
        let u = slot_uniforms(doc);
        Self {
            ink_atlas,
            t_albedo: texlib.albedo.clone(),
            t_normal: texlib.normal.clone(),
            t_orm: texlib.orm.clone(),
            mural_atlas: texlib.mural.clone(),
            u_tl: vec4_table(&u.u_tl),
            u_tl_t: vec4_table(&u.u_tl_t),
            u_tl_s: vec4_table(&u.u_tl_s),
            u_rip: [Vec4::ZERO; 24],
            u_rip_p: [Vec4::ZERO; 24],
            u_wake: [Vec4::ZERO; 48],
            u_wake_b: [Vec4::ZERO; 4],
            u_swim_h: [Vec4::ZERO; 4],
            u_swim_f: [Vec4::ZERO; 4],
            u_lamps: [Vec4::ZERO; 12],
            u_mur_a: [Vec4::ZERO; 12],
            u_mur_b: [Vec4::ZERO; 12],
            u_mur_c: [Vec4::ZERO; 12],
            params: LevelParams::defaults(doc, atlas_px),
            batch: BatchParams::default(),
        }
    }
}

/// `[f32; 4]` rows → fixed `Vec4` table (slot tables are 50 long by schema;
/// a short row list pads with zeros rather than panicking at boot).
fn vec4_table<const N: usize>(rows: &[[f32; 4]]) -> [Vec4; N] {
    let mut out = [Vec4::ZERO; N];
    for (o, r) in out.iter_mut().zip(rows) {
        *o = Vec4::from_array(*r);
    }
    out
}

// ---------------------------------------------------------------- textures

/// Placeholder GPU images for the texlib array textures + mural atlas
/// (bindings 22-24/26). Batch E replaces the contents with the baked layers
/// (`texlib::bake`); until then every layer is a neutral constant so the
/// shader path runs: albedo mid-grey × base colour, normal flat
/// (128,128,255), ORM (occlusion 1, roughness ≈ `doc.mat.roughness`,
/// metalness 0).
#[derive(Resource, Debug, Clone)]
pub struct TexLibImages {
    /// 1×1×N albedo array (`doc.layers.len()` layers).
    pub albedo: Handle<Image>,
    /// 1×1×N normal array.
    pub normal: Handle<Image>,
    /// 1×1×N ORM array.
    pub orm: Handle<Image>,
    /// 1×1 white mural atlas (⑤ inactive).
    pub mural: Handle<Image>,
}

/// One 1×1 array-texture layer of `rgba` bytes repeated `layers` times.
fn array_placeholder(rgba: [u8; 4], layers: u32, format: TextureFormat) -> Image {
    Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        vec![rgba; layers as usize].concat(),
        format,
        default(),
    )
}

/// Upload the placeholders and wrap their handles (called once by
/// `build_level_world`).
///
/// Batch E2 note: the WGSL `texlib_sampler` (binding 25) is repeat-wrap +
/// mip-linear + aniso ≤ 16 (levelMaterial.js L195-201). The placeholders ride
/// Bevy's default sampler; [`crate::texlib::res::texlib_bake_poll`] swaps
/// data + descriptor + sampler in place when the baked layers land, so the
/// handles every `LevelExt` holds stay valid.
pub fn add_texlib_placeholders(doc: &TexLibDoc, images: &mut Assets<Image>) -> TexLibImages {
    let layers = doc.layers.len() as u32;
    let rough = (doc.mat.roughness * 255.0).round() as u8;
    TexLibImages {
        albedo: images.add(array_placeholder(
            [200, 200, 200, 255],
            layers,
            TextureFormat::Rgba8Unorm,
        )),
        normal: images.add(array_placeholder(
            [128, 128, 255, 255],
            layers,
            TextureFormat::Rgba8Unorm,
        )),
        orm: images.add(array_placeholder(
            [255, rough, 0, 255],
            layers,
            TextureFormat::Rgba8Unorm,
        )),
        mural: images.add(array_placeholder(
            [255, 255, 255, 255],
            1,
            TextureFormat::Rgba8Unorm,
        )),
    }
}

// ---------------------------------------------------------------- wiring (batch E2)

/// Flood-front tracker for the ripple feed: JS fires the capture swell once
/// when the front is created (`paint.js` L729 reads `J` at flood() time);
/// `FloodRec.t0` (the zone core clock at creation) is the edge identity.
/// `-1.0` = nothing seen yet (t0 is always ≥ the first frame's dt > 0).
#[derive(Resource)]
pub struct RippleFeed {
    last_t0: [f32; crate::fx::zone_marks::MAXZ],
}

impl Default for RippleFeed {
    fn default() -> Self {
        Self {
            last_t0: [-1.0; crate::fx::zone_marks::MAXZ],
        }
    }
}

/// Per-frame presentation wiring (Task 52 batch E2): sets the ripple view
/// origin from the camera, emits a swell for every newly created capture
/// flood, advances the paint clock and writes the batch-E pools into every
/// `LevelMaterial` — `uRip`/`uRipP`/`uInkClock` plus the four swim/wake
/// buffers (D-T51-7⑦ 收口: [`crate::fx::swim_wake::SwimWakeState`] 的
/// getter 在此每帧上传, ≡ levelMaterial.js L174-177).
///
/// Splat events are fed upstream inside `DemoSim::drain_atlas` (the same
/// frame, earlier in the Update chain). `uLampN` stays 0 — the lamp-position
/// source is deferred to Task 53 (levelMaterial.js L13-35, deviation ⑤).
///
/// Perf note (Task 55): JS shares one uniform object across all level
/// materials; here every per-batch material (~1.2 k) is written each frame
/// and Bevy re-extracts each changed asset. If profiling shows this hot,
/// move the pools to a shared bindless/global uniform buffer.
#[allow(clippy::too_many_arguments)] // Bevy system injection
pub fn level_material_wiring(
    time: Res<Time>,
    pc: Res<crate::input::PlayerControls>,
    mut table: ResMut<crate::ink_ripple::RippleTable>,
    zms: Res<crate::fx::zone_marks::ZoneMarksState>,
    wake: Res<crate::fx::swim_wake::SwimWakeState>,
    demo: Option<Res<crate::ink_render::DemoSim>>,
    mut feed: ResMut<RippleFeed>,
    cams: Query<&GlobalTransform, With<Camera>>,
    mut mats: ResMut<Assets<LevelMaterial>>,
) {
    if pc.paused {
        // Same gate as `ink_upload`: the paint clock freezes with the match.
        return;
    }
    // ≡ paint.js `setView(camera.position)` per frame (L452). The table is
    // sim-side glam 0.30 (ink_ripple.rs); bridge via arrays (camera_rig 先例).
    table.set_view(
        cams.iter()
            .next()
            .map(|g| glam::Vec3::from_array(g.translation().to_array())),
    );

    // New capture floods → one swell each (team < 0 rotation wipes send
    // none, ≡ L729 `J.team >= 0`).
    if let Some(zc) = demo.as_deref().and_then(|d| d.m.zones.as_ref()) {
        for (i, f) in zms.core.floods().iter().enumerate() {
            if let Some(f) = f
                && f.team >= 0
                && feed.last_t0[i] != f.t0
            {
                feed.last_t0[i] = f.t0;
                let z = zc.zones.get(i);
                table.apply(&crate::ink_ripple::RippleSource::FloodJet(
                    crate::ink_ripple::FloodJet {
                        center: z.map_or([zms.core.u_cen()[i][0], zms.core.u_cen()[i][1]], |z| {
                            [z.center[0], z.center[2]]
                        }),
                        // JS `opts.y`: the ink surface height = zone centre y.
                        y: z.map(|z| z.center[1]),
                        reach: f.reach,
                        dur: f.dur,
                        team: f.team,
                    },
                ));
            }
        }
    }
    table.advance(time.delta_secs());

    // Pack once, write every material (see the perf note above). The packed
    // tables are sim-side glam 0.30 `Vec4`s — re-element into Bevy's.
    let (rip, rip_p) = table.pack_uniforms();
    let rip: [Vec4; 24] = rip.map(|v| Vec4::from_array(v.to_array()));
    let rip_p: [Vec4; 24] = rip_p.map(|v| Vec4::from_array(v.to_array()));
    let u_wake = vec4_table::<48>(wake.wake());
    let u_wake_b = vec4_table::<4>(wake.wake_b());
    let u_swim_h = vec4_table::<4>(wake.swim_h());
    let u_swim_f = vec4_table::<4>(wake.swim_f());
    let clock = table.clock;
    for (_id, m) in mats.iter_mut() {
        let ext = &mut m.extension;
        ext.u_rip = rip;
        ext.u_rip_p = rip_p;
        ext.u_wake = u_wake;
        ext.u_wake_b = u_wake_b;
        ext.u_swim_h = u_swim_h;
        ext.u_swim_f = u_swim_f;
        ext.params.ink_clock = clock;
    }
}

// ---------------------------------------------------------------- plugin

/// Registers the level material + shader (added by `WorldPlugin`).
pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "level.wgsl");
        let handle: Handle<Shader> = load_embedded_asset!(app, "level.wgsl");
        let _ = LEVEL_SHADER.set(handle);
        app.add_plugins(MaterialPlugin::<LevelMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texlib::schema::embedded_texlib;

    #[test]
    fn level_params_defaults_take_doc_values() {
        let doc = embedded_texlib();
        let p = LevelParams::defaults(&doc, 1024);
        assert_eq!(p.ppm, doc.ink.ppm);
        assert_eq!(p.ink_glow, doc.ink.glow);
        assert_eq!(p.tex_size, doc.tex.size_high as f32);
        assert_eq!(p.atlas_px, 1024.0);
        assert_eq!(p.gel, -1.0);
        assert_eq!(p.ao, 0.0);
        assert_eq!(p.lamp_n, 0);
        assert_eq!(p.see_on, 0.0);
        assert_eq!(p.grate, 0.0);
        // team_a #ff8a14 → (1, 138/255, 20/255)
        assert_eq!(p.team_a.x, 1.0);
        assert_eq!(p.team_a.y, 138.0 / 255.0);
        assert_eq!(p.team_a.z, 20.0 / 255.0);
        assert_eq!(p.team_b.x, 47.0 / 255.0);
        // lamp_col = #ffc48a × LAMP_I (16)
        assert_eq!(p.lamp_col.x, 16.0);
        assert!((p.lamp_col.y - 196.0 / 255.0 * 16.0).abs() < 1e-5);
        // sun fallback normalize(-0.41, 0.83, -0.38)
        let s = Vec3::new(-0.41, 0.83, -0.38).normalize();
        assert_eq!(p.sun_dir.truncate(), s);
        assert_eq!(p.sun_dir.w, 0.0);
    }

    #[test]
    fn level_ext_u_tl_tables_equal_slot_uniforms() {
        let doc = embedded_texlib();
        let u = slot_uniforms(&doc);
        let tex = TexLibImages {
            albedo: Handle::default(),
            normal: Handle::default(),
            orm: Handle::default(),
            mural: Handle::default(),
        };
        let ext = LevelExt::new_shared(Handle::default(), &tex, &doc, 512);
        let row = |r: &[f32; 4]| Vec4::from_array(*r);
        assert_eq!(
            ext.u_tl.to_vec(),
            u.u_tl.iter().map(row).collect::<Vec<_>>()
        );
        assert_eq!(
            ext.u_tl_t.to_vec(),
            u.u_tl_t.iter().map(row).collect::<Vec<_>>()
        );
        assert_eq!(
            ext.u_tl_s.to_vec(),
            u.u_tl_s.iter().map(row).collect::<Vec<_>>()
        );
        // spot pins (schema tests own the JS parity; here just the wiring)
        assert_eq!(ext.u_tl[0], Vec4::new(0.0, 0.25, 1.0, 7.0));
        assert_eq!(ext.u_tl_s[24], Vec4::new(0.28, 4.0, 0.5, 1.0));
        // batch-E pools stay zero, mur sentinel ≡ -1
        assert_eq!(ext.u_rip, [Vec4::ZERO; 24]);
        assert_eq!(ext.u_lamps, [Vec4::ZERO; 12]);
        assert_eq!(ext.batch.mur, -1.0);
    }

    #[test]
    fn texlib_placeholders_have_one_layer_per_bake_layer() {
        let doc = embedded_texlib();
        let mut images = Assets::<Image>::default();
        let tex = add_texlib_placeholders(&doc, &mut images);
        let layers = doc.layers.len();
        for h in [&tex.albedo, &tex.normal, &tex.orm] {
            let img = images.get(h).unwrap();
            assert_eq!(
                img.texture_descriptor.size.depth_or_array_layers,
                layers as u32
            );
            assert_eq!(img.texture_descriptor.size.width, 1);
            assert_eq!(img.data.as_ref().unwrap().len(), layers * 4);
        }
        let mural = images.get(&tex.mural).unwrap();
        assert_eq!(mural.texture_descriptor.size.depth_or_array_layers, 1);
        // normal placeholder is the flat tangent-space blue
        let n = images.get(&tex.normal).unwrap();
        assert_eq!(&n.data.as_ref().unwrap()[..3], &[128, 128, 255]);
        // ORM g channel ≡ doc.mat.roughness
        let o = images.get(&tex.orm).unwrap();
        assert_eq!(o.data.as_ref().unwrap()[1], 209);
    }

    /// Minimal-App harness for the per-frame wiring system: one level
    /// material, no render world, no `DemoSim` (flood branch inert).
    fn wiring_app(paused: bool) -> (App, Handle<LevelMaterial>) {
        use crate::fx::swim_wake::SwimWakeState;
        use crate::fx::zone_marks::ZoneMarksState;
        use crate::ink_ripple::RippleTable;
        use crate::input::PlayerControls;
        use bevy::asset::AssetPlugin;

        let doc = embedded_texlib();
        let tex = TexLibImages {
            albedo: Handle::default(),
            normal: Handle::default(),
            orm: Handle::default(),
            mural: Handle::default(),
        };
        let ext = LevelExt::new_shared(Handle::default(), &tex, &doc, 1024);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<RippleTable>()
            .init_resource::<RippleFeed>()
            .init_resource::<ZoneMarksState>()
            .init_resource::<SwimWakeState>()
            .insert_resource(PlayerControls {
                paused,
                ..default()
            })
            .init_asset::<LevelMaterial>();
        let h = app
            .world_mut()
            .resource_mut::<Assets<LevelMaterial>>()
            .add(ExtendedMaterial {
                base: StandardMaterial::default(),
                extension: ext,
            });
        app.add_systems(Update, level_material_wiring);
        (app, h)
    }

    #[test]
    fn wiring_feeds_ripple_wake_and_clock() {
        use crate::fx::swim_wake::SwimWakeState;
        use crate::ink_ripple::{RippleSource, RippleTable, SplatEvent};
        use bevy::time::TimeUpdateStrategy;
        use std::time::{Duration, Instant};

        let (mut app, h) = wiring_app(false);
        // Deterministic frame clock (the first frame's delta is 0 by design).
        let mut now = Instant::now();
        app.insert_resource(TimeUpdateStrategy::ManualInstant(now));
        app.update();
        // One non-cosmetic splat ≥ the 0.15 m gate lands in the pool.
        app.world_mut()
            .resource_mut::<RippleTable>()
            .apply(&RippleSource::Splat(SplatEvent {
                center: glam::Vec3::new(1.0, 0.0, 2.0),
                radius: 0.5,
                cosmetic: false,
            }));
        now += Duration::from_millis(16);
        *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualInstant(now);
        app.update();
        {
            let mats = app.world().resource::<Assets<LevelMaterial>>();
            let m = mats.get(&h).unwrap();
            assert!(
                m.extension.u_rip.iter().any(|v| *v != Vec4::ZERO),
                "splat must reach uRip"
            );
            assert!(
                m.extension.u_rip_p.iter().any(|v| *v != Vec4::ZERO),
                "splat must reach uRipP"
            );
            assert!(m.extension.params.ink_clock > 0.0, "paint clock must tick");
        }
        // The four wake buffers ≡ the SwimWakeState getters (D-T51-7⑦ 收口).
        let wake = app.world().resource::<SwimWakeState>();
        let mats = app.world().resource::<Assets<LevelMaterial>>();
        let m = mats.get(&h).unwrap();
        assert_eq!(
            m.extension.u_wake.to_vec(),
            wake.wake()
                .iter()
                .copied()
                .map(Vec4::from_array)
                .collect::<Vec<_>>()
        );
        for (dst, src) in [
            (&m.extension.u_wake_b[..], &wake.wake_b()[..]),
            (&m.extension.u_swim_h[..], &wake.swim_h()[..]),
            (&m.extension.u_swim_f[..], &wake.swim_f()[..]),
        ] {
            assert_eq!(
                dst.to_vec(),
                src.iter()
                    .copied()
                    .map(Vec4::from_array)
                    .collect::<Vec<_>>()
            );
        }
        // Clock strictly advances across frames.
        let c0 = app
            .world()
            .resource::<Assets<LevelMaterial>>()
            .get(&h)
            .unwrap()
            .extension
            .params
            .ink_clock;
        now += Duration::from_millis(16);
        *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualInstant(now);
        app.update();
        let c1 = app
            .world()
            .resource::<Assets<LevelMaterial>>()
            .get(&h)
            .unwrap()
            .extension
            .params
            .ink_clock;
        assert!(c1 > c0, "clock {c0} -> {c1} must advance");
    }

    #[test]
    fn wiring_freezes_when_paused() {
        let (mut app, h) = wiring_app(true);
        app.update();
        let m = app
            .world()
            .resource::<Assets<LevelMaterial>>()
            .get(&h)
            .unwrap();
        assert_eq!(m.extension.u_rip, [Vec4::ZERO; 24]);
        assert_eq!(m.extension.params.ink_clock, 0.0);
    }
}
