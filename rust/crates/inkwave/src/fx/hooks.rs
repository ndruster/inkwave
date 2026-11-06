//! Task 51 batch D — the gameplay → VFX glue, ported from
//! [`src/fx/fxHooks.js`](../../../../src/fx/fxHooks.js) (712 lines).
//!
//! JS drives discrete moments off the `on(name, fn)` event bus and keeps a
//! state-polling fallback for every event that "switches itself off the
//! first time the real event is seen" (L1-8). The Rust presentation layer
//! has no such bus: the only cross-boundary surface is
//! [`MatchEvent`] (`inkwave_sim::match_`, NFR-M4-1 forbids touching sim).
//! Events that exist there are consumed from `Match::events` with `.iter()`
//! (never drained — `ui_state_machine` owns the drain); every other JS
//! subscription keeps its polling fallback permanently on, exactly like the
//! JS fallbacks before their event is ever emitted.
//!
//! Consumed ≡ the L57-79 subscription table:
//!   * `weapon:fire`   ≡ [`MatchEvent::Fire`] (charger `len` also drives the
//!     beam trail + impact ≡ `_beams` L606-639 — Rust has no `P.beams` list)
//!   * `splatted`      ≡ [`MatchEvent::Splat`]
//!   * `respawn`       ≡ [`MatchEvent::Respawn`]
//!   * `hit`           ≡ [`MatchEvent::HitSend`] only (local-route hits stay
//!     in `SimEvent` and never reach the presentation layer — gap)
//!   * `special:use`   ≡ [`MatchEvent::SpecialUse`]
//!   * `special:slam`  ≡ [`MatchEvent::SpecialSlam`]
//!
//! Polled (no bus equivalent): `actor:*` (footstep/land/jump/form/dive/
//! emerge/enemyInk), `superjump` + `superjump:land` (via
//! [`Actor::super_jump`](inkwave_sim::actor::Actor)), `special:ready` (edge
//! on `special >= special_cost`), `weapon:dodge` (edge on the twins'
//! `dodge_velocity`), `storm:start` (new `kind == "cloud"` world object),
//! `lowink` — **gap**: no signal reaches the presentation layer, so
//! `_dryFire` is not wired. `weapon:impact` — **gap** (charger beam impact
//! is covered via `Fire.len` instead). `bomb:explode` was an empty handler
//! in JS already.
//!
//! Declared deviations (full list in the task report):
//!   * `_visual` ≡ `pos` — no `visualPos` smoothing layer ported;
//!   * footstep bone positions fall back to the body position (JS L334-335
//!     `character.bones` branch);
//!   * the wall-climb family (`_climb` / `climbDrip` / `climbPop`) is N/A:
//!     the Rust `Form` enum has no climb state;
//!   * `p.type === 'drop'` / `p.head` have no Rust field: flick-drop fire
//!     fallback + the slosh-head/dropHits machinery (L496-534) are not
//!     driven; `slosh` is detected via `Projectile::volley` **and**
//!     `below_falloff > 0` (bucket throws only — blade droplets share
//!     `volley` but never carry the bucket's below-falloff);
//!   * `Bomb` has no `beepT` → `beepPulse` is not driven;
//!   * `WeaponRunner` does not expose `dodge.t/dur` → `dodgeSkid` runs with
//!     the JS fallback `k = 0` (L430);
//!   * charger beam `charge` comes from the `Fire.charge` fraction instead
//!     of the JS `(th - 0.035) / 0.05` remap (L628);
//!   * `onSpeck` / `onRipple` (L39-48) → no-op placeholders: `PaintGrid`
//!     has no speck/ripple API and the presentation layer must not mutate
//!     sim paint (NFR-M4-1) — cosmetic, deferred to Task 53/55;
//!   * `decor.pulse` (respawn / super-jump pad) and `jumpMarker` for home
//!     jumps are not ported (decor surface absent; the Rust super jump is
//!     always the fixed home jump, D-T35-3);
//!   * ambient sea state is a constant `1.0` (no `G.env` port) and deck
//!     edges come from the stage AABB (the rotated-slab `env.footprint`
//!     branch of `_buildEdges` L690-695 is Cargo-Terminal-only);
//!   * per-actor / per-bomb / per-projectile state is indexed by roster /
//!     list position with `age`-continuity recycling (JS L484
//!     `p.age < p._fxAge` precedent) instead of JS object identity.

use std::collections::HashMap;

use glam::Vec3;
use inkwave_sim::actor::{Form, SpecialPhase, SplatCause};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::geometry::StageLayout;
use inkwave_sim::match_::{Match, MatchEvent};
use inkwave_sim::special::world::ObjectBody;
use inkwave_sim::tuning::Tuning;
use inkwave_sim::weapon::WeaponKind;

use super::api::{MuzzleKind, RingOpts};
use super::particles::{ColorIn, Fx, TAU, UP, hex_to_linear, th_normalize};

// ---------------------------------------------------------------------------
// constants (≡ config.js / fxHooks.js literals)
// ---------------------------------------------------------------------------

/// `PLAYER.hp` (config.js L52; JS L387 `1 - a.hp / PLAYER.hp`).
const PLAYER_HP: f32 = 100.0;
/// `SPECIALS.slam.hang` (JS L447).
const SLAM_HANG: f32 = 0.25;
/// `SPECIALS.slam.radius` (JS L212/L453).
const SLAM_RADIUS: f32 = 5.2;
/// `SPECIALS.storm.radius` (JS L263/L575).
const STORM_RADIUS: f32 = 3.4;
/// `SUB.bomb.radius` (JS L542).
const SUB_BOMB_RADIUS: f32 = 3.1;
/// `SUB.bomb.fuse` (JS L542).
const SUB_BOMB_FUSE: f32 = 0.95;

/// `G.teamColors[0]` ≡ `#ff8a14` in linear working space (ink.wgsl L27).
pub(crate) fn team_lin(team: usize) -> Vec3 {
    hex_to_linear(if team == 0 { 0xff8a14 } else { 0x2f5bff })
}

/// JS `a.enemyTeam` ≡ the other team.
fn enemy_team(team: usize) -> usize {
    if team == 0 { 1 } else { 0 }
}

/// `_inkColor(a, surface)` ≡ L105: enemy ink uses the enemy team colour.
fn ink_color_lin(a: &inkwave_sim::actor::Actor, surface: i32) -> Vec3 {
    if surface == 2 {
        team_lin(enemy_team(a.team))
    } else {
        team_lin(a.team)
    }
}

/// `_heading(a, out)` ≡ L101-104.
fn heading(a: &inkwave_sim::actor::Actor) -> Vec3 {
    let hs = a.vel.x.hypot(a.vel.z);
    if hs > 0.2 {
        Vec3::new(a.vel.x / hs, 0.0, a.vel.z / hs)
    } else {
        Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos())
    }
}

/// `a.character.getMuzzle(_v)` ≡ the muzzle anchor — the Rust port has no
/// character rig, so it approximates the gun mount (declared deviation).
fn approx_muzzle(a: &inkwave_sim::actor::Actor) -> Vec3 {
    a.pos + heading(a) * 0.55 + Vec3::Y * 1.1
}

/// JS `anim.form` ∈ {kid, squid, swim, climb}; the Rust `Form` enum only has
/// Kid/Squid and `submerged` distinguishes the swimming state (deviation:
/// climb is N/A).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxForm {
    Kid,
    Squid,
    Swim,
}

fn actor_form(a: &inkwave_sim::actor::Actor) -> FxForm {
    match a.form {
        Form::Kid => FxForm::Kid,
        Form::Squid if a.submerged => FxForm::Swim,
        Form::Squid => FxForm::Squid,
    }
}

/// `FIRE_KEY` ≡ L22 — the per-kind trigger counters.
fn fire_key(kind: WeaponKind) -> &'static str {
    match kind {
        WeaponKind::Shooter => "fire:shooter",
        WeaponKind::Blaster => "fire:blaster",
        WeaponKind::Charger => "fire:charger",
        WeaponKind::Roller => "fire:roller",
        _ => "fire:other",
    }
}

// ---------------------------------------------------------------------------
// per-frame side tables (JS stores these on the game objects themselves —
// the presentation layer must not mutate sim state, NFR-M4-1)
// ---------------------------------------------------------------------------

/// The per-actor state ≡ `FxHooks._state` (L86-96, 30 fields).
#[derive(Clone, Debug)]
pub struct ActorFxState {
    init: bool,
    born: f32,
    grounded: bool,
    vy: f32,
    form: FxForm,
    sub: bool,
    climb: bool,
    alive: bool,
    on_enemy: bool,
    step_acc: f32,
    foot: bool,
    drip_t: f32,
    spark_t: f32,
    sizzle_t: f32,
    climb_t: f32,
    bubble_t: f32,
    full: bool,
    sj: Option<&'static str>,
    sp: Option<String>,
    sp_t: f32,
    sj_land: f32,
    hit_t: f32,
    dry_t: f32,
    climb_end: f32,
    slam_pending: bool,
    last_sj_to: Vec3,
    swim_yaw: Option<f32>,
    carve_t: f32,
    emerge_t: f32,
    wet_t: f32,
    spin_full: bool,
    dodging: bool,
    dodge_dir: Vec3,
    /// polling edge tracker for the `special:ready` inference (JS is
    /// event-driven here; the Rust bus has no such event — deviation).
    ready: bool,
}

impl Default for ActorFxState {
    fn default() -> Self {
        Self {
            init: false,
            born: -9.0,
            grounded: true,
            vy: 0.0,
            form: FxForm::Kid,
            sub: false,
            climb: false,
            alive: false,
            on_enemy: false,
            step_acc: 0.0,
            foot: false,
            drip_t: 0.0,
            spark_t: 0.0,
            sizzle_t: 0.0,
            climb_t: 0.0,
            bubble_t: 0.0,
            full: false,
            sj: None,
            sp: None,
            sp_t: 0.0,
            sj_land: -9.0,
            hit_t: -9.0,
            dry_t: -9.0,
            climb_end: -9.0,
            slam_pending: false,
            last_sj_to: Vec3::ZERO,
            swim_yaw: None,
            carve_t: 0.0,
            emerge_t: -9.0,
            wet_t: 0.0,
            spin_full: false,
            dodging: false,
            dodge_dir: Vec3::new(0.0, 0.0, 1.0),
            ready: false,
        }
    }
}

impl ActorFxState {
    fn new() -> Self {
        Self::default()
    }
}

/// projectile side record ≡ the `p._fxAge/_fxD/_fxY` scratch fields (L484-512).
#[derive(Clone, Copy, Debug, Default)]
struct ProjFxRec {
    fx_d: f32,
    fx_y: f32,
    last_age: f32,
}

/// bomb side record ≡ the `bombs` Map value (L547).
#[derive(Clone, Copy, Debug)]
struct BombFxRec {
    gp: Vec3,
    gn: Vec3,
    #[allow(dead_code)] // mirrors JS `r.ground`; beepPulse is not driven
    ground: bool,
    vy: f32,
    armed: bool,
    last_age: f32,
}

impl Default for BombFxRec {
    fn default() -> Self {
        Self {
            gp: Vec3::ZERO,
            gn: UP,
            ground: false,
            vy: 0.0,
            armed: false,
            last_age: -1.0,
        }
    }
}

/// storm-cloud side record ≡ the `clouds` Map value (L583).
#[derive(Clone, Copy, Debug)]
struct CloudFxRec {
    stamp: u64,
    puddle_t: f32,
    flash_t: f32,
}

/// one candidate deck edge point for sea spray ≡ `_buildEdges` entries (L705).
#[derive(Clone, Copy, Debug)]
struct Edge {
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
}

// ---------------------------------------------------------------------------
// the runtime
// ---------------------------------------------------------------------------

/// `__inkwave.fxHooks.stats()` ≡ L711.
#[derive(Clone, Debug)]
pub struct HooksStats {
    pub count: HashMap<String, u32>,
    pub seen: Vec<&'static str>,
}

/// The `FxHooks` instance ≡ the JS class fields (L13-53).
#[derive(Clone, Debug)]
pub struct HooksRuntime {
    /// ≡ `this.enabled` (L52).
    pub enabled: bool,
    /// ≡ `this.time` (L49).
    pub time: f32,
    /// ≡ `this.stamp` (L49).
    pub stamp: u64,
    /// ≡ `this.seen` (L15) — only events that actually reach the
    /// presentation layer ever get marked, so the polling fallbacks for
    /// `actor:*` / `superjump` / `lowink` / … stay on (D-T51-7③).
    seen: Vec<&'static str>,
    /// ≡ `this.count` (L47).
    count: HashMap<String, u32>,
    /// ≡ `this.st` (L48) — indexed by roster position.
    st: Vec<ActorFxState>,
    /// ≡ the `p._fxAge/_fxD/_fxY` scratch fields (L484-512) — indexed by
    /// projectile-list position.
    proj_recs: Vec<ProjFxRec>,
    /// ≡ `this.bombs` (L48) — indexed by bomb-list position.
    bomb_recs: Vec<BombFxRec>,
    /// ≡ `this.clouds` (L48) — keyed by `WorldObject::gid`.
    clouds: HashMap<u32, CloudFxRec>,
    /// ≡ `this.flickT` (L51) — keyed by roster position.
    flick_t: HashMap<usize, f32>,
    /// ≡ `this.edges` / `this.edgeLevel` (L49/L53).
    edges: Vec<Edge>,
    edge_key: Option<(f32, f32, f32, f32)>,
    /// ≡ `this.sprayT` / `this.featherT` (L50/L64).
    spray_t: f32,
    feather_t: f32,
    /// per-frame camera inputs (≡ `G.camera`; None in headless play).
    cam: Option<Vec3>,
    cam_dir: Option<Vec3>,
    /// ≡ `a.isLocal` — the roster index of the local player, if any.
    local: Option<usize>,
}

impl Default for HooksRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl HooksRuntime {
    pub fn new() -> Self {
        Self {
            enabled: true,
            time: 0.0,
            stamp: 0,
            seen: Vec::new(),
            count: HashMap::new(),
            st: Vec::new(),
            proj_recs: Vec::new(),
            bomb_recs: Vec::new(),
            clouds: HashMap::new(),
            flick_t: HashMap::new(),
            edges: Vec::new(),
            edge_key: None,
            spray_t: 0.0,
            feather_t: 4.0, // ≡ L64 `4 + rand() * 6` — refined in `bootstrap`
            cam: None,
            cam_dir: None,
            local: None,
        }
    }

    /// JS constructor tail (L64): the feather timer consumes the first
    /// `rand()` draw of the shared stream. Also wires the paint callbacks ≡
    /// L39-48 — `PaintGrid` has no speck/ripple API and the presentation
    /// layer must not mutate sim paint (NFR-M4-1), so both are no-op
    /// placeholders (declared deviation, cosmetic, deferred to Task 53/55).
    pub fn bootstrap(&mut self, fx: &mut Fx) {
        self.feather_t = 4.0 + fx.rand() * 6.0;
        fx.on_speck = Some(Box::new(|_p, _n, _c, _s| {}));
        fx.on_ripple = Some(Box::new(|_p, _a, _w, _sp, _l| {}));
    }

    /// `_bump(k)` ≡ L84.
    pub fn bump(&mut self, k: &str) {
        *self.count.entry(k.to_string()).or_insert(0) += 1;
    }

    /// per-system trigger counter ≡ `this.count[k]` (L84).
    #[must_use]
    pub fn count_of(&self, k: &str) -> u32 {
        self.count.get(k).copied().unwrap_or(0)
    }

    /// `stats()` ≡ L711.
    #[must_use]
    pub fn stats(&self) -> HooksStats {
        let mut seen = self.seen.clone();
        seen.sort_unstable();
        HooksStats {
            count: self.count.clone(),
            seen,
        }
    }

    fn mark_seen(&mut self, name: &'static str) {
        if !self.seen.contains(&name) {
            self.seen.push(name);
        }
    }

    fn seen(&self, name: &'static str) -> bool {
        self.seen.contains(&name)
    }

    /// `_near(p, d)` ≡ L85 (camera position only).
    fn near(&self, p: Vec3, d: f32) -> bool {
        self.cam.is_some_and(|c| c.distance_squared(p) < d * d)
    }

    /// `_state(a)` ≡ L86-99 (grown lazily by roster index).
    fn state(&mut self, i: usize) -> &mut ActorFxState {
        if self.st.len() <= i {
            self.st.resize(i + 1, ActorFxState::new());
        }
        &mut self.st[i]
    }

    /// `_fresh(a)` ≡ L110.
    fn fresh(&self, i: usize) -> bool {
        let s = &self.st[i];
        self.time - s.born < 0.35 || !s.init
    }
}

// ---------------------------------------------------------------------------
// discrete moments (≡ fxHooks.js L113-288)
// ---------------------------------------------------------------------------

impl HooksRuntime {
    fn _footstep(
        &mut self,
        fx: &mut Fx,
        m: &Match,
        i: usize,
        pos: Vec3,
        surface: i32,
        speed: Option<f32>,
    ) {
        // ≡ L113-118
        let a = &m.actors[i];
        if !a.alive {
            return;
        }
        if !self.near(pos, if surface != 0 { 28.0 } else { 18.0 }) {
            return;
        }
        let hs = speed.unwrap_or_else(|| a.vel.x.hypot(a.vel.z));
        fx.footstep(
            pos,
            ColorIn::Rgb(ink_color_lin(a, surface)),
            surface,
            Some(heading(a)),
            Some(hs),
        );
        self.bump("footstep");
    }

    fn _land(&mut self, fx: &mut Fx, m: &Match, i: usize, pos: Vec3, speed: f32, surface: i32) {
        // ≡ L119-126
        let a = &m.actors[i];
        if !a.alive || speed < 3.5 {
            return;
        }
        let sj_land = self.st[i].sj_land;
        if a.special_active.is_some()
            || a.super_jump.is_some()
            || self.time - sj_land < 0.3
            || !self.near(pos, 38.0)
        {
            return;
        }
        // squid landing in own ink = a dive (actor:dive) ≡ L123 — JS tests
        // `a.form === 'squid'` regardless of submersion (raw sim form).
        if a.form == Form::Squid && surface == 1 {
            return;
        }
        fx.land(
            pos,
            ColorIn::Rgb(ink_color_lin(a, surface)),
            surface,
            Some(speed),
        );
        self.bump("land");
    }

    fn _jump(&mut self, fx: &mut Fx, m: &Match, i: usize, surface: i32, swim: bool) {
        // ≡ L127-131 — swim jumps: actor:emerge splashes
        let a = &m.actors[i];
        if swim || a.super_jump.is_some() || !self.near(a.pos, 30.0) {
            return;
        }
        // _visual(a) ≡ a.pos (deviation)
        fx.jump_off(
            a.pos,
            ColorIn::Rgb(ink_color_lin(a, surface)),
            surface,
            false,
        );
        self.bump("jump");
    }

    fn _form(&mut self, fx: &mut Fx, m: &Match, i: usize, to_squid: bool, surface: i32) {
        // ≡ L132-138
        let a = &m.actors[i];
        if !a.alive || self.fresh(i) || a.super_jump.is_some() || a.special_active.is_some() {
            return;
        }
        // in own ink the dive / emerge splash covers it ≡ L134
        if surface == 1 {
            return;
        }
        if !self.near(a.pos, 26.0) {
            return;
        }
        fx.form_pop(a.pos, ColorIn::Rgb(team_lin(a.team)), Some(to_squid), false);
        self.bump("form");
    }

    fn _dive(&mut self, fx: &mut Fx, m: &Match, i: usize, pos: Vec3, speed: f32) {
        // ≡ L139-144 — the `a.climbing` branch (L141) is N/A (no climb form)
        let a = &m.actors[i];
        if !a.alive || self.fresh(i) || !self.near(pos, 30.0) {
            return;
        }
        fx.dive(pos, ColorIn::Rgb(team_lin(a.team)), Some(speed));
        self.bump("dive");
    }

    fn _emerge(&mut self, fx: &mut Fx, m: &Match, i: usize, pos: Vec3, speed: f32) {
        // ≡ L145-152 — the climbPop branch (L148) is N/A (no climb form)
        let a = &m.actors[i];
        if !a.alive || self.fresh(i) || a.super_jump.is_some() || !self.near(pos, 30.0) {
            return;
        }
        fx.emerge(pos, ColorIn::Rgb(team_lin(a.team)), Some(speed));
        // the kid comes up wet: a few drips run off for half a second ≡ L150
        self.state(i).emerge_t = self.time;
        self.bump("emerge");
    }

    // `_climb` (L153-164) is not ported: the Rust `Form` enum has no climb
    // state, so both the event and the polling edge can never fire
    // (declared deviation, N/A).

    fn _hit(
        &mut self,
        fx: &mut Fx,
        m: &Match,
        attacker: (usize, usize),
        victim: (usize, usize),
        damage: f32,
    ) {
        // ≡ L172-183 — `killed` is not on HitSend (proxy hits never kill
        // locally; the splat arrives as MatchEvent::Splat) → always false.
        let (Some(ai), Some(vi)) = (resolve(m, attacker), resolve(m, victim)) else {
            return;
        };
        let va = &m.actors[vi];
        if damage <= 0.0 || !va.alive {
            return;
        }
        if self.time - self.st[vi].hit_t < 0.05 || !self.near(va.pos, 40.0) {
            return;
        }
        self.state(vi).hit_t = self.time;
        let aa = &m.actors[ai];
        let mut v = va.pos;
        v.y += if va.form == Form::Squid { 0.3 } else { 0.95 };
        let mut d = va.pos - aa.pos;
        d.y = 0.0;
        if d.length_squared() < 1e-4 {
            d = Vec3::new(0.0, 0.0, 1.0);
        }
        let d = th_normalize(d);
        fx.hit_splash(v, d, ColorIn::Rgb(team_lin(aa.team)), Some(damage), false);
        self.bump("hit");
    }

    fn _splatted(&mut self, fx: &mut Fx, m: &Match, victim: (usize, usize), cause: SplatCause) {
        // ≡ L184-189
        let Some(vi) = resolve(m, victim) else { return };
        let a = &m.actors[vi];
        let col = ColorIn::Rgb(team_lin(a.team));
        if cause == SplatCause::Water {
            fx.water_splash(a.pos, Some(1.1));
            self.bump("seaSplat");
        }
        let wy = fx.water_y;
        fx.ghost(Vec3::new(a.pos.x, a.pos.y.max(wy + 0.2), a.pos.z), col);
        self.bump("ghost");
    }

    fn _respawn(&mut self, _fx: &mut Fx, m: &Match, victim: (usize, usize)) {
        // ≡ L190-195 — `decor.pulse` is not ported (no decor surface).
        let Some(i) = resolve(m, victim) else { return };
        let t = self.time;
        let s = self.state(i);
        s.born = t;
        s.init = true;
        s.full = false;
        s.sj = None;
        s.sp = None;
        self.bump("respawn");
    }

    fn _special_ready(&mut self, fx: &mut Fx, m: &Match, i: usize) {
        // ≡ L196-202
        let a = &m.actors[i];
        if !a.alive || !self.near(a.pos, 40.0) {
            return;
        }
        let col = ColorIn::Rgb(team_lin(a.team));
        // ≡ L199 — sparkle height: JS `a.form === 'squid' ? 0.6 : 1.6`
        // (submersion excluded).
        let hgt = if a.form == Form::Squid { 0.6 } else { 1.6 };
        for _ in 0..10 {
            fx.special_sparkle(a.pos, col, Some(hgt));
        }
        fx.ring(
            Vec3::new(a.pos.x, a.pos.y + 0.03, a.pos.z),
            Some(UP),
            col,
            RingOpts {
                radius: Some(1.3),
                life: Some(0.45),
                style: Some(super::particles::R_THIN),
                alpha: Some(0.8),
                ..Default::default()
            },
        );
        self.bump("specialReady");
    }

    fn _special_use(&mut self, fx: &mut Fx, m: &Match, owner: (usize, usize), id: &str) {
        // ≡ L203-207
        let Some(i) = resolve(m, owner) else { return };
        {
            let s = self.state(i);
            s.sp = Some(id.to_string());
            s.sp_t = 0.0;
        }
        if id == "slam" {
            let a = &m.actors[i];
            fx.slam_launch(a.pos, ColorIn::Rgb(team_lin(a.team)));
            self.bump("slamLaunch");
        }
    }

    fn _slam(&mut self, fx: &mut Fx, m: &Match, owner: (usize, usize), pos: Vec3, radius: f32) {
        // ≡ L208-214
        let Some(i) = resolve(m, owner) else { return };
        {
            let s = self.state(i);
            s.sp = None;
            s.slam_pending = false;
        }
        let col = ColorIn::Rgb(team_lin(m.actors[i].team));
        // JS `radius || SPECIALS.slam.radius` ≡ L212
        let r = if radius > 0.0 { radius } else { SLAM_RADIUS };
        fx.slam_wave(pos, col, Some(r));
        self.bump("slamWave");
    }

    fn _superjump(&mut self, fx: &mut Fx, m: &Match, i: usize, phase: &'static str) {
        // ≡ L215-219
        self.state(i).sj = Some(phase);
        if phase == "flight" {
            let a = &m.actors[i];
            fx.super_jump_launch(a.pos, ColorIn::Rgb(team_lin(a.team)));
            self.bump("sjLaunch");
        }
    }

    fn _sj_land(&mut self, fx: &mut Fx, m: &Match, i: usize, pos: Vec3) {
        // ≡ L220-227 — the spawn-pad `decor.pulse` branch (L224-225) is not
        // ported (no decor surface).
        {
            let t = self.time;
            let s = self.state(i);
            s.sj = None;
            s.sj_land = t;
        }
        let a = &m.actors[i];
        fx.super_jump_land(pos, ColorIn::Rgb(team_lin(a.team)));
        self.bump("sjLand");
    }

    // `_dryFire` (L228-237) is not wired: the `lowink` signal stays inside
    // the sim (MatchEvent has no variant for it) — declared gap.

    #[allow(clippy::too_many_arguments)] // flat arg list mirrors the JS bus payload
    fn _weapon_fire(
        &mut self,
        fx: &mut Fx,
        m: &Match,
        world: &CollisionWorld,
        owner: (usize, usize),
        muzzle: Vec3,
        dir: Vec3,
        charge: Option<f32>,
        len: Option<f32>,
    ) {
        // ≡ L238-246 — `e.weapon` is not on MatchEvent::Fire; the kind comes
        // from the roster (`Match::kinds`).
        let Some(i) = resolve(m, owner) else { return };
        let a = &m.actors[i];
        let col = ColorIn::Rgb(team_lin(a.team));
        let kind = m.kinds.get(i).copied().unwrap_or(WeaponKind::Shooter);
        match kind {
            WeaponKind::Blaster => fx.muzzle(muzzle, dir, col, MuzzleKind::Blaster),
            WeaponKind::Charger => {
                fx.muzzle(muzzle, dir, col, MuzzleKind::Charger);
                // ≡ _beams L617-637: the Rust sim has no `P.beams` list, so
                // the beam trail + impact ride the Fire event (its `len` is
                // the hitscan range, its `charge` the 0..1 fraction).
                if let Some(l) = len {
                    self._beam(fx, world, muzzle, dir, col, charge.unwrap_or(0.0), l);
                }
            }
            WeaponKind::Roller | WeaponKind::Bucket | WeaponKind::Brush => self._flick(fx, m, i),
            _ => {}
        }
        let key = fire_key(kind);
        self.bump(key);
    }

    #[allow(clippy::too_many_arguments)] // flat arg list mirrors the JS `_beams` body
    fn _beam(
        &mut self,
        fx: &mut Fx,
        world: &CollisionWorld,
        from: Vec3,
        dir: Vec3,
        col: ColorIn,
        charge: f32,
        len: f32,
    ) {
        // ≡ L626-637 — JS culls start at 60, end at 40.
        let to = from + dir * len;
        if !self.near(from, 60.0) && !self.near(to, 40.0) {
            return;
        }
        fx.beam_trail(from, to, col, Some(charge));
        // JS L630 muzzle fallback is gated on !seen['weapon:fire'] — here the
        // beam rides the Fire event itself, so it never applies.
        // JS L631-636 impact fallback: `weapon:impact` is never seen → always.
        let probe = to - dir * 0.3;
        let h = world.raycast(probe, dir, 0.7, true);
        if h.hit {
            fx.beam_impact(h.point, h.normal, col, Some(charge));
        } else {
            fx.hit_splash(to, dir, col, Some(40.0 + 60.0 * charge), false);
        }
        self.bump("beam");
    }

    // `_impact` (L247-260) is not wired: no `weapon:impact` MatchEvent
    // (charger beam impact is covered via `_beam`). The `dropHits` log it
    // fed only served the slosh heads, which are N/A (no `p.head`).

    fn _storm_start(&mut self, fx: &mut Fx, pos: Vec3, team: usize) {
        // ≡ L261-265
        fx.storm_start(pos, ColorIn::Rgb(team_lin(team)), Some(STORM_RADIUS));
        self.bump("stormStart");
    }

    fn _flick(&mut self, fx: &mut Fx, m: &Match, i: usize) {
        // ≡ L266-275 — `flickSpreadDeg` is not exposed → the JS default 50.
        let last = self.flick_t.get(&i).copied().unwrap_or(-9.0);
        if self.time - last < 0.25 {
            return;
        }
        self.flick_t.insert(i, self.time);
        let a = &m.actors[i];
        if !self.near(a.pos, 34.0) {
            return;
        }
        let v = Vec3::new(a.pos.x, a.pos.y + 1.05, a.pos.z);
        let dir = Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos());
        fx.flick_curtain(v, dir, ColorIn::Rgb(team_lin(a.team)), Some(50.0));
        self.bump("flick");
    }

    fn _dodge(&mut self, fx: &mut Fx, m: &Match, i: usize, pos: Vec3, dir: Vec3) {
        // ≡ L278-288 (the `weapon:dodge` event has no MatchEvent twin; the
        // runner's dodge edge drives this instead — polling inference).
        let a = &m.actors[i];
        if !a.alive {
            return;
        }
        let mut dd = Vec3::new(dir.x, 0.0, dir.z);
        if dd.length_squared() < 1e-4 {
            dd = Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos());
        }
        let dd = th_normalize(dd);
        self.state(i).dodge_dir = dd;
        if !self.near(pos, 34.0) {
            return;
        }
        fx.dodge_splash(pos, dd, ColorIn::Rgb(team_lin(a.team)));
        self.bump("dodge");
    }
}

/// resolve a `(team, slot)` identity to a roster index.
fn resolve(m: &Match, id: (usize, usize)) -> Option<usize> {
    m.actors
        .iter()
        .position(|a| a.team == id.0 && a.slot == id.1)
}

// ---------------------------------------------------------------------------
// per-frame continuous paths (≡ fxHooks.js L291-709)
// ---------------------------------------------------------------------------

impl HooksRuntime {
    fn _actor(&mut self, fx: &mut Fx, m: &Match, tuning: &Tuning, i: usize, dt: f32) {
        let a = &m.actors[i];
        let col = ColorIn::Rgb(team_lin(a.team));
        // ≡ L310-311
        {
            let t = self.time;
            let s = self.state(i);
            if !a.alive {
                s.alive = false;
                s.step_acc = 0.0;
                s.full = false;
                return;
            }
            if !s.alive {
                s.alive = true;
                s.born = t;
            }
        }
        let pos = a.pos; // _visual(a) ≡ pos (deviation)
        let near = self.near(pos, 36.0);
        let hs = a.vel.x.hypot(a.vel.z);
        let form = actor_form(a);
        let sj_state = a.super_jump.is_some();
        let sp_active = a.special_active.is_some();
        // ≡ L316 first frame
        {
            let t = self.time;
            let s = self.state(i);
            if !s.init {
                s.init = true;
                s.grounded = a.grounded;
                s.vy = a.vel.y;
                s.form = form;
                s.sub = form == FxForm::Swim;
                s.born = t;
                return;
            }
        }
        let (s_grounded, s_vy, s_form, s_sub) = {
            let s = &self.st[i];
            (s.grounded, s.vy, s.form, s.sub)
        };

        // ---- polling fallbacks ≡ L318-338 — the `actor:*` bus never
        // reaches the presentation layer, so these stay permanently on.
        if a.grounded && !s_grounded && s_vy < -3.5 {
            self._land(fx, m, i, a.pos, -s_vy, a.ground_team as i32);
        }
        if !a.grounded && s_grounded && a.vel.y > 4.0 && !sj_state {
            self._jump(fx, m, i, a.ground_team as i32, s_form == FxForm::Swim);
        }
        let sub = form == FxForm::Swim;
        if sub && !s_sub {
            self._dive(fx, m, i, a.pos, hs);
        }
        if !sub && s_sub {
            self._emerge(fx, m, i, a.pos, hs);
        }
        // climb edge ≡ L324: no climb form → N/A.
        if (form == FxForm::Kid) != (s_form == FxForm::Kid) {
            self._form(fx, m, i, form != FxForm::Kid, a.ground_team as i32);
        }
        // footsteps from stride distance ≡ L327-338
        if form == FxForm::Kid && a.grounded && hs > 1.2 && !sj_state && !sp_active && near {
            let mut p: Option<Vec3> = None;
            {
                let s = self.state(i);
                s.step_acc += hs * dt;
                let stride = 0.45 + 0.12 * hs;
                if s.step_acc >= stride {
                    s.step_acc -= stride;
                    s.foot = !s.foot;
                    // JS L334-335 reads the foot bone; no rig → body pos.
                    p = Some(pos);
                }
            }
            if let Some(bp) = p {
                self._footstep(fx, m, i, bp, a.ground_team as i32, Some(hs));
            }
        } else if hs <= 1.2 {
            self.state(i).step_acc = 0.3;
        }

        // ---- continuous effects ≡ L340-443
        if near {
            // climb drips ≡ L343-347: no climb form → N/A.
            // swim carve ≡ L348-364
            if form == FxForm::Swim && hs > 4.5 {
                let yaw = a.vel.x.atan2(a.vel.z);
                if let Some(sy) = self.st[i].swim_yaw {
                    let mut dy = yaw - sy;
                    dy -= (dy / TAU).round() * TAU;
                    let rate = dy / dt;
                    let nt =
                        self.st[i].carve_t + dt * ((rate.abs() - 3.2) / 5.0).clamp(0.0, 1.0) * 22.0;
                    if nt >= 1.0 {
                        self.state(i).carve_t = 0.0;
                        let dir = Vec3::new(a.vel.x / hs, 0.0, a.vel.z / hs);
                        // positive yaw rate turns toward the right vector:
                        // spray flies left ≡ L358
                        let sg = if rate > 0.0 { -1.0 } else { 1.0 };
                        let out = Vec3::new(dir.z * sg, 0.0, -dir.x * sg);
                        let k = (hs / 11.8).clamp(0.0, 1.0);
                        fx.swim_carve(pos, dir, out, col, Some(k));
                    } else {
                        self.state(i).carve_t = nt;
                    }
                }
                self.state(i).swim_yaw = Some(yaw);
            } else {
                let s = self.state(i);
                s.swim_yaw = None;
                s.carve_t = 0.0;
            }
            // just emerged: drips run off the body ≡ L365-374
            if form == FxForm::Kid && self.time - self.st[i].emerge_t < 0.5 {
                let et = self.st[i].emerge_t;
                self.state(i).wet_t += dt * 26.0 * (1.0 - (self.time - et) / 0.5);
                while self.st[i].wet_t >= 1.0 {
                    self.state(i).wet_t -= 1.0;
                    let ang = fx.rand() * TAU;
                    let r = 0.14 + fx.rand() * 0.08;
                    let y = pos.y + 0.35 + fx.rand() * 0.8;
                    let v = Vec3::new(pos.x + ang.cos() * r, y, pos.z + ang.sin() * r);
                    fx.hurt_drip(v, col, Some(0.03));
                }
            } else {
                self.state(i).wet_t = 0.0;
            }
            // idle swimming: bubbles ≡ L375-379
            if form == FxForm::Swim && hs < 2.0 {
                self.state(i).bubble_t += dt;
                if self.st[i].bubble_t > 0.35 {
                    self.state(i).bubble_t = 0.0;
                    fx.bubbles(pos, col, Some(1.0));
                }
            }
            // enemy ink under a kid: sticky sizzle ≡ L380-385
            // (JS L381 prefers `a.onEnemy` when defined — the Rust actor
            // always defines it, so the groundTeam-2 fallback is unused.)
            let on_enemy = a.on_enemy;
            let prev_on_enemy = self.state(i).on_enemy;
            self.state(i).on_enemy = on_enemy;
            // actor:enemyInk ≡ L165-171: the bus handler fires a footstep on
            // the rising edge — polling synthesises that edge here.
            if on_enemy && !prev_on_enemy && a.alive && !self.fresh(i) && self.near(pos, 22.0) {
                fx.footstep(
                    pos,
                    ColorIn::Rgb(team_lin(enemy_team(a.team))),
                    2,
                    Some(heading(a)),
                    Some(4.0),
                );
                self.bump("enemyInk");
            }
            if on_enemy && self.near(pos, 22.0) {
                self.state(i).sizzle_t += dt;
                if self.st[i].sizzle_t > 0.11 {
                    self.state(i).sizzle_t = 0.0;
                    fx.enemy_ink_sizzle(pos, ColorIn::Rgb(team_lin(enemy_team(a.team))));
                }
            }
            // damage: enemy-ink drips off the body ≡ L386-397
            let hurt = 1.0 - a.hp / PLAYER_HP;
            if hurt > 0.08 && !sj_state {
                self.state(i).drip_t += dt * hurt * 7.0;
                while self.st[i].drip_t >= 1.0 {
                    self.state(i).drip_t -= 1.0;
                    let sq = form != FxForm::Kid;
                    let ang = fx.rand() * TAU;
                    let r = if sq { 0.12 } else { 0.16 + fx.rand() * 0.06 };
                    let y = if sq {
                        pos.y + 0.15 + fx.rand() * 0.2
                    } else {
                        pos.y + 0.55 + fx.rand() * 0.75
                    };
                    let v = Vec3::new(pos.x + ang.cos() * r, y, pos.z + ang.sin() * r);
                    fx.hurt_drip(
                        v,
                        ColorIn::Rgb(team_lin(enemy_team(a.team))),
                        Some(0.026 + hurt * 0.02),
                    );
                }
            } else {
                self.state(i).drip_t = 0.0;
            }
            // special gauge full: sparkles ≡ L398-402 (JS `a.specialReady()`
            // ≡ `special >= specialCost`; polling edge for the missing
            // `special:ready` event ≡ L196-202 via `_special_ready`).
            let ready = a.special_cost > 0.0 && a.special >= a.special_cost;
            if ready && !sj_state {
                let rate = if self.local == Some(i) { 12.0 } else { 9.0 };
                self.state(i).spark_t += dt * rate;
                while self.st[i].spark_t >= 1.0 {
                    self.state(i).spark_t -= 1.0;
                    fx.special_sparkle(
                        pos,
                        col,
                        Some(if form == FxForm::Kid { 1.55 } else { 0.55 }),
                    );
                }
            }
            let prev_ready = self.st[i].ready;
            self.state(i).ready = ready;
            if ready && !prev_ready {
                self._special_ready(fx, m, i);
            }
            // weapons ≡ L403-442
            if let Some(wr) = m.runners.get(i) {
                let kind = m.kinds.get(i).copied().unwrap_or(WeaponKind::Shooter);
                // charger + spinner glow ≡ L407-414 (JS getMuzzle → approx)
                if wr.charging_flag()
                    && form == FxForm::Kid
                    && (kind == WeaponKind::Charger || kind == WeaponKind::Spinner)
                {
                    let mz = approx_muzzle(a);
                    let q = wr.charge();
                    fx.charge_glow(mz, col, q);
                    if q >= 0.999 && !self.st[i].full {
                        self.state(i).full = true;
                        fx.charge_full(mz, col);
                        self.bump("chargeFull");
                    }
                } else {
                    self.state(i).full = false;
                }
                // splatling spin-up ≡ L415-426: retired from the Rust
                // 12-weapon roster → N/A.
                // dualies dodge ≡ L427-435 + the push-off splash (the
                // `weapon:dodge` event edge, combined here).
                let dodge_v = wr.dodge_velocity(kind, &tuning.weapons);
                if let Some(dv) = dodge_v {
                    if !self.st[i].dodging {
                        self._dodge(fx, m, i, pos, dv);
                        self.state(i).dodging = true;
                        // JS L429 keeps the event dir; dodgeDir < 0.5 fallback
                        if self.st[i].dodge_dir.length_squared() < 0.5 {
                            let d = Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos());
                            self.state(i).dodge_dir = d;
                        }
                    }
                    // dodge.t/dur are not exposed → the JS fallback k = 0
                    fx.dodge_skid(pos, self.st[i].dodge_dir, col, Some(0.0));
                } else if self.st[i].dodging {
                    self.state(i).dodging = false;
                    fx.dodge_plant(pos, self.st[i].dodge_dir, col);
                    self.bump("dodgePlant");
                }
                // roller / brush spray ≡ L436-441 (brush has no rollWidth →
                // the JS `|| 1.1` / `|| 5` fallbacks)
                if wr.rolling() && a.grounded && hs > 0.8 {
                    let dir = Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos());
                    let v = Vec3::new(pos.x + dir.x * 0.85, pos.y, pos.z + dir.z * 0.85);
                    let (width, speed) = if kind == WeaponKind::Roller {
                        (
                            tuning.weapons.roller.roll_width,
                            tuning.weapons.roller.roll_speed,
                        )
                    } else {
                        (1.1, 5.0)
                    };
                    let k = (hs / speed).clamp(0.0, 1.0);
                    fx.roller_spray(v, dir, width, col, Some(k));
                }
            }
        }

        // ---- specials (drawn at any distance) ≡ L444-454
        if let Some(sa) = &a.special_active {
            if sa.id == "slam" {
                match sa.phase {
                    SpecialPhase::Hang => {
                        let v = Vec3::new(pos.x, pos.y + 0.8, pos.z);
                        let k = (sa.t / SLAM_HANG).clamp(0.0, 1.0);
                        fx.slam_charge(v, col, Some(k));
                    }
                    SpecialPhase::Fall => fx.slam_fall(pos, col),
                    SpecialPhase::Rise => {
                        let v = Vec3::new(pos.x, pos.y + 0.5, pos.z);
                        fx.super_jump_trail(v, a.vel, col);
                    }
                    SpecialPhase::Throw => {}
                }
                if !self.seen("special:slam") {
                    self.state(i).slam_pending = true;
                }
            }
        } else if self.st[i].slam_pending {
            self.state(i).slam_pending = false;
            self._slam(fx, m, (a.team, a.slot), a.pos, SLAM_RADIUS);
        }

        // ---- super jump ≡ L455-470 (home jumps only in the Rust sim →
        // the JS `!sj.home` jumpMarker branches never fire, deviation).
        if let Some(sj) = &a.super_jump {
            let phase: &'static str = if sj.charge { "charge" } else { "flight" };
            if sj.charge {
                let k = (sj.t / 0.75).clamp(0.0, 1.0);
                fx.super_jump_charge(pos, col, Some(k));
            } else {
                fx.super_jump_trail(pos, a.vel, col);
                if sj.to.length_squared() > 0.0 {
                    self.state(i).last_sj_to = sj.to;
                }
            }
            // polling edge ≡ L467: the `superjump` bus never fires → always.
            let cur = self.st[i].sj;
            if cur != Some(phase) {
                if phase == "flight" {
                    self._superjump(fx, m, i, "flight");
                } else {
                    self.state(i).sj = Some(phase);
                }
            }
        } else if self.st[i].sj == Some("flight") {
            // polling edge ≡ L468: `superjump:land` never fires → always.
            self._sj_land(fx, m, i, a.pos);
        }

        // ≡ L471
        let s = self.state(i);
        s.grounded = a.grounded;
        s.vy = a.vel.y;
        s.form = form;
        s.sub = sub;
    }

    fn _projectiles(&mut self, fx: &mut Fx, m: &Match, world: &CollisionWorld, dt: f32) {
        // ≡ L475-535 — the slosh-head / dropHits block (L496-500, L514-534)
        // is N/A: `Projectile` has no `head` flag (deviation).
        let list = &m.projectiles.list;
        let fire_fallback = !self.seen("weapon:fire");
        let wy = fx.water_y;
        let mut budget = 48i32;
        self.proj_recs.resize(list.len(), ProjFxRec::default());
        for (i, p) in list.iter().enumerate() {
            // JS `if (!p.owner) continue` ≡ L483 — resolve the identity slot.
            if resolve(m, (p.team, p.owner_slot)).is_none() {
                continue;
            }
            // ProjFxRec is `Copy` — work on a local so `self.near` /
            // `self.bump` can borrow `self` mutably mid-loop.
            let mut rec = self.proj_recs[i];
            if p.age < rec.last_age {
                // new (or recycled) projectile ≡ L484-491
                rec.fx_d = 0.0;
                rec.fx_y = p.pos.y;
                if fire_fallback && p.age < 0.1 && p.blast && self.near(p.pos, 34.0) {
                    let d = th_normalize(p.vel);
                    fx.muzzle(
                        p.start,
                        d,
                        ColorIn::Rgb(team_lin(p.team)),
                        MuzzleKind::Blaster,
                    );
                    self.bump("fire:blaster");
                    // `p.type === 'drop'` has no Rust field → the flick
                    // fallback (L489, same `p.age < 0.1` guard) is not
                    // driven (gap).
                }
            }
            rec.last_age = p.age;
            let sp = p.vel.length();
            rec.fx_d += sp * dt;
            // JS `p.type === 'slosh'` (weapons.js L1322): bucket throws carry
            // `belowFalloff` (config 0.12); blade droplets share `volley` but
            // never set it — the AND keeps blade drips on the plain-drop path.
            let slosh = p.volley.is_some() && p.below_falloff > 0.0;
            // JS step ≡ L501 (drop 1.1 / head 0.45 branches unreachable)
            let step = if p.blast {
                0.55
            } else if slosh {
                1.0
            } else {
                0.8
            };
            if rec.fx_d >= step {
                rec.fx_d = 0.0;
                if budget > 0 && p.age > 0.03 && p.age > p.delay && self.near(p.pos, 30.0) {
                    budget -= 1;
                    let col = ColorIn::Rgb(team_lin(p.team));
                    if slosh {
                        fx.slosh_trail(p.pos, p.vel, col, false);
                    } else {
                        fx.shot_trail(p.pos, p.vel, col, p.blast);
                    }
                }
            }
            // plopping into open water ≡ L509-512
            if rec.fx_y >= wy + 0.05
                && p.pos.y < wy + 0.05
                && world.ground_height(p.pos.x, p.pos.z, p.pos.y + 3.0, false) == f32::NEG_INFINITY
                && self.near(p.pos, 45.0)
            {
                fx.water_plop(p.pos, Some(if p.blast { 0.6 } else { 0.3 }));
                self.bump("plop");
            }
            rec.fx_y = p.pos.y;
            self.proj_recs[i] = rec;
        }
    }

    fn _bombs(&mut self, fx: &mut Fx, m: &Match, world: &CollisionWorld) {
        // ≡ L538-569 — `Bomb` has no `beepT`, so the beep pulse (L564) is
        // not driven (gap).
        let bombs = &m.projectiles.bombs;
        self.bomb_recs.resize(bombs.len(), BombFxRec::default());
        for (i, b) in bombs.iter().enumerate() {
            // BombFxRec is `Copy` — local-copy borrow pattern (see `_projectiles`).
            let mut rec = self.bomb_recs[i];
            if b.age < rec.last_age {
                // recycled index → fresh bomb (JS keyed by object identity)
                rec = BombFxRec {
                    vy: b.vel.y,
                    ..BombFxRec::default()
                };
            }
            rec.last_age = b.age;
            let col = ColorIn::Rgb(team_lin(b.team));
            let near = self.near(b.pos, 40.0);
            if b.fuse < 0.0 {
                if near {
                    fx.bomb_trail(b.pos, b.vel, col);
                }
                if rec.vy < -1.5 && b.vel.y > 0.3 && near {
                    fx.bounce_splash(Vec3::new(b.pos.x, b.pos.y - 0.2, b.pos.z), UP, col);
                    self.bump("bombBounce");
                }
            } else if b.kind == "bomb" {
                if !rec.armed {
                    rec.armed = true;
                    let from = Vec3::new(b.pos.x, b.pos.y + 0.3, b.pos.z);
                    let h = world.raycast(from, Vec3::NEG_Y, 2.5, true);
                    if h.hit {
                        rec.gp = h.point;
                        rec.gn = h.normal;
                        rec.ground = true;
                    } else {
                        rec.gp = Vec3::new(b.pos.x, b.pos.y - 0.2, b.pos.z);
                    }
                    fx.bounce_splash(rec.gp, rec.gn, col);
                    self.bump("bombArm");
                }
                let k = (1.0 - b.fuse / SUB_BOMB_FUSE).clamp(0.0, 1.0);
                fx.danger_ring(
                    rec.gp + rec.gn * 0.02,
                    rec.gn,
                    col,
                    Some(SUB_BOMB_RADIUS),
                    Some(k),
                );
            }
            rec.vy = b.vel.y;
            self.bomb_recs[i] = rec;
        }
    }

    fn _clouds(&mut self, fx: &mut Fx, m: &Match, world: &CollisionWorld, dt: f32) {
        // ≡ L571-603 — `c.group.position` ≡ `WorldObject::pos`; the visual
        // `group.scale.x` is not ported → 1.0 (rain-sheet radius ≡ R).
        let r = STORM_RADIUS;
        for w in m.specials.world.iter() {
            if w.kind != "cloud" || w.dead {
                continue;
            }
            let pos = w.pos;
            let col = ColorIn::Rgb(team_lin(w.team));
            if !self.clouds.contains_key(&w.gid) {
                // new cloud ≡ L582-586
                if !self.seen("storm:start") {
                    self._storm_start(fx, pos, w.team);
                }
                self.clouds.insert(
                    w.gid,
                    CloudFxRec {
                        stamp: self.stamp,
                        puddle_t: 0.0,
                        flash_t: 0.5,
                    },
                );
            }
            let near = self.near(pos, 45.0);
            let rec = self.clouds.entry(w.gid).or_insert(CloudFxRec {
                stamp: self.stamp,
                puddle_t: 0.0,
                flash_t: 0.5,
            });
            rec.stamp = self.stamp;
            if !near {
                continue;
            }
            let (ct, cdur) = match &w.body {
                ObjectBody::Cloud(c) => (c.t, c.dur),
                _ => (0.0, 0.0),
            };
            let fade = if cdur > 0.0 {
                ((cdur - ct) / 0.6).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if ct > 0.35 && fade > 0.3 {
                fx.rain_sheet(pos, r, col, Some(dt));
            }
            rec.puddle_t += dt * 16.0 * fade;
            while rec.puddle_t >= 1.0 {
                rec.puddle_t -= 1.0;
                let a = fx.rand() * TAU;
                let rr = fx.rand().sqrt() * r * 0.95;
                let v = Vec3::new(pos.x + a.cos() * rr, pos.y - 0.6, pos.z + a.sin() * rr);
                let h = world.raycast(v, Vec3::NEG_Y, 14.0, true);
                if h.hit {
                    fx.storm_puddle(h.point, Some(h.normal), col);
                }
            }
            rec.flash_t -= dt;
            if rec.flash_t <= 0.0 {
                rec.flash_t = 0.5 + fx.rand() * 1.2;
                fx.storm_flash(pos, col, Some(r));
            }
        }
        self.clouds.retain(|_, rec| rec.stamp == self.stamp);
    }

    // `_beams` (L605-639): the laser-sight dot (L608-615) rides `P.sights`,
    // which the Rust sim does not expose → not driven (gap). The beam trail +
    // impact half is driven from `MatchEvent::Fire` via `_beam`.

    fn _ambient(&mut self, fx: &mut Fx, layout: &StageLayout, dt: f32) {
        // ≡ L642-679 — `G.env.seaState` is not ported → constant 1.0.
        let (Some(cam), Some(dir)) = (self.cam, self.cam_dir) else {
            return; // JS L644: no camera → no ambient life
        };
        if self.edge_key != Some(bounds_key(layout)) {
            self.build_edges(layout);
        }
        let sea = 1.0f32;
        self.spray_t -= dt;
        if sea < 0.3 {
            self.spray_t = self.spray_t.max(0.3);
        } else if self.spray_t <= 0.0 && !self.edges.is_empty() {
            self.spray_t = (0.3 + fx.rand() * 0.5) / sea;
            let mut best: Option<(f32, usize)> = None;
            for _ in 0..14 {
                let ei = (fx.rand() * self.edges.len() as f32) as usize;
                let e = self.edges[ei];
                let dx = e.x - cam.x;
                let dz = e.z - cam.z;
                let d = dx.hypot(dz);
                if d > 48.0 || d < 2.0 {
                    continue;
                }
                let facing = (dx * dir.x + dz * dir.z) / d;
                if facing < 0.35 {
                    continue;
                }
                let score = facing * (1.0 - d / 60.0) + fx.rand() * 0.3;
                if best.is_none_or(|(bs, _)| score > bs) {
                    best = Some((score, ei));
                }
            }
            if let Some((_, ei)) = best {
                let e = self.edges[ei];
                let v = Vec3::new(e.x, fx.water_y + 0.05, e.z);
                let n = Vec3::new(e.nx, 0.0, e.nz);
                let roll = fx.rand();
                let strength = (if roll < 0.18 {
                    1.3 + fx.rand() * 0.3
                } else {
                    0.55 + fx.rand() * 0.55
                }) * sea.min(1.0);
                fx.sea_spray(v, n, Some(strength));
                self.bump("seaSpray");
            }
        }
        // a gull feather now and then, drifting down through the view ≡ L670-678
        self.feather_t -= dt;
        if self.feather_t <= 0.0 {
            self.feather_t = 7.0 + fx.rand() * 9.0;
            let x = cam.x + dir.x * (6.0 + fx.rand() * 8.0) + (fx.rand() - 0.5) * 8.0;
            let y = cam.y + 5.0 + fx.rand() * 4.0;
            let z = cam.z + dir.z * (6.0 + fx.rand() * 8.0) + (fx.rand() - 0.5) * 8.0;
            fx.feather(Vec3::new(x, y, z));
            self.bump("feather");
        }
    }

    /// `_buildEdges` ≡ L682-709 — the Rust port only has the aligned stage
    /// AABB (the `env.deckEdges` / rotated-`footprint` branches are Cargo
    /// Terminal / env surfaces that are not ported).
    fn build_edges(&mut self, layout: &StageLayout) {
        self.edge_key = Some(bounds_key(layout));
        self.edges.clear();
        let b = &layout.stage.bounds;
        let inside = |x: f32, z: f32| x > b.min_x && x < b.max_x && z > b.min_z && z < b.max_z;
        let sides: [(f32, f32, f32, f32, f32, f32); 4] = [
            (b.min_x, b.min_z, b.max_x, b.min_z, 0.0, -1.0),
            (b.max_x, b.min_z, b.max_x, b.max_z, 1.0, 0.0),
            (b.max_x, b.max_z, b.min_x, b.max_z, 0.0, 1.0),
            (b.min_x, b.max_z, b.min_x, b.min_z, -1.0, 0.0),
        ];
        for (x0, z0, x1, z1, nx, nz) in sides {
            let len = (x1 - x0).hypot(z1 - z0);
            let n = (len / 1.6).floor().max(1.0) as usize;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let x = x0 + (x1 - x0) * t;
                let z = z0 + (z1 - z0) * t;
                if inside(x + nx * 0.6, z + nz * 0.6) {
                    continue;
                }
                self.edges.push(Edge {
                    x: x + nx * 0.05,
                    z: z + nz * 0.05,
                    nx,
                    nz,
                });
            }
        }
    }

    /// the per-frame driver ≡ `FxHooks.update` (L291-305).
    fn update(
        &mut self,
        fx: &mut Fx,
        m: &Match,
        world: &CollisionWorld,
        layout: &StageLayout,
        tuning: &Tuning,
        dt: f32,
    ) {
        // JS `!(dt > 0)` also rejects NaN — keep the negated form.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !self.enabled || !(dt > 0.0) {
            return; // ≡ L293
        }
        self.time += dt;
        self.stamp += 1;
        // ≡ L295 `paint.setView` — the paint renderer reads the camera
        // directly (world.rs), no set-view state to mirror.
        // discrete moments: consume (without draining) this frame's
        // presentation events ≡ the L57-79 subscription table
        for e in m.events.iter() {
            self.on_event(fx, m, world, e);
        }
        for i in 0..m.actors.len() {
            self._actor(fx, m, tuning, i, dt);
        }
        self._projectiles(fx, m, world, dt);
        self._bombs(fx, m, world);
        self._clouds(fx, m, world, dt);
        self._ambient(fx, layout, dt);
    }

    fn on_event(&mut self, fx: &mut Fx, m: &Match, world: &CollisionWorld, e: &MatchEvent) {
        match e {
            MatchEvent::Fire {
                owner,
                muzzle,
                dir,
                charge,
                len,
            } => {
                self.mark_seen("weapon:fire");
                self._weapon_fire(fx, m, world, *owner, *muzzle, *dir, *charge, *len);
            }
            MatchEvent::Splat {
                victim_team,
                victim_slot,
                cause,
                ..
            } => {
                self.mark_seen("splatted");
                self._splatted(fx, m, (*victim_team, *victim_slot), *cause);
            }
            MatchEvent::Respawn { team, slot } => {
                self.mark_seen("respawn");
                self._respawn(fx, m, (*team, *slot));
            }
            MatchEvent::HitSend {
                attacker,
                victim,
                damage,
            } => {
                self.mark_seen("hit");
                self._hit(fx, m, *attacker, *victim, *damage);
            }
            MatchEvent::SpecialUse { owner, id } => {
                self.mark_seen("special:use");
                self._special_use(fx, m, *owner, id);
            }
            MatchEvent::SpecialSlam { owner, pos, radius } => {
                self.mark_seen("special:slam");
                self._slam(fx, m, *owner, *pos, *radius);
            }
            _ => {}
        }
    }
}

fn bounds_key(layout: &StageLayout) -> (f32, f32, f32, f32) {
    let b = &layout.stage.bounds;
    (b.min_x, b.min_z, b.max_x, b.max_z)
}

/// The pure per-frame entry point (unit-testable without Bevy): drives the
/// whole `FxHooks.update` surface against the read-only sim state.
#[allow(clippy::too_many_arguments)]
pub fn hooks_frame(
    h: &mut HooksRuntime,
    fx: &mut Fx,
    m: &Match,
    world: &CollisionWorld,
    layout: &StageLayout,
    tuning: &Tuning,
    cam: Option<Vec3>,
    cam_dir: Option<Vec3>,
    local: Option<usize>,
    dt: f32,
) {
    h.cam = cam;
    h.cam_dir = cam_dir;
    h.local = local;
    h.update(fx, m, world, layout, tuning, dt);
}

// ---------------------------------------------------------------------------
// the Bevy system
// ---------------------------------------------------------------------------

use bevy::prelude::{Camera, GlobalTransform, Query, Res, ResMut, Time, With};

use super::FxState;
use crate::ink_render::DemoSim;
use crate::input::PlayerControls;

/// ≡ the JS `hooks.update(dt)` call in the main loop (only on unpaused
/// frames). Runs *before* `ui_state_machine` (which drains the events) and
/// consumes `Match::events` with `.iter()` only.
pub fn fx_hooks(
    time: Res<Time>,
    demo: Res<DemoSim>,
    pc: Res<PlayerControls>,
    cams: Query<&GlobalTransform, With<Camera>>,
    mut fs: ResMut<FxState>,
) {
    if pc.paused {
        return;
    }
    let (cam, cam_dir) = match cams.single().ok() {
        Some(g) => {
            let (_s, r, t) = g.to_scale_rotation_translation();
            (
                Some(glam::Vec3::from_array(t.to_array())),
                Some(glam::Vec3::from_array(
                    (r * bevy::math::Vec3::NEG_Z).to_array(),
                )),
            )
        }
        None => (None, None),
    };
    let FxState { fx, hooks } = &mut *fs;
    hooks_frame(
        hooks,
        fx,
        &demo.m,
        &demo.world,
        &demo.layout,
        &demo.tuning,
        cam,
        cam_dir,
        Some(demo.player_slot),
        time.delta_secs(),
    );
}

// ---------------------------------------------------------------------------
// tests (TR-51.1: hook面全事件覆盖 + 每事件 count 逐项 + 池增删)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink_render::DemoSim;

    fn demo() -> DemoSim {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        DemoSim::new(layout, world, 20261004)
    }

    struct Harness {
        d: DemoSim,
        fx: Fx,
        h: HooksRuntime,
    }

    impl Harness {
        fn new() -> Self {
            let d = demo();
            let mut fx = Fx::with_seed(Default::default(), 0x5eed);
            let mut h = HooksRuntime::new();
            h.bootstrap(&mut fx);
            Self { d, fx, h }
        }

        fn frame(&mut self, dt: f32) {
            let cam = Some(Vec3::new(0.0, 6.0, 14.0));
            let dir = Some(Vec3::new(0.0, -0.3, -1.0).normalize());
            let local = Some(self.d.player_slot);
            let DemoSim {
                m,
                world,
                layout,
                tuning,
                ..
            } = &mut self.d;
            hooks_frame(
                &mut self.h,
                &mut self.fx,
                m,
                world,
                layout,
                tuning,
                cam,
                dir,
                local,
                dt,
            );
            m.events.clear();
        }

        fn actor(&self, i: usize) -> &inkwave_sim::actor::Actor {
            &self.d.m.actors[i]
        }
    }

    #[test]
    fn splat_event_draws_ghost_and_counts() {
        let mut x = Harness::new();
        let a = x.actor(0);
        let id = (a.team, a.slot);
        x.d.m.events.push(MatchEvent::Splat {
            victim_team: id.0,
            victim_slot: id.1,
            attacker: None,
            cause: SplatCause::Weapon,
        });
        let before = x.fx.stats().puffs;
        x.frame(0.016);
        assert_eq!(x.h.count_of("ghost"), 1);
        assert_eq!(x.h.count_of("seaSplat"), 0);
        assert!(x.fx.stats().puffs > before, "ghost puff entered the pool");
        assert!(x.h.seen("splatted"));
    }

    #[test]
    fn water_splat_adds_sea_splash() {
        let mut x = Harness::new();
        let a = x.actor(0);
        x.d.m.events.push(MatchEvent::Splat {
            victim_team: a.team,
            victim_slot: a.slot,
            attacker: None,
            cause: SplatCause::Water,
        });
        x.frame(0.016);
        assert_eq!(x.h.count_of("seaSplat"), 1);
        assert_eq!(x.h.count_of("ghost"), 1);
    }

    #[test]
    fn respawn_resets_state() {
        let mut x = Harness::new();
        x.frame(0.1);
        {
            let a = x.actor(0);
            let id = (a.team, a.slot);
            x.d.m.events.push(MatchEvent::Respawn {
                team: id.0,
                slot: id.1,
            });
        }
        x.frame(0.1);
        assert_eq!(x.h.count_of("respawn"), 1);
        let i = resolve(&x.d.m, (x.actor(0).team, x.actor(0).slot)).unwrap();
        assert!(x.h.st[i].init);
        assert!((x.h.st[i].born - x.h.time).abs() < 1e-6);
    }

    #[test]
    fn special_use_and_slam_events() {
        let mut x = Harness::new();
        let a = x.actor(0);
        let id = (a.team, a.slot);
        x.d.m.events.push(MatchEvent::SpecialUse {
            owner: id,
            id: "slam".to_string(),
        });
        x.frame(0.016);
        assert_eq!(x.h.count_of("slamLaunch"), 1);
        let i = resolve(&x.d.m, id).unwrap();
        assert_eq!(x.h.st[i].sp.as_deref(), Some("slam"));
        x.d.m.events.push(MatchEvent::SpecialSlam {
            owner: id,
            pos: x.actor(i).pos,
            radius: 5.2,
        });
        x.frame(0.016);
        assert_eq!(x.h.count_of("slamWave"), 1);
        assert_eq!(x.h.st[i].sp, None);
        assert!(x.h.seen("special:slam"));
    }

    #[test]
    fn slam_polling_fallback_fires_when_event_never_arrives() {
        // JS L444-454: with no `special:slam` on the bus the pending flag
        // drives the wave when the active ends. Seed the pending flag the
        // way the polling path would, then run a frame without an active.
        let mut x = Harness::new();
        x.frame(0.016);
        x.h.state(0).slam_pending = true;
        assert!(x.d.m.actors[0].special_active.is_none());
        x.frame(0.016);
        assert_eq!(x.h.count_of("slamWave"), 1);
        assert!(!x.h.st[0].slam_pending);
    }

    #[test]
    fn hit_send_drives_hit_splash() {
        let mut x = Harness::new();
        x.frame(0.016);
        // park both actors inside the `_near(40)` window of the harness cam
        x.d.m.actors[1].pos = Vec3::new(0.0, 1.0, 0.0);
        x.d.m.actors[2].pos = Vec3::new(2.0, 1.0, 0.0);
        let v = x.actor(1);
        let a = x.actor(2);
        let (vid, aid) = ((v.team, v.slot), (a.team, a.slot));
        x.d.m.events.push(MatchEvent::HitSend {
            attacker: aid,
            victim: vid,
            damage: 35.0,
        });
        x.d.m.actors[1].hp = 65.0;
        x.frame(0.016);
        assert_eq!(x.h.count_of("hit"), 1);
        assert!(x.h.seen("hit"));
    }

    #[test]
    fn fire_event_muzzles_and_disables_projectile_fallback() {
        let mut x = Harness::new();
        let a = x.actor(0);
        let id = (a.team, a.slot);
        x.d.m.events.push(MatchEvent::Fire {
            owner: id,
            muzzle: a.pos + Vec3::Y,
            dir: Vec3::new(0.0, 0.0, -1.0),
            charge: None,
            len: None,
        });
        x.frame(0.016);
        assert_eq!(
            x.h.count_of("fire:shooter")
                + x.h.count_of("fire:blaster")
                + x.h.count_of("fire:other"),
            1
        );
        assert!(x.h.seen("weapon:fire"));
    }

    #[test]
    fn charger_fire_drives_beam_trail() {
        let mut x = Harness::new();
        x.d.m.actors[0].pos = Vec3::new(0.0, 1.0, 0.0);
        // `beam_trail` spawns glow sprites (api.rs L1636-1666, ≡ JS L1747+),
        // not `beams`-pool columns.
        let before = x.fx.stats().glows;
        let a = x.actor(0);
        let id = (a.team, a.slot);
        x.d.m.events.push(MatchEvent::Fire {
            owner: id,
            muzzle: a.pos + Vec3::Y,
            dir: Vec3::new(0.0, 0.0, -1.0),
            charge: Some(0.5),
            len: Some(30.0),
        });
        // force the charger branch regardless of the roster kind of slot 0
        let i = resolve(&x.d.m, id).unwrap();
        x.d.m.kinds[i] = WeaponKind::Charger;
        x.frame(0.016);
        assert_eq!(x.h.count_of("fire:charger"), 1);
        assert_eq!(x.h.count_of("beam"), 1);
        assert!(
            x.fx.stats().glows > before,
            "beam trail entered the glow pool"
        );
    }

    #[test]
    fn polling_drives_land_jump_and_footsteps() {
        let mut x = Harness::new();
        // frame 1: init snapshot
        x.frame(0.016);
        let i = 0usize;
        {
            let a = &mut x.d.m.actors[i];
            a.grounded = false;
            a.vel = Vec3::new(0.0, -8.0, 0.0);
            a.pos = Vec3::new(0.0, 2.0, 0.0);
            a.alive = true;
        }
        x.frame(0.016); // snapshot vy=-8, airborne
        {
            let a = &mut x.d.m.actors[i];
            a.grounded = true;
            a.vel = Vec3::new(5.0, 0.0, 0.0); // walking on ink afterwards
            a.ground_team = 1;
        }
        x.frame(0.016); // land edge (vy -8 < -3.5)
        assert_eq!(x.h.count_of("land"), 1);
        // stride footsteps: hs=5 → stride 1.05, acc += 0.08/frame
        for _ in 0..20 {
            x.frame(0.016);
        }
        assert!(x.h.count_of("footstep") >= 1, "stride footsteps fired");
        // jump edge
        {
            let a = &mut x.d.m.actors[i];
            a.grounded = false;
            a.vel = Vec3::new(5.0, 6.0, 0.0);
        }
        x.frame(0.016);
        assert_eq!(x.h.count_of("jump"), 1);
    }

    #[test]
    fn form_pop_edge_on_kid_squid_transition() {
        let mut x = Harness::new();
        x.d.m.actors[0].pos = Vec3::new(0.0, 1.0, 0.0);
        x.frame(0.016); // init
        // `_form` guards on `_fresh` (JS L133: birth window 0.35 s)
        for _ in 0..30 {
            x.frame(0.016);
        }
        {
            let a = &mut x.d.m.actors[0];
            a.form = Form::Squid;
            a.ground_team = 0; // not own ink → _form proceeds
        }
        x.frame(0.016);
        assert_eq!(x.h.count_of("form"), 1);
    }

    #[test]
    fn dive_emerge_cycle_via_submerged() {
        let mut x = Harness::new();
        x.d.m.actors[0].pos = Vec3::new(0.0, 1.0, 0.0);
        x.frame(0.016); // init as kid
        // dive/emerge guard on `_fresh` too (JS L140/L146)
        for _ in 0..30 {
            x.frame(0.016);
        }
        {
            let a = &mut x.d.m.actors[0];
            a.form = Form::Squid;
            a.submerged = true;
        }
        x.frame(0.016); // dive edge
        assert_eq!(x.h.count_of("dive"), 1);
        {
            let a = &mut x.d.m.actors[0];
            a.submerged = false;
        }
        x.frame(0.016); // emerge edge
        assert_eq!(x.h.count_of("emerge"), 1);
        assert!(x.h.st[0].emerge_t > 0.0, "wet timer armed (JS L150)");
    }

    #[test]
    fn superjump_polling_launch_and_land() {
        let mut x = Harness::new();
        x.d.m.actors[0].pos = Vec3::new(0.0, 1.0, 0.0);
        x.frame(0.016);
        {
            let a = &mut x.d.m.actors[0];
            a.super_jump = Some(inkwave_sim::actor::SuperJump {
                charge: true,
                t: 0.2,
                from: a.pos,
                to: a.pos + Vec3::new(0.0, 4.0, -6.0),
                dur: 1.0,
            });
        }
        x.frame(0.016); // charge phase seen
        assert_eq!(x.h.st[0].sj, Some("charge"));
        {
            let a = &mut x.d.m.actors[0];
            if let Some(sj) = &mut a.super_jump {
                sj.charge = false;
            }
        }
        x.frame(0.016);
        assert_eq!(x.h.count_of("sjLaunch"), 1);
        {
            x.d.m.actors[0].super_jump = None;
        }
        x.frame(0.016);
        assert_eq!(x.h.count_of("sjLand"), 1);
    }

    #[test]
    fn ambient_feather_and_spray_fire_with_camera() {
        let mut x = Harness::new();
        x.h.feather_t = 0.01;
        x.h.spray_t = 0.0;
        x.frame(0.016);
        assert_eq!(x.h.count_of("feather"), 1);
        assert!(
            x.h.count_of("seaSpray") + x.h.count_of("feather") >= 1,
            "ambient life runs while a camera exists"
        );
        assert!(
            !x.h.edges.is_empty(),
            "deck edges built from the stage AABB"
        );
    }

    #[test]
    fn stats_shape_matches_js_l711() {
        let mut x = Harness::new();
        x.frame(0.016);
        let s = x.h.stats();
        assert!(s.seen.is_sorted());
        for v in s.count.values() {
            assert!(*v >= 1);
        }
    }

    #[test]
    fn pools_shrink_as_particles_age_out() {
        // TR-51.1 pool 增删: spawn a burst, run the data core, entries retire.
        let mut fx = Fx::with_seed(Default::default(), 1);
        fx.burst(
            Vec3::new(0.0, 2.0, 0.0),
            Some(UP),
            ColorIn::Rgb(team_lin(0)),
            Default::default(),
        );
        let n = fx.stats().drops;
        assert!(n > 0, "burst produced droplets");
        for _ in 0..600 {
            fx.update(0.016, None);
        }
        assert_eq!(fx.stats().drops, 0, "droplet pool drained after lifetimes");
    }
}
