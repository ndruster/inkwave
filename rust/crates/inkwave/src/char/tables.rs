//! Pose tables loaded at runtime from the batch-A extractions:
//! `rust/assets/char/char-poses.json` (character.js L225-513 tables) and
//! `rust/assets/char/char.json` (rig + weapon frames). Nothing hand-copied.
#![allow(dead_code)] // Task 54 batch B/C consumers land progressively
use std::collections::HashMap;

use inkwave_sim::net::wire::{JVal, parse_json};

use super::math::TAU;

/// Pose channel bank indices ≡ `character.js` L175-206 (order is authoritative
/// in `char-poses.json:poseChannels`, which was extracted from the real file).
macro_rules! channels {
    ($($field:ident => $key:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy)]
        #[allow(non_snake_case)]
        pub struct Channels {
            /// `PN` — bank length (each channel = [pos, dir] pair).
            pub pn: usize,
            $(pub $field: usize),+
        }
        impl Channels {
            fn load(t: &JVal) -> Option<Channels> {
                let pc = t.field("poseChannels")?;
                Some(Channels {
                    pn: pc.field("PN")?.as_f64()? as usize,
                    $($field: pc.field($key)?.as_f64()? as usize),+
                })
            }
        }
    };
}
channels! {
    HIPS_P => "HIPS_P", HIPS => "HIPS", SPINE => "SPINE", CHEST => "CHEST",
    NECK => "NECK", HEAD => "HEAD", CLAVL => "CLAVL", CLAVR => "CLAVR",
    UARML => "UARML", UARMR => "UARMR", FARML => "FARML", FARMR => "FARMR",
    HANDL => "HANDL", HANDR => "HANDR", FOOTL => "FOOTL", FOOTLR => "FOOTLR",
    FOOTR => "FOOTR", FOOTRR => "FOOTRR", ANC => "ANC", ANCR => "ANCR",
    POLER => "POLER", POLEL => "POLEL", LTGT => "LTGT", LTGTR => "LTGTR",
    KNEEL => "KNEEL", KNEER => "KNEER", IKR => "IKR", IKL => "IKL",
    LTW => "LTW", LTROT => "LTROT", SPIN => "SPIN", WPL => "WPL", WPR => "WPR",
    STAB => "STAB", AFOLT => "AFOLT", AFOLR => "AFOLR", BROW => "BROW",
    BROWY => "BROWY", MCURVE => "MCURVE", MWIDTH => "MWIDTH", MOPEN => "MOPEN",
    MTILT => "MTILT", EYE => "EYE", WINK => "WINK", SQUINT => "SQUINT",
    LOOKX => "LOOKX", LOOKY => "LOOKY", MODEL => "MODEL", MODELR => "MODELR",
    SQY => "SQY", SQXZ => "SQXZ", HLY => "HLY", HLP => "HLP", CROUCH => "CROUCH",
    HANDPL => "HANDPL", HANDPR => "HANDPR", EARS => "EARS", ANL => "ANL",
    ANLR => "ANLR", TIPTOE => "TIPTOE", BRAS => "BRAS", PUPIL => "PUPIL",
    BREATH => "BREATH", SNEER => "SNEER", PUCKER => "PUCKER", BLUSH => "BLUSH",
}

/// Spring bank indices ≡ `character.js` L207-220 — each `SPG()` claims two
/// floats (`[x, v]`); the declaration order is the layout, so it is mirrored
/// line-for-line here rather than hand-copied as numbers.
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct Springs {
    pub s_lean_p: usize,
    pub s_lean_r: usize,
    pub s_pely: usize,
    pub s_sq: usize,
    pub s_arm_l: usize,
    pub s_arm_r: usize,
    pub s_elb_l: usize,
    pub s_wp_x: usize,
    pub s_wp_y: usize,
    pub s_wp_z: usize,
    pub s_wr_x: usize,
    pub s_wr_y: usize,
    pub s_rc_p: usize,
    pub s_rc_z: usize,
    pub s_rc_y: usize,
    pub s_rc_r: usize,
    pub s_hit_p: usize,
    pub s_hit_r: usize,
    pub s_hit_y: usize,
    pub s_head_p: usize,
    pub s_head_r: usize,
    pub s_hl_y: usize,
    pub s_hl_p: usize,
    pub s_shift: usize,
    pub s_tank_x: usize,
    pub s_tank_z: usize,
    pub s_tank_l: usize,
    pub s_sq_y: usize,
    pub s_sq_p: usize,
    pub s_sq_r: usize,
    pub s_sq_h: usize,
    pub s_clav: usize,
    pub s_stag: usize,
    pub s_lag_x: usize,
    pub s_lag_z: usize,
    pub s_hemp: usize,
    pub s_hemr: usize,
    pub s_tk_y: usize,
    pub s_tk_z: usize,
    pub s_tk_x: usize,
    pub s_ear_l: usize,
    pub s_ear_r: usize,
    pub s_hemv: usize,
    pub s_rc_p2: usize,
    pub s_rc_z2: usize,
    pub s_rc_y2: usize,
    pub s_att_y: usize,
    pub s_att_p: usize,
    pub s_grip: usize,
    pub s_grip_l: usize,
    /// `SPN = _sk * 2` (≡ character.js L220).
    pub spn: usize,
}

impl Springs {
    #[must_use]
    pub fn new() -> Springs {
        // ≡ character.js L183-219: `const SPG = () => (_sk++) * 2;` in order.
        let mut sk = 0usize;
        let mut g = || {
            let i = sk * 2;
            sk += 1;
            i
        };
        Springs {
            s_lean_p: g(),
            s_lean_r: g(),
            s_pely: g(),
            s_sq: g(),
            s_arm_l: g(),
            s_arm_r: g(),
            s_elb_l: g(),
            s_wp_x: g(),
            s_wp_y: g(),
            s_wp_z: g(),
            s_wr_x: g(),
            s_wr_y: g(),
            s_rc_p: g(),
            s_rc_z: g(),
            s_rc_y: g(),
            s_rc_r: g(),
            s_hit_p: g(),
            s_hit_r: g(),
            s_hit_y: g(),
            s_head_p: g(),
            s_head_r: g(),
            s_hl_y: g(),
            s_hl_p: g(),
            s_shift: g(),
            s_tank_x: g(),
            s_tank_z: g(),
            s_tank_l: g(),
            s_sq_y: g(),
            s_sq_p: g(),
            s_sq_r: g(),
            s_sq_h: g(),
            s_clav: g(),
            s_stag: g(),
            s_lag_x: g(),
            s_lag_z: g(),
            s_hemp: g(),
            s_hemr: g(),
            s_tk_y: g(),
            s_tk_z: g(),
            s_tk_x: g(),
            s_ear_l: g(),
            s_ear_r: g(),
            s_hemv: g(),
            s_rc_p2: g(),
            s_rc_z2: g(),
            s_rc_y2: g(),
            s_att_y: g(),
            s_att_p: g(),
            s_grip: g(),
            s_grip_l: g(),
            spn: sk * 2,
        }
    }
}

impl Default for Springs {
    fn default() -> Self {
        Self::new()
    }
}

/// Timer bank indices ≡ `character.js` L221-222 (`TK()` order).
#[derive(Debug, Clone, Copy)]
#[allow(non_snake_case)]
pub struct Timers {
    pub t_shoot: usize,
    pub t_flick: usize,
    pub t_throw: usize,
    pub t_land: usize,
    pub t_jump: usize,
    pub t_hit: usize,
    pub t_leap: usize,
    pub t_slam: usize,
    pub t_spawn: usize,
    pub t_rel: usize,
    pub t_impact: usize,
    pub t_brake: usize,
    pub t_form: usize,
    pub t_stag: usize,
    pub t_shoot_l: usize,
    pub t_dodge: usize,
    pub t_slosh: usize,
    pub t_admire: usize,
    pub t_flip: usize,
    pub t_wink: usize,
    /// `TN` (≡ character.js L222).
    pub tn: usize,
}

impl Timers {
    #[must_use]
    pub fn new() -> Timers {
        let mut tk = 0usize;
        let mut g = || {
            let i = tk;
            tk += 1;
            i
        };
        Timers {
            t_shoot: g(),
            t_flick: g(),
            t_throw: g(),
            t_land: g(),
            t_jump: g(),
            t_hit: g(),
            t_leap: g(),
            t_slam: g(),
            t_spawn: g(),
            t_rel: g(),
            t_impact: g(),
            t_brake: g(),
            t_form: g(),
            t_stag: g(),
            t_shoot_l: g(),
            t_dodge: g(),
            t_slosh: g(),
            t_admire: g(),
            t_flip: g(),
            t_wink: g(),
            tn: tk,
        }
    }
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}

/// Rig constants ≡ `character.js` L166-170 (`ANKLE_H`/`HIPW`/`BALL_Z`/
/// `HEEL_Z`/`HEAD_CTR`).
#[derive(Debug, Clone, Copy)]
pub struct RigConst {
    pub ankle_h: f64,
    pub hipw: f64,
    pub ball_z: f64,
    pub heel_z: f64,
    pub head_ctr: [f64; 3],
}

/// One Euler/translation frame of a HOLD table ≡ `{p:[x,y,z], r:[x,y,z]}`.
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldFrame {
    pub p: [f64; 3],
    pub r: [f64; 3],
}

/// Recoil profile ≡ `character.js` L326-331 (`HOLD.<kind>.rc`).
#[derive(Debug, Clone, Copy)]
pub struct RcProfile {
    pub kick: f64,
    pub back: f64,
    pub hz: f64,
    pub z: f64,
    pub jit: f64,
    pub torso: f64,
    pub head: f64,
    pub crouch: f64,
    pub brace: f64,
}

/// A weapon's HOLD entry ≡ `character.js` L225-360. Optional frames mirror the
/// JS `H.run || H.carry` style fallbacks (`None` = key absent).
#[derive(Debug, Clone)]
pub struct HoldDef {
    pub dual: bool,
    pub fire: Option<String>,
    pub carry: HoldFrame,
    pub two_carry: f64,
    /// ≡ `d.raise` (victory dance anchor).
    pub raise: HoldFrame,
    /// ≡ `d.lobby` (lobby dance anchor).
    pub lobby: HoldFrame,
    /// ≡ `d.lobbyTwo` (0/1).
    pub lobby_two: f64,
    pub run: Option<HoldFrame>,
    pub aim: HoldFrame,
    pub two_aim: f64,
    pub lock: Option<HoldFrame>,
    pub roll: Option<HoldFrame>,
    pub pole_r: [f64; 3],
    pub pole_l: [f64; 3],
    pub rc: RcProfile,
    pub hip: f64,
    pub chest: f64,
    pub crouch: f64,
    pub stance: [f64; 6],
}

/// Weapon mount frames ≡ `char.json:weapons.<kind>` (`frames` block plus the
/// sibling `parts` block; runtime subset: pivot / off / muzzle / parts /
/// dual-left; handR etc. are extractor-only).
#[derive(Debug, Clone)]
pub struct WeaponFrames {
    pub pivot: [f64; 3],
    pub off_pos: [f64; 3],
    pub off_quat: [f64; 4],
    pub muzzle: [f64; 3],
    /// `(partName, localPos)` in JSON order (trigger, bolt, can, led …).
    pub parts: Vec<(String, [f64; 3])>,
    /// JS `_getWeapon` L803: `d.dual && d.inHandL` builds the left-hand twin.
    pub has_left: bool,
    /// `frames.inHandL` — the weapon in LEFT-hand space (left instance:
    /// pivot at GRIP_HOLE_L, off = inHandL.pos − GRIP_HOLE_L).
    pub in_hand_l: Option<([f64; 3], [f64; 4])>,
    /// `d.handR` ≡ `char.json:frames.handR` (weapon-space fist pose used by
    /// the right-arm IK anchor, `_applyPose` L3462-3463).
    pub hand_r: ([f64; 3], [f64; 4]),
    /// `d.handL` ≡ `char.json:frames.handL` (L3482-3483, L3498-3500).
    pub hand_l: ([f64; 3], [f64; 4]),
    /// `w.spin` ≡ `char.json:frames.spinAt` (spinner / zooka barrel cluster,
    /// `_applyPose` L3532-3538).
    pub has_spin: bool,
    /// `d.drawLen || 0.2` (bow-model.js L120 `drawLen: DRAW`; only the bow def
    /// carries it) — consumed by `_poseBow` (character.js L480) and
    /// `animateBow` (bow-model.js L160, L178).
    pub draw_len: Option<f64>,
}

/// Hair strand meta ≡ `char.json:rig.hairRest[style].meta` entries.
#[derive(Debug, Clone, Copy)]
pub struct StrandMeta {
    pub dir: [f64; 3],
    pub g: f64,
    pub k: f64,
    pub len: f64,
}

/// Skeleton ≡ `char.json:rig` (boneNames / boneParent / rest).
#[derive(Debug, Clone)]
pub struct Rig {
    pub bone_names: Vec<String>,
    pub bone_index: HashMap<String, usize>,
    /// Parent bone index per bone; `None` = hangs off the kid node.
    pub bone_parent: Vec<Option<usize>>,
    /// Rest position per bone (kid space), ordered by `bone_names`.
    pub rest: Vec<[f64; 3]>,
    pub fist_offset: [f64; 3],
    pub grip_hole_l: [f64; 3],
    pub hair_max: usize,
    pub hair_segs: usize,
    /// Style-0 strand meta (7 strands for the default hair).
    pub hair_meta: Vec<StrandMeta>,
}

/// Everything the ported pose runtime reads from the asset tables.
#[derive(Debug, Clone)]
pub struct Tables {
    pub ch: Channels,
    pub spr: Springs,
    pub tim: Timers,
    pub rig_c: RigConst,
    pub rig: Rig,
    pub aim_pivot: [f64; 3],
    pub holds: HashMap<String, HoldDef>,
    pub stance_idle: [f64; 6],
    pub stance_lock: [f64; 6],
    pub shuffles: [[f64; 3]; 5],
    pub fidget_len: [f64; 7],
    /// FIDGETS names (order = index used by `_startFidget`).
    pub fidget_names: Vec<String>,
    /// ANIM_OF weapon-kind → animation-kind (≡ character.js L372-377).
    pub anim_of: HashMap<String, String>,
    pub k_gog_t: Vec<f64>,
    pub k_gog_v: Vec<f64>,
    pub k_look_t: Vec<f64>,
    pub k_look_v: Vec<f64>,
    pub k_tank_t: Vec<f64>,
    pub k_tank_v: Vec<f64>,
    pub k_menu_t: Vec<f64>,
    pub k_menu_v: Vec<f64>,
    pub k_em_st: Vec<f64>,
    pub k_em_sy: Vec<f64>,
    pub k_em_sx: Vec<f64>,
    pub k_em_kt: Vec<f64>,
    pub k_em_ky: Vec<f64>,
    pub k_em_kx: Vec<f64>,
    pub k_dv_kt: Vec<f64>,
    pub k_dv_ky: Vec<f64>,
    pub k_dv_kx: Vec<f64>,
    pub k_dv_st: Vec<f64>,
    pub k_dv_sy: Vec<f64>,
    pub k_dv_sx: Vec<f64>,
    pub weapons: HashMap<String, WeaponFrames>,
    /// ≡ character.js L378-393: expression vectors. `X_*` are 8 floats over
    /// the `XCH` channel bank, `E_*` are 3 floats (sneer/pucker/blush).
    pub expr: HashMap<String, Vec<f64>>,
    /// ≡ DANCE_VARIANTS: dance name → variant count.
    pub dance_variants: HashMap<String, f64>,
    /// ≡ NO_TWIRL (character.js L439-456): animKind → skips the twirl.
    pub no_twirl: HashMap<String, bool>,
    /// ≡ XCH (character.js L393): expression channel bank order.
    pub xch: [usize; 8],
    /// ≡ K_SL_* (slosh heave keyed curves).
    pub k_sl_t: Vec<f64>,
    pub k_sl_x: Vec<f64>,
    pub k_sl_y: Vec<f64>,
    pub k_sl_z: Vec<f64>,
    pub k_sl_p: Vec<f64>,
    pub k_sl_w: Vec<f64>,
    /// ≡ FINGER_SPREAD (character.js L477).
    pub finger_spread: [f64; 4],
    /// ≡ HOLD_HERO (character.js L476): [pos3, rot3].
    pub hold_hero: ([f64; 3], [f64; 3]),
    /// ≡ EYE_MID (character.js L396), head-bone space.
    pub eye_mid: [f64; 3],
}

const POSES_JSON: &str = include_str!("../../../../assets/char/char-poses.json");
const CHAR_JSON: &str = include_str!("../../../../assets/char/char.json");

fn arr3(v: &JVal) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

fn arr4(v: &JVal) -> Option<[f64; 4]> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    Some([
        a[0].as_f64()?,
        a[1].as_f64()?,
        a[2].as_f64()?,
        a[3].as_f64()?,
    ])
}

fn arrn(v: &JVal) -> Option<Vec<f64>> {
    let a = v.as_array()?;
    a.iter().map(|x| x.as_f64()).collect()
}

fn frame(v: &JVal) -> Option<HoldFrame> {
    Some(HoldFrame {
        p: arr3(v.field("p")?)?,
        r: arr3(v.field("r")?)?,
    })
}

/// Runtime subset of a weapon node (`weapons.<kind>`): the `frames` block plus
/// the sibling `parts` block. Kinds without a full pivot/off/muzzle set (none
/// in the shooter family) are skipped.
fn weapon_frames(w: &JVal) -> Option<WeaponFrames> {
    let fr = w.field("frames")?;
    // ≡ character-weapons.js `getWeaponDef`: `parts` sits beside `frames`.
    let parts: Vec<(String, [f64; 3])> = match w.field("parts") {
        Some(JVal::Obj(pp)) => pp
            .iter()
            .filter_map(|(pk, pv)| Some((pk.clone(), arr3(pv.field("pivot")?)?)))
            .collect(),
        _ => Vec::new(),
    };
    let off = fr.field("off")?;
    let hr = fr.field("handR")?;
    let hl = fr.field("handL")?;
    Some(WeaponFrames {
        pivot: arr3(fr.field("pivot")?)?,
        off_pos: arr3(off.field("pos")?)?,
        off_quat: arr4(off.field("quat")?)?,
        muzzle: arr3(fr.field("muzzle")?)?,
        parts,
        has_left: fr.field("dual").and_then(JVal::as_bool).unwrap_or(false)
            && fr.field("inHandL").is_some(),
        in_hand_l: fr
            .field("inHandL")
            .and_then(|ihl| Some((arr3(ihl.field("pos")?)?, arr4(ihl.field("quat")?)?))),
        hand_r: (arr3(hr.field("pos")?)?, arr4(hr.field("quat")?)?),
        hand_l: (arr3(hl.field("pos")?)?, arr4(hl.field("quat")?)?),
        has_spin: fr.field("spinAt").is_some(),
        draw_len: fr.field("drawLen").and_then(JVal::as_f64),
    })
}

fn hold(v: &JVal) -> Option<HoldDef> {
    let rc = v.field("rc")?;
    Some(HoldDef {
        // `dual` is optional in the table (JS `d.dual` → undefined ≡ false).
        dual: v.field("dual").and_then(JVal::as_bool).unwrap_or(false),
        fire: v.field("fire").and_then(JVal::as_str).map(str::to_owned),
        carry: frame(v.field("carry")?)?,
        two_carry: v.field("twoCarry")?.as_f64()?,
        raise: frame(v.field("raise")?)?,
        lobby: frame(v.field("lobby")?)?,
        lobby_two: v.field("lobbyTwo")?.as_f64()?,
        run: v.field("run").and_then(frame),
        aim: frame(v.field("aim")?)?,
        two_aim: v.field("twoAim")?.as_f64()?,
        lock: v.field("lock").and_then(frame),
        roll: v.field("roll").and_then(frame),
        pole_r: arr3(v.field("poleR")?)?,
        pole_l: arr3(v.field("poleL")?)?,
        rc: RcProfile {
            kick: rc.field("kick")?.as_f64()?,
            back: rc.field("back")?.as_f64()?,
            hz: rc.field("hz")?.as_f64()?,
            z: rc.field("z")?.as_f64()?,
            jit: rc.field("jit")?.as_f64()?,
            torso: rc.field("torso")?.as_f64()?,
            head: rc.field("head")?.as_f64()?,
            crouch: rc.field("crouch")?.as_f64()?,
            brace: rc.field("brace").and_then(JVal::as_f64).unwrap_or(0.0),
        },
        hip: v.field("hip")?.as_f64()?,
        chest: v.field("chest")?.as_f64()?,
        crouch: v.field("crouch")?.as_f64()?,
        stance: [
            v.field("stance")?.as_array()?[0].as_f64()?,
            v.field("stance")?.as_array()?[1].as_f64()?,
            v.field("stance")?.as_array()?[2].as_f64()?,
            v.field("stance")?.as_array()?[3].as_f64()?,
            v.field("stance")?.as_array()?[4].as_f64()?,
            v.field("stance")?.as_array()?[5].as_f64()?,
        ],
    })
}

impl Tables {
    /// Parse both asset JSONs. Panics on malformed assets — these are build
    /// inputs shipped in-repo, not user data.
    #[must_use]
    pub fn load() -> Tables {
        let poses = parse_json(POSES_JSON).expect("char-poses.json parse");
        let charj = parse_json(CHAR_JSON).expect("char.json parse");
        Self::from_json(&poses, &charj)
    }

    /// `expressions[name]` as an 8-float X_* vector.
    pub fn expr8(&self, name: &str) -> [f64; 8] {
        let v = self
            .expr
            .get(name)
            .unwrap_or_else(|| panic!("expression {name}"));
        [v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]
    }

    /// `expressions[name]` as a 3-float E_* vector (sneer/pucker/blush).
    pub fn expr3(&self, name: &str) -> [f64; 3] {
        let v = self
            .expr
            .get(name)
            .unwrap_or_else(|| panic!("expression {name}"));
        [v[0], v[1], v[2]]
    }

    #[must_use]
    pub fn from_json(poses: &JVal, charj: &JVal) -> Tables {
        let ch = Channels::load(poses).expect("poseChannels");
        let rc = poses.field("rigConst").expect("rigConst");
        let rig_c = RigConst {
            ankle_h: rc.field("ANKLE_H").and_then(JVal::as_f64).unwrap_or(0.085),
            hipw: rc.field("HIPW").and_then(JVal::as_f64).unwrap_or(0.092),
            ball_z: rc.field("BALL_Z").and_then(JVal::as_f64).unwrap_or(0.11),
            heel_z: rc.field("HEEL_Z").and_then(JVal::as_f64).unwrap_or(0.065),
            head_ctr: arr3(rc.field("HEAD_CTR").expect("HEAD_CTR")).expect("HEAD_CTR shape"),
        };
        let stance = |key: &str| -> [f64; 6] {
            let a = arrn(poses.field(key).expect(key)).expect(key);
            [a[0], a[1], a[2], a[3], a[4], a[5]]
        };
        let curve = |t: &str, v: &str| -> (Vec<f64>, Vec<f64>) {
            (
                arrn(poses.field(t).expect(t)).expect(t),
                arrn(poses.field(v).expect(v)).expect(v),
            )
        };
        let (k_gog_t, k_gog_v) = curve("K_GOG_T", "K_GOG_V");
        let (k_look_t, k_look_v) = curve("K_LOOK_T", "K_LOOK_V");
        let (k_tank_t, k_tank_v) = curve("K_TANK_T", "K_TANK_V");
        let (k_menu_t, k_menu_v) = curve("K_MENU_T", "K_MENU_V");
        let (k_em_st, k_em_sy) = (
            arrn(poses.field("K_EM_ST").expect("K_EM_ST")).expect("K_EM_ST"),
            arrn(poses.field("K_EM_SY").expect("K_EM_SY")).expect("K_EM_SY"),
        );
        let (k_em_sx, k_em_kt) = (
            arrn(poses.field("K_EM_SX").expect("K_EM_SX")).expect("K_EM_SX"),
            arrn(poses.field("K_EM_KT").expect("K_EM_KT")).expect("K_EM_KT"),
        );
        let (k_em_ky, k_em_kx) = (
            arrn(poses.field("K_EM_KY").expect("K_EM_KY")).expect("K_EM_KY"),
            arrn(poses.field("K_EM_KX").expect("K_EM_KX")).expect("K_EM_KX"),
        );
        let (k_dv_kt, k_dv_ky) = (
            arrn(poses.field("K_DV_KT").expect("K_DV_KT")).expect("K_DV_KT"),
            arrn(poses.field("K_DV_KY").expect("K_DV_KY")).expect("K_DV_KY"),
        );
        let (k_dv_kx, k_dv_st) = (
            arrn(poses.field("K_DV_KX").expect("K_DV_KX")).expect("K_DV_KX"),
            arrn(poses.field("K_DV_ST").expect("K_DV_ST")).expect("K_DV_ST"),
        );
        let (k_dv_sy, k_dv_sx) = (
            arrn(poses.field("K_DV_SY").expect("K_DV_SY")).expect("K_DV_SY"),
            arrn(poses.field("K_DV_SX").expect("K_DV_SX")).expect("K_DV_SX"),
        );
        // ≡ character.js L260: 5 rows of [a, b, c] triples.
        let shuf_j = poses
            .field("SHUFFLES")
            .expect("SHUFFLES")
            .as_array()
            .expect("SHUFFLES");
        let shuffles: [[f64; 3]; 5] = shuf_j
            .iter()
            .map(|row| {
                let r = arrn(row).expect("SHUFFLES row");
                [r[0], r[1], r[2]]
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("SHUFFLES len 5");
        let fl = arrn(poses.field("FIDGET_LEN").expect("FIDGET_LEN")).expect("FIDGET_LEN");
        let fidget_len = [fl[0], fl[1], fl[2], fl[3], fl[4], fl[5], fl[6]];
        let fidget_names: Vec<String> = poses
            .field("FIDGETS")
            .expect("FIDGETS")
            .as_array()
            .expect("FIDGETS")
            .iter()
            .map(|v| v.as_str().expect("fidget name").to_owned())
            .collect();
        let mut anim_of = HashMap::new();
        if let JVal::Obj(pairs) = poses.field("ANIM_OF").expect("ANIM_OF") {
            for (k, v) in pairs {
                anim_of.insert(k.clone(), v.as_str().expect("anim kind").to_owned());
            }
        }

        let mut holds = HashMap::new();
        let hj = poses.field("HOLD").expect("HOLD");
        if let JVal::Obj(pairs) = hj {
            for (k, v) in pairs {
                holds.insert(k.clone(), hold(v).unwrap_or_else(|| panic!("HOLD.{k}")));
            }
        }

        // ---- rig (char.json) ----
        let rigj = charj.field("rig").expect("rig");
        let names_j = rigj
            .field("boneNames")
            .expect("boneNames")
            .as_array()
            .expect("boneNames");
        let bone_names: Vec<String> = names_j
            .iter()
            .map(|v| v.as_str().expect("bone name").to_owned())
            .collect();
        let mut bone_index = HashMap::new();
        for (i, n) in bone_names.iter().enumerate() {
            bone_index.insert(n.clone(), i);
        }
        let bp = rigj.field("boneParent").expect("boneParent");
        let restj = rigj.field("rest").expect("rest");
        let bone_parent: Vec<Option<usize>> = bone_names
            .iter()
            .map(|n| {
                bp.field(n)
                    .and_then(JVal::as_str)
                    .and_then(|p| bone_index.get(p).copied())
            })
            .collect();
        let rest: Vec<[f64; 3]> = bone_names
            .iter()
            .map(|n| {
                arr3(restj.field(n).unwrap_or_else(|| panic!("rest.{n}"))).expect("rest shape")
            })
            .collect();
        let hair_rest0 = rigj
            .field("hairRest")
            .and_then(|h| h.field("0"))
            .expect("hairRest 0");
        let meta_j = hair_rest0
            .field("meta")
            .expect("hair meta")
            .as_array()
            .expect("meta");
        let hair_meta: Vec<StrandMeta> = meta_j
            .iter()
            .map(|m| StrandMeta {
                dir: arr3(m.field("dir").expect("dir")).expect("dir shape"),
                g: m.field("G").and_then(JVal::as_f64).unwrap_or(1.0),
                k: m.field("K").and_then(JVal::as_f64).unwrap_or(1.0),
                len: m.field("len").expect("len").as_f64().expect("len shape"),
            })
            .collect();
        let rig = Rig {
            bone_names,
            bone_index,
            bone_parent,
            rest,
            fist_offset: arr3(rigj.field("fistOffset").expect("fistOffset"))
                .expect("fistOffset shape"),
            grip_hole_l: arr3(rigj.field("gripHoleL").expect("gripHoleL"))
                .expect("gripHoleL shape"),
            hair_max: rigj.field("hairMax").and_then(JVal::as_f64).unwrap_or(8.0) as usize,
            hair_segs: rigj.field("hairSegs").and_then(JVal::as_f64).unwrap_or(3.0) as usize,
            hair_meta,
        };

        let mut weapons = HashMap::new();
        let wj = charj.field("weapons").expect("weapons");
        let wpairs: &[(String, JVal)] = if let JVal::Obj(pairs) = wj {
            pairs
        } else {
            &[]
        };
        for (k, w) in wpairs {
            if let Some(wf) = weapon_frames(w) {
                weapons.insert(k.clone(), wf);
            }
        }

        // ---- B2 expression / dance tables (char-poses.json) ----
        let mut expr = HashMap::new();
        if let JVal::Obj(pairs) = poses.field("expressions").expect("expressions") {
            for (k, v) in pairs {
                expr.insert(
                    k.clone(),
                    arrn(v).unwrap_or_else(|| panic!("expressions.{k}")),
                );
            }
        }
        let mut dance_variants = HashMap::new();
        if let JVal::Obj(pairs) = poses.field("DANCE_VARIANTS").expect("DANCE_VARIANTS") {
            for (k, v) in pairs {
                dance_variants.insert(k.clone(), v.as_f64().expect("dance variant"));
            }
        }
        let mut no_twirl = HashMap::new();
        if let JVal::Obj(pairs) = poses.field("NO_TWIRL").expect("NO_TWIRL") {
            for (k, v) in pairs {
                no_twirl.insert(k.clone(), v.as_bool().unwrap_or(false));
            }
        }
        let xch_j = arrn(poses.field("XCH").expect("XCH")).expect("XCH");
        let xch = [
            xch_j[0] as usize,
            xch_j[1] as usize,
            xch_j[2] as usize,
            xch_j[3] as usize,
            xch_j[4] as usize,
            xch_j[5] as usize,
            xch_j[6] as usize,
            xch_j[7] as usize,
        ];
        let k_sl_t = arrn(poses.field("K_SL_T").expect("K_SL_T")).expect("K_SL_T");
        let k_sl_x = arrn(poses.field("K_SL_X").expect("K_SL_X")).expect("K_SL_X");
        let k_sl_y = arrn(poses.field("K_SL_Y").expect("K_SL_Y")).expect("K_SL_Y");
        let k_sl_z = arrn(poses.field("K_SL_Z").expect("K_SL_Z")).expect("K_SL_Z");
        let k_sl_p = arrn(poses.field("K_SL_P").expect("K_SL_P")).expect("K_SL_P");
        let k_sl_w = arrn(poses.field("K_SL_W").expect("K_SL_W")).expect("K_SL_W");
        let fs = arrn(poses.field("FINGER_SPREAD").expect("FINGER_SPREAD")).expect("FINGER_SPREAD");
        let finger_spread = [fs[0], fs[1], fs[2], fs[3]];
        let hj = poses.field("HOLD_HERO").expect("HOLD_HERO");
        let hold_hero = (
            arr3(hj.field("p").expect("HOLD_HERO.p")).expect("HOLD_HERO p"),
            arr3(hj.field("r").expect("HOLD_HERO.r")).expect("HOLD_HERO r"),
        );
        // EYE_MID ≡ character.js L396; not in the legacy rigConst block, so it
        // rides as a dedicated key when present and falls back to the literal.
        let eye_mid = arr3(poses.field("EYE_MID").unwrap_or(&JVal::Arr(vec![
            JVal::Num(0.0),
            JVal::Num(0.188),
            JVal::Num(0.15),
        ])))
        .expect("EYE_MID shape");

        Tables {
            ch,
            spr: Springs::new(),
            tim: Timers::new(),
            rig_c,
            rig,
            aim_pivot: arr3(rigj.field("aimPivot").expect("aimPivot")).expect("aimPivot shape"),
            holds,
            stance_idle: stance("STANCE_IDLE"),
            stance_lock: stance("STANCE_LOCK"),
            shuffles,
            fidget_len,
            fidget_names,
            anim_of,
            k_gog_t,
            k_gog_v,
            k_look_t,
            k_look_v,
            k_tank_t,
            k_tank_v,
            k_menu_t,
            k_menu_v,
            k_em_st,
            k_em_sy,
            k_em_sx,
            k_em_kt,
            k_em_ky,
            k_em_kx,
            k_dv_kt,
            k_dv_ky,
            k_dv_kx,
            k_dv_st,
            k_dv_sy,
            k_dv_sx,
            weapons,
            expr,
            dance_variants,
            no_twirl,
            xch,
            k_sl_t,
            k_sl_x,
            k_sl_y,
            k_sl_z,
            k_sl_p,
            k_sl_w,
            finger_spread,
            hold_hero,
            eye_mid,
        }
    }

    /// `STANCE_IDLE` derived the same way the JS derives it (L364:
    /// `[HIPW + 0.012, 0.014, 0.19, -HIPW - 0.008, -0.01, -0.2]`), as an
    /// anchor check that the JSON table matches the rig constants.
    #[must_use]
    pub fn stance_idle_derived(&self) -> [f64; 6] {
        let h = self.rig_c.hipw;
        [h + 0.012, 0.014, 0.19, -h - 0.008, -0.01, -0.2]
    }

    /// Hair strand count for the default style (≡ `hairMeta.length`).
    #[must_use]
    pub fn n_strands(&self) -> usize {
        self.rig.hair_meta.len()
    }

    /// `TAU` re-export so consumers don't reach into `math`.
    #[must_use]
    pub const fn tau() -> f64 {
        TAU
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load_and_match_js_constants() {
        let t = Tables::load();
        assert_eq!(t.ch.pn, 126, "PN ≡ character.js L206");
        assert_eq!(t.spr.spn, 100, "SPN ≡ character.js L220");
        assert_eq!(t.tim.tn, 20, "TN ≡ character.js L222");
        // Channel layout must be exactly the JS declaration order.
        assert_eq!(t.ch.HIPS_P, 0);
        assert_eq!(t.ch.HIPS, 3);
        assert_eq!(t.ch.BLUSH, 125);
        assert_eq!(t.ch.IKR, 78);
        assert_eq!(t.ch.IKL, 79);
        assert_eq!(t.rig_c.ankle_h, 0.085);
        assert_eq!(t.rig_c.hipw, 0.092);
        assert_eq!(t.rig_c.ball_z, 0.11);
        assert_eq!(t.rig_c.heel_z, 0.065);
        assert_eq!(t.rig_c.head_ctr, [0.0, 0.164, 0.014]);
        assert_eq!(t.stance_idle, t.stance_idle_derived());
        assert_eq!(t.rig.bone_names.len(), 87);
        assert_eq!(t.rig.bone_parent[t.rig.bone_index["hips"]], None);
        assert_eq!(
            t.rig.bone_parent[t.rig.bone_index["tank"]],
            Some(t.rig.bone_index["chest"])
        );
        assert_eq!(
            t.n_strands(),
            7,
            "style 0 strand count ≡ fixture hair.nStrands"
        );
        assert!(t.holds.contains_key("shooter"));
        assert!(t.weapons.contains_key("shooter"));
    }
}
