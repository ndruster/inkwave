//! Task 50 batch A — procedural synthesis primitives (pure, no Bevy).
//!
//! Every primitive mirrors the JS WebAudio math it replaces, formula for
//! formula (spec D-T50-1/D-T50-2):
//!   * [`Rng`] ≡ `mulberry32` (music.js L30-38) — fixed-seed stand-in for
//!     `Math.random()` (deviation D-T50-6, appended item).
//!   * [`Param`] ≡ WebAudio `AudioParam` automation (`setValueAtTime` /
//!     `linearRampToValueAtTime` / `exponentialRampToValueAtTime` /
//!     `setTargetAtTime`).
//!   * [`perc`]/[`ahr`]/[`adsr`]/[`pts`]/[`sweep`] ≡ the envelope helpers
//!     (music.js L174-215).
//!   * [`Osc`] phase accumulator + [`Wave`] shapes; [`pulse_coeffs`] /
//!     [`stroke_coeffs`] ≡ `pulseWave`/`strokeWave` PeriodicWave harmonic
//!     tables (music.js L139-170).
//!   * [`Biquad`] RBJ cookbook coefficients ≡ `createBiquadFilter` math
//!     (WebAudio spec table, same formulas the JS nodes use).
//!   * [`noise_buffer`] ≡ `noiseBuffer` (music.js L48-95): 2 s seamless loop,
//!     seeds 11/23/37, double-pass IIR, DC block, RMS 0.3.
//!   * [`drive_curve`] ≡ `driveCurve` (music.js L126-136).
//!   * buffer-domain `pan` / `mix` / `scale` / `delay` chain helpers.
//!
//! Determinism discipline: JS evaluates AudioParam automation and oscillator
//! math in IEEE-754 **double** precision and stores buffer samples in
//! **float32** arrays; the port keeps the same widths (automation/osc/biquad
//! in `f64`, buffer contents in `f32`) and the same operation order, so two
//! renders with the same parameters are byte-identical.
//!
//! Declared deviations (D-T50-6 additions):
//!   * Basic oscillator shapes are naive math functions (WebAudio uses
//!     band-limited PeriodicWaves — aliasing only, no spectral difference in
//!     the envelope/timbre intent). `Wave::Pulse`/`Wave::Stroke` DO reproduce
//!     the JS band-limited additive tables because the JS side explicitly
//!     builds those PeriodicWaves.
//!   * `AudioParam` automation is evaluated per sample, not per 128-sample
//!     render quantum.
//!   * `WaveShaper` runs without the JS `oversample: '2x'`.
//!   * `exponentialRampToValueAtTime` with a non-positive start value would
//!     throw in JS (the build is caught and dropped); the port falls back to
//!     a linear ramp instead.

/// Sample rate of every baked cue (≡ the JS `AudioContext` default 44100).
pub const SR: f64 = 44100.0;

/// `mulberry32` (music.js L30-38) — bit-exact port.
#[derive(Clone, Copy)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }
    /// Next `[0, 1)` double, ≡ the JS closure return.
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(a | 1);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61))) ^ t;
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }
    /// `v.r(a, b)` ≡ `a + (b - a) * rng()` (music.js V.r L224).
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next_f64()
    }
}

/// `mtof` (music.js L28): MIDI note → Hz.
pub fn mtof(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

// ---------------------------------------------------------------------------
// AudioParam automation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum EvKind {
    Set,
    Lin,
    Exp,
    /// Engine parity (`setTargetAtTime`); no SFX builder schedules one.
    #[allow(dead_code)]
    Target,
}

#[derive(Clone, Copy, Debug)]
struct Ev {
    kind: EvKind,
    value: f64,
    time: f64,
    /// time constant, [`EvKind::Target`] only
    tc: f64,
}

/// A WebAudio `AudioParam`: default value + scheduled automation events.
#[derive(Clone, Debug)]
pub struct Param {
    default: f64,
    evs: Vec<Ev>,
}

impl Param {
    pub fn new(default: f64) -> Self {
        Self {
            default,
            evs: Vec::new(),
        }
    }

    /// ≡ `AudioParam.value = v` (sets the base value, not an event).
    pub fn set_default(&mut self, v: f64) {
        self.default = v;
    }

    fn push(&mut self, kind: EvKind, value: f64, time: f64, tc: f64) {
        self.evs.push(Ev {
            kind,
            value,
            time,
            tc,
        });
    }

    /// `setValueAtTime`
    pub fn set(&mut self, value: f64, time: f64) {
        self.push(EvKind::Set, value, time, 0.0);
    }
    /// `linearRampToValueAtTime`
    pub fn lin(&mut self, value: f64, time: f64) {
        self.push(EvKind::Lin, value, time, 0.0);
    }
    /// `exponentialRampToValueAtTime`
    pub fn exp(&mut self, value: f64, time: f64) {
        self.push(EvKind::Exp, value, time, 0.0);
    }
    /// `setTargetAtTime(target, startTime, timeConstant)` — engine parity,
    /// unused by the SFX table.
    #[allow(dead_code)]
    pub fn target(&mut self, value: f64, time: f64, tc: f64) {
        self.push(EvKind::Target, value, time, tc);
    }

    /// `cancelScheduledValues(t)` + `setValueAtTime(value_at(t), t)` — the
    /// music engine's fade/duck re-anchor pattern (music.js L1142/L1280,
    /// audio.js L373-374): drop every event at/after `t`, hold the current
    /// value, continue from there.
    pub fn cancel_hold(&mut self, t: f64) {
        let v = self.value_at(t);
        self.evs.retain(|e| e.time < t);
        self.push(EvKind::Set, v, t, 0.0);
    }

    /// Sort scheduled events by time (stable; JS schedules in time order,
    /// this only guards against out-of-order helper usage).
    pub fn sort(&mut self) {
        self.evs.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Automation value at absolute time `t` (modulation inputs are added by
    /// the graph, not here).
    pub fn value_at(&self, t: f64) -> f64 {
        let mut val = self.default;
        let mut prev_t = f64::NEG_INFINITY;
        let mut prev_v = self.default;
        for ev in &self.evs {
            if ev.time <= t {
                match ev.kind {
                    EvKind::Set | EvKind::Lin | EvKind::Exp => val = ev.value,
                    EvKind::Target => {
                        // start value is the pre-event value at ev.time
                        let start = val;
                        val = ev.value + (start - ev.value) * (-(t - ev.time) / ev.tc).exp();
                    }
                }
                prev_t = ev.time;
                prev_v = val;
            } else {
                // ramp into this event from the previous one
                match ev.kind {
                    EvKind::Lin => {
                        if prev_t != f64::NEG_INFINITY {
                            let f = (t - prev_t) / (ev.time - prev_t);
                            val = prev_v + (ev.value - prev_v) * f;
                        } else {
                            val = prev_v;
                        }
                    }
                    EvKind::Exp => {
                        if prev_t == f64::NEG_INFINITY {
                            val = prev_v;
                        } else if prev_v > 0.0 && ev.value > 0.0 {
                            let f = (t - prev_t) / (ev.time - prev_t);
                            val = prev_v * (ev.value / prev_v).powf(f);
                        } else {
                            // JS would throw here; fall back to linear (declared).
                            let f = (t - prev_t) / (ev.time - prev_t);
                            val = prev_v + (ev.value - prev_v) * f;
                        }
                    }
                    EvKind::Set | EvKind::Target => {}
                }
                return val;
            }
        }
        val
    }
}

// ---------------------------------------------------------------------------
// Envelope helpers (music.js L174-215) — same formulas, same return values.
// ---------------------------------------------------------------------------

const FMIN: f64 = 1e-4;

/// `perc` (L174-181). Returns the end time.
pub fn perc(p: &mut Param, t: f64, a: f64, peak: f64, d: f64) -> f64 {
    let peak = peak.max(FMIN);
    p.set(0.0, t);
    p.lin(peak, t + a);
    p.exp((peak * 0.001).max(1e-6), t + a + d);
    p.lin(0.0, t + a + d + 0.008);
    t + a + d + 0.008
}

/// `ahr` (L182-190). Returns the end time.
pub fn ahr(p: &mut Param, t: f64, a: f64, peak: f64, h: f64, r: f64) -> f64 {
    let peak = peak.max(FMIN);
    p.set(0.0, t);
    p.lin(peak, t + a);
    p.set(peak, t + a + h);
    p.exp((peak * 0.001).max(1e-6), t + a + h + r);
    p.lin(0.0, t + a + h + r + 0.008);
    t + a + h + r + 0.008
}

/// `adsr` (L192-204). `gate` = seconds from `t` until release starts.
/// Returns the end time.
#[allow(clippy::too_many_arguments)]
pub fn adsr(p: &mut Param, t: f64, a: f64, d: f64, s: f64, peak: f64, gate: f64, r: f64) -> f64 {
    let peak = peak.max(FMIN);
    let sus = (peak * s).max(1e-5);
    let ta = t + a;
    let tg = t + gate.max(a + 0.002);
    p.set(0.0, t);
    p.lin(peak, ta);
    let v;
    if tg < ta + d {
        v = peak * (sus / peak).powf((tg - ta) / d);
        p.exp(v, tg);
    } else {
        v = sus;
        p.exp(sus, ta + d);
        p.set(sus, tg);
    }
    p.exp((v * 0.001).max(1e-6), tg + r);
    p.lin(0.0, tg + r + 0.008);
    tg + r + 0.008
}

/// `pts` (L206-210): piecewise-linear from `[[dt, value], …]`.
/// Returns the end time.
pub fn pts(p: &mut Param, t: f64, list: &[(f64, f64)]) -> f64 {
    p.set(list[0].1, t + list[0].0);
    for (dt, v) in &list[1..] {
        p.lin(*v, t + *dt);
    }
    t + list[list.len() - 1].0
}

/// `sweep` (L211-215): frequency ramp helper.
pub fn sweep(p: &mut Param, t: f64, f0: f64, f1: f64, dur: f64, lin: bool) {
    p.set(f0, t);
    if lin {
        p.lin(f1, t + dur);
    } else {
        p.exp(f1.max(1e-3), t + dur);
    }
}

// ---------------------------------------------------------------------------
// Oscillators
// ---------------------------------------------------------------------------

/// Oscillator waveform. `Pulse`/`Stroke` carry the JS PeriodicWave duty /
/// sharpness and render band-limited additively like the upstream tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Square,
    Saw,
    Triangle,
    /// `pulseWave(ctx, duty)` (music.js L139-151)
    Pulse(f64),
    /// `strokeWave(ctx, sharp)` (music.js L154-170)
    Stroke(f64),
}

/// Harmonic table for a PeriodicWave: `(re, im)` per harmonic n = 1..=N.
pub type Harmonics = Vec<(f64, f64)>;

/// `pulseWave` coefficients (music.js L143-147): N = 48 harmonics.
pub fn pulse_coeffs(duty: f64) -> Harmonics {
    let n = 48;
    (1..=n)
        .map(|k| {
            let kd = k as f64;
            let re = (std::f64::consts::TAU * kd * duty).sin() / (kd * std::f64::consts::PI);
            let im =
                (1.0 - (std::f64::consts::TAU * kd * duty).cos()) / (kd * std::f64::consts::PI);
            (re, im)
        })
        .collect()
}

/// `strokeWave` coefficients (music.js L158-165): N = 40, M = 2048-point DFT
/// of the decaying-pulse shape.
pub fn stroke_coeffs(sharp: f64) -> Harmonics {
    let n = 40usize;
    let m = 2048usize;
    (1..=n)
        .map(|k| {
            let kd = k as f64;
            let mut a = 0.0f64;
            let mut b = 0.0f64;
            for i in 0..m {
                let ph = i as f64 / m as f64;
                let e = (-ph * sharp).exp() * (ph * 60.0).min(1.0);
                a += e * (std::f64::consts::TAU * kd * ph).cos();
                b += e * (std::f64::consts::TAU * kd * ph).sin();
            }
            ((2.0 * a) / m as f64, (2.0 * b) / m as f64)
        })
        .collect()
}

/// WebAudio's default PeriodicWave normalisation (peak → 1) approximated by
/// a dense peak scan of the un-normalised sum (declared deviation).
pub fn periodic_norm(h: &Harmonics) -> f64 {
    let mut pk = 0.0f64;
    let m = 8192;
    for i in 0..m {
        let ph = std::f64::consts::TAU * i as f64 / m as f64;
        let mut y = 0.0;
        for (k, (re, im)) in h.iter().enumerate() {
            let kd = (k + 1) as f64;
            y += re * (kd * ph).cos() + im * (kd * ph).sin();
        }
        pk = pk.max(y.abs());
    }
    if pk > 0.0 { 1.0 / pk } else { 1.0 }
}

/// Phase-accumulating oscillator (≡ WebAudio `OscillatorNode` semantics at
/// the sample level).
#[derive(Clone, Debug)]
pub struct Osc {
    /// phase in cycles, wraps at 1.0
    phase: f64,
    h: Option<(Harmonics, f64)>,
}

impl Osc {
    pub fn new(wave: Wave) -> Self {
        let h = match wave {
            Wave::Pulse(d) => {
                let c = pulse_coeffs(d);
                let n = periodic_norm(&c);
                Some((c, n))
            }
            Wave::Stroke(s) => {
                let c = stroke_coeffs(s);
                let n = periodic_norm(&c);
                Some((c, n))
            }
            _ => None,
        };
        Self { phase: 0.0, h }
    }

    /// Advance by `freq` Hz for one sample and return the waveform value.
    pub fn tick(&mut self, wave: Wave, freq: f64) -> f64 {
        let p = self.phase;
        self.phase = (p + freq / SR) % 1.0;
        let tau_p = std::f64::consts::TAU * p;
        match wave {
            Wave::Sine => tau_p.sin(),
            Wave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Saw => 2.0 * p - 1.0,
            Wave::Triangle => {
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
            Wave::Pulse(_) | Wave::Stroke(_) => {
                let (h, norm) = self.h.as_ref().expect("periodic table");
                let mut y = 0.0;
                for (k, (re, im)) in h.iter().enumerate() {
                    let kd = (k + 1) as f64;
                    y += re * (kd * tau_p).cos() + im * (kd * tau_p).sin();
                }
                y * norm
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Noise buffers (music.js `noiseBuffer` L48-95)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    White,
    Pink,
    Brown,
}

impl NoiseKind {
    /// ≡ `mulberry32(kind === 'white' ? 11 : kind === 'pink' ? 23 : 37)`.
    pub fn seed(self) -> u32 {
        match self {
            NoiseKind::White => 11,
            NoiseKind::Pink => 23,
            NoiseKind::Brown => 37,
        }
    }
}

/// Seamless 2 s mono noise buffer, RMS-normalised to ~0.3 — exact port of
/// `noiseBuffer` (Float32Array storage, double-precision math).
pub fn noise_buffer(kind: NoiseKind) -> Vec<f32> {
    let sr = SR as usize;
    let len = sr * 2;
    let mut d = vec![0.0f32; len];
    let mut rnd = Rng::new(kind.seed());
    let w: Vec<f32> = (0..len)
        .map(|_| (rnd.next_f64() * 2.0 - 1.0) as f32)
        .collect();
    if kind == NoiseKind::White {
        d.copy_from_slice(&w);
    } else {
        let mut b0 = 0.0f64;
        let mut b1 = 0.0f64;
        let mut b2 = 0.0f64;
        let mut b3 = 0.0f64;
        let mut b4 = 0.0f64;
        let mut b5 = 0.0f64;
        let mut b6 = 0.0f64;
        let mut last = 0.0f64;
        for pass in 0..2 {
            for i in 0..len {
                let x = w[i] as f64;
                let y = if kind == NoiseKind::Pink {
                    b0 = 0.99886 * b0 + x * 0.0555179;
                    b1 = 0.99332 * b1 + x * 0.0750759;
                    b2 = 0.969 * b2 + x * 0.153852;
                    b3 = 0.8665 * b3 + x * 0.3104856;
                    b4 = 0.55 * b4 + x * 0.5329522;
                    b5 = -0.7616 * b5 - x * 0.016898;
                    let y = b0 + b1 + b2 + b3 + b4 + b5 + b6 + x * 0.5362;
                    // ≡ L71: b6 is the one-sample-delayed component, updated
                    // *after* y and kept live across both passes.
                    b6 = x * 0.115926;
                    y
                } else {
                    last = (last + 0.02 * x) / 1.02;
                    last
                };
                if pass == 1 {
                    d[i] = y as f32;
                }
            }
        }
        // circular DC block (one-pole HPF ~25 Hz)
        let a = (-2.0 * std::f64::consts::PI * 25.0 / SR).exp();
        let mut y = 0.0f64;
        let mut px = d[len - 1] as f64;
        let mut tmp = vec![0.0f32; len];
        for pass in 0..2 {
            for i in 0..len {
                let x = d[i] as f64;
                y = a * (y + x - px);
                px = x;
                if pass == 1 {
                    tmp[i] = y as f32;
                }
            }
        }
        d.copy_from_slice(&tmp);
    }
    let mut mean = 0.0f64;
    for &v in &d {
        mean += v as f64;
    }
    mean /= len as f64;
    let mut ss = 0.0f64;
    for x in &mut d {
        let v = (*x as f64 - mean) as f32;
        *x = v;
        ss += v as f64 * v as f64;
    }
    let g = 0.3 / (ss / len as f64).sqrt();
    for v in &mut d {
        *v = (*v as f64 * g) as f32;
    }
    d
}

// ---------------------------------------------------------------------------
// Biquad (RBJ cookbook ≡ WebAudio BiquadFilterNode)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BqKind {
    Lowpass,
    Highpass,
    Bandpass,
    /// Engine parity (`peaking` filter type); no SFX builder uses it.
    #[allow(dead_code)]
    Peaking,
}

/// Direct-form-I biquad with per-channel state supplied by the caller.
#[derive(Clone, Copy, Debug)]
pub struct BqState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BqState {
    pub fn new() -> Self {
        Self {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

impl Default for BqState {
    fn default() -> Self {
        Self::new()
    }
}

/// One biquad section: coefficients + state.
#[derive(Clone, Debug)]
pub struct Biquad {
    pub kind: BqKind,
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    st: BqState,
}

impl Biquad {
    pub fn new(kind: BqKind) -> Self {
        Self {
            kind,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            st: BqState::new(),
        }
    }

    /// Recompute coefficients (WebAudio spec table ≡ RBJ cookbook).
    /// `freq` is clamped to [10, sr/2] exactly like the JS node does.
    pub fn set(&mut self, freq: f64, q: f64, gain_db: f64, sr: f64) {
        let f = freq.clamp(10.0, sr / 2.0);
        let q = q.max(1e-6);
        let w0 = std::f64::consts::TAU * f / sr;
        let (cos_w, sin_w) = (w0.cos(), w0.sin());
        let alpha = sin_w / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            BqKind::Lowpass => (
                (1.0 - cos_w) / 2.0,
                1.0 - cos_w,
                (1.0 - cos_w) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w,
                1.0 - alpha,
            ),
            BqKind::Highpass => (
                (1.0 + cos_w) / 2.0,
                -(1.0 + cos_w),
                (1.0 + cos_w) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w,
                1.0 - alpha,
            ),
            BqKind::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w, 1.0 - alpha),
            BqKind::Peaking => {
                let a = 10f64.powf(gain_db / 40.0);
                (
                    1.0 + alpha * a,
                    -2.0 * cos_w,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos_w,
                    1.0 - alpha / a,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// Reset the filter state (used by the graph when a source restarts).
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.st = BqState::new();
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let s = &mut self.st;
        let y = self.b0 * x + self.b1 * s.x1 + self.b2 * s.x2 - self.a1 * s.y1 - self.a2 * s.y2;
        s.x2 = s.x1;
        s.x1 = x;
        s.y2 = s.y1;
        s.y1 = y;
        y
    }
}

// ---------------------------------------------------------------------------
// WaveShaper (music.js `driveCurve` L126-136)
// ---------------------------------------------------------------------------

/// `tanh` drive curve, 2049 points (odd length so 0 maps exactly to 0).
pub fn drive_curve(k: f64) -> Vec<f32> {
    let n = 2049;
    let norm = k.tanh();
    (0..n)
        .map(|i| {
            let x = (i as f64 / (n - 1) as f64) * 2.0 - 1.0;
            (k * x).tanh() / norm
        })
        .map(|v| v as f32)
        .collect()
}

/// WaveShaper curve lookup with linear interpolation (no `oversample: '2x'`,
/// declared deviation).
pub fn shaper_map(curve: &[f32], x: f64) -> f64 {
    let n = curve.len() as f64;
    let idx = (x + 1.0) / 2.0 * (n - 1.0);
    if idx <= 0.0 {
        return curve[0] as f64;
    }
    if idx >= n - 1.0 {
        return curve[curve.len() - 1] as f64;
    }
    let i0 = idx as usize;
    let f = idx - i0 as f64;
    let a = curve[i0] as f64;
    let b = curve[i0 + 1] as f64;
    a + (b - a) * f
}

// ---------------------------------------------------------------------------
// Buffer-domain chain helpers
// ---------------------------------------------------------------------------

/// Multiply a buffer by `g`.
#[allow(dead_code)]
pub fn scale(buf: &[f32], g: f32) -> Vec<f32> {
    buf.iter().map(|s| s * g).collect()
}

/// `dst[i] += src[i] * g` (length = min).
#[allow(dead_code)]
pub fn mix(dst: &mut [f32], src: &[f32], g: f32) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d += *s * g;
    }
}

/// Equal-power pan of a mono buffer to two channels, ≡ WebAudio
/// `StereoPannerNode` gains (`θ = (pan + 1) · π/4`).
#[allow(dead_code)]
pub fn pan(mono: &[f32], p: f64) -> (Vec<f32>, Vec<f32>) {
    let p = p.clamp(-1.0, 1.0);
    let th = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    let (gl, gr) = (th.cos(), th.sin());
    (
        mono.iter().map(|s| (*s as f64 * gl) as f32).collect(),
        mono.iter().map(|s| (*s as f64 * gr) as f32).collect(),
    )
}

/// Fractional-sample delay (linear interpolation ≡ WebAudio `DelayNode`).
/// Feedback loops are built as graph edges in the JS, not here.
#[allow(dead_code)]
pub fn delay(mono: &[f32], time: f64) -> Vec<f32> {
    let d = (time * SR).max(1.0);
    let mut out: Vec<f32> = Vec::with_capacity(mono.len());
    for i in 0..mono.len() {
        let r = i as f64 - d;
        if r < 0.0 {
            out.push(0.0);
            continue;
        }
        let k = r.floor() as usize;
        let f = r - k as f64;
        let a = mono[k] as f64;
        let b = mono.get(k + 1).map_or(0.0, |v| *v as f64);
        out.push((a + (b - a) * f) as f32);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests (batch A)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Rng: bit-exact mulberry32 (reference values computed from the JS) --
    #[test]
    fn rng_matches_mulberry32_reference() {
        let mut r = Rng::new(11);
        assert!((r.next_f64() - 0.5115870486479253).abs() < 1e-15);
        assert!((r.next_f64() - 0.5299464082345366).abs() < 1e-15);
        assert!((r.next_f64() - 0.6081185641232878).abs() < 1e-15);
        assert!((r.next_f64() - 0.5901576359756291).abs() < 1e-15);
        let mut r = Rng::new(23);
        assert!((r.next_f64() - 0.09272266505286098).abs() < 1e-15);
        let mut r = Rng::new(37);
        assert!((r.next_f64() - 0.5163109616842121).abs() < 1e-15);
        assert!((r.next_f64() - 0.9970917752943933).abs() < 1e-15);
        // deterministic: same seed ⇒ same stream
        let mut a = Rng::new(71);
        let mut b = Rng::new(71);
        for _ in 0..64 {
            assert_eq!(a.next_f64(), b.next_f64());
        }
    }

    #[test]
    fn mtof_reference() {
        assert!((mtof(69.0) - 440.0).abs() < 1e-12);
        assert!((mtof(60.0) - 261.6255653005986).abs() < 1e-9);
        assert!((mtof(84.0) - mtof(72.0) * 2.0).abs() < 1e-9);
    }

    // ---- Param automation ----
    #[test]
    fn param_automation_semantics() {
        let mut p = Param::new(0.0);
        p.set(1.0, 0.0);
        p.lin(2.0, 1.0);
        assert_eq!(p.value_at(-1.0), 0.0); // default before first event
        assert_eq!(p.value_at(0.0), 1.0);
        assert!((p.value_at(0.5) - 1.5).abs() < 1e-12); // linear midpoint
        assert_eq!(p.value_at(1.0), 2.0);
        assert_eq!(p.value_at(5.0), 2.0); // hold after last
        p.exp(8.0, 2.0);
        // exponential midpoint = geometric mean
        assert!((p.value_at(1.5) - (2.0f64 * 8.0f64).sqrt()).abs() < 1e-12);
        assert!((p.value_at(2.0) - 8.0).abs() < 1e-12);
        p.target(0.0, 3.0, 1.0);
        assert!((p.value_at(3.0) - 8.0).abs() < 1e-12); // starts from prior value
        assert!((p.value_at(4.0) - 8.0 * (-1.0f64).exp()).abs() < 1e-9);
    }

    // ---- Envelopes: segment endpoints ≡ the JS helper formulas ----
    #[test]
    fn perc_endpoints() {
        let mut p = Param::new(0.0);
        let end = perc(&mut p, 1.0, 0.01, 0.5, 0.2);
        assert!((end - (1.0 + 0.01 + 0.2 + 0.008)).abs() < 1e-12);
        assert_eq!(p.value_at(1.0), 0.0);
        assert!((p.value_at(1.01) - 0.5).abs() < 1e-12);
        assert!((p.value_at(1.21) - 0.0005).abs() < 1e-12); // peak*0.001
        assert_eq!(p.value_at(end), 0.0);
    }

    #[test]
    fn ahr_and_adsr_endpoints() {
        let mut p = Param::new(0.0);
        let end = ahr(&mut p, 0.0, 0.002, 0.4, 0.1, 0.05);
        assert!((end - (0.002 + 0.1 + 0.05 + 0.008)).abs() < 1e-12);
        assert!((p.value_at(0.002) - 0.4).abs() < 1e-12);
        assert!((p.value_at(0.05) - 0.4).abs() < 1e-12); // hold through h
        let mut q = Param::new(0.0);
        let e2 = adsr(&mut q, 0.0, 0.01, 0.2, 0.5, 1.0, 0.5, 0.1);
        // gate 0.5 > a+d = 0.21 → full decay to sus then hold
        assert!((q.value_at(0.21) - 0.5).abs() < 1e-9);
        assert!((q.value_at(0.4) - 0.5).abs() < 1e-9);
        assert!((e2 - (0.5 + 0.1 + 0.008)).abs() < 1e-12);
    }

    #[test]
    fn pts_and_sweep() {
        let mut p = Param::new(0.0);
        let end = pts(&mut p, 0.0, &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]);
        assert!((end - 1.0).abs() < 1e-12);
        assert!((p.value_at(0.25) - 0.5).abs() < 1e-12);
        let mut f = Param::new(100.0);
        sweep(&mut f, 0.0, 100.0, 1000.0, 1.0, false);
        assert!((f.value_at(0.5) - (100.0f64 * 1000.0f64).sqrt()).abs() < 1e-6);
        let mut f2 = Param::new(100.0);
        sweep(&mut f2, 0.0, 100.0, 1000.0, 1.0, true);
        assert!((f2.value_at(0.5) - 550.0).abs() < 1e-6);
    }

    // ---- Oscillator ----
    #[test]
    fn sine_one_cycle() {
        let mut o = Osc::new(Wave::Sine);
        let f = SR / 8.0; // 8 samples per cycle
        let mut v = [0.0; 8];
        for s in v.iter_mut() {
            *s = o.tick(Wave::Sine, f);
        }
        assert!(v[0].abs() < 1e-12); // phase 0
        assert!((v[2] - 1.0).abs() < 1e-6); // quarter cycle
        assert!(v[4].abs() < 1e-6); // half
        assert!((v[6] + 1.0).abs() < 1e-6); // three quarters
        // one full cycle later the phase repeats
        assert!((o.tick(Wave::Sine, f) - v[0]).abs() < 1e-9);
    }

    #[test]
    fn naive_shapes_reference_values() {
        let mut o = Osc::new(Wave::Square);
        let f = SR / 4.0; // 4 samples per cycle
        assert!((o.tick(Wave::Square, f) - 1.0).abs() < 1e-12);
        assert!((o.tick(Wave::Square, f) - 1.0).abs() < 1e-12);
        assert!((o.tick(Wave::Square, f) + 1.0).abs() < 1e-12);
        assert!((o.tick(Wave::Square, f) + 1.0).abs() < 1e-12);
        let mut s = Osc::new(Wave::Saw);
        assert!((s.tick(Wave::Saw, f) + 1.0).abs() < 1e-12); // phase 0 → -1
        assert!((s.tick(Wave::Saw, f) - (-0.5)).abs() < 1e-12);
        let mut t = Osc::new(Wave::Triangle);
        assert!((t.tick(Wave::Triangle, f) + 1.0).abs() < 1e-12);
        assert!((t.tick(Wave::Triangle, f) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn pulse_wave_is_band_limited_and_normalized() {
        // duty 0.25: first-harmonic coefficients ≡ sin(π/2)/π, (1-cos(π/2))/π
        let h = pulse_coeffs(0.25);
        assert!((h[0].0 - 1.0 / std::f64::consts::PI).abs() < 1e-12);
        assert!((h[0].1 - 1.0 / std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(h.len(), 48);
        let mut o = Osc::new(Wave::Pulse(0.25));
        let f = SR / 64.0;
        let mut pk: f64 = 0.0;
        for _ in 0..(64 * 4) {
            pk = pk.max(o.tick(Wave::Pulse(0.25), f).abs());
        }
        assert!((pk - 1.0).abs() < 0.1, "normalized peak {pk}");
    }

    #[test]
    fn osc_is_deterministic() {
        let mut a = Osc::new(Wave::Pulse(0.3));
        let mut b = Osc::new(Wave::Pulse(0.3));
        for i in 0..1000 {
            let f = 300.0 + (i % 97) as f64;
            assert_eq!(a.tick(Wave::Pulse(0.3), f), b.tick(Wave::Pulse(0.3), f));
        }
    }

    // ---- Biquad ----
    #[test]
    fn biquad_dc_gains() {
        let mut lp = Biquad::new(BqKind::Lowpass);
        lp.set(1000.0, 0.707, 0.0, SR);
        let mut y = 0.0;
        for _ in 0..2000 {
            y = lp.process(1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "lowpass DC gain {y}");
        let mut hp = Biquad::new(BqKind::Highpass);
        hp.set(1000.0, 0.707, 0.0, SR);
        let mut y = 0.0;
        for _ in 0..2000 {
            y = hp.process(1.0);
        }
        assert!(y.abs() < 1e-3, "highpass DC gain {y}");
    }

    #[test]
    fn biquad_bandpass_peak_gain_is_unity() {
        // WebAudio bandpass (b0 = α, a0 = 1+α) has |H(f0)| = 1/(1+α) ≈ 1.
        let q = 4.0;
        let mut bp = Biquad::new(BqKind::Bandpass);
        bp.set(1000.0, q, 0.0, SR);
        let w = std::f64::consts::TAU * 1000.0 / SR;
        let mut ph: f64 = 0.0;
        let mut out = vec![];
        for _ in 0..4000 {
            out.push(bp.process(ph.sin()));
            ph += w;
        }
        let amp = out[3000..].iter().fold(0.0f64, |m, s| m.max(s.abs()));
        assert!((amp - 1.0).abs() < 0.02, "bandpass peak gain {amp}");
        // and strong rejection away from f0
        let mut bp2 = Biquad::new(BqKind::Bandpass);
        bp2.set(1000.0, q, 0.0, SR);
        let w2 = std::f64::consts::TAU * 4000.0 / SR;
        let mut ph2: f64 = 0.0;
        let mut out2 = vec![];
        for _ in 0..4000 {
            out2.push(bp2.process(ph2.sin()));
            ph2 += w2;
        }
        let amp2 = out2[3000..].iter().fold(0.0f64, |m, s| m.max(s.abs()));
        assert!(amp2 < 0.4, "bandpass 4 kHz rejection {amp2}");
    }

    #[test]
    fn biquad_peaking_boosts_at_center() {
        let mut pk = Biquad::new(BqKind::Peaking);
        pk.set(1000.0, 1.2, 5.0, SR);
        let w = std::f64::consts::TAU * 1000.0 / SR;
        let mut ph: f64 = 0.0;
        let mut out = vec![];
        for _ in 0..4000 {
            out.push(pk.process(ph.sin()));
            ph += w;
        }
        let amp = out[3000..].iter().fold(0.0f64, |m, s| m.max(s.abs()));
        // +5 dB ≈ 1.78 gain at the peak frequency
        assert!(
            (amp - 10f64.powf(5.0 / 20.0)).abs() < 0.05,
            "peaking gain {amp}"
        );
    }

    // ---- Noise buffers ----
    #[test]
    fn noise_buffer_shape_and_determinism() {
        for kind in [NoiseKind::White, NoiseKind::Pink, NoiseKind::Brown] {
            let a = noise_buffer(kind);
            let b = noise_buffer(kind);
            assert_eq!(a, b, "{kind:?} must render byte-identical");
            assert_eq!(a.len(), SR as usize * 2);
            let mean: f64 = a.iter().map(|v| *v as f64).sum::<f64>() / a.len() as f64;
            assert!(mean.abs() < 1e-3, "{kind:?} mean {mean}");
            let rms = (a.iter().map(|v| (v * v) as f64).sum::<f64>() / a.len() as f64).sqrt();
            assert!((rms - 0.3).abs() < 0.01, "{kind:?} rms {rms}");
        }
    }

    #[test]
    fn pink_noise_is_darker_than_white() {
        // crude spectral proxy: sign-change rate (white ≈ 0.5/sample, pink lower)
        let zc = |b: &[f32]| {
            b.iter()
                .zip(b[1..].iter())
                .filter(|(a, c)| (**a >= 0.0) != (**c >= 0.0))
                .count() as f64
                / b.len() as f64
        };
        let w = zc(&noise_buffer(NoiseKind::White));
        let p = zc(&noise_buffer(NoiseKind::Pink));
        assert!(w > 0.45 && p < 0.3, "white {w} pink {p}");
    }

    #[test]
    fn pink_noise_samples_match_js() {
        // Task 50 review P0-1: the Paul Kellet `b6` one-sample-delay term must
        // stay live across samples/passes. Reference = the first 16 samples of
        // `noiseBuffer(ctx, 'pink')` (music.js L48-95) run through node with
        // sr = 44100 — pinned here so the IIR state cannot silently drop again.
        const REF: [f64; 16] = [
            -0.405_756_443_738_937_4,
            -0.546_209_096_908_569_3,
            -0.274_587_601_423_263_55,
            -0.358_520_030_975_341_8,
            -0.397_375_285_625_457_76,
            -0.489_416_271_448_135_4,
            -0.359_055_131_673_812_87,
            -0.004_543_427_377_939_224,
            0.114_546_500_146_389_01,
            -0.324_344_724_416_732_8,
            -0.095_720_700_919_628_14,
            -0.291_923_671_960_830_7,
            -0.496_889_293_193_817_14,
            -0.025_288_749_486_207_962,
            0.127_281_233_668_327_33,
            -0.011_127_717_792_987_823,
        ];
        let b = noise_buffer(NoiseKind::Pink);
        for (i, r) in REF.iter().enumerate() {
            assert!(
                (f64::from(b[i]) - r).abs() < 1e-6,
                "pink[{i}] = {} vs JS {r}",
                b[i]
            );
        }
    }

    // ---- WaveShaper ----
    #[test]
    fn drive_curve_reference() {
        let c = drive_curve(4.0);
        assert_eq!(c.len(), 2049);
        assert_eq!(c[1024], 0.0); // odd length: centre maps exactly to 0
        assert!((c[2048] as f64 - 1.0).abs() < 1e-6);
        assert!((c[0] as f64 + 1.0).abs() < 1e-6);
        assert!((shaper_map(&c, 0.0) - 0.0).abs() < 1e-6);
        assert!((shaper_map(&c, 2.0) - 1.0).abs() < 1e-6); // clamps above 1
        let soft = shaper_map(&c, 0.5);
        let hard = shaper_map(&c, 0.6);
        // tanh(4x)/tanh(4) at x=0.5 ≈ 0.965: compressive, monotonic
        assert!(
            soft > 0.5 && hard > soft && hard <= 1.0,
            "soft {soft} hard {hard}"
        );
    }

    // ---- buffer helpers ----
    #[test]
    fn pan_mix_scale_delay() {
        let m = vec![1.0f32; 4];
        let (l, r) = pan(&m, 0.0);
        let c = std::f64::consts::FRAC_1_SQRT_2 as f32;
        assert!(l.iter().all(|v| (*v - c).abs() < 1e-6));
        assert!(r.iter().all(|v| (*v - c).abs() < 1e-6));
        let (l, r) = pan(&m, -1.0);
        assert!(l.iter().all(|v| (*v - 1.0).abs() < 1e-6));
        assert!(r.iter().all(|v| v.abs() < 1e-6));
        let mut dst = vec![0.5f32; 4];
        mix(&mut dst, &m, 0.5);
        assert!(dst.iter().all(|v| (*v - 1.0).abs() < 1e-6));
        let s = scale(&m, 0.25);
        assert!(s.iter().all(|v| (*v - 0.25).abs() < 1e-6));
        // integer-sample delay of an impulse
        let mut imp = vec![0.0f32; 100];
        imp[10] = 1.0;
        let d = delay(&imp, 2.0 / SR);
        assert!((d[12] - 1.0).abs() < 1e-3);
        assert!(d.iter().enumerate().all(|(i, v)| i == 12 || v.abs() < 1e-3));
    }
}
