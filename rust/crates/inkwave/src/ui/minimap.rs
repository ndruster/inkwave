//! Task 49 (D-T49-1/2) — minimap pure core, port of `src/game/minimap.js`.
//!
//! No Bevy dependency: every function here is a pure computation over the
//! sim's public accessors ([`PaintGrid`], [`CollisionWorld`], [`Actor`] …)
//! and plain buffers, mirroring the JS raster pipeline pixel-for-pixel
//! (spec "批次 A/B"). The Bevy integration lives in [`super::minimap_ui`].
//!
//! Fidelity notes (spec D-T49-5):
//! - ③ block / team colours are read as **sRGB bytes straight from the hex**
//!   — JS round-trips `THREE.Color` (sRGB→linear) through `lin2s` (linear→
//!   sRGB), which is the identity up to rounding; the `lin2s` port is kept
//!   for the formula table and any linear-space input.
//! - All `Uint8ClampedArray` writes use [`clamp_u8`] (JS clamps + rounds
//!   half-to-even); `Math.round` sites use [`js_round`] (half-up). Copying
//!   the JS operation order keeps the f32 math deterministic (NFR-M4-1 / TR-49.1).
//! - Deferred to a later batch (D-T49-5 ①): spawn pads, transient fx, clouds,
//!   bombs, subs, vortex/specials, jump lines, death markers, beacons.

use inkwave_sim::actor::{Actor, Form};
use inkwave_sim::collision::{Block, CollisionWorld, Face};
use inkwave_sim::geometry::{Bounds, Brush, ZoneDef};
use inkwave_sim::paint::PaintGrid;

/// JS `BANDS` (minimap.js L349): the ink refresh is spread over this many
/// consecutive frames so one frame never pays for the whole bilinear field
/// (L288-289 comment).
pub const INK_BANDS: usize = 3;
/// JS L356: `this.timer = 0.15` — ink poll interval (seconds).
pub const REFRESH_INTERVAL: f32 = 0.15;
/// JS L371: the flash overlay decays over this window.
pub const FLASH_DECAY: f32 = 0.45;

/// JS `lin2s` (L56): linear → sRGB, `Math.round(255 * …)`. Kept f32 so the
/// arithmetic in `colOf` / `_teamRGB` matches JS exactly (those consume the
/// rounded number as a float, not a clamped byte).
/// D-T49-5 ③: the app path reads hex as sRGB bytes directly, so this is the
/// formula table + test anchor (and the seam for any linear-space input).
#[must_use]
#[allow(dead_code)]
pub fn lin2s(c: f32) -> f32 {
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    js_round(255.0 * s)
}

/// JS `smooth` (L57): smoothstep between `a` and `b`.
#[must_use]
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// JS `Math.round` (half-up toward +∞), unlike Rust's half-away-from-zero.
#[must_use]
fn js_round(x: f32) -> f32 {
    (x + 0.5).floor()
}

/// JS `Uint8ClampedArray` element write: NaN → 0, clamp to 0..=255, round
/// half-to-even.
#[must_use]
fn clamp_u8(x: f32) -> u8 {
    if x.is_nan() || x <= 0.0 {
        0
    } else if x >= 255.0 {
        255
    } else {
        x.round_ties_even() as u8
    }
}

/// JS `lvl.blocks[id].color` → sRGB bytes (`#rgb` / `#rrggbb`; world.rs
/// `hexColor` L84 semantics: leading `#` optional, 3-digit expands, bad
/// input falls back to 221).
#[must_use]
pub fn hex_rgb(s: &str) -> [f32; 3] {
    let h = s.trim().trim_start_matches('#');
    let nib = |c: u8| -> Option<u32> {
        match c {
            b'0'..=b'9' => Some(u32::from(c - b'0')),
            b'a'..=b'f' => Some(u32::from(c - b'a') + 10),
            b'A'..=b'F' => Some(u32::from(c - b'A') + 10),
            _ => None,
        }
    };
    let byte2 = |i: usize| -> Option<u32> {
        let hi = nib(h.as_bytes()[i])?;
        let lo = nib(h.as_bytes()[i + 1])?;
        Some(hi * 16 + lo)
    };
    let parse = || -> Option<[f32; 3]> {
        if h.len() == 3 {
            let mut out = [0.0f32; 3];
            for (k, slot) in out.iter_mut().enumerate() {
                let v = nib(h.as_bytes()[k])?;
                *slot = (v * 17) as f32; // #abc → 0xaabbcc
            }
            Some(out)
        } else if h.len() >= 6 {
            Some([byte2(0)? as f32, byte2(2)? as f32, byte2(4)? as f32])
        } else {
            None
        }
    };
    parse().unwrap_or([221.0, 221.0, 221.0])
}

/// JS `G.teamHex || ['#ff8a14', '#2f5bff']` + `_teamRGB` fallback (L282-286):
/// the two squad colours as sRGB floats. `_teamRGB` linearises `THREE.Color`
/// then `lin2s` back — identity up to rounding (D-T49-5 ③), so the hex bytes
/// are used directly.
pub const TEAM_RGB: ([f32; 3], [f32; 3]) = ([255.0, 138.0, 20.0], [47.0, 91.0, 255.0]);

/// Raster resolution. JS default `pxPerM = 7` (L61) is [`MinimapTier::Med`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the app runs `Med` (the JS default); the other tiers
// are the spec D-T49-2 table for a future size-based selector.
pub enum MinimapTier {
    /// 4 px per metre.
    Low,
    /// 7 px per metre — the JS default (L61).
    Med,
    /// 10 px per metre.
    High,
}

impl MinimapTier {
    #[must_use]
    pub const fn px_per_m(self) -> f32 {
        match self {
            Self::Low => 4.0,
            Self::Med => 7.0,
            Self::High => 10.0,
        }
    }
}

/// JS `Minimap` canvas mapping (L101-109): world XZ → float canvas px,
/// with the Bravo viewer's 180° flip (L90-97).
#[derive(Debug, Clone, Copy)]
pub struct Projector {
    /// px per metre (`this.s`).
    pub s: f32,
    /// canvas width px (`this.w`).
    pub w: u32,
    /// canvas height px (`this.h`).
    pub h: u32,
    /// `this.flip` — viewer team 1.
    pub flip: bool,
    max_x: f32,
    max_z: f32,
}

impl Projector {
    #[must_use]
    pub fn new(bounds: &Bounds, px_per_m: f32, flip: bool) -> Self {
        Self {
            s: px_per_m,
            w: js_round((bounds.max_x - bounds.min_x) * px_per_m) as u32,
            h: js_round((bounds.max_z - bounds.min_z) * px_per_m) as u32,
            flip,
            max_x: bounds.max_x,
            max_z: bounds.max_z,
        }
    }

    /// JS `toCanvas` (L101-107).
    /// JS name kept verbatim (`toCanvas`, L101) — the `to_*` self-convention
    /// does not apply to a projection, only to conversions.
    #[allow(clippy::wrong_self_convention)]
    #[must_use]
    pub fn to_canvas(&self, x: f32, z: f32) -> (f32, f32) {
        let mut cx = (self.max_x - x) * self.s;
        let mut cy = (self.max_z - z) * self.s;
        if self.flip {
            cx = self.w as f32 - cx;
            cy = self.h as f32 - cy;
        }
        (cx, cy)
    }

    /// JS `_worldX` (L108).
    #[must_use]
    pub fn world_x(&self, px: f32) -> f32 {
        let c = if self.flip { self.w as f32 - px } else { px };
        self.max_x - c / self.s
    }

    /// JS `_worldZ` (L109).
    #[must_use]
    pub fn world_z(&self, py: f32) -> f32 {
        let c = if self.flip { self.h as f32 - py } else { py };
        self.max_z - c / self.s
    }
}

/// Per-block draw style resolved from the layout primitive (spec: [`Block`]
/// lacks `color`/`perch`, so they come from `StageLayout::primitives[bid]
/// .common()` — the world.rs L613-625 positional `bid ↔ block.id` precedent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockStyle {
    /// sRGB bytes 0..=255 (JS `b.color` through `lin2s`, D-T49-5 ③).
    pub color: [f32; 3],
    /// JS `b.grate`.
    pub grate: bool,
    /// JS `b.hidden`.
    pub hidden: bool,
}

/// Build the [`BlockStyle`] table + the `perch` flags array consumed by
/// [`RasterTables::build`], positionally aligned with
/// `CollisionWorld::blocks` (same filtered-primitive order as world.rs
/// L613-625).
#[must_use]
pub fn layout_styles(primitives: &[Brush], blocks: &[Block]) -> (Vec<BlockStyle>, Vec<bool>) {
    let mut styles = Vec::with_capacity(blocks.len());
    let mut perch = Vec::with_capacity(blocks.len());
    for (bid, b) in blocks.iter().enumerate() {
        let common = primitives.get(bid).map(Brush::common);
        let color = common.map_or([221.0, 221.0, 221.0], |c| hex_rgb(&c.color));
        perch.push(common.is_some_and(|c| c.perch));
        styles.push(BlockStyle {
            color,
            grate: b.grate,
            hidden: b.hidden,
        });
    }
    (styles, perch)
}

/// JS `_build` tables (L112-187): the block-top height raster (hgt/top/nrm)
/// plus the turf bilinear sampling tables (pixCell/pixFx/pixFy/pixSx/pixSy).
#[derive(Debug, Clone)]
pub struct RasterTables {
    /// JS `this.hgt` — top-surface height per pixel, `-99` where none.
    pub hgt: Vec<f32>,
    /// JS `this.topBlock` — owning block id per pixel, `-1` = sea.
    pub top: Vec<i32>,
    /// JS `this.nrm` — canvas-space top normal (2 floats per pixel).
    pub nrm: Vec<f32>,
    /// JS `this.pixCell` — paint cell index per pixel, `-1` = no turf.
    pub pix_cell: Vec<i32>,
    /// JS `this.pixFx` — bilinear u fraction × 255.
    pub pix_fx: Vec<u8>,
    /// JS `this.pixFy` — bilinear v fraction × 255.
    pub pix_fy: Vec<u8>,
    /// JS `this.pixSx` — neighbour step in +u (0/1) when that cell is alive.
    pub pix_sx: Vec<i8>,
    /// JS `this.pixSy` — neighbour step in +v (0 or `nu`) when alive.
    pub pix_sy: Vec<i32>,
}

impl RasterTables {
    /// JS `_build` (L112-187). `perch[bid]` supplies `b.perch` (the skip
    /// rule L121 needs it; `b.roof` comes from [`Block::is_roof`]).
    #[must_use]
    pub fn build(
        proj: &Projector,
        world: &CollisionWorld,
        paint: &PaintGrid,
        perch: &[bool],
    ) -> Self {
        let n = proj.w as usize * proj.h as usize;
        let mut s = Self {
            hgt: vec![-99.0; n],
            top: vec![-1; n],
            nrm: vec![0.0; n * 2],
            pix_cell: vec![-1; n],
            pix_fx: vec![0; n],
            pix_fy: vec![0; n],
            pix_sx: vec![0; n],
            pix_sy: vec![0; n],
        };
        s.raster_block_tops(proj, &world.blocks, perch);
        s.raster_turf_cells(proj, &world.faces, paint);
        s
    }

    /// JS `_build` step 1 (L119-142): rasterise every solid block's top face.
    fn raster_block_tops(&mut self, proj: &Projector, blocks: &[Block], perch: &[bool]) {
        let w = proj.w as usize;
        let h = proj.h as usize;
        for (bid, b) in blocks.iter().enumerate() {
            if !b.solid {
                continue;
            }
            // L121 verbatim: overhead steel (crane girders, booms) is not over the turf.
            if b.hidden
                && (b.is_roof() || perch.get(bid).copied().unwrap_or(false))
                && b.aabb_min.y > 3.0
            {
                continue;
            }
            let n = b.axes[1];
            if n.y < 0.45 {
                continue;
            }
            let tx = b.center.x + n.x * b.half.y;
            let ty = b.center.y + n.y * b.half.y;
            let tz = b.center.z + n.z * b.half.y;
            let (cx0, cy0) = proj.to_canvas(b.aabb_max.x, b.aabb_max.z);
            let (cx1, cy1) = proj.to_canvas(b.aabb_min.x, b.aabb_min.z);
            let (x0, x1) = (cx0.min(cx1), cx0.max(cx1));
            let (y0, y1) = (cy0.min(cy1), cy0.max(cy1));
            // JS: px0 = max(0, floor(x0)); px1 = min(W-1, ceil(x1)) — a fully
            // off-canvas block yields px1 < px0 and the loop is empty.
            let px0 = x0.floor().max(0.0) as i64;
            let px1 = (x1.ceil() as i64).min(w as i64 - 1);
            let py0 = y0.floor().max(0.0) as i64;
            let py1 = (y1.ceil() as i64).min(h as i64 - 1);
            if px0 > px1 || py0 > py1 {
                continue;
            }
            let a0 = b.axes[0];
            let a2 = b.axes[2];
            // L130: normal in canvas space.
            let cnx = if proj.flip { n.x } else { -n.x };
            let cny = if proj.flip { n.z } else { -n.z };
            for py in py0..=py1 {
                let z = proj.world_z(py as f32 + 0.5);
                for px in px0..=px1 {
                    let x = proj.world_x(px as f32 + 0.5);
                    let y = ty - (n.x * (x - tx) + n.z * (z - tz)) / n.y;
                    let dx = x - b.center.x;
                    let dy = y - 0.01 - b.center.y;
                    let dz = z - b.center.z;
                    if (dx * a0.x + dy * a0.y + dz * a0.z).abs() > b.half.x
                        || (dx * a2.x + dy * a2.y + dz * a2.z).abs() > b.half.z
                    {
                        continue;
                    }
                    let i = py as usize * w + px as usize;
                    if y > self.hgt[i] {
                        self.hgt[i] = y;
                        self.top[i] = bid as i32;
                        self.nrm[i * 2] = cnx;
                        self.nrm[i * 2 + 1] = cny;
                    }
                }
            }
        }
    }

    /// JS `_build` step 2 (L143-183): turf pixels → paint cells.
    fn raster_turf_cells(&mut self, proj: &Projector, faces: &[Face], paint: &PaintGrid) {
        let w = proj.w as usize;
        let h = proj.h as usize;
        // L145: paintable turf faces sorted bottom-first (stable sort, JS
        // `Array.sort` is stable in the target engines).
        let mut order: Vec<usize> = (0..faces.len())
            .filter(|&fi| {
                let f = &faces[fi];
                f.turf && paint.face_meta(f.id as usize).is_some()
            })
            .collect();
        order.sort_by(|&a, &b| faces[a].origin.y.total_cmp(&faces[b].origin.y));
        for fi in order {
            let f = &faces[fi];
            if f.n.y < 0.45 {
                continue;
            }
            let (ux, uz) = (f.u.x, f.u.z);
            let (vx, vz) = (f.v.x, f.v.z);
            let uh = ux * ux + uz * uz;
            let vh = vx * vx + vz * vz;
            if uh < 1e-4 || vh < 1e-4 {
                continue;
            }
            let Some((nu, nv, cu, cv, off)) = paint.face_meta(f.id as usize) else {
                continue;
            };
            if nu == 0 || nv == 0 {
                continue;
            }
            // canvas bbox of the four corners (L155-159)
            let mut bx0 = f32::INFINITY;
            let mut by0 = f32::INFINITY;
            let mut bx1 = f32::NEG_INFINITY;
            let mut by1 = f32::NEG_INFINITY;
            for (a, c) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let (cx, cy) = proj.to_canvas(
                    f.origin.x + ux * f.su * a + vx * f.sv * c,
                    f.origin.z + uz * f.su * a + vz * f.sv * c,
                );
                bx0 = bx0.min(cx);
                bx1 = bx1.max(cx);
                by0 = by0.min(cy);
                by1 = by1.max(cy);
            }
            let px0 = bx0.floor().max(0.0) as i64;
            let px1 = (bx1.ceil() as i64).min(w as i64 - 1);
            let py0 = by0.floor().max(0.0) as i64;
            let py1 = (by1.ceil() as i64).min(h as i64 - 1);
            if px0 > px1 || py0 > py1 {
                continue;
            }
            let face_top = f.origin.y + (f.u.y * f.su).max(0.0) + (f.v.y * f.sv).max(0.0);
            for py in py0..=py1 {
                let z = proj.world_z(py as f32 + 0.5);
                for px in px0..=px1 {
                    // L166 verbatim: something taller covers this face here.
                    let i = py as usize * w + px as usize;
                    if self.hgt[i] > face_top + 0.05 {
                        continue;
                    }
                    let x = proj.world_x(px as f32 + 0.5);
                    let dx = x - f.origin.x;
                    let dz = z - f.origin.z;
                    let a = (dx * ux + dz * uz) / uh;
                    let c = (dx * vx + dz * vz) / vh;
                    if a < 0.0 || c < 0.0 || a >= f.su || c >= f.sv {
                        continue;
                    }
                    let cu_ = a / cu - 0.5;
                    let cv_ = c / cv - 0.5;
                    let mut iu = cu_.floor();
                    let mut iv = cv_.floor();
                    let mut tu = cu_ - iu;
                    let mut tv = cv_ - iv;
                    if iu < 0.0 {
                        iu = 0.0;
                        tu = 0.0;
                    }
                    if iv < 0.0 {
                        iv = 0.0;
                        tv = 0.0;
                    }
                    if iu >= nu as f32 - 1.0 {
                        iu = nu as f32 - 1.0;
                        tu = 0.0;
                    }
                    if iv >= nv as f32 - 1.0 {
                        iv = nv as f32 - 1.0;
                        tv = 0.0;
                    }
                    let iu = iu as usize;
                    let iv = iv as usize;
                    let k = off + iv * nu as usize + iu;
                    if paint.cell_dead(k) {
                        continue;
                    }
                    self.pix_cell[i] = k as i32;
                    self.pix_fx[i] = js_round(tu * 255.0) as u8;
                    self.pix_fy[i] = js_round(tv * 255.0) as u8;
                    self.pix_sx[i] = i8::from(iu < nu as usize - 1 && !paint.cell_dead(k + 1));
                    self.pix_sy[i] = if iv < nv as usize - 1 && !paint.cell_dead(k + nu as usize) {
                        nu as i32
                    } else {
                        0
                    };
                }
            }
        }
    }
}

/// JS `_drawBase` (L189-279): the static deck — sea/swell/foam, block colour
/// lifted toward paper, grate hatch, height tint, hillshade, cast shadow,
/// contact AO + drop-off rim, and the 5 m grid overlay. Returns RGBA bytes.
#[must_use]
#[allow(clippy::manual_clamp)] // keep the JS `Math.max(Math.min(…))` expansion
// verbatim so each clamp maps 1:1 to its source expression; the inputs are
// finite by construction (no NaN path), so this is expression fidelity, not
// NaN semantics (Rust `f32::min/max` differ from JS there anyway).
pub fn draw_base(
    proj: &Projector,
    tables: &RasterTables,
    styles: &[BlockStyle],
    sunset: bool,
) -> Vec<u8> {
    let w = proj.w as usize;
    let h = proj.h as usize;
    let n = w * h;
    let s = proj.s;
    let hgt = &tables.hgt;
    let top = &tables.top;
    let nrm = &tables.nrm;
    let mut d = vec![0u8; n * 4];
    // L194-195: sun from the top-left; shadows fall toward the bottom-right.
    const LX: f32 = -0.62;
    const LY: f32 = -0.78;
    const SHADOW_SLOPE: f32 = 0.11; // metres of height per pixel of shadow length
    let paper: [f32; 3] = if sunset {
        [236.0, 214.0, 196.0]
    } else {
        [242.0, 236.0, 222.0]
    };
    let sea0: [f32; 3] = if sunset {
        [70.0, 64.0, 120.0]
    } else {
        [36.0, 104.0, 170.0]
    };
    let sea1: [f32; 3] = if sunset {
        [104.0, 86.0, 150.0]
    } else {
        [62.0, 146.0, 206.0]
    };
    // JS `colOf` (L198-211), computed per pixel (same value, no cache needed).
    let col_of = |id: usize| -> [f32; 3] {
        let b = &styles[id];
        let k = if b.hidden { 0.35 } else { 0.52 };
        let mut c = [
            b.color[0] + (paper[0] - b.color[0]) * k,
            b.color[1] + (paper[1] - b.color[1]) * k,
            b.color[2] + (paper[2] - b.color[2]) * k,
        ];
        if b.grate {
            c = [150.0, 162.0, 176.0];
        }
        if b.hidden {
            c = [c[0] * 0.78, c[1] * 0.78, c[2] * 0.84];
        }
        c
    };
    for py in 0..h {
        for px in 0..w {
            let i = py * w + px;
            let o = i * 4;
            let hh = hgt[i];
            if top[i] < 0 {
                // L216-228: sea gradient + soft swell bands + foam hugging the dock.
                let pxf = px as f32;
                let pyf = py as f32;
                let wave = 0.5
                    + 0.5
                        * ((pxf * 0.9 + pyf * 0.55) * 0.13).sin()
                        * (pyf * 0.05 + pxf * 0.012).sin();
                let mut near = 0.0f32;
                for r in 1..=6usize {
                    if near != 0.0 {
                        break;
                    }
                    let hit = (px >= r && top[i - r] >= 0)
                        || (px + r < w && top[i + r] >= 0)
                        || (py >= r && top[i - r * w] >= 0)
                        || (py + r < h && top[i + r * w] >= 0);
                    if hit {
                        near = 1.0 - (r as f32 - 1.0) / 6.0;
                    }
                }
                let k = 0.25 + wave * 0.35;
                d[o] = clamp_u8(sea0[0] + (sea1[0] - sea0[0]) * k + near * 60.0);
                d[o + 1] = clamp_u8(sea0[1] + (sea1[1] - sea0[1]) * k + near * 55.0);
                d[o + 2] = clamp_u8(sea0[2] + (sea1[2] - sea0[2]) * k + near * 40.0);
                d[o + 3] = 255;
                continue;
            }
            let c = col_of(top[i] as usize);
            // L232-233: height tint — trenches darker, raised decks lighter.
            let mut lum = 0.9 + (-0.14f32).max(0.16f32.min(hh * 0.035));
            // L234-236: hillshade for tilted surfaces (ramps).
            let nx = nrm[i * 2];
            let ny = nrm[i * 2 + 1];
            if nx * nx + ny * ny > 0.002 {
                lum *= 1.0 + (nx * LX + ny * LY) * 1.6;
            }
            // L237-245: soft cast shadow from taller geometry toward the light.
            let mut sh = 0.0f32;
            let mut k = 3.0f32;
            while k <= 27.0 {
                let qx = js_round(px as f32 + LX * k);
                let qy = js_round(py as f32 + LY * k);
                if qx < 0.0 || qy < 0.0 || qx >= w as f32 || qy >= h as f32 {
                    break;
                }
                let q = qx as usize + qy as usize * w;
                let dh = hgt[q] - hh - k * SHADOW_SLOPE;
                if dh > 0.0 {
                    sh = sh.max((dh / 0.5).min(1.0) * (1.0 - k / 30.0));
                }
                k += 3.0;
            }
            lum *= 1.0 - sh * 0.3;
            // L246-258: contact AO at the foot of walls + crisp rims on drop-offs.
            let mut hi = 0.0f32;
            let mut lo = 0.0f32;
            for oy in -2i32..=2 {
                for ox in -2i32..=2 {
                    if (ox == 0 && oy == 0) || (ox.abs() + oy.abs()) > 2 {
                        continue;
                    }
                    let qx = px as i32 + ox;
                    let qy = py as i32 + oy;
                    if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h {
                        continue;
                    }
                    let q = qx as usize + qy as usize * w;
                    let dh = (if top[q] < 0 { -3.0 } else { hgt[q] }) - hh;
                    let wgt = 1.0 / (ox.abs() + oy.abs()) as f32;
                    if dh > 0.3 {
                        hi += wgt * dh.min(1.0);
                    }
                    if dh < -0.3 && ox.abs() + oy.abs() == 1 {
                        lo = 1.0;
                    }
                }
            }
            lum *= 1.0 - (hi * 0.07).min(0.28);
            let mut r = c[0] * lum;
            let mut g = c[1] * lum;
            let mut bl = c[2] * lum;
            if lo != 0.0 {
                r *= 0.42;
                g *= 0.4;
                bl *= 0.48;
            }
            // L261-264: grate hatch.
            if styles[top[i] as usize].grate {
                let pxf = px as f32;
                let pyf = py as f32;
                let hatch = (pxf + pyf) % 4.0 < 1.5 || (pxf - pyf + 4000.0) % 4.0 < 1.5;
                if hatch {
                    r *= 0.62;
                    g *= 0.64;
                    bl *= 0.7;
                }
            }
            d[o] = clamp_u8(r);
            d[o + 1] = clamp_u8(g);
            d[o + 2] = clamp_u8(bl);
            d[o + 3] = 255;
        }
    }
    // L269-278: subtle 5 m grid over the deck. A 1 px line at integer+0.5
    // covers exactly one row/column; one stroke() op → overlapping pixels
    // blend once (tracked with `touched`).
    let step = 5.0 * s;
    let mut touched = vec![false; n];
    let blend = |col: usize, row: usize, d: &mut Vec<u8>, touched: &mut Vec<bool>| {
        if col >= w || row >= h || touched[row * w + col] {
            return;
        }
        touched[row * w + col] = true;
        let i = (row * w + col) * 4;
        d[i] = clamp_u8(27.0 * 0.07 + d[i] as f32 * 0.93);
        d[i + 1] = clamp_u8(24.0 * 0.07 + d[i + 1] as f32 * 0.93);
        d[i + 2] = clamp_u8(48.0 * 0.07 + d[i + 2] as f32 * 0.93);
    };
    let mut x = (proj.w as f32 % step) / 2.0;
    while x < proj.w as f32 {
        let col = js_round(x) as usize;
        for row in 0..h {
            blend(col, row, &mut d, &mut touched);
        }
        x += step;
    }
    let mut y = (proj.h as f32 % step) / 2.0;
    while y < proj.h as f32 {
        let row = js_round(y) as usize;
        for col in 0..w {
            blend(col, row, &mut d, &mut touched);
        }
        y += step;
    }
    d
}

/// JS `_alpha`/`_tm`/`_tt` scratch arrays (L296-298), persistent across
/// refreshes so band edges keep their bilinear context.
#[derive(Debug, Clone, Default)]
pub struct InkScratch {
    /// coverage 0..1 per pixel.
    pub al: Vec<f32>,
    /// team drawn at this pixel when `al > 0.5` (0/1/2).
    pub tm: Vec<u8>,
    /// nearest team (1/2) for the emboss/flash logic.
    pub tt: Vec<u8>,
}

impl InkScratch {
    #[must_use]
    pub fn with_size(n: usize) -> Self {
        Self {
            al: vec![0.0; n],
            tm: vec![0; n],
            tt: vec![0; n],
        }
    }
}

/// JS `_drawInk` (L290-338): rows `[y0, y1)` of the ink + flash overlays from
/// the paint grid via the bilinear tables. `owner` is JS `this.owner` (the
/// per-pixel team memory that turns team changes into white flashes).
/// Returns `flashes > 0` (JS `this._flashes`).
#[must_use]
#[allow(clippy::too_many_arguments)] // the JS ctx + layer buffers all arrive
// as parameters (allocation-free per frame).
pub fn draw_ink(
    proj: &Projector,
    tables: &RasterTables,
    grid: &[u8],
    y0: usize,
    y1: usize,
    rgb: ([f32; 3], [f32; 3]),
    scratch: &mut InkScratch,
    ink: &mut [u8],
    flash: &mut [u8],
    owner: &mut [u8],
) -> bool {
    let (a_rgb, b_rgb) = rgb;
    let w = proj.w as usize;
    let h = proj.h as usize;
    let cell = &tables.pix_cell;
    let fx_a = &tables.pix_fx;
    let fy_a = &tables.pix_fy;
    let sx_a = &tables.pix_sx;
    let sy_a = &tables.pix_sy;
    let al = &mut scratch.al;
    let tm = &mut scratch.tm;
    let tt = &mut scratch.tt;
    let a0 = y0.saturating_sub(1) * w;
    let a1 = (y1 + 1).min(h) * w;
    for i in a0..a1 {
        let k = cell[i];
        if k < 0 {
            al[i] = 0.0;
            continue;
        }
        let k = k as usize;
        let sx = sx_a[i] as usize;
        let sy = sy_a[i] as usize;
        let g00 = grid[k];
        let g10 = grid[k + sx];
        let g01 = grid[k + sy];
        let g11 = grid[k + sx + sy];
        if g00 == g10 && g00 == g01 && g00 == g11 {
            // L305: uniform cell block — no edge here (fast path).
            al[i] = if g00 != 0 { 1.0 } else { 0.0 };
            tm[i] = g00;
            tt[i] = if g00 != 0 { g00 } else { 1 };
            continue;
        }
        let tx = fx_a[i] as f32 * (1.0 / 255.0);
        let ty = fy_a[i] as f32 * (1.0 / 255.0);
        let w00 = (1.0 - tx) * (1.0 - ty);
        let w10 = tx * (1.0 - ty);
        let w01 = (1.0 - tx) * ty;
        let w11 = tx * ty;
        let fa = (if g00 == 1 { w00 } else { 0.0 })
            + (if g10 == 1 { w10 } else { 0.0 })
            + (if g01 == 1 { w01 } else { 0.0 })
            + (if g11 == 1 { w11 } else { 0.0 });
        let fb = (if g00 == 2 { w00 } else { 0.0 })
            + (if g10 == 2 { w10 } else { 0.0 })
            + (if g01 == 2 { w01 } else { 0.0 })
            + (if g11 == 2 { w11 } else { 0.0 });
        let t: u8 = if fa >= fb { 1 } else { 2 };
        let f = if t == 1 { fa } else { fb };
        let a = smooth(0.34, 0.62, f);
        al[i] = a;
        tm[i] = if a > 0.5 { t } else { 0 };
        tt[i] = t;
    }
    let mut flashes = 0usize;
    for py in y0..y1 {
        for px in 0..w {
            let i = py * w + px;
            let o = i * 4;
            let a = al[i];
            if a <= 0.003 {
                ink[o + 3] = 0;
                flash[o + 3] = 0;
                owner[i] = 0;
                continue;
            }
            let c = if tt[i] == 2 { b_rgb } else { a_rgb };
            // L323-326: emboss — highlight on the lit (top-left) rim, soft
            // shade on the far rim → reads as thick glossy ink.
            let up = if px > 0 && py > 0 { al[i - w - 1] } else { a };
            let dn = if px < w - 1 && py < h - 1 {
                al[i + w + 1]
            } else {
                a
            };
            let lit = (a - up).max(0.0);
            let dark = (a - dn).max(0.0);
            let k = 0.94 - dark * 0.32;
            let add = lit * 150.0;
            ink[o] = clamp_u8(c[0] * k + add);
            ink[o + 1] = clamp_u8(c[1] * k + add);
            ink[o + 2] = clamp_u8(c[2] * k + add);
            ink[o + 3] = clamp_u8(a * 255.0);
            let now = tm[i];
            if now != 0 && owner[i] != now {
                flash[o] = 255;
                flash[o + 1] = 255;
                flash[o + 2] = 255;
                flash[o + 3] = 170;
                flashes += 1;
            } else {
                flash[o + 3] = 0;
            }
            owner[i] = now;
        }
    }
    flashes > 0
}

/// JS `_drawZones` path cache (L543-553): every zone's outline projected to
/// canvas px. zone → part → closed polyline of canvas points.
#[must_use]
pub fn zone_paths(proj: &Projector, defs: &[&ZoneDef]) -> Vec<Vec<Vec<[f32; 2]>>> {
    defs.iter()
        .map(|d| {
            d.parts()
                .iter()
                .map(|part| {
                    part.iter()
                        .map(|p| {
                            let (cx, cy) = proj.to_canvas(p[0], p[1]);
                            [cx, cy]
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// JS `_drawZones` (L542-576) rasterised into an RGBA overlay (D-T49-1:
/// thick capsules + scanline polygon fill, metric line widths × s kept).
/// `active[i]` marks the operational objective; `owners[i]` is JS `z.owner`
/// (-1 neutral). `out` is cleared first.
pub fn draw_zones(
    proj: &Projector,
    paths: &[Vec<Vec<[f32; 2]>>],
    active: &[bool],
    owners: &[i8],
    rgb: ([f32; 3], [f32; 3]),
    time: f32,
    out: &mut [u8],
) {
    let w = proj.w as usize;
    let h = proj.h as usize;
    let s = proj.s;
    out.iter_mut().for_each(|v| *v = 0);
    const DARK: [f32; 3] = [21.0, 18.0, 28.0]; // '#15121c'
    const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
    const GHOST: [f32; 3] = [189.0, 182.0, 204.0]; // '#bdb6cc'
    // L557-564: inactive zones — faint dashed ghosts (dash [1.5s, 1.1s]).
    for (i, parts) in paths.iter().enumerate() {
        if active.get(i).copied().unwrap_or(false) {
            continue;
        }
        for path in parts {
            stroke_poly(
                out,
                w,
                h,
                path,
                0.95 * s * 0.5,
                DARK,
                0.5,
                Some((1.5 * s, 1.1 * s, 0.0)),
            );
            stroke_poly(
                out,
                w,
                h,
                path,
                0.45 * s * 0.5,
                WHITE,
                0.85,
                Some((1.5 * s, 1.1 * s, 0.0)),
            );
        }
    }
    // L566-573: active zones in the holder's ink (white while neutral).
    let pulse = 0.5 + 0.5 * (time * 4.0).sin();
    for (i, parts) in paths.iter().enumerate() {
        if !active.get(i).copied().unwrap_or(false) {
            continue;
        }
        let owner = owners.get(i).copied().unwrap_or(-1);
        let held = owner == 0 || owner == 1;
        let col = if owner == 0 {
            rgb.0
        } else if owner == 1 {
            rgb.1
        } else {
            WHITE
        };
        for path in parts {
            fill_poly(
                out,
                w,
                h,
                path,
                col,
                (if held { 0.36 } else { 0.3 }) + 0.1 * pulse,
            );
        }
        for path in parts {
            stroke_poly(out, w, h, path, 1.15 * s * 0.5, DARK, 1.0, None);
            stroke_poly(out, w, h, path, 0.6 * s * 0.5, col, 1.0, None);
            if !held {
                stroke_poly(
                    out,
                    w,
                    h,
                    path,
                    0.6 * s * 0.5,
                    GHOST,
                    1.0,
                    Some((0.9 * s, 0.9 * s, -time * 2.0 * s)),
                );
            }
        }
    }
}

/// Source-over one pixel with float rgb (alpha 0..1).
fn blend_px(buf: &mut [u8], i: usize, c: [f32; 3], a: f32) {
    if a <= 0.0 || i >= buf.len() / 4 {
        return;
    }
    let o = i * 4;
    let da = buf[o + 3] as f32 / 255.0;
    let na = a + da * (1.0 - a);
    for k in 0..3 {
        buf[o + k] = clamp_u8((c[k] * a + buf[o + k] as f32 * da * (1.0 - a)) / na);
    }
    buf[o + 3] = clamp_u8(na * 255.0);
}

/// Round-join closed polyline stroke (JS `lineJoin: 'round'`), optionally
/// dashed (JS `setLineDash` + `lineDashOffset`). Caps are round (capsule);
/// zone outlines are closed paths, so caps only meet at the join disks.
#[allow(clippy::too_many_arguments)]
fn stroke_poly(
    buf: &mut [u8],
    w: usize,
    h: usize,
    path: &[[f32; 2]],
    half: f32,
    c: [f32; 3],
    alpha: f32,
    dash: Option<(f32, f32, f32)>,
) {
    if path.len() < 2 || half <= 0.0 || alpha <= 0.0 {
        return;
    }
    let n = path.len();
    let mut acc = 0.0f32; // cumulative distance at the edge start
    for i in 0..n {
        let p0 = path[i];
        let p1 = path[(i + 1) % n];
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len > 0.0 {
            match dash {
                None => stamp_capsule(buf, w, h, p0, p1, half, c, alpha),
                Some((on, off, offset)) => {
                    let period = on + off;
                    if period <= 0.0 {
                        stamp_capsule(buf, w, h, p0, p1, half, c, alpha);
                    } else {
                        let mut t = 0.0f32;
                        while t < len {
                            let phase = (acc + t + offset).rem_euclid(period);
                            if phase >= on {
                                // in the gap: advance to the next "on" boundary
                                t += (period - phase).min(len - t);
                                continue;
                            }
                            let run = (on - phase).min(len - t);
                            let q0 = [p0[0] + dx * (t / len), p0[1] + dy * (t / len)];
                            let q1 = [
                                p0[0] + dx * ((t + run) / len),
                                p0[1] + dy * ((t + run) / len),
                            ];
                            stamp_capsule(buf, w, h, q0, q1, half, c, alpha);
                            t += run;
                        }
                    }
                }
            }
        }
        acc += len;
        // round join at the vertex
        stamp_capsule(buf, w, h, p1, p1, half, c, alpha);
    }
}

/// Stamp a capsule (segment ± `half`) over the pixel grid.
#[allow(clippy::too_many_arguments)]
fn stamp_capsule(
    buf: &mut [u8],
    w: usize,
    h: usize,
    p0: [f32; 2],
    p1: [f32; 2],
    half: f32,
    c: [f32; 3],
    alpha: f32,
) {
    let min_x = (p0[0].min(p1[0]) - half).floor().max(0.0) as usize;
    let max_x = ((p0[0].max(p1[0]) + half).ceil() as i64).min(w as i64 - 1) as usize;
    let min_y = (p0[1].min(p1[1]) - half).floor().max(0.0) as usize;
    let max_y = ((p0[1].max(p1[1]) + half).ceil() as i64).min(h as i64 - 1) as usize;
    if min_x > max_x || min_y > max_y {
        return;
    }
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let l2 = dx * dx + dy * dy;
    let h2 = half * half;
    for py in min_y..=max_y {
        let cy = py as f32 + 0.5 - p0[1];
        for px in min_x..=max_x {
            let cx = px as f32 + 0.5 - p0[0];
            let t = if l2 > 0.0 {
                ((cx * dx + cy * dy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let qx = cx - dx * t;
            let qy = cy - dy * t;
            if qx * qx + qy * qy <= h2 {
                blend_px(buf, py * w + px, c, alpha);
            }
        }
    }
}

/// Even-odd scanline polygon fill (pixel centres).
fn fill_poly(buf: &mut [u8], w: usize, h: usize, poly: &[[f32; 2]], c: [f32; 3], alpha: f32) {
    if poly.len() < 3 || alpha <= 0.0 {
        return;
    }
    let n = poly.len();
    let mut xs: Vec<f32> = Vec::with_capacity(16);
    for py in 0..h {
        let yc = py as f32 + 0.5;
        xs.clear();
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            if (a[1] <= yc && b[1] > yc) || (b[1] <= yc && a[1] > yc) {
                let t = (yc - a[1]) / (b[1] - a[1]);
                xs.push(a[0] + (b[0] - a[0]) * t);
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(f32::total_cmp);
        let mut j = 0;
        while j + 1 < xs.len() {
            let x0 = js_round(xs[j] - 0.5).max(0.0) as usize;
            let x1 = js_round(xs[j + 1] - 0.5).min(w as f32).max(0.0) as usize;
            for px in x0..x1 {
                blend_px(buf, py * w + px, c, alpha);
            }
            j += 2;
        }
    }
}

/// JS `_compose` (L366-371): base → ink → flash (decaying while
/// `flash_t < FLASH_DECAY`), then the zone overlay when present.
pub fn compose(
    base: &[u8],
    ink: &[u8],
    flash: &[u8],
    flash_t: f32,
    zones: Option<&[u8]>,
    out: &mut [u8],
) {
    out.copy_from_slice(base);
    let n = out.len() / 4;
    for i in 0..n {
        let o = i * 4;
        let a = ink[o + 3] as f32 / 255.0;
        if a > 0.0 {
            for k in 0..3 {
                out[o + k] = clamp_u8(ink[o + k] as f32 * a + out[o + k] as f32 * (1.0 - a));
            }
        }
    }
    if flash_t < FLASH_DECAY {
        let ga = (1.0 - flash_t / FLASH_DECAY) * 0.85;
        for i in 0..n {
            let o = i * 4;
            let a = flash[o + 3] as f32 / 255.0 * ga;
            if a > 0.0 {
                for k in 0..3 {
                    out[o + k] = clamp_u8(flash[o + k] as f32 * a + out[o + k] as f32 * (1.0 - a));
                }
            }
        }
    }
    if let Some(z) = zones {
        for i in 0..n {
            let o = i * 4;
            let a = z[o + 3] as f32 / 255.0;
            if a > 0.0 {
                for k in 0..3 {
                    out[o + k] = clamp_u8(z[o + k] as f32 * a + out[o + k] as f32 * (1.0 - a));
                }
            }
        }
    }
}

/// JS `Minimap.update` state machine (L341-364) — refresh scheduling only;
/// the actual rasterising is driven by the returned jobs.
#[derive(Debug, Clone)]
pub struct MinimapClock {
    /// JS `this.time`.
    pub time: f32,
    /// JS `this.flashT`.
    pub flash_t: f32,
    timer: f32,
    /// JS `this.version`; `-1` = full refresh pending (first / invalidated).
    pub version: i64,
    band: usize,
    quiet: bool,
    team_key: String,
    theme: String,
}

impl Default for MinimapClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MinimapClock {
    /// JS constructor fields (L75-84): `version=-1, timer=0, time=0,
    /// flashT=9, _band=0`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            time: 0.0,
            flash_t: 9.0,
            timer: 0.0,
            version: -1,
            band: 0,
            quiet: false,
            team_key: String::new(),
            theme: String::new(),
        }
    }

    /// JS `setViewerTeam` (L90-97) / team-key edge (L346): force a full
    /// refresh. On a flip change the caller additionally rebuilds the tables
    /// + base and re-projects the zone paths.
    pub fn invalidate(&mut self) {
        self.version = -1;
    }

    /// JS `update(dt, force)` body minus the draws. `h` is the canvas height.
    #[must_use]
    pub fn begin(
        &mut self,
        dt: f32,
        paint_version: u32,
        h: usize,
        team_key: &str,
        theme: &str,
        force: bool,
    ) -> ClockTick {
        let mut out = ClockTick::default();
        self.time += dt;
        self.timer -= dt;
        if team_key != self.team_key {
            self.team_key = team_key.to_string();
            self.version = -1;
        }
        if theme != self.theme {
            let had = !self.theme.is_empty();
            self.theme = theme.to_string();
            out.redraw_base = had;
        }
        if self.band > 0 {
            let b = self.band;
            out.ink = Some(InkJob {
                y0: (h * b) / INK_BANDS,
                y1: (h * (b + 1)) / INK_BANDS,
                quiet: self.quiet,
                full: false,
                first: false,
            });
            self.band = if b + 1 >= INK_BANDS { 0 } else { b + 1 };
            if self.band == 0 {
                self.quiet = false;
            }
        } else if force || (self.timer <= 0.0 && self.version != i64::from(paint_version)) {
            self.timer = REFRESH_INTERVAL;
            let first = self.version == -1;
            self.version = i64::from(paint_version);
            if first || force {
                self.quiet = first;
                out.ink = Some(InkJob {
                    y0: 0,
                    y1: h,
                    quiet: self.quiet,
                    full: true,
                    first,
                });
                self.quiet = false;
                if first {
                    self.flash_t = 9.0;
                }
            } else {
                out.ink = Some(InkJob {
                    y0: 0,
                    y1: h / INK_BANDS,
                    quiet: self.quiet,
                    full: false,
                    first: false,
                });
                self.band = 1;
            }
        }
        out
    }

    /// JS L337 + L362: apply the draw's flash result, then advance `flashT`.
    pub fn end(&mut self, dt: f32, job: Option<&InkJob>, flashes: bool) {
        if let Some(j) = job
            && flashes
            && !j.quiet
        {
            self.flash_t = 0.0;
        }
        self.flash_t += dt;
    }
}

/// One `_drawInk` call requested by [`MinimapClock::begin`].
#[derive(Debug, Clone, Copy)]
pub struct InkJob {
    /// first row (inclusive).
    pub y0: usize,
    /// last row (exclusive).
    pub y1: usize,
    /// JS `this._quiet` during the draw: suppresses the flash reset.
    pub quiet: bool,
    /// full-map draw (first / force). The app consumes the row range; these
    /// two flags are the state-machine surface the unit tests assert.
    #[allow(dead_code)]
    pub full: bool,
    /// the very first refresh after an invalidate (JS `first`).
    #[allow(dead_code)]
    pub first: bool,
}

/// JS `update` return of work for one frame.
#[derive(Debug, Default)]
pub struct ClockTick {
    /// JS L348: theme edge → `_drawBase()`.
    pub redraw_base: bool,
    /// the ink rows to refresh this frame, if any.
    pub ink: Option<InkJob>,
}

/// One minimap player marker (JS `main.js` L1293-1304 `players.push`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerDot {
    /// normalised x (`t.x / w`, 0..1, may leave the box).
    pub x: f32,
    /// normalised y (`t.y / h`).
    pub y: f32,
    pub team: usize,
    /// JS `isSelf: o.isLocal`.
    pub is_self: bool,
    /// JS `yaw: -o.yaw + (flip ? π : 0)` — canvas-space heading.
    pub yaw: f32,
    pub alive: bool,
    /// JS `tracked`: enemy revealed by our team's echo-orb.
    pub tracked: bool,
}

/// JS `main.js` L1293-1304: build the minimap dot list from the actors.
/// `local_slot` ≡ `o.isLocal` (DemoSim's player slot); `viewer_team` ≡ the
/// local actor's team (`setViewerTeam` timing, spec 批次 C).
#[must_use]
pub fn player_dots(
    actors: &[Actor],
    viewer_team: usize,
    local_slot: usize,
    proj: &Projector,
) -> Vec<PlayerDot> {
    let mut out = Vec::new();
    for o in actors {
        if !o.alive {
            continue;
        }
        let is_local = o.slot == local_slot;
        // L1297: tracked = enemy && our team's echo-orb mark on them.
        let tracked = o.team != viewer_team
            && o.status.track > 0.0
            && o.status.track_team == Some(viewer_team);
        if o.team != viewer_team && !is_local {
            // L1300: submerged squids stay hidden unless tracked. JS
            // `anim.form === 'swim'` ≡ squid + submerged (actor.js form
            // getter, L993) *after* the special override (L994):
            // `if (specialActive && (body || noSquid)) form = 'kid'`, so a
            // body/noSquid special (sonar / jetpack / strike / zipcaster)
            // keeps the squid visible on the map. The `climbing → 'climb'`
            // branch of the JS getter is exempt: wall climb is out of sim
            // scope (`actor.rs` L1225), so `form` can never be climb here.
            let special_kid = o
                .special_active
                .as_ref()
                .is_some_and(|s| s.body || s.no_squid);
            let swim = o.form == Form::Squid && o.submerged && !special_kid;
            if swim && !tracked {
                continue;
            }
        }
        let (cx, cy) = proj.to_canvas(o.pos.x, o.pos.z);
        out.push(PlayerDot {
            x: cx / proj.w as f32,
            y: cy / proj.h as f32,
            team: o.team,
            is_self: is_local,
            yaw: -o.yaw + if proj.flip { std::f32::consts::PI } else { 0.0 },
            alive: o.alive,
            tracked,
        });
    }
    out
}

/// Rust extension (JS has none, spec 批次 B): the boss marker's normalised
/// map position.
#[must_use]
pub fn boss_dot(x: f32, z: f32, proj: &Projector) -> (f32, f32) {
    let (cx, cy) = proj.to_canvas(x, z);
    (cx / proj.w as f32, cy / proj.h as f32)
}

/// JS `hud.js` `_updMap` spring (L1724-1727): k=190, c=21, dt clamped to
/// 0.05, snap-settle under 0.001.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapSpring {
    /// JS `this._mapT`.
    pub t: f32,
    /// JS `this._mapV`.
    pub v: f32,
}

impl MapSpring {
    pub fn step(&mut self, target: f32, dt: f32) {
        let k = 190.0;
        let c = 21.0;
        let hstep = dt.min(0.05);
        let a = k * (target - self.t) - c * self.v;
        self.v += a * hstep;
        self.t += self.v * hstep;
        if (target - self.t).abs() < 0.001 && self.v.abs() < 0.001 {
            self.t = target;
            self.v = 0.0;
        }
    }
}

/// JS `hud.js` `_updMap` layout (L1728-1735): the corner ↔ expanded box,
/// `aspect` = canvas w/h. Returns `(x, y, w, h)` in window px.
#[must_use]
pub fn map_layout(win_w: f32, win_h: f32, aspect: f32, t: f32) -> (f32, f32, f32, f32) {
    let lerp = |a: f32, b: f32, x: f32| a + (b - a) * x;
    let u = (win_w / 100.0).min(win_h * 1.7778 / 100.0);
    let fit = |sz: f32| {
        if aspect >= 1.0 {
            (sz, sz / aspect)
        } else {
            (sz * aspect, sz)
        }
    };
    let (w0, h0) = fit(14.5 * u);
    let (w1, h1) = fit((win_h * 0.78).min(win_w * 0.6));
    let bw = lerp(w0, w1, t);
    let bh = lerp(h0, h1, t);
    let x = lerp(2.2 * u, (win_w - w1) / 2.0, t);
    let y = lerp(win_h - 2.2 * u - h0, (win_h - h1) / 2.0 + u * 1.2, t);
    (x, y, bw, bh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        Bounds {
            min_x: -28.0,
            max_x: 28.0,
            min_z: -46.5,
            max_z: 46.5,
        }
    }

    // ---------------------------------------------------------------- helpers

    #[test]
    fn lin2s_known_values() {
        // JS L56 anchors: 0 → 0, 1 → 255, the linear threshold → round(10.32)=10.
        assert_eq!(lin2s(0.0), 0.0);
        assert_eq!(lin2s(1.0), 255.0);
        assert_eq!(lin2s(0.003_130_8), 10.0);
        // 0.5 → 255*(1.055*0.5^(1/2.4)-0.055) = 187.516 → Math.round = 188.
        assert_eq!(lin2s(0.5), 188.0);
    }

    #[test]
    fn smooth_matches_js() {
        assert_eq!(smooth(0.34, 0.62, 0.34), 0.0);
        assert_eq!(smooth(0.34, 0.62, 0.62), 1.0);
        assert_eq!(smooth(0.34, 0.62, 0.0), 0.0); // clamped
        assert_eq!(smooth(0.34, 0.62, 1.0), 1.0); // clamped
        assert!((smooth(0.34, 0.62, 0.48) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn clamp_u8_matches_uint8_clamped() {
        assert_eq!(clamp_u8(-5.0), 0);
        assert_eq!(clamp_u8(f32::NAN), 0);
        assert_eq!(clamp_u8(300.0), 255);
        // round-half-to-even (JS Uint8ClampedArray):
        assert_eq!(clamp_u8(0.5), 0);
        assert_eq!(clamp_u8(1.5), 2);
        assert_eq!(clamp_u8(2.5), 2);
        assert_eq!(clamp_u8(168.552), 169);
    }

    #[test]
    fn hex_rgb_parses_like_world_hex_color() {
        assert_eq!(hex_rgb("#ff8a14"), [255.0, 138.0, 20.0]);
        assert_eq!(hex_rgb("2f5bff"), [47.0, 91.0, 255.0]);
        assert_eq!(hex_rgb("#abc"), [170.0, 187.0, 204.0]);
        assert_eq!(hex_rgb("nope"), [221.0, 221.0, 221.0]);
    }

    // ---------------------------------------------------------------- projector

    #[test]
    fn projector_matches_js_canvas_math() {
        let p = Projector::new(&bounds(), MinimapTier::Med.px_per_m(), false);
        assert_eq!(p.w, 392); // round(56 * 7)
        assert_eq!(p.h, 651); // round(93 * 7)
        assert_eq!(p.to_canvas(28.0, 46.5), (0.0, 0.0));
        let (cx, cy) = p.to_canvas(-28.0, -46.5);
        assert!((cx - 392.0).abs() < 1e-3 && (cy - 651.0).abs() < 1e-3);
        // round-trip
        assert!((p.world_x(100.0) - (28.0 - 100.0 / 7.0)).abs() < 1e-6);
        assert!((p.world_z(70.0) - (46.5 - 70.0 / 7.0)).abs() < 1e-6);
    }

    #[test]
    fn projector_flip_mirrors_both_axes() {
        let f = Projector::new(&bounds(), 7.0, true);
        let (cx, cy) = f.to_canvas(28.0, 46.5);
        assert!((cx - 392.0).abs() < 1e-3 && (cy - 651.0).abs() < 1e-3);
        // flip round-trips too
        assert!((f.world_x(cx) - 28.0).abs() < 1e-3);
        assert!((f.world_z(cy) - 46.5).abs() < 1e-3);
    }

    // ---------------------------------------------------------------- tables

    fn synth_tables(w: usize, h: usize) -> RasterTables {
        RasterTables {
            hgt: vec![0.0; w * h],
            top: vec![0; w * h],
            nrm: vec![0.0; w * h * 2],
            pix_cell: vec![-1; w * h],
            pix_fx: vec![0; w * h],
            pix_fy: vec![0; w * h],
            pix_sx: vec![0; w * h],
            pix_sy: vec![0; w * h],
        }
    }

    fn synth_proj(w: u32, h: u32) -> Projector {
        Projector {
            s: 7.0,
            w,
            h,
            flip: false,
            max_x: w as f32 / 7.0,
            max_z: h as f32 / 7.0,
        }
    }

    #[test]
    fn raster_tables_build_is_deterministic_on_tidewater() {
        let world = CollisionWorld::tidewater();
        let layout = inkwave_sim::embedded_tidewater();
        let paint = PaintGrid::new(&world);
        let proj = Projector::new(&world.bounds, MinimapTier::Med.px_per_m(), false);
        let (_, perch) = layout_styles(&layout.primitives, &world.blocks);
        let a = RasterTables::build(&proj, &world, &paint, &perch);
        let b = RasterTables::build(&proj, &world, &paint, &perch);
        assert_eq!(a.hgt, b.hgt);
        assert_eq!(a.top, b.top);
        assert_eq!(a.nrm, b.nrm);
        assert_eq!(a.pix_cell, b.pix_cell);
        assert_eq!(a.pix_fx, b.pix_fx);
        assert_eq!(a.pix_sy, b.pix_sy);
        // some turf pixels exist, and the deck is covered somewhere
        assert!(a.pix_cell.iter().any(|&k| k >= 0));
        assert!(a.top.iter().any(|&t| t >= 0));
    }

    #[test]
    fn draw_base_full_deck_uniform_block_matches_js_formula() {
        // TR-49.1: single block covering everything, h=0, no normal →
        // lum 0.9, colour lifted to paper.
        let (w, h) = (36usize, 36usize);
        let proj = synth_proj(w as u32, h as u32);
        let tables = synth_tables(w, h);
        let styles = vec![BlockStyle {
            color: [128.0, 128.0, 128.0],
            grate: false,
            hidden: false,
        }];
        let d = draw_base(&proj, &tables, &styles, false);
        // interior pixel (2,2): c=128+114*0.52=187.28; *0.9=168.552→169.
        let o = (2 * w + 2) * 4;
        assert_eq!((d[o], d[o + 1], d[o + 2], d[o + 3]), (169, 166, 159, 255));
        // grid overlay: step 35, w%35=1 → column 1, row 1; (1,1) blends once
        // (single JS stroke op). Hand-computed constants along the JS canvas
        // path: the stroke reads the ALREADY-ROUNDED deck bytes (169,166,159),
        // not the float pre-values — R: 27*0.07+169*0.93=159.06→159;
        // G: 24*0.07+166*0.93=156.06→156; B: 48*0.07+159*0.93=151.23→151.
        let o = (w + 1) * 4;
        assert_eq!(d[o], 159);
        assert_eq!(d[o + 1], 156);
        assert_eq!(d[o + 2], 151);
        // (3,3) is off the grid lines (column 1 / row 1 carry the 35 px step)
        let o = (3 * w + 3) * 4;
        assert_eq!(d[o], 169);
    }

    #[test]
    fn draw_base_sea_and_sunset_palette() {
        let (w, h) = (8usize, 8usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        tables.top.iter_mut().for_each(|v| *v = -1);
        let styles = vec![BlockStyle {
            color: [0.0, 0.0, 0.0],
            grate: false,
            hidden: false,
        }];
        let day = draw_base(&proj, &tables, &styles, false);
        let dusk = draw_base(&proj, &tables, &styles, true);
        // sea stays inside the day gradient endpoints (near = 0 everywhere)
        for i in 0..w * h {
            let o = i * 4;
            assert_eq!(day[o + 3], 255);
            assert!((36.0..=62.0).contains(&(day[o] as f32)), "r {}", day[o]);
            assert!((104.0..=146.0).contains(&(day[o + 1] as f32)));
            assert!((170.0..=206.0).contains(&(day[o + 2] as f32)));
            assert!((70.0..=104.0).contains(&(dusk[o] as f32)));
        }
    }

    #[test]
    fn draw_base_grate_hatch_alternates() {
        let (w, h) = (8usize, 8usize);
        let proj = synth_proj(w as u32, h as u32);
        let tables = synth_tables(w, h);
        let styles = vec![BlockStyle {
            color: [200.0, 200.0, 200.0],
            grate: true,
            hidden: false,
        }];
        let d = draw_base(&proj, &tables, &styles, false);
        // grate colour [150,162,176] * 0.9; hatch pixels * 0.62/0.64/0.7
        let plain = clamp_u8(150.0 * 0.9);
        let hatched = clamp_u8(150.0 * 0.9 * 0.62);
        let o0 = 0; // (0+0)%4=0 <1.5 → hatch
        let o1 = 2 * 4; // 2%4=2, (0-2+4000)%4=2 → plain
        assert_eq!(d[o0], hatched);
        assert_eq!(d[o1], plain);
    }

    // ---------------------------------------------------------------- ink

    #[test]
    fn draw_ink_all_zero_and_all_one() {
        let (w, h) = (4usize, 4usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        for i in 0..w * h {
            tables.pix_cell[i] = 0; // single-cell grid index 0
        }
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);

        let grid0 = vec![0u8; 1];
        assert!(!draw_ink(
            &proj,
            &tables,
            &grid0,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        for i in 0..w * h {
            assert_eq!(ink[i * 4 + 3], 0);
        }

        let grid1 = vec![1u8; 1];
        // first pass with owner memory zeroed → every pixel flashes (team edge)
        assert!(draw_ink(
            &proj,
            &tables,
            &grid1,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        for i in 0..w * h {
            // fast path: al=1, interior emboss 0 → k=0.94, A=(255,138,20)
            assert_eq!(ink[i * 4], 240);
            assert_eq!(ink[i * 4 + 1], 130);
            assert_eq!(ink[i * 4 + 2], 19);
            assert_eq!(ink[i * 4 + 3], 255);
            assert_eq!(flash[i * 4 + 3], 170);
            assert_eq!(owner[i], 1);
        }
        // second pass: no owner change → no flash
        assert!(!draw_ink(
            &proj,
            &tables,
            &grid1,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        assert_eq!(flash[3], 0);
    }

    #[test]
    fn draw_ink_fast_path_team2_and_dead_cell() {
        let (w, h) = (2usize, 1usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        tables.pix_cell[0] = 0;
        tables.pix_cell[1] = -1; // dead / no turf
        let grid = vec![2u8; 1];
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);
        assert!(draw_ink(
            &proj,
            &tables,
            &grid,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        // team 2 at (0,0): the corner diagonal guards fall back to `a` →
        // lit=dark=0 → k=0.94, add=lit*150=0 (JS L323-326).
        assert_eq!(ink[0], clamp_u8(47.0 * 0.94));
        assert_eq!(ink[1], clamp_u8(91.0 * 0.94));
        assert_eq!(ink[2], clamp_u8(255.0 * 0.94));
        assert_eq!(ink[3], 255);
        // dead cell → transparent
        assert_eq!(ink[7], 0);
    }

    #[test]
    fn draw_ink_bilinear_edge_values() {
        // TR-49.1 覆盖率→像素映射: paint grid [1,0,…], neighbour step wired,
        // fractions pinned.
        let (w, h) = (3usize, 1usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        for i in 0..w * h {
            tables.pix_cell[i] = 0;
            tables.pix_sx[i] = 1; // g10 = grid[1]
            tables.pix_sy[i] = 2; // g01 = grid[2]
        }
        let grid = vec![1u8, 0, 0, 0];
        // pixel 0: fx=0, fy=0 → w00=1 → fa=1 → smooth→1
        // pixel 1: fx=255 → tx=1 → w10=1 → g10=0 → fa=0 → a=0
        // pixel 2: fx=128 → tx≈0.502 → fa=w00≈0.498 → mid smooth
        tables.pix_fx[1] = 255;
        tables.pix_fx[2] = 128;
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);
        assert!(draw_ink(
            &proj,
            &tables,
            &grid,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        assert_eq!(scratch.al[0], 1.0);
        assert_eq!(scratch.al[1], 0.0);
        let expect2 = smooth(0.34, 0.62, 1.0 - 128.0 / 255.0);
        assert!((scratch.al[2] - expect2).abs() < 1e-6);
        assert_eq!(scratch.tm[0], 1);
        assert_eq!(scratch.tt[2], 1);
        assert_eq!(ink[3], 255);
        assert_eq!(ink[4 + 3], 0);
        assert_eq!(ink[4 * 2 + 3], clamp_u8(expect2 * 255.0));
    }

    #[test]
    fn draw_ink_emboss_lit_rim() {
        // 3×3 coverage ramp 0→1 toward the bottom-right: pixels on the rising
        // diagonal edge get `lit` (add = 150).
        let (w, h) = (3usize, 3usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        // al = [0,0,0; 0,1,1; 1,1,1] → cells set on i ∈ {4,5,6,7,8}
        for i in 0..w * h {
            tables.pix_cell[i] = if (4..9).contains(&i) { 0 } else { -1 };
        }
        let grid = vec![1u8; 1];
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);
        assert!(draw_ink(
            &proj,
            &tables,
            &grid,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        // pixel 4 (1,1): up = al[2] = 0 → lit 1, dn = al[6] = 1 → dark 0
        let o = 4 * 4;
        assert_eq!(ink[o], 255);
        assert_eq!(ink[o + 1], 255);
        assert_eq!(ink[o + 2], clamp_u8(20.0 * 0.94 + 150.0));
        assert_eq!(ink[o + 3], 255);
        // pixel 8 (2,2): up = al[6] = 1 → lit 0; dn guard → k = 0.94
        let o = 8 * 4;
        assert_eq!((ink[o], ink[o + 1], ink[o + 2]), (240, 130, 19));
        // pixel 0: no coverage
        assert_eq!(ink[3], 0);
    }

    #[test]
    fn draw_ink_emboss_dark_rim() {
        // al = [1,1,1; 1,1,0; 1,0,0]: pixel 4 sees dn = al[8] = 0 → dark 1.
        let (w, h) = (3usize, 3usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        for i in 0..w * h {
            tables.pix_cell[i] = 0;
        }
        tables.pix_cell[5] = -1;
        tables.pix_cell[7] = -1;
        tables.pix_cell[8] = -1;
        let grid = vec![1u8; 1];
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);
        assert!(draw_ink(
            &proj,
            &tables,
            &grid,
            0,
            h,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        let o = 4 * 4;
        assert_eq!(ink[o], clamp_u8(255.0 * 0.62));
        assert_eq!(ink[o + 1], clamp_u8(138.0 * 0.62));
        assert_eq!(ink[o + 2], clamp_u8(20.0 * 0.62));
        assert_eq!(ink[o + 3], 255);
    }

    #[test]
    fn draw_ink_band_rows_only_touch_band() {
        let (w, h) = (2usize, 4usize);
        let proj = synth_proj(w as u32, h as u32);
        let mut tables = synth_tables(w, h);
        tables.pix_cell.iter_mut().for_each(|v| *v = 0);
        let grid = vec![1u8; 1];
        let mut ink = vec![0u8; w * h * 4];
        let mut flash = vec![0u8; w * h * 4];
        let mut owner = vec![0u8; w * h];
        let mut scratch = InkScratch::with_size(w * h);
        // band [1,3): rows 0 and 3 keep their (empty) content
        assert!(draw_ink(
            &proj,
            &tables,
            &grid,
            1,
            3,
            TEAM_RGB,
            &mut scratch,
            &mut ink,
            &mut flash,
            &mut owner
        ));
        assert_eq!(ink[3], 0);
        assert_eq!(ink[(2 * w) * 4 + 3], 255);
        assert_eq!(ink[(3 * w) * 4 + 3], 0);
    }

    // ---------------------------------------------------------------- zones

    fn square_def() -> ZoneDef {
        // world square [3,7]² → canvas [21,49]² on the 70px synth map
        ZoneDef::new(
            vec![vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]]],
            None,
            None,
        )
    }

    #[test]
    fn zone_paths_project_corners() {
        let proj = synth_proj(70, 70); // max_x = 10, max_z = 10
        let def = square_def();
        let paths = zone_paths(&proj, &[&def]);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].len(), 1);
        assert_eq!(paths[0][0].len(), 4);
        // world (3,3) → ((10-3)*7, (10-3)*7) = (49,49)
        assert_eq!(paths[0][0][0], [49.0, 49.0]);
        // world (7,7) → (21,21)
        assert_eq!(paths[0][0][2], [21.0, 21.0]);
    }

    #[test]
    fn draw_zones_active_fill_and_outline() {
        let (w, h) = (60usize, 60usize);
        let proj = synth_proj(w as u32, h as u32);
        let def = square_def();
        let paths = zone_paths(&proj, &[&def]);
        let mut out = vec![0u8; w * h * 4];
        draw_zones(&proj, &paths, &[true], &[0], TEAM_RGB, 0.0, &mut out);
        // centre pixel (30,30) inside the zone → team A fill present
        let o = (30 * w + 30) * 4;
        assert!(out[o + 3] > 0);
        assert_eq!(out[o], 255);
        // far corner outside the zone stays clear
        let o = (2 * w + 2) * 4;
        assert_eq!(out[o + 3], 0);
    }

    #[test]
    fn draw_zones_inactive_ghost_is_dashed_and_unfilled() {
        let (w, h) = (60usize, 60usize);
        let proj = synth_proj(w as u32, h as u32);
        let def = square_def();
        let paths = zone_paths(&proj, &[&def]);
        let mut out = vec![0u8; w * h * 4];
        draw_zones(&proj, &paths, &[false], &[-1], TEAM_RGB, 0.0, &mut out);
        // stroke pixels exist along the outline, interior stays clear
        let painted: usize = out.chunks(4).filter(|p| p[3] > 0).count();
        assert!(painted > 0);
        let interior = (30 * w + 30) * 4;
        assert_eq!(out[interior + 3], 0);
    }

    // ---------------------------------------------------------------- compose

    #[test]
    fn compose_flash_decay_window() {
        let n = 4;
        let base = vec![10u8; n * 4];
        let ink = vec![0u8; n * 4];
        let mut flash = vec![0u8; n * 4];
        flash[0] = 255;
        flash[1] = 255;
        flash[2] = 255;
        flash[3] = 255;
        let mut out = vec![0u8; n * 4];
        // flash_t >= 0.45 → no flash blend
        compose(&base, &ink, &flash, 0.45, None, &mut out);
        assert_eq!(out[0], 10);
        // flash_t = 0 → ga 0.85, white over 10 → 10*0.15 + 255*0.85 ≈ 218
        compose(&base, &ink, &flash, 0.0, None, &mut out);
        assert!(out[0] > 200);
        // ink over base
        let mut ink2 = vec![0u8; n * 4];
        ink2[0] = 255;
        ink2[3] = 255;
        compose(&base, &ink2, &flash, 9.0, None, &mut out);
        assert_eq!(out[0], 255);
    }

    // ---------------------------------------------------------------- clock

    #[test]
    fn clock_first_refresh_then_bands() {
        let mut c = MinimapClock::new();
        let h = 600usize;
        // tick 1: version -1 → full draw, quiet, flashT=9
        let t1 = c.begin(0.016, 7, h, "AB", "day", false);
        let job = t1.ink.expect("full ink");
        assert!(job.full && job.first && job.quiet);
        assert_eq!((job.y0, job.y1), (0, 600));
        c.end(0.016, Some(&job), true); // flashes ignored while quiet
        assert!(c.flash_t > 9.0); // 9 + dt, not reset
        assert_eq!(c.version, 7);

        // timer expiry with a changed paint version → band 0, band=1
        for _ in 0..10 {
            let t = c.begin(0.016, 7, h, "AB", "day", false);
            assert!(t.ink.is_none());
            c.end(0.016, t.ink.as_ref(), false);
        }
        let t = c.begin(0.016, 8, h, "AB", "day", false);
        let job = t.ink.expect("band 0");
        assert_eq!((job.y0, job.y1), (0, 200));
        c.end(0.016, Some(&job), true);
        assert!((c.flash_t - 0.016).abs() < 1e-6); // flash reset while not quiet

        // band continuation runs every frame (not gated by the timer)
        let t = c.begin(0.016, 8, h, "AB", "day", false);
        let job = t.ink.expect("band 1");
        assert_eq!((job.y0, job.y1), (200, 400));
        c.end(0.016, t.ink.as_ref(), false);
        let t = c.begin(0.016, 8, h, "AB", "day", false);
        let job = t.ink.expect("band 2");
        assert_eq!((job.y0, job.y1), (400, 600));
        c.end(0.016, t.ink.as_ref(), false);
        let t = c.begin(0.016, 8, h, "AB", "day", false);
        assert!(t.ink.is_none());
    }

    #[test]
    fn clock_edges() {
        let mut c = MinimapClock::new();
        // theme edge: first assignment is not a redraw (JS `had` guard)
        let t = c.begin(0.016, 1, 60, "AB", "day", false);
        assert!(!t.redraw_base);
        c.end(0.016, t.ink.as_ref(), false);
        let t = c.begin(0.016, 1, 60, "AB", "sunset", false);
        assert!(t.redraw_base);
        assert!(t.ink.is_none());
        // team-key edge → version=-1, but the 0.15 s timer still gates the
        // refresh (JS L344-346 + L352): the full draw lands on the next expiry
        let t = c.begin(0.016, 1, 60, "CD", "sunset", false);
        assert!(t.ink.is_none());
        c.end(0.016, t.ink.as_ref(), false);
        let mut job = None;
        for _ in 0..12 {
            let t = c.begin(0.016, 1, 60, "CD", "sunset", false);
            if let Some(j) = t.ink {
                job = Some(j);
                c.end(0.016, Some(&j), false);
                break;
            }
            c.end(0.016, None, false);
        }
        let job = job.expect("full after team change");
        assert!(job.full && job.first && job.quiet);
        // force → full, not quiet (JS `_quiet = first` = false)
        let t = c.begin(0.016, 1, 60, "CD", "sunset", true);
        let job = t.ink.expect("forced ink");
        assert!(job.full && !job.first && !job.quiet);
        c.end(0.016, Some(&job), true);
        assert!((c.flash_t - 0.016).abs() < 1e-6);
    }

    // ---------------------------------------------------------------- dots

    fn mk_actor(team: usize, slot: usize) -> Actor {
        let t = inkwave_sim::embedded_tuning().player;
        Actor::new(team, slot, &t)
    }

    #[test]
    fn player_dots_filter_and_transform() {
        let proj = Projector::new(&bounds(), 7.0, false);
        let mut me = mk_actor(0, 0);
        me.pos = glam::Vec3::new(0.0, 0.5, 0.0);
        me.yaw = 0.5;
        let mut ally = mk_actor(0, 1);
        ally.pos = glam::Vec3::new(7.0, 0.5, 7.0);
        let mut enemy_swim = mk_actor(1, 2);
        enemy_swim.form = Form::Squid;
        enemy_swim.submerged = true;
        let mut enemy_dead = mk_actor(1, 3);
        enemy_dead.alive = false;
        let mut enemy_tracked = mk_actor(1, 4);
        enemy_tracked.form = Form::Squid;
        enemy_tracked.submerged = true;
        enemy_tracked.status.track = 4.0;
        enemy_tracked.status.track_team = Some(0);
        let mut enemy_wrong_tracker = mk_actor(1, 5);
        enemy_wrong_tracker.form = Form::Squid;
        enemy_wrong_tracker.submerged = true;
        enemy_wrong_tracker.status.track = 4.0;
        enemy_wrong_tracker.status.track_team = Some(1);
        let actors = [
            me,
            ally,
            enemy_swim,
            enemy_dead,
            enemy_tracked,
            enemy_wrong_tracker,
        ];
        let dots = player_dots(&actors, 0, 0, &proj);
        // alive filter, swim-hide (tracked survives), normalisation, yaw
        assert_eq!(dots.len(), 3);
        let self_dot = &dots[0];
        assert!(self_dot.is_self);
        assert_eq!(self_dot.team, 0);
        assert!((self_dot.x - 0.5).abs() < 1e-3);
        assert!((self_dot.y - 0.5).abs() < 1e-3);
        assert!((self_dot.yaw + 0.5).abs() < 1e-6);
        assert!(dots.iter().any(|d| d.tracked));
        assert!(!dots.iter().any(|d| d.team == 1 && !d.tracked && !d.is_self));
    }

    #[test]
    fn player_dots_special_override_keeps_swimmer_visible() {
        // JS actor.js L993-994: the form getter's `swim` is overwritten with
        // `kid` while a special with `body || noSquid` runs, so main.js L1300
        // does NOT hide that enemy squid. Lock the override semantics.
        let proj = Projector::new(&bounds(), 7.0, false);
        let mut enemy_body = mk_actor(1, 2);
        enemy_body.form = Form::Squid;
        enemy_body.submerged = true;
        let mut s = inkwave_sim::actor::SpecialActive::new("sonar", "sonar", 4.0);
        s.body = true;
        enemy_body.special_active = Some(Box::new(s));
        let mut enemy_no_squid = mk_actor(1, 3);
        enemy_no_squid.form = Form::Squid;
        enemy_no_squid.submerged = true;
        let mut n = inkwave_sim::actor::SpecialActive::new("inkling", "inkling", 4.0);
        n.no_squid = true;
        enemy_no_squid.special_active = Some(Box::new(n));
        // plain body-less special (e.g. a run-only marker) does not override.
        let mut enemy_plain = mk_actor(1, 4);
        enemy_plain.form = Form::Squid;
        enemy_plain.submerged = true;
        enemy_plain.special_active = Some(Box::new(inkwave_sim::actor::SpecialActive::new(
            "squidstorm",
            "squidstorm",
            4.0,
        )));
        let actors = [enemy_body, enemy_no_squid, enemy_plain];
        let dots = player_dots(&actors, 0, 0, &proj);
        assert_eq!(dots.len(), 2);
        assert_eq!(dots[0].team, 1);
        assert_eq!(dots[1].team, 1);
    }

    #[test]
    fn player_dots_flip_mirrors_and_adds_pi() {
        let proj = Projector::new(&bounds(), 7.0, true);
        let mut me = mk_actor(1, 4);
        me.pos = glam::Vec3::new(0.0, 0.5, 0.0);
        me.yaw = 1.0;
        let dots = player_dots(std::slice::from_ref(&me), 1, 4, &proj);
        assert_eq!(dots.len(), 1);
        // centre maps to centre under flip too
        assert!((dots[0].x - 0.5).abs() < 1e-3);
        // yaw = -o.yaw + π
        assert!((dots[0].yaw - (-1.0 + std::f32::consts::PI)).abs() < 1e-6);
    }

    #[test]
    fn boss_dot_normalises() {
        let proj = Projector::new(&bounds(), 7.0, false);
        let (x, y) = boss_dot(28.0, 46.5, &proj);
        assert_eq!((x, y), (0.0, 0.0));
    }

    // ---------------------------------------------------------------- layout

    #[test]
    fn map_layout_corner_and_expanded() {
        // 1600×900 window, 392×651 canvas (aspect 0.602)
        let asp = 392.0 / 651.0;
        let (x, y, bw, bh) = map_layout(1600.0, 900.0, asp, 0.0);
        let u = (1600.0f32 / 100.0).min(900.0 * 1.7778 / 100.0);
        let (w0, h0) = (14.5 * u * asp, 14.5 * u);
        assert!((bw - w0).abs() < 1e-3);
        assert!((bh - h0).abs() < 1e-3);
        assert!((x - 2.2 * u).abs() < 1e-3);
        assert!((y - (900.0 - 2.2 * u - h0)).abs() < 1e-3);
        // expanded: fit(min(H*0.78, W*0.6)) centred-ish
        let (x1, y1, bw1, bh1) = map_layout(1600.0, 900.0, asp, 1.0);
        let sz = (900.0f32 * 0.78).min(1600.0 * 0.6);
        assert!((bw1 - sz * asp).abs() < 1e-3);
        assert!((x1 - (1600.0 - sz * asp) / 2.0).abs() < 1e-3);
        assert!((y1 - ((900.0 - sz) / 2.0 + u * 1.2)).abs() < 1e-3);
        assert!((bh1 - sz).abs() < 1e-3);
    }

    #[test]
    fn spring_settles_to_target() {
        let mut s = MapSpring::default();
        for _ in 0..240 {
            s.step(1.0, 1.0 / 60.0);
        }
        assert_eq!(s.t, 1.0);
        assert_eq!(s.v, 0.0);
        // dt clamp matches JS Math.min(dt, 0.05)
        let mut s2 = MapSpring::default();
        s2.step(1.0, 10.0);
        assert!(s2.t < 1.0 && s2.t > 0.0);
    }

    // ---------------------------------------------------------------- determinism

    #[test]
    fn draw_base_and_ink_are_bytewise_deterministic() {
        let world = CollisionWorld::tidewater();
        let layout = inkwave_sim::embedded_tidewater();
        let paint = PaintGrid::new(&world);
        let proj = Projector::new(&world.bounds, MinimapTier::Low.px_per_m(), true);
        let (styles, perch) = layout_styles(&layout.primitives, &world.blocks);
        let t1 = RasterTables::build(&proj, &world, &paint, &perch);
        let b1 = draw_base(&proj, &t1, &styles, false);
        let b2 = draw_base(&proj, &t1, &styles, false);
        assert_eq!(b1, b2);
        let px = proj.w as usize * proj.h as usize;
        let mut ink1 = vec![0u8; px * 4];
        let mut flash1 = vec![0u8; px * 4];
        let mut owner1 = vec![0u8; px];
        let mut sc1 = InkScratch::with_size(px);
        let f1 = draw_ink(
            &proj,
            &t1,
            paint.grid(),
            0,
            proj.h as usize,
            TEAM_RGB,
            &mut sc1,
            &mut ink1,
            &mut flash1,
            &mut owner1,
        );
        let mut ink2 = vec![0u8; px * 4];
        let mut flash2 = vec![0u8; px * 4];
        let mut owner2 = vec![0u8; px];
        let mut sc2 = InkScratch::with_size(px);
        let f2 = draw_ink(
            &proj,
            &t1,
            paint.grid(),
            0,
            proj.h as usize,
            TEAM_RGB,
            &mut sc2,
            &mut ink2,
            &mut flash2,
            &mut owner2,
        );
        assert_eq!(f1, f2);
        assert_eq!(ink1, ink2);
        assert_eq!(flash1, flash2);
        assert_eq!(owner1, owner2);
    }
}
