//! `IMPL.sonar` (specials.js L1262-1277) — Deep Sonar: a `cast`-second body
//! animation that reveals every enemy for `duration` s (plus a sub track).
//! Batch 2: the reveal ladder is [`apply_reveal`] (L1266-1271), run by the
//! match on the live `special:use` peek and by the replication layer for a
//! ghost start (`GHOST.sonar.start === IMPL.sonar.start`, L2051 — the
//! reveal is each client's own on its own roster). The `_stand` body physics
//! runs in [`crate::special::body`] through [`crate::actor::Actor::stand`].
//! The `sonar_ping` / `sonar_mark` sounds (L1270/L1272), the rings (L1274)
//! and the `special:sonar` emit (L1273) are presentation (not in the
//! netmatch `FORWARD` list, netmatch.js L47).

use crate::actor::{Actor, SpecialActive};
use crate::special::specials;

/// Kind state. `revealed` is a sim-side latch (no JS field): the roster half
/// of `IMPL.sonar.start` runs once per active, from the match / replication
/// layer that holds the roster the actor itself cannot see.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// The reveal ladder has run on this active ([`apply_reveal`]).
    pub revealed: bool,
}

/// JS `IMPL.sonar.start` (L1263-1265): the body flag + cast window. The
/// trigger pose (L1265) is presentation; the roster ladder is
/// [`apply_reveal`].
pub fn start(_a: &mut Actor, s: &mut SpecialActive) {
    s.body = true;
    s.dur = specials().sonar.cast;
    s.state = crate::special::KindState::Sonar(State::default());
}

/// JS `IMPL.sonar.start` L1266-1271 (the roster half): every *other-team*
/// actor takes a fresh `duration`-second reveal — a plain assignment, not a
/// max (JS L1268), and with no alive check — plus the echo-orb style track
/// (`G.subs?.track`, L1269). Runs on the whole roster including proxies:
/// JS loops `G.actors` the same way; the reveal on a proxy only matters to
/// this screen's minimap/HUD (frozen timers never decay there, match.js
/// L228), and the track refresh is presentation.
pub fn apply_reveal(caster: usize, actors: &mut [Actor]) {
    let dur = specials().sonar.duration;
    let team = actors[caster].team;
    for e in actors.iter_mut() {
        if e.team == team {
            continue; // JS L1267
        }
        e.status.reveal = dur; // JS L1268: assignment, not `max`
        e.status.reveal_team = Some(team);
        crate::sub::apply_track(&mut e.status, team, dur); // JS L1269
    }
    if let Some(s) = actors[caster].special_active.as_mut() {
        s.set_sonar_revealed();
    }
}

/// JS `IMPL.sonar` has no `tick` and no `end` (the active expires on `dur`
/// through the generic `SpecialSystem.tick`, L341).
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.sonar` has no `end`.
pub fn end(_a: &mut Actor, _reason: &str) {}
