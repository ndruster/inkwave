//! Task 12 → Task 54 batch C2 — character rendering driven by sim `Actor`
//! state, now through the real IKC1 rig tree instead of the placeholder
//! blocks.
//!
//! Each roster slot gets a [`crate::char::rig::spawn_char_rig`] tree
//! (root → kid → bones → parts, + weapon frame nodes, ≡ char.json manifest);
//! [`sync_actors`] keeps the engine-owned root motion (position + yaw,
//! ≡ actor.js L1015-1017) and the death hiding, while the per-frame pose /
//! material / visibility work is driven by
//! [`crate::char::bridge::char_pose_bridge`] (wired right after this system
//! in `world.rs`).
//!   * `!alive` → root hidden and every rig entity hidden (Bevy 0.19
//!     `Visible` overrides a Hidden parent, so the death hiding is applied
//!     per part — the bridge repeats it on every part/kid/model node)
//!   * paused → freeze-frame: the bridge skips `CharAnim::update` and keeps
//!     the last transforms (≡ the JS pause halting the anim step)
//!   * `invuln > 0` → spawn-protection blink runs through the CPU
//!     `_updateMaterials` pass on the per-character cloned buckets
//!     (D-T54-5①), no longer the placeholder `ActorMat` strobe.

use bevy::prelude::*;

use crate::char::rig::{self, RigSpawnOpts};
use crate::ink_render::DemoSim;
use inkwave_sim::sub::SubKind;

/// Root marker of one actor's visual tree (slot indexes `Match::actors`).
#[derive(Component)]
pub struct ActorRoot {
    pub slot: usize,
}

/// Legacy placeholder-part marker — kept so the `sync_actors` /
/// `sync_actor_count` query signatures (world.rs L707-708 wiring) stay
/// pinned; the rig tree no longer spawns any.
#[allow(dead_code)] // signature-pinned: the variants are no longer constructed
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum ActorPart {
    Kid,
    Squid,
    /// Spritzer muzzle flash (kid form + trigger held).
    Muzzle,
}

/// Legacy placeholder-material carrier — the rig buckets live on the bridge
/// clones now; kept for the pinned `sync_actors` signature.
#[allow(dead_code)] // signature-pinned: the handle list is no longer read
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
/// kept for the placeholder-era helper tests + the bridge's fallback.
#[allow(dead_code)] // helper-test anchor; the rig blink runs through the bridge
#[must_use]
pub fn blink_alpha(t: f32) -> f32 {
    0.625 + 0.325 * (t * std::f32::consts::TAU * 9.0).sin()
}

/// Forward lean from horizontal speed (m/s): up to 0.28 rad at `run_speed`
/// (placeholder-era helper; the rig lean now comes from `CharAnim`).
#[allow(dead_code)] // helper-test anchor; the rig lean runs through CharAnim
#[must_use]
pub fn lean_from_speed(speed: f32, run_speed: f32) -> f32 {
    (speed / run_speed.max(0.001)).min(1.0) * 0.28
}

/// Spawn the roster rig trees from the char archive + manifest (≡ CONTRACTS
/// §1 constructor: tier "game", held weapon from `Match::kinds`, bomb sub
/// from `Actor::sub`). Missing assets degrade to no characters (the bridge
/// guards the same way).
pub fn spawn_actors(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    demo: Res<DemoSim>,
) {
    let (Some(archive), Some(manifest)) = (rig::load_archive(), rig::manifest()) else {
        return;
    };
    for (slot, a) in demo.m.actors.iter().enumerate() {
        let kind = demo.m.kinds.get(slot).map(|k| k.id_str());
        spawn_slot_rig(
            &mut commands,
            &mut meshes,
            &mut materials,
            &archive,
            &manifest,
            slot,
            a,
            kind,
        );
    }
}

/// One slot's rig spawn + the legacy root markers the pinned `world.rs`
/// queries key on.
// All eight inputs are distinct borrows the Bevy resource split hands us
// (`Commands` + two asset collections + the two baked char assets + slot
// index + sim actor + weapon kind); bundling them would only hide the split.
#[allow(clippy::too_many_arguments)]
fn spawn_slot_rig(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    archive: &rig::CharArchive,
    manifest: &rig::CharManifest,
    slot: usize,
    a: &inkwave_sim::actor::Actor,
    kind: Option<&str>,
) {
    let opts = RigSpawnOpts {
        tier: "game",
        weapon: kind,
        bomb: a.sub == SubKind::Bomb,
    };
    let built = rig::spawn_char_rig(commands, meshes, materials, archive, manifest, &opts);
    commands.entity(built.root).insert((
        ActorRoot { slot },
        ActorMat(Vec::new()),
        Visibility::Visible,
    ));
}

/// Task 24: the net match swaps `demo.m.actors` for the session roster
/// (JS tears the whole match down and builds a new one — `startNetMatch`
/// L776-794). When the count no longer matches the visual trees, rebuild
/// them all. Runs before `sync_actors` so the same frame's sync sees the
/// fresh entities.
pub fn sync_actor_count(
    mut commands: Commands,
    demo: Res<DemoSim>,
    roots: Query<Entity, (With<ActorRoot>, Without<ActorPart>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let roots: Vec<Entity> = roots.iter().collect();
    if roots.len() == demo.m.actors.len() {
        return;
    }
    for e in roots {
        // Bevy 0.19: `despawn` removes the whole subtree (the rig trees
        // are deep).
        commands.entity(e).despawn();
    }
    let (Some(archive), Some(manifest)) = (rig::load_archive(), rig::manifest()) else {
        return;
    };
    for (slot, a) in demo.m.actors.iter().enumerate() {
        let kind = demo.m.kinds.get(slot).map(|k| k.id_str());
        spawn_slot_rig(
            &mut commands,
            &mut meshes,
            &mut materials,
            &archive,
            &manifest,
            slot,
            a,
            kind,
        );
    }
}

/// Push the engine-owned root motion onto the rig trees every render frame
/// (≡ actor.js L1015-1017: `root.position = pos + smoothY; root.rotation.y =
/// yaw` — the lean/pitch/form work moved into `CharAnim` + the bridge).
pub fn sync_actors(
    demo: Res<DemoSim>,
    _time: Res<Time>,
    _pc: Res<crate::input::PlayerControls>,
    mut roots: Query<(&ActorRoot, &mut Visibility, &mut Transform, &ActorMat), Without<ActorPart>>,
    _mats: ResMut<Assets<StandardMaterial>>,
    _parts: Query<(&ActorRoot, &ActorPart, &mut Visibility)>,
) {
    for (root, mut vis, mut tf, _actor_mat) in roots.iter_mut() {
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
        tf.rotation = Quat::from_rotation_y(a.yaw);
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

    #[test]
    fn lean_clamps_at_run_speed() {
        assert_eq!(lean_from_speed(0.0, 6.0), 0.0);
        assert!((lean_from_speed(3.0, 6.0) - 0.14).abs() < 1e-6);
        assert_eq!(lean_from_speed(50.0, 6.0), 0.28);
    }

    /// Render-layer integration: spawn the roster rig trees and drive the
    /// root sync, asserting the engine-owned root motion + death hiding
    /// still work on the new trees.
    #[test]
    fn spawn_and_sync_drive_rig_roots() {
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

        let roots = app
            .world_mut()
            .query_filtered::<&ActorRoot, Without<ActorPart>>()
            .iter(app.world())
            .count();
        assert_eq!(roots, 8, "one rig root per roster slot");
        let rigs = app
            .world_mut()
            .query_filtered::<(), With<crate::char::rig::CharRig>>()
            .iter(app.world())
            .count();
        assert_eq!(rigs, 8, "one CharRig per slot");

        // Root motion mirrors the sim (pos + smooth_y, yaw only).
        let slot0_y = {
            let mut q = app.world_mut().query::<(&ActorRoot, &Transform)>();
            q.iter(app.world())
                .find(|(r, _)| r.slot == 0)
                .map(|(_, t)| t.translation.y)
                .expect("slot 0 root transform")
        };
        let a = &app.world().resource::<DemoSim>().m.actors[0];
        assert!((slot0_y - (a.pos.y + a.smooth_y())).abs() < 1e-3);

        // Death hides the root (the bridge hides the parts per entity).
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].alive = false;
        app.update();
        let mut rq = app
            .world_mut()
            .query_filtered::<(&ActorRoot, &Visibility), Without<ActorPart>>();
        let hidden = rq
            .iter(app.world())
            .filter(|(r, v)| r.slot == 0 && *v == Visibility::Hidden)
            .count();
        assert_eq!(hidden, 1, "dead slot0 root hidden");
    }
}
