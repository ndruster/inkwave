//! Task 14 — minimal placeholder SFX (procedural PCM, no asset files).
//!
//! Upstream `src/audio/audio.js` is a full WebAudio synth engine; the M1 port
//! ships short procedurally-synthesised beeps/clicks for the same trigger set
//! (main.js L499-519 / L549 / L971+L1027, weapons.js L162 `_empty`; the sub
//! `low_ink` nag of weapons.js L138 has no M1 sub weapon, so it is not
//! ported):
//!   - shot            — local player fires (`Actor::last_fire` reset edge)
//!   - splat kill/self/ally — `MatchEvent::Splat` credit cases
//!   - empty click     — trigger held below `ink_per_shot` (JS `_empty`, 0.45 s
//!     cooldown mirrors `EMPTY_CD`)
//!   - countdown / 60 s tick — `MatchEvent::Countdown` / `OneMinute`
//!   - finish whistle / victory / defeat — `MatchEvent::Phase(Finish|End)`
//!
//! PCM is synthesised in `render()` (44.1 kHz mono, clamped to [-1, 1]) and
//! fed to Bevy through a custom [`Decodable`] asset ([`PcmSound`]) — the
//! default `AudioSource` path needs *encoded* bytes and the default feature
//! set has no `wav` decoder, so raw f32 buffers must ride their own source
//! type (`App::add_audio_source::<PcmSound>()`).
//!
//! Silent degradation (TR-14.1, wasm): `bevy_audio`'s `AudioOutput` opens the
//! default device lazily; on failure it logs `No audio device found.` once and
//! `play_queued_audio_system` returns early on every frame — no panic, no
//! error. We add nothing on top of that. The master switch is `M` (mute),
//! applied via `GlobalVolume` *and* by skipping spawns while muted.
//!
//! Event-bus note: `audio_sfx` runs BEFORE `ui_state_machine` in the Update
//! chain and only *reads* `Match::events`; the UI system drains them at the
//! end of the same frame, so every event is heard exactly once.

use std::collections::HashMap;
use std::time::Duration;

use bevy::audio::{
    AddAudioSource, AudioPlayer, ChannelCount, Decodable, GlobalVolume, PlaybackSettings,
    SampleRate, Source as RodioSource, Volume,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use inkwave_sim::boss::BossEvent;
use inkwave_sim::match_::{MatchEvent, MatchResult, Phase};

use crate::ink_render::DemoSim;
use crate::input::PlayerControls;

/// Sample rate of every synthesised cue.
const SR: u32 = 44100;
/// Countdown beeps generated for `n = 1..=COUNT_MAX` (the match countdown
/// runs from `MATCH.final_countdown` = 10 down to 1; out-of-range n clamps).
const COUNT_MAX: i32 = 10;

// ---------------------------------------------------------------------------
// Synthesis (pure, unit-testable)
// ---------------------------------------------------------------------------

/// Mono float PCM block.
#[derive(Clone, Debug)]
pub struct Pcm {
    pub sample_rate: u32,
    pub data: Vec<f32>,
}

/// Render `dur` seconds of mono audio from a sample function `f(t)`.
/// Every sample is clamped to [-1, 1] so the buffers can never clip hard.
fn render(dur: f32, gain: f32, f: impl Fn(f32, usize) -> f32) -> Pcm {
    let n = (dur * SR as f32) as usize;
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR as f32;
        data.push((f(t, i) * gain).clamp(-1.0, 1.0));
    }
    Pcm {
        sample_rate: SR,
        data,
    }
}

/// Deterministic white noise for sample index `i` (splitmix64 finaliser).
fn noise(i: usize, salt: u64) -> f32 {
    let mut x = (i as u64)
        .wrapping_add(salt)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    ((x >> 40) as f32 / 8_388_608.0) * 2.0 - 1.0
}

/// Linear frequency sweep sine (instantaneous freq f0 → f1 over `dur`).
fn sweep(t: f32, f0: f32, f1: f32, dur: f32) -> f32 {
    let k = (f1 - f0) / dur.max(1e-6);
    (std::f32::consts::TAU * (f0 * t + 0.5 * k * t * t)).sin()
}

/// Attack/decay envelope: linear ramp over `a`, exponential tail `d`.
fn env(t: f32, a: f32, d: f32) -> f32 {
    if t < a {
        t / a.max(1e-6)
    } else {
        (-(t - a) / d.max(1e-6)).exp()
    }
}

/// `shoot_shooter` (audio.js L472-487) placeholder: chirp + snap + thump.
pub fn shot_pcm() -> Pcm {
    render(0.14, 0.6, |t, i| {
        let chirp = sweep(t, 2600.0, 820.0, 0.03) * env(t, 0.0005, 0.022) * 0.6;
        let snap = noise(i, 1) * env(t, 0.0004, 0.016) * 0.5;
        let thump = sweep(t, 175.0, 62.0, 0.06) * env(t, 0.001, 0.055) * 0.7;
        let wet = noise(i, 0x5EED) * env(t, 0.004, 0.09) * 0.35;
        chirp + snap + thump + wet
    })
}

/// Wet splat: descending "gloop" + noise tail. One per credit case (main.js
/// L499-519: splat_enemy / splatted_self / ally_splatted).
fn splat_pcm(f0: f32, f1: f32, dur: f32) -> Pcm {
    render(dur, 1.0, |t, i| {
        let blob = sweep(t, f0, f1, dur) * env(t, 0.002, dur * 0.35) * 0.55;
        let splash = noise(i, 7) * env(t, 0.001, dur * 0.22) * 0.4;
        blob + splash
    })
}

pub fn splat_kill_pcm() -> Pcm {
    splat_pcm(900.0, 240.0, 0.25)
}
pub fn splat_self_pcm() -> Pcm {
    splat_pcm(500.0, 120.0, 0.32)
}
pub fn splat_ally_pcm() -> Pcm {
    splat_pcm(700.0, 200.0, 0.2)
}

/// `empty_click` (audio.js weapons L162): two dry high clicks.
pub fn empty_pcm() -> Pcm {
    render(0.06, 0.5, |t, _| {
        let click = |o: f32| {
            if t < o {
                0.0
            } else {
                let x = t - o;
                (std::f32::consts::TAU * 3200.0 * x).sin() * (-x / 0.005).exp()
            }
        };
        click(0.0) + click(0.025)
    })
}

/// Countdown beep (main.js L549 `final_count`): pitch rises as `n` falls.
pub fn count_pcm(n: i32) -> Pcm {
    let n = n.clamp(1, COUNT_MAX);
    let f = 640.0 + (COUNT_MAX - n) as f32 * 120.0;
    render(0.16, 0.5, |t, _| {
        let s = (std::f32::consts::TAU * f * t).sin()
            + 0.2 * (std::f32::consts::TAU * 2.0 * f * t).sin();
        s * env(t, 0.005, 0.08)
    })
}

/// 60-second marker tick (main.js `match:oneminute`).
pub fn tick_pcm() -> Pcm {
    render(0.12, 0.3, |t, _| {
        (std::f32::consts::TAU * 1180.0 * t).sin() * env(t, 0.004, 0.06)
    })
}

/// Finish whistle: two falling notes (JS `final_whistle`-ish).
pub fn finish_pcm() -> Pcm {
    render(0.5, 0.4, |t, _| {
        let note = |o: f32, f: f32| {
            if t < o {
                0.0
            } else {
                (std::f32::consts::TAU * f * (t - o)).sin() * env(t - o, 0.006, 0.16)
            }
        };
        note(0.0, 980.0) + note(0.24, 740.0)
    })
}

fn arpeggio(notes: &[f32], step: f32, decay: f32) -> Pcm {
    let dur = step * notes.len() as f32 + decay;
    let notes: Vec<f32> = notes.to_vec();
    render(dur, 0.45, move |t, _| {
        let idx = (t / step) as usize;
        if idx >= notes.len() {
            return 0.0;
        }
        let x = t - idx as f32 * step;
        (std::f32::consts::TAU * notes[idx] * x).sin() * env(x, 0.006, decay)
    })
}

/// `victory_fanfare` (main.js L971): rising major.
pub fn victory_pcm() -> Pcm {
    arpeggio(&[523.25, 659.25, 783.99, 1046.5], 0.13, 0.2)
}
/// `defeat_jingle` (main.js L1027): falling minor.
pub fn defeat_pcm() -> Pcm {
    arpeggio(&[440.0, 349.23, 261.63], 0.22, 0.3)
}

// ---------------------------------------------------------------------------
// Decodable asset plumbing
// ---------------------------------------------------------------------------

/// PCM block registered as a Bevy asset so `AudioPlayer<PcmSound>` can play it.
#[derive(Asset, TypePath, Debug)]
pub struct PcmSound(pub Pcm);

/// `rodio::Source` over an owned mono f32 buffer (the `decoder()` output).
pub struct PcmIter {
    data: std::vec::IntoIter<f32>,
    sample_rate: SampleRate,
    frames: usize,
}

impl Iterator for PcmIter {
    type Item = bevy::audio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        self.data.next()
    }
}

impl RodioSource for PcmIter {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.data.len())
    }
    fn channels(&self) -> ChannelCount {
        // Monaural by construction; `new(1)` is always valid.
        ChannelCount::new(1).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.frames as f64 / self.sample_rate.get() as f64,
        ))
    }
}

impl Decodable for PcmSound {
    type Decoder = PcmIter;

    fn decoder(&self) -> Self::Decoder {
        PcmIter {
            data: self.0.data.clone().into_iter(),
            sample_rate: SampleRate::new(self.0.sample_rate).unwrap(),
            frames: self.0.data.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Cues and wiring
// ---------------------------------------------------------------------------

/// One synthesised sound effect.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sfx {
    Shot,
    SplatKill,
    SplatSelf,
    SplatAlly,
    Empty,
    Count(i32),
    Tick,
    Finish,
    Victory,
    Defeat,
}

/// Pre-built handles for every cue (inserted by [`build_sfx_assets`]).
#[derive(Resource)]
pub struct SfxAssets(HashMap<Sfx, Handle<PcmSound>>);

impl SfxAssets {
    pub(crate) fn make(assets: &mut Assets<PcmSound>) -> Self {
        let mut map: HashMap<Sfx, Handle<PcmSound>> = HashMap::new();
        let cues: [(Sfx, Pcm); 9] = [
            (Sfx::Shot, shot_pcm()),
            (Sfx::SplatKill, splat_kill_pcm()),
            (Sfx::SplatSelf, splat_self_pcm()),
            (Sfx::SplatAlly, splat_ally_pcm()),
            (Sfx::Empty, empty_pcm()),
            (Sfx::Tick, tick_pcm()),
            (Sfx::Finish, finish_pcm()),
            (Sfx::Victory, victory_pcm()),
            (Sfx::Defeat, defeat_pcm()),
        ];
        for (s, pcm) in cues {
            map.insert(s, assets.add(PcmSound(pcm)));
        }
        for n in 1..=COUNT_MAX {
            map.insert(Sfx::Count(n), assets.add(PcmSound(count_pcm(n))));
        }
        Self(map)
    }

    fn get(&self, s: Sfx) -> Option<&Handle<PcmSound>> {
        let s = match s {
            Sfx::Count(n) => Sfx::Count(n.clamp(1, COUNT_MAX)),
            other => other,
        };
        self.0.get(&s)
    }
}

/// Audio runtime state: master mute switch + per-frame edge trackers.
#[derive(Resource)]
pub struct SoundState {
    pub muted: bool,
    /// `Actor::last_fire` seen at the previous frame; a drop means the local
    /// player fired (the sim's `note_fired` resets it to 0).
    prev_last_fire: f32,
    /// Own `emptyCd` replica (weapons.js L160-162) for the dry-fire click.
    empty_cd: f32,
}

impl Default for SoundState {
    fn default() -> Self {
        Self {
            muted: false,
            // Fresh/reset actors start at last_fire = 99 (actor.rs L234).
            prev_last_fire: 99.0,
            empty_cd: 0.0,
        }
    }
}

/// Marker stamped on every spawned sound entity so [`sfx_gc`] can reap ones
/// the audio backend never got to (no device ⇒ `DESPAWN` cleanup never runs).
#[derive(Component)]
pub(crate) struct SfxBorn(f32);

/// `Actor.last_fire` decreased ⇒ at least one shot was fired since the last
/// observation (multi-step frames collapse to a single cue; under heavy frame
/// drops + full-auto the accumulated value can stay above the previous frame
/// end and miss one cue — acceptable for placeholders).
#[must_use]
pub(crate) fn fired_this_frame(cur: f32, prev: f32) -> bool {
    cur < prev - 1e-6
}

/// Splat credit cases → cue (main.js L499-519): local kill, local death,
/// ally death; enemy deaths caused by bots are silent upstream too.
#[must_use]
pub(crate) fn splat_cue(ev: &MatchEvent, local_team: usize) -> Option<Sfx> {
    let MatchEvent::Splat {
        victim_team,
        victim_slot,
        attacker,
        ..
    } = *ev
    else {
        return None;
    };
    if attacker == Some((0, 0)) {
        Some(Sfx::SplatKill)
    } else if victim_team == local_team && victim_slot == 0 {
        Some(Sfx::SplatSelf)
    } else if victim_team == local_team {
        Some(Sfx::SplatAlly)
    } else {
        None
    }
}

/// Clock/phase events → cue (countdown, 60 s tick, finish, judged result).
#[must_use]
pub(crate) fn phase_cue(
    ev: &MatchEvent,
    result: Option<&MatchResult>,
    local_team: usize,
) -> Option<Sfx> {
    match ev {
        MatchEvent::Phase(Phase::Finish) => Some(Sfx::Finish),
        MatchEvent::Phase(Phase::End) => result.map(|r| {
            if r.winner == local_team {
                Sfx::Victory
            } else {
                Sfx::Defeat
            }
        }),
        MatchEvent::Countdown { n } => Some(Sfx::Count(*n)),
        MatchEvent::OneMinute => Some(Sfx::Tick),
        _ => None,
    }
}

/// `M` toggles the master mute (TR-14.1 "设置可静音"); every other frame
/// collects the cues due and spawns one-shot `AudioPlayer` entities.
///
/// Runs BEFORE `ui_state_machine` (which drains `Match::events`), reading the
/// same event vec immutably — each event is heard exactly once.
#[allow(clippy::too_many_arguments)]
pub fn audio_sfx(
    demo: Res<DemoSim>,
    pc: Res<PlayerControls>,
    mut snd: ResMut<SoundState>,
    assets: Option<Res<SfxAssets>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut gvol: ResMut<GlobalVolume>,
    time: Res<Time>,
    mut commands: Commands,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        snd.muted = !snd.muted;
        gvol.volume = if snd.muted {
            Volume::SILENT
        } else {
            Volume::Linear(1.0)
        };
        println!("[inkwave] audio {}", if snd.muted { "MUTED" } else { "on" });
    }

    let Some(a) = demo.m.actors.first() else {
        return;
    };
    // Edge trackers keep running while paused so resuming never fakes a shot.
    let lf = a.last_fire();
    let shot = fired_this_frame(lf, snd.prev_last_fire);
    snd.prev_last_fire = lf;

    // The click cooldown only ticks while live (upstream the weapon runner
    // does not step outside `playing`); `a.alive` keeps the dry-fire click
    // silent during the death-cam window, as upstream (the runner resets on
    // splat, weapons.js `_empty` never runs while dead).
    if !pc.paused {
        snd.empty_cd = (snd.empty_cd - time.delta_secs()).max(0.0);
    }
    let empty = !pc.paused
        && a.alive
        && demo.m.phase == Phase::Active
        && pc.intent.fire
        && a.ink < demo.tuning.weapons.shooter.ink_per_shot
        && snd.empty_cd <= 0.0;
    if empty {
        snd.empty_cd = inkwave_sim::weapon::EMPTY_CD;
    }

    let live = !pc.paused;
    let mut cues: Vec<Sfx> = Vec::new();
    if live && shot {
        cues.push(Sfx::Shot);
    }
    if live && empty {
        cues.push(Sfx::Empty);
    }
    if live {
        for ev in demo.m.events.iter() {
            if let Some(c) = splat_cue(ev, a.team) {
                cues.push(c);
            }
            if let Some(c) = phase_cue(ev, demo.m.result.as_ref(), a.team) {
                cues.push(c);
            }
        }
    }

    if snd.muted || cues.is_empty() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    let now = time.elapsed_secs();
    for c in cues {
        let Some(h) = assets.get(c) else {
            continue;
        };
        commands.spawn((
            AudioPlayer(h.clone()),
            PlaybackSettings::DESPAWN,
            SfxBorn(now),
        ));
    }
}

/// Reap sound entities that no backend consumed (device-less hosts keep them
/// queued forever; with a device `DESPAWN` fires first and this is a no-op).
pub(crate) fn sfx_gc(time: Res<Time>, mut commands: Commands, q: Query<(Entity, &SfxBorn)>) {
    let now = time.elapsed_secs();
    for (e, born) in q.iter() {
        // Longest cue < 1 s; 3 s is generous slack for real despawns.
        if now - born.0 > 3.0
            && let Ok(mut ent) = commands.get_entity(e)
        {
            ent.despawn();
        }
    }
}

/// Startup: synthesise every cue into the asset store once.
pub fn build_sfx_assets(mut assets: ResMut<Assets<PcmSound>>, mut commands: Commands) {
    let sfx = SfxAssets::make(&mut assets);
    let n = sfx.0.len();
    commands.insert_resource(sfx);
    // TR-14.1 console evidence: cues ready; the backend itself logs
    // "No audio device found." once when the host has no output.
    println!("[inkwave] audio: {n} procedural cues ready (M = mute)");
}

/// Register the PCM source type + resources/systems (called from WorldPlugin;
/// `DefaultPlugins` already provides `AudioPlugin`). `sfx_gc` is added by the
/// caller inside the Update chain (after `audio_sfx`'s spawns apply).
pub fn add_audio(app: &mut App) {
    app.add_audio_source::<PcmSound>()
        .init_resource::<SoundState>()
        .init_resource::<GlobalVolume>()
        .add_systems(Startup, build_sfx_assets);
}

// ---------------------------------------------------------------------------
// Task 46 batch C2 — bossAudio placeholder档 (≡ src/audio/bossAudio.js).
// No real synthesis: every bus subscription becomes a trigger RECORD
// (SFX name + delay + pitch/volume + loop open/close + music track) so the
// unit tests can pin counts / order / the dedupe gates. Wiring a real synth
// onto this table is a later task.
// ---------------------------------------------------------------------------

/// One boss-audio trigger record — the JS `snd(name, opts)` / `_later(sec, fn)`
/// / `_loop` / `_stopLoop` / `music` / `duck` calls (bossAudio.js L27-50).
#[derive(Debug, Clone, PartialEq)]
pub enum BossCue {
    /// JS `snd(name, {delay, pitch, volume})` — `at` is the delay in seconds.
    Sfx {
        name: &'static str,
        at: f32,
        pitch: f32,
        volume: f32,
    },
    /// JS `_loop(key, name, {maxDur})` (L38-42); `at` models a wrapping
    /// `_later(sec, () => _loop(...))` (e.g. the dizzy loop at L127).
    LoopStart {
        key: &'static str,
        name: &'static str,
        at: f32,
        max_dur: f32,
    },
    /// JS `_stopLoop(key)` (L43-47).
    LoopStop { key: &'static str },
    /// JS `_stopAll()` (L48).
    StopAll,
    /// JS `music(track)` — `None` stops the boss set (L9-10 `TRACK`).
    Music(Option<&'static str>),
    /// JS `duck(0.7, 3)` on the defeat (L143).
    Duck { gain: f32, dur: f32 },
}

/// Per-frame live read for the director (JS `boss:move` per-phase events are
/// synthesized from `boss.st` + the `match:state` bus, bossAudio.js L69-80).
#[derive(Debug, Clone, Copy)]
pub struct BossAudioFrame<'a> {
    pub move_id: Option<&'a str>,
    pub move_phase: Option<&'a str>,
    /// `match:state 'playing'` (m.phase == Active).
    pub playing: bool,
    /// `match:state 'finish'/'judge'` (m.phase == End).
    pub ended: bool,
}

/// JS `class BossAudio` state `st` + handlers (bossAudio.js L62-176).
#[derive(Debug, Clone)]
pub struct BossAudioDirector {
    /// JS `st.active` (L62).
    pub active: bool,
    phase: u32,
    dead: bool,
    track: Option<&'static str>,
    /// JS `this._t()` = `performance.now()/1000` — here the director's own
    /// clock; the dedupe gates (0.06 / 0.07 / 0.3 s) run on it.
    t: f64,
    hit_t: f64,
    crit_t: f64,
    prev_move: Option<(String, String)>,
    prev_playing: bool,
    prev_ended: bool,
    /// Deterministic stand-in for `Math.random()` (the placeholder档 needs
    /// reproducible barrage counts / whistle pitches).
    rng: u32,
    /// The trigger log (bounded; consumed by tests now, by the real synth later).
    pub cues: Vec<BossCue>,
}

impl Default for BossAudioDirector {
    fn default() -> Self {
        Self::new()
    }
}

impl BossAudioDirector {
    pub fn new() -> Self {
        Self {
            active: false,
            phase: 1,
            dead: false,
            track: None,
            t: 0.0,
            // JS reads `performance.now()` (always large) so the first hit
            // always passes the gates; a negative start mirrors that.
            hit_t: -9.0,
            crit_t: -9.0,
            prev_move: None,
            prev_playing: false,
            prev_ended: false,
            rng: 0x9E37_79B9,
            cues: Vec::new(),
        }
    }

    /// JS `begin()` (L62-67): a fresh boss match resets the tracker.
    pub fn begin(&mut self) {
        *self = Self {
            active: true,
            ..Self::new()
        };
    }

    fn rand(&mut self) -> f32 {
        // splitmix-ish LCG step → [0, 1).
        self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((self.rng >> 8) as f32) / (1 << 24) as f32
    }

    fn sfx(&mut self, name: &'static str, at: f32, pitch: f32, volume: f32) {
        self.cues.push(BossCue::Sfx {
            name,
            at,
            pitch,
            volume,
        });
    }

    /// JS `music(track)` (L51-55): only records on an actual change.
    fn music(&mut self, track: Option<&'static str>) {
        if self.track != track {
            self.track = track;
            self.cues.push(BossCue::Music(track));
        }
    }

    /// JS `TRACK` (L9): phase 1/2/3 → boss / boss_2 / boss_3.
    fn phase_track(&self) -> &'static str {
        match self.phase {
            2 => "boss_2",
            3 => "boss_3",
            _ => "boss",
        }
    }

    /// Feed one frame: drained boss events + the live read.
    pub fn update(&mut self, dt: f64, events: &[BossEvent], f: &BossAudioFrame) {
        self.t += dt;
        for ev in events {
            self.on_event(ev, f.playing);
        }
        // JS per-phase `boss:move` (synthesized from `boss.st`, as in the HUD).
        let cur = match (f.move_id, f.move_phase) {
            (Some(id), Some(ph)) => Some((id.to_string(), ph.to_string())),
            _ => None,
        };
        if cur != self.prev_move {
            if let Some((id, ph)) = &cur {
                self.move_cue(id, ph);
            }
            self.prev_move = cur;
        }
        // JS `match:state` (L69-76).
        if f.playing && !self.prev_playing {
            let tr = self.phase_track();
            self.music(Some(tr));
        }
        if f.ended && !self.prev_ended {
            // JS 'finish' (L72-73): silence + stop the set. The defeat path
            // already stopped everything in `boss:defeat` (L141).
            if !self.dead {
                self.cues.push(BossCue::StopAll);
                self.music(None);
            }
        }
        self.prev_playing = f.playing;
        self.prev_ended = f.ended;
        if self.cues.len() > 1024 {
            let cut = self.cues.len() - 512;
            self.cues.drain(..cut);
        }
    }

    fn on_event(&mut self, ev: &BossEvent, playing: bool) {
        match ev {
            // JS `boss:intro` (L78-83).
            BossEvent::Intro => {
                self.begin();
                self.sfx("boss_title", 0.0, 1.0, 1.0);
                self.sfx("boss_roar", 0.35, 1.0, 1.0);
                self.sfx("boss_slam", 0.1, 0.8, 0.55);
            }
            // JS `boss:hit` (L114-122): the dedupe gates.
            BossEvent::Hit {
                weak,
                local,
                blocked,
                crab,
                ..
            } => {
                if *crab || *blocked || !self.active || self.dead {
                    return;
                }
                if *local {
                    if *weak {
                        // JS L119: crit gate 0.07 s.
                        if self.t - self.crit_t > 0.07 {
                            self.crit_t = self.t;
                            self.sfx("boss_crit", 0.0, 1.0, 1.0);
                        }
                    } else if self.t - self.hit_t > 0.06 {
                        // JS L120: hit gate 0.06 s.
                        self.hit_t = self.t;
                        self.sfx("boss_hit", 0.0, 1.0, 1.0);
                    }
                } else if *weak && self.t - self.crit_t > 0.3 {
                    // JS L121: remote crits ride a wider 0.3 s gate.
                    self.crit_t = self.t;
                    self.sfx("boss_crit", 0.0, 1.0, 0.5);
                }
            }
            // JS `boss:stun` (L123-128).
            BossEvent::Stunned { dur } => {
                if !self.active || self.dead {
                    return; // JS L124.
                }
                self.cues.push(BossCue::LoopStop { key: "gallop" });
                self.sfx("boss_crash", 0.0, 1.0, 1.0);
                let d = f64::from(*dur).max(0.5) as f32;
                self.cues.push(BossCue::LoopStart {
                    key: "dizzy",
                    name: "boss_dizzy",
                    // JS L127: `later(0.35, () => loop(...))`.
                    at: 0.35,
                    // JS L127: `maxDur: Math.max(0.6, dur - 0.3)`.
                    max_dur: (d - 0.3).max(0.6),
                });
            }
            // JS `boss:phase` (L129-138).
            BossEvent::Phase { phase } => {
                let ph = (*phase).clamp(1, 3);
                if ph <= self.phase {
                    return;
                }
                self.phase = ph;
                self.cues.push(BossCue::StopAll);
                self.sfx("boss_phase", 0.0, 1.0, 1.0);
                self.sfx("boss_roar", 0.3, if ph >= 3 { 0.88 } else { 0.95 }, 1.0);
                if playing {
                    let tr = self.phase_track();
                    self.music(Some(tr));
                }
            }
            // JS `boss:defeat` (L139-147).
            BossEvent::Defeat { .. } => {
                if !self.active || self.dead {
                    return;
                }
                self.dead = true;
                self.cues.push(BossCue::StopAll);
                self.music(None);
                self.cues.push(BossCue::Duck {
                    gain: 0.7,
                    dur: 3.0,
                });
                self.sfx("boss_defeat", 0.0, 1.0, 1.0);
                self.sfx("boss_sunk", 1.35, 1.0, 1.0);
            }
            // JS `boss:crablet` (L171-176). `Bc` is the net mirror of the same
            // pop — skipped so pops count once.
            BossEvent::CrabSpawn { .. } => {
                self.sfx("crablet_chitter", 0.0, 1.0, 1.0);
            }
            BossEvent::CrabBurst { killed, .. } => {
                // JS L175: `pitch: killed ? 1.1 : 0.85`.
                self.sfx("crablet_pop", 0.0, if *killed { 1.1 } else { 0.85 }, 1.0);
            }
            _ => {}
        }
    }

    /// JS `_move` (L84-113): move × phase → cues. The loop `maxDur` uses the
    /// JS `dur` fallbacks (2.5 / 2 / 3) — the sim's `Move` JVal carries the
    /// real one but the placeholder档 doesn't parse it yet.
    fn move_cue(&mut self, id: &str, phase: &str) {
        if !self.active || self.dead {
            return; // JS L85.
        }
        match (id, phase) {
            ("slam", "tele") => self.sfx("boss_tele", 0.0, 1.0, 1.0),
            // JS L90: `if (!st.sawImpact) _later(0.12, …)` — the sim raises no
            // `boss:impact`, so the delayed slam always rides.
            ("slam", "act") => self.sfx("boss_slam", 0.12, 1.0, 1.0),
            ("barrage", "tele") => self.sfx("boss_tele", 0.0, 1.12, 1.0),
            ("barrage", "act") => {
                // JS L93: `n = 3 + floor(rand() * 3)` barrels.
                let n = 3 + (self.rand() * 3.0).floor() as usize;
                for i in 0..n {
                    let p = 0.9 + self.rand() * 0.25;
                    self.sfx("boss_whistle", i as f32 * 0.2, p, 1.0);
                    self.sfx("boss_barrel", 0.95 + i as f32 * 0.2, 1.0, 0.55);
                }
            }
            ("sweep", "tele") => self.sfx("boss_cannon_charge", 0.0, 1.0, 1.0),
            ("sweep", "act") => self.cues.push(BossCue::LoopStart {
                key: "sweep",
                name: "boss_cannon_sweep",
                at: 0.0,
                max_dur: 2.7,
            }),
            ("sweep", "rec") => self.cues.push(BossCue::LoopStop { key: "sweep" }),
            ("charge", "tele") => {
                self.sfx("boss_tele", 0.0, 0.82, 1.0);
                self.sfx("boss_roar", 0.2, 1.25, 0.45);
            }
            ("charge", "act") => self.cues.push(BossCue::LoopStart {
                key: "gallop",
                name: "boss_gallop",
                at: 0.0,
                max_dur: 2.3,
            }),
            ("charge", "rec") => self.cues.push(BossCue::LoopStop { key: "gallop" }),
            ("crablets", "tele") => self.sfx("boss_tele", 0.0, 1.25, 1.0),
            ("crablets", "act") => {
                self.sfx("crablet_chitter", 0.0, 1.0, 1.0);
                self.sfx("crablet_chitter", 0.3, 1.15, 1.0);
            }
            ("frenzy", "tele") => self.sfx("boss_roar", 0.0, 1.15, 0.7),
            ("frenzy", "act") => self.cues.push(BossCue::LoopStart {
                key: "frenzy",
                name: "boss_frenzy",
                at: 0.0,
                max_dur: 3.3,
            }),
            ("frenzy", "rec") => self.cues.push(BossCue::LoopStop { key: "frenzy" }),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::audio::AudioPlugin;
    use inkwave_sim::actor::SplatCause;
    use inkwave_sim::collision::CollisionWorld;

    fn assert_pcm_ok(p: &Pcm) {
        assert!(!p.data.is_empty());
        assert_eq!(p.sample_rate, SR);
        assert!(p.data.iter().all(|s| s.is_finite()));
        assert!(p.data.iter().all(|s| s.abs() <= 1.0));
        assert!(
            p.data.iter().any(|s| s.abs() > 0.01),
            "cue must not be silent"
        );
    }

    #[test]
    fn synthesis_is_bounded_finite_and_deterministic() {
        let cues = [
            shot_pcm(),
            splat_kill_pcm(),
            splat_self_pcm(),
            splat_ally_pcm(),
            empty_pcm(),
            count_pcm(3),
            tick_pcm(),
            finish_pcm(),
            victory_pcm(),
            defeat_pcm(),
        ];
        for c in &cues {
            assert_pcm_ok(c);
        }
        // Determinism: same inputs ⇒ same bytes (no RNG state involved).
        assert_eq!(shot_pcm().data, shot_pcm().data);
        assert_eq!(victory_pcm().data, victory_pcm().data);
        // Countdown pitch rises as n falls (final_count semantics): compare
        // zero-crossing counts over the first 0.05 s as a pitch proxy.
        let crossings = |p: &Pcm| {
            let w = (0.05 * SR as f32) as usize;
            p.data[..w]
                .iter()
                .zip(p.data[1..=w].iter())
                .filter(|(a, b)| (**a >= 0.0) != (**b >= 0.0))
                .count()
        };
        assert!(crossings(&count_pcm(1)) > crossings(&count_pcm(5)));
    }

    #[test]
    fn decodable_source_reports_metadata() {
        let pcm = count_pcm(2);
        let frames = pcm.data.len();
        let snd = PcmSound(pcm.clone());
        let mut d = snd.decoder();
        assert_eq!(d.channels().get(), 1);
        assert_eq!(d.sample_rate().get(), SR);
        assert_eq!(
            d.total_duration(),
            Some(Duration::from_secs_f64(frames as f64 / SR as f64))
        );
        assert_eq!(d.current_span_len(), Some(frames));
        assert_eq!(d.by_ref().count(), frames);
        assert_eq!(d.current_span_len(), Some(0));
    }

    #[test]
    fn cue_mapping_mirrors_upstream_credit_cases() {
        let splat = |attacker: Option<(usize, usize)>, vt: usize, vs: usize| MatchEvent::Splat {
            victim_team: vt,
            victim_slot: vs,
            attacker,
            cause: SplatCause::Weapon,
        };
        assert_eq!(
            splat_cue(&splat(Some((0, 0)), 1, 2), 0),
            Some(Sfx::SplatKill)
        );
        assert_eq!(
            splat_cue(&splat(Some((1, 1)), 0, 0), 0),
            Some(Sfx::SplatSelf)
        );
        assert_eq!(
            splat_cue(&splat(Some((1, 1)), 0, 3), 0),
            Some(Sfx::SplatAlly)
        );
        assert_eq!(splat_cue(&splat(Some((1, 1)), 1, 2), 0), None);
        assert_eq!(splat_cue(&MatchEvent::OneMinute, 0), None);

        let win = MatchResult {
            coverage: [0.6, 0.4],
            winner: 0,
            points: [0.0, 0.0],
            elapsed: 90.0,
            zones: None,
            boss: None,
        };
        let lose = MatchResult {
            winner: 1,
            ..win.clone()
        };
        assert_eq!(
            phase_cue(&MatchEvent::Phase(Phase::End), Some(&win), 0),
            Some(Sfx::Victory)
        );
        assert_eq!(
            phase_cue(&MatchEvent::Phase(Phase::End), Some(&lose), 0),
            Some(Sfx::Defeat)
        );
        // End before the judge settles (result None) stays silent.
        assert_eq!(phase_cue(&MatchEvent::Phase(Phase::End), None, 0), None);
        assert_eq!(
            phase_cue(&MatchEvent::Phase(Phase::Finish), None, 0),
            Some(Sfx::Finish)
        );
        assert_eq!(
            phase_cue(&MatchEvent::Countdown { n: 3 }, None, 0),
            Some(Sfx::Count(3))
        );
        assert_eq!(phase_cue(&MatchEvent::OneMinute, None, 0), Some(Sfx::Tick));
    }

    #[test]
    fn shot_edge_detects_last_fire_reset() {
        assert!(fired_this_frame(0.0, 99.0));
        assert!(fired_this_frame(0.016, 0.02));
        // Plain accumulation between frames is not a shot.
        assert!(!fired_this_frame(0.033, 0.016));
        assert!(!fired_this_frame(0.016, 0.016));
        // Restart/reset (value jumps back to 99) must not fake a shot.
        assert!(!fired_this_frame(99.0, 0.5));
    }

    /// Minimal world: real DemoSim + the audio_sfx system, no renderer.
    /// `InputPlugin` is deliberately NOT added — its PreUpdate system clears
    /// `just_pressed` before `Update`, which would swallow synthetic presses;
    /// the tests drive `ButtonInput<KeyCode>` directly instead.
    fn audio_test_app() -> App {
        let layout = inkwave_sim::embedded_tidewater();
        let world = CollisionWorld::from_layout(&layout);
        let demo = DemoSim::new(layout, world, 20261005);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            AudioPlugin::default(),
        ))
        .add_audio_source::<PcmSound>()
        .init_resource::<GlobalVolume>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(demo)
        .insert_resource(PlayerControls {
            paused: false,
            ..default()
        })
        .init_resource::<SoundState>();
        app.add_systems(Update, audio_sfx);
        let sfx = {
            let mut amut = app.world_mut().resource_mut::<Assets<PcmSound>>();
            SfxAssets::make(&mut amut)
        };
        app.insert_resource(sfx);
        app
    }

    /// One `just_pressed` edge for `key`: press, run a frame, then emulate
    /// the input system's per-frame housekeeping (release + clear).
    fn tap_key(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        k.release(key);
        k.clear();
    }

    fn spawned_handles(app: &mut App) -> Vec<Handle<PcmSound>> {
        app.world_mut()
            .query::<&AudioPlayer<PcmSound>>()
            .iter(app.world())
            .map(|p| p.0.clone())
            .collect()
    }

    #[test]
    fn shot_and_splat_events_spawn_players() {
        let mut app = audio_test_app();
        app.update();
        assert!(spawned_handles(&mut app).is_empty());

        // Force a last_fire drop: the tracker starts at 99 (fresh actor), so
        // nudge it above the actor's current value. (Relies on this test app
        // not stepping the sim — `last_fire` stays at its reset value.)
        app.world_mut().resource_mut::<SoundState>().prev_last_fire = 100.0;
        app.update();
        let h = spawned_handles(&mut app);
        let shot = app
            .world()
            .resource::<SfxAssets>()
            .get(Sfx::Shot)
            .unwrap()
            .clone();
        assert!(h.contains(&shot), "last_fire reset spawns the shot cue");

        // Splat events are heard before ui_state_machine drains them (here the
        // test owns the vec): local kill + ally death.
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            demo.m.events.push(MatchEvent::Splat {
                victim_team: 1,
                victim_slot: 2,
                attacker: Some((0, 0)),
                cause: SplatCause::Weapon,
            });
            demo.m.events.push(MatchEvent::Splat {
                victim_team: 0,
                victim_slot: 1,
                attacker: None,
                cause: SplatCause::Water,
            });
        }
        app.update();
        let h = spawned_handles(&mut app);
        let assets = app.world().resource::<SfxAssets>();
        assert!(h.contains(assets.get(Sfx::SplatKill).unwrap()));
        assert!(h.contains(assets.get(Sfx::SplatAlly).unwrap()));
    }

    #[test]
    fn dry_fire_clicks_once_per_cooldown() {
        let mut app = audio_test_app();
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            demo.m.phase = Phase::Active;
            demo.m.actors[0].ink = 0.0;
        }
        app.world_mut().resource_mut::<PlayerControls>().intent.fire = true;
        app.update();
        let h = spawned_handles(&mut app);
        let empty = app
            .world()
            .resource::<SfxAssets>()
            .get(Sfx::Empty)
            .unwrap()
            .clone();
        assert!(h.contains(&empty), "held trigger below ink_per_shot clicks");
        // Next frame the replica emptyCd is still armed: no second click.
        app.update();
        let h2 = spawned_handles(&mut app);
        assert_eq!(h2.len(), h.len(), "0.45 s cooldown between clicks");
        // Death guard (M-2): a dead local actor stays silent even with the
        // trigger held and the cooldown expired (upstream's runner resets on
        // splat, so `_empty` never fires while dead).
        {
            app.world_mut().resource_mut::<DemoSim>().m.actors[0].alive = false;
            app.world_mut().resource_mut::<SoundState>().empty_cd = 0.0;
        }
        app.update();
        assert_eq!(
            spawned_handles(&mut app).len(),
            h2.len(),
            "no dry-fire click while dead"
        );
    }

    #[test]
    fn mute_switch_stops_spawns_and_global_volume() {
        let mut app = audio_test_app();
        app.update();
        // Toggle mute with M (one edge).
        tap_key(&mut app, KeyCode::KeyM);
        assert!(app.world().resource::<SoundState>().muted);
        assert_eq!(
            app.world().resource::<GlobalVolume>().volume,
            Volume::SILENT
        );

        // Events arrive while muted: nothing spawns.
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            demo.m.events.push(MatchEvent::Countdown { n: 3 });
        }
        app.update();
        assert!(spawned_handles(&mut app).is_empty(), "muted: no entities");

        // Unmute restores audible playback for the next cue.
        tap_key(&mut app, KeyCode::KeyM);
        assert!(!app.world().resource::<SoundState>().muted);
        assert!(app.world().resource::<GlobalVolume>().volume > Volume::SILENT);
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            demo.m.events.push(MatchEvent::Countdown { n: 2 });
        }
        app.update();
        let h = spawned_handles(&mut app);
        let assets = app.world().resource::<SfxAssets>();
        assert!(h.contains(assets.get(Sfx::Count(2)).unwrap()));
    }

    #[test]
    fn paused_screen_hears_nothing() {
        let mut app = audio_test_app();
        app.world_mut().resource_mut::<PlayerControls>().paused = true;
        {
            let mut demo = app.world_mut().resource_mut::<DemoSim>();
            demo.m.events.push(MatchEvent::Splat {
                victim_team: 1,
                victim_slot: 0,
                attacker: Some((0, 0)),
                cause: SplatCause::Weapon,
            });
        }
        app.update();
        assert!(
            spawned_handles(&mut app).is_empty(),
            "menu/pause screens stay silent"
        );
    }

    // ---- Task 46 batch C2 — bossAudio placeholder档 -----------------------

    fn af<'a>(move_id: Option<&'a str>, move_phase: Option<&'a str>) -> BossAudioFrame<'a> {
        BossAudioFrame {
            move_id,
            move_phase,
            playing: true,
            ended: false,
        }
    }

    fn sfx_names(d: &BossAudioDirector) -> Vec<&'static str> {
        d.cues
            .iter()
            .filter_map(|c| match c {
                BossCue::Sfx { name, .. } => Some(*name),
                _ => None,
            })
            .collect()
    }

    /// JS `_move` (bossAudio.js L84-113): every move × phase row of the table.
    #[test]
    fn boss_audio_move_phase_table() {
        let mut d = BossAudioDirector::new();
        d.begin();
        // Consume the 'playing' music edge before checking move cues.
        d.update(0.0, &[], &af(None, None));
        assert_eq!(d.cues, vec![BossCue::Music(Some("boss"))]);
        d.cues.clear();
        // slam: tele → boss_tele; act → delayed boss_slam (0.12 s, JS L90).
        d.update(0.1, &[], &af(Some("slam"), Some("tele")));
        d.update(0.1, &[], &af(Some("slam"), Some("act")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues,
            vec![
                BossCue::Sfx {
                    name: "boss_tele",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_slam",
                    at: 0.12,
                    pitch: 1.0,
                    volume: 1.0
                },
            ]
        );

        // barrage: tele pitch 1.12; act = n whistles + n barrels (JS L92-99).
        d.update(0.1, &[], &af(Some("barrage"), Some("tele")));
        d.update(0.1, &[], &af(Some("barrage"), Some("act")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues[0],
            BossCue::Sfx {
                name: "boss_tele",
                at: 0.0,
                pitch: 1.12,
                volume: 1.0
            }
        );
        let n = cues[1..]
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    BossCue::Sfx {
                        name: "boss_whistle",
                        ..
                    }
                )
            })
            .count();
        assert!((3..=5).contains(&n), "n = 3 + rand3, got {n}");
        let barrels = cues[1..]
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    BossCue::Sfx {
                        name: "boss_barrel",
                        ..
                    }
                )
            })
            .count();
        assert_eq!(barrels, n, "one barrel per whistle");
        // Whistle i rides at i*0.2, barrel i at 0.95 + i*0.2 (JS L96-98).
        let first_barrel = cues
            .iter()
            .find(|c| {
                matches!(
                    c,
                    BossCue::Sfx {
                        name: "boss_barrel",
                        ..
                    }
                )
            })
            .unwrap();
        assert_eq!(
            *first_barrel,
            BossCue::Sfx {
                name: "boss_barrel",
                at: 0.95,
                pitch: 1.0,
                volume: 0.55
            }
        );

        // sweep: tele charge, act loop, rec stop (JS L100-102).
        d.update(0.1, &[], &af(Some("sweep"), Some("tele")));
        d.update(0.1, &[], &af(Some("sweep"), Some("act")));
        d.update(0.1, &[], &af(Some("sweep"), Some("rec")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues,
            vec![
                BossCue::Sfx {
                    name: "boss_cannon_charge",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::LoopStart {
                    key: "sweep",
                    name: "boss_cannon_sweep",
                    at: 0.0,
                    max_dur: 2.7
                },
                BossCue::LoopStop { key: "sweep" },
            ]
        );

        // charge: tele + 0.2 s roar, act gallop loop, rec stop (JS L103-105).
        d.update(0.1, &[], &af(Some("charge"), Some("tele")));
        d.update(0.1, &[], &af(Some("charge"), Some("act")));
        d.update(0.1, &[], &af(Some("charge"), Some("rec")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues,
            vec![
                BossCue::Sfx {
                    name: "boss_tele",
                    at: 0.0,
                    pitch: 0.82,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 0.2,
                    pitch: 1.25,
                    volume: 0.45
                },
                BossCue::LoopStart {
                    key: "gallop",
                    name: "boss_gallop",
                    at: 0.0,
                    max_dur: 2.3
                },
                BossCue::LoopStop { key: "gallop" },
            ]
        );

        // crablets: tele pitch 1.25, act chitter + 0.3 s echo (JS L106-107).
        d.update(0.1, &[], &af(Some("crablets"), Some("tele")));
        d.update(0.1, &[], &af(Some("crablets"), Some("act")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues,
            vec![
                BossCue::Sfx {
                    name: "boss_tele",
                    at: 0.0,
                    pitch: 1.25,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "crablet_chitter",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "crablet_chitter",
                    at: 0.3,
                    pitch: 1.15,
                    volume: 1.0
                },
            ]
        );

        // frenzy: tele roar, act loop, rec stop (JS L108-110).
        d.update(0.1, &[], &af(Some("frenzy"), Some("tele")));
        d.update(0.1, &[], &af(Some("frenzy"), Some("act")));
        d.update(0.1, &[], &af(Some("frenzy"), Some("rec")));
        let cues = std::mem::take(&mut d.cues);
        assert_eq!(
            cues,
            vec![
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 0.0,
                    pitch: 1.15,
                    volume: 0.7
                },
                BossCue::LoopStart {
                    key: "frenzy",
                    name: "boss_frenzy",
                    at: 0.0,
                    max_dur: 3.3
                },
                BossCue::LoopStop { key: "frenzy" },
            ]
        );
    }

    /// JS `boss:hit` (L114-122): the 0.06 / 0.07 / 0.3 s dedupe gates.
    #[test]
    fn boss_audio_hit_dedupe_gates() {
        let mut d = BossAudioDirector::new();
        d.begin();
        let hit = |weak: bool, local: bool| BossEvent::Hit {
            damage: 1.0,
            weak,
            attacker: 0,
            local,
            blocked: false,
            predicted: false,
            crab: false,
            pos: None,
        };
        // Local strong hits at t = 0.016 / 0.032 (Δ 0.016 < 0.06) / 0.080.
        d.update(0.016, &[hit(false, true)], &af(None, None));
        d.update(0.016, &[hit(false, true)], &af(None, None));
        d.update(0.048, &[hit(false, true)], &af(None, None));
        assert_eq!(
            sfx_names(&d),
            vec!["boss_hit", "boss_hit"],
            "0.06 s gate (the music edge is not an Sfx)"
        );
        d.cues.clear();
        // Local weak fires once, then the 0.07 s gate holds (JS L119).
        d.update(0.064, &[hit(true, true)], &af(None, None));
        assert_eq!(sfx_names(&d), vec!["boss_crit"]);
        d.update(0.02, &[hit(true, true)], &af(None, None));
        assert_eq!(sfx_names(&d), vec!["boss_crit"], "crit gate 0.07 s");
        d.cues.clear();
        // Remote weak shares critT with a wider 0.3 s gate, volume 0.5 (L121):
        // Δ 0.1 < 0.3 gated, Δ 0.2 (total 0.32) fires.
        d.update(0.1, &[hit(true, false)], &af(None, None));
        assert!(sfx_names(&d).is_empty(), "remote crit 0.3 s gate");
        d.update(0.22, &[hit(true, false)], &af(None, None));
        assert_eq!(
            d.cues[0],
            BossCue::Sfx {
                name: "boss_crit",
                at: 0.0,
                pitch: 1.0,
                volume: 0.5
            }
        );
        // Remote STRONG hits are silent upstream (JS L121 only weak); blocked
        // and crab hits too (L115).
        d.update(0.5, &[hit(false, false)], &af(None, None));
        let blocked = BossEvent::Hit {
            damage: 1.0,
            weak: false,
            attacker: 0,
            local: true,
            blocked: true,
            predicted: false,
            crab: false,
            pos: None,
        };
        d.update(0.5, &[blocked], &af(None, None));
        assert_eq!(
            sfx_names(&d).len(),
            1,
            "remote strong + blocked stay silent"
        );
    }

    /// JS intro / phase / defeat / stun / crablet handlers (L78-147).
    #[test]
    fn boss_audio_events_music_and_loops() {
        let mut d = BossAudioDirector::new();
        // intro begins + title/roar/slam (JS L78-83); the same frame's
        // 'playing' edge starts the phase-1 track (L9, L71).
        d.update(0.016, &[BossEvent::Intro], &af(None, None));
        assert_eq!(
            d.cues,
            vec![
                BossCue::Sfx {
                    name: "boss_title",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 0.35,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_slam",
                    at: 0.1,
                    pitch: 0.8,
                    volume: 0.55
                },
                BossCue::Music(Some("boss")),
            ]
        );
        assert!(d.active);
        d.cues.clear();
        // phase 2: stopAll + boss_phase + 0.3 s roar + music switch (L129-138).
        d.update(0.016, &[BossEvent::Phase { phase: 2 }], &af(None, None));
        assert_eq!(
            d.cues,
            vec![
                BossCue::StopAll,
                BossCue::Sfx {
                    name: "boss_phase",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 0.3,
                    pitch: 0.95,
                    volume: 1.0
                },
                BossCue::Music(Some("boss_2")),
            ]
        );
        // A repeat phase is ignored (JS L133).
        d.cues.clear();
        d.update(0.016, &[BossEvent::Phase { phase: 2 }], &af(None, None));
        assert!(d.cues.is_empty());
        d.cues.clear();
        // stun: gallop stop + crash + 0.35 s dizzy loop (L123-128).
        d.update(0.016, &[BossEvent::Stunned { dur: 3.0 }], &af(None, None));
        assert_eq!(
            d.cues,
            vec![
                BossCue::LoopStop { key: "gallop" },
                BossCue::Sfx {
                    name: "boss_crash",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::LoopStart {
                    key: "dizzy",
                    name: "boss_dizzy",
                    at: 0.35,
                    max_dur: 2.7
                },
            ]
        );
        d.cues.clear();
        // defeat: stopAll + music off + duck + defeat + 1.35 s sunk (L139-147).
        d.update(0.016, &[BossEvent::Defeat { killer: 0 }], &af(None, None));
        assert_eq!(
            d.cues,
            vec![
                BossCue::StopAll,
                BossCue::Music(None),
                BossCue::Duck {
                    gain: 0.7,
                    dur: 3.0
                },
                BossCue::Sfx {
                    name: "boss_defeat",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "boss_sunk",
                    at: 1.35,
                    pitch: 1.0,
                    volume: 1.0
                },
            ]
        );
        // The clock expiring after a defeat adds nothing (JS L72 `!st.dead`).
        d.cues.clear();
        let mut f = af(None, None);
        f.ended = true;
        d.update(0.016, &[], &f);
        assert!(d.cues.is_empty(), "defeat already silenced the set");
    }

    /// JS `boss:crablet` (L171-176): spawn chitter, pop with the killed pitch.
    #[test]
    fn boss_audio_crablet_cues() {
        let mut d = BossAudioDirector::new();
        d.begin();
        d.update(0.0, &[], &af(None, None)); // consume the music edge
        d.cues.clear();
        d.update(
            0.016,
            &[
                BossEvent::CrabSpawn {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                BossEvent::CrabBurst {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    killed: true,
                },
                BossEvent::CrabBurst {
                    id: 2,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    killed: false,
                },
                // The `Bc` net mirror must NOT double-count the pop.
                BossEvent::Bc {
                    id: 2,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    killed: false,
                },
            ],
            &af(None, None),
        );
        assert_eq!(
            d.cues,
            vec![
                BossCue::Sfx {
                    name: "crablet_chitter",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "crablet_pop",
                    at: 0.0,
                    pitch: 1.1,
                    volume: 1.0
                },
                BossCue::Sfx {
                    name: "crablet_pop",
                    at: 0.0,
                    pitch: 0.85,
                    volume: 1.0
                },
            ]
        );
    }
}
