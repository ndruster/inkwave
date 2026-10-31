//! Turf-war match flow (sim layer): lifecycle, roster, scoring, fixed-step API.
//!
//! Faithful port of `src/game/match.js` (Turf War mode) plus the splat
//! attribution hooks of `src/game/actor.js` / `src/game/weapons.js` that the
//! weapon layer deferred here in Task 7:
//!   - `Match` constructor / `setup()` roster + spawn placement -> [`Match::with_roster`]
//!   - `Match.start` / `setState`                               -> [`Match::start`] / [`Match::set_phase`]
//!   - `Match.update` state machine (intro 4.2 s → playing clock → finish
//!     2.6 s → judge) and the bot-intent zeroing (L220-226)      -> [`Match::step`]
//!   - `Match.update` soft push between actors (L231-244)        -> [`soft_push_masked`]
//!   - `Match._judge` (coverage, winner, tie-break)              -> [`Match::judge`]
//!   - `Match._onSplatted` kill log                              -> [`Match::step`] actor drain
//!   - `Actor.splat` attacker `stats.splats++` + splash splat     -> [`Match::step`] splat drain
//!     (actor.js L225-230)
//!   - `Projectiles._credit` → `Actor.addTurf`                    -> [`Match::route_turf`]
//!
//! Out of scope (M1, per PORT_MAP): `zones` / `boss` / `practice` / `attract`
//! modes, online roster (`_setupRoster`), `PlayerController` / `BotBrain`
//! wiring (step inputs come from the caller — Task 9 autopilot), `teamSummary`
//! HUD glue. `removeActor` (netcode) lands as [`Match::remove_actor`] with
//! the Task 21 replication layer; the host/guest gating (`follower`,
//! match.js L106/L211/L216) is a [`Match`] field the replication layer sets.
//!
//! Intentional deviations from JS (recorded for review):
//!   - **Spec phase names**: the spec compresses the lifecycle to intro →
//!     active → end; JS uses `intro` / `playing` / `finish` + `judge`.
//!     [`Phase`] keeps the JS granularity (`Active` = JS `playing`, `End` =
//!     JS `judge`, reached after the 2.6 s finish settle window).
//!   - **Intro confinement**: upstream has no gameplay barrier during the
//!     intro (the `spawnPad:barrier` cylinder in `decor.js` is visual;
//!     `Actor._spawnBarrier` is the always-on *enemy*-pad keep-out). Spec
//!     TR-8.2 requires the intro to hold actors inside their own spawn
//!     radius, so [`Match::step`] clamps live actors to `spawn_barrier` of
//!     their own pad during [`Phase::Intro`] only, killing the outward radial
//!     velocity (JS `_spawnBarrier` reflects it ×1.6; the intro clamp is a
//!     spec-only rule, so the simpler hold is used).
//!   - **Respawn gating**: JS `Actor.update` consults `G.match.canRespawn()`
//!     inline (actor.js L247); the sim `Actor::step` has no match handle, so
//!     [`Match::step`] holds a dead actor's `respawn_timer` just above the
//!     step while the phase does not allow respawning. Same observable
//!     behaviour, no actor-layer change.
//!   - **`Math.random()`** (judge tie-break match.js L269, splash seeds) uses
//!     the match's seeded [`Rng`] stream — draw *sequence* per event matches
//!     JS, values differ (sim determinism contract).
//!   - **Event-bus order**: JS emits `splatted` / `turf` / `hit` /
//!     `weapon:impact` interleaved per projectile; the match drains the
//!     projectile queue once per step and re-emits on its own bus, so
//!     intra-step ordering is coalesced (all credits land in the same step).
//!   - **Result fields**: JS `result` is `{coverage, winner}`; spec TR-8.1
//!     also wants the score and the final time, so [`MatchResult`] adds
//!     per-team turf points (`Σ stats.turf × MATCH.pointsPerM2`) and elapsed
//!     match time.
//!   - **Kill log / bus payloads** carry identity `(team, slot)` pairs (never
//!     array indices), like the Task 7 event payloads; the match resolves
//!     them against its own roster.
//!   - **`duration` validation**: JS clamps the match length to
//!     `MATCH.durations` / `maxDuration` (config.js L472-475); the sim takes
//!     the length as given — the caller (Task 9 CLI / menu) must pass a
//!     legal value.

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, ActorEvent, ActorInput, FireGate, Form, SimWorld, SplatCause};
use crate::paint::{PaintGrid, SplatOpts};
use crate::sub::{SubKind, SubSystem};
use crate::tuning::{MatchConfig, PlayerTuning, SubSet, WeaponSet};
use crate::weapon::blade::BladeWave;
use crate::weapon::brolly::{HELD_R, LAUNCH_R, LaunchedCanopy, held_shield};
use crate::weapon::{
    FoeScan, ProjectileSim, Rng, Shield, ShieldOwner, ShieldTarget, SimEvent, WeaponKind,
    WeaponRunner,
};
use crate::weapon::{bow, mitts};
use glam::{Vec3, vec3};

/// JS intro length (match.js L193: `stateT > 4.2`).
pub const INTRO_TIME: f32 = 4.2;
/// JS post-clock settle window before the judge runs (match.js L216: `2.6`).
pub const FINISH_TIME: f32 = 2.6;
/// JS spawn ring radius on the pad (match.js L87: turf mode `rr = 1.2`).
pub const SPAWN_RING_R: f32 = 1.2;
/// JS spawn ring phase offset (match.js L87: `+ 0.6`).
pub const SPAWN_RING_PHI: f32 = 0.6;
/// JS spawn ring divisor (match.js L87: `slot / 4` — fixed 4, not teamSize).
pub const SPAWN_RING_N: f32 = 4.0;
/// JS soft-push separation radius factor (match.js L236: `PLAYER.radius * 1.7`).
pub const SOFT_PUSH_R: f32 = 1.7;
/// JS soft-push vertical gate (match.js L237: `Math.abs(dy) < 1.2`).
pub const SOFT_PUSH_DY: f32 = 1.2;
/// JS soft-push equal-share weights (match.js L239: offline all-local → 0.5).
pub const SOFT_PUSH_K: f32 = 0.5;
/// JS attacker splash splat radius on splat (actor.js L229: `1.7`).
pub const SPLAT_SPLASH_R: f32 = 1.7;
/// JS attacker splash splat height above the victim feet (actor.js L228).
pub const SPLAT_SPLASH_Y: f32 = 0.35;

/// Match lifecycle phase (JS `state`; see module docs for the spec mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Built, not started (JS `init`).
    Init,
    /// Spawn confinement + pre-countdown (JS `intro`, [`INTRO_TIME`] s).
    Intro,
    /// Live play, clock running (JS `playing`; spec "active").
    Active,
    /// Clock at zero, settling before the judge (JS `finish`, [`FINISH_TIME`] s).
    Finish,
    /// Settled: [`Match::result`] is final (JS `judge`; spec "end").
    End,
}

/// Turf-war settlement (JS `_judge` `result`, extended per the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    /// Coverage fractions `[team0, team1]` (JS `paint.coverage()`).
    pub coverage: [f32; 2],
    /// Winning team (tie broken on the seeded RNG stream, JS L269).
    pub winner: usize,
    /// Per-team turf points: `Σ stats.turf × pointsPerM2` (spec score line).
    pub points: [f32; 2],
    /// Elapsed match time when the clock hit zero (spec "终局时间").
    pub elapsed: f32,
}

/// One splat record (JS `match.events` entry, match.js L185; identity only).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KillLog {
    /// Match time of the splat (JS `duration - time`).
    pub t: f32,
    pub victim_team: usize,
    pub victim_slot: usize,
    /// `None` when JS credits nobody: water deaths outside the 4 s chase
    /// window (actor.js L384) and environmental deaths.
    pub attacker: Option<(usize, usize)>,
    pub cause: SplatCause,
}

/// One drained splat: victim identity, cause and the JS `splat` attacker
/// credit (identity pair, `None` when nobody is credited).
type SplatRecord = (usize, usize, SplatCause, Option<(usize, usize)>);

/// Match event bus (JS `emit('match:state' / 'splatted' / 'turf' / ...)`).
/// HUD / netcode / FX consumers drain this instead of a global bus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MatchEvent {
    /// Phase transition (JS `match:state`).
    Phase(Phase),
    /// An actor was splatted (JS `splatted`, match.js L184 listener).
    Splat {
        victim_team: usize,
        victim_slot: usize,
        attacker: Option<(usize, usize)>,
        cause: SplatCause,
    },
    /// Turf credited to an actor (JS `turf` emit from `addTurf`).
    Turf {
        owner_team: usize,
        owner_slot: usize,
        area: f32,
    },
    /// Whole-second tick while `time <= finalCountdown` (JS `match:count`).
    Countdown { n: i32 },
    /// An actor respawned (JS `on('respawn')` → netmatch `_onLocalEvent`,
    /// L77/L139-143). The replication layer forwards it to proxies as the
    /// `['ev','respawn']` timeline record (Task 23); the actor layer already
    /// emits `ActorEvent::Respawn` — this only surfaces it on the match bus.
    Respawn { team: usize, slot: usize },
    /// Clock crossed 60 s in a match longer than 60 s (JS `match:oneminute`).
    OneMinute,
    /// A round hit a proxy actor — JS `applyHit` route `'send'` (weapons.js
    /// L1551, netmatch.js `shouldApplyHit` L711-717): the shooter decided the
    /// hit, the victim's *owner* must apply it. The replication layer (Task
    /// 21) forwards `{k:'hit'}` to the owner and drops the local damage.
    HitSend {
        /// Attacker identity `(team, slot)`.
        attacker: (usize, usize),
        /// Victim identity `(team, slot)`.
        victim: (usize, usize),
        damage: f32,
    },
    /// A round left a local actor's muzzle (JS `weapon:fire`, forwarded by
    /// netmatch `_onLocalEvent` L139-143 → `recProj` → the `['p', …]` timeline
    /// record + `ghostFire` on the receivers, Task 23). The replication layer
    /// drains this to record the shot; identities are `(team, slot)` pairs.
    Fire {
        /// Firing actor identity `(team, slot)`.
        owner: (usize, usize),
        muzzle: Vec3,
        dir: Vec3,
        /// Charger beam charge (JS weapons.js L1281: only `fireCharger`
        /// attaches `charge`/`len` to the `weapon:fire` payload — the remote
        /// `_ghostBeam` input). `None` for every other shot.
        #[serde(default)]
        charge: Option<f32>,
        /// Beam length in metres (JS L1281, same charger-only rule).
        #[serde(default)]
        len: Option<f32>,
    },
}

/// A kit world-object spawn queued for the net layer (JS `recKit`,
/// netmatch.js L134-137 `['k', nid, kind, data]`): the owner's launch
/// surfaces as a [`crate::weapon::SimEvent::Kit`] and the replication
/// layer turns it into a [`crate::net::TickEvent::Kit`] timeline record.
/// `owner` is the `(team, slot)` identity (the replicator resolves it to
/// the roster `nid` byte).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitSpawn {
    pub owner: (usize, usize),
    pub kind: String,
    /// Sub-system kind string for `kind == "subs"` records — JS packs the
    /// mixed array `['k', nid, 'subs', [op, gid, kindStr, …]]` (subs.js
    /// L77/L103); the wire layer writes it back at `data[2]`. `None` for
    /// the main-weapon kits.
    #[serde(default)]
    pub sub_kind: Option<String>,
    pub data: Vec<f32>,
}

/// A splat bomb that left a local hand this frame, waiting for the
/// replication layer to record `['b', nid, kind, r2×6]` (JS `recBomb`,
/// weapons.js L1463 / netmatch.js L124-127). `owner` is the `(team, slot)`
/// identity; the wire layer resolves it to the roster `nid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BombSpawnOut {
    pub owner: (usize, usize),
    pub kind: String,
    pub pos: Vec3,
    pub vel: Vec3,
}

/// A hit a local shot landed on another screen's ghost device, waiting for
/// the replication layer to send `{k:'dh', kind, id, d}` to the device's
/// owner (JS `sendDevHit`, netmatch.js L157-159). `kind` is the device
/// wire kind (`'subs'` / `'waddle'` / `'torpedo'` / `'torpedoArea'`);
/// `gid` is the wire-form device id (the ghost's own, D-T34-5); `owner` is
/// the `(team, slot)` identity of the ghost's owner actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceHitOut {
    pub owner: (usize, usize),
    #[serde(default)]
    pub kind: String,
    pub gid: u32,
    pub d: f32,
}

/// A Turf War match: the roster (spec: 8 slots, 4v4), the paint grid, the
/// projectile sim, the clock and the rules glue between them.
#[derive(Serialize, Deserialize)]
pub struct Match {
    pub phase: Phase,
    /// Seconds since the current phase began (JS `stateT`).
    pub state_t: f32,
    /// Total match length (JS `duration`), seconds.
    pub duration: f32,
    /// Clock remaining (JS `time`).
    pub time: f32,
    pub paused: bool,
    pub actors: Vec<Actor>,
    pub runners: Vec<WeaponRunner>,
    /// Per-actor main weapon kind (spec D-2; JS `actor.weapon.kind`): the
    /// runner dispatch, the twins' dodge and the brolly shield assembly all
    /// index this parallel list.
    pub kinds: Vec<WeaponKind>,
    /// Launched brolly canopies (JS `LAUNCHED` world list, brolly.js L167):
    /// advanced every frame inside the projectile-tick window.
    pub launched: Vec<LaunchedCanopy>,
    /// Blade ink waves (JS `WAVES` world list, blade.js L245): advanced every
    /// frame right after the launched canopies (kit `tick` order, Task 32 §6).
    pub waves: Vec<BladeWave>,
    /// Bow arrows (JS `ARROWS` world list, bow.js L68): flying + lodged,
    /// advanced right after the waves (kit `tick` order, Task 33 §5).
    pub arrows: Vec<bow::BowArrow>,
    /// Mitts fists (JS `FISTS` world list, mitts.js L140): advanced right
    /// after the arrows (kit `tick` order, Task 33 §5).
    pub fists: Vec<mitts::MittsFist>,
    /// Kit spawns produced this frame, waiting for the replication layer to
    /// record them onto the outgoing timeline (JS `recKit`, Task 30 §4).
    /// Drained every frame by `Replicator::update`; cleared at the top of
    /// [`Match::step_masked`] so the offline path never grows it.
    #[serde(default)]
    pub kit_out: Vec<KitSpawn>,
    /// Sub-system devices (JS `SUBS` singleton, subs.js L56; Task 34).
    /// Seeded with its own salt so the device stream is independent of the
    /// projectile (`seed`) and judge (`seed ^ 0x5bd1_e995`) streams.
    #[serde(default)]
    pub subs: SubSystem,
    /// Splat-bomb throws produced this frame (JS `recBomb` runs inline in
    /// the sim step); drained by `Replicator::update`, cleared at the top
    /// of [`Match::step_masked`] like [`Match::kit_out`].
    #[serde(default)]
    pub bomb_out: Vec<BombSpawnOut>,
    /// Ghost-device hits produced this frame (JS `sendDevHit` runs inline
    /// at hit time); drained by `Replicator::update`, cleared at the top
    /// of [`Match::step_masked`].
    #[serde(default)]
    pub dh_out: Vec<DeviceHitOut>,
    pub paint: PaintGrid,
    pub projectiles: ProjectileSim,
    /// Seeded stream for the judge tie-break + splash seeds (JS `Math.random`).
    pub rng: Rng,
    /// Seed kept so [`Match::restart`] rebuilds the projectile stream like JS
    /// rebuilding the whole match.
    pub seed: u64,
    /// `MATCH.finalCountdown` (seconds; kept on the match so `step` needs no
    /// config handle).
    pub final_countdown: f32,
    /// `MATCH.pointsPerM2` for the result score line.
    pub points_per_m2: f32,
    pub result: Option<MatchResult>,
    pub events: Vec<MatchEvent>,
    pub kills: Vec<KillLog>,
    /// JS `this.follower = !o.host` (match.js L106): online, a guest match
    /// never calls time (`playing → finish`, L211) and never judges
    /// (`finish → judge`, L216) — the host's `{k:'st'}` / `{k:'res'}` drives
    /// those transitions through the replication layer. Offline it stays
    /// `false` and the match runs its own state machine.
    pub follower: bool,
    last_minute_fired: bool,
    last_count: i32,
}

impl Match {
    /// JS `new Match(...)` + `setup()` + `start()`: the spec roster (2 teams ×
    /// `team_size` slots, 4v4 = 8 by default), everyone placed on the team
    /// spawn ring, then straight into [`Phase::Intro`].
    #[must_use]
    pub fn new(
        duration: f32,
        seed: u64,
        world: &SimWorld,
        t: &PlayerTuning,
        mc: &MatchConfig,
    ) -> Self {
        Self::with_roster(duration, seed, world, t, mc, mc.team_size.max(1) as usize)
    }

    /// [`Match::new`] with an explicit per-team roster size (tests use small
    /// rosters; team balance = equal sizes by construction).
    #[must_use]
    pub fn with_roster(
        duration: f32,
        seed: u64,
        world: &SimWorld,
        t: &PlayerTuning,
        mc: &MatchConfig,
        team_size: usize,
    ) -> Self {
        let mut actors = Vec::with_capacity(team_size * 2);
        for team in 0..2 {
            for slot in 0..team_size {
                let mut a = Actor::new(team, slot, t);
                Self::place_on_pad(&mut a, world, t);
                actors.push(a);
            }
        }
        let mut m = Self {
            // JS constructor starts in `init`; `start()` moves to `intro`.
            phase: Phase::Init,
            state_t: 0.0,
            duration,
            time: duration,
            paused: false,
            actors,
            runners: (0..team_size * 2).map(|_| WeaponRunner::new()).collect(),
            // Spec D-2: the default loadout is the Spritzer; the net builder
            // overwrites this from the roster weapon strings.
            kinds: vec![WeaponKind::Shooter; team_size * 2],
            launched: Vec::new(),
            waves: Vec::new(),
            arrows: Vec::new(),
            fists: Vec::new(),
            kit_out: Vec::new(),
            subs: SubSystem::new(seed ^ 0x9e37_79b9),
            bomb_out: Vec::new(),
            dh_out: Vec::new(),
            paint: PaintGrid::new(world.collision),
            projectiles: ProjectileSim::new(seed),
            rng: Rng::new(seed ^ 0x5bd1_e995),
            seed,
            final_countdown: mc.final_countdown as f32,
            points_per_m2: mc.points_per_m2,
            result: None,
            events: Vec::new(),
            kills: Vec::new(),
            follower: false,
            last_minute_fired: false,
            last_count: 99,
        };
        m.start();
        m
    }

    /// JS `setup()` placement (match.js L85-91): spawn-ring offset around the
    /// team pad, facing the arena, then `invuln = 0` (the match clears the
    /// spawn invulnerability `spawn_at` grants — the intro is the protection).
    fn place_on_pad(a: &mut Actor, world: &SimWorld, t: &PlayerTuning) {
        let pad = world.spawn_pads[a.team];
        let ang = (a.slot as f32 / SPAWN_RING_N) * std::f32::consts::TAU + SPAWN_RING_PHI;
        let p = vec3(
            pad.x + ang.cos() * SPAWN_RING_R,
            pad.y,
            pad.z + ang.sin() * SPAWN_RING_R,
        );
        let yaw = if a.team == 0 {
            0.0
        } else {
            std::f32::consts::PI
        };
        a.spawn_at(p, yaw, world, t);
        a.invuln = 0.0;
    }

    /// JS `start()`: enter the intro.
    pub fn start(&mut self) {
        self.set_phase(Phase::Intro);
    }

    /// JS `setState(s)`: phase + timer reset + `match:state` emit.
    /// Public for the replication layer (Task 21): guests re-enter the
    /// phase on the host's `{k:'st'}` (`_hostState`, netmatch.js L648).
    pub fn set_phase(&mut self, p: Phase) {
        self.phase = p;
        self.state_t = 0.0;
        self.events.push(MatchEvent::Phase(p));
    }

    /// JS `playing()`.
    #[must_use]
    pub fn playing(&self) -> bool {
        self.phase == Phase::Active && !self.paused
    }

    /// JS `canRespawn()` (the actor.js L247 gate; see module docs).
    #[must_use]
    pub fn can_respawn(&self) -> bool {
        self.phase == Phase::Active
    }

    /// Elapsed match time (JS `duration - time`).
    #[must_use]
    pub fn elapsed(&self) -> f32 {
        self.duration - self.time
    }

    /// Restart in place (spec "可 restart" / TR-8.1 归零): fresh clock, roster
    /// re-placed on the pads, paint / projectiles / stats cleared. JS tears
    /// the `Match` down and builds a new one; the sim reuses the object so
    /// consumers keep their handle.
    pub fn restart(&mut self, world: &SimWorld, t: &PlayerTuning) {
        self.state_t = 0.0;
        self.time = self.duration;
        self.paused = false;
        self.result = None;
        self.events.clear();
        self.kills.clear();
        self.last_minute_fired = false;
        self.last_count = 99;
        self.paint = PaintGrid::new(world.collision);
        self.projectiles = ProjectileSim::new(self.seed);
        self.rng = Rng::new(self.seed ^ 0x5bd1_e995);
        // JS `clearAll()` (brolly.js L487-490): world canopies die with the
        // match; the runner kits are released by `r.reset()` below.
        self.launched.clear();
        self.waves.clear();
        self.arrows.clear();
        self.fists.clear();
        self.kit_out.clear();
        // JS rebuilds the whole `Match` (and with it the `SUBS` singleton);
        // the sim resets it in place with the same seed salt as the
        // constructor.
        self.subs = SubSystem::new(self.seed ^ 0x9e37_79b9);
        self.bomb_out.clear();
        self.dh_out.clear();
        for (a, r) in self.actors.iter_mut().zip(self.runners.iter_mut()) {
            r.reset();
            a.reset(t);
            // JS stats live on the Actor and survive respawn; a restart is a
            // new match, so the match layer zeroes them here.
            a.turf = 0.0;
            a.splats = 0;
            a.deaths = 0;
            Self::place_on_pad(a, world, t);
        }
        self.set_phase(Phase::Intro);
    }

    /// JS `update(dt)` for the turf-war path: phase machine, clock, actor
    /// steps, weapon runners, soft push, projectile step and the scoring
    /// drains. `inputs` aligns with [`Match::actors`] (Task 9 bots / the
    /// local player supply them); outside [`Phase::Active`] every intent is
    /// zeroed like JS L220-226.
    pub fn step(
        &mut self,
        dt: f32,
        inputs: &[ActorInput],
        world: &SimWorld,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
    ) {
        self.step_masked(dt, inputs, None, world, t, w, sb);
    }

    /// [`Self::step`] with the online ownership mask (Task 21 replication,
    /// JS `Match.update` L221-244): `remote[i]` marks actor `i` a proxy —
    /// JS `if (a.remote && nm) nm.applyRemote(a, dt); else a.update(dt)`
    /// (L228). Proxies run no physics, no weapon runner and no input; their
    /// state is written by the playback layer (Task 22) through
    /// [`crate::net::replication`]. `None` = offline (every actor local).
    #[allow(clippy::too_many_arguments)]
    pub fn step_masked(
        &mut self,
        dt: f32,
        inputs: &[ActorInput],
        remote: Option<&[bool]>,
        world: &SimWorld,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
    ) {
        // JS runs the whole match once per 60 Hz frame; variable dt would
        // desync the clock, timers and the fixed-step contract.
        debug_assert!(
            (dt - crate::actor::FIXED_DT).abs() < 1e-6,
            "Match::step must run at the fixed 60 Hz step, got dt={dt}"
        );
        if self.paused {
            return;
        }
        // Per-frame kit-spawn queue (JS `recKit` is inline during the sim
        // step): the replication layer drains it in the same frame, offline
        // matches simply drop it next frame.
        self.kit_out.clear();
        self.bomb_out.clear();
        self.dh_out.clear();
        self.state_t += dt;
        match self.phase {
            Phase::Init => {}
            Phase::Intro => {
                if self.state_t > INTRO_TIME {
                    self.set_phase(Phase::Active);
                }
            }
            Phase::Active => {
                self.time -= dt;
                if !self.last_minute_fired && self.time <= 60.0 && self.duration > 60.0 {
                    self.last_minute_fired = true;
                    self.events.push(MatchEvent::OneMinute);
                }
                let c = self.time.ceil() as i32;
                if self.time <= self.final_countdown && c != self.last_count && c > 0 {
                    self.last_count = c;
                    self.events.push(MatchEvent::Countdown { n: c });
                }
                if self.time <= 0.0 {
                    self.time = 0.0;
                    // JS L211: online, only the host calls time — a guest
                    // clock hitting zero waits for the host's
                    // `{k:'st', s:'finish'}` (netmatch `_hostState` L648).
                    if !self.follower {
                        self.set_phase(Phase::Finish);
                    }
                }
            }
            Phase::Finish => {
                // JS L216: `stateT > 2.6 && !follower && !result` — the
                // judge runs once, on the host only.
                if self.state_t > FINISH_TIME && !self.follower && self.result.is_none() {
                    self.judge();
                }
            }
            Phase::End => {}
        }

        // ---- actors. JS steps bots with zeroed intents outside `playing`
        // (L220-226) and the local controller is disabled the same way; the
        // sim has no controller, so non-live steps get idle input.
        // Online (L228): remote actors skip `Actor.update` entirely — the
        // playback layer writes their state (Task 22), so here they are held
        // frozen (no physics, no timers, no input).
        let live = self.phase == Phase::Active;
        let intro = self.phase == Phase::Intro;
        let can_respawn = self.can_respawn();
        for i in 0..self.actors.len() {
            if remote.is_some_and(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let inp = if live {
                inputs.get(i).copied().unwrap_or_default()
            } else {
                ActorInput::default()
            };
            // JS actor.js L305-306: the twins' roll owns the horizontal
            // velocity — read *before* the dodge may start this frame, so a
            // fresh roll first moves the actor on the next step (JS order).
            let dv = self.runners[i].dodge_velocity(self.kinds[i], w);
            self.actors[i].dodge_vel = dv;
            // JS actor.js L310: `tryDodge` swallows the jump buffer when the
            // twins dodge off the jump press instead (squid can't roll).
            if self.actors[i].alive
                && self.actors[i].jump_edge(&inp)
                && self.actors[i].form != Form::Squid
                && self.runners[i].try_dodge(
                    self.kinds[i],
                    &mut self.actors[i],
                    w,
                    inp.move_dir,
                    inp.fire,
                )
            {
                self.actors[i].jump_swallowed = true;
            }
            // JS `actor.js` L313: the mitts jump hook creates its kit
            // lazily (`K(r)`, mitts.js L48-67) — ensure it before the step
            // so the first jump press of a fresh mitts kid is handled.
            if self.kinds[i] == WeaponKind::Mitts {
                self.runners[i].ensure_mitts(w);
            }
            let a = &mut self.actors[i];
            // Respawn gating (see module docs): hold dead timers above the
            // step while the phase does not allow respawning.
            if !a.alive && !can_respawn && a.respawn_timer <= dt {
                a.respawn_timer = dt * 1.5;
            }
            a.step(
                dt,
                &inp,
                world,
                &self.paint,
                t,
                sb,
                Some(&self.subs),
                self.runners[i].mitts.as_deref_mut(),
            );
            // Intro confinement (spec TR-8.2; deviation — see module docs).
            if intro && a.alive {
                confine_to_own_pad(a, world);
            }
        }

        // ---- weapon runners (JS: each actor's `weaponRunner.update` inside
        // `Actor.update` via the gated `winp`; the sim keeps runners on the
        // match so the projectile stream is shared). Outside play the zeroed
        // gate idles the runner (cooldown/bloom decay only, no shots). A dead
        // actor never runs its weapon: JS `update` returns before the runner
        // call (actor.js L245-249) and `splat` fires `weaponRunner.onDeath()`
        // → `reset()` (weapons.js L60), so the runner is held reset and the
        // stale trigger gate is cleared until the respawn resets both.
        // Remote actors have no runner pass at all (JS L228 — `applyRemote`
        // only poses the character).
        for i in 0..self.actors.len() {
            if remote.is_some_and(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            // Task 32: the melee scans (roller / brush / blade) need the rest
            // of the roster alongside the runner's own actor — `FoeScan`
            // splits the slice around the index (spec §6).
            // JS L145: `SK?.blocked?.(a, sub)` — the kit refusal check
            // needs the device lists, which only the match layer has.
            let sub_blocked = self.subs.kit_blocked(self.actors[i].sub, i);
            let (a, mut foes) = FoeScan::split_around(&mut self.actors, i);
            let r = &mut self.runners[i];
            if !a.alive {
                r.reset();
                a.fire_gate = FireGate::default();
                continue;
            }
            let gate = a.fire_gate;
            let inp = if live {
                inputs.get(i).copied().unwrap_or_default()
            } else {
                ActorInput::default()
            };
            // Per-actor loadout (spec D-2): the runner dispatches by kind.
            // `inp.fire` is the raw (ungated) intent the blade kit reads as
            // JS `a.intent.fire` (blade.js L630, spec D-T32-2).
            let ps_time = self.projectiles.time;
            r.update(
                dt,
                &gate,
                a,
                self.kinds[i],
                w,
                sb,
                inp.move_dir,
                world.collision,
                &mut self.projectiles,
                &mut foes,
                &mut self.paint,
                t,
                remote,
                inp.fire,
                ps_time,
                &mut self.arrows,
                &mut self.fists,
                inp.jump,
                sub_blocked,
            );
            // A brolly launch this frame joins the world canopy list (JS
            // `addLaunched` → `LAUNCHED.push`, brolly.js L285).
            if let Some(c) = r.brolly_launch.take() {
                self.launched.push(c);
            }
            // A blade wave this frame joins the world wave list (JS
            // `addWave` → `WAVES.push`, blade.js L264).
            if let Some(wv) = r.blade_wave.take() {
                self.waves.push(wv);
            }
            // D-T31-1: the charger's hitscan fires here, not inside the
            // runner (which has no actor list for the victim scan). Same
            // frame position as JS's inline `fireCharger` call (weapons.js
            // L205): the actor's pose/aim is untouched in between, so the
            // muzzle, the aim and the RNG draw order all match JS.
            // Residual (recorded): JS steps each actor move-then-weapon
            // serially (match.js L228), so a JS beam sees victims with
            // index > i still on their *previous* frame's positions, while
            // the sim's two-pass loop has already moved every actor. The
            // victim scan can therefore differ by up to one frame of
            // motion (~0.13 m at 8 m/s) on near-miss beams.
            if let Some(c) = r.charger_release.take() {
                let (slot, team) = (a.slot, a.team);
                let m = ProjectileSim::muzzle(a);
                let dir = ProjectileSim::aim_from(a, m, None);
                self.projectiles.fire_charger(
                    slot,
                    team,
                    m,
                    dir,
                    c,
                    &w.charger,
                    world.collision,
                    &mut self.actors,
                    &mut self.paint,
                    t,
                    sb,
                    Some(&mut self.subs),
                    remote,
                );
            }
            // D-T34-9 deploy timing (D-T31-1 precedent): the runner has no
            // world / paint / subs handles, so `SUBS.use(a, sub)` (JS
            // `weaponRunner.update` L152-155 calls it inline) is deferred to
            // this same-frame, same-actor-loop position. The runner already
            // paid the ink and set `pending_sub` (weapons.js L150-155).
            if self.runners[i].pending_sub {
                self.runners[i].pending_sub = false;
                let kit_level = self.runners[i].pending_kit_level;
                let ps_time = self.projectiles.time;
                self.subs.use_sub(
                    i,
                    kit_level,
                    world.collision,
                    &mut self.actors,
                    &mut self.paint,
                    &mut self.projectiles,
                    t,
                    sb,
                    remote,
                    ps_time,
                );
            }
        }

        // ---- soft push between actors (JS match.js L231-244; offline every
        // actor is local, so both sides give way with weight 0.5; online a
        // remote squidkid is where its owner says — only the local side
        // gives way, L239).
        soft_push_masked(&mut self.actors, remote, t);

        // ---- projectiles (JS main.js L1119: after `match.update`; the brolly
        // kit's `tick`, brolly.js L390-485, runs inside the projectile step —
        // the sim interleaves the same order here: launched canopies move,
        // shields refresh, rounds step, poses snap, canopies shove).

        // JS `tick` L413-421: launched canopies move first (reverse iteration,
        // dead ones spliced out). Their ground stripe credits the owner like
        // `addTurf` (L337).
        let mut li = self.launched.len();
        while li > 0 {
            li -= 1;
            let (area, keep) = self.launched[li].update(
                dt,
                world.collision,
                &mut self.paint,
                t,
                &w.brolly,
                &mut self.projectiles.rng,
            );
            if area > 0.0 {
                let (team, slot) = (self.launched[li].team, self.launched[li].slot);
                if let Some(ai) = find_actor(&self.actors, team, slot) {
                    self.actors[ai].turf += area;
                    self.events.push(MatchEvent::Turf {
                        owner_team: team,
                        owner_slot: slot,
                        area,
                    });
                }
            }
            if !keep {
                self.launched.remove(li);
            }
        }

        // JS `updateWaves` (blade.js L296-303, kit `tick` L692): blade ink
        // waves advance right after the canopies, reverse order (faded ones
        // spliced out, JS L311). Their turf / hits ride the projectile event
        // queue (`SimEvent::Turf` / `Impact` / `Hit`), drained below like
        // every other splat credit.
        let mut wi = self.waves.len();
        while wi > 0 {
            wi -= 1;
            let keep = self.waves[wi].step(
                dt,
                world.collision,
                &mut self.paint,
                &w.blade,
                t,
                &mut self.projectiles,
                &mut self.actors,
                remote,
            );
            if !keep {
                self.waves.remove(wi);
            }
        }

        // JS `MAIN_KITS.bow.tick` L514-519 (Task 33 §5): a draw left hanging
        // (the runner stopped updating — death, a special, Task 35) is
        // cancelled quietly before the arrows advance.
        let ps_time = self.projectiles.time;
        for r in self.runners.iter_mut() {
            bow::sweep_stalled_draw(r, ps_time);
        }

        // JS `updateArrows` (bow.js L175-264, `tick` L520): arrows advance
        // right after the waves, reverse order (spent ones spliced out).
        // Turf / hits / impacts ride the projectile event queue, drained
        // below like every other splat credit.
        let mut ai = self.arrows.len();
        while ai > 0 {
            ai -= 1;
            let keep = bow::BowArrow::step(
                &mut self.arrows[ai],
                dt,
                world.collision,
                &mut self.paint,
                &w.bow,
                t,
                &mut self.projectiles,
                &mut self.actors,
                remote,
            );
            if !keep {
                // JS `kill(i)` (bow.js L71): swap-remove — the reverse
                // loop never re-steps the swapped-in (already-stepped) arrow.
                self.arrows.swap_remove(ai);
            }
        }

        // JS `updateFists` (mitts.js L197-246, `tick` L920): fists advance
        // after the arrows, reverse order. The foe scan covers the full list
        // (the owner is filtered by team inside the fist's own scans).
        let mut fi = self.fists.len();
        while fi > 0 {
            fi -= 1;
            let mut foes = FoeScan::all(&mut self.actors);
            let keep = mitts::MittsFist::step(
                &mut self.fists[fi],
                dt,
                world.collision,
                &mut self.paint,
                &w.mitts,
                t,
                &mut self.projectiles,
                &mut foes,
                remote,
            );
            if !keep {
                // JS `stepFist` L244: swap-remove — same reverse-loop
                // behaviour as the arrow `kill`.
                self.fists.swap_remove(fi);
            }
        }

        // JS `refreshShields` (L178-190): held canopies from the runners,
        // launched ones from the world list; a shield without a `prev` pose
        // snapshot takes the current pose as its sweep start (spawn frame).
        let mut shields: Vec<Shield> = Vec::new();
        for i in 0..self.runners.len() {
            if self.kinds[i] != WeaponKind::Brolly {
                continue;
            }
            let Some(kit) = self.runners[i].brolly.as_mut() else {
                continue;
            };
            match held_shield(kit, &self.actors[i]) {
                Some((c, n)) => {
                    kit.shield_c = c;
                    kit.shield_n = n;
                    if !kit.shield_prev {
                        kit.shield_cp = c;
                        kit.shield_np = n;
                        kit.shield_prev = true;
                    }
                    shields.push(Shield {
                        team: self.actors[i].team,
                        c,
                        n,
                        r: HELD_R,
                        cp: kit.shield_cp,
                        np: kit.shield_np,
                        regrow_time: w.brolly.regrow_time,
                        target: ShieldTarget::Held(i),
                    });
                }
                None => kit.shield_prev = false,
            }
        }
        for (li, c) in self.launched.iter_mut().enumerate() {
            if !c.alive {
                continue;
            }
            if !c.shield_prev {
                c.shield_cp = c.cur_c;
                c.shield_np = c.cur_n;
                c.shield_prev = true;
            }
            shields.push(Shield {
                team: c.team,
                c: c.cur_c,
                n: c.cur_n,
                r: LAUNCH_R,
                cp: c.shield_cp,
                np: c.shield_np,
                regrow_time: w.brolly.regrow_time,
                target: ShieldTarget::Launched(li),
            });
        }

        let mut owner = ShieldOwner {
            runners: &mut self.runners,
            launched: &mut self.launched,
        };
        self.projectiles.step_masked(
            dt,
            world.collision,
            &mut self.actors,
            &mut self.paint,
            t,
            w,
            sb,
            Some(&mut self.subs),
            remote,
            &shields,
            &mut owner,
        );

        // JS `snapShields` (L192): this frame's pose becomes the next
        // frame's sweep start.
        for (i, r) in self.runners.iter_mut().enumerate() {
            if self.kinds[i] != WeaponKind::Brolly {
                continue;
            }
            if let Some(kit) = r.brolly.as_mut() {
                kit.shield_cp = kit.shield_c;
                kit.shield_np = kit.shield_n;
            }
        }
        for c in self.launched.iter_mut() {
            c.shield_cp = c.cur_c;
            c.shield_np = c.cur_n;
        }

        // JS `tick` L484: launched canopies hold enemy players back.
        for li in 0..self.launched.len() {
            self.launched[li].block_actors(&mut self.actors, t.radius);
        }

        // JS `'splatted'` listener (subs.js L60): a splatted owner loses
        // their live sprinkler. `owner_splatted_destroy` *peeks* the actor
        // splat events (no drain), so it runs before the subs step and the
        // `[2, gid]` end record lands in the same frame — JS's listener
        // fires the instant the actor dies, which is always earlier in the
        // frame than `SUBS.update`.
        self.subs.owner_splatted_destroy(&self.actors);

        // JS tick order (subs.js header note): the sub devices step after
        // the projectiles — blasts and sprays see this frame's rounds.
        let ps_time = self.projectiles.time;
        self.subs.update(
            dt,
            world.collision,
            &mut self.actors,
            &mut self.paint,
            &mut self.projectiles,
            t,
            sb,
            remote,
            ps_time,
        );

        // ---- scoring drains (JS `on('hit'/'weapon:impact')` + `_credit`).
        let pev = self.projectiles.drain_events();
        self.route_turf(&pev);
        let sev = self.subs.drain_events();
        // A sub blast's turf credit rides the sub event queue (subs.js
        // `_credit`, L594) — route it like the projectile queue.
        self.route_turf(&sev);

        // ---- proxy hits: JS `applyHit` route `'send'` (weapons.js L1551,
        // netmatch.js `shouldApplyHit` L711-717). The shooter decided the hit;
        // the replication layer turns these into `{k:'hit'}` frames aimed at
        // the victim's owner. Identities resolve as `(team, slot)` pairs —
        // the attacker's team rides the payload's `team`, the victim's is
        // `victim_team` (the two differ by construction: JS `applyHit` only
        // routes enemy victims, weapons.js L1543).
        for e in &pev {
            match e {
                SimEvent::HitSend {
                    attacker_slot,
                    victim_slot,
                    victim_team,
                    team,
                    damage,
                } => {
                    self.events.push(MatchEvent::HitSend {
                        attacker: (*team, *attacker_slot),
                        victim: (*victim_team, *victim_slot),
                        damage: *damage,
                    });
                }
                // JS `weapon:fire` (weapons.js `fireShooter` emit): netmatch
                // `_onLocalEvent` forwards it for local actors → `recProj`
                // records the `['p', …]` timeline record (Task 23).
                SimEvent::Fire {
                    owner_slot,
                    team,
                    muzzle,
                    dir,
                    charge,
                    len,
                } => {
                    self.events.push(MatchEvent::Fire {
                        owner: (*team, *owner_slot),
                        muzzle: *muzzle,
                        dir: *dir,
                        charge: *charge,
                        len: *len,
                    });
                }
                // JS `recKit` (netmatch.js L134-137): a kit world-object
                // spawn (brolly launch, brolly.js L266) rides the net
                // timeline, not the HUD bus — the replication layer drains
                // `kit_out` and records `['k', nid, kind, data]`.
                SimEvent::Kit {
                    owner_slot,
                    team,
                    kind,
                    data,
                } => {
                    self.kit_out.push(KitSpawn {
                        owner: (*team, *owner_slot),
                        kind: kind.clone(),
                        sub_kind: None,
                        data: data.clone(),
                    });
                }
                // JS `recBomb` (weapons.js L1463, netmatch.js L124-127): a
                // splat bomb leaving a *local* hand rides the timeline as
                // `['b', nid, kind, r2×6]`; the replication layer drains
                // `bomb_out` and applies the JS `recBomb` local-actor guard.
                SimEvent::BombSpawn {
                    owner_slot,
                    team,
                    kind,
                    pos,
                    vel,
                } => {
                    self.bomb_out.push(BombSpawnOut {
                        owner: (*team, *owner_slot),
                        kind: kind.clone(),
                        pos: *pos,
                        vel: *vel,
                    });
                }
                _ => {}
            }
        }

        // ---- sub-system records (JS `netRec(a, 'subs', …)` runs inline in
        // the subs step, subs.js L77/L103/L165): spawns and ends ride the
        // `['k', nid, 'subs', data]` timeline; a hit on another screen's
        // ghost device rides the `{k:'dh'}` direct channel (netmatch.js
        // L157-159). `sub:use` is local presentation only — it is not in
        // the netmatch `FORWARD` list (L47), so the sim drops it (D-T34-4).
        for e in &sev {
            match e {
                SimEvent::SubSpawn {
                    owner_slot,
                    team,
                    sub_kind,
                    data,
                } => {
                    // Kit subs ride the wire with the kit id as the kind
                    // (`['k', nid, 'shaker', …]`, kits/registry.js); the
                    // classic subs keep `kind: 'subs'` with the kind string
                    // lifted into `sub_kind`.
                    let kit = SubKind::from_id_str(sub_kind).is_some_and(|k| k.is_kit());
                    self.kit_out.push(KitSpawn {
                        owner: (*team, *owner_slot),
                        kind: if kit {
                            sub_kind.clone()
                        } else {
                            "subs".to_string()
                        },
                        sub_kind: if kit { None } else { Some(sub_kind.clone()) },
                        data: data.clone(),
                    });
                }
                SimEvent::SubEnd {
                    owner_slot,
                    team,
                    sub_kind,
                    data,
                } => {
                    self.kit_out.push(KitSpawn {
                        owner: (*team, *owner_slot),
                        kind: sub_kind.clone().unwrap_or_else(|| "subs".to_string()),
                        sub_kind: None,
                        data: data.clone(),
                    });
                }
                SimEvent::DeviceHit {
                    owner_slot,
                    team,
                    kind,
                    gid,
                    d,
                } => {
                    self.dh_out.push(DeviceHitOut {
                        owner: (*team, *owner_slot),
                        kind: kind.clone(),
                        gid: *gid,
                        d: *d,
                    });
                }
                _ => {}
            }
        }

        // ---- actor events → kill log + splat bus (JS `_onSplatted`).
        // Drain into a local list first: the bus push must not overlap the
        // `actors.iter_mut()` borrow. The splat carries its attacker credit
        // (weapon kill → the shooter; water → `lastAttacker` while
        // `lastDamage < 4`, JS actor.js L384).
        let mut splatted: Vec<SplatRecord> = Vec::new();
        let mut respawns: Vec<(usize, usize)> = Vec::new();
        for a in self.actors.iter_mut() {
            for e in a.drain_events() {
                match e {
                    ActorEvent::Splatted { cause, attacker } => {
                        splatted.push((a.team, a.slot, cause, attacker));
                    }
                    ActorEvent::Respawn => respawns.push((a.team, a.slot)),
                    _ => {}
                }
            }
        }
        for (team, slot) in respawns {
            self.events.push(MatchEvent::Respawn { team, slot });
        }
        for (vt, vs, cause, attacker) in splatted {
            // JS `splat(attacker)` attacker branch (actor.js L225-230):
            // `attacker.stats.splats++` then the splash splat at the victim,
            // credited through `attacker.addTurf` (its 'turf' emit lands
            // before 'splatted' — kept on the bus).
            if let Some((at, aslot)) = attacker
                && let (Some(ai), Some(vi)) = (
                    find_actor(&self.actors, at, aslot),
                    find_actor(&self.actors, vt, vs),
                )
            {
                self.actors[ai].splats += 1;
                let vp = self.actors[vi].pos;
                let seed = self.rng.next_f32();
                let area = self.paint.splat(
                    world.collision,
                    vec3(vp.x, vp.y + SPLAT_SPLASH_Y, vp.z),
                    SPLAT_SPLASH_R,
                    at,
                    &SplatOpts {
                        seed,
                        ..SplatOpts::default()
                    },
                );
                if area > 0.0 {
                    self.actors[ai].turf += area;
                    self.events.push(MatchEvent::Turf {
                        owner_team: at,
                        owner_slot: aslot,
                        area,
                    });
                }
            }
            self.kills.push(KillLog {
                t: self.elapsed(),
                victim_team: vt,
                victim_slot: vs,
                attacker,
                cause,
            });
            self.events.push(MatchEvent::Splat {
                victim_team: vt,
                victim_slot: vs,
                attacker,
                cause,
            });
        }
    }

    /// JS `removeActor(a)` (match.js L169-182, the humans-only `noBots`
    /// leave path): drop the actor and its runner from the roster. The FX /
    /// audio / `actor:removed` bus lines are presentation-only and stay in
    /// JS; the sim just splices both parallel arrays and resets the runner
    /// (`a.weaponRunner.reset()`, L180).
    pub fn remove_actor(&mut self, idx: usize) {
        if idx >= self.actors.len() {
            return;
        }
        self.actors.remove(idx);
        // The runner is dropped with the actor — JS `a.weaponRunner.reset()`
        // (L180) only releases JS-side references; the Rust runner owns none.
        self.runners.remove(idx);
        // Keep the loadout list parallel with the roster (spec D-2).
        if idx < self.kinds.len() {
            self.kinds.remove(idx);
        }
    }

    /// JS `_judge`: coverage → winner (seeded tie-break) → `judge` phase.
    fn judge(&mut self) {
        let cov = self.paint.coverage();
        let winner = if cov[0] == cov[1] {
            if self.rng.next_f32() < 0.5 { 0 } else { 1 }
        } else if cov[0] > cov[1] {
            0
        } else {
            1
        };
        let mut points = [0.0f32; 2];
        for a in &self.actors {
            points[a.team] += a.turf;
        }
        points[0] *= self.points_per_m2;
        points[1] *= self.points_per_m2;
        self.result = Some(MatchResult {
            coverage: cov,
            winner,
            points,
            elapsed: self.elapsed(),
        });
        self.set_phase(Phase::End);
    }

    /// JS `_credit(p, area)` → `owner.addTurf(area)`: resolve the identity
    /// `(team, owner_slot)` payload to a roster index and credit the actor
    /// (spec: 比赛事件总线 — splat / 比分变化).
    fn route_turf(&mut self, events: &[SimEvent]) {
        for e in events {
            let (owner_slot, team, area) = match e {
                SimEvent::Impact {
                    owner_slot,
                    team,
                    area,
                    ..
                } => (*owner_slot, *team, *area),
                SimEvent::Turf {
                    owner_slot,
                    team,
                    area,
                } => (*owner_slot, *team, *area),
                _ => continue,
            };
            if area <= 0.0 {
                continue;
            }
            if let Some(idx) = find_actor(&self.actors, team, owner_slot) {
                self.actors[idx].turf += area;
                self.events.push(MatchEvent::Turf {
                    owner_team: team,
                    owner_slot,
                    area,
                });
            }
        }
    }

    /// Fixed-step driver: run [`Match::step`] at 60 Hz until `target` seconds
    /// of match time have elapsed or the match settles (whichever comes
    /// first). Shared by bots, headless validation and the future netcode
    /// (spec: `step_until`). A paused match never advances — the loop exits
    /// immediately rather than spinning.
    pub fn step_until(
        &mut self,
        target: f32,
        inputs: &[ActorInput],
        world: &SimWorld,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
    ) {
        while !self.paused && self.elapsed() < target && self.phase != Phase::End {
            self.step(crate::actor::FIXED_DT, inputs, world, t, w, sb);
        }
    }
}

/// Resolve an identity `(team, slot)` payload to a roster index.
#[must_use]
pub fn find_actor(actors: &[Actor], team: usize, slot: usize) -> Option<usize> {
    actors.iter().position(|a| a.team == team && a.slot == slot)
}

/// JS `Match.update` L231-244: pairwise horizontal separation push (both
/// sides give way equally — offline, every actor is local).
///
/// Soft push with the online mask (JS L239): `ka = a.remote ? 0 :
/// b.remote ? 1 : 0.5` — a proxy never gives way (its owner's word is the
/// position), the local side absorbs the whole push. `None` = offline.
fn soft_push_masked(actors: &mut [Actor], remote: Option<&[bool]>, t: &PlayerTuning) {
    let is_remote = |i: usize| remote.is_some_and(|m| m.get(i).copied().unwrap_or(false));
    let r = t.radius * SOFT_PUSH_R;
    for i in 0..actors.len() {
        for j in i + 1..actors.len() {
            let (a, b) = (&actors[i], &actors[j]);
            if !a.alive || !b.alive {
                continue;
            }
            let dx = b.pos.x - a.pos.x;
            let dz = b.pos.z - a.pos.z;
            let dy = b.pos.y - a.pos.y;
            let d2 = dx * dx + dz * dz;
            if d2 < r * r && dy.abs() < SOFT_PUSH_DY && d2 > 1e-5 {
                let d = d2.sqrt();
                let push = r - d;
                // JS L239: `ka = a.remote ? 0 : b.remote ? 1 : 0.5` (and the
                // mirror for kb) — both proxies push nobody (the owner's next
                // sample redraws them anyway).
                let (ri, rj) = (is_remote(i), is_remote(j));
                let ka = if ri {
                    0.0
                } else if rj {
                    1.0
                } else {
                    SOFT_PUSH_K
                };
                let kb = if rj {
                    0.0
                } else if ri {
                    1.0
                } else {
                    SOFT_PUSH_K
                };
                let (ax, az) = (actors[i].pos.x, actors[i].pos.z);
                let (bx, bz) = (actors[j].pos.x, actors[j].pos.z);
                actors[i].pos.x = ax - (dx / d) * push * ka;
                actors[i].pos.z = az - (dz / d) * push * ka;
                actors[j].pos.x = bx + (dx / d) * push * kb;
                actors[j].pos.z = bz + (dz / d) * push * kb;
            }
        }
    }
}

/// Spec TR-8.2 intro confinement: keep an actor inside `spawn_barrier` of its
/// own pad (deviation — see module docs). The enemy-pad keep-out stays the
/// always-on `Actor::_spawnBarrier` (already inside `Actor::step`).
fn confine_to_own_pad(a: &mut Actor, world: &SimWorld) {
    let pad = world.spawn_pads[a.team];
    let r = world.spawn_barrier;
    let dx = a.pos.x - pad.x;
    let dz = a.pos.z - pad.z;
    let d = dx.hypot(dz);
    if d > r {
        let inv = r / d.max(1e-4);
        a.pos.x = pad.x + dx * inv;
        a.pos.z = pad.z + dz * inv;
        // drop the outward radial velocity so the hold sticks
        let ux = dx / d.max(1e-4);
        let uz = dz / d.max(1e-4);
        let vr = a.vel.x * ux + a.vel.z * uz;
        if vr > 0.0 {
            a.vel.x -= ux * vr;
            a.vel.z -= uz * vr;
        }
    }
}

#[cfg(test)]
mod tests;
