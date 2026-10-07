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

use std::sync::OnceLock;

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use inkwave_sim::actor::FIXED_DT;
use inkwave_sim::actor::{ActorInput, SimWorld as ActorSimWorld};
use inkwave_sim::bot::{Bot, BotCtx};
use inkwave_sim::collision::CollisionWorld;
use inkwave_sim::geometry::StageLayout;
use inkwave_sim::ink_atlas::InkAtlas;
use inkwave_sim::match_::{Match, Phase};
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
pub fn ink_upload(
    time: Res<Time>,
    mut atlas: ResMut<InkAtlasRes>,
    mut demo: ResMut<DemoSim>,
    mut images: ResMut<Assets<Image>>,
    image: Res<InkImage>,
    pc: Res<crate::input::PlayerControls>,
) {
    if pc.paused {
        // Esc pause freezes the match (camera and atlas stay as-is).
        return;
    }
    demo.player = Some((pc.intent, pc.yaw, pc.pitch));
    demo.step_and_drain(&mut atlas, time.delta_secs().min(0.1));
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
    /// Local player override for slot 0: (intent, aim_yaw, aim_pitch).
    /// `Some` replaces the easy-bot brain for that slot (Task 12 input
    /// wiring); `None` keeps the demo fully bot-driven (headless / TR-11).
    pub player: Option<(ActorInput, f32, f32)>,
    /// Seed the roster/bots were built with (kept so `replay` can rebuild
    /// the bot brains fresh, matching the first match).
    seed: u64,
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
            seed,
        }
    }

    /// Advance the match at the fixed 60 Hz step (frame-rate independent) and
    /// replay every recorded splat onto the ink atlas.
    fn step_and_drain(&mut self, atlas: &mut InkAtlasRes, dt: f32) {
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= FIXED_DT && steps < 4 {
            self.acc -= FIXED_DT;
            self.step_once();
            steps += 1;
        }
        if self.m.paint.version < self.last_grid_version {
            // Match::restart rebuilt the gameplay grid: wipe the bitmap so
            // both views stay in lock-step.
            atlas.atlas.clear();
        }
        self.last_grid_version = self.m.paint.version;
        let stream = self.m.paint.drain_ink();
        self.splats += stream.len() as u64;
        for s in stream {
            atlas
                .atlas
                .splat(&self.world, s.center, s.radius, s.team, &s.opts);
        }
    }

    fn step_once(&mut self) {
        let t = &self.tuning;
        let sim = ActorSimWorld::new(&self.world, &self.layout);
        let n = self.m.actors.len();
        if self.m.phase == Phase::Active {
            let snap = self.m.actors.clone();
            for (i, b) in self.bots.iter().enumerate() {
                self.mate_goals[i] = b.goal_node();
            }
            for i in 0..n {
                if i == 0
                    && let Some((intent, yaw, pitch)) = self.player
                {
                    // local player drives slot 0: the bot brain stands down
                    // and the aim angles come from the mouse look.
                    self.inputs[0] = intent;
                    self.m.actors[0].aim_yaw = yaw;
                    self.m.actors[0].aim_pitch = pitch;
                    continue;
                }
                let ctx = BotCtx {
                    actors: &snap,
                    nav: &self.nav,
                    paint: &self.m.paint,
                    world: &sim,
                    t: &t.player,
                    w: &t.spritzer,
                    mate_goals: &self.mate_goals,
                };
                self.inputs[i] = self.bots[i].step(FIXED_DT, &mut self.m.actors[i], &ctx);
            }
        } else {
            for i in 0..n {
                self.inputs[i] = self.bots[i].hold(&self.m.actors[i]);
            }
        }
        self.m
            .step(FIXED_DT, &self.inputs, &sim, &t.player, &t.spritzer);
        // Task 13: no auto-restart — the UI shows the results screen when the
        // match reaches `Phase::End` and replays on demand (`replay`).
    }

    /// Fresh match of `duration` seconds (menu Play / results Restart).
    /// Rebuilds the roster and the bot brains at the requested clock and
    /// re-enters the intro; the atlas bitmap follows via the grid-version
    /// rollback in [`step_and_drain`].
    pub fn replay(&mut self, duration: f32) {
        let sim = ActorSimWorld::new(&self.world, &self.layout);
        self.m.restart(&sim, &self.tuning.player);
        self.m.duration = duration;
        self.m.time = duration;
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
}
