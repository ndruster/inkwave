//! Task 9 bot-brain tests: deterministic smoke run, perception, aim
//! convergence, and the refill/retreat mode wiring on synthetic worlds.

use super::*;
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural};
use crate::tuning::Tuning;
use glam::{Vec3, vec3};

const DT: f32 = crate::actor::FIXED_DT;

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

fn bx(min: [f32; 3], max: [f32; 3]) -> Brush {
    Brush::Box {
        min,
        max,
        common: flags(),
    }
}

const B_SMALL: Bounds = Bounds {
    min_x: -10.0,
    max_x: 10.0,
    min_z: -10.0,
    max_z: 10.0,
};

struct Fix {
    world: CollisionWorld,
    layout: crate::geometry::StageLayout,
    tuning: Tuning,
}

/// A 20×20 m flat deck centred on the origin, pads at ±5 z.
fn flat_fix() -> Fix {
    let world =
        CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B_SMALL);
    let stage = crate::geometry::StageInfo {
        id: "test".into(),
        bounds: B_SMALL,
        spawn_pads: vec![[0.0, 0.0, -5.0], [0.0, 0.0, 5.0]],
        spawn_barrier: 4.2,
        water_y: -1.6,
    };
    let meta = crate::geometry::LayoutMeta {
        primitive_counts: crate::geometry::PrimitiveCounts {
            boxes: 1,
            obox: 0,
            ramp: 0,
            total: 1,
        },
        source_counts: crate::geometry::SourceCounts {
            single: 0,
            half: 0,
            half_mirrored: 0,
        },
        bounds: B_SMALL,
        spawn_pads: stage.spawn_pads.clone(),
        spawn_barrier: stage.spawn_barrier,
        water_y: stage.water_y,
        surface_slots: Default::default(),
    };
    let layout = crate::geometry::StageLayout {
        schema: "test".into(),
        source: crate::tuning::Source {
            commit: "test".into(),
            path: "test".into(),
        },
        stage,
        primitives: vec![bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])],
        meta,
    };
    Fix {
        world,
        layout,
        tuning: crate::embedded_tuning(),
    }
}

fn kid(t: &Tuning, team: usize, slot: usize, pos: Vec3, yaw: f32) -> Actor {
    let mut a = Actor::new(team, slot, &t.player);
    a.pos = pos;
    a.yaw = yaw;
    a.aim_yaw = yaw;
    a
}

fn ctx_of<'a>(
    f: &'a Fix,
    nav: &'a NavGraph,
    paint: &'a PaintGrid,
    sim: &'a SimWorld<'a>,
    actors: &'a [Actor],
    mate_goals: &'a [Option<usize>],
) -> BotCtx<'a> {
    BotCtx {
        actors,
        nav,
        paint,
        world: sim,
        t: &f.tuning.player,
        w: &f.tuning.spritzer,
        mate_goals,
    }
}

// ------------------------------------------------------------------ smoke

#[test]
fn bot_moves_and_paints_over_a_long_run() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 42, 0);
    let mates = vec![None; 1];
    for _ in 0..60 * 60 {
        let snap = vec![a.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        let it = bot.step(DT, &mut a, &ctx);
        a.pos += it.move_dir * f.tuning.player.run_speed * DT;
        a.pos.y = 0.0;
    }
    assert!(
        bot.stats.dist > 10.0,
        "patrol covers ground: {}",
        bot.stats.dist
    );
    assert_eq!(bot.stats.mode, BotMode::Paint);
    // deterministic: same seed → identical stats after the same run
    let mut a2 = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut bot2 = Bot::new(&a2, &f.tuning.difficulty.easy, 42, 0);
    for _ in 0..60 * 60 {
        let snap = vec![a2.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        let it = bot2.step(DT, &mut a2, &ctx);
        a2.pos += it.move_dir * f.tuning.player.run_speed * DT;
        a2.pos.y = 0.0;
    }
    assert_eq!(bot.stats.dist.to_bits(), bot2.stats.dist.to_bits());
    assert_eq!(a.pos, a2.pos);
}

#[test]
fn dead_bot_emits_nothing_and_clears_plan() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 7, 0);
    let mates = vec![None; 1];
    // warm up so a path exists
    for _ in 0..30 {
        let snap = vec![a.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
    }
    a.alive = false;
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let it = bot.step(DT, &mut a, &ctx);
    assert_eq!(it, ActorInput::default());
    assert!(bot.path.is_none());
    assert!(bot.target.is_none());
}

// ------------------------------------------------------------------ perceive

#[test]
fn enemy_in_awareness_becomes_the_fight_target() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, -1.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 3, 0);
    let mates = vec![None; 2];
    let mut saw_target = false;
    for _ in 0..120 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
        if bot.target == Some((1, 0)) {
            saw_target = true;
            break;
        }
    }
    assert!(saw_target, "4 m away enemy must be acquired");
    assert_eq!(bot.mode, BotMode::Fight);
}

/// M-1 regression: JS filters perception on `anim.form === 'swim'` (squid
/// *and* submerged). A squid-ing kid on dry land must stay perceptible; a
/// submerged slow squid far away must not be.
#[test]
fn dry_land_squid_enemy_is_perceived_swimming_one_is_not() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, 1.0), std::f32::consts::PI);
    foe.form = Form::Squid;
    foe.submerged = false;
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 11, 0);
    let mates = vec![None; 2];
    let mut saw = false;
    for _ in 0..120 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
        if bot.target == Some((1, 0)) {
            saw = true;
            break;
        }
    }
    assert!(
        saw,
        "dry-land squid 6 m away must be acquired (JS 'swim' filter)"
    );

    // same foe but submerged + slow + far: the swim filter drops it.
    let mut bot2 = Bot::new(&a, &f.tuning.difficulty.easy, 11, 0);
    foe.submerged = true;
    for _ in 0..120 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot2.step(DT, &mut a, &ctx);
    }
    assert_eq!(
        bot2.target, None,
        "slow submerged squid beyond 3 m is ignored"
    );
}

#[test]
fn enemy_beyond_awareness_is_ignored() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(-9.0, 0.0, -9.0), 0.0);
    let foe = kid(&f.tuning, 1, 0, vec3(9.0, 0.0, 9.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 3, 0);
    let mates = vec![None; 2];
    for _ in 0..300 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
    }
    // ~25 m apart > awareness 16 m: stay patrolling
    assert_eq!(bot.mode, BotMode::Paint);
}

// ------------------------------------------------------------------ fight

#[test]
fn close_visible_enemy_aims_and_fires_after_reaction() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, -1.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 11, 0);
    let mates = vec![None; 2];
    let mut fired = false;
    for step in 0..600 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        let it = bot.step(DT, &mut a, &ctx);
        if it.fire {
            fired = true;
            // the aim ray must point roughly at the foe (+z from the bot)
            let d = a.aim_dir();
            assert!(d.z > 0.9, "aimed at the target: {d:?} step {step}");
            break;
        }
    }
    assert!(fired, "a point-blank visible enemy gets shot at");
}

// ------------------------------------------------------------------ refill

#[test]
fn low_ink_switches_to_refill() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    a.ink = 0.05 * f.tuning.player.ink_max;
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 5, 0);
    let mates = vec![None; 1];
    for _ in 0..60 {
        let snap = vec![a.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
    }
    assert_eq!(bot.mode, BotMode::Refill);
}

// ------------------------------------------------------------------ retreat

#[test]
fn wounded_bot_with_a_target_retreats() {
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    a.hp = 0.15 * f.tuning.player.hp;
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, -1.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 9, 0);
    let mates = vec![None; 2];
    let mut retreated = false;
    for _ in 0..600 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
        if bot.mode == BotMode::Retreat {
            retreated = true;
            break;
        }
    }
    assert!(retreated, "hp<0.2 with a visible enemy must flee");
}
