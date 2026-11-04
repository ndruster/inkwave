//! `BossBrain` — the host-side Boss AI, line-by-line port of
//! `src/boss/bossBrain.js` (376 lines). Task 43 batch C of the M4 plan.
//!
//! Spec §2 mapping: `this.b` → the `boss` parameter of every method; `G` →
//! `boss.world` + the `squad` passed into [`BossBrain::update`]; `BOSS`/`HZ`/
//! `PLAYER` → `boss.tb` (tuning, D-T42-1 — the JS L12-16 constants are
//! extracted, never hand-copied); `moveTimes`/`movePhaseAt`/`chargeDist`/
//! `beamAngle`/`hz.add` → the `hazards` accessors (batch B); `nav` → the
//! shared `BossNav` (its A* scratch is interior-mutable so `path` works
//! through the entity's `&BossNav`).
//!
//! Determinism: `this.rnd` (JS L23 `Math.random`) is a dedicated [`Rng`]
//! stream seeded from the boss seed (D-T43). Records are host-only
//! (D-T42-7: JS↔Rust host interop is unsupported), so the barrage candidate
//! sort below deviates from the JS comparator's per-comparison `rnd()` draws
//! with a pre-drawn per-candidate jitter — see [`BossBrain::start`].

use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

use crate::net::wire::{self, JVal};
use crate::tuning;
use crate::weapon::Rng;

use super::hazards::{
    MoveId, beam_angle, charge_dist, charge_time, move_phase_at, move_times, rec_d, rec_id, rec_n,
    rec_p_f64, rec_t0,
};
use super::{Boss, BossSquad, CrabState};

/// JS `angleDiff` (core/ctx.js L27) — wraps `b - a` into `(-π, π]` via the
/// same while loops (f64, matching JS number semantics).
fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = b - a;
    while d > PI {
        d -= TAU;
    }
    while d < -PI {
        d += TAU;
    }
    d
}

fn clampf(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

/// JS `!(a > b)` (bossBrain.js L253/L255/L335/L367): `a` at or below `b`.
/// Unlike `a <= b`, this is also `true` when either side is NaN — JS's
/// `NaN > b` is `false`, so the negated comparison is the faithful port.
fn not_above(a: f64, b: f64) -> bool {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal) != Ordering::Greater
}

/// JS `this._lane` return shape (bossBrain.js L224 `{yaw, L, wall}`).
#[derive(Debug, Clone, Copy)]
pub struct Lane {
    pub yaw: f64,
    pub l: f64,
    pub wall: bool,
}

/// JS `this.plan` shape (bossBrain.js L209-211). `faceFn` is the dynamic
/// aim-at-target closure of JS L193/L209; Rust stores a flag and re-evaluates
/// the same expression against the squad at check time (L96).
#[derive(Debug, Clone)]
pub struct Plan {
    pub id: MoveId,
    pub face: Option<f64>,
    pub face_fn: bool,
    pub lane: Option<Lane>,
    pub t: f64,
}

/// JS `this.cd` (bossBrain.js L27 `{crablets, frenzy}`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BrainCd {
    pub crablets: f64,
    pub frenzy: f64,
}

/// The host-side brain. JS `class BossBrain` (bossBrain.js L20). The JS
/// `pendingPhase` (L29) lives on the entity as `Boss::pending_phase`
/// (hoisted in Task 42 batch A) — one source of truth, read/written here.
#[derive(Debug)]
pub struct BossBrain {
    /// JS L23 `rnd = rng(...)` — dedicated stream (D-T43 seed derivation).
    pub rnd: Rng,
    /// JS L24 `cool` — ctor 2.4, adopt 1.0 (L24).
    pub cool: f64,
    /// JS L25 `path` / `pi` — current walk path + cursor.
    pub path: Option<Vec<[f32; 2]>>,
    pub pi: usize,
    /// JS L25 `goal` / `goalT` / `stuckT`.
    pub goal: Option<[f32; 2]>,
    pub goal_t: f64,
    pub stuck_t: f64,
    /// JS L26 `attn` / `dmgBy` — target-attention / damage attribution maps
    /// keyed by actor id (`nid ?? slot`).
    pub attn: HashMap<i32, f64>,
    pub dmg_by: HashMap<i32, f64>,
    pub cd: BrainCd,
    /// JS L27 `last` — last executed move (×0.3 weight, L203).
    pub last: Option<MoveId>,
    /// JS L28 `retarget`.
    pub retarget: f64,
    /// JS L28 `plan` — in-progress move plan.
    pub plan: Option<Plan>,
    /// JS L28 `speed` — smoothed locomotion speed.
    pub speed: f64,
    /// JS L207 `force` — audit hook: next `_choose` is forced to this move.
    pub force: Option<MoveId>,
    /// JS L35 `spawned` — crablets already released for the current move.
    pub spawned: bool,
    /// JS L32 `crabSeq` — authoritative crab id counter for the host.
    pub crab_seq: u32,
}

impl BossBrain {
    /// JS ctor L21-37. `adopt` = `{adopt:true}` (guest→host handover); the
    /// `rnd` seed replaces JS `Math.random` (D-T43: `seed ^ golden-ratio`).
    pub fn new(boss: &mut Boss<'_>, adopt: bool, seed: u64) -> Self {
        let mut me = Self {
            rnd: Rng::new(seed),
            cool: if adopt { 1.0 } else { 2.4 }, // JS L24
            path: None,
            pi: 0,
            goal: None,
            goal_t: 0.0,
            stuck_t: 0.0,
            attn: HashMap::new(),
            dmg_by: HashMap::new(),
            cd: BrainCd::default(),
            last: None,
            retarget: 0.0,
            plan: None,
            speed: 0.0,
            force: None,
            spawned: false,
            crab_seq: 0,
        };
        // JS L30-32: guest-side crabs lack `age/tgt/retgt/speed` — fill the
        // defaults and take `crabSeq` past the highest live id. Rust `CrabState`
        // always carries the numeric defaults, so the fill reduces to the
        // `speed = c.speed || 0` half (JS L31); age/tgt/retgt are already the
        // JS defaults when absent.
        let mut max_id = 0u32;
        for c in boss.crabs.iter_mut() {
            max_id = max_id.max(c.id);
            c.speed = Some(c.speed.unwrap_or(0.0));
        }
        me.crab_seq = max_id + 1;
        // The entity mirror only matters when guest ids already exist (adopt):
        // with no crabs the entity keeps its batch-B counter (`spawn_crab`
        // ids), and the two streams are kept collision-free by `spawn_crab`
        // raising the brain's counter to match.
        if !boss.crabs.is_empty() {
            boss.crab_seq = me.crab_seq;
        }
        // JS L35 `spawned`: a crablets move already past its tele time has
        // already released its litter.
        me.spawned = boss.move_rec.as_ref().is_some_and(|m| {
            rec_id(m) == Some(MoveId::Crablets) && f64::from(boss.bt) - rec_t0(m) >= rec_d(m, 0)
        });
        if adopt {
            me.check_phase(boss); // JS L36
        }
        me
    }

    /// JS L39 `get pace()` — `BOSS.paceDiff[difficulty] ?? 1`.
    pub fn pace(&self, boss: &Boss) -> f64 {
        let pd = &boss.tb.boss.pace_diff;
        match boss.difficulty.as_str() {
            "easy" => f64::from(pd.easy),
            "normal" => f64::from(pd.normal),
            "hard" => f64::from(pd.hard),
            _ => 1.0,
        }
    }

    /// JS L40 `noteDamage(atk, d)` — damage attribution for target scoring.
    pub fn note_damage(&mut self, atk_nid: i32, d: f64) {
        *self.dmg_by.entry(atk_nid).or_insert(0.0) += d;
    }

    /// JS L41-44 `checkPhase()` — the hoisted entity method (Task 42 batch A).
    pub fn check_phase(&mut self, boss: &mut Boss<'_>) {
        boss.check_phase();
    }

    /// JS L45 `onDefeat()` — clear the entity move + brain path/plan.
    pub fn on_defeat(&mut self, boss: &mut Boss<'_>) {
        boss.move_rec = None;
        self.path = None;
        self.plan = None;
    }

    /// JS L48-72 `update(dt)`.
    pub fn update(&mut self, dt: f32, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>) {
        let (px, pz, pyaw) = (boss.pos.x, boss.pos.z, boss.yaw);
        // JS L50: the brain owns the boss-time clock on the host. `bt` stays
        // f32 on the entity (same as the guest `follow` path); f64 math below
        // matches the JS number semantics at the use sites.
        boss.bt += dt;
        let bt = f64::from(boss.bt);
        // JS L52-53: cooldowns tick, damage attribution decays.
        self.cd.crablets -= f64::from(dt);
        self.cd.frenzy -= f64::from(dt);
        let decay = (-f64::from(dt) / 8.0).exp();
        for v in self.dmg_by.values_mut() {
            *v *= decay;
        }
        // JS L55-56: intro reveal + intro animation beat.
        if !boss.visible && bt >= f64::from(boss.tb.boss.intro_at) {
            boss.visible = true;
            boss.anim = 1;
            boss.anim_t0 = boss.bt;
        }
        if boss.anim == 1 && f64::from(boss.bt - boss.anim_t0) >= f64::from(boss.tb.boss.intro_len)
        {
            boss.anim = 0;
        }
        // JS L57: roar animation beat.
        if boss.anim == 2 && f64::from(boss.bt - boss.anim_t0) >= f64::from(boss.tb.boss.roar_len) {
            boss.anim = 0;
            boss.invuln = false;
            self.cool = self.cool.min(0.9 * self.pace(boss));
        }
        // JS L58-65: the state machine.
        if !boss.dead && boss.visible {
            if boss.state == crate::match_::Phase::Active {
                self.crabs(dt, boss, squad); // JS L60
                if boss.move_rec.is_some() {
                    self.run_move(dt, boss, squad.player); // JS L61
                } else if boss.anim == 2 {
                    // JS L62: roaring — hold.
                } else if boss.pending_phase > boss.phase {
                    self.roar(boss); // JS L63
                } else {
                    self.idle(dt, boss, squad); // JS L64
                }
            } else if boss.move_rec.is_some() {
                // JS L65: match left `playing` mid-move — drop it.
                boss.move_rec = None;
                boss.stunned = false;
            }
        }
        // JS L67: stun is recomputed every tick from the move record.
        let stunned = boss.move_rec.as_ref().is_some_and(|m| {
            rec_p_f64(m, "stun") != 0.0 && move_phase_at(m, bt - rec_t0(m)) == Some("rec")
        });
        boss.stunned = stunned;
        // JS L69-70: ease onto the floor.
        let fy = f64::from(boss.nav.floor_at(boss.pos.x, boss.pos.z));
        boss.pos.y += ((fy - f64::from(boss.pos.y)) * 1.0f64.min(f64::from(dt) * 5.0)) as f32;
        // JS L71: back-compute vel/turn for snapshots.
        if dt > 0.0 {
            boss.vel = glam::vec3((boss.pos.x - px) / dt, 0.0, (boss.pos.z - pz) / dt);
            boss.turn = (angle_diff(f64::from(pyaw), f64::from(boss.yaw)) / f64::from(dt)) as f32;
        }
    }

    /// JS L74-81 `_roar()`.
    fn roar(&mut self, boss: &mut Boss<'_>) {
        boss.phase = boss.pending_phase;
        boss.pending_phase = 0;
        boss.anim = 2;
        boss.anim_t0 = boss.bt;
        boss.invuln = true;
        self.path = None;
        self.plan = None;
        if boss.phase >= 2 {
            self.cd.crablets = self.cd.crablets.min(2.0);
        }
        if boss.phase >= 3 {
            self.cd.frenzy = self.cd.frenzy.min(3.0);
        }
    }

    /// JS L84-105 `_idle(dt)`.
    fn idle(&mut self, dt: f32, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>) {
        self.retarget -= f64::from(dt);
        self.goal_t -= f64::from(dt);
        self.cool -= f64::from(dt);
        // JS L87-88: re-acquire on the retarget timer or a lost target.
        if self.retarget <= 0.0 || self.target_actor(boss, squad).is_none() {
            self.pick_target(boss, squad);
        }
        let t = self.target_actor(boss, squad);
        // JS L89-101: face + start the planned move.
        if self.cool <= 0.0
            && let Some(ti) = t
        {
            if self.plan.is_none() {
                self.plan = self.choose(boss, squad, ti); // JS L90
                if let Some(p) = self.plan.as_mut() {
                    p.t = 0.0;
                }
            }
            if let Some(mut p) = self.plan.take() {
                p.t += f64::from(dt); // JS L93
                if p.face.is_none() {
                    self.start(boss, squad, &p, ti); // JS L95
                    return;
                }
                let want = if p.face_fn {
                    self.aim_at(boss, squad, ti) // JS L96 faceFn()
                } else {
                    p.face.unwrap()
                };
                let dy = angle_diff(f64::from(boss.yaw), want);
                // JS L98: face thresholds 0.16 / 0.6 (JS inline literals).
                if dy.abs() < 0.16 || (p.id != MoveId::Charge && p.t > 1.2 && dy.abs() < 0.6) {
                    self.start(boss, squad, &p, ti);
                    return;
                }
                // JS L99: give up when blocked or after 2.4 s of facing.
                if !Self::turn(boss, want, dt) || p.t > 2.4 {
                    self.plan = None;
                    self.cool = 0.3;
                    self.last = Some(p.id);
                } else {
                    self.plan = Some(p);
                }
                self.speed = 0.0;
                return;
            }
        }
        self.walk(dt, boss, squad, t); // JS L104
    }

    /// JS `targetActor()` (boss.js L153) — entity-side lookup by id among the
    /// squad actors (`(a.nid ?? a.slot) === target && a.alive`).
    fn target_actor(&self, boss: &Boss, squad: &BossSquad<'_>) -> Option<usize> {
        if boss.target < 0 {
            return None;
        }
        squad
            .ids
            .iter()
            .enumerate()
            .find(|(i, id)| **id == boss.target && squad.actors[*i].alive)
            .map(|(i, _)| i)
    }

    /// JS L193 `aimAt()` — yaw from the boss to the target actor.
    fn aim_at(&self, boss: &Boss, squad: &BossSquad<'_>, ti: usize) -> f64 {
        let t = &squad.actors[ti];
        (f64::from(t.pos.x) - f64::from(boss.pos.x))
            .atan2(f64::from(t.pos.z) - f64::from(boss.pos.z))
    }

    /// JS L107-120 `_pickTarget()`.
    fn pick_target(&mut self, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>) {
        self.retarget = 3.0 + f64::from(self.rnd.next_f32()) * 2.5; // JS L109
        let mut best: Option<usize> = None;
        let mut bs = f64::NEG_INFINITY;
        for i in 0..squad.actors.len() {
            let a = &squad.actors[i];
            // JS L112: skip dead / non-player / airborne / pad-sitting actors.
            if !a.alive
                || a.team != 0
                || a.super_jump.is_some()
                || boss.nav.in_pad(a.pos.x, a.pos.z, 3.0)
            {
                continue;
            }
            let k = squad.ids[i];
            let d = (f64::from(a.pos.x) - f64::from(boss.pos.x))
                .hypot(f64::from(a.pos.z) - f64::from(boss.pos.z));
            // JS L114: score — distance, damage attribution, attention, noise.
            let s = -d * 0.1 + self.dmg_by.get(&k).copied().unwrap_or(0.0) * 0.004
                - self.attn.get(&k).copied().unwrap_or(0.0) * 1.4
                + f64::from(self.rnd.next_f32()) * 1.3;
            if s > bs {
                bs = s;
                best = Some(i);
            }
        }
        // JS L117: attention decays ×0.8 per retarget.
        for v in self.attn.values_mut() {
            *v *= 0.8;
        }
        if let Some(i) = best {
            let k = squad.ids[i];
            // JS L118.
            *self.attn.entry(k).or_insert(0.0) += 1.0;
            if boss.target != k {
                self.goal_t = 0.0;
            }
            boss.target = k;
        } else {
            boss.target = -1; // JS L119
        }
    }

    /// JS L122-143 `_walk(dt, t)`.
    fn walk(&mut self, dt: f32, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>, t: Option<usize>) {
        let ph = boss.phase as usize;
        // JS L124: re-pick when the goal expires or the path ran out early.
        let path_exhausted = self
            .path
            .as_ref()
            .is_some_and(|p| self.pi >= p.len() && self.goal_t < 1.5);
        if self.goal.is_none() || self.goal_t <= 0.0 || path_exhausted {
            self.pick_goal(boss, squad, t);
        }
        let mut want = f64::from(boss.yaw);
        let mut go = false;
        if let Some(path) = self.path.as_ref()
            && self.pi < path.len()
        {
            // JS L127-131: advance the waypoint cursor (1.1 m radius, JS
            // inline literal).
            let mut pi = self.pi;
            let mut wp = path[pi];
            while (f64::from(wp[0]) - f64::from(boss.pos.x))
                .hypot(f64::from(wp[1]) - f64::from(boss.pos.z))
                < 1.1
                && pi < path.len() - 1
            {
                pi += 1;
                wp = path[pi];
            }
            let d = (f64::from(wp[0]) - f64::from(boss.pos.x))
                .hypot(f64::from(wp[1]) - f64::from(boss.pos.z));
            if pi >= path.len() - 1 && d < 1.1 {
                pi = path.len(); // JS L130: arrived.
            } else {
                want = (f64::from(wp[0]) - f64::from(boss.pos.x))
                    .atan2(f64::from(wp[1]) - f64::from(boss.pos.z));
                go = true;
            }
            self.pi = pi;
        }
        // JS L133: no path — head straight at the target.
        if !go && let Some(ti) = t {
            want = self.aim_at(boss, squad, ti);
        }
        let dy = angle_diff(f64::from(boss.yaw), want);
        Self::turn(boss, want, dt); // JS L135
        // JS L136-137: speed ramps toward `vmax` (-4·dt / +3·dt clamp).
        let vmax = f64::from(boss.tb.brain.speed[ph])
            * if go {
                clampf(1.0 - dy.abs() / 0.9, 0.0, 1.0)
            } else {
                0.0
            };
        self.speed += clampf(vmax - self.speed, -4.0 * f64::from(dt), 3.0 * f64::from(dt));
        if self.speed > 0.01 {
            let nx = f64::from(boss.pos.x) + f64::from(boss.yaw).sin() * self.speed * f64::from(dt);
            let nz = f64::from(boss.pos.z) + f64::from(boss.yaw).cos() * self.speed * f64::from(dt);
            if boss.nav.pose_ok(nx as f32, nz as f32, boss.yaw) {
                boss.pos.x = nx as f32;
                boss.pos.z = nz as f32;
                self.stuck_t = 0.0;
            } else {
                // JS L141: stuck for 0.8 s — drop the goal.
                self.speed = 0.0;
                self.stuck_t += f64::from(dt);
                if self.stuck_t > 0.8 {
                    self.stuck_t = 0.0;
                    self.goal_t = 0.0;
                    self.path = None;
                }
            }
        }
    }

    /// JS L146-164 `_turn(want, dt)` — yaw toward `want`, or a room-field
    /// side-step when the turn would clip geometry.
    fn turn(boss: &mut Boss<'_>, want: f64, dt: f32) -> bool {
        let dy = angle_diff(f64::from(boss.yaw), want);
        if dy.abs() < 1e-3 {
            return true;
        }
        let ph = boss.phase as usize;
        let rate = f64::from(boss.tb.brain.turn[ph]) * f64::from(dt);
        let ny = clampf(dy, -rate, rate) + f64::from(boss.yaw);
        if boss.nav.turn_ok(boss.pos.x, boss.pos.z, ny as f32) {
            boss.yaw = ny as f32;
            return true;
        }
        // JS L151: room score = wallClear + min(3, floorClear)·0.5.
        let here = f64::from(boss.nav.wall_clear(boss.pos.x, boss.pos.z))
            + f64::from(boss.nav.floor_clear(boss.pos.x, boss.pos.z)).min(3.0) * 0.5;
        // JS L152-159: 8-direction strafe at half-speed·0.55·dt.
        let r = f64::from(boss.tb.brain.speed[ph]) * 0.55 * f64::from(dt);
        let mut best: Option<(f32, f32)> = None;
        let mut bs = here + 1e-3;
        for k in 0..8 {
            let a = (k as f64 / 8.0) * TAU;
            let x = (f64::from(boss.pos.x) + a.sin() * r) as f32;
            let z = (f64::from(boss.pos.z) + a.cos() * r) as f32;
            if !boss.nav.turn_ok(x, z, boss.yaw) {
                continue;
            }
            let sc = f64::from(boss.nav.wall_clear(x, z))
                + f64::from(boss.nav.floor_clear(x, z)).min(3.0) * 0.5;
            if sc > bs {
                bs = sc;
                best = Some((x, z));
            }
        }
        let Some((bx, bz)) = best else {
            return false; // JS L160
        };
        boss.pos.x = bx;
        boss.pos.z = bz;
        if boss.nav.turn_ok(bx, bz, ny as f32) {
            boss.yaw = ny as f32;
        }
        true // JS L163
    }

    /// JS L166-183 `_pickGoal(t)`.
    fn pick_goal(&mut self, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>, t: Option<usize>) {
        self.goal_t = 3.0 + f64::from(self.rnd.next_f32()) * 2.5; // JS L168
        let (pad_x, pad_z) = boss.nav.pad_center(0);
        let mut best: Option<[f32; 2]> = None;
        let mut bs = f64::NEG_INFINITY;
        // JS L170-177: 16 random floor samples, scored by openness + target
        // ring preference (PREF[tb.brain.pref]) − pad penalty.
        for _ in 0..16 {
            let [x, z] = boss.nav.random_point(&mut || self.rnd.next_f32());
            let ci = boss.nav.cell(x, z);
            let mut s = f64::from(boss.nav.dw_at(ci)).min(6.0) * 0.7
                + f64::from(boss.nav.dv_at(ci)).min(4.0) * 0.4
                - (f64::from(x) - f64::from(boss.pos.x))
                    .hypot(f64::from(z) - f64::from(boss.pos.z))
                    * 0.07
                + f64::from(self.rnd.next_f32()) * 1.5;
            if let Some(ti) = t {
                let a = &squad.actors[ti];
                s -= ((f64::from(x) - f64::from(a.pos.x)).hypot(f64::from(z) - f64::from(a.pos.z))
                    - f64::from(boss.tb.brain.pref[boss.phase as usize]))
                .abs();
            } else if let Some([hx, hz]) = boss.home {
                s -= (f64::from(x) - f64::from(hx)).hypot(f64::from(z) - f64::from(hz)) * 0.2;
            }
            // JS L176: pad penalty (18 m radius, −12, JS inline literals).
            if (f64::from(x) - f64::from(pad_x)).hypot(f64::from(z) - f64::from(pad_z)) < 18.0 {
                s -= 12.0;
            }
            if s > bs {
                bs = s;
                best = Some([x, z]);
            }
        }
        let Some(b) = best else {
            return;
        };
        self.goal = Some(b);
        self.path = boss.nav.path(boss.pos.x, boss.pos.z, b[0], b[1]);
        self.pi = 1; // JS L182
    }

    /// JS L186-213 `_choose(t)` — weighted move pick.
    fn choose(
        &mut self,
        boss: &mut Boss<'_>,
        squad: &mut BossSquad<'_>,
        ti: usize,
    ) -> Option<Plan> {
        let ph = boss.phase as usize;
        // JS L188-189: distance measured from a point 2 m in front of the boss.
        let fx = f64::from(boss.pos.x) + f64::from(boss.yaw).sin() * 2.0;
        let fz = f64::from(boss.pos.z) + f64::from(boss.yaw).cos() * 2.0;
        let t = &squad.actors[ti];
        let d = (f64::from(t.pos.x) - fx).hypot(f64::from(t.pos.z) - fz);
        // JS L190-192: nearby / far kid counts — measured from the boss body
        // (JS L191-192 use `b.pos`, not the 2 m ahead point).
        let mut near = 0usize;
        let mut far = 0usize;
        for a in squad.actors.iter() {
            if !a.alive || a.team != 0 || boss.nav.in_pad(a.pos.x, a.pos.z, 1.0) {
                continue;
            }
            let dd = (f64::from(a.pos.x) - f64::from(boss.pos.x))
                .hypot(f64::from(a.pos.z) - f64::from(boss.pos.z));
            if dd < 10.5 {
                near += 1;
            }
            if dd > 13.0 {
                far += 1;
            }
        }
        let aim_at = self.aim_at(boss, squad, ti);
        let lane = self.lane(boss, aim_at);
        // JS L195-200: the weight table.
        let mut w: Vec<(MoveId, f64)> = vec![
            (
                MoveId::Slam,
                if d < 11.0 {
                    3.0 + near as f64 * 0.6
                } else {
                    0.25
                },
            ),
            (
                MoveId::Barrage,
                1.3 + far as f64 * 0.45 + if d > 12.0 { 1.0 } else { 0.0 },
            ),
            (MoveId::Sweep, if d > 5.0 && d < 17.0 { 2.1 } else { 0.4 }),
            (
                MoveId::Charge,
                if lane.is_some() {
                    if d > 5.5 { 2.2 } else { 1.2 }
                } else {
                    0.0
                },
            ),
        ];
        // JS L201-202: gated moves.
        if ph >= 2 && self.cd.crablets <= 0.0 && boss.crabs.iter().filter(|c| !c.dead).count() < 3 {
            w.push((MoveId::Crablets, 2.4));
        }
        if ph >= 3 && self.cd.frenzy <= 0.0 {
            w.push((MoveId::Frenzy, 1.1 + near as f64 * 0.9));
        }
        // JS L203: the last move is de-weighted ×0.3.
        if let Some(last) = self.last {
            for e in w.iter_mut() {
                if e.0 == last {
                    e.1 *= 0.3;
                }
            }
        }
        let sum: f64 = w.iter().map(|e| e.1).sum();
        let mut r = f64::from(self.rnd.next_f32()) * sum;
        let mut id = MoveId::Barrage;
        for e in &w {
            r -= e.1;
            if r <= 0.0 {
                id = e.0;
                break;
            }
        }
        // JS L207: audit `force` hook.
        if let Some(f) = self.force {
            id = f;
            self.force = None;
            if id == MoveId::Charge && lane.is_none() {
                id = MoveId::Barrage;
            }
        }
        // JS L208-211: plan shapes.
        Some(match id {
            MoveId::Slam | MoveId::Sweep => Plan {
                id,
                face: Some(aim_at),
                face_fn: true,
                lane: None,
                t: 0.0,
            },
            MoveId::Charge => Plan {
                id,
                face: Some(lane.as_ref().map(|l| l.yaw).unwrap_or(aim_at)),
                face_fn: false,
                lane,
                t: 0.0,
            },
            _ => Plan {
                id,
                face: None,
                face_fn: false,
                lane: None,
                t: 0.0,
            },
        })
    }

    /// JS L215-227 `_lane(yaw)` — best charge corridor from the 13-offset fan.
    fn lane(&self, boss: &Boss<'_>, yaw: f64) -> Option<Lane> {
        let mut best: Option<Lane> = None;
        let mut bs = f64::NEG_INFINITY;
        for o in [
            0.0, 0.25, -0.25, 0.5, -0.5, 0.8, -0.8, 1.1, -1.1, 1.45, -1.45, 1.8, -1.8,
        ] {
            let y = yaw + o;
            if !boss.nav.turn_ok(boss.pos.x, boss.pos.z, y as f32) {
                continue;
            }
            let c = boss.nav.cast(boss.pos.x, boss.pos.z, y as f32, 34.0);
            if f64::from(c.dist) < f64::from(boss.tb.hz.charge_min) {
                continue;
            }
            let s =
                f64::from(c.dist).min(22.0) * 0.2 - o.abs() * 1.6 + if c.wall { 1.0 } else { 0.0 };
            if s > bs {
                bs = s;
                best = Some(Lane {
                    yaw: y,
                    l: f64::from(c.dist),
                    wall: c.wall,
                });
            }
        }
        best
    }

    /// JS L230-296 `_start(P, t)` — builds the move
    /// record (`{id, t0, s, d, p}`, tele clamped by MIN_TELE × pace) and hands
    /// it to the entity (`move_rec` + `hz.add`, spec §2; network publish is
    /// Task 44).
    fn start(&mut self, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>, p: &Plan, ti: usize) {
        let ph = boss.phase as usize;
        let tb: &tuning::Boss = boss.tb;
        // JS L231: `pace = PACE[ph]` — the per-phase tempo scaling tele/act/
        // rec below; the crablets / frenzy cooldowns use the difficulty
        // getter `this.pace` (JS L39) at L281 / L287 instead.
        let pace = f64::from(tb.pace[ph]);
        self.plan = None; // JS L232
        self.speed = 0.0;
        self.path = None;
        let fwx = f64::from(boss.yaw).sin();
        let fwz = f64::from(boss.yaw).cos();
        let y = f64::from(boss.pos.y);
        let m = match p.id {
            MoveId::Slam => tb.moves.slam,
            MoveId::Barrage => tb.moves.barrage,
            MoveId::Sweep => tb.moves.sweep,
            MoveId::Charge => tb.moves.charge,
            MoveId::Crablets => tb.moves.crablets,
            MoveId::Frenzy => tb.moves.frenzy,
        };
        // JS L234-235: tele is floored at MIN_TELE; act/rec scale with pace.
        let tele = f64::from(tb.min_tele).max(f64::from(m.tele) * pace);
        let mut act = f64::from(m.act) * pace;
        let mut rec = f64::from(m.rec) * pace;
        let hz = &tb.hz;
        let mut p_obj: Vec<(String, JVal)> = Vec::new();
        match p.id {
            // JS L237-241: slam — 4.3 m forward, fixed 1.2 s act.
            MoveId::Slam => {
                let x = f64::from(boss.pos.x) + fwx * 4.3;
                let z = f64::from(boss.pos.z) + fwz * 4.3;
                p_obj.push(("x".into(), JVal::Num(wire::r3(x))));
                p_obj.push((
                    "y".into(),
                    JVal::Num(wire::r3(f64::from(boss.nav.floor_at(x as f32, z as f32)))),
                ));
                p_obj.push(("z".into(), JVal::Num(wire::r3(z))));
                // JS L239: `ph >= 3 ? [0, 0.6] : ph >= 2 ? [0, 0.9] : [0]`.
                let rings: Vec<JVal> = if ph >= 3 {
                    vec![JVal::Num(0.0), JVal::Num(0.6)]
                } else if ph >= 2 {
                    vec![JVal::Num(0.0), JVal::Num(0.9)]
                } else {
                    vec![JVal::Num(0.0)]
                };
                p_obj.push(("rings".into(), JVal::Arr(rings)));
                act = 1.2;
            }
            // JS L243-262: barrage — barrel drop points around the kids.
            MoveId::Barrage => {
                let n = [0usize, 4, 5, 7][ph];
                let mut cands: Vec<usize> = Vec::new();
                for (i, a) in squad.actors.iter().enumerate() {
                    if !a.alive
                        || a.team != 0
                        || a.super_jump.is_some()
                        || boss.nav.in_pad(a.pos.x, a.pos.z, hz.barrel_r + 1.0)
                    {
                        continue;
                    }
                    cands.push(i);
                }
                // DEVIATION (declared per spec §5 / task brief): JS L247 sorts
                // with a comparator that draws a fresh `rnd()` *per comparison*
                // — an impure comparator whose outcome (and rnd draw count)
                // depends on the sort algorithm. Rust cannot do that inside
                // `sort_by` without risking panics/inconsistency, so one
                // jitter `(rnd()-0.5)*6` is pre-drawn per candidate and the
                // list is sorted by the pure key `dist + jitter` (descending
                // dist, matching the JS comparator's `dist(a2)-dist(a1)`
                // direction). Records are host-only (D-T42-7), so only Rust's
                // own determinism must hold — it does.
                let jitters: Vec<f64> = cands
                    .iter()
                    .map(|_| (f64::from(self.rnd.next_f32()) - 0.5) * 6.0)
                    .collect();
                let dist = |i: usize| -> f64 {
                    let a = &squad.actors[i];
                    (f64::from(a.pos.x) - f64::from(boss.pos.x))
                        .hypot(f64::from(a.pos.z) - f64::from(boss.pos.z))
                };
                let mut order: Vec<usize> = (0..cands.len()).collect();
                order.sort_by(|&x, &y| {
                    let kx = -dist(cands[x]) + jitters[x];
                    let ky = -dist(cands[y]) + jitters[y];
                    kx.partial_cmp(&ky).unwrap_or(std::cmp::Ordering::Equal)
                });
                let order: Vec<usize> = order.iter().map(|&j| cands[j]).collect();
                let mut pts: Vec<[f64; 4]> = Vec::new();
                let water_y = f64::from(squad.player.water_y);
                let mut k = 0usize;
                while k < n && !order.is_empty() {
                    let a = &squad.actors[order[k % order.len()]];
                    let extra = k >= order.len();
                    let ang = f64::from(self.rnd.next_f32()) * TAU;
                    let off = if extra {
                        3.0 + f64::from(self.rnd.next_f32()) * 2.5
                    } else {
                        0.0
                    };
                    let mut x = f64::from(a.pos.x) + f64::from(a.vel.x) * 0.3 + ang.cos() * off;
                    let mut z = f64::from(a.pos.z) + f64::from(a.vel.z) * 0.3 + ang.sin() * off;
                    let g = f64::from(boss.world.collision.ground_height(
                        x as f32,
                        z as f32,
                        a.pos.y + 1.0,
                        false,
                    ));
                    // JS L253: drop on water/pad — snap onto the target instead.
                    if not_above(g, water_y)
                        || boss.nav.in_pad(x as f32, z as f32, hz.barrel_r + 0.5)
                    {
                        x = f64::from(a.pos.x);
                        z = f64::from(a.pos.z);
                    }
                    let gy = f64::from(boss.world.collision.ground_height(
                        x as f32,
                        z as f32,
                        a.pos.y + 1.0,
                        false,
                    ));
                    if not_above(gy, water_y)
                        || boss.nav.in_pad(x as f32, z as f32, hz.barrel_r + 0.5)
                    {
                        k += 1;
                        continue;
                    }
                    pts.push([
                        wire::r3(x),
                        wire::r3(gy),
                        wire::r3(z),
                        wire::r3(tele + (pts.len() as f64) * f64::from(hz.barrel_gap)),
                    ]);
                    k += 1;
                }
                // JS L258: no usable candidates — postpone the pick.
                if pts.is_empty() {
                    self.cool = 0.5;
                    self.last = Some(MoveId::Barrage);
                    return;
                }
                act = pts[pts.len() - 1][3] - tele + f64::from(hz.barrel_flight) + 0.1;
                p_obj.push((
                    "sx".into(),
                    JVal::Num(wire::r3(f64::from(boss.pos.x) - fwx * 0.6)),
                ));
                p_obj.push(("sy".into(), JVal::Num(wire::r3(y + 5.4))));
                p_obj.push((
                    "sz".into(),
                    JVal::Num(wire::r3(f64::from(boss.pos.z) - fwz * 0.6)),
                ));
                p_obj.push((
                    "b".into(),
                    JVal::Arr(
                        pts.iter()
                            .map(|b| {
                                JVal::Arr(b.iter().copied().map(JVal::Num).collect::<Vec<_>>())
                            })
                            .collect(),
                    ),
                ));
            }
            // JS L263-267: sweep — beam arc centred on the target.
            MoveId::Sweep => {
                let t = &squad.actors[ti];
                let c = (f64::from(t.pos.x) - f64::from(boss.pos.x))
                    .atan2(f64::from(t.pos.z) - f64::from(boss.pos.z));
                let span = [0.0, 1.9, 2.2, 2.5][ph];
                let dir = if self.rnd.next_f32() < 0.5 { 1.0 } else { -1.0 };
                p_obj.push((
                    "ox".into(),
                    JVal::Num(wire::r3(f64::from(boss.pos.x) + fwx * 3.3)),
                ));
                p_obj.push(("oy".into(), JVal::Num(wire::r3(y + 2.3))));
                p_obj.push((
                    "oz".into(),
                    JVal::Num(wire::r3(f64::from(boss.pos.z) + fwz * 3.3)),
                ));
                p_obj.push(("y".into(), JVal::Num(wire::r3(y))));
                p_obj.push(("a0".into(), JVal::Num(wire::r3(c - dir * span / 2.0))));
                p_obj.push(("a1".into(), JVal::Num(wire::r3(c + dir * span / 2.0))));
                p_obj.push(("c".into(), JVal::Num(wire::r3(f64::from(boss.yaw)))));
            }
            // JS L268-277: charge — lane corridor, distance-scaled act time.
            MoveId::Charge => {
                // JS L269: `const L = P.lane || this._lane(b.yaw)` — the lane
                // value itself only gates the move (the record carries the
                // live cast distance, not the plan's lane).
                if p.lane.is_none() && self.lane(boss, f64::from(boss.yaw)).is_none() {
                    self.cool = 0.4;
                    self.last = Some(MoveId::Charge);
                    return;
                }
                let c = boss.nav.cast(boss.pos.x, boss.pos.z, boss.yaw, 34.0);
                if f64::from(c.dist) < f64::from(hz.charge_min) {
                    self.cool = 0.4;
                    self.last = Some(MoveId::Charge);
                    return;
                }
                p_obj.push(("x".into(), JVal::Num(wire::r3(f64::from(boss.pos.x)))));
                p_obj.push(("y".into(), JVal::Num(wire::r3(y))));
                p_obj.push(("z".into(), JVal::Num(wire::r3(f64::from(boss.pos.z)))));
                p_obj.push(("yaw".into(), JVal::Num(wire::r3(f64::from(boss.yaw)))));
                p_obj.push(("L".into(), JVal::Num(wire::r3(f64::from(c.dist)))));
                p_obj.push(("wall".into(), JVal::Num(if c.wall { 1.0 } else { 0.0 })));
                p_obj.push(("v".into(), JVal::Num(f64::from(hz.charge_speed[ph]))));
                p_obj.push(("stun".into(), JVal::Num(1.0)));
                // JS L274-275: `act = chargeTime(p)` on the *rounded* `p.L`
                // (L273), `rec = HZ.stun[ph]`.
                act = charge_time(
                    hz,
                    f64::from(hz.charge_speed[ph]),
                    wire::r3(f64::from(c.dist)),
                );
                rec = f64::from(hz.stun[ph]);
            }
            // JS L278-282: crablets — litter count + cooldown.
            MoveId::Crablets => {
                let n = if ph >= 3 { 5.0 } else { 3.0 };
                p_obj.push(("n".into(), JVal::Num(n)));
                // JS L281: `14 * this.pace` — the DIFFICULTY getter, not PACE[ph].
                self.cd.crablets = 14.0 * self.pace(boss);
            }
            // JS L284-289: frenzy — spin only when there is room.
            MoveId::Frenzy => {
                let roomy = f64::from(boss.nav.wall_clear(boss.pos.x, boss.pos.z)) >= 5.0
                    && f64::from(boss.nav.floor_clear(boss.pos.x, boss.pos.z)) >= 3.9;
                p_obj.push(("x".into(), JVal::Num(wire::r3(f64::from(boss.pos.x)))));
                p_obj.push(("y".into(), JVal::Num(wire::r3(y))));
                p_obj.push(("z".into(), JVal::Num(wire::r3(f64::from(boss.pos.z)))));
                p_obj.push((
                    "rot0".into(),
                    JVal::Num(wire::r3(f64::from(self.rnd.next_f32()) * TAU)),
                ));
                p_obj.push(("spin".into(), JVal::Num(if roomy { 1.0 } else { 0.0 })));
                p_obj.push(("stun".into(), JVal::Num(1.0)));
                // JS L287: `11 * this.pace` — the DIFFICULTY getter, not PACE[ph].
                self.cd.frenzy = 11.0 * self.pace(boss);
            }
        }
        // JS L291: the record — key order matches the JS object literal so
        // the wire bytes are stable.
        // JS L291: `s: (rnd()*1e9) >>> 0` — ToUint32 truncation.
        let s = ((f64::from(self.rnd.next_f32()) * 1e9) as u32) as f64;
        let rec0 = JVal::Obj(vec![
            ("id".into(), JVal::Str(p.id.as_str().to_string())),
            ("t0".into(), JVal::Num(wire::r3(f64::from(boss.bt)))),
            ("s".into(), JVal::Num(s)),
            (
                "d".into(),
                JVal::Arr(vec![
                    JVal::Num(wire::r3(tele)),
                    JVal::Num(wire::r3(act)),
                    JVal::Num(wire::r3(rec)),
                ]),
            ),
            ("p".into(), JVal::Obj(p_obj)),
        ]);
        boss.move_rec = Some(rec0.clone()); // JS L292
        boss.hz_add(rec0); // JS L293
        self.spawned = false; // JS L295
    }

    /// JS L299-327 `_runMove(dt)` — per-phase motion of the live move.
    fn run_move(&mut self, dt: f32, boss: &mut Boss<'_>, player: &tuning::PlayerTuning) {
        let m = match boss.move_rec.clone() {
            Some(m) => m,
            None => return,
        };
        let u = f64::from(boss.bt) - rec_t0(&m);
        let t = move_times(&m);
        // JS L301-306: move finished — clear it, roll the idle cooldown.
        if u >= t.end {
            boss.move_rec = None;
            self.last = rec_id(&m);
            let ph = boss.phase as usize;
            self.cool = (f64::from(boss.tb.brain.cool[ph])
                + f64::from(self.rnd.next_f32()) * f64::from(boss.tb.brain.cool_rnd[ph]))
                * self.pace(boss);
            self.goal_t = 0.0;
            return;
        }
        match rec_id(&m) {
            // JS L308-312: charge — slide along the corridor with the
            // ease-out distance profile.
            Some(MoveId::Charge) => {
                let s = charge_dist(
                    &boss.tb.hz,
                    rec_p_f64(&m, "v"),
                    rec_p_f64(&m, "L"),
                    u,
                    t.tele,
                );
                let yaw = rec_p_f64(&m, "yaw");
                boss.pos.x = (rec_p_f64(&m, "x") + yaw.sin() * s) as f32;
                boss.pos.z = (rec_p_f64(&m, "z") + yaw.cos() * s) as f32;
                boss.yaw = yaw as f32;
            }
            // JS L313-319: sweep — lean toward the beam head.
            Some(MoveId::Sweep) => {
                if u >= t.tele && u <= t.tele + t.act {
                    let c = rec_p_f64(&m, "c");
                    let want = c + angle_diff(c, beam_angle(&m, u)) * 0.5;
                    if boss.nav.turn_ok(boss.pos.x, boss.pos.z, want as f32) {
                        boss.yaw = want as f32;
                    }
                }
            }
            // JS L320-322: frenzy — spin with an ease-in/out ramp.
            Some(MoveId::Frenzy) => {
                if rec_p_f64(&m, "spin") != 0.0 && u >= t.tele && u <= t.tele + t.act {
                    let ramp = (1.0_f64).min(((u - t.tele) / 0.4).min((t.tele + t.act - u) / 0.4));
                    boss.yaw += (f64::from(dt) * 4.2 * ramp) as f32;
                }
            }
            // JS L323-325: crablets — release the litter once, at tele end.
            Some(MoveId::Crablets) if !self.spawned && u >= t.tele => {
                self.spawned = true;
                let n = rec_n(&m) as usize;
                self.spawn_crabs(boss, n, player);
            }
            _ => {}
        }
    }

    /// JS L330-340 `_spawnCrabs(n)`.
    fn spawn_crabs(&mut self, boss: &mut Boss<'_>, n: usize, player: &tuning::PlayerTuning) {
        let bx = -f64::from(boss.yaw).sin();
        let bz = -f64::from(boss.yaw).cos();
        let water_y = f64::from(player.water_y);
        let hz = &boss.tb.hz;
        for i in 0..n {
            let side = (i as f64 - (n as f64 - 1.0) / 2.0) * 1.1;
            let mut x = f64::from(boss.pos.x) + bx * 3.6 + bz * side;
            let mut z = f64::from(boss.pos.z) + bz * 3.6 - bx * side;
            let mut g = f64::from(boss.world.collision.ground_height(
                x as f32,
                z as f32,
                (f64::from(boss.pos.y) + 1.5) as f32,
                false,
            ));
            // JS L335: water/pad fallback — spawn at the boss's feet.
            if not_above(g, water_y) || boss.nav.in_pad(x as f32, z as f32, 0.8) {
                x = f64::from(boss.pos.x) + bz * side;
                z = f64::from(boss.pos.z) - bx * side;
                g = f64::from(boss.pos.y);
            }
            // JS L338: `fx?.burst` is render-side — no sim event.
            let id = self.crab_seq;
            self.crab_seq += 1;
            boss.crabs.push(CrabState {
                id,
                x: x as f32,
                y: g as f32,
                z: z as f32,
                yaw: (f64::from(boss.yaw) + PI + side * 0.4) as f32,
                hp: f64::from(hz.crab_hp) as f32,
                dead: false,
                speed: Some(0.0),
                miss_t: 0.0,
                pop_t: 0.0,
                spawned: false,
                popped: false,
                age: 0.0,
                tgt: -1,
                retgt: 0.0,
            });
        }
        boss.crab_seq = self.crab_seq;
    }

    /// JS L342-375 `_crabs(dt)` — crablet steering toward the kids.
    fn crabs(&mut self, dt: f32, boss: &mut Boss<'_>, squad: &mut BossSquad<'_>) {
        let hz = &boss.tb.hz;
        let ph = boss.phase as usize;
        // JS L343: `HZ.crabSpeed[phase] || HZ.crabSpeed[2]`.
        let sp = if f64::from(hz.crab_speed[ph]) != 0.0 {
            f64::from(hz.crab_speed[ph])
        } else {
            f64::from(hz.crab_speed[2])
        };
        let water_y = f64::from(squad.player.water_y);
        let mut scratch: Vec<u32> = Vec::new();
        let n = boss.crabs.len();
        for ci in 0..n {
            if boss.crabs[ci].dead {
                continue; // JS L345
            }
            boss.crabs[ci].age += dt; // JS L346
            boss.crabs[ci].retgt -= dt;
            // JS L347: lifetime expired — pop without a kill.
            if f64::from(boss.crabs[ci].age) > f64::from(hz.crab_life) {
                let id = boss.crabs[ci].id;
                boss.crab_pop(id, false, Some(squad));
                continue;
            }
            // JS L348-357: retarget every 0.4 s to the nearest kid.
            if boss.crabs[ci].retgt <= 0.0 {
                boss.crabs[ci].retgt = 0.4_f32;
                let (cx, cz) = (f64::from(boss.crabs[ci].x), f64::from(boss.crabs[ci].z));
                let mut best: Option<usize> = None;
                let mut bd = 30.0;
                for (i, a) in squad.actors.iter().enumerate() {
                    if !a.alive || a.team != 0 || boss.nav.in_pad(a.pos.x, a.pos.z, 0.5) {
                        continue;
                    }
                    let d = (f64::from(a.pos.x) - cx).hypot(f64::from(a.pos.z) - cz);
                    if d < bd {
                        bd = d;
                        best = Some(i);
                    }
                }
                boss.crabs[ci].tgt = best.map_or(-1, |i| squad.ids[i]);
            }
            let tgt = boss.crabs[ci].tgt;
            let ti = if tgt >= 0 {
                squad.ids.iter().position(|&id| id == tgt)
            } else {
                None
            };
            // JS L358-359: contact — pop (1.05 m radius, 1.3 m height, JS
            // inline literals).
            if let Some(ti) = ti {
                let t = &squad.actors[ti];
                let c = &boss.crabs[ci];
                if t.alive
                    && (f64::from(t.pos.x) - f64::from(c.x))
                        .hypot(f64::from(t.pos.z) - f64::from(c.z))
                        < 1.05
                    && (f64::from(t.pos.y) - f64::from(c.y)).abs() < 1.3
                {
                    let id = c.id;
                    boss.crab_pop(id, false, Some(squad));
                    continue;
                }
            }
            let mut moved = false;
            // JS L361-372: steer with the 7-offset detour fan.
            if let Some(ti) = ti {
                let t = &squad.actors[ti];
                if t.alive {
                    let want = (f64::from(t.pos.x) - f64::from(boss.crabs[ci].x))
                        .atan2(f64::from(t.pos.z) - f64::from(boss.crabs[ci].z));
                    for o in [0.0, 0.5, -0.5, 1.0, -1.0, 1.6, -1.6] {
                        let yaw = want + o;
                        let nx = f64::from(boss.crabs[ci].x) + yaw.sin() * sp * f64::from(dt);
                        let nz = f64::from(boss.crabs[ci].z) + yaw.cos() * sp * f64::from(dt);
                        if boss.nav.in_pad(nx as f32, nz as f32, 0.8) {
                            continue;
                        }
                        let g = f64::from(boss.world.collision.ground_height(
                            nx as f32,
                            nz as f32,
                            (f64::from(boss.crabs[ci].y) + 0.7) as f32,
                            false,
                        ));
                        // JS L367: reject shallow water, big drop-offs, and
                        // solid geometry at head height.
                        if not_above(g, water_y + 0.3)
                            || g < f64::from(boss.crabs[ci].y) - 1.4
                            || boss.world.collision.point_inside(
                                glam::vec3(nx as f32, (g + 0.3) as f32, nz as f32),
                                0.0,
                                -1,
                                &mut scratch,
                            )
                        {
                            continue;
                        }
                        let c = &mut boss.crabs[ci];
                        c.x = nx as f32;
                        c.z = nz as f32;
                        c.y = (f64::from(c.y)
                            + (g - f64::from(c.y)) * 1.0f64.min(f64::from(dt) * 12.0))
                            as f32;
                        c.yaw = (f64::from(c.yaw)
                            + angle_diff(f64::from(c.yaw), yaw) * 1.0f64.min(f64::from(dt) * 10.0))
                            as f32;
                        moved = true;
                        break;
                    }
                }
            }
            // JS L373: smooth the speed toward the move/stop target.
            let c = &mut boss.crabs[ci];
            let cur = f64::from(c.speed.unwrap_or(0.0));
            c.speed = Some(
                (cur + ((if moved { sp } else { 0.0 }) - cur) * 1.0f64.min(f64::from(dt) * 8.0))
                    as f32,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// TR-43.1 / TR-43.2 behaviour tests (Task 43 batch D)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Actor, SimWorld};
    use crate::boss::hazards::{rec_pts, rec_rings, rec_spin};
    use crate::boss::nav::BossNav;
    use crate::boss::{BossConfig, BossEvent};
    use crate::collision::CollisionWorld;
    use crate::match_::Phase;
    use glam::vec3;

    /// Static team-0 squad fixture on the embedded Tidewater stage (TR-43).
    struct SquadKit {
        actors: Vec<Actor>,
        ids: Vec<i32>,
        remote: Vec<bool>,
        player: tuning::PlayerTuning,
    }

    impl SquadKit {
        fn new() -> Self {
            Self {
                actors: Vec::new(),
                ids: Vec::new(),
                remote: Vec::new(),
                player: crate::embedded_tuning().player,
            }
        }
        fn add(&mut self, x: f32, y: f32, z: f32, id: i32) {
            let mut a = Actor::new(0, self.actors.len(), &self.player);
            a.pos = vec3(x, y, z);
            self.actors.push(a);
            self.ids.push(id);
            self.remote.push(false);
        }
        fn squad(&mut self) -> BossSquad<'_> {
            BossSquad {
                actors: &mut self.actors,
                ids: &self.ids,
                remote: &self.remote,
                player: &self.player,
            }
        }
    }

    fn harness<T>(
        sim: bool,
        seed: u64,
        difficulty: &str,
        f: impl FnOnce(&mut Boss<'_>, &mut SquadKit) -> T,
    ) -> T {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tb = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tb.boss.nav);
        let mut boss = Boss::new(
            &world,
            &nav,
            &tb.boss,
            BossConfig {
                max_hp: 18000.0,
                difficulty,
                sim,
                seed,
            },
        );
        let mut kit = SquadKit::new();
        f(&mut boss, &mut kit)
    }

    /// The clearer of the two pad directions from the boss's current pose.
    /// Charge needs 6 m of open floor (`HZ.chargeMin`); one side of the
    /// Tidewater middle is furniture-blocked, so aimed tests place their kid
    /// down whichever cast is longer.
    fn open_yaw(boss: &Boss<'_>) -> f64 {
        let (p0x, p0z) = boss.nav.pad_center(0);
        let (p1x, p1z) = boss.nav.pad_center(1);
        let y0 =
            (f64::from(p0x) - f64::from(boss.pos.x)).atan2(f64::from(p0z) - f64::from(boss.pos.z));
        let y1 =
            (f64::from(p1x) - f64::from(boss.pos.x)).atan2(f64::from(p1z) - f64::from(boss.pos.z));
        let c0 = f64::from(boss.nav.cast(boss.pos.x, boss.pos.z, y0 as f32, 34.0).dist);
        let c1 = f64::from(boss.nav.cast(boss.pos.x, boss.pos.z, y1 as f32, 34.0).dist);
        if c1 > c0 { y1 } else { y0 }
    }

    /// Active, post-intro boss with one kid 8 m down the open pad direction,
    /// facing it (JS yaw convention `atan2(dx, dz)`, so aimed moves start with
    /// `dy = 0`). Returns the facing yaw.
    fn armed(boss: &mut Boss<'_>, kit: &mut SquadKit, phase: u32) -> f32 {
        boss.state = Phase::Active;
        boss.visible = true;
        boss.anim = 0;
        boss.phase = phase;
        boss.pending_phase = phase;
        let yaw = open_yaw(boss);
        kit.add(
            boss.pos.x + yaw.sin() as f32 * 8.0,
            boss.pos.y,
            boss.pos.z + yaw.cos() as f32 * 8.0,
            100,
        );
        boss.yaw = yaw as f32;
        yaw as f32
    }

    /// Drive the public update path until the forced move's record is built
    /// (JS L181 `this.force` hook) and return it. The boss is reset to its
    /// home pose facing the kid first, and `force` is re-armed every tick:
    /// JS consumes it in `_choose`, and an abandoned face-up plan (JS L99
    /// give-up) would otherwise fall through to the random pick.
    fn forced_record(boss: &mut Boss<'_>, kit: &mut SquadKit, phase: u32, id: MoveId) -> JVal {
        boss.phase = phase;
        boss.pending_phase = phase;
        boss.move_rec = None;
        if let Some([hx, hz]) = boss.home {
            boss.pos.x = hx;
            boss.pos.z = hz;
        }
        let yaw = open_yaw(boss);
        boss.yaw = yaw as f32;
        if let Some(a) = kit.actors.first_mut() {
            a.pos = vec3(
                boss.pos.x + yaw.sin() as f32 * 8.0,
                boss.pos.y,
                boss.pos.z + yaw.cos() as f32 * 8.0,
            );
        }
        let mut br = boss.brain.take().unwrap();
        br.cool = 0.0;
        br.retarget = -1.0;
        br.plan = None;
        br.last = None;
        br.path = None;
        br.goal = None;
        br.pi = 0;
        br.stuck_t = 0.0;
        br.speed = 0.0;
        let dt = 1.0f32 / 60.0;
        for _ in 0..900 {
            br.force = Some(id);
            br.cool = br.cool.min(0.0);
            br.update(dt, boss, &mut kit.squad());
            if boss.move_rec.is_some() {
                let out = boss.move_rec.clone().unwrap();
                boss.brain = Some(br);
                assert_eq!(
                    rec_id(&out),
                    Some(id),
                    "forced {id:?} produced another record"
                );
                return out;
            }
        }
        panic!(
            "forced move {id:?} never started: plan={:?} cool={} retarget={} target={} anim={} state={:?} pos=({},{}) yaw={}",
            br.plan.as_ref().map(|p| p.id),
            br.cool,
            br.retarget,
            boss.target,
            boss.anim,
            boss.state,
            boss.pos.x,
            boss.pos.z,
            boss.yaw,
        );
    }

    // -- record shapes, per move (JS bossBrain.js L231-291) -----------------

    #[test]
    fn slam_rings_phase1() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 1);
            let m = forced_record(boss, kit, 1, MoveId::Slam);
            // JS bossBrain.js L239: `ph >= 3 ? [0, .6] : ph >= 2 ? [0, .9] : [0]`
            assert_eq!(rec_rings(&m), vec![0.0]);
        });
    }

    #[test]
    fn slam_rings_phase2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Slam);
            assert_eq!(rec_rings(&m), vec![0.0, 0.9]); // JS L239
        });
    }

    #[test]
    fn slam_rings_phase3() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Slam);
            assert_eq!(rec_rings(&m), vec![0.0, 0.6]); // JS L239
        });
    }

    #[test]
    fn slam_act_is_fixed_1_2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Slam);
            // JS bossBrain.js L240: `act = 1.2` — not paced.
            assert_eq!(rec_d(&m, 1), 1.2);
        });
    }

    #[test]
    fn barrage_barrel_count_phase1() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 1);
            let m = forced_record(boss, kit, 1, MoveId::Barrage);
            // JS bossBrain.js L244: `const n = [0, 4, 5, 7][ph]`
            assert_eq!(rec_pts(&m).len(), 4, "phase 1 barrel count");
        });
    }

    #[test]
    fn barrage_barrel_count_phase2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Barrage);
            assert_eq!(rec_pts(&m).len(), 5, "phase 2 barrel count"); // JS L244
        });
    }

    #[test]
    fn barrage_barrel_count_phase3() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Barrage);
            assert_eq!(rec_pts(&m).len(), 7, "phase 3 barrel count"); // JS L244
        });
    }

    #[test]
    fn barrage_pts_are_quadruples_on_the_throw_clock() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Barrage);
            let tele = rec_d(&m, 0);
            let gap = f64::from(boss.tb.hz.barrel_gap);
            let pts = rec_pts(&m);
            for (i, b) in pts.iter().enumerate() {
                // JS bossBrain.js L256: `[r3 x, r3 gy, r3 z, r3(tele + n·barrelGap)]`
                assert!(b[0].is_finite() && b[1].is_finite() && b[2].is_finite());
                assert!(
                    (b[3] - (tele + i as f64 * gap)).abs() < 1.5e-3,
                    "barrel {i} throw time {} vs {}",
                    b[3],
                    tele + i as f64 * gap
                );
            }
        });
    }

    #[test]
    fn sweep_span_phase1() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 1);
            let m = forced_record(boss, kit, 1, MoveId::Sweep);
            // JS bossBrain.js L264: `span = [0, 1.9, 2.2, 2.5][ph]`
            let (a0, a1) = (rec_p_f64(&m, "a0"), rec_p_f64(&m, "a1"));
            assert!(((a1 - a0).abs() - 1.9).abs() < 2.5e-3, "phase 1 span");
        });
    }

    #[test]
    fn sweep_span_phase2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Sweep);
            let (a0, a1) = (rec_p_f64(&m, "a0"), rec_p_f64(&m, "a1"));
            assert!(((a1 - a0).abs() - 2.2).abs() < 2.5e-3, "phase 2 span"); // JS L264
        });
    }

    #[test]
    fn sweep_span_phase3() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Sweep);
            let (a0, a1) = (rec_p_f64(&m, "a0"), rec_p_f64(&m, "a1"));
            assert!(((a1 - a0).abs() - 2.5).abs() < 2.5e-3, "phase 3 span"); // JS L264
        });
    }

    #[test]
    fn sweep_arc_is_symmetric_about_the_target_bearing() {
        harness(true, 5, "normal", |boss, kit| {
            let yaw = armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Sweep);
            // JS bossBrain.js L264-265: a0/a1 straddle `c` by ±span/2, and
            // `c` is recorded as the boss's (already aligned) yaw.
            let (a0, a1) = (rec_p_f64(&m, "a0"), rec_p_f64(&m, "a1"));
            let c = rec_p_f64(&m, "c");
            assert!(((a0 + a1) / 2.0 - c).abs() < 2.5e-3);
            // The kid sits straight ahead of the boss: `c` is the facing yaw.
            assert!((c - f64::from(yaw)).abs() < 1e-3);
        });
    }

    #[test]
    fn charge_fields_phase1() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 1);
            let m = forced_record(boss, kit, 1, MoveId::Charge);
            assert_charge_record(boss, &m, 1);
        });
    }

    #[test]
    fn charge_fields_phase2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Charge);
            assert_charge_record(boss, &m, 2);
        });
    }

    #[test]
    fn charge_fields_phase3() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Charge);
            assert_charge_record(boss, &m, 3);
        });
    }

    /// JS bossBrain.js L273-275: `v: HZ.chargeSpeed[ph]`, `stun: 1`,
    /// `act = chargeTime(p)` on the rounded L, `rec = HZ.stun[ph]`.
    fn assert_charge_record(boss: &Boss<'_>, m: &JVal, ph: u32) {
        let v = f64::from(boss.tb.hz.charge_speed[ph as usize]);
        assert_eq!(rec_p_f64(m, "v"), v, "phase {ph} charge speed");
        assert_eq!(rec_p_f64(m, "stun"), 1.0);
        assert!((rec_d(m, 2) - f64::from(boss.tb.hz.stun[ph as usize])).abs() < 1e-3);
        let l = rec_p_f64(m, "L");
        let want = charge_time(&boss.tb.hz, v, l);
        assert!((rec_d(m, 1) - want).abs() < 1e-3, "phase {ph} charge act");
        assert!(rec_p_f64(m, "wall") == 0.0 || rec_p_f64(m, "wall") == 1.0); // JS L273
    }

    #[test]
    fn crablets_count_phase2() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 2);
            let m = forced_record(boss, kit, 2, MoveId::Crablets);
            // JS bossBrain.js L279: `const n = ph >= 3 ? 5 : 3`
            assert_eq!(rec_n(&m), 3.0, "phase 2 crablet count");
        });
    }

    #[test]
    fn crablets_count_phase3() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Crablets);
            assert_eq!(rec_n(&m), 5.0, "phase 3 crablet count"); // JS L279
        });
    }

    #[test]
    fn frenzy_spin_follows_roominess_and_rot0_is_recorded() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let m = forced_record(boss, kit, 3, MoveId::Frenzy);
            // JS bossBrain.js L285-286: `spin: roomy ? 1 : 0` with
            // `roomy = wallClear >= 5 && floorClear >= 3.9`.
            let roomy = boss.nav.wall_clear(boss.pos.x, boss.pos.z) >= 5.0
                && boss.nav.floor_clear(boss.pos.x, boss.pos.z) >= 3.9;
            assert_eq!(rec_spin(&m), if roomy { 1.0 } else { 0.0 });
            // JS bossBrain.js L286: `rot0: r3(rnd() · 2π)` — present, in range.
            let rot0 = rec_p_f64(&m, "rot0");
            assert!((0.0..std::f64::consts::TAU).contains(&rot0), "rot0 {rot0}");
        });
    }

    // -- tele / act / rec pacing (JS bossBrain.js L231-235, L291) -----------

    #[test]
    fn tele_never_below_min_tele() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let min_tele = f64::from(boss.tb.min_tele); // JS bossBrain.js L10 MIN_TELE
            for id in [
                MoveId::Slam,
                MoveId::Barrage,
                MoveId::Sweep,
                MoveId::Charge,
                MoveId::Crablets,
                MoveId::Frenzy,
            ] {
                boss.move_rec = None;
                let m = forced_record(boss, kit, 3, id);
                assert!(
                    rec_d(&m, 0) >= min_tele - 1e-9,
                    "{id:?} tele {} < MIN_TELE",
                    rec_d(&m, 0)
                );
            }
        });
    }

    #[test]
    fn act_and_rec_scale_with_phase_pace() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            let pace = f64::from(boss.tb.pace[3]); // JS bossBrain.js L231 PACE[ph]
            let m = forced_record(boss, kit, 3, MoveId::Sweep);
            let mv = &boss.tb.moves.sweep;
            // JS bossBrain.js L234-235 + L291: d = [max(MIN_TELE, tele·pace),
            // act·pace, rec·pace]
            assert!((rec_d(&m, 0) - f64::from(mv.tele) * pace).abs() < 1e-3);
            assert!((rec_d(&m, 1) - f64::from(mv.act) * pace).abs() < 1e-3);
            assert!((rec_d(&m, 2) - f64::from(mv.rec) * pace).abs() < 1e-3);
        });
    }

    #[test]
    fn min_tele_floor_clips_short_phase3_tele() {
        harness(true, 5, "normal", |boss, kit| {
            armed(boss, kit, 3);
            // JS bossBrain.js L234: barrage tele 0.85 · PACE[3] 0.78 = 0.663
            // < MIN_TELE 0.7 → the floor wins.
            let m = forced_record(boss, kit, 3, MoveId::Barrage);
            assert!((rec_d(&m, 0) - f64::from(boss.tb.min_tele)).abs() < 1e-3);
        });
    }

    // -- determinism (spec §4 TR-43.1: fixed-seed brain stream) -------------

    #[test]
    fn brain_sequence_is_seed_deterministic() {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tb = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tb.boss.nav);
        let mk = |seed: u64| {
            Boss::new(
                &world,
                &nav,
                &tb.boss,
                BossConfig {
                    max_hp: 18000.0,
                    difficulty: "normal",
                    sim: true,
                    seed,
                },
            )
        };
        let mut b1 = mk(42);
        let mut b2 = mk(42);
        let (padx, padz) = nav.pad_center(0);
        let mut kit_a = SquadKit::new();
        let mut kit_b = SquadKit::new();
        for kit in [&mut kit_a, &mut kit_b] {
            kit.add(padx + 9.0, 0.0, padz + 4.0, 100);
            kit.add(padx - 7.0, 0.0, padz + 11.0, 101);
        }
        for b in [&mut b1, &mut b2] {
            b.state = Phase::Active;
            b.visible = true;
            b.anim = 0;
            if let Some(br) = b.brain.as_mut() {
                br.cool = 0.0; // skip the intro + opening think beat
            }
        }
        let mut recs1: Vec<JVal> = Vec::new();
        let mut recs2: Vec<JVal> = Vec::new();
        let mut seen1: Vec<f64> = Vec::new();
        let mut seen2: Vec<f64> = Vec::new();
        let dt = 1.0f32 / 60.0;
        for _ in 0..1500 {
            b1.update(dt, &mut kit_a.squad());
            collect_records(&mut b1, &mut recs1, &mut seen1);
            b2.update(dt, &mut kit_b.squad());
            collect_records(&mut b2, &mut recs2, &mut seen2);
        }
        assert_eq!(b1.log.moves, b2.log.moves, "move log diverged");
        assert!(
            b1.log.moves.len() >= 3,
            "expected several moves, got {}",
            b1.log.moves.len()
        );
        assert_eq!(recs1, recs2, "record payloads diverged");
        assert_eq!(b1.bt, b2.bt);
        assert_eq!(b1.pos, b2.pos);
        assert_eq!(b1.yaw, b2.yaw);
        let snap = |b: &Boss| -> Vec<(u32, f32, f32, f32, f32, i32)> {
            b.crabs
                .iter()
                .map(|c| (c.id, c.x, c.y, c.z, c.yaw, c.tgt))
                .collect()
        };
        assert_eq!(snap(&b1), snap(&b2), "crablets diverged");
    }

    fn collect_records(boss: &mut Boss<'_>, recs: &mut Vec<JVal>, seen: &mut Vec<f64>) {
        for m in &boss.moves {
            let t0 = rec_t0(m);
            if !seen.contains(&t0) {
                seen.push(t0);
                recs.push(m.clone());
            }
        }
    }

    // -- TR-43.2: crablet lifecycle (JS bossBrain.js L185-230, L335-373) ----

    /// Run the host brain until the crablets litter is out.
    fn spawn_litter(boss: &mut Boss<'_>, kit: &mut SquadKit) -> f64 {
        armed(boss, kit, 2);
        let m = forced_record(boss, kit, 2, MoveId::Crablets);
        let tele = rec_d(&m, 0);
        let dt = 1.0f32 / 60.0;
        let mut guard = 0;
        while boss.crabs.is_empty() && guard < 600 {
            boss.update(dt, &mut kit.squad());
            guard += 1;
        }
        assert!(!boss.crabs.is_empty(), "litter never spawned");
        assert!(f64::from(boss.bt) - rec_t0(&m) >= tele); // JS L196 spawn at tele
        tele
    }

    #[test]
    fn crablets_spawn_at_tele_and_target_the_nearest_kid() {
        harness(true, 9, "normal", |boss, kit| {
            spawn_litter(boss, kit);
            assert_eq!(boss.crabs.len(), 3); // JS bossBrain.js L279 n = 3 (ph 2)
            assert!(boss.crabs.iter().all(|c| c.id > 0));
            // JS L347-357: the 0.4 s retarget roll fires on the next tick.
            boss.update(1.0 / 60.0, &mut kit.squad());
            for c in &boss.crabs {
                assert_eq!(c.tgt, 100, "crab {} targets the only kid", c.id);
                assert!(c.retgt > 0.0 && c.retgt <= 0.4, "retgt {}", c.retgt);
            }
        });
    }

    #[test]
    fn crablet_contact_pops_with_victim_side_burst_damage() {
        harness(true, 9, "normal", |boss, kit| {
            spawn_litter(boss, kit);
            let dt = 1.0f32 / 60.0;
            boss.update(dt, &mut kit.squad()); // arm the retargets
            let ids: Vec<u32> = boss.crabs.iter().map(|c| c.id).collect();
            // JS L358-359: contact = 1.05 m radius, 1.3 m height window.
            let c = &boss.crabs[0];
            let (cx, cy, cz) = (c.x, c.y, c.z);
            kit.actors[0].pos = vec3(cx + 0.4, cy, cz + 0.4);
            let hp0 = f64::from(kit.actors[0].hp);
            boss.update(dt, &mut kit.squad());
            let popped: Vec<u32> = boss.crabs.iter().filter(|c| c.dead).map(|c| c.id).collect();
            assert!(!popped.is_empty(), "no crablet popped on contact");
            let ev = boss.drain_events();
            for id in &popped {
                assert!(
                    ev.iter().any(
                        |e| matches!(e, BossEvent::Bc { id: i, killed: false, .. } if *i == *id)
                    ),
                    "missing Bc for {id}"
                );
                assert!(
                    ev.iter().any(|e| matches!(e, BossEvent::CrabBurst { id: i, killed: false, .. } if *i == *id)),
                    "missing CrabBurst for {id}"
                );
            }
            // JS bossHazards.js L189-194 via the squad: burst damage is
            // victim-side, `crabDmg · mul` per popped crab.
            let mul = f64::from(boss.dmg_mul);
            let want = f64::from(boss.tb.hz.crab_dmg) * mul * popped.len() as f64;
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - want).abs() < 1e-2,
                "burst damage"
            );
            assert!(
                ids.iter()
                    .all(|id| popped.contains(id)
                        || boss.crabs.iter().any(|c| c.id == *id && !c.dead))
            );
        });
    }

    #[test]
    fn crablet_lifetime_pops_at_crab_life() {
        harness(true, 9, "normal", |boss, kit| {
            spawn_litter(boss, kit);
            // JS L346-347: `age > HZ.crabLife` pops without a kill.
            for c in &mut boss.crabs {
                c.age = boss.tb.hz.crab_life;
            }
            let ids: Vec<u32> = boss.crabs.iter().map(|c| c.id).collect();
            boss.update(1.0 / 60.0, &mut kit.squad());
            assert!(boss.crabs.iter().all(|c| c.dead), "lifetime pop");
            let ev = boss.drain_events();
            for id in &ids {
                assert!(
                    ev.iter().any(
                        |e| matches!(e, BossEvent::Bc { id: i, killed: false, .. } if *i == *id)
                    )
                );
            }
            assert!(
                ev.iter()
                    .any(|e| matches!(e, BossEvent::CrabBurst { killed: false, .. }))
            );
        });
    }

    #[test]
    fn popped_crabs_are_cleared_1_2s_after_the_pop() {
        harness(true, 9, "normal", |boss, kit| {
            spawn_litter(boss, kit);
            for c in &mut boss.crabs {
                c.age = boss.tb.hz.crab_life;
            }
            boss.update(1.0 / 60.0, &mut kit.squad());
            assert!(boss.crabs.iter().all(|c| c.dead));
            // JS boss.js L416: `popT += dt; if (popT > 1.2) delete`.
            let dt = 1.0f32 / 60.0;
            for _ in 0..70 {
                boss.update(dt, &mut kit.squad()); // 1.17 s — still shown
            }
            assert!(!boss.crabs.is_empty(), "dead crabs cleared too early");
            for _ in 0..10 {
                boss.update(dt, &mut kit.squad());
            }
            assert!(boss.crabs.is_empty(), "dead crabs not cleared after 1.2 s");
        });
    }

    #[test]
    fn crab_burst_without_squad_defers_to_the_pending_queue() {
        harness(true, 9, "normal", |boss, kit| {
            boss.state = Phase::Active;
            boss.visible = true;
            let (hx, hy, hz) = (boss.pos.x, boss.pos.y, boss.pos.z);
            kit.add(hx + 1.5, hy, hz, 100);
            let hp0 = f64::from(kit.actors[0].hp);
            boss.crabs.push(CrabState {
                id: 501,
                x: hx + 1.5,
                y: hy,
                z: hz,
                yaw: 0.0,
                hp: 1.0,
                dead: false,
                speed: Some(0.0),
                miss_t: 0.0,
                pop_t: 0.0,
                spawned: true,
                popped: false,
                age: 0.0,
                tgt: 100,
                retgt: 0.4,
            });
            // JS bossBrain.js L215-230 / batch C: a burst with no squad (the
            // host drained it before the squad was available) queues pending.
            boss.crab_pop(501, false, None);
            assert_eq!(boss.crab_burst_pending.len(), 1, "burst not queued");
            assert_eq!(
                f64::from(kit.actors[0].hp),
                hp0,
                "no damage before the drain"
            );
            // The next host update drains the queue through `hz_crab_burst`
            // (JS bossHazards.js L189-194) — victim-side damage.
            boss.update(1.0 / 60.0, &mut kit.squad());
            assert!(boss.crab_burst_pending.is_empty());
            let mul = f64::from(boss.dmg_mul);
            let want = f64::from(boss.tb.hz.crab_dmg) * mul;
            assert!(
                (hp0 - f64::from(kit.actors[0].hp) - want).abs() < 1e-2,
                "pending burst damage"
            );
        });
    }
}
