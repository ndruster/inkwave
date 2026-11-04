//! Task 46 batch C2 — boss HUD state machine (≡ `src/ui/hud-boss.js`).
//!
//! Pure functions: no Bevy, no DOM. The app layer feeds it the drained
//! [`BossEvent`] stream plus a per-frame read of the live boss
//! ([`BossHudFrame`]), and renders [`BossHudOut`] as text / rect placeholders
//! (the JS class writes CSS classes + transforms; the visual polish is a
//! REVIEW_CHECKLIST item).
//!
//! JS mapping: `_fresh` L45-49, `setMode` L100-117, `_intro` L152-160,
//! `_hp` L162-181, `_move` L183-195, `_stun` L232-241, `_phase` L243-255,
//! `_defeat` L257-266, `timesUp` L269-275, `update` L292-334, `_writeBar`
//! L336-346, `_pips` L348-351, `_titleCard` L444-461, `_banner` L463-472,
//! `_ending` L474-494.
//!
//! Placeholder档 declared gaps (DOM-only effects, no data semantics): the tele
//! callout hover cue (`ui_hover`, hud-boss.js L191) and the bar `is-zero`
//! flash class (L180) are not ported — the desk UI has no hover surface and
//! the flash rides the REVIEW_CHECKLIST visual items.

use inkwave_sim::boss::BossEvent;

/// JS `NOTCHES` (hud-boss.js L16): the 66 / 33 % break marks.
pub const NOTCHES: [f64; 2] = [2.0 / 3.0, 1.0 / 3.0];
/// JS `BOSS_NAME` (boss-art.js L10).
pub const BOSS_NAME: &str = "HULLBREAKER";

/// JS `CALL_MOVES` (hud-boss.js L17): the moves that get a callout pill.
#[must_use]
pub fn is_call_move(id: &str) -> bool {
    // JS hud-boss.js L17
    matches!(
        id,
        "slam" | "barrage" | "sweep" | "charge" | "crablets" | "frenzy"
    )
}

/// JS `MOVE_LABELS` (boss-art.js L19-24, `open` included).
#[must_use]
pub fn move_label(id: &str) -> String {
    // JS boss-art.js L19-24; unknown ids fall through to `id.toUpperCase()`
    // (hud-boss.js L199 `MOVE_LABELS[id] || id.toUpperCase()`).
    match id {
        "slam" => "SLAM!",
        "barrage" => "INCOMING!",
        "sweep" => "SWEEP!",
        "charge" => "CHARGE!",
        "crablets" => "BROOD!",
        "frenzy" => "FRENZY!",
        "open" => "OPEN!",
        _ => return id.to_uppercase(),
    }
    .to_string()
}

/// JS `PHASE_TXT` (hud-boss.js L18-21).
fn phase_txt(phase: u32) -> Option<(&'static str, &'static str)> {
    match phase {
        2 => Some(("PHASE 2!", "Crablets incoming — pop them fast!")),
        3 => Some(("SHELL CRACKED!", "Final phase · hit the glowing belly!")),
        _ => None,
    }
}

/// JS `clamp(v, 0, 1)` (ui-util.js L49). Manual form on purpose: `f64::clamp`
/// panics on NaN, the JS ternary chain passes it through (same as
/// `inkwave_sim::boss::clamp`).
#[allow(clippy::manual_clamp)]
fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// JS `easeOutCubic` (ui-util.js L51).
fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// Per-frame live-boss read (JS `this._b()` + `hud._visible` + the
/// `match:state` times-up signal, hud-boss.js L118 / L303 / L269).
#[derive(Debug, Clone, Copy)]
pub struct BossHudFrame<'a> {
    pub hp: f32,
    pub max_hp: f32,
    pub dead: bool,
    pub stunned: bool,
    pub invuln: bool,
    /// `boss.st.move_id` (JS `boss:move` per-phase events, synthesized here
    /// from the live anim state — the sim raises one `Move` per move).
    pub move_id: Option<&'a str>,
    pub move_phase: Option<&'a str>,
    /// JS `this.hud._visible` (drives the intro fill gate, L303).
    pub hud_visible: bool,
    /// The match clock ran out (JS `hud.banner('timesup')` → `timesUp()`).
    pub finished: bool,
}

/// The render descriptor consumed by the HUD (text / rect placeholder档).
#[derive(Debug, Clone, PartialEq)]
pub struct BossHudOut {
    /// JS `is-gone` (hud-boss.js L264): the bar hides 1.3 s after the defeat.
    pub bar_visible: bool,
    /// JS `_writeBar` L342: the ceil'd percent (0.01 floor → `1`).
    pub pct: i64,
    /// Fill / chip fractions for the bar widths (L338-341 transforms).
    pub shown: f32,
    pub chip: f32,
    /// JS `is-low` (L344-345): `shown <= 0.2 && shown > 0`.
    pub low: bool,
    /// JS `_pips` (L348-351): `phase` pips lit.
    pub pips: u32,
    pub phase: u32,
    /// JS `is-enraged` / cracked emblem (L251).
    pub enraged: bool,
    pub cracked: bool,
    /// JS `is-shield` (L325-326).
    pub shield: bool,
    /// JS `is-open` + the draining stun meter (L237 / L321).
    pub open: bool,
    pub open_frac: f32,
    /// JS callout pill (L183-205): the current move label while `is-on`.
    pub callout: Option<String>,
    pub callout_act: bool,
    /// JS `_titleCard` (L444-461): visible ~3.2 s.
    pub title_card: bool,
    /// JS `_banner` (L463-472): visible 3.4 s.
    pub banner: Option<&'static str>,
    pub banner_sub: Option<&'static str>,
    /// JS `_ending` (L474-494): `SUNK!` / `TIME'S UP!`, visible 5.2 s.
    pub ending: Option<&'static str>,
    pub dead: bool,
    /// JS notch `is-past` latches (L177) — for a future tick styling.
    pub notch_past: [bool; 2],
}

impl Default for BossHudOut {
    fn default() -> Self {
        Self {
            bar_visible: false,
            pct: 0,
            shown: 0.0,
            chip: 0.0,
            low: false,
            pips: 1,
            phase: 1,
            enraged: false,
            cracked: false,
            shield: false,
            open: false,
            open_frac: 0.0,
            callout: None,
            callout_act: false,
            title_card: false,
            banner: None,
            banner_sub: None,
            ending: None,
            dead: false,
            notch_past: [false; 2],
        }
    }
}

/// JS `class BossHud` `S` + the event handlers + `update` (hud-boss.js).
#[derive(Debug, Clone)]
pub struct BossHudState {
    /// JS `this.T` (L294): the HUD's own clock.
    t: f64,
    // ---- JS `S` (L45-49)
    hp: f64,
    max: f64,
    shown: f64,
    chip: f64,
    chip_hold: f64,
    got_hp: bool,
    phase: u32,
    stun_t: f64,
    stun_dur: f64,
    saw_stun: bool,
    shield_t: f64,
    dead: bool,
    intro_t: Option<f64>,
    vis_t: f64,
    open_t: f64,
    // ---- hit flash gate (JS `S.hitT`, L174-175)
    hit_t: f64,
    // ---- callout (JS `this.call` classes + `S.callT` / `dataset.move`)
    call_move: Option<String>,
    call_on: bool,
    call_act: bool,
    call_t: f64,
    // ---- move phase edges (JS `boss:move` per-phase events)
    prev_move: Option<(String, String)>,
    // ---- big moments (JS `setTimeout` removals)
    title_t: Option<f64>,
    banner: Option<(&'static str, &'static str, f64)>,
    ending: Option<(bool, f64)>,
    ended: bool,
    dead_t: f64,
    // ---- notches (JS `is-past` / `is-break`, L176-178)
    notch_past: [bool; 2],
    notch_break_t: [f64; 2],
    // ---- shield class latch (JS `S.L.sh`, L325-326)
    shield: bool,
    enraged: bool,
    cracked: bool,
}

impl BossHudState {
    /// JS `setMode(true, boss)` (L100-117): fresh state seeded from the live
    /// boss HP ratio.
    #[must_use]
    pub fn new(max_hp: f32, hp: f32) -> Self {
        // JS L104 + L115
        let mut s = Self::fresh();
        if max_hp > 0.0 {
            s.max = f64::from(max_hp);
            let r = clamp01(f64::from(hp) / f64::from(max_hp));
            s.hp = r;
            s.shown = r;
            s.chip = r;
        }
        s
    }

    fn fresh() -> Self {
        // JS `_fresh` (L45-49) + the DOM class resets of `setMode` (L107-113)
        Self {
            t: 0.0,
            hp: 1.0,
            max: 1.0,
            shown: 1.0,
            chip: 1.0,
            chip_hold: 0.0,
            got_hp: false,
            phase: 1,
            stun_t: 0.0,
            stun_dur: 0.0,
            saw_stun: false,
            shield_t: 0.0,
            dead: false,
            intro_t: None,
            vis_t: 0.0,
            open_t: 0.0,
            hit_t: -9.0,
            call_move: None,
            call_on: false,
            call_act: false,
            call_t: 0.0,
            prev_move: None,
            title_t: None,
            banner: None,
            ending: None,
            ended: false,
            dead_t: 0.0,
            notch_past: [false; 2],
            notch_break_t: [-1.0; 2],
            shield: false,
            enraged: false,
            cracked: false,
        }
    }

    /// Feed one drained [`BossEvent`]. The HUD consumes Intro / Hp / Stunned /
    /// Phase / Defeat; the move callouts are synthesized from the live
    /// `st.move_*` in [`Self::update`] (the sim raises one `Move` per move,
    /// not per phase). Hit / Bc / CrabBurst / Splat / CrabSpawn drive the
    /// world-space combo numbers / renderer — out of scope for the text档.
    pub fn on_event(&mut self, ev: &BossEvent) {
        match ev {
            BossEvent::Intro => self.intro(),
            BossEvent::Hp { hp } => self.hp_event(*hp, None),
            BossEvent::Stunned { dur } => self.stun(*dur),
            BossEvent::Phase { phase } => self.phase_event(*phase),
            BossEvent::Defeat { .. } => self.defeat(),
            _ => {}
        }
    }

    /// JS `_intro` (L152-160) + `_titleCard` (L444-461).
    fn intro(&mut self) {
        self.intro_t = Some(0.0);
        self.shown = 0.0;
        self.chip = 0.0;
        self.title_t = Some(self.t);
    }

    /// JS `_hp({ hp, max })` (L162-181). `max` rides the frame fallback only
    /// (the Rust `BossEvent::Hp` carries no `max`).
    fn hp_event(&mut self, hp: f32, max: Option<f32>) {
        if let Some(m) = max
            && m > 0.0
        {
            self.max = f64::from(m);
        }
        let t = clamp01(f64::from(hp) / self.max.max(1e-6));
        self.got_hp = true;
        let prev = self.hp;
        self.hp = t;
        if t < prev - 1e-5 {
            // JS L173: a new white chip segment only once the last caught up.
            if self.chip <= self.shown + 0.003 {
                self.chip_hold = 0.6;
            }
            // JS L174-175: the is-hit flash gate (0.07 s).
            if self.t - self.hit_t > 0.07 {
                self.hit_t = self.t;
            }
            // JS L176-178: notch is-past / is-break on the downward crossing.
            for (i, f) in NOTCHES.iter().enumerate() {
                if prev > *f && t <= *f {
                    self.notch_past[i] = true;
                    self.notch_break_t[i] = self.t;
                }
            }
        }
    }

    /// JS `_move({ id, phase })` (L183-195), synthesized from the `st` edges.
    fn move_event(&mut self, id: &str, phase: &str) {
        if self.dead {
            return;
        }
        if !is_call_move(id) {
            // JS L187: non-call moves hide the pill unless the act beat or a
            // stun window is holding it.
            if phase != "act" && self.open_t <= 0.0 {
                self.call_on = false;
                self.call_act = false;
            }
            return;
        }
        if phase == "tele" {
            if self.open_t > 0.0 {
                return;
            }
            self.show_call(id);
        } else if phase == "act" {
            // JS L193: `restartAnim(call, 'is-act')` while the same move.
            if self.call_move.as_deref() == Some(id) {
                self.call_act = true;
            }
        } else if phase == "rec" && self.open_t <= 0.0 {
            self.call_on = false;
            self.call_act = false;
        }
    }

    /// JS `_showCall` (L196-205).
    fn show_call(&mut self, id: &str) {
        self.call_move = Some(id.to_string());
        self.call_on = true;
        self.call_act = false;
        self.call_t = 0.0;
    }

    /// JS `_stun({ dur = 3 })` (L232-241).
    fn stun(&mut self, dur: f32) {
        if self.dead {
            return;
        }
        // JS L235: `Math.max(0.5, +dur || 3)` — a zero / NaN `dur` falls back
        // to 3 before the 0.5 floor.
        let d = if dur > 0.0 { f64::from(dur) } else { 3.0 };
        let d = d.max(0.5);
        self.stun_dur = d;
        self.stun_t = d;
        self.open_t = d;
        self.saw_stun = false;
        self.show_call("open");
    }

    /// JS `_phase({ phase })` (L243-255).
    fn phase_event(&mut self, phase: u32) {
        let ph = phase.clamp(1, 3);
        if ph <= self.phase {
            return;
        }
        self.phase = ph;
        self.shield_t = 1.8;
        if ph >= 3 {
            self.enraged = true;
            self.cracked = true;
        }
        if let Some((big, sub)) = phase_txt(ph) {
            self.banner = Some((big, sub, self.t));
        }
    }

    /// JS `_defeat` (L257-266) + `_ending(true)`.
    fn defeat(&mut self) {
        if self.dead {
            return;
        }
        self.dead = true;
        self.hp = 0.0;
        self.open_t = 0.0;
        self.stun_t = 0.0;
        self.call_on = false;
        self.call_act = false;
        self.dead_t = self.t;
        self.ending(true);
    }

    /// JS `timesUp()` (L269-275) — the clock ran out without a kill.
    fn times_up(&mut self) {
        if self.dead || self.ended {
            return;
        }
        self.call_on = false;
        self.call_act = false;
        self.ending(false);
    }

    /// JS `_ending(win)` (L474-494): clears the banner / title card, arms the
    /// SUNK! / TIME'S UP! word.
    fn ending(&mut self, win: bool) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.banner = None;
        self.title_t = None;
        self.ending = Some((win, self.t));
    }

    /// JS `update(dt)` (L292-334) + the `_writeBar` state (L336-346).
    pub fn update(&mut self, dt: f64, f: &BossHudFrame) {
        self.t += dt;
        // JS L297: fallback while the host's `boss:hp` events are late —
        // re-arm every frame (`gotHp = false` after the synthetic `_hp`).
        if !self.got_hp && f.max_hp > 0.0 {
            let tv = clamp01(f64::from(f.hp) / f64::from(f.max_hp));
            if (tv - self.hp).abs() > 1e-5 {
                self.hp_event(f.hp, Some(f.max_hp));
                self.got_hp = false;
            }
        }
        // JS L298: a missed `boss:defeat` still lands through the live read.
        if f.dead && !self.dead && self.hp <= 0.001 {
            self.defeat();
        }
        // JS `hud.banner('timesup')` → `timesUp()` (L269): the match clock
        // expired with the boss still afloat.
        if f.finished && !self.dead && !self.ended {
            self.times_up();
        }
        // JS per-phase `boss:move` events, synthesized from the live anim
        // state (the sim raises one `Move` per move record).
        let cur = match (f.move_id, f.move_phase) {
            (Some(id), Some(ph)) => Some((id.to_string(), ph.to_string())),
            _ => None,
        };
        if cur != self.prev_move {
            if let Some((id, ph)) = &cur {
                self.move_event(id, ph);
            }
            self.prev_move = cur;
        }
        // JS L300-314: intro fill, else the quick ease + chip drain.
        if self.intro_t.is_some() {
            // JS L302-307: fills once the title card is leaving AND the HUD
            // is up; the gate is `min(introT - 2.25, visT - 0.2) / 1.05`.
            self.intro_t = Some(self.intro_t.unwrap_or(0.0) + dt);
            self.vis_t = if f.hud_visible { self.vis_t + dt } else { 0.0 };
            let k = clamp01((self.intro_t.unwrap_or(0.0) - 2.25).min(self.vis_t - 0.2) / 1.05);
            self.shown = self.hp * ease_out_cubic(k);
            self.chip = self.shown;
            if k >= 1.0 {
                self.intro_t = None;
            }
        } else {
            self.shown += (self.hp - self.shown) * (dt * 16.0).min(1.0);
            if (self.hp - self.shown).abs() < 1e-4 {
                self.shown = self.hp;
            }
            if self.chip_hold > 0.0 {
                self.chip_hold -= dt;
            } else {
                // JS L312: the lagging drain rate `max(0.2, (chip-shown)*2.2)`.
                self.chip = self
                    .shown
                    .max(self.chip - dt * ((self.chip - self.shown) * 2.2).max(0.2));
            }
            if self.chip < self.shown {
                self.chip = self.shown;
            }
        }
        // JS L317-323: the OPEN! stun window + draining meter.
        if self.open_t > 0.0 {
            self.open_t -= dt;
            self.stun_t = self.open_t;
            if f.stunned {
                self.saw_stun = true;
            }
            if self.saw_stun && !f.stunned && self.open_t > 0.2 {
                self.open_t = 0.0;
            }
            if self.open_t <= 0.0 && self.call_move.as_deref() == Some("open") {
                self.call_on = false;
                self.call_act = false;
            }
        }
        // JS L324-326: the phase shield.
        if self.shield_t > 0.0 {
            self.shield_t -= dt;
        }
        self.shield = self.shield_t > 0.0 || f.invuln;
        // JS L328-332: the 3.2 s callout timeout (the OPEN! pill rides the
        // stun window instead).
        if self.call_on {
            self.call_t += dt;
            if self.call_t > 3.2 && self.call_move.as_deref() != Some("open") {
                self.call_on = false;
                self.call_act = false;
            }
        }
    }

    /// JS `_writeBar` (L336-346) + the class snapshot → render descriptor.
    #[must_use]
    pub fn out(&self) -> BossHudOut {
        // JS L264: `setTimeout(1300)` adds `is-gone`.
        let gone = self.dead && self.t - self.dead_t >= 1.3;
        // JS L342: percent with the 0.01 floor special case.
        let pc = if self.shown > 0.0 && self.shown < 0.01 {
            1
        } else {
            (self.shown * 100.0 - 1e-6).ceil() as i64
        };
        BossHudOut {
            bar_visible: !gone,
            pct: pc,
            shown: self.shown as f32,
            chip: self.chip as f32,
            low: self.shown <= 0.2 && self.shown > 0.0,
            pips: self.phase,
            phase: self.phase,
            enraged: self.enraged,
            cracked: self.cracked,
            shield: self.shield,
            open: self.open_t > 0.0,
            open_frac: clamp01(self.open_t / self.stun_dur.max(0.01)) as f32,
            callout: if self.call_on {
                self.call_move.as_deref().map(move_label)
            } else {
                None
            },
            callout_act: self.call_act,
            title_card: self.title_t.is_some_and(|tt| self.t - tt < 3.2),
            banner: self
                .banner
                .filter(|(_, _, bt)| self.t - *bt < 3.4)
                .map(|(big, _, _)| big),
            banner_sub: self
                .banner
                .filter(|(_, _, bt)| self.t - *bt < 3.4)
                .map(|(_, sub, _)| sub),
            ending: self
                .ending
                .filter(|(_, et)| self.t - *et < 5.2)
                .map(|(win, _)| if win { "SUNK!" } else { "TIME'S UP!" }),
            dead: self.dead,
            notch_past: self.notch_past,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame<'a>(hp: f32, max_hp: f32) -> BossHudFrame<'a> {
        BossHudFrame {
            hp,
            max_hp,
            dead: false,
            stunned: false,
            invuln: false,
            move_id: None,
            move_phase: None,
            hud_visible: true,
            finished: false,
        }
    }

    /// JS `setMode` L115: the bar starts at the live HP ratio.
    #[test]
    fn setmode_seeds_from_live_hp() {
        let s = BossHudState::new(120_000.0, 120_000.0);
        let o = s.out();
        assert_eq!(o.pct, 100);
        assert!((o.shown - 1.0).abs() < 1e-9);
        assert_eq!(o.pips, 1);
        assert!(o.bar_visible);
    }

    /// JS `_intro` L152-160 + `update` L300-307: the fill gate is
    /// `clamp(min(introT - 2.25, visT - 0.2) / 1.05)` through `easeOutCubic`.
    #[test]
    fn intro_fill_gates_and_curve() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Intro);
        assert!(s.out().title_card); // JS `_titleCard` ~3.2 s (L458)
        assert!((s.out().shown).abs() < 1e-12); // L156: shown = chip = 0

        // HUD hidden for 3 s: `visT` resets to 0 every frame (L303) → k stays
        // 0 even though `introT - 2.25` went positive.
        let mut f = frame(120_000.0, 120_000.0);
        f.hud_visible = false;
        for _ in 0..60 {
            s.update(0.05, &f);
        }
        assert!(
            (s.out().shown).abs() < 1e-12,
            "visT gate holds the fill at 0"
        );

        // HUD visible: introT = 3.0 now, so the binding gate is `visT - 0.2`.
        f.hud_visible = true;
        // k = 0.5 exactly when visT = 0.725 (min(0.75, 0.525) / 1.05 = 0.5):
        // 145 steps × 0.005 = 0.725.
        for _ in 0..145 {
            s.update(0.005, &f);
        }
        let o = s.out();
        // easeOutCubic(0.5) = 1 - 0.5³ = 0.875 (ui-util.js L51).
        assert!((o.shown - 0.875).abs() < 1e-6, "shown {o:?}");
        assert_eq!(o.pct, 88); // ceil(87.5 - 1e-6)
        assert!(
            (o.chip - o.shown).abs() < 1e-12,
            "chip rides shown in intro"
        );

        // k >= 1 clears introT (L307): visT needs 1.25 → 105 more steps.
        for _ in 0..105 {
            s.update(0.005, &f);
        }
        let o = s.out();
        assert!((o.shown - 1.0).abs() < 1e-9, "intro filled to the target");
        assert!(!s.out().title_card, "title card expired at 3.2 s");
    }

    /// JS `_hp` L173 + `update` L311-313: chipHold 0.6 s, then the drain
    /// `chip - dt * max(0.2, (chip - shown) * 2.2)`; a second hit while the
    /// chip still lags does NOT re-arm the hold.
    #[test]
    fn chip_hold_then_lagging_drain() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Hp { hp: 60_000.0 });
        let f = frame(60_000.0, 120_000.0);
        // 0.6 s of hold (12 × 0.05): chip frozen at the old shown.
        for _ in 0..12 {
            s.update(0.05, &f);
        }
        assert!((s.out().chip - 1.0).abs() < 1e-12, "chip held");
        // shown eased with factor min(1, 0.05*16) = 0.8 per step:
        // 1 - 0.5 * 0.2^12 ≈ 0.5.
        assert!((s.out().shown - 0.5).abs() < 1e-6);

        // Drain starts as soon as chipHold expires (float accumulation lands
        // a hair late — run until the first move, then pin its rate):
        // JS L312 uses the freshly eased `shown` of the same step.
        let mut guard = 0;
        loop {
            let shown_pre = s.out().shown as f64;
            let chip_pre = s.out().chip as f64;
            s.update(0.05, &f);
            guard += 1;
            let o = s.out();
            if (o.chip as f64) < chip_pre - 1e-12 {
                let shown_new = shown_pre + (0.5 - shown_pre) * (0.05f64 * 16.0).min(1.0);
                let want = chip_pre - 0.05 * ((chip_pre - shown_new) * 2.2).max(0.2);
                assert!(
                    (o.chip as f64 - want.max(shown_new)).abs() < 1e-6,
                    "drain rate pinned"
                );
                break;
            }
            assert!(guard < 40, "drain never started");
        }

        // A new damage segment while chip > shown + 0.003: no re-arm (L173).
        s.on_event(&BossEvent::Hp { hp: 30_000.0 });
        s.update(0.05, &f);
        assert!(s.out().chip > s.out().shown, "chip still draining");
        // And it never undershoots the fill (L313).
        let mut last = s.out().chip;
        for _ in 0..200 {
            s.update(0.05, &f);
            let o = s.out();
            assert!(o.chip >= o.shown - 1e-12);
            last = o.chip;
        }
        assert!((last - s.out().shown).abs() < 1e-6, "chip catches shown");
    }

    /// JS `_hp` L176-178: notches latch `is-past` on the downward crossing of
    /// 2/3 and 1/3 (exact equality counts as crossed, `t <= f`).
    #[test]
    fn notch_crossings_latch() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        assert_eq!(s.out().notch_past, [false, false]);
        s.on_event(&BossEvent::Hp { hp: 80_000.0 }); // 0.6666… <= 2/3
        assert_eq!(s.out().notch_past, [true, false]);
        s.on_event(&BossEvent::Hp { hp: 40_000.0 }); // 0.3333… <= 1/3
        assert_eq!(s.out().notch_past, [true, true]);
        // Below both notches: no re-trigger.
        s.on_event(&BossEvent::Hp { hp: 39_000.0 });
        assert_eq!(s.out().notch_past, [true, true]);
    }

    /// JS `_stun` L232-241 + `update` L317-323: OPEN! pill, draining meter,
    /// early end once the host clears `stunned`.
    #[test]
    fn stun_open_window() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Stunned { dur: 3.0 });
        let o = s.out();
        assert!(o.open);
        assert!((o.open_frac - 1.0).abs() < 1e-9);
        assert_eq!(o.callout.as_deref(), Some("OPEN!"));

        // `dur` floor: JS L235 `Math.max(0.5, +dur || 3)`.
        let mut s2 = BossHudState::new(120_000.0, 120_000.0);
        s2.on_event(&BossEvent::Stunned { dur: 0.1 });
        assert!((s2.out().open_frac - 1.0).abs() < 1e-9);
        let f = frame(120_000.0, 120_000.0);
        for _ in 0..11 {
            s2.update(0.05, &f); // 0.55 s > the 0.5 floor
        }
        assert!(!s2.out().open, "0.1 s dur floored to 0.5 s");

        // Early end: saw the stun, host cleared it while openT > 0.2 (L320).
        let mut f2 = frame(120_000.0, 120_000.0);
        f2.stunned = true;
        s.update(0.5, &f2); // openT 3 → 2.5, sawStun
        f2.stunned = false;
        s.update(0.5, &f2); // sawStun && !stunned && openT > 0.2 → openT = 0
        let o = s.out();
        assert!(!o.open);
        assert_eq!(o.callout, None, "the OPEN! pill drops with the window");

        // Natural drain: 3 s at 0.5 steps, never stunned.
        let mut s3 = BossHudState::new(120_000.0, 120_000.0);
        s3.on_event(&BossEvent::Stunned { dur: 3.0 });
        for _ in 0..5 {
            s3.update(0.5, &f);
        }
        assert!(
            (s3.out().open_frac - 1.0 / 6.0).abs() < 1e-9,
            "0.5/3.0 meter"
        );
        s3.update(0.5, &f);
        assert!(!s3.out().open);
    }

    /// JS `_phase` L243-255: pips, 1.8 s shield, the banners, phase-3 enrage.
    #[test]
    fn phase_banner_shield_pips() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Phase { phase: 2 });
        // JS toggles the shield class in `update` (L325), not in `_phase`.
        let f = frame(120_000.0, 120_000.0);
        s.update(0.05, &f);
        let o = s.out();
        assert_eq!(o.banner, Some("PHASE 2!"));
        assert_eq!(o.banner_sub, Some("Crablets incoming — pop them fast!"));
        assert_eq!(o.pips, 2);
        assert!(o.shield);
        // A repeat / downgrade is ignored (L247).
        s.on_event(&BossEvent::Phase { phase: 1 });
        assert_eq!(s.out().pips, 2);
        for _ in 0..40 {
            s.update(0.05, &f); // 2.05 s total > shieldT 1.8
        }
        assert!(!s.out().shield);
        assert_eq!(s.out().banner, Some("PHASE 2!"), "still inside 3.4 s");
        for _ in 0..30 {
            s.update(0.05, &f); // 3.55 s > the 3.4 s removal (L471)
        }
        assert_eq!(s.out().banner, None, "banner drops at 3.4 s");
        s.on_event(&BossEvent::Phase { phase: 3 });
        let o = s.out();
        assert_eq!(o.banner, Some("SHELL CRACKED!"));
        assert!(o.enraged);
        assert!(o.cracked);
        assert_eq!(o.pips, 3);
    }

    /// JS `_defeat` L257-266 + `_ending` L474-494: SUNK! then the bar hides
    /// after 1.3 s, the word after 5.2 s.
    #[test]
    fn defeat_sunk_and_gone() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Hp { hp: 0.0 });
        s.on_event(&BossEvent::Defeat { killer: 0 });
        let o = s.out();
        assert!(o.dead);
        assert_eq!(o.ending, Some("SUNK!"));
        assert!(o.bar_visible, "is-gone only lands at 1.3 s (L264)");
        assert!(!o.open);
        assert_eq!(o.callout, None);
        let f = frame(0.0, 120_000.0);
        for _ in 0..26 {
            s.update(0.05, &f); // 1.3 s
        }
        let o = s.out();
        assert!(!o.bar_visible);
        assert_eq!(o.ending, Some("SUNK!"));
        for _ in 0..80 {
            s.update(0.05, &f); // total 5.3 s > the 5.2 s removal (L491)
        }
        assert_eq!(s.out().ending, None);
    }

    /// JS `timesUp` L269-275: the clock expiring without a kill prints
    /// TIME'S UP! and clears the title card / banner (L478).
    #[test]
    fn times_up_ending_and_guards() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        s.on_event(&BossEvent::Intro);
        s.on_event(&BossEvent::Phase { phase: 2 });
        let mut f = frame(120_000.0, 120_000.0);
        f.finished = true;
        s.update(0.05, &f);
        let o = s.out();
        assert_eq!(o.ending, Some("TIME'S UP!"));
        assert!(!o.title_card, "the ending clears the title card (L478)");
        assert_eq!(o.banner, None, "and the banner");
        // `ended` latch: a second times-up changes nothing (L476).
        s.update(0.05, &f);
        assert_eq!(s.out().ending, Some("TIME'S UP!"));
        // A defeat before the clock still wins (L271).
        let mut s2 = BossHudState::new(120_000.0, 120_000.0);
        s2.on_event(&BossEvent::Defeat { killer: 0 });
        s2.update(0.05, &f);
        assert_eq!(s2.out().ending, Some("SUNK!"));
    }

    /// JS `_move` L183-195: the tele/act/rec callout lifecycle, the OPEN!
    /// suppression, the non-call moves and the 3.2 s timeout (L330).
    #[test]
    fn move_callout_lifecycle() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        let mut f = frame(120_000.0, 120_000.0);
        f.move_id = Some("slam");
        f.move_phase = Some("tele");
        s.update(0.05, &f);
        assert_eq!(s.out().callout.as_deref(), Some("SLAM!"));
        f.move_phase = Some("act");
        s.update(0.05, &f);
        let o = s.out();
        assert_eq!(o.callout.as_deref(), Some("SLAM!"));
        assert!(o.callout_act, "is-act restart (L193)");
        f.move_phase = Some("rec");
        s.update(0.05, &f);
        assert_eq!(s.out().callout, None, "rec hides the pill (L194)");

        // MOVE_LABELS coverage (boss-art.js L19-24).
        for (id, label) in [
            ("barrage", "INCOMING!"),
            ("sweep", "SWEEP!"),
            ("charge", "CHARGE!"),
            ("crablets", "BROOD!"),
            ("frenzy", "FRENZY!"),
        ] {
            let mut s = BossHudState::new(120_000.0, 120_000.0);
            let mut f = frame(120_000.0, 120_000.0);
            f.move_id = Some(id);
            f.move_phase = Some("tele");
            s.update(0.05, &f);
            assert_eq!(s.out().callout.as_deref(), Some(label), "{id}");
        }

        // A non-call move in tele hides an existing pill (L187).
        f.move_id = Some("roar");
        f.move_phase = Some("tele");
        s.update(0.05, &f);
        assert_eq!(s.out().callout, None);

        // The OPEN! window suppresses new tele callouts (L189).
        let mut s2 = BossHudState::new(120_000.0, 120_000.0);
        s2.on_event(&BossEvent::Stunned { dur: 3.0 });
        let mut f2 = frame(120_000.0, 120_000.0);
        f2.move_id = Some("slam");
        f2.move_phase = Some("tele");
        s2.update(0.05, &f2);
        assert_eq!(s2.out().callout.as_deref(), Some("OPEN!"));
    }

    /// JS L328-332: a callout older than 3.2 s drops (unless it's OPEN!).
    #[test]
    fn callout_timeout_3s2() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        let mut f = frame(120_000.0, 120_000.0);
        f.move_id = Some("slam");
        f.move_phase = Some("tele");
        s.update(0.05, &f);
        f.move_id = None;
        f.move_phase = None;
        for _ in 0..40 {
            s.update(0.05, &f); // 2.05 s total — still on screen
            assert!(s.out().callout.is_some(), "pill holds until 3.2 s");
        }
        for _ in 0..24 {
            s.update(0.05, &f); // past 3.2 s (JS L330)
        }
        assert_eq!(s.out().callout, None, "callT > 3.2 drops the pill");
    }

    /// JS `_writeBar` L342-345: the percent ceil with the <0.01 special case
    /// and the low flag.
    #[test]
    fn writebar_pct_and_low() {
        let s = BossHudState::new(120_000.0, 600.0); // ratio 0.005
        let o = s.out();
        assert_eq!(o.pct, 1, "0 < shown < 0.01 prints 1 (L342)");
        assert!(o.low, "shown <= 0.2 && shown > 0 (L344)");
        let s = BossHudState::new(120_000.0, 0.0);
        let o = s.out();
        assert_eq!(o.pct, 0);
        assert!(!o.low, "zero is not low");
        let s = BossHudState::new(120_000.0, 24_000.0); // exactly 0.2
        assert!(s.out().low);
        let s = BossHudState::new(120_000.0, 60_000.0); // 0.5
        assert_eq!(s.out().pct, 50);
    }

    /// JS `update` L297: while no `boss:hp` event has landed the live read
    /// drives the bar (and keeps re-arming until a real event arrives).
    #[test]
    fn hp_fallback_from_live_read() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        let f = frame(60_000.0, 120_000.0);
        s.update(0.05, &f);
        // The synthetic `_hp` sets the target to 0.5; the bar then eases with
        // factor 0.8 → shown 0.6 on the first frame (JS L309 + `_writeBar`).
        assert_eq!(s.out().pct, 60, "fallback _hp landed");
        // Same value: no re-fire (the |t - S.hp| > 1e-5 gate).
        s.update(0.05, &f);
        assert!((s.out().shown - 0.52).abs() < 1e-6);
        // A real event flips gotHp and the fallback stops.
        s.on_event(&BossEvent::Hp { hp: 59_000.0 });
        let f2 = frame(120_000.0, 120_000.0); // a stale live read now ignored
        s.update(0.05, &f2);
        assert!(s.out().pct <= 50);
    }

    /// Headless smoke: a mixed 11-variant event stream through the state
    /// machine never panics.
    #[test]
    fn event_stream_smoke_no_panic() {
        let mut s = BossHudState::new(120_000.0, 120_000.0);
        let events = vec![
            BossEvent::Intro,
            BossEvent::Hp { hp: 90_000.0 },
            BossEvent::Phase { phase: 2 },
            BossEvent::Stunned { dur: 3.0 },
            BossEvent::Hit {
                damage: 100.0,
                weak: true,
                attacker: 0,
                local: true,
                blocked: false,
                predicted: false,
                crab: false,
                pos: None,
            },
            BossEvent::Bc {
                id: 1,
                x: 0.0,
                y: 0.0,
                z: 0.0,
                killed: true,
            },
            BossEvent::CrabBurst {
                id: 1,
                x: 0.0,
                y: 0.0,
                z: 0.0,
                killed: true,
            },
            BossEvent::CrabSpawn {
                id: 2,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            BossEvent::Splat {
                pos: glam::Vec3::ZERO,
                radius: 2.0,
                team: 0,
                seed: 1.0,
            },
            BossEvent::Move(inkwave_sim::net::wire::JVal::Null),
            BossEvent::Defeat { killer: 0 },
        ];
        let mut f = frame(0.0, 120_000.0);
        f.dead = true;
        for ev in &events {
            s.on_event(ev);
        }
        for _ in 0..200 {
            f.move_id = Some("slam");
            f.move_phase = Some("tele");
            s.update(1.0 / 60.0, &f);
            let _ = s.out();
        }
        let o = s.out();
        assert!(o.dead);
        assert_eq!(o.ending, Some("SUNK!"));
    }
}
