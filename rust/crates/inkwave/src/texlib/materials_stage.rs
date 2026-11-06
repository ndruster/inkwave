//! Batch C2: the 21 stage-owned surf programs (≡ `src/world/stages/*/surfaces.js`
//! SURF bodies, appended as layers `'<stage>:<name>'` at texlib.js L1167).
//!
//! Translation rules (same discipline as batches B/C1, spec D-T52-2/7):
//! - `fract`/`mod`/`mix`/`step`/`smoothstep`/`clamp`/`dot`/`length` and the
//!   noise helpers (`FB`/`WO`/`aa`/`jd`/`sdRB`/`edgeProf`/`lin`/`hf`/`hf2`/
//!   `wrp`) all resolve to the batch-B ports in [`super::r#gen`]; the GLSL
//!   global `PX` arrives as `ctx.px`.
//! - Layer meta uniforms (`uHRange`/`uAO`/`uDetail`) are not referenced by
//!   any stage `surf` body (only `GEN_MAIN`, already ported in
//!   [`super::bake`]), so `ctx.layer` stays unused here.
//! - `vec3`/`vec2` are `[f32; 3]`/`[f32; 2]` with the local `v3`/`mix3`/
//!   `scale3`/`mul3`/`add3`/`dot3` helpers mirroring GLSL component-wise ops;
//!   `ivec2` is `[i32; 2]`.
//! - `pow(x, 2.0)`/`pow(x, 6.0)` on a possibly-negative `x` is GLSL UB; both
//!   libm and desktop drivers return the exact integer power there, so
//!   `f32::powf` is the faithful port (declared deviation, same class as the
//!   C1 note and the bake.rs store/knee notes).
//! - `uint(f)`/`ivec2(f)`/`int(f)` truncation ≡ Rust `as i32`/`as u32` (all
//!   call sites are non-negative or integer-valued expressions).
//! - The JS bodies are ported verbatim, including redundant terms (e.g. the
//!   `* 0.0` factors in `chequer`); no algebraic "simplification".
//! - `saltboard`'s `for (int k = 0; k < 2 * uOne; k++)` unrolls to `0..2`
//!   (`uOne = 1`, ≡ the bake.rs rolled-loop note).
//!
//! The `crossmarket` JS shares two GLSL template strings — `COURSE` (L23-38,
//! parameterised) and `SETT_LOOK` (L41-55, fixed). They are expanded by the
//! `course` / `sett_look` helpers below, once per call site with the same
//! arguments the JS template literals pass.
//!
//! `cargo`'s `tarmac` / `quay` / `chequer` GLSL templates are byte-identical
//! to `kelpline`'s (verified in the port session), so the three cargo layers
//! resolve to the same program instances in [`lookup`].

use super::bake::{PrepCtx, Surf, SurfCtx, SurfProgram};
use super::r#gen::{
    Req, TAU, WReq, aa, clampf, dot2, edge_prof, fb, fract, glsl_mod, hf, hf2, jd, length2, lin,
    mix, sd_rb, smoothstep, step, wo, wrp,
};

type V3 = [f32; 3];

/// `vec3(float)` broadcast.
#[inline]
fn v3(x: f32) -> V3 {
    [x, x, x]
}

/// `mix(vec3, vec3, float)`.
#[inline]
fn mix3(a: V3, b: V3, t: f32) -> V3 {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

/// `vec3 * float` (and `float * vec3`).
#[inline]
fn scale3(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// `vec3 * vec3` component-wise.
#[inline]
fn mul3(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

/// `vec3 + vec3`.
#[inline]
fn add3(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `dot(vec3, vec3)`.
#[inline]
fn dot3(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// ---------------------------------------------------------------------------
// tidewater (stages/tidewater/surfaces.js)
// ---------------------------------------------------------------------------

/// ≡ stages/tidewater/surfaces.js L21-59 (`herringbone`).
#[derive(Debug, Clone, Copy)]
pub struct Herringbone;

impl SurfProgram for Herringbone {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L23-24
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 2801);
        f[1] = fb(ctx.uv, [40, 40], 3, 0.5, 2803);
        f[2] = fb(ctx.uv, [160, 160], 2, 0.5, 2807);
        w[0] = wo(ctx.uv, [120, 120], 1.0, 2809);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 34-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L25-58
        let p = ctx.p;
        let px = ctx.px;
        const W: f32 = 0.12;
        let pp = [p[0] / W, p[1] / W];
        let cc = [pp[0].floor() as i32, pp[1].floor() as i32];
        let mut dd = (cc[0] - cc[1]) as f32;
        dd -= 4.0 * (dd / 4.0).floor();
        let mut org = [cc[0] as f32, cc[1] as f32];
        let mut ext = [1.0f32, 2.0];
        if dd < 1.5 {
            org[0] -= dd;
            ext = [2.0, 1.0];
        } else if dd < 2.5 {
            org[1] -= 1.0;
        }
        let hb = [ext[0] * (0.5 * W), ext[1] * (0.5 * W)];
        let q = [(pp[0] - org[0]) * W - hb[0], (pp[1] - org[1]) * W - hb[1]];
        let e = -sd_rb(q, hb, 0.008);
        let pr = edge_prof(e, 0.0028, 0.01, 0.0026, 0.009, px);
        let in_j = pr[1];
        let bid = wrp([org[0] as i32, org[1] as i32], [20, 20]);
        let b1 = hf(bid, 3);
        let b2 = hf(bid, 5);
        let b3 = hf(bid, 7);
        let b4 = hf(bid, 11);
        let (mott, tex, sand) = (n[0], n[1], n[2]);
        let wc = c[0];
        let pit = step(0.9, wc[2])
            * (1.0
                - aa(
                    0.0025 + 0.002 * fract(wc[2] * 7.3),
                    wc[0] * (2.4 / 120.0),
                    px,
                ))
            * (1.0 - in_j);
        let qn = [q[0] / hb[0], q[1] / hb[1]];
        let crown = 1.0 - 0.6 * dot2([qn[0] * qn[0], qn[1] * qn[1]], [1.0, 1.0]);
        let mut tone = 0.8
            * (1.0 + 0.24 * (b1 - 0.5))
            * (1.0 + 0.06 * mott + 0.045 * tex + 0.03 * sand)
            * (1.0 - 0.14 * pit);
        tone *= if b2 > 0.91 {
            0.74
        } else if b2 < 0.06 {
            1.12
        } else {
            1.0
        };
        let arris = clampf(1.0 - (e - 0.0028) / 0.012, 0.0, 1.0) * (1.0 - in_j);
        let grime = (1.0 - smoothstep(0.0, 0.03, e - 0.0028))
            * (1.0 - in_j)
            * (0.6 + 0.4 * smoothstep(-0.3, 0.6, mott));
        tone *= (1.0 - 0.1 * arris) * (1.0 - 0.14 * grime);
        let worn = smoothstep(0.1, 0.8, mott * 0.5 + 0.5 + 0.3 * (b4 - 0.5)) * crown.max(0.0);
        let joint = scale3(lin([0.63, 0.59, 0.51]), 0.72 + 0.2 * sand + 0.16 * mott);
        s.alb = scale3(joint, in_j);
        s.a = (1.0 - in_j) * tone * (1.0 + 0.05 * worn);
        s.h = pr[0]
            + (0.0006 * crown.max(0.0) * (0.4 + 0.6 * b3) + 0.00015 * tex + 0.00008 * sand
                - 0.0012 * pit)
                * (1.0 - in_j);
        s.rough = mix(0.8 + 0.06 * tex - 0.14 * worn, 0.95, in_j);
        s.cav = mix(1.0, 0.55, in_j) * (1.0 - 0.3 * pit);
    }
}

/// ≡ stages/tidewater/surfaces.js L61-101 (`terrazzo`).
#[derive(Debug, Clone, Copy)]
pub struct Terrazzo;

impl SurfProgram for Terrazzo {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L63-64
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 2901);
        f[1] = fb(ctx.uv, [64, 64], 2, 0.5, 2903);
        f[2] = fb(ctx.uv, [12, 12], 3, 0.5, 2907);
        w[0] = wo(ctx.uv, [72, 72], 1.0, 2909);
        w[1] = wo(ctx.uv, [180, 180], 1.0, 2911);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 36-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L65-100
        let p = ctx.p;
        let px = ctx.px;
        let pc = wrp(
            [(p[0] / 1.2).floor() as i32, (p[1] / 1.2).floor() as i32],
            [2, 2],
        );
        let chk = ((pc[0] + pc[1]) & 1) as f32;
        let ds = jd(p[0], 1.2).min(jd(p[1], 1.2));
        let strip = 1.0 - aa(0.0024, ds, px);
        let line2 = 1.0 - aa(0.0016, (ds - 0.11).abs(), px);
        let band = 1.0 - aa(0.11, ds, px);
        let (a, bc) = (c[0], c[1]);
        let ra = 0.16 + 0.2 * fract(a[2] * 5.3);
        let rb = 0.2 + 0.18 * fract(bc[2] * 3.7);
        let chip_a = (1.0 - smoothstep(ra - 0.05, ra + 0.05, a[0])) * step(0.3, a[2]);
        let chip_b =
            (1.0 - smoothstep(rb - 0.08, rb + 0.08, bc[0])) * step(0.35, bc[2]) * (1.0 - chip_a);
        let cw = lin([0.95, 0.94, 0.91]);
        let cc2 = lin([0.86, 0.83, 0.77]);
        let cg = lin([0.6, 0.6, 0.59]);
        let ck = lin([0.2, 0.2, 0.21]);
        let cp = lin([0.86, 0.62, 0.54]);
        let cs = lin([0.55, 0.72, 0.66]);
        let ia = fract(a[2] * 13.37);
        let ib = fract(bc[2] * 7.77);
        let col_a = if ia < 0.4 {
            cw
        } else if ia < 0.66 {
            cc2
        } else if ia < 0.82 {
            cg
        } else if ia < 0.89 {
            ck
        } else if ia < 0.95 {
            cp
        } else {
            cs
        };
        let col_b = if ib < 0.55 {
            cw
        } else if ib < 0.85 {
            cc2
        } else {
            cg
        };
        let (mott, fine, cloud) = (n[0], n[1], n[2]);
        let mut tone = 0.8
            * mix(1.0, 0.86, chk)
            * mix(1.0, 0.7, band)
            * (1.0 + 0.05 * mott + 0.035 * cloud + 0.02 * fine);
        let dirt = (1.0 - smoothstep(0.0, 0.05, ds)) * (1.0 - strip) * smoothstep(-0.2, 0.6, mott);
        tone *= 1.0 - 0.1 * dirt;
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(own, scale3(col_a, 0.9 + 0.18 * fract(a[2] * 3.1)), chip_a);
        cov *= 1.0 - chip_a;
        own = mix3(own, scale3(col_b, 0.92 + 0.12 * fine), chip_b);
        cov *= 1.0 - chip_b;
        let br = strip.max(line2 * 0.85);
        let brass = scale3(lin([0.74, 0.58, 0.32]), 0.8 + 0.25 * fine + 0.1 * mott);
        own = mix3(own, brass, br);
        cov *= 1.0 - br;
        s.alb = own;
        s.a = cov * tone;
        let worn = smoothstep(-0.1, 0.7, cloud);
        s.h = 0.00018 * mott + 0.00004 * fine + 0.00005 * (chip_a + chip_b)
            - 0.0005 * dirt
            - 0.0002 * br;
        s.rough = mix(0.44 + 0.06 * fine + 0.12 * worn, 0.3, chip_a + chip_b);
        s.rough = mix(s.rough, 0.38, br);
        s.metal = br * 0.8;
        s.cav = 1.0 - 0.2 * dirt;
    }
}

/// ≡ stages/tidewater/surfaces.js L103-138 (`stucco`).
#[derive(Debug, Clone, Copy)]
pub struct Stucco;

impl SurfProgram for Stucco {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L105-106
        f[0] = fb(ctx.uv, [4, 4], 4, 0.55, 3001);
        f[1] = fb(ctx.uv, [24, 24], 3, 0.5, 3003);
        f[2] = fb(ctx.uv, [28, 3], 3, 0.5, 3007);
        f[3] = fb(ctx.uv, [96, 96], 2, 0.5, 3011);
        w[0] = wo(ctx.uv, [10, 10], 0.85, 3013);
        w[1] = wo(ctx.uv, [200, 200], 1.0, 3017);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 31-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L107-137
        let p = ctx.p;
        let px = ctx.px;
        const CH: f32 = 0.4;
        const BL: f32 = 1.2;
        let row = (p[1] / CH).floor();
        let ly = p[1] - row * CH;
        let bx = p[0] - glsl_mod(row, 2.0) * 0.6;
        let col = (bx / BL).floor();
        let lx = bx - col * BL;
        let bid = wrp([col as i32, row as i32], [2, 6]);
        let eh = ly.min(CH - ly);
        let ev = lx.min(BL - lx);
        let gh = (1.0 - eh / 0.007).max(0.0);
        let gv = (1.0 - ev / 0.0035).max(0.0);
        let groove = gh.max(gv * 0.8);
        let hj = -(0.0045 * gh).max(0.002 * gv);
        let b1 = hf(bid, 3);
        let b2 = hf(bid, 5);
        let _ = b2; // declared but unused in the JS body (ported verbatim)
        let (mott, und, streak, fine) = (n[0], n[1], n[2], n[3]);
        let k = c[0];
        let crack = (1.0 - aa(0.0014, (k[1] - k[0]) * 0.24, px))
            * step(0.7, fract(k[2] * 9.1 + k[3] * 3.3))
            * smoothstep(0.1, 0.5, mott);
        let g = c[1];
        let grain = smoothstep(0.03, 0.25, g[1] - g[0]) * (1.0 - g[0] * g[0]);
        let flake = step(0.9, fract(k[3] * 13.7 + k[2]))
            * (1.0 - aa(0.0, (k[0] - 0.1 - 0.06 * fine) * 0.24, px))
            * step(0.25, mott);
        let soot =
            (1.0 - smoothstep(0.0, 0.16, CH - ly)) * smoothstep(0.1, 0.8, streak * 0.5 + 0.5);
        let rain =
            smoothstep(0.35, 0.95, streak * 0.5 + 0.5) * (0.5 + 0.5 * smoothstep(-0.2, 0.6, mott));
        let mut paint = 0.8
            * (1.0 + 0.035 * (b1 - 0.5) + 0.03 * und + 0.025 * fine + 0.04 * mott)
            * (1.0 - 0.07 * soot - 0.06 * rain)
            * (1.0 - 0.12 * crack);
        paint *= 1.0 - 0.12 * groove;
        let bare = scale3(lin([0.56, 0.55, 0.52]), 1.0 + 0.08 * grain);
        s.alb = scale3(bare, flake);
        s.a = (1.0 - flake) * paint;
        s.h = hj + (0.0005 * und + 0.00025 * grain + 0.00008 * fine) * (1.0 - groove)
            - 0.0006 * crack
            - 0.0004 * flake;
        s.rough = 0.74 + 0.06 * mott - 0.04 * und + 0.06 * flake + 0.05 * rain;
        s.cav = (1.0 - 0.32 * groove) * (1.0 - 0.15 * crack);
    }
}

// ---------------------------------------------------------------------------
// kelpline (stages/kelpline/surfaces.js) — the three instances are shared
// verbatim with the byte-identical cargo/surfaces.js layers (see module doc).
// ---------------------------------------------------------------------------

/// ≡ stages/kelpline/surfaces.js L26-58 (`tarmac`) ≡
/// stages/cargo/surfaces.js L27-58 (prep L27-28, surf L29-58; templates
/// byte-identical, so one instance serves both layers).
#[derive(Debug, Clone, Copy)]
pub struct Tarmac;

impl SurfProgram for Tarmac {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // kelpline L26-27 ≡ cargo L27-28
        f[0] = fb(ctx.uv, [3, 3], 4, 0.5, 3101);
        f[1] = fb(ctx.uv, [8, 8], 3, 0.5, 3107);
        f[2] = fb(ctx.uv, [48, 48], 2, 0.5, 3109);
        f[3] = fb(ctx.uv, [4, 4], 3, 0.55, 3113);
        w[0] = wo(ctx.uv, [190, 190], 1.0, 3119);
        w[1] = wo(ctx.uv, [7, 7], 0.9, 3121);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 30-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // kelpline L28-57 ≡ cargo L29-58
        let px = ctx.px;
        let wc = c[0];
        let stone = smoothstep(0.08, 0.3, wc[1] - wc[0]);
        let light = step(0.9, wc[2]);
        let (big, mid, fine) = (n[0], n[1], n[2]);
        let mut tone = mix(0.6 + 0.05 * fine, 0.74 + 0.16 * fract(wc[2] * 7.13), stone);
        tone = mix(tone, 0.98, light * stone * 0.55);
        let worn = smoothstep(-0.05, 0.6, big);
        tone *= 1.0 + 0.07 * big + 0.04 * mid + 0.06 * worn;
        let cr = c[1];
        let edge_m = (cr[1] - cr[0]) * (4.0 / 7.0);
        let edge_sel = fract((cr[2] + cr[3]) * 13.7 + (cr[2] - cr[3]).abs() * 5.3);
        let crack_on = smoothstep(0.3, 0.62, n[3]) * step(0.55, edge_sel);
        let seal = (1.0 - aa(0.016 + 0.006 * mid, edge_m, px)) * crack_on;
        let crack =
            (1.0 - aa(0.0018, edge_m, px)) * step(edge_sel, 0.3) * smoothstep(0.1, 0.4, n[3]) * 0.7;
        let oil = smoothstep(0.66, 0.84, 0.5 + 0.5 * mid)
            * smoothstep(0.1, 0.5, 0.5 + 0.5 * fine)
            * (1.0 - seal);
        let scuff =
            smoothstep(0.7, 0.9, 0.5 + 0.5 * n[3]) * smoothstep(0.3, 0.7, 0.5 + 0.5 * big) * 0.5;
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(
            own,
            scale3([0.035, 0.035, 0.038], 1.0 + 0.3 * fine),
            seal * 0.85,
        );
        cov *= 1.0 - seal * 0.85;
        own = mix3(own, [0.03, 0.028, 0.026], oil * 0.55);
        cov *= 1.0 - oil * 0.55;
        own = mix3(own, v3(0.05), crack * 0.6);
        cov *= 1.0 - crack * 0.6;
        s.alb = own;
        s.a = cov * clampf(tone * 1.18 * (1.0 - 0.22 * scuff), 0.0, 1.0);
        s.h = (-0.0011 * (1.0 - stone) + 0.0005 * stone * (1.0 - wc[0] * wc[0]) + 0.0001 * big)
            * (1.0 - seal)
            + 0.0006 * seal
            - 0.002 * crack;
        s.rough = mix(
            mix(mix(0.93, 0.8, stone) - 0.1 * worn, 0.38, seal * 0.9),
            0.5,
            oil * 0.6,
        );
        s.cav = (1.0 - 0.25 * (1.0 - stone)) * (1.0 - 0.4 * crack);
    }
}

/// ≡ stages/kelpline/surfaces.js L64-103 (`quay`) ≡
/// stages/cargo/surfaces.js L65-103 (prep L65-67, surf L68-103; templates
/// byte-identical, so one instance serves both layers).
#[derive(Debug, Clone, Copy)]
pub struct Quay;

impl SurfProgram for Quay {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // kelpline L64-66 ≡ cargo L65-67
        f[0] = fb(ctx.uv, [4, 4], 5, 0.55, 3201);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 3203);
        f[2] = fb(ctx.uv, [4, 150], 1, 0.5, 3209);
        f[3] = fb(ctx.uv, [150, 4], 1, 0.5, 3211);
        w[0] = wo(ctx.uv, [160, 160], 1.0, 3217);
        w[1] = wo(ctx.uv, [6, 6], 0.85, 3221);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 36-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // kelpline L67-102 ≡ cargo L68-103
        let p = ctx.p;
        let px = ctx.px;
        let cell = [(p[0] / 2.4).floor() as i32, (p[1] / 2.4).floor() as i32];
        let cw = wrp(cell, [2, 2]);
        let lp = [
            p[0] - (cell[0] as f32 * 2.4 + 1.2),
            p[1] - (cell[1] as f32 * 2.4 + 1.2),
        ];
        let e = -sd_rb(lp, [1.2, 1.2], 0.004);
        let pr = edge_prof(e, 0.005, 0.007, 0.0025, 0.008, px);
        let in_j = pr[1];
        let hid = hf(cw, 3);
        let hid2 = hf(cw, 5);
        let du = ((cw[0] + cw[1]) & 1) == 0;
        let broom = if du { n[2] } else { n[3] };
        let (mott, cloud) = (n[0], n[1]);
        let ag = c[0];
        let agg = smoothstep(0.1, 0.3, ag[1] - ag[0]);
        let worn = smoothstep(0.15, 0.7, mott + 0.3 * cloud);
        let mut tone =
            0.8 * (1.0 + 0.09 * (hid - 0.5) + 0.07 * mott + 0.04 * cloud + 0.035 * broom);
        tone *= mix(1.0, 0.86 + 0.28 * fract(ag[2] * 9.1), agg * worn * 0.7);
        let grime = (1.0 - smoothstep(0.0, 0.07, e - 0.005)) * (1.0 - in_j);
        tone *= 1.0 - 0.1 * grime;
        let rs = c[1];
        let rsel = step(0.72, rs[2]) * step(0.03, e);
        let rust = rsel
            * (1.0 - smoothstep(0.05, 0.28 + 0.1 * cloud, rs[0] * 0.8))
            * smoothstep(-0.3, 0.4, cloud);
        let crack = (1.0 - aa(0.0022, (rs[1] - rs[0]) * 0.8, px))
            * step(0.5, fract(rs[2] * 5.7 + rs[3] * 3.1))
            * smoothstep(0.2, 0.5, mott)
            * step(0.05, e);
        let scuff = smoothstep(0.72, 0.92, 0.5 + 0.5 * cloud)
            * smoothstep(0.35, 0.8, 0.5 + 0.5 * mott)
            * 0.6;
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(own, [0.03, 0.03, 0.032], in_j);
        cov *= 1.0 - in_j;
        own = mix3(
            own,
            scale3([0.34, 0.16, 0.07], 0.85 + 0.3 * cloud),
            rust * 0.55,
        );
        cov *= 1.0 - rust * 0.55;
        own = mix3(own, v3(0.08), crack * 0.55);
        cov *= 1.0 - crack * 0.55;
        s.alb = own;
        s.a = cov * tone * (1.0 - 0.2 * scuff);
        s.h = pr[0] + (0.00012 * broom + 0.0002 * mott + 0.0003 * agg * worn) * (1.0 - in_j)
            - 0.0015 * crack;
        s.rough = mix(
            mix(0.86 + 0.05 * broom - 0.08 * worn + 0.04 * hid2, 0.45, in_j),
            0.8,
            rust * 0.5,
        );
        s.cav = mix(1.0, 0.5, in_j) * (1.0 - 0.35 * crack) * (1.0 - 0.1 * grime);
    }
}

/// ≡ stages/kelpline/surfaces.js L109-148 (`chequer`) ≡
/// stages/cargo/surfaces.js L110-148 (prep L110-111, surf L112-148; templates
/// byte-identical, so one instance serves both layers).
#[derive(Debug, Clone, Copy)]
pub struct Chequer;

impl SurfProgram for Chequer {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // kelpline L109-110 ≡ cargo L110-111
        f[0] = fb(ctx.uv, [3, 3], 4, 0.5, 3301);
        f[1] = fb(ctx.uv, [24, 24], 2, 0.5, 3303);
        f[2] = fb(ctx.uv, [6, 6], 3, 0.5, 3307);
        f[3] = fb(ctx.uv, [10, 10], 3, 0.5, 3309);
        w[0] = wo(ctx.uv, [8, 8], 0.9, 3311);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 37-line GLSL body
    #[allow(clippy::approx_constant)] // 0.70710678 is the JS literal; FRAC_1_SQRT_2 would re-bake
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // kelpline L111-147 ≡ cargo L112-148
        let p = ctx.p;
        let px = ctx.px;
        const CS: f32 = 0.04;
        let cp = [p[0] / CS, p[1] / CS];
        let ci = [cp[0].floor() as i32, cp[1].floor() as i32];
        let cf = [cp[0] - ci[0] as f32 - 0.5, cp[1] - ci[1] as f32 - 0.5];
        let odd = ((ci[0] + ci[1]) & 1) == 1;
        let rot = if odd {
            [cf[0] + cf[1], cf[1] - cf[0]]
        } else {
            [cf[0] - cf[1], cf[0] + cf[1]]
        };
        let q = [rot[0] * 0.70710678, rot[1] * 0.70710678];
        let ld = length2([((q[0].abs() - 0.26).max(0.0)), q[1]]) - 0.1;
        let lw = px / CS;
        let mut lug = 1.0 - smoothstep(-lw, lw, ld);
        let dome = clampf(-ld / 0.1, 0.0, 1.0).sqrt();
        let ds = jd(p[0], 1.2).min(jd(p[1], 1.2));
        let bead = (-ds * ds / 0.00004).exp();
        let near_s = 1.0 - smoothstep(0.004, 0.012, ds);
        lug *= 1.0 - near_s;
        let bu = (fract(p[0] / 0.15) - 0.5).abs() * 0.15;
        let bv = (fract(p[1] / 0.15) - 0.5).abs() * 0.15;
        let bd = length2([bu, jd(p[1], 1.2) - 0.03]).min(length2([bv, jd(p[0], 1.2) - 0.03]));
        let bolt = 1.0 - aa(0.009, bd, px);
        let (mott, fine_n, wear_n, rust_n) = (n[0], n[1], n[2], n[3]);
        let wear = lug * dome * smoothstep(0.45, 0.8, 0.5 + 0.5 * wear_n);
        let edge_wear = bead * smoothstep(0.5, 0.8, 0.5 + 0.5 * wear_n) * 0.7;
        let rc = c[0];
        let rust = step(0.86, rc[2])
            * (1.0 - smoothstep(0.04, 0.2 + 0.08 * rust_n, rc[0] * 0.15))
            * smoothstep(-0.2, 0.4, rust_n);
        let steel = scale3([0.42, 0.43, 0.44], 1.0 + 0.08 * fine_n);
        let rust_c = scale3([0.3, 0.13, 0.05], 0.8 + 0.4 * fine_n);
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(own, steel, wear * 0.85);
        cov *= 1.0 - wear * 0.85;
        own = mix3(own, scale3(steel, 0.9), edge_wear);
        cov *= 1.0 - edge_wear;
        own = mix3(own, rust_c, rust * 0.7);
        cov *= 1.0 - rust * 0.7;
        s.alb = own;
        // the `(1.0 - 0.12 * ... * 0.0)` factor is a verbatim no-op from the JS.
        s.a = cov
            * 0.8
            * (1.0 + 0.05 * mott + 0.03 * fine_n)
            * (1.0 - 0.12 * (1.0 - lug) * (1.0 - bolt) * 0.0)
            * (1.0 + 0.06 * lug * dome);
        s.h = 0.0018 * lug * dome + 0.0008 * bead - 0.0012 * near_s * (1.0 - bead) + 0.0014 * bolt;
        s.rough = mix(mix(0.5 + 0.06 * mott, 0.32, wear), 0.82, rust * 0.7);
        s.metal = wear * 0.8 + edge_wear * 0.6;
        s.cav = (1.0 - 0.2 * (1.0 - lug) * smoothstep(-0.05, 0.02, ld) * 0.0)
            * (1.0 - 0.3 * near_s * (1.0 - bead));
    }
}

// ---------------------------------------------------------------------------
// saltpan (stages/saltpan/surfaces.js)
// ---------------------------------------------------------------------------

/// ≡ stages/saltpan/surfaces.js L24-66 (`saltcrust`).
#[derive(Debug, Clone, Copy)]
pub struct SaltCrust;

impl SurfProgram for SaltCrust {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L26-28
        let uv = ctx.uv;
        f[0] = fb(uv, [3, 3], 4, 0.55, 811);
        f[1] = fb(uv, [12, 12], 3, 0.5, 821);
        f[2] = fb(uv, [48, 48], 3, 0.5, 827);
        let wq = [
            uv[0] + 0.012 * (TAU * (3.0 * uv[1] + 2.0 * uv[0])).sin(),
            uv[1] + 0.012 * (TAU * (3.0 * uv[0] - 2.0 * uv[1]) + 1.1).sin(),
        ];
        w[0] = wo(wq, [7, 7], 0.92, 831);
        w[1] = wo(uv, [90, 90], 1.0, 839);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 37-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L29-65
        let uv = ctx.uv;
        let px = ctx.px;
        let (wc, xc) = (c[0], c[1]);
        let (big, mott, fine) = (n[0], n[1], n[2]);
        let ep = (wc[1] - wc[0]) * (3.2 / 7.0);
        let ridge = (1.0 - smoothstep(0.0, 0.04, ep))
            * (0.5 + 0.5 * step(0.35, fract((wc[2] + wc[3]) * 7.1)));
        let seam = 1.0 - aa(0.0025, ep, px);
        let dome = smoothstep(0.0, 0.18, ep);
        let facet = smoothstep(0.02, 0.35, xc[1] - xc[0]);
        let ftone = fract(xc[2] * 7.31);
        let wf = 0.92 * big + 0.1 * mott + 0.14 * (wc[2] - 0.5);
        let pool = smoothstep(0.02, -0.12, wf) * (1.0 - 0.75 * ridge * smoothstep(-0.3, -0.1, wf));
        let slush = smoothstep(0.16, 0.0, wf) * (1.0 - pool);
        let hi = [(uv[0] * 29.0).floor() as i32, (uv[1] * 29.0).floor() as i32];
        let hw = wrp(hi, [29, 29]);
        let j2 = hf2(hw, 871);
        let mut hq = [
            fract(uv[0] * 29.0) - 0.5 - (j2[0] - 0.5) * 0.5,
            fract(uv[1] * 29.0) - 0.5 - (j2[1] - 0.5) * 0.5,
        ];
        let ha = hf(hw, 873) * 1.57;
        // mat2(cos, sin, -sin, cos) * hq, written out (column-major rotation).
        let (hc, hs) = (ha.cos(), ha.sin());
        hq = [hc * hq[0] - hs * hq[1], hs * hq[0] + hc * hq[1]];
        let hsz = 0.07 + 0.08 * hf(hw, 877);
        let hop = step(0.55, hf(hw, 879))
            * (1.0 - smoothstep(hsz - 0.03, hsz, hq[0].abs().max(hq[1].abs())))
            * smoothstep(0.1, 0.6, slush + pool * (1.0 - pool) * 2.0);
        let tone = 0.95 + 0.08 * (fract(wc[2] * 13.7) - 0.5);
        let mut crust_l = 0.8
            * tone
            * (1.0 + 0.035 * mott + 0.03 * fine)
            * (0.97 + 0.05 * ftone * facet)
            * (1.0 + 0.05 * ridge)
            * (1.0 - 0.05 * (1.0 - dome));
        crust_l *= 1.0 - 0.12 * seam;
        let brine = scale3(
            mix3(
                lin([0.94, 0.83, 0.81]),
                lin([0.9, 0.73, 0.71]),
                smoothstep(-0.1, -0.5, wf),
            ),
            0.95 + 0.05 * mott,
        );
        let wet_c = scale3(lin([0.94, 0.87, 0.85]), 0.96 + 0.05 * fine);
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(own, wet_c, slush * 0.55);
        cov *= 1.0 - slush * 0.55;
        own = mix3(own, brine, pool);
        cov *= 1.0 - pool;
        own = mix3(own, lin([0.97, 0.95, 0.94]), hop);
        cov *= 1.0 - hop;
        s.alb = own;
        s.a = cov * crust_l;
        let hc = 0.0012 * dome + 0.0013 * ridge + 0.00035 * facet + 0.0002 * fine - 0.0009 * seam;
        s.h = mix(mix(hc, -0.0012 + 0.0003 * fine, slush * 0.7), -0.0028, pool) + 0.0006 * hop;
        s.rough = mix(
            mix(
                mix(0.74 - 0.12 * facet + 0.04 * mott, 0.34, slush),
                0.05,
                pool,
            ),
            0.4,
            hop,
        );
        s.cav = 1.0 - 0.35 * seam;
    }
}

/// ≡ stages/saltpan/surfaces.js L70-104 (`drymud`).
#[derive(Debug, Clone, Copy)]
pub struct DryMud;

impl SurfProgram for DryMud {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L72-75
        let uv = ctx.uv;
        f[0] = fb(uv, [4, 4], 4, 0.55, 851);
        f[1] = fb(uv, [24, 24], 3, 0.5, 853);
        f[2] = fb(uv, [96, 96], 2, 0.5, 857);
        let wq = [
            uv[0]
                + 0.025
                    * ((TAU * (2.0 * uv[1] + uv[0])).sin()
                        + 0.5 * (TAU * (5.0 * uv[1] - 3.0 * uv[0]) + 0.7).sin()),
            uv[1]
                + 0.025
                    * ((TAU * (2.0 * uv[0] - uv[1]) + 1.3).sin()
                        + 0.5 * (TAU * (4.0 * uv[0] + 5.0 * uv[1]) + 2.1).sin()),
        ];
        w[0] = wo(wq, [6, 6], 0.95, 859);
        w[1] = wo(uv, [18, 18], 1.0, 861);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 28-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L76-103
        let px = ctx.px;
        let (wc, sc) = (c[0], c[1]);
        let (big, silt, grit) = (n[0], n[1], n[2]);
        let e1 = (wc[1] - wc[0]) * 0.4;
        let cw = 0.0025 + 0.0045 * (0.5 + 0.5 * big) + 0.003 * fract(wc[2] * 5.1);
        let healed = step(0.72, fract((wc[2] + wc[3]) * 17.3)) * smoothstep(-0.2, 0.3, silt);
        let crack = (1.0 - aa(cw, e1, px)) * (1.0 - 0.85 * healed);
        let rim = (1.0 - smoothstep(cw, cw + 0.045, e1)) * (1.0 - crack);
        let cup = smoothstep(0.0, 0.16, e1);
        let e2 = (sc[1] - sc[0]) * (2.4 / 18.0);
        let sec = (1.0 - aa(0.0016, e2, px))
            * step(0.42, fract(sc[2] * 3.7 + sc[3] * 1.3))
            * smoothstep(0.02, 0.05, e1);
        let damp = smoothstep(0.2, 0.6, -big);
        let ptone = 0.93 + 0.12 * (fract(wc[2] * 11.3) - 0.5);
        let mud_l = 0.8
            * ptone
            * (1.0 + 0.06 * silt + 0.05 * grit)
            * (1.0 - 0.12 * damp)
            * (1.0 + 0.08 * rim)
            * (1.0 - 0.28 * sec);
        let bloom_f = smoothstep(0.3, 0.75, 0.55 * big + 0.45 * silt);
        let bloom = ((1.0 - smoothstep(0.0, 0.035, e1 - cw)) * bloom_f)
            .max(0.55 * damp * smoothstep(0.1, 0.5, grit + 0.3))
            * (1.0 - crack * 0.4);
        let shell =
            step(0.985, fract(sc[2] * 13.3)) * (1.0 - aa(0.012, sc[0] * 0.133, px)) * (1.0 - crack);
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(
            own,
            scale3(lin([0.93, 0.92, 0.9]), 0.95 + 0.06 * grit),
            bloom * 0.8,
        );
        cov *= 1.0 - bloom * 0.8;
        own = mix3(own, lin([0.86, 0.81, 0.74]), shell);
        cov *= 1.0 - shell;
        own = mix3(own, scale3(lin([0.24, 0.2, 0.17]), 0.8 + 0.3 * silt), crack);
        cov *= 1.0 - crack;
        s.alb = own;
        s.a = cov * mud_l;
        s.h = -0.014 * crack + 0.0024 * rim + 0.0008 * cup - 0.0015 * sec
            + 0.0004 * silt
            + 0.00015 * grit
            + 0.0003 * bloom
            + 0.0005 * shell;
        s.rough = mix(mix(0.93 - 0.05 * rim, 0.72, bloom), 1.0, crack);
        s.cav = (1.0 - 0.6 * crack) * (1.0 - 0.25 * sec);
    }
}

/// ≡ stages/saltpan/surfaces.js L108-172 (`saltboard`).
#[derive(Debug, Clone, Copy)]
pub struct SaltBoard;

impl SurfProgram for SaltBoard {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L110-115
        let uv = ctx.uv;
        let row = (ctx.p[1] / 0.2).floor() as i32;
        let ly = ctx.p[1] - (row as f32) * 0.2;
        f[0] = Req {
            x: [uv[0] * 6.0, ly * 30.0 + (row as f32) * 5.37],
            per: [6, 4096],
            oct: 3,
            gain: 0.55,
            seed: 881,
        };
        f[1] = fb(uv, [5, 5], 4, 0.5, 883);
        f[2] = Req {
            x: [uv[0] * 3.0, (row as f32) * 2.31],
            per: [3, 4096],
            oct: 3,
            gain: 0.5,
            seed: 887,
        };
        f[3] = fb(uv, [48, 48], 2, 0.5, 889);
        w[0] = wo(uv, [4, 12], 1.0, 891);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 56-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L116-171
        let p = ctx.p;
        let px = ctx.px;
        const PW: f32 = 0.2;
        let row = (p[1] / PW).floor() as i32;
        let ly = p[1] - (row as f32) * PW;
        let js = [0i32, 0, 2, 2, 0, 0, 2, 2, 0, 0, 2, 2];
        // P.y in (0, scale) => row in 0..=11 (JS relies on the same range).
        let jx = 0.3 + 0.6 * (js[row as usize] as f32);
        let ee = jd(p[0] - jx, 2.4);
        let es = ly.min(PW - ly);
        let ps = edge_prof(es, 0.0035, 0.006, 0.002, 0.02, px);
        let pe = edge_prof(ee, 0.0015, 0.004, 0.0016, 0.016, px);
        let gap = ps[1].max(pe[1]);
        let b1 = hf([row, 0], 5);
        let b2 = hf([row, 1], 5);
        let b3 = hf([row, 2], 5);
        let q = clampf((ly - PW * 0.5) / (PW * 0.5 - 0.0035), -1.0, 1.0);
        let cup_h = mix(-0.0006, 0.0008, b2) * (1.0 - q * q);
        let (streak, patch_n, warp, fib) = (n[0], n[1], n[2], n[3]);
        let late = smoothstep(
            0.72,
            0.96,
            0.5 + 0.5 * (TAU * (q * (1.4 + 1.2 * b1) + 1.1 * warp + 3.0 * b3)).cos(),
        );
        let mut chk: f32 = 0.0;
        // `for (int k = 0; k < 2 * uOne; k++)` with uOne = 1 => 0..2.
        for k in 0..2 {
            let h1 = hf([row, k + 3], 41);
            let h2 = hf([row, k + 3], 43);
            if h1 > 0.6 {
                continue;
            }
            let l = 0.05 + 0.25 * h2;
            let xc = jx + (if k == 0 { 1.0 } else { -1.0 }) * (0.01 + l);
            let yc = 0.03 + h1 * (PW - 0.06) / 0.6;
            let taper = clampf(1.0 - jd(p[0] - xc, 2.4) / l, 0.0, 1.0);
            let d = (ly - yc - 0.002 * (p[0] * 29.0 + h1 * 40.0).sin()).abs();
            chk = chk.max(
                (1.0 - smoothstep(0.0003 + 0.0012 * taper, 0.0003 + 0.0012 * taper + px, d))
                    * step(0.001, taper),
            );
        }
        chk *= 1.0 - gap;
        let nx = jd(p[0] - 0.3, 0.6);
        let ny = (ly - 0.035).abs().min((PW - 0.035 - ly).abs());
        let nd = length2([nx, ny]);
        let nail = 1.0 - aa(0.0045, nd, px);
        let halo = (1.0 - smoothstep(0.0045, 0.03, nd))
            * (1.0 - nail)
            * step(
                0.4,
                hf(wrp([(p[0] / 0.6 + 0.5).floor() as i32, row], [4, 12]), 51),
            );
        let mut board = 0.8
            * (0.86 + 0.26 * b1)
            * (1.0 + 0.1 * streak)
            * (1.0 - 0.1 * late)
            * (1.0 + 0.03 * fib)
            * (1.0 + 0.05 * patch_n);
        board *= (1.0 - 0.35 * chk) * (1.0 - 0.12 * (1.0 - smoothstep(0.0, 0.03, ee)));
        let s_edge = (1.0
            - smoothstep(0.0, 0.012 + 0.012 * patch_n, (es - 0.0035).min(ee - 0.0015)))
            * smoothstep(-0.2, 0.5, patch_n + 0.3 * fib);
        let w0 = c[0];
        let s_patch = smoothstep(0.55, 0.85, w0[2] * 0.5 + 0.5 * (0.5 + 0.5 * patch_n))
            * (1.0 - smoothstep(0.1, 0.35, w0[0]))
            * 0.5;
        let salt = clampf((s_edge * 0.75).max(s_patch), 0.0, 0.8) * (1.0 - gap);
        let mut own = v3(0.0);
        let mut cov = 1.0;
        own = mix3(
            own,
            scale3(lin([0.94, 0.93, 0.91]), 0.95 + 0.05 * fib),
            salt,
        );
        cov *= 1.0 - salt;
        own = mix3(
            own,
            scale3(lin([0.45, 0.24, 0.12]), 0.8 + 0.4 * fib),
            halo * 0.45,
        );
        cov *= 1.0 - halo * 0.45;
        own = mix3(
            own,
            scale3(
                lin([0.3, 0.17, 0.1]),
                0.7 + 0.5 * smoothstep(0.0045, 0.0, nd),
            ),
            nail,
        );
        cov *= 1.0 - nail;
        own = mix3(
            own,
            mix3(
                lin([0.05, 0.045, 0.04]),
                lin([0.6, 0.58, 0.55]),
                0.2 * smoothstep(-0.2, 0.6, patch_n),
            ),
            gap,
        );
        cov *= 1.0 - gap;
        s.alb = own;
        s.a = cov * board;
        let mut h = ps[0].min(pe[0]);
        h += (cup_h + 0.00012 * streak + 0.00018 * late + 0.00006 * fib - 0.0025 * chk
            + 0.0002 * salt)
            * (1.0 - gap);
        s.h = mix(h, -0.0006 + 0.0003 * smoothstep(0.0045, 0.0, nd), nail);
        s.rough = mix(
            mix(0.78 + 0.08 * (fib * 0.5 + 0.5) + 0.04 * late, 0.7, salt),
            0.95,
            gap,
        );
        s.metal = nail * 0.3;
        s.cav = mix(1.0, 0.3, gap) * (1.0 - 0.35 * chk);
    }
}

// ---------------------------------------------------------------------------
// crossmarket (stages/crossmarket/surfaces.js) — COURSE/SETT_LOOK templates
// ---------------------------------------------------------------------------

/// ≡ stages/crossmarket/surfaces.js L23-38 `COURSE(...)` template expansion;
/// returns the `vec4 q = vec4(e, lp/hb, hf(sid, 11u))` the macro declares.
#[allow(clippy::too_many_arguments)] // mirrors the COURSE macro's 8 parameters
fn course(a: f32, b: f32, ch: f32, sl: f32, ns: i32, nr: i32, jit: f32, r: f32) -> [f32; 4] {
    let rowf = (b / ch).floor();
    let rid = wrp([rowf as i32, 0], [nr, 1])[0];
    let ly = b - rowf * ch;
    let xs = a - hf([rid, 3], 5) * sl;
    let mut kf = (xs / sl).floor();
    let mut b0 = kf * sl + (hf(wrp([kf as i32, rid], [ns, nr]), 7) - 0.5) * jit;
    let mut b1 = (kf + 1.0) * sl + (hf(wrp([kf as i32 + 1, rid], [ns, nr]), 7) - 0.5) * jit;
    if xs < b0 {
        b1 = b0;
        kf -= 1.0;
        b0 = kf * sl + (hf(wrp([kf as i32, rid], [ns, nr]), 7) - 0.5) * jit;
    } else if xs > b1 {
        b0 = b1;
        kf += 1.0;
        b1 = (kf + 1.0) * sl + (hf(wrp([kf as i32 + 1, rid], [ns, nr]), 7) - 0.5) * jit;
    }
    let sid = wrp([kf as i32, rid], [ns, nr]);
    let hb = [(b1 - b0) * 0.5, ch * 0.5];
    let lp = [xs - (b0 + b1) * 0.5, ly - ch * 0.5];
    let e = -sd_rb(lp, hb, r);
    [e, lp[0] / hb[0], lp[1] / hb[1], hf(sid, 11)]
}

/// Outputs of the shared `SETT_LOOK` template (stages/crossmarket/surfaces.js
/// L41-55); `tone` is internal to the template and not consumed by callers.
struct SettLook {
    stone: V3,
    joint: V3,
    dome: f32,
    wear: f32,
    fleck: f32,
}

/// ≡ stages/crossmarket/surfaces.js L41-55 `SETT_LOOK` expansion.
fn sett_look(q: [f32; 4], n: &[f32; 4], c: &[[f32; 4]; 2], gs: f32, px: f32) -> SettLook {
    let t1 = q[3];
    let t2 = fract(q[3] * 7.31);
    let t3 = fract(q[3] * 13.7);
    let hue = if t2 < 0.34 {
        [1.05, 0.985, 0.94]
    } else if t2 < 0.67 {
        [0.955, 0.99, 1.04]
    } else {
        [1.0, 1.0, 1.0]
    };
    let dome = clampf((1.0 - q[1] * q[1]) * (1.0 - q[2] * q[2]), 0.0, 1.0);
    let wear = smoothstep(0.15, 0.75, n[0] * 0.5 + 0.5 + 0.35 * (t3 - 0.5)) * dome;
    let g = c[0];
    let cell = g[0] * gs;
    let fleck = step(0.86, g[2]) * (1.0 - aa(0.0035 + 0.002 * fract(g[2] * 5.3), cell, px));
    let felds =
        step(0.9, fract(g[2] * 7.1)) * (1.0 - aa(0.003, cell, px)) * (1.0 - step(0.86, g[2]));
    let tone = 0.8 * (1.0 + 0.26 * (t1 - 0.5)) * (1.0 + 0.05 * n[1] + 0.03 * n[2]);
    let mut stone = mul3(
        mul3(
            mul3(mul3(v3(tone), hue), v3(1.0 - 0.3 * fleck)),
            v3(1.0 + 0.18 * felds),
        ),
        v3(1.0 + 0.07 * wear),
    );
    stone = scale3(
        stone,
        1.0 - 0.1 * clampf(1.0 - (q[0] - 0.006) / 0.02, 0.0, 1.0),
    );
    let moss = smoothstep(0.25, 0.8, n[1] * 0.5 + 0.5 + 0.25 * n[3]);
    let joint = scale3(
        mix3([0.33, 0.315, 0.29], [0.25, 0.3, 0.2], moss * 0.7),
        1.0 + 0.18 * n[2],
    );
    SettLook {
        stone,
        joint,
        dome,
        wear,
        fleck,
    }
}

/// ≡ stages/crossmarket/surfaces.js L60-77 (`setts`).
#[derive(Debug, Clone, Copy)]
pub struct MarketSetts;

impl SurfProgram for MarketSetts {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L62-63
        f[0] = fb(ctx.uv, [3, 3], 4, 0.5, 3701);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 3703);
        f[2] = fb(ctx.uv, [96, 96], 2, 0.5, 3709);
        f[3] = fb(ctx.uv, [40, 40], 2, 0.5, 3713);
        w[0] = wo(ctx.uv, [150, 150], 1.0, 3711);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L64-76
        let p = ctx.p;
        let px = ctx.px;
        // L65-67: `COURSE(q, CH, SL, NS, NR, JIT, R)` with the L60-61 defaults
        // (0.16 / 0.24 / 8 / 12 / 0.08 / 0.024), GS = 1.92/150.
        let q = course(p[0], p[1], 0.16, 0.24, 8, 12, 0.08, 0.024);
        let look = sett_look(q, n, c, 1.92 / 150.0, px);
        let pr = edge_prof(q[0], 0.0055, 0.03, 0.009, 0.016, px);
        let in_j = pr[1];
        s.alb = mix3(look.stone, look.joint, in_j);
        s.h = pr[0] + 0.0045 * look.dome * (1.0 - in_j) + 0.00015 * n[1] - 0.0002 * look.fleck;
        s.rough = mix(0.74 - 0.26 * look.wear + 0.05 * n[2], 0.96, in_j);
        s.cav = mix(1.0, 0.42, in_j);
    }
}

/// ≡ stages/crossmarket/surfaces.js L81-111 (`ashlar`).
#[derive(Debug, Clone, Copy)]
pub struct Ashlar;

impl SurfProgram for Ashlar {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L83-84
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 3801);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 3803);
        f[2] = fb(ctx.uv, [120, 120], 2, 0.5, 3807);
        f[3] = Req {
            x: [ctx.uv[0] * 6.0, ctx.uv[1] * 90.0],
            per: [6, 90],
            oct: 3,
            gain: 0.5,
            seed: 3809,
        };
        w[0] = wo(ctx.uv, [48, 48], 0.9, 3811);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 26-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L85-110
        let p = ctx.p;
        let px = ctx.px;
        // L88: `COURSE(q, 0.36, 0.72, 4, 8, 0.36, 0.004)`.
        let q = course(p[0], p[1], 0.36, 0.72, 4, 8, 0.36, 0.004);
        let pr = edge_prof(q[0], 0.0035, 0.006, 0.0022, 0.006, px);
        let in_j = pr[1];
        let face = smoothstep(0.018, 0.03, q[0]);
        let t1 = q[3];
        let t2 = fract(q[3] * 7.31);
        let t3 = fract(q[3] * 3.77);
        let tool = 0.5 + 0.5 * ((p[0] + p[1]) * 190.0 + 3.0 * n[2] + t1 * 20.0).sin();
        let bed = n[3];
        let hue = mix3([1.035, 0.99, 0.935], [0.975, 0.99, 1.015], t2);
        let tone = 0.8
            * (1.0 + 0.14 * (t1 - 0.5))
            * (1.0 + 0.04 * n[1] + 0.03 * bed)
            * (1.0 - 0.035 * face * tool);
        let worn = step(0.86, t3) * smoothstep(-0.2, 0.5, n[0]);
        let g = c[0];
        let pit = worn * step(0.6, g[2]) * (1.0 - aa(0.006, g[0] * (2.88 / 48.0) * 0.4, px));
        let stain = step(0.95, fract(t1 * 17.3)) * smoothstep(0.2, -0.6, q[2]) * 0.6;
        let mut col = mul3(
            mul3(mul3(v3(tone), hue), v3(1.0 - 0.1 * worn)),
            v3(1.0 - 0.25 * pit),
        );
        col = mix3(col, mul3(col, [1.06, 0.86, 0.7]), stain);
        col = scale3(col, 1.0 + 0.03 * (1.0 - face));
        let mort = scale3([0.9, 0.88, 0.83], 1.0 + 0.05 * n[2]);
        s.alb = mix3(col, mort, in_j);
        s.h = pr[0]
            + (0.0012 * face * (0.55 + 0.45 * n[2]) + 0.00025 * face * tool - 0.0015 * pit
                + 0.0002 * bed)
                * (1.0 - in_j);
        s.rough = mix(0.78 + 0.06 * face + 0.08 * worn, 0.93, in_j);
        s.cav = mix(1.0, 0.7, in_j) * (1.0 - 0.3 * pit);
    }
}

/// ≡ stages/crossmarket/surfaces.js L115-161 (`tramway`).
#[derive(Debug, Clone, Copy)]
pub struct Tramway;

impl SurfProgram for Tramway {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L117-118
        f[0] = fb(ctx.uv, [3, 3], 4, 0.5, 3901);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 3903);
        f[2] = fb(ctx.uv, [96, 96], 2, 0.5, 3907);
        f[3] = fb(ctx.uv, [40, 40], 2, 0.5, 3913);
        w[0] = wo(ctx.uv, [190, 190], 1.0, 3911);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 42-line GLSL body
    #[allow(clippy::needless_late_init)] // `float a, b; vec4 q;` declared before the branch, as in the JS
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L119-160
        let p = ctx.p;
        let px = ctx.px;
        const GS: f32 = 2.5 / 190.0;
        let yc = p[1] - 1.25;
        let d = yc.abs() - 0.7175;
        let (a, b);
        let q;
        let border = d.abs() < 0.17;
        if border {
            a = p[0];
            b = (if d < 0.0 { -d - 0.05 } else { d - 0.05 }) + (if yc > 0.0 { 0.0 } else { 0.37 });
            // L131: `COURSE(q1, 0.12, 0.3125, 8, 4, 0.05, 0.018)`.
            q = course(a, b, 0.12, 0.3125, 8, 4, 0.05, 0.018);
        } else {
            a = p[1];
            b = p[0];
            // L133: `COURSE(q2, 0.15625, 0.25, 10, 16, 0.08, 0.024)`.
            q = course(a, b, 0.15625, 0.25, 10, 16, 0.08, 0.024);
        }
        let look = sett_look(q, n, c, GS, px);
        let pr = edge_prof(q[0], 0.0055, 0.03, 0.009, 0.016, px);
        let in_j = pr[1];
        let rz = 1.0 - smoothstep(0.052 - px, 0.052 + px, (d + 0.008).abs());
        let head = (1.0 - smoothstep(0.03 - px, 0.03 + px, (d - 0.018).abs())) * rz;
        let groove = (1.0 - smoothstep(0.016 - px, 0.016 + px, (d + 0.02).abs())) * rz;
        let lip = rz * (1.0 - head) * (1.0 - groove);
        let tar = (1.0 - smoothstep(0.052, 0.062, (d + 0.008).abs())) * (1.0 - rz);
        let steel_hi = scale3(lin([0.74, 0.74, 0.76]), 1.0 + 0.05 * n[2]);
        let steel_lo = scale3(lin([0.36, 0.33, 0.3]), 1.0 + 0.1 * n[1]);
        let groove_c = lin([0.12, 0.1, 0.09]);
        let rail_c = add3(
            add3(scale3(steel_hi, head), scale3(groove_c, groove)),
            scale3(steel_lo, lip),
        );
        let sett = (1.0 - rz) * (1.0 - tar);
        let own = add3(scale3(rail_c, rz), scale3(lin(v3(0.09)), tar));
        let lum_s = mix(
            dot3(look.stone, v3(0.3333)),
            dot3(look.joint, v3(0.3333)) * 0.85,
            in_j,
        );
        s.alb = scale3(own, 1.0 - sett);
        s.a = sett * lum_s;
        s.h = mix(
            pr[0] + 0.0045 * look.dome * (1.0 - in_j),
            -0.022 * groove - 0.001 * tar,
            1.0 - sett,
        );
        s.rough = mix(
            mix(0.74 - 0.26 * look.wear, 0.96, in_j),
            mix(0.3, 0.8, groove + lip * 0.6),
            1.0 - sett,
        );
        s.metal = (1.0 - sett) * (head * 0.9 + lip * 0.5);
        s.cav = mix(mix(1.0, 0.42, in_j), mix(1.0, 0.35, groove), 1.0 - sett);
    }
}

// ---------------------------------------------------------------------------
// lockgate (stages/lockgate/surfaces.js)
// ---------------------------------------------------------------------------

/// ≡ stages/lockgate/surfaces.js L26-75 (`setts`).
#[derive(Debug, Clone, Copy)]
pub struct LockgateSetts;

impl SurfProgram for LockgateSetts {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L24-25
        f[0] = fb(ctx.uv, [5, 5], 4, 0.5, 4001);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 4003);
        f[2] = fb(ctx.uv, [48, 48], 2, 0.5, 4007);
        w[0] = wo(ctx.uv, [260, 260], 1.0, 4011);
        w[1] = wo(ctx.uv, [40, 40], 0.9, 4013);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 49-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L26-74
        let p = ctx.p;
        let px = ctx.px;
        const RH: f32 = 0.125;
        let row = (p[1] / RH).floor() as i32;
        let rw = glsl_mod(row as f32, 16.0) as i32;
        let ly = p[1] - (row as f32) * RH;
        let nr = 8.0 + (hf([rw, 0], 41) * 3.999).floor();
        let sl = 2.0 / nr;
        let xs = p[0] - hf([rw, 1], 43) * sl;
        let ci = (xs / sl).floor();
        let lx = xs - ci * sl;
        let sid = [glsl_mod(ci, nr) as i32, rw];
        let j2 = hf2(sid, 47);
        let jit = [(j2[0] - 0.5) * 0.012, (j2[1] - 0.5) * 0.006];
        let jw = 0.0055 + 0.0025 * hf(sid, 53);
        let lp = [lx - sl * 0.5 - jit[0], ly - RH * 0.5 - jit[1]];
        let hs = [sl * 0.5 - jw - 0.002, RH * 0.5 - jw - 0.001];
        let e = jw - sd_rb(lp, hs, 0.02);
        let pr = edge_prof(e, jw, 0.016, 0.005, 0.013, px);
        let in_j = pr[1];
        // pow(x, 2.0) on a possibly-negative x: GLSL UB, libm exact (see header).
        let dome =
            (1.0 - (lp[0] / hs[0]).powf(2.0)).max(0.0) * (1.0 - (lp[1] / hs[1]).powf(2.0)).max(0.0);
        let t2 = hf2(sid, 59);
        let tilt = [t2[0] - 0.5, t2[1] - 0.5];
        let h1 = hf(sid, 61);
        let h2 = hf(sid, 67);
        let (mott, cloud, fine) = (n[0], n[1], n[2]);
        let g = c[0];
        let crystal = smoothstep(0.05, 0.25, g[1] - g[0]);
        let spk = step(0.86, g[2]) * crystal;
        let dk = step(g[2], 0.12) * crystal;
        let mut tone = 0.8 * (0.84 + 0.3 * h1) * (1.0 + 0.05 * mott + 0.03 * fine);
        let polish = smoothstep(0.35, 0.9, dome) * smoothstep(-0.2, 0.5, cloud);
        tone *= 1.0 + 0.06 * polish;
        let arris = clampf(1.0 - (e - jw) / 0.02, 0.0, 1.0) * (1.0 - in_j);
        tone *= 1.0 - 0.1 * arris;
        let hue = if h2 < 0.22 {
            lin([0.16, 0.09, 0.07])
        } else if h2 > 0.8 {
            lin([0.05, 0.07, 0.1])
        } else {
            v3(0.0)
        };
        let moss = smoothstep(0.25, 0.75, cloud * 0.6 + 0.4 * mott)
            * (0.4 + 0.6 * smoothstep(0.0, 0.02, 0.02 - (e - jw)));
        let grit = mix3(
            scale3(lin([0.2, 0.19, 0.17]), 0.8 + 0.4 * fine),
            lin([0.2, 0.25, 0.12]),
            moss * 0.8,
        );
        let mut own = scale3(scale3(hue, 1.0 - in_j), 0.6);
        own = mix3(own, lin([0.9, 0.88, 0.84]), spk * 0.5 * (1.0 - in_j));
        own = mix3(own, lin([0.08, 0.08, 0.09]), dk * 0.5 * (1.0 - in_j));
        own = mix3(own, grit, in_j);
        let cov = (1.0 - in_j) * (1.0 - 0.5 * spk) * (1.0 - 0.5 * dk);
        let edge_moss = moss * arris * 0.5;
        own = mix3(own, lin([0.16, 0.22, 0.1]), edge_moss);
        let cov = cov * (1.0 - edge_moss);
        s.alb = own;
        s.a = cov * tone;
        s.h = pr[0]
            + (0.0045 * dome + dot2(tilt, lp) * 0.02) * (1.0 - in_j)
            + 0.00012 * fine * (1.0 - in_j)
            + 0.0002 * crystal * (g[2] - 0.5);
        s.rough = mix(0.78 - 0.22 * polish + 0.05 * mott - 0.1 * spk, 0.96, in_j);
        s.cav = mix(1.0, 0.45, in_j) * (1.0 - 0.15 * arris);
    }
}

/// ≡ stages/lockgate/surfaces.js L83-122 (`engbrick`).
#[derive(Debug, Clone, Copy)]
pub struct EngBrick;

impl SurfProgram for EngBrick {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L81-82
        f[0] = fb(ctx.uv, [72, 72], 3, 0.5, 4101);
        f[1] = fb(ctx.uv, [5, 5], 4, 0.5, 4103);
        f[2] = fb(ctx.uv, [144, 144], 2, 0.5, 4107);
        f[3] = fb(ctx.uv, [9, 3], 3, 0.5, 4109);
        w[0] = wo(ctx.uv, [150, 150], 1.0, 4111);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 39-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L83-121 (the JS body never reads `c`; only the `n` chain)
        let p = ctx.p;
        let px = ctx.px;
        const CH: f32 = 0.075;
        let row = (p[1] / CH).floor() as i32;
        let ly = p[1] - (row as f32) * CH;
        let header = glsl_mod(row as f32, 2.0) as i32 == 1;
        let bl = if header { 0.1125 } else { 0.225 };
        let bx = p[0] - (if header { 0.0 } else { 0.05625 });
        let ci = (bx / bl).floor() as i32;
        let lx = bx - (ci as f32) * bl;
        let bid = wrp([ci, row], [if header { 16 } else { 8 }, 24]);
        let lp = [lx - bl * 0.5, ly - CH * 0.5];
        let e = 0.005 - sd_rb(lp, [bl * 0.5 - 0.005, CH * 0.5 - 0.005], 0.003);
        let pr = edge_prof(e, 0.005, 0.004, 0.0015, 0.004, px);
        let in_m = pr[1];
        let h1 = hf(bid, 3);
        let h2 = hf(bid, 5);
        let h3 = hf(bid, 7);
        let (sand, mott, fine) = (n[0], n[1], n[2]);
        let vit = header && h2 < 0.42;
        let tone = 0.8 * (0.8 + 0.34 * h1) * (1.0 + 0.03 * sand + 0.03 * mott);
        // pow(x, 6.0) on a possibly-negative x: GLSL UB, libm exact (see header).
        let face = (1.0 - (lp[0] / (bl * 0.5)).powf(6.0)).max(0.0)
            * (1.0 - (lp[1] / (CH * 0.5)).powf(6.0)).max(0.0);
        let foot = 1.0 - smoothstep(0.0, 0.55, p[1]);
        let bloom = smoothstep(0.35, 0.85, n[3] * 0.5 + 0.5)
            * (0.35 + 0.65 * foot)
            * smoothstep(0.1, 0.9, mott * 0.5 + 0.5);
        let damp = foot * smoothstep(-0.3, 0.4, mott) * 0.5;
        let mortar = scale3(lin([0.66, 0.64, 0.6]), 0.9 + 0.12 * fine + 0.05 * sand);
        let vit_c = scale3(
            scale3(lin([0.16, 0.17, 0.2]), 0.8 + 0.5 * h3),
            1.0 + 0.1 * sand,
        );
        let mut own = v3(0.0);
        let mut cov = 1.0;
        if vit {
            own = vit_c;
            cov = 0.12;
        }
        own = mix3(own, mortar, in_m);
        cov *= 1.0 - in_m;
        own = mix3(own, lin([0.82, 0.8, 0.76]), bloom * 0.35);
        cov *= 1.0 - bloom * 0.35;
        own = scale3(own, 1.0 - 0.3 * damp);
        s.alb = own;
        s.a = cov * tone * (1.0 - 0.3 * damp) * (1.0 - 0.06 * (1.0 - face) * (1.0 - in_m));
        s.h = pr[0] + (0.0004 * face * h3 + 0.00015 * sand) * (1.0 - in_m) - 0.0006 * in_m * fine;
        s.rough = mix(
            if vit {
                0.34 + 0.1 * sand
            } else {
                0.78 + 0.06 * sand - 0.05 * h1
            },
            0.93,
            in_m,
        );
        s.rough = mix(s.rough, 0.9, bloom * 0.4);
        s.cav = mix(1.0, 0.62, in_m) * (1.0 - 0.1 * damp);
    }
}

/// ≡ stages/lockgate/surfaces.js L130-179 (`hoofsteps`).
#[derive(Debug, Clone, Copy)]
pub struct HoofSteps;

impl SurfProgram for HoofSteps {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L128-129
        f[0] = fb(ctx.uv, [5, 5], 4, 0.5, 4201);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 4203);
        f[2] = fb(ctx.uv, [48, 48], 2, 0.5, 4207);
        w[0] = wo(ctx.uv, [220, 220], 1.0, 4211);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 48-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L130-178
        let p = ctx.p;
        let px = ctx.px;
        // GLSL also declares `const float RH = 0.1125` (L134), never used in
        // the body; dropped here (its 0.5 multiples appear as the 0.0425s).
        const PD: f32 = 0.45;
        let j = (p[1] / PD).floor();
        let a = p[1] - j * PD;
        let rb = 0.055 - (a - 0.055).abs();
        let rib = smoothstep(-px, px, rb);
        let rib_h = 0.025 * clampf(rb / 0.02, 0.0, 1.0).sqrt();
        let sy = a - 0.11;
        let crs = (sy.max(0.0) / 0.085).floor() as i32;
        let cw = glsl_mod(j * 4.0 + (crs as f32), 16.0) as i32;
        let ly = sy - (crs as f32) * 0.085;
        let nr = 7.0 + (hf([cw, 0], 71) * 2.999).floor();
        let sl = 1.8 / nr;
        let xs = p[0] - hf([cw, 1], 73) * sl;
        let ci = (xs / sl).floor();
        let lx = xs - ci * sl;
        let sid = [glsl_mod(ci, nr) as i32, cw];
        let lp = [lx - sl * 0.5, ly - 0.0425];
        let hs = [sl * 0.5 - 0.007, 0.0425 - 0.006];
        let e = 0.006 - sd_rb(lp, hs, 0.016);
        let pr = edge_prof(e, 0.006, 0.012, 0.004, 0.012, px);
        let in_j = pr[1] * (1.0 - rib) * step(0.0, sy);
        // pow(x, 2.0) on a possibly-negative x: GLSL UB, libm exact (see header).
        let dome =
            (1.0 - (lp[0] / hs[0]).powf(2.0)).max(0.0) * (1.0 - (lp[1] / hs[1]).powf(2.0)).max(0.0);
        let dish = -0.012 * (1.0 - smoothstep(0.11, PD, a));
        let h1 = hf(sid, 79);
        let (mott, cloud, fine) = (n[0], n[1], n[2]);
        let g = c[0];
        let crystal = smoothstep(0.05, 0.25, g[1] - g[0]);
        let spk = step(0.87, g[2]) * crystal;
        let tone = 0.8 * (0.85 + 0.28 * h1) * (1.0 + 0.05 * mott + 0.03 * fine);
        let rib_tone =
            0.8 * (0.72 + 0.1 * hf([glsl_mod(j, 4.0) as i32, 0], 83)) * (1.0 + 0.04 * fine);
        let rib_pol = rib * smoothstep(0.004, 0.02, rb);
        let grit = scale3(lin([0.2, 0.19, 0.17]), 0.8 + 0.4 * fine);
        let mut own = mix3(v3(0.0), lin([0.9, 0.88, 0.84]), spk * 0.4);
        own = mix3(own, grit, in_j);
        let mut cov = (1.0 - in_j) * (1.0 - 0.4 * spk);
        let bank = (1.0 - smoothstep(0.11, 0.16 + 0.03 * cloud, a))
            * (1.0 - rib)
            * smoothstep(-0.2, 0.4, cloud);
        own = mix3(own, scale3(grit, 1.2), bank * 0.6);
        cov *= 1.0 - bank * 0.6;
        s.alb = own;
        s.a = cov * mix(tone, rib_tone * (1.0 + 0.12 * rib_pol), rib);
        let h_s = pr[0] + 0.004 * dome * (1.0 - in_j) + dish;
        s.h = mix(h_s, h_s.max(rib_h), rib) + 0.00012 * fine;
        s.rough = mix(
            mix(0.8 + 0.05 * mott - 0.1 * spk, 0.96, in_j),
            0.5 - 0.15 * rib_pol,
            rib,
        );
        s.cav = mix(1.0, 0.45, in_j)
            * (1.0 - 0.45 * (-((a - 0.113) / 0.008).powf(2.0)).exp() * (1.0 - rib));
    }
}

// ---------------------------------------------------------------------------
// terraces (stages/terraces/surfaces.js)
// ---------------------------------------------------------------------------

/// ≡ stages/terraces/surfaces.js L23-52 (`calce`).
#[derive(Debug, Clone, Copy)]
pub struct Calce;

impl SurfProgram for Calce {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L25-28
        let uv = ctx.uv;
        f[0] = fb(uv, [3, 3], 4, 0.55, 4301);
        f[1] = fb(uv, [14, 14], 3, 0.5, 4303);
        f[2] = fb(uv, [64, 64], 2, 0.5, 4307);
        f[3] = fb(uv, [36, 3], 3, 0.5, 4309);
        let wq = [
            uv[0] + 0.01 * (TAU * (5.0 * uv[1] + 2.0 * uv[0])).sin(),
            uv[1] + 0.01 * (TAU * (4.0 * uv[0] - 3.0 * uv[1]) + 1.3).sin(),
        ];
        w[0] = wo(wq, [7, 11], 0.75, 4311);
        let wq1 = [
            uv[0] + 0.012 * (TAU * 7.0 * uv[1]).sin(),
            uv[1] + 0.012 * (TAU * 5.0 * uv[0]).sin(),
        ];
        w[1] = wo(wq1, [24, 24], 0.9, 4313);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 22-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L29-51
        let px = ctx.px;
        let st = c[0];
        let s_edge = st[1] - st[0];
        let joint = 1.0 - smoothstep(0.0, 0.22, s_edge);
        let dome = smoothstep(0.0, 0.6, s_edge);
        let (blot, brush, grain) = (n[0], n[1], n[2]);
        let runs = smoothstep(0.15, 0.75, n[3]);
        let mut wash = 0.87
            * (1.0 + 0.035 * blot + 0.025 * brush + 0.02 * grain)
            * (1.0 - 0.035 * runs)
            * (1.0 - 0.012 * joint);
        wash *= 1.0 + 0.02 * (fract(st[2] * 5.31) - 0.5);
        let fk = c[1];
        let cluster = smoothstep(0.25, 0.6, blot) * step(0.84, fract(fk[2] * 7.3 + fk[3] * 1.7));
        let flake = cluster * (1.0 - aa(0.0, (fk[0] - 0.34 - 0.14 * grain) * 0.15, px));
        let stone_c = scale3(
            mix3(
                lin([0.66, 0.62, 0.55]),
                lin([0.56, 0.55, 0.52]),
                fract(st[2] * 3.7),
            ),
            0.9 + 0.18 * fract(st[2] * 11.3),
        );
        let bare = mix3(
            stone_c,
            scale3(lin([0.72, 0.69, 0.62]), 0.95 + 0.1 * grain),
            smoothstep(0.4, 0.9, joint),
        );
        let crack = (1.0 - aa(0.0005, s_edge * 0.35, px))
            * step(0.88, fract(st[2] * 13.1 + st[3] * 5.7))
            * smoothstep(0.1, 0.5, blot);
        s.alb = scale3(bare, flake);
        s.a = (1.0 - flake) * wash * (1.0 - 0.3 * crack);
        s.h = 0.0007 * dome - 0.0003 * joint * joint + 0.0004 * brush + 0.0001 * grain
            - 0.0012 * flake
            - 0.0005 * crack;
        s.rough = 0.9 - 0.05 * blot + 0.03 * flake;
        s.cav = 1.0 - 0.02 * joint - 0.2 * flake * joint - 0.25 * crack;
    }
}

/// ≡ stages/terraces/surfaces.js L54-93 (`cotto`).
#[derive(Debug, Clone, Copy)]
pub struct Cotto;

impl SurfProgram for Cotto {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L56-58
        let uv = ctx.uv;
        let tc = wrp(
            [
                (ctx.p[0] / 0.3).floor() as i32,
                (ctx.p[1] / 0.3).floor() as i32,
            ],
            [4, 4],
        );
        f[0] = fb(uv, [4, 4], 3, 0.5, 4401);
        // tc is wrp'd into [0, 4)^2, so the seed arithmetic is non-negative.
        f[1] = fb(uv, [16, 16], 3, 0.5, 4403 + (tc[0] * 4 + tc[1]) as u32 * 97);
        f[2] = fb(uv, [96, 96], 2, 0.5, 4407);
        w[0] = wo(uv, [140, 140], 1.0, 4411);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 33-line GLSL body
    #[allow(clippy::approx_constant)] // 0.70710678 is the JS literal; FRAC_1_SQRT_2 would re-bake
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L59-92
        let p = ctx.p;
        let px = ctx.px;
        const TS: f32 = 0.3;
        const TH: f32 = 0.15;
        const TC: f32 = 0.2511;
        let cell = [(p[0] / TS).floor() as i32, (p[1] / TS).floor() as i32];
        let cw = wrp(cell, [4, 4]);
        let lp = [
            p[0] - (cell[0] as f32 + 0.5) * TS,
            p[1] - (cell[1] as f32 + 0.5) * TS,
        ];
        let ap = [lp[0].abs(), lp[1].abs()];
        let d1 = TH - ap[0].max(ap[1]);
        let d2 = (TC - (ap[0] + ap[1])) * 0.70710678;
        let ins = d2 < 0.0;
        let e = if ins { -d2 } else { d1.min(d2) };
        let pr = edge_prof(
            e,
            0.003,
            if ins { 0.004 } else { 0.009 },
            if ins { 0.0008 } else { 0.0022 },
            0.004,
            px,
        );
        let in_j = pr[1];
        let hid = hf(cw, 3);
        let hid2 = hf(cw, 7);
        let hid3 = hf(cw, 13);
        let mut terra = mix3(lin([0.74, 0.47, 0.35]), lin([0.76, 0.55, 0.43]), hid2);
        terra = mix3(terra, lin([0.6, 0.36, 0.27]), step(0.88, hid3) * 0.55);
        terra = scale3(
            terra,
            (0.9 + 0.18 * hid) * (1.0 + 0.08 * n[0] + 0.07 * n[1] + 0.03 * n[2]),
        );
        let wear = smoothstep(0.03, 0.12, e) * smoothstep(-0.2, 0.6, n[0]);
        terra = mix3(terra, mul3(terra, [1.08, 1.1, 1.14]), 0.35 * wear);
        let g0 = c[0];
        let grain = smoothstep(0.05, 0.25, g0[1] - g0[0]) * (g0[2] - 0.5);
        terra = scale3(terra, 1.0 + 0.08 * grain);
        let corner = wrp(
            [
                cell[0] + (if lp[0] > 0.0 { 1 } else { 0 }),
                cell[1] + (if lp[1] > 0.0 { 1 } else { 0 }),
            ],
            [4, 4],
        );
        let kid = hf(corner, 17);
        let glaze = if kid < 0.3 {
            lin([0.2, 0.36, 0.6])
        } else if kid < 0.95 {
            lin([0.88, 0.86, 0.79])
        } else {
            lin([0.88, 0.74, 0.34])
        };
        let ring = 1.0 - smoothstep(0.004, 0.012, e);
        let glaze = scale3(glaze, (1.0 + 0.05 * n[1]) * (1.0 - 0.25 * ring));
        let grout = scale3(lin([0.7, 0.66, 0.58]), 1.0 + 0.06 * n[2]);
        let col = if ins { glaze } else { terra };
        s.alb = mix3(col, grout, in_j);
        s.h = pr[0]
            + (if ins {
                0.0002 * n[1]
            } else {
                0.00025 * n[0] + 0.0001 * grain
            }) * (1.0 - in_j);
        s.rough = mix(
            if ins {
                0.22 + 0.05 * n[1]
            } else {
                0.72 - 0.12 * wear + 0.06 * hid2
            },
            0.93,
            in_j,
        );
        s.cav = mix(1.0, 0.55, in_j);
    }
}

/// ≡ stages/terraces/surfaces.js L95-122 (`pebble`).
#[derive(Debug, Clone, Copy)]
pub struct Pebble;

impl SurfProgram for Pebble {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L97-98
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 4501);
        f[1] = fb(ctx.uv, [24, 24], 3, 0.5, 4503);
        f[2] = fb(ctx.uv, [120, 120], 2, 0.5, 4507);
        w[0] = wo(ctx.uv, [40, 40], 0.92, 4511);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 22-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L99-121
        let p = ctx.p;
        let px = ctx.px;
        let jx = jd(p[0], 2.4);
        let jy = jd(p[1], 2.4);
        let db = jx.min(jy);
        let along_x = jy < jx;
        let band = 1.0 - aa(0.07, db, px);
        let bj = jd(if along_x { p[0] } else { p[1] }, 0.6);
        let pb = edge_prof((0.07 - db).min(bj), 0.0025, 0.008, 0.002, 0.005, px);
        let pc = c[0];
        let pe = pc[1] - pc[0];
        let peb = smoothstep(0.04, 0.15, pe);
        let dome = clampf(pe / 0.45, 0.0, 1.0).sqrt();
        let id = pc[2];
        let id2 = fract(id * 7.13);
        let mut pcol = if id < 0.4 {
            lin([0.7, 0.69, 0.66])
        } else if id < 0.7 {
            lin([0.72, 0.68, 0.61])
        } else if id < 0.84 {
            lin([0.8, 0.79, 0.75])
        } else if id < 0.93 {
            lin([0.55, 0.55, 0.54])
        } else {
            lin([0.7, 0.63, 0.52])
        };
        pcol = scale3(
            pcol,
            (0.92 + 0.14 * id2) * (1.0 + 0.06 * n[2]) * (0.94 + 0.08 * dome),
        );
        let mortar = scale3(lin([0.66, 0.62, 0.55]), 1.0 + 0.08 * n[1] + 0.04 * n[0]);
        let lime = scale3(
            scale3(
                lin([0.8, 0.77, 0.7]),
                1.0 + 0.05 * n[0] + 0.04 * n[1] + 0.03 * n[2],
            ),
            0.95 + 0.1
                * hf(
                    [
                        ((if along_x { p[0] } else { p[1] }) / 0.6).floor() as i32,
                        0,
                    ],
                    5,
                ),
        );
        let fill = mix3(mortar, pcol, peb);
        let grime = smoothstep(0.1, 0.7, n[0]) * 0.08;
        s.alb = scale3(
            mix3(fill, mix3(lime, scale3(lime, 0.7), pb[1]), band),
            1.0 - grime,
        );
        s.h = mix(
            -0.004 + 0.0065 * dome * peb + 0.0002 * n[2],
            pb[0] + 0.0002 * n[1],
            band,
        );
        s.rough = mix(
            mix(0.93, 0.6 + 0.1 * id2, peb),
            mix(0.74, 0.92, pb[1]),
            band,
        );
        s.cav = mix(mix(0.7, 1.0, peb), mix(1.0, 0.65, pb[1]), band);
    }
}

/// Resolve a `'<stage>:<name>'` layer name to its program; `None` for base
/// layers (see [`super::materials::lookup`]) and unknown names. The three
/// `cargo:*` names share the `kelpline` instances (byte-identical sources).
pub fn lookup(name: &str) -> Option<&'static dyn SurfProgram> {
    Some(match name {
        "tidewater:herringbone" => &Herringbone,
        "tidewater:terrazzo" => &Terrazzo,
        "tidewater:stucco" => &Stucco,
        "kelpline:tarmac" => &Tarmac,
        "kelpline:quay" => &Quay,
        "kelpline:chequer" => &Chequer,
        "saltpan:saltcrust" => &SaltCrust,
        "saltpan:drymud" => &DryMud,
        "saltpan:saltboard" => &SaltBoard,
        "crossmarket:setts" => &MarketSetts,
        "crossmarket:ashlar" => &Ashlar,
        "crossmarket:tramway" => &Tramway,
        "lockgate:setts" => &LockgateSetts,
        "lockgate:engbrick" => &EngBrick,
        "lockgate:hoofsteps" => &HoofSteps,
        "terraces:calce" => &Calce,
        "terraces:cotto" => &Cotto,
        "terraces:pebble" => &Pebble,
        // cargo/surfaces.js GLSL is byte-identical to kelpline's: shared.
        "cargo:tarmac" => &Tarmac,
        "cargo:quay" => &Quay,
        "cargo:chequer" => &Chequer,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texlib::bake::bake_layer;
    use crate::texlib::schema::embedded_texlib;

    /// The 21 stage layers, in `assets/texlib.json` order (layers[25..46]).
    const STAGE: [&str; 21] = [
        "tidewater:herringbone",
        "tidewater:terrazzo",
        "tidewater:stucco",
        "kelpline:tarmac",
        "kelpline:quay",
        "kelpline:chequer",
        "saltpan:saltcrust",
        "saltpan:drymud",
        "saltpan:saltboard",
        "crossmarket:setts",
        "crossmarket:ashlar",
        "crossmarket:tramway",
        "lockgate:setts",
        "lockgate:engbrick",
        "lockgate:hoofsteps",
        "terraces:calce",
        "terraces:cotto",
        "terraces:pebble",
        "cargo:tarmac",
        "cargo:quay",
        "cargo:chequer",
    ];

    #[test]
    fn registry_covers_21_stage_layers() {
        let doc = embedded_texlib();
        assert_eq!(doc.layers.len(), 46);
        for (i, name) in STAGE.iter().enumerate() {
            assert_eq!(doc.layers[25 + i].name, *name, "schema layer {}", 25 + i);
            assert!(lookup(name).is_some(), "lookup missing stage layer {name}");
        }
        assert!(lookup("concrete").is_none()); // base layers → materials.rs
        assert!(lookup("tidewater:Herringbone").is_none()); // names are exact
        assert!(lookup("no-such:layer").is_none());
    }

    /// Bake `name` at 32 px, assert the double run is byte-identical, return
    /// the golden digest (albedo + normal + orm base level concatenated).
    fn golden(name: &str) -> String {
        let doc = embedded_texlib();
        let layer = doc.layers.iter().find(|l| l.name == name).expect(name);
        let prog = lookup(name).expect(name);
        let a = bake_layer(layer, prog, 32);
        let b = bake_layer(layer, prog, 32);
        assert_eq!(a, b, "{name}: bake is not byte-deterministic");
        let mut data = Vec::new();
        for chain in [&a.albedo, &a.normal, &a.orm] {
            assert_eq!(chain.levels[0].size, 32);
            data.extend_from_slice(&chain.levels[0].rgba);
        }
        hex(&sha256(&data))
    }

    /// 32 px bake goldens, pinned after the first deterministic run (batch
    /// C2): sha256 of the base-level albedo + normal + orm bytes.
    const GOLDENS: [(&str, &str); 21] = [
        (
            "tidewater:herringbone",
            "a574ef758645912b9674ee6679ab0d7f2b69cf79a5b3fa623a7c09d8ed5687c2",
        ),
        (
            "tidewater:terrazzo",
            "01f590dc0130ad81135abbd2aaa4650c257cc7d1315496e6f902be01ab1caef9",
        ),
        (
            "tidewater:stucco",
            "061a518ee680e678d84e857465d1dc956c16e111fb65bb139c3e3aef010999ba",
        ),
        (
            "kelpline:tarmac",
            "a24eea60330e67ec7233a19694e7229a99720e252cac95f016e9b0b02126003d",
        ),
        (
            "kelpline:quay",
            "ced6a4484b0f2296b999c57f12b8be9769effa0c40f4ea3ddc0d1a199a4f9eaa",
        ),
        (
            "kelpline:chequer",
            "c3aa105ffe8a8e793e69705e06d392b7c986d4af8b004d65da9e0ff5cf7f3bdd",
        ),
        (
            "saltpan:saltcrust",
            "792027f5b7ef832e3ed8ebe6ef524d44adeb84ab8937e67c63671e0da008d775",
        ),
        (
            "saltpan:drymud",
            "5ae58fbc20072fff1414b27ba8a94362c6377a203cb039b1a8dd2db0a8d7b15c",
        ),
        (
            "saltpan:saltboard",
            "511c11b65ff6dfd4b74646d743ffdf48c779c28abbeb2c0a11ae05065f496662",
        ),
        (
            "crossmarket:setts",
            "0bebc7f78d56976cd55b578439adf921f1d8689e33c658515103736fc345e40c",
        ),
        (
            "crossmarket:ashlar",
            "2fcf7add5952320042c7e43beb1f1a4bf8cbe68a62fcc6782ecd1d6881e32bda",
        ),
        (
            "crossmarket:tramway",
            "83a1fc0ba7bce4f28b37a31ca9742f9fe6279c36ffc0a7f1b1a5d5a6c008416e",
        ),
        (
            "lockgate:setts",
            "df8d5adb725bda7a88ffd362902e69b927102ac819be86bb37a928940b9b3600",
        ),
        (
            "lockgate:engbrick",
            "9813d5f15901438cb0003b7a872fc3e7359135d4dd80e79d60e3f0cd84252812",
        ),
        (
            "lockgate:hoofsteps",
            "24dfc5a61969bdd7d47c52aee586f0d5c64eefe1c74770f9ca817a025e5880e9",
        ),
        (
            "terraces:calce",
            "5deb6fbec98ea4c21a19900cf4321e3e6787f3bc1dae1edc78f8b75d33fa7202",
        ),
        (
            "terraces:cotto",
            "3311403abce7af51e947f9542d1420b2a51b7f1c7a94ec3de1be96c03190f127",
        ),
        (
            "terraces:pebble",
            "bbd259c4d1b5d7a6cd1bbdf1a54b2a948f99c85ad30a88193941dba844b522e3",
        ),
        (
            "cargo:tarmac",
            "a24eea60330e67ec7233a19694e7229a99720e252cac95f016e9b0b02126003d",
        ),
        (
            "cargo:quay",
            "ced6a4484b0f2296b999c57f12b8be9769effa0c40f4ea3ddc0d1a199a4f9eaa",
        ),
        (
            "cargo:chequer",
            "c3aa105ffe8a8e793e69705e06d392b7c986d4af8b004d65da9e0ff5cf7f3bdd",
        ),
    ];

    #[test]
    fn bake_goldens() {
        for (name, want) in GOLDENS {
            assert_eq!(golden(name), want, "{name}: 32 px bake golden mismatch");
        }
    }

    #[test]
    fn bake_sanity() {
        let doc = embedded_texlib();
        for name in STAGE {
            let layer = doc.layers.iter().find(|l| l.name == name).expect(name);
            let prog = lookup(name).expect(name);
            let b = bake_layer(layer, prog, 32);
            let alb = &b.albedo.levels[0].rgba;
            let nrm = &b.normal.levels[0].rgba;
            let orm = &b.orm.levels[0].rgba;
            assert_eq!(alb.len(), 32 * 32 * 4, "{name}: albedo base size");
            // u8 bytes are trivially in 0..=255; the weak sanity is that the
            // fields are alive: mask layers may legitimately sit at rgb 0
            // (paint carries the colour) but must show non-zero albedo.a,
            // tint-only layers must show non-zero rgb; the normal map must
            // leave the flat (128, 128) plane somewhere; the height channel
            // must differ from the flat NEUTRAL 255 somewhere.
            if layer.mask {
                assert!(
                    alb.iter().skip(3).step_by(4).any(|v| *v > 0),
                    "{name}: mask layer with zero paint everywhere"
                );
            } else {
                assert!(
                    alb.iter().step_by(4).any(|v| *v > 0),
                    "{name}: albedo all zero"
                );
            }
            assert!(
                nrm.chunks(4).any(|px| px[0] != 128 || px[1] != 128),
                "{name}: normal map perfectly flat"
            );
            assert!(
                orm.iter().skip(3).step_by(4).any(|v| *v != 255),
                "{name}: orm.a all 255"
            );
        }
    }

    // -- std-only SHA-256 (test fixture pinning; FIPS 180-4; copy of the
    // batch-B fixture in bake.rs, which is private to that test module) ----
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
}
