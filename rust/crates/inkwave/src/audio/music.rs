//! Task 50 batch C — the procedural music engine, ported from
//! `src/audio/music.js`.
//!
//! Every track is synthesized: the grid-string score DSL (`compileSong`,
//! music.js L601-777) compiles a [`SongDef`] into per-step [`Ev`]ent lists,
//! and [`bake_song`] renders the arrangement into one looping PCM buffer
//! through the ported WebAudio graph ([`crate::audio::graph`]). The track
//! set is the nine JS ids of `SONGS` (music.js L818-1062): `title | menu |
//! battle | battle_final | results_win | results_lose | boss | boss_2 |
//! boss_3` (the JS header comment lists six, the table has nine; all nine
//! are ported).
//!
//! Deviations (registered in the batch report):
//!   - **file-backed tracks: no assets.** Upstream `songs/manifest.json`
//!     (built by `npm run music`) would swap real recordings in for
//!     `battle` / `battle_final`; this repo ships no `songs/` directory, so
//!     every id keeps its synthesized score. [`has_file`] is the declared
//!     placeholder; the JS `preload`/`pause`/`resume`/`setOvertime` surface
//!     is not implemented on the upstream singleton either (the
//!     optional-chained call sites in main.js/hud.js are no-ops).
//!   - **live bus not baked**: the JS chain
//!     groups→pumpG→fades→mix→hp30→comp→out(0.42)→musicBus is baked only
//!     up to the static gains (`MIX[g] × 0.42`); the highpass + compressor
//!     are replaced by the runtime bus volumes (batch B precedent). The
//!     kick side-chain pump *is* baked ([`PumpCurve`]).
//!   - **delay/reverb sends** (`SEND_DLY`/`SEND_REV`, music.js L1069-1070)
//!     are live buses upstream; dry-only here.
//!   - **loop point**: JS loops from `loopFrom` (skipping the intro on
//!     repeat); the baked buffer loops as a whole, so the intro re-plays.
//!     Bar-synced switching keeps its semantics through the state machine
//!     ([`MusicState::play`]).
//!   - **escalation riser** (the `play()` crossfade riser of L1402-1406)
//!     is a live one-shot built with the *new* player's rng; the Rust
//!     state machine records the intent ([`MusicAction::riser`]) but plays
//!     no audio for it. Per-song `riser` score fx *are* baked.
//!   - **per-note rendering**: JS builds one live graph; the bake renders
//!     each note through its own tiny [`Vb`] and overlap-adds it —
//!     equivalent because every JS instrument already creates fresh nodes
//!     per note (the only cross-note couplings, open-hat cutoff and pump,
//!     are reproduced explicitly).
//!   - **setIntensity**: the live layer-weight ladder (LAYERS) always
//!     evaluates to 1.0 at intensity 1; the Rust port has no live layer
//!     control, so [`layer_weight`] is fixed at 1.0.

use std::collections::HashMap;

use bevy::prelude::*;

use bevy::audio::{AudioPlayer, AudioSink, PlaybackSettings, Volume};

use super::graph::{NodeId, Nz, ParamRef, Tone, Vb};
use super::sfx_table;
use super::synth::{self, BqKind, NoiseKind, Param, Rng, Wave};
use super::{Pcm, PcmSound};
use crate::ink_render::DemoSim;
use crate::ui::{Screen, UiState};
use inkwave_sim::match_::{MatchEvent, MatchMode, Phase};

// ---------------------------------------------------------------------------
// Track ids
// ---------------------------------------------------------------------------

/// One of the nine JS track ids (music.js `SONGS`).
pub type TrackId = &'static str;

/// Every playable track id, in `SONGS` order.
pub const TRACKS: &[TrackId] = &[
    "title",
    "menu",
    "battle",
    "battle_final",
    "results_win",
    "results_lose",
    "boss",
    "boss_2",
    "boss_3",
];

/// ≡ `!!SONGS[id]` — the unknown-id guard of `play()` (L1384).
#[must_use]
pub fn is_track(id: TrackId) -> bool {
    TRACKS.contains(&id)
}

/// File-backed tracks (music.js L1342-1352 + `songs/manifest.json`): this
/// build ships no `songs/` assets, so no track has a recording and every
/// id keeps its synthesized score (declared placeholder).
#[must_use]
pub fn has_file(_id: TrackId) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Score data model
// ---------------------------------------------------------------------------

/// A grid pattern: one string for every bar, or a per-bar list (cycled).
#[derive(Clone, Debug)]
pub enum Pat {
    One(&'static str),
    Bars(Vec<&'static str>),
}

impl Pat {
    #[must_use]
    fn at(&self, b: usize) -> &'static str {
        match self {
            Pat::One(s) => s,
            Pat::Bars(v) => v[b % v.len()],
        }
    }
}

/// `sec.arp = {rate, pattern, oct, lo}` (music.js L848 etc).
#[derive(Clone, Copy, Debug)]
pub struct ArpSpec {
    pub rate: u32,
    pub pattern: &'static str,
    pub oct: u32,
    pub lo: u32,
}

/// One section of the score grid (music.js `sections` entries).
#[derive(Clone, Debug, Default)]
pub struct SectionDef {
    pub bars: u32,
    pub chords: Vec<&'static str>,
    /// Instrument key (`k s c h x p sh t`) → grid.
    pub drums: Vec<(&'static str, Pat)>,
    pub fills: Vec<(&'static str, &'static str)>,
    pub bass: Option<Pat>,
    pub stabs: Option<Pat>,
    pub pad: bool,
    pub arp: Option<ArpSpec>,
    pub lead: Option<&'static str>,
    pub lead2: Option<&'static str>,
    pub riff: Option<&'static str>,
    pub harmony: bool,
    pub crash: bool,
    /// `sec.riser` — trailing bars of riser fx (0 = none).
    pub riser: u32,
}

impl SectionDef {
    fn new(bars: u32, chords: &[&'static str]) -> Self {
        SectionDef {
            bars,
            chords: chords.to_vec(),
            ..Default::default()
        }
    }
    fn drum(mut self, key: &'static str, grid: &'static str) -> Self {
        self.drums.push((key, Pat::One(grid)));
        self
    }
    fn drum_bars(mut self, key: &'static str, grids: &[&'static str]) -> Self {
        self.drums.push((key, Pat::Bars(grids.to_vec())));
        self
    }
    fn fill(mut self, key: &'static str, grid: &'static str) -> Self {
        self.fills.push((key, grid));
        self
    }
    fn bass(mut self, grid: &'static str) -> Self {
        self.bass = Some(Pat::One(grid));
        self
    }
    fn bass_bars(mut self, grids: &[&'static str]) -> Self {
        self.bass = Some(Pat::Bars(grids.to_vec()));
        self
    }
    fn stabs(mut self, grid: &'static str) -> Self {
        self.stabs = Some(Pat::One(grid));
        self
    }
    fn stabs_bars(mut self, grids: &[&'static str]) -> Self {
        self.stabs = Some(Pat::Bars(grids.to_vec()));
        self
    }
    fn arp(mut self, rate: u32, pattern: &'static str, oct: u32, lo: u32) -> Self {
        self.arp = Some(ArpSpec {
            rate,
            pattern,
            oct,
            lo,
        });
        self
    }
    fn with_pad(mut self) -> Self {
        self.pad = true;
        self
    }
    fn lead(mut self, p: &'static str) -> Self {
        self.lead = Some(p);
        self
    }
    fn riff(mut self, p: &'static str) -> Self {
        self.riff = Some(p);
        self
    }
    fn harmony(mut self) -> Self {
        self.harmony = true;
        self
    }
    fn crash(mut self) -> Self {
        self.crash = true;
        self
    }
    fn riser(mut self, n: u32) -> Self {
        self.riser = n;
        self
    }
}

/// A full song definition (music.js `SONGS[id]`).
#[derive(Clone, Debug)]
pub struct SongDef {
    pub id: TrackId,
    pub name: &'static str,
    pub bpm: f64,
    pub swing: f64,
    pub pump: f64,
    pub inst_bass: &'static str,
    pub inst_chords: &'static str,
    pub inst_lead: &'static str,
    pub inst_lead2: Option<&'static str>,
    /// Data parity with `inst.arp` (music.js); `fire()`'s arp case is a
    /// fixed pluck, so the style string is never read.
    #[allow(dead_code)]
    pub inst_arp: &'static str,
    pub riff_bass: bool,
    pub keys_lo: u32,
    pub bass_lo: u32,
    pub guitar_lo: u32,
    pub pad_lo: u32,
    /// Per-group mix overrides on top of [`MIX`].
    pub mix: Vec<(&'static str, f64)>,
    pub sections: Vec<(&'static str, SectionDef)>,
    pub order: Vec<&'static str>,
    /// `loopFrom: order.indexOf(...)`; `None` ≡ JS `0` (whole song).
    pub loop_from: Option<usize>,
}

impl SongDef {
    /// JS defaults: bass `bass`, chords `keys`, lead `lead`, lead2 none,
    /// arp `pluck`, lo 28/55/40/52 (music.js L672/L700/L704/L710).
    fn base(id: TrackId, name: &'static str, bpm: f64) -> Self {
        SongDef {
            id,
            name,
            bpm,
            swing: 0.0,
            pump: 0.0,
            inst_bass: "bass",
            inst_chords: "keys",
            inst_lead: "lead",
            inst_lead2: None,
            inst_arp: "pluck",
            riff_bass: false,
            keys_lo: 55,
            bass_lo: 28,
            guitar_lo: 40,
            pad_lo: 52,
            mix: Vec::new(),
            sections: Vec::new(),
            order: Vec::new(),
            loop_from: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Lead/riff phrase constants (music.js L783-816) — verbatim
// ---------------------------------------------------------------------------

const BATTLE_A_LEAD: &str = "B4/2 E5/2 r/1 E5/1 G5/2 F#5/2 E5/2 D5/2 B4/2 | E5/3 G5/3 B5/2 A5/2 G5/2 E5/4 | r/2 A4/1 C5/1 E5/2 A5/2 G5/2 E5/2 C5/2 D5/2 | D#5/3 F#5/3 A5/2 B5/6 r/2 | B4/2 E5/2 r/1 E5/1 G5/2 F#5/2 E5/2 D5/2 B4/2 | E5/3 G5/3 B5/2 A5/2 G5/2 E5/4 | r/2 A4/1 C5/1 E5/2 A5/2 G5/2 E5/2 C5/2 D5/2 | F#5/2 E5/2 D#5/2 B4/2 D#5/4 F#5/4";
const BATTLE_B_LEAD: &str = "G5/4 E5/2 G5/2 A5/4 G5/4 | F#5/4 D5/2 F#5/2 A5/6 r/2 | B5/3 A5/3 F#5/2 D5/4 F#5/4 | G5/6 F#5/2 E5/8 | G5/4 E5/2 G5/2 C6/4 B5/4 | A5/4 F#5/2 A5/2 D6/6 r/2 | E6/4 D6/4 B5/4 A5/4 | A5/2 G5/2 F#5/2 D#5/2 F#5/8";
const TITLE_RIFF: &str =
    "D3/2 r/1 D3/1 F3/2 D3/1 G3/2 Ab3/1 G3/2 F3/2 D3/2 | C4/2 D4/2 r/2 A3/1 C4/1 D4/4 r/2 F3/2";
const TITLE_LEAD: &str = "D5/2 F5/2 G5/3 F5/1 D5/2 C5/2 Bb4/4 | D5/2 F5/2 Bb5/4 A5/2 F5/2 D5/2 C5/2 | C5/4 A4/2 C5/2 F5/4 E5/4 | E5/3 C#5/3 A4/2 G5/4 E5/4 | D5/2 F5/2 G5/3 F5/1 D5/2 C5/2 Bb4/4 | D5/2 F5/2 Bb5/4 A5/2 F5/2 D5/2 C5/2 | C5/4 A4/2 C5/2 F5/4 E5/4 | A4/2 C#5/2 E5/2 G5/2 A5/8";
const MENU_A_LEAD: &str = "A5/3 G5/1 F5/2 E5/2 F5/4 r/4 | E5/2 G5/2 C6/3 D6/1 C6/2 A5/2 r/4 | D6/3 C6/1 A5/2 F5/2 A5/4 G5/4 | F5/2 G5/2 Bb5/4 A5/2 G5/2 E5/4 | A5/3 G5/1 F5/2 E5/2 F5/4 r/4 | E5/2 G5/2 C6/3 D6/1 C6/2 A5/2 r/4 | D6/3 C6/1 A5/2 F5/2 A5/2 C6/2 Bb5/2 G5/2 | G5/4 F5/4 E5/8";
const MENU_B_LEAD: &str = "F5/2 A5/2 C6/2 A5/2 G5/4 F5/4 | D5/2 F5/2 Bb5/3 A5/1 G5/4 r/4 | F5/2 A5/2 D6/4 C6/2 A5/2 F5/4 | E5/4 G5/4 Bb5/4 A5/4 | F5/2 A5/2 C6/2 A5/2 G5/4 F5/4 | D5/2 F5/2 Bb5/3 A5/1 G5/4 r/4 | F5/2 A5/2 D6/4 E5/2 G5/2 Bb5/4 | A5/12 r/4";
const WIN_LEAD: &str = "D5/2 G5/2 B5/2 A5/2 G5/4 D5/4 | E5/2 G5/2 B5/2 D6/2 B5/8 | C6/3 B5/1 G5/2 E5/2 G5/4 D5/4 | F#5/2 A5/2 D6/4 C6/2 A5/2 F#5/4 | D5/2 G5/2 B5/2 A5/2 G5/4 D5/4 | F#5/2 B5/2 D6/2 F#6/2 D6/8 | E6/3 D6/1 B5/2 G5/2 C6/4 B5/4 | A5/4 G5/4 A5/2 B5/2 A5/2 F#5/2";
const LOSE_LEAD: &str = "E5/6 D5/2 C5/4 B4/4 | C5/6 A4/2 C5/4 E5/4 | F5/6 E5/2 D5/4 C5/4 | B4/8 G#4/8 | E5/6 D5/2 C5/4 B4/4 | C5/6 A4/2 C5/4 E5/4 | F5/4 A5/4 G5/4 F5/4 | E5/12 r/4";
const BOSS_RIFF: &str = "D3/2 D3/1 D3/1 F3/2 D3/2 E3/1 F3/1 A3/2 G#3/2 G3/2 | D3/2 D3/1 D3/1 F3/2 D3/2 C4/2 A3/2 Bb3/2 A3/2";
const BOSS_RIFF3: &str = "D3/1 D3/1 Eb3/2 D3/1 D3/1 A3/2 Ab3/2 D3/1 D3/1 C4/2 Bb3/2 | D3/1 D3/1 Eb3/2 D3/1 D3/1 F3/2 E3/2 Eb3/2 D3/4";
const BOSS_A_LEAD: &str = "D5/3 F5/3 A5/2 G5/2 F5/2 E5/2 C5/2 | D5/4 Bb4/4 F5/4 D5/4 | E5/3 G5/3 C6/2 Bb5/2 A5/2 G5/4 | A5/6 G5/2 E5/2 C#5/2 E5/4 | D5/3 F5/3 A5/2 G5/2 F5/2 E5/2 C5/2 | D5/4 Bb4/4 F5/4 Bb5/4 | C6/3 Bb5/3 A5/2 G5/2 E5/2 C5/4 | C#5/4 E5/4 A5/4 C#6/4";
const BOSS_B_LEAD: &str = "G5/4 Bb5/4 A5/2 G5/2 F5/4 | A5/6 F5/2 D5/8 | Eb5/2 G5/2 Bb5/4 C6/2 Bb5/2 G5/4 | A5/4 C#6/4 E6/4 A5/4 | G5/4 Bb5/4 D6/2 C6/2 Bb5/4 | A5/6 F5/2 D5/4 F5/4 | G5/2 Bb5/2 Eb6/4 D6/2 C6/2 Bb5/4 | A5/2 G5/2 F5/2 E5/2 C#5/4 E5/4";
const BOSS_C_LEAD: &str = "D6/2 C6/2 A5/2 F5/2 A5/4 D6/4 | Eb6/3 D6/3 Bb5/2 G5/4 Bb5/4 | A5/2 F5/2 D5/2 F5/2 A5/2 D6/2 F6/4 | E6/4 C#6/4 Bb5/4 G5/4 | D6/2 C6/2 A5/2 F5/2 A5/4 D6/4 | Eb6/3 G6/3 F6/2 Eb6/4 D6/4 | C6/2 A5/2 F5/2 A5/2 D6/4 C#6/4 | E6/6 D6/2 C#6/8";

// ---------------------------------------------------------------------------
// The score table — `SONGS` (music.js L818-1062), verbatim grids
// ---------------------------------------------------------------------------

/// ≡ `SONGS[id]` (fresh copy per call; the bake caches what it needs).
#[must_use]
pub fn song_def(id: TrackId) -> Option<SongDef> {
    let d = match id {
        "title" => {
            // music.js L819-851: 'Splash Attitude'
            let mut s = SongDef::base("title", "Splash Attitude", 128.0);
            s.swing = 0.05;
            s.pump = 0.22;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "saw";
            s.riff_bass = true;
            s.sections = vec![
                (
                    "intro",
                    SectionDef::new(4, &["Dm"])
                        .riff(TITLE_RIFF)
                        .riser(1)
                        .drum_bars(
                            "k",
                            &[
                                "................",
                                "................",
                                "X.......X.......",
                                "X.......X.......",
                            ],
                        )
                        .drum_bars(
                            "h",
                            &[
                                "................",
                                "................",
                                "x.c.x.c.x.c.x.c.",
                                "x.c.x.c.x.c.x.c.",
                            ],
                        )
                        .drum_bars(
                            "s",
                            &[
                                "................",
                                "................",
                                "................",
                                "x.x.x.x.xxxxXXXX",
                            ],
                        ),
                ),
                (
                    "A",
                    SectionDef::new(8, &["Dm"])
                        .crash()
                        .riff(TITLE_RIFF)
                        .drum("k", "X.....x.X.x.....")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "x.c.x.c.x.c.x.oc")
                        .fill("s", "....X.......X.xx"),
                ),
                (
                    "B",
                    SectionDef::new(8, &["Gm7", "Bb", "F/A", "A7"])
                        .crash()
                        .with_pad()
                        .lead(TITLE_LEAD)
                        .drum("k", "X.....x.X.x...x.")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "x.o.x.o.x.o.x.o.")
                        .bass("R.RR..O.R.RR..5n")
                        .stabs("X--.x-..X--.x-x."),
                ),
                (
                    "break",
                    SectionDef::new(4, &["Dm", "Bb", "C", "A7"])
                        .with_pad()
                        .riser(1)
                        .drum("k", "X.......X.......")
                        .drum("h", "x.x.x.x.x.x.x.x.")
                        .fill("s", "x.x.x.x.xxxxXXXX")
                        .bass("R-------R---5---")
                        .arp(1, "updown", 2, 62),
                ),
            ];
            s.order = vec!["intro", "A", "B", "A", "break", "B"];
            s.loop_from = Some(1);
            s
        }
        "menu" => {
            // music.js L853-869: 'Harbor Lounge'
            let mut s = SongDef::base("menu", "Harbor Lounge", 105.0);
            s.swing = 0.16;
            s.pump = 0.0;
            s.inst_bass = "funk";
            s.inst_chords = "keys";
            s.inst_lead = "soft";
            s.keys_lo = 57;
            s.bass_lo = 31;
            s.mix = vec![
                ("kick", 0.5),
                ("snare", 0.3),
                ("hats", 0.14),
                ("bass", 0.38),
                ("chords", 0.32),
                ("pad", 0.07),
                ("lead", 0.26),
                ("arp", 0.08),
            ];
            s.sections = vec![
                (
                    "intro",
                    SectionDef::new(2, &["Fmaj7", "C7sus4"])
                        .stabs_bars(&["X-------------..", "X-------x-----.."])
                        .drum("sh", "..x...x...x...x."),
                ),
                (
                    "A",
                    SectionDef::new(8, &["Fmaj7", "Am7", "Bbmaj7", "C7sus4 C7"])
                        .lead(MENU_A_LEAD)
                        .drum("k", "X......x..X.....")
                        .drum("s", "....x.......x...")
                        .drum("p", "..........g....g")
                        .drum("h", "x.c.x.c.x.c.x.cc")
                        .drum("sh", "xcxcxcxcxcxcxcxc")
                        .bass_bars(&["R..r.R5.O..R.53.", "R..r.R5.O..R.5.n"])
                        .stabs("X--..x-..x--.x-."),
                ),
                (
                    "B",
                    SectionDef::new(
                        8,
                        &[
                            "Dm7",
                            "Gm7",
                            "Bbmaj7",
                            "C9",
                            "Dm7",
                            "Gm7",
                            "Bbmaj7 C7",
                            "Fmaj7",
                        ],
                    )
                    .lead(MENU_B_LEAD)
                    .with_pad()
                    .drum("k", "X......x..X..x..")
                    .drum("s", "....x.......x...")
                    .drum("h", "x.o.x.c.x.o.x.cc")
                    .drum("sh", "xcxcxcxcxcxcxcxc")
                    .bass("R..r.R5.O..R.5.n")
                    .stabs("X--..x-..x--.x-."),
                ),
            ];
            s.order = vec!["intro", "A", "B", "A"];
            s.loop_from = Some(1);
            s
        }
        "battle" => {
            // music.js L871-905: 'Turf Riot'
            let mut s = SongDef::base("battle", "Turf Riot", 150.0);
            s.pump = 0.28;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "pulse";
            s.inst_lead2 = Some("saw");
            s.sections = vec![
                (
                    "intro",
                    SectionDef::new(2, &["E5", "C5 D5"])
                        .riser(1)
                        .drum_bars("k", &["X...............", "X.......X......."])
                        .drum_bars("s", &["................", "x.x.x.x.xxxxXXXX"])
                        .drum_bars("x", &["X...............", "................"])
                        .bass_bars(&["R---------------", "R-------R-------"])
                        .stabs_bars(&["X---------------", "X-------X-------"]),
                ),
                (
                    "A",
                    SectionDef::new(8, &["Em7", "Cmaj7", "Am7", "B7"])
                        .crash()
                        .lead(BATTLE_A_LEAD)
                        .drum("k", "X..x..x.X.....x.")
                        .drum("s", "....X.....g.X..g")
                        .drum("h", "x.c.x.c.x.c.x.oc")
                        .fill("s", "....X.....g.X.xx")
                        .bass_bars(&["R.RO.RR.5.RO.R7.", "R.RO.RR.5.RO.5On"])
                        .stabs("X-.x..x-.x..x-.x")
                        .arp(1, "up", 1, 64),
                ),
                (
                    "B",
                    SectionDef::new(8, &["C", "D", "Bm", "Em", "C", "D", "B7sus4", "B7"])
                        .crash()
                        .lead(BATTLE_B_LEAD)
                        .with_pad()
                        .drum("k", "X...x...X...x...")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "x.o.x.o.x.o.x.o.")
                        .fill("s", "....X.......X.xx")
                        .fill("k", "X...x...X...x.x.")
                        .bass("R.R.O.R.R.R.O.Rn")
                        .stabs("X-x-x-x-X-x-x-x-")
                        .arp(2, "updown", 2, 64),
                ),
                (
                    "break",
                    SectionDef::new(4, &["Am7", "Bm7", "Cmaj7", "D"])
                        .with_pad()
                        .riser(1)
                        .drum("k", "X.........x.....")
                        .drum("s", "........X.......")
                        .drum("h", "x.c.x.c.x.c.x.c.")
                        .fill("s", "x.x.x.x.xxxxXXXX")
                        .fill("k", "X...X...X...X...")
                        .bass("R-----R-5-----O-")
                        .arp(1, "updown", 2, 60),
                ),
            ];
            s.order = vec!["intro", "A", "B", "A", "break", "B"];
            s.loop_from = Some(1);
            s
        }
        "battle_final" => {
            // music.js L907-950: 'Final Splash'
            let mut s = SongDef::base("battle_final", "Final Splash", 150.0);
            s.pump = 0.32;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "pulse";
            s.inst_lead2 = Some("saw");
            s.mix = vec![("hats", 0.17), ("lead2", 0.16)];
            s.sections = vec![
                (
                    "lift",
                    SectionDef::new(1, &["B5"])
                        .riser(1)
                        .drum("k", "X...X...X...X...")
                        .drum("s", "x.x.x.x.xxxxXXXX")
                        .bass("R-----R-----R-R-")
                        .stabs("X-----X-----X-X-"),
                ),
                (
                    "F1",
                    SectionDef::new(8, &["C", "D", "Bm", "Em", "C", "D", "B7sus4", "B7"])
                        .crash()
                        .lead(BATTLE_B_LEAD)
                        .harmony()
                        .drum("k", "X...x...X...x...")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "xcxcxcxcxcxcxcxc")
                        .fill("s", "....X.....x.X.xx")
                        .bass("RrOrRrOrRrOrRrOn")
                        .stabs("X-x-x-x-X-x-x-xx")
                        .arp(1, "up", 2, 64),
                ),
                (
                    "F2",
                    SectionDef::new(8, &["Em7", "Cmaj7", "Am7", "B7"])
                        .crash()
                        .lead(BATTLE_A_LEAD)
                        .harmony()
                        .drum("k", "X..x..x.X..x..x.")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "xcxcxcxoxcxcxcxo")
                        .bass("R.ROR.RO5.5OR.On")
                        .stabs("X-.x..x-.x..x-.x")
                        .arp(1, "updown", 2, 64),
                ),
                (
                    "build",
                    SectionDef::new(4, &["C", "D", "Em", "B7"])
                        .riser(4)
                        .lead("G5/8 E5/8 | A5/8 F#5/8 | B5/8 G5/8 | D#6/4 B5/4 F#5/4 A5/4")
                        .drum("k", "X...X...X...X...")
                        .drum("h", "xcxcxcxcxcxcxcxc")
                        .drum_bars(
                            "s",
                            &[
                                "....X.......X...",
                                "x...x...x...x...",
                                "x.x.x.x.x.x.x.x.",
                                "xxxxxxxxXXXXXXXX",
                            ],
                        )
                        .bass("R.R.R.R.R.R.R.R.")
                        .stabs("X-x-x-x-X-x-x-x-"),
                ),
            ];
            s.order = vec!["lift", "F1", "F2", "build"];
            s.loop_from = Some(1);
            s
        }
        "results_win" => {
            // music.js L952-966: 'Fresh Victory'
            let mut s = SongDef::base("results_win", "Fresh Victory", 124.0);
            s.pump = 0.18;
            s.inst_bass = "funk";
            s.inst_chords = "pluck";
            s.inst_lead = "bell";
            s.keys_lo = 59;
            s.bass_lo = 31;
            s.mix = vec![
                ("lead", 0.22),
                ("chords", 0.2),
                ("arp", 0.08),
                ("hats", 0.16),
            ];
            s.sections = vec![(
                "A",
                SectionDef::new(8, &["G", "Em7", "Cadd9", "D", "G", "Bm7", "Cadd9", "Am7 D"])
                    .crash()
                    .lead(WIN_LEAD)
                    .drum("k", "X...x...X...x...")
                    .drum("c", "....x.......x...")
                    .drum("h", "c.o.c.o.c.o.c.o.")
                    .drum("sh", "xcxcxcxcxcxcxcxc")
                    .bass("R.O.R.O.5.O.R.On")
                    .stabs("..x...x...x...x.")
                    .arp(2, "up", 2, 67),
            )];
            s.order = vec!["A"];
            s.loop_from = None; // 0 — whole song
            s
        }
        "results_lose" => {
            // music.js L968-980: 'Next Time'
            let mut s = SongDef::base("results_lose", "Next Time", 84.0);
            s.swing = 0.1;
            s.pump = 0.0;
            s.inst_bass = "sub";
            s.inst_chords = "keys";
            s.inst_lead = "soft";
            s.mix = vec![
                ("kick", 0.42),
                ("hats", 0.1),
                ("bass", 0.4),
                ("chords", 0.36),
                ("lead", 0.22),
            ];
            s.sections = vec![(
                "A",
                SectionDef::new(
                    8,
                    &[
                        "Am7",
                        "Fmaj7",
                        "Dm7",
                        "Esus4 E7",
                        "Am7",
                        "Fmaj7",
                        "Dm9",
                        "E7sus4 E7",
                    ],
                )
                .lead(LOSE_LEAD)
                .drum("k", "X.........x.....")
                .drum("p", "........x.......")
                .drum("h", "c.c.c.c.c.c.c.c.")
                .bass("R-------..R-5---")
                .stabs("X-------..x-----"),
            )];
            s.order = vec!["A"];
            s.loop_from = None;
            s
        }
        "boss" => {
            // music.js L983-1007: 'Hull Alarm'
            let mut s = SongDef::base("boss", "Hull Alarm", 140.0);
            s.pump = 0.26;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "saw";
            s.inst_lead2 = Some("pulse");
            s.riff_bass = true;
            s.mix = vec![("lead", 0.26)];
            s.sections = vec![
                (
                    "intro",
                    SectionDef::new(2, &["D5"])
                        .riff(BOSS_RIFF)
                        .riser(1)
                        .drum_bars("k", &["X.......X.......", "X...X...X...X..."])
                        .drum_bars("s", &["................", "x.x.x.x.xxxxXXXX"])
                        .drum_bars("x", &["X...............", "................"]),
                ),
                (
                    "A",
                    SectionDef::new(8, &["D5"])
                        .crash()
                        .riff(BOSS_RIFF)
                        .drum("k", "X..X..X.X..X..x.")
                        .drum("s", "....X.......X...")
                        .drum("h", "x.c.x.c.x.c.x.oc")
                        .fill("s", "....X.....g.X.xx"),
                ),
                (
                    "B",
                    SectionDef::new(8, &["Dm", "Bb", "C", "A"])
                        .crash()
                        .lead(BOSS_A_LEAD)
                        .with_pad()
                        .drum("k", "X...X...X...X...")
                        .drum("s", "....X.......X...")
                        .drum("h", "x.o.x.o.x.o.x.o.")
                        .fill("s", "....X.....x.X.xx")
                        .bass("R.RR.RO.R.RR.R5n")
                        .stabs("X-.x..x-.x..x-.x"),
                ),
            ];
            s.order = vec!["intro", "A", "B", "A", "B"];
            s.loop_from = Some(1);
            s
        }
        "boss_2" => {
            // music.js L1009-1032: 'Brood Tide'
            let mut s = SongDef::base("boss_2", "Brood Tide", 140.0);
            s.pump = 0.3;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "saw";
            s.inst_lead2 = Some("pulse");
            s.riff_bass = true;
            s.mix = vec![("hats", 0.2), ("lead2", 0.15), ("arp", 0.12)];
            s.sections = vec![
                (
                    "lift",
                    SectionDef::new(1, &["A5"])
                        .riser(1)
                        .drum("k", "X...X...X...X...")
                        .drum("s", "x.x.x.x.xxxxXXXX")
                        .bass("R-----R-----R-R-")
                        .stabs("X-----X-----X-X-"),
                ),
                (
                    "A",
                    SectionDef::new(8, &["D5"])
                        .crash()
                        .riff(BOSS_RIFF)
                        .arp(1, "up", 2, 62)
                        .drum("k", "X.XX..X.X.XX..x.")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "xcxcxcxcxcxcxcxc")
                        .fill("s", "....X.....x.X.xx"),
                ),
                (
                    "B",
                    SectionDef::new(8, &["Gm", "Dm", "Eb", "A7"])
                        .crash()
                        .lead(BOSS_B_LEAD)
                        .harmony()
                        .with_pad()
                        .drum("k", "X.X.X.X.X.X.X.X.")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "x.o.x.o.x.o.x.o.")
                        .fill("s", "....X...x.x.XxXx")
                        .bass("RrOrRrOrRrOrRrOn")
                        .stabs("X-x-x-x-X-x-x-x-")
                        .arp(1, "updown", 2, 62),
                ),
            ];
            s.order = vec!["lift", "A", "B"];
            s.loop_from = Some(1);
            s
        }
        "boss_3" => {
            // music.js L1034-1060: 'Shell Shock'
            let mut s = SongDef::base("boss_3", "Shell Shock", 140.0);
            s.pump = 0.34;
            s.inst_bass = "punk";
            s.inst_chords = "guitar";
            s.inst_lead = "saw";
            s.inst_lead2 = Some("saw");
            s.riff_bass = true;
            s.mix = vec![("hats", 0.2), ("lead", 0.3), ("lead2", 0.16), ("arp", 0.12)];
            s.sections = vec![
                (
                    "lift",
                    SectionDef::new(1, &["Eb5"])
                        .riser(1)
                        .drum("k", "X.X.X.X.X.X.X.X.")
                        .drum("s", "xxxxxxxxXXXXXXXX")
                        .bass("R.R.R.R.R.R.R.R.")
                        .stabs("X-x-X-x-X-x-XxXx"),
                ),
                (
                    "A",
                    SectionDef::new(8, &["D5"])
                        .crash()
                        .riff(BOSS_RIFF3)
                        .arp(1, "updown", 2, 62)
                        .drum("k", "XxX.XxX.XxX.XxX.")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "xcxcxcxcxcxcxcxc")
                        .fill("s", "....X...xxxxXXXX"),
                ),
                (
                    "B",
                    SectionDef::new(8, &["Dm", "Eb", "Dm", "C#dim"])
                        .crash()
                        .lead(BOSS_C_LEAD)
                        .harmony()
                        .with_pad()
                        .drum("k", "X.XXX.XXX.XXX.XX")
                        .drum("s", "....X.......X...")
                        .drum("c", "....x.......x...")
                        .drum("h", "xoxoxoxoxoxoxoxo")
                        .fill("s", "x.x.x.x.xxxxXXXX")
                        .bass("RRORRROR5R5RRROn")
                        .stabs("X-x-x-X-x-x-X-xx")
                        .arp(1, "up", 2, 62),
                ),
            ];
            s.order = vec!["lift", "A", "B"];
            s.loop_from = Some(1);
            s
        }
        _ => return None,
    };
    Some(d)
}

// ---------------------------------------------------------------------------
// Groups / mix (music.js L300-309, L1066-1070)
// ---------------------------------------------------------------------------

/// A score event instrument id (`GROUP_OF` keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inst {
    Kick,
    Snare,
    Clap,
    Hat,
    Crash,
    Rim,
    Shaker,
    Tom,
    Bass,
    Chords,
    Pad,
    Arp,
    Lead,
    Lead2,
    Riff,
    Riser,
}

impl Inst {
    /// ≡ `GROUP_OF[e.i]` (music.js L1066-1070).
    #[must_use]
    pub fn group(self) -> &'static str {
        match self {
            Inst::Kick => "kick",
            Inst::Snare | Inst::Clap | Inst::Tom => "snare",
            Inst::Hat | Inst::Rim | Inst::Shaker => "hats",
            Inst::Crash | Inst::Riser => "fx",
            Inst::Bass => "bass",
            Inst::Chords | Inst::Riff => "chords",
            Inst::Pad => "pad",
            Inst::Arp => "arp",
            Inst::Lead => "lead",
            Inst::Lead2 => "lead2",
        }
    }
}

/// `MIX` defaults (music.js L300).
pub const MIX: &[(&str, f64)] = &[
    ("kick", 0.6),
    ("snare", 0.5),
    ("hats", 0.22),
    ("bass", 0.36),
    ("chords", 0.3),
    ("pad", 0.1),
    ("arp", 0.1),
    ("lead", 0.28),
    ("lead2", 0.13),
    ("fx", 0.22),
];

/// Groups routed through the kick pump (music.js `PUMPED` L309).
pub const PUMPED: &[&str] = &["chords", "pad", "arp"];

/// The `out` gain feeding the music bus (music.js L1312 area, level 0.42).
pub const BUS_OUT: f64 = 0.42;

/// `LAYERS` weights — every `[lo, hi]` band evaluates to `1.0` at
/// `intensity == 1` (`smooth(lo, hi, 1)`), so the baked mix is static
/// (deviation: `setIntensity` has no live layer control in the Rust port).
#[must_use]
pub fn layer_weight(_g: &str, _intensity: f64) -> f64 {
    1.0
}

// ---------------------------------------------------------------------------
// Harmony primitives (music.js L583-625)
// ---------------------------------------------------------------------------

fn pc_of_letter(c: u8) -> i32 {
    match c {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => -1,
    }
}

/// `QUAL` (music.js L584-588).
fn quality(q: &str) -> Option<&'static [i32]> {
    Some(match q {
        "" => &[0, 4, 7],
        "m" => &[0, 3, 7],
        "5" => &[0, 7],
        "6" => &[0, 4, 7, 9],
        "m6" => &[0, 3, 7, 9],
        "7" => &[0, 4, 7, 10],
        "m7" => &[0, 3, 7, 10],
        "maj7" => &[0, 4, 7, 11],
        "9" => &[0, 4, 7, 10, 14],
        "m9" => &[0, 3, 7, 10, 14],
        "maj9" => &[0, 4, 7, 11, 14],
        "add9" => &[0, 4, 7, 14],
        "sus2" => &[0, 2, 7],
        "sus4" => &[0, 5, 7],
        "7sus4" => &[0, 5, 7, 10],
        "dim" => &[0, 3, 6],
        "m7b5" => &[0, 3, 6, 10],
        "aug" => &[0, 4, 8],
        _ => return None,
    })
}

/// ≡ `parseChord` — `^([A-G][#b]?)([^/]*)(?:\/([A-G][#b]?))?$` (L597-606).
#[must_use]
pub fn parse_chord(sym: &'static str) -> Option<Chord> {
    let b = sym.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_uppercase() && b[0] >= b'A' && b[0] <= b'G') {
        return None;
    }
    let mut i = 1;
    if i < b.len() && (b[i] == b'#' || b[i] == b'b') {
        i += 1;
    }
    let root = (pc_of_letter(b[0])
        + if i > 1 && b[1] == b'#' {
            1
        } else if i > 1 && b[1] == b'b' {
            -1
        } else {
            0
        }
        + 12)
        % 12;
    let (qual, bass_pc) = if i < b.len() && b[i] == b'/' {
        let rest = &sym[i + 1..];
        let rb = rest.as_bytes();
        if rb.is_empty() || !(rb[0] >= b'A' && rb[0] <= b'G') {
            return None;
        }
        let acc = if rb.len() > 1 && (rb[1] == b'#' || rb[1] == b'b') {
            if rb.len() > 2 {
                return None;
            }
            (rb[1] == b'#') as i32 - (rb[1] == b'b') as i32
        } else {
            0
        };
        (&sym[i..i], (pc_of_letter(rb[0]) + acc + 12) % 12)
    } else {
        let q = &sym[i..];
        if q.contains('/') {
            return None;
        }
        (q, -1)
    };
    let iv = quality(qual)?;
    Some(Chord {
        sym,
        root,
        iv,
        bass: if bass_pc < 0 { root } else { bass_pc },
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chord {
    pub sym: &'static str,
    pub root: i32,
    pub iv: &'static [i32],
    pub bass: i32,
}

/// ≡ `parseNote` — `^([A-G])([#b]?)(-?\d)$` → `12*(oct+1)+PC+acc` (L590-595).
#[must_use]
pub fn parse_note(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0] >= b'A' && b[0] <= b'G') {
        return None;
    }
    let mut i = 1;
    let mut acc = 0;
    if i < b.len() && (b[i] == b'#' || b[i] == b'b') {
        acc = if b[i] == b'#' { 1 } else { -1 };
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if neg {
        i += 1;
    }
    let d0 = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == d0 || i > d0 + 2 || i != b.len() {
        return None;
    }
    let oct: i32 = s[d0..i].parse().ok()?;
    Some(12 * (if neg { -oct } else { oct } + 1) + pc_of_letter(b[0]) + acc)
}

/// ≡ `inRange(pc, lo)` (L607).
#[must_use]
fn in_range(pc: i32, lo: i32) -> i32 {
    lo + ((pc - lo) % 12 + 12) % 12
}

/// Close voicing of chord tones within `[lo, lo+span)` (L609-615).
fn voice(ch: &Chord, lo: i32, hi: i32) -> Vec<i32> {
    let r = in_range(ch.root, lo);
    let mut out: Vec<i32> = Vec::new();
    for &i in ch.iv {
        let mut n = r + i;
        while n > hi {
            n -= 12;
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.sort();
    out
}

/// ≡ `powerChord` (L616).
fn power_chord(ch: &Chord, lo: i32) -> Vec<i32> {
    let r = in_range(ch.root, lo);
    vec![r, r + 7, r + 12]
}

/// ≡ `harmBelow` (L617-623).
fn harm_below(m: i32, ch: &Chord) -> i32 {
    for d in 3..=9i32 {
        let pc = ((m - d) % 12 + 12) % 12;
        if ch.iv.iter().any(|&i| (ch.root + i) % 12 == pc) {
            return m - d;
        }
    }
    m - 12
}

// ---------------------------------------------------------------------------
// Phrase parsing (music.js L626-643)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhraseNote {
    pub s: u32,
    pub len: u32,
    pub m: i32,
    pub acc: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Phrase {
    pub ev: Vec<PhraseNote>,
    pub len: u32,
}

/// ≡ `parsePhrase` — tokens `name/len[!]`, `|` bar-lines, `r` rest, `_`
/// sustain (L626-643).
pub fn parse_phrase(str: &str, warn: &mut Vec<String>, label: &str) -> Phrase {
    let mut ev: Vec<PhraseNote> = Vec::new();
    let mut pos: u32 = 0;
    let mut last: Option<usize> = None;
    for tok in str.split_whitespace() {
        if tok == "|" {
            if !pos.is_multiple_of(16) {
                warn.push(format!(
                    "{label}: bar line at step {pos} (not a bar boundary)"
                ));
            }
            continue;
        }
        // ^(r|_|[A-G][#b]?-?\d)/(\d+)(!?)$
        let mut t = tok;
        let acc = t.ends_with('!');
        if acc {
            t = &t[..t.len() - 1];
        }
        let Some(slash) = t.rfind('/') else {
            warn.push(format!("{label}: bad token '{tok}'"));
            continue;
        };
        let name = &t[..slash];
        let len_s = &t[slash + 1..];
        if len_s.is_empty() || !len_s.bytes().all(|c| c.is_ascii_digit()) {
            warn.push(format!("{label}: bad token '{tok}'"));
            continue;
        }
        let len: u32 = len_s.parse().unwrap_or(0);
        if name == "r" {
            last = None;
        } else if name == "_" {
            if let Some(li) = last {
                ev[li].len += len;
            }
        } else {
            let Some(m) = parse_note(name) else {
                warn.push(format!("{label}: bad token '{tok}'"));
                continue;
            };
            ev.push(PhraseNote {
                s: pos,
                len,
                m,
                acc,
            });
            last = Some(ev.len() - 1);
        }
        pos += len;
    }
    if !pos.is_multiple_of(16) {
        warn.push(format!(
            "{label}: phrase length {pos} steps is not whole bars"
        ));
    }
    Phrase { ev, len: pos }
}

// ---------------------------------------------------------------------------
// compileSong (music.js L652-777)
// ---------------------------------------------------------------------------

/// `DRUMS` fixed push order (music.js L645).
const DRUMS: &[(&str, Inst)] = &[
    ("k", Inst::Kick),
    ("s", Inst::Snare),
    ("c", Inst::Clap),
    ("h", Inst::Hat),
    ("x", Inst::Crash),
    ("p", Inst::Rim),
    ("sh", Inst::Shaker),
    ("t", Inst::Tom),
];

/// ≡ `VEL` (music.js L650).
#[must_use]
fn vel_of(ch: char) -> f64 {
    match ch {
        'X' | 'O' => 1.0,
        'x' | 'o' => 0.8,
        'c' => 0.55,
        'g' => 0.35,
        'm' => 0.6,
        'r' => 0.45,
        _ => 0.8,
    }
}

/// ≡ `patFor` — fills replace the pattern on the last bar only (L646-650).
fn pat_for<'a>(
    src: Option<&'a Pat>,
    b: usize,
    nbars: usize,
    fill: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(f) = fill
        && b + 1 == nbars
    {
        return Some(f);
    }
    src.map(|p| p.at(b))
}

/// One compiled score event (JS `ev[s].push({i, v, m/ms, len, open, mute})`).
#[derive(Clone, Debug)]
pub struct Ev {
    pub i: Inst,
    pub v: f64,
    pub m: i32,
    pub ms: Vec<i32>,
    pub len: u32,
    pub open: bool,
    pub mute: bool,
}

impl Ev {
    fn simple(i: Inst, v: f64) -> Self {
        Ev {
            i,
            v,
            m: 0,
            ms: Vec::new(),
            len: 1,
            open: false,
            mute: false,
        }
    }
}

/// One compiled bar: 16 step event lists + the chord symbols. The section /
/// index / chord fields mirror `compileSong`'s output for debug parity; the
/// bake reads only `steps`.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Bar {
    pub sec: &'static str,
    pub bar: u32,
    pub steps: Vec<Vec<Ev>>,
    pub chords: Vec<&'static str>,
}

/// `compileSong(def)` result (music.js L777). `loop_from` / `warnings` are
/// read by the tests; the runtime loop wraps the whole baked buffer
/// (deviation: JS `loopFrom` skips the intro after the first pass).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CompiledSong {
    pub def: SongDef,
    pub bars: Vec<Bar>,
    pub loop_from: usize,
    pub warnings: Vec<String>,
}

/// `stepDur = 60/bpm/4` (music.js L1097).
#[must_use]
pub fn step_dur(bpm: f64) -> f64 {
    60.0 / bpm / 4.0
}

/// `barDur = 16*stepDur` (music.js L1097).
#[must_use]
pub fn bar_dur(bpm: f64) -> f64 {
    step_dur(bpm) * 16.0
}

impl CompiledSong {
    /// `MIX[g]` with per-song overrides (music.js L1083).
    #[must_use]
    pub fn mix(&self, g: &str) -> f64 {
        if let Some((_, v)) = self.def.mix.iter().find(|(k, _)| *k == g) {
            return *v;
        }
        MIX.iter()
            .find(|(k, _)| *k == g)
            .map(|(_, v)| *v)
            .unwrap_or(0.0)
    }

    /// `groups[g].gain = MIX[g] * layer_weight(g, intensity)` — the baked
    /// static group gain (intensity fixed at 1, [`layer_weight`]).
    #[must_use]
    pub fn group_gain(&self, g: &str) -> f64 {
        self.mix(g) * layer_weight(g, 1.0)
    }
}

/// ≡ `compileSong` (music.js L652-777).
#[must_use]
pub fn compile_song(def: SongDef) -> CompiledSong {
    let mut warn: Vec<String> = Vec::new();
    let mut bars: Vec<Bar> = Vec::new();
    let mut sec_start: HashMap<usize, usize> = HashMap::new();

    // phrases per section (L657-662)
    let mut phr: HashMap<&'static str, Vec<(&'static str, Phrase)>> = HashMap::new();
    for (name, sec) in &def.sections {
        let mut list = Vec::new();
        for (k, src) in [("lead", sec.lead), ("lead2", sec.lead2), ("riff", sec.riff)] {
            if let Some(s) = src {
                list.push((
                    k,
                    parse_phrase(s, &mut warn, &format!("{}.{name}.{k}", def.name)),
                ));
            }
        }
        phr.insert(name, list);
    }

    let chord_row = |sec: &SectionDef, b: usize| -> Vec<(Chord, u32, u32)> {
        let syms: Vec<&'static str> = if sec.chords.is_empty() {
            vec!["C"]
        } else {
            sec.chords[b % sec.chords.len()]
                .split_whitespace()
                .collect()
        };
        let n = syms.len() as u32;
        syms.iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    parse_chord(s).unwrap_or(Chord {
                        sym: "C",
                        root: 0,
                        iv: &[0, 4, 7],
                        bass: 0,
                    }),
                    ((i as u32 * 16) / n),
                    (((i as u32 + 1) * 16) / n),
                )
            })
            .collect()
    };

    for (oi, sec_name) in def.order.iter().enumerate() {
        let sec = match def.sections.iter().find(|(n, _)| n == sec_name) {
            Some((_, s)) => s,
            None => {
                warn.push(format!("missing section {sec_name}"));
                continue;
            }
        };
        sec_start.entry(oi).or_insert(bars.len());
        let nb = sec.bars as usize;
        for b in 0..nb {
            let mut ev: Vec<Vec<Ev>> = (0..16).map(|_| Vec::new()).collect();
            let row = chord_row(sec, b);
            let next_row: Vec<(Chord, u32, u32)> = if b + 1 < nb {
                chord_row(sec, b + 1)
            } else {
                let next_name = def
                    .order
                    .get(oi + 1)
                    .or_else(|| def.order.get(def.loop_from.unwrap_or(0)));
                let sym: &'static str = match next_name
                    .and_then(|nn| def.sections.iter().find(|(n, _)| n == nn))
                    .and_then(|(_, ns)| ns.chords.first())
                {
                    Some(c) => c.split_whitespace().next().unwrap_or("C"),
                    None => row[0].0.sym,
                };
                vec![(parse_chord(sym).unwrap_or(row[0].0), 0, 16)]
            };
            let chord_at = |s: usize| -> Chord {
                row.iter()
                    .find(|(_, s0, s1)| s >= (*s0 as usize) && s < (*s1 as usize))
                    .map(|(c, _, _)| *c)
                    .unwrap_or(row[0].0)
            };
            // drums (L690-703)
            for (key, inst) in DRUMS {
                let fill = sec.fills.iter().find(|(k, _)| k == key).map(|(_, f)| *f);
                let pat = sec.drums.iter().find(|(k, _)| k == key).map(|(_, p)| p);
                let p = match pat_for(pat, b, nb, fill) {
                    Some(p) => p,
                    None => continue,
                };
                if p.chars().count() != 16 {
                    warn.push(format!(
                        "{}.{sec_name}.{key} bar {b}: pattern length {}",
                        def.name,
                        p.chars().count()
                    ));
                }
                let cells: Vec<char> = p.chars().collect();
                for s in 0..16 {
                    let ch = cells[s];
                    if ch == '.' || ch == '-' {
                        continue;
                    }
                    let mut e = Ev::simple(*inst, vel_of(ch));
                    if *inst == Inst::Hat {
                        e.open = ch == 'o' || ch == 'O';
                    }
                    if *inst == Inst::Tom {
                        e.m = 45 + (s % 3) as i32 * 4;
                    }
                    ev[s].push(e);
                }
            }
            // bass (L705-727)
            let bass_fill = sec
                .fills
                .iter()
                .find(|(k, _)| *k == "bass")
                .map(|(_, f)| *f);
            if let Some(bp) = pat_for(sec.bass.as_ref(), b, nb, bass_fill) {
                if bp.chars().count() != 16 {
                    warn.push(format!(
                        "{}.{sec_name}.bass bar {b}: pattern length {}",
                        def.name,
                        bp.chars().count()
                    ));
                }
                let cells: Vec<char> = bp.chars().collect();
                let lo = def.bass_lo as i32;
                for s in 0..16 {
                    let c = cells[s];
                    if c == '.' || c == '-' {
                        continue;
                    }
                    let ch = chord_at(s);
                    let r = in_range(ch.bass, lo);
                    let third = if ch.iv.contains(&3) {
                        3
                    } else if ch.iv.contains(&4) {
                        4
                    } else {
                        7
                    };
                    let sev = if ch.iv.contains(&11) { 11 } else { 10 };
                    let m: i32 = match c {
                        'R' | 'r' => r,
                        'O' | 'o' => r + 12,
                        '5' => r + 7,
                        'L' => r - 5,
                        '3' => r + third,
                        '7' => r + sev,
                        '4' => r + 5,
                        '6' => r + 9,
                        'b' => r - 1,
                        'a' => r + 2,
                        'n' => {
                            let nr = in_range(next_row[0].0.bass, lo);
                            let mut m = nr - 1;
                            if m - r > 7 {
                                m -= 12;
                            }
                            m
                        }
                        _ => {
                            warn.push(format!("{}.{sec_name}.bass: bad token {c}", def.name));
                            continue;
                        }
                    };
                    let mut len = 1;
                    while s + len < 16 && cells[s + len] == '-' {
                        len += 1;
                    }
                    let v = if c == 'r' || c == 'o' {
                        0.5
                    } else {
                        0.85 + if s % 4 == 0 { 0.15 } else { 0.0 }
                    };
                    ev[s].push(Ev {
                        i: Inst::Bass,
                        v,
                        m,
                        ms: Vec::new(),
                        len: len as u32,
                        open: false,
                        mute: false,
                    });
                }
            }
            // chord stabs (L729-741)
            let stabs_fill = sec
                .fills
                .iter()
                .find(|(k, _)| *k == "stabs")
                .map(|(_, f)| *f);
            if let Some(sp) = pat_for(sec.stabs.as_ref(), b, nb, stabs_fill) {
                if sp.chars().count() != 16 {
                    warn.push(format!(
                        "{}.{sec_name}.stabs bar {b}: pattern length {}",
                        def.name,
                        sp.chars().count()
                    ));
                }
                let cells: Vec<char> = sp.chars().collect();
                for s in 0..16 {
                    let c = cells[s];
                    if c == '.' || c == '-' {
                        continue;
                    }
                    let mut len = 1;
                    while s + len < 16 && cells[s + len] == '-' {
                        len += 1;
                    }
                    let ch = chord_at(s);
                    let ms = if def.inst_chords == "guitar" {
                        power_chord(&ch, def.guitar_lo as i32)
                    } else {
                        let lo = def.keys_lo as i32;
                        voice(&ch, lo, lo + 19)
                    };
                    ev[s].push(Ev {
                        i: Inst::Chords,
                        v: vel_of(c),
                        m: 0,
                        ms,
                        len: len as u32,
                        open: false,
                        mute: c == 'm',
                    });
                }
            }
            // pad (L743)
            if sec.pad {
                for (ch, s0, s1) in &row {
                    let lo = def.pad_lo as i32;
                    ev[*s0 as usize].push(Ev {
                        i: Inst::Pad,
                        v: 0.8,
                        m: 0,
                        ms: voice(ch, lo, lo + 20),
                        len: s1 - s0,
                        open: false,
                        mute: false,
                    });
                }
            }
            // arp (L745-758)
            if let Some(a) = sec.arp {
                let rate = if a.rate == 0 { 1 } else { a.rate };
                let oct = if a.oct == 0 { 1 } else { a.oct };
                for s in (0..16).step_by(rate as usize) {
                    let ch = chord_at(s);
                    let tones = voice(&ch, a.lo as i32, a.lo as i32 + 11);
                    let mut seq: Vec<i32> = Vec::new();
                    for o in 0..oct {
                        for t in &tones {
                            seq.push(t + 12 * o as i32);
                        }
                    }
                    if a.pattern == "down" {
                        seq.reverse();
                    } else if a.pattern == "updown" {
                        let mut tail: Vec<i32> =
                            seq[1..seq.len().saturating_sub(1).max(1)].to_vec();
                        tail.reverse();
                        seq.extend(tail);
                    }
                    let idx = ((b * 16 + s) / rate as usize) % seq.len();
                    ev[s].push(Ev {
                        i: Inst::Arp,
                        v: if s % 4 == 0 { 0.95 } else { 0.7 },
                        m: seq[idx],
                        ms: Vec::new(),
                        len: rate,
                        open: false,
                        mute: false,
                    });
                }
            }
            // phrases (L760-773)
            if let Some(list) = phr.get(*sec_name) {
                for (k, ph) in list {
                    if ph.len == 0 {
                        continue;
                    }
                    let from = (b * 16) as i64;
                    let to = from + 16;
                    let mut base = (from / ph.len as i64) * ph.len as i64;
                    while base < to {
                        for n in &ph.ev {
                            let at = base + n.s as i64;
                            if at >= from && at < to {
                                let s = (at - from) as usize;
                                ev[s].push(Ev {
                                    i: match *k {
                                        "lead" => Inst::Lead,
                                        "lead2" => Inst::Lead2,
                                        _ => Inst::Riff,
                                    },
                                    m: n.m,
                                    len: n.len,
                                    v: if n.acc { 1.0 } else { 0.85 },
                                    ms: Vec::new(),
                                    open: false,
                                    mute: false,
                                });
                                // harmony layer: nearest chord tone 3..9
                                // semitones below the melody
                                if *k == "lead" && sec.harmony {
                                    ev[s].push(Ev {
                                        i: Inst::Lead2,
                                        m: harm_below(n.m, &chord_at(s)),
                                        len: n.len,
                                        v: 0.8,
                                        ms: Vec::new(),
                                        open: false,
                                        mute: false,
                                    });
                                }
                            }
                        }
                        base += ph.len as i64;
                    }
                }
            }
            // fx (L775-776)
            if sec.crash && b == 0 {
                ev[0].push(Ev::simple(Inst::Crash, 0.75));
            }
            let rn = if sec.riser == 0 { 1 } else { sec.riser };
            if sec.riser != 0 && b == nb - rn as usize {
                ev[0].push(Ev {
                    i: Inst::Riser,
                    v: 0.9,
                    m: 0,
                    ms: Vec::new(),
                    len: 16 * rn,
                    open: false,
                    mute: false,
                });
            }
            bars.push(Bar {
                sec: sec_name,
                bar: b as u32,
                steps: ev,
                chords: row.iter().map(|(c, _, _)| c.sym).collect(),
            });
        }
    }
    let loop_from = sec_start
        .get(&def.loop_from.unwrap_or(0))
        .copied()
        .unwrap_or(0);
    CompiledSong {
        def,
        bars,
        loop_from,
        warnings: warn,
    }
}

/// ≡ `getSong(id)` — compile on demand (the bake caches what it reuses).
#[must_use]
pub fn get_song(id: TrackId) -> Option<CompiledSong> {
    song_def(id).map(compile_song)
}

// ---------------------------------------------------------------------------
// Instruments not shared with the SFX table (music.js L370-569). The drum /
// bass / pad / bell / brass voices are the ones `audio.js` also imports,
// already ported into [`sfx_table`]; they are reused verbatim.
// ---------------------------------------------------------------------------

/// [`synth::mtof`] over the score's integer MIDI note numbers.
#[must_use]
fn mtof(m: i32) -> f64 {
    synth::mtof(f64::from(m))
}

/// ≡ `clap(v, T, vel, {d})` (music.js L370-388).
fn clap(v: &mut Vb, t: f64, vel: f64) {
    let g = v.gain(0.0, None);
    let bp = v.filter(BqKind::Bandpass, 1150.0, 1.1, Some(g), None);
    let hp = v.filter(BqKind::Highpass, 600.0, 0.7, Some(bp), None);
    let pk = vel * 0.9;
    let d = 0.2;
    v.g.param(g).set(0.0, t);
    for k in 0..3 {
        let tk = t + k as f64 * 0.0105;
        v.g.param(g).lin(pk, tk + 0.0008);
        v.g.param(g).lin(pk * 0.12, tk + 0.0095);
    }
    let t3 = t + 0.0315;
    v.g.param(g).lin(pk, t3 + 0.001);
    v.g.param(g).exp(pk * 0.001, t3 + d);
    v.g.param(g).lin(0.0, t3 + d + 0.008);
    v.noise(NoiseKind::White, t, Some(t3 + d + 0.02), Some(hp), 1.0);
}

/// ≡ `hat(v, T, vel, open, {d})` (music.js L390-398). Returns the gain node
/// so the caller can apply the open-hat cutoff (≡ the `g.gain` the JS
/// player keeps as `openHat`). `close_at` (local time) reproduces the
/// `cancelAndHoldAtTime + setTargetAtTime(0, T, 0.012)` the next hat event
/// performs upstream.
fn hat(v: &mut Vb, t: f64, vel: f64, open: bool, close_at: Option<f64>) -> NodeId {
    let g = v.gain(0.0, None);
    let d = if open { 0.32 } else { 0.045 };
    let end = synth::perc(v.g.param(g), t, 0.0008, vel, d);
    let pk = v.filter(BqKind::Peaking, 10500.0, 1.2, Some(g), Some(5.0));
    let hp = v.filter(
        BqKind::Highpass,
        if open { 6500.0 } else { 7500.0 },
        0.8,
        Some(pk),
        None,
    );
    v.noise(NoiseKind::White, t, Some(end + 0.01), Some(hp), 1.0);
    if let Some(tc) = close_at {
        v.g.param(g).cancel_hold(tc);
        v.g.param(g).target(0.0, tc, 0.012);
    }
    g
}

/// ≡ `rim(v, T, vel)` (music.js L408-412).
fn rim(v: &mut Vb, t: f64, vel: f64) {
    v.tone(Tone {
        at: Some(t),
        f: 1750.0,
        f1: Some(1500.0),
        sw: Some(0.02),
        a: Some(0.0005),
        d: Some(0.03),
        peak: Some(vel * 0.45),
        wave: Some(Wave::Triangle),
        ..Default::default()
    });
    v.nz(Nz {
        at: Some(t),
        f: Some(2600.0),
        q: Some(2.0),
        a: Some(0.0005),
        d: Some(0.02),
        peak: Some(vel * 0.45),
        ft: Some(Some(BqKind::Bandpass)),
        ..Default::default()
    });
}

/// ≡ `shaker(v, T, vel)` (music.js L414-416).
fn shaker(v: &mut Vb, t: f64, vel: f64) {
    v.nz(Nz {
        at: Some(t),
        f: Some(7000.0),
        q: Some(1.2),
        a: Some(0.006),
        d: Some(0.05),
        peak: Some(vel * 0.5),
        ft: Some(Some(BqKind::Bandpass)),
        ..Default::default()
    });
}

/// ≡ `guitar(v, T, fs, gate, vel, {mute})` (music.js L430-440).
fn guitar(v: &mut Vb, t: f64, fs: &[f64], gate: f64, vel: f64, mute: bool) -> f64 {
    let g = v.gain(0.0, None);
    let end = synth::adsr(
        v.g.param(g),
        t,
        0.003,
        0.35,
        if mute { 0.25 } else { 0.7 },
        vel,
        gate,
        if mute { 0.04 } else { 0.09 },
    );
    let hp = v.filter(BqKind::Highpass, 110.0, 0.7, Some(g), None);
    let pk = v.filter(BqKind::Peaking, 1900.0, 1.1, Some(hp), Some(4.0));
    let lp = v.filter(
        BqKind::Lowpass,
        if mute { 1500.0 } else { 3600.0 },
        0.9,
        Some(pk),
        None,
    );
    let sh = v.shaper(5.5, Some(lp));
    let pre = v.gain(0.42 / (fs.len() as f64).sqrt(), Some(sh));
    for &f in fs {
        for dt in [-7.0f64, 6.0] {
            let x = v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(pre));
            v.g.param2(x).set_default(dt);
        }
    }
    end
}

/// FM electric piano ≡ `keys(v, T, f, gate, vel)` (music.js L442-453).
fn keys(v: &mut Vb, t: f64, f: f64, gate: f64, vel: f64) -> f64 {
    let g = v.gain(0.0, None);
    let end = synth::adsr(v.g.param(g), t, 0.003, 1.3, 0.3, vel, gate, 0.3);
    let car = v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(g));
    let mg = v.gain(0.0, None);
    v.g.connect_param(mg, ParamRef::Freq(car));
    v.osc(Wave::Sine, f, t, Some(end + 0.01), Some(mg));
    v.g.param(mg).set(f * (1.4 + 1.2 * vel), t);
    v.g.param(mg).exp(f * 0.18, t + 0.7);
    v.tone(Tone {
        at: Some(t),
        f: (f * 7.0).min(9000.0),
        a: Some(0.001),
        d: Some(0.09),
        peak: Some(vel * 0.07),
        ..Default::default()
    });
    end
}

/// ≡ `pluck(v, T, f, gate, vel, {d, f0, f1, q})` (music.js L467-476). The
/// JS voice ignores `gate` (the `perc` decay sets the length); the slot is
/// kept for signature parity.
#[allow(clippy::too_many_arguments)]
fn pluck(
    v: &mut Vb,
    t: f64,
    f: f64,
    _gate: f64,
    vel: f64,
    d: f64,
    f0: f64,
    f1: f64,
    q: f64,
) -> f64 {
    let g = v.gain(0.0, None);
    let end = synth::perc(v.g.param(g), t, 0.002, vel, d);
    let lp = v.filter(BqKind::Lowpass, 1000.0, q, Some(g), None);
    synth::sweep(v.g.param(lp), t, f0, f1, 0.16, false);
    v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(lp));
    end
}

/// ≡ `lead(v, T, f, gate, vel, {style})` — styles `pulse | saw | soft |
/// square` (music.js L528-552). Music score lead events never pass
/// `glide`/`vib`/`vibRate` (fire only sends `{style}`; bell sends `{d}`),
/// so those branches are ported without the option knobs except `vib`.
fn lead(v: &mut Vb, t: f64, f: f64, gate: f64, vel: f64, style: &str) -> f64 {
    let g = v.gain(0.0, None);
    let soft = style == "soft";
    let end = synth::adsr(
        v.g.param(g),
        t,
        if soft { 0.03 } else { 0.006 },
        0.18,
        if soft { 0.85 } else { 0.72 },
        vel,
        gate,
        if soft { 0.14 } else { 0.1 },
    );
    let cut = if soft {
        2600.0
    } else if style == "saw" {
        3000.0
    } else {
        3800.0
    };
    let lp = v.filter(BqKind::Lowpass, cut, 1.1, Some(g), None);
    let mut oscs: Vec<NodeId> = Vec::new();
    if style == "saw" {
        for dt in [-6.0f64, 6.0] {
            let x = v.osc(Wave::Saw, f, t, Some(end + 0.01), Some(lp));
            v.g.param2(x).set_default(dt);
            oscs.push(x);
        }
    } else if soft {
        oscs.push(v.osc(Wave::Triangle, f, t, Some(end + 0.01), Some(lp)));
        let s2 = v.gain(0.35, Some(lp));
        oscs.push(v.osc(Wave::Sine, f * 2.0, t, Some(end + 0.01), Some(s2)));
        v.nz(Nz {
            at: Some(t),
            f: Some((f * 2.0).min(8000.0)),
            q: Some(2.0),
            a: Some(0.02),
            d: Some((gate * 0.6).max(0.05)),
            peak: Some(vel * 0.06),
            ft: Some(Some(BqKind::Bandpass)),
            ..Default::default()
        });
    } else if style == "square" {
        oscs.push(v.osc(Wave::Square, f, t, Some(end + 0.01), Some(lp)));
    } else {
        oscs.push(v.osc(Wave::Pulse(0.25), f, t, Some(end + 0.01), Some(lp)));
    }
    if gate > 0.18 {
        let lg = v.gain(0.0, None);
        v.g.param(lg).set(0.0, t);
        v.g.param(lg).lin(14.0, t + gate.min(0.35));
        v.osc(Wave::Sine, 5.6, t, Some(end + 0.01), Some(lg));
        for x in oscs {
            v.g.connect_param(lg, ParamRef::Detune(x));
        }
    }
    end
}

/// ≡ `riser(v, T, dur, vel)` (music.js L554-568).
fn riser(v: &mut Vb, t: f64, dur: f64, vel: f64) {
    let g = v.gain(0.0, None);
    let bp = v.filter(BqKind::Bandpass, 400.0, 2.5, Some(g), None);
    synth::sweep(v.g.param(bp), t, 350.0, 7000.0, dur, false);
    synth::pts(
        v.g.param(g),
        t,
        &[
            (0.0, 0.0),
            (dur * 0.9, vel),
            (dur, vel * 0.8),
            (dur + 0.02, 0.0),
        ],
    );
    v.noise(NoiseKind::White, t, Some(t + dur + 0.05), Some(bp), 1.0);
    let tg = v.gain(0.0, None);
    synth::pts(
        v.g.param(tg),
        t,
        &[(0.0, 0.0), (dur, vel * 0.08), (dur + 0.02, 0.0)],
    );
    let lpf = v.filter(BqKind::Lowpass, 3000.0, 1.0, Some(tg), None);
    let x = v.osc(Wave::Saw, 220.0, t, Some(t + dur + 0.05), Some(lpf));
    synth::sweep(v.g.param(x), t, 180.0, 1400.0, dur, false);
}

// ---------------------------------------------------------------------------
// Kick side-chain pump (music.js `pumpDuck` L1238-1243)
// ---------------------------------------------------------------------------

/// The `pumpG` gain curve: every kick at `Tk` runs
/// `setTargetAtTime(1-d, Tk, 0.004)` then `setTargetAtTime(1, Tk+0.03,
/// 0.075)`; the closed form chains the segments (JS ducks at the note
/// time, so the bake matches the live curve exactly).
#[derive(Clone, Debug, Default)]
pub struct PumpCurve {
    pub d: f64,
    kicks: Vec<f64>,
    /// value just before each kick
    a: Vec<f64>,
    /// value at kick+0.03
    b: Vec<f64>,
}

const TC_DOWN: f64 = 0.004;
const TC_UP: f64 = 0.075;

impl PumpCurve {
    #[must_use]
    fn new(d: f64, kicks: &[f64]) -> Self {
        let mut c = PumpCurve {
            d,
            kicks: kicks.to_vec(),
            a: Vec::new(),
            b: Vec::new(),
        };
        if d <= 0.0 {
            return c;
        }
        let mut v = 1.0;
        let mut prev: Option<f64> = None;
        for &tk in kicks {
            if let Some(tp) = prev {
                // recovery segment from tp+0.03 toward 1
                let dt = tk - (tp + 0.03);
                v = 1.0 + (c.b[c.b.len() - 1] - 1.0) * (-dt / TC_UP).exp();
            }
            c.a.push(v);
            let down = 1.0 - d;
            let b = down + (v - down) * (-0.03 / TC_DOWN).exp();
            c.b.push(b);
            prev = Some(tk);
        }
        c
    }

    #[must_use]
    pub fn at(&self, t: f64) -> f64 {
        if self.d <= 0.0 || self.kicks.is_empty() {
            return 1.0;
        }
        // last kick <= t
        let k = match self.kicks.binary_search_by(|x| x.partial_cmp(&t).unwrap()) {
            Ok(i) => i,
            Err(0) => return 1.0,
            Err(i) => i - 1,
        };
        let tk = self.kicks[k];
        let down = 1.0 - self.d;
        if t < tk + 0.03 {
            down + (self.a[k] - down) * (-(t - tk) / TC_DOWN).exp()
        } else {
            1.0 + (self.b[k] - 1.0) * (-(t - tk - 0.03) / TC_UP).exp()
        }
    }
}

// ---------------------------------------------------------------------------
// Offline bake (≡ the live Player, per note)
// ---------------------------------------------------------------------------

/// Render one score event through its own tiny graph at local time 0 and
/// return its PCM block (≡ the JS `Player.fire` + live node graph).
/// `close_at` is the open-hat cutoff time in local seconds (see [`hat`]).
fn render_event(
    comp: &CompiledSong,
    e: &Ev,
    vel: f64,
    sd: f64,
    rng: &mut Rng,
    close_at: Option<f64>,
) -> Vec<(String, Vec<f32>)> {
    let def = &comp.def;
    let mut out: Vec<(String, Vec<f32>)> = Vec::new();
    let mut vb = Vb::new(0.0, *rng);
    match e.i {
        Inst::Kick => {
            sfx_table::kick(&mut vb, 0.0, vel, 165.0, 46.0, 0.34, 0.085, None);
        }
        Inst::Snare => {
            sfx_table::snare(&mut vb, 0.0, vel, 205.0, 0.19, None);
        }
        Inst::Clap => clap(&mut vb, 0.0, vel),
        Inst::Hat => {
            hat(&mut vb, 0.0, vel, e.open, close_at);
        }
        Inst::Crash => {
            sfx_table::crash(&mut vb, 0.0, vel, 1.6, None);
        }
        Inst::Rim => rim(&mut vb, 0.0, vel),
        Inst::Shaker => shaker(&mut vb, 0.0, vel),
        Inst::Tom => {
            sfx_table::tom(&mut vb, 0.0, mtof(e.m), vel, 0.35, None);
        }
        Inst::Bass => {
            let gate = e.len as f64 * sd * 0.82;
            sfx_table::bass(&mut vb, 0.0, mtof(e.m), gate, vel, def.inst_bass, 2.2, None);
        }
        Inst::Chords => {
            let gate = e.len as f64 * sd * 0.9;
            match def.inst_chords {
                "guitar" => {
                    let fs: Vec<f64> = e.ms.iter().map(|&m| mtof(m)).collect();
                    guitar(&mut vb, 0.0, &fs, gate, vel, e.mute);
                }
                "keys" => {
                    let n = e.ms.len() as f64;
                    for &m in &e.ms {
                        keys(&mut vb, 0.0, mtof(m), gate, vel / n.sqrt() * 0.7);
                    }
                }
                "brass" => {
                    let n = e.ms.len() as f64;
                    for &m in &e.ms {
                        sfx_table::brass(
                            &mut vb,
                            0.0,
                            mtof(m),
                            gate,
                            vel / n.sqrt() * 0.7,
                            0.022,
                            0.2,
                            3000.0,
                            None,
                        );
                    }
                }
                _ => {
                    // 'pluck' (results_win)
                    let n = e.ms.len() as f64;
                    for &m in &e.ms {
                        pluck(
                            &mut vb,
                            0.0,
                            mtof(m),
                            gate,
                            vel / n.sqrt() * 0.7,
                            0.28,
                            3800.0,
                            700.0,
                            2.0,
                        );
                    }
                }
            }
        }
        Inst::Pad => {
            let gate = e.len as f64 * sd;
            let n = e.ms.len() as f64;
            for &m in &e.ms {
                sfx_table::pad(
                    &mut vb,
                    0.0,
                    mtof(m),
                    gate,
                    vel / n.sqrt() * 0.6,
                    0.25,
                    0.5,
                    1500.0,
                    None,
                );
            }
        }
        Inst::Arp => {
            let gate = e.len as f64 * sd;
            pluck(
                &mut vb,
                0.0,
                mtof(e.m),
                gate,
                vel * 0.6,
                0.2,
                5200.0,
                520.0,
                4.0,
            );
        }
        Inst::Lead | Inst::Lead2 => {
            let style = if e.i == Inst::Lead2 {
                def.inst_lead2.unwrap_or("saw")
            } else {
                def.inst_lead
            };
            let gate = e.len as f64 * sd * 0.92;
            if style == "bell" {
                sfx_table::bell(
                    &mut vb,
                    0.0,
                    mtof(e.m),
                    vel * 0.8,
                    (0.35 + gate).min(1.4),
                    3.5,
                    1.6,
                    None,
                );
            } else {
                lead(&mut vb, 0.0, mtof(e.m), gate, vel * 0.7, style);
            }
        }
        Inst::Riff => {
            let gate = e.len as f64 * sd * 0.85;
            let f = mtof(e.m);
            guitar(&mut vb, 0.0, &[f, f * 1.4983, f * 2.0], gate, vel, false);
        }
        Inst::Riser => {
            riser(&mut vb, 0.0, e.len as f64 * sd, vel);
        }
    }
    *rng = vb.rng;
    out.push((e.i.group().to_string(), vb.bake()));
    // riffBass: a second voice through the bass group (music.js L1192-1196)
    if e.i == Inst::Riff && def.riff_bass {
        let gate = e.len as f64 * sd * 0.85;
        let mut vb2 = Vb::new(0.0, *rng);
        sfx_table::bass(
            &mut vb2,
            0.0,
            mtof(e.m) / 2.0,
            gate,
            vel,
            def.inst_bass,
            2.2,
            None,
        );
        *rng = vb2.rng;
        out.push(("bass".to_string(), vb2.bake()));
    }
    out
}

/// Bake the first `nbars` bars of `comp` into a mono 44.1 kHz block,
/// clamped to [-1, 1] (a full song is [`bake_song`]). The buffer covers the
/// rendered span + 2 s of tail; playback loops the whole buffer (deviation
/// note at the top of the module).
#[must_use]
pub fn bake_song_bars(comp: &CompiledSong, nbars: usize) -> Vec<f32> {
    let sd = step_dur(comp.def.bpm);
    let bd = sd * 16.0;
    let nb = nbars.min(comp.bars.len());
    let total = nb as f64 * bd;
    let mut master = vec![0.0f32; ((total + 2.0) * synth::SR).ceil() as usize + 1];
    let mut rng = Rng::new(sfx_table::bake_seed(comp.def.id));

    // pre-pass: kick times (pump) and the next-hat time per open hat
    let mut kicks: Vec<f64> = Vec::new();
    let mut hat_times: Vec<f64> = Vec::new();
    for (bi, bar) in comp.bars.iter().enumerate().take(nb) {
        for (s, evs) in bar.steps.iter().enumerate() {
            let t =
                bi as f64 * bd + s as f64 * sd + if s & 1 == 1 { comp.def.swing * sd } else { 0.0 };
            for e in evs {
                match e.i {
                    Inst::Kick => kicks.push(t),
                    Inst::Hat => hat_times.push(t),
                    _ => {}
                }
            }
        }
    }
    let pump = PumpCurve::new(comp.def.pump, &kicks);

    for (bi, bar) in comp.bars.iter().enumerate().take(nb) {
        for (s, evs) in bar.steps.iter().enumerate() {
            let t =
                bi as f64 * bd + s as f64 * sd + if s & 1 == 1 { comp.def.swing * sd } else { 0.0 };
            for e in evs {
                // ≡ Player.fire: vel jitter consumes the rng first
                let vel = e.v * (0.93 + 0.12 * rng.next_f64());
                // ≡ openHat cancelAndHold at the next hat event (L1174-1178)
                let close_at = if e.i == Inst::Hat && e.open {
                    hat_times.iter().find(|&&ht| ht > t).map(|&ht| ht - t)
                } else {
                    None
                };
                for (group, pcm) in render_event(comp, e, vel, sd, &mut rng, close_at) {
                    let scale = comp.group_gain(&group) * BUS_OUT;
                    let pumped = PUMPED.contains(&group.as_str());
                    let off = (t * synth::SR) as usize;
                    for (j, smp) in pcm.iter().enumerate() {
                        let idx = off + j;
                        if idx >= master.len() {
                            break;
                        }
                        let mut x = (*smp as f64) * scale;
                        if pumped {
                            x *= pump.at(idx as f64 / synth::SR);
                        }
                        master[idx] += x as f32;
                    }
                }
            }
        }
    }
    for s in &mut master {
        *s = s.clamp(-1.0, 1.0);
    }
    master
}

/// Bake a whole song (slow in debug builds — minutes for `battle`; the
/// runtime bakes lazily per track, tests use [`bake_song_bars`]).
#[must_use]
pub fn bake_song(comp: &CompiledSong) -> Vec<f32> {
    bake_song_bars(comp, comp.bars.len())
}

// ===========================================================================
// Part D — the `play()` state machine, boss remap, duck envelope, volumes
// ===========================================================================

/// JS `taper` (audio.js L24): the perceptual 0..1 slider curve.
#[must_use]
pub fn taper(v: f64) -> f64 {
    v.clamp(0.0, 1.0).powf(1.5)
}

/// The `music.remap` state the boss director installs (bossAudio.js `st`
/// L15 + L58): while a live boss match owns the score, the generic
/// `'battle'` / `'battle_final'` requests resolve to the boss track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BossRemap {
    /// JS `st.active`.
    pub active: bool,
    /// JS `st.dead` (defeat: the set stops, no more routing).
    pub dead: bool,
    /// JS `st.phase` (1..3) — the fallback when `track` is unset.
    pub phase: u32,
    /// JS `st.track` — the explicit last-requested boss track.
    pub track: Option<TrackId>,
}

impl BossRemap {
    /// JS `phaseTrack()` (bossAudio.js L54): `TRACK[min(3,max(1,phase))]`.
    #[must_use]
    pub fn phase_track(&self) -> TrackId {
        match self.phase {
            2 => "boss_2",
            3 => "boss_3",
            _ => "boss",
        }
    }

    /// JS `mus.remap = (t) => ...` (bossAudio.js L58). Returning the request
    /// unchanged ≡ the JS `undefined` branch (`play()` keeps the request).
    #[must_use]
    pub fn apply(&self, track: Option<TrackId>) -> Option<TrackId> {
        match track {
            Some(t) if (t == "battle" || t == "battle_final") && self.active && !self.dead => {
                Some(self.track.unwrap_or_else(|| self.phase_track()))
            }
            other => other,
        }
    }
}

/// The live player (`this.current`) as far as `play()` observes it: id,
/// start time and tempo (music.js L1102-1109).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playing {
    pub track: TrackId,
    pub t0: f64,
    pub bpm: f64,
}

/// ≡ `Player.nextBarTime(after)` (music.js L1146-1149).
#[must_use]
pub fn next_bar_time(t0: f64, bpm: f64, after: f64) -> f64 {
    let bar = bar_dur(bpm);
    let k = ((after - t0) / bar - 1e-6).ceil().max(0.0);
    t0 + k * bar
}

/// One side effect `play()` asks the runtime to perform (the WebAudio
/// automation in JS; here an explicit instruction the voice bookkeeping
/// applies on the same clock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusicAction {
    /// New player at `t0`, `fadeIn(t0, fade_in)` (music.js L1403-1415).
    Start {
        track: TrackId,
        t0: f64,
        fade_in: f64,
    },
    /// `cur.fadeOut(at, dur)` (music.js L1139-1145). `riser` ≡ the
    /// escalation riser: `Some(t0 - now)` when JS fires one (L1411).
    /// Deviation: the riser voice itself is not baked into the old loop.
    FadeOut {
        at: f64,
        dur: f64,
        riser: Option<f64>,
    },
}

/// Pure port of `MusicEngine.play` (music.js L1378-1417). The file branch is
/// unreachable here ([`has_file`] is always `false`) and `_want` (the
/// no-context deferral, L1385) has no counterpart — the Rust runtime always
/// has a clock.
#[derive(Clone, Debug, Default)]
pub struct MusicState {
    pub current: Option<Playing>,
    pub remap: BossRemap,
}

impl MusicState {
    /// ≡ `play(track, { fade })`. `fade = None` ≡ JS `opts.fade ?? 1.0`.
    pub fn play(
        &mut self,
        track: Option<TrackId>,
        fade: Option<f64>,
        now: f64,
    ) -> Vec<MusicAction> {
        let track = self.remap.apply(track);
        let fade = fade.unwrap_or(1.0).max(0.0);
        if let Some(t) = track
            && !is_track(t)
            && !has_file(t)
        {
            // ≡ console.warn('[music] unknown track', track) (L1384).
            eprintln!("[music] unknown track {t}");
            return Vec::new();
        }
        let cur = self.current;
        // ≡ `cur && cur.id === track` (L1388): re-requesting the live track
        // is a no-op (`null` never matches a real id).
        if let Some(c) = cur
            && Some(c.track) == track
        {
            return Vec::new();
        }
        let Some(t) = track else {
            // ≡ L1391: stop the set, `fadeOut(now, max(0.03, fade))`.
            self.current = None;
            return match cur {
                Some(_) => vec![MusicAction::FadeOut {
                    at: now,
                    dur: fade.max(0.03),
                    riser: None,
                }],
                None => Vec::new(),
            };
        };
        // `is_track` above guarantees the song exists; only the tempo is read.
        let bpm = get_song(t).expect("track checked").def.bpm;
        // ≡ L1399-1401: default start `now + 0.06`; a same-tempo crossfade
        // lands on the old track's next bar line (beat-synced escalation).
        let mut t0 = now + 0.06;
        let mut sync = false;
        if let Some(c) = cur
            && fade > 0.0
            && c.bpm == bpm
        {
            t0 = next_bar_time(c.t0, c.bpm, now + 0.12);
            sync = true;
        }
        let mut acts = Vec::new();
        if cur.is_some() {
            if sync {
                // ≡ L1405-1411: old track plays to the bar line, new one
                // lands on the downbeat; riser when the span is audible.
                let span = t0 - now;
                acts.push(MusicAction::FadeOut {
                    at: t0,
                    dur: fade.min(0.25),
                    riser: if span > 0.3 { Some(span) } else { None },
                });
                acts.push(MusicAction::Start {
                    track: t,
                    t0,
                    fade_in: 0.008,
                });
            } else {
                // ≡ L1413-1414.
                acts.push(MusicAction::FadeOut {
                    at: now,
                    dur: fade.max(0.03),
                    riser: None,
                });
                acts.push(MusicAction::Start {
                    track: t,
                    t0,
                    fade_in: if fade > 0.0 { fade * 0.5 } else { 0.01 },
                });
            }
        } else {
            // ≡ L1415: cold start fades in over 0.01 s.
            acts.push(MusicAction::Start {
                track: t,
                t0,
                fade_in: 0.01,
            });
        }
        self.current = Some(Playing { track: t, t0, bpm });
        acts
    }

    /// ≡ `stop(fade = 1)` → `play(null, { fade })` (music.js L1470).
    /// Parity API: the runtime reaches it through `request(None, …)`.
    #[allow(dead_code)]
    pub fn stop(&mut self, fade: Option<f64>, now: f64) -> Vec<MusicAction> {
        self.play(None, Some(fade.unwrap_or(1.0)), now)
    }
}

/// ≡ the `audio.duck` envelope (audio.js L357-378): the JS gain automation
/// is mirrored by the tracked piecewise-linear point list (`_duckPts`), so
/// overlapping ducks continue from the exact current value.
#[derive(Clone, Debug, Default)]
pub struct DuckState {
    /// JS `this._duck` — `(target, until)` of the live duck.
    live: Option<(f64, f64)>,
    /// JS `this._duckPts`.
    pts: Vec<(f64, f64)>,
}

impl DuckState {
    /// ≡ `_duckAt(t)` (audio.js L357-362).
    #[must_use]
    pub fn value_at(&self, t: f64) -> f64 {
        let p = &self.pts;
        if p.is_empty() {
            return 1.0;
        }
        let last = p[p.len() - 1];
        if t >= last.0 {
            return last.1;
        }
        let first = p[0];
        if t <= first.0 {
            return first.1;
        }
        for i in 1..p.len() {
            if t <= p[i].0 {
                let (t0, v0) = p[i - 1];
                let (t1, v1) = p[i];
                return if t1 > t0 {
                    v0 + (v1 - v0) * (t - t0) / (t1 - t0)
                } else {
                    v1
                };
            }
        }
        1.0
    }

    /// ≡ `duck(amount, seconds)` at clock `now` (audio.js L364-378).
    pub fn duck(&mut self, amount: f64, seconds: f64, now: f64) {
        let mut target = 1.0 - amount.clamp(0.0, 1.0);
        let mut until = now + 0.08 + seconds.max(0.0);
        if let Some((ct, cu)) = self.live
            && now < cu
        {
            target = target.min(ct);
            until = until.max(cu);
        }
        self.live = Some((target, until));
        let v = self.value_at(now);
        self.pts = vec![
            (now, v),
            (now + 0.08, target),
            (until, target),
            (until + 0.6, 1.0),
        ];
    }
}

/// Slider defaults (config.js `DEFAULT_SETTINGS`, mirrored by audio.js L165
/// which clamps missing keys to these on `init`).
const DEFAULT_MASTER: f64 = 0.8;
const DEFAULT_MUSIC: f64 = 0.6;
const DEFAULT_SFX: f64 = 0.85;

/// ≡ `audio.setVolumes({ master, music, sfx })` (audio.js L165-172). Each
/// bus is a [`Param`] so the JS `setTargetAtTime(taper(v), now, 0.04)`
/// smoothing is reproduced deterministically (τ = 0.04 s, frame-rate
/// independent). `master` rides Bevy's `GlobalVolume`, `music` the per-voice
/// loop volume, `sfx` the per-cue spawn volume.
#[derive(Resource)]
pub struct AudioVolumes {
    pub master: Param,
    pub music: Param,
    pub sfx: Param,
}

impl Default for AudioVolumes {
    fn default() -> Self {
        Self {
            master: Param::new(taper(DEFAULT_MASTER)),
            music: Param::new(taper(DEFAULT_MUSIC)),
            sfx: Param::new(taper(DEFAULT_SFX)),
        }
    }
}

impl AudioVolumes {
    /// ≡ `setVolumes`: non-finite / `None` slots keep their old value; each
    /// finite slider re-targets its bus with τ = 0.04 s (audio.js L166-171).
    /// Parity API: M1 has no settings UI to drive it yet — the buses read
    /// the defaults until one lands.
    #[allow(dead_code)]
    pub fn set_volumes(
        &mut self,
        master: Option<f64>,
        music: Option<f64>,
        sfx: Option<f64>,
        now: f64,
    ) {
        for (slot, v) in [
            (&mut self.master, master),
            (&mut self.music, music),
            (&mut self.sfx, sfx),
        ] {
            if let Some(v) = v.filter(|v| v.is_finite()) {
                slot.target(taper(v), now, 0.04);
            }
        }
    }
}

// ===========================================================================
// Part E — runtime: baked assets, voices, systems
// ===========================================================================

/// JS `Player.fadeSeg` — the JS-side linear fade level (music.js L1133).
#[derive(Clone, Copy, Debug)]
struct FadeSeg {
    t0: f64,
    v0: f64,
    t1: f64,
    v1: f64,
}

impl FadeSeg {
    /// ≡ `fadeAt(t)`.
    fn at(&self, t: f64) -> f64 {
        if t <= self.t0 {
            return self.v0;
        }
        if t >= self.t1 {
            return self.v1;
        }
        self.v0 + (self.v1 - self.v0) * (t - self.t0) / (self.t1 - self.t0)
    }
}

/// Whole-song PCM bakes, one handle per track id. A full synth score is
/// minutes of per-note graph rendering, so each track bakes lazily on its
/// first `play()` and is cached afterwards (the handle lives in the
/// `Assets<PcmSound>` store; the Vec<f32> is dropped after upload — one
/// resident copy per played track, ≈12 MB for a 38-bar song).
#[derive(Resource, Default)]
pub struct MusicAssets {
    map: HashMap<TrackId, Handle<PcmSound>>,
    dur: HashMap<TrackId, f64>,
}

impl MusicAssets {
    /// Handle + duration for `id`, baking the whole score on first use.
    /// File-backed ids never reach here ([`has_file`] is `false`).
    pub(crate) fn track(
        &mut self,
        assets: &mut Assets<PcmSound>,
        id: TrackId,
    ) -> Option<(Handle<PcmSound>, f64)> {
        if let Some(h) = self.map.get(id) {
            return Some((h.clone(), *self.dur.get(id)?));
        }
        if !is_track(id) || has_file(id) {
            return None;
        }
        let comp = get_song(id)?;
        let data = bake_song(&comp);
        let dur = data.len() as f64 / synth::SR;
        let h = assets.add(PcmSound(Pcm {
            sample_rate: super::SR,
            data,
        }));
        self.map.insert(id, h.clone());
        self.dur.insert(id, dur);
        Some((h, dur))
    }

    /// Test seam: force the (expensive) bake now and return the duration.
    #[cfg(test)]
    pub(crate) fn prebake(&mut self, assets: &mut Assets<PcmSound>, id: TrackId) -> Option<f64> {
        self.track(assets, id).map(|(_, d)| d)
    }
}

/// One live music player (JS `Player`): its entity (`None` before the `t0`
/// spawn and after reap), the tracked fade and `stopAt`.
#[derive(Debug)]
pub struct MVoice {
    entity: Option<Entity>,
    track: TrackId,
    t0: f64,
    fade: FadeSeg,
    stop_at: f64,
}

/// Runtime side of [`MusicState`]: the voice bookkeeping and the duck bus.
/// JS schedules every fade on the WebAudio clock; the same curves are
/// tracked in JS-side bookkeeping here (`fadeSeg` / `_duckPts`) and read
/// once per frame.
#[derive(Resource, Default)]
pub struct MusicRuntime {
    pub state: MusicState,
    pub duck: DuckState,
    voices: Vec<MVoice>,
}

impl MusicRuntime {
    /// ≡ `music.play(track, { fade })` — routes through the boss remap
    /// inside [`MusicState::play`] and applies the actions to the voices.
    pub fn request(&mut self, track: Option<TrackId>, fade: Option<f64>, now: f64) {
        let cur_before = self.state.current;
        let acts = self.state.play(track, fade, now);
        for a in acts {
            match a {
                MusicAction::FadeOut { at, dur, riser: _ } => {
                    // The voice `play()` saw as `cur`: the newest live voice
                    // of that track not yet faded to silence.
                    if let Some(c) = cur_before
                        && let Some(v) = self
                            .voices
                            .iter_mut()
                            .rev()
                            .find(|v| v.track == c.track && v.stop_at > at)
                    {
                        let d = dur.max(0.01);
                        let v0 = v.fade.at(at);
                        v.fade = FadeSeg {
                            t0: at,
                            v0,
                            t1: at + d,
                            v1: 0.0,
                        };
                        v.stop_at = v.stop_at.min(at + d + 0.02);
                    }
                }
                MusicAction::Start {
                    track: t,
                    t0,
                    fade_in,
                } => {
                    self.voices.push(MVoice {
                        entity: None,
                        track: t,
                        t0,
                        fade: FadeSeg {
                            t0,
                            v0: 0.0,
                            t1: t0 + fade_in.max(0.005),
                            v1: 1.0,
                        },
                        stop_at: f64::INFINITY,
                    });
                }
            }
        }
    }

    /// ≡ `audio.duck(amount, seconds)` on the music bus.
    pub fn duck(&mut self, amount: f64, seconds: f64, now: f64) {
        self.duck.duck(amount, seconds, now);
    }
}

/// Spawn due voices, drive their bus volume, reap finished ones.
///
/// Deviations (registered in the Task 50 report): the loop wraps at the
/// whole-song end (JS `loopFrom` skips the intro after the first pass); the
/// delay/reverb send buses (`sendDly` / `sendRev`) are dry-baked; the
/// escalation riser voice is recorded in [`MusicAction`] but not synthesised.
pub fn music_update(
    time: Res<Time>,
    mut rt: ResMut<MusicRuntime>,
    mut assets: ResMut<MusicAssets>,
    mut pcm: ResMut<Assets<PcmSound>>,
    mut commands: Commands,
    mut sinks: Query<&mut AudioSink>,
    vols: Res<AudioVolumes>,
) {
    let now = time.elapsed_secs_f64();
    // Voices whose `t0` has come due (JS created the player at request time
    // and scheduled notes from `t0`; the entity appears when the clock
    // reaches it — at most one frame late).
    for v in rt.voices.iter_mut() {
        if v.entity.is_none()
            && now >= v.t0
            && v.stop_at > now
            && let Some((h, _)) = assets.track(&mut pcm, v.track)
        {
            let e = commands
                .spawn((
                    AudioPlayer(h),
                    PlaybackSettings::LOOP.with_volume(Volume::SILENT),
                ))
                .id();
            v.entity = Some(e);
        }
    }
    // ≡ `musicBus.gain = taper(music) * fade(t) * duck(t)`; the master
    // slider and the `M` mute ride `GlobalVolume` (audio_sfx).
    let music = vols.music.value_at(now);
    let duck = rt.duck.value_at(now);
    for v in rt.voices.iter() {
        if let Some(e) = v.entity
            && let Ok(mut sink) = sinks.get_mut(e)
        {
            let vol = (music * v.fade.at(now) * duck).clamp(0.0, 1.0) as f32;
            sink.set_volume(Volume::Linear(vol));
        }
    }
    // ≡ `_tick` reap (`stopAt + 4 < now` — the 4 s grace exists for the
    // live scheduler; here the baked loop is despawned at `stopAt`).
    let mut dead: Vec<usize> = Vec::new();
    for (i, v) in rt.voices.iter().enumerate() {
        if v.stop_at.is_finite() && now >= v.stop_at {
            dead.push(i);
        }
    }
    for i in dead.into_iter().rev() {
        let v = rt.voices.remove(i);
        if let Some(e) = v.entity
            && let Ok(mut ent) = commands.get_entity(e)
        {
            ent.despawn();
        }
    }
}

/// Edge tracker for the scene requests (main.js `_playMusic` call sites).
#[derive(Resource, Default)]
pub struct MusicSceneState {
    prev_screen: Option<Screen>,
    prev_phase: Option<Phase>,
    /// JS `setTimeout(() => _playMusic(win|lose), 2600)` (main.js L1028).
    results_at: Option<f64>,
}

/// ≡ main.js L419-426 (menu screens → `'menu'`, fade 1.2), L555 (`playing`
/// → `'battle'`), L548 (`oneminute` → `'battle_final'`), L1028 (results +
/// 2.6 s → `'results_win'` / `'results_lose'`). Boss mode is guarded out
/// exactly as upstream (`match.mode !== 'boss'`) — the boss director scores
/// it. Deviations: `'title'` is unreachable (Rust has no title screen); the
/// pause/`fileTrack` knobs of hud.js are optional-chain no-ops upstream.
pub fn music_scenes(
    time: Res<Time>,
    ui: Res<UiState>,
    demo: Res<DemoSim>,
    mut rt: ResMut<MusicRuntime>,
    mut sc: ResMut<MusicSceneState>,
) {
    let now = time.elapsed_secs_f64();
    if Some(ui.screen) != sc.prev_screen {
        sc.prev_screen = Some(ui.screen);
        match ui.screen {
            Screen::Menu | Screen::OnlineHub | Screen::OnlineLobby => {
                rt.request(Some("menu"), Some(1.2), now);
            }
            Screen::Results => {
                sc.results_at = Some(now);
            }
            _ => {}
        }
    }
    // ≡ `match:state 'playing'` (main.js L555): the Hud's phase edge.
    let phase = demo.m.phase;
    let phase_edge = Some(phase) != sc.prev_phase;
    sc.prev_phase = Some(phase);
    if ui.screen == Screen::Hud
        && phase_edge
        && phase == Phase::Active
        && demo.m.mode != MatchMode::Boss
    {
        rt.request(Some("battle"), Some(1.2), now);
    }
    // ≡ `match:oneminute` (main.js L548). Events are drained later in the
    // chain (ui_state_machine), so this read sees each one exactly once.
    if demo.m.mode != MatchMode::Boss
        && demo
            .m
            .events
            .iter()
            .any(|e| matches!(e, MatchEvent::OneMinute))
    {
        rt.request(Some("battle_final"), Some(1.2), now);
    }
    // ≡ L1028: the results jingle plays first (`audio_sfx`), the results
    // track lands 2.6 s later.
    if let Some(at) = sc.results_at
        && now >= at + 2.6
    {
        sc.results_at = None;
        let won = demo
            .m
            .result
            .as_ref()
            .is_some_and(|r| r.winner == demo.m.actors.first().map(|a| a.team).unwrap_or(0));
        rt.request(
            Some(if won { "results_win" } else { "results_lose" }),
            Some(1.2),
            now,
        );
    }
}

// ===========================================================================
// Part F — tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> f64 {
        (a - b).abs()
    }

    fn expect_start(a: &[MusicAction], track: TrackId, t0: f64, fade_in: f64) {
        match a {
            [
                MusicAction::Start {
                    track: t,
                    t0: x,
                    fade_in: f,
                },
            ] => {
                assert_eq!(*t, track);
                assert!(approx(*x, t0) < 1e-9, "t0 {x} vs {t0}");
                assert!(approx(*f, fade_in) < 1e-12, "fade_in {f} vs {fade_in}");
            }
            other => panic!("expected one Start, got {other:?}"),
        }
    }

    fn expect_fade_out(a: &MusicAction, at: f64, dur: f64, riser: Option<f64>) {
        match a {
            MusicAction::FadeOut {
                at: x,
                dur: d,
                riser: r,
            } => {
                assert!(approx(*x, at) < 1e-9, "at {x} vs {at}");
                assert!(approx(*d, dur) < 1e-12, "dur {d} vs {dur}");
                match (r, riser) {
                    (Some(a), Some(b)) => assert!(approx(*a, b) < 1e-9, "riser {a} vs {b}"),
                    (None, None) => {}
                    _ => panic!("riser {r:?} vs {riser:?}"),
                }
            }
            other => panic!("expected FadeOut, got {other:?}"),
        }
    }

    #[test]
    fn play_switch_sequence_matches_js() {
        // ≡ music.js L1378-1417 on fixed inputs (menu 105 → battle 150 →
        // battle_final 150 same-tempo escalation → stop).
        let mut st = MusicState::default();
        let a = st.play(Some("menu"), None, 0.0);
        expect_start(&a, "menu", 0.06, 0.01);
        // Re-requesting the live track is a no-op (L1388).
        assert!(st.play(Some("menu"), None, 2.0).is_empty());
        // Different tempo → immediate crossfade, `fadeIn(fade * 0.5)`.
        let a = st.play(Some("battle"), Some(1.2), 5.0);
        assert_eq!(a.len(), 2);
        expect_fade_out(&a[0], 5.0, 1.2, None);
        expect_start(&[a[1]], "battle", 5.06, 0.6);
        // Same tempo (150) → beat-synced: the new track lands on the next
        // bar line (barDur = 1.6 s ⇒ 5.06 + 4 bars = 11.46), riser over the
        // 1.46 s span, `fadeOut(t0, min(0.25, fade))`, `fadeIn(t0, 0.008)`.
        let a = st.play(Some("battle_final"), Some(1.2), 10.0);
        assert_eq!(a.len(), 2);
        expect_fade_out(&a[0], 11.46, 0.25, Some(1.46));
        expect_start(&[a[1]], "battle_final", 11.46, 0.008);
        // Stop: `fadeOut(now, max(0.03, fade))`, current cleared.
        let a = st.play(None, Some(0.5), 20.0);
        assert_eq!(a.len(), 1);
        expect_fade_out(&a[0], 20.0, 0.5, None);
        assert_eq!(st.current, None);
        // Stopping again is silent (L1391 `if (cur)`).
        assert!(st.stop(None, 25.0).is_empty());
    }

    #[test]
    fn play_fade_floor_and_unknown_track() {
        let mut st = MusicState::default();
        // `fade = 0` still floors the crossfade at 0.03 s (L1391/L1413).
        let a = st.play(Some("battle"), Some(0.0), 0.0);
        expect_start(&a, "battle", 0.06, 0.01);
        let a = st.play(Some("menu"), Some(0.0), 1.0);
        assert_eq!(a.len(), 2);
        expect_fade_out(&a[0], 1.0, 0.03, None);
        // fade 0 → cold `fadeIn(0.01)` (L1414 `fade > 0 ? fade*0.5 : 0.01`).
        expect_start(&[a[1]], "menu", 1.06, 0.01);
        // Unknown id: warn + keep the live track (L1384-1386).
        assert!(st.play(Some("nope"), Some(1.0), 2.0).is_empty());
        assert_eq!(st.current.map(|c| c.track), Some("menu"));
    }

    #[test]
    fn remap_routes_battle_requests_to_boss() {
        // ≡ bossAudio.js L58: battle/battle_final resolve to `st.track ||
        // phaseTrack()` while active and not dead.
        let mut st = MusicState {
            remap: BossRemap {
                active: true,
                dead: false,
                phase: 2,
                track: None,
            },
            ..Default::default()
        };
        let a = st.play(Some("battle"), Some(1.2), 0.0);
        expect_start(&a, "boss_2", 0.06, 0.01);
        // Explicit `st.track` wins over the phase fallback.
        let st2 = MusicState {
            remap: BossRemap {
                active: true,
                dead: false,
                phase: 3,
                track: Some("boss"),
            },
            ..Default::default()
        };
        assert_eq!(st2.remap.apply(Some("battle_final")), Some("boss"));
        // After the defeat the routing stops (L58 `!st.dead`).
        st.remap.dead = true;
        let a = st.play(Some("battle_final"), Some(1.2), 5.0);
        assert_eq!(a.len(), 2);
        expect_start(&[a[1]], "battle_final", 5.06, 0.6);
        // Non-battle requests are untouched.
        assert_eq!(st.remap.apply(Some("menu")), Some("menu"));
        assert_eq!(st.remap.apply(None), None);
    }

    #[test]
    fn duck_merge_and_envelope() {
        // ≡ audio.js L364-378.
        let mut d = DuckState::default();
        assert_eq!(d.value_at(0.0), 1.0);
        d.duck(0.5, 1.0, 0.0);
        assert!(approx(d.value_at(0.0), 1.0) < 1e-12);
        assert!(approx(d.value_at(0.08), 0.5) < 1e-12);
        assert!(approx(d.value_at(1.08), 0.5) < 1e-12);
        assert!(approx(d.value_at(1.68), 1.0) < 1e-12);
        // Overlap: deeper target wins, later `until` wins (L370).
        d.duck(0.2, 1.0, 0.5);
        assert!(approx(d.value_at(0.5), 0.5) < 1e-12);
        assert!(approx(d.value_at(1.58), 0.5) < 1e-12);
        assert!(approx(d.value_at(2.18), 1.0) < 1e-12);
        // Mid release ramp: linear between (1.58, 0.5) and (2.18, 1).
        assert!(approx(d.value_at(1.88), 0.75) < 1e-12);
        // Amount clamps (L367).
        d.duck(2.0, 0.0, 3.0);
        assert!(approx(d.value_at(3.08), 0.0) < 1e-12);
    }

    #[test]
    fn taper_matches_js() {
        // ≡ audio.js L24: pow(clamp(v,0,1), 1.5).
        assert!(approx(taper(0.8), 0.8_f64.powf(1.5)) < 1e-12);
        assert!(approx(taper(0.8), 0.7155417527999329) < 1e-12);
        assert_eq!(taper(-0.5), 0.0);
        assert_eq!(taper(2.0), 1.0);
        assert_eq!(taper(0.0), 0.0);
        assert_eq!(taper(1.0), 1.0);
    }

    #[test]
    fn set_volumes_smooths_like_js() {
        // ≡ audio.js L165-172: finite sliders re-target with τ = 0.04.
        let mut v = AudioVolumes::default();
        assert!(approx(v.master.value_at(0.0), taper(0.8)) < 1e-12);
        assert!(approx(v.music.value_at(0.0), taper(0.6)) < 1e-12);
        assert!(approx(v.sfx.value_at(0.0), taper(0.85)) < 1e-12);
        // The mute regression (TR-50.2 headless part): music to zero.
        v.set_volumes(None, Some(0.0), None, 0.0);
        assert!(approx(v.music.value_at(0.0), taper(0.6)) < 1e-9); // starts at the old value
        assert!(v.music.value_at(1.0) < 1e-6); // τ = 0.04 ⇒ settled to 0
        assert!(approx(v.master.value_at(5.0), taper(0.8)) < 1e-12); // untouched
        // Non-finite / clamping: NaN keeps the value, 2.0 clamps to full.
        v.set_volumes(Some(f64::NAN), None, Some(2.0), 10.0);
        assert!(approx(v.master.value_at(20.0), taper(0.8)) < 1e-9);
        assert!(approx(v.sfx.value_at(20.0), 1.0) < 1e-6);
    }

    #[test]
    fn runtime_request_tracks_voices() {
        // Voice bookkeeping ≡ the JS gain automation targets.
        let mut rt = MusicRuntime::default();
        rt.request(Some("menu"), None, 0.0);
        assert_eq!(rt.voices.len(), 1);
        assert!(approx(rt.voices[0].t0, 0.06) < 1e-12);
        assert_eq!(rt.voices[0].entity, None);
        rt.request(Some("battle"), Some(1.2), 5.0);
        assert_eq!(rt.voices.len(), 2);
        // The old voice fades from 1.0 over 1.2 s, reaped at +0.02 s grace.
        let v = &rt.voices[0];
        assert!(approx(v.fade.t0, 5.0) < 1e-12);
        assert!(approx(v.fade.v0, 1.0) < 1e-12);
        assert!(approx(v.fade.t1, 6.2) < 1e-12);
        assert!(approx(v.stop_at, 6.22) < 1e-12);
        // Duck rides the runtime bus (≡ `audio.duck`).
        rt.duck(0.5, 1.0, 7.0);
        assert!(approx(rt.duck.value_at(7.08), 0.5) < 1e-12);
    }

    #[test]
    fn compile_song_anchors() {
        // Fixed data anchors against music.js `SONGS`. `loop_from` is the
        // compiled *bar* index (≡ JS `secStart[loopFrom]`, music.js L776):
        // title's intro is 4 bars, `results_win` loops from the top.
        assert_eq!(get_song("title").unwrap().loop_from, 4);
        assert_eq!(get_song("results_win").unwrap().loop_from, 0);
        assert_eq!(get_song("title").unwrap().def.loop_from, Some(1));
        assert_eq!(get_song("results_win").unwrap().def.loop_from, None);
        assert!(approx(step_dur(150.0), 0.1) < 1e-12);
        assert!(approx(bar_dur(150.0), 1.6) < 1e-12);
        let bf = get_song("battle_final").unwrap();
        assert!(approx(bf.mix("hats"), 0.17) < 1e-12);
        assert!(approx(bf.mix("lead2"), 0.16) < 1e-12);
        let menu = get_song("menu").unwrap();
        assert!(approx(menu.mix("kick"), 0.5) < 1e-12);
        assert_eq!(menu.def.bpm, 105.0);
        assert_eq!(menu.def.swing, 0.16);
        assert!(approx(taper(1.0), 1.0) < 1e-12);
        // Every track compiles without warnings and has bars.
        for id in TRACKS {
            let c = get_song(id).unwrap_or_else(|| panic!("missing {id}"));
            assert!(!c.bars.is_empty(), "{id} compiled empty");
            assert!(c.warnings.is_empty(), "{id} warnings: {:?}", c.warnings);
            assert!(c.loop_from < c.bars.len(), "{id} loop_from");
        }
    }

    #[test]
    fn bake_short_segment_is_finite_deterministic() {
        // One bar of `results_lose` (84 bpm ≈ 2.86 s) — the per-note bake
        // path (instruments + pump + open-hat cutoff + mix scaling).
        let comp = get_song("results_lose").unwrap();
        let a = bake_song_bars(&comp, 1);
        assert!(!a.is_empty());
        assert!(a.iter().all(|s| s.is_finite()));
        assert!(a.iter().all(|s| s.abs() <= 1.0));
        assert!(
            a.iter().any(|s| s.abs() > 0.01),
            "segment must not be silent"
        );
        // Determinism: same seed, same bytes.
        let b = bake_song_bars(&comp, 1);
        assert_eq!(a, b);
    }

    #[test]
    #[ignore = "whole-song bakes are minutes of per-note graph rendering"]
    fn full_song_bake_smoke() {
        // Lazy verification: every synth track bakes end to end through the
        // runtime asset path. The battle set is the long pole (~38 bars);
        // this test is opt-in.
        let mut assets: Assets<PcmSound> = Assets::default();
        let mut ma = MusicAssets::default();
        for id in TRACKS {
            let t = std::time::Instant::now();
            let dur = ma
                .prebake(&mut assets, id)
                .unwrap_or_else(|| panic!("bake {id}"));
            println!(
                "[music-bake] {id}: {dur:.1} s of audio in {:.1} s of bake",
                t.elapsed().as_secs_f64()
            );
            assert!(dur > 0.0);
        }
    }
}
