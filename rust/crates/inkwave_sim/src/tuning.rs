//! Gameplay tuning constants, extracted from upstream `src/config.js` into
//! `assets/tuning.json` (schema `inkwave.tuning.v2`) by the Task 3 pipeline,
//! extended to the full M3 content set by Task 28 (12 weapons / 15 subs /
//! 19 specials / zones / three difficulties).
//!
//! Units: metres / seconds / m·s⁻¹ / m·s⁻²; HP and ink are points. Weapon
//! spread / arc / fan / leap-angle fields are degrees; every other angle is
//! radians. Y-up, Alpha at -Z.
//! Fields mirror the JS object keys 1:1 (camelCase in JSON) so an upstream sync
//! is a re-run of the extractor plus a compile; see `assets/tuning.schema.md`.

use std::collections::HashMap;

use serde::Deserialize;

/// Top-level tuning document.
#[derive(Debug, Clone, Deserialize)]
pub struct Tuning {
    pub schema: String,
    pub source: Source,
    pub player: PlayerTuning,
    /// M1 alias of `weapons.shooter`; consumers migrate in Task 29.
    pub spritzer: Spritzer,
    /// `WEAPON_ORDER` (config.js L303) — the 12 roster ids.
    #[serde(rename = "weaponOrder")]
    pub weapon_order: Vec<String>,
    pub weapons: WeaponSet,
    /// `weaponRange()` (config.js L307-315) evaluated per weapon id at
    /// extraction time: effective reach (m) for aim assist / bots / reticle.
    #[serde(rename = "weaponRange")]
    pub weapon_range: HashMap<String, f32>,
    /// `SUB_ORDER` (config.js L407) — the 15 sub ids.
    #[serde(rename = "subOrder")]
    pub sub_order: Vec<String>,
    pub subs: SubSet,
    /// `SPECIAL_ORDER` (config.js L469) — the 19 special ids.
    #[serde(rename = "specialOrder")]
    pub special_order: Vec<String>,
    pub specials: SpecialSet,
    pub zones: ZonesTuning,
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

// ---------------------------------------------------------------------------
// Task 28 (M3): the full roster. One typed struct per weapon / sub / special,
// fields 1:1 with the config.js object literals (line anchors in the tests).
// The retired defs (dualies / slosher / splatling, config.js L266-299) are
// never extracted (TR-28.2).
// ---------------------------------------------------------------------------

/// All 12 roster weapons (`WEAPONS` filtered by `WEAPON_ORDER`, config.js
/// L94-315), keyed by id.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeaponSet {
    pub shooter: Spritzer,
    pub twins: Twins,
    pub brolly: Brolly,
    pub blaster: Blaster,
    pub spinner: Spinner,
    pub charger: Charger,
    pub bow: Bow,
    pub roller: Roller,
    pub brush: Brush,
    pub blade: Blade,
    pub mitts: Mitts,
    pub bucket: Bucket,
}

/// `WEAPONS.twins` — Twinfire Pistols (config.js L158-168).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Twins {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
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
    pub roll_dist: f32,
    pub roll_time: f32,
    pub roll_ink: f32,
    pub roll_charges: u32,
    pub roll_lockout: f32,
    pub roll_reset: f32,
    pub turret_interval: f32,
    pub turret_spread: f32,
    pub turret_proj_speed: f32,
    pub turret_straight: f32,
    pub turret_range: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.brolly` — Canopy Brolly (config.js L180-198).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Brolly {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub range: f32,
    pub fire_interval: f32,
    pub ink_per_shot: f32,
    pub pellets: u32,
    pub pellet_damage: f32,
    pub pellet_damage_far: f32,
    pub falloff_start: f32,
    pub falloff_end: f32,
    pub proj_speed: f32,
    pub straight_time: f32,
    pub pellet_grav: f32,
    pub pellet_drag: f32,
    pub pellet_life: f32,
    pub pellet_size: f32,
    pub pellet_paint: f32,
    pub spread_deg: f32,
    pub spread_air: f32,
    pub move_speed_firing: f32,
    pub move_speed_shield: f32,
    pub canopy_hp: u32,
    pub open_delay: f32,
    pub open_time: f32,
    pub close_time: f32,
    pub launch_hold: f32,
    pub regrow_time: f32,
    pub canopy_regen: f32,
    pub canopy_regen_delay: f32,
    pub launch_speed: f32,
    pub launch_life: f32,
    pub launch_paint: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.blaster` — Popper Blaster (config.js L126-135).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blaster {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub fire_interval: f32,
    pub direct_damage: f32,
    pub splash_damage_max: f32,
    pub splash_damage_min: f32,
    pub splash_radius: f32,
    pub ink_per_shot: f32,
    pub proj_speed: f32,
    pub range: f32,
    pub impact_radius: f32,
    pub burst_radius: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.spinner` — Squall Spinner (config.js L148-157).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spinner {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub charge_time: f32,
    pub ink_full: f32,
    pub burst_min: f32,
    pub burst_max: f32,
    pub fire_interval: f32,
    pub damage: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub straight_min: f32,
    pub straight_max: f32,
    pub range_min: f32,
    pub range_max: f32,
    pub spread_ground: f32,
    pub spread_air: f32,
    pub impact_radius: f32,
    pub trail_radius: f32,
    pub trail_every: f32,
    pub move_speed_charging: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.charger` — Glint Charger (config.js L117-125).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Charger {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub charge_time: f32,
    pub range_min: f32,
    pub range_max: f32,
    pub damage_min: f32,
    pub damage_max: f32,
    pub ink_full: f32,
    pub line_splat_every: f32,
    pub line_radius: f32,
    pub impact_radius: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.bow` — Tideline Bow (config.js L199-219). Paired `[ring, full]`
/// tiers stay as two-element arrays.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub charge_time: f32,
    pub ring1: f32,
    pub ink_full: f32,
    pub ink_min: f32,
    pub range: f32,
    pub flight_tap: [f32; 2],
    pub flight_ring: [f32; 2],
    pub flight_full: f32,
    pub speed_tap: f32,
    pub speed_ring: [f32; 2],
    pub speed_full: f32,
    pub straight: f32,
    pub grav: f32,
    pub dive_grav: f32,
    pub dive_drag: f32,
    pub damage_tap: [f32; 2],
    pub damage_ring: [f32; 2],
    pub damage_full: f32,
    pub side_mul: f32,
    pub side_full: f32,
    pub fan_tap: f32,
    pub fan_ring: f32,
    pub fan_full: f32,
    pub fuse_ring: f32,
    pub fuse_full: f32,
    pub burst_radius: [f32; 2],
    pub burst_inner: f32,
    pub burst_damage: [f32; 2],
    pub burst_edge: [f32; 2],
    pub paint_tap: f32,
    pub paint_stick: f32,
    pub burst_paint: [f32; 2],
    pub trail_every: f32,
    pub trail_radius: f32,
    pub cooldown: f32,
    pub move_speed_drawing: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.roller` — Swell Roller (config.js L106-116).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Roller {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub roll_speed: f32,
    pub roll_width: f32,
    pub roll_ink_per_meter: f32,
    pub roll_damage: f32,
    pub flick_interval: f32,
    pub flick_windup: f32,
    pub flick_ink: f32,
    pub flick_drops: u32,
    pub flick_damage_near: f32,
    pub flick_damage_far: f32,
    pub flick_speed: f32,
    pub flick_spread_deg: f32,
    pub impact_radius: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.brush` — Swish Brush (config.js L169-178).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Brush {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub brush_speed: f32,
    pub brush_width: f32,
    pub brush_ink_per_meter: f32,
    pub brush_damage: f32,
    pub brush_hit_cd: f32,
    pub swipe_interval: f32,
    pub swipe_ink: f32,
    pub swipe_drops: u32,
    pub swipe_speed: f32,
    pub swipe_spread_deg: f32,
    pub swipe_damage_near: f32,
    pub swipe_damage_far: f32,
    pub impact_radius: f32,
    pub move_speed_firing: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.blade` — Brine Cutlass (config.js L220-238).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blade {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub range: f32,
    pub proj_speed: f32,
    pub tap_interval: f32,
    pub tap_ink: f32,
    pub tap_drops: u32,
    pub tap_spread_deg: f32,
    pub tap_speed: f32,
    pub tap_straight: f32,
    pub tap_damage_near: f32,
    pub tap_damage_far: f32,
    pub tap_drop_radius: f32,
    pub tap_melee: f32,
    pub charge_delay: f32,
    pub charge_time: f32,
    pub charge_store: f32,
    pub heavy_ink: f32,
    pub heavy_interval: f32,
    pub heavy_melee: f32,
    pub lunge_dist: f32,
    pub lunge_time: f32,
    pub stroke_radius: f32,
    pub wave_damage: f32,
    pub wave_damage_far: f32,
    pub wave_speed: f32,
    pub wave_range: f32,
    pub wave_width: f32,
    pub wave_height: f32,
    pub wave_paint_every: f32,
    pub wave_paint_radius: f32,
    pub wave_end_radius: f32,
    pub reach: f32,
    pub melee_arc_deg: f32,
    pub heavy_arc_deg: f32,
    pub move_speed_firing: f32,
    pub move_speed_charging: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.mitts` — Sponge Mitts (config.js L239-264).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mitts {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub range: f32,
    pub punch_interval: f32,
    pub ink_per_punch: f32,
    pub punch_spread: f32,
    pub fist_speed: f32,
    pub fist_range: f32,
    pub fist_size: f32,
    pub guard_armor: f32,
    pub guard_arc: f32,
    pub guard_after: f32,
    pub bot_approach: f32,
    pub punch_damage: f32,
    pub splash_radius: f32,
    pub splash_max: f32,
    pub splash_min: f32,
    pub fist_paint: f32,
    pub move_speed_firing: f32,
    pub leap_charge_time: f32,
    pub leap_ink_min: f32,
    pub leap_ink_max: f32,
    pub move_speed_charging: f32,
    pub leap_armor: f32,
    pub leap_armor_grace: f32,
    pub leap_speed_min: f32,
    pub leap_speed_max: f32,
    pub leap_angle: f32,
    pub leap_angle_min: f32,
    pub leap_angle_max: f32,
    pub leap_pitch_k: f32,
    pub leap_vy_max: f32,
    pub land_radius: f32,
    pub land_damage_max: f32,
    pub land_damage_min: f32,
    pub land_paint: f32,
    pub land_invuln: f32,
    pub glove_radius: f32,
    pub glove_damage: f32,
    pub cling_drain: f32,
    pub special: String,
    pub special_cost: u32,
    pub sub: String,
}

/// `WEAPONS.bucket` — Bilge Bucket (config.js L137-147).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub class: String,
    pub anim: String,
    pub blurb: String,
    pub stats: WeaponStats,
    pub fire_interval: f32,
    pub windup: f32,
    pub ink_per_shot: f32,
    pub throw_speed: f32,
    pub lob: f32,
    pub blobs: u32,
    pub gravity: f32,
    pub drag: f32,
    pub reach: f32,
    pub damage: f32,
    pub impact_radius: f32,
    pub below_falloff: f32,
    pub below_falloff_max: f32,
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

/// All 15 sub weapons (`SUBS` filtered by `SUB_ORDER`, config.js L318-407).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubSet {
    pub bomb: Bomb,
    pub sticky: Sticky,
    pub burst: Burst,
    pub seeker: Seeker,
    pub scan: Scan,
    pub curtain: Curtain,
    pub sprinkler: Sprinkler,
    pub mine: Mine,
    pub beacon: Beacon,
    pub mist: Mist,
    pub shaker: Shaker,
    pub waddle: Waddle,
    pub torpedo: Torpedo,
    pub tracer: Tracer,
    pub boomerang: Boomerang,
}

/// `SUBS.bomb` — Splat Bomb (config.js L319-322).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bomb {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub fuse: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
}

/// `SUBS.sticky` — Cling Charge (config.js L323-326).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sticky {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub fuse: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
}

/// `SUBS.burst` — Pop Pellet (config.js L327-330).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Burst {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub radius: f32,
    pub direct_damage: f32,
    pub splash_damage: f32,
    pub paint_radius: f32,
}

/// `SUBS.seeker` — Skitter Bomb (config.js L331-339).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seeker {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub speed: f32,
    pub seek_range: f32,
    pub life: f32,
    pub trail_radius: f32,
    pub trigger_dist: f32,
    pub turn_rate: f32,
    pub commit_dist: f32,
    pub creep: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
}

/// `SUBS.scan` — Echo Orb (config.js L340-343).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub fuse: f32,
    pub radius: f32,
    pub cloud_time: f32,
    pub track_time: f32,
}

/// `SUBS.curtain` — Drip Curtain (config.js L344-347).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Curtain {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub width: f32,
    pub height: f32,
    pub hp: u32,
    pub decay: f32,
    pub shot_mul: f32,
}

/// `SUBS.sprinkler` — Twirl Sprinkler (config.js L348-351).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sprinkler {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub hp: u32,
    pub pulse: f32,
    pub drops: u32,
    pub spray_radius: f32,
    pub spray_fade: f32,
    pub drop_damage: f32,
}

/// `SUBS.mine` — Lurk Mine (config.js L352-355).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mine {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub placed: bool,
    pub max: u32,
    pub trigger_radius: f32,
    pub arm_time: f32,
    pub delay: f32,
    pub radius: f32,
    pub damage: f32,
    pub track_time: f32,
    pub paint_radius: f32,
}

/// `SUBS.beacon` — Hop Beacon (config.js L356-359).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Beacon {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub placed: bool,
    pub max: u32,
    pub uses: u32,
    pub hp: u32,
}

/// `SUBS.mist` — Murk Bomb (config.js L360-363).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mist {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub fuse: f32,
    pub radius: f32,
    pub mist_time: f32,
    pub slow: f32,
    pub ink_drain: f32,
}

/// `SUBS.shaker` — Shaker Bomb (config.js L365-374).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shaker {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub charge_time: f32,
    pub move_boost: f32,
    pub jump_boost: f32,
    pub shake_boost: f32,
    pub max_rate: f32,
    pub fuse: f32,
    pub gap: f32,
    pub hop_speed: f32,
    pub hop_up: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
    pub trail_every: f32,
    pub trail_radius: f32,
}

/// `SUBS.waddle` — Waddle Bomb (config.js L375-381).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waddle {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub sense_radius: f32,
    pub sense_up: f32,
    pub fuse: f32,
    pub speed: f32,
    pub turn_rate: f32,
    pub life: f32,
    pub max_travel: f32,
    pub trigger_dist: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
    pub hp: u32,
}

/// `SUBS.torpedo` — Tide Torpedo (config.js L382-391).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Torpedo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub feet_paint: f32,
    pub arm_time: f32,
    pub lock_range: f32,
    pub fly_life: f32,
    pub unfold_time: f32,
    pub launch_speed0: f32,
    pub launch_speed: f32,
    pub launch_accel: f32,
    pub turn_rate: f32,
    pub launch_life: f32,
    pub hp: u32,
    pub radius: f32,
    pub core_radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
    pub fallback_paint: f32,
    pub drops: u32,
    pub drop_damage: f32,
    pub drop_hits: u32,
    pub drop_paint: f32,
}

/// `SUBS.tracer` — Tracer Bolt (config.js L392-396).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub speed: f32,
    pub range: f32,
    pub pitch_down: f32,
    pub floor_exit: f32,
    pub max_bounces: u32,
    pub size: f32,
    pub puddle_radius: f32,
    pub direct_damage: f32,
    pub direct_mark: f32,
    pub trail_life: f32,
    pub trail_radius: f32,
    pub trail_damage: f32,
    pub trail_mark: f32,
}

/// `SUBS.boomerang` — Whirl Boomerang (config.js L397-405).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Boomerang {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub blurb: String,
    pub ink_cost: f32,
    pub throw_speed: f32,
    pub range: f32,
    pub out_time: f32,
    pub hover: f32,
    pub hover_radius: f32,
    pub tick_rate: u32,
    pub tick_damage: f32,
    pub hover_paint: f32,
    pub return_speed: f32,
    pub orbit: f32,
    pub orbit_radius: f32,
    pub orbit_spin: f32,
    pub orbit_damage: f32,
    pub orbit_hit_cd: f32,
    pub radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
    pub paint_radius: f32,
    pub contact: f32,
    pub hit_fuse: f32,
    pub hit_radius: f32,
    pub hit_damage_max: f32,
    pub hit_damage_min: f32,
    pub hit_paint_radius: f32,
}

/// All 19 specials (`SPECIALS` filtered by `SPECIAL_ORDER`, config.js
/// L411-468). The five `barrage` variants share [`Barrage`].
#[derive(Debug, Clone, Deserialize)]
pub struct SpecialSet {
    pub slam: Slam,
    pub storm: Storm,
    pub barrage: Barrage,
    #[serde(rename = "barrage_sticky")]
    pub barrage_sticky: Barrage,
    #[serde(rename = "barrage_burst")]
    pub barrage_burst: Barrage,
    #[serde(rename = "barrage_seeker")]
    pub barrage_seeker: Barrage,
    #[serde(rename = "barrage_mist")]
    pub barrage_mist: Barrage,
    pub bubbler: Bubbler,
    pub sonar: Sonar,
    pub strike: Strike,
    pub zooka: Zooka,
    pub wail: Wail,
    pub kraken: Kraken,
    pub blower: Blower,
    pub jetpack: Jetpack,
    pub stamp: Stamp,
    pub booyah: Booyah,
    pub zipcaster: Zipcaster,
    pub crab: Crab,
}

/// `SPECIALS.slam` — Tidal Slam (config.js L412).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slam {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub rise: f32,
    pub hang: f32,
    pub radius: f32,
    pub kill_radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
}

/// `SPECIALS.storm` — Ink Tempest (config.js L413).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Storm {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub radius: f32,
    pub dps: f32,
    pub throw_speed: f32,
    pub drift_speed: f32,
}

/// `SPECIALS.barrage*` — the five Bomb Barrage variants (config.js L416-420):
/// throw `bomb` (a `SUBS` id) every `gap` s for `duration` s.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Barrage {
    pub id: String,
    pub kind: String,
    pub bomb: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub gap: f32,
}

/// `SPECIALS.bubbler` — Bubble Guard (config.js L422-423).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bubbler {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub radius: f32,
    pub knock_per_damage: f32,
    pub knock_max: f32,
    pub share_range: f32,
}

/// `SPECIALS.sonar` — Deep Sonar (config.js L425-426).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sonar {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub slow: f32,
    pub ink_mul: f32,
    pub cast: f32,
}

/// `SPECIALS.strike` — Vortex Strike (config.js L428-429).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Strike {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub aim_time: f32,
    pub flight: f32,
    pub radius: f32,
    pub duration: f32,
    pub dps: f32,
    pub pull: f32,
}

/// `SPECIALS.zooka` — Twister Zooka (config.js L431-432).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Zooka {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub interval: f32,
    pub speed: f32,
    pub range: f32,
    pub damage: f32,
    pub height: f32,
    pub radius: f32,
    pub paint_every: f32,
    pub paint_radius: f32,
}

/// `SPECIALS.wail` — Howl Box (config.js L434-435).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wail {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub charge: f32,
    pub blast: f32,
    pub radius: f32,
    pub range: f32,
    pub dps: f32,
    pub hold_time: f32,
    pub hold_speed: f32,
}

/// `SPECIALS.kraken` — Kraken (config.js L437-438).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kraken {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub speed: f32,
    pub hop_vel: f32,
    pub attack_vel: f32,
    pub attack_fwd: f32,
    pub radius: f32,
    pub damage: f32,
    pub knock_per_damage: f32,
    pub cooldown: f32,
    pub paint_radius: f32,
}

/// `SPECIALS.blower` — Bubble Blower (config.js L440-441).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blower {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub max: u32,
    pub inflate: f32,
    pub r_min: f32,
    pub r_max: f32,
    pub drift: f32,
    pub life: f32,
    pub pop_damage: f32,
    pub blast_mul: f32,
    pub damage_max: f32,
    pub damage_min: f32,
}

/// `SPECIALS.jetpack` — Ink Jet (config.js L443-444).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Jetpack {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub height: f32,
    pub speed: f32,
    pub accel: f32,
    pub boost: f32,
    pub boost_gap: f32,
    pub interval: f32,
    pub proj_speed: f32,
    pub range: f32,
    pub direct_damage: f32,
    pub splash_max: f32,
    pub splash_min: f32,
    pub splash_radius: f32,
    pub paint_radius: f32,
}

/// `SPECIALS.stamp` — Mega Stamp (config.js L446-454).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub interval: f32,
    pub reach: f32,
    pub radius: f32,
    pub damage: f32,
    pub lunge: f32,
    pub move_speed: f32,
    pub throw_speed: f32,
    pub throw_radius: f32,
    pub throw_damage_max: f32,
    pub throw_damage_min: f32,
    pub turn_rate: f32,
    pub turn_rate_still: f32,
    pub strafe: f32,
    pub guard_time: f32,
    pub deflect_arc: f32,
    pub deflect_r: f32,
    pub bomb_clear_r: f32,
    pub flip_time: f32,
    pub flip_reach: f32,
    pub flip_radius: f32,
    pub flip_back_reach: f32,
    pub flip_back_radius: f32,
}

/// `SPECIALS.booyah` — Cheer Orb (config.js L456-457).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Booyah {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub charge: f32,
    pub cheer: f32,
    pub cheer_special: f32,
    pub auto_throw: f32,
    pub move_speed: f32,
    pub throw_speed: f32,
    pub fuse: f32,
    pub radius: f32,
    pub kill_radius: f32,
    pub damage_max: f32,
    pub damage_min: f32,
}

/// `SPECIALS.zipcaster` — Zipline (config.js L459-462).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Zipcaster {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub range: f32,
    pub speed: f32,
    pub hang: f32,
    pub cooldown: f32,
    pub impact_direct: f32,
    pub impact_splash: f32,
    pub impact_direct_r: f32,
    pub impact_radius: f32,
}

/// `SPECIALS.crab` — Crab Rig (config.js L465-467).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crab {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub duration: f32,
    pub hp: u32,
    pub speed: f32,
    pub roll_speed: f32,
    pub turn_rate: f32,
    pub gun_interval: f32,
    pub gun_damage: f32,
    pub gun_speed: f32,
    pub gun_range: f32,
    pub gun_spread: f32,
    pub cannon_gap: f32,
    pub cannon_speed: f32,
    pub cannon_radius: f32,
    pub cannon_damage_max: f32,
    pub cannon_damage_min: f32,
    pub roll_armor: f32,
}

/// `ZONES` — Zone Control rules (config.js L485-501).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZonesTuning {
    pub duration: u32,
    pub count: u32,
    pub rotate_min: u32,
    pub rotate_max: u32,
    pub final_centre: u32,
    pub warn: f32,
    pub control: f32,
    pub contest: f32,
    pub rate_center: f32,
    pub rate_home: f32,
    pub rate_away: f32,
    pub penalty_k: f32,
    pub gauge_held: f32,
    pub gauge_neutral: f32,
    pub overtime_grace: u32,
    pub overtime_max: u32,
    pub sample_hz: u32,
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

/// Bot tuning per difficulty level (`DIFFICULTY`, config.js L503-508).
#[derive(Debug, Clone, Deserialize)]
pub struct DifficultySet {
    pub easy: Difficulty,
    pub normal: Difficulty,
    pub hard: Difficulty,
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
        assert_eq!(t.schema, "inkwave.tuning.v2");
        assert_eq!(t.source.path, "src/config.js");
        assert_eq!(t.source.commit.len(), 40, "commit must be a full sha");
    }

    // TR-28.2: WEAPON_ORDER (config.js L303) is the roster; the retired
    // dualies/slosher/splatling defs (L266-299) never reach the artifact.
    #[test]
    fn weapon_order_locked_and_retired_excluded() {
        let t = tuning();
        assert_eq!(
            t.weapon_order,
            [
                "shooter", "twins", "brolly", "blaster", "spinner", "charger", "bow", "roller",
                "brush", "blade", "mitts", "bucket"
            ]
        );
        for retired in ["dualies", "slosher", "splatling"] {
            assert!(
                !crate::TUNING_JSON.contains(&format!("\"{retired}\"")),
                "{retired} leaked into tuning.json"
            );
        }
    }

    // The M1 `spritzer` alias must stay equal to `weapons.shooter` until the
    // Task 29 runner refactor migrates its consumers.
    #[test]
    fn spritzer_alias_matches_weapons_shooter() {
        let t = tuning();
        let (a, b) = (&t.spritzer, &t.weapons.shooter);
        assert_eq!(a.id, b.id);
        assert_eq!(a.fire_interval, b.fire_interval);
        assert_eq!(a.damage, b.damage);
        assert_eq!(a.ink_per_shot, b.ink_per_shot);
        assert_eq!(a.proj_speed, b.proj_speed);
        assert_eq!(a.special_cost, b.special_cost);
    }

    // weaponRange() (config.js L307-315) evaluated per weapon at extraction:
    // charger/spinner take rangeMax, roller/brush fixed, bucket reach, else range.
    #[test]
    fn weapon_range_table() {
        let t = tuning();
        let expected: [(&str, f32); 12] = [
            ("shooter", 12.5),
            ("twins", 11.0),
            ("brolly", 7.0),
            ("blaster", 10.5),
            ("spinner", 21.0),
            ("charger", 27.0),
            ("bow", 25.0),
            ("roller", 6.0),
            ("brush", 5.0),
            ("blade", 6.5),
            ("mitts", 5.3),
            ("bucket", 9.5),
        ];
        assert_eq!(t.weapon_range.len(), 12);
        for (id, want) in expected {
            assert_eq!(t.weapon_range[id], want, "weaponRange[{id}]");
        }
    }

    // TR-28.1: per-weapon key values, anchored to config.js line numbers.

    #[test]
    fn weapon_twins_values() {
        // config.js L158-168
        let w = &tuning().weapons.twins;
        assert_eq!(w.id, "twins");
        assert_eq!(w.kind, "twins");
        assert_eq!(w.class, "Pistols");
        assert_eq!(w.anim, "shooter");
        assert_eq!(w.fire_interval, 0.085);
        assert_eq!(w.damage, 30.0);
        assert_eq!(w.ink_per_shot, 0.85);
        assert_eq!(w.proj_speed, 31.0);
        assert_eq!(w.straight_time, 0.11);
        assert_eq!(w.range, 11.0);
        assert_eq!(w.spread_ground, 6.0);
        assert_eq!(w.spread_air, 11.0);
        assert_eq!(w.impact_radius, 0.78);
        assert_eq!(w.trail_radius, 0.4);
        assert_eq!(w.trail_every, 1.1);
        assert_eq!(w.move_speed_firing, 5.2);
        assert_eq!(w.roll_dist, 3.4);
        assert_eq!(w.roll_time, 0.3);
        assert_eq!(w.roll_ink, 7.0);
        assert_eq!(w.roll_charges, 2);
        assert_eq!(w.roll_lockout, 2.6);
        assert_eq!(w.roll_reset, 1.4);
        assert_eq!(w.turret_interval, 0.058);
        assert_eq!(w.turret_spread, 1.6);
        assert_eq!(w.turret_proj_speed, 36.0);
        assert_eq!(w.turret_straight, 0.16);
        assert_eq!(w.turret_range, 13.5);
        assert_eq!(w.special, "bubbler");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "seeker");
    }

    #[test]
    fn weapon_brolly_values() {
        // config.js L180-198
        let w = &tuning().weapons.brolly;
        assert_eq!(w.id, "brolly");
        assert_eq!(w.class, "Brolly");
        assert_eq!(w.anim, "shooter");
        assert_eq!(w.range, 7.0);
        assert_eq!(w.fire_interval, 0.45);
        assert_eq!(w.ink_per_shot, 5.5);
        assert_eq!(w.pellets, 11);
        assert_eq!(w.pellet_damage, 8.0);
        assert_eq!(w.pellet_damage_far, 2.5);
        assert_eq!(w.falloff_start, 2.8);
        assert_eq!(w.falloff_end, 6.5);
        assert_eq!(w.proj_speed, 25.0);
        assert_eq!(w.straight_time, 0.1);
        assert_eq!(w.pellet_grav, 30.0);
        assert_eq!(w.pellet_drag, 2.0);
        assert_eq!(w.pellet_life, 0.6);
        assert_eq!(w.pellet_size, 0.1);
        assert_eq!(w.pellet_paint, 0.5);
        assert_eq!(w.spread_deg, 8.0);
        assert_eq!(w.spread_air, 10.0);
        assert_eq!(w.move_speed_firing, 4.6);
        assert_eq!(w.move_speed_shield, 3.1);
        assert_eq!(w.canopy_hp, 450);
        assert_eq!(w.open_delay, 0.12);
        assert_eq!(w.open_time, 0.14);
        assert_eq!(w.close_time, 0.1);
        assert_eq!(w.launch_hold, 0.9);
        assert_eq!(w.regrow_time, 5.0);
        assert_eq!(w.canopy_regen, 60.0);
        assert_eq!(w.canopy_regen_delay, 2.0);
        assert_eq!(w.launch_speed, 2.6);
        assert_eq!(w.launch_life, 6.0);
        assert_eq!(w.launch_paint, 0.8);
        assert_eq!(w.special, "bubbler");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "tracer");
    }

    #[test]
    fn weapon_blaster_values() {
        // config.js L126-135
        let w = &tuning().weapons.blaster;
        assert_eq!(w.id, "blaster");
        assert_eq!(w.class, "Blaster");
        assert_eq!(w.fire_interval, 0.78);
        assert_eq!(w.direct_damage, 125.0);
        assert_eq!(w.splash_damage_max, 70.0);
        assert_eq!(w.splash_damage_min, 30.0);
        assert_eq!(w.splash_radius, 2.6);
        assert_eq!(w.ink_per_shot, 9.0);
        assert_eq!(w.proj_speed, 23.0);
        assert_eq!(w.range, 10.5);
        assert_eq!(w.impact_radius, 1.5);
        assert_eq!(w.burst_radius, 1.9);
        assert_eq!(w.move_speed_firing, 4.0);
        assert_eq!(w.special, "jetpack");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "burst");
    }

    #[test]
    fn weapon_spinner_values() {
        // config.js L148-157
        let w = &tuning().weapons.spinner;
        assert_eq!(w.id, "spinner");
        assert_eq!(w.anim, "charger");
        assert_eq!(w.charge_time, 1.0);
        assert_eq!(w.ink_full, 24.0);
        assert_eq!(w.burst_min, 0.35);
        assert_eq!(w.burst_max, 1.8);
        assert_eq!(w.fire_interval, 0.05);
        assert_eq!(w.damage, 26.0);
        assert_eq!(w.speed_min, 25.0);
        assert_eq!(w.speed_max, 40.0);
        assert_eq!(w.straight_min, 0.09);
        assert_eq!(w.straight_max, 0.24);
        assert_eq!(w.range_min, 10.0);
        assert_eq!(w.range_max, 21.0);
        assert_eq!(w.spread_ground, 2.8);
        assert_eq!(w.spread_air, 6.0);
        assert_eq!(w.impact_radius, 0.72);
        assert_eq!(w.trail_radius, 0.38);
        assert_eq!(w.trail_every, 1.25);
        assert_eq!(w.move_speed_charging, 3.3);
        assert_eq!(w.move_speed_firing, 3.0);
        assert_eq!(w.special, "crab");
        assert_eq!(w.special_cost, 200);
        assert_eq!(w.sub, "curtain");
    }

    #[test]
    fn weapon_charger_values() {
        // config.js L117-125
        let w = &tuning().weapons.charger;
        assert_eq!(w.id, "charger");
        assert_eq!(w.class, "Charger");
        assert_eq!(w.charge_time, 1.0);
        assert_eq!(w.range_min, 11.0);
        assert_eq!(w.range_max, 27.0);
        assert_eq!(w.damage_min, 40.0);
        assert_eq!(w.damage_max, 160.0);
        assert_eq!(w.ink_full, 18.0);
        assert_eq!(w.line_splat_every, 1.2);
        assert_eq!(w.line_radius, 0.55);
        assert_eq!(w.impact_radius, 1.2);
        assert_eq!(w.move_speed_firing, 1.8);
        assert_eq!(w.special, "sonar");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "mine");
    }

    #[test]
    fn weapon_bow_values() {
        // config.js L199-219
        let w = &tuning().weapons.bow;
        assert_eq!(w.id, "bow");
        assert_eq!(w.class, "Bow");
        assert_eq!(w.anim, "charger");
        assert_eq!(w.charge_time, 1.0);
        assert_eq!(w.ring1, 0.45);
        assert_eq!(w.ink_full, 16.0);
        assert_eq!(w.ink_min, 0.15);
        assert_eq!(w.range, 25.0);
        assert_eq!(w.flight_tap, [5.0, 7.5]);
        assert_eq!(w.flight_ring, [12.5, 16.5]);
        assert_eq!(w.flight_full, 21.0);
        assert_eq!(w.speed_tap, 32.0);
        assert_eq!(w.speed_ring, [42.0, 48.0]);
        assert_eq!(w.speed_full, 58.0);
        assert_eq!(w.straight, 0.05);
        assert_eq!(w.grav, 6.0);
        assert_eq!(w.dive_grav, 140.0);
        assert_eq!(w.dive_drag, 6.0);
        assert_eq!(w.damage_tap, [14.0, 20.0]);
        assert_eq!(w.damage_ring, [30.0, 38.0]);
        assert_eq!(w.damage_full, 55.0);
        assert_eq!(w.side_mul, 0.8);
        assert_eq!(w.side_full, 45.0);
        assert_eq!(w.fan_tap, 8.0);
        assert_eq!(w.fan_ring, 6.0);
        assert_eq!(w.fan_full, 4.5);
        assert_eq!(w.fuse_ring, 0.7);
        assert_eq!(w.fuse_full, 0.55);
        assert_eq!(w.burst_radius, [1.35, 1.8]);
        assert_eq!(w.burst_inner, 0.6);
        assert_eq!(w.burst_damage, [30.0, 45.0]);
        assert_eq!(w.burst_edge, [12.0, 15.0]);
        assert_eq!(w.paint_tap, 0.55);
        assert_eq!(w.paint_stick, 0.42);
        assert_eq!(w.burst_paint, [1.2, 1.55]);
        assert_eq!(w.trail_every, 2.4);
        assert_eq!(w.trail_radius, 0.32);
        assert_eq!(w.cooldown, 0.22);
        assert_eq!(w.move_speed_drawing, 2.6);
        assert_eq!(w.move_speed_firing, 4.2);
        assert_eq!(w.special, "strike");
        assert_eq!(w.special_cost, 190);
        assert_eq!(w.sub, "waddle");
    }

    #[test]
    fn weapon_roller_values() {
        // config.js L106-116
        let w = &tuning().weapons.roller;
        assert_eq!(w.id, "roller");
        assert_eq!(w.class, "Roller");
        assert_eq!(w.roll_speed, 4.4);
        assert_eq!(w.roll_width, 1.9);
        assert_eq!(w.roll_ink_per_meter, 1.1);
        assert_eq!(w.roll_damage, 140.0);
        assert_eq!(w.flick_interval, 0.62);
        assert_eq!(w.flick_windup, 0.22);
        assert_eq!(w.flick_ink, 9.0);
        assert_eq!(w.flick_drops, 9);
        assert_eq!(w.flick_damage_near, 125.0);
        assert_eq!(w.flick_damage_far, 30.0);
        assert_eq!(w.flick_speed, 17.0);
        assert_eq!(w.flick_spread_deg, 34.0);
        assert_eq!(w.impact_radius, 1.0);
        assert_eq!(w.move_speed_firing, 4.4);
        assert_eq!(w.special, "kraken");
        assert_eq!(w.special_cost, 170);
        assert_eq!(w.sub, "sticky");
    }

    #[test]
    fn weapon_brush_values() {
        // config.js L169-178
        let w = &tuning().weapons.brush;
        assert_eq!(w.id, "brush");
        assert_eq!(w.class, "Brush");
        assert_eq!(w.anim, "roller");
        assert_eq!(w.brush_speed, 7.4);
        assert_eq!(w.brush_width, 0.95);
        assert_eq!(w.brush_ink_per_meter, 0.32);
        assert_eq!(w.brush_damage, 30.0);
        assert_eq!(w.brush_hit_cd, 0.35);
        assert_eq!(w.swipe_interval, 0.16);
        assert_eq!(w.swipe_ink, 2.2);
        assert_eq!(w.swipe_drops, 5);
        assert_eq!(w.swipe_speed, 12.5);
        assert_eq!(w.swipe_spread_deg, 40.0);
        assert_eq!(w.swipe_damage_near, 26.0);
        assert_eq!(w.swipe_damage_far, 12.0);
        assert_eq!(w.impact_radius, 0.7);
        assert_eq!(w.move_speed_firing, 5.8);
        assert_eq!(w.special, "stamp");
        assert_eq!(w.special_cost, 170);
        assert_eq!(w.sub, "mist");
    }

    #[test]
    fn weapon_blade_values() {
        // config.js L220-238
        let w = &tuning().weapons.blade;
        assert_eq!(w.id, "blade");
        assert_eq!(w.class, "Cutlass");
        assert_eq!(w.anim, "blade");
        assert_eq!(w.range, 6.5);
        assert_eq!(w.proj_speed, 17.0);
        assert_eq!(w.tap_interval, 0.27);
        assert_eq!(w.tap_ink, 3.2);
        assert_eq!(w.tap_drops, 7);
        assert_eq!(w.tap_spread_deg, 66.0);
        assert_eq!(w.tap_speed, 17.5);
        assert_eq!(w.tap_straight, 0.07);
        assert_eq!(w.tap_damage_near, 40.0);
        assert_eq!(w.tap_damage_far, 20.0);
        assert_eq!(w.tap_drop_radius, 0.64);
        assert_eq!(w.tap_melee, 35.0);
        assert_eq!(w.charge_delay, 0.14);
        assert_eq!(w.charge_time, 0.62);
        assert_eq!(w.charge_store, 1.0);
        assert_eq!(w.heavy_ink, 9.0);
        assert_eq!(w.heavy_interval, 0.46);
        assert_eq!(w.heavy_melee, 130.0);
        assert_eq!(w.lunge_dist, 1.05);
        assert_eq!(w.lunge_time, 0.13);
        assert_eq!(w.stroke_radius, 0.6);
        assert_eq!(w.wave_damage, 70.0);
        assert_eq!(w.wave_damage_far, 50.0);
        assert_eq!(w.wave_speed, 26.0);
        assert_eq!(w.wave_range, 12.0);
        assert_eq!(w.wave_width, 0.7);
        assert_eq!(w.wave_height, 1.7);
        assert_eq!(w.wave_paint_every, 0.7);
        assert_eq!(w.wave_paint_radius, 0.62);
        assert_eq!(w.wave_end_radius, 1.05);
        assert_eq!(w.reach, 1.5);
        assert_eq!(w.melee_arc_deg, 150.0);
        assert_eq!(w.heavy_arc_deg, 110.0);
        assert_eq!(w.move_speed_firing, 5.6);
        assert_eq!(w.move_speed_charging, 4.4);
        assert_eq!(w.special, "zipcaster");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "shaker");
    }

    #[test]
    fn weapon_mitts_values() {
        // config.js L239-264
        let w = &tuning().weapons.mitts;
        assert_eq!(w.id, "mitts");
        assert_eq!(w.class, "Mitts");
        assert_eq!(w.anim, "shooter");
        assert_eq!(w.range, 5.3);
        assert_eq!(w.punch_interval, 0.12);
        assert_eq!(w.ink_per_punch, 1.6);
        assert_eq!(w.punch_spread, 2.5);
        assert_eq!(w.fist_speed, 28.0);
        assert_eq!(w.fist_range, 4.6);
        assert_eq!(w.fist_size, 0.2);
        assert_eq!(w.guard_armor, 0.5);
        assert_eq!(w.guard_arc, 70.0);
        assert_eq!(w.guard_after, 0.25);
        assert_eq!(w.bot_approach, 1.0);
        assert_eq!(w.punch_damage, 38.0);
        assert_eq!(w.splash_radius, 1.4);
        assert_eq!(w.splash_max, 22.0);
        assert_eq!(w.splash_min, 10.0);
        assert_eq!(w.fist_paint, 0.95);
        assert_eq!(w.move_speed_firing, 5.4);
        assert_eq!(w.leap_charge_time, 0.45);
        assert_eq!(w.leap_ink_min, 9.0);
        assert_eq!(w.leap_ink_max, 15.0);
        assert_eq!(w.move_speed_charging, 1.1);
        assert_eq!(w.leap_armor, 0.35);
        assert_eq!(w.leap_armor_grace, 0.15);
        assert_eq!(w.leap_speed_min, 10.9);
        assert_eq!(w.leap_speed_max, 18.6);
        assert_eq!(w.leap_angle, 40.0);
        assert_eq!(w.leap_angle_min, 18.0);
        assert_eq!(w.leap_angle_max, 60.0);
        assert_eq!(w.leap_pitch_k, 0.55);
        assert_eq!(w.leap_vy_max, 13.0);
        assert_eq!(w.land_radius, 2.8);
        assert_eq!(w.land_damage_max, 100.0);
        assert_eq!(w.land_damage_min, 35.0);
        assert_eq!(w.land_paint, 2.5);
        assert_eq!(w.land_invuln, 0.5);
        assert_eq!(w.glove_radius, 1.1);
        assert_eq!(w.glove_damage, 40.0);
        assert_eq!(w.cling_drain, 8.0);
        assert_eq!(w.special, "slam");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "boomerang");
    }

    #[test]
    fn weapon_bucket_values() {
        // config.js L137-147
        let w = &tuning().weapons.bucket;
        assert_eq!(w.id, "bucket");
        assert_eq!(w.class, "Bucket");
        assert_eq!(w.anim, "blaster");
        assert_eq!(w.fire_interval, 0.6);
        assert_eq!(w.windup, 0.12);
        assert_eq!(w.ink_per_shot, 6.0);
        assert_eq!(w.throw_speed, 15.5);
        assert_eq!(w.lob, 0.3);
        assert_eq!(w.blobs, 5);
        assert_eq!(w.gravity, 20.0);
        assert_eq!(w.drag, 0.25);
        assert_eq!(w.reach, 9.5);
        assert_eq!(w.damage, 55.0);
        assert_eq!(w.impact_radius, 1.05);
        assert_eq!(w.below_falloff, 0.12);
        assert_eq!(w.below_falloff_max, 0.35);
        assert_eq!(w.move_speed_firing, 4.2);
        assert_eq!(w.special, "blower");
        assert_eq!(w.special_cost, 180);
        assert_eq!(w.sub, "sprinkler");
    }

    // TR-28.1: sub / special / zones / difficulty anchors.

    #[test]
    fn sub_order_locked() {
        // config.js L407
        let t = tuning();
        assert_eq!(
            t.sub_order,
            [
                "bomb",
                "sticky",
                "burst",
                "shaker",
                "seeker",
                "waddle",
                "torpedo",
                "tracer",
                "boomerang",
                "scan",
                "curtain",
                "sprinkler",
                "mine",
                "beacon",
                "mist"
            ]
        );
    }

    #[test]
    fn sub_bomb_family_values() {
        let t = tuning();
        // config.js L319-322 (bomb)
        let s = &t.subs.bomb;
        assert_eq!(s.id, "bomb");
        assert_eq!(s.ink_cost, 70.0);
        assert_eq!(s.throw_speed, 13.5);
        assert_eq!(s.fuse, 0.95);
        assert_eq!(s.radius, 3.1);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.damage_min, 35.0);
        assert_eq!(s.paint_radius, 2.7);
        // config.js L323-326 (sticky)
        let s = &t.subs.sticky;
        assert_eq!(s.ink_cost, 70.0);
        assert_eq!(s.fuse, 2.4);
        assert_eq!(s.radius, 4.0);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.paint_radius, 3.3);
        // config.js L327-330 (burst)
        let s = &t.subs.burst;
        assert_eq!(s.ink_cost, 40.0);
        assert_eq!(s.throw_speed, 16.0);
        assert_eq!(s.radius, 2.1);
        assert_eq!(s.direct_damage, 60.0);
        assert_eq!(s.splash_damage, 35.0);
        assert_eq!(s.paint_radius, 1.8);
    }

    #[test]
    fn sub_tracked_values() {
        let t = tuning();
        // config.js L331-339 (seeker)
        let s = &t.subs.seeker;
        assert_eq!(s.ink_cost, 65.0);
        assert_eq!(s.throw_speed, 9.0);
        assert_eq!(s.speed, 6.3);
        assert_eq!(s.seek_range, 15.0);
        assert_eq!(s.life, 4.5);
        assert_eq!(s.trail_radius, 0.6);
        assert_eq!(s.trigger_dist, 1.2);
        assert_eq!(s.turn_rate, 1.75);
        assert_eq!(s.commit_dist, 2.5);
        assert_eq!(s.creep, 0.6);
        assert_eq!(s.radius, 2.8);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.damage_min, 35.0);
        assert_eq!(s.paint_radius, 2.4);
        // config.js L340-343 (scan)
        let s = &t.subs.scan;
        assert_eq!(s.ink_cost, 55.0);
        assert_eq!(s.throw_speed, 14.0);
        assert_eq!(s.fuse, 1.1);
        assert_eq!(s.radius, 4.2);
        assert_eq!(s.cloud_time, 0.9);
        assert_eq!(s.track_time, 8.0);
        // config.js L360-363 (mist)
        let s = &t.subs.mist;
        assert_eq!(s.ink_cost, 60.0);
        assert_eq!(s.fuse, 1.0);
        assert_eq!(s.radius, 3.3);
        assert_eq!(s.mist_time, 4.5);
        assert_eq!(s.slow, 0.55);
        assert_eq!(s.ink_drain, 12.0);
    }

    #[test]
    fn sub_placed_values() {
        let t = tuning();
        // config.js L344-347 (curtain)
        let s = &t.subs.curtain;
        assert_eq!(s.ink_cost, 55.0);
        assert_eq!(s.width, 3.4);
        assert_eq!(s.height, 2.7);
        assert_eq!(s.hp, 170);
        assert_eq!(s.decay, 19.0);
        assert_eq!(s.shot_mul, 0.5);
        // config.js L348-351 (sprinkler)
        let s = &t.subs.sprinkler;
        assert_eq!(s.ink_cost, 60.0);
        assert_eq!(s.hp, 70);
        assert_eq!(s.pulse, 0.3);
        assert_eq!(s.drops, 6);
        assert_eq!(s.spray_radius, 3.2);
        assert_eq!(s.spray_fade, 12.0);
        assert_eq!(s.drop_damage, 8.0);
        // config.js L352-355 (mine)
        let s = &t.subs.mine;
        assert_eq!(s.ink_cost, 55.0);
        assert!(s.placed);
        assert_eq!(s.max, 2);
        assert_eq!(s.trigger_radius, 2.1);
        assert_eq!(s.arm_time, 0.9);
        assert_eq!(s.delay, 0.35);
        assert_eq!(s.radius, 2.6);
        assert_eq!(s.damage, 45.0);
        assert_eq!(s.track_time, 8.0);
        assert_eq!(s.paint_radius, 2.0);
        // config.js L356-359 (beacon)
        let s = &t.subs.beacon;
        assert_eq!(s.ink_cost, 70.0);
        assert!(s.placed);
        assert_eq!(s.max, 3);
        assert_eq!(s.uses, 2);
        assert_eq!(s.hp, 50);
    }

    #[test]
    fn sub_kit_values() {
        let t = tuning();
        // config.js L365-374 (shaker)
        let s = &t.subs.shaker;
        assert_eq!(s.ink_cost, 60.0);
        assert_eq!(s.charge_time, 2.7);
        assert_eq!(s.move_boost, 0.9);
        assert_eq!(s.jump_boost, 0.075);
        assert_eq!(s.shake_boost, 0.05);
        assert_eq!(s.max_rate, 1.6);
        assert_eq!(s.fuse, 0.5);
        assert_eq!(s.gap, 0.42);
        assert_eq!(s.hop_speed, 4.2);
        assert_eq!(s.hop_up, 4.6);
        assert_eq!(s.radius, 2.3);
        assert_eq!(s.damage_max, 110.0);
        assert_eq!(s.damage_min, 30.0);
        assert_eq!(s.paint_radius, 2.0);
        assert_eq!(s.trail_every, 0.38);
        assert_eq!(s.trail_radius, 0.3);
        // config.js L375-381 (waddle)
        let s = &t.subs.waddle;
        assert_eq!(s.ink_cost, 65.0);
        assert_eq!(s.sense_radius, 7.5);
        assert_eq!(s.sense_up, 4.0);
        assert_eq!(s.fuse, 1.1);
        assert_eq!(s.speed, 4.0);
        assert_eq!(s.turn_rate, 8.0);
        assert_eq!(s.life, 9.0);
        assert_eq!(s.max_travel, 26.0);
        assert_eq!(s.trigger_dist, 1.2);
        assert_eq!(s.radius, 3.0);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.damage_min, 35.0);
        assert_eq!(s.paint_radius, 2.6);
        assert_eq!(s.hp, 30);
        // config.js L382-391 (torpedo)
        let s = &t.subs.torpedo;
        assert_eq!(s.ink_cost, 65.0);
        assert_eq!(s.feet_paint, 1.25);
        assert_eq!(s.arm_time, 0.12);
        assert_eq!(s.lock_range, 6.5);
        assert_eq!(s.fly_life, 2.5);
        assert_eq!(s.unfold_time, 0.5);
        assert_eq!(s.launch_speed0, 2.2);
        assert_eq!(s.launch_speed, 9.5);
        assert_eq!(s.launch_accel, 8.0);
        assert_eq!(s.turn_rate, 1.9);
        assert_eq!(s.launch_life, 2.8);
        assert_eq!(s.hp, 20);
        assert_eq!(s.radius, 2.4);
        assert_eq!(s.core_radius, 1.2);
        assert_eq!(s.damage_max, 60.0);
        assert_eq!(s.damage_min, 35.0);
        assert_eq!(s.paint_radius, 3.6);
        assert_eq!(s.fallback_paint, 1.5);
        assert_eq!(s.drops, 10);
        assert_eq!(s.drop_damage, 12.0);
        assert_eq!(s.drop_hits, 3);
        assert_eq!(s.drop_paint, 0.65);
        // config.js L392-396 (tracer)
        let s = &t.subs.tracer;
        assert_eq!(s.ink_cost, 40.0);
        assert_eq!(s.speed, 40.0);
        assert_eq!(s.range, 32.0);
        assert_eq!(s.pitch_down, 9.0);
        assert_eq!(s.floor_exit, 0.3);
        assert_eq!(s.max_bounces, 12);
        assert_eq!(s.size, 0.16);
        assert_eq!(s.puddle_radius, 1.35);
        assert_eq!(s.direct_damage, 35.0);
        assert_eq!(s.direct_mark, 9.0);
        assert_eq!(s.trail_life, 1.0);
        assert_eq!(s.trail_radius, 0.42);
        assert_eq!(s.trail_damage, 22.0);
        assert_eq!(s.trail_mark, 3.0);
        // config.js L397-405 (boomerang)
        let s = &t.subs.boomerang;
        assert_eq!(s.ink_cost, 60.0);
        assert_eq!(s.throw_speed, 16.0);
        assert_eq!(s.range, 9.0);
        assert_eq!(s.out_time, 0.6);
        assert_eq!(s.hover, 1.75);
        assert_eq!(s.hover_radius, 1.35);
        assert_eq!(s.tick_rate, 8);
        assert_eq!(s.tick_damage, 9.0);
        assert_eq!(s.hover_paint, 1.0);
        assert_eq!(s.return_speed, 17.0);
        assert_eq!(s.orbit, 2.5);
        assert_eq!(s.orbit_radius, 1.5);
        assert_eq!(s.orbit_spin, 7.0);
        assert_eq!(s.orbit_damage, 6.0);
        assert_eq!(s.orbit_hit_cd, 0.3);
        assert_eq!(s.radius, 2.4);
        assert_eq!(s.damage_max, 100.0);
        assert_eq!(s.damage_min, 30.0);
        assert_eq!(s.paint_radius, 2.2);
        assert_eq!(s.contact, 0.3);
        assert_eq!(s.hit_fuse, 0.6);
        assert_eq!(s.hit_radius, 3.1);
        assert_eq!(s.hit_damage_max, 180.0);
        assert_eq!(s.hit_damage_min, 35.0);
        assert_eq!(s.hit_paint_radius, 2.7);
    }

    #[test]
    fn special_order_locked() {
        // config.js L469
        let t = tuning();
        assert_eq!(
            t.special_order,
            [
                "slam",
                "storm",
                "barrage",
                "barrage_sticky",
                "barrage_burst",
                "barrage_seeker",
                "barrage_mist",
                "bubbler",
                "sonar",
                "strike",
                "zooka",
                "wail",
                "kraken",
                "blower",
                "jetpack",
                "stamp",
                "booyah",
                "zipcaster",
                "crab"
            ]
        );
        assert_eq!(t.special_order.len(), 19);
    }

    #[test]
    fn special_instant_values() {
        let t = tuning();
        // config.js L412 (slam)
        let s = &t.specials.slam;
        assert_eq!(s.rise, 0.55);
        assert_eq!(s.hang, 0.25);
        assert_eq!(s.radius, 5.2);
        assert_eq!(s.kill_radius, 3.2);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.damage_min, 55.0);
        // config.js L413 (storm)
        let s = &t.specials.storm;
        assert_eq!(s.duration, 6.5);
        assert_eq!(s.radius, 3.4);
        assert_eq!(s.dps, 34.0);
        assert_eq!(s.throw_speed, 16.0);
        assert_eq!(s.drift_speed, 1.1);
        // config.js L422-423 (bubbler)
        let s = &t.specials.bubbler;
        assert_eq!(s.duration, 6.5);
        assert_eq!(s.radius, 1.3);
        assert_eq!(s.knock_per_damage, 0.045);
        assert_eq!(s.knock_max, 6.5);
        assert_eq!(s.share_range, 1.5);
        // config.js L425-426 (sonar)
        let s = &t.specials.sonar;
        assert_eq!(s.duration, 8.0);
        assert_eq!(s.slow, 0.8);
        assert_eq!(s.ink_mul, 1.4);
        assert_eq!(s.cast, 0.6);
        // config.js L428-429 (strike)
        let s = &t.specials.strike;
        assert_eq!(s.aim_time, 7.0);
        assert_eq!(s.flight, 2.2);
        assert_eq!(s.radius, 5.5);
        assert_eq!(s.duration, 4.5);
        assert_eq!(s.dps, 62.0);
        assert_eq!(s.pull, 1.6);
    }

    #[test]
    fn special_barrage_variants_values() {
        // config.js L416-420: five variants, same duration, distinct bomb/gap.
        let t = tuning();
        let cases: [(&super::Barrage, &str, &str, f32); 5] = [
            (&t.specials.barrage, "barrage", "bomb", 0.3),
            (&t.specials.barrage_sticky, "barrage_sticky", "sticky", 0.4),
            (&t.specials.barrage_burst, "barrage_burst", "burst", 0.2),
            (&t.specials.barrage_seeker, "barrage_seeker", "seeker", 0.5),
            (&t.specials.barrage_mist, "barrage_mist", "mist", 0.45),
        ];
        for (b, id, bomb, gap) in cases {
            assert_eq!(b.id, id);
            assert_eq!(b.kind, "barrage");
            assert_eq!(b.bomb, bomb);
            assert_eq!(b.duration, 6.5);
            assert_eq!(b.gap, gap);
        }
    }

    #[test]
    fn special_ballistic_values() {
        let t = tuning();
        // config.js L431-432 (zooka)
        let s = &t.specials.zooka;
        assert_eq!(s.duration, 6.0);
        assert_eq!(s.interval, 1.0);
        assert_eq!(s.speed, 34.0);
        assert_eq!(s.range, 44.0);
        assert_eq!(s.damage, 180.0);
        assert_eq!(s.height, 2.8);
        assert_eq!(s.radius, 0.55);
        assert_eq!(s.paint_every, 0.7);
        assert_eq!(s.paint_radius, 0.95);
        // config.js L434-435 (wail)
        let s = &t.specials.wail;
        assert_eq!(s.charge, 1.3);
        assert_eq!(s.blast, 3.2);
        assert_eq!(s.radius, 1.5);
        assert_eq!(s.range, 72.0);
        assert_eq!(s.dps, 260.0);
        assert_eq!(s.hold_time, 6.0);
        assert_eq!(s.hold_speed, 3.2);
        // config.js L437-438 (kraken)
        let s = &t.specials.kraken;
        assert_eq!(s.duration, 7.0);
        assert_eq!(s.speed, 7.2);
        assert_eq!(s.hop_vel, 10.5);
        assert_eq!(s.attack_vel, 8.5);
        assert_eq!(s.attack_fwd, 6.5);
        assert_eq!(s.radius, 2.3);
        assert_eq!(s.damage, 200.0);
        assert_eq!(s.knock_per_damage, 0.02);
        assert_eq!(s.cooldown, 0.65);
        assert_eq!(s.paint_radius, 1.15);
    }

    #[test]
    fn special_field_values() {
        let t = tuning();
        // config.js L440-441 (blower)
        let s = &t.specials.blower;
        assert_eq!(s.duration, 9.0);
        assert_eq!(s.max, 3);
        assert_eq!(s.inflate, 1.0);
        assert_eq!(s.r_min, 0.9);
        assert_eq!(s.r_max, 2.1);
        assert_eq!(s.drift, 1.4);
        assert_eq!(s.life, 9.0);
        assert_eq!(s.pop_damage, 55.0);
        assert_eq!(s.blast_mul, 1.9);
        assert_eq!(s.damage_max, 180.0);
        assert_eq!(s.damage_min, 45.0);
        // config.js L443-444 (jetpack)
        let s = &t.specials.jetpack;
        assert_eq!(s.duration, 7.0);
        assert_eq!(s.height, 3.8);
        assert_eq!(s.speed, 5.5);
        assert_eq!(s.accel, 11.0);
        assert_eq!(s.boost, 9.0);
        assert_eq!(s.boost_gap, 0.9);
        assert_eq!(s.interval, 0.55);
        assert_eq!(s.proj_speed, 30.0);
        assert_eq!(s.range, 34.0);
        assert_eq!(s.direct_damage, 125.0);
        assert_eq!(s.splash_max, 70.0);
        assert_eq!(s.splash_min, 30.0);
        assert_eq!(s.splash_radius, 2.4);
        assert_eq!(s.paint_radius, 1.5);
    }

    #[test]
    fn special_driver_values() {
        let t = tuning();
        // config.js L446-454 (stamp)
        let s = &t.specials.stamp;
        assert_eq!(s.duration, 7.5);
        assert_eq!(s.interval, 0.42);
        assert_eq!(s.reach, 1.6);
        assert_eq!(s.radius, 1.55);
        assert_eq!(s.damage, 200.0);
        assert_eq!(s.lunge, 2.6);
        assert_eq!(s.move_speed, 7.2);
        assert_eq!(s.throw_speed, 25.0);
        assert_eq!(s.throw_radius, 3.2);
        assert_eq!(s.throw_damage_max, 200.0);
        assert_eq!(s.throw_damage_min, 60.0);
        assert_eq!(s.turn_rate, 1.7);
        assert_eq!(s.turn_rate_still, 6.0);
        assert_eq!(s.strafe, 0.35);
        assert_eq!(s.guard_time, 0.45);
        assert_eq!(s.deflect_arc, 70.0);
        assert_eq!(s.deflect_r, 2.6);
        assert_eq!(s.bomb_clear_r, 3.2);
        assert_eq!(s.flip_time, 0.5);
        assert_eq!(s.flip_reach, 2.7);
        assert_eq!(s.flip_radius, 1.9);
        assert_eq!(s.flip_back_reach, 1.2);
        assert_eq!(s.flip_back_radius, 1.6);
        // config.js L456-457 (booyah)
        let s = &t.specials.booyah;
        assert_eq!(s.charge, 4.5);
        assert_eq!(s.cheer, 0.12);
        assert_eq!(s.cheer_special, 12.0);
        assert_eq!(s.auto_throw, 2.5);
        assert_eq!(s.move_speed, 1.8);
        assert_eq!(s.throw_speed, 19.6);
        assert_eq!(s.fuse, 1.5);
        assert_eq!(s.radius, 8.4);
        assert_eq!(s.kill_radius, 4.6);
        assert_eq!(s.damage_max, 220.0);
        assert_eq!(s.damage_min, 60.0);
        // config.js L459-462 (zipcaster)
        let s = &t.specials.zipcaster;
        assert_eq!(s.duration, 8.5);
        assert_eq!(s.range, 21.0);
        assert_eq!(s.speed, 22.0);
        assert_eq!(s.hang, 1.4);
        assert_eq!(s.cooldown, 0.35);
        assert_eq!(s.impact_direct, 100.0);
        assert_eq!(s.impact_splash, 30.0);
        assert_eq!(s.impact_direct_r, 1.0);
        assert_eq!(s.impact_radius, 2.4);
        // config.js L465-467 (crab)
        let s = &t.specials.crab;
        assert_eq!(s.duration, 9.0);
        assert_eq!(s.hp, 460);
        assert_eq!(s.speed, 3.0);
        assert_eq!(s.roll_speed, 9.5);
        assert_eq!(s.turn_rate, 1.5);
        assert_eq!(s.gun_interval, 0.075);
        assert_eq!(s.gun_damage, 18.0);
        assert_eq!(s.gun_speed, 36.0);
        assert_eq!(s.gun_range, 22.0);
        assert_eq!(s.gun_spread, 3.5);
        assert_eq!(s.cannon_gap, 1.15);
        assert_eq!(s.cannon_speed, 17.0);
        assert_eq!(s.cannon_radius, 3.0);
        assert_eq!(s.cannon_damage_max, 150.0);
        assert_eq!(s.cannon_damage_min, 40.0);
        assert_eq!(s.roll_armor, 0.35);
    }

    // TR-28.1: ZONES (config.js L485-501) — consumed by Task 36.
    #[test]
    fn zones_values() {
        let z = tuning().zones;
        assert_eq!(z.duration, 300);
        assert_eq!(z.count, 100);
        assert_eq!(z.rotate_min, 30);
        assert_eq!(z.rotate_max, 60);
        assert_eq!(z.final_centre, 30);
        assert_eq!(z.warn, 0.3);
        assert_eq!(z.control, 0.8);
        assert_eq!(z.contest, 0.4);
        assert_eq!(z.rate_center, 1.0);
        assert_eq!(z.rate_home, 0.5);
        assert_eq!(z.rate_away, 2.0);
        assert_eq!(z.penalty_k, 0.75);
        assert_eq!(z.gauge_held, 4.5);
        assert_eq!(z.gauge_neutral, 1.5);
        assert_eq!(z.overtime_grace, 10);
        assert_eq!(z.overtime_max, 300);
        assert_eq!(z.sample_hz, 5);
    }

    // TR-28.1: DIFFICULTY normal/hard (config.js L506-507) — consumed by Task 38.
    #[test]
    fn difficulty_normal_hard_values() {
        let t = tuning();
        let n = &t.difficulty.normal;
        assert_eq!(n.id, "normal");
        assert_eq!(n.name, "Fresh");
        assert_eq!(n.reaction, 0.32);
        assert_eq!(n.aim_error, 0.06);
        assert_eq!(n.fire_discipline, 0.8);
        assert_eq!(n.awareness, 21.0);
        assert_eq!(n.aim_omega, 13.0);
        assert_eq!(n.aim_turn, 10.0);
        let h = &t.difficulty.hard;
        assert_eq!(h.id, "hard");
        assert_eq!(h.name, "Fierce");
        assert_eq!(h.reaction, 0.17);
        assert_eq!(h.aim_error, 0.03);
        assert_eq!(h.fire_discipline, 0.95);
        assert_eq!(h.awareness, 26.0);
        assert_eq!(h.aim_omega, 18.0);
        assert_eq!(h.aim_turn, 14.0);
    }
}
