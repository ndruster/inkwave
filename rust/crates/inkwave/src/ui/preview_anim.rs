//! M6 Task 72 — real `CharAnim` bone-tree pose driving the lobby previews,
//! replacing the M5 per-slot facade's identity-only transform (D-T62-1).
//!
//! The full [`CharAnim`] runtime is pure data (`Tables::load()` ← include_str!,
//! no GPU handle) and its `kid`/`bones`/`model` transforms are all `pub`, so the
//! pose is headless-anchorable. This module:
//! - promotes [`Tables`] to a `'static` [`OnceLock`] so `CharAnim<'static>` can
//!   live in an ECS component (Send+Sync: Tables holds only owned data);
//! - exposes [`step_preview_pose`], a PURE function `AnimState → PreviewPose`
//!   that constructs/updates one `CharAnim` deterministically from a named
//!   seed, for逐帧 numeric comparison in the sandbox;
//! - provides [`PreviewAnim`] (component) that keeps the CharAnim across
//!   frames and writes `kid` pos/quat/scale onto the preview root `Transform`
//!   (the mesh/skeleton generation stays a user-machine review item, D-M6-8).
//!
//! Upstream: `src/engine/character.js` `new Character` / `_buildRig` and
//! `src/game/showcase.js` `_lobSwap`/`_updateSet` emote triggers. No upstream
//! tree edit; all data flows through the extracted char tables.

use std::sync::OnceLock;

use bevy::prelude::Component;

use crate::char::anim::{AnimState, CharAnim, TrigArg};
use crate::char::tables::Tables;

/// M6 Task 72 / review P1-3: build a non-idle `AnimState` for a lobby preview
/// from its live flags + emote progress. The extracted emote `v`/`dur`/`t0`
/// drive `speed`/`charge`; `ready` marks a subtle idle bend. This makes the
/// real bone tree visibly animate instead of being fed a constant default.
#[must_use]
pub fn preview_anim_state(ready: bool, emote_speed: f64, emote_charge: f64) -> AnimState {
    AnimState {
        grounded: Some(true),
        vy: Some(0.0),
        speed: Some(emote_speed.clamp(0.0, 1.0)),
        charge: Some(emote_charge.clamp(0.0, 1.0)),
        // A ready player stands alert (hp=1, ink high); an unready one has a
        // slightly drained idle (hp=0.85) — the spring rig reads `hp` and
        // produces a visibly different neutral pose (review P1-3).
        hp: Some(if ready { 1.0 } else { 0.85 }),
        form: "kid".into(),
        ..Default::default()
    }
}

/// Map an emote name (from `lobbyset.json`) to the nearest CharAnim one-shot.
/// `None` = no trigger (the per-frame state above still animates the rig).
#[must_use]
pub fn emote_trigger(name: &str) -> Option<(&'static str, TrigArg)> {
    match name {
        "flex" | "victory" => Some(("jump", TrigArg::None)),
        "wave" | "taunt" => Some(("special_leap", TrigArg::None)),
        "dance" => Some(("charge_release", TrigArg::None)),
        _ => Some(("jump", TrigArg::None)),
    }
}

static TABLES: OnceLock<Tables> = OnceLock::new();

fn tables() -> &'static Tables {
    TABLES.get_or_init(Tables::load)
}

/// A headless-comparable snapshot of the preview bone-tree pose (root/kid +
/// a checksum of the 87 rig bones). All values are f64 from the CharAnim banks
/// (the f32→f64 rounding follows the pinned replay fixture convention).
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewPose {
    pub kid_pos: [f32; 3],
    pub kid_quat: [f32; 4],
    pub kid_scale: [f32; 3],
    pub bone_count: usize,
    /// FNV-1a-style fold of every bone transform; deterministic for a given
    /// (name, style, seed, dt-sequence), so two runs can be逐帧 compared.
    pub bone_hash: u64,
}

/// Deterministic per-slot seed from a stable name (no `RandomState` hashing).
#[must_use]
pub fn slot_seed(name: &str) -> u32 {
    let mut h = 2166136261u32;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Pure CharAnim step: build one runtime for `(name, style, seed)`, advance by
/// `dt` with `state`, and snapshot its pose. Used by the headless anchors; no
/// Bevy world, GPU handle or asset server involved.
#[must_use]
#[allow(dead_code)] // anchor surface: headless逐帧 pose comparison (TR-72.1)
pub fn step_preview_pose(
    name: &str,
    style: [usize; 6],
    seed: u32,
    dt: f64,
    state: &AnimState,
) -> PreviewPose {
    let mut c = CharAnim::new(tables(), name, "#ffffff", "shooter", style);
    c.seed = seed;
    c.update(dt, state);
    snapshot(&c)
}

fn snapshot(c: &CharAnim<'_>) -> PreviewPose {
    let kid = &c.kid;
    let mut hash = 1469598103934665603u64;
    for b in &c.bones {
        for v in [b.pos.x, b.pos.y, b.pos.z, b.scale.x, b.scale.y, b.scale.z] {
            hash ^= v.to_bits();
            hash = hash.wrapping_mul(1099511628211);
        }
        let q = b.quat;
        for v in [q.x, q.y, q.z, q.w] {
            hash ^= v.to_bits();
            hash = hash.wrapping_mul(1099511628211);
        }
    }
    PreviewPose {
        kid_pos: [kid.pos.x as f32, kid.pos.y as f32, kid.pos.z as f32],
        kid_quat: [
            kid.quat.x as f32,
            kid.quat.y as f32,
            kid.quat.z as f32,
            kid.quat.w as f32,
        ],
        kid_scale: [kid.scale.x as f32, kid.scale.y as f32, kid.scale.z as f32],
        bone_count: c.bones.len(),
        bone_hash: hash,
    }
}

/// ECS component retaining one preview's CharAnim state across frames. The
/// body/head mesh stays the M5 facade geometry (D-M6-8); this drives the root
/// transform from the real bone-tree `kid` pose plus the emote facade arc.
#[derive(Component)]
pub struct PreviewAnim {
    pub anim: CharAnim<'static>,
    #[allow(dead_code)] // retained for anchor/debug; name drives the seed
    pub slot: usize,
    /// Last emote serial that fired a one-shot trigger (avoid re-triggering
    /// every frame; JS preempts the old queue on a fresh serial, L2002-2005).
    pub fired_emote_serial: u32,
}

impl PreviewAnim {
    #[must_use]
    pub fn new(slot: usize, name: &str, style: [usize; 6], seed: u32) -> Self {
        let mut anim = CharAnim::new(tables(), name, "#ffffff", "shooter", style);
        anim.seed = seed;
        Self {
            anim,
            slot,
            fired_emote_serial: 0,
        }
    }

    /// Advance this preview by `dt` with the supplied state and return a
    /// comparable pose snapshot.
    pub fn step(&mut self, dt: f64, state: &AnimState) -> PreviewPose {
        self.anim.update(dt, state);
        snapshot(&self.anim)
    }

    /// Root transform delta from the real `kid` pose (translation y, yaw from
    /// the quaternion, and a squash proxy from the scale y). Combined with the
    /// facade's slot position/emote arc in `showcase_members_update`. The yaw
    /// is extracted from the DQuat (glam 0.30 from the sim crate) with the
    /// standard YXZ formula to avoid a direct glam-version dependency.
    #[must_use]
    pub fn root_delta(&self) -> (f32, f32, f32) {
        let kid = &self.anim.kid;
        let q = kid.quat;
        // Y-axis component of the quaternion (YXZ yaw):
        // yaw = atan2(2(wy + xz), 1 - 2(y² + z²)).
        let yaw = (2.0 * (q.w * q.y + q.x * q.z)).atan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z));
        (kid.pos.y as f32, yaw as f32, kid.scale.y as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task72_preview_pose_is_deterministic_frame_by_frame() {
        // Same (name, style, seed) + dt-sequence ⇒ identical pose snapshots
        // every frame (the headless substitute for a visual逐帧 comparison).
        let style = [0usize, 1, 2, 3, 4, 5];
        let state = AnimState::default();
        let seq = [1.0 / 60.0, 1.0 / 60.0, 1.0 / 30.0, 0.05];
        let mut a = PreviewAnim::new(0, "inkwave-preview", style, slot_seed("p0"));
        let mut b = PreviewAnim::new(0, "inkwave-preview", style, slot_seed("p0"));
        for &dt in &seq {
            assert_eq!(a.step(dt, &state), b.step(dt, &state));
        }
    }

    #[test]
    fn task72_preview_pose_has_full_bone_tree() {
        let pose = step_preview_pose("rig-check", [0; 6], 42, 0.016, &AnimState::default());
        assert!(
            pose.bone_count >= 80,
            "full CharAnim rig has ~87 bones, got {}",
            pose.bone_count
        );
        assert!(pose.bone_hash != 0, "bone transforms must be populated");
    }

    #[test]
    fn task72_slot_seed_is_stable_and_distinct() {
        assert_eq!(slot_seed("alpha"), slot_seed("alpha"));
        assert_ne!(slot_seed("alpha"), slot_seed("beta"));
    }

    #[test]
    fn task72_emote_state_changes_bone_pose_vs_idle() {
        // Review P1-3: prove the bone tree is not fed a constant idle state —
        // an emote-driven AnimState must produce a different pose than idle.
        let mut idle = PreviewAnim::new(0, "pose-emote", [0; 6], 7);
        let mut active = PreviewAnim::new(0, "pose-emote", [0; 6], 7);
        let idle_state = preview_anim_state(false, 0.0, 0.0);
        let emote_state = preview_anim_state(true, 1.0, 1.0);
        const FRAMES: usize = 60;
        let mut last_idle = idle.step(1.0 / 60.0, &idle_state);
        let mut last_active = active.step(1.0 / 60.0, &emote_state);
        for _ in 1..FRAMES {
            last_idle = idle.step(1.0 / 60.0, &idle_state);
            last_active = active.step(1.0 / 60.0, &emote_state);
        }
        assert_ne!(
            last_idle.bone_hash, last_active.bone_hash,
            "emote-driven bone tree must differ from idle (full rig pose)"
        );
    }
}
