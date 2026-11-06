//! Batch E — `TexLibRes`: on-demand CPU bake of the slot-referenced layers
//! and upload of the three texlib array textures (spec.md D-T52-5).
//!
//! Lifecycle: `world::build_level_world` calls [`start_bake`] (windowed /
//! render-world only — the headless branch never adds `WorldPlugin`, so the
//! bake thread is never spawned there). The worker bakes every layer the slot
//! table references ([`used_layer_indices`]) at the quality tier's size
//! (`Low = 256`, `Med/High = 512`, [`crate::quality`]), packs the three
//! `size × size × doc.layers.len()` mip-chained arrays layer-major and sends
//! the result back over an `mpsc` channel. [`texlib_bake_poll`] drains it
//! once and swaps the placeholder [`TexLibImages`] contents in place — the
//! handles every `LevelExt` already references stay valid.
//!
//! Upload path: Bevy 0.19's standard asset pipeline handles multi-mip array
//! textures natively — `GpuImage::prepare_asset` calls
//! `create_texture_with_data` with `texture_descriptor.mip_level_count`,
//! which writes every layer × every mip in one pass (wgpu `util/device.rs`).
//! No manual render-world system is needed, so the LOD0 fallback registered
//! in the task brief was not taken.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDataOrder, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};

use super::bake::{self, BakedLayer};
use super::schema::TexLibDoc;
use crate::level_render::TexLibImages;

/// Mip count of a `size × size` chain down to 1×1: `log2(size) + 1`.
/// `bake::build_mips` produces exactly this many levels.
#[must_use]
pub fn mip_count_for(size: u32) -> u32 {
    assert!(size.is_power_of_two(), "texlib size must be a power of two");
    size.trailing_zeros() + 1
}

/// Total RGBA8 bytes of one `size`-based mip chain (≡ what wgpu expects per
/// array layer when `mip_level_count > 1`).
#[must_use]
pub fn chain_bytes(size: u32) -> usize {
    let mut n = 0usize;
    let mut s = size;
    loop {
        n += (s * s * 4) as usize;
        if s == 1 {
            return n;
        }
        s /= 2;
    }
}

/// Layer indices referenced by the slot table (`doc.slots.map`, deduplicated
/// by name, first-reference order). Layers no slot points at (`gel`,
/// `detail`) are never baked — their array slots keep the neutral placeholder
/// bytes.
#[must_use]
pub fn used_layer_indices(doc: &TexLibDoc) -> Vec<usize> {
    let mut seen = std::collections::HashSet::<&str>::new();
    let mut out = Vec::new();
    for name in &doc.slots.map {
        if seen.insert(name.as_str())
            && let Some(i) = super::schema::layer_index(doc, name)
        {
            out.push(i);
        }
    }
    out
}

/// Resolve a layer name to its surf program: base registry first, then the
/// stage registry (`'<stage>:<name>'`).
#[must_use]
pub fn resolve_program(name: &str) -> Option<&'static dyn bake::SurfProgram> {
    super::materials::lookup(name).or_else(|| super::materials_stage::lookup(name))
}

/// The packed upload payload: one layer-major byte blob per attachment.
#[derive(Debug)]
pub struct PackedLib {
    /// `size × size` base level.
    pub size: u32,
    /// `log2(size) + 1` (≡ `texture_descriptor.mip_level_count`).
    pub mip_count: u32,
    /// `layers` array slots × mip chains, layer-major (albedo sRGB bytes).
    pub albedo: Vec<u8>,
    /// Tangent-space normals, linear.
    pub normal: Vec<u8>,
    /// cavity / roughness / metalness / height.
    pub orm: Vec<u8>,
    /// Number of array layers (`doc.layers.len()`).
    pub layers: u32,
    /// Layers actually baked (`used_layer_indices` hits with a program).
    pub layers_baked: usize,
    /// Worker-side bake wall time, ms (perf log, TR-52 report).
    pub ms: u128,
}

/// Pack `baked` (indexed by `doc.layers` position) into the three layer-major
/// arrays; unused slots get the neutral constant of the placeholder
/// (`level_render::add_texlib_placeholders`) at every mip.
#[must_use]
pub fn pack_layers(
    doc: &TexLibDoc,
    baked: &[(usize, BakedLayer)],
    size: u32,
    ms: u128,
) -> PackedLib {
    let layers = doc.layers.len() as u32;
    let mip_count = mip_count_for(size);
    let rough = (doc.mat.roughness * 255.0).round() as u8;
    let neutral = [
        [200u8, 200, 200, 255],
        [128, 128, 255, 255],
        [255, rough, 0, 255],
    ];
    let per = chain_bytes(size);
    let mut out = [
        vec![0u8; per * layers as usize],
        vec![0u8; per * layers as usize],
        vec![0u8; per * layers as usize],
    ];
    // Neutral fill first: constant texel per mip level per unused slot.
    for (o, rgba) in out.iter_mut().zip(&neutral) {
        for layer in 0..layers as usize {
            let mut off = layer * per;
            let mut s = size;
            loop {
                let n = (s * s * 4) as usize;
                for px in o[off..off + n].as_chunks_mut::<4>().0 {
                    px.copy_from_slice(rgba);
                }
                if s == 1 {
                    break;
                }
                off += n;
                s /= 2;
            }
        }
    }
    // Overwrite the baked slots with their chains (mip 0 first per layer).
    for (i, b) in baked {
        for (o, chain) in out.iter_mut().zip([&b.albedo, &b.normal, &b.orm]) {
            let mut off = i * per;
            for level in &chain.levels {
                let n = (level.size * level.size * 4) as usize;
                o[off..off + n].copy_from_slice(&level.rgba);
                off += n;
            }
        }
    }
    let [albedo, normal, orm] = out;
    PackedLib {
        size,
        mip_count,
        albedo,
        normal,
        orm,
        layers,
        layers_baked: baked.len(),
        ms,
    }
}

/// Bake every slot-referenced layer on the calling thread (the worker half
/// of [`start_bake`]; kept public + pure so tests can run it directly).
#[must_use]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // wasm `start_bake` never spawns the worker
pub fn bake_all(doc: &TexLibDoc, size: u32) -> PackedLib {
    let t0 = Instant::now();
    let mut baked = Vec::new();
    for i in used_layer_indices(doc) {
        let layer = &doc.layers[i];
        if let Some(prog) = resolve_program(&layer.name) {
            baked.push((i, bake::bake_layer(layer, prog, size)));
        }
    }
    pack_layers(doc, &baked, size, t0.elapsed().as_millis())
}

/// The bake-in-flight resource (spec name `TexLibRes`): the worker's channel
/// receiver plus the done flag. Inserted by `build_level_world`; absent (and
/// therefore inert) in headless runs.
#[derive(Resource)]
pub struct TexLibBake {
    rx: Arc<Mutex<Option<Receiver<PackedLib>>>>,
    done: bool,
}

/// Spawn the worker thread and wrap its receiver. Called once from
/// `build_level_world` (render-world gated).
///
/// WASM deviation: `std::thread` is unavailable on `wasm32-unknown-unknown`,
/// so the browser build keeps the neutral placeholders (registered in the
/// TR-52 report; a `wasm_thread`/`Asyncify` bake pass is a Task 55 candidate).
#[must_use]
pub fn start_bake(doc: &TexLibDoc, size: u32) -> TexLibBake {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (doc, size);
        TexLibBake::inert()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let doc = doc.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // A send error only means the app dropped the resource — nothing
            // to do; a worker panic would surface in the poll as
            // `Disconnected`.
            let _ = tx.send(bake_all(&doc, size));
        });
        TexLibBake {
            rx: Arc::new(Mutex::new(Some(rx))),
            done: false,
        }
    }
}

impl TexLibBake {
    /// A bake that will never deliver (headless placeholder / tests).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native never takes the wasm branch
    #[must_use]
    pub fn inert() -> Self {
        Self {
            rx: Arc::new(Mutex::new(None)),
            done: true,
        }
    }
}

/// ≡ `levelMaterial.js` L195-201: repeat wrap on both axes, mip-linear
/// filtering, anisotropy clamped to 16 (the WGSL `texlib_sampler`, binding
/// 25, shared by the three arrays — declared on `LevelExt::t_albedo`).
#[must_use]
pub fn texlib_sampler() -> ImageSampler {
    let mut d = ImageSamplerDescriptor::linear();
    d.set_address_mode(ImageAddressMode::Repeat);
    d.anisotropy_clamp = 16;
    ImageSampler::Descriptor(d)
}

/// One-shot swap of a placeholder array image into the baked one: the
/// descriptor change (1×1/mip 1 → size×size/mip N) makes Bevy's
/// `prepare_asset` recreate the GPU texture with the full chain.
fn apply_array(
    img: &mut Image,
    size: u32,
    layers: u32,
    mip_count: u32,
    data: Vec<u8>,
    format: TextureFormat,
) {
    img.texture_descriptor = TextureDescriptor {
        label: Some("texlib"),
        size: Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        dimension: TextureDimension::D2,
        format,
        mip_level_count: mip_count,
        sample_count: 1,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    };
    img.data_order = TextureDataOrder::LayerMajor;
    img.data = Some(data);
    img.sampler = texlib_sampler();
}

/// Drain the bake worker and upload the three arrays (runs every frame until
/// the result lands; registered by `WorldPlugin`).
pub fn texlib_bake_poll(
    mut bake: ResMut<TexLibBake>,
    tex: Res<TexLibImages>,
    mut images: ResMut<Assets<Image>>,
) {
    if bake.done {
        return;
    }
    let msg = {
        let guard = bake.rx.lock().expect("texlib bake channel lock poisoned");
        match guard.as_ref() {
            Some(rx) => rx.try_recv(),
            None => return,
        }
    };
    match msg {
        Ok(lib) => {
            bake.done = true;
            *bake.rx.lock().expect("lock") = None;
            println!(
                "[inkwave] texlib bake: {} of {} layers × {}px ({} mips) in {} ms",
                lib.layers_baked, lib.layers, lib.size, lib.mip_count, lib.ms
            );
            let (size, layers, mip_count) = (lib.size, lib.layers, lib.mip_count);
            let PackedLib {
                albedo,
                normal,
                orm,
                ..
            } = lib;
            if let Some(mut img) = images.get_mut(&tex.albedo) {
                apply_array(
                    &mut img,
                    size,
                    layers,
                    mip_count,
                    albedo,
                    TextureFormat::Rgba8UnormSrgb,
                );
            }
            if let Some(mut img) = images.get_mut(&tex.normal) {
                apply_array(
                    &mut img,
                    size,
                    layers,
                    mip_count,
                    normal,
                    TextureFormat::Rgba8Unorm,
                );
            }
            if let Some(mut img) = images.get_mut(&tex.orm) {
                apply_array(
                    &mut img,
                    size,
                    layers,
                    mip_count,
                    orm,
                    TextureFormat::Rgba8Unorm,
                );
            }
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            // Worker panicked: keep the placeholders (neutral shading path)
            // and stop polling.
            bake.done = true;
            println!("[inkwave] texlib bake worker died — keeping placeholder textures");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texlib::schema::embedded_texlib;

    #[test]
    fn mip_count_for_pins() {
        assert_eq!(mip_count_for(1), 1);
        assert_eq!(mip_count_for(32), 6);
        assert_eq!(mip_count_for(256), 9);
        assert_eq!(mip_count_for(512), 10);
    }

    #[test]
    fn chain_bytes_matches_wgpu_layer_layout() {
        assert_eq!(chain_bytes(1), 4);
        assert_eq!(chain_bytes(2), (4 + 1) * 4);
        assert_eq!(chain_bytes(4), (16 + 4 + 1) * 4);
        assert_eq!(
            chain_bytes(512),
            (512 * 512
                + 256 * 256
                + 128 * 128
                + 64 * 64
                + 32 * 32
                + 16 * 16
                + 8 * 8
                + 4 * 4
                + 2 * 2
                + 1)
                * 4
        );
    }

    #[test]
    fn used_layer_indices_dedups_and_resolves() {
        let doc = embedded_texlib();
        let idx = used_layer_indices(&doc);
        // No duplicates, all resolvable, and the slot table's first entry is
        // concrete (layer 0).
        let mut uniq = idx.clone();
        uniq.sort_unstable();
        let n = uniq.len();
        uniq.dedup();
        assert_eq!(uniq.len(), n, "no layer baked twice");
        assert!(idx.iter().all(|&i| i < doc.layers.len()));
        assert!(idx.contains(&0));
        // gel / detail are referenced by no slot → never baked.
        for (i, l) in doc.layers.iter().enumerate() {
            if l.name == "gel" || l.name == "detail" {
                assert!(!idx.contains(&i), "{} must stay unbaked", l.name);
            }
        }
        // concrete appears three times in the map but once in the set; the
        // stage slots must all be present (tidewater map covers 28..48).
        assert!(idx.len() < doc.layers.len());
        for name in &doc.slots.map {
            let i = super::super::schema::layer_index(&doc, name).unwrap();
            assert!(idx.contains(&i), "slot layer {name} missing");
        }
        // Every used layer resolves to a program.
        for &i in &idx {
            assert!(
                resolve_program(&doc.layers[i].name).is_some(),
                "no program for {}",
                doc.layers[i].name
            );
        }
    }

    #[test]
    fn pack_layers_offsets_and_neutral_fill() {
        let doc = embedded_texlib();
        let size = 4;
        let per = chain_bytes(size);
        // One synthetic baked layer at index 3 (mip 0 all bytes 0x5A;
        // `build_mips` derives the chain from the base level).
        let chain = bake::build_mips(vec![0x5Au8; (size * size * 4) as usize], size);
        let b = BakedLayer {
            albedo: chain.clone(),
            normal: chain.clone(),
            orm: chain.clone(),
        };
        let lib = pack_layers(&doc, &[(3, b)], size, 1);
        assert_eq!(lib.layers as usize, doc.layers.len());
        assert_eq!(lib.mip_count, 3);
        assert_eq!(lib.albedo.len(), per * doc.layers.len());
        // Slot 3 carries the baked bytes at mip 0 and the 1×1 tail.
        assert_eq!(lib.albedo[3 * per], 0x5A);
        assert_eq!(lib.albedo[3 * per + per - 4], 0x5A);
        // Slot 4 is neutral: albedo 200, normal 128/128/255, orm 255/209/0.
        let o = 4 * per;
        assert_eq!(&lib.albedo[o..o + 4], &[200, 200, 200, 255]);
        assert_eq!(&lib.normal[o..o + 4], &[128, 128, 255, 255]);
        assert_eq!(&lib.orm[o..o + 4], &[255, 209, 0, 255]);
        // mip 1 of slot 4 (after the 4×4 block) is the same constant.
        assert_eq!(&lib.albedo[o + 64..o + 68], &[200, 200, 200, 255]);
    }

    #[test]
    fn bake_all_small_size_is_complete() {
        let doc = embedded_texlib();
        let lib = bake_all(&doc, 4);
        assert_eq!(lib.layers_baked, used_layer_indices(&doc).len());
        assert_eq!(lib.size, 4);
        assert_eq!(lib.mip_count, 3);
        // A real layer's mip 0 differs from the neutral constant.
        assert_ne!(&lib.albedo[..4], &[200, 200, 200, 255]);
    }

    /// TR-52 perf red line: full slot-referenced bake at both tier sizes,
    /// printed for the batch E report. Run with
    /// `cargo test --release -p inkwave -- --ignored --nocapture bake_timing`.
    #[test]
    #[ignore = "bake timing measurement (release only)"]
    fn bake_timing_report() {
        let doc = embedded_texlib();
        for size in [32u32, 512] {
            let lib = bake_all(&doc, size);
            println!(
                "[inkwave] texlib bake perf: {} layers × {}px ({} mips) in {} ms",
                lib.layers_baked, size, lib.mip_count, lib.ms
            );
        }
    }

    #[test]
    fn bake_poll_swaps_placeholders_in_place() {
        use bevy::asset::AssetPlugin;
        let doc = embedded_texlib();
        let mut images = Assets::<Image>::default();
        let tex = crate::level_render::add_texlib_placeholders(&doc, &mut images);
        // Run the worker synchronously through the channel.
        let (tx, rx) = std::sync::mpsc::channel();
        let d = doc.clone();
        std::thread::spawn(move || {
            let _ = tx.send(bake_all(&d, 4));
        });
        let bake = TexLibBake {
            rx: Arc::new(Mutex::new(Some(rx))),
            done: false,
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .insert_resource(bake)
            .insert_resource(tex.clone())
            .insert_resource(images);
        app.add_systems(Update, texlib_bake_poll);
        // Poll until the worker lands (size-4 bake is milliseconds).
        for _ in 0..2000 {
            app.update();
            if app.world().resource::<TexLibBake>().done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.world().resource::<TexLibBake>().done);
        let images = app.world().resource::<Assets<Image>>();
        let albedo = images.get(&tex.albedo).unwrap();
        assert_eq!(albedo.texture_descriptor.size.width, 4);
        assert_eq!(albedo.texture_descriptor.mip_level_count, 3);
        assert_eq!(
            albedo.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            albedo.texture_descriptor.size.depth_or_array_layers,
            doc.layers.len() as u32
        );
        assert_eq!(albedo.data_order, TextureDataOrder::LayerMajor);
        assert!(matches!(albedo.sampler, ImageSampler::Descriptor(_)));
        let s = match &albedo.sampler {
            ImageSampler::Descriptor(d) => d,
            _ => unreachable!(),
        };
        assert_eq!(s.address_mode_u, ImageAddressMode::Repeat);
        assert_eq!(s.anisotropy_clamp, 16);
        let normal = images.get(&tex.normal).unwrap();
        assert_eq!(normal.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        assert_eq!(
            normal.data.as_ref().unwrap().len(),
            chain_bytes(4) * doc.layers.len()
        );
    }

    #[test]
    fn inert_bake_never_touches_images() {
        // Headless gating stand-in: without `start_bake` (never called when
        // `WorldPlugin` is absent) the poll keeps the placeholders.
        use bevy::asset::AssetPlugin;
        let doc = embedded_texlib();
        let mut images = Assets::<Image>::default();
        let tex = crate::level_render::add_texlib_placeholders(&doc, &mut images);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .insert_resource(TexLibBake::inert())
            .insert_resource(tex.clone())
            .insert_resource(images);
        app.add_systems(Update, texlib_bake_poll);
        app.update();
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(
            images
                .get(&tex.albedo)
                .unwrap()
                .texture_descriptor
                .size
                .width,
            1
        );
    }
}
