//! Math helpers ≡ `character.js` L43-105 (allocation-free scalar kit + springs).
//!
//! Float32 bank semantics (REVIEW_CHECKLIST): JS `Float32Array` stores f32 but
//! computes in f64. Every bank write here is `f64 arithmetic → round to f32`
//! exactly like `a[i] = <f64 expr>` in JS; bank reads widen f32 → f64.
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively

use glam::{DMat4, DQuat, DVec3};

/// `THREE.Matrix4.decompose` (node_modules/three/src/math/Matrix4.js
/// L1071-1124 + `Quaternion.setFromRotationMatrix`): column-length scale with
/// the determinant-sign flip on X, then the raw trace method on the
/// scale-divided matrix — NO re-orthogonalization, unlike glam's
/// `to_scale_rotation_translation`, which diverges once the chain carries
/// shear (non-uniform kid squash × rotated bones). Returns `(pos, quat, scale)`
/// in that order.
#[must_use]
pub fn decompose_three(m: &DMat4) -> (DVec3, DQuat, DVec3) {
    let c0 = m.x_axis;
    let c1 = m.y_axis;
    let c2 = m.z_axis;
    let pos = m.w_axis.truncate();
    let det = m.determinant();
    if det == 0.0 {
        return (pos, DQuat::IDENTITY, DVec3::ONE);
    }
    let mut sx = c0.length();
    let sy = c1.length();
    let sz = c2.length();
    if det < 0.0 {
        sx = -sx;
    }
    // m_rc with THREE's row-major naming over column-major storage.
    let m11 = c0.x / sx;
    let m21 = c0.y / sx;
    let m31 = c0.z / sx;
    let m12 = c1.x / sy;
    let m22 = c1.y / sy;
    let m32 = c1.z / sy;
    let m13 = c2.x / sz;
    let m23 = c2.y / sz;
    let m33 = c2.z / sz;
    let trace = m11 + m22 + m33;
    let q = if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        DQuat::from_xyzw((m32 - m23) * s, (m13 - m31) * s, (m21 - m12) * s, 0.25 / s)
    } else if m11 > m22 && m11 > m33 {
        let s = 2.0 * (1.0 + m11 - m22 - m33).sqrt();
        DQuat::from_xyzw(0.25 * s, (m12 + m21) / s, (m13 + m31) / s, (m32 - m23) / s)
    } else if m22 > m33 {
        let s = 2.0 * (1.0 + m22 - m11 - m33).sqrt();
        DQuat::from_xyzw((m12 + m21) / s, 0.25 * s, (m23 + m32) / s, (m13 - m31) / s)
    } else {
        let s = 2.0 * (1.0 + m33 - m11 - m22).sqrt();
        DQuat::from_xyzw((m13 + m31) / s, (m23 + m32) / s, 0.25 * s, (m21 - m12) / s)
    };
    (pos, q, DVec3::new(sx, sy, sz))
}

/// ≡ character.js L43
#[inline]
#[must_use]
pub fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// ≡ character.js L44
#[inline]
#[must_use]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// ≡ character.js L45
#[inline]
#[must_use]
pub fn sstep(a: f64, b: f64, x: f64) -> f64 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// ≡ character.js L46
#[inline]
#[must_use]
pub fn damp(a: f64, b: f64, l: f64, dt: f64) -> f64 {
    a + (b - a) * (1.0 - (-l * dt).exp())
}

/// ≡ character.js L47
#[inline]
#[must_use]
pub fn ease(t: f64) -> f64 {
    let t = clamp(t, 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// ≡ character.js L48
#[inline]
#[must_use]
pub fn ease_out(t: f64) -> f64 {
    let t = clamp(t, 0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// ≡ character.js L49
#[inline]
#[must_use]
pub fn ease_in(t: f64) -> f64 {
    let t = clamp(t, 0.0, 1.0);
    t * t
}

/// ≡ character.js L50 (minimum-jerk)
#[inline]
#[must_use]
pub fn mj(t: f64) -> f64 {
    let t = clamp(t, 0.0, 1.0);
    t * t * t * (10.0 + t * (6.0 * t - 15.0))
}

/// ≡ character.js L51 (`s` defaulted to 2.2 at every call site so far)
#[inline]
#[must_use]
pub fn back_out(t: f64, s: f64) -> f64 {
    let t = clamp(t, 0.0, 1.0) - 1.0;
    t * t * ((s + 1.0) * t + s) + 1.0
}

/// ≡ character.js L52
#[inline]
#[must_use]
pub fn frac(x: f64) -> f64 {
    x - x.floor()
}

pub const TAU: f64 = std::f64::consts::TAU;

/// ≡ character.js L54 `hashStr` — FNV-1a over the UTF-16 code units (JS
/// `charCodeAt`), 32-bit wrapping.
#[must_use]
pub fn hash_str(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in s.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// ≡ character.js L55 `mulberry` — mulberry32 PRNG returning [0,1).
#[derive(Debug, Clone)]
pub struct Rng {
    a: u32,
}

impl Rng {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self { a: seed }
    }
    #[must_use]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let mut t = self.a ^ (self.a >> 15);
        t = t.wrapping_mul(1 | self.a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        // NB: JS is `t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t` — `+` binds
        // tighter than `^`, so the addend is folded in before the final xor.
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }
}

/// ≡ character.js L57 (impulse envelope)
#[inline]
#[must_use]
pub fn pulse(t: f64, atk: f64, dec: f64) -> f64 {
    if t < 0.0 {
        0.0
    } else if t < atk {
        t / atk
    } else {
        (-(t - atk) * dec).exp()
    }
}

/// ≡ character.js L59 (window)
#[inline]
#[must_use]
pub fn win(t: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    if t <= a || t >= d {
        0.0
    } else if t < b {
        ease((t - a) / (b - a))
    } else if t > c {
        1.0 - ease((t - c) / (d - c))
    } else {
        1.0
    }
}

/// ≡ character.js L61 (hand-shape keys)
#[inline]
#[must_use]
pub fn hk(h: f64, fist: f64, grip: f64, relaxed: f64, open: f64) -> f64 {
    if h < 0.0 {
        grip + (fist - grip) * (-h).min(1.0)
    } else if h < 1.0 {
        grip + (relaxed - grip) * h
    } else {
        relaxed + (open - relaxed) * (h - 1.0).min(1.0)
    }
}

/// ≡ character.js L62
#[must_use]
pub fn wrap_a(mut a: f64) -> f64 {
    while a > std::f64::consts::PI {
        a -= TAU;
    }
    while a < -std::f64::consts::PI {
        a += TAU;
    }
    a
}

/// ≡ character.js L63
#[inline]
#[must_use]
pub fn damp_angle(a: f64, b: f64, l: f64, dt: f64) -> f64 {
    a + wrap_a(b - a) * (1.0 - (-l * dt).exp())
}

/// ≡ character.js L65-70 (keyed curve, eased held keys)
#[must_use]
pub fn kf(t: f64, tk: &[f64], v: &[f64]) -> f64 {
    let n = tk.len();
    if t <= tk[0] {
        return v[0];
    }
    if t >= tk[n - 1] {
        return v[n - 1];
    }
    let mut i = 1;
    while tk[i] < t {
        i += 1;
    }
    let u = (t - tk[i - 1]) / (tk[i] - tk[i - 1]);
    v[i - 1] + (v[i] - v[i - 1]) * (u * u * (3.0 - 2.0 * u))
}

/// ≡ character.js L72-80 (Catmull-Rom keyed curve, flat at the ends)
#[must_use]
pub fn kc(t: f64, tk: &[f64], v: &[f64]) -> f64 {
    let n = tk.len();
    if t <= tk[0] {
        return v[0];
    }
    if t >= tk[n - 1] {
        return v[n - 1];
    }
    let mut i = 1;
    while tk[i] < t {
        i += 1;
    }
    let t0 = tk[i - 1];
    let h = tk[i] - t0;
    let u = (t - t0) / h;
    let m0 = if i > 1 {
        (v[i] - v[i - 2]) / (tk[i] - tk[i - 2])
    } else {
        0.0
    };
    let m1 = if i < n - 1 {
        (v[i + 1] - v[i - 1]) / (tk[i + 1] - tk[i - 1])
    } else {
        0.0
    };
    let u2 = u * u;
    let u3 = u2 * u;
    (2.0 * u3 - 3.0 * u2 + 1.0) * v[i - 1]
        + (u3 - 2.0 * u2 + u) * h * m0
        + (-2.0 * u3 + 3.0 * u2) * v[i]
        + (u3 - u2) * h * m1
}

/// ≡ character.js L82-88 — damped spring, semi-implicit Euler, sub-stepped.
/// `s` is the Float32 spring bank (`s[i]` = x, `s[i+1]` = v); the loop runs in
/// f64 and writes back once, exactly like the JS Float32Array.
#[allow(clippy::too_many_arguments)]
pub fn spr(s: &mut [f32], i: usize, target: f64, hz: f64, zeta: f64, dt: f64) -> f64 {
    let w = TAU * hz;
    let k = w * w;
    let c = 2.0 * zeta * w;
    let mut x = s[i] as f64;
    let mut v = s[i + 1] as f64;
    let n = (dt * w * 1.1).ceil().max(1.0) as usize;
    let h = dt / n as f64;
    for _ in 0..n {
        v += (k * (target - x) - c * v) * h;
        x += v * h;
    }
    s[i] = x as f32;
    s[i + 1] = v as f32;
    x
}

/// ≡ character.js L92-105 — spring advanced with the exact analytic solution
/// (stable for any damping ratio; used for weapon kicks).
#[allow(clippy::too_many_arguments)]
pub fn spr_a(s: &mut [f32], i: usize, target: f64, hz: f64, zeta: f64, dt: f64) -> f64 {
    let w = TAU * hz;
    let x0 = s[i] as f64 - target;
    let v0 = s[i + 1] as f64;
    let (x, v);
    if zeta < 0.999 {
        let wd = w * (1.0 - zeta * zeta).sqrt();
        let e = (-(zeta * w * dt)).exp();
        let c = (wd * dt).cos();
        let sn = (wd * dt).sin();
        let b = (v0 + zeta * w * x0) / wd;
        x = e * (x0 * c + b * sn);
        v = e * ((-zeta * w * x0 + wd * b) * c + (-zeta * w * b - wd * x0) * sn);
    } else {
        let e = (-(w * dt)).exp();
        let b = v0 + w * x0;
        x = e * (x0 + b * dt);
        v = e * (v0 - w * b * dt);
    }
    s[i] = (x + target) as f32;
    s[i + 1] = v as f32;
    s[i] as f64
}

// ---------------------------------------------------------------------------
// THREE.Euler orderings (≡ `setFromEuler` in character.js)
// ---------------------------------------------------------------------------

/// `THREE.Euler` default order `'XYZ'` ≡ intrinsic X→Y→Z ≡ matrix Rx·Ry·Rz
/// (verified against three.js `Quaternion.setFromEuler` case 'XYZ').
#[inline]
#[must_use]
pub fn euler_xyz(x: f64, y: f64, z: f64) -> glam::DQuat {
    glam::DQuat::from_rotation_x(x)
        * glam::DQuat::from_rotation_y(y)
        * glam::DQuat::from_rotation_z(z)
}

/// `THREE.Euler` order `'YXZ'` ≡ intrinsic Y→X→Z ≡ matrix Ry·Rx·Rz (verified
/// against three.js `Quaternion.setFromEuler` case 'YXZ'). Used for the bones
/// `_buildRig` marks YXZ (hips, spine, chest, neck, head, clavL, clavR) and
/// every explicit `_e1.set(..., 'YXZ')` call site.
#[inline]
#[must_use]
pub fn euler_yxz(x: f64, y: f64, z: f64) -> glam::DQuat {
    glam::DQuat::from_rotation_y(y)
        * glam::DQuat::from_rotation_x(x)
        * glam::DQuat::from_rotation_z(z)
}

/// Inverse of [`euler_yxz`] ≡ `THREE.Euler.setFromQuaternion(q, 'YXZ')`
/// (derived from the same Ry·Rx·Rz matrix; read back for the clavicle
/// protraction loop, character.js L3513-3516).
#[inline]
#[must_use]
pub fn euler_from_yxz(q: glam::DQuat) -> (f64, f64, f64) {
    let [x, y, z, w] = q.to_array();
    // From the Ry·Rx·Rz matrix rows: m[1][2] = −sin x, m[0][2] = sin y·cos x,
    // m[2][2] = cos y·cos x, m[1][0] = cos x·sin z, m[1][1] = cos x·cos z;
    // with m[1][2] = 2(yz−wx), m[0][2] = 2(xz+wy), m[2][2] = 1−2(xx+yy),
    // m[1][0] = 2(xy+wz), m[1][1] = 1−2(xx+zz).
    let ex = (2.0 * (w * x - y * z)).clamp(-1.0, 1.0).asin();
    let ey = (2.0 * (x * z + w * y)).atan2(1.0 - 2.0 * (x * x + y * y));
    let ez = (2.0 * (x * y + w * z)).atan2(1.0 - 2.0 * (x * x + z * z));
    (ex, ey, ez)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_yxz_roundtrip() {
        let (x, y, z) = (0.3, -0.7, 1.1);
        let q = euler_yxz(x, y, z);
        let (rx, ry, rz) = euler_from_yxz(q);
        assert!((rx - x).abs() < 1e-12 && (ry - y).abs() < 1e-12 && (rz - z).abs() < 1e-12);
    }

    #[test]
    fn mulberry32_matches_js_reference_values() {
        // Reference stream from `mulberry(hashStr('pin-test'))` (node): the
        // first five draws pinned by the batch-B harness cross-check.
        let seed = hash_str("pin-test");
        assert_eq!(seed, 1924120943, "hashStr('pin-test') ≡ fixture meta.seed");
        let mut r = Rng::new(seed);
        let a = [r.next(), r.next(), r.next(), r.next(), r.next()];
        assert!(a.iter().all(|v| *v >= 0.0 && *v < 1.0));
        // hairPh[0] = rng()*TAU must reproduce the fixture hair.ph[0].
        assert!(
            (a[0] * TAU - 4.432_740_149_716_98).abs() < 1e-9,
            "hairPh0 vs fixture: {}",
            a[0] * TAU
        );
    }

    #[test]
    fn spr_is_deterministic_f32_bank() {
        let mut s = [0f32, 0f32];
        let mut prev = f64::NAN;
        for _ in 0..4 {
            let x = spr(&mut s, 0, 1.0, 3.0, 0.5, 1.0 / 60.0);
            assert!(x > prev || prev.is_nan());
            prev = x;
        }
    }
}
