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
use crate::weapon::{WeaponKind, WeaponRunner};

/// FNV-1a 64-bit over the match id (deviation ⑤).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// JS `new Actor(…)` L40 + `specialCost()` L129: the equipped special id and
/// the gauge cost come from the weapon table (`WEAPONS[id].special` /
/// `.specialCost`, config.js). The sim stores both per-actor (the tick sample
/// carries the gauge `sp`, so the cost must live on the actor), stamped here
/// at build time from the embedded tuning. Unknown / retired weapon ids fall
/// back to the Spritzer, mirroring [`WeaponKind::from_kind_str`]'s rule in
/// [`build_net_match`].
fn weapon_special(weapon: &str) -> (&'static str, f32) {
    // D-T35-2 pattern: a `OnceLock` over the embedded artifact keeps the
    // table `'static` without plumbing a `&WeaponSet` through the builder.
    static WS: std::sync::OnceLock<crate::tuning::WeaponSet> = std::sync::OnceLock::new();
    let w = WS.get_or_init(|| crate::embedded_tuning().weapons);
    let (id, cost) = match weapon {
        "twins" => (w.twins.special.as_str(), w.twins.special_cost),
        "brolly" => (w.brolly.special.as_str(), w.brolly.special_cost),
        "blaster" => (w.blaster.special.as_str(), w.blaster.special_cost),
        "spinner" => (w.spinner.special.as_str(), w.spinner.special_cost),
        "charger" => (w.charger.special.as_str(), w.charger.special_cost),
        "bow" => (w.bow.special.as_str(), w.bow.special_cost),
        "roller" => (w.roller.special.as_str(), w.roller.special_cost),
        "brush" => (w.brush.special.as_str(), w.brush.special_cost),
        "blade" => (w.blade.special.as_str(), w.blade.special_cost),
        "mitts" => (w.mitts.special.as_str(), w.mitts.special_cost),
        "bucket" => (w.bucket.special.as_str(), w.bucket.special_cost),
        // `shooter` and every unknown id (the Spritzer fallback).
        _ => (w.shooter.special.as_str(), w.shooter.special_cost),
    };
    (id, cost as f32)
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
            // JS actor.js L40: a roster `special` names the id when it is a
            // real `SPECIALS` key, else the weapon default; L129: the gauge
            // cost is the weapon's `specialCost`.
            let (wsp, wcost) = weapon_special(&r.weapon);
            let id = r
                .special
                .as_deref()
                .filter(|s| crate::special::def_by_id(s).is_some())
                .unwrap_or(wsp);
            a.set_special(id);
            a.special_cost = wcost;
            a
        })
        .collect();
    m.runners = (0..m.actors.len()).map(|_| WeaponRunner::new()).collect();
    // Spec D-2: the per-actor loadout comes from the roster weapon string
    // (JS `new Actor({weapon})` → `WEAPONS[weapon].kind`); unknown / retired
    // ids fall back to the Spritzer (spec Assumptions).
    m.kinds = cfg
        .roster
        .iter()
        .map(|r| WeaponKind::from_kind_str(&r.weapon).unwrap_or(WeaponKind::Shooter))
        .collect();
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
