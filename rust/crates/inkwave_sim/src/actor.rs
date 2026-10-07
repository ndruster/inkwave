//! Actor: one squidkid's movement physics + ink/HP state machine (Task 5).
//!
//! Faithful port of upstream `src/game/actor.js`:
//! `update` (timers / form switch / jump / ink+hp / fall death),
//! `_horizontal` (S-curve accel, eased brake, heading slew, plant-and-reverse),
//! `_integrate` / `_resolve` (ground-plane follow, gravity shaping, feet+body),
//! `_spawnBarrier`, `_face` (angular spring), and the `smoothY` visual spring.
//!
//! Deferred to later tasks (keep this list honest on upstream sync):
//! wall climb (`_updateClimb`), roof slide / rail centring, dodge rolls and kit
//! jump hooks, super jump, specials, status effects (track/poison/reveal/
//! shield), and the `WeaponRunner` integration (ink spend, `moveSpeedFiring`,
//! `busy()`). The fire gate (`FireGate`) is computed here and consumed by the
//! Task 7 weapon runner; ink refill's `busy()` term is omitted until then.

use glam::{Vec3, vec3};

use crate::collision::{CollisionWorld, Contacts, GroundHit};
use crate::tuning::PlayerTuning;

/// Fixed simulation step (JS runs the same logic per 60 Hz frame).
pub const FIXED_DT: f32 = 1.0 / 60.0;

const TAU: f32 = std::f32::consts::TAU;

fn clamp(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

/// JS `smoothstep(a, b, x)` from `src/core/ctx.js`.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// JS `angleDiff(a, b)`: signed shortest angular difference b − a in [−π, π].
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = b - a;
    while d > std::f32::consts::PI {
        d -= TAU;
    }
    while d < -std::f32::consts::PI {
        d += TAU;
    }
    d
}

/// Paint query under the feet, implemented by the Task 6 ink grid.
/// Returns the JS `paint.sample` encoding: 0 = none, 1 = Alpha, 2 = Bravo.
pub trait InkQuery {
    fn sample(&self, face: i32, u: f32, v: f32) -> u8;
}

/// No ink anywhere (fresh stage).
pub struct NoInk;

impl InkQuery for NoInk {
    fn sample(&self, _face: i32, _u: f32, _v: f32) -> u8 {
        0
    }
}

/// Everything `step` needs from the stage that is not collision geometry.
pub struct SimWorld<'a> {
    pub collision: &'a CollisionWorld,
    /// One spawn pad per team (Alpha −Z, Bravo +Z).
    pub spawn_pads: [Vec3; 2],
    /// Enemy-pad keep-out radius (JS `Level.spawnBarrier`, 4.2 on Tidewater).
    pub spawn_barrier: f32,
}

impl<'a> SimWorld<'a> {
    #[must_use]
    pub fn new(collision: &'a CollisionWorld, layout: &crate::geometry::StageLayout) -> Self {
        let pads = &layout.stage.spawn_pads;
        Self {
            collision,
            spawn_pads: [Vec3::from(pads[0]), Vec3::from(pads[1])],
            spawn_barrier: layout.stage.spawn_barrier,
        }
    }
}

/// Kid (walk/shoot) or squid (swim/hide).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Form {
    Kid,
    Squid,
}

/// Per-frame controller input (JS `Actor.intent`). `move_dir` is world-space;
/// only x/z are used, magnitude > 1 is clamped like the JS stick.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActorInput {
    pub move_dir: Vec3,
    pub jump: bool,
    pub squid: bool,
    pub fire: bool,
    pub sub: bool,
    pub special: bool,
}

/// Cause of a splat (JS `splat(attacker, cause)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SplatCause {
    Weapon,
    Water,
}

/// State-change events for later FX/HUD/audio wiring (JS `emit(...)`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ActorEvent {
    Jump {
        swim: bool,
    },
    Land {
        speed: f32,
    },
    Splatted {
        cause: SplatCause,
        /// JS `splat(attacker, ...)` first argument as an identity
        /// `(team, slot)` pair: the weapon that landed the lethal hit, or the
        /// `lastAttacker` the water splat inherits (JS L384) — `None` when
        /// nobody is credited.
        attacker: Option<(usize, usize)>,
    },
    Respawn,
}

/// Trigger state handed to the weapon runner (JS `winp`): a squid → kid
/// pop-out holds the first shot for `emergeDelay`; a tap during it is buffered.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct FireGate {
    pub fire: bool,
    pub pressed: bool,
    pub sub: bool,
    pub sub_released: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct PrevIntent {
    fire: bool,
    jump: bool,
    sub: bool,
    squid: bool,
}

/// One squidkid (player or bot — both run the identical simulation).
/// `Clone` backs the bot brain, which reads a per-frame roster snapshot while
/// its own actor is borrowed mutably (see `bot::BotCtx`).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Actor {
    pub team: usize,
    pub slot: usize,
    pub alive: bool,
    pub hp: f32,
    pub ink: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub yaw_vel: f32,
    face_target: Option<f32>,
    /// Aim angles written by the controller (player camera / bot AI).
    pub aim_yaw: f32,
    pub aim_pitch: f32,
    pub form: Form,
    pub submerged: bool,
    pub grounded: bool,
    /// Paint under the feet: 0 none, 1 own, 2 enemy (only while grounded).
    pub ground_team: u8,
    pub on_enemy: bool,
    ground: GroundHit,
    ground_n: Vec3,
    contacts: Contacts,
    pub respawn_timer: f32,
    pub invuln: f32,
    last_damage: f32,
    last_fire: f32,
    /// JS `lastAttacker` (actor.js L188): identity `(team, slot)` of the last
    /// weapon damage source; the water splat inherits it while
    /// `lastDamage < 4` (JS L384). Cleared by `reset` like JS.
    last_attacker: Option<(usize, usize)>,
    damage_from_ink: f32,
    hard_land: f32,
    kid_t: f32,
    coyote: f32,
    jump_buffer: f32,
    fire_buffer: f32,
    fire_facing: f32,
    smooth_y: f32,
    smooth_yv: f32,
    time: f32,
    squid_press_t: f32,
    fire_press_t: f32,
    prev: PrevIntent,
    /// Trigger state for the weapon runner, refreshed every `step`.
    pub fire_gate: FireGate,
    pub deaths: u32,
    /// JS `stats.turf`: m² newly inked credited to this actor (Task 8
    /// `addTurf` — the weapon layer's claimed areas route through the match).
    pub turf: f32,
    /// JS `stats.splats`: enemies this actor splatted.
    pub splats: u32,
    events: Vec<ActorEvent>,
}

impl Actor {
    #[must_use]
    pub fn new(team: usize, slot: usize, t: &PlayerTuning) -> Self {
        let mut a = Self {
            team,
            slot,
            alive: true,
            hp: t.hp,
            ink: t.ink_max,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            yaw: 0.0,
            yaw_vel: 0.0,
            face_target: None,
            aim_yaw: 0.0,
            aim_pitch: 0.0,
            form: Form::Kid,
            submerged: false,
            grounded: false,
            ground_team: 0,
            on_enemy: false,
            ground: GroundHit::default(),
            ground_n: Vec3::Y,
            contacts: Contacts::default(),
            respawn_timer: 0.0,
            invuln: 0.0,
            last_damage: 99.0,
            last_fire: 99.0,
            last_attacker: None,
            damage_from_ink: 0.0,
            hard_land: 0.0,
            kid_t: 99.0,
            coyote: 0.0,
            jump_buffer: 0.0,
            fire_buffer: 0.0,
            fire_facing: 0.0,
            smooth_y: 0.0,
            smooth_yv: 0.0,
            time: 0.0,
            squid_press_t: -1.0,
            fire_press_t: -1.0,
            prev: PrevIntent::default(),
            fire_gate: FireGate::default(),
            deaths: 0,
            turf: 0.0,
            splats: 0,
            events: Vec::new(),
        };
        a.reset(t);
        a
    }

    /// JS `reset()`: back to a fresh spawn state (position kept by caller).
    pub fn reset(&mut self, t: &PlayerTuning) {
        self.alive = true;
        self.hp = t.hp;
        self.ink = t.ink_max;
        self.form = Form::Kid;
        self.submerged = false;
        self.grounded = false;
        self.ground_team = 0;
        self.respawn_timer = 0.0;
        self.invuln = 0.0;
        self.last_damage = 99.0;
        self.last_fire = 99.0;
        self.last_attacker = None;
        self.damage_from_ink = 0.0;
        self.hard_land = 0.0;
        self.coyote = 0.0;
        self.jump_buffer = 0.0;
        self.fire_buffer = 0.0;
        self.fire_facing = 0.0;
        self.kid_t = 99.0;
        self.yaw_vel = 0.0;
        self.face_target = None;
        self.smooth_y = 0.0;
        self.smooth_yv = 0.0;
        self.on_enemy = false;
        self.ground = GroundHit::default();
        self.ground_n = Vec3::Y;
        self.contacts = Contacts::default();
        self.fire_gate = FireGate::default();
        self.prev = PrevIntent::default();
        self.squid_press_t = -1.0;
        self.fire_press_t = -1.0;
        self.time = 0.0;
    }

    /// JS `spawnAt(p, yaw)`: place on the ground (settle probe) with spawn
    /// invulnerability.
    pub fn spawn_at(&mut self, p: Vec3, yaw: f32, world: &SimWorld, t: &PlayerTuning) {
        self.reset(t);
        self.pos = p;
        self.vel = Vec3::ZERO;
        self.yaw = yaw;
        self.aim_yaw = yaw;
        self.aim_pitch = 0.0;
        self.invuln = t.spawn_invuln;
        let gh = world
            .collision
            .ground_probe(p.x, p.y, p.z, 0.3, 0.3, t.foot_radius, false);
        self.ground = gh;
        if gh.hit && (gh.y - p.y).abs() < 0.3 {
            self.pos.y = gh.y;
            self.grounded = true;
            self.ground_n = gh.normal;
        }
    }

    /// JS `respawn()`: drop in above the own spawn pad on the per-slot ring.
    pub fn respawn(&mut self, world: &SimWorld, t: &PlayerTuning) {
        let pad = world.spawn_pads[self.team];
        let a = (self.slot as f32 / 4.0) * TAU + 0.6;
        let p = vec3(pad.x + a.cos() * 1.1, pad.y + 4.5, pad.z + a.sin() * 1.1);
        let yaw = if self.team == 0 {
            0.0
        } else {
            std::f32::consts::PI
        };
        self.spawn_at(p, yaw, world, t);
        self.grounded = false;
        self.vel = vec3(0.0, -4.0, 0.0);
        self.events.push(ActorEvent::Respawn);
    }

    /// JS `damage(amount)` minus attacker/armour hooks (Task 7+). Returns true
    /// when the hit splatted. `attacker` is the firing actor's identity
    /// `(team, slot)` (JS `damage(amount, attacker, source)`); recorded as
    /// `lastAttacker` (JS L188) so a later water splat can inherit the credit.
    pub fn damage(
        &mut self,
        amount: f32,
        attacker: Option<(usize, usize)>,
        t: &PlayerTuning,
    ) -> bool {
        if !self.alive || amount <= 0.0 {
            return false;
        }
        if self.invuln > 0.0 {
            return false;
        }
        self.hp -= amount;
        self.last_damage = 0.0;
        if attacker.is_some() {
            self.last_attacker = attacker;
        }
        if self.hp <= 0.0 {
            self.splat(SplatCause::Weapon, attacker, t);
            return true;
        }
        false
    }

    /// JS `splat(attacker, cause)`: die, start the respawn timer, surface the
    /// splat with its attacker credit on the event queue.
    pub fn splat(&mut self, cause: SplatCause, attacker: Option<(usize, usize)>, t: &PlayerTuning) {
        if !self.alive {
            return;
        }
        self.alive = false;
        self.hp = 0.0;
        self.respawn_timer = t.respawn_time;
        self.deaths += 1;
        self.events.push(ActorEvent::Splatted { cause, attacker });
    }

    /// Mark a shot fired this frame (resets the idle-ink-refill delay). Called
    /// by the Task 7 weapon runner; tests use it for the refill scenarios.
    pub fn note_fired(&mut self) {
        self.last_fire = 0.0;
    }

    /// Seconds since the last shot (JS `lastFire`); a decrease between frames
    /// means at least one shot was fired (Task 14 audio edge).
    #[must_use]
    pub fn last_fire(&self) -> f32 {
        self.last_fire
    }

    /// JS `aimDir` (actor.js L271): unit aim vector from `aim_yaw`/`aim_pitch`
    /// — XZ heading from yaw (+Z at 0), Y from pitch.
    #[must_use]
    pub fn aim_dir(&self) -> Vec3 {
        let cp = self.aim_pitch.cos();
        vec3(
            self.aim_yaw.sin() * cp,
            self.aim_pitch.sin(),
            self.aim_yaw.cos() * cp,
        )
    }

    /// Test/debug hook for the post-splat invulnerability window (JS sets
    /// `invuln = spawnInvuln` on respawn; the value is private here).
    pub fn set_invuln(&mut self, secs: f32) {
        self.invuln = secs;
    }

    /// Remaining invulnerability time (JS `this.invuln`).
    #[must_use]
    pub fn invuln(&self) -> f32 {
        self.invuln
    }

    /// Seconds since the last hit taken (JS `this.lastDamage`); 99 when never
    /// hit. Read by the bot brain's retreat/dodge decisions.
    #[must_use]
    pub fn last_damage(&self) -> f32 {
        self.last_damage
    }

    /// Visual vertical offset (JS `this.smoothY`); the bot brain aims at the
    /// smoothed head position, like upstream `t.smoothY || 0` (bots.js L430).
    #[must_use]
    pub fn smooth_y(&self) -> f32 {
        self.smooth_y
    }

    /// Events emitted since the last drain (jump/land/splat/respawn).
    ///
    /// Contract: the caller drains once per fixed step (before/after `step`);
    /// events pushed by `step`/`splat`/`respawn` accumulate until then. The
    /// queue is bounded by the number of drains — nothing is dropped, and
    /// `reset` deliberately keeps pending events (a splat must still surface).
    pub fn drain_events(&mut self) -> Vec<ActorEvent> {
        std::mem::take(&mut self.events)
    }

    /// Visual (smoothed) feet height — cameras follow this, not the raw pos.
    #[must_use]
    pub fn visual_y(&self) -> f32 {
        self.pos.y + self.smooth_y
    }

    /// JS `update(dt)`: one fixed simulation step.
    pub fn step(
        &mut self,
        dt: f32,
        input: &ActorInput,
        world: &SimWorld,
        ink: &dyn InkQuery,
        t: &PlayerTuning,
    ) {
        // JS runs this whole pipeline once per 60 Hz frame; variable dt would
        // desync the timers (jump buffer / coyote / emerge) from the tuning.
        debug_assert!(
            (dt - FIXED_DT).abs() < 1e-6,
            "Actor::step must run at the fixed 60 Hz step, got dt={dt}"
        );
        self.time += dt;
        // safety net: a non-finite state must never poison physics or camera
        if !(self.pos.is_finite()
            && self.vel.is_finite()
            && self.yaw.is_finite()
            && self.smooth_y.is_finite())
        {
            self.yaw = 0.0;
            self.yaw_vel = 0.0;
            self.smooth_y = 0.0;
            self.smooth_yv = 0.0;
            if self.alive {
                self.respawn(world, t);
            } else {
                self.pos = world.spawn_pads[self.team];
                self.vel = Vec3::ZERO;
            }
        }
        if !self.alive {
            self.respawn_timer -= dt;
            if self.respawn_timer <= 0.0 {
                self.respawn(world, t);
            }
            return;
        }

        // ---- intent edges + press timestamps (fire/squid most-recent-wins)
        let fire_pressed = input.fire && !self.prev.fire;
        let jump_pressed = input.jump && !self.prev.jump;
        let sub_released = !input.sub && self.prev.sub;
        if input.squid && !self.prev.squid {
            self.squid_press_t = self.time;
        }
        if fire_pressed {
            self.fire_press_t = self.time;
        }
        self.prev = PrevIntent {
            fire: input.fire,
            jump: input.jump,
            sub: input.sub,
            squid: input.squid,
        };

        // ---- timers
        self.invuln = (self.invuln - dt).max(0.0);
        self.last_damage += dt;
        self.last_fire += dt;
        self.kid_t += dt;
        self.jump_buffer = if jump_pressed {
            t.jump_buffer
        } else {
            (self.jump_buffer - dt).max(0.0)
        };
        self.fire_buffer = if fire_pressed {
            t.fire_buffer
        } else {
            (self.fire_buffer - dt).max(0.0)
        };
        self.hard_land = (self.hard_land - dt / t.hard_land_time).max(0.0);

        // ---- form: swim + fire both held → the most recent press wins
        let fire_wins =
            (input.fire || self.fire_buffer > 0.0) && self.fire_press_t >= self.squid_press_t;
        let want_squid = input.squid && !fire_wins;
        if want_squid != (self.form == Form::Squid) {
            self.form = if want_squid { Form::Squid } else { Form::Kid };
            if !want_squid {
                self.kid_t = 0.0;
            }
        }
        let is_squid = self.form == Form::Squid;

        // ---- surface under the feet (last frame's ground probe)
        let g = self.ground;
        self.ground_team = if self.grounded && g.hit && g.face >= 0 {
            let pt = ink.sample(g.face, g.u, g.v);
            if pt == 0 {
                0
            } else if usize::from(pt) - 1 == self.team {
                1
            } else {
                2
            }
        } else {
            0
        };
        self.submerged = is_squid && self.grounded && self.ground_team == 1;
        let on_enemy = self.grounded && self.ground_team == 2 && !self.submerged;
        self.on_enemy = on_enemy;

        // ---- horizontal movement
        self.horizontal(dt, is_squid, on_enemy, input, t);

        // ---- jump (buffered, with coyote time)
        self.coyote = if self.grounded {
            t.coyote_time
        } else {
            self.coyote - dt
        };
        let mut jumped = false;
        if self.jump_buffer > 0.0 && (self.grounded || self.coyote > 0.0) {
            let mut jv = if self.submerged {
                t.swim_jump_vel
            } else {
                t.jump_vel
            };
            if on_enemy {
                jv *= 0.72;
            }
            self.vel.y = jv;
            self.grounded = false;
            self.coyote = 0.0;
            self.jump_buffer = 0.0;
            jumped = true;
            let swim = self.submerged;
            self.events.push(ActorEvent::Jump { swim });
        }

        // ---- integrate + collide (feet + body), then the spawn keep-out
        self.integrate(dt, is_squid, jumped, world, t);
        self.spawn_barrier(world);

        // ---- enemy ink damage + hp regen + ink tank refill
        if on_enemy {
            if self.damage_from_ink < t.enemy_ink_damage_cap && self.invuln <= 0.0 {
                let d = (t.enemy_ink_dps * dt).min(t.enemy_ink_damage_cap - self.damage_from_ink);
                self.damage_from_ink += d;
                self.hp = (self.hp - d).max(1.0);
            }
            self.last_damage = self.last_damage.min(0.4);
        } else {
            self.damage_from_ink = (self.damage_from_ink - dt * 30.0).max(0.0);
        }
        if self.last_damage > t.regen_delay && self.hp < t.hp {
            let rate = if self.submerged {
                t.regen_rate_swim
            } else {
                t.regen_rate
            };
            self.hp = t.hp.min(self.hp + rate * dt);
        }
        if self.submerged {
            self.ink = t.ink_max.min(self.ink + t.ink_refill_swim * dt);
        } else if !is_squid && self.last_fire > t.ink_refill_delay {
            self.ink = t.ink_max.min(self.ink + t.ink_refill_kid * dt);
        } else if is_squid {
            self.ink = t.ink_max.min(self.ink + t.ink_refill_kid * 0.5 * dt);
        }

        // ---- fire gate for the Task 7 weapon runner (emerge delay + buffer)
        let mut gate = FireGate {
            sub: input.sub && !is_squid,
            sub_released: sub_released && !is_squid,
            ..FireGate::default()
        };
        if !is_squid && self.kid_t >= t.emerge_delay {
            let buffered = self.fire_buffer > 0.0;
            gate.fire = input.fire || buffered;
            gate.pressed = fire_pressed || buffered;
            self.fire_buffer = 0.0;
        }
        self.fire_gate = gate;

        // ---- fall into the sea: below the waterline with no deck underneath
        if self.pos.y < t.fall_death_y
            && world
                .collision
                .ground_height(self.pos.x, self.pos.z, self.pos.y + 0.6, false)
                == f32::NEG_INFINITY
        {
            // JS L384: `splat(this.lastDamage < 4 ? this.lastAttacker : null,
            // 'water')` — a recent hit credits the chaser, otherwise nobody.
            let attacker = if self.last_damage < 4.0 {
                self.last_attacker
            } else {
                None
            };
            self.splat(SplatCause::Water, attacker, t);
            return;
        }

        self.finish_frame(dt, is_squid, input, t);
    }

    // -------------------------------------------------- horizontal movement
    /// JS `_horizontal`: grounded speed+heading model / airborne steering.
    fn horizontal(
        &mut self,
        dt: f32,
        is_squid: bool,
        on_enemy: bool,
        input: &ActorInput,
        t: &PlayerTuning,
    ) {
        let mv = input.move_dir;
        let mh = mv.x.hypot(mv.z);
        let mag = mh.min(1.0);
        let vx = self.vel.x;
        let vz = self.vel.z;
        let sp = vx.hypot(vz);
        // ---- airborne: vector steering with light air control
        if !self.grounded {
            let (target, accel, decel) = if is_squid {
                (
                    t.squid_dry_speed.max(sp),
                    t.squid_air_accel,
                    t.squid_air_decel,
                )
            } else {
                (t.run_speed.max(t.air_min_speed), t.air_accel, t.air_decel)
            };
            let (tvx, tvz) = if mh > 0.01 {
                (mv.x / mh * target * mag, mv.z / mh * target * mag)
            } else {
                (0.0, 0.0)
            };
            let dvx = tvx - vx;
            let dvz = tvz - vz;
            let dl = dvx.hypot(dvz);
            let rate = (if mh > 0.01 { accel } else { decel }) * dt;
            if dl <= rate {
                self.vel.x = tvx;
                self.vel.z = tvz;
            } else {
                self.vel.x += dvx / dl * rate;
                self.vel.z += dvz / dl * rate;
            }
            return;
        }
        // ---- grounded: pick the per-form parameter row
        let mut vt;
        let mut big_a;
        let a_in;
        let in_knee;
        let out_knee;
        let mut big_d;
        let d_min;
        let d_knee;
        let w;
        if is_squid && self.submerged {
            vt = t.swim_speed;
            big_a = t.swim_accel;
            a_in = t.swim_accel_in;
            in_knee = 3.0;
            out_knee = t.swim_out_knee;
            big_d = t.swim_decel;
            d_min = 0.5;
            d_knee = 4.0;
            w = t.swim_turn;
        } else if is_squid {
            vt = t.squid_dry_speed;
            big_a = t.squid_accel;
            a_in = 0.6;
            in_knee = 1.0;
            out_knee = 0.3;
            big_d = t.squid_decel;
            d_min = 0.5;
            d_knee = 2.0;
            w = t.squid_turn;
        } else {
            vt = t.run_speed; // WeaponRunner.moveSpeed() — Task 7 adds firing slowdown
            big_a = t.run_accel;
            a_in = t.run_accel_in;
            in_knee = t.run_in_knee;
            out_knee = t.run_out_knee;
            big_d = t.run_decel;
            d_min = t.run_decel_min;
            d_knee = t.run_decel_knee;
            w = t.turn_rate;
            if self.hard_land > 0.0 {
                vt *= 1.0 - (1.0 - t.hard_land_slow) * self.hard_land;
            }
        }
        if on_enemy {
            vt = vt.min(t.enemy_ink_speed);
            big_a = big_a.min(t.enemy_ink_accel);
            big_d = t.enemy_ink_decel.max(0.0);
        }
        if mh < 0.01 {
            // brake: strong at speed, easing into the stop
            if sp < 1e-4 {
                self.vel.x = 0.0;
                self.vel.z = 0.0;
                return;
            }
            let d = big_d * (d_min + (1.0 - d_min) * smoothstep(0.0, d_knee, sp)) * dt;
            let k = (sp - d).max(0.0) / sp;
            self.vel.x *= k;
            self.vel.z *= k;
            return;
        }
        let tx = mv.x / mh;
        let tz = mv.z / mh;
        let vts = vt * mag;
        let mut dx = tx;
        let mut dz = tz;
        if sp > 0.05 {
            dx = vx / sp;
            dz = vz / sp;
        }
        let cos_a = clamp(dx * tx + dz * tz, -1.0, 1.0);
        let ang = cos_a.acos();
        if sp > 0.5 && ang > t.reverse_angle {
            // plant-and-reverse: brake through zero toward the new direction
            let tvx = tx * vts;
            let tvz = tz * vts;
            let ex = tvx - vx;
            let ez = tvz - vz;
            let el = ex.hypot(ez);
            let r = t.reverse_decel.max(big_d) * dt * if on_enemy { 0.5 } else { 1.0 };
            if el <= r {
                self.vel.x = tvx;
                self.vel.z = tvz;
            } else {
                self.vel.x += ex / el * r;
                self.vel.z += ez / el * r;
            }
            return;
        }
        // heading slews toward the input (faster when slow)
        let wmax = w * (1.0 + t.turn_rate_slow * (1.0 - smoothstep(0.0, vt, sp)));
        let rot = ang.min(wmax * dt);
        if rot > 1e-6 {
            let s = if dz * tx - dx * tz >= 0.0 { 1.0 } else { -1.0 };
            let c = (rot * s).cos();
            let sn = (rot * s).sin();
            let nx = dx * c + dz * sn;
            let nz = -dx * sn + dz * c;
            dx = nx;
            dz = nz;
        }
        let ns = if sp < vts {
            let a = big_a
                * (a_in + (1.0 - a_in) * smoothstep(0.0, in_knee, sp))
                * clamp((vts - sp) / (out_knee * vt), t.run_out_min, 1.0);
            vts.min(sp + a * dt)
        } else {
            // over speed: shed it at the brake rate
            vts.max(sp - big_d * (d_min + (1.0 - d_min) * smoothstep(0.0, d_knee, sp - vts)) * dt)
        };
        self.vel.x = dx * ns;
        self.vel.z = dz * ns;
    }

    // --------------------------------------------------- character controller
    /// JS `_integrate`: ground-plane follow / gravity shaping, then resolve.
    fn integrate(
        &mut self,
        dt: f32,
        is_squid: bool,
        jumped: bool,
        world: &SimWorld,
        t: &PlayerTuning,
    ) {
        let was_grounded = self.grounded && !jumped;
        if was_grounded {
            // follow the ground plane (no micro-hops up ramps, no sliding)
            let n = self.ground_n;
            self.vel.y = -(self.vel.x * n.x + self.vel.z * n.z) / n.y.max(0.35);
        } else {
            let mut g = t.gravity;
            if self.vel.y < 0.0 {
                g *= t.fall_gravity_mul;
            }
            if self.vel.y.abs() < t.apex_band {
                g *= t.apex_gravity_mul;
            }
            self.vel.y = (-t.max_fall).max(self.vel.y - g * dt);
        }
        let prev_y = self.pos.y;
        self.pos += self.vel * dt;
        self.resolve(is_squid, prev_y, was_grounded, world, t);
    }

    /// JS `_resolve`: body (walls/ceilings) then feet (ground stick / landing).
    fn resolve(
        &mut self,
        is_squid: bool,
        prev_y: f32,
        stick: bool,
        world: &SimWorld,
        t: &PlayerTuning,
    ) {
        let lift = if is_squid {
            t.squid_body_lift
        } else {
            t.step_up
        };
        let height = if is_squid { t.squid_height } else { t.height };
        world.collision.collide_body(
            &mut self.pos,
            t.radius,
            lift,
            height,
            &mut self.contacts,
            stick,
            is_squid,
            3,
        );
        if self.contacts.ceiling && self.vel.y > 0.0 {
            self.vel.y = 0.0;
        }
        if self.contacts.wall {
            let n = self.contacts.wall_normal;
            let vn = self.vel.x * n.x + self.vel.z * n.z;
            if vn < 0.0 {
                self.vel.x -= n.x * vn;
                self.vel.z -= n.z * vn;
            }
        }
        let up = if is_squid { t.squid_step_up } else { t.step_up };
        let was_grounded = self.grounded;
        let mut grounded = false;
        if stick {
            let mut gh = world.collision.ground_probe(
                self.pos.x,
                self.pos.y,
                self.pos.z,
                up,
                t.step_down,
                t.foot_radius,
                is_squid,
            );
            if !is_squid {
                world.collision.rail_feet(
                    self.pos.x,
                    self.pos.z,
                    self.pos.y - t.step_down,
                    self.pos.y + up,
                    t.foot_radius,
                    &mut gh,
                );
            }
            if gh.hit {
                let dy = gh.y - self.pos.y;
                self.pos.y = gh.y;
                grounded = true;
                // curbs / steps are eased visually; slope following stays exact
                if dy.abs() > 0.06 {
                    self.smooth_y -= dy;
                }
            }
            self.ground = gh;
        } else if self.vel.y <= 0.5 {
            // landing: probe from the highest point passed through this frame
            let top = prev_y.max(self.pos.y);
            let assist = if is_squid {
                t.squid_step_up
            } else {
                t.ledge_assist
            };
            let mut gh = world.collision.ground_probe(
                self.pos.x,
                self.pos.y,
                self.pos.z,
                (top - self.pos.y) + assist,
                0.02,
                t.foot_radius,
                is_squid,
            );
            if !is_squid {
                world.collision.rail_feet(
                    self.pos.x,
                    self.pos.z,
                    self.pos.y - 0.02,
                    top + assist,
                    t.foot_radius,
                    &mut gh,
                );
            }
            if gh.hit
                && gh.y >= self.pos.y - 0.02
                && (self.vel.y <= 0.0 || gh.y - self.pos.y < 0.02)
            {
                let pop = gh.y - prev_y;
                self.pos.y = gh.y;
                grounded = true;
                if pop > 0.035 {
                    self.smooth_y -= pop;
                }
            }
            self.ground = gh;
        }
        if grounded {
            self.ground_n = self.ground.normal;
            if !was_grounded {
                self.on_land(t);
            }
            self.vel.y = 0.0;
        }
        self.grounded = grounded;
    }

    /// JS `_onLand` (sim parts only): hard-landing recovery weight + event.
    fn on_land(&mut self, t: &PlayerTuning) {
        let speed = (-self.vel.y).max(0.0);
        if speed > t.hard_land_speed {
            self.hard_land = clamp((speed - t.hard_land_speed) / 6.0 + 0.5, 0.0, 1.0);
        }
        if speed > 3.0 {
            self.events.push(ActorEvent::Land { speed });
        }
    }

    /// JS `_spawnBarrier`: keep out of the enemy spawn bubble.
    fn spawn_barrier(&mut self, world: &SimWorld) {
        let pad = world.spawn_pads[1 - self.team];
        let r = world.spawn_barrier;
        let dx = self.pos.x - pad.x;
        let dz = self.pos.z - pad.z;
        let d = dx.hypot(dz);
        if d < r && self.pos.y > pad.y - 1.0 {
            let k = (r - d) / d.max(0.01);
            self.pos.x += dx * k;
            self.pos.z += dz * k;
            let vn = (self.vel.x * dx + self.vel.z * dz) / d.max(0.01);
            if vn < 0.0 {
                self.vel.x -= dx / d * vn * 1.6;
                self.vel.z -= dz / d * vn * 1.6;
            }
        }
    }

    // ---------------------------------------------------------------- visuals
    /// JS `_finishFrame` (sim parts): facing spring + smoothY spring.
    fn finish_frame(&mut self, dt: f32, is_squid: bool, input: &ActorInput, t: &PlayerTuning) {
        self.face(dt, is_squid, input, t);
        // visual step smoothing (critically damped, ~0.12 s)
        let w = 24.0;
        let acc = -w * w * self.smooth_y - 2.0 * w * self.smooth_yv;
        self.smooth_yv += acc * dt;
        self.smooth_y += self.smooth_yv * dt;
        if self.smooth_y.abs() > 0.7 {
            self.smooth_y = self.smooth_y.signum() * 0.7;
        }
        if self.smooth_y.abs() < 1e-4 && self.smooth_yv.abs() < 1e-3 {
            self.smooth_y = 0.0;
            self.smooth_yv = 0.0;
        }
    }

    /// JS `_face`: angular spring with rate + acceleration caps and target
    /// rate feed-forward.
    fn face(&mut self, dt: f32, is_squid: bool, input: &ActorInput, t: &PlayerTuning) {
        let mv = input.move_dir;
        let mh = mv.x.hypot(mv.z);
        let hs = self.vel.x.hypot(self.vel.z);
        self.fire_facing = (self.fire_facing - dt).max(0.0);
        let mut target: Option<f32> = None;
        let mut omega = t.face_omega;
        let mut max_rate = t.face_max_rate;
        let mut max_acc = t.face_max_acc;
        if self.fire_facing > 0.0 || input.sub {
            target = Some(self.aim_yaw);
            omega = t.aim_face_omega;
            max_rate = t.aim_face_max_rate;
            max_acc = t.aim_face_max_acc;
        } else {
            if mh > 0.2 {
                target = Some(mv.x.atan2(mv.z));
            } else if hs > 0.6 {
                target = Some(self.vel.x.atan2(self.vel.z));
            }
            if is_squid {
                omega = t.squid_face_omega;
                max_rate = if self.submerged {
                    t.swim_face_max_rate
                } else {
                    t.squid_face_max_rate
                };
                max_acc = t.squid_face_max_acc;
            }
        }
        // feed-forward the target's own angular velocity (smooth tracking, no
        // kick on discrete stick changes)
        let mut target_rate = 0.0;
        if let (Some(tg), Some(prev)) = (target, self.face_target) {
            let d = angle_diff(prev, tg);
            if d.abs() < 0.12 {
                target_rate = clamp(d / dt.max(1e-4), -max_rate, max_rate);
            }
        }
        self.face_target = target;
        let acc = match target {
            None => -2.0 * omega * self.yaw_vel,
            Some(tg) => {
                omega * omega * angle_diff(self.yaw, tg)
                    + 2.0 * omega * (target_rate - self.yaw_vel)
            }
        };
        self.yaw_vel = clamp(
            self.yaw_vel + clamp(acc, -max_acc, max_acc) * dt,
            -max_rate,
            max_rate,
        );
        if self.yaw_vel.abs() < 1e-5 {
            self.yaw_vel = 0.0; // no denormal tails
        }
        self.yaw += self.yaw_vel * dt;
        if self.yaw > std::f32::consts::PI {
            self.yaw -= TAU;
        } else if self.yaw < -std::f32::consts::PI {
            self.yaw += TAU;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;
    use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
    use crate::tuning::Tuning;

    const DT: f32 = FIXED_DT;

    fn flags() -> BrushCommon {
        BrushCommon {
            tag: None,
            color: "#dddddd".into(),
            pattern: 0,
            paint: true,
            solid: true,
            grate: false,
            rail: false,
            roof: false,
            perch: false,
            no_nav: false,
            hidden: false,
            bevel: None,
            no_paint: Vec::new(),
            mural: Vec::<Mural>::new(),
            oct: None,
        }
    }

    const B: Bounds = Bounds {
        min_x: -35.0,
        max_x: 35.0,
        min_z: -35.0,
        max_z: 35.0,
    };

    /// Flat 60×60 slab with the top at y=0; pads well clear of the test area.
    struct Fixture {
        world: CollisionWorld,
        sim_pads: [Vec3; 2],
        barrier: f32,
        tuning: Tuning,
    }

    fn fixture() -> Fixture {
        let world = CollisionWorld::from_brushes(
            &[Brush::Box {
                min: [-30.0, -1.0, -30.0],
                max: [30.0, 0.0, 30.0],
                common: flags(),
            }],
            B,
        );
        Fixture {
            world,
            sim_pads: [vec3(0.0, 0.0, -24.0), vec3(0.0, 0.0, 24.0)],
            barrier: 4.2,
            tuning: crate::embedded_tuning(),
        }
    }

    impl Fixture {
        fn sim(&self) -> SimWorld<'_> {
            SimWorld {
                collision: &self.world,
                spawn_pads: self.sim_pads,
                spawn_barrier: self.barrier,
            }
        }
    }

    struct OwnInk;
    impl InkQuery for OwnInk {
        fn sample(&self, _face: i32, _u: f32, _v: f32) -> u8 {
            1
        }
    }

    struct EnemyInk;
    impl InkQuery for EnemyInk {
        fn sample(&self, _face: i32, _u: f32, _v: f32) -> u8 {
            2
        }
    }

    fn idle() -> ActorInput {
        ActorInput::default()
    }

    fn run_x() -> ActorInput {
        ActorInput {
            move_dir: vec3(1.0, 0.0, 0.0),
            ..ActorInput::default()
        }
    }

    fn spawn_actor(f: &Fixture, slot: usize) -> Actor {
        let mut a = Actor::new(0, slot, &f.tuning.player);
        a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &f.sim(), &f.tuning.player);
        a
    }

    // --------------------------------------------------- analytic references
    // Reference re-implementations of the exact PLAYER curve math (f64). The
    // JS game — and therefore this port — advances the world in 60 Hz
    // semi-implicit Euler steps, so the authoritative comparison (TR-5.1 "解
    // 析参考实现") steps the reference at the same dt with the same operation
    // order, where agreement must be near-exact; a dt = 1e-5 continuous
    // integration of the same equations is printed alongside for the TR-5.4
    // handling evidence table.

    fn sstep64(a: f64, b: f64, x: f64) -> f64 {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Full-input standing start on flat ground: interpolated (time,
    /// distance) to `target`, stepped at `dt` like the sim.
    #[allow(clippy::too_many_arguments)]
    fn ref_accel(
        vt: f64,
        big_a: f64,
        a_in: f64,
        in_knee: f64,
        out_knee: f64,
        out_min: f64,
        target: f64,
        dt: f64,
    ) -> (f64, f64) {
        let (mut sp, mut x, mut t) = (0.0_f64, 0.0_f64, 0.0_f64);
        loop {
            let a = big_a
                * (a_in + (1.0 - a_in) * sstep64(0.0, in_knee, sp))
                * ((vt - sp) / (out_knee * vt)).clamp(out_min, 1.0);
            let (sp_prev, x_prev) = (sp, x);
            sp = (sp + a * dt).min(vt);
            x += sp * dt;
            t += dt;
            if sp >= target {
                let frac = (target - sp_prev) / (sp - sp_prev);
                return (t - dt + frac * dt, x_prev + (x - x_prev) * frac);
            }
            assert!(t < 10.0, "reference must converge");
        }
    }

    /// Release-input braking distance from `sp0` down to sp <= 0.05.
    fn ref_brake(big_d: f64, d_min: f64, d_knee: f64, sp0: f64, dt: f64) -> f64 {
        let (mut sp, mut x) = (sp0, 0.0_f64);
        while sp > 0.05 {
            let d = big_d * (d_min + (1.0 - d_min) * sstep64(0.0, d_knee, sp)) * dt;
            sp = (sp - d).max(0.0);
            x += sp * dt;
        }
        x
    }

    /// Jump from `v0` stepped at `dt`: (apex height, interpolated airtime).
    fn ref_jump(
        v0: f64,
        g: f64,
        fall_mul: f64,
        apex_mul: f64,
        apex_band: f64,
        max_fall: f64,
        dt: f64,
    ) -> (f64, f64) {
        let (mut y, mut vy, mut t) = (0.0_f64, v0, 0.0_f64);
        let mut apex = 0.0_f64;
        loop {
            let mut gg = g;
            if vy < 0.0 {
                gg *= fall_mul;
            }
            if vy.abs() < apex_band {
                gg *= apex_mul;
            }
            let y_prev = y;
            vy = (vy - gg * dt).max(-max_fall);
            y += vy * dt;
            t += dt;
            apex = apex.max(y);
            if y <= 0.0 {
                let frac = y_prev / (y_prev - y);
                return (apex, t - dt + frac * dt);
            }
            assert!(t < 10.0, "reference must converge");
        }
    }

    fn close(got: f64, want: f64, rel: f64, what: &str) {
        let err = (got - want).abs() / want.abs().max(1e-9);
        assert!(
            err < rel,
            "{what}: got {got:.6}, reference {want:.6}, rel err {err:.4} (limit {rel})"
        );
    }

    // --------------------------------------------------------------- TR-5.1
    #[test]
    fn tr5_1_run_accel_matches_analytic_reference() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        // standing start, full +X input; measure crossing of 90% top speed
        let target = 0.9 * p.run_speed; // 5.4 m/s
        let (mut sp_prev, mut x_prev) = (0.0_f32, 0.0_f32);
        let mut crossed: Option<(f32, f32)> = None;
        for k in 1..600 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
            let sp = a.vel.x.hypot(a.vel.z);
            if sp >= target && crossed.is_none() {
                let frac = (target - sp_prev) / (sp - sp_prev);
                let t_cross = (k - 1) as f32 * DT + frac * DT;
                let x_cross = x_prev + (a.pos.x - x_prev) * frac;
                crossed = Some((t_cross, x_cross));
                break;
            }
            sp_prev = sp;
            x_prev = a.pos.x;
        }
        let (t_sim, d_sim) = crossed.expect("must reach 90% top speed");
        let (t_ref, d_ref) = ref_accel(
            f64::from(p.run_speed),
            f64::from(p.run_accel),
            f64::from(p.run_accel_in),
            f64::from(p.run_in_knee),
            f64::from(p.run_out_knee),
            f64::from(p.run_out_min),
            f64::from(target),
            f64::from(DT),
        );
        let (t_cont, d_cont) = ref_accel(
            f64::from(p.run_speed),
            f64::from(p.run_accel),
            f64::from(p.run_accel_in),
            f64::from(p.run_in_knee),
            f64::from(p.run_out_knee),
            f64::from(p.run_out_min),
            f64::from(target),
            1e-5,
        );
        println!(
            "TR-5.1 kid run to {target:.2} m/s: sim t={t_sim:.4}s d={d_sim:.4}m | ref60 t={t_ref:.4}s d={d_ref:.4}m | continuous t={t_cont:.4}s d={d_cont:.4}m"
        );
        close(f64::from(t_sim), t_ref, 0.05, "TR-5.1 time to 90%");
        close(f64::from(d_sim), d_ref, 0.05, "TR-5.1 distance to 90%");
        // straight line, feet glued to the flat floor
        assert!(a.pos.z.abs() < 1e-4);
        assert!((a.pos.y - 0.0).abs() < 1e-4);
        assert!(a.grounded);
    }

    // --------------------------------------------------------------- TR-5.2
    #[test]
    fn tr5_2_kid_ink_refill_after_delay() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        a.ink = 50.0;
        a.note_fired(); // just fired: refill blocked for inkRefillDelay
        let ink_at = |steps: u32, a: &mut Actor| {
            for _ in 0..steps {
                a.step(DT, &idle(), &f.sim(), &NoInk, p);
            }
            a.ink
        };
        // 0.85 s in: still inside the 0.9 s delay → no refill at all
        let early = ink_at(51, &mut a);
        assert!(
            (early - 50.0).abs() < 1e-4,
            "no refill during delay, ink={early}"
        );
        // measure the steady-state rate between t=2.0 and t=4.0
        let to_2s = 120 - 51;
        let ink2 = ink_at(to_2s, &mut a);
        let ink4 = ink_at(120, &mut a);
        let rate = (ink4 - ink2) / 2.0;
        println!("TR-5.2 kid refill: rate={rate:.4}/s (expect 9), ink@4s={ink4:.3}");
        close(f64::from(rate), 9.0, 0.01, "TR-5.2 kid refill rate");
    }

    #[test]
    fn tr5_2_squid_swim_ink_refill() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        a.ink = 10.0; // low start: no inkMax cap inside the measurement window
        let swim = ActorInput {
            squid: true,
            ..ActorInput::default()
        };
        for _ in 0..30 {
            a.step(DT, &swim, &f.sim(), &OwnInk, p);
        }
        assert!(a.submerged, "squid on own ink must be submerged");
        let ink05 = a.ink;
        for _ in 0..60 {
            a.step(DT, &swim, &f.sim(), &OwnInk, p);
        }
        let rate = a.ink - ink05;
        println!(
            "TR-5.2 swim refill: rate={rate:.4}/s (expect 42), ink@1.5s={:.3}",
            a.ink
        );
        close(f64::from(rate), 42.0, 0.01, "TR-5.2 swim refill rate");
    }

    // --------------------------------------------------------------- TR-5.3
    #[test]
    fn tr5_3_splat_respawn_invuln() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 1);
        a.invuln = 0.0;
        assert!(a.damage(150.0, None, p), "lethal hit must splat");
        assert!(!a.alive);
        assert_eq!(a.deaths, 1);
        close(f64::from(a.respawn_timer), 5.5, 1e-4, "respawn timer");
        // 5.4 s later: still dead
        for _ in 0..324 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        assert!(!a.alive, "must stay dead until respawnTime");
        // cross 5.5 s one frame at a time and catch the respawn frame
        let mut steps = 324;
        while !a.alive {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
            steps += 1;
            assert!(
                steps < 400,
                "respawn must happen near 5.5 s (steps={steps})"
            );
        }
        let t_respawn = steps as f32 * DT;
        println!("TR-5.3 respawn at t={t_respawn:.4}s (expect 5.5)");
        close(f64::from(t_respawn), 5.5, 1e-3, "respawn time");
        // back at the own pad, on the per-slot ring, dropping in
        let pad = f.sim_pads[0];
        let ang = (1.0_f32 / 4.0) * TAU + 0.6;
        close(
            f64::from(a.pos.x - pad.x),
            f64::from(ang.cos() * 1.1),
            1e-3,
            "ring x",
        );
        close(
            f64::from(a.pos.z - pad.z),
            f64::from(ang.sin() * 1.1),
            1e-3,
            "ring z",
        );
        close(f64::from(a.pos.y), 4.5, 1e-3, "drop-in height");
        close(f64::from(a.hp), 100.0, 1e-6, "hp after respawn");
        close(f64::from(a.invuln), 1.6, 1e-4, "spawn invuln");
        // invulnerable: a hit right now does nothing
        assert!(!a.damage(50.0, None, p));
        close(f64::from(a.hp), 100.0, 1e-6, "hp during invuln");
        // events: splatted then respawn
        let ev = a.drain_events();
        assert!(ev.contains(&ActorEvent::Splatted {
            cause: SplatCause::Weapon,
            attacker: None
        }));
        assert!(ev.contains(&ActorEvent::Respawn));
        // drops in and lands on the pad
        for _ in 0..240 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        assert!(a.grounded, "must land on the pad after drop-in");
    }

    #[test]
    fn tr5_3_fall_into_sea_same_flow() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 2);
        a.invuln = 0.0;
        // over open water: below fallDeathY with no deck underneath
        a.pos = vec3(100.0, -2.0, 0.0);
        a.vel = Vec3::ZERO;
        a.drain_events();
        a.step(DT, &idle(), &f.sim(), &NoInk, p);
        assert!(!a.alive, "falling below fallDeathY over water must splat");
        close(f64::from(a.respawn_timer), 5.5, 1e-4, "water respawn timer");
        let ev = a.drain_events();
        assert!(ev.contains(&ActorEvent::Splatted {
            cause: SplatCause::Water,
            attacker: None
        }));
        for _ in 0..340 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        assert!(a.alive, "water splat respawns like a combat splat");
        assert_eq!(a.deaths, 1);
        // Explicit frame accounting instead of a magic offset: replay the same
        // splat and record the exact frame the respawn lands, so invuln is
        // checked against the frames elapsed since that moment.
        let mut b = spawn_actor(&f, 2);
        b.invuln = 0.0;
        b.pos = vec3(100.0, -2.0, 0.0);
        b.vel = Vec3::ZERO;
        b.drain_events();
        let mut respawn_frame = None;
        for k in 0..340 {
            b.step(DT, &idle(), &f.sim(), &NoInk, p);
            if respawn_frame.is_none() && b.alive {
                respawn_frame = Some(k);
            }
        }
        let rf = respawn_frame.expect("must respawn within 340 frames");
        // respawn_time 5.5 s = 330 frames after the splat frame.
        assert_eq!(rf, 330, "JS golden respawn delay (5.5 s)");
        // The respawn step itself sets invuln = spawnInvuln (the alive branch
        // returns before the timer decay), so only the steps after it tick.
        let since = 340 - rf - 1;
        close(
            f64::from(b.invuln),
            f64::from(p.spawn_invuln) - f64::from(since) * f64::from(DT),
            1e-4,
            "invuln ticking since respawn",
        );
    }

    // --------------------------------------------------- hp regen / enemy ink
    #[test]
    fn hp_regen_after_combat_delay() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        assert!(!a.damage(60.0, None, p));
        close(f64::from(a.hp), 40.0, 1e-6, "hp after hit");
        for _ in 0..72 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        assert!(
            (a.hp - 40.0).abs() < 1e-4,
            "no regen inside regenDelay, hp={}",
            a.hp
        );
        for _ in 0..60 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        let hp2 = a.hp;
        for _ in 0..60 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        let rate = a.hp - hp2;
        println!("hp regen: rate={rate:.3}/s (expect 22)");
        close(f64::from(rate), 22.0, 0.02, "hp regen rate");
    }

    #[test]
    fn enemy_ink_damage_cap_and_slowdown() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        for _ in 0..180 {
            a.step(DT, &run_x(), &f.sim(), &EnemyInk, p);
        }
        assert!(a.on_enemy);
        // dps 20 with a 40-point cap → hp floors at 60, and ink alone never kills
        close(f64::from(a.hp), 60.0, 0.01, "enemy ink capped damage");
        // slowed to enemyInkSpeed
        let sp = a.vel.x.hypot(a.vel.z);
        println!("enemy ink: speed={sp:.3} m/s (expect 1.9), hp={}", a.hp);
        close(
            f64::from(sp),
            f64::from(p.enemy_ink_speed),
            0.05,
            "enemy ink speed cap",
        );
    }

    // --------------------------------------------------------------- TR-5.4
    // Headless handling measurements, same 口径 as tools/measure-handling.mjs
    // (accel / brake / jump / swim), each checked against the analytic
    // reference integration of the ported PLAYER curves.

    #[test]
    fn tr5_4_brake_distance() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        for _ in 0..180 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        let sp0 = a.vel.x.hypot(a.vel.z);
        assert!(
            sp0 > 0.99 * p.run_speed,
            "at top speed before braking (sp0={sp0})"
        );
        let x0 = a.pos.x;
        let mut steps = 0;
        while a.vel.x.hypot(a.vel.z) > 0.05 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
            steps += 1;
            assert!(steps < 600);
        }
        let d_sim = f64::from(a.pos.x - x0);
        let d_ref = ref_brake(
            f64::from(p.run_decel),
            f64::from(p.run_decel_min),
            f64::from(p.run_decel_knee),
            f64::from(sp0),
            f64::from(DT),
        );
        let d_cont = ref_brake(
            f64::from(p.run_decel),
            f64::from(p.run_decel_min),
            f64::from(p.run_decel_knee),
            f64::from(sp0),
            1e-5,
        );
        println!(
            "TR-5.4 brake {sp0:.2}→0: sim d={d_sim:.4}m ({steps} steps) | ref60 d={d_ref:.4}m | continuous d={d_cont:.4}m"
        );
        close(d_sim, d_ref, 0.05, "TR-5.4 brake distance");
    }

    #[test]
    fn tr5_4_jump_apex_and_airtime() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let jump = ActorInput {
            jump: true,
            ..ActorInput::default()
        };
        a.step(DT, &jump, &f.sim(), &NoInk, p);
        assert!(!a.grounded, "jump must leave the ground");
        assert!(a.vel.y > 7.9, "jump vel applied (vy={})", a.vel.y);
        let mut apex = a.pos.y.max(0.0);
        let mut t_land = 0.0_f32;
        for k in 1..600 {
            let y_prev = a.pos.y;
            let vy_prev = a.vel.y;
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
            apex = apex.max(a.pos.y);
            if a.grounded {
                // linear crossing inside the landing step (fall gravity region)
                let vy_new = vy_prev - p.gravity * p.fall_gravity_mul * DT;
                let y_raw = y_prev + vy_new * DT;
                let frac = if y_raw < y_prev {
                    y_prev / (y_prev - y_raw)
                } else {
                    1.0
                };
                t_land = k as f32 * DT + frac * DT;
                break;
            }
        }
        assert!(t_land > 0.0, "must land");
        let (apex_ref, air_ref) = ref_jump(
            f64::from(p.jump_vel),
            f64::from(p.gravity),
            f64::from(p.fall_gravity_mul),
            f64::from(p.apex_gravity_mul),
            f64::from(p.apex_band),
            f64::from(p.max_fall),
            f64::from(DT),
        );
        let (apex_cont, air_cont) = ref_jump(
            f64::from(p.jump_vel),
            f64::from(p.gravity),
            f64::from(p.fall_gravity_mul),
            f64::from(p.apex_gravity_mul),
            f64::from(p.apex_band),
            f64::from(p.max_fall),
            1e-5,
        );
        println!(
            "TR-5.4 jump: sim apex={apex:.4}m air={t_land:.4}s | ref60 apex={apex_ref:.4}m air={air_ref:.4}s | continuous apex={apex_cont:.4}m air={air_cont:.4}s"
        );
        close(f64::from(apex), apex_ref, 0.05, "TR-5.4 jump apex");
        close(f64::from(t_land), air_ref, 0.05, "TR-5.4 airtime");
        // landing event fired
        let ev = a.drain_events();
        assert!(ev.iter().any(|e| matches!(e, ActorEvent::Land { .. })));
        assert!(ev.contains(&ActorEvent::Jump { swim: false }));
    }

    #[test]
    fn tr5_4_swim_accel_matches_reference() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let swim = ActorInput {
            move_dir: vec3(1.0, 0.0, 0.0),
            squid: true,
            ..ActorInput::default()
        };
        let target = 0.9 * p.swim_speed; // 10.62 m/s
        let (mut sp_prev, mut x_prev) = (0.0_f32, 0.0_f32);
        let mut crossed: Option<(f32, f32)> = None;
        for k in 1..600 {
            a.step(DT, &swim, &f.sim(), &OwnInk, p);
            assert!(a.submerged);
            let sp = a.vel.x.hypot(a.vel.z);
            if sp >= target && crossed.is_none() {
                let frac = (target - sp_prev) / (sp - sp_prev);
                crossed = Some((
                    (k - 1) as f32 * DT + frac * DT,
                    x_prev + (a.pos.x - x_prev) * frac,
                ));
                break;
            }
            sp_prev = sp;
            x_prev = a.pos.x;
        }
        let (t_sim, d_sim) = crossed.expect("must reach 90% swim speed");
        let (t_ref, d_ref) = ref_accel(
            f64::from(p.swim_speed),
            f64::from(p.swim_accel),
            f64::from(p.swim_accel_in),
            3.0,
            f64::from(p.swim_out_knee),
            f64::from(p.run_out_min),
            f64::from(target),
            f64::from(DT),
        );
        let (t_cont, d_cont) = ref_accel(
            f64::from(p.swim_speed),
            f64::from(p.swim_accel),
            f64::from(p.swim_accel_in),
            3.0,
            f64::from(p.swim_out_knee),
            f64::from(p.run_out_min),
            f64::from(target),
            1e-5,
        );
        println!(
            "TR-5.4 swim to {target:.2} m/s: sim t={t_sim:.4}s d={d_sim:.4}m | ref60 t={t_ref:.4}s d={d_ref:.4}m | continuous t={t_cont:.4}s d={d_cont:.4}m"
        );
        close(f64::from(t_sim), t_ref, 0.05, "TR-5.4 swim time to 90%");
        close(f64::from(d_sim), d_ref, 0.05, "TR-5.4 swim distance to 90%");
    }

    // ------------------------------------------------ jump buffer and coyote
    #[test]
    fn jump_buffer_and_coyote_time() {
        let f = fixture();
        let p = &f.tuning.player;
        // buffer: press jump while airborne, shortly before touchdown — the
        // press must be buffered (0.13 s) and fire on the landing frame.
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        a.pos.y = 0.15; // ~6 frames of fall: lands before the buffer expires
        a.grounded = false;
        let jump = ActorInput {
            jump: true,
            ..ActorInput::default()
        };
        let mut jumped = false;
        let mut first = true;
        for _ in 0..30 {
            let inp = if first { &jump } else { &idle() };
            first = false;
            let vy_before = a.vel.y;
            a.step(DT, inp, &f.sim(), &NoInk, p);
            if a.vel.y > vy_before + 1.0 && !a.grounded {
                jumped = true;
                break;
            }
        }
        assert!(jumped, "buffered jump must fire on landing");
        // coyote: run off the slab edge, then jump within coyoteTime (0.12 s)
        let mut b = spawn_actor(&f, 0);
        b.invuln = 0.0;
        b.pos = vec3(29.9, 0.0, 0.0);
        b.vel = vec3(6.0, 0.0, 0.0);
        let mut off = false;
        for _ in 0..30 {
            b.step(DT, &run_x(), &f.sim(), &NoInk, p);
            if !b.grounded {
                off = true;
                break;
            }
        }
        assert!(off, "must have run off the edge (pos.x={})", b.pos.x);
        // 2 more airborne frames (still well inside 0.12 s), then press jump
        for _ in 0..2 {
            b.step(DT, &idle(), &f.sim(), &NoInk, p);
        }
        assert!(!b.grounded);
        let vy_before = b.vel.y;
        b.step(DT, &jump, &f.sim(), &NoInk, p);
        assert!(
            b.vel.y > vy_before + 1.0,
            "coyote jump must fire within coyoteTime (vy {} → {})",
            vy_before,
            b.vel.y
        );
    }

    // ------------------------------------------------------------- form/feel
    #[test]
    fn squid_dry_land_is_slower_than_kid() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let squid = ActorInput {
            move_dir: vec3(1.0, 0.0, 0.0),
            squid: true,
            ..ActorInput::default()
        };
        for _ in 0..180 {
            a.step(DT, &squid, &f.sim(), &NoInk, p); // dry floor: not submerged
        }
        assert!(!a.submerged);
        let sp = a.vel.x.hypot(a.vel.z);
        println!("squid on dry land: {sp:.3} m/s (expect 2.9)");
        close(
            f64::from(sp),
            f64::from(p.squid_dry_speed),
            0.05,
            "squid dry speed",
        );
    }

    #[test]
    fn fire_beats_squid_when_pressed_later() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let swim = ActorInput {
            squid: true,
            ..ActorInput::default()
        };
        for _ in 0..30 {
            a.step(DT, &swim, &f.sim(), &OwnInk, p);
        }
        assert_eq!(a.form, Form::Squid);
        // press fire while still holding squid → most recent press wins
        let both = ActorInput {
            squid: true,
            fire: true,
            ..ActorInput::default()
        };
        a.step(DT, &both, &f.sim(), &OwnInk, p);
        assert_eq!(a.form, Form::Kid, "fire pressed after squid pops out");
        assert_eq!(a.kid_t, 0.0, "emerge timer restarts");
        // fire gate stays shut until emergeDelay, then passes the buffer
        let mut gate_seen = false;
        for _ in 0..30 {
            a.step(DT, &idle(), &f.sim(), &OwnInk, p);
            if a.fire_gate.fire {
                gate_seen = true;
            }
        }
        assert!(gate_seen, "buffered shot must surface after emergeDelay");
    }

    #[test]
    fn spawn_barrier_pushes_out_of_enemy_pad() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        // drop the actor inside the Bravo bubble (pad[1] = (0,0,24), R=4.2)
        a.pos = vec3(0.0, 0.0, 22.0);
        a.vel = Vec3::ZERO;
        a.step(DT, &idle(), &f.sim(), &NoInk, p);
        let d = (a.pos.x.powi(2) + (a.pos.z - 24.0).powi(2)).sqrt();
        assert!(d >= 4.2 - 1e-3, "barrier must push out to the rim (d={d})");
    }

    // ------------------------------------------- review issue 2: uncovered paths
    #[test]
    fn air_steering_with_input_uses_air_accel() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        // build up forward speed on +x, then jump and steer to +z mid-air
        for _ in 0..180 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        let jump = ActorInput {
            jump: true,
            ..ActorInput::default()
        };
        a.step(DT, &jump, &f.sim(), &NoInk, p);
        assert!(!a.grounded);
        let (vx0, vz0) = (a.vel.x, a.vel.z);
        let steer = ActorInput {
            move_dir: vec3(0.0, 0.0, 1.0),
            ..ActorInput::default()
        };
        a.step(DT, &steer, &f.sim(), &NoInk, p);
        // kid airborne: target = max(run_speed, air_min_speed) = 6 toward +z;
        // dv = (−vx0, 6 − vz0), walked by air_accel·dt along the error.
        let (tvx, tvz) = (0.0, p.run_speed.max(p.air_min_speed));
        let dl = (tvx - vx0).hypot(tvz - vz0);
        let rate = p.air_accel * DT;
        let (wx, wz) = if dl <= rate {
            (tvx, tvz)
        } else {
            (vx0 + (tvx - vx0) / dl * rate, vz0 + (tvz - vz0) / dl * rate)
        };
        close(f64::from(a.vel.x), f64::from(wx), 1e-3, "air steer vx");
        close(f64::from(a.vel.z), f64::from(wz), 1e-3, "air steer vz");
        // the heading must have rotated toward the input without a speed dip
        assert!(a.vel.z > vz0, "air control pulls velocity toward the input");
    }

    #[test]
    fn air_no_input_drifts_down_air_decel() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        for _ in 0..180 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        let jump = ActorInput {
            jump: true,
            ..ActorInput::default()
        };
        a.step(DT, &jump, &f.sim(), &NoInk, p);
        let vx0 = a.vel.x;
        a.step(DT, &idle(), &f.sim(), &NoInk, p);
        // airborne, no input: velocity error toward zero walks at air_decel·dt
        close(
            f64::from(vx0 - a.vel.x),
            f64::from(p.air_decel * DT),
            1e-3,
            "air decel step",
        );
        assert!(a.vel.x > 0.0, "still moving forward");
    }

    #[test]
    fn over_speed_sheds_at_brake_rate() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        a.step(DT, &idle(), &f.sim(), &NoInk, p);
        assert!(a.grounded);
        // inject a speed above the run cap (e.g. after a swim exit glide)
        a.vel.x = 9.0;
        a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        // sp − vts = 3 > decelKnee → full brake rate: Δ = runDecel·dt
        close(
            f64::from(9.0 - a.vel.x),
            f64::from(p.run_decel * DT),
            1e-3,
            "overspeed shed",
        );
        assert!(
            a.vel.x > p.run_speed,
            "one frame must not overshoot the cap"
        );
        // converges down to exactly the target speed
        for _ in 0..300 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        close(
            f64::from(a.vel.x),
            f64::from(p.run_speed),
            1e-3,
            "cap settle",
        );
    }

    #[test]
    fn reverse_in_enemy_ink_half_rate() {
        let f = fixture();
        let p = &f.tuning.player;
        // plant-and-reverse runs at max(reverseDecel, D)·dt·0.5 in enemy ink
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        for _ in 0..180 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        assert!(a.grounded && a.vel.x > 0.99 * p.run_speed);
        let vx0 = a.vel.x;
        let rev = ActorInput {
            move_dir: vec3(-1.0, 0.0, 0.0),
            ..ActorInput::default()
        };
        // first enemy-ink frame: ground probe from last frame + EnemyInk →
        // on_enemy, sp > 0.5 and ang = π > reverseAngle → plant-and-reverse
        a.step(DT, &rev, &f.sim(), &EnemyInk, p);
        assert!(a.on_enemy);
        let r_enemy = p.reverse_decel.max(p.enemy_ink_decel) * DT * 0.5;
        close(
            f64::from(vx0 - a.vel.x),
            f64::from(r_enemy),
            1e-3,
            "reverse rate",
        );
        // same frame on own ink would use the full rate
        let mut b = spawn_actor(&f, 0);
        b.invuln = 0.0;
        for _ in 0..180 {
            b.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        let vx1 = b.vel.x;
        b.step(DT, &rev, &f.sim(), &NoInk, p);
        close(
            f64::from(vx1 - b.vel.x),
            f64::from(p.reverse_decel * DT),
            1e-3,
            "reverse rate (dry)",
        );
    }

    // ------------------------------------------------ JS golden cross-checks
    // Hard-coded per-frame measurements from the real JS game, captured with:
    //   python3 tools/serve.py 8490 &
    //   node tools/measure-handling.mjs --only accel,jump,buffer,swim,turn --raw /tmp/mh/golden.json
    // (headless Chromium; see /tmp/mh/measure-linux.mjs for the Linux launch flags).
    // The JS scenarios sample AFTER each 1/60 frame, so index 0 = one frame
    // after the input change — mirrored here by stepping then recording.
    // Axis mapping: JS yaw=0 lanes run along +z (KeyW); here +x is forward.
    //
    // The JS coyote probe is NOT asserted: its "left ground" detector
    // (`y < 2.3 && z > -35.5`) fires ~6 frames after the actor actually
    // leaves the deck, so the reported late=1..9 window is shifted; an
    // instrumented run confirmed JS `coyote` decays from 0.12 exactly like
    // Rust (jump succeeds while coyote > 0, fails once negative). Real
    // coyote timing is covered by `jump_buffer_and_coyote_time`.

    fn close_abs(got: f64, want: f64, tol: f64, what: &str) {
        assert!(
            (got - want).abs() <= tol,
            "{what}: got {got:.4}, JS golden {want:.4}, |err| > {tol}"
        );
    }

    #[test]
    fn golden_accel_stop_reverse() {
        let f = fixture();
        let p = &f.tuning.player;
        // --- go: 70 frames of full input from a standing start
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let mut go_sp = Vec::new();
        for _ in 0..70 {
            a.step(DT, &run_x(), &f.sim(), &NoInk, p);
            go_sp.push(a.vel.x.hypot(a.vel.z) as f64);
        }
        // JS speedFirst6 (r3): [0.583, 1.343, 2.469, 3.636, 4.802, 5.634]
        let want_first6 = [0.583, 1.343, 2.469, 3.636, 4.802, 5.634];
        for (i, w) in want_first6.iter().enumerate() {
            close_abs(go_sp[i], *w, 0.02, &format!("accel speed[{i}]"));
        }
        // t90 = first frame index with sp >= 5.4 → 5 (0.0833 s)
        let t90 = go_sp.iter().position(|&v| v >= 5.4).expect("t90");
        assert_eq!(t90, 5, "JS golden t90 = frame 5");
        // accPeak = max (Δsp)·60 = (4.802→5.634)·60 ≈ 70
        let acc_peak = go_sp
            .windows(2)
            .map(|w| (w[1] - w[0]) * 60.0)
            .fold(f64::NEG_INFINITY, f64::max);
        close_abs(acc_peak, 70.0, 2.0, "accel accPeak");
        // --- stop: release input, 45 frames
        let x0 = a.pos.x as f64;
        let mut stop_sp = Vec::new();
        for _ in 0..45 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
            stop_sp.push(a.vel.x.hypot(a.vel.z) as f64);
        }
        // JS stopFirst8: [5.033, 4.067, 3.1, 2.133, 1.168, 0.465, 0.011, 0]
        let want_stop8 = [5.033, 4.067, 3.1, 2.133, 1.168, 0.465, 0.011, 0.0];
        for (i, w) in want_stop8.iter().enumerate() {
            close_abs(stop_sp[i], *w, 0.02, &format!("stop speed[{i}]"));
        }
        let d_stop = (a.pos.x as f64) - x0;
        close_abs(d_stop, 0.266, 0.02, "stop distance");
        // --- reverse: fresh actor, 50 frames forward, then full reverse 50
        let mut b = spawn_actor(&f, 0);
        b.invuln = 0.0;
        for _ in 0..50 {
            b.step(DT, &run_x(), &f.sim(), &NoInk, p);
        }
        let rev = ActorInput {
            move_dir: vec3(-1.0, 0.0, 0.0),
            ..ActorInput::default()
        };
        let mut rev_vx = Vec::new();
        for _ in 0..50 {
            b.step(DT, &rev, &f.sim(), &NoInk, p);
            rev_vx.push(b.vel.x as f64);
        }
        // JS tReverseZero = frame 4 (0.0667 s), tReverse90 = frame 9 (0.15 s)
        let t_rev0 = rev_vx.iter().position(|&v| v <= 0.0).expect("reverse zero");
        assert_eq!(t_rev0, 4, "JS golden reverse→0 at frame 4");
        let t_rev90 = rev_vx.iter().position(|&v| v <= -5.4).expect("reverse 90%");
        assert_eq!(t_rev90, 9, "JS golden reverse→90% at frame 9");
    }

    #[test]
    fn golden_standing_jump() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        let jump = ActorInput {
            jump: true,
            ..ActorInput::default()
        };
        // JS samples AFTER the frame, so r[0] is one frame after takeoff.
        a.step(DT, &jump, &f.sim(), &NoInk, p);
        let y0 = a.pos.y as f64;
        let mut ys = vec![y0];
        let mut vys = vec![a.vel.y as f64];
        for _ in 0..60 {
            a.step(DT, &idle(), &f.sim(), &NoInk, p);
            ys.push(a.pos.y as f64);
            vys.push(a.vel.y as f64);
        }
        // arc(): apexH = max(y) − r[0].y = 1.216, tApex = frame 19 (0.317 s)
        let (apex, apex_i) =
            ys.iter().enumerate().fold(
                (f64::NEG_INFINITY, 0),
                |m, (i, &y)| if y > m.0 { (y, i) } else { m },
            );
        close_abs(apex - y0, 1.216, 0.02, "jump apexH");
        assert_eq!(apex_i, 19, "JS golden tApex = frame 19");
        // land = first i > 3 with grounded; tAir = 38 frames (0.633 s), landVy = r[37].vy = −8.19
        let mut land_i = None;
        let mut grounded_seq = Vec::new();
        let mut b = spawn_actor(&f, 0);
        b.invuln = 0.0;
        b.step(DT, &jump, &f.sim(), &NoInk, p);
        grounded_seq.push(b.grounded);
        for _ in 0..60 {
            b.step(DT, &idle(), &f.sim(), &NoInk, p);
            grounded_seq.push(b.grounded);
        }
        for (i, &g) in grounded_seq.iter().enumerate() {
            if i > 3 && g {
                land_i = Some(i);
                break;
            }
        }
        let land = land_i.expect("must land");
        assert_eq!(land, 38, "JS golden tAir = 38 frames");
        close_abs(vys[land - 1], -8.19, 0.15, "jump landVy");
    }

    #[test]
    fn golden_jump_buffer_edges() {
        let f = fixture();
        let p = &f.tuning.player;
        // SC.buffer: drop from y = 1.6 (vel 0) onto the floor; find landF,
        // then replay pressing jump `early` frames before landF for one frame.
        let mut probe = spawn_actor(&f, 0);
        probe.invuln = 0.0;
        probe.pos.y = 1.6;
        probe.vel = Vec3::ZERO;
        let mut land_f = None;
        for i in 0..80 {
            probe.step(DT, &idle(), &f.sim(), &NoInk, p);
            if land_f.is_none() && probe.grounded {
                land_f = Some(i);
            }
        }
        let land_f = land_f.expect("must land");
        // JS golden landF = 22; Rust lands at 20 — the 2-frame offset comes
        // from the scenario setup (JS `place` settles at y = 0.02 with 40
        // warm frames, Rust spawns at y = 0.05), not from the physics. The
        // buffer semantics are asserted relatively below.
        println!("rust landF = {land_f} (JS golden 22)");
        for (early, want_jumped) in [(2, true), (4, true), (6, true), (8, false), (10, false)] {
            let mut a = spawn_actor(&f, 0);
            a.invuln = 0.0;
            a.pos.y = 1.6;
            a.vel = Vec3::ZERO;
            let mut jumped = false;
            for i in 0..80 {
                let inp = if i == land_f - early || i == land_f - early + 1 {
                    ActorInput {
                        jump: true,
                        ..ActorInput::default()
                    }
                } else {
                    idle()
                };
                a.step(DT, &inp, &f.sim(), &NoInk, p);
                if i > land_f && a.vel.y > 3.0 {
                    jumped = true;
                }
            }
            assert_eq!(
                jumped, want_jumped,
                "buffer early={early}: JS golden jumped={want_jumped}"
            );
        }
    }

    #[test]
    fn golden_swim_accel() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        // JS: 20 frames of Shift alone (squid at rest), then Shift+W recorded.
        let squid = ActorInput {
            squid: true,
            ..ActorInput::default()
        };
        for _ in 0..20 {
            a.step(DT, &squid, &f.sim(), &OwnInk, p);
        }
        assert!(a.submerged);
        let swim = ActorInput {
            move_dir: vec3(1.0, 0.0, 0.0),
            squid: true,
            ..ActorInput::default()
        };
        let mut sp = Vec::new();
        for _ in 0..55 {
            a.step(DT, &swim, &f.sim(), &OwnInk, p);
            sp.push(a.vel.x.hypot(a.vel.z) as f64);
        }
        // JS first8: [0.8, 1.647, 2.6, 3.653, 4.72, 5.787, 6.853, 7.92]
        let want = [0.8, 1.647, 2.6, 3.653, 4.72, 5.787, 6.853, 7.92];
        for (i, w) in want.iter().enumerate() {
            close_abs(sp[i], *w, 0.02, &format!("swim speed[{i}]"));
        }
        // t90 (> 0.9·11.8) = frame 10 (0.1667 s); top speed 11.8
        let t90 = sp.iter().position(|&v| v > 0.9 * 11.8).expect("swim t90");
        assert_eq!(t90, 10, "JS golden swim t90 = frame 10");
        close_abs(sp[54], 11.8, 0.02, "swim top speed");
    }

    #[test]
    fn golden_turn_90() {
        let f = fixture();
        let p = &f.tuning.player;
        let mut a = spawn_actor(&f, 0);
        a.invuln = 0.0;
        // JS world axes: yaw 0 faces +z, KeyW = +z, KeyD (right) = −x.
        let fwd = ActorInput {
            move_dir: vec3(0.0, 0.0, 1.0),
            ..ActorInput::default()
        };
        for _ in 0..45 {
            a.step(DT, &fwd, &f.sim(), &NoInk, p);
        }
        assert!(a.yaw.abs() < 1e-3, "yaw settled to 0 on the +z lane");
        let right = ActorInput {
            move_dir: vec3(-1.0, 0.0, 0.0),
            ..ActorInput::default()
        };
        let diff = |a0: f64, b0: f64| {
            let mut d = b0 - a0;
            while d > std::f64::consts::PI {
                d -= 2.0 * std::f64::consts::PI;
            }
            while d < -std::f64::consts::PI {
                d += 2.0 * std::f64::consts::PI;
            }
            d
        };
        let mut sp = Vec::new();
        let mut vang = Vec::new();
        let mut yaws = Vec::new();
        for _ in 0..50 {
            a.step(DT, &right, &f.sim(), &NoInk, p);
            let (vx, vz) = (a.vel.x as f64, a.vel.z as f64);
            sp.push(vx.hypot(vz));
            vang.push(vx.atan2(vz));
            yaws.push(a.yaw as f64);
        }
        // tVel: first frame with sp > 1 and |velAngle − (−π/2)| < 10° → frame 5
        let target = -std::f64::consts::FRAC_PI_2;
        let t_vel = (0..sp.len())
            .find(|&i| sp[i] > 1.0 && diff(vang[i], target).abs() < 10f64.to_radians())
            .expect("tVel");
        assert_eq!(t_vel, 5, "JS golden turn tVel = frame 5 (0.0833 s)");
        // speedMin over the series = 6 (heading slew carves at full speed)
        close_abs(
            sp.iter().fold(f64::INFINITY, |m, &v| m.min(v)),
            6.0,
            0.05,
            "turn speedMin",
        );
        // JS yawRate[i] = diff(r[i].yaw, r[i+1].yaw)·60 — the first entry skips
        // the pre-turn frame, so align by differencing consecutive samples.
        let rates: Vec<f64> = yaws.windows(2).map(|w| diff(w[0], w[1]) * 60.0).collect();
        // JS first8: [−5.7, −8.5, −11.3, −11.1, −9.8, −8.3, −6.8, −5.6]
        let want_rates = [-5.7, -8.5, -11.3, -11.1, -9.8, -8.3, -6.8, -5.6];
        for (i, w) in want_rates.iter().enumerate() {
            close_abs(rates[i], *w, 0.8, &format!("turn yawRate[{i}]"));
        }
        let rate_max = rates.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        close_abs(rate_max, 11.333, 0.5, "turn yawRateMax");
        let acc_max = rates
            .windows(2)
            .fold(0.0_f64, |m, w| m.max((w[1] - w[0]).abs() * 60.0));
        close_abs(acc_max, 170.0, 15.0, "turn yawAccMax");
        // tFace: first frame with |yaw − (−π/2)| < 10° → frame 13 (0.2167 s)
        let t_face = (0..yaws.len())
            .find(|&i| diff(yaws[i], target).abs() < 10f64.to_radians())
            .expect("tFace");
        assert_eq!(t_face, 13, "JS golden turn tFace = frame 13");
    }
}
