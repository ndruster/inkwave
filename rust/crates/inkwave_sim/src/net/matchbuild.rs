//! Task 25 — the net-match builder, moved verbatim out of the app crate
//! (`inkwave::net::match_link` L79-148) so the test-only `net-harness` can
//! assemble the exact same match the Bevy app does (the app crate is
//! bin-only, so the harness cannot depend on it; one builder, one truth).
//!
//! The logic is unchanged and still mirrors JS `new Match({roster: cfg.roster,
//! …})` + `setup()` (match.js `_setupRoster` L107-113, placement L85-91) and
//! `startNetMatch`'s local-slot resolution (match.js L116, netmatch.js L79).
//! Declared deviations carried over with the code: ④ bot brains use
//! `tuning.difficulty.easy` (caller-side), ⑤ the match seed is
//! `fnv1a(cfg.id)` (JS uses `Math.random()`; the Rust sim's determinism
//! contract wants a stable seed).

use crate::actor::{Actor, SimWorld};
use crate::match_::{Match, SPAWN_RING_N, SPAWN_RING_PHI, SPAWN_RING_R};
use crate::net::session::{RosterEntry, StartCfg};
use crate::tuning::{MatchConfig, PlayerTuning};
use crate::weapon::WeaponRunner;

/// FNV-1a 64-bit over the match id (deviation ⑤).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// JS `new Match({roster: cfg.roster, …})` + `setup()` (match.js `_setupRoster`
/// L107-113, placement L85-91): one actor per roster entry **in roster order**
/// (the contract that keeps the replication layer's parallel arrays aligned),
/// each placed on its team pad's spawn ring facing the arena, spawn invuln
/// cleared (the intro is the protection). The base [`Match::new`] scaffold
/// (paint grid / projectiles / rng / clock) is reused; its balanced demo
/// roster is replaced wholesale.
#[must_use]
pub fn build_net_match(
    cfg: &StartCfg,
    seed: u64,
    sim: &SimWorld,
    t: &PlayerTuning,
    mc: &MatchConfig,
) -> Match {
    let mut m = Match::new(cfg.duration.max(1) as f32, seed, sim, t, mc);
    m.actors = cfg
        .roster
        .iter()
        .map(|r| {
            let mut a = Actor::new(r.team as usize, r.slot as usize, t);
            // match_.rs L74-78 pub constants, verbatim `place_on_pad`.
            let pad = sim.spawn_pads[a.team];
            let ang = (a.slot as f32 / SPAWN_RING_N) * std::f32::consts::TAU + SPAWN_RING_PHI;
            let p = glam::vec3(
                pad.x + ang.cos() * SPAWN_RING_R,
                pad.y,
                pad.z + ang.sin() * SPAWN_RING_R,
            );
            let yaw = if a.team == 0 {
                0.0
            } else {
                std::f32::consts::PI
            };
            a.spawn_at(p, yaw, sim, t);
            a.invuln = 0.0;
            a
        })
        .collect();
    m.runners = (0..m.actors.len()).map(|_| WeaponRunner::new()).collect();
    m
}

/// JS `startNetMatch`'s local-slot resolution: `match.js` L116 picks
/// `m.local` over the *final* actor array — `cfg.roster` minus the entries
/// `prune_loading_leavers` dropped — and that drop is gated on the
/// humans-only stage (netmatch.js L79). On a bots stage nothing is dropped,
/// so no owner may be skipped here either (review M-1).
#[must_use]
pub fn local_roster_index(
    roster: &[RosterEntry],
    my_id: &str,
    members: &[String],
    no_bots: bool,
) -> usize {
    roster
        .iter()
        .filter(|r| !no_bots || r.owner == my_id || members.contains(&r.owner))
        .position(|r| !r.bot && r.owner == my_id)
        .unwrap_or(0)
}
