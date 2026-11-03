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
        only_in: None,
        not_in: None,
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
    /// Per-actor weapon kinds for [`ctx_of`] (empty = every lookup falls
    /// back to the shooter branch, Task 38 batch 1).
    kinds: Vec<WeaponKind>,
    runners: Vec<WeaponRunner>,
    /// Empty sub registry for [`ctx_of`] (Task 38 batch 2).
    subs: SubSystem,
    /// Empty teammate target memory for [`ctx_of`] (Task 38 batch 2).
    mate_minds: Vec<Option<MateMind>>,
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
        variant_counts: None,
    };
    let layout = crate::geometry::StageLayout {
        schema: "test".into(),
        source: crate::tuning::Source {
            commit: "test".into(),
            path: "test".into(),
        },
        stage,
        primitives: vec![bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])],
        zones: None,
        meta,
    };
    Fix {
        world,
        layout,
        tuning: crate::embedded_tuning(),
        kinds: Vec::new(),
        runners: Vec::new(),
        subs: SubSystem::new(0),
        mate_minds: Vec::new(),
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
        weapons: &f.tuning.weapons,
        weapon_range: &f.tuning.weapon_range,
        kinds: &f.kinds,
        runners: &f.runners,
        mate_goals,
        subs: &f.subs,
        mate_minds: &f.mate_minds,
        zones: None,
        zctl: None,
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

// ---------------------------------------------------------------- task 38

/// §1: `DifficultySet::get` ≡ JS `DIFFICULTY[d] || DIFFICULTY.normal`
/// (bots.js L276) — every roster id resolves, unknown ids fall back.
#[test]
fn difficulty_get_resolves_ids_and_falls_back_to_normal() {
    let f = flat_fix();
    let d = &f.tuning.difficulty;
    assert_eq!(d.get("easy").reaction, d.easy.reaction);
    assert_eq!(d.get("normal").aim_error, d.normal.aim_error);
    assert_eq!(d.get("hard").fire_discipline, d.hard.fire_discipline);
    for bogus in ["", "Beginner", "NORMal", "impossible"] {
        let g = d.get(bogus);
        assert_eq!(g.reaction, d.normal.reaction, "`{bogus}` must fall back");
        assert_eq!(g.awareness, d.normal.awareness);
    }
}

/// §2: the JS family tables — base rows (bots.js L18 MELEE, L19 CHARGES,
/// L35 LONG, L49 PAINTERS, L36 THROWN) plus the runtime patches the kit
/// modules apply on registration: `MELEE.blade = true` (blade.js L35),
/// `MELEE.mitts = true` (mitts.js L833), `CHARGES.bow` / `LONG.bow`
/// (bow.js L492 / L493), `THROWN.waddle` / `THROWN.torpedo` /
/// `THROWN.shaker` (waddle.js L768 / torpedo.js L750 / shaker.js L618) and
/// `THROWN[KIND]` boomerang (boomerang.js L594) —
/// over every enum member. The retired kinds never exist here, so the
/// patched tables are exact.
#[test]
fn weapon_and_sub_family_tables_match_js() {
    use crate::sub::SubKind;
    for k in WeaponKind::ALL {
        assert_eq!(
            k.is_melee(),
            matches!(
                k,
                WeaponKind::Roller | WeaponKind::Brush | WeaponKind::Blade | WeaponKind::Mitts
            ),
            "MELEE[{k:?}]"
        );
        assert_eq!(
            k.is_charge(),
            matches!(
                k,
                WeaponKind::Charger | WeaponKind::Spinner | WeaponKind::Bow
            ),
            "CHARGES[{k:?}]"
        );
        assert_eq!(
            k.is_long(),
            matches!(
                k,
                WeaponKind::Charger | WeaponKind::Spinner | WeaponKind::Bow
            ),
            "LONG[{k:?}]"
        );
        assert_eq!(
            k.is_painters(),
            matches!(
                k,
                WeaponKind::Roller | WeaponKind::Brush | WeaponKind::Bucket
            ),
            "PAINTERS[{k:?}]"
        );
    }
    for s in [
        SubKind::Bomb,
        SubKind::Sticky,
        SubKind::Burst,
        SubKind::Seeker,
        SubKind::Scan,
        SubKind::Curtain,
        SubKind::Sprinkler,
        SubKind::Mine,
        SubKind::Beacon,
        SubKind::Mist,
        SubKind::Shaker,
        SubKind::Waddle,
        SubKind::Torpedo,
        SubKind::Tracer,
        SubKind::Boomerang,
    ] {
        assert_eq!(
            s.is_thrown(),
            matches!(
                s,
                SubKind::Bomb
                    | SubKind::Sticky
                    | SubKind::Burst
                    | SubKind::Mist
                    | SubKind::Seeker
                    | SubKind::Waddle
                    | SubKind::Torpedo
                    | SubKind::Shaker
                    | SubKind::Boomerang
            ),
            "THROWN[{s:?}]"
        );
    }
}

/// A fight-mode fixture: bot (team 0, slot 0) at z −5, static foe (team 1,
/// slot 0) at `foe_z`, both kinds wired through `f.kinds`. Returns the
/// inputs over `frames` headless steps (positions frozen — the kinematic
/// assertions read the intent directly).
fn duel_kinds(kind: WeaponKind, foe_z: f32, frames: usize) -> Vec<ActorInput> {
    let mut f = flat_fix();
    f.kinds = vec![kind, WeaponKind::Shooter];
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, foe_z), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 11, 0);
    let mates = vec![None; 2];
    // warm up until the duel is on — paint-mode frames (a pusher paints while
    // moving) are not part of the recorded window
    let mut guard = 0;
    while bot.mode != BotMode::Fight && guard < 600 {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
        guard += 1;
    }
    assert_eq!(bot.mode, BotMode::Fight, "the duel must be on");
    let mut outs = Vec::with_capacity(frames);
    for _ in 0..frames {
        let snap = vec![a.clone(), foe.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        outs.push(bot.step(DT, &mut a, &ctx));
    }
    outs
}

/// L443 / L449 / L451: a pusher (roller) closes in under 7 m while the
/// generic branch keeps its preferred distance (backs off at 4 m).
#[test]
fn melee_pusher_charges_generic_duelist_backs_off() {
    // 4 m apart: shooter pref = 12.5·0.7 = 8.75 → back off (−z); roller
    // pref = 0.5 and MELEE dist<7 → charge (+z).
    let shooter = duel_kinds(WeaponKind::Shooter, 0.0, 180);
    let roller = duel_kinds(WeaponKind::Roller, 0.0, 180);
    let late = 120..180;
    assert!(
        shooter[late.clone()].iter().all(|i| i.move_dir.z < -0.3),
        "shooter must keep distance: {:?}",
        shooter[179].move_dir
    );
    assert!(
        roller[late].iter().all(|i| i.move_dir.z > 0.3),
        "roller must close in: {:?}",
        roller[179].move_dir
    );
}

/// L495 / L500: the brush only swings inside 4.5 m, the roller strikes
/// inside 5.5 m — at a frozen 5 m duel the brush never pulls the trigger,
/// the roller does.
#[test]
fn brush_and_roller_fight_fire_gates() {
    // foe at z 0 → dist 5.0: brush gate `dist < 4.5` false forever
    // (positions are frozen), roller gate `dist < 5.5` true once aimed.
    let brush = duel_kinds(WeaponKind::Brush, 0.0, 300);
    assert!(brush.iter().all(|i| !i.fire), "brush must not fire at 5 m");
    let roller = duel_kinds(WeaponKind::Roller, 0.0, 300);
    assert!(roller.iter().any(|i| i.fire), "roller must fire at 5 m");
}

/// L557: the paint aim-pitch ladder — charger −0.12 vs the generic −0.42.
/// No foe: the bot patrols in paint mode and the aim spring converges.
fn paint_pitch_of(kind: WeaponKind) -> f32 {
    let mut f = flat_fix();
    f.kinds = vec![kind];
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 5, 0);
    let mates = vec![None; 1];
    for _ in 0..180 {
        let snap = vec![a.clone()];
        let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
        bot.step(DT, &mut a, &ctx);
    }
    a.aim_pitch
}

#[test]
fn paint_pitch_ladder_per_kind() {
    let charger = paint_pitch_of(WeaponKind::Charger);
    let shooter = paint_pitch_of(WeaponKind::Shooter);
    assert!(
        (charger - (-0.12)).abs() < 0.03,
        "charger paint pitch must settle at −0.12, got {charger}"
    );
    assert!(
        (shooter - (-0.42)).abs() < 0.03,
        "generic paint pitch must settle at −0.42, got {shooter}"
    );
}

// ------------------------------------------------------- task 38 batch 2

// L638-639: the press frame stamps `_bombAim`, the next frame drops `sub`
// and the squid form so the throw goes off (JS two-frame release).
#[test]
fn tail_two_frame_sub_release() {
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
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 21, 0);
    let mates = vec![None; 1];
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let wr = WeaponRunner::new();
    let mv = Vec3::ZERO;
    // frame 1: the stamp converts into a held `sub` press, squid cleared
    bot.bomb_aim = true;
    let mut it = ActorInput {
        squid: true,
        ..Default::default()
    };
    bot.tail(
        DT,
        &mut a,
        &ctx,
        &mv,
        0.0,
        -0.4,
        false,
        false,
        &mut it,
        WeaponKind::Shooter,
        &wr,
    );
    assert!(it.sub, "press frame holds the sub button");
    assert!(!it.squid, "press frame leaves squid form");
    assert!(!bot.bomb_aim && bot.release_bomb);
    // frame 2: release — sub drops so the throw fires
    let mut it2 = ActorInput {
        sub: true,
        squid: true,
        ..Default::default()
    };
    bot.tail(
        DT,
        &mut a,
        &ctx,
        &mv,
        0.0,
        -0.4,
        false,
        false,
        &mut it2,
        WeaponKind::Shooter,
        &wr,
    );
    assert!(!it2.sub && !it2.squid, "release frame drops both");
    assert!(!bot.release_bomb);
}

// L952-967: the ink pad / beacon ban, the sprinkler dead branch, and the
// bomb throw arming `bombCd` in [5, 11).
#[test]
fn fight_sub_gates_and_bomb_throw() {
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
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, 3.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 5, 0);
    let mates = vec![None; 2];
    let snap = vec![a.clone(), foe.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    // empty tank: never throw
    a.sub = SubKind::Bomb;
    a.ink = 0.0;
    for _ in 0..200 {
        assert!(!bot.fight_sub(&a, &ctx, 8.0), "no ink, no throw");
    }
    a.ink = 200.0;
    // the beacon is banned in a duel (JS L952)
    a.sub = SubKind::Beacon;
    for _ in 0..200 {
        assert!(!bot.fight_sub(&a, &ctx, 8.0), "beacon never in a fight");
    }
    // sprinkler: the paint tool never joins the duel branch (JS L962)
    a.sub = SubKind::Sprinkler;
    for _ in 0..500 {
        assert!(!bot.fight_sub(&a, &ctx, 8.0));
    }
    // bomb: 4 < 8 < 14, the shared roll eventually lands a throw and arms
    // `bombCd = 5 + r*6`
    a.sub = SubKind::Bomb;
    let mut threw = false;
    for _ in 0..5000 {
        if bot.fight_sub(&a, &ctx, 8.0) {
            threw = true;
            break;
        }
    }
    assert!(threw, "the bomb must eventually throw");
    let cd = bot.bomb_cd.expect("a throw arms bombCd");
    assert!((5.0..11.0).contains(&cd), "bombCd {cd} in [5, 11)");
}

// L969-983: a mine only goes down around the contested middle of the
// spawn-pad line, arming `bombCd = 8 + r*8`.
#[test]
fn paint_sub_mine_requires_contested_middle() {
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
    a.sub = SubKind::Mine;
    a.ink = 200.0;
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 13, 0);
    let mates = vec![None; 1];
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    // progress = 0 at own pad: the 0.3..0.7 gate never opens
    for _ in 0..3000 {
        assert!(!bot.paint_sub(&a, &ctx), "no mine at the spawn pad");
    }
    // middle of the lane: progress = 0.5, the roll eventually lands
    a.pos = vec3(0.0, 0.0, 0.0);
    let mut threw = false;
    for _ in 0..5000 {
        if bot.paint_sub(&a, &ctx) {
            threw = true;
            break;
        }
    }
    assert!(threw, "the middle must take a mine");
    let cd = bot.bomb_cd.expect("a placement arms bombCd");
    assert!((8.0..16.0).contains(&cd), "mine bombCd {cd} in [8, 16)");
}

// L830-856: the fight-mode id table (the rng-independent arms), the unknown
// id, and the paint-mode turf whitelist.
#[test]
fn want_special_fight_table_and_paint_turf_gate() {
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
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 9, 0);
    let mates = vec![None; 1];
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let mut check = |id: &str, fight: bool, dist: f32, vis: bool| -> bool {
        a.special_id = Some(id.to_string());
        bot.want_special(&a, &ctx, fight, dist, vis)
    };
    // rng-independent fight arms
    assert!(check("slam", true, 3.0, true));
    assert!(!check("slam", true, 6.0, true));
    assert!(check("storm", true, 15.0, true));
    assert!(!check("storm", true, 20.0, true));
    assert!(check("barrage", true, 13.0, true));
    assert!(check("kraken", true, 9.0, true));
    assert!(check("blower", true, 13.0, true));
    assert!(check("stamp", true, 8.0, true));
    assert!(check("zipcaster", true, 15.0, true));
    assert!(check("crab", true, 19.0, true));
    assert!(check("jetpack", true, 10.0, true));
    assert!(!check("jetpack", true, 4.0, true));
    assert!(check("zooka", true, 20.0, true));
    assert!(!check("zooka", true, 20.0, false));
    assert!(check("wail", true, 10.0, true));
    // unknown ids never press (JS L831 `?.kind || id` then the default arm)
    assert!(!check("mystery", true, 3.0, true));
    assert!(!check("mystery", false, 0.0, false));
    // paint mode: only the turf-claiming specials, and slam is not one
    for _ in 0..200 {
        assert!(!check("slam", false, 0.0, false));
        assert!(!check("bubbler", false, 0.0, false));
    }
    // a turf special on empty owned ground: gated only by the r <= 0.012
    // draw, so it must fire sometimes and not always
    let mut some = false;
    let mut not_all = false;
    for _ in 0..2000 {
        if check("storm", false, 0.0, false) {
            some = true;
        } else {
            not_all = true;
        }
    }
    assert!(some && not_all, "the turf press must be rare but possible");
}

// L890-894: the barrage auto-throw arms the two-frame release chain and
// keeps its own cadence clock.
#[test]
fn special_ctl_barrage_presses_sub_and_chains_release() {
    use crate::actor::SpecialActive;
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
    a.special_active = Some(Box::new(SpecialActive::new("barrage", "barrage", 5.0)));
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 17, 0);
    let mates = vec![None; 1];
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let mut mv = Vec3::new(0.0, 0.0, 1.0);
    let mut it = ActorInput::default();
    bot.special_ctl(DT, &mut a, &ctx, &mut mv, &mut it, 10.0, true);
    assert!(it.sub, "the barrage presses sub");
    assert!(bot.bomb_aim, "the press stamps the release chain");
    let cd = bot.barrage_cd;
    assert!(
        (0.35 - DT..0.65).contains(&cd),
        "barrageCd {cd} in [0.35, 0.65)"
    );
    // the cadence clock blocks an immediate second press
    let mut it2 = ActorInput::default();
    bot.special_ctl(DT, &mut a, &ctx, &mut mv, &mut it2, 10.0, true);
    assert!(!it2.sub);
    // and the stamp flows through the tail's two-frame release
    let wr = WeaponRunner::new();
    let mut it3 = ActorInput::default();
    bot.tail(
        DT,
        &mut a,
        &ctx,
        &mv,
        0.0,
        -0.4,
        true,
        false,
        &mut it3,
        WeaponKind::Shooter,
        &wr,
    );
    assert!(it3.sub && !it3.squid);
    let mut it4 = ActorInput::default();
    bot.tail(
        DT,
        &mut a,
        &ctx,
        &mv,
        0.0,
        -0.4,
        true,
        false,
        &mut it4,
        WeaponKind::Shooter,
        &wr,
    );
    assert!(!it4.sub, "the barrage throw goes off");
}

// L866-870: no shots and no bombs into the front of an armed Mega Stamp.
#[test]
fn special_ctl_stamp_guard_vetoes_the_front_arc() {
    use crate::actor::SpecialActive;
    use crate::special::stamp;
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let mut a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, 5.0), 0.0);
    let mut foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, 0.0), std::f32::consts::PI);
    let mut sa = SpecialActive::new("stamp", "stamp", 6.0);
    sa.state = KindState::Stamp(stamp::State {
        guard: 1.0,
        body_yaw: 0.0,
        ..Default::default()
    });
    foe.special_active = Some(Box::new(sa));
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 23, 0);
    bot.target = Some((1, 0));
    let mates = vec![None; 2];
    let snap = vec![a.clone(), foe.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let mut mv = Vec3::ZERO;
    // in front of the guard arc (body_yaw 0 = +z, bot at +5 z): vetoed
    let mut it = ActorInput {
        fire: true,
        sub: true,
        ..Default::default()
    };
    bot.special_ctl(DT, &mut a, &ctx, &mut mv, &mut it, 5.0, true);
    assert!(!it.fire && !it.sub, "the guard eats shots and bombs");
    // behind the stamp: dot = −1, well outside cos(1.2) — untouched
    a.pos = vec3(0.0, 0.0, -5.0);
    let snap2 = vec![a.clone(), foe.clone()];
    let ctx2 = ctx_of(&f, &nav, &paint, &sim, &snap2, &mates);
    let mut it2 = ActorInput {
        fire: true,
        sub: true,
        ..Default::default()
    };
    bot.special_ctl(DT, &mut a, &ctx2, &mut mv, &mut it2, 5.0, true);
    assert!(it2.fire && it2.sub, "behind the guard is fair game");
}

// L862-864: cheer a teammate's charging Cheer Orb (a rare rng pulse per
// frame, so run enough frames for one to land).
#[test]
fn special_ctl_cheers_a_teammate_booyah() {
    use crate::actor::SpecialActive;
    use crate::special::booyah;
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
    let mut mate = kid(&f.tuning, 0, 1, vec3(0.0, 0.0, -4.0), 0.0);
    let mut sa = SpecialActive::new("booyah", "booyah", 8.0);
    sa.state = KindState::Booyah(booyah::State::default());
    mate.special_active = Some(Box::new(sa));
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 31, 0);
    let mates = vec![None; 2];
    let snap = vec![a.clone(), mate.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let mut mv = Vec3::ZERO;
    let mut cheered = false;
    for _ in 0..600 {
        let mut it = ActorInput::default();
        bot.special_ctl(DT, &mut a, &ctx, &mut mv, &mut it, 10.0, false);
        cheered |= it.cheer;
    }
    assert!(cheered, "a charging teammate's orb must be cheered");
}

// MAIN_KITS.bow/brolly/mitts `fight` gates (bow.js L453-475, brolly.js
// L560-585, mitts.js L762-816 reach test).
#[test]
fn kit_fight_hooks_gate_by_kit_state() {
    use crate::weapon::brolly::BrollyKit;
    use crate::weapon::mitts::MittsKit;
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
    a.grounded = false;
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, 0.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 41, 0);
    let mates = vec![None; 2];
    let snap = vec![a.clone(), foe.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let mut mv = Vec3::new(1.0, 0.0, 0.0);

    // brolly: an open hold window plants behind the canopy and holds fire
    let mut wr = WeaponRunner::new();
    wr.brolly = Some(Box::new(BrollyKit::new(60.0)));
    bot.t_time = 10.0;
    bot.bb_hold_until = 11.0;
    bot.bb_launch = false;
    assert!(bot.brolly_fight(&a, &ctx, DT, 3.0, 4.0, &mut mv, &wr));
    assert!((mv.x - 0.4).abs() < 1e-5, "the hold plants the stride");
    // expired hold, un-grown canopy: the plain reach test decides
    let mut k = BrollyKit::new(60.0);
    k.grow = 0.5;
    wr.brolly = Some(Box::new(k));
    bot.bb_hold_until = 9.0;
    let mut mv2 = Vec3::new(1.0, 0.0, 0.0);
    assert!(bot.brolly_fight(&a, &ctx, DT, 4.0, 4.0, &mut mv2, &wr));
    assert!(!bot.brolly_fight(&a, &ctx, DT, 5.0, 4.0, &mut mv2, &wr));

    // bow: out of range or on cooldown never starts the draw; in range the
    // hook returns true and arms the scratch (ring1 + 0.08 under 11 m)
    let wrb = WeaponRunner::new();
    let mut it = ActorInput::default();
    assert!(!bot.bow_fight(&a, &ctx, 30.0, 20.0, &mut mv, &mut it, 1, &wrb));
    assert!(bot.bow_fight(&a, &ctx, 8.0, 20.0, &mut mv, &mut it, 1, &wrb));
    let want = f.tuning.weapons.bow.ring1 + 0.08;
    assert!((bot.bow_want - want).abs() < 1e-6, "bow_want {want}");
    assert!(!bot.bow_jumped);
    assert!(bot.bow_fight(&a, &ctx, 12.0, 20.0, &mut mv, &mut it, 1, &wrb));
    assert!(
        (bot.bow_want - 1.0).abs() < 1e-6,
        "beyond 11 m draws to full"
    );

    // mitts: charging / leaping suppresses the punch; else the reach test
    let mut wrm = WeaponRunner::new();
    wrm.mitts = Some(Box::new(MittsKit {
        charging: true,
        ..Default::default()
    }));
    assert!(!bot.mitts_fight(2.0, 3.5, &wrm), "no punch mid-charge");
    wrm.mitts = Some(Box::new(MittsKit::default()));
    assert!(bot.mitts_fight(3.6, 3.5, &wrm));
    assert!(!bot.mitts_fight(4.0, 3.5, &wrm));
}

// MAIN_KITS.blade/bow/mitts `paint` gates (blade.js L726-741, bow.js
// L476-490, mitts.js L817-831).
#[test]
fn kit_paint_hooks_blade_bow_mitts() {
    use crate::weapon::blade::BladeKit;
    use crate::weapon::mitts::MittsKit;
    let f = flat_fix();
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 43, 0);
    let mates = vec![None; 1];
    let snap = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let wr = WeaponRunner::new();

    // blade: quick cuts alternate the press while moving
    bot.bl_wave_t = 3.0;
    assert!(bot.blade_paint(&a, &ctx, DT, true, 0.5, true, &wr));
    assert!(!bot.blade_paint(&a, &ctx, DT, true, 0.5, true, &wr));
    // a full charge looses the wave instead: false + the 3.5..6.5 timer
    let mut wrb = WeaponRunner::new();
    wrb.blade = Some(Box::new(BladeKit {
        charging: true,
        full: true,
        ..Default::default()
    }));
    assert!(!bot.blade_paint(&a, &ctx, DT, true, 0.5, true, &wrb));
    assert!(
        (3.5..6.5).contains(&bot.bl_wave_t),
        "wave timer rearmed: {}",
        bot.bl_wave_t
    );
    // still charging, not full: hold the cut
    wrb.blade = Some(Box::new(BladeKit {
        charging: true,
        full: false,
        ..Default::default()
    }));
    assert!(bot.blade_paint(&a, &ctx, DT, true, 0.5, true, &wrb));

    // bow: the four-condition gate, then the draw starts
    assert!(!bot.bow_paint(&a, &ctx, false, 0.9, &wr));
    assert!(!bot.bow_paint(&a, &ctx, true, 0.1, &wr));
    bot.no_prog = 0.5;
    assert!(!bot.bow_paint(&a, &ctx, true, 0.9, &wr), "stuck: hold off");
    bot.no_prog = 0.0;
    assert!(bot.bow_paint(&a, &ctx, true, 0.9, &wr));
    assert!(bot.bow_paint > 0.0, "the draw target is armed");

    // mitts: charging kills the punch; else ink + (need || roll) + move
    let mut wrm = WeaponRunner::new();
    wrm.mitts = Some(Box::new(MittsKit {
        leaping: true,
        ..Default::default()
    }));
    assert!(!bot.mitts_paint(true, 0.9, true, &wrm));
    wrm.mitts = Some(Box::new(MittsKit::default()));
    assert!(bot.mitts_paint(true, 0.9, true, &wrm));
    assert!(!bot.mitts_paint(true, 0.05, true, &wrm), "ink reserve");
    assert!(!bot.mitts_paint(true, 0.9, false, &wrm), "must be moving");
}

// blade.js L710-725: stayIn at blade reach (full charge widens it) and the
// target-bias arms through the teammate target memory.
#[test]
fn blade_stay_in_and_target_bias_arms() {
    use crate::weapon::blade::BladeKit;
    let mut f = flat_fix();
    f.kinds = vec![WeaponKind::Blade, WeaponKind::Shooter];
    let nav = NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    );
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let a = kid(&f.tuning, 0, 0, vec3(0.0, 0.0, -5.0), 0.0);
    let mut foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, -1.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 47, 0);
    bot.target = Some((1, 0));
    let mates = vec![None; 2];
    let snap = vec![a.clone(), foe.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);
    let wr = WeaponRunner::new();
    // stayIn: 4 m flat → stay; 5 m needs the full charge
    assert!(bot.blade_stay_in(&a, &ctx, &wr));
    foe.pos = vec3(0.0, 0.0, 0.0);
    let snap2 = vec![a.clone(), foe.clone()];
    let ctx2 = ctx_of(&f, &nav, &paint, &sim, &snap2, &mates);
    assert!(!bot.blade_stay_in(&a, &ctx2, &wr));
    let mut wrf = WeaponRunner::new();
    wrf.blade = Some(Box::new(BladeKit {
        full: true,
        ..Default::default()
    }));
    assert!(
        bot.blade_stay_in(&a, &ctx2, &wrf),
        "charged: 5 m still holds"
    );
    // a height difference over 1.4 m breaks it
    foe.pos = vec3(0.0, 2.0, 0.0);
    let snap3 = vec![a.clone(), foe.clone()];
    let ctx3 = ctx_of(&f, &nav, &paint, &sim, &snap3, &mates);
    assert!(!bot.blade_stay_in(&a, &ctx3, &wrf));

    // targetBias: hurt at 10 m = −2 with no teammate memory
    foe.hp = 40.0;
    foe.pos = vec3(0.0, 0.0, 5.0);
    let snap4 = vec![a.clone(), foe.clone()];
    let ctx4 = ctx_of(&f, &nav, &paint, &sim, &snap4, &mates);
    assert!(
        (bot.blade_target_bias(&a, &ctx4, 1, 10.0) + 2.0).abs() < 1e-6,
        "hp term"
    );
    // a teammate already fighting that foe: −4 more (the kill it shares)
    f.mate_minds = vec![
        None,
        Some(MateMind {
            target: Some((0, 1)),
            see_timer: 1.0,
        }),
    ];
    let ctx5 = ctx_of(&f, &nav, &paint, &sim, &snap4, &mates);
    assert!(
        (bot.blade_target_bias(&a, &ctx5, 1, 10.0) + 6.0).abs() < 1e-6,
        "shared-kill term"
    );
    // that foe has *me* spotted from beyond blade reach — but JS patches
    // `MELEE.blade = true` (blade.js L35), so for a Cutlass owner the +4
    // lost-duel arm is dead code and the bias stays 0
    let d_far = f.tuning.weapons.blade.range + 2.0;
    f.mate_minds = vec![
        None,
        Some(MateMind {
            target: Some((0, 0)),
            see_timer: 1.0,
        }),
    ];
    foe.hp = 100.0;
    let snap6 = vec![a.clone(), foe.clone()];
    let ctx6 = ctx_of(&f, &nav, &paint, &sim, &snap6, &mates);
    assert!(
        bot.blade_target_bias(&a, &ctx6, 1, d_far).abs() < 1e-6,
        "MELEE.blade (blade.js L35): +4 arm never fires for the Cutlass"
    );
    // no teammate memory (a remote brain): only the hp term survives
    f.mate_minds = vec![None, None];
    foe.hp = 40.0;
    let snap7 = vec![a.clone(), foe.clone()];
    let ctx7 = ctx_of(&f, &nav, &paint, &sim, &snap7, &mates);
    assert!(
        (bot.blade_target_bias(&a, &ctx7, 1, 10.0) + 2.0).abs() < 1e-6,
        "remote brain: hp term only"
    );
}

// SUB_KITS torpedo/boomerang/waddle fight gates (torpedo.js L738-744,
// boomerang.js L572-583, waddle.js L751-755).
#[test]
fn kit_sub_fight_range_gates_and_boomerang_chain() {
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
    a.ink = 200.0;
    let foe = kid(&f.tuning, 1, 0, vec3(0.0, 0.0, 0.0), std::f32::consts::PI);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 53, 0);
    let mates = vec![None; 2];
    let snap = vec![a.clone(), foe.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &snap, &mates);

    // torpedo: 4..13.5 m only, then the roll arms 6..11
    a.sub = SubKind::Torpedo;
    assert!(!bot.fight_sub(&a, &ctx, 2.0), "too close for the torpedo");
    let mut threw = false;
    for _ in 0..5000 {
        if bot.fight_sub(&a, &ctx, 8.0) {
            threw = true;
            break;
        }
    }
    assert!(threw, "the torpedo must fire in range");
    assert!(
        (6.0..11.0).contains(&bot.bomb_cd.expect("torpedo cd")),
        "torpedo bombCd {}",
        bot.bomb_cd.unwrap()
    );

    // boomerang: needs a seen target with the aim on it (L575-580)
    a.sub = SubKind::Boomerang;
    bot.target = None;
    for _ in 0..200 {
        assert!(!bot.fight_sub(&a, &ctx, 5.0), "no target, no boomerang");
    }
    bot.target = Some((1, 0));
    bot.see_timer = 0.0;
    for _ in 0..200 {
        assert!(!bot.fight_sub(&a, &ctx, 5.0), "unseen target, no throw");
    }
    bot.see_timer = 1.0;
    a.aim_yaw = 0.0; // foe straight ahead (+z), off = 0
    let mut threw = false;
    for _ in 0..5000 {
        if bot.fight_sub(&a, &ctx, 5.0) {
            threw = true;
            break;
        }
    }
    assert!(threw, "the aimed boomerang must go");
    assert!(
        (6.0..11.0).contains(&bot.bomb_cd.expect("boomerang cd")),
        "boomerang bombCd {}",
        bot.bomb_cd.unwrap()
    );

    // waddle: the lob window is 4.5..15 m
    a.sub = SubKind::Waddle;
    assert!(!bot.fight_sub(&a, &ctx, 2.0));
}

// L557 + MAIN_KITS paintPitch: the kit aim-pitch ladder settles the spring.
#[test]
fn paint_pitch_kit_ladder() {
    for (kind, want) in [
        (WeaponKind::Brolly, -0.36),
        (WeaponKind::Blade, -0.2),
        (WeaponKind::Bow, -0.18),
        (WeaponKind::Mitts, -0.32),
    ] {
        let p = paint_pitch_of(kind);
        assert!(
            (p - want).abs() < 0.03,
            "{kind:?} paint pitch must settle at {want}, got {p}"
        );
    }
}

// ------------------------------------------------ TR-38.2 zone equipment matrix

/// All fifteen `SUBS` kinds (config.js L319+, `SUB_ORDER`, tuning.rs L30-32).
const SUB_ALL: [SubKind; 15] = [
    SubKind::Bomb,
    SubKind::Sticky,
    SubKind::Burst,
    SubKind::Seeker,
    SubKind::Scan,
    SubKind::Curtain,
    SubKind::Sprinkler,
    SubKind::Mine,
    SubKind::Beacon,
    SubKind::Mist,
    SubKind::Shaker,
    SubKind::Waddle,
    SubKind::Torpedo,
    SubKind::Tracer,
    SubKind::Boomerang,
];

/// Shared zones fixture for the matrix (the nav graph is the expensive
/// part — built once per test, not once per run).
struct ZoneMatrix {
    layout: crate::geometry::StageLayout,
    world: CollisionWorld,
    tuning: Tuning,
    nav: NavGraph,
}

fn zone_matrix() -> ZoneMatrix {
    let layout = crate::embedded_tidewater();
    let world = CollisionWorld::from_layout_for(&layout, "zones");
    let tuning = crate::embedded_tuning();
    let nav = NavGraph::new(&world, &layout, &tuning.player);
    ZoneMatrix {
        layout,
        world,
        tuning,
        nav,
    }
}

/// Drive a 2v2 Zone Control match on embedded Tidewater for `frames`
/// *playing* frames with the given per-slot loadouts (Task 38 batch 3,
/// TR-38.2). The intro window is skipped by entering `Active` directly —
/// the zone clock only runs while playing, so the fixture shortcut costs
/// nothing the zone arms care about. Ink and the special gauge are topped
/// up every frame so the sub / special / zone-special arms all get exercised
/// (JS keeps a live gauge through normal play; the matrix wants coverage,
/// not economy).
fn zone_matrix_run(
    fx: &ZoneMatrix,
    seed: u64,
    loadouts: &[(WeaponKind, SubKind, &'static str)],
    frames: usize,
) {
    let layout = &fx.layout;
    let tuning = &fx.tuning;
    let nav = &fx.nav;
    let sim = SimWorld::new(&fx.world, layout);
    let n = loadouts.len();
    let mut m = crate::match_::Match::with_roster_mode(
        120.0,
        seed,
        &sim,
        &tuning.player,
        &tuning.match_config,
        n / 2,
        crate::match_::MatchMode::Zones,
        layout.zones.as_ref(),
        Some(tuning.zones),
    );
    assert!(m.zones.is_some(), "zones mode must be live");
    m.kinds = loadouts.iter().map(|l| l.0).collect();
    for (a, l) in m.actors.iter_mut().zip(loadouts) {
        a.sub = l.1;
        a.set_special(l.2);
    }
    // Skip the intro: the brains only run in `Active` anyway.
    m.phase = crate::match_::Phase::Active;
    let mut bots: Vec<Bot> = m
        .actors
        .iter()
        .enumerate()
        .map(|(i, a)| Bot::new(a, &tuning.difficulty.easy, seed, i))
        .collect();
    let mut inputs = vec![ActorInput::default(); n];
    let mut mate_goals = vec![None; n];
    let mut zone_plan: Option<ZonePlan> = None;
    for _ in 0..frames {
        // Top the economy up so every arm fires (module docs of `bot::zones`).
        for a in m.actors.iter_mut() {
            if a.alive {
                a.ink = tuning.player.ink_max;
                a.special = a.special_cost;
            }
        }
        let snap = m.actors.clone();
        for (i, b) in bots.iter().enumerate() {
            mate_goals[i] = b.goal_node();
        }
        let mate_minds = bots.iter().map(|b| Some(b.mind())).collect::<Vec<_>>();
        let mut zp = zone_plan.take();
        if let Some(zctl) = m.zones.as_ref() {
            let mut p = zp
                .take()
                .unwrap_or_else(|| ZonePlan::build(zctl, nav, &m.paint, &sim));
            p.tick(zctl, &m.paint, &sim, &snap, &m.kinds, &vec![true; n]);
            zp = Some(p);
        }
        zone_plan = zp;
        for i in 0..n {
            let ctx = BotCtx {
                actors: &snap,
                nav,
                paint: &m.paint,
                world: &sim,
                t: &tuning.player,
                weapons: &tuning.weapons,
                weapon_range: &tuning.weapon_range,
                kinds: &m.kinds,
                runners: &m.runners,
                mate_goals: &mate_goals,
                subs: &m.subs,
                mate_minds: &mate_minds,
                zones: zone_plan.as_ref(),
                zctl: m.zones.as_ref(),
            };
            inputs[i] = bots[i].step(DT, &mut m.actors[i], &ctx);
        }
        m.step(
            DT,
            &inputs,
            &sim,
            &tuning.player,
            &tuning.weapons,
            &tuning.subs,
        );
    }
}

/// TR-38.2 ①: every weapon kind, held by every bot of a 2v2 zones match,
/// survives 120 playing frames with the full zone arms live (no panic, no
/// divergence). `WeaponKind::ALL` (weapon.rs L194) — the 12 roster kinds.
#[test]
fn tr38_zone_bots_survive_every_weapon_kind() {
    let fx = zone_matrix();
    for (ki, kind) in WeaponKind::ALL.iter().copied().enumerate() {
        zone_matrix_run(
            &fx,
            0x38_0000 + ki as u64,
            &[(kind, SubKind::Bomb, "storm"); 4],
            120,
        );
    }
}

/// TR-38.2 ②: the 15 subs × 19 specials equipment matrix — four distinct
/// loadouts share one zones match, so 72 runs cover all 285 pairs, each held
/// by a live bot for 120 playing frames (no panic).
#[test]
fn tr38_zone_bots_survive_the_sub_special_matrix() {
    let fx = zone_matrix();
    let combos: Vec<(SubKind, &'static str)> = SUB_ALL
        .iter()
        .flat_map(|s| {
            crate::net::session::SPECIAL_IDS
                .iter()
                .map(move |sp| (*s, *sp))
        })
        .collect();
    assert_eq!(combos.len(), 15 * 19);
    for (ci, chunk) in combos.chunks(4).enumerate() {
        let mut loadouts = Vec::new();
        for (sub, sp) in chunk {
            loadouts.push((WeaponKind::Shooter, *sub, *sp));
        }
        while loadouts.len() < 4 {
            let l = loadouts[0];
            loadouts.push(l);
        }
        zone_matrix_run(&fx, 0x38_8000 + ci as u64, &loadouts, 120);
    }
}
