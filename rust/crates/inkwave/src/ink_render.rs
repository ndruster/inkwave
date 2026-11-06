//! Task 11 - ink rendering: atlas upload + extended PBR material + demo sim.
//!
//! The sim owns the gameplay grid (`PaintGrid`); this module owns the
//! presentation bitmap (`InkAtlas`): every frame the demo match's recorded
//! splat stream is replayed onto the atlas bitmap and the RGBA8 image is
//! re-uploaded (whole-texture `write_texture` on the dirty-version change -
//! the gpu_image reuse path keeps the GPU texture alive across updates).
//!
//! `InkExt` is a `MaterialExtension` over `StandardMaterial` whose fragment
//! shader (`ink.wgsl`) samples the atlas through mesh UV1 (the per-face atlas
//! rect written by `world::emit_box`) and blends team ink over the surface
//! colour before lighting.

use std::sync::{Mutex, OnceLock};

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use inkwave_sim::actor::FIXED_DT;
use inkwave_sim::actor::{ActorInput, SimWorld as ActorSimWorld};
use inkwave_sim::boss::BossEvent;
use inkwave_sim::boss::BossMode;
use inkwave_sim::boss::nav::BossNav;
use inkwave_sim::bot::{Bot, BotCtx};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::geometry::StageLayout;
use inkwave_sim::ink_atlas::InkAtlas;
use inkwave_sim::match_::{Match, MatchMode, Phase};
use inkwave_sim::nav::NavGraph;
use inkwave_sim::tuning::Tuning;

// ---------------------------------------------------------------- shader

static INK_SHADER: OnceLock<Handle<Shader>> = OnceLock::new();

/// Material extension adding the ink-atlas blend to `StandardMaterial`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct InkExt {
    /// The shared ink atlas texture (RGBA8: share / wet / tone / coverage).
    /// Bindings 20/21 sit above every default `StandardMaterial` slot; if the
    /// `pbr_transmission_textures` / `pbr_multi_layer_material_textures`
    /// features are ever enabled these numbers must move.
    #[texture(20)]
    #[sampler(21)]
    pub ink_atlas: Handle<Image>,
}

/// Ink-blended standard material.
///
/// Legacy since Task 52 D2: the level geometry batches switched to
/// [`crate::level_render::LevelExt`] (the full ported `level.wgsl` surface);
/// this extension and `ink.wgsl` remain only as the M1 reference path.
pub type InkMaterial = ExtendedMaterial<StandardMaterial, InkExt>;

impl MaterialExtension for InkExt {
    fn fragment_shader() -> ShaderRef {
        INK_SHADER
            .get()
            .cloned()
            .map(ShaderRef::Handle)
            .unwrap_or(ShaderRef::Default)
    }
}

// ---------------------------------------------------------------- plugin

/// Registers the ink material + atlas upload systems.
pub struct InkPlugin;

impl Plugin for InkPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "ink.wgsl");
        let handle: Handle<Shader> = load_embedded_asset!(app, "ink.wgsl");
        let _ = INK_SHADER.set(handle);

        app.add_plugins(MaterialPlugin::<InkMaterial>::default());
    }
}

// ---------------------------------------------------------------- atlas

/// The presentation bitmap, owned by the render layer.
#[derive(Resource)]
pub struct InkAtlasRes {
    pub atlas: InkAtlas,
    /// `InkAtlas::version` at the last GPU upload.
    pub last_upload: u32,
}

impl InkAtlasRes {
    /// Build the atlas for `world` and its (initially empty) GPU image.
    #[must_use]
    pub fn new(world: &CollisionWorld) -> Self {
        Self {
            atlas: InkAtlas::new(world),
            last_upload: 0,
        }
    }

    /// RGBA8 atlas image; `COPY_DST` keeps the gpu-image reuse path alive so
    /// data updates upload in place instead of recreating the texture.
    #[must_use]
    pub fn make_image(&self) -> Image {
        let size = self.atlas.size();
        Image::new(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0u8; (size as usize) * (size as usize) * 4],
            TextureFormat::Rgba8Unorm,
            default(),
        )
    }
}

/// Handle of the GPU atlas image (inserted by `build_level_world`).
#[derive(Resource)]
pub struct InkImage(pub Handle<Image>);

/// Replay the demo match's recorded splats onto the atlas and upload the
/// bitmap when it changed (whole-texture copy on the dirty-version edge).
#[allow(clippy::too_many_arguments)] // Bevy system params are individual resources by design
pub fn ink_upload(
    time: Res<Time>,
    mut atlas: ResMut<InkAtlasRes>,
    mut demo: ResMut<DemoSim>,
    mut images: ResMut<Assets<Image>>,
    image: Res<InkImage>,
    pc: Res<crate::input::PlayerControls>,
    nms: Res<crate::net::match_link::NetMatchState>,
    mut ripple: ResMut<crate::ink_ripple::RippleTable>,
) {
    if pc.paused {
        // Esc pause freezes the match (camera and atlas stay as-is).
        return;
    }
    if nms.rep.is_some() {
        // Task 24: a net match is live — `net_tick` owns the sim step (the
        // replication controller drives bots, masked physics and playback).
        // Only replay its paint stream onto the atlas here.
        demo.drain_atlas(&mut atlas, &mut ripple);
    } else {
        demo.player = Some((pc.intent, pc.yaw, pc.pitch));
        demo.step_and_drain(&mut atlas, &mut ripple, time.delta_secs().min(0.1));
    }
    if atlas.atlas.version() != atlas.last_upload {
        atlas.last_upload = atlas.atlas.version();
        atlas.atlas.take_dirty();
        if let Some(mut img) = images.get_mut(&image.0) {
            img.data = Some(atlas.atlas.pixels().to_vec());
        }
    }
}

// ---------------------------------------------------------------- demo sim

/// A running 4v4 easy-bot match driving the ink stream for TR-11 verification
/// (Task 12 replaces this with player input + character rendering).
#[derive(Resource)]
pub struct DemoSim {
    pub layout: StageLayout,
    pub world: CollisionWorld,
    pub tuning: Tuning,
    pub nav: NavGraph,
    pub m: Match,
    pub bots: Vec<Bot>,
    pub inputs: Vec<ActorInput>,
    pub mate_goals: Vec<Option<usize>>,
    pub acc: f32,
    pub splats: u64,
    /// `PaintGrid::version` seen at the previous drain; a drop below it means
    /// `Match::restart` rebuilt the grid and the atlas bitmap must follow.
    last_grid_version: u32,
    /// Local player override for `player_slot`: (intent, aim_yaw, aim_pitch).
    /// `Some` replaces the easy-bot brain for that slot (Task 12 input
    /// wiring); `None` keeps the demo fully bot-driven (headless / TR-11).
    pub player: Option<(ActorInput, f32, f32)>,
    /// Task 24: the local player's roster slot (JS `m.local`). Offline it is
    /// 0; a net match sets it to the session's local index, and the camera
    /// rig follows the same slot.
    pub player_slot: usize,
    /// Seed the roster/bots were built with (kept so `replay` can rebuild
    /// the bot brains fresh, matching the first match).
    seed: u64,
    /// Task 46 batch B: the live Boss Battle wrapper — `Some` only while a
    /// boss match runs (offline `replay_boss` / a net boss room's host or
    /// follower). D-T45-9 unblocked: batch A made `Boss` own its nav /
    /// collision / tuning, so `BossMode` is `'static` and this `Resource`
    /// can hold it across frames (≡ `autopilot::run_boss` owning it in the
    /// driver, JS `bossMode.js` L18-28). The `Mutex` is not about
    /// contention: `BossNav`'s A* scratch `RefCell`/`Cell` buffers make
    /// `BossMode` `Send` but not `Sync`, and a Bevy `Resource` needs both —
    /// exactly one system (the match stepper) ever holds the lock.
    pub bm: Mutex<Option<BossMode>>,
    /// Every [`BossEvent`] the live boss raised since the match started, in
    /// frame order (JS `emit('boss:*')` bus — the sim has none; drained from
    /// `Boss::drain_events` each step). Batch C (HUD / render / audio)
    /// consumes this stream.
    pub boss_events: Vec<BossEvent>,
}

impl DemoSim {
    #[must_use]
    pub fn new(layout: StageLayout, world: CollisionWorld, seed: u64) -> Self {
        let tuning = inkwave_sim::embedded_tuning();
        let nav = NavGraph::new(&world, &layout, &tuning.player);
        let sim = ActorSimWorld::new(&world, &layout);
        let m = Match::new(90.0, seed, &sim, &tuning.player, &tuning.match_config);
        let n = m.actors.len();
        let bots: Vec<Bot> = m
            .actors
            .iter()
            .enumerate()
            .map(|(i, a)| Bot::new(a, &tuning.difficulty.easy, seed, i))
            .collect();
        Self {
            layout,
            world,
            tuning,
            nav,
            m,
            bots,
            inputs: vec![ActorInput::default(); n],
            mate_goals: vec![None; n],
            acc: 0.0,
            splats: 0,
            last_grid_version: 0,
            player: None,
            player_slot: 0,
            seed,
            bm: Mutex::new(None),
            boss_events: Vec::new(),
        }
    }

    /// Advance the match at the fixed 60 Hz step (frame-rate independent) and
    /// replay every recorded splat onto the ink atlas.
    fn step_and_drain(
        &mut self,
        atlas: &mut InkAtlasRes,
        ripple: &mut crate::ink_ripple::RippleTable,
        dt: f32,
    ) {
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= FIXED_DT && steps < 4 {
            self.acc -= FIXED_DT;
            self.step_once();
            steps += 1;
        }
        self.drain_atlas(atlas, ripple);
    }

    /// Task 24: replay the match's recorded splat stream onto the atlas
    /// (the drain half of [`step_and_drain`], reused while `net_tick` owns
    /// the sim step of a net match).
    ///
    /// Task 52 batch E2: each splat also feeds the presentation ripple table
    /// (≡ paint.js `splat()` → `ripple()` L540 — the birth clock is the
    /// table's pre-advance value, matching the JS flush order).
    pub fn drain_atlas(
        &mut self,
        atlas: &mut InkAtlasRes,
        ripple: &mut crate::ink_ripple::RippleTable,
    ) {
        if self.m.paint.version < self.last_grid_version {
            // Match::restart rebuilt the gameplay grid: wipe the bitmap so
            // both views stay in lock-step.
            atlas.atlas.clear();
        }
        self.last_grid_version = self.m.paint.version;
        let stream = self.m.paint.drain_ink();
        self.splats += stream.len() as u64;
        for s in stream {
            ripple.apply(&crate::ink_ripple::RippleSource::Splat(
                crate::ink_ripple::splat_event_from_ink(&s),
            ));
            atlas
                .atlas
                .splat(&self.world, s.center, s.radius, s.team, &s.opts);
        }
    }

    pub fn step_once(&mut self) {
        let t = &self.tuning;
        let sim = ActorSimWorld::new(&self.world, &self.layout);
        let n = self.m.actors.len();
        // One guard for the whole step (see the `bm` field doc).
        let mut bmg = self.bm.lock().unwrap();
        if self.m.phase == Phase::Active {
            let snap = self.m.actors.clone();
            for (i, b) in self.bots.iter().enumerate() {
                self.mate_goals[i] = b.goal_node();
            }
            let mate_minds = self.bots.iter().map(|b| Some(b.mind())).collect::<Vec<_>>();
            // Task 46: a live boss hands the brains its entity (JS `G.boss`,
            // bots.js L345 ≡ `autopilot::run_boss`'s `BotCtx { boss:
            // Some(&bm.boss) }`).
            let boss_ent = bmg.as_ref().map(|bm| &bm.boss);
            for i in 0..n {
                if i == self.player_slot
                    && let Some((intent, yaw, pitch)) = self.player
                {
                    // local player drives its slot: the bot brain stands down
                    // and the aim angles come from the mouse look.
                    self.inputs[i] = intent;
                    self.m.actors[i].aim_yaw = yaw;
                    self.m.actors[i].aim_pitch = pitch;
                    continue;
                }
                let ctx = BotCtx {
                    actors: &snap,
                    nav: &self.nav,
                    paint: &self.m.paint,
                    world: &sim,
                    t: &t.player,
                    weapons: &t.weapons,
                    weapon_range: &t.weapon_range,
                    kinds: &self.m.kinds,
                    runners: &self.m.runners,
                    mate_goals: &self.mate_goals,
                    subs: &self.m.subs,
                    mate_minds: &mate_minds,
                    // The demo match is turf-only (`Match::new`), so the zone
                    // plan is never live here (Task 38 batch 3).
                    zones: None,
                    zctl: None,
                    boss: boss_ent,
                    launched: &self.m.launched,
                    time: self.m.projectiles.time,
                };
                self.inputs[i] = self.bots[i].step(FIXED_DT, &mut self.m.actors[i], &ctx);
            }
        } else {
            for i in 0..n {
                self.inputs[i] = self.bots[i].hold(&self.m.actors[i]);
            }
        }
        self.m.step(
            FIXED_DT,
            &self.inputs,
            &sim,
            &t.player,
            &t.weapons,
            &t.subs,
            // Task 46: the driver hands the owned `BossMode` to the match
            // every frame (≡ `run_boss`'s `Some(&mut bm)`; JS match.js L229
            // `bossMode?.update(dt)`). `None` keeps the turf path byte-for-bite
            // identical to before.
            bmg.as_mut().map(|bm| &mut *bm),
        );
        // JS `emit('boss:*')` had no bus in the sim — drain the queue into
        // the app-side event stream (batch C HUD / render / audio consumer).
        if let Some(bm) = bmg.as_mut() {
            self.boss_events.extend(bm.boss.drain_events());
        }
        // Task 13: no auto-restart — the UI shows the results screen when the
        // match reaches `Phase::End` and replays on demand (`replay`).
    }

    /// Fresh match of `duration` seconds (menu Play / results Restart).
    /// Rebuilds the roster and the bot brains at the requested clock and
    /// re-enters the intro; the atlas bitmap follows via the grid-version
    /// rollback in [`step_and_drain`].
    pub fn replay(&mut self, duration: f32) {
        let sim = ActorSimWorld::new(&self.world, &self.layout);
        // Bind the probe result first: an `if`-condition temporary guard
        // lives through the whole block and the body re-locks below.
        let boss_live = self.bm.lock().unwrap().is_some();
        if boss_live {
            // Task 46: leaving a boss match for turf — `restart` would keep
            // the Boss-shaped match (`boss_rules`), so rebuild the plain turf
            // match fresh instead (≡ `restore_offline_demo`'s shape).
            *self.bm.lock().unwrap() = None;
            self.boss_events.clear();
            let seed = self.m.seed;
            self.m = Match::new(
                duration,
                seed,
                &sim,
                &self.tuning.player,
                &self.tuning.match_config,
            );
        } else {
            self.m.restart(&sim, &self.tuning.player);
            self.m.duration = duration;
            self.m.time = duration;
        }
        self.acc = 0.0;
        self.splats = 0;
        // Fresh brains: no leftover target nodes / timers from the last match.
        let t = &self.tuning;
        self.bots = self
            .m
            .actors
            .iter()
            .enumerate()
            .map(|(i, a)| Bot::new(a, &t.difficulty.easy, self.seed, i))
            .collect();
        self.mate_goals = vec![None; self.bots.len()];
    }

    /// Task 46 batch B (D-T44-5): start a Boss Battle — the offline entry the
    /// JS game only ever had through `netmatch` (JS `netmatch.js` L164-171
    /// boss branch). The shape replicates `inkwave_sim::autopilot::run_boss`
    /// L255-300: one team-0 squad of `rules.squad` slots, the local player in
    /// slot 0, and the owned [`BossMode`] held across frames by this Resource
    /// (D-T45-9 unblocked by batch A's owned refactor).
    ///
    /// `duration <= 0` takes `BOSS_MODE.duration` (JS `match.js` L21: the
    /// boss clock ignores the lobby time). `difficulty` picks the HP tier —
    /// JS `bossMode.js` L21-22: `maxHp = round(hpPerUnit × squadUnits(humans,
    /// bots) × hpDiff[diff])`, with the offline squad = 1 local + 7 bots.
    pub fn replay_boss(&mut self, duration: f32, difficulty: &str) {
        let sim = ActorSimWorld::new(&self.world, &self.layout);
        let t = &self.tuning;
        let rules = t.boss.mode;
        let squad = rules.squad.max(1) as usize;
        // JS bossMode.js L20-21: slot 0 is the local human, the rest bots.
        let units = inkwave_sim::boss::squad_units(1, squad - 1, &t.boss.squad_units);
        let nav = BossNav::new(&sim, &t.boss.nav);
        self.m = Match::with_roster_mode(
            duration,
            self.seed,
            &sim,
            &t.player,
            &t.match_config,
            squad,
            MatchMode::Boss,
            None,
            None,
            Some(rules),
        );
        // JS bossMode.js L23: `sim: !match.follower` — offline is always the
        // host.
        *self.bm.lock().unwrap() = Some(BossMode::new(
            &sim,
            nav,
            t.boss.clone(),
            units,
            difficulty,
            true,
            self.seed,
        ));
        self.boss_events.clear();
        self.acc = 0.0;
        self.splats = 0;
        let t = &self.tuning;
        self.bots = self
            .m
            .actors
            .iter()
            .enumerate()
            .map(|(i, a)| Bot::new(a, &t.difficulty.easy, self.seed, i))
            .collect();
        self.mate_goals = vec![None; self.bots.len()];
        self.inputs = vec![ActorInput::default(); self.bots.len()];
        self.player_slot = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwave_sim::actor::Form;

    fn demo() -> DemoSim {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        DemoSim::new(layout, world, 20261004)
    }

    /// TR-12.1 (headless half): the injected player intent drives slot 0 —
    /// aim angles land on the actor, the move dir moves the kid, and the bot
    /// brain stands down for that slot only.
    #[test]
    fn tr12_1_player_input_drives_slot0() {
        let mut d = demo();
        // run through the intro (4.2 s = 252 steps) with the bots driving
        for _ in 0..300 {
            d.step_once();
        }
        assert_eq!(d.m.phase, Phase::Active);

        let intent = ActorInput {
            move_dir: glam::Vec3::new(0.0, 0.0, 1.0),
            jump: true,
            ..ActorInput::default()
        };
        d.player = Some((intent, 1.25, -0.2));
        // keep slot 0 alive through the window so the test is not flaky when
        // a bot lands a kill (the sim's own respawn would hide the actor).
        d.m.actors[0].set_invuln(2.0);
        let p0 = d.m.actors[0].pos;
        for _ in 0..60 {
            d.step_once();
        }
        let a0 = &d.m.actors[0];
        assert_eq!(a0.aim_yaw, 1.25);
        assert_eq!(a0.aim_pitch, -0.2);
        assert_eq!(a0.form, Form::Kid);
        assert_eq!(d.inputs[0], intent, "the bot must not overwrite slot 0");
        assert!(
            a0.pos.distance(p0) > 0.3,
            "player move input did not translate: {:?}",
            a0.pos - p0
        );
    }

    /// Without an override the demo stays fully bot-driven (TR-11 baseline).
    #[test]
    fn tr12_1_no_override_keeps_bots() {
        let mut d = demo();
        for _ in 0..300 {
            d.step_once();
        }
        assert_eq!(d.m.phase, Phase::Active);
        let p0 = d.m.actors[0].pos;
        for _ in 0..60 {
            d.step_once();
        }
        assert!(
            d.m.actors[0].pos.distance(p0) > 0.05,
            "slot 0 bot must keep driving without an override"
        );
    }

    /// Task 46 batch B: the offline Boss Battle entry (D-T44-5). The HP
    /// formula is pinned to JS `bossMode.js` L21-22 — `units = 1 + 7 × 0.6 =
    /// 5.2`, `maxHp = round(18000 × 5.2 × hpDiff[diff])` (JS `boss.js` L17-18)
    /// — and the frame loop must drain the boss event queue into
    /// `boss_events`.
    #[test]
    fn demo_boss_match_drives_and_records_events() {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut d = DemoSim::new(layout, world, 20261004);
        d.replay_boss(0.0, "normal");
        assert!(
            d.bm.lock().unwrap().is_some(),
            "the boss mode lives on the resource"
        );
        assert_eq!(d.m.mode, MatchMode::Boss);
        assert_eq!(d.m.actors.len(), 8, "one team-0 squad of 8");
        assert_eq!(d.m.duration, d.tuning.boss.mode.duration as f32);
        assert_eq!(d.bm.lock().unwrap().as_ref().unwrap().boss.max_hp, 93_600.0);
        assert_eq!(d.bm.lock().unwrap().as_ref().unwrap().boss.hp, 93_600.0);
        for _ in 0..1200 {
            d.step_once();
        }
        let bmg = d.bm.lock().unwrap();
        let bm = bmg.as_ref().unwrap();
        assert!(
            (f64::from(bm.boss.bt) - 20.0).abs() < 0.01,
            "boss clock: {}",
            bm.boss.bt
        );
        assert!(
            d.boss_events
                .iter()
                .any(|e| matches!(e, BossEvent::Move(_) | BossEvent::Hp { .. })),
            "the Move/Hp event stream must reach the app"
        );
        assert!(
            bm.boss.hp < bm.boss.max_hp,
            "weapon fire must reach the boss"
        );
    }

    #[test]
    fn demo_boss_hp_tracks_the_difficulty_tier() {
        // JS boss.js L18: hpDiff = {easy 0.75, normal 1, hard 1.3}.
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut d = DemoSim::new(layout, world, 20261004);
        d.replay_boss(0.0, "easy");
        assert_eq!(d.bm.lock().unwrap().as_ref().unwrap().boss.max_hp, 70_200.0);
        d.replay_boss(0.0, "hard");
        assert_eq!(
            d.bm.lock().unwrap().as_ref().unwrap().boss.max_hp,
            121_680.0
        );
        assert!(d.boss_events.is_empty(), "a fresh match resets the stream");
    }

    #[test]
    fn demo_replay_leaves_the_boss_mode_behind() {
        // The turf path must not keep a live BossMode: `replay` rebuilds the
        // match plain (a `restart` would carry `boss_rules` over).
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let mut d = DemoSim::new(layout, world, 20261004);
        d.replay_boss(0.0, "normal");
        d.step_once();
        d.replay(90.0);
        assert!(d.bm.lock().unwrap().is_none());
        assert!(d.boss_events.is_empty());
        assert_eq!(d.m.mode, MatchMode::Turf);
        d.step_once();
        assert!(d.boss_events.is_empty(), "turf raises no boss events");
    }
}

// Task 52 batch D1: structural smoke tests for the ported level shader text.
// Weak anchors on purpose - D2 wiring + review will strengthen them.
#[cfg(test)]
mod level_wgsl_tests {
    const LEVEL: &str = include_str!("level.wgsl");

    fn code_only(src: &str) -> String {
        src.lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn level_wgsl_braces_and_parens_balance() {
        let code = code_only(LEVEL);
        for (open, close) in [('{', '}'), ('(', ')')] {
            let mut depth = 0i32;
            for c in code.chars() {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                }
                assert!(depth >= 0, "unbalanced '{open}{close}' in level.wgsl");
            }
            assert_eq!(depth, 0, "unclosed '{open}' in level.wgsl");
        }
    }

    #[test]
    fn level_wgsl_binding_contract_complete() {
        // Header contract: bindings 20..=42 (atlas/sampler follow InkExt 20/21).
        for b in 20..=42u32 {
            assert!(
                LEVEL.contains(&format!("@binding({b})")),
                "binding {b} missing from level.wgsl"
            );
        }
        for name in [
            "ink_atlas",
            "t_albedo",
            "t_normal",
            "t_orm",
            "mural_atlas",
            "uTL",
            "uTLt",
            "uTLs",
            "uRip",
            "uRipP",
            "uWake",
            "uWakeB",
            "uSwimH",
            "uSwimF",
            "uLamps",
            "uMurA",
            "uMurB",
            "uMurC",
            "LevelParams",
            "BatchParams",
        ] {
            assert!(
                LEVEL.contains(name),
                "binding-contract name {name} missing from level.wgsl"
            );
        }
    }

    #[test]
    fn level_wgsl_uniform_table_parity() {
        // Names ≡ JS uniforms (camelCase -> snake_case), incl. deviation ③ batch params.
        for name in [
            "u_ink_clock",
            "u_time",
            "u_ppm",
            "u_atlas_px",
            "u_ink_glow",
            "u_gel",
            "u_tex_size",
            "u_ao",
            "u_lamp_n",
            "u_lamp_col",
            "u_team_a",
            "u_team_b",
            "u_see_on",
            "u_see_feet",
            "u_see_a2c",
            "u_grate",
            "u_sun_dir",
            "u_pattern",
            "u_wet_base",
            "u_fsz",
            "u_tan",
            "u_mur",
        ] {
            assert!(
                LEVEL.contains(name),
                "JS-parity uniform {name} missing from level.wgsl"
            );
        }
    }

    #[test]
    fn level_wgsl_array_lengths_match_js_constants() {
        // INK_RIPPLES = 24 (inkShading.js L22) -> uRip + uRipP
        assert_eq!(LEVEL.matches("array<vec4<f32>, 24u>").count(), 2);
        // TL_SLOTS = 50 (levelMaterial.js L101) -> uTL/uTLt/uTLs
        assert_eq!(LEVEL.matches("array<vec4<f32>, 50u>").count(), 3);
        // wake pool: 4 swimmers x 12 nodes (L174) + per-swimmer vec4 tables
        assert_eq!(LEVEL.matches("array<vec4<f32>, 48u>").count(), 1);
        assert_eq!(LEVEL.matches("array<vec4<f32>, 4u>").count(), 3);
        // MAX_LAMPS = 12 (L11), MUR = 12 (L106)
        assert_eq!(LEVEL.matches("array<vec3<f32>, 12u>").count(), 1);
        // uMurC is vec2[12] in JS, vec4-padded for WGSL uniform stride.
        assert_eq!(LEVEL.matches("array<vec4<f32>, 12u>").count(), 3);
    }

    #[test]
    fn level_wgsl_key_functions_present() {
        for f in [
            "fn texlib_sample",
            "fn texlib_tap",
            "fn texlib_tri_grid",
            "fn texlib_detail",
            "fn texlib_specular_aa",
            "fn texlib_macro",
            "fn texlib_coverage",
            "fn texlib_perturb_normal",
            "fn texlib_tangent_frame",
            "fn ink_bspl",
            "fn paint_at",
            "fn paint_cubic",
            "fn perturb_ink",
            "fn fragment(",
        ] {
            assert!(LEVEL.contains(f), "missing function anchor {f:?}");
        }
    }

    #[test]
    fn level_wgsl_all_nine_ink_chunks_marked() {
        // inkShading.js chunks L49/L98/L189/L194/L221/L257/L261/L282/L303.
        for chunk in [
            "INK_PARS",
            "INK_COLOR",
            "INK_ROUGH",
            "INK_GEL",
            "INK_SLOPE",
            "INK_EMISSIVE",
            "INK_LIGHTS",
            "INK_LIGHT_MAPS",
            "INK_SHADE",
        ] {
            assert!(LEVEL.contains(chunk), "chunk marker {chunk} missing");
        }
        // Toksvig modulation + sym bit-mask mirror/transpose ported (texlib.js L1250/L1241).
        assert!(LEVEL.contains("Toksvig"));
        assert!(LEVEL.contains("mirror-u"));
        // Deviation declarations ②③④⑤ (spec.md D-T52-7) documented in the header.
        for dev in ["②", "③", "④", "⑤"] {
            assert!(LEVEL.contains(dev), "deviation {dev} not declared");
        }
    }
}
