//! Turf-war match flow (sim layer): lifecycle, roster, scoring, fixed-step API.
//!
//! Faithful port of `src/game/match.js` (Turf War mode) plus the splat
//! attribution hooks of `src/game/actor.js` / `src/game/weapons.js` that the
//! weapon layer deferred here in Task 7:
//!   - `Match` constructor / `setup()` roster + spawn placement -> [`Match::with_roster`]
//!   - `Match.start` / `setState`                               -> [`Match::start`] / [`Match::set_phase`]
//!   - `Match.update` state machine (intro 4.2 s → playing clock → finish
//!     2.6 s → judge) and the bot-intent zeroing (L220-226)      -> [`Match::step`]
//!   - `Match.update` soft push between actors (L231-244)        -> [`soft_push`]
//!   - `Match._judge` (coverage, winner, tie-break)              -> [`Match::judge`]
//!   - `Match._onSplatted` kill log                              -> [`Match::step`] actor drain
//!   - `Actor.splat` attacker `stats.splats++` + splash splat     -> [`Match::step`] splat drain
//!     (actor.js L225-230)
//!   - `Projectiles._credit` → `Actor.addTurf`                    -> [`Match::route_turf`]
//!
//! Out of scope (M1, per PORT_MAP): `zones` / `boss` / `practice` / `attract`
//! modes, online roster (`_setupRoster`), `PlayerController` / `BotBrain`
//! wiring (step inputs come from the caller — Task 9 autopilot), `teamSummary`
//! HUD glue, `removeActor` (netcode).
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

use crate::actor::{Actor, ActorEvent, ActorInput, FireGate, SimWorld, SplatCause};
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{MatchConfig, PlayerTuning, Spritzer};
use crate::weapon::{ProjectileSim, Rng, SimEvent, WeaponRunner};
use glam::vec3;

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
    /// Clock crossed 60 s in a match longer than 60 s (JS `match:oneminute`).
    OneMinute,
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
            paint: PaintGrid::new(world.collision),
            projectiles: ProjectileSim::new(seed),
            rng: Rng::new(seed ^ 0x5bd1_e995),
            seed,
            final_countdown: mc.final_countdown as f32,
            points_per_m2: mc.points_per_m2,
            result: None,
            events: Vec::new(),
            kills: Vec::new(),
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
    fn set_phase(&mut self, p: Phase) {
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
        w: &Spritzer,
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
                    self.set_phase(Phase::Finish);
                }
            }
            Phase::Finish => {
                if self.state_t > FINISH_TIME {
                    self.judge();
                }
            }
            Phase::End => {}
        }

        // ---- actors. JS steps bots with zeroed intents outside `playing`
        // (L220-226) and the local controller is disabled the same way; the
        // sim has no controller, so non-live steps get idle input.
        let live = self.phase == Phase::Active;
        let intro = self.phase == Phase::Intro;
        let can_respawn = self.can_respawn();
        for (i, a) in self.actors.iter_mut().enumerate() {
            let inp = if live {
                inputs.get(i).copied().unwrap_or_default()
            } else {
                ActorInput::default()
            };
            // Respawn gating (see module docs): hold dead timers above the
            // step while the phase does not allow respawning.
            if !a.alive && !can_respawn && a.respawn_timer <= dt {
                a.respawn_timer = dt * 1.5;
            }
            a.step(dt, &inp, world, &self.paint, t);
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
        for (a, r) in self.actors.iter_mut().zip(self.runners.iter_mut()) {
            if !a.alive {
                r.reset();
                a.fire_gate = FireGate::default();
                continue;
            }
            let gate = a.fire_gate;
            r.update(dt, &gate, a, w, &mut self.projectiles);
        }

        // ---- soft push between actors (JS match.js L231-244; offline every
        // actor is local, so both sides give way with weight 0.5).
        soft_push(&mut self.actors, t);

        // ---- projectiles (JS main.js L1119: after `match.update`).
        self.projectiles
            .step(dt, world.collision, &mut self.actors, &mut self.paint, t);

        // ---- scoring drains (JS `on('hit'/'weapon:impact')` + `_credit`).
        let pev = self.projectiles.drain_events();
        self.route_turf(&pev);

        // ---- actor events → kill log + splat bus (JS `_onSplatted`).
        // Drain into a local list first: the bus push must not overlap the
        // `actors.iter_mut()` borrow. The splat carries its attacker credit
        // (weapon kill → the shooter; water → `lastAttacker` while
        // `lastDamage < 4`, JS actor.js L384).
        let mut splatted: Vec<SplatRecord> = Vec::new();
        for a in self.actors.iter_mut() {
            for e in a.drain_events() {
                if let ActorEvent::Splatted { cause, attacker } = e {
                    splatted.push((a.team, a.slot, cause, attacker));
                }
            }
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
        w: &Spritzer,
    ) {
        while !self.paused && self.elapsed() < target && self.phase != Phase::End {
            self.step(crate::actor::FIXED_DT, inputs, world, t, w);
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
fn soft_push(actors: &mut [Actor], t: &PlayerTuning) {
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
                let (ax, az) = (actors[i].pos.x, actors[i].pos.z);
                let (bx, bz) = (actors[j].pos.x, actors[j].pos.z);
                actors[i].pos.x = ax - (dx / d) * push * SOFT_PUSH_K;
                actors[i].pos.z = az - (dz / d) * push * SOFT_PUSH_K;
                actors[j].pos.x = bx + (dx / d) * push * SOFT_PUSH_K;
                actors[j].pos.z = bz + (dz / d) * push * SOFT_PUSH_K;
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
