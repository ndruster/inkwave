//! CPU port of the texlib generator math (`GEN_COMMON`, ≡ texlib.js L97-169).
//!
//! Batch B of M4 Task 52 (spec D-T52-2). Every function mirrors one GLSL
//! helper line-for-line; `f32` matches GLSL `highp float` width and the
//! integer hash is exact `uint` arithmetic, so bake output is deterministic
//! on the CPU (D-T52-7⑧: same machine / same binary byte-exact, no
//! cross-backend bit-parity promise against the GPU original).
//!
//! The GLSL global `float PX` (L114, the AA width unit) is threaded through
//! as an explicit `px` parameter on [`aa`] / [`edge_prof`].
//!
//! Batch C (`texlib/materials.rs`) consumes these as the shared noise
//! library; batches D/E wire it into the render resource, after which the
//! module-level `#[allow(dead_code)]` in `mod.rs` can be dropped.

/// `#define PI` (≡ texlib.js L109). The GLSL literal is the source of
/// truth — not `std::f32::consts::PI` — so the value stays verbatim.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const PI: f32 = 3.14159265;
/// `#define TAU` (≡ texlib.js L110), verbatim GLSL literal.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const TAU: f32 = 6.28318531;

/// fbm request (≡ `struct Req`, texlib.js L112).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Req {
    /// evaluation point, lattice units (`x`).
    pub x: [f32; 2],
    /// period in lattice cells (`per`).
    pub per: [i32; 2],
    /// octave count (`oct`).
    pub oct: i32,
    /// per-octave amplitude gain (`gain`).
    pub gain: f32,
    /// hash seed (`seed`).
    pub seed: u32,
}

impl Default for Req {
    /// The GEN_MAIN neutral request (≡ texlib.js L181:
    /// `Req(vec2(0.0), ivec2(1), 0, 0.5, 0u)`) — `oct = 0` ⇒ [`fbm_r`] = 0.
    fn default() -> Self {
        Req {
            x: [0.0, 0.0],
            per: [1, 1],
            oct: 0,
            gain: 0.5,
            seed: 0,
        }
    }
}

/// worley request (≡ `struct WReq`, texlib.js L113).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WReq {
    /// tile-space coordinate, 1 = one repeat (`uv`).
    pub uv: [f32; 2],
    /// cells per repeat (`n`); `n.x == 0` ⇒ disabled.
    pub n: [i32; 2],
    /// feature-point jitter in cell units (`jit`).
    pub jit: f32,
    /// hash seed (`seed`).
    pub seed: u32,
}

impl Default for WReq {
    /// The GEN_MAIN disabled request (≡ texlib.js L182:
    /// `WReq(vec2(0.0), ivec2(0), 0.0, 0u)`) — [`wor_r`] = zeros.
    fn default() -> Self {
        WReq {
            uv: [0.0, 0.0],
            n: [0, 0],
            jit: 0.0,
            seed: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// GLSL built-in helpers (scalar/vec2 subset used by GEN_COMMON + batch C)
// ---------------------------------------------------------------------------

/// `mix(a, b, t) = a * (1 - t) + b * t` — GLSL definition, NOT `lerp`
/// (differs at t = 0/1 when a component is NaN/inf; mirrors GPU behaviour).
#[must_use]
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// `step(edge, x)`: `x < edge ? 0 : 1` (≡ GLSL).
#[must_use]
pub fn step(edge: f32, x: f32) -> f32 {
    if x < edge { 0.0 } else { 1.0 }
}

/// `clamp(x, 0, 1)`. NaN-safe (`max`/`min` drop NaN instead of panicking,
/// mirroring the undefined-but-stable GLSL result).
#[must_use]
#[allow(clippy::manual_clamp)] // f32::clamp panics on NaN in debug builds
pub fn clamp01(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}

/// `clamp(x, lo, hi)` (NaN-safe, as [`clamp01`]).
#[must_use]
#[allow(clippy::manual_clamp)]
pub fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    x.max(lo).min(hi)
}

/// `fract(x) = x - floor(x)` — always in `[0, 1)`, unlike Rust `%`.
#[must_use]
pub fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// GLSL `mod(a, b) = a - b * floor(a / b)` — unlike Rust `%` it follows the
/// sign of `b` (e.g. `glsl_mod(-1.5, 2.0) == 0.5`).
#[must_use]
pub fn glsl_mod(a: f32, b: f32) -> f32 {
    a - b * (a / b).floor()
}

/// `smoothstep(edge0, edge1, x)` (GLSL definition).
#[must_use]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp01((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

/// `dot(vec2, vec2)`.
#[must_use]
pub fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// `length(vec2)`.
#[must_use]
pub fn length2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

// ---------------------------------------------------------------------------
// Integer hash (≡ texlib.js L116-119)
// ---------------------------------------------------------------------------

/// `pcg` (≡ texlib.js L116). Exact wrapping `uint` arithmetic.
#[must_use]
pub fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    // (s >> 28u) + 4u is in [4, 19], so the shift is always < 32.
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

/// `hu` (≡ texlib.js L117). GLSL `uint(negative int)` is the two's-complement
/// bit pattern; Rust `i32 as u32` is the same reinterpretation.
#[must_use]
pub fn hu(p: [i32; 2], s: u32) -> u32 {
    pcg((p[0] as u32).wrapping_mul(0x8da6_b343)
        ^ (p[1] as u32).wrapping_mul(0xd816_3841)
        ^ s.wrapping_mul(0xcb1a_b31f))
}

/// `hf` (≡ texlib.js L118): `float(hu >> 8u) * 2^-24` — exact in f32
/// (the shifted value is < 2^24).
#[must_use]
pub fn hf(p: [i32; 2], s: u32) -> f32 {
    (hu(p, s) >> 8) as f32 * (1.0 / 16777216.0)
}

/// `hf2` (≡ texlib.js L119): two exact f32s from one hash + its pcg.
#[must_use]
pub fn hf2(p: [i32; 2], s: u32) -> [f32; 2] {
    let a = hu(p, s);
    [
        (a >> 8) as f32 * (1.0 / 16777216.0),
        (pcg(a) >> 8) as f32 * (1.0 / 16777216.0),
    ]
}

// ---------------------------------------------------------------------------
// Noise (≡ texlib.js L121-139)
// ---------------------------------------------------------------------------

/// `wrp` (≡ texlib.js L121): float-form periodic wrap into `[0, n)`.
/// The expression is integer-valued, so the `ivec2()` truncation is exact.
#[must_use]
pub fn wrp(i: [i32; 2], n: [i32; 2]) -> [i32; 2] {
    let fi = [i[0] as f32, i[1] as f32];
    let fn_ = [n[0] as f32, n[1] as f32];
    [
        (fi[0] - fn_[0] * ((fi[0] + 0.5) / fn_[0]).floor()) as i32,
        (fi[1] - fn_[1] * ((fi[1] + 0.5) / fn_[1]).floor()) as i32,
    ]
}

/// `gr` (≡ texlib.js L123): hash → two gradients in `[-1, 1]`.
#[must_use]
pub fn gr(h: u32) -> [f32; 2] {
    let s = 1.0 / 32767.5;
    [(h & 0xffff) as f32 * s - 1.0, (h >> 16) as f32 * s - 1.0]
}

/// `lin` (≡ texlib.js L122): per-channel sRGB → linear decode.
#[must_use]
pub fn lin(c: [f32; 3]) -> [f32; 3] {
    let one = |x: f32| mix(x / 12.92, ((x + 0.055) / 1.055).powf(2.4), step(0.04045, x));
    [one(c[0]), one(c[1]), one(c[2])]
}

/// `gn` (≡ texlib.js L126-134): periodic gradient noise, ~`[-1, 1]`.
/// `x` in lattice units, `n` = period in lattice cells.
#[must_use]
pub fn gn(x: [f32; 2], n: [i32; 2], s: u32) -> f32 {
    let i = [x[0].floor() as i32, x[1].floor() as i32];
    let f = [x[0] - i[0] as f32, x[1] - i[1] as f32];
    let quintic = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let u = [quintic(f[0]), quintic(f[1])];
    let v = |g: [i32; 2]| {
        dot2(
            gr(hu(wrp([i[0] + g[0], i[1] + g[1]], n), s)),
            [f[0] - g[0] as f32, f[1] - g[1] as f32],
        )
    };
    let (v0, v1, v2, v3) = (v([0, 0]), v([1, 0]), v([0, 1]), v([1, 1]));
    1.6 * mix(mix(v0, v1, u[0]), mix(v2, v3, u[0]), u[1])
}

/// `fbmR` (≡ texlib.js L135-139): normalised octave sum over a [`Req`].
/// `oct <= 0` ⇒ 0 (`nrm > 0.0 ? sum / nrm : 0.0`).
#[must_use]
pub fn fbm_r(r: Req) -> f32 {
    let (mut sum, mut amp, mut nrm) = (0.0f32, 1.0f32, 0.0f32);
    let (mut x, mut per) = (r.x, r.per);
    for o in 0..r.oct {
        sum += amp * gn(x, per, r.seed.wrapping_add((o as u32).wrapping_mul(1013)));
        nrm += amp;
        amp *= r.gain;
        x = [x[0] * 2.0, x[1] * 2.0];
        per = [per[0].wrapping_mul(2), per[1].wrapping_mul(2)];
    }
    if nrm > 0.0 { sum / nrm } else { 0.0 }
}

/// `worR` (≡ texlib.js L141-152): periodic worley, returns
/// `(F1, F2, id of nearest, id of 2nd)` in cell units. `n.x == 0` ⇒ zeros.
#[must_use]
pub fn wor_r(q: WReq) -> [f32; 4] {
    if q.n[0] == 0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let nf = [q.n[0] as f32, q.n[1] as f32];
    let x = [q.uv[0] * nf[0], q.uv[1] * nf[1]];
    let i = [x[0].floor() as i32, x[1].floor() as i32];
    let f = [x[0] - i[0] as f32, x[1] - i[1] as f32];
    let (mut f1, mut f2, mut id, mut id2) = (9.0f32, 9.0f32, 0.0f32, 0.0f32);
    for y in -1..=1 {
        for xx in -1..=1 {
            let w = wrp([i[0] + xx, i[1] + y], q.n);
            let o2 = hf2(w, q.seed);
            let o = [
                xx as f32 + 0.5 + (o2[0] - 0.5) * q.jit,
                y as f32 + 0.5 + (o2[1] - 0.5) * q.jit,
            ];
            let d = length2([o[0] - f[0], o[1] - f[1]]);
            if d < f1 {
                f2 = f1;
                f1 = d;
                id2 = id;
                id = hf(w, q.seed.wrapping_add(7));
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    [f1, f2, id, id2]
}

// ---------------------------------------------------------------------------
// Request builders + profile helpers (≡ texlib.js L154-168)
// ---------------------------------------------------------------------------

/// `FB` (≡ texlib.js L154): periodic fbm over the repeat, `f0` cells.
#[must_use]
pub fn fb(uv: [f32; 2], f0: [i32; 2], oct: i32, gain: f32, s: u32) -> Req {
    Req {
        x: [uv[0] * f0[0] as f32, uv[1] * f0[1] as f32],
        per: f0,
        oct,
        gain,
        seed: s,
    }
}

/// `FNP` (≡ texlib.js L155): "aperiodic" fbm on explicit coords (4096-cell
/// period, gain 0.5).
#[must_use]
pub fn fnp(x: [f32; 2], oct: i32, s: u32) -> Req {
    Req {
        x,
        per: [4096, 4096],
        oct,
        gain: 0.5,
        seed: s,
    }
}

/// `WO` (≡ texlib.js L156).
#[must_use]
pub fn wo(uv: [f32; 2], n: [i32; 2], jit: f32, s: u32) -> WReq {
    WReq {
        uv,
        n,
        jit,
        seed: s,
    }
}

/// `aa` (≡ texlib.js L158): AA'd edge over `±0.75 * PX`; `px` is the GLSL
/// global `PX` (L114) threaded as a parameter.
#[must_use]
pub fn aa(edge: f32, x: f32, px: f32) -> f32 {
    smoothstep(edge - px * 0.75, edge + px * 0.75, x)
}

/// `jd` (≡ texlib.js L159): distance to the nearest joint line of period `p`.
#[must_use]
pub fn jd(x: f32, p: f32) -> f32 {
    (x - p * ((x / p) + 0.5).floor()).abs()
}

/// `sdRB` (≡ texlib.js L160): signed distance to a rounded box.
#[must_use]
pub fn sd_rb(p: [f32; 2], b: [f32; 2], r: f32) -> f32 {
    let q = [p[0].abs() - b[0] + r, p[1].abs() - b[1] + r];
    length2([q[0].max(0.0), q[1].max(0.0)]) + q[0].max(q[1]).min(0.0) - r
}

/// `edgeProf` (≡ texlib.js L163-168): joint + rounded arris profile.
/// `e` = distance from the joint centre line (m), `jw` = half joint width,
/// `r` = rounding width, `de` = arris drop, `dj` = joint depth.
/// Returns `(height, jointMask)` — the GLSL `vec2`.
#[must_use]
pub fn edge_prof(e: f32, jw: f32, r: f32, de: f32, dj: f32, px: f32) -> [f32; 2] {
    let t = clamp01(1.0 - (e - jw) / r);
    let h_e = -de * (1.0 - (1.0 - t * t).max(0.0).sqrt());
    let in_j = 1.0 - aa(jw, e, px);
    [mix(h_e, -dj, in_j), in_j]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// f32-vs-node-f64 tolerance for the transcendental paths.
    const TOL: f32 = 1e-5;

    fn approx(a: f32, b: impl Into<f64>) {
        let b: f64 = b.into();
        assert!(
            ((a as f64) - b).abs() <= f64::from(TOL),
            "{a} vs reference {b}"
        );
    }

    #[test]
    fn pcg_pins_node_reference() {
        // node: Math.imul / >>> 0 replay of texlib.js L116 (exact uint).
        assert_eq!(pcg(0), 129708002);
        assert_eq!(pcg(1), 2831084092);
        assert_eq!(pcg(7), 2120684060);
        assert_eq!(pcg(12345), 4099845390);
        assert_eq!(pcg(u32::MAX), 3861530882);
    }

    #[test]
    fn hu_pins_node_reference() {
        assert_eq!(hu([0, 0], 0), 129708002);
        assert_eq!(hu([3, 5], 11), 2405990672);
        // negative coords: uint reinterpretation = two's complement (L117).
        assert_eq!(hu([-1, -2], 7), 1583319611);
        assert_eq!(hu([1, -3], 19), 3909394707);
    }

    #[test]
    fn hf_hf2_are_exact_f32() {
        // L118-119: (hu >> 8) / 2^24 is exact in f32; pinned against node.
        approx(hf([3, 5], 11), 0.5601883531);
        approx(hf([-1, -2], 7), 0.3686453104);
        assert_eq!(
            hf([3, 5], 11),
            (hu([3, 5], 11) >> 8) as f32 * (1.0 / 16777216.0)
        );
        let two = hf2([3, 5], 11);
        approx(two[0], 0.5601883531);
        approx(two[1], 0.9836179018);
        assert_eq!(two[0], hf([3, 5], 11));
    }

    #[test]
    fn gr_maps_to_unit_range() {
        // L123, node: [0.0083161669, 0.1203784237] for hu([3,5],11).
        let g = gr(hu([3, 5], 11));
        approx(g[0], 0.0083161669);
        approx(g[1], 0.1203784237);
        assert_eq!(gr(0), [-1.0, -1.0]);
        assert_eq!(gr(0xffff_0000), [-1.0, 1.0]); // low half = gr[0]
        assert_eq!(gr(u32::MAX), [1.0, 1.0]);
    }

    #[test]
    fn wrp_wraps_negatives_into_period() {
        // L121; node: wrp(-3,5|2,4)=[1,1], wrp(-1,-1|3,7)=[2,6].
        assert_eq!(wrp([-3, 5], [2, 4]), [1, 1]);
        assert_eq!(wrp([-1, -1], [3, 7]), [2, 6]);
        assert_eq!(wrp([5, 5], [2, 4]), [1, 1]);
        assert_eq!(wrp([0, 0], [1, 1]), [0, 0]);
        for iy in -9..9i32 {
            let w = wrp([3, iy], [2, 4]);
            assert!((0..2).contains(&w[0]) && (0..4).contains(&w[1]), "{w:?}");
        }
    }

    #[test]
    fn gn_pins_and_lattice_zero() {
        // L126-134; node f64 references above.
        assert_eq!(gn([0.0, 0.0], [3, 3], 11), 0.0); // f = 0 ⇒ v0 = 0 exactly
        approx(gn([0.3, 0.7], [3, 3], 11), -0.5304116816);
        approx(gn([2.5, -1.25], [8, 8], 19), 0.0879839613);
        approx(gn([1.5, 2.25], [4, 4], 5), -0.0167389682);
    }

    #[test]
    fn gn_is_periodic() {
        // x + n maps to the same wrapped cells.
        let a = gn([0.3, 0.7], [3, 3], 11);
        let b = gn([3.3, 3.7], [3, 3], 11); // node: identical to f64
        assert!((a - b).abs() <= 1e-6, "{a} vs {b}");
    }

    #[test]
    fn fbm_r_pins_and_empty() {
        // L135-139; node f64 references.
        approx(
            fbm_r(Req {
                x: [0.3, 0.7],
                per: [3, 3],
                oct: 5,
                gain: 0.55,
                seed: 11,
            }),
            -0.1790246305,
        );
        approx(
            fbm_r(Req {
                x: [2.5, -1.25],
                per: [8, 8],
                oct: 3,
                gain: 0.5,
                seed: 19,
            }),
            0.1307744294,
        );
        assert_eq!(fbm_r(Req::default()), 0.0); // oct = 0 ⇒ nrm = 0 ⇒ 0
    }

    #[test]
    fn wor_r_pins_and_invariants() {
        // L141-152; node f64 reference for uv=(0.3,0.7), n=64, jit=0.8, seed=37.
        let c = wor_r(wo([0.3, 0.7], [64, 64], 0.8, 37));
        approx(c[0], 0.6967301384);
        approx(c[1], 1.0002889689);
        approx(c[2], 0.9394593239);
        approx(c[3], 0.3627473712);
        let c2 = wor_r(wo([0.9, 0.1], [3, 3], 1.0, 5));
        approx(c2[0], 0.7004820989);
        approx(c2[1], 0.721565993);
        approx(c2[2], 0.2607318163);
        approx(c2[3], 0.8356996179);
        assert!(c[0] <= c[1] && c2[0] <= c2[1]);
        assert!(c[2] < 1.0 && c[3] < 1.0);
        assert_eq!(wor_r(WReq::default()), [0.0, 0.0, 0.0, 0.0]); // n.x == 0
    }

    #[test]
    fn wor_r_is_periodic() {
        // uv + 1 shifts the integer cell index by exactly n; the wrapped
        // cells and ids are bit-identical, only the in-cell fraction can
        // differ by an ulp when `uv * n` is not exactly representable
        // (1.3f32 * 64 needs 25 significand bits).
        let a = wor_r(wo([1.3, 0.7], [64, 64], 0.8, 37));
        let b = wor_r(wo([0.3, 0.7], [64, 64], 0.8, 37));
        assert_eq!(&a[2..4], &b[2..4]); // ids exact
        approx(a[0], b[0]);
        approx(a[1], b[1]);
    }

    #[test]
    fn lin_decodes_srgb() {
        // L122; node: lin(0.02)=0.0015479876 (linear branch),
        // lin(0.5)=0.2140411405 (pow branch), lin(1)=1.
        let l = lin([0.02, 0.5, 1.0]);
        approx(l[0], 0.0015479876);
        approx(l[1], 0.2140411405);
        approx(l[2], 1.0);
        // step() threshold: 0.04045 itself takes the pow branch.
        approx(lin([0.04045, 0.0, 0.0])[0], 0.0031308073);
        approx(lin([0.04044, 0.0, 0.0])[0], 0.003130031);
    }

    #[test]
    fn jd_sd_rb_and_helpers() {
        // L159: node jd(3.7,2)=0.3, jd(-0.2,2)=0.2, jd(0.5,1)=0.5, jd(1,2)=1.
        approx(jd(3.7, 2.0), 0.3);
        approx(jd(-0.2, 2.0), 0.2);
        approx(jd(0.5, 1.0), 0.5);
        approx(jd(1.0, 2.0), 1.0);
        // L160: node sdRB values.
        approx(sd_rb([0.0, 0.0], [1.0, 2.0], 0.5), -1.0);
        approx(sd_rb([3.0, 0.0], [1.0, 2.0], 0.5), 2.0);
        approx(sd_rb([0.5, 0.5], [1.0, 2.0], 0.5), -0.5);
        // GLSL mod/fract sign semantics (concrete surf L230 uses mod()).
        approx(glsl_mod(-1.5, 2.0), 0.5);
        approx(fract(-0.25), 0.75);
        approx(mix(2.0, 4.0, 0.5), 3.0);
        approx(smoothstep(0.0, 1.0, 0.5), 0.5);
        assert_eq!(step(0.5, 0.4), 0.0);
        assert_eq!(step(0.5, 0.5), 1.0);
    }

    #[test]
    fn aa_endpoints_are_exact() {
        // L158: smoothstep over ±0.75*PX ⇒ exactly 0/1 at the endpoints.
        let px = 0.001;
        assert_eq!(aa(0.5, 0.5 - 0.75 * px, px), 0.0);
        assert_eq!(aa(0.5, 0.5 + 0.75 * px, px), 1.0);
        approx(aa(0.5, 0.5, px), 0.5);
    }

    #[test]
    fn edge_prof_pins_node_reference() {
        // L163-168; node with jw=0.003, r=0.01, de=0.001, dj=0.002, px=0.001.
        let p = |e: f32| edge_prof(e, 0.003, 0.01, 0.001, 0.002, 0.001);
        let r0 = p(0.0); // inside the joint: full depth
        approx(r0[0], -0.002);
        approx(r0[1], 1.0);
        let r1 = p(0.003); // on the AA centre: half in
        approx(r1[0], -0.0015);
        approx(r1[1], 0.5);
        let r2 = p(0.005); // arris quarter-circle
        approx(r2[0], -0.0004);
        approx(r2[1], 0.0);
        let r3 = p(0.02); // past the rounding: flat
        approx(r3[0], 0.0);
        approx(r3[1], 0.0);
    }

    #[test]
    fn request_builders_mirror_glsl() {
        // L154-156.
        let f = fb([0.25, 0.5], [3, 3], 5, 0.55, 11);
        assert_eq!(f.x, [0.75, 1.5]);
        assert_eq!(f.per, [3, 3]);
        assert_eq!(f.gain, 0.55);
        assert_eq!(f.oct, 5);
        assert_eq!(f.seed, 11);
        let a = fnp([7.0, 9.0], 4, 2);
        assert_eq!(a.x, [7.0, 9.0]);
        assert_eq!(a.per, [4096, 4096]);
        assert_eq!(a.gain, 0.5);
        let w = wo([0.3, 0.7], [64, 64], 0.8, 37);
        assert_eq!(w.uv, [0.3, 0.7]);
        assert_eq!(w.n, [64, 64]);
        assert_eq!(w.jit, 0.8);
        assert!((PI - std::f32::consts::PI).abs() < 1e-7);
        assert!((TAU - 2.0 * std::f32::consts::PI).abs() < 5e-7);
    }
}
