//! Ink atlas: the presentation-side RGBA8 bitmap that mirrors the gameplay
//! [`PaintGrid`](crate::paint::PaintGrid) with per-texel soft coverage.
//!
//! Encoding (identical to the upstream `src/world/paint.js` atlas, RGBA8
//! premultiplied by coverage):
//!   R = team share (0 = team0, 1 = team1, weighted by coverage)
//!   G = wetness (constant 1 in M1 - no drying)
//!   B = per-splat tone `hsh(seed * 1.73)`
//!   A = coverage, ~3-texel soft outline around the same SDF the CPU grid uses
//!
//! Composite per texel: RGB "over" (`new*a + old*(1-a)`), A `max` - newest splat
//! wins the colour, coverage is the union.
//!
//! The atlas is owned by the render layer only: [`InkAtlas::splat`] replays the
//! same splat stream as [`PaintGrid::splat`](crate::paint::PaintGrid::splat) so
//! the bitmap stays in lock-step with the gameplay grid without `PaintGrid`
//! having to hold 16 MB of pixels (it is serde-serialised for snapshots).

use glam::Vec3;

use crate::collision::{CollisionWorld, Face};
use crate::paint::{
    BAND_L, BAND_R, BAND_W, DRIP_REACH, REACH, SplatArgs, SplatKind, SplatOpts, WOB_MAX,
    blob_wobble, infer_kind,
};

/// Atlas edge length in texels (spec cap for Task 11).
pub const ATLAS_SIZE: u32 = 2048;
/// Initial texels-per-metre; reduced by 8 % per retry if the shelf pack fails.
pub const MAX_DENSITY: f32 = 30.0;
/// Guard border (texels) around every face rect so the soft edge never bleeds
/// into a neighbour (JS `pad`).
pub const PAD: u32 = 8;

/// Shelf-packed rect of one paintable face inside the atlas.
#[derive(Debug, Clone, Copy)]
pub struct FaceAtlas {
    /// Texels per metre for this atlas generation.
    pub ppm: f32,
    /// Rect origin in atlas texels (includes the pad border).
    pub x: u32,
    pub y: u32,
    /// Rect size in texels: `ceil(su * ppm) + 2 * PAD` (JS `_tryPack`).
    pub w: u32,
    pub h: u32,
}

impl FaceAtlas {
    /// First ink (non-pad) texel column.
    #[must_use]
    pub fn ink_x(&self) -> u32 {
        self.x + PAD
    }

    /// First ink (non-pad) texel row.
    #[must_use]
    pub fn ink_y(&self) -> u32 {
        self.y + PAD
    }

    /// UV of face-local metre coords `(cu, cv)` (JS `_pushQuad`):
    /// `px = a.x + a.pad + cu * a.ppm`, normalised by the atlas size.
    #[must_use]
    pub fn uv_at(&self, cu: f32, cv: f32, size: u32) -> (f32, f32) {
        let s = size as f32;
        (
            (self.x as f32 + PAD as f32 + cu * self.ppm) / s,
            (self.y as f32 + PAD as f32 + cv * self.ppm) / s,
        )
    }
}

/// GLSL `fract` (always in `[0, 1)`; Rust's `fract` keeps the sign).
#[must_use]
fn hsh(n: f32) -> f32 {
    // `43758.5453123` is the upstream GLSL/JS literal; trimming its precision
    // would desync tones from the JS build.
    #[allow(clippy::excessive_precision)]
    let v = n.sin() * 43758.5453123;
    v - v.floor()
}

/// Final-state body SDF of one splat at face-local offset `(dx, dy)` metres
/// from the splat centre (GPU `PAINT_FS` body branch with `grow = 1`).
#[must_use]
fn body_sdf(kind: SplatKind, dx: f32, dy: f32, a: SplatArgs) -> f32 {
    let SplatArgs {
        r,
        seed,
        sdu,
        sdv,
        sa,
        ..
    } = a;
    if kind == SplatKind::Roll {
        // roller band: straight segment across the drum, edges gently wavy
        let qx = dx * sdu + dy * sdv;
        let qy = -dx * sdv + dy * sdu;
        let wv = r
            * (0.03 * (qx / r * 9.0 + seed * 30.0).sin()
                + 0.018 * (qx / r * 23.0 + seed * 11.0).sin());
        let dqx = qx.abs() - r * BAND_L;
        let dqy = qy.abs() - (r * BAND_W + wv);
        dqx.max(0.0).hypot(dqy.max(0.0)) + dqx.max(dqy).min(0.0) - r * BAND_R
    } else {
        let (mut px, mut py) = (dx, dy);
        if sa > 0.0 {
            // undo the forward smear, then test the round blob
            let aa = px * sdu + py * sdv;
            let qx = px - aa * sdu;
            let qy = py - aa * sdv;
            let s = if aa > 0.0 { 1.0 + sa } else { 1.0 + 0.25 * sa };
            px = qx + sdu * (aa / s);
            py = qy + sdv * (aa / s);
        }
        px.hypot(py) - r * blob_wobble(py.atan2(px), seed)
    }
}

/// RGBA8 ink atlas with one shelf-packed rect per paintable face.
pub struct InkAtlas {
    size: u32,
    ppm: f32,
    faces: Vec<Option<FaceAtlas>>,
    pixels: Vec<u8>,
    /// Inclusive dirty rect `(x0, y0, x1, y1)` since the last `take_dirty`.
    dirty: Option<(u32, u32, u32, u32)>,
    version: u32,
    splat_ids: Vec<u32>,
}

impl InkAtlas {
    /// Pack every paintable face of `world` into a fresh zeroed atlas.
    #[must_use]
    pub fn new(world: &CollisionWorld) -> Self {
        Self::with_layout(world, ATLAS_SIZE, MAX_DENSITY)
    }

    #[must_use]
    fn with_layout(world: &CollisionWorld, size: u32, max_density: f32) -> Self {
        let order: Vec<u32> = world
            .faces
            .iter()
            .filter(|f| f.paintable)
            .map(|f| f.id)
            .collect();
        let mut faces: Vec<Option<FaceAtlas>> = vec![None; world.faces.len()];
        let mut ppm = max_density;
        // JS `_layout`: shrink the density and retry until the shelf pack fits.
        let mut packed = false;
        for _ in 0..30 {
            if try_pack(world, &order, ppm, size, &mut faces) {
                packed = true;
                break;
            }
            faces.fill(None);
            ppm *= 0.92;
        }
        debug_assert!(
            packed || order.is_empty(),
            "ink atlas: no paintable face fits at density {ppm}"
        );
        let pixels = vec![0u8; (size as usize) * (size as usize) * 4];
        Self {
            size,
            ppm,
            faces,
            pixels,
            dirty: None,
            version: 0,
            splat_ids: Vec::new(),
        }
    }

    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    #[must_use]
    pub fn ppm(&self) -> f32 {
        self.ppm
    }

    /// Atlas rect of a paintable face, `None` if not paintable / not packed.
    #[must_use]
    pub fn face_atlas(&self, face: u32) -> Option<&FaceAtlas> {
        self.faces.get(face as usize)?.as_ref()
    }

    /// Raw RGBA8 pixels, row-major, `size * size * 4` bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Bumped once per face stamp that wrote pixels (upload trigger).
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Wipe every pixel and mark the whole atlas dirty (bumps `version`).
    /// Used when the gameplay grid is rebuilt (`Match::restart`) so both
    /// views stay in lock-step.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
        self.dirty = Some((0, 0, self.size - 1, self.size - 1));
        self.version += 1;
    }

    /// Take the dirty rect since the last call (inclusive bounds).
    pub fn take_dirty(&mut self) -> Option<(u32, u32, u32, u32)> {
        self.dirty.take()
    }

    /// Normalised RGBA at face-local metre coords `(cu, cv)` (texel floor).
    /// Test / debug helper mirroring `sample_face` UV math.
    #[must_use]
    pub fn sample_face(&self, face: u32, cu: f32, cv: f32) -> Option<[f32; 4]> {
        let fa = *self.faces.get(face as usize)?.as_ref()?;
        let px = fa.x as f32 + PAD as f32 + cu * fa.ppm;
        let py = fa.y as f32 + PAD as f32 + cv * fa.ppm;
        if px < fa.x as f32 || py < fa.y as f32 {
            return None;
        }
        let (px, py) = (px.floor() as u32, py.floor() as u32);
        if px >= fa.x + fa.w || py >= fa.y + fa.h {
            return None;
        }
        let k = (py as usize * self.size as usize + px as usize) * 4;
        let p = &self.pixels[k..k + 4];
        Some([
            p[0] as f32 / 255.0,
            p[1] as f32 / 255.0,
            p[2] as f32 / 255.0,
            p[3] as f32 / 255.0,
        ])
    }

    /// Replay a splat onto the bitmap. Mirrors the face traversal of
    /// [`PaintGrid::splat`](crate::paint::PaintGrid::splat) exactly so both
    /// views stay consistent for the same `(world, center, radius, team, opts)`
    /// stream; only the per-texel rasterisation differs (soft alpha, no
    /// gameplay bookkeeping).
    pub fn splat(
        &mut self,
        world: &CollisionWorld,
        center: Vec3,
        radius: f32,
        team: usize,
        opts: &SplatOpts,
    ) {
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

        let mut ids = std::mem::take(&mut self.splat_ids);
        ids.clear();
        world.query_blocks(
            center.x - reach,
            center.z - reach,
            center.x + reach,
            center.z + reach,
            &mut ids,
        );
        for &bid in &ids {
            let b = &world.blocks[bid as usize];
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
                    }
                }
                self.stamp_face(
                    f,
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
    }

    /// Rasterise one splat onto one face rect (CPU counterpart of the GPU
    /// splat pass body branch: `a = 1 - smoothstep(-1.5 fw, 1.5 fw, sd)` with
    /// `fw = 1 / ppm` as the `fwidth` proxy).
    fn stamp_face(&mut self, f: &Face, a: SplatArgs) {
        let SplatArgs {
            lu,
            lv,
            r,
            team,
            seed,
            kind,
            ..
        } = a;
        if r <= 0.02 {
            return;
        }
        let fa = match self.faces[f.id as usize] {
            Some(fa) => fa,
            None => return,
        };
        let ppm = fa.ppm;
        let roll = kind == SplatKind::Roll;
        let ext = if roll {
            r * ((BAND_L * BAND_L + BAND_W * BAND_W).sqrt() + BAND_R + 0.05)
        } else {
            r * (1.0 + a.sa) * WOB_MAX
        };
        // clamp the stamp range into the rect, pad included (JS `padM`)
        let pad_m = (PAD as f32 - 0.5) / ppm;
        let u0 = (lu - ext).max(-pad_m);
        let u1 = (lu + ext).min(f.su + pad_m);
        let v0 = (lv - ext).max(-pad_m);
        let v1 = (lv + ext).min(f.sv + pad_m);
        if u1 <= u0 || v1 <= v0 {
            return;
        }
        let tx = 1.0 / ppm;
        let tone = hsh(seed * 1.73);
        let tf = team as f32;
        let c0 = fa.x + ((u0 + pad_m) * ppm).floor().max(0.0) as u32;
        let c1 = fa.x + (((u1 + pad_m) * ppm).floor() as i64).clamp(0, fa.w as i64 - 1) as u32;
        let r0 = fa.y + ((v0 + pad_m) * ppm).floor().max(0.0) as u32;
        let r1 = fa.y + (((v1 + pad_m) * ppm).floor() as i64).clamp(0, fa.h as i64 - 1) as u32;

        let size = self.size as usize;
        let mut wrote = false;
        for py in r0..=r1 {
            let v = (py - fa.y) as f32 - PAD as f32 + 0.5;
            let dy = v / ppm - lv;
            for px in c0..=c1 {
                let u = (px - fa.x) as f32 - PAD as f32 + 0.5;
                let dx = u / ppm - lu;
                let sd = body_sdf(kind, dx, dy, a);
                let t = ((sd + 1.5 * tx) / (3.0 * tx)).clamp(0.0, 1.0);
                let al = 1.0 - t * t * (3.0 - 2.0 * t);
                if al <= 0.002 {
                    continue;
                }
                let k = (py as usize * size + px as usize) * 4;
                let p = &mut self.pixels[k..k + 4];
                let inv = 1.0 - al;
                let nr = tf * al + (p[0] as f32 / 255.0) * inv;
                let ng = al + (p[1] as f32 / 255.0) * inv;
                let nb = tone * al + (p[2] as f32 / 255.0) * inv;
                let na = al.max(p[3] as f32 / 255.0);
                p[0] = (nr * 255.0).round() as u8;
                p[1] = (ng * 255.0).round() as u8;
                p[2] = (nb * 255.0).round() as u8;
                p[3] = (na * 255.0).round() as u8;
                wrote = true;
            }
        }
        if wrote {
            self.dirty = Some(match self.dirty {
                Some((x0, y0, x1, y1)) => (x0.min(c0), y0.min(r0), x1.max(c1), y1.max(r1)),
                None => (c0, r0, c1, r1),
            });
            self.version += 1;
        }
    }
}

/// Shelf-pack `order` face rects at density `ppm` into `size x size`; writes
/// `out` on success and returns `true` (JS `_tryPack`).
fn try_pack(
    world: &CollisionWorld,
    order: &[u32],
    ppm: f32,
    size: u32,
    out: &mut [Option<FaceAtlas>],
) -> bool {
    let mut rects: Vec<(u32, u32, u32)> = order
        .iter()
        .map(|&id| {
            let f = &world.faces[id as usize];
            (
                id,
                (f.su * ppm).ceil() as u32 + 2 * PAD,
                (f.sv * ppm).ceil() as u32 + 2 * PAD,
            )
        })
        .collect();
    rects.sort_by_key(|r| std::cmp::Reverse(r.2));
    // Start at (PAD, PAD): non-paintable faces get the uv_b = (0, 0)
    // sentinel, which must never fall inside a writable rect.
    let (mut x, mut y, mut row_h) = (PAD, PAD, 0u32);
    for &(id, w, h) in &rects {
        if w > size {
            return false;
        }
        if x + w > size {
            x = PAD;
            y += row_h;
            row_h = 0;
        }
        if y + h > size {
            return false;
        }
        out[id as usize] = Some(FaceAtlas { ppm, x, y, w, h });
        x += w;
        row_h = row_h.max(h);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
    use crate::paint::{CELL, PaintGrid};
    use glam::vec3;

    const B: Bounds = Bounds {
        min_x: -30.0,
        max_x: 30.0,
        min_z: -30.0,
        max_z: 30.0,
    };

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

    /// 20x20 floor + wall strip along x=-10..-9 + a ramp (same stage as the
    /// paint tests).
    fn stage() -> CollisionWorld {
        CollisionWorld::from_brushes(
            &[
                bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
                bx([-10.0, 0.0, -10.0], [-9.0, 3.0, 10.0]),
                ramp(5.0),
            ],
            B,
        )
    }

    fn opts(seed: f32) -> SplatOpts {
        SplatOpts {
            seed,
            ..SplatOpts::default()
        }
    }

    fn top_face(world: &CollisionWorld) -> &Face {
        world
            .faces
            .iter()
            .find(|f| f.block == 0 && f.n.y > 0.9 && f.turf)
            .expect("floor has a turf top face")
    }

    // Review M-1: tone hash matches upstream GLSL `fract(sin(n) * 43758.5453123)`.
    #[test]
    fn hsh_matches_upstream_formula() {
        #[allow(clippy::excessive_precision)]
        let upstream = |n: f32| (n.sin() * 43758.5453123) - (n.sin() * 43758.5453123).floor();
        for n in [0.0f32, 1.73, 7.31, 13.1, 123.456] {
            assert_eq!(hsh(n), upstream(n), "hsh({n})");
        }
    }

    // Layout: every paintable face gets a padded rect, rects never overlap and
    // all fit inside the atlas.
    #[test]
    fn layout_is_deterministic_and_pads_faces() {
        let w = stage();
        let a = InkAtlas::new(&w);
        assert!(a.ppm() > 0.0);
        let mut rects: Vec<(u32, &FaceAtlas)> = w
            .faces
            .iter()
            .filter(|f| f.paintable)
            .map(|f| (f.id, a.face_atlas(f.id).expect("paintable face is packed")))
            .collect();
        assert!(!rects.is_empty());
        for (id, fa) in rects.iter() {
            let f = &w.faces[*id as usize];
            assert_eq!(fa.w, (f.su * fa.ppm).ceil() as u32 + 2 * PAD);
            assert_eq!(fa.h, (f.sv * fa.ppm).ceil() as u32 + 2 * PAD);
            assert!(fa.x + fa.w <= a.size());
            assert!(fa.y + fa.h <= a.size());
        }
        rects.sort_by_key(|(id, _)| *id);
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (_, p) = rects[i];
                let (_, q) = rects[j];
                let disjoint =
                    p.x + p.w <= q.x || q.x + q.w <= p.x || p.y + p.h <= q.y || q.y + q.h <= p.y;
                assert!(disjoint, "face rects {} and {} overlap", i, j);
            }
        }
    }

    // TR-11.1: the atlas bitmap agrees with the gameplay grid cell-by-cell.
    // Interior cells (all four neighbours share the grid value) must cross the
    // 0.5 alpha line; boundary cells may differ by up to one cell.
    #[test]
    fn tr11_1_atlas_matches_grid_per_cell() {
        let w = stage();
        let mut g = PaintGrid::new(&w);
        let mut a = InkAtlas::new(&w);
        let stream = [
            (vec3(0.0, 0.0, 0.0), 2.0, 0usize, 1.3f32),
            (vec3(5.0, 0.0, -5.0), 1.5, 1, 7.1),
            (vec3(-6.0, 0.0, 5.0), 1.0, 0, 3.7),
            (vec3(-9.0, 1.5, 0.0), 1.2, 1, 5.5),
        ];
        for (c, r, t, s) in stream {
            let o = opts(s);
            g.splat(&w, c, r, t, &o);
            a.splat(&w, c, r, t, &o);
        }
        let mut checked = 0usize;
        let mut violations = 0usize;
        for f in &w.faces {
            if !f.paintable {
                continue;
            }
            let nu = (f.su / CELL).round().max(1.0) as i64;
            let nv = (f.sv / CELL).round().max(1.0) as i64;
            let cu = f.su / nu as f32;
            let cv = f.sv / nv as f32;
            let gs = |i: i64, j: i64| -> u8 {
                let i = i.clamp(0, nu - 1);
                let j = j.clamp(0, nv - 1);
                g.sample(f.id as i32, (i as f32 + 0.5) * cu, (j as f32 + 0.5) * cv)
            };
            for j in 0..nv {
                for i in 0..nu {
                    let u = (i as f32 + 0.5) * cu;
                    let v = (j as f32 + 0.5) * cv;
                    let gv = gs(i, j);
                    let Some(s) = a.sample_face(f.id, u, v) else {
                        continue;
                    };
                    checked += 1;
                    let painted = gv != 0;
                    let interior = (gs(i - 1, j) != 0) == painted
                        && (gs(i + 1, j) != 0) == painted
                        && (gs(i, j - 1) != 0) == painted
                        && (gs(i, j + 1) != 0) == painted;
                    if !interior {
                        continue; // boundary cell: one-cell error is allowed
                    }
                    if painted && s[3] < 0.5 {
                        violations += 1;
                    }
                    if !painted && s[3] > 0.5 {
                        violations += 1;
                    }
                    if painted && s[3] > 0.98 {
                        // fully covered: premultiplied R / A recovers the team
                        let expect = (gv - 1) as f32;
                        assert!(
                            (s[0] / s[3] - expect).abs() < 0.05,
                            "team share mismatch at face {} ({u},{v}): {:?}",
                            f.id,
                            s
                        );
                    }
                }
            }
        }
        assert!(checked > 1000, "too few cells checked: {checked}");
        assert_eq!(violations, 0, "atlas/grid disagreement over interior cells");
    }

    // Over-composite: the newest splat wins the colour.
    #[test]
    fn over_composite_newest_wins() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        a.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 0, &opts(1.3));
        a.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 1, &opts(9.9));
        let f = top_face(&w);
        let s = a.sample_face(f.id, 10.0, 10.0).expect("centre painted");
        assert!(s[3] > 0.9, "coverage {s:?}");
        assert!(s[0] / s[3] > 0.9, "team1 wins: {s:?}");
        assert!(s[1] > 0.9, "wetness: {s:?}");
    }

    // TR-11.2 (headless half): 1000 splats - fixed pixel buffer, bounded dirty
    // rect, version keeps advancing, no allocation growth.
    #[test]
    fn tr11_2_1000_splats_stable() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        let expect_len = (ATLAS_SIZE as usize) * (ATLAS_SIZE as usize) * 4;
        assert_eq!(a.pixels().len(), expect_len);
        let mut x = 1.0f32;
        for n in 0..1000u32 {
            x = (x * 1664525.0 + 1013904223.0) % 4294967296.0;
            let r1 = x / 4294967296.0;
            x = (x * 1664525.0 + 1013904223.0) % 4294967296.0;
            let r2 = x / 4294967296.0;
            let c = vec3(r1 * 18.0 - 9.0, 0.0, r2 * 18.0 - 9.0);
            a.splat(&w, c, 0.5 + r1 * 2.0, (n % 2) as usize, &opts(n as f32));
            assert_eq!(a.pixels().len(), expect_len);
        }
        assert!(a.version() > 0);
        let d = a.take_dirty().expect("dirty after splats");
        let area = (d.2 - d.0 + 1) as usize * (d.3 - d.1 + 1) as usize;
        assert!(area <= expect_len / 4);
        assert!(a.take_dirty().is_none(), "dirty cleared");
    }

    // A splat on a wall stamps the wall face rect (not just the floor).
    #[test]
    fn wall_splat_stamps_wall_face() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        let f = w
            .faces
            .iter()
            .find(|fc| fc.block == 1 && fc.n.x > 0.9)
            .expect("wall strip has a +X face");
        let center = vec3(-9.0, 1.5, 0.0);
        a.splat(&w, center, 1.2, 0, &opts(4.2));
        let rel = center - f.origin;
        let s = a
            .sample_face(f.id, rel.dot(f.u), rel.dot(f.v))
            .expect("wall centre painted");
        assert!(s[3] > 0.5, "wall alpha {s:?}");
    }

    // Determinism: the same splat stream yields byte-identical bitmaps.
    #[test]
    fn splat_sequence_is_deterministic() {
        let w = stage();
        let mut a1 = InkAtlas::new(&w);
        let mut a2 = InkAtlas::new(&w);
        for n in 0..50u32 {
            let c = vec3(
                (n as f32 * 1.7).sin() * 8.0,
                0.0,
                (n as f32 * 2.3).cos() * 8.0,
            );
            let o = opts(n as f32 * 0.11);
            a1.splat(&w, c, 1.0 + (n % 3) as f32 * 0.5, (n % 2) as usize, &o);
            a2.splat(&w, c, 1.0 + (n % 3) as f32 * 0.5, (n % 2) as usize, &o);
        }
        assert_eq!(a1.pixels(), a2.pixels());
        assert_eq!(a1.version(), a2.version());
    }

    // Review M-2: the shelf pack starts at (PAD, PAD) so the uv_b = (0, 0)
    // sentinel of non-paintable faces can never read writable ink.
    #[test]
    fn pack_origin_keeps_the_zero_uv_sentinel_clean() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        // paint everything hard against the lowest-left corner of the first
        // rect (a wall splat near the floor/wall seam reaches the pad ring)
        for n in 0..30u32 {
            a.splat(
                &w,
                vec3(-9.5, 0.05, n as f32 * 0.6 - 9.0),
                1.5,
                (n % 2) as usize,
                &opts(n as f32),
            );
        }
        // texel (0, 0) must stay untouched
        let k = 0usize;
        assert_eq!(
            &a.pixels()[k..k + 4],
            &[0, 0, 0, 0],
            "sentinel texel written"
        );
        // every packed rect starts at >= PAD
        for f in w.faces.iter().filter(|f| f.paintable) {
            let fa = a.face_atlas(f.id).expect("packed");
            assert!(fa.x >= PAD && fa.y >= PAD, "rect at ({}, {})", fa.x, fa.y);
        }
    }

    // Review M-3: clear() wipes the bitmap and bumps the version.
    #[test]
    fn clear_wipes_pixels_and_bumps_version() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        a.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 0, &opts(1.3));
        assert!(a.pixels().iter().any(|&v| v != 0));
        let v = a.version();
        a.clear();
        assert!(a.pixels().iter().all(|&v| v == 0));
        assert_eq!(a.version(), v + 1);
        assert_eq!(a.take_dirty(), Some((0, 0, a.size() - 1, a.size() - 1)));
    }

    // The UV mapping fed to the renderer round-trips to the sampled texel.
    #[test]
    fn uv_at_matches_sample_face() {
        let w = stage();
        let mut a = InkAtlas::new(&w);
        a.splat(&w, vec3(0.0, 0.0, 0.0), 2.0, 1, &opts(1.3));
        let f = top_face(&w);
        let fa = a.face_atlas(f.id).expect("packed");
        let (u, v) = (9.6f32, 10.4f32);
        let (uu, vv) = fa.uv_at(u, v, a.size());
        let px = (uu * a.size() as f32).floor() as usize;
        let py = (vv * a.size() as f32).floor() as usize;
        let k = (py * a.size() as usize + px) * 4;
        let p = &a.pixels()[k..k + 4];
        let s = a.sample_face(f.id, u, v).expect("inside rect");
        assert_eq!(
            s,
            [
                p[0] as f32 / 255.0,
                p[1] as f32 / 255.0,
                p[2] as f32 / 255.0,
                p[3] as f32 / 255.0
            ]
        );
        assert!(s[3] > 0.5);
    }
}
