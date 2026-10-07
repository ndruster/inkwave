//! Task 12 — placeholder character rendering driven by sim `Actor` state.
//!
//! Each of the 8 roster slots gets an entity tree:
//!   root (position / yaw / lean / alive-hiding / invuln blink)
//!     ├─ kid parts:   capsule body + sphere head + hair slab + muzzle flash
//!     └─ squid parts: ground-hugging spindle (capsule on its side) + head
//!
//! Form, facing, movement lean, respawn hiding and spawn-invulnerability
//! blinking all come straight from the sim state (`Actor` fields), matching
//! upstream `actor.js` / `character.js` behaviour at placeholder fidelity:
//!   * `!alive` → root hidden and every part hidden (Bevy 0.19 `Visible`
//!     overrides a Hidden parent, so the death hiding is applied per part)
//!   * paused → freeze-frame: blink stops, muzzle flash forced off, the rest
//!     of the scene keeps rendering as-is
//!   * `invuln > 0` → team material alpha blinks (spawn protection, upstream
//!     `PLAYER.spawnInvuln` = 1.6 s)
//!   * `form` → only the matching part subtree stays visible
//!   * horizontal speed → forward lean (upstream character.js lean is
//!     spring-damped; M1 uses a direct clamp — declared simplification)
//!   * `fire_gate.fire` → muzzle flash visible while the trigger is held
//!     (Spritzer muzzle-flash placeholder)

use bevy::prelude::*;
use inkwave_sim::actor::Form;

use crate::ink_render::DemoSim;

/// Root marker of one actor's visual tree (slot indexes `Match::actors`).
#[derive(Component)]
pub struct ActorRoot {
    pub slot: usize,
}

/// Which visual part an entity is, for the per-frame visibility switch.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum ActorPart {
    Kid,
    Squid,
    /// Spritzer muzzle flash (kid form + trigger held).
    Muzzle,
}

/// All materials of one actor the invuln blink dims (team-coloured body/head/
/// squid/gun parts plus the darkened hair).
#[derive(Component)]
pub struct ActorMat(pub Vec<Handle<StandardMaterial>>);

/// Team colours (config.js `teamHex`: Alpha #ff8a14, Bravo #2f5bff) — the
/// same values the spawn barriers use in `world.rs`.
#[must_use]
pub fn team_color(team: usize) -> Color {
    if team == 0 {
        Color::srgb(1.0, 0.541, 0.078)
    } else {
        Color::srgb(0.184, 0.357, 1.0)
    }
}

/// Blink window for spawn invulnerability: upstream reads as a fast strobe;
/// M1 uses a 9 Hz full-alpha swing in [0.30, 0.95].
#[must_use]
pub fn blink_alpha(t: f32) -> f32 {
    0.625 + 0.325 * (t * std::f32::consts::TAU * 9.0).sin()
}

/// Forward lean from horizontal speed (m/s): up to 0.28 rad at `run_speed`.
#[must_use]
pub fn lean_from_speed(speed: f32, run_speed: f32) -> f32 {
    (speed / run_speed.max(0.001)).min(1.0) * 0.28
}

/// Spawn the 8-slot visual trees (kid + squid subtrees, muzzle flash).
pub fn spawn_actors(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    demo: Res<DemoSim>,
) {
    // Shared geometry (one mesh per part kind, reused across all slots).
    let kid_body = meshes.add(Capsule3d::new(0.25, 0.45));
    let head = meshes.add(Sphere::new(0.32));
    let hair = meshes.add(Cuboid::new(0.62, 0.20, 0.62));
    let gun = meshes.add(Cuboid::new(0.10, 0.14, 0.55));
    let squid_body = meshes.add(Capsule3d::new(0.22, 0.55));
    let squid_head = meshes.add(Sphere::new(0.26));
    let muzzle = meshes.add(Sphere::new(0.11));

    for (slot, a) in demo.m.actors.iter().enumerate() {
        let col = team_color(a.team);
        let mat = materials.add(StandardMaterial {
            base_color: col,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.6,
            ..default()
        });
        // Hair is a darkened team colour.
        let hair_mat = materials.add(StandardMaterial {
            base_color: col.mix(&Color::srgb(0.12, 0.12, 0.14), 0.55),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.8,
            ..default()
        });
        let flash_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.97, 0.85),
            emissive: LinearRgba::new(3.0, 2.4, 1.2, 1.0),
            unlit: true,
            ..default()
        });

        let root = commands
            .spawn((
                ActorRoot { slot },
                ActorMat(vec![mat.clone(), hair_mat.clone()]),
                Visibility::Visible,
                Transform::default(),
            ))
            .id();

        // ---- kid parts (face +Z, the sim yaw convention) ----
        let kid_parts = [
            (
                kid_body.clone(),
                mat.clone(),
                Vec3::new(0.0, 0.95, 0.0),
                ActorPart::Kid,
            ),
            (
                head.clone(),
                mat.clone(),
                Vec3::new(0.0, 1.62, 0.0),
                ActorPart::Kid,
            ),
            (
                hair.clone(),
                hair_mat,
                Vec3::new(0.0, 1.86, 0.0),
                ActorPart::Kid,
            ),
            (
                gun.clone(),
                mat.clone(),
                Vec3::new(0.34, 1.05, 0.38),
                ActorPart::Kid,
            ),
            (
                muzzle.clone(),
                flash_mat.clone(),
                Vec3::new(0.34, 1.15, 0.66),
                ActorPart::Muzzle,
            ),
        ];
        for (mesh, m, pos, part) in kid_parts {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(m),
                part,
                ActorRoot { slot },
                Visibility::default(),
                Transform::from_translation(pos),
                ChildOf(root),
            ));
        }

        // ---- squid parts (spindle hugging the ink surface) ----
        commands.spawn((
            Mesh3d(squid_body.clone()),
            MeshMaterial3d(mat.clone()),
            ActorPart::Squid,
            ActorRoot { slot },
            Visibility::default(),
            Transform {
                translation: Vec3::new(0.0, 0.26, 0.0),
                // Bevy capsule axis is +Y; rotate onto +Z so length runs
                // along the facing.
                rotation: Quat::from_rotation_x(90_f32.to_radians()),
                ..default()
            },
            ChildOf(root),
        ));
        commands.spawn((
            Mesh3d(squid_head.clone()),
            MeshMaterial3d(mat),
            ActorPart::Squid,
            ActorRoot { slot },
            Visibility::default(),
            Transform::from_translation(Vec3::new(0.0, 0.28, 0.72)),
            ChildOf(root),
        ));
    }
}

/// Push sim state onto the visual trees every render frame.
pub fn sync_actors(
    demo: Res<DemoSim>,
    time: Res<Time>,
    pc: Res<crate::input::PlayerControls>,
    mut roots: Query<(&ActorRoot, &mut Visibility, &mut Transform, &ActorMat), Without<ActorPart>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut parts: Query<(&ActorRoot, &ActorPart, &mut Visibility)>,
) {
    let t = time.elapsed_secs();
    let paused = pc.paused;
    for (root, mut vis, mut tf, actor_mat) in roots.iter_mut() {
        let Some(a) = demo.m.actors.get(root.slot) else {
            continue;
        };
        // respawn hiding (upstream hides the character until it re-spawns)
        *vis = if a.alive {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        // glam 0.30 (sim) vs 0.32 (Bevy): bridge via arrays.
        tf.translation = Vec3::from_array(a.pos.to_array());
        tf.translation.y += a.smooth_y();

        let speed = Vec2::new(a.vel.x, a.vel.z).length();
        let lean = lean_from_speed(speed, demo.tuning.player.run_speed);
        let pitch_lean = match a.form {
            Form::Kid => 0.0,
            // a swimming squid points along its aim pitch
            Form::Squid => -a.aim_pitch.clamp(-0.9, 0.9),
        };
        tf.rotation = Quat::from_rotation_y(a.yaw) * Quat::from_rotation_x(lean + pitch_lean);

        // spawn-invulnerability blink on every material of the slot
        let alpha = if a.invuln > 0.0 && !paused {
            blink_alpha(t + root.slot as f32 * 0.7)
        } else {
            1.0
        };
        for h in &actor_mat.0 {
            if let Some(mut m) = mats.get_mut(h) {
                m.base_color.set_alpha(alpha);
            }
        }
    }

    // part visibility: form switch + muzzle flash (kid form, trigger held)
    for (root, part, mut vis) in parts.iter_mut() {
        let Some(a) = demo.m.actors.get(root.slot) else {
            *vis = Visibility::Hidden;
            continue;
        };
        // Bevy 0.19: `Visibility::Visible` is unconditional and overrides a
        // Hidden parent, so the death hiding must be applied per part too.
        // Pause is a freeze-frame: only the muzzle flash is forced off.
        *vis = if !a.alive {
            Visibility::Hidden
        } else {
            match part {
                ActorPart::Kid if a.form == Form::Kid => Visibility::Visible,
                ActorPart::Squid if a.form == Form::Squid => Visibility::Visible,
                ActorPart::Muzzle if a.form == Form::Kid && a.fire_gate.fire && !paused => {
                    Visibility::Visible
                }
                _ => Visibility::Hidden,
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_colors_match_upstream_hex() {
        // #ff8a14 / #2f5bff in sRGB components.
        let a = team_color(0).to_srgba();
        assert!((a.red - 1.0).abs() < 0.01 && (a.green - 0.541).abs() < 0.01);
        let b = team_color(1).to_srgba();
        assert!((b.red - 0.184).abs() < 0.01 && (b.blue - 1.0).abs() < 0.01);
    }

    #[test]
    fn blink_stays_in_band_and_swings() {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..200 {
            let v = blink_alpha(i as f32 * 0.005);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo >= 0.29 && hi <= 0.96, "band [{lo},{hi}]");
        assert!(hi - lo > 0.5, "blink must actually swing");
    }

    /// Render-layer integration: spawn the 8-slot trees and drive them from a
    /// live demo match, asserting the form switch / alive hiding actually flip
    /// part visibilities (the part of Task 12 most likely to get an ECS query
    /// or ChildOf wiring wrong).
    #[test]
    fn spawn_and_sync_builds_and_switches_visual_trees() {
        use crate::ink_render::{DemoSim, InkAtlasRes};
        use crate::input::PlayerControls;
        use bevy::asset::{AssetApp, AssetPlugin};
        use bevy::transform::TransformPlugin;
        use inkwave_sim::collision::CollisionWorld;
        use inkwave_sim::geometry::StageLayout;

        let layout: StageLayout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout.clone(), world.clone(), 20261004);
        let atlas = InkAtlasRes::new(&world);

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), TransformPlugin))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(demo)
            .insert_resource(atlas)
            .insert_resource(PlayerControls::default());
        app.add_systems(Startup, spawn_actors);
        app.add_systems(Update, sync_actors);
        app.update(); // Startup: spawn_actors

        // 8 roots + 8 * (4 kid + 1 muzzle + 2 squid) = 8 + 56 entities.
        let roots = app
            .world_mut()
            .query_filtered::<&ActorRoot, Without<ActorPart>>()
            .iter(app.world())
            .count();
        assert_eq!(roots, 8, "one root per roster slot");
        let parts = app
            .world_mut()
            .query::<&ActorPart>()
            .iter(app.world())
            .count();
        assert_eq!(parts, 56, "7 parts per slot");

        // Force slot 0 into Squid to exercise the form switch (slot-exact).
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].form = Form::Squid;
        app.update();
        {
            let mut q = app
                .world_mut()
                .query::<(&ActorRoot, &ActorPart, &Visibility)>();
            let (mut kid0_vis, mut squid0_vis) = (0, 0);
            for (r, part, vis) in q.iter(app.world()) {
                if r.slot != 0 {
                    continue;
                }
                if *vis != Visibility::Visible {
                    continue;
                }
                match part {
                    ActorPart::Kid | ActorPart::Muzzle => kid0_vis += 1,
                    ActorPart::Squid => squid0_vis += 1,
                }
            }
            assert_eq!(kid0_vis, 0, "slot0 kid parts must hide in Squid form");
            assert_eq!(squid0_vis, 2, "slot0 squid parts visible");
        }

        // Death must hide the PARTS too (Bevy 0.19 `Visible` overrides a
        // Hidden parent — review blocker regression guard).
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].alive = false;
        app.update();
        {
            let mut rq = app
                .world_mut()
                .query_filtered::<(&ActorRoot, &Visibility), Without<ActorPart>>();
            let mut hidden_roots = 0;
            for (r, v) in rq.iter(app.world()) {
                if r.slot == 0 && *v == Visibility::Hidden {
                    hidden_roots += 1;
                }
            }
            assert_eq!(hidden_roots, 1, "dead slot0 root hidden");
            let mut q = app
                .world_mut()
                .query::<(&ActorRoot, &ActorPart, &Visibility)>();
            let visible_parts = q
                .iter(app.world())
                .filter(|(r, _, v)| r.slot == 0 && *v == Visibility::Visible)
                .count();
            assert_eq!(visible_parts, 0, "dead slot0 parts must all hide");
        }

        // Pause is a freeze-frame: form parts stay visible, only the muzzle
        // flash is forced off and the blink stops.
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].alive = true;
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].form = Form::Kid;
        app.world_mut().resource_mut::<DemoSim>().m.actors[0]
            .fire_gate
            .fire = true;
        app.world_mut().resource_mut::<PlayerControls>().paused = true;
        app.update();
        {
            let mut q = app
                .world_mut()
                .query::<(&ActorRoot, &ActorPart, &Visibility)>();
            let (mut kid0_vis, mut muzzle0_vis) = (0, 0);
            for (r, part, vis) in q.iter(app.world()) {
                if r.slot != 0 || *vis != Visibility::Visible {
                    continue;
                }
                match part {
                    ActorPart::Kid => kid0_vis += 1,
                    ActorPart::Muzzle => muzzle0_vis += 1,
                    ActorPart::Squid => {}
                }
            }
            assert_eq!(kid0_vis, 4, "paused: kid form parts stay visible");
            assert_eq!(muzzle0_vis, 0, "paused: muzzle flash forced off");
        }
    }

    #[test]
    fn lean_clamps_at_run_speed() {
        assert_eq!(lean_from_speed(0.0, 6.0), 0.0);
        assert!((lean_from_speed(3.0, 6.0) - 0.14).abs() < 1e-6);
        assert_eq!(lean_from_speed(50.0, 6.0), 0.28);
    }
}
