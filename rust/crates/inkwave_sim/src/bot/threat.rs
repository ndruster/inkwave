//! Bot threat AI (Task 47): the JS `BotBrain` "threats" section (bots.js
//! L1260-1615) — enemy Waddle Bombs / Tide Torpedoes hunting us (shoot them
//! down or get out of the way) and enemy Brolly canopies (a held one between
//! us and our duel target: flank round it; a launched one across our way:
//! steer round the nearer edge, or shoot apart one that boxes us in).
//!
//! Upstream mapping (bots.js):
//!   - `THREAT_AI` / `THREAT_STATS` (L52-53)      -> [`set_threat_ai`] /
//!     [`threat_stats`] (A/B switch + engagement counters for tests)
//!   - `_threatScan` (L1272-1290)                 -> [`Bot::threat_scan`]
//!   - `_threatScore` (L1292-1326)                -> [`Bot::threat_score`]
//!   - `_threatCtl` (L1329-1409)                  -> [`Bot::threat_ctl`]
//!   - `_thrEnd` / `_thrDrift` (L1410-1425)       -> [`Bot::thr_end`] /
//!     [`Bot::thr_drift`]
//!   - `_thrEvade` / `_thrSidestep` / `_pickEvade`
//!     (L1428-1474)                               -> [`Bot::thr_evade`] /
//!     [`Bot::thr_sidestep`] / [`Bot::pick_evade`]
//!   - `_meleeWindow` / `_devTrigger` (L1477-1502) -> [`Bot::melee_window`] /
//!     [`Bot::dev_trigger`]
//!   - `_shieldFight` (L1507-1549, fight call L525) -> [`Bot::shield_fight`]
//!   - `_canopyCtl` (L1554-1615, step call L623-625) -> [`Bot::canopy_ctl`]
//!   - stamp threat-steer (L460-470, fight move)    -> [`Bot::stamp_steer`]
//!   - `_dryLine` (L779-783) / `_fatLos` (L1781-1793) -> [`Bot::dry_line`] /
//!     [`Bot::fat_los`]
//!
//! JS `_thrAim` (the shared mutable aim object, L56) becomes a by-value
//! [`AimHold`]; the JS descriptor-object identities (`thr` / `thrCand` /
//! `canRef`) become [`crate::sub::ThreatRef`] / [`ShieldRef`] re-resolved
//! against the live world every frame (a stale ref resolving to `None` is JS
//! `!d.live`).
//!
//! Deviations from JS (recorded for review):
//!   - JS `G.nav.nearest` gate in `_pickEvade` (L1463): the sim bot has the
//!     nav graph in `BotCtx`, so this stays faithful.
//!   - `a.climbing` / `a.superJumpState` / `_climbAim` gates: the sim has no
//!     climb / super jump (M1 deviations), so those terms are `false` /
//!     absent by construction.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::actor::{Actor, ActorInput, Form};
use crate::bot::{Bot, BotCtx, BotMode, angle_diff, wander};
use crate::match_::find_actor;
use crate::sub::{ThreatRef, ThreatState, ThreatView, ink_cost};
use crate::weapon::WeaponKind;
use crate::weapon::WeaponRunner;
use crate::weapon::brolly::{HELD_R, LAUNCH_R, canopy_depth, held_shield};
use glam::Vec3;

/// JS `THREAT_AI = { enabled: true }` (bots.js L52): the tests flip this off
/// for an A/B on the same code.
static THREAT_AI_ENABLED: AtomicBool = AtomicBool::new(true);

/// JS `THREAT_AI.enabled = on`.
pub fn set_threat_ai(on: bool) {
    THREAT_AI_ENABLED.store(on, Ordering::Relaxed);
}

/// JS `THREAT_AI.enabled`.
#[must_use]
pub fn threat_ai_enabled() -> bool {
    THREAT_AI_ENABLED.load(Ordering::Relaxed)
}

/// JS `THREAT_STATS` (bots.js L53): what the bots did (engagements, not
/// frames — `hold_fire` is seconds). Thread-local, test-only: never enters a
/// snapshot or a JSON baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThreatStats {
    /// `noticed` — a new device crossed the reaction-time gate.
    pub noticed: u32,
    /// `shoot` — first shoot/wait-in frame on a device.
    pub shoot: u32,
    /// `evade` — first evade frame on a device.
    pub evade: u32,
    /// `swim` — evaded in squid form through own ink.
    pub swim: u32,
    /// `sidestep` — first sidestep pick on a device.
    pub sidestep: u32,
    /// `gone` — a device we acted on died / left.
    pub gone: u32,
    /// `steer` — first steer pick on a launched canopy.
    pub steer: u32,
    /// `breakWall` — switched to breaking a boxed-in canopy.
    pub break_wall: u32,
    /// `flank` — first flank frame behind a shield.
    pub flank: u32,
    /// `holdFire` — seconds of shots held behind a shield.
    pub hold_fire: f32,
    /// `shieldSub` — subs lobbed over a shield.
    pub shield_sub: u32,
}

thread_local! {
    static THREAT_STATS: RefCell<ThreatStats> = const { RefCell::new(ThreatStats {
        noticed: 0,
        shoot: 0,
        evade: 0,
        swim: 0,
        sidestep: 0,
        gone: 0,
        steer: 0,
        break_wall: 0,
        flank: 0,
        hold_fire: 0.0,
        shield_sub: 0,
    }) };
}

/// Snapshot of the counters (JS `THREAT_STATS` read).
#[must_use]
pub fn threat_stats() -> ThreatStats {
    THREAT_STATS.with(|s| *s.borrow())
}

/// JS test reset of `THREAT_STATS`.
pub fn reset_threat_stats() {
    THREAT_STATS.with(|s| *s.borrow_mut() = ThreatStats::default());
}

/// One `THREAT_STATS[...]++` / `+= dt` site.
pub(crate) fn bump_stats(f: impl FnOnce(&mut ThreatStats)) {
    THREAT_STATS.with(|s| f(&mut s.borrow_mut()));
}

/// JS `_thrAim` (bots.js L56): the aim to hold while drawing a bead on a
/// device / canopy.
#[derive(Debug, Clone, Copy)]
pub struct AimHold {
    /// JS `_thrAim.yaw`.
    pub yaw: f32,
    /// JS `_thrAim.pitch`.
    pub pitch: f32,
    /// JS `_thrAim.dist` (the aim-ray distance, `max(1, d3)`).
    pub dist: f32,
}

/// JS `this._shl` entry identity (brolly.js `shieldOf` descriptors): a held
/// canopy by owner actor index, or a launched one by `Match::launched` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShieldRef {
    /// JS `k.sh` of a brolly runner (`owner` actor-list index).
    Held(usize),
    /// JS `LAUNCHED[i]` (`ctx.launched` index).
    Launched(usize),
}

/// JS `refreshShields` + `shieldOf` (brolly.js L178-190 / L604-627): a
/// per-frame read-only view of one enemy canopy.
#[derive(Debug, Clone, Copy)]
pub struct ShieldView {
    /// This shield's cross-frame identity.
    pub r#ref: ShieldRef,
    /// JS `s.held`.
    pub held: bool,
    /// JS `s.team`.
    pub team: usize,
    /// JS `s.C` — the blocking plane's centre.
    pub c: glam::Vec3,
    /// JS `s.N` — the plane normal (held: the aim facing; launched: the
    /// travel direction).
    pub n: glam::Vec3,
    /// JS `s.R` — the block radius (`HELD_R` / `LAUNCH_R`).
    pub r: f32,
    /// JS `s.pos` — the launched foot position (`Vec3::ZERO` for held, JS
    /// `null`: `_canopyCtl` only reads `pos` on `blocksActors` shields).
    pub pos: glam::Vec3,
    /// JS `s.halfW` — the launched half-width (`LAUNCH_R`; held never blocks
    /// actors).
    pub half_w: f32,
    /// JS `s.planeOff` — the blocking plane's offset along `N` from `pos`
    /// (`-DEPTH * 0.5`).
    pub plane_off: f32,
    /// JS `s.speed` (held `0`; launched `w.launchSpeed`).
    pub speed: f32,
    /// JS `s.hp` (held: the kit's; launched: the canopy's).
    pub hp: f32,
    /// JS `s.live` — a shield failing its gate resolves to `live == false`.
    pub live: bool,
    /// JS `s.left` (held `0`; launched: seconds until it fades).
    pub left: f32,
    /// JS `s.blocksActors` (held `false`; launched `true`).
    pub blocks_actors: bool,
}

/// JS `x || c` on an f32 (0 is falsy).
fn f_or(x: f32, c: f32) -> f32 {
    if x != 0.0 { x } else { c }
}

/// JS `w.inkPerShot || 0` for our weapon table row (`w` = `WEAPONS[kind]`).
fn w_ink_per_shot(ctx: &BotCtx, kind: WeaponKind) -> f32 {
    match kind {
        WeaponKind::Shooter => ctx.weapons.shooter.ink_per_shot,
        WeaponKind::Twins => ctx.weapons.twins.ink_per_shot,
        WeaponKind::Blaster => ctx.weapons.blaster.ink_per_shot,
        WeaponKind::Brolly => ctx.weapons.brolly.ink_per_shot,
        WeaponKind::Bucket => ctx.weapons.bucket.ink_per_shot,
        _ => 0.0,
    }
}

/// JS `w.inkFull || 0` for our weapon table row.
fn w_ink_full(ctx: &BotCtx, kind: WeaponKind) -> f32 {
    match kind {
        WeaponKind::Spinner => ctx.weapons.spinner.ink_full,
        WeaponKind::Charger => ctx.weapons.charger.ink_full,
        WeaponKind::Bow => ctx.weapons.bow.ink_full,
        _ => 0.0,
    }
}

/// JS `w.damage && w.fireInterval && !CHARGES[w.kind] ? w.damage / w.fireInterval : 120`
/// (bots.js L1538): only the shooter / twins / bucket rows carry both fields.
fn w_dps(ctx: &BotCtx, kind: WeaponKind) -> f32 {
    if kind.is_charge() {
        return 120.0;
    }
    let (d, f) = match kind {
        WeaponKind::Shooter => (
            ctx.weapons.shooter.damage,
            ctx.weapons.shooter.fire_interval,
        ),
        WeaponKind::Twins => (ctx.weapons.twins.damage, ctx.weapons.twins.fire_interval),
        WeaponKind::Bucket => (ctx.weapons.bucket.damage, ctx.weapons.bucket.fire_interval),
        _ => return 120.0,
    };
    if d != 0.0 && f != 0.0 { d / f } else { 120.0 }
}

impl Bot {
    /// JS `_shl` fill (`_threatScan` L1274-1277 + brolly.js `shields` /
    /// `refreshShields` L172-190 + `shieldOf` L604-627): rebuild the enemy
    /// canopy views. Deviation: JS refreshes the *membership* only at scan
    /// time (~4 Hz; the pose fields are live getters), the sim rebuilds the
    /// whole list every frame — a canopy appears / disappears one scan
    /// earlier, and a launched index reused after a removal reads as a new
    /// canopy only when the old one left the list first (JS identity was the
    /// object; `ShieldRef::Launched` is the current `ctx.launched` index).
    fn rebuild_shields(&mut self, a: &Actor, ctx: &BotCtx) {
        let mut v = std::mem::take(&mut self.shl);
        v.clear();
        // held canopies, actors order (JS `refreshShields` loop 1). JS reads
        // `a.weaponRunner?.kit` — an actor without a runner is skipped, so zip
        // (not index) keeps the fixture's short `runners` slice faithful.
        for (i, (o, runner)) in ctx.actors.iter().zip(ctx.runners.iter()).enumerate() {
            let Some(kit) = runner.brolly.as_deref() else {
                continue;
            };
            // JS `shieldOf.live` gate: alive kid with a ready, open canopy
            let Some((c, n)) = held_shield(kit, o) else {
                continue;
            };
            v.push(ShieldView {
                r#ref: ShieldRef::Held(i),
                held: true,
                team: o.team,
                c,
                n,
                r: HELD_R,
                pos: Vec3::ZERO,
                half_w: LAUNCH_R,
                plane_off: -canopy_depth() * 0.5,
                speed: 0.0,
                hp: kit.hp.max(0.0),
                live: true,
                left: 0.0,
                blocks_actors: false,
            });
        }
        // launched canopies, `LAUNCHED` order (JS `refreshShields` loop 2)
        for (i, c) in ctx.launched.iter().enumerate() {
            if !c.alive {
                continue;
            }
            v.push(ShieldView {
                r#ref: ShieldRef::Launched(i),
                held: false,
                team: c.team,
                c: c.cur_c,
                n: c.cur_n,
                r: LAUNCH_R,
                pos: c.pos,
                half_w: LAUNCH_R,
                plane_off: -canopy_depth() * 0.5,
                speed: ctx.weapons.brolly.launch_speed,
                hp: c.hp.max(0.0),
                live: true,
                left: (c.life - c.t).max(0.0),
                blocks_actors: true,
            });
        }
        // JS L1277: drop our own team's shields (swap-remove order)
        for i in (0..v.len()).rev() {
            if v[i].team == a.team {
                v.swap_remove(i);
            }
        }
        self.shl = v;
    }

    /// JS `_threatScan` (L1272-1290): pick the device worth acting on and
    /// start its reaction timer when it is new to us.
    fn threat_scan(&mut self, a: &Actor, ctx: &BotCtx) {
        let sb = Self::sub_set();
        let pr = ctx.t.radius;
        let list = ctx.subs.threats(sb, pr);
        let mut best: Option<ThreatRef> = None;
        let mut bs = 0.2f32;
        for d in &list {
            if d.team == a.team {
                continue;
            }
            let s = self.threat_score(a, ctx, d);
            if s > bs {
                bs = s;
                best = Some(d.r#ref());
            }
        }
        // JS L1286: forget devices that died
        self.thr_mem
            .retain(|r, _| ctx.subs.find_threat(*r, sb, pr).is_some());
        // JS L1288: a new one is acted on only after our reaction time
        if let Some(r) = best
            && !self.thr_mem.contains_key(&r)
        {
            let gate = self.t_time + self.diff.reaction * (0.8 + self.rng.next_f32() * 0.7) + 0.05;
            self.thr_mem.insert(r, gate);
            bump_stats(|st| st.noticed += 1);
        }
        self.thr_cand = best;
    }

    /// JS `_threatScore` (L1292-1326): how much a device matters to us now.
    fn threat_score(&self, a: &Actor, ctx: &BotCtx, d: &ThreatView) -> f32 {
        let dx = d.pos.x - a.pos.x;
        let dz = d.pos.z - a.pos.z;
        let dh = dx.hypot(dz);
        let dy = d.pos.y + d.aim_y - (a.pos.y + 0.8);
        if dh > 18.0 || dy.abs() > 6.0 {
            return 0.0;
        }
        let own = find_actor(ctx.actors, a.team, a.slot);
        let mine = d.target.is_some() && d.target == own;
        let st = d.state;
        let v = d.vel;
        if st == ThreatState::Fly {
            if d.ground {
                let t = (v.y + (v.y * v.y + 48.0 * (d.pos.y - a.pos.y)).max(0.0).sqrt()) / 24.0;
                let lx = d.pos.x + v.x * t - a.pos.x;
                let lz = d.pos.z + v.z * t - a.pos.z;
                let sr = d.sense_radius.map_or(7.0, |s| f_or(s, 7.0));
                return if lx.hypot(lz) < sr + 1.0 { 0.6 } else { 0.0 };
            }
            let lr = d.lock_range.map_or(6.5, |s| f_or(s, 6.5));
            return if dh < lr + 3.0 && dx * v.x + dz * v.z < 0.0 {
                0.5
            } else {
                0.0
            };
        }
        if !d.ground {
            if st == ThreatState::Unfold {
                return if mine {
                    3.0
                } else if dh < d.radius {
                    0.8
                } else {
                    0.0
                };
            }
            if st != ThreatState::Launch {
                return 0.0;
            }
            let sp = v.length().max(2.0);
            let d3 = dh.hypot(dy);
            if mine {
                return 4.0 / (0.25 + d3 / sp.max(6.0));
            }
            let tca = -(dx * v.x + dy * v.y + dz * v.z) / (sp * sp);
            if tca > 0.0 && tca < 1.2 {
                let mx = -dx - v.x * tca;
                let my = -dy - v.y * tca;
                let mz = -dz - v.z * tca;
                if mx * mx + my * my + mz * mz < 1.6 {
                    return 2.0;
                }
            }
            return if d3 < d.radius { 0.8 } else { 0.0 };
        }
        if st == ThreatState::Sense {
            let sr = d.sense_radius.map_or(7.0, |s| f_or(s, 7.0));
            return if dh < sr + 0.6 { 0.7 } else { 0.0 };
        }
        if !d.locked {
            return 0.0;
        }
        let eta = (dh - f_or(d.trigger, 1.2)).max(0.0) / f_or(d.speed, 4.0);
        if mine {
            let fade = d.left.is_some_and(|l| l < eta * 0.8);
            return (4.0 / (0.5 + eta)) * if fade { 0.3 } else { 1.0 };
        }
        if dh < d.radius + 0.8 {
            return 1.7;
        }
        let vl = v.x.hypot(v.z);
        if dh < 9.0 && vl > 0.5 && -(dx * v.x + dz * v.z) / (dh * vl) > 0.8 {
            return 1.2 / (0.5 + eta);
        }
        0.0
    }

    /// JS `_thrEnd` (L1410-1418): drop the device, back to the plan.
    fn thr_end(&mut self, a: &Actor, ctx: &BotCtx, gone: bool) {
        if gone && self.thr_acts != 0 {
            bump_stats(|st| st.gone += 1);
        }
        let acted = self.thr_acts != 0;
        self.thr = None;
        self.thr_act = None;
        self.thr_acts = 0;
        self.thr_firing = false;
        // JS L1415-1418: re-plan the route from wherever the dodge left us
        if acted {
            if self.mode == BotMode::Paint && self.goal != usize::MAX && self.path.is_some() {
                let n = ctx.nav.nodes[self.goal].pos;
                self.path_to(ctx, a.pos, a.grounded, n, 0.3);
            } else {
                self.repath = 0.0;
            }
        }
    }

    /// JS `_thrDrift` (L1421-1425): carry on, edging away from it.
    fn thr_drift(&self, d: &ThreatView, a: &Actor, move_v: &mut Vec3, k: f32) {
        let rx = a.pos.x - d.pos.x;
        let rz = a.pos.z - d.pos.z;
        let rl = f_or(rx.hypot(rz), 1.0);
        let x = move_v.x + (rx / rl) * k;
        let z = move_v.z + (rz / rl) * k;
        let l = x.hypot(z);
        if l > 1.0 {
            *move_v = Vec3::new(x / l, 0.0, z / l);
        } else {
            *move_v = Vec3::new(x, 0.0, z);
        }
    }

    /// JS `_thrEvade` (L1428-1438): can't pop it — run, swim, or sidestep.
    #[allow(clippy::too_many_arguments)]
    fn thr_evade(
        &mut self,
        d: &ThreatView,
        dt: f32,
        a: &mut Actor,
        ctx: &BotCtx,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        ttl: f32,
    ) {
        if !d.ground && d.state == ThreatState::Launch && ttl < 0.55 {
            self.thr_sidestep(d, a, ctx, move_v, it, ttl < 0.35);
            return;
        }
        self.thr_ev_t -= dt;
        if self.thr_ev_t <= 0.0 {
            self.thr_ev_t = 0.45 + self.rng.next_f32() * 0.3;
            self.thr_ev_yaw = self.pick_evade(a, ctx, d);
        }
        *move_v = Vec3::new(self.thr_ev_yaw.sin(), 0.0, self.thr_ev_yaw.cos());
        let swim = a.ground_team == 1 && !self.squid_would_drop(a, ctx, *move_v);
        if swim && self.thr_acts & 4 == 0 {
            self.thr_acts |= 4;
            bump_stats(|st| st.swim += 1);
        }
        it.squid = swim;
        if !swim && d.ground && self.thr_los {
            let ff = a.fire_facing().max(0.25);
            a.set_fire_facing(ff);
        }
    }

    /// JS `_thrSidestep` (L1440-1455): step off its line, sticky side.
    fn thr_sidestep(
        &mut self,
        d: &ThreatView,
        a: &Actor,
        ctx: &BotCtx,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        hop: bool,
    ) {
        let v = d.vel;
        let vl = v.x.hypot(v.z);
        let rx = a.pos.x - d.pos.x;
        let rz = a.pos.z - d.pos.z;
        let (px, pz) = if vl > 0.3 {
            (-v.z / vl, v.x / vl)
        } else {
            let rl = f_or(rx.hypot(rz), 1.0);
            (-rz / rl, rx / rl)
        };
        if self.thr_side == 0 {
            // JS L1446: the side we're already on, unless a wall / the sea
            let ax = a.pos.x;
            let ay = a.pos.y;
            let az = a.pos.z;
            let ok = |sg: i32| -> bool {
                let s = sg as f32 * 2.0;
                let fx = ax + px * s;
                let fz = az + pz * s;
                self.dry_line(ctx, ax, ay, az, fx, fz) && self.fat_los(ctx, ax, ay, az, fx, ay, fz)
            };
            let mut s = if rx * px + rz * pz >= 0.0 { 1 } else { -1 };
            if !ok(s) && ok(-s) {
                s = -s;
            }
            self.thr_side = s;
            if self.thr_acts & 8 == 0 {
                self.thr_acts |= 8;
                bump_stats(|st| st.sidestep += 1);
            }
        }
        let side = self.thr_side as f32;
        *move_v = Vec3::new(px * side, 0.0, pz * side);
        it.squid = false;
        if hop && a.grounded && self.jump_cd <= 0.0 && !self.near_water(a, ctx, 1.4) {
            it.jump = true;
            self.jump_cd = 0.9;
        }
    }

    /// JS `_pickEvade` (L1457-1474): a heading to run from it.
    fn pick_evade(&mut self, a: &Actor, ctx: &BotCtx, d: &ThreatView) -> f32 {
        let x0 = a.pos.x;
        let y0 = a.pos.y;
        let z0 = a.pos.z;
        let base = (x0 - d.pos.x).atan2(z0 - d.pos.z);
        let wp = self
            .path
            .as_ref()
            .and_then(|p| p.get(self.pi))
            .map(|&n| ctx.nav.nodes[n].pos);
        let mut best = base;
        let mut bs = f32::NEG_INFINITY;
        for off in [0.0f32, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5, 2.1, -2.1] {
            let yw = base + off;
            let sx = yw.sin();
            let sz = yw.cos();
            let px = x0 + sx * 3.2;
            let pz = z0 + sz * 3.2;
            // JS L1463: never run off the walkable graph
            if ctx.nav.nearest(Vec3::new(px, y0, pz), 1.0, false).is_none() {
                continue;
            }
            if !self.dry_line(ctx, x0, y0, z0, px, pz) || !self.fat_los(ctx, x0, y0, z0, px, y0, pz)
            {
                continue;
            }
            let mut sc = off.cos() * if d.ground { 2.0 } else { 1.0 };
            if !d.ground {
                sc += off.sin().abs() * 1.2;
            }
            let st = ctx
                .paint
                .region_stats(ctx.world.collision, px, y0, pz, 1.3, a.team);
            if st.n != 0 {
                sc += st.own * 1.5;
            }
            if let Some(w) = wp {
                let wx = w.x - x0;
                let wz = w.z - z0;
                let wl = f_or(wx.hypot(wz), 1.0);
                sc += 0.4 * (wx * sx + wz * sz) / wl;
            }
            if !d.ground && !ctx.world.collision.los(d.pos, Vec3::new(px, y0 + 1.0, pz)) {
                sc += 2.0;
            }
            sc += self.rng.next_f32() * 0.3;
            if sc > bs {
                bs = sc;
                best = yw;
            }
        }
        best
    }

    /// JS `_meleeWindow` (L1477-1485): the distance window our melee pops it in.
    fn melee_window(&self, ctx: &BotCtx, kind: WeaponKind, d: &ThreatView) -> (f32, f32) {
        match kind {
            WeaponKind::Roller => {
                if !d.ground {
                    (1.5, 6.0)
                } else if d.state == ThreatState::Walk {
                    (7.3, 9.0)
                } else {
                    (5.0, 6.6)
                }
            }
            WeaponKind::Brush => {
                if !d.ground {
                    (0.8, 4.5)
                } else if d.state == ThreatState::Walk {
                    (99.0, 99.0)
                } else {
                    (0.8, 2.8)
                }
            }
            WeaponKind::Blade => {
                if d.ground {
                    (0.0, 4.0)
                } else {
                    (0.0, 5.5)
                }
            }
            _ => (0.0, ctx.weapon_reach(kind) * 0.95),
        }
    }

    /// JS `_devTrigger` (L1487-1502): the trigger for a shot at a device.
    #[allow(clippy::too_many_arguments)]
    fn dev_trigger(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        wr: &WeaponRunner,
        kind: WeaponKind,
        dist: f32,
        aimed: bool,
        roughly: bool,
        lo: f32,
        hi: f32,
    ) -> bool {
        let in_win = dist >= lo && dist <= hi;
        if kind.is_melee() {
            // JS L1491: a flick / swipe goes where the body faces — squared up?
            let faced = (kind != WeaponKind::Roller && kind != WeaponKind::Brush)
                || angle_diff(a.yaw, self.aim_yaw).abs() < 0.3;
            self.thr_pulse = aimed && faced && in_win && !self.thr_pulse;
            return self.thr_pulse;
        }
        if kind.is_charge() {
            if wr.burst_t() > 0.0 || wr.streaming() {
                return false;
            }
            // JS L1497: a short charge does it (splatling retired; the only
            // other charge kind is the bow, `w.ring1 ?? 0.4 + 0.06`)
            let rel = if kind == WeaponKind::Charger {
                0.12
            } else if kind == WeaponKind::Spinner {
                0.3
            } else {
                ctx.weapons.bow.ring1 + 0.06
            };
            if wr.charging_flag() {
                return !(aimed && wr.charge() >= rel);
            }
            return roughly && in_win && wr.cooldown() <= 0.0;
        }
        aimed && in_win
    }

    /// JS `_dryLine` (L779-783): ground all the way along a straight walk.
    pub(super) fn dry_line(
        &self,
        ctx: &BotCtx,
        x0: f32,
        y0: f32,
        z0: f32,
        x1: f32,
        z1: f32,
    ) -> bool {
        let dx = x1 - x0;
        let dz = z1 - z0;
        let n = (dx.hypot(dz) / 0.45).ceil() as i32;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            if self.wet(ctx, x0 + dx * t, z0 + dz * t, y0) {
                return false;
            }
        }
        true
    }

    /// JS `_fatLos` (L1781-1793): the *body* sees it (shoulder-wide ray pair).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fat_los(
        &self,
        ctx: &BotCtx,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
    ) -> bool {
        let c = ctx.world.collision;
        let dx = bx - ax;
        let dy = by - ay;
        let dz = bz - az;
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        if len < 0.3 {
            return true;
        }
        let ux = dx / len;
        let uz = dz / len;
        let px = -uz;
        let pz = ux;
        for s in [-0.34f32, 0.34] {
            let ox = ax + px * s;
            let oz = az + pz * s;
            let vx = bx - ox;
            let vy = by + 0.45 - ay;
            let vz = bz - oz;
            let vl = (vx * vx + vy * vy + vz * vz).sqrt();
            if c.raycast(
                Vec3::new(ox, ay, oz),
                Vec3::new(vx / vl, vy / vl, vz / vl),
                vl - 0.05,
                false,
            )
            .hit
            {
                return false;
            }
        }
        true
    }

    /// JS `_threatCtl` (L1329-1409): deal with the device we've noticed.
    pub(crate) fn threat_ctl(
        &mut self,
        dt: f32,
        a: &mut Actor,
        ctx: &BotCtx,
        it: &mut ActorInput,
        move_v: &mut Vec3,
    ) -> Option<AimHold> {
        // JS L1330: the A/B switch — off means zero behaviour, zero draws.
        if !threat_ai_enabled() {
            return None;
        }
        self.rebuild_shields(a, ctx);
        let sb = Self::sub_set();
        let pr = ctx.t.radius;
        // JS L1332: the ~4 Hz scan
        self.thr_scan_t -= dt;
        if self.thr_scan_t <= 0.0 {
            self.thr_scan_t = 0.2 + self.rng.next_f32() * 0.1;
            self.threat_scan(a, ctx);
        }
        // JS L1333-1334: ours died
        if let Some(r) = self.thr
            && ctx.subs.find_threat(r, sb, pr).is_none()
        {
            self.thr_end(a, ctx, true);
        }
        let mut d_ref = self.thr;
        // JS L1335-1339: a clearly better one takes over
        if let Some(c) = self.thr_cand
            && Some(c) != d_ref
            && let Some(cv) = ctx.subs.find_threat(c, sb, pr)
        {
            let gate = self.thr_mem.get(&c).copied().unwrap_or(f32::INFINITY);
            let dominated = match d_ref {
                None => true,
                Some(dr) => match ctx.subs.find_threat(dr, sb, pr) {
                    None => true,
                    Some(dv) => {
                        self.threat_score(a, ctx, &cv) > self.threat_score(a, ctx, &dv) * 1.3
                    }
                },
            };
            if self.t_time >= gate && dominated {
                self.thr = Some(c);
                self.thr_act = None;
                self.thr_acts = 0;
                self.thr_side = 0;
                self.thr_ev_t = 0.0;
                self.thr_acq_t = 0.0;
                self.thr_los_t = 0.0;
                let sign = if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
                self.thr_sign_y = sign * (0.5 + self.rng.next_f32() * 0.5);
                self.thr_sign_p = (self.rng.next_f32() - 0.5) * 1.2;
                d_ref = Some(c);
            }
        }
        // JS L1342-1343: a body-owning special plays out; no device: nothing
        let sp = a.special_active.as_deref();
        let spk: Option<&str> = sp.map(|s| {
            if s.kind.is_empty() {
                s.id.as_str()
            } else {
                s.kind.as_str()
            }
        });
        let body_special = matches!(
            spk,
            Some("kraken") | Some("stamp") | Some("crab") | Some("jetpack") | Some("zipcaster")
        );
        let move_only = sp.is_some();
        if d_ref.is_none() || body_special {
            return None;
        }
        let d = ctx.subs.find_threat(d_ref.unwrap(), sb, pr)?;
        let score = self.threat_score(a, ctx, &d);
        if score < 0.15 {
            self.thr_end(a, ctx, false);
            return None;
        }
        self.thr_acq_t += dt;
        let (kind, wr) = ctx.own(self.team, self.slot);
        // JS L1350-1356: eye -> the hit centre, led by the shot's flight time
        let ex = a.pos.x;
        let ey = a.pos.y + 1.1;
        let ez = a.pos.z;
        let hx = d.pos.x;
        let hy = d.pos.y + d.aim_y;
        let hz = d.pos.z;
        let mut dx = hx - ex;
        let mut dy = hy - ey;
        let mut dz = hz - ez;
        let dh = dx.hypot(dz);
        let d3 = dh.hypot(dy);
        let v = d.vel;
        let lt = if kind == WeaponKind::Charger {
            0.0
        } else {
            (0.5f32).min(d3 / ctx.shot_speed(kind))
        };
        dx += v.x * lt;
        dy += v.y * lt;
        dz += v.z * lt;
        let ideal_yaw = dx.atan2(dz);
        let ideal_pitch = if kind == WeaponKind::Roller && d.ground {
            -0.3
        } else {
            dy.atan2(dx.hypot(dz))
        };
        // JS L1357: the 0.1 s line-of-sight sample
        self.thr_los_t -= dt;
        if self.thr_los_t <= 0.0 {
            self.thr_los_t = 0.1;
            self.thr_los = ctx
                .world
                .collision
                .los(Vec3::new(ex, ey + 0.1, ez), Vec3::new(hx, hy + 0.05, hz));
        }
        // JS L1359-1363: can we pop it?
        let melee = kind.is_melee();
        let (lo, hi) = if melee {
            self.melee_window(ctx, kind, &d)
        } else {
            (0.0, ctx.bot_range(kind) * if d.ground { 0.95 } else { 1.0 })
        };
        let ink_ok = a.ink
            > 2.5f32
                .max(w_ink_per_shot(ctx, kind))
                .max(w_ink_full(ctx, kind) * 0.25);
        let can_shoot = !move_only && d.shootable && self.thr_los && d3 >= lo && d3 <= hi && ink_ok;
        let wait_in = !can_shoot
            && !move_only
            && d.ground
            && d.shootable
            && self.thr_los
            && ink_ok
            && d3 > hi
            && d3 < hi + 4.0
            && hi - f_or(d.trigger, 1.2) > 2.5;
        // JS L1364-1365: how soon it gets to us
        let own = find_actor(ctx.actors, a.team, a.slot);
        let mine = d.target.is_some() && d.target == own;
        let ttl = if !d.ground && d.state == ThreatState::Launch {
            d3 / v.length().max(2.0)
        } else {
            9.0
        };
        let eta = if d.ground {
            (dh - f_or(d.trigger, 1.2)).max(0.0) / f_or(d.speed, 4.0)
        } else {
            ttl
        };
        // JS L1367-1372: a duel close by outranks a device not about to arrive
        let t_idx = self
            .target
            .and_then(|(team, slot)| find_actor(ctx.actors, team, slot));
        if let Some(ti) = t_idx
            && self.mode == BotMode::Fight
            && self.see_timer > 0.0
        {
            let t = &ctx.actors[ti];
            let td = (a.pos.x - t.pos.x).hypot(a.pos.z - t.pos.z);
            let tkind = ctx.kinds[ti];
            let pd = if td < 4.0 {
                2.4
            } else if td < ctx.bot_range(kind) * 0.9 || td < ctx.weapon_reach(tkind) * 0.9 {
                1.5
            } else {
                0.5
            };
            if score < pd {
                if mine || dh < 4.0 {
                    self.thr_drift(&d, a, move_v, 0.6);
                }
                return None;
            }
        }
        // JS L1374-1375: far off and not arriving: carry on, edge away
        let urgent = if d.ground {
            eta < 2.2 || dh < f_or(d.radius, 3.0) + 0.5
        } else {
            d.state == ThreatState::Launch || d.state == ThreatState::Unfold
        };
        if !can_shoot && !wait_in && !urgent {
            self.thr_drift(&d, a, move_v, 0.5);
            return None;
        }
        // JS L1376-1378
        let shoot = can_shoot || wait_in;
        let bit = if shoot { 1 } else { 2 };
        if self.thr_acts & bit == 0 {
            self.thr_acts |= bit;
            if shoot {
                bump_stats(|st| st.shoot += 1);
            } else {
                bump_stats(|st| st.evade += 1);
            }
        }
        self.thr_act = Some(if shoot { "shoot" } else { "evade" });
        // JS L1379-1392: the footwork
        let mut dodge = false;
        if shoot {
            it.squid = false;
            let ff = a.fire_facing().max(0.25);
            a.set_fire_facing(ff);
            let rx = a.pos.x - d.pos.x;
            let rz = a.pos.z - d.pos.z;
            let rl = f_or(rx.hypot(rz), 1.0);
            dodge = !d.ground
                && d.state == ThreatState::Launch
                && ttl < if kind.is_charge() { 0.8 } else { 0.45 };
            if dodge {
                self.thr_sidestep(&d, a, ctx, move_v, it, ttl < 0.3);
            } else if wait_in || (d.ground && kind == WeaponKind::Roller) {
                *move_v = Vec3::ZERO;
            } else if d.ground && melee {
                *move_v = Vec3::new(rx / rl * 0.35, 0.0, rz / rl * 0.35);
            } else if d.ground && dh < 4.5 {
                *move_v = Vec3::new(rx / rl, 0.0, rz / rl);
            } else {
                *move_v *= 0.25;
            }
            if kind.is_charge() && wr.charging_flag() && !dodge {
                *move_v *= 0.4;
            }
        } else {
            self.thr_evade(&d, dt, a, ctx, move_v, it, ttl);
        }
        self.no_prog = 0.0;
        self.best_d = f32::INFINITY;
        // JS L1395-1398: the aim to hold (the usual error, a little tighter)
        let e = self.diff.aim_error * 0.7;
        let acq = (-self.thr_acq_t / (0.12f32.max(self.diff.reaction * 0.9))).exp();
        let aim = AimHold {
            yaw: ideal_yaw
                + e * (0.7 * wander(self.t_time * 1.7 + self.ph1) + 2.2 * acq * self.thr_sign_y),
            pitch: ideal_pitch
                + e * 0.6
                    * (0.7 * wander(self.t_time * 2.1 + self.ph2) + 1.5 * acq * self.thr_sign_p),
            dist: 1.0f32.max(d3),
        };
        // JS L1400-1401: the actual aim ray's offset from the ideal
        let off = angle_diff(self.aim_yaw, ideal_yaw).hypot(self.aim_pitch - ideal_pitch);
        let tol = 0.035f32.max((if d.ground { 0.24f32 } else { 0.28f32 }).atan2(d3))
            * if self.thr_firing { 2.2 } else { 1.3 };
        if move_only {
            it.squid = false;
            return None;
        }
        it.fire = if can_shoot {
            self.dev_trigger(a, ctx, wr, kind, d3, off < tol, off < tol * 4.0, lo, hi)
        } else {
            kind.is_charge() && wr.charging_flag() && !it.squid
        };
        if dodge && kind.is_charge() && wr.charging_flag() && wr.charge() >= 0.12 {
            it.fire = false;
        }
        self.thr_firing = it.fire && off < tol;
        if it.fire {
            it.squid = false;
        }
        it.sub = false;
        self.bomb_aim = false;
        Some(aim)
    }

    /// JS `_shieldFight` (L1507-1549, fight call L525): an enemy canopy is
    /// between us and our duel target — flank round it, hold the shots that
    /// would only ink it, lob over it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shield_fight(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        t: &Actor,
        dist: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        dt: f32,
        kind: WeaponKind,
        wr: &WeaponRunner,
    ) {
        // JS L1508-1510: switch off / no shields: decay the block timer
        if !threat_ai_enabled() || self.shl.is_empty() {
            self.block_t = 0.0;
            return;
        }
        let ex = a.pos.x;
        let ey = a.pos.y + 1.1;
        let ez = a.pos.z;
        let tx = t.pos.x;
        let ty = t.visual_y() + if t.form == Form::Squid { 0.3 } else { 0.85 };
        let tz = t.pos.z;
        // JS L1512-1522: the first shield the eye -> target segment pierces
        let mut blk: Option<&ShieldView> = None;
        for s in &self.shl {
            if !s.live {
                continue;
            }
            let p0 = (ex - s.c.x) * s.n.x + (ey - s.c.y) * s.n.y + (ez - s.c.z) * s.n.z;
            let p1 = (tx - s.c.x) * s.n.x + (ty - s.c.y) * s.n.y + (tz - s.c.z) * s.n.z;
            if (p0 > 0.0) == (p1 > 0.0) || p0 == p1 {
                continue;
            }
            let k = p0 / (p0 - p1);
            let qx = ex + (tx - ex) * k - s.c.x;
            let qy = ey + (ty - ey) * k - s.c.y;
            let qz = ez + (tz - ez) * k - s.c.z;
            if qx * qx + qy * qy + qz * qz > (s.r + 0.12) * (s.r + 0.12) {
                continue;
            }
            blk = Some(s);
            break;
        }
        let Some(blk) = blk else {
            // JS L1524-1526: nothing between us now — unwind, then re-flank fresh
            if self.block_t > 0.0 {
                self.block_t = (self.block_t - dt * 3.0).max(0.0);
                if self.block_t == 0.0 {
                    self.flank_side = 0;
                }
            }
            return;
        };
        let blk = *blk; // copy: the per-frame scratch below mutates `self`
        if self.block_t == 0.0 {
            bump_stats(|st| st.flank += 1);
        }
        self.block_t += dt;
        // JS L1528-1536: slide along the shield, sticky side, probe 1.8 m
        let sx = -blk.n.z;
        let sz = blk.n.x;
        let hl = 0.3f32.max((tx - ex).hypot(tz - ez));
        let nx = (tx - ex) / hl;
        let nz = (tz - ez) / hl;
        if self.flank_side == 0 {
            self.flank_side = if (ex - blk.c.x) * sx + (ez - blk.c.z) * sz >= 0.0 {
                1
            } else {
                -1
            };
            self.flank_t = 0.0;
        }
        let mut lx = -nz;
        let mut lz = nx;
        if (lx * sx + lz * sz) * (self.flank_side as f32) < 0.0 {
            lx = -lx;
            lz = -lz;
        }
        self.flank_t -= dt;
        if self.flank_t <= 0.0 {
            self.flank_t = 0.5;
            let fx = ex + lx * 1.8;
            let fz = ez + lz * 1.8;
            let ok = self.dry_line(ctx, ex, a.pos.y, ez, fx, fz)
                && self.fat_los(ctx, ex, a.pos.y, ez, fx, a.pos.y, fz);
            if !ok {
                self.flank_side = -self.flank_side;
                lx = -lx;
                lz = -lz;
            }
        }
        let inward = if dist > 6.0 {
            0.45
        } else if dist < 2.5 {
            -0.35
        } else {
            0.1
        };
        let mx = lx + nx * inward;
        let mz = lz + nz * inward;
        let ml = f_or(mx.hypot(mz), 1.0);
        *move_v = Vec3::new(mx / ml, 0.0, mz / ml);
        // JS L1537-1544: hold the shots that would only ink the canopy
        let dps = w_dps(ctx, kind);
        let through = blk.hp <= dps * 1.3
            || self.block_t > 2.5
            || (!blk.held && blk.left < 0.5)
            || (self.can_ref == Some(blk.r#ref) && self.can_side == 2);
        if !through && self.block_t > 0.15 {
            if kind.is_charge() {
                it.fire = it.fire || wr.charging_flag();
            } else if it.fire {
                it.fire = false;
                bump_stats(|st| st.hold_fire += dt);
            }
        }
        // JS L1545-1548: lob a bomb over it
        let sub = a.sub;
        let blocked =
            find_actor(ctx.actors, a.team, a.slot).is_some_and(|i| ctx.subs.kit_blocked(sub, i));
        if self.block_t > 0.5
            && self.bomb_cd.unwrap_or(0.0) <= 0.0
            && sub.is_thrown()
            && a.ink >= ink_cost(sub, Self::sub_set()) + 8.0
            && dist > 3.0
            && dist < 13.0
            && !blocked
            && self.rng.next_f32() < dt * 2.5
        {
            it.sub = true;
            self.bomb_aim = true;
            self.bomb_cd = Some(5.0 + self.rng.next_f32() * 4.0);
            bump_stats(|st| st.shield_sub += 1);
        }
    }

    /// JS `_canopyCtl` (L1554-1615, step call L623-625): a launched canopy
    /// boxes us in — round the nearer edge, or shoot the one that won't move.
    pub(crate) fn canopy_ctl(
        &mut self,
        dt: f32,
        a: &Actor,
        ctx: &BotCtx,
        it: &mut ActorInput,
        move_v: &mut Vec3,
        free: bool,
    ) -> Option<AimHold> {
        // JS L1555-1557: switch off; climbing / super jump absent in the sim
        if !threat_ai_enabled() {
            return None;
        }
        if self.shl.is_empty() || a.special_active.is_some() {
            self.can_ref = None;
            return None;
        }
        let mut ml = move_v.x.hypot(move_v.z);
        // JS L1558-1575: the first canopy our line of travel runs into
        let mut hit: Option<ShieldView> = None;
        let mut h_along = 0.0f32;
        let mut h_side = 1i32;
        let mut h_w = 1.0f32;
        for s in &self.shl {
            if !s.blocks_actors || !s.live {
                continue;
            }
            let n = s.n;
            let tx = -n.z;
            let tz = n.x;
            let px = a.pos.x - s.pos.x;
            let pz = a.pos.z - s.pos.z;
            let dy = a.pos.y - s.pos.y;
            if dy < -1.2 || dy > 1.6 {
                continue;
            }
            let along = px * tx + pz * tz;
            let across = px * n.x + pz * n.z - s.plane_off;
            let w = s.half_w + ctx.t.radius * 0.6 + 0.3;
            if across.abs() > 3.0 || along.abs() > w + 2.5 {
                continue;
            }
            let side = if across >= 0.0 { 1 } else { -1 };
            // JS L1568-1573: coming at us, or we're walking into it
            let coming = side > 0 && across < 2.2 && along.abs() < w && s.speed > 0.5;
            if !coming {
                if ml <= 0.05 {
                    continue;
                }
                let m_n = (move_v.x / ml) * n.x + (move_v.z / ml) * n.z;
                let m_t = (move_v.x / ml) * tx + (move_v.z / ml) * tz;
                if m_n * (side as f32) > -0.2 {
                    continue;
                }
                let t_cross = -across / m_n;
                if t_cross > 2.5 || (along + m_t * t_cross).abs() > w {
                    continue;
                }
            }
            hit = Some(*s);
            h_along = along;
            h_side = side;
            h_w = w;
            break;
        }
        if hit.is_some() && ml <= 0.05 {
            ml = 1.0;
        }
        let Some(h) = hit else {
            self.can_ref = None;
            self.can_broke = false;
            return None;
        };
        if self.can_ref != Some(h.r#ref) {
            self.can_ref = Some(h.r#ref);
            self.can_side = 0;
            self.can_t = 0.0;
            self.can_broke = false;
            bump_stats(|st| st.steer += 1);
        }
        self.can_t += dt;
        let n = h.n;
        let tx = -n.z;
        let tz = n.x;
        let hs = h_side as f32;
        // JS L1581: the edge point `off` in from the near side, `sg` along
        let edge_x = |sg: i32, off: f32| -> f32 {
            h.pos.x + n.x * (h.plane_off + hs * off) + tx * sg as f32 * (h_w + 0.55)
        };
        let edge_z = |sg: i32, off: f32| -> f32 {
            h.pos.z + n.z * (h.plane_off + hs * off) + tz * sg as f32 * (h_w + 0.55)
        };
        if self.can_side == 0 {
            // JS L1582-1586: the nearer edge we can actually walk to
            let ax = a.pos.x;
            let ay = a.pos.y;
            let az = a.pos.z;
            let ok = |sg: i32| -> bool {
                let x1 = edge_x(sg, 0.9);
                let z1 = edge_z(sg, 0.9);
                let x2 = edge_x(sg, -1.4);
                let z2 = edge_z(sg, -1.4);
                self.dry_line(ctx, ax, ay, az, x1, z1)
                    && self.fat_los(ctx, ax, ay, az, x1, ay, z1)
                    && self.dry_line(ctx, x1, ay, z1, x2, z2)
                    && self.fat_los(ctx, x1, ay, z1, x2, ay, z2)
            };
            let pref = if h_along >= 0.0 { 1 } else { -1 };
            let picked = if ok(pref) {
                pref
            } else if ok(-pref) {
                -pref
            } else {
                2
            };
            self.can_side = picked;
        }
        if self.can_side != 2 && self.can_t > 4.0 {
            self.can_side = 2;
        }
        let (kind, wr) = ctx.own(self.team, self.slot);
        if self.can_side == 2 {
            // JS L1588-1598: boxed in — break it (only when nothing else has the aim)
            if !free {
                return None;
            }
            if !self.can_broke {
                self.can_broke = true;
                bump_stats(|st| st.break_wall += 1);
            }
            let c = h.c;
            let dx = c.x - a.pos.x;
            let dy = c.y - (a.pos.y + 1.1);
            let dz = c.z - a.pos.z;
            let dh = dx.hypot(dz);
            let yaw = dx.atan2(dz);
            let pitch = dy.atan2(dh);
            let dist = 1.0f32.max(dh.hypot(dy));
            let off = angle_diff(self.aim_yaw, yaw).hypot(self.aim_pitch - pitch);
            let melee = kind.is_melee();
            it.fire = a.ink > 3.0
                && self.dev_trigger(
                    a,
                    ctx,
                    wr,
                    kind,
                    dist,
                    off < 0.3,
                    off < 0.7,
                    0.0,
                    if melee { 3.5 } else { ctx.bot_range(kind) },
                );
            if it.fire {
                it.squid = false;
            }
            *move_v = Vec3::new(n.x * hs * 0.3, 0.0, n.z * hs * 0.3);
            self.no_prog = 0.0;
            self.best_d = f32::INFINITY;
            return Some(AimHold { yaw, pitch, dist });
        }
        self.can_broke = false;
        // JS L1599-1605: walk to the picked edge (in front of it, then through)
        let off = if h_along.abs() < h_w + 0.1 { 0.9 } else { -1.4 };
        let gx = edge_x(self.can_side, off) - a.pos.x;
        let gz = edge_z(self.can_side, off) - a.pos.z;
        let gl = gx.hypot(gz);
        if gl > 0.05 {
            *move_v = Vec3::new(gx / gl * ml, 0.0, gz / gl * ml);
        }
        self.no_prog = 0.0;
        self.best_d = f32::INFINITY;
        None
    }

    /// JS fight move L460-470: a stomping Stamp — steer off its line, or
    /// (non-melee) keep edging away while it's close.
    pub(crate) fn stamp_steer(
        &self,
        a: &Actor,
        t: &Actor,
        dist: f32,
        kind: WeaponKind,
        mvx: &mut f32,
        mvz: &mut f32,
    ) {
        let Some(ts) = t.special_active.as_deref() else {
            return;
        };
        if ts.id != "stamp" {
            return;
        }
        // JS `ts.bodyYaw ?? t.yaw` — the sim's stamp locks `face_yaw`
        let fy = ts.face_yaw.unwrap_or(t.yaw);
        let fx = fy.sin();
        let fz = fy.cos();
        let rx = a.pos.x - t.pos.x;
        let rz = a.pos.z - t.pos.z;
        let rl = f_or(rx.hypot(rz), 1.0);
        if (rx * fx + rz * fz) / rl > 1.25f32.cos() && dist < 10.0 {
            let side = if rx * fz - rz * fx >= 0.0 { 1.0 } else { -1.0 };
            *mvx = fz * side + rx / rl * 0.45;
            *mvz = -fx * side + rz / rl * 0.45;
        } else if dist < 4.5 && !kind.is_melee() {
            *mvx += rx / rl * 0.4;
            *mvz += rz / rl * 0.4;
        }
    }
}

#[cfg(test)]
#[path = "threat_tests.rs"]
mod threat_tests;
