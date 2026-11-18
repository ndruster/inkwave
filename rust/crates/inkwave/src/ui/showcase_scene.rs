//! Task 56 batch C — Bevy wiring for the 3D showcase lobby (placeholder set).
//!
//! The state machine lives in [`super::showcase_state`]; this module owns the
//! runtime bridge:
//!   * `showcase_sync` consumes the same `ui::online::project()` view as the
//!     2D hub/lobby panel, so room code, roster, ready, weapon and the low-tier
//!     gate have exactly one source of truth (D-T56-⑥).
//!   * `showcase_scene_update` builds a placeholder alley set once — floor,
//!     side walls, dock, eight lineup marks and the ink pedestal — from the
//!     extracted `lobbyset.json`, and toggles its visibility from
//!     `ShowcaseState::full_frame`.
//!   * `showcase_camera_update` drives a REAL second camera (layer 1) with
//!     the extracted hub/line-up pose while the set is full-frame, instead of
//!     overriding the main camera (Task 61 removes the D-T56-⑦ placeholder).
//!     Every single-camera consumer now targets `MainCamera` explicitly.
//!
//! Upstream has no THREE.Layers numbers: the split is a separate Scene +
//! camera + render target (`showcase.js` L3042-3085). The Rust layer mapping
//! (world/FX/UI = 0, lobby set = 1) is registered as D-T61-1.
//! Materials, neon, steam, planar reflections, PMREM, nameplate anchors,
//! cross-dissolves and the character preview rig are GPU review items.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use crate::environment::EnvState;
use crate::net::OnlineSession;
use crate::world::{MainCamera, SHOWCASE_LAYER, ShowcaseCamera};

use super::showcase_state::{
    ShowcaseCue, ShowcaseState, alley_dims, camera_pose, hub_spot, pedestal, spots,
};
use super::{Screen, UiState, online, online_facts};

/// Task 61: the second camera's query (kept as an alias so clippy's
/// type-complexity lint does not obscure the system signature).
type ShowcaseCameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Projection,
        &'static mut Camera,
    ),
    (With<ShowcaseCamera>, Without<MainCamera>),
>;

/// Root of the placeholder alley set; its `Visibility` gates every child.
#[derive(Component)]
pub struct ShowcaseSetRoot;

/// The ink pedestal; its rotation is driven by the one-shot loadout cue.
#[derive(Component)]
pub struct ShowcasePedestal;

/// Build-once handles + the runtime latches consumed by the render layer.
#[derive(Resource, Default)]
pub struct ShowcaseScene {
    pub root: Option<Entity>,
    /// The layer-1 camera, spawned once with the set and toggled by
    /// `showcase_camera_update` (Task 61).
    pub camera: Option<Entity>,
    pub cue_serial: u32,
    /// First frame on an online screen seeds the state from the whole view
    /// instead of diffing against the default weapon (review P1-2).
    pub synced: bool,
    /// Whether `showcase_camera_update` currently owns near/far and must
    /// restore the world clip planes on the way out (review P1-1).
    pub camera_overridden: bool,
}

/// Placeholder one-shot rotation per cue (NOT a per-frame JS port — the real
/// hop/twirl/dip/dive paths are GPU visual review items, D-T56-①).
fn cue_yaw(cue: ShowcaseCue) -> f32 {
    match cue {
        ShowcaseCue::None => 0.0,
        ShowcaseCue::HopTwirl => std::f32::consts::TAU,
        ShowcaseCue::Pop => 0.35,
        ShowcaseCue::Dip => -0.35,
        ShowcaseCue::SwapChar => std::f32::consts::PI,
    }
}

/// TR-56.1 frame sync. Outside the online screens the state is hidden but the
/// cue/emote history stays intact, exactly like `Showcase.hide()` (L1234).
pub fn showcase_sync(
    st: Res<UiState>,
    env: Res<EnvState>,
    os: NonSend<OnlineSession>,
    mut state: ResMut<ShowcaseState>,
    mut scene: ResMut<ShowcaseScene>,
) {
    if !st.net_mode || !matches!(st.screen, Screen::OnlineHub | Screen::OnlineLobby) {
        scene.synced = false;
        if state.mode != super::showcase_state::ShowcaseMode::Hidden {
            state.hide();
        }
        return;
    }
    let view = online::project(&online_facts(&os));
    if scene.synced {
        // The returned cue is kept for callers/tests; the render layer
        // consumes the same one-shot through `replay_count` diffing.
        let _cue = state.sync_from_online_view(&view, env.params.tier);
    } else {
        // Seed from the full view: diffing a custom weapon against the
        // default "shooter" would fire a spurious HopTwirl on entry (P1-2).
        *state = ShowcaseState::from_online_view(&view, env.params.tier);
        scene.synced = true;
    }
}

/// Build the set once and gate it by `full_frame` (low tier never builds it).
#[allow(clippy::too_many_arguments)] // Bevy system injection
pub fn showcase_scene_update(
    state: Res<ShowcaseState>,
    mut scene: ResMut<ShowcaseScene>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut roots: Query<&mut Visibility, With<ShowcaseSetRoot>>,
    mut pedestals: Query<&mut Transform, With<ShowcasePedestal>>,
) {
    if state.full_frame && scene.root.is_none() {
        scene.root = Some(build_set(&mut commands, &mut meshes, &mut mats));
        // Task 61: the set gets its OWN camera on layer 1 (the main camera
        // only sees layer 0). Initial pose/near/far are the extracted values;
        // `showcase_camera_update` writes the live pose each frame.
        let pose = camera_pose(state.mode);
        let cam = commands
            .spawn((
                Camera3d::default(),
                ShowcaseCamera,
                RenderLayers::layer(SHOWCASE_LAYER),
                Projection::from(PerspectiveProjection {
                    fov: pose.fov.to_radians(),
                    near: pose.near,
                    far: pose.far,
                    ..default()
                }),
                Transform::default(),
            ))
            .id();
        commands.entity(cam).insert(Camera {
            is_active: false,
            ..default()
        });
        scene.camera = Some(cam);
    }
    if let Some(root) = scene.root {
        // The root is spawned Visible (full_frame implies the set is wanted),
        // so the deferred-Commands build frame has no invisible gap (P2-5).
        if let Ok(mut vis) = roots.get_mut(root) {
            *vis = if state.full_frame {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
    if state.replay_count != scene.cue_serial {
        scene.cue_serial = state.replay_count;
        let yaw = cue_yaw(state.last_cue);
        for mut tf in &mut pedestals {
            tf.rotation = Quat::from_rotation_y(yaw);
        }
    }
}

/// Task 61: drive the layer-1 showcase camera (never the main camera). While
/// `full_frame`, the extracted pose wins and the second camera becomes active;
/// on exit it is deactivated, so the main camera's transform / FOV / clip
/// planes are untouched (the old D-T56-⑦ restore hack is gone).
pub fn showcase_camera_update(
    state: Res<ShowcaseState>,
    mut scene: ResMut<ShowcaseScene>,
    mut cams: ShowcaseCameraQuery,
) {
    let Some(cam_entity) = scene.camera else {
        return;
    };
    let Ok((mut tf, mut proj, mut camera)) = cams.get_mut(cam_entity) else {
        return;
    };
    if !state.full_frame {
        camera.is_active = false;
        scene.camera_overridden = false;
        return;
    }
    let pose = camera_pose(state.mode);
    let pos = Vec3::from_array(pose.pos);
    let dir = Vec3::from_array(pose.target) - pos;
    if dir.length_squared() < 1e-8 {
        return;
    }
    tf.translation = pos;
    tf.look_to(Dir3::new_unchecked(dir.normalize()), Vec3::Y);
    if let Projection::Perspective(p) = &mut *proj {
        p.fov = pose.fov.to_radians();
        p.near = pose.near;
        p.far = pose.far;
    }
    camera.is_active = true;
    scene.camera_overridden = true;
}

/// Geometry is a placeholder shell: no atlas, no reflections, no shader work.
/// All dimensions come from the extracted document (never hand-copied).
fn build_set(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) -> Entity {
    let a = alley_dims();
    let p = pedestal();
    let (hub_pos, _) = hub_spot();
    let width = a.x_r - a.x_l;
    let depth = a.z_near - a.z_mouth;
    let mid_z = (a.z_near + a.z_mouth) * 0.5;

    let floor = meshes.add(Cuboid::new(width, 0.1, depth));
    let wall_l = meshes.add(Cuboid::new(0.2, a.h_l, depth));
    let wall_r = meshes.add(Cuboid::new(0.2, a.h_r, depth));
    let dock = meshes.add(Cuboid::new(
        a.dock[1] - a.dock[0],
        a.dock[4],
        a.dock[3] - a.dock[2],
    ));
    let mark = meshes.add(Cylinder::new(0.34, 0.06));
    let plate = meshes.add(Cylinder::new(p.slot_r, p.slot_top));

    // Blue-hour placeholder palette (D-T56-①: the JS material stack is not
    // ported). Team marks use the two team hues.
    let floor_mat = mats.add(StandardMaterial {
        base_color: Color::srgb(0.09, 0.11, 0.16),
        perceptual_roughness: 0.42,
        ..default()
    });
    let wall_mat = mats.add(StandardMaterial {
        base_color: Color::srgb(0.13, 0.16, 0.23),
        ..default()
    });
    let dock_mat = mats.add(StandardMaterial {
        base_color: Color::srgb(0.17, 0.19, 0.24),
        ..default()
    });
    let team0 = mats.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.85, 0.55),
        ..default()
    });
    let team1 = mats.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.42, 0.35),
        ..default()
    });
    let ink_mat = mats.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.22, 0.55),
        emissive: LinearRgba::rgb(0.02, 0.05, 0.14),
        ..default()
    });

    let root = commands
        .spawn((
            ShowcaseSetRoot,
            Visibility::Visible,
            RenderLayers::layer(SHOWCASE_LAYER),
            Transform::default(),
        ))
        .id();
    let child = |mesh: Handle<Mesh>, mat: Handle<StandardMaterial>, tf: Transform| {
        (
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            RenderLayers::layer(SHOWCASE_LAYER),
            tf,
            ChildOf(root),
        )
    };
    commands.spawn(child(
        floor,
        floor_mat,
        Transform::from_xyz((a.x_l + a.x_r) * 0.5, -0.05, mid_z),
    ));
    commands.spawn(child(
        wall_l,
        wall_mat.clone(),
        Transform::from_xyz(a.x_l, a.h_l * 0.5, mid_z),
    ));
    commands.spawn(child(
        wall_r,
        wall_mat,
        Transform::from_xyz(a.x_r, a.h_r * 0.5, mid_z),
    ));
    commands.spawn(child(
        dock,
        dock_mat,
        Transform::from_xyz(
            (a.dock[0] + a.dock[1]) * 0.5,
            a.dock[4] * 0.5,
            (a.dock[2] + a.dock[3]) * 0.5,
        ),
    ));
    for s in spots() {
        let mat = if s.team == 0 {
            team0.clone()
        } else {
            team1.clone()
        };
        commands.spawn(child(
            mark.clone(),
            mat,
            Transform {
                translation: Vec3::from_array(s.pos) + Vec3::Y * 0.03,
                rotation: Quat::from_rotation_y(s.yaw),
                ..default()
            },
        ));
    }
    commands.spawn((
        ShowcasePedestal,
        Mesh3d(plate),
        MeshMaterial3d(ink_mat),
        Transform::from_xyz(hub_pos[0], p.slot_top * 0.5, hub_pos[2]),
        ChildOf(root),
    ));
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bevy 0.19 `RenderLayers::bits()` is a `&[u64]` bitset, not an integer.
    fn has_layer(app: &App, e: Entity, bit: u64) -> bool {
        app.world()
            .get::<RenderLayers>(e)
            .is_some_and(|l| l.bits().first().is_some_and(|w| w & (1u64 << bit) != 0))
    }
    use crate::ui::showcase_state::ShowcaseMode;

    #[test]
    fn placeholder_geometry_uses_only_extracted_dimensions() {
        let a = alley_dims();
        let s = spots();
        assert_eq!(s.len(), 8);
        assert!(a.x_r > a.x_l && a.z_near > a.z_mouth);
        assert!(a.dock[4] > 0.0 && a.dock[3] > a.dock[2]);
        // Rival marks sit on the dock height; friendly marks on the asphalt.
        assert!(s[..4].iter().all(|m| m.pos[1] == 0.0 && m.team == 0));
        assert!(s[4..].iter().all(|m| m.pos[1] == a.dock[4] && m.team == 1));
        let p = pedestal();
        assert!(p.slot_r > 0.0 && p.slot_top > 0.0);
        // Values read from lobbySet.js L38-44, not recomputed from the doc.
        assert_eq!(
            [a.x_l, a.x_r, a.z_near, a.z_mouth],
            [-4.0, 4.0, 14.0, -34.0]
        );
        assert_eq!(a.dock, [-1.35, 4.0, -10.5, -1.9, 1.0]);
        assert_eq!((p.slot_r, p.slot_top), (0.7, 0.72));
        assert_eq!(camera_pose(ShowcaseMode::Lobby).pos, [-2.45, 2.25, 10.4]);
        assert_eq!(camera_pose(ShowcaseMode::Hub).pos, [-1.2, 1.1, 5.6]);
        for mode in [ShowcaseMode::Hub, ShowcaseMode::Lobby] {
            let c = camera_pose(mode);
            assert!(c.fov > 0.0 && c.near > 0.0 && c.far > c.near);
            assert!(Vec3::from_array(c.target).distance(Vec3::from_array(c.pos)) > 1e-3);
        }
    }

    // TR-56.1 runtime smoke: the headless branch does not load WorldPlugin, so
    // exercise the two Bevy systems (set build/gate + camera override) on a
    // minimal App without a GPU. `showcase_sync` is a thin wrapper over the
    // state-layer `sync_from_online_view` (covered by its own tests); the full
    // Update chain is checked by cargo check / clippy, not by this App.
    #[test]
    fn wired_systems_build_gate_and_drive_the_second_camera() {
        use bevy::asset::{AssetApp, AssetPlugin};

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>();
        app.init_resource::<ShowcaseScene>();
        app.insert_resource(ShowcaseState {
            mode: ShowcaseMode::Lobby,
            full_frame: true,
            ..Default::default()
        });
        app.add_systems(
            Update,
            (showcase_scene_update, showcase_camera_update).chain(),
        );
        // The gameplay camera is layer 0 and MUST NOT be touched by the
        // showcase systems (Task 61 removes the D-T56-⑦ override).
        let main_cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                MainCamera,
                RenderLayers::layer(crate::world::WORLD_LAYER),
                Projection::Perspective(PerspectiveProjection {
                    near: crate::world::CAM_NEAR,
                    far: crate::world::CAM_FAR,
                    ..default()
                }),
                Transform::from_translation(Vec3::new(7.0, 8.0, 9.0)),
            ))
            .id();

        app.update(); // build frame (Commands are deferred)
        app.update(); // visibility + camera writes see the spawned entities
        let scene = app.world().resource::<ShowcaseScene>();
        let root = scene.root.expect("placeholder set built");
        let second = scene.camera.expect("layer-1 camera spawned");
        assert_eq!(
            *app.world().get::<Visibility>(root).unwrap(),
            Visibility::Visible
        );
        // Layer split: main = layer 0 only, second = layer 1 only, set = layer1.
        assert!(has_layer(&app, main_cam, 0) && !has_layer(&app, main_cam, 1));
        assert!(has_layer(&app, second, 1) && !has_layer(&app, second, 0));
        assert!(has_layer(&app, root, 1) && !has_layer(&app, root, 0));
        assert!(app.world().get::<Camera>(second).unwrap().is_active);
        assert!(app.world().get::<Camera>(main_cam).unwrap().is_active);
        // Expected values are the JS literals (lobbySet.js L44), NOT a
        // recomputation through the implementation under test.
        let tf = app.world().get::<Transform>(second).unwrap();
        assert!((tf.translation - Vec3::new(-2.45, 2.25, 10.4)).length() < 1e-4);
        let Projection::Perspective(p) = app.world().get::<Projection>(second).unwrap() else {
            panic!("showcase camera is perspective");
        };
        assert!((p.fov - 32.0_f32.to_radians()).abs() < 1e-5);
        assert_eq!((p.near, p.far), (0.1, 140.0));
        // The main camera's own state is unchanged (no override hack).
        assert_eq!(
            app.world().get::<Transform>(main_cam).unwrap().translation,
            Vec3::new(7.0, 8.0, 9.0)
        );
        let Projection::Perspective(main_p) = app.world().get::<Projection>(main_cam).unwrap()
        else {
            panic!("main camera is perspective");
        };
        assert_eq!(
            (main_p.near, main_p.far),
            (crate::world::CAM_NEAR, crate::world::CAM_FAR)
        );

        // A one-shot cue must actually drive the pedestal transform.
        {
            let mut st = app.world_mut().resource_mut::<ShowcaseState>();
            st.replay_count = 1;
            st.last_cue = ShowcaseCue::Pop;
        }
        app.update();
        let ped_tf = app
            .world()
            .iter_entities()
            .find(|e| e.contains::<ShowcasePedestal>())
            .and_then(|e| e.get::<Transform>().copied())
            .expect("pedestal spawned");
        assert!(
            ped_tf
                .rotation
                .angle_between(Quat::from_rotation_y(0.35))
                .abs()
                < 1e-4,
            "cue drives the pedestal rotation"
        );

        // Leaving full-frame hides the set AND deactivates the second camera;
        // the main camera's clip planes were never touched.
        app.world_mut().resource_mut::<ShowcaseState>().full_frame = false;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(root).unwrap(),
            Visibility::Hidden
        );
        assert!(!app.world().get::<Camera>(second).unwrap().is_active);
        let Projection::Perspective(p) = app.world().get::<Projection>(main_cam).unwrap() else {
            panic!("main camera is perspective");
        };
        assert_eq!(p.near, crate::world::CAM_NEAR);
        assert_eq!(p.far, crate::world::CAM_FAR);
        assert!(!app.world().resource::<ShowcaseScene>().camera_overridden);
    }
}
