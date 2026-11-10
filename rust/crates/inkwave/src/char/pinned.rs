//! Pinned reference tests for the Task 54 batch B1 port: the harness inputs
//! recorded by `rust/tools/extract/char-pose-reference.mjs` are replayed
//! through the pure-Rust `CharAnim` and every recorded LOCAL node transform is
//! diffed within 1e-4. The fixture is read from disk (no embedding); the
//! tables it cross-checks against are the runtime-loaded ones.
#![cfg(test)]

use glam::{DMat4, DQuat, DVec3};
use inkwave_sim::net::wire::{JVal, parse_json};

use super::anim::{AnimState, CharAnim, GroupXform, TrigArg, WeaponInstance, pose_neutral};
use super::math::decompose_three;
use super::tables::Tables;

const DT: f64 = 1.0 / 60.0;

/// ≡ the harness `trigger(name, arg)` argument shape. Mirrors the JS
/// coercions (`+v || 0`, `amount ?? amp ?? 1`, `arg.hand | 0`) at the fixture
/// boundary so `CharAnim::trigger` sees exactly what JS saw.
fn parse_trig_arg(name: &str, v: Option<&JVal>) -> TrigArg {
    let Some(v) = v else {
        return TrigArg::None;
    };
    match v {
        JVal::Null => TrigArg::None,
        JVal::Num(n) => TrigArg::Num(*n),
        JVal::Obj(entries) => {
            let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, vv)| vv);
            let num_or0 = |k: &str| get(k).and_then(JVal::as_f64).unwrap_or(0.0);
            match name {
                "hit" => {
                    let amount = get("amount")
                        .and_then(JVal::as_f64)
                        .or_else(|| get("amp").and_then(JVal::as_f64))
                        .unwrap_or(1.0);
                    TrigArg::Hit {
                        x: num_or0("x"),
                        z: num_or0("z"),
                        amount,
                    }
                }
                "dodge" => TrigArg::Dodge {
                    x: num_or0("x"),
                    z: num_or0("z"),
                    t: num_or0("t"),
                },
                "shoot" => TrigArg::Shoot {
                    hand: num_or0("hand") as i64,
                },
                other => panic!("unsupported object trig arg for '{other}'"),
            }
        }
        other => panic!("unsupported trig arg kind for '{name}': {other:?}"),
    }
}
const TOL: f64 = 1e-4;
const SEG_NAMES: [&str; 7] = [
    "A idle",
    "B-E motion",
    "F jump/land",
    "G shoot",
    "H form-switch",
    "I kid+tail",
    "J dances",
];

fn fixture() -> JVal {
    let path = format!(
        "{}/tests/fixtures/char-pose-ref.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).expect("char-pose-ref.json fixture exists");
    parse_json(&src).expect("fixture parses as JSON")
}

fn nums(v: &JVal) -> Vec<f64> {
    v.as_array()
        .expect("json array")
        .iter()
        .map(|e| e.as_f64().expect("json number"))
        .collect()
}

fn num(o: &JVal, k: &str) -> Option<f64> {
    o.field(k).and_then(JVal::as_f64)
}

fn boolean(o: &JVal, k: &str) -> Option<bool> {
    o.field(k).and_then(JVal::as_bool)
}

/// `s` → `AnimState` (harness `mkState` writes every key each frame; `special`
/// and `runner` are `null` → `None`).
fn state_from_json(s: &JVal) -> AnimState {
    let lm = s.field("localMove").expect("localMove");
    AnimState {
        grounded: boolean(s, "grounded"),
        vy: num(s, "vy"),
        speed: num(s, "speed"),
        local_move: (num(lm, "x").unwrap_or(0.0), num(lm, "z").unwrap_or(0.0)),
        turn_rate: num(s, "turnRate"),
        aim_pitch: num(s, "aimPitch").unwrap_or(0.0),
        aim_yaw: num(s, "aimYaw").unwrap_or(0.0),
        firing: boolean(s, "firing").unwrap_or(false),
        charge: num(s, "charge"),
        sub_aim: boolean(s, "subAim"),
        rolling: boolean(s, "rolling").unwrap_or(false),
        low_ink: boolean(s, "lowInk").unwrap_or(false),
        ink: num(s, "ink"),
        hp: num(s, "hp"),
        in_enemy_ink: boolean(s, "inEnemyInk").unwrap_or(false),
        form: s
            .field("form")
            .and_then(JVal::as_str)
            .unwrap_or("kid")
            .to_owned(),
        special: num(s, "special"),
        invuln: boolean(s, "invuln").unwrap_or(false),
        runner: None,
        wall_normal: None,
    }
}

/// `THREE.Euler.setFromRotationMatrix(q, 'XYZ').x` ≡
/// `atan2(2(wx − yz), 1 − 2(xx + yy))` (the recorded `rotation.x` of nodes
/// written with `rotation.set(x, 0, z)` / `rotation.x = …`).
fn quat_rot_x(q: &DQuat) -> f64 {
    let [x, y, z, w] = q.to_array();
    (2.0 * (w * x - y * z)).atan2(1.0 - 2.0 * (x * x + y * y))
}

/// `THREE.Object3D.updateMatrix()` ≡ compose(T·R·S).
fn compose(g: &GroupXform) -> DMat4 {
    DMat4::from_scale_rotation_translation(g.scale, g.quat, g.pos)
}

fn part<'a>(w: &'a WeaponInstance, name: &str) -> &'a super::anim::PartXform {
    w.parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("weapon part `{name}`"))
}

/// Scenario segment boundaries (frame indices), detected from the fixture
/// inputs themselves — never hand-copied from the harness source.
#[derive(Clone, Copy)]
struct Bounds {
    run: usize,
    jump: usize,
    shoot: usize,
    squid: usize,
    kid_back: usize,
    dance: usize,
}

impl Bounds {
    fn seg(&self, t: usize) -> usize {
        if t < self.run {
            0
        } else if t < self.jump {
            1
        } else if t < self.shoot {
            2
        } else if t < self.squid {
            3
        } else if t < self.kid_back {
            4
        } else if t < self.dance {
            5
        } else {
            6
        }
    }
}

#[derive(Default)]
struct Replay {
    bounds: Option<Bounds>,
    max_err: f64,
    max_label: String,
    max_frame: usize,
    seg_max: [f64; 7],
    seg_label: [String; 7],
    seg_frame: [usize; 7],
    violations: Vec<String>,
}

impl Replay {
    fn note(&mut self, frame: usize, label: &str, got: f64, want: f64) {
        let e = (got - want).abs();
        if let Some(b) = &self.bounds {
            let s = b.seg(frame);
            if e > self.seg_max[s] {
                self.seg_max[s] = e;
                self.seg_label[s] = label.to_owned();
                self.seg_frame[s] = frame;
            }
        }
        if e > self.max_err {
            self.max_err = e;
            self.max_label = label.to_owned();
            self.max_frame = frame;
        }
        if e > TOL && self.violations.len() < 30 {
            self.violations.push(format!(
                "f{frame} {label}: want {want:.6} got {got:.6} err {e:.3e}"
            ));
        }
    }

    /// Quaternion compare, sign-agnostic (`q` ≡ `-q` in THREE).
    fn note_quat(&mut self, frame: usize, label: &str, got: &DQuat, want: &[f64]) {
        let g = got.to_array();
        let dot: f64 = g.iter().zip(want.iter()).map(|(a, b)| a * b).sum();
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        for k in 0..4 {
            self.note(frame, &format!("{label}.q{k}"), g[k] * s, want[k]);
        }
    }

    /// `PS(o)` ≡ [pos3, quat4, scale3].
    fn note_ps(&mut self, frame: usize, label: &str, got: &GroupXform, want: &[f64]) {
        let p = got.pos.to_array();
        let sc = got.scale.to_array();
        for k in 0..3 {
            self.note(frame, &format!("{label}.p{k}"), p[k], want[k]);
        }
        self.note_quat(frame, label, &got.quat, &want[3..7]);
        for k in 0..3 {
            self.note(frame, &format!("{label}.s{k}"), sc[k], want[7 + k]);
        }
    }
}

#[test]
fn char_pose_reference_pinned() {
    let fx = fixture();
    let header = fx.field("header").expect("header");
    let frames = fx
        .field("frames")
        .expect("frames")
        .as_array()
        .expect("frames array");
    let tab = Tables::load();
    let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);

    // ---- header anchors: the tables/instances agree before the replay ----
    let meta = header.field("meta").expect("meta");
    assert_eq!(
        meta.field("frames").and_then(JVal::as_f64).unwrap() as usize,
        frames.len()
    );
    assert_eq!(
        meta.field("name").and_then(JVal::as_str).unwrap(),
        "pin-test"
    );
    assert_eq!(
        meta.field("seed").and_then(JVal::as_f64).unwrap() as u32,
        c.seed
    );
    assert_eq!(
        meta.field("weapon").and_then(JVal::as_str).unwrap(),
        c.weaponKind
    );
    let layout = header.field("layout").expect("layout");
    assert_eq!(
        layout.field("PN").and_then(JVal::as_f64).unwrap() as usize,
        tab.ch.pn
    );
    assert_eq!(
        layout.field("SPN").and_then(JVal::as_f64).unwrap() as usize,
        tab.spr.spn
    );
    assert_eq!(
        layout.field("TN").and_then(JVal::as_f64).unwrap() as usize,
        tab.tim.tn
    );
    // `init` is read off the live instance AFTER the scenario; with the B2
    // layers enabled every clock in it advances during the replay, so all of
    // them are asserted post-replay (below) instead of against construction.
    let init = header.field("init").expect("init");
    let tank = header.field("tank").expect("tank");
    assert!((tank.field("tilt").and_then(JVal::as_f64).unwrap() - c.tank.tilt).abs() < 1e-9);
    assert!((tank.field("h").and_then(JVal::as_f64).unwrap() - c.tank.h).abs() < 1e-9);
    assert!((tank.field("bottom").and_then(JVal::as_f64).unwrap() - c.tank.bottom).abs() < 1e-9);
    let w = c.weapon.clone().expect("shooter weapon instance");
    let wh = header.field("weapon").expect("weapon header");
    assert_eq!(wh.field("kind").and_then(JVal::as_str).unwrap(), w.kind);
    assert_eq!(
        wh.field("animKind").and_then(JVal::as_str).unwrap(),
        c.animKind
    );
    let pivot_pos = nums(wh.field("pivotPos").expect("pivotPos"));
    for (k, (&want, got)) in pivot_pos.iter().zip(w.pivot.pos.to_array()).enumerate() {
        assert!((want - got).abs() < 1e-9, "weapon pivotPos[{k}]");
    }
    let off_pos = nums(wh.field("offPos").expect("offPos"));
    let off_quat = nums(wh.field("offQuat").expect("offQuat"));
    for (k, (&want, got)) in off_pos.iter().zip(w.off.pos.to_array()).enumerate() {
        assert!((want - got).abs() < 1e-9, "weapon offPos[{k}]");
    }
    let oq = w.off.quat.to_array();
    for k in 0..4 {
        assert!((off_quat[k] - oq[k]).abs() < 1e-9, "weapon offQuat[{k}]");
    }
    let muz = nums(wh.field("muzzle").expect("muzzle"));
    for (k, (&want, got)) in muz.iter().zip(w.muzzle_pos.to_array()).enumerate() {
        assert!((want - got).abs() < 1e-9, "weapon muzzle[{k}]");
    }
    let part_pivots = wh.field("partPivots").expect("partPivots");
    for p in &w.parts {
        let want = nums(
            part_pivots
                .field(&p.name)
                .unwrap_or_else(|| panic!("partPivots.{}", p.name)),
        );
        for (k, (&wk, gk)) in want.iter().zip(p.base_pos.to_array()).enumerate() {
            assert!((wk - gk).abs() < 1e-9, "part {} pivot[{}]", p.name, k);
        }
    }

    // ---- bone name → Rust index mapping (fixture order is authoritative) ----
    let bone_names = header
        .field("boneNames")
        .expect("boneNames")
        .as_array()
        .expect("array");
    assert_eq!(bone_names.len(), 87);
    let bidx: Vec<usize> = bone_names
        .iter()
        .map(|n| tab.rig.bone_index[n.as_str().expect("bone name")])
        .collect();

    // ---- parent chain handR → kid (exclusive) for the wpOff world compose ----
    let mut chain = Vec::new();
    {
        let mut o = tab.rig.bone_index["handR"];
        loop {
            chain.push(o);
            match tab.rig.bone_parent[o] {
                Some(p) => o = p,
                None => break,
            }
        }
        chain.reverse();
    }
    // ≡ same, for the dual LEFT instance (handL chain).
    let mut chain_l = Vec::new();
    {
        let mut o = tab.rig.bone_index["handL"];
        loop {
            chain_l.push(o);
            match tab.rig.bone_parent[o] {
                Some(p) => o = p,
                None => break,
            }
        }
        chain_l.reverse();
    }

    // ---- segment bounds from the inputs ----
    let mut bounds = Bounds {
        run: usize::MAX,
        jump: usize::MAX,
        shoot: usize::MAX,
        squid: usize::MAX,
        kid_back: usize::MAX,
        dance: usize::MAX,
    };
    for f in frames.iter() {
        let t = f.field("t").and_then(JVal::as_f64).unwrap() as usize;
        let form = f
            .field("s")
            .and_then(|s| s.field("form"))
            .and_then(JVal::as_str)
            .unwrap_or("kid");
        let trig = f.field("trig").and_then(JVal::as_array).unwrap_or(&[]);
        let has = |n: &str| {
            trig.iter()
                .any(|e| e.as_array().and_then(|a| a.first()).and_then(JVal::as_str) == Some(n))
        };
        if bounds.run == usize::MAX && num(f.field("s").unwrap(), "speed").unwrap_or(0.0) > 0.0 {
            bounds.run = t;
        }
        if bounds.jump == usize::MAX && has("jump") {
            bounds.jump = t;
        }
        if bounds.shoot == usize::MAX && has("shoot") {
            bounds.shoot = t;
        }
        if bounds.squid == usize::MAX && form == "squid" {
            bounds.squid = t;
        }
        if bounds.squid != usize::MAX
            && bounds.kid_back == usize::MAX
            && t > bounds.squid
            && form == "kid"
        {
            bounds.kid_back = t;
        }
        if bounds.dance == usize::MAX && f.field("dance").is_some() {
            bounds.dance = t;
        }
    }
    let mut rep = Replay {
        bounds: Some(bounds),
        ..Replay::default()
    };

    // ---- replay ----
    for f in frames.iter() {
        let t = f.field("t").and_then(JVal::as_f64).unwrap() as usize;
        let rp = nums(f.field("rp").expect("rp"));
        c.root_pos = DVec3::new(rp[0], rp[1], rp[2]);
        c.root_yaw = f.field("ry").and_then(JVal::as_f64).unwrap();
        if let Some(kw) = f.field("setW").and_then(JVal::as_str) {
            c.set_weapon(kw);
        }
        // ≡ harness `emit` extra: `setDance` then a raw `danceVar` override,
        // both before `update` (character.js L1223-1235 / L599).
        if let Some(d) = f.field("dance") {
            c.set_dance(d.as_str());
        }
        if let Some(dv) = f.field("danceVar").and_then(JVal::as_f64) {
            c.danceVar = dv;
        }
        for tr in f
            .field("trig")
            .expect("trig")
            .as_array()
            .expect("trig array")
        {
            let a = tr.as_array().expect("trigger pair");
            let name = a[0].as_str().expect("trigger name");
            let arg = parse_trig_arg(name, a.get(1));
            c.trigger(name, arg);
        }
        let st = state_from_json(f.field("s").expect("s"));
        c.update(DT, &st);
        let out = f.field("o").expect("o");
        let ob = out
            .field("bones")
            .expect("bones")
            .as_array()
            .expect("bones array");
        assert_eq!(ob.len(), 87 * 2);
        for i in 0..87 {
            let bp = nums(&ob[2 * i]);
            let bq = nums(&ob[2 * i + 1]);
            let got = &c.bones[bidx[i]];
            let p = got.pos.to_array();
            for k in 0..3 {
                rep.note(t, &format!("bones[{i}].p{k}"), p[k], bp[k]);
            }
            rep.note_quat(t, &format!("bones[{i}]"), &got.quat, &bq);
        }
        for (label, got) in [
            ("kid", &c.kid),
            ("model", &c.model),
            ("squidRoot", &c.squidRoot),
            ("squidPivot", &c.squidPivot),
        ] {
            let want = nums(out.field(label).unwrap());
            rep.note_ps(t, label, got, &want);
        }
        let tf = nums(out.field("tankFill").expect("tankFill"));
        rep.note(t, "tankFill.y", c.tankFill.pos.y, tf[0]);
        rep.note(t, "tankFill.rx", quat_rot_x(&c.tankFill.quat), tf[1]);
        rep.note(t, "tankFill.sx", c.tankFill.scale.x, tf[2]);
        rep.note(t, "tankFill.sz", c.tankFill.scale.z, tf[3]);

        let wref = c.weapon.as_ref().expect("weapon");
        let wp = nums(out.field("wpPivot").expect("wpPivot"));
        rep.note(t, "wpPivot.rx", quat_rot_x(&wref.pivot.quat), wp[0]);
        rep.note(t, "wpPivot.sx", wref.pivot.scale.x, wp[1]);

        // wpOff ≡ WPS(w.off): world compose root·model·kid·chain·pivot·off.
        // The recorded scale pins the squash carried by the kid group.
        let wo = nums(out.field("wpOff").expect("wpOff"));
        let mut m = DMat4::from_scale_rotation_translation(
            DVec3::ONE,
            DQuat::from_rotation_y(c.root_yaw),
            c.root_pos,
        );
        m *= compose(&c.model);
        m *= compose(&c.kid);
        for &bi in &chain {
            m *= compose(&c.bones[bi]);
        }
        m *= compose(&wref.pivot);
        m *= compose(&wref.off);
        let (mp, mr, ms) = decompose_three(&m);
        let ma = ms.to_array();
        for k in 0..3 {
            rep.note(t, &format!("wpOff.s{k}"), ma[k], wo[7 + k]);
        }
        let pa = mp.to_array();
        for k in 0..3 {
            rep.note(t, &format!("wpOff.p{k}"), pa[k], wo[k]);
        }
        rep.note_quat(t, "wpOff", &mr, &wo[3..7]);

        let wm = nums(out.field("wpMuzzle").expect("wpMuzzle"));
        let mp3 = wref.muzzle_pos.to_array();
        for k in 0..3 {
            rep.note(t, &format!("wpMuzzle.{k}"), mp3[k], wm[k]);
        }
        // slosher worldQuat anchor ≡ harness `wpWq` (captured at the
        // _animWeapon call point; null on every other weapon frame)
        match out.field("wpWq") {
            Some(JVal::Arr(a)) => {
                let q = c.last_off_wq.expect("rust off_world_quat missing");
                rep.note_quat(
                    t,
                    "wpWq",
                    &q,
                    &a.iter()
                        .map(|e| e.as_f64().expect("wpWq num"))
                        .collect::<Vec<_>>(),
                );
            }
            Some(JVal::Null) => {
                assert!(
                    c.last_off_wq.is_none(),
                    "f{t}: wpWq null but rust has a quat"
                );
                assert!(
                    c.last_wq_chain.is_empty(),
                    "f{t}: wpChain null but rust has a chain"
                );
            }
            _ => {}
        }
        if let Some(JVal::Arr(rows)) = out.field("wpChain") {
            const CH: [&str; 10] = [
                "kid", "hips", "spine", "chest", "clavR", "uArmR", "fArmR", "handR", "pivot", "off",
            ];
            assert_eq!(
                c.last_wq_chain.len(),
                CH.len(),
                "f{t}: rust wq chain length"
            );
            for (i, row) in rows.iter().enumerate() {
                rep.note_quat(
                    t,
                    &format!("chain.{}", CH[i]),
                    &c.last_wq_chain[i],
                    &nums(row),
                );
            }
        }
        // per-instance spring bank ≡ harness `wpPs` (Float32Array(12); null for
        // kit weapons whose animateWeapon early-returns before L1350 init)
        if let Some(JVal::Arr(_)) = out.field("wpPs") {
            let ps = nums(out.field("wpPs").expect("wpPs"));
            for (k, (&got, &want)) in wref.ps.iter().zip(ps.iter()).enumerate() {
                rep.note(t, &format!("wpPs[{k}]"), got as f64, want);
            }
        }
        // dynamic per-part locals ≡ harness `wpParts`: [name, pos3, quat4, scale3]
        for e in out
            .field("wpParts")
            .expect("wpParts")
            .as_array()
            .expect("array")
        {
            let a = e.as_array().expect("part entry");
            let name = a[0].as_str().expect("part name");
            let p = part(wref, name);
            let pp = nums(&a[1]);
            let pa = p.pos.to_array();
            for k in 0..3 {
                rep.note(t, &format!("{name}.p{k}"), pa[k], pp[k]);
            }
            rep.note_quat(t, name, &p.quat, &nums(&a[2]));
            let sc = p.scale.to_array();
            let ss = nums(&a[3]);
            for k in 0..3 {
                rep.note(t, &format!("{name}.s{k}"), sc[k], ss[k]);
            }
        }

        // dual left-hand instance ≡ harness `wpLeft` (pivot/off/parts)
        match out.field("wpLeft") {
            Some(v @ JVal::Obj(_)) => {
                let lw = wref
                    .left
                    .as_ref()
                    .unwrap_or_else(|| panic!("f{t}: rust has no left instance"));
                let pv = nums(v.field("pivot").expect("left pivot"));
                rep.note(t, "wpLeftPivot.rx", quat_rot_x(&lw.pivot.quat), pv[0]);
                rep.note(t, "wpLeftPivot.sx", lw.pivot.scale.x, pv[1]);
                let ow = nums(v.field("off").expect("left off"));
                let mut m = DMat4::from_scale_rotation_translation(
                    DVec3::ONE,
                    DQuat::from_rotation_y(c.root_yaw),
                    c.root_pos,
                );
                m *= compose(&c.model);
                m *= compose(&c.kid);
                for &bi in &chain_l {
                    m *= compose(&c.bones[bi]);
                }
                m *= compose(&lw.pivot);
                m *= compose(&lw.off);
                let (mp, mr, ms) = decompose_three(&m);
                let ma = ms.to_array();
                for k in 0..3 {
                    rep.note(t, &format!("wpLeftOff.s{k}"), ma[k], ow[7 + k]);
                }
                let pa = mp.to_array();
                for k in 0..3 {
                    rep.note(t, &format!("wpLeftOff.p{k}"), pa[k], ow[k]);
                }
                rep.note_quat(t, "wpLeftOff", &mr, &ow[3..7]);
                for e in v
                    .field("parts")
                    .expect("left parts")
                    .as_array()
                    .expect("array")
                {
                    let a = e.as_array().expect("part entry");
                    let name = a[0].as_str().expect("part name");
                    let p = part(lw, name);
                    let pp = nums(&a[1]);
                    let pa = p.pos.to_array();
                    for k in 0..3 {
                        rep.note(t, &format!("L.{name}.p{k}"), pa[k], pp[k]);
                    }
                    rep.note_quat(t, &format!("L.{name}"), &p.quat, &nums(&a[2]));
                    let sc = p.scale.to_array();
                    let ss = nums(&a[3]);
                    for k in 0..3 {
                        rep.note(t, &format!("L.{name}.s{k}"), sc[k], ss[k]);
                    }
                }
            }
            Some(JVal::Null) => assert!(
                wref.left.is_none(),
                "f{t}: wpLeft null but rust has a left instance"
            ),
            _ => {}
        }
    }

    let mut report = String::new();
    for s in 0..7 {
        report.push_str(&format!(
            "  seg {} [{}, {}): max err {:.3e} @ f{} {}",
            SEG_NAMES[s],
            [
                bounds.run,
                bounds.jump,
                bounds.shoot,
                bounds.squid,
                bounds.kid_back,
                bounds.dance,
                usize::MAX
            ][s]
                .min(frames.len()),
            [
                bounds.jump,
                bounds.shoot,
                bounds.squid,
                bounds.kid_back,
                bounds.dance,
                usize::MAX,
                usize::MAX
            ][s]
                .min(frames.len()),
            rep.seg_max[s],
            rep.seg_frame[s],
            rep.seg_label[s],
        ));
        report.push('\n');
    }
    assert!(
        rep.max_err <= TOL,
        "pose reference replay exceeded {TOL}e0: max err {:.3e} at frame {} ({})\nsegments:\n{report}first violations:\n{}",
        rep.max_err,
        rep.max_frame,
        rep.max_label,
        rep.violations.join("\n")
    );

    // end-of-scenario clocks (the harness read them at this exact point): the
    // B1 timers plus the B2 blink/microexpr/look/life clocks — matching these
    // pins the mulberry32 draw order through the enabled B2 layers.
    for (k, v) in [
        ("shufT", c.shufT),
        ("shiftT", c.shiftT),
        ("nextFidget", c.nextFidget),
        ("blNext", c.bl.next),
        ("attT", c.att.t),
        ("mxT", c.mxT),
        ("sighT", c.sighT),
        ("gripT", c.gripT),
        ("earT", c.earT),
        ("lookT", c.lookT),
        ("danceOfs", c.danceOfs),
    ] {
        let want = init
            .field(k)
            .and_then(JVal::as_f64)
            .unwrap_or_else(|| panic!("init.{k}"));
        assert!(
            (want - v).abs() < 1e-6,
            "init.{k} (post-replay): fixture {want} rust {v}"
        );
    }
}

/// kid → squid → kid form-switch state machine: the gesture timeline drives
/// `kidScale`/`sqScale` through the pop/merge curves and the visibility
/// mirrors follow (≡ `_updateFormScales` L1514-1534 + `_formEnter` L1537-1544).
#[test]
fn kid_squid_form_switch() {
    let tab = Tables::load();
    let mut c = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);

    // settled kid
    for _ in 0..30 {
        c.update(DT, &AnimState::default());
    }
    assert_eq!(c.form, "kid");
    assert!(c.kidScale > 0.999 && c.sqScale < 1e-3);
    assert!(c.kid_visible && !c.squidRoot_visible);

    // kid → squid (idle): pop curve completes within 0.45 s
    let squid = AnimState {
        form: "squid".into(),
        ..Default::default()
    };
    for _ in 0..30 {
        c.update(DT, &squid);
    }
    assert_eq!(c.form, "squid");
    assert_eq!(c.formPrev, "kid");
    assert!(c.sqScale > 0.999, "sqScale {}", c.sqScale);
    assert!(c.kidScale < 1e-3, "kidScale {}", c.kidScale);
    assert!(!c.kid_visible && c.squidRoot_visible);
    assert!(c.sqInit);
    assert!(c.squidPivot.pos.y > 0.0 && c.squidPivot.scale.y > 0.0);
    assert_eq!(c.kidPop, 0.0);

    // dry-hop run: the hop phase advances and the pivot bounces (the root
    // translates like the harness `run()`, since `hs` is root-derived).
    let squid_run = AnimState {
        form: "squid".into(),
        speed: Some(3.0),
        local_move: (0.0, 1.0),
        ..Default::default()
    };
    let mut max_hop_y = 0.0f64;
    for _ in 0..60 {
        c.root_pos.z += 3.0 * DT;
        c.update(DT, &squid_run);
        max_hop_y = max_hop_y.max(c.squidPivot.pos.y);
    }
    assert!(c.hopPhase > 0.0, "hopPhase {}", c.hopPhase);
    assert!(max_hop_y > 0.165 + 0.01, "hop peak y {max_hop_y}");

    // squid → kid (merge curve), then settled kid again
    for _ in 0..60 {
        c.update(DT, &AnimState::default());
    }
    assert_eq!(c.form, "kid");
    assert_eq!(c.formPrev, "squid");
    assert!(c.kidScale > 0.999 && c.sqScale < 1e-3);
    assert!(c.kid_visible && !c.squidRoot_visible);
    // `_updateSquid` early-returns once `sqScale <= 0.001`, so the root scale
    // freezes at its last live value (≡ JS: hidden by `visible`, not by 0).
    assert!(c.squidRoot.scale.x > 0.0 && c.squidRoot.scale.x < 1.0);
    assert!(!c.sqInit);

    // no NaN anywhere in the mirrored scene graph
    for b in &c.bones {
        assert!(b.pos.is_finite() && b.scale.is_finite() && b.quat.is_finite());
    }
    assert!(c.kid.pos.is_finite() && c.kid.scale.is_finite() && c.kid.quat.is_finite());
    assert!(c.tankFill.pos.is_finite() && c.tankFill.scale.is_finite());
}

/// Batch-A bank layout + `poseNeutral` + HOLD anchors, cross-checked against
/// the fixture header (no hand-copied numbers).
#[test]
fn pose_neutral_and_hold_anchors() {
    let tab = Tables::load();
    let fx = fixture();
    let layout = fx.field("header").unwrap().field("layout").unwrap();

    // pose bank ≡ PN=126 / SPN=100 / TN=20, agreeing with the harness layout.
    assert_eq!(tab.ch.pn, 126);
    assert_eq!(tab.spr.spn, 100);
    assert_eq!(tab.tim.tn, 20);
    assert_eq!(
        layout.field("PN").and_then(JVal::as_f64).unwrap() as usize,
        tab.ch.pn
    );
    assert_eq!(
        layout.field("SPN").and_then(JVal::as_f64).unwrap() as usize,
        tab.spr.spn
    );
    assert_eq!(
        layout.field("TN").and_then(JVal::as_f64).unwrap() as usize,
        tab.tim.tn
    );
    let chans = layout.field("channels").unwrap();
    for (name, got) in [
        ("IKR", tab.ch.IKR),
        ("SQY", tab.ch.SQY),
        ("STAB", tab.ch.STAB),
        ("FOOTL", tab.ch.FOOTL),
        ("HANDR", tab.ch.HANDR),
    ] {
        assert_eq!(
            chans.field(name).and_then(JVal::as_f64).unwrap() as usize,
            got,
            "channel index {name}"
        );
    }
    let rc = layout.field("rigConst").unwrap();
    assert!((rc.field("ANKLE_H").and_then(JVal::as_f64).unwrap() - tab.rig_c.ankle_h).abs() < 1e-9);
    assert!((rc.field("HIPW").and_then(JVal::as_f64).unwrap() - tab.rig_c.hipw).abs() < 1e-9);

    // `poseNeutral` ≡ character.js L191-201 (fresh bank + the constructor call).
    let mut p = vec![7.0f32; tab.ch.pn];
    pose_neutral(&tab, &mut p);
    let c = &tab.ch;
    assert_eq!(p[c.FOOTL], tab.rig_c.hipw as f32);
    assert_eq!(p[c.FOOTL + 1], tab.rig_c.ankle_h as f32);
    assert_eq!(p[c.FOOTL + 2], -0.004f32);
    assert_eq!(p[c.FOOTR], -(tab.rig_c.hipw as f32));
    assert_eq!(p[c.POLER], -0.7f32);
    assert_eq!(p[c.POLEL + 2], -0.45f32);
    assert_eq!(p[c.IKR], 1.0f32);
    assert_eq!(p[c.WPL], 1.0f32);
    assert_eq!(p[c.WPR], 1.0f32);
    assert_eq!(p[c.STAB], 0.82f32);
    assert_eq!(p[c.MCURVE], 0.75f32);
    assert_eq!(p[c.EYE], 1.0f32);
    assert_eq!(p[c.SQY], 1.0f32);
    assert_eq!(p[c.SQXZ], 1.0f32);
    assert_eq!(p[c.PUPIL], 0.5f32);
    assert_eq!(p[c.FARML], -0.3f32);
    assert_eq!(p[c.HANDL + 2], -0.1f32);
    assert_eq!(p[c.HANDR + 2], 0.1f32);
    assert_eq!(p[c.BROW], 0.0f32); // untouched channels stay zero
    assert_eq!(p[c.MOPEN], 0.0f32);
    // the constructor's own bank equals the standalone call.
    let anim = CharAnim::new(&tab, "pin-test", "#ff8a14", "shooter", [0, 0, 0, 0, 0, 0]);
    assert_eq!(anim.P, p);

    // HOLD anchors: the shooter family is single-hand with a live rc profile.
    let h = tab.holds.get("shooter").expect("shooter hold");
    assert!(!h.dual);
    assert!(h.rc.hz > 0.0);
    assert!(h.rc.kick > 0.0);
    assert_eq!(anim.hold.rc.hz, h.rc.hz);
    assert_eq!(anim.weaponKind, "shooter");
    assert!(!anim.dual);
    assert!(anim.weapon.is_some());
}
