//! Port of `src/boss/bossNav.js` (Task 42 batch B).
//!
//! HULLBREAKER navigation: a 0.5 m grid over the stage with two clearance
//! fields — distance to the nearest WALL (solid blocks and raised decks taller
//! than it can stride over) and distance to the nearest DROP (sea, gaps, the
//! edge of the stage, the spawn pads). A pose is valid when the full footprint
//! (claws + legs) clears every wall and the body itself (shell + belly) is
//! over floor: on a narrow pier its legs may splay out over the water, but it
//! never wades in, never pushes a claw through a wall and never sets foot on a
//! spawn pad. Paths are A* over cells with room to walk and turn,
//! string-pulled into straight legs.
//!
//! Determinism: every floating-point expression keeps the JS operation order
//! (`// JS src/boss/bossNav.js L<line>` anchors below), `Math.round` is
//! reproduced as `floor(x + 0.5)` (ties toward +∞), the A* heap is the same
//! hand-rolled binary heap with the same push/pop and tie-breaking order, and
//! `random_point` takes the caller's rng so batch C can inject a seeded one.

use crate::actor::SimWorld;
use crate::collision::CollisionWorld;
use crate::tuning::{BossNavTuning, NavFootprint};
use glam::vec3;

/// JS `Math.round` — half toward **+∞** (`floor(x + 0.5)`), NOT Rust's
/// half-away-from-zero `f32::round`.
// JS src/boss/bossNav.js L39, L117, L171 (Math.round call sites)
fn js_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

/// One spawn keep-out circle (JS L27 `{ x, z, r: level.spawnBarrier }`).
#[derive(Debug, Clone, Copy)]
struct Pad {
    x: f32,
    z: f32,
    r: f32,
}

/// `cast` return value (JS L158 `{ dist, wall }`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cast {
    /// Distance the body travelled before the pose stopped fitting (m).
    pub dist: f32,
    /// A solid wall is what stopped it (vs an edge / the spawn pad).
    pub wall: bool,
}

/// BossNav grid cell kinds (JS L26): 0 floor · 1 wall (solid / raised) ·
/// 2 drop (sea, gap) · 3 spawn pad.
pub const KIND_FLOOR: u8 = 0;
pub const KIND_WALL: u8 = 1;
pub const KIND_DROP: u8 = 2;
pub const KIND_PAD: u8 = 3;

/// A 0.5 m navigation grid over the stage (JS `BossNav`, L18-234).
pub struct BossNav {
    /// Grid step (m) — JS L21 hard-codes `0.5` (not a tuning constant).
    step: f32,
    x0: f32,
    z0: f32,
    nx: usize,
    nz: usize,
    /// Ground height under each cell (JS L25 `this.floor`).
    floor: Vec<f32>,
    /// Cell kind (JS L26 `this.kind`).
    kind: Vec<u8>,
    /// Spawn keep-out circles, team-indexed (JS L27 `this.pads`).
    pads: Vec<Pad>,
    /// JS L28 `this.planR = PLAN_WALL`.
    plan_r: f32,
    /// `PLAN_FLOOR` (bossNav.js L13) — via tuning.
    plan_floor: f32,
    /// `WALL_BODY` (bossNav.js L10) — via tuning.
    wall_body: [NavFootprint; 2],
    /// `FLOOR_BODY` (bossNav.js L11) — via tuning.
    floor_body: [NavFootprint; 3],
    /// Main floor height, the 0.25 m histogram mode (JS L43 `this.floorY`).
    floor_y: f32,
    /// Distance to the nearest wall cell (JS L59 `this.Dw`).
    dw: Vec<f32>,
    /// Distance to the nearest drop/pad cell (JS L60 `this.Dv`).
    dv: Vec<f32>,
    /// Plannable cells (JS L87 `this.plan`).
    plan: Vec<u8>,
    /// Cell ids of the chosen connected plan region (JS L88 `this.planIds`).
    plan_ids: Vec<i32>,
    /// Plan region area in m² (JS L90 `this.area`).
    area: f32,
    /// Pre-computed, distance-sorted search offsets (JS L96-100 `this._offs`).
    /// f64 like the JS `Math.hypot` values, so the sort order and the stored
    /// `Float32Array` rounding match the original bit for bit.
    offs: Vec<(i32, i32, f64)>,
    // A* scratch buffers reused across `path` calls (JS L193-194
    // `this._g/_from/_seen/_closed/_stamp`); `g`/`from` are only read when
    // `seen == stamp`, so the reuse is safe.
    g_buf: Option<Vec<f32>>,
    from_buf: Option<Vec<i32>>,
    seen_buf: Option<Vec<u32>>,
    closed_buf: Option<Vec<u32>>,
    stamp: u32,
}

impl BossNav {
    /// JS L19-30 constructor. `world` supplies the collision geometry, spawn
    /// pads and barrier; every footprint / threshold comes from `nav` tuning
    /// (WALL_BODY / FLOOR_BODY / PAD_MARGIN / PLAN_WALL / PLAN_FLOOR / STRIDE).
    pub fn new(world: &SimWorld, nav: &BossNavTuning) -> Self {
        // JS L21-23: bounds + 0.5 m grid over the stage.
        let b = world.collision.bounds;
        let st = 0.5f32;
        let x0 = b.min_x + st / 2.0;
        let z0 = b.min_z + st / 2.0;
        let nx = ((b.max_x - b.min_x) / st).floor() as usize;
        let nz = ((b.max_z - b.min_z) / st).floor() as usize;
        let n = nx * nz;
        // JS L27: pads = level.spawnPads, r = level.spawnBarrier.
        let pads = world
            .spawn_pads
            .iter()
            .map(|p| Pad {
                x: p.x,
                z: p.z,
                r: world.spawn_barrier,
            })
            .collect();
        let mut me = Self {
            step: st,
            x0,
            z0,
            nx,
            nz,
            floor: vec![0.0; n], // JS L25 Float32Array(N)
            kind: vec![KIND_FLOOR; n],
            pads,
            plan_r: nav.plan_wall, // JS L28 planR = PLAN_WALL
            plan_floor: nav.plan_floor,
            wall_body: nav.wall_body,
            floor_body: nav.floor_body,
            floor_y: 0.0,
            dw: Vec::new(),
            dv: Vec::new(),
            plan: Vec::new(),
            plan_ids: Vec::new(),
            area: 0.0,
            offs: Self::build_offs(st),
            g_buf: None,
            from_buf: None,
            seen_buf: None,
            closed_buf: None,
            stamp: 0,
        };
        me.build(world.collision, nav);
        me
    }

    /// JS L32-91 `_build`.
    #[allow(clippy::needless_range_loop)]
    fn build(&mut self, col: &CollisionWorld, nav: &BossNavTuning) {
        let st = self.step;
        let n = self.nx * self.nz;
        let stride = nav.stride; // STRIDE (bossNav.js L14) via tuning

        // JS L34-43: main floor height — the most common ground level across
        // the stage in 0.25 m bins. JS uses a `Map` (insertion order); the Vec
        // keeps the same first-seen order so the `n > bn` tie-break matches.
        let mut hist: Vec<(i32, u32)> = Vec::new();
        for i in 0..n {
            let (x, z) = self.xz(i as i32);
            // JS L38: `L.groundHeight(x, z, 30)` — yMax 30, skipGrates false
            // (level.js L389 defaults).
            let g = col.ground_height(x, z, 30.0, false);
            if g > -5.0 {
                let k = js_round(g as f64 * 4.0); // JS L39 Math.round(g * 4)
                match hist.iter_mut().find(|e| e.0 == k) {
                    Some(e) => e.1 += 1,
                    None => hist.push((k, 1)),
                }
            }
        }
        // JS L41-43: `if (n > bn)` — strictly greater, first key wins ties.
        let mut best_key = 0i32;
        let mut bn: i64 = -1;
        for (k, cnt) in &hist {
            if (*cnt as i64) > bn {
                bn = *cnt as i64;
                best_key = *k;
            }
        }
        let best = best_key as f32 / 4.0;
        self.floor_y = best;

        // JS L44-58: per-cell kind + floor.
        let mut scratch = Vec::new();
        for i in 0..n {
            let (x, z) = self.xz64(i as i32);
            let (xf, zf) = (x as f32, z as f32);
            // JS L48: `L.groundHeight(x, z, best + STRIDE)` — anything low
            // enough to stride over is ground, taller blocks are walls.
            let g = col.ground_height(xf, zf, best + stride, false);
            let mut k = KIND_FLOOR;
            // JS L50 `if (!(g >= best - 0.55))` — the negated form keeps the
            // JS NaN semantics (a NaN ground read counts as a drop), so the
            // lint's `g < X` rewrite is NOT equivalent here.
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(g as f64 >= best as f64 - 0.55) {
                k = KIND_DROP;
            } else {
                self.floor[i] = g;
                // JS L53: four probe heights above the main floor; anything
                // solid there makes this cell a wall. `pointInside(p, 0)` →
                // pad 0, exclude -1 (level.js L147 defaults).
                for h in [stride + 0.1, 2.6, 3.8, 5.2] {
                    if col.point_inside(vec3(xf, best + h, zf), 0.0, -1, &mut scratch) {
                        k = KIND_WALL;
                        break;
                    }
                }
            }
            // JS L55: not a wall → the spawn pads (team-indexed PAD_MARGIN);
            // forEach keeps going, so the last matching pad wins.
            if k != KIND_WALL {
                for (t, pd) in self.pads.iter().enumerate() {
                    if (x - pd.x as f64).hypot(z - pd.z as f64) < pd.r as f64 + nav.padr[t] as f64 {
                        k = KIND_PAD;
                    }
                }
            }
            self.kind[i] = k;
            // JS L57: `if (k)` — every non-floor cell (1, 2 and 3) keeps the
            // body at the main floor height.
            if k != KIND_FLOOR {
                self.floor[i] = best;
            }
        }

        // JS L59-60: the two clearance fields.
        self.dw = self.field(|k| k == KIND_WALL, false);
        self.dv = self.field(|k| k >= KIND_DROP, true);

        // JS L61-90: plannable cells; keep one connected region — the boss's
        // home ground, big and away from the squad's spawn (team 0).
        let nx = self.nx as i32;
        let nz = self.nz as i32;
        let mut ok = vec![0u8; n];
        for i in 0..n {
            // JS L65: `ok[i] = Dw[i] >= PLAN_WALL && Dv[i] >= PLAN_FLOOR` (f64)
            ok[i] = if self.dw[i] as f64 >= self.plan_r as f64
                && self.dv[i] as f64 >= self.plan_floor as f64
            {
                1
            } else {
                0
            };
        }
        let mut comp = vec![-1i32; n];
        let mut best_c = -1i32;
        let mut best_score = 0.0f64;
        let mut c = 0i32;
        let pad = self.pads[0];
        // JS L68: span = diagonal of the stage bounds.
        let span = (col.bounds.max_x as f64 - col.bounds.min_x as f64)
            .hypot(col.bounds.max_z as f64 - col.bounds.min_z as f64);
        for i in 0..n as i32 {
            if ok[i as usize] == 0 || comp[i as usize] >= 0 {
                continue;
            }
            // JS L71-82: 8-neighbourhood flood on a LIFO stack (pop from the
            // end, push dz-outer / dx-inner — the exact pop order is what
            // fixes the accumulation order of the centroid sums).
            let mut size = 0usize;
            let mut sx = 0.0f64;
            let mut sz = 0.0f64;
            let mut stack = vec![i];
            comp[i as usize] = c;
            while let Some(k) = stack.pop() {
                size += 1;
                let (kx0, kz0) = self.xz64(k);
                sx += kx0;
                sz += kz0;
                let kx = k % nx;
                let kz = k / nx;
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let jx = kx + dx;
                        let jz = kz + dz;
                        if jx < 0 || jz < 0 || jx >= nx || jz >= nz {
                            continue;
                        }
                        let j = (jz * nx + jx) as usize;
                        if ok[j] == 1 && comp[j] < 0 {
                            comp[j] = c;
                            stack.push(j as i32);
                        }
                    }
                }
            }
            // JS L83: score = size * (0.4 + dist(centroid, pad0) / span)
            let score = size as f64
                * (0.4
                    + (sx / size as f64 - pad.x as f64).hypot(sz / size as f64 - pad.z as f64)
                        / span);
            if score > best_score {
                best_score = score;
                best_c = c;
            }
            c += 1;
        }
        self.plan = vec![0u8; n];
        self.plan_ids.clear();
        for i in 0..n {
            if ok[i] == 1 && comp[i] == best_c {
                self.plan[i] = 1;
                self.plan_ids.push(i as i32);
            }
        }
        self.area = self.plan_ids.len() as f32 * st * st; // JS L90
    }

    /// JS L94-113 `_field`: distance from each cell centre to the nearest
    /// matching cell's edge (brute force within 7 m; off-grid counts when
    /// `edge`).
    fn field(&self, matches: impl Fn(u8) -> bool, edge: bool) -> Vec<f32> {
        // JS L95: R = 14 cells (7 m) search radius.
        let r = 14i32;
        let n = self.nx * self.nz;
        let nx = self.nx as i32;
        let nz = self.nz as i32;
        let st = self.step;
        let mut d = vec![0.0f32; n];
        let st64 = st as f64;
        for iz in 0..nz {
            for ix in 0..nx {
                let i = (iz * nx + ix) as usize;
                if matches(self.kind[i]) {
                    d[i] = 0.0;
                    continue;
                }
                let mut dist = r as f64 * st64;
                for &(dx, dz, dd) in &self.offs {
                    let jx = ix + dx;
                    let jz = iz + dz;
                    let out = jx < 0 || jz < 0 || jx >= nx || jz >= nz;
                    // JS L108: `out ? edge : match(kind[j])`
                    let hit = if out {
                        edge
                    } else {
                        matches(self.kind[(jz * nx + jx) as usize])
                    };
                    if hit {
                        // JS L108 in f64, stored into the Float32Array (f32).
                        dist = (dd - st64 * 0.5).max(0.0);
                        break;
                    }
                }
                d[i] = dist as f32;
            }
        }
        d
    }

    /// JS L96-100: the distance-sorted offset list (built once, like the
    /// lazily-cached `this._offs`).
    fn build_offs(st: f32) -> Vec<(i32, i32, f64)> {
        let r = 14i32;
        let st64 = st as f64;
        let mut offs = Vec::new();
        for dz in -r..=r {
            for dx in -r..=r {
                let dd = (dx as f64).hypot(dz as f64) * st64;
                if dd <= r as f64 * st64 {
                    offs.push((dx, dz, dd));
                }
            }
        }
        // JS L99 `sort((a, b) => a[2] - b[2])` — V8 sort is stable; Rust's
        // `sort_by` is stable too, so equal distances keep the same push order.
        offs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
        offs
    }

    /// JS L115 `xz(i)`: cell centre coordinates. The JS arithmetic is f64;
    /// the f32 return rounds exactly (centres are multiples of 0.5 m).
    #[must_use]
    pub fn xz(&self, i: i32) -> (f32, f32) {
        let (x, z) = self.xz64(i);
        (x as f32, z as f32)
    }

    /// f64 cell centre (JS `this.x0 + (i % this.nx) * this.step`, f64).
    #[must_use]
    fn xz64(&self, i: i32) -> (f64, f64) {
        let nx = self.nx as i32;
        (
            self.x0 as f64 + (i % nx) as f64 * self.step as f64,
            self.z0 as f64 + (i / nx) as f64 * self.step as f64,
        )
    }

    /// JS L116-120 `cell(x, z)`: cell id, or -1 outside the grid.
    #[must_use]
    pub fn cell(&self, x: f32, z: f32) -> i32 {
        self.cell64(x as f64, z as f64)
    }

    /// f64 `cell` — the JS `Math.round((x - this.x0) / this.step)` runs in f64.
    #[must_use]
    fn cell64(&self, x: f64, z: f64) -> i32 {
        let ix = js_round((x - self.x0 as f64) / self.step as f64);
        let iz = js_round((z - self.z0 as f64) / self.step as f64);
        if ix < 0 || iz < 0 || ix >= self.nx as i32 || iz >= self.nz as i32 {
            return -1;
        }
        iz * self.nx as i32 + ix
    }

    /// JS L121 `wallClear`.
    #[must_use]
    pub fn wall_clear(&self, x: f32, z: f32) -> f32 {
        self.wall_clear64(x as f64, z as f64) as f32
    }

    #[must_use]
    fn wall_clear64(&self, x: f64, z: f64) -> f64 {
        let i = self.cell64(x, z);
        if i < 0 {
            0.0
        } else {
            self.dw[i as usize] as f64
        }
    }

    /// JS L122 `floorClear`.
    #[must_use]
    pub fn floor_clear(&self, x: f32, z: f32) -> f32 {
        self.floor_clear64(x as f64, z as f64) as f32
    }

    #[must_use]
    fn floor_clear64(&self, x: f64, z: f64) -> f64 {
        let i = self.cell64(x, z);
        if i < 0 {
            0.0
        } else {
            self.dv[i as usize] as f64
        }
    }

    /// JS L123 `kindAt`: off-grid reads as a drop.
    #[must_use]
    pub fn kind_at(&self, x: f32, z: f32) -> u8 {
        self.kind_at64(x as f64, z as f64)
    }

    #[must_use]
    fn kind_at64(&self, x: f64, z: f64) -> u8 {
        let i = self.cell64(x, z);
        if i < 0 {
            KIND_DROP
        } else {
            self.kind[i as usize]
        }
    }

    /// JS L124 `floorAt`: off-grid / non-floor cells sit at `floorY`, and any
    /// floor more than 0.45 m above the main floor snaps down to it.
    #[must_use]
    pub fn floor_at(&self, x: f32, z: f32) -> f32 {
        let i = self.cell64(x as f64, z as f64);
        let f = if i < 0 || self.kind[i as usize] != KIND_FLOOR {
            self.floor_y as f64
        } else {
            self.floor[i as usize] as f64
        };
        if f - (self.floor_y as f64) < 0.45 {
            f as f32
        } else {
            self.floor_y
        }
    }

    /// JS L125 `inPad(x, z, margin = 0)`.
    #[must_use]
    pub fn in_pad(&self, x: f32, z: f32, margin: f32) -> bool {
        let x = x as f64;
        let z = z as f64;
        let margin = margin as f64;
        self.pads
            .iter()
            .any(|p| (x - p.x as f64).hypot(z - p.z as f64) < p.r as f64 + margin)
    }

    /// JS L126 `isPlan`.
    #[must_use]
    pub fn is_plan(&self, x: f32, z: f32) -> bool {
        let i = self.cell64(x as f64, z as f64);
        i >= 0 && self.plan[i as usize] != 0
    }

    /// JS L129-134 `poseOk`: the whole footprint fits — claws/legs clear of
    /// walls, body over floor.
    #[must_use]
    pub fn pose_ok(&self, x: f32, z: f32, yaw: f32) -> bool {
        // JS f64: `const s = Math.sin(yaw), c = Math.cos(yaw)` then
        // `this.wallClear(x + s * b.z, z + c * b.z) < b.r`.
        let s = (yaw as f64).sin();
        let c = (yaw as f64).cos();
        let x = x as f64;
        let z = z as f64;
        for b in self.wall_body {
            if self.wall_clear64(x + s * b.z as f64, z + c * b.z as f64) < b.r as f64 {
                return false;
            }
        }
        for b in self.floor_body {
            if self.floor_clear64(x + s * b.z as f64, z + c * b.z as f64) < b.r as f64 {
                return false;
            }
        }
        true
    }

    /// JS L137-141 `turnOk`: turning on the spot — the claws must clear the
    /// walls at the new heading and the belly stays over floor (the ends may
    /// swing out over the water for a moment — a crab turning on a pier).
    #[must_use]
    pub fn turn_ok(&self, x: f32, z: f32, yaw: f32) -> bool {
        let s = (yaw as f64).sin();
        let c = (yaw as f64).cos();
        let x = x as f64;
        let z = z as f64;
        for b in self.wall_body {
            if self.wall_clear64(x + s * b.z as f64, z + c * b.z as f64) < b.r as f64 {
                return false;
            }
        }
        self.floor_clear64(x, z) >= self.floor_body[1].r as f64
    }

    /// JS L144-159 `cast`: how far the body can travel straight along `yaw`
    /// before the pose stops fitting; `wall` = a solid is what stopped it.
    /// The JS default `stepLen = 0.25` (L144) is the only value ever used.
    #[must_use]
    pub fn cast(&self, x: f32, z: f32, yaw: f32, max_dist: f32) -> Cast {
        let step_len = 0.25f64; // JS L144
        let s = (yaw as f64).sin();
        let c = (yaw as f64).cos();
        let x = x as f64;
        let z = z as f64;
        let max_dist = max_dist as f64;
        let mut d = 0.0f64;
        while d + step_len <= max_dist
            && self.pose_ok64(x + s * (d + step_len), z + c * (d + step_len), s, c)
        {
            d += step_len;
        }
        let mut wall = false;
        if d + step_len <= max_dist {
            // JS L150-156: what's just ahead of the claws — a wall bonks; an
            // edge or the pad makes it dig in and skid. 9-point fan, ±0.9 rad.
            let f = self.wall_body[0];
            let fx = x + s * (d + f.z as f64);
            let fz = z + c * (d + f.z as f64);
            for k in 0..=8usize {
                if wall {
                    break; // JS L152 `k <= 8 && !wall`
                }
                let a = -0.9 + (k as f64 / 8.0) * 1.8;
                let px = fx + (yaw as f64 + a).sin() * (f.r as f64 + 0.6);
                let pz = fz + (yaw as f64 + a).cos() * (f.r as f64 + 0.6);
                if self.kind_at64(px, pz) == KIND_WALL {
                    wall = true;
                }
            }
        }
        Cast {
            dist: d as f32,
            wall,
        }
    }

    /// `poseOk` with pre-computed `sin(yaw)` / `cos(yaw)` (JS L129-134 — the
    /// same test `cast` runs at every step; yaw enters only through s/c).
    #[must_use]
    fn pose_ok64(&self, x: f64, z: f64, s: f64, c: f64) -> bool {
        for b in self.wall_body {
            if self.wall_clear64(x + s * b.z as f64, z + c * b.z as f64) < b.r as f64 {
                return false;
            }
        }
        for b in self.floor_body {
            if self.floor_clear64(x + s * b.z as f64, z + c * b.z as f64) < b.r as f64 {
                return false;
            }
        }
        true
    }

    /// JS L161 `_cellOk(i, slack = 0)`.
    #[must_use]
    fn cell_ok(&self, i: i32, slack: f64) -> bool {
        i >= 0
            && self.dw[i as usize] as f64 >= self.plan_r as f64 - slack
            && self.dv[i as usize] as f64 >= self.plan_floor as f64 - slack
    }

    /// JS L163-167 `lineOk(x0, z0, x1, z1, slack = 0.3)`: a straight walk
    /// between two points stays on plannable ground.
    #[must_use]
    pub fn line_ok(&self, x0: f32, z0: f32, x1: f32, z1: f32, slack: f32) -> bool {
        // JS f64 arithmetic (L164-166).
        let (x0, z0, x1, z1) = (x0 as f64, z0 as f64, x1 as f64, z1 as f64);
        let slack = slack as f64;
        let d = (x1 - x0).hypot(z1 - z0);
        let n = ((d / (self.step as f64 * 0.8)).ceil() as i32).max(1) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            if !self.cell_ok(self.cell64(x0 + (x1 - x0) * t, z0 + (z1 - z0) * t), slack) {
                return false;
            }
        }
        true
    }

    /// JS L170-186 `nearestPlan`: the plannable cell nearest to (x, z)
    /// (Chebyshev ring search, `d = dx² + dz²`, first cell wins ties).
    #[must_use]
    pub fn nearest_plan(&self, x: f32, z: f32) -> i32 {
        let ix = js_round((x as f64 - self.x0 as f64) / self.step as f64);
        let iz = js_round((z as f64 - self.z0 as f64) / self.step as f64);
        let nx = self.nx as i32;
        let nz = self.nz as i32;
        for r in 0..60i32 {
            let mut best = -1i32;
            let mut bd = f32::INFINITY;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let jx = ix + dx;
                    let jz = iz + dz;
                    if jx < 0 || jz < 0 || jx >= nx || jz >= nz {
                        continue;
                    }
                    let j = jz * nx + jx;
                    if self.plan[j as usize] == 0 {
                        continue;
                    }
                    let d = (dx * dx + dz * dz) as f32;
                    if d < bd {
                        bd = d;
                        best = j;
                    }
                }
            }
            if best >= 0 {
                return best;
            }
        }
        // JS L185: `this.planIds[0] ?? -1`
        self.plan_ids.first().copied().unwrap_or(-1)
    }

    /// JS L189-230 `path`: A* over plannable cells (prefers open floor),
    /// string-pulled into a few straight legs. Returns cell-centre `[x, z]`
    /// points, or `None`.
    pub fn path(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) -> Option<Vec<[f32; 2]>> {
        let a = self.nearest_plan(x0, z0);
        let b = self.nearest_plan(x1, z1);
        if a < 0 || b < 0 {
            return None;
        }
        let n = self.nx * self.nz;
        let nx = self.nx as i32;
        let nz = self.nz as i32;
        // JS runs the whole A* in f64 (`_g` is a Float32Array — values are
        // rounded on store, comparisons happen in f64).
        let (gx0, gz0, gst) = (self.x0 as f64, self.z0 as f64, self.step as f64);
        // JS L193-194: lazily allocated scratch buffers reused across calls.
        if self.g_buf.is_none() {
            self.g_buf = Some(vec![0.0f32; n]);
            self.from_buf = Some(vec![-1i32; n]);
            self.seen_buf = Some(vec![0u32; n]);
            self.closed_buf = Some(vec![0u32; n]);
        }
        self.stamp += 1;
        let st = self.stamp;
        let xz =
            |i: i32| -> (f64, f64) { (gx0 + (i % nx) as f64 * gst, gz0 + (i / nx) as f64 * gst) };
        let (bx, bz) = xz(b);
        // JS L196: heuristic = straight-line distance to the goal cell centre.
        let h = |i: i32| -> f64 {
            let (x, z) = xz(i);
            (x - bx).hypot(z - bz)
        };
        let mut cells: Vec<i32> = Vec::new();
        {
            let g = self.g_buf.as_mut().unwrap();
            let from = self.from_buf.as_mut().unwrap();
            let seen = self.seen_buf.as_mut().unwrap();
            let closed = self.closed_buf.as_mut().unwrap();
            let plan = &self.plan;
            let dw = &self.dw;
            let dv = &self.dv;
            let mut heap = MinHeap::default();
            g[a as usize] = 0.0;
            from[a as usize] = -1;
            seen[a as usize] = st;
            heap.push(a, h(a));
            let mut it = 0u32;
            // JS L200: `while (heap.size && it++ < 40000)`
            while heap.size() > 0 && it < 40000 {
                it += 1;
                let cur = heap.pop();
                if cur == b {
                    break;
                }
                if closed[cur as usize] == st {
                    continue;
                }
                closed[cur as usize] = st;
                let cx = cur % nx;
                let cz = cur / nx;
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let jx = cx + dx;
                        let jz = cz + dz;
                        if jx < 0 || jz < 0 || jx >= nx || jz >= nz {
                            continue;
                        }
                        let j = (jz * nx + jx) as usize;
                        if plan[j] == 0 {
                            continue;
                        }
                        // JS L212: cost prefers open floor (penalty for tight
                        // wall / thin floor clearance at the neighbour).
                        let ng = g[cur as usize] as f64
                            + (dx as f64).hypot(dz as f64)
                                * gst
                                * (1.0
                                    + (5.0 - dw[j] as f64).max(0.0) * 0.1
                                    + (3.0 - dv[j] as f64).max(0.0) * 0.15);
                        if seen[j] != st || ng < g[j] as f64 {
                            seen[j] = st;
                            g[j] = ng as f32;
                            from[j] = cur;
                            heap.push(j as i32, ng + h(j as i32));
                        }
                    }
                }
            }
            if seen[b as usize] != st {
                return None;
            }
            // JS L217-219: backtrack, truncate at > 8000 cells, reverse.
            let mut k = b;
            while k != -1 {
                cells.push(k);
                if cells.len() > 8000 {
                    break;
                }
                k = from[k as usize];
            }
        }
        cells.reverse();
        // JS L220-228: string-pulling — greedy straight legs (`lineOk` with
        // the JS default slack 0.3, L163).
        let mut out = Vec::with_capacity(cells.len());
        {
            let (x, z) = self.xz(cells[0]);
            out.push([x, z]);
        }
        let mut i = 0usize;
        while i + 1 < cells.len() {
            let mut j = cells.len() - 1;
            let (ax, az) = self.xz(cells[i]);
            while j > i + 1 {
                let (qx, qz) = self.xz(cells[j]);
                if self.line_ok(ax, az, qx, qz, 0.3) {
                    break;
                }
                j -= 1;
            }
            let (px, pz) = self.xz(cells[j]);
            out.push([px, pz]);
            i = j;
        }
        Some(out)
    }

    /// JS L233 `randomPoint(rnd)`: a plannable point; `rnd()` → [0, 1). The
    /// rng is caller-supplied so batch C can drive it deterministically.
    #[must_use]
    pub fn random_point(&self, rnd: &mut impl FnMut() -> f32) -> [f32; 2] {
        // JS L233: `(rnd() * this.planIds.length) | 0` (truncate toward zero);
        // out of range (undefined) → [0, 0].
        let idx = (rnd() as f64 * self.plan_ids.len() as f64) as usize;
        match self.plan_ids.get(idx) {
            Some(&i) => {
                let (x, z) = self.xz(i);
                [x, z]
            }
            None => [0.0, 0.0],
        }
    }

    // ------------------------------------------------------------- accessors

    /// Main floor height (JS L43 `this.floorY`).
    #[must_use]
    pub fn floor_y(&self) -> f32 {
        self.floor_y
    }
    /// Cell ids of the plan region (JS L88 `this.planIds`).
    #[must_use]
    pub fn plan_ids(&self) -> &[i32] {
        &self.plan_ids
    }
    /// Plan region area in m² (JS L90 `this.area`).
    #[must_use]
    pub fn area(&self) -> f32 {
        self.area
    }
    /// Grid dimensions (JS L23 `this.nx / this.nz`).
    #[must_use]
    pub fn grid_size(&self) -> (usize, usize) {
        (self.nx, self.nz)
    }
    /// Grid step in m (JS L21, fixed 0.5).
    #[must_use]
    pub fn step(&self) -> f32 {
        self.step
    }

    /// Wall clearance of cell `i` (JS `this.Dw[i]`, read by boss.js
    /// `_spawnPoint` L83).
    #[must_use]
    pub fn dw_at(&self, i: i32) -> f32 {
        self.dw[i as usize]
    }

    /// Floor clearance of cell `i` (JS `this.Dv[i]`, read by boss.js
    /// `_spawnPoint` L83).
    #[must_use]
    pub fn dv_at(&self, i: i32) -> f32 {
        self.dv[i as usize]
    }

    /// Spawn pad centre, team-indexed (JS `this.pads[team]`, read by boss.js
    /// `_spawnPoint` L87).
    #[must_use]
    pub fn pad_center(&self, team: usize) -> (f32, f32) {
        let p = self.pads[team];
        (p.x, p.z)
    }
}

/// JS L236-260 `MinHeap`: the hand-rolled binary heap the A* uses. Push and
/// pop follow the exact JS sift steps and comparisons (`pr[j] <= p` on the way
/// up, `<` on the way down), so equal-priority entries pop in the same order
/// and the A* path is bit-deterministic against the JS original.
#[derive(Default)]
struct MinHeap {
    ids: Vec<i32>,
    pr: Vec<f64>,
}

impl MinHeap {
    /// JS L238 `get size()`.
    fn size(&self) -> usize {
        self.ids.len()
    }

    /// JS L239-244 `push(id, p)`.
    fn push(&mut self, id: i32, p: f64) {
        let mut i = self.ids.len();
        self.ids.push(id);
        self.pr.push(p);
        while i > 0 {
            let j = (i - 1) >> 1;
            if self.pr[j] <= p {
                break;
            }
            self.ids[i] = self.ids[j];
            self.pr[i] = self.pr[j];
            i = j;
        }
        self.ids[i] = id;
        self.pr[i] = p;
    }

    /// JS L245-258 `pop()`.
    fn pop(&mut self) -> i32 {
        let top = self.ids[0];
        let lid = self.ids.pop().unwrap();
        let lp = self.pr.pop().unwrap();
        if !self.ids.is_empty() {
            let mut i = 0usize;
            let n = self.ids.len();
            loop {
                let l = i * 2 + 1;
                let r = l + 1;
                // JS L247-252 sifts the smaller child up, left wins ties;
                // `mp` starts at `lp` (the moving leaf), not at `pr[i]`.
                let mut m = i;
                let mut mp = lp;
                if l < n && self.pr[l] < mp {
                    m = l;
                    mp = self.pr[l];
                }
                if r < n && self.pr[r] < mp {
                    m = r;
                }
                if m == i {
                    break;
                }
                self.ids[i] = self.ids[m];
                self.pr[i] = self.pr[m];
                i = m;
            }
            self.ids[i] = lid;
            self.pr[i] = lp;
        }
        top
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::SimWorld;

    /// Tidewater exactly as the `collision.rs` tests build it: the embedded
    /// layout through `CollisionWorld::from_layout` (turf mode).
    struct Tw {
        layout: crate::geometry::StageLayout,
        collision: CollisionWorld,
        tuning: crate::tuning::Tuning,
    }

    fn tw() -> Tw {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tuning = crate::embedded_tuning();
        Tw {
            layout,
            collision,
            tuning,
        }
    }

    fn tidewater_nav() -> (BossNav, Tw) {
        let t = tw();
        let world = SimWorld::new(&t.collision, &t.layout);
        let nav = BossNav::new(&world, &t.tuning.boss.nav);
        (nav, t)
    }

    /// JS-reference build smoke (values produced by the real
    /// `src/boss/bossNav.js` + `src/world/level.js` on the turf-filtered
    /// Tidewater layout, 2026-11-03): floorY −0.25, 112×188 grid, 7590 plan
    /// cells, area 1897.5 m².
    #[test]
    fn tidewater_build_matches_js() {
        let (nav, _t) = tidewater_nav();
        assert_eq!(nav.grid_size(), (112, 188));
        assert_eq!(nav.step(), 0.5);
        assert_eq!(nav.floor_y(), -0.25);
        assert_eq!(nav.plan_ids().len(), 7590);
        assert_eq!(nav.area(), 1897.5);
        assert!(!nav.plan_ids().is_empty());
    }

    /// Off-grid reads as a drop (JS L123); pad cells sit at floorY.
    #[test]
    fn kind_at_out_of_bounds_is_drop() {
        let (nav, t) = tidewater_nav();
        let b = t.collision.bounds;
        assert_eq!(nav.kind_at(b.min_x - 100.0, 0.0), KIND_DROP);
        assert_eq!(nav.kind_at(0.0, b.max_z + 100.0), KIND_DROP);
        assert_eq!(nav.cell(b.min_x - 1.0, 0.0), -1);
        // JS reference: (0.25, 0.25) is a wall cell, (3.25, −32.75) a plan cell
        assert_eq!(nav.kind_at(0.25, 0.25), KIND_WALL);
        assert_eq!(nav.kind_at(3.25, -32.75), KIND_FLOOR);
    }

    /// floorAt: non-floor cells and off-grid points return floorY; the 0.45 m
    /// snap keeps the body near the main floor (JS L124).
    #[test]
    fn floor_at_snaps_to_main_floor() {
        let (nav, t) = tidewater_nav();
        let b = t.collision.bounds;
        assert_eq!(nav.floor_at(b.min_x - 50.0, 0.0), nav.floor_y());
        // wall cell → floorY (JS reference: floorAt(0.25, 0.25) === -0.25)
        assert_eq!(nav.floor_at(0.25, 0.25), -0.25);
        // pad centre is kind 3 → floorY
        let pad0 = t.layout.stage.spawn_pads[0];
        assert_eq!(nav.floor_at(pad0[0], pad0[2]), nav.floor_y());
        // sampled floorAt never sits more than 0.45 m above floorY
        for &i in nav.plan_ids().iter().step_by(97) {
            let (x, z) = nav.xz(i);
            assert!(nav.floor_at(x, z) <= nav.floor_y() + 0.45);
        }
    }

    /// Spawn pads: `inPad` honours the margin argument (JS L125); the pad
    /// centre is never a valid pose (JS L129-134).
    #[test]
    fn pads_block_poses() {
        let (nav, t) = tidewater_nav();
        let pad0 = t.layout.stage.spawn_pads[0];
        let r = t.layout.stage.spawn_barrier;
        assert!(nav.in_pad(pad0[0], pad0[2], 0.0));
        assert!(!nav.in_pad(pad0[0], pad0[2] + r + 0.5, 0.0));
        assert!(nav.in_pad(pad0[0], pad0[2] + r + 0.5, 1.0));
        assert!(!nav.pose_ok(pad0[0], pad0[2], 0.0));
        assert!(!nav.turn_ok(pad0[0], pad0[2], 0.0));
        assert!(!nav.is_plan(pad0[0], pad0[2]));
    }

    /// The boss's home ground: some plan point admits a full footprint.
    #[test]
    fn plan_has_poseable_points() {
        let (nav, _t) = tidewater_nav();
        let hit = nav.plan_ids().iter().any(|&i| {
            let (x, z) = nav.xz(i);
            nav.pose_ok(x, z, 0.0)
        });
        assert!(hit, "no plan cell admits the full footprint");
    }

    /// nearestPlan: a plan cell maps to itself; the JS reference point
    /// (0.25, 0.25) — a wall cell — maps to the plan cell at (8.25, 7.75).
    #[test]
    fn nearest_plan_matches_js() {
        let (nav, _t) = tidewater_nav();
        let i = nav.cell(3.25, -32.75);
        assert_eq!(nav.nearest_plan(3.25, -32.75), i);
        let j = nav.nearest_plan(0.25, 0.25);
        assert_eq!(nav.xz(j), (8.25, 7.75));
    }

    /// A* reference pinning: the exact JS `path()` output for two fixed
    /// endpoint pairs (real `bossNav.js` + `level.js`, turf Tidewater).
    #[test]
    fn path_pinned_to_js_reference() {
        let (mut nav, _t) = tidewater_nav();
        let check = |nav: &mut BossNav, a: [f32; 2], b: [f32; 2], want: &[[f32; 2]]| {
            let got = nav.path(a[0], a[1], b[0], b[1]).expect("path exists");
            assert_eq!(got.len(), want.len(), "legs: {got:?} vs {want:?}");
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g[0] - w[0]).abs() < 1e-3 && (g[1] - w[1]).abs() < 1e-3,
                    "vertex {g:?} != {w:?}"
                );
                assert!(nav.is_plan(g[0], g[1]), "vertex off-plan {g:?}");
            }
        };
        // alpha-side → bravo-side (JS: [[3.25,-32.75],[12.75,4.25],[0.25,32.75]])
        check(
            &mut nav,
            [3.25, -32.75],
            [0.25, 32.75],
            &[[3.25, -32.75], [12.75, 4.25], [0.25, 32.75]],
        );
        // west → east through the middle (JS:
        // [[-19.75,0.25],[-1.75,12.75],[11.75,7.75],[20.25,0.25]])
        check(
            &mut nav,
            [-19.75, 0.25],
            [20.25, 0.25],
            &[[-19.75, 0.25], [-1.75, 12.75], [11.75, 7.75], [20.25, 0.25]],
        );
    }

    /// Path endpoints snap to `nearestPlan` and the result is short (the
    /// string-pull collapses thousands of cells into a few legs).
    #[test]
    fn path_endpoints_and_bounds() {
        let (mut nav, t) = tidewater_nav();
        let b = t.collision.bounds;
        let a = nav.nearest_plan(1.0, b.min_z + 10.0);
        let e = nav.nearest_plan(-2.0, b.max_z - 12.0);
        let (ax, az) = nav.xz(a);
        let (ex, ez) = nav.xz(e);
        let p = nav
            .path(ax + 0.13, az - 0.11, ex + 0.07, ez + 0.05)
            .unwrap();
        assert_eq!(p[0], [ax, az]);
        assert_eq!(*p.last().unwrap(), [ex, ez]);
        assert!(p.len() <= 12, "string-pull left {p:?}");
        for pt in &p {
            assert!(nav.is_plan(pt[0], pt[1]));
        }
        // unreachable query (off-grid both ends still snaps to plan, but an
        // empty plan would return the fallback) — sanity: no panic
        let _ = nav.path(0.0, 0.0, 0.0, 0.0);
    }

    /// cast: walking into a wall reports `wall` and stops on the 0.25 m
    /// lattice; a zero-length cast is trivially safe (JS L144-159).
    #[test]
    fn cast_detects_walls() {
        let (nav, _t) = tidewater_nav();
        assert_eq!(
            nav.cast(3.25, -32.75, 0.0, 0.0),
            Cast {
                dist: 0.0,
                wall: false
            }
        );
        // Scan plan points × 16 headings for a cast that bonks a wall.
        let mut wall_cast = None;
        'outer: for &i in nav.plan_ids().iter().step_by(31) {
            let (x, z) = nav.xz(i);
            for k in 0..16 {
                let yaw = k as f32 * std::f32::consts::TAU / 16.0;
                let c = nav.cast(x, z, yaw, 10.0);
                assert!((c.dist / 0.25).fract().abs() < 1e-3, "off-lattice {c:?}");
                if c.wall {
                    wall_cast = Some((x, z, yaw, c));
                    break 'outer;
                }
            }
        }
        let (x, z, yaw, c) = wall_cast.expect("no wall cast found on the whole grid");
        // the claw fan must really see a wall cell ahead of the stop point
        assert!(c.dist + 0.25 <= 10.0, "wall at max range: {c:?}");
        let s = yaw.sin();
        let cw = yaw.cos();
        let fx = x + s * (c.dist + 2.3);
        let fz = z + cw * (c.dist + 2.3);
        let seen = (0..=8).any(|k| {
            let a = -0.9 + (k as f32 / 8.0) * 1.8;
            nav.kind_at(fx + (yaw + a).sin() * 3.1, fz + (yaw + a).cos() * 3.1) == KIND_WALL
        });
        assert!(seen, "wall flag without a wall in the fan");
    }

    /// lineOk: adjacent plan cells connect; a line crossing the sea does not
    /// (JS L161-167).
    #[test]
    fn line_ok_walkable_vs_water() {
        let (nav, _t) = tidewater_nav();
        assert!(nav.line_ok(3.25, -32.75, 3.25, -32.25, 0.3));
        // find a drop cell and a plan cell; the straight line over water fails
        let mut drop = None;
        for i in 0..(nav.grid_size().0 * nav.grid_size().1) as i32 {
            let (x, z) = nav.xz(i);
            if nav.kind_at(x, z) == KIND_DROP {
                drop = Some((x, z));
                break;
            }
        }
        let (dx, dz) = drop.expect("Tidewater has sea cells");
        assert!(!nav.line_ok(3.25, -32.75, dx, dz, 0.3));
    }

    /// turnOk on the spot: some plan point turns freely at some heading
    /// (claws clear, belly over floor); the pad centre never does
    /// (covered in `pads_block_poses`).
    #[test]
    fn turn_ok_behaviour() {
        let (nav, _t) = tidewater_nav();
        let any = nav.plan_ids().iter().step_by(53).any(|&i| {
            let (x, z) = nav.xz(i);
            (0..8).any(|k| nav.turn_ok(x, z, k as f32 * std::f32::consts::TAU / 8.0))
        });
        assert!(any, "no plan cell allows turning on the spot");
    }

    /// randomPoint is deterministic for a given rng and always lands on the
    /// plan (JS L233).
    #[test]
    fn random_point_deterministic() {
        let (nav, _t) = tidewater_nav();
        let mut seed = 0x1234_5678u32;
        let mut lcg = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let a: Vec<[f32; 2]> = (0..20).map(|_| nav.random_point(&mut lcg)).collect();
        let mut seed = 0x1234_5678u32;
        let mut lcg2 = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let b: Vec<[f32; 2]> = (0..20).map(|_| nav.random_point(&mut lcg2)).collect();
        assert_eq!(a, b);
        for p in &a {
            assert!(nav.is_plan(p[0], p[1]));
        }
        // rnd() === 1.0 falls off the end of planIds → the JS [0, 0] fallback
        assert_eq!(nav.random_point(&mut || 1.0), [0.0, 0.0]);
    }

    /// MinHeap: the sift order for equal priorities must match the JS heap
    /// (the A* pin above depends on it).
    #[test]
    fn min_heap_tie_order() {
        let mut h = MinHeap::default();
        for (id, p) in [(10i32, 5.0f64), (11, 5.0), (12, 3.0), (13, 5.0), (14, 7.0)] {
            h.push(id, p);
        }
        // hand-traced JS sift order: 12 (lowest), then the 5.0 tie pops in
        // heap position order 11, 13, 10, then 14
        assert_eq!(h.pop(), 12);
        assert_eq!(h.pop(), 11);
        assert_eq!(h.pop(), 13);
        assert_eq!(h.pop(), 10);
        assert_eq!(h.pop(), 14);
        assert_eq!(h.size(), 0);
    }
}
