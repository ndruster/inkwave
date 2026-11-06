//! CPU bake core: renders one surf program into the three texlib layers
//! (≡ GEN_MAIN, texlib.js L171-201, + `createTextureLibrary` L1332-1424).
//!
//! Batch B of M4 Task 52 (spec D-T52-2). The JS original renders each layer
//! through a GPU MRT pass (3 × RGBA8 attachments, 2×2 sub-samples per texel,
//! hardware sRGB encode on attachment 0, driver `generateMipmap` at the end).
//! This is the CPU translation: same f32 math, same channel semantics, no
//! GPU dependency.
//!
//! **Declared deviations (D-T52-7①/⑧)** — byte-parity is *self-consistency*
//! (same binary ⇒ same bytes), not parity with the JS GPU output:
//! - UNORM8 store uses round-half-away-from-zero (`.round()`); the GL spec's
//!   float→fixed conversion is round-to-nearest (tie rule not pinned here —
//!   exact `.5` byte boundaries are measure-zero in the noise fields).
//! - sRGB encode uses the canonical `0.0031308` knee (≡ the inverse of
//!   [`r#gen::lin`], texlib.js L122); the GL hardware encoder's internal
//!   threshold may differ by an ulp near the knee.
//! - The mip chain: JS relies on `gl.generateMipmap` (WebGLTextures.js L116),
//!   a GPU driver box filter whose rounding/tap behaviour is
//!   implementation-defined — there is **no explicit JS function to port**.
//!   [`build_mips`] is therefore a self-defined deterministic chain: 2×2
//!   neighbourhood per output texel, edge indices clamped (replicate),
//!   per-channel integer average with round-half-up (`(a+b+c+d+2) >> 2`) on
//!   the stored bytes (sRGB channels are averaged in encoded space —
//!   drivers may decode/encode; the deviation is cosmetic and registered
//!   for the Task 55 visual pass).
//! - Data rows: `row 0` ↔ `gl_FragCoord.y = 0.5` ↔ `v = 0` (render-target
//!   semantics, `flipY = false`).

use super::r#gen::{self, Req, WReq};
use super::schema::TexLayer;

/// Surface result of one sub-sample evaluation (≡ `struct S`, texlib.js
/// L111). Heights in metres (0 = nominal top), colours linear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surf {
    /// linear albedo (`alb`).
    pub alb: [f32; 3],
    /// alpha mask, 1 = solid (`a`).
    pub a: f32,
    /// height in metres (`h`).
    pub h: f32,
    /// cavity/AO, 1 = open (`cav`).
    pub cav: f32,
    pub rough: f32,
    pub metal: f32,
}

impl Surf {
    /// The GEN_MAIN default (≡ texlib.js L187: `s.alb = vec3(0.8); s.a = 1;
    /// s.h = 0; s.cav = 1; s.rough = 0.8; s.metal = 0`).
    pub const NEUTRAL: Surf = Surf {
        alb: [0.8, 0.8, 0.8],
        a: 1.0,
        h: 0.0,
        cav: 1.0,
        rough: 0.8,
        metal: 0.0,
    };
}

/// Per-sub-sample context handed to a prep pass (≡ `PREP_SIG` uv/P,
/// texlib.js L208). The GLSL `PX` global (L114/L175) is not threaded here:
/// noise *declarations* never read it (only `surf` AA does).
#[derive(Debug, Clone, Copy)]
pub struct PrepCtx {
    /// tile coordinate, 1 = one repeat (`uv`).
    pub uv: [f32; 2],
    /// surface position in metres (`P = uv * uScale`, L179).
    pub p: [f32; 2],
}

/// Per-sub-sample context handed to a surf pass (≡ `SURF_SIG` uv/P,
/// texlib.js L209; `px` = the GLSL `PX` half-texel AA unit, L114/L175).
/// The layer meta stays on the bake loop itself (hr/ao are resolved there);
/// no surf body reads it.
#[derive(Debug, Clone)]
pub struct SurfCtx {
    pub uv: [f32; 2],
    pub p: [f32; 2],
    pub px: f32,
}

/// One material's noise declaration + surface evaluation — the CPU shape of
/// the JS `prep` / `surf` GLSL bodies (texlib.js L204-209). Batch C provides
/// the 46 implementations.
pub trait SurfProgram {
    /// Declare noise requests (≡ `prep`, PREP_SIG L208). `f`/`w` arrive as
    /// the GEN_MAIN defaults (L181-182: `oct = 0` / `n = 0` ⇒ inert).
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]);
    /// Fill the surface (≡ `surf`, SURF_SIG L209). `n` = the four
    /// [`r#gen::fbm_r`] results, `c` = the two [`r#gen::wor_r`] results, `s`
    /// arrives as [`Surf::NEUTRAL`] (L187).
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf);
}

/// One mip level: `size × size` RGBA8, row 0 = `v = 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipLevel {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// The full chain, `levels[0]` = base level (≡ `generateMipmaps: true`,
/// texlib.js L1341).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipChain {
    pub levels: Vec<MipLevel>,
}

/// One baked layer across the three attachments (≡ `out.textures[0..3]`
/// slice for layer `i`, texlib.js L1353/L1395).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakedLayer {
    /// rgb albedo (sRGB-encoded) + alpha mask (linear) — attachment 0.
    pub albedo: MipChain,
    /// tangent-space normal, linear — attachment 1.
    pub normal: MipChain,
    /// r cavity · g roughness · b metalness · a height — attachment 2.
    pub orm: MipChain,
}

/// Bake `layer` with `prog` at `size × size` (≡ one
/// `renderer.render(scene, cam)` pass with `uMat` set, texlib.js L1387-1397;
/// `size` = `uRes.x`, `layer.scale` = `uScale`, `layer.hr` = `uHRange`,
/// `layer.ao` = `uAO`).
#[must_use]
pub fn bake_layer(layer: &TexLayer, prog: &dyn SurfProgram, size: u32) -> BakedLayer {
    let scale = layer.scale;
    let hr = layer.hr;
    let ao = layer.ao;
    // L175: PX = uScale / (2.0 * uRes.x) — uOne = 1, so the rolled loops
    // unroll to their literal trip counts here.
    let px = scale / (2.0 * size as f32);
    let n_px = (size * size) as usize;
    let mut albedo = vec![0u8; n_px * 4];
    let mut normal = vec![0u8; n_px * 4];
    let mut orm = vec![0u8; n_px * 4];
    let sf = size as f32;

    // L177-193: 2×2 sub-samples per texel, offsets (k&1, k>>1 - 0.5) * 0.5
    // around the texel centre (gl_FragCoord.xy = texel + 0.5, L178).
    const OFF: [[f32; 2]; 4] = [[-0.25, -0.25], [0.25, -0.25], [-0.25, 0.25], [0.25, 0.25]];
    for y in 0..size {
        for x in 0..size {
            let (mut alb, mut alpha, mut cav, mut rough, mut metal) =
                ([0.0f32; 3], 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut hs = [0.0f32; 4];
            for (k, off) in OFF.iter().enumerate() {
                let uv = [
                    (x as f32 + 0.5 + off[0]) / sf,
                    (y as f32 + 0.5 + off[1]) / sf,
                ];
                let p = [uv[0] * scale, uv[1] * scale];
                let mut f = [Req::default(); 4];
                let mut w = [WReq::default(); 2];
                prog.prep(&PrepCtx { uv, p }, &mut f, &mut w);
                let mut n = [0.0f32; 4];
                for q in 0..4 {
                    n[q] = r#gen::fbm_r(f[q]);
                }
                let mut c = [[0.0f32; 4]; 2];
                for q in 0..2 {
                    c[q] = r#gen::wor_r(w[q]);
                }
                let mut s = Surf::NEUTRAL;
                prog.surf(&SurfCtx { uv, p, px }, &n, &c, &mut s);
                // L189-192.
                for (acc, v) in alb.iter_mut().zip(s.alb) {
                    *acc += v.max(0.0);
                }
                alpha += r#gen::clamp01(s.a);
                cav += r#gen::clamp01(s.cav) * (1.0 - ao * r#gen::clamp01(-s.h / -hr[0]));
                #[allow(clippy::manual_clamp)] // NaN-safe, see r#gen::clamp01
                {
                    rough += s.rough.max(0.02).min(1.0);
                }
                metal += r#gen::clamp01(s.metal);
                hs[k] = s.h;
            }
            // L194-199: normal from the sub-sample height gradient.
            let dx = (hs[1] - hs[0] + hs[3] - hs[2]) / (2.0 * px);
            let dy = (hs[2] - hs[0] + hs[3] - hs[1]) / (2.0 * px);
            let h = (hs[0] + hs[1] + hs[2] + hs[3]) * 0.25;
            let idx = ((y * size + x) as usize) * 4;
            // attachment 0: hardware sRGB on rgb, alpha linear (L106/L197).
            for ch in 0..3 {
                albedo[idx + ch] = store8(srgb_encode(r#gen::clamp01(alb[ch] * 0.25)));
            }
            albedo[idx + 3] = store8(r#gen::clamp01(alpha * 0.25));
            // attachment 1: normalize(vec3(-dx, -dy, 1)) * 0.5 + 0.5 (L198).
            let inv = 1.0 / (dx * dx + dy * dy + 1.0).sqrt();
            normal[idx] = store8(r#gen::clamp01(-dx * inv * 0.5 + 0.5));
            normal[idx + 1] = store8(r#gen::clamp01(-dy * inv * 0.5 + 0.5));
            normal[idx + 2] = store8(r#gen::clamp01(inv * 0.5 + 0.5));
            normal[idx + 3] = 255;
            // attachment 2: cavity, roughness, metalness, height→orm.a (L199).
            orm[idx] = store8(r#gen::clamp01(cav * 0.25));
            orm[idx + 1] = store8(r#gen::clamp01(rough * 0.25));
            orm[idx + 2] = store8(r#gen::clamp01(metal * 0.25));
            orm[idx + 3] = store8(r#gen::clamp01((h - hr[0]) / (hr[1] - hr[0])));
        }
    }
    BakedLayer {
        albedo: build_mips(albedo, size),
        normal: build_mips(normal, size),
        orm: build_mips(orm, size),
    }
}

/// Build the mip chain down to 1×1 (≡ `generateMipmaps: true`, texlib.js
/// L1341 — see the module docs for the box-filter definition).
#[must_use]
pub fn build_mips(rgba: Vec<u8>, size: u32) -> MipChain {
    let mut levels = Vec::new();
    let (mut data, mut s) = (rgba, size);
    loop {
        levels.push(MipLevel {
            size: s,
            rgba: data,
        });
        if s == 1 {
            break;
        }
        let next = (s / 2).max(1);
        data = box_down(&levels.last().expect("just pushed").rgba, s, next);
        s = next;
    }
    MipChain { levels }
}

/// One 2×2 box downsample with clamped edge taps (see module docs).
fn box_down(src: &[u8], src_size: u32, dst_size: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_size * dst_size * 4) as usize];
    for dy in 0..dst_size {
        for dx in 0..dst_size {
            let sx = (dx * 2).min(src_size - 1);
            let sy = (dy * 2).min(src_size - 1);
            // odd edge: the +1 tap clamps back onto the last row/column.
            let ex = (dx * 2 + 1).min(src_size - 1);
            let ey = (dy * 2 + 1).min(src_size - 1);
            let at = |x: u32, y: u32, ch: usize| src[((y * src_size + x) as usize) * 4 + ch];
            let di = ((dy * dst_size + dx) as usize) * 4;
            for ch in 0..4 {
                let sum = at(sx, sy, ch) as u32
                    + at(ex, sy, ch) as u32
                    + at(sx, ey, ch) as u32
                    + at(ex, ey, ch) as u32;
                dst[di + ch] = ((sum + 2) >> 2) as u8;
            }
        }
    }
    dst
}

/// GL float→UNORM8 store: clamp to `[0, 1]`, scale to 255, round
/// half-away-from-zero (see module docs for the tie deviation).
#[must_use]
pub fn store8(v: f32) -> u8 {
    (r#gen::clamp01(v) * 255.0).round() as u8
}

/// Hardware sRGB encode of attachment 0 (≡ the inverse of `lin`, texlib.js
/// L122): `c <= 0.0031308 ? 12.92 c : 1.055 c^(1/2.4) - 0.055`.
#[must_use]
pub fn srgb_encode(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texlib::schema::embedded_texlib;

    struct Flat;
    impl SurfProgram for Flat {
        fn prep(&self, _ctx: &PrepCtx, _f: &mut [Req; 4], _w: &mut [WReq; 2]) {}
        fn surf(&self, _ctx: &SurfCtx, _n: &[f32; 4], _c: &[[f32; 4]; 2], _s: &mut Surf) {}
    }

    /// Synthetic batch-C stand-in: one periodic fbm drives height/colour.
    struct MiniFbm;
    impl SurfProgram for MiniFbm {
        fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
            f[0] = r#gen::fb(ctx.uv, [4, 4], 3, 0.5, 7);
        }
        fn surf(&self, _ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
            let v = n[0];
            s.alb = [0.5 + 0.4 * v, 0.45 + 0.3 * v, 0.4 + 0.2 * v];
            s.h = v * 0.001;
            s.rough = 0.6 + 0.2 * v;
            s.cav = 0.9;
        }
    }

    #[test]
    fn neutral_surf_defaults() {
        // ≡ texlib.js L187.
        let s = Surf::NEUTRAL;
        assert_eq!(s.alb, [0.8, 0.8, 0.8]);
        assert_eq!(
            (s.a, s.h, s.cav, s.rough, s.metal),
            (1.0, 0.0, 1.0, 0.8, 0.0)
        );
    }

    #[test]
    fn store8_pins() {
        assert_eq!(store8(0.0), 0);
        assert_eq!(store8(1.0), 255);
        assert_eq!(store8(-0.2), 0);
        assert_eq!(store8(1.7), 255);
        assert_eq!(store8(0.5), 128); // 127.5, half away from zero
        assert_eq!(store8(0.2), 51); // 51.0 exactly
    }

    #[test]
    fn srgb_encode_pins() {
        assert_eq!(srgb_encode(0.0), 0.0);
        assert!((srgb_encode(0.5) - 0.7353569831f64 as f32).abs() < 1e-5);
        assert_eq!(store8(srgb_encode(0.5)), 188); // node reference
        assert_eq!(store8(srgb_encode(1.0)), 255);
        // round-trip against lin() (texlib.js L122).
        for c in [0.0f32, 0.01, 0.2, 0.5, 0.9, 1.0] {
            let back = r#gen::lin([srgb_encode(c), 0.0, 0.0])[0];
            assert!((back - c).abs() < 1e-5, "lin(srgb({c})) = {back}");
        }
    }

    #[test]
    fn box_down_rounds_and_clamps() {
        // 2×2 → 1: (1+2+3+4+2)>>2 = 3 per channel.
        let src: Vec<u8> = (1..=4u8).flat_map(|v| [v, v, v, v]).collect();
        assert_eq!(box_down(&src, 2, 1), vec![3, 3, 3, 3]);
        // 3×3 → 2: the bottom-right tap clamps onto the edge pixel (index 8).
        let mut src = vec![0u8; 9 * 4];
        for i in 0..9u8 {
            for ch in 0..4 {
                src[i as usize * 4 + ch] = i * 10;
            }
        }
        let dst = box_down(&src, 3, 2);
        // dst(1,1) taps src(2,2),(2,2),(2,2),(2,2) = 80 (byte 12 = ch 0).
        assert_eq!(&dst[12..13], &[80]);
        // dst(0,0) = (0+10+30+40+2)>>2 = 20.
        assert_eq!(dst[0], 20);
    }

    #[test]
    fn build_mips_shape() {
        let chain = build_mips(vec![7u8; 32 * 32 * 4], 32);
        let sizes: Vec<u32> = chain.levels.iter().map(|l| l.size).collect();
        assert_eq!(sizes, vec![32, 16, 8, 4, 2, 1]);
        assert_eq!(chain.levels[0].rgba.len(), 32 * 32 * 4);
        // flat input stays flat down to 1×1.
        assert_eq!(chain.levels[5].rgba, vec![7, 7, 7, 7]);
        let one = build_mips(vec![0u8; 4], 1);
        assert_eq!(one.levels.len(), 1);
    }

    #[test]
    fn flat_program_channel_semantics() {
        // NEUTRAL everywhere: albedo 0.8 sRGB, alpha 1, flat normal,
        // cav 1 (ao 0.5 · height 0 ⇒ no occlusion), rough 0.8, metal 0,
        // orm.a = (0 - hr0)/(hr1 - hr0) for the concrete layer.
        let layer = &embedded_texlib().layers[0];
        assert_eq!(layer.name, "concrete");
        let b = bake_layer(layer, &Flat, 4);
        for mip in [&b.albedo, &b.normal, &b.orm] {
            assert_eq!(mip.levels[0].size, 4);
        }
        fn px(chain: &MipChain) -> &[u8] {
            &chain.levels[0].rgba[0..4]
        }
        assert_eq!(
            px(&b.albedo),
            &[
                store8(srgb_encode(0.8)),
                store8(srgb_encode(0.8)),
                store8(srgb_encode(0.8)),
                255
            ]
        );
        assert_eq!(px(&b.normal), &[128, 128, 255, 255]); // (0.5, 0.5, 1)
        assert_eq!(px(&b.orm)[0], 255); // cav
        assert_eq!(px(&b.orm)[1], 204); // 0.8 * 255
        assert_eq!(px(&b.orm)[2], 0); // metal
        // concrete hr = [-0.004, 0.0008] (texlib.js L213): h = 0 maps to
        // ~5/6 of the range; the f32 quotient lands just over 212.5.
        assert_eq!(px(&b.orm)[3], 213);
    }

    #[test]
    fn bake_is_byte_deterministic() {
        let layer = &embedded_texlib().layers[0];
        let a = bake_layer(layer, &MiniFbm, 32);
        let b = bake_layer(layer, &MiniFbm, 32);
        assert_eq!(a, b);
        assert_eq!(a.albedo.levels.len(), 6);
    }

    #[test]
    fn mini_fbm_golden_sha256() {
        // 32 px representative-layer golden (spec D-T52-2 / TR-52.1).
        let layer = &embedded_texlib().layers[0];
        let b = bake_layer(layer, &MiniFbm, 32);
        let mut data = Vec::new();
        for chain in [&b.albedo, &b.normal, &b.orm] {
            data.extend_from_slice(&chain.levels[0].rgba);
        }
        assert_eq!(hex(&sha256(&data)), GOLDEN_MINI_FBM_32);
    }

    /// Pinned after the first deterministic run (batch B).
    const GOLDEN_MINI_FBM_32: &str =
        "a200252ad7698253e646208508bbcf0788c2f61cb46b6cb72391d65b8ef427f9";

    // -- std-only SHA-256 (test fixture pinning; FIPS 180-4) ---------------
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn sha256(msg: &[u8]) -> [u8; 32] {
        let mut h = [
            0x6a09e667u32,
            0xbb67ae85,
            0x3c6ef372,
            0xa54ff53a,
            0x510e527f,
            0x9b05688c,
            0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut data = msg.to_vec();
        data.push(0x80);
        while data.len() % 64 != 56 {
            data.push(0);
        }
        data.extend_from_slice(&((msg.len() as u64) * 8).to_be_bytes());
        for block in data.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
            let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (x, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *x = x.wrapping_add(v);
            }
        }
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }

    fn hex(d: &[u8; 32]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_self_test() {
        // node crypto ground truth.
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
