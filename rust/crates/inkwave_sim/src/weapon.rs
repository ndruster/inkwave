//! Weapon runner (per-actor, kind-dispatched) and the generalized
//! projectile system (sim layer).
//!
//! Faithful port of `src/game/weapons.js`: the per-actor `WeaponRunner`
//! (`reset` L37, `busy` L62, `moveSpeed` L63-92, `_spreadDeg` L94-107, the
//! `update` kind dispatch L108-156, the shared auto-fire ink path `_auto`
//! L165-182) and the global `Projectiles` (L776: `_step` L1583-1648 —
//! straight phase → gravity + drag, capsule hit test with the round's own
//! `size`, world segment hit, trail drips, impact splat — plus the
//! shooter-family spawn `fireShooter` L1027-1046 generalized through
//! [`RoundSpec`]).
//!
//! Keep in lockstep with upstream on sync:
//!   - `WeaponRunner.update` common decay (L110-116) -> [`WeaponRunner::update`]
//!   - `WeaponRunner.update` kind dispatch (L117-129) -> [`WeaponRunner::update`]
//!   - `WeaponRunner._auto` (ink spend + empty-click lockout)
//!     -> [`WeaponRunner::auto_fire`]
//!   - `WeaponRunner._spreadDeg` (L94-107)          -> [`WeaponRunner::spread_deg`]
//!   - `WeaponRunner.moveSpeed` (L63-92)            -> [`WeaponRunner::move_speed`]
//!   - `WeaponRunner.busy` (L62)                    -> [`WeaponRunner::busy`]
//!   - `Projectiles.fireShooter` / `_fireRound`     -> [`ProjectileSim::fire_round`]
//!     (the per-round physics fields: `projSpeed` / `grav` / `drag` /
//!     `straightTime` / `size` / `impactRadius` / `trail*` / `life`)
//!   - `Projectiles._muzzle` (rig fallback)  -> [`ProjectileSim::muzzle`]
//!   - `Projectiles._aimFrom`                -> [`ProjectileSim::aim_from`]
//!   - `Projectiles._spread`                 -> [`Rng::spread`]
//!   - `Projectiles._ballistic`              -> [`ballistic`]
//!   - `Projectiles._step` (shot type)       -> [`ProjectileSim::step_one`]
//!   - `Projectiles._impact` (non-slosh)     -> inlined in [`ProjectileSim::step_one`]
//!   - `MAX_BLOBS` (L630)                    -> [`MAX_ROUNDS`]
//!   - `Physics.pointCapsuleDist` / `segmentCapsuleDist`
//!     -> [`point_capsule_dist`] / [`segment_capsule_dist`]
//!   - `hitBase(e)` (visual feet)            -> [`ProjectileSim::step_one`]
//!   - `applyHit` (local route only)         -> [`apply_hit`]
//!
//! Scope (Task 29): the runner dispatches over all 12 roster kinds
//! ([`WeaponKind::ALL`]); only `Shooter` fires for real. The other kinds'
//! update branches land in Tasks 30-33 on top of this scaffold — the shared
//! state fields ([`WeaponRunner`] charge / roll / flick / burst / turret /
//! lock / dodge timers) and the full `moveSpeed` / `_spreadDeg` tables are
//! ported now so no weapon repeats the scaffolding. Sub weapons
//! (JS `update` L130-155), specials, boss and kit dispatch are out of scope.
//!
//! Intentional deviations from JS (recorded for review):
//!   - `Math.random()` is replaced by the explicit [`Rng`] stream (sim-layer
//!     determinism rule). The order of the draws the sim actually performs
//!     matches JS (spread cone `sqrt(u1)`/`u2`, then `seed`, then the
//!     impact-radius jitter at hit time), but the values differ from a JS
//!     run. JS also draws one presentation-only `vis` random per shot
//!     (weapons.js L1035); the sim drops that field and does not consume it.
//!   - `range` is enforced as a flight-time cap (`life = range / projSpeed`,
//!     the same formula JS uses for the blaster blast at weapons.js L1178)
//!     plus the ballistic solver's `maxDist` gate. JS shooter rounds live for
//!     a fixed 1.2 s and only drop under the sea at `waterY − 1.8`; the spec
//!     TR-7.3 requires the 12.5 m cut-off, so the sim expires the round at
//!     `range / projSpeed` instead. The sea-drop rule is kept as well.
//!   - `p.size` rides the round as a per-spec field ([`RoundSpec::size`]);
//!     the GPU look fields (`vis/tail0/tailK/wob/wobF/nose/sats`) are
//!     presentation-only and dropped.
//!   - The character rig's muzzle (`character.getMuzzle` / `aimReady`) does
//!     not exist in the sim: [`ProjectileSim::muzzle`] uses JS's own fallback
//!     branch (eye position + a short push along the aim direction), with the
//!     form-dependent eye height kept (JS L936: `squid ? 0.4 : 1.05`).
//!   - `emit('weapon:fire' / 'weapon:impact' / 'hit' / 'lowink')` events are
//!     surfaced as the [`SimEvent`] queue on [`ProjectileSim`] instead of a
//!     global bus; the JS `_credit(p, area)` → `owner.addTurf(area)` routing
//!     is the Task 8 match layer's job — the claimed m² rides the
//!     [`SimEvent::Impact`] / [`SimEvent::Turf`] payloads to it.

pub mod brolly;

use std::collections::BTreeMap;

use glam::{Vec3, vec3};

use crate::actor::{Actor, FireGate, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{Blaster, PlayerTuning, Spritzer, Twins, WeaponSet};
use crate::weapon::brolly::{BrollyKit, CanopyState, LaunchedCanopy};

/// Upstream projectile gravity for shooter-family rounds (weapons.js
/// `fireShooter`: `grav: 28`), distinct from the player's `gravity: 25`.
pub const PROJ_GRAV: f32 = 28.0;
/// Upstream projectile air drag for shooter-family rounds (`drag: 0.8`).
pub const PROJ_DRAG: f32 = 0.8;
/// Teardrop visual radius used in the hit test (`size: 0.15`).
pub const PROJ_SIZE: f32 = 0.15;
/// Shooter round lifetime cap in JS (JS `life: 1.2`); the sim takes the
/// `range / projSpeed` cap instead when smaller (see module docs).
pub const SHOT_LIFE: f32 = 1.2;
/// Bloom recovery time constant (JS `w.bloomRecover ?? 0.28`).
pub const BLOOM_RECOVER: f32 = 0.28;
/// Bloom added per shot (JS `w.bloomPerShot ?? 0.3`).
pub const BLOOM_PER_SHOT: f32 = 0.3;
/// First-shot spread factor (JS `w.spreadFirst ?? 0.45`).
pub const SPREAD_FIRST: f32 = 0.45;
/// `firingT` pose timer set on trigger (JS `_auto`: `this.firingT = 0.35`).
pub const FIRING_T: f32 = 0.35;
/// Empty-reservoir click cooldown (JS `_empty`: `emptyCd = 0.45`).
pub const EMPTY_CD: f32 = 0.45;
/// Trail starts this far behind the muzzle so shots don't drip on the
/// shooter's own feet (JS `trail: -(2.5 - w.trailEvery)`).
pub const TRAIL_START_BACK: f32 = 2.5;
/// Muzzle fallback: eye height above `pos` for the kid form
/// (JS `_muzzle` degenerate branch: `_v3.y += a.form === 'squid' ? 0.4 : 1.05`).
pub const MUZZLE_EYE_Y: f32 = 1.05;
/// Muzzle fallback eye height for the squid form (JS `_muzzle` L936 `0.4`).
pub const MUZZLE_EYE_Y_SQUID: f32 = 0.4;
/// Muzzle fallback distance along `aimDir` (JS `addScaledVector(a.aimDir, 0.3)`).
pub const MUZZLE_FALLBACK_DIST: f32 = 0.3;
/// Horizontal quick-reject radius around each victim (JS `_step`: `3 + hr`).
pub const HIT_REJECT: f32 = 3.0;
/// Generous hitbox: fraction of the victim radius plus the blob size
/// (JS `_res.dist < hr * 0.95 + p.size`).
pub const HITBOX_R_MUL: f32 = 0.95;
/// Degrees → radians.
const DEG: f32 = std::f32::consts::PI / 180.0;
/// Reference integrator step for the ballistic solver (JS `SIM_DT = 1/60`).
const SIM_DT: f32 = 1.0 / 60.0;
/// JS `Projectiles` instanced-draw cap `MAX_BLOBS = 700` (weapons.js L630).
/// Upstream never culls the sim list — only `_draw` stops emitting past the
/// cap (L1880) — so the sim keeps every round alive; this constant is the
/// presentation budget and the count guard for the tests.
pub const MAX_ROUNDS: usize = 700;

/// JS `lerp` (ctx.js): linear interpolation.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// JS `smoothstep` (ctx.js).
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The 12 roster weapon kinds — the `kind` field of the `WEAPON_ORDER`
/// (config.js L303) weapons. JS dispatches the runner on `w.kind`
/// (weapons.js L117-129); the retired defs (`dualies` / `slosher` /
/// `splatling`, config.js L266-299) and the kit-weapon fallback (JS L128
/// `MAIN_KITS[w.kind]`) are deliberately absent — `Brolly` / `Bow` /
/// `Blade` / `Mitts` are the JS kit kinds whose behaviour lands in
/// Tasks 30-33.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeaponKind {
    /// `shooter` — Spritzer stream rounds (wired, M1).
    Shooter,
    /// `blaster` — ammo-segment blast (Task 30).
    Blaster,
    /// `charger` — charge → beam + drops (Task 31).
    Charger,
    /// `roller` — drum roll + flick drops (Task 32).
    Roller,
    /// `bucket` — arcing blob volleys (Task 31).
    Bucket,
    /// `spinner` — spin-up stream (Task 31).
    Spinner,
    /// `twins` — alternating-hand pistols + dodge roll (Task 30).
    Twins,
    /// `brush` — drag stroke + swipe (Task 32).
    Brush,
    /// `brolly` — open/close canopy (Task 30).
    Brolly,
    /// `bow` — charged arrow (Task 33).
    Bow,
    /// `blade` — dash slash (Task 32).
    Blade,
    /// `mitts` — fists + guard + leap (Task 33).
    Mitts,
}

impl WeaponKind {
    /// The 12 kinds in `WEAPON_ORDER` order (config.js L303).
    pub const ALL: [Self; 12] = [
        Self::Shooter,
        Self::Twins,
        Self::Brolly,
        Self::Blaster,
        Self::Spinner,
        Self::Charger,
        Self::Bow,
        Self::Roller,
        Self::Brush,
        Self::Blade,
        Self::Mitts,
        Self::Bucket,
    ];

    /// JS `w.kind` string → enum. `None` for retired / unknown kinds
    /// (the retired defs never enter the tuning artifact — TR-28.2).
    #[must_use]
    pub fn from_kind_str(s: &str) -> Option<Self> {
        Some(match s {
            "shooter" => Self::Shooter,
            "blaster" => Self::Blaster,
            "charger" => Self::Charger,
            "roller" => Self::Roller,
            "bucket" => Self::Bucket,
            "spinner" => Self::Spinner,
            "twins" => Self::Twins,
            "brush" => Self::Brush,
            "brolly" => Self::Brolly,
            "bow" => Self::Bow,
            "blade" => Self::Blade,
            "mitts" => Self::Mitts,
            _ => return None,
        })
    }

    /// Whether this kind's update branch is wired (Task 29: `Shooter`;
    /// Task 30: `Twins` / `Blaster` / `Brolly`; Tasks 31-33 flip the rest).
    #[must_use]
    pub fn is_wired(self) -> bool {
        matches!(
            self,
            Self::Shooter | Self::Twins | Self::Blaster | Self::Brolly
        )
    }
}

/// Per-round launch parameters for the generalized shooter-family spawn
/// (JS `fireShooter` L1027-1046 / `_fireRound` L1068-1082: the same
/// `Object.assign` field set, values from the weapon def). `grav` / `drag`
/// / `size` / `life` are the hardcoded upstream shot constants
/// (weapons.js L1034: `28, 0.8, size 0.15, life 1.2`); later kinds pass
/// their own (e.g. brolly pellets use `pelletGrav` / `pelletSize`).
#[derive(Debug, Clone, Copy)]
pub struct RoundSpec {
    pub proj_speed: f32,
    pub straight_time: f32,
    pub range: f32,
    pub grav: f32,
    pub drag: f32,
    pub size: f32,
    pub life: f32,
    pub impact_radius: f32,
    pub damage: f32,
    pub trail_every: f32,
    pub trail_radius: f32,
    /// Drip accumulator start (JS `p.trail` at spawn: shooter `-(2.5 −
    /// trailEvery)` L1034, blaster blast `-1.5` L1178, twin `-(2.4 −
    /// trailEvery)` L1381, brolly pellet `-1.5` via `fireCustom` L1204).
    pub trail_start: f32,
}

impl RoundSpec {
    /// JS `fireShooter(a, w, spreadDeg)` — the Spritzer round (config.js
    /// L95-105 `WEAPONS.shooter` + the L1034 shot constants).
    #[must_use]
    pub fn shooter(w: &Spritzer) -> Self {
        Self {
            proj_speed: w.proj_speed,
            straight_time: w.straight_time,
            range: w.range,
            grav: PROJ_GRAV,
            drag: PROJ_DRAG,
            size: PROJ_SIZE,
            life: SHOT_LIFE,
            impact_radius: w.impact_radius,
            damage: w.damage,
            trail_every: w.trail_every,
            trail_radius: w.trail_radius,
            trail_start: -(TRAIL_START_BACK - w.trail_every),
        }
    }
}

/// Distance-falloff mark carried by a brolly pellet (JS `p._bid` + the
/// kit's per-pellet `pelletDamage(p.start.distanceTo(at))`, brolly.js
/// L64-68 / L471). `blast` is the blast group id: pellets of one blast that
/// reach the same victim within [`MERGE`] sum into one hit (brolly.js L468,
/// key `blast * 64 + victimIdx`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PelletMark {
    pub blast: u32,
    pub falloff_start: f32,
    pub falloff_end: f32,
    pub dmg_near: f32,
    pub dmg_far: f32,
}

impl PelletMark {
    /// JS `pelletDamage(d)` (brolly.js L64-68): full damage up to
    /// `falloff_start`, then a linear lerp to `dmg_far` at `falloff_end`.
    #[must_use]
    pub fn pellet_damage(&self, d: f32) -> f32 {
        if d <= self.falloff_start {
            return self.dmg_near;
        }
        lerp(
            self.dmg_near,
            self.dmg_far,
            clamp_f(
                (d - self.falloff_start) / (self.falloff_end - self.falloff_start),
                0.0,
                1.0,
            ),
        )
    }
}

/// Seedable xorshift32 RNG (sim-layer determinism: no `Math.random`).
/// One stream per [`ProjectileSim`]; every actor's shots draw from it in
/// step order (the spec's "seeded RNG" contract, AC-6).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    s: u32,
}

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        // xorshift32 requires a non-zero state.
        let mut s = (seed as u32) ^ ((seed >> 32) as u32);
        if s == 0 {
            s = 0x9e37_79b9;
        }
        Self { s }
    }

    #[must_use]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.s = x;
        x
    }

    /// Uniform in [0, 1) (JS `Math.random()` replacement).
    #[must_use]
    pub fn next_f32(&mut self) -> f32 {
        // 24 high bits → [0, 2^24) / 2^24.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// JS `Projectiles._spread`: cone around `dir` with half-angle
    /// `deg · sqrt(random)`, azimuth `random · 2π`; the vertical component of
    /// the cone is flattened by 0.55 as upstream.
    pub fn spread(&mut self, dir: Vec3, deg: f32) -> Vec3 {
        if deg <= 0.0 {
            return dir;
        }
        let r = deg * DEG * self.next_f32().sqrt();
        let t = self.next_f32() * std::f32::consts::TAU;
        // perpendicular in the horizontal plane (JS: (-z, 0, x), fallback (1,0,0))
        let mut p = vec3(-dir.z, 0.0, dir.x);
        if p.length_squared() < 1e-4 {
            p = Vec3::X;
        }
        p = p.normalize();
        let q = dir.cross(p);
        (dir + p * (t.cos() * r.tan()) + q * (t.sin() * r.tan() * 0.55)).normalize()
    }
}

/// One ink round in flight (JS projectile `type: 'shot'`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Projectile {
    pub pos: Vec3,
    pub prev: Vec3,
    pub start: Vec3,
    pub vel: Vec3,
    pub team: usize,
    /// Firing actor's **identity slot** (`Actor::slot`), used only as the
    /// `attacker_slot` payload in [`SimEvent::Hit`] / [`SimEvent::Fire`].
    /// It is NOT an array index into the actor list — never use it to
    /// subscript `actors` (the slot ≠ index case is exercised in the tests).
    pub owner_slot: usize,
    pub age: f32,
    pub life: f32,
    /// Straight-line phase duration before gravity kicks in (JS `straight`).
    pub straight: f32,
    /// Paint radius before the impact-time jitter (JS `radius: w.impactRadius`).
    pub radius: f32,
    /// Round hit radius (JS `p.size` — the blob's own radius added to the
    /// generous hitbox, `_step` L1598; `0.15` for shooter-family rounds).
    pub size: f32,
    pub damage: f32,
    pub grav: f32,
    pub drag: f32,
    /// Drip accumulator in metres of travel (JS `p.trail`).
    pub trail: f32,
    pub trail_every: f32,
    pub trail_radius: f32,
    /// Deterministic blob seed (JS `p.seed`).
    pub seed: f32,
    /// Visual-only replay of someone else's round (JS `p.ghost`,
    /// netmatch.js `ghostProjectile` L867-875): it never paints and never
    /// damages (JS wraps its step in `nm.mute++/--`, weapons.js L1572-1574,
    /// and `damage: 0` skips `applyHit`, L1608), but it still integrates
    /// motion and despawns on the same hit/life/water expiry as a live round.
    #[serde(default)]
    pub ghost: bool,
    /// Blaster blast (JS `type: 'blast'`, weapons.js L1178): on any death it
    /// detonates [`ProjectileSim::blast_burst`] (JS `_blastBurst` L1689-1709)
    /// instead of the plain `weapon:impact`.
    #[serde(default)]
    pub blast: bool,
    /// Brolly pellet mark (JS `p._bid` + the kit's `PELLETS` side table,
    /// brolly.js L72/L105): body hits merge per blast+victim and score with
    /// distance falloff instead of the round's `damage` (which is 0 — the
    /// core pass never applies it, JS L1608 `if (dmg > 0)`).
    #[serde(default)]
    pub pellet: Option<PelletMark>,
}

/// Gameplay events produced by the weapon layer (JS `emit(...)`). The match
/// layer drains these for scoring/FX/netcode (Task 8, [`crate::match_`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SimEvent {
    /// A round left the muzzle (JS `weapon:fire`). `team` joins `owner_slot`
    /// into the full firing identity (slots repeat per team), needed by the
    /// net recorder to pair the muzzle with its roster entry (Task 23).
    Fire {
        owner_slot: usize,
        team: usize,
        muzzle: Vec3,
        dir: Vec3,
    },
    /// A round hit the world or a body (JS `weapon:impact`). `victim_slot` is
    /// `Some` only for body hits, where the splat radius is halved. `area` is
    /// the m² newly claimed by the splat (JS `_credit(p, area)` →
    /// `owner.addTurf(area)`; `0` for body hits, which paint nothing), and
    /// `owner_slot` is the firing actor's identity slot so the match layer
    /// can route the credit (resolved together with `team`, never alone).
    Impact {
        pos: Vec3,
        normal: Vec3,
        team: usize,
        radius: f32,
        victim_slot: Option<usize>,
        owner_slot: usize,
        area: f32,
    },
    /// A trail drip painted the ground (JS `_step` trail branch → `_credit`).
    /// No `weapon:impact` is emitted for drips upstream, so the turf credit
    /// rides its own event.
    Turf {
        owner_slot: usize,
        team: usize,
        area: f32,
    },
    /// A round hit an actor (JS `hit`). `killed` reflects the victim state
    /// *after* the damage was applied (JS `applyHit` → `victim.damage()`).
    /// `team` is the attacker's team, so the match layer can resolve the
    /// identity slots to roster indices unambiguously (slots repeat per team).
    Hit {
        attacker_slot: usize,
        victim_slot: usize,
        team: usize,
        damage: f32,
        killed: bool,
    },
    /// A round hit a *remote* (proxy) actor — JS `shouldApplyHit` → `'send'`
    /// (netmatch.js L711-717, `applyHit` L1551): the shooter's client decides
    /// the hit, the victim's owner applies it over the `{k:'hit'}` channel
    /// (Task 21). No damage is applied locally and `killed` is not known yet
    /// (JS: the kill confirm arrives with the victim's splat replay).
    HitSend {
        attacker_slot: usize,
        victim_slot: usize,
        /// Victim's team — the proxy identity `(victim_team, victim_slot)`
        /// resolves to its roster entry (the attacker's `team` alone only
        /// identifies the shooter).
        victim_team: usize,
        team: usize,
        damage: f32,
    },
    /// Trigger held but the reservoir is below `ink_per_shot` (JS
    /// `_empty()` → `emit('lowink')` + `empty_click`).
    LowInk { owner_slot: usize },
    /// A kit object entered the world (JS `netRec(a, 'brolly', [...])`,
    /// brolly.js L266 → netmatch `recKit` L134-137 `['k', nid, kind, data]`).
    /// `owner_slot` + `team` carry the owner identity like [`Self::Fire`]
    /// (the replication layer maps the roster index to the wire `nid` byte —
    /// Task 30 §4). `data` is the launch payload: `[x, y, z, dx, dz, hp, h0]`
    /// with the JS `r2`/`r3`/`round` quantisation applied (brolly.js L266).
    Kit {
        owner_slot: usize,
        team: usize,
        kind: String,
        data: Vec<f32>,
    },
}

/// Per-actor weapon state machine (JS `WeaponRunner` L32): the common
/// timers + the per-kind state fields the `moveSpeed` / `_spreadDeg` /
/// `busy` tables read. Task 29 wires the `Shooter` branch; the other kinds'
/// update branches (Tasks 30-33) set these fields in place.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeaponRunner {
    cooldown: f32,
    empty_cd: f32,
    firing_t: f32,
    bloom: f32,
    /// Current shot cone half-angle, degrees (JS `this.spread`).
    pub spread: f32,
    // ---- Task 29 scaffold: JS `reset()` L37-61 fields, written by the
    // per-kind update branches landing in Tasks 30-33.
    /// JS `charging` — a charge weapon is holding the trigger.
    charging: bool,
    /// JS `charge` — charge fraction `0..1`.
    charge: f32,
    /// JS `rolling` — roller drum / brush stroke in progress.
    rolling: bool,
    /// JS `rollT` — seconds the roll/stroke has been held.
    roll_t: f32,
    /// JS `flick` — flick wind-up timer, `-1` = idle (JS L39 sentinel).
    flick: f32,
    /// JS `flickRecover` — post-flick recovery timer.
    flick_recover: f32,
    /// JS `burstT` — spinner release stream left.
    burst_t: f32,
    /// JS `turret` — twins planted stance.
    turret: bool,
    /// JS `lockT` — post-roll lockout timer.
    lock_t: f32,
    /// JS `dodgeT` — dodge-roll timer.
    dodge_t: f32,
    // ---- Task 30: twins dodge-roll chain (JS `reset()` L47-51).
    /// JS `rollLock` — chain lockout timer.
    roll_lock: f32,
    /// JS `sinceRoll` — seconds since the last roll (reset to 9).
    since_roll: f32,
    /// JS `rollsUsed` — rolls used in the current chain.
    rolls_used: u32,
    /// JS `rollQueued` — a second roll tapped mid-roll chains on.
    roll_queued: bool,
    /// JS `twinSide` — alternating hand (reset to 1).
    twin_side: i32,
    /// JS `sinceHand` — per-hand shot clocks, seconds (reset to `[99, 99]`).
    since_hand: [f32; 2],
    /// JS `dodgeDir` — the twins roll direction (unit XZ).
    dodge_dir: Vec3,
    /// JS `streaming` — splatling stream in progress (net flag; written by
    /// the splatling branch, Task 33).
    streaming: bool,
    /// JS `burstFrac` — stream left fraction (the `ch` tick slot while
    /// streaming, Task 33).
    burst_frac: f32,
    // ---- Task 30: brolly kit state (JS `this.kit`, weapons.js L57; the
    // `kit = null` on reset is modelled by `brolly: None` — a dead/respawned
    // kid regrows a full canopy, matching JS `reset` + `kitOf`).
    pub(crate) brolly: Option<Box<BrollyKit>>,
    /// A canopy launched this frame (JS `addLaunched` → the world `LAUNCHED`
    /// list, brolly.js L265/L285): the match takes it into its registry.
    pub(crate) brolly_launch: Option<LaunchedCanopy>,
}

impl Default for WeaponRunner {
    fn default() -> Self {
        Self {
            cooldown: 0.0,
            empty_cd: 0.0,
            firing_t: 0.0,
            bloom: 0.0,
            spread: 0.0,
            charging: false,
            charge: 0.0,
            rolling: false,
            roll_t: 0.0,
            flick: -1.0,
            flick_recover: 0.0,
            burst_t: 0.0,
            turret: false,
            lock_t: 0.0,
            dodge_t: 0.0,
            roll_lock: 0.0,
            since_roll: 9.0,
            rolls_used: 0,
            roll_queued: false,
            twin_side: 1,
            since_hand: [99.0, 99.0],
            dodge_dir: Vec3::ZERO,
            streaming: false,
            burst_frac: 0.0,
            brolly: None,
            brolly_launch: None,
        }
    }
}

impl WeaponRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// JS `reset()` (L37-61).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// JS `busy()` (L62): any state that blocks the actor. The retired-kind
    /// terms (`slosh` / `streaming` / `dodge`) and the kit-weapon hook
    /// (`MAIN_KITS[kind].busy`) are absent until their tasks land.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.charging
            || self.flick >= 0.0
            || self.burst_t > 0.0
            || self.dodge_t > 0.0
            || self.lock_t > 0.0
    }

    /// Whether the muzzle-flash window is open (JS `wr.firingT > 0`, the
    /// `F.firing` net flag; netmatch.js L737). Read-only accessor for the
    /// `net` module's actor-tick view (Task 15).
    #[must_use]
    pub fn firing(&self) -> bool {
        self.firing_t > 0.0
    }

    /// JS `moveSpeed()` (weapons.js L63-92) — the full per-kind table.
    /// The four JS `switch` cases (L67-78: brush/bucket/spinner/twins) and
    /// the general tail (L79-91) are ported 1:1 for the wired + scaffolded
    /// kinds; the retired branches (`lockT` dualies / `streaming` splatling
    /// / splatling `charging` / `slosh`) are unreachable for the 12 roster
    /// kinds. The JS kit kinds (brolly/bow/blade/mitts, L64
    /// `MAIN_KITS[kind].moveSpeed`) fall back to `runSpeed` until
    /// Tasks 30/32/33 wire their kit branches.
    #[must_use]
    pub fn move_speed(&self, kind: WeaponKind, t: &PlayerTuning, w: &WeaponSet) -> f32 {
        let run = t.run_speed;
        match kind {
            // JS L67-69: brushing is faster than running; swipes barely slow you.
            WeaponKind::Brush => {
                if self.rolling {
                    lerp(
                        w.brush.move_speed_firing,
                        w.brush.brush_speed,
                        smoothstep(0.0, 0.35, self.roll_t),
                    )
                } else if self.firing_t > 0.0 {
                    w.brush.move_speed_firing
                } else {
                    run
                }
            }
            // JS L70-72.
            WeaponKind::Bucket => {
                if self.flick >= 0.0 {
                    w.bucket.move_speed_firing * 0.7
                } else if self.firing_t > 0.0 {
                    w.bucket.move_speed_firing
                } else {
                    run
                }
            }
            // JS L73-76.
            WeaponKind::Spinner => {
                if self.burst_t > 0.0 {
                    w.spinner.move_speed_firing
                } else if self.charging {
                    lerp(
                        run * 0.75,
                        w.spinner.move_speed_charging,
                        (self.charge * 2.0).min(1.0),
                    )
                } else {
                    run
                }
            }
            // JS L77-78.
            WeaponKind::Twins => {
                if self.firing_t > 0.0 {
                    w.twins.move_speed_firing
                } else {
                    run
                }
            }
            // JS L79-91 general tail. `rolling` / `flick` / `flickRecover`
            // are only ever set by the roller branch (brush/bucket return
            // above), so the roller fields read there.
            WeaponKind::Roller => {
                if self.rolling {
                    lerp(
                        w.roller.roll_speed * 0.5,
                        w.roller.roll_speed,
                        smoothstep(0.0, 0.45, self.roll_t),
                    )
                } else if self.flick >= 0.0 {
                    lerp(
                        w.roller.move_speed_firing,
                        w.roller.move_speed_firing * 0.45,
                        clamp_f(self.flick / w.roller.flick_windup, 0.0, 1.0),
                    )
                } else if self.flick_recover > 0.0 {
                    lerp(
                        run,
                        w.roller.move_speed_firing * 0.6,
                        self.flick_recover / 0.18,
                    )
                } else if self.charging {
                    lerp(
                        run * 0.7,
                        w.roller.move_speed_firing,
                        (self.charge * 3.0).min(1.0),
                    )
                } else if self.firing_t > 0.0 {
                    w.roller.move_speed_firing
                } else {
                    run
                }
            }
            WeaponKind::Charger => {
                if self.charging {
                    lerp(
                        run * 0.7,
                        w.charger.move_speed_firing,
                        (self.charge * 3.0).min(1.0),
                    )
                } else if self.firing_t > 0.0 {
                    w.charger.move_speed_firing
                } else {
                    run
                }
            }
            WeaponKind::Shooter => {
                if self.firing_t > 0.0 {
                    w.shooter.move_speed_firing
                } else {
                    run
                }
            }
            WeaponKind::Blaster => {
                if self.firing_t > 0.0 {
                    w.blaster.move_speed_firing
                } else {
                    run
                }
            }
            // JS L64 `MAIN_KITS.brolly.moveSpeed` (brolly.js L664):
            // `open > 0.2 ? moveSpeedShield : 0` — the falsy `0` falls
            // through to the general tail (L89-90: `firingT > 0 ?
            // moveSpeedFiring : runSpeed`).
            WeaponKind::Brolly => {
                if self.brolly.as_ref().is_some_and(|k| k.open > 0.2) {
                    w.brolly.move_speed_shield
                } else if self.firing_t > 0.0 {
                    w.brolly.move_speed_firing
                } else {
                    run
                }
            }
            // JS L64: the remaining kit weapons own their move-speed curve
            // (kits/*.js). Without a kit `moveSpeed` JS falls through to the
            // tail chain (L89-90: `firingT > 0 ? moveSpeedFiring : runSpeed`);
            // the kits are unwired until Tasks 32/33, so the sim returns
            // `runSpeed`.
            WeaponKind::Bow | WeaponKind::Blade | WeaponKind::Mitts => run,
        }
    }

    /// JS `WeaponRunner._spreadDeg` (weapons.js L94-107) for the roster
    /// kinds. Shooter-family bloom cone (L97-100), blaster ground/air
    /// (L102 — the def sets neither `spread` nor `spreadAir`, config.js
    /// L126-135, so the JS `?? 1.2` / `?? 4` defaults hold), twins
    /// turret/bloom (L103), spinner ground/air (L104). Everything else
    /// returns `0` (JS L105).
    #[must_use]
    pub fn spread_deg(&self, grounded: bool, kind: WeaponKind, w: &WeaponSet) -> f32 {
        match kind {
            WeaponKind::Shooter => {
                let base = if grounded {
                    w.shooter.spread_ground
                } else {
                    w.shooter.spread_air
                };
                base * lerp(SPREAD_FIRST, 1.0, self.bloom)
            }
            WeaponKind::Blaster => {
                if grounded {
                    1.2
                } else {
                    4.0
                }
            }
            WeaponKind::Twins => {
                if self.turret {
                    w.twins.turret_spread
                } else {
                    let base = if grounded {
                        w.twins.spread_ground
                    } else {
                        w.twins.spread_air
                    };
                    base * lerp(0.5, 1.0, self.bloom)
                }
            }
            WeaponKind::Spinner => {
                if grounded {
                    w.spinner.spread_ground
                } else {
                    w.spinner.spread_air
                }
            }
            // JS L95 `MAIN_KITS.brolly.spreadDeg` (brolly.js L665):
            // `grounded ? spreadDeg : spreadAir` (no bloom).
            WeaponKind::Brolly => {
                if grounded {
                    w.brolly.spread_deg
                } else {
                    w.brolly.spread_air
                }
            }
            _ => 0.0,
        }
    }

    /// JS `WeaponRunner.update(dt, inp)` (L108-156): the common timer decay
    /// (L110-116) + the kind dispatch (L117-129). JS's `inp` is the gated
    /// `winp` (actor.js L370-373), which [`Actor::fire_gate`] already
    /// resolves (emerge delay + fire buffer) — hence the `gate` parameter
    /// alone; `move_dir` is the raw intent's move vector (JS reads
    /// `a.intent.move` in `_twins`, L493/L499/L534 — *not* the gated one).
    /// The sub-weapon block (JS L130-155) is Task 34.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        gate: &FireGate,
        actor: &mut Actor,
        kind: WeaponKind,
        w: &WeaponSet,
        move_dir: Vec3,
        world: &CollisionWorld,
        projectiles: &mut ProjectileSim,
    ) {
        // ---- JS L110-116 common decay. JS `inp` here is the *gated* `winp`
        // (actor.js L370-373: `fire = intent.fire || buffered`, suppressed
        // during emerge), so the bloom decay follows `gate.fire`, not the
        // raw intent.
        self.cooldown -= dt;
        self.empty_cd -= dt;
        self.firing_t = (self.firing_t - dt).max(0.0);
        self.flick_recover = (self.flick_recover - dt).max(0.0);
        if !gate.fire {
            self.bloom = (self.bloom - dt / BLOOM_RECOVER).max(0.0);
        }
        self.spread = self.spread_deg(actor.grounded, kind, w);
        // JS L116: per-hand shot clocks (dualies / twins HUD).
        self.since_hand[0] += dt;
        self.since_hand[1] += dt;

        // ---- JS L117-129 kind dispatch. Task 30 wires `shooter`/`blaster`
        // (`_auto`), `twins` (`_twins`) and `brolly` (`MAIN_KITS.brolly.
        // update`); the remaining arms land in Tasks 31-33:
        //   L119-124 `charger`/`roller`/`bucket`/`spinner`/`brush`
        //     → Tasks 31/32/31/31/32.
        //   L128 `default` → `MAIN_KITS` (bow/blade/mitts, Tasks 33/32/33).
        match kind {
            WeaponKind::Shooter | WeaponKind::Blaster => {
                self.auto_fire(gate, actor, kind, w, projectiles)
            }
            WeaponKind::Twins => self.twins(gate, actor, w, dt, move_dir, world, projectiles),
            WeaponKind::Brolly => brolly::update(self, dt, gate, actor, w, world, projectiles),
            WeaponKind::Charger
            | WeaponKind::Roller
            | WeaponKind::Bucket
            | WeaponKind::Spinner
            | WeaponKind::Brush
            | WeaponKind::Bow
            | WeaponKind::Blade
            | WeaponKind::Mitts => {}
        }
    }

    /// JS `_auto` (weapons.js L165-182) — the shared auto-fire path every
    /// stream kind runs through: trigger gate, `firingT` pose window, the
    /// up-to-3-shots-per-frame catch-up loop, ink gating with the
    /// empty-click lockout (`_empty`, L158-163), the per-shot spread
    /// recompute + bloom step, and the cooldown advance. `spawn_shot` is the
    /// kind's muzzle call (JS L176-177).
    fn auto_fire(
        &mut self,
        gate: &FireGate,
        actor: &mut Actor,
        kind: WeaponKind,
        w: &WeaponSet,
        projectiles: &mut ProjectileSim,
    ) {
        let (fire_interval, ink_per_shot) = match kind {
            WeaponKind::Shooter => (w.shooter.fire_interval, w.shooter.ink_per_shot),
            // JS L118: blaster shares the `_auto` ink path with its own def
            // fields (config.js L126-135).
            WeaponKind::Blaster => (w.blaster.fire_interval, w.blaster.ink_per_shot),
            _ => return,
        };
        if !gate.fire {
            if self.cooldown < 0.0 {
                self.cooldown = 0.0;
            }
            return;
        }
        self.firing_t = FIRING_T;
        // JS also sets a.fireFacing = 0.5; the facing assist is Task 8 wiring.
        let mut guard = 0usize;
        while self.cooldown <= 0.0 && guard < 3 {
            guard += 1;
            if actor.ink < ink_per_shot {
                // JS `_empty()` + `cooldown += fireInterval; break`.
                if self.empty_cd <= 0.0 {
                    self.empty_cd = EMPTY_CD;
                    projectiles.push_event(SimEvent::LowInk {
                        owner_slot: actor.slot,
                    });
                }
                self.cooldown += fire_interval;
                break;
            }
            // JS order: ink -= cost; lastFire = 0; spread = _spreadDeg; fire.
            actor.ink -= ink_per_shot;
            actor.note_fired();
            self.spread = self.spread_deg(actor.grounded, kind, w);
            self.fire_kind_round(actor, kind, w, projectiles);
            self.bloom = (self.bloom + BLOOM_PER_SHOT).min(1.0);
            self.cooldown += fire_interval;
        }
    }

    /// JS `_auto` L176-177: the per-kind muzzle call.
    fn fire_kind_round(
        &self,
        actor: &Actor,
        kind: WeaponKind,
        w: &WeaponSet,
        projectiles: &mut ProjectileSim,
    ) {
        match kind {
            WeaponKind::Shooter => projectiles.fire_shooter(actor, &w.shooter, self.spread, None),
            WeaponKind::Blaster => projectiles.fire_blaster(actor, &w.blaster, self.spread),
            _ => {}
        }
    }

    /// JS `_twins` (weapons.js L489-517): rapid alternating fire; after a
    /// dodge roll, holding fire plants you with both guns together (faster,
    /// tighter `turret*` group). `move_dir` is the raw intent move (JS L493
    /// reads `a.intent.move`).
    #[allow(clippy::too_many_arguments)]
    fn twins(
        &mut self,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        dt: f32,
        move_dir: Vec3,
        world: &CollisionWorld,
        ps: &mut ProjectileSim,
    ) {
        let tw = &w.twins;
        // JS L491: lockout decay + roll-chain idle clock.
        self.roll_lock = (self.roll_lock - dt).max(0.0);
        self.since_roll += dt;
        // JS L492: a quiet spell refills the chain.
        if self.roll_lock <= 0.0 && self.since_roll > tw.roll_reset && self.dodge_t <= 0.0 {
            self.rolls_used = 0;
        }
        // JS L493: walking (or releasing the trigger) un-plants the turret.
        let mh = move_dir.x.hypot(move_dir.z);
        if self.turret && (!gate.fire || mh > 0.3) {
            self.turret = false;
        }
        if self.dodge_t > 0.0 {
            self.dodge_t -= dt;
            if self.dodge_t <= 0.0 {
                self.dodge_t = 0.0;
                // JS L497: the roll ends planted if the trigger is still held.
                self.turret = gate.fire;
                // JS L499: a roll tapped during this one chains straight on.
                if self.roll_queued {
                    self.roll_queued = false;
                    if self.twin_dodge(a, tw, move_dir, gate.fire) {
                        return;
                    }
                }
            }
            return;
        }
        if !gate.fire {
            if self.cooldown < 0.0 {
                self.cooldown = 0.0;
            }
            return;
        }
        self.firing_t = FIRING_T;
        let mut guard = 0usize;
        while self.cooldown <= 0.0 && guard < 3 {
            guard += 1;
            if a.ink < tw.ink_per_shot {
                // JS `_empty()` + `cooldown += fireInterval; break`.
                if self.empty_cd <= 0.0 {
                    self.empty_cd = EMPTY_CD;
                    ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
                }
                self.cooldown += tw.fire_interval;
                break;
            }
            a.ink -= tw.ink_per_shot;
            a.note_fired();
            // JS L509-510: alternate hands; the planted turret fires both
            // guns together (`side = 0`).
            self.twin_side = -self.twin_side;
            let side = if self.turret { 0 } else { self.twin_side };
            let sd = self.spread_deg(a.grounded, WeaponKind::Twins, w);
            ps.fire_twin(a, tw, sd, side, world);
            // Deviation D-4: JS L511 hardcodes `bloom + 0.25` (no
            // `bloomPerShot` in the twins def) — kept verbatim.
            self.bloom = (self.bloom + 0.25).min(1.0);
            // JS L513-514: side +1 = the left gun, -1 = the right.
            let hand = if self.twin_side > 0 { 1 } else { 0 };
            self.since_hand[hand] = 0.0;
            self.cooldown += if self.turret {
                tw.turret_interval
            } else {
                tw.fire_interval
            };
        }
    }

    /// JS `_twinDodge` (weapons.js L529-548): jump tapped while firing and
    /// moving → a dodge roll instead. Returns true when a roll started (or
    /// was queued), which the caller uses to swallow the jump press.
    fn twin_dodge(&mut self, a: &mut Actor, tw: &Twins, move_dir: Vec3, fire: bool) -> bool {
        // JS L532: a tap mid-roll with a roll left in the chain queues it.
        if self.dodge_t > 0.0
            && fire
            && self.roll_lock <= 0.0
            && self.rolls_used > 0
            && self.rolls_used < tw.roll_charges
        {
            self.roll_queued = true;
            return true;
        }
        // JS L533: needs fire held, no roll running / locked, grounded, kid.
        if !fire || self.dodge_t > 0.0 || self.roll_lock > 0.0 || !a.grounded || a.form != Form::Kid
        {
            return false;
        }
        let mh = move_dir.x.hypot(move_dir.z);
        if mh < 0.3 || a.ink < tw.roll_ink {
            return false;
        }
        a.ink -= tw.roll_ink;
        self.dodge_dir = vec3(move_dir.x / mh, 0.0, move_dir.z / mh);
        self.dodge_t = tw.roll_time;
        self.turret = false;
        self.since_roll = 0.0;
        // JS L540: two rolls in a row, then a lockout before the next.
        self.rolls_used += 1;
        if self.rolls_used >= tw.roll_charges {
            self.roll_lock = tw.roll_lockout;
            self.rolls_used = 0;
        }
        true
    }

    /// JS `tryDodge` (weapons.js L522-527): the jump-press dispatcher. The
    /// retired `dualies` branch is absent (config.js L266-299).
    pub fn try_dodge(
        &mut self,
        kind: WeaponKind,
        a: &mut Actor,
        w: &WeaponSet,
        move_dir: Vec3,
        fire: bool,
    ) -> bool {
        match kind {
            WeaponKind::Twins => self.twin_dodge(a, &w.twins, move_dir, fire),
            _ => false,
        }
    }

    /// JS `dodgeVelocity` (weapons.js L550-555): the twins roll's horizontal
    /// velocity — fast start easing out — sampled at the *start* of the
    /// frame and written to [`crate::actor::Actor::dodge_vel`] by the match
    /// layer (actor.js L305-306). `None` when not rolling.
    #[must_use]
    pub fn dodge_velocity(&self, kind: WeaponKind, w: &WeaponSet) -> Option<Vec3> {
        if kind != WeaponKind::Twins || self.dodge_t <= 0.0 {
            return None;
        }
        let tw = &w.twins;
        // JS L552-554: `u = 1 - dodgeT/rollTime`, `v0 = rollDist/(0.7·
        // rollTime)`, `v = dodgeDir · v0(1 − 0.6u)`.
        let u = 1.0 - self.dodge_t / tw.roll_time;
        let v0 = tw.roll_dist / (0.7 * tw.roll_time);
        Some(self.dodge_dir * (v0 * (1.0 - 0.6 * u)))
    }

    /// JS `this.charge` (the `ch` tick slot is `streaming ? burstFrac :
    /// charge`, netmatch.js L744): read-only accessors for the net layer.
    #[must_use]
    pub fn charge(&self) -> f32 {
        self.charge
    }

    /// JS `this.lockT` (the `lock` tick slot, netmatch.js L751).
    #[must_use]
    pub fn lock_t(&self) -> f32 {
        self.lock_t
    }

    /// JS `this.streaming` (the `F.streaming` flag, netmatch.js L737).
    #[must_use]
    pub fn streaming(&self) -> bool {
        self.streaming
    }

    /// JS `this.burstFrac` (the `ch` slot while streaming, netmatch.js L744).
    #[must_use]
    pub fn burst_frac(&self) -> f32 {
        self.burst_frac
    }

    /// The brolly kit, created lazily like JS `kitOf` (brolly.js L48-61).
    pub(crate) fn ensure_brolly(&mut self, w: &WeaponSet) -> &mut BrollyKit {
        if self.brolly.is_none() {
            self.brolly = Some(Box::new(BrollyKit::new(w.brolly.canopy_hp as f32)));
        }
        self.brolly.as_mut().unwrap()
    }

    /// JS `MAIN_KITS.brolly.netState` (brolly.js L636-641): the canopy pose
    /// packed into one word — `state` bits 0-1, `open` 0-15 bits 2-5,
    /// `grow` 0-7 bits 6-8, `hp/hpMax` 0-31 bits 9-13, `regrowK` 0-7 bits
    /// 14-16. 0 when the runner carries no kit.
    #[must_use]
    pub fn brolly_net_state(&self) -> u32 {
        let Some(k) = self.brolly.as_deref() else {
            return 0;
        };
        // JS `q(v, n) = max(0, min(n, round(clamp(v, 0, 1) · n)))`.
        let q = |v: f32, n: u32| ((clamp_f(v, 0.0, 1.0) * n as f32 + 0.5) as u32).min(n);
        let st = match k.state {
            CanopyState::Ready => 0,
            CanopyState::Broken => 1,
            CanopyState::Launched => 2,
        };
        st | (q(k.open, 15) << 2)
            | (q(k.grow, 7) << 6)
            | (q(k.hp / k.hp_max, 31) << 9)
            | (q(k.regrow_k, 7) << 14)
    }

    /// JS `MAIN_KITS.brolly.netApply` (brolly.js L642-649): rebuild the kit
    /// pose from a [`Self::brolly_net_state`] word. The hit-flash side
    /// effect (L646) is presentation-only and dropped (deviation D-7).
    pub fn net_apply_brolly(&mut self, bits: u32, w: &WeaponSet) {
        let k = self.ensure_brolly(w);
        k.state = match bits & 3 {
            1 => CanopyState::Broken,
            2 => CanopyState::Launched,
            _ => CanopyState::Ready,
        };
        k.hp = ((bits >> 9) & 31) as f32 / 31.0 * k.hp_max;
        k.open = ((bits >> 2) & 15) as f32 / 15.0;
        k.grow = ((bits >> 6) & 7) as f32 / 7.0;
        k.regrow_k = ((bits >> 14) & 7) as f32 / 7.0;
    }
}

/// JS `clamp` (ctx.js).
fn clamp_f(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

/// Pellet-merge window (JS `MERGE = 1/30`, brolly.js L389): pellets of one
/// blast reaching the same victim within this many seconds sum into one hit.
pub const MERGE: f32 = 1.0 / 30.0;

/// A swept canopy plane a round may be blocked by this frame (JS shield `s`
/// in `segShield`, brolly.js L195-200). `cp`/`np` are the *previous* frame's
/// pose (the swept test's start), `c`/`n` the current one.
#[derive(Debug, Clone, Copy)]
pub struct Shield {
    pub team: usize,
    pub c: Vec3,
    pub n: Vec3,
    pub r: f32,
    pub cp: Vec3,
    pub np: Vec3,
    /// `WEAPONS.brolly.regrowTime` — the held kit needs it when the canopy
    /// breaks (brolly.js L235).
    pub regrow_time: f32,
    pub target: ShieldTarget,
}

/// Which registry a shield's hp lives in (JS `hpOf(s)`, brolly.js L202:
/// `s.held ? s.kit : s.ref`).
#[derive(Debug, Clone, Copy)]
pub enum ShieldTarget {
    /// A held canopy: [`WeaponRunner::brolly`] of the runner at this actor
    /// index.
    Held(usize),
    /// A launched canopy: [`crate::match_::Match::launched`] at this index.
    Launched(usize),
}

/// The hp owners [`ProjectileSim::step_masked` may deduct canopy damage
/// from (JS `hitShield`/`breakShield`, brolly.js L203-239).
pub struct ShieldOwner<'a> {
    pub runners: &'a mut [WeaponRunner],
    pub launched: &'a mut [LaunchedCanopy],
}

/// JS `segShield(prev, pos, s, out)` (brolly.js L195-200): signed-distance
/// crossing of the swept disc, either direction. Some = the crossing point.
#[must_use]
fn seg_shield(prev: Vec3, pos: Vec3, s: &Shield) -> Option<Vec3> {
    let a = (prev - s.cp).dot(s.np);
    let b = (pos - s.c).dot(s.n);
    if (a > 0.0) == (b > 0.0) || a == b || a == 0.0 {
        return None;
    }
    let at = prev.lerp(pos, a / (a - b));
    if at.distance_squared(s.c) < s.r * s.r {
        Some(at)
    } else {
        None
    }
}

/// JS `PEND` record (brolly.js L468-471): the merged pellet hit waiting to
/// land. Private to the projectile sim.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct PendingHit {
    owner_slot: usize,
    team: usize,
    victim: usize,
    dmg: f32,
    at: f32,
}

/// Global projectile system (JS `Projectiles`): owns the live rounds, the
/// event queue and the RNG stream.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectileSim {
    pub list: Vec<Projectile>,
    events: Vec<SimEvent>,
    pub rng: Rng,
    /// Accumulated stepped time — the kit's clock (JS `G.time`, used by the
    /// pellet-merge flush, brolly.js L477-481, and the canopy idle check).
    #[serde(default)]
    pub time: f32,
    /// Blast group counter (JS module-global `BLAST`, brolly.js L72/L91):
    /// ids start at 1. Deviation: JS keeps it module-level; the sim hangs it
    /// on the projectile stream so snapshots restore it.
    #[serde(default)]
    pub blast_seq: u32,
    /// Pending merged pellet hits, keyed `blast * 64 + victim_idx` (JS L468;
    /// `BTreeMap` for deterministic snapshot serialisation).
    #[serde(default)]
    pending: BTreeMap<u64, PendingHit>,
}

impl ProjectileSim {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            list: Vec::new(),
            events: Vec::new(),
            rng: Rng::new(seed),
            time: 0.0,
            blast_seq: 0,
            pending: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn push_event(&mut self, e: SimEvent) {
        self.events.push(e);
    }

    /// JS `Projectiles._muzzle` degenerate branch (no character rig in the
    /// sim): eye position + a short push along the aim direction, so a round
    /// never spawns inside its owner's capsule. The eye height follows the
    /// form (JS L936: `squid ? 0.4 : 1.05`).
    #[must_use]
    pub fn muzzle(a: &Actor) -> Vec3 {
        let eye_y = if a.form == Form::Squid {
            MUZZLE_EYE_Y_SQUID
        } else {
            MUZZLE_EYE_Y
        };
        let eye = vec3(a.pos.x, a.pos.y + eye_y, a.pos.z);
        eye + a.aim_dir() * MUZZLE_FALLBACK_DIST
    }

    /// JS `Projectiles._aimFrom`: direction from muzzle toward the aim point,
    /// falling back to `aimDir` when the point is closer than 2 m or behind
    /// the muzzle. `aim_point` is the crosshair's world position (player
    /// camera raycast / bot solver; the actor stores only yaw/pitch).
    #[must_use]
    pub fn aim_from(a: &Actor, from: Vec3, aim_point: Option<Vec3>) -> Vec3 {
        if let Some(t) = aim_point {
            let mut out = t - from;
            let d = out.length();
            if d >= 2.0 && out.dot(a.aim_dir()) >= 0.0 {
                out *= 1.0 / d;
                return out;
            }
        }
        a.aim_dir()
    }

    /// JS `Projectiles.fireShooter` (minus audio/FX/rumble/netcode) — the
    /// Spritzer convenience over [`Self::fire_round`].
    pub fn fire_shooter(
        &mut self,
        a: &Actor,
        w: &Spritzer,
        spread_deg: f32,
        aim_point: Option<Vec3>,
    ) {
        self.fire_round(a, &RoundSpec::shooter(w), spread_deg, aim_point);
    }

    /// JS `Projectiles.fireShooter` / `_fireRound` (weapons.js L1027-1046 /
    /// L1068-1082): the generalized shooter-family spawn — muzzle, aim,
    /// ballistic correction with the spec's `projSpeed` / `straightTime` /
    /// `grav` / `drag` / `range`, spread cone, then the round with its own
    /// `straight` / `radius` / `damage` / `size` / `trail*` / `life`. The ink
    /// cost and `lastFire` reset happen in the caller (`_auto`), keeping the
    /// fire loop and the shot emission atomic.
    pub fn fire_round(
        &mut self,
        a: &Actor,
        spec: &RoundSpec,
        spread_deg: f32,
        aim_point: Option<Vec3>,
    ) {
        let m = Self::muzzle(a);
        let dir0 = Self::aim_from(a, m, aim_point);
        // JS `_ballistic(m, dir, a.aimPoint, ...)`: the launch-pitch
        // correction only makes sense with a crosshair world point. Without
        // one (sim/bot aim = yaw/pitch only) the round flies the aim line as
        //-is — the same result JS produces when the correction can't resolve.
        let dir = match aim_point {
            Some(aim) => ballistic(
                m,
                dir0,
                aim,
                spec.proj_speed,
                spec.straight_time,
                spec.grav,
                spec.drag,
                spec.range,
            ),
            None => dir0,
        };
        let dir = self.rng.spread(dir, spread_deg);
        // Deviation: range-enforced life (see module docs). JS: 1.2 s.
        let life = (spec.range / spec.proj_speed).min(spec.life);
        self.push_round(a, spec, m, dir, life, false, None, true);
    }

    /// JS `Projectiles.fireBlaster` (weapons.js L1173-1192): the ammo-segment
    /// blast — a straight, gravity-free `type: 'blast'` round that detonates
    /// [`Self::blast_burst`] on any death. `life` is the range cap
    /// (`range / projSpeed`, JS L1178), `straight: 99` never expires.
    pub fn fire_blaster(&mut self, a: &Actor, w: &Blaster, spread_deg: f32) {
        let m = Self::muzzle(a);
        let dir0 = Self::aim_from(a, m, None);
        let dir = self.rng.spread(dir0, spread_deg);
        let spec = RoundSpec {
            proj_speed: w.proj_speed,
            straight_time: 99.0, // JS L1178 `straight: 99`
            range: w.range,
            grav: 0.0,
            drag: 0.0,
            size: 0.26, // JS L1178 `size: 0.26`
            life: f32::INFINITY,
            impact_radius: w.impact_radius,
            damage: w.direct_damage,
            trail_every: 2.2, // JS L1178 `trailEvery: 2.2`
            trail_radius: 0.45,
            trail_start: -1.5, // JS L1178 `trail: -1.5`
        };
        let life = (spec.range / spec.proj_speed).min(spec.life);
        self.push_round(a, &spec, m, dir, life, true, None, true);
    }

    /// JS `Projectiles.fireTwin` (weapons.js L1356-1392), the no-dual-rig
    /// fallback branch (L1360-1373): the sim has no character rig, so every
    /// twin shot takes the offset-muzzle path — the firing side's muzzle is
    /// the base muzzle nudged along the aim-perpendicular, `side > 0`
    /// averaging in the midline-mirrored position, `side < 0` pushing back
    /// half as far; a muzzle that fails the eye→muzzle LOS check falls back
    /// to the generic eye + aim push. `side = 0` (planted turret) fires from
    /// the plain muzzle with the faster, straighter, longer-reaching
    /// `turret*` shot group.
    pub fn fire_twin(
        &mut self,
        a: &Actor,
        w: &Twins,
        spread_deg: f32,
        side: i32,
        world: &CollisionWorld,
    ) {
        let m0 = Self::muzzle(a);
        let m = if side == 0 {
            m0
        } else {
            let aim = a.aim_dir();
            // JS L1365: aim-perpendicular in the horizontal plane.
            let mut perp = vec3(aim.z, 0.0, -aim.x);
            if perp.length_squared() > 1e-12 {
                perp = perp.normalize();
            }
            // JS L1366-1368: the mirrored (left-pistol) muzzle across the
            // kid's midline, in the body-local frame.
            let (cy, sy) = (a.yaw.cos(), a.yaw.sin());
            let (dx, dz) = (m0.x - a.pos.x, m0.z - a.pos.z);
            let lx = dx * cy - dz * sy;
            let lz = dx * sy + dz * cy;
            let mirror = vec3(
                a.pos.x - lx * cy + lz * sy,
                m0.y,
                a.pos.z + lx * sy + lz * cy,
            );
            let mut m = if side > 0 {
                // JS L1369: half-way between the +17 cm offset and the mirror.
                (m0 + perp * 0.17 + mirror) * 0.5
            } else {
                // JS L1370.
                m0 + perp * (-0.085)
            };
            // JS L1371-1372: a muzzle inside geometry falls back to the eye.
            let eye = vec3(a.pos.x, a.pos.y + MUZZLE_EYE_Y, a.pos.z);
            if !world.los(eye, m) {
                m = eye + aim * MUZZLE_FALLBACK_DIST;
            }
            m
        };
        let dir0 = Self::aim_from(a, m, None);
        // JS L1377: planted mode fires the tighter `turret*` group.
        let (speed, straight, range) = if side != 0 {
            (w.proj_speed, w.straight_time, w.range)
        } else {
            (w.turret_proj_speed, w.turret_straight, w.turret_range)
        };
        let dir = self.rng.spread(dir0, spread_deg);
        let spec = RoundSpec {
            proj_speed: speed,
            straight_time: straight,
            range,
            grav: PROJ_GRAV,
            drag: PROJ_DRAG,
            size: 0.13, // JS L1381 `size: 0.13`
            life: SHOT_LIFE,
            impact_radius: w.impact_radius,
            damage: w.damage,
            trail_every: w.trail_every,
            trail_radius: w.trail_radius,
            trail_start: -(2.4 - w.trail_every), // JS L1381
        };
        // JS L1381 lives 1.2 s; the sim keeps its range-cap deviation.
        let life = (range / speed).min(SHOT_LIFE);
        self.push_round(a, &spec, m, dir, life, false, None, true);
    }

    /// Shared spawn tail of [`Self::fire_round`] / [`Self::fire_blaster`] /
    /// [`Self::fire_twin`] / the brolly pellets (JS `_new`, `Object.assign`,
    /// `_push` and the `weapon:fire` emit): draw the blob seed, push the
    /// round, optionally emit the fire event.
    #[allow(clippy::too_many_arguments)]
    fn push_round(
        &mut self,
        a: &Actor,
        spec: &RoundSpec,
        m: Vec3,
        dir: Vec3,
        life: f32,
        blast: bool,
        pellet: Option<PelletMark>,
        emit_fire: bool,
    ) {
        let seed = self.rng.next_f32();
        self.list.push(Projectile {
            pos: m,
            prev: m,
            start: m,
            vel: dir * spec.proj_speed,
            team: a.team,
            owner_slot: a.slot,
            age: 0.0,
            life,
            straight: spec.straight_time,
            radius: spec.impact_radius,
            size: spec.size,
            damage: spec.damage,
            grav: spec.grav,
            drag: spec.drag,
            trail: spec.trail_start,
            trail_every: spec.trail_every,
            trail_radius: spec.trail_radius,
            seed,
            blast,
            pellet,
            ghost: false,
        });
        if emit_fire {
            self.push_event(SimEvent::Fire {
                owner_slot: a.slot,
                team: a.team,
                muzzle: m,
                dir,
            });
        }
    }

    /// Live round count against the JS `MAX_BLOBS` presentation cap
    /// (weapons.js L630). The sim never culls (JS `_draw` L1880 is the only
    /// place the cap bites); this is the guard the tests / presentation use.
    #[must_use]
    pub fn round_count(&self) -> usize {
        self.list.len()
    }

    /// JS `Projectiles.update(dt)` loop over `this.list` (shots only).
    /// Reverse iteration like JS (L1567): a despawn swaps the tail element
    /// into the hole and the loop continues below it, so that element is not
    /// re-stepped this frame — matching JS's `list[i] = list[last]; pop()`.
    pub fn step(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        w: &WeaponSet,
    ) {
        let mut owner = ShieldOwner {
            runners: &mut [],
            launched: &mut [],
        };
        self.step_masked(dt, world, actors, paint, t, w, None, &[], &mut owner);
    }

    /// [`Self::step`] with the online actor mask (Task 21): `remote[i]`
    /// marks actor `i` as a proxy. A round that hits a proxy is *decided*
    /// here but not applied — JS `applyHit` → `shouldApplyHit` → `'send'`
    /// (weapons.js L1551, netmatch.js L711-717): the shooter's client
    /// streams the damage to the victim's owner over `{k:'hit'}`, which we
    /// surface as [`SimEvent::HitSend`]. `None` = offline (every hit local).
    #[allow(clippy::too_many_arguments)]
    pub fn step_masked(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        w: &WeaponSet,
        remote: Option<&[bool]>,
        shields: &[Shield],
        owner: &mut ShieldOwner,
    ) {
        // JS `tick` clock: the kit's merge window and canopy idle checks run
        // on the projectile time (brolly.js L477-481 / L397).
        self.time += dt;
        // JS `tick` step 2 tail (brolly.js L477-481): due merged pellet hits
        // land before the rounds step. (A record is created with `at = time +
        // MERGE`, so one can never come due in its own frame — flushing here
        // instead of after the step is equivalent.)
        self.flush_pending(actors, t, remote);
        let mut i = self.list.len();
        while i > 0 {
            i -= 1;
            if self.step_one(dt, world, actors, paint, t, w, i, remote, shields, owner) {
                self.list.swap_remove(i);
            }
        }
    }

    /// JS `tick` PEND flush (brolly.js L477-481): a merged pellet hit whose
    /// window elapsed lands as one `applyHit` on its victim. A victim that
    /// died inside the window is skipped (JS `applyHit` L1546 early return).
    fn flush_pending(&mut self, actors: &mut [Actor], t: &PlayerTuning, remote: Option<&[bool]>) {
        if self.pending.is_empty() {
            return;
        }
        let due: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, r)| self.time >= r.at - 1e-6)
            .map(|(k, _)| *k)
            .collect();
        for key in due {
            let Some(rec) = self.pending.remove(&key) else {
                continue;
            };
            let Some(v) = actors.get(rec.victim) else {
                continue;
            };
            if !v.alive || v.team == rec.team {
                continue;
            }
            let (victim_slot, victim_team) = (v.slot, v.team);
            let send = remote.is_some_and(|m| m.get(rec.victim).copied().unwrap_or(false));
            if send {
                self.push_event(SimEvent::HitSend {
                    attacker_slot: rec.owner_slot,
                    victim_slot,
                    victim_team,
                    team: rec.team,
                    damage: rec.dmg,
                });
            } else {
                let outcome = apply_hit(
                    actors,
                    rec.victim,
                    rec.dmg,
                    Some((rec.team, rec.owner_slot)),
                    t,
                );
                self.push_event(SimEvent::Hit {
                    attacker_slot: rec.owner_slot,
                    victim_slot,
                    team: rec.team,
                    damage: rec.dmg,
                    killed: outcome.killed,
                });
            }
        }
    }

    /// JS `Projectiles._step(p, dt)` for `type === 'shot'` (plus the brolly
    /// canopy pre-scan of `tick` step 1, brolly.js L426-438). True = despawn.
    #[allow(clippy::too_many_arguments)]
    fn step_one(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        w: &WeaponSet,
        idx: usize,
        remote: Option<&[bool]>,
        shields: &[Shield],
        owner: &mut ShieldOwner,
    ) -> bool {
        // ---- integrate (JS order: gravity/drag, then position)
        {
            let p = &mut self.list[idx];
            p.age += dt;
            p.prev = p.pos;
            if p.age > p.straight {
                p.vel.y -= p.grav * dt;
            }
            if p.drag > 0.0 && p.age > p.straight {
                let k = 1.0 - p.drag * dt;
                p.vel *= k;
            }
            p.pos += p.vel * dt;
        }
        let mut dead = false;

        // ---- enemy canopy block (JS `tick` step 1, brolly.js L426-438): the
        // swept disc is tested against the round's *actual* segment this frame
        // — `predict` (L249-255) replays the exact integrator above, so the
        // real prev→pos segment is equivalent. A blocked round dies on the
        // canopy before the body behind it.
        if !shields.is_empty() {
            let (prev, pos, team, damage, pellet, start) = {
                let p = &self.list[idx];
                (p.prev, p.pos, p.team, p.damage, p.pellet, p.start)
            };
            for s in shields {
                if s.team == team {
                    continue;
                }
                if seg_shield(prev, pos, s).is_none() {
                    continue;
                }
                // JS `projDamage(p)` (brolly.js L242-247): a brolly pellet
                // uses the falloff measured from the segment start; anything
                // else takes `damage || 10` (CANOPY_MUL only scales drop /
                // slosh types, not modelled yet). The ghost `mute` does not
                // exempt canopy damage — JS runs `hitShield` unguarded.
                let dmg = if let Some(pm) = pellet {
                    pm.pellet_damage(start.distance(prev))
                } else if damage != 0.0 {
                    damage
                } else {
                    10.0
                };
                match s.target {
                    ShieldTarget::Held(ai) => {
                        if let Some(k) = owner.runners.get_mut(ai).and_then(|r| r.brolly.as_mut()) {
                            brolly::hit_kit(k, dmg, self.time, s.regrow_time);
                        }
                    }
                    ShieldTarget::Launched(li) => {
                        if let Some(c) = owner.launched.get_mut(li) {
                            // JS `hitShield`/`breakShield` on a launched ref
                            // (brolly.js L205/L237): hp gate, then `alive = false`.
                            if c.hp > 0.0 && dmg > 0.0 {
                                c.hp -= dmg;
                                c.last_hit = self.time;
                                if c.hp <= 0.0 {
                                    c.alive = false;
                                }
                            }
                        }
                    }
                }
                return true;
            }
        }
        // JS wraps a ghost round's whole `_step` in `nm.mute++/--`
        // (weapons.js L1572-1574): every `paint.splat` it would make returns
        // 0 before rasterising (paint.js L470), and `damage: 0` skips
        // `applyHit` (L1608). The round still integrates, still dies on the
        // same hit / life / water expiry, and its `weapon:impact` emits are
        // FX/audio-only with no sim consumer.
        let ghost = self.list[idx].ghost;

        // ---- actors: enemy capsules only (JS `e.team === p.team || !e.alive`
        // skip). The victim's *visual* body is the hitbox (JS `hitBase`).
        // Gather the hit first, apply damage after the scan (borrow shape).
        let mut body_hit: Option<(usize, Vec3, Vec3, f32)> = None;
        for (si, e) in actors.iter().enumerate() {
            if e.team == self.list[idx].team || !e.alive {
                continue;
            }
            let hr = t.radius;
            let h = if e.form == Form::Squid {
                t.squid_height
            } else {
                t.height
            };
            let p = &self.list[idx];
            if (e.pos.x - p.pos.x).abs() > HIT_REJECT + hr
                || (e.pos.z - p.pos.z).abs() > HIT_REJECT + hr
            {
                continue;
            }
            let base = vec3(e.pos.x, e.visual_y(), e.pos.z);
            let res = segment_capsule_dist(p.prev, p.pos, base, hr, h);
            if res.dist < hr * HITBOX_R_MUL + p.size {
                let at = p.prev.lerp(p.pos, res.t);
                let n = -p.vel.normalize_or_zero();
                body_hit = Some((si, at, n, p.radius * 0.5));
                break;
            }
        }
        if let Some((si, at, n, r)) = body_hit {
            let (owner_slot, dmg, team, blast, pellet, start, ghost) = {
                let p = &self.list[idx];
                (
                    p.owner_slot,
                    p.damage,
                    p.team,
                    p.blast,
                    p.pellet,
                    p.start,
                    p.ghost,
                )
            };
            let victim_slot = actors[si].slot;
            let victim_team = actors[si].team;
            // JS `applyHit` route (weapons.js L1549-1552): a proxy victim is
            // hit-*decided* here but the damage lands on its owner —
            // `shouldApplyHit` → 'send' (netmatch.js L715). The round still
            // dies and the impact FX still fires (JS L1610 runs after the
            // route branch). JS L1608 only routes a round that actually
            // carries damage — a ghost round (`damage: 0`, netmatch.js
            // `ghostProjectile` L871) never reaches `applyHit`, so it neither
            // damages nor decides a hit on any victim.
            let send = remote.is_some_and(|m| m.get(si).copied().unwrap_or(false));
            if let Some(pm) = pellet {
                // JS `tick` step 2 (brolly.js L460-475): a pellet scores no
                // core damage (`damage: 0`); the kit accumulates one merged
                // hit per blast per victim over the falloff distance from the
                // round's start to the hit point, landing after `MERGE` s.
                let key = (pm.blast as u64) * 64 + (si as u64 & 63);
                let rec = self.pending.entry(key).or_insert(PendingHit {
                    owner_slot,
                    team,
                    victim: si,
                    dmg: 0.0,
                    at: self.time + MERGE,
                });
                rec.dmg += pm.pellet_damage(start.distance(at));
            } else if dmg > 0.0 {
                if send {
                    self.push_event(SimEvent::HitSend {
                        attacker_slot: owner_slot,
                        victim_slot,
                        victim_team,
                        team,
                        damage: dmg,
                    });
                } else {
                    // JS body hit: damage + FX only — no paint splat (`_credit` is
                    // never called on this branch), so `area` is 0. The attacker
                    // identity `(team, owner_slot)` rides `damage` → `lastAttacker`
                    // (JS L188) so a later water splat of the victim can credit it.
                    let outcome = apply_hit(actors, si, dmg, Some((team, owner_slot)), t);
                    self.push_event(SimEvent::Hit {
                        attacker_slot: owner_slot,
                        victim_slot,
                        team,
                        damage: dmg,
                        killed: outcome.killed,
                    });
                }
            }
            if blast {
                // JS L1610-1611: a blast suppresses the plain `weapon:impact`
                // and detonates instead (the victim of the direct hit is
                // excluded from its own splash, L1700).
                self.blast_burst(
                    at,
                    Some(si),
                    team,
                    owner_slot,
                    ghost,
                    world,
                    actors,
                    paint,
                    w,
                    t,
                    remote,
                );
            } else {
                self.push_event(SimEvent::Impact {
                    pos: at,
                    normal: n,
                    team,
                    radius: r,
                    victim_slot: Some(victim_slot),
                    owner_slot,
                    area: 0.0,
                });
            }
            dead = true;
        }

        // ---- world segment hit (JS `G.physics.segment(prev, pos, skipGrates)`).
        if !dead {
            let (prev, pos) = {
                let p = &self.list[idx];
                (p.prev, p.pos)
            };
            let hit = world.segment(prev, pos, true);
            if hit.hit {
                // JS `_impact` non-slosh branch: splat lifted 0.14 m along the
                // face normal, stretched along the travel direction
                // (`stretchAmt: 0.7`), radius jittered `0.85 + random*0.3`.
                let (rad_base, seed, team, vel, blast, owner_slot) = {
                    let p = &self.list[idx];
                    (p.radius, p.seed, p.team, p.vel, p.blast, p.owner_slot)
                };
                let at = hit.point + hit.normal * 0.14;
                let mut stretch = vel.normalize_or_zero();
                if stretch.length_squared() < 1e-8 {
                    stretch = Vec3::Z;
                }
                let rad = rad_base * (0.85 + self.rng.next_f32() * 0.3);
                let opts = SplatOpts {
                    seed,
                    stretch: Some(stretch),
                    stretch_amt: Some(0.7),
                    kind: None,
                };
                // JS `_impact` → `_credit(p, G.paint.splat(...))`: the newly
                // claimed area rides the impact event to the match layer.
                // Under the ghost `mute` the splat call itself returns 0
                // before rasterising (paint.js L470), so nothing paints and
                // `_credit` gets 0 — the jitter draw and the impact emit
                // still happen (JS L1670/L1680 run outside the mute guard).
                let area = if ghost {
                    0.0
                } else {
                    paint.splat(world, at, rad, team, &opts)
                };
                if blast {
                    // JS L1680/L1686: a blast suppresses the plain
                    // `weapon:impact`; its own impact (with the under-burst
                    // paint area) is emitted by `blast_burst`. The round's
                    // direct splat still credits turf (JS `_credit` L1679) —
                    // carried as a `Turf` event.
                    if area > 0.0 {
                        self.push_event(SimEvent::Turf {
                            owner_slot,
                            team,
                            area,
                        });
                    }
                    self.blast_burst(
                        hit.point, None, team, owner_slot, ghost, world, actors, paint, w, t,
                        remote,
                    );
                } else {
                    self.push_event(SimEvent::Impact {
                        pos: hit.point,
                        normal: hit.normal,
                        team,
                        radius: rad,
                        victim_slot: None,
                        owner_slot,
                        area,
                    });
                }
                dead = true;
            }
        }

        // ---- trail drips (JS `_step` trail branch): every `trailEvery` metres
        // of travel, raycast 4 m down and splat on the ground found.
        if !dead {
            let drip = {
                let p = &mut self.list[idx];
                if p.trail_every > 0.0 {
                    p.trail += p.vel.length() * dt;
                    if p.trail > p.trail_every {
                        p.trail = 0.0;
                        Some((p.pos, p.team, p.trail_radius))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some((from, team, trail_r)) = drip {
                let g = world.raycast(from, -Vec3::Y, 4.0, true);
                if g.hit {
                    // JS: radius jitters `0.8 + random*0.4`, seed fresh per drip.
                    let jit = 0.8 + self.rng.next_f32() * 0.4;
                    let seed = self.rng.next_f32();
                    // JS `_credit(p, splat(...))`: drip turf counts for the
                    // owner (upstream emits no `weapon:impact` here). Under
                    // the ghost `mute` the splat returns 0 (paint.js L470),
                    // so the drip paints nothing but the raycast and the
                    // fresh jitter/seed draws still happen (rng stream parity).
                    let area = if ghost {
                        0.0
                    } else {
                        paint.splat(
                            world,
                            g.point + g.normal * 0.1,
                            trail_r * jit,
                            team,
                            &SplatOpts {
                                seed,
                                ..SplatOpts::default()
                            },
                        )
                    };
                    if area > 0.0 {
                        let owner_slot = self.list[idx].owner_slot;
                        self.push_event(SimEvent::Turf {
                            owner_slot,
                            team,
                            area,
                        });
                    }
                }
            }
        }

        // ---- life / sea expiry (JS `p.age > p.life`, `pos.y < waterY - 1.8`).
        // A blast detonates on life expiry (L1641-1644); the sea only kills
        // silently (L1645).
        if !dead {
            let (age, life, y, blast, at, team, owner_slot, ghost) = {
                let p = &self.list[idx];
                (
                    p.age,
                    p.life,
                    p.pos.y,
                    p.blast,
                    p.pos,
                    p.team,
                    p.owner_slot,
                    p.ghost,
                )
            };
            if age > life {
                if blast {
                    self.blast_burst(
                        at, None, team, owner_slot, ghost, world, actors, paint, w, t, remote,
                    );
                }
                dead = true;
            } else if y < t.water_y - 1.8 {
                dead = true;
            }
        }
        dead
    }

    /// JS `Projectiles._blastBurst(p, at, direct)` (weapons.js L1689-1709).
    /// The sim's only blast weapon is the Blaster, so the `p.burst` override
    /// branch (L1691) collapses to `WEAPONS.blaster` defaults. `direct` is
    /// the actor index that took the point-blank hit (excluded from the
    /// splash, L1700); `None` for world / life-expiry detonations.
    #[allow(clippy::too_many_arguments)]
    fn blast_burst(
        &mut self,
        at: Vec3,
        direct: Option<usize>,
        team: usize,
        owner_slot: usize,
        ghost: bool,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        w: &WeaponSet,
        t: &PlayerTuning,
        remote: Option<&[bool]>,
    ) {
        let burst_r = w.blaster.burst_radius;
        let paint_r = w.blaster.impact_radius;
        let splash_r = w.blaster.splash_radius;
        let (dmax, dmin) = (w.blaster.splash_damage_max, w.blaster.splash_damage_min);
        // JS L1695: the blast's own `weapon:impact` (normal up, burst radius).
        // The under-burst paint area rides the event for the match's credit
        // (JS `_credit`, L1698 — deviating from the event-less upstream route).
        let mut area = 0.0;
        let g = world.raycast(vec3(at.x, at.y + 0.2, at.z), -Vec3::Y, 3.5, false);
        if g.hit {
            // JS L1698: the seed is drawn *only* when the raycast hits; under
            // the ghost `mute` the splat returns 0 but the draw still happens.
            let seed = self.rng.next_f32();
            if !ghost {
                area = paint.splat(
                    world,
                    g.point + g.normal * 0.1,
                    paint_r,
                    team,
                    &SplatOpts {
                        seed,
                        ..SplatOpts::default()
                    },
                );
            }
        }
        self.push_event(SimEvent::Impact {
            pos: at,
            normal: Vec3::Y,
            team,
            radius: burst_r,
            victim_slot: None,
            owner_slot,
            area,
        });
        // JS L1699-1706 splash loop: gather first (the `los` test and the
        // damage application need disjoint borrows), then route like
        // `applyHit` — proxies decide via `HitSend`, locals take damage.
        let mut splash: Vec<(usize, f32)> = Vec::new();
        for (ei, e) in actors.iter().enumerate() {
            if e.team == team || !e.alive || Some(ei) == direct {
                continue;
            }
            let c = e.pos + vec3(0.0, 0.7, 0.0);
            let d = c.distance(at);
            if d > splash_r || !world.los(at, c) {
                continue;
            }
            splash.push((ei, lerp(dmax, dmin, d / splash_r)));
        }
        for (ei, dmg) in splash {
            let (victim_slot, victim_team) = (actors[ei].slot, actors[ei].team);
            let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
            if send {
                self.push_event(SimEvent::HitSend {
                    attacker_slot: owner_slot,
                    victim_slot,
                    victim_team,
                    team,
                    damage: dmg,
                });
            } else {
                let outcome = apply_hit(actors, ei, dmg, Some((team, owner_slot)), t);
                self.push_event(SimEvent::Hit {
                    attacker_slot: owner_slot,
                    victim_slot,
                    team,
                    damage: dmg,
                    killed: outcome.killed,
                });
            }
        }
    }
}

/// JS `Projectiles.applyHit` local route: `victim.damage`. The wrong-team /
/// dead guards of JS L1546 are enforced *upstream* here — the
/// [`ProjectileSim::step_one`] scan already skips `e.team === p.team ||
/// !e.alive` (JS L1593) before a body hit is ever recorded, so this function
/// only sees live enemies. JS still emits `hit` when invulnerability blocks
/// the damage (`victim.damage()` returns false), which [`HitOutcome::blocked`]
/// records; `killed` mirrors JS (the `victim.damage()` return).
fn apply_hit(
    actors: &mut [Actor],
    victim: usize,
    dmg: f32,
    attacker: Option<(usize, usize)>,
    t: &PlayerTuning,
) -> HitOutcome {
    let v = &actors[victim];
    let blocked = !v.alive || v.invuln() > 0.0;
    if blocked {
        return HitOutcome {
            killed: false,
            blocked: true,
        };
    }
    let killed = actors[victim].damage(dmg, attacker, t);
    HitOutcome {
        killed,
        blocked: false,
    }
}

/// Result of routing a hit through [`Actor::damage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitOutcome {
    /// This hit splatted the victim (JS `victim.damage()` return).
    pub killed: bool,
    /// The hit landed on a live enemy body but dealt no damage
    /// (invulnerability). JS emits `hit` in this case (L1553 after
    /// `victim.damage()` returns false); wrong-team / dead hits never reach
    /// this point — the scan loop filters them first (JS L1593), matching
    /// JS `applyHit`'s early return at L1546.
    pub blocked: bool,
}

/// JS `Physics.pointCapsuleDist`: distance from `p` to the vertical capsule
/// with feet at `base`, `radius`, `height` (clamped axis segment).
#[must_use]
pub fn point_capsule_dist(p: Vec3, base: Vec3, radius: f32, height: f32) -> f32 {
    let lo = base.y + radius;
    let hi = base.y + radius.max(height - radius);
    let y = p.y.clamp(lo.min(hi), hi.max(lo));
    vec3(p.x - base.x, p.y - y, p.z - base.z).length()
}

/// Closest-approach result for [`segment_capsule_dist`].
#[derive(Debug, Clone, Copy)]
pub struct CapsuleHit {
    /// Parametric position along the segment [0, 1].
    pub t: f32,
    /// Distance at the closest point.
    pub dist: f32,
}

/// JS `Physics.segmentCapsuleDist`: 7-point sample + ternary-search refine.
#[must_use]
pub fn segment_capsule_dist(a: Vec3, b: Vec3, base: Vec3, radius: f32, height: f32) -> CapsuleHit {
    let mut best = f32::INFINITY;
    let mut bt = 0.0f32;
    let steps = 6usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let d = point_capsule_dist(a.lerp(b, t), base, radius, height);
        if d < best {
            best = d;
            bt = t;
        }
    }
    let mut lo = (bt - 1.0 / steps as f32).max(0.0);
    let mut hi = (bt + 1.0 / steps as f32).min(1.0);
    for _ in 0..8 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        let d1 = point_capsule_dist(a.lerp(b, m1), base, radius, height);
        let d2 = point_capsule_dist(a.lerp(b, m2), base, radius, height);
        if d1 < d2 {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let t = (lo + hi) * 0.5;
    CapsuleHit {
        t,
        dist: point_capsule_dist(a.lerp(b, t), base, radius, height),
    }
}

/// JS `Projectiles._ballistic`: secant-iterate the launch pitch so the
/// straight+gravity+drag integrator passes through `target`. No-op outside
/// `1.5 < hd < maxDist`, when `grav == 0`, or when the solution is
/// unreachable (`|err| > 0.25` or pitch change `> 0.35`).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn ballistic(
    from: Vec3,
    dir: Vec3,
    target: Vec3,
    speed: f32,
    straight: f32,
    grav: f32,
    drag: f32,
    max_dist: f32,
) -> Vec3 {
    let hx = target.x - from.x;
    let hz = target.z - from.z;
    let hd = hx.hypot(hz);
    if hd < 1.5 || hd > max_dist || grav == 0.0 {
        return dir;
    }
    let dy = target.y - from.y;
    let hdir = dir.x.hypot(dir.z);
    if hdir < 1e-4 {
        return dir;
    }
    let sim = |pitch: f32| -> f32 {
        let mut vh = pitch.cos() * speed;
        let mut vy = pitch.sin() * speed;
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut age = 0.0f32;
        for _ in 0..90 {
            age += SIM_DT;
            let px = x;
            let py = y;
            if age > straight {
                vy -= grav * SIM_DT;
                let k = 1.0 - drag * SIM_DT;
                vh *= k;
                vy *= k;
            }
            x += vh * SIM_DT;
            y += vy * SIM_DT;
            if x >= hd {
                let f = (hd - px) / (x - px).max(1e-6);
                return py + (y - py) * f;
            }
            if vh < 0.5 {
                break;
            }
        }
        -1e3
    };
    let mut p0 = dir.y.atan2(hdir);
    let mut e0 = sim(p0) - dy;
    if e0.abs() < 0.01 {
        return dir;
    }
    let mut p1 = p0 - e0.atan2(hd);
    let mut e1 = sim(p1) - dy;
    for _ in 0..4 {
        if e1.abs() <= 0.005 {
            break;
        }
        let d = e1 - e0;
        if d.abs() < 1e-6 {
            break;
        }
        let p2 = p1 - e1 * (p1 - p0) / d;
        p0 = p1;
        e0 = e1;
        p1 = p2.clamp(-1.2, 1.2);
        e1 = sim(p1) - dy;
    }
    if e1.abs() > 0.25 || (p1 - dir.y.atan2(hdir)).abs() > 0.35 {
        return dir;
    }
    let cp = p1.cos();
    vec3(dir.x / hdir * cp, p1.sin(), dir.z / hdir * cp)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod brolly_tests;
