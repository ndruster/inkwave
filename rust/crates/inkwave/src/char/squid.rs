//! Squid form + tank ink ≡ `character.js` `_updateSquid` (L3917-4019) and
//! `_updateTank` (L3737-3749).
//!
//! `_squidBlink` (L4021 → `_blinkTick`, B2 no-op) is skipped; the render-side
//! writes (`u.uTime` / `u.uWig` uniforms, `sq.eyes` / `sq.ghost` / `sq.dark`
//! visibility, `mats.squid`) are not part of the recorded pose (fixture
//! `meta.b2Noops` / harness dump).
#![allow(dead_code)] // Task 54 batch B1/C consumers land progressively
use std::f64::consts::PI;

use glam::{DMat3, DMat4, DQuat, DVec3};

use super::anim::{AnimState, CharAnim};
use super::math::*;

/// `THREE.Object3D.updateMatrix()` ≡ compose(T·R·S).
#[inline]
fn compose(pos: DVec3, quat: DQuat, scale: DVec3) -> DMat4 {
    DMat4::from_scale_rotation_translation(scale, quat, pos)
}

/// ≡ `_updateTank(dt)` (character.js L3737-3749).
pub fn update_tank(c: &mut CharAnim, dt: f64) {
    let ss = &c.tab.spr;
    // slosh driven by body acceleration (kid space), lagging and ringing like a liquid
    let tx = spr(
        &mut c.sp,
        ss.s_tank_x,
        clamp(c.kax * 0.004, -0.1, 0.1),
        1.6,
        0.14,
        dt,
    );
    let tz = spr(
        &mut c.sp,
        ss.s_tank_z,
        clamp(-c.kaz * 0.004, -0.1, 0.1),
        1.6,
        0.14,
        dt,
    );
    let tl = spr(&mut c.sp, ss.s_tank_l, 0.0, 2.4, 0.18, dt);
    c.slosh = (tx * tx + tz * tz).sqrt();
    // ≡ L3745: `f.rotation.set(clamp(tz), 0, clamp(tx))` — default 'XYZ' order
    c.tankFill.quat = euler_xyz(clamp(tz, -0.06, 0.06), 0.0, clamp(tx, -0.06, 0.06));
    let lvl = (c.inkS * (1.0 + clamp(tl * 0.06, -0.08, 0.08))).max(0.004);
    c.tankFill.scale = DVec3::new(1.0 - c.slosh * 0.03, lvl * c.tank.h, 1.0 - c.slosh * 0.03);
    // `f.visible = inkS > 0.005` (L3748): render-side only.
}

/// ≡ `_updateSquid(dt, s)` (character.js L3917-4019).
pub fn update_squid(c: &mut CharAnim, dt: f64, s: &AnimState) {
    let ss = &c.tab.spr;
    if c.sqScale <= 0.001 {
        c.sqInit = false;
        return;
    }
    let form = if c.form == "kid" {
        if c.formPrev == "kid" {
            "squid".to_string()
        } else {
            c.formPrev.clone()
        }
    } else {
        c.form.clone()
    };
    let t = c.t;
    let v = c.hs;
    let mut p;
    let q;
    let mut sy;
    let mut sxz;
    let airborne = !c.grounded;
    if form == "climb"
        && let Some(wn) = s.wall_normal
    {
        // belly (+Z) into the wall, mantle (+Y) up the wall, wiggling side to side
        let n = wn.normalize();
        let mut by = DVec3::Y - n * n.y; // UP.dot(n) = n.y
        if by.length_squared() < 1e-4 {
            by = DVec3::Z;
        }
        by = by.normalize();
        let bz = -n;
        let bx = by.cross(bz).normalize();
        let mut q2 = DQuat::from_mat3(&DMat3::from_cols(bx, by, bz));
        let climb_v = (c.vyS.abs() / 5.0 + v / 6.0).min(1.0);
        let q3 = DQuat::from_axis_angle(bz, (t * 11.0).sin() * 0.16 * climb_v);
        q2 = q3 * q2;
        // `model.matrixWorld = rootWorld · model.matrix` (L3938-3940)
        let root_w = compose(c.root_pos, DQuat::from_rotation_y(c.root_yaw), DVec3::ONE);
        let model_w = root_w * compose(c.model.pos, c.model.quat, c.model.scale);
        let (_v3, q3d, _v4) = decompose_three(&model_w);
        q = q3d.inverse() * q2;
        let mut v5 = c.root_pos
            + DVec3::Y * (0.26 + 0.025 * (t * 14.0).sin() * climb_v)
            + n * (-c.climbInset - 0.07);
        v5 += bx * (0.022 * (t * 5.5).sin() * climb_v);
        p = model_w.inverse().transform_point3(v5); // `model.worldToLocal`
        sy = 1.0 + 0.1 * climb_v + 0.05 * (t * 14.0).sin() * climb_v;
        sxz = 1.0 / sy.sqrt();
    } else if form == "swim" && !airborne {
        if v > 0.3 {
            c.sqYaw = damp_angle(c.sqYaw, c.mdx.atan2(c.mdz), 10.0, dt);
        }
        let sv = (v / 11.0).min(1.0);
        let und = (t * lerp(5.0, 14.0, sv)).sin();
        c.sqRoll = damp(c.sqRoll, clamp(-c.yawRate * 0.08, -0.5, 0.5), 6.0, dt);
        q = euler_yxz(
            PI / 2.0 + 0.06 * und * sv,
            c.sqYaw + 0.1 * (t * lerp(4.0, 11.0, sv) + 1.0).sin() * sv,
            c.sqRoll + und * 0.12 * (v / 4.0).min(1.0),
        );
        p = DVec3::new(0.0, -0.085 + 0.012 * (t * 5.0).sin(), 0.0);
        p += q * DVec3::new(0.0, -0.12, 0.0);
        sy = 1.0 + 0.2 * sv + 0.03 * und * sv;
        sxz = 1.0 / sy.sqrt();
    } else if airborne && (c.hs > 3.0 || form == "swim" || c.formPrev == "climb") {
        // dolphin arc: mantle follows the flight path, stretched along it
        let pit = c.vyS.atan2(c.hs.max(0.5));
        if v > 0.3 {
            c.sqYaw = damp_angle(c.sqYaw, c.mdx.atan2(c.mdz), 8.0, dt);
        }
        q = euler_yxz(PI / 2.0 - pit, c.sqYaw, 0.0);
        p = DVec3::new(0.0, 0.22, 0.0);
        let sv = (c.hs.hypot(c.vyS) / 10.0).min(1.0);
        sy = 1.0 + 0.22 * sv;
        sxz = 1.0 / sy.sqrt();
    } else {
        // dry squid: hops (anticipation squash → stretch → splat), or an idle bob
        let mut hop = 0.0;
        let mut tilt = 0.0;
        if !airborne && v > 0.25 {
            c.hopPhase += dt * (v / 0.72);
            let hp = frac(c.hopPhase);
            let k = (v / 1.5).min(1.0);
            let air_u = clamp((hp - 0.14) / 0.72, 0.0, 1.0);
            hop = 0.14 * (PI * air_u).sin() * k;
            tilt = (0.3 * (PI * air_u).cos() - 0.1) * k;
            let anti = if hp < 0.14 {
                (PI * hp / 0.14).sin()
            } else {
                0.0
            };
            let land = if hp > 0.86 {
                (PI * (hp - 0.86) / 0.14).sin()
            } else {
                0.0
            };
            sy = 1.0 - 0.2 * anti * k + 0.16 * (PI * air_u).sin() * k - 0.16 * land * k;
            if v > 0.3 {
                c.sqYaw = damp_angle(c.sqYaw, c.mdx.atan2(c.mdz), 10.0, dt);
            }
        } else if airborne {
            c.hopPhase = 0.0;
            sy = 1.0 + clamp(c.vyS.abs() * 0.022, 0.0, 0.24);
            tilt = clamp(-c.vyS * 0.03, -0.3, 0.3);
        } else {
            c.hopPhase = 0.0;
            sy = 1.0 + 0.035 * (t * 3.1).sin() + 0.012 * (t * 7.3).sin();
            c.sqYaw = damp_angle(c.sqYaw, 0.0, 3.0, dt);
        }
        let sqy = spr(&mut c.sp, ss.s_sq_y, 0.0, 5.0, 0.3, dt);
        let sqp = spr(&mut c.sp, ss.s_sq_p, 0.0, 4.0, 0.35, dt);
        sy *= 1.0 + clamp(sqy, -0.3, 0.3);
        tilt += sqp * 0.3;
        q = euler_yxz(tilt, c.sqYaw, 0.05 * (t * 2.3).sin());
        p = DVec3::new(0.0, 0.165 + hop, 0.0);
        sxz = 1.0 / sy.sqrt();
    }
    // transform gesture (≡ `_updateFormScales`): puddle → stretch → wobble in,
    // crouch → shoot up out.
    sy *= c.sqSY;
    sxz *= c.sqSXZ;
    c.squidPivot.scale = DVec3::new(sxz, sy, sxz);
    if !c.sqInit {
        c.sqPos = p;
        c.sqQuat = q;
        c.sqInit = true;
    }
    let k = 1.0 - (-dt * 26.0).exp();
    c.sqPos = c.sqPos.lerp(p, k);
    c.sqQuat = c.sqQuat.slerp(q, k);
    c.squidPivot.pos = c.sqPos;
    c.squidPivot.quat = c.sqQuat;
    let pop = c.sqScale;
    c.squidRoot.scale = DVec3::splat(pop.max(0.001));
    c.squidRoot.pos = DVec3::ZERO;
    // eyes / ghost / dark / materials (L4009-4017) + `_squidBlink` (L4018,
    // B2 no-op): render-side only, not recorded by the harness.
}
