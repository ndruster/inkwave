//! `IMPL.bubbler` (specials.js L1256-1259) — Bubble Guard: a damage-proof
//! shield for `duration` s. Batch 2: the grant runs through
//! [`crate::special::give_shield`] (L486-488); the decay / teammate sharing
//! ladder is [`crate::special::update_shields`] (L542-563, run by the match
//! after the sub step). The bubble mesh, the `shield_up` / `shield_pop`
//! sounds and the `special:shield` emit (L497) are presentation (the event
//! is not in the netmatch `FORWARD` list, netmatch.js L47).

use crate::actor::SpecialActive;
use crate::special::specials;

/// Kind state: the bubbler keeps nothing on the active beyond the shield
/// (JS `giveShield` writes `a.status.shield`, L487).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {}

/// JS `IMPL.bubbler.start` (L1257): `this.giveShield(a, s.def.duration,
/// true)` → `a.status.shield = max(a.status.shield, time)` + the owner flag
/// (L487-488). Ghost consumption runs the same call (no GHOST table entry →
/// the L282 fallback to `IMPL.bubbler.start`): each client shields its own
/// copy of the proxy.
pub fn start(a: &mut crate::actor::Actor, s: &mut SpecialActive) {
    let dur = specials().bubbler.duration;
    crate::special::give_shield(a, dur, true);
    s.state = crate::special::KindState::Bubbler(State {});
}

/// JS `IMPL.bubbler` has no `tick` (the shield decays via `status.shield`).
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.bubbler.end` (L1258): the field runs out on its own timer —
/// nothing to undo (the shield lives on `status`, and the splat drop of
/// [`crate::special::on_splat`] clears it on death).
pub fn end(_a: &mut crate::actor::Actor, _reason: &str) {}
