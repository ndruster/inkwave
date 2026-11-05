//! Task 50 batch B — the full SFX table, translated name-by-name from
//! `src/audio/audio.js` (`SFX` object, L399-2478; 148 entries in 11 groups,
//! verified 1:1 against `SFX_GROUPS` L2479-2501).
//!
//! Pipeline (spec D-T50-1): SFX name → build/loop function on [`Vb`] →
//! [`Graph`] offline evaluation → mono f32 PCM @ 44.1 kHz, baked once and
//! handed to the existing `PcmSound` / `AudioPlayer` plumbing.
//!
//! Play semantics ≡ `AudioEngine.play()` (audio.js L258-304): minGap merge,
//! per-name voice cap (oldest stolen), global `MAX_VOICES` cap, pitch jitter.
//! Deviation D-T50-6①: pitch/jitter are applied post-bake as a playback
//! *rate* multiplier (rodio `speed`) instead of scaling the synthesis — for
//! a baked sample this is the exact WebAudio-equivalent of playing the
//! nominal-pitch render faster/slower.
//!
//! Declared deviations (D-T50-6 additions):
//!   * `reverb` send / 3-D panner / distance low-pass are live-bus features
//!     (audio.js L15-18) and are NOT baked; the `gain` field is applied as
//!     the play volume, `minGap`/`max`/`jitter`/`oneShot` drive the gate;
//!   * JS draws randomness from a shared `Math.random` stream, so a JS bake
//!     is not reproducible; Rust seeds a fresh `mulberry32` per bake from a
//!     fixed hash of the name → byte-stable assets;
//!   * voice *steal* in JS is a 25 ms fade-out (`_steal` L244-250); the Rust
//!     gate despawns the stolen voice immediately (count semantics identical,
//!     the click is inaudible under the replacement).

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use std::collections::HashMap;
use std::sync::Arc;

use super::graph::{NodeId, Nz, ParamRef, Tone, Vb};
use super::synth::{self, BqKind, NoiseKind, Rng, SR, Wave};

/// ≡ `MAX_VOICES` (audio.js L26).
pub const MAX_VOICES: usize = 48;
/// ≡ `d.minGap ?? 0.018` (L267).
pub const MIN_GAP_DEFAULT: f64 = 0.018;
/// ≡ `d.max ?? 6` (L275).
pub const MAX_DEFAULT: usize = 6;
/// ≡ `d.jitter ?? 0.06` (L280).
pub const JITTER_DEFAULT: f64 = 0.06;
/// ≡ `d.gain ?? 0.5` (L268).
pub const GAIN_DEFAULT: f64 = 0.5;
/// ≡ `d.oneShot ?? 1.2` (L287).
pub const ONE_SHOT_DEFAULT: f64 = 1.2;

/// Bake a loop def as a CLEAN repeating segment — `build_loop` only, no
/// burst envelope (the JS `_loop` path plays the loop graph continuously and
/// fades in/out on start/stop; the envelope in [`bake`] belongs to one-shot
/// hits). Returns `None` for unknown or non-loop names.
#[must_use]
pub fn bake_loop(name: &str) -> Option<Vec<f32>> {
    let m = meta(name)?;
    if !m.is_loop {
        return None;
    }
    let len = m.one_shot.unwrap_or(ONE_SHOT_DEFAULT);
    let mut v = Vb::new(0.0, Rng::new(bake_seed(name)));
    build_loop(name, &mut v, 1.0);
    v.t = len;
    Some(v.bake())
}

/// One-shot / loop metadata ≡ the fields of a `def(name, {...})` entry.
#[derive(Clone, Copy, Debug)]
pub struct SfxMeta {
    pub gain: f64,
    pub max: usize,
    pub jitter: f64,
    pub min_gap: f64,
    /// burst length for loop sounds fired through `play()` (L287)
    pub one_shot: Option<f64>,
    /// true when the entry defines `loop(v, p)` instead of `build(v, p)`
    pub is_loop: bool,
}

impl Default for SfxMeta {
    fn default() -> Self {
        Self {
            gain: GAIN_DEFAULT,
            max: MAX_DEFAULT,
            jitter: JITTER_DEFAULT,
            min_gap: MIN_GAP_DEFAULT,
            one_shot: None,
            is_loop: false,
        }
    }
}

/// `SFX_GROUPS` ≡ audio.js L2479-2501 — 11 groups, 148 unique names.
pub const SFX_GROUPS: &[(&str, &[&str])] = &[
    (
        "UI",
        &[
            "ui_hover",
            "ui_click",
            "ui_back",
            "ui_confirm",
            "ui_toggle",
            "ui_slider",
            "ui_error",
        ],
    ),
    (
        "Weapons",
        &[
            "shoot_shooter",
            "shoot_blaster",
            "blaster_pump",
            "blaster_boom",
            "charger_charge",
            "charger_full",
            "shoot_charger",
            "roller_flick",
            "roll",
            "shoot_dualies",
            "dualies_roll",
            "slosh_throw",
            "slosh_land",
            "splatling_spin",
            "splatling_ready",
            "shoot_splatling",
            "splatling_wind",
        ],
    ),
    (
        "Ink",
        &[
            "splat_small",
            "splat_big",
            "ink_hit_wall",
            "bomb_throw",
            "bomb_beep",
            "bomb_explode",
        ],
    ),
    (
        "Squid",
        &[
            "squid_in",
            "squid_out",
            "swim",
            "swim_splash",
            "jump",
            "land",
            "climb",
            "step_dry",
            "step_ink",
            "step_enemy",
            "ink_drip",
        ],
    ),
    (
        "World",
        &["gull", "harbor_ambience", "ferry_horn", "halyard_clink"],
    ),
    (
        "Combat",
        &[
            "hit_marker",
            "ink_hit_body",
            "hurt",
            "splat_enemy",
            "splatted_self",
            "ally_splatted",
            "enemy_ink_sizzle",
        ],
    ),
    (
        "Status",
        &[
            "low_ink",
            "empty_click",
            "refill_full",
            "special_ready",
            "special_activate",
            "special_slam",
            "storm_rain",
            "storm_thunder",
            "respawn",
            "super_jump",
        ],
    ),
    (
        "Specials",
        &[
            "special_ending",
            "special_end",
            "shield_up",
            "shield_hit",
            "shield_pop",
            "sonar_ping",
            "sonar_mark",
            "strike_launch",
            "strike_whistle",
            "strike_impact",
            "tornado",
            "zooka_fire",
            "wail_charge",
            "wail_blast",
            "kraken_on",
            "kraken_jump",
            "kraken_slam",
            "blower_inflate",
            "bubble_release",
            "bubble_pop",
            "bubble_blast",
            "jet_loop",
            "jet_fire",
            "jet_end",
            "stamp_swing",
            "stamp_slam",
            "stamp_throw",
            "booyah_charge",
            "booyah_cheer",
            "booyah_throw",
            "booyah_blast",
            "zip_fire",
            "zip_latch",
            "zip_pull",
            "crab_move",
            "crab_gatling",
            "crab_cannon",
            "crab_roll",
            "crab_hit",
            "crab_break",
        ],
    ),
    (
        "Match",
        &[
            "ready",
            "go_horn",
            "countdown_tick",
            "one_minute",
            "final_count",
            "times_up",
            "judge_drumroll",
            "judge_reveal",
            "victory_fanfare",
            "defeat_jingle",
            "xp_tick",
            "level_up",
        ],
    ),
    (
        "Zones",
        &[
            "zone_ours",
            "zone_theirs",
            "zone_lost",
            "zone_broken",
            "zone_warn",
            "zone_chance",
            "zone_penalty",
            "zone_shift",
            "zone_final",
            "zone_overtime",
            "zone_tick",
            "zone_flood",
            "zone_wipe",
            "zone_hum",
        ],
    ),
    (
        "Boss",
        &[
            "boss_roar",
            "boss_step",
            "boss_slam",
            "boss_tele",
            "boss_whistle",
            "boss_barrel",
            "boss_cannon_charge",
            "boss_cannon_sweep",
            "boss_gallop",
            "boss_crash",
            "boss_dizzy",
            "boss_frenzy",
            "crablet_chitter",
            "crablet_pop",
            "boss_hit",
            "boss_crit",
            "boss_phase",
            "boss_title",
            "boss_defeat",
            "boss_sunk",
        ],
    ),
];

/// Flattened table (group order preserved).
pub fn sfx_names() -> Vec<&'static str> {
    SFX_GROUPS
        .iter()
        .flat_map(|(_, v)| v.iter().copied())
        .collect()
}

/// Fixed per-name bake seed (deviation: replaces the JS shared Math.random
/// stream so assets are byte-stable).
pub fn bake_seed(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

// ---------------------------------------------------------------------------
// Shared wet layers (audio.js L403-423)
// ---------------------------------------------------------------------------

/// `bloops` ≡ audio.js L403-408 (rng draw order: f, t, f1, sw, d, peak).
pub fn bloops(v: &mut Vb, t0: f64, n: usize, spread: f64, fr: [f64; 2], peak: f64, p: f64) {
    for _ in 0..n {
        let f = v.r(fr[0], fr[1]) * p;
        let t = t0 + v.r(0.0, spread);
        let f1 = f * v.r(0.3, 0.45);
        let sw = v.r(0.04, 0.07);
        let d = v.r(0.05, 0.09);
        let pk = peak * v.r(0.6, 1.0);
        v.tone(Tone {
            t,
            f,
            f1: Some(f1),
            sw: Some(sw),
            a: Some(0.0015),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
    }
}

/// `plips` ≡ audio.js L409-411.
pub fn plips(v: &mut Vb, t0: f64, n: usize, spread: f64, fr: [f64; 2], peak: f64, p: f64) {
    for _ in 0..n {
        let t = v.t + t0 + v.r(0.0, spread);
        let f = v.r(fr[0], fr[1]) * p;
        let pk = peak * v.r(0.5, 1.0);
        let d = v.r(0.02, 0.04);
        let rise = v.r(1.4, 2.0);
        v.bub(t, f, pk, d, rise, None);
    }
}

/// `bigSplat` ≡ audio.js L412-418.
pub fn big_splat(v: &mut Vb, t: f64, p: f64, s: f64) {
    v.tone(Tone {
        t,
        f: 120.0 * p,
        f1: Some(40.0 * p),
        sw: Some(0.15),
        a: Some(0.002),
        d: Some(0.28),
        peak: Some(1.0 * s),
        ..Default::default()
    });
    v.nz(Nz {
        t,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(2500.0),
        f1: Some(300.0),
        sw: Some(0.25),
        q: Some(1.5),
        a: Some(0.002),
        d: Some(0.28),
        peak: Some(0.9 * s),
        ..Default::default()
    });
    let q = v.r(5.0, 8.0);
    v.nz(Nz {
        t: t + 0.01,
        f: Some(1600.0 * p),
        f1: Some(320.0 * p),
        sw: Some(0.3),
        q: Some(q),
        a: Some(0.003),
        d: Some(0.32),
        peak: Some(0.9 * s),
        ..Default::default()
    });
    bloops(v, t + 0.02, 3, 0.12, [400.0, 650.0], 0.35 * s, p);
    v.nz(Nz {
        t,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3500.0),
        a: Some(0.005),
        d: Some(0.35),
        peak: Some(0.3 * s),
        ..Default::default()
    });
}

/// `uiNote` ≡ audio.js L419-423.
pub fn ui_note(v: &mut Vb, t: f64, m: f64, p: f64, peak: f64) {
    let f = synth::mtof(m) * p;
    v.tone(Tone {
        t,
        f: f * 0.5,
        f1: Some(f),
        sw: Some(0.018),
        a: Some(0.002),
        d: Some(0.2),
        peak: Some(peak),
        ..Default::default()
    });
    v.tone(Tone {
        t,
        wave: Some(Wave::Triangle),
        f: f * 2.0,
        a: Some(0.001),
        d: Some(0.07),
        peak: Some(peak * 0.18),
        ..Default::default()
    });
}

// ---------------------------------------------------------------------------
// Grain textures (audio.js `texture` L43-80)
// ---------------------------------------------------------------------------

/// `TEX` table ≡ audio.js L35-41.
pub struct TexSpec {
    pub dur: f64,
    pub rate: f64,
    pub f: [f64; 2],
    pub d: [f64; 2],
    pub amp: [f64; 2],
    pub rise: [f64; 2],
    pub crackle: bool,
}

pub fn tex_spec(kind: &str) -> Option<TexSpec> {
    let s = match kind {
        "bubbles" => TexSpec {
            dur: 3.0,
            rate: 46.0,
            f: [230.0, 950.0],
            d: [0.018, 0.065],
            amp: [0.25, 1.0],
            rise: [1.35, 2.3],
            crackle: false,
        },
        "bubbles_bright" => TexSpec {
            dur: 2.5,
            rate: 60.0,
            f: [650.0, 2300.0],
            d: [0.01, 0.035],
            amp: [0.2, 1.0],
            rise: [1.4, 2.4],
            crackle: false,
        },
        "squelch" => TexSpec {
            dur: 2.5,
            rate: 75.0,
            f: [110.0, 460.0],
            d: [0.03, 0.09],
            amp: [0.3, 1.0],
            rise: [1.1, 1.7],
            crackle: false,
        },
        "rain" => TexSpec {
            dur: 3.0,
            rate: 280.0,
            f: [1500.0, 5500.0],
            d: [0.004, 0.016],
            amp: [0.05, 1.0],
            rise: [1.2, 1.9],
            crackle: false,
        },
        "sizzle" => TexSpec {
            dur: 2.0,
            rate: 450.0,
            f: [0.0, 0.0],
            d: [0.0006, 0.003],
            amp: [0.05, 1.0],
            rise: [0.0, 0.0],
            crackle: true,
        },
        _ => return None,
    };
    Some(s)
}

/// `texture(ctx, kind)` ≡ audio.js L43-80 (Float32 storage, double math).
pub fn texture_render(kind: &str) -> Option<Arc<Vec<f32>>> {
    let s = tex_spec(kind)?;
    let sr = SR;
    let len = (s.dur * sr) as usize;
    let mut d = vec![0.0f32; len];
    let mut seed: u32 = 0;
    for c in kind.bytes() {
        seed = seed.wrapping_mul(31).wrapping_add(c as u32);
    }
    let mut rnd = Rng::new(seed);
    let lerp = |r: [f64; 2], x: f64| r[0] + (r[1] - r[0]) * x;
    let n = (s.rate * s.dur) as usize;
    let atk = 0.0012 * sr;
    for _ in 0..n {
        let start = (rnd.next_f64() * len as f64) as usize;
        let dec = lerp(s.d, rnd.next_f64()) * sr;
        let amp = lerp(s.amp, rnd.next_f64().powi(3));
        let l = len.min((dec * 5.0) as usize);
        if s.crackle {
            for i in 0..l {
                let idx = (start + i) % len;
                let x = (d[idx] as f64)
                    + amp * (rnd.next_f64() * 2.0 - 1.0) * (-(i as f64) / dec).exp();
                d[idx] = x as f32;
            }
        } else {
            let f0 = lerp(s.f, rnd.next_f64());
            let f1 = f0 * lerp(s.rise, rnd.next_f64());
            let mut ph = rnd.next_f64() * 0.2;
            for i in 0..l {
                let f = f0 * (f1 / f0).powf((i as f64 / (dec * 2.0)).min(1.0));
                ph += (2.0 * std::f64::consts::PI * f) / sr;
                let idx = (start + i) % len;
                let x = (d[idx] as f64)
                    + amp
                        * (1.0 - (-(i as f64) / atk).exp())
                        * (-(i as f64) / dec).exp()
                        * ph.sin();
                d[idx] = x as f32;
            }
        }
    }
    let mut mean = 0.0f64;
    for &v in &d {
        mean += v as f64;
    }
    mean /= len as f64;
    let mut ss = 0.0f64;
    let mut pk = 0.0f64;
    for x in &mut d {
        let v = (*x as f64 - mean) as f32;
        *x = v;
        ss += v as f64 * v as f64;
        pk = pk.max(v.abs() as f64);
    }
    let g = (0.25 / (ss / len as f64).max(1.0)).min(0.95 / pk.max(1.0));
    for v in &mut d {
        *v = (*v as f64 * g) as f32;
    }
    Some(Arc::new(d))
}

/// Cached ≡ the JS `texCache`.
pub fn texture(kind: &str) -> Arc<Vec<f32>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<HashMap<&'static str, Arc<Vec<f32>>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        let mut m = HashMap::new();
        for k in ["bubbles", "bubbles_bright", "squelch", "rain", "sizzle"] {
            if let Some(b) = texture_render(k) {
                m.insert(k, b);
            }
        }
        m
    });
    map.get(kind)
        .cloned()
        .unwrap_or_else(|| Arc::new(Vec::new()))
}

// ---------------------------------------------------------------------------
// Instrument helpers used by SFX builds (music.js L350-531)
// ---------------------------------------------------------------------------

/// `kick` ≡ music.js L350-360.
pub fn kick(v: &mut Vb, t: f64, vel: f64, f0: f64, f1: f64, d: f64, sw: f64, to: Option<NodeId>) {
    let to = to.unwrap_or(v.out);
    let g = v.gain(0.0, Some(to));
    let end = synth::perc(v.g.param(g), t, 0.0015, vel, d);
    let x = v.osc(Wave::Sine, f0, t, Some(end + 0.01), Some(g));
    synth::sweep(v.g.param(x), t, f0, f1, sw, false);
    v.tone(Tone {
        at: Some(t),
        wave: Some(Wave::Triangle),
        f: f0 * 0.9,
        f1: Some(f1),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.08),
        peak: Some(vel * 0.18),
        to: Some(to),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1600.0),
        q: Some(0.7),
        a: Some(0.0006),
        d: Some(0.012),
        peak: Some(vel * 0.32),
        to: Some(to),
        ..Default::default()
    });
}

/// `snare` ≡ music.js L362-368.
pub fn snare(v: &mut Vb, t: f64, vel: f64, f: f64, d: f64, to: Option<NodeId>) {
    let to = to.unwrap_or(v.out);
    v.tone(Tone {
        at: Some(t),
        wave: Some(Wave::Triangle),
        f,
        f1: Some(f * 0.82),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.11),
        peak: Some(vel * 0.5),
        to: Some(to),
        ..Default::default()
    });
    v.tone(Tone {
        at: Some(t),
        wave: Some(Wave::Sine),
        f: 330.0,
        f1: Some(290.0),
        sw: Some(0.03),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(vel * 0.22),
        to: Some(to),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1300.0),
        q: Some(0.8),
        a: Some(0.001),
        d: Some(d),
        peak: Some(vel * 0.72),
        to: Some(to),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Bandpass)),
        f: Some(4200.0),
        q: Some(0.9),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(vel * 0.32),
        to: Some(to),
        ..Default::default()
    });
}

/// `crash` ≡ music.js L399-406.
pub fn crash(v: &mut Vb, t: f64, vel: f64, d: f64, to: Option<NodeId>) {
    let to = to.unwrap_or(v.out);
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Highpass)),
        f: Some(4200.0),
        q: Some(0.6),
        a: Some(0.001),
        d: Some(d),
        peak: Some(vel * 0.55),
        to: Some(to),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Bandpass)),
        f: Some(7200.0),
        q: Some(0.9),
        a: Some(0.001),
        d: Some(d * 0.5),
        peak: Some(vel * 0.35),
        to: Some(to),
        ..Default::default()
    });
    let hp = v.filter(BqKind::Highpass, 3000.0, 0.7, Some(to), None);
    for f in [3410.0, 4870.0, 6230.0] {
        v.tone(Tone {
            at: Some(t),
            wave: Some(Wave::Square),
            f,
            a: Some(0.001),
            d: Some(0.5),
            peak: Some(vel * 0.02),
            to: Some(hp),
            ..Default::default()
        });
    }
}

/// `tom` ≡ music.js L418-422.
pub fn tom(v: &mut Vb, t: f64, f: f64, vel: f64, d: f64, to: Option<NodeId>) {
    let to = to.unwrap_or(v.out);
    v.tone(Tone {
        at: Some(t),
        f: f * 1.5,
        f1: Some(f),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(d),
        peak: Some(vel * 0.8),
        to: Some(to),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1800.0),
        q: Some(0.8),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(vel * 0.25),
        kind: Some(NoiseKind::Pink),
        to: Some(to),
        ..Default::default()
    });
}

/// `bass` (punk/funk/sub) ≡ music.js L424-454.
pub fn bass(
    v: &mut Vb,
    t: f64,
    f: f64,
    gate: f64,
    vel: f64,
    style: &str,
    drive: f64,
    to: Option<NodeId>,
) {
    let to = to.unwrap_or(v.out);
    let g = v.gain(0.0, Some(to));
    if style == "funk" {
        let end = synth::adsr(v.g.param(g), t, 0.002, 0.22, 0.45, vel, gate, 0.05);
        let lp = v.filter(BqKind::Lowpass, 300.0, 7.0, Some(g), None);
        synth::sweep(
            v.g.param(lp),
            t,
            2600.0 * (0.6 + 0.4 * vel),
            330.0,
            0.12,
            false,
        );
        v.osc(Wave::Square, f, t, Some(end + 0.01), Some(lp));
        let sg = v.gain(0.55, Some(g));
        v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(sg));
        return;
    }
    if style == "sub" {
        let end = synth::adsr(v.g.param(g), t, 0.008, 0.3, 0.75, vel, gate, 0.1);
        let lp = v.filter(BqKind::Lowpass, 900.0, 0.8, Some(g), None);
        v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(lp));
        let h = v.gain(0.25, Some(lp));
        v.osc(Wave::Triangle, f * 2.0, t, Some(end + 0.01), Some(h));
        return;
    }
    // punk
    let end = synth::adsr(v.g.param(g), t, 0.003, 0.2, 0.72, vel, gate, 0.05);
    let sh = v.shaper(drive, Some(g));
    let pre = v.gain(0.55, Some(sh));
    let lp = v.filter(BqKind::Lowpass, 200.0, 4.5, Some(pre), None);
    v.g.param(lp).set(220.0, t);
    v.g.param(lp).exp(700.0 + 1300.0 * vel, t + 0.012);
    v.g.param(lp).exp(520.0, t + 0.22);
    v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(lp));
    let sq = v.osc(Wave::Square, f, t, Some(end + 0.01), Some(lp));
    v.g.param2(sq).set_default(-8.0);
    let sub = v.gain(0.6, Some(g));
    v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(sub));
}

/// `pad` ≡ music.js L483-490.
pub fn pad(
    v: &mut Vb,
    t: f64,
    f: f64,
    gate: f64,
    vel: f64,
    a: f64,
    r: f64,
    cut: f64,
    to: Option<NodeId>,
) {
    let to = to.unwrap_or(v.out);
    let g = v.gain(0.0, Some(to));
    let end = synth::adsr(v.g.param(g), t, a, 0.6, 0.8, vel, gate, r);
    let lp = v.filter(BqKind::Lowpass, cut, 0.9, Some(g), None);
    for dt in [-11.0, 10.0] {
        let x = v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(lp));
        v.g.param2(x).set_default(dt);
    }
}

/// `bell` ≡ music.js L503-515.
pub fn bell(
    v: &mut Vb,
    t: f64,
    f: f64,
    vel: f64,
    d: f64,
    ratio: f64,
    index: f64,
    to: Option<NodeId>,
) {
    let to = to.unwrap_or(v.out);
    let g = v.gain(0.0, Some(to));
    let end = synth::perc(v.g.param(g), t, 0.001, vel, d);
    let car = v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(g));
    let mg = v.gain(0.0, None);
    v.g.connect_param(mg, ParamRef::Freq(car));
    v.osc(Wave::Sine, f * ratio, t, Some(end + 0.01), Some(mg));
    v.g.param(mg).set(f * index, t);
    v.g.param(mg).exp(f * 0.05, t + d * 0.6);
    v.tone(Tone {
        at: Some(t),
        f: f * 2.01,
        a: Some(0.001),
        d: Some(d * 0.4),
        peak: Some(vel * 0.25),
        to: Some(to),
        ..Default::default()
    });
}

/// `brass` ≡ music.js L517-531.
pub fn brass(
    v: &mut Vb,
    t: f64,
    f: f64,
    gate: f64,
    vel: f64,
    a: f64,
    r: f64,
    bright: f64,
    pan: Option<f64>,
) {
    let to = match pan {
        Some(p) => v.pan(p, Some(v.out)),
        None => v.out,
    };
    let g = v.gain(0.0, Some(to));
    let end = synth::adsr(v.g.param(g), t, a, 0.3, 0.72, vel / 2.2, gate, r);
    let lp = v.filter(BqKind::Lowpass, 400.0, 1.3, Some(g), None);
    v.g.param(lp).set(380.0, t);
    v.g.param(lp).exp(bright, t + 0.05);
    v.g.param(lp).exp(bright * 0.55, t + 0.35);
    for dt in [-9.0, 0.0, 9.0] {
        let x = v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(lp));
        v.g.param2(x).set(dt - 45.0, t);
        v.g.param2(x).lin(dt, t + 0.05);
    }
}

// ---------------------------------------------------------------------------
// Specials shared helpers (audio.js L1045-1079)
// ---------------------------------------------------------------------------

/// `clank` ≡ audio.js L1045-1049 — struck metal: inharmonic free-bar
/// partials + a contact click.
pub fn clank(v: &mut Vb, t: f64, f: f64, peak: f64, d: f64, p: f64, n: usize, to: Option<NodeId>) {
    const P: [[f64; 3]; 4] = [
        [1.0, 1.0, 1.0],
        [2.76, 0.55, 0.65],
        [5.4, 0.3, 0.4],
        [8.93, 0.15, 0.25],
    ];
    for row in P.iter().take(n) {
        let [r, a, k] = *row;
        v.tone(Tone {
            t,
            f: f * r * p,
            a: Some(0.0006),
            d: Some(d * k),
            peak: Some(peak * a),
            to,
            ..Default::default()
        });
    }
    v.nz(Nz {
        t,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.0004),
        d: Some(0.01),
        peak: Some(peak * 0.5),
        to,
        ..Default::default()
    });
}

/// `whoosh` ≡ audio.js L1051-1059 — band-passed noise sweep centre f0 → fm
/// (at `mid` of dur) → f1, level follows `env` (fractions of dur × peak).
/// Returns the band-pass node.
pub fn whoosh(
    v: &mut Vb,
    t: f64,
    dur: f64,
    f0: f64,
    fm: f64,
    f1: f64,
    peak: f64,
    q: Option<f64>,
    mid: Option<f64>,
    env: Option<&[[f64; 2]]>,
    kind: Option<NoiseKind>,
    to: Option<NodeId>,
) -> NodeId {
    let t_abs = v.t + t;
    let dest = to.unwrap_or(v.out);
    let g = v.gain(0.0, Some(dest));
    let bp = v.filter(BqKind::Bandpass, f0, q.unwrap_or(1.5), Some(g), None);
    v.g.param(bp).set(f0, t_abs);
    v.g.param(bp).exp(fm, t_abs + dur * mid.unwrap_or(0.55));
    v.g.param(bp).exp(f1, t_abs + dur);
    let default_env = [[0.0, 0.0], [0.5, 1.0], [0.7, 0.6], [1.0, 0.0]];
    let scaled: Vec<(f64, f64)> = env
        .unwrap_or(&default_env)
        .iter()
        .map(|[x, y]| (*x * dur, *y * peak))
        .collect();
    synth::pts(v.g.param(g), t_abs, &scaled);
    v.noise(
        kind.unwrap_or(NoiseKind::Pink),
        t_abs,
        Some(t_abs + dur + 0.02),
        Some(bp),
        1.0,
    );
    bp
}

/// `inkBoom` options ≡ the `o` object (audio.js L1061).
#[derive(Clone, Copy, Debug)]
pub struct BoomOpts {
    pub len: f64,
    pub f: f64,
    pub f1: f64,
    pub crack: f64,
    pub bright: f64,
    pub bloops: usize,
    pub plips: usize,
}

impl Default for BoomOpts {
    fn default() -> Self {
        Self {
            len: 1.0,
            f: 85.0,
            f1: 26.0,
            crack: 1.0,
            bright: 5500.0,
            bloops: 7,
            plips: 5,
        }
    }
}

/// `inkBoom` ≡ audio.js L1061-1072 — big wet explosion core shared by the
/// heavy impacts (sub drop, crack, driven noise burst, wash, bloops, hiss
/// tail, droplet plips).
pub fn ink_boom(v: &mut Vb, t: f64, p: f64, s: f64, o: BoomOpts) {
    let len = o.len;
    v.tone(Tone {
        t,
        f: o.f * p,
        f1: Some(o.f1 * p),
        sw: Some(0.75 * len),
        a: Some(0.002),
        d: Some(0.95 * len),
        peak: Some(s),
        ..Default::default()
    });
    v.tone(Tone {
        t,
        wave: Some(Wave::Triangle),
        f: 180.0 * p,
        f1: Some(55.0 * p),
        sw: Some(0.14),
        a: Some(0.001),
        d: Some(0.18),
        peak: Some(0.45 * s),
        ..Default::default()
    });
    v.nz(Nz {
        t,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1800.0),
        a: Some(0.0003),
        d: Some(0.05),
        peak: Some(o.crack * s),
        ..Default::default()
    });
    let sh_out = v.gain(0.75, Some(v.out));
    let sh = v.shaper(2.4, Some(sh_out));
    v.nz(Nz {
        t,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(o.bright),
        f1: Some(160.0),
        sw: Some(0.7 * len),
        q: Some(0.85),
        a: Some(0.002),
        d: Some(0.85 * len),
        peak: Some(s),
        to: Some(sh),
        ..Default::default()
    });
    v.nz(Nz {
        t: t + 0.04,
        f: Some(1300.0 * p),
        f1: Some(280.0 * p),
        sw: Some(0.8 * len),
        q: Some(2.5),
        a: Some(0.006),
        d: Some(0.85 * len),
        peak: Some(0.65 * s),
        ..Default::default()
    });
    bloops(
        v,
        t + 0.05,
        o.bloops,
        0.45 * len,
        [280.0, 680.0],
        0.3 * s,
        p,
    );
    v.nz(Nz {
        t: t + 0.07,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.05),
        d: Some(0.8 * len),
        peak: Some(0.28 * s),
        ..Default::default()
    });
    plips(v, t + 0.2, o.plips, 0.6 * len, [800.0, 1700.0], 0.1 * s, p);
}

/// `vox` ≡ audio.js L1074-1079 — sawtooth "voice" through parallel
/// band-pass formants → `to`; returns (source oscs, formant filter nodes).
pub fn vox(
    v: &mut Vb,
    t: f64,
    t1: f64,
    f0: f64,
    formants: &[[f64; 3]],
    to: NodeId,
    detune: f64,
) -> (Vec<NodeId>, Vec<NodeId>) {
    let mix = v.gain(1.0, None);
    let mut srcs = Vec::with_capacity(2);
    for dt in [-detune, detune] {
        let o = v.osc(Wave::Saw, f0, t, Some(t1), Some(mix));
        v.g.param2(o).set_default(dt);
        srcs.push(o);
    }
    let mut fs = Vec::with_capacity(formants.len());
    for [f, q, a] in formants {
        let g = v.gain(*a, Some(to));
        let bp = v.filter(BqKind::Bandpass, *f, *q, Some(g), None);
        v.g.connect(mix, bp);
        fs.push(bp);
    }
    (srcs, fs)
}

// ---------------------------------------------------------------------------
// Definitions — translated chunk-by-chunk from audio.js L426-2478.
// ---------------------------------------------------------------------------

/* ---- UI (audio.js L425-469) ---- */

/// `ui_hover` build ≡ audio.js L428-431.
fn b_ui_hover(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 1900.0 * p,
        f1: Some(1450.0 * p),
        sw: Some(0.03),
        a: Some(0.002),
        d: Some(0.05),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5000.0),
        a: Some(0.0006),
        d: Some(0.012),
        peak: Some(0.12),
        ..Default::default()
    });
}

/// `ui_click` build ≡ audio.js L435-439.
fn b_ui_click(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 380.0 * p,
        f1: Some(1150.0 * p),
        sw: Some(0.035),
        a: Some(0.002),
        d: Some(0.075),
        peak: Some(0.6),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 760.0 * p,
        f1: Some(2300.0 * p),
        sw: Some(0.03),
        a: Some(0.001),
        d: Some(0.04),
        peak: Some(0.16),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(3000.0),
        q: Some(1.5),
        a: Some(0.0005),
        d: Some(0.01),
        peak: Some(0.18),
        ..Default::default()
    });
}

/// `ui_confirm` build ≡ audio.js L441.
fn b_ui_confirm(v: &mut Vb, p: f64) {
    ui_note(v, 0.0, 84.0, p, 0.5);
    ui_note(v, 0.075, 91.0, p, 0.55);
}

/// `ui_back` build ≡ audio.js L442.
fn b_ui_back(v: &mut Vb, p: f64) {
    ui_note(v, 0.0, 79.0, p, 0.5);
    ui_note(v, 0.07, 72.0, p, 0.5);
}

/// `ui_toggle` build ≡ audio.js L445-450.
fn b_ui_toggle(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 2200.0, 1.2, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 640.0 * p,
        f1: Some(540.0 * p),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(0.3),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        f: 1280.0 * p,
        a: Some(0.001),
        d: Some(0.03),
        peak: Some(0.25),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(4000.0),
        a: Some(0.0005),
        d: Some(0.008),
        peak: Some(0.15),
        ..Default::default()
    });
}

/// `ui_slider` build ≡ audio.js L454-457.
fn b_ui_slider(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 2600.0 * p,
        a: Some(0.001),
        d: Some(0.022),
        peak: Some(0.5),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 5200.0 * p,
        a: Some(0.0005),
        d: Some(0.01),
        peak: Some(0.1),
        ..Default::default()
    });
}

/// `ui_error` build ≡ audio.js L461-468.
fn b_ui_error(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 1100.0, 1.5, Some(v.out), None);
    for t in [0.0, 0.11] {
        for f in [155.0, 163.0] {
            let t = v.t + t;
            let g = v.gain(0.0, Some(lp));
            let end = synth::ahr(v.g.param(g), t, 0.004, 0.25, 0.065, 0.03);
            v.osc(Wave::Square, f * p, t, Some(end + 0.01), Some(g));
        }
    }
}

/* ---- Weapons (audio.js L471-704) ---- */

/// `shoot_shooter` build ≡ audio.js L474-486.
fn b_shoot_shooter(v: &mut Vb, p: f64) {
    // three voicings so a 10-shot/s stream never machine-guns one sample
    let vo = v.pick(&[(1.0, 1.0), (1.12, 0.93), (0.9, 1.08)]);
    let k = vo.0 * p;
    let w = vo.1 * p;
    v.nz(Nz {
        f: Some(4200.0 * k),
        q: Some(1.6),
        a: Some(0.0004),
        d: Some(0.018),
        peak: Some(0.66),
        ..Default::default()
    });
    v.tone(Tone {
        f: 2600.0 * k,
        f1: Some(820.0 * k),
        sw: Some(0.014),
        a: Some(0.0005),
        d: Some(0.02),
        peak: Some(0.26),
        ..Default::default()
    });
    let bp = v.r(2500.0, 3100.0) * k;
    v.nz(Nz {
        f: Some(bp),
        f1: Some(bp * 0.62),
        sw: Some(0.05),
        q: Some(1.4),
        a: Some(0.0008),
        d: Some(0.05),
        peak: Some(0.8),
        ..Default::default()
    });
    v.tone(Tone {
        f: 175.0 * w,
        f1: Some(62.0 * w),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.055),
        peak: Some(0.52),
        ..Default::default()
    });
    let f = v.r(1500.0, 1900.0) * w;
    let f1 = v.r(520.0, 680.0) * w;
    let q = v.r(4.5, 7.0);
    v.nz(Nz {
        t: 0.01,
        f: Some(f),
        f1: Some(f1),
        sw: Some(0.09),
        q: Some(q),
        a: Some(0.004),
        d: Some(0.09),
        peak: Some(0.72),
        ..Default::default()
    });
    let t = v.t + v.r(0.018, 0.035);
    let f = v.r(900.0, 1400.0) * w;
    v.bub(t, f, 0.12, 0.022, 1.8, None);
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5200.0),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `shoot_blaster` build ≡ audio.js L490-500.
fn b_shoot_blaster(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3800.0),
        a: Some(0.0003),
        d: Some(0.008),
        peak: Some(0.5),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(1500.0 * p),
        f1: Some(900.0 * p),
        sw: Some(0.02),
        q: Some(2.2),
        a: Some(0.0006),
        d: Some(0.022),
        peak: Some(0.75),
        ..Default::default()
    });
    v.tone(Tone {
        f: 150.0 * p,
        f1: Some(40.0 * p),
        sw: Some(0.16),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(1.0),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 300.0 * p,
        f1: Some(88.0 * p),
        sw: Some(0.08),
        a: Some(0.002),
        d: Some(0.1),
        peak: Some(0.35),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1600.0),
        f1: Some(280.0),
        sw: Some(0.12),
        q: Some(2.0),
        a: Some(0.002),
        d: Some(0.14),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.008,
        f: Some(320.0 * p),
        f1: Some(950.0 * p),
        sw: Some(0.06),
        q: Some(6.0),
        a: Some(0.004),
        d: Some(0.08),
        peak: Some(0.55),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.01,
        f: 190.0 * p,
        f1: Some(430.0 * p),
        sw: Some(0.07),
        a: Some(0.003),
        d: Some(0.075),
        peak: Some(0.4),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(700.0 * p),
        f1: Some(2400.0 * p),
        sw: Some(0.22),
        q: Some(1.4),
        a: Some(0.03),
        d: Some(0.18),
        peak: Some(0.26),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.05,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5000.0),
        a: Some(0.02),
        d: Some(0.16),
        peak: Some(0.08),
        ..Default::default()
    });
}

/// `blaster_pump` build ≡ audio.js L506-518.
fn b_blaster_pump(v: &mut Vb, p: f64) {
    let clack = |v: &mut Vb, t: f64, hard: f64, p: f64| {
        v.nz(Nz {
            t,
            f: Some(3100.0 * p),
            q: Some(2.4),
            a: Some(0.0003),
            d: Some(0.009),
            peak: Some(0.8 * hard),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: 1870.0 * p,
            a: Some(0.0005),
            d: Some(0.045),
            peak: Some(0.12 * hard),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: 3050.0 * p,
            a: Some(0.0005),
            d: Some(0.03),
            peak: Some(0.07 * hard),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: 230.0 * p,
            f1: Some(120.0 * p),
            sw: Some(0.025),
            a: Some(0.001),
            d: Some(0.03),
            peak: Some(0.5 * hard),
            ..Default::default()
        });
    };
    v.nz(Nz {
        f: Some(1200.0 * p),
        f1: Some(2200.0 * p),
        sw: Some(0.02),
        q: Some(1.2),
        a: Some(0.008),
        d: Some(0.014),
        peak: Some(0.12),
        ..Default::default()
    });
    clack(v, 0.02, 0.85, p);
    v.nz(Nz {
        t: 0.08,
        f: Some(2000.0 * p),
        f1: Some(1300.0 * p),
        sw: Some(0.1),
        q: Some(1.2),
        a: Some(0.03),
        d: Some(0.08),
        peak: Some(0.1),
        ..Default::default()
    });
    clack(v, 0.19, 1.0, p);
    v.nz(Nz {
        t: 0.2,
        f: Some(900.0 * p),
        f1: Some(500.0 * p),
        sw: Some(0.08),
        q: Some(1.5),
        a: Some(0.01),
        d: Some(0.09),
        peak: Some(0.25),
        ..Default::default()
    });
}

/// `blaster_boom` build ≡ audio.js L522-533.
fn b_blaster_boom(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 110.0 * p,
        f1: Some(34.0 * p),
        sw: Some(0.3),
        a: Some(0.002),
        d: Some(0.45),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2500.0),
        a: Some(0.0005),
        d: Some(0.03),
        peak: Some(0.7),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.004,
        f: 950.0 * p,
        f1: Some(210.0 * p),
        sw: Some(0.03),
        a: Some(0.0006),
        d: Some(0.03),
        peak: Some(0.28),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(3500.0),
        f1: Some(250.0),
        sw: Some(0.35),
        q: Some(1.0),
        a: Some(0.002),
        d: Some(0.4),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.03,
        f: Some(1400.0 * p),
        f1: Some(450.0 * p),
        sw: Some(0.4),
        q: Some(3.5),
        a: Some(0.004),
        d: Some(0.45),
        peak: Some(0.7),
        ..Default::default()
    });
    bloops(v, 0.04, 3, 0.1, [400.0, 700.0], 0.25, p);
    // the shell of droplets raining back down: a patter that thins out
    let mut t = 0.09;
    for i in 0..9 {
        t += v.r(0.022, 0.05);
        let tt = v.t + t;
        let f = v.r(700.0, 1900.0) * p;
        let d = v.r(0.012, 0.026);
        let rise = v.r(1.5, 2.1);
        v.bub(tt, f, 0.16 * (1.0 - i as f64 / 11.0), d, rise, None);
    }
    v.nz(Nz {
        t: 0.05,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3500.0),
        a: Some(0.02),
        d: Some(0.4),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `charger_charge` loop ≡ audio.js L537-555 (pitch ctl is live-loop only,
/// not baked; the burst bake builds the nominal graph).
fn l_charger_charge(v: &mut Vb, p: f64) {
    let t = v.t;
    let base = 250.0;
    let amp = v.gain(0.7, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, base * 4.0 * p, 5.0, Some(amp), None);
    v.osc(Wave::Saw, base * p, t, None, Some(bp));
    let g2 = v.gain(0.35, Some(amp));
    v.osc(Wave::Sine, base * 2.0 * p, t, None, Some(g2));
    let g3 = v.gain(0.12, Some(amp));
    v.osc(Wave::Sine, base * 3.01 * p, t, None, Some(g3));
    v.lfo(7.0 * p, 0.3, ParamRef::Gain(amp), t, None, Wave::Sine);
    let sh = v.gain(0.05 * p, Some(v.out));
    let hp = v.filter(BqKind::Highpass, 7000.0, 0.7, Some(sh), None);
    v.noise(NoiseKind::White, t, None, Some(hp), 1.0);
}

/// `charger_full` build ≡ audio.js L559-565.
fn b_charger_full(v: &mut Vb, p: f64) {
    let ding = |v: &mut Vb, t: f64, f: f64, k: f64| {
        for (r, a, d) in [
            (1.0, 0.5, 0.9),
            (2.0, 0.22, 0.6),
            (3.01, 0.14, 0.4),
            (4.2, 0.09, 0.28),
            (5.43, 0.05, 0.2),
        ] {
            v.tone(Tone {
                t,
                f: f * r,
                a: Some(0.001),
                d: Some(d * k),
                peak: Some(a),
                ..Default::default()
            });
        }
    };
    ding(v, 0.0, 1568.0 * p, 1.0);
    ding(v, 0.065, 2349.0 * p, 0.8);
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(8000.0),
        a: Some(0.002),
        d: Some(0.15),
        peak: Some(0.08),
        ..Default::default()
    });
}

/// `shoot_charger` build ≡ audio.js L569-579.
fn b_shoot_charger(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2600.0),
        a: Some(0.0002),
        d: Some(0.03),
        peak: Some(1.0),
        ..Default::default()
    });
    v.tone(Tone {
        f: 5200.0 * p,
        a: Some(0.0002),
        d: Some(0.006),
        peak: Some(0.22),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(6000.0 * p),
        f1: Some(900.0 * p),
        sw: Some(0.14),
        q: Some(3.0),
        a: Some(0.001),
        d: Some(0.16),
        peak: Some(0.5),
        ..Default::default()
    });
    let lp = v.filter(BqKind::Lowpass, 6000.0, 1.0, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 2600.0 * p,
        f1: Some(260.0 * p),
        sw: Some(0.13),
        a: Some(0.0008),
        d: Some(0.15),
        peak: Some(0.16),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        f: 140.0 * p,
        f1: Some(46.0 * p),
        sw: Some(0.12),
        a: Some(0.001),
        d: Some(0.2),
        peak: Some(0.85),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.03,
        f: Some(1700.0 * p),
        f1: Some(420.0 * p),
        sw: Some(0.5),
        q: Some(2.5),
        a: Some(0.01),
        d: Some(0.55),
        peak: Some(0.5),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.04,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.03),
        d: Some(0.45),
        peak: Some(0.2),
        ..Default::default()
    });
    bloops(v, 0.05, 2, 0.12, [500.0, 800.0], 0.2, p);
}

/// `roller_flick` build ≡ audio.js L583-594.
fn b_roller_flick(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 350.0, 1.8, Some(g), None);
    v.g.param(bp).set(320.0 * p, t);
    v.g.param(bp).exp(1700.0 * p, t + 0.15);
    v.g.param(bp).exp(600.0 * p, t + 0.32);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.13, 0.7), (0.2, 0.5), (0.34, 0.0)],
    );
    v.noise(NoiseKind::Pink, t, Some(t + 0.36), Some(bp), 1.0);
    v.tone(Tone {
        t: 0.12,
        f: 120.0 * p,
        f1: Some(45.0 * p),
        sw: Some(0.18),
        a: Some(0.002),
        d: Some(0.25),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.14,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1100.0),
        f1: Some(220.0),
        sw: Some(0.2),
        q: Some(1.5),
        a: Some(0.002),
        d: Some(0.2),
        peak: Some(0.6),
        ..Default::default()
    });
    // the sheet leaves the drum as a ripple of overlapping slaps
    for i in 0..4 {
        let tt = v.t + 0.145 + i as f64 * 0.018 + v.r(0.0, 0.006);
        let f = v.r(1100.0, 1500.0) * p;
        let f1 = v.r(350.0, 480.0) * p;
        let q = v.r(2.5, 3.5);
        v.nz(Nz {
            at: Some(tt),
            f: Some(f),
            f1: Some(f1),
            sw: Some(0.3 + i as f64 * 0.05),
            q: Some(q),
            a: Some(0.003),
            d: Some(0.34),
            peak: Some(0.55 - i as f64 * 0.08),
            ..Default::default()
        });
    }
    bloops(v, 0.16, 4, 0.2, [350.0, 700.0], 0.3, p);
    v.nz(Nz {
        t: 0.16,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3500.0),
        a: Some(0.01),
        d: Some(0.5),
        peak: Some(0.25),
        ..Default::default()
    });
}

/// `roll` loop ≡ audio.js L598-612.
fn l_roll(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 1200.0 + 600.0 * p, 0.8, Some(v.out), None);
    let g09 = v.gain(0.9, Some(lp));
    v.buffer(texture("squelch"), t, None, Some(g09), p, None);
    let g55 = v.gain(0.55, Some(v.out));
    let lp2 = v.filter(BqKind::Lowpass, 220.0, 1.0, Some(g55), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp2), 1.0);
    let sq = v.gain(0.1, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 750.0, 3.0, Some(sq), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp), 1.0);
    v.lfo(5.0 * p, 0.09, ParamRef::Gain(sq), t, None, Wave::Sine);
}

/// `shoot_dualies` build ≡ audio.js L620-627.
fn b_shoot_dualies(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(4800.0 * p),
        q: Some(1.8),
        a: Some(0.0003),
        d: Some(0.01),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.002,
        f: Some(3200.0 * p),
        q: Some(3.0),
        a: Some(0.0003),
        d: Some(0.006),
        peak: Some(0.35),
        ..Default::default()
    });
    let bp = v.r(2700.0, 3300.0) * p;
    v.nz(Nz {
        f: Some(bp),
        f1: Some(bp * 0.6),
        sw: Some(0.04),
        q: Some(1.5),
        a: Some(0.0006),
        d: Some(0.04),
        peak: Some(0.72),
        ..Default::default()
    });
    v.tone(Tone {
        f: 210.0 * p,
        f1: Some(80.0 * p),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.042),
        peak: Some(0.45),
        ..Default::default()
    });
    let f = v.r(1700.0, 2100.0) * p;
    let f1 = v.r(600.0, 760.0) * p;
    let q = v.r(5.0, 7.0);
    v.nz(Nz {
        t: 0.008,
        f: Some(f),
        f1: Some(f1),
        sw: Some(0.07),
        q: Some(q),
        a: Some(0.003),
        d: Some(0.07),
        peak: Some(0.55),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5600.0),
        a: Some(0.001),
        d: Some(0.045),
        peak: Some(0.16),
        ..Default::default()
    });
}

/// `dualies_roll` build ≡ audio.js L633-642.
fn b_dualies_roll(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 500.0, 1.2, Some(g), None);
    synth::sweep(v.g.param(bp), t, 450.0 * p, 1900.0 * p, 0.12, false);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.05, 0.6), (0.16, 0.35), (0.3, 0.0)],
    );
    v.noise(NoiseKind::Pink, t, Some(t + 0.32), Some(bp), 1.0);
    v.nz(Nz {
        t: 0.04,
        f: Some(900.0 * p),
        f1: Some(1400.0 * p),
        sw: Some(0.2),
        q: Some(4.0),
        a: Some(0.02),
        d: Some(0.2),
        peak: Some(0.3),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.06,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1400.0),
        f1: Some(400.0),
        sw: Some(0.25),
        q: Some(1.5),
        a: Some(0.01),
        d: Some(0.22),
        peak: Some(0.45),
        ..Default::default()
    });
    plips(v, 0.08, 3, 0.18, [800.0, 1500.0], 0.14, p);
    v.tone(Tone {
        t: 0.29,
        f: 140.0 * p,
        f1: Some(60.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.09),
        peak: Some(0.7),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.29,
        f: Some(700.0 * p),
        f1: Some(380.0 * p),
        sw: Some(0.06),
        q: Some(3.0),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.4),
        ..Default::default()
    });
}

/// `slosh_throw` build ≡ audio.js L647-657.
fn b_slosh_throw(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 300.0, 1.6, Some(g), None);
    v.g.param(bp).set(260.0 * p, t);
    v.g.param(bp).exp(1400.0 * p, t + 0.13);
    v.g.param(bp).exp(500.0 * p, t + 0.3);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.11, 0.65), (0.17, 0.4), (0.32, 0.0)],
    );
    v.noise(NoiseKind::Pink, t, Some(t + 0.34), Some(bp), 1.0);
    v.tone(Tone {
        t: 0.12,
        f: 180.0 * p,
        f1: Some(420.0 * p),
        sw: Some(0.09),
        a: Some(0.004),
        d: Some(0.12),
        peak: Some(0.55),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.12,
        f: Some(380.0 * p),
        f1: Some(1100.0 * p),
        sw: Some(0.1),
        q: Some(6.0),
        a: Some(0.006),
        d: Some(0.12),
        peak: Some(0.5),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.125,
        f: 110.0 * p,
        f1: Some(48.0 * p),
        sw: Some(0.14),
        a: Some(0.003),
        d: Some(0.2),
        peak: Some(0.7),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.14,
        f: Some(1500.0 * p),
        f1: Some(500.0 * p),
        sw: Some(0.35),
        q: Some(2.5),
        a: Some(0.01),
        d: Some(0.4),
        peak: Some(0.45),
        ..Default::default()
    });
    bloops(v, 0.15, 3, 0.18, [350.0, 650.0], 0.22, p);
}

/// `slosh_land` build ≡ audio.js L662-667.
fn b_slosh_land(v: &mut Vb, p: f64) {
    big_splat(v, 0.0, p * 0.9, 0.95);
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2800.0),
        a: Some(0.0005),
        d: Some(0.025),
        peak: Some(0.45),
        ..Default::default()
    });
    let mut t = 0.06;
    for i in 0..6 {
        t += v.r(0.02, 0.045);
        let tt = v.t + t;
        let f = v.r(650.0, 1500.0) * p;
        let d = v.r(0.014, 0.028);
        let rise = v.r(1.5, 2.0);
        v.bub(tt, f, 0.14 * (1.0 - i as f64 / 8.0), d, rise, None);
    }
}

/// `splatling_spin` loop ≡ audio.js L672-691.
fn l_splatling_spin(v: &mut Vb, p: f64) {
    let t = v.t;
    let base = 180.0;
    let amp = v.gain(0.6, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 1800.0 * p, 0.9, Some(amp), None);
    v.osc(Wave::Saw, base * p, t, None, Some(lp));
    let g08 = v.gain(0.08, Some(amp));
    let bp2 = v.filter(BqKind::Bandpass, base * 6.0 * p, 3.0, Some(g08), None);
    v.osc(Wave::Square, base * 2.02 * p, t, None, Some(bp2));
    let rat = v.gain(0.06, Some(v.out));
    let rbp = v.filter(BqKind::Bandpass, 2400.0, 2.5, Some(rat), None);
    v.noise(NoiseKind::White, t, None, Some(rbp), 1.0);
    v.lfo(
        base * p / 6.0,
        0.05,
        ParamRef::Gain(rat),
        t,
        None,
        Wave::Square,
    );
    let air = v.gain(0.04 * p, Some(v.out));
    let hp = v.filter(BqKind::Highpass, 3000.0, 0.7, Some(air), None);
    v.noise(NoiseKind::Pink, t, None, Some(hp), 1.0);
}

/// `splatling_ready` build ≡ audio.js L696-700.
fn b_splatling_ready(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 170.0 * p,
        f1: Some(90.0 * p),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.8),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(2600.0 * p),
        q: Some(3.0),
        a: Some(0.0004),
        d: Some(0.012),
        peak: Some(0.6),
        ..Default::default()
    });
    for (t, m) in [(0.02, 91.0), (0.075, 96.0)] {
        let f = synth::mtof(m) * p;
        v.tone(Tone {
            t,
            f,
            a: Some(0.001),
            d: Some(0.5),
            peak: Some(0.34),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: f * 2.76,
            a: Some(0.001),
            d: Some(0.14),
            peak: Some(0.08),
            ..Default::default()
        });
    }
}

/// `shoot_splatling` build ≡ audio.js L703-712.
fn b_shoot_splatling(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(3600.0 * p),
        q: Some(1.6),
        a: Some(0.0003),
        d: Some(0.012),
        peak: Some(0.75),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(2400.0 * p),
        f1: Some(1300.0 * p),
        sw: Some(0.03),
        q: Some(1.6),
        a: Some(0.0005),
        d: Some(0.04),
        peak: Some(0.85),
        ..Default::default()
    });
    v.tone(Tone {
        f: 160.0 * p,
        f1: Some(70.0 * p),
        sw: Some(0.03),
        a: Some(0.001),
        d: Some(0.04),
        peak: Some(0.42),
        ..Default::default()
    });
    let f = v.r(1500.0, 1900.0) * p;
    v.nz(Nz {
        t: 0.006,
        f: Some(f),
        f1: Some(600.0 * p),
        sw: Some(0.06),
        q: Some(5.0),
        a: Some(0.002),
        d: Some(0.075),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.01,
        f: Some(2100.0 * p),
        f1: Some(1500.0 * p),
        sw: Some(0.08),
        q: Some(1.2),
        a: Some(0.004),
        d: Some(0.08),
        peak: Some(0.28),
        ..Default::default()
    });
}

/// `splatling_wind` build ≡ audio.js L714-723.
fn b_splatling_wind(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 2400.0, 0.9, Some(g), None);
    let o = v.osc(Wave::Saw, 270.0 * p, t, Some(t + 0.62), Some(lp));
    synth::sweep(v.g.param(o), t, 270.0 * p, 60.0 * p, 0.6, false);
    synth::sweep(v.g.param(lp), t, 2400.0, 300.0, 0.6, false);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.02, 0.5), (0.3, 0.25), (0.6, 0.0)],
    );
    v.nz(Nz {
        t: 0.03,
        f: Some(3500.0),
        f1: Some(1200.0),
        sw: Some(0.4),
        q: Some(1.0),
        a: Some(0.04),
        d: Some(0.35),
        peak: Some(0.12),
        ..Default::default()
    });
}

/* ---- Ink (audio.js L726-772) ---- */

/// `splat_small` build ≡ audio.js L726-735.
fn b_splat_small(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 170.0 * p,
        f1: Some(65.0 * p),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.08),
        peak: Some(0.8),
        ..Default::default()
    });
    let f = v.r(2200.0, 3000.0) * p;
    let f1 = v.r(800.0, 1100.0) * p;
    let q = v.r(2.0, 3.5);
    v.nz(Nz {
        f: Some(f),
        f1: Some(f1),
        sw: Some(0.09),
        q: Some(q),
        a: Some(0.001),
        d: Some(0.1),
        peak: Some(0.9),
        ..Default::default()
    });
    let f = v.r(550.0, 750.0) * p;
    v.tone(Tone {
        t: 0.006,
        f,
        f1: Some(f * 0.32),
        sw: Some(0.05),
        a: Some(0.0015),
        d: Some(0.06),
        peak: Some(0.45),
        ..Default::default()
    });
    let n = if v.rng.next_f64() < 0.5 { 2 } else { 1 };
    plips(v, 0.03, n, 0.05, [700.0, 1300.0], 0.15, p);
}

/// `splat_big` build ≡ audio.js L736.
fn b_splat_big(v: &mut Vb, p: f64) {
    big_splat(v, 0.0, p, 1.0);
}

/// `ink_hit_wall` build ≡ audio.js L737-745.
fn b_ink_hit_wall(v: &mut Vb, p: f64) {
    let f = v.r(1100.0, 1500.0) * p;
    let f1 = v.r(420.0, 560.0) * p;
    v.nz(Nz {
        f: Some(f),
        f1: Some(f1),
        sw: Some(0.06),
        q: Some(4.0),
        a: Some(0.001),
        d: Some(0.07),
        peak: Some(0.9),
        ..Default::default()
    });
    let f = v.r(850.0, 1000.0) * p;
    v.tone(Tone {
        f,
        f1: Some(f * 0.44),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.045),
        peak: Some(0.35),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(4000.0),
        a: Some(0.0005),
        d: Some(0.02),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `bomb_throw` build ≡ audio.js L746-752.
fn b_bomb_throw(v: &mut Vb, p: f64) {
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 330.0 * p,
        f1: Some(210.0 * p),
        sw: Some(0.03),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(0.4),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(500.0 * p),
        f1: Some(2200.0 * p),
        sw: Some(0.22),
        q: Some(1.5),
        a: Some(0.08),
        d: Some(0.14),
        peak: Some(0.5),
        ..Default::default()
    });
}

/// `bomb_beep` build ≡ audio.js L753-760.
fn b_bomb_beep(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 4000.0, 0.8, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 1480.0 * p,
        a: Some(0.002),
        h: Some(0.05),
        d: Some(0.03),
        peak: Some(0.22),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        f: 1480.0 * p,
        a: Some(0.002),
        h: Some(0.05),
        d: Some(0.04),
        peak: Some(0.35),
        ..Default::default()
    });
}

/// `bomb_explode` build ≡ audio.js L761-772.
fn b_bomb_explode(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 90.0 * p,
        f1: Some(30.0 * p),
        sw: Some(0.5),
        a: Some(0.002),
        d: Some(0.7),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2000.0),
        a: Some(0.0003),
        d: Some(0.04),
        peak: Some(0.9),
        ..Default::default()
    });
    let shg = v.gain(0.8, Some(v.out));
    let sh = v.shaper(2.0, Some(shg));
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(5000.0),
        f1: Some(180.0),
        sw: Some(0.6),
        q: Some(0.9),
        a: Some(0.002),
        d: Some(0.7),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.04,
        f: Some(1500.0 * p),
        f1: Some(350.0 * p),
        sw: Some(0.5),
        q: Some(3.0),
        a: Some(0.004),
        d: Some(0.6),
        peak: Some(0.7),
        ..Default::default()
    });
    bloops(v, 0.05, 5, 0.25, [350.0, 700.0], 0.3, p);
    v.nz(Nz {
        t: 0.06,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.04),
        d: Some(0.7),
        peak: Some(0.3),
        ..Default::default()
    });
}

/* ---- Squid (audio.js L775-839) ---- */

/// `squid_in` build ≡ audio.js L775-782.
fn b_squid_in(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 950.0 * p,
        f1: Some(240.0 * p),
        sw: Some(0.09),
        a: Some(0.002),
        d: Some(0.12),
        peak: Some(0.7),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(900.0 * p),
        f1: Some(380.0 * p),
        sw: Some(0.1),
        q: Some(6.0),
        a: Some(0.004),
        d: Some(0.1),
        peak: Some(0.55),
        ..Default::default()
    });
    plips(v, 0.05, 2, 0.05, [900.0, 1400.0], 0.18, p);
}

/// `squid_out` build ≡ audio.js L783-790.
fn b_squid_out(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 260.0 * p,
        f1: Some(1050.0 * p),
        sw: Some(0.07),
        a: Some(0.002),
        d: Some(0.09),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.25),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(600.0 * p),
        f1: Some(1400.0 * p),
        sw: Some(0.06),
        q: Some(4.0),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.4),
        ..Default::default()
    });
}

/// `swim` loop ≡ audio.js L791-803 (the `pitch(q)` param callback is a
/// runtime control; the baked burst uses q = 1).
fn l_swim(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 1300.0, 0.9, Some(v.out), None);
    let bg = v.gain(1.0, Some(lp));
    v.buffer(texture("bubbles"), t, None, Some(bg), p, None);
    v.lfo(0.7, 420.0, ParamRef::Freq(lp), t, None, Wave::Sine);
    let rum = v.gain(0.35, Some(v.out));
    let lp2 = v.filter(BqKind::Lowpass, 300.0, 1.0, Some(rum), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp2), 1.0);
    v.lfo(0.31, 0.12, ParamRef::Gain(rum), t, None, Wave::Sine);
}

/// `climb` loop ≡ audio.js L804-815.
fn l_climb(v: &mut Vb, p: f64) {
    let t = v.t;
    let bp = v.filter(BqKind::Bandpass, 1800.0, 0.8, Some(v.out), None);
    let bg = v.gain(1.2, Some(bp));
    v.buffer(texture("bubbles_bright"), t, None, Some(bg), p, None);
    let sc = v.gain(0.12, Some(v.out));
    let bp2 = v.filter(BqKind::Bandpass, 1200.0, 2.0, Some(sc), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp2), 1.0);
    v.lfo(7.0 * p, 0.1, ParamRef::Gain(sc), t, None, Wave::Sine);
}

/// `swim_splash` build ≡ audio.js L816-823.
fn b_swim_splash(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(1600.0 * p),
        f1: Some(700.0 * p),
        sw: Some(0.12),
        q: Some(2.0),
        a: Some(0.002),
        d: Some(0.16),
        peak: Some(0.8),
        ..Default::default()
    });
    v.tone(Tone {
        f: 500.0 * p,
        f1: Some(200.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.3),
        ..Default::default()
    });
    plips(v, 0.02, 3, 0.1, [700.0, 1500.0], 0.16, p);
}

/// `jump` build ≡ audio.js L824-831.
fn b_jump(v: &mut Vb, p: f64) {
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 280.0 * p,
        f1: Some(620.0 * p),
        sw: Some(0.09),
        a: Some(0.003),
        d: Some(0.1),
        peak: Some(0.35),
        ..Default::default()
    });
    v.tone(Tone {
        f: 280.0 * p,
        f1: Some(620.0 * p),
        sw: Some(0.09),
        a: Some(0.003),
        d: Some(0.1),
        peak: Some(0.3),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(800.0),
        f1: Some(1800.0),
        sw: Some(0.1),
        q: Some(1.5),
        a: Some(0.02),
        d: Some(0.08),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `land` build ≡ audio.js L832-839.
fn b_land(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 140.0 * p,
        f1: Some(55.0 * p),
        sw: Some(0.07),
        a: Some(0.001),
        d: Some(0.1),
        peak: Some(0.8),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(750.0 * p),
        f1: Some(420.0 * p),
        sw: Some(0.06),
        q: Some(3.0),
        a: Some(0.002),
        d: Some(0.08),
        peak: Some(0.5),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.0005),
        d: Some(0.015),
        peak: Some(0.15),
        ..Default::default()
    });
}

/* ---- Combat feedback (audio.js L845-929) ---- */

/// `hit_marker` build ≡ audio.js L845-857. The per-context combo counter
/// (L844-851) is runtime state; baking uses its initial value
/// `{ t: -9, n: 0 }` ⇒ q = p, lift = 1 (declared deviation).
fn b_hit_marker(v: &mut Vb, p: f64) {
    let q = p;
    let lift = 1.0;
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(4500.0),
        a: Some(0.0003),
        d: Some(0.007),
        peak: Some(0.42),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(3400.0 * q),
        q: Some(4.0),
        a: Some(0.0006),
        d: Some(0.022),
        peak: Some(0.32),
        ..Default::default()
    });
    v.tone(Tone {
        f: 1650.0 * q,
        f1: Some(1250.0 * q),
        sw: Some(0.02),
        a: Some(0.0008),
        d: Some(0.05),
        peak: Some(0.62 * lift),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 3300.0 * q,
        a: Some(0.0005),
        d: Some(0.025),
        peak: Some(0.16 * lift),
        ..Default::default()
    });
}

/// `ink_hit_body` build ≡ audio.js L859-868.
fn b_ink_hit_body(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 2600.0, 0.8, Some(v.out), None);
    v.tone(Tone {
        f: 190.0 * p,
        f1: Some(70.0 * p),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.85),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(1300.0 * p),
        f1: Some(520.0 * p),
        sw: Some(0.07),
        q: Some(3.0),
        a: Some(0.001),
        d: Some(0.08),
        peak: Some(0.8),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(900.0),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(0.5),
        to: Some(lp),
        ..Default::default()
    });
    let t = v.t + v.r(0.015, 0.03);
    let f = v.r(700.0, 1100.0) * p;
    v.bub(t, f, 0.14, 0.025, 1.7, None);
}

/// `hurt` build ≡ audio.js L869-877.
fn b_hurt(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 1400.0, 0.8, Some(v.out), None);
    v.tone(Tone {
        f: 130.0 * p,
        f1: Some(50.0 * p),
        sw: Some(0.12),
        a: Some(0.002),
        d: Some(0.18),
        peak: Some(0.9),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(700.0 * p),
        f1: Some(260.0 * p),
        sw: Some(0.14),
        q: Some(5.0),
        a: Some(0.003),
        d: Some(0.16),
        peak: Some(0.8),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(600.0),
        a: Some(0.002),
        d: Some(0.1),
        peak: Some(0.5),
        to: Some(lp),
        ..Default::default()
    });
}

/// `splat_enemy` build ≡ audio.js L878-893.
fn b_splat_enemy(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.0003),
        d: Some(0.02),
        peak: Some(0.45),
        ..Default::default()
    });
    v.tone(Tone {
        f: 780.0 * p,
        f1: Some(170.0 * p),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(2000.0),
        f1: Some(700.0),
        sw: Some(0.1),
        q: Some(2.0),
        a: Some(0.001),
        d: Some(0.1),
        peak: Some(0.6),
        ..Default::default()
    });
    v.tone(Tone {
        f: 110.0,
        f1: Some(45.0),
        sw: Some(0.1),
        a: Some(0.001),
        d: Some(0.15),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.005,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(2400.0),
        f1: Some(300.0),
        sw: Some(0.2),
        q: Some(1.2),
        a: Some(0.002),
        d: Some(0.22),
        peak: Some(0.5),
        ..Default::default()
    });
    for (i, m) in [88.0, 92.0, 95.0, 100.0].iter().enumerate() {
        let f = synth::mtof(*m) * p;
        let t = 0.03 + i as f64 * 0.045;
        v.tone(Tone {
            t,
            f,
            a: Some(0.001),
            d: Some(0.35),
            peak: Some(0.26),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: f * 2.76,
            a: Some(0.001),
            d: Some(0.12),
            peak: Some(0.07),
            ..Default::default()
        });
    }
    v.nz(Nz {
        t: 0.03,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(7000.0),
        a: Some(0.05),
        d: Some(0.3),
        peak: Some(0.08),
        ..Default::default()
    });
}

/// `splatted_self` build ≡ audio.js L894-907.
fn b_splatted_self(v: &mut Vb, p: f64) {
    big_splat(v, 0.0, p, 1.1);
    let t = v.t + 0.12;
    let g = v.gain(0.0, Some(v.out));
    let end = synth::adsr(v.g.param(g), t, 0.02, 0.3, 0.8, 0.22, 0.9, 0.25);
    let lp = v.filter(BqKind::Lowpass, 2000.0, 3.0, Some(g), None);
    v.g.param(lp).set(2000.0, t);
    v.g.param(lp).exp(700.0, t + 1.0);
    v.lfo(5.0, 300.0, ParamRef::Freq(lp), t, Some(end), Wave::Sine);
    let o = v.osc(Wave::Square, 620.0 * p, t, Some(end + 0.01), Some(lp));
    synth::sweep(v.g.param(o), t, 620.0 * p, 150.0 * p, 0.95, false);
    v.lfo(6.0, 30.0, ParamRef::Detune(o), t, Some(end), Wave::Sine);
}

/// `ally_splatted` build ≡ audio.js L908-916.
fn b_ally_splatted(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 1800.0, 0.7, Some(v.out), None);
    v.nz(Nz {
        f: Some(1200.0),
        f1: Some(400.0),
        sw: Some(0.15),
        q: Some(3.0),
        a: Some(0.002),
        d: Some(0.15),
        peak: Some(0.45),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.02,
        wave: Some(Wave::Triangle),
        f: synth::mtof(74.0) * p,
        a: Some(0.005),
        d: Some(0.25),
        peak: Some(0.35),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.13,
        wave: Some(Wave::Triangle),
        f: synth::mtof(69.0) * p,
        a: Some(0.005),
        d: Some(0.35),
        peak: Some(0.35),
        ..Default::default()
    });
}

/// `enemy_ink_sizzle` loop ≡ audio.js L917-929.
fn l_enemy_ink_sizzle(v: &mut Vb, p: f64) {
    let t = v.t;
    let hp = v.filter(BqKind::Highpass, 2000.0, 0.7, Some(v.out), None);
    let bg = v.gain(1.0, Some(hp));
    v.buffer(texture("sizzle"), t, None, Some(bg), p, None);
    let fz = v.gain(0.1, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 5000.0, 0.7, Some(fz), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp), 1.0);
    v.lfo(11.0, 0.05, ParamRef::Gain(fz), t, None, Wave::Sine);
    v.lfo(17.3, 0.04, ParamRef::Gain(fz), t, None, Wave::Sine);
    let rum = v.gain(0.14, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 400.0, 1.0, Some(rum), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp), 1.0);
}

/* ---- Status (audio.js L932-1038) ---- */

/// `low_ink` build ≡ audio.js L932-940.
fn b_low_ink(v: &mut Vb, p: f64) {
    for t in [0.0, 0.15] {
        v.tone(Tone {
            t,
            f: 520.0 * p,
            f1: Some(300.0 * p),
            sw: Some(0.07),
            a: Some(0.002),
            d: Some(0.1),
            peak: Some(0.5),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            wave: Some(Wave::Triangle),
            f: 260.0 * p,
            f1: Some(150.0 * p),
            sw: Some(0.07),
            a: Some(0.002),
            d: Some(0.08),
            peak: Some(0.2),
            ..Default::default()
        });
    }
}

/// `empty_click` build ≡ audio.js L941-950.
fn b_empty_click(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(2400.0 * p),
        q: Some(2.0),
        a: Some(0.0003),
        d: Some(0.006),
        peak: Some(0.8),
        ..Default::default()
    });
    let lp = v.filter(BqKind::Lowpass, 900.0, 1.0, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 190.0 * p,
        a: Some(0.0005),
        d: Some(0.018),
        peak: Some(0.3),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.028,
        f: Some(3200.0 * p),
        q: Some(2.0),
        a: Some(0.0003),
        d: Some(0.005),
        peak: Some(0.4),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.035,
        f: Some(2800.0 * p),
        f1: Some(1600.0 * p),
        sw: Some(0.06),
        q: Some(1.4),
        a: Some(0.004),
        d: Some(0.06),
        peak: Some(0.2),
        ..Default::default()
    });
    v.bub(v.t + 0.05, 1300.0 * p, 0.06, 0.02, 1.6, None);
}

/// `refill_full` build ≡ audio.js L951-958.
fn b_refill_full(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 400.0 * p,
        f1: Some(1200.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.4),
        ..Default::default()
    });
    bell(
        v,
        v.t + 0.05,
        synth::mtof(84.0) * p,
        0.35,
        0.45,
        3.5,
        1.2,
        None,
    );
    bell(
        v,
        v.t + 0.11,
        synth::mtof(91.0) * p,
        0.35,
        0.55,
        3.5,
        1.2,
        None,
    );
}

/// `special_ready` build ≡ audio.js L959-971.
fn b_special_ready(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 3500.0, 1.0, Some(v.out), None);
    for (i, m) in [60.0, 64.0, 67.0, 72.0, 76.0, 79.0, 84.0]
        .iter()
        .enumerate()
    {
        v.tone(Tone {
            t: i as f64 * 0.045,
            wave: Some(Wave::Pulse(0.25)),
            f: synth::mtof(*m) * p,
            a: Some(0.002),
            d: Some(0.18),
            peak: Some(0.22),
            to: Some(lp),
            ..Default::default()
        });
    }
    bell(
        v,
        v.t + 0.32,
        synth::mtof(96.0) * p,
        0.3,
        0.7,
        3.5,
        1.6,
        None,
    );
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 800.0, 2.0, Some(g), None);
    synth::sweep(v.g.param(bp), v.t, 800.0, 6000.0, 0.32, false);
    synth::pts(v.g.param(g), v.t, &[(0.0, 0.0), (0.3, 0.35), (0.36, 0.0)]);
    v.noise(NoiseKind::White, v.t, Some(v.t + 0.38), Some(bp), 1.0);
    for m in [72.0, 76.0, 79.0] {
        pad(
            v,
            v.t + 0.3,
            synth::mtof(m) * p,
            0.25,
            0.1,
            0.02,
            0.4,
            2600.0,
            None,
        );
    }
}

/// `special_activate` build ≡ audio.js L972-984.
fn b_special_activate(v: &mut Vb, p: f64) {
    let t = v.t;
    v.nz(Nz {
        f: Some(300.0),
        f1: Some(3000.0),
        sw: Some(0.35),
        q: Some(2.0),
        a: Some(0.25),
        d: Some(0.15),
        peak: Some(0.6),
        ..Default::default()
    });
    let g = v.gain(0.0, Some(v.out));
    let end = synth::ahr(v.g.param(g), t, 0.05, 0.25, 0.25, 0.2);
    let lp = v.filter(BqKind::Lowpass, 600.0, 2.0, Some(g), None);
    synth::sweep(v.g.param(lp), t, 600.0, 4000.0, 0.35, false);
    for r in [1.0, 1.5] {
        let o = v.osc(Wave::Saw, 110.0 * r * p, t, Some(end + 0.01), Some(lp));
        synth::sweep(v.g.param(o), t, 110.0 * r * p, 440.0 * r * p, 0.35, false);
    }
    v.tone(Tone {
        t: 0.35,
        f: 120.0,
        f1: Some(40.0),
        sw: Some(0.25),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.35,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(3000.0),
        f1: Some(200.0),
        sw: Some(0.3),
        q: Some(1.0),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(0.6),
        ..Default::default()
    });
}

/// `special_slam` build ≡ audio.js L985-996.
fn b_special_slam(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 70.0 * p,
        f1: Some(24.0 * p),
        sw: Some(0.9),
        a: Some(0.002),
        d: Some(1.1),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1800.0),
        a: Some(0.0003),
        d: Some(0.05),
        peak: Some(1.0),
        ..Default::default()
    });
    let shg = v.gain(0.7, Some(v.out));
    let sh = v.shaper(2.5, Some(shg));
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(6000.0),
        f1: Some(150.0),
        sw: Some(0.8),
        q: Some(0.8),
        a: Some(0.002),
        d: Some(1.0),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.05,
        f: Some(900.0),
        f1: Some(250.0),
        sw: Some(0.9),
        q: Some(2.0),
        a: Some(0.01),
        d: Some(1.0),
        peak: Some(0.6),
        ..Default::default()
    });
    bloops(v, 0.06, 6, 0.4, [300.0, 700.0], 0.3, p);
    v.nz(Nz {
        t: 0.08,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.05),
        d: Some(0.9),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `storm_rain` loop ≡ audio.js L997-1008.
fn l_storm_rain(v: &mut Vb, p: f64) {
    let t = v.t;
    let hp = v.filter(BqKind::Highpass, 900.0, 0.7, Some(v.out), None);
    let bg = v.gain(1.0, Some(hp));
    v.buffer(texture("rain"), t, None, Some(bg), p, None);
    let hs = v.gain(0.22, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 3000.0, 0.5, Some(hs), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp), 1.0);
    v.lfo(0.23, 0.06, ParamRef::Gain(hs), t, None, Wave::Sine);
    let rum = v.gain(0.3, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 180.0, 1.0, Some(rum), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp), 1.0);
}

/// `storm_thunder` build ≡ audio.js L1009-1020.
fn b_storm_thunder(v: &mut Vb, p: f64) {
    let n = 3 + v.r(0.0, 3.0).floor() as usize;
    for _ in 0..n {
        let t = v.r(0.0, 0.25);
        let d = v.r(0.03, 0.08);
        let peak = v.r(0.3, 0.6);
        v.nz(Nz {
            t,
            ft: Some(Some(BqKind::Highpass)),
            f: Some(1500.0),
            a: Some(0.001),
            d: Some(d),
            peak: Some(peak),
            ..Default::default()
        });
    }
    v.nz(Nz {
        kind: Some(NoiseKind::Brown),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(400.0 * p),
        f1: Some(120.0 * p),
        sw: Some(2.0),
        q: Some(1.0),
        a: Some(0.05),
        d: Some(2.2),
        peak: Some(1.0),
        ..Default::default()
    });
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        v.t,
        &[
            (0.0, 0.0),
            (0.3, 1.0),
            (0.6, 0.5),
            (0.9, 0.8),
            (1.6, 0.2),
            (2.4, 0.0),
        ],
    );
    let lp = v.filter(BqKind::Lowpass, 250.0, 1.0, Some(g), None);
    v.noise(NoiseKind::Brown, v.t, Some(v.t + 2.45), Some(lp), 1.0);
    v.tone(Tone {
        f: 60.0 * p,
        f1: Some(35.0 * p),
        sw: Some(0.6),
        a: Some(0.01),
        d: Some(0.8),
        peak: Some(0.6),
        ..Default::default()
    });
}

/// `respawn` build ≡ audio.js L1021-1029.
fn b_respawn(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 300.0 * p,
        f1: Some(1300.0 * p),
        sw: Some(0.35),
        a: Some(0.03),
        h: Some(0.22),
        d: Some(0.2),
        peak: Some(0.3),
        ..Default::default()
    });
    for i in 0..8 {
        let t = v.t + i as f64 * 0.05 + v.r(0.0, 0.02);
        v.bub(t, (500.0 + i as f64 * 140.0) * p, 0.14, 0.03, 1.8, None);
    }
    for (i, m) in [79.0, 84.0, 88.0].iter().enumerate() {
        bell(
            v,
            v.t + 0.35 + i as f64 * 0.03,
            synth::mtof(*m) * p,
            0.25,
            0.6,
            3.5,
            1.6,
            None,
        );
    }
    v.nz(Nz {
        f: Some(600.0),
        f1: Some(3000.0),
        sw: Some(0.4),
        q: Some(1.5),
        a: Some(0.3),
        d: Some(0.15),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `super_jump` build ≡ audio.js L1030-1038.
fn b_super_jump(v: &mut Vb, p: f64) {
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 200.0 * p,
        f1: Some(900.0 * p),
        sw: Some(0.25),
        a: Some(0.01),
        d: Some(0.28),
        peak: Some(0.35),
        ..Default::default()
    });
    v.tone(Tone {
        f: 600.0 * p,
        f1: Some(2400.0 * p),
        sw: Some(0.7),
        a: Some(0.05),
        h: Some(0.4),
        d: Some(0.3),
        peak: Some(0.2),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(400.0),
        f1: Some(2500.0),
        sw: Some(0.5),
        q: Some(1.2),
        a: Some(0.02),
        d: Some(0.6),
        peak: Some(0.6),
        ..Default::default()
    });
    v.tone(Tone {
        f: 110.0,
        f1: Some(50.0),
        sw: Some(0.15),
        a: Some(0.002),
        d: Some(0.2),
        peak: Some(0.7),
        ..Default::default()
    });
}

/* ---- Specials (audio.js L1081-…) ---- */

/// `special_end` build ≡ audio.js L1081-1098.
fn b_special_end(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let lp = v.filter(BqKind::Lowpass, 5000.0, 0.8, Some(v.out), None);
    synth::sweep(v.g.param(lp), t_abs, 5000.0, 900.0, 0.5, false);
    for (i, m) in [84.0, 79.0, 76.0, 72.0, 67.0, 60.0].iter().enumerate() {
        let f = synth::mtof(*m) * p;
        v.tone(Tone {
            t: (i as f64) * 0.05,
            wave: Some(Wave::Pulse(0.25)),
            f,
            f1: Some(f * 0.94),
            sw: Some(0.15),
            a: Some(0.002),
            d: Some(0.15),
            peak: Some(0.2),
            to: Some(lp),
            ..Default::default()
        });
    }
    let g = v.gain(0.0, Some(v.out));
    let end = synth::ahr(v.g.param(g), t_abs, 0.02, 0.22, 0.18, 0.3); // falling glide
    let o = v.osc(Wave::Triangle, 1400.0 * p, t_abs, Some(end + 0.01), Some(g));
    synth::sweep(v.g.param(o), t_abs, 1400.0 * p, 200.0 * p, 0.5, false);
    v.lfo(9.0, 25.0, ParamRef::Detune(o), t_abs, Some(end), Wave::Sine);
    let ng = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 7000.0, 1.5, Some(ng), None); // falling air
    synth::sweep(v.g.param(bp), t_abs, 7000.0, 500.0, 0.5, false);
    synth::pts(v.g.param(ng), t_abs, &[(0.0, 0.0), (0.04, 0.3), (0.5, 0.0)]);
    v.noise(NoiseKind::White, t_abs, Some(t_abs + 0.52), Some(bp), 1.0);
    v.tone(Tone {
        t: 0.05,
        f: 220.0 * p,
        f1: Some(55.0 * p),
        sw: Some(0.4),
        a: Some(0.01),
        d: Some(0.4),
        peak: Some(0.35),
        ..Default::default()
    }); // power-down
}

/// `special_ending` build ≡ audio.js L1101-1114 — a looping 8-bit figure
/// that speeds up and climbs, like a power-up wearing off.
fn b_special_ending(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 5200.0, 0.7, Some(v.out), None);
    let fig = [72.0, 76.0, 79.0, 84.0, 79.0, 76.0];
    let mut t = 0.0;
    let mut i = 0usize;
    while t < 1.9 {
        let k = t / 1.9;
        let gap = 0.15 - 0.085 * k;
        let m = fig[i % fig.len()] + if k > 0.66 { 2.0 } else { 0.0 };
        v.tone(Tone {
            t,
            wave: Some(Wave::Pulse(0.25)),
            f: synth::mtof(m) * p,
            a: Some(0.002),
            h: Some(gap * 0.35),
            d: Some(gap * 0.55),
            peak: Some(0.16 + 0.08 * k),
            to: Some(lp),
            ..Default::default()
        });
        t += gap;
        i += 1;
    }
}

/// `shield_up` build ≡ audio.js L1115-1134.
fn b_shield_up(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let g = v.gain(0.0, Some(v.out));
    let end = synth::pts(
        v.g.param(g),
        t_abs,
        &[
            (0.0, 0.0),
            (0.05, 0.22),
            (0.26, 0.32),
            (0.5, 0.06),
            (0.62, 0.0),
        ],
    );
    // wobble, speeding up
    let wob_depth = v.gain(45.0, None);
    let wob_osc = v.osc(Wave::Sine, 8.0, t_abs, Some(end), Some(wob_depth));
    synth::sweep(v.g.param(wob_osc), t_abs, 8.0, 17.0, 0.3, false);
    for (i, (r, wave, a)) in [
        (1.0, Wave::Triangle, 1.0),
        (1.5, Wave::Sine, 0.5),
        (2.005, Wave::Sine, 0.35),
    ]
    .iter()
    .enumerate()
    {
        let og = v.gain(*a, Some(g));
        let o = v.osc(*wave, 300.0 * r * p, t_abs, Some(end + 0.01), Some(og));
        synth::sweep(
            v.g.param(o),
            t_abs,
            300.0 * r * p,
            900.0 * r * p,
            0.28,
            false,
        );
        v.g.param2(o).set_default((i as f64 - 1.0) * 7.0);
        v.g.connect_param(wob_depth, ParamRef::Detune(o));
    }
    whoosh(
        v,
        0.0,
        0.32,
        900.0,
        4500.0,
        6000.0,
        0.22,
        Some(2.0),
        Some(0.8),
        Some(&[[0.0, 0.0], [0.8, 1.0], [1.0, 0.0]]),
        Some(NoiseKind::White),
        None,
    );
    v.tone(Tone {
        t: 0.26,
        f: 720.0 * p,
        f1: Some(260.0 * p),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.07),
        peak: Some(0.6),
        ..Default::default()
    }); // soft pop
    v.tone(Tone {
        t: 0.26,
        f: 170.0 * p,
        f1: Some(85.0 * p),
        sw: Some(0.1),
        a: Some(0.003),
        d: Some(0.14),
        peak: Some(0.45),
        ..Default::default()
    }); // whum
    bell(
        v,
        t_abs + 0.27,
        synth::mtof(96.0) * p,
        0.1,
        0.5,
        3.5,
        1.6,
        None,
    );
    plips(v, 0.28, 2, 0.06, [1200.0, 1800.0], 0.1, p);
}

/// `shield_hit` build ≡ audio.js L1135-1151.
fn b_shield_hit(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    v.tone(Tone {
        f: 260.0 * p,
        f1: Some(120.0 * p),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.7),
        ..Default::default()
    }); // thump
    for (f, a, d) in [
        (1850.0, 0.28, 0.09),
        (2790.0, 0.18, 0.07),
        (4150.0, 0.1, 0.05),
    ] {
        v.tone(Tone {
            f: f * p,
            a: Some(0.0006),
            d: Some(d),
            peak: Some(a),
            ..Default::default()
        }); // glass
    }
    let g = v.gain(0.0, Some(v.out));
    let end = synth::perc(v.g.param(g), t_abs + 0.008, 0.004, 0.45, 0.2); // boing
    let o = v.osc(
        Wave::Sine,
        330.0 * p,
        t_abs + 0.008,
        Some(end + 0.01),
        Some(g),
    );
    synth::sweep(
        v.g.param(o),
        t_abs + 0.008,
        330.0 * p,
        520.0 * p,
        0.2,
        false,
    );
    let o2g = v.gain(0.2, Some(g));
    let o2 = v.osc(
        Wave::Triangle,
        660.0 * p,
        t_abs + 0.008,
        Some(end + 0.01),
        Some(o2g),
    );
    synth::sweep(
        v.g.param(o2),
        t_abs + 0.008,
        660.0 * p,
        1040.0 * p,
        0.2,
        false,
    );
    let (w_depth, _w_osc) = v.lfo(
        19.0,
        0.0,
        ParamRef::Detune(o),
        t_abs + 0.008,
        Some(end),
        Wave::Sine,
    );
    v.g.connect_param(w_depth, ParamRef::Detune(o2));
    synth::pts(
        v.g.param(w_depth),
        t_abs + 0.008,
        &[(0.0, 160.0), (0.2, 0.0)],
    );
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5000.0),
        a: Some(0.0004),
        d: Some(0.01),
        peak: Some(0.25),
        ..Default::default()
    });
}

/// `shield_pop` build ≡ audio.js L1152-1165.
fn b_shield_pop(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2500.0),
        a: Some(0.0004),
        d: Some(0.03),
        peak: Some(0.8),
        ..Default::default()
    });
    v.tone(Tone {
        f: 1100.0 * p,
        f1: Some(300.0 * p),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.08),
        peak: Some(0.7),
        ..Default::default()
    }); // pop
    v.tone(Tone {
        f: 200.0 * p,
        f1: Some(70.0 * p),
        sw: Some(0.12),
        a: Some(0.002),
        d: Some(0.16),
        peak: Some(0.6),
        ..Default::default()
    }); // whump
    for _ in 0..9 {
        // glassy shards
        let f = v.r(2200.0, 5200.0) * p;
        let t = 0.01 + v.r(0.0, 0.22);
        let d = v.r(0.08, 0.2);
        let peak = v.r(0.06, 0.14);
        v.tone(Tone {
            t,
            f,
            f1: Some(f * 0.9),
            sw: Some(0.1),
            a: Some(0.0006),
            d: Some(d),
            peak: Some(peak),
            ..Default::default()
        });
    }
    whoosh(
        v,
        0.0,
        0.4,
        6000.0,
        3500.0,
        1500.0,
        0.35,
        Some(3.0),
        None,
        Some(&[[0.0, 0.0], [0.03, 1.0], [1.0, 0.0]]),
        Some(NoiseKind::White),
        None,
    );
    plips(v, 0.03, 3, 0.1, [900.0, 1600.0], 0.2, p);
}

/// `sonar_ping` build ≡ audio.js L1166-1182.
fn b_sonar_ping(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let f = 523.0 * p;
    v.tone(Tone {
        f: 70.0 * p,
        f1: Some(42.0 * p),
        sw: Some(0.4),
        a: Some(0.004),
        d: Some(0.55),
        peak: Some(0.7),
        ..Default::default()
    }); // sub pulse
    for k in 0..6 {
        // ping + darkening echoes
        let t = k as f64 * 0.3;
        let a = 0.48f64.powi(k);
        let to = if k != 0 {
            v.pan(if k % 2 == 1 { 0.45 } else { -0.45 }, Some(v.out))
        } else {
            v.out
        };
        v.tone(Tone {
            t,
            f: f * 1.04,
            f1: Some(f),
            sw: Some(0.05),
            a: Some(0.003),
            d: Some(if k != 0 { 0.5 } else { 1.1 }),
            peak: Some(0.6 * a),
            to: Some(to),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: f * 2.0,
            a: Some(0.002),
            d: Some(if k != 0 { 0.12 } else { 0.35 }),
            peak: Some(if k != 0 { 0.04 } else { 0.14 } * a),
            to: Some(to),
            ..Default::default()
        });
    }
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 2400.0, 2.0, Some(g), None); // wave rolling outward
    synth::sweep(v.g.param(bp), t_abs, 2400.0, 300.0, 1.6, false);
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.08, 0.22), (0.5, 0.14), (1.7, 0.0)],
    );
    v.noise(NoiseKind::Pink, t_abs, Some(t_abs + 1.72), Some(bp), 1.0);
    bell(v, t_abs, 1568.0 * p, 0.08, 1.2, 2.01, 0.8, None);
}

/// `sonar_mark` build ≡ audio.js L1183-1193.
fn b_sonar_mark(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 3000.0, 0.8, Some(v.out), None);
    for (t, m) in [(0.0, 81.0), (0.1, 76.0)] {
        v.tone(Tone {
            t,
            wave: Some(Wave::Triangle),
            f: synth::mtof(m) * p,
            a: Some(0.006),
            h: Some(0.035),
            d: Some(0.08),
            peak: Some(0.5),
            to: Some(lp),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: synth::mtof(m) * 2.0 * p,
            a: Some(0.005),
            d: Some(0.06),
            peak: Some(0.12),
            ..Default::default()
        });
    }
    v.tone(Tone {
        t: 0.2,
        f: 523.0 * p,
        a: Some(0.006),
        d: Some(0.35),
        peak: Some(0.12),
        ..Default::default()
    }); // faint sonar tail
}

/// `strike_launch` build ≡ audio.js L1194-1211.
fn b_strike_launch(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    v.tone(Tone {
        f: 160.0 * p,
        f1: Some(55.0 * p),
        sw: Some(0.12),
        a: Some(0.002),
        d: Some(0.2),
        peak: Some(0.9),
        ..Default::default()
    }); // launch thoonk
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(2500.0),
        f1: Some(400.0),
        sw: Some(0.15),
        q: Some(1.0),
        a: Some(0.002),
        d: Some(0.15),
        peak: Some(0.7),
        ..Default::default()
    });
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 350.0 * p, 1.2, Some(g), None);
    let am = v.gain(0.75, Some(bp)); // fluttering rocket roar
    synth::sweep(v.g.param(bp), t_abs, 350.0 * p, 3200.0 * p, 1.0, false);
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.06, 0.9), (0.35, 0.7), (1.05, 0.0)],
    );
    v.lfo(
        28.0,
        0.25,
        ParamRef::Gain(am),
        t_abs,
        Some(t_abs + 1.08),
        Wave::Sine,
    );
    v.noise(NoiseKind::Pink, t_abs, Some(t_abs + 1.08), Some(am), 1.0);
    let cg = v.gain(0.0, Some(v.out)); // exhaust crackle
    synth::pts(v.g.param(cg), t_abs, &[(0.0, 0.0), (0.05, 0.4), (0.9, 0.0)]);
    let bq = v.filter(BqKind::Bandpass, 2500.0, 0.8, Some(cg), None);
    v.buffer(
        texture("sizzle"),
        t_abs,
        Some(t_abs + 0.92),
        Some(bq),
        1.0,
        None,
    );
    v.tone(Tone {
        t: 0.03,
        f: 500.0 * p,
        f1: Some(2000.0 * p),
        sw: Some(1.0),
        a: Some(0.1),
        h: Some(0.4),
        d: Some(0.45),
        peak: Some(0.08),
        ..Default::default()
    }); // rising doppler whistle
    v.nz(Nz {
        t: 0.02,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3500.0),
        a: Some(0.05),
        h: Some(0.2),
        d: Some(0.6),
        peak: Some(0.12),
        ..Default::default()
    });
}

/// `strike_whistle` build ≡ audio.js L1212-1231.
fn b_strike_whistle(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let d_len = 1.4;
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.3, 0.22), (1.2, 0.8), (1.36, 0.9), (1.4, 0.0)],
    );
    let o = v.osc(
        Wave::Sine,
        1800.0 * p,
        t_abs,
        Some(t_abs + d_len + 0.01),
        Some(g),
    );
    synth::sweep(v.g.param(o), t_abs, 1800.0 * p, 500.0 * p, d_len, false);
    v.lfo(
        6.0,
        12.0,
        ParamRef::Detune(o),
        t_abs,
        Some(t_abs + d_len),
        Wave::Sine,
    );
    let o2g = v.gain(0.12, Some(g));
    let o2 = v.osc(
        Wave::Triangle,
        1800.0 * p,
        t_abs,
        Some(t_abs + d_len + 0.01),
        Some(o2g),
    );
    synth::sweep(v.g.param(o2), t_abs, 1800.0 * p, 500.0 * p, d_len, false);
    let ng = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 1800.0 * p, 6.0, Some(ng), None); // air rush following the pitch
    synth::sweep(v.g.param(bp), t_abs, 1800.0 * p, 500.0 * p, d_len, false);
    synth::pts(v.g.param(ng), t_abs, &[(0.0, 0.0), (1.2, 0.5), (1.4, 0.0)]);
    v.noise(
        NoiseKind::White,
        t_abs,
        Some(t_abs + d_len + 0.01),
        Some(bp),
        1.0,
    );
    let rg = v.gain(0.0, Some(v.out)); // approaching rumble
    synth::pts(
        v.g.param(rg),
        t_abs,
        &[(0.0, 0.0), (0.5, 0.0), (1.35, 0.5), (1.4, 0.0)],
    );
    let rlp = v.filter(BqKind::Lowpass, 300.0, 1.0, Some(rg), None);
    v.noise(
        NoiseKind::Brown,
        t_abs,
        Some(t_abs + d_len + 0.01),
        Some(rlp),
        1.0,
    );
}

/// `strike_impact` build ≡ audio.js L1232-1235.
fn b_strike_impact(v: &mut Vb, p: f64) {
    ink_boom(
        v,
        0.0,
        p,
        1.0,
        BoomOpts {
            crack: 1.0,
            bright: 6000.0,
            ..Default::default()
        },
    );
}

/// `tornado` loop ≡ audio.js L1236-1266 (the `pitch(q)` param callback is a
/// runtime control; the baked burst uses q = 1).
fn l_tornado(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let g1 = v.gain(0.7, Some(v.out));
    let bp1 = v.filter(BqKind::Bandpass, 650.0 * p, 1.4, Some(g1), None); // swirling wind
    v.noise(NoiseKind::Pink, t_abs, None, Some(bp1), 1.0);
    let (s1_depth, s1_osc) = v.lfo(
        1.2 * p,
        380.0 * p,
        ParamRef::Freq(bp1),
        t_abs,
        None,
        Wave::Sine,
    );
    let s1a = v.gain(0.3, None); // louder as it brightens
    v.g.connect_param(s1a, ParamRef::Gain(g1));
    v.g.connect(s1_osc, s1a);
    let g2 = v.gain(0.18 * p, Some(v.out));
    let bp2 = v.filter(BqKind::Bandpass, 2200.0 * p, 2.2, Some(g2), None); // airy counter-swirl
    v.noise(NoiseKind::White, t_abs, None, Some(bp2), 1.0);
    let (s2_depth, s2_osc) = v.lfo(
        1.9 * p,
        1100.0 * p,
        ParamRef::Freq(bp2),
        t_abs,
        None,
        Wave::Sine,
    );
    let s2a = v.gain(0.08 * p, None);
    v.g.connect_param(s2a, ParamRef::Gain(g2));
    v.g.connect(s2_osc, s2a);
    let lg = v.gain(0.55, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 1300.0 * p, 0.8, Some(lg), None); // churning liquid
    v.buffer(texture("squelch"), t_abs, None, Some(lp), p, None);
    v.lfo(0.63, 0.2, ParamRef::Gain(lg), t_abs, None, Wave::Sine);
    let bg = v.gain(0.25, Some(v.out));
    let bub_lp = v.filter(BqKind::Lowpass, 1800.0, 0.7, Some(bg), None);
    v.buffer(texture("bubbles"), t_abs, None, Some(bub_lp), 0.8 * p, None);
    let rlp_g = v.gain(0.55, Some(v.out));
    let rlp = v.filter(BqKind::Lowpass, 170.0 * p, 1.0, Some(rlp_g), None); // low rumble
    v.noise(NoiseKind::Brown, t_abs, None, Some(rlp), 1.0);
    let _ = (s1_depth, s2_depth);
}

/// `zooka_fire` build ≡ audio.js L1267-1283.
fn b_zooka_fire(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    v.tone(Tone {
        f: 130.0 * p,
        f1: Some(40.0 * p),
        sw: Some(0.18),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(1.0),
        ..Default::default()
    }); // thump
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1800.0),
        f1: Some(250.0),
        sw: Some(0.15),
        q: Some(1.5),
        a: Some(0.002),
        d: Some(0.16),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2500.0),
        a: Some(0.0005),
        d: Some(0.03),
        peak: Some(0.5),
        ..Default::default()
    });
    let g = v.gain(0.0, Some(v.out));
    let am = v.gain(0.55, Some(g));
    let bp = v.filter(BqKind::Bandpass, 350.0 * p, 2.0, Some(am), None); // rising swirl, spinning faster
    synth::sweep(
        v.g.param(bp),
        t_abs + 0.03,
        350.0 * p,
        2600.0 * p,
        0.7,
        false,
    );
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.08, 0.6), (0.45, 0.8), (0.9, 0.0)],
    );
    let (_spin_depth, spin_osc) = v.lfo(
        7.0,
        0.45,
        ParamRef::Gain(am),
        t_abs,
        Some(t_abs + 0.92),
        Wave::Sine,
    );
    synth::sweep(v.g.param(spin_osc), t_abs, 7.0, 22.0, 0.8, false);
    v.noise(NoiseKind::Pink, t_abs, Some(t_abs + 0.92), Some(bp), 1.0);
    v.tone(Tone {
        t: 0.04,
        wave: Some(Wave::Triangle),
        f: 260.0 * p,
        f1: Some(1100.0 * p),
        sw: Some(0.7),
        a: Some(0.08),
        h: Some(0.3),
        d: Some(0.35),
        peak: Some(0.1),
        ..Default::default()
    });
    bloops(v, 0.03, 3, 0.1, [350.0, 650.0], 0.25, p);
}

/// `wail_charge` build ≡ audio.js L1284-1310.
fn b_wail_charge(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let e_abs = t_abs + 1.24;
    let g = v.gain(0.0, Some(v.out)); // rising electrical whine
    synth::pts(
        v.g.param(g),
        t_abs,
        &[
            (0.0, 0.0),
            (0.1, 0.12),
            (1.05, 0.45),
            (1.15, 0.45),
            (1.22, 0.0),
        ],
    );
    let lp = v.filter(BqKind::Lowpass, 900.0, 2.0, Some(g), None);
    synth::sweep(v.g.param(lp), t_abs, 900.0, 5000.0, 1.15, false);
    for (r, dt, a) in [(1.0, -8.0, 1.0), (1.0, 8.0, 1.0), (2.0, 0.0, 0.3)] {
        let dest = if a < 1.0 { v.gain(a, Some(lp)) } else { lp };
        let o = v.osc(
            if r > 1.0 { Wave::Square } else { Wave::Saw },
            110.0 * r * p,
            t_abs,
            Some(e_abs),
            Some(dest),
        );
        v.g.param2(o).set_default(dt);
        synth::sweep(
            v.g.param(o),
            t_abs,
            110.0 * r * p,
            660.0 * r * p,
            1.15,
            false,
        );
    }
    let hg = v.gain(0.0, Some(v.out)); // mains buzz
    synth::pts(
        v.g.param(hg),
        t_abs,
        &[(0.0, 0.0), (0.2, 0.1), (1.2, 0.15), (1.22, 0.0)],
    );
    let hlp = v.filter(BqKind::Lowpass, 400.0, 1.0, Some(hg), None);
    v.osc(Wave::Square, 60.0 * p, t_abs, Some(e_abs), Some(hlp));
    let sg = v.gain(0.0, Some(v.out));
    let sam = v.gain(0.8, Some(sg)); // throbbing sub
    synth::pts(
        v.g.param(sg),
        t_abs,
        &[(0.0, 0.0), (1.1, 0.75), (1.22, 0.0)],
    );
    let so = v.osc(Wave::Sine, 38.0 * p, t_abs, Some(e_abs), Some(sam));
    synth::sweep(v.g.param(so), t_abs, 38.0 * p, 55.0 * p, 1.2, false);
    let (_th_depth, th_osc) = v.lfo(
        4.0,
        0.2,
        ParamRef::Gain(sam),
        t_abs,
        Some(e_abs),
        Wave::Sine,
    );
    synth::sweep(v.g.param(th_osc), t_abs, 4.0, 11.0, 1.2, false);
    let cg = v.gain(0.0, Some(v.out)); // arcing crackle
    synth::pts(
        v.g.param(cg),
        t_abs,
        &[(0.0, 0.0), (0.3, 0.05), (1.1, 0.25), (1.2, 0.0)],
    );
    let cbp = v.filter(BqKind::Bandpass, 3000.0, 0.8, Some(cg), None);
    v.buffer(texture("sizzle"), t_abs, Some(e_abs), Some(cbp), 1.0, None);
    v.tone(Tone {
        t: 1.12,
        f: 140.0 * p,
        f1: Some(60.0 * p),
        sw: Some(0.08),
        a: Some(0.002),
        d: Some(0.12),
        peak: Some(0.5),
        ..Default::default()
    }); // armed
}

/// `wail_blast` loop ≡ audio.js L1311-1340 (the `pitch(q)` param callback is
/// a runtime control; the baked burst uses q = 1).
fn l_wail_blast(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let w_len = 5.2;
    let bg = v.gain(0.55, Some(v.out));
    let sh = v.shaper(2.2, Some(bg));
    let lp = v.filter(BqKind::Lowpass, 420.0, 5.0, Some(sh), None); // wub bass
    for (r, dt, wave) in [
        (1.0, -12.0, Wave::Saw),
        (1.0, 12.0, Wave::Saw),
        (0.5, 0.0, Wave::Square),
    ] {
        let dest = if r < 1.0 { v.gain(0.5, Some(lp)) } else { lp };
        let o = v.osc(wave, 55.0 * r * p, t_abs, None, Some(dest));
        v.g.param2(o).set_default(dt);
    }
    let (_wub_depth, wub_osc) = v.lfo(
        w_len * p,
        320.0,
        ParamRef::Freq(lp),
        t_abs,
        None,
        Wave::Sine,
    );
    let sg = v.gain(0.6, Some(v.out)); // sub throb
    v.osc(Wave::Sine, 55.0 * p, t_abs, None, Some(sg));
    let sam = v.gain(0.5, None);
    v.g.connect_param(sam, ParamRef::Gain(sg));
    v.g.connect(wub_osc, sam);
    let rg = v.gain(0.22, Some(v.out));
    let rlp = v.filter(BqKind::Lowpass, 3200.0, 0.7, Some(rg), None);
    let rsh = v.shaper(4.0, Some(rlp)); // distorted roar
    let rg2 = v.gain(2.5, Some(rsh));
    let rbp = v.filter(BqKind::Bandpass, 700.0 * p, 0.9, Some(rg2), None);
    v.noise(NoiseKind::Pink, t_abs, None, Some(rbp), 1.0);
    let ram = v.gain(0.12, None);
    v.g.connect_param(ram, ParamRef::Gain(rg));
    v.g.connect(wub_osc, ram);
    let wg = v.gain(0.07, Some(v.out)); // wailing siren edge
    let wlp = v.filter(BqKind::Lowpass, 1800.0, 0.7, Some(wg), None);
    let wo = v.osc(Wave::Triangle, 220.0 * p, t_abs, None, Some(wlp));
    v.lfo(
        w_len * p,
        60.0,
        ParamRef::Detune(wo),
        t_abs,
        None,
        Wave::Sine,
    );
}

/// `kraken_on` build ≡ audio.js L1341-1360.
fn b_kraken_on(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let t0 = t_abs + 0.2;
    let d_len = 0.85;
    v.nz(Nz {
        f: Some(250.0),
        f1: Some(1800.0),
        sw: Some(0.22),
        q: Some(1.5),
        a: Some(0.2),
        d: Some(0.08),
        peak: Some(0.4),
        ..Default::default()
    }); // transform swell
    v.tone(Tone {
        t: 0.2,
        f: 110.0 * p,
        f1: Some(36.0 * p),
        sw: Some(0.3),
        a: Some(0.002),
        d: Some(0.4),
        peak: Some(0.9),
        ..Default::default()
    }); // whump
    let g = v.gain(0.0, Some(v.out)); // gargling growl
    let end = synth::pts(
        v.g.param(g),
        t0,
        &[(0.0, 0.0), (0.06, 0.8), (0.45, 0.65), (0.85, 0.0)],
    );
    let sh = v.shaper(2.5, Some(g));
    let am = v.gain(0.6, Some(sh));
    v.lfo(27.0, 0.4, ParamRef::Gain(am), t0, Some(end), Wave::Sine);
    let (srcs, fs) = vox(
        v,
        t0,
        end + 0.01,
        70.0 * p,
        &[[550.0, 4.0, 1.2], [1000.0, 5.0, 0.7], [250.0, 1.0, 0.5]],
        am,
        14.0,
    );
    for o in srcs {
        v.g.param(o).set(70.0 * p, t0);
        v.g.param(o).exp(96.0 * p, t0 + 0.25);
        v.g.param(o).exp(58.0 * p, t0 + d_len);
    }
    v.g.param(fs[0]).set(480.0, t0);
    v.g.param(fs[0]).exp(820.0, t0 + 0.25);
    v.g.param(fs[0]).exp(560.0, t0 + d_len);
    v.g.param(fs[1]).set(900.0, t0);
    v.g.param(fs[1]).exp(1250.0, t0 + 0.25);
    v.g.param(fs[1]).exp(950.0, t0 + d_len);
    let bg = v.gain(0.0, Some(v.out)); // gloop
    synth::pts(
        v.g.param(bg),
        t0,
        &[(0.0, 0.0), (0.05, 0.7), (0.7, 0.3), (0.95, 0.0)],
    );
    let blp = v.filter(BqKind::Lowpass, 1500.0, 0.8, Some(bg), None);
    v.buffer(
        texture("squelch"),
        t0,
        Some(t0 + 0.97),
        Some(blp),
        0.8 * p,
        None,
    );
    bloops(v, 0.22, 6, 0.6, [220.0, 480.0], 0.35, p);
}

/// `kraken_jump` build ≡ audio.js L1361-1377.
fn b_kraken_jump(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    v.tone(Tone {
        f: 120.0 * p,
        f1: Some(60.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.1),
        peak: Some(0.7),
        ..Default::default()
    }); // push-off thud
    let g = v.gain(0.0, Some(v.out));
    let end = synth::perc(v.g.param(g), t_abs, 0.006, 0.8, 0.32); // boing
    let o = v.osc(Wave::Sine, 110.0 * p, t_abs, Some(end + 0.01), Some(g));
    synth::sweep(v.g.param(o), t_abs, 110.0 * p, 330.0 * p, 0.22, false);
    let o2g = v.gain(0.3, Some(g));
    let o2 = v.osc(
        Wave::Triangle,
        220.0 * p,
        t_abs,
        Some(end + 0.01),
        Some(o2g),
    );
    synth::sweep(v.g.param(o2), t_abs, 220.0 * p, 660.0 * p, 0.22, false);
    let (w_depth, _w_osc) = v.lfo(15.0, 0.0, ParamRef::Detune(o), t_abs, Some(end), Wave::Sine);
    v.g.connect_param(w_depth, ParamRef::Detune(o2));
    synth::pts(v.g.param(w_depth), t_abs, &[(0.0, 250.0), (0.3, 0.0)]);
    v.nz(Nz {
        f: Some(500.0 * p),
        f1: Some(1500.0 * p),
        sw: Some(0.15),
        q: Some(4.0),
        a: Some(0.004),
        d: Some(0.16),
        peak: Some(0.6),
        ..Default::default()
    }); // squelch
    bloops(v, 0.02, 3, 0.1, [300.0, 550.0], 0.25, p);
}

/// `kraken_slam` build ≡ audio.js L1378-1385.
fn b_kraken_slam(v: &mut Vb, p: f64) {
    ink_boom(
        v,
        0.0,
        p * 0.9,
        1.0,
        BoomOpts {
            crack: 0.5,
            bright: 4000.0,
            bloops: 9,
            plips: 6,
            ..Default::default()
        },
    );
    big_splat(v, 0.018, p * 0.8, 0.6);
    v.nz(Nz {
        t: 0.02,
        f: Some(900.0 * p),
        f1: Some(260.0 * p),
        sw: Some(0.6),
        q: Some(4.0),
        a: Some(0.004),
        d: Some(0.6),
        peak: Some(0.5),
        ..Default::default()
    }); // sloppy squelch tail
}

/// `blower_inflate` loop ≡ audio.js L1386-1411 (the `pitch(q)` param
/// callback is a runtime control; the baked burst uses q = 1).
fn l_blower_inflate(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let f0 = 150.0;
    let rg = v.gain(0.5, Some(v.out));
    let am = v.gain(0.75, Some(rg));
    let lp = v.filter(BqKind::Lowpass, 700.0 * p, 5.0, Some(am), None); // stretching rubber
    let o1 = v.osc(Wave::Pulse(0.3), f0 * p, t_abs, None, Some(lp));
    let o2g = v.gain(0.4, Some(lp));
    let o2 = v.osc(Wave::Triangle, f0 * 2.01 * p, t_abs, None, Some(o2g));
    let (vib_depth, _vib_osc) = v.lfo(5.7, 35.0, ParamRef::Detune(o1), t_abs, None, Wave::Sine);
    v.g.connect_param(vib_depth, ParamRef::Detune(o2));
    let (vib2_depth, _vib2_osc) = v.lfo(0.83, 60.0, ParamRef::Detune(o1), t_abs, None, Wave::Sine);
    v.g.connect_param(vib2_depth, ParamRef::Detune(o2));
    v.lfo(3.3 * p, 0.25, ParamRef::Gain(am), t_abs, None, Wave::Sine);
    v.lfo(2.1, 250.0 * p, ParamRef::Freq(lp), t_abs, None, Wave::Sine);
    let ag = v.gain(0.12, Some(v.out));
    let abp = v.filter(BqKind::Bandpass, 1100.0 * p, 1.2, Some(ag), None); // air blown in
    v.noise(NoiseKind::Pink, t_abs, None, Some(abp), 1.0);
    let bg = v.gain(0.35, Some(v.out)); // wet soap film
    let blp = v.filter(BqKind::Lowpass, 1600.0, 0.7, Some(bg), None);
    v.buffer(texture("bubbles"), t_abs, None, Some(blp), 0.8 * p, None);
}

/// `bubble_release` build ≡ audio.js L1412-1421.
fn b_bubble_release(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 240.0 * p,
        f1: Some(520.0 * p),
        sw: Some(0.07),
        a: Some(0.003),
        d: Some(0.1),
        peak: Some(0.7),
        ..Default::default()
    }); // plop
    v.tone(Tone {
        f: 130.0 * p,
        f1: Some(80.0 * p),
        sw: Some(0.08),
        a: Some(0.003),
        d: Some(0.1),
        peak: Some(0.4),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(900.0 * p),
        f1: Some(1600.0 * p),
        sw: Some(0.1),
        q: Some(3.0),
        a: Some(0.005),
        d: Some(0.08),
        peak: Some(0.2),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.08,
        f: 700.0 * p,
        f1: Some(1300.0 * p),
        sw: Some(0.25),
        a: Some(0.05),
        d: Some(0.2),
        peak: Some(0.08),
        ..Default::default()
    }); // airy lift-off
    plips(v, 0.04, 2, 0.08, [900.0, 1400.0], 0.1, p);
}

/// `bubble_pop` build ≡ audio.js L1422-1430.
fn b_bubble_pop(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2500.0),
        a: Some(0.0003),
        d: Some(0.012),
        peak: Some(0.7),
        ..Default::default()
    });
    v.tone(Tone {
        f: 950.0 * p,
        f1: Some(420.0 * p),
        sw: Some(0.03),
        a: Some(0.0008),
        d: Some(0.04),
        peak: Some(0.6),
        ..Default::default()
    });
    plips(v, 0.01, 3, 0.07, [1100.0, 2200.0], 0.15, p);
    v.nz(Nz {
        t: 0.005,
        f: Some(2200.0 * p),
        f1: Some(900.0 * p),
        sw: Some(0.08),
        q: Some(2.0),
        a: Some(0.002),
        d: Some(0.08),
        peak: Some(0.25),
        ..Default::default()
    }); // fizz
}

/// `bubble_blast` build ≡ audio.js L1431-1439.
fn b_bubble_blast(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.0003),
        d: Some(0.02),
        peak: Some(0.8),
        ..Default::default()
    }); // glassy pop
    v.tone(Tone {
        f: 1500.0 * p,
        f1: Some(500.0 * p),
        sw: Some(0.04),
        a: Some(0.0008),
        d: Some(0.05),
        peak: Some(0.6),
        ..Default::default()
    });
    for (f, a) in [(2637.0, 0.18), (3951.0, 0.12), (5274.0, 0.07)] {
        v.tone(Tone {
            f: f * p,
            a: Some(0.0005),
            d: Some(0.18),
            peak: Some(a),
            ..Default::default()
        });
    }
    ink_boom(
        v,
        0.025,
        p,
        0.95,
        BoomOpts {
            f: 95.0,
            f1: 28.0,
            crack: 0.4,
            bright: 5000.0,
            ..Default::default()
        },
    ); // the blast
}

/// `jet_loop` loop ≡ audio.js L1440-1464 (the `pitch(q)` param callback is a
/// runtime control; the baked burst uses q = 1).
fn l_jet_loop(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let fl = v.gain(0.8, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 1400.0 * p, 0.9, Some(fl), None); // roaring thrust
    v.noise(NoiseKind::Pink, t_abs, None, Some(lp), 1.0);
    v.lfo(24.0 * p, 0.2, ParamRef::Gain(fl), t_abs, None, Wave::Sine);
    v.lfo(0.37, 200.0, ParamRef::Freq(lp), t_abs, None, Wave::Sine);
    v.lfo(7.3, 0.06, ParamRef::Gain(fl), t_abs, None, Wave::Sine);
    let bg = v.gain(0.7, Some(v.out)); // body
    let blp = v.filter(BqKind::Lowpass, 260.0, 1.0, Some(bg), None);
    v.noise(NoiseKind::Brown, t_abs, None, Some(blp), 1.0);
    let hg = v.gain(0.14, Some(v.out)); // hiss
    let hbp = v.filter(BqKind::Bandpass, 4200.0 * p, 0.8, Some(hg), None);
    v.noise(NoiseKind::White, t_abs, None, Some(hbp), 1.0);
    let gg = v.gain(0.2, Some(v.out)); // burner grit
    let glp = v.filter(BqKind::Lowpass, 2500.0, 0.7, Some(gg), None);
    let gsh = v.shaper(3.0, Some(glp));
    let g3 = v.gain(3.0, Some(gsh));
    let gbp = v.filter(BqKind::Bandpass, 500.0 * p, 1.2, Some(g3), None);
    v.noise(NoiseKind::Pink, t_abs, None, Some(gbp), 1.0);
}

/// `jet_fire` build ≡ audio.js L1465-1477.
fn b_jet_fire(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 140.0 * p,
        f1: Some(36.0 * p),
        sw: Some(0.2),
        a: Some(0.002),
        d: Some(0.38),
        peak: Some(1.0),
        ..Default::default()
    }); // THOOMP
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 280.0 * p,
        f1: Some(80.0 * p),
        sw: Some(0.1),
        a: Some(0.002),
        d: Some(0.12),
        peak: Some(0.45),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2200.0),
        a: Some(0.0004),
        d: Some(0.03),
        peak: Some(0.6),
        ..Default::default()
    });
    let g8 = v.gain(0.8, Some(v.out));
    let sh = v.shaper(2.0, Some(g8));
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(2200.0),
        f1: Some(260.0),
        sw: Some(0.18),
        q: Some(1.5),
        a: Some(0.002),
        d: Some(0.2),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(600.0 * p),
        f1: Some(2600.0 * p),
        sw: Some(0.25),
        q: Some(1.3),
        a: Some(0.03),
        d: Some(0.2),
        peak: Some(0.35),
        ..Default::default()
    }); // projectile whoosh
    v.nz(Nz {
        t: 0.01,
        f: Some(1300.0 * p),
        f1: Some(400.0 * p),
        sw: Some(0.18),
        q: Some(4.0),
        a: Some(0.003),
        d: Some(0.2),
        peak: Some(0.6),
        ..Default::default()
    });
    bloops(v, 0.02, 3, 0.08, [380.0, 650.0], 0.25, p);
}

/// `jet_end` build ≡ audio.js L1478-1492.
fn b_jet_end(v: &mut Vb, p: f64) {
    let mut t = 0.0;
    let mut a = 0.8;
    for i in 0..7 {
        // sputters, slowing down
        v.nz(Nz {
            t,
            kind: Some(NoiseKind::Pink),
            ft: Some(Some(BqKind::Lowpass)),
            f: Some(1600.0 * (1.0 - i as f64 * 0.09)),
            q: Some(1.2),
            a: Some(0.008),
            d: Some(0.04 + i as f64 * 0.008),
            peak: Some(a),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: 110.0 * p * (1.0 - i as f64 * 0.05),
            f1: Some(70.0 * p),
            sw: Some(0.04),
            a: Some(0.005),
            d: Some(0.05),
            peak: Some(a * 0.5),
            ..Default::default()
        });
        t += 0.055 + i as f64 * 0.022 + v.r(0.0, 0.02);
        a *= 0.8;
    }
    let g = v.gain(0.0, Some(v.out)); // dying whine
    let end = synth::pts(
        v.g.param(g),
        v.t,
        &[(0.0, 0.0), (0.02, 0.12), (0.6, 0.04), (0.8, 0.0)],
    );
    let lp = v.filter(BqKind::Lowpass, 1500.0, 1.0, Some(g), None);
    let o = v.osc(Wave::Triangle, 520.0 * p, v.t, Some(end + 0.01), Some(lp));
    synth::sweep(v.g.param(o), v.t, 520.0 * p, 110.0 * p, 0.8, false);
    v.nz(Nz {
        t: t + 0.02,
        f: Some(900.0),
        f1: Some(400.0),
        sw: Some(0.2),
        q: Some(1.0),
        a: Some(0.02),
        d: Some(0.18),
        peak: Some(0.3),
        ..Default::default()
    }); // last puff
}

/// `stamp_swing` build ≡ audio.js L1493-1503.
fn b_stamp_swing(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    whoosh(
        v,
        0.0,
        0.46,
        220.0 * p,
        900.0 * p,
        300.0 * p,
        0.9,
        Some(1.4),
        Some(0.48),
        Some(&[[0.0, 0.0], [0.45, 1.0], [0.6, 0.75], [1.0, 0.0]]),
        None,
        None,
    );
    whoosh(
        v,
        0.03,
        0.4,
        1200.0 * p,
        2800.0 * p,
        1400.0 * p,
        0.2,
        Some(2.0),
        None,
        None,
        Some(NoiseKind::White),
        None,
    );
    let g = v.gain(0.0, Some(v.out)); // heavy "vwoom"
    let end = synth::pts(v.g.param(g), t_abs, &[(0.0, 0.0), (0.2, 0.4), (0.46, 0.0)]);
    let o = v.osc(Wave::Sine, 60.0 * p, t_abs, Some(end + 0.01), Some(g));
    v.g.param(o).set(60.0 * p, t_abs);
    v.g.param(o).exp(95.0 * p, t_abs + 0.2);
    v.g.param(o).exp(60.0 * p, t_abs + 0.46);
}

/// `stamp_slam` build ≡ audio.js L1504-1521.
fn b_stamp_slam(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let g = v.gain(0.0, Some(v.out)); // rubbery THWUMP
    let end = synth::perc(v.g.param(g), t_abs, 0.002, 1.0, 0.5);
    let o = v.osc(Wave::Sine, 170.0 * p, t_abs, Some(end + 0.01), Some(g));
    v.g.param(o).set(170.0 * p, t_abs);
    v.g.param(o).exp(48.0 * p, t_abs + 0.09);
    v.g.param(o).exp(62.0 * p, t_abs + 0.2);
    v.g.param(o).exp(44.0 * p, t_abs + 0.5);
    let (w_depth, _w_osc) = v.lfo(11.0, 0.0, ParamRef::Detune(o), t_abs, Some(end), Wave::Sine);
    synth::pts(
        v.g.param(w_depth),
        t_abs,
        &[(0.0, 0.0), (0.05, 120.0), (0.4, 0.0)],
    );
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 240.0 * p,
        f1: Some(90.0 * p),
        sw: Some(0.08),
        a: Some(0.001),
        d: Some(0.14),
        peak: Some(0.55),
        ..Default::default()
    }); // rubber smack
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1400.0),
        f1: Some(200.0),
        sw: Some(0.18),
        q: Some(2.5),
        a: Some(0.001),
        d: Some(0.2),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2000.0),
        a: Some(0.0004),
        d: Some(0.025),
        peak: Some(0.6),
        ..Default::default()
    });
    big_splat(v, 0.015, p, 0.8); // ink splash
    bloops(v, 0.05, 5, 0.3, [300.0, 650.0], 0.3, p);
    plips(v, 0.12, 4, 0.35, [800.0, 1600.0], 0.1, p);
}

/// `stamp_throw` build ≡ audio.js L1522-1534.
fn b_stamp_throw(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let d_len = 0.65;
    let g = v.gain(0.0, Some(v.out)); // spinning whoosh
    let am = v.gain(0.5, Some(g));
    let bp = v.filter(BqKind::Bandpass, 420.0 * p, 1.6, Some(am), None);
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.12, 0.8), (0.4, 0.6), (d_len, 0.0)],
    );
    v.lfo(
        9.0 * p,
        0.5,
        ParamRef::Gain(am),
        t_abs,
        Some(t_abs + d_len + 0.02),
        Wave::Sine,
    );
    v.g.param(bp).set(420.0 * p, t_abs);
    v.g.param(bp).exp(1300.0 * p, t_abs + 0.25);
    v.g.param(bp).exp(600.0 * p, t_abs + d_len);
    v.noise(
        NoiseKind::Pink,
        t_abs,
        Some(t_abs + d_len + 0.02),
        Some(bp),
        1.0,
    );
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 330.0 * p,
        f1: Some(210.0 * p),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(0.3),
        ..Default::default()
    }); // release
    v.tone(Tone {
        f: 110.0 * p,
        f1: Some(70.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.08),
        peak: Some(0.4),
        ..Default::default()
    });
}

/// `booyah_charge` loop ≡ audio.js L1535-1563 (the `pitch(q)` param callback
/// is a runtime control; the baked burst uses q = 1).
fn l_booyah_charge(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let lp = v.filter(BqKind::Lowpass, 2400.0 * p, 0.7, Some(v.out), None);
    let lvl = v.gain(0.75 + 0.25 * p, Some(lp)); // swells with charge
    let am = v.gain(0.8, Some(lvl));
    v.lfo(4.0 * p, 0.2, ParamRef::Gain(am), t_abs, None, Wave::Sine); // energy pulse
    let mix = v.gain(0.35, None); // "ah" choir: saws through formants
    for (f, q, a) in [(750.0, 5.0, 1.0), (1150.0, 6.0, 0.55), (2600.0, 8.0, 0.25)] {
        let ag = v.gain(a, Some(am));
        let fbp = v.filter(BqKind::Bandpass, f, q, Some(ag), None);
        v.g.connect(mix, fbp);
    }
    let vib = v.gain(12.0, None);
    v.osc(Wave::Sine, 5.2, t_abs, None, Some(vib));
    for m in [48.0, 55.0, 60.0, 64.0, 67.0] {
        for dt in [-9.0, 9.0] {
            let o = v.osc(Wave::Saw, synth::mtof(m) * p, t_abs, None, Some(mix));
            v.g.param2(o).set_default(dt);
            v.g.connect_param(vib, ParamRef::Detune(o));
        }
    }
    let hg = v.gain(0.3, Some(am)); // hum
    for m in [36.0, 48.0] {
        v.osc(Wave::Sine, synth::mtof(m) * p, t_abs, None, Some(hg));
    }
    let sg = v.gain(0.025 * p, Some(v.out)); // shimmer
    let shp = v.filter(BqKind::Highpass, 6000.0, 0.7, Some(sg), None);
    v.noise(NoiseKind::White, t_abs, None, Some(shp), 1.0);
    v.lfo(9.0 * p, 0.012, ParamRef::Gain(sg), t_abs, None, Wave::Sine);
}

/// `booyah_cheer` build ≡ audio.js L1564-1576.
fn b_booyah_cheer(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let d_len = 0.34;
    let f0 = 330.0 * p;
    let g = v.gain(0.0, Some(v.out));
    let end = synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.025, 1.0), (0.2, 0.85), (d_len, 0.0)],
    );
    let (srcs, fs) = vox(
        v,
        t_abs,
        end + 0.01,
        f0,
        &[[350.0, 6.0, 1.0], [2100.0, 8.0, 0.7], [2900.0, 10.0, 0.25]],
        g,
        8.0,
    );
    for o in &srcs {
        v.g.param(*o).set(f0 * 0.85, t_abs);
        v.g.param(*o).exp(f0 * 1.25, t_abs + 0.1);
        v.g.param(*o).exp(f0 * 1.12, t_abs + d_len);
    }
    // "ye" → "ah"
    for (i, (a, b)) in [(350.0, 800.0), (2100.0, 1250.0), (2900.0, 2700.0)]
        .iter()
        .enumerate()
    {
        v.g.param(fs[i]).set(*a, t_abs + 0.03);
        v.g.param(fs[i]).exp(*b, t_abs + 0.14);
    }
    bell(
        v,
        t_abs + 0.02,
        synth::mtof(88.0) * p,
        0.08,
        0.25,
        3.5,
        1.6,
        None,
    );
}

/// `booyah_throw` build ≡ audio.js L1577-1589.
fn b_booyah_throw(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    v.tone(Tone {
        f: 150.0 * p,
        f1: Some(60.0 * p),
        sw: Some(0.1),
        a: Some(0.002),
        d: Some(0.15),
        peak: Some(0.7),
        ..Default::default()
    });
    whoosh(
        v,
        0.0,
        0.45,
        300.0 * p,
        1500.0 * p,
        2400.0 * p,
        0.7,
        Some(1.5),
        None,
        Some(&[[0.0, 0.0], [0.22, 1.0], [0.5, 0.6], [1.0, 0.0]]),
        None,
        None,
    );
    let g = v.gain(0.0, Some(v.out)); // energy streak
    let end = synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.04, 0.2), (0.3, 0.15), (0.45, 0.0)],
    );
    let o = v.osc(Wave::Triangle, 220.0 * p, t_abs, Some(end + 0.01), Some(g));
    synth::sweep(v.g.param(o), t_abs, 220.0 * p, 880.0 * p, 0.4, false);
    v.lfo(
        11.0,
        30.0,
        ParamRef::Detune(o),
        t_abs,
        Some(end),
        Wave::Sine,
    );
    for (i, m) in [79.0, 84.0, 88.0].iter().enumerate() {
        bell(
            v,
            t_abs + 0.04 + i as f64 * 0.05,
            synth::mtof(*m) * p,
            0.08,
            0.35,
            3.5,
            1.6,
            None,
        );
    }
}

/// `booyah_blast` build ≡ audio.js L1590-1600.
fn b_booyah_blast(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    ink_boom(
        v,
        0.0,
        p,
        1.0,
        BoomOpts {
            len: 1.6,
            f: 65.0,
            f1: 20.0,
            bright: 7000.0,
            bloops: 12,
            plips: 8,
            ..Default::default()
        },
    );
    let g = v.gain(0.0, Some(v.out)); // long rolling rumble
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.15, 0.9), (0.6, 0.6), (1.2, 0.35), (2.1, 0.0)],
    );
    let lp = v.filter(BqKind::Lowpass, 260.0, 1.0, Some(g), None);
    v.noise(NoiseKind::Brown, t_abs, Some(t_abs + 2.15), Some(lp), 1.0);
    for m in [84.0, 88.0, 91.0] {
        // victorious glint
        bell(
            v,
            t_abs + 0.05,
            synth::mtof(m) * p,
            0.06,
            1.0,
            3.5,
            1.6,
            None,
        );
    }
}

/// `zip_fire` build ≡ audio.js L1601-1616.
fn b_zip_fire(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let lp = v.filter(BqKind::Lowpass, 5000.0, 0.8, Some(v.out), None); // zip
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 300.0 * p,
        f1: Some(1800.0 * p),
        sw: Some(0.12),
        a: Some(0.002),
        d: Some(0.14),
        peak: Some(0.22),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(1200.0 * p),
        f1: Some(5000.0 * p),
        sw: Some(0.14),
        q: Some(3.0),
        a: Some(0.003),
        d: Some(0.14),
        peak: Some(0.5),
        ..Default::default()
    });
    let g = v.gain(0.0, Some(v.out)); // twang of the taut line
    let end = synth::perc(v.g.param(g), t_abs + 0.01, 0.002, 0.6, 0.3);
    let flp = v.filter(BqKind::Lowpass, 4000.0, 3.0, Some(g), None);
    synth::sweep(v.g.param(flp), t_abs + 0.01, 4000.0, 500.0, 0.2, false);
    let o = v.osc(
        Wave::Saw,
        210.0 * p,
        t_abs + 0.01,
        Some(end + 0.01),
        Some(flp),
    );
    synth::sweep(
        v.g.param(o),
        t_abs + 0.01,
        210.0 * p,
        160.0 * p,
        0.25,
        false,
    );
    v.lfo(
        14.0,
        18.0,
        ParamRef::Detune(o),
        t_abs + 0.01,
        Some(end),
        Wave::Sine,
    );
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(4000.0),
        a: Some(0.0004),
        d: Some(0.01),
        peak: Some(0.4),
        ..Default::default()
    });
}

/// `zip_latch` build ≡ audio.js L1617-1626.
fn b_zip_latch(v: &mut Vb, p: f64) {
    clank(v, 0.0, 620.0, 0.5, 0.25, p, 4, None);
    v.tone(Tone {
        f: 180.0 * p,
        f1: Some(80.0 * p),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.07),
        peak: Some(0.6),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.008,
        f: Some(2200.0 * p),
        f1: Some(800.0 * p),
        sw: Some(0.1),
        q: Some(2.5),
        a: Some(0.001),
        d: Some(0.12),
        peak: Some(0.7),
        ..Default::default()
    }); // splat
    bloops(v, 0.015, 2, 0.05, [500.0, 750.0], 0.3, p);
    plips(v, 0.04, 2, 0.06, [900.0, 1500.0], 0.12, p);
}

/// `zip_pull` build ≡ audio.js L1627-1643.
fn b_zip_pull(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let d_len = 0.5;
    let g = v.gain(0.0, Some(v.out)); // rushing air, rising
    let bp = v.filter(BqKind::Bandpass, 300.0 * p, 1.5, Some(g), None);
    synth::sweep(v.g.param(bp), t_abs, 300.0 * p, 3500.0 * p, d_len, false);
    synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.3, 0.7), (0.44, 0.8), (d_len, 0.0)],
    );
    v.noise(
        NoiseKind::Pink,
        t_abs,
        Some(t_abs + d_len + 0.02),
        Some(bp),
        1.0,
    );
    let wg = v.gain(0.0, Some(v.out)); // line whirr
    let am = v.gain(0.6, Some(wg));
    let lp = v.filter(BqKind::Lowpass, 2500.0, 1.0, Some(am), None);
    synth::pts(
        v.g.param(wg),
        t_abs,
        &[(0.0, 0.0), (0.05, 0.12), (0.4, 0.16), (d_len, 0.0)],
    );
    let o = v.osc(
        Wave::Saw,
        160.0 * p,
        t_abs,
        Some(t_abs + d_len + 0.02),
        Some(lp),
    );
    synth::sweep(v.g.param(o), t_abs, 160.0 * p, 700.0 * p, d_len, false);
    let (_r_depth, r_osc) = v.lfo(
        30.0,
        0.4,
        ParamRef::Gain(am),
        t_abs,
        Some(t_abs + d_len + 0.02),
        Wave::Sine,
    );
    synth::sweep(v.g.param(r_osc), t_abs, 30.0, 80.0, d_len, false);
    v.nz(Nz {
        t: 0.05,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5000.0),
        a: Some(0.3),
        d: Some(0.15),
        peak: Some(0.12),
        ..Default::default()
    });
}

/// `crab_move` loop ≡ audio.js L1644-1671 (the `pitch(q)` param callback is a
/// runtime control; the baked burst uses q = 1).
fn l_crab_move(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let step = 4.4;
    let sg = v.gain(0.25, Some(v.out)); // servo whirr
    let sbp = v.filter(BqKind::Bandpass, 900.0 * p, 2.5, Some(sg), None);
    let s1 = v.osc(Wave::Saw, 115.0 * p, t_abs, None, Some(sbp));
    let s2g = v.gain(0.5, Some(sg));
    let s2 = v.osc(Wave::Triangle, 231.0 * p, t_abs, None, Some(s2g));
    let (sv_depth, _sv_osc) = v.lfo(
        step * p,
        60.0,
        ParamRef::Detune(s1),
        t_abs,
        None,
        Wave::Sine,
    );
    v.g.connect_param(sv_depth, ParamRef::Detune(s2));
    for (ph, fs) in [
        (0.0, [640.0, 1730.0, 2980.0]),
        (0.5, [540.0, 1490.0, 2610.0]),
    ] {
        // clanky legs, alternating
        let lg = v.gain(0.12, Some(v.out));
        let st = v.osc(
            Wave::Stroke(9.0),
            step * p,
            t_abs + ph / (step * p),
            None,
            None,
        );
        let dp = v.gain(0.9, None);
        v.g.connect(st, dp);
        v.g.connect_param(dp, ParamRef::Gain(lg));
        let mg = v.gain(0.06, Some(lg));
        for f in fs {
            v.osc(Wave::Sine, f, t_abs, None, Some(mg));
        }
        let ng = v.gain(0.35, Some(lg));
        let nbp = v.filter(BqKind::Bandpass, 2500.0, 2.0, Some(ng), None);
        v.noise(NoiseKind::White, t_abs, None, Some(nbp), 1.0);
        let og = v.gain(0.45, Some(lg));
        v.osc(Wave::Sine, 85.0, t_abs, None, Some(og));
    }
}

/// `crab_gatling` build ≡ audio.js L1672-1679.
fn b_crab_gatling(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 190.0 * p,
        f1: Some(85.0 * p),
        sw: Some(0.03),
        a: Some(0.0008),
        d: Some(0.035),
        peak: Some(0.7),
        ..Default::default()
    });
    let nz_f = v.r(2000.0, 2800.0) * p;
    v.nz(Nz {
        f: Some(nz_f),
        f1: Some(900.0 * p),
        sw: Some(0.03),
        q: Some(1.5),
        a: Some(0.0006),
        d: Some(0.03),
        peak: Some(0.6),
        ..Default::default()
    });
    let tone_f = v.r(650.0, 800.0) * p;
    v.tone(Tone {
        f: tone_f,
        f1: Some(300.0 * p),
        sw: Some(0.025),
        a: Some(0.001),
        d: Some(0.03),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `crab_cannon` build ≡ audio.js L1680-1691.
fn b_crab_cannon(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 105.0 * p,
        f1: Some(42.0 * p),
        sw: Some(0.14),
        a: Some(0.002),
        d: Some(0.32),
        peak: Some(1.0),
        ..Default::default()
    }); // thoo-
    v.tone(Tone {
        f: 260.0 * p,
        f1: Some(150.0 * p),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(0.45),
        ..Default::default()
    }); // -onk
    v.nz(Nz {
        f: Some(320.0 * p),
        q: Some(7.0),
        a: Some(0.001),
        d: Some(0.14),
        peak: Some(0.9),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(1500.0),
        f1: Some(250.0),
        sw: Some(0.15),
        q: Some(1.2),
        a: Some(0.002),
        d: Some(0.18),
        peak: Some(0.7),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.03,
        f: Some(500.0 * p),
        f1: Some(1800.0 * p),
        sw: Some(0.3),
        q: Some(1.4),
        a: Some(0.05),
        d: Some(0.25),
        peak: Some(0.2),
        ..Default::default()
    }); // shell leaving
    clank(v, 0.0, 900.0, 0.12, 0.15, p, 4, None);
    bloops(v, 0.02, 2, 0.06, [380.0, 600.0], 0.2, p);
}

/// `crab_roll` loop ≡ audio.js L1692-1717 (the `pitch(q)` param callback is a
/// runtime control; the baked burst uses q = 1).
fn l_crab_roll(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let rev = 3.0;
    let rg = v.gain(0.7, Some(v.out)); // rumble
    let rlp = v.filter(BqKind::Lowpass, 200.0 * p, 1.0, Some(rg), None);
    v.noise(NoiseKind::Brown, t_abs, None, Some(rlp), 1.0);
    let outg = v.gain(0.35, Some(v.out)); // hum throbbing once per turn
    let am = v.gain(0.6, Some(outg));
    let alp = v.filter(BqKind::Lowpass, 400.0, 0.8, Some(am), None);
    v.osc(Wave::Triangle, 62.0 * p, t_abs, None, Some(alp));
    v.lfo(rev * p, 0.4, ParamRef::Gain(am), t_abs, None, Wave::Sine);
    let cg = v.gain(0.3, Some(v.out)); // gravel crunch
    let cbp = v.filter(BqKind::Bandpass, 900.0 * p, 1.2, Some(cg), None);
    v.buffer(texture("sizzle"), t_abs, None, Some(cbp), 0.6 * p, None);
    let pg = v.gain(0.1, Some(v.out)); // armour plates clacking
    let st = v.osc(Wave::Stroke(9.0), 2.0 * rev * p, t_abs, None, None);
    let dp = v.gain(0.9, None);
    v.g.connect(st, dp);
    v.g.connect_param(dp, ParamRef::Gain(pg));
    let mg = v.gain(0.05, Some(pg));
    for f in [410.0, 1130.0, 2210.0] {
        v.osc(Wave::Sine, f, t_abs, None, Some(mg));
    }
    let ng = v.gain(0.3, Some(pg));
    let nbp = v.filter(BqKind::Bandpass, 1800.0, 2.0, Some(ng), None);
    v.noise(NoiseKind::Pink, t_abs, None, Some(nbp), 1.0);
}

/// `crab_hit` build ≡ audio.js L1718-1724.
fn b_crab_hit(v: &mut Vb, p: f64) {
    let cf = v.r(1500.0, 1900.0);
    clank(v, 0.0, cf, 0.45, 0.12, p, 3, None);
    v.nz(Nz {
        f: Some(2600.0 * p),
        f1: Some(1200.0 * p),
        sw: Some(0.04),
        q: Some(2.0),
        a: Some(0.001),
        d: Some(0.04),
        peak: Some(0.3),
        ..Default::default()
    }); // ink spatter
}

/// `crab_break` build ≡ audio.js L1725-1743.
fn b_crab_break(v: &mut Vb, p: f64) {
    let t_abs = v.t;
    let g = v.gain(0.0, Some(v.out)); // servo power-down
    let end = synth::pts(
        v.g.param(g),
        t_abs,
        &[(0.0, 0.0), (0.02, 0.3), (0.5, 0.2), (0.75, 0.0)],
    );
    let lp = v.filter(BqKind::Lowpass, 2500.0, 2.0, Some(g), None);
    synth::sweep(v.g.param(lp), t_abs, 2500.0, 300.0, 0.75, false);
    let o = v.osc(Wave::Saw, 320.0 * p, t_abs, Some(end + 0.01), Some(lp));
    synth::sweep(v.g.param(o), t_abs, 320.0 * p, 55.0 * p, 0.75, false);
    for (i, (t, a)) in [(0.0, 1.0), (0.16, 0.8), (0.34, 0.6)].iter().enumerate() {
        // clunks
        clank(v, *t, 520.0 - i as f64 * 90.0, 0.35 * a, 0.2, p, 4, None);
        v.tone(Tone {
            t: *t,
            f: 150.0 * p,
            f1: Some(60.0 * p),
            sw: Some(0.06),
            a: Some(0.001),
            d: Some(0.12),
            peak: Some(0.7 * a),
            ..Default::default()
        });
        v.nz(Nz {
            t: *t,
            kind: Some(NoiseKind::Pink),
            ft: Some(Some(BqKind::Lowpass)),
            f: Some(900.0),
            a: Some(0.001),
            d: Some(0.06),
            peak: Some(0.5 * a),
            ..Default::default()
        });
    }
    big_splat(v, 0.42, p, 0.9); // ink burst
    bloops(v, 0.45, 5, 0.3, [300.0, 650.0], 0.3, p);
    v.nz(Nz {
        t: 0.42,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3000.0),
        a: Some(0.02),
        d: Some(0.5),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `floodBurst` ≡ audio.js L1818-1825.
pub fn flood_burst(v: &mut Vb, t: f64, p: f64, s: f64) {
    let g = v.gain(0.0, Some(v.out));
    let lp = v.filter(BqKind::Lowpass, 500.0, 1.2, Some(g), None);
    synth::sweep(v.g.param(lp), t, 420.0 * p, 3600.0 * p, 0.2, false);
    v.g.param(lp).exp(900.0 * p, t + 0.55);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.03, 0.75 * s), (0.22, 0.5 * s), (0.6, 0.0)],
    );
    v.noise(NoiseKind::Pink, t, Some(t + 0.62), Some(lp), 1.0);
    bloops(v, t - v.t + 0.04, 4, 0.2, [320.0, 620.0], 0.3 * s, p);
    v.tone(Tone {
        at: Some(t),
        f: 95.0 * p,
        f1: Some(44.0 * p),
        sw: Some(0.2),
        a: Some(0.003),
        d: Some(0.3),
        peak: Some(0.75 * s),
        ..Default::default()
    });
}

/// `crack` ≡ audio.js L1827-1834 (glass + ink crack; `dir` = -1 down, +1 up).
pub fn crack(v: &mut Vb, t: f64, p: f64, s: f64, dir: f64) {
    v.nz(Nz {
        at: Some(t),
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2200.0),
        a: Some(0.0006),
        d: Some(0.05),
        peak: Some(0.8 * s),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        f: Some(4200.0 * p),
        f1: Some(if dir < 0.0 { 1400.0 * p } else { 7000.0 * p }),
        sw: Some(0.18),
        q: Some(6.0),
        a: Some(0.001),
        d: Some(0.2),
        peak: Some(0.45 * s),
        ..Default::default()
    });
    for (r, a, d) in [
        (1.0, 0.3, 0.3),
        (2.43, 0.18, 0.2),
        (3.87, 0.11, 0.14),
        (5.61, 0.07, 0.09),
    ] {
        let f = 1150.0 * p * r;
        v.tone(Tone {
            at: Some(t + 0.004),
            f,
            f1: Some(f * if dir < 0.0 { 0.82 } else { 1.12 }),
            sw: Some(d),
            a: Some(0.0008),
            d: Some(d),
            peak: Some(a * s),
            ..Default::default()
        });
    }
}

/// `snareRoll` ≡ audio.js L2044-2053 (stroked-noise snare roll body).
pub fn snare_roll(v: &mut Vb, t: f64, t1: Option<f64>, p: f64, to: NodeId) -> NodeId {
    let am = v.gain(0.0, Some(to));
    let st = v.osc(Wave::Stroke(7.0), 22.0 * p, t, t1, None);
    let depth = v.gain(0.85, None);
    v.g.connect(st, depth);
    v.g.connect_param(depth, ParamRef::Gain(am));
    v.g.param(am).set_default(0.16);
    v.lfo(11.0 * p, 0.12, ParamRef::Gain(am), t, t1, Wave::Sine);
    let hp = v.filter(BqKind::Highpass, 1200.0, 0.7, Some(am), None);
    v.noise(NoiseKind::White, t, t1, Some(hp), 1.0);
    let bodyg = v.gain(0.35, Some(am));
    v.osc(Wave::Triangle, 190.0, t, t1, Some(bodyg));
    st
}

/// `ready` build ≡ audio.js L1746-1764.
fn b_ready(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 300.0, 1.8, Some(g), None);
    synth::sweep(v.g.param(bp), t, 300.0, 4000.0, 1.1, false);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.9, 0.5), (1.1, 0.6), (1.22, 0.0)],
    );
    v.noise(NoiseKind::White, t, Some(t + 1.25), Some(bp), 1.0);
    let cg = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(cg),
        t,
        &[(0.0, 0.0), (1.02, 0.1), (1.1, 0.1), (1.25, 0.0)],
    );
    let lp = v.filter(BqKind::Lowpass, 400.0, 1.5, Some(cg), None);
    synth::sweep(v.g.param(lp), t, 400.0, 5000.0, 1.1, false);
    for (i, m) in [48.0, 55.0, 60.0, 64.0].iter().enumerate() {
        let o = v.osc(Wave::Saw, synth::mtof(*m) * p, t, Some(t + 1.27), Some(lp));
        v.g.param2(o).set(-1200.0 + (i as f64 - 1.5) * 6.0, t);
        v.g.param2(o).lin((i as f64 - 1.5) * 6.0, t + 1.1);
    }
    v.tone(Tone {
        t: 1.08,
        f: 600.0,
        f1: Some(1400.0),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.12),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `go_horn` build ≡ audio.js L1765-1773.
fn b_go_horn(v: &mut Vb, p: f64) {
    let t = v.t;
    for (m, pan) in [
        (60.0, -0.3),
        (64.0, 0.25),
        (67.0, -0.15),
        (72.0, 0.3),
        (76.0, 0.0),
    ] {
        brass(
            v,
            t,
            synth::mtof(m) * p,
            0.42,
            0.34,
            0.012,
            0.35,
            4200.0,
            Some(pan),
        );
    }
    crash(v, t, 0.45, 1.6, None);
    kick(v, t, 0.8, 165.0, 46.0, 0.34, 0.085, None);
}

/// `countdown_tick` build ≡ audio.js L1774-1780.
fn b_countdown_tick(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 880.0 * p,
        a: Some(0.001),
        h: Some(0.05),
        d: Some(0.06),
        peak: Some(0.55),
        ..Default::default()
    });
    let lp = v.filter(BqKind::Lowpass, 3000.0, 0.7, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 880.0 * p,
        a: Some(0.001),
        h: Some(0.04),
        d: Some(0.04),
        peak: Some(0.1),
        to: Some(lp),
        ..Default::default()
    });
}

/// `final_count` build ≡ audio.js L1781-1790.
fn b_final_count(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 1320.0 * p,
        a: Some(0.001),
        h: Some(0.08),
        d: Some(0.08),
        peak: Some(0.55),
        ..Default::default()
    });
    let lp = v.filter(BqKind::Lowpass, 4500.0, 0.7, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 660.0 * p,
        a: Some(0.001),
        h: Some(0.07),
        d: Some(0.06),
        peak: Some(0.16),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 1326.0 * p,
        a: Some(0.001),
        h: Some(0.05),
        d: Some(0.05),
        peak: Some(0.06),
        to: Some(lp),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5000.0),
        a: Some(0.0004),
        d: Some(0.01),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `one_minute` build ≡ audio.js L1791-1800.
fn b_one_minute(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 4000.0, 0.8, Some(v.out), None);
    let pw = Wave::Pulse(0.25);
    for (m, t, h) in [
        (67.0, 0.0, 0.08),
        (72.0, 0.1, 0.08),
        (76.0, 0.2, 0.08),
        (79.0, 0.3, 0.35),
    ] {
        v.tone(Tone {
            t,
            wave: Some(pw),
            f: synth::mtof(m) * p,
            a: Some(0.003),
            h: Some(h),
            d: Some(0.12),
            peak: Some(0.25),
            to: Some(lp),
            ..Default::default()
        });
        bell(
            v,
            v.t + t,
            synth::mtof(m + 12.0) * p,
            0.12,
            0.4,
            3.5,
            1.6,
            None,
        );
    }
}

/// `times_up` build ≡ audio.js L1801-1814.
fn b_times_up(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    synth::ahr(v.g.param(g), t, 0.02, 0.35, 0.55, 0.08);
    let o = v.osc(Wave::Sine, 2600.0, t, Some(t + 0.7), Some(g));
    v.lfo(28.0, 160.0, ParamRef::Freq(o), t, Some(t + 0.7), Wave::Sine);
    v.nz(Nz {
        f: Some(2600.0),
        q: Some(3.0),
        a: Some(0.02),
        h: Some(0.55),
        d: Some(0.08),
        peak: Some(0.12),
        ..Default::default()
    });
    let t2 = t + 0.72;
    for (m, pan) in [(58.0, -0.3), (62.0, 0.3), (65.0, -0.1), (70.0, 0.2)] {
        brass(
            v,
            t2,
            synth::mtof(m) * p,
            0.14,
            0.38,
            0.01,
            0.45,
            3800.0,
            Some(pan),
        );
    }
    crash(v, t2, 0.4, 1.6, None);
    tom(v, t2, 70.0, 0.8, 0.35, None);
    kick(v, t2, 0.7, 165.0, 46.0, 0.34, 0.085, None);
}

/// `zone_ours` build ≡ audio.js L1836-1849.
fn b_zone_ours(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 4600.0, 0.8, Some(v.out), None);
    let pw = Wave::Pulse(0.25);
    flood_burst(v, t, p, 0.55);
    kick(v, t, 0.75, 165.0, 46.0, 0.34, 0.085, None);
    for (i, m) in [67.0, 72.0, 76.0, 79.0].iter().enumerate() {
        v.tone(Tone {
            t: i as f64 * 0.055,
            wave: Some(pw),
            f: synth::mtof(*m) * p,
            a: Some(0.002),
            h: Some(0.04),
            d: Some(0.13),
            peak: Some(0.26),
            to: Some(lp),
            ..Default::default()
        });
    }
    for (m, pan) in [(72.0, -0.3), (76.0, 0.3), (79.0, 0.0), (84.0, 0.12)] {
        brass(
            v,
            t + 0.22,
            synth::mtof(m) * p,
            0.42,
            if m == 84.0 { 0.2 } else { 0.34 },
            0.01,
            0.36,
            4600.0,
            Some(pan),
        );
    }
    bass(
        v,
        t + 0.22,
        synth::mtof(48.0) * p,
        0.4,
        0.55,
        "sub",
        2.2,
        None,
    );
    crash(v, t + 0.22, 0.28, 0.9, None);
    for (i, m) in [91.0, 96.0].iter().enumerate() {
        bell(
            v,
            t + 0.26 + i as f64 * 0.07,
            synth::mtof(*m) * p,
            0.16,
            0.7,
            3.5,
            1.6,
            None,
        );
    }
    v.nz(Nz {
        t: 0.08,
        f: Some(900.0),
        f1: Some(5200.0),
        sw: Some(0.2),
        q: Some(1.5),
        a: Some(0.12),
        d: Some(0.14),
        peak: Some(0.26),
        ..Default::default()
    });
}

/// `zone_theirs` build ≡ audio.js L1852-1866.
fn b_zone_theirs(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 2600.0, 0.9, Some(v.out), None);
    let pw = Wave::Pulse(0.3);
    kick(v, t, 0.95, 120.0, 38.0, 0.5, 0.085, None);
    tom(v, t + 0.02, 55.0, 0.7, 0.35, None);
    for (i, m) in [70.0, 66.0, 63.0, 58.0].iter().enumerate() {
        v.tone(Tone {
            t: i as f64 * 0.06,
            wave: Some(pw),
            f: synth::mtof(*m) * p,
            a: Some(0.002),
            h: Some(0.045),
            d: Some(0.12),
            peak: Some(0.24),
            to: Some(lp),
            ..Default::default()
        });
    }
    for (m, pan) in [(51.0, -0.3), (54.0, 0.3), (57.0, -0.1), (58.0, 0.15)] {
        brass(
            v,
            t + 0.24,
            synth::mtof(m) * p,
            0.5,
            0.32,
            0.02,
            0.4,
            1700.0,
            Some(pan),
        );
    }
    bass(
        v,
        t + 0.24,
        synth::mtof(39.0) * p,
        0.5,
        0.6,
        "sub",
        2.2,
        None,
    );
    let lp2 = v.filter(BqKind::Lowpass, 1000.0, 2.5, Some(v.out), None);
    v.tone(Tone {
        t: 0.24,
        wave: Some(Wave::Saw),
        f: 233.0 * p,
        f1: Some(98.0 * p),
        sw: Some(0.6),
        a: Some(0.01),
        d: Some(0.6),
        peak: Some(0.16),
        to: Some(lp2),
        ..Default::default()
    });
    let hl = v.filter(BqKind::Lowpass, 1800.0, 1.4, Some(v.out), None);
    v.tone(Tone {
        t: 0.26,
        wave: Some(Wave::Square),
        f: 466.0 * p,
        a: Some(0.01),
        h: Some(0.12),
        d: Some(0.05),
        peak: Some(0.12),
        to: Some(hl),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.44,
        wave: Some(Wave::Square),
        f: 415.0 * p,
        a: Some(0.01),
        h: Some(0.16),
        d: Some(0.12),
        peak: Some(0.12),
        to: Some(hl),
        ..Default::default()
    });
}

/// `zone_lost` build ≡ audio.js L1868-1878.
fn b_zone_lost(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 2800.0, 0.9, Some(v.out), None);
    let pw = Wave::Pulse(0.25);
    crack(v, t, p, 0.9, -1.0);
    let lp2 = v.filter(BqKind::Lowpass, 2400.0, 3.0, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 980.0 * p,
        f1: Some(150.0 * p),
        sw: Some(0.38),
        a: Some(0.002),
        d: Some(0.4),
        peak: Some(0.2),
        to: Some(lp2),
        ..Default::default()
    });
    tom(v, t + 0.02, 82.0, 0.7, 0.35, None);
    for (i, m) in [74.0, 69.0].iter().enumerate() {
        v.tone(Tone {
            t: 0.1 + i as f64 * 0.12,
            wave: Some(pw),
            f: synth::mtof(*m) * p,
            a: Some(0.002),
            h: Some(0.07),
            d: Some(0.18),
            peak: Some(0.22),
            to: Some(lp),
            ..Default::default()
        });
    }
    for (m, pan) in [(57.0, -0.2), (60.0, 0.2), (64.0, 0.0)] {
        brass(
            v,
            t + 0.24,
            synth::mtof(m) * p,
            0.3,
            0.2,
            0.02,
            0.3,
            1900.0,
            Some(pan),
        );
    }
}

/// `zone_broken` build ≡ audio.js L1881-1891.
fn b_zone_broken(v: &mut Vb, p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 4200.0, 0.8, Some(v.out), None);
    let pw = Wave::Pulse(0.25);
    crack(v, t, p, 0.8, 1.0);
    v.nz(Nz {
        f: Some(700.0),
        f1: Some(4800.0),
        sw: Some(0.25),
        q: Some(1.6),
        a: Some(0.08),
        d: Some(0.14),
        peak: Some(0.24),
        ..Default::default()
    });
    for (i, m) in [76.0, 83.0].iter().enumerate() {
        v.tone(Tone {
            t: 0.08 + i as f64 * 0.08,
            wave: Some(pw),
            f: synth::mtof(*m) * p,
            a: Some(0.002),
            h: Some(0.05),
            d: Some(0.16),
            peak: Some(0.24),
            to: Some(lp),
            ..Default::default()
        });
    }
    bell(v, t + 0.16, synth::mtof(88.0) * p, 0.2, 0.6, 3.5, 1.6, None);
    kick(v, t, 0.45, 165.0, 46.0, 0.34, 0.085, None);
}

/// `zone_warn` build ≡ audio.js L1893-1903.
fn b_zone_warn(v: &mut Vb, p: f64) {
    let bp = v.filter(BqKind::Bandpass, 1500.0, 1.1, Some(v.out), None);
    for t in [0.0, 0.13, 0.36, 0.49] {
        v.tone(Tone {
            t,
            wave: Some(Wave::Square),
            f: 1245.0 * p,
            f1: Some(1175.0 * p),
            sw: Some(0.08),
            a: Some(0.002),
            h: Some(0.06),
            d: Some(0.03),
            peak: Some(0.34),
            to: Some(bp),
            ..Default::default()
        });
        v.tone(Tone {
            t,
            f: 622.0 * p,
            a: Some(0.002),
            h: Some(0.06),
            d: Some(0.04),
            peak: Some(0.2),
            ..Default::default()
        });
    }
    let lp = v.filter(BqKind::Lowpass, 500.0, 1.0, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 110.0 * p,
        a: Some(0.01),
        h: Some(0.5),
        d: Some(0.1),
        peak: Some(0.1),
        to: Some(lp),
        ..Default::default()
    });
}

/// `zone_chance` build ≡ audio.js L1905-1913.
fn b_zone_chance(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 3600.0, 0.8, Some(v.out), None);
    let pw = Wave::Pulse(0.2);
    for (t, m0, m1) in [(0.0, 67.0, 74.0), (0.16, 71.0, 79.0)] {
        v.tone(Tone {
            t,
            wave: Some(pw),
            f: synth::mtof(m0) * p,
            f1: Some(synth::mtof(m1) * p),
            sw: Some(0.1),
            a: Some(0.002),
            h: Some(0.07),
            d: Some(0.12),
            peak: Some(0.45),
            to: Some(lp),
            ..Default::default()
        });
    }
    v.tone(Tone {
        t: 0.16,
        wave: Some(Wave::Triangle),
        f: synth::mtof(83.0) * p,
        a: Some(0.002),
        d: Some(0.25),
        peak: Some(0.16),
        ..Default::default()
    });
    plips(v, 0.04, 4, 0.3, [900.0, 1900.0], 0.22, p);
}

/// `zone_flood` build ≡ audio.js L1915-1927.
fn b_zone_flood(v: &mut Vb, p: f64) {
    let t = v.t;
    flood_burst(v, t, p, 1.0);
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 700.0 * p, 1.4, Some(g), None);
    synth::sweep(v.g.param(bp), t, 700.0 * p, 2600.0 * p, 0.45, false);
    synth::pts(v.g.param(g), t, &[(0.0, 0.0), (0.12, 0.45), (0.5, 0.0)]);
    v.noise(NoiseKind::White, t, Some(t + 0.52), Some(bp), 1.0);
    let mut tt = 0.12;
    for i in 0..8 {
        tt += v.r(0.03, 0.07);
        let f = v.r(500.0, 1500.0) * p;
        let d = v.r(0.018, 0.035);
        let rise = v.r(1.4, 2.0);
        v.bub(t + tt, f, 0.2 * (1.0 - i as f64 / 10.0), d, rise, None);
    }
}

/// `zone_wipe` build ≡ audio.js L1929-1938.
fn b_zone_wipe(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let hp = v.filter(BqKind::Bandpass, 5000.0, 0.9, Some(g), None);
    synth::sweep(v.g.param(hp), t, 6500.0 * p, 700.0 * p, 0.6, false);
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.08, 0.5), (0.45, 0.35), (0.7, 0.0)],
    );
    v.noise(NoiseKind::Pink, t, Some(t + 0.72), Some(hp), 1.0);
    v.tone(Tone {
        t: 0.35,
        f: 140.0 * p,
        f1: Some(420.0 * p),
        sw: Some(0.22),
        a: Some(0.01),
        d: Some(0.22),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `zone_final` build ≡ audio.js L1940-1952.
fn b_zone_final(v: &mut Vb, p: f64) {
    let t = v.t;
    let c = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(c),
        t,
        &[(0.0, 0.0), (0.05, 0.12), (0.55, 0.7), (0.6, 0.0)],
    );
    snare_roll(v, t, Some(t + 0.62), p, c);
    for (bt, ms) in [
        (0.62, &[62.0, 69.0][..]),
        (0.8, &[64.0, 71.0][..]),
        (0.98, &[67.0, 74.0, 79.0][..]),
    ] {
        for m in ms.iter() {
            brass(
                v,
                t + bt,
                synth::mtof(*m) * p,
                if bt > 0.9 { 0.6 } else { 0.12 },
                0.3,
                0.008,
                if bt > 0.9 { 0.45 } else { 0.08 },
                4200.0,
                Some(if (*m as i64) % 2 != 0 { 0.25 } else { -0.25 }),
            );
        }
        tom(v, t + bt, synth::mtof(ms[0] - 24.0) * p, 0.6, 0.35, None);
    }
    crash(v, t + 0.98, 0.4, 1.4, None);
    kick(v, t + 0.98, 0.8, 165.0, 46.0, 0.34, 0.085, None);
}

/// `zone_hum` loop ≡ audio.js L1962-1995 (the `pitch(q)` / `params(mode,
/// tension)` callbacks are runtime controls; the baked burst uses q = 1,
/// mode = neutral, tension = 0, so only the neutral HUM preset is built).
fn l_zone_hum(v: &mut Vb, p: f64) {
    let t = v.t;
    // HUM.neutral (audio.js L1957-1961)
    let lp = v.filter(BqKind::Lowpass, 1300.0, 0.9, Some(v.out), None);
    let amp = v.gain(0.72, Some(lp));
    v.lfo(0.7, 0.18, ParamRef::Gain(amp), t, None, Wave::Sine);
    let soft = v.gain(0.62, Some(amp));
    let iv = v.gain(0.42, Some(soft));
    let hard = v.gain(0.0, Some(amp));
    v.osc(Wave::Triangle, 196.0 * p, t, None, Some(soft));
    v.osc(Wave::Triangle, 196.0 * 1.5 * p, t, None, Some(iv));
    v.osc(Wave::Saw, 196.0 * 0.5 * p, t, None, Some(hard));
    let o4 = v.osc(Wave::Saw, 196.0 * 0.5 * p, t, None, Some(hard));
    v.g.param2(o4).set_default(4.0);
    let air = v.gain(0.05, Some(v.out));
    let airbp = v.filter(BqKind::Bandpass, 2600.0, 1.1, Some(air), None);
    v.noise(NoiseKind::Pink, t, None, Some(airbp), 1.0);
}

/// `zone_penalty` build ≡ audio.js L1997-2006.
fn b_zone_penalty(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 2400.0, 1.0, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 330.0 * p,
        a: Some(0.002),
        h: Some(0.07),
        d: Some(0.05),
        peak: Some(0.26),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.11,
        wave: Some(Wave::Square),
        f: 247.0 * p,
        a: Some(0.002),
        h: Some(0.14),
        d: Some(0.12),
        peak: Some(0.28),
        to: Some(lp),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.11,
        wave: Some(Wave::Saw),
        f: 123.5 * p,
        a: Some(0.004),
        h: Some(0.12),
        d: Some(0.14),
        peak: Some(0.15),
        to: Some(lp),
        ..Default::default()
    });
    tom(v, v.t + 0.1, 70.0, 0.55, 0.35, None);
}

/// `zone_shift` build ≡ audio.js L2008-2020.
fn b_zone_shift(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 400.0, 1.6, Some(g), None);
    synth::sweep(v.g.param(bp), t, 400.0, 3200.0, 0.35, false);
    synth::pts(v.g.param(g), t, &[(0.0, 0.0), (0.25, 0.28), (0.42, 0.0)]);
    v.noise(NoiseKind::White, t, Some(t + 0.44), Some(bp), 1.0);
    bell(v, t + 0.12, synth::mtof(84.0) * p, 0.3, 0.9, 3.5, 1.4, None);
    bell(v, t + 0.36, synth::mtof(79.0) * p, 0.3, 1.1, 3.5, 1.4, None);
    pad(
        v,
        t + 0.12,
        synth::mtof(72.0) * p,
        0.5,
        0.08,
        0.02,
        0.5,
        2200.0,
        None,
    );
}

/// `zone_overtime` build ≡ audio.js L2022-2033.
fn b_zone_overtime(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    synth::ahr(v.g.param(g), t, 0.03, 0.2, 0.8, 0.25);
    let lp = v.filter(BqKind::Lowpass, 2600.0, 1.0, Some(g), None);
    let o = v.osc(Wave::Square, 700.0 * p, t, Some(t + 1.12), Some(lp));
    v.lfo(
        5.0,
        180.0 * p,
        ParamRef::Freq(o),
        t,
        Some(t + 1.12),
        Wave::Square,
    );
    for (m, pan) in [(62.0, -0.3), (65.0, 0.3), (69.0, -0.1), (74.0, 0.15)] {
        brass(
            v,
            t + 0.05,
            synth::mtof(m) * p,
            0.5,
            0.3,
            0.01,
            0.4,
            4000.0,
            Some(pan),
        );
    }
    crash(v, t + 0.05, 0.4, 1.6, None);
    kick(v, t + 0.05, 0.7, 165.0, 46.0, 0.34, 0.085, None);
}

/// `zone_tick` build ≡ audio.js L2035-2042.
fn b_zone_tick(v: &mut Vb, p: f64) {
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 1760.0 * p,
        a: Some(0.0008),
        d: Some(0.05),
        peak: Some(0.45),
        ..Default::default()
    });
    v.tone(Tone {
        f: 880.0 * p,
        a: Some(0.001),
        d: Some(0.07),
        peak: Some(0.22),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(6000.0),
        a: Some(0.0003),
        d: Some(0.008),
        peak: Some(0.22),
        ..Default::default()
    });
}

/// `judge_drumroll` build ≡ audio.js L2054-2065. JS also defines an open
/// `loop` variant (L2061-2064) for held rolls; `play()` prefers `build`
/// (L284), so the baked one-shot uses the build path.
fn b_judge_drumroll(v: &mut Vb, p: f64) {
    let t = v.t;
    let c = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(c),
        t,
        &[(0.0, 0.0), (0.04, 0.25), (2.85, 1.0), (3.0, 0.0)],
    );
    snare_roll(v, t, Some(t + 3.02), p, c);
}

/// `judge_reveal` build ≡ audio.js L2066-2074.
fn b_judge_reveal(v: &mut Vb, p: f64) {
    let t = v.t;
    crash(v, t, 0.8, 2.0, None);
    kick(v, t, 0.9, 165.0, 46.0, 0.34, 0.085, None);
    tom(v, t, 65.0, 0.6, 0.35, None);
    for (m, pan) in [
        (48.0, 0.0),
        (60.0, -0.3),
        (64.0, 0.3),
        (67.0, -0.15),
        (72.0, 0.15),
    ] {
        brass(
            v,
            t,
            synth::mtof(m) * p,
            1.0,
            0.3,
            0.015,
            0.6,
            3600.0,
            Some(pan),
        );
    }
    for (i, m) in [84.0, 88.0, 91.0, 96.0].iter().enumerate() {
        bell(
            v,
            t + 0.05 + i as f64 * 0.05,
            synth::mtof(*m) * p,
            0.14,
            0.9,
            3.5,
            1.6,
            None,
        );
    }
}

/// `victory_fanfare` build ≡ audio.js L2075-2096.
fn b_victory_fanfare(v: &mut Vb, p: f64) {
    let t = v.t;
    let b = 60.0 / 132.0;
    for (bt, m, len) in [
        (0.0, 67.0, 0.3),
        (0.333, 72.0, 0.3),
        (0.667, 76.0, 0.3),
        (1.0, 79.0, 0.9),
        (2.0, 76.0, 0.45),
        (2.5, 79.0, 0.45),
        (3.0, 81.0, 0.7),
        (3.75, 83.0, 0.22),
        (4.0, 84.0, 2.6),
    ] {
        brass(
            v,
            t + bt * b,
            synth::mtof(m) * p,
            len * b,
            0.5,
            0.012,
            0.25,
            4500.0,
            None,
        );
    }
    for (bt, ms, len) in [
        (1.0, &[60.0, 64.0, 67.0][..], 0.9),
        (2.0, &[57.0, 60.0, 64.0][..], 0.9),
        (3.0, &[53.0, 57.0, 60.0][..], 0.45),
        (3.5, &[55.0, 59.0, 62.0][..], 0.45),
        (4.0, &[48.0, 55.0, 60.0, 64.0, 67.0][..], 2.6),
    ] {
        for (i, m) in ms.iter().enumerate() {
            brass(
                v,
                t + bt * b,
                synth::mtof(*m) * p,
                len * b,
                0.2,
                0.02,
                0.3,
                2600.0,
                Some(if i % 2 != 0 { 0.35 } else { -0.35 }),
            );
        }
    }
    for (bt, m) in [
        (1.0, 36.0),
        (2.0, 33.0),
        (3.0, 29.0),
        (3.5, 31.0),
        (4.0, 36.0),
    ] {
        bass(
            v,
            t + bt * b,
            synth::mtof(m) * p,
            (if bt == 4.0 { 2.4 } else { 0.8 }) * b,
            0.7,
            "sub",
            2.2,
            None,
        );
        tom(v, t + bt * b, synth::mtof(m + 12.0) * p, 0.6, 0.35, None);
    }
    for bt in [0.0, 0.333, 0.667] {
        snare(v, t + bt * b, 0.45, 205.0, 0.19, None);
    }
    kick(v, t + b, 0.8, 165.0, 46.0, 0.34, 0.085, None);
    kick(v, t + 4.0 * b, 0.9, 165.0, 46.0, 0.34, 0.085, None);
    crash(v, t + 4.0 * b, 0.6, 2.0, None);
    for (i, m) in [84.0, 88.0, 91.0, 96.0].iter().enumerate() {
        bell(
            v,
            t + (4.5 + i as f64 * 0.25) * b,
            synth::mtof(*m) * p,
            0.12,
            0.8,
            3.5,
            1.6,
            None,
        );
    }
}

/// `defeat_jingle` build ≡ audio.js L2097-2117.
fn b_defeat_jingle(v: &mut Vb, p: f64) {
    let t = v.t;
    for (tt, m, len) in [
        (0.0, 72.0, 0.22),
        (0.28, 67.0, 0.22),
        (0.56, 68.0, 0.26),
        (0.9, 67.0, 1.1),
    ] {
        let t0 = t + tt;
        let f = synth::mtof(m) * p;
        let g = v.gain(0.0, Some(v.out));
        let end = synth::adsr(v.g.param(g), t0, 0.03, 0.2, 0.8, 0.3, len, 0.2);
        let lp = v.filter(BqKind::Lowpass, 300.0, 5.0, Some(g), None);
        v.g.param(lp).set(300.0, t0);
        v.g.param(lp).exp(1500.0, t0 + 0.12f64.min(len * 0.5));
        v.g.param(lp).exp(700.0, t0 + len + 0.1);
        let mut oscs = Vec::new();
        for dt in [-6.0, 6.0] {
            let o = v.osc(Wave::Saw, f, t0, Some(end + 0.01), Some(lp));
            v.g.param2(o).set_default(dt);
            oscs.push(o);
        }
        if len > 0.5 {
            v.lfo(
                5.0,
                300.0,
                ParamRef::Freq(lp),
                t0 + 0.15,
                Some(end),
                Wave::Sine,
            );
            let (depth, _lfo_osc) = v.lfo(
                5.0,
                20.0,
                ParamRef::Detune(oscs[0]),
                t0 + 0.15,
                Some(end),
                Wave::Sine,
            );
            v.g.connect_param(depth, ParamRef::Detune(oscs[1]));
        }
        bass(v, t0, f / 4.0, len, 0.35, "sub", 2.2, None);
    }
    v.bub(t + 2.15, 300.0 * p, 0.2, 0.08, 2.2, None);
}

/// `xp_tick` build ≡ audio.js L2118-2124.
fn b_xp_tick(v: &mut Vb, p: f64) {
    let hp = v.filter(BqKind::Highpass, 1000.0, 0.7, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 1900.0 * p,
        a: Some(0.0005),
        d: Some(0.03),
        peak: Some(0.18),
        to: Some(hp),
        ..Default::default()
    });
    v.tone(Tone {
        f: 3800.0 * p,
        a: Some(0.0005),
        d: Some(0.02),
        peak: Some(0.2),
        ..Default::default()
    });
}

/// `level_up` build ≡ audio.js L2125-2134.
fn b_level_up(v: &mut Vb, p: f64) {
    let t = v.t;
    for (i, m) in [72.0, 76.0, 79.0, 84.0, 88.0, 91.0, 96.0]
        .iter()
        .enumerate()
    {
        bell(
            v,
            t + i as f64 * 0.05,
            synth::mtof(*m) * p,
            0.2,
            0.5,
            2.0,
            1.2,
            None,
        );
    }
    for m in [72.0, 76.0, 79.0, 84.0] {
        pad(
            v,
            t + 0.35,
            synth::mtof(m) * p,
            0.3,
            0.12,
            0.02,
            0.6,
            3000.0,
            None,
        );
    }
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(8000.0),
        a: Some(0.3),
        d: Some(0.6),
        peak: Some(0.06),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.35,
        f: 65.4 * p,
        a: Some(0.005),
        d: Some(0.5),
        peak: Some(0.3),
        ..Default::default()
    });
}

/// `step_dry` build ≡ audio.js L2137-2145.
fn b_step_dry(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 120.0 * p,
        f1: Some(70.0 * p),
        sw: Some(0.03),
        a: Some(0.001),
        d: Some(0.045),
        peak: Some(0.55),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(2600.0 * p),
        q: Some(0.9),
        a: Some(0.002),
        d: Some(0.035),
        peak: Some(0.28),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(5200.0),
        a: Some(0.001),
        d: Some(0.012),
        peak: Some(0.12),
        ..Default::default()
    });
}

/// `step_ink` build ≡ audio.js L2146-2154.
fn b_step_ink(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(1300.0 * p),
        f1: Some(520.0 * p),
        sw: Some(0.05),
        q: Some(3.5),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.6),
        ..Default::default()
    });
    v.tone(Tone {
        f: 190.0 * p,
        f1: Some(110.0 * p),
        sw: Some(0.04),
        a: Some(0.001),
        d: Some(0.05),
        peak: Some(0.35),
        ..Default::default()
    });
    plips(v, 0.01, 2, 0.05, [900.0, 1900.0], 0.12, p);
}

/// `step_enemy` build ≡ audio.js L2155-2163.
fn b_step_enemy(v: &mut Vb, p: f64) {
    v.nz(Nz {
        f: Some(700.0 * p),
        f1: Some(330.0 * p),
        sw: Some(0.09),
        q: Some(5.0),
        a: Some(0.004),
        d: Some(0.1),
        peak: Some(0.6),
        ..Default::default()
    });
    v.tone(Tone {
        f: 140.0 * p,
        f1: Some(85.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.08),
        peak: Some(0.35),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.07,
        ft: Some(Some(BqKind::Highpass)),
        f: Some(3500.0),
        a: Some(0.001),
        d: Some(0.015),
        peak: Some(0.25),
        ..Default::default()
    });
}

/// `ink_drip` build ≡ audio.js L2164-2167.
fn b_ink_drip(v: &mut Vb, p: f64) {
    plips(v, 0.0, 1, 0.01, [1100.0, 2100.0], 0.3, p);
}

/// `gull` build ≡ audio.js L2168-2176.
fn b_gull(v: &mut Vb, p: f64) {
    let bp = v.filter(BqKind::Bandpass, 2300.0 * p, 2.2, Some(v.out), None);
    v.tone(Tone {
        wave: Some(Wave::Saw),
        f: 1450.0 * p,
        f1: Some(980.0 * p),
        sw: Some(0.18),
        a: Some(0.02),
        d: Some(0.2),
        peak: Some(0.35),
        to: Some(bp),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.26,
        wave: Some(Wave::Saw),
        f: 1320.0 * p,
        f1: Some(900.0 * p),
        sw: Some(0.22),
        a: Some(0.02),
        d: Some(0.26),
        peak: Some(0.3),
        to: Some(bp),
        ..Default::default()
    });
}

/// `ferry_horn` build ≡ audio.js L2177-2189.
fn b_ferry_horn(v: &mut Vb, p: f64) {
    let lp = v.filter(BqKind::Lowpass, 820.0 * p, 0.9, Some(v.out), None);
    for (tt, h) in [(0.0, 1.35), (2.05, 0.55)] {
        v.tone(Tone {
            t: tt,
            wave: Some(Wave::Saw),
            f: 98.0 * p,
            f1: Some(97.0 * p),
            sw: Some(h),
            a: Some(0.14),
            h: Some(h),
            d: Some(0.65),
            peak: Some(0.42),
            to: Some(lp),
            ..Default::default()
        });
        v.tone(Tone {
            t: tt,
            wave: Some(Wave::Saw),
            f: 123.5 * p,
            f1: Some(122.4 * p),
            sw: Some(h),
            a: Some(0.16),
            h: Some(h),
            d: Some(0.7),
            peak: Some(0.34),
            to: Some(lp),
            ..Default::default()
        });
        v.tone(Tone {
            t: tt,
            wave: Some(Wave::Square),
            f: 49.0 * p,
            a: Some(0.2),
            h: Some(h),
            d: Some(0.6),
            peak: Some(0.16),
            to: Some(lp),
            ..Default::default()
        });
    }
}

/// `halyard_clink` build ≡ audio.js L2190-2199 (the second strike offset is
/// drawn once when the strike array literal is built, before the loop).
fn b_halyard_clink(v: &mut Vb, p: f64) {
    let f = 1850.0 * p;
    let t2 = 0.07 + v.r(0.0, 0.05);
    for (tt, k) in [(0.0, 1.0), (t2, 0.45)] {
        for (r, a, d) in [
            (1.0, 0.5, 0.2),
            (2.76, 0.26, 0.1),
            (5.4, 0.12, 0.05),
            (8.93, 0.06, 0.03),
        ] {
            v.tone(Tone {
                t: tt,
                f: f * r,
                a: Some(0.0008),
                d: Some(d),
                peak: Some(a * k),
                ..Default::default()
            });
        }
    }
}

/// `harbor_ambience` loop ≡ audio.js L2200-2215 (no runtime controls).
fn l_harbor_ambience(v: &mut Vb, _p: f64) {
    let t = v.t;
    let lp = v.filter(BqKind::Lowpass, 520.0, 0.7, Some(v.out), None);
    let wash = v.gain(0.9, Some(lp));
    v.noise(NoiseKind::Brown, t, None, Some(wash), 1.0);
    v.lfo(0.11, 0.45, ParamRef::Gain(wash), t, None, Wave::Sine);
    let hp = v.filter(BqKind::Bandpass, 2600.0, 0.6, Some(v.out), None);
    let hiss = v.gain(0.08, Some(hp));
    v.noise(NoiseKind::Pink, t, None, Some(hiss), 1.0);
    v.lfo(0.17, 0.06, ParamRef::Gain(hiss), t, None, Wave::Sine);
}

/// `boss_roar` build ≡ audio.js L2219-2244 (the `ref: 16` field is a live
/// 3-D referenceDistance, not baked — deviation D-T50-6①).
fn b_boss_roar(v: &mut Vb, p: f64) {
    let t = v.t;
    let d = 1.9;
    let envg = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(envg),
        t,
        &[(0.0, 0.0), (0.12, 0.75), (0.5, 1.0), (1.3, 0.85), (d, 0.0)],
    );
    let flutter = v.gain(1.0, Some(envg));
    v.lfo(
        24.0,
        0.4,
        ParamRef::Gain(flutter),
        t,
        Some(t + d),
        Wave::Sine,
    );
    let drive = v.shaper(3.0, Some(flutter));
    let f1 = v.filter(BqKind::Bandpass, 380.0, 2.4, Some(drive), None);
    let g07 = v.gain(0.7, Some(drive));
    let f2 = v.filter(BqKind::Bandpass, 1050.0, 3.2, Some(g07), None);
    synth::sweep(v.g.param(f1), t, 300.0, 720.0, 0.5, false);
    v.g.param(f1).exp(240.0, t + d);
    synth::sweep(v.g.param(f2), t, 900.0, 1500.0, 0.5, false);
    v.g.param(f2).exp(700.0, t + d);
    let mix = v.gain(1.0, None);
    v.g.connect(mix, f1);
    v.g.connect(mix, f2);
    let lp = v.filter(BqKind::Lowpass, 2600.0, 0.8, Some(mix), None);
    for (det, wave, lv) in [
        (-14.0, Wave::Saw, 0.5),
        (9.0, Wave::Saw, 0.5),
        (0.0, Wave::Square, 0.3),
    ] {
        let og = v.gain(lv, Some(lp));
        let o = v.osc(wave, 52.0 * p, t, Some(t + d + 0.05), Some(og));
        v.g.param2(o).set_default(det);
        v.g.param(o).set(50.0 * p, t);
        v.g.param(o).lin(80.0 * p, t + 0.45);
        v.g.param(o).lin(58.0 * p, t + 1.3);
        v.g.param(o).lin(38.0 * p, t + d);
    }
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        f: Some(950.0),
        f1: Some(420.0),
        sw: Some(d),
        q: Some(0.9),
        a: Some(0.1),
        h: Some(1.1),
        d: Some(0.6),
        peak: Some(0.5),
        ..Default::default()
    });
    v.tone(Tone {
        f: 46.0 * p,
        f1: Some(30.0 * p),
        sw: Some(d),
        a: Some(0.1),
        h: Some(1.0),
        d: Some(0.7),
        peak: Some(0.75),
        ..Default::default()
    });
    bloops(v, 0.25, 6, 1.2, [170.0, 420.0], 0.22, p);
}

/// `boss_step` build ≡ audio.js L2245-2255.
fn b_boss_step(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 72.0 * p,
        f1: Some(32.0 * p),
        sw: Some(0.18),
        a: Some(0.002),
        d: Some(0.32),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        kind: Some(NoiseKind::Brown),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(700.0),
        f1: Some(110.0),
        sw: Some(0.25),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(0.8),
        ..Default::default()
    });
    v.nz(Nz {
        t: 0.004,
        f: Some(380.0 * p),
        q: Some(6.0),
        a: Some(0.001),
        d: Some(0.12),
        peak: Some(0.32),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.01,
        wave: Some(Wave::Triangle),
        f: 233.0 * p,
        a: Some(0.001),
        d: Some(0.2),
        peak: Some(0.12),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.01,
        wave: Some(Wave::Triangle),
        f: 347.0 * p,
        a: Some(0.001),
        d: Some(0.15),
        peak: Some(0.08),
        ..Default::default()
    });
    plips(v, 0.02, 2, 0.06, [260.0, 520.0], 0.1, p);
}

/// `boss_slam` build ≡ audio.js L2256-2267.
fn b_boss_slam(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 82.0 * p,
        f1: Some(25.0 * p),
        sw: Some(0.9),
        a: Some(0.002),
        d: Some(1.25),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1600.0),
        a: Some(0.0003),
        d: Some(0.06),
        peak: Some(1.0),
        ..Default::default()
    });
    let g07 = v.gain(0.7, Some(v.out));
    let sh = v.shaper(3.0, Some(g07));
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(5200.0),
        f1: Some(120.0),
        sw: Some(0.9),
        q: Some(0.8),
        a: Some(0.002),
        d: Some(1.1),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    for (f, pk, d) in [
        (183.0, 0.26, 0.95),
        (497.0, 0.18, 0.6),
        (1130.0, 0.1, 0.42),
        (1720.0, 0.06, 0.3),
    ] {
        v.tone(Tone {
            wave: Some(Wave::Triangle),
            f: f * p,
            a: Some(0.001),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
    }
    big_splat(v, 0.03, p * 0.8, 0.9);
    v.nz(Nz {
        t: 0.12,
        f: Some(700.0),
        f1: Some(190.0),
        sw: Some(1.0),
        q: Some(1.4),
        a: Some(0.02),
        d: Some(1.0),
        peak: Some(0.5),
        ..Default::default()
    });
}

/// `boss_tele` build ≡ audio.js L2268-2278.
fn b_boss_tele(v: &mut Vb, p: f64) {
    let t = v.t;
    for (m, pan) in [(38.0, -0.2), (45.0, 0.2), (50.0, 0.0)] {
        brass(
            v,
            t,
            synth::mtof(m) * p,
            0.34,
            0.4,
            0.03,
            0.3,
            1400.0,
            Some(pan),
        );
    }
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.05, 0.25), (0.45, 0.18), (0.6, 0.0)],
    );
    let bp = v.filter(BqKind::Bandpass, 700.0 * p, 9.0, Some(g), None);
    synth::sweep(v.g.param(bp), t, 700.0 * p, 1100.0 * p, 0.55, false);
    v.noise(NoiseKind::Pink, t, Some(t + 0.62), Some(bp), 1.0);
}

/// `boss_whistle` build ≡ audio.js L2279-2289.
fn b_boss_whistle(v: &mut Vb, p: f64) {
    let t = v.t;
    let d = 0.95;
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.08, 0.5), (d - 0.1, 0.75), (d, 0.0)],
    );
    let o = v.osc(Wave::Sine, 2100.0 * p, t, Some(t + d + 0.02), Some(g));
    v.g.param(o).exp(640.0 * p, t + d);
    v.lfo(9.0, 38.0, ParamRef::Freq(o), t, Some(t + d), Wave::Sine);
    v.nz(Nz {
        f: Some(2000.0 * p),
        f1: Some(700.0 * p),
        sw: Some(d),
        q: Some(4.0),
        a: Some(0.1),
        h: Some(d - 0.2),
        d: Some(0.1),
        peak: Some(0.12),
        ..Default::default()
    });
}

/// `boss_barrel` build ≡ audio.js L2290-2298.
fn b_boss_barrel(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1200.0),
        a: Some(0.0005),
        d: Some(0.05),
        peak: Some(0.9),
        ..Default::default()
    });
    for _ in 0..3 {
        let tt = v.r(0.0, 0.04);
        let f = v.r(500.0, 1400.0) * p;
        let d = v.r(0.04, 0.09);
        v.nz(Nz {
            t: tt,
            f: Some(f),
            q: Some(8.0),
            a: Some(0.001),
            d: Some(d),
            peak: Some(0.4),
            ..Default::default()
        });
    }
    v.tone(Tone {
        f: 140.0 * p,
        f1: Some(58.0 * p),
        sw: Some(0.2),
        a: Some(0.002),
        d: Some(0.3),
        peak: Some(0.7),
        ..Default::default()
    });
    big_splat(v, 0.01, p, 1.1);
}

/// `boss_cannon_charge` build ≡ audio.js L2299-2311.
fn b_boss_cannon_charge(v: &mut Vb, p: f64) {
    let t = v.t;
    let d = 1.25;
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.2, 0.35), (d - 0.08, 1.0), (d, 0.0)],
    );
    let trem = v.gain(1.0, Some(g));
    v.lfo(14.0, 0.3, ParamRef::Gain(trem), t, Some(t + d), Wave::Sine);
    let lp = v.filter(BqKind::Lowpass, 600.0, 5.0, Some(trem), None);
    synth::sweep(v.g.param(lp), t, 500.0, 5200.0, d, false);
    let o = v.osc(Wave::Saw, 110.0 * p, t, Some(t + d + 0.02), Some(lp));
    v.g.param(o).exp(640.0 * p, t + d);
    let og05 = v.gain(0.5, Some(trem));
    let o2 = v.osc(Wave::Sine, 220.0 * p, t, Some(t + d + 0.02), Some(og05));
    v.g.param(o2).exp(1280.0 * p, t + d);
    let bub = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(bub),
        t,
        &[(0.0, 0.0), (0.3, 0.3), (d, 0.65), (d + 0.05, 0.0)],
    );
    v.buffer(
        texture("bubbles_bright"),
        t,
        Some(t + d + 0.05),
        Some(bub),
        1.2 * p,
        None,
    );
}

/// `boss_cannon_sweep` loop ≡ audio.js L2312-2324 (the `pitch(q)` callback
/// is a runtime control; the baked burst uses q = 1).
fn l_boss_cannon_sweep(v: &mut Vb, p: f64) {
    let t = v.t;
    let hs = v.gain(0.5, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 1800.0 * p, 0.8, Some(hs), None);
    v.noise(NoiseKind::White, t, None, Some(bp), 1.0);
    v.lfo(6.0, 0.12, ParamRef::Gain(hs), t, None, Wave::Sine);
    let g06 = v.gain(0.6, Some(v.out));
    let lp2 = v.filter(BqKind::Lowpass, 260.0, 1.0, Some(g06), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp2), 1.0);
    let lp3 = v.filter(BqKind::Lowpass, 1400.0, 0.8, Some(v.out), None);
    let g07 = v.gain(0.7, Some(lp3));
    v.buffer(texture("squelch"), t, None, Some(g07), p, None);
    let lp4 = v.filter(BqKind::Lowpass, 300.0, 1.0, Some(v.out), None);
    let g012 = v.gain(0.12, Some(lp4));
    v.osc(Wave::Saw, 55.0 * p, t, None, Some(g012));
}

/// `boss_gallop` loop ≡ audio.js L2325-2338 (the `pitch(q)` callback is a
/// runtime control; the baked burst uses q = 1).
fn l_boss_gallop(v: &mut Vb, p: f64) {
    let t = v.t;
    let am = v.gain(0.0, Some(v.out));
    let st = v.osc(Wave::Stroke(9.0), 8.5 * p, t, None, None);
    let dep = v.gain(1.0, None);
    v.g.connect(st, dep);
    v.g.connect_param(dep, ParamRef::Gain(am));
    let g09 = v.gain(0.9, Some(am));
    v.osc(Wave::Sine, 58.0 * p, t, None, Some(g09));
    let g12 = v.gain(1.2, Some(am));
    let lp = v.filter(BqKind::Lowpass, 500.0, 1.0, Some(g12), None);
    v.noise(NoiseKind::Brown, t, None, Some(lp), 1.0);
    let g035 = v.gain(0.35, Some(am));
    let bp = v.filter(BqKind::Bandpass, 420.0 * p, 5.0, Some(g035), None);
    v.noise(NoiseKind::White, t, None, Some(bp), 1.0);
    let g012 = v.gain(0.12, Some(v.out));
    let bp2 = v.filter(BqKind::Bandpass, 900.0, 0.7, Some(g012), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp2), 1.0);
}

/// `boss_crash` build ≡ audio.js L2339-2349.
fn b_boss_crash(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 96.0 * p,
        f1: Some(30.0 * p),
        sw: Some(0.7),
        a: Some(0.002),
        d: Some(1.0),
        peak: Some(1.0),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(1500.0),
        a: Some(0.0003),
        d: Some(0.08),
        peak: Some(1.0),
        ..Default::default()
    });
    let g06 = v.gain(0.6, Some(v.out));
    let sh = v.shaper(4.0, Some(g06));
    v.nz(Nz {
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(7000.0),
        f1: Some(300.0),
        sw: Some(0.6),
        q: Some(0.8),
        a: Some(0.002),
        d: Some(0.85),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    for (f, pk, d) in [
        (151.0, 0.3, 1.3),
        (412.0, 0.22, 0.9),
        (867.0, 0.16, 0.7),
        (1391.0, 0.1, 0.5),
        (2210.0, 0.06, 0.35),
    ] {
        v.tone(Tone {
            wave: Some(Wave::Triangle),
            f: f * p,
            f1: Some(f * p * 0.96),
            sw: Some(d),
            a: Some(0.001),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
    }
    for _ in 0..9 {
        let tt = 0.08 + v.r(0.0, 0.6);
        let f = v.r(900.0, 3500.0);
        let d = v.r(0.02, 0.06);
        let pk = v.r(0.1, 0.3);
        v.nz(Nz {
            t: tt,
            f: Some(f),
            q: Some(6.0),
            a: Some(0.001),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
    }
}

/// `boss_dizzy` loop ≡ audio.js L2350-2364 (the `pitch(q)` callback is a
/// runtime control; the baked burst uses q = 1).
fn l_boss_dizzy(v: &mut Vb, p: f64) {
    let t = v.t;
    let gate = v.gain(0.0, Some(v.out));
    let am = v.gain(0.5, None);
    v.g.connect_param(am, ParamRef::Gain(gate));
    v.osc(Wave::Square, 3.2, t, None, Some(am));
    v.g.param(gate).set_default(0.5);
    let o = v.osc(Wave::Sine, 1700.0 * p, t, None, Some(gate));
    v.lfo(7.0, 480.0 * p, ParamRef::Freq(o), t, None, Wave::Saw);
    v.lfo(0.8, 260.0 * p, ParamRef::Freq(o), t, None, Wave::Sine);
    let g007 = v.gain(0.07, Some(v.out));
    let o2 = v.osc(Wave::Triangle, 2500.0 * p, t, None, Some(g007));
    v.lfo(5.3, 650.0 * p, ParamRef::Freq(o2), t, None, Wave::Saw);
}

/// `boss_frenzy` loop ≡ audio.js L2365-2379 (the `pitch(q)` callback is a
/// runtime control; the baked burst uses q = 1).
fn l_boss_frenzy(v: &mut Vb, p: f64) {
    let t = v.t;
    let g08 = v.gain(0.8, Some(v.out));
    let bp = v.filter(BqKind::Bandpass, 900.0, 1.2, Some(g08), None);
    v.noise(NoiseKind::Pink, t, None, Some(bp), 1.0);
    v.lfo(3.2 * p, 650.0, ParamRef::Freq(bp), t, None, Wave::Sine);
    let hs = v.gain(0.35, Some(v.out));
    let hp = v.filter(BqKind::Highpass, 2500.0, 0.7, Some(hs), None);
    v.noise(NoiseKind::White, t, None, Some(hp), 1.0);
    v.lfo(3.2 * p, 0.2, ParamRef::Gain(hs), t, None, Wave::Sine);
    let g05 = v.gain(0.5, Some(v.out));
    v.buffer(texture("squelch"), t, None, Some(g05), 1.3 * p, None);
    let lp = v.filter(BqKind::Lowpass, 240.0, 1.0, Some(v.out), None);
    let g01 = v.gain(0.1, Some(lp));
    v.osc(Wave::Saw, 70.0 * p, t, None, Some(g01));
}

/// `crablet_chitter` build ≡ audio.js L2380-2387.
fn b_crablet_chitter(v: &mut Vb, p: f64) {
    for i in 0..9 {
        let tt = i as f64 * 0.035 + v.r(0.0, 0.012);
        let f = v.r(2500.0, 4500.0);
        let d = v.r(0.008, 0.02);
        let pk = v.r(0.3, 0.6);
        v.nz(Nz {
            t: tt,
            ft: Some(Some(BqKind::Highpass)),
            f: Some(f),
            a: Some(0.0005),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
    }
    v.tone(Tone {
        t: 0.05,
        wave: Some(Wave::Square),
        f: 1300.0 * p,
        f1: Some(1900.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.07),
        peak: Some(0.1),
        ..Default::default()
    });
    v.tone(Tone {
        t: 0.18,
        wave: Some(Wave::Square),
        f: 1500.0 * p,
        f1: Some(1100.0 * p),
        sw: Some(0.06),
        a: Some(0.002),
        d: Some(0.06),
        peak: Some(0.08),
        ..Default::default()
    });
}

/// `crablet_pop` build ≡ audio.js L2388-2396.
fn b_crablet_pop(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 900.0 * p,
        f1: Some(170.0 * p),
        sw: Some(0.07),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(0.8),
        ..Default::default()
    });
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2500.0),
        a: Some(0.0005),
        d: Some(0.03),
        peak: Some(0.6),
        ..Default::default()
    });
    bloops(v, 0.01, 3, 0.08, [500.0, 900.0], 0.35, p);
    plips(v, 0.03, 3, 0.1, [900.0, 1500.0], 0.15, p);
}

/// `boss_hit` build ≡ audio.js L2397-2404.
fn b_boss_hit(v: &mut Vb, p: f64) {
    v.tone(Tone {
        f: 190.0 * p,
        f1: Some(88.0 * p),
        sw: Some(0.06),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(0.8),
        ..Default::default()
    });
    v.tone(Tone {
        wave: Some(Wave::Triangle),
        f: 640.0 * p,
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.18),
        ..Default::default()
    });
    v.nz(Nz {
        f: Some(1800.0 * p),
        q: Some(2.0),
        a: Some(0.0005),
        d: Some(0.04),
        peak: Some(0.4),
        ..Default::default()
    });
}

/// `boss_crit` build ≡ audio.js L2405-2415.
fn b_boss_crit(v: &mut Vb, p: f64) {
    v.nz(Nz {
        ft: Some(Some(BqKind::Highpass)),
        f: Some(2200.0),
        a: Some(0.0003),
        d: Some(0.05),
        peak: Some(0.8),
        ..Default::default()
    });
    v.tone(Tone {
        f: 260.0 * p,
        f1: Some(105.0 * p),
        sw: Some(0.08),
        a: Some(0.001),
        d: Some(0.12),
        peak: Some(0.8),
        ..Default::default()
    });
    bell(v, v.t, synth::mtof(91.0) * p, 0.34, 0.35, 3.5, 1.6, None);
    bell(
        v,
        v.t + 0.028,
        synth::mtof(98.0) * p,
        0.2,
        0.3,
        3.5,
        1.6,
        None,
    );
    v.tone(Tone {
        wave: Some(Wave::Square),
        f: 1760.0 * p,
        f1: Some(2640.0 * p),
        sw: Some(0.05),
        a: Some(0.001),
        d: Some(0.06),
        peak: Some(0.07),
        ..Default::default()
    });
    bloops(v, 0.01, 2, 0.05, [500.0, 800.0], 0.22, p);
}

/// `boss_phase` build ≡ audio.js L2416-2426.
fn b_boss_phase(v: &mut Vb, p: f64) {
    let t = v.t;
    crash(v, t, 0.7, 2.0, None);
    kick(v, t, 0.9, 165.0, 46.0, 0.34, 0.085, None);
    tom(v, t, 55.0, 0.8, 0.35, None);
    tom(v, t + 0.17, 49.0, 0.7, 0.35, None);
    tom(v, t + 0.34, 43.0, 0.85, 0.35, None);
    for (m, pan) in [
        (38.0, 0.0),
        (50.0, -0.3),
        (53.0, 0.3),
        (56.0, -0.15),
        (62.0, 0.2),
    ] {
        brass(
            v,
            t,
            synth::mtof(m) * p,
            0.95,
            0.34,
            0.02,
            0.7,
            3000.0,
            Some(pan),
        );
    }
    v.tone(Tone {
        f: 44.0 * p,
        f1: Some(30.0 * p),
        sw: Some(1.2),
        a: Some(0.01),
        d: Some(1.3),
        peak: Some(0.6),
        ..Default::default()
    });
}

/// `boss_title` build ≡ audio.js L2427-2438.
fn b_boss_title(v: &mut Vb, p: f64) {
    let t = v.t;
    kick(v, t, 1.0, 165.0, 46.0, 0.34, 0.085, None);
    crash(v, t, 0.8, 2.2, None);
    tom(v, t, 48.0, 0.9, 0.35, None);
    v.tone(Tone {
        f: 55.0 * p,
        f1: Some(29.0 * p),
        sw: Some(1.2),
        a: Some(0.002),
        d: Some(1.4),
        peak: Some(0.8),
        ..Default::default()
    });
    for (m, pan) in [
        (38.0, 0.0),
        (50.0, -0.3),
        (53.0, 0.3),
        (57.0, -0.15),
        (62.0, 0.2),
        (65.0, 0.0),
    ] {
        brass(
            v,
            t,
            synth::mtof(m) * p,
            0.62,
            0.32,
            0.015,
            0.7,
            3400.0,
            Some(pan),
        );
    }
    let t2 = t + 0.72;
    tom(v, t2, 43.0, 0.85, 0.35, None);
    kick(v, t2, 0.85, 165.0, 46.0, 0.34, 0.085, None);
    for (m, pan) in [
        (34.0, 0.0),
        (46.0, -0.25),
        (50.0, 0.25),
        (53.0, -0.1),
        (58.0, 0.1),
    ] {
        brass(
            v,
            t2,
            synth::mtof(m) * p,
            1.0,
            0.3,
            0.02,
            1.0,
            2800.0,
            Some(pan),
        );
    }
}

/// `boss_defeat` build ≡ audio.js L2439-2463.
fn b_boss_defeat(v: &mut Vb, p: f64) {
    let t = v.t;
    let g = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(g),
        t,
        &[(0.0, 0.0), (0.1, 0.7), (1.0, 0.5), (1.6, 0.0)],
    );
    let lp = v.filter(BqKind::Lowpass, 1500.0, 1.5, Some(g), None);
    synth::sweep(v.g.param(lp), t, 1500.0, 280.0, 1.6, false);
    for det in [-12.0, 7.0] {
        let o = v.osc(Wave::Saw, 92.0 * p, t, Some(t + 1.65), Some(lp));
        v.g.param2(o).set_default(det);
        v.g.param(o).exp(30.0 * p, t + 1.6);
    }
    for i in 0..6 {
        let tt = 0.2 + i as f64 * 0.16 + v.r(0.0, 0.08);
        let f = v.r(180.0, 900.0) * p;
        let d = v.r(0.2, 0.5);
        let pk = v.r(0.08, 0.2);
        v.tone(Tone {
            t: tt,
            wave: Some(Wave::Triangle),
            f,
            a: Some(0.001),
            d: Some(d),
            peak: Some(pk),
            ..Default::default()
        });
        let f2 = v.r(700.0, 2400.0);
        v.nz(Nz {
            t: tt,
            f: Some(f2),
            q: Some(5.0),
            a: Some(0.001),
            d: Some(0.06),
            peak: Some(0.25),
            ..Default::default()
        });
    }
    v.tone(Tone {
        t: 1.1,
        f: 76.0 * p,
        f1: Some(24.0 * p),
        sw: Some(1.0),
        a: Some(0.002),
        d: Some(1.3),
        peak: Some(1.0),
        ..Default::default()
    });
    let g06 = v.gain(0.6, Some(v.out));
    let sh = v.shaper(3.0, Some(g06));
    v.nz(Nz {
        t: 1.1,
        kind: Some(NoiseKind::Pink),
        ft: Some(Some(BqKind::Lowpass)),
        f: Some(4500.0),
        f1: Some(120.0),
        sw: Some(1.0),
        q: Some(0.8),
        a: Some(0.002),
        d: Some(1.2),
        peak: Some(1.0),
        to: Some(sh),
        ..Default::default()
    });
    v.nz(Nz {
        t: 1.35,
        f: Some(300.0),
        f1: Some(2600.0),
        sw: Some(1.2),
        q: Some(0.9),
        a: Some(0.25),
        h: Some(0.6),
        d: Some(1.0),
        peak: Some(0.7),
        ..Default::default()
    });
    let bg = v.gain(0.0, Some(v.out));
    synth::pts(
        v.g.param(bg),
        t + 1.35,
        &[(0.0, 0.0), (0.3, 0.7), (1.6, 0.5), (2.3, 0.0)],
    );
    v.buffer(
        texture("bubbles"),
        t + 1.35,
        Some(t + 3.7),
        Some(bg),
        1.1,
        None,
    );
    for i in 0..5 {
        let tt = 2.0 + i as f64 * 0.28 + v.r(0.0, 0.1);
        let pp = p * v.r(0.8, 1.2);
        big_splat(v, tt, pp, 0.45);
    }
}

/// `boss_sunk` build ≡ audio.js L2464-2476.
fn b_boss_sunk(v: &mut Vb, p: f64) {
    let t = v.t;
    let b = 60.0 / 140.0;
    for (bt, ms, len) in [
        (0.0, &[62.0, 66.0, 69.0, 74.0][..], 0.4),
        (0.5, &[64.0, 67.0, 71.0, 76.0][..], 0.4),
        (1.0, &[66.0, 69.0, 74.0, 78.0][..], 2.2),
    ] {
        for (i, m) in ms.iter().enumerate() {
            brass(
                v,
                t + bt * b,
                synth::mtof(*m) * p,
                len * b,
                if i == ms.len() - 1 { 0.5 } else { 0.24 },
                0.012,
                0.4,
                4200.0,
                Some((i as f64 - 1.5) * 0.25),
            );
        }
    }
    kick(v, t, 0.7, 165.0, 46.0, 0.34, 0.085, None);
    snare(v, t + 0.5 * b, 0.5, 205.0, 0.19, None);
    kick(v, t + b, 0.9, 165.0, 46.0, 0.34, 0.085, None);
    crash(v, t + b, 0.7, 2.0, None);
    bass(
        v,
        t + b,
        synth::mtof(38.0) * p,
        2.0 * b,
        0.7,
        "sub",
        2.2,
        None,
    );
    for (i, m) in [86.0, 90.0, 93.0, 98.0].iter().enumerate() {
        bell(
            v,
            t + (1.3 + i as f64 * 0.18) * b,
            synth::mtof(*m) * p,
            0.12,
            0.8,
            3.5,
            1.6,
            None,
        );
    }
}

// ---------------------------------------------------------------------------
// Bake pipeline
// ---------------------------------------------------------------------------

/// Bake one SFX at nominal pitch (p = 1) exactly like `play()` would spawn
/// it: `build(v, 1, {})` for one-shots; the loop→burst path (L286-292) for
/// loop definitions. Returns the mono 44.1 kHz f32 block.
/// `None` for an unknown name ≡ JS `play()`'s `if (!d) return null` (L261) —
/// nothing is baked and the caller must not spawn (review P2-8).
pub fn bake(name: &str) -> Option<Vec<f32>> {
    let meta = meta(name)?;
    let mut v = Vb::new(0.0, Rng::new(bake_seed(name)));
    if meta.is_loop {
        let len = meta.one_shot.unwrap_or(ONE_SHOT_DEFAULT);
        build_loop(name, &mut v, 1.0);
        // burst envelope on the voice out gain ≡ L290-291 (vol = 1 baked)
        v.out_param().set(0.0, 0.0);
        v.out_param().lin(1.0, 0.05);
        v.out_param().set(1.0, (len - 0.15).max(0.05));
        v.out_param().lin(0.0, len);
        v.kill(len + 0.01);
    } else {
        build_one_shot(name, &mut v, 1.0);
    }
    Some(v.bake())
}

// ---------------------------------------------------------------------------
// Play gate — AudioEngine.play() semantics without the live context
// (minGap merge, per-name + global voice caps, pitch jitter).
// ---------------------------------------------------------------------------

/// A live voice as tracked by the gate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceRec {
    /// JS `born` — parity field (the deque order is what steal uses); read by
    /// the batch-B gate tests.
    #[allow(dead_code)]
    pub born: f64,
    pub end: f64,
}

/// Outcome of a `play()` attempt.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayDecision {
    /// merged into a recent identical sound (minGap) — L267 dropped++
    Merged,
    /// volume below 1e-4 — L269
    Silent,
    /// spawned; `steal` lists the voices to despawn (oldest-first)
    Spawned {
        speed: f64,
        volume: f64,
        steal: usize,
    },
}

/// Gate state ≡ `engine.last` / `engine.byName` / `engine.voices`.
pub struct SfxGate {
    pub last: HashMap<String, f64>,
    pub by_name: HashMap<String, Vec<VoiceRec>>,
    pub voices: Vec<VoiceRec>,
    pub rng: Rng,
    pub dropped: u64,
    pub stolen: u64,
    pub played: u64,
}

impl Default for SfxGate {
    fn default() -> Self {
        Self {
            last: HashMap::new(),
            by_name: HashMap::new(),
            voices: Vec::new(),
            rng: Rng::new(0),
            dropped: 0,
            stolen: 0,
            played: 0,
        }
    }
}

impl SfxGate {
    pub fn new(seed: u32) -> Self {
        Self {
            rng: Rng::new(seed),
            ..Default::default()
        }
    }

    /// ≡ `play(name, {at, delay, volume, pitch})` at engine time `now`.
    /// Returns `Some(decision)`; the caller spawns with
    /// `speed = pitch·jitter`, `volume`, and despawns the `steal` oldest
    /// voices of that name (deviation: immediate instead of 25 ms fade).
    pub fn play(
        &mut self,
        name: &str,
        now: f64,
        at: Option<f64>,
        delay: f64,
        volume: f64,
        pitch: f64,
        baked_dur: f64,
    ) -> Option<PlayDecision> {
        let m = meta(name).unwrap_or_default();
        let t = now.max(at.unwrap_or(now)) + delay;
        // minGap merge (L266-267)
        if let Some(&lt) = self.last.get(name)
            && t >= lt
            && t - lt < m.min_gap
        {
            self.dropped += 1;
            return Some(PlayDecision::Merged);
        }
        let vol = volume.max(0.0) * m.gain;
        if vol < 1e-4 {
            return Some(PlayDecision::Silent);
        }
        self.last.insert(name.to_string(), t);
        // prune finished voices (≡ v.dead check)
        if let Some(list) = self.by_name.get_mut(name) {
            list.retain(|v| v.end > now);
        }
        let mut steal = 0usize;
        let mut stolen_recs: Vec<VoiceRec> = Vec::new();
        if let Some(list) = self.by_name.get_mut(name) {
            while list.len() >= m.max {
                stolen_recs.push(list.remove(0));
                steal += 1;
            }
        }
        self.stolen += steal as u64;
        // ≡ JS `_steal` + the L277 global prune: a per-name steal takes the
        // voice out of the global list immediately, not only at its `end`
        // (review P2-1 — otherwise near MAX_VOICES the Rust gate steals extra
        // global voices the JS engine has already freed).
        for rec in stolen_recs {
            if let Some(pos) = self.voices.iter().position(|v| *v == rec) {
                self.voices.remove(pos);
            }
        }
        self.voices.retain(|v| v.end > now);
        while self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
            self.stolen += 1;
        }
        // pitch jitter (L280)
        let p = pitch.max(0.05) * (1.0 + (self.rng.next_f64() * 2.0 - 1.0) * m.jitter);
        let speed = p.max(0.05);
        let rec = VoiceRec {
            born: t,
            end: t + baked_dur / speed,
        };
        self.by_name.entry(name.to_string()).or_default().push(rec);
        self.voices.push(rec);
        self.played += 1;
        Some(PlayDecision::Spawned {
            speed,
            volume: vol,
            steal,
        })
    }
}

// ---------------------------------------------------------------------------
// Definitions below (translated chunk-by-chunk from audio.js)
// ---------------------------------------------------------------------------

pub fn meta(name: &str) -> Option<SfxMeta> {
    META.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
}

pub fn build_one_shot(name: &str, v: &mut Vb, p: f64) {
    match name {
        "ui_hover" => b_ui_hover(v, p),
        "ui_click" => b_ui_click(v, p),
        "ui_confirm" => b_ui_confirm(v, p),
        "ui_back" => b_ui_back(v, p),
        "ui_toggle" => b_ui_toggle(v, p),
        "ui_slider" => b_ui_slider(v, p),
        "ui_error" => b_ui_error(v, p),
        "shoot_shooter" => b_shoot_shooter(v, p),
        "shoot_blaster" => b_shoot_blaster(v, p),
        "blaster_pump" => b_blaster_pump(v, p),
        "blaster_boom" => b_blaster_boom(v, p),
        "charger_full" => b_charger_full(v, p),
        "shoot_charger" => b_shoot_charger(v, p),
        "roller_flick" => b_roller_flick(v, p),
        "shoot_dualies" => b_shoot_dualies(v, p),
        "dualies_roll" => b_dualies_roll(v, p),
        "slosh_throw" => b_slosh_throw(v, p),
        "slosh_land" => b_slosh_land(v, p),
        "splatling_ready" => b_splatling_ready(v, p),
        "shoot_splatling" => b_shoot_splatling(v, p),
        "splatling_wind" => b_splatling_wind(v, p),
        "splat_small" => b_splat_small(v, p),
        "splat_big" => b_splat_big(v, p),
        "ink_hit_wall" => b_ink_hit_wall(v, p),
        "bomb_throw" => b_bomb_throw(v, p),
        "bomb_beep" => b_bomb_beep(v, p),
        "bomb_explode" => b_bomb_explode(v, p),
        "squid_in" => b_squid_in(v, p),
        "squid_out" => b_squid_out(v, p),
        "swim_splash" => b_swim_splash(v, p),
        "jump" => b_jump(v, p),
        "land" => b_land(v, p),
        "hit_marker" => b_hit_marker(v, p),
        "ink_hit_body" => b_ink_hit_body(v, p),
        "hurt" => b_hurt(v, p),
        "splat_enemy" => b_splat_enemy(v, p),
        "splatted_self" => b_splatted_self(v, p),
        "ally_splatted" => b_ally_splatted(v, p),
        "low_ink" => b_low_ink(v, p),
        "empty_click" => b_empty_click(v, p),
        "refill_full" => b_refill_full(v, p),
        "special_ready" => b_special_ready(v, p),
        "special_activate" => b_special_activate(v, p),
        "special_slam" => b_special_slam(v, p),
        "storm_thunder" => b_storm_thunder(v, p),
        "respawn" => b_respawn(v, p),
        "super_jump" => b_super_jump(v, p),
        "special_end" => b_special_end(v, p),
        "special_ending" => b_special_ending(v, p),
        "shield_up" => b_shield_up(v, p),
        "shield_hit" => b_shield_hit(v, p),
        "shield_pop" => b_shield_pop(v, p),
        "sonar_ping" => b_sonar_ping(v, p),
        "sonar_mark" => b_sonar_mark(v, p),
        "strike_launch" => b_strike_launch(v, p),
        "strike_whistle" => b_strike_whistle(v, p),
        "strike_impact" => b_strike_impact(v, p),
        "zooka_fire" => b_zooka_fire(v, p),
        "wail_charge" => b_wail_charge(v, p),
        "kraken_on" => b_kraken_on(v, p),
        "kraken_jump" => b_kraken_jump(v, p),
        "kraken_slam" => b_kraken_slam(v, p),
        "bubble_release" => b_bubble_release(v, p),
        "bubble_pop" => b_bubble_pop(v, p),
        "bubble_blast" => b_bubble_blast(v, p),
        "jet_fire" => b_jet_fire(v, p),
        "jet_end" => b_jet_end(v, p),
        "stamp_swing" => b_stamp_swing(v, p),
        "stamp_slam" => b_stamp_slam(v, p),
        "stamp_throw" => b_stamp_throw(v, p),
        "booyah_cheer" => b_booyah_cheer(v, p),
        "booyah_throw" => b_booyah_throw(v, p),
        "booyah_blast" => b_booyah_blast(v, p),
        "zip_fire" => b_zip_fire(v, p),
        "zip_latch" => b_zip_latch(v, p),
        "zip_pull" => b_zip_pull(v, p),
        "crab_gatling" => b_crab_gatling(v, p),
        "crab_cannon" => b_crab_cannon(v, p),
        "crab_hit" => b_crab_hit(v, p),
        "crab_break" => b_crab_break(v, p),
        "ready" => b_ready(v, p),
        "go_horn" => b_go_horn(v, p),
        "countdown_tick" => b_countdown_tick(v, p),
        "final_count" => b_final_count(v, p),
        "one_minute" => b_one_minute(v, p),
        "times_up" => b_times_up(v, p),
        "zone_ours" => b_zone_ours(v, p),
        "zone_theirs" => b_zone_theirs(v, p),
        "zone_lost" => b_zone_lost(v, p),
        "zone_broken" => b_zone_broken(v, p),
        "zone_warn" => b_zone_warn(v, p),
        "zone_chance" => b_zone_chance(v, p),
        "zone_flood" => b_zone_flood(v, p),
        "zone_wipe" => b_zone_wipe(v, p),
        "zone_final" => b_zone_final(v, p),
        "zone_penalty" => b_zone_penalty(v, p),
        "zone_shift" => b_zone_shift(v, p),
        "zone_overtime" => b_zone_overtime(v, p),
        "zone_tick" => b_zone_tick(v, p),
        "judge_drumroll" => b_judge_drumroll(v, p),
        "judge_reveal" => b_judge_reveal(v, p),
        "victory_fanfare" => b_victory_fanfare(v, p),
        "defeat_jingle" => b_defeat_jingle(v, p),
        "xp_tick" => b_xp_tick(v, p),
        "level_up" => b_level_up(v, p),
        "step_dry" => b_step_dry(v, p),
        "step_ink" => b_step_ink(v, p),
        "step_enemy" => b_step_enemy(v, p),
        "ink_drip" => b_ink_drip(v, p),
        "gull" => b_gull(v, p),
        "ferry_horn" => b_ferry_horn(v, p),
        "halyard_clink" => b_halyard_clink(v, p),
        "boss_roar" => b_boss_roar(v, p),
        "boss_step" => b_boss_step(v, p),
        "boss_slam" => b_boss_slam(v, p),
        "boss_tele" => b_boss_tele(v, p),
        "boss_whistle" => b_boss_whistle(v, p),
        "boss_barrel" => b_boss_barrel(v, p),
        "boss_cannon_charge" => b_boss_cannon_charge(v, p),
        "boss_crash" => b_boss_crash(v, p),
        "crablet_chitter" => b_crablet_chitter(v, p),
        "crablet_pop" => b_crablet_pop(v, p),
        "boss_hit" => b_boss_hit(v, p),
        "boss_crit" => b_boss_crit(v, p),
        "boss_phase" => b_boss_phase(v, p),
        "boss_title" => b_boss_title(v, p),
        "boss_defeat" => b_boss_defeat(v, p),
        "boss_sunk" => b_boss_sunk(v, p),
        _ => {
            debug_assert!(
                meta(name).is_none_or(|m| m.is_loop),
                "unknown one-shot {name}"
            );
        }
    }
}

pub fn build_loop(name: &str, v: &mut Vb, p: f64) {
    match name {
        "charger_charge" => l_charger_charge(v, p),
        "roll" => l_roll(v, p),
        "splatling_spin" => l_splatling_spin(v, p),
        "swim" => l_swim(v, p),
        "climb" => l_climb(v, p),
        "enemy_ink_sizzle" => l_enemy_ink_sizzle(v, p),
        "storm_rain" => l_storm_rain(v, p),
        "tornado" => l_tornado(v, p),
        "wail_blast" => l_wail_blast(v, p),
        "blower_inflate" => l_blower_inflate(v, p),
        "jet_loop" => l_jet_loop(v, p),
        "booyah_charge" => l_booyah_charge(v, p),
        "crab_move" => l_crab_move(v, p),
        "crab_roll" => l_crab_roll(v, p),
        "zone_hum" => l_zone_hum(v, p),
        "harbor_ambience" => l_harbor_ambience(v, p),
        "boss_cannon_sweep" => l_boss_cannon_sweep(v, p),
        "boss_gallop" => l_boss_gallop(v, p),
        "boss_dizzy" => l_boss_dizzy(v, p),
        "boss_frenzy" => l_boss_frenzy(v, p),
        _ => {
            debug_assert!(false, "unknown loop {name}");
        }
    }
}

pub static META: &[(&str, SfxMeta)] = &[
    (
        "ui_hover",
        SfxMeta {
            gain: 0.14,
            max: 3,
            jitter: 0.02,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_click",
        SfxMeta {
            gain: 0.26,
            max: 3,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_confirm",
        SfxMeta {
            gain: 0.2,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_back",
        SfxMeta {
            gain: 0.19,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_toggle",
        SfxMeta {
            gain: 0.32,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_slider",
        SfxMeta {
            gain: 0.22,
            max: 3,
            jitter: 0.0,
            min_gap: 0.025,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ui_error",
        SfxMeta {
            gain: 0.13,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shoot_shooter",
        SfxMeta {
            gain: 0.72,
            max: 8,
            jitter: 0.05,
            min_gap: 0.02,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shoot_blaster",
        SfxMeta {
            gain: 0.55,
            max: 4,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "blaster_pump",
        SfxMeta {
            gain: 0.62,
            max: 3,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "blaster_boom",
        SfxMeta {
            gain: 0.55,
            max: 4,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "charger_charge",
        SfxMeta {
            gain: 0.28,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.0),
            is_loop: true,
        },
    ),
    (
        "charger_full",
        SfxMeta {
            gain: 0.24,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shoot_charger",
        SfxMeta {
            gain: 0.55,
            max: 4,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "roller_flick",
        SfxMeta {
            gain: 0.6,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "roll",
        SfxMeta {
            gain: 0.24,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.2),
            is_loop: true,
        },
    ),
    (
        "shoot_dualies",
        SfxMeta {
            gain: 0.8,
            max: 10,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "dualies_roll",
        SfxMeta {
            gain: 0.55,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "slosh_throw",
        SfxMeta {
            gain: 0.6,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "slosh_land",
        SfxMeta {
            gain: 0.58,
            max: 4,
            jitter: 0.06,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splatling_spin",
        SfxMeta {
            gain: 0.21,
            max: 3,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.2),
            is_loop: true,
        },
    ),
    (
        "splatling_ready",
        SfxMeta {
            gain: 0.3,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shoot_splatling",
        SfxMeta {
            gain: 1.0,
            max: 10,
            jitter: 0.05,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splatling_wind",
        SfxMeta {
            gain: 0.34,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splat_small",
        SfxMeta {
            gain: 0.42,
            max: 8,
            jitter: 0.07,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splat_big",
        SfxMeta {
            gain: 0.55,
            max: 4,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ink_hit_wall",
        SfxMeta {
            gain: 1.0,
            max: 6,
            jitter: 0.07,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "bomb_throw",
        SfxMeta {
            gain: 1.0,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "bomb_beep",
        SfxMeta {
            gain: 0.2,
            max: 3,
            jitter: 0.0,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "bomb_explode",
        SfxMeta {
            gain: 0.6,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "squid_in",
        SfxMeta {
            gain: 0.4,
            max: 3,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "squid_out",
        SfxMeta {
            gain: 0.48,
            max: 3,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "swim",
        SfxMeta {
            gain: 0.18,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "climb",
        SfxMeta {
            gain: 0.16,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "swim_splash",
        SfxMeta {
            gain: 0.8,
            max: 3,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "jump",
        SfxMeta {
            gain: 0.48,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "land",
        SfxMeta {
            gain: 0.4,
            max: 3,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "hit_marker",
        SfxMeta {
            gain: 0.48,
            max: 4,
            jitter: 0.0,
            min_gap: 0.035,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ink_hit_body",
        SfxMeta {
            gain: 0.5,
            max: 5,
            jitter: 0.07,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "hurt",
        SfxMeta {
            gain: 0.5,
            max: 3,
            jitter: 0.06,
            min_gap: 0.06,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splat_enemy",
        SfxMeta {
            gain: 0.45,
            max: 3,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "splatted_self",
        SfxMeta {
            gain: 0.55,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ally_splatted",
        SfxMeta {
            gain: 0.34,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "enemy_ink_sizzle",
        SfxMeta {
            gain: 0.26,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.2),
            is_loop: true,
        },
    ),
    (
        "low_ink",
        SfxMeta {
            gain: 0.38,
            max: 1,
            jitter: 0.0,
            min_gap: 0.4,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "empty_click",
        SfxMeta {
            gain: 0.6,
            max: 2,
            jitter: 0.03,
            min_gap: 0.08,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "refill_full",
        SfxMeta {
            gain: 0.26,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "special_ready",
        SfxMeta {
            gain: 0.35,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "special_activate",
        SfxMeta {
            gain: 0.45,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "special_slam",
        SfxMeta {
            gain: 0.48,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "storm_rain",
        SfxMeta {
            gain: 0.22,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.0),
            is_loop: true,
        },
    ),
    (
        "storm_thunder",
        SfxMeta {
            gain: 0.6,
            max: 2,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "respawn",
        SfxMeta {
            gain: 0.35,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "super_jump",
        SfxMeta {
            gain: 0.42,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "special_end",
        SfxMeta {
            gain: 0.43,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "special_ending",
        SfxMeta {
            gain: 0.34,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shield_up",
        SfxMeta {
            gain: 0.37,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shield_hit",
        SfxMeta {
            gain: 0.37,
            max: 4,
            jitter: 0.08,
            min_gap: 0.04,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "shield_pop",
        SfxMeta {
            gain: 0.53,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "sonar_ping",
        SfxMeta {
            gain: 0.35,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "sonar_mark",
        SfxMeta {
            gain: 0.235,
            max: 1,
            jitter: 0.0,
            min_gap: 0.3,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "strike_launch",
        SfxMeta {
            gain: 0.55,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "strike_whistle",
        SfxMeta {
            gain: 0.134,
            max: 3,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "strike_impact",
        SfxMeta {
            gain: 0.445,
            max: 3,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "tornado",
        SfxMeta {
            gain: 0.32,
            max: 3,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.0),
            is_loop: true,
        },
    ),
    (
        "zooka_fire",
        SfxMeta {
            gain: 0.55,
            max: 3,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "wail_charge",
        SfxMeta {
            gain: 0.22,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "wail_blast",
        SfxMeta {
            gain: 0.16,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.0),
            is_loop: true,
        },
    ),
    (
        "kraken_on",
        SfxMeta {
            gain: 0.38,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "kraken_jump",
        SfxMeta {
            gain: 0.45,
            max: 2,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "kraken_slam",
        SfxMeta {
            gain: 0.45,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "blower_inflate",
        SfxMeta {
            gain: 0.24,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "bubble_release",
        SfxMeta {
            gain: 0.5,
            max: 3,
            jitter: 0.08,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "bubble_pop",
        SfxMeta {
            gain: 0.64,
            max: 4,
            jitter: 0.1,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "bubble_blast",
        SfxMeta {
            gain: 0.42,
            max: 3,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "jet_loop",
        SfxMeta {
            gain: 0.25,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "jet_fire",
        SfxMeta {
            gain: 0.485,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "jet_end",
        SfxMeta {
            gain: 0.6,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "stamp_swing",
        SfxMeta {
            gain: 0.47,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "stamp_slam",
        SfxMeta {
            gain: 0.63,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "stamp_throw",
        SfxMeta {
            gain: 1.0,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "booyah_charge",
        SfxMeta {
            gain: 0.25,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "booyah_cheer",
        SfxMeta {
            gain: 0.27,
            max: 3,
            jitter: 0.04,
            min_gap: 0.06,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "booyah_throw",
        SfxMeta {
            gain: 0.55,
            max: 2,
            jitter: 0.03,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "booyah_blast",
        SfxMeta {
            gain: 0.4,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zip_fire",
        SfxMeta {
            gain: 0.58,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zip_latch",
        SfxMeta {
            gain: 0.48,
            max: 3,
            jitter: 0.05,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zip_pull",
        SfxMeta {
            gain: 0.55,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "crab_move",
        SfxMeta {
            gain: 0.39,
            max: 3,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "crab_gatling",
        SfxMeta {
            gain: 0.51,
            max: 4,
            jitter: 0.08,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "crab_cannon",
        SfxMeta {
            gain: 0.55,
            max: 3,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "crab_roll",
        SfxMeta {
            gain: 0.27,
            max: 3,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.5),
            is_loop: true,
        },
    ),
    (
        "crab_hit",
        SfxMeta {
            gain: 0.38,
            max: 4,
            jitter: 0.1,
            min_gap: 0.04,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "crab_break",
        SfxMeta {
            gain: 0.45,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ready",
        SfxMeta {
            gain: 0.4,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "go_horn",
        SfxMeta {
            gain: 0.5,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "countdown_tick",
        SfxMeta {
            gain: 0.3,
            max: 2,
            jitter: 0.0,
            min_gap: 0.2,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "final_count",
        SfxMeta {
            gain: 0.38,
            max: 2,
            jitter: 0.0,
            min_gap: 0.2,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "one_minute",
        SfxMeta {
            gain: 0.36,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "times_up",
        SfxMeta {
            gain: 0.27,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_ours",
        SfxMeta {
            gain: 0.33,
            max: 1,
            jitter: 0.0,
            min_gap: 0.25,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_theirs",
        SfxMeta {
            gain: 0.33,
            max: 1,
            jitter: 0.0,
            min_gap: 0.25,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_lost",
        SfxMeta {
            gain: 0.58,
            max: 1,
            jitter: 0.0,
            min_gap: 0.25,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_broken",
        SfxMeta {
            gain: 0.58,
            max: 1,
            jitter: 0.0,
            min_gap: 0.25,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_warn",
        SfxMeta {
            gain: 0.42,
            max: 1,
            jitter: 0.0,
            min_gap: 1.5,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_chance",
        SfxMeta {
            gain: 0.55,
            max: 1,
            jitter: 0.0,
            min_gap: 1.5,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_flood",
        SfxMeta {
            gain: 0.7,
            max: 3,
            jitter: 0.03,
            min_gap: 0.1,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_wipe",
        SfxMeta {
            gain: 0.7,
            max: 3,
            jitter: 0.03,
            min_gap: 0.1,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_final",
        SfxMeta {
            gain: 0.4,
            max: 1,
            jitter: 0.0,
            min_gap: 1.0,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_hum",
        SfxMeta {
            gain: 0.14,
            max: 4,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.5),
            is_loop: true,
        },
    ),
    (
        "zone_penalty",
        SfxMeta {
            gain: 0.3,
            max: 2,
            jitter: 0.0,
            min_gap: 0.2,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_shift",
        SfxMeta {
            gain: 0.38,
            max: 1,
            jitter: 0.0,
            min_gap: 0.4,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_overtime",
        SfxMeta {
            gain: 0.32,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "zone_tick",
        SfxMeta {
            gain: 0.2,
            max: 2,
            jitter: 0.0,
            min_gap: 0.12,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "judge_drumroll",
        SfxMeta {
            gain: 0.42,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "judge_reveal",
        SfxMeta {
            gain: 0.5,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "victory_fanfare",
        SfxMeta {
            gain: 0.28,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "defeat_jingle",
        SfxMeta {
            gain: 0.3,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "xp_tick",
        SfxMeta {
            gain: 0.4,
            max: 4,
            jitter: 0.01,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "level_up",
        SfxMeta {
            gain: 0.4,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "step_dry",
        SfxMeta {
            gain: 0.16,
            max: 6,
            jitter: 0.08,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "step_ink",
        SfxMeta {
            gain: 0.2,
            max: 6,
            jitter: 0.08,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "step_enemy",
        SfxMeta {
            gain: 0.2,
            max: 6,
            jitter: 0.06,
            min_gap: 0.06,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ink_drip",
        SfxMeta {
            gain: 0.18,
            max: 4,
            jitter: 0.12,
            min_gap: 0.04,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "gull",
        SfxMeta {
            gain: 0.2,
            max: 2,
            jitter: 0.1,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "ferry_horn",
        SfxMeta {
            gain: 0.3,
            max: 1,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "halyard_clink",
        SfxMeta {
            gain: 0.07,
            max: 6,
            jitter: 0.12,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "harbor_ambience",
        SfxMeta {
            gain: 0.12,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(3.0),
            is_loop: true,
        },
    ),
    (
        "boss_roar",
        SfxMeta {
            gain: 0.37,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_step",
        SfxMeta {
            gain: 0.62,
            max: 6,
            jitter: 0.08,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_slam",
        SfxMeta {
            gain: 0.4,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_tele",
        SfxMeta {
            gain: 0.34,
            max: 2,
            jitter: 0.02,
            min_gap: 0.25,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_whistle",
        SfxMeta {
            gain: 0.21,
            max: 6,
            jitter: 0.06,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_barrel",
        SfxMeta {
            gain: 0.6,
            max: 6,
            jitter: 0.07,
            min_gap: 0.04,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_cannon_charge",
        SfxMeta {
            gain: 0.23,
            max: 2,
            jitter: 0.02,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_cannon_sweep",
        SfxMeta {
            gain: 0.27,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.4),
            is_loop: true,
        },
    ),
    (
        "boss_gallop",
        SfxMeta {
            gain: 0.42,
            max: 2,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(1.8),
            is_loop: true,
        },
    ),
    (
        "boss_crash",
        SfxMeta {
            gain: 0.49,
            max: 2,
            jitter: 0.04,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_dizzy",
        SfxMeta {
            gain: 0.1,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(2.5),
            is_loop: true,
        },
    ),
    (
        "boss_frenzy",
        SfxMeta {
            gain: 0.26,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: Some(3.0),
            is_loop: true,
        },
    ),
    (
        "crablet_chitter",
        SfxMeta {
            gain: 1.0,
            max: 4,
            jitter: 0.1,
            min_gap: 0.08,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "crablet_pop",
        SfxMeta {
            gain: 0.45,
            max: 5,
            jitter: 0.08,
            min_gap: 0.03,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_hit",
        SfxMeta {
            gain: 0.56,
            max: 4,
            jitter: 0.06,
            min_gap: 0.05,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_crit",
        SfxMeta {
            gain: 0.46,
            max: 3,
            jitter: 0.03,
            min_gap: 0.06,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_phase",
        SfxMeta {
            gain: 0.42,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_title",
        SfxMeta {
            gain: 0.31,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_defeat",
        SfxMeta {
            gain: 0.36,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
    (
        "boss_sunk",
        SfxMeta {
            gain: 0.3,
            max: 1,
            jitter: 0.0,
            min_gap: 0.018,
            one_shot: None,
            is_loop: false,
        },
    ),
];

// ---------------------------------------------------------------------------
// Tests (batch B)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The 20 `loop(v, p)` definitions of audio.js (every other entry is a
    /// `build(v, p)` one-shot).
    const LOOP_NAMES: &[&str] = &[
        "charger_charge",
        "roll",
        "splatling_spin",
        "swim",
        "climb",
        "enemy_ink_sizzle",
        "storm_rain",
        "tornado",
        "wail_blast",
        "blower_inflate",
        "jet_loop",
        "booyah_charge",
        "crab_move",
        "crab_roll",
        "zone_hum",
        "harbor_ambience",
        "boss_cannon_sweep",
        "boss_gallop",
        "boss_dizzy",
        "boss_frenzy",
    ];

    fn peak(p: &[f32]) -> f32 {
        p.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn table_is_complete_and_unique() {
        assert_eq!(SFX_GROUPS.len(), 11);
        let names = sfx_names();
        assert_eq!(names.len(), 148);
        let uniq: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(uniq.len(), 148, "duplicate name across groups");
        assert_eq!(META.len(), 148);
        for n in &names {
            assert!(meta(n).is_some(), "META missing {n}");
        }
        for (n, _) in META {
            assert!(uniq.contains(n), "META has unknown key {n}");
        }
        assert_eq!(LOOP_NAMES.len(), 20);
        for n in &names {
            assert_eq!(
                meta(n).unwrap().is_loop,
                LOOP_NAMES.contains(n),
                "is_loop mismatch for {n}"
            );
        }
    }

    #[test]
    fn loop_entries_have_no_one_shot_arm() {
        for &n in LOOP_NAMES {
            let mut v = Vb::new(0.0, Rng::new(bake_seed(n)));
            build_one_shot(n, &mut v, 1.0);
            assert!(v.bake().is_empty(), "{n} must be loop-only");
        }
    }

    #[test]
    fn smoke_bake_every_name() {
        for n in sfx_names() {
            let Some(p) = bake(n) else {
                panic!("{n}: table name must bake");
            };
            assert!(!p.is_empty(), "{n}: empty bake");
            assert!(p.len() < (6.0 * SR) as usize, "{n}: {} samples", p.len());
            assert!(p.iter().all(|s| s.is_finite()), "{n}: non-finite sample");
            assert!(peak(&p) > 1e-4, "{n}: silent bake");
        }
    }

    #[test]
    fn bake_is_deterministic() {
        for n in ["shoot_shooter", "zone_hum", "boss_roar", "final_count"] {
            assert_eq!(bake(n), bake(n), "{n}");
        }
    }

    /// One value pin per group: exact bake length + four fixed sample taps
    /// (deterministic through the per-name `bake_seed`).
    #[test]
    fn group_value_goldens() {
        // (name, len, s[0], s[44], s[2205], s[len-1])
        let goldens: &[(&str, usize, f32, f32, f32, f32)] = &[
            ("ui_hover", 3088, 0.0, -0.33976695, -0.00047562912, 0.0),
            ("shoot_shooter", 5381, 0.0, 1.0, 0.00218979, 0.0),
            ("splat_big", 16450, 0.0, 0.6320425, 0.26857555, 0.0),
            ("squid_in", 6541, 0.0, -0.25719064, 0.087757334, 0.0),
            ("gull", 24608, 0.0, 0.009983776, -0.017350545, 1.4889723e-20),
            ("hit_marker", 3035, 0.0, -0.80788213, 0.0012542694, 0.0),
            ("empty_click", 5160, 0.0, 0.6242264, -0.005336547, -0.0),
            ("sonar_ping", 89127, 0.0, 0.036782358, -0.22369713, 0.0),
            ("final_count", 7894, 0.0, 0.60621357, -0.38726825, 0.0),
            ("zone_hum", 110691, -0.0, -0.0053012543, 0.06423784, -0.0),
            ("boss_roar", 85995, 0.0, -0.0010293932, 0.790972, 0.0),
        ];
        for (name, len, s0, s44, s2205, last) in goldens {
            let p = bake(name).unwrap();
            assert_eq!(p.len(), *len, "{name}: bake length");
            let tap = |i: usize| p[i.min(p.len() - 1)];
            assert_eq!(tap(0), *s0, "{name}: s[0]");
            assert_eq!(tap(44), *s44, "{name}: s[44]");
            assert_eq!(tap(2205), *s2205, "{name}: s[2205]");
            assert_eq!(tap(p.len() - 1), *last, "{name}: last sample");
        }
    }

    #[test]
    fn gate_min_gap_merges_identical_plays() {
        let mut g = SfxGate::new(1);
        let m = meta("ui_hover").unwrap(); // min_gap 0.03
        assert!(matches!(
            g.play("ui_hover", 0.0, None, 0.0, 1.0, 1.0, 0.05),
            Some(PlayDecision::Spawned { .. })
        ));
        assert_eq!(
            g.play("ui_hover", m.min_gap * 0.5, None, 0.0, 1.0, 1.0, 0.05),
            Some(PlayDecision::Merged)
        );
        assert_eq!(g.dropped, 1);
        // Once the gap elapses the next play is heard again.
        assert!(matches!(
            g.play("ui_hover", m.min_gap + 0.01, None, 0.0, 1.0, 1.0, 0.05),
            Some(PlayDecision::Spawned { .. })
        ));
    }

    #[test]
    fn gate_zero_volume_is_silent() {
        let mut g = SfxGate::new(1);
        assert_eq!(
            g.play("ui_click", 0.0, None, 0.0, 0.0, 1.0, 0.05),
            Some(PlayDecision::Silent)
        );
        assert_eq!(g.played, 0);
    }

    #[test]
    fn gate_jitter_zero_speed_is_exact_pitch() {
        let mut g = SfxGate::new(1);
        // ui_confirm: jitter 0.0 ⇒ no random draw shifts the rate.
        let Some(PlayDecision::Spawned {
            speed,
            volume,
            steal,
        }) = g.play("ui_confirm", 0.0, None, 0.0, 2.0, 1.5, 0.05)
        else {
            panic!("expected spawn");
        };
        assert_eq!(speed, 1.5);
        assert_eq!(volume, 2.0 * meta("ui_confirm").unwrap().gain);
        assert_eq!(steal, 0);
    }

    #[test]
    fn gate_jitter_bounds() {
        let mut g = SfxGate::new(1);
        // ink_drip: jitter 0.12 ⇒ speed ∈ pitch·[0.88, 1.12].
        for _ in 0..32 {
            let mut g2 = SfxGate::new(g.played as u32 + 7);
            let Some(PlayDecision::Spawned { speed, .. }) =
                g2.play("ink_drip", 0.0, None, 0.0, 1.0, 1.0, 0.05)
            else {
                panic!("expected spawn");
            };
            assert!((0.88..=1.12).contains(&speed), "speed {speed}");
            g.played += 1;
        }
    }

    #[test]
    fn gate_per_name_cap_steals_oldest() {
        let mut g = SfxGate::new(1);
        // ferry_horn: max 1; 100 s voices never prune, so the second play
        // must steal the first.
        assert!(matches!(
            g.play("ferry_horn", 0.0, None, 0.0, 1.0, 1.0, 100.0),
            Some(PlayDecision::Spawned { steal: 0, .. })
        ));
        let Some(PlayDecision::Spawned { steal, .. }) =
            g.play("ferry_horn", 1.0, None, 0.0, 1.0, 1.0, 100.0)
        else {
            panic!("expected spawn");
        };
        assert_eq!(steal, 1);
        assert_eq!(g.stolen, 1);
        let v = &g.by_name["ferry_horn"];
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].born, 1.0, "the surviving voice is the newest");
        // Review P2-1: the stolen voice leaves the *global* list immediately
        // too (≡ JS `_steal` marking + the L277 prune), not at its `end`.
        assert_eq!(g.voices.len(), 1, "per-name steal prunes the global list");
        assert_eq!(g.voices[0].born, 1.0);
    }

    #[test]
    fn gate_global_voice_cap_evicts_oldest() {
        let mut g = SfxGate::new(1);
        let mut played = 0usize;
        for n in sfx_names() {
            if meta(n).unwrap().is_loop {
                continue;
            }
            g.play(n, 0.0, None, 0.0, 1.0, 1.0, 1000.0);
            played += 1;
            if played > MAX_VOICES {
                break;
            }
        }
        assert_eq!(played, MAX_VOICES + 1);
        assert_eq!(g.voices.len(), MAX_VOICES);
        assert_eq!(g.stolen, 1, "exactly one global eviction");
    }

    #[test]
    fn gate_at_and_delay_schedule_the_birth_time() {
        let mut g = SfxGate::new(1);
        // ui_confirm: jitter 0.0 ⇒ speed = pitch = 1.0 exactly.
        g.play("ui_confirm", 0.0, Some(5.0), 0.25, 1.0, 1.0, 0.05);
        let v = &g.by_name["ui_confirm"];
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].born, 5.25);
        assert_eq!(v[0].end, 5.30);
    }
}
