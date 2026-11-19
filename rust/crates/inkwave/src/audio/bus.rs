//! M5 Task 63 — the live audio bus, spatial send and plaza reverb facade.
//!
//! All scalar parameters and the 11-edge topology come from the pinned
//! `assets/audio/bus.json` (extracted from the READ-ONLY `audio.js` /
//! `music.js`); `tools/audio-bus-crosscheck.mjs` independently re-parses the
//! upstream sources. Bevy 0.19 has no WebAudio node-graph injection point, so
//! the Rust port applies the same topology offline when a voice is spawned:
//! mono cue → distance low-pass → inverse gain → equal-power pan (dry) and
//! → reverb send → compact plaza tail → `sfxBus` return (wet), then a soft
//! glue/limiter stage. HRTF, full 1.5 s convolution and WebAudio envelope
//! dynamics are declared deviations (D-T63-1..3) rather than faked parity.

use std::sync::OnceLock;

use inkwave_sim::net::wire::{JVal, parse_json};

use super::synth::Rng;

const DOC: &str = include_str!("../../../../assets/audio/bus.json");

fn doc() -> &'static JVal {
    static VAL: OnceLock<JVal> = OnceLock::new();
    VAL.get_or_init(|| parse_json(DOC).expect("bus.json parses"))
}

fn field<'a>(v: &'a JVal, key: &str) -> &'a JVal {
    v.field(key)
        .unwrap_or_else(|| panic!("bus.json: missing field {key}"))
}

fn num(v: &JVal, key: &str) -> f64 {
    field(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("bus.json: missing numeric field {key}"))
}

fn text(v: &JVal, key: &str) -> String {
    field(v, key)
        .as_str()
        .unwrap_or_else(|| panic!("bus.json: missing string field {key}"))
        .to_string()
}

/// WebAudio `DynamicsCompressor` descriptor (all five fields pinned from JS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compressor {
    pub threshold: f64,
    pub knee: f64,
    pub ratio: f64,
    pub attack: f64,
    pub release: f64,
}

/// Every bus parameter the Rust facade consumes.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioSpec {
    pub source_commit: String,
    pub taper_exponent: f64,
    pub sfx_in: f64,
    pub rev_send: f64,
    pub rev_return: f64,
    pub highpass_hz: f64,
    pub highpass_q: f64,
    pub glue: Compressor,
    pub limiter: Compressor,
    pub edges: Vec<(String, String)>,
    pub panning_model: String,
    pub fallback_model: String,
    pub distance_model: String,
    pub ref_distance: f64,
    pub max_distance_min: f64,
    pub max_distance_factor: f64,
    pub rolloff: f64,
    pub cone_inner: f64,
    pub cone_outer: f64,
    pub dist_cut_near: f64,
    pub dist_cut_ceiling: f64,
    pub dist_cut_floor: f64,
    pub dist_cut_half_life: f64,
    pub dist_cut_q: f64,
    pub send_numerator: f64,
    pub send_base: f64,
    pub send_rolloff: f64,
    pub send_ref: f64,
    pub reverb_extra_cap: f64,
    pub reverb_extra_start: f64,
    pub reverb_extra_divisor: f64,
    pub default_reverb: f64,
    pub impulse: ImpulseSpec,
}

/// Deterministic plaza IR descriptor ≡ `music.js` L98-122 + `audio.js` L124.
#[derive(Clone, Debug, PartialEq)]
pub struct ImpulseSpec {
    pub seconds: f64,
    pub decay: f64,
    pub pre: f64,
    pub seed: u32,
    pub bright: f64,
    pub dark: f64,
    pub decay_multiplier: f64,
    pub attack_seconds: f64,
    pub darken_rate: f64,
    pub channel_seed_step: u32,
    pub taps: Vec<(f64, f64)>,
    pub tap_tail_gain: f64,
    pub right_offset: f64,
}

pub fn spec() -> &'static AudioSpec {
    static VAL: OnceLock<AudioSpec> = OnceLock::new();
    VAL.get_or_init(|| {
        let root = doc();
        let source = field(root, "source");
        let graph = field(root, "graph");
        let spatial = field(root, "spatial");
        let impulse = field(root, "impulse");
        let cut = field(spatial, "dist_cut");
        let send = field(spatial, "send");
        let extra = field(spatial, "reverb_distance");
        let comp = |key: &str| {
            let v = field(graph, key);
            Compressor {
                threshold: num(v, "threshold"),
                knee: num(v, "knee"),
                ratio: num(v, "ratio"),
                attack: num(v, "attack"),
                release: num(v, "release"),
            }
        };
        AudioSpec {
            source_commit: text(source, "commit"),
            taper_exponent: num(graph, "taper_exponent"),
            sfx_in: num(graph, "sfx_in"),
            rev_send: num(graph, "rev_send"),
            rev_return: num(graph, "rev_return"),
            highpass_hz: num(field(graph, "highpass"), "frequency"),
            highpass_q: num(field(graph, "highpass"), "q"),
            glue: comp("glue"),
            limiter: comp("limiter"),
            edges: field(graph, "edges")
                .as_array()
                .expect("graph.edges")
                .iter()
                .map(|e| (text(e, "from"), text(e, "to")))
                .collect(),
            panning_model: text(spatial, "panning_model"),
            fallback_model: text(spatial, "fallback_model"),
            distance_model: text(spatial, "distance_model"),
            ref_distance: num(spatial, "ref_distance"),
            max_distance_min: num(spatial, "max_distance_min"),
            max_distance_factor: num(spatial, "max_distance_factor"),
            rolloff: num(spatial, "rolloff"),
            cone_inner: num(spatial, "cone_inner"),
            cone_outer: num(spatial, "cone_outer"),
            dist_cut_near: num(cut, "near"),
            dist_cut_ceiling: num(cut, "ceiling"),
            dist_cut_floor: num(cut, "floor"),
            dist_cut_half_life: num(cut, "half_life"),
            dist_cut_q: num(cut, "q"),
            send_numerator: num(send, "numerator"),
            send_base: num(send, "base"),
            send_rolloff: num(send, "rolloff"),
            send_ref: num(send, "ref"),
            reverb_extra_cap: num(extra, "cap"),
            reverb_extra_start: num(extra, "start"),
            reverb_extra_divisor: num(extra, "divisor"),
            default_reverb: num(spatial, "default_reverb"),
            impulse: ImpulseSpec {
                seconds: num(impulse, "seconds"),
                decay: num(impulse, "decay"),
                pre: num(impulse, "pre"),
                seed: num(impulse, "seed") as u32,
                bright: num(impulse, "bright"),
                dark: num(impulse, "dark"),
                decay_multiplier: num(impulse, "decay_multiplier"),
                attack_seconds: num(impulse, "attack_seconds"),
                darken_rate: num(impulse, "darken_rate"),
                channel_seed_step: num(impulse, "channel_seed_step") as u32,
                taps: field(impulse, "taps")
                    .as_array()
                    .expect("impulse.taps")
                    .iter()
                    .map(|t| (num(t, "time"), num(t, "amp")))
                    .collect(),
                tap_tail_gain: num(impulse, "tap_tail_gain"),
                right_offset: num(impulse, "right_offset"),
            },
        }
    })
}

/// Bit-exact port of `makeImpulse` (music.js L98-122) for any sample rate.
/// The full 1.5 s IR is baked here for determinism anchoring; live voices use
/// the compact tail derived from the same taps (D-T63-2).
// Full FIR is used by the deterministic IR anchor; the live path uses the
// extracted-tap tail below (D-T63-2) to stay O(n) per voice.
#[allow(dead_code)]
#[must_use]
pub fn bake_ir(s: &ImpulseSpec, sample_rate: u32) -> [Vec<f32>; 2] {
    let sr = sample_rate as f64;
    let len = (sr * s.seconds).floor().max(1.0) as usize;
    let pre = (s.pre * sr).floor() as usize;
    let denom = (len - pre).max(1) as f64;
    let mut out = [vec![0.0; len], vec![0.0; len]];
    for (ch, data) in out.iter_mut().enumerate() {
        let mut rng = Rng::new(s.seed + ch as u32 * s.channel_seed_step);
        let mut lp = 0.0f64;
        for (k, sample) in data[pre..len].iter_mut().enumerate() {
            let x = k as f64 / denom;
            let attack = (k as f64 / (s.attack_seconds * sr).max(1.0)).min(1.0);
            let env = (-x * s.decay * s.decay_multiplier).exp() * (1.0 - x) * attack;
            let a = s.dark + (s.bright - s.dark) * (-x * s.darken_rate).exp();
            lp += (rng.next_f64() * 2.0 - 1.0 - lp) * a;
            *sample = (lp * env) as f32;
        }
        for &(seconds, amp) in &s.taps {
            let offset = seconds + if ch == 1 { s.right_offset } else { 0.0 };
            let i = pre + (offset * sr).floor() as usize;
            if i < len.saturating_sub(2) {
                data[i] += (if ch == 1 { -amp } else { amp }) as f32;
                data[i + 1] += (amp * s.tap_tail_gain) as f32;
            }
        }
    }
    out
}

/// Listener/source pose for one voice. `forward`/`up` are world-space unit
/// vectors from the main camera (JS `setListener`, audio.js L174-190).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    pub listener: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub source: [f32; 3],
    pub ref_distance: f64,
}

impl Spatial {
    #[must_use]
    pub fn distance(&self) -> f64 {
        let d = [
            self.source[0] - self.listener[0],
            self.source[1] - self.listener[1],
            self.source[2] - self.listener[2],
        ];
        f64::from(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if n < 1e-6 {
        return [0.0, 0.0, 1.0];
    }
    [a[0] / n, a[1] / n, a[2] / n]
}

/// ≡ `distCut` (audio.js L30): near field stays unfiltered, far field falls
/// to the pinned 900 Hz floor with the 13 m halving distance.
#[must_use]
pub fn distance_cut(d: f64, s: &AudioSpec) -> f64 {
    if d < s.dist_cut_near {
        s.dist_cut_ceiling
    } else {
        s.dist_cut_floor
            .max(s.dist_cut_ceiling * 0.5f64.powf((d - s.dist_cut_near) / s.dist_cut_half_life))
    }
}

/// ≡ `_sendScale` (audio.js L216).
#[must_use]
pub fn send_scale(d: f64, s: &AudioSpec) -> f64 {
    (s.send_numerator / (s.send_base + s.send_rolloff * (d - s.send_ref).max(0.0))).sqrt()
}

/// ≡ WebAudio inverse distance model with the pinned ref/rolloff (L199).
#[must_use]
pub fn distance_gain(d: f64, sp: &Spatial, s: &AudioSpec) -> f64 {
    let r = sp.ref_distance.max(1e-3);
    r / (r + s.rolloff * (d - r).max(0.0))
}

/// Equal-power pan (D-T63-1: HRTF is not available in Bevy 0.19).
#[must_use]
pub fn pan(sp: &Spatial) -> (f64, f64) {
    let right = normalize(cross(sp.forward, sp.up));
    let dir = normalize([
        sp.source[0] - sp.listener[0],
        sp.source[1] - sp.listener[1],
        sp.source[2] - sp.listener[2],
    ]);
    let x = (f64::from(dir[0]) * f64::from(right[0])
        + f64::from(dir[1]) * f64::from(right[1])
        + f64::from(dir[2]) * f64::from(right[2]))
    .clamp(-1.0, 1.0);
    let theta = (x + 1.0) * std::f64::consts::FRAC_PI_4;
    (theta.cos(), theta.sin())
}

/// Static compressor curve in dB (D-T63-3: WebAudio attack/release envelopes
/// are not reproduced). Values above threshold are reduced by the ratio knee.
#[must_use]
pub fn compressor_gain(x: f64, c: &Compressor) -> f64 {
    let a = x.abs();
    if a <= 1e-9 {
        return 1.0;
    }
    let db = 20.0 * a.log10();
    let over = (db - c.threshold).max(0.0);
    let reduced = db - over * (1.0 - 1.0 / c.ratio.max(1.0));
    10f64.powf((reduced - db) / 20.0)
}

/// A voice after the Task63 bus: interleaved stereo PCM plus its channel count.
#[derive(Clone, Debug, PartialEq)]
pub struct MixedVoice {
    pub data: Vec<f32>,
    pub channels: u16,
}

/// One ear of the sparse plaza tail: a bank of feedback combs, one per extracted
/// early-reflection tap. The right ear uses `right_offset` and inverted tap
/// polarity, mirroring the baked IR (`music.js` L117-118).
struct EarTail {
    delays: Vec<usize>,
    bufs: Vec<Vec<f32>>,
    idx: Vec<usize>,
    feedback: Vec<f64>,
    polarity: f64,
}

impl EarTail {
    fn new(s: &AudioSpec, sample_rate: u32, distance: f64, right: bool) -> Self {
        let sr = sample_rate as f64;
        let pre = (s.impulse.pre * sr) as usize;
        let offset = if right { s.impulse.right_offset } else { 0.0 };
        let polarity = if right { -1.0 } else { 1.0 };
        let mut delays = Vec::new();
        let mut feedback = Vec::new();
        for (t, amp) in &s.impulse.taps {
            let d = pre + ((t + offset) * sr) as usize;
            delays.push(d.max(1));
            feedback.push((amp * 0.9).clamp(0.05, 0.72));
        }
        let air = 1.0 - (distance / 220.0).clamp(0.0, 0.55);
        for f in &mut feedback {
            *f *= air;
        }
        Self {
            bufs: delays.iter().map(|&d| vec![0.0; d]).collect(),
            idx: vec![0; delays.len()],
            delays,
            feedback,
            polarity,
        }
    }

    fn push(&mut self, x: f64) -> f64 {
        let mut acc = 0.0;
        for i in 0..self.delays.len() {
            let d = self.delays[i];
            let y = self.bufs[i][self.idx[i]];
            self.bufs[i][self.idx[i]] = (x + f64::from(y) * self.feedback[i]) as f32;
            self.idx[i] = (self.idx[i] + 1) % d;
            acc += f64::from(y);
        }
        acc / self.delays.len() as f64 * self.polarity
    }
}

/// Sparse stereo plaza tail derived from the extracted IR taps. O(n) per voice
/// (vs. O(n·m) for the 1.5 s FIR), keeping the pre-delay, per-ear polarity and
/// right-offset de-correlation of the baked IR (D-T63-2).
struct Tail {
    left: EarTail,
    right: EarTail,
}

impl Tail {
    fn new(s: &AudioSpec, sample_rate: u32, distance: f64) -> Self {
        Self {
            left: EarTail::new(s, sample_rate, distance, false),
            right: EarTail::new(s, sample_rate, distance, true),
        }
    }

    fn push(&mut self, x: f64) -> (f64, f64) {
        (self.left.push(x), self.right.push(x))
    }

    fn tail_len(&self) -> usize {
        self.left
            .delays
            .iter()
            .chain(self.right.delays.iter())
            .copied()
            .max()
            .unwrap_or(0)
            * 2
    }
}

/// Apply the pinned bus to one cached mono cue. `reverb` is the cue's extracted
/// send amount (kit table) or the JS default 0.08 (D-T63-4).
#[must_use]
pub fn mix_voice(
    mono: &[f32],
    reverb: f64,
    spatial: Option<Spatial>,
    sample_rate: u32,
    s: &AudioSpec,
) -> MixedVoice {
    let (pan_l, pan_r) = spatial.map(|sp| pan(&sp)).unwrap_or((
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ));
    let (distance, gain, cut, send) = spatial.map_or((0.0, 1.0, s.dist_cut_ceiling, 0.0), |sp| {
        let d = sp.distance();
        (
            d,
            distance_gain(d, &sp, s),
            distance_cut(d, s),
            send_scale(d, s),
        )
    });
    let send_amount = (reverb + extra_reverb(distance, s)) * send;
    let mut tail = Tail::new(s, sample_rate, distance);
    let alpha = 1.0 - (-2.0 * std::f64::consts::PI * cut / sample_rate as f64).exp();
    let mut lp = 0.0f64;
    let tail_len = tail.tail_len();
    let mut data = Vec::with_capacity((mono.len() + tail_len) * 2);
    let glue_gain = compressor_gain(0.7, &s.glue);
    let lim_gain = compressor_gain(0.95, &s.limiter);
    for &sample in mono {
        let input = f64::from(sample);
        lp += (input - lp) * alpha;
        // dry ≡ JS `head→lp→panner→sfxIn`: distance low-pass + inverse gain
        // then voice pan. wet ≡ `head→send→revSend→conv→revRet→sfxBus`: sampled
        // from the PRE-low-pass head, no voice gain/pan (sendScale already
        // carries distance), returned in the convolver's stereo field.
        let dry = lp * gain;
        let (wet_l, wet_r) = tail.push(input * send_amount);
        let wet_l = wet_l * s.rev_return;
        let wet_r = wet_r * s.rev_return;
        let l = dry * pan_l + wet_l;
        let r = dry * pan_r + wet_r;
        data.push((l * glue_gain * lim_gain).clamp(-1.0, 1.0) as f32);
        data.push((r * glue_gain * lim_gain).clamp(-1.0, 1.0) as f32);
    }
    for _ in 0..tail_len {
        let (wet_l, wet_r) = tail.push(0.0);
        let l = wet_l * s.rev_return;
        let r = wet_r * s.rev_return;
        data.push((l * glue_gain * lim_gain).clamp(-1.0, 1.0) as f32);
        data.push((r * glue_gain * lim_gain).clamp(-1.0, 1.0) as f32);
    }
    MixedVoice { data, channels: 2 }
}

fn extra_reverb(d: f64, s: &AudioSpec) -> f64 {
    (s.reverb_extra_cap * ((d - s.reverb_extra_start) / s.reverb_extra_divisor).clamp(0.0, 1.0))
        .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ir_spec() -> ImpulseSpec {
        let mut s = spec().impulse.clone();
        s.seconds = 0.25; // small deterministic fixture (Node reference below)
        s
    }

    #[test]
    fn task63_bus_graph_matches_extracted_topology() {
        let s = spec();
        assert_eq!(s.edges.len(), 11);
        for (from, to) in [
            ("sfxIn", "sfxBus"),
            ("revSend", "conv"),
            ("conv", "revRet"),
            ("revRet", "sfxBus"),
            ("master", "highpass"),
            ("highpass", "glue"),
            ("glue", "limiter"),
            ("limiter", "destination"),
        ] {
            assert!(
                s.edges.iter().any(|e| e.0 == from && e.1 == to),
                "missing edge {from}->{to}"
            );
        }
        assert_eq!(s.rev_send, 1.0);
        assert_eq!(s.rev_return, 0.6);
        assert_eq!(s.taper_exponent, 1.5);
        assert_eq!((s.highpass_hz, s.highpass_q), (25.0, 0.707));
        assert_eq!(
            (
                s.glue.threshold,
                s.glue.knee,
                s.glue.ratio,
                s.glue.attack,
                s.glue.release
            ),
            (-12.0, 8.0, 3.0, 0.003, 0.18)
        );
        assert_eq!(
            (
                s.limiter.threshold,
                s.limiter.knee,
                s.limiter.ratio,
                s.limiter.attack,
                s.limiter.release
            ),
            (-2.0, 0.0, 20.0, 0.001, 0.08)
        );
        assert_eq!(s.panning_model, "HRTF");
        assert_eq!(s.distance_model, "inverse");
        assert_eq!(s.default_reverb, 0.08);
    }

    /// Reference values rendered from the READ-ONLY `music.js::makeImpulse`
    /// with Node at sr=800, seconds=0.25, decay=3.4, pre=0.012, seed=3.
    #[test]
    fn task63_ir_is_bit_deterministic_and_matches_js_reference() {
        let spec = ir_spec();
        let a = bake_ir(&spec, 800);
        let b = bake_ir(&spec, 800);
        assert_eq!(a, b);
        let samples = [9usize, 10, 11, 17, 24, 30, 41, 55, 80, 120, 199];
        let want_l = [
            0.0,
            -0.422819405794,
            -0.181743457913,
            0.166266292334,
            0.674369871616,
            0.461215615273,
            0.242470741272,
            0.156265094876,
            -0.001268036896,
            -0.001020952594,
            -0.000000072595,
        ];
        let want_r = [
            0.0,
            0.541434228420,
            0.495320022106,
            -0.292455911636,
            0.297887742519,
            -0.179555535316,
            0.051969029009,
            -0.046749699861,
            -0.004698513076,
            -0.001241897233,
            -0.000000255910,
        ];
        for ((i, wl), wr) in samples.into_iter().zip(want_l).zip(want_r) {
            assert!(
                (f64::from(a[0][i]) - wl).abs() < 2e-6,
                "L[{i}] got {} want {wl}",
                a[0][i]
            );
            assert!(
                (f64::from(a[1][i]) - wr).abs() < 2e-6,
                "R[{i}] got {} want {wr}",
                a[1][i]
            );
        }
        assert!(a[0].iter().chain(&a[1]).all(|v| v.is_finite()));
    }

    #[test]
    fn task63_wet_dry_and_spatial_curves_are_pinned() {
        let s = spec();
        assert_eq!(distance_cut(0.0, s), 22000.0);
        assert_eq!(distance_cut(10.0, s), 22000.0);
        assert_eq!(distance_cut(36.0, s), 5500.0, "two 13 m halvings past 10 m");
        assert_eq!(distance_cut(100.0, s), 900.0, "far field floor");
        assert!((send_scale(3.0, s) - 1.0).abs() < 1e-9);
        assert!(send_scale(30.0, s) < send_scale(10.0, s));
        assert_eq!(s.reverb_extra_cap, 0.25);
        let sp = Spatial {
            listener: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            source: [3.0, 0.0, -3.0],
            ref_distance: 3.0,
        };
        assert!((sp.distance() - 18.0_f64.sqrt()).abs() < 1e-6);
        assert!(distance_gain(30.0, &sp, s) < distance_gain(3.0, &sp, s));
        let (l, r) = pan(&sp);
        assert!(r > l, "source to the camera-right favours the right ear");

        let mono: Vec<f32> = (0..1024)
            .map(|i| (i as f32 / 1024.0 * std::f32::consts::TAU).sin() * 0.4)
            .collect();
        let dry = mix_voice(&mono, 0.0, None, 44100, s);
        let wet = mix_voice(&mono, 0.2, Some(sp), 44100, s);
        assert_eq!(dry.channels, 2);
        let energy = |v: &[f32]| -> f64 { v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum() };
        let tail_energy = |v: &[f32]| -> f64 { energy(&v[v.len().saturating_sub(2000)..]) };
        assert_eq!(tail_energy(&dry.data), 0.0, "no send means no wet tail");
        assert!(tail_energy(&wet.data) > 0.0, "reverb send feeds the tail");
        assert!(energy(&wet.data) > 0.0 && energy(&dry.data) > 0.0);
        assert!(wet.data.iter().chain(&dry.data).all(|v| v.is_finite()));
    }

    /// Review P1-2/P1-3: the wet send is drawn from the pre-lowpass head and
    /// routed to the convolver's stereo field WITHOUT the voice's inverse
    /// distance gain or pan (audio.js L232 vs L239). A mono input with a hard
    /// right pan must still put de-correlated energy in BOTH ears' tails; the
    /// dry path keeps the pan. Baked IR has per-ear polarity + right offset.
    #[test]
    fn task63_wet_tail_is_stereo_and_pre_lowpass() {
        let s = spec();
        let mono: Vec<f32> = (0..1024)
            .map(|i| (i as f32 / 1024.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let sp = Spatial {
            listener: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            source: [30.0, 0.0, 0.0], // hard right → pan_r ≈ 1, pan_l ≈ 0
            ref_distance: 3.0,
        };
        let (pl, pr) = pan(&sp);
        assert!(pr > pl + 0.5, "source panned hard right");
        let wet = mix_voice(&mono, 0.6, Some(sp), 44100, s);
        // Tail region = everything after the dry body (2 samples/frame).
        let tail = &wet.data[mono.len() * 2..];
        assert!(!tail.is_empty(), "a wet send must extend the voice");
        let mut left_e = 0.0f64;
        let mut right_e = 0.0f64;
        let mut decorrelated = false;
        for frame in tail.as_chunks::<2>().0 {
            let (l, r) = (f64::from(frame[0]), f64::from(frame[1]));
            left_e += l * l;
            right_e += r * r;
            if (l - r).abs() > 1e-6 {
                decorrelated = true;
            }
        }
        assert!(left_e > 0.0, "wet must reach the LEFT ear despite pan_l≈0");
        assert!(right_e > 0.0, "wet must reach the RIGHT ear");
        assert!(
            decorrelated,
            "per-ear polarity/offset de-correlates the tail"
        );
    }
}
