//! Pose-layer assembly — ported from `character.js`: `_buildPose`
//! (L1756-1918), `_poseAir` (L1923-1971), `_poseWeapon` (L1975-2123).
//!
//! The remaining B2 hook in this file is none: `_poseDance` + the nine dance
//! clips are ported in full at the bottom (L3036-3301 anchors).
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively
use std::f64::consts::PI;

use super::anim::{AnimState, CharAnim, pose_neutral};
use super::math::*;
use super::tables::{Channels, HoldDef, HoldFrame, Tables};

// ---------------------------------------------------------------------------
// ≡ character.js L202-204: bank helpers (Float32 read-modify-write semantics)
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn set_e(p: &mut [f32], i: usize, x: f64, y: f64, z: f64) {
    p[i] = x as f32;
    p[i + 1] = y as f32;
    p[i + 2] = z as f32;
}

#[inline]
pub(crate) fn lerp_e(p: &mut [f32], i: usize, x: f64, y: f64, z: f64, w: f64) {
    p[i] = (p[i] as f64 + (x - p[i] as f64) * w) as f32;
    p[i + 1] = (p[i + 1] as f64 + (y - p[i + 1] as f64) * w) as f32;
    p[i + 2] = (p[i + 2] as f64 + (z - p[i + 2] as f64) * w) as f32;
}

/// `p[i] = v` — single-channel Float32 write (JS `D[i] = v`).
#[inline]
pub(crate) fn set_1(p: &mut [f32], i: usize, v: f64) {
    p[i] = v as f32;
}

#[inline]
pub(crate) fn add_e(p: &mut [f32], i: usize, v: f64) {
    p[i] = (p[i] as f64 + v) as f32;
}

#[inline]
pub(crate) fn sub_e(p: &mut [f32], i: usize, v: f64) {
    p[i] = (p[i] as f64 - v) as f32;
}

#[inline]
pub(crate) fn mul_e(p: &mut [f32], i: usize, v: f64) {
    p[i] = (p[i] as f64 * v) as f32;
}

/// `p[i] = lerp(p[i], target, w)` — single-channel Float32 read-modify-write.
#[inline]
pub(crate) fn lerp_1(p: &mut [f32], i: usize, target: f64, w: f64) {
    p[i] = (p[i] as f64 + (target - p[i] as f64) * w) as f32;
}

/// `p[i] = Math.max(p[i], v)` — single-channel Float32 read-modify-write.
#[inline]
pub(crate) fn max_1(p: &mut [f32], i: usize, v: f64) {
    if (p[i] as f64) < v {
        p[i] = v as f32;
    }
}

/// `mitPunch` (character.js L458): glove thrust envelope over time.
#[inline]
fn mit_punch(u: f64) -> f64 {
    if u < 0.045 {
        ease_out(u / 0.045)
    } else if u < 0.21 {
        1.0 - ease((u - 0.045) / 0.165)
    } else {
        0.0
    }
}

/// `mitOnWall` (character.js L459): push a kid-space point onto the wall plane
/// `p·n = wd`, `off` metres out from it; returns `[x, z]`.
#[inline]
fn mit_on_wall(x: f64, z: f64, off: f64, nx: f64, nz: f64, wd: f64) -> [f64; 2] {
    let d = x * nx + z * nz - wd - off;
    [x - nx * d, z - nz * d]
}

/// `poseLerp(out, a, b, t)` (L202): out[i] = a[i] + (b[i] - a[i]) * t —
/// written as an in-place pass (`out` aliases `a`) so Rust's borrow checker
/// stays happy.
fn pose_lerp_self(p: &mut [f32], x: &[f32], t: f64) {
    for i in 0..p.len() {
        let pi = p[i] as f64;
        p[i] = (pi + (x[i] as f64 - pi) * t) as f32;
    }
}

/// ≡ `_buildPose(dt, s)` (L1756-1918).
#[allow(non_snake_case)]
pub fn build_pose(c: &mut CharAnim, dt: f64, s: &AnimState) {
    let tab = c.tab;
    let ch = &tab.ch;
    let spr_ = &tab.spr;
    let tim = &tab.tim;
    // ---- scalars copied out of `c` before the bank borrows (≡ L1757-1764)
    let t = c.t;
    let gw = c.gaitW;
    let v = c.gs;
    let rw = c.runW * gw;
    let phase = c.phase;
    let duty = c.duty;
    let p_l = frac(phase);
    let air = c.wAir;
    let idle_w = (1.0 - gw) * (1.0 - air);
    let w_tired = c.wTired;
    let w_aim = c.wAim;
    let shift_tgt = c.shiftTgt;
    let foot_twist = c.footTwist;
    let mdz = c.mdz;
    let hip_twist = c.hipTwist;
    let w_goo = c.wGoo;
    let run_w = c.runW;
    let kgx = c.kgx;
    let kaz = c.kaz;
    let kax = c.kax;
    let gs = c.gs;
    let two = c.wTwo; // ≡ L1855 `const two = this.wTwo` (value from LAST frame)
    // ---- bank borrows (disjoint fields)
    let p = &mut c.P;
    let sp = &mut c.sp;
    let tr = &mut c.tr;
    let hv = &mut c.hv;

    super::anim::pose_neutral(tab, p);

    // ---- ready stance (L1766-1772)
    let rdy = idle_w * (1.0 - 0.35 * w_tired);
    sub_e(p, ch.HIPS_P + 1, 0.036 * idle_w);
    add_e(p, ch.HIPS, 0.07 * rdy);
    sub_e(p, ch.SPINE, 0.03 * rdy);
    sub_e(p, ch.CHEST, 0.012 * rdy);
    sub_e(p, ch.NECK, 0.02 * rdy);
    sub_e(p, ch.UARML, 0.1 * rdy);
    add_e(p, ch.UARML + 2, 0.07 * rdy);
    sub_e(p, ch.FARML, 0.32 * rdy);
    sub_e(p, ch.HANDL + 2, 0.12 * rdy);
    sub_e(p, ch.CLAVL + 2, 0.02 * rdy);
    add_e(p, ch.CLAVR + 2, 0.02 * rdy);

    // ---- weight shift (L1775-1785)
    let shift = spr(
        sp,
        spr_.s_shift,
        shift_tgt * idle_w * (1.0 - w_aim * 0.8) * (1.0 - w_tired * 0.3),
        0.7,
        0.85,
        dt,
    );
    add_e(p, ch.HIPS_P, 0.03 * shift);
    add_e(p, ch.HIPS + 2, 0.08 * shift);
    sub_e(p, ch.SPINE + 2, 0.05 * shift);
    sub_e(p, ch.CHEST + 2, 0.035 * shift);
    add_e(p, ch.HIPS + 1, 0.04 * shift);
    sub_e(p, ch.CHEST + 1, 0.025 * shift);
    add_e(p, ch.HEAD + 2, 0.035 * shift);
    sub_e(p, ch.HIPS_P + 1, 0.008 * shift.abs());
    c.shiftS = shift;
    let ms = idle_w * (1.0 - w_aim);
    add_e(p, ch.HIPS + 1, 0.018 * (t * 0.41 + 1.3).sin() * ms);
    add_e(p, ch.SPINE + 2, 0.012 * (t * 0.53).sin() * ms);
    add_e(p, ch.CHEST + 1, 0.02 * (t * 0.29 + 2.0).sin() * ms);
    let ft = foot_twist * (1.0 - gw);
    add_e(p, ch.HIPS + 1, ft * 0.55);
    sub_e(p, ch.SPINE + 1, ft * 0.3);
    sub_e(p, ch.CHEST + 1, ft * 0.25);

    // ---- locomotion (L1788-1816)
    if gw > 0.001 {
        let bk = sstep(0.1, -0.7, mdz);
        let rn = run_w;
        let yaw_osc = -lerp(0.11, 0.2, rn) * (TAU * p_l).cos() * gw * (1.0 - bk * 0.5);
        let roll_osc = lerp(0.06, 0.075, rn) * (TAU * (p_l - duty * 0.5)).cos() * gw;
        let sway_x = lerp(0.022, 0.013, rn) * (TAU * (p_l - duty * 0.5 - 0.06)).cos() * gw;
        let c2 = (TAU * 2.0 * (phase - duty * 0.45)).cos();
        let bob = lerp(0.014, -0.036, rn) * c2 * gw * sstep(0.3, 2.0, v);
        add_e(
            p,
            ch.HIPS_P + 1,
            gw * lerp(-0.024, -0.064, rn) * (1.0 + 0.5 * w_goo) + bob,
        );
        add_e(p, ch.HIPS_P, sway_x);
        add_e(p, ch.HIPS + 1, hip_twist + yaw_osc);
        add_e(p, ch.HIPS + 2, roll_osc);
        sub_e(p, ch.SPINE + 1, hip_twist * 0.45 + yaw_osc * 0.65);
        sub_e(p, ch.CHEST + 1, hip_twist * 0.55 + yaw_osc * 0.8);
        sub_e(p, ch.SPINE + 2, roll_osc * 0.6);
        sub_e(p, ch.CHEST + 2, roll_osc * 0.35);
        let lean =
            gw * (lerp(0.05, 0.3, rn) * (1.0 - bk * 1.3) * (1.0 - 0.55 * w_aim) + 0.14 * w_goo);
        add_e(p, ch.HIPS, lean * 0.3);
        add_e(p, ch.SPINE, lean * 0.45);
        add_e(p, ch.CHEST, lean * 0.25);
        let lat = clamp(kgx / 6.0, -1.0, 1.0) * gw * (1.0 - 0.3 * w_goo);
        sub_e(p, ch.SPINE + 2, 0.07 * lat);
        sub_e(p, ch.CHEST + 2, 0.04 * lat);
        sub_e(p, ch.HIPS + 2, 0.03 * lat);
        let sq2 = 0.024 * c2 * rn * gw;
        mul_e(p, ch.SQY, 1.0 - sq2);
        mul_e(p, ch.SQXZ, 1.0 + sq2 * 0.45);
        sub_e(p, ch.HEAD, 0.03 * c2 * rn * gw);
        add_e(p, ch.NECK, 0.02 * c2 * rn * gw);
    }

    // ---- lean springs (L1819-1833)
    {
        let af = clamp(kaz, -48.0, 48.0);
        let al = clamp(kax, -48.0, 48.0);
        let g = 1.0 - air * 0.75;
        let lp = spr(
            sp,
            spr_.s_lean_p,
            clamp(af * 0.0075, -0.36, 0.3) * g,
            2.2,
            0.4,
            dt,
        );
        let lr = spr(
            sp,
            spr_.s_lean_r,
            clamp(-al * 0.0068, -0.34, 0.34) * g,
            2.0,
            0.48,
            dt,
        );
        add_e(p, ch.HIPS, lp * 0.3);
        add_e(p, ch.SPINE, lp * 0.42);
        add_e(p, ch.CHEST, lp * 0.28);
        add_e(p, ch.HIPS + 2, lr * 0.4);
        add_e(p, ch.SPINE + 2, lr * 0.35);
        add_e(p, ch.CHEST + 2, lr * 0.25);
        let lag_z = spr(
            sp,
            spr_.s_lag_z,
            clamp(-kaz * 0.0012, -0.055, 0.07) * g,
            2.3,
            0.6,
            dt,
        );
        let lag_x = spr(
            sp,
            spr_.s_lag_x,
            clamp(-kax * 0.0012, -0.05, 0.05) * g,
            2.3,
            0.6,
            dt,
        );
        add_e(p, ch.HIPS_P + 2, lag_z);
        add_e(p, ch.HIPS_P, lag_x);
        sub_e(p, ch.HIPS_P + 1, lp.abs() * 0.06 + lr.abs() * 0.03);
        // ≡ L1832: hard braking skid (hair kick inlined — ≡ `_hairKick(0, 0, 3)` L1305-1312)
        if kaz < -26.0 && gs > 3.0 && (tr[tim.t_brake] as f64) > 0.5 {
            tr[tim.t_brake] = 0.0;
            sp[spr_.s_pely + 1] = (sp[spr_.s_pely + 1] as f64 - 0.55) as f32;
            sp[spr_.s_arm_l + 1] = (sp[spr_.s_arm_l + 1] as f64 - 5.0) as f32;
            sp[spr_.s_arm_r + 1] = (sp[spr_.s_arm_r + 1] as f64 - 1.5) as f32;
            // ≡ `_hairKick(0, 0, 3)`: x=0,y=0,z=3 → hv[i] += 3*0.6; the
            // y/z lines are exact zeros (hv[i+1] += x*0.4, hv[i+2] +=
            // y*0.5 - x*0.2) and are omitted.
            let mut i = 0;
            while i < hv.len() {
                hv[i] += 3.0 * 0.6;
                i += 3;
            }
            sp[spr_.s_tank_z + 1] = (sp[spr_.s_tank_z + 1] as f64 + 1.8) as f32;
        }
    }

    // ---- pelvis / squash / hit / stagger / clavicle springs (L1836-1847)
    let pel_y = spr(sp, spr_.s_pely, 0.0, 3.4, 0.42, dt);
    add_e(p, ch.HIPS_P + 1, clamp(pel_y, -0.2, 0.08));
    let sq = spr(sp, spr_.s_sq, 0.0, 4.8, 0.34, dt);
    mul_e(p, ch.SQY, 1.0 + clamp(sq, -0.22, 0.16));
    mul_e(p, ch.SQXZ, 1.0 - clamp(sq, -0.22, 0.16) * 0.5);
    let hp = spr(sp, spr_.s_hit_p, 0.0, 3.4, 0.38, dt);
    let hrl = spr(sp, spr_.s_hit_r, 0.0, 3.4, 0.38, dt);
    let hy = spr(sp, spr_.s_hit_y, 0.0, 3.4, 0.42, dt);
    add_e(p, ch.SPINE, hp * 0.5);
    add_e(p, ch.CHEST, hp * 0.6);
    add_e(p, ch.HIPS, hp * 0.2);
    add_e(p, ch.SPINE + 2, hrl * 0.5);
    add_e(p, ch.CHEST + 2, hrl * 0.5);
    add_e(p, ch.SPINE + 1, hy * 0.5);
    add_e(p, ch.CHEST + 1, hy * 0.6);
    let stag = spr(sp, spr_.s_stag, 0.0, 2.2, 0.5, dt);
    add_e(p, ch.HIPS_P + 2, hp * 0.04 + stag * 0.05);
    let clv = spr(sp, spr_.s_clav, 0.0, 3.5, 0.45, dt);
    add_e(p, ch.CLAVL + 2, clv * 0.12);
    sub_e(p, ch.CLAVR + 2, clv * 0.12);
    sub_e(p, ch.UARML + 2, clv * 0.08);
    add_e(p, ch.UARMR + 2, clv * 0.08);
    sub_e(p, ch.HIPS_P + 1, hp.abs() * 0.08);

    // ---- tired posture (L1850)
    if w_tired > 0.01 {
        let w = w_tired;
        add_e(p, ch.SPINE, 0.1 * w);
        add_e(p, ch.CHEST, 0.08 * w);
        add_e(p, ch.NECK, 0.05 * w);
        sub_e(p, ch.HIPS_P + 1, 0.02 * w);
        sub_e(p, ch.CLAVL + 2, 0.05 * w);
        add_e(p, ch.CLAVR + 2, 0.05 * w);
    }

    // ---- arm swing springs (L1854-1870)
    {
        let rn = run_w;
        let arm_a = gw * lerp(0.3, 0.95, rn) * (1.0 - 0.35 * w_goo);
        let tgt = arm_a * (TAU * (p_l - 0.03)).cos();
        let a_l = spr(sp, spr_.s_arm_l, tgt, lerp(3.2, 4.8, rn), 0.5, dt);
        let a_r = spr(sp, spr_.s_arm_r, -tgt, lerp(3.2, 4.8, rn), 0.5, dt);
        let fw = 1.0 - two;
        add_e(p, ch.UARML, a_l * fw);
        add_e(p, ch.UARMR, a_r);
        sub_e(
            p,
            ch.FARML,
            (gw * lerp(0.3, 1.45, rn) + 0.4 * (0.0f64).max(-a_l) * rn) * fw,
        );
        add_e(
            p,
            ch.UARML + 2,
            (0.05 + 0.13 * rw + 0.08 * a_l.max(0.0) * rn + 0.35 * w_goo * gw) * fw,
        );
        add_e(p, ch.UARML + 1, 0.18 * (0.0f64).max(-a_l) * rn * fw);
        add_e(p, ch.CLAVL + 1, 0.09 * a_l * fw);
        sub_e(p, ch.CLAVR + 1, 0.07 * a_r);
        sub_e(p, ch.CLAVL, 0.05 * (0.0f64).max(-a_l) * rn * fw);
        sub_e(p, ch.HANDL, 0.15 * rw * fw);
        p[ch.HANDPL] = (1.0 - 0.45 * rw) as f32;
        c.armR = a_r;
        c.armL = a_l;
    }

    // ---- air / weapon (L1873-1876)
    if air > 0.001 {
        pose_air(c, dt, air);
    }
    // ≡ L1876: unconditional — JS has no kid-form gate on the weapon hold.
    if c.weapon.is_some() {
        pose_weapon(c, dt, s);
    }
    // ≡ L1823: aiming re-enables the left-hand IK target
    if c.wAim > 0.001 && (c.weapon.is_some() || c.dual) {
        c.P[ch.IKR] = 1.0;
    }

    // ---- one-shots (L1879-1897)
    if (c.tr[tim.t_flick] as f64) < 0.7 {
        let ft = c.tr[tim.t_flick] as f64;
        if c.weaponKind == "brush" {
            pose_swipe(c, ft);
        } else if c.weaponKind == "bucket" {
            pose_bucket(c, ft);
        } else if c.animKind == "roller" {
            pose_flick(c, ft);
        }
    }
    if c.animKind == "blade" {
        pose_blade(c, dt, s);
    }
    if c.wSub > 0.001 && (c.tr[tim.t_throw] as f64) > 0.05 {
        let w = c.wSub;
        pose_sub_aim(c, w);
    }
    if (c.tr[tim.t_throw] as f64) < 0.62 {
        let tt = c.tr[tim.t_throw] as f64;
        pose_throw(c, tt);
    }
    if (c.tr[tim.t_spawn] as f64) < 1.4 {
        let st = c.tr[tim.t_spawn] as f64;
        pose_spawn(c, st);
    }
    if (c.tr[tim.t_leap] as f64) < 1.9 {
        let lt = c.tr[tim.t_leap] as f64;
        pose_leap(c, lt);
    }
    if (c.tr[tim.t_slam] as f64) < 1.4 {
        let st = c.tr[tim.t_slam] as f64;
        pose_slam(c, st);
    }
    if c.fidget >= 0 {
        let (id, ft) = (c.fidget, c.fidgetT);
        pose_fidget(c, id, ft);
    }
    if (c.tr[tim.t_land] as f64) < 0.8
        && c.landAmp > 0.3
        && c.kidForm
        && c.grounded
        && (c.tr[tim.t_spawn] as f64) > 1.4
    {
        let lt = c.tr[tim.t_land] as f64;
        pose_land(c, lt);
    }
    if c.formT < 0.5 {
        pose_form(c);
    }
    if (c.tr[tim.t_admire] as f64) < 1.6
        || (c.tr[tim.t_flip] as f64) < 1.0
        || (c.tr[tim.t_wink] as f64) < 0.8
    {
        pose_locker(c);
    }
    if (c.tr[tim.t_slosh] as f64) < 0.66 && c.hold.fire.as_deref() == Some("slosh") {
        let st = c.tr[tim.t_slosh] as f64;
        pose_slosh(c, st);
    }
    if (c.tr[tim.t_dodge] as f64) < c.dodgeDur + 0.3 {
        let tt = c.tr[tim.t_dodge] as f64;
        pose_dodge(c, tt);
    } else {
        c.tumble = 0.0;
        c.tumbleDrop = 0.0;
    }
    if c.weaponKind == "mitts" {
        pose_mitts(c, dt, s);
    }

    // ---- head look + face (L1900-1901)
    pose_look(c, dt, s);
    pose_face(c, dt, s);

    // ---- dances (L1904-1915): blend math is B1, the dance clips are B2
    if c.wDance > 0.001 || c.dance.is_some() {
        {
            let hold = c.hold;
            let dvar = c.danceVar;
            let anim_kind = c.animKind.as_str();
            let pd = &mut c.PD;
            pose_neutral(tab, pd);
            if let Some(d) = c.dance.clone() {
                let dt_now = c.danceT + c.danceOfs;
                pose_dance(tab, hold, anim_kind, dvar, pd, &d, dt_now);
                if c.danceFade < 1.0
                    && let Some(pd_name) = c.prevDance.clone()
                {
                    let px = &mut c.PY;
                    pose_neutral(tab, px);
                    let pt = c.prevDanceT + c.danceOfs;
                    pose_dance(tab, hold, anim_kind, dvar, px, &pd_name, pt);
                    // ≡ L1910: poseLerp(D, X, D, ease(danceFade))
                    let w = ease(c.danceFade);
                    for i in 0..px.len() {
                        let xi = px[i] as f64;
                        pd[i] = (xi + (pd[i] as f64 - xi) * w) as f32;
                    }
                }
                c.lastDance = Some(d);
            } else if let Some(ld) = c.lastDance.clone() {
                let dt_now = c.danceT + c.danceOfs;
                pose_dance(tab, hold, anim_kind, dvar, pd, &ld, dt_now);
            }
        }
        // ≡ L1914: poseLerp(P, P, D, ease(wDance))
        let w = ease(c.wDance);
        let pd = &c.PD;
        let p = &mut c.P;
        for i in 0..p.len() {
            let pi = p[i] as f64;
            p[i] = (pi + (pd[i] as f64 - pi) * w) as f32;
        }
    }

    // ---- life layer on top (L1917): B2 port (life_post, not a no-op)
    life_post(c, dt);
}

/// ≡ `_poseAir(P, dt, air)` (L1923-1971) — operates on the PX scratch copy.
#[allow(non_snake_case)]
fn pose_air(c: &mut CharAnim, dt: f64, air: f64) {
    let tab = c.tab;
    let ch = &tab.ch;
    let tim = &tab.tim;
    // ≡ L1924: X.set(P)
    c.PX.copy_from_slice(&c.P);
    let vy = c.vyS;
    let jt = c.tr[tim.t_jump] as f64;
    let up = sstep(-1.5, 4.0, vy);
    let launch = if jt < 0.3 {
        1.0 - sstep(0.03, 0.2, jt)
    } else {
        0.0
    };
    let apex = 1.0 - sstep(0.6, 3.2, vy.abs());
    let reach = (1.0 - up) * sstep(0.95, 0.15, c.gnd);
    let fall_long = sstep(0.4, 1.0, c.airT) * (1.0 - up) * (1.0 - reach);
    let lp = if jt < 1.2 { c.jumpRun } else { 0.0 };
    let ld = if c.jumpLead == 1.0 { -1.0 } else { 1.0 };
    let tk = (1.0 - launch) * apex.max(up * 0.85) * (1.0 - reach);
    let x = &mut c.PX;
    let t = c.t;
    let w_aim = c.wAim;
    // ≡ L1935-1936
    set_e(x, ch.FOOTL, 0.1, lerp(0.16, 0.34, tk), lerp(0.0, 0.07, tk));
    set_e(x, ch.FOOTLR, lerp(0.15, 0.55, tk), 0.16, 0.0);
    set_e(
        x,
        ch.FOOTR,
        -0.1,
        lerp(0.16, 0.31, tk),
        lerp(-0.03, 0.04, tk),
    );
    set_e(x, ch.FOOTRR, lerp(0.25, 0.6, tk), -0.16, 0.0);
    // ≡ L1939-1946: running leap
    if lp > 0.001 {
        let pass = sstep(0.1, -3.5, vy);
        let lf = if ld > 0.0 { ch.FOOTL } else { ch.FOOTR };
        let lr = if ld > 0.0 { ch.FOOTLR } else { ch.FOOTRR };
        let tf = if ld > 0.0 { ch.FOOTR } else { ch.FOOTL };
        let tr2 = if ld > 0.0 { ch.FOOTRR } else { ch.FOOTLR };
        let w = lp * (1.0 - reach * 0.7);
        lerp_e(
            x,
            lf,
            0.09 * ld,
            lerp(0.36, 0.16, pass),
            lerp(0.2, -0.12, pass),
            w,
        );
        lerp_e(x, lr, lerp(0.25, 0.9, pass), 0.12 * ld, 0.0, w);
        lerp_e(
            x,
            tf,
            -0.09 * ld,
            lerp(0.15, 0.33, pass),
            lerp(-0.27, 0.14, pass),
            w,
        );
        lerp_e(x, tr2, lerp(1.15, 0.3, pass), -0.12 * ld, 0.0, w);
        add_e(x, ch.HIPS + 1, 0.12 * ld * (1.0 - 2.0 * pass) * w);
        sub_e(x, ch.CHEST + 1, 0.1 * ld * (1.0 - 2.0 * pass) * w);
    }
    // ≡ L1948-1952: push-off, landing reach
    lerp_e(x, ch.FOOTL, 0.09, 0.07, -0.06 - 0.06 * lp, launch);
    lerp_e(x, ch.FOOTLR, 1.0, 0.08, 0.0, launch);
    lerp_e(x, ch.FOOTR, -0.09, 0.09, -0.1 - 0.06 * lp, launch);
    lerp_e(x, ch.FOOTRR, 1.1, -0.08, 0.0, launch);
    lerp_e(x, ch.FOOTL, 0.11, 0.1, 0.07, reach);
    lerp_e(x, ch.FOOTLR, -0.08, 0.14, 0.0, reach);
    lerp_e(x, ch.FOOTR, -0.11, 0.12, -0.03, reach);
    lerp_e(x, ch.FOOTRR, 0.05, -0.14, 0.0, reach);
    // ≡ L1953-1957
    x[ch.WPL] = 0.0;
    x[ch.WPR] = 0.0;
    x[ch.KNEEL] = 0.12;
    x[ch.KNEER] = -0.12;
    x[ch.HIPS_P + 1] = (-0.03 + 0.03 * up - 0.02 * tk - 0.03 * reach) as f32;
    add_e(x, ch.HIPS, 0.1 * tk - 0.06 * launch);
    add_e(x, ch.SPINE, 0.12 * tk - 0.1 * launch + 0.04 * reach);
    add_e(x, ch.CHEST, 0.05 * tk - 0.05 * launch);
    add_e(x, ch.HLP, -0.1 * launch + 0.12 * reach + 0.05 * tk);
    // ≡ L1960-1962: free arm + windmill
    let wm = (t * 10.0).sin() * 0.5 * fall_long;
    x[ch.UARML] = (lerp(lerp(-1.0, -0.5, up), -2.3, launch) + wm) as f32;
    x[ch.UARML + 2] = (lerp(lerp(1.25, 0.95, up), 0.4, launch) + 0.2 * apex) as f32;
    x[ch.FARML] = (lerp(-0.7, -0.45, up) - 0.4 * launch) as f32;
    x[ch.UARMR] = (lerp(lerp(-0.8, -0.35, up), -1.9, launch) - wm) as f32;
    x[ch.UARMR + 2] = (-lerp(lerp(1.15, 0.75, up), 0.3, launch)) as f32;
    x[ch.FARMR] = (lerp(-0.6, -0.4, up)) as f32;
    add_e(x, ch.CLAVL + 2, 0.1 * (1.0 - up) + 0.06 * launch);
    sub_e(x, ch.CLAVR + 2, 0.1 * (1.0 - up));
    // ≡ L1964-1969
    add_e(x, ch.ANC + 1, 0.03 * (1.0 - w_aim) * (apex + launch));
    sub_e(x, ch.ANCR, 0.2 * (1.0 - w_aim) * launch);
    x[ch.HANDPL] = (1.4 + 0.5 * (1.0 - up) + 0.1 * fall_long) as f32;
    add_e(x, ch.EARS, 0.7 * fall_long - 0.4 * launch + 0.4 * apex);
    mul_e(
        x,
        ch.SQY,
        1.0 + 0.09 * launch + 0.035 * sstep(-4.0, -12.0, vy) - 0.02 * apex,
    );
    mul_e(x, ch.SQXZ, 1.0 - 0.045 * launch);
    x[ch.MOPEN] = (x[ch.MOPEN] as f64).max(0.25 * apex + 0.45 * fall_long) as f32;
    add_e(x, ch.MCURVE, 0.25 * apex - 0.8 * fall_long);
    add_e(x, ch.EYE, 0.12 * apex + 0.15 * fall_long);
    add_e(x, ch.BROWY, 0.3 * apex + 0.6 * fall_long);
    sub_e(x, ch.LOOKY, 0.15 * reach);
    // ≡ L1970: poseLerp(P, P, X, air)
    let _ = dt;
    pose_lerp_self(&mut c.P, &c.PX, air);
}

/// ≡ `_poseWeapon(dt, s)` (L1975-2123).
#[allow(non_snake_case)]
fn pose_weapon(c: &mut CharAnim, dt: f64, s: &AnimState) {
    let tab = c.tab;
    let ch = &tab.ch;
    let spr_ = &tab.spr;
    let h = c.hold;
    let aim_pivot = tab.aim_pivot;
    let t = c.t;
    let phase = c.phase;
    let gw = c.gaitW;
    let run_w = c.runW;
    let w_aim = c.wAim;
    let w_roll = c.wRoll;
    let dual = c.dual;
    let lock_w = c.lockW;
    let charge = c.charge;
    let br_ph = c.brPh;
    let last_shot = c.lastShot;
    let anim_kind = c.animKind.clone();
    let weapon_kind = c.weaponKind.clone();
    let lab_stream = false; // ≡ this._labStream — labs-only, undefined in the harness
    let effort_old = c._effort;

    // ≡ L1977-1986
    let aim_p = clamp(s.aim_pitch, -1.0, 1.15);
    let aim_pose = clamp(aim_p, -0.8, 1.0);
    c.aimP = aim_p;
    let carry = h.carry;
    let one = 1.0 - h.two_carry;
    let sw_r = clamp(c.armR, -0.45, 0.45) * one;
    let sw_l = if dual {
        clamp(c.armL, -0.45, 0.45)
    } else {
        0.0
    };
    let sw_a = if dual { 0.0 } else { sw_r };

    let p = &mut c.P;
    let sp = &mut c.sp;

    // ≡ L1987-1988: carry
    set_e(
        p,
        ch.ANC,
        carry.p[0],
        carry.p[1] + 0.012 * (TAU * 2.0 * phase).sin() * gw,
        carry.p[2] - sw_a * 0.1,
    );
    set_e(p, ch.ANCR, carry.r[0] + sw_a * 0.55, carry.r[1], carry.r[2]);
    // ≡ L1990-1994: running carry
    let rk = gw * run_w;
    if rk > 0.001 {
        if let Some(ru) = h.run {
            lerp_e(
                p,
                ch.ANC,
                ru.p[0],
                ru.p[1] + 0.012 * (TAU * 2.0 * phase).sin(),
                ru.p[2] - sw_a * 0.08,
                rk,
            );
            lerp_e(p, ch.ANCR, ru.r[0] + sw_a * 0.4, ru.r[1], ru.r[2], rk);
        } else {
            add_e(p, ch.ANC + 1, 0.025 * rk);
            add_e(p, ch.ANC + 2, 0.04 * rk);
            sub_e(p, ch.ANCR, 0.18 * rk);
        }
    }
    // ≡ L1996-2027: aim
    if w_aim > 0.001 {
        let a = h.aim;
        let lk = if h.lock.is_some() { lock_w } else { 0.0 };
        let lock_p = h.lock.unwrap_or_default();
        let ax = lerp(a.p[0], if lk != 0.0 { lock_p.p[0] } else { 0.0 }, lk);
        let ay = lerp(a.p[1], if lk != 0.0 { lock_p.p[1] } else { 0.0 }, lk);
        let az = lerp(a.p[2], if lk != 0.0 { lock_p.p[2] } else { 0.0 }, lk);
        // ≡ L1999: _v1.set(ax,ay,az).applyAxisAngle(XAX, -aimPose).add(AIM_PIVOT)
        // (rotation about +X by -aimPose, expanded to avoid the f32-angle API)
        let (ca, sa) = (aim_pose.cos(), aim_pose.sin());
        let vv = glam::DVec3::new(ax, ay * ca + az * sa, -ay * sa + az * ca)
            + glam::DVec3::new(aim_pivot[0], aim_pivot[1], aim_pivot[2]);
        lerp_e(p, ch.ANC, vv.x, vv.y, vv.z, w_aim);
        lerp_e(
            p,
            ch.ANCR,
            a.r[0] - aim_p,
            lerp(a.r[1], if lk != 0.0 { lock_p.r[1] } else { 0.0 }, lk),
            lerp(a.r[2], if lk != 0.0 { lock_p.r[2] } else { 0.0 }, lk),
            w_aim,
        );
        lerp_e(p, ch.POLER, h.pole_r[0], h.pole_r[1], h.pole_r[2], w_aim);
        lerp_e(p, ch.POLEL, h.pole_l[0], h.pole_l[1], h.pole_l[2], w_aim);
        p[ch.AFOLR] = lerp(p[ch.AFOLR] as f64, 0.0, w_aim) as f32;
        let st = w_aim * (1.0 - 0.6 * gw);
        add_e(p, ch.HIPS + 1, h.hip * st);
        sub_e(p, ch.SPINE + 1, h.hip * 0.35 * st);
        add_e(p, ch.CHEST + 1, (h.chest - h.hip * 0.65) * st);
        sub_e(p, ch.SPINE, aim_pose * 0.1 * w_aim);
        sub_e(p, ch.CHEST, aim_pose * 0.18 * w_aim);
        sub_e(p, ch.HIPS_P + 1, h.crouch * st);
        sub_e(p, ch.NECK, aim_p * 0.08 * w_aim);
        // ≡ L2012-2020: charger breathing
        if anim_kind == "charger" {
            let bw = (1.0 - charge) * 0.012 * w_aim;
            add_e(p, ch.ANC + 1, (TAU * br_ph).sin() * bw);
            add_e(p, ch.ANC, (TAU * br_ph * 0.5 + 1.0).sin() * bw * 0.6);
            let tremble = if charge >= 0.99 {
                0.0025 * (t * 71.0).sin() + 0.0018 * (t * 53.0 + 1.0).sin()
            } else {
                0.0
            };
            add_e(p, ch.ANC + 1, tremble);
            add_e(p, ch.ANCR, tremble * 3.0);
            sub_e(p, ch.HEAD + 2, 0.12 * charge * w_aim);
            sub_e(p, ch.NECK + 2, 0.05 * charge * w_aim);
            sub_e(p, ch.HLY, 0.05 * charge * w_aim);
            sub_e(p, ch.HIPS_P + 1, 0.02 * charge * w_aim);
            add_e(p, ch.CHEST, 0.04 * charge * w_aim);
        }
        // ≡ L2022-2026: dualies post-roll lock
        if lk > 0.001 {
            sub_e(p, ch.HIPS_P + 1, 0.05 * lk);
            add_e(p, ch.HIPS, 0.1 * lk);
            add_e(p, ch.SPINE, 0.04 * lk);
            sub_e(p, ch.CHEST, 0.02 * lk);
            add_e(p, ch.HLP, 0.04 * lk);
            add_e(p, ch.KNEEL, 0.12 * lk);
            sub_e(p, ch.KNEER, 0.12 * lk);
            sub_e(p, ch.CLAVL + 1, 0.08 * lk);
            add_e(p, ch.CLAVR + 1, 0.08 * lk);
        }
    }
    // ≡ L2028: bow (not exercised by the pinned scenario; B2-adjacent layer)
    if weapon_kind == "bow" {
        pose_bow(dt);
    }
    // ≡ L2031-2058: splatling spin / stream
    if h.fire.as_deref() == Some("spin") {
        let r = s.runner;
        let charging = if let Some(r) = r {
            r.charging
        } else {
            s.firing && s.charge.unwrap_or(0.0) > 0.001 && !lab_stream
        };
        let streaming = if let Some(r) = r { r.streaming } else { false };
        let chg = if let Some(r) = r {
            clamp(r.charge, 0.0, 1.0)
        } else {
            clamp(s.charge.unwrap_or(0.0), 0.0, 1.0)
        };
        c.spinW = damp(
            c.spinW,
            if charging { 1.0 } else { 0.0 },
            if charging { 12.0 } else { 7.0 },
            dt,
        );
        c.streamW = damp(
            c.streamW,
            if streaming { 1.0 } else { 0.0 },
            if streaming { 16.0 } else { 5.0 },
            dt,
        );
        let cw = c.spinW * w_aim;
        let sw2 = c.streamW * w_aim;
        if cw > 0.001 {
            add_e(p, ch.ANCR, 0.42 * (1.0 - ease(chg)) * cw);
            sub_e(p, ch.ANC + 1, 0.03 * (1.0 - chg) * cw);
            sub_e(p, ch.ANC + 2, 0.02 * cw);
            sub_e(p, ch.SPINE, 0.07 * cw);
            sub_e(p, ch.CHEST, 0.05 * cw);
            sub_e(p, ch.HIPS_P + 2, 0.025 * cw);
            sub_e(p, ch.HIPS_P, 0.012 * cw);
            sub_e(p, ch.HIPS_P + 1, 0.012 * cw);
            add_e(p, ch.HLP, 0.03 * cw);
            let full = sstep(0.9, 1.0, chg) * cw;
            if full > 0.001 {
                let tr1 = 0.0035 * (t * 67.0).sin() + 0.0024 * (t * 91.0 + 1.7).sin();
                let tr2 = 0.0028 * (t * 59.0 + 0.6).sin();
                add_e(p, ch.ANC + 1, tr1 * full);
                add_e(p, ch.ANC, tr2 * full);
                add_e(p, ch.ANCR, tr1 * 3.0 * full);
                add_e(p, ch.CHEST + 2, tr2 * 2.0 * full);
            }
            c._effort = effort_old.max(0.35 + 0.5 * chg * cw);
        }
        if sw2 > 0.001 {
            add_e(p, ch.SPINE, 0.075 * sw2);
            add_e(p, ch.CHEST, 0.045 * sw2);
            sub_e(p, ch.HIPS_P + 2, 0.018 * sw2);
            sub_e(p, ch.HIPS_P + 1, 0.018 * sw2);
            sub_e(p, ch.ANC + 2, 0.012 * sw2);
            add_e(p, ch.NECK, 0.02 * sw2);
            add_e(p, ch.KNEEL, 0.08 * sw2);
            sub_e(p, ch.KNEER, 0.08 * sw2);
            let bf = if let Some(r) = r {
                r.burst_frac.unwrap_or(1.0).clamp(0.0, 1.0)
            } else {
                1.0
            };
            add_e(
                p,
                ch.ANCR,
                (0.008 * (t * 5.3).sin() + 0.004 * (t * 11.1).sin()) * sw2 * bf,
            );
            c._effort = c._effort.max(0.7 * sw2);
        }
    }
    // ≡ L2060-2070: roller push
    if w_roll > 0.001
        && let Some(a) = h.roll
    {
        lerp_e(p, ch.ANC, a.p[0], a.p[1], a.p[2], w_roll);
        lerp_e(p, ch.ANCR, a.r[0], a.r[1], a.r[2], w_roll);
        p[ch.AFOLT] = lerp(p[ch.AFOLT] as f64, 0.45, w_roll) as f32;
        p[ch.AFOLR] = lerp(p[ch.AFOLR] as f64, 0.15, w_roll) as f32;
        add_e(p, ch.SPINE, 0.17 * w_roll);
        add_e(p, ch.CHEST, 0.1 * w_roll);
        add_e(p, ch.HIPS, 0.08 * w_roll);
        sub_e(p, ch.HIPS_P + 1, h.crouch * w_roll);
        sub_e(p, ch.HIPS_P + 2, 0.02 * w_roll);
        add_e(p, ch.HLP, 0.05 * w_roll);
        lerp_e(p, ch.POLER, -0.7, -0.35, -0.7, w_roll);
        lerp_e(p, ch.POLEL, 0.7, -0.35, -0.7, w_roll);
        add_e(p, ch.HIPS + 1, h.hip * w_roll * (1.0 - gw));
    }
    // ≡ L2072-2080: two-handed IK + reach integration
    c.wTwo = if dual {
        0.0
    } else {
        lerp(h.two_carry, h.two_aim, w_aim).max(if h.roll.is_some() { 1.0 } else { 0.0 })
    };
    p[ch.IKL] = (if dual { 1.0 } else { c.wTwo }) as f32;
    sub_e(p, ch.CLAVL + 1, 0.28 * c.wTwo);
    add_e(p, ch.CLAVR + 1, 0.1 * c.wTwo);
    add_e(p, ch.CLAVL + 2, 0.04 * c.wTwo);
    let le = c.ikErrPre * if c.wTwo > 0.5 { 1.0 } else { 0.0 };
    c.lReach = clamp(
        c.lReach + (if le > 0.004 { le * 30.0 } else { -0.5 }) * dt,
        0.0,
        0.55,
    );
    sub_e(p, ch.CLAVL + 1, c.lReach);
    sub_e(p, ch.CHEST + 1, c.lReach * 0.35);
    sub_e(p, ch.CLAVL, c.lReach * 0.3);
    if c.wTwo > 0.0 {
        mul_e(p, ch.UARML, 1.0 - c.wTwo);
    }
    // ≡ L2083-2097: sustained-fire brace
    {
        let br = h.rc.brace;
        let want = if br > 0.0 && s.firing && last_shot < 0.22 && anim_kind != "roller" {
            1.0
        } else {
            0.0
        };
        if want == 0.0 && c._fireWant != 0.0 {
            sp[spr_.s_rc_p + 1] = (sp[spr_.s_rc_p + 1] as f64 - 1.1 * br * c.fireHold) as f32;
        }
        c._fireWant = want;
        c.fireHold = damp(c.fireHold, want, if want > 0.0 { 9.0 } else { 4.0 }, dt);
        let fh = c.fireHold * w_aim * br;
        if fh > 0.001 {
            add_e(p, ch.CHEST, 0.055 * fh);
            add_e(p, ch.SPINE, 0.02 * fh);
            sub_e(p, ch.HIPS_P + 1, 0.012 * fh);
            add_e(p, ch.NECK, 0.02 * fh);
            add_e(
                p,
                ch.ANCR,
                (0.012 * (t * 4.1 + 1.0).sin() + 0.006 * (t * 9.7).sin()) * fh,
            );
            add_e(p, ch.ANCR + 1, 0.01 * (t * 3.3).sin() * fh);
            sub_e(p, ch.ANC + 2, 0.012 * fh);
        }
        let pk = c.weapon.as_ref().map(|w| w.pump).unwrap_or(0.0);
        if pk != 0.0 {
            add_e(p, ch.ANCR, 0.06 * pk);
            add_e(p, ch.CHEST + 1, 0.05 * pk * w_aim);
            add_e(p, ch.SPINE + 1, 0.02 * pk * w_aim);
            sub_e(p, ch.ANC + 2, 0.01 * pk);
        }
    }
    // ≡ L2099-2105: dual wield left anchor
    if dual {
        p[ch.ANL] = -p[ch.ANC];
        p[ch.ANL + 1] = p[ch.ANC + 1];
        p[ch.ANL + 2] = p[ch.ANC + 2];
        p[ch.ANLR] = p[ch.ANCR];
        p[ch.ANLR + 1] = -p[ch.ANCR + 1];
        p[ch.ANLR + 2] = -p[ch.ANCR + 2];
        let cw = 1.0 - w_aim;
        sub_e(p, ch.ANC + 2, sw_r * 0.1 * cw);
        add_e(p, ch.ANCR, sw_r * 0.55 * cw);
        sub_e(p, ch.ANL + 2, sw_l * 0.1 * cw);
        add_e(p, ch.ANLR, sw_l * 0.55 * cw);
    }
    // ≡ L2107-2122: recoil springs
    {
        let rc = h.rc;
        let rp = spr_a(sp, spr_.s_rc_p, 0.0, rc.hz, rc.z, dt);
        let rz = spr_a(sp, spr_.s_rc_z, 0.0, rc.hz, rc.z, dt);
        spr_a(sp, spr_.s_rc_y, 0.0, rc.hz * 1.3, 0.5, dt);
        spr_a(sp, spr_.s_rc_r, 0.0, rc.hz * 1.3, 0.5, dt);
        sub_e(p, ch.CHEST, rp * rc.torso);
        sub_e(p, ch.SPINE, rp * rc.torso * 0.4);
        sub_e(p, ch.HEAD, rp * rc.head);
        sub_e(p, ch.HIPS_P + 2, rz * 0.35);
        sub_e(p, ch.HIPS_P + 1, rz.abs() * rc.crouch * 8.0);
        sub_e(p, ch.CLAVR + 1, rz * 1.5);
        c.rcP = rp;
        c.rcZ = rz;
        if dual {
            let rp2 = spr_a(sp, spr_.s_rc_p2, 0.0, rc.hz, rc.z, dt);
            let rz2 = spr_a(sp, spr_.s_rc_z2, 0.0, rc.hz, rc.z, dt);
            spr_a(sp, spr_.s_rc_y2, 0.0, rc.hz * 1.3, 0.5, dt);
            sub_e(p, ch.CHEST, rp2 * rc.torso);
            sub_e(p, ch.SPINE, rp2 * rc.torso * 0.4);
            sub_e(p, ch.HEAD, rp2 * rc.head);
            sub_e(p, ch.HIPS_P + 2, rz2 * 0.35);
            add_e(p, ch.CLAVL + 1, rz2 * 1.5);
            add_e(p, ch.CHEST + 1, (rz2 - rz) * 0.6);
            c.rcP2 = rp2;
            c.rcZ2 = rz2;
        }
    }
}

// ---------------------------------------------------------------------------
// B2 one-shot pose layers (≡ character.js L2159-2731).
// ---------------------------------------------------------------------------

/// ≡ `_poseFlick` (L2159): roller flick — coil, whip, follow-through, recover.
fn pose_flick(c: &mut CharAnim, ft: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let k_up = ease(ft / 0.15);
    let k_whip = ease((ft - 0.15) / 0.08);
    let k_fol = ease_out((ft - 0.23) / 0.14);
    let k_rec = ease((ft - 0.42) / 0.26);
    let mut ax = -0.11;
    let mut ay = 0.84;
    let mut az = 0.18;
    let mut rx = 0.8;
    let mut ry = 0.1;
    ax = lerp(ax, -0.13, k_up);
    ay = lerp(ay, 1.22, k_up);
    az = lerp(az, -0.02, k_up);
    rx = lerp(rx, -2.3, k_up);
    ry = lerp(ry, 0.3, k_up);
    ax = lerp(ax, -0.07, k_whip);
    ay = lerp(ay, 0.98, k_whip);
    az = lerp(az, 0.3, k_whip);
    rx = lerp(rx, -0.1, k_whip);
    ry = lerp(ry, 0.05, k_whip);
    ax = lerp(ax, -0.06, k_fol);
    ay = lerp(ay, 0.74, k_fol);
    az = lerp(az, 0.3, k_fol);
    rx = lerp(rx, 0.75, k_fol);
    let w = 1.0 - k_rec;
    let coil = k_up * (1.0 - k_whip);
    let whip = k_whip * (1.0 - k_rec);
    {
        let x = &mut c.PX;
        lerp_e(x, ch.ANC, ax, ay, az, w);
        lerp_e(x, ch.ANCR, rx, ry, 0.0, w);
        lerp_1(x, ch.AFOLT, 0.6, w);
        lerp_1(x, ch.AFOLR, 0.25, w);
        add_e(x, ch.SPINE, -0.22 * coil + 0.34 * whip);
        add_e(x, ch.CHEST, -0.14 * coil + 0.2 * whip);
        add_e(x, ch.SPINE + 1, 0.2 * coil - 0.12 * whip);
        add_e(x, ch.CHEST + 1, 0.18 * coil - 0.1 * whip);
        sub_e(x, ch.HIPS_P + 1, 0.03 * coil + 0.05 * whip);
        add_e(x, ch.HIPS_P + 2, -0.02 * coil + 0.03 * whip);
        add_e(x, ch.HLP, 0.06 * coil - 0.1 * whip);
        lerp_e(x, ch.POLER, -0.8, 0.1, -0.3, coil);
        lerp_e(x, ch.POLEL, 0.8, 0.1, -0.3, coil);
    }
    c._effort = c._effort.max(coil + whip * 0.7);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, 1.0);
}

/// ≡ `_poseBucket` (L2184): two-handed underarm pail throw.
fn pose_bucket(c: &mut CharAnim, ft: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let k_back = ease(ft / 0.11);
    let k_fwd = ease((ft - 0.09) / 0.1);
    let k_fol = ease_out((ft - 0.19) / 0.12);
    let k_rec = ease((ft - 0.34) / 0.24);
    let mut ax = -0.1;
    let mut ay = 0.7;
    let mut az = 0.24;
    let mut rx = 0.25;
    ax = lerp(ax, -0.2, k_back);
    ay = lerp(ay, 0.48, k_back);
    az = lerp(az, -0.1, k_back);
    rx = lerp(rx, 0.95, k_back);
    ax = lerp(ax, -0.08, k_fwd);
    ay = lerp(ay, 0.9, k_fwd);
    az = lerp(az, 0.42, k_fwd);
    rx = lerp(rx, -0.5, k_fwd);
    ay = lerp(ay, 0.98, k_fol);
    az = lerp(az, 0.46, k_fol);
    rx = lerp(rx, -0.85, k_fol);
    let w = 1.0 - k_rec;
    let back = k_back * (1.0 - k_fwd);
    let heave = k_fwd * (1.0 - k_rec);
    {
        let x = &mut c.PX;
        lerp_e(x, ch.ANC, ax, ay, az, w);
        lerp_e(x, ch.ANCR, rx, 0.05, 0.0, w);
        add_e(x, ch.SPINE, 0.3 * back - 0.12 * heave);
        add_e(x, ch.CHEST, 0.14 * back - 0.1 * heave);
        add_e(x, ch.SPINE + 1, 0.16 * back - 0.06 * heave);
        add_e(x, ch.CHEST + 1, 0.12 * back);
        sub_e(x, ch.HIPS_P + 1, 0.07 * back - 0.02 * heave);
        add_e(x, ch.HIPS_P + 2, -0.04 * back + 0.05 * heave);
        add_e(x, ch.HLP, 0.08 * back - 0.06 * heave);
        lerp_e(x, ch.POLER, -0.8, -0.7, 0.1, back);
        lerp_e(x, ch.POLEL, 0.8, -0.7, 0.1, back);
    }
    c._effort = c._effort.max(back * 0.6 + heave);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, 1.0);
}

/// ≡ `_poseSwipe` (L2207): brush sideways swipe.
fn pose_swipe(c: &mut CharAnim, ft: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let sd = if c.swipeSide != 0.0 { c.swipeSide } else { 1.0 };
    let k = ease(ft / 0.13);
    let k_rec = ease((ft - 0.15) / 0.2);
    let u = lerp(-sd, sd, k);
    let w = 1.0 - k_rec;
    {
        let x = &mut c.PX;
        lerp_e(x, ch.ANC, -0.06 + 0.3 * u, 0.78, 0.28, w);
        lerp_e(x, ch.ANCR, 0.9, 0.95 * u, -0.2 * u, w);
        add_e(x, ch.SPINE + 1, 0.3 * u * w);
        add_e(x, ch.CHEST + 1, 0.24 * u * w);
        add_e(x, ch.HIPS + 1, 0.1 * u * w);
        add_e(x, ch.SPINE, 0.14 * w);
        add_e(x, ch.CHEST, 0.06 * w);
        sub_e(x, ch.HIPS_P + 1, 0.03 * w);
        lerp_e(x, ch.POLER, -0.7, -0.35, -0.7, w);
        lerp_e(x, ch.POLEL, 0.7, -0.35, -0.7, w);
    }
    c._effort = c._effort.max(0.5 * w);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, 1.0);
}

/// ≡ `_poseBlade` (L2229): cutlass cuts + charge raise.
fn pose_blade(c: &mut CharAnim, dt: f64, s: &AnimState) {
    let ch = &c.tab.ch;
    c.blT += dt;
    let bt = c.blT;
    let sd = if c.blSide != 0.0 { c.blSide } else { 1.0 };
    let heavy = c.blHeavy;
    let charge = clamp(s.charge.unwrap_or(0.0), 0.0, 1.0);
    let up = charge > 0.001 && c.kidForm && c.dance.is_none();
    c.blRaise = damp(
        c.blRaise,
        if up { 1.0 } else { 0.0 },
        if up { 18.0 } else { 9.0 },
        dt,
    );
    let rw = c.blRaise * (if heavy && bt < 0.45 { 0.0 } else { 1.0 });
    let cut_w = if heavy {
        if bt < 0.6 {
            1.0 - ease((bt - 0.26) / 0.3)
        } else {
            0.0
        }
    } else if bt < 0.4 {
        ease(bt / 0.03) * (1.0 - ease((bt - 0.17) / 0.2))
    } else {
        0.0
    };
    if rw < 0.001 && cut_w < 0.001 {
        return;
    }
    c.PX.copy_from_slice(&c.P);
    let t = c.t;
    {
        let x = &mut c.PX;
        if rw > 0.001 {
            let k = ease(charge);
            let trem = if charge >= 0.999 {
                0.004 * (t * 63.0).sin() + 0.003 * (t * 41.0 + 1.0).sin()
            } else {
                0.0
            };
            lerp_e(
                x,
                ch.ANC,
                -0.2,
                lerp(1.16, 1.3, k) + trem,
                lerp(0.05, -0.03, k),
                rw,
            );
            lerp_e(
                x,
                ch.ANCR,
                lerp(-1.75, -2.35, k) + trem * 4.0,
                lerp(0.12, 0.22, k),
                lerp(0.05, 0.22, k),
                rw,
            );
            lerp_1(x, ch.AFOLT, 0.55, rw);
            lerp_1(x, ch.AFOLR, 0.25, rw);
            sub_e(x, ch.SPINE + 1, 0.16 * k * rw);
            sub_e(x, ch.CHEST + 1, 0.18 * k * rw);
            sub_e(x, ch.HIPS + 1, 0.05 * rw);
            sub_e(x, ch.SPINE, 0.05 * k * rw);
            sub_e(x, ch.CHEST, 0.05 * rw);
            sub_e(x, ch.HIPS_P + 1, 0.025 * k * rw);
            lerp_e(x, ch.POLER, -0.95, 0.15, -0.35, rw);
            c._effort = c._effort.max((0.3 + 0.6 * k) * rw);
        }
        if cut_w > 0.001 {
            let w = cut_w;
            let ax;
            let ay;
            let az;
            let rx;
            let ry;
            let rz;
            if !heavy {
                let k = ease_out(clamp((bt - 0.015) / 0.095, 0.0, 1.0));
                let f0 = if sd > 0.0 { -1.3 } else { 1.15 };
                let f1 = if sd > 0.0 { 1.15 } else { -1.3 };
                let f = lerp(f0, f1, k);
                ax = -0.08 + 0.34 * f.sin();
                az = 0.06 + 0.3 * f.cos();
                ay = if sd > 0.0 {
                    1.03 - 0.13 * k
                } else {
                    0.91 + 0.11 * k
                };
                ry = f * 1.08;
                rx = if sd > 0.0 {
                    lerp(-0.18, 0.28, k)
                } else {
                    lerp(0.24, -0.16, k)
                };
                rz = sd * std::f64::consts::FRAC_PI_2;
                let u = lerp(-sd, sd, k);
                add_e(x, ch.SPINE + 1, 0.26 * u * w);
                add_e(x, ch.CHEST + 1, 0.24 * u * w);
                add_e(x, ch.HIPS + 1, 0.1 * u * w);
                add_e(x, ch.SPINE, 0.08 * w);
                sub_e(x, ch.HIPS_P + 1, 0.02 * w);
                add_e(x, ch.CLAVR + 1, 0.12 * u.max(0.0) * w);
                lerp_e(x, ch.POLER, -0.8, -0.55, -0.1, w);
                c._effort = c._effort.max(0.55 * w);
            } else {
                let k1 = ease(bt / 0.06);
                let k2 = ease_out((bt - 0.055) / 0.07);
                ax = lerp(lerp(-0.2, -0.1, k1), -0.07, k2);
                ay = lerp(lerp(1.3, 1.08, k1), 0.62, k2);
                az = lerp(lerp(-0.03, 0.42, k1), 0.47, k2);
                rx = lerp(lerp(-2.35, -0.15, k1), 0.95, k2);
                ry = lerp(lerp(0.22, 0.08, k1), 0.04, k2);
                rz = lerp(0.22, 0.0, k1);
                let lg = ease(bt / 0.08) * (1.0 - ease((bt - 0.3) / 0.25));
                add_e(x, ch.SPINE, 0.2 * lg);
                add_e(x, ch.CHEST, 0.1 * lg);
                add_e(x, ch.HIPS, 0.06 * lg);
                sub_e(x, ch.NECK, 0.1 * lg);
                sub_e(x, ch.HIPS_P + 1, 0.045 * lg);
                add_e(x, ch.HIPS_P + 2, 0.04 * lg);
                add_e(x, ch.SPINE + 1, 0.12 * lg);
                add_e(x, ch.CHEST + 1, 0.08 * lg);
                add_e(x, ch.KNEEL, 0.05 * lg);
                sub_e(x, ch.KNEER, 0.05 * lg);
                lerp_e(x, ch.POLER, -0.85, -0.45, -0.3, w);
                c._effort = c._effort.max(0.9 * w);
            }
            lerp_e(x, ch.ANC, ax, ay, az, w);
            lerp_e(x, ch.ANCR, rx, ry, rz, w);
            lerp_1(x, ch.AFOLT, 0.35, w);
            lerp_1(x, ch.AFOLR, 0.12, w);
        }
    }
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, 1.0);
}

/// ≡ `_poseSubAim` (L2438): bomb held up behind the head.
fn pose_sub_aim(c: &mut CharAnim, w: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    {
        let x = &mut c.PX;
        x[ch.IKL] = 0.0;
        x[ch.LTW] = 0.0;
        x[ch.UARML] = -2.55;
        x[ch.UARML + 2] = 0.48;
        x[ch.FARML] = -1.85;
        x[ch.HANDL] = 0.2;
        add_e(x, ch.CLAVL + 2, 0.15);
        sub_e(x, ch.CLAVL + 1, 0.08);
        add_e(x, ch.CHEST + 1, 0.34);
        add_e(x, ch.SPINE + 1, 0.12);
        sub_e(x, ch.SPINE, 0.06);
        sub_e(x, ch.HLY, 0.25);
        x[ch.HANDPL] = 0.0;
    }
    c._effort = c._effort.max(0.35 * w);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseThrow` (L2421): bomb/storm throw with the free arm.
fn pose_throw(c: &mut CharAnim, tt: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let w = win(tt, 0.0, 0.03, 0.4, 0.62);
    let cock = 1.0 - ease(tt / 0.06);
    let whip = ease(tt / 0.1) * (1.0 - ease((tt - 0.1) / 0.16));
    let fol = ease((tt - 0.08) / 0.14);
    {
        let x = &mut c.PX;
        x[ch.IKL] = 0.0;
        x[ch.LTW] = 0.0;
        x[ch.UARML] = (-2.6 * cock + -1.35 * whip + -0.55 * fol * (1.0 - whip)) as f32;
        x[ch.UARML + 2] = (0.45 * cock + 0.1 * whip - 0.1 * fol) as f32;
        x[ch.FARML] = (-1.9 * cock - 0.3 * whip - 0.4 * fol) as f32;
        x[ch.HANDL] = (0.4 * whip) as f32;
        add_e(x, ch.CLAVL + 2, 0.16 * cock);
        add_e(x, ch.CLAVL + 1, -0.1 * cock + 0.12 * whip);
        add_e(x, ch.CHEST + 1, 0.4 * cock - 0.42 * whip - 0.18 * fol);
        add_e(x, ch.SPINE + 1, 0.16 * cock - 0.18 * whip);
        add_e(x, ch.SPINE, -0.1 * cock + 0.22 * whip + 0.08 * fol);
        add_e(x, ch.CHEST, 0.1 * whip);
        add_e(x, ch.HIPS_P + 2, 0.03 * whip);
        sub_e(x, ch.HIPS_P + 1, 0.025 * whip);
        x[ch.HANDPL] = lerp(1.95, 1.05, ease((tt - 0.12) / 0.35)) as f32;
    }
    c._effort = c._effort.max(whip);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseSpawn` (L2450): superhero dive → three-point landing → pop up.
fn pose_spawn(c: &mut CharAnim, st: f64) {
    let ch = &c.tab.ch;
    let tim = &c.tab.tim;
    c.PX.copy_from_slice(&c.P);
    let lt_land = c.tr[tim.t_land] as f64;
    let landed = lt_land < st;
    let lt = if landed { lt_land } else { -1.0 };
    if !landed {
        let k = ease(st / 0.12);
        {
            let x = &mut c.PX;
            set_e(x, ch.FOOTL, 0.07, 0.07, -0.02);
            set_e(x, ch.FOOTLR, 0.9, 0.05, 0.0);
            set_e(x, ch.FOOTR, -0.07, 0.09, -0.06);
            set_e(x, ch.FOOTRR, 1.0, -0.05, 0.0);
            x[ch.WPL] = 0.0;
            x[ch.WPR] = 0.0;
            x[ch.UARML] = -2.3;
            x[ch.UARML + 2] = 0.55;
            x[ch.FARML] = -0.2;
            sub_e(x, ch.SPINE, 0.08);
            sub_e(x, ch.HLP, 0.2);
            x[ch.HANDPL] = 1.7;
            add_e(x, ch.EARS, 0.8);
            mul_e(x, ch.SQY, 1.1);
            mul_e(x, ch.SQXZ, 0.95);
        }
        let px = &c.PX;
        let p = &mut c.P;
        pose_lerp_self(p, px, k);
        return;
    }
    let crouch = win(lt, 0.0, 0.05, 0.3, 0.55);
    let pop = win(lt, 0.3, 0.42, 0.5, 0.8);
    {
        let x = &mut c.PX;
        sub_e(x, ch.HIPS_P + 1, 0.24 * crouch);
        sub_e(x, ch.HIPS_P + 2, 0.02 * crouch);
        add_e(x, ch.SPINE, 0.42 * crouch);
        add_e(x, ch.CHEST, 0.18 * crouch);
        add_e(x, ch.HLP, 0.25 * crouch);
        sub_e(x, ch.NECK, 0.2 * crouch);
        x[ch.LTW] = crouch as f32;
        set_e(x, ch.LTGT, 0.2, 0.08, 0.2);
        mul_e(x, ch.IKL, 1.0 - crouch);
        lerp_e(x, ch.POLEL, 1.0, 0.2, 0.0, crouch);
        add_e(x, ch.KNEEL, 0.25 * crouch);
        sub_e(x, ch.KNEER, 0.25 * crouch);
        sub_e(x, ch.UARMR, 0.4 * crouch);
        mul_e(x, ch.SQY, 1.0 + 0.08 * pop);
        mul_e(x, ch.SQXZ, 1.0 - 0.04 * pop);
        lerp_1(x, ch.UARML, -1.2, pop);
        lerp_1(x, ch.UARML + 2, 1.2, pop);
        lerp_1(x, ch.FARML, -0.3, pop);
        sub_e(x, ch.SPINE, 0.08 * pop);
        add_e(x, ch.HLP, 0.08 * pop);
        lerp_1(x, ch.HANDPL, 2.0, crouch);
        lerp_1(x, ch.HANDPL, 1.8, pop);
        add_e(x, ch.EARS, 0.9 * pop - 0.4 * crouch);
    }
    let w = 1.0 - ease((lt - 0.75) / 0.3);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseLeap` (L2484): Tidal Slam launch/tuck/hang.
fn pose_leap(c: &mut CharAnim, lt: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let launch = win(lt, 0.0, 0.04, 0.12, 0.24);
    let tuck = win(lt, 0.12, 0.26, 0.42, 0.56);
    let hang = win(lt, 0.44, 0.58, 1.6, 1.9);
    {
        let x = &mut c.PX;
        x[ch.WPL] = 0.0;
        x[ch.WPR] = 0.0;
        lerp_e(x, ch.FOOTL, 0.08, 0.08, -0.05, launch);
        lerp_e(x, ch.FOOTLR, 1.0, 0.1, 0.0, launch);
        lerp_e(x, ch.FOOTR, -0.08, 0.1, -0.1, launch);
        lerp_e(x, ch.FOOTRR, 1.1, -0.1, 0.0, launch);
        lerp_e(x, ch.FOOTL, 0.1, 0.4, 0.14, tuck);
        lerp_e(x, ch.FOOTLR, 0.7, 0.1, 0.0, tuck);
        lerp_e(x, ch.FOOTR, -0.1, 0.36, 0.1, tuck);
        lerp_e(x, ch.FOOTRR, 0.8, -0.1, 0.0, tuck);
        lerp_e(x, ch.FOOTL, 0.12, 0.3, 0.1, hang);
        lerp_e(x, ch.FOOTLR, 0.4, 0.15, 0.0, hang);
        lerp_e(x, ch.FOOTR, -0.12, 0.2, -0.12, hang);
        lerp_e(x, ch.FOOTRR, 0.9, -0.15, 0.0, hang);
        x[ch.HIPS_P + 1] = (-0.03 * tuck) as f32;
        add_e(x, ch.SPINE, -0.14 * launch + 0.5 * tuck - 0.22 * hang);
        add_e(x, ch.CHEST, -0.08 * launch + 0.25 * tuck - 0.16 * hang);
        add_e(x, ch.HLP, 0.2 * launch - 0.1 * tuck - 0.35 * hang);
        add_e(x, ch.NECK, 0.12 * hang);
        mul_e(x, ch.SQY, 1.0 + 0.12 * launch);
        mul_e(x, ch.SQXZ, 1.0 - 0.06 * launch);
        let flip = ease((lt - 0.14) / 0.36);
        if lt > 0.14 {
            add_e(x, ch.MODELR, wrap_a(flip * TAU));
        }
        let up = launch.max(hang);
        lerp_e(x, ch.ANC, -0.06, 1.34, 0.06, up);
        lerp_e(x, ch.ANCR, -1.85, 0.05, 0.0, up);
        lerp_e(x, ch.ANC, -0.1, 0.86, 0.2, tuck);
        lerp_e(x, ch.ANCR, -0.6, 0.1, 0.0, tuck);
        x[ch.AFOLT] = 1.0;
        x[ch.AFOLR] = (0.5 + 0.5 * tuck) as f32;
        lerp_1(x, ch.IKL, 1.0, hang.max(tuck));
        lerp_1(x, ch.IKL, 0.0, launch);
        lerp_1(x, ch.UARML, -2.7, launch);
        lerp_1(x, ch.UARML + 2, 0.25, launch);
        lerp_e(x, ch.POLER, -0.9, 0.3, -0.2, up);
        lerp_e(x, ch.POLEL, 0.9, 0.3, -0.2, up);
        x[ch.STAB] = 0.3;
        lerp_1(x, ch.HANDPL, -1.0, launch);
        add_e(x, ch.EARS, 0.8 * (launch + hang));
    }
    c._effort = c._effort.max(launch + hang);
    let mut w = win(lt, 0.0, 0.04, 1.55, 1.9);
    if c.leapEnd >= 0.0 {
        w *= 1.0 - ease((lt - c.leapEnd) / 0.1);
    }
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseSlam` (L2521): dive + impact crouch with the weapon planted.
fn pose_slam(c: &mut CharAnim, st: f64) {
    let ch = &c.tab.ch;
    let tim = &c.tab.tim;
    let spr_ = &c.tab.spr;
    if !c.slamGround && c.grounded && st > 0.02 {
        c.slamGround = true;
        c.tr[tim.t_impact] = 0.0;
        sub_e(&mut c.sp, spr_.s_sq + 1, 3.2);
        sub_e(&mut c.sp, spr_.s_pely + 1, 0.8);
        c.hair_kick(0.0, -3.0, 0.0);
    }
    let it = if c.slamGround {
        c.tr[tim.t_impact] as f64
    } else {
        -1.0
    };
    let dive = if c.slamGround { 0.0 } else { ease(st / 0.07) };
    let imp = if it >= 0.0 {
        win(it, 0.0, 0.02, 0.35, 0.8)
    } else {
        0.0
    };
    let roller = c.animKind == "roller";
    let ankle_h = c.tab.rig_c.ankle_h;
    c.PX.copy_from_slice(&c.P);
    {
        let x = &mut c.PX;
        x[ch.WPL] = 0.0;
        x[ch.WPR] = 0.0;
        lerp_e(x, ch.FOOTL, 0.1, 0.1, 0.06, dive);
        lerp_e(x, ch.FOOTLR, 0.5, 0.1, 0.0, dive);
        lerp_e(x, ch.FOOTR, -0.1, 0.12, -0.08, dive);
        lerp_e(x, ch.FOOTRR, 0.7, -0.1, 0.0, dive);
        let dw = dive * (1.0 - ease(st / 0.12));
        lerp_e(x, ch.ANC, -0.06, 1.3, 0.12, dw);
        lerp_e(x, ch.ANCR, -1.9, 0.05, 0.0, dw);
        let fw = ease((st - 0.04) / 0.1) * (1.0 - imp);
        lerp_e(x, ch.ANC, -0.05, 0.62, 0.36, fw);
        lerp_e(x, ch.ANCR, 0.9, 0.05, 0.0, fw);
        add_e(x, ch.SPINE, 0.3 * dive);
        add_e(x, ch.CHEST, 0.15 * dive);
        lerp_e(x, ch.FOOTL, 0.2, ankle_h, 0.06, imp);
        lerp_e(x, ch.FOOTLR, 0.0, 0.35, 0.0, imp);
        lerp_e(x, ch.FOOTR, -0.2, ankle_h, -0.1, imp);
        lerp_e(x, ch.FOOTRR, 0.0, -0.5, 0.0, imp);
        sub_e(x, ch.HIPS_P + 1, 0.25 * imp);
        add_e(x, ch.SPINE, 0.45 * imp);
        add_e(x, ch.CHEST, 0.25 * imp);
        add_e(x, ch.HLP, 0.2 * imp);
        lerp_e(x, ch.ANC, -0.05, if roller { 0.5 } else { 0.36 }, 0.4, imp);
        lerp_e(x, ch.ANCR, if roller { 1.2 } else { 1.35 }, 0.05, 0.0, imp);
        add_e(x, ch.KNEEL, 0.3 * imp);
        sub_e(x, ch.KNEER, 0.3 * imp);
        x[ch.IKL] = 1.0;
        x[ch.AFOLT] = 1.0;
        x[ch.AFOLR] = 0.6;
        sub_e(x, ch.EARS, 0.6 * dive);
        x[ch.HANDPL] = -1.0;
        lerp_e(x, ch.POLER, -0.9, -0.2, -0.3, imp);
        lerp_e(x, ch.POLEL, 0.9, -0.2, -0.3, imp);
    }
    c._effort = c._effort.max(dive + imp);
    let w = win(st, 0.0, 0.03, 1.0, 1.4);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseLand` (L2288): heavy-landing absorb.
fn pose_land(c: &mut CharAnim, lt: f64) {
    let ch = &c.tab.ch;
    c.PX.copy_from_slice(&c.P);
    let a = c.landAmp;
    let k = win(lt, 0.0, 0.035, 0.07 + 0.13 * a, 0.3 + 0.38 * a) * sstep(0.3, 0.75, a);
    if k <= 0.001 {
        return;
    }
    let big = sstep(0.7, 0.95, a) * (1.0 - c.wAim) * (1.0 - c.wTwo);
    {
        let x = &mut c.PX;
        sub_e(x, ch.HIPS_P + 1, 0.11 * a);
        sub_e(x, ch.HIPS_P + 2, 0.02 * a);
        add_e(x, ch.HIPS, 0.14 * a);
        add_e(x, ch.SPINE, 0.26 * a);
        add_e(x, ch.CHEST, 0.1 * a);
        add_e(x, ch.HLP, 0.12 * a);
        add_e(x, ch.NECK, 0.08 * a);
        add_e(x, ch.KNEEL, 0.16 * a);
        sub_e(x, ch.KNEER, 0.16 * a);
        lerp_1(x, ch.UARML, -0.35, 0.8);
        lerp_1(x, ch.UARML + 2, 0.95, 0.8);
        lerp_1(x, ch.FARML, -0.25, 0.8);
        lerp_1(x, ch.HANDPL, 2.0, 0.8);
        max_1(x, ch.LTW, big);
        set_e(x, ch.LTGT, 0.24, 0.1, 0.2);
        mul_e(x, ch.IKL, 1.0 - big);
        lerp_e(x, ch.POLEL, 1.0, 0.2, 0.0, big);
        sub_e(x, ch.HIPS_P + 1, 0.05 * big);
        add_e(x, ch.SPINE, 0.12 * big);
        add_e(x, ch.SQUINT, 0.5 * a);
        sub_e(x, ch.MCURVE, 0.3 * a);
        sub_e(x, ch.EARS, 0.7 * a);
    }
    c._effort = c._effort.max(0.5 * a * k);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, k);
}

/// ≡ `_poseForm` (L2340): transform in/out acting.
fn pose_form(c: &mut CharAnim) {
    let ch = &c.tab.ch;
    let spr_ = &c.tab.spr;
    let t = c.formT;
    if c.form != "kid" && c.formPrev == "kid" {
        if t > 0.1 {
            return;
        }
        let k = ease(t / 0.05);
        let fw = 1.0 - c.wTwo;
        let p = &mut c.P;
        lerp_1(p, ch.UARML, -2.4, k * fw);
        lerp_1(p, ch.UARML + 2, 0.35, k * fw);
        lerp_1(p, ch.FARML, -0.2, k * fw);
        add_e(p, ch.SPINE, 0.2 * k);
        add_e(p, ch.CHEST, 0.08 * k);
        add_e(p, ch.HLP, 0.22 * k);
        sub_e(p, ch.HIPS_P + 1, 0.05 * k);
        sub_e(p, ch.EARS, 0.8 * k);
        lerp_1(p, ch.HANDPL, 2.0, k);
        sub_e(p, ch.EYE, 0.45 * k);
        add_e(p, ch.MCURVE, 0.35 * k);
    } else if c.form == "kid" && c.formPrev != "kid" {
        if t >= 0.045 && t - c._dt < 0.045 {
            c.hx.fill(0.0);
            c.hv.fill(0.0);
            c.tipX.fill(0.0);
            c.tipV.fill(0.0);
            c.hair_kick(0.0, 4.2, 0.6);
            add_e(&mut c.sp, spr_.s_tank_l + 1, 2.2);
            add_e(&mut c.sp, spr_.s_ear_l + 1, 5.0);
            add_e(&mut c.sp, spr_.s_ear_r + 1, 5.0);
        }
        let fl = win(t, 0.045, 0.1, 0.17, 0.42);
        let fw = 1.0 - c.wTwo;
        let na = 1.0 - c.wAim;
        let p = &mut c.P;
        lerp_1(p, ch.UARML, -1.75, fl * fw);
        lerp_1(p, ch.UARML + 2, 1.0, fl * fw);
        lerp_1(p, ch.FARML, -0.35, fl * fw);
        lerp_1(p, ch.HANDPL, 2.0, fl);
        add_e(p, ch.EARS, 1.0 * fl);
        sub_e(p, ch.SPINE, 0.08 * fl);
        sub_e(p, ch.CHEST, 0.05 * fl);
        sub_e(p, ch.HLP, 0.12 * fl);
        add_e(p, ch.ANC + 1, 0.04 * fl * na);
        sub_e(p, ch.ANCR, 0.3 * fl * na);
        add_e(p, ch.MCURVE, 0.5 * fl);
        max_1(p, ch.MOPEN, 0.35 * fl);
        add_e(p, ch.BROWY, 0.6 * fl);
    }
}

/// ≡ `_poseLocker` — no-op (hard constraint: stays harness-patched).
fn pose_locker(_c: &mut CharAnim) {}

/// ≡ `_poseSlosh` (L2365): slosher heave.
fn pose_slosh(c: &mut CharAnim, st: f64) {
    let ch = &c.tab.ch;
    let spr_ = &c.tab.spr;
    let tab = c.tab;
    c.PX.copy_from_slice(&c.P);
    let dx = kc(st, &tab.k_sl_t, &tab.k_sl_x);
    let dy = kc(st, &tab.k_sl_t, &tab.k_sl_y);
    let dz = kc(st, &tab.k_sl_t, &tab.k_sl_z);
    let rp = kc(st, &tab.k_sl_t, &tab.k_sl_p);
    let ry = kc(st, &tab.k_sl_t, &tab.k_sl_w);
    let wu = win(st, 0.0, 0.1, 0.12, 0.2);
    let hv = win(st, 0.12, 0.2, 0.3, 0.46);
    let rel = win(st, 0.06, 0.13, 0.36, 0.56);
    {
        let x = &mut c.PX;
        add_e(x, ch.ANC, dx);
        add_e(x, ch.ANC + 1, dy);
        add_e(x, ch.ANC + 2, dz);
        add_e(x, ch.ANCR, rp);
        add_e(x, ch.ANCR + 1, ry);
        max_1(x, ch.AFOLR, 0.35);
        add_e(x, ch.HIPS_P + 1, -0.045 * wu + 0.022 * hv);
        add_e(x, ch.HIPS_P + 2, -0.02 * wu + 0.025 * hv);
        add_e(x, ch.SPINE, 0.14 * wu - 0.13 * hv);
        add_e(x, ch.CHEST, 0.06 * wu - 0.1 * hv);
        add_e(x, ch.HIPS, 0.06 * wu);
        add_e(x, ch.SPINE + 1, 0.12 * wu - 0.1 * hv);
        add_e(x, ch.CHEST + 1, 0.22 * wu - 0.18 * hv);
        add_e(x, ch.HLP, -0.08 * wu + 0.1 * hv);
        max_1(x, ch.TIPTOE, 0.85 * hv);
        add_e(x, ch.KNEEL, 0.1 * wu);
        sub_e(x, ch.KNEER, 0.1 * wu);
        mul_e(x, ch.IKL, 1.0 - rel);
        lerp_1(x, ch.UARML, 0.15 + 0.75 * hv - 0.2 * wu, rel);
        lerp_1(x, ch.UARML + 2, 0.25 + 0.2 * hv, rel);
        lerp_1(x, ch.FARML, -0.5 - 0.3 * hv, rel);
        lerp_1(x, ch.HANDPL, 1.6, rel);
        mul_e(x, ch.SQY, 1.0 - 0.03 * wu + 0.045 * hv);
        mul_e(x, ch.SQXZ, 1.0 + 0.015 * wu - 0.02 * hv);
        add_e(x, ch.EARS, 0.5 * hv - 0.3 * wu);
    }
    c._effort = c._effort.max(0.6 * wu + hv);
    if st >= 0.12 && st - c._dt < 0.12 {
        add_e(&mut c.sp, spr_.s_tank_l + 1, 2.2);
        c.hair_kick(0.0, 2.4, 1.6);
        add_e(&mut c.sp, spr_.s_pely + 1, 0.35);
    }
    let w = win(st, 0.0, 0.015, 0.5, 0.66);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseDodge` (L2393): dualies dodge roll.
fn pose_dodge(c: &mut CharAnim, tt: f64) {
    let ch = &c.tab.ch;
    let tab = c.tab;
    let sl = tab.stance_lock;
    let ankle_h = tab.rig_c.ankle_h;
    c.PX.copy_from_slice(&c.P);
    let d = c.dodgeDur;
    let u = clamp(tt / d, 0.0, 1.0);
    let tuck = win(u, 0.0, 0.1, 0.62, 0.96);
    let out = sstep(0.6, 1.0, u);
    {
        let x = &mut c.PX;
        lerp_1(x, ch.WPL, 0.0, tuck);
        lerp_1(x, ch.WPR, 0.0, tuck);
        lerp_e(x, ch.FOOTL, 0.1, 0.38, 0.12, tuck);
        lerp_e(x, ch.FOOTLR, 1.15, 0.25, 0.0, tuck);
        lerp_e(x, ch.FOOTR, -0.1, 0.35, 0.08, tuck);
        lerp_e(x, ch.FOOTRR, 1.25, -0.25, 0.0, tuck);
        let ow = out * (1.0 - tuck);
        lerp_e(x, ch.FOOTL, sl[0], ankle_h, sl[1], ow);
        lerp_e(x, ch.FOOTR, sl[3], ankle_h, sl[4], ow);
        lerp_1(x, ch.FOOTLR + 1, sl[2], out);
        lerp_1(x, ch.FOOTRR + 1, sl[5], out);
        add_e(x, ch.KNEEL, 0.25 * tuck);
        sub_e(x, ch.KNEER, 0.25 * tuck);
        sub_e(x, ch.HIPS_P + 1, 0.05 * tuck + 0.05 * out * (1.0 - tuck));
        add_e(x, ch.SPINE, 0.6 * tuck);
        add_e(x, ch.CHEST, 0.35 * tuck);
        add_e(x, ch.NECK, 0.25 * tuck);
        add_e(x, ch.HLP, 0.4 * tuck);
        mul_e(x, ch.STAB, 1.0 - 0.85 * tuck);
        lerp_e(x, ch.ANC, -0.09, 0.93, 0.17, tuck);
        lerp_e(x, ch.ANCR, -0.5, 0.35, 0.4, tuck);
        lerp_e(x, ch.ANL, 0.09, 0.93, 0.17, tuck);
        lerp_e(x, ch.ANLR, -0.5, -0.35, -0.4, tuck);
        lerp_1(x, ch.AFOLT, 1.0, tuck);
        lerp_1(x, ch.AFOLR, 1.0, tuck);
        lerp_e(x, ch.POLER, -0.9, -0.2, -0.3, tuck);
        lerp_e(x, ch.POLEL, 0.9, -0.2, -0.3, tuck);
        mul_e(x, ch.SQY, 1.0 - 0.07 * tuck + 0.05 * out * (1.0 - out));
        mul_e(x, ch.SQXZ, 1.0 + 0.035 * tuck);
        sub_e(x, ch.EARS, 0.7 * tuck);
        add_e(x, ch.SQUINT, 0.7 * tuck);
        sub_e(x, ch.MCURVE, 0.5 * tuck);
        sub_e(x, ch.BROW, 0.3 * tuck);
    }
    c._effort = c._effort.max(tuck);
    c.tumble = if u < 1.0 {
        TAU * (1.0 - (1.0 - u).powf(2.4))
    } else {
        0.0
    };
    c.tumbleX = c.dodgeZ;
    c.tumbleZ = -c.dodgeX;
    c.tumbleDrop = 0.26 * tuck;
    let w = win(tt, 0.0, 0.015, d, d + 0.28);
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `_poseMitts` (L2556): Sponge Mitts body motion. The leap-charge / leap /
/// landing / wall-cling branches read `R.kit.mitts` from the runner; the
/// headless harness never provides a runner (`_runner(s)` → null → `K = null`
/// in JS), so only the punch branch can execute there.
fn pose_mitts(c: &mut CharAnim, _dt: f64, _s: &AnimState) {
    let ch = &c.tab.ch;
    let tim = &c.tab.tim;
    let kid = c.kidForm && c.dance.is_none();
    let e_r = if kid {
        mit_punch(c.tr[tim.t_shoot] as f64)
    } else {
        0.0
    };
    let e_l = if kid {
        mit_punch(c.tr[tim.t_shoot_l] as f64)
    } else {
        0.0
    };
    if e_r + e_l > 0.001 {
        let ap = clamp(c.aimP, -0.8, 1.0);
        let fy = ap.sin() * 0.24;
        let fz = ap.cos() * 0.24;
        let p = &mut c.P;
        add_e(p, ch.ANC, 0.07 * e_r);
        add_e(p, ch.ANC + 1, fy * e_r);
        add_e(p, ch.ANC + 2, fz * e_r);
        sub_e(p, ch.ANL, 0.07 * e_l);
        add_e(p, ch.ANL + 1, fy * e_l);
        add_e(p, ch.ANL + 2, fz * e_l);
        add_e(p, ch.ANCR + 1, 0.22 * e_r);
        sub_e(p, ch.ANCR + 2, 0.45 * e_r);
        sub_e(p, ch.ANLR + 1, 0.22 * e_l);
        add_e(p, ch.ANLR + 2, 0.45 * e_l);
        let tw = e_r - e_l;
        add_e(p, ch.CHEST + 1, 0.3 * tw);
        add_e(p, ch.SPINE + 1, 0.12 * tw);
        add_e(p, ch.HIPS + 1, 0.05 * tw);
        add_e(p, ch.CLAVR + 1, 0.22 * e_r);
        sub_e(p, ch.CLAVL + 1, 0.22 * e_l);
        add_e(p, ch.SPINE, 0.035 * (e_r + e_l));
        c._effort = c._effort.max(0.45 * e_r.max(e_l));
    }
}

/// ≡ `_poseFidget` (L2650): idle one-offs.
fn pose_fidget(c: &mut CharAnim, id: i64, ft: f64) {
    let ch = &c.tab.ch;
    let spr_ = &c.tab.spr;
    let tab = c.tab;
    c.PX.copy_from_slice(&c.P);
    let l = tab.fidget_len[id as usize];
    let w = win(ft, 0.0, 0.22, l - 0.28, l);
    let kind = c.animKind.clone();
    let name = tab.fidget_names[id as usize].as_str();
    let mut kick: Option<(f64, f64, f64)> = None;
    {
        let x = &mut c.PX;
        match name {
            "goggles" => {
                let a = kf(ft, &tab.k_gog_t, &tab.k_gog_v);
                let jig = (0.0f64.max(ft - 0.4) * 22.0).sin() * win(ft, 0.4, 0.5, 0.85, 0.95);
                x[ch.LTW] = a as f32;
                set_e(x, ch.LTGT, 0.075, 1.18 + 0.008 * jig, 0.2);
                mul_e(x, ch.IKL, 1.0 - a);
                lerp_e(x, ch.POLEL, 0.9, -0.6, -0.3, a);
                add_e(x, ch.HLP, 0.08 * a);
                add_e(x, ch.HEAD + 2, 0.08 * a);
                add_e(x, ch.SQUINT, 0.5 * a);
                sub_e(x, ch.MCURVE, 0.3 * a);
                lerp_1(x, ch.HANDPL, 0.3, a);
            }
            "twirl" => {
                if kind == "shooter" || kind == "blaster" || kind == "dualies" {
                    let k = ease((ft - 0.2) / 0.6);
                    let lift = win(ft, 0.1, 0.3, 0.75, 1.0);
                    x[ch.SPIN] = wrap_a(TAU * 2.0 * k) as f32;
                    if !c.dual {
                        x[ch.IKL] = 0.0;
                    }
                    add_e(x, ch.ANC + 1, 0.08 * lift);
                    add_e(x, ch.ANC + 2, 0.08 * lift);
                    sub_e(x, ch.ANC, 0.03 * lift);
                    sub_e(x, ch.ANCR, 0.7 * lift);
                    sub_e(x, ch.HLY, 0.15 * lift);
                    sub_e(x, ch.HLP, 0.1 * lift);
                    add_e(x, ch.MCURVE, 0.3 * lift);
                    add_e(x, ch.MTILT, 0.15 * lift);
                } else if kind == "slosher" {
                    let k = win(ft, 0.1, 0.35, 0.8, 1.15);
                    let sw = ((ft - 0.35) * 14.0).sin() * win(ft, 0.35, 0.45, 0.7, 0.8);
                    lerp_e(x, ch.ANC, -0.1, 0.8, 0.24, k);
                    lerp_e(x, ch.ANCR, 0.25, 0.25 + 0.25 * sw, 0.12 * sw, k);
                    add_e(x, ch.HLP, 0.28 * k);
                    sub_e(x, ch.HLY, 0.1 * k);
                    add_e(x, ch.SPINE, 0.05 * k);
                    sub_e(x, ch.LOOKY, 0.2 * k);
                    add_e(x, ch.MCURVE, 0.25 * k);
                    add_e(x, ch.BROWY, 0.4 * k);
                } else if kind == "splatling" {
                    let k = win(ft, 0.1, 0.3, 0.75, 1.1);
                    let b = 0.0f64.max(((ft - 0.3) * 17.0).sin()) * win(ft, 0.3, 0.35, 0.6, 0.7);
                    add_e(x, ch.ANC + 1, 0.07 * k + 0.02 * b);
                    sub_e(x, ch.ANCR, 0.35 * k);
                    sub_e(x, ch.HIPS_P + 1, 0.02 * b);
                    sub_e(x, ch.SPINE, 0.06 * k);
                    sub_e(x, ch.HLP, 0.08 * k);
                    add_e(x, ch.MCURVE, 0.3 * k);
                    sub_e(x, ch.MTILT, 0.15 * k);
                } else if kind == "charger" {
                    let k = win(ft, 0.1, 0.4, 0.85, 1.15);
                    lerp_e(x, ch.ANC, -0.05, 1.1, 0.1, k);
                    lerp_e(x, ch.ANCR, -0.05, 0.06, 0.0, k);
                    x[ch.IKL] = 1.0;
                    sub_e(x, ch.HEAD + 2, 0.15 * k);
                    add_e(x, ch.SQUINT, 0.6 * k);
                    sub_e(x, ch.HLY, 0.08 * k);
                } else {
                    let k = win(ft, 0.1, 0.35, 0.8, 1.15);
                    lerp_e(x, ch.ANC, -0.14, 1.06, 0.02, k);
                    lerp_e(x, ch.ANCR, -2.3, 0.3, 0.15, k);
                    lerp_1(x, ch.IKL, 0.0, k);
                    sub_e(x, ch.HIPS_P + 1, 0.03 * win(ft, 0.3, 0.4, 0.45, 0.6));
                }
            }
            "look" => {
                let y = kc(ft, &tab.k_look_t, &tab.k_look_v);
                add_e(x, ch.HLY, y);
                add_e(x, ch.SPINE + 1, y * 0.18);
                add_e(x, ch.CHEST + 1, y * 0.22);
                add_e(x, ch.LOOKX, y * 0.25);
                add_e(x, ch.HLP, 0.06 * y.abs());
            }
            "stretch" => {
                let k = win(ft, 0.1, 0.55, 1.2, 1.7);
                lerp_1(x, ch.UARML, -2.85, k);
                lerp_1(x, ch.UARML + 2, 0.25, k);
                lerp_1(x, ch.FARML, -0.15, k);
                mul_e(x, ch.IKL, 1.0 - k);
                lerp_e(x, ch.ANC, -0.12, 1.25, -0.02, k * 0.8);
                lerp_e(x, ch.ANCR, -2.6, 0.2, 0.2, k * 0.8);
                sub_e(x, ch.SPINE, 0.14 * k);
                sub_e(x, ch.CHEST, 0.12 * k);
                add_e(x, ch.HLP, 0.22 * k);
                add_e(x, ch.HIPS_P + 2, 0.02 * k);
                max_1(x, ch.MOPEN, 0.85 * win(ft, 0.35, 0.6, 1.05, 1.3));
                sub_e(x, ch.MCURVE, 0.7 * k);
                sub_e(x, ch.EYE, 0.85 * k);
                add_e(x, ch.BROW, 0.3 * k);
                mul_e(x, ch.SQY, 1.0 + 0.02 * k);
                lerp_1(x, ch.HANDPL, 2.0, k);
                sub_e(x, ch.EARS, 0.5 * k);
            }
            "tank" => {
                let a = kf(ft, &tab.k_tank_t, &tab.k_tank_v);
                let tap = 0.0f64.max(((ft - 0.35) * 18.0).sin()) * win(ft, 0.35, 0.4, 0.8, 0.9);
                x[ch.LTW] = a as f32;
                set_e(x, ch.LTGT, 0.165, 0.83 + 0.012 * tap, -0.125 - 0.012 * tap);
                mul_e(x, ch.IKL, 1.0 - a);
                lerp_e(x, ch.POLEL, 0.7, -0.5, -0.5, a);
                add_e(x, ch.CHEST + 1, 0.2 * a);
                add_e(x, ch.HLY, 0.35 * a);
                sub_e(x, ch.HLP, 0.1 * a);
                add_e(x, ch.LOOKX, 0.3 * a);
                lerp_1(x, ch.HANDPL, 1.6, a);
                if tap > 0.9 && !c._tapped {
                    c._tapped = true;
                    add_e(&mut c.sp, spr_.s_tank_x + 1, 0.8);
                    add_e(&mut c.sp, spr_.s_tank_l + 1, 1.2);
                } else if tap < 0.2 {
                    c._tapped = false;
                }
            }
            "bounce" => {
                let b = 0.0f64.max(((ft - 0.15) * TAU * 2.6).sin()) * win(ft, 0.1, 0.2, 1.0, 1.2);
                add_e(
                    x,
                    ch.HIPS_P + 1,
                    0.035 * b - 0.02 * win(ft, 0.1, 0.2, 1.0, 1.2),
                );
                c._toeUp = b;
                add_e(x, ch.UARML + 2, 0.12 * b);
                add_e(x, ch.CLAVL + 2, 0.05 * b);
                sub_e(x, ch.CLAVR + 2, 0.05 * b);
                add_e(x, ch.MCURVE, 0.2);
            }
            "shake" => {
                let k = win(ft, 0.05, 0.15, 0.7, 0.95);
                let s1 = (ft * 42.0).sin() * k;
                add_e(x, ch.HIPS + 2, 0.06 * s1);
                sub_e(x, ch.CHEST + 2, 0.1 * s1);
                add_e(x, ch.HEAD + 2, 0.14 * s1);
                add_e(x, ch.HLY, 0.2 * (ft * 38.0 + 1.0).sin() * k);
                add_e(x, ch.UARML + 2, 0.15 * k + 0.1 * s1);
                add_e(x, ch.SQUINT, 0.8 * k);
                sub_e(x, ch.MCURVE, 0.3 * k);
                lerp_1(x, ch.HANDPL, 1.85, k);
                add_e(x, ch.EARS, 0.6 * s1);
                if k > 0.3 && s1.abs() > 0.95 {
                    kick = Some((s1 * 0.8, 0.3, 0.0));
                }
            }
            _ => {}
        }
    }
    if let Some((kx, ky, kz)) = kick {
        c.hair_kick(kx, ky, kz);
    }
    let px = &c.PX;
    let p = &mut c.P;
    pose_lerp_self(p, px, w);
}

/// ≡ `addExpr(P, X, w)` (character.js L423): add-blend an 8-vector over the
/// `XCH` channels; the `w <= 0.001` early-out is load-bearing (it skips the
/// Float32 round-trips JS would otherwise take).
#[inline]
fn add_expr(p: &mut [f32], xch: &[usize; 8], x: &[f64; 8], w: f64) {
    if w <= 0.001 {
        return;
    }
    for i in 0..8 {
        add_e(p, xch[i], x[i] * w);
    }
}

/// ≡ `_poseBow` — no-op (bow never held in the pinned scenario).
fn pose_bow(_dt: f64) {}
/// ≡ `_poseLook(dt, s)` (character.js L2737-2796). The harness never has
/// `G.actors`, so `lookActor` is always `None` and `_pickLook` is unreachable;
/// `inWorld` is always true, so `_attendMenu` is unreachable too.
#[allow(non_snake_case)]
fn pose_look(c: &mut CharAnim, dt: f64, _s: &AnimState) {
    let tab = c.tab;
    let ch = &tab.ch;
    let tim = &tab.tim;
    let spr_ = &tab.spr;
    let mut ty: f64;
    let mut tp: f64;
    // ≡ L2740: `!this.inWorld` → `_attendMenu` — unreachable in the harness.
    let mut actor_gaze = false;
    c.lookT -= dt;
    if c.lookT <= 0.0 {
        c.lookT = 0.9 + c.rng.next() * 2.2;
        c.lookActor = None;
        // ≡ L2747: `if (G.actors && G.actors.length) _pickLook();` — G.actors
        // is undefined in the harness, so this never runs.
        c.glanceYaw = (c.rng.next() - 0.5) * (if c.idleT > 1.0 { 1.1 } else { 0.4 });
        c.glancePitch = (c.rng.next() - 0.5) * 0.25;
    }
    if c.wAim > 0.5 {
        ty = 0.0;
        tp = c.aimP * 0.85;
    } else {
        // ≡ L2752-2761: the `lookActor` branch is dead (always None); the
        // glance path below is what JS takes.
        ty = c.glanceYaw * (1.0 - c.gaitW * 0.7);
        tp = c.glancePitch;
        ty = lerp(
            ty,
            clamp(c.mdx.atan2(c.mdz.max(0.2)) * 0.5, -0.6, 0.6),
            c.gaitW * 0.6,
        );
        ty += clamp(c.yawRate * 0.1, -0.35, 0.35) * c.gaitW;
        tp = lerp(tp, c.aimP * 0.6, 0.5);
        if c.gaitW > 0.5 {
            actor_gaze = false;
        }
    }
    // ≡ L2771: a flinching glance toward the last hit.
    let th = c.tr[tim.t_hit] as f64;
    let hg = if th < 1.1 {
        win(th, 0.02, 0.08, 0.55, 1.1) * clamp(c.hitAmp, 0.5, 1.0)
    } else {
        0.0
    };
    if hg > 0.01 {
        ty = lerp(ty, clamp(c.hitX.atan2(c.hitZ), -1.3, 1.3), 0.65 * hg);
        tp = lerp(tp, 0.05, hg);
        actor_gaze = false;
    }
    ty = clamp(ty, -1.1, 1.1);
    tp = clamp(tp, -0.6, 0.55);
    if !actor_gaze {
        let cp = tp.cos();
        c.att.p = glam::DVec3::new(
            ty.sin() * cp * 8.0,
            1.2 + tp.sin() * 8.0,
            ty.cos() * cp * 8.0,
        );
    }
    c.att.on = true;
    c.att.kind = 0; // K_NONE
    // ≡ L2782-2796: stabilised head springs + pose-driven eye direction.
    let hy = spr(
        &mut c.sp,
        spr_.s_hl_y,
        clamp(ty, -0.85, 0.85),
        2.4,
        0.62,
        dt,
    );
    let hpp = spr(&mut c.sp, spr_.s_hl_p, clamp(tp, -0.5, 0.45), 2.6, 0.62, dt);
    let p = &mut c.P;
    add_e(p, ch.HLY, hy);
    add_e(p, ch.HLP, hpp);
    add_e(p, ch.NECK + 1, hy * 0.3);
    sub_e(p, ch.NECK, hpp * 0.2);
    add_e(p, ch.LOOKX, clamp((ty - hy) * 0.9, -0.36, 0.36));
    add_e(p, ch.LOOKY, clamp((tp - hpp) * 0.8 + 0.02, -0.3, 0.3));
    let hp = spr(&mut c.sp, spr_.s_head_p, 0.0, 3.2, 0.4, dt);
    let hr = spr(&mut c.sp, spr_.s_head_r, 0.0, 3.2, 0.4, dt);
    let p = &mut c.P;
    add_e(p, ch.HEAD, hp * 0.5);
    add_e(p, ch.HEAD + 2, hr * 0.4);
    sub_e(p, ch.HLP, hp * 0.4);
    sub_e(p, ch.HEAD + 2, clamp(c.yawRate * c.gs * 0.006, -0.12, 0.12));
}

/// ≡ `_poseFace(dt, s)` (character.js L3001-3035): blink envelope + smoothed
/// expression weights (`xw`) blended into the face channels.
#[allow(non_snake_case)]
fn pose_face(c: &mut CharAnim, dt: f64, _s: &AnimState) {
    let tab = c.tab;
    let ch = &tab.ch;
    let tim = &tab.tim;
    if c.lifeLv == 0.0 {
        c._effort = 0.0;
        c.blinkL = 0.0;
        c.blinkR = 0.0;
        c.blinkK = 0.0;
        return;
    }
    c.blink_tick(dt);
    let eff = c._effort;
    c._effort = 0.0;
    let th = c.tr[tim.t_hit] as f64;
    let hit_k = if th < 0.7 {
        pulse(th, 0.03, 4.5) * clamp(c.hitAmp, 0.5, 1.2)
    } else {
        0.0
    };
    let firing = if c.lastShot < 0.25 { 1.0 } else { 0.0 };
    let charger = c.animKind == "charger";
    let shooter = c.animKind == "shooter";
    let tgt0 = c.wAim * (1.0 - firing * 0.5) * (if charger { 1.0 } else { 0.8 });
    let tgt1 = firing * (if shooter { 1.0 } else { 0.4 });
    let tgt2 = eff.max(if charger { c.charge * c.wAim } else { 0.0 });
    let tgt3 = hit_k;
    let tgt4 = c.wLow * (1.0 - hit_k);
    let tgt5 = c.wGlow * (1.0 - c.wLow) * 0.9;
    let ts = c.tr[tim.t_spawn] as f64;
    let tgt6 = if ts < 1.3 {
        win(ts, 0.0, 0.05, 0.5, 0.9)
    } else {
        0.0
    };
    let tgt7 = c.wTired.max(c.wGoo * 0.7);
    let tgt8 = if ts < 1.6 {
        win(ts, 0.75, 0.9, 1.2, 1.6)
    } else {
        0.0
    };
    // ≡ L3021-3023: damp into the Float32 `xw` bank (round-trip per write).
    {
        let xw = &mut c.xw;
        xw[0] = damp(xw[0] as f64, tgt0, 12.0, dt) as f32;
        xw[1] = damp(xw[1] as f64, tgt1, 12.0, dt) as f32;
        xw[2] = damp(xw[2] as f64, tgt2, 18.0, dt) as f32;
        xw[3] = tgt3.max(damp(xw[3] as f64, tgt3, 10.0, dt)) as f32;
        xw[4] = damp(xw[4] as f64, tgt4, 6.0, dt) as f32;
        xw[5] = damp(xw[5] as f64, tgt5, 5.0, dt) as f32;
        xw[6] = damp(xw[6] as f64, tgt6, 20.0, dt) as f32;
        xw[7] = damp(xw[7] as f64, tgt7, 4.0, dt) as f32;
        xw[8] = damp(xw[8] as f64, tgt8, 8.0, dt) as f32;
    }
    let (x0, x1, x2, x3, x4, x5, x6, x7, x8) = {
        let xw = &c.xw;
        (
            xw[0] as f64,
            xw[1] as f64,
            xw[2] as f64,
            xw[3] as f64,
            xw[4] as f64,
            xw[5] as f64,
            xw[6] as f64,
            xw[7] as f64,
            xw[8] as f64,
        )
    };
    let xch = tab.xch;
    let p = &mut c.P;
    add_expr(p, &xch, &tab.expr8("X_FOCUS"), x0 * (1.0 - x3));
    add_expr(p, &xch, &tab.expr8("X_GRIN"), x1 * (1.0 - x3));
    add_expr(p, &xch, &tab.expr8("X_EFFORT"), x2 * (1.0 - x3));
    add_expr(p, &xch, &tab.expr8("X_WINCE"), x3);
    add_expr(p, &xch, &tab.expr8("X_WORRY"), x4);
    add_expr(p, &xch, &tab.expr8("X_DETERM"), x5 * (1.0 - x0 * 0.5));
    add_expr(p, &xch, &tab.expr8("X_SURPRISE"), x6);
    add_expr(p, &xch, &tab.expr8("X_TIRED"), x7 * (1.0 - x3));
    add_expr(p, &xch, &tab.expr8("X_JOY"), x8 * 0.6);
    add_e(
        p,
        ch.EARS,
        0.25 * x0 + 0.1 * x1 + 0.15 * x2 - 0.6 * x3 - 0.9 * x4 + 0.5 * x5 + 0.8 * x6 - 0.8 * x7
            + 0.8 * x8
            - 0.3 * c.runW * c.gaitW,
    );
    p[ch.HANDPL] = (p[ch.HANDPL] as f64 + 0.9 * hit_k).min(2.0) as f32;
    let et = c.exert.max(c.wTired);
    let gasp = (0.08 + 0.14 * et) * (0.5 + 0.5 * (TAU * c.brPh).sin()) * et;
    max_1(p, ch.MOPEN, gasp);
    if c.runW * c.gaitW > 0.5 {
        max_1(p, ch.MOPEN, 0.12 * c.runW);
    }
}

// ---------------------------------------------------------------------------
// ≡ character.js L3036-3301: `_poseDance` + the nine dance clips.
// ---------------------------------------------------------------------------

/// ≡ `setAnc(D, h)` (character.js L502).
#[inline]
fn set_anc(ch: &Channels, d: &mut [f32], h: &HoldFrame) {
    set_e(d, ch.ANC, h.p[0], h.p[1], h.p[2]);
    set_e(d, ch.ANCR, h.r[0], h.r[1], h.r[2]);
}

/// ≡ `_poseDance(D, name, t, dt)` (character.js L3036-3053). The JS `dt`
/// argument is never read inside — dropped here.
#[allow(non_snake_case)]
fn pose_dance(
    tab: &Tables,
    hold: &HoldDef,
    anim_kind: &str,
    dance_var: f64,
    d: &mut [f32],
    name: &str,
    t: f64,
) {
    let ch = &tab.ch;
    set_1(d, ch.WPL, 0.0);
    set_1(d, ch.WPR, 0.0);
    set_1(d, ch.STAB, 0.0);
    set_1(d, ch.AFOLT, 0.0);
    set_1(d, ch.AFOLR, 0.0);
    match name {
        "victory" => {
            if dance_var == 0.0 {
                d_victory_pump(tab, hold, d, t);
            } else if dance_var == 1.0 {
                d_victory_flourish(tab, hold, anim_kind, d, t);
            } else {
                d_victory_hops(tab, hold, d, t);
            }
        }
        "defeat" => {
            if dance_var == 0.0 {
                d_defeat_slump(tab, d, t);
            } else if dance_var == 1.0 {
                d_defeat_sulk(tab, hold, d, t);
            } else {
                d_defeat_kick(tab, hold, d, t);
            }
        }
        "menu_idle" => d_menu_idle(tab, hold, anim_kind, d, t),
        "lobby_pose" => d_lobby(tab, hold, anim_kind, d, t),
        "locker_idle" => d_locker(tab, hold, d, t),
        _ => {}
    }
}

/// ≡ `_dVictoryPump` (character.js L3056-3096): bouncing fist pumps →
/// hop-spin → hero pose + wink (8 beats @ 126 bpm).
#[allow(non_snake_case)]
fn d_victory_pump(tab: &Tables, hold: &HoldDef, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let b = (t * 2.1) % 8.0;
    let bf = frac(b);
    let pump = (PI * b).sin().abs().powf(0.6);
    let hit = (-bf * 7.0).exp();
    set_1(d, ch.HIPS_P + 1, -0.055 + 0.045 * pump);
    set_1(d, ch.SPINE, 0.05 - 0.05 * pump);
    set_1(d, ch.CHEST, -0.08 * pump);
    set_1(d, ch.HEAD, 0.12 * hit - 0.1);
    set_1(d, ch.HEAD + 2, 0.1 * (PI * b * 0.5).sin());
    set_1(d, ch.SQY, 1.0 - 0.05 * hit);
    set_1(d, ch.SQXZ, 1.0 + 0.03 * hit);
    set_anc(ch, d, &hold.raise);
    set_1(d, ch.IKL, 0.0);
    set_1(d, ch.POLER, -0.8);
    set_1(d, ch.POLER + 1, 0.1);
    set_1(d, ch.POLER + 2, -0.5);
    set_e(d, ch.FOOTL, 0.11, ankle_h, 0.01);
    set_e(d, ch.FOOTR, -0.11, ankle_h, -0.01);
    set_1(d, ch.FOOTLR + 1, 0.2);
    set_1(d, ch.FOOTRR + 1, -0.2);
    set_1(d, ch.FOOTLR, 0.25 * pump * if b < 4.0 { 1.0 } else { 0.0 });
    if b < 4.0 {
        set_1(d, ch.UARML, -2.6 - 0.35 * pump);
        set_1(d, ch.UARML + 2, 0.35);
        set_1(d, ch.FARML, -0.2 - 1.3 * (1.0 - pump));
        set_1(d, ch.HANDL, 0.0);
        set_1(d, ch.HANDPL, -1.0);
        add_e(d, ch.ANC + 1, 0.05 * pump);
        set_1(d, ch.CHEST + 1, 0.12 * (PI * b).sin());
        // anticipation crouch before the spin
        let pre = win(b, 3.4, 3.8, 3.9, 4.0);
        sub_e(d, ch.HIPS_P + 1, 0.07 * pre);
        add_e(d, ch.SPINE, 0.15 * pre);
        lerp_1(d, ch.UARML, -0.6, pre);
    } else if b < 6.0 {
        let k = (b - 4.0) / 2.0;
        set_1(d, ch.MODELR + 1, wrap_a(TAU * ease(k)));
        let air = (PI * clamp(k * 1.15, 0.0, 1.0)).sin();
        set_1(d, ch.MODEL + 1, 0.2 * air);
        add_e(d, ch.FOOTL + 1, 0.14 * air);
        add_e(d, ch.FOOTR + 1, 0.14 * air);
        set_1(d, ch.FOOTLR, 0.45 * air);
        set_1(d, ch.FOOTRR, 0.45 * air);
        set_1(d, ch.UARML, -1.4);
        set_1(d, ch.UARML + 2, 1.1);
        set_1(d, ch.FARML, -0.6);
        set_1(d, ch.HANDPL, 1.8);
        set_1(d, ch.HEAD + 1, -0.3 * (TAU * k).sin());
        let land = if k > 0.87 {
            (-(k - 0.87) * 18.0).exp()
        } else {
            0.0
        };
        mul_e(d, ch.SQY, 1.0 - 0.1 * land);
        sub_e(d, ch.HIPS_P + 1, 0.06 * land);
    } else {
        let k = back_out((b - 6.0) / 0.4, 2.0);
        set_1(d, ch.FOOTL, lerp(0.11, 0.16, k));
        set_1(d, ch.FOOTR, lerp(-0.11, -0.16, k));
        set_1(d, ch.FOOTLR + 1, 0.3);
        set_1(d, ch.FOOTRR + 1, -0.3);
        set_1(d, ch.HIPS_P + 1, -0.07);
        set_1(d, ch.HIPS + 2, 0.06 * k);
        set_1(d, ch.CHEST, -0.12);
        set_1(d, ch.UARML, -2.0);
        set_1(d, ch.UARML + 2, 1.0 * k);
        set_1(d, ch.FARML, -0.9);
        set_1(d, ch.HANDL + 2, 0.3);
        set_1(d, ch.HANDPL, 1.9);
        set_1(d, ch.HEAD + 2, -0.18 * k);
        set_1(d, ch.HEAD, -0.1);
        set_1(
            d,
            ch.WINK,
            ease((b - 6.3) / 0.12) * (1.0 - ease((b - 7.6) / 0.15)),
        );
        let (hp, hr) = &tab.hold_hero;
        set_e(d, ch.ANC, hp[0], hp[1], hp[2]);
        set_e(d, ch.ANCR, hr[0], hr[1], hr[2]);
    }
    set_1(d, ch.MOPEN, 0.75);
    set_1(d, ch.MCURVE, 1.0);
    set_1(d, ch.BROW, -0.1);
    set_1(d, ch.BROWY, 0.8);
    set_1(d, ch.EYE, 0.92);
    set_1(d, ch.LOOKY, 0.1);
    set_1(d, ch.EARS, 0.9 + 0.1 * hit);
}

/// ≡ `_dVictoryFlourish` (character.js L3099-3124): anticipation dip →
/// weapon twirl overhead on tiptoe → point it at the camera, hand on hip,
/// wink.
#[allow(non_snake_case)]
fn d_victory_flourish(tab: &Tables, hold: &HoldDef, anim_kind: &str, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let c = t % 4.6;
    let dip = win(c, 0.0, 0.3, 0.42, 0.62);
    let rise = win(c, 0.45, 0.7, 1.4, 1.7);
    let strike = win(c, 1.45, 1.62, 3.9, 4.45);
    let bounce = strike * ((c - 1.7) * TAU * 1.05).sin().max(0.0);
    set_e(d, ch.FOOTL, 0.1, ankle_h, 0.03);
    set_e(d, ch.FOOTR, -0.12, ankle_h, -0.04);
    set_1(d, ch.FOOTLR + 1, 0.25);
    set_1(d, ch.FOOTRR + 1, -0.35);
    set_1(
        d,
        ch.HIPS_P + 1,
        -0.03 - 0.08 * dip + 0.03 * rise - 0.04 * strike + 0.015 * bounce,
    );
    set_1(d, ch.SPINE, 0.2 * dip - 0.12 * rise - 0.02 * strike);
    set_1(d, ch.CHEST, 0.1 * dip - 0.1 * rise);
    set_1(d, ch.FOOTLR, 0.45 * rise);
    set_1(d, ch.FOOTRR, 0.45 * rise);
    add_e(d, ch.FOOTL + 1, 0.03 * rise);
    add_e(d, ch.FOOTR + 1, 0.03 * rise);
    // weapon: pulled back → overhead twirl → thrust forward to the camera
    set_anc(ch, d, &hold.carry);
    lerp_e(d, ch.ANC, -0.16, 0.8, -0.02, dip);
    lerp_e(d, ch.ANCR, 0.9, 0.3, 0.2, dip);
    lerp_e(d, ch.ANC, -0.15, 1.36, 0.06, rise);
    lerp_e(d, ch.ANCR, -1.6, 0.2, 0.0, rise);
    let no_twirl = tab.no_twirl.get(anim_kind).copied().unwrap_or(false);
    set_1(
        d,
        ch.SPIN,
        wrap_a(TAU * 2.0 * ease((c - 0.55) / 0.85)) * if no_twirl { 0.0 } else { 1.0 },
    );
    lerp_e(d, ch.ANC, -0.1, 1.02, 0.32, strike);
    lerp_e(d, ch.ANCR, -0.12, -0.05, 0.25, strike);
    set_1(d, ch.IKL, 0.0);
    set_1(d, ch.POLER, -0.9);
    set_1(d, ch.POLER + 1, 0.05);
    set_1(d, ch.POLER + 2, -0.3);
    // free hand: up with the twirl, then on the hip
    set_1(d, ch.UARML, -2.4 * rise - 0.4 * dip);
    set_1(d, ch.UARML + 2, 0.3 + 0.2 * dip);
    set_1(d, ch.FARML, -0.3 - 0.8 * dip);
    set_1(d, ch.LTW, strike);
    set_e(d, ch.LTGT, 0.19, 0.735, -0.01);
    lerp_e(d, ch.POLEL, 1.0, 0.1, -0.35, strike);
    set_1(d, ch.HANDPL, lerp(lerp(1.0, 1.9, rise), 1.5, strike));
    set_1(d, ch.EARS, 0.4 + 0.5 * rise + 0.4 * strike);
    set_1(d, ch.HIPS + 2, 0.08 * strike);
    set_1(d, ch.HIPS_P, 0.02 * strike);
    set_1(d, ch.HIPS + 1, 0.12 * strike);
    set_1(d, ch.CHEST + 1, -0.15 * strike);
    set_1(d, ch.HEAD + 2, -0.16 * strike + 0.04 * bounce);
    set_1(d, ch.HEAD, -0.06 * strike - 0.1 * rise);
    set_1(d, ch.HEAD + 1, 0.08 * strike);
    set_1(d, ch.WINK, win(c, 1.75, 1.85, 2.4, 2.55));
    set_1(d, ch.MCURVE, 1.0);
    set_1(d, ch.MOPEN, 0.3 + 0.45 * rise + 0.2 * bounce);
    set_1(d, ch.MTILT, 0.2 * strike);
    set_1(d, ch.BROW, -0.2 * strike);
    set_1(d, ch.BROWY, 0.6 * rise);
    set_1(d, ch.LOOKX, -0.05);
    set_1(d, ch.LOOKY, 0.05 + 0.2 * rise);
    set_1(d, ch.EYE, 1.0 - 0.1 * strike);
}

/// ≡ `_dVictoryHops` (character.js L3127-3159): side-to-side happy hops with
/// alternating arm waves → big V jump (8 beats @ 150 bpm).
#[allow(non_snake_case)]
fn d_victory_hops(tab: &Tables, hold: &HoldDef, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let b = (t * 2.5) % 8.0;
    let bf = frac(b);
    let bi = b.floor() as i64;
    let sd = if bi % 2 != 0 { -1.0 } else { 1.0 };
    set_anc(ch, d, &hold.raise);
    set_1(d, ch.IKL, 0.0);
    if b < 6.0 {
        let hop = (PI * clamp(bf / 0.62, 0.0, 1.0)).sin();
        let squash = if bf > 0.62 {
            (-(bf - 0.62) * 14.0).exp()
        } else {
            0.0
        };
        set_1(d, ch.MODEL, lerp(-0.07 * sd, 0.07 * sd, ease(bf / 0.62)));
        set_1(d, ch.MODEL + 1, 0.09 * hop);
        set_1(d, ch.HIPS + 2, -0.1 * sd * hop);
        set_1(d, ch.CHEST + 2, 0.08 * sd * hop);
        set_1(d, ch.HEAD + 2, 0.12 * sd * hop);
        set_e(
            d,
            ch.FOOTL,
            0.11,
            ankle_h + 0.06 * hop * if sd > 0.0 { 1.0 } else { 0.4 },
            0.0,
        );
        set_e(
            d,
            ch.FOOTR,
            -0.11,
            ankle_h + 0.06 * hop * if sd < 0.0 { 1.0 } else { 0.4 },
            0.0,
        );
        set_1(d, ch.FOOTLR, 0.4 * hop);
        set_1(d, ch.FOOTRR, 0.4 * hop);
        set_1(d, ch.FOOTLR + 1, 0.15);
        set_1(d, ch.FOOTRR + 1, -0.15);
        set_1(d, ch.HIPS_P + 1, -0.04 - 0.06 * squash + 0.01 * hop);
        set_1(d, ch.SQY, 1.0 - 0.06 * squash + 0.03 * hop);
        // left arm waves big arcs; weapon arm pumps
        set_1(d, ch.UARML, -2.5);
        set_1(d, ch.UARML + 2, 0.3 + 0.45 * (PI * b).sin());
        set_1(d, ch.FARML, -0.3 - 0.3 * (PI * b * 2.0).sin());
        add_e(d, ch.ANC + 1, 0.06 * hop);
        add_e(d, ch.ANC, 0.03 * sd);
        set_1(d, ch.HANDPL, 2.0);
        set_1(d, ch.EARS, 0.7 + 0.3 * hop);
    } else {
        let k = (b - 6.0) / 2.0;
        let crouch = win(k, 0.0, 0.12, 0.18, 0.26);
        let air = (PI * clamp((k - 0.2) / 0.62, 0.0, 1.0)).sin();
        let land = if k > 0.82 {
            (-(k - 0.82) * 16.0).exp()
        } else {
            0.0
        };
        set_1(d, ch.MODEL + 1, 0.28 * air);
        set_1(d, ch.HIPS_P + 1, -0.03 - 0.1 * crouch - 0.07 * land);
        set_1(d, ch.SPINE, 0.2 * crouch - 0.1 * air);
        set_1(d, ch.SQY, 1.0 - 0.08 * crouch + 0.08 * air - 0.08 * land);
        set_e(d, ch.FOOTL, 0.12, ankle_h + 0.18 * air, 0.02 * air);
        set_e(d, ch.FOOTR, -0.12, ankle_h + 0.18 * air, 0.02 * air);
        set_1(d, ch.FOOTLR, 0.6 * air);
        set_1(d, ch.FOOTRR, 0.6 * air);
        set_1(d, ch.UARML, lerp(-0.5, -2.7, air));
        set_1(d, ch.UARML + 2, lerp(0.2, 0.75, air));
        set_1(d, ch.FARML, -0.2);
        lerp_e(d, ch.ANC, -0.18, 1.4, 0.05, air);
        lerp_e(d, ch.ANCR, -1.9, 0.3, -0.5, air);
        set_1(d, ch.HEAD, -0.2 * air);
        set_1(d, ch.MOPEN, 0.9 * air);
        set_1(d, ch.HANDPL, 2.0);
        set_1(d, ch.EARS, 1.0);
    }
    max_1(d, ch.MOPEN, 0.65);
    set_1(d, ch.MCURVE, 1.0);
    set_1(d, ch.BROWY, 0.9);
    set_1(d, ch.EYE, 0.88);
    set_1(d, ch.LOOKY, 0.12);
}

/// ≡ `_dDefeatSlump` (character.js L3162-3179): slumped sway, big sigh, head
/// drop.
#[allow(non_snake_case)]
fn d_defeat_slump(tab: &Tables, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let cyc = t % 6.0;
    let sw = (t * TAU * 0.28).sin();
    let sigh = win(cyc, 3.0, 3.7, 3.9, 4.8);
    let drop = win(cyc, 4.3, 4.6, 4.7, 5.6);
    set_1(d, ch.HIPS_P + 1, -0.045 - 0.02 * drop);
    set_1(d, ch.HIPS_P, 0.01 * sw);
    set_1(d, ch.HIPS + 2, -0.03 * sw);
    set_1(d, ch.SPINE, 0.2 - 0.1 * sigh + 0.05 * drop);
    set_1(d, ch.CHEST, 0.16 - 0.12 * sigh + 0.05 * drop);
    set_1(d, ch.NECK, 0.1);
    set_1(d, ch.HEAD, 0.2 - 0.16 * sigh + 0.1 * drop + 0.03 * sw);
    set_1(d, ch.HEAD + 2, 0.06 * sw);
    set_1(d, ch.HEAD + 1, 0.05 * (t * 0.7).sin());
    set_1(d, ch.CLAVL + 2, -0.12 + 0.2 * sigh);
    set_1(d, ch.CLAVR + 2, 0.12 - 0.2 * sigh);
    set_1(d, ch.CLAVL + 1, -0.1);
    set_1(d, ch.CLAVR + 1, 0.1);
    set_1(d, ch.UARML, 0.1 + 0.03 * sw);
    set_1(d, ch.UARML + 2, 0.05);
    set_1(d, ch.FARML, -0.12);
    set_1(d, ch.UARMR, 0.1 - 0.03 * sw);
    set_1(d, ch.UARMR + 2, -0.05);
    set_1(d, ch.FARMR, -0.12);
    set_1(d, ch.IKR, 0.0);
    set_1(d, ch.IKL, 0.0);
    set_1(d, ch.FOOTL, 0.075);
    set_1(d, ch.FOOTR, -0.075);
    set_1(d, ch.FOOTLR + 1, -0.18);
    set_1(d, ch.FOOTRR + 1, 0.18);
    set_1(d, ch.FOOTLR, 0.12);
    set_1(d, ch.FOOTL + 2, 0.02);
    set_1(d, ch.KNEEL, -0.2);
    set_1(d, ch.KNEER, 0.2);
    set_1(d, ch.MCURVE, -0.9);
    set_1(d, ch.MWIDTH, 0.8);
    set_1(d, ch.BROW, 0.75);
    set_1(d, ch.BROWY, -0.3);
    set_1(d, ch.EYE, 0.45 + 0.25 * sigh);
    set_1(d, ch.LOOKY, -0.4);
    set_1(d, ch.LOOKX, 0.1 * sw);
    set_1(d, ch.MOPEN, 0.3 * sigh);
    set_1(d, ch.HANDPL, 1.15);
    set_1(d, ch.EARS, -1.0 + 0.2 * sigh);
}

/// ≡ `_dDefeatSulk` (character.js L3182-3205): two frustrated stomps → turn
/// away, arms crossed, pouting, sneaking a glance back.
#[allow(non_snake_case)]
fn d_defeat_sulk(tab: &Tables, hold: &HoldDef, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let c = t % 6.5;
    let st1 = win(c, 0.1, 0.25, 0.3, 0.42);
    let st2 = win(c, 0.55, 0.7, 0.75, 0.87);
    let stomp = st1.max(st2);
    let imp = (if c > 0.42 && c < 0.6 {
        (-(c - 0.42) * 16.0).exp()
    } else {
        0.0
    }) + (if c > 0.87 && c < 1.05 {
        (-(c - 0.87) * 16.0).exp()
    } else {
        0.0
    });
    let turn = win(c, 1.0, 1.5, 5.7, 6.4);
    let glance = win(c, 3.2, 3.45, 3.9, 4.15);
    set_e(d, ch.FOOTL, 0.095, ankle_h, 0.01);
    set_e(d, ch.FOOTR, -0.1, ankle_h + 0.14 * stomp, 0.03 * stomp);
    set_1(d, ch.FOOTLR + 1, 0.12);
    set_1(d, ch.FOOTRR + 1, -0.2);
    set_1(d, ch.FOOTRR, -0.2 * stomp);
    set_1(d, ch.HIPS_P + 1, -0.035 - 0.05 * imp - 0.02 * stomp);
    set_1(d, ch.SPINE, 0.1 + 0.15 * imp);
    set_1(d, ch.CHEST, 0.05);
    set_1(d, ch.HIPS + 2, -0.06 * stomp);
    // fists clenched down by the sides while stomping
    set_1(d, ch.UARML, 0.15);
    set_1(d, ch.UARML + 2, 0.2);
    set_1(d, ch.FARML, -0.5);
    set_1(d, ch.CLAVL + 2, 0.12 * stomp);
    set_1(d, ch.CLAVR + 2, -0.12 * stomp);
    set_anc(ch, d, &hold.carry);
    sub_e(d, ch.ANC + 1, 0.05 * stomp);
    add_e(d, ch.ANCR, 0.4);
    // turned away with the free arm across the chest
    set_1(d, ch.MODELR + 1, -0.9 * ease(turn));
    set_1(d, ch.LTW, turn);
    set_e(d, ch.LTGT, -0.06, 0.9, 0.13);
    lerp_e(d, ch.POLEL, 1.0, -0.3, 0.2, turn);
    set_1(d, ch.IKL, 0.0);
    lerp_e(d, ch.ANC, -0.16, 0.86, 0.1, turn);
    lerp_e(d, ch.ANCR, 1.1, 0.8, 0.4, turn);
    sub_e(d, ch.CHEST, 0.08 * turn);
    set_1(d, ch.HEAD, -0.12 * turn + 0.2 * imp);
    set_1(d, ch.HEAD + 2, 0.1 * turn);
    set_1(d, ch.HEAD + 1, 0.8 * glance * turn - 0.15 * turn);
    set_1(d, ch.NECK + 1, 0.3 * glance * turn);
    set_1(d, ch.LOOKX, 0.35 * glance - 0.2 * turn * (1.0 - glance));
    set_1(d, ch.MCURVE, -1.4);
    set_1(d, ch.MWIDTH, 0.55);
    set_1(d, ch.MTILT, 0.15);
    set_1(d, ch.BROW, -0.5 + 1.1 * turn * (1.0 - glance));
    set_1(d, ch.BROWY, -0.2);
    set_1(d, ch.EYE, 0.75 - 0.3 * imp);
    set_1(d, ch.SQUINT, 0.6 * stomp);
    set_1(d, ch.MOPEN, 0.25 * stomp);
    set_1(d, ch.HANDPL, lerp(-1.0, 0.6, ease(turn)));
    set_1(d, ch.EARS, -0.6 - 0.4 * turn + 0.5 * glance);
}

/// ≡ `_dDefeatKick` (character.js L3208-3223): droop, scuff the ground with
/// a foot, sigh, sniff.
#[allow(non_snake_case)]
fn d_defeat_kick(tab: &Tables, hold: &HoldDef, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let c = t % 5.2;
    let k1 = win(c, 0.6, 0.85, 0.95, 1.2);
    let k2 = win(c, 1.4, 1.65, 1.75, 2.05);
    let scuff = k1.max(k2);
    let sigh = win(c, 2.6, 3.2, 3.4, 4.2);
    let sniff = win(c, 4.4, 4.5, 4.55, 4.75);
    set_e(d, ch.FOOTL, 0.085, ankle_h, 0.0);
    set_1(d, ch.FOOTLR + 1, -0.1);
    set_1(d, ch.FOOTRR + 1, -0.25);
    set_e(
        d,
        ch.FOOTR,
        -0.08,
        ankle_h + 0.03 * scuff,
        -0.02
            + 0.12
                * (PI * clamp((c - if k2 > k1 { 1.4 } else { 0.6 }) / 0.65, 0.0, 1.0)).sin()
                * scuff,
    );
    set_1(d, ch.FOOTRR, -0.3 * scuff);
    set_1(d, ch.HIPS_P + 1, -0.05);
    set_1(d, ch.HIPS_P, 0.02);
    set_1(d, ch.HIPS + 2, 0.05);
    set_1(d, ch.SPINE, 0.18 - 0.08 * sigh);
    set_1(d, ch.CHEST, 0.13 - 0.1 * sigh);
    set_1(d, ch.NECK, 0.1);
    set_1(d, ch.HEAD, 0.2 - 0.16 * sigh - 0.2 * sniff);
    set_1(d, ch.CLAVL + 2, -0.1 + 0.18 * sigh);
    set_1(d, ch.CLAVR + 2, 0.1 - 0.18 * sigh);
    set_1(d, ch.UARML, 0.05);
    set_1(d, ch.UARML + 2, 0.04);
    set_1(d, ch.FARML, -0.15);
    set_1(d, ch.IKL, 0.0);
    set_anc(ch, d, &hold.carry);
    sub_e(d, ch.ANC + 1, 0.12);
    add_e(d, ch.ANCR, 0.55);
    sub_e(d, ch.ANC + 2, 0.04);
    set_1(d, ch.MCURVE, -1.0);
    set_1(d, ch.MWIDTH, 0.7);
    set_1(d, ch.BROW, 0.85);
    set_1(d, ch.BROWY, -0.2);
    set_1(d, ch.EYE, 0.5 + 0.3 * sniff);
    set_1(d, ch.LOOKY, -0.35);
    set_1(d, ch.LOOKX, -0.1 * scuff);
    set_1(d, ch.MOPEN, 0.35 * sigh);
    set_1(d, ch.HANDPL, 1.1);
    set_1(d, ch.EARS, -0.9 + 0.3 * sniff);
}

/// ≡ `_dMenuIdle` (character.js L3226-3255): relaxed weight shifts + fidgets
/// + look-arounds (same systems as gameplay idle, feet in kid space).
#[allow(non_snake_case)]
fn d_menu_idle(tab: &Tables, hold: &HoldDef, anim_kind: &str, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let hipw = tab.rig_c.hipw;
    let cyc = t % 8.0;
    let w = (t * TAU / 8.0).sin();
    set_1(d, ch.HIPS_P, 0.028 * w);
    set_1(d, ch.HIPS + 2, -0.05 * w);
    set_1(d, ch.SPINE + 2, 0.03 * w);
    set_1(d, ch.CHEST + 2, 0.015 * w);
    set_1(d, ch.HIPS_P + 1, -0.024 - 0.01 * w.abs());
    set_e(d, ch.FOOTL, hipw + 0.005, ankle_h, -0.004);
    set_e(d, ch.FOOTR, -hipw - 0.005, ankle_h, -0.004);
    set_1(d, ch.FOOTLR + 1, 0.14);
    set_1(d, ch.FOOTRR + 1, -0.14);
    if w > 0.0 {
        set_1(d, ch.FOOTRR, 0.25 * w);
        add_e(d, ch.FOOTR + 2, 0.03 * w);
        sub_e(d, ch.FOOTR, 0.01 * w);
    } else {
        set_1(d, ch.FOOTLR, -0.25 * w);
        sub_e(d, ch.FOOTL + 2, 0.03 * w);
        add_e(d, ch.FOOTL, 0.01 * w);
    }
    let br = 0.0; // breathing: `_breathe` (post-dance, same for every pose)
    let yaw = kc(cyc, &tab.k_menu_t, &tab.k_menu_v);
    let pitch = -0.08 * (cyc * 0.9).sin();
    set_1(d, ch.HEAD + 1, yaw * 0.7);
    set_1(d, ch.NECK + 1, yaw * 0.3);
    set_1(d, ch.HEAD, pitch);
    set_1(d, ch.HEAD + 2, -0.05 * w);
    let yaw_l = kc(cyc + 0.25, &tab.k_menu_t, &tab.k_menu_v);
    set_1(d, ch.LOOKX, clamp(yaw_l * 0.9, -0.35, 0.35));
    set_1(d, ch.LOOKY, -pitch * 0.5);
    set_anc(ch, d, &hold.carry);
    set_1(d, ch.IKL, if hold.two_carry != 0.0 { 1.0 } else { 0.0 });
    let tw = if (6.1..7.1).contains(&cyc) {
        (cyc - 6.1) / 1.0
    } else {
        -1.0
    };
    let no_twirl = tab.no_twirl.get(anim_kind).copied().unwrap_or(false);
    if tw >= 0.0 && !no_twirl {
        let k = ease(clamp(tw / 0.8, 0.0, 1.0));
        let lift = (PI * clamp(tw, 0.0, 1.0)).sin();
        set_1(d, ch.SPIN, wrap_a(TAU * 2.0 * k));
        set_1(d, ch.IKL, 0.0);
        add_e(d, ch.ANC + 1, 0.1 * lift);
        add_e(d, ch.ANC + 2, 0.08 * lift);
        sub_e(d, ch.ANC, 0.03 * lift);
        sub_e(d, ch.ANCR, 0.5 * lift);
        set_1(d, ch.MCURVE, 1.0);
        set_1(d, ch.MOPEN, 0.25 * lift);
    } else if tw >= 0.0 {
        let lift = (PI * clamp(tw, 0.0, 1.0)).sin();
        add_e(d, ch.ANC + 1, 0.06 * lift);
        sub_e(d, ch.ANCR, 0.25 * lift);
    }
    set_1(d, ch.UARML, -0.05 + 0.03 * br);
    set_1(d, ch.UARML + 2, 0.12);
    set_1(d, ch.FARML, -0.35);
    max_1(d, ch.MCURVE, 0.8);
    set_1(d, ch.EARS, 0.15 + 0.2 * yaw.abs());
}

/// ≡ `_dLocker` (character.js L3258-3274): relaxed hand-on-hip weight
/// shifts, looking at the camera (the viewer), a head tilt now and then.
#[allow(non_snake_case)]
fn d_locker(tab: &Tables, hold: &HoldDef, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let w = (t * TAU / 6.5).sin();
    let br = 0.0; // breathing: `_breathe`
    let cyc = t % 9.0;
    set_e(d, ch.FOOTL, 0.12, ankle_h, 0.02);
    set_e(d, ch.FOOTR, -0.115, ankle_h, -0.02);
    set_1(d, ch.FOOTLR + 1, 0.26);
    set_1(d, ch.FOOTRR + 1, -0.24);
    if w > 0.0 {
        set_1(d, ch.FOOTRR, 0.22 * w);
        add_e(d, ch.FOOTR + 2, 0.02 * w);
    } else {
        set_1(d, ch.FOOTLR, -0.22 * w);
        sub_e(d, ch.FOOTL + 2, 0.02 * w);
    }
    set_1(d, ch.HIPS_P, 0.03 * w);
    set_1(d, ch.HIPS_P + 1, -0.035 - 0.012 * w.abs());
    set_1(d, ch.HIPS + 2, 0.075 * w);
    set_1(d, ch.HIPS + 1, 0.05 * w);
    set_1(d, ch.SPINE + 2, -0.045 * w);
    set_1(d, ch.CHEST + 2, -0.03 * w);
    set_1(d, ch.CHEST, -0.03 - 0.02 * br);
    set_1(d, ch.CLAVL + 2, 0.02 * br);
    set_1(d, ch.CLAVR + 2, -0.02 * br);
    let tilt = win(cyc, 3.2, 3.6, 4.6, 5.1);
    let look = win(cyc, 6.2, 6.6, 7.2, 7.7);
    set_1(d, ch.HEAD + 2, -0.04 * w - 0.14 * tilt);
    set_1(d, ch.HEAD, -0.04 + 0.02 * br);
    set_1(d, ch.HEAD + 1, 0.25 * look);
    set_1(d, ch.LOOKX, 0.28 * look);
    set_1(d, ch.LOOKY, 0.05);
    // free hand on the hip, the weapon hand relaxed at the side
    set_1(d, ch.LTW, 1.0);
    set_e(d, ch.LTGT, 0.17, 0.745, -0.005);
    set_1(d, ch.IKL, 0.0);
    set_1(d, ch.UARML + 2, 0.9);
    set_1(d, ch.FARML, -1.6);
    set_1(d, ch.HANDL, 0.3);
    set_1(d, ch.HANDL + 2, 0.6);
    set_1(d, ch.POLEL, 1.0);
    set_1(d, ch.POLEL + 1, 0.1);
    set_1(d, ch.POLEL + 2, -0.35);
    set_anc(ch, d, &hold.carry);
    add_e(d, ch.ANC + 1, 0.005 * br);
    if hold.two_carry != 0.0 {
        set_1(d, ch.IKL, 0.0);
        lerp_e(d, ch.ANC, -0.19, 0.74, 0.12, 1.0);
    }
    set_1(d, ch.MCURVE, 0.95);
    set_1(d, ch.MTILT, 0.12 * tilt);
    set_1(d, ch.BROWY, 0.2 * tilt);
    set_1(d, ch.EYE, 0.95);
    set_1(d, ch.HANDPL, 1.4);
    set_1(d, ch.EARS, 0.3 + 0.3 * tilt);
}

/// ≡ `_dLobby` (character.js L3277-3301): confident weapon-presenting stance
/// with breathing and a periodic flourish.
#[allow(non_snake_case)]
fn d_lobby(tab: &Tables, hold: &HoldDef, anim_kind: &str, d: &mut [f32], t: f64) {
    let ch = &tab.ch;
    let ankle_h = tab.rig_c.ankle_h;
    let br = 0.0; // breathing: `_breathe`
    let cyc = t % 7.0;
    let fl = win(cyc, 4.6, 4.9, 5.6, 6.1);
    set_e(d, ch.FOOTL, 0.15, ankle_h, 0.02);
    set_e(d, ch.FOOTR, -0.15, ankle_h, -0.02);
    set_1(d, ch.FOOTLR + 1, 0.3);
    set_1(d, ch.FOOTRR + 1, -0.28);
    set_1(d, ch.HIPS_P + 1, -0.05 - 0.004 * br);
    set_1(d, ch.HIPS_P, 0.012);
    set_1(d, ch.HIPS + 2, -0.05);
    set_1(d, ch.HIPS + 1, 0.08);
    set_1(d, ch.SPINE + 1, -0.05);
    set_1(d, ch.SPINE + 2, 0.03);
    set_1(d, ch.CHEST, -0.1 - 0.015 * br);
    set_1(d, ch.CHEST + 1, -0.08);
    set_1(d, ch.HEAD, -0.1);
    set_1(d, ch.HEAD + 2, -0.1);
    set_1(d, ch.HEAD + 1, 0.12);
    set_1(d, ch.NECK + 1, 0.05);
    set_anc(ch, d, &hold.lobby);
    add_e(d, ch.ANC + 1, 0.006 * br);
    set_1(d, ch.POLER, -0.9);
    set_1(d, ch.POLER + 1, -0.4);
    set_1(d, ch.POLER + 2, -0.1);
    if hold.lobby_two != 0.0 {
        set_1(d, ch.IKL, 1.0);
        set_1(d, ch.LTW, 0.0);
        set_1(d, ch.CHEST + 1, 0.05);
    } else {
        set_1(d, ch.LTW, 1.0);
        set_e(d, ch.LTGT, 0.17, 0.745, -0.005);
        set_1(d, ch.IKL, 0.0);
        set_1(d, ch.UARML + 2, 0.9);
        set_1(d, ch.FARML, -1.6);
        set_1(d, ch.HANDL, 0.3);
        set_1(d, ch.HANDL + 2, 0.6);
        set_1(d, ch.POLEL, 1.0);
        set_1(d, ch.POLEL + 1, 0.1);
        set_1(d, ch.POLEL + 2, -0.35);
    }
    // flourish: a quick re-grip / twirl and a proud chin-up
    if anim_kind == "shooter" || anim_kind == "blaster" {
        set_1(
            d,
            ch.SPIN,
            wrap_a(TAU * ease((cyc - 4.8) / 0.55)) * if cyc > 4.8 && cyc < 5.5 { 1.0 } else { 0.0 },
        );
        add_e(d, ch.ANC + 1, 0.05 * fl);
    } else {
        sub_e(d, ch.ANCR, 0.2 * fl);
    }
    sub_e(d, ch.HEAD, 0.08 * fl);
    sub_e(d, ch.HIPS_P + 1, 0.015 * fl);
    set_1(d, ch.MCURVE, 1.0);
    set_1(d, ch.MTILT, 0.22);
    set_1(d, ch.BROW, -0.25);
    set_1(d, ch.EYE, 0.92);
    set_1(d, ch.LOOKX, -0.12);
    set_1(d, ch.LOOKY, 0.04);
    set_1(d, ch.MOPEN, 0.2 * fl);
    set_1(d, ch.HANDPL, if hold.lobby_two != 0.0 { 0.0 } else { 1.5 });
    set_1(d, ch.EARS, 0.45 + 0.4 * fl);
}

/// ≡ `_lifePost(dt)` (character.js L2839-2879): breathing, attention head
/// springs, idle micro-expressions (RNG), blush and pupils.
#[allow(non_snake_case)]
fn life_post(c: &mut CharAnim, dt: f64) {
    let tab = c.tab;
    let ch = &tab.ch;
    let tim = &tab.tim;
    let spr_ = &tab.spr;
    if c.lifeLv == 0.0 {
        return;
    }
    c.breathe();
    // ---- head follows the gaze out of a match (menus only; inWorld is
    // always true in the harness, so this branch never runs, but it is
    // ported for fidelity).
    let mut wy = 0.0f64;
    let mut wp = 0.0f64;
    if !c.inWorld && c.att.on && c.kidForm && (c.tr[tim.t_spawn] as f64) > 1.4 {
        let hw = (match c.dance.as_deref() {
            Some("victory") => 0.3,
            Some("defeat") => 0.25,
            Some(_) => 0.55,
            None => 0.75,
        }) * (1.0 - clamp(c.extGl.abs() * 8.0, 0.0, 1.0))
            * (1.0 - c.wAir);
        let p = &c.P;
        let cur_y = p[ch.MODELR + 1] as f64
            + p[ch.HIPS + 1] as f64
            + p[ch.SPINE + 1] as f64
            + p[ch.CHEST + 1] as f64
            + p[ch.NECK + 1] as f64
            + p[ch.HEAD + 1] as f64
            + p[ch.HLY] as f64;
        let cur_p = p[ch.HLP] as f64
            - (p[ch.NECK] as f64
                + p[ch.HEAD] as f64
                + 0.5 * (p[ch.CHEST] as f64 + p[ch.SPINE] as f64));
        let a = c.att.p;
        let y_t = a.x.atan2(a.z);
        let p_t = (a.y - 1.2).atan2((a.x.hypot(a.z)).max(0.3));
        wy = clamp(wrap_a(y_t - cur_y), -0.85, 0.85) * hw;
        wp = clamp(p_t - cur_p, -0.35, 0.35) * hw * 0.6;
    }
    let ay = spr(&mut c.sp, spr_.s_att_y, wy, 1.5, 0.78, dt);
    let ap = spr(&mut c.sp, spr_.s_att_p, wp, 1.6, 0.8, dt);
    {
        let p = &mut c.P;
        add_e(p, ch.HLY, ay);
        add_e(p, ch.HLP, ap);
        add_e(p, ch.NECK + 1, ay * 0.25);
    }
    // ---- idle micro-expressions (RNG): never twice the same in a row.
    let calm = c.kidForm
        && !c.moving
        && c.wAim < 0.2
        && (c.tr[tim.t_hit] as f64) > 1.5
        && (c.dance.is_none()
            || c.dance.as_deref() == Some("menu_idle")
            || c.dance.as_deref() == Some("locker_idle")
            || c.dance.as_deref() == Some("lobby_pose"));
    c.mxT -= dt;
    if c.mxT <= 0.0 && c.lifeLv < 2.0 {
        c.mxT = 1.0 + c.rng.next() * 2.0;
    } else if c.mxT <= 0.0 {
        c.mxT = (if c.inWorld { 4.0 } else { 2.6 }) + c.rng.next() * 4.0;
        if calm {
            let mut k = (c.rng.next() * 5.0) as i64;
            if k as f64 == c._mxLast {
                k = (k + 1) % 5;
            }
            c._mxLast = k as f64;
            let sd = if c.rng.next() < 0.5 { 1.0 } else { -1.0 };
            let hold = 0.35 + c.rng.next() * 0.9;
            match k {
                0 => {
                    let x = tab.expr8("X_LIPPRESS");
                    c.mx_play(&x, 0.7, 0.14, hold, 0.4, 0.0, None);
                }
                1 => {
                    let x = tab.expr8("X_SMIRK");
                    c.mx_play(&x, 0.8 * sd, 0.18, hold, 0.5, 0.0, None);
                }
                2 => {
                    let x = tab.expr8("X_BROWFLASH");
                    c.mx_play(&x, 0.25, 0.2, hold, 0.5, 0.55 * sd, None);
                }
                3 => {
                    let x = tab.expr8("X_HMM");
                    let e = tab.expr3("E_HMM");
                    c.mx_play(&x, 0.7 * sd, 0.16, hold, 0.45, 0.0, Some(&e));
                }
                _ => {
                    let x = tab.expr8("X_SOFTSMILE");
                    c.mx_play(&x, 0.6, 0.3, hold + 0.6, 0.7, 0.0, None);
                }
            }
        }
    }
    c.mx_tick(dt);
    // ---- blush: warm flush after exertion, in victory, on compliments.
    let ta = c.tr[tim.t_admire] as f64;
    let tw = c.tr[tim.t_wink] as f64;
    let victory = c.dance.as_deref() == Some("victory");
    {
        let p = &mut c.P;
        add_e(
            p,
            ch.BLUSH,
            0.4 * c.exert
                + 0.2 * c.wTired
                + (if victory { 0.5 } else { 0.0 })
                + 0.45 * win(ta, 0.0, 0.3, 1.2, 1.6).max(win(tw, 0.0, 0.1, 0.5, 0.8)),
        );
    }
    // ---- pupils.
    let x8 = c.xw[8] as f64;
    let viewer_bonus = if c.att.kind == 1 && !c.inWorld {
        0.12
    } else {
        0.0
    };
    let p = &mut c.P;
    p[ch.PUPIL] = clamp(
        p[ch.PUPIL] as f64 + 0.22 * x8 + 0.2 * c.wGlow - 0.16 * c.wAim
            + viewer_bonus
            + (if victory { 0.2 } else { 0.0 }),
        0.0,
        1.0,
    ) as f32;
}
