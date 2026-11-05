//! Task 50 batch D — real playback for the boss director's cue table
//! (≡ the `A()` / `G.music` calls inside `src/audio/bossAudio.js`).
//!
//! The director (Task 46 batch C2, [`super::BossAudioDirector`]) records every
//! bus subscription as a [`super::BossCue`]; [`boss_audio_playback`] consumes
//! the records made since the previous frame and turns them into the audio
//! they stand for:
//!   - `Sfx` → the [`super::SfxGate`] decision + a baked one-shot
//!     (JS `snd(name, {delay, pitch, volume})` → `A()?.play`, bossAudio.js
//!     L27-31; the `delay` becomes a due-time on the Bevy clock)
//!   - `LoopStart` / `LoopStop` / `StopAll` → baked clean-loop players
//!     (JS `_loop`/`_stopLoop`/`stopAll`, L33-48 + the `audio.loop` bus)
//!   - `Music` → [`MusicRuntime::request`] through the boss remap
//!     (JS `music(track)`, L60-66; the remap closure itself is re-synced from
//!     the director every frame, ≡ bossAudio.js L58)
//!   - `Duck` → [`MusicRuntime::duck`] on the music bus (JS `duck()`, L49)
//!
//! Deviations (registered in the Task 50 report): the cue table does not
//! carry every JS parameter — `stopAll(fade)` collapses to a hard loop stop,
//! `music(null)`'s 0.5 (defeat) / 0.6 (finish) fades collapse to 0.6, loop
//! stops despawn without the JS 0.25 s fade, and the `<name>_end` one-shot of
//! `h.stop` (audio.js L290) is not emitted. Delayed one-shots dropped while
//! `M`-muted are not replayed (the JS bus plays them silently instead). The
//! loop *start* also plays the baked buffer at full gain: the JS `loop()`
//! entry's 60 ms `fadeG` 0→1 ramp (audio.js L317-318, review P2-9) is
//! neither baked into the loop PCM nor applied at runtime.

use bevy::audio::{AudioPlayer, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::ui::BossHudRes;

use super::music::{AudioVolumes, MusicRuntime};
use super::sfx_table::{self, PlayDecision};
use super::{BossCue, Pcm, PcmSound, SR, SfxAssets, SfxBorn, SoundState};

/// One delayed one-shot: `(due, name, pitch, volume)` — the JS `_later` /
/// `delay` timer + `play()` call.
type PendingSfx = (f64, &'static str, f64, f64);

/// One delayed loop start: `(due, key, name, deadline)`.
type PendingLoop = (f64, &'static str, &'static str, f64);

/// A live loop player (JS `this.loops` entry, audio.js L237 `maxDur`).
struct LiveLoop {
    key: &'static str,
    entity: Entity,
    /// `LoopStart` deadline + `max_dur` — the watchdog stop (JS fires the
    /// automatic `h.stop()` without a matching `_stopLoop`).
    deadline: f64,
}

/// Playback side of [`BossAudioDirector`]: due-queues for the delayed cues
/// and the live loop registry.
#[derive(Resource, Default)]
pub struct BossAudioRuntime {
    pending: Vec<PendingSfx>,
    pending_loops: Vec<PendingLoop>,
    loops: Vec<LiveLoop>,
}

impl BossAudioRuntime {
    /// JS `_stopAll` / `end()` cleanup: drop every queued timer and despawn
    /// the live loops (deviation: hard stop, no JS fade).
    fn clear(&mut self, commands: &mut Commands) {
        self.pending.clear();
        self.pending_loops.clear();
        for l in self.loops.drain(..) {
            despawn(commands, l.entity);
        }
    }

    /// JS `stopLoop(key)` (audio.js L260-266).
    fn stop_loop(&mut self, key: &str, commands: &mut Commands) {
        if let Some(pos) = self.loops.iter().position(|l| l.key == key) {
            let l = self.loops.remove(pos);
            despawn(commands, l.entity);
        }
    }
}

fn despawn(commands: &mut Commands, e: Entity) {
    if let Ok(mut ent) = commands.get_entity(e) {
        ent.despawn();
    }
}

/// Consume the director's cue table and play it (runs after `ui_update`,
/// which records the cues, and after `music_update`, which owns the music
/// voices).
#[allow(clippy::too_many_arguments)]
pub fn boss_audio_playback(
    time: Res<Time>,
    mut hud: ResMut<BossHudRes>,
    mut rt: ResMut<MusicRuntime>,
    mut bar: ResMut<BossAudioRuntime>,
    mut sfx: ResMut<SfxAssets>,
    mut pcm: ResMut<Assets<PcmSound>>,
    mut commands: Commands,
    mut snd: ResMut<SoundState>,
    vols: Res<AudioVolumes>,
) {
    let now = time.elapsed_secs() as f64;
    let director = &mut hud.audio;
    // ≡ bossAudio.js L58: the remap closure reads `st` live, so the routing
    // state mirrors the director every frame.
    rt.state.remap = director.remap();
    // Consume the cues recorded since the last frame.
    while director.consumed < director.cues.len() {
        let cue = director.cues[director.consumed].clone();
        director.consumed += 1;
        match cue {
            BossCue::Sfx {
                name,
                at,
                pitch,
                volume,
            } => bar.pending.push((
                now + f64::from(at),
                name,
                f64::from(pitch),
                f64::from(volume),
            )),
            BossCue::LoopStart {
                key,
                name,
                at,
                max_dur,
            } => {
                // JS `_loop` replaces a live loop of the same key (L38-42).
                bar.stop_loop(key, &mut commands);
                let due = now + f64::from(at);
                bar.pending_loops
                    .push((due, key, name, due + f64::from(max_dur)));
            }
            BossCue::LoopStop { key } => {
                bar.pending_loops.retain(|(_, k, _, _)| *k != key);
                bar.stop_loop(key, &mut commands);
            }
            BossCue::StopAll => bar.clear(&mut commands),
            BossCue::Music(t) => match t {
                // JS `music(track)` → `play(track, {fade: 1})`; `music(None)`
                // → `play(null, {fade})` (0.5 defeat / 0.6 finish — the table
                // loses the split, the finish value is used).
                Some(id) => rt.request(Some(id), Some(1.0), now),
                None => rt.request(None, Some(0.6), now),
            },
            BossCue::Duck { gain, dur } => rt.duck(f64::from(gain), f64::from(dur), now),
        }
    }
    // Delayed one-shots whose due has arrived ≡ `A()?.play(name, o)` — the
    // same gate + spawn path as `audio_sfx` (deviation: while `M`-muted the
    // JS bus still plays them silently; here they are dropped).
    let mut ready: Vec<(&'static str, f64, f64)> = Vec::new();
    bar.pending.retain(|(due, name, pitch, volume)| {
        if now >= *due {
            ready.push((*name, *pitch, *volume));
            false
        } else {
            true
        }
    });
    if !ready.is_empty() && !snd.muted {
        for (name, pitch, volume) in ready {
            let Some((handle, dur)) = sfx.cue(&mut pcm, name) else {
                continue;
            };
            let Some(PlayDecision::Spawned { speed, volume, .. }) =
                snd.gate.play(name, now, None, 0.0, volume, pitch, dur)
            else {
                continue;
            };
            let v = (volume * vols.sfx.value_at(now)).clamp(0.0, 1.0) as f32;
            commands.spawn((
                AudioPlayer(handle),
                PlaybackSettings::DESPAWN
                    .with_volume(Volume::Linear(v))
                    .with_speed(speed as f32),
                SfxBorn((now + dur / speed + f64::from(super::GC_SLACK)) as f32),
            ));
        }
    }
    // Delayed loops whose due has arrived: bake the clean loop segment
    // (≡ the `oneShot`-free loop entries of the SFX table) and run it until
    // `LoopStop` / the `maxDur` watchdog.
    let mut spawn_loops: Vec<(&'static str, &'static str, f64)> = Vec::new();
    bar.pending_loops.retain(|(due, key, name, deadline)| {
        if now >= *due {
            spawn_loops.push((*key, *name, *deadline));
            false
        } else {
            true
        }
    });
    for (key, name, deadline) in spawn_loops {
        bar.stop_loop(key, &mut commands);
        let Some(samples) = sfx_table::bake_loop(name) else {
            continue;
        };
        let h = pcm.add(PcmSound(Pcm {
            sample_rate: SR,
            data: samples,
        }));
        let v = vols.sfx.value_at(now).clamp(0.0, 1.0) as f32;
        let e = commands
            .spawn((
                AudioPlayer(h),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(v)),
            ))
            .id();
        bar.loops.push(LiveLoop {
            key,
            entity: e,
            deadline,
        });
    }
    // `maxDur` watchdog (audio.js L237).
    let due: Vec<usize> = bar
        .loops
        .iter()
        .enumerate()
        .filter(|(_, l)| now >= l.deadline)
        .map(|(i, _)| i)
        .collect();
    for i in due.into_iter().rev() {
        let l = bar.loops.remove(i);
        despawn(&mut commands, l.entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::music::MusicAction;
    use bevy::MinimalPlugins;
    use bevy::audio::{AddAudioSource, AudioPlugin, GlobalVolume};

    fn boss_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            AudioPlugin::default(),
        ))
        .add_audio_source::<PcmSound>()
        .init_resource::<GlobalVolume>()
        .init_resource::<SoundState>()
        .init_resource::<SfxAssets>()
        .init_resource::<AudioVolumes>()
        .init_resource::<MusicRuntime>()
        .init_resource::<BossAudioRuntime>()
        .init_resource::<BossHudRes>();
        app.add_systems(Update, boss_audio_playback);
        app
    }

    fn push(hud: &mut BossHudRes, cue: BossCue) {
        hud.audio.cues.push(cue);
    }

    fn audio_entities(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<(), With<AudioPlayer<PcmSound>>>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn music_and_duck_cues_drive_the_runtime() {
        let mut app = boss_app();
        app.update();
        push(
            &mut app.world_mut().resource_mut::<BossHudRes>(),
            BossCue::Music(Some("boss")),
        );
        push(
            &mut app.world_mut().resource_mut::<BossHudRes>(),
            BossCue::Duck {
                gain: 0.5,
                dur: 1.0,
            },
        );
        app.update();
        let rt = app.world().resource::<MusicRuntime>();
        assert_eq!(rt.state.current.map(|c| c.track), Some("boss"));
        // The duck hold region is `now + 0.08 .. now + 1.08`.
        let t = app.world().resource::<Time>().elapsed_secs() as f64;
        assert!((rt.duck.value_at(t + 0.5) - 0.5).abs() < 0.01);
        // `music(null)` stops the set (JS finish/defeat; fade collapses to 0.6).
        push(
            &mut app.world_mut().resource_mut::<BossHudRes>(),
            BossCue::Music(None),
        );
        app.update();
        assert_eq!(app.world().resource::<MusicRuntime>().state.current, None);
    }

    #[test]
    fn sfx_and_loop_cues_spawn_and_despawn_players() {
        let mut app = boss_app();
        app.update();
        {
            let mut hud = app.world_mut().resource_mut::<BossHudRes>();
            push(
                &mut hud,
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 0.0,
                    pitch: 1.0,
                    volume: 0.9,
                },
            );
            push(
                &mut hud,
                BossCue::LoopStart {
                    key: "gallop",
                    name: "boss_gallop",
                    at: 0.0,
                    max_dur: 6.0,
                },
            );
        }
        app.update(); // consume + queue spawns
        app.update(); // apply the spawn commands
        assert_eq!(audio_entities(&mut app), 2, "one-shot + loop");
        {
            let mut hud = app.world_mut().resource_mut::<BossHudRes>();
            push(&mut hud, BossCue::LoopStop { key: "gallop" });
        }
        app.update();
        app.update();
        let loops = app.world().resource::<BossAudioRuntime>().loops.len();
        assert_eq!(loops, 0, "LoopStop despawns the loop");
    }

    #[test]
    fn stop_all_clears_pending_and_loops() {
        let mut app = boss_app();
        app.update();
        {
            let mut hud = app.world_mut().resource_mut::<BossHudRes>();
            push(
                &mut hud,
                BossCue::Sfx {
                    name: "boss_roar",
                    at: 5.0, // still pending when StopAll lands
                    pitch: 1.0,
                    volume: 0.9,
                },
            );
            push(
                &mut hud,
                BossCue::LoopStart {
                    key: "dizzy",
                    name: "boss_dizzy",
                    at: 0.0,
                    max_dur: 4.0,
                },
            );
            push(&mut hud, BossCue::StopAll);
        }
        app.update();
        app.update();
        let bar = app.world().resource::<BossAudioRuntime>();
        assert!(bar.pending.is_empty());
        assert!(bar.pending_loops.is_empty());
        assert!(bar.loops.is_empty());
        assert_eq!(audio_entities(&mut app), 0);
    }

    #[test]
    fn director_state_syncs_into_the_remap() {
        let mut app = boss_app();
        app.update();
        app.world_mut().resource_mut::<BossHudRes>().audio.begin();
        app.update();
        let rt = app.world().resource::<MusicRuntime>();
        assert!(rt.state.remap.active);
        // While active, a generic battle request lands on the boss track.
        let a = rt.state.clone().play(Some("battle"), Some(1.0), 0.0);
        assert!(matches!(a[0], MusicAction::Start { track: "boss", .. }));
    }
}
