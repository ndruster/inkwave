//! The Boss-battle segment of the bot brain — a line-for-line port of the
//! `_bossTick` family in `src/game/bots.js` (L1870-2087): [`Bot::boss_tick`]
//! (`_bossTick` L1874-1979), [`Bot::boss_perceive`] (`_bossPerceive`
//! L1982-2022), [`Bot::boss_goal`] (`_bossGoal` L2026-2067) and
//! [`Bot::boss_evade`] (`_bossEvade` L2070-2087). Entered from
//! [`Bot::step`]'s short-circuit (JS L345) whenever [`BotCtx::boss`] is set;
//! the whole normal perception / mode-selection / fight / paint pipeline is
//! skipped, exactly like the JS early `return`.
//!
//! Intentional deviations from JS (recorded for review):
//!   - **splatling branch** (bots.js L1938 / L1951 / L1965): the Rust
//!     [`crate::weapon::WeaponKind`] roster has no splatling (retired kind,
//!     Task 38 precedent), so those arms are absent by construction.
//!   - **`_tail` `aimDist`**: the JS call passes `aimDist` (L1903/L1914);
//!     the Rust [`Bot::tail`] consumes the actor's own aim ray instead and
//!     takes no such argument (same deviation as the normal path).
//!   - **`this.target = T`** (L1920): the Rust `target` slot is an actor
//!     pair `(team, slot)`; a boss target has no representation and no
//!     consumer in a boss match (the squad is all-vs-boss), so it stays
//!     `None` every frame (the JS L1906 clear is still ported).
//!   - **threat reach probe**: [`crate::boss::Boss::threat`] is `&self`, so
//!     the slam-ring reach is recomputed per query instead of using the
//!     boss's per-move cache — perf-only, values identical (D-T43-9).
//!   - the boss tick calls only [`Bot::steer`] (JS L1900): the normal
//!     update's `_unstick` / `_backOnNav` (JS L409-411) are *not* run, and
//!     the `_tail` call passes `enemyVisible = false` (JS L1977 is a 6-arg
//!     call, so the 7th parameter is `undefined`).

use crate::actor::{Actor, ActorInput};
use crate::boss::{Boss, Threat};
use crate::match_::find_actor;
use crate::sub::{SubKind, ink_cost};
use crate::weapon::WeaponKind;
use glam::{Vec3, vec3};

use super::{BBotTgt, Bot, BotCtx, BotMode, angle_diff, wander};

impl Bot {
    /// JS `_bossTick(dt)` (bots.js L1874-1979): the whole frame of a
    /// Boss-Battle brain — refill gate, telegraph evade, fight-from-range
    /// aim / fire, ink clean-up and dodges, then the shared aim-spring tail.
    pub(super) fn boss_tick(
        &mut self,
        dt: f32,
        a: &mut Actor,
        ctx: &BotCtx<'_>,
        boss: &Boss,
    ) -> ActorInput {
        let mut it = ActorInput::default();
        let (kind, wr) = ctx.own(self.team, self.slot);
        let ink_frac = a.ink / ctx.t.ink_max;
        // JS L1877 zeroes the persistent intent flags; the Rust intent is
        // rebuilt every frame, so the fresh default already covers it.
        // JS L1878: the boss perception runs on its own 0.12 + rand·0.1 clock.
        if self.think <= 0.0 {
            self.think = 0.12 + self.rng.next_f32() * 0.1;
            self.boss_perceive(a, ctx, boss, kind);
        }
        // JS L1879
        self.react -= dt;
        self.evade_t -= dt;
        // JS L1880: the threat read at the bot's feet, horizon 1.4.
        let mut th = boss.threat(
            f64::from(a.pos.x),
            f64::from(a.pos.y),
            f64::from(a.pos.z),
            1.4,
        );
        // JS L1881-1885: human-ish — a new telegraph takes a reaction time
        // to register, and now and then a ring hop is simply missed.
        if th.level > 0.0 || th.ring_in >= 0.0 {
            if self.th_seen.is_none() {
                let r = self.rng.next_f32();
                self.th_seen = Some(self.t_time + self.diff.reaction * (0.5 + r * 0.9));
                let h = self.rng.next_f32();
                self.hop_miss = h < 0.45 - self.diff.fire_discipline * 0.4;
            }
            if self.t_time < self.th_seen.unwrap_or(f32::INFINITY) {
                th.level = 0.0;
                th.ring_in = -1.0;
                th.beam = false;
                th.cover = false;
            }
        } else {
            self.th_seen = None;
        }
        self.th = th;
        // JS L1886-1889: refill when dry — unless a crablet is right on us.
        if self.mode == BotMode::Refill && ink_frac >= self.refill_until {
            self.mode = BotMode::Boss;
            self.path = None;
            self.goal_timer = 0.0;
        }
        let crab_on_us = self.b_tgt.is_some_and(|t| t.is_crab() && t.dist() < 4.0);
        if self.mode != BotMode::Refill && ink_frac < 0.1 && !(crab_on_us && ink_frac > 0.03) {
            self.mode = BotMode::Refill;
            self.refill_until = 0.8 + self.rng.next_f32() * 0.15;
            self.path = None;
        }
        if self.mode != BotMode::Refill {
            self.mode = BotMode::Boss;
        }
        // JS L1890-1899: where to go — evade out of a telegraph, else the
        // refill spot or a fresh fight-from spot.
        self.goal_timer -= dt;
        self.repath -= dt;
        // in own ink under a sweep: just dive (JS L1892).
        let evading = th.level > 0.2 && !(th.beam && a.ground_team == 1);
        if evading && (self.evade_t <= 0.0 || self.path.is_none()) {
            self.boss_evade(a, ctx, boss, &th);
            self.evade_t = 0.45;
        } else if !evading && self.was_evading {
            // leaving the evade: drop the stale escape path once (JS L1894).
            self.path = None;
            self.goal_timer = 0.0;
        }
        self.was_evading = evading;
        if !evading {
            if self.mode == BotMode::Refill {
                if self.repath <= 0.0 || self.path.is_none() {
                    self.pick_refill(a, ctx);
                }
            } else if self.goal_timer <= 0.0
                || self.path.is_none()
                || self.pi >= self.path.as_ref().map_or(0, |p| p.len())
                || (boss.stunned && !self.rushing)
            {
                self.boss_goal(a, ctx, boss, kind);
            }
        }
        // JS L1900-1901: steer only — no unstick / back-on-nav (those are
        // normal-update-only, JS L409-411).
        let mut move_v = self.steer(a, ctx, dt);
        let mut want_move = move_v.length_squared() > 0.01;
        // JS L1903: `aimDist = 6` initial — the Rust tail takes no aimDist
        // (file-head deviation).
        let mut want_yaw = if want_move {
            move_v.x.atan2(move_v.z)
        } else {
            a.yaw
        };
        let mut want_pitch = -0.1f32;
        // JS L1904-1906: the duel-target slot is cleared every frame (the
        // boss target has no Rust representation — file-head deviation).
        self.target = None;
        let dive = th.beam && a.ground_team == 1;
        let mut tgt = self.b_tgt;
        if let Some(mut t) = tgt
            && self.mode == BotMode::Boss
            && !dive
        {
            // ---- aim + fire (JS L1907-1957)
            // JS L1908-1909: the shape socket is live; the crab pos froze
            // at perceive time.
            let tp = match t {
                BBotTgt::Shape { idx, .. } => boss.hit_sphere(idx).0,
                BBotTgt::Crab { pos, .. } => pos,
            };
            let v2 = vec3(tp.x - a.pos.x, tp.y - (a.pos.y + 1.1), tp.z - a.pos.z);
            let dist = v2.x.hypot(v2.z);
            // JS L1912: `T.dist = dist` — the live XZ distance, stored back.
            t = match t {
                BBotTgt::Shape {
                    idx,
                    rad,
                    weak,
                    los,
                    ..
                } => BBotTgt::Shape {
                    idx,
                    rad,
                    weak,
                    dist,
                    los,
                },
                BBotTgt::Crab { id, pos, los, .. } => BBotTgt::Crab { id, pos, dist, los },
            };
            let ideal_yaw = v2.x.atan2(v2.z);
            let ideal_pitch = v2.y.atan2(dist);
            // JS L1915-1919: aim error at 0.8× the duel spread + the
            // acquisition overshoot settling over the reaction window.
            let e = self.diff.aim_error * 0.8;
            let acq = (-self.acq_t / (0.12f32).max(self.diff.reaction * 0.9)).exp();
            want_yaw = ideal_yaw
                + e * (0.75 * wander(self.t_time * 1.7 + self.ph1) + 2.4 * acq * self.acq_sign_y);
            want_pitch = ideal_pitch
                + e * 0.6
                    * (0.75 * wander(self.t_time * 2.1 + self.ph2) + 1.6 * acq * self.acq_sign_p);
            // JS L1921-1922: shape radius buys extra range; a charger holds
            // its exact reach.
            let range = ctx.bot_range(kind) + if t.is_crab() { 0.0 } else { t.rad() * 0.6 };
            let in_range = dist
                < range
                    * if kind == WeaponKind::Charger {
                        1.0
                    } else {
                        1.05
                    };
            // JS L1923-1931: circle-strafe a little while holding position
            // (a squad that stands still gets slammed).
            if !want_move && th.level == 0.0 && kind != WeaponKind::Charger {
                if self.strafe_t <= 0.0 {
                    let r1 = self.rng.next_f32();
                    let r2 = self.rng.next_f32();
                    let r3 = self.rng.next_f32();
                    self.strafe_t = 0.8 + r1 * 1.4;
                    self.strafe = if r2 < 0.5 { -1.0 } else { 1.0 };
                    self.strafe_amp = 0.35 + r3 * 0.4;
                }
                self.strafe_s +=
                    (self.strafe * self.strafe_amp - self.strafe_s) * (1.0 - (-4.0 * dt).exp());
                let nx = v2.x / dist.max(0.01);
                let nz = v2.z / dist.max(0.01);
                move_v = vec3(-nz * self.strafe_s, 0.0, nx * self.strafe_s);
                if dist < 4.0 {
                    // JS L1929: not right under its claws.
                    move_v.x -= nx * 0.6;
                    move_v.z -= nz * 0.6;
                }
                want_move = move_v.length_squared() > 0.01;
            }
            // JS L1932-1934: the aim tolerance widens while already firing.
            let off = angle_diff(self.aim_yaw, ideal_yaw).hypot(self.aim_pitch - ideal_pitch);
            let tol =
                0.05f32.max(t.rad().atan2(dist.max(0.5))) * if self.firing { 2.4 } else { 1.5 };
            self.firing = false;
            if in_range && t.los() && off < tol && self.react <= 0.0 && ink_frac > 0.02 {
                // JS L1935-1947: per-kind fire gates (splatling arm absent —
                // file-head deviation).
                let mut fire = true;
                if kind == WeaponKind::Charger {
                    fire = !(wr.charging_flag() && wr.charge() >= self.charge_release());
                    if wr.charging_flag() {
                        move_v *= 0.3;
                    }
                } else if kind == WeaponKind::Spinner {
                    fire = wr.burst_t() <= 0.0
                        && !(wr.charging_flag() && wr.charge() >= self.charge_release());
                    if wr.charging_flag() {
                        move_v *= 0.6;
                    }
                } else if kind == WeaponKind::Bow {
                    fire =
                        !(wr.charging_flag() && wr.charge() >= 0.5f32.max(self.charge_release()));
                } else if kind == WeaponKind::Blade {
                    // the Cutlass charges its heavy cut to full and lets go.
                    fire = wr.charge() < 0.98;
                } else if kind == WeaponKind::Brolly {
                    // pump the trigger (holding it would unfold the canopy).
                    fire = !self.pump;
                    self.pump = fire;
                } else if kind == WeaponKind::Roller {
                    fire = dist < 5.5 || (wr.rolling() && dist < 8.0);
                }
                it.fire = fire;
                self.firing = fire;
                // JS L1949: the aim spring's combat stiffness while shooting
                // (reset to 'boss' at the end of the tick, L1978) — set even
                // when `fire` is false, exactly like the JS assignment.
                self.mode = BotMode::Fight;
                // JS L1950: bombs go onto the shell, never on a crablet.
                if self.bomb_cd.is_none_or(|v| v <= 0.0)
                    && !t.is_crab()
                    && a.ink > ink_cost(SubKind::Bomb, Self::sub_set()) + 10.0
                    && dist > 5.0
                    && dist < 13.0
                    && self.rng.next_f32() < 0.025
                {
                    it.sub = true;
                    self.bomb_cd = Some(6.0 + self.rng.next_f32() * 6.0);
                    self.bomb_aim = true;
                }
            } else if matches!(
                kind,
                WeaponKind::Charger | WeaponKind::Spinner | WeaponKind::Bow
            ) && wr.charging_flag()
                && t.los()
            {
                // JS L1951: hold a charge through a blink.
                it.fire = true;
            }
            // JS L1952-1956: specials — slam from under its claws, the
            // storm cloud onto it.
            if a.special_ready() && th.level == 0.0 && !t.is_crab() {
                if a.special_id.as_deref() == Some("slam") && dist < 5.5 {
                    it.special = true;
                }
                if a.special_id.as_deref() == Some("storm") && dist < 13.0 && t.los() {
                    it.special = true;
                }
            }
            tgt = Some(t);
        }
        self.b_tgt = tgt;
        // JS L1958-1967: not shooting it — clean boss ink off the way (and
        // rollers roll it up).
        if !it.fire && self.mode != BotMode::Refill && !dive && ink_frac > 0.15 {
            let ahead_yaw = if want_move {
                move_v.x.atan2(move_v.z)
            } else {
                a.yaw
            };
            let st = ctx.paint.region_stats(
                ctx.world.collision,
                a.pos.x + ahead_yaw.sin() * 3.0,
                a.pos.y,
                a.pos.z + ahead_yaw.cos() * 3.0,
                2.5,
                a.team,
            );
            if a.ground_team == 2 || (st.n != 0 && st.enemy > 0.2) {
                self.sweep += dt * 2.1;
                if self.mode != BotMode::Fight {
                    want_yaw = ahead_yaw
                        + if kind == WeaponKind::Roller {
                            0.0
                        } else {
                            self.sweep.sin() * 0.5
                        };
                    want_pitch = match kind {
                        WeaponKind::Charger => -0.12,
                        WeaponKind::Blaster => -0.28,
                        _ => -0.42,
                    };
                }
                it.fire = if kind == WeaponKind::Roller {
                    want_move
                } else if kind != WeaponKind::Charger {
                    true
                } else {
                    !wr.charging_flag() || wr.charge() < 0.6
                };
            }
        }
        // JS L1968-1971: the refill block is the normal one verbatim.
        if self.mode == BotMode::Refill {
            self.refill(a, ctx, &mut it, &mut want_pitch);
        }
        // JS L1972-1976: dodges — hop the shockwave, dive under the sweep,
        // swim when travelling through own ink.
        if th.ring_in >= 0.0
            && th.ring_in < 0.2
            && a.grounded
            && self.jump_cd <= 0.0
            && !self.hop_miss
        {
            it.jump = true;
            self.jump_cd = 0.5;
        }
        if dive {
            it.squid = true;
            it.fire = false;
        } else if !it.fire
            && !wr.charging_flag()
            && a.ground_team == 1
            && (self.path_remaining(a, ctx) > 4.0 || evading)
        {
            it.squid = true;
        }
        if !want_move && !it.fire && a.ground_team != 1 {
            it.squid = false;
        }
        // JS L1977: the 6-arg `_tail` call → `enemyVisible` is undefined
        // (false); the `aimDist` argument has no Rust counterpart.
        self.tail(
            dt, a, ctx, &move_v, want_yaw, want_pitch, want_move, false, &mut it, kind, wr, None,
        );
        if self.mode == BotMode::Fight {
            self.mode = BotMode::Boss;
        }
        // the JS L345 early return skips `step`'s tail stats edges — replay
        // them here so the autopilot counters keep counting.
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

    /// JS `_bossPerceive(boss)` (bots.js L1982-2022): what to shoot — a
    /// crablet that's closing in, else the part of the boss worth hitting
    /// that it can see (sticky pick; the open belly and, for chargers / the
    /// disciplined, the eyes score down).
    fn boss_perceive(&mut self, a: &Actor, ctx: &BotCtx<'_>, boss: &Boss, kind: WeaponKind) {
        // JS L1983: the boss eye height is 1.2 (not the duel `EYE_Y`).
        let eye = Vec3::new(a.pos.x, a.pos.y + 1.2, a.pos.z);
        let col = ctx.world.collision;
        let mut t: Option<BBotTgt> = None;
        // JS L1985-1992: the nearest live crablet within 9 m with a
        // sightline (L1992 re-reads the same fresh values — a no-op here).
        for c in &boss.crabs {
            if c.dead {
                continue;
            }
            let d = (c.x - a.pos.x).hypot(c.z - a.pos.z);
            if d > 9.0 || t.is_some_and(|b| d >= b.dist()) {
                continue;
            }
            let cp = vec3(c.x, c.y + 0.35, c.z);
            if !col.los(eye, cp) {
                continue;
            }
            t = Some(BBotTgt::Crab {
                id: c.id,
                pos: cp,
                dist: d,
                los: true,
            });
        }
        // JS L1993-2015: else a hit shape worth hitting that it can see.
        if t.is_none() && boss.visible && !boss.dead {
            self.shape_t -= 0.2;
            let active: Vec<usize> = (0..boss.tb.hit.len())
                .filter(|&i| boss.shape_active(i))
                .collect();
            let belly = active
                .iter()
                .copied()
                .find(|&i| boss.tb.hit[i].weak && boss.tb.hit[i].socket == "belly");
            let mut pick: Option<usize> = None;
            if let Some(BBotTgt::Shape { idx, .. }) = self.b_tgt
                && self.shape_t > 0.0
                && boss.shape_active(idx)
            {
                pick = Some(idx);
            }
            if belly.is_some() && pick != belly {
                pick = None;
            }
            if pick.is_none() {
                self.shape_t = 1.2 + self.rng.next_f32() * 1.5;
                // JS L2002: a charger always goes for the eyes; the rest
                // roll for it (the charger short-circuits the draw).
                let eyes = kind == WeaponKind::Charger
                    || self.rng.next_f32() < 0.25 + self.diff.fire_discipline * 0.3;
                let mut scored: Vec<(f32, usize)> = active
                    .iter()
                    .map(|&i| {
                        let (p, _r, weak, _act) = boss.hit_sphere(i);
                        // JS L2004: belly −100, weak eyes −50, weak 10,
                        // plus the 3D distance from the eye.
                        let base = if Some(i) == belly {
                            -100.0
                        } else if weak && eyes {
                            -50.0
                        } else if weak {
                            10.0
                        } else {
                            0.0
                        };
                        (base + p.distance(eye), i)
                    })
                    .collect();
                scored.sort_by(|x, y| x.0.total_cmp(&y.0));
                // JS L2007-2008: the first seen one, else the first
                // non-weak one (armour is better than nothing).
                pick = scored
                    .iter()
                    .find(|&&(_, i)| col.los(eye, boss.hit_sphere(i).0))
                    .map(|&(_, i)| i)
                    .or_else(|| {
                        scored
                            .iter()
                            .find(|&&(_, i)| !boss.tb.hit[i].weak)
                            .map(|&(_, i)| i)
                    });
            }
            if let Some(pi) = pick {
                let (pp, pr, pweak, _) = boss.hit_sphere(pi);
                let los = col.los(eye, pp);
                // JS L2012: re-picking the same shape keeps the old target
                // object (and its last live `dist`); a fresh pick measures
                // from the feet. `rad` / `weak` are constants per socket, so
                // re-deriving them is identical to carrying them over.
                let dist = match self.b_tgt {
                    Some(BBotTgt::Shape { idx, dist, .. }) if idx == pi => dist,
                    _ => pp.distance(a.pos),
                };
                t = Some(BBotTgt::Shape {
                    idx: pi,
                    rad: pr * 0.85,
                    weak: pweak,
                    dist,
                    los,
                });
            }
        }
        // JS L2016-2021: a new target identity restarts the reaction and
        // acquisition jitter.
        let changed = match (self.b_tgt, t) {
            (Some(prev), Some(nt)) => match (prev, nt) {
                (BBotTgt::Shape { idx: p, .. }, BBotTgt::Shape { idx: n, .. }) => p != n,
                (BBotTgt::Crab { id: p, .. }, BBotTgt::Crab { id: n, .. }) => p != n,
                _ => true,
            },
            (None, Some(_)) => true,
            _ => false,
        };
        if changed {
            let r = self.rng.next_f32();
            self.react = self.diff.reaction * (0.6 + r * 0.5);
            self.acq_t = 0.0;
            let s = self.rng.next_f32();
            let m = self.rng.next_f32();
            self.acq_sign_y = (if s < 0.5 { -1.0 } else { 1.0 }) * (0.5 + m * 0.5);
            let p = self.rng.next_f32();
            self.acq_sign_p = (p - 0.5) * 1.2;
        }
        self.b_tgt = t;
    }

    /// JS `_bossGoal(boss)` (bots.js L2026-2067): a spot to fight from —
    /// weapon range off its flank (not in front: slams, sweeps and charges
    /// go that way), clear of the other bots, seeing it. When it's stunned
    /// everyone rushes the belly; rollers keep the squad's routes clean.
    fn boss_goal(&mut self, a: &Actor, ctx: &BotCtx<'_>, boss: &Boss, kind: WeaponKind) {
        let r = self.rng.next_f32();
        self.goal_timer = 2.4 + r * 2.0;
        // JS L2029: the phase-3 roll is only taken when it isn't stunned.
        let stunned = boss.stunned || (boss.phase >= 3 && self.rng.next_f32() < 0.3);
        self.rushing = boss.stunned;
        let fwd = boss.yaw;
        let (bx, by, bz) = (boss.pos.x, boss.pos.y, boss.pos.z);
        let col = ctx.world.collision;
        // JS L2032-2044: rollers keep the routes clean while the boss isn't
        // open — the neediest clean, threat-free patch within 15 m.
        if kind == WeaponKind::Roller && !boss.stunned {
            let mut best_p: Option<Vec3> = None;
            let mut bs = 0.25f32;
            for _ in 0..12 {
                let ang = self.rng.next_f32() * std::f32::consts::TAU;
                let rr = 3.0 + self.rng.next_f32() * 12.0;
                let v = vec3(a.pos.x + ang.cos() * rr, a.pos.y, a.pos.z + ang.sin() * rr);
                let st = ctx.paint.region_stats(col, v.x, v.y, v.z, 2.2, a.team);
                if st.n == 0
                    || boss
                        .threat(f64::from(v.x), f64::from(v.y), f64::from(v.z), 1.5)
                        .level
                        > 0.0
                {
                    continue;
                }
                let sc = st.enemy - rr * 0.02;
                if sc > bs {
                    bs = sc;
                    best_p = Some(v);
                }
            }
            if let Some(p) = best_p
                && self.rng.next_f32() < 0.7
            {
                self.path_to(ctx, a.pos, a.grounded, p, 0.4);
                return;
            }
        }
        // JS L2045-2046: the ring radius is the weapon's comfortable reach.
        let reach = if kind == WeaponKind::Charger {
            15.0
        } else if kind == WeaponKind::Roller {
            3.2
        } else {
            (ctx.bot_range(kind) * 0.7).clamp(4.5, 11.0)
        };
        let rrad = 3.4 + if boss.stunned { reach.min(6.0) } else { reach };
        // JS L2047: the other bots' goals (mate_goals ≡ `o.bot.goal >= 0`).
        let my_idx = find_actor(ctx.actors, a.team, a.slot).unwrap_or(usize::MAX);
        let mates: Vec<usize> = ctx
            .actors
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                *i != my_idx && o.alive && ctx.mate_goals.get(*i).is_some_and(|g| g.is_some())
            })
            .map(|(i, _)| i)
            .collect();
        let mut best = usize::MAX;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..12 {
            // JS L2050-2051: bearing — its front when it's open (belly),
            // else a flank or the rear.
            let off = if stunned {
                (self.rng.next_f32() - 0.5) * 1.3
            } else {
                let sgn = if self.rng.next_f32() < 0.5 { 1.0 } else { -1.0 };
                sgn * (0.95 + self.rng.next_f32() * 1.9)
            };
            let ang = fwd + off;
            let rr = rrad * (0.85 + self.rng.next_f32() * 0.3);
            let v = vec3(bx + ang.sin() * rr, by + 0.5, bz + ang.cos() * rr);
            let Some(id) = ctx.nav.nearest(v, 2.5, false) else {
                continue;
            };
            let n = ctx.nav.nodes[id].pos;
            if (n.x - v.x).hypot(n.z - v.z) > 2.5 {
                continue;
            }
            let mut s = -(n.x - a.pos.x).hypot(n.z - a.pos.z) * 0.08 + self.rng.next_f32();
            if boss
                .threat(f64::from(n.x), f64::from(n.y), f64::from(n.z), 1.5)
                .level
                > 0.0
            {
                s -= 6.0;
            }
            if col.los(vec3(n.x, n.y + 1.3, n.z), vec3(bx, by + 2.5, bz)) {
                s += 3.0;
            }
            for &mi in &mates {
                if let Some(g) = ctx.mate_goals[mi] {
                    let gp = ctx.nav.nodes[g].pos;
                    if (gp.x - n.x).hypot(gp.z - n.z) < 4.0 {
                        s -= 2.5;
                    }
                }
            }
            if s > bs {
                bs = s;
                best = id;
            }
        }
        if best == usize::MAX {
            self.path = None;
            return;
        }
        let n = ctx.nav.nodes[best].pos;
        self.path_to(ctx, a.pos, a.grounded, n, 1.0);
    }

    /// JS `_bossEvade(boss, th)` (bots.js L2070-2087): out of a telegraph —
    /// the reachable spot nearby with the least danger, biased along the
    /// escape direction.
    fn boss_evade(&mut self, a: &Actor, ctx: &BotCtx<'_>, boss: &Boss, th: &Threat) {
        let mut best = usize::MAX;
        let mut bs = f64::NEG_INFINITY;
        for _ in 0..12 {
            // JS L2074: scatter around the escape bearing.
            let ang = th.ax.atan2(th.az) + (f64::from(self.rng.next_f32()) - 0.5) * 2.4;
            let r = 3.0 + f64::from(self.rng.next_f32()) * 5.0;
            let v = vec3(
                (f64::from(a.pos.x) + ang.sin() * r) as f32,
                a.pos.y,
                (f64::from(a.pos.z) + ang.cos() * r) as f32,
            );
            let Some(id) = ctx.nav.nearest(v, 1.2, false) else {
                continue;
            };
            let n = ctx.nav.nodes[id].pos;
            let t = boss.threat(f64::from(n.x), f64::from(n.y), f64::from(n.z), 1.6);
            // JS L2080: least danger, closest, along the escape direction.
            let s = -t.level * 8.0 - f64::from((n.x - a.pos.x).hypot(n.z - a.pos.z)) * 0.25
                + (ang.sin() * th.ax + ang.cos() * th.az) * 1.5
                + f64::from(self.rng.next_f32()) * 0.5;
            if s > bs {
                bs = s;
                best = id;
            }
        }
        if best == usize::MAX {
            return;
        }
        let n = ctx.nav.nodes[best].pos;
        self.path_to(ctx, a.pos, a.grounded, n, 1.0);
        self.goal_timer = 0.8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::SimWorld;
    use crate::boss::nav::BossNav;
    use crate::boss::{BossConfig, CrabState};
    use crate::bot::MateMind;
    use crate::collision::CollisionWorld;
    use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
    use crate::nav::NavGraph;
    use crate::net::wire::JVal;
    use crate::paint::PaintGrid;
    use crate::sub::SubSystem;
    use crate::tuning::Tuning;
    use crate::weapon::WeaponRunner;

    const DT: f32 = crate::actor::FIXED_DT;

    fn flags() -> BrushCommon {
        BrushCommon {
            tag: None,
            color: "#dddddd".into(),
            pattern: 0,
            paint: true,
            solid: true,
            grate: false,
            rail: false,
            roof: false,
            perch: false,
            no_nav: false,
            hidden: false,
            bevel: None,
            no_paint: Vec::new(),
            mural: Vec::<Mural>::new(),
            oct: None,
            only_in: None,
            not_in: None,
        }
    }

    fn bx(min: [f32; 3], max: [f32; 3]) -> Brush {
        Brush::Box {
            min,
            max,
            common: flags(),
        }
    }

    const B: Bounds = Bounds {
        min_x: -10.0,
        max_x: 10.0,
        min_z: -10.0,
        max_z: 10.0,
    };

    struct Fix {
        world: CollisionWorld,
        layout: crate::geometry::StageLayout,
        tuning: Tuning,
        kinds: Vec<WeaponKind>,
        runners: Vec<WeaponRunner>,
        subs: SubSystem,
        mate_minds: Vec<Option<MateMind>>,
    }

    /// A 20×20 m flat deck centred on the origin, pads at ±5 z (the shape
    /// of the `bot/tests.rs` harness).
    fn flat_fix() -> Fix {
        let world = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let stage = crate::geometry::StageInfo {
            id: "test".into(),
            bounds: B,
            spawn_pads: vec![[0.0, 0.0, -5.0], [0.0, 0.0, 5.0]],
            spawn_barrier: 4.2,
            water_y: -1.6,
        };
        let meta = crate::geometry::LayoutMeta {
            primitive_counts: crate::geometry::PrimitiveCounts {
                boxes: 1,
                obox: 0,
                ramp: 0,
                total: 1,
            },
            source_counts: crate::geometry::SourceCounts {
                single: 0,
                half: 0,
                half_mirrored: 0,
            },
            bounds: B,
            spawn_pads: stage.spawn_pads.clone(),
            spawn_barrier: stage.spawn_barrier,
            water_y: stage.water_y,
            surface_slots: Default::default(),
            variant_counts: None,
        };
        let layout = crate::geometry::StageLayout {
            schema: "test".into(),
            source: crate::tuning::Source {
                commit: "test".into(),
                path: "test".into(),
            },
            stage,
            primitives: vec![bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])],
            zones: None,
            meta,
        };
        Fix {
            world,
            layout,
            tuning: crate::embedded_tuning(),
            kinds: Vec::new(),
            runners: Vec::new(),
            subs: SubSystem::new(0),
            mate_minds: Vec::new(),
        }
    }

    fn kid(f: &Fix, pos: Vec3) -> Actor {
        let mut a = Actor::new(0, 0, &f.tuning.player);
        a.pos = pos;
        a.yaw = 0.0;
        a.aim_yaw = 0.0;
        a
    }

    fn ctx_of<'a>(
        f: &'a Fix,
        nav: &'a NavGraph,
        paint: &'a PaintGrid,
        sim: &'a SimWorld<'a>,
        actors: &'a [Actor],
        boss: Option<&'a Boss>,
    ) -> BotCtx<'a> {
        BotCtx {
            t: &f.tuning.player,
            weapons: &f.tuning.weapons,
            weapon_range: &f.tuning.weapon_range,
            actors,
            nav,
            paint,
            world: sim,
            kinds: &f.kinds,
            runners: &f.runners,
            mate_goals: &[],
            subs: &f.subs,
            mate_minds: &f.mate_minds,
            zones: None,
            zctl: None,
            boss,
            launched: &[],
            time: 0.0,
        }
    }

    fn crab(id: u32, x: f32, z: f32) -> CrabState {
        CrabState {
            id,
            x,
            y: 0.06,
            z,
            yaw: 0.0,
            hp: 1.0,
            dead: false,
            speed: Some(0.0),
            miss_t: 0.0,
            pop_t: 0.0,
            spawned: true,
            popped: false,
            age: 0.0,
            tgt: -1,
            retgt: 0.0,
        }
    }

    /// The slam record of the `boss/hazards.rs` tests (tele 1.0, act 1.9,
    /// rings at +0.0 / +0.6 after the tele).
    fn slam_rec(t0: f64, mark: [f64; 3]) -> JVal {
        let num = |v: f64| JVal::Num(v);
        JVal::Obj(vec![
            ("id".into(), JVal::Str("slam".into())),
            ("t0".into(), num(t0)),
            ("s".into(), num(1.0)),
            ("d".into(), JVal::Arr(vec![num(1.0), num(1.9), num(1.0)])),
            (
                "p".into(),
                JVal::Obj(vec![
                    ("x".into(), num(mark[0])),
                    ("y".into(), num(mark[1])),
                    ("z".into(), num(mark[2])),
                    ("rings".into(), JVal::Arr(vec![num(0.0), num(0.6)])),
                ]),
            ),
        ])
    }

    // JS L1985-1991: the nearest live crablet within 9 m with a sightline
    // wins over any shape, and the brain stays in the boss mode.
    #[test]
    fn perceive_targets_a_live_crablet_in_sight() {
        let f = flat_fix();
        let nav = NavGraph::without_layout(
            &f.world,
            [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
            4.2,
            &f.tuning.player,
        );
        let paint = PaintGrid::new(&f.world);
        let sim = SimWorld::new(&f.world, &f.layout);
        let bnav = BossNav::new(&sim, &f.tuning.boss.nav);
        let mut boss = Boss::new(
            &sim,
            bnav.clone(),
            f.tuning.boss.clone(),
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: true,
                seed: 7,
            },
        );
        boss.crabs.push(crab(7, 2.0, 0.0));
        let mut a = kid(&f, vec3(0.0, 0.0, 0.0));
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
        // the boss think clock pre-draws `r_think·0.2` — run until the
        // first perception lands.
        let mut seen = false;
        for _ in 0..40 {
            let snap = vec![a.clone()];
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
            if bot.b_tgt.is_some() {
                seen = true;
                break;
            }
        }
        assert!(
            matches!(bot.b_tgt, Some(BBotTgt::Crab { id: 7, .. })),
            "crab perception: {:?}",
            bot.b_tgt
        );
        assert!(seen);
        assert_eq!(bot.mode, BotMode::Boss);
    }

    // JS L1886-1889: dry ink breaks off to refill — unless a crablet is
    // right on us (> 0.03 of ink and < 4 m keeps the fight).
    #[test]
    fn dry_ink_refills_but_a_crab_on_us_holds() {
        let f = flat_fix();
        let nav = NavGraph::without_layout(
            &f.world,
            [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
            4.2,
            &f.tuning.player,
        );
        let paint = PaintGrid::new(&f.world);
        let sim = SimWorld::new(&f.world, &f.layout);
        let bnav = BossNav::new(&sim, &f.tuning.boss.nav);
        let mk_boss = |with_crab: bool| {
            let mut boss = Boss::new(
                &sim,
                bnav.clone(),
                f.tuning.boss.clone(),
                BossConfig {
                    max_hp: 18000.0,
                    difficulty: "normal",
                    sim: true,
                    seed: 7,
                },
            );
            if with_crab {
                boss.crabs.push(crab(9, 3.0, 0.0));
            }
            boss
        };
        // (a) nothing near, 5 % ink → Refill with a fresh refillUntil.
        let boss = mk_boss(false);
        let mut a = kid(&f, vec3(0.0, 0.0, 0.0));
        a.ink = 0.05 * f.tuning.player.ink_max;
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
        let snap = vec![a.clone()];
        {
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
            assert_eq!(bot.mode, BotMode::Refill, "dry → refill");
            assert!((0.8..=0.95).contains(&bot.refill_until));
        }
        // (b) a crablet 3 m away perceived *before* the dry break-off: the
        // 0.05 > 0.03 ink holds the fight (JS L1888).
        let boss = mk_boss(true);
        let mut a = kid(&f, vec3(0.0, 0.0, 0.0));
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
        for _ in 0..40 {
            let snap = vec![a.clone()];
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
            if bot.b_tgt.is_some() {
                break;
            }
        }
        assert!(matches!(bot.b_tgt, Some(BBotTgt::Crab { id: 9, .. })));
        a.ink = 0.05 * f.tuning.player.ink_max;
        let snap = vec![a.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
        let _it = bot.step(DT, &mut a, &ctx);
        assert_eq!(bot.mode, BotMode::Boss, "crab on us holds the fight");
    }

    // JS L1881-1885 + L1892-1899: a fresh telegraph is invisible until the
    // reaction window closes, then the brain evades off the nav graph.
    #[test]
    fn slam_telegraph_gates_on_reaction_then_evades() {
        let f = flat_fix();
        let nav = NavGraph::without_layout(
            &f.world,
            [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
            4.2,
            &f.tuning.player,
        );
        let paint = PaintGrid::new(&f.world);
        let sim = SimWorld::new(&f.world, &f.layout);
        let bnav = BossNav::new(&sim, &f.tuning.boss.nav);
        let mut boss = Boss::new(
            &sim,
            bnav.clone(),
            f.tuning.boss.clone(),
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: true,
                seed: 7,
            },
        );
        // The slam wind-up (u = 0.5 < tele + 0.1): a bot 3 m from the mark
        // (inside slamR + 1.2 = 4.5) reads `level = 1.0`. The corner of the
        // deck keeps both points clear of the spawn-pad keep-out that
        // `threat` short-circuits on.
        let t0 = 100.0f64;
        boss.bt = (t0 + 0.5) as f32;
        boss.hz_add(slam_rec(t0, [8.0, 0.0, 8.0]));
        let mut a = kid(&f, vec3(8.0, 0.0, 5.0));
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
        // First threatening frame: thSeen is set, the level is zeroed → no
        // evade yet (JS L1883-1885).
        {
            let snap = vec![a.clone()];
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
        }
        assert!(bot.th_seen.is_some(), "reaction window opened");
        assert!(!bot.was_evading, "still reacting");
        assert_eq!(bot.th.level, 0.0, "the fresh telegraph is gated out");
        assert!(
            bot.th_seen.unwrap() > DT,
            "the window ends ahead: t_time = {DT}"
        );
        // …and once `t_time` passes it, the evade takes a nav node.
        let mut evaded = false;
        for _ in 0..300 {
            let snap = vec![a.clone()];
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
            if bot.was_evading {
                evaded = true;
                break;
            }
        }
        assert!(evaded, "the slam telegraph eventually evades");
        assert!(bot.evade_t > 0.0, "evade re-arm timer set");
        assert!(bot.path.is_some(), "the evade took a nav path");
    }

    // JS L1993-2015: a visible boss offers its hit shapes; the pick is a
    // live socket with a recorded sightline flag.
    #[test]
    fn perceive_picks_a_visible_hit_shape() {
        let f = flat_fix();
        let nav = NavGraph::without_layout(
            &f.world,
            [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
            4.2,
            &f.tuning.player,
        );
        let paint = PaintGrid::new(&f.world);
        let sim = SimWorld::new(&f.world, &f.layout);
        let bnav = BossNav::new(&sim, &f.tuning.boss.nav);
        let mut boss = Boss::new(
            &sim,
            bnav.clone(),
            f.tuning.boss.clone(),
            BossConfig {
                max_hp: 18000.0,
                difficulty: "normal",
                sim: true,
                seed: 7,
            },
        );
        boss.visible = true;
        boss.pos = vec3(0.0, 0.0, -6.0);
        let mut a = kid(&f, vec3(0.0, 0.0, 2.0));
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
        for _ in 0..40 {
            let snap = vec![a.clone()];
            let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, Some(&boss));
            let _it = bot.step(DT, &mut a, &ctx);
            if bot.b_tgt.is_some() {
                break;
            }
        }
        let Some(BBotTgt::Shape { idx, rad, .. }) = bot.b_tgt else {
            panic!("shape perception: {:?}", bot.b_tgt);
        };
        assert!(boss.shape_active(idx), "the pick is an active socket");
        assert!(
            (rad - boss.hit_sphere(idx).1 * 0.85).abs() < 1e-6,
            "rad = r·0.85"
        );
    }
}
