//! Task 8 tests: match lifecycle, intro barrier, scoring wiring, serde.

use super::*;
use crate::actor::FIXED_DT;
use crate::collision::CollisionWorld;
use crate::geometry::StageLayout;
use crate::tuning::Tuning;
use glam::Vec3;

const DT: f32 = FIXED_DT;

struct Fix {
    layout: StageLayout,
    world: CollisionWorld,
    tuning: Tuning,
}

fn fix() -> Fix {
    Fix {
        layout: crate::embedded_tidewater(),
        world: CollisionWorld::tidewater(),
        tuning: crate::embedded_tuning(),
    }
}

impl Fix {
    fn sim(&self) -> SimWorld<'_> {
        SimWorld::new(&self.world, &self.layout)
    }
}

fn xz_dist(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x).hypot(a.z - b.z)
}

// ------------------------------------------------------------------ roster
#[test]
fn roster_is_4v4_on_the_spawn_rings() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let m = Match::new(6.0, 7, &sim, t, &f.tuning.match_config);
    assert_eq!(m.actors.len(), 8);
    assert_eq!(m.actors.iter().filter(|a| a.team == 0).count(), 4);
    assert_eq!(m.actors.iter().filter(|a| a.team == 1).count(), 4);
    for a in &m.actors {
        let pad = sim.spawn_pads[a.team];
        let d = xz_dist(a.pos, pad);
        assert!(
            (d - SPAWN_RING_R).abs() < 0.05,
            "slot {} on the ring: {d}",
            a.slot
        );
        let yaw = if a.team == 0 {
            0.0
        } else {
            std::f32::consts::PI
        };
        assert!((a.yaw - yaw).abs() < 1e-6, "facing the arena");
        assert_eq!(a.invuln, 0.0, "the match zeroes spawn invuln");
        assert!(a.alive);
    }
    assert_eq!(m.phase, Phase::Intro);
    assert_eq!(m.events[0], MatchEvent::Phase(Phase::Intro));
}

// ------------------------------------------------------------------ TR-8.1
#[test]
fn tr8_1_full_run_settles_with_result_and_restart_zeroes() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(6.0, 0xABCD_1234, &sim, t, &f.tuning.match_config);
    let mut inputs = vec![ActorInput::default(); 8];
    let max_steps = ((INTRO_TIME + 6.0 + FINISH_TIME) * 60.0) as usize + 120;
    let mut settled = false;
    for _ in 0..max_steps {
        let live = m.playing();
        for inp in inputs.iter_mut() {
            inp.fire = false;
        }
        if live {
            inputs[0].fire = true;
            inputs[1].fire = true;
        }
        m.step(DT, &inputs, &sim, t, w);
        if m.phase == Phase::End {
            settled = true;
            break;
        }
    }
    assert!(settled, "match must settle inside the step budget");

    let r = m.result.expect("end carries a result");
    assert!(
        (r.elapsed - 6.0).abs() < 0.1,
        "clock ran the full duration: {}",
        r.elapsed
    );
    assert!(
        r.coverage[0] > r.coverage[1],
        "firing team painted more: {:?}",
        r.coverage
    );
    assert!(r.points[0] > 0.0, "score line: {:?}", r.points);
    assert_eq!(r.winner, 0, "higher coverage wins");
    assert_eq!(m.time, 0.0);

    // phase sequence observed on the bus
    let seq: Vec<Phase> = m
        .events
        .iter()
        .filter_map(|e| match e {
            MatchEvent::Phase(p) => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(
        seq,
        [Phase::Intro, Phase::Active, Phase::Finish, Phase::End]
    );

    // ---- restart: everything back to the fresh-match state (TR-8.1 归零)
    m.restart(&sim, t);
    assert_eq!(m.phase, Phase::Intro);
    assert!((m.time - 6.0).abs() < 1e-6, "clock reset");
    assert_eq!(m.state_t, 0.0);
    assert!(m.result.is_none());
    assert!(m.kills.is_empty());
    assert_eq!(m.paint.coverage(), [0.0, 0.0]);
    assert!(m.projectiles.list.is_empty());
    for a in &m.actors {
        assert!(a.alive);
        assert!((a.hp - t.hp).abs() < 1e-6);
        assert_eq!(a.turf, 0.0);
        assert_eq!(a.splats, 0);
        assert_eq!(a.deaths, 0);
        let d = xz_dist(a.pos, sim.spawn_pads[a.team]);
        assert!((d - SPAWN_RING_R).abs() < 0.05, "re-placed on the ring");
    }
    // events were cleared, then the new Intro transition was emitted
    assert_eq!(m.events, vec![MatchEvent::Phase(Phase::Intro)]);
    println!(
        "TR-8.1: settled at {:.2}s, cov={:.4}/{:.4}, pts={:.1}/{:.1}, winner={}",
        r.elapsed, r.coverage[0], r.coverage[1], r.points[0], r.points[1], r.winner
    );
}

#[test]
fn clock_emits_final_countdown_and_skips_one_minute() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(8.0, 3, &sim, t, &f.tuning.match_config);
    let idle = [];
    let mut counts = Vec::new();
    let mut one_minute = false;
    for _ in 0..((INTRO_TIME + 8.0 + FINISH_TIME) * 60.0) as usize + 60 {
        m.step(DT, &idle, &sim, t, w);
        for e in m.events.drain(..) {
            match e {
                MatchEvent::Countdown { n } => counts.push(n),
                MatchEvent::OneMinute => one_minute = true,
                _ => {}
            }
        }
        if m.phase == Phase::End {
            break;
        }
    }
    // finalCountdown (10) covers the whole 8 s clock: 8 → 1, never 0.
    assert_eq!(counts, (1..=8).rev().collect::<Vec<i32>>());
    assert!(!one_minute, "duration 8 < 60: no last-minute event");
}

// ------------------------------------------------------------------ TR-8.2
#[test]
fn tr8_2_intro_barrier_holds_until_active() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(6.0, 11, &sim, t, &f.tuning.match_config);
    let idle = [];
    assert_eq!(m.phase, Phase::Intro);

    // Shove actor 0 toward the arena, well outside its own pad radius.
    let pad = sim.spawn_pads[0];
    m.actors[0].pos = vec3(pad.x, pad.y, pad.z + 9.0);
    m.actors[0].vel = Vec3::ZERO;
    m.step(DT, &idle, &sim, t, w);
    let d = xz_dist(m.actors[0].pos, pad);
    assert!(
        d <= sim.spawn_barrier + 1e-3,
        "intro clamp must hold at the barrier radius, got {d}"
    );

    // Run the intro out; the clamp keeps releasing as soon as play starts.
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &idle, &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    assert_eq!(m.phase, Phase::Active);

    // Same shove during play: no clamp — the actor stays where it is.
    m.actors[0].pos = vec3(pad.x, pad.y, pad.z + 9.0);
    m.actors[0].vel = Vec3::ZERO;
    for _ in 0..5 {
        m.step(DT, &idle, &sim, t, w);
    }
    let d = xz_dist(m.actors[0].pos, pad);
    assert!(d > 8.5, "barrier must be lifted during play, got {d}");
    println!(
        "TR-8.2: intro hold d={:.3} m, active release d={:.3} m",
        sim.spawn_barrier, d
    );
}

#[test]
fn respawn_is_gated_outside_play() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(6.0, 12, &sim, t, &f.tuning.match_config);
    let idle = [];
    // A dead actor with an expired timer must NOT respawn during the intro.
    m.actors[3].alive = false;
    m.actors[3].respawn_timer = 0.01;
    m.step(DT, &idle, &sim, t, w);
    assert!(!m.actors[3].alive, "intro holds respawns (JS canRespawn)");
    // The gate pins the expired timer just above zero (it can never reach the
    // respawn call), unlike a full `respawn_time` re-arm.
    assert!(
        m.actors[3].respawn_timer > 0.0 && m.actors[3].respawn_timer < 1.0,
        "timer held near zero, got {}",
        m.actors[3].respawn_timer
    );

    // Once play starts the same actor respawns on its timer.
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &idle, &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    let steps = (t.respawn_time * 60.0) as usize + 30;
    for _ in 0..steps {
        m.step(DT, &idle, &sim, t, w);
    }
    assert!(m.actors[3].alive, "respawned during play");
    let pad = sim.spawn_pads[0];
    assert!(xz_dist(m.actors[3].pos, pad) < 2.0, "back on the own pad");
}

// ------------------------------------------------------------------ scoring
#[test]
fn kill_routes_splash_credit_splat_count_and_bus() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(30.0, 0x5EED, &sim, t, &f.tuning.match_config);
    let mut inputs = vec![ActorInput::default(); 8];

    // Get to play.
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &inputs, &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    assert_eq!(m.phase, Phase::Active);

    // Park a victim 3.5 m down actor 0's aim line (facing +Z from the pad).
    let a0 = m.actors[0].pos;
    m.actors[4].spawn_at(vec3(a0.x, a0.y, a0.z + 3.5), std::f32::consts::PI, &sim, t);
    m.actors[4].set_invuln(0.0);
    m.actors[4].pos = vec3(a0.x, a0.y, a0.z + 3.5);

    let mut killed = None;
    for step in 0..120 {
        for inp in inputs.iter_mut() {
            inp.fire = false;
        }
        inputs[0].fire = true;
        m.step(DT, &inputs, &sim, t, w);
        if !m.actors[4].alive {
            killed = Some(step);
            break;
        }
    }
    let step = killed.expect("victim splatted by sustained fire");
    assert!(m.actors[0].splats >= 1, "attacker credited with the splat");
    assert_eq!(m.actors[4].deaths, 1);
    assert!(
        m.actors[0].turf > 0.0,
        "splash area credited to the attacker"
    );

    let log = m
        .kills
        .iter()
        .find(|k| k.victim_team == 1 && k.victim_slot == 0);
    let log = log.expect("kill logged for the victim");
    assert_eq!(log.attacker, Some((0, 0)), "attacker identity resolved");
    assert_eq!(log.cause, SplatCause::Weapon);
    assert!(
        m.events.contains(&MatchEvent::Splat {
            victim_team: 1,
            victim_slot: 0,
            attacker: Some((0, 0)),
            cause: SplatCause::Weapon,
        }),
        "splat on the match bus"
    );
    assert!(m.events.iter().any(
        |e| matches!(e, MatchEvent::Turf { owner_team: 0, owner_slot: 0, area } if *area > 0.0)
    ));
    println!(
        "TR-8 scoring: kill at step {step}, splats={}, turf={:.3} m2",
        m.actors[0].splats, m.actors[0].turf
    );
}

// ---------------------------------------------------- review regressions (T8)
#[test]
fn dead_actor_fires_nothing() {
    // Review B-1/M-2: JS `update` returns before `weaponRunner.update` when
    // dead (actor.js L245-249) and `splat` calls `onDeath()` → `reset()`
    // (weapons.js L60). A corpse holding the trigger must not shoot, drain
    // ink, or earn turf/splats.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(30.0, 0xBAD, &sim, t, &f.tuning.match_config);
    let mut inputs = vec![ActorInput::default(); 8];
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &inputs, &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    // Actor 0 fires until the gate is live (trigger held, runner warm), then
    // dies by weapon.
    let mut warmed = false;
    for _ in 0..30 {
        for inp in inputs.iter_mut() {
            inp.fire = false;
        }
        inputs[0].fire = true;
        m.step(DT, &inputs, &sim, t, w);
        if m.actors[0].fire_gate.fire {
            warmed = true;
            break;
        }
    }
    assert!(warmed, "gate must go live while firing");
    m.actors[0].splat(SplatCause::Water, None, t);
    m.actors[0].drain_events();
    let mut fired_after_death = 0;
    let mut ink = 0.0;
    let mut turf = 0.0;
    let mut splats = 0;
    for k in 0..60 {
        for inp in inputs.iter_mut() {
            inp.fire = false;
        }
        inputs[0].fire = true;
        drop(m.projectiles.drain_events());
        m.step(DT, &inputs, &sim, t, w);
        let pev = m.projectiles.drain_events();
        fired_after_death += pev
            .iter()
            .filter(|e| matches!(e, SimEvent::Fire { owner_slot: 0, .. }))
            .count();
        // The round fired before death expires after ~22 steps; only once
        // the sky is clear can the corpse's ledger be snapshotted.
        if k == 30 {
            ink = m.actors[0].ink;
            turf = m.actors[0].turf;
            splats = m.actors[0].splats;
        }
    }
    assert_eq!(fired_after_death, 0, "a corpse must not fire");
    assert_eq!(m.actors[0].ink, ink, "a corpse must not drain ink");
    assert_eq!(m.actors[0].turf, turf, "a corpse earns no turf");
    assert_eq!(m.actors[0].splats, splats, "a corpse earns no splats");
    // The runner was reset on death (JS `onDeath`) and held reset while the
    // corpse keeps holding the trigger. `spread` is the runner's only pub
    // state — it is non-zero right after a shot, zero after `reset`.
    assert_eq!(m.runners[0].spread, 0.0, "runner held reset (M-2)");
    assert!(!m.actors[0].fire_gate.fire, "stale gate cleared (B-1)");
}

#[test]
fn water_death_inherits_recent_attacker() {
    // Review M-1: JS L384 `splat(this.lastDamage < 4 ? this.lastAttacker :
    // null, 'water')` — a chased victim credits the chaser.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(30.0, 0xA11E, &sim, t, &f.tuning.match_config);
    let idle = [];
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &idle, &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    // Hit actor 4 (non-lethal), then drown it while `lastDamage < 4`.
    assert!(!m.actors[4].damage(30.0, Some((0, 1)), t), "non-lethal hit");
    m.actors[4].pos = vec3(100.0, -2.0, 0.0);
    m.actors[4].vel = Vec3::ZERO;
    m.step(DT, &idle, &sim, t, w);
    assert!(!m.actors[4].alive, "drowned");
    let log = m
        .kills
        .iter()
        .find(|k| k.victim_team == 1 && k.victim_slot == 0)
        .expect("water kill logged");
    assert_eq!(log.cause, SplatCause::Water);
    assert_eq!(
        log.attacker,
        Some((0, 1)),
        "chase credit inherited (JS L384)"
    );
    assert_eq!(m.actors[1].splats, 1, "chaser credited with the splat");
    // The splash splat lands over open sea here, so (like JS) it claims no
    // turf — only the `splats` credit applies.
    assert_eq!(m.actors[1].turf, 0.0, "sea splash claims no area");
}

#[test]
fn soft_push_separates_overlapping_bodies() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(6.0, 13, &sim, t, &f.tuning.match_config);
    for _ in 0..(INTRO_TIME * 60.0) as usize + 2 {
        m.step(DT, &[], &sim, t, w);
        if m.phase == Phase::Active {
            break;
        }
    }
    let a0 = m.actors[0].pos;
    m.actors[1].pos = vec3(a0.x + 0.3, a0.y, a0.z);
    m.actors[1].vel = Vec3::ZERO;
    m.step(DT, &[], &sim, t, w);
    let d = xz_dist(m.actors[0].pos, m.actors[1].pos);
    assert!(d > 0.45, "pair pushed apart: {d}");
}

// ------------------------------------------------------------------ TR-8.3
#[test]
fn tr8_3_gameplay_state_serde_roundtrips() {
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.spritzer;
    let mut m = Match::new(6.0, 0x00C0_FFEE, &sim, t, &f.tuning.match_config);
    let mut inputs = vec![ActorInput::default(); 8];
    // Run into mid-play with live projectiles, paint and bus events.
    for _ in 0..((INTRO_TIME + 2.0) * 60.0) as usize {
        let live = m.playing();
        for inp in inputs.iter_mut() {
            inp.fire = false;
        }
        if live {
            inputs[0].fire = true;
        }
        m.step(DT, &inputs, &sim, t, w);
    }
    assert_eq!(m.phase, Phase::Active);
    assert!(m.paint.coverage()[0] > 0.0);

    let json = serde_json::to_string(&m).expect("serialize");
    let back: Match = serde_json::from_str(&json).expect("deserialize");
    let json2 = serde_json::to_string(&back).expect("re-serialize");
    assert_eq!(json, json2, "serde roundtrip must be byte-stable");

    assert_eq!(back.phase, m.phase);
    assert!((back.time - m.time).abs() < 1e-6);
    assert_eq!(back.actors.len(), 8);
    assert_eq!(back.projectiles.list.len(), m.projectiles.list.len());
    assert_eq!(back.events.len(), m.events.len());
    let s1: f32 = m.actors.iter().map(|a| a.turf).sum();
    let s2: f32 = back.actors.iter().map(|a| a.turf).sum();
    assert!((s1 - s2).abs() < 1e-3, "turf totals survive the roundtrip");
    assert_eq!(back.paint.coverage(), m.paint.coverage());
    println!(
        "TR-8.3: roundtrip stable, {} bytes, turf {s1:.3} m2",
        json.len()
    );
}
