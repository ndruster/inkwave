//! `IMPL.barrage` (specials.js L1247-1253) — Bomb Barrage: throw the
//! `bomb` sub every `gap` s for `duration` s (the five `barrage_*` variants
//! share this kind). Batch 3: the volley ladder rides the runner's sub path
//! (weapons.js L132-155 reads [`effective_sub`] + the `nextThrow` gate
//! through [`state_of_mut`]); the end clears the runner's aim state through
//! [`crate::actor::Actor::sp_reset_aim`]. The `character.setSub` swaps
//! (L1250/L1252) are presentation — the sim derives the thrown kind from the
//! active instead of the runner.

use crate::actor::{Actor, SpecialActive};
use crate::special::KindState;
use crate::sub::SubKind;

/// Kind state (JS `s.bomb`, `s.nextThrow`, L1249).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.bomb = SUBS[s.def.bomb] || SUBS.bomb` (L1249): the sub id the
    /// barrage throws.
    pub bomb: String,
    /// JS `s.nextThrow` (L1249): the absolute clock time of the next volley
    /// bomb (`G.time` basis; 0 = ready from the start).
    pub next_throw: f32,
}

/// JS `IMPL.barrage.start` (L1248-1251).
pub fn start(_a: &mut Actor, s: &mut SpecialActive, _id: &str) {
    // JS L1249: `s.bomb = SUBS[s.def.bomb] || SUBS.bomb` — the def's bomb
    // id, defaulting to plain `bomb`.
    let bomb = match crate::special::def_by_id(&s.id) {
        Some(crate::special::SpecialEntry::Barrage(d)) => d.bomb.clone(),
        _ => String::from("bomb"),
    };
    s.state = KindState::Barrage(State {
        bomb,
        next_throw: 0.0,
    });
    // JS L1250: `a.character.setSub?.(s.bomb.kind)` — presentation; the
    // runner reads [`effective_sub`] instead.
}

/// JS `IMPL.barrage` has no `tick` (L1247-1253): the timer/duration ladder
/// is the generic one; the volley gate lives in the runner's sub path.
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.barrage.end` (L1252): `a.weaponRunner.aimingSub = false` (the
/// `setSub` restore is presentation) — routed through
/// [`crate::actor::Actor::sp_reset_aim`] for the match to drain.
pub fn end(a: &mut Actor, _reason: &str) {
    a.sp_reset_aim = true;
}

/// JS `IMPL.barrage` state of a *live* (non-ghost, not-ended) barrage
/// active, if any.
#[must_use]
pub fn state_of(s: &SpecialActive) -> Option<&State> {
    if s.ghost || s.ended || s.kind != "barrage" {
        return None;
    }
    match &s.state {
        KindState::Barrage(st) => Some(st),
        _ => None,
    }
}

/// [`state_of`]'s mutable half (the `nextThrow` re-arm).
#[must_use]
pub fn state_of_mut(s: &mut SpecialActive) -> Option<&mut State> {
    if s.ghost || s.ended || s.kind != "barrage" {
        return None;
    }
    match &mut s.state {
        KindState::Barrage(st) => Some(st),
        _ => None,
    }
}

/// JS `weaponRunner.update` L136-137: `const bar = a.specialActive &&
/// a.specialActive.kind === 'barrage' ? a.specialActive : null; const sub =
/// bar ? bar.bomb : (a.sub || SUB.bomb)` — the sub kind the runner throws
/// this frame (the barrage's bomb overrides the loadout; an unknown id falls
/// back to the plain bomb like JS's `SUBS[...] || SUBS.bomb`, L1249).
#[must_use]
pub fn effective_sub(a: &Actor) -> SubKind {
    match a.special_active.as_ref().and_then(|s| state_of(s)) {
        // JS L133 + L1249: under a live barrage the thrown kind is the def's
        // bomb id, unknown ids falling back to the plain bomb
        // (`SUBS[def.bomb] || SUBS.bomb`).
        Some(st) => SubKind::from_id_str(&st.bomb).unwrap_or(SubKind::Bomb),
        // JS L133: otherwise the actor's own sub (`a.sub || SUB.bomb` — the
        // sim's `a.sub` always holds a valid kind).
        None => a.sub,
    }
}
