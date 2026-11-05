//! TR-47.1 — fixed-seed behaviour tests for the Task 47 threat AI
//! (`bot/threat.rs`, bots.js L1260-1615) and the batch-C shield / canopy /
//! stamp hooks.
//!
//! Parallelism note: [`set_threat_ai`] flips a process-wide `AtomicBool` and
//! [`threat_stats`] is a thread-local tally, so every test here takes the
//! file lock, pins the switch ON (the A/B test toggles it *inside* one test
//! body, sequentially), and reads its own thread's counters.

use super::*;
use crate::actor::{SimWorld, SpecialActive};
use crate::collision::CollisionWorld;
use crate::geometry::{Bounds, Brush, BrushCommon, Mural, StageLayout};
use crate::nav::NavGraph;
use crate::paint::PaintGrid;
use crate::sub::waddle::WaddleState;
use crate::sub::{SubSystem, ThreatKind};
use crate::tuning::Tuning;
use crate::weapon::ProjectileSim;
use glam::vec3;
use std::sync::{Mutex, MutexGuard};

const DT: f32 = crate::actor::FIXED_DT;

static LOCK: Mutex<()> = Mutex::new(());

/// Serialise this file's tests around the process-wide A/B switch and pin it
/// ON (the A/B test owns the flag for its whole body).
fn begin() -> MutexGuard<'static, ()> {
    let g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_threat_ai(true);
    reset_threat_stats();
    g
}

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
    layout: StageLayout,
    tuning: Tuning,
    subs: SubSystem,
    kinds: Vec<WeaponKind>,
    runners: Vec<WeaponRunner>,
    mate_minds: Vec<Option<crate::bot::MateMind>>,
}

fn layout_of(primitives: Vec<Brush>) -> StageLayout {
    let stage = crate::geometry::StageInfo {
        id: "test".into(),
        bounds: B_SMALL,
        spawn_pads: vec![[0.0, 0.0, -5.0], [0.0, 0.0, 5.0]],
        spawn_barrier: 4.2,
        water_y: -1.6,
    };
    let meta = crate::geometry::LayoutMeta {
        primitive_counts: crate::geometry::PrimitiveCounts {
            boxes: primitives.len() as u32,
            obox: 0,
            ramp: 0,
            total: primitives.len() as u32,
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
    StageLayout {
        schema: "test".into(),
        source: crate::tuning::Source {
            commit: "test".into(),
            path: "test".into(),
        },
        stage,
        primitives,
        zones: None,
        meta,
    }
}

fn fix_with(primitives: Vec<Brush>) -> Fix {
    let world = CollisionWorld::from_brushes(&primitives, B_SMALL);
    Fix {
        world,
        layout: layout_of(primitives),
        tuning: crate::embedded_tuning(),
        subs: SubSystem::new(0),
        kinds: Vec::new(),
        runners: Vec::new(),
        mate_minds: Vec::new(),
    }
}

/// A 20×20 m flat deck centred on the origin (same shape as the smoke
/// fixture in `bot/tests.rs`).
fn flat_fix() -> Fix {
    fix_with(vec![bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])])
}

/// Deck with a hole (no ground = wet, `Bot::wet`) over `x ∈ [1.5, 10],
/// z ∈ [-2, 2]` — the `pick_evade` dry-preference scenario.
fn hole_fix() -> Fix {
    fix_with(vec![
        bx([-10.0, -1.0, -10.0], [10.0, 0.0, -2.0]),
        bx([-10.0, -1.0, 2.0], [10.0, 0.0, 10.0]),
        bx([-10.0, -1.0, -2.0], [1.5, 0.0, 2.0]),
    ])
}

/// Deck whose only ground at `z > 0` is `x ≤ -4.7`, `x ≥ 4.7` and the middle
/// strip `|x| ≤ 1.5` — the launched-canopy edge points `(±(w+0.55), z > 0)`
/// (≈ ±1.92 m, bots.js L1581-1586) both fall into the side holes.
fn pinched_fix() -> Fix {
    fix_with(vec![
        bx([-10.0, -1.0, -10.0], [10.0, 0.0, 0.0]),
        bx([-10.0, -1.0, 0.0], [-4.7, 0.0, 10.0]),
        bx([4.7, -1.0, 0.0], [10.0, 0.0, 10.0]),
        bx([-1.5, -1.0, 0.0], [1.5, 0.0, 10.0]),
    ])
}

fn kid(t: &Tuning, team: usize, slot: usize, pos: Vec3, yaw: f32) -> Actor {
    let mut a = Actor::new(team, slot, &t.player);
    a.pos = pos;
    a.yaw = yaw;
    a.aim_yaw = yaw;
    a
}

fn flat_nav(f: &Fix) -> NavGraph {
    NavGraph::without_layout(
        &f.world,
        [vec3(0.0, 0.0, -5.0), vec3(0.0, 0.0, 5.0)],
        4.2,
        &f.tuning.player,
    )
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
        boss: None,
        launched: &[],
        time: 0.0,
    }
}

/// A synthetic waddle view (the `threat_score` probes never touch the live
/// `SubSystem`).
fn wv(
    pos: Vec3,
    state: ThreatState,
    ground: bool,
    locked: bool,
    target: Option<usize>,
    left: Option<f32>,
) -> ThreatView {
    ThreatView {
        kind: ThreatKind::Waddle,
        owner_idx: 1,
        gid: 7,
        team: 1,
        pos,
        aim_y: 0.6,
        radius: 3.0,
        trigger: 1.2,
        speed: 4.0,
        sense_radius: Some(7.5),
        lock_range: None,
        ground,
        state,
        shootable: true,
        locked,
        target,
        vel: Vec3::ZERO,
        left,
    }
}

// ------------------------------------------------------------------ score

/// JS `_threatScore` (bots.js L1292-1326): every branch on a fixed synthetic
/// view set — walker in / out of the sense radius, the fly-phase low score, a
/// locked-on-me walker's eta score, the trigger-distance spike, the dying
/// (`left`) fade, and the far / high rejection.
#[test]
fn tr471_threat_score_branches() {
    let _g = begin();
    let f = flat_fix();
    let nav = flat_nav(&f);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let bot_a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let actors = vec![bot_a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&bot_a, &f.tuning.difficulty.hard, 11, 0);
    bot.t_time = 0.0;

    // JS L1307-1310: a ground sensor inside its sense radius scores 0.7.
    let d = wv(
        Vec3::new(0.0, 0.0, 5.0),
        ThreatState::Sense,
        true,
        false,
        None,
        None,
    );
    assert!(
        (bot.threat_score(&bot_a, &ctx, &d) - 0.7).abs() < 1e-6,
        "walker in sense radius = 0.7"
    );
    // JS L1310: outside the sense radius it is not worth a reaction yet.
    let d = wv(
        Vec3::new(0.0, 0.0, 12.0),
        ThreatState::Sense,
        true,
        false,
        None,
        None,
    );
    assert_eq!(
        bot.threat_score(&bot_a, &ctx, &d),
        0.0,
        "walker out of sense radius = 0"
    );

    // JS L1300-1305: a flyer still inbound scores low (it may not land on us).
    let mut d = wv(
        Vec3::new(0.0, 0.0, 5.0),
        ThreatState::Fly,
        false,
        false,
        None,
        None,
    );
    d.lock_range = Some(6.5);
    d.vel = Vec3::new(0.0, -2.0, -3.0); // moving toward us: dz·vz < 0
    assert!(
        (bot.threat_score(&bot_a, &ctx, &d) - 0.5).abs() < 1e-6,
        "fly phase = 0.5"
    );

    // JS L1318-1320: a locked walker aimed at ME dominates everything.
    let d = wv(
        Vec3::new(0.0, 0.0, 3.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        None,
    );
    let s = bot.threat_score(&bot_a, &ctx, &d);
    let want = 4.0 / (0.5 + (3.0 - 1.2) / 4.0);
    assert!(
        (s - want).abs() < 1e-4,
        "locked-on-me eta score {want}, got {s}"
    );
    assert!(s > 3.5, "locked-on-me is the top score, got {s}");

    // JS L1321: the trigger-distance spike (not aimed at us, but on top of us).
    let d = wv(
        Vec3::new(0.0, 0.0, 3.0),
        ThreatState::Walk,
        true,
        true,
        None,
        None,
    );
    assert!(
        (bot.threat_score(&bot_a, &ctx, &d) - 1.7).abs() < 1e-6,
        "trigger distance = 1.7"
    );

    // JS L1318-1320: a walker about to die out (`left < eta·0.8`) fades to 30%.
    let d = wv(
        Vec3::new(0.0, 0.0, 3.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        Some(0.1),
    );
    let s = bot.threat_score(&bot_a, &ctx, &d);
    assert!(
        (s - want * 0.3).abs() < 1e-4,
        "dying locked-on-me fades, got {s}"
    );
    // ... while a fresh one keeps the full score.
    let d = wv(
        Vec3::new(0.0, 0.0, 3.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        Some(8.0),
    );
    let s2 = bot.threat_score(&bot_a, &ctx, &d);
    assert!(
        (s2 - want).abs() < 1e-4 && s2 > 3.5,
        "full-life locked-on-me keeps the score, got {s2}"
    );

    // JS L1294: too far / too high is never a threat.
    let d = wv(
        Vec3::new(0.0, 0.0, 19.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        None,
    );
    assert_eq!(bot.threat_score(&bot_a, &ctx, &d), 0.0, "dh > 18 = 0");
    let d = wv(
        Vec3::new(0.0, 7.0, 3.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        None,
    );
    assert_eq!(bot.threat_score(&bot_a, &ctx, &d), 0.0, "dy > 6 = 0");
}

/// JS `_threatScan` (bots.js L1272-1290): own-team devices and dead ones are
/// never candidates; a hostile locked walker is, and noticing it bumps the
/// counter exactly once (the reaction gate keys on the ref).
#[test]
fn tr471_threat_scan_team_and_dead_filter() {
    let _g = begin();
    let mut f = flat_fix();
    let mut ps = ProjectileSim::new(3);
    let mut paint = PaintGrid::new(&f.world);
    let mut actors = vec![
        kid(&f.tuning, 0, 0, Vec3::new(0.0, 0.0, -5.0), 0.0),
        kid(
            &f.tuning,
            1,
            1,
            Vec3::new(0.0, 0.0, 3.0),
            std::f32::consts::PI,
        ),
    ];
    actors[1].sub = crate::sub::SubKind::Waddle;
    actors[1].ink = 60.0;
    f.subs.use_sub(
        1,
        1,
        &f.world,
        &mut actors,
        &mut paint,
        &mut ps,
        &f.tuning.player,
        &f.tuning.subs,
        None,
        0.0,
    );
    {
        let items = f.subs.waddle_items_mut();
        assert_eq!(items.len(), 1, "the waddle spawned");
        let it = &mut items[0];
        it.state = WaddleState::Walk;
        it.pos = Vec3::new(0.0, 0.0, -1.0);
        it.air = false;
        it.target = Some(0);
        it.heading = std::f32::consts::PI;
    }
    drop(paint);
    let nav = flat_nav(&f);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let bot_a = actors[0].clone();
    let mut bot = Bot::new(&bot_a, &f.tuning.difficulty.hard, 12, 0);
    bot.t_time = 0.0;

    bot.threat_scan(&bot_a, &ctx_of(&f, &nav, &paint, &sim, &actors, &[]));
    assert!(bot.thr_cand.is_some(), "hostile walker is a candidate");
    assert_eq!(threat_stats().noticed, 1, "noticed once");
    bot.threat_scan(&bot_a, &ctx_of(&f, &nav, &paint, &sim, &actors, &[]));
    assert_eq!(threat_stats().noticed, 1, "no re-notice of the same device");

    // Own team: JS L1276 `if (d.team === a.team) continue`.
    f.subs.waddle_items_mut()[0].team = 0;
    bot.threat_scan(&bot_a, &ctx_of(&f, &nav, &paint, &sim, &actors, &[]));
    assert!(bot.thr_cand.is_none(), "own-team device filtered");

    // Dead / ghost: `threats()` skips `state === 'dead'` (JS L1274).
    let items = f.subs.waddle_items_mut();
    items[0].team = 1;
    items[0].state = WaddleState::Dead;
    bot.threat_scan(&bot_a, &ctx_of(&f, &nav, &paint, &sim, &actors, &[]));
    assert!(bot.thr_cand.is_none(), "dead device filtered");
    assert!(bot.thr_mem.is_empty(), "dead devices leave the memory");
}

// ------------------------------------------------------------------ evade

/// JS `_pickEvade` (bots.js L1457-1474) + `_dryLine` (L779-783): with a wet
/// hole straight away from the threat, the chosen heading is a dry one — the
/// `off = 0` candidate is rejected and a ±1.0 rad offset wins.
#[test]
fn tr471_pick_evade_prefers_dry_ground() {
    let _g = begin();
    let f = hole_fix();
    let nav = NavGraph::new(&f.world, &f.layout, &f.tuning.player);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let bot_a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let actors = vec![bot_a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&bot_a, &f.tuning.difficulty.hard, 13, 0);

    // JS L779-783: the +x ray crosses the hole, the +z ray stays on deck.
    assert!(
        !bot.dry_line(&ctx, 0.0, 0.0, 0.0, 3.2, 0.0),
        "dry_line rejects the hole"
    );
    assert!(
        bot.dry_line(&ctx, 0.0, 0.0, 0.0, 0.0, 3.2),
        "dry_line accepts solid deck"
    );

    // Threat to the west → base heading is +x (straight into the hole).
    let d = wv(
        Vec3::new(-3.0, 0.0, 0.0),
        ThreatState::Walk,
        true,
        true,
        Some(0),
        None,
    );
    let yaw = bot.pick_evade(&bot_a, &ctx, &d);
    let px = yaw.sin() * 3.2;
    let pz = yaw.cos() * 3.2;
    assert!(
        bot.dry_line(&ctx, 0.0, 0.0, 0.0, px, pz),
        "the chosen evade heads to dry ground ({yaw})"
    );
    let off = angle_diff(yaw, std::f32::consts::FRAC_PI_2);
    assert!(
        (off - 1.0).abs() < 0.05 || (off + 1.0).abs() < 0.05,
        "the ±1.0 rad offsets are the only dry high-scoring picks, got off {off}"
    );
}

// ------------------------------------------------------------------ trigger

/// JS `_devTrigger` (bots.js L1487-1502): the stream branch (`aimed && in
/// window`), the melee pulse latch (L1491-1494), and the charge gates (L1495-
/// 1501: burst/stream veto, bail early on a short charge, cooldown veto).
#[test]
fn tr471_dev_trigger_branches() {
    let _g = begin();
    let f = flat_fix();
    let nav = flat_nav(&f);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let actors = vec![a.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.hard, 14, 0);
    let idle = WeaponRunner::new();

    // JS L1502: the plain stream — aimed and inside the window, both gates.
    assert!(
        bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Shooter,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "shooter aimed in window fires"
    );
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Shooter,
            5.0,
            false,
            true,
            0.0,
            10.0
        ),
        "not aimed: hold"
    );
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Shooter,
            15.0,
            true,
            true,
            0.0,
            10.0
        ),
        "out of window: hold"
    );

    // JS L1491-1494: the melee pulse — one shot per wind-up, latched until
    // the aim leaves the window.
    bot.thr_pulse = false;
    assert!(
        bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Blade,
            2.0,
            true,
            true,
            0.0,
            4.0
        ),
        "blade pulse on the rising edge"
    );
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Blade,
            2.0,
            true,
            true,
            0.0,
            4.0
        ),
        "pulse latches while still in the window"
    );
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Blade,
            2.0,
            false,
            true,
            0.0,
            4.0
        ),
        "losing the aim clears the pulse"
    );
    assert!(
        bot.dev_trigger(
            &a,
            &ctx,
            &idle,
            WeaponKind::Blade,
            2.0,
            true,
            true,
            0.0,
            4.0
        ),
        "and a fresh rising edge fires again"
    );

    // JS L1495-1496: an already flying burst / stream vetoes.
    let wr = WeaponRunner::test_charge(false, 0.0, 0.0, 0.3, false);
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "burst_t > 0 vetoes"
    );
    let wr = WeaponRunner::test_charge(false, 0.0, 0.0, 0.0, true);
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "streaming vetoes"
    );
    // JS L1500-1501: mid-charge — release early on a short charge, hold a
    // full one.
    let wr = WeaponRunner::test_charge(true, 0.05, 0.0, 0.0, false);
    assert!(
        bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "a short charge bails out (release)"
    );
    let wr = WeaponRunner::test_charge(true, 0.3, 0.0, 0.0, false);
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "a full charge keeps holding"
    );
    // JS L1502: not charging — roughly aimed, in window, off cooldown.
    let wr = WeaponRunner::test_charge(false, 0.0, 0.5, 0.0, false);
    assert!(
        !bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "cooldown vetoes"
    );
    let wr = WeaponRunner::test_charge(false, 0.0, 0.0, 0.0, false);
    assert!(
        bot.dev_trigger(
            &a,
            &ctx,
            &wr,
            WeaponKind::Charger,
            5.0,
            true,
            true,
            0.0,
            10.0
        ),
        "charge release on the window"
    );
}

// ------------------------------------------------------------------ shields

#[allow(clippy::too_many_arguments)]
fn shield(
    r#ref: ShieldRef,
    held: bool,
    team: usize,
    c: Vec3,
    n: Vec3,
    r: f32,
    hp: f32,
    left: f32,
    blocks: bool,
) -> ShieldView {
    ShieldView {
        r#ref,
        held,
        team,
        c,
        n,
        r,
        pos: c,
        half_w: r,
        plane_off: 0.0,
        speed: 0.0,
        hp,
        left,
        live: true,
        blocks_actors: blocks,
    }
}

/// JS `_shieldFight` (bots.js L1507-1549): a held enemy canopy between us and
/// our duel target rewrites the strafe to flank round it and, past the 0.15 s
/// block timer, holds the shots that would only ink the canopy — until the
/// through-gate (a canopy we can actually pop) lets them fly again.
#[test]
fn tr471_shield_fight_flanks_and_holds_fire() {
    let _g = begin();
    let f = flat_fix();
    let nav = flat_nav(&f);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let t = kid(
        &f.tuning,
        1,
        1,
        Vec3::new(0.0, 0.0, 8.0),
        std::f32::consts::PI,
    );
    let actors = vec![a.clone(), t.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.hard, 15, 0);
    // The canopy held over the duel line, facing us.
    bot.shl.push(shield(
        ShieldRef::Held(1),
        true,
        1,
        Vec3::new(0.0, 1.0, 4.0),
        Vec3::new(0.0, 0.0, -1.0),
        HELD_R,
        1.0e6, // too tough to pop: the hold-fire branch decides, not `through`
        0.0,
        true,
    ));
    let wr = WeaponRunner::new();

    let mut mv = Vec3::new(0.0, 0.0, 1.0);
    let mut it = ActorInput {
        fire: true,
        ..Default::default()
    };
    bot.shield_fight(
        &a,
        &ctx,
        &t,
        8.0,
        &mut mv,
        &mut it,
        DT,
        WeaponKind::Shooter,
        &wr,
    );

    // JS L1527: first blocked frame counts the flank once.
    assert_eq!(
        threat_stats().flank,
        1,
        "the flank is counted once per block"
    );
    // JS L1528-1536: the strafe is rewritten along the shield's slide axis.
    assert!(mv.x.abs() > 0.8, "flank move is lateral, got {mv}");
    assert!(bot.block_t > 0.0 && bot.block_t < DT + 1e-6);
    assert!(it.fire, "under 0.15 s the shots still fly");

    // Past the 0.15 s block timer the shots are held (JS L1537-1544).
    bot.block_t = 0.16;
    for _ in 0..3 {
        let mut it = ActorInput {
            fire: true,
            ..Default::default()
        };
        let mut mv = Vec3::new(0.0, 0.0, 1.0);
        bot.shield_fight(
            &a,
            &ctx,
            &t,
            8.0,
            &mut mv,
            &mut it,
            DT,
            WeaponKind::Shooter,
            &wr,
        );
        assert!(!it.fire, "blocked shots are suppressed");
    }
    assert!(
        threat_stats().hold_fire > 0.03,
        "hold_fire tallied, got {}",
        threat_stats().hold_fire
    );
    assert_eq!(
        threat_stats().flank,
        1,
        "no re-count while the block continues"
    );

    // JS L1537-1539: a canopy we can pop in one volley is shot through.
    bot.shl.clear();
    bot.shl.push(shield(
        ShieldRef::Held(1),
        true,
        1,
        Vec3::new(0.0, 1.0, 4.0),
        Vec3::new(0.0, 0.0, -1.0),
        HELD_R,
        1.0, // hp <= dps * 1.3 → through
        0.0,
        true,
    ));
    let mut it = ActorInput {
        fire: true,
        ..Default::default()
    };
    let mut mv = Vec3::new(0.0, 0.0, 1.0);
    bot.shield_fight(
        &a,
        &ctx,
        &t,
        8.0,
        &mut mv,
        &mut it,
        DT,
        WeaponKind::Shooter,
        &wr,
    );
    assert!(it.fire, "a poppable canopy does not hold the shots");
}

/// JS `_canopyCtl` (bots.js L1554-1615): a launched canopy across our line of
/// travel boxes us in — steer to the nearer walkable edge (steer counter +
/// lateral rewrite), and when both edges are wet (the pinched world) break it
/// instead (break_wall + fire + back-off move).
#[test]
fn tr471_canopy_ctl_sidesteps_and_breaks() {
    let _g = begin();
    // (a) flat deck: the nearer edge is walkable → steer.
    let f = flat_fix();
    let nav = flat_nav(&f);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let t = kid(
        &f.tuning,
        1,
        1,
        Vec3::new(0.0, 0.0, 8.0),
        std::f32::consts::PI,
    );
    let actors = vec![a.clone(), t.clone()];
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.hard, 16, 0);
    bot.shl.push(shield(
        ShieldRef::Launched(0),
        false,
        1,
        Vec3::new(0.0, 0.9, 2.4),
        Vec3::new(0.0, 0.0, 1.0),
        LAUNCH_R,
        40.0,
        9.0,
        true,
    ));
    let mut mv = Vec3::new(0.0, 0.0, 1.0);
    let mut it = ActorInput::default();
    let hold = bot.canopy_ctl(DT, &a, &ctx, &mut it, &mut mv, true);
    assert!(hold.is_none(), "steering does not take the aim");
    assert_eq!(threat_stats().steer, 1, "a new canopy counts one steer");
    assert_eq!(bot.can_side, 1, "the preferred edge is walkable");
    assert!(
        mv.x.abs() > 0.7 && mv.x < 0.0,
        "the move is rewritten toward the edge, got {mv}"
    );

    // (b) both edges over the holes: boxed in → break it (JS L1588-1598).
    let f = pinched_fix();
    let nav = NavGraph::new(&f.world, &f.layout, &f.tuning.player);
    let paint = PaintGrid::new(&f.world);
    let sim = SimWorld::new(&f.world, &f.layout);
    let ctx = ctx_of(&f, &nav, &paint, &sim, &actors, &[]);
    let mut bot = Bot::new(&a, &f.tuning.difficulty.hard, 17, 0);
    bot.shl.push(shield(
        ShieldRef::Launched(0),
        false,
        1,
        Vec3::new(0.0, 0.9, 2.4),
        Vec3::new(0.0, 0.0, 1.0),
        LAUNCH_R,
        40.0,
        9.0,
        true,
    ));
    let mut mv = Vec3::new(0.0, 0.0, 1.0);
    let mut it = ActorInput::default();
    let hold = bot.canopy_ctl(DT, &a, &ctx, &mut it, &mut mv, true);
    assert_eq!(bot.can_side, 2, "both edges are wet: pinched");
    assert_eq!(threat_stats().break_wall, 1, "the break is counted once");
    assert!(hold.is_some(), "breaking takes the aim");
    assert!(it.fire, "a free, aimed shot pops the canopy");
    assert!(mv.z < -0.1, "and we back off it, got {mv}");
}

/// JS fight move L460-470: a stomping Stamp on our line — steer off its
/// facing cone; outside the cone (non-melee) keep edging away while it is
/// close; melee keeps its ground.
#[test]
fn tr471_stamp_steer_cone_and_backoff() {
    let _g = begin();
    let f = flat_fix();
    let a = kid(&f.tuning, 0, 0, Vec3::ZERO, 0.0);
    let mut t = kid(
        &f.tuning,
        1,
        1,
        Vec3::new(0.0, 0.0, 6.0),
        std::f32::consts::PI,
    );
    let bot = Bot::new(&a, &f.tuning.difficulty.hard, 18, 0);

    // The stamp facing us: the bot sits inside the facing cone → slide off
    // the line (JS L462-466).
    let mut sa = SpecialActive::new("stamp", "stamp", 2.0);
    sa.face_yaw = Some(std::f32::consts::PI);
    t.special_active = Some(Box::new(sa));
    let mut mvx = 0.0f32;
    let mut mvz = 1.0f32;
    bot.stamp_steer(&a, &t, 6.0, WeaponKind::Shooter, &mut mvx, &mut mvz);
    assert!(
        mvx.abs() > 0.5,
        "stepped off the stamp line, got ({mvx}, {mvz})"
    );

    // Facing away: outside the cone, close, non-melee → back off (JS L467-469).
    t.special_active.as_mut().unwrap().face_yaw = Some(0.0);
    let a2 = kid(&f.tuning, 0, 0, Vec3::new(0.0, 0.0, 2.0), 0.0);
    let mut mvx = 0.0f32;
    let mut mvz = 0.0f32;
    bot.stamp_steer(&a2, &t, 4.0, WeaponKind::Shooter, &mut mvx, &mut mvz);
    assert!(mvz < -0.1, "edged away from the stomp, got ({mvx}, {mvz})");

    // Melee keeps its ground (JS L467 `!MELEE[k]`).
    let mut mvx = 0.0f32;
    let mut mvz = 0.0f32;
    bot.stamp_steer(&a2, &t, 4.0, WeaponKind::Blade, &mut mvx, &mut mvz);
    assert_eq!((mvx, mvz), (0.0, 0.0), "melee does not back off");
}

// ------------------------------------------------------------------ A/B

/// JS `THREAT_AI` (bots.js L52) / the L1330 early-return: the same fixed-seed
/// scenario with a hostile locked waddle must be bit-identical with the switch
/// OFF (zero draws, zero behaviour — vs the no-threat baseline), and must
/// notice / engage with it ON. The switch is flipped only inside this one
/// test body, sequentially, under the file lock.
#[test]
fn tr471_threat_ai_ab_switch() {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_threat_stats();

    // Build two identical fixtures; only `with_waddle` gets the walker.
    let mk = |with_waddle: bool| {
        let mut f = flat_fix();
        // The duel-priority branch of `_threatCtl` (JS L1367-1372) reads
        // `ctx.kinds[ti]` — give the two-actor scene its weapon row.
        f.kinds = vec![WeaponKind::Shooter, WeaponKind::Shooter];
        if with_waddle {
            let mut ps = ProjectileSim::new(3);
            let mut paint = PaintGrid::new(&f.world);
            let mut actors = vec![
                kid(&f.tuning, 0, 0, Vec3::new(0.0, 0.0, -5.0), 0.0),
                kid(
                    &f.tuning,
                    1,
                    1,
                    Vec3::new(0.0, 0.0, 3.0),
                    std::f32::consts::PI,
                ),
            ];
            actors[1].sub = crate::sub::SubKind::Waddle;
            actors[1].ink = 60.0;
            f.subs.use_sub(
                1,
                1,
                &f.world,
                &mut actors,
                &mut paint,
                &mut ps,
                &f.tuning.player,
                &f.tuning.subs,
                None,
                0.0,
            );
            let it = &mut f.subs.waddle_items_mut()[0];
            it.state = WaddleState::Walk;
            it.pos = Vec3::new(0.0, 0.0, -1.0);
            it.air = false;
            it.target = Some(0);
            it.heading = std::f32::consts::PI;
        }
        f
    };

    let run = |f: &Fix, on: bool| -> Vec<(Vec3, bool, bool, bool)> {
        set_threat_ai(on);
        reset_threat_stats();
        let nav = NavGraph::new(&f.world, &f.layout, &f.tuning.player);
        let paint = PaintGrid::new(&f.world);
        let sim = SimWorld::new(&f.world, &f.layout);
        let mut a = kid(&f.tuning, 0, 0, Vec3::new(0.0, 0.0, -5.0), 0.0);
        let actors = vec![
            a.clone(),
            kid(
                &f.tuning,
                1,
                1,
                Vec3::new(0.0, 0.0, 3.0),
                std::f32::consts::PI,
            ),
        ];
        let mates = vec![None; 2];
        let ctx = ctx_of(f, &nav, &paint, &sim, &actors, &mates);
        let mut bot = Bot::new(&a, &f.tuning.difficulty.easy, 471, 0);
        let mut out = Vec::new();
        for _ in 0..150 {
            let it = bot.step(DT, &mut a, &ctx);
            out.push((it.move_dir, it.fire, it.squid, it.jump));
        }
        out
    };

    let f_waddle = mk(true);
    let f_plain = mk(false);

    // OFF: the threat path is dead code — bit-identical to no threat at all.
    let off_w = run(&f_waddle, false);
    let off_p = run(&f_plain, false);
    assert_eq!(off_w, off_p, "threat AI off must not change a single input");
    let st = threat_stats();
    assert!(
        st.noticed == 0 && st.shoot == 0 && st.evade == 0 && st.swim == 0 && st.sidestep == 0,
        "the off path must not touch the engagement counters: {st:?}"
    );

    // ON: the locked walker is noticed and engaged (shoot / evade / sidestep),
    // and the inputs diverge from the baseline.
    let on_w = run(&f_waddle, true);
    let st = threat_stats();
    assert!(st.noticed >= 1, "the waddle was noticed, got {st:?}");
    assert!(
        st.shoot + st.evade + st.sidestep + st.swim >= 1,
        "and acted on, got {st:?}"
    );
    assert_ne!(on_w, off_w, "the threat changes the bot's inputs");

    // Restore the production default for every other test in the process.
    set_threat_ai(true);
    reset_threat_stats();
}
