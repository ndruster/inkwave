//! Boss Battle rules (JS `src/boss/bossMode.js`, 55 lines): the whole room is
//! one squad (team 0) against HULLBREAKER. Win: its HP hits 0. Lose: the clock
//! runs out (turf is shown, but secondary). The boss's HP scales with the
//! squad (humans 1.0, bots 0.6) and difficulty.
//!
//! Ownership (Task 44 §2, key decision; revised by D-T45-9): [`crate::boss::Boss`]
//! owns its nav grid / collision / tuning outright, so this [`BossMode`]
//! wrapper is `'static` — the driver layer (autopilot / nettest / the game
//! loop) can hold it across frames (a Bevy `Resource`) and pass it into
//! [`crate::match_::Match::step`] frame by frame; the match only keeps the
//! [`crate::tuning::BossModeRules`] data slice.
//!
//! Deviations from JS (recorded for review):
//!   - **`onDefeat` edge-detection** (L37-42): JS calls `BossMode.onDefeat`
//!     from the `boss.js` defeat path with the killer actor. The Rust
//!     [`Boss::defeat`] has no match handle, so [`BossMode::update`]
//!     edge-detects the `dead` transition instead — same frame, same effect:
//!     `won` / `kill_t` latch and the killer (found via [`Boss::killer`]
//!     against [`BossSquad::ids`]) gets `Actor::boss_kill = 1`.
//!   - **`emit('boss:spawn')` / `G.boss`** (L24/L27) and
//!     **`dispose`** (L54): the presentation globals stay upstream; the Rust
//!     driver owns the struct outright.
//!   - **`result()`** (L44-52): the JS one-shot object splits — `coverage` /
//!     `winner` land on [`crate::match_::MatchResult`], the nested `boss`
//!     payload is [`BossOutcome`].

use serde::{Deserialize, Serialize};

use crate::actor::SimWorld;
use crate::boss::Boss;
use crate::boss::BossConfig;
use crate::boss::BossSquad;
use crate::boss::nav::BossNav;
use crate::tuning;

/// JS `Math.round` (tie toward +∞), same convention as `boss/mod.rs`
/// `js_round`.
#[inline]
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// JS `squadUnits(actors)` (bossMode.js L15): `Σ (a.isBot && !a.isLocal ?
/// 0.6 : 1)`. The sim [`crate::actor::Actor`] carries no bot / local flag,
/// so the driver counts the two kinds and this helper weighs them from the
/// extracted `squadUnits` table (`{human: 1, bot: 0.6}`).
#[must_use]
pub fn squad_units(humans: usize, bots: usize, su: &tuning::BossSquadUnits) -> f32 {
    humans as f32 * su.human + bots as f32 * su.bot
}

/// The Boss Battle settlement (JS `BossMode.result().boss`, bossMode.js
/// L48-50). Folded into [`crate::match_::MatchResult::boss`] the way
/// [`crate::match_::ZoneOutcome`] folds the zones branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BossOutcome {
    /// JS `BOSS.name` (boss.js L16) — `HULLBREAKER`.
    pub name: String,
    /// JS `win = b.dead` (L47).
    pub win: bool,
    /// JS `time` (L50): `round1(killT >= 0 && win ? killT : duration - time)`
    /// — the kill clock on a win, the remaining clock on a loss.
    pub time: f32,
    /// JS `hp = Math.round(b.hp)` (L50).
    pub hp: f32,
    pub max_hp: f32,
    pub phase: u32,
}

/// JS `class BossMode` (bossMode.js L17-55): owns the boss entity, the finish
/// conditions and the result. The driver constructs it (with the boss HP
/// scaled per L22) and hands it to [`crate::match_::Match::step`].
/// D-T45-9: no lifetime — [`Boss`] owns its nav / tuning / collision, so this
/// wrapper is `'static` and the app layer can hold it across frames.
pub struct BossMode {
    pub boss: Boss,
    /// JS `this.won` (L25).
    won: bool,
    /// JS `this.killT` (L26): remaining clock at the kill, `-1` while alive.
    kill_t: f32,
}

impl BossMode {
    /// JS constructor (bossMode.js L18-28), minus the `match` back-reference
    /// (the driver passes `remaining` into [`Self::update`] instead).
    /// `units` is the squad weight (JS L21 sums team 0 through L15);
    /// `sim` is JS `!match.follower` (L23) — the caller resolves it.
    /// `difficulty` is the raw `opts.difficulty` (JS L22/L23: the HP table
    /// lookup falls back to ×1 for unknown values, the entity defaults an
    /// empty string to `'normal'`).
    #[must_use]
    pub fn new(
        world: &SimWorld,
        nav: BossNav,
        tb: tuning::Boss,
        units: f32,
        difficulty: &str,
        sim: bool,
        seed: u64,
    ) -> Self {
        // JS L22: `maxHp = Math.round(BOSS.hpPerUnit * units *
        // (BOSS.hpDiff[o.difficulty] ?? 1))`.
        let hp_diff = match difficulty {
            "easy" => f64::from(tb.boss.hp_diff.easy),
            "normal" => f64::from(tb.boss.hp_diff.normal),
            "hard" => f64::from(tb.boss.hp_diff.hard),
            _ => 1.0,
        };
        let max_hp = js_round(f64::from(tb.boss.hp_per_unit) * f64::from(units) * hp_diff) as f32;
        // JS L23: `difficulty: o.difficulty || 'normal'`.
        let entity_diff = if difficulty.is_empty() {
            "normal"
        } else {
            difficulty
        };
        Self {
            boss: Boss::new(
                world,
                nav,
                tb,
                BossConfig {
                    max_hp,
                    difficulty: entity_diff,
                    sim,
                    seed,
                },
            ),
            won: false,
            kill_t: -1.0,
        }
    }

    /// JS `update(dt)` (L30-35) + `onDefeat(by)` (L37-42, edge-detected —
    /// see the module docs). `remaining` is the match clock
    /// (`duration - time`, JS L40). Returns JS L34's `b.dead` — the caller
    /// performs the `playing → finish` transition (the match owns the phase
    /// machine).
    pub fn update(&mut self, dt: f32, squad: &mut BossSquad<'_>, remaining: f32) -> bool {
        self.boss.update(dt, squad);
        if self.boss.dead && !self.won {
            // JS L39-40: the defeat lands once — latch the win and the
            // remaining clock.
            self.won = true;
            self.kill_t = remaining;
            // JS L41: `by.stats.bossKill = 1` — resolve the killer nid
            // against the squad ids (`nid ?? slot`, JS L364).
            let killer = self.boss.killer;
            if killer >= 0 {
                for (i, id) in squad.ids.iter().enumerate() {
                    if *id == killer {
                        squad.actors[i].boss_kill = 1;
                        break;
                    }
                }
            }
        }
        self.boss.dead
    }

    /// JS `result()` (L44-52): the boss fight's settlement. `remaining` is
    /// the match clock at judge time (JS `m.duration - m.time`); `coverage`
    /// rides [`crate::match_::MatchResult`] instead of this payload.
    #[must_use]
    pub fn result(&self, remaining: f32) -> BossOutcome {
        let b = &self.boss;
        // JS L47: `win = b.dead` (not `this.won` — the live entity decides).
        let win = b.dead;
        // JS L50: `Math.round((win && killT >= 0 ? killT : duration - time)
        // * 10) / 10`.
        let t = if win && self.kill_t >= 0.0 {
            self.kill_t
        } else {
            remaining
        };
        BossOutcome {
            name: b.tb.boss.name.clone(),
            win,
            time: (js_round(f64::from(t) * 10.0) / 10.0) as f32,
            hp: js_round(f64::from(b.hp)) as f32,
            max_hp: b.max_hp,
            phase: b.phase,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;

    /// TR-44.1 (hp line): JS L15/L22 — the 8-slot squad weighs
    /// `1 + 7 × 0.6 = 5.2` units, `maxHp = round(18000 × units × hpDiff)`.
    #[test]
    fn boss_mode_hp_scaling() {
        let layout = crate::embedded_tidewater();
        let collision = CollisionWorld::from_layout(&layout);
        let tb = crate::embedded_tuning();
        let world = SimWorld::new(&collision, &layout);
        let nav = BossNav::new(&world, &tb.boss.nav);
        let units = squad_units(1, 7, &tb.boss.squad_units);
        assert!((units - 5.2).abs() < 1e-6, "units: {units}");
        for (diff, want) in [
            ("easy", 70_200.0),
            ("normal", 93_600.0),
            ("hard", 121_680.0),
        ] {
            let bm = BossMode::new(&world, nav.clone(), tb.boss.clone(), units, diff, true, 1);
            assert_eq!(bm.boss.max_hp, want, "{diff} maxHp");
        }
        // Unknown difficulty: JS L22 `?? 1` fallback.
        let bm = BossMode::new(&world, nav.clone(), tb.boss.clone(), units, "", true, 1);
        assert_eq!(bm.boss.max_hp, 93_600.0);
    }
}
