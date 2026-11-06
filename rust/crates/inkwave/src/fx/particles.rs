//! Task 51 batches A+B — the FX data core, ported from `src/fx/fx.js`:
//! constants (L39-53), helpers (L577-611), the constructor / pool caps
//! (L663-707), colour + culling (L712-724) and the six pools with their
//! spawn/update paths (L726-1235).
//!
//! Every numeric is anchored to the JS line it came from (`// ≡ fx.js L###`).
//! The RNG is `crate::audio::synth::Rng` (mulberry32, music.js L30-38) — a
//! fixed-seed stand-in for `Math.random()` (deviation D-T50-6 precedent);
//! the *call order* of `rand()` is reproduced exactly so a fixed seed replays
//! the same particle stream.

use std::collections::HashMap;

use glam::Vec3;

use crate::audio::synth::Rng;

// ---------------------------------------------------------------------------
// constants (≡ fx.js L41-53, L658)
// ---------------------------------------------------------------------------

pub(crate) const UP: Vec3 = Vec3::new(0.0, 1.0, 0.0); // ≡ L41
pub(crate) const TAU: f32 = std::f32::consts::TAU;

/// droplet flags ≡ L47 `F_PAINT, F_RING, F_NOCOL, F_QUIET, F_MATTE`
pub(crate) const F_PAINT: u32 = 1;
pub(crate) const F_RING: u32 = 2;
pub(crate) const F_NOCOL: u32 = 4;
pub(crate) const F_QUIET: u32 = 8;
pub(crate) const F_MATTE: u32 = 16;

/// glow kinds ≡ L49 (stored as `f32`: the API passes fractional cores)
pub(crate) const G_SOFT: f32 = 0.0;
pub(crate) const G_STAR: f32 = 10.0;
pub(crate) const G_BUBBLE: f32 = 20.0;
pub(crate) const G_HALO: f32 = 30.0;

/// puff kinds ≡ L51
pub(crate) const P_MIST: f32 = 0.0;
pub(crate) const P_CLOUD: f32 = 1.0;
pub(crate) const P_FEATHER: f32 = 2.0;
pub(crate) const P_GHOST: f32 = 3.0;
pub(crate) const P_DUST: f32 = 4.0;

/// ring styles ≡ L53
pub(crate) const R_WAVE: f32 = 0.0;
pub(crate) const R_RIPPLE: f32 = 1.0;
pub(crate) const R_DISC: f32 = 2.0;
pub(crate) const R_DANGER: f32 = 3.0;
pub(crate) const R_TARGET: f32 = 4.0;
pub(crate) const R_DUST: f32 = 5.0;
pub(crate) const R_BLOT: f32 = 6.0;
pub(crate) const R_THIN: f32 = 7.0;

/// scheduler ops ≡ L658 `OP_RING, OP_CROWN, OP_DUSTRING`
pub(crate) const OP_RING: u32 = 1;
pub(crate) const OP_CROWN: u32 = 2;
pub(crate) const OP_DUSTRING: u32 = 3;

/// immediate-mark pool cap ≡ L695 `cap: 48`
pub(crate) const MARK_CAP: usize = 48;
/// immediate-pillar pool cap ≡ L696 `cap: 8`
pub(crate) const PILLAR_CAP: usize = 8;
/// schedule queue cap ≡ L697 `cap: 96`
pub(crate) const SCHED_CAP: usize = 96;

/// default RNG seed — fixed stand-in for `Math.random()` (deviation).
pub const DEFAULT_SEED: u32 = 0x1234_5678;

// ---------------------------------------------------------------------------
// math helpers (≡ fx.js L43-44, L592-611)
// ---------------------------------------------------------------------------

/// `easeOut` ≡ L43 `1 - (1 - t) * (1 - t) * (1 - t)`.
pub(crate) fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t;
    1.0 - u * u * u
}

/// `clamp` ≡ L44.
pub(crate) fn clamp(v: f32, a: f32, b: f32) -> f32 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// `THREE.Color.lerp` ≡ `a + (b - a) * t`.
pub(crate) fn col_lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

/// `THREE.Vector3.normalize()` semantics: divide by `length() || 1` — a zero
/// vector stays zero instead of producing NaN.
pub(crate) fn th_normalize(v: Vec3) -> Vec3 {
    let l = v.length();
    if l == 0.0 { Vec3::ZERO } else { v / l }
}

/// `basis` ≡ L607-611: orthonormal tangent pair around `n` (no rand).
pub(crate) fn basis(n: Vec3) -> (Vec3, Vec3) {
    let mut t = if n.y.abs() < 0.95 { Vec3::Y } else { Vec3::X };
    t = th_normalize(t.cross(n));
    (t, n.cross(t))
}

// ---------------------------------------------------------------------------
// colour (≡ fx.js L583-589, L712-721; three r152+ ColorManagement)
// ---------------------------------------------------------------------------

/// sRGB component → linear working space (three `setHex` / `new Color(css)`
/// convert; `setRGB` does not — `ColorIn::Rgb` is already linear).
pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// `THREE.Color.setHex(hex)` (sRGB → linear) ≡ L715.
pub(crate) fn hex_to_linear(h: u32) -> Vec3 {
    Vec3::new(
        srgb_to_linear(((h >> 16) & 0xff) as f32 / 255.0),
        srgb_to_linear(((h >> 8) & 0xff) as f32 / 255.0),
        srgb_to_linear((h & 0xff) as f32 / 255.0),
    )
}

/// `new THREE.Color(css)` ≡ L717 — supports `#rgb` / `#rrggbb` and a named
/// subset (declared deviation; unknown strings fall back to white, matching
/// three's console-warning path closely enough for the data plane).
pub(crate) fn css_to_linear(s: &str) -> Vec3 {
    if let Some(h) = s.strip_prefix('#') {
        let parse = |len: usize, expand: bool| -> Option<Vec3> {
            if h.len() != len {
                return None;
            }
            let mut v = 0u32;
            for ch in h.chars() {
                let d = ch.to_digit(16)?;
                v = v << 4 | d;
            }
            Some(if expand {
                // #rgb → each nibble replicated (0xf → 0xff)
                let r = (v >> 8) & 0xf;
                let g = (v >> 4) & 0xf;
                let b = v & 0xf;
                hex_to_linear(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
            } else {
                hex_to_linear(v)
            })
        };
        if let Some(c) = parse(3, true) {
            return c;
        }
        if let Some(c) = parse(6, false) {
            return c;
        }
        return WHITE;
    }
    let named: u32 = match s.to_ascii_lowercase().as_str() {
        "white" => 0xff_ffff,
        "black" => 0x00_0000,
        "red" => 0xff_0000,
        "lime" => 0x00_ff00,
        "green" => 0x00_8000,
        "blue" => 0x00_00ff,
        "yellow" => 0xff_ff00,
        "cyan" => 0x00_ffff,
        "magenta" => 0xff_00ff,
        "gray" | "grey" => 0x80_8080,
        "silver" => 0xc0_c0c0,
        "orange" => 0xff_a500,
        "purple" => 0x80_0080,
        "gold" => 0xff_d700,
        _ => return WHITE,
    };
    hex_to_linear(named)
}

/// `_white` ≡ L583 (linear 1,1,1).
pub(crate) const WHITE: Vec3 = Vec3::new(1.0, 1.0, 1.0);
/// `DUST` ≡ L585 `'#b8a98f'`.
pub(crate) fn dust_col() -> Vec3 {
    hex_to_linear(0xb8a98f)
}
/// `GRAIN` ≡ L586 `'#b9ae9c'`.
pub(crate) fn grain_col() -> Vec3 {
    hex_to_linear(0xb9ae9c)
}
/// `FOAM` ≡ L587 `'#eef9ff'`.
pub(crate) fn foam_col() -> Vec3 {
    hex_to_linear(0xeef9ff)
}
/// `WATER` ≡ L588 `'#bfe9ff'`.
pub(crate) fn water_col() -> Vec3 {
    hex_to_linear(0xbfe9ff)
}
/// `FEATHER` ≡ L589 `'#f3f0e8'`.
pub(crate) fn feather_col() -> Vec3 {
    hex_to_linear(0xf3f0e8)
}

/// Colour input for the spawn API ≡ "THREE.Color | hex | css string" (L712).
#[derive(Clone, Copy, Debug)]
pub enum ColorIn {
    /// already-linear rgb (≡ a `THREE.Color` instance / `{r,g,b}` / `setRGB`)
    Rgb(Vec3),
    /// sRGB hex, converted on use (≡ `setHex`)
    Hex(u32),
    /// CSS string, converted + cached (≡ `new THREE.Color(css)`)
    Css(&'static str),
}

impl From<Vec3> for ColorIn {
    fn from(v: Vec3) -> Self {
        ColorIn::Rgb(v)
    }
}
impl From<u32> for ColorIn {
    fn from(v: u32) -> Self {
        ColorIn::Hex(v)
    }
}
impl From<&'static str> for ColorIn {
    fn from(v: &'static str) -> Self {
        ColorIn::Css(v)
    }
}

// ---------------------------------------------------------------------------
// construction options (≡ fx.js L663-668)
// ---------------------------------------------------------------------------

/// Quality input ≡ the `quality` ctor option: a `QUALITY` preset key or a
/// direct particles multiplier.
#[derive(Clone, Copy, Debug)]
pub enum QualityInput {
    /// direct `particles` multiplier (≡ `typeof q === 'number'`)
    Factor(f32),
    /// `QUALITY` key; unknown keys fall back to `high` (≡ L667)
    Preset(&'static str),
}

/// `new FX(scene, opts)` options (minus the scene).
#[derive(Default, Clone, Copy, Debug)]
pub struct FxOptions {
    /// ≡ `quality` (L666-668); `None` ≡ `high` → 1.0
    pub quality: Option<QualityInput>,
    /// ≡ `opts.waterY ?? PLAYER.waterY` (L670; config.js L54 = -1.6)
    pub water_y: Option<f32>,
    /// ≡ `opts.gravity ?? 17` (L671)
    pub gravity: Option<f32>,
}

/// Lighting state ≡ `_light` (L678-680); `setLighting` mutates it.
#[derive(Clone, Copy, Debug)]
pub struct FxLight {
    pub sun_dir: Vec3,
    pub sun_col: Vec3,
    pub sky: Vec3,
    pub ground: Vec3,
}

// ---------------------------------------------------------------------------
// pool entries (AoS; ≡ the Float32Array layouts L664-675)
// ---------------------------------------------------------------------------

/// droplet ≡ `dP`/`dV`/`dC`/`dK`/`dA` (L727-732; `A` slots L747-757)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
    pub pos: Vec3,
    pub vel: Vec3,
    pub col: Vec3,
    /// last collision-check point ≡ `dK` (L732/L751)
    pub last: Vec3,
    pub size: f32,
    pub age: f32,
    pub life: f32,
    pub grav: f32,
    pub stretch: f32,
    pub seed: f32,
    pub flags: u32,
}

/// puff/glow sprite ≡ `P`/`V`/`C`/`X` (L867-871; X layout L884-894)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub pos: Vec3,
    pub vel: Vec3,
    pub col: Vec3,
    pub size0: f32,
    pub size1: f32,
    pub age: f32,
    pub life: f32,
    pub alpha: f32,
    pub rot: f32,
    pub rot_vel: f32,
    pub drag: f32,
    pub buoy: f32,
    pub kind: f32,
    pub fade_in: f32,
    pub seed: f32,
    pub wobble: f32,
    pub fade_out: f32,
}

/// ring ≡ `R` (L938-942; layout L951-962)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub pos: Vec3,
    pub vel: Vec3,
    pub normal: Vec3,
    pub col: Vec3,
    pub radius: f32,
    pub age: f32,
    pub life: f32,
    pub alpha: f32,
    pub style: f32,
    pub thick: f32,
    pub seed: f32,
    pub grow0: f32,
}

/// immediate mark ≡ `marks` (L695; layout L966-969)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub pos: Vec3,
    pub normal: Vec3,
    pub col: Vec3,
    pub radius: f32,
    pub t: f32,
    pub alpha: f32,
    pub style: f32,
    pub thick: f32,
    pub seed: f32,
}

/// displaced ink shell ≡ `shells` (L692; layout L1033-1043). Only `vy` is
/// ever stored/used (JS keeps the full V but sets x/z to 0, L1037).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    pub pos: Vec3,
    pub vy: f32,
    pub col: Vec3,
    pub r0: f32,
    pub r1: f32,
    pub age: f32,
    pub life: f32,
    pub expand: f32,
    pub alpha: f32,
    pub kind: f32,
    pub seed: f32,
    pub wobble: f32,
    pub axis: Vec3,
    pub crown: f32,
}

/// geyser beam ≡ `beams` (L693; layout L1089-1095)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    pub pos: Vec3,
    pub col: Vec3,
    pub radius: f32,
    pub height: f32,
    pub age: f32,
    pub life: f32,
    pub seed: f32,
}

/// immediate light pillar ≡ `pillars` (L696; layout L1098-1101)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pillar {
    pub pos: Vec3,
    pub col: Vec3,
    pub alpha: f32,
    pub radius: f32,
    pub height: f32,
    pub seed: f32,
}

/// scheduled op ≡ `_sq` row (L698, L1163-1167)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SchedItem {
    pub(crate) delay: f32,
    pub(crate) op: u32,
    pub(crate) pos: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) col: Vec3,
    pub(crate) a0: f32,
    pub(crate) a1: f32,
    pub(crate) a2: f32,
}

/// `_sprite` argument bundle (≡ L884 positional defaults).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpriteInit {
    pub pos: Vec3,
    pub vel: Vec3,
    pub col: Vec3,
    pub size0: f32,
    pub size1: f32,
    pub life: f32,
    pub alpha: f32,
    pub drag: f32,
    pub buoy: f32,
    pub kind: f32,
    pub fade_in: f32,
    pub spin: f32,
    pub wobble: f32,
    pub fade_out: f32,
}

impl SpriteInit {
    /// positional args `(pos, vel, col, size0, size1, life, alpha)`; the rest
    /// keep the JS defaults `drag = 2.2, buoy = 0, kind = 0, fadeIn = 0.08,
    /// spin = 1.2, wobble = 0, fadeOut = 0` (≡ L884).
    pub fn new(
        pos: Vec3,
        vel: Vec3,
        col: Vec3,
        size0: f32,
        size1: f32,
        life: f32,
        alpha: f32,
    ) -> Self {
        Self {
            pos,
            vel,
            col,
            size0,
            size1,
            life,
            alpha,
            drag: 2.2,
            buoy: 0.0,
            kind: 0.0,
            fade_in: 0.08,
            spin: 1.2,
            wobble: 0.0,
            fade_out: 0.0,
        }
    }
    pub fn drag(mut self, v: f32) -> Self {
        self.drag = v;
        self
    }
    pub fn buoy(mut self, v: f32) -> Self {
        self.buoy = v;
        self
    }
    pub fn kind(mut self, v: f32) -> Self {
        self.kind = v;
        self
    }
    pub fn fade_in(mut self, v: f32) -> Self {
        self.fade_in = v;
        self
    }
    pub fn spin(mut self, v: f32) -> Self {
        self.spin = v;
        self
    }
    pub fn wobble(mut self, v: f32) -> Self {
        self.wobble = v;
        self
    }
    pub fn fade_out(mut self, v: f32) -> Self {
        self.fade_out = v;
        self
    }
}

/// `mark()` options ≡ the JS defaulted tail (L966).
#[derive(Default, Clone, Copy, Debug)]
pub struct MarkOpts {
    pub style: Option<f32>,
    pub alpha: Option<f32>,
    pub t: Option<f32>,
    pub thick: Option<f32>,
    pub seed: Option<f32>,
}

/// `pillar()` options ≡ the JS defaulted tail (L1098).
#[derive(Default, Clone, Copy, Debug)]
pub struct PillarOpts {
    pub radius: Option<f32>,
    pub height: Option<f32>,
    pub alpha: Option<f32>,
    pub seed: Option<f32>,
}

// ---------------------------------------------------------------------------
// callback aliases (≡ L672-675; deviation: by-value, non-re-entrant)
// ---------------------------------------------------------------------------

/// `setCollider((from, to) => ({ point, normal }) | null)` (L672).
/// `Send + Sync` so the whole `Fx` can live in a Bevy `Resource`.
pub type ColliderFn = dyn FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)> + Send + Sync;
/// `onDropletLand` / `onSpeck` `(point, normal, color, size)` (L673-674)
pub type LandFn = dyn FnMut(Vec3, Vec3, Vec3, f32) + Send + Sync;
/// `onRipple` `(pos, amp, wavelength, speed, life)` (L675)
pub type RippleFn = dyn FnMut(Vec3, f32, f32, f32, f32) + Send + Sync;

// ---------------------------------------------------------------------------
// Fx
// ---------------------------------------------------------------------------

/// The ink FX system (data plane). Field order mirrors the JS constructor.
pub struct Fx {
    /// quality multiplier ≡ `this.q` (L668)
    pub q: f32,
    /// ≡ `this.waterY` (L670)
    pub water_y: f32,
    /// ≡ `this.gravity` (L671)
    pub gravity: f32,
    /// ≡ `this.paintEffects` (L672)
    pub paint_effects: bool,
    /// ≡ `this.maxChecks` (L673)
    pub max_checks: i32,
    /// ≡ `this._checks` (L683, reset per frame L2210)
    pub(crate) checks: i32,
    /// ≡ `this._dt` (L682)
    pub(crate) dt: f32,
    /// ≡ `this._time` (L683)
    pub(crate) time: f32,
    /// ≡ `this._recycle` (L684)
    pub(crate) recycle: u64,

    /// ≡ `Math.random` (deviation: fixed-seed mulberry32)
    pub(crate) rng: Rng,
    /// ≡ `this._colCache` (L686)
    pub(crate) col_cache: HashMap<&'static str, Vec3>,

    /// ≡ `this._camPos` (L687)
    pub cam_pos: Vec3,
    /// ≡ `this._camDir` (L687; batch C sets it from the camera)
    pub cam_dir: Vec3,
    /// ≡ `this._light` (L678-680)
    pub light: FxLight,
    /// ≡ `this._moteU.uCol` (L708; set by `setLighting`)
    pub mote_col: Vec3,

    // ---- pools (≡ L689-698; batch C reads these for the write-back pass) ----
    pub d_cap: usize,
    pub drops: Vec<Drop>,
    pub puff_cap: usize,
    pub puffs: Vec<Sprite>,
    pub glow_cap: usize,
    pub glows: Vec<Sprite>,
    pub ring_cap: usize,
    pub rings: Vec<Ring>,
    pub marks: Vec<Mark>,
    pub shell_cap: usize,
    pub shells: Vec<Shell>,
    pub beam_cap: usize,
    pub beams: Vec<Beam>,
    pub pillars: Vec<Pillar>,
    pub(crate) sched: Vec<SchedItem>,

    // ---- GPU dust-mote field ≡ `this.motes` (L704, L1142-1160) ----
    pub mote_count: usize,
    pub mote_seeds: Vec<f32>,
    /// ≡ `this.motes.visible` (L1152; toggled by the ambient system)
    pub motes_visible: bool,

    // ---- callbacks (≡ L672-675) ----
    pub collider: Option<Box<ColliderFn>>,
    pub on_droplet_land: Option<Box<LandFn>>,
    pub on_speck: Option<Box<LandFn>>,
    pub on_ripple: Option<Box<RippleFn>>,
}

impl Fx {
    /// ≡ `new FX(scene, opts)` with the default RNG seed.
    pub fn new(opts: FxOptions) -> Self {
        Self::with_seed(opts, DEFAULT_SEED)
    }

    /// ≡ `new FX(scene, opts)` with an explicit `Math.random` seed.
    pub fn with_seed(opts: FxOptions, seed: u32) -> Self {
        // ≡ L666-668: number → itself; QUALITY key → preset particles;
        // unknown key → high; `?? 1`; `max(0.25, q)`.
        let q = match opts.quality {
            None => 1.0,
            Some(QualityInput::Factor(f)) => f,
            Some(QualityInput::Preset(k)) => match k {
                "low" => 0.4,    // ≡ config.js L587
                "medium" => 0.7, // ≡ L588
                "high" => 1.0,   // ≡ L589
                "ultra" => 1.0,  // ≡ L590
                _ => 1.0,        // QUALITY[q] || QUALITY.high
            },
        };
        let q = q.max(0.25);
        // ≡ L673 maxChecks = Math.round(1100 * q)
        let max_checks = (1100.0 * q).round() as i32;
        let mut rng = Rng::new(seed);
        // ≡ L704 `_initMotes(count = Math.round(300 * q))`: count * 4 rand()
        // calls consumed at construction (they shift the shared stream).
        let mote_count = (300.0 * q).round() as usize;
        let mote_seeds: Vec<f32> = (0..mote_count * 4).map(|_| rng.next_f64() as f32).collect();
        Self {
            q,
            water_y: opts.water_y.unwrap_or(-1.6), // ≡ PLAYER.waterY, config.js L54
            gravity: opts.gravity.unwrap_or(17.0), // ≡ L671
            paint_effects: true,                   // ≡ L672
            max_checks,
            checks: 0,
            dt: 1.0 / 60.0, // ≡ L682
            time: 0.0,
            recycle: 0,
            rng,
            col_cache: HashMap::new(),
            cam_pos: Vec3::new(0.0, 10.0, 0.0), // ≡ L687
            cam_dir: Vec3::new(0.0, 0.0, -1.0),
            light: FxLight {
                // ≡ L678 sunDir(-0.41, 0.83, -0.38).normalize()
                sun_dir: th_normalize(Vec3::new(-0.41, 0.83, -0.38)),
                // ≡ L679 setRGB(1.0, 0.93, 0.84) — no sRGB conversion
                sun_col: Vec3::new(1.0, 0.93, 0.84),
                sky: Vec3::new(0.42, 0.62, 0.95),   // ≡ L679
                ground: Vec3::new(0.5, 0.46, 0.42), // ≡ L680
            },
            // ≡ L708 new Color(1.0, 0.94, 0.8) * 0.9 (setRGB → no conversion)
            mote_col: Vec3::new(0.9, 0.846, 0.72),
            // ≡ L689-698 pool caps
            d_cap: (2600.0 * q).round() as usize,
            drops: Vec::new(),
            puff_cap: (520.0 * q).round() as usize,
            puffs: Vec::new(),
            glow_cap: (300.0 * q).round() as usize,
            glows: Vec::new(),
            ring_cap: (300.0 * q).round() as usize,
            rings: Vec::new(),
            marks: Vec::new(),
            shell_cap: 32,
            shells: Vec::new(),
            beam_cap: 10,
            beams: Vec::new(),
            pillars: Vec::new(),
            sched: Vec::new(),
            mote_count,
            mote_seeds,
            motes_visible: true, // THREE default `visible = true`
            collider: None,
            on_droplet_land: None,
            on_speck: None,
            on_ripple: None,
        }
    }

    /// ≡ `setCollider(fn)` (L672).
    pub fn set_collider<F>(&mut self, f: F)
    where
        F: FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)> + Send + Sync + 'static,
    {
        self.collider = Some(Box::new(f));
    }

    /// `rand()` ≡ `Math.random()` (L42) — the single shared stream.
    pub(crate) fn rand(&mut self) -> f32 {
        self.rng.next_f64() as f32
    }

    /// `_color` ≡ L712-721: resolve any accepted colour form into the linear
    /// working space; CSS strings are cached like `_colCache`.
    pub(crate) fn color(&mut self, c: ColorIn) -> Vec3 {
        match c {
            ColorIn::Rgb(v) => v,
            ColorIn::Hex(h) => hex_to_linear(h),
            ColorIn::Css(s) => {
                if let Some(v) = self.col_cache.get(s) {
                    return *v;
                }
                let v = css_to_linear(s);
                self.col_cache.insert(s, v);
                v
            }
        }
    }

    /// `_near` ≡ L723: cheap camera-distance culling.
    pub(crate) fn near(&self, pos: Vec3, d: f32) -> bool {
        self.cam_pos.distance_squared(pos) < d * d
    }

    /// `_toCam` ≡ L1235: unit vector from `pos` toward the camera.
    pub(crate) fn to_cam(&self, pos: Vec3) -> Vec3 {
        th_normalize(self.cam_pos - pos)
    }

    /// `coneDir` ≡ L592-600: random unit vector within `max_angle` of `n`.
    /// The two `rand()` calls happen in JS order (z, then phi).
    pub(crate) fn cone_dir(&mut self, n: Vec3, max_angle: f32) -> Vec3 {
        let cos_max = max_angle.cos();
        let z = 1.0 - self.rand() * (1.0 - cos_max);
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = self.rand() * TAU;
        let mut t_a = if n.y.abs() < 0.95 { Vec3::Y } else { Vec3::X };
        t_a = th_normalize(t_a.cross(n));
        let t_b = n.cross(t_a);
        // ≡ out = n*z + _tA*(r*cos(phi)) + _tB*(r*sin(phi)) (addScaledVector order)
        n * z + t_a * (r * phi.cos()) + t_b * (r * phi.sin())
    }

    /// `randSphere` ≡ L602-605: random unit vector (two `rand()` calls).
    pub(crate) fn rand_sphere(&mut self) -> Vec3 {
        let z = self.rand() * 2.0 - 1.0;
        let r = (1.0 - z * z).sqrt();
        let p = self.rand() * TAU;
        Vec3::new(r * p.cos(), z, r * p.sin())
    }

    // -----------------------------------------------------------------------
    // droplets (≡ L726-863)
    // -----------------------------------------------------------------------

    /// `_spawnDrop` ≡ L747-758. Full pool recycles at `_recycle % cap`,
    /// `_recycle += 7` (no rand). The `seed` rand is consumed last, matching
    /// the JS argument-evaluation order.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_spawnDrop`
    pub(crate) fn spawn_drop(
        &mut self,
        pos: Vec3,
        vel: Vec3,
        col: Vec3,
        size: f32,
        life: f32,
        grav: f32,
        stretch: f32,
        flags: u32,
    ) -> usize {
        let d = Drop {
            pos,
            vel,
            col,
            last: pos, // ≡ K = P at spawn (L751)
            size,
            age: 0.0,
            life,
            grav,
            stretch,
            seed: 0.0,
            flags,
        };
        let i = if self.drops.len() < self.d_cap {
            let i = self.drops.len();
            self.drops.push(d);
            i
        } else {
            let i = (self.recycle % self.d_cap as u64) as usize;
            self.recycle += 7;
            self.drops[i] = d;
            i
        };
        self.drops[i].seed = self.rand(); // ≡ A[i8+5] = rand() (L757)
        i
    }

    /// `_ripple` ≡ L772-776: the ink-surface hook, else a ring decal.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_ripple`
    pub(crate) fn ripple(
        &mut self,
        pos: Vec3,
        amp: f32,
        wavelength: f32,
        speed: f32,
        life: f32,
        normal: Vec3,
        col: Option<Vec3>,
    ) {
        if self.on_ripple.is_some() {
            if let Some(cb) = self.on_ripple.as_mut() {
                cb(pos, amp, wavelength, speed, life);
            }
            return;
        }
        if let Some(c) = col {
            // ≡ radius = speed*life*0.8 + 0.1, life*0.7, R_RIPPLE, 0.45, 1
            let r = speed * life * 0.8 + 0.1;
            self.ring_raw(
                pos,
                normal,
                c,
                r,
                life * 0.7,
                R_RIPPLE,
                0.45,
                1.0,
                Vec3::ZERO,
            );
        }
    }

    /// `_landDrop` ≡ L778-810. `i` indexes `self.drops` (the caller writes the
    /// updated drop back before calling).
    pub(crate) fn land_drop(&mut self, i: usize, point: Vec3, normal: Vec3, water: bool) {
        let (size, flags, col) = {
            let d = &self.drops[i];
            (d.size, d.flags, d.col)
        };
        if water {
            // ≡ L779: matte/quiet droplets just vanish under the surface
            if flags & (F_MATTE | F_QUIET) != 0 {
                return;
            }
            // ≡ L780-784: foam ripple decal at the crossing point
            let p = Vec3::new(point.x, self.water_y + 0.01, point.z);
            let c = col_lerp(col, WHITE, 0.55);
            self.ring_raw(
                p,
                UP,
                c,
                0.18 + size * 2.5,
                0.45,
                R_RIPPLE,
                0.7,
                1.0,
                Vec3::ZERO,
            );
            return;
        }
        // ≡ L785-789: paint hook, else a speck for visible non-paint drops
        if flags & F_PAINT != 0 && self.on_droplet_land.is_some() {
            if let Some(cb) = self.on_droplet_land.as_mut() {
                cb(point, normal, col, size);
            }
        } else if flags & F_MATTE == 0 && size > 0.012 && self.near(point, 26.0) {
            // ≡ L793 (`onSpeck &&` guard folded into the hook lookup; `near`
            // consumes no rand, so the reorder is stream-safe)
            if let Some(cb) = self.on_speck.as_mut() {
                cb(point, normal, col, size);
            }
        }
        if flags & F_MATTE != 0 {
            return;
        }
        // ≡ L793-797: ring flag forces a ripple decal; bigger drops ripple too
        if flags & F_PAINT == 0 && ((flags & F_RING != 0) || size > 0.03) {
            let c = if flags & F_RING != 0 { Some(col) } else { None };
            self.ripple(
                point,
                0.0016 + size * 0.05,
                0.07 + size * 0.5,
                0.75,
                0.42,
                normal,
                c,
            );
        }
        if flags & F_QUIET != 0 {
            return;
        }
        // ≡ L800-801: without the speck hook, non-paint drops leave a decal
        if self.on_speck.is_none() && flags & F_PAINT == 0 {
            self.ring_raw(
                point,
                normal,
                col,
                size * 2.3 + 0.03,
                0.2 + size * 0.6,
                R_DISC,
                0.95,
                1.0,
                Vec3::ZERO,
            );
        }
        // ≡ L802-809: big splashes throw 1–2 beads
        if size > 0.075 && self.drops.len() < self.d_cap - 8 && self.rand() < 0.6 {
            let n = 1 + (if self.rand() < 0.5 { 1 } else { 0 });
            for _ in 0..n {
                let dir = self.cone_dir(normal, 1.15);
                let sp = 1.3 + self.rand() * 1.9;
                let p = point + normal * 0.03;
                self.spawn_drop(p, dir * sp, col, size * 0.3, 0.45, 1.0, 1.0, F_NOCOL);
            }
        }
    }

    /// `_updateDrops` ≡ L812-843.
    pub(crate) fn update_drops(&mut self, dt: f32) {
        let g = self.gravity;
        let wy = self.water_y;
        let mut i = 0usize;
        while i < self.drops.len() {
            let mut d = self.drops[i];
            d.age += dt;
            if d.age >= d.life {
                self.drops.swap_remove(i); // ≡ L817 killDrop (swap-last)
                continue;
            }
            // ≡ L818-821: horizontal drag, gravity
            let drag = 1.0 - 0.35 * dt;
            d.vel.x *= drag;
            d.vel.z *= drag;
            d.vel.y = d.vel.y * drag - g * d.grav * dt;
            let o = d.pos;
            let np = o + d.vel * dt;
            d.pos = np;
            // ≡ L822-829: one segment raycast per droplet, budget-capped
            let mut hit: Option<(Vec3, Vec3)> = None;
            if d.flags & F_NOCOL == 0 && self.checks < self.max_checks {
                if let Some(cb) = self.collider.as_mut() {
                    self.checks += 1;
                    hit = (cb)(d.last, np);
                    d.last = np;
                }
            } else if d.flags & F_NOCOL != 0 {
                d.last = np;
            }
            self.drops[i] = d;
            if let Some((hp, hn)) = hit {
                self.land_drop(i, hp, hn, false);
                self.drops.swap_remove(i); // ≡ L828
                continue;
            }
            // ≡ L830-841: water crossing → foam + splash crown
            if o.y >= wy && np.y < wy {
                let t = (o.y - wy) / (o.y - np.y).max(1e-5);
                let lp = Vec3::new(o.x + (np.x - o.x) * t, wy, o.z + (np.z - o.z) * t);
                self.land_drop(i, lp, UP, true);
                self.drops.swap_remove(i);
                continue;
            }
            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // puffs + glows (≡ L866-934)
    // -----------------------------------------------------------------------

    /// `_sprite` ≡ L884-895. `puff` selects the pool (≡ `pool === this.puffs`).
    /// Full-pool replacement consumes one `rand()` *before* the rot/rotVel/seed
    /// stream (JS order preserved).
    pub(crate) fn sprite(&mut self, puff: bool, init: SpriteInit) -> usize {
        let (cap, len) = if puff {
            (self.puff_cap, self.puffs.len())
        } else {
            (self.glow_cap, self.glows.len())
        };
        let i = if len < cap {
            len
        } else {
            (self.rand() * cap as f32).floor() as usize
        };
        // ≡ L890: rotation is random except for the squid ghost
        let rot = if puff && init.kind == P_GHOST {
            0.0
        } else {
            self.rand() * TAU
        };
        let rot_vel = (self.rand() - 0.5) * init.spin; // ≡ L891
        let seed = self.rand(); // ≡ L892
        let s = Sprite {
            pos: init.pos,
            vel: init.vel,
            col: init.col,
            size0: init.size0,
            size1: init.size1,
            age: 0.0,
            life: init.life,
            alpha: init.alpha,
            rot,
            rot_vel,
            drag: init.drag,
            buoy: init.buoy,
            kind: init.kind,
            fade_in: init.fade_in,
            seed,
            wobble: init.wobble,
            fade_out: init.fade_out,
        };
        if len < cap {
            if puff {
                self.puffs.push(s);
            } else {
                self.glows.push(s);
            }
        } else if puff {
            self.puffs[i] = s;
        } else {
            self.glows[i] = s;
        }
        i
    }

    /// `_updateSprites` ≡ L897-919 (the `additive` argument only fed the
    /// batch-C write-back).
    pub(crate) fn update_sprites(&mut self, puff: bool, dt: f32) {
        let pool = if puff {
            &mut self.puffs
        } else {
            &mut self.glows
        };
        let mut i = 0usize;
        while i < pool.len() {
            let s = &mut pool[i];
            s.age += dt;
            if s.age >= s.life {
                pool.swap_remove(i); // ≡ L901 killSprite
                continue;
            }
            // ≡ L902-904: drag + buoyancy
            let drag = (1.0 - s.drag * dt).max(0.0);
            s.vel.x *= drag;
            s.vel.y = s.vel.y * drag + s.buoy * dt;
            s.vel.z *= drag;
            // ≡ L905-908: wobble — a Lissajous drift
            if s.wobble > 0.0 {
                let ph = s.age * 2.1 + s.seed * 40.0;
                s.vel.x += ph.cos() * s.wobble * dt * 2.4;
                s.vel.z += (ph * 0.83).sin() * s.wobble * dt * 2.4;
            }
            s.pos += s.vel * dt; // ≡ L909
            s.rot += s.rot_vel * dt; // ≡ L910
            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // rings + marks (≡ L937-1017)
    // -----------------------------------------------------------------------

    /// `_ringRaw` ≡ L951-962.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_ringRaw`
    pub(crate) fn ring_raw(
        &mut self,
        pos: Vec3,
        normal: Vec3,
        col: Vec3,
        radius: f32,
        life: f32,
        style: f32,
        alpha: f32,
        thick: f32,
        vel: Vec3,
    ) -> usize {
        let grow0 = if style == R_DISC || style == R_BLOT {
            0.55
        } else if style == R_DUST {
            0.35
        } else {
            0.22
        };
        let nl = {
            let l = normal.length();
            if l == 0.0 { 1.0 } else { l } // ≡ Math.hypot(...) || 1 (L952)
        };
        let i = if self.rings.len() < self.ring_cap {
            let i = self.rings.len();
            self.rings.push(Ring {
                pos,
                vel,
                normal: normal / nl,
                col,
                radius,
                age: 0.0,
                life,
                alpha,
                style,
                thick,
                seed: 0.0,
                grow0,
            });
            i
        } else {
            let i = (self.rand() * self.ring_cap as f32).floor() as usize;
            self.rings[i] = Ring {
                pos,
                vel,
                normal: normal / nl,
                col,
                radius,
                age: 0.0,
                life,
                alpha,
                style,
                thick,
                seed: 0.0,
                grow0,
            };
            i
        };
        self.rings[i].seed = self.rand(); // ≡ L961
        i
    }

    /// `mark` ≡ L966-970 (immediate, cap 48, cleared every update).
    pub fn mark(
        &mut self,
        pos: Vec3,
        normal: Option<Vec3>,
        color: ColorIn,
        radius: f32,
        opts: MarkOpts,
    ) {
        if self.marks.len() >= MARK_CAP {
            return;
        }
        let col = self.color(color);
        let n = normal.unwrap_or(UP);
        let nl = {
            let l = n.length();
            if l == 0.0 { 1.0 } else { l }
        };
        self.marks.push(Mark {
            pos,
            normal: n / nl,
            col,
            radius,
            t: opts.t.unwrap_or(0.0),              // ≡ t = 0
            alpha: opts.alpha.unwrap_or(1.0),      // ≡ alpha = 1
            style: opts.style.unwrap_or(R_TARGET), // ≡ style = R_TARGET
            thick: opts.thick.unwrap_or(1.0),      // ≡ thick = 1
            seed: opts.seed.unwrap_or(0.5),        // ≡ seed = 0.5
        });
    }

    /// `_updateRings` ≡ L977-1015 (the write-back pass is batch C).
    pub(crate) fn update_rings(&mut self, dt: f32) {
        let mut i = 0usize;
        while i < self.rings.len() {
            let r = &mut self.rings[i];
            r.age += dt;
            if r.age >= r.life {
                self.rings.swap_remove(i); // ≡ L979
                continue;
            }
            // ≡ L980-984: travelling decals (muzzle rings) drift and slow
            if r.vel != Vec3::ZERO {
                let k = (1.0 - 1.6 * dt).max(0.0);
                r.pos += r.vel * dt;
                r.vel *= k;
            }
            i += 1;
        }
        self.marks.clear(); // ≡ L1014 M.n = 0
    }

    // -----------------------------------------------------------------------
    // shells (≡ L1020-1073)
    // -----------------------------------------------------------------------

    /// `_shell` ≡ L1033-1044.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_shell`
    pub(crate) fn shell(
        &mut self,
        pos: Vec3,
        col: Vec3,
        r0: f32,
        r1: f32,
        expand: f32,
        life: f32,
        alpha: f32,
        kind: f32,
        vy: f32,
        wobble: f32,
        axis: Option<Vec3>,
        crown: f32,
    ) -> usize {
        let i = if self.shells.len() < self.shell_cap {
            let i = self.shells.len();
            self.shells.push(Shell {
                pos,
                vy,
                col,
                r0,
                r1,
                age: 0.0,
                life,
                expand,
                alpha,
                kind,
                seed: 0.0,
                wobble,
                axis: axis.unwrap_or(UP), // ≡ L1042 axis || UP
                crown,
            });
            i
        } else {
            let i = (self.rand() * self.shell_cap as f32).floor() as usize;
            self.shells[i] = Shell {
                pos,
                vy,
                col,
                r0,
                r1,
                age: 0.0,
                life,
                expand,
                alpha,
                kind,
                seed: 0.0,
                wobble,
                axis: axis.unwrap_or(UP),
                crown,
            };
            i
        };
        self.shells[i].seed = self.rand(); // ≡ L1041
        i
    }

    /// `_updateShells` ≡ L1046-1057: only the vertical drift is integrated.
    pub(crate) fn update_shells(&mut self, dt: f32) {
        let mut i = 0usize;
        while i < self.shells.len() {
            let s = &mut self.shells[i];
            s.age += dt;
            if s.age >= s.life {
                self.shells.swap_remove(i); // ≡ L1048
                continue;
            }
            s.pos.y += s.vy * dt; // ≡ L1049
            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // beams + pillars (≡ L1076-1139)
    // -----------------------------------------------------------------------

    /// `_beam` ≡ L1089-1096 (full pool overwrites slot 0, no rand).
    pub(crate) fn beam(&mut self, pos: Vec3, col: Vec3, radius: f32, height: f32, life: f32) {
        let b = Beam {
            pos,
            col,
            radius,
            height,
            age: 0.0,
            life,
            seed: self.rand(), // ≡ L1094
        };
        if self.beams.len() < self.beam_cap {
            self.beams.push(b);
        } else {
            self.beams[0] = b; // ≡ L1092 i = 0
        }
    }

    /// `pillar` ≡ L1098-1102 (immediate, cap 8, cleared every update).
    pub fn pillar(&mut self, pos: Vec3, color: ColorIn, opts: PillarOpts) {
        if self.pillars.len() >= PILLAR_CAP {
            return;
        }
        let col = self.color(color);
        self.pillars.push(Pillar {
            pos,
            col,
            alpha: opts.alpha.unwrap_or(0.6),   // ≡ alpha = 0.6
            radius: opts.radius.unwrap_or(0.5), // ≡ radius = 0.5
            height: opts.height.unwrap_or(8.0), // ≡ height = 8
            seed: opts.seed.unwrap_or(0.3),     // ≡ seed = 0.3
        });
    }

    /// `_updateBeams` ≡ L1107-1116 (aging only; pillars cleared at L1136).
    pub(crate) fn update_beams(&mut self, dt: f32) {
        let mut i = 0usize;
        while i < self.beams.len() {
            let b = &mut self.beams[i];
            b.age += dt;
            if b.age >= b.life {
                self.beams.swap_remove(i); // ≡ L1109
                continue;
            }
            i += 1;
        }
        self.pillars.clear();
    }

    // -----------------------------------------------------------------------
    // scheduler (≡ L1163-1188)
    // -----------------------------------------------------------------------

    /// `_after` ≡ L1163-1168.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_after`
    pub(crate) fn after(
        &mut self,
        delay: f32,
        op: u32,
        pos: Vec3,
        normal: Vec3,
        col: Vec3,
        a0: f32,
        a1: f32,
        a2: f32,
    ) {
        if self.sched.len() >= SCHED_CAP {
            return;
        }
        self.sched.push(SchedItem {
            delay,
            op,
            pos,
            normal,
            col,
            a0,
            a1,
            a2,
        });
    }

    /// `_runSchedule` ≡ L1169-1188: swap-remove first, then dispatch.
    pub(crate) fn run_schedule(&mut self, dt: f32) {
        let mut i = 0usize;
        while i < self.sched.len() {
            self.sched[i].delay -= dt;
            if self.sched[i].delay > 0.0 {
                i += 1;
                continue;
            }
            let it = self.sched[i];
            self.sched.swap_remove(i);
            match it.op {
                OP_RING => {
                    self.ring_raw(
                        it.pos,
                        it.normal,
                        it.col,
                        it.a0,
                        it.a1,
                        it.a2,
                        0.9,
                        1.0,
                        Vec3::ZERO,
                    );
                }
                OP_CROWN => self.crown(it.pos, it.normal, it.col, it.a0, it.a1, it.a2, 0),
                OP_DUSTRING => self.dust_ring(it.pos, it.col, it.a0, it.a1),
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // probes + shared recipes (≡ L1191-1235)
    // -----------------------------------------------------------------------

    /// `_probeDown` ≡ L1191-1202: find the surface below `pos`.
    pub(crate) fn probe_down(&mut self, pos: Vec3, depth: f32) -> Option<(Vec3, Vec3)> {
        if let Some(cb) = self.collider.as_mut() {
            let to = Vec3::new(pos.x, pos.y - depth, pos.z);
            let h = (cb)(pos, to);
            if let Some((p, n)) = h {
                return Some((p, n));
            }
        }
        // ≡ L1199: no geometry → the water plane, when the probe reaches it
        if pos.y - depth <= self.water_y && pos.y >= self.water_y {
            return Some((Vec3::new(pos.x, self.water_y + 0.01, pos.z), UP));
        }
        None
    }

    /// `_crown` ≡ L1203-1217: an even ring of droplets flung along the normal.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors `_crown`
    pub(crate) fn crown(
        &mut self,
        pos: Vec3,
        normal: Vec3,
        col: Vec3,
        n: f32,
        speed: f32,
        size: f32,
        flags: u32,
    ) {
        let (t_a, t_b) = basis(normal);
        let count = (n * self.q).round().max(1.0) as usize; // ≡ L1205
        let a0 = self.rand() * TAU; // ≡ L1206
        for i in 0..count {
            let a = a0 + (i as f32 / count as f32) * TAU + (self.rand() - 0.5) * 0.5;
            let (c, s) = (a.cos(), a.sin());
            let up = 0.7 + self.rand() * 0.9; // ≡ L1209
            let out = 0.6 + self.rand() * 0.6; // ≡ L1210
            let sp = speed * (0.6 + self.rand() * 0.6); // ≡ L1211
            let dx = (t_a.x * c + t_b.x * s) * out + normal.x * up;
            let dy = (t_a.y * c + t_b.y * s) * out + normal.y * up;
            let dz = (t_a.z * c + t_b.z * s) * out + normal.z * up;
            let sz = size * (0.55 + self.rand() * 0.8); // ≡ L1214
            self.spawn_drop(
                pos + normal * 0.04, // ≡ L1212 px = pos.x + normal.x*0.04
                Vec3::new(dx * sp, dy * sp, dz * sp),
                col,
                sz,
                1.2, // ≡ L1214 life
                1.0,
                1.0,
                flags,
            );
        }
    }

    /// `_dustRing` ≡ L1218-1226: a ring of soft dust puffs.
    pub(crate) fn dust_ring(&mut self, pos: Vec3, col: Vec3, n: f32, speed: f32) {
        let count = (n * self.q).round().max(2.0) as usize; // ≡ L1219
        let a0 = self.rand() * TAU; // ≡ L1220
        for i in 0..count {
            let a = a0 + (i as f32 / count as f32) * TAU + (self.rand() - 0.5) * 0.4;
            let sp = speed * (0.6 + self.rand() * 0.6); // ≡ L1222
            let py = pos.y + 0.12 + self.rand() * 0.12;
            let vy = 0.3 + self.rand() * 0.5;
            let s0 = 0.16 + self.rand() * 0.1;
            let s1 = 0.5 + self.rand() * 0.35;
            let life = 0.55 + self.rand() * 0.35;
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x + a.cos() * 0.25, py, pos.z + a.sin() * 0.25),
                    Vec3::new(a.cos() * sp, vy, a.sin() * sp),
                    col,
                    s0,
                    s1,
                    life,
                    0.3,
                )
                .drag(3.2)
                .buoy(0.25)
                .kind(P_DUST)
                .fade_in(0.05)
                .spin(1.2),
            );
        }
    }

    /// `_grains` ≡ L1227-1234: tiny matte dust specks (no decals).
    pub(crate) fn grains(&mut self, pos: Vec3, dir: Vec3, n: f32, speed: f32) {
        let count = (n * self.q).round().max(1.0) as usize; // ≡ L1228
        for _ in 0..count {
            let v = self.cone_dir(dir, 0.9); // ≡ L1230
            let sp = speed * (0.6 + self.rand() * 0.7); // ≡ L1231
            let sz = 0.01 + self.rand() * 0.012;
            let life = 0.3 + self.rand() * 0.2;
            self.spawn_drop(
                Vec3::new(pos.x, pos.y + 0.03, pos.z), // ≡ L1232
                v * sp,
                grain_col(),
                sz,
                life,
                1.1,
                1.0,
                F_NOCOL | F_MATTE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fx_q(q: f32) -> Fx {
        Fx::with_seed(
            FxOptions {
                quality: Some(QualityInput::Factor(q)),
                ..Default::default()
            },
            42,
        )
    }

    #[test]
    // TR-51.1: construction consumes `mote_count * 4` stream draws before
    // any spawn call shifts `Math.random` parity.
    fn motes_shift_the_stream() {
        let mut fx = Fx::with_seed(FxOptions::default(), 7);
        let mut raw = Rng::new(7);
        for _ in 0..fx.mote_count * 4 {
            raw.next_f64();
        }
        assert_eq!(fx.mote_count, 300); // round(300 * q=1)
        assert_eq!(fx.rand(), raw.next_f64() as f32);
    }

    #[test]
    fn quality_presets_map_to_config_particles() {
        // ≡ config.js L585-591 low .4 / medium .7 / high 1 / ultra 1,
        // unknown key → high, floor 0.25.
        let caps = |k: &'static str| {
            let fx = Fx::with_seed(
                FxOptions {
                    quality: Some(QualityInput::Preset(k)),
                    ..Default::default()
                },
                1,
            );
            (fx.q, fx.d_cap, fx.puff_cap, fx.mote_count)
        };
        assert_eq!(caps("low"), (0.4, 1040, 208, 120));
        assert_eq!(caps("medium"), (0.7, 1820, 364, 210));
        assert_eq!(caps("high"), (1.0, 2600, 520, 300));
        assert_eq!(caps("ultra"), (1.0, 2600, 520, 300));
        assert_eq!(caps("nonsense"), (1.0, 2600, 520, 300));
        let fx = fx_q(0.1); // max(0.25, ·)
        assert_eq!(fx.q, 0.25);
        assert_eq!(fx.max_checks, 275); // round(1100 * 0.25)
    }

    #[test]
    fn color_forms_srgb_and_named() {
        let mut fx = fx_q(1.0);
        // ≡ THREE setHex: sRGB → linear (r152+)
        let r = fx.color(ColorIn::Hex(0xff0000));
        assert!((r.x - 1.0).abs() < 1e-6 && r.y == 0.0 && r.z == 0.0);
        let g = fx.color(ColorIn::Css("#808080"));
        assert!((g.x - 0.2158605).abs() < 1e-4);
        let n = fx.color(ColorIn::Css("orange"));
        assert_eq!(n, fx.color(ColorIn::Hex(0xffa500)));
        // unknown css → WHITE
        assert_eq!(fx.color(ColorIn::Css("not-a-color")), WHITE);
        // cache path returns the identical value
        assert_eq!(fx.color(ColorIn::Css("#808080")), g);
    }

    #[test]
    // TR-51.1: full pools never grow past cap; drops recycle deterministically.
    fn drops_recycle_at_cap() {
        let mut fx = fx_q(0.25); // d_cap = 650
        for i in 0..fx.d_cap {
            fx.spawn_drop(
                Vec3::splat(i as f32),
                Vec3::ZERO,
                WHITE,
                0.1,
                2.0,
                1.0,
                1.0,
                0,
            );
        }
        assert_eq!(fx.drops.len(), fx.d_cap);
        fx.spawn_drop(
            Vec3::splat(9999.0),
            Vec3::ZERO,
            WHITE,
            0.1,
            2.0,
            1.0,
            1.0,
            0,
        );
        assert_eq!(fx.drops.len(), fx.d_cap); // no growth
        assert_eq!(fx.drops[0].pos.x, 9999.0); // recycle 0 % 650
        fx.spawn_drop(
            Vec3::splat(8888.0),
            Vec3::ZERO,
            WHITE,
            0.1,
            2.0,
            1.0,
            1.0,
            0,
        );
        assert_eq!(fx.drops[7].pos.x, 8888.0); // recycle += 7
    }

    #[test]
    fn sprites_rings_shells_random_replace() {
        let mut fx = fx_q(0.25); // puff_cap = 130
        let init = || SpriteInit::new(Vec3::ZERO, Vec3::ZERO, WHITE, 0.1, 0.2, 1.0, 0.5);
        for _ in 0..fx.puff_cap {
            fx.sprite(true, init());
        }
        let before = fx.puffs.clone();
        let i = fx.sprite(true, init());
        assert_eq!(fx.puffs.len(), fx.puff_cap);
        // exactly one slot changed — the random replacement at `i` (≡ L878)
        let changed: Vec<usize> = (0..before.len())
            .filter(|k| fx.puffs[*k] != before[*k])
            .collect();
        assert_eq!(changed, vec![i]);
    }

    #[test]
    fn beams_replace_first_consuming_only_the_seed_rand() {
        let mut fx = fx_q(1.0); // beam_cap = 10
        let mut raw = Rng::new(42);
        for _ in 0..fx.mote_count * 4 {
            raw.next_f64();
        }
        for h in 0..fx.beam_cap {
            fx.beam(Vec3::new(0.0, h as f32, 0.0), WHITE, 1.0, 2.0, 5.0);
            raw.next_f64(); // each beam consumes exactly its seed rand
        }
        fx.beam(Vec3::new(7.0, 7.0, 7.0), WHITE, 1.0, 2.0, 5.0);
        assert_eq!(fx.beams.len(), 10);
        assert_eq!(fx.beams[0].pos, Vec3::new(7.0, 7.0, 7.0)); // ≡ L1092 i = 0
        // the full-pool branch must NOT draw a replacement rand: the stored
        // seed is the very next stream value.
        assert_eq!(fx.beams[0].seed, raw.next_f64() as f32);
    }

    #[test]
    fn kill_is_swap_remove() {
        let mut fx = fx_q(1.0);
        let a = fx.spawn_drop(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            WHITE,
            0.1,
            5.0,
            1.0,
            1.0,
            0,
        );
        let b = fx.spawn_drop(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::ZERO,
            WHITE,
            0.2,
            0.1,
            1.0,
            1.0,
            0,
        );
        let c = fx.spawn_drop(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::ZERO,
            WHITE,
            0.3,
            5.0,
            1.0,
            1.0,
            0,
        );
        assert_eq!((a, b, c), (0, 1, 2));
        fx.update_drops(0.2); // b dies → c swaps into index 1
        assert_eq!(fx.drops.len(), 2);
        assert_eq!(fx.drops[0].pos.x, 1.0);
        assert_eq!(fx.drops[1].pos.x, 3.0);
    }

    #[test]
    fn ripple_hook_or_decal() {
        let mut fx = fx_q(1.0);
        // no hook → ring decal with the JS radius/life formula (L774)
        fx.ripple(Vec3::ZERO, 0.01, 0.2, 2.0, 1.0, UP, Some(WHITE));
        assert_eq!(fx.rings.len(), 1);
        let r = fx.rings[0];
        assert!((r.radius - (2.0 * 1.0 * 0.8 + 0.1)).abs() < 1e-6);
        assert!((r.life - 0.7).abs() < 1e-6);
        assert_eq!(r.style, R_RIPPLE);
        // hook present → callback instead of a decal
        // `Arc<AtomicU32>` — the hook closure must be `Send + Sync` (the
        // `Fx` lives in a Bevy `Resource`).
        let hits = std::sync::Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        fx.on_ripple = Some(Box::new(move |_p, _a, _w, _s, _l| {
            h.fetch_add(1, Ordering::Relaxed);
        }));
        fx.ripple(Vec3::ZERO, 0.01, 0.2, 2.0, 1.0, UP, Some(WHITE));
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        assert_eq!(fx.rings.len(), 1);
    }

    #[test]
    fn water_crossing_and_paint_landing() {
        let mut fx = fx_q(1.0);
        let lands = std::sync::Arc::new(AtomicU32::new(0));
        let l = lands.clone();
        fx.on_droplet_land = Some(Box::new(move |_p, _n, _c, _s| {
            l.fetch_add(1, Ordering::Relaxed);
        }));
        fx.spawn_drop(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, -10.0, 0.0),
            WHITE,
            0.05,
            2.0,
            1.0,
            1.0,
            0,
        );
        fx.spawn_drop(
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, -10.0, 0.0),
            WHITE,
            0.05,
            2.0,
            1.0,
            1.0,
            F_QUIET,
        );
        for _ in 0..10 {
            fx.update_drops(1.0 / 60.0);
        }
        // ≡ L782-786: crossing waterY never calls onDropletLand — the plain
        // drop leaves a foam ring, the quiet one vanishes silently.
        assert_eq!(lands.load(Ordering::Relaxed), 0);
        assert_eq!(fx.rings.len(), 1);
        assert_eq!(fx.rings[0].style, R_RIPPLE);
        assert!((fx.rings[0].radius - (0.18 + 0.05 * 2.5)).abs() < 1e-6);
        assert!(fx.drops.is_empty());
        // ≡ L789-791: a painting droplet on a collider surface fires the hook.
        fx.set_collider(|_from, _to| Some((Vec3::new(0.0, 0.5, 0.0), UP)));
        fx.spawn_drop(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, -5.0, 0.0),
            WHITE,
            0.05,
            5.0,
            1.0,
            1.0,
            F_PAINT,
        );
        fx.update_drops(1.0 / 60.0);
        assert_eq!(lands.load(Ordering::Relaxed), 1);
        assert!(fx.drops.is_empty());
    }

    #[test]
    fn collider_probe_budget() {
        let mut fx = fx_q(0.25); // max_checks = 275
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        fx.set_collider(move |_from, _to| {
            c.fetch_add(1, Ordering::Relaxed);
            Some((Vec3::new(0.0, -1.6, 0.0), UP))
        });
        for i in 0..600 {
            fx.spawn_drop(
                Vec3::new(i as f32 * 0.01, 1.0, 0.0),
                Vec3::new(0.0, -5.0, 0.0),
                WHITE,
                0.05,
                5.0,
                1.0,
                1.0,
                0,
            );
        }
        fx.update_drops(1.0 / 60.0);
        assert!(
            fx.checks <= fx.max_checks,
            "budget respected: {} <= {}",
            fx.checks,
            fx.max_checks
        );
        assert!(calls.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn schedule_cap_and_timing() {
        let mut fx = fx_q(1.0);
        for _ in 0..120 {
            fx.after(0.1, OP_RING, Vec3::ZERO, UP, WHITE, 1.0, 0.5, R_THIN);
        }
        assert_eq!(fx.sched.len(), SCHED_CAP); // ≡ L1165 cap 96
        fx.update_rings(0.0);
        assert_eq!(fx.rings.len(), 0);
        fx.update(0.05, None); // not due yet
        assert_eq!(fx.rings.len(), 0);
        fx.update(0.06, None); // due → OP_RING dispatches ring_raw
        assert_eq!(fx.sched.len(), 0);
        assert_eq!(fx.rings.len(), SCHED_CAP);
        let r = fx.rings[0];
        assert_eq!(r.style, R_THIN);
        assert!((r.radius - 1.0).abs() < 1e-6);
        assert!((r.life - 0.5).abs() < 1e-6);
        assert!((r.thick - 1.0).abs() < 1e-6); // ring_raw(..., 0.9, 1, ZERO)
    }

    #[test]
    fn update_clamps_dt() {
        let mut fx = fx_q(1.0);
        fx.update(10.0, Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(fx.dt, 0.05); // ≡ L2207 min(dt, 0.05)
        assert_eq!(fx.cam_pos, Vec3::new(1.0, 2.0, 3.0)); // ≡ L2215
        fx.update(0.0, Some(Vec3::new(5.0, 5.0, 5.0)));
        assert_eq!(fx.dt, 0.05); // ≡ L2208 dt > 0 guard keeps the old dt
    }

    #[test]
    fn mark_pillar_immediate_no_rand_defaults() {
        let mut fx = fx_q(1.0);
        fx.mark(
            Vec3::ZERO,
            None,
            ColorIn::Rgb(WHITE),
            1.0,
            MarkOpts::default(),
        );
        let m = fx.marks[0];
        assert_eq!(m.normal, UP); // ≡ L967 normal || UP
        assert_eq!(m.style, R_TARGET);
        assert!((m.seed - 0.5).abs() < 1e-6); // ≡ L967 seed = 0.5 (no rand)
        fx.pillar(Vec3::ZERO, ColorIn::Rgb(WHITE), PillarOpts::default());
        let p = fx.pillars[0];
        assert!((p.height - 8.0).abs() < 1e-6 && (p.alpha - 0.6).abs() < 1e-6);
        assert!((p.seed - 0.3).abs() < 1e-6); // ≡ L1098 default seed 0.3
    }

    #[test]
    fn ghost_rot_is_zero_others_random() {
        let mut fx = fx_q(1.0);
        // ≡ L890: only the squid ghost skips the rotation rand
        let i = fx.sprite(
            true,
            SpriteInit::new(Vec3::ZERO, Vec3::ZERO, WHITE, 0.1, 0.2, 1.0, 0.5).kind(P_GHOST),
        );
        assert_eq!(fx.puffs[i].rot, 0.0);
        let j = fx.sprite(
            true,
            SpriteInit::new(Vec3::ZERO, Vec3::ZERO, WHITE, 0.1, 0.2, 1.0, 0.5),
        );
        assert_ne!(fx.puffs[j].rot, 0.0);
    }
}
