//! Gameplay tuning constants, extracted from upstream `src/config.js` into
//! `assets/tuning.json` (schema `inkwave.tuning.v1`) by the Task 3 pipeline.
//!
//! Units: metres / seconds / m·s⁻¹ / m·s⁻²; HP and ink are points. Weapon
//! spread angles are degrees; every other angle is radians. Y-up, Alpha at -Z.
//! Fields mirror the JS object keys 1:1 (camelCase in JSON) so an upstream sync
//! is a re-run of the extractor plus a compile; see `assets/tuning.schema.md`.

use serde::Deserialize;

/// Top-level tuning document.
#[derive(Debug, Clone, Deserialize)]
pub struct Tuning {
    pub schema: String,
    pub source: Source,
    pub player: PlayerTuning,
    pub spritzer: Spritzer,
    /// `match` is a Rust keyword, hence the rename.
    #[serde(rename = "match")]
    pub match_config: MatchConfig,
    #[serde(rename = "teamPalettes")]
    pub team_palettes: Vec<TeamPalette>,
    pub difficulty: DifficultySet,
}

/// Provenance of an extracted artifact.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// Upstream git commit the data was extracted from.
    pub commit: String,
    /// Source JS file (relative to repo root).
    pub path: String,
}

/// All of upstream PLAYER: physics + feel constants consumed by the Task 5
/// character controller. Field order/grouping follows `src/config.js`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTuning {
    // survival
    pub hp: f32,
    pub special_charge_rate: f32,
    pub radius: f32,
    pub height: f32,
    pub squid_height: f32,
    // base speeds
    pub run_speed: f32,
    pub squid_dry_speed: f32,
    pub swim_speed: f32,
    pub enemy_ink_speed: f32,
    pub climb_speed: f32,
    pub accel_ground: f32,
    pub accel_air: f32,
    pub accel_swim: f32,
    // jump / gravity
    pub jump_vel: f32,
    pub swim_jump_vel: f32,
    pub gravity: f32,
    pub max_fall: f32,
    // ink tank
    pub ink_max: f32,
    pub ink_refill_swim: f32,
    pub ink_refill_kid: f32,
    pub ink_refill_delay: f32,
    // enemy ink damage + hp regen
    pub enemy_ink_dps: f32,
    pub enemy_ink_damage_cap: f32,
    pub regen_delay: f32,
    pub regen_rate: f32,
    pub regen_rate_swim: f32,
    pub respawn_time: f32,
    pub special_keep_on_splat: f32,
    pub spawn_invuln: f32,
    pub fall_death_y: f32,
    pub water_y: f32,
    // handling curve (JS actor.js _horizontal / _integrate)
    pub run_accel: f32,
    pub run_accel_in: f32,
    pub run_in_knee: f32,
    pub run_out_knee: f32,
    pub run_out_min: f32,
    pub run_decel: f32,
    pub run_decel_min: f32,
    pub run_decel_knee: f32,
    pub reverse_decel: f32,
    pub reverse_angle: f32,
    pub turn_rate: f32,
    pub turn_rate_slow: f32,
    pub air_accel: f32,
    pub air_decel: f32,
    pub air_min_speed: f32,
    pub squid_accel: f32,
    pub squid_decel: f32,
    pub squid_turn: f32,
    pub swim_accel: f32,
    pub swim_accel_in: f32,
    pub swim_decel: f32,
    pub swim_turn: f32,
    pub swim_out_knee: f32,
    pub squid_air_accel: f32,
    pub squid_air_decel: f32,
    pub enemy_ink_decel: f32,
    pub enemy_ink_accel: f32,
    // jumping feel
    pub jump_buffer: f32,
    pub coyote_time: f32,
    pub fall_gravity_mul: f32,
    pub apex_gravity_mul: f32,
    pub apex_band: f32,
    pub hard_land_speed: f32,
    pub hard_land_slow: f32,
    pub hard_land_time: f32,
    // character controller geometry
    pub foot_radius: f32,
    pub step_up: f32,
    pub step_down: f32,
    pub squid_step_up: f32,
    pub squid_body_lift: f32,
    pub ledge_assist: f32,
    // facing springs
    pub face_omega: f32,
    pub face_max_rate: f32,
    pub face_max_acc: f32,
    pub squid_face_omega: f32,
    pub squid_face_max_rate: f32,
    pub swim_face_max_rate: f32,
    pub squid_face_max_acc: f32,
    pub aim_face_omega: f32,
    pub aim_face_max_rate: f32,
    pub aim_face_max_acc: f32,
    // wall climb
    pub climb_accel: f32,
    pub climb_side_speed: f32,
    pub climb_attach_dot: f32,
    pub climb_detach_dot: f32,
    pub ledge_pop_clear: f32,
    pub ledge_pop_carry: f32,
    // shooting interlock
    pub emerge_delay: f32,
    pub fire_buffer: f32,
}

/// 0..1 display bars for the loadout screen; not used by the simulation.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct WeaponStats {
    pub range: f32,
    pub damage: f32,
    pub rate: f32,
    pub mobility: f32,
    pub paint: f32,
}

/// Upstream `WEAPONS.shooter` ("Spritzer") — the M1 weapon.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spritzer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub fire_interval: f32,
    pub damage: f32,
    pub ink_per_shot: f32,
    pub proj_speed: f32,
    pub straight_time: f32,
    pub range: f32,
    pub spread_ground: f32,
    pub spread_air: f32,
    pub impact_radius: f32,
    pub trail_radius: f32,
    pub trail_every: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// Upstream `MATCH` (Turf War rules).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchConfig {
    pub durations: [u32; 2],
    pub default_duration: u32,
    pub max_duration: u32,
    pub final_countdown: u32,
    pub team_size: u32,
    pub points_per_m2: f32,
    pub death_mark_life: f32,
    pub death_mark_fade: f32,
}

/// One selectable pair of team ink colours.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamPalette {
    pub id: String,
    /// Alpha ink colour (sRGB hex).
    pub a: String,
    /// Bravo ink colour (sRGB hex).
    pub b: String,
    pub names: [String; 2],
}

/// Bot tuning per difficulty level.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Difficulty {
    pub id: String,
    pub name: String,
    pub reaction: f32,
    pub aim_error: f32,
    pub fire_discipline: f32,
    pub awareness: f32,
    pub aim_omega: f32,
    pub aim_turn: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DifficultySet {
    pub easy: Difficulty,
}

/// Parse a tuning document from JSON.
pub fn tuning_from_str(s: &str) -> Result<Tuning, serde_json::Error> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> Tuning {
        crate::embedded_tuning()
    }

    // TR-3.2: the gameplay-critical Spritzer numbers are locked to upstream.
    #[test]
    fn spritzer_key_values() {
        let w = &tuning().spritzer;
        assert_eq!(w.id, "shooter");
        assert_eq!(w.name, "Spritzer");
        assert_eq!(w.fire_interval, 0.1);
        assert_eq!(w.damage, 36.0);
        assert_eq!(w.ink_per_shot, 0.95);
        assert_eq!(w.proj_speed, 34.0);
        assert_eq!(w.straight_time, 0.13);
        assert_eq!(w.range, 12.5);
        assert_eq!(w.spread_ground, 5.5);
        assert_eq!(w.spread_air, 11.0);
        assert_eq!(w.impact_radius, 0.85);
        assert_eq!(w.trail_radius, 0.44);
        assert_eq!(w.trail_every, 1.05);
        assert_eq!(w.move_speed_firing, 4.6);
    }

    // TR-3.2: the gameplay-critical PLAYER numbers are locked to upstream.
    #[test]
    fn player_key_values() {
        let p = &tuning().player;
        assert_eq!(p.hp, 100.0);
        assert_eq!(p.run_speed, 6.0);
        assert_eq!(p.swim_speed, 11.8);
        assert_eq!(p.respawn_time, 5.5);
        assert_eq!(p.squid_dry_speed, 2.9);
        assert_eq!(p.jump_vel, 8.4);
        assert_eq!(p.gravity, 25.0);
        assert_eq!(p.ink_max, 100.0);
        assert_eq!(p.ink_refill_swim, 42.0);
        assert_eq!(p.ink_refill_kid, 9.0);
        assert_eq!(p.ink_refill_delay, 0.9);
        assert_eq!(p.enemy_ink_dps, 20.0);
        assert_eq!(p.enemy_ink_damage_cap, 40.0);
        assert_eq!(p.spawn_invuln, 1.6);
        assert_eq!(p.fall_death_y, -1.45);
        assert_eq!(p.water_y, -1.6);
        assert_eq!(p.foot_radius, 0.24);
        assert_eq!(p.step_up, 0.35);
        assert_eq!(p.step_down, 0.45);
        assert_eq!(p.radius, 0.38);
        assert_eq!(p.height, 1.45);
        assert_eq!(p.squid_face_max_acc, 260.0);
    }

    #[test]
    fn match_and_difficulty_and_palettes() {
        let t = tuning();
        let m = t.match_config;
        assert_eq!(m.durations, [90, 180]);
        assert_eq!(m.default_duration, 180);
        assert_eq!(m.max_duration, 180);
        assert_eq!(m.team_size, 4);
        assert_eq!(m.points_per_m2, 1.0);
        assert_eq!(m.final_countdown, 10);
        let e = &t.difficulty.easy;
        assert_eq!(e.id, "easy");
        assert_eq!(e.reaction, 0.55);
        assert_eq!(e.aim_error, 0.11);
        assert_eq!(e.awareness, 16.0);
        assert_eq!(e.aim_omega, 9.0);
        assert_eq!(e.aim_turn, 7.0);
        assert_eq!(t.team_palettes.len(), 5);
        assert_eq!(t.team_palettes[0].id, "tangerine-cobalt");
        assert_eq!(t.team_palettes[0].a, "#ff8a14");
        assert_eq!(t.team_palettes[0].b, "#2f5bff");
        assert_eq!(t.team_palettes[0].names, ["Tangerine", "Cobalt"]);
    }

    #[test]
    fn schema_and_provenance() {
        let t = tuning();
        assert_eq!(t.schema, "inkwave.tuning.v1");
        assert_eq!(t.source.path, "src/config.js");
        assert_eq!(t.source.commit.len(), 40, "commit must be a full sha");
    }
}
