//! Collision world built from extracted stage geometry, plus the query API used
//! by the character controller (Task 5) and projectiles/ink (Task 6/7).
//!
//! Faithful port of:
//!   - `src/world/level.js`  `Level._addBlock` / `_buildHash` / `_buildFaces`
//!     / `groundHeight` (block frames, face ids, spatial hash), and
//!   - `src/game/physics.js` `raycast` / `groundProbe` / `_railFeet` /
//!     `collideCapsule` / `collideBody` / `bodyFits`.
//!
//! Keep this file in lockstep with those JS functions on upstream sync; the
//! Task 4 tests pin heights/distances to <1 mm against hand-computed values.

use glam::{Vec3, vec3};

use crate::geometry::{Bounds, Brush, StageLayout};

/// Minimum surface `normal.y` a character can stand on (JS `WALKABLE`, ≈47°).
pub const WALKABLE: f32 = 0.68;
/// Broadphase spatial-hash cell size in metres (JS `hashCell`).
const HASH_CELL: f32 = 4.0;

/// One collider block: an oriented box (axis-aligned / turned / ramp slab).
#[derive(Debug, Clone)]
pub struct Block {
    pub center: Vec3,
    pub half: Vec3,
    /// Local axes as world directions: [side/x, normal/y, slope/z].
    pub axes: [Vec3; 3],
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    pub solid: bool,
    /// Kids walk on grates; squids/shots/ink pass through (rails included).
    pub grate: bool,
    pub rail: bool,
    /// Collision-only (no faces).
    pub hidden: bool,
    /// Face id per (axis, sign); -1 when absent.
    pub faces: [i32; 6],
    // build-time paint flags (consumed while faces are built)
    paint: bool,
    roof: bool,
    perch: bool,
    no_paint: Vec<Vec3>,
    murals: Vec<(u32, Vec3)>,
}

impl Block {
    /// Off-limits top (landing slides the actor off). Read by the nav graph;
    /// kept private like the other build-time flags.
    #[must_use]
    pub fn is_roof(&self) -> bool {
        self.roof
    }
}

/// One exposed block face. Ids match the upstream `level.faces` build order.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub id: u32,
    pub block: u32,
    pub n: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub origin: Vec3,
    pub su: f32,
    pub sv: f32,
    pub wall: bool,
    pub turf: bool,
    pub ceiling: bool,
    pub paintable: bool,
    pub grounded_bottom: bool,
    /// Mural decal id, or -1.
    pub mural: i32,
}

/// Ray/segment intersection result.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub hit: bool,
    pub dist: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub block: i32,
    pub face: i32,
    pub u: f32,
    pub v: f32,
}

impl Default for Hit {
    fn default() -> Self {
        Self {
            hit: false,
            dist: 0.0,
            point: Vec3::ZERO,
            normal: Vec3::ZERO,
            block: -1,
            face: -1,
            u: 0.0,
            v: 0.0,
        }
    }
}

/// Ground probe result under a character's feet.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GroundHit {
    pub hit: bool,
    pub y: f32,
    pub normal: Vec3,
    pub block: i32,
    pub face: i32,
    pub u: f32,
    pub v: f32,
    /// Support came from the centre sample (exact slope follow) vs a ring foot.
    pub center: bool,
    pub grate: bool,
}

impl Default for GroundHit {
    fn default() -> Self {
        Self {
            hit: false,
            y: 0.0,
            normal: Vec3::Y,
            block: -1,
            face: -1,
            u: 0.0,
            v: 0.0,
            center: false,
            grate: false,
        }
    }
}

/// Contacts reported by capsule resolution.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Contacts {
    pub ground: bool,
    pub wall: bool,
    pub ceiling: bool,
    pub ground_normal: Vec3,
    pub wall_normal: Vec3,
    pub ground_block: i32,
    pub wall_block: i32,
}

impl Contacts {
    fn reset(&mut self) {
        self.ground = false;
        self.wall = false;
        self.ceiling = false;
        self.ground_normal = Vec3::Y;
        self.wall_normal = Vec3::ZERO;
        self.ground_block = -1;
        self.wall_block = -1;
    }
}

/// Static collision geometry for one stage.
#[derive(Clone)]
pub struct CollisionWorld {
    pub blocks: Vec<Block>,
    pub faces: Vec<Face>,
    pub bounds: Bounds,
    pub has_rails: bool,
    hx0: f32,
    hz0: f32,
    hw: usize,
    hash: Vec<Vec<u32>>,
}

impl CollisionWorld {
    /// Build the collision world for the embedded Tidewater layout.
    #[must_use]
    pub fn tidewater() -> Self {
        Self::from_layout(&crate::embedded_tidewater())
    }

    #[must_use]
    pub fn from_layout(layout: &StageLayout) -> Self {
        Self::from_brushes(&layout.primitives, layout.stage.bounds)
    }

    #[must_use]
    pub fn from_brushes(brushes: &[Brush], bounds: Bounds) -> Self {
        let mut blocks: Vec<Block> = brushes.iter().map(build_block).collect();
        let has_rails = blocks.iter().any(|b| b.rail);

        let hx0 = bounds.min_x - 8.0;
        let hz0 = bounds.min_z - 8.0;
        let hw = ((bounds.max_x - bounds.min_x + 16.0) / HASH_CELL).ceil() as usize;
        let hd = ((bounds.max_z - bounds.min_z + 16.0) / HASH_CELL).ceil() as usize;
        let mut hash = vec![Vec::new(); hw * hd];
        for (id, b) in blocks.iter().enumerate() {
            let x0 = hxi(b.aabb_min.x, hx0, hw);
            let x1 = hxi(b.aabb_max.x, hx0, hw);
            let z0 = hzi(b.aabb_min.z, hz0, hd);
            let z1 = hzi(b.aabb_max.z, hz0, hd);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    hash[z * hw + x].push(id as u32);
                }
            }
        }

        let faces = build_faces(&mut blocks, &hash, hw, hx0, hz0);
        Self {
            blocks,
            faces,
            bounds,
            has_rails,
            hx0,
            hz0,
            hw,
            hash,
        }
    }

    // ------------------------------------------------------------ broadphase

    /// Unique block ids whose hash cells overlap the XZ rectangle; `out` is
    /// cleared first (same contract as JS `queryBlocks`).
    pub fn query_blocks(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32, out: &mut Vec<u32>) {
        query_into(
            &self.hash, self.hw, self.hx0, self.hz0, min_x, min_z, max_x, max_z, out,
        );
    }

    // --------------------------------------------------------- point queries

    /// Is `p` within block `id` (optionally padded)?
    pub fn point_in_block(&self, id: u32, p: Vec3, pad: f32) -> bool {
        point_in_block(&self.blocks[id as usize], p, pad)
    }

    /// Is `p` inside any solid block other than `exclude`?
    pub fn point_inside(&self, p: Vec3, pad: f32, exclude: i32, scratch: &mut Vec<u32>) -> bool {
        point_inside(
            &self.blocks,
            &self.hash,
            self.hw,
            self.hx0,
            self.hz0,
            p,
            pad,
            exclude,
            scratch,
        )
    }

    // --------------------------------------------------------------- ground

    /// Highest walkable block-top plane under (x,z), at or below `y_max`.
    /// Port of `Level.groundHeight`. Returns -∞ when nothing is below.
    pub fn ground_height(&self, x: f32, z: f32, y_max: f32, skip_grates: bool) -> f32 {
        let mut best = f32::NEG_INFINITY;
        let mut ids = Vec::new();
        self.query_blocks(x - 0.01, z - 0.01, x + 0.01, z + 0.01, &mut ids);
        for id in ids {
            let b = &self.blocks[id as usize];
            if !b.solid || (skip_grates && b.grate) {
                continue;
            }
            let n = b.axes[1];
            if n.y < 0.5 {
                continue;
            }
            let top = b.center + n * b.half.y;
            let y = top.y - (n.x * (x - top.x) + n.z * (z - top.z)) / n.y;
            if y <= y_max && y > best && self.point_in_block(id, vec3(x, y - 0.01, z), 0.001) {
                best = y;
            }
        }
        best
    }

    // ------------------------------------------------------------- raycasts

    /// Ray vs all solid blocks; `dir` must be normalized. Blocks containing the
    /// origin are ignored (same as JS). `skip_grates` passes through grates.
    #[must_use]
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32, skip_grates: bool) -> Hit {
        let mut out = Hit {
            dist: max_dist,
            ..Hit::default()
        };
        let ex = origin + dir * max_dist;
        let mut ids = Vec::new();
        self.query_blocks(
            origin.x.min(ex.x),
            origin.z.min(ex.z),
            origin.x.max(ex.x),
            origin.z.max(ex.z),
            &mut ids,
        );
        let mut best = max_dist;
        let mut best_k = 0usize;
        let mut best_sign = 0.0f32;
        let mut best_b: i32 = -1;
        for id in ids {
            let b = &self.blocks[id as usize];
            if !b.solid || (skip_grates && b.grate) {
                continue;
            }
            let o = origin - b.center;
            let mut tmin = f32::NEG_INFINITY;
            let mut tmax = f32::INFINITY;
            let mut kmin = 0usize;
            let mut smin = 0.0f32;
            let mut miss = false;
            for k in 0..3 {
                let ax = b.axes[k];
                let ok = o.dot(ax);
                let dk = dir.dot(ax);
                let h = [b.half.x, b.half.y, b.half.z][k];
                if dk.abs() < 1e-9 {
                    if ok < -h || ok > h {
                        miss = true;
                        break;
                    }
                    continue;
                }
                let mut t1 = (-h - ok) / dk;
                let mut t2 = (h - ok) / dk;
                let mut s1 = -1.0f32;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                    s1 = 1.0;
                }
                if t1 > tmin {
                    tmin = t1;
                    kmin = k;
                    smin = s1;
                }
                tmax = tmax.min(t2);
                if tmin > tmax {
                    miss = true;
                    break;
                }
            }
            if miss || tmax < 0.0 || tmin < 0.0 || tmin > best {
                continue;
            }
            best = tmin;
            best_k = kmin;
            best_sign = smin;
            best_b = id as i32;
        }
        if best_b < 0 {
            return out;
        }
        let b = &self.blocks[best_b as usize];
        out.hit = true;
        out.dist = best;
        out.block = best_b;
        out.point = origin + dir * best;
        out.normal = b.axes[best_k] * best_sign;
        out.face = b.faces[best_k * 2 + if best_sign > 0.0 { 0 } else { 1 }];
        if out.face >= 0 {
            let f = self.faces[out.face as usize];
            let p = out.point - f.origin;
            out.u = p.dot(f.u);
            out.v = p.dot(f.v);
        }
        out
    }

    /// Segment `a -> b` intersection.
    #[must_use]
    pub fn segment(&self, a: Vec3, b: Vec3, skip_grates: bool) -> Hit {
        let ab = b - a;
        let len = ab.length();
        if len < 1e-6 {
            return Hit::default();
        }
        self.raycast(a, ab / len, len, skip_grates)
    }

    /// Line of sight (true = clear), stopping 5 cm short of the target.
    #[must_use]
    pub fn los(&self, a: Vec3, b: Vec3) -> bool {
        let ab = b - a;
        let len = ab.length();
        if len < 1e-4 {
            return true;
        }
        !self.raycast(a, ab / len, len - 0.05, true).hit
    }

    // -------------------------------------------------------- ground probing

    /// Flat-footprint ground probe (JS `Physics.groundProbe`): centre ray plus
    /// an 8-point ring of radius `foot`, cast from `y + up` down to `y - down`.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn ground_probe(
        &self,
        x: f32,
        y: f32,
        z: f32,
        up: f32,
        down: f32,
        foot: f32,
        skip_grates: bool,
    ) -> GroundHit {
        self.ground_probe_step(x, y, z, up, down, foot, skip_grates, 0.12)
    }

    /// Same as [`ground_probe`](Self::ground_probe) with an explicit `step_min`
    /// (JS default 0.12: ring samples must clear the centre surface by this).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn ground_probe_step(
        &self,
        x: f32,
        y: f32,
        z: f32,
        up: f32,
        down: f32,
        foot: f32,
        skip_grates: bool,
        step_min: f32,
    ) -> GroundHit {
        let mut out = GroundHit::default();
        let len = up + down;
        let mut cy = f32::NEG_INFINITY;
        let mut have = false;
        let h = self.raycast(vec3(x, y + up, z), -Vec3::Y, len, skip_grates);
        if h.hit && h.normal.y >= WALKABLE {
            have = true;
            cy = h.point.y;
            fill_ground(&mut out, &h, true, self);
        }
        // ring: best clear step above the centre, or the highest sample if none
        let mut by = if have {
            cy + step_min
        } else {
            f32::NEG_INFINITY
        };
        let mut found = false;
        for i in 0..8 {
            let a = (i as f32 / 8.0) * std::f32::consts::TAU;
            let hr = self.raycast(
                vec3(x + a.cos() * foot, y + up, z + a.sin() * foot),
                -Vec3::Y,
                len,
                skip_grates,
            );
            if !hr.hit || hr.normal.y < WALKABLE {
                continue;
            }
            if hr.point.y > by {
                by = hr.point.y;
                found = true;
                fill_ground(&mut out, &hr, false, self);
            }
        }
        out.hit = have || found;
        out
    }

    /// Railing foot support (JS `Actor._railFeet`): rail tops are only a few cm
    /// thick, so the flat footprint circle can hold a kid where the ring rays
    /// straddle the rail. Mutates `out` when a higher rail support exists.
    pub fn rail_feet(&self, x: f32, z: f32, lo: f32, hi: f32, foot: f32, out: &mut GroundHit) {
        if !self.has_rails {
            return;
        }
        let mut ids = Vec::new();
        self.query_blocks(
            x - foot - 0.05,
            z - foot - 0.05,
            x + foot + 0.05,
            z + foot + 0.05,
            &mut ids,
        );
        let mut best_y = if out.hit {
            out.y + 1e-3
        } else {
            f32::NEG_INFINITY
        };
        let mut best_block: i32 = -1;
        for id in ids {
            let b = &self.blocks[id as usize];
            if !b.rail || !b.solid || b.axes[1].y < 0.999 {
                continue;
            }
            let top_y = b.center.y + b.half.y;
            if top_y < lo || top_y > hi || top_y <= best_y {
                continue;
            }
            let dx = x - b.center.x;
            let dz = z - b.center.z;
            let ex = ((dx * b.axes[0].x + dz * b.axes[0].z).abs() - b.half.x).max(0.0);
            let ez = ((dx * b.axes[2].x + dz * b.axes[2].z).abs() - b.half.z).max(0.0);
            if ex * ex + ez * ez > foot * foot {
                continue;
            }
            best_y = top_y;
            best_block = id as i32;
        }
        if best_block < 0 {
            return;
        }
        out.hit = true;
        out.y = best_y;
        out.normal = Vec3::Y;
        out.block = best_block;
        out.face = -1;
        out.u = 0.0;
        out.v = 0.0;
        out.center = false;
        out.grate = true;
    }

    // --------------------------------------------------- capsule resolution

    /// Resolve a vertical capsule (feet at `pos`, spanning
    /// y+radius .. y+height-radius) against the level. Mutates `pos`.
    pub fn collide_capsule(
        &self,
        pos: &mut Vec3,
        radius: f32,
        height: f32,
        contacts: &mut Contacts,
        iterations: u32,
        squid: bool,
    ) {
        contacts.reset();
        let top = radius.max(height - radius);
        for _ in 0..iterations {
            let mut ids = Vec::new();
            self.query_blocks(
                pos.x - radius - 0.2,
                pos.z - radius - 0.2,
                pos.x + radius + 0.2,
                pos.z + radius + 0.2,
                &mut ids,
            );
            let mut moved = false;
            for id in ids {
                let b = &self.blocks[id as usize];
                if !b.solid || (squid && b.grate) {
                    continue;
                }
                if pos.y + height < b.aabb_min.y - 0.05 || pos.y > b.aabb_max.y + 0.05 {
                    continue;
                }
                let a = vec3(pos.x, pos.y + radius, pos.z);
                let bpt = vec3(pos.x, pos.y + top, pos.z);
                let Some((n, pen)) = capsule_penetration(b, a, bpt, radius) else {
                    continue;
                };
                *pos += n * (pen + 1e-4);
                moved = true;
                if n.y > 0.6 {
                    contacts.ground = true;
                    contacts.ground_normal = n;
                    contacts.ground_block = id as i32;
                } else if n.y < -0.6 {
                    contacts.ceiling = true;
                } else if n.y.abs() < 0.55 {
                    contacts.wall = true;
                    contacts.wall_normal = n;
                    contacts.wall_block = id as i32;
                }
            }
            if !moved {
                break;
            }
        }
    }

    /// Resolve the character body capsule, whose bottom is lifted `lift` above
    /// the feet (feet handle curbs/step-ups). `horizontal` (grounded) turns
    /// push-outs into sideways pushes so walls never fight the ground snap.
    #[allow(clippy::too_many_arguments)]
    pub fn collide_body(
        &self,
        pos: &mut Vec3,
        radius: f32,
        lift: f32,
        height: f32,
        contacts: &mut Contacts,
        horizontal: bool,
        skip_grates: bool,
        iterations: u32,
    ) {
        contacts.reset();
        let bot = lift + radius;
        let top = bot.max(height - radius);
        for _ in 0..iterations {
            let mut ids = Vec::new();
            self.query_blocks(
                pos.x - radius - 0.2,
                pos.z - radius - 0.2,
                pos.x + radius + 0.2,
                pos.z + radius + 0.2,
                &mut ids,
            );
            let mut moved = false;
            for id in ids {
                let b = &self.blocks[id as usize];
                if !b.solid || (skip_grates && b.grate) {
                    continue;
                }
                if pos.y + height < b.aabb_min.y - 0.05 || pos.y + lift > b.aabb_max.y + 0.05 {
                    continue;
                }
                let a = vec3(pos.x, pos.y + bot, pos.z);
                let bpt = vec3(pos.x, pos.y + top, pos.z);
                let Some((n, pen)) = capsule_penetration(b, a, bpt, radius) else {
                    continue;
                };
                let hl = n.x.hypot(n.z);
                if horizontal && hl > 0.3 && n.y > -0.6 {
                    let push = (pen / hl).min(0.45) + 1e-4;
                    let hn = vec3(n.x / hl, 0.0, n.z / hl);
                    pos.x += hn.x * push;
                    pos.z += hn.z * push;
                    contacts.wall = true;
                    contacts.wall_normal = hn;
                    contacts.wall_block = id as i32;
                } else {
                    *pos += n * (pen + 1e-4);
                    if n.y > 0.6 {
                        contacts.ground = true;
                        contacts.ground_normal = n;
                        contacts.ground_block = id as i32;
                    } else if n.y < -0.6 {
                        contacts.ceiling = true;
                    } else if n.y.abs() < 0.6 {
                        contacts.wall = true;
                        contacts.wall_normal = n;
                        contacts.wall_block = id as i32;
                    }
                }
                moved = true;
            }
            if !moved {
                break;
            }
        }
    }

    /// Would a body capsule at `pos` overlap solid geometry?
    #[must_use]
    pub fn body_fits(
        &self,
        pos: Vec3,
        radius: f32,
        lift: f32,
        height: f32,
        skip_grates: bool,
        margin: f32,
    ) -> bool {
        let bot = lift + radius;
        let top = bot.max(height - radius);
        let mut ids = Vec::new();
        self.query_blocks(
            pos.x - radius - 0.1,
            pos.z - radius - 0.1,
            pos.x + radius + 0.1,
            pos.z + radius + 0.1,
            &mut ids,
        );
        for id in ids {
            let b = &self.blocks[id as usize];
            if !b.solid || (skip_grates && b.grate) {
                continue;
            }
            if pos.y + height < b.aabb_min.y || pos.y + lift > b.aabb_max.y {
                continue;
            }
            let a = vec3(pos.x, pos.y + bot, pos.z);
            let bpt = vec3(pos.x, pos.y + top, pos.z);
            let q = closest_on_segment_block(b, a, bpt);
            let s = point_on_segment(a, bpt, q);
            if s.distance_squared(q) < (radius - margin).powi(2) {
                return false;
            }
        }
        true
    }
}

// ------------------------------------------------------------- free-function
// broadphase / point queries (also usable while faces are being built, before
// the CollisionWorld exists).

fn hxi(x: f32, hx0: f32, hw: usize) -> usize {
    (((x - hx0) / HASH_CELL).floor() as isize).clamp(0, hw as isize - 1) as usize
}
fn hzi(z: f32, hz0: f32, hd: usize) -> usize {
    (((z - hz0) / HASH_CELL).floor() as isize).clamp(0, hd as isize - 1) as usize
}

#[allow(clippy::too_many_arguments)]
fn query_into(
    hash: &[Vec<u32>],
    hw: usize,
    hx0: f32,
    hz0: f32,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    out: &mut Vec<u32>,
) {
    out.clear();
    let x0 = hxi(min_x, hx0, hw);
    let x1 = hxi(max_x, hx0, hw);
    let z0 = hzi(min_z, hz0, hash.len() / hw);
    let z1 = hzi(max_z, hz0, hash.len() / hw);
    for z in z0..=z1 {
        for x in x0..=x1 {
            for &id in &hash[z * hw + x] {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
}

fn point_in_block(b: &Block, p: Vec3, pad: f32) -> bool {
    let d = p - b.center;
    d.dot(b.axes[0]).abs() < b.half.x + pad
        && d.dot(b.axes[1]).abs() < b.half.y + pad
        && d.dot(b.axes[2]).abs() < b.half.z + pad
}

#[allow(clippy::too_many_arguments)]
fn point_inside(
    blocks: &[Block],
    hash: &[Vec<u32>],
    hw: usize,
    hx0: f32,
    hz0: f32,
    p: Vec3,
    pad: f32,
    exclude: i32,
    scratch: &mut Vec<u32>,
) -> bool {
    query_into(
        hash,
        hw,
        hx0,
        hz0,
        p.x - 0.01,
        p.z - 0.01,
        p.x + 0.01,
        p.z + 0.01,
        scratch,
    );
    scratch.iter().copied().any(|id| {
        id as i32 != exclude
            && blocks[id as usize].solid
            && point_in_block(&blocks[id as usize], p, pad)
    })
}

// ------------------------------------------------------------- construction

fn build_block(brush: &Brush) -> Block {
    use crate::geometry::Brush;
    let (center, half, axes, common) = match brush {
        Brush::Box { min, max, common } => (
            vec3(
                (min[0] + max[0]) / 2.0,
                (min[1] + max[1]) / 2.0,
                (min[2] + max[2]) / 2.0,
            ),
            vec3(
                (max[0] - min[0]) / 2.0,
                (max[1] - min[1]) / 2.0,
                (max[2] - min[2]) / 2.0,
            ),
            [Vec3::X, Vec3::Y, Vec3::Z],
            common,
        ),
        Brush::Obox {
            center,
            size,
            rot_y,
            common,
        } => {
            let a = rot_y.to_radians();
            let (c, s) = (a.cos(), a.sin());
            (
                vec3(center[0], center[1], center[2]),
                vec3(size[0] / 2.0, size[1] / 2.0, size[2] / 2.0),
                [vec3(c, 0.0, -s), Vec3::Y, vec3(s, 0.0, c)],
                common,
            )
        }
        Brush::Ramp {
            low,
            high,
            width,
            thickness,
            thin,
            common,
        } => {
            let low = vec3(low[0], low[1], low[2]);
            let high = vec3(high[0], high[1], high[2]);
            let svec = high - low;
            let len = svec.length();
            let s = svec / len;
            let flat = vec3(s.x, 0.0, s.z).normalize_or_zero();
            let mut side = Vec3::Y.cross(flat).normalize_or_zero();
            let mut n = s.cross(side).normalize_or_zero();
            if n.y < 0.0 {
                n = -n;
                side = -side;
            }
            let cos_t = n.y;
            let rise = high.y - low.y;
            let thick = if *thin {
                *thickness
            } else {
                thickness.max(rise * cos_t + 0.35)
            };
            const EXT: f32 = 0.6; // extend under the floor at the low end
            let a = low - s * EXT;
            let top_mid = (a + high) * 0.5;
            let center = top_mid - n * (thick / 2.0);
            let half = vec3(width / 2.0, thick / 2.0, (len + EXT) / 2.0);
            let mut axes = [side, n, s];
            if side.cross(n).dot(s) < 0.0 {
                axes[0] = -side; // keep right-handed: side x n == s
            }
            (center, half, axes, common)
        }
    };

    let mut aabb_min = vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut aabb_max = vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for i in 0..8 {
        let v = center
            + axes[0] * (if i & 1 == 1 { half.x } else { -half.x })
            + axes[1] * (if i & 2 == 2 { half.y } else { -half.y })
            + axes[2] * (if i & 4 == 4 { half.z } else { -half.z });
        aabb_min = aabb_min.min(v);
        aabb_max = aabb_max.max(v);
    }

    Block {
        center,
        half,
        axes,
        aabb_min,
        aabb_max,
        solid: common.solid,
        grate: common.grate,
        rail: common.rail,
        hidden: common.hidden,
        faces: [-1; 6],
        paint: common.paint,
        roof: common.roof,
        perch: common.perch,
        no_paint: common
            .no_paint
            .iter()
            .map(|n| vec3(n[0], n[1], n[2]))
            .collect(),
        murals: common
            .mural
            .iter()
            .map(|m| (m.id, vec3(m.n[0], m.n[1], m.n[2])))
            .collect(),
    }
}

/// Build every visible face, exactly like `Level._buildFaces` (same order/ids).
fn build_faces(
    blocks: &mut [Block],
    hash: &[Vec<u32>],
    hw: usize,
    hx0: f32,
    hz0: f32,
) -> Vec<Face> {
    let mut faces = Vec::new();
    let mut scratch = Vec::new();
    for bid in 0..blocks.len() {
        if blocks[bid].hidden {
            continue;
        }
        let b = blocks[bid].clone();
        let h = [b.half.x, b.half.y, b.half.z];
        for k in 0..3 {
            for sign in [1.0f32, -1.0] {
                let n = b.axes[k] * sign;
                if n.y < -0.5 && b.center.y - h[1] < 0.5 {
                    continue; // underside at floor level
                }
                let others: [usize; 2] = if k == 0 {
                    [1, 2]
                } else if k == 1 {
                    [0, 2]
                } else {
                    [0, 1]
                };
                let (ui, vi);
                if n.y.abs() < 0.5 {
                    // wall: v = in-plane axis most aligned with world up
                    vi = if b.axes[others[0]].y.abs() > b.axes[others[1]].y.abs() {
                        others[0]
                    } else {
                        others[1]
                    };
                    ui = if others[0] == vi {
                        others[1]
                    } else {
                        others[0]
                    };
                } else {
                    ui = if b.axes[others[0]].x.abs() >= b.axes[others[1]].x.abs() {
                        others[0]
                    } else {
                        others[1]
                    };
                    vi = if others[0] == ui {
                        others[1]
                    } else {
                        others[0]
                    };
                }
                let mut v = b.axes[vi];
                if n.y.abs() < 0.5 {
                    if v.y < 0.0 {
                        v = -v;
                    }
                } else if v.z < 0.0 && v.z.abs() > 0.3 {
                    v = -v;
                }
                let u = v.cross(n); // u x v = n
                let su = 2.0 * h[ui];
                let sv = 2.0 * h[vi];
                let origin = b.center + n * h[k] - u * (su / 2.0) - v * (sv / 2.0);
                // `face_hidden` uses a 1e-4 m closed-set tolerance: upstream
                // runs this f64 strict-containment test, where f64 noise on
                // exact block seams lands samples inside one neighbour; our f32
                // quantisation can put the same sample exactly on the seam.
                if face_hidden(
                    blocks,
                    hash,
                    hw,
                    hx0,
                    hz0,
                    bid,
                    origin,
                    u,
                    v,
                    n,
                    su,
                    sv,
                    &mut scratch,
                ) {
                    continue;
                }
                let mut paintable = b.paint && n.y > -0.5;
                let mut mural = -1;
                for &(mid, mn) in &b.murals {
                    if n.dot(mn) > 0.9 {
                        mural = mid as i32;
                    }
                }
                for np in &b.no_paint {
                    if n.dot(*np) > 0.9 {
                        paintable = false;
                    }
                }
                if (b.roof || b.perch) && n.y > 0.5 {
                    paintable = false;
                }
                let grounded_bottom = if n.y.abs() < 0.3 {
                    let p = origin + u * (su / 2.0) + v * -0.06 + n * 0.06;
                    point_inside(blocks, hash, hw, hx0, hz0, p, 0.0, bid as i32, &mut scratch)
                        || p.y < 0.02
                } else {
                    false
                };
                let id = faces.len() as u32;
                faces.push(Face {
                    id,
                    block: bid as u32,
                    n,
                    u,
                    v,
                    origin,
                    su,
                    sv,
                    wall: n.y.abs() < 0.3,
                    turf: n.y > 0.7,
                    ceiling: n.y < -0.5,
                    paintable,
                    grounded_bottom,
                    mural,
                });
                blocks[bid].faces[k * 2 + if sign > 0.0 { 0 } else { 1 }] = id as i32;
            }
        }
    }
    faces
}

/// All sample points just outside a face lie inside another solid block.
#[allow(clippy::too_many_arguments)]
fn face_hidden(
    blocks: &[Block],
    hash: &[Vec<u32>],
    hw: usize,
    hx0: f32,
    hz0: f32,
    block: usize,
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    n: Vec3,
    su: f32,
    sv: f32,
    scratch: &mut Vec<u32>,
) -> bool {
    let nu = ((su / 1.25).ceil() as i32).max(2);
    let nv = ((sv / 1.25).ceil() as i32).max(2);
    for j in 0..=nv {
        for i in 0..=nu {
            let uu = ((i as f32 / nu as f32) * su).clamp(0.05, su - 0.05);
            let vv = ((j as f32 / nv as f32) * sv).clamp(0.05, sv - 0.05);
            let p = origin + u * uu + v * vv + n * 0.03;
            if !point_inside(blocks, hash, hw, hx0, hz0, p, 1e-4, block as i32, scratch) {
                return false;
            }
        }
    }
    true
}

fn fill_ground(out: &mut GroundHit, h: &Hit, center: bool, world: &CollisionWorld) {
    out.y = h.point.y;
    out.normal = h.normal;
    out.block = h.block;
    out.face = h.face;
    out.u = h.u;
    out.v = h.v;
    out.center = center;
    out.grate = h.block >= 0 && world.blocks[h.block as usize].grate;
}

/// Closest point on a block's oriented box to point `p`.
fn closest_on_block(b: &Block, p: Vec3) -> Vec3 {
    let o = p - b.center;
    let mut q = b.center;
    for k in 0..3 {
        let h = [b.half.x, b.half.y, b.half.z][k];
        let d = o.dot(b.axes[k]).clamp(-h, h);
        q += b.axes[k] * d;
    }
    q
}

/// Closest point on segment a->b to point `q` (clamped to the segment).
fn point_on_segment(a: Vec3, bpt: Vec3, q: Vec3) -> Vec3 {
    let ab = bpt - a;
    let len2 = ab.length_squared().max(1e-6);
    let t = ((q - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// Iterative closest point of segment a->b to the block (3 iterations, as in
/// JS), then penetration normal/depth of that point against a capsule of
/// `radius`. Returns None when the segment is already clear.
fn capsule_penetration(b: &Block, a: Vec3, bpt: Vec3, radius: f32) -> Option<(Vec3, f32)> {
    let ab = bpt - a;
    let ab_len2 = ab.length_squared().max(1e-6);
    let mut t = 0.5f32;
    for _ in 0..3 {
        let s = a + ab * t;
        let q = closest_on_block(b, s);
        t = ((q - a).dot(ab) / ab_len2).clamp(0.0, 1.0);
    }
    let s = a + ab * t;
    let q = closest_on_block(b, s);
    let n = s - q;
    let dist = n.length();
    if dist > 1e-5 {
        if dist >= radius {
            return None;
        }
        Some((n / dist, radius - dist))
    } else {
        // segment point inside the box: least-penetration axis
        let o = s - b.center;
        let mut best_pen = f32::INFINITY;
        let mut normal = Vec3::ZERO;
        for k in 0..3 {
            let h = [b.half.x, b.half.y, b.half.z][k];
            let d = o.dot(b.axes[k]);
            let pk = h - d.abs();
            if pk < best_pen {
                best_pen = pk;
                normal = b.axes[k] * if d >= 0.0 { 1.0 } else { -1.0 };
            }
        }
        Some((normal, best_pen + radius))
    }
}

/// Closest point of block `b` to the segment a->b (returns the block point).
fn closest_on_segment_block(b: &Block, a: Vec3, bpt: Vec3) -> Vec3 {
    let ab = bpt - a;
    let len2 = ab.length_squared().max(1e-6);
    let mut t = 0.5f32;
    for _ in 0..3 {
        let s = a + ab * t;
        let q = closest_on_block(b, s);
        t = ((q - a).dot(ab) / len2).clamp(0.0, 1.0);
    }
    closest_on_block(b, a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Bounds, Brush, BrushCommon, Mural};

    const MM: f32 = 1e-3;

    fn flags() -> BrushCommon {
        BrushCommon {
            tag: None,
            color: "#dddddd".into(),
            pattern: 0,
            paint: true,
            solid: true,
            grate: false,
            rail: false,
            roof: false,
            perch: false,
            no_nav: false,
            hidden: false,
            bevel: None,
            no_paint: Vec::new(),
            mural: Vec::<Mural>::new(),
            oct: None,
        }
    }

    fn bx(min: [f32; 3], max: [f32; 3]) -> Brush {
        Brush::Box {
            min,
            max,
            common: flags(),
        }
    }
    fn grate_box(min: [f32; 3], max: [f32; 3]) -> Brush {
        let mut c = flags();
        c.grate = true;
        c.paint = false;
        Brush::Box {
            min,
            max,
            common: c,
        }
    }
    /// 4 m rise over 8 m run ramp at x=xc; surface y = (z+4)/4.
    fn ramp(xc: f32) -> Brush {
        Brush::Ramp {
            low: [xc, 0.0, -4.0],
            high: [xc, 2.0, 4.0],
            width: 2.0,
            thickness: 0.3,
            thin: true,
            common: flags(),
        }
    }

    const B: Bounds = Bounds {
        min_x: -10.0,
        max_x: 10.0,
        min_z: -10.0,
        max_z: 10.0,
    };

    // TR-4.1: flat box top height, hand computed.
    #[test]
    fn flat_box_ground_height() {
        let w = CollisionWorld::from_brushes(&[bx([-4.0, -1.0, -4.0], [4.0, 0.0, 4.0])], B);
        for (x, z) in [(0.0, 0.0), (3.9, 3.9), (-3.5, 2.1), (0.2, -3.7)] {
            let y = w.ground_height(x, z, 50.0, false);
            assert!((y - 0.0).abs() < MM, "flat top at ({x},{z}) = {y}");
        }
        // y_max clamps: asking below the surface yields nothing.
        assert_eq!(w.ground_height(0.0, 0.0, -0.5, false), f32::NEG_INFINITY);
        // outside the footprint: nothing.
        assert_eq!(w.ground_height(5.0, 5.0, 50.0, false), f32::NEG_INFINITY);
    }

    // TR-4.1: ramp high/middle/low points vs hand-computed (z+4)/4, <1 mm.
    #[test]
    fn ramp_ground_three_points() {
        let w = CollisionWorld::from_brushes(&[ramp(-3.0)], B);
        for z in [-3.0_f32, -1.5, 0.0, 1.5, 3.0] {
            let expected = (z + 4.0) / 4.0;
            let y = w.ground_height(-3.0, z, 50.0, false);
            assert!((y - expected).abs() < MM, "ramp z={z}: {y} != {expected}");
        }
        // slope plane normal: n.y = horizontal-run / slope-length = 8 / sqrt(68)
        let h = w.raycast(vec3(-3.0, 3.0, 0.0), -Vec3::Y, 6.0, false);
        assert!(h.hit);
        assert!((h.point.y - 1.0).abs() < MM);
        assert!((h.normal.y - 8.0 / 68.0_f32.sqrt()).abs() < MM);
        assert!((h.normal.z + 2.0 / 68.0_f32.sqrt()).abs() < MM);
    }

    // TR-4.1: mirrored ramps give identical heights at mirrored points.
    #[test]
    fn mirrored_ramp_heights() {
        let w = CollisionWorld::from_brushes(&[ramp(-3.0), ramp(3.0)], B);
        for z in [-3.5_f32, -2.0, -0.3, 1.1, 2.8, 3.6] {
            let ya = w.ground_height(-3.0, z, 50.0, false);
            let yb = w.ground_height(3.0, z, 50.0, false);
            assert!((ya - yb).abs() < MM, "mirror ramps z={z}: {ya} vs {yb}");
        }
    }

    // TR-4.1 on the real stage: spawn pads sit at y=2.4, and ground height is
    // mirror symmetric (x,z)->(-x,-z) across a dense grid, within 1 mm.
    #[test]
    fn tidewater_mirror_and_spawns() {
        let w = CollisionWorld::tidewater();
        assert_eq!(w.blocks.len(), 359);
        assert_eq!(w.faces.len(), 996, "face build pinned by Python crosscheck");
        assert!(w.has_rails);
        let ys = [-41.8_f32, 41.8];
        for z in ys {
            let y = w.ground_height(0.0, z, 50.0, false);
            assert!((y - 2.4).abs() < MM, "spawn pad z={z}: {y}");
        }
        for &x in &[-20.0_f32, -10.0, -5.0, -2.0, 0.0, 2.0, 5.0, 10.0, 20.0] {
            for &z in &[
                -40.0_f32, -30.0, -20.0, -10.0, -5.0, 0.0, 5.0, 10.0, 20.0, 30.0, 40.0,
            ] {
                let a = w.ground_height(x, z, 50.0, false);
                let b = w.ground_height(-x, -z, 50.0, false);
                match (a.is_finite(), b.is_finite()) {
                    (false, false) => {}
                    (true, true) => assert!((a - b).abs() < MM, "mirror ({x},{z}): {a} vs {b}"),
                    _ => panic!("mirror presence differs at ({x},{z}): {a} vs {b}"),
                }
            }
        }
    }

    // TR-4.2: vertical/horizontal rays hit the expected face with known
    // distance and outward normal.
    #[test]
    fn raycast_known_faces() {
        let w = CollisionWorld::from_brushes(&[bx([-2.0, -1.0, -2.0], [2.0, 0.0, 2.0])], B);
        // top
        let h = w.raycast(vec3(0.0, 2.0, 0.0), -Vec3::Y, 3.0, false);
        assert!(h.hit && (h.dist - 2.0).abs() < MM && h.normal == Vec3::Y);
        assert!(h.face >= 0);
        assert!(w.faces[h.face as usize].turf);
        assert!((h.point - vec3(0.0, 0.0, 0.0)).length() < MM);
        // +X wall, entered from the +X side travelling -X
        let h = w.raycast(vec3(5.0, -0.5, 0.0), -Vec3::X, 6.0, false);
        assert!(h.hit && (h.dist - 3.0).abs() < MM && h.normal == Vec3::X);
        assert!(h.face >= 0 && w.faces[h.face as usize].wall);
        // -X wall
        let h = w.raycast(vec3(-5.0, -0.5, 0.0), Vec3::X, 6.0, false);
        assert!(h.hit && (h.dist - 3.0).abs() < MM && h.normal == -Vec3::X);
        // pointing into open space: no hit
        assert!(!w.raycast(vec3(0.0, -0.5, 0.0), Vec3::Z, 6.0, false).hit);
        // los stops 5 cm short of the target, so a point resting exactly on
        // the surface is "visible"; only a target inside the wall is blocked.
        assert!(w.los(vec3(0.0, -0.5, 2.05), vec3(0.0, -0.5, 2.0)));
        assert!(!w.los(vec3(0.0, -0.5, 5.0), vec3(0.0, -0.5, 1.9)));
    }

    // TR-4.2: grates are transparent to skip-grate rays (squid shots/los).
    #[test]
    fn raycast_grate_filter() {
        let w = CollisionWorld::from_brushes(&[grate_box([-0.5, -0.2, -0.5], [0.5, 0.0, 0.5])], B);
        assert!(w.raycast(vec3(0.0, 1.0, 0.0), -Vec3::Y, 2.0, false).hit);
        assert!(!w.raycast(vec3(0.0, 1.0, 0.0), -Vec3::Y, 2.0, true).hit);
    }

    // TR-4.2 on the stage: the spawn ray lands on a turf face whose stored
    // normal equals the reported hit normal.
    #[test]
    fn tidewater_spawn_ray_face() {
        let w = CollisionWorld::tidewater();
        let h = w.raycast(vec3(0.0, 3.4, -41.8), -Vec3::Y, 2.0, false);
        assert!(h.hit);
        assert!((h.point.y - 2.4).abs() < MM);
        assert!(h.face >= 0);
        let f = w.faces[h.face as usize];
        assert!(f.turf);
        assert!((f.n - h.normal).length() < 1e-5);
    }

    // TR-4.3: capsule swept into a wall never crosses it and never leaves the
    // world bounds; wall contact is reported, feet stay on the floor.
    #[test]
    fn capsule_cannot_cross_wall() {
        let w = CollisionWorld::from_brushes(
            &[
                bx([-6.0, -1.0, -2.0], [6.0, 0.0, 2.0]), // floor
                bx([3.0, 0.0, -2.0], [3.5, 2.5, 2.0]),   // wall
            ],
            B,
        );
        let (r, ht) = (0.3_f32, 1.7_f32);
        let mut pos = vec3(-2.0, 0.0, 0.0);
        let mut c = Contacts::default();
        let mut touched_wall = false;
        for _ in 0..80 {
            pos.x += 0.15;
            w.collide_capsule(&mut pos, r, ht, &mut c, 3, false);
            assert!(pos.is_finite());
            assert!(pos.x <= 3.0 - r + 1e-3, "passed wall: x={}", pos.x);
            assert!(pos.x.abs() < 10.0 && pos.z.abs() < 10.0, "left bounds");
            assert!(pos.y.abs() < MM, "feet lifted by wall push: y={}", pos.y);
            touched_wall |= c.wall;
        }
        assert!(touched_wall);
        // embedded capsule pops out on top of the floor with ground contact
        let mut pos = vec3(0.0, -0.15, 0.0);
        w.collide_capsule(&mut pos, r, ht, &mut c, 3, false);
        assert!(
            c.ground && (pos.y - 0.0).abs() < MM,
            "ground pop y={}",
            pos.y
        );
    }

    // TR-4.3: body_fits discriminates free space from overlap.
    #[test]
    fn body_fits_basic() {
        let w = CollisionWorld::from_brushes(&[bx([-4.0, -1.0, -4.0], [4.0, 0.0, 4.0])], B);
        assert!(w.body_fits(vec3(-3.0, 0.0, 0.0), 0.3, 0.0, 1.7, false, 0.01));
        assert!(!w.body_fits(vec3(-3.0, -0.12, 0.0), 0.3, 0.0, 1.7, false, 0.01));
        // head under a low ceiling does not fit
        let w2 = CollisionWorld::from_brushes(
            &[
                bx([-4.0, -1.0, -4.0], [4.0, 0.0, 4.0]),
                bx([-4.0, 1.4, -4.0], [4.0, 1.8, 4.0]),
            ],
            B,
        );
        assert!(!w2.body_fits(vec3(-3.0, 0.0, 0.0), 0.3, 0.0, 1.7, false, 0.01));
    }

    // ground_probe: centre sample on flat ground, and a ring foot finding the
    // higher adjacent slab (0.30 m step > step_min 0.12).
    #[test]
    fn ground_probe_center_and_step_up() {
        let w = CollisionWorld::from_brushes(&[bx([-4.0, -1.0, -2.0], [4.0, 0.0, 2.0])], B);
        let g = w.ground_probe(0.0, 0.0, 0.0, 0.35, 0.45, 0.24, false);
        assert!(g.hit && g.center && (g.y - 0.0).abs() < MM);

        let w2 = CollisionWorld::from_brushes(
            &[
                bx([-4.0, -1.0, -2.0], [0.0, 0.0, 2.0]), // low slab y=0
                bx([0.0, -0.7, -2.0], [4.0, 0.3, 2.0]),  // high slab y=0.3
            ],
            B,
        );
        // standing on the low slab 15 cm from the seam, 24 cm foot overhangs
        let g = w2.ground_probe(-0.15, 0.0, 0.0, 0.35, 0.45, 0.24, false);
        assert!(
            g.hit && !g.center,
            "step support must come from a ring foot"
        );
        assert!((g.y - 0.3).abs() < MM, "ring step y={}", g.y);
        // centre wins on an exact slope: probe follows the ramp at y=1
        let w3 = CollisionWorld::from_brushes(&[ramp(-3.0)], B);
        let g = w3.ground_probe(-3.0, 1.0, 0.0, 0.35, 0.45, 0.24, false);
        assert!(
            g.hit && g.center && (g.y - 1.0).abs() < MM,
            "ramp center y={}",
            g.y
        );
    }

    // point_in_block / query_blocks sanity on a pair of disjoint boxes.
    #[test]
    fn point_and_block_queries() {
        let w = CollisionWorld::from_brushes(
            &[
                bx([-6.0, -1.0, -6.0], [-5.0, 0.0, -5.0]),
                bx([5.0, -1.0, 5.0], [6.0, 0.0, 6.0]),
            ],
            B,
        );
        let mut out = Vec::new();
        w.query_blocks(-6.1, -6.1, -4.9, -4.9, &mut out);
        assert_eq!(out, vec![0]);
        assert!(w.point_in_block(0, vec3(-5.5, -0.5, -5.5), 0.0));
        assert!(!w.point_in_block(0, vec3(0.0, -0.5, 0.0), 0.0));
        let mut s = Vec::new();
        assert!(w.point_inside(vec3(-5.5, -0.5, -5.5), 0.0, -1, &mut s));
        assert!(!w.point_inside(vec3(0.0, -0.5, 0.0), 0.0, -1, &mut s));
    }
}
