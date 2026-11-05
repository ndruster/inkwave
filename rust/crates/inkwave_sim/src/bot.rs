//! Easy bot brain (spec Task 9): patrol-and-paint → spot → engage with the
//! `difficulty.easy` reaction/aim-error parameters → break off to heal /
//! refill → respawn and continue. Faithful *simplification* of
//! `src/game/bots.js` (`BotBrain`): the upstream state machine, perception,
//! aim spring and stuck-recovery are ported; the M1 Spritzer-only build
//! dropped the per-kind branches (Task 38 batch 1 restores them), and M2+
//! systems are out of scope.
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
//! Task 38 batch 1 wires the per-kind branches for the eight non-kit roster
//! kinds (`shooter` / `twins` / `blaster` / `spinner` / `charger` / `roller`
//! / `brush` / `bucket`): the MELEE / CHARGES tables (bots.js L18-19), the
//! fight dispatch (L478-503), the paint ladder (L540-597) and the tail
//! `trying` gate (L683). The retired kinds (dualies / slosher / splatling)
//! never enter the enum, so their branches are absent by construction.
//!
//! Task 38 batch 2 wires the four kit main-weapon hooks (`MAIN_KITS[k].bot`:
//! brolly / blade / bow / mitts — bots.js L478-503 / L528 / L557 / L367 /
//! L1627), the sub decision chain (`_fightSub` L950-967, `_paintSub`
//! L969-983, the `bombCd` timer L286/L343, the `_bombAim` / `_releaseBomb`
//! two-frame release L638-639 and the five `SUB_KITS[k].bot` hooks) and the
//! special chain (`_wantSpecial` L830-856, `_specialCtl` L859-948). Batch 2
//! also activates the kit-side runtime family-table patches — `MELEE.blade`
//! (blade.js L35) / `MELEE.mitts` (mitts.js L833) / `CHARGES.bow` +
//! `LONG.bow` (bow.js L492-493) / `THROWN.waddle|torpedo|shaker` (waddle.js
//! L768 / torpedo.js L750 / shaker.js L618 / boomerang.js L594) — so the
//! Cutlass and the Mitts
//! now get full MELEE treatment in `fight` / `paint` and the Bow joins the
//! charging-long ladder (see `WeaponKind::is_melee` / `is_charge` /
//! `is_long` and `SubKind::is_thrown` docs).
//!
//! Out of scope (per spec §6 batches):
//!   - Boss Battle, super jumps. (The threat AI — Waddle / Torpedo devices
//!     and enemy canopies — is Task 47, in [`bot::threat`].)
//!   - Climb edges (see `nav` module docs): the bot never plans a climb.
//!
//! Batch-2 segments cut from the JS originals (recorded for review):
//!   - the Cutlass deep fight tactics (`botTactics` in blade.js — pre-charge,
//!     flank, swim-in; only `stayIn` / `targetBias` / `paint` / `paintPitch`
//!     are ported) and its `_blHold` bookkeeping — restored by Task 47
//!     batch D in [`bot::blade_tactics`] (the `paint` hook's hold stamp too);
//!   - the Mitts leap / wall-cling driving, mid-range pounce and travel
//!     pounce (mitts.js L768-779 / L783-813 / L822-829) — `fight` keeps the
//!     reach punch, `paint` the quick-cut tail, `tactics` the 5–14 m
//!     approach; restored by Task 47 batch D (`fight` / `paint` run the full
//!     JS branches, leaps ride [`crate::actor::MittsReq`] through the kit's
//!     jump hook);
//!   - the brolly launched-canopy advance segment (brolly.js L586-592) —
//!     `_shieldFight` / `_canopyCtl` (bots.js L525 / L623-626) and the stamp
//!     threat-steer (L460-470) are restored by Task 47 in [`bot::threat`];
//!   - the climb aim override in `_tail`;
//!   - the bow group-aim `botYaw` on release (bow.js L473 — the runner fires
//!     along the bot's own aim spring);
//!   - the `_zoneJump` / `zJumpAt` super-jump-to-the-zone arms (bots.js
//!     L1026 / L986-1016 — the sim has no super jumps; batch 3 wires every
//!     other Zone Control arm: `_zoneSync` / `_zoneEngage` /
//!     `_zoneHoldGround` / `_pickZoneGoal` + the four spot pickers /
//!     `_zoneAim` / `_zoneSpecial` / `_zoneBombAim` and the strike
//!     `_zoneAimed` retarget).
//!
//! Intentional deviations from JS (recorded for review):
//!   - **`Math.random()`** → the bot's own seeded [`Rng`] stream (one per
//!     bot, derived from the match seed + roster index): the sim determinism
//!     contract (AC-6 / Task 8 module docs). The draw *count and purpose*
//!     mirror JS; the interleaved call *order* across the update does not
//!     (the ctor pre-draws its seven randomised fields, the rest are drawn at
//!     their use sites) — values differ from JS by design.
//!   - **`_steer` lookahead**: upstream re-aims at the furthest walk-visible
//!     waypoint every ~0.1 s (`_fatLos` + `_dryLine` probes). The easy bot
//!     steers straight at the current waypoint; the 1 m waypoint spacing plus
//!     the move-heading slew keeps motion smooth without the probe cost.
//!   - **`_tail` bomb release / `aimPoint`**: the weapon runner consumes the
//!     actor's `aim_dir()` directly; no separate aim point is needed.
//!   - **`chargeRelease` draw timing**: JS `reset()` L290 pre-draws it for
//!     every brain; the Rust bot draws it lazily on first use (only the
//!     charge kinds ever read it), keeping the M1 shooter RNG stream — and
//!     the recorded harness baselines — byte-identical.
//!   - **`bombCd` draw timing**: JS `reset()` L286 pre-draws `3 + rand·4`
//!     for every brain; the Rust bot draws it lazily in the step timer block
//!     (same precedent as `charge_release`). Batch 2 inserts one extra draw
//!     early in every brain's stream, so the recorded bot-run baselines move
//!     with the new behaviour.
//!   - **brolly `G.time`**: the canopy hold / shield windows (brolly.js
//!     L565-576) ride the brain's own `t_time` clock instead of the global
//!     match clock — monotonic per brain, the same deltas.
//!   - **bow bot scratch fields**: JS stores `botWant` / `botJump` /
//!     `botJumped` / `botPaint` on `runner.kit` (bow.js L458-488); the Rust
//!     [`crate::weapon::bow::BowKit`] has no bot slot, so they live on the
//!     brain (same values, same lifetimes — the kit is per-runner either
//!     way).
//!   - **blade `targetBias` teammate memory**: JS reads the live
//!     `e.bot.target` / `e.bot.seeTimer` of every enemy brain
//!     (blade.js L718-725); the sim snapshots them through
//!     [`BotCtx::mate_minds`]. Brains the replicator does not own (remote
//!     actors) report `None` — the bias then only weighs the low-hp term.
//!   - **fight-block gate**: JS L421 runs the fight / retreat block on any
//!     `this.target` (not on visibility); M1 gated it on the target being
//!     visible. Batch 1 widens it to the JS gate so the charge-keep branch
//!     (L506-508) is reachable — the fire gate inside still uses
//!     `enemyVisible` (L479), so the visible-duel behaviour is unchanged.
//!   - **dodge-hop**: the twins probability / cooldown split (JS L519-522,
//!     `0.6` / `0.9 + rand` vs `0.3` / `2 + rand·2.5`) and the CHARGES no-jump
//!     gate (L519) are ported; the retired dualies roll branch (L511-518)
//!     is absent by construction.
//!   - **`_pickPaintGoal` goal-coverage check** (JS `goalCheckT`): the 1 s
//!     "already covered → move on" probe is folded into the goal timer.
//!   - **`_pickPaintGoal` `bot.goal` penalty**: teammate goals come from
//!     [`BotCtx::mate_goals`] (roster-indexed) instead of reading the live
//!     `G.actors[].bot.goal`.
//!   - **`_backOnNav` dry-line/LOS candidate filter**: the easy bot takes
//!     the nearest valid node outright (M1 simplification; the walk-out
//!     behaviour is identical on Tidewater's open plaza).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, ActorInput, Form, MittsReq, SimWorld};
use crate::bot::zones::{WaveState, ZoneInfo, ZonePlan, ZoneRole, lob_pitch};
use crate::match_::find_actor;
use crate::match_::zones::ZoneState;
use crate::nav::{EdgeType, NavGraph};
use crate::paint::PaintGrid;
use crate::special::{KindState, def_by_id, specials};
use crate::sub::{SubKind, SubState, SubSystem, ink_cost};
use crate::tuning::{Difficulty, PlayerTuning, WeaponSet};
use crate::weapon::brolly::CanopyState;
use crate::weapon::mitts::{LeapKind, LeapPred, MittsKit, plan_velocity, predict_leap};
use crate::weapon::{Rng, WeaponKind, WeaponRunner};
use glam::{Vec2, Vec3, vec3};

/// JS `brain._mitts` minus the weave clocks (kits/mitts.js L667): the Mitts
/// leap / wall / travel / escape clocks (Task 47 batch D).
struct MittsCd {
    leap_cd: f32,
    wall_cd: f32,
    travel_cd: f32,
    escape_cd: f32,
}

/// Paint-mode aim pitch for the generic branch (JS `else`, bots.js L557).
const PAINT_PITCH: f32 = -0.42;
/// Paint scan reach for the generic branch (JS `else`, bots.js L544).
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
    /// JS `'boss'` — the Boss Battle brain (`_bossTick`, bots.js L1874;
    /// Task 44 batch C). Only ever live while [`BotCtx::boss`] is `Some`.
    Boss,
}

/// JS `this.bTgt` (bots.js L1888 / L1990 / L2012): the current Boss-Battle
/// shoot target — a hit shape of the boss (`idx` indexes `tuning::Boss::hit`;
/// its world position is read live every frame, like the JS model socket) or
/// a crablet (position frozen at perceive time, like the JS target object).
#[derive(Debug, Clone, Copy)]
pub enum BBotTgt {
    Shape {
        idx: usize,
        rad: f32,
        weak: bool,
        dist: f32,
        los: bool,
    },
    Crab {
        id: u32,
        pos: Vec3,
        dist: f32,
        los: bool,
    },
}

impl BBotTgt {
    /// JS `T.crab` — the crablet flag.
    #[must_use]
    fn is_crab(self) -> bool {
        matches!(self, Self::Crab { .. })
    }

    /// JS `T.dist` / `T.los` (the crab `rad` is the constant `0.5`, L1990).
    #[must_use]
    fn dist(self) -> f32 {
        match self {
            Self::Shape { dist, .. } | Self::Crab { dist, .. } => dist,
        }
    }

    #[must_use]
    fn los(self) -> bool {
        match self {
            Self::Shape { los, .. } | Self::Crab { los, .. } => los,
        }
    }

    #[must_use]
    fn rad(self) -> f32 {
        match self {
            Self::Shape { rad, .. } => rad,
            Self::Crab { .. } => 0.5,
        }
    }
}

/// A teammate brain's target memory, for the Cutlass target bias (JS reads
/// the live `e.bot.target` / `e.bot.seeTimer`, blade.js L718-725; the sim
/// snapshots it through [`BotCtx::mate_minds`]).
#[derive(Debug, Clone, Copy)]
pub struct MateMind {
    /// JS `e.bot.target` — `(team, slot)`.
    pub target: Option<(usize, usize)>,
    /// JS `e.bot.seeTimer`.
    pub see_timer: f32,
}

/// JS `this._zAim` (bots.js L304 / L1199): a needy patch of the zone the
/// paint aim is locked onto (Task 38 batch 3).
#[derive(Debug, Clone, Copy)]
struct ZAim {
    x: f32,
    y: f32,
    z: f32,
    /// JS `z.d`: the live XZ distance, refreshed every frame (L1203).
    d: f32,
}

/// JS `this.zBomb` (bots.js L1256): the lob aim to hold while a Zone Control
/// bomb throw winds up (two-frame release like the plain `_bombAim`).
#[derive(Debug, Clone, Copy)]
struct ZoneBomb {
    yaw: f32,
    pitch: f32,
    /// JS `z.t`: the hold window (1.1 s).
    t: f32,
    release: bool,
}

/// Shared read-only context for one bot decision step. Task 38 batch 1: the
/// Spritzer-only `w` slice became the full weapon table plus the per-id
/// `weaponRange` map (JS `this.a.weapon` / `weaponRange(w)`, bots.js L1619),
/// and the parallel `kinds` / `runners` arrays (JS reads the live
/// `a.weapon.kind` and `a.weaponRunner` state). Batch 2 adds the sub
/// registry (JS `G.subs`, bots.js L980 / the kit `blocked` hooks) and the
/// teammate target snapshot (blade.js L718-725).
pub struct BotCtx<'a> {
    pub actors: &'a [Actor],
    pub nav: &'a NavGraph,
    pub paint: &'a PaintGrid,
    pub world: &'a SimWorld<'a>,
    pub t: &'a PlayerTuning,
    /// `WEAPONS` (config.js L94-264) — the full 12-weapon table.
    pub weapons: &'a WeaponSet,
    /// `weaponRange()` (config.js L307-315) precomputed per weapon id
    /// (`Tuning::weapon_range`).
    pub weapon_range: &'a HashMap<String, f32>,
    /// Per-actor weapon kinds, parallel to `actors` (`Match::kinds`).
    pub kinds: &'a [WeaponKind],
    /// Per-actor weapon runners, parallel to `actors` (`Match::runners`;
    /// JS `a.weaponRunner`).
    pub runners: &'a [WeaponRunner],
    /// Teammate paint goals by roster index (`None` = no goal); mirrors the
    /// JS `m.bot.goal` read in `_pickPaintGoal`.
    pub mate_goals: &'a [Option<usize>],
    /// `G.subs` (bots.js L980, the kit `blocked` hooks L733/L574).
    pub subs: &'a SubSystem,
    /// Teammate target memory by roster index, parallel to `actors`.
    pub mate_minds: &'a [Option<MateMind>],
    /// The shared Zone Control plan — `Some` only in a zones match (JS
    /// `zonePlan()`, bots.js L356; Task 38 batch 3).
    pub zones: Option<&'a crate::bot::zones::ZonePlan>,
    /// The live Zone Control rules state (JS `G.zones`).
    pub zctl: Option<&'a crate::match_::zones::ZoneControl>,
    /// The live HULLBREAKER — `Some` only in a Boss Battle (JS `G.boss`,
    /// bots.js L345; Task 44 batch C). `step` short-circuits into
    /// [`Bot::boss_tick`] whenever this is set.
    pub boss: Option<&'a crate::boss::Boss>,
    /// The live launched canopies — JS `MAIN_KITS.brolly.shields`'
    /// `LAUNCHED` half (bots.js L1276; Task 47 `_shieldFight` /
    /// `_canopyCtl`). Held shields are rebuilt from `runners` + `actors`
    /// (JS `refreshShields` L178-190).
    pub launched: &'a [crate::weapon::brolly::LaunchedCanopy],
    /// JS `G.time` (blade.js L611 / L642): the accumulated match clock — the
    /// sim maps it to `projectiles.time` (the blade kit's `ps_time` precedent,
    /// [`crate::weapon::blade::update`]). Task 47 batch D: the Cutlass
    /// charge-hold stamp (`brain._blHold`) rides it, so it must be the
    /// accumulating clock, never `Match::time` (the remaining countdown).
    pub time: f32,
}

impl<'a> BotCtx<'a> {
    /// JS `weaponRange(w)` (config.js L307-315) — the precomputed per-id
    /// effective reach; the `|| 12` default of the JS `default:` arm covers
    /// a missing id.
    #[must_use]
    pub fn weapon_reach(&self, kind: WeaponKind) -> f32 {
        self.weapon_range
            .get(kind.id_str())
            .copied()
            .unwrap_or(12.0)
    }

    /// JS `_range()` (bots.js L1617-1620): `weaponRange(w) * (CHARGES ? 0.9 : 1)`.
    #[must_use]
    pub fn bot_range(&self, kind: WeaponKind) -> f32 {
        self.weapon_reach(kind) * if kind.is_charge() { 0.9 } else { 1.0 }
    }

    /// JS L428 lead denominator `w.projSpeed || w.speedMax || w.throwSpeed || 30`
    /// (the retired slosher windup arm is absent by construction; the kit
    /// kinds' own `MAIN_KITS[k].bot` triggers land in batch 2).
    #[must_use]
    pub fn shot_speed(&self, kind: WeaponKind) -> f32 {
        let w = self.weapons;
        match kind {
            WeaponKind::Shooter => w.shooter.proj_speed,
            WeaponKind::Twins => w.twins.proj_speed,
            WeaponKind::Blaster => w.blaster.proj_speed,
            WeaponKind::Brolly => w.brolly.proj_speed,
            WeaponKind::Blade => w.blade.proj_speed,
            WeaponKind::Spinner => w.spinner.speed_max,
            WeaponKind::Bucket => w.bucket.throw_speed,
            // charger / roller / brush / bow / mitts carry none of the three
            // JS fields → the `|| 30` fallback.
            WeaponKind::Charger
            | WeaponKind::Roller
            | WeaponKind::Brush
            | WeaponKind::Bow
            | WeaponKind::Mitts => 30.0,
        }
    }

    /// This bot's own kind / runner (JS `this.a.weapon` / `this.a.weaponRunner`).
    fn own(&self, team: usize, slot: usize) -> (WeaponKind, &WeaponRunner) {
        static IDLE: std::sync::OnceLock<WeaponRunner> = std::sync::OnceLock::new();
        let idx = find_actor(self.actors, team, slot);
        let kind = idx
            .and_then(|i| self.kinds.get(i))
            .copied()
            .unwrap_or(WeaponKind::Shooter);
        let runner = idx
            .and_then(|i| self.runners.get(i))
            .unwrap_or_else(|| IDLE.get_or_init(WeaponRunner::default));
        (kind, runner)
    }
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
    paint_pause: f32,
    // ---- per-kind fight state (Task 38 batch 1)
    /// JS `this.chargeRelease` (bots.js L290) — drawn lazily on first read
    /// (module-docs deviation).
    charge_release: Option<f32>,
    /// JS `this.plantT` / `this._wasPlanted` (bots.js L455-457, twins).
    plant_t: f32,
    was_planted: bool,
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
    // ---- batch 2: sub / special / kit scratch (Task 38 batch 2)
    /// JS `this.bombCd` (bots.js L286/L343) — drawn lazily on the first frame
    /// (module-docs deviation).
    bomb_cd: Option<f32>,
    /// JS `this._bombAim` (bots.js L505/L600/L638) — press-frame stamp.
    bomb_aim: bool,
    /// JS `this._releaseBomb` (bots.js L639) — release-frame stamp.
    release_bomb: bool,
    /// JS `this.spT` (bots.js L887).
    sp_t: f32,
    /// JS `this.barrageCd` (bots.js L890).
    barrage_cd: f32,
    /// JS `this.stampFlip` (bots.js L906).
    stamp_flip: f32,
    /// JS `this.blowT` (bots.js L932).
    blow_t: f32,
    /// JS `brain._brolly.holdUntil` (kits/brolly.js L563).
    bb_hold_until: f32,
    /// JS `brain._brolly.nextShield` (kits/brolly.js L563).
    bb_next_shield: f32,
    /// JS `brain._brolly.launch` (kits/brolly.js L563).
    bb_launch: bool,
    /// JS `runner.kit.botWant` (kits/bow.js L458) — brain-side scratch
    /// (module-docs deviation).
    bow_want: f32,
    /// JS `runner.kit.botJump` (kits/bow.js L460).
    bow_jump: bool,
    /// JS `runner.kit.botJumped` (kits/bow.js L461).
    bow_jumped: bool,
    /// JS `runner.kit.botPaint` (kits/bow.js L488).
    bow_paint: f32,
    /// JS `brain._bl.waveT` (kits/blade.js L408, seeded to 3).
    bl_wave_t: f32,
    /// JS `brain._bl.press` (kits/blade.js L408).
    bl_press: bool,
    /// JS `brain._mw.weaveT` (kits/mitts.js L745).
    mw_weave_t: f32,
    /// JS `brain._mw.weaveP` (kits/mitts.js L746) — drawn once, lazily.
    mw_weave_p: Option<f32>,
    /// JS `brain._bl` minus `waveT` / `press` (the flat fields above;
    /// kits/blade.js L407-411 `botState`) — the Cutlass deep-tactics state,
    /// created lazily on the first tactics frame (one `side` draw, Task 47
    /// batch D).
    bl_state: Option<crate::bot::blade_tactics::BladeState>,
    /// JS `brain._mw` minus `weaveT` / `weaveP` (kits/mitts.js L665-667
    /// `botState`) — the Mitts leap clocks, created lazily on the first
    /// fight / paint hook frame (three draws, Task 47 batch D).
    mitts_cd: Option<MittsCd>,
    // ---- Zone Control scratch (Task 38 batch 3; JS L304)
    /// JS `this.zRole`.
    z_role: Option<crate::bot::zones::ZoneRole>,
    /// JS `this.zZone` (`-1` = none).
    z_zone: isize,
    /// JS `this.zHoldUntil`.
    z_hold_until: f32,
    /// JS `this.zHoldDur`.
    z_hold_dur: f32,
    /// JS `this._zAct` (the active objective at the last sync; `None` = JS
    /// `null`).
    z_act: Option<usize>,
    /// JS `this.zAimT`.
    z_aim_t: f32,
    /// JS `this._zAim`.
    z_aim: Option<ZAim>,
    /// JS `this.zFail`.
    z_fail: u8,
    /// JS `this.zFace`.
    z_face: f32,
    /// JS `this.zBomb`.
    z_bomb: Option<ZoneBomb>,
    /// JS `this.zBombScan`.
    z_bomb_scan: f32,
    // ---- Boss Battle scratch (Task 44 batch C; JS `_bossTick` family)
    /// JS `this.bTgt` (bots.js L1888).
    b_tgt: Option<BBotTgt>,
    /// JS `this._th` (L1880) — this frame's threat read, post reaction gate.
    th: crate::boss::Threat,
    /// JS `this.thSeen` (L1883; `None` ≡ JS `undefined`).
    th_seen: Option<f32>,
    /// JS `this.hopMiss` (L1883).
    hop_miss: bool,
    /// JS `this.evadeT` (L1879 / L1893; `(evadeT || 0) - dt` → init `0`).
    evade_t: f32,
    /// JS `this._wasEvading` (L1894-1895).
    was_evading: bool,
    /// JS `this._rushing` (L1898 / L2030).
    rushing: bool,
    /// JS `this._pump` (L1945, the brolly trigger pump).
    pump: bool,
    /// JS `this.shapeT` (L1995; `(shapeT || 0) - 0.2` → init `0`).
    shape_t: f32,
    // ---- threat AI scratch (Task 47 batch B; JS `_threatScan` family L1272-1615)
    /// JS `this.thr` (bots.js L307): the device we act on, by cross-frame
    /// identity (the JS descriptor becomes [`threat::ThreatRef`], re-resolved
    /// against the live sub registry every frame).
    thr: Option<crate::sub::ThreatRef>,
    /// JS `this.thrCand` (L307): the best candidate from the last scan.
    thr_cand: Option<crate::sub::ThreatRef>,
    /// JS `this.thrScanT` (L307; ctor `Math.random() * 0.25`).
    thr_scan_t: f32,
    /// JS `this._thrMem` (L307): when each noticed device becomes actionable
    /// (reaction gate, on the brain's own `t_time` clock — the brolly
    /// `G.time` deviation applies here too).
    thr_mem: HashMap<crate::sub::ThreatRef, f32>,
    /// JS `this._shl` (L307): the per-frame enemy-canopy views
    /// (rebuild deviation: JS refreshes only at scan time, the sim rebuilds
    /// every frame — see `threat.rs` module docs).
    shl: Vec<crate::bot::threat::ShieldView>,
    /// JS `this.thrLos` / `this.thrLosT` (L309): the 0.1 s line-of-sight sample.
    thr_los: bool,
    thr_los_t: f32,
    /// JS `this.thrAct` (L309): written, never read by the JS brain.
    #[allow(dead_code)]
    thr_act: Option<&'static str>,
    /// JS `this.thrActs` (L309): per-device THREAT_STATS bits (1 shoot,
    /// 2 evade, 4 swim, 8 sidestep).
    thr_acts: u32,
    /// JS `this.thrFiring` (L309): widens the aim tolerance mid-burst.
    thr_firing: bool,
    /// JS `this.thrPulse` (L309): the melee press/release latch at a device.
    thr_pulse: bool,
    /// JS `this.thrEvT` / `this.thrEvYaw` (L309): the evade-heading timer.
    thr_ev_t: f32,
    thr_ev_yaw: f32,
    /// JS `this.thrSide` (L309): the sticky sidestep side (`0` = unpicked).
    thr_side: i32,
    /// JS `this.thrAcqT` (L309; ctor `9`): the aim-acquisition clock.
    thr_acq_t: f32,
    /// JS `this.thrSignY` / `this.thrSignP` (L309): the acquisition overshoot signs.
    thr_sign_y: f32,
    thr_sign_p: f32,
    // ---- canopy / shield scratch (Task 47 batch C; JS L310 / L1507-1615)
    /// JS `this.canRef` (L310): the launched canopy we steer round / break.
    can_ref: Option<crate::bot::threat::ShieldRef>,
    /// JS `this.canSide` (L310): the edge to round (`2` = break it instead).
    can_side: i32,
    /// JS `this.canT` (L310): seconds steering this canopy.
    can_t: f32,
    /// JS `this._canBroke` (L1580): the `breakWall` stat latch.
    can_broke: bool,
    /// JS `this.flankSide` (L310): the sticky flank side behind a shield.
    flank_side: i32,
    /// JS `this.blockT` (L310): seconds a shield has blocked our duel.
    block_t: f32,
    /// JS `this.flankT` (L310): the flank-probe re-pick timer.
    flank_t: f32,
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
        // Task 47 batch B: JS `reset()` L307 draws `thrScanT = Math.random() *
        // 0.25`; appended as the 7th draw of this brain's stream (the sim's
        // ctor pre-draw order is its own — see the module-docs RNG deviation).
        let r_thr_scan = rng.next_f32();
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
            paint_pause: 0.0,
            charge_release: None,
            plant_t: 0.0,
            was_planted: false,
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
            bomb_cd: None,
            bomb_aim: false,
            release_bomb: false,
            sp_t: 0.0,
            barrage_cd: 0.0,
            stamp_flip: 0.0,
            blow_t: 0.0,
            bb_hold_until: 0.0,
            bb_next_shield: 0.0,
            bb_launch: false,
            bow_want: 0.0,
            bow_jump: false,
            bow_jumped: false,
            bow_paint: 0.0,
            bl_wave_t: 3.0,
            bl_press: false,
            mw_weave_t: 0.0,
            mw_weave_p: None,
            bl_state: None,
            mitts_cd: None,
            z_role: None,
            z_zone: -1,
            z_hold_until: 0.0,
            z_hold_dur: 0.0,
            z_act: None,
            z_aim_t: 0.0,
            z_aim: None,
            z_fail: 0,
            z_face: 0.0,
            z_bomb: None,
            z_bomb_scan: 0.0,
            b_tgt: None,
            th: crate::boss::Threat::default(),
            th_seen: None,
            hop_miss: false,
            evade_t: 0.0,
            was_evading: false,
            rushing: false,
            pump: false,
            shape_t: 0.0,
            thr: None,
            thr_cand: None,
            thr_scan_t: r_thr_scan * 0.25,
            thr_mem: HashMap::new(),
            shl: Vec::new(),
            thr_los: false,
            thr_los_t: 0.0,
            thr_act: None,
            thr_acts: 0,
            thr_firing: false,
            thr_pulse: false,
            thr_ev_t: 0.0,
            thr_ev_yaw: 0.0,
            thr_side: 0,
            thr_acq_t: 9.0,
            thr_sign_y: 0.0,
            thr_sign_p: 0.0,
            can_ref: None,
            can_side: 0,
            can_t: 0.0,
            can_broke: false,
            flank_side: 0,
            block_t: 0.0,
            flank_t: 0.0,
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

    /// JS `this.chargeRelease` (bots.js L290: `0.95 + Math.random() * 0.05`).
    /// Drawn lazily (module-docs deviation): only the charge kinds ever read
    /// it, so the M1 shooter RNG stream and the recorded baselines are
    /// unchanged.
    fn charge_release(&mut self) -> f32 {
        *self
            .charge_release
            .get_or_insert_with(|| 0.95 + self.rng.next_f32() * 0.05)
    }

    /// JS `BotBrain.update`: returns this frame's
    /// intent and writes the aim angles onto `a` (JS `a.aimYaw = ...`).
    pub fn step(&mut self, dt: f32, a: &mut Actor, ctx: &BotCtx) -> ActorInput {
        // JS `a.isBot` / `a.bot` (bots.js): every brain-driven actor is a bot
        // — the kit-side bot branches (Mitts leap / cling, blade `lungeSafe`
        // ledge guard) read it (Task 47 batch D).
        a.is_bot = true;
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
        // JS L286 pre-draws `bombCd = 3 + rand·4` at reset; the Rust bot
        // draws it lazily here (module-docs deviation).
        let bomb_cd = self
            .bomb_cd
            .get_or_insert_with(|| 3.0 + self.rng.next_f32() * 4.0);
        *bomb_cd -= dt;
        self.strafe_t -= dt;
        self.paint_pause -= dt;
        self.dodge_cd -= dt;
        self.acq_t += dt;
        self.t_time += dt;

        // ---- Boss Battle (JS L345): a different job — the whole normal
        // pipeline below (perception / mode / goal / fight / paint) is
        // skipped; `boss_tick` owns the frame (Task 44 batch C).
        if let Some(boss) = ctx.boss {
            return self.boss_tick(dt, a, ctx, boss);
        }

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

        // ---- Zone Control: the team plan (JS L355-357); a rotation or a
        // changed role / zone re-targets straight away.
        if ctx.zones.is_some() {
            self.zone_sync(ctx, a);
        }

        // ---- mode selection (JS L359-381)
        let ink_frac = a.ink / ctx.t.ink_max;
        let hp_frac = a.hp / ctx.t.hp;
        let last_damage = a.last_damage();
        let (kind, wr) = ctx.own(self.team, self.slot);
        self.select_mode(dt, ink_frac, hp_frac, last_damage, kind, a, ctx, wr);

        // ---- navigation goal (JS L383-406)
        self.goal_timer -= dt;
        self.repath -= dt;
        match self.mode {
            BotMode::Fight => {
                if let Some(tid) = self.target
                    && let Some(idx) = find_actor(ctx.actors, tid.0, tid.1)
                {
                    let p = ctx.actors[idx].pos;
                    if ctx.zones.is_some() && self.zone_hold_ground(ctx, a, p, kind) {
                        // watchers / guards: don't chase off the zone (JS L386)
                        self.path = None;
                        self.repath = 0.3;
                    } else if self.repath <= 0.0 {
                        self.path_to(ctx, a.pos, a.grounded, p, 0.6);
                    }
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
                if let Some(zp) = ctx.zones {
                    // Zone Control: arrive → hold a moment (guards /
                    // watchers) → next spot on the zone (JS L400-404).
                    let arrived =
                        self.path.is_none() || self.pi >= self.path.as_ref().map_or(0, |p| p.len());
                    if arrived && self.z_hold_dur > 0.0 {
                        self.z_hold_until = self.t_time + self.z_hold_dur;
                        self.z_hold_dur = 0.0;
                    }
                    if self.wiggle_t <= 0.0
                        && (self.goal_timer <= 0.0 || (arrived && self.t_time >= self.z_hold_until))
                    {
                        self.pick_zone_goal(a, ctx, zp, kind);
                    }
                } else if self.goal_timer <= 0.0
                    || self.path.is_none()
                    || self.pi >= self.path.as_ref().map_or(0, |p| p.len())
                {
                    self.pick_paint_goal(a, ctx);
                }
            }
            // The boss short-circuit (JS L345) returns before mode selection,
            // so `Boss` never reaches this match — JS's normal update has no
            // 'boss' arm either.
            BotMode::Boss => {}
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

        // JS L419: the fight distance feeds `_specialCtl` (L620).
        let mut fight_dist = 0.0f32;
        // JS L421: the block runs on any target — visibility is only the fire
        // gate's concern (L479).
        let tgt_idx = self.target.and_then(|(t, s)| find_actor(ctx.actors, t, s));
        if (self.mode == BotMode::Fight || self.mode == BotMode::Retreat)
            && let Some(ti) = tgt_idx
        {
            let t = &ctx.actors[ti];
            let dx = t.pos.x - a.pos.x;
            let dz = t.pos.z - a.pos.z;
            let dist = dx.hypot(dz);
            fight_dist = dist;
            let range = ctx.bot_range(kind);
            // lead the target by the projectile flight time (JS L428; the
            // charger's beam needs no lead, the retired slosher windup arm
            // is absent by construction)
            let lead = if kind == WeaponKind::Charger {
                0.0
            } else {
                dist / ctx.shot_speed(kind)
            };
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
                    kind,
                    wr,
                    enemy_visible,
                );
            } else {
                // retreat: swim away through own ink, eyes on the threat
                it.squid = true;
                // JS L534-536: kits with their own tactics run the retreat
                // too (`ctx.retreat`) — the Cutlass runs where there's no ink
                // (Task 47 batch D).
                if kind == WeaponKind::Blade {
                    self.blade_tactics(
                        a,
                        ctx,
                        dt,
                        dist,
                        &mut move_v,
                        &mut it,
                        ti,
                        wr,
                        enemy_visible,
                        false,
                        true,
                    );
                }
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
                kind,
                wr,
            );
        } else if self.mode == BotMode::Refill {
            self.refill(a, ctx, &mut it, &mut want_pitch);
        }

        // JS L620: driving a running special + cheering a teammate's Cheer
        // Orb (the climb aim overrides are cut — module docs).
        self.special_ctl(dt, a, ctx, &mut move_v, &mut it, fight_dist, enemy_visible);

        // JS L623-630 (Task 47): deal with a noticed device, then an enemy
        // canopy across our way — the device wins the frame's aim.
        let thr_aim = self.threat_ctl(dt, a, ctx, &mut it, &mut move_v);
        let can_aim = self.canopy_ctl(dt, a, ctx, &mut it, &mut move_v, thr_aim.is_none());
        let thr_aim = thr_aim.or(can_aim);
        if let Some(aim) = &thr_aim {
            want_yaw = aim.yaw;
            want_pitch = aim.pitch;
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
            kind,
            wr,
            thr_aim,
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

    /// JS `update` L359-381 (turf-war branch). Task 38 batch 1 adds the
    /// MELEE gates on the retreat roll (L366) and the refill guard (L370);
    /// batch 2 adds the kit `stayIn` veto (L367 — only the Cutlass defines
    /// one).
    #[allow(clippy::too_many_arguments)]
    fn select_mode(
        &mut self,
        dt: f32,
        ink_frac: f32,
        hp_frac: f32,
        last_damage: f32,
        kind: WeaponKind,
        a: &Actor,
        ctx: &BotCtx,
        wr: &WeaponRunner,
    ) {
        if self.mode == BotMode::Retreat {
            self.retreat_t -= dt;
            if hp_frac > 0.85 || self.retreat_t <= 0.0 || (self.target.is_none() && hp_frac > 0.6) {
                self.mode = BotMode::Paint;
                self.path = None;
                self.goal_timer = 0.0;
            }
        } else if self.target.is_some()
            && self.see_timer > 0.0
            // JS L366: pushers don't break off a scratch at a third of hp
            && ((hp_frac < 0.34 && !kind.is_melee() && last_damage < 0.8) || hp_frac < 0.2)
            && self.rng.next_f32() < 0.6 * dt * 60.0 * self.diff.fire_discipline
            // JS L367: a kit may veto the retreat — the Cutlass finishes a
            // fight it can win at blade range instead of turning
            && !(kind == WeaponKind::Blade && self.blade_stay_in(a, ctx, wr))
        {
            // losing the duel: break line of sight and heal in own ink
            self.mode = BotMode::Retreat;
            self.retreat_t = 2.2 + self.rng.next_f32() * 1.4;
            self.repath = 0.0;
        }
        if self.mode != BotMode::Refill
            && self.mode != BotMode::Retreat
            && ink_frac < 0.12
            // JS L370: a melee weapon with a visible target finishes the duel
            // on a trickle of ink instead of refilling
            && !(self.target.is_some()
                && self.see_timer > 0.0
                && !kind.is_melee()
                && ink_frac > 0.05)
        {
            self.mode = BotMode::Refill;
            self.refill_until = 0.85 + self.rng.next_f32() * 0.1;
        }
        if self.mode == BotMode::Refill && ink_frac >= self.refill_until {
            self.mode = BotMode::Paint;
        }
        if ctx.zones.is_some() {
            // JS L374-380: the objective first — only take fights that are
            // in range or on / by the zone (pushers fight like Turf War).
            if self.mode != BotMode::Refill && self.mode != BotMode::Retreat {
                let m = if self.target.is_some() && self.zone_engage(ctx, a, kind) {
                    BotMode::Fight
                } else {
                    BotMode::Paint
                };
                if m == BotMode::Paint && self.mode == BotMode::Fight {
                    // back to the zone
                    self.goal_timer = 0.0;
                    self.z_hold_until = 0.0;
                }
                self.mode = m;
            }
        } else if self.mode != BotMode::Refill && self.mode != BotMode::Retreat {
            self.mode = if self.target.is_some() {
                BotMode::Fight
            } else {
                BotMode::Paint
            };
        }
    }

    // ---------------------------------------------------------------- perceive

    /// JS `_perceive` (L1622-1656). Batch 2 adds the kit target bias
    /// (L1627 / L1637 — only the Cutlass defines one).
    fn perceive(&mut self, a: &Actor, ctx: &BotCtx) {
        let eye = Vec3::new(a.pos.x, a.pos.y + EYE_Y, a.pos.z);
        let aw = self.diff.awareness;
        let own_kind = ctx.own(self.team, self.slot).0;
        let mut best: Option<(usize, usize)> = None;
        let mut bd = f32::INFINITY;
        for (ei, e) in ctx.actors.iter().enumerate() {
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
            if own_kind == WeaponKind::Blade {
                score += self.blade_target_bias(a, ctx, ei, d);
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

    /// JS `update` fight block (L441-523): per-kind preferred distance,
    /// eased strafing, fire when the *actual* aim is on the body, the per-kind
    /// trigger dispatch, swim-in over own ink, dodge-hop after a hit.
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
        kind: WeaponKind,
        wr: &WeaponRunner,
        enemy_visible: bool,
    ) {
        let t = &ctx.actors[ti];
        let dx = t.pos.x - a.pos.x;
        let dz = t.pos.z - a.pos.z;
        // preferred duel distance (JS L443): chargers hold back, pushers
        // close to arm's length
        let pref = if kind == WeaponKind::Charger {
            range * 0.8
        } else if kind.is_melee() {
            0.5
        } else {
            range * FIGHT_PREF_F
        };
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
        } else if dist < pref - 1.5 && !kind.is_melee() {
            mvx = -nx;
            mvz = -nz;
        }
        // a charging long weapon plants instead of strafing (JS L450)
        if !(kind.is_charge() && wr.charging_flag()) {
            mvx += -nz * self.strafe_s * 0.9;
            mvz += nx * self.strafe_s * 0.9;
        }
        // pushers always close the distance (JS L451)
        if kind.is_melee() && dist < 7.0 {
            mvx = nx;
            mvz = nz;
        }
        // twin pistols: stand planted for a moment after the turret re-mounts
        // — moving would drop it back to dual mode (JS L453-459)
        if kind == WeaponKind::Twins {
            let planted = wr.turret();
            if planted && !self.was_planted {
                self.plant_t = 0.7 + self.rng.next_f32() * 0.7;
            }
            self.was_planted = planted;
            self.plant_t -= dt;
            if planted && self.plant_t > 0.0 {
                mvx = 0.0;
                mvz = 0.0;
            }
        }
        // JS L460-470 (Task 47): a stomping Stamp — steer off its line.
        self.stamp_steer(a, t, dist, kind, &mut mvx, &mut mvz);
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
        // per-kind trigger dispatch (JS L479-508; batch 2 adds the kit
        // `BK.fight` hooks, the retired splatling / slosher / dualies arms
        // are absent by construction)
        if enemy_visible && self.react <= 0.0 && aimed && ink_frac > 0.02 {
            it.fire = match kind {
                // kit main weapons decide their own trigger and may steer
                // (JS L480-482; the Cutlass has no `fight` hook and falls
                // through to the generic reach test)
                WeaponKind::Brolly => self.brolly_fight(a, ctx, dt, dist, range, move_v, wr),
                WeaponKind::Bow => self.bow_fight(a, ctx, dist, range, move_v, it, ti, wr),
                WeaponKind::Mitts => self.mitts_fight(a, ctx, dt, dist, range, it, ti, wr),
                // spin up to the release point, let go, run the stream
                // (JS L483-486)
                WeaponKind::Spinner => {
                    wr.burst_t() <= 0.0
                        && !(wr.charging_flag() && wr.charge() >= self.charge_release())
                }
                WeaponKind::Brush => dist < 4.5,
                WeaponKind::Charger => {
                    !(wr.charging_flag() && wr.charge() >= self.charge_release())
                }
                WeaponKind::Roller => dist < 5.5 || (wr.rolling() && dist < 8.0),
                _ => dist < range * 1.08,
            };
            // a charging charger creases its stride (JS L498)
            if kind == WeaponKind::Charger && wr.charging_flag() {
                *move_v *= 0.3;
            }
            self.firing = it.fire;
            // JS L505: a sub throw can ride the same fire frame
            if self.bomb_cd.unwrap_or(0.0) <= 0.0 && self.fight_sub(a, ctx, dist) {
                it.sub = true;
                self.bomb_aim = true;
            }
        } else if kind.is_charge() && wr.charging_flag() && !enemy_visible {
            // keep the charge while the target is briefly hidden (JS L506-508)
            it.fire = true;
        }
        // out of range with own ink underfoot: swim in (JS L509-510; a
        // charging weapon holds its ground)
        if !it.fire && !wr.charging_flag() && dist > range * 1.15 && a.ground_team == 1 {
            it.squid = true;
        }
        // dodge-hop right after taking a hit (JS L519-522; twins roll with a
        // wider water probe, chargers never jump)
        if a.last_damage() < 0.25
            && self.dodge_cd <= 0.0
            && a.grounded
            && !kind.is_charge()
            && self.rng.next_f32() < if kind == WeaponKind::Twins { 0.6 } else { 0.3 }
            && !self.near_water(
                a,
                ctx,
                if kind == WeaponKind::Twins {
                    ctx.weapons.twins.roll_dist + 0.4
                } else {
                    1.6
                },
            )
        {
            it.jump = true;
            self.dodge_cd = if kind == WeaponKind::Twins {
                0.9 + self.rng.next_f32()
            } else {
                2.0 + self.rng.next_f32() * 2.5
            };
        }
        // JS L525 (Task 47): an enemy canopy between us and the target —
        // flank round it, hold the shots that would only ink it.
        if enemy_visible {
            self.shield_fight(a, ctx, t, dist, move_v, it, dt, kind, wr);
        } else {
            self.block_t = 0.0;
        }
        // kit fight tactics that run every frame after the defaults (JS L528;
        // the Cutlass deep tactics are restored by Task 47 batch D)
        if kind == WeaponKind::Mitts {
            self.mitts_tactics(a, ctx, dt, dist, move_v, it, ti, enemy_visible, wr);
        } else if kind == WeaponKind::Blade {
            self.blade_tactics(
                a,
                ctx,
                dt,
                dist,
                move_v,
                it,
                ti,
                wr,
                enemy_visible,
                enemy_visible && self.react <= 0.0 && aimed && ink_frac > 0.02,
                false,
            );
        }
        // JS L530: the special gate (batch 3 adds the Zone Control arm)
        if a.special_ready()
            && (self.want_special(a, ctx, true, dist, enemy_visible)
                || (ctx.zones.is_some() && self.zone_special(ctx, a, true)))
        {
            it.special = true;
        }
    }

    // ---------------------------------------------------------------- paint

    /// JS `update` paint block (L538-599): the per-kind sweep-rate / scan-
    /// reach / aim-pitch ladders and the per-kind trigger dispatch.
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
        kind: WeaponKind,
        wr: &WeaponRunner,
    ) {
        // chargers sweep slowly — every degree of the arc is a beam line
        // (JS L540)
        self.sweep += dt
            * if kind == WeaponKind::Charger {
                0.8
            } else {
                2.1
            };
        self.paint_scan_t -= dt;
        // pushers paint straight ahead — no yaw scan (JS L542)
        if self.paint_scan_t <= 0.0 && !kind.is_melee() {
            self.paint_scan_t = 0.35 + self.rng.next_f32() * 0.15;
            // scan reach ladder (JS L544; splatling / slosher retired)
            let reach = if kind == WeaponKind::Charger || kind == WeaponKind::Spinner {
                8.5
            } else if kind == WeaponKind::Bucket {
                7.0
            } else if kind == WeaponKind::Blaster {
                5.0
            } else {
                PAINT_REACH
            };
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
                    a.pos.x + yw.sin() * reach,
                    a.pos.y,
                    a.pos.z + yw.cos() * reach,
                    2.2,
                    a.team,
                );
                let mut v = if st.n != 0 {
                    st.empty + st.enemy * 1.4 - off.abs() * 0.12
                } else {
                    -1.0
                };
                // JS L550: the zone's ink first
                if let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl)
                    && st.n != 0
                    && zp.on_active(
                        a.pos.x + yw.sin() * reach,
                        a.pos.y,
                        a.pos.z + yw.cos() * reach,
                        zctl,
                    )
                {
                    v += 0.8;
                }
                if v > best_v {
                    best_v = v;
                    best_off = off;
                }
            }
            self.paint_yaw_off = best_off;
        }
        // pushers hold their heading: no yaw offset, no sweep wobble (JS L555)
        let sweep_amt = if kind.is_melee() { 0.0 } else { 0.35 };
        *want_yaw += (if kind.is_melee() {
            0.0
        } else {
            self.paint_yaw_off
        }) + self.sweep.sin() * sweep_amt;
        *want_pitch = paint_pitch(kind);
        // Zone Control: a needy patch of the zone in reach → ink that (long
        // range: from the perch, straight at it) (JS L560-569)
        let mut z_aim = if ctx.zones.is_some() && !kind.is_melee() {
            self.zone_aim(ctx, a, dt, want_move, *move_v, kind)
        } else {
            None
        };
        if ctx.zones.is_some() && self.z_face > 0.0 {
            // about to throw a special onto the zone: turn to it first
            self.z_face -= dt;
            if let Some(zctl) = ctx.zctl
                && self.z_zone >= 0
            {
                let c = zctl.zones[self.z_zone as usize].center;
                z_aim = Some(ZAim {
                    x: c[0],
                    y: c[1],
                    z: c[2],
                    d: (c[0] - a.pos.x).hypot(c[2] - a.pos.z),
                });
            }
        }
        if let Some(za) = z_aim {
            *want_yaw = (za.x - a.pos.x).atan2(za.z - a.pos.z) + self.sweep.sin() * 0.22;
            if kind.is_charge() || za.d < 3.2 {
                *want_pitch = (za.y + 0.05 - (a.pos.y + 1.1))
                    .atan2(za.d.max(0.5))
                    .clamp(-0.95, 0.15);
            }
        }
        let ahead = ctx.paint.region_stats(
            ctx.world.collision,
            a.pos.x + want_yaw.sin() * 4.0,
            a.pos.y,
            a.pos.z + want_yaw.cos() * 4.0,
            3.0,
            a.team,
        );
        let need_paint = ahead.n == 0 || ahead.own < 0.75 || z_aim.is_some();
        // per-kind trigger dispatch (JS L573-597; batch 2 adds the kit
        // `PK.paint` hooks first, splatling retired)
        it.fire = match kind {
            // kit main weapons decide the paint trigger first (JS L573-574)
            WeaponKind::Brolly => need_paint && ink_frac > 0.2,
            WeaponKind::Blade => {
                self.blade_paint(a, ctx, dt, it, need_paint, ink_frac, want_move, wr)
            }
            WeaponKind::Bow => self.bow_paint(a, ctx, need_paint, ink_frac, wr),
            WeaponKind::Mitts => {
                self.mitts_paint(a, ctx, dt, move_v, it, need_paint, ink_frac, want_move, wr)
            }
            _ => {
                if kind.is_melee() {
                    ink_frac > 0.08 && (need_paint || self.rng.next_f32() < 0.02) && want_move
                } else if kind == WeaponKind::Spinner {
                    // spin to ~60 %, release, let the stream paint, breather
                    if wr.burst_t() > 0.0 {
                        false
                    } else if wr.charging_flag() {
                        let f = wr.charge() < 0.6;
                        if !f {
                            self.paint_pause = 0.25 + self.rng.next_f32() * 0.3;
                        }
                        f
                    } else {
                        need_paint && ink_frac > 0.3 && self.paint_pause <= 0.0
                    }
                } else if kind == WeaponKind::Charger {
                    // charge to ~70 % and release a paint line, then a short
                    // breather before the next one
                    if wr.charging_flag() {
                        let f = wr.charge() < 0.7;
                        if !f {
                            self.paint_pause = 0.3 + self.rng.next_f32() * 0.35;
                        }
                        f
                    } else {
                        need_paint && ink_frac > 0.3 && self.paint_pause <= 0.0
                    }
                } else {
                    need_paint && ink_frac > 0.18
                }
            }
        };
        // travel as a squid through own ink when not painting (JS L599)
        if !it.fire && self.path_remaining(a, ctx) > 5.0 && a.ground_team == 1 {
            it.squid = true;
        }
        // JS L600: a sub throw while painting
        if self.bomb_cd.unwrap_or(0.0) <= 0.0 && self.paint_sub(a, ctx) {
            it.sub = true;
            self.bomb_aim = true;
        } else if ctx.zones.is_some() {
            // JS L601-608: Zone Control — lob a bomb onto a patch of their
            // ink on the zone (turn to it, then throw)
            if let Some(zb) = self.zone_bomb_aim(ctx, a, dt) {
                *want_yaw = zb.yaw;
                *want_pitch = zb.pitch;
                if zb.release {
                    self.z_bomb = None;
                } else if angle_diff(self.aim_yaw, zb.yaw).abs() < 0.09
                    && (self.aim_pitch - zb.pitch).abs() < 0.09
                {
                    self.bomb_aim = true;
                    self.bomb_cd = Some(4.0 + self.rng.next_f32() * 3.0);
                    if let Some(cur) = self.z_bomb.as_mut() {
                        cur.release = true;
                    }
                }
            }
        }
        // JS L610: the special gate (batch 3: the Zone Control arm replaces
        // the plain paint roll)
        if a.special_ready()
            && if ctx.zones.is_some() {
                self.zone_special(ctx, a, false)
            } else {
                self.want_special(a, ctx, false, 0.0, false)
            }
        {
            it.special = true;
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
        kind: WeaponKind,
        wr: &WeaponRunner,
        // JS `_tail`'s `thrAim` (L643): the threat / canopy aim this frame
        // (Task 47) — makes the aim spring snappy like a duel does.
        thr_aim: Option<crate::bot::threat::AimHold>,
    ) {
        // ---- two-frame sub release (JS L638-639): the press frame stamps
        // `_bombAim`, the next frame drops `sub` and the squid form so the
        // throw actually goes off.
        if self.bomb_aim {
            it.sub = true;
            it.squid = false;
            self.bomb_aim = false;
            self.release_bomb = true;
        } else if self.release_bomb {
            it.sub = false;
            it.squid = false;
            self.release_bomb = false;
        }

        // ---- aim spring (JS L641-654)
        let fighting = self.mode == BotMode::Fight;
        // JS L643-646: holding aim on a device / canopy is snappy too
        let snappy = fighting || thr_aim.is_some();
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
        // a charging / bursting long weapon is not "trying" to walk (JS L683)
        let trying = self.path.is_some()
            && want_move
            && !(kind.is_charge() && (wr.charging_flag() || wr.burst_t() > 0.0));
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

    // ---------------------------------------------------------------- zone control

    /// JS `_zoneSync` (bots.js L1019-1033): a rotation or a changed role /
    /// zone re-targets straight away. The `zJumpAt` super-jump arm (L1026 /
    /// L1029-1032 → `_zoneJump` L986-1016) is cut — the sim has no super
    /// jumps (module docs).
    fn zone_sync(&mut self, ctx: &BotCtx, a: &Actor) {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return;
        };
        let Some(idx) = find_actor(ctx.actors, a.team, a.slot) else {
            return;
        };
        let r = zp.role_of(idx, a.team, zctl);
        if Some(zctl.active) != self.z_act
            || Some(r.role) != self.z_role
            || r.zone as isize != self.z_zone
        {
            let rotated = self.z_act.is_some() && Some(zctl.active) != self.z_act;
            self.z_act = Some(zctl.active);
            self.z_role = Some(r.role);
            self.z_zone = r.zone as isize;
            self.goal_timer = 0.0;
            self.z_hold_until = 0.0;
            self.z_hold_dur = 0.0;
            self.z_aim_t = 0.0;
            self.z_aim = None;
            if rotated && self.mode == BotMode::Paint {
                self.path = None;
            }
        }
    }

    /// JS `_zoneEngage` (L1036-1045): take the fight? In range, or the foe
    /// is on / by our zone; otherwise keep inking the objective.
    fn zone_engage(&self, ctx: &BotCtx, a: &Actor, kind: WeaponKind) -> bool {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return false;
        };
        let Some((t, s)) = self.target else {
            return false;
        };
        if self.z_role == Some(ZoneRole::Push) {
            return true;
        }
        let Some(ti) = find_actor(ctx.actors, t, s) else {
            return false;
        };
        let tp = ctx.actors[ti].pos;
        let d = (tp.x - a.pos.x).hypot(tp.z - a.pos.z);
        let range = ctx.bot_range(kind);
        if d < range
            * if self.z_role == Some(ZoneRole::Watch) {
                1.05
            } else {
                1.25
            }
        {
            return true;
        }
        // stagers wait for the group (JS L1042)
        if matches!(self.z_role, Some(ZoneRole::Watch) | Some(ZoneRole::Stage)) {
            return false;
        }
        let zi = self.z_zone as usize;
        let c = zctl.zones[zi].center;
        (tp.x - c[0]).hypot(tp.z - c[2]) < zp.info[zi].r + 5.0
    }

    /// JS `_zoneHoldGround` (L1047-1057): in a fight, watchers keep their
    /// perch while the foe is in range; guards don't chase off the zone.
    fn zone_hold_ground(&self, ctx: &BotCtx, a: &Actor, tp: Vec3, kind: WeaponKind) -> bool {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return false;
        };
        let range = ctx.bot_range(kind);
        let d = (tp.x - a.pos.x).hypot(tp.z - a.pos.z);
        match self.z_role {
            Some(ZoneRole::Watch) => return d < range * 1.05 && d > 3.0,
            // hold the staging spot: shoot, don't chase (JS L1051)
            Some(ZoneRole::Stage) => return d > 4.0,
            Some(ZoneRole::Push) | None => return false,
            _ => {}
        }
        if d < 4.0 {
            return false;
        }
        // guards / painters: never chase a foe further off the zone than we
        // are (shoot from here, then back to the ink) (JS L1053-1056)
        let zi = self.z_zone as usize;
        let c = zctl.zones[zi].center;
        let r = zp.info[zi].r;
        let tz = (tp.x - c[0]).hypot(tp.z - c[2]);
        let sz = (a.pos.x - c[0]).hypot(a.pos.z - c[2]);
        tz > r + if self.z_role == Some(ZoneRole::Guard) {
            3.0
        } else {
            6.0
        } && tz > sz + 2.0
    }

    /// JS `_pickZoneGoal` (L1060-1095): the next spot to go to for the role.
    fn pick_zone_goal(&mut self, a: &Actor, ctx: &BotCtx, zp: &ZonePlan, kind: WeaponKind) {
        let Some(zctl) = ctx.zctl else {
            return;
        };
        let Some(idx) = find_actor(ctx.actors, a.team, a.slot) else {
            return;
        };
        let r = zp.role_of(idx, a.team, zctl);
        let zi = r.zone;
        let i = &zp.info[zi];
        let zone = &zctl.zones[zi];
        self.z_role = Some(r.role);
        self.z_zone = zi as isize;
        self.z_hold_dur = 0.0;
        let mut role = r.role;
        let need_frac = i.need_sum[a.team] / (i.reach.max(1)) as f32;
        // a guard with touch-ups to do paints; a watcher with nowhere to
        // perch (or a zone slipping away) paints too (JS L1066)
        if role == ZoneRole::Guard && need_frac > 0.1 {
            role = ZoneRole::Paint;
        }
        let mut id: isize = -1;
        let mut hold = 0.0f32;
        if role == ZoneRole::Stage {
            // regroup just outside the zone on our side and wait for the
            // push (JS L1068-1073)
            if let Some(st) = i.stage[a.team].as_ref() {
                let k = ((self.rng.next_f32() * st.spots.len() as f32) as usize)
                    .min(st.spots.len() - 1);
                id = st.spots[k] as isize;
                hold = 2.0 + self.rng.next_f32() * 2.0;
                self.goal_timer = 12.0;
            } else {
                role = ZoneRole::Guard;
            }
        }
        if role == ZoneRole::Push {
            id = self.zone_push_node(a, ctx, zp, i, zone) as isize;
            self.goal_timer = 4.0 + self.rng.next_f32() * 3.0;
        } else if role == ZoneRole::Watch {
            let w = self.zone_watch_node(a, ctx, zp, i, zone, kind);
            id = w;
            if w >= 0 {
                hold = 3.0 + self.rng.next_f32() * 3.0;
                self.goal_timer = 12.0;
            }
        } else if role == ZoneRole::Guard && id < 0 {
            id = self.zone_guard_node(a, ctx, zp, i, zone) as isize;
            hold = 1.0 + self.rng.next_f32() * 1.6;
            self.goal_timer = 7.0;
        }
        if id < 0 {
            id = self.zone_paint_node(a, ctx, i) as isize;
            hold = 0.0;
            self.goal_timer = 3.5 + self.rng.next_f32() * 2.0;
        }
        // nothing left to ink on it: stand guard on it (or, failing that,
        // press on past it) (JS L1089-1090)
        if id < 0 {
            id = self.zone_guard_node(a, ctx, zp, i, zone) as isize;
            hold = 1.0 + self.rng.next_f32() * 1.5;
            self.goal_timer = 6.0;
        }
        if id < 0 {
            id = self.zone_push_node(a, ctx, zp, i, zone) as isize;
            hold = 0.0;
            self.goal_timer = 4.0 + self.rng.next_f32() * 3.0;
        }
        if id < 0 {
            self.pick_paint_goal(a, ctx);
            return;
        }
        let n = ctx.nav.nodes[id as usize].pos;
        if self.path_to(ctx, a.pos, a.grounded, n, 0.3) {
            self.z_hold_dur = hold;
            self.z_fail = 0;
        } else {
            self.z_fail += 1;
            if self.z_fail >= 3 {
                self.z_fail = 0;
                self.wiggle(0.8, a.grounded);
            }
        }
    }

    /// JS `_zonePaintNode` (L1098-1115): the neediest reachable patch of the
    /// zone (its not-ours ink round a node), nearby first, away from
    /// teammates' goals. `usize::MAX` = JS `-1`.
    fn zone_paint_node(&mut self, a: &Actor, ctx: &BotCtx, i: &ZoneInfo) -> usize {
        let n_count = i.nodes.len();
        if n_count == 0 {
            return usize::MAX;
        }
        let my_idx = find_actor(ctx.actors, a.team, a.slot).unwrap_or(usize::MAX);
        let need = &i.need[a.team];
        let mut best = usize::MAX;
        let mut bs = f32::NEG_INFINITY;
        let step = if n_count > 110 { 2 } else { 1 };
        let mut li = ((self.rng.next_f32() * step as f32) as usize).min(step - 1);
        while li < n_count {
            let mut v = 0.0f32;
            for &k in &i.nb[li] {
                v += need[k];
            }
            if v >= 4.0 {
                let n = &ctx.nav.nodes[i.nodes[li]];
                let d =
                    (n.pos.x - a.pos.x).hypot(n.pos.z - a.pos.z) + (n.pos.y - a.pos.y).abs() * 1.5;
                let mut sc = v.min(60.0) * 0.25 - d * 0.16 + self.rng.next_f32() * 2.0;
                for (j, m) in ctx.actors.iter().enumerate() {
                    if j == my_idx || m.team != a.team || !m.alive {
                        continue;
                    }
                    if let Some(g) = ctx.mate_goals.get(j).copied().flatten() {
                        let gp = ctx.nav.nodes[g].pos;
                        if (gp.x - n.pos.x).hypot(gp.z - n.pos.z) < 3.5 {
                            sc -= 4.0;
                        }
                    }
                }
                if sc > bs {
                    bs = sc;
                    best = i.nodes[li];
                }
            }
            li += step;
        }
        best
    }

    /// JS `_zoneGuardNode` (L1117-1132): a spot on / at the edge of the
    /// zone, on its enemy-facing side, apart from the other guards.
    fn zone_guard_node(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        zp: &ZonePlan,
        i: &ZoneInfo,
        zone: &ZoneState,
    ) -> usize {
        let e = zp.enemy_dir(zone, a.team, ctx.world);
        let c = zone.center;
        let my_idx = find_actor(ctx.actors, a.team, a.slot).unwrap_or(usize::MAX);
        let mut best = usize::MAX;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..40 {
            if i.ring.is_empty() {
                break;
            }
            let id = i.ring
                [((self.rng.next_f32() * i.ring.len() as f32) as usize).min(i.ring.len() - 1)];
            let n = &ctx.nav.nodes[id];
            let dx = n.pos.x - c[0];
            let dz = n.pos.z - c[2];
            let d = dx.hypot(dz);
            if d > i.r + 3.0 || n.wet == 2 {
                continue;
            }
            let side = (dx * e[0] + dz * e[1]) / d.max(1.0);
            let mut sc = side * 2.0 + if d > i.r * 0.4 { 1.0 } else { 0.0 }
                - (n.pos.x - a.pos.x).hypot(n.pos.z - a.pos.z) * 0.05
                + self.rng.next_f32() * 1.5
                - (n.pos.y - c[1]).abs() * 0.5;
            for (j, m) in ctx.actors.iter().enumerate() {
                if j == my_idx || m.team != a.team || !m.alive {
                    continue;
                }
                if let Some(g) = ctx.mate_goals.get(j).copied().flatten() {
                    let gp = ctx.nav.nodes[g].pos;
                    if (gp.x - n.pos.x).hypot(gp.z - n.pos.z) < 4.0 {
                        sc -= 3.0;
                    }
                }
            }
            if sc > bs {
                bs = sc;
                best = id;
            }
        }
        best
    }

    /// JS `_zoneWatchNode` (L1134-1155): long range — a perch on our side of
    /// the zone, a little above it if possible, that can see it. `-1` = none.
    fn zone_watch_node(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        zp: &ZonePlan,
        i: &ZoneInfo,
        zone: &ZoneState,
        kind: WeaponKind,
    ) -> isize {
        let e = zp.enemy_dir(zone, a.team, ctx.world);
        let c = zone.center;
        let range = ctx.bot_range(kind);
        let my_idx = find_actor(ctx.actors, a.team, a.slot).unwrap_or(usize::MAX);
        let d_max = (range * 0.85).min(i.r + 12.0);
        let d_min = (i.r * 0.55).min(d_max - 2.0);
        let mut cands: Vec<(f32, usize)> = Vec::new();
        for _ in 0..70 {
            if i.ring.is_empty() {
                break;
            }
            let id = i.ring
                [((self.rng.next_f32() * i.ring.len() as f32) as usize).min(i.ring.len() - 1)];
            let n = &ctx.nav.nodes[id];
            let dx = n.pos.x - c[0];
            let dz = n.pos.z - c[2];
            let d = dx.hypot(dz);
            if d < d_min || d > d_max || n.wet == 2 {
                continue;
            }
            let side = (dx * e[0] + dz * e[1]) / d.max(1.0);
            let mut sc = (n.pos.y - c[1]).clamp(-2.0, 4.0) * 1.1
                - side * 1.6
                - (d - range * 0.55).abs() * 0.1
                - (n.pos.x - a.pos.x).hypot(n.pos.z - a.pos.z) * 0.04
                + self.rng.next_f32() * 1.2
                - if n.wet != 0 { 1.5 } else { 0.0 };
            for (j, m) in ctx.actors.iter().enumerate() {
                if j == my_idx || m.team != a.team || !m.alive {
                    continue;
                }
                if let Some(g) = ctx.mate_goals.get(j).copied().flatten() {
                    let gp = ctx.nav.nodes[g].pos;
                    if (gp.x - n.pos.x).hypot(gp.z - n.pos.z) < 5.0 {
                        sc -= 4.0;
                    }
                }
            }
            cands.push((sc, id));
        }
        // JS `cands.sort((x, y) => y[0] - x[0])` — a stable sort, so ties
        // keep insertion order; Rust's `sort_by` is stable too.
        cands.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
        let look = Vec3::new(c[0], c[1] + 0.6, c[2]);
        for &(_, id) in &cands[..cands.len().min(6)] {
            let n = &ctx.nav.nodes[id];
            if ctx
                .world
                .collision
                .los(Vec3::new(n.pos.x, n.pos.y + 1.4, n.pos.z), look)
            {
                return id as isize;
            }
        }
        -1
    }

    /// JS `_zonePushNode` (L1157-1174): pressure forward — unclaimed / enemy
    /// turf 3–18 m past the zone toward the enemy's side (never camping
    /// their spawn).
    fn zone_push_node(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        zp: &ZonePlan,
        i: &ZoneInfo,
        zone: &ZoneState,
    ) -> usize {
        let e = zp.enemy_dir(zone, a.team, ctx.world);
        let c = zone.center;
        let ep = ctx.world.spawn_pads[1 - a.team];
        let mut best = usize::MAX;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..28 {
            if i.far.is_empty() {
                break;
            }
            let id =
                i.far[((self.rng.next_f32() * i.far.len() as f32) as usize).min(i.far.len() - 1)];
            let n = &ctx.nav.nodes[id];
            if n.zone >= 0 || n.wet == 2 || (n.pos.x - ep.x).hypot(n.pos.z - ep.z) < 22.0 {
                continue;
            }
            let dx = n.pos.x - c[0];
            let dz = n.pos.z - c[2];
            let d = dx.hypot(dz);
            let side = (dx * e[0] + dz * e[1]) / d.max(1.0);
            if side < -0.1 {
                continue;
            }
            let near =
                ctx.paint
                    .region_stats(ctx.world.collision, n.pos.x, n.pos.y, n.pos.z, 3.0, a.team);
            if near.n == 0 {
                continue;
            }
            let v = near.empty + near.enemy * 1.4;
            let sc = v * 10.0 + side * 2.0 - (n.pos.x - a.pos.x).hypot(n.pos.z - a.pos.z) * 0.1
                + self.rng.next_f32() * 1.5
                - if n.wet != 0 { 1.5 } else { 0.0 };
            if sc > bs {
                bs = sc;
                best = id;
            }
        }
        best
    }

    /// JS `_zoneAim` (L1177-1205): a needy patch of our zone in reach of
    /// where we stand (re-picked ~1×/s); `None` = the normal paint scan
    /// decides.
    fn zone_aim(
        &mut self,
        ctx: &BotCtx,
        a: &Actor,
        dt: f32,
        moving: bool,
        move_v: Vec3,
        kind: WeaponKind,
    ) -> Option<ZAim> {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return None;
        };
        self.z_aim_t -= dt;
        if self.z_aim_t <= 0.0 {
            self.z_aim_t = 0.7 + self.rng.next_f32() * 0.4;
            self.z_aim = None;
            let zi = self.z_zone;
            if zi < 0 || !zp.is_active(zi as usize, zctl) {
                return None;
            }
            let i = &zp.info[zi as usize];
            let need = &i.need[a.team];
            // scan-reach ladder (JS L1185; slosher retired)
            let far = if kind.is_charge() {
                (ctx.bot_range(kind) * 0.9).min(16.0)
            } else if kind == WeaponKind::Bucket {
                7.0
            } else if kind == WeaponKind::Blaster {
                6.0
            } else {
                5.5
            };
            let c = zctl.zones[zi as usize].center;
            if (a.pos.x - c[0]).hypot(a.pos.z - c[2]) > i.r + far {
                return None;
            }
            let mut bs = f32::NEG_INFINITY;
            let mut bp: Option<Vec3> = None;
            for (li, &need_sc) in need.iter().enumerate() {
                if need_sc < 3.0 {
                    continue;
                }
                let n = ctx.nav.nodes[i.nodes[li]].pos;
                let d = (n.x - a.pos.x).hypot(n.z - a.pos.z);
                if d < 1.4 || d > far || (n.y - a.pos.y).abs() > 3.5 {
                    continue;
                }
                let sc = need_sc - d * 0.35;
                if sc > bs {
                    bs = sc;
                    bp = Some(n);
                }
            }
            if let Some(bp) = bp {
                // short-range kids only look aside for it when they're
                // standing, or it's roughly the way they're going (JS L1198)
                let ahead = !moving
                    || angle_diff(
                        move_v.x.atan2(move_v.z),
                        (bp.x - a.pos.x).atan2(bp.z - a.pos.z),
                    )
                    .abs()
                        < 1.1;
                if kind.is_charge() || ahead {
                    self.z_aim = Some(ZAim {
                        x: bp.x,
                        y: bp.y,
                        z: bp.z,
                        d: 0.0,
                    });
                }
            }
        }
        let z = self.z_aim.as_mut()?;
        z.d = (z.x - a.pos.x).hypot(z.z - a.pos.z);
        Some(*z)
    }

    /// JS `_zoneSpecial` (L1209-1229): specials break a hold on the zone
    /// (standing on / facing it), or defend it when foes turn up. JS rides
    /// `G.time`; the sim uses [`ZoneControl::clock`] (zones.rs module docs).
    fn zone_special(&mut self, ctx: &BotCtx, a: &Actor, fighting: bool) -> bool {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return false;
        };
        let t = a.team;
        let zi = self.z_zone;
        if zi < 0 || !zp.is_active(zi as usize, zctl) {
            return false;
        }
        let w = *zp.wave_of(t);
        if w.state == WaveState::Stage && !fighting {
            // saved for the push (JS L1212)
            return false;
        }
        if w.state == WaveState::Push && zctl.clock < zp.sp_next(t) {
            // chained: one teammate at a time (JS L1213)
            return false;
        }
        let zone = &zctl.zones[zi as usize];
        let i = &zp.info[zi as usize];
        let c = zone.center;
        let dz = (a.pos.x - c[0]).hypot(a.pos.z - c[2]);
        if dz > i.r + 12.0 {
            return false;
        }
        let ours = zctl.owner == t as i8;
        let foes = ctx
            .actors
            .iter()
            .filter(|o| {
                o.team != t && o.alive && (o.pos.x - c[0]).hypot(o.pos.z - c[2]) < i.r + 8.0
            })
            .count();
        // holding it quietly: keep it for the push-back (JS L1220)
        if ours && foes == 0 {
            return false;
        }
        // JS L1221: `SPECIALS[a.specialId]?.kind || a.specialId`
        let id_raw = a.special_id.as_deref().unwrap_or("");
        let id = def_by_id(id_raw).map_or_else(|| id_raw.to_string(), |d| d.kind(id_raw));
        let foes = foes > 0;
        if matches!(
            id.as_str(),
            "sonar" | "bubbler" | "wail" | "zooka" | "stamp" | "crab"
        ) && !foes
        {
            return false;
        }
        // thrown / aimed ones go where we look: only while facing the zone
        // or standing on it (fights: `_wantSpecial`'s aim) (JS L1224-1225)
        let thrown = matches!(
            id.as_str(),
            "storm" | "barrage" | "booyah" | "zooka" | "wail" | "blower"
        );
        if thrown
            && !fighting
            && dz > i.r * 0.7
            && angle_diff(self.aim_yaw, (c[0] - a.pos.x).atan2(c[2] - a.pos.z)).abs() > 0.6
        {
            self.z_face = 0.8;
            return false;
        }
        let go = self.rng.next_f32()
            < if fighting {
                0.03
            } else if w.state == WaveState::Push {
                0.2
            } else {
                0.08
            };
        if go && w.state == WaveState::Push {
            zp.set_sp_next(t, zctl.clock + 2.2 + self.rng.next_f32() * 0.8);
        }
        go
    }

    /// JS `_zoneBombAim` (L1233-1258): a thrown sub onto the biggest patch
    /// of their ink on our zone in lobbing range (re-looked-for twice a
    /// second); returns the aim to hold while it's thrown.
    fn zone_bomb_aim(&mut self, ctx: &BotCtx, a: &Actor, dt: f32) -> Option<ZoneBomb> {
        let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) else {
            return None;
        };
        if let Some(zb) = self.z_bomb.as_mut() {
            zb.t -= dt;
            if zb.t <= 0.0 {
                self.z_bomb = None;
                return None;
            }
            return Some(*zb);
        }
        self.z_bomb_scan -= dt;
        // JS L1236: `const sub = a.sub || SUB.bomb` — the sim always has a
        // sub slot set.
        let sub = a.sub;
        if self.z_bomb_scan > 0.0
            || self.bomb_cd.unwrap_or(0.0) > 0.0
            || !sub.is_thrown()
            || a.ink < ink_cost(sub, Self::sub_set()) + 10.0
            || self.z_role == Some(ZoneRole::Watch)
        {
            return None;
        }
        // JS L1238: one already out (Torpedo / Boomerang) — don't wind up
        if find_actor(ctx.actors, a.team, a.slot).is_some_and(|i| ctx.subs.kit_blocked(sub, i)) {
            return None;
        }
        self.z_bomb_scan = 0.5;
        let t = a.team;
        let zi = self.z_zone;
        if zi < 0 || !zp.is_active(zi as usize, zctl) {
            return None;
        }
        let zone = &zctl.zones[zi as usize];
        // only to open up a zone they hold / are taking (JS L1243)
        if zone.owner != (1 - t) as i8 && zone.share[1 - t] < 0.25 {
            return None;
        }
        let i = &zp.info[zi as usize];
        let ink = &i.ink[1 - t];
        let mut bv = 24.0f32;
        let mut bn: Option<Vec3> = None;
        let mut bd = 0.0f32;
        for li in 0..i.nodes.len() {
            if ink[li] < 6 {
                continue;
            }
            let n = ctx.nav.nodes[i.nodes[li]].pos;
            let d = (n.x - a.pos.x).hypot(n.z - a.pos.z);
            if d < 4.5 || d > 12.0 || (n.y - a.pos.y).abs() > 3.0 {
                continue;
            }
            let mut v = 0.0f32;
            for &k in &i.nb[li] {
                v += ink[k] as f32;
            }
            if v > bv {
                bv = v;
                bn = Some(n);
                bd = d;
            }
        }
        let bn = bn?;
        let pitch = lob_pitch(bd, bn.y - a.pos.y, sub_throw_speed(sub))?;
        let zb = ZoneBomb {
            yaw: (bn.x - a.pos.x).atan2(bn.z - a.pos.z),
            pitch,
            t: 1.1,
            release: false,
        };
        self.z_bomb = Some(zb);
        Some(zb)
    }

    // ---------------------------------------------------------------- subs

    /// Embedded `SUBS` (config.js L317+ via the tuning artifact) for
    /// [`ink_cost`]. Mirrors the [`crate::special::specials`] accessor.
    #[must_use]
    fn sub_set() -> &'static crate::tuning::SubSet {
        static SUBS: OnceLock<crate::tuning::SubSet> = OnceLock::new();
        SUBS.get_or_init(|| crate::embedded_tuning().subs)
    }

    /// This brain's target memory for [`BotCtx::mate_minds`] (JS reads the
    /// live `e.bot.target` / `e.bot.seeTimer`; the sim snapshots it).
    #[must_use]
    pub fn mind(&self) -> MateMind {
        MateMind {
            target: self.target,
            see_timer: self.see_timer,
        }
    }

    /// JS `_fightSub` (bots.js L950-967): a mid-duel sub throw. Sets
    /// [`Bot::bomb_cd`] and returns whether to throw this frame.
    fn fight_sub(&mut self, a: &Actor, ctx: &BotCtx, dist: f32) -> bool {
        // JS L952: the ink pad and the beacon ban run before the kit hook
        let cost = ink_cost(a.sub, Self::sub_set());
        if a.ink < cost + 8.0 || a.sub == SubKind::Beacon {
            return false;
        }
        // JS L953: kit subs decide for themselves
        if a.sub.is_kit() {
            return self.kit_sub_fight(a, ctx, dist);
        }
        // JS L954: the shared roll is drawn even when the arm rolls again
        let disc = self.diff.fire_discipline;
        let roll = self.rng.next_f32() < 0.02 * (1.0 + disc);
        let go = match a.sub {
            SubKind::Burst => dist > 3.0 && dist < 12.0 && self.rng.next_f32() < 0.04,
            SubKind::Scan => dist > 8.0 && dist < 18.0 && roll,
            SubKind::Curtain => {
                dist > 4.0 && dist < 14.0 && a.last_damage() < 0.6 && self.rng.next_f32() < 0.05
            }
            SubKind::Sprinkler => false,
            SubKind::Mine => dist < 10.0 && a.last_damage() < 1.5 && self.rng.next_f32() < 0.05,
            SubKind::Seeker => dist > 5.0 && dist < 15.0 && roll,
            // bomb, sticky, mist (JS `default`)
            _ => dist > 4.0 && dist < 14.0 && roll,
        };
        if go {
            self.bomb_cd = Some(if a.sub == SubKind::Burst {
                1.5 + self.rng.next_f32() * 2.0
            } else {
                5.0 + self.rng.next_f32() * 6.0
            });
        }
        go
    }

    /// The kit sub `SUB_KITS[k].bot.fight` hooks: waddle.js L751-755,
    /// tracer.js L463-467, torpedo.js L738-744, shaker.js L605-609,
    /// boomerang.js L572-583.
    fn kit_sub_fight(&mut self, a: &Actor, ctx: &BotCtx, dist: f32) -> bool {
        let disc = self.diff.fire_discipline;
        let own_idx = find_actor(ctx.actors, a.team, a.slot);
        match a.sub {
            // waddle: lob it into the fight, it walks the rest of the way
            SubKind::Waddle => {
                let go = dist > 4.5 && dist < 15.0 && self.rng.next_f32() < 0.02 * (1.0 + disc);
                if go {
                    self.bomb_cd = Some(6.0 + self.rng.next_f32() * 5.0);
                }
                go
            }
            // tracer: a cheap snap at a foe in mid range
            SubKind::Tracer => {
                let go = dist > 3.0 && dist < 17.0 && self.rng.next_f32() < 0.05;
                if go {
                    self.bomb_cd = Some(2.5 + self.rng.next_f32() * 3.0);
                }
                go
            }
            // torpedo: one out at a time (JS `blocked`, L733/L740)
            SubKind::Torpedo => {
                if own_idx.is_some_and(|i| ctx.subs.kit_blocked(SubKind::Torpedo, i)) {
                    return false;
                }
                let go = dist > 4.0 && dist < 13.5 && self.rng.next_f32() < 0.035;
                if go {
                    self.bomb_cd = Some(6.0 + self.rng.next_f32() * 5.0);
                }
                go
            }
            // shaker: a mid-range lob (the hold then charges it)
            SubKind::Shaker => {
                let go = dist > 4.0 && dist < 14.0 && self.rng.next_f32() < 0.03 * (1.0 + disc);
                if go {
                    self.bomb_cd = Some(5.0 + self.rng.next_f32() * 5.0);
                }
                go
            }
            // boomerang: only with the aim on the target (L572-583)
            SubKind::Boomerang => {
                if own_idx.is_some_and(|i| ctx.subs.kit_blocked(SubKind::Boomerang, i)) {
                    return false;
                }
                let Some(ti) = self.target.and_then(|(t, s)| find_actor(ctx.actors, t, s)) else {
                    return false;
                };
                if self.see_timer <= 0.0 {
                    return false;
                }
                let t = &ctx.actors[ti];
                let off = angle_diff(a.aim_yaw, (t.pos.x - a.pos.x).atan2(t.pos.z - a.pos.z)).abs();
                if off > 0.35f32.min(0.9 / dist.max(1.0)) {
                    return false;
                }
                let close = dist < 3.2 && a.last_damage() < 0.8 && self.rng.next_f32() < 0.08;
                let mid = dist > 4.0 && dist < 10.5 && self.rng.next_f32() < 0.035 * (1.0 + disc);
                if !close && !mid {
                    return false;
                }
                self.bomb_cd = Some(6.0 + self.rng.next_f32() * 5.0);
                true
            }
            _ => false,
        }
    }

    /// JS `_paintSub` (bots.js L969-983): a sub throw while painting.
    fn paint_sub(&mut self, a: &Actor, ctx: &BotCtx) -> bool {
        let cost = ink_cost(a.sub, Self::sub_set());
        // JS L971: the kit branch carries its own `+4` ink gate and returns
        // the hook's verdict directly — the generic placed / probability
        // gates below never see a kit sub.
        if a.sub.is_kit() {
            return a.ink >= cost + 4.0 && self.kit_sub_paint(a, ctx);
        }
        let placed = matches!(a.sub, SubKind::Mine | SubKind::Beacon);
        let pad = if placed { 4.0 } else { 20.0 };
        let roll_max = if placed { 0.04 } else { 0.01 };
        if a.ink < cost + pad || self.rng.next_f32() > roll_max {
            return false;
        }
        // JS L973-974: progress along the spawn-pad line (the unclamped JS
        // division is mirrored — `total` is never 0 on a real stage)
        let pads = ctx.world.spawn_pads;
        let total = pads[0].distance(pads[1]);
        let foe_pad = pads[1 - a.team];
        let progress = 1.0 - (a.pos.x - foe_pad.x).hypot(a.pos.z - foe_pad.z) / total;
        let go = match a.sub {
            // sprinkler: onto open turf ahead (JS L976-978)
            SubKind::Sprinkler => {
                let st = ctx.paint.region_stats(
                    ctx.world.collision,
                    a.pos.x + a.aim_yaw.sin() * 5.0,
                    a.pos.y,
                    a.pos.z + a.aim_yaw.cos() * 5.0,
                    3.0,
                    a.team,
                );
                st.n != 0 && st.own < 0.4
            }
            // mine: around the contested middle (JS L979)
            SubKind::Mine => progress > 0.3 && progress < 0.7,
            // beacon: up front, one per team stretch (JS L980)
            SubKind::Beacon => {
                progress > 0.4
                    && !ctx.subs.items.iter().any(|b| {
                        b.kind == SubKind::Beacon
                            && b.team == a.team
                            && b.state != SubState::Dead
                            && b.pos.distance(a.pos) < 12.0
                    })
            }
            _ => false,
        };
        if go {
            self.bomb_cd = Some(8.0 + self.rng.next_f32() * 8.0);
        }
        go
    }

    /// The kit sub `SUB_KITS[k].bot.paint` hooks: waddle.js L757-765,
    /// tracer.js L468-472, torpedo.js L745, shaker.js L611-615,
    /// boomerang.js L585-592.
    fn kit_sub_paint(&mut self, a: &Actor, ctx: &BotCtx) -> bool {
        match a.sub {
            // waddle: a foe we can't see is lurking near the landing point
            SubKind::Waddle => {
                if self.rng.next_f32() > 0.03 {
                    return false;
                }
                let lx = a.pos.x + a.aim_yaw.sin() * 7.5;
                let lz = a.pos.z + a.aim_yaw.cos() * 7.5;
                for e in ctx.actors {
                    if e.team == a.team || !e.alive || self.target == Some((e.team, e.slot)) {
                        continue;
                    }
                    if (e.pos.x - lx).hypot(e.pos.z - lz) < 6.5 && (e.pos.y - a.pos.y).abs() < 3.0 {
                        self.bomb_cd = Some(9.0 + self.rng.next_f32() * 6.0);
                        return true;
                    }
                }
                false
            }
            // tracer: now and then skim one along the ground for puddles
            SubKind::Tracer => {
                let go = self.rng.next_f32() < 0.006;
                if go {
                    self.bomb_cd = Some(6.0 + self.rng.next_f32() * 6.0);
                }
                go
            }
            // torpedo: never while painting
            SubKind::Torpedo => false,
            // shaker: three hops leave a long stripe of ink
            SubKind::Shaker => {
                let go = self.rng.next_f32() < 0.0012;
                if go {
                    self.bomb_cd = Some(9.0 + self.rng.next_f32() * 8.0);
                }
                go
            }
            // boomerang: onto a patch ahead that isn't ours yet
            SubKind::Boomerang => {
                let blocked = find_actor(ctx.actors, a.team, a.slot)
                    .is_some_and(|i| ctx.subs.kit_blocked(SubKind::Boomerang, i));
                if blocked || self.rng.next_f32() > 0.006 {
                    return false;
                }
                let st = ctx.paint.region_stats(
                    ctx.world.collision,
                    a.pos.x + a.aim_yaw.sin() * 8.0,
                    a.pos.y,
                    a.pos.z + a.aim_yaw.cos() * 8.0,
                    2.5,
                    a.team,
                );
                if st.n == 0 || st.own > 0.45 {
                    return false;
                }
                self.bomb_cd = Some(9.0 + self.rng.next_f32() * 7.0);
                true
            }
            _ => false,
        }
    }

    // ---------------------------------------------------------------- specials

    /// JS `_wantSpecial` (bots.js L830-856): when to fire the special. The
    /// single `Math.random()` draw of the JS prologue happens once here.
    fn want_special(&mut self, a: &Actor, ctx: &BotCtx, fight: bool, dist: f32, vis: bool) -> bool {
        // JS L831: `SPECIALS[a.specialId]?.kind || a.specialId`
        let id_raw = a.special_id.as_deref().unwrap_or("");
        let id = def_by_id(id_raw).map_or_else(|| id_raw.to_string(), |d| d.kind(id_raw));
        let r = self.rng.next_f32();
        if fight {
            let last_damage = a.last_damage();
            return match id.as_str() {
                "slam" => dist < 4.5,
                "storm" => dist < 16.0,
                "barrage" => dist < 14.0,
                "bubbler" => dist < 12.0 && (last_damage < 0.6 || r < 0.02),
                "sonar" => r < 0.02,
                "strike" => dist > 14.0 && r < 0.01,
                "zooka" => vis && dist > 7.0 && dist < 36.0,
                "wail" => vis && dist > 5.0 && dist < 40.0,
                "kraken" => dist < 10.0,
                "blower" => dist < 14.0,
                "jetpack" => dist > 5.0 && dist < 22.0,
                "stamp" => dist < 9.0,
                "booyah" => dist > 9.0 && dist < 26.0 && r < 0.03,
                "zipcaster" => dist < 16.0,
                "crab" => dist < 20.0,
                _ => false,
            };
        }
        // JS L852: only the turf-claiming specials get a paint-mode press
        let turf = matches!(
            id.as_str(),
            "storm"
                | "strike"
                | "booyah"
                | "barrage"
                | "sonar"
                | "kraken"
                | "crab"
                | "zooka"
                | "blower"
                | "stamp"
        );
        if !turf || r > 0.012 {
            return false;
        }
        ctx.paint
            .region_stats(ctx.world.collision, a.pos.x, a.pos.y, a.pos.z, 6.0, a.team)
            .own
            < 0.55
    }

    /// JS `_specialCtl` (bots.js L859-948): driving a running special +
    /// cheering a teammate's Cheer Orb. Batch 3 adds the Zone Control strike
    /// aiming (L872-877).
    #[allow(clippy::too_many_arguments)]
    fn special_ctl(
        &mut self,
        dt: f32,
        a: &mut Actor,
        ctx: &BotCtx,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        dist: f32,
        vis: bool,
    ) {
        let cur = a.special_active.as_deref();
        // JS L862-864: cheer on a teammate's charging Cheer Orb
        if (cur.is_none() || cur.is_some_and(|x| x.id != "booyah"))
            && self.rng.next_f32() < dt * 1.3
        {
            let own_idx = find_actor(ctx.actors, a.team, a.slot);
            for (i, o) in ctx.actors.iter().enumerate() {
                if Some(i) == own_idx || o.team != a.team {
                    continue;
                }
                if let Some(os) = o.special_active.as_deref()
                    && os.id == "booyah"
                    && !matches!(&os.state, KindState::Booyah(st) if st.thrown)
                {
                    it.cheer = true;
                    break;
                }
            }
        }
        // JS L866-870: don't waste shots or bombs into a Mega Stamp's front
        // guard
        if let Some(ti) = self
            .target
            .and_then(|(t, sl)| find_actor(ctx.actors, t, sl))
            && let Some(tsa) = ctx.actors[ti].special_active.as_deref()
            && tsa.id == "stamp"
            && matches!(&tsa.state, KindState::Stamp(st) if st.guard > 0.0)
        {
            let t = &ctx.actors[ti];
            let fy = match &tsa.state {
                KindState::Stamp(st) => st.body_yaw,
                _ => t.yaw,
            };
            let rx = a.pos.x - t.pos.x;
            let rz = a.pos.z - t.pos.z;
            let rl = rx.hypot(rz);
            let rl = if rl == 0.0 { 1.0 } else { rl };
            if (rx * fy.sin() + rz * fy.cos()) / rl > 1.2f32.cos() {
                it.fire = false;
                it.sub = false;
            }
        }
        // JS L872-877: Zone Control — a Vortex Strike lands on the live
        // zone (on its enemy ink when we're taking it back).
        if let (Some(zp), Some(zctl)) = (ctx.zones, ctx.zctl) {
            let pending = a.special_active.as_deref().is_some_and(|sa| {
                sa.id == "strike"
                    && matches!(&sa.state, KindState::Strike(st) if st.aiming && !st.zone_aimed)
            });
            if pending
                && let Some(st) = a
                    .special_active
                    .as_mut()
                    .and_then(|sa| match &mut sa.state {
                        KindState::Strike(st) => Some(st),
                        _ => None,
                    })
            {
                st.zone_aimed = true;
                if let Some(p) = zp.hotspot(self.z_zone, a.team, zctl, ctx.nav) {
                    st.target = Vec3::new(p.x, 0.0, p.z);
                }
            }
        }
        let s = a.special_active.as_deref();
        let Some(s) = s else { return };
        let fighting = self.mode == BotMode::Fight && self.target.is_some();
        self.sp_t += dt;
        // JS L888: `switch (s.kind || s.id)`
        let key = if s.kind.is_empty() { &s.id } else { &s.kind };
        match key.as_str() {
            "barrage" => {
                self.barrage_cd -= dt;
                if self.barrage_cd <= 0.0 && (!fighting || dist < 16.0) {
                    it.sub = true;
                    self.bomb_aim = true;
                    self.barrage_cd = 0.35 + self.rng.next_f32() * 0.3;
                }
            }
            "zooka" => {
                it.fire = if fighting {
                    vis && dist < 42.0
                } else {
                    self.rng.next_f32() < 0.06
                };
                it.squid = false;
            }
            "kraken" => {
                it.squid = false;
                if fighting {
                    self.sp_toward(a, ctx, move_v, dist);
                    it.fire = dist < 3.6 && vis;
                    if a.grounded && dist > 6.0 && self.rng.next_f32() < dt * 0.8 {
                        it.jump = true;
                    }
                }
            }
            "stamp" => {
                it.squid = false;
                self.stamp_flip -= dt;
                if self.stamp_flip > 0.0 {
                    // mid-jump: swing once airborne = the flip (JS L907)
                    if !a.grounded {
                        it.fire = true;
                    }
                    return;
                }
                if fighting {
                    if let Some(ti) = self
                        .target
                        .and_then(|(t, sl)| find_actor(ctx.actors, t, sl))
                    {
                        let t = &ctx.actors[ti];
                        let body_yaw = match &s.state {
                            KindState::Stamp(st) => st.body_yaw,
                            _ => a.yaw,
                        };
                        let off =
                            angle_diff(body_yaw, (t.pos.x - a.pos.x).atan2(t.pos.z - a.pos.z))
                                .abs();
                        let near3 = (t.pos.y - a.pos.y).abs() < 1.8;
                        // JS L911-912: two flip arms with identical bodies
                        // (right behind / just out of reach) — merged, the
                        // conditions are pure comparisons
                        if a.grounded
                            && near3
                            && ((dist < 2.6 && off > 2.3)
                                || (off < 0.3 && dist > 3.3 && dist < 4.3))
                        {
                            it.jump = true;
                            self.stamp_flip = 0.4;
                        } else if off > 0.9 && dist < 9.0 {
                            *move_v = Vec3::ZERO;
                        } else {
                            // charge straight at the target (unless it runs
                            // off a ledge) (JS L916-918)
                            if near3 && dist < 12.0 {
                                let dx = t.pos.x - a.pos.x;
                                let dz = t.pos.z - a.pos.z;
                                let l = dx.hypot(dz);
                                let l = if l == 0.0 { 1.0 } else { l };
                                if ctx.world.collision.ground_height(
                                    a.pos.x + (dx / l) * 1.5,
                                    a.pos.z + (dz / l) * 1.5,
                                    a.pos.y + 1.0,
                                    false,
                                ) >= a.pos.y - 1.5
                                {
                                    *move_v = Vec3::new(dx / l, 0.0, dz / l);
                                }
                            }
                            let d = &specials().stamp;
                            it.fire = off < 0.7 && dist < d.reach + d.radius + 0.5;
                        }
                        if vis && dist > 9.0 && dist < 20.0 && self.rng.next_f32() < dt * 0.8 {
                            it.sub = true;
                        }
                    }
                } else {
                    it.fire = self.rng.next_f32() < 0.5;
                }
            }
            "crab" => {
                it.fire = if fighting {
                    vis && dist < 24.0
                } else {
                    self.rng.next_f32() < 0.3
                };
                it.sub =
                    fighting && vis && dist > 8.0 && dist < 22.0 && self.rng.next_f32() < dt * 1.5;
                let low = matches!(&s.state, KindState::Crab(st) if st.hp < specials().crab.hp as f32 * 0.3);
                it.squid = (fighting && low && dist > 8.0)
                    || (!fighting && self.path_remaining(a, ctx) > 12.0);
            }
            "blower" => {
                self.blow_t += dt;
                it.fire = self.blow_t < 0.9;
                if self.blow_t > 1.4 {
                    self.blow_t = 0.0;
                }
                it.squid = false;
            }
            "booyah" => {
                it.squid = false;
                let charged = matches!(&s.state, KindState::Booyah(st) if st.charge >= 1.0);
                it.fire = charged && self.rng.next_f32() < dt * 3.0;
            }
            "jetpack" => {
                it.fire = fighting && vis && dist < 30.0;
            }
            "zipcaster"
                if fighting
                    && vis
                    && dist > 6.0
                    && dist < 18.0
                    && self.rng.next_f32() < dt * 0.6 =>
            {
                it.sub = true;
            }
            _ => {}
        }
    }

    /// JS `toward()` (bots.js L881-886): charge straight at the target only
    /// when it's close and the way doesn't drop off a ledge.
    fn sp_toward(&self, a: &Actor, ctx: &BotCtx, move_v: &mut Vec3, dist: f32) {
        let Some(ti) = self
            .target
            .and_then(|(t, sl)| find_actor(ctx.actors, t, sl))
        else {
            return;
        };
        let t = &ctx.actors[ti];
        if dist > 5.0 || (t.pos.y - a.pos.y).abs() > 1.2 {
            return;
        }
        let dx = t.pos.x - a.pos.x;
        let dz = t.pos.z - a.pos.z;
        let l = dx.hypot(dz);
        let l = if l == 0.0 { 1.0 } else { l };
        if ctx.world.collision.ground_height(
            a.pos.x + (dx / l) * 1.5,
            a.pos.z + (dz / l) * 1.5,
            a.pos.y + 1.0,
            false,
        ) < a.pos.y - 1.5
        {
            return;
        }
        *move_v = Vec3::new(dx / l, 0.0, dz / l);
    }

    // ---------------------------------------------------------------- kit hooks

    /// JS `MAIN_KITS.brolly.bot.fight` (kits/brolly.js L560-594): open the
    /// canopy when taking fire or when a long weapon lines up from roughly
    /// ahead; plant / launch behind it. The `advanceUntil` follow-the-
    /// launched-canopy steering (L586-592) is cut (module docs).
    #[allow(clippy::too_many_arguments)]
    fn brolly_fight(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        dist: f32,
        range: f32,
        move_v: &mut Vec3,
        wr: &WeaponRunner,
    ) -> bool {
        // JS `G.time` → the brain's own clock (module docs)
        let t = self.t_time;
        let grown = wr
            .brolly
            .as_ref()
            .is_some_and(|k| k.state == CanopyState::Ready && k.grow > 0.9);
        if grown && self.bb_hold_until <= t && t >= self.bb_next_shield && dist < 18.0 {
            let thr = self.line_threat(a, ctx);
            let ahead = thr.is_some_and(|i| {
                let e = &ctx.actors[i];
                angle_diff(a.aim_yaw, (e.pos.x - a.pos.x).atan2(e.pos.z - a.pos.z)).abs() < 0.9
            });
            let hurt = a.last_damage() < 0.45
                && a.last_attacker()
                    .and_then(|(tk, sl)| find_actor(ctx.actors, tk, sl))
                    .is_some_and(|i| {
                        ctx.actors[i].alive
                            && angle_diff(
                                a.aim_yaw,
                                (ctx.actors[i].pos.x - a.pos.x)
                                    .atan2(ctx.actors[i].pos.z - a.pos.z),
                            )
                            .abs()
                                < 1.0
                    });
            let disc = self.diff.fire_discipline;
            if (ahead && self.rng.next_f32() < 0.35 * dt * 60.0 * disc)
                || (hurt && self.rng.next_f32() < 0.2 * dt * 60.0 * disc)
            {
                let own_idx = find_actor(ctx.actors, a.team, a.slot);
                let allies = ctx
                    .actors
                    .iter()
                    .enumerate()
                    .filter(|(i, o)| {
                        Some(*i) != own_idx
                            && o.alive
                            && o.team == a.team
                            && o.pos.distance_squared(a.pos) < 49.0
                    })
                    .count();
                let launch_it = dist > 4.0
                    && dist < 14.0
                    && (allies > 0 || ahead)
                    && self.rng.next_f32() < if allies > 0 { 0.45 } else { 0.2 };
                let w = &ctx.weapons.brolly;
                let hold = if launch_it {
                    w.open_delay + w.open_time + w.launch_hold + 0.12
                } else {
                    0.5 + self.rng.next_f32() * 0.3
                };
                self.bb_hold_until = t + hold;
                self.bb_launch = launch_it;
                self.bb_next_shield = t + hold + 0.9 + self.rng.next_f32() * 1.4;
            }
        }
        let ready_state = wr
            .brolly
            .as_ref()
            .is_some_and(|k| k.state == CanopyState::Ready);
        if self.bb_hold_until > t && ready_state {
            if !self.bb_launch {
                // plant behind the canopy (it slows you anyway) (JS L581)
                *move_v *= 0.4;
            }
            return true;
        }
        if self.bb_hold_until > t {
            // broke / launched mid-hold (JS L584)
            self.bb_hold_until = 0.0;
        }
        dist < range * 1.1
    }

    /// JS `lineThreat` (kits/brolly.js L545-557): an enemy long weapon
    /// lining up on this bot. JS gates on `CHARGES || LONG || KB.long ||
    /// KB.charges`; no kit bot hook defines `long` / `charges`, so the
    /// filter is `is_long()` (module docs).
    fn line_threat(&self, a: &Actor, ctx: &BotCtx) -> Option<usize> {
        for (i, e) in ctx.actors.iter().enumerate() {
            if e.team == a.team || !e.alive {
                continue;
            }
            let ek = ctx.kinds.get(i).copied()?;
            if !ek.is_long() {
                continue;
            }
            let er = &ctx.runners[i];
            if !(er.charging_flag() || er.firing() || er.streaming() || er.burst_t() > 0.0) {
                continue;
            }
            let dx = a.pos.x - e.pos.x;
            let dy = a.pos.y + 0.8 - (e.pos.y + 1.1);
            let dz = a.pos.z - e.pos.z;
            let d = Vec3::new(dx, dy, dz).length();
            if d > ctx.weapon_reach(ek) + 3.0 || d < 1.0 {
                continue;
            }
            let ad = e.aim_dir();
            if (ad.x * dx + ad.y * dy + ad.z * dz) / d < 0.14f32.cos() {
                continue;
            }
            return Some(i);
        }
        None
    }

    /// JS `MAIN_KITS.bow.bot.fight` (kits/bow.js L453-475): start the draw
    /// in range off cooldown, then loose at the wanted ring (jump-laning the
    /// release when asked). The `groupYaw` fan bias (L473) is cut (module
    /// docs).
    #[allow(clippy::too_many_arguments)]
    fn bow_fight(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dist: f32,
        range: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        ti: usize,
        wr: &WeaponRunner,
    ) -> bool {
        let bow = &ctx.weapons.bow;
        if !wr.charging_flag() {
            if dist > range * 1.1 || wr.cooldown() > 0.0 {
                return false;
            }
            self.bow_want = if dist > 11.0 { 1.0 } else { bow.ring1 + 0.08 };
            let dy = ctx.actors[ti].pos.y - a.pos.y;
            self.bow_jump = a.grounded
                && dist > 4.0
                && dist < 17.0
                && (dy.abs() > 1.3 || self.rng.next_f32() < 0.18);
            self.bow_jumped = false;
            return true;
        }
        *move_v *= 0.4;
        let want = self
            .bow_want
            .min((a.ink / bow.ink_full).clamp(0.0, 1.0) - 0.001);
        if wr.charge() < want {
            return true;
        }
        if self.bow_jump {
            if !self.bow_jumped {
                if a.grounded {
                    it.jump = true;
                    self.bow_jumped = true;
                }
                return true;
            }
            if a.grounded && a.vel.y <= 0.5 {
                // landed already: just loose (JS L469)
                self.bow_jump = false;
                return false;
            }
            if a.vel.y > 1.5 {
                // rising: loose near the top (JS L470)
                return true;
            }
            return false;
        }
        false
    }

    /// JS `MAIN_KITS.bow.bot.paint` (kits/bow.js L476-490): draw a paint
    /// ring down the lane, hold off when the path is stuck.
    fn bow_paint(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        need_paint: bool,
        ink_frac: f32,
        wr: &WeaponRunner,
    ) -> bool {
        let bow = &ctx.weapons.bow;
        if wr.charging_flag() {
            // JS L480: `k.botPaint || w.ring1 + 0.05`
            let target = if self.bow_paint > 0.0 {
                self.bow_paint
            } else {
                bow.ring1 + 0.05
            };
            let done = wr.charge() >= target.min((a.ink / bow.ink_full).clamp(0.0, 1.0) - 0.001);
            if done {
                self.paint_pause = 0.3 + self.rng.next_f32() * 0.35;
            }
            return !done;
        }
        if !(need_paint && ink_frac > 0.25 && self.paint_pause <= 0.0 && wr.cooldown() <= 0.0) {
            return false;
        }
        // not getting anywhere: hold off the next draw (JS L487)
        if self.no_prog > 0.25 {
            return false;
        }
        self.bow_paint = if self.rng.next_f32() < 0.3 {
            1.0
        } else {
            bow.ring1 + 0.05
        };
        true
    }

    /// JS `MAIN_KITS.blade.bot.paint` (kits/blade.js L726-741): quick cuts
    /// on the move with a reserve, now and then a charged wave down a long
    /// lane — including the `_blHold` bookkeeping (Task 47 batch D).
    #[allow(clippy::too_many_arguments)]
    fn blade_paint(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        it: &mut ActorInput,
        need_paint: bool,
        ink_frac: f32,
        want_move: bool,
        wr: &WeaponRunner,
    ) -> bool {
        self.bl_wave_t -= dt;
        if wr.blade.as_ref().is_some_and(|k| k.charging) {
            if wr.blade.as_ref().is_some_and(|k| k.full) {
                // loose a paint wave down the lane (JS L732)
                it.bl_hold = Some(-1.0);
                self.bl_wave_t = 3.5 + self.rng.next_f32() * 3.0;
                return false;
            }
            it.bl_hold = Some(ctx.time); // JS L733: keep holding the charge
            return true;
        }
        it.bl_hold = Some(-1.0); // JS L735
        if self.bl_wave_t <= 0.0
            && want_move
            && ink_frac > 0.45
            && need_paint
            && self.path_remaining(a, ctx) > 9.0
        {
            it.bl_hold = Some(ctx.time); // JS L737: arm the wave charge
            return true;
        }
        if need_paint && ink_frac > 0.25 && want_move {
            self.bl_press = !self.bl_press;
            return self.bl_press;
        }
        false
    }

    /// JS `MAIN_KITS.mitts.bot.fight` (kits/mitts.js L762-816, Task 47 batch
    /// D): punch in reach; leap away when hurt and cornered; pounce from mid
    /// range; now and then onto a wall by the fight, punching down from it.
    #[allow(clippy::too_many_arguments)]
    fn mitts_fight(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        dist: f32,
        range: f32,
        it: &mut ActorInput,
        ti: usize,
        wr: &WeaponRunner,
    ) -> bool {
        let w = &ctx.weapons.mitts;
        self.mitts_cd_state();
        {
            // JS L765: the clocks tick even on a charging frame.
            let b = self.mitts_cd.as_mut().unwrap();
            b.leap_cd -= dt;
            b.wall_cd -= dt;
            b.escape_cd -= dt;
        }
        if wr.mitts.as_ref().is_some_and(|k| k.charging || k.leaping) {
            return false;
        }
        let in_reach = dist < range * 1.05;
        let t = &ctx.actors[ti];
        if let Some(kit) = wr.mitts.as_ref()
            && kit.cling
        {
            // punch down from the wall while they're in reach; after a
            // while leap at them (or drop) (JS L768-779)
            if kit.bot_hold <= 0.0 || a.ink < 18.0 {
                if a.ink > w.leap_ink_max + 8.0 && dist > 4.0 && dist < 12.0 {
                    let yaw = (t.pos.x - a.pos.x).atan2(t.pos.z - a.pos.z);
                    let (c, pred) = self.bot_plan(a, ctx, kit, yaw, -0.1, dist);
                    if self.landing_ok(a, ctx, &pred)
                        && self.bot_leap(a, ctx, it, c, yaw, -0.1, false)
                    {
                        self.mitts_cd.as_mut().unwrap().leap_cd = 3.0 + self.rng.next_f32() * 3.0;
                        return false;
                    }
                }
                self.mitts_drop(it); // JS L776
            }
            return in_reach;
        }
        let hp = a.hp / ctx.t.hp;
        let clear = ctx.world.collision.los(
            vec3(a.pos.x, a.pos.y + 1.1, a.pos.z),
            vec3(t.pos.x, t.pos.y + 1.0, t.pos.z),
        );
        // escape: hurt and in their face → leap away (never into the sea /
        // onto a roof) (JS L782-788)
        if a.grounded
            && hp < 0.35
            && dist < 5.0
            && a.ink > w.leap_ink_max
            && self.mitts_cd.as_ref().unwrap().escape_cd <= 0.0
            && self.rng.next_f32() < dt * 3.0
        {
            self.mitts_cd.as_mut().unwrap().escape_cd = 3.0;
            let away =
                (a.pos.x - t.pos.x).atan2(a.pos.z - t.pos.z) + (self.rng.next_f32() - 0.5) * 1.2;
            let (c, pred) = self.bot_plan(a, ctx, wr.mitts.as_ref().unwrap(), away, 0.05, 11.0);
            if self.landing_ok(a, ctx, &pred) && self.bot_leap(a, ctx, it, c, away, 0.05, false) {
                self.mitts_cd.as_mut().unwrap().leap_cd = 4.0;
                return false;
            }
        }
        // pounce from mid range (JS L789-797)
        if a.grounded
            && self.mitts_cd.as_ref().unwrap().leap_cd <= 0.0
            && dist > 5.5
            && dist < 12.5
            && a.ink >= w.leap_ink_max + 8.0
            && clear
            && (t.pos.y - a.pos.y).abs() < 2.5
            && self.rng.next_f32() < dt * 3.0
        {
            self.mitts_cd.as_mut().unwrap().leap_cd = 2.0 + self.rng.next_f32() * 2.0;
            let lead = 0.8;
            let tx = t.pos.x + t.vel.x * lead;
            let tz = t.pos.z + t.vel.z * lead;
            let yaw = (tx - a.pos.x).atan2(tz - a.pos.z);
            let d = (tx - a.pos.x).hypot(tz - a.pos.z);
            let (c, pred) = self.bot_plan(a, ctx, wr.mitts.as_ref().unwrap(), yaw, 0.0, d);
            if self.landing_ok(a, ctx, &pred)
                && (pred.pos.x - tx).hypot(pred.pos.z - tz) < 2.6
                && self.bot_leap(a, ctx, it, c, yaw, 0.0, false)
            {
                return false;
            }
        }
        // now and then: onto a wall by the fight (JS L798-813)
        if a.grounded
            && self.mitts_cd.as_ref().unwrap().wall_cd <= 0.0
            && dist > 3.0
            && dist < 11.0
            && a.ink >= 70.0
            && self.rng.next_f32() < dt * 0.7
        {
            self.mitts_cd.as_mut().unwrap().wall_cd = 7.0 + self.rng.next_f32() * 7.0;
            if let Some((wyaw, _)) = self.bot_find_wall(a, ctx, t) {
                let kit = wr.mitts.as_ref().unwrap();
                for c in [0.35, 0.55, 0.75, 1.0] {
                    // JS L804-806: arm the scratch only for the probe — the
                    // plan probe rides the explicit yaw / pitch instead.
                    let pred = predict_leap(
                        a.pos,
                        plan_velocity(kit, c, wyaw, 0.3, w),
                        ctx.world.collision,
                        ctx.t,
                    );
                    if pred.kind == LeapKind::Wall && pred.pos.y > a.pos.y + 0.8 {
                        if self.bot_leap(a, ctx, it, c, wyaw, 0.3, true) {
                            return false;
                        }
                        break;
                    }
                }
            }
        }
        // JS L814-815: `void move;` — the fight never touches the move.
        in_reach
    }

    /// JS `MAIN_KITS.mitts.bot.paint` (kits/mitts.js L817-831, Task 47 batch
    /// D): hang on a wall and ink the ground below it, now and then a long
    /// pounce along the path, else the quick-cut tail.
    #[allow(clippy::too_many_arguments)]
    fn mitts_paint(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        move_v: &Vec3,
        it: &mut ActorInput,
        need_paint: bool,
        ink_frac: f32,
        want_move: bool,
        wr: &WeaponRunner,
    ) -> bool {
        self.mitts_cd_state();
        self.mitts_cd.as_mut().unwrap().travel_cd -= dt;
        if wr.mitts.as_ref().is_some_and(|k| k.charging || k.leaping) {
            return false;
        }
        if let Some(kit) = wr.mitts.as_ref()
            && kit.cling
        {
            // hang a moment, ink the wall + the ground below (JS L822)
            if kit.bot_hold <= 0.0 || ink_frac < 0.15 {
                self.mitts_drop(it);
            }
            return ink_frac > 0.1;
        }
        // cover ground: a long pounce along the path (paints the landing),
        // with plenty of ink and a long way to go (JS L823-829)
        if self.mitts_cd.as_ref().unwrap().travel_cd <= 0.0
            && a.grounded
            && ink_frac > 0.7
            && want_move
            && self.path_remaining(a, ctx) > 14.0
            && self.rng.next_f32() < dt * 0.8
        {
            self.mitts_cd.as_mut().unwrap().travel_cd = 6.0 + self.rng.next_f32() * 6.0;
            let yaw = move_v.x.atan2(move_v.z);
            let (c, pred) = self.bot_plan(a, ctx, wr.mitts.as_ref().unwrap(), yaw, 0.0, 11.0);
            if self.landing_ok(a, ctx, &pred) && self.bot_leap(a, ctx, it, c, yaw, 0.0, false) {
                return false;
            }
        }
        ink_frac > 0.08 && (need_paint || self.rng.next_f32() < 0.02) && want_move
    }

    /// JS `botState(brain)` (kits/mitts.js L667): the leap clocks, created
    /// lazily on the first fight / paint hook frame (three draws).
    fn mitts_cd_state(&mut self) {
        if self.mitts_cd.is_none() {
            self.mitts_cd = Some(MittsCd {
                leap_cd: 1.5 + self.rng.next_f32() * 2.0,
                wall_cd: 4.0 + self.rng.next_f32() * 4.0,
                travel_cd: 4.0 + self.rng.next_f32() * 4.0,
                escape_cd: 0.0,
            });
        }
    }

    /// JS `botPlan(a, k, yaw, pitch, dist)` L669-682: the charge fraction
    /// whose leap lands at `dist` (two secant passes over the real
    /// prediction). Deviation: JS arms `k.botWant` for the probe; the brain
    /// rides [`plan_velocity`] instead — same arc, kit scratch untouched.
    fn bot_plan(
        &self,
        a: &Actor,
        ctx: &BotCtx,
        kit: &MittsKit,
        yaw: f32,
        pitch: f32,
        dist: f32,
    ) -> (f32, LeapPred) {
        let w = &ctx.weapons.mitts;
        let mut c = ((dist - 4.5) / 8.5).clamp(0.0, 1.0);
        let mut pred = predict_leap(
            a.pos,
            plan_velocity(kit, c, yaw, pitch, w),
            ctx.world.collision,
            ctx.t,
        );
        for i in 0..3 {
            if i > 0 {
                pred = predict_leap(
                    a.pos,
                    plan_velocity(kit, c, yaw, pitch, w),
                    ctx.world.collision,
                    ctx.t,
                );
            }
            if i == 2 {
                break; // JS L675: the third pass is the answer, not a probe
            }
            let got = (pred.pos.x - a.pos.x).hypot(pred.pos.z - a.pos.z);
            // JS `!(got > 0.5)` (L676): breaks on a NaN read too.
            if got.is_nan() || got <= 0.5 {
                break;
            }
            c = (c + (dist - got) / 8.5).clamp(0.0, 1.0);
        }
        (c, pred)
    }

    /// JS `landingOk(a, P)` L685-695: a landing a bot may aim for.
    /// Deviation: the Rust layout's contacts carry no `roof` / `perch` /
    /// `rail` block flags (same rule as `clingable_at`), so the
    /// `blocks[P.block]` test (L687-688) is absent; JS `!G.nav` never holds.
    fn landing_ok(&self, a: &Actor, ctx: &BotCtx, pred: &LeapPred) -> bool {
        if pred.kind != LeapKind::Ground {
            return false;
        }
        if (pred.pos.y - a.pos.y).abs() > 4.0 {
            return false;
        }
        let Some(id) = ctx.nav.nearest(pred.pos, 0.6, false) else {
            return false;
        };
        let n = ctx.nav.nodes[id].pos;
        (n.x - pred.pos.x).hypot(n.z - pred.pos.z) < 1.3 && (n.y - pred.pos.y).abs() < 0.6
    }

    /// JS `botLeap(r, k, c, yaw, pitch, kind)` L696-701, routed through the
    /// jump hook: the ink gate is `startCharge`'s (L357) — the kit arms the
    /// plan on the press and releases through `bot_release_ok`.
    #[allow(clippy::too_many_arguments)]
    fn bot_leap(
        &self,
        a: &Actor,
        ctx: &BotCtx,
        it: &mut ActorInput,
        c: f32,
        yaw: f32,
        pitch: f32,
        wall: bool,
    ) -> bool {
        if a.ink < ctx.weapons.mitts.leap_ink_min {
            return false;
        }
        it.mitts_req = Some(MittsReq {
            want: c,
            yaw,
            pitch,
            wall,
            drop: false,
        });
        it.jump = true;
        true
    }

    /// JS `dropCling(r, k, 'jump')` from the bot hooks (L776 / L822): a
    /// one-frame drop request on the jump hook.
    fn mitts_drop(&self, it: &mut ActorInput) {
        it.mitts_req = Some(MittsReq {
            drop: true,
            ..Default::default()
        });
        it.jump = true;
    }

    /// JS `botFindWall(a, target)` L708-722: a wall near the fight worth
    /// sticking to — probe round the kid at chest and head height (one rng
    /// draw per probe, like JS).
    fn bot_find_wall(&mut self, a: &Actor, ctx: &BotCtx, t: &Actor) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        let mut best_s = f32::INFINITY;
        for i in 0..10 {
            let yaw = (i as f32 / 10.0) * std::f32::consts::TAU + self.rng.next_f32() * 0.3;
            let dir = vec3(yaw.sin(), 0.0, yaw.cos());
            let h1 =
                ctx.world
                    .collision
                    .raycast(vec3(a.pos.x, a.pos.y + 1.2, a.pos.z), dir, 9.0, false);
            if !h1.hit || h1.dist < 3.5 || h1.normal.y.abs() > 0.3 {
                continue;
            }
            let h2 =
                ctx.world
                    .collision
                    .raycast(vec3(a.pos.x, a.pos.y + 2.8, a.pos.z), dir, 9.5, false);
            if !h2.hit || (h2.dist - h1.dist).abs() > 0.6 {
                continue; // needs height above the cling point (JS L717)
            }
            let s = (h1.point.x - t.pos.x).hypot(h1.point.z - t.pos.z);
            if s < 7.0 && s < best_s {
                best_s = s;
                best = Some((yaw, h1.dist));
            }
        }
        best
    }

    /// JS `botTactics` (kits/mitts.js L739-757): the weaving approach from
    /// 5-14 m — swim an own-ink lane, else punch the way in. Runs every
    /// fight frame after the core's defaults (bots.js L528).
    #[allow(clippy::too_many_arguments)]
    fn mitts_tactics(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        dist: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        ti: usize,
        visible: bool,
        wr: &WeaponRunner,
    ) {
        let m = &ctx.weapons.mitts;
        // JS L741: `!w.botApproach || ctx.retreat || !t || !t.pos ||
        // !ctx.visible` — the retreat / target guards are held by the caller
        if m.bot_approach <= 0.0 || !visible {
            return;
        }
        if wr
            .mitts
            .as_ref()
            .is_some_and(|k| k.charging || k.leaping || k.cling)
        {
            return;
        }
        if dist <= m.range * 1.05 || dist > 14.0 {
            return;
        }
        self.mw_weave_t += dt;
        // JS writes the literal `Math.random() * 6.28` (mitts.js L746), not
        // Math.PI * 2 — kept byte-faithful
        #[allow(clippy::approx_constant)]
        let p = *self
            .mw_weave_p
            .get_or_insert_with(|| self.rng.next_f32() * 6.28);
        let t = &ctx.actors[ti];
        let nx = (t.pos.x - a.pos.x) / dist;
        let nz = (t.pos.z - a.pos.z) / dist;
        // heading: straight at them when the way is open, else the nav
        // path's; plus the weave (JS L748-754)
        let (mut ux, mut uz) = (nx, nz);
        let ml = move_v.length();
        if dist > 7.0 && ml > 0.1 {
            ux = move_v.x / ml;
            uz = move_v.z / ml;
        }
        let sway = (self.mw_weave_t * 4.2 + p).sin() * 0.6;
        let mx = ux - uz * sway;
        let mz = uz + ux * sway;
        let l = mx.hypot(mz);
        let l = if l == 0.0 { 1.0 } else { l };
        *move_v = Vec3::new(mx / l, 0.0, mz / l);
        if a.ground_team == 1 && self.own_lane(a, ctx, nx, nz, (dist - m.range).min(5.0)) {
            it.squid = true;
            it.fire = false;
            return;
        }
        if a.ink > m.ink_per_punch * 6.0 {
            it.squid = false;
            it.fire = true;
        }
    }

    /// JS `ownLane` (kits/mitts.js L725-732): own ink along (nx, nz) for
    /// `d` m — a lane to swim in on.
    fn own_lane(&self, a: &Actor, ctx: &BotCtx, nx: f32, nz: f32, d: f32) -> bool {
        for k in [1.2f32, 2.6, 4.2] {
            if k > 1.2 && k > d - 0.6 {
                break;
            }
            let st = ctx.paint.region_stats(
                ctx.world.collision,
                a.pos.x + nx * k,
                a.pos.y,
                a.pos.z + nz * k,
                0.8,
                a.team,
            );
            if st.n == 0 || st.own < 0.5 {
                return false;
            }
        }
        true
    }

    /// JS `MAIN_KITS.blade.bot.stayIn` (kits/blade.js L710-714): low on
    /// health in a fight at blade range (or charged and nearly there):
    /// finish it rather than turn and run.
    fn blade_stay_in(&self, a: &Actor, ctx: &BotCtx, wr: &WeaponRunner) -> bool {
        let Some(ti) = self
            .target
            .and_then(|(t, sl)| find_actor(ctx.actors, t, sl))
        else {
            return false;
        };
        let t = &ctx.actors[ti];
        let d = (t.pos.x - a.pos.x).hypot(t.pos.z - a.pos.z);
        let full = wr.blade.as_ref().is_some_and(|k| k.full);
        (t.pos.y - a.pos.y).abs() < 1.4 && (d < 4.5 || (full && d < 5.5))
    }

    /// JS `MAIN_KITS.blade.bot.targetBias` (kits/blade.js L718-725), added
    /// to the perception score (bots.js L1650). Teammate target memory comes
    /// from [`BotCtx::mate_minds`]; `None` (a remote brain the replicator
    /// does not own) leaves only the low-hp term (module docs).
    fn blade_target_bias(&self, a: &Actor, ctx: &BotCtx, ei: usize, d: f32) -> f32 {
        let e = &ctx.actors[ei];
        let mut b = if e.hp <= 50.0 && d < 12.0 { -2.0 } else { 0.0 };
        let Some(Some(m)) = ctx.mate_minds.get(ei) else {
            return b;
        };
        let own_kind = ctx.own(self.team, self.slot).0;
        if let Some((tt, ts)) = m.target {
            if (tt, ts) != (a.team, a.slot) && tt == a.team && m.see_timer > 0.0 && d < 14.0 {
                b -= 4.0;
            } else if (tt, ts) == (a.team, a.slot)
                && m.see_timer > 0.0
                && d > ctx.weapons.blade.range
                // JS `!MELEE[e.weapon.kind]` (blade.js L724): MELEE.blade is
                // patched true (blade.js L35), so this arm is dead code for a
                // Cutlass owner — kept faithful to the JS runtime table
                && !own_kind.is_melee()
            {
                b += 4.0;
            }
        }
        b
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

/// JS `sub.throwSpeed || 13.5` (bots.js L1254): the tuning table's lob speed
/// for a thrown sub. Only the [`SubKind::is_thrown`] kinds ever reach it (the
/// `zone_bomb_aim` gate); the rest fall through to the JS default.
fn sub_throw_speed(sub: SubKind) -> f32 {
    let s = Bot::sub_set();
    let v = match sub {
        SubKind::Bomb => s.bomb.throw_speed,
        SubKind::Sticky => s.sticky.throw_speed,
        SubKind::Burst => s.burst.throw_speed,
        SubKind::Seeker => s.seeker.throw_speed,
        SubKind::Mist => s.mist.throw_speed,
        SubKind::Shaker => s.shaker.throw_speed,
        SubKind::Waddle => s.waddle.throw_speed,
        SubKind::Torpedo => s.torpedo.throw_speed,
        SubKind::Boomerang => s.boomerang.throw_speed,
        _ => 0.0,
    };
    if v == 0.0 { 13.5 } else { v }
}

/// JS L557 paint aim-pitch ladder + the kit `paintPitch` hooks (brolly.js
/// L559, blade.js L707, bow.js L452, mitts.js L760; splatling / slosher
/// retired).
fn paint_pitch(kind: WeaponKind) -> f32 {
    match kind {
        WeaponKind::Charger => -0.12,
        WeaponKind::Spinner => -0.16,
        WeaponKind::Blaster => -0.28,
        WeaponKind::Bucket => -0.5,
        WeaponKind::Brolly => -0.36,
        WeaponKind::Blade => -0.2,
        WeaponKind::Bow => -0.18,
        WeaponKind::Mitts => -0.32,
        _ => PAINT_PITCH,
    }
}

pub mod blade_tactics;
pub mod boss;
pub mod threat;
pub mod zones;

#[cfg(test)]
mod tests;
