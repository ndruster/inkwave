//! M5 Task 66 — environment bake DATA PLANE, ported from
//! `src/world/environment.js` (cloud bake / PMREM / far+planar reflection /
//! sea swell+marina+foam). All constants come from the pinned
//! `assets/environment.json` `bake` section, independently re-parsed by
//! `tools/environment-crosscheck.mjs` (139 checks).
//!
//! D-T66-1: the cloud ray-march shader, PMREM cube, planar/far reflection and
//! foam/caustic GPU visuals cannot be validated in the sandbox — only the
//! parameter data plane is tested; visuals are a user-machine REVIEW_CHECKLIST
//! item. D-T66-2: bake artifacts are regenerated deterministically at runtime
//! from the seed/descriptor (no ~10 MB cloud RT in wasm); the wave texture bake
//! here uses the same mulberry32 as upstream `makeWaveTexture` and is pinned by
//! a cross-language SHA256 golden (`bake.wave_sha256` in environment.json,
//! produced by the independent JS reference in extract-environment.mjs and
//! re-verified by environment-crosscheck.mjs).

use std::sync::OnceLock;

use inkwave_sim::net::wire::{JVal, parse_json};

use crate::environment::Theme;

const ENV_DOC: &str = include_str!("../../../assets/environment.json");

fn env_doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(ENV_DOC).expect("environment.json parses"))
}

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("environment.json bake: missing {key}"))
}

fn num(v: &JVal, key: &str) -> f64 {
    field(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("environment.json bake: missing numeric {key}"))
}

fn arr<'a>(v: &'a JVal, key: &str) -> &'a [JVal] {
    field(v, key)
        .as_array()
        .unwrap_or_else(|| panic!("environment.json bake: missing array {key}"))
}

fn bake() -> &'static JVal {
    field(env_doc(), "bake")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudBake {
    pub width: usize,
    pub height: usize,
    pub seed: f64,
    pub coverage: f64,
    pub scan_bands: usize,
}

pub fn cloud() -> CloudBake {
    let c = field(bake(), "cloud");
    CloudBake {
        width: num(c, "width") as usize,
        height: num(c, "height") as usize,
        seed: num(c, "seed"),
        coverage: num(c, "coverage"),
        scan_bands: num(c, "scan_bands") as usize,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PmremSpec {
    pub size: usize,
    pub env_k_default: f64,
}

pub fn pmrem() -> PmremSpec {
    let p = field(bake(), "pmrem");
    PmremSpec {
        size: num(p, "size") as usize,
        env_k_default: num(p, "env_k_default"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionSpec {
    pub cube_size: usize,
    pub cube_near: f64,
    pub cube_far: f64,
}

pub fn far_reflection() -> ReflectionSpec {
    let f = field(bake(), "far");
    ReflectionSpec {
        cube_size: num(f, "cube_size") as usize,
        cube_near: num(f, "cube_near"),
        cube_far: num(f, "cube_far"),
    }
}

/// Planar reflection scale per tier (low = 0 → off, the保底 path).
pub fn reflection_tier_scale(tier: &str) -> f64 {
    num(field(field(bake(), "reflection"), "tier_scale"), tier)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeaSpec {
    pub wave_texture_size: usize,
    pub wave_seed: u32,
    pub wave_count: usize,
    pub wind: f64,
    pub freq_rnd_pow: f64,
    pub freq_max: f64,
    pub amp_pow: f64,
}

pub fn sea() -> SeaSpec {
    let s = field(bake(), "sea");
    SeaSpec {
        wave_texture_size: num(s, "wave_texture_size") as usize,
        wave_seed: num(s, "wave_seed") as u32,
        wave_count: num(s, "wave_count") as usize,
        wind: num(s, "wind"),
        freq_rnd_pow: num(s, "freq_rnd_pow"),
        freq_max: num(s, "freq_max"),
        amp_pow: num(s, "amp_pow"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwellComponent {
    pub k: [f64; 2],
    pub phase: f64,
    pub speed: f64,
    pub amp: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwellSpec {
    pub components: Vec<SwellComponent>,
    pub amp_min: f64,
    pub amp_max: f64,
    pub deck_min: f64,
    pub deck_max: f64,
}

pub fn swell() -> SwellSpec {
    let s = field(bake(), "swell");
    let components = arr(s, "components")
        .iter()
        .map(|c| {
            let k = arr(c, "k");
            SwellComponent {
                k: [k[0].as_f64().unwrap(), k[1].as_f64().unwrap()],
                phase: num(c, "phase"),
                speed: num(c, "speed"),
                amp: num(c, "amp"),
            }
        })
        .collect();
    SwellSpec {
        components,
        amp_min: num(s, "amp_min"),
        amp_max: num(s, "amp_max"),
        deck_min: num(s, "deck_min"),
        deck_max: num(s, "deck_max"),
    }
}

/// Deterministic CPU bake of the sea wave texture descriptors ≡ upstream
/// `makeWaveTexture(size, seed)` wave construction (environment.js L1262-1273).
/// Returns the 44 `(kx, ky, amp, phase)` tuples as a byte buffer; hashed in the
/// anchor test to pin bit-determinism without storing a ~256² float texture.
pub fn bake_wave_descriptors() -> Vec<(i32, i32, f64, f64)> {
    use crate::char::math::Rng;
    let s = sea();
    let mut rnd = Rng::new(s.wave_seed);
    let mut waves = Vec::with_capacity(s.wave_count);
    for _ in 0..s.wave_count {
        let f = 2.0 + rnd.next().powf(s.freq_rnd_pow) * s.freq_max;
        let ang = s.wind + (rnd.next() - 0.5) * 2.4;
        let mut kx = (ang.cos() * f).round() as i32;
        let ky = (ang.sin() * f).round() as i32;
        if kx == 0 && ky == 0 {
            kx = 2;
        }
        let kl = (kx as f64).hypot(ky as f64);
        let amp = 1.0 / kl.powf(s.amp_pow);
        // JS: `rnd() * Math.PI * 2` — keep the exact multiplication order
        // ((rnd·π)·2) for bit-exact agreement with the JS golden; TAU would
        // change the f64 bits even though the value is mathematically equal.
        let phase = rnd.next() * std::f64::consts::PI * 2.0;
        waves.push((kx, ky, amp, phase));
    }
    waves
}

/// Theme-driven cloud/sea params resolved for the current env, using the
/// existing `Theme` accessors (Task 55). Returns the vec4-style cloud tuple
/// `[overhead, horizon, drift, bank_height]` and `envK`.
pub fn theme_cloud(theme: &Theme) -> [f64; 4] {
    let a = theme.array("cloud");
    [a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64]
}

pub fn theme_env_k(theme: &Theme) -> f64 {
    theme
        .opt_num("envK")
        .map(f64::from)
        .unwrap_or_else(|| pmrem().env_k_default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task66_bake_constants_match_extracted_js() {
        let c = cloud();
        assert_eq!((c.width, c.height), (2048, 640));
        assert_eq!(c.seed, 3.0);
        assert_eq!(c.coverage, 0.46);
        assert_eq!(c.scan_bands, 10);
        let p = pmrem();
        assert_eq!(p.size, 256);
        assert_eq!(p.env_k_default, 0.66);
        let f = far_reflection();
        assert_eq!((f.cube_size, f.cube_near, f.cube_far), (512, 2.0, 30000.0));
        assert_eq!(
            (
                reflection_tier_scale("low"),
                reflection_tier_scale("medium"),
                reflection_tier_scale("high"),
                reflection_tier_scale("ultra")
            ),
            (0.0, 0.28, 0.4, 0.5)
        );
        let s = sea();
        assert_eq!(
            (s.wave_texture_size, s.wave_seed, s.wave_count),
            (256, 11, 44)
        );
        assert_eq!(s.wind, 0.55);
        assert_eq!(s.freq_rnd_pow, 1.5);
        assert_eq!(s.freq_max, 26.0);
        assert_eq!(s.amp_pow, 1.3);
    }

    #[test]
    fn task66_swell_components_and_amp_are_pinned() {
        let s = swell();
        assert_eq!(s.components.len(), 3);
        assert_eq!(s.components[0].k, [0.110, 0.047]);
        assert_eq!(s.components[0].speed, 0.95);
        assert_eq!(s.components[0].amp, 0.45);
        assert_eq!(s.components[1].k, [-0.052, 0.097]);
        assert_eq!(s.components[1].speed, 1.13);
        assert_eq!(s.components[1].phase, 1.7);
        assert_eq!(s.components[1].amp, 0.35);
        assert_eq!(s.components[2].k, [0.173, -0.141]);
        assert_eq!(s.components[2].speed, 1.61);
        assert_eq!(s.components[2].phase, 4.1);
        assert_eq!(s.components[2].amp, 0.20);
        assert_eq!(
            (s.amp_min, s.amp_max, s.deck_min, s.deck_max),
            (0.035, 0.16, 3.0, 70.0)
        );
    }

    #[test]
    fn task66_wave_descriptors_are_byte_deterministic() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        fn hash() -> u64 {
            let waves = bake_wave_descriptors();
            let mut h = DefaultHasher::new();
            for (kx, ky, amp, phase) in &waves {
                kx.hash(&mut h);
                ky.hash(&mut h);
                amp.to_bits().hash(&mut h);
                phase.to_bits().hash(&mut h);
            }
            h.finish()
        }
        let a = hash();
        let b = hash();
        assert_eq!(a, b, "wave bake must be byte-deterministic across runs");
        let waves = bake_wave_descriptors();
        assert_eq!(waves.len(), 44);
        assert!(
            waves
                .iter()
                .all(|(kx, ky, amp, phase)| (*kx != 0 || *ky != 0)
                    && amp.is_finite()
                    && *amp > 0.0
                    && phase.is_finite()
                    && (0.0..std::f64::consts::TAU).contains(phase))
        );
    }

    fn wave_sha256() -> String {
        // Canonical bytes: per wave kx(i32 LE) + ky(i32 LE) +
        // round(amp·1e9)(i64 LE) + round(phase·1e9)(i64 LE) = 24 bytes —
        // identical to the JS reference in extract-environment.mjs. Amp/phase
        // are quantized so libm's last-bit pow rounding (Rust powf vs JS
        // Math.pow) cannot break the golden, while RNG/exponent/constant drift
        // still can.
        const WAVE_Q: f64 = 1e9;
        let mut bytes = Vec::with_capacity(44 * 24);
        for (kx, ky, amp, phase) in bake_wave_descriptors() {
            bytes.extend_from_slice(&kx.to_le_bytes());
            bytes.extend_from_slice(&ky.to_le_bytes());
            bytes.extend_from_slice(&((amp * WAVE_Q).round() as i64).to_le_bytes());
            bytes.extend_from_slice(&((phase * WAVE_Q).round() as i64).to_le_bytes());
        }
        hex(&sha256(&bytes))
    }

    #[test]
    fn task66_wave_bake_matches_js_sha256_golden() {
        // Golden produced by the independent JS reference implementation
        // (extract-environment.mjs) from the UPSTREAM makeWaveTexture formula,
        // pinned into environment.json → bake.wave_sha256. A formula/RNG drift
        // in either language breaks this cross-language anchor.
        let golden = field(bake(), "wave_sha256")
            .as_str()
            .expect("wave_sha256 hex");
        assert_eq!(golden.len(), 64, "SHA256 hex length");
        assert_eq!(wave_sha256(), golden, "Rust bake must match the JS golden");
    }

    #[test]
    fn task66_theme_cloud_and_env_k_are_finite() {
        for name in ["day", "sunset", "golden"] {
            let t = Theme::get(name).expect("theme exists");
            let c = theme_cloud(&t);
            assert!(
                c.iter().all(|v| v.is_finite() && *v >= 0.0),
                "{name} cloud finite"
            );
            let ek = theme_env_k(&t);
            assert!(ek > 0.0 && ek.is_finite(), "{name} envK finite");
        }
    }

    // -- std-only SHA-256 (FIPS 180-4), same fixture-pinning pattern as
    //    texlib/bake.rs; verified against node crypto via the JS golden above.
    const SHA_K: [u32; 64] = [
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
                    .wrapping_add(SHA_K[i])
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
    fn task66_sha256_self_test() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
