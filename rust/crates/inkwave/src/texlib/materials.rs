//! Batch C1: the 25 base MATERIALS surf programs (≡ `src/world/texlib.js`
//! L211-1163, prep/surf GLSL → Rust, line-for-line).
//!
//! Translation rules (same discipline as batch B, spec D-T52-2/7):
//! - `fract`/`mod`/`mix`/`step`/`smoothstep`/`clamp`/`dot`/`length` and the
//!   noise helpers (`FB`/`FNP`/`WO`/`aa`/`jd`/`sdRB`/`edgeProf`/`lin`/`hf`/
//!   `hf2`/`wrp`) all resolve to the batch-B ports in [`super::r#gen`]; the
//!   GLSL global `PX` arrives as `ctx.px`.
//! - Layer meta uniforms (`uHRange`/`uAO`/`uDetail`) are not referenced by
//!   any base-layer `surf` body (they are only read by `GEN_MAIN`, already
//!   ported in [`super::bake`]), so `ctx.layer` stays unused here.
//! - `vec3`/`vec2` are `[f32; 3]`/`[f32; 2]` with the local `mix3`/`scale3`/
//!   `mul3` helpers mirroring GLSL component-wise ops; `ivec2` is `[i32; 2]`.
//! - `pow(x, 2.0)`/`pow(x, 3.0)` on a possibly-negative `x` is GLSL UB;
//!   both libm and desktop drivers return the exact integer power there, so
//!   `f32::powf` (which matches libm for integral exponents) is the faithful
//!   port. Declared deviation, same class as the bake.rs store/knee notes.
//! - `uint(f)`/`ivec2(f)` truncation ≡ Rust `as u32`/`as i32` (all call sites
//!   are non-negative integer-valued expressions).
//! - The JS bodies are ported verbatim, including redundant terms (e.g. the
//!   `oilK = 0.0` multiplier in `yard`); no algebraic "simplification".

use super::bake::{PrepCtx, Surf, SurfCtx, SurfProgram};
use super::r#gen::{
    PI, Req, TAU, WReq, aa, clamp01, clampf, dot2, edge_prof, fb, fnp, fract, glsl_mod, hf, hf2,
    jd, length2, lin, mix, sd_rb, smoothstep, step, wo, wrp,
};

type V2 = [f32; 2];
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

/// ≡ texlib.js L213-236.
#[derive(Debug, Clone, Copy)]
pub struct Concrete;

impl SurfProgram for Concrete {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L214-215
        f[0] = fb(ctx.uv, [3, 3], 5, 0.55, 11);
        f[1] = fb(ctx.uv, [8, 8], 3, 0.5, 19);
        f[2] = fb(ctx.uv, [24, 24], 3, 0.5, 23);
        w[0] = wo(ctx.uv, [64, 64], 0.8, 37);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L216-235
        let p = ctx.p;
        let px = ctx.px;
        let pan = wrp(
            [(p[0] / 2.0).floor() as i32, (p[1] / 2.0).floor() as i32],
            [2, 2],
        );
        let d_seam = jd(p[0], 2.0).min(jd(p[1], 2.0));
        let seam = 1.0 - aa(0.003, d_seam, px);
        let seam_soft = 1.0 - smoothstep(0.0, 0.045, d_seam);
        let (mott, cloud, fine) = (n[0], n[1], n[2]);
        let tone = 1.0 + 0.07 * (hf(pan, 5) - 0.5);
        let col = v3(0.805 * tone * (1.0 + 0.09 * mott + 0.05 * cloud + 0.025 * fine));
        let rh = length2([glsl_mod(p[0], 2.0) - 1.0, glsl_mod(p[1], 2.0) - 1.0]);
        let hole = 1.0 - aa(0.011, rh, px);
        let ring = (1.0 - aa(0.02, rh, px)) * (1.0 - hole);
        let wc = c[0];
        let pr = wc[0] * (4.0 / 64.0);
        let fz = fract(wc[2] * 13.7);
        let pore = step(0.975, wc[2])
            * (1.0 - smoothstep(0.003 + 0.004 * fz, 0.0045 + 0.004 * fz + px, pr));
        s.alb = scale3(
            col,
            (1.0 - 0.07 * pore)
                * (1.0 - 0.10 * hole)
                * (1.0 - 0.12 * seam)
                * (1.0 - 0.035 * seam_soft)
                * (1.0 + 0.02 * ring),
        );
        s.h = 0.00035 * mott + 0.0001 * fine - 0.0015 * pore - 0.0032 * hole - 0.0008 * seam
            + 0.0001 * ring;
        s.rough = 0.78 + 0.07 * mott - 0.04 * cloud + 0.04 * (hf(pan, 9) - 0.5);
        s.cav = 1.0 - 0.3 * hole - 0.2 * pore - 0.25 * seam;
    }
}

/// ≡ texlib.js L238-261.
#[derive(Debug, Clone, Copy)]
pub struct Pavers;

impl SurfProgram for Pavers {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L239
        f[0] = fb(ctx.uv, [6, 6], 4, 0.5, 17);
        f[1] = fb(ctx.uv, [3, 3], 3, 0.5, 29);
        w[0] = wo(ctx.uv, [240, 240], 1.0, 31);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L240-260
        let p = ctx.p;
        let px = ctx.px;
        let cell = [(p[0] / 2.0).floor() as i32, (p[1] / 2.0).floor() as i32];
        let cw = wrp(cell, [2, 2]);
        let lp = [
            p[0] - (cell[0] as f32 * 2.0 + 1.0),
            p[1] - (cell[1] as f32 * 2.0 + 1.0),
        ];
        let e = -sd_rb(lp, [1.0, 1.0], 0.016);
        let pr = edge_prof(e, 0.0075, 0.016, 0.0045, 0.0075, px);
        let in_j = pr[1];
        let hid = hf(cw, 3);
        let hid2 = hf(cw, 11);
        let th = hf2(cw, 9);
        let tilt = [th[0] - 0.5, th[1] - 0.5];
        let (mott, cloud) = (n[0], n[1]);
        let grain = smoothstep(0.05, 0.2, c[0][1] - c[0][0]) * (c[0][2] - 0.5);
        let col =
            v3(0.84 * (1.0 + 0.2 * (hid - 0.5)) * (1.0 + 0.07 * mott + 0.05 * cloud + 0.1 * grain));
        let arris = clampf(1.0 - (e - 0.0075) / 0.016, 0.0, 1.0) * (1.0 - in_j);
        // L254: soft dirt band hugging the joints.
        let grime = (1.0 - smoothstep(0.0, 0.06, e - 0.0075)) * (1.0 - in_j);
        let col = scale3(
            col,
            (1.0 - 0.06 * arris) * (1.0 - 0.09 * grime * (0.7 + 0.3 * cloud)),
        );
        let col = mix3(col, v3(0.42 * (1.0 + 0.1 * mott)), in_j);
        s.alb = col;
        s.h = pr[0] + dot2(tilt, lp) * 0.0012 * (1.0 - in_j) + 0.00015 * mott + 0.00005 * grain;
        s.rough = mix(0.7 + 0.12 * hid2 + 0.06 * mott + 0.05 * grime, 0.95, in_j);
        s.cav = mix(1.0, 0.5, in_j) * (1.0 - 0.1 * grime);
    }
}

/// ≡ texlib.js L263-283.
#[derive(Debug, Clone, Copy)]
pub struct Tiles;

impl SurfProgram for Tiles {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L264-265; `uint(cw.x*4 + cw.y)` is a non-negative integer cell id.
        let p = ctx.p;
        let cw = wrp(
            [(p[0] / 0.5).floor() as i32, (p[1] / 0.5).floor() as i32],
            [4, 4],
        );
        f[0] = fb(
            ctx.uv,
            [12, 12],
            1,
            0.5,
            41u32.wrapping_add(((cw[0] * 4 + cw[1]) as u32).wrapping_mul(131)),
        );
        f[1] = fb(ctx.uv, [8, 8], 3, 0.5, 13);
        f[2] = fb(ctx.uv, [96, 96], 2, 0.5, 7);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L266-282
        let p = ctx.p;
        let px = ctx.px;
        let cell = [(p[0] / 0.5).floor() as i32, (p[1] / 0.5).floor() as i32];
        let cw = wrp(cell, [4, 4]);
        let lp = [
            p[0] - (cell[0] as f32 * 0.5 + 0.25),
            p[1] - (cell[1] as f32 * 0.5 + 0.25),
        ];
        let e = -sd_rb(lp, [0.25, 0.25], 0.007);
        let pr = edge_prof(e, 0.0045, 0.006, 0.0014, 0.0026, px);
        let in_g = pr[1];
        let hid = hf(cw, 3);
        let hid2 = hf(cw, 5);
        let (wave, mott) = (n[0], n[1]);
        let edge_tone = 1.0 - smoothstep(0.0, 0.035, e - 0.0045);
        let col = v3(0.84 * (1.0 + 0.07 * (hid - 0.5) + 0.015 * mott) * (1.0 - 0.05 * edge_tone));
        let grout = v3(0.6 * (1.0 + 0.06 * n[2]));
        s.alb = mix3(col, grout, in_g);
        s.h = pr[0] + (0.00022 * wave + 0.00004 * mott) * (1.0 - in_g);
        s.rough = mix(0.18 + 0.08 * hid2 + 0.03 * mott, 0.9, in_g);
        s.cav = mix(1.0, 0.6, in_g);
    }
}

/// ≡ texlib.js L285-301.
#[derive(Debug, Clone, Copy)]
pub struct Asphalt;

impl SurfProgram for Asphalt {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L286-287
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 13);
        f[1] = fb(ctx.uv, [64, 64], 2, 0.5, 7);
        w[0] = wo(ctx.uv, [150, 150], 1.0, 41);
        w[1] = wo(ctx.uv, [360, 360], 1.0, 53);
    }

    fn surf(&self, _ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L288-300
        let (wc, w2) = (c[0], c[1]);
        let stone = smoothstep(0.06, 0.24, wc[1] - wc[0]);
        let light = step(0.9, wc[2]);
        let st = mix(0.76 + 0.16 * fract(wc[2] * 7.13), 1.05, light);
        let big = n[0];
        let binder = 0.63 + 0.05 * n[1];
        let grit = smoothstep(0.05, 0.25, w2[1] - w2[0]) * (fract(w2[2] * 5.31) - 0.5);
        // L297: traffic-polished patches: lighter, a bit smoother.
        let worn = smoothstep(-0.2, 0.5, big);
        s.alb = v3(mix(binder, st, stone) * (1.0 + 0.06 * big + 0.06 * grit) * (1.0 + 0.04 * worn));
        s.h = -0.0011 * (1.0 - stone)
            + 0.0005 * stone * (1.0 - wc[0] * wc[0])
            + 0.00012 * grit
            + 0.0001 * big;
        s.rough = mix(0.94, 0.8, stone * (1.0 - light * 0.5)) - 0.1 * worn;
    }
}

/// ≡ texlib.js L303-347.
#[derive(Debug, Clone, Copy)]
pub struct Boardwalk;

impl SurfProgram for Boardwalk {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L304-309
        let p = ctx.p;
        let row = (p[1] / 0.25).floor() as i32;
        let rw = (row + 8) % 8;
        const KK: [i32; 8] = [0, 2, 1, 3, 2, 0, 3, 1];
        let xp = glsl_mod(p[0] - 0.25 - 0.5 * KK[rw as usize] as f32, 2.0);
        let ly = p[1] - row as f32 * 0.25;
        let rh = hf([rw, 0], 3);
        let rh2 = hf([rw, 1], 3);
        let sd = (rw as u32).wrapping_mul(7919).wrapping_add(101);
        f[0] = fnp([xp * 1.3 + rh * 40.0, ly * 17.0 + rh * 40.0], 4, sd);
        f[1] = fnp(
            [xp * 0.5 + rh2 * 30.0, ly * 4.0 + rh2 * 30.0],
            3,
            sd.wrapping_add(17),
        );
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L310-346
        let p = ctx.p;
        let px = ctx.px;
        const PW: f32 = 0.25;
        let row = (p[1] / PW).floor() as i32;
        let rw = (row + 8) % 8;
        let ly = p[1] - row as f32 * PW;
        const KK: [i32; 8] = [0, 2, 1, 3, 2, 0, 3, 1];
        let jx = 0.25 + 0.5 * KK[rw as usize] as f32;
        let xp = glsl_mod(p[0] - jx, 2.0);
        let e_side = ly.min(PW - ly);
        let e_end = xp.min(2.0 - xp);
        let ps = edge_prof(e_side, 0.004, 0.006, 0.0022, 0.015, px);
        let pe = edge_prof(e_end, 0.0015, 0.004, 0.0014, 0.012, px);
        let gap = ps[1].max(pe[1]);
        let mut h = ps[0].min(pe[0]);
        let rh = hf([rw, 0], 3);
        let rh2 = hf([rw, 1], 3);
        let rh3 = hf([rw, 2], 3);
        let (streak, warp) = (n[0], n[1]);
        let ring = 0.5 + 0.5 * ((ly * 64.0 + warp * 3.5 + rh2 * 9.0) * PI).cos();
        let lines = smoothstep(0.72, 1.0, ring);
        let wa = lin([0.79, 0.64, 0.47]);
        let wb = lin([0.74, 0.57, 0.42]);
        let wc = lin([0.82, 0.69, 0.51]);
        let base = mix3(wa, if rh3 > 0.5 { wb } else { wc }, (rh3 - 0.5).abs() * 1.4);
        let base = scale3(base, 0.94 + 0.12 * rh);
        let mut col = scale3(base, (1.0 + 0.07 * streak) * (1.0 - 0.07 * lines));
        col = scale3(
            col,
            1.0 - 0.1 * (1.0 - smoothstep(0.0, 0.035, e_end - 0.0015)),
        );
        col = scale3(
            col,
            1.0 - 0.05 * (1.0 - smoothstep(0.0, 0.02, e_side - 0.004)),
        );
        let nx = if e_end < 0.1 {
            (e_end - 0.03).abs()
        } else {
            jd(p[0] - 0.25, 0.5)
        };
        let ny = (ly - 0.045).abs().min((ly - (PW - 0.045)).abs());
        let nd = length2([nx, ny]);
        let nail = 1.0 - aa(0.0042, nd, px);
        col = scale3(col, 1.0 - 0.1 * (1.0 - smoothstep(0.004, 0.018, nd)));
        col = mix3(
            col,
            scale3(
                lin([0.46, 0.45, 0.44]),
                0.85 + 0.3 * smoothstep(0.0042, 0.0, nd),
            ),
            nail,
        );
        col = mix3(col, lin([0.20, 0.15, 0.11]), gap);
        h += (0.0007 * (1.0 - ((ly - PW * 0.5) / (PW * 0.5)).powf(2.0)) + 0.00011 * streak
            - 0.00006 * lines)
            * (1.0 - gap);
        s.alb = col;
        s.h = h - nail * 0.0003;
        s.rough = mix(
            mix(0.64 + 0.05 * streak + 0.05 * lines, 0.42, nail),
            0.92,
            gap,
        );
        s.metal = nail * 0.75;
        s.cav = mix(1.0, 0.4, gap);
    }
}

/// ≡ texlib.js L349-377.
#[derive(Debug, Clone, Copy)]
pub struct MetalPanel;

impl SurfProgram for MetalPanel {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L350
        f[0] = fb(ctx.uv, [20, 2], 4, 0.55, 3);
        f[1] = fb(ctx.uv, [4, 4], 4, 0.5, 5);
        f[2] = fb(ctx.uv, [40, 40], 4, 0.55, 7);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L351-376
        let p = ctx.p;
        let px = ctx.px;
        let cell = [(p[0] / 1.0).floor() as i32, (p[1] / 2.0).floor() as i32];
        let lp = [
            p[0] - (cell[0] as f32 * 1.0 + 0.5),
            p[1] - (cell[1] as f32 * 2.0 + 1.0),
        ];
        let e = -sd_rb(lp, [0.5, 1.0], 0.015);
        let pr = edge_prof(e, 0.0035, 0.01, 0.0032, 0.006, px);
        let in_s = pr[1];
        let rib = (-((lp[1].abs() - 0.333) / 0.022).powf(2.0)).exp() * smoothstep(0.03, 0.09, e);
        let ry = -0.9 + 0.2 * clampf(((lp[1] + 1.0) / 0.2).floor(), 0.0, 9.0);
        let d1 = length2([lp[0].abs() - 0.465, lp[1] - ry]);
        let rx = -0.4 + 0.2 * clampf(((lp[0] + 0.5) / 0.2).floor(), 0.0, 4.0);
        let d2 = length2([lp[0] - rx, lp[1].abs() - 0.965]);
        let rd = d1.min(d2);
        let rv = 1.0 - aa(0.0065, rd, px);
        let dome = (1.0 - (rd / 0.0065).powf(2.0)).max(0.0).sqrt();
        let (streak, mott) = (n[0], n[1]);
        let wn = n[2] * 0.5 + 0.5;
        let edge_p = 1.0 - smoothstep(0.004, 0.03, e);
        let mut wear = smoothstep(0.7, 0.78, wn + 0.3 * edge_p * (1.0 - in_s)) * (1.0 - in_s);
        wear = wear.max(rv * smoothstep(0.55, 0.9, dome) * smoothstep(0.35, 0.6, wn));
        let col = v3(0.81 * (1.0 + 0.035 * streak + 0.025 * mott) * (1.0 - 0.08 * edge_p));
        let col = mix3(col, v3(0.93), wear * 0.7);
        s.alb = mix3(col, v3(0.36), in_s);
        s.h = pr[0] + 0.0022 * rib + rv * 0.0025 * dome + 0.00008 * mott - wear * 0.00005;
        s.rough = mix(mix(0.36 + 0.04 * streak, 0.42, wear), 0.7, in_s);
        s.metal = mix(mix(0.3, 0.45, wear), 0.2, in_s);
        s.cav = mix(1.0, 0.55, in_s) * (1.0 - 0.25 * rv * (1.0 - dome));
    }
}

/// ≡ texlib.js L379-398.
#[derive(Debug, Clone, Copy)]
pub struct Corrugated;

impl SurfProgram for Corrugated {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L380-381
        f[0] = fb(ctx.uv, [3, 4], 3, 0.5, 5);
        f[1] = fb(ctx.uv, [16, 2], 4, 0.55, 9);
        f[2] = fb(ctx.uv, [5, 5], 4, 0.5, 11);
        f[3] = fb(ctx.uv, [8, 40], 4, 0.55, 13);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L382-397
        let p = ctx.p;
        let x = glsl_mod(p[0] + 0.15, 0.3) - 0.15;
        let a = x.abs();
        let t = clampf((a - 0.042) / 0.066, 0.0, 1.0);
        let prof = mix(t, t * t * (3.0 - 2.0 * t), 0.65);
        let (dent, streak, mott) = (n[0], n[1], n[2]);
        let wn = n[3] * 0.5 + 0.5;
        let crest_edge = (-((a - 0.045) / 0.007).powf(2.0)).exp();
        let trough_edge = (-((a - 0.105) / 0.009).powf(2.0)).exp();
        let wear = crest_edge * smoothstep(0.52, 0.62, wn);
        let col = v3(0.81
            * (1.0 + 0.03 * streak + 0.025 * mott)
            * (1.0 - 0.07 * smoothstep(0.1, 0.15, a)));
        s.alb = mix3(col, v3(0.95), wear * 0.55);
        s.h = -0.03 * prof + 0.0006 * dent - wear * 0.00005;
        s.rough = mix(0.5 + 0.05 * streak, 0.45, wear);
        s.metal = mix(0.25, 0.4, wear);
        s.cav = 1.0 - 0.18 * trough_edge;
    }
}

/// ≡ texlib.js L400-420.
#[derive(Debug, Clone, Copy)]
pub struct Hazard;

impl SurfProgram for Hazard {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L401
        f[0] = fb(ctx.uv, [5, 5], 5, 0.55, 5);
        f[1] = fb(ctx.uv, [40, 40], 3, 0.5, 9);
        f[2] = fb(ctx.uv, [4, 80], 3, 0.5, 15);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L402-419
        let p = ctx.p;
        let px = ctx.px;
        const PER: f32 = 1.0 / 3.0;
        let q = fract((p[0] + p[1]) / PER);
        let d_y = if q < 0.5 {
            q.min(0.5 - q)
        } else {
            -((q - 0.5).min(1.0 - q))
        };
        // GLSL literal kept verbatim; same f32 bits as FRAC_1_SQRT_2.
        #[allow(clippy::approx_constant)]
        let dm = d_y * PER * 0.70710678;
        let yel = smoothstep(-px * 0.75, px * 0.75, dm);
        let y = lin([0.96, 0.77, 0.16]);
        let k = lin([0.14, 0.14, 0.15]);
        let n1 = n[0] * 0.5 + 0.5;
        let n2 = n[1] * 0.5 + 0.5;
        let scuff = n[2] * 0.5 + 0.5;
        let edge_w = 1.0 - smoothstep(0.0, 0.012, dm.abs());
        // L412: paint worn thin, broad and soft.
        let thin = smoothstep(0.5, 0.75, n1 + 0.15 * (n2 - 0.5));
        // L413: worn through, rare.
        let wear = smoothstep(0.72, 0.8, n1 + 0.35 * (n2 - 0.5) + 0.05 * edge_w);
        let paint = mix3(k, y, yel);
        let paint = scale3(
            paint,
            (1.0 - 0.05 * (n1 - 0.5)) * (1.0 - 0.06 * smoothstep(0.55, 0.8, scuff)),
        );
        let under = scale3(lin([0.63, 0.62, 0.6]), 1.0 + 0.06 * (n2 - 0.5));
        let paint = mix3(paint, mix3(paint, under, 0.3), thin * (0.2 + 0.25 * yel));
        s.alb = mix3(paint, under, wear * 0.85);
        s.h = 0.0003 * (1.0 - wear) + 0.00005 * n2;
        s.rough = mix(0.52 + 0.08 * scuff, 0.86, wear);
    }
}

/// ≡ texlib.js L422-447.
#[derive(Debug, Clone, Copy)]
pub struct Grate;

impl SurfProgram for Grate {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L423
        f[0] = fb(ctx.uv, [6, 6], 4, 0.5, 3);
        f[1] = fb(ctx.uv, [12, 12], 3, 0.5, 9);
        w[0] = wo(ctx.uv, [30, 30], 1.0, 21);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L424-446
        let p = ctx.p;
        let px = ctx.px;
        let db = jd(p[1], 0.075);
        let dc = jd(p[0], 0.075);
        let bb = 1.0 - aa(0.01, db, px);
        let cb = 1.0 - aa(0.007, dc, px);
        let hb = -0.0026 * clamp01(db / 0.01).powf(2.0)
            - 0.0005 * (0.5 + 0.5 * (TAU * p[0] / 0.0125).cos()) * smoothstep(0.0075, 0.0, db);
        let hc = -0.003 - 0.002 * clamp01(dc / 0.007).powf(2.0);
        let h = mix(-0.009, hb, bb).max(mix(-0.009, hc, cb));
        let spang = (c[0][2] - 0.5) * smoothstep(0.0, 0.08, c[0][1] - c[0][0]);
        let mott = n[0];
        let steel = scale3([0.53, 0.55, 0.57], 1.0 + 0.07 * spang + 0.04 * mott);
        let top = bb * (1.0 - smoothstep(0.0, 0.01, db));
        let polish = top * smoothstep(0.45, 0.7, n[1] * 0.5 + 0.75);
        let steel = mix3(steel, [0.64, 0.65, 0.66], polish * 0.7);
        let junction = cb * (1.0 - bb) * (1.0 - smoothstep(0.01, 0.02, db));
        s.alb = scale3(steel, 1.0 - 0.18 * junction);
        s.a = bb.max(cb);
        s.h = h;
        s.rough = mix(0.46 + 0.05 * mott, 0.3, polish);
        s.metal = 0.85;
        s.cav = 1.0 - 0.35 * junction;
    }
}

/// ≡ texlib.js L449-475.
#[derive(Debug, Clone, Copy)]
pub struct Brick;

impl SurfProgram for Brick {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L450-451
        f[0] = fb(ctx.uv, [80, 80], 3, 0.5, 11);
        f[1] = fb(ctx.uv, [6, 6], 4, 0.5, 13);
        f[2] = fb(ctx.uv, [128, 128], 2, 0.5, 19);
        w[0] = wo(ctx.uv, [120, 120], 1.0, 17);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L452-474 (`col` renamed `col_i` to free the colour name)
        let p = ctx.p;
        let px = ctx.px;
        let row = (p[1] / 0.1).floor() as i32;
        let ly = p[1] - row as f32 * 0.1;
        let bx = p[0] - if row % 2 == 1 { 0.125 } else { 0.0 };
        let col_i = (bx / 0.25).floor() as i32;
        let lx = bx - col_i as f32 * 0.25;
        let bid = wrp([col_i, row], [8, 20]);
        let lp = [lx - 0.125, ly - 0.05];
        let e = -sd_rb(lp, [0.125, 0.05], 0.007);
        let pr = edge_prof(e, 0.005, 0.007, 0.0035, 0.0065, px);
        let in_m = pr[1];
        let hid = hf(bid, 3);
        let hid2 = hf(bid, 5);
        let (sand, mott) = (n[0], n[1]);
        let pit = step(0.9, c[0][2]) * (1.0 - aa(0.0022, c[0][0] * (2.0 / 120.0), px));
        let bulge = (1.0 - (lp[0] / 0.125).powf(2.0)) * (1.0 - (lp[1] / 0.05).powf(2.0));
        let cb = v3(0.79
            * (1.0 + 0.1 * (hid - 0.5))
            * (1.0 + 0.025 * sand + 0.02 * mott)
            * (1.0 - 0.1 * pit));
        let cb = scale3(
            cb,
            1.0 - 0.04 * clampf(1.0 - (e - 0.005) / 0.01, 0.0, 1.0) * (1.0 - in_m),
        );
        let mort = v3(0.9 * (1.0 + 0.05 * n[2]));
        s.alb = mix3(cb, mort, in_m);
        s.h = pr[0]
            + (0.0005 * bulge * hid2 + 0.00012 * sand - 0.0008 * pit) * (1.0 - in_m)
            + in_m * 0.0008 * (clamp01(e / 0.005).powf(2.0) - 1.0);
        s.rough = mix(0.82 + 0.05 * sand, 0.95, in_m);
        s.cav = mix(1.0, 0.72, in_m);
    }
}

/// ≡ texlib.js L477-488.
#[derive(Debug, Clone, Copy)]
pub struct Rubber;

impl SurfProgram for Rubber {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L478
        f[0] = fb(ctx.uv, [5, 5], 4, 0.5, 71);
        w[0] = wo(ctx.uv, [240, 240], 1.0, 61);
    }

    fn surf(&self, _ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L479-487
        let wc = c[0];
        let gr = smoothstep(0.04, 0.18, wc[1] - wc[0]);
        let tone = mix(0.78 + 0.08 * fract(wc[2] * 3.71), 0.97, step(0.95, wc[2]));
        let big = n[0];
        s.alb = v3(mix(0.7, tone, gr) * (1.0 + 0.03 * big));
        s.h = 0.00055 * gr * (1.0 - 0.8 * wc[0] * wc[0]) - 0.0004 * (1.0 - gr) + 0.00008 * big;
        s.rough = 0.9 - 0.04 * gr;
    }
}

/// ≡ texlib.js L490-510.
#[derive(Debug, Clone, Copy)]
pub struct GlassTile;

impl SurfProgram for GlassTile {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], _w: &mut [WReq; 2]) {
        // L491
        f[0] = fb(ctx.uv, [96, 96], 2, 0.5, 7);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], _c: &[[f32; 4]; 2], s: &mut Surf) {
        // L492-509
        let p = ctx.p;
        let px = ctx.px;
        let cell = [(p[0] / 0.1).floor() as i32, (p[1] / 0.1).floor() as i32];
        let cw = wrp(cell, [12, 12]);
        let lp = [
            p[0] - (cell[0] as f32 * 0.1 + 0.05),
            p[1] - (cell[1] as f32 * 0.1 + 0.05),
        ];
        let e = -sd_rb(lp, [0.05, 0.05], 0.01);
        let pr = edge_prof(e, 0.002, 0.008, 0.002, 0.0036, px);
        let in_g = pr[1];
        let hid = hf(cw, 3);
        let hid2 = hf(cw, 5);
        let th = hf2(cw, 9);
        let tilt = [th[0] - 0.5, th[1] - 0.5];
        let dome = 1.0 - dot2(lp, lp) / 0.005;
        let inner = (-((e - 0.016) / 0.004).powf(2.0)).exp();
        let col = v3(0.8 * (1.0 + 0.09 * (hid - 0.5)) * (1.0 + 0.04 * inner));
        let grout = v3(0.7 * (1.0 + 0.05 * n[0]));
        s.alb = mix3(col, grout, in_g);
        s.h = pr[0] + (0.0007 * dome + dot2(tilt, lp) * 0.012 + 0.0001 * inner) * (1.0 - in_g);
        s.rough = mix(0.1 + 0.08 * hid2, 0.85, in_g);
        s.cav = mix(1.0, 0.65, in_g);
    }
}

/// ≡ texlib.js L518-613.
#[derive(Debug, Clone, Copy)]
pub struct Planks;

impl SurfProgram for Planks {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L519-524
        let p = ctx.p;
        let uv = ctx.uv;
        let row = (p[1] / 0.14).floor() as i32;
        let ly = p[1] - row as f32 * 0.14;
        const JS_A: [i32; 16] = [0, 2, 3, 1, 2, 0, 1, 3, 0, 2, 1, 3, 0, 1, 3, 2];
        let rn = hf([row, 0], 17);
        let j1 = 0.28 + 0.56 * JS_A[row as usize] as f32;
        let seg = if rn >= 0.8 && glsl_mod(p[0] - j1, 2.24) >= 1.12 {
            1
        } else {
            0
        };
        let bo = row as f32 * 6.37 + seg as f32 * 17.3;
        f[0] = Req {
            x: [uv[0] * 10.0, ly * 40.0 + bo],
            per: [10, 4096],
            oct: 3,
            gain: 0.55,
            seed: 211,
        };
        f[1] = fb(uv, [5, 5], 4, 0.5, 223);
        f[2] = Req {
            x: [uv[0] * 4.0, bo * 0.37],
            per: [4, 4096],
            oct: 3,
            gain: 0.5,
            seed: 227,
        };
        f[3] = fb(uv, [48, 48], 2, 0.5, 229);
        w[0] = wo(uv, [4, 16], 1.0, 233);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 95-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L525-612
        let p = ctx.p;
        let px = ctx.px;
        const PW: f32 = 0.14;
        const GAP: f32 = 0.0045;
        const JS: f32 = 0.56;
        let row = (p[1] / PW).floor() as i32;
        let ly = p[1] - row as f32 * PW;
        const JS_A: [i32; 16] = [0, 2, 3, 1, 2, 0, 1, 3, 0, 2, 1, 3, 0, 1, 3, 2];
        let rn = hf([row, 0], 17);
        let nj = if rn < 0.14 {
            0
        } else if rn < 0.8 {
            1
        } else {
            2
        };
        let j1 = 0.28 + JS * JS_A[row as usize] as f32;
        let seg = if nj == 2 && glsl_mod(p[0] - j1, 2.24) >= 1.12 {
            1
        } else {
            0
        };
        let bid = [row, seg];
        let b1 = hf(bid, 3);
        let b2 = hf(bid, 5);
        let b3 = hf(bid, 7);
        let b4 = hf(bid, 11);
        let b5 = hf(bid, 13);
        let e_e = if nj == 0 {
            1.0
        } else {
            jd(p[0] - j1, if nj == 2 { 1.12 } else { 2.24 })
        };
        let e_s = ly.min(PW - ly);
        let ps = edge_prof(e_s, GAP, 0.0045, 0.0018, 0.022, px);
        let pe = edge_prof(e_e, 0.0014, 0.003, 0.0012, 0.016, px);
        let gap = ps[1].max(pe[1]);
        let q = clampf((ly - PW * 0.5) / (PW * 0.5 - GAP), -1.0, 1.0);
        let crown = mix(-0.0005, 0.0009, b2) * (1.0 - q * q) + (b3 - 0.5) * 0.0007 * q;
        let (streak, warp, fib) = (n[0], n[2], n[3]);
        let patch_n = n[1] * 0.5 + 0.5;
        let kxc = (if nj == 2 {
            j1 + 1.12 * seg as f32 + 0.56
        } else {
            j1 + 1.12
        }) + (b5 * 4.0 - 0.5) * 0.7;
        let kd = [
            jd(p[0] - kxc, 2.24) / 0.024,
            (ly - PW * (0.3 + 0.4 * b1)) / 0.012,
        ];
        let kr = if b5 < 0.25 { length2(kd) } else { 99.0 };
        let knot = 1.0 - smoothstep(0.75, 1.0 + px / 0.012, kr);
        let flat_sawn = step(0.4, b4);
        let rc = q * (1.6 + 1.4 * b1)
            + (0.9 + 1.4 * flat_sawn) * warp
            + 3.0 * b3
            + 0.9 * (-0.25 * kr * kr).exp();
        let late = smoothstep(0.7, 0.95, 0.5 + 0.5 * (TAU * rc).cos()) * mix(0.45, 1.0, flat_sawn);
        let wa = lin([0.68, 0.56, 0.46]);
        let wb = lin([0.59, 0.47, 0.39]);
        let wc = lin([0.74, 0.62, 0.51]);
        let wd = lin([0.64, 0.56, 0.49]);
        let mut wood = scale3(
            mix3(mix3(wa, wb, b1), mix3(wc, wd, b2), step(0.55, b3)),
            0.8 + 0.34 * b4,
        );
        let fresh = b5 > 0.94;
        if fresh {
            wood = mul3(wood, [1.08, 0.96, 0.84]);
        }
        wood = scale3(
            wood,
            (1.0 + 0.12 * streak) * (1.0 - 0.12 * late) * (1.0 + 0.03 * fib),
        );
        wood = scale3(
            wood,
            1.0 - 0.26 * (1.0 - smoothstep(0.0014, 0.032, e_e)) * step(0.5, nj as f32),
        );
        wood = mix3(wood, lin([0.3, 0.2, 0.13]), knot * 0.85);
        let stain = (1.0 - smoothstep(0.15, 0.45, c[0][0] + 0.15 * n[1])) * step(0.7, c[0][2]);
        wood = scale3(wood, 1.0 - 0.1 * stain);
        let edge_w = 1.0 - smoothstep(0.0, 0.016, e_s - GAP);
        let end_w = if nj == 0 {
            0.0
        } else {
            1.0 - smoothstep(0.0, 0.035, e_e - 0.0014)
        };
        let mut silver =
            (0.08 + 0.45 * b2 * b2 * b3) * (0.7 + 0.6 * patch_n) + 0.38 * edge_w + 0.3 * end_w;
        silver = if fresh {
            0.0
        } else {
            clampf(
                silver
                    * (0.55 + 0.9 * smoothstep(-0.5, 0.7, streak))
                    * (0.85 + 0.3 * (fib * 0.5 + 0.5)),
                0.0,
                0.75,
            )
        };
        let mut grey = lin([0.6, 0.59, 0.56]);
        grey = scale3(
            grey,
            (1.0 + 0.14 * streak) * (1.0 - 0.2 * late) * (1.0 - 0.12 * stain),
        );
        let mut chk: f32 = 0.0;
        for k in 0..2i32 {
            let ck = [row * 2 + seg, k];
            let h1 = hf(ck, 41);
            let h2 = hf(ck, 43);
            let h3 = hf(ck, 47);
            if h1 > 0.5 {
                continue;
            }
            let l_len = 0.04 + 0.22 * h3;
            let xc = if k == 0 && nj > 0 {
                j1 + (if h2 > 0.5 { 1.0 } else { -1.0 }) * (0.02 + l_len)
            } else {
                h2 * 2.24
            };
            let yc = GAP + 0.02 + h3 * (PW - 2.0 * GAP - 0.04);
            let taper = clampf(1.0 - jd(p[0] - xc, 2.24) / l_len, 0.0, 1.0);
            let d = (ly - yc - 0.003 * (p[0] * 23.0 + h1 * 40.0).sin()).abs();
            let wdt = 0.0003 + 0.0011 * taper;
            chk = chk.max((1.0 - smoothstep(wdt, wdt + px, d)) * step(0.001, taper));
        }
        chk *= 1.0 - gap;
        let jx = jd(p[0] - 0.28, JS);
        let at_j = if nj == 0 { 0.0 } else { step(e_e, 0.08) };
        let sx = if at_j > 0.5 { (e_e - 0.03).abs() } else { jx };
        let sy = (ly - GAP - 0.026).abs().min((PW - GAP - 0.026 - ly).abs());
        let s_d = length2([sx, sy]);
        let screw = 1.0 - aa(0.0042, s_d, px);
        let halo = (1.0 - smoothstep(0.004, 0.02, s_d)) * (1.0 - screw);
        let sh = hf(
            wrp(
                [
                    (p[0] / 0.07).floor() as i32,
                    row * 2 + (ly > PW * 0.5) as i32,
                ],
                [32, 32],
            ),
            53,
        );
        let steel = scale3(
            if sh < 0.82 {
                lin([0.62, 0.62, 0.6])
            } else {
                lin([0.3, 0.25, 0.21])
            },
            0.78 + 0.35 * smoothstep(0.0042, 0.0, s_d),
        );
        let joist = 1.0 - smoothstep(0.018, 0.026, jx);
        let gap_c = mix3(
            lin([0.03, 0.028, 0.025]),
            lin([0.16, 0.12, 0.09]),
            (joist * ps[1]).max(pe[1]) * 0.85,
        );
        let mut col = mix3(wood, grey, silver);
        col = scale3(col, (1.0 - 0.45 * halo) * (1.0 - 0.55 * chk));
        col = mix3(col, steel, screw);
        col = mix3(col, gap_c, gap);
        let mut h = ps[0].min(pe[0]);
        h += (crown + 0.00011 * streak + 0.00017 * late + 0.00006 * fib
            - 0.0001 * knot
            - 0.0024 * chk)
            * (1.0 - gap);
        h = mix(
            h,
            -0.0004 + 0.00025 * (1.0 - (s_d / 0.0042).powf(2.0)).max(0.0).sqrt(),
            screw,
        );
        s.alb = col;
        s.h = h;
        s.rough = mix(
            mix(
                0.68 + 0.08 * (fib * 0.5 + 0.5) + 0.05 * late + 0.1 * silver - 0.05 * knot,
                0.36,
                screw,
            ),
            0.95,
            gap,
        );
        s.metal = screw * (if sh < 0.82 { 0.85 } else { 0.35 });
        s.cav = mix(1.0, 0.3, gap) * (1.0 - 0.35 * chk) * (1.0 - 0.25 * halo);
    }
}

/// ≡ texlib.js L615-689.
#[derive(Debug, Clone, Copy)]
pub struct HullPaint;

impl SurfProgram for HullPaint {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L616
        f[0] = fb(ctx.uv, [5, 5], 4, 0.5, 301);
        f[1] = fb(ctx.uv, [128, 128], 2, 0.5, 307);
        f[2] = fb(ctx.uv, [32, 4], 3, 0.5, 311);
        f[3] = fb(ctx.uv, [16, 16], 3, 0.5, 313);
        w[0] = wo(ctx.uv, [32, 32], 0.9, 317);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 74-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L617-688
        let p = ctx.p;
        let px = ctx.px;
        let x = p[0];
        let y = p[1];
        let d_w = y.min(3.2 - y);
        let d_b = if y < 1.6 {
            x.min(3.2 - x)
        } else {
            (x - 1.6).abs()
        };
        let bead = (-d_w * d_w / 0.00006)
            .exp()
            .max((-d_b * d_b / 0.00006).exp());
        let toe = (-((d_w - 0.011) / 0.004).powf(2.0))
            .exp()
            .max(-((d_b - 0.011) / 0.004).powf(2.0))
            .exp();
        let d_l = (y - 1.6).abs();
        let seam_l = 1.0 - aa(0.001, d_l, px);
        const RP: f32 = 3.2 / 36.0;
        let r_d = length2([(glsl_mod(x, RP) - RP * 0.5).abs(), (d_l - 0.036).abs()]);
        let riv = 1.0 - aa(0.0105, r_d, px);
        let dome = (1.0 - (r_d / 0.0105).powf(2.0)).max(0.0).sqrt();
        let dish = ((PI * fract(x / 0.4)).sin() * (PI * fract(y / 0.8)).sin())
            .max(0.0)
            .powf(0.9);
        let wc = c[0];
        let cp = step(
            1.0 - (0.008 + 0.025 * smoothstep(0.1, 0.7, n[3] * 0.5 + 0.5)),
            wc[2],
        );
        let cr = 0.002 + 0.006 * fract(wc[2] * 37.1);
        let cdist = wc[0] * 0.1 + cr * 0.9 * n[1] + 0.002 * n[2];
        let chip = cp * (1.0 - aa(cr, cdist, px));
        let core = cp * (1.0 - aa(cr * 0.55, cdist, px));
        let rsty = step(0.45, fract(wc[2] * 71.3));
        let mut weep: f32 = 0.0;
        let mut spot: f32 = 0.0;
        let ci = (x / 0.1).floor() as i32;
        for it in 0..6i32 {
            let dc = (it >> 1) - 1;
            let k = it & 1;
            let cid = wrp([ci + dc, k], [32, 2]);
            let h1 = hf(cid, 61);
            let h2 = hf(cid, 67);
            let h3 = hf(cid, 71);
            let h4 = hf(cid, 73);
            if h1 > 0.2 {
                continue;
            }
            let mut xs = ((ci + dc) as f32 + 0.15 + 0.7 * h2) * 0.1;
            let ys;
            if h3 < 0.35 {
                ys = 3.194;
            } else if h3 < 0.7 {
                xs = ((xs / RP).floor() + 0.5) * RP;
                ys = 1.6 - 0.036 - 0.009;
            } else {
                ys = 0.3 + 2.7 * fract(h4 * 7.3);
            }
            let l_len = (0.12 + 0.7 * h4 * h4).min(ys - 0.03);
            let dy = ys - y;
            if dy < -0.02 || dy > l_len {
                continue;
            }
            let t = dy.max(0.0) / l_len;
            let xm = xs + 0.005 * (dy * 7.0 + h2 * 30.0).sin() + 0.006 * n[2] * t;
            let w = (0.01 + 0.018 * h2) * (0.5 + 1.5 * t);
            let lat = 1.0 - smoothstep(w * 0.1, w, (x - xm).abs() * (1.0 + 0.35 * n[1]));
            weep = weep.max(
                lat * lat
                    * (1.0 - t).powf(1.2)
                    * smoothstep(-0.015, 0.01, dy)
                    * (0.55 + 0.45 * h4)
                    * (0.75 + 0.25 * n[1]),
            );
            if h3 >= 0.7 {
                spot = spot.max(
                    1.0 - aa(
                        0.003 + 0.004 * h2,
                        length2([x - xs, y - ys]) + 0.003 * n[1],
                        px,
                    ),
                );
            }
        }
        let pc = [(p[0] / 0.8).floor() as i32, (p[1] / 0.4).floor() as i32];
        let pcw = wrp(pc, [4, 8]);
        let ph2 = hf2(pcw, 89);
        let pl = [
            p[0] - (pc[0] as f32 * 0.8 + 0.4) - (ph2[0] - 0.5) * 0.2,
            p[1] - (pc[1] as f32 * 0.4 + 0.2) - (ph2[1] - 0.5) * 0.08,
        ];
        let p_d = sd_rb(
            pl,
            [0.1 + 0.16 * hf(pcw, 83), 0.05 + 0.08 * hf(pcw, 87)],
            0.025,
        ) + 0.01 * n[1]
            + 0.012 * n[2];
        let patch_m = step(hf(pcw, 81), 0.07) * (1.0 - aa(0.0, p_d, px));
        let (mott, stip) = (n[0], n[1]);
        let chalk = smoothstep(0.15, 0.75, mott);
        let mut paint = 0.8 * (1.0 + 0.04 * mott + 0.012 * stip) * (1.0 + 0.04 * chalk);
        paint *= mix(1.0, 0.94 + 0.12 * hf(pcw, 91), patch_m);
        paint *= 1.0 + 0.12 * (bead * 0.7 + riv * smoothstep(0.55, 1.0, dome));
        paint *= 1.0 - 0.14 * toe;
        weep *= 1.0 - patch_m;
        let chip_t = chip.max(spot) * (1.0 - patch_m);
        let rust = scale3(lin([0.56, 0.33, 0.19]), 0.85 + 0.3 * n[2]);
        let chip_c = mix3(
            lin([0.36, 0.34, 0.33]),
            if rsty > 0.5 {
                lin([0.33, 0.17, 0.09])
            } else {
                lin([0.27, 0.27, 0.28])
            },
            core,
        );
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        own = mix3(own, rust, weep * 0.62);
        cov *= 1.0 - weep * 0.62;
        own = mix3(own, chip_c, chip_t);
        cov *= 1.0 - chip_t;
        s.alb = own;
        s.a = cov * paint;
        s.h = 0.0016 * bead - 0.0002 * toe + 0.0028 * riv * dome
            - 0.0018 * dish
            - 0.0003 * seam_l
            - 0.00025 * chip_t
            + 0.00003 * stip
            + 0.0001 * patch_m;
        s.rough = mix(
            mix(
                mix(
                    0.44 + 0.12 * chalk + 0.03 * stip + 0.06 * bead,
                    0.3,
                    patch_m,
                ),
                0.72,
                weep * 0.6,
            ),
            if core * (1.0 - rsty) > 0.5 {
                0.42
            } else {
                0.82
            },
            chip_t,
        );
        s.metal = chip_t * core * (1.0 - rsty) * 0.75;
        s.cav = (1.0 - 0.3 * seam_l) * (1.0 - 0.3 * riv * (1.0 - dome)) * (1.0 - 0.2 * toe);
    }
}

/// ≡ texlib.js L691-736.
#[derive(Debug, Clone, Copy)]
pub struct NonSlip;

impl SurfProgram for NonSlip {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L692
        f[0] = fb(ctx.uv, [5, 5], 4, 0.5, 401);
        f[1] = fb(ctx.uv, [64, 64], 2, 0.5, 409);
        f[2] = fb(ctx.uv, [12, 12], 3, 0.5, 419);
        w[0] = wo(ctx.uv, [260, 260], 1.0, 421);
        w[1] = wo(ctx.uv, [10, 10], 0.9, 431);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 44-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L693-735
        let p = ctx.p;
        let px = ctx.px;
        let d_w = jd(p[0], 2.4).min(jd(p[1], 2.4));
        let bead = (-d_w * d_w / 0.00003).exp();
        let pu = [p[0] / 2.4, p[1] / 2.4];
        let sag = (PI * pu[0]).sin() * (PI * pu[1]).sin();
        let g = c[0];
        let gran = smoothstep(0.02, 0.22, g[1] - g[0]) * (1.0 - g[0] * g[0]);
        let gt = fract(g[2] * 7.7) - 0.5;
        let pc = [(p[0] / 1.2).floor() as i32, (p[1] / 1.2).floor() as i32];
        let lp = [
            p[0] - (pc[0] as f32 * 1.2 + 0.6),
            p[1] - (pc[1] as f32 * 1.2 + 0.6),
        ];
        let r = length2(lp);
        let ph = hf(wrp(pc, [2, 2]), 7);
        let plate = 1.0 - aa(0.068, r, px);
        let rim = (-((r - 0.064) / 0.004).powf(2.0)).exp();
        let lobe = length2([lp[0] - 0.021, lp[1]])
            .min(length2([lp[0] + 0.021, lp[1]]))
            .min(length2([lp[0], lp[1] - 0.021]).min(length2([lp[0], lp[1] + 0.021])))
            - 0.019;
        let hole = 1.0 - aa(0.0, lobe.min(lp[0].abs().max(lp[1].abs()) - 0.02), px);
        let wear_ring = (1.0 - smoothstep(0.068, 0.1 + 0.02 * n[2], r)) * (1.0 - plate);
        let rust_halo = step(ph, 0.4)
            * (1.0 - smoothstep(0.07, 0.15 + 0.05 * n[2], r))
            * (1.0 - plate)
            * smoothstep(-0.3, 0.3, n[2]);
        let worn = smoothstep(0.4, 0.75, n[0] * 0.5 + 0.5 + 0.1 * n[1]);
        let scuff = step(0.8, c[1][2]) * (1.0 - smoothstep(0.03, 0.2, c[1][0] + 0.22 * n[2]));
        let seam_rust = (-d_w * d_w / 0.00018).exp() * smoothstep(0.62, 0.85, n[2] * 0.5 + 0.5);
        let grime = (1.0 - gran) * smoothstep(0.1, 0.6, g[0]);
        let mut paint = 0.8
            * (1.0 + 0.03 * n[0] + 0.03 * n[1])
            * (1.0 + 0.2 * gt * gran)
            * (1.0 - 0.1 * grime * (1.0 - worn))
            * (1.0 + 0.035 * worn)
            * (1.0 - 0.22 * scuff);
        paint *= 1.0 + 0.08 * bead;
        let galv = scale3(
            scale3(lin([0.47, 0.48, 0.49]), 0.8 + 0.2 * n[1]),
            1.0 - 0.3 * smoothstep(0.045, 0.068, r),
        );
        let rust = scale3(lin([0.42, 0.21, 0.1]), 0.85 + 0.3 * n[2]);
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        own = mix3(own, lin([0.36, 0.36, 0.37]), wear_ring * 0.45);
        cov *= 1.0 - wear_ring * 0.45;
        own = mix3(own, rust, rust_halo * 0.75);
        cov *= 1.0 - rust_halo * 0.75;
        own = mix3(own, rust, seam_rust * 0.7);
        cov *= 1.0 - seam_rust * 0.7;
        let mut pot_c = mix3(galv, v3(0.015), hole);
        pot_c = mix3(
            pot_c,
            scale3(rust, 0.8),
            step(ph, 0.4) * smoothstep(0.024, 0.06, r) * 0.45 * (1.0 - hole),
        );
        own = mix3(own, pot_c, plate);
        cov *= 1.0 - plate;
        let mut h =
            0.0009 * bead - 0.0012 * sag + 0.0006 * gran * (1.0 - 0.5 * worn) + 0.00006 * n[1];
        h = mix(h, 0.0003 + 0.0006 * rim - 0.012 * hole, plate);
        s.alb = own;
        s.a = cov * paint;
        s.h = h;
        s.rough = mix(
            mix(
                0.86 - 0.06 * gran - 0.09 * worn + 0.04 * n[1],
                0.66,
                seam_rust * 0.5,
            ),
            mix(0.5, 0.95, hole),
            plate,
        );
        s.metal = plate * (1.0 - hole) * 0.8 + wear_ring * 0.4;
        s.cav = mix(1.0, 0.15, hole * plate) * (1.0 - 0.2 * rim);
    }
}

/// ≡ texlib.js L738-766.
#[derive(Debug, Clone, Copy)]
pub struct GelCoat;

impl SurfProgram for GelCoat {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L739-740
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 501);
        f[1] = fb(ctx.uv, [40, 4], 3, 0.5, 503);
        f[2] = fb(ctx.uv, [3, 30], 3, 0.5, 509);
        f[3] = fb(ctx.uv, [16, 16], 3, 0.5, 511);
        w[0] = wo(ctx.uv, [110, 110], 1.0, 521);
        w[1] = wo(ctx.uv, [40, 40], 1.0, 523);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L741-765
        let p = ctx.p;
        let px = ctx.px;
        // GLSL keeps `cell` as a float vec2 here; `ivec2(cell)` truncates an
        // already-integer, non-negative value → `as i32` is exact.
        let cell = [(p[0] / 2.4).floor(), (p[1] / 0.8).floor()];
        let lp = [p[0] - (cell[0] * 2.4 + 1.2), p[1] - (cell[1] * 0.8 + 0.4)];
        let e = jd(p[1], 0.8).min(jd(p[0], 2.4) + 0.0006);
        let line = (-e * e / 0.0000035).exp()
            * (if jd(p[1], 0.8) < jd(p[0], 2.4) {
                1.0
            } else {
                0.7
            });
        let ridge = (-((e - 0.005) / 0.002).powf(2.0)).exp();
        let peel = smoothstep(0.0, 0.25, c[0][1] - c[0][0]) * (1.0 - c[0][0] * c[0][0]);
        let ox = smoothstep(0.1, 0.65, n[0]);
        let corner = 1.0 - smoothstep(0.03, 0.22, length2([lp[0].abs() - 1.2, lp[1].abs() - 0.4]));
        let ch = hf(wrp([cell[0] as i32, cell[1] as i32], [1, 3]), 5);
        let craze = (1.0 - aa(0.0005, (c[1][1] - c[1][0]) * 0.03, px))
            * corner
            * step(ch, 0.4)
            * step(0.001, e);
        let run = smoothstep(0.45, 0.85, n[1] * 0.5 + 0.5) * smoothstep(0.0, 0.5, n[3]);
        let scuff =
            smoothstep(0.74, 0.94, n[2] * 0.5 + 0.5) * smoothstep(0.6, 0.85, n[3] * 0.5 + 0.5);
        let paint = 0.8
            * (1.0 + 0.012 * n[3])
            * (1.0 + 0.025 * ox)
            * (1.0 - 0.14 * scuff)
            * (1.0 - 0.16 * line)
            * (1.0 - 0.3 * craze);
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        let ox_c = 0.06 * ox;
        own = mix3(own, scale3(lin([0.9, 0.87, 0.8]), 1.0 - 0.14 * scuff), ox_c);
        cov *= 1.0 - ox_c;
        let run_c = 0.1 * run;
        own = mix3(own, lin([0.62, 0.52, 0.38]), run_c);
        cov *= 1.0 - run_c;
        s.alb = own;
        s.a = cov * paint;
        s.h = -0.0006 * line + 0.00008 * ridge + 0.00009 * peel + 0.00006 * n[3]
            - 0.00025 * craze
            - 0.00002 * scuff;
        s.rough = 0.12 + 0.1 * ox + 0.06 * scuff + 0.25 * line + 0.2 * craze + 0.015 * n[3];
        s.cav = (1.0 - 0.3 * line) * (1.0 - 0.3 * craze);
    }
}

/// ≡ texlib.js L768-826.
#[derive(Debug, Clone, Copy)]
pub struct Yard;

impl SurfProgram for Yard {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L769-772
        let uv = ctx.uv;
        f[0] = fb(uv, [4, 4], 5, 0.55, 601);
        f[1] = fb(uv, [16, 16], 3, 0.5, 607);
        f[2] = fb(uv, [10, 96], 1, 0.5, 613);
        f[3] = fb(uv, [64, 64], 3, 0.5, 617);
        let wq = [
            uv[0]
                + 0.02
                    * ((TAU * (3.0 * uv[1] + 2.0 * uv[0])).sin()
                        + 0.5 * (TAU * (7.0 * uv[1] - 5.0 * uv[0]) + 1.3).sin()),
            uv[1]
                + 0.02
                    * ((TAU * (3.0 * uv[0] - 2.0 * uv[1]) + 0.7).sin()
                        + 0.5 * (TAU * (6.0 * uv[0] + 5.0 * uv[1]) + 2.1).sin()),
        ];
        w[0] = wo(wq, [4, 4], 0.85, 619);
        w[1] = wo(uv, [64, 64], 1.0, 631);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 57-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L773-825
        let p = ctx.p;
        let px = ctx.px;
        let d_e = jd(p[0], 3.2).min(jd(p[1], 3.2));
        let spall = 0.002 * smoothstep(0.25, 0.85, n[3] * 0.5 + 0.5);
        let pe = edge_prof(d_e, 0.0055, 0.004 + spall, 0.0016 + spall * 0.5, 0.004, px);
        let mott = n[0];
        let broom = n[2];
        let wc = c[0];
        let kind = wc[2];
        let d = wc[0] + 0.3 * n[1] + 0.07 * n[3];
        let r_s = 0.08 + 0.2 * fract(kind * 17.3);
        let away = smoothstep(0.35, 0.75, d_e);
        // L785: the oil terms are carried verbatim from the JS, where the
        // `oilK` gate is hard-wired to 0.0 (kept for a possible re-enable).
        let oil_k = 0.0;
        let oil = oil_k * (1.0 - smoothstep(r_s * 0.85, r_s, d));
        let oil_core = oil_k * (1.0 - smoothstep(r_s * 0.2, r_s * 0.8, d + 0.06 * n[3]));
        let ring_d = (wc[0] * 0.8 - (0.07 + 0.07 * fract(kind * 31.0))).abs();
        let rust = step(0.34, kind)
            * step(kind, 0.42)
            * ((-ring_d * ring_d / 0.0003).exp())
                .max(0.3 * (1.0 - smoothstep(0.0, 0.12, wc[0] * 0.8)))
            * smoothstep(-0.35, 0.3, n[1])
            * away;
        let cluster = step(0.46, kind)
            * step(kind, 0.6)
            * (1.0 - smoothstep(0.1, 0.75, wc[0] + 0.2 * n[1]))
            * away;
        let sc = c[1];
        let s_r = 0.0018
            + 0.004 * fract(sc[2] * 13.1).powf(3.0)
            + 0.006 * step(0.985, fract(sc[2] * 29.3));
        let spk = step(1.0 - 0.45 * cluster * smoothstep(-0.2, 0.3, n[1]), sc[2])
            * (1.0 - aa(s_r, sc[0] * 0.05 + 0.25 * s_r * n[3], px));
        let dust = cluster * smoothstep(-0.3, 0.5, n[1]) * 0.22;
        let kf = fract(kind * 7.0);
        let af = if kf < 0.55 {
            lin([0.42, 0.27, 0.22])
        } else if kf < 0.85 {
            lin([0.24, 0.28, 0.34])
        } else {
            lin([0.18, 0.18, 0.19])
        };
        let crack = (1.0 - aa(0.0006, (wc[1] - wc[0]) * 0.4, px))
            * step(0.55, fract(wc[2] * 5.3 + wc[3] * 2.7))
            * smoothstep(0.1, 0.4, mott)
            * step(0.02, d_e);
        let pc = [(p[0] / 1.6).floor() as i32, (p[1] / 1.6).floor() as i32];
        let pcw = wrp(pc, [2, 2]);
        let plh = hf2(pcw, 15);
        let pl = [
            p[0] - (pc[0] as f32 * 1.6 + 0.8) - (plh[0] - 0.5) * 0.5,
            p[1] - (pc[1] as f32 * 1.6 + 0.8) - (plh[1] - 0.5) * 0.5,
        ];
        let p_d = sd_rb(
            pl,
            [0.2 + 0.22 * hf(pcw, 17), 0.16 + 0.16 * hf(pcw, 19)],
            0.03,
        ) + 0.006 * n[3];
        let patch_m = step(hf(pcw, 13), 0.16) * (1.0 - aa(0.0, p_d, px));
        let patch_edge = step(hf(pcw, 13), 0.16) * (1.0 - aa(0.0015, p_d.abs(), px));
        let scuff = smoothstep(0.62, 0.92, n[1] * 0.5 + 0.5) * smoothstep(-0.1, 0.5, mott);
        let agg = step(0.8, fract(sc[2] * 5.3))
            * (1.0 - aa(0.004, sc[0] * 0.05 + 0.002 * n[3], px))
            * smoothstep(0.0, 0.6, mott * 0.5 + 0.5);
        let pore = step(0.9, fract(sc[3] * 7.9 + sc[2])) * (1.0 - aa(0.0018, sc[0] * 0.05, px));
        let mut paint = 0.8
            * (1.0 + 0.11 * mott + 0.045 * n[1] + 0.02 * broom + 0.05 * n[3])
            * (1.0 - 0.35 * pore);
        paint *= 1.0 - 0.1 * smoothstep(0.3, 0.9, -mott);
        paint *= mix(1.0, 0.92 + 0.04 * n[3], patch_m);
        paint *= 1.0 - 0.25 * patch_edge;
        paint *= 1.0 - 0.25 * oil;
        paint *= 1.0 - 0.14 * scuff;
        paint *= 1.0 - 0.45 * crack;
        paint *= 1.0 - 0.18 * (1.0 - smoothstep(0.007, 0.03, d_e)) * (1.0 - pe[1]);
        let rust_c = scale3(lin([0.5, 0.27, 0.13]), 0.85 + 0.3 * n[3]);
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        own = mix3(
            own,
            scale3(lin([0.46, 0.43, 0.39]), 0.8 + 0.4 * fract(sc[2] * 11.3)),
            agg * 0.6,
        );
        cov *= 1.0 - agg * 0.6;
        own = mix3(own, lin([0.1, 0.09, 0.08]), oil_core * 0.4);
        cov *= 1.0 - oil_core * 0.4;
        own = mix3(own, scale3(af, 1.3), dust);
        cov *= 1.0 - dust;
        own = mix3(own, rust_c, rust * 0.7);
        cov *= 1.0 - rust * 0.7;
        own = mix3(own, scale3(af, 0.85 + 0.3 * fract(sc[2] * 3.7)), spk);
        cov *= 1.0 - spk;
        own = mix3(
            own,
            scale3(lin([0.17, 0.165, 0.16]), 0.85 + 0.3 * n[3]),
            pe[1],
        );
        cov *= 1.0 - pe[1];
        s.alb = own;
        s.a = cov * paint;
        s.h = pe[0]
            + (0.00007 * broom + 0.0001 * n[3] + 0.0002 * spk + 0.00035 * patch_m
                - 0.0015 * crack
                - 0.0012 * pore
                + 0.00012 * agg)
                * (1.0 - pe[1])
            - 0.0003 * patch_edge;
        s.rough = mix(
            mix(
                mix(0.87 + 0.04 * n[3] - 0.05 * agg, 0.72, patch_m) - 0.2 * oil_core - 0.06 * oil,
                0.5,
                spk,
            ),
            0.62,
            pe[1],
        );
        s.cav = mix(1.0, 0.8, pe[1])
            * (1.0 - 0.4 * crack)
            * (1.0 - 0.2 * patch_edge)
            * (1.0 - 0.4 * pore);
    }
}

/// ≡ texlib.js L828-892.
#[derive(Debug, Clone, Copy)]
pub struct WeatherBoard;

impl SurfProgram for WeatherBoard {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L829-832
        let p = ctx.p;
        let uv = ctx.uv;
        let row = (p[1] / 0.15).floor() as i32;
        let ly = p[1] - row as f32 * 0.15;
        f[0] = Req {
            x: [uv[0] * 8.0, ly * 45.0 + row as f32 * 5.3],
            per: [8, 4096],
            oct: 3,
            gain: 0.5,
            seed: 701,
        };
        f[1] = fb(uv, [5, 5], 4, 0.5, 709);
        f[2] = fb(uv, [40, 16], 3, 0.5, 719);
        f[3] = fb(uv, [24, 24], 3, 0.5, 727);
        w[0] = wo(uv, [12, 64], 0.9, 733);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 62-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L833-891
        let p = ctx.p;
        let px = ctx.px;
        const BW: f32 = 0.15;
        let row = (p[1] / BW).floor() as i32;
        let ly = p[1] - row as f32 * BW;
        let t = ly / BW;
        let grain = n[0];
        let weath = smoothstep(0.0, 0.7, n[1]);
        let pn = n[2];
        let h = -0.013 * t.powf(1.15) - 0.0022 * (1.0 - smoothstep(0.0, 0.005, ly));
        const JSW: [i32; 16] = [1, 3, 0, 2, 3, 1, 2, 0, 1, 2, 3, 0, 2, 0, 3, 1];
        let jxp = 0.3 + 0.6 * JSW[row as usize] as f32;
        let e_j = if hf([row, 0], 19) < 0.55 {
            jd(p[0] - jxp, 2.4)
        } else {
            1.0
        };
        let pj = edge_prof(e_j, 0.0009, 0.002, 0.0008, 0.008, px);
        let nx = if e_j < 0.05 {
            (e_j - 0.022).abs()
        } else {
            jd(p[0] - 0.3, 0.6)
        };
        let nd = length2([nx, ly - 0.028]);
        let nail = 1.0 - aa(0.0035, nd, px);
        let wc = c[0];
        let pp = step(1.0 - (0.02 + 0.11 * weath) * (1.4 - t), wc[2]);
        let mut peel = pp
            * (1.0
                - aa(
                    0.0,
                    (wc[0] + 0.28 * pn - (0.3 + 0.35 * fract(wc[2] * 19.3))) * 0.0375,
                    px,
                ));
        let edge_p = (1.0 - smoothstep(0.0, 0.005 + 0.016 * weath, ly + 0.005 * pn))
            * step(0.64 - 0.3 * weath, pn * 0.5 + 0.5);
        let joint_p = (1.0 - smoothstep(0.0, 0.008 + 0.016 * weath, e_j + 0.004 * pn))
            * step(0.52, pn * 0.5 + 0.5);
        peel = peel.max(edge_p).max(joint_p) * (1.0 - nail);
        let pedge = (1.0 - smoothstep(0.0, 0.6, (peel - 0.5).abs() * 2.0)) * step(0.02, peel);
        let check_p = (1.0 - smoothstep(0.0, 0.05, grain.abs())) * smoothstep(0.35, 0.9, weath);
        let mut weep: f32 = 0.0;
        let sk = ((p[0] - 0.3) / 0.6 + 0.5).floor() as i32;
        let xs0 = 0.3 + 0.6 * sk as f32;
        for dr in 0..4i32 {
            let rr = row + dr;
            if rr > 15 {
                break;
            }
            let id = wrp([sk, rr], [4, 16]);
            let h1 = hf(id, 61);
            if h1 > 0.24 {
                continue;
            }
            let ys = rr as f32 * BW + 0.024;
            let l_len = (0.06 + 0.4 * hf(id, 67)).min(ys - 0.02);
            let dy = ys - p[1];
            if dy < -0.004 || dy > l_len {
                continue;
            }
            let tt = dy.max(0.0) / l_len;
            let xm = xs0 + 0.002 * (dy * 40.0 + h1 * 50.0).sin() + 0.002 * pn;
            let w = 0.007 * (1.0 + 1.4 * tt);
            let lat = 1.0 - smoothstep(0.0, w, (p[0] - xm).abs());
            weep = weep.max(lat * lat * (1.0 - tt).powf(1.2) * (0.6 + 0.4 * h1 / 0.24));
        }
        let mut paint = 0.8 * (1.0 + 0.025 * grain + 0.02 * n[3]) * (1.0 + 0.05 * weath);
        paint *= 1.0 - 0.06 * smoothstep(0.75, 1.0, t);
        paint *= 1.0 + 0.12 * pedge;
        paint *= 1.0 - 0.3 * check_p;
        paint *= 1.0 - 0.15 * nail;
        let gl = smoothstep(
            0.55,
            0.95,
            0.5 + 0.5 * (TAU * (ly * 38.0 + 2.2 * grain)).cos(),
        );
        let mut bare = lin([0.61, 0.59, 0.55]);
        bare = scale3(
            bare,
            (1.0 + 0.1 * grain) * (1.0 - 0.28 * gl) * (1.0 + 0.06 * n[3]),
        );
        bare = scale3(
            bare,
            1.0 - 0.35 * (1.0 - smoothstep(0.0, 0.35, peel)) * step(0.02, peel),
        );
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        own = mix3(own, bare, peel);
        cov *= 1.0 - peel;
        own = mix3(own, lin([0.52, 0.28, 0.13]), weep * 0.6);
        cov *= 1.0 - weep * 0.6;
        own = mix3(own, v3(0.05), pj[1]);
        cov *= 1.0 - pj[1];
        s.alb = own;
        s.a = cov * paint;
        s.h = h.min(pj[0]) + 0.00006 * grain * (1.0 + peel) - 0.00012 * peel - 0.0004 * nail;
        s.rough = mix(
            mix(0.5 + 0.18 * weath + 0.03 * n[3], 0.82, peel),
            0.75,
            weep * 0.5,
        );
        s.cav = (1.0 - 0.45 * smoothstep(0.8, 1.0, t)) * mix(1.0, 0.4, pj[1]);
    }
}

/// ≡ texlib.js L894-922.
#[derive(Debug, Clone, Copy)]
pub struct Render;

impl SurfProgram for Render {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L895-898
        let uv = ctx.uv;
        f[0] = fb(uv, [4, 4], 4, 0.55, 801);
        f[1] = fb(uv, [12, 12], 3, 0.5, 809);
        f[2] = fb(uv, [48, 48], 2, 0.5, 811);
        let wq = [
            uv[0]
                + 0.018
                    * ((TAU * (4.0 * uv[1] + 3.0 * uv[0])).sin()
                        + 0.45 * (TAU * (11.0 * uv[1] - 7.0 * uv[0]) + 1.9).sin()),
            uv[1]
                + 0.018
                    * ((TAU * (4.0 * uv[0] - 3.0 * uv[1]) + 0.4).sin()
                        + 0.45 * (TAU * (9.0 * uv[0] + 8.0 * uv[1]) + 2.6).sin()),
        ];
        w[0] = wo(uv, [220, 220], 1.0, 821);
        w[1] = wo(wq, [5, 5], 0.85, 823);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L899-916
        let px = ctx.px;
        let g = c[0];
        let grain = smoothstep(0.03, 0.25, g[1] - g[0]) * (1.0 - g[0] * g[0]);
        let gt = fract(g[2] * 5.7) - 0.5;
        let und = n[1];
        let blot = smoothstep(0.0, 0.7, n[0]);
        let k = c[1];
        let crack_on = step(0.72, fract(k[2] * 9.1 + k[3] * 3.3)) * smoothstep(0.0, 0.35, n[0]);
        let crack = (1.0 - aa(0.0003, (k[1] - k[0]) * 0.24, px)) * crack_on;
        let flake = step(0.82, fract(k[3] * 13.7 + k[2]))
            * (1.0 - aa(0.0, (k[0] - 0.08 - 0.05 * n[2]) * 0.48, px))
            * step(0.3, n[0]);
        let pass = smoothstep(-0.25, 0.25, n[1] + 0.35 * n[0]);
        let mut paint = 0.8
            * (1.0 + 0.1 * gt * grain - 0.05 * (1.0 - grain) + 0.035 * n[2] + 0.03 * und)
            * (1.0 + 0.06 * blot)
            * (1.0 - 0.035 * pass)
            * (1.0 - 0.3 * crack);
        paint *= 1.0 - 0.1 * smoothstep(0.2, 0.8, n[2]);
        let bare = scale3(lin([0.55, 0.54, 0.51]), 1.0 + 0.06 * gt * grain);
        // L912: premultiplied — bare render (flakes) or paint.
        s.alb = scale3(bare, flake);
        s.a = (1.0 - flake) * paint;
        s.h = 0.0006 * grain + 0.0011 * und + 0.0001 * n[2] - 0.0008 * crack - 0.0003 * flake;
        s.rough = 0.8 + 0.06 * blot - 0.04 * grain + 0.03 * flake;
        s.cav = 1.0 - 0.35 * crack;
    }
}

/// ≡ texlib.js L924-982.
#[derive(Debug, Clone, Copy)]
pub struct Treads;

impl SurfProgram for Treads {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L925-926
        f[0] = fb(ctx.uv, [6, 6], 4, 0.5, 901);
        f[1] = fb(ctx.uv, [48, 48], 2, 0.5, 907);
        f[2] = fb(ctx.uv, [16, 4], 3, 0.5, 911);
        f[3] = fb(ctx.uv, [10, 10], 3, 0.5, 913);
        w[0] = wo(ctx.uv, [40, 40], 1.0, 919);
        w[1] = wo(ctx.uv, [12, 16], 0.9, 929);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 58-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L927-981
        let p = ctx.p;
        let px = ctx.px;
        const PD: f32 = 0.28;
        const S2: f32 = 0.1403;
        const TT: f32 = 0.4040;
        const CT: f32 = 2.4751;
        let j = (p[1] / PD).floor();
        let a = p[1] - j * PD;
        let ar = PD * S2;
        let r1 = -a * CT;
        let r2 = (a - PD) * TT;
        let k_f = 0.004;
        let q_f = (k_f - (r1 - r2).abs()).max(0.0) / k_f;
        let mut h = r1.max(r2) + q_f * q_f * k_f * 0.25;
        let xa = if a < PD * 0.5 { a } else { a - PD };
        let l1 = xa * TT;
        let l2 = -xa * CT;
        let k_n = 0.014;
        let q_n = (k_n - (l1 - l2).abs()).max(0.0) / k_n;
        if xa.abs() < 0.02 {
            h = h.min(l1.min(l2) - q_n * q_n * k_n * 0.25);
        }
        let riser = 1.0 - smoothstep(ar - px, ar + px, a);
        let cp = [p[0] / 0.028, p[1] / 0.028];
        let ci = [cp[0].floor() as i32, cp[1].floor() as i32];
        let cf = [cp[0] - ci[0] as f32 - 0.5, cp[1] - ci[1] as f32 - 0.5];
        let odd = ((ci[0] + ci[1]) & 1) == 1;
        let rot: V2 = if odd {
            [cf[0] + cf[1], cf[1] - cf[0]]
        } else {
            [cf[0] - cf[1], cf[0] + cf[1]]
        };
        // GLSL literal kept verbatim; same f32 bits as FRAC_1_SQRT_2.
        #[allow(clippy::approx_constant)]
        let q = [rot[0] * 0.70710678, rot[1] * 0.70710678];
        let ld = length2([(q[0].abs() - 0.3).max(0.0), q[1]]) - 0.12;
        let lw = px / 0.028;
        let on_plate =
            smoothstep(ar + 0.006, ar + 0.014, a) * (1.0 - smoothstep(PD - 0.012, PD - 0.005, a));
        let loz = (1.0 - smoothstep(-lw, lw, ld)) * on_plate;
        let ldome = clampf(-ld / 0.12, 0.0, 1.0).sqrt();
        let lbase = (1.0 - smoothstep(0.0, 0.06, ld.abs())) * on_plate;
        let sp = c[0];
        let (mott, fine_n, strk, dull_n) = (n[0], n[1], n[2], n[3]);
        let mut galv = scale3(
            lin([0.6, 0.62, 0.64]),
            1.0 + 0.08 * (sp[2] - 0.5) + 0.05 * mott + 0.02 * fine_n,
        );
        let dull = smoothstep(0.1, 0.6, dull_n);
        galv = mix3(galv, lin([0.68, 0.69, 0.68]), dull * 0.35);
        let polish = loz * ldome * smoothstep(0.35, 0.75, 0.5 + 0.5 * strk);
        galv = mix3(galv, lin([0.76, 0.77, 0.79]), polish * 0.6);
        let nose = (1.0 - smoothstep(0.045 - px, 0.045 + px, PD - a))
            .max(1.0 - smoothstep(0.012 - px, 0.012 + px, a));
        let worn = smoothstep(
            0.62,
            0.86,
            0.5 + 0.5 * strk
                + 0.35 * (1.0 - smoothstep(0.0, 0.01, xa.abs()))
                + 0.15 * (sp[3] - 0.5),
        );
        let yel = scale3(
            lin([0.93, 0.72, 0.17]),
            (1.0 + 0.05 * mott) * (1.0 - 0.1 * smoothstep(0.3, 0.8, dull_n)),
        );
        let rc = c[1];
        let foot = (-((a - ar - 0.004) / 0.007).powf(2.0)).exp();
        let mut rust = foot * smoothstep(0.05, 0.5, dull_n + 0.35 * (rc[2] - 0.5));
        rust = rust.max(step(0.9, rc[2]) * (1.0 - smoothstep(0.06, 0.2, rc[0])) * on_plate * 0.8);
        let scuff = step(0.72, fract(rc[2] * 7.3))
            * (1.0 - smoothstep(0.1, 0.35, rc[0] + 0.15 * fine_n))
            * smoothstep(PD - 0.13, PD - 0.06, a)
            * (1.0 - nose);
        galv = scale3(galv, 1.0 - 0.3 * scuff);
        let rust_c = scale3(lin([0.46, 0.25, 0.12]), 0.85 + 0.3 * fine_n);
        let y_n = nose * (1.0 - worn);
        let r_p = riser * (1.0 - nose);
        let mut own = mix3(galv, yel, y_n);
        own = mix3(own, [0.0, 0.0, 0.0], r_p);
        let mut cov = r_p;
        own = mix3(own, rust_c, rust * 0.8);
        cov *= 1.0 - rust * 0.8;
        s.alb = own;
        s.a = cov * 0.8 * (0.94 + 0.08 * mott);
        s.h = h + 0.0013 * loz * ldome - 0.0002 * rust;
        s.rough = mix(
            mix(
                mix(
                    0.58 + 0.08 * (sp[2] - 0.5) + 0.1 * dull - 0.16 * polish + 0.08 * scuff,
                    0.62,
                    y_n,
                ),
                0.5,
                r_p,
            ),
            0.85,
            rust * 0.8,
        );
        s.metal = (0.35 + 0.3 * polish)
            * (1.0 - y_n)
            * (1.0 - r_p)
            * (1.0 - rust * 0.8)
            * (1.0 - 0.5 * scuff);
        s.cav = (1.0 - 0.35 * (-((a - ar) / 0.006).powf(2.0)).exp()) * (1.0 - 0.18 * lbase);
    }
}

/// ≡ texlib.js L984-1039.
#[derive(Debug, Clone, Copy)]
pub struct StoneStep;

impl SurfProgram for StoneStep {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L985-986
        f[0] = fb(ctx.uv, [4, 4], 4, 0.5, 1001);
        f[1] = fb(ctx.uv, [24, 24], 3, 0.5, 1009);
        f[2] = fb(ctx.uv, [96, 96], 2, 0.5, 1013);
        f[3] = fb(ctx.uv, [6, 30], 3, 0.5, 1019);
        w[0] = wo(ctx.uv, [120, 120], 1.0, 1021);
        w[1] = wo(ctx.uv, [24, 24], 0.9, 1031);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 52-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L987-1038
        let p = ctx.p;
        let px = ctx.px;
        const PD: f32 = 0.32;
        const S2: f32 = 0.1344;
        const TT: f32 = 0.3939;
        const CT: f32 = 2.5386;
        let j = (p[1] / PD).floor();
        let ji = j as i32;
        let a = p[1] - j * PD;
        let ar = PD * S2;
        let r1 = -a * CT;
        let r2 = (a - PD) * TT;
        let k_f = 0.005;
        let q_f = (k_f - (r1 - r2).abs()).max(0.0) / k_f;
        let mut h = r1.max(r2) + q_f * q_f * k_f * 0.25;
        let xa = if a < PD * 0.5 { a } else { a - PD };
        let l1 = xa * TT;
        let l2 = -xa * CT;
        let k_n = 0.03;
        let q_n = (k_n - (l1 - l2).abs()).max(0.0) / k_n;
        if xa.abs() < 0.03 {
            h = h.min(l1.min(l2) - q_n * q_n * k_n * 0.25);
        }
        let riser = 1.0 - smoothstep(ar - px, ar + px, a);
        let jx1 = hf([ji, 0], 3) * 1.92;
        let two = hf([ji, 1], 3) > 0.4;
        let span = 0.82 + 0.28 * hf([ji, 2], 3);
        let sx = glsl_mod(p[0] - jx1, 1.92);
        let d_j = if two {
            sx.min(1.92 - sx).min((sx - span).abs())
        } else {
            sx.min(1.92 - sx)
        };
        let pj = edge_prof(d_j, 0.0016, 0.004, 0.0014, 0.005, px);
        let si = if two && sx > span { 1 } else { 0 };
        let t1 = hf([ji, si], 5);
        let t2 = hf([ji, si], 7);
        let (mott, fine_n, grain, bed) = (n[0], n[1], n[2], n[3]);
        let fc = c[0];
        let fr = fc[0] * (1.92 / 120.0);
        let shell = step(0.78, fc[2]) * (1.0 - aa(0.0016 + 0.0022 * fract(fc[2] * 11.0), fr, px));
        let light = fract(fc[2] * 3.1) > 0.5;
        let pore = step(0.93, fract(fc[2] * 5.7)) * (1.0 - aa(0.0009, fr, px));
        let ins = smoothstep(0.02 - px, 0.02 + px, PD - a)
            * (1.0 - smoothstep(0.055 - px, 0.055 + px, PD - a));
        let grit = step(0.55, fract(fc[2] * 13.7 + fc[3]));
        let gl = (-((a - ar - 0.003) / 0.006).powf(2.0)).exp();
        let stain = smoothstep(0.1, 0.7, bed * 0.5 + 0.5 + 0.2 * mott)
            * (-(a - ar).max(0.0) / 0.035).exp()
            * (1.0 - riser);
        let cc = c[1];
        let chip = step(0.8, cc[2])
            * (1.0
                - aa(
                    0.01 + 0.012 * fract(cc[2] * 13.0),
                    cc[0] * (1.92 / 24.0) + 0.004 * fine_n,
                    px,
                ))
            * (1.0 - smoothstep(0.004, 0.02, xa.abs()));
        let mut tone = 0.8
            * (1.0 + 0.1 * (t1 - 0.5))
            * (1.0 + 0.05 * mott + 0.03 * fine_n + 0.025 * grain)
            * (1.0 + 0.035 * bed);
        tone *=
            (1.0 - 0.13 * riser) * (1.0 - 0.5 * gl) * (1.0 - 0.12 * stain) * (1.0 + 0.08 * chip);
        tone *= 1.0 + shell * (if light { 0.12 } else { -0.22 }) - 0.3 * pore;
        let mut own = [0.0f32; 3];
        let mut cov = 1.0;
        let h_k = 0.16 * (t2 - 0.5).abs() * 2.0;
        let h_c = scale3(
            if t2 > 0.5 {
                lin([0.84, 0.78, 0.66])
            } else {
                lin([0.7, 0.71, 0.7])
            },
            tone / 0.8,
        );
        own = mix3(own, h_c, h_k);
        cov *= 1.0 - h_k;
        let ins_c = scale3(
            lin([0.17, 0.17, 0.18]),
            (1.0 + 0.5 * (grit - 0.5)) * (1.0 + 0.1 * mott),
        );
        own = mix3(own, ins_c, ins);
        cov *= 1.0 - ins;
        own = mix3(
            own,
            scale3(lin([0.38, 0.37, 0.35]), 1.0 + 0.1 * grain),
            pj[1],
        );
        cov *= 1.0 - pj[1];
        s.alb = own;
        s.a = cov * tone;
        s.h = h
            + pj[0]
            + (0.00012 * grain + 0.00008 * fine_n - 0.0006 * ins - 0.0025 * chip - 0.0004 * pore)
                * (1.0 - pj[1]);
        s.rough = mix(
            mix(0.64 + 0.05 * fine_n + 0.06 * riser + 0.08 * chip, 0.93, ins),
            0.9,
            pj[1],
        );
        s.cav = (1.0 - 0.3 * gl) * mix(1.0, 0.6, pj[1]) * (1.0 - 0.3 * pore);
    }
}

/// ≡ texlib.js L1041-1095.
#[derive(Debug, Clone, Copy)]
pub struct RampBoard;

impl SurfProgram for RampBoard {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L1042-1047
        let p = ctx.p;
        let uv = ctx.uv;
        let bw = 1.6 / 7.0;
        let bi = (p[0] / bw).floor() as i32;
        let lx = p[0] - bi as f32 * bw;
        f[0] = Req {
            x: [lx * 28.0 + bi as f32 * 13.1, uv[1] * 6.0],
            per: [4096, 6],
            oct: 3,
            gain: 0.5,
            seed: 1101,
        };
        f[1] = fb(uv, [5, 5], 4, 0.5, 1103);
        f[2] = Req {
            x: [lx * 90.0 + bi as f32 * 7.3, uv[1] * 40.0],
            per: [4096, 40],
            oct: 2,
            gain: 0.5,
            seed: 1109,
        };
        f[3] = fb(uv, [8, 4], 3, 0.5, 1117);
        w[0] = wo(uv, [7, 12], 1.0, 1123);
    }

    #[allow(clippy::too_many_lines)] // faithful 1:1 port of a 47-line GLSL body
    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L1048-1094
        let p = ctx.p;
        let px = ctx.px;
        const BW: f32 = 1.6 / 7.0;
        const PD: f32 = 0.4;
        let bi = (p[0] / BW).floor() as i32;
        let lx = p[0] - bi as f32 * BW;
        let e_s = lx.min(BW - lx);
        let ps = edge_prof(e_s, 0.004, 0.006, 0.002, 0.022, px);
        let b1 = hf([bi, 0], 3);
        let b2 = hf([bi, 1], 3);
        let b3 = hf([bi, 2], 3);
        let e_e = jd(p[1] - b1 * 1.6, 1.6);
        let pe = edge_prof(e_e, 0.002, 0.004, 0.0015, 0.018, px);
        let cy = p[1] - ((p[1] / PD).floor() + 0.5) * PD;
        let ce = 0.025 - cy.abs();
        let cl = smoothstep(-px, px, ce);
        let h_cl = 0.028
            * (1.0 - (1.0 - clampf(ce / 0.007, 0.0, 1.0)).powf(2.0))
                .max(0.0)
                .sqrt();
        let gap = ps[1].max(pe[1]) * (1.0 - cl);
        let band = smoothstep(0.018 - px, 0.018 + px, e_e)
            * (1.0 - smoothstep(0.043 - px, 0.043 + px, e_e))
            * (1.0 - ps[1])
            * (1.0 - cl);
        let nd = length2([(lx - 0.05).abs().min((lx - (BW - 0.05)).abs()), cy]);
        let nail = (1.0 - aa(0.0045, nd, px)) * cl;
        let bnail = (1.0 - aa(0.004, length2([lx - BW * 0.5, e_e - 0.0305]), px)) * band;
        let dirt = smoothstep(0.02, 0.03, -cy)
            * (1.0 - smoothstep(0.035, 0.08 + 0.03 * n[3], -cy))
            * smoothstep(-0.3, 0.4, n[3])
            * (1.0 - cl);
        let (streak, blot, fib) = (n[0], n[1], n[2]);
        let wa = lin([0.6, 0.5, 0.39]);
        let wb = lin([0.5, 0.43, 0.35]);
        let wc = lin([0.66, 0.57, 0.46]);
        let mut wood = scale3(
            mix3(mix3(wa, wb, b2), wc, step(0.62, b3) * 0.7),
            0.84 + 0.28 * b1,
        );
        wood = scale3(wood, 1.0 + 0.14 * streak + 0.05 * fib);
        let grey = scale3(lin([0.52, 0.51, 0.48]), 1.0 + 0.12 * streak);
        wood = mix3(
            wood,
            grey,
            clampf(
                0.3 + 0.35 * smoothstep(-0.4, 0.6, blot) + 0.2 * (b3 - 0.5),
                0.0,
                0.85,
            ),
        );
        let kc = c[0];
        let knot = step(0.86, kc[2]) * (1.0 - smoothstep(0.1, 0.22, kc[0]));
        wood = mix3(wood, lin([0.28, 0.2, 0.14]), knot * 0.7);
        wood = scale3(
            wood,
            1.0 - 0.18 * (1.0 - smoothstep(0.0, 0.03, e_e - 0.002)),
        );
        let mut cleat_c = scale3(
            lin([0.44, 0.36, 0.28]),
            (0.9 + 0.2 * hf([(p[1] / PD).floor() as i32, 5], 9)) * (1.0 + 0.1 * streak),
        );
        cleat_c = mix3(cleat_c, scale3(grey, 0.9), 0.25);
        let mut col = mix3(wood, cleat_c, cl);
        col = mix3(
            col,
            scale3(lin([0.3, 0.26, 0.21]), 1.0 + 0.2 * blot),
            dirt * 0.7,
        );
        col = mix3(col, scale3(lin([0.5, 0.52, 0.54]), 1.0 + 0.1 * fib), band);
        col = mix3(col, lin([0.38, 0.38, 0.39]), nail.max(bnail));
        col = mix3(col, lin([0.06, 0.05, 0.045]), gap);
        let h_b = ps[0].min(pe[0])
            + (0.0004 * streak + 0.0002 * fib) * (1.0 - ps[1].max(pe[1]))
            + 0.0008 * band;
        s.alb = col;
        s.h = mix(h_b, h_b.max(h_cl), cl) + 0.0004 * nail.max(bnail);
        s.rough = mix(
            mix(
                mix(mix(0.78 + 0.08 * fib, 0.82, cl), 0.9, dirt * 0.7),
                0.5,
                band,
            ),
            0.95,
            gap,
        );
        s.rough = mix(s.rough, 0.4, nail.max(bnail));
        s.metal = 0.7 * band + 0.8 * nail.max(bnail);
        s.cav = mix(1.0, 0.35, gap)
            * (1.0 - 0.15 * dirt)
            * (1.0 - 0.55 * (-((ce + 0.003) / 0.005).powf(2.0)).exp() * (1.0 - cl));
    }
}

/// ≡ texlib.js L1097-1134.
#[derive(Debug, Clone, Copy)]
pub struct GangDeck;

impl SurfProgram for GangDeck {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L1098-1099
        f[0] = fb(ctx.uv, [40, 3], 3, 0.5, 1201);
        f[1] = fb(ctx.uv, [5, 5], 4, 0.5, 1203);
        f[2] = fb(ctx.uv, [24, 24], 3, 0.5, 1207);
        w[0] = wo(ctx.uv, [12, 12], 0.9, 1213);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L1100-1133
        let p = ctx.p;
        let px = ctx.px;
        const PW: f32 = 0.15;
        const PD: f32 = 0.3;
        let pk = (p[0] / PW).floor() as i32;
        let lx = p[0] - pk as f32 * PW;
        let e_s = lx.min(PW - lx);
        let ps = edge_prof(e_s, 0.001, 0.003, 0.001, 0.004, px);
        let rib = (0.5 + 0.5 * (TAU * (lx - 0.0075) / 0.015).cos()).powf(3.0)
            * smoothstep(0.004, 0.009, e_s);
        let cy = p[1] - ((p[1] / PD).floor() + 0.5) * PD;
        let ce = 0.0175 - cy.abs();
        let cl = smoothstep(-px, px, ce);
        let h_cl = 0.018
            * (1.0 - (1.0 - clampf(ce / 0.005, 0.0, 1.0)).powf(2.0))
                .max(0.0)
                .sqrt();
        let b_d = length2([lx - PW * 0.5, cy]);
        let bolt = (1.0 - aa(0.0065, b_d, px)) * cl;
        let b_dome = (1.0 - (b_d / 0.0065).powf(2.0)).max(0.0).sqrt();
        let strk = n[0];
        let ox = smoothstep(0.1, 0.7, n[1]);
        let sc = n[2];
        let wc = c[0];
        let scuff = step(0.82, wc[2])
            * (1.0 - smoothstep(0.05, 0.3, (wc[0] - 0.25).abs() + 0.5 * sc.abs()))
            * smoothstep(0.2, 0.6, strk * 0.5 + 0.5)
            * (0.5 + 0.5 * cl);
        let dent = step(0.93, fract(wc[2] * 7.1)) * (1.0 - smoothstep(0.0, 0.25, wc[0]));
        let mut al = scale3(lin([0.72, 0.73, 0.74]), 1.0 + 0.05 * strk + 0.02 * sc);
        al = mix3(al, lin([0.77, 0.78, 0.78]), ox * 0.4);
        let pol = cl * smoothstep(0.004, 0.012, ce) * smoothstep(-0.2, 0.5, sc);
        al = mix3(al, lin([0.82, 0.83, 0.85]), pol * 0.5);
        let mut col = mix3(al, lin([0.07, 0.07, 0.075]), scuff * 0.8);
        col = mix3(col, lin([0.5, 0.51, 0.52]), bolt);
        col = scale3(col, 1.0 - 0.5 * ps[1] * (1.0 - cl));
        let h_d = ps[0] + 0.0009 * rib - 0.0015 * dent;
        s.alb = col;
        s.h = mix(h_d, h_d.max(h_cl), cl) + 0.002 * bolt * b_dome;
        s.rough = mix(
            0.48 + 0.05 * strk + 0.16 * ox - 0.14 * pol,
            0.75,
            scuff * 0.8,
        );
        s.metal = (0.42 + 0.3 * pol) * (1.0 - scuff * 0.8);
        s.cav = mix(1.0, 0.55, ps[1] * (1.0 - cl))
            * (1.0 - 0.2 * (1.0 - rib) * (1.0 - cl))
            * (1.0 - 0.3 * (-((ce + 0.0015) / 0.002).powf(2.0)).exp());
    }
}

/// ≡ texlib.js L1138-1150.
#[derive(Debug, Clone, Copy)]
pub struct Detail;

impl SurfProgram for Detail {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L1139-1140
        f[0] = fb(ctx.uv, [24, 24], 3, 0.5, 3);
        f[1] = fb(ctx.uv, [96, 96], 2, 0.5, 5);
        w[0] = wo(ctx.uv, [150, 150], 1.0, 7);
        w[1] = wo(ctx.uv, [40, 40], 0.9, 9);
    }

    fn surf(&self, ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L1141-1149
        let px = ctx.px;
        let g = c[0];
        let grain = smoothstep(0.05, 0.3, g[1] - g[0]);
        let tone = fract(g[2] * 7.31) - 0.5;
        let pit = step(0.93, c[1][2])
            * (1.0 - smoothstep(0.0007, 0.0007 + 2.0 * px, c[1][0] * (0.5 / 40.0)));
        let m = 1.0 + 0.14 * tone * grain + 0.06 * n[0] + 0.035 * n[1] - 0.16 * pit;
        s.alb = v3(0.5 * m);
        s.h = 0.0002 * grain * (1.0 - g[0] * g[0]) + 0.00006 * n[1] - 0.00025 * pit;
        s.rough = 0.5 + 0.05 * n[0] + 0.04 * tone * grain;
    }
}

/// ≡ texlib.js L1154-1162.
#[derive(Debug, Clone, Copy)]
pub struct Gel;

impl SurfProgram for Gel {
    fn prep(&self, ctx: &PrepCtx, f: &mut [Req; 4], w: &mut [WReq; 2]) {
        // L1155
        f[0] = fb(ctx.uv, [4, 4], 3, 0.45, 91);
        f[1] = fb(ctx.uv, [12, 12], 2, 0.5, 97);
        w[0] = wo(ctx.uv, [7, 7], 1.0, 101);
    }

    fn surf(&self, _ctx: &SurfCtx, n: &[f32; 4], c: &[[f32; 4]; 2], s: &mut Surf) {
        // L1156-1161
        let wc = c[0];
        let dimple = (1.0 - smoothstep(0.0, 0.22, wc[0])) * step(0.6, wc[2]);
        s.h = 0.0034 * n[0] + 0.0009 * n[1] - 0.0012 * dimple * dimple;
        s.alb = v3(0.5);
        s.rough = 0.5;
    }
}

/// Base-layer registry: the 25 names of `MATERIALS` (texlib.js L211-1163).
/// Stage layers (`'<id>:<name>'`) resolve in [`super::materials_stage`].
#[must_use]
pub fn lookup(name: &str) -> Option<&'static dyn SurfProgram> {
    match name {
        "concrete" => Some(&Concrete),
        "pavers" => Some(&Pavers),
        "tiles" => Some(&Tiles),
        "asphalt" => Some(&Asphalt),
        "boardwalk" => Some(&Boardwalk),
        "metalpanel" => Some(&MetalPanel),
        "corrugated" => Some(&Corrugated),
        "hazard" => Some(&Hazard),
        "grate" => Some(&Grate),
        "brick" => Some(&Brick),
        "rubber" => Some(&Rubber),
        "glasstile" => Some(&GlassTile),
        "planks" => Some(&Planks),
        "hullpaint" => Some(&HullPaint),
        "nonslip" => Some(&NonSlip),
        "gelcoat" => Some(&GelCoat),
        "yard" => Some(&Yard),
        "weatherboard" => Some(&WeatherBoard),
        "render" => Some(&Render),
        "treads" => Some(&Treads),
        "stonestep" => Some(&StoneStep),
        "rampboard" => Some(&RampBoard),
        "gangdeck" => Some(&GangDeck),
        "detail" => Some(&Detail),
        "gel" => Some(&Gel),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texlib::bake::bake_layer;
    use crate::texlib::schema::embedded_texlib;

    /// The 25 base layer names in schema order (≡ texlib.js L211-1163).
    const BASE: [&str; 25] = [
        "concrete",
        "pavers",
        "tiles",
        "asphalt",
        "boardwalk",
        "metalpanel",
        "corrugated",
        "hazard",
        "grate",
        "brick",
        "rubber",
        "glasstile",
        "planks",
        "hullpaint",
        "nonslip",
        "gelcoat",
        "yard",
        "weatherboard",
        "render",
        "treads",
        "stonestep",
        "rampboard",
        "gangdeck",
        "detail",
        "gel",
    ];

    #[test]
    fn registry_covers_25_base_layers() {
        let doc = embedded_texlib();
        for (i, name) in BASE.iter().enumerate() {
            assert_eq!(doc.layers[i].name, *name, "schema layer {i}");
            assert!(lookup(name).is_some(), "lookup missing base layer {name}");
        }
        assert!(lookup("Concrete").is_none()); // names are exact
        assert!(lookup("no-such-layer").is_none());
        assert!(lookup("hull:paintwet").is_none()); // stage names → materials_stage
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
    /// C1): sha256 of the base-level albedo + normal + orm bytes.
    const GOLDENS: [(&str, &str); 25] = [
        (
            "concrete",
            "7cd30e126906c3d5f812d0e934de3e2db84032ae052cd55e5969d1612db43ecc",
        ),
        (
            "pavers",
            "edc223b03934bbef6c8c6090cac3275ef6c8c4e83737f25c912f405249bf5cf9",
        ),
        (
            "tiles",
            "366a976b30a82a49bb20a00c26ebef6eb8fe9a2bb233fe74def03d055cacc50f",
        ),
        (
            "asphalt",
            "d59fdfa18f1ed30d3979915c888bbef885ebf596e4062ebf480debb26edabb2a",
        ),
        (
            "boardwalk",
            "8855fab16b1fbfcf58d28dd353629e2fa49552db47bdb39b917e8de8fc0a687c",
        ),
        (
            "metalpanel",
            "766c2172b35dcaec8172879aa841491c65bf17e2f0bfd12de8ac18937abb4547",
        ),
        (
            "corrugated",
            "8f52736542158774460a45480f4d19635e86da4baa114c63e86c69bce2fe210b",
        ),
        (
            "hazard",
            "01c149db841ecb51c990d4597b14238ebd4cd2ae19d44ef78dffdb3a76200c46",
        ),
        (
            "grate",
            "1a1fdf199099b657384e3499361d54bb0ad487d67df31110b0603e9902173aa4",
        ),
        (
            "brick",
            "27175dbd798d23889b85c96834c946af05bbb5574bb59520f34812b81fe1d08c",
        ),
        (
            "rubber",
            "0486a4163bae69d71680e166239e438096f7944b715583f6d12470095b4a9ebf",
        ),
        (
            "glasstile",
            "ca62cd6e9915850aebce2095a69448a7c6d12b0028184d13231ceeb0c3abc4d7",
        ),
        (
            "planks",
            "d3dc97034450d1d04f6b371ddf80b4de02456525aa61bf5b6d628517ba2954c1",
        ),
        (
            "hullpaint",
            "bfe1aaccc8813a418850745bc25e7bc9c9bc62b0c01965f88ef503bb1096a44e",
        ),
        (
            "nonslip",
            "8881c18a75ceea182e39dc1c0f0cb4b177aebab17ad077ec9d371907f8fb8ef4",
        ),
        (
            "gelcoat",
            "ca81f337d5222c4cc92164e0da82c82aea117f4627c8a87069799a5a4fa40110",
        ),
        (
            "yard",
            "875dc98731c327118a3cc4588f288df71129cf926c3d4d86776eba0f0c21b8bf",
        ),
        (
            "weatherboard",
            "8c3162e3d6f8534990eab67772241a0784fe3078fe337db8d9f621c1a5bf377e",
        ),
        (
            "render",
            "73268db43d85c8b9cc748fe74cba1790a0918552b4f4806e2691706911db6261",
        ),
        (
            "treads",
            "2f6f01708e026752b5dffc703e9102d340d551b1ee3f8e9d7f188f1fbbbf112d",
        ),
        (
            "stonestep",
            "088b2656ac8e4b7d580f510ab5c22bc4bd981fcf96ac4c8bbeea924762a86e63",
        ),
        (
            "rampboard",
            "5be71dc624bc74d69038c3bfd32b3acff2d9db7888f750053a8f4c4e2b90ad46",
        ),
        (
            "gangdeck",
            "98c11d29c46ab0f3f2b4ce5d5f04e45dbe9bfb0836870835dd20e41b4b2962a5",
        ),
        (
            "detail",
            "fe55bff443ecaf0c62b54faf872c849abeb4abd8fe80a15a72d7896d06046efe",
        ),
        (
            "gel",
            "0781b3fcfc0cd48f4fe94e37180e7009752de464e400c91e412f90dc2bdb492a",
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
        for name in BASE {
            let layer = doc.layers.iter().find(|l| l.name == name).expect(name);
            let prog = lookup(name).expect(name);
            let b = bake_layer(layer, prog, 32);
            let alb = &b.albedo.levels[0].rgba;
            let orm = &b.orm.levels[0].rgba;
            assert_eq!(alb.len(), 32 * 32 * 4, "{name}: albedo base size");
            // u8 bytes are trivially in 0..=255; the weak sanity is that the
            // fields are alive. Mask layers are premultiplied, so their own
            // rgb may legitimately sit at 0 (paint carries the colour); they
            // must still show non-zero coverage in albedo.a.
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
