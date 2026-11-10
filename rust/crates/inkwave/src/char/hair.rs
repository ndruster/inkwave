//! Hair springs ≡ `character.js` `_updateHair` (L3803-3916).
//!
//! Called from `apply::apply_pose` (≡ L3528). The head world motion is
//! reconstructed from the LOCAL scene-graph mirrors (`root_pos`/`root_yaw`,
//! `model`, `kid`, the head bone chain) exactly as THREE's
//! `updateWorldMatrix` would; the per-strand springs then write the hair
//! bones' LOCAL quaternions.
#![allow(dead_code)] // Task 54 batch B1/C consumers land progressively
use glam::{DMat4, DQuat, DVec3};

use super::anim::CharAnim;
use super::math::*;

/// `THREE.Object3D.updateMatrix()` ≡ compose(T·R·S).
#[inline]
fn compose(pos: DVec3, quat: DQuat, scale: DVec3) -> DMat4 {
    DMat4::from_scale_rotation_translation(scale, quat, pos)
}

/// ≡ `_updateHair(dt)` (character.js L3803-3916).
pub fn update_hair(c: &mut CharAnim, dt: f64) {
    let tab = c.tab;
    // ---- world matrices (L3804-3806): `model.updateWorldMatrix(true,false)`
    // then `kid.matrixWorld = model.matrixWorld · kid.matrix` (the root is
    // parented to the scene at `root_pos` with `rotation.y = root_yaw`).
    let root_w = compose(c.root_pos, DQuat::from_rotation_y(c.root_yaw), DVec3::ONE);
    let model_w = root_w * compose(c.model.pos, c.model.quat, c.model.scale);
    let kid_mw = model_w * compose(c.kid.pos, c.kid.quat, c.kid.scale);

    // ---- head centre / orientation in world space (L3807-3811)
    let (v2, q1) = c.kid_xform(tab.rig.bone_index["head"]);
    let mut v1 = q1 * DVec3::from_array(tab.rig_c.head_ctr) + v2;
    v1 = kid_mw.transform_point3(v1);
    let (_v3, q3, _v4) = decompose_three(&kid_mw);
    let q2 = q3 * q1; // head world quaternion

    // ---- head motion tracking (L3812-3819)
    if !c.headInit || dt <= 0.0 {
        c.headPrevPos = v1;
        c.headPrevVel = DVec3::ZERO;
        c.headVel = DVec3::ZERO;
        c.headPrevQuat = q2;
        c.headInit = true;
        if dt <= 0.0 {
            return;
        }
    }

    // ---- apparent gravity in head space (L3820-3834)
    let mut v3 = (v1 - c.headPrevPos) / dt;
    if v3.length_squared() > 900.0 {
        v3 = v3.normalize() * 30.0;
    }
    let mut v4 = (v3 - c.headPrevVel) / dt;
    if v4.length() > 34.0 {
        v4 = v4.normalize() * 34.0;
    }
    let acc_k = 1.0 - (-24.0 * dt).exp();
    let vel_k = 1.0 - (-12.0 * dt).exp();
    c.headAcc = c.headAcc.lerp(v4, acc_k);
    c.headVel = c.headVel.lerp(v3, vel_k);
    c.headPrevPos = v1;
    c.headPrevVel = v3;
    let g = 9.8;
    let mut v5 = DVec3::new(0.0, -g, 0.0) - c.headAcc - c.headVel * 0.55;
    let q4 = q2.inverse();
    v5 = q4 * v5;
    v5.y += g; // gravity always acts in world space

    // ---- rotational kick from the head's angular delta (L3835-3842)
    let mut q5 = c.headPrevQuat.inverse() * q2;
    if q5.w < 0.0 {
        q5 = -q5;
    }
    let s_ang = 2.0 * clamp(q5.w, -1.0, 1.0).acos();
    let sn = (1e-12f64).max(1.0 - q5.w * q5.w).sqrt();
    let rx = (q5.x / sn) * s_ang;
    let ry = (q5.y / sn) * s_ang;
    let rz = (q5.z / sn) * s_ang;
    c.headRY = clamp(ry, -0.2, 0.2);
    c.headRZ = clamp(rz, -0.2, 0.2);
    c.headPrevQuat = q2;

    // ---- strand springs (L3843-3915)
    let t = c.t;
    let steps = ((dt * 60.0 - 0.25).ceil()).clamp(1.0, 4.0) as usize;
    let h = dt / steps as f64;
    let idle_l = (1.0 - c.gaitW) * (1.0 - c.wAir) * if c.kidForm { 1.0 } else { 0.0 };
    let wv = TAU * 0.42 * t;
    let gain = 0.5;
    let gain1 = 0.3;
    let gain2 = 0.2;
    let hk = 0.62;
    let hz = 0.3;
    let h_ine = 1.15;
    let hgk = 0.85;
    let h_cl = 0.68;
    let h_br = 0.6;

    let segs = tab.rig.hair_segs;
    let n_strands = tab.rig.hair_meta.len();
    // disjoint field borrows (JS mutates `hx/hv/px/pv` + bone quaternions)
    let hx = &mut c.hx;
    let hv = &mut c.hv;
    let tip_x = &mut c.tipX;
    let tip_v = &mut c.tipV;
    let bones = &mut c.bones;
    let hair_in = &c.hairIn;
    let hair_ax = &c.hairAx;
    let hair_a1 = &c.hairA1;
    let hair_a2 = &c.hairA2;
    let hair_ph = &c.hairPh;
    let hair_bones = &c.hairBones;
    let hair_tips = &c.hairTips;

    for si in 0..n_strands {
        let m = tab.rig.hair_meta[si];
        let (ux, uy, uz) = (m.dir[0], m.dir[1], m.dir[2]);
        // gravity / inertia in strand base space
        let mut tx = uy * v5.z - uz * v5.y;
        let mut ty = uz * v5.x - ux * v5.z;
        let mut tz = ux * v5.y - uy * v5.x;
        let gk = (m.g * 0.075 * hgk) * clamp(m.len / 0.22, 0.4, 1.6);
        tx *= gk;
        ty *= gk * 0.3;
        tz *= gk;
        let tl = (tx * tx + ty * ty + tz * tz).sqrt();
        if tl > 0.62 {
            tx *= 0.62 / tl;
            ty *= 0.62 / tl;
            tz *= 0.62 / tl;
        }
        // strand local wind (L3850-3853)
        let ix = hair_in[si * 3] as f64;
        let iy = hair_in[si * 3 + 1] as f64;
        let iz = hair_in[si * 3 + 2] as f64;
        let wx = uy * iz - uz * iy;
        let wy = uz * ix - ux * iz;
        let wz = ux * iy - uy * ix;
        let ww = wx * wx + wy * wy + wz * wz;
        // global breeze (L3858)
        let breeze = h_br
            * (0.03 * (t * 1.7 + si as f64 * 1.3).sin()
                + 0.012 * (t * 4.3 + si as f64 * 2.1).sin());
        let seg = si * segs;
        for k in 0..segs {
            let i = (seg + k) * 3;
            let gain_k = if k == 0 {
                gain
            } else if k == 1 {
                gain1
            } else {
                gain2
            };
            let ine = (if k == 0 {
                0.55
            } else if k == 1 {
                0.3
            } else {
                0.15
            }) * clamp(1.2 - m.k * 0.3, 0.3, 1.0)
                * h_ine;
            hx[i] = (hx[i] as f64 - rx * ine) as f32;
            hx[i + 1] = (hx[i + 1] as f64 - ry * ine * 0.5) as f32;
            hx[i + 2] = (hx[i + 2] as f64 - rz * ine) as f32;
            let kk = 170.0
                * hk
                * m.k
                * (if k == 0 {
                    1.0
                } else if k == 1 {
                    0.75
                } else {
                    0.55
                })
                / clamp(m.len / 0.22, 0.6, 1.6);
            let dd = 2.0 * hz * kk.sqrt();
            let mut gx = tx * gain_k + breeze * 0.3;
            let mut gy = ty * gain_k;
            let mut gz = tz * gain_k + breeze * 0.5;
            // idle micro-sway (L3872-3875)
            let j = (si * (segs + 1) + k) * 3;
            if idle_l > 0.01 {
                let w = idle_l
                    * 0.03
                    * (wv - 0.6 * k as f64 + hair_ph[si] as f64).sin()
                    * clamp(1.6 - m.k * 0.4, 0.3, 1.0);
                gx += (hair_a2[j] as f64 * 0.8 + hair_a1[j] as f64 * 0.35) * w;
                gy += (hair_a2[j + 1] as f64 * 0.8 + hair_a1[j + 1] as f64 * 0.35) * w;
                gz += (hair_a2[j + 2] as f64 * 0.8 + hair_a1[j + 2] as f64 * 0.35) * w;
            }
            for _n in 0..steps {
                hv[i] = (hv[i] as f64 + (kk * (gx - hx[i] as f64) - dd * hv[i] as f64) * h) as f32;
                hv[i + 1] = (hv[i + 1] as f64
                    + (kk * (gy - hx[i + 1] as f64) - dd * hv[i + 1] as f64) * h)
                    as f32;
                hv[i + 2] = (hv[i + 2] as f64
                    + (kk * (gz - hx[i + 2] as f64) - dd * hv[i + 2] as f64) * h)
                    as f32;
                hx[i] = (hx[i] as f64 + hv[i] as f64 * h) as f32;
                hx[i + 1] = (hx[i + 1] as f64 + hv[i + 1] as f64 * h) as f32;
                hx[i + 2] = (hx[i + 2] as f64 + hv[i + 2] as f64 * h) as f32;
            }
            // soft cone constraint along the strand root axis (L3884-3890)
            if ww > 1e-6 {
                let into = (hx[i] as f64 * wx + hx[i + 1] as f64 * wy + hx[i + 2] as f64 * wz) / ww;
                if into > 0.05 {
                    let e = into - 0.05;
                    hx[i] = (hx[i] as f64 - wx * e) as f32;
                    hx[i + 1] = (hx[i + 1] as f64 - wy * e) as f32;
                    hx[i + 2] = (hx[i + 2] as f64 - wz * e) as f32;
                    let vi =
                        (hv[i] as f64 * wx + hv[i + 1] as f64 * wy + hv[i + 2] as f64 * wz) / ww;
                    if vi > 0.0 {
                        hv[i] = (hv[i] as f64 - wx * vi) as f32;
                        hv[i + 1] = (hv[i + 1] as f64 - wy * vi) as f32;
                        hv[i + 2] = (hv[i + 2] as f64 - wz * vi) as f32;
                    }
                }
            }
            hx[i] = clamp(hx[i] as f64, -h_cl, h_cl) as f32;
            hx[i + 1] = clamp(hx[i + 1] as f64, -h_cl, h_cl) as f32;
            hx[i + 2] = clamp(hx[i + 2] as f64, -h_cl, h_cl) as f32;
            // twist limit around the bone's rest axis (L3892-3897)
            {
                let ax = hair_ax[j] as f64;
                let ay = hair_ax[j + 1] as f64;
                let az = hair_ax[j + 2] as f64;
                let tw = hx[i] as f64 * ax + hx[i + 1] as f64 * ay + hx[i + 2] as f64 * az;
                let e = tw - clamp(tw, -0.12, 0.12);
                hx[i] = (hx[i] as f64 - ax * e) as f32;
                hx[i + 1] = (hx[i + 1] as f64 - ay * e) as f32;
                hx[i + 2] = (hx[i + 2] as f64 - az * e) as f32;
                let tv = hv[i] as f64 * ax + hv[i + 1] as f64 * ay + hv[i + 2] as f64 * az;
                hv[i] = (hv[i] as f64 - ax * tv) as f32;
                hv[i + 1] = (hv[i + 1] as f64 - ay * tv) as f32;
                hv[i + 2] = (hv[i + 2] as f64 - az * tv) as f32;
            }
            if let Some(bone) = hair_bones[seg + k] {
                let (ax, ay, az) = (hx[i] as f64, hx[i + 1] as f64, hx[i + 2] as f64);
                let ang = (ax * ax + ay * ay + az * az).sqrt();
                if ang > 1e-6 {
                    bones[bone].quat =
                        DQuat::from_axis_angle(DVec3::new(ax / ang, ay / ang, az / ang), ang);
                } else {
                    bones[bone].quat = DQuat::IDENTITY;
                }
            }
        }
        // free-swinging tip (L3901-3913)
        if let Some(tip) = hair_tips[si] {
            let j = si * 3;
            let i2 = (si * segs + 2) * 3;
            tip_x[j] = (tip_x[j] as f64 - rx * 0.12) as f32;
            tip_x[j + 1] = (tip_x[j + 1] as f64 - ry * 0.06) as f32;
            tip_x[j + 2] = (tip_x[j + 2] as f64 - rz * 0.12) as f32;
            let kk = 95.0 * 0.7 * m.k / clamp(m.len / 0.22, 0.6, 1.6);
            let dd = 2.0 * 0.24 * kk.sqrt();
            let jt = (si * (segs + 1) + segs) * 3;
            let coil = idle_l * 0.08 * (wv * 0.95 + hair_ph[si] as f64 * 1.7 - 1.8).sin();
            let gx = hx[i2] as f64 * 0.45 + tx * 0.12 + breeze * 0.25 + hair_a1[jt] as f64 * coil;
            let gy = hx[i2 + 1] as f64 * 0.3 + ty * 0.1 + hair_a1[jt + 1] as f64 * coil;
            let gz = hx[i2 + 2] as f64 * 0.45
                + tz * 0.12
                + breeze * 0.35
                + hair_a1[jt + 2] as f64 * coil;
            for _n in 0..steps {
                tip_v[j] = (tip_v[j] as f64
                    + (kk * (gx - tip_x[j] as f64) - dd * tip_v[j] as f64) * h)
                    as f32;
                tip_v[j + 1] = (tip_v[j + 1] as f64
                    + (kk * (gy - tip_x[j + 1] as f64) - dd * tip_v[j + 1] as f64) * h)
                    as f32;
                tip_v[j + 2] = (tip_v[j + 2] as f64
                    + (kk * (gz - tip_x[j + 2] as f64) - dd * tip_v[j + 2] as f64) * h)
                    as f32;
                tip_x[j] = (tip_x[j] as f64 + tip_v[j] as f64 * h) as f32;
                tip_x[j + 1] = (tip_x[j + 1] as f64 + tip_v[j + 1] as f64 * h) as f32;
                tip_x[j + 2] = (tip_x[j + 2] as f64 + tip_v[j + 2] as f64 * h) as f32;
            }
            tip_x[j] = clamp(tip_x[j] as f64, -0.4, 0.4) as f32;
            tip_x[j + 1] = clamp(tip_x[j + 1] as f64, -0.4, 0.4) as f32;
            tip_x[j + 2] = clamp(tip_x[j + 2] as f64, -0.4, 0.4) as f32;
            {
                let ax = hair_ax[jt] as f64;
                let ay = hair_ax[jt + 1] as f64;
                let az = hair_ax[jt + 2] as f64;
                let tw = tip_x[j] as f64 * ax + tip_x[j + 1] as f64 * ay + tip_x[j + 2] as f64 * az;
                let e = tw - clamp(tw, -0.12, 0.12);
                tip_x[j] = (tip_x[j] as f64 - ax * e) as f32;
                tip_x[j + 1] = (tip_x[j + 1] as f64 - ay * e) as f32;
                tip_x[j + 2] = (tip_x[j + 2] as f64 - az * e) as f32;
            }
            let ang = (tip_x[j] as f64 * tip_x[j] as f64
                + tip_x[j + 1] as f64 * tip_x[j + 1] as f64
                + tip_x[j + 2] as f64 * tip_x[j + 2] as f64)
                .sqrt();
            if ang > 1e-6 {
                bones[tip].quat = DQuat::from_axis_angle(
                    DVec3::new(
                        tip_x[j] as f64 / ang,
                        tip_x[j + 1] as f64 / ang,
                        tip_x[j + 2] as f64 / ang,
                    ),
                    ang,
                );
            } else {
                bones[tip].quat = DQuat::IDENTITY;
            }
        }
    }
}
