//! M5 Task 64 — the screen-FX composite overlay DATA PLANE, ported from
//! `src/fx/screenfx.js`, `src/ui/hud-boss.js`, `src/ui/hud.js` and
//! `src/game/minimap.js`.
//!
//! All scalar parameters come from the pinned `assets/screenfx.json`
//! (extracted from the READ-ONLY JS tree); `tools/screenfx-crosscheck.mjs`
//! independently re-parses those constants. This module owns the headless
//! state machines for the composite pass enable gate, boss damage numbers and
//! minimap transient FX, plus a unified DOM-overlay data resource that the
//! Task 62 lobby nameplates feed into.
//!
//! D-T64-1: upstream has no heal screen overlay (healing is game logic only),
//! so no fake heal effect is added. D-T64-2: the GPU composite pass and real
//! DOM node rendering cannot be validated in the sandbox — this module only
//! exposes parameter/state data; the visual result is a user-machine review
//! item (REVIEW_CHECKLIST). D-T64-3: `low` tier keeps `post_enabled=false` and
//! an empty overlay queue (all overlays off), preserving the swiftshader path.
//! D-T64-4: transient FX producers are wired for `special:slam`, `respawn` and
//! `HitSend` (damage numbers); the super-jump `jump`/`land` FX helpers and the
//! `boom` FX are kept as data-plane primitives (no MatchEvent carries
//! super-jump flight state or `bomb:explode`), with `land_jump()` not filtered
//! per actor (JS keys on `f.actor`). Their lifecycle state machines are covered
//! by unit tests rather than live event producers.

use std::sync::OnceLock;

use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use inkwave_sim::net::wire::{JVal, parse_json};

const DOC: &str = include_str!("../../../../assets/screenfx.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("screenfx.json parses"))
}

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("screenfx.json: missing field {key}"))
}

fn num(v: &JVal, key: &str) -> f64 {
    field(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("screenfx.json: missing numeric field {key}"))
}

fn int(v: &JVal, key: &str) -> usize {
    num(v, key) as usize
}

fn array<'a>(v: &'a JVal, key: &str) -> &'a [JVal] {
    field(v, key)
        .as_array()
        .unwrap_or_else(|| panic!("screenfx.json: missing array field {key}"))
}

/// Composite material / pass placement constants (all pinned from JS).
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeSpec {
    pub material_name: String,
    pub taps: usize,
    pub pass_order: Vec<String>,
    pub pass_thresholds: Vec<f64>,
    pub res: [u32; 2],
    pub aspect: f64,
    pub heart: [f64; 4],
    pub hurt: f64,
    pub charge: [f64; 4],
    pub sat: f64,
    pub kill: [f64; 4],
}

fn f64arr(v: &JVal, key: &str) -> Vec<f64> {
    array(v, key)
        .iter()
        .map(|x| {
            x.as_f64()
                .unwrap_or_else(|| panic!("screenfx.json: {key} element"))
        })
        .collect()
}

impl CompositeSpec {
    fn load() -> Self {
        let c = field(doc(), "composite");
        let u = field(c, "uniform_defaults");
        let res = f64arr(u, "res");
        let heart = f64arr(u, "heart");
        let charge = f64arr(u, "charge");
        let kill = f64arr(u, "kill");
        Self {
            material_name: field(c, "material_name")
                .as_str()
                .unwrap_or("InkwaveScreenFX")
                .to_string(),
            taps: int(c, "taps"),
            pass_order: array(c, "pass_order")
                .iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect(),
            pass_thresholds: f64arr(c, "pass_thresholds"),
            res: [res[0] as u32, res[1] as u32],
            aspect: num(u, "aspect"),
            heart: [heart[0], heart[1], heart[2], heart[3]],
            hurt: num(u, "hurt"),
            charge: [charge[0], charge[1], charge[2], charge[3]],
            sat: num(u, "sat"),
            kill: [kill[0], kill[1], kill[2], kill[3]],
        }
    }
}

/// Per-term visibility thresholds ≡ screenfx.js L862-865 `any` expression.
pub fn pass_term(name: &str) -> f64 {
    let terms = field(field(doc(), "composite"), "pass_terms");
    num(terms, name)
}

/// Decay / heartbeat constants (screenfx.js L870-879 / L953-963 / L599).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecaySpec {
    pub rate: f64,
    pub low_k_denom: f64,
    pub bpm_base: f64,
    pub bpm_scale: f64,
    pub hurt_damp: f64,
    pub heart_damp: f64,
    pub damage_window: f64,
}

pub fn decay() -> DecaySpec {
    let d = field(doc(), "decay");
    DecaySpec {
        rate: num(d, "rate"),
        low_k_denom: num(d, "low_k_denom"),
        bpm_base: num(d, "bpm_base"),
        bpm_scale: num(d, "bpm_scale"),
        hurt_damp: num(d, "hurt_damp"),
        heart_damp: num(d, "heart_damp"),
        damage_window: num(d, "damage_window"),
    }
}

/// Boss damage-number pool spec (hud-boss.js L402-440).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageNumberSpec {
    pub pool_cap: usize,
    pub combo_window: f64,
    pub life: f64,
    pub out_after: f64,
    pub weak_scale_bonus: f64,
    pub crit_ms: u64,
}

pub fn damage_number_spec() -> DamageNumberSpec {
    let d = field(doc(), "damage_number");
    DamageNumberSpec {
        pool_cap: int(d, "pool_cap"),
        combo_window: num(d, "combo_window"),
        life: num(d, "life"),
        out_after: num(d, "out_after"),
        weak_scale_bonus: num(d, "weak_scale_bonus"),
        crit_ms: num(d, "crit_ms") as u64,
    }
}

/// Minimap dot / death-mark / transient-FX spec (hud.js + minimap.js).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapSpec {
    pub dot_count: usize,
    pub death_mark_count: usize,
    pub fx_cap: usize,
    pub slam_life: f64,
    pub boom_life: f64,
    pub jump_life: f64,
    pub land_life: f64,
    pub spawn_life: f64,
}

pub fn minimap_spec() -> MinimapSpec {
    let m = field(doc(), "minimap");
    let fx = field(m, "fx_life");
    MinimapSpec {
        dot_count: int(m, "dot_count"),
        death_mark_count: int(m, "death_mark_count"),
        fx_cap: int(m, "fx_cap"),
        slam_life: num(fx, "slam"),
        boom_life: num(fx, "boom"),
        jump_life: num(fx, "jump"),
        land_life: num(fx, "land"),
        spawn_life: num(fx, "spawn"),
    }
}

pub fn composite() -> &'static CompositeSpec {
    static VAL: OnceLock<CompositeSpec> = OnceLock::new();
    VAL.get_or_init(CompositeSpec::load)
}

pub fn source_commit() -> &'static str {
    static VAL: OnceLock<String> = OnceLock::new();
    VAL.get_or_init(|| {
        field(field(doc(), "source"), "commit")
            .as_str()
            .unwrap_or("")
            .to_string()
    })
    .as_str()
}

// --- Composite pass enable gate -------------------------------------------

/// The per-field activity magnitudes the composite pass gate inspects. These
/// are the absolute values of the animated `SfxVars` the JS `any` expression
/// tests against the per-term thresholds (screenfx.js L862-865).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassActivity {
    pub lens_on: bool,
    pub speed: f64,
    pub stretch: f64,
    pub punch: f64,
    pub blast: f64,
    pub chroma: f64,
    pub edge_ink: f64,
    pub aura: f64,
    pub heart: f64,
    pub hurt: f64,
    pub urg: f64,
    pub swim: f64,
    pub focus: f64,
    pub charge_pulse: f64,
    pub shimmer: f64,
    pub kill: f64,
    pub flash: f64,
    pub desat: f64,
    pub sat_delta: f64,
    pub sat_pop: f64,
    pub flood: f64,
}

impl PassActivity {
    /// Evaluate the exact JS `any` expression against the extracted thresholds.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.lens_on
            || self.speed > pass_term("speed")
            || self.stretch > pass_term("stretch")
            || self.punch.abs() > pass_term("punch")
            || self.blast > pass_term("blast")
            || self.chroma > pass_term("chroma")
            || self.edge_ink > pass_term("edgeInk")
            || self.aura > pass_term("aura")
            || self.heart > pass_term("heart")
            || self.hurt > pass_term("hurt")
            || self.urg > pass_term("urg")
            || self.swim > pass_term("swim")
            || self.focus > pass_term("focus")
            || self.charge_pulse > pass_term("chargePulse")
            || self.shimmer > pass_term("shimmer")
            || self.kill > pass_term("kill")
            || self.flash > pass_term("flash")
            || self.desat > pass_term("desat")
            || self.sat_delta.abs() > pass_term("sat")
            || self.sat_pop > pass_term("satPop")
            || self.flood > pass_term("flood")
    }
}

/// Composite pass enable decision. `post_enabled=false` (the low-tier
/// swiftshader path) forces the pass off regardless of activity — D-T64-3.
#[must_use]
pub fn composite_pass_enabled(activity: &PassActivity, post_enabled: bool) -> bool {
    post_enabled && activity.visible()
}

// --- Damage-number pool (hud-boss.js L402-440) -----------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct DamageNumber {
    pub sum: i64,
    pub last: f64,
    pub born: f64,
    pub weak: bool,
    pub out: bool,
}

impl DamageNumber {
    fn scale(&self, spec: &DamageNumberSpec) -> f64 {
        // hud-boss.js L416, pinned constants 0.85/0.16/1.6.
        let log = (self.sum.max(1) as f64).log10();
        (0.85
            + log * 0.16
            + if self.weak {
                spec.weak_scale_bonus
            } else {
                0.0
            })
        .clamp(0.85, 1.6)
    }
}

/// Pool of boss damage numbers with combo merge + expiry (all pure data).
#[derive(Clone, Debug, Default)]
pub struct DamageNumberPool {
    pub items: Vec<DamageNumber>,
}

impl DamageNumberPool {
    /// Add damage at time `now`. Combo merges into the last live number within
    /// `combo_window` (hud-boss.js L405); otherwise a new one is created and
    /// the pool is capped (L410).
    pub fn add(&mut self, dmg: i64, weak: bool, now: f64) {
        let spec = damage_number_spec();
        let merge = self
            .items
            .last()
            .is_some_and(|c| !c.out && now - c.last <= spec.combo_window);
        if merge {
            let c = self.items.last_mut().expect("checked above");
            c.sum += dmg;
            c.last = now;
            c.weak |= weak;
        } else {
            self.items.push(DamageNumber {
                sum: dmg,
                last: now,
                born: now,
                weak,
                out: false,
            });
            while self.items.len() > spec.pool_cap {
                self.items.remove(0);
            }
        }
    }

    /// Advance expiry (hud-boss.js L432-434): mark `out` past `out_after`,
    /// remove past `life`. Returns the retained count.
    pub fn tick(&mut self, now: f64) -> usize {
        let spec = damage_number_spec();
        self.items.retain(|c| now - c.last <= spec.life);
        for c in &mut self.items {
            if now - c.last > spec.out_after {
                c.out = true;
            }
        }
        self.items.len()
    }

    /// Current display scale of the live combo number (hud-boss.js L416).
    #[must_use]
    pub fn scale(&self) -> Option<f64> {
        self.items.last().map(|c| c.scale(&damage_number_spec()))
    }
}

// --- Minimap transient FX lane (minimap.js L19-24) -------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapFxKind {
    Slam,
    Boom,
    Jump,
    Spawn,
}

impl MapFxKind {
    fn life(self, spec: &MinimapSpec) -> f64 {
        match self {
            Self::Slam => spec.slam_life,
            Self::Boom => spec.boom_life,
            Self::Jump => spec.jump_life,
            Self::Spawn => spec.spawn_life,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapFx {
    pub kind: MapFxKind,
    /// Age in seconds ≡ JS `f.t` (minimap.js L395 `f.t += dt`).
    pub t: f64,
    /// Max age in seconds ≡ JS `f.life` (expired when `f.t >= f.life`, L396).
    pub life: f64,
}

/// Lifecycle of one transient minimap effect (pure data, no canvas).
#[derive(Clone, Debug, Default)]
pub struct MinimapFxLane {
    pub items: Vec<MapFx>,
}

impl MinimapFxLane {
    pub fn push(&mut self, kind: MapFxKind) {
        let spec = minimap_spec();
        self.items.push(MapFx {
            kind,
            t: 0.0,
            life: kind.life(&spec),
        });
        if self.items.len() > spec.fx_cap {
            self.items.remove(0);
        }
    }

    /// A landing shortens every jump FX to `t + land_life` (minimap.js L23:
    /// `f.life = Math.min(f.life, f.t + 0.35)`). `t` is the FX's current age,
    /// so the remaining lifetime after landing is exactly `land_life`.
    pub fn land_jump(&mut self) {
        let spec = minimap_spec();
        for fx in &mut self.items {
            if fx.kind == MapFxKind::Jump {
                fx.life = fx.life.min(fx.t + spec.land_life);
            }
        }
    }

    /// Advance by `dt` seconds and drop expired FX (≡ minimap.js L395-396).
    pub fn tick(&mut self, dt: f64) {
        for fx in &mut self.items {
            fx.t += dt;
        }
        self.items.retain(|fx| fx.t < fx.life);
    }

    #[must_use]
    pub fn remaining(fx: &MapFx) -> f64 {
        (fx.life - fx.t).max(0.0)
    }
}

// --- Bevy production wiring ------------------------------------------------

/// The live composite-overlay state, updated every frame from the real
/// `ScreenFxState` (pass activity) and `EnvState` (post-processing / tier).
/// Consumed by the DOM overlay bridge (Task 62 nameplates + future damage
/// numbers / map dots). Headless-safe: no GPU or DOM handle is held.
#[derive(Resource, Debug, Default)]
pub struct CompositeOverlay {
    pub pass_enabled: bool,
    pub activity: PassActivity,
    pub damage_numbers: DamageNumberPool,
    pub minimap_fx: MinimapFxLane,
    /// Number of visible Task 62 lobby nameplates in the unified DOM overlay
    /// channel (D-T64-3). The plates themselves live in `LobbyNameplates`.
    pub visible_nameplates: usize,
    pub elapsed: f64,
}

impl CompositeOverlay {
    /// Mirror the animated `SfxVars` into the pass gate's activity view.
    pub fn activity_from(v: &crate::fx::screen_fx::SfxVars) -> PassActivity {
        PassActivity {
            lens_on: false,
            speed: v.speed as f64,
            stretch: v.stretch as f64,
            punch: v.punch as f64,
            blast: v.blast as f64,
            chroma: v.chroma as f64,
            edge_ink: v.edge_ink as f64,
            aura: v.aura as f64,
            heart: v.heart as f64,
            hurt: v.hurt as f64,
            urg: v.urg as f64,
            swim: v.swim as f64,
            focus: v.focus as f64,
            charge_pulse: v.charge_pulse as f64,
            shimmer: v.shimmer as f64,
            kill: v.kill as f64,
            flash: v.flash as f64,
            desat: v.desat as f64,
            sat_delta: (v.sat - 1.0) as f64,
            sat_pop: v.sat_pop as f64,
            flood: v.flood as f64,
        }
    }
}

/// Per-frame pass gate: runs after `screen_fx_update` so it sees this frame's
/// state. `post_enabled=false` (low tier) forces every overlay off — D-T64-3.
pub fn composite_overlay_update(
    time: bevy::prelude::Res<bevy::prelude::Time>,
    env: bevy::prelude::Res<crate::environment::EnvState>,
    sfx: bevy::prelude::Res<crate::fx::screen_fx::ScreenFxState>,
    plates: bevy::prelude::Res<crate::ui::showcase_state::LobbyNameplates>,
    demo: bevy::prelude::Res<crate::ink_render::DemoSim>,
    mut overlay: ResMut<CompositeOverlay>,
) {
    let dt = time.delta_secs() as f64;
    overlay.elapsed += dt;
    let post = env.params.post_enabled;
    // Unified DOM overlay channel: nameplates projected by Task 62 feed the
    // same data resource (D-T64-3). Real DOM nodes remain a user-machine item.
    overlay.visible_nameplates = plates
        .0
        .iter()
        .filter(|p| p.tag.vis && !p.tag.behind && !p.free)
        .count();
    overlay.activity = CompositeOverlay::activity_from(&sfx.s);
    // The pass's own `lens_on` term is carried by ScreenFxState.u.u_lens_on.
    overlay.activity.lens_on = sfx.u.u_lens_on;
    overlay.pass_enabled = composite_pass_enabled(&overlay.activity, post);
    if !post {
        overlay.damage_numbers.items.clear();
        overlay.minimap_fx.items.clear();
        return;
    }

    // Real producers: consume read-only `Match::events` BEFORE the UI drain
    // (this system runs right after `screen_fx_update`, same rule). A hit that
    // landed on a proxy feeds the boss-style damage-number pool; slam / respawn
    // feed the transient minimap FX lane (minimap.js L20/L24).
    let now = overlay.elapsed;
    for ev in demo.m.events.iter() {
        match ev {
            inkwave_sim::match_::MatchEvent::HitSend { damage, .. } => {
                if *damage > 0.0 {
                    overlay
                        .damage_numbers
                        .add(damage.round() as i64, false, now);
                }
            }
            inkwave_sim::match_::MatchEvent::SpecialSlam { .. } => {
                overlay.minimap_fx.push(MapFxKind::Slam);
            }
            inkwave_sim::match_::MatchEvent::Respawn { .. } => {
                overlay.minimap_fx.push(MapFxKind::Spawn);
            }
            _ => {}
        }
    }
    overlay.damage_numbers.tick(now);
    overlay.minimap_fx.tick(dt);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task64_composite_spec_matches_extracted_js() {
        let c = composite();
        assert_eq!(c.material_name, "InkwaveScreenFX");
        assert_eq!(c.taps, 8);
        assert_eq!(c.pass_order.join(","), "grade,screenfx,output");
        assert_eq!(c.res, [1600, 900]);
        assert!((c.aspect - 16.0 / 9.0).abs() < 1e-9);
        assert_eq!(c.heart, [0.6, 0.02, 0.06, 0.0]);
        assert_eq!(c.hurt, 0.0);
        assert_eq!(c.charge, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(c.sat, 1.0);
        assert_eq!(c.kill, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(source_commit().len(), 40);
        // crosscheck pins the exact threshold set; verify the Rust view agrees.
        assert_eq!(pass_term("flood"), 0.001);
        assert_eq!(pass_term("punch"), 0.0004);
        assert_eq!(pass_term("stretch"), 0.0005);
        assert_eq!(pass_term("chroma"), 0.004);
    }

    #[test]
    fn task64_decay_and_pool_specs_are_pinned() {
        let d = decay();
        assert_eq!(d.rate, 6.0);
        assert_eq!(d.low_k_denom, 0.38);
        assert_eq!(d.bpm_base, 80.0);
        assert_eq!(d.bpm_scale, 70.0);
        assert_eq!(d.hurt_damp, 4.0);
        assert_eq!(d.heart_damp, 6.0);
        assert_eq!(d.damage_window, 0.06);
        let b = damage_number_spec();
        assert_eq!(b.pool_cap, 6);
        assert_eq!(b.combo_window, 0.45);
        assert_eq!(b.life, 1.05);
        assert_eq!(b.out_after, 0.45);
        assert_eq!(b.crit_ms, 900);
        let m = minimap_spec();
        assert_eq!(m.dot_count, 8);
        assert_eq!(m.death_mark_count, 12);
        assert_eq!(m.fx_cap, 40);
        assert_eq!(
            (
                m.slam_life,
                m.boom_life,
                m.jump_life,
                m.land_life,
                m.spawn_life
            ),
            (0.9, 0.7, 3.0, 0.35, 0.8)
        );
    }

    #[test]
    fn task64_pass_gate_low_tier_forces_all_off() {
        let a = PassActivity {
            flood: 1.0,
            heart: 1.0,
            lens_on: true,
            ..Default::default()
        };
        assert!(a.visible(), "activity alone is visible");
        assert!(
            !composite_pass_enabled(&a, false),
            "low tier (post off) forces pass off"
        );
        assert!(
            composite_pass_enabled(&a, true),
            "post on + activity enables pass"
        );
        let idle = PassActivity::default();
        assert!(!composite_pass_enabled(&idle, true), "idle = zero cost");
        // Threshold boundary: flood uses 0.001, others 0.002.
        let mut edge = PassActivity {
            flood: 0.001,
            ..Default::default()
        };
        assert!(!edge.visible(), "at threshold is not visible (strict >)");
        edge.flood = 0.0011;
        assert!(edge.visible());
    }

    #[test]
    fn task64_damage_number_pool_combo_cap_and_expiry() {
        let mut pool = DamageNumberPool::default();
        pool.add(10, false, 0.0);
        pool.add(15, true, 0.2); // within 0.45 combo → merge, weak
        assert_eq!(pool.items.len(), 1);
        assert_eq!(pool.items[0].sum, 25);
        assert!(pool.items[0].weak);
        let scaled = pool.scale().expect("live combo");
        assert!((0.85..=1.6).contains(&scaled));
        pool.add(5, false, 0.3);
        assert_eq!(pool.items[0].sum, 30);
        // Expiry: out after 0.45, removed after 1.05.
        assert_eq!(pool.tick(0.8), 1);
        assert!(pool.items[0].out);
        assert_eq!(pool.tick(1.4), 0, "past life removed");
        // Cap at 6 distinct combos (>0.45 apart).
        for i in 0..9 {
            pool.add(1, false, i as f64 * 1.0);
        }
        assert_eq!(pool.items.len(), 6, "pool capped at 6");
    }

    fn test_demo() -> crate::ink_render::DemoSim {
        let layout = inkwave_sim::embedded_tidewater();
        let world = inkwave_sim::collision::CollisionWorld::from_layout(&layout);
        crate::ink_render::DemoSim::new(layout, world, 20261004)
    }

    #[test]
    fn task64_production_system_low_tier_forces_all_overlays_off() {
        use bevy::prelude::{App, MinimalPlugins, Time};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<CompositeOverlay>();
        app.init_resource::<crate::fx::screen_fx::ScreenFxState>();
        app.init_resource::<crate::ui::showcase_state::LobbyNameplates>();
        app.insert_resource(test_demo());
        // low tier: post_enabled must be false (the swiftshader保底 path).
        let mut low = crate::settings::Settings::load();
        low.quality = "low".to_string();
        low.boot_quality = None;
        app.insert_resource(crate::environment::EnvState::from_settings(&low));
        app.add_systems(bevy::prelude::Update, composite_overlay_update);

        // Seed visible activity + a damage number + a map FX.
        {
            let mut sfx = app
                .world_mut()
                .resource_mut::<crate::fx::screen_fx::ScreenFxState>();
            sfx.s.flood = 1.0;
            sfx.s.heart = 1.0;
        }
        {
            let mut ov = app.world_mut().resource_mut::<CompositeOverlay>();
            ov.damage_numbers.add(30, true, 0.0);
            ov.minimap_fx.push(MapFxKind::Slam);
        }
        app.update();
        let ov = app.world().resource::<CompositeOverlay>();
        assert!(!ov.pass_enabled, "low tier (post off) must force pass off");
        assert!(
            ov.damage_numbers.items.is_empty(),
            "low tier clears damage overlays"
        );
        assert!(
            ov.minimap_fx.items.is_empty(),
            "low tier clears map FX overlays"
        );
        assert!(
            ov.activity.flood > 0.0,
            "activity is still mirrored for diagnostics"
        );

        // High tier: post on, activity drives the pass, and a real HitSend
        // event on the demo bus produces a damage number via the system.
        let mut high = crate::settings::Settings::load();
        high.quality = "high".to_string();
        high.boot_quality = None;
        app.insert_resource(crate::environment::EnvState::from_settings(&high));
        app.world_mut()
            .resource_mut::<crate::ink_render::DemoSim>()
            .m
            .events
            .push(inkwave_sim::match_::MatchEvent::HitSend {
                attacker: (0, 0),
                victim: (1, 0),
                damage: 42.0,
            });
        app.update();
        let ov = app.world().resource::<CompositeOverlay>();
        assert!(
            ov.pass_enabled,
            "high tier + flood activity enables the pass"
        );
        assert_eq!(
            ov.damage_numbers.items.len(),
            1,
            "HitSend event produces a damage number (no dead data)"
        );
        assert_eq!(ov.damage_numbers.items[0].sum, 42);
        // Time advanced across updates without panic (headless, no window/GPU).
        assert!(app.world().resource::<Time>().elapsed_secs() >= 0.0);
    }

    #[test]
    fn task64_minimap_fx_lifecycle_and_landing_age() {
        let mut lane = MinimapFxLane::default();
        lane.push(MapFxKind::Jump);
        assert_eq!(lane.items.len(), 1);
        assert_eq!(lane.items[0].life, 3.0);
        // P1-1 regression: landing at age t=1.2 must leave 0.35 s remaining,
        // NOT set life to an absolute 0.35 (which would instantly expire it).
        lane.tick(1.2);
        assert!((lane.items[0].t - 1.2).abs() < 1e-9);
        lane.land_jump();
        assert!(
            (lane.items[0].life - 1.55).abs() < 1e-9,
            "life = min(3.0, t + 0.35) = 1.55"
        );
        assert!(
            (MinimapFxLane::remaining(&lane.items[0]) - 0.35).abs() < 1e-9,
            "landing leaves exactly land_life remaining"
        );
        lane.tick(0.34);
        assert_eq!(lane.items.len(), 1, "jump alive just before landing+0.35");
        lane.tick(0.02); // total t = 1.56 > 1.55 → landed jump expires
        assert!(lane.items.is_empty(), "landed jump expired at t+0.35");
    }

    #[test]
    fn task64_minimap_fx_push_and_events() {
        let mut lane = MinimapFxLane::default();
        lane.push(MapFxKind::Slam);
        lane.push(MapFxKind::Boom);
        lane.push(MapFxKind::Jump);
        lane.push(MapFxKind::Spawn);
        assert_eq!(lane.items.len(), 4);
        assert_eq!(lane.items[0].life, 0.9);
        assert_eq!(lane.items[1].life, 0.7);
        assert_eq!(lane.items[2].life, 3.0);
        assert_eq!(lane.items[3].life, 0.8);
        lane.tick(0.95);
        // slam (0.9), boom (0.7), spawn (0.8) all expired; jump (3.0) retained.
        assert_eq!(lane.items.len(), 1, "short FX expired, jump retained");
        assert_eq!(lane.items[0].kind, MapFxKind::Jump);
        lane.tick(2.1); // jump: t=3.05 > 3.0 expired
        assert!(lane.items.is_empty(), "jump expired past 3.0 s");
    }
}
