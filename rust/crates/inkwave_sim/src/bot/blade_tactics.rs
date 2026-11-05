//! Brine Cutlass deep fight tactics (Task 47 batch D): the faithful port of
//! `botTactics` in `src/game/kits/blade.js` L455-619 — close in on the nav
//! path (flanking out of a ranged enemy's sightline), pre-charge behind cover
//! or as a surprise, poke with the ink wave, dive the charge in along own
//! ink, quick-cut combos up close, and the heal-retreat hand-off. The kit
//! hooks `stayIn` / `targetBias` / `paint` / `paintPitch` live in bot.rs;
//! this module owns the per-brain [`BladeState`] scratch (JS `brain._bl`,
//! blade.js L407-411) minus `press` / `waveT` (the brain's flat
//! `bl_press` / `bl_wave_t`, shared with the paint hook like JS).
//!
//! Intentional deviations from JS (recorded for review):
//!   - **`pathDir` `_laPath` smoothing** (blade.js L430-435): the sim steers
//!     at the current waypoint directly — same rule as `_steer` in the bot
//!     module docs.
//!   - **`t.hitR`** (L471): the sim has no per-actor hit radius — always
//!     `PLAYER.radius`.
//!   - **`brain.diff?.id` / `brain._pickRetreat`** (L458 / L582): the sim's
//!     brain always has a difficulty and a retreat picker.
//!   - **rng draws**: JS `Math.random()` → the bot's seeded stream, drawn at
//!     the same use sites and in the same order within the tactics.

use super::{Actor, ActorInput, Bot, BotCtx, BotMode, Form, Vec3, WeaponRunner, angle_diff, vec3};
use crate::weapon::blade::lunge_safe;

/// JS `botState(brain)._bl` (blade.js L407-411). Created lazily on the first
/// tactics frame — the single `side` draw (L408) rides the bot's rng.
pub(super) struct BladeState {
    /// JS `hold`: seconds the charge has been held.
    pub(super) hold: f32,
    /// JS `full`: seconds the charge has been full.
    pub(super) full: f32,
    /// JS `dive`: seconds spent swimming in with the charge.
    pub(super) dive: f32,
    /// JS `dove`: 1 = diving with the charge, 2 = surfaced from the dive.
    pub(super) dove: f32,
    /// JS `diveOK`: this charge may be carried in a swim (one draw per
    /// charge start, L462).
    pub(super) dive_ok: bool,
    /// JS `rush`: rush-the-painted-stripe clock.
    pub(super) rush: f32,
    /// JS `side`: the strafe side (`-1.0` / `1.0`).
    pub(super) side: f32,
    /// JS `sideT`: strafe-side flip clock.
    pub(super) side_t: f32,
    /// JS `flankT`: flank-probe cache clock.
    pub(super) flank_t: f32,
    /// JS `flank`: the cached flank heading.
    pub(super) flank: Option<(f32, f32)>,
    /// JS `coverT`: cover-search cache clock.
    pub(super) cover_t: f32,
    /// JS `cover`: the cached cover spot (world x, z).
    pub(super) cover: Option<(f32, f32)>,
    /// JS `laneT`: own-lane probe cache clock.
    pub(super) lane_t: f32,
    /// JS `lane`: the cached own-lane answer.
    pub(super) lane: bool,
    /// JS `walkT`: straight-walk probe cache clock.
    pub(super) walk_t: f32,
    /// JS `walk`: straight to the target is walkable.
    pub(super) walk: bool,
    /// JS `patience`: seconds to wait out a slow approach before poking.
    pub(super) patience: f32,
    /// JS `target`: the duel target the scratch was reset for.
    pub(super) target: Option<usize>,
    /// JS `lastD`: last frame's distance (the closing-speed term).
    pub(super) last_d: f32,
    /// JS `closeV`: smoothed closing speed.
    pub(super) close_v: f32,
    /// JS `square`: wave-poke square-up clock.
    pub(super) square: f32,
    /// JS `lurk`: seconds spent lurking in own ink.
    pub(super) lurk: f32,
}

impl BladeState {
    /// JS L408: the fresh scratch (`side` drawn by the caller, `waveT` /
    /// `press` live as the brain's flat fields).
    fn new(side: f32) -> Self {
        Self {
            hold: 0.0,
            full: 0.0,
            dive: 0.0,
            dove: 0.0,
            dive_ok: false,
            rush: 0.0,
            side,
            side_t: 0.0,
            flank_t: 0.0,
            flank: None,
            cover_t: 0.0,
            cover: None,
            lane_t: 0.0,
            lane: false,
            walk_t: 0.0,
            walk: false,
            patience: 1.0,
            target: None,
            last_d: 0.0,
            close_v: 0.0,
            square: 0.0,
            lurk: 0.0,
        }
    }
}

/// JS `SKILL` (blade.js L403): the flank / cover / lurk skill factor.
fn skill(id: &str) -> f32 {
    match id {
        "easy" => 0.35,
        "normal" => 0.7,
        "hard" => 1.0,
        _ => 0.7,
    }
}

/// JS `DIVE` (blade.js L404): chance a full charge is carried in a swim.
fn dive_chance(id: &str) -> f32 {
    match id {
        "easy" => 0.0,
        "normal" => 0.6,
        "hard" => 0.9,
        _ => 0.6,
    }
}

/// JS `lerp` (kits/util).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Bot {
    /// JS `botTactics(brain, ctx)` (blade.js L455-619). `visible` /
    /// `can_fire` / `retreat` ride the JS ctx (bots.js L528 / L536).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn blade_tactics(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        dist: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        ti: usize,
        wr: &WeaponRunner,
        visible: bool,
        can_fire: bool,
        retreat: bool,
    ) {
        if self.bl_state.is_none() {
            // JS L408: `side: Math.random() < 0.5 ? -1 : 1` — one draw, lazy.
            let side = if self.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
            self.bl_state = Some(BladeState::new(side));
        }
        let mut b = self.bl_state.take().unwrap();
        self.blade_tactics_inner(
            a, ctx, dt, dist, move_v, it, ti, wr, visible, can_fire, retreat, &mut b,
        );
        self.bl_state = Some(b);
    }

    #[allow(clippy::too_many_arguments)]
    fn blade_tactics_inner(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        dt: f32,
        dist: f32,
        move_v: &mut Vec3,
        it: &mut ActorInput,
        ti: usize,
        wr: &WeaponRunner,
        visible: bool,
        can_fire: bool,
        retreat: bool,
        b: &mut BladeState,
    ) {
        let w = &ctx.weapons.blade;
        let t = &ctx.actors[ti];
        let kit = wr.blade.as_ref();
        // JS L458: `sk = SKILL[brain.diff?.id] ?? 0.7`.
        let sk = skill(&self.diff.id);
        if b.target != Some(ti) {
            // JS L459: new duel — one draw (patience jitter).
            b.target = Some(ti);
            b.rush = 0.0;
            b.cover = None;
            b.cover_t = 0.0;
            b.flank = None;
            b.flank_t = 0.0;
            b.patience = lerp(1.3, 0.5, sk) + self.rng.next_f32() * 0.5;
            b.last_d = dist;
            b.close_v = 0.0;
        }
        let charging = kit.is_some_and(|k| k.charging);
        let full = kit.is_some_and(|k| k.full);
        // JS L461: the scratch clocks.
        b.side_t -= dt;
        b.rush -= dt;
        b.cover_t -= dt;
        b.flank_t -= dt;
        b.lane_t -= dt;
        b.walk_t -= dt;
        if charging && b.hold == 0.0 {
            // JS L462: dive-or-not decided once per charge — one draw.
            b.dive_ok = self.rng.next_f32() < dive_chance(&self.diff.id);
        }
        b.hold = if charging { b.hold + dt } else { 0.0 };
        b.full = if full { b.full + dt } else { 0.0 };
        if !charging {
            // JS L465: the dive is a committed charge only.
            b.dive = 0.0;
            b.dove = 0.0;
            b.square = 0.0;
        }
        // JS L466: closing speed, smoothed.
        b.close_v += ((b.last_d - dist) / dt.max(1e-3) - b.close_v) * (1.0 - (-5.0 * dt).exp());
        b.last_d = dist;
        // --- geometry (JS L468-485)
        let dx = t.pos.x - a.pos.x;
        let dz = t.pos.z - a.pos.z;
        let dy = t.pos.y + t.smooth_y() - a.pos.y;
        let nx = dx / dist.max(0.01);
        let nz = dz / dist.max(0.01);
        let pd = (dx + t.vel.x * 0.1).hypot(dz + t.vel.z * 0.1);
        let hit_r = ctx.t.radius; // JS L471: `t.hitR || PLAYER.radius`
        let level = dy < 1.5 && dy > -1.1;
        let aim_off = angle_diff(a.aim_yaw, dx.atan2(dz)).abs();
        // JS L475: does the target (a bot) have us spotted?
        let mind = ctx.mate_minds.get(ti).and_then(|m| m.as_ref());
        let spotted = mind.is_none_or(|m| m.target == Some((a.team, a.slot)) && m.see_timer > 0.0);
        // JS L477: do they outrange us?
        let tk = ctx.kinds[ti];
        let ranged = !tk.is_melee() && ctx.weapon_reach(tk) > w.range + 1.0;
        let facing = -(t.aim_yaw.sin() * nx + t.aim_yaw.cos() * nz);
        let sighted = visible && facing > if mind.is_some() && spotted { 0.6 } else { 0.85 };
        let exposed = sighted && ranged;
        // JS L485: fully out of the ink (can cut).
        let kid_ok = a.form != Form::Squid && a.kid_t() >= ctx.t.emerge_delay + 0.02;

        if retreat {
            // JS L486-496: falling back to heal — cornered at blade range cut;
            // else swim where the way back is our ink, run where it isn't; a
            // charge in hand is kept.
            let reach = w.reach + hit_r + if a.grounded { w.lunge_dist * 0.6 } else { 0.0 };
            if full && kid_ok && level && pd <= reach && aim_off < 0.4 {
                it.fire = false;
                it.bl_hold = Some(-1.0);
                it.squid = false;
                return;
            }
            if !charging && dist < 2.3 && level && visible && aim_off < 0.75 && a.ink >= w.tap_ink {
                it.squid = false;
                self.bl_press = !self.bl_press;
                it.fire = self.bl_press;
                it.bl_hold = Some(-1.0);
                return;
            }
            let ml = move_v.length();
            it.squid = a.ground_team == 1
                && (ml < 0.1 || self.own_lane(a, ctx, move_v.x / ml, move_v.z / ml, 2.4));
            it.fire = charging;
            it.bl_hold = Some(if charging { ctx.time } else { -1.0 });
            return;
        }

        let mut fire = false;
        let mut hold = false;
        let mut squid = false;
        let mut mv: Option<(f32, f32)> = None;

        if self.block_t > 0.15 {
            // JS L530-535: an enemy canopy in the way — hold the charge for
            // the opening, no cuts into it (bots.js flanks round it).
            it.fire = charging;
            it.bl_hold = Some(if charging { ctx.time } else { -1.0 });
            it.squid = false;
            if it.jump && !(a.hp < 45.0 && !charging) {
                it.jump = false;
            }
            return;
        }
        if charging {
            // JS L536-570: holding a charge.
            hold = true;
            let sees = visible || dist < 2.2;
            let near = full && pd < 3.4;
            let cut_r = w.reach + hit_r - 0.05
                + if near && a.grounded && lunge_safe(a, nx, nz, w, ctx.world.collision, ctx.t) {
                    w.lunge_dist * 0.7
                } else {
                    0.0
                };
            let eta = (pd - cut_r) / w.move_speed_charging;
            if full && kid_ok && level && sees && pd <= cut_r && aim_off < 0.32 {
                // JS L545: in blade + lunge reach — cut.
                hold = false;
                mv = Some((nx, nz));
            } else if full
                && kid_ok
                && b.dove == 0.0
                && b.dive_ok
                && pd > cut_r + 1.4
                && dist < 10.0
                && level
                && self.bl_lane(a, ctx, nx, nz, dist, b)
                && self.bl_walkable(a, ctx, nx, nz, (dist - 1.0).min(4.0))
            {
                // JS L547: dive with the charge stored.
                b.dove = 1.0;
                b.dive = 0.0;
            } else if full
                && kid_ok
                && sees
                && dist > cut_r + 0.4
                && dist < w.wave_range * 0.9
                && ((exposed && dist > 4.2)
                    || !level
                    || (b.full > b.patience && eta > 0.5 && b.close_v < 1.5)
                    || (a.hp < 40.0 && dist > 4.0))
            {
                // JS L550-553: poke with the wave — square up first.
                b.square += dt;
                if aim_off < 0.45_f32.atan2(dist) || b.square > 0.45 {
                    hold = false;
                    b.rush = 1.5;
                    b.square = 0.0;
                }
                mv = Some((nx * 0.3, nz * 0.3));
            } else if b.hold > 3.2
                || (!full && dist < w.reach + hit_r && kit.is_some_and(|k| k.charge < 0.65))
            {
                // JS L554-556: never got there / they rushed us.
                hold = false;
            }
            if hold && b.dove == 1.0 {
                // JS L557-562: swimming in with the charge — surface short of
                // reach, or before the store runs out.
                b.dive += dt;
                squid = b.dive < 0.8 && pd > cut_r + 0.9 && self.bl_lane(a, ctx, nx, nz, dist, b);
                if !squid {
                    b.dove = 2.0;
                }
                mv = Some((nx, nz));
            } else if hold {
                // JS L563-570: walking in — charge behind cover while they
                // have us in their sights (one draw per cover search).
                if !full && exposed && sk >= 0.5 && dist > 4.5 {
                    if b.cover_t <= 0.0 {
                        b.cover_t = 0.6;
                        b.cover = if self.rng.next_f32() < sk {
                            self.bl_cover_spot(a, ctx, t, dist)
                        } else {
                            None
                        };
                    }
                    if let Some((cxp, czp)) = b.cover {
                        let cx = cxp - a.pos.x;
                        let cz = czp - a.pos.z;
                        let cl = cx.hypot(cz);
                        mv = if cl > 0.4 {
                            Some((cx / cl, cz / cl))
                        } else {
                            Some((0.0, 0.0))
                        };
                    }
                }
                if mv.is_none() {
                    mv = self.bl_approach(a, ctx, t, dist, nx, nz, exposed, sk, visible, b);
                }
            }
        } else if dist <= 3.1 && level {
            // JS L571-581: up close — a quick-cut combo, circling them (two
            // draws per side flip: the clock, then the flip).
            fire = visible && self.react <= 0.0 && aim_off < 0.75 && a.ink >= w.tap_ink && {
                self.bl_press = !self.bl_press;
                self.bl_press
            };
            if b.side_t <= 0.0 {
                b.side_t = lerp(1.0, 0.45, sk) + self.rng.next_f32() * 0.5;
                if self.rng.next_f32() < 0.55 {
                    b.side = -b.side;
                }
                if !self.bl_walkable(a, ctx, -nz * b.side, nx * b.side, 1.2) {
                    b.side = -b.side;
                }
            }
            let rad = ((dist - 1.45) * 1.3).clamp(-0.6, 0.9);
            let amp = lerp(0.3, 0.55, sk);
            mv = Some((-nz * b.side * amp + nx * rad, nx * b.side * amp + nz * rad));
        } else if a.hp < 50.0 && t.hp > 40.0 && exposed && dist > 5.0 && sk >= 0.5 {
            // JS L582-586: hurt, and a ranged enemy has us spotted out of
            // reach — fall back to heal in our ink (one draw, the retreat
            // clock). `brain._pickRetreat` always exists in the sim.
            self.mode = BotMode::Retreat;
            self.retreat_t = 1.6 + self.rng.next_f32() * 1.2;
            self.repath = 0.0;
            self.pick_retreat(a, ctx);
            squid = a.ground_team == 1;
        } else {
            // JS L587-606: closing in without a charge.
            let busy_heavy = kit.is_some_and(|k| k.swing.as_ref().is_some_and(|s| s.heavy));
            mv = if b.rush > 0.0 && (busy_heavy || self.bl_lane(a, ctx, nx, nz, dist, b)) {
                Some((nx, nz))
            } else {
                self.bl_approach(a, ctx, t, dist, nx, nz, exposed, sk, visible, b)
            };
            let can_charge =
                b.rush <= 0.0 && !busy_heavy && self.react <= 0.0 && a.ink >= w.heavy_ink + 2.0;
            let surprise = w.reach
                + hit_r
                + w.lunge_dist * 0.7
                + (w.charge_delay + w.charge_time) * w.move_speed_charging;
            if self.bl_lane(a, ctx, nx, nz, dist, b) && dist > 3.4 {
                squid = true;
                if !spotted && dist < 10.0 {
                    // JS L598: unspotted — a steady swim stays unseen.
                    mv = mv.map(|(mx, mz)| (mx * 0.55, mz * 0.55));
                }
            } else if exposed
                && sk >= 0.5
                && a.ground_team == 1
                && dist > 5.0
                && b.rush <= 0.0
                && b.lurk < lerp(0.8, 1.6, sk)
            {
                // JS L599-602: no lane and in their sights — stay under in
                // our ink until it looks away.
                squid = true;
                b.lurk += dt;
                mv = Some((0.0, 0.0));
                self.no_prog = 0.0;
            } else if can_charge
                && (if exposed {
                    dist > w.range && dist < 12.0
                } else {
                    dist > surprise - 0.8 && dist < surprise + 2.0
                })
            {
                // JS L603-604: pre-charge from the surprise band.
                hold = true;
            } else {
                // JS L605: quick cuts as we close (one draw per press flip).
                fire = can_fire && dist < w.range && {
                    self.bl_press = !self.bl_press;
                    self.bl_press
                };
            }
        }
        if !squid || !exposed {
            // JS L607: the lurk clock only holds while hidden from a shooter.
            b.lurk = (b.lurk - dt * 0.4).max(0.0);
        }
        // JS L609: no dodge-hop on every hit — only when hurt and not about
        // to cut.
        if it.jump && !(a.hp < 45.0 && !charging && dist > 3.2) {
            it.jump = false;
        }
        it.fire = fire || hold;
        // JS L611: the hold stamp the kit's fire rescue reads (`botHolds`).
        it.bl_hold = Some(if hold { ctx.time } else { -1.0 });
        it.squid = squid;
        if let Some((mx, mz)) = mv {
            let l = mx.hypot(mz);
            let k = if l > 1.0 { 1.0 / l } else { 1.0 };
            if l > 0.01 {
                *move_v = vec3(mx * k, 0.0, mz * k);
            } else {
                *move_v = Vec3::ZERO;
            }
            // JS L617: steering our own way (arcs, circling, cover) — the
            // waypoint-progress watchdog mustn't read it as stuck.
            if a.vel.x.hypot(a.vel.z) > 1.5 {
                self.no_prog = 0.0;
            }
        }
    }

    /// JS `lane()` (blade.js L481-484): the own-ink probe, cached 0.15 s.
    fn bl_lane(
        &self,
        a: &Actor,
        ctx: &BotCtx,
        nx: f32,
        nz: f32,
        dist: f32,
        b: &mut BladeState,
    ) -> bool {
        if b.lane_t <= 0.0 {
            b.lane_t = 0.15;
            b.lane = a.ground_team == 1 && self.own_lane(a, ctx, nx, nz, dist);
        }
        b.lane
    }

    /// JS `walkable` (blade.js L423-428): can the kid walk `len` m along
    /// `(ux, uz)` — no wall at knee height, no water, no drop off a ledge.
    fn bl_walkable(&self, a: &Actor, ctx: &BotCtx, ux: f32, uz: f32, len: f32) -> bool {
        let x1 = a.pos.x + ux * len;
        let z1 = a.pos.z + uz * len;
        if !self.fat_los(ctx, a.pos.x, a.pos.y, a.pos.z, x1, a.pos.y, z1) {
            return false;
        }
        if !self.dry_line(ctx, a.pos.x, a.pos.y, a.pos.z, x1, z1) {
            return false;
        }
        ctx.world
            .collision
            .ground_height(x1, z1, a.pos.y + 0.6, false)
            > a.pos.y - 1.4
    }

    /// JS `pathDir` (blade.js L430-435) — see the module docs for the
    /// `_laPath` deviation.
    fn bl_path_dir(&self, a: &Actor, ctx: &BotCtx) -> Option<(f32, f32)> {
        let p = self.path.as_ref()?;
        if self.pi >= p.len() {
            return None;
        }
        let n = ctx.nav.nodes[p[self.pi]].pos;
        let dx = n.x - a.pos.x;
        let dz = n.z - a.pos.z;
        let l = dx.hypot(dz);
        if l > 0.05 {
            Some((dx / l, dz / l))
        } else {
            None
        }
    }

    /// JS `approach` (blade.js L500-527): footwork toward the target — the
    /// nav path (straight when the way is open and short), arcing out of a
    /// ranged enemy's sightline. `None`: no path and far — leave the core's
    /// move alone.
    #[allow(clippy::too_many_arguments)]
    fn bl_approach(
        &self,
        a: &Actor,
        ctx: &BotCtx,
        t: &Actor,
        dist: f32,
        nx: f32,
        nz: f32,
        exposed: bool,
        sk: f32,
        visible: bool,
        b: &mut BladeState,
    ) -> Option<(f32, f32)> {
        let mut ux = nx;
        let mut uz = nz;
        let pdir = self.bl_path_dir(a, ctx);
        if pdir.is_none() && dist > 7.0 {
            return None;
        }
        if let Some((px, pz)) = pdir {
            if dist < 6.0 && visible {
                if b.walk_t <= 0.0 {
                    b.walk_t = 0.12;
                    b.walk = self.bl_walkable(a, ctx, nx, nz, (dist - 0.8).min(3.0));
                }
            } else {
                b.walk = false;
            }
            if !b.walk {
                ux = px;
                uz = pz;
            }
        }
        if exposed && dist > 3.2 && sk >= 0.5 {
            if b.flank_t <= 0.0 {
                b.flank_t = 0.2;
                b.flank = None;
                // JS L512-513: spiral in, on the side away from their aim.
                let fx = t.aim_yaw.sin();
                let fz = t.aim_yaw.cos();
                let f = fx * -nz + fz * nx;
                let s = if f.abs() < 0.05 {
                    b.side
                } else if f > 0.0 {
                    -1.0
                } else {
                    1.0
                };
                let th0 = lerp(0.35, 0.9, sk);
                for k in [1.0, 0.6, 0.3] {
                    let th = -th0 * k * s;
                    let (c, sn) = (th.cos(), th.sin());
                    let mx = ux * c + uz * sn;
                    let mz = -ux * sn + uz * c;
                    if self.bl_walkable(a, ctx, mx, mz, 2.2) {
                        b.flank = Some((mx, mz));
                        break;
                    }
                }
            }
            if let Some((fx2, fz2)) = b.flank {
                ux = fx2;
                uz = fz2;
            }
        }
        Some((ux, uz))
    }

    /// JS `coverSpot` (blade.js L437-453): a spot 1.6–3.2 m away, level with
    /// us and reachable on foot, that the target can't see (one rng draw,
    /// the base angle).
    fn bl_cover_spot(
        &mut self,
        a: &Actor,
        ctx: &BotCtx,
        t: &Actor,
        dist: f32,
    ) -> Option<(f32, f32)> {
        let base = self.rng.next_f32() * std::f32::consts::TAU;
        let mut best: Option<(f32, f32)> = None;
        let mut bs = f32::NEG_INFINITY;
        for i in 0..10 {
            let ang = base + (i as f32 / 10.0) * std::f32::consts::TAU;
            let r = if i % 2 == 1 { 1.6 } else { 3.2 };
            let ux = ang.sin();
            let uz = ang.cos();
            let x = a.pos.x + ux * r;
            let z = a.pos.z + uz * r;
            let gy = ctx
                .world
                .collision
                .ground_height(x, z, a.pos.y + 0.6, false);
            if (gy - a.pos.y).abs() >= 0.5 {
                continue;
            }
            let d_t = (t.pos.x - x).hypot(t.pos.z - z);
            if d_t > dist + 1.5 || d_t < 3.0 {
                continue; // not a retreat, not into their face
            }
            if ctx
                .world
                .collision
                .los(vec3(t.pos.x, t.pos.y + 1.3, t.pos.z), vec3(x, gy + 1.0, z))
            {
                continue; // they'd still see us there
            }
            if !self.bl_walkable(a, ctx, ux, uz, r) {
                continue;
            }
            let sc = -(d_t - dist) * 0.6 - r * 0.3;
            if sc > bs {
                bs = sc;
                best = Some((x, z));
            }
        }
        best
    }
}
