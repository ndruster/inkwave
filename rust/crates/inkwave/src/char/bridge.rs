//! Task 54 batch C2: the pose bridge — per-frame sim → CharAnim → rig tree.
//!
//! [`char_pose_bridge`] runs right after [`crate::actors::sync_actors`]
//! (≡ the JS `Actor._finishFrame` → `character.update(dt, s)` driver,
//! actor.js L970-1017). For every roster slot it:
//!   * builds the [`AnimState`] from the sim `Actor` / `WeaponRunner`
//!     (≡ CONTRACTS §1 field table, ≡ actor.js L970-1013),
//!   * drives the 11 one-shot triggers from state edges (JS emits them from
//!     the event bus; the Rust render layer polls instead — same pattern as
//!     `fx/hooks.rs`, deviation registered),
//!   * runs [`CharAnim::update`] (which internally applies the pose,
//!     ≡ character.js L1356-1365),
//!   * writes every output into the Bevy rig tree spawned by
//!     [`crate::char::rig::spawn_char_rig`]: bone / kid / model / weapon
//!     frame-node / tank LOCAL transforms, weapon part transforms from
//!     `animateWeapon` (≡ rig.rs L933-935 "C2 attaches … here"),
//!   * runs the CPU `_updateMaterials` (D-T54-5①: uniforms → per-character
//!     cloned bucket materials),
//!   * resolves the CONTRACTS §1 query API (`getMuzzle` / `getHeadPosition`
//!     / `aimReady`) by composing the local chain the same way the pinned
//!     harness does (≡ char/pinned.rs L418-433).
//!
//! Scene-graph adaptation (rig.rs C1 tree vs the JS tree, deviation ⑦):
//! JS is `root → model → {kid → bones, squidRoot → pivot}`; rig.rs spawns
//! `root → {kid → bones, model}` with the squid parts mounted on `model`.
//! The bridge therefore folds `model ∘ kid` into the kid entity and
//! `model ∘ squidRoot ∘ squidPivot` into the model entity so both subtrees
//! see the same world transforms as in JS.
//!
//! Weapon meshes: the manifest mounts them on `handR`/`handL` with the
//! `pivot ∘ off` chain baked into the matrix (≡ rig.rs L903-931); the bridge
//! re-parents them under the `OffR`/`OffL` frame nodes at attach time with
//! `rest = (pivot ∘ off)⁻¹ · baked`, then overwrites the animated parts with
//! the `animateWeapon` outputs (≡ character.js L3531-3538 drum/spin,
//! character-weapons.js L1370).

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::OnceLock;

use bevy::prelude::*;
use glam::{DQuat, DVec3};
use inkwave_sim::actor::Form;
use inkwave_sim::sub::SubKind;

use crate::actors::{ActorMat, ActorRoot, team_color};
use crate::ink_render::DemoSim;
use crate::input::PlayerControls;

use super::anim::{AnimState, CharAnim, GroupXform, RunnerState, TrigArg};
use super::rig::{
    self, CharBone, CharKid, CharManifest, CharModel, CharPart, CharPartMatrix, CharRig,
    CharWeaponNode, MatBucket, RigSpawnOpts, WeaponFrames, WeaponNodeKind, bucket_material,
    mat_bucket,
};
use super::tables::Tables;

// ------------------------------------------------------------------ resources

/// Shared table load — `CharAnim` borrows `'t`, so the bridge leaks one
/// process-wide instance (deterministic, no per-slot copies).
fn shared_tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(Tables::load)
}

// Tuning anchors (ink max, hp max) cached once per system run so the
// helpers do not need `Tuning` threaded through every signature.
thread_local! {
    static TUNING: Cell<(f32, f32)> = const { Cell::new((100.0, 100.0)) };
}

/// One footfall edge emitted by the pose core (≡ character.js L1605-1614
/// `onEvent('footstep', {foot, pos, speed})` → bus `actor:<name>`).
///
/// DEFERRED (registered): the Task 50 audio bus has no `actor:*` hook yet
/// (`fx/hooks.rs` module header: "Polled (no bus equivalent): actor:*
/// (footstep/land/jump/form/dive/…)") — the events are queued here for the
/// batch that wires the footstep SFX consumer; nothing pops them yet.
#[allow(dead_code)] // queued for the deferred footstep consumer (see above)
#[derive(Debug, Clone, Copy)]
pub struct FootstepEvent {
    pub slot: usize,
    /// 0 = left foot, 1 = right (≡ `feet[i]`, i = `f.i`).
    pub foot: usize,
    /// World foot position at touchdown (≡ `f.pw`).
    pub pos: Vec3,
    /// Ground speed at touchdown (≡ `Math.max(gv, 0.6)`).
    pub speed: f32,
}

/// Which logical group a rig part entity belongs to (manifest row key class,
/// ≡ char.json `parts[].key` grammar, rig.rs L376-394).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Static skinned kid part (skin/cloth/eyes).
    Kid,
    /// Hair strand style N — visible only when N == the slot's style hair.
    Hair(u8),
    /// Weapon mesh; `left` = the dual-hand `@L` twin instance.
    Weapon { left: bool, drum: bool, spin: bool },
    /// The splat-bomb prop (handL-mounted).
    Bomb,
    /// Tank ink fill (the `_updateMaterials` water level rides this node).
    TankFill,
    /// Tank glass shell (static; the tank group is baked in its matrix).
    TankGlass,
    /// Squid-form mesh mounted on the model node.
    Squid,
}

/// One part entity's bridge bookkeeping.
#[derive(Debug)]
pub struct PartSlot {
    pub entity: Entity,
    pub kind: PartKind,
    /// `animateWeapon` part name (drum/pump/trigger/needle/bulb/lens/…),
    /// matched against `WeaponInstance.parts` (≡ rig.rs manifest `:part:`).
    pub name: Option<String>,
    /// Rest local transform under the CURRENT parent (weapon rows: the
    /// `off`-space rest = `(pivot ∘ off)⁻¹ · baked`, ≡ module header).
    pub rest: Transform,
    /// The manifest 3×4 row-major bone-local matrix (≡ rig.rs L792-801).
    pub baked: [f32; 12],
    pub bucket: MatBucket,
}

/// Per-frame sim-edge memory for the trigger polling.
#[derive(Debug, Clone, Copy)]
struct EdgePrev {
    fire: bool,
    grounded: bool,
    invuln: bool,
    hp: f32,
    vy: f32,
    charge: f64,
    sub: bool,
    special: bool,
    planted: [bool; 2],
}

impl Default for EdgePrev {
    fn default() -> Self {
        EdgePrev {
            fire: false,
            grounded: true,
            invuln: false,
            hp: f32::MAX,
            vy: 0.0,
            charge: 0.0,
            sub: false,
            special: false,
            planted: [false; 2],
        }
    }
}

/// One roster slot's rig binding + pose core instance. `anim` is `None`
/// only on gap placeholders before the first attach.
pub struct SlotRig {
    /// The rig root entity this slot is bound to (re-attach when it changes
    /// — `sync_actor_count` rebuilds the trees on roster changes, and
    /// [`CharBridge::set_weapon`] queues a tree rebuild).
    root: Entity,
    kid: Entity,
    model: Entity,
    /// Bone entities indexed by `CharBone.index` (≡ rig.rs L752-754).
    bones: Vec<Entity>,
    parts: Vec<PartSlot>,
    /// `[PivotR, OffR, Muzzle, PivotL, OffL]` (≡ rig.rs L783-789).
    nodes: [Option<Entity>; 5],
    /// The slot's weapon frames (drumAt/spinAt anchors, ≡ rig.rs L330-350).
    frames: Option<WeaponFrames>,
    anim: Option<CharAnim<'static>>,
    prev: EdgePrev,
    /// Enemy ink colour for the hurt splotch mix (≡ setHurt arg 2).
    hurt_color: [f64; 3],
    /// Per-character CLONED bucket materials (D-T54-5①: the shared C1
    /// buckets cannot carry per-slot ink/hurt/flash state).
    mats: Vec<(MatBucket, Handle<StandardMaterial>)>,
    /// Team colour (linear sRGB) for the team buckets (≡ setColor).
    team_color: [f64; 3],
    /// CONTRACTS §1 query outputs, refreshed every bridge frame.
    muzzle: Vec3,
    muzzle_hand: [Vec3; 2],
    head: Vec3,
    aim_ready: f32,
    /// The weapon kind the rig tree was spawned for (rebuild on change).
    weapon_kind: String,
    /// Held-sub visibility gate (≡ setSub).
    sub_bomb: bool,
}

/// The pose-bridge state resource (inserted by `world.rs`, `Default` empty).
#[derive(Resource, Default)]
pub struct CharBridge {
    archive: Option<self::rig::CharArchive>,
    manifest: Option<CharManifest>,
    slots: Vec<SlotRig>,
    /// Footstep queue (see [`FootstepEvent`] — deferred audio wiring).
    pub footsteps: VecDeque<FootstepEvent>,
    /// Slots pending a rig rebuild after [`CharBridge::set_weapon`].
    pending: Vec<(usize, String)>,
}

#[allow(dead_code)] // CONTRACTS §1 API surface: consumers land in later batches
impl CharBridge {
    fn ensure_assets(&mut self) -> bool {
        if self.archive.is_none() {
            self.archive = rig::load_archive();
        }
        if self.manifest.is_none() {
            self.manifest = rig::manifest();
        }
        self.archive.is_some() && self.manifest.is_some()
    }

    // ------------------------------------------------ CONTRACTS §1 API

    /// `c.setColor(color)` ≡ CONTRACTS §1 L36 — team ink colour.
    pub fn set_color(&mut self, slot: usize, css: &str) {
        let Some(s) = self.slots.get_mut(slot).and_then(|s| s.anim.as_mut()) else {
            return;
        };
        s.color = super::anim::parse_color(css);
    }

    /// `c.setWeapon(kind)` ≡ CONTRACTS §1 L37. The pose state switches
    /// immediately; the rig MESH set needs a tree rebuild, queued for the
    /// next bridge frame (JS swaps meshes in `_getWeapon`, ≡ L803-825).
    pub fn set_weapon(&mut self, slot: usize, kind: &str) {
        if let Some(s) = self.slots.get_mut(slot) {
            if s.weapon_kind == kind {
                return;
            }
            if let Some(c) = s.anim.as_mut() {
                c.set_weapon(kind);
            }
            self.pending.push((slot, kind.to_owned()));
        }
    }

    /// `c.setSub(kind)` ≡ CONTRACTS §1 (constructor opts.sub): the held sub
    /// prop. The manifest only carries the `bomb:` meshes — other subs hide
    /// the bomb prop (deviation registered in the task report).
    pub fn set_sub(&mut self, slot: usize, kind: &str) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.sub_bomb = kind == "bomb";
        }
    }

    /// `c.trigger(name, arg)` ≡ CONTRACTS §1 L39-41 (11 one-shots).
    pub fn trigger(&mut self, slot: usize, name: &str, arg: TrigArg) {
        if let Some(c) = self.slots.get_mut(slot).and_then(|s| s.anim.as_mut()) {
            c.trigger(name, arg);
        }
    }

    /// `c.setDance(name|null)` ≡ CONTRACTS §1 L43.
    pub fn set_dance(&mut self, slot: usize, name: Option<&str>) {
        if let Some(c) = self.slots.get_mut(slot).and_then(|s| s.anim.as_mut()) {
            c.set_dance(name);
        }
    }

    /// `c.setHurt(amount0to1, enemyColor)` ≡ CONTRACTS §1 L44 — the enemy
    /// ink splotch on the body/clothes buckets.
    pub fn set_hurt(&mut self, slot: usize, amount: f64, enemy: [f64; 3]) {
        if let Some(s) = self.slots.get_mut(slot) {
            if let Some(c) = s.anim.as_mut() {
                c.hurt = amount.clamp(0.0, 1.0);
            }
            s.hurt_color = enemy;
        }
    }

    /// `c.setVisible(bool)` ≡ CONTRACTS §1 L45.
    pub fn set_visible(&mut self, slot: usize, visible: bool) {
        if let Some(c) = self.slots.get_mut(slot).and_then(|s| s.anim.as_mut()) {
            c.visible = visible;
            c.root_visible = visible;
        }
    }

    /// `c.getMuzzle(out)` ≡ CONTRACTS §1 L46 / character.js L1245-1250:
    /// world-space muzzle of the held weapon; falls back to the head when
    /// not in kid form / unarmed. `None` before the first bridge frame.
    #[must_use]
    pub fn get_muzzle(&self, slot: usize) -> Option<Vec3> {
        self.attached(slot).map(|s| s.muzzle)
    }

    /// `c.getMuzzleHand(out, hand)` ≡ character.js L1251-1255: hand 1 = the
    /// dual left pistol's muzzle, else the main muzzle.
    #[must_use]
    pub fn get_muzzle_hand(&self, slot: usize, hand: usize) -> Option<Vec3> {
        self.attached(slot).map(|s| s.muzzle_hand[hand.min(1)])
    }

    /// `c.getHeadPosition(out)` ≡ character.js L1273-1277.
    #[must_use]
    pub fn get_head_position(&self, slot: usize) -> Option<Vec3> {
        self.attached(slot).map(|s| s.head)
    }

    /// `c.aimReady()` ≡ CONTRACTS §1 / character.js L1257: 0..1 aim-pose
    /// blend (1 = up and aimed; rollers never aim).
    #[must_use]
    pub fn aim_ready(&self, slot: usize) -> f32 {
        self.attached(slot).map_or(1.0, |s| s.aim_ready)
    }

    /// Drain the footstep queue (consumer hook, see [`FootstepEvent`]).
    pub fn take_footsteps(&mut self) -> Vec<FootstepEvent> {
        self.footsteps.drain(..).collect()
    }

    /// Test/inspection accessor: the slot's part entities.
    #[must_use]
    pub fn slot_parts(&self, slot: usize) -> &[PartSlot] {
        self.attached(slot).map_or(&[], |s| s.parts.as_slice())
    }

    /// Test/inspection accessor: the slot's part count (≤45 budget anchor,
    /// ≡ CONTRACTS §1 L86).
    #[must_use]
    pub fn slot_part_count(&self, slot: usize) -> usize {
        self.attached(slot).map_or(0, |s| s.parts.len())
    }

    fn attached(&self, slot: usize) -> Option<&SlotRig> {
        self.slots.get(slot).filter(|s| s.anim.is_some())
    }
}

// --------------------------------------------------------------------- system

/// WeaponNodeKind → the slot's `nodes` array index (≡ rig.rs L783-789).
fn node_idx(k: WeaponNodeKind) -> usize {
    match k {
        WeaponNodeKind::PivotR => 0,
        WeaponNodeKind::OffR => 1,
        WeaponNodeKind::Muzzle => 2,
        WeaponNodeKind::PivotL => 3,
        WeaponNodeKind::OffL => 4,
    }
}

/// `GroupXform` → `Transform` (f64 core → f32 render, ≡ batch-B f32
/// rounding discipline).
fn gx_transform(g: &GroupXform) -> Transform {
    Transform {
        translation: vec3(g.pos),
        rotation: quat(g.quat),
        scale: vec3(g.scale),
    }
}

fn vec3(v: DVec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

fn quat(q: DQuat) -> Quat {
    Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32)
}

fn row_of(m: &Mat4) -> [f32; 12] {
    [
        m.x_axis.x, m.y_axis.x, m.z_axis.x, m.w_axis.x, m.x_axis.y, m.y_axis.y, m.z_axis.y,
        m.w_axis.y, m.x_axis.z, m.y_axis.z, m.z_axis.z, m.w_axis.z,
    ]
}

/// Determinant of the 3×3 part of a row-major 3×4 matrix.
fn det3(m: &[f32; 12]) -> f32 {
    let c0 = Vec3::new(m[0], m[4], m[8]);
    let c1 = Vec3::new(m[1], m[5], m[9]);
    let c2 = Vec3::new(m[2], m[6], m[10]);
    c0.dot(c1.cross(c2))
}

/// Manual decomposition of a row-major 3×4 bone-local matrix expressing a
/// MIRROR (det < 0) — the twins far rows (rig.rs module header: "Bevy
/// Transform cannot represent the two negative-determinant twins mirror
/// rows"). The reflection rides on X (one negative scale axis); rotation
/// comes from the de-scaled columns. Total, never panics.
#[must_use]
pub fn mirror_transform(m: &[f32; 12]) -> Transform {
    let col = |c: usize| Vec3::new(m[c], m[4 + c], m[8 + c]);
    let (cx, cy, cz) = (col(0), col(1), col(2));
    let mut sx = cx.length();
    let sy = cy.length();
    let sz = cz.length();
    let det = cx.dot(cy.cross(cz));
    if det < 0.0 {
        sx = -sx;
    }
    if sx.abs() < 1e-9 || sy < 1e-9 || sz < 1e-9 {
        // Degenerate row: translation-only fallback, never panic.
        return Transform::from_translation(Vec3::new(m[3], m[7], m[11]));
    }
    let basis = Mat3::from_cols(cx / sx, cy / sy, cz / sz);
    Transform {
        translation: Vec3::new(m[3], m[7], m[11]),
        rotation: Quat::from_mat3(&basis),
        scale: Vec3::new(sx, sy, sz),
    }
}

/// Decompose a 3×4 row: mirrored rows go through [`mirror_transform`],
/// everything else uses Bevy's own decomposition (≡ rig.rs L801).
#[must_use]
pub fn decompose_row(m: &[f32; 12]) -> Transform {
    // `<= 0.0`: degenerate rows also take the total fallback (glam's
    // `from_matrix` can emit NaN on zero-scale matrices).
    if det3(m) <= 0.0 {
        mirror_transform(m)
    } else {
        Transform::from_matrix(row_to_mat4(m))
    }
}

fn row_to_mat4(m: &[f32; 12]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        Vec4::new(m[3], m[7], m[11], 1.0),
    )
}

/// Merged rig-node classification probe: the separate `CharKid` / `CharModel`
/// / `CharBone` / `CharWeaponNode` / `CharPart` queries folded into one item.
/// Bevy 0.19 caps system functions at 16 `SystemParam`s
/// (`bevy_ecs::system::function_system`, `all_tuples!(impl_system_function,
/// 0, 16, F)`), so the bridge keeps its world accesses in one query.
type RigClass<'w> = (
    Option<&'w CharKid>,
    Option<&'w CharModel>,
    Option<&'w CharBone>,
    Option<&'w CharWeaponNode>,
    Option<(&'w CharPart, &'w CharPartMatrix)>,
);

/// The per-frame pose bridge (runs right after `sync_actors`, ≡ world.rs
/// L707-708 wiring).
#[allow(clippy::too_many_arguments)]
pub fn char_pose_bridge(
    demo: Res<DemoSim>,
    time: Res<Time>,
    pc: Res<PlayerControls>,
    mut bridge: ResMut<CharBridge>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cmds: Commands,
    roots: Query<(Entity, &ActorRoot), With<CharRig>>,
    children: Query<&Children>,
    class: Query<RigClass<'_>>,
    mut qtf: Query<&mut Transform>,
    mut qvis: Query<&mut Visibility>,
    par_q: Query<&ChildOf>,
) {
    if !bridge.ensure_assets() {
        return; // char assets missing: the render layer degrades to nothing
    }
    TUNING.with(|t| {
        t.set((demo.tuning.player.ink_max, demo.tuning.player.hp));
    });
    let manifest = bridge.manifest.clone().expect("checked above");

    // ---- (re)attach when the roster trees changed -------------------------
    let mut current: Vec<(usize, Entity)> = roots.iter().map(|(e, r)| (r.slot, e)).collect();
    current.sort_by_key(|(s, _)| *s);
    bridge.slots.truncate(current.len());
    for (slot, root) in current.iter().copied() {
        let stale = bridge.slots.get(slot).is_none_or(|s| s.root != root);
        if stale {
            attach_slot(
                &mut bridge,
                slot,
                root,
                &demo,
                &children,
                &class,
                &qtf,
                &mut mats,
                &mut cmds,
                &manifest,
            );
        }
    }

    // ---- queued weapon swaps: rebuild the slot's rig tree -----------------
    let pending = std::mem::take(&mut bridge.pending);
    for (slot, kind) in pending {
        let (Some(archive), Some(manifest)) = (bridge.archive.as_ref(), bridge.manifest.as_ref())
        else {
            continue;
        };
        let (Some(s), Some(a)) = (bridge.slots.get(slot), demo.m.actors.get(slot)) else {
            continue;
        };
        let root = s.root;
        // Bevy 0.19: `despawn` removes the whole subtree.
        cmds.entity(root).despawn();
        let opts = RigSpawnOpts {
            tier: "game",
            weapon: Some(kind.as_str()),
            bomb: a.sub == SubKind::Bomb,
        };
        let built =
            rig::spawn_char_rig(&mut cmds, &mut meshes, &mut mats, archive, manifest, &opts);
        cmds.entity(built.root).insert((
            ActorRoot { slot },
            ActorMat(Vec::new()),
            Visibility::Visible,
        ));
        if let Some(s) = bridge.slots.get_mut(slot) {
            s.weapon_kind = kind;
            // Force a re-attach next frame (the new entities do not exist
            // until the commands flush); the CharAnim state carries over.
            s.root = Entity::PLACEHOLDER;
        }
    }

    // ---- per-slot frame ----------------------------------------------------
    let dt = time.delta_secs_f64();
    let CharBridge {
        slots, footsteps, ..
    } = &mut *bridge;
    for (slot, a) in demo.m.actors.iter().enumerate() {
        if pc.paused {
            // Freeze-frame: skip update, keep state + last transforms
            // (≡ the JS pause halting the whole sim/anim step).
            continue;
        }
        let Some(root) = current.iter().find(|(i, _)| *i == slot).map(|(_, e)| *e) else {
            continue;
        };
        let Some(s) = slots.get_mut(slot) else {
            continue;
        };
        if s.root != root || s.anim.is_none() {
            continue; // stale binding until the next attach
        }
        let runner = demo.m.runners.get(slot);
        drive_slot(s, a, runner, slot, dt, footsteps, &mut mats);
        write_slot(s, a, &mut qtf, &mut qvis, &par_q, &manifest);
    }
}

// -------------------------------------------------------------------- attach

/// Bind one freshly-spawned rig tree to a slot: classify parts, clone the
/// bucket materials, re-parent the weapon meshes, build (or carry over) the
/// CharAnim.
#[allow(clippy::too_many_arguments)]
fn attach_slot(
    bridge: &mut CharBridge,
    slot: usize,
    root: Entity,
    demo: &DemoSim,
    children: &Query<&Children>,
    class: &Query<RigClass<'_>>,
    qtf: &Query<&mut Transform>,
    mats: &mut Assets<StandardMaterial>,
    cmds: &mut Commands,
    manifest: &CharManifest,
) {
    let a = &demo.m.actors[slot];
    let kind_str = demo
        .m
        .kinds
        .get(slot)
        .map_or("none", |k| k.id_str())
        .to_owned();

    // Deterministic pre-order walk of the tree (spawn order = node order;
    // no HashMap iteration anywhere — determinism discipline).
    let mut bones: Vec<Entity> = vec![Entity::PLACEHOLDER; manifest.rig.bone_names.len()];
    let mut parts: Vec<PartSlot> = Vec::new();
    let mut nodes: [Option<Entity>; 5] = [None; 5];
    let mut kid_e = Entity::PLACEHOLDER;
    let mut model_e = Entity::PLACEHOLDER;
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok((kid, model, bone, node, part)) = class.get(e) {
            if kid.is_some() {
                kid_e = e;
            } else if model.is_some() {
                model_e = e;
            } else if let Some(b) = bone {
                bones[b.index as usize] = e;
            } else if let Some(n) = node {
                nodes[node_idx(n.0)] = Some(e);
            } else if let Some((p, pm)) = part
                && let Some(row) = manifest.parts.get(p.part as usize)
            {
                let bucket = mat_bucket(&row.mat_class).unwrap_or(MatBucket::Plastic);
                parts.push(PartSlot {
                    entity: e,
                    kind: classify_part(&row.key),
                    name: part_name(&row.key),
                    rest: qtf.get(e).copied().unwrap_or_default(),
                    baked: pm.0,
                    bucket,
                });
            }
        }
        if let Ok(ch) = children.get(e) {
            for c in ch.iter().rev() {
                stack.push(c);
            }
        }
    }

    // ---- twins mirror rows: fix C1's `from_matrix` approximation ----------
    for ps in &mut parts {
        if det3(&ps.baked) < 0.0 {
            let t = mirror_transform(&ps.baked);
            ps.rest = t;
            cmds.entity(ps.entity).insert(t);
        }
    }

    // ---- per-entity Visibility handles ------------------------------------
    // Bevy 0.19: `Visible` overrides a Hidden parent, so the hid set + death
    // hiding must run per entity — the C1 tree spawns no Visibility component,
    // so `write_slot` needs one on every node it drives.
    for e in [kid_e, model_e]
        .into_iter()
        .chain(nodes.into_iter().flatten())
        .chain(parts.iter().map(|p| p.entity))
        .filter(|e| *e != Entity::PLACEHOLDER)
    {
        cmds.entity(e).insert(Visibility::Inherited);
    }

    // ---- weapon mesh re-parent: handR/handL-baked → off-space ------------
    // rest_off = (pivot ∘ off)⁻¹ · baked (pivot/off are rigid scale-1 at
    // rest, read from the frame nodes BEFORE any overwrite).
    if let (Some(p), Some(o)) = (nodes[0], nodes[1]) {
        let pt = qtf.get(p).copied().unwrap_or_default();
        let ot = qtf.get(o).copied().unwrap_or_default();
        let inv_r = (chain_transform(&pt) * chain_transform(&ot))
            .try_inverse()
            .map(|m| row_of(&m));
        let inv_l = nodes[3].zip(nodes[4]).and_then(|(pl, ol)| {
            let plt = qtf.get(pl).copied().unwrap_or_default();
            let olt = qtf.get(ol).copied().unwrap_or_default();
            (chain_transform(&plt) * chain_transform(&olt))
                .try_inverse()
                .map(|m| row_of(&m))
        });
        if let Some(inv_r) = inv_r {
            for ps in &mut parts {
                let PartKind::Weapon { left, .. } = ps.kind else {
                    continue;
                };
                let (parent, inv) = if left {
                    match (nodes[4], &inv_l) {
                        (Some(e), Some(i)) => (e, i),
                        _ => continue,
                    }
                } else {
                    (o, &inv_r)
                };
                // (inv · baked) as a row, then the mirror-safe decomposition.
                let prod = row_of(&(row_to_mat4(inv) * row_to_mat4(&ps.baked)));
                ps.rest = decompose_row(&prod);
                cmds.entity(ps.entity).insert(ChildOf(parent));
            }
        }
    }

    // ---- per-character cloned bucket materials (D-T54-5①) -----------------
    let mut mats_list: Vec<(MatBucket, Handle<StandardMaterial>)> = Vec::new();
    for ps in &parts {
        if !mats_list.iter().any(|(b, _)| *b == ps.bucket) {
            let mut m = bucket_material(ps.bucket);
            m.alpha_mode = AlphaMode::Blend;
            mats_list.push((ps.bucket, mats.add(m)));
        }
    }
    for ps in &parts {
        let Some((_, h)) = mats_list.iter().find(|(b, _)| *b == ps.bucket) else {
            continue;
        };
        cmds.entity(ps.entity).insert(MeshMaterial3d(h.clone()));
    }

    // ---- pose core (carried over across rig rebuilds) --------------------
    let anim = bridge
        .slots
        .get_mut(slot)
        .and_then(|s| s.anim.take())
        .unwrap_or_else(|| {
            let tab = shared_tables();
            // Deterministic per-slot style (JS randomStyle uses the roster
            // seed — deviation: index-derived, no RNG in the render layer).
            let style = [slot % 8, slot % 4, 0, 0, 0, 0];
            let mut c = CharAnim::new(tab, &format!("slot{slot}"), "#ffffff", &kind_str, style);
            let tc = team_color(a.team).to_linear();
            c.color = [tc.red as f64, tc.green as f64, tc.blue as f64];
            c
        });
    let tc = team_color(a.team).to_linear();
    let enemy = team_color(1 - a.team).to_linear();
    let frames = manifest
        .weapons
        .get(kind_str.as_str())
        .map(|w| w.frames.clone());

    let slot_rig = SlotRig {
        root,
        kid: kid_e,
        model: model_e,
        bones,
        parts,
        nodes,
        frames,
        anim: Some(anim),
        prev: EdgePrev::default(),
        hurt_color: [enemy.red as f64, enemy.green as f64, enemy.blue as f64],
        mats: mats_list,
        team_color: [tc.red as f64, tc.green as f64, tc.blue as f64],
        muzzle: Vec3::ZERO,
        muzzle_hand: [Vec3::ZERO; 2],
        head: Vec3::ZERO,
        aim_ready: 1.0,
        weapon_kind: kind_str,
        sub_bomb: a.sub == SubKind::Bomb,
    };
    if slot < bridge.slots.len() {
        bridge.slots[slot] = slot_rig;
    } else {
        bridge.slots.push(slot_rig);
    }
}

/// Manifest part key → logical group.
fn classify_part(key: &str) -> PartKind {
    if let Some(rest) = key.strip_prefix("weapon:") {
        let left = key.ends_with("@L");
        let leaf = rest.rsplit(':').next().unwrap_or("");
        let leaf = leaf.strip_suffix("@L").unwrap_or(leaf);
        let drum = leaf == "drum" || leaf == "drumCaps";
        let spin = leaf == "spin";
        return PartKind::Weapon { left, drum, spin };
    }
    if key.starts_with("bomb:") {
        return PartKind::Bomb;
    }
    if let Some(rest) = key.strip_prefix("hair")
        && let Some(n) = rest.split(':').next().and_then(|s| s.parse::<u8>().ok())
    {
        return PartKind::Hair(n);
    }
    if key.starts_with("tankFill") {
        return PartKind::TankFill;
    }
    if key.starts_with("tankGlass") {
        return PartKind::TankGlass;
    }
    if key.starts_with("squid") {
        return PartKind::Squid;
    }
    PartKind::Kid
}

/// `animateWeapon` part name from the key (`…:part:<name>` or `…:<name>`);
/// the static body/ink/far/drum/spin meshes have no animated entry.
fn part_name(key: &str) -> Option<String> {
    let leaf = key.rsplit(':').next()?;
    let leaf = leaf.strip_suffix("@L").unwrap_or(leaf);
    if !key.starts_with("weapon:") {
        return None;
    }
    if matches!(
        leaf,
        "body" | "ink" | "farBody" | "farInk" | "drum" | "drumCaps" | "spin"
    ) {
        return None;
    }
    Some(leaf.to_owned())
}

// ------------------------------------------------------------------ per-frame

/// Build the [`AnimState`], drive the trigger edges and run the pose core
/// (≡ actor.js `_finishFrame` L970-1017).
fn drive_slot(
    s: &mut SlotRig,
    a: &inkwave_sim::actor::Actor,
    runner: Option<&inkwave_sim::weapon::WeaponRunner>,
    slot: usize,
    dt: f64,
    footsteps: &mut VecDeque<FootstepEvent>,
    mats: &mut Assets<StandardMaterial>,
) {
    let Some(c) = s.anim.as_mut() else { return };
    let (ink_max, hp_max) = TUNING.get();
    let hs = (a.vel.x as f64).hypot(a.vel.z as f64);
    let (cy, sy) = (a.yaw as f64, a.yaw as f64);
    let (cy, sy) = (cy.cos(), sy.sin());
    let inv = 1.0 / hs.max(0.001);
    let lmx = if hs > 0.1 {
        (a.vel.x as f64 * cy - a.vel.z as f64 * sy) * inv
    } else {
        0.0
    };
    let lmz = if hs > 0.1 {
        (a.vel.x as f64 * sy + a.vel.z as f64 * cy) * inv
    } else {
        0.0
    };

    let mut st = AnimState {
        speed: Some(hs),
        local_move: (-lmx, lmz),
        grounded: Some(a.grounded),
        vy: Some(a.vel.y as f64),
        aim_pitch: a.aim_pitch as f64,
        aim_yaw: a.aim_yaw as f64,
        firing: runner.is_some_and(|r| r.firing())
            || a.special_active.as_ref().is_some_and(|sp| sp.firing > 0.0),
        charge: Some(runner.map_or(0.0, |r| r.charge() as f64)),
        rolling: runner.is_some_and(|r| r.rolling()),
        low_ink: a.ink < 18.0, // ≡ actor.js L997 (`this.ink < 18`)
        ink: Some((a.ink / ink_max).clamp(0.0, 1.0) as f64),
        hp: Some((a.hp / hp_max).clamp(0.0, 1.0) as f64),
        in_enemy_ink: a.on_enemy,
        form: match a.form {
            Form::Kid => "kid".to_owned(),
            Form::Squid if a.submerged => "swim".to_owned(),
            Form::Squid => "squid".to_owned(),
        },
        special: Some(a.special_frac() as f64),
        invuln: a.invuln > 0.0,
        turn_rate: Some(a.yaw_vel as f64),
        runner: runner.map(|r| RunnerState {
            lock_t: r.lock_t() as f64,
            dodge: a.dodge_vel.is_some(),
            charging: r.charging_flag(),
            streaming: r.streaming(),
            charge: r.charge() as f64,
            burst_frac: Some(r.burst_frac() as f64),
        }),
        ..AnimState::default()
    };
    // subAim: `aiming_sub` is pub(crate) in the sim (zero-change rule) —
    // deviation: the cocked-throw pose is not driven from the runner.
    st.sub_aim = None;

    // ---- trigger polling (≡ the JS event-bus triggers; deviation: the
    // render layer polls sim state, same pattern as fx/hooks.rs) ----------
    let p = s.prev;
    let prev_planted = p.planted;
    if a.fire_gate.fire && !p.fire {
        c.trigger("shoot", TrigArg::Shoot { hand: 0 });
    }
    if p.charge > 0.02 && st.charge.unwrap_or(0.0) <= 0.001 && !a.fire_gate.fire {
        c.trigger("charge_release", TrigArg::None);
    }
    if p.grounded && !a.grounded && a.vel.y > 0.0 {
        c.trigger("jump", TrigArg::None);
    }
    if !p.grounded && a.grounded {
        c.trigger("land", TrigArg::Num(p.vy.abs() as f64));
    }
    let hp = a.hp;
    if hp < p.hp - 0.001 {
        c.trigger(
            "hit",
            TrigArg::Hit {
                x: 0.0,
                z: 0.0,
                amount: ((p.hp - hp) / 10.0).clamp(0.4, 1.2) as f64,
            },
        );
    }
    if !p.invuln && a.invuln > 0.0 {
        c.trigger("spawn", TrigArg::None);
    }
    if !p.special && a.special_active.is_some() {
        c.trigger("special_leap", TrigArg::None);
    }
    if a.fire_gate.sub_released && !p.sub {
        c.trigger("throw", TrigArg::None);
    }

    // ---- root motion handed to the core (engine-owned, ≡ actor.js L1014-1016)
    c.root_pos = DVec3::new(a.pos.x as f64, a.pos.y as f64, a.pos.z as f64);
    c.root_yaw = a.yaw as f64;
    c.root_visible = a.alive;
    c.root_parent_visible = true;
    c.visible = a.alive;
    // setHurt hp term (≡ actor.js L1013; `hurtFlash` is private in the sim —
    // deviation: hp deficit only).
    let hp_term = if a.hp < hp_max {
        1.0 - (a.hp / hp_max).clamp(0.0, 1.0) as f64
    } else {
        0.0
    };
    c.hurt = c.hurt.max(hp_term);
    c.bombHeld = s.sub_bomb && a.form == Form::Kid;

    c.update(dt, &st);

    s.prev = EdgePrev {
        fire: a.fire_gate.fire,
        grounded: a.grounded,
        invuln: a.invuln > 0.0,
        hp,
        vy: a.vel.y,
        charge: st.charge.unwrap_or(0.0),
        sub: a.fire_gate.sub_released,
        special: a.special_active.is_some(),
        planted: [c.feet[0].planted, c.feet[1].planted],
    };

    // ---- footstep edges (≡ character.js L1605-1614 touchdown) ------------
    for (i, f) in c.feet.iter().enumerate() {
        if f.planted && !prev_planted[i] && c.gv.max(0.0) > 0.6 {
            if footsteps.len() >= 128 {
                footsteps.pop_front();
            }
            footsteps.push_back(FootstepEvent {
                slot,
                foot: i,
                pos: vec3(f.pw),
                speed: c.gv.max(0.6) as f32,
            });
        }
    }

    // ---- CPU `_updateMaterials` → cloned buckets (D-T54-5①) --------------
    let flash = c.mat.u_flash[0].max(c.mat.u_flash[1]).max(c.mat.u_flash[2]) as f32;
    let alpha = (1.0 - flash).clamp(0.0, 1.0);
    let glow_i = c.mat.glow_emissive_intensity as f32;
    let glow = c.mat.u_glow;
    let fill_e = c.mat.fill_emissive;
    let team = s.team_color;
    let hurt = c.hurt as f32;
    let enemy = s.hurt_color;
    for (bucket, h) in &s.mats {
        let Some(mut m) = mats.get_mut(h) else {
            continue;
        };
        m.base_color = match bucket {
            MatBucket::Hair | MatBucket::Ink | MatBucket::Fill | MatBucket::Squid => {
                Color::linear_rgba(team[0] as f32, team[1] as f32, team[2] as f32, alpha)
            }
            MatBucket::Skin | MatBucket::Cloth => Color::linear_rgba(
                (1.0 - hurt) + hurt * enemy[0] as f32,
                (1.0 - hurt) + hurt * enemy[1] as f32,
                (1.0 - hurt) + hurt * enemy[2] as f32,
                alpha,
            ),
            _ => Color::linear_rgba(1.0, 1.0, 1.0, alpha),
        };
        m.emissive = match bucket {
            MatBucket::Glow => LinearRgba::new(
                glow[0] as f32 * glow_i,
                glow[1] as f32 * glow_i,
                glow[2] as f32 * glow_i,
                1.0,
            ),
            MatBucket::Fill => {
                LinearRgba::new(fill_e[0] as f32, fill_e[1] as f32, fill_e[2] as f32, 1.0)
            }
            _ => LinearRgba::BLACK,
        };
    }
}

// --------------------------------------------------------------------- writes

/// Push the CharAnim outputs into the entity tree + resolve the query API.
fn write_slot(
    s: &mut SlotRig,
    a: &inkwave_sim::actor::Actor,
    qtf: &mut Query<&mut Transform>,
    qvis: &mut Query<&mut Visibility>,
    par_q: &Query<&ChildOf>,
    manifest: &CharManifest,
) {
    let Some(c) = s.anim.as_ref() else { return };
    let alive = a.alive;
    let vis_of = |v: bool| {
        if alive && v {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    };

    // kid = model ∘ kid (≡ module header scene-graph adaptation)
    if let Ok(mut t) = qtf.get_mut(s.kid) {
        *t = Transform::from_matrix(
            chain_transform(&gx_transform(&c.model)) * chain_transform(&gx_transform(&c.kid)),
        );
    }
    set_vis(qvis, s.kid, vis_of(c.kid_visible));
    // model = model ∘ squidRoot ∘ squidPivot (squid parts mount here)
    if let Ok(mut t) = qtf.get_mut(s.model) {
        let m = chain_transform(&gx_transform(&c.model))
            * chain_transform(&gx_transform(&c.squidRoot))
            * chain_transform(&gx_transform(&c.squidPivot));
        *t = Transform::from_matrix(m);
    }
    set_vis(qvis, s.model, vis_of(c.squidRoot_visible));

    // bones (≡ apply.rs local writes; CharBone.index → slot)
    for (i, b) in c.bones.iter().enumerate() {
        if let Some(&e) = s.bones.get(i)
            && let Ok(mut t) = qtf.get_mut(e)
        {
            *t = gx_transform(b);
        }
    }

    // weapon frame nodes (≡ apply.rs weapon anchor + rig.rs L933-935)
    let wvis = vis_of(!c.weaponHidden && c.kid_visible);
    if let (Some(p), Some(o), Some(mz)) = (s.nodes[0], s.nodes[1], s.nodes[2]) {
        if let Some(w) = c.weapon.as_ref() {
            if let Ok(mut t) = qtf.get_mut(p) {
                *t = gx_transform(&w.pivot);
            }
            if let Ok(mut t) = qtf.get_mut(o) {
                *t = gx_transform(&w.off);
            }
        }
        set_vis(qvis, p, wvis);
        set_vis(qvis, o, wvis);
        set_vis(qvis, mz, wvis);
    }
    let lwvis = vis_of(!c.bombHeld && !c.weaponHidden && c.kid_visible); // ≡ L3539
    if let (Some(p), Some(o)) = (s.nodes[3], s.nodes[4]) {
        if let Some(w) = c.weapon.as_ref().and_then(|w| w.left.as_ref()) {
            if let Ok(mut t) = qtf.get_mut(p) {
                *t = gx_transform(&w.pivot);
            }
            if let Ok(mut t) = qtf.get_mut(o) {
                *t = gx_transform(&w.off);
            }
        }
        set_vis(qvis, p, lwvis);
        set_vis(qvis, o, lwvis);
    }

    // weapon parts (≡ animateWeapon outputs, character-weapons.js L1370,
    // character.js L3531-3538)
    for ps in &s.parts {
        match ps.kind {
            PartKind::Weapon { left, drum, spin } => {
                let w = if left {
                    c.weapon
                        .as_ref()
                        .and_then(|w| w.left.as_ref())
                        .map(|b| b.as_ref())
                } else {
                    c.weapon.as_ref()
                };
                let mut local = ps.rest;
                let mut wrote = false;
                if let (Some(w), Some(name)) = (w, &ps.name)
                    && let Some(p) = w.parts.iter().find(|p| &p.name == name)
                {
                    local = Transform {
                        translation: vec3(p.pos),
                        rotation: quat(p.quat),
                        scale: vec3(p.scale),
                    };
                    wrote = true;
                }
                if !wrote
                    && drum
                    && let (Some(w), Some(f)) = (w, &s.frames)
                    && let Some(d) = f.drum_at
                {
                    let g = Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32);
                    local = Transform::from_matrix(
                        Mat4::from_translation(g)
                            * Mat4::from_rotation_x(w.drum_a as f32)
                            * Mat4::from_translation(-g)
                            * ps.rest.to_matrix(),
                    );
                    wrote = true;
                }
                if !wrote
                    && spin
                    && let (Some(w), Some(f)) = (w, &s.frames)
                    && let Some(sp) = f.spin_at
                {
                    let g = Vec3::new(sp[0] as f32, sp[1] as f32, sp[2] as f32);
                    local = Transform::from_matrix(
                        Mat4::from_translation(g)
                            * Mat4::from_rotation_z(w.spin_mesh_angle as f32)
                            * Mat4::from_translation(-g)
                            * ps.rest.to_matrix(),
                    );
                }
                if let Ok(mut t) = qtf.get_mut(ps.entity) {
                    *t = local;
                }
                set_vis(
                    qvis,
                    ps.entity,
                    vis_of(!c.weaponHidden && c.kid_visible && !(left && c.bombHeld)),
                );
            }
            PartKind::Bomb => set_vis(qvis, ps.entity, vis_of(c.bomb_visible && s.sub_bomb)),
            PartKind::Hair(n) => {
                set_vis(
                    qvis,
                    ps.entity,
                    vis_of(c.kid_visible && n as usize == c.style[0]),
                );
            }
            PartKind::Squid => set_vis(qvis, ps.entity, vis_of(c.squidRoot_visible)),
            PartKind::TankFill => {
                // fill = baked_rest ∘ (slosh quat, ink-level scale) — the
                // tank group is static in the baked matrix (≡ character.js
                // L781-783, L3744-3748; `tankFill.pos` never moves).
                if let Ok(mut t) = qtf.get_mut(ps.entity) {
                    *t = Transform::from_matrix(
                        ps.rest.to_matrix()
                            * Mat4::from_quat(quat(c.tankFill.quat))
                            * Mat4::from_scale(vec3(c.tankFill.scale)),
                    );
                }
                set_vis(qvis, ps.entity, vis_of(c.tankFill_visible && c.kid_visible));
            }
            PartKind::TankGlass => set_vis(qvis, ps.entity, vis_of(c.kid_visible)),
            PartKind::Kid => set_vis(qvis, ps.entity, vis_of(c.kid_visible)),
        }
    }

    // ---- CONTRACTS §1 query API (≡ character.js L1245-1277) --------------
    let kid_form = c.form == "kid";
    s.head = if kid_form {
        let head_i = manifest.rig.bone_index.get("head").copied().unwrap_or(0);
        match s.bones.get(head_i) {
            Some(&e) => {
                let m = world_matrix(e, qtf, par_q);
                let ctr = shared_tables().rig_c.head_ctr;
                m.transform_point3(Vec3::new(ctr[0] as f32, ctr[1] as f32, ctr[2] as f32))
            }
            None => Vec3::ZERO,
        }
    } else {
        world_pos(s.model, qtf, par_q)
    };
    s.muzzle = if !kid_form || c.weapon.is_none() {
        s.head
    } else {
        match s.nodes[2] {
            Some(e) => world_pos(e, qtf, par_q),
            None => s.head,
        }
    };
    s.muzzle_hand = if c.weapon.as_ref().and_then(|w| w.left.as_ref()).is_some() && kid_form {
        [
            s.muzzle,
            s.nodes[4].map_or(s.muzzle, |e| world_pos(e, qtf, par_q)),
        ]
    } else {
        [s.muzzle, s.muzzle]
    };
    // aimReady ≡ character.js L1257
    s.aim_ready = if !kid_form || c.weapon.is_none() || c.animKind == "roller" {
        1.0
    } else {
        c.wAim as f32
    };
}

fn set_vis(qvis: &mut Query<&mut Visibility>, e: Entity, v: Visibility) {
    if let Ok(mut vis) = qvis.get_mut(e) {
        *vis = v;
    }
}

/// Compose a `Transform` into its local matrix (parent → child order).
fn chain_transform(tf: &Transform) -> Mat4 {
    tf.to_matrix()
}

/// World matrix of an entity: compose the local Transforms root → entity
/// (Bevy's `GlobalTransform` has not propagated yet this frame, ≡ the
/// pinned harness composition char/pinned.rs L418-433).
fn world_matrix(e: Entity, qtf: &Query<&mut Transform>, par_q: &Query<&ChildOf>) -> Mat4 {
    let mut chain: Vec<Entity> = Vec::new();
    let mut cur = e;
    while let Ok(p) = par_q.get(cur) {
        let pe = p.parent();
        chain.push(cur);
        cur = pe;
    }
    chain.push(cur);
    let mut m = Mat4::IDENTITY;
    for ent in chain.iter().rev() {
        if let Ok(t) = qtf.get(*ent) {
            m *= t.to_matrix();
        }
    }
    m
}

fn world_pos(e: Entity, qtf: &Query<&mut Transform>, par_q: &Query<&ChildOf>) -> Vec3 {
    world_matrix(e, qtf, par_q).transform_point3(Vec3::ZERO)
}

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::{spawn_actors, sync_actors};
    use bevy::asset::{AssetApp, AssetPlugin};
    use bevy::transform::TransformPlugin;
    use inkwave_sim::collision::CollisionWorld;
    use inkwave_sim::geometry::StageLayout;

    #[derive(Resource, Default)]
    struct Script {
        frame: usize,
    }

    /// Deterministic scripted sim mutations before the bridge each frame —
    /// the "scripted AnimState sequence" the batch-C2 test spec asks for.
    fn script_drive(mut demo: ResMut<DemoSim>, mut script: ResMut<Script>) {
        script.frame += 1;
        let f = script.frame;
        for (i, a) in demo.m.actors.iter_mut().enumerate() {
            let ph = f as f32 * 0.1 + i as f32;
            a.vel.x = ph.sin() * 4.0;
            a.vel.z = ph.cos() * 3.0;
            a.vel.y = (ph * 0.7).sin() * 2.0;
            a.grounded = (ph * 0.7).sin() > 0.0;
            a.ink = 50.0 + 40.0 * (ph * 0.3).cos();
            a.hp = 100.0 - i as f32 * 5.0 - (f as f32 * 0.2).min(20.0);
            a.invuln = if f.is_multiple_of(7) { 1.0 } else { 0.0 };
            a.fire_gate.fire = f.is_multiple_of(3);
            a.yaw = ph * 0.2;
        }
        // slot 2 cycles the squid/swim forms
        let a2 = &mut demo.m.actors[2];
        a2.form = Form::Squid;
        a2.submerged = f.is_multiple_of(2);
    }

    fn bridge_app() -> App {
        let layout: StageLayout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout, world, 20261004);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), TransformPlugin))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(demo)
            .insert_resource(PlayerControls::default())
            // Fixed 60 Hz step: the double-run determinism check must not
            // depend on the wall clock.
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ))
            .init_resource::<Script>()
            .init_resource::<CharBridge>();
        app.add_systems(Startup, spawn_actors);
        app.add_systems(
            Update,
            (script_drive, sync_actors, char_pose_bridge).chain(),
        );
        app
    }

    fn tf_row(t: &Transform) -> [f32; 10] {
        let mut r = [0.0f32; 10];
        let q = t.rotation;
        r[0..3].copy_from_slice(&t.translation.to_array());
        r[3..7].copy_from_slice(&[q.x, q.y, q.z, q.w]);
        r[7..10].copy_from_slice(&t.scale.to_array());
        r
    }

    /// Snapshot of every bridge-written node (kid/model/bones/parts), ordered
    /// by (entity slot, bone index) — identical across two identical runs.
    fn snapshot(world: &mut World) -> Vec<f32> {
        let mut rows: Vec<(u32, u16, [f32; 10])> = Vec::new();
        let mut bq = world.query::<(Entity, &CharBone, &Transform)>();
        for (e, b, t) in bq.iter(world) {
            rows.push((e.index().index(), b.index, tf_row(t)));
        }
        let mut pq = world.query::<(Entity, &CharPart, &Transform)>();
        for (e, _, t) in pq.iter(world) {
            rows.push((e.index().index(), u16::MAX, tf_row(t)));
        }
        let mut kq = world.query::<(Entity, &CharKid, &Transform)>();
        for (e, _, t) in kq.iter(world) {
            rows.push((e.index().index(), u16::MAX - 1, tf_row(t)));
        }
        let mut mq = world.query::<(Entity, &CharModel, &Transform)>();
        for (e, _, t) in mq.iter(world) {
            rows.push((e.index().index(), u16::MAX - 2, tf_row(t)));
        }
        rows.sort_unstable_by_key(|(ei, bi, _)| (*ei, *bi));
        let mut out = Vec::with_capacity(rows.len() * 10);
        for (_, _, r) in rows {
            out.extend_from_slice(&r);
        }
        out
    }

    fn drive(app: &mut App, frames: usize) {
        app.update(); // Startup + attach frame
        for _ in 0..frames {
            app.update();
        }
    }

    /// Spec: 30+ scripted frames → every written Transform finite, and the
    /// double run is bit-identical (no hidden RNG / iteration-order state).
    #[test]
    fn pose_bridge_30_frames_finite_and_deterministic() {
        let mut app = bridge_app();
        drive(&mut app, 30);
        let snap_a = snapshot(app.world_mut());
        assert!(
            snap_a.len() > 300,
            "expected a full rig snapshot, got {}",
            snap_a.len()
        );
        assert!(
            snap_a.iter().all(|v| v.is_finite()),
            "all rig transforms finite"
        );

        let mut app2 = bridge_app();
        drive(&mut app2, 30);
        let snap_b = snapshot(app2.world_mut());
        assert_eq!(snap_a, snap_b, "double run must be bit-identical");
    }

    /// Spec: `getMuzzle` / `getHeadPosition` world coords sane relative to
    /// the sim root; `aimReady` in 0..1.
    #[test]
    fn muzzle_and_head_resolve_in_world() {
        let mut app = bridge_app();
        drive(&mut app, 10);
        let bridge = app.world().resource::<CharBridge>();
        let demo = app.world().resource::<DemoSim>();
        for slot in 0..demo.m.actors.len() {
            let a = &demo.m.actors[slot];
            let root = Vec3::from_array(a.pos.to_array());
            let head = bridge.get_head_position(slot).expect("attached");
            assert!(head.is_finite());
            assert!(
                (head - root).length() < 3.0,
                "slot {slot}: head {head} vs {root}"
            );
            let mz = bridge.get_muzzle(slot).expect("attached");
            assert!(
                mz.is_finite() && (mz - root).length() < 3.5,
                "slot {slot}: muzzle {mz}"
            );
            let hand = bridge.get_muzzle_hand(slot, 1).expect("attached");
            assert!(hand.is_finite());
            let ar = bridge.aim_ready(slot);
            assert!((0.0..=1.0).contains(&ar), "slot {slot}: aim_ready {ar}");
        }
    }

    /// Spec: `!alive` hides every part entity (0.19 `Visible` overrides a
    /// Hidden parent, so the hiding is per entity).
    #[test]
    fn death_hides_every_part_entity() {
        let mut app = bridge_app();
        drive(&mut app, 3);
        app.world_mut().resource_mut::<DemoSim>().m.actors[0].alive = false;
        app.update();
        let parts: Vec<Entity> = app
            .world()
            .resource::<CharBridge>()
            .slot_parts(0)
            .iter()
            .map(|p| p.entity)
            .collect();
        assert!(!parts.is_empty(), "slot 0 attached");
        for e in parts {
            assert_eq!(
                app.world().get::<Visibility>(e).copied(),
                Some(Visibility::Hidden),
                "dead slot must hide every part"
            );
        }
    }

    /// Spec: the draw-call budget — the game-tier tree (kid rows + the held
    /// weapon's near rows) stays within the CONTRACTS §1 ≤45 part budget.
    #[test]
    fn part_budget_within_45_per_slot() {
        let mut app = bridge_app();
        drive(&mut app, 2);
        let bridge = app.world().resource::<CharBridge>();
        for slot in 0..8 {
            let n = bridge.slot_part_count(slot);
            assert!(n > 0 && n <= 45, "slot {slot}: {n} parts out of budget");
        }
    }

    /// Spec: `paused` freezes the pose — no bridge writes while paused.
    #[test]
    fn paused_freezes_pose() {
        let mut app = bridge_app();
        drive(&mut app, 5);
        let before = snapshot(app.world_mut());
        app.world_mut().resource_mut::<PlayerControls>().paused = true;
        drive(&mut app, 5);
        assert_eq!(
            before,
            snapshot(app.world_mut()),
            "pause must freeze rig writes"
        );
    }

    /// Footstep queue: scripted walking must eventually emit touchdown
    /// edges. The test itself drains the queue; the audio consumer hook is
    /// DEFERRED (D-T54-5③), so there is no production consumer yet.
    #[test]
    fn footstep_queue_emits_touchdowns() {
        let mut app = bridge_app();
        drive(&mut app, 30);
        let mut bridge = app.world_mut().resource_mut::<CharBridge>();
        let events = bridge.take_footsteps();
        // Not a hard requirement per frame, but 30 walking frames must
        // produce at least one footfall across the roster.
        assert!(!events.is_empty(), "footstep edges expected while walking");
        for e in events {
            assert!(e.pos.is_finite() && e.speed.is_finite() && e.foot < 2);
        }
    }

    /// Twins mirror rows (det = −1) and degenerate rows decompose totally —
    /// no panic, no NaN (the C1 `from_matrix` approximation is replaced).
    #[test]
    fn mirror_and_degenerate_rows_never_panic() {
        let m = [-1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 1.0, -4.0];
        assert!(det3(&m) < 0.0);
        let t = decompose_row(&m);
        assert!(t.translation.is_finite() && t.rotation.is_finite() && t.scale.is_finite());
        let (a, b) = (t.to_matrix(), row_to_mat4(&m));
        let (ac, bc) = (a.to_cols_array_2d(), b.to_cols_array_2d());
        for i in 0..4 {
            for j in 0..4 {
                assert!(
                    (ac[i][j] - bc[i][j]).abs() < 1e-5,
                    "mirror roundtrip [{i}][{j}]"
                );
            }
        }
        let t = decompose_row(&[0.0f32; 12]);
        assert!(t.translation.is_finite() && t.rotation.is_finite() && t.scale.is_finite());
    }
}
