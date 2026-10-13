//! Task 21/22 — authoritative sharding, host duties and the playback layer
//! (replication + [`super::playback`]).
//!
//! Faithful port of the *ownership / routing / host-control* half of
//! `src/net/netmatch.js`; the playback half (per-sender clocks, Hermite
//! interpolation, corrections — Task 22) lives in [`super::playback`] and is
//! driven through [`Replication::playback`]. The event timeline (Task 23) is
//! the remaining seam (`out`, the per-peer `events` queue).
//!
//! Mapping (line numbers are the JS source):
//!   - ownership mask `a.remote = r.owner !== me` (match.js L110) ->
//!     [`Replication::remote`] fed to [`crate::match_::Match::step_masked`]
//!   - `_sendTick` (L190-203, 20 Hz, host clock snapshot `c` every 0.5 s) ->
//!     [`Replication::update`]
//!   - `sendHit` (L163-167) / `_hit` (L630-636) -> the `MatchEvent::HitSend`
//!     drain in [`Replication::update`] and the `'hit'` case of
//!     [`Replication::on_message`]
//!   - `onMessage` switch (L206-221) -> [`Replication::on_message`]
//!   - `_hostClock` / `_hostState` / `_result` / `sendResult` / `sendEnd`
//!     (L639-666) -> the `'t'` / `'st'` / `'res'` / `'end'` cases and
//!     [`Replication::send_result`] / [`Replication::send_end`]
//!   - `onLeave` / `_adopt` / `_remove` (L671-707, match.js L169-182) ->
//!     [`Replication::on_leave`]
//!   - bot driving for owned + adopted actors (match.js L222-224) ->
//!     [`Replication::drive_bots`] (the autopilot snapshot pattern, Task 9)
//!   - `_tick` clock bookkeeping / `_advance` / `_pathAt` / `_sample` /
//!     `applyRemote` / `hermite` / `quantile` (L229-346, L349-428, L761-786)
//!     -> [`super::playback`], driven by [`Replication::update`] (frame
//!     order: `now` → advance → sample → record ink → play events → tick
//!     timer → event bus → apply, matching JS `NetMatch.update` before
//!     `Match.update`) and [`Replication::on_tick`]
//!   - event timeline recording / replay (Task 23): local splats, shots,
//!     triggers and `splatted` / `respawn` events are recorded onto the
//!     outgoing tick `e` (JS `_rec` / `recSplat` / `recProj` /
//!     `_onLocalEvent`, L107-143) by [`Replication::record_ink`] and the bus
//!     drain; received events play on the sender timeline (JS `_playEvents` /
//!     `_play` / `_playEvent` / `_remoteSplat` / `_remoteRespawn`,
//!     L447-619) via [`Replication::play_events`]
//!
//! Scope and deviations (recorded for review):
//!   - **Direct time source**: JS stamps ticks with `now() =
//!     performance.now()/1000`; the sim has no wall clock, so the replicator
//!     accumulates its own logical seconds (`now += dt`, f64 — JS numbers
//!     are f64 end to end) and feeds it to the playback clock. Rust↔Rust
//!     only — both ends use the same rule, and only time *differences*
//!     (`o = now − ts`, `target = now − off − delay`) ever matter.
//!   - **Proxy state comes off the playback timeline** (Task 22): received
//!     samples enter the per-actor buffer; [`super::playback::Playback::apply`]
//!     writes position (path + correction), velocity, facing, flags,
//!     hp/ink/invuln/turf onto the proxy [`Actor`] each frame. JS app-layer
//!     fields the sim does not model (`hurtFlash`, `aimDir`/`aimPoint`,
//!     climbing `wallN`, special/super-jump/kit pose state, FX/audio) are
//!     skipped — see the playback module docs for the line-by-line list.
//!   - **Death / respawn are event-driven** (Task 23, closing the Task 22
//!     ALIVE-edge deviation): the victim's owner records `'ev splatted'` /
//!     `'ev respawn'` on its timeline (JS `_onLocalEvent`, L139-143) and every
//!     receiver runs [`super::playback::Playback::remote_splat`] /
//!     [`super::playback::Playback::remote_respawn`] when the event matures
//!     (JS `_remoteSplat` / `_remoteRespawn`, L603-619). [`super::playback::Playback::apply`]
//!     no longer touches the ALIVE edge. A player who dies and respawns
//!     between two samples is now handled exactly like JS (the events land in
//!     order on the sender timeline).
//!   - **`w:'shot'`**: the JS shooter round reports `p.weaponId || p.wid ||
//!     p.type` (weapons.js L1608) and the M1 Spritzer has no `weaponId`/`wid`,
//!     so the wire id is `'shot'` — the victim's owner only forwards it to
//!     FX/audio (M1 has no kit damage modifiers reading `w`).
//!   - **`dh` / `bhit`**: M2 is turf-only with no devices/boss — received as
//!     ignored stubs (JS L210-215), per the spec's 收口要求.
//!   - **Adopted-bot difficulty**: JS `new BotBrain(a, cfg.difficulty ||
//!     'normal')` (L698); the embedded tuning ships only `easy`
//!     ([`crate::tuning::DifficultySet`]), so adopted bots always get `easy`.
//!   - **`spawnPending` / `handoff`**: implemented in [`super::playback`]
//!     (JS L679 `buf.length = 0; handoff = true`, L704 mid-respawn adopt
//!     finishes the respawn locally). [`Replication::on_leave`] hands the
//!     buffer clear + glide flag to [`super::playback::Playback::handoff`];
//!     [`Replication::adopt`] continues the teleport counter from the
//!     *shown* `tp` (JS `a.net.tp`, L702) via
//!     [`super::playback::Playback::adopt_reset`].
//!   - **Guest result `points`/`elapsed`**: the wire `{k:'res'}` carries only
//!     `cov/win/st` (L652-654); the Rust [`crate::match_::MatchResult`] adds
//!     the spec score line, so guests rebuild `points` from the stat rows
//!     (`Σ turf × pointsPerM2`) and `elapsed` from the duration (the clock is
//!     hard-set to 0, L660).
//!   - **`{k:'own'}`**: reserved stub on both sides (JS `_ownership` L709 is
//!     empty, the host never sends it) — received and ignored.
//!   - **`net.tp` initial value**: JS seeds `a.net.tp = -1` (L85) and copies
//!     `a.netTp = a.net.tp || 0` (L702), so a never-respawned actor reports
//!     `-1` (falsy → 0) until its first teleport; the Rust [`Actor`] starts
//!     `net_tp` at 0 and bumps it on respawn. The counter only gates the
//!     Task 22 path-cut, so the absolute base is irrelevant — Rust↔Rust both
//!     ends agree.
//!   - **HitSend timing**: JS calls `sendHit` inline the frame the projectile
//!     lands (weapons.js L1608 path); the Rust sim surfaces the same hit as a
//!     [`crate::match_::MatchEvent::HitSend`] drained by the *next*
//!     [`Replication::update`], so the `{k:'hit'}` frame can leave up to one
//!     frame (1/60 s) later. The victim's owner applies it on arrival either
//!     way.

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, ActorInput, FIXED_DT, SimWorld};
use crate::bot::{Bot, BotCtx};
use crate::match_::{Match, MatchEvent, MatchResult, Phase, find_actor};
use crate::nav::NavGraph;
use crate::net::playback::Playback;
use crate::net::session::{StartCfg, map_no_bots};
use crate::net::transport::Transport;
use crate::net::wire::{
    self, JVal, decode_direct, decode_host, decode_tick, encode_direct, encode_host, encode_tick,
    parse_json,
};
use crate::net::{
    ActorSample, ActorStatsRow, DirectMsg, EventRecord, ForwardEvent, HostMsg, ProjectileRecord,
    SessionId, SplatRecord, Tick, TickEvent, TimedEvent, TrigData, TriggerRecord,
};
use crate::paint::{SplatKind, SplatOpts};
use crate::tuning::{Difficulty, PlayerTuning, Spritzer};
use crate::weapon::{PROJ_DRAG, PROJ_GRAV, PROJ_SIZE, SHOT_LIFE};
use glam::{Vec3, vec3};

/// JS `TICK = 1/20` (netmatch.js L30): owners stream every 1/20 s.
pub const TICK_DT: f32 = 1.0 / 20.0;
/// JS host clock snapshot period (`clockT = 0.5`, L199).
pub const CLOCK_PERIOD: f32 = 0.5;

/// JS `match.state` strings (match.js) ↔ [`Phase`]. The wire carries the JS
/// spelling (`{k:'st', s:'playing'}`, netmatch.js L80); `End` is JS `judge`.
#[must_use]
pub fn phase_str(p: Phase) -> &'static str {
    match p {
        Phase::Init => "init",
        Phase::Intro => "intro",
        Phase::Active => "playing",
        Phase::Finish => "finish",
        Phase::End => "judge",
    }
}

/// [`Phase`] from a wire state string (`None` for unknown strings — JS's
/// `setState` would accept them, the sim keeps the enum closed).
#[must_use]
pub fn phase_of(s: &str) -> Option<Phase> {
    Some(match s {
        "init" => Phase::Init,
        "intro" => Phase::Intro,
        "playing" => Phase::Active,
        "finish" => Phase::Finish,
        "judge" => Phase::End,
        _ => return None,
    })
}

/// Something the app layer must react to (JS `G.game?.netMatchEnd?.()`,
/// netmatch.js L218 — the host's `{k:'end'}` returns everyone to the lobby).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationEvent {
    /// The host ended the match — call
    /// [`crate::net::session::NetSession::end_match`] and tear down.
    MatchEnd,
}

/// The replication controller for one client of one match: who owns which
/// squidkid, the 20 Hz tick stream, the hit channel, the host's clock/state/
/// result broadcasts, and leave-time adoption / host migration.
///
/// All roster-parallel vectors ([`Replication::nids`], [`Replication::owners`],
/// [`Replication::remote`], …) are indexed by the *match actor index* — the
/// roster order the [`Match`] was built from (match.js `_setupRoster` L107-113
/// pushes actors in roster order, so the arrays stay aligned;
/// [`Replication::on_leave`] prunes them together with [`Match::remove_actor`]).
pub struct Replication {
    /// Our relay session id (`this.myId`, netmatch.js L63).
    pub my_id: SessionId,
    /// The current host (`this.s.hostId`). The app updates this from the
    /// session's `Host` / `Leave` events **before** calling
    /// [`Replication::on_leave`] (JS session.js L153-157 re-elects, then
    /// `netm.onLeave` runs).
    pub host_id: SessionId,
    /// Roster-order actor nids (JS `a.nid`, match.js L110).
    pub nids: Vec<u8>,
    /// Roster-order owner session ids (JS `a.owner`).
    pub owners: Vec<SessionId>,
    /// The ownership mask (JS `a.remote`, match.js L110) — pass to
    /// [`Match::step_masked`], or read via [`Replication::remote_mask`].
    pub remote: Vec<bool>,
    /// `true` once a leave made this client the host (drove
    /// `m.follower = false`, L681). Introspection only.
    pub adopted_host: bool,
    /// Adopted / roster bots by actor index (JS `a.bot`, match.js L112).
    /// Only ever `Some` for *owned* actors — the host runs the bots. Roster
    /// bots need [`Replication::attach_bot`] once the actor exists; adopted
    /// bots get their brain inside [`Replication::on_leave`].
    bots: Vec<Option<Bot>>,
    /// The playback layer (Task 22): per-sender clocks, per-actor sample
    /// buffers, Hermite paths and corrections. Roster-parallel like the
    /// arrays above (pruned together in [`Replication::remove_at`]).
    playback: Playback,
    /// Outgoing event timeline (JS `this.out`, L107) — filled by the local
    /// recorder (Task 23) and shipped non-empty as the tick's `e`.
    pub out: Vec<TimedEvent>,
    /// How many entries of [`crate::paint::PaintGrid::ink_stream`] this
    /// client has already recorded onto `out`. The cursor advances past the
    /// local splats each frame and skips the replayed ones — the Rust
    /// stand-in of JS's `applying` loop guard (`recSplat`, L110: a splat
    /// replayed from a peer's timeline is never re-recorded and never
    /// re-broadcast).
    recorded_ink: usize,
    /// JS `tickT` (L57, starts 0 → the first frame sends).
    tick_t: f32,
    /// JS `clockT` (L61, starts 0 → the first host tick carries `c`).
    clock_t: f32,
    /// Logical sender clock, seconds (see module docs).
    now: f64,
    /// The humans-only stage flag (JS `mapNoBots(this.cfg.map)`, L673).
    no_bots: bool,
}

impl Replication {
    /// Bind to a started match: the roster decides ownership — JS
    /// `_setupRoster` (match.js L104-113). Roster bots (owned, `bot: true`)
    /// still need [`Replication::attach_bot`] once the [`Match`] actors
    /// exist (JS builds the brain right after the actor, match.js L112);
    /// [`Replication::bind_match`] applies the host/guest flag.
    #[must_use]
    pub fn new(cfg: &StartCfg, my_id: &str, host_id: &str) -> Self {
        let n = cfg.roster.len();
        Self {
            my_id: my_id.to_string(),
            host_id: host_id.to_string(),
            nids: cfg.roster.iter().map(|r| r.nid).collect(),
            owners: cfg.roster.iter().map(|r| r.owner.clone()).collect(),
            remote: cfg.roster.iter().map(|r| r.owner != my_id).collect(),
            adopted_host: false,
            bots: (0..n).map(|_| None).collect(),
            playback: Playback::new(n, TICK_DT as f64),
            out: Vec::new(),
            recorded_ink: 0,
            tick_t: 0.0,
            clock_t: 0.0,
            now: 0.0,
            no_bots: map_no_bots(&cfg.map),
        }
    }

    /// Apply the roster to a freshly built [`Match`]: the host/guest flag
    /// (JS `this.follower = !o.host`, match.js L106).
    pub fn bind_match(&self, m: &mut Match) {
        m.follower = !self.is_host();
    }

    /// JS `bind` L79: on a humans-only stage, an actor whose owner left
    /// *while the match was loading* (before a NetMatch existed to hear the
    /// leave frame) is dropped now. The app passes the session's current
    /// member ids (`s._members`, session.js L788); own actors are never
    /// pruned. No-op on stages that keep bots.
    pub fn prune_loading_leavers(&mut self, m: &mut Match, members: &[SessionId]) {
        if !self.no_bots {
            return;
        }
        // Iterate high→low: pruning shifts the parallel arrays.
        for i in (0..self.owners.len()).rev() {
            if self.owners[i] != self.my_id && !members.iter().any(|x| *x == self.owners[i]) {
                self.remove_at(m, i);
            }
        }
    }

    /// Drop actor `i` from the match and every roster-parallel array
    /// (JS `_remove`, L686-690).
    fn remove_at(&mut self, m: &mut Match, i: usize) {
        m.remove_actor(i);
        self.nids.remove(i);
        self.owners.remove(i);
        self.remote.remove(i);
        self.bots.remove(i);
        self.playback.remove_at(i);
    }

    /// Attach the brain for roster bot `idx` (JS match.js L112). Only valid
    /// for owned, non-remote actors — the host never follows its own bots.
    pub fn attach_bot(&mut self, idx: usize, a: &Actor, diff: &Difficulty, seed: u64) {
        if idx < self.nids.len() && !self.remote[idx] {
            self.bots[idx] = Some(Bot::new(a, diff, seed, idx));
        }
    }

    #[must_use]
    pub fn is_host(&self) -> bool {
        self.my_id == self.host_id
    }

    /// The [`Match::step_masked`] mask.
    #[must_use]
    pub fn remote_mask(&self) -> &[bool] {
        &self.remote
    }

    #[must_use]
    pub fn owner_of(&self, idx: usize) -> Option<&str> {
        self.owners.get(idx).map(String::as_str)
    }

    #[must_use]
    pub fn nid_of(&self, idx: usize) -> Option<u8> {
        self.nids.get(idx).copied()
    }

    /// Roster index by `nid` (JS `byNid`, netmatch.js L74).
    #[must_use]
    pub fn idx_of_nid(&self, nid: u8) -> Option<usize> {
        self.nids.iter().position(|&n| n == nid)
    }

    /// The last *shown* teleport counter for an actor (JS `a.net.tp`, the
    /// playback path's current `tp`).
    #[must_use]
    pub fn last_tp(&self, idx: usize) -> u32 {
        self.playback.tp_shown(idx)
    }

    /// The playback layer (read-only view for tests / the app: buffer depth,
    /// shown `tp`, proxy visibility, [`Playback::proxy_shown`]).
    #[must_use]
    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// The app's host view changed (session `Host` / `Leave` events).
    pub fn set_host(&mut self, host_id: &str) {
        self.host_id = host_id.to_string();
    }

    /// One 60 Hz frame of replication (JS `NetMatch.update`, L172-188, then
    /// the `Match.update` → `applyRemote` consumption, match.js L228):
    /// advance every sender's playback clock, sample every remote actor's
    /// path, **replay the due event-timeline records** (JS `_playEvents`,
    /// L179 — before the tick send, exactly like JS), run the 20 Hz tick
    /// timer (owners stream their actors + the locally recorded timeline;
    /// the host rides its `c` clock snapshot every 0.5 s), drain the match
    /// event bus — the host re-broadcasts phase changes (`{k:'st'}`, L80),
    /// every client routes proxy hits to the victim's owner (`{k:'hit'}`,
    /// L163-167) and records the local splats / shots / triggers / deaths /
    /// respawns onto the outgoing timeline (Task 23) — then apply the
    /// sampled timeline onto the proxies.
    ///
    /// Returns the events the replicator did *not* consume (Splat / Turf /
    /// Countdown / OneMinute — HUD consumers; Fire / Respawn / HitSend are
    /// recorded / routed here).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        m: &mut Match,
        sim: &SimWorld,
        t: &PlayerTuning,
        w: &Spritzer,
        tr: &mut impl Transport,
    ) -> Vec<MatchEvent> {
        self.now += dt as f64;

        // ---- playback clocks + path sampling (JS L176-177, before the
        // tick send so the frame's samples ride the *next* packet's
        // timeline, exactly like `update`'s ordering).
        let dt64 = dt as f64;
        let gravity = t.gravity as f64;
        self.playback.advance_all(self.now, dt64);
        {
            let playback = &mut self.playback;
            let owners = &self.owners;
            for (i, &rem) in self.remote.iter().enumerate() {
                if rem {
                    playback.sample(i, &owners[i], dt64, gravity);
                }
            }
        }

        // ---- record the local splats painted since the last frame (JS
        // `paint.splat` → `recSplat`, paint.js L469-472). Runs *before* the
        // replay so the cursor never counts a replayed splat — the Rust
        // stand-in for JS's `applying` guard (see `play_events`).
        self.record_ink(m);

        // ---- event timeline replay (JS `_playEvents`, L179: after
        // sampling, before the tick send — so a due splat paints into this
        // frame's grid and a due death already hides the proxy before its
        // sample is applied below).
        self.play_events(m, sim, t);

        // ---- outgoing tick (L182-187: `tickT -= dt; if (<=0) { += TICK;
        // if (<0) = TICK; _sendTick() }`).
        self.tick_t -= dt;
        if self.tick_t <= 0.0 {
            self.tick_t += TICK_DT;
            if self.tick_t < 0.0 {
                self.tick_t = TICK_DT;
            }
            self.send_tick(m, tr);
        }

        // ---- event bus (JS listens on `match:state` L80, `splatted` /
        // `respawn` / `weapon:fire` through the FORWARD subscription
        // L77/L139-143, and calls `sendHit` inline, L163-167; the sim
        // surfaces all of these as bus events drained here).
        let drained = std::mem::take(&mut m.events);
        let mut rest = Vec::new();
        for e in drained {
            match e {
                MatchEvent::Phase(p) => {
                    if self.is_host() {
                        let msg = HostMsg::State {
                            phase: phase_str(p).to_string(),
                            t: Some(m.time),
                        };
                        tr.broadcast(&encode_host(&msg));
                    }
                    rest.push(MatchEvent::Phase(p));
                }
                MatchEvent::HitSend {
                    attacker,
                    victim,
                    damage,
                } => {
                    self.send_hit(m, t, tr, attacker, victim, damage);
                }
                MatchEvent::Fire { owner, muzzle, dir } => {
                    // JS order inside one shot: `recProj` (weapons.js
                    // `_push` → netmatch L116) → `emit('weapon:fire')` →
                    // `_onLocalEvent` (L139) → `character.trigger('shoot')`
                    // (L179, after `fireShooter` returns).
                    self.record_fire(m, w, owner, muzzle, dir);
                }
                MatchEvent::Respawn { team, slot } => {
                    // JS `respawn()` (actor.js L172-175): `trigger('spawn')`
                    // fires *before* `emit('respawn')`, so the timeline
                    // carries `tr spawn` ahead of `ev respawn`.
                    if let Some(i) = find_actor(&m.actors, team, slot)
                        && !self.remote[i]
                    {
                        let nid = self.nids[i];
                        self.rec(TickEvent::Trigger(TriggerRecord {
                            nid,
                            name: "spawn".to_string(),
                            data: TrigData::Zero,
                        }));
                    }
                    self.record_event(m, ForwardEvent::Respawn, team, slot);
                    rest.push(MatchEvent::Respawn { team, slot });
                }
                MatchEvent::Splat {
                    victim_team,
                    victim_slot,
                    attacker,
                    cause,
                } => {
                    // JS `_onLocalEvent` (L139-143): forwarded only for
                    // *local* victims — a proxy's death is decided by its
                    // owner and arrives on the owner's timeline instead.
                    if let Some(vi) = find_actor(&m.actors, victim_team, victim_slot)
                        && !self.remote[vi]
                    {
                        self.record_splat_event(m, vi, attacker, cause);
                    }
                    rest.push(MatchEvent::Splat {
                        victim_team,
                        victim_slot,
                        attacker,
                        cause,
                    });
                }
                other => rest.push(other),
            }
        }

        // ---- apply the sampled path + correction onto the proxies (JS
        // `applyRemote`, L349-428, run by `Match.update` after
        // `NetMatch.update` in the same frame).
        {
            let playback = &mut self.playback;
            for (i, &rem) in self.remote.iter().enumerate() {
                if rem {
                    playback.apply(i, &mut m.actors[i], dt);
                }
            }
        }

        rest
    }

    /// JS `_rec` (L107): prefix the record with this client's logical
    /// sender-clock (`r3(now())`) and queue it onto the outgoing timeline.
    fn rec(&mut self, kind: TickEvent) {
        self.out.push(TimedEvent {
            ts: wire::r3(self.now) as f32,
            kind,
        });
    }

    /// JS `recSplat` (L109-114) for every splat the local sim painted since
    /// the last frame. The replayed (remote) splats are *not* re-recorded:
    /// `play_events` advances this cursor past them before the next call,
    /// the Rust stand-in of JS's `applying` guard (paint.js L471 never
    /// reaches `recSplat` while applying).
    fn record_ink(&mut self, m: &Match) {
        let stream = m.paint.ink_stream();
        let start = self.recorded_ink.min(stream.len());
        for s in &stream[start..] {
            self.rec(TickEvent::Splat(SplatRecord::from_ink_splat(s)));
        }
        self.recorded_ink = stream.len();
    }

    /// JS `recProj` (L116-122) + `_onLocalEvent('weapon:fire')` (L139-143) +
    /// the `character.trigger('shoot')` wrapper (L90-93, weapons.js L179):
    /// one local shot leaves three timeline records, in that order. The
    /// projectile's own fields are rebuilt from the [`Spritzer`] tuning —
    /// the M1 Spritzer round is fully determined by it (`ptype: 'shot'`, no
    /// `wid`, no `delay`, `life = min(range/speed, 1.2)` like the local
    /// `fire_shooter`).
    fn record_fire(
        &mut self,
        m: &Match,
        w: &Spritzer,
        owner: (usize, usize),
        muzzle: Vec3,
        dir: Vec3,
    ) {
        let Some(i) = find_actor(&m.actors, owner.0, owner.1) else {
            return;
        };
        if self.remote[i] {
            return; // JS `recProj`: `if (!o || o.remote …) return`
        }
        let nid = self.nids[i];
        let speed = w.proj_speed;
        self.rec(TickEvent::Proj(ProjectileRecord {
            nid,
            ptype: "shot".to_string(),
            wid: None,
            pos: [muzzle.x, muzzle.y, muzzle.z],
            vel: [dir.x * speed, dir.y * speed, dir.z * speed],
            delay: 0.0,
            life: (w.range / w.proj_speed).min(SHOT_LIFE),
            straight: w.straight_time,
            radius: w.impact_radius,
            size: PROJ_SIZE,
            grav: PROJ_GRAV,
            drag: PROJ_DRAG,
            trail_every: w.trail_every,
            head: false,
            vis: 0.1,
            tail0: 0.8,
            tail_k: 1.3,
            wob: 0.035,
            wob_f: 26.0,
            nose: 0.3,
            sats: 3.0,
        }));
        // `packEvent({actor, weapon, muzzle, dir})` (L803-813): actor →
        // `{n}`, weapon id string passes through, Vector3 → `[r2 ×3]`.
        let payload = JVal::Obj(vec![
            (
                "actor".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(nid)))]),
            ),
            ("weapon".to_string(), JVal::Str(w.id.clone())),
            (
                "muzzle".to_string(),
                JVal::Arr(vec![
                    JVal::Num(wire::r2(f64::from(muzzle.x))),
                    JVal::Num(wire::r2(f64::from(muzzle.y))),
                    JVal::Num(wire::r2(f64::from(muzzle.z))),
                ]),
            ),
            (
                "dir".to_string(),
                JVal::Arr(vec![
                    JVal::Num(wire::r2(f64::from(dir.x))),
                    JVal::Num(wire::r2(f64::from(dir.y))),
                    JVal::Num(wire::r2(f64::from(dir.z))),
                ]),
            ),
        ]);
        self.rec(TickEvent::Event(EventRecord {
            name: ForwardEvent::WeaponFire,
            payload,
        }));
        self.rec(TickEvent::Trigger(TriggerRecord {
            nid,
            name: "shoot".to_string(),
            data: TrigData::Zero,
        }));
    }

    /// JS `_onLocalEvent('splatted')` → `packEvent({victim, attacker,
    /// cause})` (L139-143, L803-813). Key order is the JS object-literal
    /// insertion order (`actor.js` L233): `victim`, `attacker`, `cause`.
    /// `attacker` is `null` for water deaths — `packEvent` drops nulls, so
    /// the key is simply absent (JS L807-810 skips `v == null`).
    fn record_splat_event(
        &mut self,
        m: &Match,
        vi: usize,
        attacker: Option<(usize, usize)>,
        cause: crate::actor::SplatCause,
    ) {
        let mut pairs = vec![(
            "victim".to_string(),
            JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[vi])))]),
        )];
        if let Some((at, aslot)) = attacker
            && let Some(ai) = find_actor(&m.actors, at, aslot)
        {
            pairs.push((
                "attacker".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[ai])))]),
            ));
        }
        pairs.push((
            "cause".to_string(),
            JVal::Str(
                match cause {
                    crate::actor::SplatCause::Weapon => "weapon",
                    crate::actor::SplatCause::Water => "water",
                }
                .to_string(),
            ),
        ));
        self.rec(TickEvent::Event(EventRecord {
            name: ForwardEvent::Splatted,
            payload: JVal::Obj(pairs),
        }));
    }

    /// JS `_onLocalEvent` for a named event whose payload is
    /// `{actor: <local actor>}` (`respawn`, actor.js L175 → `{n: nid}`).
    fn record_event(&mut self, m: &Match, name: ForwardEvent, team: usize, slot: usize) {
        let Some(i) = find_actor(&m.actors, team, slot) else {
            return;
        };
        if self.remote[i] {
            return;
        }
        self.rec(TickEvent::Event(EventRecord {
            name,
            payload: JVal::Obj(vec![(
                "actor".to_string(),
                JVal::Obj(vec![("n".to_string(), JVal::Num(f64::from(self.nids[i])))]),
            )]),
        }));
    }

    /// JS `_playEvents` / `_play` (L452-498): dequeue every sender's due
    /// timeline records and apply them in sender-clock order. `'s'` splats
    /// paint through [`crate::paint::PaintGrid::splat`] (the cursor below
    /// keeps them out of the recorder, JS's `applying` guard stand-in);
    /// `'p'` rounds spawn visual-only ghosts; `'tr'` triggers land on
    /// remote actors only (the sim has no character layer to animate, so
    /// they are consumed — the guard of JS L480 still runs); `'ev'`
    /// dispatches `splatted` / `respawn` into the proxy state machines and
    /// ignores the FX-only names (`weapon:fire` presentation is the `'p'`
    /// ghost + audio, out of sim scope).
    fn play_events(&mut self, m: &mut Match, sim: &SimWorld, t: &PlayerTuning) {
        let due = self.playback.due_events();
        if due.is_empty() {
            return;
        }
        // Replay writes paint + ghosts into the match; JS runs the same
        // mutations inline in `_play`. Our recorder skips the replayed
        // splats via the `recorded_ink` cursor (see `record_ink`): advance
        // it past whatever the replay appends.
        let mut replayed = m.paint.ink_stream().len();
        for (_from, events) in due {
            for ev in events {
                match ev.kind {
                    TickEvent::Splat(s) => {
                        let opts = SplatOpts {
                            seed: s.seed,
                            // JS L468: the stretch vector exists only when
                            // any component is non-zero; `stretchAmt` rides
                            // it (L471), defaulting like `recSplat`'s `?? 1`.
                            stretch: if s.stretch[0] != 0.0
                                || s.stretch[1] != 0.0
                                || s.stretch[2] != 0.0
                            {
                                Some(vec3(s.stretch[0], s.stretch[1], s.stretch[2]))
                            } else {
                                None
                            },
                            stretch_amt: if s.stretch[0] != 0.0
                                || s.stretch[1] != 0.0
                                || s.stretch[2] != 0.0
                            {
                                Some(s.stretch_amt)
                            } else {
                                None
                            },
                            kind: s.kind.as_deref().and_then(SplatKind::from_name),
                        };
                        m.paint.splat(
                            sim.collision,
                            vec3(s.x, s.y, s.z),
                            s.radius,
                            s.team as usize,
                            &opts,
                        );
                        replayed = m.paint.ink_stream().len();
                    }
                    TickEvent::Proj(p) => {
                        // JS `_play` 'p' (L476): `byNid.get(e[2])`, then
                        // `ghostProjectile` — no `remote` guard (a ghost of
                        // your own round can arrive from a peer you adopt
                        // mid-flight; JS lets it die silently).
                        let Some(i) = self.idx_of_nid(p.nid) else {
                            continue;
                        };
                        let owner_team = m.actors[i].team;
                        m.projectiles.list.push(crate::weapon::Projectile {
                            pos: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            prev: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            start: vec3(p.pos[0], p.pos[1], p.pos[2]),
                            vel: vec3(p.vel[0], p.vel[1], p.vel[2]),
                            team: owner_team,
                            owner_slot: m.actors[i].slot,
                            age: 0.0,
                            life: p.life,
                            straight: p.straight,
                            radius: p.radius,
                            // JS `ghostProjectile` L870: `damage: 0` — the
                            // ghost never hurts; its motion comes from the
                            // record's own `grav` / `drag`.
                            damage: 0.0,
                            grav: p.grav,
                            drag: p.drag,
                            trail: -1.5,
                            trail_every: p.trail_every,
                            trail_radius: 0.45,
                            // JS L871: `seed: Math.random()` — the ghost's
                            // paint is muted, so the seed only feeds this
                            // client's own rng stream.
                            seed: m.projectiles.rng.next_f32(),
                            ghost: true,
                        });
                    }
                    TickEvent::Trigger(trig) => {
                        // JS L478-483: the guard runs, then the character
                        // trigger + loop side effects — presentation only,
                        // no sim state in M2 (Spritzer has no charge/spin/
                        // roll loops).
                        let _ = trig;
                    }
                    TickEvent::Event(e) => {
                        self.play_event(m, t, &e);
                    }
                    // M1/M2 placeholders (`b`/`k`/`z`): no kit/bomb/zone
                    // gameplay in scope — JS L477/L486-493 would hand them
                    // to the kit/boss/zones layers, which the sim lacks.
                    _ => {}
                }
            }
        }
        // The replayed splats are presentation for us, not our own
        // timeline — keep the cursor past them (JS `applying` guard).
        self.recorded_ink = self.recorded_ink.max(replayed);
    }

    /// JS `_playEvent` (L573-601): unpack the payload, resolve
    /// `e.actor || e.victim`, require a *remote* actor, then dispatch by
    /// name. `splatted` / `respawn` drive the proxy state machines; the
    /// remaining FORWARD names are FX/audio (out of sim scope) and the
    /// trailing `emit(name, e)` feeds the app bus, not the sim.
    fn play_event(&mut self, m: &mut Match, t: &PlayerTuning, e: &EventRecord) {
        let payload = match &e.payload {
            JVal::Obj(pairs) => pairs,
            _ => return,
        };
        let nid_of = |key: &str| -> Option<u8> {
            payload.iter().find(|(k, _)| k == key).and_then(|(_, v)| {
                let JVal::Obj(inner) = v else { return None };
                inner
                    .iter()
                    .find(|(k, _)| k == "n")
                    .and_then(|(_, n)| n.as_f64())
                    .map(|n| n as u8)
            })
        };
        // `const a = e.actor || e.victim` (L576) — respawn carries `actor`,
        // splatted carries `victim`.
        let Some(a_nid) = nid_of("actor").or_else(|| nid_of("victim")) else {
            return;
        };
        let Some(ai) = self.idx_of_nid(a_nid) else {
            return;
        };
        if !self.remote[ai] {
            return; // JS L577: our own events never replay through us
        }
        match e.name {
            ForwardEvent::Splatted => {
                // JS L580 → `_remoteSplat(e.victim, e.attacker, e.cause)`
                // (L603-619). The `!victim` half of the L604 guard is the
                // nid resolution above (the payload's `victim` is the same
                // actor as `e.victim` for a splat record).
                if self
                    .playback
                    .remote_splat(ai, &mut m.actors[ai], &mut m.runners[ai], t)
                    && let Some(at_nid) = nid_of("attacker")
                    && let Some(ti) = self.idx_of_nid(at_nid)
                    && !self.remote[ti]
                {
                    // JS L613: `attacker.stats.splats++` — the owner that
                    // locally decided the kill also counts it when the
                    // victim's splat record confirms it.
                    m.actors[ti].splats += 1;
                }
            }
            ForwardEvent::Respawn => {
                // JS L581 → `_remoteRespawn(a)` (L623-627).
                self.playback.remote_respawn(ai, &mut m.actors[ai], t);
            }
            // FX/audio-only forwards (jump / superjump / special / dodge /
            // weapon:fire — the ghost round itself rode the `'p'` record).
            _ => {}
        }
    }

    /// JS `_sendTick` (L190-203): pack every **owned** actor
    /// (`if (!x.remote)`, L192), stamp with the sender clock, ride the
    /// pending event timeline, and for the host the 0.5 s clock snapshot
    /// `c = [state, r2(time)]` (L197-200).
    fn send_tick(&mut self, m: &Match, tr: &mut impl Transport) {
        let ts = wire::r3(self.now) as f32;
        let mut actors = Vec::new();
        for (i, (a, wr)) in m.actors.iter().zip(&m.runners).enumerate() {
            if self.remote[i] {
                continue;
            }
            actors.push((self.nids[i], ActorSample::from_actor(a, wr, a.net_tp, ts)));
        }
        let clock = if self.is_host() {
            self.clock_t -= TICK_DT;
            if self.clock_t <= 0.0 {
                self.clock_t = CLOCK_PERIOD;
                Some((phase_str(m.phase), m.time))
            } else {
                None
            }
        } else {
            None
        };
        let tick = Tick {
            ts,
            actors,
            events: std::mem::take(&mut self.out),
        };
        tr.broadcast(&encode_tick(&tick, clock));
    }

    /// JS `sendHit` (L163-167): `{k:'hit', v, a, d: r2(dmg), w}` to the
    /// victim's owner. `victim.owner === myId` cannot happen through the
    /// mask (a proxy is never ours), but JS guards it anyway — we apply the
    /// damage locally instead (the `'local'` route, weapons.js L1552).
    fn send_hit(
        &self,
        m: &mut Match,
        t: &PlayerTuning,
        tr: &mut impl Transport,
        attacker: (usize, usize),
        victim: (usize, usize),
        damage: f32,
    ) {
        let (Some(vi), Some(ai)) = (
            find_actor(&m.actors, victim.0, victim.1),
            find_actor(&m.actors, attacker.0, attacker.1),
        ) else {
            return;
        };
        let d = wire::r2(wire::dec(damage)) as f32;
        if self.owners[vi] == self.my_id {
            m.actors[vi].damage(d, Some(attacker), t);
            return;
        }
        let msg = DirectMsg::Hit {
            v: self.nids[vi],
            a: self.nids[ai],
            d,
            w: Some("shot".to_string()),
        };
        tr.send_to(&self.owners[vi], &encode_direct(&msg));
    }

    /// One inbound relay payload (JS `onMessage`, L206-221). `from` is the
    /// sender's session id as the transport reports it.
    pub fn on_message(
        &mut self,
        from: &str,
        payload: &str,
        m: &mut Match,
        t: &PlayerTuning,
    ) -> Option<ReplicationEvent> {
        let v = parse_json(payload)?;
        let k = v.field("k").and_then(JVal::as_str).unwrap_or("");
        match k {
            "t" => self.on_tick(from, &v, m),
            // The victim's owner applies the damage right away — health must
            // be current, so hits never wait on the playback timeline
            // (netmatch.js L162 comment, `_hit` L630-636).
            "hit" => {
                if let Some(DirectMsg::Hit {
                    v: vn, a: an, d, ..
                }) = decode_direct(&v)
                {
                    self.apply_hit(m, t, vn, an, d);
                }
            }
            // M2 turf-only: no kit devices and no boss — ignored stubs
            // (JS L210-215 runs `netHurt` / `boss.remoteHit`; spec 收口).
            "dh" | "bhit" => {}
            "st" => {
                if from == self.host_id
                    && let Some(HostMsg::State { phase, t: time }) = decode_host(&v)
                {
                    self.host_state(m, &phase, time);
                }
            }
            "res" => {
                if from == self.host_id
                    && let Some(HostMsg::Result {
                        coverage,
                        winner,
                        stats,
                    }) = decode_host(&v)
                {
                    self.host_result(m, coverage, winner, &stats);
                }
            }
            "end" => {
                if from == self.host_id {
                    return Some(ReplicationEvent::MatchEnd);
                }
            }
            // `{k:'own'}`: JS `_ownership` is an empty reserved stub (L709)
            // and the host never sends it — receive and ignore.
            "own" => {}
            _ => {}
        }
        None
    }

    /// JS `_tick` (L229-263): feed the sender's playback clock (offset
    /// floor, the late/gap windows, `want` — [`Playback::on_tick`]), route
    /// the actor samples into their buffers (the `!a || !a.remote || a.owner
    /// !== from` guard, L248, and the monotonic-`t` guard + 40-capacity
    /// trim, L251-253), queue the tick's events on the sender's timeline
    /// (L256, played by Task 23), and steer the guest clock off the host's
    /// `c` (L257).
    fn on_tick(&mut self, from: &str, v: &JVal, m: &mut Match) {
        let (tick, clock) = decode_tick(v);
        self.playback.on_tick(from, tick.ts as f64, self.now);
        self.playback.push_events(from, tick.events);
        if let Some((state, time)) = clock
            && from == self.host_id
        {
            self.host_clock(m, &state, time);
        }
        for (nid, s) in &tick.actors {
            let Some(i) = self.idx_of_nid(*nid) else {
                continue;
            };
            if !self.remote[i] || self.owners[i] != from {
                continue;
            }
            self.playback.push_sample(i, s);
        }
    }

    /// JS `_hostClock` (L639-643): guests only, both sides `playing`, off by
    /// more than 0.2 s → nudge halfway. The clock keeps running locally
    /// between nudges (the sim's `step_masked` decrements it like JS's).
    fn host_clock(&self, m: &mut Match, state: &str, time: f32) {
        if self.is_host() || m.phase != Phase::Active || state != "playing" {
            return;
        }
        if (m.time - time).abs() > 0.2 {
            m.time += (time - m.time) * 0.5;
        }
    }

    /// JS `_hostState` (L644-649): guests only; hard-set the clock **only
    /// when the frame carries a numeric `t`** (`typeof d.t === 'number'`,
    /// L647); re-enter the phase on change, never *into* `judge` (that comes
    /// with `res`).
    fn host_state(&self, m: &mut Match, state: &str, time: Option<f32>) {
        if self.is_host() {
            return;
        }
        if let Some(time) = time {
            m.time = time;
        }
        let Some(p) = phase_of(state) else { return };
        if p != m.phase && p != Phase::End {
            m.set_phase(p);
        }
    }

    /// JS `_result` (L656-665): guests only; per-nid stat overwrite; the
    /// turf clock is pinned to 0; the host's count becomes the local result;
    /// straight into `judge` (bypassing the `st` guard — this *is* the judge
    /// transition).
    fn host_result(&self, m: &mut Match, coverage: [f32; 2], winner: u8, stats: &[ActorStatsRow]) {
        if self.is_host() {
            return;
        }
        for row in stats {
            if let Some(i) = self.idx_of_nid(row.nid) {
                let a = &mut m.actors[i];
                a.turf = row.turf as f32;
                a.splats = row.splats;
                a.deaths = row.deaths;
            }
        }
        // The host's stat rows are the authority for the whole point line
        // (a guest only simulates its own actors — see module docs).
        let mut points = [0.0f32; 2];
        for row in stats {
            if let Some(i) = self.idx_of_nid(row.nid) {
                points[m.actors[i].team] += row.turf as f32;
            }
        }
        points[0] *= m.points_per_m2;
        points[1] *= m.points_per_m2;
        m.time = 0.0;
        m.result = Some(MatchResult {
            coverage,
            winner: winner as usize,
            points,
            elapsed: m.duration,
        });
        m.set_phase(Phase::End);
    }

    /// JS `_hit` (L630-636) + `applyHit` local route (weapons.js L1545-1552):
    /// the victim must exist, be ours (`!remote`), be alive; the attacker
    /// must exist and be on the other team; then `victim.damage(d, attacker)`.
    /// The `w` payload only drives JS FX/audio (M1 has no kit modifiers).
    fn apply_hit(&self, m: &mut Match, t: &PlayerTuning, victim_nid: u8, atk_nid: u8, dmg: f32) {
        let (Some(vi), Some(ai)) = (self.idx_of_nid(victim_nid), self.idx_of_nid(atk_nid)) else {
            return;
        };
        if self.remote[vi] || !m.actors[vi].alive || m.actors[ai].team == m.actors[vi].team {
            return;
        }
        let atk = (m.actors[ai].team, m.actors[ai].slot);
        m.actors[vi].damage(dmg, Some(atk), t);
    }

    /// JS `sendResult` (L650-655): the host's final count, broadcast. The JS
    /// call site is `_judge` (match.js L271) — the app calls this when the
    /// host's match settles; the stat rows ride in roster order.
    pub fn send_result(&self, m: &Match, tr: &mut impl Transport) {
        if !self.is_host() {
            return;
        }
        let Some(r) = m.result else { return };
        let stats = m
            .actors
            .iter()
            .enumerate()
            .map(|(i, a)| ActorStatsRow {
                nid: self.nids[i],
                turf: wire::round_int(a.turf) as i32,
                splats: a.splats,
                deaths: a.deaths,
            })
            .collect();
        let msg = HostMsg::Result {
            coverage: r.coverage,
            winner: r.winner as u8,
            stats,
        };
        tr.broadcast(&encode_host(&msg));
    }

    /// JS `sendEnd` (L666): results shown, everyone back to the lobby.
    pub fn send_end(&self, tr: &mut impl Transport) {
        if self.is_host() {
            tr.broadcast(&encode_host(&HostMsg::End));
        }
    }

    /// JS `onLeave` (L671-684): a member dropped mid-match. Their squidkids
    /// carry on as bots owned by the (possibly new) host — or, on a
    /// humans-only stage, are removed outright (`_remove`, L686-690).
    /// `host_changed` is the leave frame's re-election flag; the app must
    /// have applied [`Replication::set_host`] first. `sim` / `t` drive the
    /// mid-respawn adopt branch (JS L704 `a.respawn()`).
    #[allow(clippy::too_many_arguments)] // JS `onLeave(id, hostChanged)` + the match context it threads
    pub fn on_leave(
        &mut self,
        id: &str,
        host_changed: bool,
        m: &mut Match,
        diff: &Difficulty,
        seed: u64,
        sim: &SimWorld,
        t: &PlayerTuning,
    ) {
        let new_owner = self.host_id.clone();
        // Iterate high→low: the noBots path prunes the parallel arrays.
        for i in (0..self.owners.len()).rev() {
            if self.owners[i] != id {
                continue;
            }
            if self.no_bots {
                self.remove_at(m, i);
                continue;
            }
            // JS L677-679: the squidkid now belongs to the host.
            self.owners[i] = new_owner.clone();
            if new_owner == self.my_id {
                self.adopt(i, m, diff, seed, sim, t);
            } else {
                // Same squidkid, new sender: drop the stale buffer and glide
                // onto the new path (`buf.length = 0; handoff = true`, L679).
                self.playback.handoff(i);
            }
        }
        // JS L681: the new host stops following and re-arms its clock feed.
        if host_changed && self.is_host() {
            m.follower = false;
            self.clock_t = 0.0;
            self.adopted_host = true;
        }
    }

    /// JS `_adopt` (L692-707): continue from exactly where it was drawn —
    /// no pop. `remote = false`, a bot brain takes over, the weapon runner
    /// resets, and the teleport counter continues from what everyone has
    /// seen (`a.netTp = a.net.tp || 0`, L702 — the *shown* counter,
    /// [`Playback::adopt_reset`]). JS L704: a proxy mid-respawn finishes the
    /// respawn locally (the pad drop, the `net_tp` bump); otherwise the
    /// actor simply keeps its drawn position (L705, `a.pos` already holds
    /// path + correction from [`Playback::apply`]).
    fn adopt(
        &mut self,
        i: usize,
        m: &mut Match,
        diff: &Difficulty,
        seed: u64,
        sim: &SimWorld,
        t: &PlayerTuning,
    ) {
        self.remote[i] = false;
        let pending = self.playback.spawn_pending(i);
        let tp = self.playback.adopt_reset(i);
        m.actors[i].net_tp = tp;
        if m.actors[i].alive && pending {
            m.actors[i].respawn(sim, t);
        }
        self.bots[i] = Some(Bot::new(&m.actors[i], diff, seed, i));
        m.runners[i].reset();
        // JS zeroes the intent and clears superJump/specialActive — the sim
        // actor has no intent field (inputs come per-frame) and neither
        // state exists in M1.
    }

    /// Step every owned bot brain (JS `if (a.bot) a.bot.update(dt)`,
    /// match.js L222-224, plus the L224 intent zeroing outside `playing`),
    /// writing into `inputs` for [`Match::step_masked`]. The autopilot
    /// snapshot pattern (Task 9): brains read a roster snapshot taken before
    /// any bot steps.
    pub fn drive_bots(
        &mut self,
        m: &mut Match,
        nav: &NavGraph,
        world: &SimWorld,
        t: &PlayerTuning,
        w: &Spritzer,
        inputs: &mut [ActorInput],
    ) {
        if !self.bots.iter().any(|b| b.is_some()) {
            return;
        }
        let live = m.phase == Phase::Active;
        let snap = m.actors.clone();
        let mut mate_goals = vec![None; m.actors.len()];
        for (i, b) in self.bots.iter().enumerate() {
            if let Some(b) = b {
                mate_goals[i] = b.goal_node();
            }
        }
        for (i, slot) in self.bots.iter_mut().enumerate() {
            // Only owned actors ever hold a brain (module docs).
            if self.remote[i] {
                continue;
            }
            let Some(b) = slot else {
                continue;
            };
            if live {
                let ctx = BotCtx {
                    actors: &snap,
                    nav,
                    paint: &m.paint,
                    world,
                    t,
                    w,
                    mate_goals: &mate_goals,
                };
                inputs[i] = b.step(FIXED_DT, &mut m.actors[i], &ctx);
            } else {
                inputs[i] = b.hold(&m.actors[i]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;
    use crate::geometry::StageLayout;
    use crate::match_::{FINISH_TIME, INTRO_TIME};
    use crate::net::session::RosterEntry;
    use crate::net::transport::{FakeRelay, FakeTransport, TransportEvent};
    use crate::tuning::Tuning;
    use crate::weapon::WeaponRunner;
    use glam::vec3;

    struct Fix {
        layout: StageLayout,
        world: CollisionWorld,
        tuning: Tuning,
    }

    fn fix() -> Fix {
        Fix {
            layout: crate::embedded_tidewater(),
            world: CollisionWorld::tidewater(),
            tuning: crate::embedded_tuning(),
        }
    }

    impl Fix {
        fn sim(&self) -> SimWorld<'_> {
            SimWorld::new(&self.world, &self.layout)
        }
        fn nav(&self) -> NavGraph {
            NavGraph::new(&self.world, &self.layout, &self.tuning.player)
        }
    }

    fn entry(nid: u8, owner: &str, bot: bool, team: u8, slot: u8) -> RosterEntry {
        RosterEntry {
            nid,
            owner: owner.into(),
            bot,
            team,
            slot,
            name: format!("p{nid}"),
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
        }
    }

    fn cfg(roster: Vec<RosterEntry>, map: &str, host: &str) -> StartCfg {
        StartCfg {
            roster,
            map: map.into(),
            time: "day".into(),
            duration: 120,
            difficulty: "normal".into(),
            palette: 0,
            mode: "turf".into(),
            host: host.into(),
            id: "m1".into(),
        }
    }

    /// 1v1 + one host-owned bot, actors in roster order:
    /// `[H(nid0, team0), G(nid1, team1), B(nid2, team1 slot1)]`.
    fn roster111(h: &str, g: &str) -> Vec<RosterEntry> {
        vec![
            entry(0, h, false, 0, 0),
            entry(1, g, false, 1, 0),
            entry(2, h, true, 1, 1),
        ]
    }

    /// A 3-actor match matching [`roster111`]: [`Match::with_roster`] with
    /// team_size 1 plus the bot appended on the team-1 ring.
    fn match111(f: &Fix, seed: u64, duration: f32) -> Match {
        let sim = f.sim();
        let t = &f.tuning.player;
        let mut m = Match::with_roster(duration, seed, &sim, t, &f.tuning.match_config, 1);
        let mut a = Actor::new(1, 1, t);
        let pad = sim.spawn_pads[1];
        let ang = (1.0 / 4.0) * std::f32::consts::TAU + 0.6;
        a.spawn_at(
            vec3(pad.x + ang.cos() * 1.2, pad.y, pad.z + ang.sin() * 1.2),
            std::f32::consts::PI,
            &sim,
            t,
        );
        a.invuln = 0.0;
        m.actors.push(a);
        m.runners.push(WeaponRunner::new());
        m
    }

    /// A [`Transport`] that records framed sends (envelope-split), so the
    /// tick-content checks need no relay semantics.
    #[derive(Default)]
    struct Rec {
        out: Vec<(Option<String>, String)>,
    }
    impl Rec {
        /// The first recorded send whose JSON carries message key `k`.
        fn find(&self, k: &str) -> &(Option<String>, String) {
            self.out
                .iter()
                .find(|(_, p)| {
                    parse_json(p)
                        .and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                        == Some(k.to_string())
                })
                .unwrap_or_else(|| panic!("a {k} frame was recorded"))
        }
    }
    impl Transport for Rec {
        fn connect(&mut self, _code: &str, _name: &str, _create: bool) {}
        fn poll_events(&mut self) -> Vec<TransportEvent> {
            Vec::new()
        }
        fn send_raw(&mut self, s: &str) -> bool {
            if let Some(rest) = s.strip_prefix("b|") {
                self.out.push((None, rest.to_string()));
            } else if let Some(rest) = s.strip_prefix("s|")
                && let Some((to, body)) = rest.split_once('|')
            {
                self.out.push((Some(to.to_string()), body.to_string()));
            } else {
                self.out.push((None, s.to_string()));
            }
            true
        }
        fn is_open(&self) -> bool {
            true
        }
        fn rtt(&self) -> f64 {
            0.0
        }
        fn close(&mut self) {}
    }

    fn ep(relay: &FakeRelay, seed: u64) -> FakeTransport {
        relay.transport(None, seed)
    }

    fn drain_msgs(tr: &mut FakeTransport) -> Vec<(SessionId, String)> {
        tr.poll_events()
            .into_iter()
            .filter_map(|e| match e {
                TransportEvent::Message { from, payload } => Some((from, payload)),
                _ => None,
            })
            .collect()
    }

    /// Both endpoints on one relay, handshaked, all chatter drained.
    fn pair(relay: &FakeRelay) -> (FakeTransport, FakeTransport) {
        let mut a = ep(relay, 1);
        let mut b = ep(relay, 2);
        a.connect("ROOM", "a", true);
        b.connect("ROOM", "b", false);
        for _ in 0..20 {
            a.advance(1.0);
            b.advance(0.0);
        }
        drain_msgs(&mut a);
        drain_msgs(&mut b);
        (a, b)
    }

    // ---------------------------------------------------------------- phase map
    #[test]
    fn phase_strings_match_js_states() {
        // netmatch.js L80/L648: the wire carries match.js state names.
        assert_eq!(phase_str(Phase::Init), "init");
        assert_eq!(phase_str(Phase::Intro), "intro");
        assert_eq!(phase_str(Phase::Active), "playing");
        assert_eq!(phase_str(Phase::Finish), "finish");
        assert_eq!(phase_str(Phase::End), "judge");
        for p in [
            Phase::Init,
            Phase::Intro,
            Phase::Active,
            Phase::Finish,
            Phase::End,
        ] {
            assert_eq!(phase_of(phase_str(p)), Some(p));
        }
        assert_eq!(phase_of("nonsense"), None);
    }

    // ------------------------------------------------------------------ TR-21.1
    #[test]
    fn tr21_1_owner_proxy_split_and_nid_routing() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);

        let mut mh = match111(&f, 7, 60.0);
        let mut mg = match111(&f, 7, 60.0);
        let mut rh = Replication::new(&cfg, h, h);
        let mut rg = Replication::new(&cfg, g, h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 7);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        // Mask: each client owns exactly its roster actors; the host owns
        // the bot; `follower` only on the guest (match.js L106).
        assert_eq!(rh.remote_mask(), [false, true, false].as_slice());
        assert_eq!(rg.remote_mask(), [true, false, true].as_slice());
        assert!(!mh.follower);
        assert!(mg.follower, "guests never call time / judge");

        // Run both to Active.
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(mh.phase, Phase::Active);

        // A proxy runs no physics and no input: shove the guest's own actor
        // with input and confirm the proxy of host-actor-0 never moves.
        let pinned = mg.actors[0].pos;
        let mut inputs = vec![ActorInput::default(); 3];
        inputs[0].move_dir = vec3(0.0, 0.0, 1.0);
        inputs[1].move_dir = vec3(1.0, 0.0, 0.0);
        for _ in 0..30 {
            mg.step_masked(FIXED_DT, &inputs, Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(
            mg.actors[0].pos, pinned,
            "a proxy must not integrate input/physics"
        );
        assert!(
            mg.actors[1].pos != pinned,
            "the owned actor must run the sim"
        );

        // Tick content: the host streams only its owned actors (L192) and
        // rides the clock snapshot (L197-200).
        let mut rec = Rec::default();
        rh.update(TICK_DT, &mut mh, &sim, t, w, &mut rec);
        let (to, payload) = rec.find("t").clone();
        assert_eq!(to, None, "ticks are broadcast");
        let v = parse_json(&payload).unwrap();
        let (tick, clock) = decode_tick(&v);
        assert_eq!(tick.actors.len(), 2, "owned actors only: nid0 + bot nid2");
        assert_eq!(tick.actors[0].0, 0);
        assert_eq!(tick.actors[1].0, 2);
        assert!(clock.is_some(), "the host tick carries `c`");

        // A spoofed tick from the wrong sender must not move the proxy
        // (L248 `a.owner !== from`), and a non-newer ts is dropped (L251).
        let before = mg.actors[0].pos;
        let mut fake = tick.clone();
        fake.ts += 1.0;
        for (_, s) in &mut fake.actors {
            s.x += 50.0;
        }
        rg.on_message("IMPOSTOR", &encode_tick(&fake, None), &mut mg, t);
        assert_eq!(mg.actors[0].pos, before, "owner mismatch drops the sample");

        let mut good = fake.clone();
        good.ts += 1.0;
        rg.on_message(h, &encode_tick(&good, None), &mut mg, t);
        // Task 22: the sample lands in the playback buffer — advance the
        // clock one frame so the path is sampled and applied (tr sits
        // behind the newest sample → the Early hold shows it verbatim).
        rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut rec);
        assert_eq!(
            mg.actors[0].pos.x, good.actors[0].1.x,
            "the owner's word lands"
        );

        // Same ts again (a reordered retransmission): rejected.
        let mut stale = good.clone();
        stale.ts += 1.0;
        for (_, s) in &mut stale.actors {
            s.x += 100.0;
        }
        // `stale` reuses `good`'s sample timestamps? No — decode stamps
        // `t` from the tick ts, so bump nothing: same ts as `good`.
        stale.ts = good.ts;
        rg.on_message(h, &encode_tick(&stale, None), &mut mg, t);
        rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut rec);
        assert_eq!(
            mg.actors[0].pos.x, good.actors[0].1.x,
            "a non-newer sample is dropped (L251)"
        );
    }

    // ------------------------------------------------------------------ TR-21.2
    #[test]
    fn tr21_2_hit_channel_end_to_end() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        // Real relay ids — `send_to` routes by them (FakeRelay.route).
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);

        let mut mh = match111(&f, 0xABCD_1234, 120.0);
        let mut mg = match111(&f, 0xABCD_1234, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        // Both to Active, then pin the victim 3 m down the shooter's aim
        // line (team0 faces +Z from its pad).
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(mh.phase, Phase::Active);
        let pad0 = sim.spawn_pads[0];
        let victim_p = vec3(pad0.x, pad0.y, pad0.z + 3.0);
        mh.actors[1].invuln = 0.0;
        mg.actors[1].invuln = 0.0;
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = pad0;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
            a.invuln = 0.0;
        }

        // Hold the trigger on the host until the round hits the proxy.
        let hp_before = mh.actors[1].hp;
        let mut inputs = vec![ActorInput::default(); 3];
        let mut sent = None;
        for _ in 0..(10.0 * 60.0) as usize {
            inputs[0].fire = true;
            // keep the pinned geometry: the guest's ticks would otherwise
            // re-draw the proxy at the guest's own (unmoved) sim position.
            mh.actors[1].pos = victim_p;
            mh.actors[0].pos = pad0;
            mh.actors[0].aim_yaw = 0.0;
            mh.actors[0].aim_pitch = 0.0;
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
            let hit = mh.events.iter().find_map(|e| match e {
                MatchEvent::HitSend { damage, victim, .. } => Some((*victim, *damage)),
                _ => None,
            });
            if let Some((victim, damage)) = hit {
                assert_eq!(victim, (1, 0), "the victim is the guest's actor");
                assert!(damage > 0.0);
                // The proxy took NO local damage — JS 'send' route.
                assert_eq!(mh.actors[1].hp, hp_before, "proxy hp untouched");
                sent = Some(damage);
            }
            // rh.update consumes the HitSend and routes `{k:'hit'}` to G.
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if sent.is_some() {
                assert!(
                    mg.actors[1].hp < hp_before,
                    "the owner applied the damage immediately"
                );
                break;
            }
        }
        assert!(sent.is_some(), "a held trigger must hit the proxy");

        // The owner's chain: keep firing until the hits splat the victim →
        // respawn timer → respawn, tp bumped; the shooter's proxy follows
        // through the owner's ticks. Once the victim is down, stop firing
        // and stop pinning so the respawn is clean.
        let mut saw_respawn = false;
        let mut splatted = false;
        for _ in 0..(t.respawn_time * 60.0) as usize + 600 {
            inputs[0].fire = !splatted;
            if !splatted {
                mh.actors[1].pos = victim_p;
                mh.actors[0].pos = pad0;
                mh.actors[0].aim_yaw = 0.0;
                mh.actors[0].aim_pitch = 0.0;
            }
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if !mg.actors[1].alive {
                splatted = true;
            }
            if splatted && mg.actors[1].alive && mg.actors[1].net_tp >= 1 {
                saw_respawn = true;
                break;
            }
        }
        assert!(saw_respawn, "the victim respawned on the owner");
        assert!(
            mg.actors[1].deaths >= 1,
            "the hit splatted the victim: {:?}",
            mg.kills.last()
        );
        let mut followed = false;
        for _ in 0..120 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            // The guest's respawned actor-1 sample arrives at the HOST inbox.
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            if mh.actors[1].alive && rh.last_tp(1) == 1 && mh.actors[1].hp > 0.0 {
                followed = true;
                break;
            }
        }
        assert!(
            followed,
            "the tp bump and respawn state reached the shooter"
        );
    }

    // ------------------------------------------------------------------ TR-21.3
    #[test]
    fn tr21_3_guest_leave_actor_adopted_as_bot() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let (h, g) = ("HOST", "GUEST");
        let cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh = match111(&f, 5, 120.0);
        let mut rh = Replication::new(&cfg, h, h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 5);
        rh.bind_match(&mut mh);
        assert!(rh.remote_mask()[1], "the guest actor starts as a proxy");

        let nav = f.nav();
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            let mut inputs = vec![ActorInput::default(); 3];
            rh.drive_bots(&mut mh, &nav, &sim, t, w, &mut inputs);
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
        }
        // The leave: the host stays host, so `host_changed = false`.
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 5, &sim, t);
        assert!(
            !rh.remote_mask()[1],
            "the left player's squidkid is adopted"
        );
        assert_eq!(rh.owner_of(1), Some(h));
        assert_eq!(rh.nid_of(1), Some(1), "the ownership mapping keeps the nid");

        // It now simulates: the pinned proxy position must move under the
        // adopted bot.
        let before = mh.actors[1].pos;
        for _ in 0..120 {
            let mut inputs = vec![ActorInput::default(); 3];
            rh.drive_bots(&mut mh, &nav, &sim, t, w, &mut inputs);
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
        }
        assert_ne!(mh.actors[1].pos, before, "the adopted bot lives again");
    }

    #[test]
    fn tr21_3_host_migration_completes_the_match() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let mut obs = ep(&relay, 3);
        obs.connect("ROOM", "obs", false);
        for _ in 0..20 {
            trh.advance(1.0);
            trg.advance(0.0);
            obs.advance(0.0);
        }
        drain_msgs(&mut trh);
        drain_msgs(&mut trg);
        drain_msgs(&mut obs);
        // Real relay ids: the leave/re-election frames and `from == host_id`
        // checks compare against what the relay mints.
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let dur = 6.0;
        let mut mh = match111(&f, 9, dur);
        let mut mg = match111(&f, 9, dur);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 9);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);

        let nav = f.nav();
        // Intro out on both.
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            let mut ih = vec![ActorInput::default(); 3];
            rh.drive_bots(&mut mh, &nav, &sim, t, w, &mut ih);
            mh.step_masked(FIXED_DT, &ih, Some(rh.remote_mask()), &sim, t, w);
            let mut ig = vec![ActorInput::default(); 3];
            ig[1].move_dir = vec3(0.0, 0.0, -1.0);
            mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        assert_eq!(mg.phase, Phase::Active);

        // The host leaves: the relay re-elects the guest (oldest member).
        trh.simulate_closed("bye");
        let mut host_changed = false;
        let mut left = String::new();
        for _ in 0..20 {
            trg.advance(1.0);
            obs.advance(0.0);
            for e in trg.poll_events() {
                if let TransportEvent::Leave { id, host } = e {
                    left = id;
                    host_changed = host == g;
                }
            }
            if host_changed {
                break;
            }
        }
        assert!(host_changed, "the guest is the new host");
        assert_eq!(left, h);
        rg.set_host(&g);
        rg.on_leave(&h, true, &mut mg, &f.tuning.difficulty.easy, 9, &sim, t);

        // Ownership mapping: the old host's actors (his player + his bot)
        // now belong to the new host and are adopted; the guest's own actor
        // stays local; `follower` clears (L681).
        assert_eq!(rg.owner_of(0), Some(g.as_str()));
        assert_eq!(rg.owner_of(2), Some(g.as_str()));
        assert!(!rg.remote_mask()[0] && !rg.remote_mask()[2], "adopted");
        assert!(!rg.remote_mask()[1], "still mine");
        assert!(!mg.follower, "the new host calls time and judges");

        // Play out the rest of the short match: the new host drives all
        // three actors and settles with a result.
        let max = ((dur + FINISH_TIME) * 60.0) as usize + 120;
        for _ in 0..max {
            let mut ig = vec![ActorInput::default(); 3];
            rg.drive_bots(&mut mg, &nav, &sim, t, w, &mut ig);
            mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            drain_msgs(&mut trg);
            trg.advance(16.0);
            obs.advance(0.0);
            if mg.phase == Phase::End {
                break;
            }
        }
        assert_eq!(mg.phase, Phase::End, "the migrated host finishes");
        let r = mg.result.expect("settled with a result");
        assert!(
            (r.elapsed - dur).abs() < 0.1,
            "clock ran to zero: {}",
            r.elapsed
        );

        // The result broadcast + end frame reach an observer (the same
        // frames every guest applies).
        let mut m3 = match111(&f, 9, dur);
        let mut r3 = Replication::new(&cfg, "OBS", &g);
        r3.bind_match(&mut m3);
        m3.phase = Phase::Finish;
        rg.send_result(&mg, &mut trg);
        trg.advance(16.0);
        obs.advance(0.0);
        let msgs = drain_msgs(&mut obs);
        let res = msgs
            .iter()
            .find(|(_, p)| {
                parse_json(p).and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                    == Some("res".into())
            })
            .expect("sendResult broadcasts");
        assert_eq!(res.0, g);
        r3.on_message(&res.0, &res.1, &mut m3, t);
        assert_eq!(m3.phase, Phase::End, "res carries the guest to judge");
        assert_eq!(
            m3.result.expect("result").winner,
            r.winner,
            "the host's count"
        );
        assert_eq!(m3.time, 0.0);

        // `{k:'end'}` → the app seam.
        rg.send_end(&mut trg);
        trg.advance(16.0);
        obs.advance(0.0);
        let end = drain_msgs(&mut obs)
            .into_iter()
            .find(|(_, p)| {
                parse_json(p).and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                    == Some("end".into())
            })
            .expect("sendEnd broadcasts");
        assert_eq!(
            r3.on_message(&end.0, &end.1, &mut m3, t),
            Some(ReplicationEvent::MatchEnd)
        );
    }

    #[test]
    fn tr21_3_no_bots_stage_removes_leavers() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let (h, g) = ("HOST", "GUEST");
        // Cargo Terminal: humans-only (config.js L524).
        let cfg = cfg(roster111(h, g), "cargo", h);
        let mut mh = match111(&f, 4, 30.0);
        let mut rh = Replication::new(&cfg, h, h);
        rh.bind_match(&mut mh);
        assert_eq!(mh.actors.len(), 3);
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 4, &sim, t);
        // JS `_remove` (L686-690): the squidkid is gone from every screen.
        assert_eq!(mh.actors.len(), 2, "the leaver's actor is removed");
        assert_eq!(rh.nid_of(0), Some(0));
        assert_eq!(rh.nid_of(1), Some(2), "the bot shifted down with it");
        assert_eq!(rh.remote_mask().len(), 2);
        // The match still runs.
        let nav = f.nav();
        let mut inputs = vec![ActorInput::default(); 2];
        for _ in 0..60 {
            rh.drive_bots(&mut mh, &nav, &sim, t, w, &mut inputs);
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
        }
        assert_eq!(mh.actors.len(), 2);
    }

    #[test]
    fn guest_clock_steers_and_follower_gates_hold() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let mut mg = match111(&f, 3, 120.0);
        let cfg = cfg(roster111("HOST", "GUEST"), "tidewater", "HOST");
        let mut rg = Replication::new(&cfg, "GUEST", "HOST");
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(mg.phase, Phase::Active);
        mg.time = 100.0;
        // |diff| <= 0.2: untouched (L642).
        rg.on_message(
            "HOST",
            &encode_tick(
                &Tick {
                    ts: 1.0,
                    actors: vec![],
                    events: vec![],
                },
                Some(("playing", 100.1)),
            ),
            &mut mg,
            t,
        );
        assert_eq!(mg.time, 100.0);
        // |diff| > 0.2: nudge halfway (L642).
        rg.on_message(
            "HOST",
            &encode_tick(
                &Tick {
                    ts: 2.0,
                    actors: vec![],
                    events: vec![],
                },
                Some(("playing", 102.0)),
            ),
            &mut mg,
            t,
        );
        assert!((mg.time - 101.0).abs() < 1e-3, "halfway nudge: {}", mg.time);
        // `st` hard-sets the clock and re-enters the phase (L647-648)…
        rg.on_message(
            "HOST",
            &encode_host(&HostMsg::State {
                phase: "finish".into(),
                t: Some(0.0),
            }),
            &mut mg,
            t,
        );
        assert_eq!(mg.phase, Phase::Finish);
        assert_eq!(mg.time, 0.0);
        // …but a follower never judges on its own (L216): Finish + follower
        // must not settle even past the window.
        for _ in 0..(FINISH_TIME * 60.0) as usize + 10 {
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(mg.phase, Phase::Finish, "the follower waits for res");
        assert!(mg.result.is_none());
        // `judge` via `st` is ignored (L648) — only `res` settles it.
        rg.on_message(
            "HOST",
            &encode_host(&HostMsg::State {
                phase: "judge".into(),
                t: Some(0.0),
            }),
            &mut mg,
            t,
        );
        assert_eq!(mg.phase, Phase::Finish);
    }

    #[test]
    fn hit_wire_shape_matches_js() {
        // sendHit frame (netmatch.js L165): {k:'hit', v, a, d, w} with d
        // already r2'd; our channel uses w:'shot' (weapons.js L1608 fallback).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let cfg = cfg(roster111("H", "G"), "tidewater", "H");
        let mut m = match111(&f, 1, 30.0);
        let mut r = Replication::new(&cfg, "H", "H");
        r.bind_match(&mut m);
        let mut rec = Rec::default();
        m.events.push(MatchEvent::HitSend {
            attacker: (0, 0),
            victim: (1, 0),
            damage: 33.333,
        });
        let rest = r.update(FIXED_DT, &mut m, &sim, t, w, &mut rec);
        assert!(
            !rest.iter().any(|e| matches!(e, MatchEvent::HitSend { .. })),
            "HitSend is consumed"
        );
        let (to, payload) = rec.find("hit");
        assert_eq!(
            to.as_deref(),
            Some("G"),
            "point-to-point to the victim owner"
        );
        let v = parse_json(payload).unwrap();
        assert_eq!(v.field("k").and_then(JVal::as_str), Some("hit"));
        assert_eq!(v.field("v").and_then(JVal::as_f64), Some(1.0));
        assert_eq!(v.field("a").and_then(JVal::as_f64), Some(0.0));
        assert_eq!(v.field("d").and_then(JVal::as_f64), Some(33.33));
        assert_eq!(v.field("w").and_then(JVal::as_str), Some("shot"));
    }

    #[test]
    fn st_frame_without_t_leaves_the_clock_alone() {
        // Review M-4: JS `_hostState` L647 guards the hard-set with
        // `typeof d.t === 'number'` — a frame missing `t` must not zero the
        // guest clock (and `encode_host` drops the key, like JS
        // `JSON.stringify` dropping `undefined`).
        let f = fix();
        let mut mg = match111(&f, 3, 120.0);
        let cfg = cfg(roster111("HOST", "GUEST"), "tidewater", "HOST");
        let mut rg = Replication::new(&cfg, "GUEST", "HOST");
        rg.bind_match(&mut mg);
        mg.time = 42.0;
        let frame = encode_host(&HostMsg::State {
            phase: "playing".into(),
            t: None,
        });
        assert!(!frame.contains("\"t\""), "{frame}");
        rg.on_message("HOST", &frame, &mut mg, &f.tuning.player);
        assert_eq!(mg.time, 42.0, "a missing t must not zero the clock");
        assert_eq!(mg.phase, Phase::Active, "the state is still re-entered");
    }

    #[test]
    fn no_bots_bind_prunes_leavers() {
        // Review M-2: JS `bind` L79 — on a humans-only stage, actors whose
        // owner left *while the match was loading* are dropped at bind.
        let f = fix();
        let (h, g) = ("HOST", "GUEST");
        let cargo_cfg = cfg(roster111(h, g), "cargo", h);
        let mut mh = match111(&f, 4, 30.0);
        let mut rh = Replication::new(&cargo_cfg, h, h);
        rh.bind_match(&mut mh);
        // `g` left during the load — the session no longer lists them.
        rh.prune_loading_leavers(&mut mh, &[h.to_string()]);
        assert_eq!(mh.actors.len(), 2, "the leaver's actor is gone at bind");
        assert_eq!(rh.nid_of(1), Some(2), "the bot shifted down with it");
        assert_eq!(rh.remote_mask().len(), 2);
        // A stage that keeps bots is untouched (JS only prunes mapNoBots).
        let tide_cfg = cfg(roster111(h, g), "tidewater", h);
        let mut mh2 = match111(&f, 5, 30.0);
        let mut rh2 = Replication::new(&tide_cfg, h, h);
        rh2.bind_match(&mut mh2);
        rh2.prune_loading_leavers(&mut mh2, &[h.to_string()]);
        assert_eq!(mh2.actors.len(), 3, "bot stages keep everyone");
    }

    /// NFR-M2-2 / TR-22.4: the **playback layer's** CPU overhead must not
    /// push the native frame time more than 20 % over the bare sim step.
    /// Three arms over both endpoints of a 1v1+bot match (600 frames, 120
    /// warm-up, p50 of the per-frame wall time):
    ///   - `bare` — `step_masked` only (the M1 frame);
    ///   - `playback` — bare + the Task 22 layer driven directly
    ///     (`on_tick`/`push_sample` at 20 Hz from `ActorSample::from_actor`,
    ///     `advance_all`/`sample`/`apply` every frame), no JSON;
    ///   - `full` — bare + the whole [`Replication`] overlay including the
    ///     tick encode/decode (reported, not gated — the wire codec is
    ///     Task 21's surface and dominates its cost).
    ///
    /// Ignored by default (wall-clock, release-profile gate): run with
    /// `cargo test --release -p inkwave_sim -- --ignored`. Takes the min p50
    /// over five interleaved repeats per arm — a shared box throws
    /// scheduler hiccups (one run read +132 %), and best-case is the robust
    /// estimator for CPU overhead.
    #[test]
    #[ignore]
    fn nfr_m2_2_playback_overlay_stays_under_20_percent() {
        use std::time::Instant;

        use crate::net::playback::Playback;

        /// A transport that keeps the framed payloads for direct delivery
        /// to the peer's `on_message` (no relay latency inside the timed
        /// loop — we measure the CPU overlay, not the fake network).
        struct Tap {
            out: Vec<String>,
        }
        impl Transport for Tap {
            fn connect(&mut self, _c: &str, _n: &str, _r: bool) {}
            fn poll_events(&mut self) -> Vec<TransportEvent> {
                Vec::new()
            }
            fn send_raw(&mut self, s: &str) -> bool {
                if let Some(rest) = s.strip_prefix("b|") {
                    self.out.push(rest.to_string());
                } else if let Some(rest) = s.strip_prefix("s|")
                    && let Some((_, body)) = rest.split_once('|')
                {
                    self.out.push(body.to_string());
                }
                true
            }
            fn is_open(&self) -> bool {
                true
            }
            fn rtt(&self) -> f64 {
                0.0
            }
            fn close(&mut self) {}
        }

        fn p50(mut v: Vec<f64>) -> f64 {
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        }

        fn run(arm: u8) -> (f64, usize) {
            let f = fix();
            let sim = f.sim();
            let t = &f.tuning.player;
            let w = &f.tuning.spritzer;
            let (h, g) = ("HOST", "GUEST");
            let cfg = cfg(roster111(h, g), "tidewater", h);
            let mut mh = match111(&f, 11, 30.0);
            let mut mg = match111(&f, 11, 30.0);
            let mut rh = Replication::new(&cfg, h, h);
            let mut rg = Replication::new(&cfg, g, h);
            rh.attach_bot(2, &mh.actors[2], &f.tuning.difficulty.easy, 11);
            rh.bind_match(&mut mh);
            rg.bind_match(&mut mg);
            let nav = f.nav();
            let mut th = Tap { out: Vec::new() };
            let mut tg = Tap { out: Vec::new() };

            // `playback` arm: the Task 22 layer per client, driven by hand
            // (the same call sequence `Replication::update` runs, minus the
            // wire codec).
            let mut pbh = Playback::new(3, 1.0 / 20.0);
            let mut pbg = Playback::new(3, 1.0 / 20.0);
            let mut now = 0.0f64;
            let dt64 = FIXED_DT as f64;
            let gravity = t.gravity as f64;
            let mut tick_t = 0.0f32;

            const FRAMES: usize = 600;
            const WARM: usize = 120;
            let mut times: Vec<f64> = Vec::with_capacity(FRAMES - WARM);
            for frame in 0..FRAMES {
                let t0 = Instant::now();
                let mut ih = vec![ActorInput::default(); 3];
                rh.drive_bots(&mut mh, &nav, &sim, t, w, &mut ih);
                mh.step_masked(FIXED_DT, &ih, Some(rh.remote_mask()), &sim, t, w);
                let ig = vec![ActorInput::default(); 3];
                mg.step_masked(FIXED_DT, &ig, Some(rg.remote_mask()), &sim, t, w);
                match arm {
                    0 => {}
                    1 => {
                        now += dt64;
                        tick_t -= FIXED_DT;
                        if tick_t <= 0.0 {
                            tick_t += 1.0 / 20.0;
                            // owners stream their actors (JS `_sendTick`):
                            // the host's tick (host-owned samples) feeds the
                            // guest's playback under sender `h`, and vice
                            // versa — so `pbh.sample(_, g)` / `pbg.sample(_,
                            // h)` below find a live clock.
                            pbh.on_tick(g, now, now);
                            pbg.on_tick(h, now, now);
                            for (i, &rem) in rh.remote.iter().enumerate() {
                                if !rem {
                                    let s = ActorSample::from_actor(
                                        &mh.actors[i],
                                        &mh.runners[i],
                                        mh.actors[i].net_tp,
                                        now as f32,
                                    );
                                    pbg.push_sample(i, &s);
                                }
                            }
                            for (i, &rem) in rg.remote.iter().enumerate() {
                                if !rem {
                                    let s = ActorSample::from_actor(
                                        &mg.actors[i],
                                        &mg.runners[i],
                                        mg.actors[i].net_tp,
                                        now as f32,
                                    );
                                    pbh.push_sample(i, &s);
                                }
                            }
                        }
                        pbh.advance_all(now, dt64);
                        pbg.advance_all(now, dt64);
                        for (i, &rem) in rh.remote.iter().enumerate() {
                            if rem {
                                pbh.sample(i, g, dt64, gravity);
                            }
                        }
                        for (i, &rem) in rg.remote.iter().enumerate() {
                            if rem {
                                pbg.sample(i, h, dt64, gravity);
                            }
                        }
                        for (i, &rem) in rh.remote.iter().enumerate() {
                            if rem {
                                pbh.apply(i, &mut mh.actors[i], FIXED_DT);
                            }
                        }
                        for (i, &rem) in rg.remote.iter().enumerate() {
                            if rem {
                                pbg.apply(i, &mut mg.actors[i], FIXED_DT);
                            }
                        }
                    }
                    _ => {
                        let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut th);
                        let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut tg);
                        for p in th.out.drain(..) {
                            rg.on_message(h, &p, &mut mg, t);
                        }
                        for p in tg.out.drain(..) {
                            rh.on_message(g, &p, &mut mh, t);
                        }
                    }
                }
                if frame >= WARM {
                    times.push(t0.elapsed().as_secs_f64() * 1e6);
                }
            }
            // The proxy shown-count: the timing arms are only meaningful if
            // the overlay actually interpolates and displays them. The
            // `full` arm drives the clients' internal playback, the
            // `playback` arm the hand-built ones.
            let mut shown = 0;
            for (i, &rem) in rh.remote.iter().enumerate() {
                let live = if arm == 2 {
                    rh.playback().proxy_shown(i, mh.actors[i].alive)
                } else {
                    pbh.proxy_shown(i, mh.actors[i].alive)
                };
                if rem && live {
                    shown += 1;
                }
            }
            for (i, &rem) in rg.remote.iter().enumerate() {
                let live = if arm == 2 {
                    rg.playback().proxy_shown(i, mg.actors[i].alive)
                } else {
                    pbg.proxy_shown(i, mg.actors[i].alive)
                };
                if rem && live {
                    shown += 1;
                }
            }
            (p50(times), shown)
        }

        // Wall-clock arms on a shared box are noisy (one observed run read
        // +132 % from a scheduler hiccup). Take the **min** p50 over five
        // interleaved repeats per arm — best-case is the robust estimator
        // for CPU overhead — and check the shown-count from the last repeat.
        let mut base = f64::INFINITY;
        let mut pb = f64::INFINITY;
        let mut full = f64::INFINITY;
        let (mut base_shown, mut pb_shown, mut full_shown) = (0, 0, 0);
        for _ in 0..5 {
            let (v, s) = run(0);
            base = f64::min(base, v);
            base_shown = s;
            let (v, s) = run(1);
            pb = f64::min(pb, v);
            pb_shown = s;
            let (v, s) = run(2);
            full = f64::min(full, v);
            full_shown = s;
        }
        assert_eq!(pb_shown, 3, "the playback arm interpolates every proxy");
        assert_eq!(full_shown, 3, "the full overlay interpolates every proxy");
        assert_eq!(base_shown, 0, "the bare arm drives none");
        let gain = (pb - base) / base;
        eprintln!(
            "[nfr-m2-2] bare p50 = {base:.1} µs, +playback = {pb:.1} µs ({:+.1}%), +full overlay = {full:.1} µs ({:+.1}%)",
            gain * 100.0,
            (full - base) / base * 100.0
        );
        assert!(gain < 0.20, "playback gain {:+.1}% ≥ 20%", gain * 100.0);
    }

    // ------------------------------------------------------------------ TR-23.1
    #[test]
    fn tr23_1_splat_replay_matches_grid_and_never_loops_back() {
        // A sprays ink → B replays the timeline: byte-identical coverage
        // grids, and B re-broadcasts nothing (JS `applying` guard, L110 —
        // Rust: the `recorded_ink` cursor in `play_events`).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 21, 120.0);
        let mut mg = match111(&f, 21, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        assert_eq!(mh.phase, Phase::Active);
        assert!(
            mh.paint.grid().iter().all(|&v| v == 0),
            "both grids start ink-free"
        );

        // A's splat at an r2-clean centre (the wire quantization is a no-op,
        // so a faithful replay must land on byte-identical cells).
        let pad0 = sim.spawn_pads[0];
        let c = vec3(
            wire::r2(pad0.x as f64) as f32,
            (wire::r2(pad0.y as f64) + 0.3) as f32,
            wire::r2(pad0.z as f64) as f32,
        );
        let opts = SplatOpts {
            seed: 0.42,
            stretch: None,
            stretch_amt: None,
            kind: None,
        };
        mh.paint.splat(sim.collision, c, 2.0, 0, &opts);
        assert!(
            mh.paint.grid().iter().any(|&v| v != 0),
            "the host splat painted cells"
        );

        // Run until B's grid catches up. B's own outgoing ticks must never
        // carry a splat record for the replayed ink (loopback).
        let mut guest_splat_records = 0;
        let mut settled = false;
        for _ in 0..600 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            // Host inbox: B's ticks — scan them for splat records *before*
            // routing (the loopback check).
            for (from, p) in drain_msgs(&mut trh) {
                if from == g
                    && let Some(v) = parse_json(&p)
                    && v.field("k").and_then(JVal::as_str) == Some("t")
                {
                    let (tick, _) = decode_tick(&v);
                    guest_splat_records += tick
                        .events
                        .iter()
                        .filter(|e| matches!(e.kind, TickEvent::Splat(_)))
                        .count();
                }
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
            if mg.paint.grid() == mh.paint.grid() {
                settled = true;
                break;
            }
        }
        assert!(settled, "B replays A's ink: grids byte-identical");
        assert_eq!(
            guest_splat_records, 0,
            "a replayed splat is never re-broadcast (JS `applying`)"
        );
    }

    // ------------------------------------------------------------------ TR-23.2
    #[test]
    fn tr23_2_ghost_round_never_damages() {
        // One host round decides exactly one hit; the `'hit'` channel applies
        // it once on the owner. The guest's ghost copy flies the same path
        // and must not damage its own victim a second time (JS
        // `ghostProjectile` `damage: 0`, weapons.js L871 + the `dmg > 0`
        // guard, L1608).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 22, 120.0);
        let mut mg = match111(&f, 22, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        let pad0 = sim.spawn_pads[0];
        let victim_p = vec3(pad0.x, pad0.y, pad0.z + 3.0);
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = pad0;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
            a.invuln = 0.0;
        }
        for m in [&mut mh.actors[1], &mut mg.actors[1]] {
            m.pos = victim_p;
            m.invuln = 0.0;
        }
        let hp_before = mg.actors[1].hp;

        // Exactly one round: the trigger is held for a single frame.
        let mut inputs = vec![ActorInput::default(); 3];
        let mut dmg = None;
        let mut saw_ghost = false;
        for frame in 0..(1.5 * 60.0) as usize {
            inputs[0].fire = frame == 0;
            // keep the pinned geometry (proxies redraw from samples)
            mh.actors[1].pos = victim_p;
            mg.actors[1].pos = victim_p;
            mh.actors[0].pos = pad0;
            mh.actors[0].aim_yaw = 0.0;
            mh.actors[0].aim_pitch = 0.0;
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
            if dmg.is_none() {
                dmg = mh.events.iter().find_map(|e| match e {
                    MatchEvent::HitSend { damage, .. } => Some(*damage),
                    _ => None,
                });
            }
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            if !saw_ghost {
                saw_ghost = mg
                    .projectiles
                    .list
                    .iter()
                    .any(|p| p.ghost && p.damage == 0.0);
            }
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        let d = dmg.expect("the host round hit the proxy");
        assert!(saw_ghost, "the 'p' record spawned a ghost round on B");
        assert!(mg.actors[1].alive, "one round does not splat the victim");
        // The owner applied the round's damage exactly once; the guest's
        // ghost copy must not hurt its own victim a second time (a double
        // application would land near `hp_before - 2*d`, far below the
        // tolerance band — hp regen over the test window is a few points).
        let lost = hp_before - mg.actors[1].hp;
        assert!(
            lost > 0.0 && lost < d + 6.0,
            "exactly one damage application — the ghost never hurts: lost {lost} vs d {d}"
        );
        assert!(
            mh.projectiles.list.is_empty() && mg.projectiles.list.is_empty(),
            "both rounds die on expiry"
        );
    }

    #[test]
    fn tr23_2_ghost_never_paints_and_splats_land_on_the_sender_clock() {
        // A steep round into the turf: the host paints (its impact splat is
        // recorded as `'s'`), the guest's ghost flies the same path and must
        // add nothing of its own — and the replayed splat matures on the
        // playback timeline (~delay behind the sender clock), not the moment
        // the packet arrives (JS `_playEvents` gates on `p.tr`, L452-462).
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.spritzer;
        let relay = FakeRelay::new();
        let (mut trh, mut trg) = pair(&relay);
        let (h, g) = (trh.id(), trg.id());
        let cfg = cfg(roster111(&h, &g), "tidewater", &h);
        let mut mh = match111(&f, 23, 120.0);
        let mut mg = match111(&f, 23, 120.0);
        let mut rh = Replication::new(&cfg, &h, &h);
        let mut rg = Replication::new(&cfg, &g, &h);
        rh.bind_match(&mut mh);
        rg.bind_match(&mut mg);
        for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
            mh.step_masked(FIXED_DT, &[], Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
        }
        let pad0 = sim.spawn_pads[0];
        let stand = vec3(pad0.x, pad0.y, pad0.z + 2.0);
        for a in [&mut mh.actors[0], &mut mg.actors[0]] {
            a.pos = stand;
            a.yaw = 0.0;
            a.aim_yaw = 0.0;
            a.aim_pitch = 0.0;
        }

        let mut inputs = vec![ActorInput::default(); 3];
        let mut host_at = None;
        let mut guest_at = None;
        let mut saw_ghost = false;
        for frame in 0..150 {
            inputs[0].fire = frame == 0;
            mh.actors[0].pos = stand;
            mh.actors[0].aim_pitch = -1.4;
            mg.actors[0].pos = stand;
            mh.step_masked(FIXED_DT, &inputs, Some(rh.remote_mask()), &sim, t, w);
            mg.step_masked(FIXED_DT, &[], Some(rg.remote_mask()), &sim, t, w);
            let _ = rh.update(FIXED_DT, &mut mh, &sim, t, w, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, &sim, t, w, &mut trg);
            if host_at.is_none() && !mh.paint.ink_stream().is_empty() {
                host_at = Some(frame);
            }
            if !saw_ghost {
                saw_ghost = mg.projectiles.list.iter().any(|p| p.ghost);
            }
            if guest_at.is_none() && !mg.paint.ink_stream().is_empty() {
                guest_at = Some(frame);
            }
            trh.advance(16.0);
            trg.advance(0.0);
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            for (from, p) in drain_msgs(&mut trg) {
                rg.on_message(&from, &p, &mut mg, t);
            }
        }
        let hf = host_at.expect("the host round painted the turf");
        let gf = guest_at.expect("the splat replayed onto B");
        assert!(
            gf > hf,
            "the replay waits for the playback clock (arrived frame {hf}, played {gf})"
        );
        assert!(
            gf - hf >= 3,
            "the splat lands ~sender-clock + playback delay, not on packet arrival: {gf} - {hf}"
        );
        assert!(saw_ghost, "the 'p' record spawned a ghost on B");
        assert_eq!(
            mg.paint.ink_stream().len(),
            mh.paint.ink_stream().len(),
            "the ghost painted nothing extra — B's stream is pure replay"
        );
        assert!(
            mh.projectiles.list.is_empty() && mg.projectiles.list.is_empty(),
            "the ghost disappears on expiry"
        );
    }
}
