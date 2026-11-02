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

pub mod blade;
pub mod bow;
pub mod brolly;
pub mod mitts;

use std::collections::BTreeMap;

use glam::{Vec3, vec3};

use crate::actor::{Actor, FireGate, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatKind, SplatOpts};
use crate::sub::{SubKind, SubSystem, ink_cost};
use crate::tuning::{
    Blaster, Brush, Bucket, Charger, PlayerTuning, Roller, Spinner, Spritzer, SubSet, Twins,
    WeaponSet,
};
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

/// The batch-3 bubble registry handle threaded into the projectile step
/// (JS `G.specials` at the `shotHit` / `rayHit` / `areaHit` call sites,
/// specials.js L442-470): the world-object registry plus the match RNG the
/// bubble effects draw from. `None` = no specials system wired (the
/// standalone weapon tests, the sub-kit bomb fan).
pub type Bubbles<'a> = Option<(&'a mut Vec<crate::special::world::WorldObject>, &'a mut Rng)>;

/// `explode_bomb`'s device-blast callback: sub-device damage + the splash
/// geometry, kept behind a closure so the bomb loop's borrows stay disjoint.
type DevBlast<'a> = &'a mut dyn FnMut(Option<&mut SubSystem>, Vec3, f32, f32, usize, bool);

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

    /// The content `id` string (config.js L96-240; id == kind for all 12
    /// roster weapons). JS `packEvent` (netmatch.js L803-813) passes
    /// `w.id` through to the `['ev']` payload, and the remote dispatches
    /// `WEAPONS[e.weapon]` — so the wire id must be the real weapon's,
    /// never the Spritzer default.
    #[must_use]
    pub fn id_str(self) -> &'static str {
        match self {
            Self::Shooter => "shooter",
            Self::Blaster => "blaster",
            Self::Charger => "charger",
            Self::Roller => "roller",
            Self::Bucket => "bucket",
            Self::Spinner => "spinner",
            Self::Twins => "twins",
            Self::Brush => "brush",
            Self::Brolly => "brolly",
            Self::Bow => "bow",
            Self::Blade => "blade",
            Self::Mitts => "mitts",
        }
    }

    /// Whether this kind's update branch is wired (Task 29: `Shooter`;
    /// Task 30: `Twins` / `Blaster` / `Brolly`; Task 31: `Charger` /
    /// `Spinner` / `Bucket`; Task 32: `Roller` / `Brush` / `Blade`;
    /// Task 33 flips the rest).
    #[must_use]
    pub fn is_wired(self) -> bool {
        matches!(
            self,
            Self::Shooter
                | Self::Twins
                | Self::Blaster
                | Self::Brolly
                | Self::Charger
                | Self::Spinner
                | Self::Bucket
                | Self::Roller
                | Self::Brush
                | Self::Blade
                | Self::Bow
                | Self::Mitts
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
    /// Blast-burst override (JS `o.burst`, weapons.js `fireCustom` L1205:
    /// `burst: o.burst || null`): a specials' blast round detonates with
    /// these numbers instead of `WEAPONS.blaster` (`_blastBurst` L1691).
    /// `None` for every runner-spawned round.
    pub burst: Option<BurstSpec>,
}

/// Per-round blast-burst override (JS `p.burst`, weapons.js L1205, read by
/// `_blastBurst` L1691: `w = b ? { burstRadius: b.radius, impactRadius:
/// b.paint ?? b.radius * 0.6, splashRadius: b.splashRadius, splashDamageMax:
/// b.dmgMax, splashDamageMin: b.dmgMin } : WEAPONS.blaster`). Task 35 batch
/// 4: carried by the jetpack's blast round (specials.js L1566-1567).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct BurstSpec {
    /// JS `b.radius` → `burstRadius` (the `weapon:impact` event radius).
    pub radius: f32,
    /// JS `b.paint` → `impactRadius` (under-burst splat radius; `None` =
    /// `b.radius * 0.6`).
    #[serde(default)]
    pub paint: Option<f32>,
    /// JS `b.splashRadius` → `splashRadius`.
    pub splash_radius: f32,
    /// JS `b.dmgMax` → `splashDamageMax`.
    pub splash_damage_max: f32,
    /// JS `b.dmgMin` → `splashDamageMin`.
    pub splash_damage_min: f32,
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
            burst: None,
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
    /// Blast-burst override (JS `p.burst`, weapons.js `fireCustom` L1205):
    /// `Some` = detonate with these numbers instead of `WEAPONS.blaster`
    /// (`_blastBurst` L1691). Only specials' `fireCustom` rounds carry it
    /// (Task 35 batch 4: the jetpack blast, specials.js L1566-1567).
    #[serde(default)]
    pub burst: Option<BurstSpec>,
    /// Brolly pellet mark (JS `p._bid` + the kit's `PELLETS` side table,
    /// brolly.js L72/L105): body hits merge per blast+victim and score with
    /// distance falloff instead of the round's `damage` (which is 0 — the
    /// core pass never applies it, JS L1608 `if (dmg > 0)`).
    #[serde(default)]
    pub pellet: Option<PelletMark>,
    /// Bucket below-falloff rate per metre (JS `p.belowFalloff`, weapons.js
    /// L1322 / config.js L144): a body hit takes
    /// `1 − clamp((start.y − (victim.y + 0.8) − 0.5) · rate, 0, max)` of the
    /// damage when the victim stands below the throw (`_step` L1605).
    /// `0` = no falloff (every non-bucket round).
    #[serde(default)]
    pub below_falloff: f32,
    /// Below-falloff cap (JS `p.belowFalloffMax`, L1322 / config.js L144).
    #[serde(default)]
    pub below_falloff_max: f32,
    /// Bucket volley group id (JS `p.volley`, L1316/L1322): blobs of one
    /// throw share it; the first blob to reach a victim hurts it, the rest
    /// pass through and still paint (`_step` L1599). Bookkeeping lives on
    /// [`ProjectileSim`] (`volley_hits` / `volley_alive`).
    #[serde(default)]
    pub volley: Option<u32>,
    /// Drop-type far damage (JS `p.dmgFar`, weapons.js L1226/L1407/blade.js
    /// L149): a body hit takes `lerp(damage, dmg_far, clamp(start→hit / 7))`
    /// instead of the flat `damage` (`_step` L1603, applied *before* the
    /// bucket below-falloff). `None` = shot-family round (no falloff).
    #[serde(default)]
    pub dmg_far: Option<f32>,
    /// Staggered launch delay in seconds (JS `p.delay`, weapons.js L1571 /
    /// blade.js L151 `delay: t * 0.018`): while positive the round does not
    /// integrate at all — the step only counts the delay down.
    #[serde(default)]
    pub delay: f32,
    /// JS `p.sp` (weapons.js L851 `undefined` + the lazy stamp at L1570):
    /// `None` = not yet stamped (JS `_new` leaves it undefined), `Some` =
    /// `!!(owner && owner.specialActive)` read on the round's first update
    /// tick. A stamped-true round's ink never charges the special meter
    /// (`_credit` → `addTurfNoSpecial`, L845) even after the special ends.
    #[serde(default)]
    pub sp: Option<bool>,
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
        /// Charger-only fields (JS L1281 `charge` / `len`, consumed by the
        /// remote `_ghostBeam` presentation, weapons.js L898); `None` for
        /// every other kind (Task 31 §2).
        #[serde(default)]
        charge: Option<f32>,
        #[serde(default)]
        len: Option<f32>,
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
        /// JS `_credit(o, area)` (weapons.js L845): the round/bomb was
        /// stamped `sp` (fired during a special), so the credit routes to
        /// `addTurfNoSpecial` — turf counts, the gauge never charges.
        #[serde(default)]
        no_special: bool,
    },
    /// A trail drip painted the ground (JS `_step` trail branch → `_credit`).
    /// No `weapon:impact` is emitted for drips upstream, so the turf credit
    /// rides its own event.
    Turf {
        owner_slot: usize,
        team: usize,
        area: f32,
        /// JS `_credit(o, area)` (weapons.js L845 / subs.js L594): the
        /// source was stamped `sp` — credit without charging the gauge.
        #[serde(default)]
        no_special: bool,
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
    /// A sub device entered the world (JS `netRec(a, 'subs', [op, gid, kind,
    /// ...])`, subs.js L77/L103 → replication `['k', nid, 'subs', data]`).
    /// `data` is the JS record array with the kind string lifted out to
    /// `sub_kind` (the wire `data` payload is numeric-only) and the kind's
    /// slot kept as a `0.0` placeholder so the element indices match JS.
    SubSpawn {
        owner_slot: usize,
        team: usize,
        sub_kind: String,
        data: Vec<f32>,
    },
    /// A sub device ended (JS `netRec(it.owner, 'subs', [2, gid])`,
    /// subs.js L165). `data` is `[2, gid]`. `sub_kind` is `Some(kit)` for
    /// the five kit subs (the wire kind IS the kit id) and `None` for the
    /// classic `subs` devices (the kind rides `data[2]`).
    SubEnd {
        owner_slot: usize,
        team: usize,
        sub_kind: Option<String>,
        data: Vec<f32>,
    },
    /// A sub was deployed (JS `emit('sub:use', ...)`, subs.js L78/L104) —
    /// local presentation only, never forwarded (not in the netmatch
    /// `FORWARD` list).
    SubUse {
        owner_slot: usize,
        team: usize,
        kind: SubKind,
    },
    /// A splat bomb left a hand (JS `recBomb` → `['b', nid, kind, r2×6]`,
    /// weapons.js L1463 / netmatch.js L139-141). `kind` is `'bomb'` (the
    /// storm special never reaches the sim in Task 34, D-T34-1/D-T34-6).
    BombSpawn {
        owner_slot: usize,
        team: usize,
        kind: String,
        pos: Vec3,
        vel: Vec3,
    },
    /// A hit landed on one of *another screen's* devices (JS `sendDevHit` →
    /// `{k:'dh', kind, id, d}`, netmatch.js L157-159). `kind` is the device
    /// wire kind (`'subs'` / `'waddle'` / `'torpedo'` / `'torpedoArea'`);
    /// `gid` is the device's owner-side id; the replication layer rewrites
    /// it to the wire form (D-T34-5).
    DeviceHit {
        owner_slot: usize,
        team: usize,
        kind: String,
        gid: u32,
        d: f32,
    },
}

/// JS `runner.shaker` hold state (`newState`, shaker.js L166-170) — the
/// shaker bomb's charge meter. The presentation fields (`t0` / `fizz` /
/// `loop` / `axis` / `botLevel` / `botMax`) and the stat / FX counters
/// (`jumps` / `shakes` / `kick`) are dropped: they never feed the sim.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct ShakerCharge {
    /// JS `st.meter` — the 0..=1 charge fill.
    meter: f32,
    /// JS `st.level` — 1..=3, stepped by the meter thresholds.
    level: u32,
    /// JS `st.t1` — last hold timestamp (staleness + release window).
    t1: f32,
    /// JS `st.budget` — the rate-cap budget.
    budget: f32,
    /// JS `st.jumpPrev` — previous-frame jump intent (edge detect).
    jump_prev: bool,
    /// JS `st.yawPrev` — previous-frame aim yaw (waggle detect).
    yaw_prev: f32,
    /// JS `st.lookSign` — last fast-turn direction.
    look_sign: i32,
    /// JS `st.lastWag` — last counted waggle time.
    last_wag: f32,
}

/// `pending_kit_level` serde default (the shaker's un-charged level).
fn default_kit_level() -> u32 {
    1
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
    // ---- Task 31: charger / spinner charge clocks (JS `reset()` L40/L55).
    /// JS `chargeT` — charge clock seconds (charger / spinner / bow, L40).
    charge_t: f32,
    /// JS `chargeDinged` — full-charge feedback latch (audio/rumble only in
    /// JS; kept so the release path mirrors the state writes).
    charge_dinged: bool,
    /// JS `burstPower` — the spinner stream's charge at release (L45).
    burst_power: f32,
    /// JS `burstDur` — the spinner stream's full length (`ch` slot math for
    /// the retired splatling; kept for state parity, L55).
    burst_dur: f32,
    /// JS `streamCd` — spinner stream shot clock (L45).
    stream_cd: f32,
    /// A charger beam released this frame (JS `fireCharger` L210, called
    /// inline). The sim defers the hitscan to the match layer — the runner
    /// has no access to the actor list / paint grid — so the beam's RNG
    /// draws land after the whole runner pass instead of inline (deviation
    /// D-T31-1, spec §1; determinism holds, the order is fixed).
    pub(crate) charger_release: Option<f32>,
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
    // ---- Task 32: roller / brush melee state (JS `reset()` L41/L48/L57).
    /// JS `swipeSide` — brush swipe direction, alternating (reset to 1).
    swipe_side: i32,
    /// JS `lastRollPos` — the position sampled at the last paint step
    /// (roller drum / brush bristles; JS `reset()` L43 `null`, set on the
    /// roll edge before the first read).
    last_roll_pos: Vec3,
    /// JS `rollHits` (weapons.js L41/L264/L608): per-victim melee hit
    /// cooldowns. JS stores absolute `G.time` stamps and tests
    /// `G.time - last > cd`; the sim counts the window down per frame
    /// instead (no global clock) — equivalent: an absent entry is ready,
    /// a hit re-arms the countdown, every frame decays it.
    roll_hits: BTreeMap<usize, f32>,
    /// JS `this.kit` for the blade (weapons.js L57; blade.js `st()` L38-44):
    /// `None` = no kit yet (JS creates it lazily, same as brolly's `kitOf`).
    pub(crate) blade: Option<Box<blade::BladeKit>>,
    /// A wave spawned this frame (JS `addWave` → the world `WAVES` list,
    /// blade.js L261-265): the match takes it into its registry.
    pub(crate) blade_wave: Option<blade::BladeWave>,
    // ---- Task 33: bow / mitts kit state (JS `this.kit`, weapons.js L57;
    // `None` = no kit yet, like blade / brolly). The bow's draw lives on the
    // runner itself (`charging` / `charge` / `charge_t`, weapons.js L40-41).
    pub(crate) bow: Option<Box<bow::BowKit>>,
    pub(crate) mitts: Option<Box<mitts::MittsKit>>,
    // ---- Task 34: sub aiming (JS `aimingSub`, weapons.js L139-155).
    /// JS `this.aimingSub` — the sub button is held, the throw pose is on.
    pub(crate) aiming_sub: bool,
    /// D-T31-1 deploy flag (mirrors `charger_release`): the runner paid the
    /// ink for a non-bomb sub; the match layer calls
    /// [`crate::sub::SubSystem::use_sub`] with the full actor list (the
    /// runner has no access to it).
    pub(crate) pending_sub: bool,
    /// JS `runner.shaker` (shaker.js L166) — the shaker bomb's live hold
    /// charge; the release reads the level off it (shaker.js L359).
    #[serde(default)]
    pub(crate) shaker: Option<ShakerCharge>,
    /// D-T34 kit deploy level: the shaker's charge level (1..=3). Every
    /// other kit deploys at 1; the match layer hands this to
    /// [`crate::sub::SubSystem::use_sub`].
    #[serde(default = "default_kit_level")]
    pub(crate) pending_kit_level: u32,
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
            charge_t: 0.0,
            charge_dinged: false,
            burst_power: 0.0,
            burst_dur: 0.0,
            stream_cd: 0.0,
            charger_release: None,
            turret: false,
            aiming_sub: false,
            pending_sub: false,
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
            swipe_side: 1,
            last_roll_pos: Vec3::ZERO,
            roll_hits: BTreeMap::new(),
            blade: None,
            blade_wave: None,
            bow: None,
            mitts: None,
            shaker: None,
            pending_kit_level: 1,
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

    /// JS `endDodge()` (weapons.js L564-567): cut any dodge roll / planted
    /// lock short — called when a special takes over the body (actor.js
    /// L844). JS clears `dodgeT`, `turret`, `dodge` and `lockT`; the sim
    /// models the twins' roll through `dodge_t` (module docs), so the same
    /// three timers go: the roll stops, the stance unplants, the post-roll
    /// lock releases. `roll_lock` (JS `rollLock`, the dualies chain) is
    /// *not* JS-cleared here — it stays as-is.
    pub fn end_dodge(&mut self) {
        self.dodge_t = 0.0;
        self.turret = false;
        self.lock_t = 0.0;
    }

    /// JS `busy()` (L62): any state that blocks the actor. The retired-kind
    /// terms (`slosh` / `streaming` / `dodge`) are absent; the kit-weapon
    /// hook (L62 `MAIN_KITS[kind].busy`) is wired for brolly (Task 30) and
    /// blade (Task 32 — blade.js L681 `busy = !!K.swing`; a charge is
    /// deliberately *not* busy: the kid may dive with it held).
    #[must_use]
    pub fn busy(&self) -> bool {
        self.charging
            || self.flick >= 0.0
            || self.burst_t > 0.0
            || self.dodge_t > 0.0
            || self.lock_t > 0.0
            || self.blade.as_ref().is_some_and(|k| k.swing.is_some())
            // Task 33: JS L62 `MAIN_KITS.mitts.busy` (mitts.js L879) — only
            // the leap blocks the actor; the crouch and the cling do not.
            || self.mitts.as_ref().is_some_and(|k| k.leaping)
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
            // JS L64 `MAIN_KITS.blade.moveSpeed` (blade.js L683-689): the
            // heavy cut plants you (`moveSpeedFiring * 0.55` for the first
            // 0.3 s), a charge walks at `moveSpeedCharging`, the pose window
            // after a cut runs `moveSpeedFiring`; the falsy `0` fall-through
            // is the general tail (`firingT > 0 ? moveSpeedFiring : runSpeed`,
            // JS L89-90 — already covered by the third arm).
            WeaponKind::Blade => {
                let heavy_swing = self
                    .blade
                    .as_ref()
                    .is_some_and(|k| k.swing.as_ref().is_some_and(|s| s.heavy && s.t < 0.3));
                if heavy_swing {
                    w.blade.move_speed_firing * 0.55
                } else if self.blade.as_ref().is_some_and(|k| k.charging) {
                    w.blade.move_speed_charging
                } else if self.firing_t > 0.0 {
                    w.blade.move_speed_firing
                } else {
                    run
                }
            }
            // JS L64 `MAIN_KITS.bow.moveSpeed` (bow.js L500-504): drawing is
            // slowed, the loose pose window runs `moveSpeedFiring`; without
            // either the falsy fall-through is the general tail (JS L89-90).
            WeaponKind::Bow => {
                if self.charging {
                    lerp(
                        run * 0.75,
                        w.bow.move_speed_drawing,
                        (self.charge * 2.5).min(1.0),
                    )
                } else if self.firing_t > 0.0 {
                    w.bow.move_speed_firing
                } else {
                    run
                }
            }
            // JS L64 `MAIN_KITS.mitts.moveSpeed` (mitts.js L905-912): the
            // pounce keeps its launch speed, the crouch crawls, the post-land
            // hop is fast, the punch window slows you.
            WeaponKind::Mitts => {
                let k = self.mitts.as_ref();
                if k.is_some_and(|m| m.leaping) {
                    k.unwrap().leap_h.max(t.air_min_speed)
                } else if k.is_some_and(|m| m.charging) {
                    w.mitts.move_speed_charging
                } else if k.is_some_and(|m| m.land_t < 0.25) {
                    w.mitts.move_speed_charging * 2.0
                } else if self.firing_t > 0.0 {
                    w.mitts.move_speed_firing
                } else {
                    run
                }
            }
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
            // JS L95 `MAIN_KITS.mitts.spreadDeg` (mitts.js L913): punches
            // always use the kit spread (no bloom, no air branch).
            WeaponKind::Mitts => w.mitts.punch_spread,
            _ => 0.0,
        }
    }

    /// JS `WeaponRunner.update(dt, inp)` (L108-156): the common timer decay
    /// (L110-116) + the kind dispatch (L117-129). JS's `inp` is the gated
    /// `winp` (actor.js L370-373), which [`Actor::fire_gate`] already
    /// resolves (emerge delay + fire buffer) — hence the `gate` parameter
    /// alone; `move_dir` is the raw intent's move vector (JS reads
    /// `a.intent.move` in `_twins`, L493/L499/L534 — *not* the gated one).
    /// Task 32 additions: `foes` is the actor list split around `actor`
    /// (the roller / brush / blade melee scans hurt other actors while the
    /// runner keeps mutating its own); `paint` receives the melee stripe
    /// splats; `remote` routes proxy victims like [`ProjectileSim`]'s body
    /// hits; `intent_fire` is the raw `a.intent.fire` the blade kit reads
    /// (blade.js L630, *not* the gated `winp.fire`); `ps_time` is
    /// [`ProjectileSim::time`] — the kit's `G.time` stand-in.
    /// The sub-weapon block (JS L130-155) is Task 34.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        gate: &FireGate,
        actor: &mut Actor,
        kind: WeaponKind,
        w: &WeaponSet,
        sb: &SubSet,
        move_dir: Vec3,
        world: &CollisionWorld,
        projectiles: &mut ProjectileSim,
        foes: &mut FoeScan<'_>,
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        remote: Option<&[bool]>,
        intent_fire: bool,
        ps_time: f32,
        arrows: &mut Vec<bow::BowArrow>,
        fists: &mut Vec<mitts::MittsFist>,
        jump_held: bool,
        sub_blocked: bool,
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
        // Task 32: JS `rollHits` (L264/L608) stores absolute `G.time`
        // stamps; the sim counts each per-victim window down instead (no
        // global clock) — decay first, a hit re-arms `cd`, an absent entry
        // is ready (spec §1: equivalent windows).
        for v in self.roll_hits.values_mut() {
            *v -= dt;
        }
        self.roll_hits.retain(|_, v| *v > 0.0);

        // ---- JS L117-129 kind dispatch. Task 30 wires `shooter`/`blaster`
        // (`_auto`), `twins` (`_twins`) and `brolly` (`MAIN_KITS.brolly.
        // update`); Task 31 wires `charger` (`_charger`), `bucket`
        // (`_bucket`) and `spinner` (`_spinner`); Task 32 wires `roller`
        // (L120), `brush` (L124) and `blade` (L128 `default` → MAIN_KITS).
        // Remaining arms land in Task 33:
        //   L128 `default` → `MAIN_KITS` (bow/mitts).
        match kind {
            WeaponKind::Shooter | WeaponKind::Blaster => {
                self.auto_fire(gate, actor, kind, w, projectiles)
            }
            WeaponKind::Twins => self.twins(gate, actor, w, dt, move_dir, world, projectiles),
            WeaponKind::Brolly => brolly::update(self, dt, gate, actor, w, world, projectiles),
            // Task 31: JS L119 `charger` / L121 `bucket` / L122 `spinner`.
            WeaponKind::Charger => self.charger(dt, gate, actor, w, projectiles),
            WeaponKind::Spinner => self.spinner(dt, gate, actor, w, projectiles),
            WeaponKind::Bucket => self.bucket(dt, gate, actor, w, projectiles),
            // Task 32: JS L120 `roller` / L124 `brush`.
            WeaponKind::Roller => self.roller(
                dt,
                gate,
                actor,
                w,
                world,
                projectiles,
                foes,
                paint,
                t,
                remote,
            ),
            WeaponKind::Brush => self.brush(
                dt,
                gate,
                actor,
                w,
                move_dir,
                world,
                projectiles,
                foes,
                paint,
                t,
                remote,
            ),
            // Task 32: JS L128 `default` → `MAIN_KITS.blade.update`.
            WeaponKind::Blade => blade::update(
                self,
                dt,
                gate,
                intent_fire,
                actor,
                &w.blade,
                world,
                projectiles,
                foes,
                paint,
                t,
                remote,
                ps_time,
            ),
            // Task 33: JS L128 `default` → `MAIN_KITS.bow.update` /
            // `MAIN_KITS.mitts.update`. `jump_held` is the raw
            // `a.intent.jump` the mitts leap release reads (mitts.js L574).
            WeaponKind::Bow => {
                bow::update(self, dt, gate, actor, &w.bow, arrows, projectiles, ps_time)
            }
            WeaponKind::Mitts => mitts::update(
                self,
                dt,
                gate,
                jump_held,
                actor,
                &w.mitts,
                world,
                paint,
                t,
                fists,
                projectiles,
                foes,
                remote,
                ps_time,
            ),
        }

        // ---- Task 34/35: JS L130-155 sub weapon. Bomb Barrage (batch 3,
        // JS L132-133): while a barrage special is live the runner throws
        // the barrage's own bomb sub for *free* on the `nextThrow` gate —
        // no ink, no low-ink chirp, as fast as the throw gap allows. The
        // `sub` / `sub_released` edges ride `FireGate` (actor.rs L820-823,
        // the JS input layer's own edge computation).
        let sub_kind = crate::special::barrage::effective_sub(actor);
        // JS L132: `const bar = a.specialActive && a.specialActive.kind ===
        // 'barrage' ? a.specialActive : null` (live kinds only — a ghost or
        // ended active never runs a runner here).
        let bar = actor
            .special_active
            .as_ref()
            .and_then(|s| crate::special::barrage::state_of(s))
            .is_some();
        // JS L135: `if (this.aimingSub && SK?.hold) SK.hold(...)` — the
        // shaker is the only kit with a gameplay hold (the charge meter
        // feeds the release level, shaker.js L359).
        if self.aiming_sub && sub_kind == SubKind::Shaker {
            self.shaker_hold(dt, actor, sb, t, jump_held, ps_time);
        }
        if gate.sub && !self.aiming_sub {
            self.aiming_sub = true;
            // JS L138: the empty-tank chirp fires on the press edge for
            // every ink sub — never under the barrage (`!bar`, the throws
            // are free) and never for proxies (they run no runner).
            if !bar && actor.ink < ink_cost(sub_kind, sb) {
                projectiles.push_event(SimEvent::LowInk {
                    owner_slot: actor.slot,
                });
            }
        }
        if self.aiming_sub {
            // JS L140: holding the sub keeps the throw pose facing the aim.
            actor.set_fire_facing(0.3);
        }
        if gate.sub_released && self.aiming_sub {
            self.aiming_sub = false;
            // JS L143-145: a kit sub that refuses ("can't use") keeps its
            // ink; `sub:cantuse` is presentation-only, so the sim is silent.
            if !sub_blocked {
                // JS L146: `ok = !refused && (bar ? G.time >= (bar.nextThrow
                // || 0) : a.ink >= sub.inkCost)`.
                let next_throw = actor
                    .special_active
                    .as_ref()
                    .and_then(|s| crate::special::barrage::state_of(s))
                    .map_or(0.0, |st| st.next_throw);
                let ok = if bar {
                    ps_time >= next_throw
                } else {
                    actor.ink >= ink_cost(sub_kind, sb)
                };
                if ok {
                    if bar {
                        // JS L148: `bar.nextThrow = G.time + bar.def.gap` —
                        // the volley re-arms on the throw gap, no ink spent.
                        let gap = actor
                            .special_active
                            .as_ref()
                            .and_then(|s| match crate::special::def_by_id(&s.id) {
                                Some(crate::special::SpecialEntry::Barrage(d)) => Some(d.gap),
                                _ => None,
                            })
                            .unwrap_or(0.0);
                        if let Some(s) = actor.special_active.as_mut()
                            && let Some(st) = crate::special::barrage::state_of_mut(s)
                        {
                            st.next_throw = ps_time + gap;
                        }
                    } else {
                        actor.ink -= ink_cost(sub_kind, sb);
                    }
                    // JS L149: `a.lastFire = 0` (the idle-refill clock resets
                    // on a throw too).
                    actor.note_fired();
                    if sub_kind == SubKind::Bomb {
                        // JS L151: the splat bomb stays in the projectile
                        // system; everything else deploys through `G.subs`.
                        projectiles.throw_bomb(actor, sb);
                    } else {
                        self.pending_sub = true;
                        // JS `use` (shaker.js L358-360): the release level
                        // rides the fresh charge (dropped after the throw);
                        // every other kit deploys at 1.
                        self.pending_kit_level = if sub_kind == SubKind::Shaker {
                            let lvl = self
                                .shaker
                                .as_ref()
                                .filter(|st| ps_time - st.t1 < 0.25)
                                .map_or(1, |st| st.level);
                            self.shaker = None;
                            lvl
                        } else {
                            1
                        };
                    }
                }
            }
        }
        if !gate.sub && !gate.sub_released {
            // JS L155: the button went up without a release edge (e.g. the
            // actor splatted mid-throw) — drop the aim.
            self.aiming_sub = false;
        }
    }

    /// JS `hold` (shaker.js L192-222) minus the presentation branches (the
    /// clink / `sub:charge` emit, the fizz loop, the hand FX and the bot
    /// hold). Deviation D-T34-10: the gamepad-axis waggles (`waggles`
    /// L181-187) read raw stick axes the sim input layer does not model;
    /// only the aim-yaw reversal waggles are ported.
    fn shaker_hold(
        &mut self,
        dt: f32,
        actor: &Actor,
        sb: &SubSet,
        t: &PlayerTuning,
        jump_held: bool,
        now: f32,
    ) {
        let cfg = &sb.shaker;
        // JS L194: a stale charge (the hold was interrupted) drops — the
        // fizzle FX is presentation-only.
        if self.shaker.is_some_and(|st| now - st.t1 > 0.15) {
            self.shaker = None;
        }
        let st = self.shaker.get_or_insert(ShakerCharge {
            meter: 0.0,
            level: 1,
            t1: now,
            budget: 0.1,
            jump_prev: jump_held,
            yaw_prev: actor.aim_yaw,
            look_sign: 0,
            last_wag: -9.0,
        });
        st.t1 = now;
        // JS L197: the barrage free-ink branch is not wired (D-T34-6).
        let paid = actor.ink >= cfg.ink_cost;
        let jp = jump_held && !st.jump_prev;
        st.jump_prev = jump_held;
        // JS `waggles` L175-180: a fast aim-turn reversal counts once.
        let mut dy = actor.aim_yaw - st.yaw_prev;
        while dy > std::f32::consts::PI {
            dy -= std::f32::consts::TAU;
        }
        while dy < -std::f32::consts::PI {
            dy += std::f32::consts::TAU;
        }
        st.yaw_prev = actor.aim_yaw;
        let mut wag = 0.0f32;
        if dt > 0.0 && (dy / dt).abs() > 3.0 {
            let s = dy.signum() as i32;
            if s != st.look_sign {
                if st.look_sign != 0 && now - st.last_wag > 0.05 {
                    wag = 1.0;
                    st.last_wag = now;
                }
                st.look_sign = s;
            }
        }
        if paid && st.meter < 1.0 {
            let mv = ((actor.vel.x.hypot(actor.vel.z) - 1.0) / (t.run_speed - 1.0)).clamp(0.0, 1.0);
            let mut add = (dt / cfg.charge_time) * (1.0 + cfg.move_boost * mv);
            if jp {
                add += cfg.jump_boost;
            }
            if wag > 0.0 {
                add += wag * cfg.shake_boost;
            }
            // JS L206: bursts of presses can't beat the rate cap.
            st.budget = (st.budget + cfg.max_rate * dt).min(0.3);
            add = add.min(st.budget);
            st.budget -= add;
            st.meter = (st.meter + add).min(1.0);
        }
        let lv = if st.meter >= 1.0 {
            3
        } else if st.meter >= 0.5 {
            2
        } else {
            1
        };
        if lv > st.level {
            st.level = lv;
            // JS L216-219: the clink + `sub:charge` emit are presentation
            // only (never forwarded) — the sim stays silent.
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

    /// JS `_charger` (weapons.js L184-216): hold to charge on the S-curve
    /// (quick first 20 %, committed middle, crisp top-off, L195), release to
    /// fire the hitscan beam. The beam itself is deferred to the match layer
    /// ([`Self::charger_release`], deviation D-T31-1): the runner has no
    /// actor list / paint grid for the hitscan + damage routing.
    fn charger(
        &mut self,
        dt: f32,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        ps: &mut ProjectileSim,
    ) {
        let cw = &w.charger;
        if gate.fire && self.cooldown <= 0.0 {
            if !self.charging {
                // JS L188: a charge needs at least 20 % of `inkFull`.
                if a.ink < cw.ink_full * 0.2 {
                    if self.empty_cd <= 0.0 {
                        self.empty_cd = EMPTY_CD;
                        ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
                    }
                    return;
                }
                self.charging = true;
                self.charge = 0.0;
                self.charge_t = 0.0;
                self.charge_dinged = false;
            }
            // JS L192-196: charge builds on the S-curve, capped by the ink.
            let max_charge = clamp_f(a.ink / cw.ink_full, 0.0, 1.0);
            // JS's double sum of `dt/chargeTime` overshoots 1.0 on the final
            // frame and clamps exactly there; an f32 sum lands ~3e-7 low, so
            // the clock snaps to 1.0 within an ULP-scale tolerance to keep
            // the full-charge latch and the release value JS-observable.
            let next = self.charge_t + dt / cw.charge_time;
            self.charge_t = if next > 1.0 - 1e-6 { 1.0 } else { next };
            let t = self.charge_t;
            let curve = if t < 0.2 {
                t * 1.25
            } else {
                0.25 + (t - 0.2) * 0.9375
            };
            self.charge = max_charge.min(curve);
            // JS L199-203: the full-charge ding is audio/rumble only; the
            // latch mirrors the state write.
            if self.charge >= 1.0 && !self.charge_dinged {
                self.charge_dinged = true;
            }
        } else if self.charging {
            // JS L204-215: release. `c` floors at 0.12 so a tap still fires.
            self.charging = false;
            let c = self.charge.max(0.12);
            a.ink = (a.ink - cw.ink_full * c).max(0.0);
            a.note_fired();
            self.charger_release = Some(c);
            self.charge = 0.0;
            self.charge_t = 0.0;
            self.firing_t = FIRING_T;
            self.cooldown = 0.28; // JS L214
        }
    }

    /// JS `_spinner` (weapons.js L450-486): hold to spin up (linear charge,
    /// no S-curve), release to fire a stream whose length, speed and reach
    /// follow the charge. The stream is a burst of `fireStream` rounds
    /// driven by `burstT`/`streamCd` — *not* the retired splatling's
    /// `streaming`/`burstFrac` state (JS L381-426), so the `ch` tick slot
    /// stays `charge` (0 during the burst) and `F.charging` covers the
    /// spin-up only (spec §3).
    fn spinner(
        &mut self,
        dt: f32,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        ps: &mut ProjectileSim,
    ) {
        let sp = &w.spinner;
        if self.burst_t > 0.0 {
            // JS L452-461: stream phase.
            self.burst_t -= dt;
            self.stream_cd -= dt;
            self.firing_t = 0.2; // JS L453
            a.note_fired();
            let mut guard = 0;
            while self.stream_cd <= 0.0 && guard < 4 {
                guard += 1;
                let sd = self.spread_deg(a.grounded, WeaponKind::Spinner, w);
                ps.fire_stream(a, sp, self.burst_power, sd, None);
                self.stream_cd += sp.fire_interval;
            }
            if self.burst_t <= 0.0 {
                self.burst_t = 0.0;
                self.cooldown = 0.2; // JS L458
            }
            return;
        }
        if gate.fire && self.cooldown <= 0.0 {
            if !self.charging {
                // JS L465: a spin needs at least 15 % of `inkFull`.
                if a.ink < sp.ink_full * 0.15 {
                    if self.empty_cd <= 0.0 {
                        self.empty_cd = EMPTY_CD;
                        ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
                    }
                    return;
                }
                self.charging = true;
                self.charge = 0.0;
                self.charge_t = 0.0;
                self.charge_dinged = false;
            }
            // JS L469-470: linear charge clock, ink-capped (same f32 snap
            // as the charger clock above).
            let next = self.charge_t + dt / sp.charge_time;
            self.charge_t = if next > 1.0 - 1e-6 { 1.0 } else { next };
            self.charge = clamp_f(a.ink / sp.ink_full, 0.0, 1.0).min(self.charge_t);
            if self.charge >= 1.0 && !self.charge_dinged {
                self.charge_dinged = true;
            }
        } else if self.charging {
            // JS L478-485: release into the burst.
            self.charging = false;
            let c = self.charge.max(0.15);
            a.ink = (a.ink - sp.ink_full * c).max(0.0);
            self.burst_power = c;
            self.burst_t = lerp(sp.burst_min, sp.burst_max, c);
            self.burst_dur = self.burst_t;
            self.stream_cd = 0.0;
            self.charge = 0.0;
            self.charge_t = 0.0;
        }
    }

    /// JS `_bucket` (weapons.js L434-447): a short wind-up swing (`flick`),
    /// then the arc volley leaves (`fireSlosh`); holding fire keeps hurling
    /// once the cooldown clears. A fresh throw may start from a tap
    /// (`pressed`), not only a held trigger (JS L441 `inp.fire ||
    /// inp.firePressed`).
    fn bucket(
        &mut self,
        dt: f32,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        ps: &mut ProjectileSim,
    ) {
        let bw = &w.bucket;
        if self.flick >= 0.0 {
            self.flick += dt;
            if self.flick >= bw.windup {
                self.flick = -1.0;
                ps.fire_slosh(a, bw, None);
                self.cooldown = bw.fire_interval - bw.windup; // JS L438
                self.firing_t = FIRING_T;
            }
            return;
        }
        if !(gate.fire || gate.pressed) || self.cooldown > 0.0 {
            return;
        }
        if a.ink < bw.ink_per_shot {
            if self.empty_cd <= 0.0 {
                self.empty_cd = EMPTY_CD;
                ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
            }
            return;
        }
        a.ink -= bw.ink_per_shot;
        a.note_fired();
        self.flick = 0.0; // JS L444
    }

    /// JS `_empty()` (weapons.js L158-163): the low-ink `lowink` emit behind
    /// an `EMPTY_CD` throttle (audio / rumble are presentation-only).
    fn empty_click(&mut self, ps: &mut ProjectileSim, a: &Actor) {
        if self.empty_cd <= 0.0 {
            self.empty_cd = EMPTY_CD;
            ps.push_event(SimEvent::LowInk { owner_slot: a.slot });
        }
    }

    /// JS `_roller` (weapons.js L218-290): the drum rolls forward laying a
    /// `roll` stripe and crushes enemies in front of it; `firePressed`
    /// winds up the flick lob (boss L269-273 and audio/rumble out of scope).
    /// The melee scan runs *before* the `moved < 0.28` paint gate (JS L258
    /// vs L274) and collects victims first (disjoint borrows).
    #[allow(clippy::too_many_arguments)]
    fn roller(
        &mut self,
        dt: f32,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        world: &CollisionWorld,
        ps: &mut ProjectileSim,
        foes: &mut FoeScan<'_>,
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        remote: Option<&[bool]>,
    ) {
        let rw = &w.roller;
        // JS L221-232: flick wind-up → release.
        if self.flick >= 0.0 {
            self.flick += dt;
            if self.flick >= rw.flick_windup {
                self.flick = -1.0;
                ps.fire_flick(a, rw);
                self.cooldown = rw.flick_interval - rw.flick_windup;
                self.firing_t = 0.25; // JS L228 literal
                self.flick_recover = 0.18; // JS L229 literal
            }
            return;
        }
        // JS L233-242: the flick throw — a `pressed` edge, not held. A
        // low-ink press only clicks empty and *falls through* to rolling
        // (JS L234 has no `return` on the empty branch).
        if gate.pressed && self.cooldown <= 0.0 {
            if a.ink < rw.flick_ink {
                self.empty_click(ps, a);
            } else {
                a.ink -= rw.flick_ink;
                a.note_fired();
                self.flick = 0.0;
                return;
            }
        }
        // JS L243-252: the roll state machine.
        let can_roll = gate.fire && a.grounded && a.ink > 0.5 && self.cooldown <= 0.25;
        self.roll_t = if can_roll { self.roll_t + dt } else { 0.0 };
        if can_roll != self.rolling {
            self.rolling = can_roll;
            if can_roll {
                self.last_roll_pos = a.pos;
            }
        }
        if gate.fire && a.ink <= 0.5 {
            self.empty_click(ps, a);
        }
        if !self.rolling {
            return;
        }
        a.note_fired();
        // JS L254-267: drum damage in front of the roller, gated on the
        // roll speed (`hs > 1.0`). Scan first, apply after (borrow shape).
        let hs = a.vel.x.hypot(a.vel.z);
        let fx = a.yaw.sin();
        let fz = a.yaw.cos();
        let mut hits: Vec<usize> = Vec::new();
        for (ei, e) in foes.iter() {
            if e.team == a.team || !e.alive {
                continue;
            }
            let dx = e.pos.x - a.pos.x;
            let dz = e.pos.z - a.pos.z;
            let dy = e.pos.y - a.pos.y;
            let fwd = dx * fx + dz * fz;
            let lat = (dx * fz - dz * fx).abs();
            if fwd > -0.2
                && fwd < 1.35
                && lat < rw.roll_width / 2.0 + 0.35
                && dy.abs() < 1.2
                && hs > 1.0
                && !self.roll_hits.contains_key(&ei)
            {
                hits.push(ei);
            }
        }
        let (a_slot, a_team, a_pos) = (a.slot, a.team, a.pos);
        for ei in hits {
            self.roll_hits.insert(ei, 0.5); // JS L265 literal window
            let Some(v) = foes.get_mut(ei) else {
                continue;
            };
            let (victim_slot, victim_team) = (v.slot, v.team);
            let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
            if send {
                ps.push_event(SimEvent::HitSend {
                    attacker_slot: a_slot,
                    victim_slot,
                    victim_team,
                    team: a_team,
                    damage: rw.roll_damage,
                });
            } else {
                let outcome = apply_hit(v, rw.roll_damage, Some((a_team, a_slot)), Some(a_pos), t);
                ps.push_event(SimEvent::Hit {
                    attacker_slot: a_slot,
                    victim_slot,
                    team: a_team,
                    damage: rw.roll_damage,
                    killed: outcome.killed,
                });
            }
        }
        // JS L274-288: displacement ink + the three-stripe drum paint.
        let moved = a.pos.distance(self.last_roll_pos);
        if moved < 0.28 {
            return;
        }
        self.last_roll_pos = a.pos;
        a.ink = (a.ink - rw.roll_ink_per_meter * moved).max(0.0);
        let rx = fz;
        let rz = -fx;
        let stretch = vec3(fx, 0.0, fz);
        let mut area = 0.0;
        for i in -1..=1 {
            let off = i as f32 * rw.roll_width * 0.33;
            let pt = vec3(
                a.pos.x + fx * 0.75 + rx * off,
                a.pos.y + 0.35,
                a.pos.z + fz * 0.75 + rz * off,
            );
            // JS L285: one `Math.random` seed per splat, radius `0.62`
            // literal, `kind: 'roll'` (straight-edged band, not a blob).
            let seed = ps.rng.next_f32();
            area += paint.splat(
                world,
                pt,
                0.62,
                a.team,
                &SplatOpts {
                    seed,
                    stretch: Some(stretch),
                    stretch_amt: None,
                    kind: Some(SplatKind::Roll),
                },
            );
        }
        ps.push_event(SimEvent::Impact {
            pos: vec3(a.pos.x + fx * 0.75, a.pos.y + 0.02, a.pos.z + fz * 0.75),
            normal: Vec3::Y,
            team: a_team,
            radius: rw.roll_width / 2.0,
            victim_slot: None,
            owner_slot: a_slot,
            area,
            // JS L287: the roller band credits through `a.addTurf` (live
            // specialActive gate in `add_turf`), not the stamped `_credit`.
            no_special: false,
        });
    }

    /// JS `_brush` (weapons.js L571-626): brushing drags a thin stretched
    /// line and hurts enemies in front; standing still (or a `pressed` edge)
    /// swipes instead, lobbing a side sweep of drops (`swipeSide`
    /// alternates). The swipe wind-up is the JS-literal 0.04 s, not a config
    /// field (L575). Boss (L613-617) and audio/rumble out of scope.
    #[allow(clippy::too_many_arguments)]
    fn brush(
        &mut self,
        dt: f32,
        gate: &FireGate,
        a: &mut Actor,
        w: &WeaponSet,
        move_dir: Vec3,
        world: &CollisionWorld,
        ps: &mut ProjectileSim,
        foes: &mut FoeScan<'_>,
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        remote: Option<&[bool]>,
    ) {
        let bw = &w.brush;
        // JS L573-577: swipe wind-up → release.
        if self.flick >= 0.0 {
            self.flick += dt;
            if self.flick >= 0.04 {
                self.flick = -1.0;
                ps.fire_swipe(a, bw, self.swipe_side);
                self.cooldown = bw.swipe_interval;
                self.firing_t = 0.25; // JS L575 literal
            }
            return;
        }
        // JS L578-579: "moving" reads the raw intent (not the gated move).
        let hs = a.vel.x.hypot(a.vel.z);
        let moving = a.grounded && hs > 2.0 && move_dir.x.hypot(move_dir.z) > 0.3;
        // JS L580-590: the swipe throw. A low-ink press clicks empty and
        // falls through to the brushing block (no `return` on that branch).
        if (gate.pressed || (gate.fire && !moving)) && self.cooldown <= 0.0 {
            if a.ink < bw.swipe_ink {
                self.empty_click(ps, a);
            } else {
                a.ink -= bw.swipe_ink;
                a.note_fired();
                self.flick = 0.0;
                self.swipe_side = -self.swipe_side;
                self.rolling = false; // JS L587
                return;
            }
        }
        // JS L591-599: the brushing drag.
        let brushing = gate.fire && moving && a.ink > 0.3;
        self.roll_t = if brushing { self.roll_t + dt } else { 0.0 };
        if brushing != self.rolling {
            self.rolling = brushing;
            if brushing {
                self.last_roll_pos = a.pos;
                self.roll_hits.clear(); // JS L595: fresh bristles, fresh hits
            }
        }
        if !self.rolling {
            return;
        }
        a.note_fired();
        let fx = a.yaw.sin();
        let fz = a.yaw.cos();
        let mut hits: Vec<usize> = Vec::new();
        for (ei, e) in foes.iter() {
            if e.team == a.team || !e.alive {
                continue;
            }
            let dx = e.pos.x - a.pos.x;
            let dz = e.pos.z - a.pos.z;
            let dy = e.pos.y - a.pos.y;
            let fwd = dx * fx + dz * fz;
            let lat = (dx * fz - dz * fx).abs();
            if fwd > -0.2
                && fwd < 1.2
                && lat < bw.brush_width / 2.0 + 0.3
                && dy.abs() < 1.2
                && !self.roll_hits.contains_key(&ei)
            {
                hits.push(ei);
            }
        }
        let (a_slot, a_team, a_pos) = (a.slot, a.team, a.pos);
        for ei in hits {
            self.roll_hits.insert(ei, bw.brush_hit_cd);
            let Some(v) = foes.get_mut(ei) else {
                continue;
            };
            let (victim_slot, victim_team) = (v.slot, v.team);
            let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
            if send {
                ps.push_event(SimEvent::HitSend {
                    attacker_slot: a_slot,
                    victim_slot,
                    victim_team,
                    team: a_team,
                    damage: bw.brush_damage,
                });
            } else {
                let outcome = apply_hit(v, bw.brush_damage, Some((a_team, a_slot)), Some(a_pos), t);
                ps.push_event(SimEvent::Hit {
                    attacker_slot: a_slot,
                    victim_slot,
                    team: a_team,
                    damage: bw.brush_damage,
                    killed: outcome.killed,
                });
            }
        }
        // JS L618-624: displacement ink + the single bristle line splat.
        let moved = a.pos.distance(self.last_roll_pos);
        if moved < 0.22 {
            return;
        }
        self.last_roll_pos = a.pos;
        a.ink = (a.ink - bw.brush_ink_per_meter * moved).max(0.0);
        let seed = ps.rng.next_f32();
        let pt = vec3(a.pos.x + fx * 0.65, a.pos.y + 0.35, a.pos.z + fz * 0.65);
        // JS L623: no `kind` — a round blob stretched along the heading.
        let area = paint.splat(
            world,
            pt,
            bw.brush_width * 0.45,
            a_team,
            &SplatOpts {
                seed,
                stretch: Some(vec3(fx, 0.0, fz)),
                stretch_amt: Some(0.5),
                kind: None,
            },
        );
        ps.push_event(SimEvent::Impact {
            pos: vec3(a.pos.x + fx * 0.65, a.pos.y + 0.02, a.pos.z + fz * 0.65),
            normal: Vec3::Y,
            team: a_team,
            radius: bw.brush_width / 2.0,
            victim_slot: None,
            owner_slot: a_slot,
            area,
            // JS L623: direct `a.addTurf` (live gate), not `_credit`.
            no_special: false,
        });
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

    /// JS `this.charging` (the `F.charging` flag, netmatch.js L732; the
    /// charger / spinner spin-up window, Task 31).
    #[must_use]
    pub fn charging_flag(&self) -> bool {
        self.charging
    }

    /// JS `wr.flick >= 0` (the `F.flick` flag, netmatch.js L740; the
    /// bucket wind-up window, Task 31 — `flick` counts 0..windup while a
    /// throw is winding up and −1 otherwise).
    #[must_use]
    pub fn flick_flag(&self) -> bool {
        self.flick >= 0.0
    }

    /// The mitts kit, created lazily like JS `K(r)` (mitts.js L48-67) —
    /// the jump hook (`actor.js` L313 → [`mitts::MittsKit::jump`]) can run before
    /// the first [`mitts::update`], and `ink_min` must already read the
    /// leap-ink floor (L951) so an under-ink press is swallowed like JS.
    pub(crate) fn ensure_mitts(&mut self, w: &WeaponSet) -> &mut mitts::MittsKit {
        if self.mitts.is_none() {
            let mut k = mitts::MittsKit::new();
            k.ink_min = w.mitts.leap_ink_min;
            self.mitts = Some(Box::new(k));
        }
        self.mitts.as_mut().unwrap()
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

/// The actor list split around the runner's own actor (Task 32): the roller
/// / brush / blade melee scans read *and damage* other actors while the
/// runner keeps mutating its own, so the match hands the runner the two
/// disjoint sides (`lo` = indices `< self`, `hi` = indices `> self`,
/// `hi_off` = the first `hi` index in the full list). Indices are always
/// full-list indices, like the `remote` mask.
pub struct FoeScan<'a> {
    lo: &'a mut [Actor],
    hi: &'a mut [Actor],
    hi_off: usize,
}

impl<'a> FoeScan<'a> {
    /// Split `all` around `self_idx`: the runner's own actor plus the foe
    /// scan over everyone else (the match's per-actor loop).
    pub fn split_around(all: &'a mut [Actor], self_idx: usize) -> (&'a mut Actor, Self) {
        let (lo, rest) = all.split_at_mut(self_idx);
        let (me, hi) = rest.split_at_mut(1);
        (
            &mut me[0],
            Self {
                lo,
                hi,
                hi_off: self_idx + 1,
            },
        )
    }

    /// A scan over the full actor list (Task 33 §5: the fist steps run
    /// match-side with no "own" actor — the owner is filtered by team inside
    /// the scans themselves, so nothing needs excluding).
    pub fn all(all: &'a mut [Actor]) -> Self {
        let hi_off = all.len();
        Self {
            lo: all,
            hi: &mut [],
            hi_off,
        }
    }

    /// An empty scan (offline unit tests that only exercise fire paths).
    pub fn empty() -> Self {
        Self {
            lo: &mut [],
            hi: &mut [],
            hi_off: 0,
        }
    }

    /// `(full-list index, actor)` for every foe, low indices first.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Actor)> {
        self.lo.iter().enumerate().chain(
            self.hi
                .iter()
                .enumerate()
                .map(|(j, e)| (j + self.hi_off, e)),
        )
    }

    #[must_use]
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Actor> {
        if idx < self.lo.len() {
            Some(&mut self.lo[idx])
        } else {
            self.hi.get_mut(idx - self.hi_off)
        }
    }
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

/// Shared parameter bag for the `type: 'drop'` round family (roller
/// flick / brush swipe / blade drops): the JS `Object.assign` literals
/// (weapons.js L1226-1227 / L1407-1408, blade.js L147-152).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DropSpec {
    pub life: f32,
    pub straight: f32,
    pub size: f32,
    pub radius: f32,
    pub damage: f32,
    pub dmg_far: f32,
    pub trail_start: f32,
    pub trail_every: f32,
    pub trail_radius: f32,
    pub grav: f32,
    pub drag: f32,
    pub delay: f32,
}

/// A splat bomb in flight / armed (JS `this.bombs` entries, weapons.js
/// L1462 / L1477). The bomb family lives on the projectile stream because
/// its physics shares the round integrator; Task 35 batch 2 added the
/// `storm` special variant (`throwStorm`, weapons.js L1467-1479) — it
/// carries the throw heading in [`Self::dir`] and turns into a cloud on
/// landing instead of blasting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Bomb {
    pub kind: String,
    pub owner_slot: usize,
    pub team: usize,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    /// JS `fuse`: `-1` until the bomb first lands on a floor, then the
    /// countdown to the blast (weapons.js L1735-1741).
    pub fuse: f32,
    /// JS `b.ghost` (netmatch `ghostBomb`, weapons.js L877-885): a remote
    /// player's visual copy — it flies and explodes, but the mute wrap
    /// means it paints nothing and hurts nothing on this screen (the
    /// owner's screen decides the damage).
    pub ghost: bool,
    /// JS `b.dir` (weapons.js L1477, storm only): the throw heading, unit
    /// XZ — the cloud's drift vector. `None` for plain bombs (JS has no
    /// `dir` field there, L884 `if (b.dir)`).
    #[serde(default)]
    pub dir: Option<Vec3>,
    /// JS `b.sp` (weapons.js L1461 `sp: !!a.specialActive`, stamped at
    /// spawn): a bomb thrown during a special never charges the owner's
    /// gauge (`_credit` L845 → `addTurfNoSpecial`). Storm bombs carry the
    /// JS default (never set, L1477 — a storm bomb never blasts-credits).
    #[serde(default)]
    pub sp: bool,
}

/// Global projectile system (JS `Projectiles`): owns the live rounds, the
/// event queue and the RNG stream.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProjectileSim {
    pub list: Vec<Projectile>,
    /// Task 34: live splat bombs (JS `this.bombs`, weapons.js L1462).
    #[serde(default)]
    pub bombs: Vec<Bomb>,
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
    /// Bucket volley group counter (JS `volley = { hit: new Set() }` per
    /// `fireSlosh` throw, weapons.js L1316; the sim ids the group instead).
    #[serde(default)]
    volley_next: u32,
    /// Storm bombs that landed / timed out this step, waiting to become
    /// clouds (JS `_spawnCloud` called inline from `_updateBombs` L1729 /
    /// L1740, before `_updateClouds` L1760 steps them in the same frame).
    /// The match drains this right after [`Self::step_masked`].
    #[serde(default)]
    pub pending_clouds: Vec<crate::special::world::CloudSpawn>,
    /// `(group, victim_actor_index)` already hurt by a volley blob (JS
    /// `p.volley.hit` — the per-throw Set, L1599).
    #[serde(default)]
    volley_hits: BTreeMap<(u32, usize), ()>,
    /// Live blob count per volley group: the group's hit Set is dropped when
    /// its last blob despawns (JS lets the Set go with the blobs).
    #[serde(default)]
    volley_alive: BTreeMap<u32, usize>,
}

impl ProjectileSim {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            list: Vec::new(),
            bombs: Vec::new(),
            events: Vec::new(),
            rng: Rng::new(seed),
            time: 0.0,
            blast_seq: 0,
            pending: BTreeMap::new(),
            volley_next: 0,
            pending_clouds: Vec::new(),
            volley_hits: BTreeMap::new(),
            volley_alive: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn push_event(&mut self, e: SimEvent) {
        self.events.push(e);
    }

    /// Borrow the event queue (the specials' `blast` pushes directly so the
    /// weapon step's bubble hooks can hold `ps.rng` alongside it).
    pub fn events_mut(&mut self) -> &mut Vec<SimEvent> {
        &mut self.events
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
            burst: None,
        };
        let life = (spec.range / spec.proj_speed).min(spec.life);
        self.push_round(a, &spec, m, dir, life, true, None, true);
    }

    /// JS `Projectiles.fireCustom(a, m, dir, { type: 'blast', speed:
    /// d.projSpeed, damage: d.directDamage, range: d.range, weaponId:
    /// 'jetpack', burst: { radius: d.splashRadius, splashRadius:
    /// d.splashRadius, dmgMax: d.splashMax, dmgMin: d.splashMin, paint:
    /// d.paintRadius } })` (specials.js L1566-1567) — the jetpack's blast.
    /// The `fireCustom` blast defaults (weapons.js L1201-1205): `straight:
    /// 99`, `radius: 0.8`, `size: 0.26`, `grav: 0`, `drag: 0`, `trail:
    /// -1.5`, `trailEvery: 1.2`, `trailRadius: 0.42`, `life: range / speed`
    /// (the blast adds 0). JS does not emit `weapon:fire` from `fireCustom`
    /// (L1198-1213), so `emit_fire` is false.
    pub fn fire_jetpack_blast(&mut self, a: &Actor, d: &crate::tuning::Jetpack) {
        let m = Self::muzzle(a);
        let dir = Self::aim_from(a, m, None);
        let spec = RoundSpec {
            proj_speed: d.proj_speed,
            straight_time: 99.0, // JS `fireCustom` blast default (L1203)
            range: d.range,
            grav: 0.0, // JS blast default (L1204)
            drag: 0.0,
            size: 0.26,          // JS blast default (L1201)
            life: f32::INFINITY, // JS L1203: `range / speed` + 0
            impact_radius: 0.8,  // JS L1204 `radius ?? 0.8`
            damage: d.direct_damage,
            trail_every: 1.2, // JS L1204 `trailEvery ?? 1.2`
            trail_radius: 0.42,
            trail_start: -1.5, // JS L1204 `trail: -1.5`
            burst: Some(BurstSpec {
                radius: d.splash_radius,
                paint: Some(d.paint_radius),
                splash_radius: d.splash_radius,
                splash_damage_max: d.splash_max,
                splash_damage_min: d.splash_min,
            }),
        };
        let life = spec.range / spec.proj_speed;
        self.push_round(a, &spec, m, dir, life, true, None, false);
    }

    /// JS `Projectiles.fireCustom(a, m, dir, { type: 'shot', speed:
    /// d.gunSpeed, damage: d.gunDamage, range: d.gunRange, straight: 0.4,
    /// radius: 0.55, trailEvery: 1.6, trailRadius: 0.35, weaponId: 'crab' })`
    /// (specials.js L1972) — one crab gatling round. `m` / `dir` come from
    /// the caller (the hull-frame muzzle + the spread-jittered heading,
    /// L1969-1971). The `fireCustom` shot defaults (weapons.js L1201-1205):
    /// `size: 0.14`, `grav: 28`, `drag: 0.8`, `trail: -1.5`, `life: range /
    /// speed + 0.6`. No `weapon:fire` emit (JS `fireCustom` L1198-1213).
    pub fn fire_crab_gatling(&mut self, a: &Actor, d: &crate::tuning::Crab, m: Vec3, dir: Vec3) {
        let spec = RoundSpec {
            proj_speed: d.gun_speed,
            straight_time: 0.4, // JS L1972 `straight: 0.4`
            range: d.gun_range,
            grav: 28.0, // JS shot default (L1204)
            drag: 0.8,
            size: 0.14,          // JS shot default (L1201)
            life: f32::INFINITY, // JS L1203: `range / speed + 0.6` below
            impact_radius: 0.55, // JS L1972 `radius: 0.55`
            damage: d.gun_damage,
            trail_every: 1.6, // JS L1972 `trailEvery: 1.6`
            trail_radius: 0.35,
            trail_start: -1.5, // JS L1204 `trail: -1.5`
            burst: None,
        };
        let life = spec.range / spec.proj_speed + 0.6;
        self.push_round(a, &spec, m, dir, life, false, None, false);
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
            burst: None,
        };
        // JS L1381 lives 1.2 s; the sim keeps its range-cap deviation.
        let life = (range / speed).min(SHOT_LIFE);
        self.push_round(a, &spec, m, dir, life, false, None, true);
    }

    /// JS `Projectiles.fireStream` (weapons.js L1335-1353): the spinner's
    /// stream shot — a shooter-family round whose speed / straight phase /
    /// range follow the burst power (L1338), fired through
    /// [`Self::fire_round`] so the RNG order matches JS (`_spread` draws,
    /// then the blob seed).
    pub fn fire_stream(
        &mut self,
        a: &Actor,
        w: &Spinner,
        power: f32,
        spread_deg: f32,
        aim_point: Option<Vec3>,
    ) {
        let speed = lerp(w.speed_min, w.speed_max, power);
        let straight = lerp(w.straight_min, w.straight_max, power);
        let range = lerp(w.range_min, w.range_max, power);
        let spec = RoundSpec {
            proj_speed: speed,
            straight_time: straight,
            range,
            grav: PROJ_GRAV, // JS L1339/1342 `28`
            drag: PROJ_DRAG, // JS L1339/1342 `0.8`
            size: 0.12,      // JS L1342
            life: 1.3,       // JS L1342
            impact_radius: w.impact_radius,
            damage: w.damage,
            trail_every: w.trail_every,
            trail_radius: w.trail_radius,
            trail_start: -(2.2 - w.trail_every), // JS L1342
            burst: None,
        };
        self.fire_round(a, &spec, spread_deg, aim_point);
    }

    /// JS `Projectiles._lobAim` (weapons.js L997-1025): the lowest launch
    /// pitch whose arc passes through `target`, swept with the integrator
    /// `update()` uses (gravity + drag from launch) and bisected at the
    /// first crossing. Keeps `dir`'s heading; leaves it alone (returns
    /// false) when the target is out of reach. No RNG draws (deterministic
    /// solver), `SIM_DT` steps like the ballistic solver.
    fn lob_aim(
        from: Vec3,
        dir: &mut Vec3,
        target: Vec3,
        speed: f32,
        grav: f32,
        drag: f32,
        max_dist: f32,
    ) -> bool {
        let hd = ((target.x - from.x).powi(2) + (target.z - from.z).powi(2)).sqrt();
        if hd < 1.0 || hd > max_dist {
            return false;
        }
        let dy = target.y - from.y;
        let f = |pitch: f32| -> f32 {
            let mut vh = pitch.cos() * speed;
            let mut vy = pitch.sin() * speed;
            let (mut x, mut y) = (0.0f32, 0.0f32);
            for _ in 0..180 {
                let (px, py) = (x, y);
                vy -= grav * SIM_DT;
                let k = 1.0 - drag * SIM_DT;
                vh *= k;
                vy *= k;
                x += vh * SIM_DT;
                y += vy * SIM_DT;
                if x >= hd {
                    return py + (y - py) * ((hd - px) / (x - px).max(1e-6)) - dy;
                }
                if y < dy - 8.0 {
                    break;
                }
            }
            -1e3
        };
        let mut fp = f(-0.6);
        let mut p = -0.58f32;
        while p <= 1.2 {
            let fv = f(p);
            if fp < 0.0 && fv >= 0.0 {
                let mut lo = p - 0.02;
                let mut hi = p;
                for _ in 0..10 {
                    let mid = (lo + hi) / 2.0;
                    if f(mid) >= 0.0 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let h = dir.x.hypot(dir.z);
                let hdir = if h == 0.0 { 1.0 } else { h }; // JS `|| 1`
                let cp = hi.cos();
                *dir = vec3(dir.x / hdir * cp, hi.sin(), dir.z / hdir * cp);
                return true;
            }
            fp = fv;
            p += 0.02;
        }
        false
    }

    /// JS `Projectiles.fireSlosh` (weapons.js L1307-1332): the bucket's
    /// volley — `blobs` heavy blobs thrown along the aim, the default arc
    /// lifted by `lob` (sails over cover) and bent onto the crosshair by
    /// [`Self::lob_aim`] when it is in reach. The lead blob is fastest and
    /// fattest; each later blob tapers speed (`1 − i·0.075`), size
    /// (`0.22 − i·0.02`) and paint radius (`1 − i·0.08`), and starts
    /// `0.12 m` further back along the throw direction. RNG order per blob:
    /// side jitter → pitch jitter → blob seed (3 draws × blobs); one
    /// `weapon:fire` per throw (JS L1330).
    pub fn fire_slosh(&mut self, a: &Actor, w: &Bucket, aim_point: Option<Vec3>) {
        let m = Self::muzzle(a);
        let mut dir = Self::aim_from(a, m, aim_point);
        // JS L1312-1313: lift the aim by the lob, keeping the heading.
        let h0 = dir.x.hypot(dir.z);
        let hd0 = if h0 == 0.0 { 1.0 } else { h0 }; // JS `|| 1`
        let p0 = dir.y.atan2(hd0) + w.lob;
        dir = vec3(dir.x / hd0 * p0.cos(), p0.sin(), dir.z / hd0 * p0.cos());
        // JS L1314: bend the arc onto the crosshair when it's in reach.
        if let Some(aim) = aim_point {
            Self::lob_aim(m, &mut dir, aim, w.throw_speed, w.gravity, w.drag, w.reach);
        }
        let hd = dir.x.hypot(dir.z);
        let pitch = dir.y.atan2(if hd == 0.0 { 1.0 } else { hd }); // JS L1315
        let group = self.volley_next;
        self.volley_next += 1;
        let blobs = w.blobs as usize;
        self.volley_alive.insert(group, blobs);
        let yaw0 = dir.x.atan2(dir.z);
        for i in 0..blobs {
            let fi = i as f32;
            // JS L1318: per-blob speed taper + side/pitch jitter.
            let sp = w.throw_speed * (1.0 - fi * 0.075);
            let side = (self.rng.next_f32() - 0.5) * 0.05;
            let pi = pitch + (self.rng.next_f32() - 0.5) * 0.04;
            let size = 0.22 - fi * 0.02; // JS L1321
            let radius = w.impact_radius * (1.0 - fi * 0.08); // JS L1322
            let yaw = yaw0 + side;
            let cp = pi.cos();
            let vdir = vec3(yaw.sin() * cp, pi.sin(), yaw.cos() * cp);
            let pos = m - dir * (fi * 0.12); // JS L1324
            let spec = RoundSpec {
                proj_speed: sp,
                straight_time: 0.0, // JS L1322 `straight: 0`
                range: f32::INFINITY,
                grav: w.gravity,
                drag: w.drag,
                size,
                life: 2.2, // JS L1322
                impact_radius: radius,
                damage: w.damage,
                trail_every: 1.6, // JS L1322
                trail_radius: 0.42,
                trail_start: 0.0, // JS L1322 `trail: 0`
                burst: None,
            };
            self.push_round(a, &spec, pos, vdir, 2.2, false, None, false);
            if let Some(p) = self.list.last_mut() {
                p.below_falloff = w.below_falloff;
                p.below_falloff_max = w.below_falloff_max;
                p.volley = Some(group);
            }
        }
        self.push_event(SimEvent::Fire {
            owner_slot: a.slot,
            team: a.team,
            muzzle: m,
            dir,
            charge: None,
            len: None,
        });
    }

    /// JS `Projectiles._new()` + the drop `Object.assign` + `_push`
    /// (weapons.js L1223-1231): a drop round with its seed already drawn by
    /// the caller (the per-family RNG order must stay verbatim), `prev` /
    /// `start` pinned to the spawn position like JS.
    pub(crate) fn push_drop(&mut self, a: &Actor, d: &DropSpec, pos: Vec3, vel: Vec3, seed: f32) {
        self.list.push(Projectile {
            pos,
            prev: pos,
            start: pos,
            vel,
            team: a.team,
            owner_slot: a.slot,
            age: 0.0,
            life: d.life,
            straight: d.straight,
            radius: d.radius,
            size: d.size,
            damage: d.damage,
            grav: d.grav,
            drag: d.drag,
            trail: d.trail_start,
            trail_every: d.trail_every,
            trail_radius: d.trail_radius,
            seed,
            blast: false,
            pellet: None,
            burst: None,
            ghost: false,
            below_falloff: 0.0,
            below_falloff_max: 0.0,
            volley: None,
            dmg_far: Some(d.dmg_far),
            delay: d.delay,
            sp: None,
        });
    }

    /// JS `Projectiles.fireFlick` (weapons.js L1215-1236, minus the
    /// presentation `vis` draw and the boss/recoil/audio tail): the roller's
    /// lobbed ink sheet. RNG order per drop: [ang jitter, speed, radius,
    /// seed, cu] — `vis` (L1227) is presentation-only and not drawn (Task 29
    /// rule).
    pub(crate) fn fire_flick(&mut self, a: &Actor, w: &Roller) {
        let m = vec3(a.pos.x, a.pos.y + 1.0, a.pos.z);
        let fx = a.yaw.sin();
        let fz = a.yaw.cos();
        let up = clamp_f(a.aim_pitch, -0.2, 0.5) + 0.32;
        let drops = w.flick_drops as f32;
        for i in 0..w.flick_drops {
            let fi = i as f32;
            let t = (fi / (drops - 1.0)) * 2.0 - 1.0;
            let ang =
                a.yaw + t * w.flick_spread_deg * DEG * 0.5 + (self.rng.next_f32() - 0.5) * 0.05;
            let sp = w.flick_speed * (0.82 + 0.28 * (1.0 - t.abs()) + self.rng.next_f32() * 0.08);
            let radius = 0.85 + self.rng.next_f32() * 0.3;
            let seed = self.rng.next_f32();
            let cu = (up + (self.rng.next_f32() - 0.5) * 0.12).cos();
            let pos = vec3(m.x + fx * 0.6, m.y + 0.3, m.z + fz * 0.6); // JS L1231
            let vel = vec3(ang.sin() * cu * sp, up.sin() * sp, ang.cos() * cu * sp);
            self.push_drop(
                a,
                &DropSpec {
                    // JS L1226-1227: life 1.4, size 0.15, trailEvery 1.8,
                    // trailRadius 0.45, grav 26, drag 0.4.
                    life: 1.4,
                    straight: 0.0,
                    size: 0.15,
                    radius,
                    damage: w.flick_damage_near,
                    dmg_far: w.flick_damage_far,
                    trail_start: 0.0,
                    trail_every: 1.8,
                    trail_radius: 0.45,
                    grav: 26.0,
                    drag: 0.4,
                    delay: 0.0,
                },
                pos,
                vel,
                seed,
            );
        }
        self.push_event(SimEvent::Fire {
            owner_slot: a.slot,
            team: a.team,
            muzzle: vec3(m.x + fx * 0.6, m.y + 0.3, m.z + fz * 0.6),
            dir: vec3(fx, up.sin(), fz).normalize(),
            charge: None,
            len: None,
        });
    }

    /// JS `Projectiles.fireSwipe` (weapons.js L1396-1417, minus the
    /// presentation `vis` draw and the rumble tail): the brush's side sweep
    /// of drops. RNG order per drop: [speed, radius, seed, cu].
    pub(crate) fn fire_swipe(&mut self, a: &Actor, w: &Brush, side: i32) {
        let fx = a.yaw.sin();
        let fz = a.yaw.cos();
        let up = clamp_f(a.aim_pitch, -0.3, 0.3) * 0.5 + 0.08;
        let arc = w.swipe_spread_deg * DEG * 1.6;
        let sidef = side as f32;
        let denom = (w.swipe_drops as f32 - 1.0).max(1.0); // JS L1401 `max(1, …)`
        for i in 0..w.swipe_drops {
            let t = i as f32 / denom;
            let ang = a.yaw - sidef * (t - 0.5) * arc;
            let ox = ang.sin();
            let oz = ang.cos();
            let tx = -oz * sidef;
            let tz = ox * sidef;
            let sp = w.swipe_speed * (0.8 + self.rng.next_f32() * 0.3);
            let radius = 0.55 + self.rng.next_f32() * 0.2;
            let seed = self.rng.next_f32();
            let cu = (up + (self.rng.next_f32() - 0.5) * 0.08).cos();
            let pos = vec3(a.pos.x + ox * 0.85, a.pos.y + 0.45, a.pos.z + oz * 0.85); // JS L1409
            let hx = ox * 0.8 + tx * 0.45 + fx * 0.25;
            let hz = oz * 0.8 + tz * 0.45 + fz * 0.25;
            let hl = hx.hypot(hz);
            let vel = vec3(hx / hl * cu * sp, up.sin() * sp, hz / hl * cu * sp);
            self.push_drop(
                a,
                &DropSpec {
                    // JS L1404-1405: life 1.0, size 0.1, trailEvery 2.2,
                    // trailRadius 0.32, grav 26, drag 0.6.
                    life: 1.0,
                    straight: 0.0,
                    size: 0.1,
                    radius,
                    damage: w.swipe_damage_near,
                    dmg_far: w.swipe_damage_far,
                    trail_start: 0.0,
                    trail_every: 2.2,
                    trail_radius: 0.32,
                    grav: 26.0,
                    drag: 0.6,
                    delay: 0.0,
                },
                pos,
                vel,
                seed,
            );
        }
        self.push_event(SimEvent::Fire {
            owner_slot: a.slot,
            team: a.team,
            muzzle: vec3(a.pos.x + fx * 0.85, a.pos.y + 0.45, a.pos.z + fz * 0.85),
            dir: vec3(fx, up.sin(), fz).normalize(),
            charge: None,
            len: None,
        });
    }

    /// JS `Projectiles.fireCharger` (weapons.js L1238-1303, minus the
    /// boss/subs/specials cuts L1245-1246/L1259-1263 and the presentation
    /// tail): the full hitscan beam — world raycast, nearest enemy capsule
    /// along the beam (`+0.12` capsule pad, `+0.14` hit threshold, JS
    /// L1252-1253), damage routed like a body hit (`applyHit` L1264, before
    /// the paint), the ground-projected line splat (L1266-1272) and the
    /// terminal splat on a world hit with no victim (L1273-1278). RNG
    /// order: line seeds (one per ground hit) → terminal seed. The beam
    /// ribbon's `uSeed` (L1293) is presentation-only and not drawn (same
    /// rule as the shooter `vis`, Task 29 deviation).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_charger(
        &mut self,
        owner_slot: usize,
        team: usize,
        m: Vec3,
        dir: Vec3,
        charge: f32,
        cw: &Charger,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        sb: &SubSet,
        mut subs: Option<&mut SubSystem>,
        remote: Option<&[bool]>,
        mut bubbles: Bubbles<'_>,
    ) {
        // JS L1241-1244: range and damage follow the charge.
        let range = lerp(cw.range_min, cw.range_max, charge);
        let dmg = if charge >= 0.999 {
            cw.damage_max
        } else {
            lerp(cw.damage_min, cw.damage_max * 0.62, charge)
        };
        let mut hit = world.raycast(m, dir, range, true);
        let mut len = if hit.hit { hit.dist } else { range };
        // JS L1245: an enemy curtain / device cuts the beam short and cancels
        // the world hit; the beam still paints out to the cut distance.
        if let Some(s) = subs.as_deref_mut() {
            let cut = s.block_ray(m, dir, len, team, dmg, false, sb);
            if cut < len {
                len = cut;
                hit.hit = false;
            }
        }
        // JS L1246: `G.specials.rayHit(m, dir, len, a.team, dmg, a)` — a
        // bubble cuts the beam the same way. The charger beam is fired by a
        // live local actor, never under the ghost mute (`muted = false`).
        if let (Some((objs, brng)), Some(su)) = (bubbles.as_mut(), subs) {
            let mut ctx = crate::special::world::WorldCtx {
                actors: &mut *actors,
                collision: world,
                paint: &mut *paint,
                rng: brng,
                subs: su,
                sb,
                t,
                events: &mut self.events,
                remote,
                time: self.time,
            };
            let cut = crate::special::world::ray_hit(
                m,
                dir,
                len,
                team,
                dmg,
                Some(owner_slot),
                objs,
                &mut ctx,
                false,
            );
            if cut < len {
                len = cut;
                hit.hit = false;
            }
        }
        // JS L1247-1257: the first enemy along the beam (nearest wins).
        let mut victim: Option<(usize, f32)> = None;
        {
            let end = m + dir * len;
            for (ei, e) in actors.iter().enumerate() {
                if e.team == team || !e.alive {
                    continue;
                }
                let h = if e.form == Form::Squid {
                    t.squid_height
                } else {
                    t.height
                };
                let base = vec3(e.pos.x, e.visual_y(), e.pos.z);
                let res = segment_capsule_dist(m, end, base, t.radius + 0.12, h);
                if res.dist < t.radius + 0.14 {
                    let d = res.t * len;
                    let better = match victim {
                        Some((_, vd)) => d < vd,
                        None => true,
                    };
                    if better {
                        victim = Some((ei, d));
                    }
                }
            }
        }
        if let Some((_, d)) = victim {
            len = d;
        }
        // JS L1264: the beam stops on the victim and hurts them — before
        // the paint, like JS's emit order (`hit` then `weapon:fire/impact`).
        if let Some((ei, _)) = victim {
            let (victim_slot, victim_team) = (actors[ei].slot, actors[ei].team);
            let send = remote.is_some_and(|rm| rm.get(ei).copied().unwrap_or(false));
            if send {
                self.push_event(SimEvent::HitSend {
                    attacker_slot: owner_slot,
                    victim_slot,
                    victim_team,
                    team,
                    damage: dmg,
                });
            } else {
                let atk_pos = actor_pos_by_slot(actors, owner_slot);
                let outcome = apply_hit(&mut actors[ei], dmg, Some((team, owner_slot)), atk_pos, t);
                self.push_event(SimEvent::Hit {
                    attacker_slot: owner_slot,
                    victim_slot,
                    team,
                    damage: dmg,
                    killed: outcome.killed,
                });
            }
        }
        // JS L1265-1272: paint along the line, projected to the ground.
        let mut area = 0.0;
        let mut s = 1.2f32;
        while s < len - 0.3 {
            let g = world.raycast(m + dir * s, -Vec3::Y, 3.5, true);
            if g.hit {
                let seed = self.rng.next_f32();
                area += paint.splat(
                    world,
                    g.point + g.normal * 0.1,
                    cw.line_radius * (0.8 + charge * 0.4),
                    team,
                    &SplatOpts {
                        seed,
                        stretch: Some(dir),
                        stretch_amt: Some(1.2),
                        kind: None,
                    },
                );
            }
            s += cw.line_splat_every;
        }
        // JS L1273-1278: the terminal splat only on a world hit with no body.
        let rad = cw.impact_radius * (0.6 + 0.4 * charge);
        let mut normal = -dir;
        if hit.hit && victim.is_none() {
            let seed = self.rng.next_f32();
            area += paint.splat(
                world,
                hit.point + hit.normal * 0.12,
                rad,
                team,
                &SplatOpts {
                    seed,
                    stretch: Some(dir),
                    stretch_amt: Some(0.6),
                    kind: None,
                },
            );
            normal = hit.normal;
        }
        // JS L1280-1284: the fire event carries the beam's charge/len (the
        // remote `_ghostBeam` input); the impact lands at the beam end and
        // `a.addTurf(area)` rides its `area` field.
        self.push_event(SimEvent::Fire {
            owner_slot,
            team,
            muzzle: m,
            dir,
            charge: Some(charge),
            len: Some(len),
        });
        self.push_event(SimEvent::Impact {
            pos: m + dir * len,
            normal,
            team,
            radius: rad,
            victim_slot: victim.map(|(ei, _)| actors[ei].slot),
            owner_slot,
            area,
            // JS L1284: direct `a.addTurf` (live gate), not `_credit`.
            no_special: false,
        });
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
            burst: spec.burst,
            pellet,
            ghost: false,
            below_falloff: 0.0,
            below_falloff_max: 0.0,
            volley: None,
            dmg_far: None,
            delay: 0.0,
            sp: None,
        });
        if emit_fire {
            self.push_event(SimEvent::Fire {
                owner_slot: a.slot,
                team: a.team,
                muzzle: m,
                dir,
                charge: None,
                len: None,
            });
        }
    }

    /// JS `Projectiles.removeShot` (weapons.js L841): a round knocked out of
    /// the air — swap-remove, no impact, no blast (Mega Stamp's guard sweep,
    /// specials.js L1679). `idx` is the caller's current index into `list`.
    pub fn remove_shot(&mut self, idx: usize) {
        if idx < self.list.len() {
            self.list.swap_remove(idx);
        }
    }

    /// JS `Projectiles.defuseBomb` (weapons.js L843): "a bomb smashed before
    /// it goes off: gone, no blast" (Mega Stamp's guard sweep, specials.js
    /// L1685). `idx` is the caller's current index into `bombs`.
    pub fn defuse_bomb(&mut self, idx: usize) {
        if idx < self.bombs.len() {
            self.bombs.swap_remove(idx);
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
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        w: &WeaponSet,
        sb: &SubSet,
        bubbles: Bubbles<'_>,
    ) {
        let mut owner = ShieldOwner {
            runners: &mut [],
            launched: &mut [],
        };
        self.step_masked(
            dt,
            world,
            actors,
            paint,
            t,
            w,
            sb,
            None,
            None,
            &[],
            &mut owner,
            bubbles,
        );
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
        sb: &SubSet,
        mut subs: Option<&mut SubSystem>,
        remote: Option<&[bool]>,
        shields: &[Shield],
        owner: &mut ShieldOwner,
        mut bubbles: Bubbles<'_>,
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
            // JS `update` L1570: the lazy `sp` stamp — a round fired while
            // its owner's special was running never charges the meter,
            // decided on its first update tick (before the delay check, so
            // even a delayed glob stamps this frame). The owner resolves by
            // the `(slot, team)` identity (slots repeat per team).
            if self.list[i].sp.is_none() {
                let (slot, team) = (self.list[i].owner_slot, self.list[i].team);
                self.list[i].sp = Some(
                    actors
                        .iter()
                        .any(|x| x.slot == slot && x.team == team && x.special_active.is_some()),
                );
            }
            if self.step_one(
                dt,
                world,
                actors,
                paint,
                t,
                w,
                sb,
                subs.as_deref_mut(),
                i,
                remote,
                shields,
                owner,
                bubbles.as_mut().map(|(o, r)| (&mut **o, &mut **r)),
            ) {
                // JS lets the per-throw `volley.hit` Set die with its blobs
                // (the object is shared, weapons.js L1316/L1599): drop the
                // group's hit record when its last blob despawns.
                if let Some(g) = self.list[i].volley
                    && let Some(n) = self.volley_alive.get_mut(&g)
                {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        self.volley_alive.remove(&g);
                        self.volley_hits.retain(|&(gg, _), _| gg != g);
                    }
                }
                self.list.swap_remove(i);
            }
        }
        // JS `update` L1576: `_updateBombs` runs after the rounds loop.
        self.step_bombs(dt, world, actors, paint, t, sb, subs, remote, bubbles);
    }

    /// JS `throwBomb` (weapons.js L1449-1465): a splat bomb leaves the hand
    /// at `pos.y + 1.35` on the shared lob arc, `fuse: -1` until it lands.
    /// The JS `spin` random draws are presentation-only and not consumed
    /// (module docs). The `recBomb` record (L1463) rides
    /// [`SimEvent::BombSpawn`].
    pub(crate) fn throw_bomb(&mut self, a: &Actor, sb: &SubSet) {
        let pos = vec3(a.pos.x, a.pos.y + 1.35, a.pos.z);
        let vel = throw_velocity(a, sb.bomb.throw_speed);
        self.bombs.push(Bomb {
            kind: "bomb".to_string(),
            owner_slot: a.slot,
            team: a.team,
            pos,
            vel,
            age: 0.0,
            fuse: -1.0,
            ghost: false,
            dir: None,
            // JS L1461: `sp: !!a.specialActive`.
            sp: a.special_active.is_some(),
        });
        self.push_event(SimEvent::BombSpawn {
            owner_slot: a.slot,
            team: a.team,
            kind: "bomb".to_string(),
            pos,
            vel,
        });
    }

    /// JS `throwStorm` (weapons.js L1467-1479, `G.projectiles.throwStorm(a)`
    /// from actor.js L857): the storm special's lob at `pos.y + 1.45` on the
    /// shared `throwVelocity` arc with `SPECIALS.storm.throwSpeed`, `fuse: -1`
    /// (it never arms — it clouds on landing instead, L1729/L1740) and the
    /// unit-XZ throw heading on `dir` (L1477). Like `throwBomb` the JS
    /// presentation `spin` draws are not consumed (storm's spin is even
    /// constant, L1477). The `recBomb` (L1478) rides [`SimEvent::BombSpawn`]
    /// with `kind: "storm"`.
    pub fn throw_storm(&mut self, a: &Actor) {
        let sp = &crate::special::specials().storm;
        let pos = vec3(a.pos.x, a.pos.y + 1.45, a.pos.z); // JS L1473
        let vel = throw_velocity(a, sp.throw_speed); // JS L1476
        let dir = vec3(vel.x, 0.0, vel.z)
            .try_normalize()
            .unwrap_or(Vec3::ZERO); // JS L1477
        self.bombs.push(Bomb {
            kind: "storm".to_string(),
            owner_slot: a.slot,
            team: a.team,
            pos,
            vel,
            age: 0.0,
            fuse: -1.0,
            ghost: false,
            dir: Some(dir),
            // JS L1477 leaves `sp` unset on a storm bomb (it never
            // blast-credits — the cloud's rain is no-special ink by rule).
            sp: false,
        });
        self.push_event(SimEvent::BombSpawn {
            owner_slot: a.slot,
            team: a.team,
            kind: "storm".to_string(),
            pos,
            vel,
        });
    }

    /// JS `ghostBomb` (weapons.js L877-885, netmatch `'b'` inbound): the
    /// visual copy of a remote player's bomb. Task 35 batch 2 let the
    /// `storm` kind through (D-T34-1 retired): JS replays `throwStorm`
    /// (L879) and then overwrites `pos`/`vel` with the wire values and
    /// re-derives `dir` from the wire velocity (L884) — the sim builds the
    /// bomb straight from the wire values, so the discarded throw arc
    /// costs no RNG (JS `throwStorm` draws none either).
    pub fn ghost_bomb(
        &mut self,
        kind: String,
        pos: Vec3,
        vel: Vec3,
        owner_slot: usize,
        team: usize,
        sp: bool,
    ) {
        let dir = if kind == "storm" {
            // JS L884: `b.dir.set(vx, 0, vz).normalize()` (three.js
            // normalizes a zero vector to zero).
            let d = vec3(vel.x, 0.0, vel.z);
            if d.length_squared() > 0.0 {
                Some(d.normalize())
            } else {
                None
            }
        } else {
            None
        };
        self.bombs.push(Bomb {
            kind,
            owner_slot,
            team,
            pos,
            vel,
            age: 0.0,
            fuse: -1.0,
            ghost: true,
            dir,
            // JS L879 replays `throwBomb(a)` with the remote actor: its
            // `sp` (L1461) reads that actor's current `specialActive`.
            sp,
        });
    }

    /// JS `_updateBombs` (weapons.js L1711-1758) minus the boss /
    /// presentation branches. Bombs step after the rounds (JS `update`
    /// L1576). Task 35 batch 2 added the storm branch (L1729 / L1740): a
    /// `storm` bomb never bounces, arms or explodes — the first world
    /// touch *or* `age > 1.1` turns it into a cloud at the raw integrated
    /// position (`_spawnCloud(b)` reads `b.pos` before the bounce
    /// correction of L1730), queued on [`Self::pending_clouds`] so the
    /// match spawns it in the same frame (JS `_updateClouds` follows
    /// `_updateBombs` inside one `update`).
    #[allow(clippy::too_many_arguments)]
    fn step_bombs(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        sb: &SubSet,
        mut subs: Option<&mut SubSystem>,
        remote: Option<&[bool]>,
        mut bubbles: Bubbles<'_>,
    ) {
        let mut i = self.bombs.len();
        while i > 0 {
            i -= 1;
            let mut exploded = false;
            let mut clouded = false;
            {
                let b = &mut self.bombs[i];
                b.age += dt;
                b.vel.y -= 24.0 * dt;
                let prev = b.pos;
                b.pos += b.vel * dt;
                let hit = world.segment(prev, b.pos, false);
                if hit.hit && b.kind == "storm" {
                    // JS L1729: cloud + splice + `continue` — before the
                    // bounce / fuse arm of L1730-1738.
                    clouded = true;
                } else if hit.hit {
                    let n = hit.normal;
                    b.pos = hit.point + n * 0.21;
                    let vn = b.vel.dot(n);
                    b.vel += n * (-vn * 1.35);
                    b.vel *= if n.y > 0.6 { 0.45 } else { 0.6 };
                    if n.y > 0.6 && b.fuse < 0.0 {
                        // JS L1735: the first floor touch arms the fuse.
                        b.fuse = sb.bomb.fuse;
                    }
                }
                if b.kind == "storm" && !clouded && b.age > 1.1 {
                    // JS L1740: the age cap clouds it too (checked after
                    // the hit block, before the fuse ladder).
                    clouded = true;
                }
                if clouded {
                    self.pending_clouds.push(crate::special::world::CloudSpawn {
                        owner_slot: b.owner_slot,
                        team: b.team,
                        pos: b.pos,
                        dir: b.dir.unwrap_or(Vec3::ZERO),
                        ghost: b.ghost,
                    });
                }
                if !clouded && b.fuse >= 0.0 {
                    b.fuse -= dt;
                    if b.fuse <= 0.0 {
                        exploded = true;
                    }
                }
            }
            let mut dead = clouded;
            if exploded {
                let b = self.bombs[i].clone();
                // JS `damageArea` (subs.js L650-656): the bubble registry is
                // hurt inside `explode_bomb` (the `areaHit` line, before the
                // device loop); the `SubSystem` handle rides the fan argument
                // so the closure holds no borrow across the call.
                let mut blast = |su: Option<&mut SubSystem>,
                                 c: Vec3,
                                 r: f32,
                                 d: f32,
                                 team: usize,
                                 muted: bool| {
                    if let Some(su) = su {
                        su.damage_area(c, r, d, team, muted);
                    }
                };
                self.explode_bomb(
                    &b,
                    world,
                    actors,
                    paint,
                    t,
                    sb,
                    subs.as_deref_mut(),
                    bubbles.as_mut().map(|(o, r)| (&mut **o, &mut **r)),
                    &mut blast,
                    remote,
                );
                dead = true;
            }
            if !dead && self.bombs[i].pos.y < t.water_y - 1.8 {
                // JS L1758: a bomb that sails under the sea vanishes
                // silently — no blast, no credit.
                dead = true;
            }
            if dead {
                self.bombs.swap_remove(i);
            }
        }
    }

    /// JS `_explodeBomb` (weapons.js L1481-1507) minus the FX / audio /
    /// boss / rumble branches. The ghost mute wrap (L1752) is mirrored by
    /// `b.ghost`: the splats draw their seeds and paint nothing
    /// (paint.js L470), the device damage is dropped (`SubSystem::
    /// damage_area` mute guard) and every hit routes to `'drop'` — the
    /// owner's screen decides (netmatch.js L711-717).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn explode_bomb(
        &mut self,
        b: &Bomb,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        t: &PlayerTuning,
        sb: &SubSet,
        mut subs: Option<&mut SubSystem>,
        mut bubbles: Bubbles<'_>,
        dev_blast: DevBlast<'_>,
        remote: Option<&[bool]>,
    ) {
        let s = &sb.bomb;
        let c = b.pos;
        let mut area = 0.0;
        // JS L1483-1486: the main splat at `c.y + 0.2`, then five satellite
        // splats at `c.y + 0.5` — RNG order `[seed, (a, r, radius, seed)×5]`.
        let seed = self.rng.next_f32();
        if !b.ghost {
            let opts = SplatOpts {
                seed,
                stretch: None,
                stretch_amt: None,
                kind: None,
            };
            area += paint.splat(
                world,
                vec3(c.x, c.y + 0.2, c.z),
                s.paint_radius,
                b.team,
                &opts,
            );
        }
        for _ in 0..5 {
            let a = self.rng.next_f32() * std::f32::consts::TAU;
            let rr = s.paint_radius * (0.6 + self.rng.next_f32() * 0.4);
            let rad = 0.7 + self.rng.next_f32() * 0.5;
            let seed = self.rng.next_f32();
            if !b.ghost {
                let opts = SplatOpts {
                    seed,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                };
                area += paint.splat(
                    world,
                    vec3(c.x + a.cos() * rr, c.y + 0.5, c.z + a.sin() * rr),
                    rad,
                    b.team,
                    &opts,
                );
            }
        }
        if area > 0.0 {
            self.push_event(SimEvent::Turf {
                owner_slot: b.owner_slot,
                team: b.team,
                area,
                // JS `_credit(b, area)` L1489 → L845: `b.sp` routes to
                // `addTurfNoSpecial`.
                no_special: b.sp,
            });
        }
        // JS L1488: the blast hurts enemy devices (60 dmg) before the
        // player loop. `damageArea` (subs.js L650-652) runs the bubble
        // registry *first* and returns whole under the ghost mute.
        if !b.ghost
            && let (Some((objs, brng)), Some(su)) = (bubbles.as_mut(), subs.as_deref_mut())
        {
            let mut ctx = crate::special::world::WorldCtx {
                actors: &mut *actors,
                collision: world,
                paint: &mut *paint,
                rng: brng,
                subs: su,
                sb,
                t,
                events: &mut self.events,
                remote,
                time: self.time,
            };
            crate::special::world::area_hit(c, s.radius, 60.0, b.team, None, objs, &mut ctx, false);
        }
        dev_blast(subs, c, s.radius, 60.0, b.team, b.ghost);
        // JS L1500-1506: enemies inside the blast cone, line-of-sight
        // gated, quadratic falloff from `damage_min` to `damage_max`.
        let mut hits: Vec<(usize, f32)> = Vec::new();
        for (vi, e) in actors.iter().enumerate() {
            if e.team == b.team || !e.alive {
                continue;
            }
            let ep = vec3(e.pos.x, e.pos.y + 0.7, e.pos.z);
            let d = ep.distance(c);
            if d > s.radius {
                continue;
            }
            if !world.los(vec3(c.x, c.y + 0.3, c.z), ep) {
                continue;
            }
            let k = 1.0 - ((d - 0.8) / (s.radius - 0.8)).clamp(0.0, 1.0);
            hits.push((vi, lerp(s.damage_min, s.damage_max, k * k)));
        }
        for (vi, dmg) in hits {
            if b.ghost {
                continue; // `shouldApplyHit`: attacker remote → 'drop'
            }
            let (vs, vt) = (actors[vi].slot, actors[vi].team);
            let send = remote.is_some_and(|m| m.get(vi).copied().unwrap_or(false));
            if send {
                self.push_event(SimEvent::HitSend {
                    attacker_slot: b.owner_slot,
                    victim_slot: vs,
                    victim_team: vt,
                    team: b.team,
                    damage: dmg,
                });
            } else {
                let atk_pos = actor_pos_by_slot(actors, b.owner_slot);
                let outcome = apply_hit(
                    &mut actors[vi],
                    dmg,
                    Some((b.team, b.owner_slot)),
                    atk_pos,
                    t,
                );
                self.push_event(SimEvent::Hit {
                    attacker_slot: b.owner_slot,
                    victim_slot: vs,
                    team: b.team,
                    damage: dmg,
                    killed: outcome.killed,
                });
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
                let atk_pos = actor_pos_by_slot(actors, rec.owner_slot);
                let outcome = apply_hit(
                    &mut actors[rec.victim],
                    rec.dmg,
                    Some((rec.team, rec.owner_slot)),
                    atk_pos,
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
        sb: &SubSet,
        mut subs: Option<&mut SubSystem>,
        idx: usize,
        remote: Option<&[bool]>,
        shields: &[Shield],
        owner: &mut ShieldOwner,
        mut bubbles: Bubbles<'_>,
    ) -> bool {
        // JS `update` L1571: a delayed round only counts its delay down —
        // no integration, no age, no hit tests until it clears (blade
        // `fireDrops` staggers the sheet, blade.js L151). The brolly canopy
        // pre-scan skips the same rounds (brolly.js L428), which this early
        // return reproduces: a still-delayed round is not even looked at.
        if self.list[idx].delay > 0.0 {
            let p = &mut self.list[idx];
            p.delay -= dt;
            if p.delay > 0.0 {
                return false;
            }
        }
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
        let mut body_hit: Option<(usize, Vec3, Vec3, f32, f32)> = None;
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
                // JS L1599-1600: bucket volley — a blob whose throw already
                // landed on this victim passes through to the next enemy.
                if let Some(g) = p.volley {
                    if self.volley_hits.contains_key(&(g, si)) {
                        continue;
                    }
                    self.volley_hits.insert((g, si), ());
                }
                let at = p.prev.lerp(p.pos, res.t);
                let n = -p.vel.normalize_or_zero();
                // JS L1603: drop rounds fall off over 7 m from the launch
                // point to the hit point, *before* the below-falloff.
                let mut dmg = p.damage;
                if let Some(far) = p.dmg_far {
                    dmg = lerp(dmg, far, (p.start.distance(at) / 7.0).clamp(0.0, 1.0));
                }
                // JS L1605: bucket below-falloff — full damage at any range,
                // less only when the target stands below the thrower.
                if p.below_falloff > 0.0 {
                    dmg *= 1.0
                        - ((p.start.y - (e.pos.y + 0.8) - 0.5) * p.below_falloff)
                            .clamp(0.0, p.below_falloff_max);
                }
                body_hit = Some((si, at, n, p.radius * 0.5, dmg));
                break;
            }
        }
        if let Some((si, at, n, r, dmg)) = body_hit {
            let (owner_slot, team, blast, burst, pellet, start, ghost, no_sp) = {
                let p = &self.list[idx];
                (
                    p.owner_slot,
                    p.team,
                    p.blast,
                    p.burst,
                    p.pellet,
                    p.start,
                    p.ghost,
                    p.sp.unwrap_or(false),
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
                    let atk_pos = actor_pos_by_slot(actors, owner_slot);
                    let outcome =
                        apply_hit(&mut actors[si], dmg, Some((team, owner_slot)), atk_pos, t);
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
                    no_sp,
                    world,
                    actors,
                    paint,
                    w,
                    t,
                    remote,
                    subs.as_deref_mut(),
                    sb,
                    bubbles.as_mut().map(|(o, r)| (&mut **o, &mut **r)),
                    burst,
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
                    // A body hit paints nothing (`area` 0, no credit upstream);
                    // the flag rides the event for payload symmetry with `_sp`.
                    no_special: no_sp,
                });
            }
            dead = true;
        }

        // ---- enemy curtains / devices absorb the shot (JS weapons.js L1617:
        // `G.subs.blockShot(p.prev, p.pos, p.team, p.damage || 10)`). A ghost
        // round passes the `|| 10` damage but is muted inside, so the geometry
        // still swallows the shot while the hurt is discarded.
        if !dead {
            let (prev, pos, team, damage, ghost, owner_slot) = {
                let p = &self.list[idx];
                (p.prev, p.pos, p.team, p.damage, p.ghost, p.owner_slot)
            };
            let dmg = if damage != 0.0 { damage } else { 10.0 };
            if let Some(s) = subs.as_deref_mut()
                && s.block_shot(prev, pos, team, dmg, ghost, sb)
            {
                dead = true;
            }
            // JS L1618: `G.specials.shotHit(p.prev, p.pos, p.team,
            // p.damage || 10, p.owner)` — the first bubble the segment grazes
            // absorbs the round. Under the ghost mute the bubble effects are
            // dropped but the geometry hit still swallows the shot (JS
            // `_bubbleHit`'s `netMuted` guard, specials.js L474).
            if !dead && let (Some((objs, brng)), Some(su)) = (bubbles.as_mut(), subs.as_deref_mut())
            {
                let mut ctx = crate::special::world::WorldCtx {
                    actors: &mut *actors,
                    collision: world,
                    paint: &mut *paint,
                    rng: brng,
                    subs: su,
                    sb,
                    t,
                    events: &mut self.events,
                    remote,
                    time: self.time,
                };
                if crate::special::world::shot_hit(
                    prev,
                    pos,
                    team,
                    dmg,
                    Some(owner_slot),
                    objs,
                    &mut ctx,
                    ghost,
                ) {
                    dead = true;
                }
            }
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
                let (rad_base, seed, team, vel, blast, burst, owner_slot, no_sp) = {
                    let p = &self.list[idx];
                    (
                        p.radius,
                        p.seed,
                        p.team,
                        p.vel,
                        p.blast,
                        p.burst,
                        p.owner_slot,
                        p.sp.unwrap_or(false),
                    )
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
                            no_special: no_sp,
                        });
                    }
                    self.blast_burst(
                        hit.point,
                        None,
                        team,
                        owner_slot,
                        ghost,
                        no_sp,
                        world,
                        actors,
                        paint,
                        w,
                        t,
                        remote,
                        subs.as_deref_mut(),
                        sb,
                        bubbles.as_mut().map(|(o, r)| (&mut **o, &mut **r)),
                        burst,
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
                        no_special: no_sp,
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
                        let (owner_slot, no_sp) = {
                            let p = &self.list[idx];
                            (p.owner_slot, p.sp.unwrap_or(false))
                        };
                        self.push_event(SimEvent::Turf {
                            owner_slot,
                            team,
                            area,
                            no_special: no_sp,
                        });
                    }
                }
            }
        }

        // ---- life / sea expiry (JS `p.age > p.life`, `pos.y < waterY - 1.8`).
        // A blast detonates on life expiry (L1641-1644); the sea only kills
        // silently (L1645).
        if !dead {
            let (age, life, y, blast, burst, at, team, owner_slot, ghost, no_sp) = {
                let p = &self.list[idx];
                (
                    p.age,
                    p.life,
                    p.pos.y,
                    p.blast,
                    p.burst,
                    p.pos,
                    p.team,
                    p.owner_slot,
                    p.ghost,
                    p.sp.unwrap_or(false),
                )
            };
            if age > life {
                if blast {
                    self.blast_burst(
                        at,
                        None,
                        team,
                        owner_slot,
                        ghost,
                        no_sp,
                        world,
                        actors,
                        paint,
                        w,
                        t,
                        remote,
                        subs,
                        sb,
                        bubbles.as_mut().map(|(o, r)| (&mut **o, &mut **r)),
                        burst,
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
        no_sp: bool,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        w: &WeaponSet,
        t: &PlayerTuning,
        remote: Option<&[bool]>,
        subs: Option<&mut SubSystem>,
        sb: &SubSet,
        mut bubbles: Bubbles<'_>,
        burst: Option<BurstSpec>,
    ) {
        // JS `_blastBurst` L1691: `w = b ? { burstRadius: b.radius,
        // impactRadius: b.paint ?? b.radius * 0.6, splashRadius:
        // b.splashRadius, splashDamageMax: b.dmgMax, splashDamageMin:
        // b.dmgMin } : WEAPONS.blaster`.
        let (burst_r, paint_r, splash_r, dmax, dmin) = match burst {
            Some(b) => (
                b.radius,
                b.paint.unwrap_or(b.radius * 0.6),
                b.splash_radius,
                b.splash_damage_max,
                b.splash_damage_min,
            ),
            None => (
                w.blaster.burst_radius,
                w.blaster.impact_radius,
                w.blaster.splash_radius,
                w.blaster.splash_damage_max,
                w.blaster.splash_damage_min,
            ),
        };
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
            no_special: no_sp,
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
                let atk_pos = actor_pos_by_slot(actors, owner_slot);
                let outcome = apply_hit(&mut actors[ei], dmg, Some((team, owner_slot)), atk_pos, t);
                self.push_event(SimEvent::Hit {
                    attacker_slot: owner_slot,
                    victim_slot,
                    team,
                    damage: dmg,
                    killed: outcome.killed,
                });
            }
        }
        // JS L1707: `G.specials?.areaHit(c, w.splashRadius,
        // w.splashDamageMin, p.team, p.owner)` — every bubble in the splash
        // takes `dmin * 1.5` charge / soak (the `1.5` rides `area_hit`); a
        // ghost round's blast is muted inside `_bubbleHit`.
        if let (Some((objs, brng)), Some(su)) = (bubbles.as_mut(), subs) {
            let mut ctx = crate::special::world::WorldCtx {
                actors: &mut *actors,
                collision: world,
                paint: &mut *paint,
                rng: brng,
                subs: su,
                sb,
                t,
                events: &mut self.events,
                remote,
                time: self.time,
            };
            crate::special::world::area_hit(
                at,
                splash_r,
                dmin,
                team,
                Some(owner_slot),
                objs,
                &mut ctx,
                ghost,
            );
        }
    }
}

/// The attacker's live position by slot (JS passes the owner actor into
/// `victim.damage`; the Task 33 guard-arc snapshot reads the attacker XZ).
/// `None` when the owner is gone (despawned / never present).
pub(crate) fn actor_pos_by_slot(actors: &[Actor], slot: usize) -> Option<Vec3> {
    actors.iter().find(|x| x.slot == slot).map(|x| x.pos)
}

/// JS `Projectiles.applyHit` local route: `victim.damage`. The wrong-team /
/// dead guards of JS L1546 are enforced *upstream* here — the
/// [`ProjectileSim::step_one`] scan already skips `e.team === p.team ||
/// !e.alive` (JS L1593) before a body hit is ever recorded, so this function
/// only sees live enemies. JS still emits `hit` when invulnerability blocks
/// the damage (`victim.damage()` returns false), which [`HitOutcome::blocked`]
/// records; `killed` mirrors JS (the `victim.damage()` return).
pub(crate) fn apply_hit(
    v: &mut Actor,
    dmg: f32,
    attacker: Option<(usize, usize)>,
    attacker_pos: Option<Vec3>,
    t: &PlayerTuning,
) -> HitOutcome {
    let blocked = !v.alive || v.invuln() > 0.0;
    if blocked {
        return HitOutcome {
            killed: false,
            blocked: true,
        };
    }
    let killed = v.damage(dmg, attacker, attacker_pos, t);
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

/// JS `throwVelocity` (weapons.js L1443-1447): the lob arc shared by thrown
/// subs and splat bombs — aim yaw/pitch with a fixed +0.28 pitch bias, plus
/// 40 % of the thrower's own velocity.
#[must_use]
pub(crate) fn throw_velocity(a: &Actor, speed: f32) -> Vec3 {
    let pitch = (a.aim_pitch + 0.28).clamp(-0.3, 1.1);
    let cp = pitch.cos();
    vec3(
        a.aim_yaw.sin() * cp * speed + a.vel.x * 0.4,
        pitch.sin() * speed + 1.5,
        a.aim_yaw.cos() * cp * speed + a.vel.z * 0.4,
    )
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

#[cfg(test)]
mod blade_tests;

#[cfg(test)]
mod bow_tests;

#[cfg(test)]
mod mitts_tests;
