//! Navigation graph: walkable top-surface samples connected by walk / jump /
//! drop edges, with A* pathing (faithful port of `src/game/nav.js`, spec
//! Task 9 "航点图（台阶/斜坡连接）").
//!
//! The upstream module is ported near-verbatim; the sim-side plumbing differs
//! only in how the level queries are reached:
//!   - `NavGraph` ctor + `_build` (samples, water flags, edges) -> [`NavGraph::new`]
//!   - `_clear` / `_has` / `_openAbove` / `_blocked`           -> private helpers
//!   - `_prune` (two-way spawn reachability, `validIds`)       -> [`NavGraph::prune`]
//!   - `nearest` (ring-expanding cell search)                  -> [`NavGraph::nearest`]
//!   - `path` (A* + binary heap)                               -> [`NavGraph::path`]
//!   - `edgeType`                                              -> [`NavGraph::edge_type`]
//!
//! Intentional deviations from JS (recorded for review):
//!   - **`_climbEdges` omitted** (M1): wall-ink climbing is a nav feature the
//!     easy bot never needs (its routes stay on walk/jump/drop edges), and it
//!     depends on per-wall-column paint sampling. PORT_MAP scopes this module
//!     to "顶面连通航点图的简化实现".
//!   - **`b.noNav` flag**: upstream reads the flag off the level block; the
//!     sim `Block` keeps build-time flags private, so the graph consults the
//!     layout primitives positionally (index == block id, the same invariant
//!     `CollisionWorld::from_layout` relies on). [`NavGraph::without_layout`]
//!     (hand-built test worlds) treats every top as nav-eligible.
//!   - **`Block::roof`**: exposed through a small `is_roof()` accessor rather
//!     than made fully public.
//!   - **open water**: JS `groundHeight` returns `-Infinity` over the sea
//!     because the level has no floor under it; the extracted Tidewater has
//!     no sea-floor brush either, so `ground_height == -inf` is the same test
//!     (the JS comment at bots.js L772 describes the identical rule).
//!   - **no `noClimb` path flag**: with climb edges absent there is nothing
//!     to exclude; [`NavGraph::path`] always plans on the existing edges.

use glam::Vec3;

use crate::collision::CollisionWorld;
use crate::geometry::StageLayout;
use crate::tuning::PlayerTuning;

/// Grid step between samples (JS `this.step = 1.0`).
pub const NAV_STEP: f32 = 1.0;

/// Edge class between two nodes (JS `nb[].type`; `climb` is out of scope —
/// see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    Walk,
    Jump,
    Drop,
}

/// One directed nav edge (JS `nb[]` entry).
#[derive(Debug, Clone, Copy)]
pub struct NavEdge {
    pub to: usize,
    pub cost: f32,
    pub kind: EdgeType,
}

/// One walkable sample (JS `nodes[]` entry).
#[derive(Debug, Clone)]
pub struct NavNode {
    pub id: usize,
    pub pos: Vec3,
    pub ix: usize,
    pub iz: usize,
    /// `-1` open field, `0`/`1` inside a team spawn barrier (JS `zone`).
    pub zone: i8,
    /// 0 dry, 1 near open water (≤ 2.2 m), 2 right at the edge (≤ 1.2 m).
    pub wet: u8,
    /// Grate top with open water under it (a squid would drop through).
    pub over_water: bool,
    pub nb: Vec<NavEdge>,
}

/// The nav graph (JS `NavGraph`).
pub struct NavGraph {
    pub nodes: Vec<NavNode>,
    /// Per XZ cell: node ids standing in it (JS `cells`).
    cells: Vec<Vec<usize>>,
    x0: f32,
    z0: f32,
    nx: usize,
    nz: usize,
    /// Node is two-way reachable from the main spawn region (JS `valid`).
    valid: Vec<bool>,
    /// Node can reach the spawn region (usable as a path *start*; JS
    /// `exitable`).
    exitable: Vec<bool>,
    pub valid_ids: Vec<usize>,
}

impl NavGraph {
    /// JS `new NavGraph(level, physics)` + `_build()` for a real layout: the
    /// spawn pads / barrier come from the stage info (same source as
    /// [`crate::actor::SimWorld::new`]).
    #[must_use]
    pub fn new(world: &CollisionWorld, layout: &StageLayout, t: &PlayerTuning) -> Self {
        let pads = &layout.stage.spawn_pads;
        Self::build(
            world,
            Some(layout),
            [Vec3::from(pads[0]), Vec3::from(pads[1])],
            layout.stage.spawn_barrier,
            t,
        )
    }

    /// [`NavGraph::new`] without a layout (unit-test worlds with hand-built
    /// brushes; every top is nav-eligible, no `noNav` consulted).
    #[must_use]
    pub fn without_layout(
        world: &CollisionWorld,
        spawn_pads: [Vec3; 2],
        spawn_barrier: f32,
        t: &PlayerTuning,
    ) -> Self {
        Self::build(world, None, spawn_pads, spawn_barrier, t)
    }

    fn build(
        world: &CollisionWorld,
        layout: Option<&StageLayout>,
        spawn_pads: [Vec3; 2],
        spawn_barrier: f32,
        t: &PlayerTuning,
    ) -> Self {
        let st = NAV_STEP;
        let b = world.bounds;
        let x0 = b.min_x + st / 2.0;
        let z0 = b.min_z + st / 2.0;
        let nx = ((b.max_x - b.min_x) / st).floor() as usize;
        let nz = ((b.max_z - b.min_z) / st).floor() as usize;
        let mut cells = vec![Vec::new(); nx * nz];
        let mut nodes: Vec<NavNode> = Vec::new();
        let mut ids = Vec::new();
        let mut scratch = Vec::new();

        // ---- samples (JS `_build` first pass): every solid, upward-facing
        // top inside the bounds, deduped per 0.15 m, headroom-checked.
        for iz in 0..nz {
            for ix in 0..nx {
                let x = x0 + ix as f32 * st;
                let z = z0 + iz as f32 * st;
                world.query_blocks(x - 0.01, z - 0.01, x + 0.01, z + 0.01, &mut ids);
                let mut heights: Vec<f32> = Vec::new();
                for id in ids.iter().copied() {
                    let blk = &world.blocks[id as usize];
                    let n = blk.axes[1];
                    if !blk.solid || n.y < 0.6 || blk.is_roof() || blk.rail {
                        continue;
                    }
                    if let Some(l) = layout
                        && l.primitives[id as usize].common().no_nav
                    {
                        continue;
                    }
                    let top = blk.center + n * blk.half.y;
                    let y = top.y - (n.x * (x - top.x) + n.z * (z - top.z)) / n.y;
                    if !world.point_in_block(id, Vec3::new(x, y - 0.02, z), 0.001) {
                        continue;
                    }
                    if heights.iter().any(|h| (h - y).abs() < 0.15) {
                        continue;
                    }
                    heights.push(y);
                }
                for y in heights {
                    if !clear(world, x, y, z, t, &mut scratch) {
                        continue;
                    }
                    let mut zone: i8 = -1;
                    for (ti, pad) in spawn_pads.iter().enumerate() {
                        if (x - pad.x).hypot(z - pad.z) < spawn_barrier + 0.6 && y > pad.y - 1.0 {
                            zone = ti as i8;
                        }
                    }
                    let id = nodes.len();
                    nodes.push(NavNode {
                        id,
                        pos: Vec3::new(x, y, z),
                        ix,
                        iz,
                        zone,
                        wet: 0,
                        over_water: false,
                        nb: Vec::new(),
                    });
                    cells[iz * nx + ix].push(id);
                }
            }
        }

        // ---- water proximity (JS `_build` second pass).
        for n in &mut nodes {
            let (x, y, z) = (n.pos.x, n.pos.y, n.pos.z);
            for (rad, level) in [(1.2f32, 2u8), (2.2, 1)] {
                if n.wet != 0 {
                    break;
                }
                for k in 0..8 {
                    let a = (k as f32 / 8.0) * std::f32::consts::TAU;
                    if world.ground_height(x + a.cos() * rad, z + a.sin() * rad, y + 0.6, false)
                        == f32::NEG_INFINITY
                    {
                        n.wet = level;
                        break;
                    }
                }
            }
            n.over_water = world.ground_height(x, z, y + 0.3, true) == f32::NEG_INFINITY;
        }

        // ---- edges (JS `_build` third pass).
        for cur in 0..nodes.len() {
            let (ix, iz, ny) = {
                let n = &nodes[cur];
                (n.ix, n.iz, n.pos.y)
            };
            let a = nodes[cur].pos;
            let mut new_edges: Vec<NavEdge> = Vec::new();
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let jx = ix as i32 + dx;
                    let jz = iz as i32 + dz;
                    if jx < 0 || jz < 0 || jx as usize >= nx || jz as usize >= nz {
                        continue;
                    }
                    let cell = &cells[jz as usize * nx + jx as usize];
                    for &mid in cell {
                        let m = nodes[mid].pos;
                        let dy = m.y - ny;
                        let flat = ((dx * dx + dz * dz) as f32).sqrt() * st;
                        let diag = dx != 0 && dz != 0;
                        if dy.abs() <= 0.5 {
                            if diag
                                && (!has(&nodes, &cells, nx, nz, (ix as i32 + dx) as usize, iz, ny)
                                    || !has(
                                        &nodes,
                                        &cells,
                                        nx,
                                        nz,
                                        ix,
                                        (iz as i32 + dz) as usize,
                                        ny,
                                    ))
                            {
                                continue;
                            }
                            if blocked(world, a, m, a.y.max(m.y)) {
                                continue;
                            }
                            new_edges.push(NavEdge {
                                to: mid,
                                cost: flat,
                                kind: EdgeType::Walk,
                            });
                        } else if dy > 0.5 && dy <= 1.25 && !diag {
                            if blocked(world, a, m, m.y) {
                                continue;
                            }
                            new_edges.push(NavEdge {
                                to: mid,
                                cost: flat + 2.5,
                                kind: EdgeType::Jump,
                            });
                        } else if (-3.4..-0.5).contains(&dy) && !diag {
                            if !open_above(world, m, ny, &mut scratch) {
                                continue;
                            }
                            if blocked(world, a, m, ny) {
                                continue;
                            }
                            new_edges.push(NavEdge {
                                to: mid,
                                cost: flat + 0.8,
                                kind: EdgeType::Drop,
                            });
                        }
                    }
                }
            }
            nodes[cur].nb = new_edges;
        }

        let mut g = NavGraph {
            nodes,
            cells,
            x0,
            z0,
            nx,
            nz,
            valid: Vec::new(),
            exitable: Vec::new(),
            valid_ids: Vec::new(),
        };
        g.prune(spawn_pads[0]);
        g
    }

    /// JS `_prune`: keep the nodes two-way reachable from the team-0 spawn
    /// (never send a bot into a pit it can't leave); fall back to the whole
    /// main undirected component when the two-way set is implausibly small.
    fn prune(&mut self, pad: Vec3) {
        let n = self.nodes.len();
        let mut radj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for node in &self.nodes {
            for e in &node.nb {
                radj[e.to].push(node.id);
            }
        }
        let walk = |start: usize, next: &dyn Fn(usize) -> Vec<usize>| -> Vec<bool> {
            let mut seen = vec![false; n];
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(k) = stack.pop() {
                for j in next(k) {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            seen
        };
        // largest undirected component (JS seed = nearest node of it to the
        // team-0 pad, never a lone prop top)
        let und = |k: usize| -> Vec<usize> {
            let mut v: Vec<usize> = self.nodes[k].nb.iter().map(|e| e.to).collect();
            v.extend(radj[k].iter().copied());
            v
        };
        let mut comp = vec![-1i32; n];
        let mut best_c = -1i32;
        let mut best_size = 0usize;
        let mut c = 0i32;
        for i in 0..n {
            if comp[i] >= 0 {
                continue;
            }
            let seen = walk(i, &und);
            let mut size = 0;
            for (k, s) in seen.iter().enumerate() {
                if *s {
                    comp[k] = c;
                    size += 1;
                }
            }
            if size > best_size {
                best_size = size;
                best_c = c;
            }
            c += 1;
        }
        let mut seed = usize::MAX;
        let mut sd = f32::INFINITY;
        for node in &self.nodes {
            if comp[node.id] != best_c {
                continue;
            }
            let dy = (node.pos.y - pad.y) * 3.0;
            let d = (node.pos.x - pad.x) * (node.pos.x - pad.x)
                + (node.pos.z - pad.z) * (node.pos.z - pad.z)
                + dy * dy;
            if d < sd {
                sd = d;
                seed = node.id;
            }
        }
        self.valid = vec![false; n];
        self.exitable = vec![false; n];
        if seed == usize::MAX {
            self.valid_ids = Vec::new();
            return;
        }
        let fwd = walk(seed, &|k| self.nodes[k].nb.iter().map(|e| e.to).collect());
        let back = walk(seed, &|k| radj[k].clone());
        let mut count = 0usize;
        for i in 0..n {
            self.valid[i] = fwd[i] && back[i];
            self.exitable[i] = back[i];
            if self.valid[i] {
                count += 1;
            }
        }
        // JS L177 `count < bestSize * 0.5` (float compare); integer-exact form.
        if best_size > 0 && count * 2 < best_size {
            for (i, c) in comp.iter().enumerate() {
                self.valid[i] = *c == best_c;
                self.exitable[i] = self.valid[i];
            }
        }
        self.valid_ids = (0..n).filter(|&i| self.valid[i]).collect();
    }

    /// JS `nearest(pos, maxUp, start)`: ring-expanding cell search; `start`
    /// also accepts nodes that are only a way *out* (path starts).
    #[must_use]
    pub fn nearest(&self, pos: Vec3, max_up: f32, start: bool) -> Option<usize> {
        // JS `Math.round` = floor(x + 0.5) (ties toward +∞), unlike Rust
        // `f32::round` (ties away from zero).
        let ix = ((pos.x - self.x0) / NAV_STEP + 0.5).floor() as i32;
        let iz = ((pos.z - self.z0) / NAV_STEP + 0.5).floor() as i32;
        let mut best: Option<usize> = None;
        let mut bd = f32::INFINITY;
        for r in 0..=3i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let jx = ix + dx;
                    let jz = iz + dz;
                    if jx < 0 || jz < 0 || jx as usize >= self.nx || jz as usize >= self.nz {
                        continue;
                    }
                    for &id in &self.cells[jz as usize * self.nx + jx as usize] {
                        if !self.valid[id] && !(start && self.exitable[id]) {
                            continue;
                        }
                        let n = &self.nodes[id];
                        if n.pos.y > pos.y + max_up {
                            continue;
                        }
                        let dy = (n.pos.y - pos.y) * 2.5;
                        let d = (n.pos.x - pos.x) * (n.pos.x - pos.x)
                            + (n.pos.z - pos.z) * (n.pos.z - pos.z)
                            + dy * dy;
                        if d < bd {
                            bd = d;
                            best = Some(id);
                        }
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }
        best
    }

    /// JS `path(a, b, team)`: A* with the binary heap; blocks the enemy spawn
    /// zone, pays a wet-edge surcharge. Returns node ids incl. both ends, or
    /// `None` when unreachable (JS `maxIter = 6000` default).
    #[must_use]
    pub fn path(&self, a: usize, b: usize, team: usize) -> Option<Vec<usize>> {
        self.path_iter(a, b, team, 6000)
    }

    /// [`NavGraph::path`] with an explicit iteration budget (tests).
    #[must_use]
    pub fn path_iter(
        &self,
        a: usize,
        b: usize,
        team: usize,
        max_iter: usize,
    ) -> Option<Vec<usize>> {
        let n = self.nodes.len();
        if a >= n || b >= n {
            return None;
        }
        let goal = self.nodes[b].pos;
        let h = |p: Vec3| (p.x - goal.x).hypot(p.z - goal.z) + (p.y - goal.y).abs() * 0.5;
        let mut g = vec![0.0f32; n];
        let mut from = vec![usize::MAX; n];
        let mut seen = vec![false; n];
        let mut closed = vec![false; n];
        let mut heap = Heap::new();
        g[a] = 0.0;
        seen[a] = true;
        heap.push(a, h(self.nodes[a].pos));
        let mut it = 0usize;
        while !heap.is_empty() && it < max_iter {
            it += 1;
            let cur = heap.pop();
            if cur == b {
                break;
            }
            if closed[cur] {
                continue;
            }
            closed[cur] = true;
            for e in &self.nodes[cur].nb {
                let m = &self.nodes[e.to];
                if m.zone >= 0 && (m.zone as usize) != team {
                    continue;
                }
                let surcharge = match m.wet {
                    2 => 2.0,
                    1 => 0.5,
                    _ => 0.0,
                };
                let ng = g[cur] + e.cost + surcharge;
                if !seen[e.to] || ng < g[e.to] {
                    seen[e.to] = true;
                    g[e.to] = ng;
                    from[e.to] = cur;
                    heap.push(e.to, ng + h(m.pos));
                }
            }
        }
        if !seen[b] {
            return None;
        }
        let mut out = Vec::new();
        let mut k = b;
        loop {
            out.push(k);
            if k == a || out.len() > 4000 {
                break;
            }
            k = from[k];
        }
        out.reverse();
        Some(out)
    }

    /// JS `edgeType(a, b)` (defaults to `walk` when no edge exists).
    #[must_use]
    pub fn edge_type(&self, a: usize, b: usize) -> EdgeType {
        self.nodes[a]
            .nb
            .iter()
            .find(|e| e.to == b)
            .map_or(EdgeType::Walk, |e| e.kind)
    }

    /// JS `valid[id]`.
    #[must_use]
    pub fn is_valid(&self, id: usize) -> bool {
        self.valid[id]
    }

    /// JS `exitable[id]`.
    #[must_use]
    pub fn is_exitable(&self, id: usize) -> bool {
        self.exitable[id]
    }
}

/// JS nav `_clear`: headroom column + body ring above the step-up height.
fn clear(
    world: &CollisionWorld,
    x: f32,
    y: f32,
    z: f32,
    t: &PlayerTuning,
    scratch: &mut Vec<u32>,
) -> bool {
    let r = t.radius + 0.08;
    for h in [0.15f32, 0.4, 0.85, 1.4] {
        if world.point_inside(Vec3::new(x, y + h, z), 0.0, -1, scratch) {
            return false;
        }
        if h < t.step_up {
            continue;
        }
        for k in 0..8 {
            let a = (k as f32 / 8.0) * std::f32::consts::TAU;
            if world.point_inside(
                Vec3::new(x + a.cos() * r, y + h, z + a.sin() * r),
                0.0,
                -1,
                scratch,
            ) {
                return false;
            }
        }
    }
    true
}

/// JS nav `_blocked`: a railing / thin wall between two cell centres at
/// waist height (kids can't pass grates or rails — `skipGrates=false`).
fn blocked(world: &CollisionWorld, a: Vec3, b: Vec3, y: f32) -> bool {
    let from = Vec3::new(a.x, y + 0.6, a.z);
    let d = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
    let len = d.length();
    if len < 1e-4 {
        return false;
    }
    world.raycast(from, d / len, len, false).hit
}

/// JS nav `_openAbove`: nothing solid hangs over the landing between the
/// drop target and the ledge it drops from.
fn open_above(world: &CollisionWorld, m: Vec3, from_y: f32, scratch: &mut Vec<u32>) -> bool {
    let mut y = m.y + 1.0;
    while y < from_y - 0.05 {
        if world.point_inside(Vec3::new(m.x, y, m.z), 0.0, -1, scratch) {
            return false;
        }
        y += 0.3;
    }
    true
}

/// JS nav `_has`: a node near height `y` exists in cell (ix, iz).
fn has(
    nodes: &[NavNode],
    cells: &[Vec<usize>],
    nx: usize,
    nz: usize,
    ix: usize,
    iz: usize,
    y: f32,
) -> bool {
    if ix >= nx || iz >= nz {
        return false;
    }
    cells[iz * nx + ix]
        .iter()
        .any(|&id| (nodes[id].pos.y - y).abs() <= 0.5)
}

/// Binary min-heap over node ids (JS nav `Heap`).
struct Heap {
    ids: Vec<usize>,
    pr: Vec<f32>,
}

impl Heap {
    fn new() -> Self {
        Self {
            ids: Vec::new(),
            pr: Vec::new(),
        }
    }
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    fn push(&mut self, id: usize, p: f32) {
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
    fn pop(&mut self) -> usize {
        let top = self.ids[0];
        let lid = self.ids.pop().unwrap();
        let lp = self.pr.pop().unwrap();
        if !self.ids.is_empty() {
            let mut i = 0usize;
            let n = self.ids.len();
            loop {
                let l = i * 2 + 1;
                let r = l + 1;
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
mod tests;
