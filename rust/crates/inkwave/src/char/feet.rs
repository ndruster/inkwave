//! World-locked foot plants + predictive gait stepping — ported from
//! `character.js`: `_ground` (L1549-1561), `_idealFoot` (L1564-1570),
//! `_gaitTarget` (L1573-1589), `_liftOff` (L1591-1599), `_touchDown`
//! (L1601-1614), `_startSettle` (L1616-1623), `_footErr` (L1625-1628),
//! `_updateFeet` (L1630-1726), `_footPose` (L1729-1751).
//!
//! The harness physics stub never hits a raycast, so `_ground` degenerates to
//! "return the root height, normal = +Y" — mirrored exactly below.
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively
use glam::DVec3;

use super::anim::{AnimState, CharAnim};
use super::math::*;

// ≡ character.js L165: `const M_GAIT = 0, M_CATCH = 1, M_SETTLE = 2;`
pub const M_GAIT: i64 = 0;
pub const M_CATCH: i64 = 1;
pub const M_SETTLE: i64 = 2;

/// ≡ `_ground(x, z, n)` (L1549-1561). `this.phys` is the harness stub whose
/// `raycast` never hits → the `h.hit` branch is dead; normal = +Y, height =
/// root.y.
pub fn ground(c: &CharAnim, _x: f64, _z: f64, n: &mut DVec3) -> f64 {
    // ≡ L1551-1558: phys truthy, raycast miss → fall through.
    *n = DVec3::Y;
    c.root_pos.y
}

/// ≡ `_idealFoot(f, out)` (L1564-1570): ideal standing plant (world) under
/// the smoothed stance bank; returns the foot's world yaw.
pub fn ideal_foot(c: &CharAnim, fi: usize, out: &mut DVec3) -> f64 {
    let st = &c.st;
    let side = c.feet[fi].side;
    // ≡ L1566 (f32 bank values widen to f64)
    let kx = (if side > 0.0 { st[0] } else { st[3] }) as f64 + c.stepOfsX;
    let kz = (if side > 0.0 { st[1] } else { st[4] }) as f64 + c.stepOfsZ;
    let cc = c.yaw.cos();
    let sn = c.yaw.sin();
    let r = c.root_pos;
    out.x = r.x + kx * cc + kz * sn;
    out.z = r.z - kx * sn + kz * cc;
    out.y = r.y;
    // ≡ L1569
    c.yaw + (if side > 0.0 { st[2] } else { st[5] }) as f64
}

/// ≡ `_gaitTarget(f, tRem, out)` (L1573-1589): predicted landing (world) for
/// a swing touching down in `tRem` seconds. Returns (target, world yaw,
/// ground normal) — JS writes `f.tn` through `_ground`.
fn gait_target(c: &CharAnim, fi: usize, t_rem: f64) -> (DVec3, f64, DVec3) {
    let f = &c.feet[fi];
    let r = c.root_pos;
    let st_half = (c.duty / c.cad) * 0.5;
    let yaw_p = c.yaw + c.yawRate * t_rem.min(0.25) * 0.8;
    let hy = yaw_p + c.hipTwist;
    // ≡ L1578 (stance bank; `Math.abs` — the bank stores the left offset negative)
    let w = lerp(
        (if f.side > 0.0 { c.st[0] } else { c.st[3] }).abs() as f64,
        0.068,
        c.runW,
    ) * f.side;
    let cc = hy.cos();
    let sn = hy.sin();
    let ta = t_rem.min(0.15);
    let ax = if c.tread {
        0.0
    } else {
        clamp(c.ra.x, -40.0, 40.0)
    };
    let az = if c.tread {
        0.0
    } else {
        clamp(c.ra.z, -40.0, 40.0)
    };
    let px = r.x + c.gvx * t_rem + 0.5 * ax * ta * ta;
    let pz = r.z + c.gvz * t_rem + 0.5 * az * ta * ta;
    let mut dx = c.gvx * st_half + w * cc;
    let mut dz = c.gvz * st_half - w * sn;
    let dl = dx.hypot(dz);
    let mx = 0.3;
    if dl > mx {
        dx *= mx / dl;
        dz *= mx / dl;
    }
    let mut out = DVec3::new(px + dx, 0.0, pz + dz);
    let mut n = DVec3::Y;
    out.y = ground(c, out.x, out.z, &mut n);
    // ≡ L1588: foot points along the hips (toe-out shrinks as speed grows)
    (out, hy + f.side * lerp(0.1, 0.04, c.runW), n)
}

/// ≡ `_liftOff(f, mode)` (L1591-1599).
fn lift_off(c: &mut CharAnim, fi: usize, mode: i64) {
    let fwd = {
        let mdz = c.mdz;
        let mdx = c.mdx;
        let ht = c.hipTwist;
        clamp(mdz * ht.cos() + mdx * ht.sin(), -1.0, 1.0)
    };
    let lift_h = c.liftH;
    let run_w = c.runW;
    let duty = c.duty;
    let cad = c.cad;
    let f = &mut c.feet[fi];
    f.planted = false;
    f.sw = true;
    f.mode = mode;
    f.su = 0.0;
    f.from = f.pw;
    f.from_yaw = f.yaw;
    f.to = f.pw;
    f.to_yaw = f.yaw;
    // ≡ L1595-1597
    f.lift = lift_h * if mode == M_CATCH { 0.7 } else { 1.0 };
    f.toe = lerp(0.3, 0.95, run_w) * if fwd >= 0.0 { fwd } else { fwd * 0.55 };
    f.land = lerp(0.3, 0.1, run_w) * if fwd >= 0.0 { fwd } else { fwd * 0.6 };
    f.dur = if mode == M_CATCH {
        0.13
    } else {
        (1.0 - duty) / cad
    };
}

/// ≡ `_touchDown(f, loud)` (L1601-1614). `this.onEvent` is unset in the
/// harness → the footstep event branch is dead.
fn touch_down(c: &mut CharAnim, fi: usize, _loud: f64) {
    let moving = c.moving;
    let i = c.feet[fi].i;
    {
        let f = &mut c.feet[fi];
        f.pw = f.to;
        f.n = f.tn;
        f.yaw = f.to_yaw;
        f.planted = true;
        f.sw = false;
        f.st_u = 0.0;
        f.su = 0.0;
        f.st_t = 0.0;
        // ≡ L1605-1608: off-beat touchdown re-syncs the gait clock
        if moving && f.mode == M_CATCH {
            f.in_st = true;
            c.phase = -(i as f64) * 0.5;
            let duty = c.duty;
            c.feet[1 - i].in_st = 0.5 < duty;
        }
    }
    c.settleCd = 0.045;
}

/// ≡ `_startSettle(f, err)` (L1616-1623).
fn start_settle(c: &mut CharAnim, fi: usize, err: f64) {
    {
        let f = &mut c.feet[fi];
        f.planted = false;
        f.sw = true;
        f.mode = M_SETTLE;
        f.su = 0.0;
        f.from = f.pw;
        f.from_yaw = f.yaw;
    }
    let mut to = DVec3::ZERO;
    let yaw = ideal_foot(c, fi, &mut to);
    let mut n = DVec3::Y;
    to.y = ground(c, to.x, to.z, &mut n);
    let from_y = c.feet[fi].from.y;
    let f = &mut c.feet[fi];
    f.to = to;
    f.to_yaw = yaw;
    f.tn = n;
    // ≡ L1621-1622
    f.dur = clamp(0.15 + err * 0.42, 0.15, 0.3);
    f.lift = clamp(0.028 + err * 0.22, 0.03, 0.085) + (f.to.y - from_y).max(0.0);
    f.toe = 0.25;
    f.land = 0.12;
}

/// ≡ `_footErr(f)` (L1625-1628).
fn foot_err(c: &CharAnim, fi: usize) -> f64 {
    let mut v6 = DVec3::ZERO;
    let yaw_i = ideal_foot(c, fi, &mut v6);
    let f = &c.feet[fi];
    ((f.pw.x - v6.x).hypot(f.pw.z - v6.z)) + 0.11 * wrap_a(f.yaw - yaw_i).abs()
}

/// ≡ `_footPose(f)` (L1729-1751): current world contact / yaw / pitch.
fn foot_pose(c: &mut CharAnim, fi: usize) {
    let moving = c.moving;
    let run_w = c.runW;
    let f = &mut c.feet[fi];
    if f.planted || !f.sw {
        f.cw = f.pw;
        f.cyaw = f.yaw;
        f.cn = f.n;
        if moving {
            let st = f.st_u;
            f.pitch = -f.land * (1.0 - sstep(0.0, 0.28, st)) + f.toe * sstep(0.42, 1.0, st);
        } else {
            // ≡ L1736: hardcoded 1/60, NOT dt
            f.pitch = damp(f.pitch, 0.0, 12.0, 1.0 / 60.0);
        }
        return;
    }
    let u = f.su;
    let e = lerp(u, mj(u), 0.8);
    f.cw.x = lerp(f.from.x, f.to.x, e);
    f.cw.z = lerp(f.from.z, f.to.z, e);
    let rise = (f.to.y - f.from.y).max(0.0);
    let gy = lerp(f.from.y, f.to.y, sstep(0.15, 0.7, u));
    let peak = lerp(0.5, 0.4, run_w);
    let lc = if u < peak {
        ((u / peak) * std::f64::consts::FRAC_PI_2).sin()
    } else {
        (((u - peak) / (1.0 - peak)) * std::f64::consts::FRAC_PI_2).cos()
    };
    f.cw.y = gy + (f.lift + rise * 0.6) * lc.max(0.0).powf(1.15);
    f.cyaw = f.from_yaw + wrap_a(f.to_yaw - f.from_yaw) * e;
    f.cn = f.n.lerp(f.tn, e);
    f.pitch = f.toe * (1.0 - sstep(0.0, 0.5, u)) - f.land * sstep(0.55, 0.96, u)
        + 0.12 * (std::f64::consts::PI * u).sin() * run_w;
}

/// ≡ `_updateFeet(dt, s)` (L1630-1726).
pub fn update_feet(c: &mut CharAnim, dt: f64, _s: &AnimState) {
    let t = c.tab.tim;
    // ≡ L1632: plant gate (f32 timer bank widens to f64)
    let plant_ok = c.kidForm
        && c.grounded
        && c.dance.is_none()
        && (c.tr[t.t_leap] as f64) > 1.9
        && (c.tr[t.t_slam] as f64) > 1.4
        && (c.tr[t.t_dodge] as f64) > c.dodgeDur * 0.86;
    // ≡ L1634: treadmill — planted contacts slide back under a stationary root
    if c.tread {
        let (tvx, tvz) = (c.tvx, c.tvz);
        for i in 0..2 {
            let f = &mut c.feet[i];
            f.pw.x -= tvx * dt;
            f.pw.z -= tvz * dt;
            f.from.x -= tvx * dt;
            f.from.z -= tvz * dt;
            f.disp.x -= tvx * dt;
            f.disp.z -= tvz * dt;
        }
    }
    if !plant_ok {
        c.moving = false;
        c.replant = true;
        c.feet[0].sw = false;
        c.feet[1].sw = false;
        c.plantW = damp(c.plantW, 0.0, 30.0, dt);
        foot_pose(c, 0);
        foot_pose(c, 1);
        return;
    }
    c.plantW = damp(c.plantW, 1.0, 14.0, dt);
    // ≡ L1643-1653: (re)plant from remembered displacements or the stance
    if c.replant || !c.feetValid {
        let feet_valid = c.feetValid;
        for i in 0..2 {
            let (pw, disp_ok, disp, disp_yaw) = {
                let f = &c.feet[i];
                (f.pw, f.disp_ok, f.disp, f.disp_yaw)
            };
            let (mut pw, yaw) = if feet_valid && disp_ok {
                (disp, disp_yaw)
            } else {
                let mut o = pw;
                let y = ideal_foot(c, i, &mut o);
                (o, y)
            };
            let mut n = DVec3::Y;
            pw.y = ground(c, pw.x, pw.z, &mut n);
            let f = &mut c.feet[i];
            f.pw = pw;
            f.n = n;
            f.yaw = yaw;
            f.planted = true;
            f.sw = false;
            f.su = 0.0;
            f.st_u = 0.5;
            f.in_st = true;
        }
        if !c.feetValid {
            c.plantW = 1.0;
        }
        c.replant = false;
        c.feetValid = true;
        c.settleCd = 0.06;
        c.moving = false;
    }
    let was = c.moving;
    c.moving = if was { c.gv > 0.3 } else { c.gv > 0.62 };
    if c.moving {
        // ≡ L1657-1666: the foot most behind the travel direction steps first
        if !was {
            let gv = if c.gv != 0.0 { c.gv } else { 1.0 };
            let dx = c.gvx / gv;
            let dz = c.gvz / gv;
            let r = c.root_pos;
            let b_l = (c.feet[0].pw.x - r.x) * dx + (c.feet[0].pw.z - r.z) * dz;
            let b_r = (c.feet[1].pw.x - r.x) * dx + (c.feet[1].pw.z - r.z) * dz;
            let first = if b_l <= b_r { 0 } else { 1 };
            c.phase = c.duty + 1e-3 - (first as f64) * 0.5;
            if !c.feet[first].sw {
                lift_off(c, first, M_CATCH);
                c.feet[first].dur = 0.15;
                c.feet[first].lift *= 0.8;
            }
            let duty = c.duty;
            let phase = c.phase;
            c.feet[first].in_st = false;
            c.feet[1 - first].in_st = frac(phase + (1 - first) as f64 * 0.5) < duty;
        }
        c.phase += c.cad * dt;
        for i in 0..2 {
            let duty = c.duty;
            let cad = c.cad;
            let p = frac(c.phase + (i as f64) * 0.5);
            let in_st = p < duty;
            let was_st = c.feet[i].in_st;
            c.feet[i].in_st = in_st;
            let r = c.root_pos;
            if c.feet[i].planted {
                c.feet[i].st_u = if in_st { p / duty } else { 1.0 };
                let hx = c.feet[i].pw.x - r.x;
                let hz = c.feet[i].pw.z - r.z;
                let far = hx * hx + hz * hz > 0.37 * 0.37;
                let other_planted = c.feet[1 - i].planted;
                let st_t = c.feet[i].st_t;
                if was_st && !in_st {
                    lift_off(c, i, M_GAIT);
                } else if !in_st && other_planted && st_t > 0.06 {
                    lift_off(c, i, M_CATCH);
                    c.feet[i].dur = clamp((1.0 - p) / cad, 0.09, 0.24);
                } else if far {
                    lift_off(c, i, M_CATCH);
                }
                c.feet[i].st_t += dt;
            }
            if c.feet[i].sw {
                if c.feet[i].mode == M_GAIT {
                    let u = if in_st {
                        1.0
                    } else {
                        (p - duty) / (1.0 - duty)
                    };
                    c.feet[i].su = c.feet[i].su.max(u.min(1.0));
                    let t_rem = if in_st { 0.0 } else { (1.0 - p) / cad };
                    let (tp, yaw_t, tn) = gait_target(c, i, t_rem);
                    let su_now = c.feet[i].su;
                    let k = if su_now > 0.82 {
                        1.0
                    } else {
                        1.0 - (-(dt * 28.0)).exp()
                    };
                    let to = c.feet[i].to;
                    c.feet[i].to = to.lerp(tp, k);
                    c.feet[i].to_yaw = yaw_t;
                    c.feet[i].tn = tn;
                    if c.feet[i].su >= 1.0 {
                        touch_down(c, i, 1.0);
                    }
                } else {
                    let dur = c.feet[i].dur;
                    c.feet[i].su = (c.feet[i].su + dt / dur).min(1.0);
                    let t_rem = (1.0 - c.feet[i].su) * dur;
                    let (tp, yaw_t, tn) = gait_target(c, i, t_rem);
                    let f = &mut c.feet[i];
                    f.to = tp;
                    f.to_yaw = yaw_t;
                    f.tn = tn;
                    if f.su >= 1.0 {
                        touch_down(c, i, 0.8);
                    }
                }
            }
        }
    } else {
        // ≡ L1701-1720: standing — finish steps onto the stance, settle the worst foot
        let mut swinging = 0i32;
        for i in 0..2 {
            if !c.feet[i].sw {
                continue;
            }
            if c.feet[i].mode != M_SETTLE {
                let duty = c.duty;
                let cad = c.cad;
                let su = c.feet[i].su;
                let dur = c.feet[i].dur;
                let mode = c.feet[i].mode;
                let rem = clamp(
                    (1.0 - su)
                        * if mode == M_GAIT {
                            (1.0 - duty) / cad.max(1.5)
                        } else {
                            dur
                        },
                    0.09,
                    0.22,
                );
                c.feet[i].mode = M_SETTLE;
                c.feet[i].dur = rem / (1.0 - su).max(0.05);
                c.feet[i].lift = (c.feet[i].lift * 0.8).max(0.04);
            }
            let dur = c.feet[i].dur;
            c.feet[i].su = (c.feet[i].su + dt / dur).min(1.0);
            let mut to = DVec3::ZERO;
            let yaw = ideal_foot(c, i, &mut to);
            let mut n = DVec3::Y;
            to.y = ground(c, to.x, to.z, &mut n);
            let f = &mut c.feet[i];
            f.to = to;
            f.to_yaw = yaw;
            f.tn = n;
            if f.su >= 1.0 {
                touch_down(c, i, 0.45);
            } else {
                swinging += 1;
            }
        }
        c.settleCd -= dt;
        if swinging == 0 && c.settleCd <= 0.0 {
            let e_l = foot_err(c, 0);
            let e_r = foot_err(c, 1);
            let b = if e_l >= e_r { 0 } else { 1 };
            let e = e_l.max(e_r);
            if e > 0.068 {
                start_settle(c, b, e);
            }
        }
    }
    foot_pose(c, 0);
    foot_pose(c, 1);
    // ≡ L1724-1725: planted-foot yaw vs body drives the hip counter-twist
    let st = &c.st;
    let t_l = wrap_a(c.feet[0].cyaw - (c.yaw + st[2] as f64));
    let t_r = wrap_a(c.feet[1].cyaw - (c.yaw + st[5] as f64));
    c.footTwist = damp(c.footTwist, clamp((t_l + t_r) * 0.5, -1.2, 1.2), 20.0, dt);
}
