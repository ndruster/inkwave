//! Weapon part animation — ported from `character.js` `_animWeapon`
//! (L2127-2149) and `character-weapons.js` `animateWeapon` (L1347-1460).
//!
//! Material / lamp / coil uniform writes (`ledShot`, `emissive*`, `cu.u*`)
//! are render-side (B2) and become no-ops; every part transform and spring
//! write is ported in full.
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively
use glam::DQuat;

use super::anim::{AnimState, BowKin, CharAnim, RunnerState, WeaponInstance};
use super::math::*;

/// The single reused `animateWeapon` state object ≡ `this._wst`
/// (`_animWeapon` L2138-2143). Field meanings per the character-weapons.js
/// L1319-1329 contract.
#[derive(Debug, Clone, Default)]
pub struct Wst {
    pub t: f64,
    pub dt: f64,
    pub near: bool,
    pub hand: i64,
    pub runner: Option<RunnerState>,
    pub since_shoot: f64,
    pub since_flick: f64,
    pub since_release: f64,
    pub charge: f64,
    pub full: bool,
    pub charge_flash: f64,
    pub low_ink: f64,
    pub firing: bool,
    pub rolling: f64,
    pub grounded: bool,
    pub ground_speed: f64,
    pub world_quat: Option<DQuat>,
    /// ≡ `w.bowDraw` (character.js poseBow L488 writes it, bow-model.js L156
    /// reads it). The pose layer owns `CharAnim::bow.draw`; the weapon layer
    /// reads it through here (JS routes the same value over the weapon
    /// instance).
    pub bow_draw: f64,
    /// ≡ `d.drawLen || 0.2` for `animateBow` (L160, L178) — from the weapon
    /// frames table (only the bow def carries it).
    pub bow_len: f64,
}

/// ≡ `_animWeapon(dt, s, w)` (L2127-2149).
pub fn anim_weapon(c: &mut CharAnim, dt: f64, s: &AnimState) {
    // ≡ L2128-2132: the harness kid is local (camera gate never runs) and
    // sits at the game LOD tier, so `near` is always true and `farT` false.
    let near = true;
    // ≡ L2133-2137: far-decimation geometry swap is render-side (B2).
    let mut st = Wst {
        t: c.t,
        dt,
        near,
        hand: 0,
        runner: s.runner,
        ..Wst::default()
    };
    let t = &c.tab.tim;
    st.since_shoot = c.tr[t.t_shoot] as f64;
    st.since_flick = c.tr[t.t_flick] as f64;
    st.since_release = c.lastRelease;
    st.charge = c.charge;
    st.full = c.fullT > 0.0;
    st.charge_flash = c.chargeFlash;
    st.low_ink = c.wLow;
    st.firing = s.firing;
    st.rolling = c.wRoll;
    st.grounded = c.grounded;
    st.ground_speed = c.gv;
    // ≡ poseBow L488: `w.bowDraw = draw` — the pose layer ran before
    // _animWeapon in the same update, so animateBow sees this frame's draw.
    st.bow_draw = c.bow.draw.max(0.0);
    // ≡ `d.drawLen || 0.2` (bow-model.js L160/L178 via def.drawLen): JS's
    // `||` falls back on 0 too, so the Option is filtered, not just unwrapped.
    st.bow_len = c
        .tab
        .weapons
        .get("bow")
        .and_then(|f| f.draw_len)
        .filter(|v| *v != 0.0)
        .unwrap_or(0.2);
    // ≡ L2143: worldQuat only for the slosher (off.getWorldQuaternion) — the
    // B2 scenario holds a slosher for the aim sweep, so compose the off
    // node's world rotation from the current locals (same call point).
    st.world_quat = if near && c.weapon.as_ref().is_some_and(|w| w.kind == "slosher") {
        Some(c.off_world_quat())
    } else {
        None
    };
    c.last_off_wq = st.world_quat;
    if st.world_quat.is_none() {
        c.last_wq_chain.clear();
    }
    if let Some(w) = &mut c.weapon {
        animate_weapon(w, &st);
        // ≡ L2145-2148: dual wield left-hand instance.
        if w.left.is_some() {
            st.hand = 1;
            st.since_shoot = c.tr[t.t_shoot_l] as f64;
            if let Some(lw) = &mut w.left {
                animate_weapon(lw, &st);
            }
        }
    }
}

/// ≡ `animateWeapon(w, st)` (character-weapons.js L1347-1460).
fn animate_weapon(w: &mut WeaponInstance, st: &Wst) {
    // ≡ L1348: kit weapons animate themselves. `blade`'s animateBlade only
    // touches material uniforms (no transform writes) → early return;
    // `mitts`' animateMitts drives the pad punch spring (below).
    let kind = w.kind.clone();
    let dt = (st.dt).clamp(0.0, 0.1);
    if kind == "blade" {
        return;
    }
    if kind == "mitts" {
        anim_mitts(w, st, dt);
        return;
    }
    if kind == "bow" {
        anim_bow(w, st, dt);
        return;
    }
    if kind == "brolly" {
        anim_brolly(w, st, dt);
        return;
    }
    let t = st.t;
    // ≡ L1350: per-instance state init (`if (!w.ps)` runs once at build here).
    let r = st.runner;
    // ≡ L1353-1354: per-instance shot clock.
    let ts = st.since_shoot;
    if w.dual && r.is_some() {
        // runner.sinceHand[hand] — not exercised by the pinned harness.
    }
    // ≡ L1356-1362: near/far LOD visibility is render-side; keep the flag.
    let near = st.near;
    w.near = near;
    // ≡ L1364-1371: roller drum (only when the builder made a drum part).
    if part(w, "drum").is_some() {
        if st.rolling > 0.3 && st.grounded {
            w.drum_w = st.ground_speed / w.drum_r;
        } else {
            w.drum_w *= (-dt * 2.2).exp();
        }
        let ft = st.since_flick;
        if ft >= 0.15 && ft - dt < 0.15 {
            w.drum_w += 34.0;
        }
        w.drum_a += w.drum_w * dt;
        let drum_q = DQuat::from_rotation_x(w.drum_a);
        if let Some(p) = part_mut(w, "drum") {
            p.quat = drum_q;
        }
    }
    let u = st.since_shoot;
    // ≡ L1373-1377: blaster pump stroke (computed at every distance).
    if kind == "blaster" {
        let pk = if u < 0.6 {
            if u < 0.14 {
                0.0
            } else if u < 0.29 {
                mj((u - 0.14) / 0.15)
            } else if u < 0.33 {
                1.0
            } else if u < 0.46 {
                1.0 - mj((u - 0.33) / 0.13)
            } else {
                -0.1 * (std::f64::consts::PI * ((u - 0.46) / 0.12).min(1.0)).sin()
            }
        } else {
            0.0
        };
        w.pump = pk;
    }
    // ≡ L1378-1383: coil / meter uniforms — material-side (B2 no-op).
    if !near {
        return;
    }
    let shot_k = pulse(ts, 0.006, 30.0);
    // ≡ L1386-1391: trigger squeeze.
    if part(w, "trigger").is_some() {
        let want: f64 = if kind == "charger" {
            if st.charge > 0.01 { 1.0 } else { 0.0 }
        } else if kind == "splatling" {
            match r {
                Some(r) => {
                    if r.charging || r.streaming {
                        1.0
                    } else {
                        0.0
                    }
                }
                None => {
                    if st.firing {
                        1.0
                    } else {
                        0.0
                    }
                }
            }
        } else if kind == "blaster" || kind == "dualies" {
            if ts < 0.07 { 1.0 } else { 0.0 }
        } else if st.firing && u < 0.16 {
            1.0
        } else {
            0.0
        };
        w.trig = damp(w.trig, want, if want > w.trig { 45.0 } else { 22.0 }, dt);
        let trig_q = DQuat::from_rotation_x(0.42 * w.trig);
        if let Some(p) = part_mut(w, "trigger") {
            p.quat = trig_q;
        }
    }
    // ≡ L1392-1459: per-family part motion.
    if kind == "shooter" {
        if let Some(p) = part_mut(w, "bolt") {
            p.pos.z = p.base_pos.z - 0.0095 * shot_k;
        }
        let ck = pulse(ts, 0.01, 18.0);
        if let Some(p) = part_mut(w, "can") {
            p.scale = glam::DVec3::new(1.0 + 0.07 * ck, 1.0 + 0.07 * ck, 1.0 - 0.035 * ck);
        }
        // ledShot(P.led, st, shotK, t) — LED emissive material (B2 no-op).
    } else if kind == "dualies" {
        let k = pulse(ts, 0.005, 26.0);
        if let Some(p) = part_mut(w, "slide") {
            p.pos.z = p.base_pos.z - 0.013 * k;
        }
        if let Some(p) = part_mut(w, "slideInk") {
            p.pos.z = p.base_pos.z - 0.013 * k;
        }
    } else if kind == "blaster" {
        let pump = w.pump;
        if let Some(p) = part_mut(w, "pump") {
            p.pos.z = p.base_pos.z - 0.036 * pump;
        }
        let dep = u < 0.36;
        let needle = spr_a(
            &mut w.ps,
            0,
            if dep { -1.95 } else { 1.3 },
            if dep { 9.0 } else { 4.2 },
            if dep { 0.55 } else { 0.28 },
            dt,
        );
        if let Some(p) = part_mut(w, "needle") {
            p.quat = DQuat::from_rotation_z(
                needle + 0.018 * (t * 41.0).sin() * if u > 0.6 { 1.0 } else { 0.3 },
            );
        }
        if ts < dt * 1.5 {
            w.ps[3] = (w.ps[3] as f64 - 7.0) as f32;
        }
        if u >= 0.33 && u - dt < 0.33 {
            w.ps[3] = (w.ps[3] as f64 + 4.5) as f32;
        }
        let bq = clamp(spr_a(&mut w.ps, 2, 0.0, 7.0, 0.22, dt), -0.3, 0.3);
        if let Some(p) = part_mut(w, "bulb") {
            p.scale = glam::DVec3::new(1.0 - 0.35 * bq, 1.0 - 0.35 * bq, 1.0 + 0.9 * bq);
        }
    } else if kind == "charger" {
        let ch = st.charge;
        // `full` (L?) only feeds the lens/ports emissive materials — B2 no-op.
        let btg = -0.03 * ch.clamp(0.0, 1.0);
        let hz = if btg < w.ps[4] as f64 { 5.0 } else { 16.0 };
        let bolt = spr_a(&mut w.ps, 4, btg, hz, 0.32, dt);
        if let Some(p) = part_mut(w, "bolt") {
            p.pos.z = p.base_pos.z + clamp(bolt, -0.034, 0.004);
        }
        // lens / eyepiece / ports emissive materials — B2 no-ops.
        let rel = st.since_release;
        if rel < dt * 1.5 {
            w.heat = 1.0;
        }
        w.heat *= (-dt * 3.2).exp();
    } else if kind == "roller" {
        // LED emissive pulse — B2 no-op.
    } else if kind == "slosher" {
        if let Some(wq) = st.world_quat {
            let av = wq * glam::DVec3::Y;
            let aq = DQuat::from_rotation_arc(av, glam::DVec3::Y);
            // ≡ L1437: `worldQuat.invert().multiply(aq).multiply(worldQuat)` —
            // wq⁻¹·aq·wq (world tilt expressed in bucket space).
            let aq2 = wq.conjugate() * aq * wq;
            let av2 = glam::DVec3::new(aq2.x, aq2.y, aq2.z);
            let sn = av2.length();
            let mut ang = 2.0 * (sn).atan2(aq2.w);
            if ang > std::f64::consts::PI {
                ang -= 2.0 * std::f64::consts::PI;
            }
            let a = ang.clamp(-0.6, 0.6);
            let av2 = if sn > 1e-5 { av2 / sn } else { glam::DVec3::X };
            spr_a(&mut w.ps, 6, av2.x * a, 2.2, 0.16, dt);
            spr_a(&mut w.ps, 8, av2.z * a, 2.2, 0.16, dt);
        }
        let surf_q = DQuat::from_euler(glam::EulerRot::XYZ, w.ps[6] as f64, 0.0, w.ps[8] as f64);
        if let Some(p) = part_mut(w, "surface") {
            p.quat = surf_q;
            let drain = if u < 0.6 {
                if u < 0.16 {
                    mj(u / 0.16)
                } else {
                    1.0 - mj((u - 0.16) / 0.44)
                }
            } else {
                0.0
            };
            p.pos.y = p.base_pos.y - 0.034 * drain + 0.002 * (t * 7.3).sin();
            let rip = 1.0 + 0.02 * (t * 11.0 + 1.3).sin() * (0.3 + drain);
            p.scale = glam::DVec3::new(
                rip * (1.0 - 0.1 * drain),
                1.0,
                (2.0 - rip) * (1.0 - 0.1 * drain),
            );
        }
        if let Some(p) = part_mut(w, "lever") {
            p.quat = DQuat::from_rotation_x(
                -0.4 * if u < 0.3 {
                    1.0 - mj(((u - 0.18).max(0.0)) / 0.12)
                } else {
                    0.0
                },
            );
        }
    } else if kind == "splatling" {
        let mut want = 0.0;
        if let Some(r) = r {
            want = if r.charging {
                14.0 + 46.0 * r.charge
            } else if r.streaming {
                64.0
            } else {
                0.0
            };
        } else if st.firing {
            want = 30.0 + 30.0 * st.charge;
        }
        w.spin_w = damp(w.spin_w, want, if want > w.spin_w { 5.0 } else { 1.6 }, dt);
        w.spin_a = (w.spin_a + w.spin_w * dt) % (std::f64::consts::TAU);
        let spin_q = DQuat::from_rotation_z(w.spin_a);
        if let Some(p) = part_mut(w, "barrels") {
            p.quat = spin_q;
            p.pos.z = p.base_pos.z - 0.004 * shot_k;
        }
    }
}

/// ≡ `animateMitts(w, st)` (mitts-model.js L144-175), harness path: no runner
/// (charge/cling/leap/land/wob ≡ 0), materials untouched.
fn anim_mitts(w: &mut WeaponInstance, st: &Wst, dt: f64) {
    // ≡ L146: per-instance init (Float32Array(8), S[0]=S[2]=1).
    if !w.ms_init {
        w.ms = [0.0; 8];
        w.ms[0] = 1.0;
        w.ms[2] = 1.0;
        w.last_punch = 99.0;
        w.ms_init = true;
    }
    // ≡ L156: `const pad = P.pad; if (!pad) return;`
    if part(w, "pad").is_none() {
        return;
    }
    // ≡ L163: no runner → `st.sinceShoot ?? 99`.
    let ts = st.since_shoot;
    // ≡ L165-166: new-punch edge → kick the spring velocity.
    if ts < w.last_punch {
        w.ms[1] = (w.ms[1] as f64 - 7.5) as f32;
    }
    w.last_punch = ts;
    // ≡ L169: zT = 1 − 0.28·land + 0.05·leap + … ≡ 1 (all runner signals 0).
    let zt = 1.0;
    // ≡ L170-171: pad spring + squash (xy = 1 + (1−z)·0.55).
    let z = spr_a(&mut w.ms, 0, zt, 9.0, 0.3, dt);
    let xy = 1.0 + (1.0 - z) * 0.55;
    if let Some(p) = part_mut(w, "pad") {
        p.scale = glam::DVec3::new(xy, xy, z.max(0.55));
    }
    // ≡ L172-174: glow envelope `S[4] += (g−S[4])·(1−e^{−30dt})` feeds only
    // material uniforms → no transform writes, not ported.
}

fn part<'a>(w: &'a WeaponInstance, name: &str) -> Option<&'a super::anim::PartXform> {
    w.parts.iter().find(|p| p.name == name)
}

fn part_mut<'a>(w: &'a mut WeaponInstance, name: &str) -> Option<&'a mut super::anim::PartXform> {
    w.parts.iter_mut().find(|p| p.name == name)
}

/// bow-model.js geometry constants (L24-30): string ends on the limb tips and
/// the limb roots in the riser pockets, weapon space (nock at the origin).
const TIP_U: glam::DVec3 = glam::DVec3::new(0.0, 0.382, 0.0);
const TIP_L: glam::DVec3 = glam::DVec3::new(0.0, -0.407, 0.0);
const ROOT_U: glam::DVec3 = glam::DVec3::new(0.0, 0.165, 0.118);
const ROOT_L: glam::DVec3 = glam::DVec3::new(0.0, -0.19, 0.118);
/// L26: limb bend at full draw (rad, about each limb's root).
const FLEX: f64 = 0.3;
/// config.js WEAPONS.bow.ring1 (L204); `WEAPONS.bow?.ring1 ?? 0.45`. Not
/// hand-copied blindly: the test below pins it to the extracted tuning table
/// (NFR-M5-6, data over literals).
const BOW_RING1: f64 = 0.45;

/// ≡ `tipAt(tip, root, th, out)` (bow-model.js L140-143): the limb tip swung
/// about the limb root by `th` around X, in riser space.
fn bow_tip_at(tip: glam::DVec3, root: glam::DVec3, th: f64) -> glam::DVec3 {
    let dy = tip.y - root.y;
    let dz = tip.z - root.z;
    let (c, s) = (th.cos(), th.sin());
    glam::DVec3::new(tip.x, root.y + dy * c - dz * s, root.z + dy * s + dz * c)
}

/// ≡ `aimString(g, nock, tip, dirRest, L)` (bow-model.js L144-147): aim the
/// string half from the nock to the (flexed) tip and stretch it along Y.
fn bow_aim_string(
    p: &mut super::anim::PartXform,
    nock: glam::DVec3,
    tip: glam::DVec3,
    dir_rest: glam::DVec3,
    l_rest: f64,
) {
    let d = tip - nock;
    // ≡ L145 `_v2.length() || 1e-4`: JS keeps a true (0, 1e-4) length, Rust
    // lifts it to 1e-4 — unreachable here (len ≳ 0.35 over the str domain).
    let len = d.length().max(1e-4);
    p.pos = nock;
    p.quat = DQuat::from_rotation_arc(dir_rest, d / len);
    p.scale = glam::DVec3::new(1.0, len / l_rest, 1.0);
}

/// ≡ `animateBow(w, st)` (bow-model.js L148-199), harness path: no runner
/// (`drawing = (st.charge||0) > 0.01`, `c = st.charge`); the glow envelope
/// state is tracked but its material writes (heads emissive / coil uniforms)
/// are render-side B2 no-ops, as is the `visible` render cull.
fn anim_bow(w: &mut WeaponInstance, st: &Wst, dt: f64) {
    // ≡ L156-158: the pose layer's draw; no runner in the harness.
    let hand = st.bow_draw.max(0.0);
    let drawing = st.charge > 0.01;
    let c = st.charge;
    let rel = st.since_release;
    // ≡ L150-154 + L160-176: per-instance init and the string spring, all on
    // the scoped `bw` borrow (part writes below need `w` again afterwards).
    // Statement order vs JS (glow after limbs here) carries no data
    // dependency between the blocks — the borrow split forces it.
    let (new_off, str_now) = {
        let s = w.bw.get_or_insert_with(|| BowKin {
            // ≡ L152: rest = the off node's built position, ax = +Z pushed
            // through the off quaternion (the arrow axis; `.normalize()` is a
            // defensive extra — off.quat is unit so JS's `applyQuaternion`
            // keeps length; no-op on real data).
            rest: w.off.pos,
            ax: (w.off.quat * glam::DVec3::new(0.0, 0.0, 1.0)).normalize(),
            str: 0.0,
            str_v: 0.0,
            loose: false,
            last_rel: 99.0,
            glow: 0.4,
        });
        if !w.ms_init {
            // ≡ L153: `w.pump = 0; w.trig = 0;` — already the built-in
            // defaults. Gated by the shared first-call flag (JS's gate is
            // `if (!s)`, L151): equivalent because set_weapon rebuilds the
            // instance per kind, so one instance never runs both paths.
            w.pump = 0.0;
            w.trig = 0.0;
            w.ms_init = true;
        }
        // ≡ L160: the bow rides forward by the fist's draw (stays put in the
        // world).
        let new_off = s.rest + s.ax * (st.bow_len * hand);
        // ≡ L162-165: release edge detector → the string goes loose.
        if rel < s.last_rel - 1e-4 {
            s.loose = true;
            s.str_v = 0.0;
        }
        s.last_rel = rel;
        if drawing {
            s.loose = false;
        }
        // ≡ L166-173: loose string rings (exact underdamped spring toward 0),
        // otherwise it rides the fist.
        if s.loose {
            let om = std::f64::consts::TAU * 15.0;
            let z: f64 = 0.3;
            let wd = om * (1.0 - z * z).sqrt();
            let e = (-z * om * dt).exp();
            let co = (wd * dt).cos();
            let sn = (wd * dt).sin();
            let (x0, v0) = (s.str, s.str_v);
            let bq = (v0 + z * om * x0) / wd;
            s.str = e * (x0 * co + bq * sn);
            s.str_v = e * ((-z * om * x0 + wd * bq) * co + (-z * om * bq - wd * x0) * sn);
            s.str = s.str.max(-0.16);
            if rel > 0.3 && hand < 0.03 && s.str.abs() < 0.02 {
                s.loose = false;
            }
        } else {
            s.str = hand;
            s.str_v = 0.0;
        }
        (new_off, s.str)
    };
    w.off.pos = new_off;
    // ≡ L189-194: glow envelope — state kept (s.glow), material writes
    // (heads.color/emissive/emissiveIntensity) are render-side B2 no-ops.
    let ring = if c >= 0.999 {
        2
    } else if c >= BOW_RING1 {
        1
    } else {
        0
    };
    let want = if drawing {
        0.35 + 1.4 * c
            + if ring >= 1 { 1.4 } else { 0.0 }
            + if ring == 2 {
                2.2 + 0.9 * (st.t * 30.0).sin()
            } else {
                0.0
            }
    } else {
        0.35
    };
    if let Some(s) = w.bw.as_mut() {
        s.glow += (want - s.glow) * (1.0 - (-dt * 18.0).exp());
    }
    // ≡ L175-176: limbs flex with the string.
    let phi = FLEX * str_now.clamp(-0.2, 1.05);
    for (name, rot) in [
        ("limbU", -phi),
        ("limbUInk", -phi),
        ("limbL", phi),
        ("limbLInk", phi),
    ] {
        if let Some(p) = part_mut(w, name) {
            p.quat = DQuat::from_rotation_x(rot);
        }
    }
    // ≡ L178-182: string halves from the nock to the flexed tips.
    let nock = glam::DVec3::new(0.0, 0.0, -st.bow_len * str_now);
    let tu = bow_tip_at(TIP_U, ROOT_U, -phi);
    let tl = bow_tip_at(TIP_L, ROOT_L, phi);
    if let Some(p) = part_mut(w, "stringU") {
        bow_aim_string(p, nock, tu, glam::DVec3::Y, TIP_U.y.abs());
    }
    if let Some(p) = part_mut(w, "stringL") {
        bow_aim_string(p, nock, tl, glam::DVec3::NEG_Y, TIP_L.y.abs());
    }
    // ≡ L184-187: nocked arrows — gone on the release, a fresh trio nocks a
    // beat later (`visible = show > 0.01` is render-side culling; the 0.001
    // scale floor already matches).
    let show = if drawing {
        1.0
    } else if rel < 0.3 {
        0.0
    } else {
        ((rel - 0.3) / 0.12).min(1.0)
    };
    let sc = show.max(0.001);
    for name in ["arrows", "heads", "fletch"] {
        if let Some(p) = part_mut(w, name) {
            p.pos = nock;
            p.scale = glam::DVec3::splat(sc);
        }
    }
    // ≡ L195-198: `w.coil` uniforms — render-side (no coil in the port).
}

/// ≡ `animateBrolly(w, st)` (brolly-model.js L304-381), **transformation
/// surface only**. The canopy gores / tears / lamp / fabric are driven by
/// `st.runner.kit.brolly` (kits/brolly.js runner state machine), which has no
/// Rust counterpart — with `k = null` every gore term reduces to its furled
/// authored pose (`eo = 0`, `kick = 0`, `flutter = 0`, `gs = 1`). The struts
/// (L288, L336-348) are extra meshes JS adds under `w.off` (not in
/// `w.parts`), rebuilt from the gore matrices each frame — no extracted part
/// surface exists for them, so they are out of the port's data model (they
/// stay registered as a visual-review item). What remains transform-side and
/// is ported in full: the runner collar ride (L354) and the trigger follow
/// spring (L355-359). Registered as a deviation in REVIEW_CHECKLIST.
fn anim_brolly(w: &mut WeaponInstance, st: &Wst, dt: f64) {
    // ≡ L354: `P.runner.position.z = rest.z + (Z1−Z0)·clamp(eo)` — eo ≡ 0
    // with no kit state (Z0 = 0.305, brolly-model.js L34), so the collar sits
    // at its authored rest position.
    if let Some(p) = part_mut(w, "runner") {
        p.pos.z = p.base_pos.z;
    }
    // ≡ L355-359: the trigger follows shots (no kit → `open > 0.1` never).
    if part(w, "trigger").is_some() {
        let want = if st.since_shoot < 0.12 { 1.0 } else { 0.0 };
        w.trig += (want - w.trig) * (1.0 - (-(if want != 0.0 { 40.0 } else { 20.0 }) * dt).exp());
        let trig_q = DQuat::from_rotation_x(0.42 * w.trig);
        if let Some(p) = part_mut(w, "trigger") {
            p.quat = trig_q;
        }
    }
    // L361-380: lamp / fabric / opacity — material-side B2 no-ops.
}

#[cfg(test)]
mod tests {
    use super::BOW_RING1;

    /// `BOW_RING1` must track the extracted tuning table, not drift as a
    /// hand-typed literal (NFR-M5-6): ≡ bow-model.js L139
    /// `WEAPONS.bow?.ring1 ?? 0.45` with the config value from
    /// `src/config.js` L204 flowing through `run-all.sh` into
    /// `assets/tuning.json`.
    #[test]
    fn bow_ring1_matches_extracted_tuning() {
        const TUNING: &str = include_str!("../../../../assets/tuning.json");
        let root = inkwave_sim::net::wire::parse_json(TUNING).expect("tuning.json parses");
        let bow = root
            .field("weapons")
            .and_then(|w| w.field("bow"))
            .expect("weapons.bow");
        let ring1 = bow
            .field("ring1")
            .and_then(|v| v.as_f64())
            // ≡ the `?? 0.45` fallback when the config entry is absent.
            .unwrap_or(0.45);
        assert_eq!(BOW_RING1, ring1);
    }
}
