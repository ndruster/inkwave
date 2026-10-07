//! Ink coverage grid and turf scoring (sim layer).
//!
//! Faithful port of the **CPU side only** of `src/world/paint.js`: the coarse
//! 0.25 m per-face grid that answers gameplay queries ("is this spot my ink?")
//! and tracks turf coverage for scoring. The GPU half (atlas render target,
//! growing splats, ripples, wall drips, flood, drying) is pure presentation and
//! lives in the renderer layer, not here.
//!
//! Keep in lockstep with upstream on sync:
//!   - `PaintSystem._initGrid`  -> [`PaintGrid::new`]
//!   - `PaintSystem.splat`      -> [`PaintGrid::splat`] (net/ripple/growing branches dropped)
//!   - `PaintSystem._cpuSplat`  -> [`PaintGrid::cpu_splat`]
//!   - `PaintSystem.sample` / `sampleWorld` / `coverage` / `regionStats`
//!     -> [`PaintGrid::sample`] / [`PaintGrid::sample_world`] /
//!     [`PaintGrid::coverage`] / [`PaintGrid::region_stats`]
//!   - `blobWobble` / `WOB_MAX` / `BAND_*` / `REACH` / `DRIP_REACH` -> same constants.
//!
//! Intentional deviations from JS (reviewed):
//!   - All maths run in `f32` where JS uses f64 doubles. The blob edge test
//!     keeps a 0.97 threshold margin (JS `_cpuSplat`), which absorbs the ~1e-7
//!     relative error of `blob_wobble`/`powi(28)`; no cell flips ownership
//!     because of it.
//!   - [`PaintGrid::coverage`] returns `[0.0, 0.0]` on a turf-less stage where
//!     JS divides `0/0` and yields NaN.
//!   - `flood()` (Zone Control region ink) and `clear()` (round reset) are not
//!     ported yet: `flood` belongs to the weapon/specials task and `clear` can
//!     be rebuilt by constructing a fresh `PaintGrid`.

use glam::{Vec3, vec3};

use crate::actor::InkQuery;
use crate::collision::{CollisionWorld, Face};

/// CPU gameplay-grid cell size in metres (JS `PaintSystem` ctor default `cell`).
pub const CELL: f32 = 0.25;
/// Upper bound of [`blob_wobble`] — the reach of the CPU cell loop.
pub const WOB_MAX: f32 = 1.5;
/// Roller band segment (face space): half length / half width / corner rounding, × radius.
pub(crate) const BAND_L: f32 = 0.55;
pub(crate) const BAND_W: f32 = 0.62;
pub(crate) const BAND_R: f32 = 0.1;
/// Quad half-extent in footprint radii per kind (JS `REACH`, indexed by kind).
pub(crate) const REACH: [f32; 8] = [2.45, 2.1, 2.7, 2.75, 2.3, 1.9, 1.25, 1.35];
/// Extra reach below wall splats (drips).
pub(crate) const DRIP_REACH: f32 = 3.9;

/// Splat body kinds; the discriminant order matches JS `K`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplatKind {
    Shot = 0,
    Line = 1,
    Blast = 2,
    Bomb = 3,
    Trail = 4,
    Drop = 5,
    Roll = 6,
    Speck = 7,
}

impl SplatKind {
    #[must_use]
    pub(crate) fn idx(self) -> usize {
        self as usize
    }

    /// Wire name (JS `K` table, paint.js L35) — the net protocol carries
    /// `opts.kind` as a string (`recSplat` `o.kind ?? 0`, replayed via the
    /// same table; netmatch.js L112/L470).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Shot => "shot",
            Self::Line => "line",
            Self::Blast => "blast",
            Self::Bomb => "bomb",
            Self::Trail => "trail",
            Self::Drop => "drop",
            Self::Roll => "roll",
            Self::Speck => "speck",
        }
    }
}

/// Main-blob outline: organic lobes + two narrow "fingers" thrown out by the
/// impact (JS `blobWobble`; the GPU shader evaluates the identical function).
// `6.2831` is the upstream literal (not an approximation of TAU to be "fixed");
// changing it would desync the blob edge from the GPU shader.
#[allow(clippy::approx_constant)]
#[must_use]
pub fn blob_wobble(ang: f32, seed: f32) -> f32 {
    1.0 + 0.12 * (3.0 * ang + seed * 6.2831).sin()
        + 0.08 * (5.0 * ang + seed * 17.0).sin()
        + 0.05 * (7.0 * ang + seed * 41.0).sin()
        + 0.03 * (11.0 * ang + seed * 73.0).sin()
        + 0.018 * (17.0 * ang + seed * 29.0).sin()
        + 0.17 * (ang - seed * 37.7).cos().max(0.0).powi(28)
        + 0.12 * (ang - seed * 53.3 - 2.1).cos().max(0.0).powi(36)
}

/// Per-face grid bookkeeping (JS `f.nu/f.nv/f.cu/f.cv/f.grid`).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct FaceGrid {
    nu: u32,
    nv: u32,
    cu: f32,
    cv: f32,
    off: usize,
}

/// Options for [`PaintGrid::splat`] (JS `opts`, minus the GPU/net-only fields).
#[derive(Debug, Clone, Copy, Default)]
pub struct SplatOpts {
    /// Deterministic blob shape seed (JS `opts.seed`; upstream defaults to
    /// `Math.random()`, which the sim forbids — always pass one).
    pub seed: f32,
    /// Roll/smear direction in world space (JS `opts.stretch`).
    pub stretch: Option<Vec3>,
    /// Smear amount (JS `opts.stretchAmt`, default 1 when `stretch` is set).
    pub stretch_amt: Option<f32>,
    /// Force a kind instead of inferring it (JS `opts.kind`).
    pub kind: Option<SplatKind>,
}

/// Per-face rasterisation arguments (JS `_cpuSplat` parameter list).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SplatArgs {
    pub(crate) lu: f32,
    pub(crate) lv: f32,
    pub(crate) r: f32,
    pub(crate) team: usize,
    pub(crate) seed: f32,
    pub(crate) sdu: f32,
    pub(crate) sdv: f32,
    pub(crate) sa: f32,
    pub(crate) kind: SplatKind,
}

/// Fractions of live turf cells near a point, relative to `team` (JS `regionStats` out).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RegionStats {
    pub own: f32,
    pub enemy: f32,
    pub empty: f32,
    pub n: u32,
}

/// One recorded splat, replayed by the render layer onto
/// [`InkAtlas`](crate::ink_atlas::InkAtlas). The sim owns the gameplay grid;
/// the presentation bitmap is a separate view fed from this stream so both
/// stay in lock-step without `PaintGrid` holding 16 MB of pixels (it is
/// serde-serialised for snapshots).
#[derive(Debug, Clone, Copy)]
pub struct InkSplat {
    pub center: Vec3,
    pub radius: f32,
    pub team: usize,
    pub opts: SplatOpts,
}

/// Coarse per-face ink grid: 0 none, 1 = team0 (Alpha), 2 = team1 (Bravo).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PaintGrid {
    /// Indexed by face id; `None` for non-paintable faces (JS `!f.atlas`).
    faces: Vec<Option<FaceGrid>>,
    grid: Vec<u8>,
    /// Cells buried inside other geometry (JS `dead`).
    dead: Vec<bool>,
    /// Live (non-dead) turf cells; denominator of [`PaintGrid::coverage`].
    pub turf_total: usize,
    /// Physical area of live turf cells, m².
    pub turf_area: f32,
    counts: [i64; 2],
    /// Bumps whenever the grid changes (minimap polling, JS `version`).
    pub version: u32,
    /// Block-id scratch reused across `splat` calls (JS `_qb`).
    splat_ids: Vec<u32>,
    /// Splats recorded since the render layer last drained them (transient;
    /// excluded from snapshots). Headless callers that never drain grow this
    /// unboundedly - call [`PaintGrid::drain_ink`](Self::drain_ink) regularly.
    #[serde(skip)]
    ink_stream: Vec<InkSplat>,
}

impl PaintGrid {
    /// Allocate one grid per paintable face and classify turf/dead cells
    /// (JS `PaintSystem` ctor + `_initGrid`).
    #[must_use]
    pub fn new(world: &CollisionWorld) -> Self {
        let cell = CELL;
        let mut faces: Vec<Option<FaceGrid>> = vec![None; world.faces.len()];
        let mut total = 0usize;
        for f in &world.faces {
            if !f.paintable {
                continue;
            }
            let nu = (f.su / cell).round().max(1.0) as u32;
            let nv = (f.sv / cell).round().max(1.0) as u32;
            faces[f.id as usize] = Some(FaceGrid {
                nu,
                nv,
                cu: f.su / nu as f32,
                cv: f.sv / nv as f32,
                off: total,
            });
            total += nu as usize * nv as usize;
        }
        let grid = vec![0u8; total];
        let mut dead = vec![false; total];
        let mut turf_total = 0usize;
        let mut turf_area = 0.0f32;
        let mut scratch = Vec::new();
        for f in &world.faces {
            let Some(fg) = faces[f.id as usize] else {
                continue;
            };
            for j in 0..fg.nv {
                for i in 0..fg.nu {
                    // cell centre lifted 6 cm off the face plane (JS `_initGrid`)
                    let p = f.origin
                        + f.u * ((i as f32 + 0.5) * fg.cu)
                        + f.v * ((j as f32 + 0.5) * fg.cv)
                        + f.n * 0.06;
                    let k = fg.off + (j as usize) * (fg.nu as usize) + i as usize;
                    if world.point_inside(p, 0.0, f.block as i32, &mut scratch) {
                        dead[k] = true;
                    } else if f.turf {
                        turf_total += 1;
                        turf_area += fg.cu * fg.cv;
                    }
                }
            }
        }
        Self {
            faces,
            grid,
            dead,
            turf_total,
            turf_area,
            counts: [0, 0],
            version: 0,
            splat_ids: Vec::new(),
            ink_stream: Vec::new(),
        }
    }

    // ------------------------------------------------------------- splat

    /// Paint a splat at `center` (JS `splat`, CPU half). `team` is 0 or 1.
    /// Returns the area (m²) newly claimed by `team`.
    ///
    /// Drops the upstream net-recording, growing-quad, ripple and speck
    /// branches: those are GPU presentation / online plumbing.
    pub fn splat(
        &mut self,
        world: &CollisionWorld,
        center: Vec3,
        radius: f32,
        team: usize,
        opts: &SplatOpts,
    ) -> f32 {
        self.ink_stream.push(InkSplat {
            center,
            radius,
            team,
            opts: *opts,
        });
        let seed = opts.seed;
        let st = opts.stretch;
        let mut s_amt = if st.is_some() {
            opts.stretch_amt.unwrap_or(1.0)
        } else {
            0.0
        };
        let kind = infer_kind(st, s_amt, radius, opts.kind);
        if kind == SplatKind::Roll {
            s_amt = 0.0; // the direction orients the band; no smear
        }
        let reach_k = REACH[kind.idx()];
        let reach = radius * (reach_k + 1.4 * s_amt + 0.3).max(3.2);

        // Reuse the block-id scratch across splats (JS `this._qb`).
        let mut ids = std::mem::take(&mut self.splat_ids);
        ids.clear();
        world.query_blocks(
            center.x - reach,
            center.z - reach,
            center.x + reach,
            center.z + reach,
            &mut ids,
        );
        let mut claimed = 0.0f32;
        for &bid in &ids {
            let b = &world.blocks[bid as usize];
            // quick reject by AABB distance (JS `splat`)
            if center.x < b.aabb_min.x - reach
                || center.x > b.aabb_max.x + reach
                || center.y < b.aabb_min.y - reach
                || center.y > b.aabb_max.y + reach
                || center.z < b.aabb_min.z - reach
                || center.z > b.aabb_max.z + reach
            {
                continue;
            }
            for fi in 0..6 {
                let fid = b.faces[fi];
                if fid < 0 {
                    continue;
                }
                let fu = fid as usize;
                if self.faces[fu].is_none() {
                    continue; // not paintable (JS `!f.atlas`)
                }
                let f = &world.faces[fu];
                let rel = center - f.origin;
                let dn = rel.dot(f.n);
                if dn > radius || dn < -0.12 {
                    continue;
                }
                let lu = rel.dot(f.u);
                let lv = rel.dot(f.v);
                let rr = (radius * radius - dn * dn).max(0.0).sqrt();
                let ext = rr * (reach_k + 1.4 * s_amt);
                let drip = if f.wall { rr * DRIP_REACH } else { 0.0 };
                if lu < -ext || lu > f.su + ext || lv < -ext - drip || lv > f.sv + ext {
                    continue;
                }
                // stretch / band direction projected into face space
                let (mut sdu, mut sdv, mut sa) = (0.0f32, 0.0f32, 0.0f32);
                if let Some(stv) = st {
                    sdu = stv.dot(f.u);
                    sdv = stv.dot(f.v);
                    let l = sdu.hypot(sdv);
                    if l > 0.2 {
                        sdu /= l;
                        sdv /= l;
                        sa = s_amt * l;
                    } else if kind == SplatKind::Roll {
                        sdu = 1.0;
                        sdv = 0.0;
                    } else {
                        sdu = 0.0;
                        sdv = 0.0;
                    }
                }
                claimed += self.cpu_splat(
                    f,
                    fu,
                    SplatArgs {
                        lu,
                        lv,
                        r: rr,
                        team,
                        seed,
                        sdu,
                        sdv,
                        sa,
                        kind,
                    },
                );
            }
        }
        self.splat_ids = ids;
        claimed
    }

    /// Take the splats recorded since the last call; the render layer replays
    /// them onto its [`InkAtlas`](crate::ink_atlas::InkAtlas) bitmap.
    pub fn drain_ink(&mut self) -> Vec<InkSplat> {
        std::mem::take(&mut self.ink_stream)
    }

    /// Rasterise one splat onto a single face (JS `_cpuSplat`).
    pub(crate) fn cpu_splat(&mut self, f: &Face, fi: usize, a: SplatArgs) -> f32 {
        let SplatArgs {
            lu,
            lv,
            r,
            team,
            seed,
            sdu,
            sdv,
            sa,
            kind,
        } = a;
        if r <= 0.02 {
            return 0.0;
        }
        let fg = self.faces[fi].expect("paintable face has a grid");
        let val = (team + 1) as u8;
        let roll = kind == SplatKind::Roll;
        let ext = if roll {
            r * ((BAND_L * BAND_L + BAND_W * BAND_W).sqrt() + BAND_R + 0.05)
        } else {
            r * (1.0 + sa) * WOB_MAX
        };
        let i0 = (((lu - ext) / fg.cu).floor() as i64).max(0);
        let i1 = (((lu + ext) / fg.cu).floor() as i64).min(fg.nu as i64 - 1);
        let j0 = (((lv - ext) / fg.cv).floor() as i64).max(0);
        let j1 = (((lv + ext) / fg.cv).floor() as i64).min(fg.nv as i64 - 1);
        if i1 < i0 || j1 < j0 {
            return 0.0;
        }
        let mut claimed = 0.0f32;
        let cell_a = fg.cu * fg.cv;
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (mut px, mut py) =
                    ((i as f32 + 0.5) * fg.cu - lu, (j as f32 + 0.5) * fg.cv - lv);
                if roll {
                    // rounded-rectangle SDF of the band segment
                    let qa = (px * sdu + py * sdv).abs() - r * BAND_L;
                    let qb = (-px * sdv + py * sdu).abs() - r * BAND_W;
                    let sd = qa.max(0.0).hypot(qb.max(0.0)) + qa.max(qb).min(0.0) - r * BAND_R;
                    if sd > -0.03 * r {
                        continue;
                    }
                } else {
                    if sa > 0.0 {
                        // undo the smear, then test the round blob
                        let a = px * sdu + py * sdv;
                        let qx = px - a * sdu;
                        let qy = py - a * sdv;
                        let s = if a > 0.0 { 1.0 + sa } else { 1.0 + 0.25 * sa };
                        px = qx + sdu * (a / s);
                        py = qy + sdv * (a / s);
                    }
                    let d = px.hypot(py);
                    if d > r * WOB_MAX {
                        continue;
                    }
                    if d / (r * blob_wobble(py.atan2(px), seed)) > 0.97 {
                        continue;
                    }
                }
                let k = fg.off + (j as usize) * (fg.nu as usize) + i as usize;
                let prev = self.grid[k];
                if prev == val {
                    continue;
                }
                self.grid[k] = val;
                claimed += cell_a;
                if f.turf && !self.dead[k] {
                    if prev != 0 {
                        self.counts[(prev - 1) as usize] -= 1;
                    }
                    self.counts[team] += 1;
                }
            }
        }
        if claimed > 0.0 {
            self.version += 1;
        }
        claimed
    }

    // ----------------------------------------------------------- queries

    /// Team at face-local (u, v): 0 none, 1 = team0, 2 = team1 (JS `sample`).
    #[must_use]
    pub fn sample(&self, face: i32, u: f32, v: f32) -> u8 {
        if face < 0 {
            return 0;
        }
        let Some(Some(fg)) = self.faces.get(face as usize) else {
            return 0;
        };
        let i = ((u / fg.cu).floor() as i64).clamp(0, fg.nu as i64 - 1) as usize;
        let j = ((v / fg.cv).floor() as i64).clamp(0, fg.nv as i64 - 1) as usize;
        self.grid[fg.off + j * fg.nu as usize + i]
    }

    /// Team at a world point lying on face `face` (JS `sampleWorld`).
    #[must_use]
    pub fn sample_world(&self, world: &CollisionWorld, face: i32, p: Vec3) -> u8 {
        if face < 0 {
            return 0;
        }
        let f = &world.faces[face as usize];
        let rel = p - f.origin;
        self.sample(face, rel.dot(f.u), rel.dot(f.v))
    }

    /// Turf coverage fractions [team0, team1] of all live turf cells (JS `coverage`).
    ///
    /// Deviation: JS returns `[NaN, NaN]` on a stage with zero live turf cells
    /// (`0/0`); here that case yields `[0.0, 0.0]`.
    #[must_use]
    pub fn coverage(&self) -> [f32; 2] {
        if self.turf_total == 0 {
            return [0.0, 0.0];
        }
        let t = self.turf_total as f32;
        [self.counts[0] as f32 / t, self.counts[1] as f32 / t]
    }

    /// Fractions of live turf cells within `radius` of (x, z) near height `y`,
    /// relative to `team` (JS `regionStats`, step-2 sampling included).
    #[must_use]
    pub fn region_stats(
        &self,
        world: &CollisionWorld,
        x: f32,
        y: f32,
        z: f32,
        radius: f32,
        team: usize,
    ) -> RegionStats {
        let mut out = RegionStats::default();
        let mut ids = Vec::new();
        world.query_blocks(x - radius, z - radius, x + radius, z + radius, &mut ids);
        let own = (team + 1) as u8;
        for bid in ids {
            let b = &world.blocks[bid as usize];
            for fi in 0..6 {
                let fid = b.faces[fi];
                if fid < 0 {
                    continue;
                }
                let fu = fid as usize;
                let f = &world.faces[fu];
                let Some(fg) = self.faces[fu] else { continue };
                if !f.turf {
                    continue;
                }
                if (f.origin.y - y).abs() > 2.5 {
                    continue;
                }
                let rel = vec3(x, y, z) - f.origin;
                let lu = rel.dot(f.u);
                let lv = rel.dot(f.v);
                let i0 = (((lu - radius) / fg.cu).floor() as i64).max(0);
                let i1 = (((lu + radius) / fg.cu).floor() as i64).min(fg.nu as i64 - 1);
                let j0 = (((lv - radius) / fg.cv).floor() as i64).max(0);
                let j1 = (((lv + radius) / fg.cv).floor() as i64).min(fg.nv as i64 - 1);
                let mut j = j0;
                while j <= j1 {
                    let mut i = i0;
                    while i <= i1 {
                        let du = (i as f32 + 0.5) * fg.cu - lu;
                        let dv = (j as f32 + 0.5) * fg.cv - lv;
                        if du * du + dv * dv <= radius * radius {
                            let k = fg.off + (j as usize) * (fg.nu as usize) + i as usize;
                            if !self.dead[k] {
                                out.n += 1;
                                let g = self.grid[k];
                                if g == own {
                                    out.own += 1.0;
                                } else if g != 0 {
                                    out.enemy += 1.0;
                                } else {
                                    out.empty += 1.0;
                                }
                            }
                        }
                        i += 2;
                    }
                    j += 2;
                }
            }
        }
        if out.n != 0 {
            let n = out.n as f32;
            out.own /= n;
            out.enemy /= n;
            out.empty /= n;
        }
        out
    }
}

impl InkQuery for PaintGrid {
    fn sample(&self, face: i32, u: f32, v: f32) -> u8 {
        PaintGrid::sample(self, face, u, v)
    }
}

/// JS `PaintSystem._kind` (the cosmetic-speck default lives on the GPU path).
#[must_use]
pub(crate) fn infer_kind(
    stretch: Option<Vec3>,
    s_amt: f32,
    radius: f32,
    kind: Option<SplatKind>,
) -> SplatKind {
    if let Some(k) = kind
        && (k != SplatKind::Roll || stretch.is_some())
    {
        return k;
    }
    if stretch.is_some() {
        return if s_amt >= 1.0 {
            SplatKind::Line
        } else {
            SplatKind::Shot
        };
    }
    if radius >= 1.9 {
        SplatKind::Bomb
    } else if radius >= 1.05 {
        SplatKind::Blast
    } else if radius < 0.3 {
        SplatKind::Drop
    } else {
        SplatKind::Trail
    }
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

    fn obox(center: [f32; 3], size: [f32; 3], rot_y: f32) -> Brush {
        Brush::Obox {
            center,
            size,
            rot_y,
            common: flags(),
        }
    }

    const B: Bounds = Bounds {
        min_x: -30.0,
        max_x: 30.0,
        min_z: -30.0,
        max_z: 30.0,
    };

    fn opts(seed: f32) -> SplatOpts {
        SplatOpts {
            seed,
            ..SplatOpts::default()
        }
    }

    /// The top face of block `blk` (normal +Y, turf).
    fn top_face(world: &CollisionWorld, blk: u32) -> &Face {
        world
            .faces
            .iter()
            .find(|f| f.block == blk && f.n.y > 0.9 && f.turf)
            .expect("block has a turf top face")
    }

    // TR-6.1: stamping a known radius on a flat face claims ≈ πr² (< 3%).
    #[test]
    fn tr6_1_circle_area_matches_pi_r_squared() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        assert!(pg.turf_total > 0, "flat stage must expose turf cells");

        let r = 2.0f32;
        let want = std::f32::consts::PI * r * r;
        let got = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 0, &opts(0.37));
        println!("stamp r={r}: claimed {got:.4} m² vs πr² = {:.4}", want);
        assert!(
            (got - want).abs() / want < 0.03,
            "TR-6.1 area error {got} vs {want} exceeds 3%"
        );

        // Same team re-stamping the identical splat claims nothing new.
        let again = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 0, &opts(0.37));
        assert_eq!(again, 0.0, "same-team re-stamp must claim 0");

        // The other team overwriting claims the same footprint again (every
        // cell flips ownership, each flip counted once) and then 0 on repeat.
        let flip = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 1, &opts(0.37));
        println!("enemy overwrite: claimed {flip:.4} m²");
        assert!(
            (flip - want).abs() / want < 0.03,
            "TR-6.1 overwrite area {flip} vs {want}"
        );
        let flip_again = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 1, &opts(0.37));
        assert_eq!(flip_again, 0.0);

        // Same-team overlapping stamp only counts the crescent beyond the
        // existing ink: the centre blob is team1 now, so a team1 stamp at
        // (2,0) must claim strictly less than its own full footprint.
        let part = pg.splat(&w, vec3(2.0, 0.0, 0.0), r, 1, &opts(0.37));
        println!("same-team overlap: claimed {part:.4} m² (full blob would be {want:.4})");
        assert!(
            part > 0.1 * want && part < 0.8 * want,
            "overlap must claim only the incremental crescent, got {part}"
        );
        // Re-stamping the same crescent claims nothing further.
        let part_again = pg.splat(&w, vec3(2.0, 0.0, 0.0), r, 1, &opts(0.37));
        assert_eq!(part_again, 0.0, "idempotent re-stamp must claim 0");
    }

    // TR-6.1: ownership flips are reflected in the per-team turf counts.
    #[test]
    fn tr6_1_counts_track_ownership() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        let r = 2.0f32;
        let a0 = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 0, &opts(0.11));
        let cov = pg.coverage();
        let expect = a0 / pg.turf_area;
        println!(
            "coverage after team0 stamp: {:?} (claimed/turfArea = {expect:.4})",
            cov
        );
        assert!(
            (cov[0] - expect).abs() < 0.02,
            "coverage must track claimed turf"
        );
        assert_eq!(cov[1], 0.0);

        // Enemy overwrite moves the same cells out of team0's count.
        let a1 = pg.splat(&w, vec3(0.0, 0.0, 0.0), r, 1, &opts(0.11));
        let cov = pg.coverage();
        assert!(cov[0] < expect - 0.5 * a1 / pg.turf_area);
        assert!((cov[1] - a1 / pg.turf_area).abs() < 0.02);
    }

    // TR-6.2: ramp faces use their own local (u,v) basis.
    #[test]
    fn tr6_2_ramp_local_coords() {
        let w = CollisionWorld::from_brushes(&[ramp(0.0)], B);
        let mut pg = PaintGrid::new(&w);
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        // surface point at z = 0 -> y = 1
        let c = vec3(0.0, 1.0, 0.0);
        let rel = c - f.origin;
        let (u, v) = (rel.dot(f.u), rel.dot(f.v));
        let got = pg.splat(&w, c, 1.0, 0, &opts(0.5));
        assert!(got > 0.0, "ramp splat must claim cells");
        assert_eq!(pg.sample(fid, u, v), 1, "stamp centre samples as team0");
        assert_eq!(pg.sample_world(&w, fid, c), 1);
        // a point ~2 m up the slope is outside the r=1 blob
        let far = vec3(0.0, 1.5, 2.0);
        assert_eq!(
            pg.sample_world(&w, fid, far),
            0,
            "outside blob stays unpainted"
        );
    }

    // TR-6.2: rotated (obox) faces use their rotated local basis for queries.
    #[test]
    fn tr6_2_rotated_face_local_coords() {
        let w = CollisionWorld::from_brushes(&[obox([0.0, 0.5, 0.0], [6.0, 1.0, 6.0], 30.0)], B);
        let mut pg = PaintGrid::new(&w);
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        let c = vec3(0.0, 1.0, 0.0);
        let got = pg.splat(&w, c, 1.0, 1, &opts(0.7));
        assert!(got > 0.0, "rotated-top splat must claim cells");
        let rel = c - f.origin;
        let (u, v) = (rel.dot(f.u), rel.dot(f.v));
        assert_eq!(pg.sample(fid, u, v), 2);
        // +0.5 m along the *rotated* u axis is still inside; +3 m is not
        let p_near = c + f.u * 0.5;
        assert_eq!(pg.sample_world(&w, fid, p_near), 2, "rotated u axis");
        let p_far = c + f.u * 3.0;
        assert_eq!(pg.sample_world(&w, fid, p_far), 0);
    }

    // TR-6.3: coverage is normalised to live turf, in [0,1], empty stage = 0.
    #[test]
    fn tr6_3_coverage_normalisation() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        assert!(pg.turf_total > 0);
        let empty = pg.coverage();
        assert_eq!(empty, [0.0, 0.0], "fresh stage covers 0");

        pg.splat(&w, vec3(-4.0, 0.0, -4.0), 2.0, 0, &opts(0.2));
        pg.splat(&w, vec3(4.0, 0.0, 4.0), 2.0, 1, &opts(0.9));
        let cov = pg.coverage();
        println!("coverage = {cov:?}, turf_total = {}", pg.turf_total);
        for &c in &cov {
            assert!(
                (0.0..=1.0).contains(&c),
                "coverage fraction out of [0,1]: {c}"
            );
        }
        assert!(cov[0] + cov[1] <= 1.0 + MM, "teams cannot overlap");
        assert!(cov[0] > 0.0 && cov[1] > 0.0);

        // region stats near a team-0 blob see mostly own ink
        let rs = pg.region_stats(&w, -4.0, 0.0, -4.0, 2.0, 0);
        println!("region_stats = {rs:?}");
        assert!(rs.n > 0);
        assert!(rs.own > 0.5, "own blob dominates its region: {rs:?}");
        assert!((rs.own + rs.enemy + rs.empty - 1.0).abs() < 1e-3);
    }

    // Non-paintable faces get no grid and never claim.
    #[test]
    fn non_paintable_never_claims() {
        let mut c = flags();
        c.paint = false;
        let w = CollisionWorld::from_brushes(
            &[Brush::Box {
                min: [-5.0, -1.0, -5.0],
                max: [5.0, 0.0, 5.0],
                common: c,
            }],
            B,
        );
        let mut pg = PaintGrid::new(&w);
        assert!(pg.faces.iter().all(|e| e.is_none()), "no grids allocated");
        assert_eq!(pg.turf_total, 0);
        let got = pg.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 0, &opts(0.3));
        assert_eq!(got, 0.0);
        assert_eq!(pg.coverage(), [0.0, 0.0]);
    }

    // Roller band: `stretch` with kind Roll paints a straight-edged band.
    #[test]
    fn roll_band_stamps_along_direction() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        let got = pg.splat(
            &w,
            vec3(0.0, 0.0, 0.0),
            1.0,
            0,
            &SplatOpts {
                seed: 0.42,
                stretch: Some(vec3(0.0, 0.0, 1.0)),
                stretch_amt: None,
                kind: Some(SplatKind::Roll),
            },
        );
        assert!(got > 0.0);
        // band half-length ≈ r * BAND_L = 0.55 along +z
        let rel = vec3(0.0, 0.0, 0.4) - f.origin;
        assert_eq!(pg.sample(fid, rel.dot(f.u), rel.dot(f.v)), 1, "inside band");
        let rel = vec3(0.0, 0.0, 1.2) - f.origin;
        assert_eq!(
            pg.sample(fid, rel.dot(f.u), rel.dot(f.v)),
            0,
            "past band end"
        );
    }

    // InkQuery wiring for the actor (Task 5 interface stub).
    #[test]
    fn ink_query_trait() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        pg.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 1, &opts(0.5));
        let ink: &dyn InkQuery = &pg;
        let f = top_face(&w, 0);
        let rel = vec3(0.0, 0.0, 0.0) - f.origin;
        assert_eq!(ink.sample(f.id as i32, rel.dot(f.u), rel.dot(f.v)), 2);
        assert_eq!(ink.sample(-1, 0.0, 0.0), 0);
    }

    // blobWobble bounds sanity (CPU reach constant depends on it).
    #[test]
    fn blob_wobble_bounds() {
        // NOTE: WOB_MAX = 1.5 is the upstream empirical constant (JS L48), not a
        // strict mathematical bound — the sum of the sinusoid amplitudes is
        // 1.598. This sampled sweep (4 seeds × 360°) mirrors what the CPU loop
        // relies on; JS shares the same constant.
        for seed in [0.0f32, 0.17, 0.5, 0.91] {
            for k in 0..360 {
                let a = k as f32 * std::f32::consts::PI / 180.0;
                let w = blob_wobble(a, seed);
                assert!(w > 0.0 && w <= WOB_MAX, "wob {w} out of (0, {WOB_MAX}]");
            }
        }
    }

    // Grazing splat: dn in [-0.12, 0) still paints with rr = sqrt(r² - dn²).
    #[test]
    fn grazing_dn_behind_plane_still_paints() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        // centre 5 cm below the plane: dn = -0.05 >= -0.12 -> accepted,
        // rr = sqrt(1 - 0.0025) shrinks the footprint slightly.
        let got = pg.splat(&w, vec3(0.0, -0.05, 0.0), 1.0, 0, &opts(0.3));
        println!("grazing dn=-0.05: claimed {got:.4} m²");
        assert!(got > 0.0, "dn in [-0.12, 0) must still paint");
        assert!(got < std::f32::consts::PI, "rr shrink must reduce area");
        let rel = vec3(0.0, 0.0, 0.0) - f.origin;
        assert_eq!(pg.sample(fid, rel.dot(f.u), rel.dot(f.v)), 1);
        // 20 cm below the plane: dn = -0.2 < -0.12 -> rejected (JS L498).
        let miss = pg.splat(&w, vec3(0.0, -0.2, 0.0), 1.0, 1, &opts(0.3));
        assert_eq!(miss, 0.0, "dn < -0.12 must be rejected");
    }

    // Wall splat: paints the wall face (non-turf: claimed > 0 but no coverage).
    // NOTE on DRIP_REACH (JS L502): the `lv < -ext - rr·DRIP_REACH` lower bound
    // only widens the cell loop; the blob edge test (`d > r·WOB_MAX` / 0.97
    // threshold) rejects cells far below the centre first, so the drip bound is
    // unobservable in the CPU grid path — it exists for GPU quad extents only.
    #[test]
    fn wall_face_splat_paints_without_turf() {
        let w = CollisionWorld::from_brushes(
            &[
                bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
                bx([-0.5, 0.0, -4.0], [0.5, 3.0, 4.0]),
            ],
            B,
        );
        let mut pg = PaintGrid::new(&w);
        let wall = w
            .faces
            .iter()
            .find(|f| f.block == 1 && f.wall && f.n.x > 0.9)
            .expect("wall block has a +x wall face");
        let wid = wall.id as i32;
        let got = pg.splat(&w, vec3(0.5, 1.5, 0.0), 1.0, 0, &opts(0.25));
        println!("wall splat: claimed {got:.4} m²");
        assert!(got > 0.0, "wall splat must claim cells");
        let rel = vec3(0.5, 1.5, 0.0) - wall.origin;
        assert_eq!(pg.sample(wid, rel.dot(wall.u), rel.dot(wall.v)), 1);
        // wall faces are not turf: coverage must stay 0 (counts skip non-turf).
        assert_eq!(pg.coverage()[0], 0.0, "wall ink must not count as turf");
    }

    // Non-roll stretch (Line): asymmetric smear along the direction (JS L628-633).
    #[test]
    fn smear_stretch_line_is_asymmetric() {
        let w = CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
        let mut pg = PaintGrid::new(&w);
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        let got = pg.splat(
            &w,
            vec3(0.0, 0.0, 0.0),
            1.0,
            0,
            &SplatOpts {
                seed: 0.3,
                stretch: Some(vec3(0.0, 0.0, 1.0)),
                stretch_amt: Some(2.0),
                kind: None,
            },
        );
        println!("line smear: claimed {got:.4} m²");
        assert!(got > 0.0);
        // forward (s = 1 + sa = 3): world +1.5 m maps to inverse d = 0.5 -> inside.
        let rel = vec3(0.0, 0.0, 1.5) - f.origin;
        assert_eq!(
            pg.sample(fid, rel.dot(f.u), rel.dot(f.v)),
            1,
            "forward smear"
        );
        // backward (s = 1 + 0.25·sa = 1.5): world −2.5 m maps to |d| = 1.667 >
        // 0.97·WOB_MAX·r -> always outside.
        let rel = vec3(0.0, 0.0, -2.5) - f.origin;
        assert_eq!(pg.sample(fid, rel.dot(f.u), rel.dot(f.v)), 0, "rear edge");
    }

    // One splat crossing a floor/wall corner accumulates both faces.
    #[test]
    fn cross_face_splat_accumulates() {
        let w = CollisionWorld::from_brushes(
            &[
                bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
                bx([-0.5, 0.0, -4.0], [0.5, 2.0, 4.0]),
            ],
            B,
        );
        let mut pg = PaintGrid::new(&w);
        // centre on the wall plane at y = 0.5: floor face sees dn = 0.5,
        // wall +x face sees dn = 0 -> both rasterise.
        let got = pg.splat(&w, vec3(0.5, 0.5, 0.0), 1.0, 0, &opts(0.6));
        println!("cross-face splat: claimed {got:.4} m²");
        let wall = w
            .faces
            .iter()
            .find(|f| f.block == 1 && f.wall && f.n.x > 0.9)
            .expect("wall face");
        let floor = top_face(&w, 0);
        let rel = vec3(0.5, 0.5, 0.0) - wall.origin;
        assert_eq!(
            pg.sample(wall.id as i32, rel.dot(wall.u), rel.dot(wall.v)),
            1,
            "wall"
        );
        let rel = vec3(0.5, 0.0, 0.0) - floor.origin;
        assert_eq!(
            pg.sample(floor.id as i32, rel.dot(floor.u), rel.dot(floor.v)),
            1,
            "floor under the wall splat"
        );
        // floor blob (rr = sqrt(1 - 0.25) ≈ 0.866) plus wall blob > either alone.
        assert!(got > std::f32::consts::PI * 0.6);
    }

    // Dead cells (buried under other geometry) write the grid but never count.
    #[test]
    fn dead_cells_write_grid_but_skip_counts() {
        let w = CollisionWorld::from_brushes(
            &[
                bx([-5.0, -1.0, -5.0], [5.0, 0.0, 5.0]),
                bx([-1.0, 0.0, -1.0], [1.0, 0.5, 1.0]),
            ],
            B,
        );
        let pg0 = PaintGrid::new(&w);
        // ground cells under the small box are buried -> dead.
        assert!(pg0.dead.iter().any(|&d| d), "stacked box must bury cells");
        let mut pg = pg0;
        let f = top_face(&w, 0);
        let fid = f.id as i32;
        let got = pg.splat(&w, vec3(0.0, 0.0, 0.0), 1.5, 0, &opts(0.4));
        let live = pg.coverage()[0] * pg.turf_area;
        println!("dead-region splat: claimed {got:.4} m², counted turf {live:.4} m²");
        assert!(got > 0.0);
        // grid written even under the box (JS `_cpuSplat` sets grid[k] regardless
        // of dead; only counts skips it).
        let rel = vec3(0.0, 0.0, 0.0) - f.origin;
        assert_eq!(
            pg.sample(fid, rel.dot(f.u), rel.dot(f.v)),
            1,
            "dead cell still samples"
        );
        // the buried ~4 m² footprint must not have been counted as turf.
        let uncounted = got - live;
        assert!(
            uncounted > 2.0 && uncounted < 5.5,
            "dead area must be excluded from counts, got {uncounted}"
        );
    }
}
