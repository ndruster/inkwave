//! Task 53 (D-T53-5) — the setup-screen stage thumbnail: a byte-exact port of
//! `src/world/mapThumb.js` `layoutThumbSVG()`.
//!
//! Pure data → SVG string (no three.js, no Bevy): the JS module is re-run
//! under node by `gen-mapthumb-golden.mjs` over all eight stages × three
//! themes and the fixtures in `assets/props/thumb/` are diffed verbatim by
//! [`crate::props::tests`].
//!
//! Input contract: the renderer parses the embedded maps JSON text with its
//! own tiny JSON reader into f64 values — NOT the f32
//! [`inkwave_sim::geometry::StageLayout`] and NOT the sim's
//! `JVal`/`parse_json` (which rides `serde_json`). The thumbnail's geometry
//! maths is all `toFixed(1)` output, and stage coordinates land on exact
//! `.x5` ties (e.g. `X(z) = 54.5 + 85.3·2.5 = 267.75`): the f32 round-trip of
//! the extracted layout flips those ties and even re-orders same-`top` blocks
//! in the painter's sort. The JSON text carries the original doubles (node
//! `JSON.stringify` round-trips them exactly), so parsing at f64 reproduces
//! the JS bit-for-bit.
//!
//! Numeric fidelity notes (REVIEW_CHECKLIST):
//!  * `serde_json` is NOT correctly rounded for every decimal literal:
//!    `from_str::<f64>("18.531722026092094")` lands 1 ulp high (a mantissa
//!    above 2^53 takes its approximation path), which flips one `toFixed(1)`
//!    digit in tidewater's cabin decals. The mini parser below slices the
//!    number token and uses `str::parse::<f64>()` (correctly rounded, the
//!    same value JS `Number()` produces).
//!  * `toFixed(1/2)` is NOT Rust's `{:.1/2}`: JS breaks exact ties toward the
//!    larger digit (`(0.25).toFixed(1) === "0.3"`), Rust rounds
//!    half-to-even. [`fmt_fixed`] reproduces the JS rule (an exact tie can
//!    only occur at `odd / 2^(f+1)`, where `x * 10^f + 0.5` is computed
//!    exactly).
//!  * Raw number interpolation (`${cx}` in the spawn rings) maps to
//!    `Display for f64` — both sides emit the shortest round-trip repr.
// The setup-screen consumer wires this in with Task 57; until then only the
// golden tests read it (fx/mod.rs precedent).
#![allow(dead_code)]
use std::collections::HashSet;

const W: f64 = 344.0;
const H: f64 = 160.0;
const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;

// ---------------------------------------------------------------------------
// Mini JSON reader — correctly-rounded f64 numbers.
//
// `serde_json`'s float parser takes an approximation path once the decimal
// mantissa exceeds 2^53, so `18.531722026092094` parses 1 ulp away from the
// value JS `JSON.parse` holds. The thumbnail feeds those coordinates straight
// into `toFixed(1)`, where 1 ulp flips a printed digit; this reader instead
// slices the number token and hands it to `str::parse::<f64>()`, which is
// correctly rounded (identical to JS `Number()`).
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Jv {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Jv>),
    /// Object entries in document order.
    Obj(Vec<(String, Jv)>),
}

impl Jv {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Jv::Num(v) => Some(*v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Jv::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_bool(&self) -> Option<bool> {
        match self {
            Jv::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<&[Jv]> {
        match self {
            Jv::Arr(a) => Some(a),
            _ => None,
        }
    }
    fn field(&self, key: &str) -> Option<&Jv> {
        match self {
            Jv::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Parse a JSON document (the embedded maps text); `None` on malformed input.
fn parse_json(s: &str) -> Option<Jv> {
    let mut p = P {
        b: s.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn lit(&mut self, w: &[u8]) -> bool {
        if self.b[self.i..].starts_with(w) {
            self.i += w.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Option<Jv> {
        match self.peek()? {
            b'{' => self.obj(),
            b'[' => self.arr(),
            b'"' => self.string().map(Jv::Str),
            b't' => self.lit(b"true").then_some(Jv::Bool(true)),
            b'f' => self.lit(b"false").then_some(Jv::Bool(false)),
            b'n' => self.lit(b"null").then_some(Jv::Null),
            c if c == b'-' || c.is_ascii_digit() => self.number(),
            _ => None,
        }
    }
    /// JSON number grammar (`-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?`), then a
    /// correctly-rounded `str::parse` of the exact token slice.
    fn number(&mut self) -> Option<Jv> {
        let start = self.i;
        self.eat(b'-');
        match self.peek()? {
            b'0' => self.i += 1,
            c if c.is_ascii_digit() => {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.i += 1;
                }
            }
            _ => return None,
        }
        if self.eat(b'.') {
            let d = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let d = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d {
                return None;
            }
        }
        let tok = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        tok.parse::<f64>().ok().map(Jv::Num)
    }
    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut s = String::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return Some(s),
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{c}'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => {
                            let c1 = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&c1) {
                                if !self.eat(b'\\') || !self.eat(b'u') {
                                    return None;
                                }
                                let c2 = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&c2) {
                                    return None;
                                }
                                char::from_u32(0x10000 + ((c1 - 0xD800) << 10) + (c2 - 0xDC00))?
                            } else {
                                char::from_u32(c1)?
                            };
                            s.push(ch);
                        }
                        _ => return None,
                    }
                }
                _ => {
                    // pass the raw UTF-8 sequence through (ASCII fast path).
                    let start = self.i - 1;
                    let len = if c < 0x80 {
                        1
                    } else if (0xC2..=0xDF).contains(&c) {
                        2
                    } else if (0xE0..=0xEF).contains(&c) {
                        3
                    } else if (0xF0..=0xF4).contains(&c) {
                        4
                    } else {
                        return None;
                    };
                    if start + len > self.b.len() {
                        return None;
                    }
                    s.push_str(std::str::from_utf8(&self.b[start..start + len]).ok()?);
                    self.i = start + len;
                }
            }
        }
    }
    fn hex4(&mut self) -> Option<u32> {
        if self.i + 4 > self.b.len() {
            return None;
        }
        let raw = std::str::from_utf8(&self.b[self.i..self.i + 4]).ok()?;
        self.i += 4;
        u32::from_str_radix(raw, 16).ok()
    }
    fn arr(&mut self) -> Option<Jv> {
        if !self.eat(b'[') {
            return None;
        }
        let mut v = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Some(Jv::Arr(v));
        }
        loop {
            self.ws();
            v.push(self.value()?);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if !self.eat(b']') {
                return None;
            }
            return Some(Jv::Arr(v));
        }
    }
    fn obj(&mut self) -> Option<Jv> {
        if !self.eat(b'{') {
            return None;
        }
        let mut v = Vec::new();
        self.ws();
        if self.eat(b'}') {
            return Some(Jv::Obj(v));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return None;
            }
            self.ws();
            let val = self.value()?;
            v.push((k, val));
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if !self.eat(b'}') {
                return None;
            }
            return Some(Jv::Obj(v));
        }
    }
}

/// JS `Number.prototype.toFixed(f)` with the tie rule "pick the larger n"
/// (after the sign is stripped — see the tie branch; `pub(super)` for the
/// review-P0-1 unit test).
#[must_use]
pub(super) fn fmt_fixed(x: f64, f: usize) -> String {
    let scale = 10f64.powi(f as i32);
    // exact tie ⇔ x = odd / 2^(f+1) (only such dyadics land on a `…5` cutoff)
    let t = x * 2f64.powi(f as i32 + 1);
    if t.fract() == 0.0 && (t as i64) % 2 != 0 && t.abs() < 9e15 {
        // ECMA `toFixed` strips the sign first ("if x < 0, s = '-' and x = -x",
        // spec step 6), so a negative tie rounds away from zero:
        // `(-0.25).toFixed(1) === "-0.3"`.
        let n = (x.abs() * scale + 0.5).floor();
        let a = n as i64;
        let s = scale as i64;
        return format!(
            "{}{}.{:0>width$}",
            if x < 0.0 { "-" } else { "" },
            a / s,
            a % s,
            width = f
        );
    }
    format!("{:.*}", f, x)
}
fn fmt1(x: f64) -> String {
    fmt_fixed(x, 1)
}
fn fmt2(x: f64) -> String {
    fmt_fixed(x, 2)
}

/// JS `mix(a, b, k)` (mapThumb.js L111-115) — channels are non-negative, so
/// `Math.round` and `f64::round` agree.
fn mix(a: &str, b: &str, k: f64) -> String {
    let pa = i64::from_str_radix(&a[1..], 16).unwrap_or(0);
    let pb = i64::from_str_radix(&b[1..], 16).unwrap_or(0);
    let ch = |sh: u32| -> i64 {
        let va = ((pa >> sh) & 255) as f64;
        let vb = ((pb >> sh) & 255) as f64;
        ((va * (1.0 - k) + vb * k).round()) as i64
    };
    let v = (ch(16) << 16) | (ch(8) << 8) | ch(0);
    format!("#{v:06x}")
}

/// One expand() item (JS `d`): geometry in f64 + the flags the painter reads.
struct Item {
    kind: Kind,
    oct: Option<[f64; 3]>,
    rail: bool,
    grate: bool,
    pattern: u16,
    color: String,
}

enum Kind {
    /// box: min/max; `top = max.y`.
    Box { min: [f64; 3], max: [f64; 3] },
    /// obox: center/size/rotY(deg); `top = center.y + size.y/2`.
    Obox {
        center: [f64; 3],
        size: [f64; 3],
        rot_y: f64,
    },
    /// ramp: low/high/width; `top = high.y`.
    Ramp {
        low: [f64; 3],
        high: [f64; 3],
        width: f64,
    },
    /// octagon platform (expand() rewrites box+oct into this).
    Octagon { oct: [f64; 3], top: f64 },
}

impl Item {
    fn top(&self) -> f64 {
        match &self.kind {
            Kind::Box { max, .. } => max[1],
            Kind::Obox { center, size, .. } => center[1] + size[1] / 2.0,
            Kind::Ramp { high, .. } => high[1],
            Kind::Octagon { top, .. } => *top,
        }
    }
}

fn arr3(v: Option<&Jv>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

fn from_prim(p: &Jv) -> Option<Item> {
    let kind = match p.field("kind")?.as_str()? {
        "box" => Kind::Box {
            min: arr3(p.field("min"))?,
            max: arr3(p.field("max"))?,
        },
        "obox" => Kind::Obox {
            center: arr3(p.field("center"))?,
            size: arr3(p.field("size"))?,
            rot_y: p.field("rotY")?.as_f64()?,
        },
        "ramp" => Kind::Ramp {
            low: arr3(p.field("low"))?,
            high: arr3(p.field("high"))?,
            width: p.field("width")?.as_f64()?,
        },
        _ => return None,
    };
    Some(Item {
        kind,
        oct: arr3(p.field("oct")),
        rail: p.field("rail")?.as_bool().unwrap_or(false),
        grate: p.field("grate")?.as_bool().unwrap_or(false),
        pattern: p.field("pattern")?.as_f64()? as u16,
        color: p
            .field("color")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// mapThumb.js `mirror(d)` (L8-11) — box keeps oct untouched (the caller
/// rewrites it), obox flips oct, ramp has none.
fn mirror(mut d: Item, flip_oct: bool) -> Item {
    match &mut d.kind {
        Kind::Box { min, max } => {
            let (mn, mx) = (*min, *max);
            d.kind = Kind::Box {
                min: [-mx[0], mn[1], -mx[2]],
                max: [-mn[0], mx[1], -mn[2]],
            };
        }
        Kind::Obox { center, .. } => {
            center[0] = -center[0];
            center[2] = -center[2];
        }
        Kind::Ramp { low, high, width } => {
            let (l, h, w) = (*low, *high, *width);
            d.kind = Kind::Ramp {
                low: [-l[0], l[1], -l[2]],
                high: [-h[0], h[1], -h[2]],
                width: w,
            };
        }
        Kind::Octagon { .. } => {}
    }
    if flip_oct && let Some(o) = &mut d.oct {
        *o = [-o[0], -o[1], o[2]];
    }
    d
}

/// mapThumb.js `expand(layout)` (L7-24). The extractor stores
/// `primitives = single ++ half ++ level-mirrored(half)`; the JS thumbnail
/// re-mirrors `half` with its OWN mirror (which flips `oct`, level.js's does
/// not), so the mirrored slice is rebuilt from `half` rather than reused.
fn expand(prims: &[Jv], single: usize, half: usize) -> Vec<Item> {
    let mut all: Vec<Item> = Vec::with_capacity(single + 2 * half);
    for b in &prims[..single + half] {
        all.push(from_prim(b).expect("thumbnail primitive parses"));
    }
    for b in &prims[single..single + half] {
        let d = from_prim(b).expect("thumbnail primitive parses");
        let flip_oct = d.oct.is_some();
        all.push(mirror(d, flip_oct));
    }
    all.retain(|d| !d.rail);
    let mut out = Vec::with_capacity(all.len());
    let mut seen: HashSet<String> = HashSet::new();
    for d in all {
        let Some(oct) = d.oct else {
            out.push(d);
            continue;
        };
        let is_box = matches!(d.kind, Kind::Box { .. });
        let key = format!(
            "{},{},{}:{}",
            fmt2(oct[0]),
            fmt2(oct[1]),
            fmt2(oct[2]),
            fmt1(d.top())
        );
        if seen.contains(&key) || !is_box {
            continue;
        }
        seen.insert(key);
        let top = d.top();
        out.push(Item {
            kind: Kind::Octagon { oct, top },
            oct: Some(oct),
            rail: d.rail,
            grate: d.grate,
            pattern: d.pattern,
            color: d.color,
        });
    }
    out
}

/// One painter block: either an axis-aligned rect or a polygon footprint.
struct Block {
    poly: Option<Vec<[f64; 2]>>,
    /// rect bounds in world x/z (poly blocks leave these unset).
    x0: f64,
    x1: f64,
    z0: f64,
    z1: f64,
    top: f64,
    ramp: bool,
    grate: bool,
    pattern: u16,
    color: String,
}

fn to_blocks(items: Vec<Item>) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::with_capacity(items.len());
    for d in items {
        let (grate, pattern, color) = (d.grate, d.pattern, d.color.clone());
        let block = match d.kind {
            Kind::Box { min, max } => Block {
                poly: None,
                x0: min[0],
                x1: max[0],
                z0: min[2],
                z1: max[2],
                top: max[1],
                ramp: false,
                grate,
                pattern,
                color,
            },
            Kind::Octagon { oct, top } => {
                let [cx, cz, r] = oct;
                let poly = (0..8)
                    .map(|i| {
                        let a = PI / 8.0 + (i as f64 * PI) / 4.0;
                        [cx + r * a.cos(), cz + r * a.sin()]
                    })
                    .collect();
                Block {
                    poly: Some(poly),
                    x0: 0.0,
                    x1: 0.0,
                    z0: 0.0,
                    z1: 0.0,
                    top,
                    ramp: false,
                    grate,
                    pattern,
                    color,
                }
            }
            Kind::Obox {
                center,
                size,
                rot_y,
            } => {
                let a = (rot_y * PI) / 180.0;
                let (c, sn) = (a.cos(), a.sin());
                let (hx, hz) = (size[0] / 2.0, size[2] / 2.0);
                let poly = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                    .map(|[i, k]| {
                        [
                            center[0] + c * hx * i + sn * hz * k,
                            center[2] - sn * hx * i + c * hz * k,
                        ]
                    })
                    .to_vec();
                Block {
                    poly: Some(poly),
                    x0: 0.0,
                    x1: 0.0,
                    z0: 0.0,
                    z1: 0.0,
                    top: center[1] + size[1] / 2.0,
                    ramp: false,
                    grate,
                    pattern,
                    color,
                }
            }
            Kind::Ramp { low, high, width } => {
                let dx = high[0] - low[0];
                let dz = high[2] - low[2];
                if dx.abs() > 1e-3 && dz.abs() > 1e-3 {
                    let l = dx.hypot(dz);
                    let px = (-dz / l) * (width / 2.0);
                    let pz = (dx / l) * (width / 2.0);
                    let poly = vec![
                        [low[0] + px, low[2] + pz],
                        [high[0] + px, high[2] + pz],
                        [high[0] - px, high[2] - pz],
                        [low[0] - px, low[2] - pz],
                    ];
                    Block {
                        poly: Some(poly),
                        x0: 0.0,
                        x1: 0.0,
                        z0: 0.0,
                        z1: 0.0,
                        top: high[1],
                        ramp: true,
                        grate,
                        pattern,
                        color,
                    }
                } else {
                    let along = dz.abs() > dx.abs();
                    let hw = width / 2.0;
                    let (x0, x1, z0, z1) = if along {
                        (
                            low[0] - hw,
                            low[0] + hw,
                            low[2].min(high[2]),
                            low[2].max(high[2]),
                        )
                    } else {
                        (
                            low[0].min(high[0]),
                            low[0].max(high[0]),
                            low[2] - hw,
                            low[2] + hw,
                        )
                    };
                    Block {
                        poly: None,
                        x0,
                        x1,
                        z0,
                        z1,
                        top: high[1],
                        ramp: true,
                        grate,
                        pattern,
                        color,
                    }
                }
            }
        };
        blocks.push(block);
    }
    blocks.sort_by(|a, b| a.top.total_cmp(&b.top));
    blocks
}

/// The top-down stage thumbnail ≡ `layoutThumbSVG(layout, theme, teams)` over
/// the embedded maps JSON text (see the module doc for the f64 rationale).
#[must_use]
pub fn layout_thumb_svg(layout_json: &str, theme: &str, teams: [&str; 2]) -> Option<String> {
    let root = parse_json(layout_json)?;
    let stage = root.field("stage")?;
    let id = stage.field("id")?.as_str()?;
    let b = stage.field("bounds")?;
    let (min_x, max_x) = (b.field("minX")?.as_f64()?, b.field("maxX")?.as_f64()?);
    let (min_z, max_z) = (b.field("minZ")?.as_f64()?, b.field("maxZ")?.as_f64()?);
    let pads = stage.field("spawnPads")?.as_array()?;
    let meta = root.field("meta")?;
    let counts = meta.field("sourceCounts")?;
    let single = counts.field("single")?.as_f64()? as usize;
    let half = counts.field("half")?.as_f64()? as usize;
    let prims = root.field("primitives")?.as_array()?;
    if prims.len() != single + 2 * half {
        return None;
    }

    let pad = 10.0;
    let s = ((W - pad * 2.0) / (max_z - min_z)).min((H - pad * 2.0) / (max_x - min_x));
    let ox = (W - (max_z - min_z) * s) / 2.0;
    let oy = (H - (max_x - min_x) * s) / 2.0;
    let x_of = |z: f64| ox + (z - min_z) * s;
    let y_of = |x: f64| oy + (max_x - x) * s;
    let sunset = theme == "sunset";
    let golden = theme == "golden";
    let sea: [&str; 3] = if sunset {
        ["#ffb36b", "#e0607e", "#5b3b9a"]
    } else if golden {
        ["#ffd49a", "#3aa9b6", "#185f86"]
    } else {
        ["#7fe3f5", "#2fb1e6", "#1e76cf"]
    };
    let defs = format!(
        "<defs><linearGradient id=\"tsea{id}{theme}\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"{sea0}\"/><stop offset=\".55\" stop-color=\"{sea1}\"/><stop offset=\"1\" stop-color=\"{sea2}\"/></linearGradient>\n    <pattern id=\"tgr{id}\" width=\"2.4\" height=\"2.4\" patternUnits=\"userSpaceOnUse\"><rect width=\"2.4\" height=\"2.4\" fill=\"#8fa0b3\"/><rect width=\"1.2\" height=\"1.2\" fill=\"#dfe6ee\"/></pattern>\n    <pattern id=\"tst{id}\" width=\"4\" height=\"4\" patternUnits=\"userSpaceOnUse\" patternTransform=\"rotate(45)\"><rect width=\"4\" height=\"4\" fill=\"#f4ecdc\"/><rect width=\"2\" height=\"4\" fill=\"#ffd66b\"/></pattern></defs>",
        sea0 = sea[0],
        sea1 = sea[1],
        sea2 = sea[2]
    );
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!(
        "<rect width=\"344\" height=\"160\" fill=\"url(#tsea{id}{theme})\"/>"
    ));
    // soft wave marks
    for i in 0..6 {
        let y = 14 + i * 26;
        let x = (i * 53) % 300;
        parts.push(format!(
            "<path d=\"M{x} {y} q8 -5 16 0 t16 0\" stroke=\"#fff\" stroke-opacity=\".35\" stroke-width=\"2.4\" fill=\"none\" stroke-linecap=\"round\"/>"
        ));
    }
    let blocks = to_blocks(expand(prims, single, half));
    let flat = if sunset {
        "#f1cfae"
    } else if golden {
        "#f4e6cf"
    } else {
        "#f3ecdd"
    };
    let base = if sunset {
        "#e8bf99"
    } else if golden {
        "#ead6b6"
    } else {
        "#e7dcc6"
    };
    let white = if sunset {
        "#fbe6d0"
    } else if golden {
        "#fff8ec"
    } else {
        "#ffffff"
    };
    for blk in &blocks {
        let top = blk.top;
        let tint = if blk.pattern == 5 && !blk.color.is_empty() {
            Some(blk.color.as_str())
        } else {
            None
        };
        let stroke_op = if top > 0.05 { "0.35" } else { "0.15" };
        if let Some(poly) = &blk.poly {
            let pts = |dx: f64, dy: f64| -> String {
                poly.iter()
                    .map(|p| format!("{},{}", fmt1(x_of(p[1]) + dx), fmt1(y_of(p[0]) + dy)))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            if top > 0.2 && !blk.ramp && !blk.grate {
                parts.push(format!(
                    "<polygon points=\"{}\" fill=\"#1b2a44\" opacity=\".28\"/>",
                    pts(1.2 + top * 0.35, 1.2 + top * 0.45)
                ));
            }
            let k = (top.max(0.0) / 5.0).min(1.0);
            // JS `tint || (grate ? … : ramp ? … : top<=0.05 ? … : mix(…))`.
            let fill_owned = tint.map(String::from).unwrap_or_else(|| {
                if blk.grate {
                    format!("url(#tgr{id})")
                } else if blk.ramp {
                    format!("url(#tst{id})")
                } else if top <= 0.05 {
                    flat.to_string()
                } else {
                    mix(base, white, k)
                }
            });
            parts.push(format!(
                "<polygon points=\"{}\" fill=\"{fill_owned}\" stroke=\"#2a3552\" stroke-opacity=\"{stroke_op}\" stroke-width=\"1\" stroke-linejoin=\"round\"/>",
                pts(0.0, 0.0)
            ));
            continue;
        }
        let x = x_of(blk.z0);
        let y = y_of(blk.x1);
        let w = (blk.z1 - blk.z0) * s;
        let h = (blk.x1 - blk.x0) * s;
        if top > 0.2 && !blk.ramp && !blk.grate {
            parts.push(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"1.2\" fill=\"#1b2a44\" opacity=\".28\"/>",
                fmt1(x + 1.2 + top * 0.35),
                fmt1(y + 1.2 + top * 0.45),
                fmt1(w),
                fmt1(h)
            ));
        }
        let k = (top / 5.0).min(1.0);
        let fill_owned = if blk.grate {
            format!("url(#tgr{id})")
        } else if blk.ramp {
            format!("url(#tst{id})")
        } else if top <= 0.05 {
            flat.to_string()
        } else {
            mix(base, white, k)
        };
        let rx = if top > 0.05 { "1.4" } else { "2.5" };
        parts.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{rx}\" fill=\"{}\" stroke=\"#2a3552\" stroke-opacity=\"{stroke_op}\" stroke-width=\"1\"/>",
            fmt1(x),
            fmt1(y),
            fmt1(w),
            fmt1(h),
            tint.unwrap_or(&fill_owned)
        ));
    }
    // team splats + spawn rings
    for (t, pad) in pads.iter().enumerate() {
        let [px, _py, pz] = arr3(Some(pad))?;
        let cx = x_of(pz);
        let cy = y_of(px);
        let c = teams[t.min(1)];
        for i in 0..5 {
            let a = i as f64 * 1.7 + t as f64 * 2.0;
            let r = 9.0 + (i % 3) as f64 * 6.0;
            let sx = cx
                + a.cos() * r * (if t != 0 { -1.4 } else { 1.4 })
                + (if t != 0 { -14.0 } else { 14.0 });
            let sy = cy + a.sin() * r;
            parts.push(blob(
                sx,
                sy,
                5.0 + (i % 2) as f64 * 3.5,
                c,
                (i + t * 7) as f64,
            ));
        }
        parts.push(format!(
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"7.5\" fill=\"none\" stroke=\"#fff\" stroke-width=\"3\"/><circle cx=\"{cx}\" cy=\"{cy}\" r=\"7.5\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.6\"/>"
        ));
    }
    Some(format!(
        "<svg viewBox=\"0 0 344 160\" preserveAspectRatio=\"xMidYMid slice\" xmlns=\"http://www.w3.org/2000/svg\">{defs}{}</svg>",
        parts.join("")
    ))
}

/// mapThumb.js `blob()` (L101-110).
fn blob(x: f64, y: f64, r: f64, c: &str, seed: f64) -> String {
    let mut d = String::new();
    let n = 14.0;
    for i in 0..=14 {
        let a = (i as f64 / n) * TAU;
        let rr = r * (1.0 + 0.18 * (a * 3.0 + seed).sin() + 0.1 * (a * 5.0 + seed * 2.0).sin());
        d.push_str(if i == 0 { "M" } else { "L" });
        d.push_str(&fmt1(x + a.cos() * rr));
        d.push(' ');
        d.push_str(&fmt1(y + a.sin() * rr));
    }
    format!("<path d=\"{d}Z\" fill=\"{c}\" opacity=\".92\"/>")
}

/// The embedded maps JSON text for one stage id.
#[must_use]
pub fn embedded_layout_json(stage: &str) -> Option<&'static str> {
    Some(match stage {
        "tidewater" => inkwave_sim::TIDEWATER_JSON,
        "kelpline" => inkwave_sim::KELPLINE_JSON,
        "halyard" => inkwave_sim::HALYARD_JSON,
        "saltpan" => inkwave_sim::SALTPAN_JSON,
        "crossmarket" => inkwave_sim::CROSSMARKET_JSON,
        "lockgate" => inkwave_sim::LOCKGATE_JSON,
        "terraces" => inkwave_sim::TERRACES_JSON,
        "cargo" => inkwave_sim::CARGO_JSON,
        _ => return None,
    })
}

/// Convenience wrapper: render the thumbnail for one embedded stage id.
#[must_use]
pub fn embedded_thumb_svg(stage: &str, theme: &str, teams: [&str; 2]) -> String {
    let json = embedded_layout_json(stage).expect("thumbnail stage id");
    layout_thumb_svg(json, theme, teams).expect("embedded stage layout renders")
}
