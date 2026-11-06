//! Pure-data ripple table (M4 Task 52 batch E1, spec D-T52-5).
//!
//! Port of the presentation-side ripple pool from `src/world/paint.js`
//! (the sim crate's ripple branch was cut — spec 调研锚点 "Rust 现状").
//! Every formula is anchored `// ≡ paint.js L###` / `// ≡ inkShading.js L###`
//! against the JS source; no renderer wiring here (batch E2 feeds
//! [`RippleTable::pack_uniforms`] into `LevelExt::u_rip` / `u_rip_p`).
//!
//! Layouts (verbatim from the JS Float32Arrays):
//! * `rip[i*4 + 0..3]` = ripple centre xyz, `+ 3` = **birth** (paint clock at
//!   insert, ≡ paint.js L584 `rip[o+3] = this.clock`);
//! * `rip_p[i*4 + 0..3]` = `(amp, wavelength, speed, life)` (≡ L585);
//! * `rip_s[i]` = the stored priority score (≡ L586, `_ripS`).
//!
//! The shader consumes them as `uRip`/`uRipP` (`[Vec4; 24]`, level.wgsl
//! bindings 31/32 ≡ inkShading.js L51-52) plus `uInkClock` = [`RippleTable::clock`].
//! Batch E2 wired the feed: `DemoSim::drain_atlas` splats + capture-flood
//! swells → [`crate::level_render::level_material_wiring`] → materials.

use bevy::prelude::Resource;
use glam::{Vec3, Vec4};

/// `RIP_N` (paint.js L282-284) ≡ `INK_RIPPLES` (inkShading.js L22) ≡
/// `embedded_texlib().ink.ripples` (texlib/schema.rs).
pub const RIP_N: usize = 24;

/// View-cull radius, m (≡ paint.js L571 `d2 > 38 * 38`).
pub const VIEW_CULL_M: f32 = 38.0;
/// Distance term in the priority score (≡ L573 `1 + d2 * 0.012`).
const SCORE_DIST_K: f32 = 0.012;
/// Dedup window, s (≡ L556 `age > 0.16`).
const NEAR_WINDOW_S: f32 = 0.16;
/// Dedup base amplitude threshold, m (≡ L556 `0.0036 + …`).
const NEAR_AMP_BASE: f32 = 0.0036;
/// Dedup reach at radius 0, m (≡ L553 `0.55 + radius * 0.6`).
const NEAR_REACH0: f32 = 0.55;
/// Minimum splat radius that makes a ripple (≡ L537 `radius >= 0.15`).
const SPLAT_RIPPLE_MIN_R: f32 = 0.15;
/// Default `ripple()` parameters, `(amp, wavelength, speed, life)`
/// (≡ paint.js L565 default arguments; showcase/gamepad callers, Task 57).
#[allow(dead_code)] // JS default-argument anchor; first consumer is Task 57
pub const RIPPLE_DEFAULT: (f32, f32, f32, f32) = (0.006, 0.14, 1.2, 0.7);

/// Splat-replay event feeding the table (≡ the JS `splat()` tail, L537-541).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplatEvent {
    /// Splat centre, world (JS `center`).
    pub center: Vec3,
    /// Splat radius, m (JS `radius`).
    pub radius: f32,
    /// JS `opts.cosmetic` — cosmetic splats never ripple (L537 `!cosmetic`).
    pub cosmetic: bool,
}

/// Flood-front jet feeding the table (≡ the JS `flood()` swell, L729).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloodJet {
    /// Front centre (x, z) — JS `c[0]`, `c[1]` (L719/L724 `cx`/`cz`).
    pub center: [f32; 2],
    /// Ink surface height, m; `None` = JS `opts.y == null` → no swell (L729).
    pub y: Option<f32>,
    /// Front reach, m (JS `J.reach`, L724).
    pub reach: f32,
    /// Duration, s — the *resolved* `J.dur` (L723-726: `max(0, +opts.dur || 0)`,
    /// defaulting to 0.5 when `opts.dur` is absent).
    pub dur: f32,
    /// JS `J.team` (already clamped to 0/1/-1 by the flood ctor, L723);
    /// `team < 0` (rotation wipe) sends no swell (L729 `J.team >= 0`).
    pub team: i8,
}

/// One input of the ripple feed (batch E2: `DemoSim::drain_atlas` splat
/// stream → [`SplatEvent`] via [`splat_event_from_ink`]; `fx/zone_marks.rs`
/// capture/wipe fronts → [`FloodJet`] from `FloodRec` + `ZoneState.center`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RippleSource {
    Splat(SplatEvent),
    FloodJet(FloodJet),
}

/// `inkwave_sim::paint::InkSplat` → [`SplatEvent`].
///
/// The sim's [`inkwave_sim::paint::SplatOpts`] carries no `cosmetic` flag
/// (JS `opts.cosmetic` only ever comes from `speck()`, paint.js L547-549,
/// which forces `kind: 'speck'` and clamps `radius ≤ 0.12`), so
/// `kind == Speck` is the faithful replay-side equivalent of `!cosmetic`.
/// Specks would also fail the `radius >= 0.15` gate on their own.
#[must_use]
pub fn splat_event_from_ink(s: &inkwave_sim::paint::InkSplat) -> SplatEvent {
    SplatEvent {
        center: s.center,
        radius: s.radius,
        cosmetic: s.opts.kind == Some(inkwave_sim::paint::SplatKind::Speck),
    }
}

/// Splat impact ripple parameters (≡ paint.js L540):
/// `ripple(center, 0.0038 + 0.0036·min(r,3), 0.1 + 0.05·min(r,3),
/// 0.85 + 0.35·min(r,3), 0.55 + 0.2·min(r,3))` → `(amp, wl, speed, life)`.
#[must_use]
pub fn splat_params(r: f32) -> (f32, f32, f32, f32) {
    let k = r.min(3.0);
    (
        0.0038 + 0.0036 * k,
        0.1 + 0.05 * k,
        0.85 + 0.35 * k,
        0.55 + 0.2 * k,
    )
}

/// Flood swell ripple parameters (≡ paint.js L729):
/// `ripple(pos, 0.03, 0.45, J.reach / max(0.2, J.dur) * 0.85,
/// max(0.6, J.dur * 1.6))` → `(amp, wl, speed, life)`.
#[must_use]
pub fn flood_jet_params(reach: f32, dur: f32) -> (f32, f32, f32, f32) {
    (
        0.03,
        0.45,
        reach / dur.max(0.2) * 0.85,
        (dur * 1.6).max(0.6),
    )
}

/// The RIP_N-slot priority-retaining ripple ring (≡ paint.js L282-284 +
/// `ripple()` L565-587 + `_rippledNear()` L551-561).
/// Batch E2: lives as a `Resource` fed by `DemoSim::drain_atlas` + the
/// capture-flood wiring and packed into the level materials each frame.
#[derive(Debug, Clone, Resource)]
pub struct RippleTable {
    /// xyz + birth (paint clock at insert) — JS `this.rip`.
    rip: [f32; RIP_N * 4],
    /// amp, wavelength, speed, life — JS `this.ripP`.
    rip_p: [f32; RIP_N * 4],
    /// Stored priority score — JS `this._ripS`.
    rip_s: [f32; RIP_N],
    /// Paint clock, s (JS `this.clock`, advanced per flush, L677).
    pub clock: f32,
    /// Camera position for priority/culling (JS `this.viewPos`, L280/L452).
    view_pos: Option<Vec3>,
}

impl Default for RippleTable {
    /// ≡ paint.js constructor L282-284: fresh `Float32Array`s (all zeros).
    /// A zero slot reads `age = clock - 0 ≥ life = 0` → dead, so the table
    /// starts empty in the eviction scan.
    fn default() -> Self {
        Self {
            rip: [0.0; RIP_N * 4],
            rip_p: [0.0; RIP_N * 4],
            rip_s: [0.0; RIP_N],
            clock: 0.0,
            view_pos: None,
        }
    }
}

impl RippleTable {
    /// ≡ paint.js `setView(pos)` (L452): `None` disables view culling
    /// (ripples score as if at the camera, `d2 = 0`).
    pub fn set_view(&mut self, pos: Option<Vec3>) {
        self.view_pos = pos;
    }

    /// ≡ paint.js `flush()` clock tick (L677 `this.clock += dt`).
    pub fn advance(&mut self, dt: f32) {
        self.clock += dt;
    }

    /// Resync the clock (match restart / net replay re-anchor; JS rebuilds
    /// the whole `PaintGrid` instead). Dead-slot detection tolerates the
    /// jump: `age < -1` counts as free, ≡ L578.
    #[allow(dead_code)] // JS anchor; the restart-path consumer lands with Task 57
    pub fn set_clock(&mut self, clock: f32) {
        self.clock = clock;
    }

    /// ≡ paint.js reset ripple branch (L443): birth = -99, life = 0.01,
    /// score = 0 for every slot (xyz/amp/wl/speed keep their stale values,
    /// exactly like the JS in-place `Float32Array` poke — dead either way).
    #[allow(dead_code)] // JS anchor; the restart-path consumer lands with Task 57
    pub fn reset(&mut self) {
        for i in 0..RIP_N {
            self.rip[i * 4 + 3] = -99.0;
            self.rip_p[i * 4 + 3] = 0.01;
            self.rip_s[i] = 0.0;
        }
    }

    /// "A ripple of at least this size already started nearby within the
    /// last ~0.16 s?" ≡ paint.js `_rippledNear(pos, radius)` (L551-561).
    ///
    /// The amplitude test compares the **stored** ripple amp `rip_p[o]`
    /// against a threshold built from the *incoming* radius
    /// (`0.0036 + 0.0036·min(radius,3)·0.7`, L556).
    #[must_use]
    pub fn rippled_near(&self, pos: Vec3, radius: f32) -> bool {
        let reach = NEAR_REACH0 + radius * 0.6; // ≡ L553
        let amp_min = NEAR_AMP_BASE + NEAR_AMP_BASE * radius.min(3.0) * 0.7; // ≡ L556
        for i in 0..RIP_N {
            let o = i * 4;
            let age = self.clock - self.rip[o + 3];
            if age < 0.0 || age > NEAR_WINDOW_S || self.rip_p[o] < amp_min {
                continue;
            }
            let dx = self.rip[o] - pos.x;
            let dy = self.rip[o + 1] - pos.y;
            let dz = self.rip[o + 2] - pos.z;
            if dx * dx + dy * dy + dz * dz < reach * reach {
                return true;
            }
        }
        false
    }

    /// Insert a ripple, keeping the RIP_N most important live ones
    /// (size × nearness × life left) ≡ paint.js `ripple(pos, amp,
    /// wavelength, speed, life)` (L565-587). Returns whether it was kept.
    #[allow(clippy::too_many_arguments)] // verbatim JS parameter list (L565)
    pub fn ripple(&mut self, pos: Vec3, amp: f32, wavelength: f32, speed: f32, life: f32) -> bool {
        // view cull + score (L566-573)
        let mut d2 = 0.0;
        if let Some(vp) = self.view_pos {
            let dx = pos.x - vp.x;
            let dy = pos.y - vp.y;
            let dz = pos.z - vp.z;
            d2 = dx * dx + dy * dy + dz * dz;
            if d2 > VIEW_CULL_M * VIEW_CULL_M {
                return false;
            }
        }
        let score = amp / (1.0 + d2 * SCORE_DIST_K);
        // find the victim: first dead slot wins outright, else the live slot
        // with the weakest remaining score (L574-581)
        let mut best: Option<usize> = None;
        let mut best_s = f32::INFINITY;
        for i in 0..RIP_N {
            let o = i * 4;
            let age = self.clock - self.rip[o + 3];
            let l = self.rip_p[o + 3];
            if age >= l || age < -1.0 {
                best = Some(i);
                best_s = -1.0;
                break;
            }
            let s = self.rip_s[i] * (1.0 - age / l);
            if s < best_s {
                best_s = s;
                best = Some(i);
            }
        }
        // refuse to evict something still more important (L582)
        let Some(best) = best else {
            return false;
        };
        if best_s > score {
            return false;
        }
        let o = best * 4;
        self.rip[o] = pos.x;
        self.rip[o + 1] = pos.y;
        self.rip[o + 2] = pos.z;
        self.rip[o + 3] = self.clock;
        self.rip_p[o] = amp;
        self.rip_p[o + 1] = wavelength;
        self.rip_p[o + 2] = speed;
        self.rip_p[o + 3] = life;
        self.rip_s[best] = score;
        true
    }

    /// The splat → ripple trigger gate ≡ paint.js L537-541: a non-cosmetic
    /// splat of radius ≥ 0.15 that has no qualifying near neighbour makes
    /// one impact ripple with [`splat_params`]. Returns whether it fired.
    pub fn on_splat(&mut self, ev: &SplatEvent) -> bool {
        if ev.cosmetic || ev.radius < SPLAT_RIPPLE_MIN_R {
            return false;
        }
        if self.rippled_near(ev.center, ev.radius) {
            return false;
        }
        let (amp, wl, speed, life) = splat_params(ev.radius);
        self.ripple(ev.center, amp, wl, speed, life)
    }

    /// The flood-front swell ≡ paint.js L729: only a team flood
    /// (`team >= 0`) with a known ink height (`y != null`) ripples, at
    /// `(cx, y, cz)` with [`flood_jet_params`]. Returns whether it fired.
    pub fn on_flood_jet(&mut self, jet: &FloodJet) -> bool {
        if jet.team < 0 {
            return false;
        }
        let Some(y) = jet.y else { return false };
        let (amp, wl, speed, life) = flood_jet_params(jet.reach, jet.dur);
        let pos = Vec3::new(jet.center[0], y, jet.center[1]);
        self.ripple(pos, amp, wl, speed, life)
    }

    /// Feed one source event (batch E2 pumps the drained splat stream and
    /// the zone-mark flood fronts through this).
    pub fn apply(&mut self, src: &RippleSource) -> bool {
        match src {
            RippleSource::Splat(ev) => self.on_splat(ev),
            RippleSource::FloodJet(jet) => self.on_flood_jet(jet),
        }
    }

    /// Per-draw uniform pack ≡ inkShading.js `inkBeforeRender(U, P)`
    /// (L34-45): a verbatim 4-float copy of `rip` → `uRip` and `ripP` →
    /// `uRipP` (dead slots stay in the arrays; the shader skips them via
    /// `age <= 0 || age >= life`, level.wgsl L1055-1056). The companion
    /// scalar is `uInkClock = self.clock` (L36) — exposed via [`Self::clock`]
    /// for the upload site, since it is not part of either array.
    #[must_use]
    pub fn pack_uniforms(&self) -> ([Vec4; RIP_N], [Vec4; RIP_N]) {
        let mut u_rip = [Vec4::ZERO; RIP_N];
        let mut u_rip_p = [Vec4::ZERO; RIP_N];
        for i in 0..RIP_N {
            let o = i * 4;
            u_rip[i] = Vec4::new(
                self.rip[o],
                self.rip[o + 1],
                self.rip[o + 2],
                self.rip[o + 3],
            );
            u_rip_p[i] = Vec4::new(
                self.rip_p[o],
                self.rip_p[o + 1],
                self.rip_p[o + 2],
                self.rip_p[o + 3],
            );
        }
        (u_rip, u_rip_p)
    }

    /// Test/inspection view of one slot: `(xyz, birth, amp, wl, speed, life,
    /// score)`.
    #[cfg(test)]
    #[must_use]
    pub fn slot(&self, i: usize) -> (Vec3, f32, f32, f32, f32, f32, f32) {
        let o = i * 4;
        (
            Vec3::new(self.rip[o], self.rip[o + 1], self.rip[o + 2]),
            self.rip[o + 3],
            self.rip_p[o],
            self.rip_p[o + 1],
            self.rip_p[o + 2],
            self.rip_p[o + 3],
            self.rip_s[i],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `score = amp / (1 + d²·0.012)` pinned (≡ L573), incl. the no-view
    /// case (`d2 = 0` → score = amp) and the 38 m boundary (L571 `d2 > 38²`
    /// is strict: exactly 38 m still passes).
    #[test]
    fn score_formula_pins() {
        let mut t = RippleTable::default();
        // no view: score == amp
        assert!(t.ripple(Vec3::ZERO, 0.006, 0.14, 1.2, 0.7));
        assert_eq!(t.slot(0).6, 0.006);
        // view 10 m away: score = amp / (1 + 100·0.012) = amp / 2.2
        let mut t = RippleTable::default();
        t.set_view(Some(Vec3::new(10.0, 0.0, 0.0)));
        assert!(t.ripple(Vec3::ZERO, 0.011, 0.14, 1.2, 0.7));
        assert!((t.slot(0).6 - 0.011 / 2.2).abs() < 1e-7);
        // exactly 38 m: kept (strict >); 38.01 m: rejected
        assert!(t.ripple(Vec3::new(-28.0, 0.0, 0.0), 0.01, 0.14, 1.2, 0.7)); // 38 m from view? view=(10,0,0), pos=(-28,0,0) → d=38
        assert!(!t.ripple(Vec3::new(-28.01, 0.0, 0.0), 0.01, 0.14, 1.2, 0.7));
    }

    /// The 38 m view cull drops far ripples outright (≡ L568-572).
    #[test]
    fn view_cull_38m() {
        let mut t = RippleTable::default();
        t.set_view(Some(Vec3::ZERO));
        assert!(!t.ripple(Vec3::new(39.0, 0.0, 0.0), 0.03, 0.45, 1.0, 0.7));
        assert_eq!(t.slot(0).0, Vec3::ZERO); // untouched (zeroed birth=life=0)
        assert!(t.ripple(Vec3::new(37.9, 0.0, 0.0), 0.03, 0.45, 1.0, 0.7));
    }

    /// Full-table score retention ≡ L574-586: with 24 live slots the
    /// weakest *remaining* score (`rip_s · (1 − age/L)`) is evicted, and an
    /// incoming ripple weaker than the weakest is refused.
    #[test]
    fn full_table_score_eviction_order() {
        let mut t = RippleTable::default();
        // 24 live ripples, scores 1..=24 (amp = score, no view → d2 = 0),
        // life 10 s so all stay alive; slot k holds score k+1.
        for k in 0..RIP_N {
            let pos = Vec3::new(k as f32 * 0.01, 0.0, 0.0); // distinct, near-crowded
            assert!(t.ripple(pos, (k + 1) as f32, 0.14, 1.2, 10.0));
            assert_eq!(t.slot(k).6, (k + 1) as f32, "fills in index order");
        }
        // weaker than the weakest → refused, table unchanged
        assert!(!t.ripple(Vec3::new(5.0, 0.0, 0.0), 0.5, 0.14, 1.2, 10.0));
        assert_eq!(t.slot(0).1, 0.0, "slot 0 still birth 0");
        // stronger than the weakest → evicts slot 0 (score 1)
        assert!(t.ripple(Vec3::new(5.0, 0.0, 0.0), 1.5, 0.2, 0.9, 10.0));
        let (pos, birth, amp, wl, speed, life, score) = t.slot(0);
        assert_eq!(pos, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(birth, t.clock); // ≡ L584 rip[o+3] = this.clock
        assert_eq!((amp, wl, speed, life), (1.5, 0.2, 0.9, 10.0)); // ≡ L585
        assert_eq!(score, 1.5);
        // the victim is the weakest *remaining* score rip_s·(1−age/L)
        // (≡ L579-580), not the raw-score weakest: slot 0 (raw 1.5, life 10)
        // and slot 1 (raw 2, life 10) both decay, but an incoming 1.2 >
        // slot 0's remaining 1.5·(1−2/10)=1.2 is kept out (best_s > score),
        // while 1.21 evicts slot 0 even though its raw score 1.5 > 1.21.
        t.advance(2.0);
        assert!(!t.ripple(Vec3::new(6.0, 0.0, 0.0), 1.19, 0.14, 1.2, 10.0));
        assert_eq!(t.slot(0).0, Vec3::new(5.0, 0.0, 0.0));
        assert!(t.ripple(Vec3::new(6.0, 0.0, 0.0), 1.21, 0.14, 1.2, 10.0));
        assert_eq!(t.slot(0).0, Vec3::new(6.0, 0.0, 0.0));
    }

    /// Dead slots (age ≥ life) are reused before any eviction contest, and
    /// the first dead index wins outright even against a tiny score
    /// (bestS = −1 ≤ score, ≡ L578 + L582). `age < −1` (clock jumped back)
    /// also counts as free.
    #[test]
    fn dead_slot_reuse_and_negative_age() {
        let mut t = RippleTable::default();
        assert!(t.ripple(Vec3::X, 100.0, 0.14, 1.2, 1.0)); // slot 0, huge score
        t.advance(1.0); // age = 1 ≥ life → dead
        assert!(t.ripple(Vec3::Z, 0.001, 0.14, 1.2, 0.7)); // evicts slot 0 anyway
        assert_eq!(t.slot(0).0, Vec3::Z);
        // clock resync backwards by > 1 s: slot 0 (birth 1, clock −0.5) has
        // age = −1.5 < −1 → free slot again
        t.set_clock(-0.5);
        assert!(t.ripple(Vec3::Y, 0.001, 0.14, 1.2, 0.7));
        assert_eq!(t.slot(0).0, Vec3::Y);
    }

    /// `splat_params(r)` ≡ paint.js L540 at representative radii (the
    /// `min(r, 3)` clamp saturates at 3).
    #[test]
    fn splat_params_pins() {
        // eps 1e-6: the f32 ulp at amp ≈ 0.0146 is ≈ 1.9e-9, so the JS
        // literals match the product/sum only to ~1e-8.
        let eps = 1e-6;
        assert!(close(splat_params(0.0), (0.0038, 0.1, 0.85, 0.55), eps));
        assert!(close(splat_params(1.0), (0.0074, 0.15, 1.2, 0.75), eps));
        assert!(close(splat_params(3.0), (0.0146, 0.25, 1.9, 1.15), eps));
        assert!(close(splat_params(5.0), (0.0146, 0.25, 1.9, 1.15), eps));
    }

    /// `flood_jet_params(reach, dur)` ≡ paint.js L729: amp/wl are literals,
    /// speed = reach/max(0.2,dur)·0.85, life = max(0.6, dur·1.6).
    #[test]
    fn flood_jet_params_pins() {
        assert!(close(
            flood_jet_params(10.0, 0.5),
            (0.03, 0.45, 17.0, 0.8),
            1e-6
        ));
        assert!(close(
            flood_jet_params(10.0, 0.2),
            (0.03, 0.45, 42.5, 0.6),
            1e-6
        ));
        // dur floored at 0.2 in the speed term, life floored at 0.6
        assert!(close(
            flood_jet_params(10.0, 0.1),
            (0.03, 0.45, 42.5, 0.6),
            1e-6
        ));
        assert!(close(
            flood_jet_params(3.0, 1.0),
            (0.03, 0.45, 2.55, 1.6),
            1e-6
        ));
    }

    /// The splat trigger gate ≡ L537: cosmetic never ripples; r < 0.15
    /// never ripples; otherwise one ripple with the L540 parameters.
    #[test]
    fn splat_gate_semantics() {
        let mut t = RippleTable::default();
        assert!(!t.on_splat(&SplatEvent {
            center: Vec3::ZERO,
            radius: 0.9,
            cosmetic: true
        }));
        assert!(!t.on_splat(&SplatEvent {
            center: Vec3::ZERO,
            radius: 0.149,
            cosmetic: false
        }));
        assert!(t.on_splat(&SplatEvent {
            center: Vec3::new(1.0, 2.0, 3.0),
            radius: 0.15,
            cosmetic: false
        }));
        let (pos, _, amp, wl, speed, life, _) = t.slot(0);
        assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!((amp, wl, speed, life), splat_params(0.15));
    }

    /// `_rippledNear` dedup ≡ L551-561: 0.16 s window, reach 0.55 + r·0.6,
    /// amp threshold 0.0036 + 0.0036·min(r,3)·0.7 — pinned at r = 0 / 1 / 3.
    #[test]
    fn rippled_near_gate_pins() {
        let mut t = RippleTable::default();
        // a 0.0074-amp ripple (the r=1 splat amp) at the origin, birth 0
        t.ripple(Vec3::ZERO, 0.0074, 0.14, 1.2, 0.7);
        t.advance(0.1); // age 0.1 ∈ [0, 0.16]
        // incoming r = 1: reach 1.15, threshold 0.0036+0.0036·0.7 = 0.00612
        assert!(t.rippled_near(Vec3::new(1.0, 0.0, 0.0), 1.0));
        assert!(!t.rippled_near(Vec3::new(1.2, 0.0, 0.0), 1.0)); // outside reach²
        // incoming r = 0: threshold 0.0036 ≤ 0.0074, reach 0.55
        assert!(t.rippled_near(Vec3::new(0.5, 0.0, 0.0), 0.0));
        assert!(!t.rippled_near(Vec3::new(0.6, 0.0, 0.0), 0.0));
        // incoming r = 3: threshold 0.0036·(1+3·0.7) = 0.01116 > 0.0074 → miss
        assert!(!t.rippled_near(Vec3::ZERO, 3.0));
        // window: age > 0.16 stops counting (L556)
        t.advance(0.1); // age 0.2
        assert!(!t.rippled_near(Vec3::ZERO, 0.0));
        // future birth (age < 0) also misses (L556 `age < 0`)
        let mut t2 = RippleTable::default();
        t2.ripple(Vec3::ZERO, 0.01, 0.14, 1.2, 0.7); // birth 0
        t2.set_clock(-0.5); // age = -0.5 - 0 = -0.5 < 0 → miss
        assert!(!t2.rippled_near(Vec3::ZERO, 0.0));
    }

    /// The gate actually dedupes: a second splat inside the window/radius
    /// makes no ripple, a distant one does (≡ L537 call site).
    #[test]
    fn splat_gate_dedupes_via_rippled_near() {
        let mut t = RippleTable::default();
        let big = SplatEvent {
            center: Vec3::ZERO,
            radius: 2.0,
            cosmetic: false,
        };
        assert!(t.on_splat(&big));
        assert!(!t.on_splat(&SplatEvent {
            center: Vec3::new(1.0, 0.0, 0.0), // within reach 0.55+2·0.6 = 1.75
            radius: 2.0,
            cosmetic: false,
        }));
        assert_eq!(
            t.slot(1).1,
            0.0,
            "second splat inserted nothing (slot 1 still default birth 0)"
        );
        assert!(t.on_splat(&SplatEvent {
            center: Vec3::new(3.0, 0.0, 0.0),
            radius: 2.0,
            cosmetic: false,
        }));
        assert_eq!(t.slot(1).0.x, 3.0);
    }

    /// Flood swell gate ≡ L729: team < 0 (wipe) or `y == None` sends
    /// nothing; a live flood ripples at (cx, y, cz) with L729 params.
    #[test]
    fn flood_jet_gate_and_ripple() {
        let mut t = RippleTable::default();
        assert!(!t.on_flood_jet(&FloodJet {
            center: [0.0, 0.0],
            y: Some(1.0),
            reach: 10.0,
            dur: 0.5,
            team: -1,
        }));
        assert!(!t.on_flood_jet(&FloodJet {
            center: [0.0, 0.0],
            y: None,
            reach: 10.0,
            dur: 0.5,
            team: 0,
        }));
        assert!(t.on_flood_jet(&FloodJet {
            center: [2.0, 3.0],
            y: Some(0.5),
            reach: 10.0,
            dur: 0.5,
            team: 1,
        }));
        let (pos, _, amp, wl, speed, life, _) = t.slot(0);
        assert_eq!(pos, Vec3::new(2.0, 0.5, 3.0));
        assert_eq!((amp, wl, speed, life), flood_jet_params(10.0, 0.5));
    }

    /// `pack_uniforms` ≡ inkShading.js `inkBeforeRender` (L40-44): verbatim
    /// copies — uRip = (x, y, z, birth), uRipP = (amp, wl, speed, life),
    /// dead slots included unchanged; `clock` is the separate `uInkClock`
    /// scalar (L36).
    #[test]
    fn pack_uniforms_verbatim_copy() {
        let mut t = RippleTable::default();
        t.ripple(Vec3::new(1.0, 2.0, 3.0), 0.006, 0.14, 1.2, 0.7);
        t.advance(0.3);
        t.ripple(Vec3::new(-4.0, 5.0, -6.0), 0.03, 0.45, 17.0, 0.8);
        let (u_rip, u_rip_p) = t.pack_uniforms();
        assert_eq!(u_rip[0], Vec4::new(1.0, 2.0, 3.0, 0.0)); // birth = clock at insert
        assert_eq!(u_rip_p[0], Vec4::new(0.006, 0.14, 1.2, 0.7));
        assert_eq!(u_rip[1], Vec4::new(-4.0, 5.0, -6.0, 0.3));
        assert_eq!(u_rip_p[1], Vec4::new(0.03, 0.45, 17.0, 0.8));
        // untouched slots copy the zeroed arrays (shader kills them by age)
        assert_eq!(u_rip[23], Vec4::ZERO);
        assert_eq!(u_rip_p[23], Vec4::ZERO);
        assert_eq!(t.clock, 0.3); // ≡ U.uInkClock.value = P.clock (L36)
    }

    /// `reset()` ≡ paint.js L443: birth −99 / life 0.01 / score 0, xyz and
    /// the other ripP fields keep their stale values (dead either way).
    #[test]
    fn reset_pokes_like_js() {
        let mut t = RippleTable::default();
        t.ripple(Vec3::new(7.0, 8.0, 9.0), 0.5, 0.2, 1.0, 2.0);
        t.reset();
        let (pos, birth, amp, wl, speed, life, score) = t.slot(0);
        assert_eq!(
            pos,
            Vec3::new(7.0, 8.0, 9.0),
            "xyz untouched (JS pokes only o+3)"
        );
        assert_eq!((birth, life, score), (-99.0, 0.01, 0.0));
        assert_eq!((amp, wl, speed), (0.5, 0.2, 1.0), "ripP xyz untouched");
    }

    /// `RippleSource::apply` dispatches both arms (E2 feed path).
    #[test]
    fn source_apply_dispatch() {
        let mut t = RippleTable::default();
        assert!(t.apply(&RippleSource::Splat(SplatEvent {
            center: Vec3::ZERO,
            radius: 1.0,
            cosmetic: false,
        })));
        assert!(t.apply(&RippleSource::FloodJet(FloodJet {
            center: [20.0, 20.0],
            y: Some(1.0),
            reach: 12.0,
            dur: 0.5,
            team: 0,
        })));
        assert_eq!(t.slot(1).0.x, 20.0);
    }

    /// The RIP_N constant agrees with the extracted texlib document
    /// (`embedded_texlib().ink.ripples` ≡ INK_RIPPLES, inkShading.js L22).
    #[test]
    fn rip_n_matches_texlib_document() {
        assert_eq!(
            RIP_N,
            crate::texlib::schema::embedded_texlib().ink.ripples as usize
        );
    }

    /// `splat_event_from_ink` maps the sim's InkSplat: kind Speck is the
    /// replay-side `cosmetic` flag.
    #[test]
    fn ink_splat_conversion_marks_specks_cosmetic() {
        use inkwave_sim::paint::{InkSplat, SplatKind, SplatOpts};
        let plain = InkSplat {
            center: Vec3::X,
            radius: 0.5,
            team: 0,
            opts: SplatOpts::default(),
            seq: 0,
            replayed: false,
        };
        let speck = InkSplat {
            opts: SplatOpts {
                kind: Some(SplatKind::Speck),
                ..SplatOpts::default()
            },
            radius: 0.1,
            ..plain
        };
        let ev = splat_event_from_ink(&plain);
        assert_eq!(
            ev,
            SplatEvent {
                center: Vec3::X,
                radius: 0.5,
                cosmetic: false
            }
        );
        assert!(splat_event_from_ink(&speck).cosmetic);
    }

    /// Component-wise `|a − b| ≤ eps` for the `(amp, wl, speed, life)` pins.
    fn close(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32), eps: f32) -> bool {
        let (a0, a1, a2, a3) = a;
        let (b0, b1, b2, b3) = b;
        (a0 - b0).abs() <= eps
            && (a1 - b1).abs() <= eps
            && (a2 - b2).abs() <= eps
            && (a3 - b3).abs() <= eps
    }
}
