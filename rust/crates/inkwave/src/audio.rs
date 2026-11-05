//! Task 14 → Task 50 batch B — the SFX engine, ported from `src/audio/audio.js`.
//!
//! The M1 placeholder beeps are gone: every cue the M1 event set can reach is
//! now baked from the real 148-entry `SFX` table ([`sfx_table`]) through the
//! ported WebAudio graph ([`graph`] / [`synth`]), and played through the ported
//! `AudioEngine.play()` gate (minGap merge, per-name + global voice caps,
//! pitch jitter — deviation D-T50-6: jitter rides rodio `speed`). The trigger
//! set (main.js L499-519 / L548-549 / L561 / L971+L1027, hud.js L510,
//! weapons.js L162/L1040; the sub `low_ink` nag of weapons.js L138 has no M1
//! sub weapon, so it is not wired):
//!   - shot            — local player fires (`Actor::last_fire` reset edge),
//!     cue chosen by `Match::kinds[0]` (weapons.js L893-896/L914/L1040)
//!   - splat kill/self/ally — `MatchEvent::Splat` credit cases
//!   - empty click     — trigger held below `ink_per_shot` (JS `_empty`, 0.45 s
//!     cooldown mirrors `EMPTY_CD`)
//!   - countdown / 60 s tick — `MatchEvent::Countdown` (`final_count` +
//!     `countdown_tick` for n ≤ 3) / `OneMinute` (`one_minute`)
//!   - finish whistle / victory / defeat — `MatchEvent::Phase(Finish|End)`
//!
//! PCM is baked lazily on first trigger by [`sfx_table::bake`] (44.1 kHz
//! mono, clamped to [-1, 1]; the handle is cached per name, so each graph
//! render runs at most once) and fed to Bevy through a custom [`Decodable`]
//! asset ([`PcmSound`]) — the default `AudioSource` path needs *encoded*
//! bytes and the default feature set has no `wav` decoder, so raw f32
//! buffers must ride their own source type (`App::add_audio_source::<PcmSound>()`).
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

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

pub mod boss_audio;
pub mod graph;
pub mod music;
pub mod sfx_table;
pub mod synth;

use bevy::audio::{
    AddAudioSource, AudioPlayer, ChannelCount, Decodable, GlobalVolume, PlaybackSettings,
    SampleRate, Source as RodioSource, Volume,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use inkwave_sim::boss::BossEvent;
use inkwave_sim::match_::{MatchEvent, MatchResult, Phase};
use inkwave_sim::weapon::WeaponKind;

use sfx_table::{PlayDecision, SfxGate};

use crate::ink_render::DemoSim;
use crate::input::PlayerControls;

/// Sample rate of every baked cue (≡ the WebAudio context rate).
const SR: u32 = 44100;
/// [`SfxBorn`] slack: a voice entity is reaped this long after its expected
/// end if the backend never got to it (device-less hosts).
const GC_SLACK: f32 = 2.0;

// ---------------------------------------------------------------------------
// Synthesis (pure, unit-testable)
// ---------------------------------------------------------------------------

/// Mono float PCM block.
#[derive(Clone, Debug)]
pub struct Pcm {
    pub sample_rate: u32,
    pub data: Vec<f32>,
}

/// Bake `name` through the ported graph and clamp to [-1, 1]: the JS master
/// chain's glue/limiter (audio.js L15-18) is live-bus and not baked, so hot
/// peaks are hard-clamped instead of allowed to wrap in rodio. `None` for an
/// unknown name (≡ JS `play()` returning null, review P2-8).
fn baked(name: &str) -> Option<Pcm> {
    let mut data = sfx_table::bake(name)?;
    for s in &mut data {
        *s = s.clamp(-1.0, 1.0);
    }
    Some(Pcm {
        sample_rate: SR,
        data,
    })
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

/// Sfx → the upstream `audio.play(name, {volume})` calls it mirrors:
/// weapons.js L893-896/L914/L1040 (shot, per weapon kind), main.js L499-519
/// (splat credits 0.9 / 1.0 / 0.5), L548-549 (`final_count`, plus hud.js
/// L510's `countdown_tick` at 0.5 for n ≤ 3), L561 (`times_up`), L971/L1027
/// (victory/defeat), weapons.js L162 (`empty_click`). Pitch stays 1.0: the
/// JS runtime pitch knobs (dualies hand alternation, charger charge level)
/// are not plumbed into the M1 event set (deviation, D-T50-6).
#[must_use]
pub(crate) fn sfx_cue(s: Sfx, shot_kind: WeaponKind) -> Vec<(&'static str, f64)> {
    match s {
        Sfx::Shot => vec![(shot_name(shot_kind), 0.55)],
        Sfx::SplatKill => vec![("splat_enemy", 0.9)],
        Sfx::SplatSelf => vec![("splatted_self", 1.0)],
        Sfx::SplatAlly => vec![("ally_splatted", 0.5)],
        Sfx::Empty => vec![("empty_click", 1.0)],
        Sfx::Count(n) => {
            let mut v = vec![("final_count", 1.0)];
            if (1..=3).contains(&n) {
                v.push(("countdown_tick", 0.5));
            }
            v
        }
        Sfx::Tick => vec![("one_minute", 1.0)],
        Sfx::Finish => vec![("times_up", 1.0)],
        Sfx::Victory => vec![("victory_fanfare", 1.0)],
        Sfx::Defeat => vec![("defeat_jingle", 1.0)],
    }
}

/// Main-weapon shot cue ≡ weapons.js `IMPL.shoot` cases (L893-895: `dualies`
/// = `Twins`, `splatling` = `Spinner`; L914 charger). Kinds without a
/// dedicated shot cue in the 148-entry table (brush/bow/blade/mitts/brolly
/// kit sounds live elsewhere upstream) fall back to the shooter chirp.
#[must_use]
pub(crate) fn shot_name(kind: WeaponKind) -> &'static str {
    match kind {
        WeaponKind::Blaster => "shoot_blaster",
        WeaponKind::Charger => "shoot_charger",
        WeaponKind::Twins => "shoot_dualies",
        WeaponKind::Spinner => "shoot_splatling",
        WeaponKind::Roller => "roller_flick",
        WeaponKind::Bucket => "slosh_throw",
        _ => "shoot_shooter",
    }
}

/// Baked handles for every cue [`sfx_cue`] can reach, lazily rendered on
/// first trigger (the graph bake is one-shot per name; the handle is cached
/// afterwards — startup stays free of synthesis cost).
#[derive(Resource, Default)]
pub struct SfxAssets {
    map: HashMap<&'static str, Handle<PcmSound>>,
    dur: HashMap<&'static str, f64>,
}

impl SfxAssets {
    /// Every name [`sfx_cue`] can reach — the M1 bakeable set (asserted by
    /// the tests; the bake itself accepts any table name).
    #[allow(dead_code)]
    pub const CUES: &'static [&'static str] = &[
        "shoot_shooter",
        "shoot_blaster",
        "shoot_charger",
        "shoot_dualies",
        "shoot_splatling",
        "roller_flick",
        "slosh_throw",
        "splat_enemy",
        "splatted_self",
        "ally_splatted",
        "empty_click",
        "final_count",
        "countdown_tick",
        "one_minute",
        "times_up",
        "victory_fanfare",
        "defeat_jingle",
    ];

    /// Handle + baked duration, baking `name` on first use. Any name in the
    /// 148-entry table bakes (the M1 set is [`Self::CUES`]; the boss
    /// director's [`BossCue::Sfx`](boss_audio) adds the rest).
    pub(crate) fn cue(
        &mut self,
        assets: &mut Assets<PcmSound>,
        name: &'static str,
    ) -> Option<(Handle<PcmSound>, f64)> {
        if let Some(h) = self.map.get(name) {
            return Some((h.clone(), self.dur.get(name).copied()?));
        }
        let pcm = baked(name)?;
        let dur = pcm.data.len() as f64 / f64::from(SR);
        let h = assets.add(PcmSound(pcm));
        self.map.insert(name, h.clone());
        self.dur.insert(name, dur);
        Some((h, dur))
    }

    /// Read-only access to an already-baked cue (test seam: production reads
    /// through [`Self::cue`]).
    #[cfg(test)]
    fn get(&self, name: &str) -> Option<(&Handle<PcmSound>, f64)> {
        let h = self.map.get(name)?;
        Some((h, self.dur.get(name).copied().unwrap_or(0.0)))
    }
}

/// Audio runtime state: master mute switch, per-frame edge trackers and the
/// [`SfxGate`] voice bookkeeping (≡ `engine.last` / `byName` / `voices`).
#[derive(Resource)]
pub struct SoundState {
    pub muted: bool,
    /// `Actor::last_fire` seen at the previous frame; a drop means the local
    /// player fired (the sim's `note_fired` resets it to 0).
    prev_last_fire: f32,
    /// Own `emptyCd` replica (weapons.js L160-162) for the dry-fire click.
    empty_cd: f32,
    /// Play gate: minGap merge, voice caps, pitch jitter (fixed seed ⇒ the
    /// jitter stream is deterministic, unlike the JS `Math.random`).
    pub gate: SfxGate,
    /// Live voice entities per cue name, oldest first — the steal victims.
    voices: HashMap<&'static str, VecDeque<(Entity, f64)>>,
}

impl Default for SoundState {
    fn default() -> Self {
        Self {
            muted: false,
            // Fresh/reset actors start at last_fire = 99 (actor.rs L234).
            prev_last_fire: 99.0,
            empty_cd: 0.0,
            gate: SfxGate::new(0x494E_4B57),
            voices: HashMap::new(),
        }
    }
}

/// Marker stamped on every spawned sound entity so [`sfx_gc`] can reap ones
/// the audio backend never got to (no device ⇒ `DESPAWN` cleanup never runs).
/// Stores the *deadline* (expected end + [`GC_SLACK`]); baked cues run up to
/// ~3 s, so a fixed birth-age threshold would cut live playback.
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
    mut sfx: ResMut<SfxAssets>,
    mut pcm: ResMut<Assets<PcmSound>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut gvol: ResMut<GlobalVolume>,
    time: Res<Time>,
    mut commands: Commands,
    vols: Res<music::AudioVolumes>,
) {
    // Master: `M` mute + the `master` slider (audio.js L165-172). The WebAudio
    // master gain ≡ Bevy's `GlobalVolume`; it is refreshed every frame so
    // `set_volumes` re-targets (τ = 0.04 s) land without an edge.
    let now = time.elapsed_secs() as f64;
    if keys.just_pressed(KeyCode::KeyM) {
        snd.muted = !snd.muted;
        println!("[inkwave] audio {}", if snd.muted { "MUTED" } else { "on" });
    }
    gvol.volume = if snd.muted {
        Volume::SILENT
    } else {
        Volume::Linear(vols.master.value_at(now) as f32)
    };

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
    let shot_kind = demo.m.kinds.first().copied().unwrap_or(WeaponKind::Shooter);
    for c in cues {
        for (name, vol) in sfx_cue(c, shot_kind) {
            // Lazy bake: the graph render runs once per name, cached after.
            let Some((handle, dur)) = sfx.cue(&mut pcm, name) else {
                continue;
            };
            // ≡ JS `audio.play(name, {volume})` → gate → `src.start(at)`;
            // Merged/Silent collapse the same way upstream does.
            let Some(PlayDecision::Spawned {
                speed,
                volume,
                steal,
            }) = snd.gate.play(name, now, None, 0.0, vol, 1.0, dur)
            else {
                continue;
            };
            let q = snd.voices.entry(name).or_default();
            q.retain(|(_, end)| *end > now);
            // Voice steal ≡ JS `_steal` 25 ms fade; deviation D-T50-6: the
            // stolen (oldest) voice is despawned immediately.
            for _ in 0..steal {
                if let Some((e, _)) = q.pop_front()
                    && let Ok(mut ent) = commands.get_entity(e)
                {
                    ent.despawn();
                }
            }
            let end = now + dur / speed;
            let e = commands
                .spawn((
                    AudioPlayer(handle.clone()),
                    PlaybackSettings::DESPAWN
                        .with_volume(Volume::Linear((volume * vols.sfx.value_at(now)) as f32))
                        .with_speed(speed as f32),
                    SfxBorn((end + f64::from(GC_SLACK)) as f32),
                ))
                .id();
            q.push_back((e, end));
        }
    }
}

/// Reap sound entities that no backend consumed (device-less hosts keep them
/// queued forever; with a device `DESPAWN` fires first and this is a no-op).
pub(crate) fn sfx_gc(time: Res<Time>, mut commands: Commands, q: Query<(Entity, &SfxBorn)>) {
    let now = time.elapsed_secs();
    for (e, born) in q.iter() {
        // `SfxBorn` is the deadline (expected end + slack).
        if now > born.0
            && let Ok(mut ent) = commands.get_entity(e)
        {
            ent.despawn();
        }
    }
}

/// Startup evidence line (TR-14.1): the table is live; cues bake lazily on
/// first trigger. The backend itself logs "No audio device found." once when
/// the host has no output.
pub fn log_audio_ready() {
    println!(
        "[inkwave] audio: SFX table ready ({} cues / {} groups, lazy-baked; M = mute)",
        sfx_table::sfx_names().len(),
        sfx_table::SFX_GROUPS.len()
    );
}

/// Register the PCM source type + resources/systems (called from WorldPlugin;
/// `DefaultPlugins` already provides `AudioPlugin`). `sfx_gc` is added by the
/// caller inside the Update chain (after `audio_sfx`'s spawns apply).
pub fn add_audio(app: &mut App) {
    app.add_audio_source::<PcmSound>()
        .init_resource::<SoundState>()
        .init_resource::<SfxAssets>()
        .init_resource::<GlobalVolume>()
        .init_resource::<music::AudioVolumes>()
        .init_resource::<music::MusicAssets>()
        .init_resource::<music::MusicRuntime>()
        .init_resource::<music::MusicSceneState>()
        .init_resource::<boss_audio::BossAudioRuntime>()
        .add_systems(Startup, log_audio_ready);
}

// ---------------------------------------------------------------------------
// Task 46 batch C2 / Task 50 batch D — bossAudio (≡ src/audio/bossAudio.js).
// Every bus subscription becomes a trigger RECORD (SFX name + delay +
// pitch/volume + loop open/close + music track) so the unit tests can pin
// counts / order / the dedupe gates; `boss_audio::boss_audio_playback`
// consumes the records and turns them into real playback.
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
    /// The trigger log (bounded; consumed by tests and by the real playback
    /// system, [`boss_audio::boss_audio_playback`]).
    pub cues: Vec<BossCue>,
    /// Batch D playback cursor: index into `cues` already consumed.
    pub(crate) consumed: usize,
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
            consumed: 0,
        }
    }

    /// The `music.remap` state this director installs (bossAudio.js L58).
    #[must_use]
    pub(crate) fn remap(&self) -> music::BossRemap {
        music::BossRemap {
            active: self.active,
            dead: self.dead,
            phase: self.phase,
            track: self.track,
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
            self.consumed = self.consumed.saturating_sub(cut);
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
        for name in SfxAssets::CUES {
            assert_pcm_ok(&baked(name).unwrap());
        }
        // Determinism: fixed bake seed ⇒ same name, same bytes.
        assert_eq!(
            baked("shoot_shooter").unwrap().data,
            baked("shoot_shooter").unwrap().data
        );
        assert_eq!(
            baked("victory_fanfare").unwrap().data,
            baked("victory_fanfare").unwrap().data
        );
    }

    #[test]
    fn unknown_names_bake_and_cue_to_none() {
        // Review P2-8: ≡ JS `play()`'s `if (!d) return null` (audio.js L261) —
        // nothing is baked and no player can be spawned.
        assert!(sfx_table::bake("no_such_sound").is_none());
        assert!(baked("no_such_sound").is_none());
        let mut sfx = SfxAssets::default();
        let mut pcm = Assets::<PcmSound>::default();
        assert!(sfx.cue(&mut pcm, "no_such_sound").is_none());
    }

    #[test]
    fn cue_names_mirror_the_js_play_calls() {
        let shooter = WeaponKind::Shooter;
        assert_eq!(sfx_cue(Sfx::Shot, shooter), vec![("shoot_shooter", 0.55)]);
        assert_eq!(
            sfx_cue(Sfx::Shot, WeaponKind::Charger),
            vec![("shoot_charger", 0.55)]
        );
        assert_eq!(sfx_cue(Sfx::SplatKill, shooter), vec![("splat_enemy", 0.9)]);
        assert_eq!(
            sfx_cue(Sfx::SplatSelf, shooter),
            vec![("splatted_self", 1.0)]
        );
        assert_eq!(
            sfx_cue(Sfx::SplatAlly, shooter),
            vec![("ally_splatted", 0.5)]
        );
        assert_eq!(sfx_cue(Sfx::Empty, shooter), vec![("empty_click", 1.0)]);
        assert_eq!(sfx_cue(Sfx::Tick, shooter), vec![("one_minute", 1.0)]);
        assert_eq!(sfx_cue(Sfx::Finish, shooter), vec![("times_up", 1.0)]);
        assert_eq!(
            sfx_cue(Sfx::Victory, shooter),
            vec![("victory_fanfare", 1.0)]
        );
        assert_eq!(sfx_cue(Sfx::Defeat, shooter), vec![("defeat_jingle", 1.0)]);
        // main.js L549 plays final_count for every n; hud.js L510 adds
        // countdown_tick (volume 0.5) only for n ≤ 3.
        assert_eq!(
            sfx_cue(Sfx::Count(2), shooter),
            vec![("final_count", 1.0), ("countdown_tick", 0.5)]
        );
        assert_eq!(sfx_cue(Sfx::Count(5), shooter), vec![("final_count", 1.0)]);
        // weapons.js shoot cases (dualies = Twins, splatling = Spinner);
        // kinds without a table shot cue fall back to the shooter chirp.
        assert_eq!(shot_name(WeaponKind::Twins), "shoot_dualies");
        assert_eq!(shot_name(WeaponKind::Spinner), "shoot_splatling");
        assert_eq!(shot_name(WeaponKind::Blaster), "shoot_blaster");
        assert_eq!(shot_name(WeaponKind::Roller), "roller_flick");
        assert_eq!(shot_name(WeaponKind::Brush), "shoot_shooter");
        // Everything `sfx_cue` can emit is in the lazy-bake set.
        for s in [
            Sfx::Shot,
            Sfx::SplatKill,
            Sfx::SplatSelf,
            Sfx::SplatAlly,
            Sfx::Empty,
            Sfx::Count(1),
            Sfx::Tick,
            Sfx::Finish,
            Sfx::Victory,
            Sfx::Defeat,
        ] {
            for (name, _) in sfx_cue(s, shooter) {
                assert!(SfxAssets::CUES.contains(&name), "{name} not baked");
            }
        }
    }

    #[test]
    fn decodable_source_reports_metadata() {
        let pcm = baked("final_count").unwrap();
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
        .init_resource::<SoundState>()
        .init_resource::<music::AudioVolumes>();
        app.add_systems(Update, audio_sfx);
        // Cues bake lazily inside `audio_sfx` on first trigger.
        app.init_resource::<SfxAssets>();
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
            .get("shoot_shooter")
            .unwrap()
            .0
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
        assert!(h.contains(assets.get("splat_enemy").unwrap().0));
        assert!(h.contains(assets.get("ally_splatted").unwrap().0));
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
            .get("empty_click")
            .unwrap()
            .0
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
        assert!(h.contains(assets.get("final_count").unwrap().0));
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
