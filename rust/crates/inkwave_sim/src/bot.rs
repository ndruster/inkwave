//! Easy bot brain (spec Task 9): patrol-and-paint → spot → engage with the
//! `difficulty.easy` reaction/aim-error parameters → break off to heal /
//! refill → respawn and continue. Faithful *simplification* of
//! `src/game/bots.js` (`BotBrain`): the upstream state machine, perception,
//! aim spring and stuck-recovery are ported; the Spritzer-only M1 weapon set
//! removes every per-kind branch, and M2+ systems are out of scope.
//!
//! Upstream mapping (bots.js):
//!   - `BotBrain.reset` / ctor                          -> [`Bot::new`]
//!   - `update` dead / not-playing early exits          -> zeroed input (the
//!     match already zeroes intents outside `playing`, match.js L220-226)
//!   - `_perceive` (awareness / LOS / squid filter)     -> [`Bot::perceive`]
//!   - `update` mode selection (paint/fight/refill/
//!     retreat, fireDiscipline)                         -> [`Bot::select_mode`]
//!   - `_pickPaintGoal` / `_pathTo` / `_pickRefill` /
//!     `_pickRetreat`                                   -> goal helpers
//!   - `update` fight block (lead / aim error / strafe
//!     / fire gate / swim-in / dodge-hop)               -> [`Bot::fight`]
//!   - `update` paint block (sweep / region scan /
//!     needPaint / squid travel)                        -> [`Bot::paint`]
//!   - `update` refill block                            -> [`Bot::refill`]
//!   - `_tail` aim spring + move slew + water guard +
//!     stuck / displacement watchdogs                   -> [`Bot::tail`]
//!   - `_steer` / `_unstick` / `_backOnNav`             -> steering helpers
//!   - `_wet` / `_nearWater` / `_edgeGuard` /
//!     `_squidWouldDrop` / `_avoidWater`                -> water helpers
//!
//! Out of scope (M1, per PORT_MAP — "仅 easy" + Spritzer):
//!   - Zone Control plan / roles / retake waves, Boss Battle, threat AI
//!     (Waddle/Torpedo/canopy), kits, subs, specials, super jumps.
//!   - Weapon kinds other than `shooter`: the per-kind branches (MELEE /
//!     CHARGES / spinner / splatling / slosher / brush / roller / dualies /
//!     twins) collapse to the Spritzer `else` branches.
//!   - Climb edges (see `nav` module docs): the bot never plans a climb.
//!
//! Intentional deviations from JS (recorded for review):
//!   - **`Math.random()`** → the bot's own seeded [`Rng`] stream (one per
//!     bot, derived from the match seed + roster index): the sim determinism
//!     contract (AC-6 / Task 8 module docs). The draw *count and purpose*
//!     mirror JS; the interleaved call *order* across the update does not
//!     (the ctor pre-draws its six randomised fields, the rest are drawn at
//!     their use sites) — values differ from JS by design.
//!   - **`_steer` lookahead**: upstream re-aims at the furthest walk-visible
//!     waypoint every ~0.1 s (`_fatLos` + `_dryLine` probes). The easy bot
//!     steers straight at the current waypoint; the 1 m waypoint spacing plus
//!     the move-heading slew keeps motion smooth without the probe cost.
//!   - **`_tail` bomb release / `aimPoint`**: the weapon runner consumes the
//!     actor's `aim_dir()` directly; no separate aim point is needed.
//!   - **dodge-hop**: upstream also rolls for dualies/twins kit moves; the
//!     Spritzer branch (strafe-hop right after a hit, `lastDamage < 0.25`,
//!     `dodgeCd`, never near water) is kept verbatim.
//!   - **`_pickPaintGoal` goal-coverage check** (JS `goalCheckT`): the 1 s
//!     "already covered → move on" probe is folded into the goal timer.
//!   - **`_pickPaintGoal` `bot.goal` penalty**: teammate goals come from
//!     [`BotCtx::mate_goals`] (roster-indexed) instead of reading the live
//!     `G.actors[].bot.goal`.
//!   - **`_backOnNav` dry-line/LOS candidate filter**: the easy bot takes
//!     the nearest valid node outright (M1 simplification; the walk-out
//!     behaviour is identical on Tidewater's open plaza).

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, ActorInput, Form, SimWorld};
use crate::match_::find_actor;
use crate::nav::{EdgeType, NavGraph};
use crate::paint::PaintGrid;
use crate::tuning::{Difficulty, PlayerTuning, Spritzer};
use crate::weapon::Rng;
use glam::{Vec2, Vec3};

/// Paint-mode aim pitch for the shooter (JS `else` branch, bots.js L557).
const PAINT_PITCH: f32 = -0.42;
/// Paint scan reach for the shooter (JS `else` branch, bots.js L544).
const PAINT_REACH: f32 = 4.5;
/// Preferred duel distance factor (JS `range * 0.7`, non-melee/charger).
const FIGHT_PREF_F: f32 = 0.7;
/// Fire tolerance base: `atan2(0.55, dist)` (JS L475).
const AIM_TOL_HALF: f32 = 0.55;
/// JS `PLAYER` eye height for perception (bots.js L1624).
const EYE_Y: f32 = 1.3;

/// Bot high-level mode (JS `this.mode`; 'climb' absent — no climb edges).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BotMode {
    #[default]
    Paint,
    Fight,
    Refill,
    Retreat,
}

/// Shared read-only context for one bot decision step.
pub struct BotCtx<'a> {
    pub actors: &'a [Actor],
    pub nav: &'a NavGraph,
    pub paint: &'a PaintGrid,
    pub world: &'a SimWorld<'a>,
    pub t: &'a PlayerTuning,
    pub w: &'a Spritzer,
    /// Teammate paint goals by roster index (`None` = no goal); mirrors the
    /// JS `m.bot.goal` read in `_pickPaintGoal`.
    pub mate_goals: &'a [Option<usize>],
}

/// Per-bot observable counters for the autopilot report (TR-9.1/9.3/9.4).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct BotStats {
    /// Total XZ distance travelled while alive, m (movement evidence).
    pub dist: f32,
    /// Rising-edge fire presses (shots attempted).
    pub shots: u32,
    /// Squid presses (swim starts).
    pub swims: u32,
    /// Splats credited by the match (mirrors `Actor::splats`).
    pub splats: u32,
    /// Splatted deaths (mirrors `Actor::deaths`).
    pub deaths: u32,
    /// Longest observed 1.5 s stall window while wanting to move, s
    /// (displacement watchdog evidence, TR-9.3).
    pub max_stall: f32,
    /// Watchdog firings (stall windows counted).
    pub stalls: u32,
    /// Mode at report time.
    pub mode: BotMode,
}

/// One easy bot (JS `BotBrain`, shooter branch).
pub struct Bot {
    pub team: usize,
    pub slot: usize,
    diff: Difficulty,
    rng: Rng,
    // ---- mode / goals
    mode: BotMode,
    path: Option<Vec<usize>>,
    pi: usize,
    goal: usize,
    goal_timer: f32,
    repath: f32,
    think: f32,
    // ---- target
    target: Option<(usize, usize)>,
    see_timer: f32,
    react: f32,
    lost_timer: f32,
    // ---- aim spring (JS `aimYaw/aimPitch/aimYawV/aimPitchV`)
    aim_yaw: f32,
    aim_pitch: f32,
    aim_yaw_v: f32,
    aim_pitch_v: f32,
    // ---- humanised error
    ph1: f32,
    ph2: f32,
    t_time: f32,
    acq_t: f32,
    acq_sign_y: f32,
    acq_sign_p: f32,
    strafe: f32,
    strafe_t: f32,
    strafe_s: f32,
    strafe_amp: f32,
    // ---- move slew
    mv_yaw: f32,
    mv_mag: f32,
    // ---- paint
    sweep: f32,
    paint_scan_t: f32,
    paint_yaw_off: f32,
    // ---- refill / retreat
    refill_until: f32,
    retreat_t: f32,
    // ---- stuck recovery
    best_d: f32,
    no_prog: f32,
    jump_cd: f32,
    dodge_cd: f32,
    strikes: u32,
    strike_t: f32,
    wiggle_t: f32,
    wiggle_yaw: f32,
    need_jump: bool,
    air_still: f32,
    skipped: bool,
    ledge_n: u32,
    ledge_t: f32,
    // ---- displacement watchdog (JS `dispT/moveAcc/snap`)
    disp_t: f32,
    move_acc: f32,
    snap: Vec3,
    // ---- water guard side memory (JS `_waterSide`)
    water_side: f32,
    // ---- intent edges
    prev_fire: bool,
    prev_squid: bool,
    was_dead: bool,
    /// Whether the last frame's fire command was accepted (JS `_firing`,
    /// widens the aim tolerance while a burst is already running).
    firing: bool,
    pub stats: BotStats,
    last_pos: Vec3,
}

impl Bot {
    /// JS `new BotBrain(actor, 'easy')` + `reset()`. `seed` derives the bot's
    /// private RNG stream (deterministic per match seed + roster index).
    #[must_use]
    pub fn new(a: &Actor, diff: &Difficulty, seed: u64, roster_index: usize) -> Self {
        // Mix the stream seed through a golden-ratio multiply so adjacent
        // roster indices don't fold (via xorshift32's `lo ^ hi`) into
        // near-identical states — that would synchronise the bots' random
        // draws and collapse their behavioural diversity.
        let stream_seed = (seed ^ 0x9e37_79b9_5c4e_35ea ^ ((roster_index as u64) << 3))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut rng = Rng::new(stream_seed);
        // JS `new BotBrain` randomises these fields in a fixed order; the same
        // draw order keeps the RNG stream deterministic.
        let r_think = rng.next_f32();
        let r_ph1 = rng.next_f32();
        let r_ph2 = rng.next_f32();
        let r_tt = rng.next_f32();
        let r_sweep = rng.next_f32();
        let r_dodge = rng.next_f32();
        Self {
            team: a.team,
            slot: a.slot,
            diff: diff.clone(),
            rng,
            mode: BotMode::Paint,
            path: None,
            pi: 0,
            goal: usize::MAX,
            goal_timer: 0.0,
            repath: 0.0,
            think: r_think * 0.2,
            target: None,
            see_timer: 0.0,
            react: 0.0,
            lost_timer: 0.0,
            aim_yaw: a.yaw,
            aim_pitch: 0.0,
            aim_yaw_v: 0.0,
            aim_pitch_v: 0.0,
            ph1: r_ph1 * 20.0,
            ph2: r_ph2 * 20.0,
            t_time: r_tt * 10.0,
            acq_t: 9.0,
            acq_sign_y: 0.0,
            acq_sign_p: 0.0,
            strafe: 1.0,
            strafe_t: 0.0,
            strafe_s: 0.0,
            strafe_amp: 1.0,
            mv_yaw: a.yaw,
            mv_mag: 0.0,
            sweep: r_sweep * 10.0,
            paint_scan_t: 0.0,
            paint_yaw_off: 0.0,
            refill_until: 0.0,
            retreat_t: 0.0,
            best_d: f32::INFINITY,
            no_prog: 0.0,
            jump_cd: 0.0,
            dodge_cd: 1.0 + r_dodge * 2.0,
            strikes: 0,
            strike_t: 0.0,
            wiggle_t: 0.0,
            wiggle_yaw: 0.0,
            need_jump: false,
            air_still: 0.0,
            skipped: false,
            ledge_n: 0,
            ledge_t: -9.0,
            disp_t: 0.0,
            move_acc: 0.0,
            snap: a.pos,
            water_side: 1.0,
            prev_fire: false,
            prev_squid: false,
            was_dead: false,
            firing: false,
            stats: BotStats::default(),
            last_pos: a.pos,
        }
    }

    /// Current paint goal node (for [`BotCtx::mate_goals`]).
    #[must_use]
    pub fn goal_node(&self) -> Option<usize> {
        if self.goal == usize::MAX {
            None
        } else {
            Some(self.goal)
        }
    }

    /// JS `BotBrain.update` for the shooter/easy branch: returns this frame's
    /// intent and writes the aim angles onto `a` (JS `a.aimYaw = ...`).
    pub fn step(&mut self, dt: f32, a: &mut Actor, ctx: &BotCtx) -> ActorInput {
        let mut it = ActorInput::default();
        // ---- stats: travel + death bookkeeping (report-only)
        if a.alive {
            self.stats.dist += (a.pos.x - self.last_pos.x).hypot(a.pos.z - self.last_pos.z);
        }
        self.last_pos = a.pos;
        self.stats.deaths = a.deaths;
        self.stats.splats = a.splats;

        // ---- dead: zero intents, clear the plan (JS L317).
        if !a.alive {
            self.path = None;
            self.target = None;
            self.mv_mag = 0.0;
            self.was_dead = true;
            self.stats.mode = self.mode;
            return it;
        }
        // just respawned: re-aim along the body facing (JS L318-321; the
        // super-jump-to-teammate branch is M2+ specials scope).
        if self.was_dead {
            self.was_dead = false;
            self.aim_yaw = a.yaw;
            self.aim_pitch = 0.0;
            self.aim_yaw_v = 0.0;
            self.aim_pitch_v = 0.0;
        }

        // ---- timers (JS L343-344)
        self.think -= dt;
        self.jump_cd -= dt;
        self.strafe_t -= dt;
        self.dodge_cd -= dt;
        self.acq_t += dt;
        self.t_time += dt;

        // ---- perception (JS L348-351)
        if self.think <= 0.0 {
            self.think = 0.15 + self.rng.next_f32() * 0.1;
            self.perceive(a, ctx);
        }
        if let Some(tid) = self.target
            && let Some(idx) = find_actor(ctx.actors, tid.0, tid.1)
            && !ctx.actors[idx].alive
        {
            self.target = None;
        }

        // ---- mode selection (JS L359-381)
        let ink_frac = a.ink / ctx.t.ink_max;
        let hp_frac = a.hp / ctx.t.hp;
        let last_damage = a.last_damage();
        self.select_mode(dt, ink_frac, hp_frac, last_damage);

        // ---- navigation goal (JS L383-406)
        self.goal_timer -= dt;
        self.repath -= dt;
        match self.mode {
            BotMode::Fight => {
                if let Some(tid) = self.target
                    && self.repath <= 0.0
                    && let Some(idx) = find_actor(ctx.actors, tid.0, tid.1)
                {
                    let p = ctx.actors[idx].pos;
                    self.path_to(ctx, a.pos, a.grounded, p, 0.6);
                }
            }
            BotMode::Refill => {
                if self.repath <= 0.0 || self.path.is_none() {
                    self.pick_refill(a, ctx);
                }
            }
            BotMode::Retreat => {
                if self.repath <= 0.0 || self.path.is_none() {
                    self.pick_retreat(a, ctx);
                }
            }
            BotMode::Paint => {
                if self.goal_timer <= 0.0
                    || self.path.is_none()
                    || self.pi >= self.path.as_ref().map_or(0, |p| p.len())
                {
                    self.pick_paint_goal(a, ctx);
                }
            }
        }

        // ---- steering (JS L409-412)
        let mut move_v = self.steer(a, ctx, dt);
        self.unstick(dt, a, &mut move_v);
        if self.path.is_none() && self.wiggle_t <= 0.0 {
            self.back_on_nav(a, ctx, &mut move_v);
        }
        let want_move = move_v.length_squared() > 0.01;

        // ---- actions (JS L415-618)
        let mut want_yaw = if want_move {
            move_v.x.atan2(move_v.z)
        } else {
            a.yaw
        };
        let mut want_pitch = -0.1f32;
        let enemy_visible = self.target.is_some() && self.see_timer > 0.0;

        let tgt_idx = if enemy_visible {
            self.target.and_then(|(t, s)| find_actor(ctx.actors, t, s))
        } else {
            None
        };
        if (self.mode == BotMode::Fight || self.mode == BotMode::Retreat)
            && let Some(ti) = tgt_idx
        {
            let t = &ctx.actors[ti];
            let dx = t.pos.x - a.pos.x;
            let dz = t.pos.z - a.pos.z;
            let dist = dx.hypot(dz);
            let range = ctx.w.range;
            // lead the target by the projectile flight time (JS L428)
            let lead = dist / ctx.w.proj_speed;
            let tx = t.pos.x + t.vel.x * lead;
            let ty = t.pos.y + t.smooth_y() + if t.form == Form::Squid { 0.3 } else { 0.85 };
            let tz = t.pos.z + t.vel.z * lead;
            let vx = tx - a.pos.x;
            let vy = ty - (a.pos.y + 1.1);
            let vz = tz - a.pos.z;
            let ideal_yaw = vx.atan2(vz);
            let ideal_pitch = vy.atan2(vx.hypot(vz));
            // human aim error: slow wander + acquisition overshoot settling
            // over the reaction window (JS L436-440)
            let e = self.diff.aim_error;
            let acq = (-self.acq_t / (0.12f32).max(self.diff.reaction * 0.9)).exp();
            want_yaw = ideal_yaw
                + e * (0.75 * wander(self.t_time * 1.7 + self.ph1) + 2.4 * acq * self.acq_sign_y);
            want_pitch = ideal_pitch
                + e * 0.6
                    * (0.75 * wander(self.t_time * 2.1 + self.ph2) + 1.6 * acq * self.acq_sign_p);
            if self.mode == BotMode::Fight {
                self.fight(
                    a,
                    ctx,
                    dt,
                    &mut move_v,
                    &mut it,
                    dist,
                    range,
                    ideal_yaw,
                    ideal_pitch,
                    want_move,
                    ink_frac,
                    ti,
                );
            } else {
                // retreat: swim away through own ink, eyes on the threat
                it.squid = true;
            }
        } else if self.mode == BotMode::Paint {
            self.paint(
                a,
                ctx,
                dt,
                &move_v,
                &mut it,
                &mut want_yaw,
                &mut want_pitch,
                want_move,
                ink_frac,
            );
        } else if self.mode == BotMode::Refill {
            self.refill(a, ctx, &mut it, &mut want_pitch);
        }

        self.tail(
            dt,
            a,
            ctx,
            &move_v,
            want_yaw,
            want_pitch,
            want_move,
            enemy_visible,
            &mut it,
        );
        if it.fire && !self.prev_fire {
            self.stats.shots += 1;
        }
        if it.squid && !self.prev_squid {
            self.stats.swims += 1;
        }
        self.prev_fire = it.fire;
        self.prev_squid = it.squid;
        self.stats.mode = self.mode;
        it
    }

    /// JS `update` L317-321 + L342: the dead-plan-clear runs every frame, but
    /// outside `playing` the brain early-exits — timers and RNG stay frozen.
    /// The autopilot calls this during [`crate::match_::Phase::Intro`] /
    /// [`crate::match_::Phase::Finish`] instead of [`Bot::step`].
    pub fn hold(&mut self, a: &Actor) -> ActorInput {
        if a.alive {
            self.stats.dist += (a.pos.x - self.last_pos.x).hypot(a.pos.z - self.last_pos.z);
        }
        self.last_pos = a.pos;
        self.stats.deaths = a.deaths;
        self.stats.splats = a.splats;
        if !a.alive {
            self.path = None;
            self.target = None;
            self.mv_mag = 0.0;
            self.was_dead = true;
        }
        self.prev_fire = false;
        self.prev_squid = false;
        self.stats.mode = self.mode;
        ActorInput::default()
    }

    // ---------------------------------------------------------------- modes

    /// JS `update` L359-381 (turf-war branch; no kit vetoes).
    fn select_mode(&mut self, dt: f32, ink_frac: f32, hp_frac: f32, last_damage: f32) {
        if self.mode == BotMode::Retreat {
            self.retreat_t -= dt;
            if hp_frac > 0.85 || self.retreat_t <= 0.0 || (self.target.is_none() && hp_frac > 0.6) {
                self.mode = BotMode::Paint;
                self.path = None;
                self.goal_timer = 0.0;
            }
        } else if self.target.is_some()
            && self.see_timer > 0.0
            && ((hp_frac < 0.34 && last_damage < 0.8) || hp_frac < 0.2)
            && self.rng.next_f32() < 0.6 * dt * 60.0 * self.diff.fire_discipline
        {
            // losing the duel: break line of sight and heal in own ink
            self.mode = BotMode::Retreat;
            self.retreat_t = 2.2 + self.rng.next_f32() * 1.4;
            self.repath = 0.0;
        }
        if self.mode != BotMode::Refill
            && self.mode != BotMode::Retreat
            && ink_frac < 0.12
            && !(self.target.is_some() && self.see_timer > 0.0 && ink_frac > 0.05)
        {
            self.mode = BotMode::Refill;
            self.refill_until = 0.85 + self.rng.next_f32() * 0.1;
        }
        if self.mode == BotMode::Refill && ink_frac >= self.refill_until {
            self.mode = BotMode::Paint;
        }
        if self.mode != BotMode::Refill && self.mode != BotMode::Retreat {
            self.mode = if self.target.is_some() {
                BotMode::Fight
            } else {
                BotMode::Paint
            };
        }
    }

    // ---------------------------------------------------------------- perceive

    /// JS `_perceive`.
    fn perceive(&mut self, a: &Actor, ctx: &BotCtx) {
        let eye = Vec3::new(a.pos.x, a.pos.y + EYE_Y, a.pos.z);
        let aw = self.diff.awareness;
        let mut best: Option<(usize, usize)> = None;
        let mut bd = f32::INFINITY;
        for e in ctx.actors {
            if e.team == a.team || !e.alive {
                continue;
            }
            let d = e.pos.distance(a.pos);
            if d > aw {
                continue;
            }
            // JS `e.anim.form === 'swim'`: squid *and* submerged; a kid
            // squid-ing on dry land is still fully perceptible.
            let swimming = e.form == Form::Squid && e.submerged;
            let hs = e.vel.x.hypot(e.vel.z);
            if swimming && d > 3.0 && !(hs > 7.0 && d < 9.0) {
                continue;
            }
            let head = Vec3::new(
                e.pos.x,
                e.pos.y + if e.form == Form::Squid { 0.3 } else { 1.0 },
                e.pos.z,
            );
            if !ctx.world.collision.los(eye, head) {
                continue;
            }
            let mut score = d;
            if let Some((tt, ts)) = self.target
                && tt == e.team
                && ts == e.slot
            {
                score -= 4.0;
            }
            if score < bd {
                bd = score;
                best = Some((e.team, e.slot));
            }
        }
        if let Some(b) = best {
            if self.target != Some(b) {
                self.target = Some(b);
                self.react = self.diff.reaction * (0.7 + self.rng.next_f32() * 0.6);
                self.repath = 0.0;
                // first look lands a little off and settles (JS L1643-1644)
                self.acq_t = 0.0;
                self.acq_sign_y = (if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 })
                    * (0.5 + self.rng.next_f32() * 0.5);
                self.acq_sign_p = (self.rng.next_f32() - 0.5) * 1.2;
            }
            self.see_timer = 1.2;
            self.lost_timer = 0.0;
        } else {
            self.see_timer -= 0.2;
            if let Some((tt, ts)) = self.target {
                let far = find_actor(ctx.actors, tt, ts)
                    .is_none_or(|idx| ctx.actors[idx].pos.distance(a.pos) > aw + 6.0);
                self.lost_timer += 0.2;
                if self.lost_timer > 2.5 || far {
                    self.target = None;
                }
            }
        }
        self.react -= 0.2;
    }

    // ---------------------------------------------------------------- goals

    /// JS `_pathTo` (`start_pos` is the bot's own position, passed by the
    /// caller so `ctx.actors` stays immutable-borrowed; `grounded` mirrors
    /// the JS `this.a.grounded` guard on the step-foot re-snap).
    fn path_to(
        &mut self,
        ctx: &BotCtx,
        start_pos: Vec3,
        grounded: bool,
        pos: Vec3,
        max_up: f32,
    ) -> bool {
        let mut s = ctx.nav.nearest(start_pos, 1.2, true);
        // standing at the foot of a step: don't start on the ledge above
        if let Some(sid) = s
            && grounded
            && ctx.nav.nodes[sid].pos.y - start_pos.y > 0.5
            && let Some(s2) = ctx.nav.nearest(start_pos, 0.45, true)
        {
            s = Some(s2);
        }
        self.repath = 0.8 + self.rng.next_f32() * 0.4;
        let (Some(start), Some(goal)) = (s, ctx.nav.nearest(pos, max_up, false)) else {
            self.path = None;
            return false;
        };
        match ctx.nav.path(start, goal, self.team) {
            Some(p) => {
                self.pi = 1.min(p.len() - 1);
                self.goal = goal;
                self.best_d = f32::INFINITY;
                self.no_prog = 0.0;
                self.path = Some(p);
                true
            }
            None => {
                self.path = None;
                false
            }
        }
    }

    /// JS `_pickPaintGoal` (turf-war branch; CHARGES perch bonus dropped —
    /// the Spritzer is not a charger).
    fn pick_paint_goal(&mut self, a: &Actor, ctx: &BotCtx) {
        let enemy_pad = ctx.world.spawn_pads[1 - a.team];
        let own_pad = ctx.world.spawn_pads[a.team];
        let total = enemy_pad.distance(own_pad).max(1.0);
        let my_idx = find_actor(ctx.actors, a.team, a.slot).unwrap_or(usize::MAX);
        let ids = &ctx.nav.valid_ids;
        let mut best = usize::MAX;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..28 {
            if ids.is_empty() {
                break;
            }
            let k = ((self.rng.next_f32() * ids.len() as f32) as usize).min(ids.len() - 1);
            let id = ids[k];
            let n = &ctx.nav.nodes[id];
            if n.zone >= 0 || n.wet == 2 {
                continue;
            }
            let d = (n.pos.x - a.pos.x).hypot(n.pos.z - a.pos.z);
            if d > 38.0 {
                continue;
            }
            let near =
                ctx.paint
                    .region_stats(ctx.world.collision, n.pos.x, n.pos.y, n.pos.z, 3.0, a.team);
            if near.n == 0 {
                continue;
            }
            let v_near = near.empty + near.enemy * 1.4;
            let wide =
                ctx.paint
                    .region_stats(ctx.world.collision, n.pos.x, n.pos.y, n.pos.z, 6.5, a.team);
            let value = v_near * 0.55 + (wide.empty + wide.enemy * 1.4) * 0.45;
            let progress = 1.0 - (n.pos.x - enemy_pad.x).hypot(n.pos.z - enemy_pad.z) / total;
            let mut score = value * 16.0 - d * 0.14
                + progress.clamp(0.0, 0.8) * 2.5
                + self.rng.next_f32() * 1.5
                - if n.wet != 0 { 1.5 } else { 0.0 };
            if value < 0.15 {
                score -= 8.0;
            }
            for (i, m) in ctx.actors.iter().enumerate() {
                if m.team != a.team || i == my_idx {
                    continue;
                }
                if let Some(g) = ctx.mate_goals.get(i).copied().flatten() {
                    let gn = &ctx.nav.nodes[g];
                    if (gn.pos.x - n.pos.x).hypot(gn.pos.z - n.pos.z) < 8.0 {
                        score -= 5.0;
                    }
                }
            }
            if score > bs {
                bs = score;
                best = id;
            }
        }
        self.goal_timer = 4.0 + self.rng.next_f32() * 3.0;
        if best == usize::MAX {
            return;
        }
        let n = ctx.nav.nodes[best].pos;
        self.path_to(ctx, a.pos, a.grounded, n, 0.3);
    }

    /// JS `_pickRefill`.
    fn pick_refill(&mut self, a: &Actor, ctx: &BotCtx) {
        let mut best_p: Option<Vec3> = None;
        let mut bd = f32::INFINITY;
        for _ in 0..14 {
            let ang = self.rng.next_f32() * std::f32::consts::TAU;
            let r = 1.0 + self.rng.next_f32() * 7.0;
            let p = Vec3::new(a.pos.x + ang.cos() * r, a.pos.y, a.pos.z + ang.sin() * r);
            let st = ctx
                .paint
                .region_stats(ctx.world.collision, p.x, p.y, p.z, 1.2, a.team);
            if st.n != 0 && st.own > 0.6 && r < bd {
                bd = r;
                best_p = Some(p);
            }
        }
        for _ in 0..16 {
            if best_p.is_some() {
                break;
            }
            let ang = self.rng.next_f32() * std::f32::consts::TAU;
            let r = 8.0 + self.rng.next_f32() * 14.0;
            let p = Vec3::new(a.pos.x + ang.cos() * r, a.pos.y, a.pos.z + ang.sin() * r);
            let st = ctx
                .paint
                .region_stats(ctx.world.collision, p.x, p.y, p.z, 1.5, a.team);
            if st.n != 0 && st.own > 0.6 {
                best_p = Some(p);
            }
        }
        let pad = ctx.world.spawn_pads[a.team];
        let ok = match best_p {
            Some(p) => self.path_to(ctx, a.pos, a.grounded, p, 0.4),
            None => false,
        };
        if !ok {
            self.path_to(ctx, a.pos, a.grounded, pad, 1.2);
        }
        self.repath = 1.2;
    }

    /// JS `_pickRetreat`.
    fn pick_retreat(&mut self, a: &Actor, ctx: &BotCtx) {
        let ti = self.target.and_then(|(t, s)| find_actor(ctx.actors, t, s));
        let mut best_p: Option<Vec3> = None;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..16 {
            let ang = self.rng.next_f32() * std::f32::consts::TAU;
            let r = 3.0 + self.rng.next_f32() * 8.0;
            let p = Vec3::new(a.pos.x + ang.cos() * r, a.pos.y, a.pos.z + ang.sin() * r);
            let st = ctx
                .paint
                .region_stats(ctx.world.collision, p.x, p.y, p.z, 1.4, a.team);
            if st.n == 0 {
                continue;
            }
            let away = ti.map_or(0.0, |i| {
                let t = &ctx.actors[i];
                (p.x - t.pos.x).hypot(p.z - t.pos.z) - (a.pos.x - t.pos.x).hypot(a.pos.z - t.pos.z)
            });
            let los_safe = ti.is_some_and(|i| {
                let t = &ctx.actors[i];
                !ctx.world.collision.los(
                    Vec3::new(p.x, p.y + 1.0, p.z),
                    Vec3::new(t.pos.x, t.pos.y + 1.0, t.pos.z),
                )
            });
            let score = st.own * 6.0 + away * 0.8 - r * 0.15 + if los_safe { 4.0 } else { 0.0 };
            if score > bs {
                bs = score;
                best_p = Some(p);
            }
        }
        match best_p {
            Some(p) => {
                self.path_to(ctx, a.pos, a.grounded, p, 0.5);
            }
            None => self.path = None,
        }
        self.repath = 1.0;
    }

    // ---------------------------------------------------------------- fight

    /// JS `update` fight block, shooter branch (L441-510): preferred
    /// distance + eased strafing, fire when the *actual* aim is on the body,
    /// swim-in over own ink, dodge-hop after a hit.
    #[allow(clippy::too_many_arguments)]
    fn fight(
        &mut self,
        a: &mut Actor,
        ctx: &BotCtx,
        dt: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        dist: f32,
        range: f32,
        ideal_yaw: f32,
        ideal_pitch: f32,
        want_move: bool,
        ink_frac: f32,
        ti: usize,
    ) {
        let t = &ctx.actors[ti];
        let dx = t.pos.x - a.pos.x;
        let dz = t.pos.z - a.pos.z;
        let pref = range * FIGHT_PREF_F;
        if self.strafe_t <= 0.0 {
            self.strafe_t = 0.6 + self.rng.next_f32() * 1.2;
            self.strafe = if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
            self.strafe_amp = 0.5 + self.rng.next_f32() * 0.5;
        }
        self.strafe_s +=
            (self.strafe * self.strafe_amp - self.strafe_s) * (1.0 - (-5.0 * dt).exp());
        let nx = dx / dist.max(0.01);
        let nz = dz / dist.max(0.01);
        let mut mvx = 0.0;
        let mut mvz = 0.0;
        if dist > pref + 1.2 && want_move {
            mvx = move_v.x;
            mvz = move_v.z;
        } else if dist < pref - 1.5 {
            mvx = -nx;
            mvz = -nz;
        }
        mvx += -nz * self.strafe_s * 0.9;
        mvz += nx * self.strafe_s * 0.9;
        let l = mvx.hypot(mvz);
        *move_v = if l > 0.01 {
            Vec3::new(mvx / l, 0.0, mvz / l)
        } else {
            Vec3::ZERO
        };

        // fire only when the actual aim is on the body (JS L473-477)
        let off = Vec2::new(
            angle_diff(self.aim_yaw, ideal_yaw),
            self.aim_pitch - ideal_pitch,
        )
        .length();
        let tol = (0.05f32).max((AIM_TOL_HALF / dist).atan()) * if self.firing { 2.4 } else { 1.5 };
        let aimed = off < tol;
        self.firing = false;
        if self.see_timer > 0.0 && self.react <= 0.0 && aimed && ink_frac > 0.02 {
            it.fire = dist < range * 1.08;
            self.firing = it.fire;
        }
        // out of range with own ink underfoot: swim in (JS L509-510)
        if !it.fire && dist > range * 1.15 && a.ground_team == 1 {
            it.squid = true;
        }
        // dodge-hop right after taking a hit (JS L519-523, Spritzer branch)
        if a.last_damage() < 0.25
            && self.dodge_cd <= 0.0
            && a.grounded
            && self.rng.next_f32() < 0.3
            && !self.near_water(a, ctx, 1.6)
        {
            it.jump = true;
            self.dodge_cd = 2.0 + self.rng.next_f32() * 2.5;
        }
    }

    // ---------------------------------------------------------------- paint

    /// JS `update` paint block, shooter branch (L538-599).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        move_v: &Vec3,
        it: &mut ActorInput,
        want_yaw: &mut f32,
        want_pitch: &mut f32,
        want_move: bool,
        ink_frac: f32,
    ) {
        self.sweep += dt * 2.1;
        self.paint_scan_t -= dt;
        if self.paint_scan_t <= 0.0 {
            self.paint_scan_t = 0.35 + self.rng.next_f32() * 0.15;
            let mut best_off = 0.0f32;
            let mut best_v = -1.0f32;
            for off in [0.0f32, -0.6, 0.6, -1.2, 1.2] {
                let yw = (if want_move {
                    move_v.x.atan2(move_v.z)
                } else {
                    self.aim_yaw
                }) + off;
                let st = ctx.paint.region_stats(
                    ctx.world.collision,
                    a.pos.x + yw.sin() * PAINT_REACH,
                    a.pos.y,
                    a.pos.z + yw.cos() * PAINT_REACH,
                    2.2,
                    a.team,
                );
                let v = if st.n != 0 {
                    st.empty + st.enemy * 1.4 - off.abs() * 0.12
                } else {
                    -1.0
                };
                if v > best_v {
                    best_v = v;
                    best_off = off;
                }
            }
            self.paint_yaw_off = best_off;
        }
        *want_yaw += self.paint_yaw_off + self.sweep.sin() * 0.35;
        *want_pitch = PAINT_PITCH;
        let ahead = ctx.paint.region_stats(
            ctx.world.collision,
            a.pos.x + want_yaw.sin() * 4.0,
            a.pos.y,
            a.pos.z + want_yaw.cos() * 4.0,
            3.0,
            a.team,
        );
        let need_paint = ahead.n == 0 || ahead.own < 0.75;
        it.fire = need_paint && ink_frac > 0.18;
        // travel as a squid through own ink when not painting (JS L599)
        if !it.fire && self.path_remaining(a, ctx) > 5.0 && a.ground_team == 1 {
            it.squid = true;
        }
    }

    /// JS `update` refill block (L611-617).
    fn refill(&self, a: &Actor, ctx: &BotCtx, it: &mut ActorInput, want_pitch: &mut f32) {
        it.squid = a.ground_team == 1 || self.path_remaining(a, ctx) > 2.0;
        if a.ground_team != 1 && self.path_remaining(a, ctx) < 1.5 && a.ink / ctx.t.ink_max > 0.03 {
            // no ink here: paint a puddle to swim in
            it.squid = false;
            it.fire = true;
            *want_pitch = -1.0;
        }
    }

    fn path_remaining(&self, a: &Actor, ctx: &BotCtx) -> f32 {
        let Some(p) = &self.path else {
            return 0.0;
        };
        let n = ctx.nav.nodes[*p.last().unwrap()].pos;
        (n.x - a.pos.x).hypot(n.z - a.pos.z)
    }

    // ---------------------------------------------------------------- tail

    /// JS `_tail`: aim spring, water guard, move slew, edge guard, stuck and
    /// displacement watchdogs.
    #[allow(clippy::too_many_arguments)]
    fn tail(
        &mut self,
        dt: f32,
        a: &mut Actor,
        ctx: &BotCtx,
        move_v: &Vec3,
        want_yaw: f32,
        want_pitch: f32,
        want_move: bool,
        enemy_visible: bool,
        it: &mut ActorInput,
    ) {
        // ---- aim spring (JS L641-654)
        let fighting = self.mode == BotMode::Fight;
        let snappy = fighting;
        let om = if snappy { self.diff.aim_omega } else { 8.0 };
        let max_rate = if snappy { self.diff.aim_turn } else { 6.0 };
        let want_pitch = want_pitch.clamp(-1.1, 1.0);
        self.aim_yaw_v +=
            (om * om * angle_diff(self.aim_yaw, want_yaw) - 2.0 * om * self.aim_yaw_v) * dt;
        self.aim_yaw_v = self.aim_yaw_v.clamp(-max_rate, max_rate);
        self.aim_yaw += self.aim_yaw_v * dt;
        if self.aim_yaw > std::f32::consts::PI {
            self.aim_yaw -= std::f32::consts::TAU;
        } else if self.aim_yaw < -std::f32::consts::PI {
            self.aim_yaw += std::f32::consts::TAU;
        }
        self.aim_pitch_v +=
            (om * om * (want_pitch - self.aim_pitch) - 2.0 * om * self.aim_pitch_v) * dt;
        self.aim_pitch_v = self.aim_pitch_v.clamp(-max_rate * 0.7, max_rate * 0.7);
        self.aim_pitch = (self.aim_pitch + self.aim_pitch_v * dt).clamp(-1.1, 1.0);
        a.aim_yaw = self.aim_yaw;
        a.aim_pitch = self.aim_pitch;

        // ---- water guards (JS L663-665)
        let mut guarded = *move_v;
        self.avoid_water(a, ctx, &mut guarded);
        if it.squid && self.squid_would_drop(a, ctx, guarded) {
            it.squid = false;
        }

        // ---- move slew (JS L667-681)
        let ml = guarded.length().min(1.0);
        if ml > 0.01 {
            let des = guarded.x.atan2(guarded.z);
            let d = angle_diff(self.mv_yaw, des);
            if self.mv_mag < 0.05 {
                self.mv_yaw = des;
            } else if d.abs() > 2.1 {
                self.mv_yaw = des;
                self.mv_mag *= 0.35;
            } else {
                self.mv_yaw += d.clamp(-11.0 * dt, 11.0 * dt);
            }
        }
        self.mv_mag += (ml - self.mv_mag) * (1.0 - (-14.0 * dt).exp());
        it.move_dir = Vec3::new(
            self.mv_yaw.sin() * self.mv_mag,
            0.0,
            self.mv_yaw.cos() * self.mv_mag,
        );
        if self.mv_mag > 0.05 && a.grounded {
            self.edge_guard(a, ctx, &mut it.move_dir);
        }

        // ---- waypoint-progress stuck recovery (JS L683-694)
        let trying = self.path.is_some() && want_move;
        if !trying {
            self.no_prog = 0.0;
        }
        if self.no_prog > 0.7 && self.jump_cd <= 0.0 && a.grounded && !self.near_water(a, ctx, 1.2)
        {
            it.jump = true;
            self.jump_cd = 1.0;
        }
        if self.no_prog > 1.5
            && let Some(p) = &self.path
            && self.pi + 1 < p.len()
            && !self.skipped
        {
            self.pi += 1;
            self.skipped = true;
            self.best_d = f32::INFINITY;
        }
        if self.no_prog > 2.4 {
            self.no_prog = 0.0;
            self.skipped = false;
            self.path = None;
            self.goal_timer = 0.0;
            self.repath = 0.0;
            self.strikes += 1;
            if self.strikes >= 2 {
                self.strikes = 0;
                self.wiggle(0.9, a.grounded);
            }
            self.strike_t = 8.0;
        }
        if self.no_prog == 0.0 {
            self.skipped = false;
        }
        if self.need_jump && self.jump_cd <= 0.0 && a.grounded {
            it.jump = true;
            self.jump_cd = 0.6;
            self.need_jump = false;
        }

        // ---- displacement watchdog (JS L698-708; TR-9.3 evidence)
        self.disp_t += dt;
        self.move_acc += self.mv_mag * dt;
        if self.disp_t >= 1.5 {
            let moved = (a.pos.x - self.snap.x).hypot(a.pos.z - self.snap.z);
            let wanting = self.move_acc / self.disp_t > 0.45;
            if wanting
                && moved < 0.4
                && !(self.mode == BotMode::Fight && enemy_visible)
                && a.grounded
            {
                self.stats.stalls += 1;
                self.stats.max_stall = self.stats.max_stall.max(self.disp_t);
                self.strikes += 1;
                if self.strikes >= 2 {
                    self.strikes = 0;
                    self.goal_timer = 0.0;
                }
                self.strike_t = 8.0;
                self.wiggle(0.7, a.grounded);
            }
            self.snap = a.pos;
            self.disp_t = 0.0;
            self.move_acc = 0.0;
        }
    }

    // ---------------------------------------------------------------- steer

    /// JS `_steer` (simplified lookahead — see module docs).
    fn steer(&mut self, a: &Actor, ctx: &BotCtx, dt: f32) -> Vec3 {
        let mut out = Vec3::ZERO;
        let Some(p) = self.path.clone() else {
            return out;
        };
        if self.pi >= p.len() {
            return out;
        }
        // advance reached waypoints (JS L1801-1806)
        while self.pi < p.len() {
            let n = ctx.nav.nodes[p[self.pi]].pos;
            let dx = n.x - a.pos.x;
            let dz = n.z - a.pos.z;
            let dy = n.y - a.pos.y;
            if dx * dx + dz * dz < 0.36 && dy < 0.9 && dy > -1.8 {
                self.pi += 1;
                self.best_d = f32::INFINITY;
                self.no_prog = 0.0;
            } else {
                break;
            }
        }
        if self.pi >= p.len() {
            return out;
        }
        let cur = ctx.nav.nodes[p[self.pi]].pos;
        let hd = (cur.x - a.pos.x).hypot(cur.z - a.pos.z);
        // waypoint above us and unreachable from here: replan (JS L1811-1817)
        if a.grounded && cur.y - a.pos.y > 0.9 && hd < 1.2 {
            let et = if self.pi > 0 {
                ctx.nav.edge_type(p[self.pi - 1], p[self.pi])
            } else {
                EdgeType::Walk
            };
            if et != EdgeType::Jump {
                self.path = None;
                self.repath = 0.0;
                self.goal_timer = 0.0;
                if self.t_time - self.ledge_t > 2.0 {
                    self.ledge_n = 0;
                }
                self.ledge_t = self.t_time;
                self.ledge_n += 1;
                if self.ledge_n >= 4 {
                    self.ledge_n = 0;
                    self.wiggle(0.8, a.grounded);
                }
                return out;
            }
        }
        // drop edge: keep running in the path direction to step off (JS L1820-1824)
        if cur.y - a.pos.y < -0.9 && hd < 0.8 && self.pi > 0 {
            let prev = ctx.nav.nodes[p[self.pi - 1]].pos;
            out = Vec3::new(cur.x - prev.x, 0.0, cur.z - prev.z);
            if out.length() > 0.01 {
                return out.normalize();
            }
        }
        out = Vec3::new(cur.x - a.pos.x, 0.0, cur.z - a.pos.z);
        if out.length() > 0.001 {
            out = out.normalize();
        }
        // jump edges (JS L1847-1850)
        if self.pi > 0 {
            let et = ctx.nav.edge_type(p[self.pi - 1], p[self.pi]);
            if et == EdgeType::Jump && cur.y - a.pos.y > 0.4 && hd < 1.6 {
                self.need_jump = true;
            }
        }
        // separation from other actors (JS L1852-1861)
        for o in ctx.actors {
            if o.team == self.team && o.slot == self.slot {
                continue;
            }
            if !o.alive {
                continue;
            }
            let dx = a.pos.x - o.pos.x;
            let dz = a.pos.z - o.pos.z;
            let d2 = dx * dx + dz * dz;
            if d2 < 1.96 && d2 > 1e-4 {
                let d = d2.sqrt();
                let k = (1.4 - d) * 0.7;
                let side = if dx * -out.z + dz * out.x >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                let (ox, oz) = (out.x, out.z);
                out.x = ox - oz * side * k;
                out.z = oz + ox * side * k;
            }
        }
        if out.length() > 1.0 {
            out = out.normalize();
        }
        // progress toward the current waypoint (JS L1864-1865)
        if hd < self.best_d - 0.2 {
            self.best_d = hd;
            self.no_prog = 0.0;
        } else {
            self.no_prog += dt;
        }
        out
    }

    /// JS `_unstick` (air-still escape + wiggle).
    fn unstick(&mut self, dt: f32, a: &Actor, move_v: &mut Vec3) {
        self.strike_t -= dt;
        if self.strike_t <= 0.0 {
            self.strikes = 0;
        }
        let still = !a.grounded && a.vel.y.abs() < 0.6 && a.vel.x.hypot(a.vel.z) < 0.4;
        self.air_still = if still { self.air_still + dt } else { 0.0 };
        if self.air_still > 0.4 && self.wiggle_t <= 0.0 {
            self.wiggle(0.6, a.grounded);
        }
        if self.wiggle_t > 0.0 {
            self.wiggle_t -= dt;
            *move_v = Vec3::new(self.wiggle_yaw.sin(), 0.0, self.wiggle_yaw.cos());
            self.no_prog = 0.0;
            self.best_d = f32::INFINITY;
        }
    }

    fn wiggle(&mut self, t: f32, grounded: bool) {
        self.wiggle_t = t;
        self.wiggle_yaw = self.rng.next_f32() * std::f32::consts::TAU;
        self.path = None;
        self.goal_timer = 0.0;
        self.repath = 0.0;
        if grounded {
            self.need_jump = true;
        }
    }

    /// JS `_backOnNav` (walk to the nearest graph node when off-graph).
    fn back_on_nav(&mut self, a: &Actor, ctx: &BotCtx, move_v: &mut Vec3) {
        if !a.grounded || ctx.nav.nearest(a.pos, 1.2, true).is_some() {
            return; // on the graph: the normal re-plan works
        }
        let mut best: Option<Vec3> = None;
        let mut bd = f32::INFINITY;
        for &id in &ctx.nav.valid_ids {
            let q = ctx.nav.nodes[id].pos;
            let d2 = (q.x - a.pos.x) * (q.x - a.pos.x) + (q.z - a.pos.z) * (q.z - a.pos.z);
            if d2 < 100.0 && (q.y - a.pos.y).abs() < 1.2 && ctx.nav.nodes[id].wet == 0 && d2 < bd {
                bd = d2;
                best = Some(q);
            }
        }
        if let Some(q) = best {
            let dx = q.x - a.pos.x;
            let dz = q.z - a.pos.z;
            let l = dx.hypot(dz);
            if l > 0.3 {
                *move_v = Vec3::new(dx / l, 0.0, dz / l);
            }
        }
    }

    // ---------------------------------------------------------------- water

    /// JS `_wet`: open sea under (x, z) — no ground at all below y + 0.6.
    fn wet(&self, ctx: &BotCtx, x: f32, z: f32, y: f32) -> bool {
        ctx.world.collision.ground_height(x, z, y + 0.6, false) == f32::NEG_INFINITY
    }

    /// JS `_nearWater`.
    fn near_water(&self, a: &Actor, ctx: &BotCtx, r: f32) -> bool {
        for k in 0..8 {
            let t = (k as f32 / 8.0) * std::f32::consts::TAU;
            if self.wet(ctx, a.pos.x + t.cos() * r, a.pos.z + t.sin() * r, a.pos.y) {
                return true;
            }
        }
        false
    }

    /// JS `_avoidWater`.
    fn avoid_water(&mut self, a: &Actor, ctx: &BotCtx, move_v: &mut Vec3) {
        let l = move_v.x.hypot(move_v.z);
        if l < 0.05 {
            return;
        }
        let yaw = move_v.x.atan2(move_v.z);
        let safe = |yw: f32| -> bool {
            for d in [0.7f32, 1.3] {
                // JS L762 calls `groundHeight(..., 50)` directly (no +0.6
                // like `_wet`); probe the same way.
                if ctx.world.collision.ground_height(
                    a.pos.x + yw.sin() * d,
                    a.pos.z + yw.cos() * d,
                    50.0,
                    false,
                ) == f32::NEG_INFINITY
                {
                    return false;
                }
            }
            true
        };
        if safe(yaw) {
            return;
        }
        for off in [0.5f32, -0.5, 1.0, -1.0, 1.6, -1.6, 2.2, -2.2] {
            let yw = yaw + self.water_side * off;
            if safe(yw) {
                self.water_side *= off.signum();
                *move_v = Vec3::new(yw.sin() * l, 0.0, yw.cos() * l);
                return;
            }
        }
        *move_v = Vec3::ZERO;
    }

    /// JS `_edgeGuard`.
    fn edge_guard(&self, a: &Actor, ctx: &BotCtx, mv: &mut Vec3) {
        let m = mv.x.hypot(mv.z);
        if m < 1e-4 {
            return;
        }
        let dx = mv.x / m;
        let dz = mv.z / m;
        let look = 0.6 + a.vel.x.hypot(a.vel.z) * 0.17;
        let (px, py, pz) = (a.pos.x, a.pos.y, a.pos.z);
        let bad = |ux: f32, uz: f32| -> bool {
            self.wet(ctx, px + ux * 0.45, pz + uz * 0.45, py)
                || self.wet(ctx, px + ux * look, pz + uz * look, py)
        };
        if !bad(dx, dz) {
            return;
        }
        for ang in [0.8f32, -0.8, 1.45, -1.45] {
            let c = ang.cos();
            let s = ang.sin();
            let nx = dx * c + dz * s;
            let nz = -dx * s + dz * c;
            if !bad(nx, nz) {
                *mv = Vec3::new(nx * m, 0.0, nz * m);
                return;
            }
        }
        *mv = Vec3::ZERO;
    }

    /// JS `_squidWouldDrop` (grates do not hold a squid over water).
    fn squid_would_drop(&self, a: &Actor, ctx: &BotCtx, move_v: Vec3) -> bool {
        let c = ctx.world.collision;
        if c.ground_height(a.pos.x, a.pos.z, 50.0, true) == f32::NEG_INFINITY {
            return true;
        }
        let l = move_v.x.hypot(move_v.z);
        l > 0.05
            && c.ground_height(
                a.pos.x + move_v.x / l * 1.2,
                a.pos.z + move_v.z / l * 1.2,
                50.0,
                true,
            ) == f32::NEG_INFINITY
    }
}

/// JS `angleDiff`.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = b - a;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// JS `wander` (bots.js L57).
fn wander(x: f32) -> f32 {
    x.sin() * 0.6 + (x * 2.27 + 1.3).sin() * 0.4
}

#[cfg(test)]
mod tests;
