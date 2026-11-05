//! TR-36.1 — Zone Control rules tests (Task 36 batch 2).
//!
//! Synthetic scenes on the embedded Tidewater geometry + tuning with fixed
//! seeded [`Rng`] streams (no wall-clock randomness); the rules are driven
//! through the host's own entry points (`update`, `time_up`, `net_event`)
//! and the private helpers they call.

use super::*;
use crate::collision::CollisionWorld;
use crate::geometry::StageLayout;

struct Fix {
    layout: StageLayout,
    world: CollisionWorld,
    zt: ZonesTuning,
}

fn fix() -> Fix {
    Fix {
        layout: crate::embedded_tidewater(),
        world: CollisionWorld::tidewater(),
        zt: crate::embedded_tuning().zones,
    }
}

/// The placeholder side-zone anchor (JS `level.spawnPads[0]`, zones.js L69).
const PAD0: Vec3 = Vec3::new(18.0, 0.0, 0.0);

impl Fix {
    fn paint(&self) -> PaintGrid {
        PaintGrid::new(&self.world)
    }

    fn host(&self, paint: &PaintGrid, seed: u64, time: f32) -> ZoneControl {
        ZoneControl::new(
            paint,
            &self.world,
            self.layout.zones.as_ref(),
            PAD0,
            &self.zt,
            time,
            false,
            &mut Rng::new(seed),
        )
    }

    fn follower(&self, paint: &PaintGrid, seed: u64, time: f32) -> ZoneControl {
        ZoneControl::new(
            paint,
            &self.world,
            self.layout.zones.as_ref(),
            PAD0,
            &self.zt,
            time,
            true,
            &mut Rng::new(seed),
        )
    }
}

/// Flip every live cell of zone `zid` to team `t` immediately: a dur-0 flood
/// (the front is "done" on the first [`PaintGrid::step_floods`], so the whole
/// region converts). Replaces any running flood of the same region, exactly
/// like a real capture/wipe would (paint.js L721).
fn flood_now(zc: &ZoneControl, paint: &mut PaintGrid, world: &CollisionWorld, zid: usize, t: i8) {
    let z = &zc.zones[zid];
    let cells = z.cells.clone();
    let center = (z.center[0], z.center[2]);
    let reach = z.reach + 1.0;
    paint.flood(
        world,
        &cells,
        center,
        t,
        reach,
        0.0,
        FloodEase::Out,
        zid as u32,
    );
    paint.step_floods(0.01);
}

/// The `['zs']` slot helper: `JVal::Num` of the r3'd wire value.
fn num(v: f64) -> JVal {
    JVal::Num(v)
}

fn tag(e: &JVal) -> &str {
    e.as_array()
        .and_then(|a| a.first())
        .and_then(JVal::as_str)
        .unwrap_or_default()
}

// ---------------------------------------------------------------- TR-36.1 ①
#[test]
fn tr36_01_zone_thresholds_match_the_config_values() {
    // JS config.js L485-501 (`ZONES`) — the rules' numbers, verbatim.
    let z = crate::embedded_tuning().zones;
    assert_eq!(z.duration, 300); // L486
    assert_eq!(z.count, 100); // L487
    assert_eq!(z.rotate_min, 30); // L488
    assert_eq!(z.rotate_max, 60); // L488
    assert_eq!(z.final_centre, 30); // L489
    assert_eq!(z.warn, 0.30); // L490
    assert_eq!(z.control, 0.80); // L491
    assert_eq!(z.contest, 0.40); // L491
    assert_eq!(z.rate_center, 1.0); // L492
    assert_eq!(z.rate_home, 0.5); // L493
    assert_eq!(z.rate_away, 2.0); // L494
    assert_eq!(z.penalty_k, 0.75); // L495
    assert_eq!(z.gauge_held, 4.5); // L496
    assert_eq!(z.gauge_neutral, 1.5); // L497
    assert_eq!(z.overtime_grace, 10); // L498
    assert_eq!(z.overtime_max, 300); // L499
    assert_eq!(z.sample_hz, 5); // L500
}

// ---------------------------------------------------------------- TR-36.1 ②
#[test]
fn tr36_02_tick_rates_and_the_penalty_counts_off_first() {
    let f = fix();
    let paint = f.paint();
    let mut zc = f.host(&paint, 11, 300.0);
    zc.owner = 0;
    // JS zones.js L238-249 (`_tick`)
    // centre objective (home -1): rateCenter = 1 / s (L240, config.js L492)
    zc._tick(0, 1.0);
    assert!((zc.count[0] - 99.0).abs() < 1e-5);
    // sideA (home 0): the home team at rateHome .5 (L493) …
    zc.active = 1;
    zc._tick(0, 1.0);
    assert!((zc.count[0] - 98.5).abs() < 1e-5);
    // … the other team at rateAway 2 (L494)
    zc._tick(1, 1.0);
    assert!((zc.count[1] - 98.0).abs() < 1e-5);
    // JS L243-245: the penalty is counted off before the count
    zc.active = 0;
    zc.count[0] = 100.0;
    zc.penalty[0] = 0.3;
    zc._tick(0, 1.0);
    assert_eq!(zc.penalty[0], 0.0);
    assert!((zc.count[0] - 99.3).abs() < 1e-4);
    // JS L246-248: the tie point — the behind team catches the other's count
    zc.count = [50.0, 49.0];
    zc.tie_end = [None, None];
    zc._tick(0, 2.0);
    assert_eq!(zc.tie_end[0], Some(49.0));
}

// ---------------------------------------------------------------- TR-36.1 ③
#[test]
fn tr36_03_penalty_formula_bounds_and_zp_before_zo() {
    let f = fix();
    let paint = f.paint();
    let mut zc = f.host(&paint, 12, 300.0);
    // JS zones.js L228-236: ROUND(0.75 × (start − end)) + (start == 100 ? 1 : 0)
    // 0.75 × 10 = 7.5 -> Math.round half-up 8 (L231), +1 for start == count
    zc.count[1] = 90.0;
    zc.penalty[1] = 0.0;
    zc.start[1] = 100.0;
    zc.tie_end[1] = None;
    zc._penalise(1, 0.0);
    assert!((zc.penalty[1] - 9.0).abs() < 1e-5);
    // 0.75 × 5 = 3.75 -> 4; start != count -> no +1
    zc.count[1] = 45.0;
    zc.penalty[1] = 0.0;
    zc.start[1] = 50.0;
    zc._penalise(1, 0.0);
    assert!((zc.penalty[1] - 4.0).abs() < 1e-5);
    // a negative gap clamps to 0 (JS L231 `Math.max(0, …)`)
    zc.count[1] = 45.0;
    zc.penalty[1] = 0.0;
    zc.start[1] = 40.0;
    zc._penalise(1, 0.0);
    assert_eq!(zc.penalty[1], 0.0);
    // JS L230: past a tie point, progress beyond it is not penalised
    // end = max(tieEnd, total) = 60 -> 0.75 × 40 = 30, +1 = 31
    zc.count[1] = 55.0;
    zc.penalty[1] = 0.0;
    zc.start[1] = 100.0;
    zc.tie_end[1] = Some(60.0);
    zc._penalise(1, 0.0);
    assert!((zc.penalty[1] - 31.0).abs() < 1e-5);
    // JS L215 vs L221: `['zp']` lands before `['zo']` (a follower plays them
    // in the same order)
    let mut zc2 = f.host(&paint, 13, 300.0);
    zc2.last_owner = 1;
    zc2.count[1] = 90.0;
    zc2.start[1] = 100.0;
    zc2._set_owner(0, 5.0);
    let net = zc2.drain_net();
    assert_eq!(net.len(), 2);
    assert_eq!(tag(&net[0]), "zp");
    assert_eq!(tag(&net[1]), "zo");
    // the log entry (JS L234)
    assert_eq!(zc2.log.len(), 1);
    assert_eq!(zc2.log[0].team, 1);
    assert!((zc2.log[0].penalty - 9.0).abs() < 1e-5);
    assert!((zc2.log[0].next - 99.0).abs() < 1e-5); // total(1) = 90 + 9
}

// ---------------------------------------------------------------- TR-36.1 ④
#[test]
fn tr36_04_rotation_window_min_stint_final_lock_and_overtime_freeze() {
    let f = fix();
    let mut paint = f.paint();
    let mut zc = f.host(&paint, 14, 300.0);
    // JS zones.js L151 (`_swapDelay`): rotateMin … rotateMax
    let mut rng = Rng::new(99);
    for _ in 0..200 {
        let d = zc.swap_delay(&mut rng);
        assert!((30.0..=60.0).contains(&d), "window: {d}");
    }
    // JS L154-157 (`_schedule`) + L50 (`MIN_STINT` = 10): a rotation that
    // would land inside the last FINAL s never runs — from the centre there
    // are none left
    zc.active = 0;
    zc.schedule(40.0, &mut rng); // left = 40 - 30 = 10; 10 - d < MIN_STINT
    assert!(zc.next_swap.is_infinite());
    // … from a side zone the final lock itself takes it back (left + 0.5)
    zc.active = 1;
    zc.schedule(40.0, &mut rng);
    assert!((zc.next_swap - 10.5).abs() < 1e-5);
    // with the full clock the delay is the raw window value
    zc.active = 0;
    zc.schedule(300.0, &mut rng);
    assert!((30.0..=60.0).contains(&zc.next_swap));
    // JS L306-312 (`_finalLock`) path 1: active side -> swap to the centre
    zc.active = 1;
    zc._final_lock(25.0, &mut rng, &mut paint, &f.world);
    assert!(zc.final_lock);
    assert_eq!(zc.active, 0);
    assert_eq!(zc.next_swap, 0.0);
    let net = zc.drain_net();
    assert_eq!(
        net.last().unwrap(),
        &JVal::Arr(vec![
            JVal::Str("za".to_string()),
            num(0.0),
            num(1.0) // `final ? 1 : 0` (JS L294)
        ])
    );
    let ev = zc.drain_events();
    assert!(matches!(
        ev.last().unwrap(),
        ZoneEvent::Active { objective, final_lock: true, moved: true, .. }
            if objective == "center"
    ));
    // path 2: already at the centre -> just the `['zf']` announcement
    let mut zc2 = f.host(&paint, 15, 300.0);
    zc2._final_lock(25.0, &mut rng, &mut paint, &f.world);
    assert_eq!(tag(zc2.drain_net().last().unwrap()), "zf");
    let ev = zc2.drain_events();
    assert!(matches!(
        ev.last().unwrap(),
        ZoneEvent::Active {
            final_lock: true,
            moved: false,
            ..
        }
    ));
    // JS L284: no rotation in overtime — the swap clock is frozen
    let mut zc3 = f.host(&paint, 16, 300.0);
    zc3.overtime = true;
    zc3.sample_t = 10.0;
    zc3.snap_t = 10.0;
    zc3.next_swap = 5.0;
    let out = zc3.update(
        1.0,
        25.0,
        300.0,
        &mut paint,
        &f.world,
        &mut [],
        None,
        &mut rng,
    );
    assert_eq!(out, None); // level counts, sudden death, under the cap
    assert_eq!(zc3.active, 0);
    assert_eq!(zc3.next_swap, 5.0);
    assert!((zc3.overtime_t - 1.0).abs() < 1e-5);
}

// ---------------------------------------------------------------- TR-36.1 ⑤
#[test]
fn tr36_05_knockout_ends_with_the_exact_r3_ze_record() {
    let f = fix();
    let mut paint = f.paint();
    let mut rng = Rng::new(17);
    let mut zc = f.host(&paint, 17, 300.0);
    zc.owner = 0;
    zc.sample_t = 10.0; // no coverage read this frame
    zc.snap_t = 10.0; // no snapshot noise
    zc.count[0] = 0.5;
    zc.penalty[0] = 0.2;
    // JS zones.js L274: count and penalty both at 0 -> `_end(owner, 'knockout')`
    let out = zc.update(
        1.0,
        299.0,
        300.0,
        &mut paint,
        &f.world,
        &mut [],
        None,
        &mut rng,
    );
    assert_eq!(out, Some((0, "knockout".to_string())));
    assert_eq!(zc.count[0], 0.0);
    assert_eq!(zc.penalty[0], 0.0);
    assert_eq!(zc.winner, Some(0));
    assert_eq!(zc.reason.as_deref(), Some("knockout"));
    // JS L365: `['ze', winner, reason, r3(count0), r3(count1), r3(pen0), r3(pen1)]`
    let net = zc.drain_net();
    assert_eq!(
        net,
        vec![JVal::Arr(vec![
            JVal::Str("ze".to_string()),
            num(0.0),
            JVal::Str("knockout".to_string()),
            num(0.0),
            num(100.0),
            num(0.0),
            num(0.0),
        ])]
    );
    // JS L267: the winner guard — a decided match does nothing
    let out = zc.update(
        1.0,
        298.0,
        300.0,
        &mut paint,
        &f.world,
        &mut [],
        None,
        &mut rng,
    );
    assert_eq!(out, None);
    assert!(zc.drain_net().is_empty());
}

// ---------------------------------------------------------------- TR-36.1 ⑥
#[test]
fn tr36_06_time_up_branches_and_the_overtime_exits() {
    let f = fix();
    let paint = f.paint();
    let mut rng = Rng::new(18);
    // JS zones.js L315-333 (`timeUp`)
    // (a) the behind team still holds it at the horn -> overtime against it
    let mut zc = f.host(&paint, 18, 300.0);
    zc.owner = 1;
    zc.count = [50.0, 60.0];
    assert_eq!(zc.time_up(), None);
    assert!(zc.overtime);
    assert_eq!(zc.ot_losing, 1);
    assert_eq!(zc.overtime_t, 0.0);
    assert_eq!(tag(zc.drain_net().last().unwrap()), "zt");
    // (b) grace (L317): just knocked off the objective, < 10 s neutral
    let mut zc = f.host(&paint, 19, 300.0);
    zc.owner = -1;
    zc.last_owner = 1;
    zc.neutral_t = 5.0;
    zc.count = [50.0, 60.0];
    assert_eq!(zc.time_up(), None);
    assert!(zc.overtime && zc.ot_losing == 1);
    // (c) level at the horn -> sudden death (L324-330)
    let mut zc = f.host(&paint, 20, 300.0);
    zc.count = [50.0, 50.0];
    assert_eq!(zc.time_up(), None);
    assert!(zc.overtime && zc.ot_losing == -1);
    assert_eq!(
        zc.drain_net().last().unwrap(),
        &JVal::Arr(vec![JVal::Str("zt".to_string()), num(-1.0)])
    );
    // (d) the ahead team holds it -> the match ends now (L331)
    let mut zc = f.host(&paint, 21, 300.0);
    zc.owner = 0;
    zc.count = [50.0, 60.0];
    assert_eq!(zc.time_up(), Some((0, "time".to_string())));
    // JS L338-355 (`_overtime`) — the four exits
    // comeback: L's count drops below W's
    let mut zc = f.host(&paint, 22, 300.0);
    zc.overtime = true;
    zc.ot_losing = 0;
    zc.count = [40.0, 50.0];
    assert_eq!(
        zc._overtime(0.1, &mut rng),
        Some((0, "comeback".to_string()))
    );
    // retake: W takes the objective
    let mut zc = f.host(&paint, 23, 300.0);
    zc.overtime = true;
    zc.ot_losing = 0;
    zc.count = [50.0, 50.0];
    zc.owner = 1;
    assert_eq!(zc._overtime(0.1, &mut rng), Some((1, "retake".to_string())));
    // neutralised: L stays off it for the full grace
    let mut zc = f.host(&paint, 24, 300.0);
    zc.overtime = true;
    zc.ot_losing = 0;
    zc.count = [50.0, 50.0];
    zc.owner = -1;
    zc.neutral_t = 10.0;
    assert_eq!(
        zc._overtime(0.1, &mut rng),
        Some((1, "neutralised".to_string()))
    );
    // overtime-cap (config.js L499): five minutes of extra time
    let mut zc = f.host(&paint, 25, 300.0);
    zc.overtime = true;
    zc.ot_losing = 0;
    zc.count = [50.0, 50.0];
    zc.owner = 0;
    zc.overtime_t = 300.0;
    assert_eq!(
        zc._overtime(0.1, &mut rng),
        Some((1, "overtime-cap".to_string()))
    );
    // sudden death while level: play on under the cap, tiebreak at it
    // (JS L343-348 + L357-361; equal shares -> the seeded coin)
    let mut zc = f.host(&paint, 26, 300.0);
    zc.overtime = true;
    zc.ot_losing = -1;
    zc.count = [50.0, 50.0];
    assert_eq!(zc._overtime(0.1, &mut rng), None);
    zc.overtime_t = 300.0;
    let out = zc._overtime(0.1, &mut rng).expect("cap decides it");
    assert_eq!(out.1, "overtime-cap");
    assert!(out.0 == 0 || out.0 == 1);
}

// ---------------------------------------------------------------- TR-36.1 ⑦
#[test]
fn tr36_07_capture_floods_to_share_one_and_the_wipe_gates_recapture() {
    let f = fix();
    let mut paint = f.paint();
    let mut rng = Rng::new(27);
    let mut zc = f.host(&paint, 27, 300.0);
    assert!(!zc.zones[0].cells.is_empty(), "the centre has live cells");
    // ink the centre fully (share 1 >= control 0.80, config.js L491)
    flood_now(&zc, &mut paint, &f.world, 0, 0);
    zc.sample_t = 10.0;
    zc._sample(&mut paint, &f.world, 0.0);
    // JS zones.js L181-182: taken at the `control` share …
    assert_eq!(zc.zones[0].owner, 0);
    assert!((zc.zones[0].share[0] - 1.0).abs() < 1e-6);
    // … L188-189: the objective is held -> `_setOwner(0)`; the net order is
    // `['zz']` then `['zo']` (L193 / L221)
    let net = zc.drain_net();
    assert_eq!(net.iter().map(tag).collect::<Vec<_>>(), vec!["zz", "zo"]);
    assert_eq!(zc.owner, 0);
    // JS L161-167 (`_flood`): the capture floods the zone with the taker's
    // ink — after the front passes the coverage reads share 1 for the holder
    paint.step_floods(0.7); // FILL_T = 0.6 (L51) + margin
    zc._sample(&mut paint, &f.world, 0.0);
    assert!((zc.zones[0].share[0] - 1.0).abs() < 1e-6);
    assert_eq!(zc.zones[0].owner, 0);
    // JS L297-298 (`_swap`): the new objective is wiped to bare floor and
    // gated — `clock < z.hold` ignores the sampled share (L179)
    zc._swap(Some(1), false, 299.0, &mut rng, &mut paint, &f.world);
    let hold = zc.zones[1].hold;
    assert!(hold > zc.clock);
    flood_now(&zc, &mut paint, &f.world, 1, 1); // ink it anyway
    zc._sample(&mut paint, &f.world, 0.0);
    assert_eq!(zc.zones[1].owner, -1, "the wipe gates the capture");
    // once the wipe has run its turn the ink takes it (L181)
    zc.clock = hold + 0.05;
    zc._sample(&mut paint, &f.world, 0.0);
    assert_eq!(zc.zones[1].owner, 1);
    assert_eq!(zc.owner, 1);
    // the team 0 stint ends -> its penalty (L215 + L228-236): start 100,
    // end 100 -> ROUND(0) + 1
    assert!((zc.penalty[0] - 1.0).abs() < 1e-5);
}

// ---------------------------------------------------------------- TR-36.1 ⑧
#[test]
fn tr36_08_zs_snapshot_carries_ten_r3_fields() {
    let f = fix();
    let mut paint = f.paint();
    let mut rng = Rng::new(28);
    let mut zc = f.host(&paint, 28, 300.0);
    zc.count = [98.7654, 45.5009];
    zc.penalty = [8.12345, 0.0];
    zc.next_swap = f32::INFINITY;
    zc.overtime_t = 1.23457;
    zc.hold_t = [7.77778, 8.88889];
    zc.sample_t = 0.0; // the coverage read fires (empty grid -> no-ops) …
    zc.snap_t = 0.0; // … and so does the snapshot (JS zones.js L278-282)
    let out = zc.update(
        0.1,
        300.0,
        300.0,
        &mut paint,
        &f.world,
        &mut [],
        None,
        &mut rng,
    );
    assert_eq!(out, None);
    // JS L270: the sample period is 1 / sampleHz
    assert!((zc.sample_t - 0.2).abs() < 1e-6);
    let net = zc.drain_net();
    // owner -1 -> neutralT ticked by dt before the snapshot (L275)
    assert_eq!(
        net,
        vec![JVal::Arr(vec![
            JVal::Str("zs".to_string()),
            num(98.765), // r3(count0)
            num(45.501), // r3(count1)
            num(8.123),  // r3(penalty0)
            num(0.0),    // r3(penalty1)
            num(-1.0),   // nextSwap not finite -> -1 (L280)
            num(1.235),  // r3(overtimeT)
            num(0.1),    // r3(neutralT)
            num(7.778),  // r3(holdT0)
            num(8.889),  // r3(holdT1)
            num(0.1),    // Task 47 follow-up: zone clock at record time
        ])]
    );
    // a finite nextSwap rides the same slot
    zc.next_swap = 12.345;
    zc.snap_t = 0.1;
    zc.update(
        0.1,
        300.0,
        300.0,
        &mut paint,
        &f.world,
        &mut [],
        None,
        &mut rng,
    );
    let net = zc.drain_net();
    let zs = net[0].as_array().unwrap();
    assert_eq!(zs[5], num(12.345));
}

// ---------------------------------------------------------------- TR-36.1 ⑨
#[test]
fn tr36_09_follower_replays_the_host_stream_to_the_same_state() {
    let f = fix();
    let mut paint_h = f.paint();
    let mut paint_f = f.paint();
    let mut rng_h = Rng::new(31);
    let mut rng_f = Rng::new(31); // same seed: the rotation draws line up
    let mut host = f.host(&paint_h, 31, 300.0);
    let mut fol = f.follower(&paint_f, 31, 300.0);
    // a scripted host flow (no paint stepping — the grid stays bare, so the
    // coverage read is a no-op on both sides)
    host._zone_owner(0, 0, &mut paint_h, &f.world); // JS L192-197
    host._set_owner(0, 0.0); // L208-223
    host.update(
        1.0,
        299.0,
        300.0,
        &mut paint_h,
        &f.world,
        &mut [],
        None,
        &mut rng_h,
    ); // L266-289: ticks the count, pushes the ['zs']
    host._swap(Some(1), false, 298.0, &mut rng_h, &mut paint_h, &f.world); // L291-303
    let out = host.time_up(); // L315-333: team 1 is behind -> _end(0, 'time')
    assert_eq!(out, Some((0, "time".to_string())));
    let stream = host.drain_net();
    assert_eq!(
        stream.iter().map(tag).collect::<Vec<_>>(),
        vec!["zz", "zo", "zs", "za", "ze"]
    );
    // JS L372-413 (`netEvent`): the follower replays the records verbatim
    for (i, e) in stream.iter().enumerate() {
        let time = if tag(e) == "za" { 298.0 } else { 299.0 };
        let decided = fol.net_event(e, &mut paint_f, &f.world, time, 300.0, &mut rng_f);
        assert_eq!(
            decided.is_some(),
            i + 1 == stream.len(),
            "only ['ze'] decides"
        );
    }
    // the snapshot (JS L416-424) must match the host's exactly
    assert_eq!(host.state(), fol.state());
}

// ---------------------------------------------------------------- TR-36.1 ⑩
#[test]
fn tr36_10_placeholder_layout_branch() {
    let f = fix();
    let paint = f.paint();
    // JS zones.js L108-109: no `layout.zones` -> `placeholderZones(level)`
    let zc = ZoneControl::new(
        &paint,
        &f.world,
        None,
        PAD0,
        &f.zt,
        300.0,
        false,
        &mut Rng::new(32),
    );
    assert!(zc.placeholder);
    assert_eq!(zc.zones.len(), 3);
    assert_eq!(zc.objectives[0].name, "center");
    assert_eq!(zc.objectives[1].name, "sideA");
    assert_eq!(zc.objectives[2].name, "sideB");
    assert_eq!(zc.zones[0].home, -1);
    assert_eq!(zc.zones[1].home, 0);
    assert_eq!(zc.zones[2].home, 1);
    // JS L69: a 10 × 10 m centre at mid
    let cp = &zc.zones[0].def.parts()[0];
    assert!(inside_poly(cp, 0.0, 0.0));
    assert!(inside_poly(cp, 4.9, 4.9));
    assert!(!inside_poly(cp, 6.0, 0.0));
    // JS L71: the side zone a third-ish of the way to the spawn …
    let sp = &zc.zones[1].def.parts()[0];
    assert!(inside_poly(sp, PAD0.x * 0.4, PAD0.z * 0.45));
    // … and Bravo's is its 180° mirror (L56 / L113)
    let mp = &zc.zones[2].def.parts()[0];
    assert!(inside_poly(mp, -PAD0.x * 0.4, -PAD0.z * 0.45));
    assert!(!inside_poly(mp, PAD0.x * 0.4, PAD0.z * 0.45));
    // the operational objective is the centre (JS L122)
    assert!(zc.point_in_active(Vec3::new(0.0, 0.0, 0.0)));
    assert!(!zc.point_in_active(Vec3::new(100.0, 0.0, 100.0)));
    assert!(zc.state().placeholder);
}

// ------------------------------------------------ TR-37 ③ zone-cells smoke

/// TR-37 collision smoke ③: the new stages' frozen zone outlines sit on real
/// paintable turf in `zones` mode — every centre zone and the side zone of
/// each stage returns non-empty [`zone_cells`] (the saltpan centre is the
/// Great Pan's two quarters, zones-data.js L32, so both parts must be live).
/// Observed `zones`-mode counts (commit a9dc44a): saltpan centre 1348 +
/// 1348, shed-roof side 1408; crossmarket centre 1526, pier side 1566.
#[test]
fn tr37_zone_cells_are_live_on_the_new_stages() {
    for id in ["saltpan", "crossmarket"] {
        let l = crate::embedded_layout(id);
        let world = CollisionWorld::from_layout_for(&l, "zones");
        let paint = PaintGrid::new(&world);
        let zl = l.zones.as_ref().expect("extracted zone layout");
        for (name, def) in zl
            .center
            .iter()
            .enumerate()
            .map(|(i, z)| (format!("center{i}"), z))
            .chain([("side".to_string(), &zl.side)])
        {
            let (cells, center, reach) = zone_cells(def, &paint, &world);
            println!("zone-cells {id}/{name}: {} cells", cells.len());
            assert!(!cells.is_empty(), "{id}: {name} zone has no live cells");
            assert!(reach.is_finite() && reach > 0.0, "{id}: {name} reach");
            // the flood centre lies over the stage inside the band
            let y0 = def.y0.unwrap_or(-2.0);
            let y1 = def.y1.unwrap_or(6.0);
            assert!(center[1] >= y0 - 0.1 && center[1] <= y1 + 0.1);
        }
        // saltpan: TWO centre quarters (zones-data.js L32), crossmarket: one
        assert_eq!(
            zl.center.len(),
            if id == "saltpan" { 2 } else { 1 },
            "{id}: centre zone count"
        );
    }
}

/// TR-37 ③b: the crossmarket side zone is the tram pier deck (zones-data.js
/// L38, y -0.2..0.5). The zones-only landing pieces (layout.js L144-146:
/// hoist-stage top y 2.8-3.1, lighter-deck top y -0.9..-0.3, the gangplank
/// at x -22.85 outside the pier outline) fall outside that band/polygon, so
/// the pier's own turf carries the cells in BOTH modes — the variant drop
/// does not shrink the zone (deviation from the batch-1 brief's "drops
/// materially" premise, verified empirically). Observed (commit a9dc44a):
/// side-zone cells 1566 in `zones` mode, 1566 in `turf` mode.
#[test]
fn tr37_crossmarket_side_zone_survives_the_turf_drop() {
    let l = crate::embedded_layout("crossmarket");
    let zl = l.zones.as_ref().expect("crossmarket zones");
    let mut counts = Vec::new();
    for mode in ["zones", "turf"] {
        let world = CollisionWorld::from_layout_for(&l, mode);
        let paint = PaintGrid::new(&world);
        let (cells, _, _) = zone_cells(&zl.side, &paint, &world);
        println!("crossmarket side zone cells [{mode}]: {}", cells.len());
        counts.push(cells.len());
    }
    // the zone never empties, in either mode…
    assert!(counts[0] > 0 && counts[1] > 0, "side zone live: {counts:?}");
    // …and the 6 variant blocks the turf mode drops (106 vs 112, meta
    // variantCounts) are not pier-floor cells: the count is unchanged.
    assert_eq!(
        counts[0], counts[1],
        "pier cells must not depend on the zones-only landing"
    );
    // the evidence that the mode drop IS real: the turf world loses the six
    // landing blocks (and their faces) entirely.
    let zones_world = CollisionWorld::from_layout_for(&l, "zones");
    let turf_world = CollisionWorld::from_layout_for(&l, "turf");
    assert_eq!(zones_world.blocks.len(), 112);
    assert_eq!(turf_world.blocks.len(), 106);
    assert!(
        zones_world.faces.len() > turf_world.faces.len(),
        "the dropped landing contributed faces: zones {} vs turf {}",
        zones_world.faces.len(),
        turf_world.faces.len()
    );
    // the landing itself is zones-only in the layout (layout.js L144-146).
    let tagged = |mode: &str| {
        l.for_mode(mode)
            .primitives
            .iter()
            .filter(|p| {
                matches!(
                    p.common().tag.as_deref(),
                    Some("hoist-stage") | Some("lighter-deck") | Some("gangplank")
                )
            })
            .count()
    };
    assert_eq!(tagged("zones"), 6, "zones mode carries the landing");
    assert_eq!(tagged("turf"), 0, "turf mode drops it");
}
