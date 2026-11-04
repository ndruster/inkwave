//! Task 46 batch C1 — the Boss Battle presentation placeholder tier.
//!
//! Three entity groups driven straight from the live [`DemoSim::bm`]
//! (the owned [`inkwave_sim::boss::BossMode`]):
//!   * **silhouette** — a body-block reading of the HULLBREAKER: the rusted
//!     container shell + two claw boxes + eye-stalk columns with spheres +
//!     a six-cylinder walking-leg array (JS `bossModel.js` L13-56 container
//!     shell, L124-143 claw / eye-stalk / leg primitives — the placeholder
//!     tier takes their bounding-box shapes, not the skinned rig);
//!   * **telegraph markers** — one flat marker per active hazard feature
//!     (slam target disc + shockwave bands, barrage landing discs + falling
//!     barrels, sweep danger fan + beam, charge lane quad, frenzy zone
//!     disc), every parameter a pure function of the move record through the
//!     sim geometry helpers (JS `bossHazards.js` L357-600 `HazardFX.draw`);
//!   * **crablets** — small shell blocks, pop swell + hide on death
//!     (JS `bossModel.js` L222-234).
//!
//! Declared placeholder simplifications (vs JS `HazardFX.draw`):
//!   * no floor-`reach()` clipping — rings / fans draw their full circle
//!     (JS L498/L531/L559 clip to the floor extent; the sim exposes
//!     `Boss::hz_reach` but it needs `&mut` for the per-move cache and the
//!     visual gain at placeholder fidelity is nil);
//!   * the sweep fan radius is the constant `HZ.beamLen` instead of the
//!     per-heading `beamLen(m, a)` wall raycast (JS L534);
//!   * shader-driven stripe / chevron animation is replaced by an alpha ramp
//!     (JS `LANE_FRAG` / `FAN_FRAG` / `RING_FRAG`);
//!   * the boss model's spring-damped animator (JS `bossAnim.js`) is replaced
//!     by direct exponential smoothing on a handful of scalars.
//!
//! Non-boss matches (`bm == None`) cost one mutex check and an early return.

use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use inkwave_sim::boss::BossSt;
use inkwave_sim::boss::hazards::{
    MoveId, barrel_pos, beam_angle, charge_dist, move_phase_at, rec_d, rec_id, rec_p_f64, rec_pts,
    rec_rings, rec_t0, ring_radius, ring_start,
};
use inkwave_sim::net::wire::JVal;
use inkwave_sim::tuning::BossHazardTuning;

use crate::ink_render::DemoSim;

/// The boss ink: team-1 colour `#2f5bff` (JS `bossModel.js` L29 reads
/// `G.teamColors[1]` as the default ink).
const BOSS_INK: Color = Color::srgb(0.184, 0.357, 1.0);
/// The hazard-marker red (JS `bossHazards.js` L414 `new THREE.Color(1, .3, .3)`).
const TELE_RED: Color = Color::srgb(1.0, 0.3, 0.3);
/// The act-phase marker tint: the same red pushed brighter (JS mixes the
/// colour toward white on the wave crest, L394).
const ACT_RED: Color = Color::srgb(1.0, 0.55, 0.5);
/// Lobbed-barrel rust (JS `bossHazards.js` L437 `0x8a4a32`).
const BARREL_RUST: Color = Color::srgb(0.541, 0.290, 0.196);
/// Container steel — the placeholder read of the corrugated hull
/// (JS `bossModelGeo.js` L384 CONTAINER bucket, rusted steel).
const SHELL_STEEL: Color = Color::srgb(0.30, 0.32, 0.36);

// ---------------------------------------------------------------------------
// Pure telegraph parameter helpers — the JS `HazardFX.draw` formulas
// (bossHazards.js L487-575), kept as free functions so the tests can pin
// them value-by-value against the JS literals.
// ---------------------------------------------------------------------------

/// JS `clamp` (core/ctx.js L27) in f64.
#[inline]
fn js_clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// JS `smoothstep` (core/ctx.js L30) in f64.
#[inline]
fn js_smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = js_clamp((x - a) / (b - a), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The slam target-mark fade-in `k` — JS L497 `clamp(u / tele, 0, 1)`.
#[must_use]
pub fn slam_mark_k(u: f64, tele: f64) -> f64 {
    js_clamp(u / tele, 0.0, 1.0)
}

/// Slam ring `k`'s band alpha — JS L504
/// `1 - smoothstep(ringMax - 4, ringMax, r)`.
#[must_use]
pub fn band_alpha(hz: &BossHazardTuning, r: f64) -> f64 {
    let m = f64::from(hz.ring_max);
    1.0 - js_smoothstep(m - 4.0, m, r)
}

/// Whether barrel `b`'s landing circle has appeared yet — JS L520
/// `u > Math.min(tele * 0.4, b[3] - 0.5)`.
#[must_use]
pub fn barrel_show(u: f64, tele: f64, throw: f64) -> bool {
    u > (tele * 0.4).min(throw - 0.5)
}

/// Barrel circle pulse `k` — JS L520
/// `clamp(1 - (land - u) / (barrelFlight + 0.5), 0, 1)` with `land =
/// throw + barrelFlight` (L517).
#[must_use]
pub fn barrel_mark_k(u: f64, throw: f64, hz: &BossHazardTuning) -> f64 {
    let land = throw + f64::from(hz.barrel_flight);
    js_clamp(
        1.0 - (land - u) / (f64::from(hz.barrel_flight) + 0.5),
        0.0,
        1.0,
    )
}

/// The sweep fan's `k` (danger-zone intensity) — JS L536
/// `u < tele ? u / tele : 1 - (u - tele) / act`.
#[must_use]
pub fn sweep_k(u: f64, tele: f64, act: f64) -> f64 {
    if u < tele {
        u / tele
    } else {
        1.0 - (u - tele) / act
    }
}

/// The charge lane's fade-in `k` — JS L551 `clamp(u / tele, 0, 1)`.
#[must_use]
pub fn charge_lane_k(u: f64, tele: f64) -> f64 {
    js_clamp(u / tele, 0.0, 1.0)
}

/// The charge lane's remaining length — JS L552
/// `Math.max(0.5, p.L + 4.2 - s)`.
#[must_use]
pub fn charge_lane_len(p_l: f64, s: f64) -> f64 {
    (p_l + 4.2 - s).max(0.5)
}

/// The frenzy zone's `k` — JS L561 `u < tele ? clamp(u / tele, 0, 1) : 1`.
#[must_use]
pub fn frenzy_k(u: f64, tele: f64) -> f64 {
    if u < tele {
        js_clamp(u / tele, 0.0, 1.0)
    } else {
        1.0
    }
}

/// The sweep fan's angular bounds — JS L531
/// `lo = Math.min(p.a0, p.a1), hi = Math.max(p.a0, p.a1)`.
#[must_use]
pub fn sweep_span(m: &JVal) -> (f64, f64) {
    let (a0, a1) = (rec_p_f64(m, "a0"), rec_p_f64(m, "a1"));
    (a0.min(a1), a0.max(a1))
}

// ---------------------------------------------------------------------------
// Flat marker meshes (built once, reused; all lie in the XZ plane at y = 0)
// ---------------------------------------------------------------------------

/// A triangle-list mesh in the XZ plane with an up normal (the markers are
/// unlit translucent decals; winding is irrelevant thanks to `double_sided`).
fn flat_mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    let normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
    let uvs = vec![[0.0f32, 0.0]; positions.len()];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A unit disc in the XZ plane (JS `fx.mark`'s target circle; scaled by the
/// radius per marker).
fn disc_mesh() -> Mesh {
    // JS `_fan` (bossHazards.js L472-485) with a0 = 0, a1 = 2π, r = 1.
    let n = 32usize;
    let mut pos = vec![[0.0f32, 0.0, 0.0]];
    let mut idx = Vec::new();
    for i in 0..=n {
        let a = TAU * (i as f64 / n as f64);
        pos.push([a.sin() as f32, 0.0, a.cos() as f32]);
        if i > 0 {
            idx.extend_from_slice(&[0, i as u32, (i + 1) as u32]);
        }
    }
    flat_mesh(pos, idx)
}

/// A flat shockwave band between radii `inner` and `outer` in the XZ plane —
/// the placeholder of JS's per-heading reach-clipped ring strip
/// (bossHazards.js L426-434); scaled by the ring radius, so the band width
/// grows with it (declared simplification).
fn band_mesh(inner: f32, outer: f32) -> Mesh {
    let n = 48usize;
    let mut pos = Vec::with_capacity(n * 4);
    let mut idx = Vec::with_capacity(n * 6);
    for k in 0..n {
        // JS L429: a0 = k/n * 2π, a1 = (k+1)/n * 2π; 4 verts per segment.
        let a0 = TAU * (k as f64 / n as f64);
        let a1 = TAU * ((k + 1) as f64 / n as f64);
        let o = (k * 4) as u32;
        pos.push([
            (a0.sin() * f64::from(inner)) as f32,
            0.0,
            (a0.cos() * f64::from(inner)) as f32,
        ]);
        pos.push([
            (a0.sin() * f64::from(outer)) as f32,
            0.0,
            (a0.cos() * f64::from(outer)) as f32,
        ]);
        pos.push([
            (a1.sin() * f64::from(inner)) as f32,
            0.0,
            (a1.cos() * f64::from(inner)) as f32,
        ]);
        pos.push([
            (a1.sin() * f64::from(outer)) as f32,
            0.0,
            (a1.cos() * f64::from(outer)) as f32,
        ]);
        // JS L431: idx.push(o, o+1, o+2, o+2, o+1, o+3).
        idx.extend_from_slice(&[o, o + 1, o + 2, o + 2, o + 1, o + 3]);
    }
    flat_mesh(pos, idx)
}

/// A unit quad spanning local z 0..1, x ±0.5 — the charge-lane / beam
/// template (JS L421-422: `PlaneGeometry(1,1).rotateX(-π/2).translate(0,0,0.5)`
/// and the open-ended cylinder rotated the same way).
fn lane_mesh() -> Mesh {
    flat_mesh(
        vec![
            [-0.5, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [-0.5, 0.0, 1.0],
            [0.5, 0.0, 1.0],
        ],
        vec![0, 1, 2, 2, 1, 3],
    )
}

/// A floor fan from heading `a0` to `a1` at radius `r` (JS `_fan`,
/// bossHazards.js L472-485: centre vert + arc verts, triangles `(0, i, i+1)`).
fn fan_mesh(a0: f64, a1: f64, r: f64) -> Mesh {
    // JS L474: n = max(8, ceil(|a1-a0| / 2π * REACH_N)); REACH_N = 96.
    let n = ((a1 - a0).abs() / TAU * 96.0).ceil().max(8.0) as usize;
    let mut pos = vec![[0.0f32, 0.0, 0.0]];
    let mut idx = Vec::new();
    for i in 0..=n {
        let a = a0 + (a1 - a0) * (i as f64 / n as f64);
        // JS L478: x + sin(a) * r, z + cos(a) * r (centre at the origin).
        pos.push([(a.sin() * r) as f32, 0.0, (a.cos() * r) as f32]);
        if i > 0 {
            idx.extend_from_slice(&[0, i as u32, (i + 1) as u32]);
        }
    }
    flat_mesh(pos, idx)
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Root of the boss silhouette tree.
#[derive(Component)]
pub struct BossSilhouetteRoot;

/// One silhouette part, for the per-frame animation switch.
#[derive(Component)]
pub enum BossPart {
    Shell,
    Body,
    Head,
    ClawL,
    ClawR,
    StalkL,
    StalkR,
    EyeL,
    EyeR,
    Leg(u8),
}

/// Root of one crablet's block tree (JS `boss.crabs` Map entry).
#[derive(Component)]
pub struct CrabRoot {
    pub id: u32,
}

/// One telegraph marker entity.
#[derive(Component)]
pub struct Telegraph;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Marker identity: the move's `t0` bits, the kind, and the per-move index
/// (ring / barrel number).
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct MarkerKey {
    t0: u64,
    kind: u8,
    idx: u32,
}

/// Marker kinds (the JS draw switch cases, bossHazards.js L495-574).
mod kind {
    pub const SLAM_DISC: u8 = 0;
    pub const SLAM_BAND: u8 = 1;
    pub const BARREL_DISC: u8 = 2;
    pub const BARREL_BODY: u8 = 3;
    pub const SWEEP_FAN: u8 = 4;
    pub const SWEEP_BEAM: u8 = 5;
    pub const CHARGE_LANE: u8 = 6;
    pub const FRENZY_DISC: u8 = 7;
}

/// The silhouette's role materials (the hurt flash / phase-3 enrage tint
/// drive the chitin pair; the steel hull and the eye glow stay put).
struct SilhouetteMats {
    /// Chitin / flesh parts (JS `bossModel.js` L29 ink colour).
    ink: Handle<StandardMaterial>,
    /// Darkened ink for claws and leg tips (JS `rivalOf`-adjacent shading).
    ink_dark: Handle<StandardMaterial>,
    /// The container hull.
    steel: Handle<StandardMaterial>,
    /// The eye spheres (JS `uEyeGlow` L100).
    eye: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct BossRenderState {
    silhouette: Option<Entity>,
    silhouette_mats: Option<SilhouetteMats>,
    crabs: HashMap<u32, Entity>,
    markers: HashMap<MarkerKey, (Entity, Handle<StandardMaterial>)>,
    /// Per-sweep-move fan meshes, keyed by the record's `t0` bits.
    fans: HashMap<u64, Handle<Mesh>>,
    /// Shared meshes / materials, built on first use.
    disc: Option<Handle<Mesh>>,
    band: Option<Handle<Mesh>>,
    lane: Option<Handle<Mesh>>,
    barrel: Option<Handle<Mesh>>,
    crab_shell: Option<Handle<Mesh>>,
    crab_leg: Option<Handle<Mesh>>,
    crab_mat: Option<Handle<StandardMaterial>>,
    /// Smoothed animation scalars (placeholder of the JS spring animator).
    claw_lift: f32,
    dead_flat: f32,
}

impl Default for BossRenderState {
    fn default() -> Self {
        Self {
            silhouette: None,
            silhouette_mats: None,
            crabs: HashMap::new(),
            markers: HashMap::new(),
            fans: HashMap::new(),
            disc: None,
            band: None,
            lane: None,
            barrel: None,
            crab_shell: None,
            crab_leg: None,
            crab_mat: None,
            claw_lift: 0.0,
            dead_flat: 1.0,
        }
    }
}

/// Plugin wiring the boss presentation tier (Task 46 batch C1).
pub struct BossRenderPlugin;

impl Plugin for BossRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BossRenderState>()
            .init_resource::<BossShake>()
            .add_systems(Update, (boss_render_sync, boss_shake_decay));
    }
}

/// Placeholder screen shake for JS `emit('shake', {amount, pos})` — the
/// defeat quake (boss.js L182, `amount: 0.9`); foot/impact shakes (L59/L60)
/// ride events the sim does not raise yet (D-T46-10). The full directional
/// screen-shake system is Task 51 (screenfx.js).
#[derive(Resource, Default)]
pub struct BossShake {
    pub amount: f32,
}

/// Placeholder exponential decay (screenfx.js semantics deferred to Task 51).
fn boss_shake_decay(time: Res<Time>, mut shake: ResMut<BossShake>) {
    if shake.amount > 0.0 {
        shake.amount *= (-6.0 * time.delta_secs()).exp();
        if shake.amount < 0.001 {
            shake.amount = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------
// Silhouette
// ---------------------------------------------------------------------------

/// Translucent flat marker material (JS `mat(...)` L415-419: transparent,
/// depthWrite off, DoubleSide, fog off).
fn marker_mat(
    materials: &mut Assets<StandardMaterial>,
    color: Color,
    alpha: f32,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color.with_alpha(alpha),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        fog_enabled: false,
        ..default()
    })
}

/// Spawn the boss silhouette tree. Bounding-box reading of the JS rig:
/// the container shell at `CONT` (bossModelGeo.js L274: pos (0, 3.0, -1.3),
/// pitch 0.12, half-extents 3.03 × 1.3 × 1.22 → a 6.06 × 2.6 × 2.44 hull),
/// the crab body under it, the head at `HEAD.pivot` (L309-310), the claws at
/// the arm wrists `ARMS[].W` (L304-305), eye-stalk tips + eyes
/// `HEAD.stalk` (L311, eyeR 0.28) and six leg columns spread along the
/// `LEGS` hip→foot fan (L280-283).
fn spawn_silhouette(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> (Entity, SilhouetteMats) {
    // JS bossModelGeo.js L274 CONT: long axis hl=3.03 along **z** (the
    // container's rear wall sits at z = −hl, L436), width hw=1.22 along x,
    // height hh=1.3 along y → a 2.44 × 2.6 × 6.06 hull facing +Z.
    let shell = meshes.add(Cuboid::new(2.44, 2.6, 6.06));
    let body = meshes.add(Cuboid::new(3.4, 1.7, 3.8));
    let head = meshes.add(Cuboid::new(1.5, 1.0, 1.3));
    let claw = meshes.add(Cuboid::new(1.3, 0.95, 1.9)); // JS claw propodus box reading
    // Bevy `Cylinder::new(radius, half_height)`.
    let stalk = meshes.add(Cylinder::new(0.11, 0.42)); // JS HEAD.stalk column (L311)
    let eye = meshes.add(Sphere::new(0.28)); // JS HEAD.eyeR = 0.28 (L311)
    let leg = meshes.add(Cylinder::new(0.17, 1.15));

    let mats = SilhouetteMats {
        ink: materials.add(StandardMaterial {
            base_color: BOSS_INK,
            perceptual_roughness: 0.6,
            ..default()
        }),
        ink_dark: materials.add(StandardMaterial {
            base_color: BOSS_INK.mix(&Color::srgb(0.1, 0.1, 0.12), 0.45),
            perceptual_roughness: 0.7,
            ..default()
        }),
        steel: materials.add(StandardMaterial {
            base_color: SHELL_STEEL,
            perceptual_roughness: 0.85,
            ..default()
        }),
        eye: materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.93, 0.8),
            emissive: LinearRgba::new(2.2, 2.0, 1.2, 1.0),
            unlit: true,
            ..default()
        }),
    };

    let root = commands
        .spawn((
            BossSilhouetteRoot,
            Visibility::Visible,
            Transform::default(),
        ))
        .id();

    // (mesh, mat, local pos, local rot, part) — model space, feet at y 0,
    // facing +Z (JS bossModel.js header L1-2).
    let parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>, Vec3, BossPart)> = vec![
        (
            shell,
            mats.steel.clone(),
            // JS bossModelGeo.js L274: CONT.pos (0, 3.0, -1.3), pitch 0.12.
            Vec3::new(0.0, 3.0, -1.3),
            BossPart::Shell,
        ),
        (
            body,
            mats.ink.clone(),
            Vec3::new(0.0, 1.55, 0.4),
            BossPart::Body,
        ),
        (
            head,
            mats.ink.clone(),
            // JS HEAD.pivot (0, 2.25, 1.55) (bossModelGeo.js L310).
            Vec3::new(0.0, 2.25, 1.55),
            BossPart::Head,
        ),
        (
            claw.clone(),
            mats.ink_dark.clone(),
            // JS ARMS[0].W (2.02, 1.05, 4.3) (bossModelGeo.js L304).
            Vec3::new(2.02, 1.05, 4.3),
            BossPart::ClawL,
        ),
        (
            claw,
            mats.ink_dark.clone(),
            // JS ARMS[1].W (-1.8, 1.6, 4.3) (L305).
            Vec3::new(-1.8, 1.6, 4.3),
            BossPart::ClawR,
        ),
        (
            stalk.clone(),
            mats.ink.clone(),
            // JS HEAD.stalk mid ≈ (0.45, 3.03, 2.96) (L311).
            Vec3::new(0.45, 3.03, 2.96),
            BossPart::StalkL,
        ),
        (
            stalk,
            mats.ink.clone(),
            Vec3::new(-0.45, 3.03, 2.96),
            BossPart::StalkR,
        ),
        (
            eye.clone(),
            mats.eye.clone(),
            // JS HEAD.stalk[2] (0.64, 3.64, 3.1) (L311).
            Vec3::new(0.64, 3.64, 3.1),
            BossPart::EyeL,
        ),
        (
            eye,
            mats.eye.clone(),
            Vec3::new(-0.64, 3.64, 3.1),
            BossPart::EyeR,
        ),
    ];
    for (m, mt, p, part) in parts {
        // The shell carries CONT.pitch (JS bossModelGeo.js L274); the
        // rest sit axis-aligned at placeholder fidelity.
        let rot = if matches!(part, BossPart::Shell) {
            Quat::from_rotation_x(0.12)
        } else {
            Quat::IDENTITY
        };
        commands.spawn((
            Mesh3d(m),
            MeshMaterial3d(mt),
            part,
            Visibility::Visible,
            Transform {
                translation: p,
                rotation: rot,
                ..default()
            },
            ChildOf(root),
        ));
    }
    // Six walking-leg columns: 3 per side along the LEGS hip z fan
    // (bossModelGeo.js L281-283: feet at z 3.55 / 0.3 / -3.45, x 3.05-4.0).
    for (i, z) in [3.55f32, 0.3, -3.45].into_iter().enumerate() {
        for side in [1.0f32, -1.0] {
            commands.spawn((
                Mesh3d(leg.clone()),
                MeshMaterial3d(mats.ink_dark.clone()),
                BossPart::Leg((i * 2 + if side > 0.0 { 0 } else { 1 }) as u8),
                Visibility::Visible,
                Transform {
                    translation: Vec3::new(side * 3.4, 1.15, z),
                    rotation: Quat::from_rotation_z(side * -0.32),
                    ..default()
                },
                ChildOf(root),
            ));
        }
    }
    (root, mats)
}

/// Per-frame silhouette drive (JS `BossModel.update` st consumption,
/// bossModel.js L76-123 — placeholder fidelity: the animator's spring clips
/// become direct smoothing).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_silhouette(
    root_tf: &mut Transform,
    parts: &mut Query<
        (&BossPart, &mut Transform),
        (Without<BossSilhouetteRoot>, Without<CrabRoot>),
    >,
    state: &mut BossRenderState,
    pos: Vec3,
    yaw: f32,
    turn: f32,
    st: &BossSt,
    dt: f32,
    t: f32,
    materials: &mut Assets<StandardMaterial>,
) {
    // JS boss.js L71 st contract: speed / turn / move / movePhase / phase /
    // stunned / hurt / dead.
    root_tf.translation = pos;
    // turn → body lean (JS animator banks the body on turn; placeholder tilt;
    // stunned droops, JS bossModel.js L86-101 stunned pose).
    let stun_pitch = if st.stunned { 0.14 } else { 0.0 };
    root_tf.rotation = Quat::from_rotation_y(yaw)
        * Quat::from_rotation_z(-turn * 0.05)
        * Quat::from_rotation_x(stun_pitch);

    // dead → flatten (JS dead pose collapses the model; the brief pins the
    // flattened scale.y at 0.35).
    let flat_target = if st.dead { 0.35 } else { 1.0 };
    state.dead_flat += (flat_target - state.dead_flat) * (dt * 2.5).min(1.0);
    root_tf.scale = Vec3::new(1.0, state.dead_flat, 1.0);

    // Claw lift target: tele raises (JS slam / roar wind-up), the slam act
    // dives, stunned droops (JS bossModel.js L86-101 move / movePhase /
    // stunned pose reads).
    let in_tele = st.move_phase.as_deref() == Some("tele");
    let in_act = st.move_phase.as_deref() == Some("act");
    let tele_k = if st.phase_dur > 0.0 {
        js_clamp(f64::from(st.move_t) / f64::from(st.phase_dur), 0.0, 1.0) as f32
    } else {
        0.0
    };
    let target = if st.stunned || st.dead {
        0.8
    } else if in_act && st.move_id.as_deref() == Some("slam") {
        -0.4 // the slam down-stroke follows the raise
    } else if in_tele {
        -tele_k
    } else {
        0.0
    };
    state.claw_lift += (target - state.claw_lift) * (dt * 8.0).min(1.0);

    // speed → leg gait phase (JS step frequency scales with speed; the
    // crablet tripod pattern, bossModel.js L211, reused for the boss legs).
    let sp = (st.speed / 3.0).min(1.0);
    for (part, mut tf) in parts.iter_mut() {
        match part {
            BossPart::ClawL | BossPart::ClawR => {
                tf.rotation = Quat::from_rotation_x(state.claw_lift);
            }
            BossPart::Leg(i) => {
                let i = *i as usize;
                let grp = (i % 2) ^ (if i >= 3 { 1 } else { 0 });
                let a = t * (2.0 + st.speed * 2.4) + grp as f32 * std::f32::consts::PI;
                let side = if i.is_multiple_of(2) { -0.32 } else { 0.32 };
                tf.rotation =
                    Quat::from_rotation_z(side) * Quat::from_rotation_x(a.sin() * 0.45 * sp);
            }
            BossPart::EyeL | BossPart::EyeR => {
                // eyes wobble alive, dim dead (JS uEyeMode / uEyeGlow, L98-100).
                tf.scale = if st.dead {
                    Vec3::splat(0.6)
                } else {
                    Vec3::splat(1.0 + 0.1 * (t * 7.0).sin())
                };
            }
            _ => {}
        }
    }

    // hurt flash + phase-3 enrage tint on the chitin materials
    // (JS uFlash L89 / _enrage L87).
    let enrage = st.phase >= 3 && !st.dead;
    let flash = st.hurt.clamp(0.0, 1.0);
    if let Some(mats) = &state.silhouette_mats {
        for h in [&mats.ink, &mats.ink_dark] {
            let Some(mut m) = materials.get_mut(h) else {
                continue;
            };
            let dark = h.id() == mats.ink_dark.id();
            let base = if dark {
                BOSS_INK.mix(&Color::srgb(0.1, 0.1, 0.12), 0.45)
            } else {
                BOSS_INK
            };
            m.base_color = if enrage {
                base.mix(&TELE_RED, 0.3)
            } else {
                base
            };
            m.emissive = LinearRgba::new(flash, flash, flash, 1.0);
        }
        if let Some(mut m) = materials.get_mut(&mats.steel) {
            m.emissive = LinearRgba::new(flash, flash, flash, 1.0);
        }
        // eye glow dies on defeat (JS uEyeGlow L100: dead → max(0.35, …)).
        if let Some(mut m) = materials.get_mut(&mats.eye) {
            let g = if st.dead {
                0.35
            } else {
                1.0 + 0.35 * i32::from(enrage) as f32
            };
            m.emissive = LinearRgba::new(2.2 * g, 2.0 * g, 1.2 * g, 1.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Crablets
// ---------------------------------------------------------------------------

/// Spawn one crablet block tree (JS `BossModel.makeCrablet` — the container-
/// lid crab reads as a small shell box + four leg stubs; its bounding sphere
/// is r 0.9 at y 0.3, bossModel.js L187).
#[allow(clippy::too_many_arguments)]
fn spawn_crab(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    state: &mut BossRenderState,
    id: u32,
) -> Entity {
    let shell = state
        .crab_shell
        .get_or_insert_with(|| meshes.add(Cuboid::new(0.62, 0.26, 0.62)))
        .clone();
    let leg = state
        .crab_leg
        .get_or_insert_with(|| meshes.add(Cuboid::new(0.5, 0.07, 0.09)))
        .clone();
    let mat = state
        .crab_mat
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: BOSS_INK,
                perceptual_roughness: 0.6,
                ..default()
            })
        })
        .clone();

    let root = commands
        .spawn((CrabRoot { id }, Visibility::Visible, Transform::default()))
        .id();
    commands.spawn((
        Mesh3d(shell),
        MeshMaterial3d(mat.clone()),
        Visibility::Visible,
        // JS crablet body sits at y 0.26 (bossModel.js L205).
        Transform::from_translation(Vec3::new(0.0, 0.3, 0.0)),
        ChildOf(root),
    ));
    for (sx, sz) in [(0.3f32, 0.3f32), (-0.3, 0.3), (0.3, -0.3), (-0.3, -0.3)] {
        commands.spawn((
            Mesh3d(leg.clone()),
            MeshMaterial3d(mat.clone()),
            Visibility::Visible,
            Transform {
                translation: Vec3::new(sx * 1.4, 0.12, sz * 1.4),
                ..default()
            },
            ChildOf(root),
        ));
    }
    root
}

// ---------------------------------------------------------------------------
// The per-frame sync system
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn boss_render_sync(
    demo: Res<DemoSim>,
    time: Res<Time>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<BossRenderState>,
    mut sil_root: Query<
        &mut Transform,
        (
            With<BossSilhouetteRoot>,
            Without<BossPart>,
            Without<CrabRoot>,
        ),
    >,
    mut sil_parts: Query<
        (&BossPart, &mut Transform),
        (Without<BossSilhouetteRoot>, Without<CrabRoot>),
    >,
    mut crab_q: Query<(&mut Transform, &mut Visibility, &CrabRoot), Without<BossPart>>,
) {
    let guard = demo.bm.lock().unwrap();
    let Some(bm) = guard.as_ref() else {
        drop(guard);
        // Non-boss match: tear down if anything lingered, else zero cost.
        if state.silhouette.is_some() || !state.crabs.is_empty() || !state.markers.is_empty() {
            teardown(&mut commands, &mut state);
        }
        return;
    };
    let boss = &bm.boss;
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();

    // ---- silhouette ----
    if state.silhouette.is_none() {
        let (root, mats) = spawn_silhouette(&mut commands, &mut meshes, &mut materials);
        state.silhouette = Some(root);
        state.silhouette_mats = Some(mats);
    }
    // glam 0.30 (sim) vs 0.32 (Bevy): bridge via arrays (actors.rs precedent).
    let pos = Vec3::from_array(boss.pos.to_array());
    if let Ok(mut tf) = sil_root.single_mut() {
        update_silhouette(
            &mut tf,
            &mut sil_parts,
            &mut state,
            pos,
            boss.yaw,
            boss.turn,
            &boss.st,
            dt,
            t,
            &mut materials,
        );
    }

    // ---- crablets ----
    for c in &boss.crabs {
        let entity = match state.crabs.get(&c.id).copied() {
            Some(e) => e,
            None => spawn_crab(&mut commands, &mut meshes, &mut materials, &mut state, c.id),
        };
        state.crabs.insert(c.id, entity);
        // JS Crablet.update (bossModel.js L193-237): position + yaw, the
        // speed bob, and the pop swell → hide.
        if let Ok((mut tf, mut vis, crab_id)) = crab_q.get_mut(entity) {
            let sp = (c.speed.unwrap_or(0.0) / 3.0).min(1.0);
            tf.translation = Vec3::new(
                c.x,
                c.y + (t * 3.0 + crab_id.id as f32).sin().abs() * 0.02 * sp,
                c.z,
            );
            tf.rotation = Quat::from_rotation_y(c.yaw);
            if c.dead {
                // pop: swell 1 + 0.45 u² over 0.14 s, then hide
                // (JS bossModel.js L222-234).
                let u = (c.pop_t / 0.14).clamp(0.0, 1.0);
                tf.scale = Vec3::splat(1.0 + 0.45 * u * u);
                if c.pop_t >= 0.14 {
                    *vis = Visibility::Hidden;
                }
            } else {
                tf.scale = Vec3::splat(1.0);
                *vis = Visibility::Visible;
            }
        }
    }
    // Crabs the sim dropped (burst consumed): despawn their trees.
    let alive: HashSet<u32> = boss.crabs.iter().map(|c| c.id).collect();
    let gone: Vec<u32> = state
        .crabs
        .keys()
        .copied()
        .filter(|id| !alive.contains(id))
        .collect();
    for id in gone {
        if let Some(e) = state.crabs.remove(&id) {
            commands.entity(e).despawn();
        }
    }

    // ---- telegraph markers (JS HazardFX.draw, bossHazards.js L487-575) ----
    let disc = state
        .disc
        .get_or_insert_with(|| meshes.add(disc_mesh()))
        .clone();
    let band = state
        .band
        .get_or_insert_with(|| meshes.add(band_mesh(0.86, 1.0)))
        .clone();
    let lane = state
        .lane
        .get_or_insert_with(|| meshes.add(lane_mesh()))
        .clone();
    let barrel = state
        .barrel
        .get_or_insert_with(|| {
            // JS L423: CylinderGeometry(0.42, 0.42, 0.95).
            meshes.add(Cylinder::new(0.42, 0.475))
        })
        .clone();
    let hz = &boss.tb.hz;
    let bt = boss.bt;
    let mut seen: HashSet<MarkerKey> = HashSet::new();

    for m in &boss.moves {
        // JS L493: only moves still live at `bt` draw.
        if !boss.hz_live(m, bt) {
            continue;
        }
        let t0 = rec_t0(m);
        let t0b = t0.to_bits();
        let u = f64::from(bt) - t0;
        let tele = rec_d(m, 0);
        let act = rec_d(m, 1);
        let is_act = move_phase_at(m, u) == Some("act");
        let col = if is_act { ACT_RED } else { TELE_RED };
        let Some(id) = rec_id(m) else { continue };
        match id {
            MoveId::Slam => {
                let (px, py, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                // JS L497: the target mark through the tele + 0.15 s.
                if u < tele + 0.15 {
                    let k = slam_mark_k(u, tele);
                    let key = MarkerKey {
                        t0: t0b,
                        kind: kind::SLAM_DISC,
                        idx: 0,
                    };
                    seen.insert(key);
                    set_marker(
                        &mut commands,
                        &mut state,
                        &mut materials,
                        key,
                        &disc,
                        col,
                        (0.25 + 0.55 * k) as f32,
                        // JS L497: centre (p.x, p.y + 0.04, p.z), radius slamR.
                        Transform::from_translation(Vec3::new(
                            px as f32,
                            (py + 0.04) as f32,
                            pz as f32,
                        ))
                        .with_scale(Vec3::splat(f64::from(hz.slam_r) as f32)),
                    );
                }
                // JS L499-504: each ring band from its launch to ringMax
                // (no reach clip at placeholder tier — declared).
                for (k, _) in rec_rings(m).iter().enumerate() {
                    let r = ring_radius(hz, m, k, u);
                    if u < ring_start(m, k) || r > f64::from(hz.ring_max) {
                        continue; // JS L501
                    }
                    let key = MarkerKey {
                        t0: t0b,
                        kind: kind::SLAM_BAND,
                        idx: k as u32,
                    };
                    seen.insert(key);
                    set_marker(
                        &mut commands,
                        &mut state,
                        &mut materials,
                        key,
                        &band,
                        col,
                        band_alpha(hz, r) as f32,
                        // JS L503: uC = (p.x, p.y + 0.06, p.z), uR = r.
                        Transform::from_translation(Vec3::new(
                            px as f32,
                            (py + 0.06) as f32,
                            pz as f32,
                        ))
                        .with_scale(Vec3::splat(r as f32)),
                    );
                }
            }
            MoveId::Barrage => {
                let shell = [rec_p_f64(m, "sx"), rec_p_f64(m, "sy"), rec_p_f64(m, "sz")];
                for (i, b) in rec_pts(m).iter().enumerate() {
                    let land = b[3] + f64::from(hz.barrel_flight);
                    if u > land + 0.05 {
                        continue; // JS L518
                    }
                    // JS L520: the landing circle from the wind-up on.
                    if barrel_show(u, tele, b[3]) {
                        let k = barrel_mark_k(u, b[3], hz);
                        let key = MarkerKey {
                            t0: t0b,
                            kind: kind::BARREL_DISC,
                            idx: i as u32,
                        };
                        seen.insert(key);
                        set_marker(
                            &mut commands,
                            &mut state,
                            &mut materials,
                            key,
                            &disc,
                            col,
                            (0.2 + 0.7 * k) as f32,
                            Transform::from_translation(Vec3::new(
                                b[0] as f32,
                                (b[1] + 0.04) as f32,
                                b[2] as f32,
                            ))
                            .with_scale(Vec3::splat(f64::from(hz.barrel_r) as f32)),
                        );
                    }
                    // JS L521-524: the barrel in flight along the arc.
                    if u >= b[3] {
                        let kk = ((u - b[3]) / f64::from(hz.barrel_flight)).min(1.0);
                        let p = barrel_pos(shell, b, kk);
                        let key = MarkerKey {
                            t0: t0b,
                            kind: kind::BARREL_BODY,
                            idx: i as u32,
                        };
                        seen.insert(key);
                        set_marker(
                            &mut commands,
                            &mut state,
                            &mut materials,
                            key,
                            &barrel,
                            BARREL_RUST,
                            1.0,
                            // JS L524: rotation (k * 9 + b[0], 0, k * 6) —
                            // the placeholder keeps the yaw tumble only.
                            Transform::from_translation(Vec3::new(
                                p[0] as f32,
                                p[1] as f32,
                                p[2] as f32,
                            ))
                            .with_rotation(Quat::from_rotation_y((kk * 9.0 + b[0]) as f32)),
                        );
                    }
                }
            }
            MoveId::Sweep => {
                if u > tele + act {
                    continue; // JS L529
                }
                let (ox, oy, oz) = (rec_p_f64(m, "ox"), rec_p_f64(m, "oy"), rec_p_f64(m, "oz"));
                let (lo, hi) = sweep_span(m);
                // JS L532-534: radius per heading min(beamLen, reach); the
                // placeholder uses the constant HZ.beamLen (declared).
                let fan = state
                    .fans
                    .entry(t0b)
                    .or_insert_with(|| meshes.add(fan_mesh(lo, hi, f64::from(hz.beam_len))))
                    .clone();
                let k = sweep_k(u, tele, act);
                // JS L537: uAlpha = 0.55 + 0.45 * min(1, k * 1.5).
                let alpha = (0.55 + 0.45 * (k * 1.5).min(1.0)) as f32;
                let fkey = MarkerKey {
                    t0: t0b,
                    kind: kind::SWEEP_FAN,
                    idx: 0,
                };
                seen.insert(fkey);
                set_marker(
                    &mut commands,
                    &mut state,
                    &mut materials,
                    fkey,
                    &fan,
                    col,
                    alpha,
                    // JS L532: fan at (p.ox, p.y + 0.05, p.oz) — the fan
                    // sits on the floor (record `y`), not the muzzle
                    // height (`oy`).
                    Transform::from_translation(Vec3::new(
                        ox as f32,
                        (rec_p_f64(m, "y") + 0.05) as f32,
                        oz as f32,
                    )),
                );
                // JS L538-544: the beam itself during act.
                if u >= tele {
                    let ang = beam_angle(m, u);
                    let bkey = MarkerKey {
                        t0: t0b,
                        kind: kind::SWEEP_BEAM,
                        idx: 0,
                    };
                    seen.insert(bkey);
                    let l = f64::from(hz.beam_len);
                    // JS L540/L544: tip (ex, p.y + 0.3, ez); the beam
                    // length is the 3D distance muzzle → tip.
                    let py = rec_p_f64(m, "y");
                    let ex = ox + (ang as f64).sin() * l;
                    let ez = oz + (ang as f64).cos() * l;
                    let len =
                        ((ex - ox).powi(2) + (py + 0.3 - oy).powi(2) + (ez - oz).powi(2)).sqrt();
                    set_marker(
                        &mut commands,
                        &mut state,
                        &mut materials,
                        bkey,
                        &lane,
                        col,
                        0.92, // JS L436 beamMat opacity 0.92
                        // JS L541-544: position (p.ox, p.oy, p.oz), lookAt
                        // the tip, width w ≈ 0.32-0.38, length to the tip.
                        Transform::from_translation(Vec3::new(ox as f32, oy as f32, oz as f32))
                            .with_rotation(Quat::from_rotation_y(ang as f32))
                            .with_scale(Vec3::new(0.7, 0.7, len as f32)),
                    );
                }
            }
            MoveId::Charge => {
                if u > tele + act {
                    continue; // JS L550
                }
                let (px, py, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                let (yaw, v, p_l) = (rec_p_f64(m, "yaw"), rec_p_f64(m, "v"), rec_p_f64(m, "L"));
                let s = charge_dist(hz, v, p_l, u, tele);
                let len = charge_lane_len(p_l, s);
                let k = charge_lane_k(u, tele);
                // JS L554: uAlpha = u < tele ? 0.35 + 0.65 k : 0.9.
                let alpha = if u < tele {
                    (0.35 + 0.65 * k) as f32
                } else {
                    0.9
                };
                let key = MarkerKey {
                    t0: t0b,
                    kind: kind::CHARGE_LANE,
                    idx: 0,
                };
                seen.insert(key);
                set_marker(
                    &mut commands,
                    &mut state,
                    &mut materials,
                    key,
                    &lane,
                    col,
                    alpha,
                    // JS L553: position (p.x + sin(yaw) s, p.y + 0.05,
                    // p.z + cos(yaw) s), rotation (0, p.yaw, 0), scale
                    // (5.0, 1, len).
                    Transform::from_translation(Vec3::new(
                        (px + yaw.sin() * s) as f32,
                        (py + 0.05) as f32,
                        (pz + yaw.cos() * s) as f32,
                    ))
                    .with_rotation(Quat::from_rotation_y(yaw as f32))
                    .with_scale(Vec3::new(5.0, 1.0, len as f32)),
                );
            }
            MoveId::Crablets => {
                // JS: crablets carry no ground mark — the crabs themselves
                // are the tell (bossHazards.js L495 switch has no case).
            }
            MoveId::Frenzy => {
                if u >= tele + act {
                    continue; // JS L558
                }
                let (px, py, pz) = (rec_p_f64(m, "x"), rec_p_f64(m, "y"), rec_p_f64(m, "z"));
                let k = frenzy_k(u, tele);
                let key = MarkerKey {
                    t0: t0b,
                    kind: kind::FRENZY_DISC,
                    idx: 0,
                };
                seen.insert(key);
                set_marker(
                    &mut commands,
                    &mut state,
                    &mut materials,
                    key,
                    &disc,
                    col,
                    (0.5 + 0.5 * k) as f32, // JS L562
                    // JS L560: zone at (p.x, p.y + 0.05, p.z), radius frenzyR.
                    Transform::from_translation(Vec3::new(
                        px as f32,
                        (py + 0.05) as f32,
                        pz as f32,
                    ))
                    .with_scale(Vec3::splat(f64::from(hz.frenzy_r) as f32)),
                );
            }
        }
    }

    // JS L576-581: markers whose feature is gone get hidden / disposed.
    let stale: Vec<MarkerKey> = state
        .markers
        .keys()
        .copied()
        .filter(|k| !seen.contains(k))
        .collect();
    for k in stale {
        if let Some((e, _)) = state.markers.remove(&k) {
            commands.entity(e).despawn();
        }
    }
    // Fan meshes for records the sim pruned away.
    let live_t0: HashSet<u64> = boss.moves.iter().map(|m| rec_t0(m).to_bits()).collect();
    state.fans.retain(|t0b, _| live_t0.contains(t0b));
}

/// Create-or-update one marker entity with a fresh transform / alpha.
#[allow(clippy::too_many_arguments)]
fn set_marker(
    commands: &mut Commands,
    state: &mut BossRenderState,
    materials: &mut Assets<StandardMaterial>,
    key: MarkerKey,
    mesh: &Handle<Mesh>,
    color: Color,
    alpha: f32,
    tf: Transform,
) {
    if let Some((entity, mat)) = state.markers.get_mut(&key) {
        if let Some(mut m) = materials.get_mut(mat) {
            m.base_color = color.with_alpha(alpha);
        }
        commands.entity(*entity).insert((Mesh3d(mesh.clone()), tf));
        return;
    }
    let mat = marker_mat(materials, color, alpha);
    let entity = commands
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Telegraph,
            Visibility::Visible,
            tf,
        ))
        .id();
    state.markers.insert(key, (entity, mat));
}

/// Tear the whole boss presentation tier down (the match left the boss path).
fn teardown(commands: &mut Commands, state: &mut BossRenderState) {
    if let Some(e) = state.silhouette.take() {
        commands.entity(e).despawn();
    }
    state.silhouette_mats = None;
    for (_, e) in state.crabs.drain() {
        commands.entity(e).despawn();
    }
    for (e, _) in state.markers.values() {
        commands.entity(*e).despawn();
    }
    state.markers.clear();
    state.fans.clear();
    state.claw_lift = 0.0;
    state.dead_flat = 1.0;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::{AssetApp, AssetPlugin};
    use bevy::transform::TransformPlugin;
    use inkwave_sim::boss::CrabState;
    use inkwave_sim::collision::CollisionWorld;
    use inkwave_sim::geometry::StageLayout;

    /// A synthetic move record in the wire shape (brain.rs `start` L948-960).
    fn rec(id: &str, t0: f64, d: [f64; 3], p: Vec<(&str, JVal)>) -> JVal {
        JVal::Obj(vec![
            ("id".into(), JVal::Str(id.to_string())),
            ("t0".into(), JVal::Num(t0)),
            ("s".into(), JVal::Num(1.0)),
            (
                "d".into(),
                JVal::Arr(d.iter().copied().map(JVal::Num).collect()),
            ),
            (
                "p".into(),
                JVal::Obj(p.into_iter().map(|(k, v)| (k.to_string(), v)).collect()),
            ),
        ])
    }

    fn num(v: f64) -> JVal {
        JVal::Num(v)
    }

    fn hz() -> BossHazardTuning {
        inkwave_sim::embedded_tuning().boss.hz
    }

    /// slam: target-mark fade + ring launch / radius / alpha, pinned against
    /// the JS literals (HZ.slamR 3.3, ringR0 2.4, ringSpeed 9.5, ringMax 19,
    /// tele 1.15, rings [0, 0.9] — bossHazards.js L18/L28, brain L742-749).
    #[test]
    fn slam_marker_params_match_js() {
        let hz = hz();
        assert_eq!(hz.slam_r, 3.3);
        let m = rec(
            "slam",
            10.0,
            [1.15, 1.9, 1.0],
            vec![("rings", JVal::Arr(vec![num(0.0), num(0.9)]))],
        );
        // JS L497: k = clamp(u / tele, 0, 1).
        assert!((slam_mark_k(0.575, 1.15) - 0.5).abs() < 1e-12);
        assert_eq!(slam_mark_k(-0.2, 1.15), 0.0);
        assert_eq!(slam_mark_k(5.0, 1.15), 1.0);
        // JS L62: ringStart(m, k) = d[0] + rings[k].
        assert!((ring_start(&m, 0) - 1.15).abs() < 1e-12);
        assert!((ring_start(&m, 1) - 2.05).abs() < 1e-12);
        // JS L63: ringRadius = ringR0 + ringSpeed * (u - ringStart).
        let r = ring_radius(&hz, &m, 0, 1.6); // 2.4 + 9.5 * 0.45
        assert!((r - 6.675).abs() < 1e-5, "ring radius: {r}");
        // JS L504: alpha = 1 - smoothstep(15, 19, r).
        assert!((band_alpha(&hz, 15.0) - 1.0).abs() < 1e-12);
        assert!((band_alpha(&hz, 17.0) - 0.5).abs() < 1e-12);
        assert!((band_alpha(&hz, 19.0) - 0.0).abs() < 1e-12);
        assert_eq!(band_alpha(&hz, 20.0), 0.0);
    }

    /// barrage: the barrel arc endpoints / apex and the circle reveal gate,
    /// pinned against JS `barrelPos` (bossHazards.js L57-60) and L520.
    #[test]
    fn barrage_barrel_params_match_js() {
        let hz = hz();
        assert_eq!(hz.barrel_r, 2.4);
        assert_eq!(hz.barrel_flight, 1.3);
        let shell = [1.0, 5.4, 2.0];
        let b = [5.0, 0.0, 2.0, 1.2];
        // k = 0 → the shell; k = 1 → the circle (the arc term dies).
        let p0 = barrel_pos(shell, &b, 0.0);
        assert_eq!(p0, shell);
        let p1 = barrel_pos(shell, &b, 1.0);
        assert!(
            (p1[0] - 5.0).abs() < 1e-12 && p1[1].abs() < 1e-12 && (p1[2] - 2.0).abs() < 1e-12,
            "k=1 lands on the circle: {p1:?}"
        );
        // JS L58: peak = 5 + 0.35 * hypot(5 - 1, 2 - 2) = 6.4.
        // JS L59 at k = 0.5: x = 1 + 4 * 0.5 = 3;
        // y = 5.4 + (0 - 5.4) * 0.5 + 6.4 * 4 * 0.25 = 2.7 + 6.4 = 9.1;
        // z = 2.
        let pm = barrel_pos(shell, &b, 0.5);
        assert!((pm[0] - 3.0).abs() < 1e-12);
        assert!((pm[1] - 9.1).abs() < 1e-12, "apex y: {}", pm[1]);
        assert!((pm[2] - 2.0).abs() < 1e-12);
        // JS L520: the circle appears at u > min(tele * 0.4, throw - 0.5).
        assert!(!barrel_show(0.3, 0.85, 1.2)); // min(0.34, 0.7) = 0.34
        assert!(barrel_show(0.4, 0.85, 1.2));
        assert!(!barrel_show(0.3, 2.0, 0.9)); // min(0.8, 0.4) = 0.4
        assert!(barrel_show(0.5, 2.0, 0.9));
        // JS L520 pulse: land = 1.2 + 1.3 = 2.5; k = clamp(1 - (2.5 - u) / 1.8).
        assert!((barrel_mark_k(2.5, 1.2, &hz) - 1.0).abs() < 1e-12);
        assert!((barrel_mark_k(1.6, 1.2, &hz) - 0.5).abs() < 1e-6);
        assert_eq!(barrel_mark_k(0.0, 1.2, &hz), 0.0);
    }

    /// sweep: the beam easing and the fan span / intensity, pinned against JS
    /// `beamAngle` (bossHazards.js L61) and L531/L536.
    #[test]
    fn sweep_params_match_js() {
        let m = rec(
            "sweep",
            4.0,
            [1.15, 2.1, 0.9],
            vec![("a0", num(0.5)), ("a1", num(1.5))],
        );
        // JS L61: k = clamp((u - d[0]) / d[1], 0, 1), smoothstep-eased a0→a1.
        assert!((beam_angle(&m, 1.15) - 0.5).abs() < 1e-12);
        assert!((beam_angle(&m, 3.25) - 1.5).abs() < 1e-12);
        assert!((beam_angle(&m, 2.2) - 1.0).abs() < 1e-12);
        // Reversed arcs still fan lo → hi (JS L531 min/max).
        let rev = rec(
            "sweep",
            4.0,
            [1.15, 2.1, 0.9],
            vec![("a0", num(1.5)), ("a1", num(0.5))],
        );
        assert_eq!(sweep_span(&rev), (0.5, 1.5));
        // JS L536: k = u < tele ? u / tele : 1 - (u - tele) / act.
        assert!((sweep_k(0.575, 1.15, 2.1) - 0.5).abs() < 1e-12);
        assert!((sweep_k(2.2, 1.15, 2.1) - 0.5).abs() < 1e-12);
    }

    /// charge: the lane acceleration / length, pinned against JS
    /// `chargeDist` (bossHazards.js L48-54) and L551-553 (HZ.chargeRamp 0.35,
    /// chargeSpeed[1] 13).
    #[test]
    fn charge_lane_params_match_js() {
        let hz = hz();
        assert_eq!(hz.charge_ramp, 0.35);
        assert_eq!(hz.charge_speed[1], 13.0);
        let (v, l, tele) = (13.0, 20.0, 1.15);
        // t <= 0 → 0 (JS L50).
        assert_eq!(charge_dist(&hz, v, l, 1.0, tele), 0.0);
        // t = ramp exactly: s = 0.5 * (v / ramp) * ramp² = 0.5 * v * ramp
        // = 2.275 (JS L52).
        let s_ramp = charge_dist(&hz, v, l, tele + 0.35, tele);
        assert!((s_ramp - 2.275).abs() < 1e-5, "ramp dist: {s_ramp}");
        // t = ramp + 0.65: s = 2.275 + 13 * 0.65 = 10.725 (JS L52 else arm).
        let s_mid = charge_dist(&hz, v, l, tele + 1.0, tele);
        assert!((s_mid - 10.725).abs() < 1e-5, "mid dist: {s_mid}");
        // Capped at the lane length (JS L53 min(p.L, s)).
        assert_eq!(charge_dist(&hz, v, l, tele + 100.0, tele), 20.0);
        // JS L552: len = max(0.5, L + 4.2 - s).
        assert!((charge_lane_len(l, 0.0) - 24.2).abs() < 1e-12);
        assert!((charge_lane_len(l, 20.0) - 4.2).abs() < 1e-12);
        assert_eq!(charge_lane_len(0.0, 10.0), 0.5);
        // JS L551/L554: alpha 0.35 + 0.65 k during tele, 0.9 in act.
        assert!((charge_lane_k(0.575, 1.15) - 0.5).abs() < 1e-12);
    }

    /// frenzy: the full-circle zone radius + fade, pinned against JS L558-561
    /// (HZ.frenzyR 8.8, tele 1.2).
    #[test]
    fn frenzy_params_match_js() {
        let hz = hz();
        assert_eq!(hz.frenzy_r, 8.8);
        assert!((frenzy_k(0.6, 1.2) - 0.5).abs() < 1e-12);
        assert!((frenzy_k(-1.0, 1.2) - 0.0).abs() < 1e-12);
        assert_eq!(frenzy_k(2.0, 1.2), 1.0);
    }

    /// The fan mesh builder mirrors the JS `_fan` vertex layout
    /// (bossHazards.js L472-485): centre vert 0, arc verts 1..n+1, n =
    /// max(8, ceil(span / 2π * 96)).
    #[test]
    fn fan_mesh_matches_js_vertex_layout() {
        let m = fan_mesh(0.0, 1.9, 17.0);
        // JS L474: ceil(1.9 / 2π * 96) = ceil(29.03) = 30.
        let n = 30usize;
        assert_eq!((1.9 / TAU * 96.0).ceil() as usize, n);
        let pos = m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        // centre + (n + 1) arc verts.
        assert_eq!(pos.len(), 1 + n + 1);
        let idx = m.indices().unwrap();
        assert_eq!(idx.len(), n * 3);
    }

    fn demo() -> DemoSim {
        let layout: StageLayout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        DemoSim::new(layout, world, 20261004)
    }

    fn crab(id: u32, x: f32, z: f32) -> CrabState {
        CrabState {
            id,
            x,
            y: 0.0,
            z,
            yaw: 0.5,
            hp: 30.0,
            dead: false,
            speed: Some(1.0),
            miss_t: 0.0,
            pop_t: -1.0,
            spawned: true,
            popped: false,
            age: 1.0,
            tgt: -1,
            retgt: 1.0,
        }
    }

    /// Headless smoke: drive a real `replay_boss` match through the sync
    /// system (no GPU), then pin the telegraph marker lifecycle with a
    /// synthetic slam record across the tele → act → expired clocks, and the
    /// crablet entity count against the sim's crab list.
    #[test]
    fn boss_smoke_headless_entities() {
        let mut d = demo();
        d.replay_boss(0.0, "normal");
        for _ in 0..300 {
            d.step_once();
        }

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), TransformPlugin))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(d)
            .init_resource::<BossRenderState>()
            .add_systems(Update, boss_render_sync);
        app.update();

        // The silhouette spawned and drives without panicking.
        assert!(
            app.world()
                .resource::<BossRenderState>()
                .silhouette
                .is_some()
        );

        // Inject crablets: the entity count must equal the sim's list.
        {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            let boss = &mut bm.as_mut().unwrap().boss;
            boss.crabs.push(crab(1, 5.0, 5.0));
            boss.crabs.push(crab(2, 6.0, 5.0));
            let mut c = crab(3, 7.0, 5.0);
            c.dead = true;
            c.pop_t = 0.07; // mid-swell
            boss.crabs.push(c);
        }
        app.update();
        {
            let st = app.world().resource::<BossRenderState>();
            assert_eq!(st.crabs.len(), 3, "one entity per crab");
            let mut q = app.world_mut().query::<&CrabRoot>();
            assert_eq!(q.iter(app.world()).count(), 3);
        }

        // Synthetic slam at the current boss clock; walk it through the
        // phases. Clear the brain's own records first so the assertions are
        // deterministic.
        let (t0, tele) = {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            let boss = &mut bm.as_mut().unwrap().boss;
            boss.moves.clear();
            boss.hz_hits.clear();
            (f64::from(boss.bt), 1.15)
        };
        let m = rec(
            "slam",
            t0,
            [tele, 1.9, 1.0],
            vec![
                ("x", num(10.0)),
                ("y", num(0.0)),
                ("z", num(10.0)),
                ("rings", JVal::Arr(vec![num(0.0)])),
            ],
        );
        {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            bm.as_mut().unwrap().boss.hz_add(m);
        }
        // tele: the target disc only.
        {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            bm.as_mut().unwrap().boss.bt = (t0 + 0.5) as f32;
        }
        app.update();
        {
            let st = app.world().resource::<BossRenderState>();
            let slam: Vec<&MarkerKey> = st
                .markers
                .keys()
                .filter(|k| k.kind == kind::SLAM_DISC)
                .collect();
            assert_eq!(slam.len(), 1, "the slam target disc is up");
            let key = slam[0];
            let (e, _) = st.markers.get(key).unwrap();
            // JS L497: the disc scales by HZ.slamR = 3.3.
            let tf = app.world().get::<Transform>(*e).unwrap();
            assert!(
                (tf.scale.x - 3.3).abs() < 1e-5,
                "slamR scale: {}",
                tf.scale.x
            );
        }
        // act: the shockwave band joins (u = tele + 0.45 → r = 2.4 + 9.5 * 0.45).
        {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            bm.as_mut().unwrap().boss.bt = (t0 + tele + 0.45) as f32;
        }
        app.update();
        {
            let st = app.world().resource::<BossRenderState>();
            let key = MarkerKey {
                t0: t0.to_bits(),
                kind: kind::SLAM_BAND,
                idx: 0,
            };
            let (e, _) = st.markers.get(&key).expect("the slam band is up in act");
            let tf = app.world().get::<Transform>(*e).unwrap();
            assert!((tf.scale.x - 6.675).abs() < 1e-3, "ring r: {}", tf.scale.x);
        }
        // Expired (past end + extra + 0.2): every marker of the record clears.
        {
            let mut bm = app.world().resource::<DemoSim>().bm.lock().unwrap();
            bm.as_mut().unwrap().boss.bt = (t0 + 30.0) as f32;
        }
        app.update();
        {
            let st = app.world().resource::<BossRenderState>();
            assert!(
                !st.markers.keys().any(|k| k.t0 == t0.to_bits()),
                "the expired slam cleared"
            );
        }
    }

    /// Non-boss matches spawn nothing (the zero-cost guarantee).
    #[test]
    fn non_boss_match_spawns_nothing() {
        let app_demo = demo();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), TransformPlugin))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(app_demo)
            .init_resource::<BossRenderState>()
            .add_systems(Update, boss_render_sync);
        app.update();
        app.update();
        let mut q1 = app.world_mut().query::<&BossSilhouetteRoot>();
        assert_eq!(q1.iter(app.world()).count(), 0);
        let mut q2 = app.world_mut().query::<&CrabRoot>();
        assert_eq!(q2.iter(app.world()).count(), 0);
        let mut q3 = app.world_mut().query::<&Telegraph>();
        assert_eq!(q3.iter(app.world()).count(), 0);
    }
}
