//! Sponge Mitts kit: the Inkbox Mitts punch / charged leap / wall cling
//! melee weapon (M3 Task 33).
//!
//! Faithful port of the sim-side subset of `src/game/kits/mitts.js`:
//!   - `K()` L48-67 (sim fields)           -> [`MittsKit`]
//!   - `gloveMuzzle` L114-125              -> [`glove_muzzle`] (the rig
//!     branch has no character in the sim, so only the fallback is kept)
//!   - `spawnFist` L128-136                -> [`spawn_fist`]
//!   - `fireFist` L137-146                 -> [`fire_fist`]
//!   - `burstFist` L152-174                -> [`burst_fist`] (the
//!     `specials.areaHit` L171 / boss L172 blocks are not ported)
//!   - `stepFist` L197-246                 -> [`MittsFist::step`] (the boss
//!     L219-222 and device/special blocks L224-226 are not ported)
//!   - `leapVelocity` L275-287             -> [`leap_velocity`]
//!   - `predictLeap` L294-342 / `telegraph` L415-424 / local preview
//!     L492-544                            -> not ported (D-T33-1: the leap
//!     lands where real physics contact says, no prediction)
//!   - `clingableAt` L345-353              -> [`clingable_at`] (the rail /
//!     grate block flags are not carried by the Rust layout — see the fn)
//!   - `startCharge` L355-362 / `cancelCharge` L363-366 ->
//!     [`MittsKit::start_charge`] / [`cancel_charge`]
//!   - `releaseLeap` L368-390              -> [`release_leap`]
//!   - `landSplash` L426-461               -> [`land_splash`]
//!   - `startCling` L463-477               -> [`start_cling`]
//!   - `dropCling` L479-490                -> [`MittsKit::drop_cling`]
//!   - `update` L546-646                   -> [`update`]
//!   - `jump` L650-660                     -> [`MittsKit::jump`] (called
//!     from `Actor::step`, JS actor.js L311-313)
//!   - `MAIN_KITS.mitts.damageTaken` L883-903 -> the [`KitArmor`] snapshot
//!     written by [`update`] and read by `Actor::damage` / `apply_hit`
//!     (Task 33 §4)
//!   - `MAIN_KITS.mitts.busy` L879         -> `WeaponRunner::busy`
//!   - `MAIN_KITS.mitts.moveSpeed` L905-912 / `spreadDeg` L913 ->
//!     `WeaponRunner::move_speed` / `spread_deg` (weapon.rs)
//!   - `MAIN_KITS.mitts.tick` L914-921     -> the match's reverse-step of
//!     `Match::fists` (Task 33 §5)
//!   - `MAIN_KITS.mitts.clear` L922-929    -> `Match::reset` clears `fists`
//!   - `ghost` L838 (the `['k','mitts']` case 0 replay) ->
//!     `net/replication.rs` (Task 33 §6)
//!
//! Intentional deviations from JS (recorded for review):
//!   - The leap-charge state (`charging` / `charge` / `chargeT`) lives on
//!     the kit, not the runner (JS keeps it on `k` and mirrors `k.charge`
//!     onto `r.charge` for the `ch` tick slot — mirrored here too).
//!   - `a.specialActive` (L552) and the bot branches (`botWant` / `botHold`
//!     / `botReleaseOk`, L574-577 / L628 / L654-658) are Task 39 / Task 35;
//!     the sim has no specials and no bot leap driving, so the structure
//!     that reads them is kept where it costs nothing.
//!   - `a.fireFacing` writes (L561/L600/L634) are presentation-only in the
//!     sim (private facing-lerp input, never set by kits upstream of the
//!     sim) — dropped.
//!   - The `damageTaken` hook is evaluated per damage event in JS; here it
//!     rides a per-frame [`KitArmor`] snapshot written by [`update`] (spec
//!     §4). The snapshot is written after this frame's form / firing-t
//!     changes and read by every hit resolved later in the same frame —
//!     equivalent within the fixed step.
//!   - `startCharge`'s `r._empty()` on an under-ink jump press (L358) is
//!     dropped: the hook runs inside `Actor::step`, which has no projectile
//!     stream to push the empty-click event onto. The press is still
//!     swallowed / not swallowed exactly like JS.
//!   - Ghost fists (remote replay) never paint and never hurt (spec §2:
//!     mute semantics = `!ghost` guards); JS ghosts paint locally under a
//!     net `mute` — the Rust replay only needs the flight + burst visuals.
//!   - The per-fist `Math.random()` seeds ride [`ProjectileSim::rng`] in
//!     JS's draw order: 1 per fist spawn (L130), 1 per surface-burst splat
//!     (L158), 2 per trail drip (L241), 25 per land splash (L438-444), 1
//!     per leap push-off (L387), 2 per flight trail drip (L606).
//!   - `spin` (L131) is presentation — no rng draw (Task 29 vis rule).

use glam::{Vec3, vec3};

use crate::actor::{Actor, FireGate, Form, KitArmor};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{Mitts, PlayerTuning};

use super::{
    DEG, FIRING_T, FoeScan, ProjectileSim, SimEvent, WeaponRunner, apply_hit, clamp_f, lerp,
    segment_capsule_dist,
};

/// A remote player's fist replayed from the `['k','mitts']` record (JS
/// `ghost` L838 case 0): it flies and bursts for the eye only — never
/// paints, never hurts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MittsFist {
    pub owner_slot: usize,
    pub team: usize,
    pub pos: Vec3,
    pub prev: Vec3,
    pub vel: Vec3,
    pub dir: Vec3,
    pub age: f32,
    pub life: f32,
    /// 0 = right glove, 1 = left (JS `f.hand`).
    pub hand: i32,
    pub seed: f32,
    /// Trail-drip distance accumulator (JS `f.trail`, starts `-0.8`).
    pub trail: f32,
    #[serde(default)]
    pub ghost: bool,
}

/// Per-runner kit state (JS `K()` L48-67; the `bot*` / `land` / `predT`
/// prediction fields are not ported — D-T33-1). `ink_min` caches
/// `leapInkMin` so [`MittsKit::jump`] can run inside `Actor::step` without
/// the tuning table (deviation noted in the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MittsKit {
    /// JS `kit.seen` — the last runner-update timestamp (L549-550).
    pub seen: f32,
    /// JS `cd` — punch shot clock.
    pub cd: f32,
    /// JS `side` — alternating glove (reset to 1; the first punch flips to
    /// the right glove).
    pub side: i32,
    /// JS `charging` — the leap crouch is held.
    pub charging: bool,
    /// JS `charge` — leap charge fraction `0..1`.
    pub charge: f32,
    /// JS `chargeT` — leap charge clock, seconds.
    pub charge_t: f32,
    /// JS `chargeAir` — seconds the crouch has been off the ground (the
    /// walked-off-a-ledge cancel, L572).
    pub charge_air: f32,
    /// JS `full` — the charge reached full (one-shot latch, audio only).
    pub full: bool,
    /// JS `leaping` — airborne from a leap.
    pub leaping: bool,
    /// JS `leapT` — seconds airborne.
    pub leap_t: f32,
    /// JS `leapV` — the launch's horizontal velocity (the pounce keeps it).
    pub leap_v: Vec3,
    /// JS `leapH` — `|leapV|` (the airborne move-speed floor).
    pub leap_h: f32,
    /// JS `slid` — something bent the pounce; physics takes over.
    pub slid: bool,
    /// JS `trail` / `trailP` — the flight's paint-drip clock.
    pub trail: f32,
    pub trail_p: Vec3,
    /// JS `landT` — seconds since landing (reset to 9; the armour grace
    /// and the post-land speed hop read it).
    pub land_t: f32,
    /// JS `cling` — stuck to a wall.
    pub cling: bool,
    /// JS `hang` — the actor-freeze flag (actor.js L502-507).
    pub hang: bool,
    /// JS `clingT` — seconds holding on.
    pub cling_t: f32,
    /// JS `clingN` — the wall normal, flattened + normalised.
    pub cling_n: Vec3,
    /// JS `clingPt` — the glove's wall contact point.
    pub cling_pt: Vec3,
    /// JS `W().leapInkMin` cache for the jump hook (deviation).
    pub ink_min: f32,
}

impl Default for MittsKit {
    fn default() -> Self {
        Self::new()
    }
}

impl MittsKit {
    /// JS `K()` fresh-kit initialiser (the sim fields).
    #[must_use]
    pub fn new() -> Self {
        Self {
            seen: 0.0,
            cd: 0.0,
            side: 1,
            charging: false,
            charge: 0.0,
            charge_t: 0.0,
            charge_air: 0.0,
            full: false,
            leaping: false,
            leap_t: 0.0,
            leap_v: Vec3::ZERO,
            leap_h: 0.0,
            slid: false,
            trail: 0.0,
            trail_p: Vec3::ZERO,
            land_t: 9.0,
            cling: false,
            hang: false,
            cling_t: 0.0,
            cling_n: Vec3::X,
            cling_pt: Vec3::ZERO,
            ink_min: 0.0,
        }
    }

    /// JS `startCharge(r, k)` L355-362 minus the audio loop. The `_empty()`
    /// click is dropped (module docs): the jump hook has no projectile
    /// stream. `r.charge` is *not* zeroed here — JS zeroes it only in
    /// `cancelCharge`; the runner mirror updates every charging frame.
    pub fn start_charge(&mut self, a: &mut Actor) -> bool {
        if self.charging || self.leaping {
            return true;
        }
        if a.ink < self.ink_min {
            return false;
        }
        self.charging = true;
        self.charge = 0.0;
        self.charge_t = 0.0;
        self.charge_air = 0.0;
        self.full = false;
        true
    }

    /// JS `dropCling(r, k, why)` L479-490. `'jump'` hops off the wall,
    /// `'squid'` just releases (no velocity kick), anything else pushes a
    /// little off it.
    pub fn drop_cling(&mut self, a: &mut Actor, why: &str) {
        if !self.cling {
            return;
        }
        self.cling = false;
        self.hang = false;
        let n = self.cling_n;
        if why == "jump" {
            a.vel = vec3(n.x * 3.4, 4.6, n.z * 3.4);
        } else if why != "squid" {
            a.vel = vec3(n.x * 1.2, 0.5, n.z * 1.2);
        }
        a.grounded = false;
    }

    /// JS `jump(r, intent)` L650-660 — the actor.js L311-313 hook. Returns
    /// whether the jump press is swallowed (never becomes a jump). Bots are
    /// Task 39: the `a.isBot` guards collapse to their local-player values.
    pub fn jump(&mut self, a: &mut Actor, intent_fire: bool) -> bool {
        if self.charging || self.leaping {
            return true;
        }
        if self.cling {
            if intent_fire && a.ink >= self.ink_min {
                self.start_charge(a);
            } else {
                self.drop_cling(a, "jump");
            }
            return true;
        }
        if !intent_fire || !a.grounded || a.form != Form::Kid {
            return false;
        }
        self.start_charge(a)
    }
}

/// JS `cancelCharge(r, k)` L363-366.
fn cancel_charge(r: &mut WeaponRunner, k: &mut MittsKit) {
    k.charging = false;
    k.charge = 0.0;
    k.charge_t = 0.0;
    r.charge = 0.0;
}

/// JS `leapVelocity(a, k, c, out)` L275-287 (the bot yaw/pitch override is
/// Task 39). The wall-exit guard (L282-285) keeps a cling leap from
/// launching back into the wall.
#[must_use]
pub fn leap_velocity(a: &Actor, k: &MittsKit, c: f32, w: &Mitts) -> Vec3 {
    let ang = clamp_f(
        w.leap_angle * DEG + a.aim_pitch * w.leap_pitch_k,
        w.leap_angle_min * DEG,
        w.leap_angle_max * DEG,
    );
    let v = lerp(
        w.leap_speed_min * w.leap_speed_min,
        w.leap_speed_max * w.leap_speed_max,
        clamp_f(c, 0.0, 1.0),
    )
    .sqrt();
    let vy = (v * ang.sin()).min(w.leap_vy_max);
    let vh = v * ang.cos();
    let mut dx = a.aim_yaw.sin();
    let mut dz = a.aim_yaw.cos();
    if k.cling {
        let dn = dx * k.cling_n.x + dz * k.cling_n.z;
        if dn < 0.35 {
            dx += k.cling_n.x * (0.35 - dn);
            dz += k.cling_n.z * (0.35 - dn);
            let l = dx.hypot(dz);
            let l = if l == 0.0 { 1.0 } else { l };
            dx /= l;
            dz /= l;
        }
    }
    vec3(dx * vh, vy, dz * vh)
}

/// JS `clingableAt(pos, n, blockId)` L345-353: solid, rising past the head,
/// no floor right under the feet. Deviation: the Rust layout's contacts
/// carry no `rail` / `grate` block flags (spec §2), so every block counts
/// as clingable — railings are climbable solids here anyway.
#[must_use]
pub fn clingable_at(pos: Vec3, n: Vec3, world: &CollisionWorld, t: &PlayerTuning) -> bool {
    let h = world.raycast(
        vec3(pos.x, pos.y + 1.35, pos.z),
        vec3(-n.x, 0.0, -n.z),
        t.radius + 0.45,
        false,
    );
    if !h.hit || h.normal.y.abs() > 0.5 {
        return false;
    }
    !world
        .raycast(vec3(pos.x, pos.y + 0.2, pos.z), Vec3::NEG_Y, 0.5, false)
        .hit
}

/// JS `releaseLeap(r, k)` L368-390 (the `predictLeap` / `leapFx` /
/// `character.trigger` calls are not ported).
#[allow(clippy::too_many_arguments)]
fn release_leap(
    r: &mut WeaponRunner,
    k: &mut MittsKit,
    w: &Mitts,
    a: &mut Actor,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    ps: &mut ProjectileSim,
) {
    let c = k.charge;
    k.charging = false;
    r.charge = 0.0;
    a.ink = (a.ink - lerp(w.leap_ink_min, w.leap_ink_max, c)).max(0.0);
    a.note_fired();
    let v = leap_velocity(a, k, c, w);
    let from_wall = k.cling;
    if from_wall {
        k.cling = false;
        k.hang = false;
    }
    a.vel = v;
    a.grounded = false;
    a.coyote = 0.0;
    a.jump_buffer = 0.0;
    k.leaping = true;
    k.leap_t = 0.0;
    k.slid = false;
    k.leap_v = vec3(v.x, 0.0, v.z);
    k.leap_h = v.x.hypot(v.z);
    k.trail = 0.0;
    k.trail_p = a.pos;
    let q2 = |x: f32| (x * 100.0).round() / 100.0;
    // JS L385 `netRec(a, 'mitts', [2, r2 pos×3, r2 c, fromWall ? r2 n.x : 0,
    // fromWall ? r2 n.z : 0, fromWall ? 1 : 0])`.
    ps.push_event(SimEvent::Kit {
        owner_slot: a.slot,
        team: a.team,
        kind: "mitts".to_string(),
        data: vec![
            2.0,
            q2(a.pos.x),
            q2(a.pos.y),
            q2(a.pos.z),
            q2(c),
            if from_wall { q2(k.cling_n.x) } else { 0.0 },
            if from_wall { q2(k.cling_n.z) } else { 0.0 },
            if from_wall { 1.0 } else { 0.0 },
        ],
    });
    // JS L387: push-off splat (ground leaps only), 1 rng draw.
    if !from_wall {
        let seed = ps.rng.next_f32();
        let area = paint.splat(
            world,
            vec3(a.pos.x, a.pos.y + 0.25, a.pos.z),
            0.9,
            a.team,
            &SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
        if area > 0.0 {
            ps.push_event(SimEvent::Turf {
                owner_slot: a.slot,
                team: a.team,
                area,
                no_special: false,
            });
        }
    }
}

/// Route a mitts hit through the remote mask like [`ProjectileSim`]'s body
/// hits (JS `applyHit` → `'send'` for proxy victims). `a_slot` / `a_team` /
/// `a_pos` are the leaper's identity and *live* position — JS hands the
/// owner actor itself to `applyHit` (L169 / L212), so the `damageTaken`
/// guard arc reads the attacker's current spot (`None` when the roster
/// carries no such actor: the arc test then never matches).
#[allow(clippy::too_many_arguments)]
fn route_hit(
    ei: usize,
    dmg: f32,
    a_slot: usize,
    a_team: usize,
    a_pos: Option<Vec3>,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    remote: Option<&[bool]>,
) {
    let (victim_slot, victim_team) = {
        let Some(v) = foes.get_mut(ei) else {
            return;
        };
        (v.slot, v.team)
    };
    let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
    if send {
        ps.push_event(SimEvent::HitSend {
            attacker_slot: a_slot,
            victim_slot,
            victim_team,
            team: a_team,
            damage: dmg,
        });
    } else {
        let Some(v) = foes.get_mut(ei) else {
            return;
        };
        let outcome = apply_hit(v, dmg, Some((a_team, a_slot)), a_pos, t);
        ps.push_event(SimEvent::Hit {
            attacker_slot: a_slot,
            victim_slot,
            team: a_team,
            damage: dmg,
            killed: outcome.killed,
        });
    }
}

/// JS `landSplash(a, at, n)` L426-461 (the `specials.areaHit` / boss blocks
/// are not ported). 25 rng draws: 1 for the centre seed + 4 per ring blob.
#[allow(clippy::too_many_arguments)]
fn land_splash(
    a: &mut Actor,
    at: Vec3,
    n: Vec3,
    w: &Mitts,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    remote: Option<&[bool]>,
) {
    // JS L428: a moment untouchable on landing.
    if w.land_invuln > 0.0 {
        a.set_invuln(a.invuln().max(w.land_invuln));
    }
    let (a_slot, a_team, a_pos) = (a.slot, a.team, a.pos);
    // JS L430-436: the gloves themselves — whoever it comes down on takes a
    // glove smash on top of the splash.
    if w.glove_damage > 0.0 {
        let mut smash: Vec<usize> = Vec::new();
        for (ei, e) in foes.iter() {
            if e.team == a_team || !e.alive {
                continue;
            }
            let reach = w.glove_radius + t.radius;
            if (e.pos.x - at.x).hypot(e.pos.z - at.z) > reach {
                continue;
            }
            if (e.pos.y + e.smooth_y() + 0.7 - at.y).abs() > 1.6 {
                continue;
            }
            smash.push(ei);
        }
        for ei in smash {
            route_hit(
                ei,
                w.glove_damage,
                a_slot,
                a_team,
                Some(a_pos),
                t,
                ps,
                foes,
                remote,
            );
        }
    }
    let c = at + n * 0.35;
    let mut area = paint.splat(
        world,
        c,
        w.land_paint,
        a_team,
        &SplatOpts {
            seed: ps.rng.next_f32(),
            stretch: None,
            stretch_amt: None,
            kind: None,
        },
    );
    for i in 0..6 {
        // JS L440-444: round the contact in the surface plane. 4 draws per
        // ring blob (angle, radius, splat radius, seed).
        let ang = (i as f32 / 6.0) * std::f32::consts::TAU + ps.rng.next_f32() * 0.5;
        let rr = w.land_paint * (0.75 + ps.rng.next_f32() * 0.35);
        let xa = if n.y > 0.7 {
            Vec3::X
        } else {
            vec3(-n.z, 0.0, n.x).normalize_or(Vec3::X)
        };
        let ya = n.cross(xa);
        let pt = c + xa * (ang.cos() * rr) + ya * (ang.sin() * rr);
        area += paint.splat(
            world,
            pt,
            0.55 + ps.rng.next_f32() * 0.35,
            a_team,
            &SplatOpts {
                seed: ps.rng.next_f32(),
                stretch: None,
                stretch_amt: None,
                kind: None,
            },
        );
    }
    if area > 0.0 {
        ps.push_event(SimEvent::Turf {
            owner_slot: a_slot,
            team: a_team,
            area,
            no_special: false,
        });
    }
    // JS L447-454: the damage ring, falloff from 0.6 m out, line of sight.
    let mut splash: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in foes.iter() {
        if e.team == a_team || !e.alive {
            continue;
        }
        let v2 = vec3(
            e.pos.x,
            e.pos.y + e.smooth_y() + if e.form == Form::Squid { 0.3 } else { 0.7 },
            e.pos.z,
        );
        let d = v2.distance(c);
        if d > w.land_radius || !world.los(c, v2) {
            continue;
        }
        let dmg = lerp(
            w.land_damage_max,
            w.land_damage_min,
            clamp_f((d - 0.6) / (w.land_radius - 0.6), 0.0, 1.0),
        );
        splash.push((ei, dmg));
    }
    for (ei, dmg) in splash {
        route_hit(ei, dmg, a_slot, a_team, Some(a_pos), t, ps, foes, remote);
    }
    let q2 = |x: f32| (x * 100.0).round() / 100.0;
    // JS L458 `netRec(a, 'mitts', [1, r2 at×3, r2 n×3])` — decode-tolerant
    // on the receiving side (no sim state to apply, spec §2).
    ps.push_event(SimEvent::Kit {
        owner_slot: a_slot,
        team: a_team,
        kind: "mitts".to_string(),
        data: vec![1.0, q2(at.x), q2(at.y), q2(at.z), q2(n.x), q2(n.y), q2(n.z)],
    });
    // JS L459 `emit('mitts:land', …)` rides the Impact feed.
    ps.push_event(SimEvent::Impact {
        pos: c,
        normal: n,
        team: a_team,
        radius: w.land_radius,
        victim_slot: None,
        owner_slot: a_slot,
        area: 0.0,
        no_special: false,
    });
}

/// JS `startCling(r, k, n)` L463-477. Deviation: JS L466's bot hang timer
/// (`k.botHold = 1.3 + Math.random() * 1.7`) is not ported (Task 39) — the
/// draw comes off JS's global `Math.random`, not a sim-stream seed, so the
/// ported rng order is unaffected.
#[allow(clippy::too_many_arguments)]
fn start_cling(
    k: &mut MittsKit,
    a: &mut Actor,
    n: Vec3,
    w: &Mitts,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    remote: Option<&[bool]>,
) {
    k.cling = true;
    k.hang = true;
    k.leaping = false;
    k.cling_t = 0.0;
    k.cling_n = vec3(n.x, 0.0, n.z).normalize_or(Vec3::X);
    k.cling_pt = a.pos - k.cling_n * t.radius;
    k.cling_pt.y = a.pos.y + 1.0;
    a.vel = Vec3::ZERO;
    a.grounded = false;
    k.land_t = 0.0;
    // JS L471: the wall hit's landing splash, at the glove's contact point.
    land_splash(
        a, k.cling_pt, k.cling_n, w, world, paint, t, ps, foes, remote,
    );
}

/// JS `gloveMuzzle(a, hand, out)` L114-125 — the no-character fallback
/// branch (L118-122) is the only one the sim can run: a point in front of
/// the chest on the punching side, LOS-guarded.
#[must_use]
pub fn glove_muzzle(a: &Actor, hand: i32, world: &CollisionWorld) -> Vec3 {
    let base = vec3(a.pos.x, a.pos.y + 1.05, a.pos.z);
    let sx = if hand != 0 { 1.0 } else { -1.0 };
    let mut out = base + a.aim_dir() * 0.35;
    out.x += a.yaw.cos() * 0.16 * sx;
    out.z -= a.yaw.sin() * 0.16 * sx;
    if !world.los(base, out) {
        out = base;
    }
    out
}

/// JS `spawnFist(a, w, hand, m, dir, ghost)` L128-136. 1 rng draw (the
/// splat seed); `spin` is presentation.
#[must_use]
pub fn spawn_fist(
    a: &Actor,
    w: &Mitts,
    hand: i32,
    m: Vec3,
    dir: Vec3,
    ghost: bool,
    ps: &mut ProjectileSim,
) -> MittsFist {
    MittsFist {
        owner_slot: a.slot,
        team: a.team,
        pos: m,
        prev: m,
        vel: dir * w.fist_speed,
        dir,
        age: 0.0,
        life: w.fist_range / w.fist_speed,
        hand,
        seed: ps.rng.next_f32(),
        trail: -0.8,
        ghost,
    }
}

/// JS `fireFist(a, w, hand)` L137-146: muzzle, aim, spread, the fist, the
/// `['k','mitts']` record and the fire event.
pub fn fire_fist(
    a: &Actor,
    w: &Mitts,
    hand: i32,
    world: &CollisionWorld,
    fists: &mut Vec<MittsFist>,
    ps: &mut ProjectileSim,
) {
    let m = glove_muzzle(a, hand, world);
    let dir = ProjectileSim::aim_from(a, m, None);
    let dir = ps.rng.spread(dir, w.punch_spread);
    let f = spawn_fist(a, w, hand, m, dir, false, ps);
    fists.push(f);
    let q2 = |x: f32| (x * 100.0).round() / 100.0;
    let q3 = |x: f32| (x * 1000.0).round() / 1000.0;
    // JS L142 `netRec(a, 'mitts', [0, r2 m×3, r3 dir×3, hand])`.
    ps.push_event(SimEvent::Kit {
        owner_slot: a.slot,
        team: a.team,
        kind: "mitts".to_string(),
        data: vec![
            0.0,
            q2(m.x),
            q2(m.y),
            q2(m.z),
            q3(dir.x),
            q3(dir.y),
            q3(dir.z),
            hand as f32,
        ],
    });
    // JS L143 `emit('weapon:fire', …)` — the `['p']`-record feed.
    ps.push_event(SimEvent::Fire {
        owner_slot: a.slot,
        team: a.team,
        muzzle: m,
        dir,
        charge: None,
        len: None,
    });
}

/// JS `burstFist(f, at, direct, normal)` L152-174 (the `specials.areaHit` /
/// boss blocks are not ported). `direct` is the full-list foe index of the
/// body it hit, if any. Ghosts burst for the eye only: the Impact feed is
/// pushed, the paint and damage are not (module docs).
#[allow(clippy::too_many_arguments)]
pub fn burst_fist(
    f: &MittsFist,
    at: Vec3,
    direct: Option<usize>,
    normal: Option<Vec3>,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    w: &Mitts,
    t: &PlayerTuning,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    remote: Option<&[bool]>,
) {
    let c = at;
    if !f.ghost {
        let mut area = 0.0;
        if let Some(n) = normal {
            // JS L158: splat on the surface it hit, 1 rng draw.
            area = paint.splat(
                world,
                at + n * 0.12,
                w.fist_paint * (0.85 + ps.rng.next_f32() * 0.3),
                f.team,
                &SplatOpts {
                    seed: f.seed,
                    stretch: Some(f.dir),
                    stretch_amt: Some(0.45),
                    kind: None,
                },
            );
        } else {
            // JS L160-161: else the ground under it (no rng draw).
            let g = world.raycast(vec3(at.x, at.y + 0.2, at.z), Vec3::NEG_Y, 3.4, true);
            if g.hit {
                area = paint.splat(
                    world,
                    g.point + g.normal * 0.1,
                    w.fist_paint * 0.85,
                    f.team,
                    &SplatOpts {
                        seed: f.seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                );
            }
        }
        if area > 0.0 {
            ps.push_event(SimEvent::Turf {
                owner_slot: f.owner_slot,
                team: f.team,
                area,
                no_special: false,
            });
        }
        // JS L164-170: splash damage to everyone near but the one it hit.
        let mut splash: Vec<(usize, f32)> = Vec::new();
        for (ei, e) in foes.iter() {
            if e.team == f.team || !e.alive || Some(ei) == direct {
                continue;
            }
            let v2 = vec3(
                e.pos.x,
                e.pos.y + e.smooth_y() + if e.form == Form::Squid { 0.3 } else { 0.7 },
                e.pos.z,
            );
            let d = v2.distance(c);
            if d > w.splash_radius || !world.los(c, v2) {
                continue;
            }
            let dmg = lerp(
                w.splash_max,
                w.splash_min,
                clamp_f(d / w.splash_radius, 0.0, 1.0),
            );
            splash.push((ei, dmg));
        }
        // JS L169: `applyHit(o, e, …)` — the attacker position is the
        // owner's *live* actor (the `damageTaken` guard arc), looked up by
        // slot + team in the full roster the match hands the fist scan.
        let owner_pos = foes
            .iter()
            .find(|(_, e)| e.slot == f.owner_slot && e.team == f.team)
            .map(|(_, e)| e.pos);
        for (ei, dmg) in splash {
            route_hit(
                ei,
                dmg,
                f.owner_slot,
                f.team,
                owner_pos,
                t,
                ps,
                foes,
                remote,
            );
        }
    }
    // JS `burstFx` L185 `emit('weapon:impact', …)` — pushed for ghosts too.
    let victim_slot =
        direct.and_then(|ei| foes.iter().find(|(i, _)| *i == ei).map(|(_, e)| e.slot));
    ps.push_event(SimEvent::Impact {
        pos: c,
        normal: normal.unwrap_or(Vec3::Y),
        team: f.team,
        radius: w.splash_radius,
        victim_slot,
        owner_slot: f.owner_slot,
        area: 0.0,
        no_special: false,
    });
}

impl MittsFist {
    /// JS `stepFist(f, i, dt)` L197-246. Returns `false` when the fist is
    /// spent and the match must remove it (JS `dead` L244).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        f: &mut Self,
        dt: f32,
        world: &CollisionWorld,
        paint: &mut PaintGrid,
        w: &Mitts,
        t: &PlayerTuning,
        ps: &mut ProjectileSim,
        foes: &mut FoeScan<'_>,
        remote: Option<&[bool]>,
    ) -> bool {
        f.age += dt;
        f.prev = f.pos;
        f.pos += f.vel * dt;
        // JS L205-217: bodies — the first enemy capsule takes the direct
        // hit and the burst.
        let mut body: Option<(usize, Vec3)> = None;
        for (ei, e) in foes.iter() {
            if e.team == f.team || !e.alive {
                continue;
            }
            let h = if e.form == Form::Squid {
                t.squid_height
            } else {
                t.height
            };
            let hr = t.radius;
            if (e.pos.x - f.pos.x).abs() > 3.0 + hr || (e.pos.z - f.pos.z).abs() > 3.0 + hr {
                continue;
            }
            let base = vec3(e.pos.x, e.pos.y + e.smooth_y(), e.pos.z);
            let res = segment_capsule_dist(f.prev, f.pos, base, hr, h);
            if res.dist < hr * 0.95 + w.fist_size {
                body = Some((ei, f.prev.lerp(f.pos, res.t)));
                break;
            }
        }
        if let Some((ei, at)) = body {
            if !f.ghost {
                // JS L212: `applyHit(f.owner, e, …)` — the guard arc reads
                // the owner's live position, not the hit point.
                let owner_pos = foes
                    .iter()
                    .find(|(_, e)| e.slot == f.owner_slot && e.team == f.team)
                    .map(|(_, e)| e.pos);
                route_hit(
                    ei,
                    w.punch_damage,
                    f.owner_slot,
                    f.team,
                    owner_pos,
                    t,
                    ps,
                    foes,
                    remote,
                );
            }
            burst_fist(f, at, Some(ei), None, world, paint, w, t, ps, foes, remote);
            return false;
        }
        // JS L219-226: boss / device catches — not ported (Tasks 34-36).
        // JS L228-231: the level.
        let hit = world.segment(f.prev, f.pos, true);
        if hit.hit {
            burst_fist(
                f,
                hit.point,
                None,
                Some(hit.normal),
                world,
                paint,
                w,
                t,
                ps,
                foes,
                remote,
            );
            return false;
        }
        // JS L233: end of its reach — bursts in the air.
        if f.age >= f.life {
            burst_fist(f, f.pos, None, None, world, paint, w, t, ps, foes, remote);
            return false;
        }
        // JS L234: the sea swallows it silently.
        if f.pos.y < t.water_y - 1.5 {
            return false;
        }
        // JS L236-243: a thin drip trail under it. 2 rng draws per drip.
        f.trail += w.fist_speed * dt;
        if f.trail > 1.5 {
            f.trail = 0.0;
            let g = world.raycast(f.pos, Vec3::NEG_Y, 3.0, true);
            if g.hit && !f.ghost {
                let radius = 0.28 + ps.rng.next_f32() * 0.1;
                let seed = ps.rng.next_f32();
                let area = paint.splat(
                    world,
                    g.point + g.normal * 0.08,
                    radius,
                    f.team,
                    &SplatOpts {
                        seed,
                        stretch: None,
                        stretch_amt: None,
                        kind: None,
                    },
                );
                if area > 0.0 {
                    ps.push_event(SimEvent::Turf {
                        owner_slot: f.owner_slot,
                        team: f.team,
                        area,
                        no_special: false,
                    });
                }
            }
        }
        true
    }
}

/// JS `update(r, dt, inp, w)` L546-646. `jump_held` is the raw
/// `a.intent.jump` (the release condition, L574 — *not* gated); `ps_time`
/// is [`ProjectileSim::time`] — the kit's `G.time` stand-in.
#[allow(clippy::too_many_arguments)]
pub fn update(
    r: &mut WeaponRunner,
    dt: f32,
    gate: &FireGate,
    jump_held: bool,
    a: &mut Actor,
    w: &Mitts,
    world: &CollisionWorld,
    paint: &mut PaintGrid,
    t: &PlayerTuning,
    fists: &mut Vec<MittsFist>,
    ps: &mut ProjectileSim,
    foes: &mut FoeScan<'_>,
    remote: Option<&[bool]>,
    ps_time: f32,
) {
    let mut k = r.mitts.take().map(|m| *m).unwrap_or_default();
    // JS L549: the body was owned by something else (death, a special …) —
    // nothing carries over.
    if k.seen > 0.0 && ps_time - k.seen > 0.25 {
        if k.charging {
            cancel_charge(r, &mut k);
        }
        k.leaping = false;
        if k.cling {
            k.drop_cling(a, "reset");
        }
    }
    k.seen = ps_time;
    k.ink_min = w.leap_ink_min;
    k.cd -= dt;
    k.land_t += dt;
    // JS L552 `a.specialActive` cling drop: Task 35 (no specials in the
    // sim yet).
    if a.form == Form::Squid {
        if k.cling {
            k.drop_cling(a, "squid");
        }
        if k.charging {
            cancel_charge(r, &mut k);
        }
    }

    // ---- leap charge (JS L556-579; the bot want / release gates are
    // Task 39).
    if k.charging {
        k.charge_t += dt;
        let max_c = clamp_f(
            (a.ink - w.leap_ink_min) / (w.leap_ink_max - w.leap_ink_min),
            0.0,
            1.0,
        );
        k.charge = max_c.min(k.charge_t / w.leap_charge_time);
        r.charge = k.charge;
        a.note_fired();
        if k.charge >= 0.999 {
            k.full = true;
        }
        // JS L572: walked off a ledge mid-charge — the crouch has nothing
        // to push off.
        if !a.grounded && !k.cling {
            k.charge_air += dt;
            if k.charge_air > 0.12 {
                cancel_charge(r, &mut k);
            }
        } else {
            k.charge_air = 0.0;
        }
        if k.charging && !jump_held {
            release_leap(r, &mut k, w, a, world, paint, ps);
        }
    }

    // ---- leap in flight (JS L582-612, after this frame's integrate:
    // contacts are fresh).
    if k.leaping {
        k.leap_t += dt;
        if a.grounded {
            k.leaping = false;
            k.land_t = 0.0;
            let n = if a.ground_n.y > 0.5 {
                a.ground_n
            } else {
                Vec3::Y
            };
            land_splash(a, a.pos, n, w, world, paint, t, ps, foes, remote);
        } else if a.form != Form::Kid || k.leap_t > 4.0 {
            k.leaping = false;
        } else {
            let c = a.contacts;
            if !k.slid && c.wall && k.leap_t > 0.03 {
                let into = -(k.leap_v.x * c.wall_normal.x + k.leap_v.z * c.wall_normal.z);
                if into > 1.5 && clingable_at(a.pos, c.wall_normal, world, t) {
                    start_cling(
                        &mut k,
                        a,
                        c.wall_normal,
                        w,
                        world,
                        paint,
                        t,
                        ps,
                        foes,
                        remote,
                    );
                } else {
                    k.slid = true; // physics takes it from here
                }
            }
            if k.leaping {
                // JS L598-599: hold the launch's horizontal speed unless
                // something else moved us.
                if !k.slid && (a.vel.x - k.leap_v.x).hypot(a.vel.z - k.leap_v.z) > 2.2 {
                    k.slid = true;
                }
                if !k.slid {
                    a.vel.x = k.leap_v.x;
                    a.vel.z = k.leap_v.z;
                }
                a.note_fired();
                // JS L602-607: drips painted under the path. 2 rng draws.
                k.trail += a.pos.distance(k.trail_p);
                k.trail_p = a.pos;
                if k.trail > 1.6 {
                    k.trail = 0.0;
                    let g = world.raycast(
                        vec3(a.pos.x, a.pos.y + 0.3, a.pos.z),
                        Vec3::NEG_Y,
                        5.0,
                        true,
                    );
                    if g.hit {
                        let radius = 0.42 + ps.rng.next_f32() * 0.15;
                        let seed = ps.rng.next_f32();
                        let area = paint.splat(
                            world,
                            g.point + g.normal * 0.08,
                            radius,
                            a.team,
                            &SplatOpts {
                                seed,
                                stretch: None,
                                stretch_amt: None,
                                kind: None,
                            },
                        );
                        if area > 0.0 {
                            ps.push_event(SimEvent::Turf {
                                owner_slot: a.slot,
                                team: a.team,
                                area,
                                no_special: false,
                            });
                        }
                    }
                }
                // JS L609 `telegraph` is presentation (D-T33-1).
            }
        }
    }

    // ---- wall cling (JS L615-629; the bot backstop is Task 39).
    if k.cling {
        k.cling_t += dt;
        a.vel = Vec3::ZERO;
        a.note_fired(); // holding on: no idle refill
        a.ink = (a.ink - w.cling_drain * dt).max(0.0);
        // JS L624-625: does the wall still hold the glove?
        let wall_there = world
            .raycast(
                vec3(a.pos.x, a.pos.y + 1.0, a.pos.z),
                vec3(-k.cling_n.x, 0.0, -k.cling_n.z),
                t.radius + 0.5,
                false,
            )
            .hit;
        if a.ink <= 0.0 {
            k.drop_cling(a, "empty");
        } else if a.grounded || !wall_there || !a.alive {
            k.drop_cling(a, "lost");
        }
    }

    // ---- punches (JS L632-645: not while charging or mid-leap; from the
    // wall only the right glove is free).
    if !gate.fire || k.charging || k.leaping {
        if k.cd < 0.0 {
            k.cd = 0.0;
        }
    } else {
        r.firing_t = FIRING_T;
        let mut guard = 0;
        while k.cd <= 0.0 && guard < 2 {
            guard += 1;
            if a.ink < w.ink_per_punch {
                r.empty_click(ps, a);
                k.cd += w.punch_interval;
                break;
            }
            a.ink -= w.ink_per_punch;
            a.note_fired();
            k.side = if k.cling { 0 } else { 1 - k.side };
            fire_fist(a, w, k.side, world, fists, ps);
            r.since_hand[k.side as usize] = 0.0;
            k.cd += w.punch_interval;
        }
    }

    // JS `MAIN_KITS.mitts.damageTaken` L883-903 as a per-frame snapshot
    // (spec §4): the leap armour and the guard arc read the same values.
    a.kit_armor = Some(KitArmor {
        leaping: k.leaping,
        land_t: k.land_t,
        guard_up: !k.cling && a.form == Form::Kid && r.firing_t > FIRING_T - w.guard_after,
        leap_armor: w.leap_armor,
        leap_grace: w.leap_armor_grace,
        guard_armor: w.guard_armor,
        guard_arc_cos: (w.guard_arc * DEG).cos(),
    });

    r.mitts = Some(Box::new(k));
}
