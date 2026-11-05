//! Task 50 batch B engine — offline evaluation of the WebAudio synthesis
//! graphs the JS builds (spec D-T50-1: "WebAudio 合成图离线求值为 f32 PCM").
//!
//! [`Vb`] mirrors the JS `V` voice builder (music.js L218-344) method for
//! method, so every `SFX.<name>.build(v, p)` translates mechanically.
//! [`Graph`] is a tiny render graph: gain / biquad / waveshaper /
//! stereo-panner / delay nodes with `AudioParam` automation plus
//! source→param modulation edges, evaluated sample-by-sample in topological
//! order (edges leaving a `Delay` node are excluded from the sort, which is
//! exactly how WebAudio breaks feedback cycles).
//!
//! Declared deviations (D-T50-6 additions):
//!   * the convolver reverb send / HRTF panner / distance low-pass of the
//!     live bus (audio.js L113-143, L195-241) are NOT baked — only the
//!     voice graph up to `v.out` is rendered;
//!   * `StereoPanner` inputs are treated as mono (all JS SFX panner inputs
//!     are mono source chains);
//!   * baked samples are clamped to ±1 (the JS bus limiter does this at
//!     runtime); a panned voice is down-mixed to `(L + R)` mono, which
//!     matches the JS destination sum (spatial imaging only is lost);
//!   * modulation sums are mono (L channel), ≡ the JS sources feeding
//!     AudioParams are all mono.

use std::collections::VecDeque;
use std::sync::Arc;

use super::synth::{
    self, Biquad, BqKind, NoiseKind, Osc, Param, Rng, SR, Wave, drive_curve, noise_buffer,
    shaper_map,
};

pub type NodeId = usize;

/// Which parameter of a node a modulation edge targets. Full ported engine
/// surface (audio.js `_param`); SFX modulation currently only wires Gain/Freq.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamRef {
    Gain(NodeId),
    Freq(NodeId),
    Q(NodeId),
    Pan(NodeId),
    Time(NodeId),
    Rate(NodeId),
    Detune(NodeId),
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Osc(Wave),
    Buf,
    Gain,
    Biquad(BqKind),
    Shaper,
    Panner,
    /// Engine parity (audio.js L251); no SFX builder calls `v.delay`.
    #[allow(dead_code)]
    Delay,
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    None,
    One(usize),
    /// (freq, Q) for biquads; (freq, detune) for oscillators
    Two(usize, usize),
}

struct NodeState {
    osc: Osc,
    pos: f64,
    ring: Vec<f64>,
    wpos: usize,
    bq: [Biquad; 2],
    curve: Option<Arc<Vec<f32>>>,
}

struct Node {
    kind: Kind,
    slots: Slot,
    start: f64,
    stop: f64,
    inputs: Vec<NodeId>,
    stereo: bool,
    st: NodeState,
}

/// The offline graph. Times are seconds relative to voice start (t = 0).
pub struct Graph {
    nodes: Vec<Node>,
    params: Vec<Param>,
    /// per-node peaking gain in dB (parallel to `nodes`)
    bq_gain_db: Vec<f64>,
    mods: Vec<(NodeId, ParamRef)>,
    buffers: Vec<Option<Arc<Vec<f32>>>>,
    order: Vec<NodeId>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            params: Vec::new(),
            bq_gain_db: Vec::new(),
            mods: Vec::new(),
            buffers: Vec::new(),
            order: Vec::new(),
        }
    }

    fn add(&mut self, kind: Kind, slots: Slot) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            slots,
            start: 0.0,
            stop: f64::INFINITY,
            inputs: Vec::new(),
            stereo: false,
            st: NodeState {
                osc: Osc::new(Wave::Sine),
                pos: 0.0,
                ring: Vec::new(),
                wpos: 0,
                bq: [Biquad::new(BqKind::Lowpass), Biquad::new(BqKind::Lowpass)],
                curve: None,
            },
        });
        self.bq_gain_db.push(0.0);
        self.buffers.push(None);
        id
    }

    fn new_param(&mut self, default: f64) -> usize {
        self.params.push(Param::new(default));
        self.params.len() - 1
    }

    pub fn param(&mut self, node: NodeId) -> &mut Param {
        match self.nodes[node].slots {
            Slot::One(i) | Slot::Two(i, _) => &mut self.params[i],
            Slot::None => panic!("node has no primary parameter"),
        }
    }

    pub fn param2(&mut self, node: NodeId) -> &mut Param {
        match self.nodes[node].slots {
            Slot::Two(_, i) => &mut self.params[i],
            _ => panic!("node has no secondary parameter"),
        }
    }

    pub fn connect(&mut self, src: NodeId, dst: NodeId) {
        self.nodes[dst].inputs.push(src);
        if self.nodes[src].stereo {
            self.nodes[dst].stereo = true;
        }
    }

    pub fn connect_param(&mut self, src: NodeId, target: ParamRef) {
        self.mods.push((src, target));
    }

    fn slot_of(&self, r: &ParamRef) -> usize {
        match *r {
            ParamRef::Gain(n) | ParamRef::Pan(n) | ParamRef::Time(n) | ParamRef::Rate(n) => {
                match self.nodes[n].slots {
                    Slot::One(i) => i,
                    _ => panic!("bad param ref"),
                }
            }
            ParamRef::Freq(n) | ParamRef::Q(n) | ParamRef::Detune(n) => match self.nodes[n].slots {
                Slot::Two(a, b) => {
                    if matches!(r, ParamRef::Freq(_)) {
                        a
                    } else {
                        b
                    }
                }
                _ => panic!("bad param ref"),
            },
        }
    }

    /// Topological order over data + modulation edges; edges leaving a
    /// `Delay` node are ignored so feedback loops resolve (WebAudio rule).
    fn topo(&mut self) {
        let n = self.nodes.len();
        let mut indeg = vec![0usize; n];
        let mut adj = vec![Vec::new(); n];
        let is_delay = |i: usize| matches!(self.nodes[i].kind, Kind::Delay);
        for (dst, node) in self.nodes.iter().enumerate() {
            for &src in &node.inputs {
                if !is_delay(src) {
                    adj[src].push(dst);
                    indeg[dst] += 1;
                }
            }
        }
        for (src, tgt) in &self.mods {
            let owner = node_of(tgt);
            if !is_delay(*src) && owner != *src {
                adj[*src].push(owner);
                indeg[owner] += 1;
            }
        }
        let mut q: VecDeque<NodeId> = VecDeque::new();
        q.extend((0..n).filter(|&i| indeg[i] == 0));
        let mut order = Vec::with_capacity(n);
        while let Some(i) = q.pop_front() {
            order.push(i);
            for &j in &adj[i] {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    q.push_back(j);
                }
            }
        }
        if order.len() != n {
            // residual cycle (WebAudio requires a delay in every cycle, so
            // this cannot happen for valid graphs). Insertion order fallback
            // — declared deviation.
            order = (0..n).collect();
        }
        self.order = order;
    }

    /// Render `n` samples of the graph rooted at `out` into a mono PCM block.
    pub fn render_mono(&mut self, out: NodeId, n: usize) -> Vec<f32> {
        if self.order.is_empty() {
            self.topo();
        }
        let nn = self.nodes.len();
        let mut ol = vec![0.0f64; nn];
        let mut or = vec![0.0f64; nn];
        let mut mod_sum = vec![0.0f64; self.params.len()];
        let mut mods_by_src: Vec<Vec<usize>> = vec![Vec::new(); nn];
        for (src, tgt) in &self.mods {
            let slot = self.slot_of(tgt);
            mods_by_src[*src].push(slot);
        }
        let mut pcm: Vec<f32> = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f64 / SR;
            mod_sum.fill(0.0);
            for idx in 0..self.order.len() {
                let id = self.order[idx];
                let (l, r) = self.eval_node(id, t, &mod_sum, &ol, &or);
                ol[id] = l;
                or[id] = r;
                for &slot in &mods_by_src[id] {
                    mod_sum[slot] += l;
                }
            }
            pcm.push((ol[out] + or[out]).clamp(-1.0, 1.0) as f32);
        }
        pcm
    }

    fn eval_node(
        &mut self,
        id: NodeId,
        t: f64,
        mod_sum: &[f64],
        ol: &[f64],
        or: &[f64],
    ) -> (f64, f64) {
        let mut il = 0.0f64;
        let mut ir = 0.0f64;
        for &s in &self.nodes[id].inputs {
            il += ol[s];
            ir += or[s];
        }
        let stereo = self.nodes[id].stereo;
        let slots = self.nodes[id].slots;
        let pv =
            |params: &[Param], slot: usize| -> f64 { params[slot].value_at(t) + mod_sum[slot] };
        match self.nodes[id].kind {
            Kind::Osc(wave) => {
                let active = t >= self.nodes[id].start && t < self.nodes[id].stop;
                if !active {
                    return (0.0, 0.0);
                }
                let (fs, ds) = match slots {
                    Slot::Two(a, b) => (a, b),
                    _ => panic!("osc slots"),
                };
                let f = pv(&self.params, fs);
                let d = pv(&self.params, ds);
                let freq = f * 2f64.powf(d / 1200.0);
                let w = self.nodes[id].st.osc.tick(wave, freq);
                (w, w)
            }
            Kind::Buf => {
                let active = t >= self.nodes[id].start && t < self.nodes[id].stop;
                if !active {
                    return (0.0, 0.0);
                }
                let rs = match slots {
                    Slot::One(a) => a,
                    _ => panic!("buf slots"),
                };
                let rate = pv(&self.params, rs);
                let buf = self.buffers[id]
                    .as_ref()
                    .expect("buffer source without data");
                let len = buf.len() as f64;
                let pos = self.nodes[id].st.pos;
                let x = read_loop(buf, pos);
                self.nodes[id].st.pos = (pos + rate) % len;
                (x, x)
            }
            Kind::Gain => {
                let g = match slots {
                    Slot::One(a) => pv(&self.params, a),
                    _ => panic!("gain slots"),
                };
                (il * g, ir * g)
            }
            Kind::Biquad(kind) => {
                let (fs, qs) = match slots {
                    Slot::Two(a, b) => (a, b),
                    _ => panic!("bq slots"),
                };
                let f = pv(&self.params, fs);
                let q = pv(&self.params, qs);
                let gdb = self.bq_gain_db[id];
                let n = &mut self.nodes[id];
                n.st.bq[0].kind = kind;
                n.st.bq[1].kind = kind;
                n.st.bq[0].set(f, q, gdb, SR);
                let l = n.st.bq[0].process(il);
                n.st.bq[1].set(f, q, gdb, SR);
                let r = n.st.bq[1].process(ir);
                (l, r)
            }
            Kind::Shaper => {
                let curve = self.nodes[id].st.curve.clone().expect("shaper curve");
                let l = shaper_map(&curve, il);
                let r = if stereo { shaper_map(&curve, ir) } else { l };
                (l, r)
            }
            Kind::Panner => {
                let ps = match slots {
                    Slot::One(a) => pv(&self.params, a),
                    _ => panic!("pan slots"),
                };
                let th = (ps.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
                (il * th.cos(), il * th.sin())
            }
            Kind::Delay => {
                let time = match slots {
                    Slot::One(a) => pv(&self.params, a),
                    _ => panic!("delay slots"),
                };
                let d = (time * SR).max(1.0);
                let n = &mut self.nodes[id];
                let len = n.st.ring.len();
                let wp = n.st.wpos;
                let k = d.floor() as usize;
                let fr = d - k as f64;
                let i0 = (wp + len - k) % len;
                let i1 = (i0 + 1) % len;
                let a = n.st.ring[i0];
                let b = n.st.ring[i1];
                let y = a + (b - a) * fr;
                n.st.ring[wp] = il;
                n.st.wpos = (wp + 1) % len;
                (y, y)
            }
        }
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

fn node_of(r: &ParamRef) -> NodeId {
    match r {
        ParamRef::Gain(n)
        | ParamRef::Freq(n)
        | ParamRef::Q(n)
        | ParamRef::Pan(n)
        | ParamRef::Time(n)
        | ParamRef::Rate(n)
        | ParamRef::Detune(n) => *n,
    }
}

fn read_loop(d: &[f32], pos: f64) -> f64 {
    let len = d.len() as f64;
    let p = ((pos % len) + len) % len;
    let k = p.floor() as usize;
    let f = p - k as f64;
    let a = d[k] as f64;
    let b = d[(k + 1) % d.len()] as f64;
    a + (b - a) * f
}

// ---------------------------------------------------------------------------
// Vb — the JS `V` voice builder (music.js L218-344)
// ---------------------------------------------------------------------------

/// Options for [`Vb::tone`] ≡ the JS `v.tone({...})` object literal.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tone {
    pub at: Option<f64>,
    pub t: f64,
    pub f: f64,
    pub f1: Option<f64>,
    pub sw: Option<f64>,
    pub a: Option<f64>,
    pub d: Option<f64>,
    pub h: Option<f64>,
    pub peak: Option<f64>,
    pub wave: Option<Wave>,
    pub detune: Option<f64>,
    pub to: Option<NodeId>,
    pub lin: bool,
}

/// Options for [`Vb::nz`] ≡ the JS `v.nz({...})` object literal.
#[derive(Clone, Copy, Debug, Default)]
pub struct Nz {
    pub at: Option<f64>,
    pub t: f64,
    pub f: Option<f64>,
    pub f1: Option<f64>,
    pub sw: Option<f64>,
    pub q: Option<f64>,
    pub a: Option<f64>,
    pub d: Option<f64>,
    pub h: Option<f64>,
    pub peak: Option<f64>,
    pub kind: Option<NoiseKind>,
    /// `ft`: outer `None` = key absent (bandpass), `Some(None)` = explicit null
    pub ft: Option<Option<BqKind>>,
    pub rate: Option<f64>,
    pub to: Option<NodeId>,
    pub lin: bool,
}

/// Cached shared noise buffers (≡ the JS per-ctx cache).
fn noise_buf(kind: NoiseKind) -> Arc<Vec<f32>> {
    use std::sync::OnceLock;
    static WHITE: OnceLock<Arc<Vec<f32>>> = OnceLock::new();
    static PINK: OnceLock<Arc<Vec<f32>>> = OnceLock::new();
    static BROWN: OnceLock<Arc<Vec<f32>>> = OnceLock::new();
    let cell = match kind {
        NoiseKind::White => &WHITE,
        NoiseKind::Pink => &PINK,
        NoiseKind::Brown => &BROWN,
    };
    cell.get_or_init(|| Arc::new(noise_buffer(kind))).clone()
}

/// Voice builder mirroring the JS `V` class.
pub struct Vb {
    pub g: Graph,
    pub out: NodeId,
    pub t: f64,
    pub end: f64,
    pub rng: Rng,
    pub nyq: f64,
    pub endless: bool,
    srcs: Vec<(NodeId, f64)>,
}

impl Vb {
    pub fn new(t: f64, rng: Rng) -> Self {
        let mut g = Graph::new();
        let slot = g.new_param(1.0);
        let out = g.add(Kind::Gain, Slot::One(slot));
        Self {
            g,
            out,
            t,
            end: t,
            rng,
            nyq: SR * 0.5 - 200.0,
            endless: false,
            srcs: Vec::new(),
        }
    }

    /// ≡ `v.r(a, b)`
    pub fn r(&mut self, a: f64, b: f64) -> f64 {
        self.rng.range(a, b)
    }

    /// ≡ `v.pick(arr)`
    pub fn pick<T: Copy>(&mut self, arr: &[T]) -> T {
        let i = (self.rng.next_f64() * arr.len() as f64).floor() as usize % arr.len();
        arr[i]
    }

    /// ≡ `v.gain(v, to)`
    pub fn gain(&mut self, v: f64, to: Option<NodeId>) -> NodeId {
        let slot = self.g.new_param(v);
        let n = self.g.add(Kind::Gain, Slot::One(slot));
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.filter(type, f, Q, to, gainDb)`
    pub fn filter(
        &mut self,
        kind: BqKind,
        f: f64,
        q: f64,
        to: Option<NodeId>,
        gain_db: Option<f64>,
    ) -> NodeId {
        let f = f.clamp(10.0, self.nyq);
        let fs = self.g.new_param(f);
        let qs = self.g.new_param(q);
        let n = self.g.add(Kind::Biquad(kind), Slot::Two(fs, qs));
        self.g.bq_gain_db[n] = gain_db.unwrap_or(0.0);
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.shaper(k, to)`
    pub fn shaper(&mut self, k: f64, to: Option<NodeId>) -> NodeId {
        let n = self.g.add(Kind::Shaper, Slot::None);
        self.g.nodes[n].st.curve = Some(Arc::new(drive_curve(k)));
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.pan(p, to)`
    pub fn pan(&mut self, p: f64, to: Option<NodeId>) -> NodeId {
        let slot = self.g.new_param(p.clamp(-1.0, 1.0));
        let n = self.g.add(Kind::Panner, Slot::One(slot));
        self.g.nodes[n].stereo = true;
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.delay(time, to, max)` — engine parity, unused by the SFX table.
    #[allow(dead_code)]
    pub fn delay(&mut self, time: f64, to: Option<NodeId>, max: f64) -> NodeId {
        let slot = self.g.new_param(time);
        let ring_len = (max * SR).ceil() as usize + 2;
        let n = self.g.add(Kind::Delay, Slot::One(slot));
        self.g.nodes[n].st.ring = vec![0.0; ring_len];
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v._src` start/stop bookkeeping.
    fn src_end(&mut self, n: NodeId, t0: f64, t1: Option<f64>) {
        self.g.nodes[n].start = t0.max(0.0);
        if let Some(t1) = t1 {
            let e = t1.max(t0 + 0.002);
            self.g.nodes[n].stop = e;
            self.srcs.push((n, e));
            self.end = self.end.max(e);
        } else {
            self.g.nodes[n].stop = f64::INFINITY;
            self.endless = true;
        }
    }

    /// ≡ `v.osc(type, f, t0, t1, to)`
    pub fn osc(
        &mut self,
        wave: Wave,
        f: f64,
        t0: f64,
        t1: Option<f64>,
        to: Option<NodeId>,
    ) -> NodeId {
        let f = f.min(self.nyq);
        let fs = self.g.new_param(f);
        let ds = self.g.new_param(0.0);
        let n = self.g.add(Kind::Osc(wave), Slot::Two(fs, ds));
        self.g.nodes[n].st.osc = Osc::new(wave);
        self.src_end(n, t0, t1);
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.noise(kind, t0, t1, to, rate)` — start offset consumes the rng.
    pub fn noise(
        &mut self,
        kind: NoiseKind,
        t0: f64,
        t1: Option<f64>,
        to: Option<NodeId>,
        rate: f64,
    ) -> NodeId {
        let buf = noise_buf(kind);
        let offset = self.rng.next_f64() * 1.9;
        self.buffer(buf, t0, t1, to, rate, Some(offset))
    }

    /// ≡ `v.buffer(buf, t0, t1, to, rate, offset)` — offset defaults to
    /// `rng() * dur * 0.95`.
    pub fn buffer(
        &mut self,
        buf: Arc<Vec<f32>>,
        t0: f64,
        t1: Option<f64>,
        to: Option<NodeId>,
        rate: f64,
        offset: Option<f64>,
    ) -> NodeId {
        let dur = buf.len() as f64 / SR;
        let offset = offset.unwrap_or_else(|| self.rng.next_f64() * dur * 0.95);
        let rs = self.g.new_param(rate);
        let n = self.g.add(Kind::Buf, Slot::One(rs));
        self.g.buffers[n] = Some(buf);
        self.g.nodes[n].st.pos = offset * SR;
        self.src_end(n, t0, t1);
        if let Some(to) = to {
            self.g.connect(n, to);
        }
        n
    }

    /// ≡ `v.lfo(rate, depth, param, t0, t1, type)` → (gain node, osc node)
    pub fn lfo(
        &mut self,
        rate: f64,
        depth: f64,
        param: ParamRef,
        t0: f64,
        t1: Option<f64>,
        wave: Wave,
    ) -> (NodeId, NodeId) {
        let g = self.gain(depth, None);
        self.g.connect_param(g, param);
        let o = self.osc(wave, rate, t0, t1, Some(g));
        (g, o)
    }

    /// ≡ `v.tone(o)` — returns the oscillator node.
    pub fn tone(&mut self, o: Tone) -> NodeId {
        let t = o.at.unwrap_or(self.t + o.t);
        let to = Some(o.to.unwrap_or(self.out));
        let g = self.gain(0.0, to);
        let a = o.a.unwrap_or(0.002);
        let d = o.d.unwrap_or(0.1);
        let peak = o.peak.unwrap_or(0.5);
        let end = match o.h {
            Some(h) => synth::ahr(self.g.param(g), t, a, peak, h, d),
            None => synth::perc(self.g.param(g), t, a, peak, d),
        };
        let x = self.osc(
            o.wave.unwrap_or(Wave::Sine),
            o.f,
            t,
            Some(end + 0.01),
            Some(g),
        );
        if let Some(f1) = o.f1 {
            let sw = o.sw.unwrap_or(a + d);
            synth::sweep(
                self.g.param(x),
                t,
                o.f.min(self.nyq),
                f1.min(self.nyq),
                sw,
                o.lin,
            );
        }
        if let Some(det) = o.detune {
            self.g.param2(x).set_default(det);
        }
        x
    }

    /// ≡ `v.nz(o)` — returns the filter node (or None when `ft: null`).
    pub fn nz(&mut self, o: Nz) -> Option<NodeId> {
        let t = o.at.unwrap_or(self.t + o.t);
        let to = Some(o.to.unwrap_or(self.out));
        let g = self.gain(0.0, to);
        let ft = match o.ft {
            None => Some(BqKind::Bandpass),
            Some(f) => f,
        };
        let f = o.f.unwrap_or(1000.0);
        let fl = ft.map(|k| self.filter(k, f, o.q.unwrap_or(1.0), Some(g), None));
        let a = o.a.unwrap_or(0.001);
        let d = o.d.unwrap_or(0.1);
        let peak = o.peak.unwrap_or(0.5);
        let end = match o.h {
            Some(h) => synth::ahr(self.g.param(g), t, a, peak, h, d),
            None => synth::perc(self.g.param(g), t, a, peak, d),
        };
        self.noise(
            o.kind.unwrap_or(NoiseKind::White),
            t,
            Some(end + 0.01),
            Some(fl.unwrap_or(g)),
            o.rate.unwrap_or(1.0),
        );
        if let (Some(fl), Some(f1)) = (fl, o.f1) {
            let sw = o.sw.unwrap_or(a + d);
            synth::sweep(
                self.g.param(fl),
                t,
                f.min(self.nyq),
                f1.min(self.nyq),
                sw,
                o.lin,
            );
        }
        fl
    }

    /// ≡ `v.bub(T, f, peak, d, rise, to)`
    pub fn bub(
        &mut self,
        t: f64,
        f: f64,
        peak: f64,
        d: f64,
        rise: f64,
        to: Option<NodeId>,
    ) -> NodeId {
        self.tone(Tone {
            at: Some(t),
            f,
            f1: Some(f * rise),
            sw: Some(d),
            a: Some(0.0012),
            d: Some(d),
            peak: Some(peak),
            to,
            ..Default::default()
        })
    }

    /// ≡ `v.kill(at)` — stop every source at `at`.
    pub fn kill(&mut self, at: f64) {
        let srcs = std::mem::take(&mut self.srcs);
        for (n, e) in srcs {
            if e > at {
                self.g.nodes[n].stop = at;
            }
        }
        self.end = at;
        self.endless = false;
    }

    /// Sort the automation event lists (JS schedules in time order; the sort
    /// is a safety net).
    pub fn finalize(&mut self) {
        for p in &mut self.g.params {
            p.sort();
        }
    }

    /// Render the voice graph to a mono PCM block.
    pub fn bake(mut self) -> Vec<f32> {
        self.finalize();
        let end = self.end.max(0.0);
        let n = (end * SR).ceil() as usize;
        self.g.render_mono(self.out, n)
    }

    /// The voice output gain parameter (≡ `voice.out.gain`).
    pub fn out_param(&mut self) -> &mut Param {
        self.g.param(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_bakes_deterministically_and_finite() {
        let mk = || {
            let mut vb = Vb::new(0.0, Rng::new(7));
            vb.tone(Tone {
                f: 440.0,
                f1: Some(220.0),
                sw: Some(0.05),
                a: Some(0.002),
                d: Some(0.08),
                peak: Some(0.5),
                ..Default::default()
            });
            vb.bake()
        };
        let a = mk();
        let b = mk();
        assert_eq!(a, b, "same params ⇒ byte-identical");
        assert!(!a.is_empty());
        assert!(a.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        // perc end: a + d + 0.008 + 0.01 = 0.100 → 4410 samples
        assert_eq!(a.len(), (0.1 * SR).ceil() as usize);
    }

    #[test]
    fn lfo_modulation_is_applied() {
        // sine osc at 100 Hz modulated ±50 by a 10 Hz LFO: the baked signal
        // must differ from the un-modulated one.
        let render = |with_lfo: bool| {
            let mut vb = Vb::new(0.0, Rng::new(1));
            let g = vb.gain(0.5, Some(vb.out));
            let x = vb.osc(Wave::Sine, 100.0, 0.0, Some(0.2), Some(g));
            if with_lfo {
                vb.lfo(10.0, 50.0, ParamRef::Freq(x), 0.0, Some(0.2), Wave::Sine);
            }
            vb.g.param(g).set(1.0, 0.0);
            vb.bake()
        };
        let a = render(false);
        let b = render(true);
        assert_ne!(a, b);
        assert!(b.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn delay_feedback_cycle_renders() {
        let mut vb = Vb::new(0.0, Rng::new(1));
        let d = vb.delay(0.02, None, 0.5);
        let fb = vb.gain(0.4, None);
        vb.g.connect(d, fb);
        vb.g.connect(fb, d);
        let g = vb.gain(1.0, None);
        vb.g.connect(fb, vb.out);
        vb.g.connect(g, vb.out);
        // impulse into the delay loop
        let src = vb.osc(Wave::Square, 200.0, 0.0, Some(0.005), Some(g));
        let _ = src;
        let pcm = vb.bake();
        assert!(!pcm.is_empty());
        assert!(pcm.iter().all(|s| s.is_finite()));
        // energy must persist past the source stop (feedback tail)
        let tail = pcm[(pcm.len() / 2)..]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail > 0.01, "feedback tail silent");
    }

    #[test]
    fn panner_downmix_matches_sum() {
        let mut vb = Vb::new(0.0, Rng::new(1));
        let p = vb.pan(0.5, Some(vb.out));
        // keep the (L+R) sum below the ±1 bake clamp: gain 0.5 on the out node
        vb.g.param(vb.out).set_default(0.5);
        let x = vb.osc(Wave::Sine, 1000.0, 0.0, Some(0.05), Some(p));
        let _ = x;
        let pcm = vb.bake();
        assert!(pcm.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        // L+R = cos(θ)+sin(θ) with θ = 0.75·π/4 ⇒ ≈ 1.306; × 0.5 out gain
        let pk = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((pk - 0.653).abs() < 0.03, "panned peak {pk}");
    }

    #[test]
    fn kill_stops_sources() {
        let mut vb = Vb::new(0.0, Rng::new(1));
        let g = vb.gain(1.0, Some(vb.out));
        vb.osc(Wave::Sine, 440.0, 0.0, None, Some(g));
        assert!(vb.endless);
        vb.kill(0.1);
        let pcm = vb.bake();
        assert_eq!(pcm.len(), (0.1 * SR).ceil() as usize);
    }
}
