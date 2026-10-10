//! Task 21 — authoritative sharding and host duties (replication).
//!
//! Faithful port of the *ownership / routing / host-control* half of
//! `src/net/netmatch.js`. The playback half (per-sender clocks, Hermite
//! interpolation, corrections) is Task 22 and the event timeline is Task 23;
//! this layer already carries their seams (sample buffers, the `tp` cut, the
//! `out` queue) so those tasks extend in place.
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
//!
//! Scope and deviations (recorded for review):
//!   - **Direct time source**: JS stamps ticks with `now() =
//!     performance.now()/1000`; the sim has no wall clock, so the replicator
//!     accumulates its own logical seconds (`now += dt`). Rust↔Rust only —
//!     both ends use the same rule, and the per-actor monotonic-`t` guard
//!     (L251) is the only consumer.
//!   - **Proxy state write is discrete** (Task 22 replaces it): received
//!     samples are applied straight onto the proxy [`Actor`] — position,
//!     velocity, facing, flags, hp/ink/turf and the `tp` counter — with no
//!     interpolation buffer yet. The monotonic guard keeps stale samples from
//!     regressing the actor.
//!   - **`w:'shot'`**: the JS shooter round reports `p.weaponId || p.wid ||
//!     p.type` (weapons.js L1608) and the M1 Spritzer has no `weaponId`/`wid`,
//!     so the wire id is `'shot'` — the victim's owner only forwards it to
//!     FX/audio (M1 has no kit damage modifiers reading `w`).
//!   - **`dh` / `bhit`**: M2 is turf-only with no devices/boss — received as
//!     ignored stubs (JS L210-215), per the spec's 收口要求.
//!   - **Adopted-bot difficulty**: JS `new BotBrain(a, cfg.difficulty ||
//!     'normal')` (L698); the embedded tuning ships only `easy`
//!     ([`crate::tuning::DifficultySet`]), so adopted bots always get `easy`.
//!   - **`spawnPending` / `handoff`**: JS `_adopt` finishes a mid-respawn
//!     proxy (L704) and non-host owners glide onto the new sender's path
//!     (L679) — both are playback states (Task 22); here the buffer guard is
//!     reset (the JS `buf.length = 0`) and the actor is taken where it was
//!     last drawn (`a.pos` already holds the applied sample).
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

use crate::actor::{Actor, ActorInput, FIXED_DT, Form, SimWorld};
use crate::bot::{Bot, BotCtx};
use crate::match_::{Match, MatchEvent, MatchResult, Phase, find_actor};
use crate::nav::NavGraph;
use crate::net::session::{StartCfg, map_no_bots};
use crate::net::transport::Transport;
use crate::net::wire::{
    self, JVal, decode_direct, decode_host, decode_tick, encode_direct, encode_host, encode_tick,
    parse_json,
};
use crate::net::{ActorSample, ActorStatsRow, DirectMsg, HostMsg, SessionId, Tick, TimedEvent};
use crate::tuning::{Difficulty, PlayerTuning, Spritzer};
use glam::vec3;

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
    /// Per-actor last-applied sample timestamp (JS `buf[buf.length-1].t`
    /// monotonic guard, L251). `f32::MIN` = empty buffer.
    last_t: Vec<f32>,
    /// Per-actor last-seen teleport counter — JS `a.net.tp`, read by `_adopt`
    /// to continue the counter without a false snap (L702).
    last_tp: Vec<u32>,
    /// Outgoing event timeline (JS `this.out`, L107) — Task 23 fills it;
    /// [`Replication::update`] ships it non-empty as the tick's `e`.
    pub out: Vec<TimedEvent>,
    /// JS `tickT` (L57, starts 0 → the first frame sends).
    tick_t: f32,
    /// JS `clockT` (L61, starts 0 → the first host tick carries `c`).
    clock_t: f32,
    /// Logical sender clock, seconds (see module docs).
    now: f32,
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
            last_t: vec![f32::MIN; n],
            last_tp: vec![0; n],
            out: Vec::new(),
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
        self.last_t.remove(i);
        self.last_tp.remove(i);
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

    /// The last teleport counter seen for an actor (JS `a.net.tp`).
    #[must_use]
    pub fn last_tp(&self, idx: usize) -> u32 {
        self.last_tp.get(idx).copied().unwrap_or(0)
    }

    /// The app's host view changed (session `Host` / `Leave` events).
    pub fn set_host(&mut self, host_id: &str) {
        self.host_id = host_id.to_string();
    }

    /// One 60 Hz frame of replication (JS `NetMatch.update`, L172-188):
    /// advance the sender clock, run the 20 Hz tick timer (owners stream
    /// their actors; the host rides its `c` clock snapshot every 0.5 s),
    /// and drain the match event bus — the host re-broadcasts phase changes
    /// (`{k:'st'}`, L80) and every client routes proxy hits to the victim's
    /// owner (`{k:'hit'}`, L163-167).
    ///
    /// Returns the events the replicator did *not* consume (Splat / Turf /
    /// Countdown / Respawn / OneMinute — HUD and Task 23's consumers).
    pub fn update(
        &mut self,
        dt: f32,
        m: &mut Match,
        t: &PlayerTuning,
        tr: &mut impl Transport,
    ) -> Vec<MatchEvent> {
        self.now += dt;

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

        // ---- event bus (JS listens on `match:state` L80 and the weapon
        // layer calls `sendHit` inline, L163-167; the sim surfaces both as
        // bus events drained here).
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
                other => rest.push(other),
            }
        }
        rest
    }

    /// JS `_sendTick` (L190-203): pack every **owned** actor
    /// (`if (!x.remote)`, L192), stamp with the sender clock, ride the
    /// pending event timeline, and for the host the 0.5 s clock snapshot
    /// `c = [state, r2(time)]` (L197-200).
    fn send_tick(&mut self, m: &Match, tr: &mut impl Transport) {
        let ts = wire::r3(wire::dec(self.now)) as f32;
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

    /// JS `_tick` (L229-263) — the actor-sample half. The per-sender playback
    /// clock (`p.off` / `p.want`) and the event queue are Task 22/23; here:
    /// route by `nid`, drop samples that are not for a proxy we own from
    /// this sender (`!a || !a.remote || a.owner !== from`, L248), guard
    /// monotonicity (L251), apply discretely, and steer the guest clock off
    /// the host's `c` (L257).
    fn on_tick(&mut self, from: &str, v: &JVal, m: &mut Match) {
        let (tick, clock) = decode_tick(v);
        if let Some((state, time)) = clock
            && from == self.host_id
        {
            self.host_clock(m, &state, time);
        }
        for (nid, s) in tick.actors {
            let Some(i) = self.idx_of_nid(nid) else {
                continue;
            };
            if !self.remote[i] || self.owners[i] != from {
                continue;
            }
            // JS L251: a sample no newer than the buffer tail is dropped
            // (late reordering); `f32::MIN` = empty buffer accepts anything.
            if s.t <= self.last_t[i] {
                continue;
            }
            self.last_t[i] = s.t;
            self.last_tp[i] = s.tp;
            apply_sample(&mut m.actors[i], &s);
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
    /// have applied [`Replication::set_host`] first.
    pub fn on_leave(
        &mut self,
        id: &str,
        host_changed: bool,
        m: &mut Match,
        diff: &Difficulty,
        seed: u64,
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
                self.adopt(i, m, diff, seed);
            } else {
                // Same squidkid, new sender: drop the stale buffer so the
                // first sample from the new owner lands (`buf.length = 0`).
                self.last_t[i] = f32::MIN;
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
    /// seen (`a.netTp = a.net.tp || 0`, L702).
    fn adopt(&mut self, i: usize, m: &mut Match, diff: &Difficulty, seed: u64) {
        self.remote[i] = false;
        let tp = self.last_tp[i];
        m.actors[i].net_tp = tp;
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

/// Discrete proxy write (Task 22 replaces this with the interpolation
/// buffer): position (the sample's `y` is the owner's *visual* feet height,
/// `pos.y + smoothY`, netmatch.js L745 — the proxy keeps `smooth_y = 0`),
/// velocity, facing, the flag word decoded back onto the actor, hp/ink/turf
/// and the teleport counter (`_adopt` continues from it, L702).
fn apply_sample(a: &mut Actor, s: &ActorSample) {
    a.pos.x = s.x;
    a.pos.z = s.z;
    a.set_visual_y(s.y);
    a.vel = vec3(s.vx, s.vy, s.vz);
    a.yaw = s.yaw;
    a.aim_yaw = s.aim_yaw;
    a.aim_pitch = s.aim_pitch;
    a.form = if s.flag(crate::net::flags::SQUID) {
        Form::Squid
    } else {
        Form::Kid
    };
    a.submerged = s.flag(crate::net::flags::SUB);
    a.grounded = s.flag(crate::net::flags::GROUNDED);
    a.ground_team = if s.flag(crate::net::flags::GT1) {
        1
    } else if s.flag(crate::net::flags::GT2) {
        2
    } else {
        0
    };
    a.on_enemy = s.flag(crate::net::flags::ENEMY);
    a.alive = s.flag(crate::net::flags::ALIVE);
    a.hp = s.hp;
    a.ink = s.ink;
    // JS `applyRemote` L377-378: the invuln bit is a hard 0.1 s window (the
    // proxy never runs `Actor::step`, so nothing else decrements it), and the
    // turf stat only ever ratchets up (`Math.max`).
    a.invuln = if s.flag(crate::net::flags::INVULN) {
        0.1
    } else {
        0.0
    };
    a.turf = a.turf.max(s.turf);
    a.net_tp = s.tp;
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
        rh.update(TICK_DT, &mut mh, t, &mut rec);
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
            let _ = rh.update(FIXED_DT, &mut mh, t, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, t, &mut trg);
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
            let _ = rh.update(FIXED_DT, &mut mh, t, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, t, &mut trg);
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
            let _ = rh.update(FIXED_DT, &mut mh, t, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, t, &mut trg);
            trh.advance(16.0);
            trg.advance(0.0);
            // The guest's respawned actor-1 sample arrives at the HOST inbox.
            for (from, p) in drain_msgs(&mut trh) {
                rh.on_message(&from, &p, &mut mh, t);
            }
            if mh.actors[1].alive && mh.actors[1].net_tp == 1 && mh.actors[1].hp > 0.0 {
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
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 5);
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
            let _ = rh.update(FIXED_DT, &mut mh, t, &mut trh);
            let _ = rg.update(FIXED_DT, &mut mg, t, &mut trg);
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
        rg.on_leave(&h, true, &mut mg, &f.tuning.difficulty.easy, 9);

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
            let _ = rg.update(FIXED_DT, &mut mg, t, &mut trg);
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
        rh.on_leave(g, false, &mut mh, &f.tuning.difficulty.easy, 4);
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
        let t = &f.tuning.player;
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
        let rest = r.update(FIXED_DT, &mut m, t, &mut rec);
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

    #[test]
    fn apply_sample_writes_invuln_window_and_turf_ratchet() {
        // Review M-5: JS `applyRemote` L377-378 — the invuln bit is a hard
        // 0.1 s window, and turf only ratchets up (`Math.max`).
        let f = fix();
        let t = &f.tuning.player;
        let mut a = Actor::new(0, 0, t);
        let mut s = ActorSample::from_actor(&a, &WeaponRunner::new(), 0, 1.0);
        s.turf = 50.0;
        s.f |= crate::net::flags::INVULN;
        apply_sample(&mut a, &s);
        assert_eq!(a.invuln, 0.1, "the invuln bit is a 0.1 s window");
        assert_eq!(a.turf, 50.0);
        // A smaller turf must not regress the stat; the bit off clears the
        // window.
        s.t = 2.0;
        s.turf = 40.0;
        s.f &= !crate::net::flags::INVULN;
        apply_sample(&mut a, &s);
        assert_eq!(a.turf, 50.0, "turf only ratchets up");
        assert_eq!(a.invuln, 0.0, "bit off clears the window");
    }
}
