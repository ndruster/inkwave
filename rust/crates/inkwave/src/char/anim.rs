//! `CharAnim` — the per-character animation state machine, ported from
//! `character.js`: constructor (L514-648), `_buildRig` (L662-753),
//! `_buildTank`/`_buildSquid` (L775-797), `setWeapon` (L1122-1139),
//! `trigger` (L1140-1223), `_recoil`/`_hairKick` (L1299-1312), `update`
//! (L1318-1366), `_trackRoot` (L1369-1417), `_updateStates` (L1420-1498),
//! `_startFidget` (L1500-1506), `_updateFormScales` (L1514-1534),
//! `_formEnter` (L1537-1544).
//!
//! Batch B2 layers face / fingers / jiggle (`apply.rs`), dances, gaze /
//! micro-expressions, one-shot pose layers (`build.rs`) and the pure-data
//! material pass (`update_materials` below) are all ported. Only the five
//! methods listed in the fixture header `meta.b2Noops` are patched out on
//! both sides (render-side uniforms / locker / menu attention), so the
//! mulberry32 stream stays aligned with the JS reference harness.
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively
use std::collections::HashMap;

use glam::{DMat4, DQuat, DVec3};

use super::build;
use super::feet;
use super::math::*;
use super::squid;
use super::tables::{HoldDef, Tables};
use super::weapon;

/// A node's LOCAL transform in the JS scene graph (position + quaternion +
/// scale); world matrices are composed by the pinned test (≡ THREE.Object3D).
#[derive(Debug, Clone)]
pub struct GroupXform {
    pub pos: DVec3,
    pub quat: DQuat,
    pub scale: DVec3,
}

impl Default for GroupXform {
    fn default() -> Self {
        Self {
            pos: DVec3::ZERO,
            quat: DQuat::IDENTITY,
            scale: DVec3::ONE,
        }
    }
}

/// One weapon part (trigger / bolt / can / led …) ≡ a mesh under `off`
/// (`_weaponInstance` L838-870); `base_pos` = JSON mount pivot.
#[derive(Debug, Clone)]
pub struct PartXform {
    pub name: String,
    pub base_pos: DVec3,
    pub pos: DVec3,
    pub quat: DQuat,
    pub scale: DVec3,
}

/// The held-weapon instance ≡ `this.weapon`. Moving-part state is owned by
/// `animateWeapon` (≡ character.js L863 comment); the per-instance banks
/// mirror the `w.ps / w.pump / w.trig / w.heat / w.drumW / …` fields created
/// at character-weapons.js L1350.
#[derive(Debug, Clone)]
pub struct WeaponInstance {
    pub kind: String,
    pub pivot: GroupXform,
    pub off: GroupXform,
    pub muzzle_pos: DVec3,
    pub parts: Vec<PartXform>,
    /// dual-wield left-hand twin (≡ `w.left`, only for `d.dual && d.inHandL`).
    pub left: Option<Box<WeaponInstance>>,
    /// ≡ `d.dual` on the def (per-instance shot clock switch, L1354).
    pub dual: bool,
    /// ≡ `w.ps`: Float32Array(12), `ps[0] = 1.3` (needle), 2/3 gauge,
    /// 4/5 charger bolt, 6..9 slosher surface springs.
    pub ps: [f32; 12],
    /// blaster slide pump (≡ `w.pump`, read by `_applyPose` L3436+).
    pub pump: f64,
    /// shooter trigger rotation (≡ `w.trig`).
    pub trig: f64,
    /// charger muzzle-port heat (≡ `w.heat`, material-side read → B2).
    pub heat: f64,
    /// roller drum angular state (≡ `w.drumW` / `w.drumA`).
    pub drum_w: f64,
    pub drum_a: f64,
    /// ≡ `d.drumR || 0.1` (L1365); no drum part in the shooter family.
    pub drum_r: f64,
    /// splatling barrel cluster (≡ `w.spinW` / `w.spinA`).
    pub spin_w: f64,
    pub spin_a: f64,
    /// `w.spin` ≡ spinner / zooka barrel cluster (`char.json:frames.spinAt`),
    /// turned by `_applyPose` L3532-3538. `spin_mesh_angle` ≡
    /// `w.spin.rotation.z`.
    pub has_spin: bool,
    pub spin_mesh_angle: f64,
    pub near: bool,
    pub far_dec: f64,
    pub hidden: bool,
    /// ≡ `w.ms` (mitts-model.js L146): Float32Array(8) punch spring
    /// (`[0]` pad-z position, `[1]` velocity, `[4]` glow envelope).
    pub ms: [f32; 8],
    /// ≡ `if (!w.ms)` first-call init flag.
    pub ms_init: bool,
    /// ≡ `w.lastPunch` (punch-edge detector, L165-166).
    pub last_punch: f64,
}

/// IK limb ≡ `this.limbs.<name>` (`_buildRig` L684-695): `a`/`b` are segment
/// lengths, `ru`/`rf` the rest directions of the two bones (kid space),
/// `Mu0T`/`Mf0T` the rest basis matrices (rows = basis axes).
#[derive(Debug, Clone)]
pub struct Limb {
    pub up: usize,
    pub lo: usize,
    pub end: usize,
    pub a: f64,
    pub b: f64,
    pub ru: DVec3,
    pub rf: DVec3,
    pub mu0t: [DVec3; 3],
    pub mf0t: [DVec3; 3],
}

/// One foot ≡ `_mkFoot` (character.js L650-659).
#[derive(Debug, Clone)]
pub struct Foot {
    pub i: usize,
    pub side: f64,
    pub pw: DVec3,
    pub yaw: f64,
    pub n: DVec3,
    pub planted: bool,
    pub in_st: bool,
    pub st_u: f64,
    pub st_t: f64,
    pub sw: bool,
    pub mode: i64,
    pub su: f64,
    pub dur: f64,
    pub from: DVec3,
    pub to: DVec3,
    pub from_yaw: f64,
    pub to_yaw: f64,
    pub lift: f64,
    pub toe: f64,
    pub land: f64,
    pub tn: DVec3,
    pub cw: DVec3,
    pub cyaw: f64,
    pub pitch: f64,
    pub cn: DVec3,
    pub disp: DVec3,
    pub disp_yaw: f64,
    pub disp_ok: bool,
}

/// ≡ `this.bl` (character.js L604-605): the blink envelope.
#[derive(Debug, Clone)]
pub struct Blink {
    pub t: f64,
    pub c: f64,
    pub h: f64,
    pub o: f64,
    pub amp: f64,
    pub lag: f64,
    pub next: f64,
    pub dbl: bool,
    pub squeeze: f64,
}

/// Argument of `trigger(name, arg)` as the harness fixture encodes it (JS
/// `arg` may be undefined, a number, or a plain object).
#[derive(Debug, Clone, Copy)]
pub enum TrigArg {
    None,
    Num(f64),
    /// `hit` object arg: `{ x, z, amount }` (defaults already resolved by the
    /// parser: `amount ?? amp ?? 1`).
    Hit {
        x: f64,
        z: f64,
        amount: f64,
    },
    /// `dodge` object arg: `{ x, z, t }` (raw fields, `+v || 0` semantics).
    Dodge {
        x: f64,
        z: f64,
        t: f64,
    },
    /// `shoot` object arg: `{ hand }` (`arg.hand | 0`).
    Shoot {
        hand: i64,
    },
}

impl TrigArg {
    /// ≡ `arg || d` (falsy number/undefined → default).
    pub fn num_or(&self, d: f64) -> f64 {
        match self {
            TrigArg::Num(n) => {
                if *n != 0.0 {
                    *n
                } else {
                    d
                }
            }
            _ => d,
        }
    }

    /// ≡ `arg ?? d` (only undefined/null → default).
    pub fn num_nullish(&self, d: f64) -> f64 {
        match self {
            TrigArg::Num(n) => *n,
            _ => d,
        }
    }
}

/// ≡ `blinkCurve(b, t)` (character.js L398): lid closure at time `t` into a
/// blink envelope (accelerating close · hold · decelerating open).
pub fn blink_curve(b: &Blink, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t < b.c {
        let u = t / b.c;
        return u * u;
    }
    let t = t - b.c;
    if t < b.h {
        return 1.0;
    }
    let t = t - b.h;
    if t < b.o {
        return (1.0 - t / b.o).powf(2.2);
    }
    0.0
}

/// ≡ `this.att` (character.js L606-607): the attention target. `who` is
/// always `None` in the harness (`G.actors` unset → `_pickLook` never runs).
#[derive(Debug, Clone)]
pub struct Att {
    pub kind: i64,
    pub prev: i64,
    pub t: f64,
    pub p: DVec3,
    pub on: bool,
    pub who: Option<()>,
    pub gy: f64,
    pub gp: f64,
    pub dist: f64,
}

/// ≡ one `this.mx[i]` slot (character.js L608): a micro-expression with
/// `X = None` ≡ `null` (inactive).
#[derive(Debug, Clone, Copy)]
pub struct MxSlot {
    pub x: Option<[f64; 8]>,
    pub w: f64,
    pub t: f64,
    pub a: f64,
    pub h: f64,
    pub d: f64,
    pub bras: f64,
    /// ≡ `m.E`: [sneer, pucker, blush] secondary vector.
    pub e: Option<[f64; 3]>,
}

/// ≡ `this.gz` (character.js L616-617): the eye-gaze spring.
#[derive(Debug, Clone, Copy)]
pub struct Gaze {
    pub x: f64,
    pub y: f64,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub st: f64,
    pub sd: f64,
    pub fx: f64,
    pub fy: f64,
    pub mx: f64,
    pub my: f64,
    pub mt: f64,
}

/// Optional rig bones ≡ `this.xb` (`_buildRig` L708-713): bone indices,
/// `None` when the skeleton lacks the bone (the JS `if (xb.jaw)` guards).
#[derive(Debug, Clone, Copy, Default)]
#[allow(non_snake_case)]
pub struct Xb {
    pub jaw: Option<usize>,
    pub lidL: Option<usize>,
    pub lidR: Option<usize>,
    pub tank: Option<usize>,
    pub hem: Option<usize>,
    pub hemF: Option<usize>,
    pub hemB: Option<usize>,
    pub toeL: Option<usize>,
    pub toeR: Option<usize>,
    pub earL: Option<usize>,
    pub earR: Option<usize>,
    pub cheekL: Option<usize>,
    pub cheekR: Option<usize>,
}

/// One articulated hand ≡ `this.fing[sd]` (`_buildRig` L716-723): thumb plus
/// the four fingers (index/middle/ring/pinky), two bones each.
#[derive(Debug, Clone, Copy)]
pub struct Fing {
    pub t1: usize,
    pub t2: usize,
    pub f1: [usize; 4],
    pub f2: [usize; 4],
}

/// ≡ `this.face` (constructor L612-613): the face-rig channels written by
/// `_applyFace` (render-side output; the pinned harness never reads them).
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct FaceState {
    pub blinkL: f64,
    pub blinkR: f64,
    pub lidL: f64,
    pub lidR: f64,
    pub squintL: f64,
    pub squintR: f64,
    pub gazeX: f64,
    pub gazeY: f64,
    pub verge: f64,
    pub pupil: f64,
    pub browInL: f64,
    pub browInR: f64,
    pub browOutL: f64,
    pub browOutR: f64,
    pub furrow: f64,
    pub curve: f64,
    pub width: f64,
    pub open: f64,
    pub tilt: f64,
    pub jaw: f64,
    pub smile: f64,
    pub sneer: f64,
    pub pucker: f64,
    pub breath: f64,
}

impl Default for FaceState {
    /// ≡ constructor L612-613.
    fn default() -> Self {
        FaceState {
            blinkL: 0.0,
            blinkR: 0.0,
            lidL: 0.0,
            lidR: 0.0,
            squintL: 0.0,
            squintR: 0.0,
            gazeX: 0.0,
            gazeY: 0.0,
            verge: 0.0,
            pupil: 0.5,
            browInL: 0.0,
            browInR: 0.0,
            browOutL: 0.0,
            browOutR: 0.0,
            furrow: 0.0,
            curve: 0.75,
            width: 1.0,
            open: 0.0,
            tilt: 0.0,
            jaw: 0.0,
            smile: 0.0,
            sneer: 0.0,
            pucker: 0.0,
            breath: 0.0,
        }
    }
}

/// Render-side uniform / material outputs ≡ `this.u.*` values + `this.mats.*`
/// (face hooks written by `_applyFace` L3618-3631, materials by
/// `_updateMaterials` L4045-4059). `makeCharUniforms` (character-mats.js
/// L90-104) creates every uniform, so the JS `if (U.uGaze)` guards always
/// pass and the Rust port writes unconditionally.
#[derive(Debug, Clone, Copy)]
pub struct RenderOut {
    /// ≡ `u.uMouth.value` (Vector4).
    pub u_mouth: [f64; 4],
    /// ≡ `u.uLook.value` (Vector2).
    pub u_look: [f64; 2],
    /// ≡ `u.uGaze.value` (Vector4).
    pub u_gaze: [f64; 4],
    /// ≡ `u.uLid.value` (Vector4).
    pub u_lid: [f64; 4],
    /// ≡ `u.uPupil.value`.
    pub u_pupil: f64,
    /// ≡ `u.uMouth2.value` (Vector4).
    pub u_mouth2: [f64; 4],
    /// ≡ `u.uFlash.value` (Color).
    pub u_flash: [f64; 3],
    /// ≡ `u.uGlow.value` (Color).
    pub u_glow: [f64; 3],
    /// ≡ `mats.fill.emissive` (Color).
    pub fill_emissive: [f64; 3],
    /// ≡ `mats.glow.emissiveIntensity`.
    pub glow_emissive_intensity: f64,
}

impl Default for RenderOut {
    /// ≡ `makeCharUniforms` defaults (character-mats.js L90-104);
    /// `emissiveIntensity` starts at the material default (overwritten per
    /// frame by `_updateMaterials`).
    fn default() -> Self {
        RenderOut {
            u_mouth: [0.75, 1.0, 0.0, 0.0],
            u_look: [0.0, 0.0],
            u_gaze: [0.0; 4],
            u_lid: [0.0; 4],
            u_pupil: 0.5,
            u_mouth2: [0.0; 4],
            u_flash: [0.0; 3],
            u_glow: [0.0; 3],
            fill_emissive: [0.0; 3],
            glow_emissive_intensity: 1.0,
        }
    }
}

/// The per-frame character state input ≡ the `s` object (harness `mkState`).
#[derive(Debug, Clone)]
pub struct AnimState {
    pub grounded: Option<bool>,
    pub vy: Option<f64>,
    pub speed: Option<f64>,
    pub local_move: (f64, f64),
    pub turn_rate: Option<f64>,
    pub aim_pitch: f64,
    pub aim_yaw: f64,
    pub firing: bool,
    pub charge: Option<f64>,
    pub sub_aim: Option<bool>,
    pub rolling: bool,
    pub low_ink: bool,
    pub ink: Option<f64>,
    pub hp: Option<f64>,
    pub in_enemy_ink: bool,
    pub form: String,
    pub special: Option<f64>,
    /// ≡ `s.invuln` (read by `_updateMaterials` L4048; absent → false).
    pub invuln: bool,
    pub runner: Option<RunnerState>,
    /// `s.wallNormal` ≡ the climb-form wall normal (read by `_updateSquid`
    /// L3928; null in the pinned harness, which never enters climb).
    pub wall_normal: Option<DVec3>,
}

impl Default for AnimState {
    /// ≡ harness `mkState()` defaults (char-pose-reference.mjs L142-148).
    fn default() -> Self {
        AnimState {
            grounded: Some(true),
            vy: Some(0.0),
            speed: Some(0.0),
            local_move: (0.0, 0.0),
            turn_rate: Some(0.0),
            aim_pitch: 0.0,
            aim_yaw: 0.0,
            firing: false,
            charge: Some(0.0),
            sub_aim: Some(false),
            rolling: false,
            low_ink: false,
            ink: Some(0.9),
            hp: Some(1.0),
            in_enemy_ink: false,
            form: "kid".into(),
            special: None,
            invuln: false,
            runner: None,
            wall_normal: None,
        }
    }
}

/// `s.runner` ≡ the local player's weapon-runner read by `_updateStates` /
/// `_poseWeapon` (null in the pinned harness).
#[derive(Debug, Clone, Copy, Default)]
pub struct RunnerState {
    pub lock_t: f64,
    pub dodge: bool,
    pub charging: bool,
    pub streaming: bool,
    pub charge: f64,
    pub burst_frac: Option<f64>,
}

/// The full spring-pose runtime state for one character (≡ `new Character()`).
#[allow(non_snake_case)]
#[derive(Debug, Clone)]
pub struct CharAnim<'t> {
    pub tab: &'t Tables,
    pub name: String,
    pub seed: u32,
    pub style: [usize; 6],
    pub rng: Rng,

    // ---- banks (Float32Array ≡ f32 storage; every write rounds f64→f32) ----
    pub P: Vec<f32>,
    pub PD: Vec<f32>,
    pub PX: Vec<f32>,
    pub PY: Vec<f32>,
    pub sp: Vec<f32>,
    pub tr: Vec<f32>,
    pub st: Vec<f32>,
    pub stVar: Vec<f32>,

    // ---- clocks / idle life (≡ constructor L581-596) ----
    pub t: f64,
    pub shufT: f64,
    pub lastFidget: f64,
    pub fidget: i64,
    pub fidgetT: f64,
    pub nextFidget: f64,
    pub shiftT: f64,
    pub shiftTgt: f64,
    pub shiftS: f64,

    // ---- form (≡ L600-601, L636) ----
    pub form: String,
    pub formPrev: String,
    pub formT: f64,
    pub kidForm: bool,
    pub kidScale: f64,
    pub sqScale: f64,
    pub kidPop: f64,
    pub kidSY: f64,
    pub kidSXZ: f64,
    pub sqSY: f64,
    pub sqSXZ: f64,
    pub kidLift: f64,

    // ---- weapon (≡ L574-575, L640) ----
    pub weaponKind: String,
    pub animKind: String,
    pub hold: &'t HoldDef,
    pub dual: bool,
    pub weapon: Option<WeaponInstance>,
    pub weapons: HashMap<String, WeaponInstance>,
    pub lastShot: f64,
    pub lastRelease: f64,
    pub charge: f64,
    pub chargeFlash: f64,
    pub fullT: f64,
    pub fireHold: f64,
    pub _fireWant: f64,
    pub _subPrev: bool,
    pub bombHeld: bool,
    pub bombT: f64,
    pub bombSwap: f64,
    pub lockW: f64,
    pub spinW: f64,
    pub streamW: f64,
    pub spinRate: f64,
    pub spinAngle: f64,
    pub drumAngle: f64,
    pub weaponHidden: bool,
    pub subPropHidden: bool,
    pub swipeSide: f64,
    pub blT: f64,
    pub blSide: f64,
    pub blHeavy: bool,

    // ---- dance clocks (≡ L598-599; the pose layer itself is B2) ----
    pub dance: Option<String>,
    pub danceT: f64,
    pub prevDance: Option<String>,
    pub prevDanceT: f64,
    pub danceFade: f64,
    pub lastDance: Option<String>,
    pub danceVar: f64,
    pub danceOfs: f64,

    // ---- blend weights (≡ L593-594) ----
    pub wSub: f64,
    pub wAim: f64,
    pub wRoll: f64,
    pub wAir: f64,
    pub wDance: f64,
    pub wTwo: f64,
    pub wGlow: f64,
    pub wLow: f64,
    pub wTired: f64,
    pub wGoo: f64,
    pub exert: f64,
    pub brPh: f64,
    pub brN: f64,
    pub brAmp: f64,
    pub brRate: f64,
    pub sighT: f64,
    pub sigh: f64,
    pub idleT: f64,
    pub inkS: f64,
    pub hurt: f64,
    pub slosh: f64,

    // ---- B2 life state (≡ constructor L604-627) ----
    pub bl: Blink,
    pub att: Att,
    pub mx: [MxSlot; 6],
    pub mxT: f64,
    pub gripT: f64,
    pub earT: f64,
    pub lookT: f64,
    /// ≡ `this.lookActor` — always `None` in the harness (`G.actors` unset).
    pub lookActor: Option<()>,
    pub lookYaw: f64,
    pub lookPitch: f64,
    pub glanceYaw: f64,
    pub glancePitch: f64,
    pub gz: Gaze,
    pub _turnBl: f64,
    pub _mxLast: f64,
    pub blinkK: f64,
    pub blinkL: f64,
    pub blinkR: f64,
    pub extGl: f64,
    pub _headQW: DQuat,
    pub _headSet: bool,
    pub lifeLv: f64,
    pub _hairAcc: f64,
    pub _hairOdd: bool,
    pub _tapped: bool,
    pub _tugged: bool,
    /// ≡ `this.mitCh` (scalar damp weight, lazily created → 0).
    pub mitCh: f64,
    pub mitLp: f64,
    pub mitCl: f64,
    pub _mitO: [f64; 2],
    /// ≡ `this.blRaise` (lazily 0).
    pub blRaise: f64,
    /// ≡ `this.eyeX` / `this.eyeY` (lazily 0).
    pub eyeX: f64,
    pub eyeY: f64,
    /// Rest brow positions (kid space) ≡ `this.faceRest.browL/browR`.
    pub faceRestBrowL: DVec3,
    pub faceRestBrowR: DVec3,
    /// Rest cheek positions ≡ `this.cheekRest[0/1]`.
    pub cheekRest: [DVec3; 2],
    /// ≡ `this._tankRest` (lazily captured tank rest pos+euler).
    pub tankRest: Option<(DVec3, DVec3)>,
    /// ≡ `this.bomb.group.visible` (recomputed each `apply_pose`, L3309-3313).
    pub bomb_visible: bool,
    /// ≡ `this.color` (THREE.Color; `setColor` L1103-1104, hex → linear sRGB).
    pub color: [f64; 3],
    /// ≡ `this.xb` (`_buildRig` L710-713, filled by `build_rig`).
    pub xb: Xb,
    /// ≡ `this.fing` (`_buildRig` L716-723).
    pub fing: [Option<Fing>; 2],
    /// ≡ `this.face` (constructor L612-613).
    pub face: FaceState,
    /// ≡ `this.u.*` values + `this.mats.*` material props (render outputs).
    pub mat: RenderOut,
    /// ≡ `tank.fill.visible` (written by `_updateMaterials` L4055).
    pub tankFill_visible: bool,

    // ---- root motion (≡ L583-586) ----
    pub root_pos: DVec3,
    pub root_yaw: f64,
    /// ≡ the `worldQuat` handed to `_animWeapon` (L2143), kept for the pinned
    /// test's `wpWq` anchor (None on frames that are not slosher-held).
    pub last_off_wq: Option<DQuat>,
    /// ≡ harness `wpChain`: [kid, hips..handR, pivot, off] local quats at the
    /// `_animWeapon` call point (debug anchor for the worldQuat path).
    pub last_wq_chain: Vec<DQuat>,
    pub root_visible: bool,
    pub root_parent_visible: bool,
    pub inWorld: bool,
    pub phys: bool,
    pub rp: DVec3,
    pub rv: DVec3,
    pub ra: DVec3,
    pub rootInit: bool,
    pub yaw: f64,
    pub prevYaw: f64,
    pub yawRate: f64,
    pub kvx: f64,
    pub kvz: f64,
    pub kax: f64,
    pub kaz: f64,
    pub hs: f64,
    pub gv: f64,
    pub gs: f64,
    pub gvx: f64,
    pub gvz: f64,
    pub tread: bool,
    pub tvx: f64,
    pub tvz: f64,
    pub kgx: f64,
    pub kgz: f64,
    pub mdx: f64,
    pub mdz: f64,
    pub hipTwist: f64,
    pub vyS: f64,
    pub gnd: f64,
    pub airT: f64,
    pub grounded: bool,

    // ---- gait (≡ L588-591) ----
    pub phase: f64,
    pub moving: bool,
    pub gaitW: f64,
    pub runW: f64,
    pub duty: f64,
    pub cad: f64,
    pub liftH: f64,
    pub feet: [Foot; 2],
    pub feetValid: bool,
    pub replant: bool,
    pub settleCd: f64,
    pub plantW: f64,
    pub footTwist: f64,
    pub hipDrop: f64,
    pub stepOfsX: f64,
    pub stepOfsZ: f64,

    // ---- impulses / one-shots (≡ L597, L635-640) ----
    pub lReach: f64,
    pub ikErrPre: f64,
    pub leapEnd: f64,
    pub landAmp: f64,
    pub hitX: f64,
    pub hitZ: f64,
    pub hitAmp: f64,
    pub hitAcc: f64,
    pub slamGround: bool,
    pub jumpRun: f64,
    pub jumpLead: f64,
    pub tumble: f64,
    pub tumbleX: f64,
    pub tumbleZ: f64,
    pub tumbleDrop: f64,
    pub dodge: Option<Dodge>,
    pub dodgeX: f64,
    pub dodgeZ: f64,
    pub dodgeDur: f64,
    pub ikErr: [f64; 4],

    // ---- `_applyPose` scratch (≡ this._fL/_fR/_fLq/_fRq/_toeUp, L632) ----
    pub _fL: DVec3,
    pub _fR: DVec3,
    pub _fLq: DQuat,
    pub _fRq: DQuat,
    pub _toeUp: f64,

    // ---- scene-graph mirrors (LOCAL transforms; ≡ bones/groups) ----
    pub bones: Vec<GroupXform>,
    pub bone_order: Vec<usize>,
    pub kid: GroupXform,
    pub kid_visible: bool,
    pub model: GroupXform,
    pub squidRoot: GroupXform,
    pub squidRoot_visible: bool,
    pub limbs: HashMap<String, Limb>,
    pub legReach: f64,
    pub hairBones: Vec<Option<usize>>,
    pub hairIn: Vec<f32>,
    pub hairAx: Vec<f32>,
    pub hairA1: Vec<f32>,
    pub hairA2: Vec<f32>,
    pub hairPh: Vec<f32>,
    pub hairTips: Vec<Option<usize>>,
    pub handS: Vec<f32>,
    pub xw: Vec<f32>,
    pub hx: Vec<f32>,
    pub hv: Vec<f32>,
    pub tipX: Vec<f32>,
    pub tipV: Vec<f32>,
    pub headRY: f64,
    pub headRZ: f64,
    pub headPrevPos: DVec3,
    pub headPrevVel: DVec3,
    pub headAcc: DVec3,
    pub headVel: DVec3,
    pub headPrevQuat: DQuat,
    pub headInit: bool,
    pub visible: bool,

    // ---- tank / squid (≡ _buildTank L775-785 / _buildSquid L786-797) ----
    pub tank: TankState,
    pub tankFill: GroupXform,
    pub squidPivot: GroupXform,
    pub sqPos: DVec3,
    pub sqQuat: DQuat,
    pub sqInit: bool,
    pub sqBlink: f64,
    pub sqRoll: f64,
    pub sqYaw: f64,
    pub hopPhase: f64,
    pub hopAir: bool,
    /// `this.climbInset` ≡ L525 (`PLAYER.radius`); set by the game layer — the
    /// headless harness never enters the climb form (`s.wallNormal` is null).
    pub climbInset: f64,

    // ---- weapon scratch (≡ _wst/_wq L641-643) ----
    pub _wst: weapon::Wst,
    pub _wq: DQuat,
    pub armR: f64,
    pub armL: f64,
    pub aimP: f64,
    pub rcP: f64,
    pub rcZ: f64,
    pub rcP2: f64,
    pub rcZ2: f64,
    pub _effort: f64,
    pub _dt: f64,
}

/// `this.dodge` (twins roll) ≡ L1172.
#[derive(Debug, Clone, Copy)]
pub struct Dodge {
    pub t: f64,
    pub dir: (f64, f64),
    pub dur: f64,
}

/// Tank ink state ≡ `this.tank` (`_buildTank` L775-785).
#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct TankState {
    pub tilt: f64,
    pub h: f64,
    pub bottom: f64,
    pub offset: DVec3,
    pub fillPos: DVec3,
    pub fillScale: DVec3,
}

/// ≡ `poseNeutral(P)` (character.js L191-201): zero the bank, then write the
/// neutral stance defaults (feet at the rest plant, IK on, squash = 1 …).
pub fn pose_neutral(tab: &Tables, p: &mut [f32]) {
    // ≡ L192: `P.fill(0)`
    p.fill(0.0);
    let c = &tab.rig_c;
    let ch = &tab.ch;
    // ≡ L193-200 (Float32 writes round the f64 constants to f32)
    p[ch.FOOTL] = c.hipw as f32;
    p[ch.FOOTL + 1] = c.ankle_h as f32;
    p[ch.FOOTL + 2] = -0.004_f32;
    p[ch.FOOTLR + 1] = 0.1_f32;
    p[ch.FOOTR] = -c.hipw as f32;
    p[ch.FOOTR + 1] = c.ankle_h as f32;
    p[ch.FOOTR + 2] = -0.004_f32;
    p[ch.FOOTRR + 1] = -0.1_f32;
    p[ch.POLER] = -0.7_f32;
    p[ch.POLER + 1] = -0.55_f32;
    p[ch.POLER + 2] = -0.45_f32;
    p[ch.POLEL] = 0.7_f32;
    p[ch.POLEL + 1] = -0.55_f32;
    p[ch.POLEL + 2] = -0.45_f32;
    p[ch.IKR] = 1.0_f32;
    p[ch.WPL] = 1.0_f32;
    p[ch.WPR] = 1.0_f32;
    p[ch.STAB] = 0.82_f32;
    p[ch.AFOLT] = 1.0_f32;
    p[ch.AFOLR] = 1.0_f32;
    p[ch.MCURVE] = 0.75_f32;
    p[ch.MWIDTH] = 1.0_f32;
    p[ch.EYE] = 1.0_f32;
    p[ch.SQY] = 1.0_f32;
    p[ch.SQXZ] = 1.0_f32;
    p[ch.HANDPL] = 1.0_f32;
    p[ch.PUPIL] = 0.5_f32;
    p[ch.UARML + 2] = 0.1_f32;
    p[ch.UARMR + 2] = -0.1_f32;
    p[ch.FARML] = -0.3_f32;
    p[ch.FARMR] = -0.3_f32;
    p[ch.HANDL + 2] = -0.1_f32;
    p[ch.HANDR + 2] = 0.1_f32;
}

/// ≡ `new THREE.Color(cssHex)` with three r152+ colour management (Color.set
/// L1104): `#rrggbb` is sRGB, stored in the linear-sRGB working space —
/// `SRGBToLinear(c) = c < 0.04045 ? c*0.0773993808 : (c*0.9478672986 +
/// 0.0521327014)^2.4` on each `0…1` channel.
pub fn parse_color(css: &str) -> [f64; 3] {
    let h = css.strip_prefix('#').unwrap_or(css);
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    let ch = |shift: u32| {
        let c = ((v >> shift) & 0xff) as f64 / 255.0;
        if c < 0.04045 {
            c * 0.0773993808
        } else {
            (c * 0.9478672986 + 0.0521327014).powf(2.4)
        }
    };
    [ch(16), ch(8), ch(0)]
}

/// ≡ `_mkFoot(i)` (character.js L650-659).
fn mk_foot(i: usize) -> Foot {
    Foot {
        i,
        side: if i == 0 { 1.0 } else { -1.0 },
        pw: DVec3::ZERO,
        yaw: 0.0,
        n: DVec3::Y,
        planted: true,
        in_st: true,
        st_u: 0.5,
        st_t: 1.0,
        sw: false,
        mode: 2, // M_SETTLE (≡ L165 const map)
        su: 0.0,
        dur: 0.2,
        from: DVec3::ZERO,
        to: DVec3::ZERO,
        from_yaw: 0.0,
        to_yaw: 0.0,
        lift: 0.06,
        toe: 0.0,
        land: 0.0,
        tn: DVec3::Y,
        cw: DVec3::ZERO,
        cyaw: 0.0,
        pitch: 0.0,
        cn: DVec3::Y,
        disp: DVec3::ZERO,
        disp_yaw: 0.0,
        disp_ok: false,
    }
}

impl<'t> CharAnim<'t> {
    /// ≡ `new Character(opts)` (character.js L514-648). The RNG stream order
    /// is load-bearing: `_buildRig` (L569) consumes the strand phases FIRST,
    /// then the life-detail timers in source order — the fixture header
    /// `init` cross-checks every value.
    #[must_use]
    pub fn new(
        tab: &'t Tables,
        name: &str,
        color: &str,
        weapon_kind: &str,
        style: [usize; 6],
    ) -> CharAnim<'t> {
        // ≡ L515-522 + L645 (`setColor(opts.color ?? '#ff8a14')` → L1104).
        let color = parse_color(color);
        let seed = hash_str(name);
        let style = resolve_style(style, seed);
        let rng = Rng::new(seed);
        let pn = tab.ch.pn;
        let mut a = CharAnim {
            tab,
            name: name.to_owned(),
            seed,
            style,
            rng,
            P: vec![0.0; pn],
            PD: vec![0.0; pn],
            PX: vec![0.0; pn],
            PY: vec![0.0; pn],
            sp: vec![0.0; tab.spr.spn],
            // ≡ L580: `new Float32Array(TN).fill(99)`
            tr: vec![99.0; tab.tim.tn],
            st: tab.stance_idle.iter().map(|v| *v as f32).collect(),
            stVar: vec![0.0; 6],
            // ≡ L581-596
            t: 0.0,
            shufT: 0.0,
            lastFidget: -1.0,
            fidget: -1,
            fidgetT: 0.0,
            nextFidget: 0.0,
            shiftT: 0.0,
            shiftTgt: 1.0,
            shiftS: 0.0,
            // ≡ L600-601
            form: "kid".into(),
            formPrev: "kid".into(),
            formT: 99.0,
            kidForm: true,
            kidScale: 1.0,
            sqScale: 0.0,
            kidPop: 1.0,
            kidSY: 1.0,
            kidSXZ: 1.0,
            sqSY: 1.0,
            sqSXZ: 1.0,
            kidLift: 0.0,
            // ≡ L574-575 (setWeapon fills these in)
            weaponKind: "none".into(),
            animKind: "generic".into(),
            hold: &tab.holds["shooter"],
            dual: false,
            weapon: None,
            weapons: HashMap::new(),
            lastShot: 99.0,
            lastRelease: 99.0,
            charge: 0.0,
            chargeFlash: 0.0,
            fullT: 0.0,
            fireHold: 0.0,
            _fireWant: 0.0,
            _subPrev: false,
            bombHeld: false,
            bombT: 0.0,
            bombSwap: 0.0,
            lockW: 0.0,
            spinW: 0.0,
            streamW: 0.0,
            spinRate: 0.0,
            spinAngle: 0.0,
            drumAngle: 0.0,
            weaponHidden: false,
            subPropHidden: false,
            swipeSide: 1.0,
            blT: 99.0,
            blSide: 0.0,
            blHeavy: false,
            // ≡ L598-599
            dance: None,
            danceT: 0.0,
            prevDance: None,
            prevDanceT: 0.0,
            danceFade: 1.0,
            lastDance: None,
            danceVar: 0.0,
            danceOfs: 0.0,
            // ≡ L593-594, L610-611, L618
            wSub: 0.0,
            wAim: 0.0,
            wRoll: 0.0,
            wAir: 0.0,
            wDance: 0.0,
            wTwo: 0.0,
            wGlow: 0.0,
            wLow: 0.0,
            wTired: 0.0,
            wGoo: 0.0,
            exert: 0.0,
            brPh: 0.0,
            brN: 0.0,
            brAmp: 1.0,
            brRate: 1.0,
            sighT: 0.0,
            sigh: 0.0,
            idleT: 0.0,
            inkS: 1.0,
            hurt: 0.0,
            slosh: 0.0,
            // ≡ L604-627 (B2 life state; the rng draws land below)
            bl: Blink {
                t: -1.0,
                c: 0.08,
                h: 0.03,
                o: 0.16,
                amp: 1.0,
                lag: 0.008,
                next: 0.0,
                dbl: false,
                squeeze: 0.0,
            },
            att: Att {
                kind: 0,
                prev: 0,
                t: 0.0,
                p: DVec3::new(0.0, 1.2, 5.0),
                on: false,
                who: None,
                gy: 0.0,
                gp: 0.0,
                dist: 5.0,
            },
            mx: [MxSlot {
                x: None,
                w: 0.0,
                t: 99.0,
                a: 0.1,
                h: 0.2,
                d: 0.4,
                bras: 0.0,
                e: None,
            }; 6],
            mxT: 0.0,
            gripT: 0.0,
            earT: 0.0,
            lookT: 0.0,
            lookActor: None,
            lookYaw: 0.0,
            lookPitch: 0.0,
            glanceYaw: 0.0,
            glancePitch: 0.0,
            gz: Gaze {
                x: 0.0,
                y: 0.0,
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
                st: -1.0,
                sd: 0.05,
                fx: 0.0,
                fy: 0.0,
                mx: 0.0,
                my: 0.0,
                mt: 0.5,
            },
            _turnBl: 0.0,
            _mxLast: 0.0,
            blinkK: 0.0,
            blinkL: 0.0,
            blinkR: 0.0,
            extGl: 0.0,
            _headQW: DQuat::IDENTITY,
            _headSet: false,
            lifeLv: 2.0,
            _hairAcc: 0.0,
            _hairOdd: false,
            _tapped: false,
            _tugged: false,
            mitCh: 0.0,
            mitLp: 0.0,
            mitCl: 0.0,
            _mitO: [0.0; 2],
            blRaise: 0.0,
            eyeX: 0.0,
            eyeY: 0.0,
            faceRestBrowL: DVec3::ZERO,
            faceRestBrowR: DVec3::ZERO,
            cheekRest: [DVec3::ZERO; 2],
            tankRest: None,
            bomb_visible: false,
            color,
            xb: Xb::default(),
            fing: [None, None],
            face: FaceState::default(),
            mat: RenderOut::default(),
            tankFill_visible: true,
            // ≡ L583-586
            root_pos: DVec3::ZERO,
            root_yaw: 0.0,
            last_off_wq: None,
            last_wq_chain: Vec::new(),
            root_visible: true,
            root_parent_visible: true,
            inWorld: false,
            phys: false,
            rp: DVec3::ZERO,
            rv: DVec3::ZERO,
            ra: DVec3::ZERO,
            rootInit: false,
            yaw: 0.0,
            prevYaw: 0.0,
            yawRate: 0.0,
            kvx: 0.0,
            kvz: 0.0,
            kax: 0.0,
            kaz: 0.0,
            hs: 0.0,
            gv: 0.0,
            gs: 0.0,
            gvx: 0.0,
            gvz: 0.0,
            tread: false,
            tvx: 0.0,
            tvz: 0.0,
            kgx: 0.0,
            kgz: 0.0,
            mdx: 0.0,
            mdz: 1.0,
            hipTwist: 0.0,
            vyS: 0.0,
            gnd: 9.0,
            airT: 0.0,
            grounded: true,
            // ≡ L588-591
            phase: 0.0,
            moving: false,
            gaitW: 0.0,
            runW: 0.0,
            duty: 0.6,
            cad: 1.5,
            liftH: 0.06,
            feet: [mk_foot(0), mk_foot(1)],
            feetValid: false,
            replant: true,
            settleCd: 0.0,
            plantW: 1.0,
            footTwist: 0.0,
            hipDrop: 0.0,
            stepOfsX: 0.0,
            stepOfsZ: 0.0,
            // ≡ L597, L622, L635-640
            lReach: 0.0,
            ikErrPre: 0.0,
            leapEnd: -1.0,
            landAmp: 0.0,
            hitX: 0.0,
            hitZ: 1.0,
            hitAmp: 1.0,
            hitAcc: 0.0,
            slamGround: false,
            jumpRun: 0.0,
            jumpLead: 0.0,
            tumble: 0.0,
            tumbleX: 1.0,
            tumbleZ: 0.0,
            tumbleDrop: 0.0,
            dodge: None,
            dodgeX: 0.0,
            dodgeZ: 1.0,
            dodgeDur: 0.3,
            ikErr: [0.0; 4],
            // ≡ L632 (`_toeUp = 0`; the scratch vectors start at identity)
            _fL: DVec3::ZERO,
            _fR: DVec3::ZERO,
            _fLq: DQuat::IDENTITY,
            _fRq: DQuat::IDENTITY,
            _toeUp: 0.0,
            // ---- scene-graph mirrors ----
            bones: Vec::new(),
            bone_order: Vec::new(),
            kid: GroupXform::default(),
            kid_visible: true,
            model: GroupXform::default(),
            squidRoot: GroupXform::default(),
            squidRoot_visible: true,
            limbs: HashMap::new(),
            legReach: 0.0,
            hairBones: Vec::new(),
            hairIn: Vec::new(),
            hairAx: Vec::new(),
            hairA1: Vec::new(),
            hairA2: Vec::new(),
            hairPh: Vec::new(),
            hairTips: Vec::new(),
            handS: vec![1.0, 0.0],
            xw: vec![0.0; 12],
            // ≡ L624-626
            hx: vec![0.0; tab.rig.hair_max * tab.rig.hair_segs * 3],
            hv: vec![0.0; tab.rig.hair_max * tab.rig.hair_segs * 3],
            tipX: vec![0.0; tab.rig.hair_max * 3],
            tipV: vec![0.0; tab.rig.hair_max * 3],
            // ≡ L627-628
            headRY: 0.0,
            headRZ: 0.0,
            headPrevPos: DVec3::ZERO,
            headPrevVel: DVec3::ZERO,
            headAcc: DVec3::ZERO,
            headVel: DVec3::ZERO,
            headPrevQuat: DQuat::IDENTITY,
            headInit: false,
            visible: true,
            // ≡ L775-797 (filled by build_tank/build_squid below)
            tank: TankState {
                tilt: 0.0,
                h: 0.0,
                bottom: 0.0,
                offset: DVec3::ZERO,
                fillPos: DVec3::ZERO,
                fillScale: DVec3::ONE,
            },
            tankFill: GroupXform::default(),
            squidPivot: GroupXform::default(),
            sqPos: DVec3::ZERO,
            sqQuat: DQuat::IDENTITY,
            sqInit: false,
            sqBlink: 0.0,
            sqRoll: 0.0,
            sqYaw: 0.0,
            hopPhase: 0.0,
            hopAir: false,
            climbInset: 0.0,
            // ≡ L641-643
            _wst: weapon::Wst::default(),
            _wq: DQuat::IDENTITY,
            armR: 0.0,
            armL: 0.0,
            aimP: 0.0,
            rcP: 0.0,
            rcZ: 0.0,
            rcP2: 0.0,
            rcZ2: 0.0,
            _effort: 0.0,
            _dt: 0.0,
        };
        // ≡ L569-573: _buildRig → _buildTank → _buildBomb → _buildSquid
        // (hairPh consumes the FIRST draws of the stream).
        a.build_rig();
        a.build_tank();
        a.build_squid();
        // ≡ L590: `this.shufT = 4 + this.rng() * 6;`
        a.shufT = 4.0 + a.rng.next() * 6.0;
        // ≡ L594: `this.shiftT = 2 + this.rng() * 3;`
        a.shiftT = 2.0 + a.rng.next() * 3.0;
        // ≡ L595: `this.nextFidget = 3 + this.rng() * 3;`
        a.nextFidget = 3.0 + a.rng.next() * 3.0;
        // ≡ L605: `bl.next = 0.6 + this.rng() * 2.5;` (B2 blink clock)
        a.bl.next = 0.6 + a.rng.next() * 2.5;
        // ≡ L607: `att.t = 0.3 + this.rng() * 0.8;` (B2 attention clock)
        a.att.t = 0.3 + a.rng.next() * 0.8;
        // ≡ L609: `this.mxT = 2 + this.rng() * 4;` (B2 micro-expression clock)
        a.mxT = 2.0 + a.rng.next() * 4.0;
        // ≡ L610: `this.sighT = 4 + this.rng() * 5;`
        a.sighT = 4.0 + a.rng.next() * 5.0;
        // ≡ L611: `this.gripT = 4 + this.rng() * 5;`
        a.gripT = 4.0 + a.rng.next() * 5.0;
        // ≡ L615: `this.lookT = 0.5 + this.rng();`
        a.lookT = 0.5 + a.rng.next();
        // ≡ L627: `this.earT = 3 + this.rng() * 4;`
        a.earT = 3.0 + a.rng.next() * 4.0;
        // ≡ L599: `this.danceOfs = frac(seed * 0.61803) * 2.3;` (no RNG)
        a.danceOfs = frac(a.seed as f64 * 0.61803) * 2.3;
        // ≡ L645-647: setColor → setWeapon → poseNeutral(P).
        a.set_weapon(weapon_kind);
        pose_neutral(a.tab, &mut a.P);
        a
    }

    /// ≡ `_buildRig` (character.js L662-753) — the data part (bones, limb
    /// chains, hair axes/phases). Meshes/skeleton are render-side (B2).
    fn build_rig(&mut self) {
        let tab = self.tab;
        let ns = tab.rig.hair_meta.len();
        let segs = tab.rig.hair_segs;
        let hmax = tab.rig.hair_max;
        // ≡ L668-677: bone list + parent chain; LOCAL rest offset
        // `rest[n] - rest[p]` (identity rest rotation, ≡ L669-670).
        let n_bones = tab.rig.bone_names.len();
        self.bones = (0..n_bones)
            .map(|idx| {
                let rest = DVec3::from_array(tab.rig.rest[idx]);
                let pos = match tab.rig.bone_parent[idx] {
                    Some(p) => rest - DVec3::from_array(tab.rig.rest[p]),
                    None => rest,
                };
                GroupXform {
                    pos,
                    quat: DQuat::IDENTITY,
                    scale: DVec3::ONE,
                }
            })
            .collect();
        self.bone_order = (0..n_bones).collect();
        // ≡ L684-695: limb constants (`lim(up, lo, end, h0)`).
        let mut limbs = HashMap::new();
        for (name, (up, lo, end, h0)) in [
            (
                "armL",
                ("uArmL", "fArmL", "handL", DVec3::new(-1.0, 0.0, 0.0)),
            ),
            (
                "armR",
                ("uArmR", "fArmR", "handR", DVec3::new(-1.0, 0.0, 0.0)),
            ),
            ("legL", ("thighL", "shinL", "footL", DVec3::X)),
            ("legR", ("thighR", "shinR", "footR", DVec3::X)),
        ] {
            // ≡ L686: bone local positions (parent-relative, identity parents).
            let gi = |n: &str| tab.rig.bone_index[n];
            let local = |n: &str| self.bones[gi(n)].pos;
            let (a, b) = (local(lo).length(), local(end).length());
            let ru = local(lo).normalize();
            let rf = local(end).normalize();
            let mk0 = |r: DVec3| -> [DVec3; 3] {
                // ≡ L687: h = normalize(h0 - r*(h0·r)); c = r × h;
                // Matrix4.makeBasis(r, h, c).transpose() → rows (r, h, c).
                let h = (h0 - r * h0.dot(r)).normalize();
                let c = r.cross(h);
                [r, h, c]
            };
            limbs.insert(
                name.to_owned(),
                Limb {
                    up: gi(up),
                    lo: gi(lo),
                    end: gi(end),
                    a,
                    b,
                    ru,
                    rf,
                    mu0t: mk0(ru),
                    mf0t: mk0(rf),
                },
            );
        }
        self.limbs = limbs;
        // ≡ L696: `legReach = (legL.a + legL.b) * 0.985`
        self.legReach = (self.limbs["legL"].a + self.limbs["legL"].b) * 0.985;
        // ≡ L700: hair bone refs for ALL HAIR_MAX strands (missing → None).
        self.hairBones = (0..hmax)
            .flat_map(|s| (0..segs).map(move |k| (s, k)))
            .map(|(s, k)| {
                let name = format!("hair{s}_{k}");
                tab.rig.bone_index.get(&name).copied()
            })
            .collect();
        // ≡ L701-707: `hairIn[s]` = direction from strand root to the head
        // centre (kid space): hc = rest.head + HEAD_CTR.
        let hc = DVec3::from_array(tab.rig.rest[tab.rig.bone_index["head"]])
            + DVec3::from_array(tab.rig_c.head_ctr);
        self.hairIn = vec![0.0; hmax * 3];
        for s in 0..ns {
            let r0 = DVec3::from_array(tab.rig.rest[tab.rig.bone_index[&format!("hair{s}_0")]]);
            let v = (hc - r0).normalize();
            self.hairIn[s * 3] = v.x as f32;
            self.hairIn[s * 3 + 1] = v.y as f32;
            self.hairIn[s * 3 + 2] = v.z as f32;
        }
        // ≡ L724: hand shapes (L, R).
        self.handS = vec![1.0, 0.0];
        // ≡ L727: tentacle tips (one more stage after hair{s}_2).
        self.hairTips = (0..hmax)
            .map(|s| {
                let name = format!("hairTip{s}");
                tab.rig.bone_index.get(&name).copied()
            })
            .collect();
        // ≡ L732-750: per-segment axes (NS = HAIR_MAX*(SEGS+1)) + phases.
        let nn = hmax * (segs + 1);
        self.hairAx = vec![0.0; nn * 3];
        self.hairA1 = vec![0.0; nn * 3];
        self.hairA2 = vec![0.0; nn * 3];
        self.hairPh = vec![0.0; hmax];
        for s in 0..ns {
            // ≡ L736: the constructor's FIRST RNG draws (strand phases).
            self.hairPh[s] = (self.rng.next() * TAU) as f32;
            for k in 0..=segs {
                // ≡ L738-745: axis = next - a (tip stage reuses the previous
                // axis); A1 = coil plane (cross with out-of-head), A2 = sideways.
                // ≡ L738: `a` is bone k for k < SEGS, the tip bone at k == SEGS.
                let a = if k < segs {
                    DVec3::from_array(tab.rig.rest[tab.rig.bone_index[&format!("hair{s}_{k}")]])
                } else {
                    DVec3::from_array(tab.rig.rest[tab.rig.bone_index[&format!("hairTip{s}")]])
                };
                let j = (s * (segs + 1) + k) * 3;
                let v1 = if k < segs - 1 {
                    let b = DVec3::from_array(
                        tab.rig.rest[tab.rig.bone_index[&format!("hair{s}_{}", k + 1)]],
                    );
                    b - a
                } else if k == segs - 1 {
                    let tip =
                        DVec3::from_array(tab.rig.rest[tab.rig.bone_index[&format!("hairTip{s}")]]);
                    tip - a
                } else {
                    // k == segs: the tip stage has no "next" → previous axis.
                    DVec3::new(
                        self.hairAx[j - 3] as f64,
                        self.hairAx[j - 2] as f64,
                        self.hairAx[j - 1] as f64,
                    )
                };
                // ≡ L741: degenerate axis → (0, -1, 0).
                let v1 = if v1.length_squared() < 1e-10 {
                    DVec3::new(0.0, -1.0, 0.0)
                } else {
                    v1
                };
                let v1 = v1.normalize();
                let v2 = (a - hc).normalize();
                let mut v3 = v1.cross(v2);
                if v3.length_squared() < 1e-8 {
                    v3 = DVec3::X;
                }
                let v3 = v3.normalize();
                let v4 = v1.cross(v3).normalize();
                for c in 0..3 {
                    self.hairAx[j + c] = v1.to_array()[c] as f32;
                    self.hairA1[j + c] = v3.to_array()[c] as f32;
                    self.hairA2[j + c] = v4.to_array()[c] as f32;
                }
            }
        }
        // ≡ L753: `faceRest.browL/browR` and `cheekRest` capture the bone
        // positions right after rig construction (rest offsets).
        let gi = |n: &str| tab.rig.bone_index[n];
        self.faceRestBrowL = self.bones[gi("browL")].pos;
        self.faceRestBrowR = self.bones[gi("browR")].pos;
        self.cheekRest = [self.bones[gi("cheekL")].pos, self.bones[gi("cheekR")].pos];
        // ≡ L708-713: optional bones (None when the skeleton lacks them).
        let opt = |n: &str| tab.rig.bone_index.get(n).copied();
        self.xb = Xb {
            jaw: opt("jaw"),
            lidL: opt("lidL"),
            lidR: opt("lidR"),
            tank: opt("tank"),
            hem: opt("hem"),
            hemF: opt("hemF"),
            hemB: opt("hemB"),
            toeL: opt("toeL"),
            toeR: opt("toeR"),
            earL: opt("earL"),
            earR: opt("earR"),
            cheekL: opt("cheekL"),
            cheekR: opt("cheekR"),
        };
        // ≡ L716-723: articulated hands — thumb + 4 fingers (index/middle/
        // ring/pinky), two bones each; Some only when every bone exists.
        self.fing = [None, None];
        for (sd, slot) in self.fing.iter_mut().enumerate() {
            let sn = if sd == 0 { "L" } else { "R" };
            let t1 = opt(&format!("hand{sn}_thumb1"));
            let t2 = opt(&format!("hand{sn}_thumb2"));
            let mut f1 = [0usize; 4];
            let mut f2 = [0usize; 4];
            let mut n_f = 0;
            for (k, fn_) in ["index", "middle", "ring", "pinky"].iter().enumerate() {
                if let (Some(b1), Some(b2)) = (
                    opt(&format!("hand{sn}_{fn_}1")),
                    opt(&format!("hand{sn}_{fn_}2")),
                ) {
                    f1[k] = b1;
                    f2[k] = b2;
                    n_f += 1;
                }
            }
            *slot = match (t1, t2, n_f) {
                (Some(t1), Some(t2), 4) => Some(Fing { t1, t2, f1, f2 }),
                _ => None,
            };
        }
    }

    /// ≡ `_buildTank` (character.js L775-785).
    fn build_tank(&mut self) {
        // ≡ L776-784: the tank group hangs off bone `tank` (local identity);
        // the fill node sits at `bottom` inside the tilted shell.
        self.tank.tilt = -0.1;
        self.tank.h = 0.18;
        self.tank.bottom = -0.09;
        let tab = self.tab;
        self.tank.offset = DVec3::ZERO;
        self.tank.fillPos = DVec3::new(0.0, self.tank.bottom, 0.0);
        self.tank.fillScale = DVec3::ONE;
        // ≡ L781: `fill.position.y = T.fillBottom` — the LOCAL node transform
        // the harness records as `tankFill` (rotation/scale written per frame
        // by `squid::update_tank`).
        self.tankFill = GroupXform {
            pos: self.tank.fillPos,
            quat: DQuat::IDENTITY,
            scale: DVec3::ONE,
        };
        let _ = tab;
    }

    /// ≡ `_buildSquid` (character.js L786-797).
    fn build_squid(&mut self) {
        // ≡ L787-796: squidRoot holds the pivot; both start at identity.
        self.squidPivot = GroupXform::default();
    }

    /// ≡ `setWeapon` (character.js L1122-1138).
    pub fn set_weapon(&mut self, kind: &str) {
        // ≡ L1123-1124: `if (!HOLD[ANIM_OF[kind] || kind]) kind = 'shooter';`
        fn anim_of(tab: &Tables, k: &str) -> String {
            tab.anim_of.get(k).cloned().unwrap_or_else(|| k.to_owned())
        }
        let mut kind = kind.to_owned();
        if !self.tab.holds.contains_key(&anim_of(self.tab, &kind)) {
            kind = "shooter".to_owned();
        }
        if kind == self.weaponKind {
            return;
        }
        // ≡ L1131-1137: instance cache + attach; `dual = !!w.left`.
        let w = self
            .weapons
            .get(&kind)
            .cloned()
            .unwrap_or_else(|| self.build_weapon_instance(&kind));
        self.weapons.insert(kind.clone(), w.clone());
        self.weaponKind = kind.clone();
        self.animKind = anim_of(self.tab, &kind);
        self.dual = w.left.is_some();
        self.weapon = Some(w);
        self.hold = self
            .tab
            .holds
            .get(&self.weaponKind)
            .or_else(|| self.tab.holds.get(&self.animKind))
            .expect("hold table");
    }

    /// ≡ `w.off.getWorldQuaternion(...)` (`_animWeapon` L2143): the off node's
    /// world rotation composed from the CURRENT local transforms
    /// `root·model·kid·[rig chain …→handR]·pivot·off` — full matrix product
    /// + THREE decompose (non-uniform scales in the chain shear the rotation;
    ///   a plain quaternion product is NOT exact).
    pub fn off_world_quat(&mut self) -> DQuat {
        let mut o = self.tab.rig.bone_index["handR"];
        let mut chain = Vec::new();
        loop {
            chain.push(o);
            match self.tab.rig.bone_parent[o] {
                Some(p) => o = p,
                None => break,
            }
        }
        chain.reverse();
        let mut m = DMat4::from_scale_rotation_translation(
            DVec3::ONE,
            DQuat::from_rotation_y(self.root_yaw),
            self.root_pos,
        );
        m *= DMat4::from_scale_rotation_translation(
            self.model.scale,
            self.model.quat,
            self.model.pos,
        );
        m *= DMat4::from_scale_rotation_translation(self.kid.scale, self.kid.quat, self.kid.pos);
        for &bi in &chain {
            let g = &self.bones[bi];
            m *= DMat4::from_scale_rotation_translation(g.scale, g.quat, g.pos);
        }
        let mut snap: Vec<DQuat> = vec![self.kid.quat];
        for &bi in &chain {
            snap.push(self.bones[bi].quat);
        }
        if let Some(w) = &self.weapon {
            m *= DMat4::from_scale_rotation_translation(w.pivot.scale, w.pivot.quat, w.pivot.pos);
            m *= DMat4::from_scale_rotation_translation(w.off.scale, w.off.quat, w.off.pos);
            snap.push(w.pivot.quat);
            snap.push(w.off.quat);
        }
        self.last_wq_chain = snap;
        decompose_three(&m).1
    }

    /// ≡ `_weaponInstance` (character.js L838-870, shooter family): the pivot
    /// hangs off `handR` at FIST_OFFSET, `off` under it, muzzle + parts under
    /// `off` (all local to the weapon group, ≡ the fixture `weapon` header).
    fn build_weapon_instance(&mut self, kind: &str) -> WeaponInstance {
        let fr = self.tab.weapons.get(kind).cloned();
        let Some(fr) = fr else {
            return WeaponInstance {
                kind: kind.to_owned(),
                pivot: GroupXform::default(),
                off: GroupXform::default(),
                muzzle_pos: DVec3::ZERO,
                parts: Vec::new(),
                left: None,
                dual: false,
                ps: {
                    let mut p = [0.0f32; 12];
                    p[0] = 1.3; // ≡ L1350: w.ps[0] = 1.3
                    p
                },
                pump: 0.0,
                trig: 0.0,
                heat: 0.0,
                drum_w: 0.0,
                drum_a: 0.0,
                drum_r: 0.1,
                spin_w: 0.0,
                spin_a: 0.0,
                has_spin: false,
                spin_mesh_angle: 0.0,
                near: true,
                far_dec: 0.0,
                hidden: false,
                ms: [0.0; 8],
                ms_init: false,
                last_punch: 99.0,
            };
        };
        let fist = DVec3::from_array(self.tab.rig.fist_offset);
        let parts = fr
            .parts
            .iter()
            .map(|(n, p)| PartXform {
                name: n.clone(),
                base_pos: DVec3::from_array(*p),
                pos: DVec3::from_array(*p),
                quat: DQuat::IDENTITY,
                scale: DVec3::ONE,
            })
            .collect::<Vec<_>>();
        // ≡ `_getWeapon` L803 + `_weaponInstance(d, true)` L823-825: the dual
        // left-hand twin hangs off handL at GRIP_HOLE_L with inHandL frames.
        let left = if fr.has_left {
            fr.in_hand_l.map(|(ip, iq)| {
                let hole = DVec3::from_array(self.tab.rig.grip_hole_l);
                Box::new(WeaponInstance {
                    kind: kind.to_owned(),
                    pivot: GroupXform {
                        pos: hole,
                        quat: DQuat::IDENTITY,
                        scale: DVec3::ONE,
                    },
                    off: GroupXform {
                        pos: DVec3::from_array(ip) - hole,
                        quat: DQuat::from_array(iq),
                        scale: DVec3::ONE,
                    },
                    muzzle_pos: DVec3::from_array(fr.muzzle),
                    parts: parts.clone(),
                    left: None,
                    dual: true,
                    ps: {
                        let mut p = [0.0f32; 12];
                        p[0] = 1.3;
                        p
                    },
                    pump: 0.0,
                    trig: 0.0,
                    heat: 0.0,
                    drum_w: 0.0,
                    drum_a: 0.0,
                    drum_r: 0.1,
                    spin_w: 0.0,
                    spin_a: 0.0,
                    has_spin: false,
                    spin_mesh_angle: 0.0,
                    near: true,
                    far_dec: 0.0,
                    hidden: false,
                    ms: [0.0; 8],
                    ms_init: false,
                    last_punch: 99.0,
                })
            })
        } else {
            None
        };
        WeaponInstance {
            kind: kind.to_owned(),
            // ≡ L845-848: pivot.position = FIST_OFFSET.
            pivot: GroupXform {
                pos: fist,
                quat: DQuat::IDENTITY,
                scale: DVec3::ONE,
            },
            // ≡ L849-852: off.position = inHand.pos - FIST_OFFSET,
            // off.quaternion = inHand.quat (both from char.json frames).
            off: GroupXform {
                pos: DVec3::from_array(fr.off_pos),
                quat: DQuat::from_array(fr.off_quat),
                scale: DVec3::ONE,
            },
            muzzle_pos: DVec3::from_array(fr.muzzle),
            parts,
            left,
            dual: fr.has_left,
            ps: {
                let mut p = [0.0f32; 12];
                p[0] = 1.3; // ≡ L1350: w.ps[0] = 1.3
                p
            },
            pump: 0.0,
            trig: 0.0,
            heat: 0.0,
            drum_w: 0.0,
            drum_a: 0.0,
            drum_r: 0.1,
            spin_w: 0.0,
            spin_a: 0.0,
            has_spin: fr.has_spin,
            spin_mesh_angle: 0.0,
            near: true,
            far_dec: 0.0,
            hidden: false,
            ms: [0.0; 8],
            ms_init: false,
            last_punch: 99.0,
        }
    }

    /// ≡ `_recoil(k, hand)` (character.js L1299-1311). Spring VELOCITY kicks
    /// (`sp[i+1] += …`); consumes 3 RNG draws for `hand == 0` (RCY, RCR,
    /// TANKX), 2 for `hand == 1` (RCY2, TANKX).
    pub fn recoil(&mut self, k: f64, hand: usize) {
        // ≡ L1300: rc = this.hold.rc, w = TAU * rc.hz
        let rc = self.hold.rc;
        let w = TAU * rc.hz;
        let s = self.tab.spr;
        if hand == 1 {
            // ≡ L1302-1303
            self.sp[s.s_rc_p2 + 1] =
                (self.sp[s.s_rc_p2 + 1] as f64 + rc.kick * w * 1.35 * k) as f32;
            self.sp[s.s_rc_z2 + 1] =
                (self.sp[s.s_rc_z2 + 1] as f64 + rc.back * w * 1.35 * k) as f32;
            self.sp[s.s_rc_y2 + 1] = (self.sp[s.s_rc_y2 + 1] as f64
                + (self.rng.next() - 0.5) * 2.0 * rc.jit * w * k)
                as f32;
        } else {
            // ≡ L1305-1308
            self.sp[s.s_rc_p + 1] = (self.sp[s.s_rc_p + 1] as f64 + rc.kick * w * 1.35 * k) as f32;
            self.sp[s.s_rc_z + 1] = (self.sp[s.s_rc_z + 1] as f64 + rc.back * w * 1.35 * k) as f32;
            self.sp[s.s_rc_y + 1] = (self.sp[s.s_rc_y + 1] as f64
                + (self.rng.next() - 0.5) * 2.0 * rc.jit * w * k)
                as f32;
            self.sp[s.s_rc_r + 1] = (self.sp[s.s_rc_r + 1] as f64
                + (self.rng.next() - 0.5) * 2.0 * rc.jit * w * k)
                as f32;
        }
        // ≡ L1310
        self.sp[s.s_tank_x + 1] =
            (self.sp[s.s_tank_x + 1] as f64 + (self.rng.next() - 0.5) * 0.4 * k) as f32;
        self.sp[s.s_tank_l + 1] = (self.sp[s.s_tank_l + 1] as f64 - 0.4 * k) as f32;
    }

    /// ≡ `_hairKick(x, y, z)` (character.js L1312): impulse into the hair
    /// velocity springs (all HAIR_MAX*SEGS segments).
    pub fn hair_kick(&mut self, x: f64, y: f64, z: f64) {
        // ≡ L1312: `for (i += 3) { hv[i] += z*0.6 + x*0.3; hv[i+1] += x*0.4;
        // hv[i+2] += y*0.5 - x*0.2; }`
        let hv = &mut self.hv;
        for i in (0..hv.len()).step_by(3) {
            hv[i] = (hv[i] as f64 + z * 0.6 + x * 0.3) as f32;
            hv[i + 1] = (hv[i + 1] as f64 + x * 0.4) as f32;
            hv[i + 2] = (hv[i + 2] as f64 + y * 0.5 - x * 0.2) as f32;
        }
    }

    /// ≡ `trigger(name, arg)` (character.js L1140-1223). Only the branches the
    /// harness exercises (shoot/jump/land) run B1 code; the rest keep their
    /// timer writes so the bank state matches. `_mxPlay`/`_blink` are B2.
    pub fn trigger(&mut self, name: &str, arg: TrigArg) {
        // ≡ L1141
        let s = self.tab.spr;
        let t = self.tab.tim;
        match name {
            // ≡ L1143-1150
            "shoot" => {
                if self.animKind == "roller" {
                    self.tr[t.t_flick] = 0.0;
                    return;
                }
                if self.animKind == "slosher" {
                    if self.tr[t.t_slosh] > 0.3 {
                        self.tr[t.t_slosh] = 0.0;
                    }
                    self.lastShot = 0.0;
                    return;
                }
                // ≡ L1146: `arg && typeof arg === 'object' ? (arg.hand | 0) : 0`
                let hand = match arg {
                    TrigArg::Shoot { hand } => hand,
                    _ => 0,
                };
                if self.dual && hand == 1 {
                    self.tr[t.t_shoot_l] = 0.0;
                    self.recoil(1.0, 1);
                    self.sp[s.s_grip_l + 1] = (self.sp[s.s_grip_l + 1] as f64 + 2.2) as f32;
                } else {
                    self.tr[t.t_shoot] = 0.0;
                    self.recoil(1.0, 0);
                    self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 2.2) as f32;
                    if !self.dual {
                        self.sp[s.s_grip_l + 1] = (self.sp[s.s_grip_l + 1] as f64 + 1.2) as f32;
                    }
                }
                self.lastShot = 0.0;
            }
            // ≡ L1151
            "slosh" => {
                self.tr[t.t_slosh] = 0.0;
                self.lastShot = 0.0;
            }
            // ≡ L1153-1155
            "admire" => self.tr[t.t_admire] = 0.0,
            "hairflip" => self.tr[t.t_flip] = 0.0,
            "wink" => self.tr[t.t_wink] = 0.0,
            // ≡ L1156-1164
            "dodge" => {
                // ≡ L1161-1165
                let (mut x, mut z, d) = match arg {
                    TrigArg::Dodge { x, z, t } => {
                        (x, z, clamp(if t != 0.0 { t } else { 0.3 }, 0.15, 0.8))
                    }
                    _ => (0.0f64, 1.0f64, 0.3f64),
                };
                let l = (x * x + z * z).sqrt();
                if l > 1e-4 {
                    x /= l;
                    z /= l;
                } else {
                    x = 0.0;
                    z = 1.0;
                }
                self.dodgeX = x;
                self.dodgeZ = z;
                self.dodgeDur = d;
                self.tr[t.t_dodge] = 0.0;
                self.lockW = self.lockW.max(0.001);
                self.sp[s.s_sq + 1] = (self.sp[s.s_sq + 1] as f64 - 1.6) as f32;
                self.sp[s.s_tank_l + 1] = (self.sp[s.s_tank_l + 1] as f64 + 1.5) as f32;
                self.hair_kick(-x * 2.0, 1.2, -z * 2.0);
            }
            // ≡ L1166
            "flick" => {
                self.tr[t.t_flick] = 0.0;
                self.lastShot = 0.0;
                self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 3.0) as f32;
                self.sp[s.s_grip_l + 1] = (self.sp[s.s_grip_l + 1] as f64 + 3.0) as f32;
                // ≡ L1165: `if (arg) this.swipeSide = arg;`
                if let TrigArg::Num(a) = arg
                    && a != 0.0
                {
                    self.swipeSide = a;
                }
            }
            // ≡ L1168-1170
            "blade_slash" => {
                self.blT = 0.0;
                self.blSide = arg.num_or(1.0);
                self.blHeavy = false;
                self.lastShot = 0.0;
                self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 2.4) as f32;
            }
            "blade_heavy" => {
                self.blT = 0.0;
                self.blHeavy = true;
                self.lastShot = 0.0;
                self.sp[s.s_sq + 1] = (self.sp[s.s_sq + 1] as f64 - 1.2) as f32;
                self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 3.2) as f32;
            }
            "stream" => self.lastShot = 0.0,
            // ≡ L1173
            "throw" => {
                self.tr[t.t_throw] = 0.0;
                self.bombHeld = false;
            }
            // ≡ L1174-1183
            "land" => {
                let a = clamp((arg.num_nullish(8.0) - 2.5) / 13.0, 0.12, 1.0);
                self.landAmp = if self.tr[t.t_land] < 0.25 {
                    self.landAmp.max(a)
                } else {
                    a
                };
                self.tr[t.t_land] = 0.0;
                self.sp[s.s_pely + 1] = (self.sp[s.s_pely + 1] as f64 - 3.0 * a) as f32;
                self.sp[s.s_sq + 1] = (self.sp[s.s_sq + 1] as f64 - 3.6 * a) as f32;
                self.sp[s.s_lean_p + 1] = (self.sp[s.s_lean_p + 1] as f64 + 2.2 * a) as f32;
                self.sp[s.s_head_p + 1] = (self.sp[s.s_head_p + 1] as f64 + 3.5 * a) as f32;
                self.sp[s.s_arm_l + 1] = (self.sp[s.s_arm_l + 1] as f64 - 4.0 * a) as f32;
                self.sp[s.s_arm_r + 1] = (self.sp[s.s_arm_r + 1] as f64 - 4.0 * a) as f32;
                self.sp[s.s_wp_y + 1] = (self.sp[s.s_wp_y + 1] as f64 - 0.9 * a) as f32;
                self.sp[s.s_wr_x + 1] = (self.sp[s.s_wr_x + 1] as f64 + 4.0 * a) as f32;
                self.sp[s.s_tank_l + 1] = (self.sp[s.s_tank_l + 1] as f64 - 3.0 * a) as f32;
                self.hair_kick(0.0, -3.2 * a, 0.0);
                self.sp[s.s_ear_l + 1] = (self.sp[s.s_ear_l + 1] as f64 - 5.0 * a) as f32;
                self.sp[s.s_ear_r + 1] = (self.sp[s.s_ear_r + 1] as f64 - 5.0 * a) as f32;
                // ≡ L1181: `if (a > 0.55) this._blink();`
                if a > 0.55 {
                    self.blink(false);
                }
            }
            // ≡ L1184-1192
            "jump" => {
                self.tr[t.t_jump] = 0.0;
                self.sp[s.s_sq + 1] = (self.sp[s.s_sq + 1] as f64 + 2.6) as f32;
                self.sp[s.s_tank_l + 1] = (self.sp[s.s_tank_l + 1] as f64 + 2.0) as f32;
                self.hair_kick(0.0, 2.4, 0.0);
                // ≡ L1186: `if (kidForm && !dance && rng() < 0.6) _mxPlay(X_HUP, ...)`
                if self.kidForm && self.dance.is_none() && self.rng.next() < 0.6 {
                    let x = self.tab.expr8("X_HUP");
                    self.mx_play(&x, 0.6, 0.05, 0.12, 0.3, 0.0, None);
                }
                // ≡ L1188-1190: running-jump lead foot.
                self.jumpRun = if self.kidForm {
                    sstep(1.5, 4.5, self.gs)
                } else {
                    0.0
                };
                let (sw0, sw1, su0, su1) = (
                    self.feet[0].sw,
                    self.feet[1].sw,
                    self.feet[0].su,
                    self.feet[1].su,
                );
                self.jumpLead = if sw0 && !sw1 {
                    0.0
                } else if sw1 && !sw0 {
                    1.0
                } else if su0 > su1 {
                    0.0
                } else {
                    1.0
                };
            }
            // ≡ L1193-1213
            "hit" => {
                self.tr[t.t_hit] = 0.0;
                // ≡ L1194-1197: object arg normalizes {x,z}; otherwise the
                // direction is a draw and amp defaults to 1.
                let (hx, hz, amp) = match arg {
                    TrigArg::Hit { x, z, amount } => {
                        let (mut hx, mut hz) = (x, z);
                        let l = (hx * hx + hz * hz).sqrt();
                        if l > 1e-4 {
                            hx /= l;
                            hz /= l;
                        } else {
                            hx = 0.0;
                            hz = 1.0;
                        }
                        (hx, hz, clamp(amount, 0.3, 1.6))
                    }
                    TrigArg::Num(a) => {
                        let hx = (self.rng.next() - 0.5) * 1.2;
                        (hx, 1.0f64, clamp(a, 0.3, 1.4))
                    }
                    _ => {
                        let hx = (self.rng.next() - 0.5) * 1.2;
                        (hx, 1.0f64, 1.0f64)
                    }
                };
                self.hitX = hx;
                self.hitZ = hz;
                self.hitAmp = amp;
                self.hitAcc += amp;
                self.sp[s.s_hit_p + 1] = (self.sp[s.s_hit_p + 1] as f64 - hz * 6.5 * amp) as f32;
                self.sp[s.s_hit_r + 1] = (self.sp[s.s_hit_r + 1] as f64 + hx * 6.5 * amp) as f32;
                self.sp[s.s_hit_y + 1] = (self.sp[s.s_hit_y + 1] as f64 + hx * 5.0 * amp) as f32;
                self.sp[s.s_pely + 1] = (self.sp[s.s_pely + 1] as f64 - 0.5 * amp) as f32;
                self.sp[s.s_head_p + 1] = (self.sp[s.s_head_p + 1] as f64 - hz * 7.5 * amp) as f32;
                self.sp[s.s_head_r + 1] = (self.sp[s.s_head_r + 1] as f64 + hx * 7.0 * amp) as f32;
                self.sp[s.s_clav + 1] = (self.sp[s.s_clav + 1] as f64 + 3.5 * amp) as f32;
                self.sp[s.s_wr_x + 1] = (self.sp[s.s_wr_x + 1] as f64 - 2.0 * amp) as f32;
                self.hair_kick(hx * 1.5, 1.2, -hz * 1.8);
                self.sp[s.s_ear_l + 1] =
                    (self.sp[s.s_ear_l + 1] as f64 + (4.0 + 3.0 * hx) * amp) as f32;
                self.sp[s.s_ear_r + 1] =
                    (self.sp[s.s_ear_r + 1] as f64 + (4.0 - 3.0 * hx) * amp) as f32;
                // ≡ L1207-1208
                self.blink(amp > 0.7);
                if amp > 0.8 && self.kidForm {
                    let x = self.tab.expr8("X_OOF");
                    self.mx_play(&x, 0.35 + 0.3 * amp, 0.04, 0.1, 0.45, 0.0, None);
                }
                self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 3.0 * amp) as f32;
                if self.hitAcc > 2.6 && self.tr[t.t_stag] > 0.9 {
                    self.tr[t.t_stag] = 0.0;
                    self.hitAcc = 0.0;
                    self.stepOfsZ = -0.16 * hz;
                    self.stepOfsX = -0.1 * hx;
                    self.sp[s.s_stag + 1] = (self.sp[s.s_stag + 1] as f64 - 2.5) as f32;
                }
                if self.form != "kid" {
                    self.sp[s.s_sq_p + 1] = (self.sp[s.s_sq_p + 1] as f64 + 5.0 * amp) as f32;
                    self.sp[s.s_sq_y + 1] = (self.sp[s.s_sq_y + 1] as f64 - 3.0 * amp) as f32;
                }
            }
            // ≡ L1214-1215
            "special_leap" => {
                self.tr[t.t_leap] = 0.0;
                self.tr[t.t_slam] = 99.0;
                self.tr[t.t_impact] = 99.0;
                self.slamGround = false;
                self.leapEnd = -1.0;
                self.hair_kick(0.0, -2.0, 0.0);
            }
            "special_slam" => {
                self.tr[t.t_slam] = 0.0;
                self.leapEnd = if (self.tr[t.t_leap] as f64) < 1.9 {
                    self.tr[t.t_leap] as f64
                } else {
                    -1.0
                };
                self.tr[t.t_impact] = 99.0;
                self.slamGround = false;
            }
            // ≡ L1216-1219
            "spawn" => {
                self.tr[t.t_spawn] = 0.0;
                self.form = "kid".into();
                self.formPrev = "kid".into();
                self.formT = 99.0;
                self.kidScale = 1.0;
                self.sqScale = 0.0;
                self.feetValid = false;
                self.replant = true;
                self.headInit = false;
                self.rootInit = false;
            }
            // ≡ L1220
            "charge_release" => {
                self.tr[t.t_rel] = 0.0;
                self.lastRelease = 0.0;
                self.lastShot = 0.0;
                self.chargeFlash = 1.0;
                self.recoil(0.5 + 0.7 * self.charge, 0);
                self.sp[s.s_grip + 1] = (self.sp[s.s_grip + 1] as f64 + 3.0) as f32;
                self.sp[s.s_grip_l + 1] = (self.sp[s.s_grip_l + 1] as f64 + 2.0) as f32;
                // ≡ L1220: `if (this.charge > 0.8) this._mxPlay(X_EFFORTP, ...)`
                if self.charge > 0.8 {
                    let x = self.tab.expr8("X_EFFORTP");
                    self.mx_play(&x, 0.5, 0.03, 0.08, 0.3, 0.0, None);
                }
            }
            _ => {}
        }
    }

    /// ≡ `_blink(hard)` (character.js L1313): `this._blinkStart(1, !!hard)`.
    pub fn blink(&mut self, hard: bool) {
        self.blink_start(1.0, hard);
    }

    /// ≡ `_blinkStart(amp, hard)` (L2992). RNG draws: c, h, o, lag, then dbl
    /// only when `!hard && amp > 0.9` (JS short-circuit).
    pub fn blink_start(&mut self, amp: f64, hard: bool) {
        let b = &mut self.bl;
        if b.t >= 0.0 && b.t < b.c {
            return; // already closing
        }
        b.t = 0.0;
        b.amp = amp;
        b.c = 0.06 + self.rng.next() * 0.025;
        b.h = if hard {
            0.08 + self.rng.next() * 0.06
        } else {
            0.008 + self.rng.next() * 0.03
        };
        b.o = 0.14 + self.rng.next() * 0.06 + if hard { 0.05 } else { 0.0 };
        b.lag = (self.rng.next() - 0.5) * 0.024;
        b.squeeze = if hard { 1.0 } else { 0.0 };
        b.dbl = !hard && amp > 0.9 && self.rng.next() < 0.14;
        b.next = 99.0;
    }

    /// ≡ `_blinkTick(dt)` (L2962).
    pub fn blink_tick(&mut self, dt: f64) {
        self.bl.next -= dt;
        self._turnBl -= dt;
        if self.yawRate.abs() > 5.5 && self._turnBl <= 0.0 && self.bl.t < 0.0 && self.kidForm {
            self._turnBl = 1.1;
            if self.rng.next() < 0.7 {
                let amp = 0.9 + 0.1 * self.rng.next();
                self.blink_start(amp, false);
            }
        }
        if self.bl.t < 0.0 && self.bl.next <= 0.0 {
            // ≡ `this.rng() < 0.18 ? 0.55 + this.rng() * 0.3 : 1`
            let amp = if self.rng.next() < 0.18 {
                0.55 + self.rng.next() * 0.3
            } else {
                1.0
            };
            self.blink_start(amp, false);
        }
        let (mut l, mut r) = (0.0f64, 0.0f64);
        if self.bl.t >= 0.0 {
            self.bl.t += dt;
            let b = &self.bl;
            l = blink_curve(b, b.t - (if b.lag < 0.0 { -b.lag } else { 0.0 })) * b.amp;
            r = blink_curve(b, b.t - (if b.lag > 0.0 { b.lag } else { 0.0 })) * b.amp;
            let done = b.t > b.c + b.h + b.o + b.lag.abs();
            if done {
                let b = &mut self.bl;
                b.t = -1.0;
                b.squeeze = 0.0;
                if b.dbl {
                    b.dbl = false;
                    b.next = 0.05 + self.rng.next() * 0.05;
                } else {
                    let mut mean = if self.inWorld { 6.4 } else { 6.0 };
                    if self.wAim > 0.5 {
                        mean = 7.5;
                    }
                    if self.weaponKind == "charger" && self.charge > 0.3 {
                        mean = 11.0;
                    }
                    mean = lerp(mean, 2.8, self.exert.max(self.wTired));
                    let rnd = self.rng.next();
                    b.next = mean * (0.3 + 1.35 * rnd.powf(1.7));
                }
            }
        }
        self.blinkL = l;
        self.blinkR = r;
        self.blinkK = l.max(r);
    }

    /// ≡ `_mxPlay(X, w, a, h, d, bras, E)` (L2842): pick the freest slot
    /// (expired slots score -1, live ones `w²`) and load it.
    #[allow(clippy::too_many_arguments)] // 1:1 with the JS signature
    pub fn mx_play(
        &mut self,
        x: &[f64; 8],
        w: f64,
        a: f64,
        h: f64,
        d: f64,
        bras: f64,
        e: Option<&[f64; 3]>,
    ) {
        let mut best = 0usize;
        let mut bw = f64::INFINITY;
        for i in 0..self.mx.len() {
            let m = &self.mx[i];
            let r = if m.t > m.a + m.h + m.d {
                -1.0
            } else {
                m.w * m.w
            };
            if r < bw {
                bw = r;
                best = i;
            }
        }
        let slot = &mut self.mx[best];
        slot.x = Some(*x);
        slot.w = w;
        slot.a = a;
        slot.h = h;
        slot.d = d;
        slot.bras = bras;
        slot.t = 0.0;
        slot.e = e.copied();
    }

    /// ≡ `_mxTick(P, dt)` (L2913): advance live slots and add-blend their
    /// expression vectors into `P` via the `XCH` channel bank (MTILT gets the
    /// sign flip; `bras`/`E` land on BRAS/SNEER/PUCKER/BLUSH).
    pub fn mx_tick(&mut self, dt: f64) {
        let tab = self.tab;
        let xch = tab.xch;
        let ch = &tab.ch;
        let mtilt = ch.MTILT;
        let p = &mut self.P;
        for i in 0..self.mx.len() {
            let m = &mut self.mx[i];
            let x = match m.x {
                Some(x) => x,
                None => continue,
            };
            m.t += dt;
            let e = m.a + m.h + m.d;
            if m.t >= e {
                m.x = None;
                continue;
            }
            let k = if m.t < m.a {
                ease(m.t / m.a)
            } else if m.t < m.a + m.h {
                1.0
            } else {
                1.0 - ease((m.t - m.a - m.h) / m.d)
            };
            let w = m.w.abs() * k;
            let sg = if m.w < 0.0 { -1.0 } else { 1.0 };
            for j in 0..8 {
                let g = if xch[j] == mtilt { sg } else { 1.0 };
                build::add_e(p, xch[j], x[j] * w * g);
            }
            build::add_e(p, ch.BRAS, m.bras * k);
            if let Some(ev) = m.e {
                build::add_e(p, ch.SNEER, ev[0] * w);
                build::add_e(p, ch.PUCKER, ev[1] * w);
                build::add_e(p, ch.BLUSH, ev[2] * w);
            }
        }
    }

    /// ≡ `_breathe(P)` (L2881): rib-cage breath cycle driven by `brPh`, with
    /// per-breath depth/rate draws and the idle deep-sigh timer.
    pub fn breathe(&mut self) {
        let bu = frac(self.brPh);
        let bn = self.brPh.floor();
        if bn != self.brN {
            self.brN = bn;
            self.brAmp = 0.8 + self.rng.next() * 0.4;
            self.brRate = 0.88 + self.rng.next() * 0.24;
            if self.sigh > 0.0 {
                self.sigh = 0.0;
            } else if self.idleT > 3.0
                || (!self.inWorld && self.wDance > 0.5 && self.dance.as_deref() != Some("victory"))
            {
                self.sighT -= 1.0;
                if self.sighT <= 0.0 {
                    self.sighT = 5.0 + self.rng.next() * 6.0;
                    self.sigh = 1.0;
                }
            }
        }
        let bb = if bu < 0.4 {
            ease(bu / 0.4)
        } else if bu < 0.85 {
            1.0 - ease((bu - 0.4) / 0.45)
        } else {
            0.0
        };
        let idle_w = (1.0 - self.gaitW) * (1.0 - self.wAir);
        let br_a = lerp(1.0, 2.3, self.exert.max(self.wTired))
            * self.brAmp
            * (if self.sigh > 0.0 { 2.3 } else { 1.0 })
            * (if self.kidForm { 1.0 } else { 0.0 });
        let br = (bb * 2.0 - 1.0) * br_a;
        let w_dance = self.wDance;
        let sigh = self.sigh;
        let ch = &self.tab.ch;
        let p = &mut self.P;
        build::sub_e(p, ch.CHEST, 0.034 * br);
        build::sub_e(p, ch.SPINE, 0.012 * br);
        build::add_e(p, ch.CLAVL + 2, 0.032 * br);
        build::sub_e(p, ch.CLAVR + 2, 0.032 * br);
        build::sub_e(p, ch.CLAVL, 0.012 * br);
        build::sub_e(p, ch.CLAVR, 0.012 * br);
        build::add_e(p, ch.UARML + 2, 0.008 * br);
        build::sub_e(p, ch.UARMR + 2, 0.008 * br);
        build::sub_e(p, ch.HIPS_P + 1, 0.003 * br * idle_w);
        build::add_e(p, ch.HEAD, 0.016 * br);
        build::add_e(p, ch.NECK, 0.004 * br);
        build::add_e(p, ch.ANC + 1, 0.003 * br * w_dance);
        p[ch.BREATH] = bb as f32;
        if sigh > 0.0 && bu > 0.4 {
            let k = win(bu, 0.4, 0.5, 0.75, 0.95);
            build::max_1(p, ch.MOPEN, 0.2 * k);
            build::sub_e(p, ch.EYE, 0.25 * k);
            build::add_e(p, ch.BROW, 0.2 * k);
            build::sub_e(p, ch.MCURVE, 0.2 * k);
            build::sub_e(p, ch.CLAVL + 2, 0.03 * k);
            build::add_e(p, ch.CLAVR + 2, 0.03 * k);
        }
    }

    /// ≡ `setDance(name)` (L1223).
    pub fn set_dance(&mut self, name: Option<&str>) {
        // ≡ `name = name || null`
        let name = name.filter(|n| !n.is_empty());
        if self.dance.as_deref() == name {
            return;
        }
        if self.dance.is_some() && name.is_some() {
            self.prevDance = self.dance.clone();
            self.prevDanceT = self.danceT;
            self.danceFade = 0.0;
        }
        self.dance = name.map(|n| n.to_owned());
        self.danceT = 0.0;
        let nv = self
            .tab
            .dance_variants
            .get(name.unwrap_or(""))
            .copied()
            .filter(|v| *v != 0.0)
            .unwrap_or(1.0);
        self.danceVar = ((self.seed >> 5) % (nv as u32)) as f64;
        if name == Some("victory") {
            let x = self.tab.expr8("X_JOY");
            let e = self.tab.expr3("E_BLUSH");
            self.mx_play(&x, 0.7, 0.08, 0.5, 0.8, 0.0, Some(&e));
            self.blink_start(1.0, false);
        } else if name == Some("defeat") {
            let x = self.tab.expr8("X_SIGH");
            self.mx_play(&x, 0.9, 0.3, 0.8, 1.2, 0.0, None);
            self.sigh = 1.0;
        }
    }

    /// ≡ `_gazeTick(dt, dx, dy)` (L3561): ballistic saccades (minimum-jerk)
    /// + fixation micro-saccades in head space.
    pub fn gaze_tick(&mut self, dt: f64, dx: f64, dy: f64) {
        let dx = clamp(dx, -0.62, 0.62);
        let dy = clamp(dy, -0.42, 0.38);
        let mut g = self.gz;
        if g.st >= 0.0 {
            g.st += dt;
            let u = (g.st / g.sd).min(1.0);
            let k = mj(u);
            g.x = lerp(g.x0, g.x1, k);
            g.y = lerp(g.y0, g.y1, k);
            if u >= 1.0 {
                g.st = -1.0;
                g.fx = g.x1;
                g.fy = g.y1;
            }
            self.gz = g;
            return;
        }
        let ex = dx - g.fx;
        let ey = dy - g.fy;
        let amp = (ex * ex + ey * ey).sqrt();
        if amp > 0.075 && dt > 0.0 {
            // ≡ `const us = amp > 0.25 ? 0.9 + 0.05 * this.rng() : 1;`
            let us = if amp > 0.25 {
                0.9 + 0.05 * self.rng.next()
            } else {
                1.0
            };
            g.x0 = g.x;
            g.y0 = g.y;
            g.x1 = g.fx + ex * us;
            g.y1 = g.fy + ey * us;
            g.sd = 0.022 + 0.1 * amp;
            g.st = 0.0;
            self.gz = g;
            // ≡ gaze-evoked blink: the draw is gated by every preceding term.
            if amp > 0.3
                && self.kidForm
                && self.bl.t < 0.0
                && self.bl.next > 0.8
                && self.rng.next() < amp * 0.3
            {
                let bamp = 0.85 + 0.15 * self.rng.next();
                self.blink_start(bamp, false);
            }
            if amp > 0.4 && self.rng.next() < 0.3 {
                let x = self.tab.expr8("X_BROWFLASH");
                self.mx_play(&x, 0.3, 0.06, 0.1, 0.3, 0.0, None);
            }
            return;
        }
        g.fx = dx;
        g.fy = dy;
        g.mt -= dt;
        if g.mt <= 0.0 {
            g.mt = 0.28 + self.rng.next() * 1.1;
            g.mx = (self.rng.next() - 0.5) * 0.07;
            g.my = (self.rng.next() - 0.5) * 0.045;
        }
        g.x = damp(g.x, g.fx + g.mx, 45.0, dt);
        g.y = damp(g.y, g.fy + g.my, 45.0, dt);
        self.gz = g;
    }

    /// ≡ `update(dt, s)` (character.js L1318-1366). `_updateLod` is render-side
    /// (B2); `lifeLv` is pinned to 2 by the harness (game tier, px 0).
    pub fn update(&mut self, dt: f64, s: &AnimState) {
        // ≡ L1319-1320
        let dt = clamp(dt.max(0.0), 0.0, 0.1);
        self._dt = dt;
        self.t += dt;
        // ≡ L1323-1325: timers count UP (elapsed since trigger). Float32 bank:
        // each write rounds the f64 sum back to f32.
        for i in 0..self.tr.len() {
            self.tr[i] = (self.tr[i] as f64 + dt) as f32;
        }
        self.lastShot += dt;
        self.lastRelease += dt;
        // ≡ L1326-1327
        if self.dance.is_some() {
            self.danceT += dt;
        }
        self.prevDanceT += dt;
        self.danceFade = (self.danceFade + dt / 0.45).min(1.0);
        // ≡ L1328-1329: the harness parents root to G.scene with a physics
        // stub → inWorld = true, phys = stub.
        self.inWorld = self.root_visible && self.root_parent_visible;
        self.phys = self.inWorld;

        // ≡ L1331-1340: form switch.
        let form = if s.form.is_empty() {
            "kid"
        } else {
            s.form.as_str()
        };
        self.grounded = s.grounded.unwrap_or(true);
        if form != self.form {
            let k0 = self.form == "kid";
            let k1 = form == "kid";
            if k0 != k1 {
                self.form_enter(form);
            } else {
                self.form = form.to_owned();
            }
            if form == "kid" {
                self.feetValid = false;
            }
        }
        self.formT += dt;
        self.kidForm = self.form == "kid";

        // ≡ L1342-1344
        self.track_root(dt, s);
        self.update_states(dt, s);
        self.update_form_scales(dt);

        // ≡ L1346-1351: external head-turn detection. `_headQW` holds the
        // LOCAL head quaternion written last frame; nothing external touches
        // it in the harness, so `extGl` stays 0 (mirrored faithfully anyway).
        if self._headSet && self.kidForm {
            let idx = self.tab.rig.bone_index["head"];
            // ≡ `_q1 = headQW⁻¹` premultiplied by the bone quat → q·q1⁻¹.
            let q1 = self.bones[idx].quat * self._headQW.inverse();
            let g = 2.0 * q1.y.atan2(q1.w);
            self.extGl = if g.abs() < 1.5 { g } else { 0.0 };
        } else {
            self.extGl = 0.0;
        }
        // ≡ L1352-1355: life detail. Harness: inWorld, lod tier game, px 0 → 2.
        self.lifeLv = 2.0;
        // ≡ L1356-1363: shown gate.
        let shown = self.root_visible && self.root_parent_visible;
        if self.kidScale > 0.001 && shown {
            feet::update_feet(self, dt, s);
            build::build_pose(self, dt, s);
            super::apply::apply_pose(self, dt, s);
        } else {
            self.feetValid = false;
            self.headInit = false;
            self._headSet = false;
        }
        // ≡ L1363
        if shown {
            squid::update_squid(self, dt, s);
        } else {
            self.sqInit = false;
        }
        // ≡ L1364-1365: `_updateMaterials` + `_updateLod` (render-side).
        self.update_materials(dt, s);
    }

    /// ≡ `_updateMaterials(dt, s)` (character.js L4045-4059): pure data
    /// outputs into `self.mat` / `self.tankFill`. `_updateRim` (L4047) drives
    /// the light rig — render-side, not ported.
    pub fn update_materials(&mut self, _dt: f64, s: &AnimState) {
        let t = self.t;
        // ≡ L4048-4049: invuln flash.
        let fl = if s.invuln {
            0.22 * (0.5 + 0.5 * (t * TAU * 6.0).sin())
        } else {
            0.0
        };
        self.mat.u_flash = [fl, fl, fl];
        // ≡ L4050-4052: special-glow pulse.
        let pul = 0.5 + 0.5 * (t * TAU * 1.6).sin();
        let gl = self.wGlow * (0.35 + 0.45 * pul);
        self.mat.u_glow = [self.color[0] * gl, self.color[1] * gl, self.color[2] * gl];
        // ≡ L4053-4054: low-ink blink on the fill emissive.
        let blink = self.wLow * (0.5 + 0.5 * (t * TAU * 3.2).sin());
        let fe = 0.12 + 0.9 * blink + 0.3 * self.wGlow * pul;
        self.mat.fill_emissive = [self.color[0] * fe, self.color[1] * fe, self.color[2] * fe];
        // ≡ L4055: squid form hides the kid → the tank ink fill shows instead.
        if !self.kid_visible {
            self.tankFill.scale.y = self.inkS.max(0.004) * self.tank.h;
            self.tankFill_visible = self.inkS > 0.005;
        }
        // ≡ L4056-4058: charger glow (charge², full-charge strobe, flash decay).
        let ch = if self.animKind == "charger" {
            self.charge
        } else {
            0.0
        };
        let full = if ch >= 0.995 {
            0.5 + 0.5 * (t * TAU * 8.0).sin()
        } else {
            0.0
        };
        self.mat.glow_emissive_intensity =
            0.15 + 2.6 * ch * ch + 1.5 * full + 3.0 * self.chargeFlash;
    }

    /// ≡ `_trackRoot(dt, s)` (character.js L1369-1417).
    pub fn track_root(&mut self, dt: f64, s: &AnimState) {
        // ≡ L1370-1374
        let r = self.root_pos;
        let yaw = self.root_yaw;
        if !self.rootInit || r.distance_squared(self.rp) > 9.0 {
            self.rp = r;
            self.rv = DVec3::ZERO;
            self.ra = DVec3::ZERO;
            self.prevYaw = yaw;
            self.rootInit = true;
            self.feetValid = false;
            self.yawRate = 0.0;
        }
        self.yaw = yaw;
        if dt > 0.0 {
            // ≡ L1377-1381: smoothed velocity / acceleration.
            let v1 = (r - self.rp) / dt;
            let v2 = self.rv;
            self.rv = self.rv.lerp(v1, 1.0 - (-dt * 32.0).exp());
            let v3 = (self.rv - v2) / dt;
            self.ra = self.ra.lerp(v3, 1.0 - (-dt * 16.0).exp());
            // ≡ L1382-1383: turn rate (authoritative `s.turnRate` when given).
            let dy = wrap_a(yaw - self.prevYaw);
            let (target, lam) = match s.turn_rate {
                Some(tr) => (clamp(tr, -14.0, 14.0), 30.0),
                None => (clamp(dy / dt, -14.0, 14.0), 14.0),
            };
            self.yawRate = damp(self.yawRate, target, lam, dt);
        }
        // ≡ L1385-1389
        self.rp = r;
        self.prevYaw = yaw;
        let (c, sn) = (yaw.cos(), yaw.sin());
        self.kvx = self.rv.x * c - self.rv.z * sn;
        self.kvz = self.rv.x * sn + self.rv.z * c;
        self.kax = self.ra.x * c - self.ra.z * sn;
        self.kaz = self.ra.x * sn + self.ra.z * c;
        self.hs = self.kvx.hypot(self.kvz);
        // ≡ L1390-1401: treadmill (root stays put, ground slides).
        let sv = s.speed.unwrap_or(0.0);
        let lm = s.local_move;
        let lml = lm.0.hypot(lm.1);
        self.tread = self.hs < 0.12 && sv > 0.4 && lml > 0.05 && self.grounded;
        if self.tread {
            let kx = (-lm.0 / lml) * sv;
            let kz = (lm.1 / lml) * sv;
            self.tvx = kx * c + kz * sn;
            self.tvz = -kx * sn + kz * c;
            self.gvx = self.tvx;
            self.gvz = self.tvz;
            self.kgx = kx;
            self.kgz = kz;
        } else {
            self.tvx = 0.0;
            self.tvz = 0.0;
            self.gvx = self.rv.x;
            self.gvz = self.rv.z;
            self.kgx = self.kvx;
            self.kgz = self.kvz;
        }
        // ≡ L1402-1408: ground speed + move direction.
        self.gv = self.gvx.hypot(self.gvz);
        let lam = if self.gv > self.gs { 16.0 } else { 7.0 };
        self.gs = damp(
            self.gs,
            if self.grounded { self.gv } else { self.gs },
            lam,
            dt,
        );
        if self.gv > 0.35 {
            let il = 1.0 / self.gv;
            self.mdx = damp(self.mdx, self.kgx * il, 10.0, dt);
            self.mdz = damp(self.mdz, self.kgz * il, 10.0, dt);
            let ml = self.mdx.hypot(self.mdz).max(1e-12);
            self.mdx /= ml;
            self.mdz /= ml;
        }
        // ≡ L1409-1416: vertical speed, air time, ground distance.
        self.vyS = damp(self.vyS, s.vy.unwrap_or(self.rv.y), 14.0, dt);
        self.airT = if self.grounded { 0.0 } else { self.airT + dt };
        self.gnd = if !self.grounded && self.phys {
            // harness physics stub: raycast never hits → gnd = 9.
            9.0
        } else if self.grounded {
            0.0
        } else {
            9.0
        };
    }

    /// ≡ `_updateStates(dt, s)` (character.js L1420-1498). All RNG draws are
    /// mirrored in order (shuffle block = 5 draws, fidget start/end, shift).
    pub fn update_states(&mut self, dt: f64, s: &AnimState) {
        // ≡ L1421-1422
        let kid = self.kidForm;
        let dance = self.dance.is_some();
        let ch = s.charge.unwrap_or(0.0);
        // ≡ L1423-1424: `_owner()` is null in the harness → s.subAim wins.
        let sub = s.sub_aim.unwrap_or(false);
        self.wSub = damp(
            self.wSub,
            if sub && kid && !dance { 1.0 } else { 0.0 },
            if sub { 14.0 } else { 9.0 },
            dt,
        );
        // ≡ L1426-1430: bomb latch (dualies only).
        if sub && !self._subPrev && kid {
            self.bombHeld = true;
            self.bombT = 0.0;
        }
        if !sub && self.wSub < 0.3 {
            self.bombHeld = false;
        }
        self._subPrev = sub;
        self.bombT += dt;
        self.bombSwap = damp(
            self.bombSwap,
            if self.dual && kid && (self.bombHeld || self.tr[self.tab.tim.t_throw] < 0.32) {
                1.0
            } else {
                0.0
            },
            16.0,
            dt,
        );
        // ≡ L1431-1436: dualies lock stance (runner null → tr[T_DODGE] path).
        {
            let td = self.tr[self.tab.tim.t_dodge] as f64;
            let dk = td / self.dodgeDur;
            let lock = kid
                && !dance
                && self.dual
                && ((td < self.dodgeDur + 0.5) || (dk > 0.55 && dk < 1.0));
            self.lockW = damp(
                self.lockW,
                if lock { 1.0 } else { 0.0 },
                if lock { 18.0 } else { 7.0 },
                dt,
            );
        }
        // ≡ L1437-1441: aim / roll / air weights.
        let aiming = kid
            && !dance
            && self.animKind != "roller"
            && (s.firing
                || ch > 0.01
                || self.lastShot < 0.5
                || self.lastRelease < 0.35
                || self.lockW > 0.5);
        self.wAim = damp(
            self.wAim,
            if aiming { 1.0 } else { 0.0 },
            if aiming { 22.0 } else { 4.5 },
            dt,
        );
        let rolling = kid
            && !dance
            && s.rolling
            && self.animKind == "roller"
            && self.tr[self.tab.tim.t_flick] > 0.6;
        self.wRoll = damp(
            self.wRoll,
            if rolling { 1.0 } else { 0.0 },
            if rolling { 11.0 } else { 6.0 },
            dt,
        );
        self.wAir = damp(
            self.wAir,
            if self.grounded { 0.0 } else { 1.0 },
            if self.grounded { 24.0 } else { 12.0 },
            dt,
        );
        self.wDance = damp(self.wDance, if dance { 1.0 } else { 0.0 }, 5.0, dt);
        // ≡ L1443-1446: charge + glow.
        self.charge = damp(self.charge, ch, 25.0, dt);
        self.fullT = if ch >= 0.995 { self.fullT + dt } else { 0.0 };
        self.chargeFlash = (self.chargeFlash - dt * 3.5).max(0.0);
        self.wGlow = damp(
            self.wGlow,
            if s.special.unwrap_or(0.0) >= 0.999 {
                1.0
            } else {
                0.0
            },
            6.0,
            dt,
        );
        // ≡ L1447-1451: low ink / ink level / tired / goo.
        self.wLow = damp(self.wLow, if s.low_ink { 1.0 } else { 0.0 }, 8.0, dt);
        self.inkS = damp(self.inkS, clamp(s.ink.unwrap_or(1.0), 0.0, 1.0), 8.0, dt);
        self.wTired = damp(
            self.wTired,
            match s.hp {
                Some(hp) => {
                    let hp = if hp > 1.001 { hp / PLAYER_HP } else { hp };
                    sstep(0.45, 0.12, hp)
                }
                None => sstep(0.55, 0.9, self.hurt) * 0.6,
            },
            3.0,
            dt,
        );
        self.wGoo = damp(
            self.wGoo,
            if s.in_enemy_ink && self.grounded && kid {
                1.0
            } else {
                0.0
            },
            6.0,
            dt,
        );
        // ≡ L1452-1456: hit accumulation, step offsets, exertion, breath.
        self.hitAcc = (self.hitAcc - dt * 1.4).max(0.0);
        self.stepOfsX = damp(self.stepOfsX, 0.0, 2.2, dt);
        self.stepOfsZ = damp(self.stepOfsZ, 0.0, 2.2, dt);
        self.exert = clamp(
            self.exert
                + (if self.gs > 3.5 { dt * 0.12 } else { -dt * 0.06 })
                + (if (self.tr[self.tab.tim.t_land] as f64) < dt * 1.5 {
                    0.05 * self.landAmp
                } else {
                    0.0
                }),
            0.0,
            1.0,
        );
        self.brPh += dt
            * lerp(0.26, 0.72, self.exert.max(self.wTired * 0.8))
            * (if self.brRate != 0.0 { self.brRate } else { 1.0 })
            * (if self.sigh > 0.0 { 0.62 } else { 1.0 });
        // ≡ L1457-1464: gait parameters from the smoothed speed.
        let v = if self.moving {
            self.gv.max(0.6)
        } else {
            self.gs
        };
        self.runW = sstep(1.7, 4.3, v);
        let rw = self.runW;
        self.duty = lerp(0.62, 0.3, rw) + 0.06 * self.wGoo;
        let half = lerp(0.19, 0.265, sstep(0.4, 5.0, v)) * (1.0 - 0.18 * self.wGoo);
        self.cad = clamp(v.max(0.6) * self.duty / (2.0 * half), 1.1, 4.4) * (1.0 - 0.1 * self.wGoo);
        self.liftH = lerp(0.05, 0.2, sstep(1.2, 5.5, v)) * (1.0 + 0.9 * self.wGoo);
        self.gaitW = damp(
            self.gaitW,
            if self.moving { 1.0 } else { 0.0 },
            if self.moving { 7.0 } else { 4.5 },
            dt,
        );
        // ≡ L1465-1472: hip twist toward the travel direction.
        let mut tw = 0.0;
        if self.moving {
            tw = self.mdx.atan2(self.mdz.abs() + 0.3) * 0.78;
            if self.mdz < -0.25 {
                tw = -tw * 0.8;
            }
            tw = clamp(tw, -0.8, 0.8) * sstep(0.5, 2.2, v);
        }
        self.hipTwist = damp(self.hipTwist, tw, 7.0, dt);
        // ≡ L1473-1492: stance preset blend (idle ⇄ aim/roll ⇄ lock).
        let aim_st = self.wAim.max(self.wRoll) * (1.0 - self.gaitW);
        let lk = self.lockW;
        let idle_now = kid
            && !dance
            && !self.moving
            && self.grounded
            && self.wAim < 0.05
            && self.wRoll < 0.05
            && self.tr[self.tab.tim.t_land] > 0.5
            && self.tr[self.tab.tim.t_spawn] > 1.2;
        // ≡ L1480-1491: idle shuffle (5 RNG draws per shuffle).
        if idle_now {
            if self.idleT > 2.5 {
                self.shufT -= dt;
            }
            if self.shufT <= 0.0 {
                self.shufT = 5.0 + self.rng.next() * 8.0;
                let o = if self.rng.next() < 0.5 { 0 } else { 3 };
                let sd = if o == 0 { 1.0 } else { -1.0 };
                let mi = (self.rng.next() * self.tab.shuffles.len() as f64) as usize;
                let m = self.tab.shuffles[mi % self.tab.shuffles.len()];
                let j = 0.88 + 0.24 * self.rng.next();
                self.stVar[o] = (sd * m[0] * j) as f32;
                self.stVar[o + 1] = (m[1] * j) as f32;
                self.stVar[o + 2] = (sd * (m[2] + (self.rng.next() - 0.5) * 0.15)) as f32;
                let q = 3 - o;
                self.stVar[q] = (self.stVar[q] as f64 * 0.3) as f32;
                self.stVar[q + 1] = (self.stVar[q + 1] as f64 * 0.3) as f32;
                self.stVar[q + 2] = (self.stVar[q + 2] as f64 * 0.3) as f32;
            }
        } else if self.moving || self.wAim > 0.2 || dance {
            for i in 0..6 {
                self.stVar[i] = 0.0;
            }
            self.shufT = self.shufT.max(3.0);
        }
        // ≡ L1492: damp the live stance toward the blended preset.
        let a = self.hold.stance;
        for (i, &ai) in a.iter().enumerate() {
            let base = self.tab.stance_idle[i] + self.stVar[i] as f64;
            let aim = lerp(ai, self.tab.stance_lock[i], lk);
            let target = lerp(base, aim, aim_st.max(lk));
            self.st[i] = damp(
                self.st[i] as f64,
                target,
                if lk > 0.5 { 16.0 } else { 9.0 },
                dt,
            ) as f32;
        }
        // ≡ L1493-1495: idle clock + fidget scheduling.
        self.idleT = if idle_now { self.idleT + dt } else { 0.0 };
        if self.fidget >= 0 {
            self.fidgetT += dt;
            if self.fidgetT > self.tab.fidget_len[self.fidget as usize] || !idle_now {
                self.fidget = -1;
                self.nextFidget = 4.0 + self.rng.next() * 5.0;
                self.idleT = self.idleT.min(1.5);
            }
        } else if idle_now && self.idleT > self.nextFidget {
            self.start_fidget();
        }
        // ≡ L1496-1497: weight shift toggle (1 draw).
        self.shiftT -= dt;
        if self.shiftT <= 0.0 {
            self.shiftTgt = -self.shiftTgt;
            self.shiftT = 3.2 + self.rng.next() * 4.0;
        }
    }

    /// ≡ `_startFidget` (character.js L1500-1506): 1-2 RNG draws (pick, and a
    /// re-pick when it repeats the last fidget).
    pub fn start_fidget(&mut self) {
        let n = self.tab.fidget_names.len() as f64;
        let mut id = (self.rng.next() * n) as i64;
        if id == self.lastFidget as i64 {
            id = (id + 1 + ((self.rng.next() * 3.0) as i64)) % self.tab.fidget_names.len() as i64;
        }
        // ≡ L1504: dual wield has no free hand for goggles/tank taps (the
        // re-pick consumes one more draw).
        if (self.dual || self.weapon.as_ref().is_some_and(|w| w.kind == "dualies"))
            && (self.tab.fidget_names[id as usize] == "goggles"
                || self.tab.fidget_names[id as usize] == "tank")
        {
            id = self
                .tab
                .fidget_names
                .iter()
                .position(|f| {
                    *f == if self.rng.next() < 0.5 {
                        "twirl"
                    } else {
                        "look"
                    }
                })
                .unwrap_or(id as usize) as i64;
        }
        self.fidget = id;
        self.lastFidget = id as f64;
        self.fidgetT = 0.0;
    }

    /// ≡ `_updateFormScales(dt)` (character.js L1514-1534): the kid ⇄ squid
    /// elastic gesture as explicit curve evaluation (no damping).
    pub fn update_form_scales(&mut self, dt: f64) {
        let _ = dt; // ≡ L1514: purely timeline-driven
        let to_kid = self.form == "kid";
        let from_kid = self.formPrev == "kid";
        let t = self.formT;
        let (mut k_u, mut k_y, mut k_xz, mut s_u, mut s_y, mut s_xz, mut lift) = (
            if to_kid { 1.0 } else { 0.0 },
            1.0,
            1.0,
            if to_kid { 0.0 } else { 1.0 },
            1.0,
            1.0,
            0.0,
        );
        if to_kid && !from_kid && t < 0.45 {
            // ≡ L1519-1523
            s_u = if t < 0.06 {
                1.0
            } else {
                1.0 - ease_in((t - 0.06) / 0.035)
            };
            s_y = kc(t, &self.tab.k_em_st, &self.tab.k_em_sy);
            s_xz = kc(t, &self.tab.k_em_st, &self.tab.k_em_sx);
            k_u = if t < 0.045 {
                0.0
            } else {
                ease_out((t - 0.045) / 0.055)
            };
            k_y = kc(t, &self.tab.k_em_kt, &self.tab.k_em_ky);
            k_xz = kc(t, &self.tab.k_em_kt, &self.tab.k_em_kx);
            lift = (if self.formPrev == "swim" {
                -0.3
            } else if self.formPrev == "climb" {
                -0.05
            } else {
                -0.12
            }) * (1.0 - ease_out((t - 0.045) / 0.1));
        } else if !to_kid && from_kid && t < 0.45 {
            // ≡ L1525-1528
            k_y = kc(t, &self.tab.k_dv_kt, &self.tab.k_dv_ky);
            k_xz = kc(t, &self.tab.k_dv_kt, &self.tab.k_dv_kx);
            k_u = if t < 0.07 {
                1.0
            } else {
                1.0 - ease_in((t - 0.07) / 0.03)
            };
            s_u = if t < 0.045 {
                0.0
            } else {
                ease_out((t - 0.045) / 0.035)
            };
            s_y = kc(t, &self.tab.k_dv_st, &self.tab.k_dv_sy);
            s_xz = kc(t, &self.tab.k_dv_st, &self.tab.k_dv_sx);
        }
        self.kidScale = k_u;
        self.sqScale = s_u;
        self.kidSY = k_y;
        self.kidSXZ = k_xz;
        self.sqSY = s_y;
        self.sqSXZ = s_xz;
        self.kidLift = lift;
        self.kidPop = if to_kid { k_u } else { 0.0 };
        self.kid_visible = k_u > 0.001;
        self.squidRoot_visible = s_u > 0.001;
    }

    /// ≡ `_formEnter(newForm)` (character.js L1537-1544): enter the new
    /// gesture timeline at the point matching what is on screen now.
    pub fn form_enter(&mut self, new_form: &str) {
        let was_kid_t = self.formT;
        let prev = self.form.clone();
        self.formPrev = prev.clone();
        self.form = new_form.to_owned();
        self.formT = 0.0;
        self.tr[self.tab.tim.t_form] = 0.0;
        if was_kid_t < 0.1 {
            if new_form == "kid" && prev != "kid" {
                self.formT = clamp(0.06 + 0.035 * (1.0 - self.sqScale), 0.0, 0.095)
                    * if self.sqScale < 0.999 { 1.0 } else { 0.0 };
            } else if new_form != "kid" && prev == "kid" {
                self.formT = if self.kidScale < 0.999 {
                    clamp(0.07 + 0.03 * (1.0 - self.kidScale), 0.0, 0.1)
                } else {
                    0.0
                };
            }
        }
    }
}

/// `PLAYER.hp` (the engine's max health) ≡ engine/player — only used for the
/// `hp > 1.001` rescale in `_updateStates` (harness hp = 1 → never taken).
const PLAYER_HP: f64 = 4.0;

/// ≡ `resolveStyle(st, seed)` (character.js L520 + character-appearance.js):
/// the harness passes every index explicitly, so this is an identity map for
/// B1 (style-derived appearance is B2/render-side).
fn resolve_style(style: [usize; 6], _seed: u32) -> [usize; 6] {
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_updateMaterials` at `t = 0`: every sine term is 0, so all envelopes
    /// sit exactly at their midpoints (character.js L4045-4059).
    #[test]
    fn update_materials_t0_exact() {
        let tab = Tables::load();
        let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);
        let s = AnimState::default(); // invuln: false
        c.wGlow = 1.0;
        c.wLow = 0.5;
        c.chargeFlash = 0.25;
        c.update_materials(1.0 / 60.0, &s);
        // L4048-4049: not invulnerable → no flash.
        assert_eq!(c.mat.u_flash, [0.0; 3]);
        // L4050-4054: pul = 0.5 → gl = 0.575; blink = 0.25 → fe = 0.495.
        for i in 0..3 {
            assert!(
                (c.mat.u_glow[i] - c.color[i] * 0.575).abs() < 1e-12,
                "u_glow[{i}]"
            );
            assert!(
                (c.mat.fill_emissive[i] - c.color[i] * 0.495).abs() < 1e-12,
                "fill[{i}]"
            );
        }
        // L4055: kid visible → tank fill untouched.
        assert_eq!(c.tankFill.scale.y, 1.0);
        assert!(c.tankFill_visible);
        // L4056-4058: not a charger → ch = 0 → 0.15 + 3·chargeFlash.
        assert!((c.mat.glow_emissive_intensity - 0.9).abs() < 1e-12);
    }

    /// L4048-4049: the invuln flash pulses in [0, 0.22], grey (r=g=b).
    #[test]
    fn update_materials_invuln_flash() {
        let tab = Tables::load();
        let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);
        let s = AnimState {
            invuln: true,
            ..AnimState::default()
        };
        c.t = 0.0;
        c.update_materials(1.0 / 60.0, &s);
        assert!((c.mat.u_flash[0] - 0.11).abs() < 1e-12);
        for k in 1..40 {
            c.t = k as f64 * 0.013;
            c.update_materials(1.0 / 60.0, &s);
            let f = c.mat.u_flash;
            assert!(f[0] >= 0.0 && f[0] <= 0.22 + 1e-12);
            assert!(f[0] == f[1] && f[1] == f[2]);
        }
    }

    /// L4055: squid form (kid hidden) → the tank ink fill tracks `inkS`.
    #[test]
    fn update_materials_squid_tank_fill() {
        let tab = Tables::load();
        let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);
        let s = AnimState::default();
        c.kid_visible = false;
        c.inkS = 0.5;
        c.update_materials(1.0 / 60.0, &s);
        assert!((c.tankFill.scale.y - 0.5 * c.tank.h).abs() < 1e-12);
        assert!(c.tankFill_visible);
        // Below the visibility floor: clamped scale, hidden fill.
        c.inkS = 0.003;
        c.update_materials(1.0 / 60.0, &s);
        assert!((c.tankFill.scale.y - 0.004 * c.tank.h).abs() < 1e-12);
        assert!(!c.tankFill_visible);
    }

    /// L4056-4058: the charger glow is ch² + a full-charge strobe + the
    /// charge-flash decay; other weapon kinds stay at the base intensity.
    #[test]
    fn update_materials_charger_glow() {
        let tab = Tables::load();
        let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);
        let s = AnimState::default();
        c.animKind = "charger".into();
        c.charge = 0.5;
        c.chargeFlash = 0.0;
        c.update_materials(1.0 / 60.0, &s);
        // t = 0, ch = 0.5 < 0.995 → 0.15 + 2.6·0.25.
        assert!((c.mat.glow_emissive_intensity - 0.8).abs() < 1e-12);
        c.charge = 1.0;
        c.update_materials(1.0 / 60.0, &s);
        // full strobe at t = 0 → 0.5: 0.15 + 2.6 + 0.75.
        assert!((c.mat.glow_emissive_intensity - 3.5).abs() < 1e-12);
        c.animKind = "shooter".into();
        c.update_materials(1.0 / 60.0, &s);
        assert!((c.mat.glow_emissive_intensity - 0.15).abs() < 1e-12);
    }
}
