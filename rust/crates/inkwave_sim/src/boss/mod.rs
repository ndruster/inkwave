//! HULLBREAKER — the boss entity (Task 42 batch C), a line-for-line port of
//! `src/boss/boss.js`.
//!
//! Owns the replicated state (pos, yaw, hp, phase, current move, stun,
//! crablets …) and the damage the squad deals it. One client simulates it
//! (offline: you; online: the host) through `BossBrain` — **Task 43**;
//! everyone else follows the host's 20 Hz snapshots through [`Boss::follow`]
//! on the playback clock, exactly like the JS guest path.
//!
//! What the sim layer deliberately does NOT own (the JS class touches them
//! through `G.*` render globals):
//! - the model / fx / scene (`BossModel`, `G.fx`, `G.scene`, `emit`): the
//!   state the renderer needs is exposed as [`Boss::st`] plus the
//!   [`BossEvent`] queue;
//! - `BossHazards` (`hz`) and `BossBrain` (`brain`) — Task 43;
//! - the actor list (`_pushActors`, `targetActor`) and the
//!   `special:slam` event subscription (boss.js L68-71) — Task 44 wiring;
//! - paint: [`BossEvent::Splat`] is recorded instead of calling
//!   `PaintGrid::splat` (the boss has no paint handle; the match layer owns
//!   it).
//!
//! Determinism: every floating-point expression keeps the JS operation order
//! (`// JS src/boss/boss.js L<line>` anchors below), `Math.round` is
//! reproduced as `floor(x + 0.5)` (ties toward +∞), the snapshot quantisers
//! `r2`/`r3` are the JS originals (L496-497), and every random draw goes
//! through the seeded [`crate::weapon::Rng`] stream.

use std::collections::{HashMap, HashSet};

use glam::{Vec3, vec3};

use crate::actor::SimWorld;
use crate::match_::Phase;
use crate::net::wire::{self, JVal};
use crate::net::{BossCrab, BossSnap, DirectMsg};
use crate::tuning;
use crate::weapon::Rng;

use self::nav::BossNav;

pub mod nav;

// JS src/boss/boss.js L496-497: `Math.round(x * 100) / 100` etc., in f64.
#[inline]
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

#[inline]
fn r2(x: f32) -> f32 {
    (js_round(f64::from(x) * 100.0) / 100.0) as f32
}

#[inline]
fn r3(x: f32) -> f32 {
    (js_round(f64::from(x) * 1000.0) / 1000.0) as f32
}

/// JS `clamp` (core/ctx.js L27): `v < a ? a : v > b ? b : v`.
#[inline]
fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// The squidkid dealing damage, reduced to what the boss needs (JS passes
/// the whole actor; `nid` is its roster id, boss.js L364 `attacker.nid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BossAttacker {
    /// Roster id (`nid ?? slot`, JS L364/L379).
    pub nid: i32,
    /// JS `attacker.remote` — ghosts / replayed actors never hurt the boss
    /// (L289/L305/L324).
    pub is_remote: bool,
    /// JS `attacker.isLocal` — carried into the `boss:hit` events.
    pub is_local: bool,
    /// JS `attacker.nid !== undefined` — `sendBossHit` drops the message for
    /// attackers without a roster id (netmatch.js L152), while the local
    /// flash/predict event still fires (boss.js L297-298).
    pub has_nid: bool,
}

/// What a connected shot hit (JS `target`: `null` · a `hitShapes` entry · a
/// crablet, boss.js L288-291).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    /// No shape (rain / splash relay).
    None,
    /// Index into `tuning::Boss::hit`.
    Shape(usize),
    /// Crablet id.
    Crab(u32),
}

/// `segHit` result — the JS `_hs` object, returned by value instead of
/// reused (boss.js L205-250).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SegHit {
    /// A hit sphere; `idx` indexes `tuning::Boss::hit`.
    Shape {
        /// Segment parameter of the entry point (JS `_hs.t`).
        t: f64,
        idx: usize,
        weak: bool,
        point: Vec3,
        dist: f64,
    },
    /// A crablet.
    Crab {
        t: f64,
        id: u32,
        point: Vec3,
        dist: f64,
    },
}

/// `rollHit` result (JS `this._roll`, boss.js L252-269).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RollHit {
    /// What the drum reached.
    pub target: RollTarget,
    pub point: Vec3,
    /// `false` when a crablet was hit (JS `out.boss`).
    pub boss: bool,
}

/// [`RollHit::target`] variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollTarget {
    /// Index into `tuning::Boss::hit`.
    Shape(usize),
    Crab(u32),
}

/// A guest's point-to-point boss hit awaiting transport (JS
/// `G.netm.sendBossHit`, boss.js L296/L374). The match layer drains these
/// into `DirectMsg::BossHit` frames.
///
/// One sim-side event the boss raises for the layer above (JS `emit(...)` —
/// the sim has no event bus). Damage numbers already carry the weak / stun /
/// weapon multipliers where JS applied them.
#[derive(Debug, Clone, PartialEq)]
pub enum BossEvent {
    /// `recBoss(['bc', …])` (boss.js L386): a crablet popped; `r2` coords.
    Bc {
        id: u32,
        x: f32,
        y: f32,
        z: f32,
        killed: bool,
    },
    /// A new move started playing (JS `boss:move` per phase, L162-170 — the
    /// per-phase split needs `movePhaseAt`, Task 43; one event per move).
    Move(JVal),
    /// `boss:hit` (L294/L298/L346/L375/L379), including the guest's
    /// `predicted` / `blocked` variants and the host-counted flash the guest
    /// replays at L455-456 (zero `damage`).
    Hit {
        damage: f32,
        weak: bool,
        /// `nid` of the attacker, `-1` when none.
        attacker: i32,
        local: bool,
        blocked: bool,
        predicted: bool,
        crab: bool,
        pos: Option<Vec3>,
    },
    /// `boss:crablet` phase `'pop'` (L395): the burst itself (JS emits at
    /// `y + 0.3`; the raw crab position is kept).
    CrabBurst {
        id: u32,
        x: f32,
        y: f32,
        z: f32,
        killed: bool,
    },
    /// The host's crablet-burst paint (L400 `G.paint.splat`) — recorded,
    /// not applied (the match layer owns the [`crate::paint::PaintGrid`]).
    Splat {
        pos: Vec3,
        radius: f32,
        team: usize,
        /// Seeded from the boss rng (JS `Math.random()`).
        seed: f32,
    },
    /// `boss:phase` (L175).
    Phase { phase: u32 },
    /// `boss:defeat` (L181); `killer` is the JS `boss:defeat` `by` nid.
    Defeat { killer: i32 },
    /// `boss:crablet` phase `'spawn'` (L410); JS emits at `y + 0.3`.
    CrabSpawn { id: u32, x: f32, y: f32, z: f32 },
    /// `boss:intro` (L160).
    Intro,
    /// `boss:stun` (L173; the `dur` needs the move tables — Task 43).
    Stunned,
    /// `boss:hp` (L176).
    Hp { hp: f32 },
}

/// One crablet (JS `boss.crabs` Map value, boss.js L47/L465).
#[derive(Debug, Clone)]
pub struct CrabState {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub hp: f32,
    pub dead: bool,
    /// JS `c.speed` — smoothed in `_follow` (L468); `None` ≡ JS
    /// `undefined` (first sample sets it to 0).
    pub speed: Option<f32>,
    /// JS `c.missT` (L471): seconds absent from the snapshots.
    pub miss_t: f32,
    /// JS `c.popT` (L416): seconds since the pop started.
    pub pop_t: f32,
    /// JS `c.model` existing — the visuals have seen this crab at least once
    /// (gates the pop fx, L404, and the spawn event, L409).
    pub spawned: bool,
    /// JS `c.popped` (L404).
    pub popped: bool,
}

/// Per-attacker boss bookkeeping (JS `attacker.stats.bossDmg` / `weakHits` /
/// `splats`, boss.js L341/L380 — kept on the boss instead of on the actors).
#[derive(Debug, Clone, Copy, Default)]
pub struct AttackerStats {
    pub boss_dmg: f32,
    pub weak_hits: u32,
    pub splats: u32,
}

/// The animation state the model consumes (JS `this.st`, boss.js L62; the
/// renderer reads it every frame).
#[derive(Debug, Clone)]
pub struct BossSt {
    pub speed: f32,
    pub turn: f32,
    /// `'intro' | 'roar' | <move id> | None` (JS `st.move`).
    pub move_id: Option<String>,
    pub move_t: f32,
    /// `'tele' | 'act' | 'rec' | None` (JS `st.movePhase`).
    pub move_phase: Option<String>,
    pub phase_dur: f32,
    pub phase: u32,
    pub stunned: bool,
    pub hurt: f32,
    pub dead: bool,
    pub aim: Option<Vec3>,
}

impl Default for BossSt {
    fn default() -> Self {
        Self {
            speed: 0.0,
            turn: 0.0,
            move_id: None,
            move_t: 0.0,
            move_phase: None,
            phase_dur: 0.0,
            // JS L62: `phase: 1`.
            phase: 1,
            stunned: false,
            hurt: 0.0,
            dead: false,
            aim: None,
        }
    }
}

/// Snapshot playback bookkeeping (JS `this.net`, boss.js L64, plus the
/// per-frame `_follow` scratch at L455-460).
#[derive(Debug, Default)]
pub struct BossNet {
    /// Buffered host snapshots awaiting the playback clock (JS `buf`).
    pub buf: Vec<(f64, BossSnap)>,
    /// Playback position error (JS `err`).
    pub err: Vec3,
    /// Error velocity (JS `errV`).
    pub err_v: Vec3,
    /// Last applied sample (JS `prev`).
    pub prev: Option<BossSnap>,
    pub ready: bool,
    pub handoff: bool,
    /// This frame's sampled state (`netmatch._sampleBoss` fills it; JS
    /// `this.net.cur`).
    pub cur: Option<BossSnap>,
    /// Previous snapshot flash count (JS `n.fl`, L455).
    pub fl: Option<u32>,
    /// Previous snapshot weak count (JS `n.wk`, L456).
    pub wk: Option<u32>,
    /// Crablet ids in the current sample (JS `n.crabSeen`, L460).
    pub crab_seen: HashSet<u32>,
}

/// Netcode audit (JS `this.log`, boss.js L48). `moves` is the brain's
/// (Task 43); `sent`/`sentDmg` are bumped by the guest uplink
/// `sendBossHit` (netmatch.js L153 — raw `d`, not the `r2(d)` sent on the
/// wire).
#[derive(Debug, Clone, Copy, Default)]
pub struct BossLog {
    pub recv: u32,
    pub recv_dmg: f32,
    pub sent: u32,
    pub sent_dmg: f32,
}

/// A guest's delivered hit (JS `d` in `remoteHit`, boss.js L352-359).
#[derive(Debug, Clone, Copy)]
pub struct RemoteHit {
    /// Attacker `nid`.
    pub a: i32,
    pub d: f32,
    pub weak: bool,
    /// Crablet id, `-1` for boss hits.
    pub c: i32,
}

/// `this._ev` (boss.js L65): the edge-detect bookkeeping for the event queue.
#[derive(Debug, Clone)]
struct EvState {
    /// JS `_ev.key` — the displayed move's `t0` (the phase suffix needs
    /// `movePhaseAt`, Task 43).
    key_t0: Option<f64>,
    phase: u32,
    stun: bool,
    dead: bool,
    anim: u32,
    hp: f32,
    intro: bool,
}

/// Constructor options (JS `new Boss(match, { maxHp, difficulty, sim })`,
/// boss.js L31; `seed` replaces the host's `Math.random()` draws).
#[derive(Debug, Clone)]
pub struct BossConfig<'d> {
    pub max_hp: f32,
    pub difficulty: &'d str,
    pub sim: bool,
    pub seed: u64,
}

/// HULLBREAKER (JS `class Boss`, boss.js L30-494).
pub struct Boss<'a> {
    /// JS `match.state` (Task 44 drives it; `'playing'` ≡ [`Phase::Active`],
    /// `'judge'`/`'results'` both map onto [`Phase::End`]).
    pub state: Phase,
    /// JS `this.sim` — this client runs the boss (host / offline).
    pub sim: bool,
    pub difficulty: String,
    /// JS `this.dmgMul` (L36): `dmgDiff[difficulty] ?? 1`.
    pub dmg_mul: f32,
    /// Boss clock, seconds since the match began (JS `this.bt`, L39).
    pub bt: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub turn: f32,
    pub max_hp: f32,
    pub hp: f32,
    pub phase: u32,
    /// Phase the brain wants to enter (JS `brain.pendingPhase`,
    /// bossBrain.js L44 — hoisted onto the entity; promotion is Task 43).
    pub pending_phase: u32,
    pub dead: bool,
    /// `nid` of the killer, `-1` while alive (JS L42/L364).
    pub killer: i32,
    pub stunned: bool,
    pub invuln: bool,
    pub visible: bool,
    /// Set pieces: 0 none, 1 intro, 2 roar (JS `this.anim`, L43).
    pub anim: u32,
    pub anim_t0: f32,
    /// The move record being played (JS `this.move`, L44; `move` is a Rust
    /// keyword — same rename as [`BossSnap::move_rec`]).
    pub move_rec: Option<JVal>,
    /// Move records seen so far, insertion order (JS `hz.moves` — the host's
    /// timeline, Task 43 grows it).
    pub moves: Vec<JVal>,
    /// `nid` focused on (JS `this.target`, L45; `targetActor` is Task 44).
    pub target: i32,
    pub flash_n: u32,
    pub weak_n: u32,
    /// Big-hit intensity consumed by the model each frame (JS L46/L124).
    pub hurt: f32,
    /// Crablets, insertion order (JS `this.crabs` Map, L47).
    pub crabs: Vec<CrabState>,
    /// Next crablet id (JS `crabSeq++`, bossBrain.js L336).
    pub crab_seq: u32,
    /// Home-ground centre (JS `this.home`, L92).
    pub home: Option<[f32; 2]>,
    pub log: BossLog,
    /// Per-attacker bookkeeping keyed by `nid` (JS `attacker.stats`, L341).
    pub stats_by_nid: HashMap<i32, AttackerStats>,
    /// The animation state for the renderer (JS `this.st`, L62).
    pub st: BossSt,
    pub net: BossNet,
    /// Queued `emit(...)` calls (JS event bus; drain with
    /// [`Boss::drain_events`]).
    pub events: Vec<BossEvent>,
    /// `sendBossHit` messages a guest queued for the host (L296/L374).
    pub guest_hits: Vec<DirectMsg>,
    /// Navigation grid (JS `this.nav`, L37 — shared, built once per level).
    pub nav: &'a BossNav,
    /// The `boss` tuning section (JS `BOSS` / `HZ` / `FLOOR_BODY` tables).
    pub tb: &'a tuning::Boss,
    pub world: &'a SimWorld<'a>,
    /// Seeded rng replacing the host's `Math.random()` draws (L400 splat
    /// seed; Task 43 brain rolls).
    rng: Rng,
    /// JS `this._rain` Map (L67) — insertion order preserved, keyed by
    /// `nid`.
    rain: Vec<(BossAttacker, f32)>,
    /// JS `this._rainT` (L329).
    rain_t: f64,
    /// JS `this._paintBt` (L66).
    paint_bt: f32,
    ev: EvState,
}

impl<'a> Boss<'a> {
    /// JS constructor (boss.js L31-72), minus the render objects (model /
    /// hazards / brain / scene / event subscriptions — see the module docs).
    pub fn new(
        world: &'a SimWorld<'a>,
        nav: &'a BossNav,
        tb: &'a tuning::Boss,
        cfg: BossConfig<'_>,
    ) -> Self {
        // JS L36: `BOSS.dmgDiff[difficulty] ?? 1`.
        let dmg_mul = match cfg.difficulty {
            "easy" => tb.boss.dmg_diff.easy,
            "normal" => tb.boss.dmg_diff.normal,
            "hard" => tb.boss.dmg_diff.hard,
            _ => 1.0,
        };
        let mut me = Self {
            // JS `match.state` starts `'init'` (L101).
            state: Phase::Init,
            sim: cfg.sim,
            difficulty: cfg.difficulty.to_string(),
            dmg_mul,
            bt: 0.0,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            yaw: 0.0,
            turn: 0.0,
            max_hp: cfg.max_hp,
            hp: cfg.max_hp,
            phase: 1,
            pending_phase: 1,
            dead: false,
            killer: -1,
            stunned: false,
            invuln: false,
            visible: false,
            anim: 0,
            anim_t0: 0.0,
            move_rec: None,
            moves: Vec::new(),
            target: -1,
            flash_n: 0,
            weak_n: 0,
            hurt: 0.0,
            crabs: Vec::new(),
            crab_seq: 0,
            home: None,
            log: BossLog::default(),
            stats_by_nid: HashMap::new(),
            st: BossSt::default(),
            net: BossNet::default(),
            events: Vec::new(),
            guest_hits: Vec::new(),
            nav,
            tb,
            world,
            rng: Rng::new(cfg.seed),
            rain: Vec::new(),
            rain_t: 0.0,
            paint_bt: 0.0,
            // JS L65 `_ev`.
            ev: EvState {
                key_t0: None,
                phase: 1,
                stun: false,
                dead: false,
                anim: 0,
                hp: -1.0,
                intro: false,
            },
        };
        me.spawn_point(); // JS L49
        me
    }

    /// Spawn: the roomiest floor near the middle of its home ground, facing
    /// down the stage toward the squad (JS `_spawnPoint`, L75-93).
    fn spawn_point(&mut self) {
        let nav = self.nav;
        let ids = nav.plan_ids();
        // JS L77-79: centroid of the plan region (f64).
        let mut sx = 0.0f64;
        let mut sz = 0.0f64;
        for &i in ids {
            let (x, z) = nav.xz(i);
            sx += f64::from(x);
            sz += f64::from(z);
        }
        let n = ids.len().max(1) as f64; // JS L79 `Math.max(1, length)`
        let cx = sx / n;
        let cz = sz / n;
        // JS L80-85: score each plan cell.
        let mut best = -1i32;
        let mut bs = f64::NEG_INFINITY;
        for &i in ids {
            let (x, z) = nav.xz(i);
            let s = f64::from(nav.dw_at(i)).min(6.0) * 1.5 + f64::from(nav.dv_at(i)).min(5.0)
                - (f64::from(x) - cx).hypot(f64::from(z) - cz) * 0.25;
            if s > bs {
                bs = s;
                best = i;
            }
        }
        // JS L86-89.
        let (x, z) = if best >= 0 { nav.xz(best) } else { (0.0, 0.0) };
        let (pad_x, pad_z) = nav.pad_center(0);
        self.pos = vec3(x, nav.floor_at(x, z), z);
        let mut yaw = (f64::from(pad_x) - f64::from(x)).atan2(f64::from(pad_z) - f64::from(z));
        // JS L91: the facing may not fit here (a pier): take the nearest
        // heading that does.
        for k in 0..24 {
            if nav.turn_ok(x, z, yaw as f32) {
                break;
            }
            yaw += (if k % 2 == 1 { 1.0 } else { -1.0 }) * (k as f64) * 0.13;
        }
        self.yaw = yaw as f32;
        self.home = Some([x, z]);
    }

    /// JS L95.
    #[must_use]
    pub fn hp_frac(&self) -> f64 {
        if self.max_hp > 0.0 {
            f64::from(self.hp) / f64::from(self.max_hp)
        } else {
            0.0
        }
    }

    /// JS L97 `live()`: `'playing'` ≡ [`Phase::Active`].
    #[must_use]
    pub fn live(&self) -> bool {
        self.state == Phase::Active && !self.dead
    }

    /// JS `BossBrain.checkPhase` (bossBrain.js L41-45), hoisted onto the
    /// entity: the phase the HP fraction wants, recorded in
    /// [`Boss::pending_phase`] (the brain promotes it at move boundaries —
    /// Task 43).
    pub fn check_phase(&mut self) {
        let f = self.hp_frac();
        let want: u32 = if f <= f64::from(self.tb.boss.phases[1]) {
            3
        } else if f <= f64::from(self.tb.boss.phases[0]) {
            2
        } else {
            1
        };
        if want > self.phase && want > self.pending_phase {
            self.pending_phase = want;
        }
    }

    /// Is hit sphere `i` hittable this frame? The JS rules are the model's
    /// (bossModel.js L106-108): `belly` opens on stun / phase 3 (its
    /// animation-weight term needs the pose — Task 43), the `crack` sockets
    /// open only while the shell is torn (Task 43), and the `below` term
    /// (the boss dipping under the deck) is always false in the sim.
    #[must_use]
    pub fn shape_active(&self, i: usize) -> bool {
        if self.dead {
            return false;
        }
        match self.tb.hit[i].socket.as_str() {
            "belly" => self.stunned || self.phase >= 3,
            "crackL" | "crackR" => false,
            _ => true,
        }
    }

    /// Hit sphere `i` as `(world centre, radius, weak, active)`. JS reads
    /// `model.hitShapes[i].pos` — the socket world point from `getSocket`
    /// (bossModel.js L105); the rest bone chain is pure translation, so
    /// `world = pos + Ry(yaw) · rest` with THREE's `rotation.y`
    /// (x' = cos·x + sin·z, z' = −sin·x + cos·z; boss.js L57/L122). THREE
    /// `Vector3` stores f32, matching [`Self::pos`].
    #[must_use]
    pub fn hit_sphere(&self, i: usize) -> (Vec3, f32, bool, bool) {
        let h = &self.tb.hit[i];
        let (rx, ry, rz) = (
            f64::from(h.pos[0]),
            f64::from(h.pos[1]),
            f64::from(h.pos[2]),
        );
        let (s, c) = f64::from(self.yaw).sin_cos();
        let p = vec3(
            (f64::from(self.pos.x) + c * rx + s * rz) as f32,
            (f64::from(self.pos.y) + ry) as f32,
            (f64::from(self.pos.z) - s * rx + c * rz) as f32,
        );
        (p, h.r, h.weak, self.shape_active(i))
    }

    // ------------------------------------------------------------------ per frame
    /// JS `update(dt)` (L100-116).
    pub fn update(&mut self, dt: f32) {
        // JS L101: loading — the clock starts with the intro (root.visible
        // is render-side).
        if self.state == Phase::Init {
            return;
        }
        let _bt0 = self.bt; // JS L102 (consumed by `hz.paint`, Task 43)
        if self.sim {
            // JS L103: `this.brain.update(dt)` — BossBrain is Task 43.
        } else {
            self.follow(dt);
        }
        let _live = self.live() && self.visible; // JS L105
        // JS L106-110: hazard fizzle / paint slice / `hz.update` — Task 43.
        self.paint_bt = self.bt; // JS L109
        self.emit_events(); // JS L111
        self.crab_visuals(dt); // JS L112
        // JS L113: `_pushActors` — needs the actor list (Task 44).
        self.model_update(dt); // JS L114
        self.flush_rain(dt); // JS L115
    }

    /// JS `_modelUpdate(dt)` (L119-140): the animation state, from state
    /// alone — identical everywhere. The root transform / `model.update`
    /// calls are render-side; the sim keeps [`Boss::st`].
    fn model_update(&mut self, _dt: f32) {
        let st = &mut self.st;
        st.speed = f64::from(self.vel.x).hypot(f64::from(self.vel.z)) as f32; // JS L123
        st.turn = self.turn;
        st.phase = self.phase;
        st.stunned = self.stunned;
        st.dead = self.dead;
        st.hurt = self.hurt; // JS L124
        self.hurt = 0.0;
        st.aim = None; // JS L125
        if self.anim != 0 {
            // JS L126-127: set pieces.
            st.move_id = Some(if self.anim == 1 { "intro" } else { "roar" }.to_string());
            st.move_phase = Some("act".to_string());
            st.move_t = self.bt - self.anim_t0;
            st.phase_dur = if self.anim == 1 {
                self.tb.boss.intro_len
            } else {
                self.tb.boss.roar_len
            };
        } else {
            // JS L128-133: move playback needs `movePhaseAt` / `moveTimes`
            // (bossHazards — Task 43); L135-138: the target aim needs the
            // actor list (Task 44). Idle path for now.
            st.move_id = None;
            st.move_phase = None;
            st.move_t = 0.0;
            st.phase_dur = 0.0;
        }
    }

    /// JS `_events()` (L156-184): the UI / audio queue, raised from the
    /// displayed state on every client.
    fn emit_events(&mut self) {
        // JS L158-161: intro.
        if self.anim != self.ev.anim {
            self.ev.anim = self.anim;
            if self.anim == 1 && !self.ev.intro {
                self.ev.intro = true;
                self.events.push(BossEvent::Intro);
            }
        }
        // JS L162-170: `boss:move` fires per phase (`m.t0 + ph`); the phase
        // split needs `movePhaseAt` (Task 43), so the port raises one event
        // when the displayed move changes.
        let cur_t0 = if self.dead {
            None
        } else {
            self.move_rec
                .as_ref()
                .and_then(|m| m.field("t0"))
                .and_then(JVal::as_f64)
        };
        if cur_t0 != self.ev.key_t0 {
            self.ev.key_t0 = cur_t0;
            if let Some(m) = self.move_rec.as_ref().filter(|_| cur_t0.is_some()) {
                self.events.push(BossEvent::Move(m.clone()));
            }
        }
        // JS L171-174: stun (the `dur` needs the move tables — Task 43).
        if self.stunned != self.ev.stun {
            self.ev.stun = self.stunned;
            if self.stunned {
                self.events.push(BossEvent::Stunned);
            }
        }
        // JS L175: phase (`model.setPhase` is render-side).
        if self.phase != self.ev.phase {
            self.ev.phase = self.phase;
            self.events.push(BossEvent::Phase { phase: self.phase });
        }
        // JS L176: hp.
        if (f64::from(self.hp) - f64::from(self.ev.hp)).abs() > 0.01 {
            self.ev.hp = self.hp;
            self.events.push(BossEvent::Hp { hp: self.hp });
        }
        // JS L177-183: defeat — every live crablet pops.
        if self.dead && !self.ev.dead {
            self.ev.dead = true;
            for idx in 0..self.crabs.len() {
                if !self.crabs[idx].dead {
                    self.crabs[idx].dead = true;
                    self.crab_pop_fx(idx);
                }
            }
            self.events.push(BossEvent::Defeat {
                killer: self.killer,
            });
        }
    }

    /// JS `_crabVisuals(dt)` (L407-418): the model lifecycle — the sim keeps
    /// the spawn event and the pop-and-forget timer (positioning /
    /// `model.update` / dispose are render-side).
    fn crab_visuals(&mut self, dt: f32) {
        for c in &mut self.crabs {
            if !c.spawned {
                // JS L410: `emit('boss:crablet', { phase: 'spawn', pos:
                // (c.x, c.y + 0.3, c.z) })`.
                c.spawned = true;
                self.events.push(BossEvent::CrabSpawn {
                    id: c.id,
                    x: c.x,
                    y: c.y + 0.3,
                    z: c.z,
                });
            }
            if c.dead {
                c.pop_t += dt; // JS L416
            }
        }
        // JS L416: `popT > 1.2` → dispose + delete.
        self.crabs.retain(|c| !(c.dead && c.pop_t > 1.2));
    }

    // ------------------------------------------------------------- damage intake
    /// JS `segHit(a, b, pad = 0)` (L205-231): nearest hit shape (or crablet)
    /// along the segment a→b, within `pad` of its surface.
    #[must_use]
    pub fn seg_hit(&self, a: Vec3, b: Vec3, pad: f32) -> Option<SegHit> {
        // JS L206.
        if !self.visible || self.dead {
            return self.seg_crabs(a, b, pad, None);
        }
        let (abx, aby, abz) = (
            f64::from(b.x) - f64::from(a.x),
            f64::from(b.y) - f64::from(a.y),
            f64::from(b.z) - f64::from(a.z),
        );
        // JS L208: `L2 = … || 1e-9` (0 and NaN both fall back).
        let l2_raw = abx * abx + aby * aby + abz * abz;
        let l2 = if l2_raw > 0.0 { l2_raw } else { 1e-9 };
        // JS L209-222: best sphere by entry key (weak spheres win ties by
        // 0.02).
        let mut best: Option<(f64, f64, usize, bool)> = None; // (key, te, idx, weak)
        for i in 0..self.tb.hit.len() {
            let (cp, hr, weak, active) = self.hit_sphere(i);
            if !active {
                continue;
            }
            let (cx, cy, cz) = (f64::from(cp.x), f64::from(cp.y), f64::from(cp.z));
            let mut t = ((cx - f64::from(a.x)) * abx
                + (cy - f64::from(a.y)) * aby
                + (cz - f64::from(a.z)) * abz)
                / l2;
            t = clamp(t, 0.0, 1.0);
            let px = f64::from(a.x) + abx * t - cx;
            let py = f64::from(a.y) + aby * t - cy;
            let pz = f64::from(a.z) + abz * t - cz;
            let r = f64::from(hr) + f64::from(pad);
            let d2 = px * px + py * py + pz * pz;
            if d2 > r * r {
                continue;
            }
            // JS L218-219: entry point along the segment.
            let back = ((r * r - d2).max(0.0) / l2).sqrt();
            let te = (t - back).max(0.0);
            // JS L221: weak spheres sit on the shell — the weak one wins only
            // if it is (nearly) in front.
            let key = te - if weak { 0.02 } else { 0.0 };
            let take = match best {
                None => true,
                Some((bk, _, _, _)) => key < bk,
            };
            if take {
                best = Some((key, te, i, weak));
            }
        }
        // JS L224-225: a crab in front of the sphere wins.
        let crab = self.seg_crabs(a, b, pad, best.map(|(_, te, _, _)| te));
        if crab.is_some() {
            return crab;
        }
        // JS L226-230 (`_hs`).
        let (_, te, idx, weak) = best?;
        Some(SegHit::Shape {
            t: te,
            idx,
            weak,
            point: vec3(
                (f64::from(a.x) + abx * te) as f32,
                (f64::from(a.y) + aby * te) as f32,
                (f64::from(a.z) + abz * te) as f32,
            ),
            dist: l2.sqrt() * te,
        })
    }

    /// JS `_segCrabs(a, b, pad, best)` (L232-250).
    fn seg_crabs(&self, a: Vec3, b: Vec3, pad: f32, best_t: Option<f64>) -> Option<SegHit> {
        let (abx, aby, abz) = (
            f64::from(b.x) - f64::from(a.x),
            f64::from(b.y) - f64::from(a.y),
            f64::from(b.z) - f64::from(a.z),
        );
        let l2_raw = abx * abx + aby * aby + abz * abz;
        let l2 = if l2_raw > 0.0 { l2_raw } else { 1e-9 };
        let mut hit: Option<(f64, u32)> = None;
        for c in &self.crabs {
            if c.dead {
                continue;
            }
            // JS L237: the aim point sits 0.35 above the crab's feet.
            let cy = f64::from(c.y) + 0.35;
            let mut t = ((f64::from(c.x) - f64::from(a.x)) * abx
                + (cy - f64::from(a.y)) * aby
                + (f64::from(c.z) - f64::from(a.z)) * abz)
                / l2;
            t = clamp(t, 0.0, 1.0);
            let px = f64::from(a.x) + abx * t - f64::from(c.x);
            let py = f64::from(a.y) + aby * t - cy;
            let pz = f64::from(a.z) + abz * t - f64::from(c.z);
            let r = 0.55 + f64::from(pad); // JS L240
            if px * px + py * py + pz * pz > r * r {
                continue;
            }
            // JS L242: only in front of the sphere hit / the current best.
            if (best_t.is_some() && t >= best_t.unwrap()) || (hit.is_some() && t >= hit.unwrap().0)
            {
                continue;
            }
            hit = Some((t, c.id));
        }
        let (ht, id) = hit?;
        Some(SegHit::Crab {
            t: ht,
            id,
            point: vec3(
                (f64::from(a.x) + abx * ht) as f32,
                (f64::from(a.y) + aby * ht) as f32,
                (f64::from(a.z) + abz * ht) as f32,
            ),
            dist: l2.sqrt() * ht,
        })
    }

    /// JS `rollHit(pos, fx, fz, width)` (L252-269): a roller's drum
    /// (`pos + forward · 0.8`) reaching the lowest shapes it can — claws,
    /// belly — or a crablet.
    #[must_use]
    pub fn roll_hit(&self, pos: Vec3, fx: f32, fz: f32, width: f32) -> Option<RollHit> {
        // JS L253.
        let cx = f64::from(pos.x) + f64::from(fx) * 0.8;
        let cz = f64::from(pos.z) + f64::from(fz) * 0.8;
        let cy = f64::from(pos.y) + 0.45;
        let r = f64::from(width) / 2.0 + 0.35;
        // JS L255-258: crablets first.
        for c in &self.crabs {
            if c.dead
                || (f64::from(c.y) - f64::from(pos.y)).abs() > 1.0
                || (f64::from(c.x) - cx).hypot(f64::from(c.z) - cz) > r + 0.4
            {
                continue;
            }
            return Some(RollHit {
                target: RollTarget::Crab(c.id),
                point: vec3(c.x, c.y + 0.35, c.z),
                boss: false,
            });
        }
        // JS L259.
        if !self.visible || self.dead {
            return None;
        }
        let mut best: Option<(f64, usize, Vec3, f32)> = None; // (d, idx, wpos, r)
        let mut bd = f64::INFINITY;
        for i in 0..self.tb.hit.len() {
            let (wp, hr, _weak, active) = self.hit_sphere(i);
            // JS L262: the low-layer filter — shapes sitting above the drum
            // can't be reached.
            if !active || f64::from(wp.y) - f64::from(hr) > cy + 0.7 {
                continue;
            }
            let d = (f64::from(wp.x) - cx).hypot(f64::from(wp.z) - cz) - f64::from(hr);
            if d < r && d < bd {
                bd = d;
                best = Some((d, i, wp, hr));
            }
        }
        let (_d, idx, wp, hr) = best?;
        // JS L267.
        Some(RollHit {
            target: RollTarget::Shape(idx),
            point: vec3(
                cx as f32,
                cy.max(f64::from(wp.y) - f64::from(hr) * 0.5) as f32,
                cz as f32,
            ),
            boss: true,
        })
    }

    /// JS `rayDist(o, d, max)` (L272-284): exact ray vs the hit spheres —
    /// distance or `-1`.
    #[must_use]
    pub fn ray_dist(&self, o: Vec3, d: Vec3, max: f32) -> f32 {
        if !self.visible || self.dead {
            return -1.0;
        }
        let mut best = -1.0f64;
        for i in 0..self.tb.hit.len() {
            let (wp, hr, _weak, active) = self.hit_sphere(i);
            if !active {
                continue;
            }
            let ox = f64::from(o.x) - f64::from(wp.x);
            let oy = f64::from(o.y) - f64::from(wp.y);
            let oz = f64::from(o.z) - f64::from(wp.z);
            let bq = ox * f64::from(d.x) + oy * f64::from(d.y) + oz * f64::from(d.z);
            let cq = ox * ox + oy * oy + oz * oz - f64::from(hr) * f64::from(hr);
            let disc = bq * bq - cq;
            if disc < 0.0 {
                continue;
            }
            let t = -bq - disc.sqrt();
            if t > 0.0 && t < f64::from(max) && (best < 0.0 || t < best) {
                best = t;
            }
        }
        best as f32
    }

    /// JS `hit(attacker, dmg, target, wid, point)` (L288-302): a shot / blob
    /// / beam connected. Ghosts never hurt; sim (host / offline) applies,
    /// guests queue the hit to the host and predict the marker.
    pub fn hit(
        &mut self,
        attacker: Option<BossAttacker>,
        dmg: f32,
        target: HitTarget,
        wid: &str,
        point: Option<Vec3>,
    ) {
        let Some(atk) = attacker else {
            return;
        };
        if atk.is_remote || dmg <= 0.0 || self.dead {
            return;
        }
        // JS L290: a crablet target (has `hp` and `id`).
        if let HitTarget::Crab(id) = target {
            self.hit_crab(atk, id, dmg, false);
            return;
        }
        // JS L291-292.
        let weak = matches!(target, HitTarget::Shape(i) if self.tb.hit[i].weak);
        let d = (f64::from(dmg)
            * if weak {
                f64::from(self.tb.boss.weak)
            } else {
                1.0
            }
            * if self.stunned {
                f64::from(self.tb.boss.stunned)
            } else {
                1.0
            }
            * self.weapon_mul(wid)) as f32;
        // JS L293: the fx burst at `point` — render-side.
        if self.invuln || !self.visible {
            // JS L294: the blocked event is emitted for local attackers only.
            if atk.is_local {
                self.events.push(BossEvent::Hit {
                    damage: 0.0,
                    weak,
                    attacker: atk.nid,
                    local: atk.is_local,
                    blocked: true,
                    predicted: false,
                    crab: false,
                    pos: point,
                });
            }
            return;
        }
        if !self.sim {
            // JS L295-300: guest — send the hit (dropped for nid-less
            // attackers, netmatch.js L152), flash locally, predict.
            if atk.has_nid {
                // JS netmatch.js L153: counters take the raw `d`; the wire
                // carries `r2(d)`.
                self.log.sent += 1;
                self.log.sent_dmg += d;
                self.guest_hits.push(DirectMsg::BossHit {
                    a: atk.nid as u8,
                    d: crate::net::wire::r2(f64::from(d)) as f32,
                    weak,
                    w: wid.to_string(),
                    c: -1,
                });
            }
            self.events.push(BossEvent::Hit {
                damage: d,
                weak,
                attacker: atk.nid,
                local: atk.is_local,
                blocked: false,
                predicted: true,
                crab: false,
                pos: point,
            });
            return;
        }
        self.apply_damage(Some(atk), d, weak, point); // JS L301
    }

    /// `BOSS.weapon[wid] ?? 1` (boss.js L22/L292): the area-weapon
    /// multiplier against the boss; unknown ids (`'storm'`, `'crab'`) → 1.
    #[must_use]
    pub fn weapon_mul(&self, wid: &str) -> f64 {
        let w = self.tb.boss.weapon;
        f64::from(match wid {
            "roller" => w.roller,
            "slam" => w.slam,
            "bomb" => w.bomb,
            "sticky" => w.sticky,
            "seeker" => w.seeker,
            "burst" => w.burst,
            "shaker" => w.shaker,
            "waddle" => w.waddle,
            "torpedo" => w.torpedo,
            _ => 1.0,
        })
    }

    /// JS `splash(attacker, c, R, dmax, dmin, wid)` (L304-321): splash
    /// damage — falloff from the nearest body sphere, cover blocks.
    pub fn splash(
        &mut self,
        attacker: Option<BossAttacker>,
        c: Vec3,
        r: f32,
        dmax: f32,
        dmin: f32,
        wid: &str,
    ) {
        let Some(atk) = attacker else {
            return;
        };
        if atk.is_remote || self.dead {
            return;
        }
        // JS L306-310: crablets inside `R + 0.4` take the full `dmax`.
        let ids: Vec<u32> = self
            .crabs
            .iter()
            .filter(|cr| !cr.dead)
            .filter(|cr| {
                (f64::from(cr.x) - f64::from(c.x)).hypot(
                    (f64::from(cr.y) + 0.3 - f64::from(c.y))
                        .hypot(f64::from(cr.z) - f64::from(c.z)),
                ) < f64::from(r) + 0.4
            })
            .map(|cr| cr.id)
            .collect();
        for id in ids {
            self.hit_crab(atk, id, dmax, false);
        }
        // JS L311.
        if !self.visible {
            return;
        }
        // JS L312-317: nearest non-weak active sphere surface distance.
        let mut best: Option<(f64, usize, Vec3, f32)> = None; // (bd, idx, wpos, r)
        for i in 0..self.tb.hit.len() {
            let (wp, hr, weak, active) = self.hit_sphere(i);
            if !active || weak {
                continue;
            }
            let dist = (f64::from(wp.x) - f64::from(c.x))
                .hypot((f64::from(wp.y) - f64::from(c.y)).hypot(f64::from(wp.z) - f64::from(c.z)));
            let d = (dist - f64::from(hr)).max(0.0);
            if d < best.map_or(f64::INFINITY, |b| b.0) {
                best = Some((d, i, wp, hr));
            }
        }
        let Some((bd, bi, bp, br)) = best else {
            return;
        };
        if bd > f64::from(r) {
            return;
        }
        // JS L319: cover blocks — `los(c raised 0.3, sphere lerped toward c)`.
        let a = vec3(c.x, c.y + 0.3, c.z);
        let dist = (f64::from(bp.x) - f64::from(c.x))
            .hypot((f64::from(bp.y) - f64::from(c.y)).hypot(f64::from(bp.z) - f64::from(c.z)));
        let t = (f64::from(br) / f64::from(br).max(dist)).min(0.9);
        let b = bp.lerp(c, t as f32); // THREE `Vector3.lerp` (f64 components;
        // computed in f32 here — sub-ulp)
        if !self.world.collision.los(a, b) {
            return;
        }
        // JS L320.
        let dmg = (f64::from(dmax)
            + (f64::from(dmin) - f64::from(dmax)) * clamp(bd / f64::from(r), 0.0, 1.0))
            as f32;
        self.hit(Some(atk), dmg, HitTarget::Shape(bi), wid, None);
    }

    /// JS `rain(owner, cx, cz, R, dmg)` (L323-327): Ink Tempest rain — small
    /// per-frame amounts, batched by the 0.3 s flush.
    #[allow(clippy::too_many_arguments)]
    pub fn rain(&mut self, owner: Option<BossAttacker>, cx: f32, cz: f32, r: f32, dmg: f32) {
        let Some(o) = owner else {
            return;
        };
        if o.is_remote || self.dead || !self.visible {
            return;
        }
        if (f64::from(self.pos.x) - f64::from(cx)).hypot(f64::from(self.pos.z) - f64::from(cz))
            > f64::from(r) + 2.4
        {
            return;
        }
        // JS L326: `this._rain.set(owner, (get || 0) + dmg)` — Map insertion
        // order; the port keys by `nid`.
        if let Some(e) = self.rain.iter_mut().find(|(a, _)| a.nid == o.nid) {
            e.1 += dmg;
        } else {
            self.rain.push((o, dmg));
        }
    }

    /// JS `_flushRain(dt)` (L328-334).
    fn flush_rain(&mut self, dt: f32) {
        self.rain_t -= f64::from(dt);
        if self.rain_t > 0.0 || self.rain.is_empty() {
            return;
        }
        self.rain_t = 0.3;
        let entries = std::mem::take(&mut self.rain);
        for (owner, d) in entries {
            // JS L332: `this.hit(owner, d, null, 'storm', null)`.
            self.hit(Some(owner), d, HitTarget::None, "storm", None);
        }
    }

    /// JS `applyDamage(attacker, d, weak, point)` (L337-350): host / offline
    /// — the damage lands. Returns the damage applied.
    pub fn apply_damage(
        &mut self,
        attacker: Option<BossAttacker>,
        d: f32,
        weak: bool,
        point: Option<Vec3>,
    ) -> f32 {
        if self.dead || self.invuln || !self.visible || self.state != Phase::Active {
            return 0.0;
        }
        let d = f64::from(d).min(f64::from(self.hp)); // JS L339
        self.hp = (f64::from(self.hp) - d) as f32; // JS L340
        // JS L341: attacker.stats bookkeeping (kept on the boss).
        if let Some(atk) = attacker {
            let st = self.stats_by_nid.entry(atk.nid).or_default();
            st.boss_dmg += d as f32;
            if weak {
                st.weak_hits += 1;
            }
        }
        // JS L342: `brain.noteDamage` — Task 43.
        self.flash_n += 1; // JS L343
        if weak {
            self.weak_n += 1;
        }
        // JS L344: `model.flash(clamp(d / 90, 0.25, 1), weak)` — render-side.
        if d >= 60.0 {
            self.hurt = clamp(d / 160.0, 0.3, 1.0) as f32; // JS L345
        }
        self.events.push(BossEvent::Hit {
            damage: d as f32,
            weak,
            attacker: attacker.map_or(-1, |a| a.nid),
            local: attacker.is_some_and(|a| a.is_local),
            blocked: false,
            predicted: false,
            crab: false,
            pos: point,
        }); // JS L346
        if self.hp <= 0.001 {
            // JS L347
            self.hp = 0.0;
            self.defeat(attacker);
        } else if self.sim {
            // JS L348: `this.brain?.checkPhase()` — the brain exists only
            // when simulating.
            self.check_phase();
        }
        d as f32
    }

    /// JS `remoteHit(d)` (L352-359): a guest's hit arrived (`{k:'bhit'}`).
    /// `attacker_exists` models `G.netm.byNid.get(d.a)` — a hit from an
    /// unknown roster id is dropped.
    pub fn remote_hit(&mut self, h: &RemoteHit, attacker_exists: bool) {
        if !self.sim {
            return;
        }
        if !attacker_exists {
            return; // JS L355
        }
        let atk = BossAttacker {
            nid: h.a,
            is_remote: false,
            is_local: false,
            has_nid: true,
        };
        self.log.recv += 1; // JS L356
        self.log.recv_dmg += h.d;
        if h.c >= 0 {
            // JS L357: a crablet hit.
            if let Some(idx) = self.crabs.iter().position(|c| c.id == h.c as u32) {
                let id = self.crabs[idx].id;
                self.hit_crab(atk, id, h.d, true);
            }
            return;
        }
        // JS L358: body hits are capped at 2000 per message.
        let d = f64::from(h.d).min(2000.0) as f32;
        self.apply_damage(Some(atk), d, h.weak, None);
    }

    /// JS `_defeat(attacker)` (L361-368).
    fn defeat(&mut self, attacker: Option<BossAttacker>) {
        if self.dead {
            return;
        }
        self.dead = true;
        self.stunned = false;
        self.anim = 0;
        self.killer = attacker.map_or(-1, |a| a.nid); // JS L364
        // JS L365-367: `hz.fizzle` / `brain.onDefeat` — Task 43;
        // `match.bossMode.onDefeat` — Task 44.
    }

    /// JS `_hitCrab(attacker, c, dmg, fromNet)` (L371-381).
    fn hit_crab(&mut self, attacker: BossAttacker, id: u32, dmg: f32, from_net: bool) {
        let Some(idx) = self.crabs.iter().position(|c| c.id == id) else {
            return;
        };
        if self.crabs[idx].dead {
            return;
        }
        if !self.sim && !from_net {
            // JS L373-377: guest — send (dropped for nid-less attackers,
            // netmatch.js L152), predict the marker.
            if attacker.has_nid {
                // JS netmatch.js L153: counters take the raw `dmg`; the wire
                // carries `r2(dmg)`.
                self.log.sent += 1;
                self.log.sent_dmg += dmg;
                self.guest_hits.push(DirectMsg::BossHit {
                    a: attacker.nid as u8,
                    d: crate::net::wire::r2(f64::from(dmg)) as f32,
                    weak: false,
                    w: "crab".to_string(),
                    c: id as i32,
                });
            }
            let c = &self.crabs[idx];
            self.events.push(BossEvent::Hit {
                damage: dmg,
                weak: false,
                attacker: attacker.nid,
                local: attacker.is_local,
                blocked: false,
                predicted: true,
                crab: true,
                pos: Some(vec3(c.x, c.y + 0.4, c.z)),
            });
            return;
        }
        self.crabs[idx].hp -= dmg; // JS L378
        if !from_net {
            // JS L379.
            let c = &self.crabs[idx];
            self.events.push(BossEvent::Hit {
                damage: dmg,
                weak: false,
                attacker: attacker.nid,
                local: attacker.is_local,
                blocked: false,
                predicted: false,
                crab: true,
                pos: Some(vec3(c.x, c.y + 0.4, c.z)),
            });
        }
        if self.crabs[idx].hp <= 0.0 {
            // JS L380.
            self.stats_by_nid.entry(attacker.nid).or_default().splats += 1;
            self.crab_pop(id, true);
        }
    }

    /// JS `crabPop(c, killed)` (L383-388): a crablet bursts (killed:
    /// harmless, into the squad's ink · otherwise: a boss-ink burst).
    fn crab_pop(&mut self, id: u32, killed: bool) {
        let Some(idx) = self.crabs.iter().position(|c| c.id == id) else {
            return;
        };
        if self.crabs[idx].dead {
            return;
        }
        self.crabs[idx].dead = true; // JS L385
        let (x, y, z) = (self.crabs[idx].x, self.crabs[idx].y, self.crabs[idx].z);
        // JS L386: `recBoss(['bc', id, r2 x, r2 y, r2 z, killed])`.
        self.events.push(BossEvent::Bc {
            id,
            x: r2(x),
            y: r2(y),
            z: r2(z),
            killed,
        });
        self.crab_burst(id, x, y, z, killed); // JS L387
    }

    /// JS `_crabBurst(id, x, y, z, killed)` (L390-402): the burst itself,
    /// everyone — guests get it on the host's timeline.
    #[allow(clippy::too_many_arguments)]
    fn crab_burst(&mut self, id: u32, x: f32, y: f32, z: f32, killed: bool) {
        if let Some(idx) = self.crabs.iter().position(|c| c.id == id) {
            self.crabs[idx].dead = true; // JS L392
            self.crab_pop_fx(idx);
        }
        // JS L393-395: the fx explosion at (x, y + 0.3, z) + the `'pop'`
        // emit.
        self.events.push(BossEvent::CrabBurst {
            id,
            x,
            y,
            z,
            killed,
        });
        // JS L396: `hz.crabBurst` when `!killed && live()` — Task 43.
        if self.sim {
            // JS L397-400: painted once, by the host (`!G.netm || isHost` —
            // sim implies host/offline here).
            let g = self.world.collision.ground_height(x, z, y + 0.8, false);
            if g > -5.0 && !self.nav.in_pad(x, z, 0.3) {
                self.events.push(BossEvent::Splat {
                    pos: vec3(x, g + 0.1, z),
                    radius: if killed { 1.3 } else { 1.7 },
                    team: if killed { 0 } else { 1 },
                    seed: self.rng.next_f32(),
                });
            }
        }
    }

    /// JS `_crabPopFx(c, killed)` (L403-405): gated on the crab having a
    /// model — i.e. the visuals having seen it (`spawned`).
    fn crab_pop_fx(&mut self, idx: usize) {
        let c = &mut self.crabs[idx];
        if c.spawned && !c.popped {
            c.popped = true;
            c.pop_t = 0.0;
        }
    }

    /// JS `pack(full)` (L422-431) → the [`BossSnap`] wire shape (the slot
    /// array itself is [`crate::net::wire::encode_boss_snap`]).
    #[must_use]
    pub fn pack(&self, full: bool) -> BossSnap {
        // JS L423-424: flags.
        let mut f = 0u32;
        if self.stunned {
            f |= 1;
        }
        if self.dead {
            f |= 2;
        }
        if self.invuln {
            f |= 4;
        }
        if self.visible {
            f |= 8;
        }
        // JS L425-426: live crablets only.
        let crabs = self
            .crabs
            .iter()
            .filter(|c| !c.dead)
            .map(|c| BossCrab {
                id: c.id,
                x: r2(c.x),
                y: r2(c.y),
                z: r2(c.z),
                yaw: r2(c.yaw),
                hp: js_round(f64::from(c.hp)) as u32,
            })
            .collect();
        // JS L427-428: `mv` is the move's t0 or -1. `BossSnap.mv` is f32, so
        // `_follow` compares `t0 as f32` against it.
        let mv = self
            .move_rec
            .as_ref()
            .and_then(|m| m.field("t0"))
            .and_then(JVal::as_f64)
            .map(|t| t as f32)
            .unwrap_or(-1.0);
        BossSnap {
            bt: r3(self.bt),
            x: r2(self.pos.x),
            y: r2(self.pos.y),
            z: r2(self.pos.z),
            yaw: r3(self.yaw),
            vx: r2(self.vel.x),
            vz: r2(self.vel.z),
            hp: (js_round(f64::from(self.hp) * 10.0) / 10.0) as f32,
            phase: self.phase,
            f,
            mv,
            anim: self.anim,
            turn: r2(self.turn),
            flash: self.flash_n,
            weak: self.weak_n,
            killer: self.killer,
            crabs,
            max_hp: js_round(f64::from(self.max_hp)) as u32,
            // JS L429: the record rides only on full snapshots.
            move_rec: if full { self.move_rec.clone() } else { None },
        }
    }

    /// JS `Boss.unpack(s, ts)` (L432-434): the slot array → `{t, …}`. The
    /// wire decoder owns the index mapping; the port returns the timestamp
    /// alongside the decoded snapshot.
    #[must_use]
    pub fn unpack(s: &JVal, ts: f64) -> Option<(f64, BossSnap)> {
        wire::decode_boss_snap(s).map(|snap| (ts, snap))
    }

    /// JS `onMove(m)` (L436): a move record from the host's timeline (or a
    /// snapshot's copy) — idempotent by `t0`.
    pub fn on_move(&mut self, m: JVal) {
        // JS L436: `if (m && typeof m.t0 === 'number') this.hz.add(m)`.
        let Some(t0) = m.field("t0").and_then(JVal::as_f64) else {
            return;
        };
        if self
            .moves
            .iter()
            .any(|e| e.field("t0").and_then(JVal::as_f64) == Some(t0))
        {
            return;
        }
        self.moves.push(m);
    }

    /// JS `_follow(dt)` (L439-472): guest — apply this frame's sampled
    /// snapshot (`net.cur`).
    fn follow(&mut self, dt: f32) {
        let Some(s) = self.net.cur.clone() else {
            return; // JS L441
        };
        let px = self.pos.x;
        let pz = self.pos.z;
        // JS L443-444: position + playback error, heading straight off the
        // sample.
        self.pos = vec3(
            s.x + self.net.err.x,
            s.y + self.net.err.y,
            s.z + self.net.err.z,
        );
        self.yaw = s.yaw;
        self.turn = s.turn;
        // JS L445-446: velocity from the position delta, clamped to the
        // sampled planar velocity.
        if dt > 0.0 {
            self.vel = vec3((self.pos.x - px) / dt, 0.0, (self.pos.z - pz) / dt);
        }
        if self.vel.length_squared() > 400.0 {
            self.vel = vec3(s.vx, 0.0, s.vz);
        }
        // JS L447: the boss clock never runs backwards on screen.
        self.bt = self.bt.max(s.bt);
        self.hp = s.hp; // JS L448
        self.phase = s.phase;
        if s.max_hp != 0 {
            self.max_hp = s.max_hp as f32;
        }
        // JS L449: flags.
        self.stunned = s.f & 1 != 0;
        self.invuln = s.f & 4 != 0;
        self.visible = s.f & 8 != 0;
        if s.f & 2 != 0 && !self.dead {
            // JS L450 (`hz.fizzle` — Task 43; the defeat broadcast rides
            // `emit_events`).
            self.killer = s.killer;
            self.dead = true;
        }
        // JS L451: set pieces.
        if s.anim != self.anim {
            self.anim = s.anim;
            self.anim_t0 = self.bt;
        }
        // JS L452-453: the move timeline, keyed by t0.
        if let Some(m) = s.move_rec.clone() {
            self.on_move(m);
        }
        self.move_rec = if s.mv >= 0.0 {
            self.moves
                .iter()
                .find(|m| {
                    m.field("t0")
                        .and_then(JVal::as_f64)
                        .is_some_and(|t| t as f32 == s.mv)
                })
                .cloned()
        } else {
            None
        };
        // JS L454-458: flashes for everyone's hits (the host counts them).
        // The JS `model.flash(0.6 / 0.8)` calls are approximated as zero
        // damage Hit events for the UI layer.
        if self.net.fl.is_some_and(|fl| s.flash > fl) {
            self.events.push(BossEvent::Hit {
                damage: 0.0,
                weak: false,
                attacker: -1,
                local: false,
                blocked: false,
                predicted: false,
                crab: false,
                pos: None,
            });
        }
        if self.net.wk.is_some_and(|wk| s.weak > wk) {
            self.events.push(BossEvent::Hit {
                damage: 0.0,
                weak: true,
                attacker: -1,
                local: false,
                blocked: false,
                predicted: false,
                crab: false,
                pos: None,
            });
        }
        self.net.fl = Some(s.flash);
        self.net.wk = Some(s.weak);
        self.flash_n = s.flash;
        self.weak_n = s.weak;
        // JS L459-470: crablets — the sampled set, interpolated per id.
        self.net.crab_seen.clear();
        for k in &s.crabs {
            self.net.crab_seen.insert(k.id);
            let idx = match self.crabs.iter().position(|c| c.id == k.id) {
                Some(i) => i,
                None => {
                    self.crabs.push(CrabState {
                        id: k.id,
                        x: k.x,
                        y: k.y,
                        z: k.z,
                        yaw: k.yaw,
                        hp: f64::from(k.hp) as f32,
                        dead: false,
                        speed: None,
                        miss_t: 0.0,
                        pop_t: 0.0,
                        spawned: false,
                        popped: false,
                    });
                    self.crabs.len() - 1
                }
            };
            if self.crabs[idx].dead {
                continue; // JS L466
            }
            // JS L467-468: the speed estimate, smoothed.
            let sp = {
                let c = &self.crabs[idx];
                (f64::from(k.x) - f64::from(c.x)).hypot(f64::from(k.z) - f64::from(c.z))
                    / f64::from(dt.max(1e-3))
            };
            self.crabs[idx].speed = Some(match self.crabs[idx].speed {
                None => 0.0,
                Some(prev) => {
                    (f64::from(prev)
                        + (sp.min(8.0) - f64::from(prev)) * (f64::from(dt) * 8.0).min(1.0))
                        as f32
                }
            });
            // JS L469.
            let c = &mut self.crabs[idx];
            c.x = k.x;
            c.y = k.y;
            c.z = k.z;
            c.yaw = k.yaw;
            c.hp = f64::from(k.hp) as f32;
        }
        // JS L471: ones gone from the snapshots without a burst event drop.
        for idx in 0..self.crabs.len() {
            let seen = self.net.crab_seen.contains(&self.crabs[idx].id);
            if !self.crabs[idx].dead && !seen {
                self.crabs[idx].miss_t += f64::from(dt) as f32;
                if self.crabs[idx].miss_t > 0.6 {
                    self.crabs[idx].dead = true;
                    self.crab_pop_fx(idx);
                }
            } else {
                self.crabs[idx].miss_t = 0.0;
            }
        }
    }

    /// Host migration: this client now runs the boss, from exactly what it
    /// was showing (JS `adopt`, L475-480).
    pub fn adopt(&mut self) {
        if self.sim {
            return;
        }
        self.sim = true;
        self.net.err = Vec3::ZERO; // JS L478
        self.net.err_v = Vec3::ZERO;
        // JS L479: `this.brain = new BossBrain(this, { adopt: true })` —
        // Task 43.
    }

    /// A new host took over: keep drawing from here, settle the offset
    /// (JS `handoff`, L482).
    pub fn handoff(&mut self) {
        self.net.buf.clear();
        self.net.handoff = true;
    }

    /// Host / brain: add a crablet (JS `bossBrain._spawnCrab`'s
    /// `crabs.set(id, …)`, id from `crabSeq++`, bossBrain.js L336).
    pub fn spawn_crab(&mut self, x: f32, y: f32, z: f32, yaw: f32, hp: f32) -> u32 {
        let id = self.crab_seq;
        self.crab_seq += 1;
        self.crabs.push(CrabState {
            id,
            x,
            y,
            z,
            yaw,
            hp,
            dead: false,
            speed: None,
            miss_t: 0.0,
            pop_t: 0.0,
            spawned: false,
            popped: false,
        });
        id
    }

    /// Drain the queued [`BossEvent`]s (JS `emit(...)`).
    pub fn drain_events(&mut self) -> Vec<BossEvent> {
        std::mem::take(&mut self.events)
    }

    /// Drain the guest's queued `sendBossHit` messages (JS L296/L374).
    pub fn drain_guest_hits(&mut self) -> Vec<DirectMsg> {
        std::mem::take(&mut self.guest_hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;

    /// TR-42.1 harness: a Tidewater [`Boss`] with its nav grid and the
    /// embedded tuning, built the same way as the batch-B nav tests.
    fn with_boss<T>(
        sim: bool,
        seed: u64,
        max_hp: f32,
        diff: &str,
        f: impl FnOnce(&mut Boss<'_>, &BossNav, &tuning::Boss, &CollisionWorld) -> T,
    ) -> T {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tuning = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tuning.boss.nav);
        let mut boss = Boss::new(
            &world,
            &nav,
            &tuning.boss,
            BossConfig {
                max_hp,
                difficulty: diff,
                sim,
                seed,
            },
        );
        f(&mut boss, &nav, &tuning.boss, &collision)
    }

    fn socket(tb: &tuning::Boss, name: &str) -> usize {
        tb.hit.iter().position(|h| h.socket == name).unwrap()
    }

    fn move_rec(t0: JVal) -> JVal {
        JVal::Obj(vec![
            ("id".to_string(), JVal::Str("slam".to_string())),
            ("t0".to_string(), t0),
        ])
    }

    // JS BOSS L17-24 + tuning: the HP the squad actually faces.
    #[test]
    fn hp_scaling_follows_tuning() {
        let t = crate::embedded_tuning();
        let b = &t.boss.boss;
        assert_eq!(b.hp_per_unit, 18000);
        assert_eq!(t.boss.squad_units.human, 1.0);
        assert_eq!(t.boss.squad_units.bot, 0.6);
        assert_eq!(b.hp_diff.easy, 0.75);
        assert_eq!(b.hp_diff.normal, 1.0);
        assert_eq!(b.hp_diff.hard, 1.3);
        // 1 human + 1 bot (bossMode multiplies hpPerUnit × units × diff).
        let units = f64::from(t.boss.squad_units.human) + f64::from(t.boss.squad_units.bot);
        let scaled = |diff: f32| f64::from(b.hp_per_unit) * units * f64::from(diff);
        assert!((scaled(1.0) - 28800.0).abs() < 1.0);
        assert!((scaled(0.75) - 21600.0).abs() < 1.0);
        assert!((scaled(1.3) - 37440.0).abs() < 1.0);
        // damage / pace tables ride the same section
        assert_eq!(b.dmg_diff.easy, 0.7);
        assert_eq!(b.dmg_diff.hard, 1.2);
        assert_eq!(b.pace_diff.easy, 1.25);
        assert_eq!(b.pace_diff.hard, 0.85);
    }

    // JS L36: dmgMul = dmgDiff[difficulty] ?? 1
    #[test]
    fn dmg_mul_per_difficulty() {
        with_boss(true, 1, 18000.0, "easy", |b, _, tb, _| {
            assert_eq!(b.dmg_mul, tb.boss.dmg_diff.easy);
            assert_eq!(b.dmg_mul, 0.7);
        });
        with_boss(true, 1, 18000.0, "hard", |b, _, _, _| {
            assert_eq!(b.dmg_mul, 1.2)
        });
        with_boss(true, 1, 18000.0, "insane", |b, _, _, _| {
            assert_eq!(b.dmg_mul, 1.0)
        });
    }

    // JS bossBrain.js L41-45: phase boundaries at 0.66 / 0.33.
    #[test]
    fn check_phase_boundaries() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            // exactly at phases[0] → want 2
            b.hp = 18000.0 * tb.boss.phases[0];
            b.check_phase();
            assert_eq!(b.pending_phase, 2);
            // exactly at phases[1] → want 3
            b.hp = 18000.0 * tb.boss.phases[1];
            b.check_phase();
            assert_eq!(b.pending_phase, 3);
            // between the two: want 2 does not overwrite a pending 3
            b.hp = 18000.0 * 0.5;
            b.check_phase();
            assert_eq!(b.pending_phase, 3);
            // full HP: want 1 never raises anything
            b.phase = 1;
            b.pending_phase = 1;
            b.hp = 18000.0;
            b.check_phase();
            assert_eq!(b.pending_phase, 1);
            b.hp = 18000.0 * 0.9;
            b.check_phase();
            assert_eq!(b.pending_phase, 1);
        });
    }

    // JS L75-93: the spawn pose must be a legal plan-region pose.
    #[test]
    fn spawn_point_is_a_valid_plan_pose() {
        with_boss(true, 1, 18000.0, "normal", |b, nav, _, _| {
            let [x, z] = b.home.unwrap();
            assert_eq!(b.pos.x, x);
            assert_eq!(b.pos.z, z);
            assert_eq!(b.pos.y, nav.floor_at(x, z));
            assert!(nav.is_plan(x, z));
            assert!(nav.pose_ok(x, z, b.yaw));
            assert!(!b.live()); // still Phase::Init
        });
    }

    // JS bossModel.js L106-108 (sim-side approximation, Task 43 notes).
    #[test]
    fn shape_active_rules() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            let (belly, crack_l, crack_r, shell_f) = (
                socket(tb, "belly"),
                socket(tb, "crackL"),
                socket(tb, "crackR"),
                socket(tb, "shellF"),
            );
            assert!(!b.shape_active(belly)); // phase 1, not stunned
            assert!(!b.shape_active(crack_l));
            assert!(!b.shape_active(crack_r));
            assert!(b.shape_active(shell_f));
            b.stunned = true;
            assert!(b.shape_active(belly));
            b.stunned = false;
            b.phase = 3;
            assert!(b.shape_active(belly));
            b.dead = true;
            assert!(!b.shape_active(shell_f));
            assert!(!b.shape_active(belly));
        });
    }

    // JS L205-231: weak spheres win by the te − 0.02 key when in front.
    #[test]
    fn seg_hit_prefers_weak_in_front() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            b.pos = Vec3::ZERO;
            b.yaw = 0.0;
            b.visible = true;
            let eye_l = socket(tb, "eyeL");
            let body = socket(tb, "body");
            // a −z ray through the eyeL centre also crosses shellF (r 1.8);
            // the weak sphere enters first and wins by the 0.02 bias.
            let hit = b
                .seg_hit(vec3(0.64, 3.74, 20.0), vec3(0.64, 3.74, -20.0), 0.0)
                .unwrap();
            match hit {
                SegHit::Shape { idx, weak, .. } => {
                    assert_eq!(idx, eye_l);
                    assert!(weak);
                }
                _ => panic!("expected a shape hit"),
            }
            // a ray through the shell centre misses the eyes (r 0.4) and
            // lands on the body sphere (te 0.5946 < shellF 0.6503).
            let hit = b
                .seg_hit(vec3(0.0, 2.87, 15.0), vec3(0.0, 2.87, -5.0), 0.0)
                .unwrap();
            match hit {
                SegHit::Shape { idx, weak, .. } => {
                    assert_eq!(idx, body);
                    assert!(!weak);
                }
                _ => panic!("expected a shape hit"),
            }
            // invisible → shapes drop out entirely
            b.visible = false;
            assert!(
                b.seg_hit(vec3(0.64, 3.74, 20.0), vec3(0.64, 3.74, -20.0), 0.0)
                    .is_none()
            );
        });
    }

    // JS L206/L232-250: a dead boss only exposes crablets.
    #[test]
    fn seg_hit_dead_boss_only_crabs() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.pos = Vec3::ZERO;
            b.visible = true;
            b.dead = true;
            let id = b.spawn_crab(0.0, 0.0, 10.0, 0.0, 90.0);
            let hit = b
                .seg_hit(vec3(0.0, 0.35, 20.0), vec3(0.0, 0.35, -20.0), 0.0)
                .unwrap();
            match hit {
                SegHit::Crab { id: cid, .. } => assert_eq!(cid, id),
                _ => panic!("expected a crab hit"),
            }
        });
    }

    // JS L252-269: the drum reaches low shapes only, crablets first.
    #[test]
    fn roll_hit_low_layer_filter_and_crabs() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            b.pos = Vec3::ZERO;
            b.yaw = 0.0;
            b.visible = true;
            let body = socket(tb, "body");
            let claw_l = socket(tb, "clawL");
            // ground drum: eyeL is geometrically closer (d 0.248 < body's
            // −0.15) but its bottom (3.34) is above cy + 0.7 → skipped.
            let hit = b.roll_hit(vec3(0.0, 0.2, 4.0), 0.0, -1.0, 1.5).unwrap();
            assert_eq!(hit.target, RollTarget::Shape(body));
            assert!(hit.boss);
            // y = max(cy, 2.2 − 0.525); f32-quantised, not bit-exact to
            // the decimal literal (1.6750001).
            let p = hit.point;
            assert!(
                (p.x - 0.0).abs() < 1e-5 && (p.y - 1.675).abs() < 1e-5 && (p.z - 3.2).abs() < 1e-5
            );
            // beside clawL the drum reaches the claw
            let hit = b.roll_hit(vec3(1.5, 0.2, 5.9), 0.0, -1.0, 1.5).unwrap();
            assert_eq!(hit.target, RollTarget::Shape(claw_l));
            // crablets win over body shapes
            let id = b.spawn_crab(1.5, 0.3, 5.1, 0.0, 90.0);
            let hit = b.roll_hit(vec3(1.5, 0.2, 5.9), 0.0, -1.0, 1.5).unwrap();
            assert_eq!(hit.target, RollTarget::Crab(id));
            assert!(!hit.boss);
            // dead: crabs still, no body
            b.dead = true;
            assert!(b.roll_hit(vec3(1.5, 0.2, 5.9), 0.0, -1.0, 1.5).is_some());
            b.crabs[0].dead = true;
            assert!(b.roll_hit(vec3(0.0, 0.2, 4.0), 0.0, -1.0, 1.5).is_none());
        });
    }

    // JS L272-284: exact ray/sphere distance, −1 on miss.
    #[test]
    fn ray_dist_hits_nearest_sphere() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.pos = Vec3::ZERO;
            b.yaw = 0.0;
            b.visible = true;
            // aimed straight at eyeL (centre z 3.1, r 0.4) from z 10
            assert_eq!(
                b.ray_dist(vec3(0.64, 3.74, 10.0), vec3(0.0, 0.0, -1.0), 20.0),
                6.5
            );
            // nothing within 3 m behind the boss
            assert_eq!(
                b.ray_dist(vec3(0.0, 0.0, -10.0), vec3(0.0, 0.0, 1.0), 3.0),
                -1.0
            );
            b.visible = false;
            assert_eq!(
                b.ray_dist(vec3(0.64, 3.74, 10.0), vec3(0.0, 0.0, -1.0), 20.0),
                -1.0
            );
        });
    }

    // JS L292 + L22: weak × stunned × weapon multipliers, per weapon key.
    #[test]
    fn hit_multiplier_chain() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            b.state = Phase::Active;
            b.visible = true;
            let eye_l = socket(tb, "eyeL");
            let atk = Some(BossAttacker {
                nid: 1,
                is_remote: false,
                is_local: true,
                has_nid: true,
            });
            // weak × roller: 100 × 2.5 × 0.3 = 75
            b.hit(atk, 100.0, HitTarget::Shape(eye_l), "roller", None);
            assert_eq!(b.hp, 18000.0 - 75.0);
            // weak × stunned × roller: 100 × 2.5 × 1.25 × 0.3 = 93.75
            b.stunned = true;
            b.hit(atk, 100.0, HitTarget::Shape(eye_l), "roller", None);
            assert_eq!(b.hp, 18000.0 - 75.0 - 93.75);
            // unknown weapon id → ×1: 100 × 2.5 × 1.25 = 312.5
            b.hit(atk, 100.0, HitTarget::Shape(eye_l), "storm", None);
            assert_eq!(b.hp, 18000.0 - 75.0 - 93.75 - 312.5);
            b.stunned = false;
            // every weapon-table key, no weak / stun bonus
            let expect = |wid: &str, mul: f32, b: &mut Boss<'_>| {
                let before = b.hp;
                b.hit(atk, 1000.0, HitTarget::None, wid, None);
                assert_eq!(before - b.hp, 1000.0 * mul, "weapon {wid}");
            };
            expect("slam", 0.6, b);
            expect("bomb", 0.75, b);
            expect("sticky", 0.75, b);
            expect("seeker", 0.75, b);
            expect("burst", 0.75, b);
            expect("shaker", 0.75, b);
            expect("waddle", 0.75, b);
            expect("torpedo", 0.75, b);
            expect("roller", 0.3, b);
            // per-attacker bookkeeping + counters
            let st = b.stats_by_nid[&1];
            assert_eq!(st.weak_hits, 3);
            assert!(st.boss_dmg > 0.0);
            assert_eq!(b.flash_n, 12);
            assert_eq!(b.weak_n, 3);
        });
    }

    // JS L289/L294-300: blocked, remote and guest-predicted paths.
    #[test]
    fn hit_blocked_and_guest_predict() {
        with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            b.state = Phase::Active;
            b.visible = true;
            b.invuln = true;
            let body = socket(tb, "body");
            let atk = Some(BossAttacker {
                nid: 2,
                is_remote: false,
                is_local: true,
                has_nid: true,
            });
            b.hit(
                atk,
                100.0,
                HitTarget::Shape(body),
                "slam",
                Some(vec3(1.0, 2.0, 3.0)),
            );
            assert_eq!(b.hp, 18000.0);
            let ev = b.drain_events();
            assert!(matches!(
                &ev[..],
                [BossEvent::Hit { blocked: true, damage: 0.0, pos, .. }]
                    if pos == &Some(vec3(1.0, 2.0, 3.0))
            ));
            // JS L294: a non-local attacker emits no blocked event.
            b.hit(
                Some(BossAttacker {
                    nid: 2,
                    is_remote: false,
                    is_local: false,
                    has_nid: true,
                }),
                100.0,
                HitTarget::Shape(body),
                "slam",
                Some(vec3(4.0, 5.0, 6.0)),
            );
            assert!(b.drain_events().is_empty());
            // remote attackers never hurt (L289)
            b.invuln = false;
            b.hit(
                Some(BossAttacker {
                    nid: 2,
                    is_remote: true,
                    is_local: false,
                    has_nid: true,
                }),
                100.0,
                HitTarget::None,
                "slam",
                None,
            );
            assert_eq!(b.hp, 18000.0);
            assert!(b.drain_events().is_empty());
        });
        // guest: queue the message, predict the marker, apply nothing
        with_boss(false, 1, 18000.0, "normal", |b, _, tb, _| {
            b.state = Phase::Active;
            b.visible = true;
            let body = socket(tb, "body");
            b.hit(
                Some(BossAttacker {
                    nid: 5,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                100.0,
                HitTarget::Shape(body),
                "slam",
                None,
            );
            assert_eq!(b.hp, 18000.0);
            let msgs = b.drain_guest_hits();
            assert!(
                matches!(
                    &msgs[..],
                    // JS netmatch.js L153: the wire carries `r2(d)` —
                    // f32(0.6)·100 = 60.000004 quantizes to exactly 60.0.
                    [DirectMsg::BossHit {
                        a: 5,
                        d: 60.0,
                        weak: false,
                        c: -1,
                        ..
                    }]
                ),
                "msgs={msgs:?}"
            );
            match &msgs[0] {
                DirectMsg::BossHit { w, .. } => assert_eq!(w, "slam"),
                _ => unreachable!(),
            }
            // JS netmatch.js L153: audit counters take the raw d.
            assert_eq!(b.log.sent, 1);
            assert!((f64::from(b.log.sent_dmg) - 60.000004).abs() < 1e-5);
            let ev = b.drain_events();
            assert!(matches!(
                &ev[..],
                // JS boss.js L298: the predicted event carries the raw d.
                [BossEvent::Hit { predicted: true, damage, local: true, .. }]
                    if (*damage - 60.0).abs() < 1e-5
            ));
        });
        // JS netmatch.js L152: a nid-less guest attacker sends nothing but
        // still predicts locally.
        with_boss(false, 1, 18000.0, "normal", |b, _, tb, _| {
            b.state = Phase::Active;
            b.visible = true;
            let body = socket(tb, "body");
            b.hit(
                Some(BossAttacker {
                    nid: 5,
                    is_remote: false,
                    is_local: true,
                    has_nid: false,
                }),
                100.0,
                HitTarget::Shape(body),
                "slam",
                None,
            );
            assert!(b.drain_guest_hits().is_empty());
            assert_eq!(b.log.sent, 0);
            assert!(matches!(
                &b.drain_events()[..],
                [BossEvent::Hit {
                    predicted: true,
                    ..
                }]
            ));
        });
    }

    // JS L304-321: falloff, crablet full damage, cover blocking.
    #[test]
    fn splash_falloff_crabs_and_cover() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            // pin the pose so the body socket sits exactly at pos + (0,2.2,2.3)
            b.pos = Vec3::ZERO;
            b.yaw = 0.0;
            // open ground: a blast 1.5 m in front of the body sphere
            let bp = vec3(b.pos.x, b.pos.y + 2.2, b.pos.z + 2.3);
            let c = vec3(bp.x, bp.y, bp.z + 1.5);
            // bd = 1.5 − 1.05 = 0.45 → 100 + (50 − 100) × 0.45/2 = 88.75
            assert!(
                b.world
                    .collision
                    .los(vec3(c.x, c.y + 0.3, c.z), bp.lerp(c, 0.7))
            );
            b.splash(
                Some(BossAttacker {
                    nid: 1,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                c,
                2.0,
                100.0,
                50.0,
                "storm",
            );
            assert_eq!(b.hp, 18000.0 - 88.75);
            // crablets inside R + 0.4 take the full dmax
            let id = b.spawn_crab(c.x, c.y - 2.2, c.z, 0.0, 60.0);
            b.splash(
                Some(BossAttacker {
                    nid: 1,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                c,
                2.0,
                100.0,
                50.0,
                "storm",
            );
            assert!(b.crabs.iter().find(|cr| cr.id == id).unwrap().dead);
            assert_eq!(b.stats_by_nid[&1].splats, 1);
            // invisible → no body damage any more
            b.visible = false;
            let before = b.hp;
            b.splash(
                Some(BossAttacker {
                    nid: 1,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                c,
                2.0,
                100.0,
                50.0,
                "storm",
            );
            assert_eq!(b.hp, before);
        });
        // cover blocks: park the boss behind a tall wall block, blast the
        // far side — the los probe the splash runs is blocked.
        with_boss(true, 1, 18000.0, "normal", |b, _, _, col| {
            b.state = Phase::Active;
            b.visible = true;
            // Tidewater's `crescent-corner` plinths: 3.55 × 3.3 × 3.55 m,
            // solid, axis-aligned — tall and deep enough to swallow the
            // los probe below.
            let blk = col
                .blocks
                .iter()
                .find(|bl| {
                    bl.solid
                        && !bl.grate
                        && bl.half.y >= 3.0
                        && bl.half.x >= 3.4
                        && bl.half.z >= 3.4
                        && bl.axes[1].y > 0.999
                        && bl.axes[0].z.abs() < 0.001
                })
                .expect("a tall axis-aligned wall block");
            let bc = blk.center;
            b.pos = vec3(bc.x, bc.y - 2.2, bc.z - 5.33);
            b.yaw = 0.0;
            let c = vec3(bc.x, bc.y, bc.z + blk.half.z + 1.2);
            // geometry self-check: clawL sits at the block centre, the
            // 0.11-lerped los target is inside the solid
            let claw = vec3(bc.x + 2.08, bc.y - 1.356, bc.z);
            let t = (0.85f64 / (0.85f64).max((claw - c).length() as f64)).min(0.9) as f32;
            assert!(!col.los(vec3(c.x, c.y + 0.3, c.z), claw.lerp(c, t)));
            b.splash(
                Some(BossAttacker {
                    nid: 1,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                }),
                c,
                8.0,
                100.0,
                50.0,
                "storm",
            );
            assert_eq!(b.hp, 18000.0);
            assert!(b.drain_events().is_empty());
        });
    }

    // JS L323-334: rain accumulates per owner, flushed every 0.3 s as 'storm'.
    #[test]
    fn rain_batches_per_03s_as_storm() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            let atk = Some(BossAttacker {
                nid: 7,
                is_remote: false,
                is_local: true,
                has_nid: true,
            });
            b.rain(atk, b.pos.x, b.pos.z, 1.0, 5.0);
            b.rain(atk, b.pos.x, b.pos.z, 1.0, 5.0);
            b.rain(atk, b.pos.x + 100.0, b.pos.z, 1.0, 5.0); // out of range
            b.update(0.1); // (0 − 0.1) ≤ 0 → flush
            assert_eq!(b.hp, 18000.0 - 10.0); // one batched hit, storm ×1
            let ev = b.drain_events();
            assert_eq!(
                ev.iter()
                    .filter(|e| matches!(e, BossEvent::Hit { .. }))
                    .count(),
                1
            );
            // the next batch waits out the 0.3 s window
            b.rain(atk, b.pos.x, b.pos.z, 1.0, 3.0);
            b.update(0.2); // 0.3 − 0.2 = 0.1 > 0 → no flush
            assert_eq!(b.hp, 18000.0 - 10.0);
            b.update(0.15); // → −0.05 → flush
            assert_eq!(b.hp, 18000.0 - 13.0);
        });
    }

    // JS L352-359: 2000 body cap, crablet branch, unknown nid dropped.
    #[test]
    fn remote_hit_body_cap_and_crab() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            b.remote_hit(
                &RemoteHit {
                    a: 9,
                    d: 5000.0,
                    weak: false,
                    c: -1,
                },
                false,
            );
            assert_eq!(b.hp, 18000.0);
            b.remote_hit(
                &RemoteHit {
                    a: 3,
                    d: 5000.0,
                    weak: false,
                    c: -1,
                },
                true,
            );
            assert_eq!(b.hp, 16000.0); // min(5000, 2000)
            assert_eq!(b.log.recv, 1);
            assert_eq!(b.log.recv_dmg, 5000.0);
            // crablet branch (c ≥ 0)
            let id = b.spawn_crab(0.0, 0.0, 0.0, 0.0, 90.0);
            b.remote_hit(
                &RemoteHit {
                    a: 3,
                    d: 100.0,
                    weak: false,
                    c: id as i32,
                },
                true,
            );
            assert_eq!(b.crabs[0].hp, -10.0);
            assert!(b.crabs[0].dead);
            assert_eq!(b.stats_by_nid[&3].splats, 1);
            // from_net → no local crab markers
            assert!(
                !b.events
                    .iter()
                    .any(|e| matches!(e, BossEvent::Hit { crab: true, .. }))
            );
        });
        // guests never run remoteHit
        with_boss(false, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            b.remote_hit(
                &RemoteHit {
                    a: 3,
                    d: 100.0,
                    weak: false,
                    c: -1,
                },
                true,
            );
            assert_eq!(b.hp, 18000.0);
        });
    }

    // JS L422-434 + L439-472: pack → wire → unpack → _follow, field by field.
    #[test]
    fn pack_unpack_follow_roundtrip() {
        let host = with_boss(true, 1, 18000.0, "normal", |b, _, tb, _| {
            b.state = Phase::Active;
            b.visible = true;
            b.stunned = true;
            b.pos = vec3(1.25, -0.25, 3.75);
            b.yaw = 0.7;
            b.vel = vec3(2.0, 0.0, -3.0);
            b.turn = 0.1234;
            b.hp = 12345.678;
            b.phase = 2;
            b.anim = 2;
            b.flash_n = 4;
            b.weak_n = 2;
            b.move_rec = Some(move_rec(JVal::Num(5.0)));
            b.spawn_crab(1.0, 0.0, 2.0, 0.5, 90.0);
            let _ = tb;
            b.pack(true)
        });
        // wire round trip: encode → decode is lossless for a packed snapshot
        let j = wire::encode_boss_snap(&host, true);
        let (ts, snap) = Boss::unpack(&j, 12.5).unwrap();
        assert_eq!(ts, 12.5);
        assert_eq!(snap, host);
        assert_eq!(snap.f, 1 | 8); // stunned | visible
        assert_eq!(snap.hp, 12345.7); // one decimal
        assert_eq!(snap.max_hp, 18000);
        assert_eq!(snap.mv, 5.0);

        // the guest follows it
        with_boss(false, 2, 18000.0, "normal", move |b, _, _, _| {
            b.state = Phase::Active;
            b.net.cur = Some(snap.clone());
            b.update(0.05);
            assert_eq!(b.pos, vec3(snap.x, snap.y, snap.z));
            assert_eq!(b.yaw, snap.yaw);
            assert_eq!(b.turn, snap.turn);
            assert_eq!(b.bt, snap.bt);
            assert_eq!(b.hp, snap.hp);
            assert_eq!(b.phase, 2);
            assert!(b.stunned && b.visible && !b.invuln && !b.dead);
            assert_eq!(b.anim, 2);
            assert_eq!(b.anim_t0, b.bt); // L451: anim changed → t0 = bt
            assert_eq!(b.flash_n, 4);
            assert_eq!(b.weak_n, 2);
            // delta velocity (25, 0, 75) exceeds 20 m/s → sampled (L446)
            assert_eq!(b.vel, vec3(snap.vx, 0.0, snap.vz));
            // the move is found by its t0 (L453)
            assert_eq!(b.move_rec, Some(move_rec(JVal::Num(5.0))));
            assert_eq!(b.crabs.len(), 1);
            assert_eq!(b.crabs[0].id, 0);
            assert_eq!(b.crabs[0].speed, Some(0.0)); // first sample
            let ev = b.drain_events();
            assert!(ev.iter().any(|e| matches!(e, BossEvent::Stunned)));
            assert!(
                ev.iter()
                    .any(|e| matches!(e, BossEvent::Phase { phase: 2 }))
            );
            assert!(ev.iter().any(|e| matches!(e, BossEvent::Move(_))));
            assert!(ev.iter().any(|e| matches!(e, BossEvent::CrabSpawn { .. })));
            assert!(ev.iter().any(|e| matches!(e, BossEvent::Hp { .. })));
            assert!(!ev.iter().any(|e| matches!(e, BossEvent::Intro)));
        });
    }

    // JS L459-472: crab speed / miss / removal; L450: the dead bit.
    #[test]
    fn follow_crab_lifecycle_and_defeat_bit() {
        let base = with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            b.spawn_crab(0.0, 0.0, 0.0, 0.0, 90.0);
            b.pack(true)
        });
        with_boss(false, 2, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.net.cur = Some(base.clone());
            b.update(0.05);
            // the crab moved +2 m in x over 0.25 s → sp = 8 → speed → 8
            let mut s2 = base.clone();
            s2.crabs[0].x = 2.0;
            b.net.cur = Some(s2);
            b.update(0.25);
            assert_eq!(b.crabs[0].speed, Some(8.0));
            assert_eq!(b.crabs[0].x, 2.0);
            // gone from the sample: missT accumulates, then the crab drops
            let mut s3 = base.clone();
            s3.crabs.clear();
            b.net.cur = Some(s3);
            b.update(0.3);
            assert!(!b.crabs[0].dead);
            assert!((b.crabs[0].miss_t - 0.3).abs() < 1e-6);
            b.update(0.35); // 0.65 > 0.6 → dead + pop fx
            assert!(b.crabs[0].dead);
            assert!(b.crabs[0].popped);
            b.update(1.3); // popT > 1.2 → forgotten
            assert!(b.crabs.is_empty());
        });
        // the dead bit carries the killer (L450)
        with_boss(false, 3, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            let mut s = base.clone();
            s.f |= 2;
            s.killer = 4;
            b.net.cur = Some(s);
            b.update(0.05);
            assert!(b.dead);
            assert_eq!(b.killer, 4);
            let ev = b.drain_events();
            assert!(
                ev.iter()
                    .any(|e| matches!(e, BossEvent::Defeat { killer: 4 }))
            );
        });
    }

    // JS L475-482.
    #[test]
    fn adopt_and_handoff() {
        with_boss(false, 1, 18000.0, "normal", |b, _, _, _| {
            b.net.err = vec3(1.0, 2.0, 3.0);
            b.net.buf.push((1.0, b.pack(false)));
            b.handoff();
            assert!(b.net.buf.is_empty());
            assert!(b.net.handoff);
            b.adopt();
            assert!(b.sim);
            assert_eq!(b.net.err, Vec3::ZERO);
            assert_eq!(b.net.err_v, Vec3::ZERO);
        });
    }

    // JS L436: hz.add is idempotent by t0; non-numeric t0 is dropped.
    #[test]
    fn on_move_is_idempotent_by_t0() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.on_move(move_rec(JVal::Num(1.0)));
            b.on_move(move_rec(JVal::Num(1.0)));
            b.on_move(move_rec(JVal::Num(2.0)));
            b.on_move(move_rec(JVal::Str("x".to_string())));
            b.on_move(JVal::Null);
            assert_eq!(b.moves.len(), 2);
        });
    }

    // Same seed → byte-identical state and event queue (spec determinism).
    #[test]
    fn same_seed_same_result() {
        let run = |seed: u64| {
            with_boss(true, seed, 18000.0, "normal", move |b, _, _, _| {
                b.state = Phase::Active;
                b.visible = true;
                let atk = BossAttacker {
                    nid: 1,
                    is_remote: false,
                    is_local: true,
                    has_nid: true,
                };
                b.splash(
                    Some(atk),
                    vec3(b.pos.x, b.pos.y + 2.2, b.pos.z + 3.8),
                    2.0,
                    100.0,
                    50.0,
                    "storm",
                );
                let id = b.spawn_crab(b.pos.x, b.pos.y, b.pos.z + 4.0, 0.0, 90.0);
                b.hit_crab(atk, id, 200.0, false); // pops → Splat (rng seed)
                (b.pack(true), b.drain_events())
            })
        };
        let a = run(42);
        let b2 = run(42);
        let c = run(43);
        assert_eq!(a.0, b2.0);
        assert_eq!(a.1, b2.1);
        // the state itself is seed-independent; only the splat seed differs
        assert_eq!(a.0, c.0);
        assert_ne!(a.1, c.1);
    }

    // JS L101: nothing runs while the match is still loading.
    #[test]
    fn update_returns_early_while_loading() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.update(0.1);
            assert!(b.events.is_empty());
            assert_eq!(b.bt, 0.0);
            assert_eq!(b.paint_bt, 0.0);
        });
    }

    // JS L383-402: kill bookkeeping, the ['bc'] record and the burst paint.
    #[test]
    fn crab_pop_records_and_paints() {
        with_boss(true, 1, 18000.0, "normal", |b, _, _, _| {
            b.state = Phase::Active;
            b.visible = true;
            let id = b.spawn_crab(1.234, -0.25, 5.678, 0.0, 90.0);
            b.crab_pop(id, false); // killed by the boss's own burst
            let c = &b.crabs[0];
            assert!(c.dead);
            let ev = b.drain_events();
            assert_eq!(ev.len(), 3);
            match &ev[0] {
                BossEvent::Bc {
                    id,
                    x,
                    y,
                    z,
                    killed,
                } => {
                    assert_eq!(*id, 0);
                    assert!((x - 1.23).abs() < 1e-6);
                    assert!((y - (-0.25)).abs() < 1e-6);
                    assert!((z - 5.68).abs() < 1e-6);
                    assert!(!killed);
                }
                other => panic!("expected Bc, got {other:?}"),
            }
            match &ev[1] {
                BossEvent::CrabBurst { id, killed, .. } => {
                    assert_eq!(*id, 0);
                    assert!(!killed);
                }
                other => panic!("expected CrabBurst, got {other:?}"),
            }
            let seed = match &ev[2] {
                BossEvent::Splat {
                    radius, team, seed, ..
                } => {
                    assert!((radius - 1.7).abs() < 1e-6);
                    assert_eq!(*team, 1);
                    *seed
                }
                other => panic!("expected Splat, got {other:?}"),
            };
            // The burst splat consumed the seeded rng stream (JS
            // `Math.random()`): xorshift32 never repeats back to back.
            assert_ne!(b.rng.next_f32(), seed);
            // a dead crab pops are idempotent
            b.crab_pop(id, true);
            assert!(b.drain_events().is_empty());
        });
    }
}
