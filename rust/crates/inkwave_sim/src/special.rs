//! SpecialSystem — the 19 specials' lifecycle (Task 35 batch 1: core layer).
//!
//! Faithful port of `src/game/specials.js`: the `SpecialSystem` lifecycle
//! methods (`start` L254-263, `netGhost` L267-276, `_startGhost` L277-285,
//! `_ghostTick` L287-298, `_spawn` L300-305, `_ghostObj` L306-322,
//! `netHurtObj` L324-327, `tick` L336-342, `body` L344-355, `move` L357,
//! `weapon` L359-362, `end` L363-376, `onSplat` L377-380, `remaining`
//! L382-390, `filterDamage` L412-429, `_knock` L430-438, `giveShield`
//! L486-495, `specialNetState` L2124-2130, `specialNetApply` L2131-2136)
//! and the `KIT_GHOSTS.sp` channel (L2138).
//!
//! Batch-1 scope: the lifecycle skeleton, the gauge charge/keep rules, the
//! net-ghost dispatch, the world-object registry and `remaining` /
//! `filterDamage` / net-state packing. Batch-2 (the aura family) landed the
//! `slam` impact ([`slam_impact`], actor.js L900-924), the storm cloud
//! ( [`world`] + `weapon::step_bombs`), the bubbler shield
//! ([`give_shield`] / [`update_shields`], specials.js L486-562), the sonar
//! reveal (`special::sonar`) and the kraken trample (`special::kraken`).
//! Batch 3 (the weapon-trigger family) landed the strike missile (aim /
//! bot target / launch), the zooka twister volleys, the wail speaker
//! placement, the blower bubble ladder, the booyah orb throw + cheers, and
//! the barrage volley sub path — with the ghost world objects (`mi` / `tw`
//! / `sk` / `bu` / `or`) stepping through the same mechanics. Batch 4 (the
//! body / transform family) landed the jetpack hover, the Mega Stamp swing
//! / flip / throw + guard sweep, the crab rig (gatling / cannon / roll),
//! the zipcaster tether, the `ts` / `sh` ghost world objects and the
//! `ReturnMarker` (`jumpBack`) routing. `slam` / `storm` run their phase
//! machines in [`crate::actor`] (JS `_updateSpecial`, actor.js L861-898).
//!
//! Tuning access (D-T35-2): the lifecycle functions read the embedded
//! `SPECIALS` / `SPECIAL_ORDER` artifact through [`specials`] / [`order`]
//! (a `OnceLock` over [`crate::embedded_tuning`]) instead of a plumbed
//! `&SpecialSet` — every existing signature (`Actor::step`,
//! `Match::step_masked`, `Actor::damage`, `Replicator::update`) keeps its
//! shape, and the values still come from the compile-time tuning artifact,
//! never hand-copied.

pub mod barrage;
pub mod blower;
pub mod booyah;
pub mod bubbler;
pub mod crab;
pub mod jetpack;
pub mod kraken;
pub mod sonar;
pub mod stamp;
pub mod strike;
pub mod wail;
pub mod world;
pub mod zipcaster;
pub mod zooka;

use std::sync::OnceLock;

use glam::{Vec3, vec3};

use crate::actor::{Actor, ActorInput, FireGate, SpecialActive, angle_diff};
use crate::collision::CollisionWorld;
use crate::net::KitVal;
use crate::paint::{PaintGrid, SplatOpts};
use crate::sub::SubSystem;
use crate::tuning::{PlayerTuning, SpecialSet};
use crate::weapon::{ProjectileSim, Rng, SimEvent, actor_pos_by_slot, apply_hit};

/// JS `lerp` (ctx.js).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Embedded `SPECIALS` table (config.js L411-468 via the tuning artifact).
#[must_use]
pub fn specials() -> &'static SpecialSet {
    static SP: OnceLock<SpecialSet> = OnceLock::new();
    SP.get_or_init(|| crate::embedded_tuning().specials)
}

/// Embedded `SPECIAL_ORDER` (config.js L469) — the 19 ids, wire-indexed.
#[must_use]
pub fn order() -> &'static [String] {
    static ORD: OnceLock<Vec<String>> = OnceLock::new();
    ORD.get_or_init(|| crate::embedded_tuning().special_order)
}

/// JS `SPECIAL_ORDER.indexOf(id)` — `-1` for unknown ids (L258/L269).
#[must_use]
pub fn order_index(id: &str) -> i32 {
    order()
        .iter()
        .position(|s| s == id)
        .map_or(-1, |i| i as i32)
}

/// JS `SPECIALS[id]` by id string (the table is keyed by id; the five
/// `barrage_*` variants share the [`Barrage`](crate::tuning::Barrage) shape).
#[must_use]
pub fn def_by_id(id: &str) -> Option<SpecialEntry<'static>> {
    let sp = specials();
    let e = match id {
        "slam" => SpecialEntry::Slam(&sp.slam),
        "storm" => SpecialEntry::Storm(&sp.storm),
        "barrage" => SpecialEntry::Barrage(&sp.barrage),
        "barrage_sticky" => SpecialEntry::Barrage(&sp.barrage_sticky),
        "barrage_burst" => SpecialEntry::Barrage(&sp.barrage_burst),
        "barrage_seeker" => SpecialEntry::Barrage(&sp.barrage_seeker),
        "barrage_mist" => SpecialEntry::Barrage(&sp.barrage_mist),
        "bubbler" => SpecialEntry::Bubbler(&sp.bubbler),
        "sonar" => SpecialEntry::Sonar(&sp.sonar),
        "strike" => SpecialEntry::Strike(&sp.strike),
        "zooka" => SpecialEntry::Zooka(&sp.zooka),
        "wail" => SpecialEntry::Wail(&sp.wail),
        "kraken" => SpecialEntry::Kraken(&sp.kraken),
        "blower" => SpecialEntry::Blower(&sp.blower),
        "jetpack" => SpecialEntry::Jetpack(&sp.jetpack),
        "stamp" => SpecialEntry::Stamp(&sp.stamp),
        "booyah" => SpecialEntry::Booyah(&sp.booyah),
        "zipcaster" => SpecialEntry::Zipcaster(&sp.zipcaster),
        "crab" => SpecialEntry::Crab(&sp.crab),
        _ => return None,
    };
    Some(e)
}

/// Borrowed view over the 19 special defs (the JS `SPECIALS[id]` lookup).
#[derive(Debug, Clone, Copy)]
pub enum SpecialEntry<'a> {
    Slam(&'a crate::tuning::Slam),
    Storm(&'a crate::tuning::Storm),
    Barrage(&'a crate::tuning::Barrage),
    Bubbler(&'a crate::tuning::Bubbler),
    Sonar(&'a crate::tuning::Sonar),
    Strike(&'a crate::tuning::Strike),
    Zooka(&'a crate::tuning::Zooka),
    Wail(&'a crate::tuning::Wail),
    Kraken(&'a crate::tuning::Kraken),
    Blower(&'a crate::tuning::Blower),
    Jetpack(&'a crate::tuning::Jetpack),
    Stamp(&'a crate::tuning::Stamp),
    Booyah(&'a crate::tuning::Booyah),
    Zipcaster(&'a crate::tuning::Zipcaster),
    Crab(&'a crate::tuning::Crab),
}

impl SpecialEntry<'_> {
    /// JS `def.duration || 0` (specials.js L256). `slam` / `storm` run their
    /// own phase machines (actor.js L861-898) and JS gives them no
    /// `duration`; `wail` / `booyah` have no duration field either (their
    /// `IMPL.start` sets `dur` explicitly — batch 3).
    #[must_use]
    pub fn duration(&self) -> f32 {
        match self {
            Self::Slam(_) | Self::Storm(_) | Self::Wail(_) | Self::Booyah(_) => 0.0,
            Self::Barrage(d) => d.duration,
            Self::Bubbler(d) => d.duration,
            Self::Sonar(d) => d.duration,
            Self::Zooka(d) => d.duration,
            Self::Kraken(d) => d.duration,
            Self::Blower(d) => d.duration,
            Self::Jetpack(d) => d.duration,
            Self::Stamp(d) => d.duration,
            Self::Zipcaster(d) => d.duration,
            Self::Crab(d) => d.duration,
            Self::Strike(d) => d.duration,
        }
    }

    /// JS `def.kind || id` (specials.js L255) — the dispatch key.
    #[must_use]
    pub fn kind(&self, id: &str) -> String {
        match self {
            Self::Barrage(d) => d.kind.clone(),
            _ => id.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// per-kind state (the JS free-form `s` object's kind-specific fields)
// ---------------------------------------------------------------------------

/// The kind-specific half of a [`SpecialActive`] (JS keeps these as loose
/// fields on the `s` object — `s.cd`, `s.hull`, `s.target`, …). One variant
/// per kind; the [`crate::actor::SpecialActive`] holds the shared lifecycle
/// fields. `None` = the net-`placeholder` active (netmatch.js L381) and the
/// not-yet-dispatched default.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum KindState {
    #[default]
    None,
    /// `slam` (actor.js L850): the jump apex reference (`startY`).
    Slam(SlamState),
    /// `storm` (actor.js L855): no extra fields (the phase machine uses
    /// `s.t` alone).
    Storm(StormState),
    Barrage(barrage::State),
    Bubbler(bubbler::State),
    Sonar(sonar::State),
    Strike(strike::State),
    Zooka(zooka::State),
    Wail(wail::State),
    Kraken(kraken::State),
    Blower(blower::State),
    Jetpack(jetpack::State),
    Stamp(stamp::State),
    Booyah(booyah::State),
    Zipcaster(zipcaster::State),
    Crab(crab::State),
}

/// JS slam active extras (actor.js L850: `{phase, armor, body, startY}` —
/// the phase itself lives on the active as [`SpecialPhase`]).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct SlamState {
    /// JS `s.startY` (actor.js L850): the feet height at the leap's start.
    pub start_y: f32,
}

/// JS storm active extras (actor.js L855): none beyond the phase.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct StormState {}

// ---------------------------------------------------------------------------
// SpecialSystem (JS `class SpecialSystem`, specials.js L565+)
// ---------------------------------------------------------------------------

/// The specials singleton: the world-object list + the gid sequence
/// (JS `this.world`, `netId` via `registry.js` L65 — D-T34-5 gid scheme).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SpecialSystem {
    /// JS `this.world` (L566): missiles, bubbles, storm clouds, …
    pub world: Vec<world::WorldObject>,
    /// JS `registry.js` `_seq` (L65): the per-system gid counter. The wire
    /// gid is `owner_nid * 100_000 + (++seq % 100_000)`; the sim-side owner
    /// form is `(team, slot)`-keyed and rewritten to the nid form by the
    /// replication layer (D-T34-5).
    pub gid_seq: u32,
    /// JS `G.nav.nodes` (specials.js L1314): the walkable sample points the
    /// strike bot's paint-based target prefers over raw bounds-box samples.
    /// The sim stages carry no nav mesh — empty (the default) takes the
    /// `lerp(min+3, max-3, rand)` sampler exactly like JS's `!nodes` branch.
    #[serde(default)]
    pub nav_nodes: Vec<Vec3>,
}

impl SpecialSystem {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// JS `netId(a)` (registry.js L65) with the sim-side owner slot:
    /// `owner_slot * 100_000 + (++seq % 100_000)` (D-T34-5).
    pub fn next_gid(&mut self, owner_slot: usize) -> u32 {
        self.gid_seq = (self.gid_seq + 1) % 100_000;
        (owner_slot as u32) * 100_000 + self.gid_seq
    }

    /// JS `world.find(x => x.ghost && x.gid === d[1] && !x.dead)` (L274).
    #[must_use]
    pub fn find_ghost(&self, gid: u32) -> Option<usize> {
        self.world
            .iter()
            .position(|w| w.ghost && w.gid == gid && !w.dead)
    }
}

// ---------------------------------------------------------------------------
// lifecycle (JS `G.specials.*`, called from the actor / match frame order)
// ---------------------------------------------------------------------------

/// JS `SpecialSystem.start(a, id)` (specials.js L254-263): build the active,
/// record `[0, SPECIAL_ORDER.indexOf(id)]`, dispatch `IMPL[kind].start`.
/// Called from [`Actor::start_special`] (actor.js L837-859) for every id
/// except `slam` / `storm`, which build their own active in the actor.
pub fn start(a: &mut Actor, id: &str) {
    let Some(def) = def_by_id(id) else {
        return;
    };
    let kind = def.kind(id);
    let dur = def.duration();
    let mut s = SpecialActive::new(id, &kind, dur);
    // JS L258: `rec(a, [0, SPECIAL_ORDER.indexOf(id)])`.
    a.sp_records
        .push(vec![KitVal::Num(0.0), KitVal::Num(order_index(id) as f32)]);
    // IMPL[kind].start dispatch (specials.js L1247+). Batch-1 stubs set the
    // behaviour flags JS sets on `s`; mechanics are batch 3/4.
    match kind.as_str() {
        "barrage" => barrage::start(a, &mut s, id),
        "bubbler" => bubbler::start(a, &mut s),
        "sonar" => sonar::start(a, &mut s),
        "strike" => strike::start(a, &mut s),
        "zooka" => zooka::start(a, &mut s),
        "wail" => wail::start(a, &mut s),
        "kraken" => kraken::start(a, &mut s),
        "blower" => blower::start(a, &mut s),
        "jetpack" => jetpack::start(a, &mut s),
        "stamp" => stamp::start(a, &mut s),
        "booyah" => booyah::start(a, &mut s),
        "zipcaster" => zipcaster::start(a, &mut s),
        "crab" => crab::start(a, &mut s),
        _ => {}
    }
    a.special_active = Some(Box::new(s));
}

/// JS `SpecialSystem.tick(a, s, dt)` (L336-342): advance the timer, decay
/// `firing`, run `IMPL.tick`, end on the duration cap. The body kinds run
/// [`body`] instead (JS L275 early-return). `inp` is the frame's actor
/// input — JS's `IMPL.zipcaster.tick` reads `a.intent` for the hang
/// release (L1882-1888).
pub fn tick(a: &mut Actor, dt: f32, inp: &ActorInput) {
    let Some(mut s) = a.special_active.take() else {
        return;
    };
    if s.ghost || s.body || s.ended {
        a.special_active = Some(s);
        return;
    }
    tick_inner(&mut s, dt);
    let reason = dispatch_tick(a, &mut s, dt, inp);
    a.special_active = Some(s);
    if let Some(reason) = reason {
        // JS L2012: `IMPL.crab.tick` ends the special on the hull break.
        end(a, reason);
        return;
    }
    // JS L341: `if (a.specialActive === s && s.dur && s.t >= s.dur) end(a, 'time')`.
    let expire = a
        .special_active
        .as_ref()
        .is_some_and(|s| s.dur > 0.0 && s.t >= s.dur);
    if expire {
        end(a, "time");
    }
}

/// JS `tick` L337-338: `s.t += dt; if (s.firing != null) s.firing = max(0, s.firing - dt)`.
fn tick_inner(s: &mut SpecialActive, dt: f32) {
    s.t += dt;
    s.firing = (s.firing - dt).max(0.0);
}

/// JS `tick` L339: the `IMPL[kind].tick` dispatch. Returns an end reason
/// when the kind's own tick decides the special is over (the crab hull
/// break, JS L2012 `s.hp <= 0 → this.end(a, 'broken')`). The stamp's tick
/// (flip windows + guard sweep) needs the projectile stream and the sub
/// devices, so the match runs [`stamp::tick_full`] right before [`tick`].
fn dispatch_tick(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    inp: &ActorInput,
) -> Option<&'static str> {
    match s.kind.as_str() {
        "barrage" => barrage::tick(s, dt),
        "bubbler" => bubbler::tick(s, dt),
        "sonar" => sonar::tick(s, dt),
        "strike" => strike::tick(s, dt),
        "zooka" => zooka::tick(s, dt),
        "wail" => wail::tick(s, dt),
        // JS `IMPL.kraken.tick` (L1436-1463) needs the roster, the paint
        // grid and the hit router — the match runs it with that context
        // right before this call (the mechanics never read `s.t`, so the
        // one-frame timer offset of running before `tick_inner` is
        // unobservable).
        "blower" => blower::tick(s, dt),
        "jetpack" => jetpack::tick(s, dt),
        "booyah" => booyah::tick(s, dt),
        // JS `IMPL.zipcaster.tick` (L1864-1903): the hang release ladder.
        "zipcaster" => zipcaster::tick(a, s, dt, inp),
        "crab" => {
            crab::tick(a, s, dt);
            if s.crab_hp() <= 0.0 {
                return Some("broken");
            }
        }
        _ => {}
    }
    None
}

/// JS `SpecialSystem.body(a, s, dt)` (L344-355): the body-owned kinds —
/// timer + IMPL.body. The sea-death check (JS L350-353) runs in the actor
/// body branch (it needs the collision probe + tuning). `slam` / `storm`
/// are handled in [`Actor::update_special`] (actor.js L861-898) and never
/// reach here. `world` / `t` / `input` drive the body kinds' own physics
/// (JS `_stand` L643-649; the jetpack / zipcaster ladders read `a.intent`).
pub fn body(
    a: &mut Actor,
    dt: f32,
    world: &crate::actor::SimWorld,
    t: &PlayerTuning,
    input: &ActorInput,
) {
    let Some(mut s) = a.special_active.take() else {
        return;
    };
    if s.ended {
        a.special_active = Some(s);
        return;
    }
    tick_inner(&mut s, dt);
    let kind = s.kind.clone();
    // The batch-4 body kinds need `a` and `s` disjointly (they drive the
    // actor's own integration), so they run before the box goes back.
    match kind.as_str() {
        // JS `IMPL.jetpack.body` (L1530-1591): the hover / boost ladder
        // (the fire / paint moments ride actor flags).
        "jetpack" => jetpack::body(a, &mut s, dt, world, t, input),
        // JS `IMPL.zipcaster.body` (L1811-1845): the tether flight.
        "zipcaster" => zipcaster::body(a, &mut s, dt, world, t, input),
        _ => {}
    }
    a.special_active = Some(s);
    match kind.as_str() {
        // JS L1276: `IMPL.sonar.body` → `this._stand(a, dt)`.
        "sonar" => a.stand(dt, world, t),
        // JS L1288-1293: `IMPL.strike.body` — `_stand` first (L1289), then
        // the aim ladder. The launch itself needs the specials registry +
        // collision (only the match holds them), so `body` raises the
        // `launch` flag and the match runs [`strike::launch`] the same frame
        // through [`crate::actor::Actor::peek_strike_launch`].
        "strike" => {
            a.stand(dt, world, t);
            if let Some(s) = a.special_active.as_mut() {
                strike::body(s, dt);
            }
        }
        _ => {}
    }
    let expire = a
        .special_active
        .as_ref()
        .is_some_and(|s| s.dur > 0.0 && s.t >= s.dur);
    if expire {
        end(a, "time");
    }
}

/// JS `SpecialSystem.move(a, dt)` (L357): the pre-`_horizontal` input hook.
/// JS gives `kraken` no `move` (L1410-1469: start / weapon / tick / end
/// only) and the jetpack steers inside its `body`; the one `move`
/// implementation is `IMPL.stamp.move` (the body-yaw slew + strafe filter,
/// L1605-1618). `intent` is the actor's fresh per-frame copy of the input
/// (JS's `mvIn` / `mvOut` compound guard, L1611-1613, is unnecessary).
pub fn move_hook(a: &mut Actor, dt: f32, intent: &mut ActorInput) {
    let Some(mut s) = a.special_active.take() else {
        return;
    };
    if s.ghost || s.ended || s.kind != "stamp" {
        a.special_active = Some(s);
        return;
    }
    stamp::move_hook(a, &mut s, dt, intent);
    a.special_active = Some(s);
}

/// JS `SpecialSystem.weapon(a, dt)` (L359-362): `true` skips the weapon
/// runner this frame (actor.js L373). The trigger-taking kinds: kraken
/// (jump-attack, L1423), zooka (twister volleys, L1332), wail (speaker
/// placement, L1367), blower (bubble inflation, L1474), booyah (orb throw,
/// L1727), stamp (swing / flip / throw, L1619), crab (gatling / cannon,
/// L1944) and jetpack (hover fire, L1568). The `zipcaster` arm runs its
/// tether ladder but returns `false` (JS L1786-1792 returns nothing — the
/// main weapon keeps working; the arm only swallows the sub trigger on the
/// gate). The kind arms report an end reason when they call `this.end`
/// inline (JS `IMPL.wail.place` L1398 / `IMPL.booyah.throwIt` L1761 /
/// `IMPL.stamp.weapon` throw L1630) — the active is taken out here, so the
/// end runs after it goes back.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn weapon_hook(
    a: &mut Actor,
    dt: f32,
    sys: &mut SpecialSystem,
    collision: &CollisionWorld,
    rng: &mut Rng,
    paint: &mut PaintGrid,
    ps: &mut ProjectileSim,
    t: &PlayerTuning,
    inp: &ActorInput,
    gate: &mut FireGate,
) -> bool {
    let takes_trigger = a.special_active.as_ref().is_some_and(|s| {
        !s.ghost
            && !s.ended
            && matches!(
                s.kind.as_str(),
                "kraken"
                    | "zooka"
                    | "wail"
                    | "blower"
                    | "booyah"
                    | "stamp"
                    | "crab"
                    | "jetpack"
                    | "zipcaster"
            )
    });
    if !takes_trigger {
        return false;
    }
    // JS passes `a` and `s` together to `IMPL[kind].weapon`; the sim takes
    // the `Box` out so both halves are borrowed disjointly.
    let Some(mut s) = a.special_active.take() else {
        return false;
    };
    if s.body {
        // JS actor.js L276: a body-owned frame returns *before* the weapon
        // call — neither the kind's `weapon` ladder nor the runner runs (the
        // jetpack's whole life, the zipcaster's tether flight).
        a.special_active = Some(s);
        return true;
    }
    let (r, end_reason) = match s.kind.as_str() {
        "kraken" => (kraken::weapon(a, &mut s, dt), None),
        "zooka" => zooka::weapon(a, &mut s, sys, dt),
        "wail" => wail::weapon(a, &mut s, sys, collision, dt),
        "blower" => blower::weapon(a, &mut s, sys, rng, dt),
        "stamp" => stamp::weapon(a, &mut s, dt, gate, sys),
        "crab" => (crab::weapon(a, &mut s, dt, inp, gate, sys, rng, ps), None),
        // JS L1568-1580: the jetpack's fire lives in its `body` (the raised
        // `sp_jet_fire` flag). The jetpack is always body-owned, so the body
        // short-circuit above already skips the runner (JS actor.js L276);
        // this arm is defensive.
        "jetpack" => (true, None),
        "zipcaster" => {
            zipcaster::weapon(a, &mut s, dt, gate, collision, paint, rng, t);
            (false, None)
        }
        _ => booyah::weapon(a, &mut s, sys, dt),
    };
    a.special_active = Some(s);
    if let Some(reason) = end_reason {
        end(a, reason);
    }
    r
}

/// JS `SpecialSystem.end(a, reason)` (L363-376): the lifecycle end —
/// record `[1, reason]` (live actors only), dispatch `IMPL.end`, clear the
/// active, and route the `jumpBack` marker (JS L371-375: the owner's home
/// super jump / the ghost's `returning` decision rides
/// [`Actor::sp_marker_action`] for [`drain_marker_action`]).
pub fn end(a: &mut Actor, reason: &str) {
    // JS L364: `if (!s || s.ended || s.id === 'slam' || s.id === 'storm') return`.
    let eligible = a
        .special_active
        .as_ref()
        .is_some_and(|s| !s.ended && s.id != "slam" && s.id != "storm");
    if !eligible {
        return;
    }
    let ghost = a.special_active.as_ref().is_some_and(|s| s.ghost);
    if !ghost {
        // JS L367: `rec(a, [1, reason])`.
        a.sp_records
            .push(vec![KitVal::Num(1.0), KitVal::Str(reason.to_string())]);
    }
    let kind = a
        .special_active
        .as_ref()
        .map(|s| s.kind.clone())
        .unwrap_or_default();
    // JS L373-375 reads `s.jumpBack` / `s.origin` after the active is
    // cleared — capture them here (the origin lives in the kind state).
    let (jump_back, origin) = a
        .special_active
        .as_ref()
        .map(|s| {
            let origin = match &s.state {
                KindState::Jetpack(st) => Some(st.origin),
                KindState::Zipcaster(st) => Some(st.origin),
                _ => None,
            };
            (s.jump_back, origin)
        })
        .unwrap_or((false, None));
    if let Some(s) = a.special_active.as_mut() {
        s.ended = true;
    }
    // The stubs reach their own state through `a.special_active` (JS passes
    // `s` alongside `a`; Rust cannot hold both borrows).
    match kind.as_str() {
        "barrage" => barrage::end(a, reason),
        "bubbler" => bubbler::end(a, reason),
        "sonar" => sonar::end(a, reason),
        "strike" => strike::end(a, reason),
        "zooka" => zooka::end(a, reason),
        "wail" => wail::end(a, reason),
        "kraken" => kraken::end(a, reason),
        "blower" => blower::end(a, reason),
        "jetpack" => jetpack::end(a, reason),
        "stamp" => stamp::end(a, reason),
        "booyah" => booyah::end(a, reason),
        "zipcaster" => zipcaster::end(a, reason),
        "crab" => crab::end(a, reason),
        _ => {}
    }
    // JS L369-370: `if (a.specialActive === s) a.specialActive = null`.
    a.special_active = None;
    // JS L371-375: the `jumpBack` marker routing. Deviation: a mid-
    // super-jump owner splat never finishes its marker (JS L710 listens on
    // the live flight; the sim only wires the `land` / end listeners).
    let mut returning = false;
    if ghost {
        // JS L373: a ghost's marker only turns back on a clean `time` end.
        returning = jump_back && reason == "time" && a.alive;
    } else if jump_back && reason != "splat" && a.alive {
        const MARKER_HOME_DIST: f32 = 2.5; // JS L374 (D-T35-4)
        if let Some(o) = origin {
            // JS L374: the home super jump (`instant + home`); `false` =
            // no flight started (already flying).
            if a.pos.distance(o) > MARKER_HOME_DIST && a.super_jump(o) {
                returning = true;
            }
        }
    }
    a.sp_marker_action = Some(returning);
}

/// JS `SpecialSystem.onSplat(a)` (L377-380): `end(a, 'splat')` (the `end`
/// guard skips slam/storm, whose actives the caller nulls) + the shield
/// drop (`_dropShield(a, false)` → `status.shield = 0`). The gauge-keep
/// formula runs in [`Actor::splat`] (actor.js L216-219) *before* this call.
pub fn on_splat(a: &mut Actor) {
    end(a, "splat");
    if a.status.shield > 0.0 {
        // JS L379: `if (a.status.shield > 0) this._dropShield(a, false)` —
        // the pop=false branch just clears the shield (no burst FX).
        a.status.shield = 0.0;
    }
}

/// JS `SpecialSystem.remaining(a)` (L382-390): the HUD fraction left.
/// JS L384: no active → `0`.
#[must_use]
pub fn remaining(a: &Actor) -> f32 {
    let Some(s) = a.special_active.as_ref() else {
        return 0.0; // JS L384: `if (!s) return 0`
    };
    let def_dur = s.dur;
    match s.kind.as_str() {
        // JS L383: booyah — `thrown ? 0 : 1`.
        "booyah" => {
            if s.booyah_thrown() {
                0.0
            } else {
                1.0
            }
        }
        // JS L384: blower — `max(0, 1 - count/def.max) * (dur ? max(0, 1-t/dur) : 1)`.
        "blower" => {
            let max = specials().blower.max as f32;
            let f = (1.0 - s.blower_count() / max).max(0.0);
            if def_dur > 0.0 {
                f * (1.0 - s.t / def_dur).max(0.0)
            } else {
                f
            }
        }
        // JS L385: crab — `min(max(0, 1-t/dur), max(0, hp/def.hp))`.
        "crab" => {
            let t = if def_dur > 0.0 {
                (1.0 - s.t / def_dur).max(0.0)
            } else {
                1.0
            };
            let hp = s.crab_hp().max(0.0) / specials().crab.hp as f32;
            t.min(hp.max(0.0))
        }
        // JS L386-388: `dur ? max(0, 1 - t/dur) : 1`.
        _ => {
            if def_dur > 0.0 {
                (1.0 - s.t / def_dur).max(0.0)
            } else {
                1.0
            }
        }
    }
}

/// JS `SpecialSystem.filterDamage(v, amount, attacker, source)` (L412-429):
/// the special-side damage gate. `Some(v)` replaces the amount (0 = fully
/// absorbed), `None` = no filtering (JS's `return amount`). The gates match
/// by `s.id` (L413-417), with no ghost exclusion — a ghost never takes
/// damage anyway (its owner decides hits).
///
/// `attacker` is the JS attacker actor's identity: `Some(pos)` only counts
/// when it is *not* the victim (JS `attacker && attacker !== v`, L417/433),
/// so self-inflicted damage knocks/deflects as if from nowhere.
#[must_use]
pub fn filter_damage(
    a: &mut Actor,
    amount: f32,
    attacker: Option<(usize, usize)>,
    attacker_pos: Option<glam::Vec3>,
) -> Option<f32> {
    // JS L417/L433: `attacker !== v` — a self hit carries no direction.
    let from_other = attacker.is_some_and(|id| id != (a.team, a.slot));
    let ap = if from_other { attacker_pos } else { None };
    // JS L412: the active-special gates first, then the shield (L421).
    let id = a.special_active.as_ref().map(|s| s.id.clone());
    match id.as_deref() {
        // JS L414: kraken — knock the blow aside, flash, 0 dmg. The `4`
        // knock cap is hardcoded in JS (D-T35-4).
        Some("kraken") => {
            // JS L414's `_hitFlash` poke + thud sound are presentation.
            const KRAKEN_KNOCK_CAP: f32 = 4.0; // JS L414 `Math.min(4, …)`
            let d = &specials().kraken;
            knock(a, (amount * d.knock_per_damage).min(KRAKEN_KNOCK_CAP), ap);
            Some(0.0)
        }
        // JS L415: crab — `IMPL.crab.hurt.call(this, v, s, amount, attacker)`.
        Some("crab") => Some(crab::hurt(a, amount, ap)),
        // JS L417-420: stamp — mid-swing, anything from the front arc is
        // deflected; otherwise fall through to the shield check (L421).
        Some("stamp") => {
            let deflect = a
                .special_active
                .as_ref()
                .is_some_and(|s| s.stamp_guard() > 0.0)
                && ap.is_some_and(|p| stamp_front(a, p, specials().stamp.deflect_arc));
            if deflect {
                // JS L418: the deflect spark/FX are presentation.
                return Some(0.0);
            }
            shield_gate(a, amount, ap)
        }
        _ => shield_gate(a, amount, ap),
    }
}

/// JS L421-427: the bubbler shield absorbs with a knock; `None` lets the
/// amount through (JS `return amount`, L428).
fn shield_gate(a: &mut Actor, amount: f32, attacker_pos: Option<glam::Vec3>) -> Option<f32> {
    if a.status.shield > 0.0 {
        let d = &specials().bubbler;
        knock(
            a,
            (amount * d.knock_per_damage).min(d.knock_max),
            attacker_pos,
        );
        // JS L424-425: the shield-hit mesh flash + sound are presentation.
        return Some(0.0);
    }
    None
}

/// JS `stampFront(a, s, p, arcDeg)` (specials.js L57-61): the attacker sits
/// inside the guard arc in front of the stamp's body yaw (`s.bodyYaw`,
/// mirrored on the active as `face_yaw` — [`stamp::move_hook`] keeps them
/// equal). JS L59: a co-located attacker (`l < 0.3`) counts as in front.
pub(crate) fn stamp_front(a: &Actor, attacker_pos: glam::Vec3, arc_deg: f32) -> bool {
    let Some(yaw) = a.special_active.as_ref().and_then(|s| s.face_yaw) else {
        return false;
    };
    let dx = attacker_pos.x - a.pos.x;
    let dz = attacker_pos.z - a.pos.z;
    let l = (dx * dx + dz * dz).sqrt();
    if l < 0.3 {
        return true; // JS L59: `if (l < 0.3) return true`
    }
    (dx * yaw.sin() + dz * yaw.cos()) / l >= (arc_deg * (std::f32::consts::PI / 180.0)).cos()
}

/// JS `SpecialSystem._knock(v, attacker, k)` (L430-438): the absorb-and-shove.
fn knock(a: &mut Actor, k: f32, attacker_pos: Option<glam::Vec3>) {
    if k <= 0.0 || k.is_nan() {
        return; // JS L431: `if (!(k > 0)) return`
    }
    // JS L433: direction from the attacker (already identity-filtered by
    // the caller), else away from the facing.
    let (dx, dz) = if let Some(ap) = attacker_pos {
        (a.pos.x - ap.x, a.pos.z - ap.z)
    } else {
        (0.0, 0.0)
    };
    // JS L434-435: `l = Math.hypot(dx, dz); if (l < 1e-3)` use the facing.
    let (nx, nz) = if dx.hypot(dz) < 1e-3 {
        (-a.yaw.sin(), -a.yaw.cos())
    } else {
        let l = dx.hypot(dz);
        (dx / l, dz / l)
    };
    a.vel.x += nx * k;
    a.vel.z += nz * k;
    if a.grounded {
        // JS L436: `vel.y = max(vel.y, 1.4 + k * 0.18); grounded = false`.
        // The `1.4`/`0.18` pair is hardcoded in JS (D-T35-4).
        const KNOCK_LIFT: f32 = 1.4; // JS L436
        const KNOCK_LIFT_K: f32 = 0.18; // JS L436
        a.vel.y = a.vel.y.max(KNOCK_LIFT + k * KNOCK_LIFT_K);
        a.grounded = false;
    }
}

// ---------------------------------------------------------------------------
// "Yeah!" cheers (JS `cheer` L510-531, fired from the `update` head L535)
// ---------------------------------------------------------------------------

/// JS `SpecialSystem.cheer(a)` (L510-531): the one-frame "Yeah!" pulse.
/// Rate-limited per actor (`_cheerT`, L511 — `0` reads as the JS
/// `undefined || -9` "never cheered" default), then every live same-team
/// booyah holder (not yet thrown) tops up by `def.cheer` and flashes the
/// `cheered` visual timer (L517-523). A cheer that *helped* someone tops the
/// cheater's own gauge toward its cost (L525-529). The cheer sound (L513,
/// random pitch) and the `cheers` float-text list (L514) are presentation;
/// the random pitch consumes no sim RNG (recorded deviation). The caller
/// (match) clears `intent.cheer` exactly like JS L535.
pub fn cheer(a_idx: usize, actors: &mut [Actor], time: f32) {
    let (last, alive, team) = {
        let a = &actors[a_idx];
        (
            if a.cheer_t == 0.0 { -9.0 } else { a.cheer_t },
            a.alive,
            a.team,
        )
    };
    if !alive || time - last < 0.4 {
        return; // JS L511
    }
    actors[a_idx].cheer_t = time; // JS L512
    let d = &specials().booyah;
    let mut helped = false;
    for o in actors.iter_mut() {
        // JS L519: `o.team !== a.team || !o.alive || !s || s.id !== 'booyah' || s.thrown`
        if o.team != team || !o.alive {
            continue;
        }
        let Some(s) = o.special_active.as_mut() else {
            continue;
        };
        if s.id != "booyah" || s.booyah_thrown() {
            continue;
        }
        // JS L520-522: `charge = min(1, charge + def.cheer); cheered = 0.35`.
        if let crate::special::KindState::Booyah(st) = &mut s.state {
            st.charge = (st.charge + d.cheer).min(1.0);
            st.cheered = 0.35;
        }
        helped = true;
    }
    // JS L525-529: cheering on a teammate's orb tops up your own special a
    // little — only while you have nothing running (the gauge cap is the
    // actor's own `specialCost`, L131/L2127).
    if helped && actors[a_idx].special_active.is_none() {
        let was = actors[a_idx].special_ready();
        actors[a_idx].special = actors[a_idx]
            .special_cost
            .min(actors[a_idx].special + d.cheer_special);
        if !was && actors[a_idx].special_ready() {
            // JS L528 `emit('special:ready')` — the same edge pattern as
            // `addTurf` (actor.js L140, actor.rs gauge ladder).
            actors[a_idx].push_event(crate::actor::ActorEvent::SpecialReady);
        }
    }
}

// ---------------------------------------------------------------------------
// shields (JS `giveShield` L486-498 + the `update` ladder L542-563)
// ---------------------------------------------------------------------------

/// JS `SpecialSystem.giveShield(a, time, owner)` (L486-488): the shield is a
/// plain timer on `status` — a fresh grant takes the *max* of what is left
/// and stamps the ownership flag (only owner fields share, L556). The mesh
/// creation (L489-495), the `shield_up` sound (L496) and the
/// `special:shield` emit (L497) are presentation (the event is not in the
/// netmatch `FORWARD` list, netmatch.js L47).
pub fn give_shield(a: &mut Actor, time: f32, owner: bool) {
    a.status.shield = a.status.shield.max(time);
    a.status.shield_owner = owner;
}

/// JS `SpecialSystem.update`'s shield ladder (L542-563), run by the match
/// after the sub step (main.js L1119: `specials.update` is the last system
/// tick). Iterates the *whole* roster including proxies — JS does the same
/// (`G.actors`), so a teammate's field shares onto this screen's copies of
/// remote players too; their timers simply never decay there (remotes are
/// held frozen, match.js L228).
pub fn update_shields(actors: &mut [Actor], dt: f32) {
    let share = specials().bubbler.share_range;
    for ai in 0..actors.len() {
        // JS L543: no shield → `_dropShield(a, false)` (the mesh removal is
        // presentation; the sim just clamps the timer to 0). `!(s > 0)` ≡
        // `s <= 0` for the non-NaN timers the sim stores.
        if actors[ai].status.shield <= 0.0 {
            actors[ai].status.shield = 0.0;
            continue;
        }
        // JS L544: dead → drop.
        if !actors[ai].alive {
            actors[ai].status.shield = 0.0;
            continue;
        }
        actors[ai].status.shield -= dt; // JS L545
        // JS L546-555: the bubble mesh follow / breathe are presentation.
        // JS L556-561: an *owner* field still running shares to teammates
        // inside `shareRange` (3-D distance), passing the remaining time and
        // `owner = false` (shared shields never re-share).
        let owner_field = actors[ai].status.shield_owner
            && actors[ai]
                .special_active
                .as_ref()
                .is_some_and(|s| s.id == "bubbler");
        if owner_field {
            let (team, time) = (actors[ai].team, actors[ai].status.shield);
            let ap = actors[ai].pos;
            for (oi, o) in actors.iter_mut().enumerate() {
                if oi == ai {
                    continue; // JS L558: `o === a`
                }
                if o.team != team || !o.alive || o.status.shield > 0.0 {
                    continue; // JS L558
                }
                if o.pos.distance(ap) < share {
                    give_shield(o, time, false);
                }
            }
        }
        // JS L562: the field expired → `_dropShield(a, true)` (pop FX are
        // presentation).
        if actors[ai].status.shield <= 0.0 {
            actors[ai].status.shield = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------
// blast / slam impact (JS `blast` L66-78, `Actor._slamImpact` actor.js
// L900-924) — the batch-2 area-damage helpers
// ---------------------------------------------------------------------------

/// JS `blast(owner, team, c, radius, dmgMax, dmgMin, weaponId, killRadius)`
/// (specials.js L66-78): enemies near `c` with line of sight take a
/// quadratic falloff from `dmgMax` (`killRadius` and in) to `dmgMin`; then
/// enemy *devices* in the radius take a flat 60 (`subs.damageArea`, L77 —
/// after the player loop, JS order). The `weaponId` string only rides JS's
/// `hit` event payload; the sim's [`SimEvent::Hit`] carries no weapon, so it
/// is dropped. `G.boss?.splash` (L78) is Boss Battle (out of M3 scope).
#[allow(clippy::too_many_arguments)]
pub(crate) fn blast(
    events: &mut Vec<SimEvent>,
    subs: &mut SubSystem,
    actors: &mut [Actor],
    collision: &CollisionWorld,
    owner_slot: usize,
    team: usize,
    c: Vec3,
    radius: f32,
    dmg_max: f32,
    dmg_min: f32,
    kill_radius: f32,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
    muted: bool,
) {
    const RIM: f32 = 0.3; // JS L70: `radius + 0.3` slack (D-T35-4)
    const LOS_FROM_UP: f32 = 0.35; // JS L71 (D-T35-4)
    const LOS_TARGET_UP: f32 = 0.8; // JS L69 (D-T35-4)
    const DEVICE_DMG: f32 = 60.0; // JS L77 (D-T35-4)
    // JS netmatch.js L711-717 `shouldApplyHit`: `attacker.remote → 'drop'` is
    // checked *before* the victim, so a ghost world object's blast (the
    // `ts` / `sh` / `or` spawns of the `['k','sp']` op-2 ghost route and the
    // ghost bubble `netEvent 'x'` route) never hurts anyone on this screen —
    // the owner's screen already decided the hits. Same gate as `sub.rs`
    // `apply_hit_routed` (JS weapons.js L1549-1551).
    let attacker_remote = crate::match_::find_actor(actors, team, owner_slot)
        .is_some_and(|ai| remote.is_some_and(|m| m.get(ai).copied().unwrap_or(false)));
    let mut hits: Vec<(usize, f32)> = Vec::new();
    if !attacker_remote {
        for (ei, e) in actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue; // JS L68
            }
            let ep = vec3(e.pos.x, e.pos.y + LOS_TARGET_UP, e.pos.z);
            let d = ep.distance(c);
            if d > radius + RIM {
                continue; // JS L70
            }
            if !collision.los(vec3(c.x, c.y + LOS_FROM_UP, c.z), ep) {
                continue; // JS L71
            }
            // JS L72: flat inside `killRadius`, quadratic falloff outside.
            let k = if d <= kill_radius {
                0.0
            } else {
                ((d - kill_radius) / (radius - kill_radius).max(0.01)).clamp(0.0, 1.0)
            };
            hits.push((ei, lerp(dmg_max, dmg_min, k * k)));
        }
    }
    for (ei, dmg) in hits {
        // JS L73 `G.projectiles.applyHit(owner, e, …)` (weapons.js L1543-1556):
        // past the attacker gate above the owner is local here, so the route
        // is victim-side: remote → `HitSend`, local → `apply_hit`.
        let (vs, vt) = (actors[ei].slot, actors[ei].team);
        let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
        if send {
            events.push(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let atk_pos = actor_pos_by_slot(actors, owner_slot);
            let outcome = apply_hit(&mut actors[ei], dmg, Some((team, owner_slot)), atk_pos, t);
            events.push(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }
    // JS L77: `subs.damageArea` runs under the caller's `netMuted()` (the
    // ghost bubble's blast never churns devices either).
    subs.damage_area(c, radius, DEVICE_DMG, team, muted);
}

/// JS `Actor._slamImpact(sp)` (actor.js L900-924): the tidal slam's landing —
/// one main splat at `c + 0.3y` (radius `0.72 · R`) plus nine satellites on a
/// jittered ring, all credited through `addTurfNoSpecial` (special ink never
/// charges the gauge, L910); then every enemy inside `R` with line of sight
/// takes the kill-radius damage ladder (L916-923). The 37 `Math.random()`
/// draws run in JS order: `[main_seed, (angle-jitter, ring-r, width, seed) ×
/// 9]` (L903-908) — the sim consumes them from the match's stream. The
/// `special:slam` bus event (L914) is already queued by
/// [`crate::actor::Actor::update_special`] when the fall phase lands, so this
/// function does not push it again.
#[allow(clippy::too_many_arguments)]
pub fn slam_impact(
    rng: &mut Rng,
    ps: &mut ProjectileSim,
    actors: &mut [Actor],
    idx: usize,
    c: Vec3,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    const PAINT_MAIN_UP: f32 = 0.3; // JS L903 (D-T35-4)
    const PAINT_MAIN_R: f32 = 0.72; // JS L903 (D-T35-4)
    const PAINT_SATS: usize = 9; // JS L904 (D-T35-4)
    const PAINT_JITTER: f32 = 0.3; // JS L905 (D-T35-4)
    const PAINT_RING_LO: f32 = 0.55; // JS L906 (D-T35-4)
    const PAINT_RING_SPAN: f32 = 0.3; // JS L906 (D-T35-4)
    const PAINT_SAT_UP: f32 = 0.6; // JS L907 (D-T35-4)
    const PAINT_SAT_R_LO: f32 = 1.1; // JS L908 (D-T35-4)
    const PAINT_SAT_R_SPAN: f32 = 0.6; // JS L908 (D-T35-4)
    const FALLOFF: f32 = 0.3; // JS L920 (D-T35-4)
    const LOS_UP: f32 = 0.8; // JS L921-922 (D-T35-4)
    let d = &specials().slam;
    let (team, owner_slot) = (actors[idx].team, actors[idx].slot);
    let mut area = 0.0;
    // JS L903: the main splat — one seed draw.
    let seed = rng.next_f32();
    area += paint.splat(
        collision,
        vec3(c.x, c.y + PAINT_MAIN_UP, c.z),
        d.radius * PAINT_MAIN_R,
        team,
        &SplatOpts {
            seed,
            stretch: None,
            stretch_amt: None,
            kind: None,
        },
    );
    // JS L904-909: nine satellites, four draws each (angle jitter, ring
    // radius, splat width, seed).
    for i in 0..PAINT_SATS {
        let ang =
            (i as f32 / PAINT_SATS as f32) * std::f32::consts::TAU + rng.next_f32() * PAINT_JITTER;
        let rr = d.radius * (PAINT_RING_LO + rng.next_f32() * PAINT_RING_SPAN);
        let rad = PAINT_SAT_R_LO + rng.next_f32() * PAINT_SAT_R_SPAN;
        let seed = rng.next_f32();
        area += paint.splat(
            collision,
            vec3(
                c.x + ang.cos() * rr,
                c.y + PAINT_SAT_UP,
                c.z + ang.sin() * rr,
            ),
            rad,
            team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    actors[idx].add_turf_no_special(area); // JS L910
    // JS L911-915: explosion FX, sound, `shake`, rumble are presentation.
    // JS L916-923: the damage ladder (raw `a.pos` distance, LOS from
    // `c + 0.8y` to `a.pos + 0.8y`).
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ai, a) in actors.iter().enumerate() {
        if a.team == team || !a.alive {
            continue; // JS L917
        }
        let dist = a.pos.distance(c);
        if dist > d.radius {
            continue; // JS L919
        }
        // JS L920: full damage inside `killRadius`, then a *linear* tail at
        // 30 % of the range span (not the quadratic blast falloff).
        let dmg = if dist < d.kill_radius {
            d.damage_max
        } else {
            d.damage_min
                + (d.damage_max - d.damage_min)
                    * FALLOFF
                    * (1.0 - (dist - d.kill_radius) / (d.radius - d.kill_radius))
        };
        let ap = vec3(a.pos.x, a.pos.y + LOS_UP, a.pos.z);
        if collision.los(vec3(c.x, c.y + LOS_UP, c.z), ap) {
            hits.push((ai, dmg)); // JS L922
        }
    }
    for (ai, dmg) in hits {
        // JS L922 `G.projectiles.applyHit(this, a, dmg, 'slam')`: the slammer
        // is local (the phase machine only runs on live actors), so the route
        // is victim-side like [`blast`]. The slammer has not moved since the
        // impact, so `c` is its live `pos` for the guard-arc snapshot.
        let (vs, vt) = (actors[ai].slot, actors[ai].team);
        let send = remote.is_some_and(|m| m.get(ai).copied().unwrap_or(false));
        if send {
            ps.push_event(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(&mut actors[ai], dmg, Some((team, owner_slot)), Some(c), t);
            ps.push_event(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// net ghost (JS `netGhost` L267-276, `_startGhost` L277-285, `_ghostObj`
// L306-322, `netHurtObj` L324-327, `_ghostTick` L287-298)
// ---------------------------------------------------------------------------

/// JS `SpecialSystem.netGhost(a, d)` (L267-276) — the `['k', nid, 'sp', d]`
/// consumer (`KIT_GHOSTS.sp.ghost`, L2138). `d[0]` selects the op:
/// `0` start, `1` end, `2` ghost world object, `3` world-object event,
/// `4` kind-specific ghost event. The actor is addressed by roster index
/// (the object ops need the whole registry + context alongside the owner),
/// mirroring [`crate::sub::SubSystem::net_ghost`].
pub fn net_ghost(a_idx: usize, d: &[KitVal], sys: &mut SpecialSystem, ctx: &mut world::WorldCtx) {
    let op = d.first().and_then(KitVal::as_f32).unwrap_or(-1.0) as i32;
    match op {
        // JS L269: `_startGhost(a, SPECIAL_ORDER[d[1]])`.
        0 => {
            let idx = d.get(1).and_then(KitVal::as_f32).unwrap_or(-1.0) as i32;
            if idx < 0 || idx as usize >= order().len() {
                return;
            }
            let id = order()[idx as usize].clone();
            start_ghost(&mut ctx.actors[a_idx], &id, sys);
            // The ghost's `IMPL.start` raised a `ReturnMarker` request
            // (jetpack / zipcaster) — build it ghost-side at the owner's
            // position (JS L282 → the live start's `new ReturnMarker`).
            if let Some(icon) = ctx.actors[a_idx].sp_marker.take() {
                let a = &ctx.actors[a_idx];
                let (slot, team, pos) = (a.slot, a.team, a.pos);
                sys.world
                    .push(world::make_marker(slot, team, &icon, pos, true));
            }
        }
        // JS L270: `if (a.specialActive) end(a, d[1] || 'net')` — ghost
        // actives only (the live owner never replays its own records).
        1 => {
            if ctx.actors[a_idx]
                .special_active
                .as_ref()
                .is_some_and(|s| s.ghost)
            {
                let reason = d
                    .get(1)
                    .and_then(KitVal::as_str)
                    .unwrap_or("net")
                    .to_string();
                end(&mut ctx.actors[a_idx], &reason);
                drain_marker_action(&mut ctx.actors[a_idx], sys);
            }
        }
        // JS L271: `_ghostObj(a, d[1], d[2], d.slice(3))`.
        2 => {
            let kind = d.get(1).and_then(KitVal::as_str).unwrap_or("").to_string();
            let gid = d.get(2).and_then(KitVal::as_f32).unwrap_or(0.0) as u32;
            let rest: Vec<f32> = d[3..].iter().map(|v| v.num_or(0.0)).collect();
            ghost_obj(sys, ctx, a_idx, &kind, gid, &rest);
        }
        // JS L273: world-object netEvent — `world.find(ghost && gid && !dead)`
        // then `w.netEvent?.(d)`. The batch-3 objects: only `Bubble` has a
        // `netEvent` (L1080: `d[2] === 'x' → explode(null)`); the missile /
        // twister / speaker / orb ghosts carry no net events.
        3 => {
            let gid = d.get(1).and_then(KitVal::as_f32).unwrap_or(0.0) as u32;
            let Some(wi) = sys.find_ghost(gid) else {
                return;
            };
            if sys.world[wi].kind == "bu" && d.get(2).and_then(KitVal::as_str) == Some("x") {
                world::bubble_explode(wi, None, &mut sys.world, ctx);
            }
        }
        // JS L274: `if (s.ghost) GHOST[kind].event(a, s, d)`.
        4 if ctx.actors[a_idx]
            .special_active
            .as_ref()
            .is_some_and(|s| s.ghost) =>
        {
            // JS L274: `GHOST[kind].event(a, s, d)` (L2049-2121). The crab
            // ghost carries no event arm (the hull / firing ride `spst`).
            let what = d.get(1).and_then(KitVal::as_str).unwrap_or("").to_string();
            let rest: Vec<f32> = d[2..].iter().map(|v| v.num_or(0.0)).collect();
            let Some(s) = ctx.actors[a_idx].special_active.as_mut() else {
                return;
            };
            match (s.kind.as_str(), what.as_str()) {
                // JS `GHOST.stamp.event` (L2077-2083): the `['4','sl'…]`
                // moment is explosion FX + sound only — nothing sim-side.
                ("stamp", "sl") => {}
                // JS `GHOST.zipcaster.event` (L2090-2103).
                ("zipcaster", "zf") => {
                    // L2092: `s.zip = { anchor: V(d,2) }; s.hang = 0` — the
                    // ghost only shows the tether anchor (no flight).
                    if let KindState::Zipcaster(st) = &mut s.state {
                        st.zip = Some(zipcaster::ZipFlight {
                            anchor: v3at(&rest, 0),
                            ..zipcaster::ZipFlight::default()
                        });
                    }
                    s.hang = 0.0;
                }
                ("zipcaster", "zm") => {} // L2095: the miss line is FX-only
                ("zipcaster", "zi") => {
                    // L2096-2098: `s.zip = null; s.hang = d[5] ? def.hang : 0`
                    // (`rest[3]` is the record's wall flag).
                    if let KindState::Zipcaster(st) = &mut s.state {
                        st.zip = None;
                    }
                    s.hang = if rest.get(3).copied().unwrap_or(0.0) != 0.0 {
                        specials().zipcaster.hang
                    } else {
                        0.0
                    };
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// JS `_startGhost(a, id)` (L277-285): a live/`net` active that never heard
/// its end closes first; build the ghost active with `ghost = true`.
fn start_ghost(a: &mut Actor, id: &str, _sys: &mut SpecialSystem) {
    // JS L278: `if (a.specialActive?.ghost) this.end(a, 'net')` — a previous
    // *ghost* active that never heard its end closes first (ghost ends never
    // record, L367 guard).
    if a.special_active.as_ref().is_some_and(|s| s.ghost) {
        end(a, "net");
    }
    let Some(def) = def_by_id(id) else {
        return;
    };
    let kind = def.kind(id);
    let dur = def.duration();
    let mut s = SpecialActive::new(id, &kind, dur);
    s.ghost = true;
    // JS L282: `g && 'start' in g ? g.start : IMPL[s.kind]?.start` — kinds
    // with no `GHOST.start` fall back to the live `IMPL.start`. Batch-2
    // fallout: `bubbler` has no GHOST entry at all (the field is each
    // client's own, L2049-2121), `sonar`'s GHOST arm is `IMPL.sonar.start`
    // itself (L2066 — the ghost reveals this screen's enemies too; the match
    // / replication layer runs the roster half), `kraken`'s GHOST arm has
    // only `tick` (L2103) → the fallback runs `IMPL.kraken.start`.
    // `slam` / `storm` have neither GHOST nor IMPL entries → bare ghost
    // actives (the actor layer never builds ghost phase machines).
    // Batch-3: `GHOST.strike.start` (L2050) is the throw trigger only
    // (presentation) → an empty arm blocks the IMPL fallback (the ghost
    // never aims or launches; its missile arrives over op 2). `wail` /
    // `zooka` / `blower` / `barrage` have no GHOST start → the fallback
    // runs the live `IMPL.start` (L2052: `GHOST.wail.start` *calls*
    // `IMPL.wail.start`, so the sim arm is identical).
    match kind.as_str() {
        "crab" => crab::start(a, &mut s),
        "booyah" => booyah::start(a, &mut s),
        "bubbler" => bubbler::start(a, &mut s),
        "sonar" => sonar::start(a, &mut s),
        "kraken" => kraken::start(a, &mut s),
        "strike" => {}
        "zooka" => zooka::start(a, &mut s),
        "wail" => wail::start(a, &mut s),
        "blower" => blower::start(a, &mut s),
        "barrage" => barrage::start(a, &mut s, id),
        // Batch-4: `jetpack` / `stamp` / `zipcaster` have no `GHOST.start`
        // (L2066-2104: jetpack's GHOST arm is `tick` only, stamp's is
        // `event` only, zipcaster's is `tick` + `event`) → the fallback
        // runs the live `IMPL.start` (the `sp_marker` it raises is built
        // ghost-side by the op-0 drain).
        "jetpack" => jetpack::start(a, &mut s),
        "stamp" => stamp::start(a, &mut s),
        "zipcaster" => zipcaster::start(a, &mut s),
        _ => {}
    }
    a.special_active = Some(Box::new(s));
}

/// Record triple at `data[i..]` (JS `V(d, 3)` / `V(d, 6)` / `V(d, 7)` —
/// `data` is the record payload after `[op, kind, gid]`, so `V(d, 3+n)` maps
/// to `v3at(data, n)`).
fn v3at(d: &[f32], i: usize) -> Vec3 {
    vec3(
        d.get(i).copied().unwrap_or(0.0),
        d.get(i + 1).copied().unwrap_or(0.0),
        d.get(i + 2).copied().unwrap_or(0.0),
    )
}

/// JS `_ghostObj(a, kind, gid, data)` (L306-322): the visual ghost world
/// objects (`mi/tw/sk/bu/ts/sh/or`). Batch 3 builds the real bodies so the
/// ghost steps through the same object mechanics (muted by construction —
/// [`world`] gates paint / damage on `ghost`). Batch 4 added `ts` / `sh`
/// (the thrown stamp / crab shell — their classes have no `netEvent`).
pub fn ghost_obj(
    sys: &mut SpecialSystem,
    ctx: &mut world::WorldCtx,
    a_idx: usize,
    kind: &str,
    gid: u32,
    data: &[f32],
) {
    if !matches!(kind, "mi" | "tw" | "sk" | "bu" | "ts" | "sh" | "or") {
        return;
    }
    if sys.world.iter().any(|w| w.gid == gid) {
        return; // JS L309: `if (this.world.some(x => x.gid === gid)) return`
    }
    let a = &ctx.actors[a_idx];
    let (slot, team) = (a.slot, a.team);
    let mut obj = match kind {
        // JS L311: `new Missile(this, a, V(d, 3))` — the ctor derives `from`
        // from the actor (`a.pos + 1.6y`, L779; the `1.6` is hardcoded).
        "mi" => {
            let from = a.pos + vec3(0.0, 1.6, 0.0);
            world::make_missile(slot, team, true, from, v3at(data, 0))
        }
        // JS L312-313: pos = `V(d, 3)`, dir = `V(d, 6)`.
        "tw" => world::make_twister(slot, team, true, v3at(data, 0), v3at(data, 3)),
        "sk" => world::make_speaker(slot, team, true, v3at(data, 0), v3at(data, 3)),
        // JS L314: `new Bubble(this, a)` (one `ph` draw) then
        // `pos = V(d,3)`, `r = d[6]`, `release(V(d,7))`.
        "bu" => {
            let mut w = world::make_bubble(slot, team, true, ctx.rng);
            w.pos = v3at(data, 0);
            if let world::ObjectBody::Bubble(b) = &mut w.body {
                b.r = data.get(3).copied().unwrap_or(0.0);
                world::bubble_release(b, v3at(data, 4));
            }
            w
        }
        // JS L317: `new Orb(this, a, V(d, 3), V(d, 6))`.
        "or" => world::make_orb(slot, team, true, v3at(data, 0), v3at(data, 3)),
        // JS L315: `new ThrownStamp(this, a, V(d, 3), V(d, 6))` — the
        // record payload is `[...v3(from), ...v3(dir)]` (L1629).
        "ts" => world::make_tstamp(slot, team, true, v3at(data, 0), v3at(data, 3)),
        // JS L316: `new Shell(this, a, V(d, 3), V(d, 6))` — `[...v3(m),
        // ...v3(vel)]` (crab cannon, L1985).
        "sh" => world::make_shell(slot, team, true, v3at(data, 0), v3at(data, 3)),
        _ => world::WorldObject::ghost(kind, gid, team, v3at(data, 0)),
    };
    obj.gid = gid; // JS L320: `w.ghost = true; w.gid = gid`
    sys.world.push(obj);
}

/// JS `netHurtObj(gid, dmg)` (L324-327): only the *owner's* bubble takes
/// ghost-side damage records (`!x.ghost && x.gid === gid && !x.dead`,
/// L325); `dmg < 0` marks own-team ink (`team = w.team`, charge path),
/// positive is enemy ink (`team = 1 - w.team`, soak path).
pub fn net_hurt_obj(sys: &mut SpecialSystem, gid: u32, dmg: f32, ctx: &mut world::WorldCtx) {
    // JS L325-326: the owner-side object lookup (not the ghost list), and
    // only `kind === 'bubble'` (the wire tag `"bu"`) is hurtable — the storm
    // cloud takes no damage.
    let Some(wi) = sys
        .world
        .iter()
        .position(|w| !w.ghost && w.gid == gid && !w.dead && w.kind == "bu")
    else {
        return;
    };
    let wt = sys.world[wi].team;
    world::bubble_hit(
        wi,
        if dmg < 0.0 { wt } else { 1 - wt },
        dmg.abs(),
        None,
        &mut sys.world,
        ctx,
        false,
    );
}

/// JS `_ghostTick(dt)` (L287-298) for one proxy actor: advance the ghost
/// timer, decay `firing`, end on death / duration. `ghostMute` (registry.js
/// L73) means the ghost never paints or damages — the sim enforces that by
/// construction (OQ-3: outline-block tier, state consumption only).
pub fn ghost_tick(a: &mut Actor, dt: f32) {
    let Some(mut s) = a.special_active.take() else {
        return;
    };
    if !s.ghost || s.ended {
        a.special_active = Some(s);
        return;
    }
    if !a.alive {
        // JS L291: `if (!a.alive) { this.end(a, 'splat'); continue }` — a
        // ghost end never records (L367 guard).
        a.special_active = Some(s);
        end(a, "splat");
        return;
    }
    s.t += dt;
    s.firing = (s.firing - dt).max(0.0);
    // JS L295-296: the GHOST tick table under ghostMute. The batch-2
    // kinds need nothing here: `GHOST.kraken.tick` (L2057-2064) is the
    // body3 mesh pose only, `GHOST.jetpack.tick` (L2066-2076) is nozzle
    // FX / loop audio only, and ghost bubbler / sonar / slam / storm have
    // no GHOST tick at all.
    match s.kind.as_str() {
        // JS `GHOST.crab.tick` (L2106-2119): the hull lerp toward the
        // snapshot, the roll flip, the firing hold, `faceYaw = hull` and the
        // `IMPL.crab.tick` half (L2118) — all under the `ghostMute` window
        // (L296). The IMPL half's sim output is the rolled-footprint request
        // (L2005): the ghost raises it too and the replication drain runs it
        // muted — the seed draw still happens (specials.js L53 evaluates
        // `Math.random()` before paint.js L470's mute return), the splat and
        // turf do not. The hull break (JS L2012) can't fire ghost-side: the
        // proxy's hull hp never takes local damage (`filterDamage` L412).
        "crab" => {
            if let KindState::Crab(st) = &mut s.state {
                if let Some(n) = st.net {
                    st.hull += angle_diff(st.hull, n.hull) * 1.0f32.min(dt * 15.0); // JS L2109
                    st.roll = n.roll; // JS L2110-2114 (the loop swap is audio)
                    if n.firing {
                        s.firing = 0.3; // JS L2115
                    }
                }
                s.face_yaw = Some(st.hull); // JS L2117
            }
            crab::tick(a, &mut s, dt); // JS L2118
        }
        // JS L2089: `GHOST.zipcaster.tick` = `IMPL.zipcaster.tick` — the
        // hang release ladder (a ghost never moves, so the push-away gate
        // reads a neutral input).
        "zipcaster" => zipcaster::tick(a, &mut s, dt, &ActorInput::default()),
        // JS L2086: `GHOST.booyah.tick` — without the `spst` charge
        // channel the ghost estimates the fill from its own timer, then
        // runs the IMPL tick (the sim half is the `cheered` decay,
        // L1746; the rest of that arm is ball visuals).
        "booyah" => {
            if let crate::special::KindState::Booyah(st) = &mut s.state
                && !st.net_charge
            {
                st.charge = (s.t / specials().booyah.charge).min(1.0);
            }
            booyah::tick(&mut s, dt);
        }
        _ => {}
    }
    a.special_active = Some(s);
    let expire = a
        .special_active
        .as_ref()
        .is_some_and(|s| s.dur > 0.0 && s.t >= s.dur);
    if expire {
        // JS L297: `if (s.dur && s.t >= s.dur) end(a, 'time')` — ghost ends
        // never record (L367 guard).
        end(a, "time");
    }
}

// ---------------------------------------------------------------------------
// specialNetState / specialNetApply (specials.js L2124-2136)
// ---------------------------------------------------------------------------

/// JS `specialNetState(a)` (L2124-2130): the `spst` (s[23]) bit pack.
#[must_use]
pub fn net_state(a: &Actor) -> u32 {
    let Some(s) = a.special_active.as_ref() else {
        return 0;
    };
    // JS L2125: `if (!s || s.ghost || !s.def) return 0` — the `net`
    // placeholder active (no def, netmatch.js L381) packs 0.
    if s.ghost || !s.has_def {
        return 0;
    }
    match s.kind.as_str() {
        // JS L2126-2128: crab — roll | firing<<1 | hull byte<<2.
        "crab" => {
            let roll = s.crab_roll() as u32;
            let firing = (s.firing > 0.0) as u32;
            let hull = s.crab_hull();
            let tau = std::f32::consts::TAU;
            let h = ((hull % tau + tau) % tau) / tau * 255.0;
            // JS L2127: `Math.round(hull / TAU * 255) & 255`.
            ((h.round() as u32) & 255) << 2 | firing << 1 | roll
        }
        // JS L2129: booyah — `round(clamp(charge, 0, 1) * 63)`.
        "booyah" => (s.booyah_charge().clamp(0.0, 1.0) * 63.0).round() as u32,
        _ => 0,
    }
}

/// JS `specialNetApply(a, v)` (L2131-2136): ghost-side consumption only.
pub fn net_apply(a: &mut Actor, v: u32) {
    let Some(s) = a.special_active.as_mut() else {
        return;
    };
    if !s.ghost {
        return;
    }
    match s.kind.as_str() {
        "crab" => {
            // JS L2133: `{roll: v&1, firing: v&2, hull: ((v>>2)&255)/255*TAU}`.
            s.set_crab_net(
                v & 1 != 0,
                (v >> 1) & 1 != 0,
                (((v >> 2) & 255) as f32) / 255.0 * std::f32::consts::TAU,
            );
        }
        "booyah" => {
            // JS L2135: `netCharge = true; charge = (v & 63) / 63`.
            s.set_booyah_net_charge((v & 63) as f32 / 63.0);
        }
        _ => {}
    }
}

/// JS `_spawn(a, w, kind, ...data)` (L300-305): push the world object, stamp
/// `gid = netId(a)` and record `[2, kind, gid, ...data]`. Batch-1 registry
/// entry point (called by batch-2 mechanics).
pub fn spawn(a: &mut Actor, sys: &mut SpecialSystem, obj: world::WorldObject, data: &[f32]) {
    let mut obj = obj;
    obj.gid = sys.next_gid(a.slot);
    if obj.gid != 0 {
        let mut rec = vec![
            KitVal::Num(2.0),
            KitVal::Str(obj.kind.clone()),
            KitVal::Num(obj.gid as f32),
        ];
        rec.extend(data.iter().map(|v| KitVal::Num(*v)));
        a.sp_records.push(rec);
    }
    sys.world.push(obj);
}

/// Drain [`Actor::sp_marker_action`] (raised by [`end`], JS L363-376) onto
/// the owner's live [`world::Marker`]: `returning()` sets `back` and clamps
/// `t` (JS L714), `finish()` marks `done` (JS L744). Called match-side after
/// the special tick (and ghost-side after `_ghostTick`).
pub fn drain_marker_action(a: &mut Actor, sys: &mut SpecialSystem) {
    let Some(returning) = a.sp_marker_action.take() else {
        return;
    };
    let Some(wi) = world::find_marker(&sys.world, a.slot) else {
        return;
    };
    if let world::ObjectBody::Marker(mk) = &mut sys.world[wi].body {
        if returning {
            // JS `returning()` (L714): `m.back = true; m.t = Math.min(m.t, 1)`.
            mk.back = true;
            mk.t = mk.t.min(1.0);
        } else {
            // JS `finish()` (L744): `m.done = true`.
            mk.done = true;
        }
    }
}

/// The slam/storm phase machine lives in [`crate::actor`] (JS
/// `_updateSpecial`, actor.js L861-898); re-exported for docs.
pub use crate::actor::SpecialPhase as Phase;

#[cfg(test)]
mod tests;
