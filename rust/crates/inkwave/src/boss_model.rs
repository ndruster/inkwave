//! Task 68 — HULLBREAKER high-fidelity MODEL presentation metadata.
//!
//! `rust/assets/boss.json` is produced by `tools/extract/extract-boss.mjs`,
//! which imports the READ-ONLY `src/boss/bossModelGeo.js` and calls the real
//! `buildRig()` / `buildParts()` to obtain the deterministic 100-bone skeleton
//! and per-tier triangle budgets. This module is the headless data plane:
//! bone-tree SHA256 golden, HIT table, six-move animation durations, material
//! slots and the `['bm']`-timeline clip-selection pure function.
//!
//! The sim's hitShapes geometry (M4 Task41/46) and `tuning.json::boss.hz` are
//! untouched — this is presentation metadata only. The 3D skinned mesh,
//! materials and animation feel are GPU-review items (D-T68-1/2).
#![allow(dead_code)] // CONTRACTS-style data-plane API: GPU consumers are M5 follow-ups

use std::sync::OnceLock;

use inkwave_sim::net::wire::{JVal, parse_json};

const DOC: &str = include_str!("../../../assets/boss.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("boss.json parses"))
}

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("boss.json: missing field {key}"))
}

fn f64_field(v: &JVal, key: &str) -> f64 {
    field(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("boss.json: missing numeric field {key}"))
}

fn str_field<'a>(v: &'a JVal, key: &str) -> &'a str {
    field(v, key)
        .as_str()
        .unwrap_or_else(|| panic!("boss.json: missing string field {key}"))
}

// ------------------------------------------------------------------ data types

/// A bone's structural identity: name, parent (empty for root) and rest
/// position quantized to 1e-6 (i64) — enough to recompute the SHA256 golden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bone {
    pub name: &'static str,
    pub parent: &'static str,
    pub rest_q: [i64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitShape {
    pub socket: &'static str,
    pub radius: f64,
    pub weak: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriBudget {
    pub scale: f64,
    pub total: u32,
    pub steel: u32,
    pub cara: u32,
    pub flesh: u32,
    pub eye: u32,
    pub junk: u32,
}

/// Clip keys the animator can select, in the priority order of JS L484-495.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Dead,
    Intro,
    Stun,
    Roar,
    MoveTele,
    MoveAct,
    MoveRec,
    Recover,
    Loco,
}

#[must_use]
pub fn bone_count() -> usize {
    f64_field(field(doc(), "rig"), "bone_count") as usize
}

#[must_use]
pub fn bone_tree_sha256() -> &'static str {
    str_field(field(doc(), "rig"), "bone_tree_sha256")
}

#[must_use]
pub fn bones() -> Vec<Bone> {
    field(doc(), "rig")
        .field("bones")
        .and_then(JVal::as_array)
        .expect("rig.bones array")
        .iter()
        .map(|b| {
            let rest = b
                .field("rest_q")
                .and_then(JVal::as_array)
                .expect("rig.bones.rest_q");
            Bone {
                name: str_field(b, "name"),
                parent: str_field(b, "parent"),
                rest_q: [
                    rest[0].as_f64().unwrap() as i64,
                    rest[1].as_f64().unwrap() as i64,
                    rest[2].as_f64().unwrap() as i64,
                ],
            }
        })
        .collect()
}

#[must_use]
pub fn hit_shapes() -> Vec<HitShape> {
    field(doc(), "hit")
        .as_array()
        .expect("hit array")
        .iter()
        .map(|h| HitShape {
            socket: str_field(h, "socket"),
            radius: f64_field(h, "radius"),
            weak: field(h, "weak").as_bool().expect("hit.weak bool"),
        })
        .collect()
}

#[must_use]
pub fn tri_budget(tier: &str) -> Option<TriBudget> {
    let row = field(doc(), "tri_budget").field(tier)?;
    let b = field(row, "buckets");
    Some(TriBudget {
        scale: f64_field(row, "scale"),
        total: f64_field(row, "total") as u32,
        steel: f64_field(b, "steel") as u32,
        cara: f64_field(b, "cara") as u32,
        flesh: f64_field(b, "flesh") as u32,
        eye: f64_field(b, "eye") as u32,
        junk: f64_field(b, "junk") as u32,
    })
}

/// Duration `[tele, act, rec]` for a move id (JS bossAnim.js L22 `DUR`).
#[must_use]
pub fn move_dur(move_id: &str) -> Option<[f64; 3]> {
    let d = field(field(doc(), "animation"), "dur").field(move_id)?;
    let a = d.as_array()?;
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

#[must_use]
pub fn moves() -> Vec<&'static str> {
    field(field(doc(), "animation"), "moves")
        .as_array()
        .expect("animation.moves")
        .iter()
        .map(|v| v.as_str().expect("move id"))
        .collect()
}

/// Material slot names (5 skinned buckets, JS bossModelGeo.js L378).
#[must_use]
pub fn material_slots() -> Vec<&'static str> {
    field(field(doc(), "materials"), "slots")
        .as_array()
        .expect("materials.slots")
        .iter()
        .map(|v| v.as_str().expect("slot"))
        .collect()
}

/// Pure clip selection mirroring JS `BossAnimator.update` L482-495. Inputs are
/// the sim state fields the JS reads; output is the animation clip key.
#[derive(Clone, Copy, Debug, Default)]
pub struct BmClipState<'a> {
    pub dead: bool,
    pub move_id: &'a str,
    pub move_phase: &'a str,
    pub stunned: bool,
    pub move_is_roar: bool,
    pub move_is_intro: bool,
    pub recovering: bool,
    pub auto_roar: bool,
    pub known_move: bool,
    pub known_phase: bool,
}

#[must_use]
pub fn select_clip(s: &BmClipState<'_>) -> Clip {
    if s.dead {
        Clip::Dead
    } else if s.move_is_intro || s.move_id == "intro" {
        Clip::Intro
    } else if s.stunned {
        Clip::Stun
    } else if s.move_is_roar || s.move_id == "roar" || (s.auto_roar && s.move_id.is_empty()) {
        Clip::Roar
    } else if !s.move_id.is_empty() && s.known_move && s.known_phase {
        match s.move_phase {
            "tele" => Clip::MoveTele,
            "act" => Clip::MoveAct,
            "rec" => Clip::MoveRec,
            _ => Clip::Loco,
        }
    } else if s.recovering {
        Clip::Recover
    } else {
        Clip::Loco
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- std-only SHA-256 (FIPS 180-4), same fixture-pinning pattern as
    //    texlib/bake.rs / env_bake.rs; canonic serialization matches the JS
    //    reference in boss-crosscheck.mjs.
    const SHA_K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    fn sha256(msg: &[u8]) -> [u8; 32] {
        let mut h = [
            0x6a09e667u32,
            0xbb67ae85,
            0x3c6ef372,
            0xa54ff53a,
            0x510e527f,
            0x9b05688c,
            0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut data = msg.to_vec();
        data.push(0x80);
        while data.len() % 64 != 56 {
            data.push(0);
        }
        data.extend_from_slice(&((msg.len() as u64) * 8).to_be_bytes());
        for block in data.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
            let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(SHA_K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (x, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *x = x.wrapping_add(v);
            }
        }
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }
    fn hex(d: &[u8; 32]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn task68_sha256_self_test() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn task68_bone_tree_matches_js_sha256_golden() {
        // Cross-language golden: boss.json stores the quantized rest per bone;
        // we re-derive the EXACT canonical string the JS reference hashed
        // (extract-boss.mjs / boss-crosscheck.mjs: name|parent|qx,qy,qz per line)
        // and recompute SHA256 with this std-only implementation. The golden in
        // boss.json was produced by node crypto from the live three.js rig —
        // this is a genuine cross-language anchor, not a self-comparison.
        assert_eq!(bone_count(), 100);
        let rows = bones();
        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0].name, "base");
        assert_eq!(rows[0].parent, "");
        assert!(rows.iter().skip(1).all(|b| !b.parent.is_empty()));
        let canonical = rows
            .iter()
            .map(|b| {
                format!(
                    "{}|{}|{},{},{}",
                    b.name, b.parent, b.rest_q[0], b.rest_q[1], b.rest_q[2]
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let golden = bone_tree_sha256();
        assert_eq!(golden.len(), 64, "SHA256 hex length");
        assert_eq!(
            hex(&sha256(canonical.as_bytes())),
            golden,
            "Rust bake must match the JS bone-tree golden"
        );
    }

    #[test]
    fn task68_hit_shapes_and_tri_budget_pinned() {
        let hits = hit_shapes();
        assert_eq!(hits.len(), 10);
        assert_eq!(
            (hits[0].socket, hits[0].radius, hits[0].weak),
            ("shellF", 1.8, false)
        );
        assert_eq!(
            (hits[5].socket, hits[5].radius, hits[5].weak),
            ("eyeL", 0.4, true)
        );
        assert_eq!(
            (hits[7].socket, hits[7].radius, hits[7].weak),
            ("belly", 0.8, true)
        );
        assert!(hits.iter().filter(|h| h.weak).count() == 5);
        // Real per-tier budgets from buildParts(), all under the 150k budget.
        for (tier, total, steel) in [
            ("low", 31830u32, 15320u32),
            ("medium", 40283, 18118),
            ("high", 59010, 25326),
            ("ultra", 85977, 36220),
        ] {
            let b = tri_budget(tier).unwrap();
            assert_eq!((b.total, b.steel), (total, steel), "tier {tier}");
            assert!(b.total <= 150000);
            assert_eq!(
                b.steel + b.cara + b.flesh + b.eye + b.junk,
                b.total,
                "bucket sum {tier}"
            );
        }
        assert_eq!(
            material_slots(),
            vec!["steel", "cara", "flesh", "eye", "junk"]
        );
    }

    #[test]
    fn task68_animation_durations_and_clip_timeline() {
        assert_eq!(
            moves(),
            vec!["slam", "barrage", "sweep", "charge", "crablets", "frenzy"]
        );
        assert_eq!(move_dur("slam"), Some([1.15, 1.2, 1.0]));
        assert_eq!(move_dur("frenzy"), Some([1.2, 3.0, 1.7]));
        // Clip selection mirrors JS L484-495 priority chain.
        assert_eq!(
            select_clip(&BmClipState {
                dead: true,
                ..Default::default()
            }),
            Clip::Dead
        );
        assert_eq!(
            select_clip(&BmClipState {
                move_id: "intro",
                move_is_intro: true,
                ..Default::default()
            }),
            Clip::Intro
        );
        assert_eq!(
            select_clip(&BmClipState {
                stunned: true,
                ..Default::default()
            }),
            Clip::Stun
        );
        assert_eq!(
            select_clip(&BmClipState {
                move_id: "roar",
                move_is_roar: true,
                ..Default::default()
            }),
            Clip::Roar
        );
        assert_eq!(
            select_clip(&BmClipState {
                move_id: "slam",
                move_phase: "tele",
                known_move: true,
                known_phase: true,
                ..Default::default()
            }),
            Clip::MoveTele
        );
        assert_eq!(
            select_clip(&BmClipState {
                move_id: "barrage",
                move_phase: "act",
                known_move: true,
                known_phase: true,
                ..Default::default()
            }),
            Clip::MoveAct
        );
        assert_eq!(
            select_clip(&BmClipState {
                move_id: "charge",
                move_phase: "rec",
                known_move: true,
                known_phase: true,
                ..Default::default()
            }),
            Clip::MoveRec
        );
        assert_eq!(
            select_clip(&BmClipState {
                recovering: true,
                ..Default::default()
            }),
            Clip::Recover
        );
        assert_eq!(select_clip(&BmClipState::default()), Clip::Loco);
    }
}
