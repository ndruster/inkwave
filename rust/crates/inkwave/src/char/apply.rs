//! Pose application ≡ `character.js` `_applyPose` (L3306-3556), `_kidXform`
//! (L3752-3759), `_solveLimb` (L3760-3802), `_updateTank` (L3737-3751),
//! `_updateDodge` (L868-880).
//!
//! The B2 consumers inside `_applyPose` are ported here (`_applyFace` L3585,
//! `_applyJiggle` L3655, `_applyFingers` L3704) or in `anim.rs`
//! (`_updateMaterials` L4045); render-side-only writes (bomb visibility/scale,
//! `w.spin` mesh rotation is kept as state, `pivotL.visible`) are mirrored
//! as state on the instance, never as scene-graph effects.
#![allow(dead_code)] // Task 54 batch B1/C consumers land progressively
use std::f64::consts::PI;

use glam::{DMat3, DMat4, DQuat, DVec3};

use super::anim::{AnimState, CharAnim, Limb};
use super::hair;
use super::math::*;
use super::squid;
use super::weapon;

/// `THREE.Object3D.updateMatrix()` ≡ compose(T·R·S).
#[inline]
fn compose(pos: DVec3, quat: DQuat, scale: DVec3) -> DMat4 {
    DMat4::from_scale_rotation_translation(scale, quat, pos)
}

/// `Matrix4.makeBasis(newBasis) · Mu0ᵀ` as a quaternion ≡ the scratch matrix
/// path in `_solveLimb` (L3782/L3788): the rotation that carries the rest
/// basis rows `(r, h, c)` onto the new basis columns.
fn basis_quat(new_basis: [DVec3; 3], rest_rows: &[DVec3; 3]) -> DQuat {
    let r = rest_rows[0].to_array();
    let h = rest_rows[1].to_array();
    let c = rest_rows[2].to_array();
    let cols =
        [0usize, 1, 2].map(|j| new_basis[0] * r[j] + new_basis[1] * h[j] + new_basis[2] * c[j]);
    DQuat::from_mat3(&DMat3::from_cols(cols[0], cols[1], cols[2]))
}

impl CharAnim<'_> {
    /// ≡ `_kidXform(o, pos, quat)` (character.js L3752-3759): walk the parent
    /// chain from `bone` up to (exclusive) the `kid` group, accumulating the
    /// LOCAL transforms into a kid-space position + quaternion.
    pub fn kid_xform(&self, bone: usize) -> (DVec3, DQuat) {
        let mut pos = DVec3::ZERO;
        let mut quat = DQuat::IDENTITY;
        let mut o = bone;
        loop {
            let b = &self.bones[o];
            pos = b.quat * pos + b.pos;
            quat = b.quat * quat;
            match self.tab.rig.bone_parent[o] {
                Some(p) => o = p,
                None => break, // hangs off `kid` (`_buildRig` L674)
            }
        }
        (pos, quat)
    }

    /// ≡ `_solveLimb(L, target, pole, endQuat, weight, errSlot)`
    /// (character.js L3760-3802): two-bone IK solved in kid space; writes the
    /// up/lo (and optional end) bone quaternions and `ikErr[errSlot]`.
    pub fn solve_limb(
        &mut self,
        l: &Limb,
        target: DVec3,
        pole: DVec3,
        end_quat: Option<DQuat>,
        weight: f64,
        err_slot: usize,
    ) {
        // `_kidXform(L.up.parent, _sP, _sQp)`
        let (s_p, s_qp) = match self.tab.rig.bone_parent[l.up] {
            Some(p) => self.kid_xform(p),
            None => (DVec3::ZERO, DQuat::IDENTITY),
        };
        let up_pos = self.bones[l.up].pos;
        let p_a = s_qp * up_pos + s_p;
        let a = l.a;
        let b = l.b;
        let mut p_d = target - p_a;
        let mut dist = p_d.length();
        let dmin = (a - b).abs() + 1e-3;
        let dmax = (a + b) * 0.9995;
        self.ikErr[err_slot] = (dist - dmax).max(0.0);
        dist = clamp(dist, dmin, dmax);
        p_d = p_d.normalize();
        let cos_a = clamp((a * a + dist * dist - b * b) / (2.0 * a * dist), -1.0, 1.0);
        let sin_a = (1.0 - cos_a * cos_a).sqrt();
        // `_pN = pole − d̂ (pole·d̂)`; degenerate → `Z − d̂ d̂.z`
        let mut p_n = pole - p_d * pole.dot(p_d);
        if p_n.length_squared() < 1e-8 {
            p_n = DVec3::new(0.0, 0.0, 1.0) - p_d * p_d.z;
        }
        p_n = p_n.normalize();
        let p_e = p_a + p_d * (a * cos_a) + p_n * (a * sin_a);
        let s_t = p_a + p_d * dist;
        let p_h = p_n.cross(p_d).normalize();
        let by = (p_e - p_a).normalize();
        let bz = by.cross(p_h);
        let qa = basis_quat([by, p_h, bz], &l.mu0t);
        let qb = s_qp.inverse() * qa;
        self.bones[l.up].quat = self.bones[l.up].quat.slerp(qb, weight);
        let by2 = (s_t - p_e).normalize();
        let bz2 = by2.cross(p_h);
        let qa2 = basis_quat([by2, p_h, bz2], &l.mf0t);
        let s_qp2 = s_qp * self.bones[l.up].quat;
        let qb2 = s_qp2.inverse() * qa2;
        self.bones[l.lo].quat = self.bones[l.lo].quat.slerp(qb2, weight);
        if let Some(eq) = end_quat {
            let s_qp3 = s_qp2 * self.bones[l.lo].quat;
            let qb3 = s_qp3.inverse() * eq;
            self.bones[l.end].quat = self.bones[l.end].quat.slerp(qb3, weight);
        }
    }
}

/// ≡ `_updateDodge(dt)` (character.js L868-880): the twin dodge-roll tumbles
/// the whole `model` group. Never active in the pinned harness (no `dodge`
/// trigger), but ported for the state machine.
pub fn update_dodge(c: &mut CharAnim, dt: f64) {
    let Some(dg) = c.dodge.as_mut() else { return }; // L870: `if (!this.dodge) return;`
    dg.t += dt;
    let dur = if dg.dur != 0.0 { dg.dur } else { 0.3 }; // L872: `R.dur || 0.3`
    let u = dg.t / dur;
    let dir = dg.dir;
    if u >= 1.0 {
        c.dodge = None;
        c.model.quat = DQuat::IDENTITY; // L873: quaternion.identity(); position.set(0,0,0)
        c.model.pos = DVec3::ZERO;
        return;
    }
    // L875-876: roll direction into root space (engine owns root.rotation.y)
    let ry = c.root_yaw;
    let (cc, sn) = ((-ry).cos(), (-ry).sin());
    let lx = dir.0 * cc + dir.1 * sn;
    let lz = -dir.0 * sn + dir.1 * cc;
    let axis = DVec3::new(lz, 0.0, -lx).normalize(); // L877: tumble axis = up × direction
    let e = if u < 0.5 {
        2.0 * u * u
    } else {
        1.0 - 2.0 * (1.0 - u) * (1.0 - u)
    }; // L878
    let dq = DQuat::from_axis_angle(axis, TAU * e); // L879
    c.model.quat = dq; // L880
    let dc = DVec3::new(0.0, 0.55, 0.0); // L881: _dC
    c.model.pos = dc - dq * dc;
    c.model.pos.y -= (PI * u).sin() * 0.28;
}

/// ≡ `_applyPose(dt, s)` (character.js L3306-3556). Writes the LOCAL
/// transforms of `kid`, the 87 rig bones, the held weapon pivot and the
/// `tankFill` node; the harness records exactly these (`fixture.frames[i].o`).
#[allow(non_snake_case)]
pub fn apply_pose(c: &mut CharAnim, dt: f64, s: &AnimState) {
    let tab = c.tab;
    let ch = &tab.ch;
    let ss = &tab.spr;
    let rc = &tab.rig_c;
    let p = c.P.clone(); // read-only inside `_applyPose`
    let pv = |i: usize| p[i] as f64;
    let bi = |n: &str| tab.rig.bone_index[n];

    // ---- bomb (L3309-3313): the bomb group itself is not a recorded node,
    // but its visibility feeds `_applyFingers`' gL (L3705-3706), so track it.
    // (`this.bomb` is built unconditionally at L763-767, so no null guard.)
    c.bomb_visible = c.bombHeld && c.kidForm && c.wSub > 0.2 && !c.subPropHidden;

    // ---- kid group transform (L3315-3327)
    let sq = c.kidScale;
    let mut sq_y = pv(ch.SQY) * c.kidSY * sq;
    let mut sq_x = pv(ch.SQXZ) * c.kidSXZ * sq;
    sq_x = sq_x.max(1e-3);
    sq_y = sq_y.max(1e-3);
    c.kid.scale = DVec3::new(sq_x, sq_y, sq_x);
    c.kid.quat = euler_yxz(pv(ch.MODELR), pv(ch.MODELR + 1), pv(ch.MODELR + 2));
    if c.tumble != 0.0 {
        let axis = DVec3::new(c.tumbleX, 0.0, c.tumbleZ).normalize();
        c.kid.quat = DQuat::from_axis_angle(axis, c.tumble) * c.kid.quat;
    }
    // L3322-3324: pivot compensation about the lowered centre, then the
    // form-transition lift.
    let v1 = DVec3::new(0.0, if c.tumble != 0.0 { 0.56 } else { 0.62 }, 0.0);
    let v2 = c.kid.quat * v1;
    let kp = DVec3::new(
        pv(ch.MODEL),
        pv(ch.MODEL + 1) - c.tumbleDrop,
        pv(ch.MODEL + 2),
    ) + v1
        - v2;
    c.kid.pos = kp;
    c.kid.pos.y += c.kidLift;
    let q6inv = c.kid.quat.inverse();
    let isx = 1.0 / sq_x;
    let isy = 1.0 / sq_y;

    // ---- foot targets: planted foot pose wins while the foot is planted
    // (L3329-3365)
    let yaw = c.yaw;
    let (cyaw, syaw) = (yaw.cos(), yaw.sin());
    let plant = c.plantW;
    let feet_valid = if c.feetValid { 1.0 } else { 0.0 };
    let shift_side = if c.shiftS > 0.0 { 1usize } else { 0usize };
    let r = c.root_pos;
    for i in 0..2 {
        let (fo, fr) = if i == 0 {
            (ch.FOOTL, ch.FOOTLR)
        } else {
            (ch.FOOTR, ch.FOOTRR)
        };
        let wp = pv(if i == 0 { ch.WPL } else { ch.WPR }) * plant * feet_valid;
        let mut q1 = euler_yxz(pv(fr), pv(fr + 1), pv(fr + 2));
        let mut v3 = DVec3::new(pv(fo), pv(fo + 1), pv(fo + 2));
        if wp > 0.001 {
            let f = &c.feet[i];
            let dx = f.cw.x - r.x;
            let dy = f.cw.y - r.y;
            let dz = f.cw.z - r.z;
            let v4 = DVec3::new(dx * cyaw - dz * syaw, dy, dx * syaw + dz * cyaw);
            let fy = f.cyaw - yaw;
            let mut pitch = f.pitch;
            if !c.moving && i == shift_side {
                pitch += 0.1 * c.shiftS.abs() * (1.0 - c.gaitW);
            }
            let tip = pv(ch.TIPTOE).max(c._toeUp);
            if tip > 0.001 {
                pitch += 0.55 * tip;
            }
            let v5 = DVec3::new(
                f.cn.x * cyaw - f.cn.z * syaw,
                f.cn.y,
                f.cn.x * syaw + f.cn.z * cyaw,
            );
            let mut q2 = DQuat::from_rotation_arc(DVec3::Y, v5);
            q2 *= DQuat::from_rotation_y(fy);
            let (ay, az) = if pitch >= 0.0 {
                (
                    rc.ankle_h * pitch.cos() + rc.ball_z * pitch.sin(),
                    rc.ball_z + rc.ankle_h * pitch.sin() - rc.ball_z * pitch.cos(),
                )
            } else {
                (
                    rc.ankle_h * pitch.cos() - rc.heel_z * pitch.sin(),
                    -rc.heel_z + rc.ankle_h * pitch.sin() + rc.heel_z * pitch.cos(),
                )
            };
            let mut v6 = q2 * DVec3::new(0.0, ay, az) + v4;
            q2 *= DQuat::from_rotation_x(pitch);
            v6 = q6inv * (v6 - c.kid.pos);
            v6.x *= isx;
            v6.y *= isy;
            v6.z *= isx;
            q2 = q6inv * q2;
            v3 = v3.lerp(v6, wp);
            q1 = q1.slerp(q2, wp);
        }
        if i == 0 {
            c._fL = v3;
            c._fLq = q1;
        } else {
            c._fR = v3;
            c._fRq = q1;
        }
    }
    c._toeUp = 0.0;

    // ---- pelvis + hip drop (L3367-3395)
    let hips_i = bi("hips");
    let rest_hips = DVec3::from_array(tab.rig.rest[hips_i]);
    c.bones[hips_i].pos =
        rest_hips + DVec3::new(pv(ch.HIPS_P), pv(ch.HIPS_P + 1), pv(ch.HIPS_P + 2));
    c.bones[hips_i].quat = euler_yxz(pv(ch.HIPS), pv(ch.HIPS + 1), pv(ch.HIPS + 2));
    {
        let leg_l = c.limbs["legL"].clone();
        let leg_r = c.limbs["legR"].clone();
        let up_l = c.bones[leg_l.up].pos;
        let up_r = c.bones[leg_r.up].pos;
        let hips_q = c.bones[hips_i].quat;
        let hips_p = c.bones[hips_i].pos;
        let mut drop = 0.0f64;
        for i in 0..2 {
            let up_pos = if i == 0 { up_l } else { up_r };
            let ft = if i == 0 { c._fL } else { c._fR };
            let f = &c.feet[i];
            let wpl = pv(if i == 0 { ch.WPL } else { ch.WPR });
            if plant > 0.5 && wpl > 0.5 && f.sw && f.su > 0.02 && f.su < 0.9 {
                continue;
            }
            let v4 = hips_q * up_pos + hips_p;
            let hx = ft.x - v4.x;
            let hz = ft.z - v4.z;
            let reach = c.legReach;
            let h2 = hx * hx + hz * hz;
            let vmax = (reach * reach - h2).max(0.0).sqrt();
            let need = v4.y - (ft.y + vmax);
            if need > drop {
                drop = need;
            }
        }
        let drop = drop.min(0.26);
        c.hipDrop = if drop > c.hipDrop {
            damp(c.hipDrop, drop, 40.0, dt)
        } else {
            damp(c.hipDrop, drop, 16.0, dt)
        };
        c.bones[hips_i].pos.y -= (drop * 0.85).max(c.hipDrop);
        // clamp the swing targets to the (post-drop) reachable sphere
        let hips_q2 = c.bones[hips_i].quat;
        let hips_p2 = c.bones[hips_i].pos;
        for i in 0..2 {
            let up_pos = if i == 0 { up_l } else { up_r };
            let v4 = hips_q2 * up_pos + hips_p2;
            let mut ft = if i == 0 { c._fL } else { c._fR };
            let v5 = ft - v4;
            let d = v5.length();
            let mxr = c.legReach * 0.97;
            if d > mxr {
                ft = v4 + v5 * (mxr / d);
                if i == 0 {
                    c._fL = ft;
                } else {
                    c._fR = ft;
                }
            }
        }
    }

    // ---- torso FK (L3397-3408): hips/spine/chest/neck/clavL/clavR are the
    // YXZ bones (`_buildRig` L667); the rest use the default XYZ order.
    c.bones[bi("spine")].quat = euler_yxz(pv(ch.SPINE), pv(ch.SPINE + 1), pv(ch.SPINE + 2));
    c.bones[bi("chest")].quat = euler_yxz(pv(ch.CHEST), pv(ch.CHEST + 1), pv(ch.CHEST + 2));
    c.bones[bi("neck")].quat = euler_yxz(pv(ch.NECK), pv(ch.NECK + 1), pv(ch.NECK + 2));
    c.bones[bi("clavL")].quat = euler_yxz(pv(ch.CLAVL), pv(ch.CLAVL + 1), pv(ch.CLAVL + 2));
    c.bones[bi("clavR")].quat = euler_yxz(pv(ch.CLAVR), pv(ch.CLAVR + 1), pv(ch.CLAVR + 2));
    c.bones[bi("uArmL")].quat = euler_xyz(pv(ch.UARML), pv(ch.UARML + 1), pv(ch.UARML + 2));
    c.bones[bi("uArmR")].quat = euler_xyz(pv(ch.UARMR), pv(ch.UARMR + 1), pv(ch.UARMR + 2));
    c.bones[bi("fArmL")].quat = euler_xyz(pv(ch.FARML), pv(ch.FARML + 1), pv(ch.FARML + 2));
    c.bones[bi("fArmR")].quat = euler_xyz(pv(ch.FARMR), pv(ch.FARMR + 1), pv(ch.FARMR + 2));
    c.bones[bi("handL")].quat = euler_xyz(pv(ch.HANDL), pv(ch.HANDL + 1), pv(ch.HANDL + 2));
    c.bones[bi("handR")].quat = euler_xyz(pv(ch.HANDR), pv(ch.HANDR + 1), pv(ch.HANDR + 2));

    // ---- head (L3398, L3410-3421): the neck chain is solved, then the
    // stabilizer blends toward the counter-rotation of the bent torso.
    let mut q1 = euler_yxz(pv(ch.HEAD), pv(ch.HEAD + 1), pv(ch.HEAD + 2));
    let stab = pv(ch.STAB);
    if stab > 0.001 {
        let (_v1, mut q2) = c.kid_xform(bi("neck"));
        let tp = pv(ch.HIPS) + pv(ch.SPINE) + pv(ch.CHEST) + pv(ch.NECK);
        let trl = pv(ch.HIPS + 2) + pv(ch.SPINE + 2) + pv(ch.CHEST + 2);
        let q3 = euler_yxz(
            -pv(ch.HLP) + pv(ch.HEAD) * 0.5 + tp * 0.4,
            pv(ch.HLY),
            pv(ch.HEAD + 2) + trl * 0.35,
        );
        q2 = q2.inverse() * q3;
        q1 = q1.slerp(q2, stab);
    } else if pv(ch.HLY).abs() + pv(ch.HLP).abs() > 1e-4 {
        q1 = euler_yxz(-pv(ch.HLP), pv(ch.HLY), 0.0) * q1;
    }
    c.bones[bi("head")].quat = q1;

    // ---- legs (L3423-3433)
    let hyw = pv(ch.HIPS + 1);
    for i in 0..2 {
        let leg = if i == 0 {
            c.limbs["legL"].clone()
        } else {
            c.limbs["legR"].clone()
        };
        let sd = if i == 0 { 1.0 } else { -1.0 };
        let ft = if i == 0 { c._fL } else { c._fR };
        let fq = if i == 0 { c._fLq } else { c._fRq };
        let v1 = fq * DVec3::Z;
        let kn = if i == 0 { ch.KNEEL } else { ch.KNEER };
        let pn = DVec3::new(
            v1.x * 0.7 + hyw.sin() * 0.3 + 0.1 * sd + pv(kn),
            0.05 + pv(kn + 1),
            v1.z * 0.7 + hyw.cos() * 0.3 + pv(kn + 2),
        );
        c.solve_limb(&leg, ft, pn, Some(fq), 1.0, if sd > 0.0 { 2 } else { 3 });
    }

    // ---- weapon (L3435-3522)
    if c.weapon.is_some() {
        weapon::anim_weapon(c, dt, s);
        let d = tab.weapons.get(&c.weaponKind).cloned();
        if let Some(d) = d {
            // anchor → right arm IK (L3438-3465)
            let mut aq = euler_yxz(pv(ch.ANCR), pv(ch.ANCR + 1), pv(ch.ANCR + 2));
            let mut ap = DVec3::new(pv(ch.ANC), pv(ch.ANC + 1), pv(ch.ANC + 2));
            let fol = pv(ch.AFOLT) > 0.001 || pv(ch.AFOLR) > 0.001;
            let (cp, cq);
            if fol {
                let x = c.kid_xform(bi("chest"));
                cp = x.0;
                cq = x.1;
                let q2 = DQuat::IDENTITY.slerp(cq, pv(ch.AFOLR));
                let rest_chest = DVec3::from_array(tab.rig.rest[bi("chest")]);
                let v1 = q2 * (ap - rest_chest) + cp;
                ap = ap.lerp(v1, pv(ch.AFOLT));
                aq = q2 * aq;
            } else {
                cp = DVec3::ZERO;
                cq = DQuat::IDENTITY;
            }
            // sway springs (L3450-3458)
            let sw_w = lerp(1.0, 0.3, c.wAim) * (1.0 - c.lockW);
            let sx = spr(
                &mut c.sp,
                ss.s_wp_x,
                clamp(-c.kax * 0.0011, -0.035, 0.035),
                3.0,
                0.34,
                dt,
            );
            let sy = spr(
                &mut c.sp,
                ss.s_wp_y,
                clamp(-c.vyS * 0.002, -0.03, 0.03),
                3.4,
                0.34,
                dt,
            );
            let sz = spr(
                &mut c.sp,
                ss.s_wp_z,
                clamp(-c.kaz * 0.0011, -0.035, 0.035),
                3.0,
                0.34,
                dt,
            );
            let rx = spr(
                &mut c.sp,
                ss.s_wr_x,
                clamp(c.vyS * 0.01, -0.12, 0.12),
                2.6,
                0.32,
                dt,
            );
            let ry = spr(
                &mut c.sp,
                ss.s_wr_y,
                clamp(-c.yawRate * 0.035, -0.22, 0.22),
                2.6,
                0.38,
                dt,
            );
            ap += DVec3::new(sx, sy, sz) * sw_w;
            aq = euler_yxz(rx * sw_w, ry * sw_w, 0.0) * aq;
            // recoil (L3459-3461)
            ap += aq * DVec3::new(0.0, 0.0, -c.rcZ);
            aq *= euler_yxz(-c.rcP, c.sp[ss.s_rc_y] as f64, c.sp[ss.s_rc_r] as f64);
            let q2 = aq * DQuat::from_array(d.hand_r.1);
            let pt = aq * DVec3::from_array(d.hand_r.0) + ap;
            let pn = DVec3::new(pv(ch.POLER), pv(ch.POLER + 1), pv(ch.POLER + 2));
            let ikr = pv(ch.IKR);
            if ikr > 0.001 {
                let armr = c.limbs["armR"].clone();
                c.solve_limb(&armr, pt, pn, Some(q2), ikr, 1);
            }
            let spin = pv(ch.SPIN);
            if let Some(w) = c.weapon.as_mut() {
                w.pivot.quat = DQuat::from_rotation_x(spin);
            }
            if c.dual {
                // dual wield left fist (L3467-3492)
                let mut aq2 = euler_yxz(pv(ch.ANLR), pv(ch.ANLR + 1), pv(ch.ANLR + 2));
                let mut ap2 = DVec3::new(pv(ch.ANL), pv(ch.ANL + 1), pv(ch.ANL + 2));
                if fol {
                    let q2b = DQuat::IDENTITY.slerp(cq, pv(ch.AFOLR));
                    let rest_chest = DVec3::from_array(tab.rig.rest[bi("chest")]);
                    let v1 = q2b * (ap2 - rest_chest) + cp;
                    ap2 = ap2.lerp(v1, pv(ch.AFOLT));
                    aq2 = q2b * aq2;
                }
                ap2 += DVec3::new(sx, sy, sz) * sw_w;
                aq2 = euler_yxz(rx * sw_w, ry * sw_w, 0.0) * aq2;
                ap2 += aq2 * DVec3::new(0.0, 0.0, -c.rcZ2);
                aq2 *= euler_yxz(-c.rcP2, -(c.sp[ss.s_rc_y2] as f64), 0.0);
                let q2c = aq2 * DQuat::from_array(d.hand_l.1);
                let mut pt2 = aq2 * DVec3::from_array(d.hand_l.0) + ap2;
                let ltw = pv(ch.LTW);
                if ltw > 0.001 {
                    pt2 = pt2.lerp(
                        DVec3::new(pv(ch.LTGT), pv(ch.LTGT + 1), pv(ch.LTGT + 2)),
                        ltw,
                    );
                }
                let pn2 = DVec3::new(pv(ch.POLEL), pv(ch.POLEL + 1), pv(ch.POLEL + 2));
                let wgt = clamp(pv(ch.IKL).max(ltw), 0.0, 1.0) * (1.0 - c.bombSwap);
                if wgt > 0.001 {
                    let arml = c.limbs["armL"].clone();
                    c.solve_limb(
                        &arml,
                        pt2,
                        pn2,
                        if ltw > 0.5 { None } else { Some(q2c) },
                        wgt,
                        0,
                    );
                }
                let spinl = -spin;
                let k = 1.0 - c.bombSwap;
                let sc = if k < 1.0 { back_out(k, 2.0) } else { 1.0 };
                if let Some(w) = c.weapon.as_mut()
                    && let Some(wl) = w.left.as_mut()
                {
                    wl.pivot.quat = DQuat::from_rotation_x(spinl);
                    // ≡ L3488-3491: JS writes only rotation/scale here —
                    // pivot.position stays GRIP_HOLE_L from construction.
                    wl.pivot.scale = DVec3::splat(sc.max(0.001));
                    // `wl.pivot.visible = k > 0.01 && !weaponHidden`
                    wl.hidden = !(k > 0.01 && !c.weaponHidden);
                }
            } else if pv(ch.IKL) > 0.001 || pv(ch.LTW) > 0.001 {
                // the left hand holds the weapon's left grip (L3493-3522)
                let (v3, q3) = c.kid_xform(bi("handR"));
                let (wp_pos, wp_quat, wo_pos, wo_quat, pump) = {
                    let w = c.weapon.as_ref().unwrap();
                    (w.pivot.pos, w.pivot.quat, w.off.pos, w.off.quat, w.pump)
                };
                let v4 = q3 * wp_pos + v3;
                let q4 = q3 * wp_quat;
                let v5 = q4 * wo_pos + v4;
                let q5 = q4 * wo_quat;
                let mut pt3 = DVec3::from_array(d.hand_l.0);
                if pump != 0.0 {
                    pt3.z -= 0.036 * pump;
                }
                pt3 = q5 * pt3 + v5;
                let q2d = q5 * DQuat::from_array(d.hand_l.1);
                let pn3 = DVec3::new(pv(ch.POLEL), pv(ch.POLEL + 1), pv(ch.POLEL + 2));
                let ltw = pv(ch.LTW);
                let ikl = pv(ch.IKL);
                let arml = c.limbs["armL"].clone();
                if ltw > 0.001 {
                    pt3 = pt3.lerp(
                        DVec3::new(pv(ch.LTGT), pv(ch.LTGT + 1), pv(ch.LTGT + 2)),
                        ltw,
                    );
                    let wgt = ikl.max(ltw);
                    c.solve_limb(
                        &arml,
                        pt3,
                        pn3,
                        if ikl > ltw { Some(q2d) } else { None },
                        wgt,
                        0,
                    );
                } else {
                    c.solve_limb(&arml, pt3, pn3, Some(q2d), ikl, 0);
                    c.ikErrPre = c.ikErr[0];
                    if ikl > 0.5 && c.ikErr[0] > 0.0005 {
                        // protract the left shoulder until the grip is reachable
                        let clav = bi("clavL");
                        let cy0 = euler_from_yxz(c.bones[clav].quat).1;
                        let mut it = 0usize;
                        while it < 3 && c.ikErr[0] > 0.0005 {
                            let (cx, mut cy, cz) = euler_from_yxz(c.bones[clav].quat);
                            if cy <= cy0 - 0.55 {
                                break;
                            }
                            let extra = clamp(c.ikErr[0] * 12.0, 0.0, 0.3);
                            cy -= extra;
                            c.bones[clav].quat = euler_yxz(cx - extra * 0.35, cy, cz);
                            c.solve_limb(&arml, pt3, pn3, Some(q2d), ikl, 0);
                            it += 1;
                        }
                    }
                }
            }
        }
    }

    // ---- face (L3524-3526)
    if c.lifeLv > 0.0 {
        apply_face(c, dt);
    }

    // ---- hair (L3525-3528): every frame while `lifeLv > 0`; the odd-frame
    // accumulator branch is short-circuited exactly like JS.
    let lv = c.lifeLv;
    let run_hair = lv > 0.0 || {
        c._hairOdd = !c._hairOdd;
        !c._hairOdd
    };
    if run_hair {
        let step = dt + c._hairAcc;
        c._hairAcc = 0.0;
        hair::update_hair(c, step);
    } else {
        c._hairAcc += dt;
    }

    // ---- tank ink slosh (L3530)
    squid::update_tank(c, dt);

    // ---- spinner / zooka barrel cluster (L3532-3538)
    let has_spin = c.weapon.as_ref().is_some_and(|w| w.has_spin);
    if has_spin {
        let want = if c.lastShot < 0.12 {
            38.0
        } else {
            c.charge * 30.0
        };
        c.spinRate = damp(
            c.spinRate,
            want,
            if want > c.spinRate { 6.0 } else { 1.6 },
            dt,
        );
        c.spinAngle = (c.spinAngle + c.spinRate * dt) % TAU;
        if let Some(w) = c.weapon.as_mut() {
            w.spin_mesh_angle = c.spinAngle; // `w.spin.rotation.z`
        }
    }
    // `w.pivotL.visible` (L3539): render-side only.

    // ---- dodge roll (L3540)
    update_dodge(c, dt);

    // ---- jiggle bones + articulated hands (L3541-3544)
    if c.lifeLv > 0.0 {
        apply_jiggle(c, dt);
        apply_fingers(c, dt);
    }

    // ---- head-turn anchor for the next frame (L3545)
    let head_i = bi("head");
    c._headQW = c.bones[head_i].quat;
    c._headSet = true;

    // ---- record the planted feet in WORLD space (L3546-3555)
    let kid_mat = compose(c.kid.pos, c.kid.quat, c.kid.scale);
    for i in 0..2 {
        let bone = if i == 0 { bi("footL") } else { bi("footR") };
        let (mut v1, q1) = c.kid_xform(bone);
        v1 = kid_mat.transform_point3(v1);
        let r = c.root_pos;
        let disp = DVec3::new(
            r.x + v1.x * cyaw + v1.z * syaw,
            r.y + (v1.y - rc.ankle_h).max(0.0),
            r.z - v1.x * syaw + v1.z * cyaw,
        );
        let v2 = c.kid.quat * (q1 * DVec3::Z);
        c.feet[i].disp = disp;
        c.feet[i].disp_yaw = yaw + v2.x.atan2(v2.z);
        c.feet[i].disp_ok = true;
    }
}

/// ≡ `_applyFace(P, dt)` (character.js L3585-3653): gaze solve, lids, brows,
/// mouth decal uniforms and the face channel bank; optional jaw/cheek/lid
/// bones. Uniform + `face` writes land in `c.mat` / `c.face`.
#[allow(non_snake_case)]
pub fn apply_face(c: &mut CharAnim, dt: f64) {
    let tab = c.tab;
    let ch = &tab.ch;
    let p = c.P.clone();
    let pv = |i: usize| p[i] as f64;
    let bi = |n: &str| tab.rig.bone_index[n];

    // ---- gaze: exact eye-in-head direction to the attention point (L3588-3597)
    let mut ex = pv(ch.LOOKX) / 0.9;
    let mut ey = (pv(ch.LOOKY) - 0.02) / 0.8;
    if c.att.on {
        let q6inv = c.kid.quat.inverse();
        let mut v1 = q6inv * (c.att.p - c.kid.pos);
        v1.x /= c.kid.scale.x;
        v1.y /= c.kid.scale.y;
        v1.z /= c.kid.scale.z;
        let (v2, q1) = c.kid_xform(bi("head"));
        let v3 = q1 * DVec3::from_array(tab.eye_mid) + v2;
        let v1 = q1.inverse() * (v1 - v3);
        let d = v1.length();
        if d > 0.05 {
            ex = v1.x.atan2(v1.z);
            ey = v1.y.atan2(v1.x.hypot(v1.z));
            c.att.dist = d;
        }
    }
    // external head turn carries the eyes the rest of the way (L3599)
    if c.extGl.abs() > 0.06 {
        ex = c.extGl * 0.62;
        ey = -0.03;
    }
    c.gaze_tick(dt, ex, ey);
    let gzx = c.gz.x;
    let gzy = c.gz.y;
    c.face.verge = 0.062f64.atan2(c.att.dist.max(0.3)); // L3602
    // ---- lids: blink (per eye) + wink + squint; the upper lid rides the gaze (L3604-3610)
    let sqz = clamp(pv(ch.SQUINT) + 0.35 * c.bl.squeeze * c.blinkK, 0.0, 1.0);
    let follow = clamp(-gzy * 0.75, 0.0, 0.3) - clamp(gzy * 0.3, 0.0, 0.1);
    let base = clamp(
        pv(ch.EYE) - 0.22 * sqz - follow + 0.04 * (pv(ch.PUPIL) - 0.5),
        0.07,
        1.25,
    );
    let open = (base * (1.0 - 0.94 * c.blinkL)).max(0.07);
    let open_r =
        (base * (1.0 - 0.94 * c.blinkR) * (1.0 - 0.93 * clamp(pv(ch.WINK), 0.0, 1.0))).max(0.07);
    c.bones[bi("eyeL")].scale = DVec3::new(1.0 + 0.06 * (1.0 - open), open, 1.0);
    c.bones[bi("eyeR")].scale = DVec3::new(1.0 + 0.06 * (1.0 - open_r), open_r, 1.0);
    // ---- brows (+ asymmetry channel), mouth decal, gaze uniform (L3612-3620)
    let bra = clamp(pv(ch.BRAS), -1.0, 1.0);
    c.bones[bi("browL")].quat = DQuat::from_rotation_z(-pv(ch.BROW) * 0.42 - 0.12 * bra);
    c.bones[bi("browR")].quat = DQuat::from_rotation_z(pv(ch.BROW) * 0.42 - 0.12 * bra);
    c.bones[bi("browL")].pos.y = c.faceRestBrowL.y + (pv(ch.BROWY) + 0.6 * bra) * 0.008
        - 0.004 * sqz
        + (1.0 - open) * -0.004;
    c.bones[bi("browR")].pos.y = c.faceRestBrowR.y + (pv(ch.BROWY) - 0.6 * bra) * 0.008
        - 0.004 * sqz
        + (1.0 - open_r) * -0.004;
    let mo = clamp(pv(ch.MOPEN), 0.0, 1.0);
    c.face.smile = clamp((pv(ch.MCURVE) - 0.3) / 0.6, 0.0, 1.0);
    c.mat.u_mouth = [
        clamp(pv(ch.MCURVE), -1.2, 1.2),
        clamp(pv(ch.MWIDTH), 0.2, 1.4),
        mo,
        pv(ch.MTILT),
    ];
    let lx = clamp(gzx * 0.9, -0.4, 0.4) * 0.55;
    let ly = clamp(gzy * 0.8 + 0.02, -0.4, 0.4) * 0.5;
    c.mat.u_look = [lx, ly];
    // face hooks (L3624-3631): the uniforms always exist (makeCharUniforms)
    let vg = c.face.verge * if c.att.on { 1.0 } else { 0.0 };
    let ex2 = 0.9 * gzx - lx * 1.25;
    let ey2 = 0.9 * gzy - ly * 1.25;
    c.mat.u_gaze = [ex2 - vg, ey2, ex2 + vg, ey2];
    let lo = clamp(0.4 * sqz + 0.22 * c.face.smile, 0.0, 0.6);
    c.mat.u_lid = [0.0, 0.0, lo, lo];
    c.mat.u_pupil = pv(ch.PUPIL);
    c.mat.u_mouth2 = [
        c.face.smile,
        clamp(pv(ch.SNEER), 0.0, 1.0),
        clamp(pv(ch.PUCKER), 0.0, 1.0),
        clamp(pv(ch.BLUSH), 0.0, 1.0),
    ];
    // ---- face channels (L3633-3637)
    let wink = clamp(pv(ch.WINK), 0.0, 1.0);
    let eye_denom = pv(ch.EYE).max(0.07);
    c.face.blinkL = c.blinkL;
    c.face.blinkR = c.blinkR.max(wink);
    c.face.lidL = 1.0 - open / eye_denom;
    c.face.lidR = 1.0 - open_r / eye_denom;
    c.face.squintL = sqz;
    c.face.squintR = sqz;
    c.face.gazeX = gzx;
    c.face.gazeY = gzy;
    c.face.pupil = pv(ch.PUPIL);
    c.face.browInL = clamp(pv(ch.BROWY) * 0.6 + pv(ch.BROW) * 0.5, -1.0, 1.0);
    c.face.browInR = c.face.browInL;
    c.face.browOutL = clamp(pv(ch.BROWY) + 0.6 * bra, -1.0, 1.0);
    c.face.browOutR = clamp(pv(ch.BROWY) - 0.6 * bra, -1.0, 1.0);
    c.face.furrow = clamp(-pv(ch.BROW), 0.0, 1.0);
    c.face.curve = pv(ch.MCURVE);
    c.face.width = pv(ch.MWIDTH);
    c.face.open = mo;
    c.face.tilt = pv(ch.MTILT);
    c.face.jaw = mo;
    c.face.breath = pv(ch.BREATH);
    c.face.sneer = pv(ch.SNEER);
    c.face.pucker = pv(ch.PUCKER);
    // ---- optional rig bones (L3638-3652)
    if let Some(jaw) = c.xb.jaw {
        c.bones[jaw].quat = DQuat::from_rotation_x(mo * 0.35);
    }
    if c.xb.cheekL.is_some() || c.xb.cheekR.is_some() {
        let mc = pv(ch.MCURVE);
        let smile = clamp(
            clamp((mc - 0.3) / 0.6, 0.0, 1.0) * (1.0 - 0.4 * mo)
                + if mc > 0.3 {
                    clamp(mo - 0.35, 0.0, 1.0) * 0.9
                } else {
                    0.0
                },
            0.0,
            1.0,
        );
        let pout = clamp((-mc - 1.2) / 0.4, 0.0, 1.0);
        let puff = clamp(smile.max(pout * 0.85) + 0.3 * sqz, 0.0, 1.0);
        for ci in 0..2 {
            let b = if ci == 0 { c.xb.cheekL } else { c.xb.cheekR };
            let Some(b) = b else { continue };
            let r = c.cheekRest[ci];
            c.bones[b].pos = DVec3::new(r.x, r.y + 0.0035 * smile + 0.001 * pout, r.z);
            c.bones[b].scale = DVec3::splat(1.0 + 0.06 * puff);
        }
    }
    if let Some(l) = c.xb.lidL {
        c.bones[l].quat = DQuat::from_rotation_x((1.0 - open) * 1.2);
    }
    if let Some(l) = c.xb.lidR {
        c.bones[l].quat = DQuat::from_rotation_x((1.0 - open_r) * 1.2);
    }
}

/// ≡ `_applyJiggle(P, dt)` (character.js L3655-3700): toes, tee hem flaps,
/// backpack sway, ears.
#[allow(non_snake_case)]
pub fn apply_jiggle(c: &mut CharAnim, dt: f64) {
    let tab = c.tab;
    let ch = &tab.ch;
    let ss = &tab.spr;
    let p = c.P.clone();
    let pv = |i: usize| p[i] as f64;
    let xb = c.xb;

    // ---- toes stay flat on the ground while the heel peels up (L3658-3666)
    for i in 0..2 {
        let toe = if i == 0 { xb.toeL } else { xb.toeR };
        let Some(toe) = toe else { continue };
        let (planted, sw, su, pitch) = {
            let f = &c.feet[i];
            (f.planted, f.sw, f.su, f.pitch)
        };
        let wp = pv(if i == 0 { ch.WPL } else { ch.WPR })
            * c.plantW
            * if c.feetValid { 1.0 } else { 0.0 };
        let on_gnd = if planted || !sw {
            1.0
        } else {
            (1.0 - su * 5.0).max(0.0)
        };
        let (fo, fr) = if i == 0 {
            (ch.FOOTL, ch.FOOTLR)
        } else {
            (ch.FOOTR, ch.FOOTRR)
        };
        let posed = pv(fr) * sstep(0.05, 0.005, pv(fo + 1) - tab.rig_c.ankle_h);
        c.bones[toe].quat =
            DQuat::from_rotation_x(-clamp(lerp(posed, pitch * on_gnd, wp), 0.0, 0.5));
    }
    // ---- tee hem (L3668-3676)
    if xb.hemF.is_some() || xb.hemB.is_some() || xb.hem.is_some() {
        let fl = spr(
            &mut c.sp,
            ss.s_hemp,
            clamp(c.kaz * 0.005 + c.gs * 0.022 * c.gaitW, -0.3, 0.3),
            3.0,
            0.2,
            dt,
        );
        let lift = spr(
            &mut c.sp,
            ss.s_hemv,
            clamp(-c.vyS * 0.018, -0.1, 0.25),
            3.4,
            0.25,
            dt,
        );
        let lat = spr(
            &mut c.sp,
            ss.s_hemr,
            clamp(-c.kax * 0.005, -0.15, 0.15),
            3.0,
            0.25,
            dt,
        );
        let flut = 0.05 * (TAU * 2.0 * c.phase + 0.7).sin() * c.gaitW * c.runW;
        if let Some(b) = xb.hemF {
            c.bones[b].quat = euler_xyz(
                clamp(fl.min(0.0) - lift.max(0.0) - flut.max(0.0), -0.25, 0.0),
                0.0,
                lat,
            );
        }
        if let Some(b) = xb.hemB {
            c.bones[b].quat = euler_xyz(
                clamp(fl.max(0.0) + lift.max(0.0) + (-flut).max(0.0), 0.0, 0.25),
                0.0,
                lat,
            );
        }
        if let Some(b) = xb.hem {
            c.bones[b].quat = euler_xyz(clamp(fl, -0.25, 0.25), 0.0, lat);
        }
    }
    // ---- backpack: hangs from the straps (L3678-3686)
    if let Some(b) = xb.tank {
        if c.tankRest.is_none() {
            c.tankRest = Some((c.bones[b].pos, DVec3::ZERO));
        }
        let rest = c.tankRest.unwrap().0;
        let bob = c.gaitW * c.runW * (TAU * 2.0 * (c.phase - c.duty * 0.5)).cos();
        let ty = spr(
            &mut c.sp,
            ss.s_tk_y,
            clamp(-c.headAcc.y * 0.001, -0.012, 0.012),
            4.5,
            0.22,
            dt,
        );
        let rx = spr(
            &mut c.sp,
            ss.s_tk_x,
            clamp(c.kaz * 0.0022 + 0.035 * bob - c.vyS * 0.004, -0.12, 0.12),
            3.0,
            0.26,
            dt,
        );
        let rz = spr(
            &mut c.sp,
            ss.s_tk_z,
            clamp(
                -c.kax * 0.002 + 0.02 * (TAU * c.phase).sin() * c.gaitW,
                -0.1,
                0.1,
            ),
            3.0,
            0.26,
            dt,
        );
        c.bones[b].pos = DVec3::new(rest.x, rest.y + ty, rest.z);
        c.bones[b].quat = euler_xyz(rx, 0.0, rz);
    }
    // ---- ears: perk / droop with the mood, trail head turns, twitch (L3688-3699)
    if xb.earL.is_some() || xb.earR.is_some() {
        c.earT -= dt;
        if c.earT <= 0.0 {
            c.earT = 3.0 + c.rng.next() * 6.0;
            let e = if c.rng.next() < 0.5 {
                ss.s_ear_l
            } else {
                ss.s_ear_r
            };
            c.sp[e + 1] = (c.sp[e + 1] as f64 + if c.rng.next() < 0.7 { 5.0 } else { -4.0 }) as f32;
        }
        // left-ear convention: + perk, − droop; hats tuck the ears down
        let hat = c.style[4];
        let hd = if hat == 3 {
            -0.35
        } else if hat == 1 {
            -0.3
        } else {
            0.0
        };
        let e = clamp(pv(ch.EARS), -1.0, 1.0);
        let base = hd
            + if e >= 0.0 {
                (if hd != 0.0 { 0.04 } else { 0.2 }) * e
            } else {
                0.3 * e
            };
        c.sp[ss.s_ear_l + 1] =
            (c.sp[ss.s_ear_l + 1] as f64 + (c.headRY * 1.2 - c.headRZ * 1.6) * 60.0 * dt) as f32;
        c.sp[ss.s_ear_r + 1] =
            (c.sp[ss.s_ear_r + 1] as f64 + (-c.headRY * 1.2 - c.headRZ * 1.6) * 60.0 * dt) as f32;
        let a_l = spr(&mut c.sp, ss.s_ear_l, base, 3.6, 0.2, dt);
        let a_r = spr(&mut c.sp, ss.s_ear_r, base, 3.6, 0.2, dt);
        let (e_hi, e_lo) = if hd != 0.0 {
            (hd + 0.08, hd - 0.3)
        } else {
            (0.35, -0.45)
        };
        if let Some(b) = xb.earL {
            c.bones[b].quat = DQuat::from_rotation_z(clamp(a_l, e_lo, e_hi));
        }
        if let Some(b) = xb.earR {
            c.bones[b].quat = DQuat::from_rotation_z(-clamp(a_r, e_lo, e_hi));
        }
    }
}

/// ≡ `_applyFingers(P, dt)` (character.js L3704-3735): grip springs, idle
/// re-grips, per-finger curl/spread, thumb. `h`: −1 fist · 0 grip · 1 relaxed ·
/// 2 open; a gripped weapon or the bomb forces 0.
#[allow(non_snake_case)]
pub fn apply_fingers(c: &mut CharAnim, dt: f64) {
    let tab = c.tab;
    let ch = &tab.ch;
    let ss = &tab.spr;
    let p = c.P.clone();
    let pv = |i: usize| p[i] as f64;

    let bomb_on = c.bomb_visible;
    let g_l = (clamp(pv(ch.IKL), 0.0, 1.0) * (1.0 - clamp(pv(ch.LTW), 0.0, 1.0))).max(if bomb_on {
        1.0
    } else {
        0.0
    });
    // ---- micro-grip: squeeze on shots / hits, idle re-grip (L3709-3713)
    c.gripT -= dt;
    if c.gripT <= 0.0 {
        c.gripT = 4.5 + c.rng.next() * 6.0;
        if c.lifeLv == 2.0 && c.kidForm && c.dance.is_none() && c.wAim < 0.2 && c.gaitW < 0.3 {
            c.sp[ss.s_grip + 1] = (c.sp[ss.s_grip + 1] as f64 - 3.2) as f32;
            if g_l > 0.5 && c.rng.next() < 0.5 {
                c.sp[ss.s_grip_l + 1] = (c.sp[ss.s_grip_l + 1] as f64 - 2.6) as f32;
            }
        }
    }
    let sq_r = spr(&mut c.sp, ss.s_grip, 0.0, 5.2, 0.42, dt);
    let sq_l = spr(&mut c.sp, ss.s_grip_l, 0.0, 5.2, 0.42, dt);
    c.handS[0] = damp(
        c.handS[0] as f64,
        clamp(lerp(pv(ch.HANDPL), 0.0, g_l), -1.0, 2.0),
        20.0,
        dt,
    ) as f32;
    c.handS[1] = damp(c.handS[1] as f64, clamp(pv(ch.HANDPR), -1.0, 2.0), 20.0, dt) as f32;
    let t = c.t;
    for sd in 0..2 {
        let Some(f) = c.fing[sd] else { continue };
        let side = if sd == 0 { 1.0 } else { -1.0 };
        let h = c.handS[sd] as f64;
        let gq = clamp((if sd == 0 { sq_l } else { sq_r }) * 0.05, -0.2, 0.14)
            * clamp(1.0 - h.abs(), 0.0, 1.0);
        let f1 = hk(h, -0.35, 0.0, 0.45, 0.9) - gq;
        let f2 = hk(h, -0.5, 0.0, 0.35, 0.75) - gq * 1.3;
        let relax_w = clamp(h, 0.0, 1.0) * clamp(2.0 - h, 0.0, 1.0);
        let open_w = clamp(h - 1.0, 0.0, 1.0);
        let life = 0.025 * (t * 0.9 + sd as f64 * 2.1).sin() * relax_w;
        // trigger finger: the right index follows the weapon's trigger blade (L3725)
        let trig = if sd == 1 {
            c.weapon.as_ref().map_or(0.0, |w| w.trig) * clamp(1.0 - h.abs(), 0.0, 1.0)
        } else {
            0.0
        };
        for k in 0..4 {
            let casc = (k as f64 - 1.5) * 0.07 * relax_w;
            let sq = if k == 0 { trig } else { 0.0 };
            c.bones[f.f1[k]].quat = euler_xyz(
                tab.finger_spread[k] * (open_w + 0.35 * relax_w),
                0.0,
                side * (f1 - casc + life - 0.3 * sq),
            );
            c.bones[f.f2[k]].quat =
                DQuat::from_rotation_z(side * (f2 - casc * 0.8 + life * 0.5 - 0.35 * sq));
        }
        c.bones[f.t1].quat = euler_xyz(
            hk(h, 0.0, 0.0, 0.35, 0.55),
            side * hk(h, -0.1, 0.0, 0.0, 0.0),
            side * hk(h, 0.12, 0.0, 0.0, 0.0),
        );
        c.bones[f.t2].quat = DQuat::from_rotation_z(side * hk(h, 0.3, 0.0, -0.25, -0.5));
    }
}
