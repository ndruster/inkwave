//! Task 35 batch-1 tests: the specials core layer — the gauge charge/keep
//! rules (actor.js L127-142/L216-219), the lifecycle + records
//! (specials.js L254-380), the net-ghost dispatch (L267-327), the `sp` /
//! `spx` / `spst` sample slots and the `['k', nid, 'sp', d]` wire channel
//! (netmatch.js L376-385/L745-750/L803-813), and the slam / storm phase
//! machines (actor.js L861-898).

use super::*;
use crate::actor::{Actor, ActorEvent, ActorInput, FIXED_DT, SpecialActive, SpecialPhase};
use crate::collision::CollisionWorld;
use crate::geometry::StageLayout;
use crate::match_::{Match, MatchEvent};
use crate::net::playback::Playback;
use crate::net::session::{RosterEntry, StartCfg};
use crate::net::transport::{Transport, TransportEvent};
use crate::net::wire::{JVal, decode_record, decode_tick, encode_record, parse_json};
use crate::net::{ActorSample, KitRecord, TickEvent, flags};
use crate::weapon::{WeaponKind, WeaponRunner};
use glam::{Vec3, vec3};

struct Fix {
    layout: StageLayout,
    world: CollisionWorld,
    tuning: crate::tuning::Tuning,
}

fn fix() -> Fix {
    Fix {
        layout: crate::embedded_tidewater(),
        world: CollisionWorld::tidewater(),
        tuning: crate::embedded_tuning(),
    }
}

impl Fix {
    fn sim(&self) -> crate::actor::SimWorld<'_> {
        crate::actor::SimWorld::new(&self.world, &self.layout)
    }
}

/// A bare actor with a known gauge (the match builder normally equips the
/// weapon's special + cost, net/matchbuild.rs — set directly here).
fn actor_with_special(t: &crate::tuning::PlayerTuning, id: &str, cost: f32) -> Actor {
    let mut a = Actor::new(0, 0, t);
    a.set_special(id);
    a.special_cost = cost;
    a.special = cost;
    a
}

/// Batch-3 `net_ghost` harness: the new signature (specials.js L267-327)
/// needs a full `WorldCtx`. The ghost ops under test only touch the actor,
/// the registry (`sys`) and fresh scratch, so the scratch lives per call.
fn net_ghost(a: &mut Actor, d: &[KitVal], sys: &mut SpecialSystem, f: &Fix) {
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng = Rng::new(0x5EED);
    let mut subs = crate::sub::SubSystem::new(0x5EED);
    let mut events = Vec::new();
    let mut ctx = world::WorldCtx {
        actors: std::slice::from_mut(a),
        collision: &f.world,
        paint: &mut paint,
        rng: &mut rng,
        subs: &mut subs,
        sb: &f.tuning.subs,
        t: &f.tuning.player,
        events: &mut events,
        remote: None,
        time: 0.0,
    };
    super::net_ghost(0, d, sys, &mut ctx);
}

/// Batch-4 `weapon_hook` harness for the trigger-owning kinds (specials.js
/// L359-362): the full signature needs the registry, collision, rng, paint,
/// projectile stream, tuning, input and the trigger gate. The batch-3 kinds
/// read `a.fire_gate` directly (no gate mutation), so the gate rides a copy
/// and the scratch (paint / stream) lives per call.
fn weapon_hook_at(a: &mut Actor, dt: f32, sys: &mut SpecialSystem, f: &Fix, rng: &mut Rng) -> bool {
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut ps = crate::weapon::ProjectileSim::new(5);
    let mut gate = a.fire_gate;
    crate::special::weapon_hook(
        a,
        dt,
        sys,
        &f.world,
        rng,
        &mut paint,
        &mut ps,
        &f.tuning.player,
        &ActorInput::default(),
        &mut gate,
    )
}

/// Batch-3 `weapon_hook` harness (fresh registry + rng per call).
fn weapon_hook_1(a: &mut Actor, dt: f32, f: &Fix) -> bool {
    let mut sys = SpecialSystem::new();
    let mut rng = Rng::new(0x5EED);
    weapon_hook_at(a, dt, &mut sys, f, &mut rng)
}

/// The JS `sample` builder shape (mirrors the playback tests).
fn sample(t: f32, x: f32) -> ActorSample {
    ActorSample {
        t,
        x,
        y: 0.0,
        z: 0.0,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw: 0.0,
        aim_yaw: 0.0,
        aim_pitch: 0.0,
        f: flags::ALIVE | flags::GROUNDED,
        hp: 100.0,
        ink: 100.0,
        sp: 0.0,
        ch: 0.0,
        turf: 0.0,
        tp: 0,
        wx: 0.0,
        wy: 0.0,
        wz: 0.0,
        lock: 0.0,
        ks: 0,
        spx: 0.0,
        spst: 0,
    }
}

/// Drive a [`Playback`] to `ready` with one sample (the tr22 recipe).
fn ready_playback(s: &ActorSample) -> Playback {
    let mut pb = Playback::new(1, 1.0 / 20.0);
    pb.on_tick("H", 0.0, 0.0);
    pb.push_sample(0, s);
    pb.advance_all(0.0, FIXED_DT as f64);
    pb.sample(0, "H", FIXED_DT as f64, 25.0);
    assert!(pb.cur_ks(0).is_some(), "the playback must be ready");
    pb
}

// ---------------------------------------------------------------------------
// TR-35.3 — the gauge: charge, ready edge, splat keep
// ---------------------------------------------------------------------------

#[test]
fn tr35_3_gauge_charges_and_emits_ready_once() {
    // JS `addTurf` (actor.js L133-142): `special = min(cost, special +
    // area*rate)`, the `specialReady()` false→true edge emits `special:ready`
    // exactly once, and the value caps at the cost.
    let f = fix();
    let t = &f.tuning.player;
    let rate = t.special_charge_rate;
    let mut a = actor_with_special(t, "slam", 0.0);
    a.special_cost = 1000.0;
    a.special = 0.0;
    a.add_turf(100.0 / rate, t); // exactly 100 points
    assert!(
        (a.special - 100.0).abs() < 1e-3,
        "area*rate credit: {}",
        a.special
    );
    assert!(!a.special_ready());
    assert!(a.drain_events().is_empty(), "no ready event below the cap");
    a.add_turf(900.0 / rate, t); // crosses the cost
    assert_eq!(a.special, 1000.0);
    let evs = a.drain_events();
    assert_eq!(evs, vec![ActorEvent::SpecialReady], "the edge fires once");
    a.add_turf(500.0 / rate, t); // capped, no second edge
    assert_eq!(a.special, 1000.0);
    assert!(
        a.drain_events().is_empty(),
        "already ready ⇒ no repeat event"
    );
    assert!(
        (a.turf - 1500.0 / rate).abs() < 1e-2,
        "turf credits regardless: {}",
        a.turf
    );
    assert_eq!(a.special_frac(), 1.0);
    // JS L134: `area <= 0` is a no-op.
    a.add_turf(0.0, t);
    assert!((a.turf - 1500.0 / rate).abs() < 1e-2);
}

#[test]
fn tr35_3_gauge_blocked_while_active_and_no_special_variant() {
    // JS L137: a running special blocks charging (its points were spent at
    // start); `addTurfNoSpecial` (L926) credits turf without charging.
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "barrage", 1000.0);
    a.special = 0.0;
    start(&mut a, "barrage");
    a.add_turf(5000.0, t);
    assert_eq!(a.special, 0.0, "JS L137: no charge while an active runs");
    assert_eq!(a.turf, 5000.0);
    a.add_turf_no_special(300.0); // JS L926
    assert_eq!(a.turf, 5300.0);
    assert_eq!(a.special, 0.0, "addTurfNoSpecial never charges");
}

#[test]
fn tr35_3_splat_keeps_share_and_drops_shield() {
    // JS `splat` L216-219: a cut-short special restarts from `cost*keep`;
    // an idle gauge from `min(cost,special)*keep`; then `onSplat` ends the
    // active (record `[1,'splat']`) and drops the shield (specials.js L379).
    let f = fix();
    let t = &f.tuning.player;
    let keep = t.special_keep_on_splat;
    let mut a = actor_with_special(t, "barrage", 1000.0);
    a.special = 400.0;
    a.splat(crate::actor::SplatCause::Weapon, None, t);
    assert!(
        (a.special - 400.0 * keep).abs() < 1e-3,
        "idle gauge keeps {} of 400: {}",
        keep,
        a.special
    );
    // the active branch: spend first, then die mid-special
    let mut b = actor_with_special(t, "barrage", 1000.0);
    b.status.shield = 5.0; // JS L379: the shield drops on splat
    b.start_special(t);
    b.splat(crate::actor::SplatCause::Weapon, None, t);
    assert!(
        (b.special - 1000.0 * keep).abs() < 1e-3,
        "a cut-short special restarts at cost*keep: {}",
        b.special
    );
    assert_eq!(b.status.shield, 0.0);
    assert!(b.special_active.is_none(), "onSplat ended the active");
    assert_eq!(
        b.sp_records.last().map(|r| (r[0].clone(), r[1].clone())),
        Some((KitVal::Num(1.0), KitVal::Str("splat".to_string()))),
        "JS L367: the splat end records `[1,'splat']`"
    );
    // respawn keeps the share (JS L166-168)
    b.respawn(&f.sim(), t);
    assert!(
        (b.special - 1000.0 * keep).abs() < 1e-3,
        "respawn preserves the kept share: {}",
        b.special
    );
}

// ---------------------------------------------------------------------------
// TR-35.2 — the sample slots + the `['k', nid, 'sp', d]` wire channel
// ---------------------------------------------------------------------------

#[test]
fn tr35_2_three_slots_pack_from_actor_state() {
    // netmatch.js L748 (`sp` = a.special), L750 (`spx` = SPECIAL_ORDER id
    // + 1), specials.js L2124 (`spst` = specialNetState).
    let f = fix();
    let t = &f.tuning.player;
    let wr = WeaponRunner::new();
    let mut a = actor_with_special(t, "storm", 1000.0);
    a.special = 123.5;
    let s = ActorSample::from_actor(&a, &wr, WeaponKind::Shooter, 0, 0.0);
    assert_eq!(s.sp, 123.5, "JS L748: the raw gauge");
    assert_eq!(s.spx, order_index("storm") as f32 + 1.0, "JS L750: 1-based");
    assert_eq!(s.spst, 0, "no active ⇒ specialNetState 0");
    // unknown equipped id ⇒ `indexOf` −1 + 1 = 0 (JS L750)
    a.special_id = Some("nope".to_string());
    let s = ActorSample::from_actor(&a, &wr, WeaponKind::Shooter, 0, 0.0);
    assert_eq!(s.spx, 0.0);
    // owner booyah: `round(clamp(charge,0,1)*63)` (specials.js L2129)
    let mut b = actor_with_special(t, "booyah", 1000.0);
    start(&mut b, "booyah");
    if let KindState::Booyah(st) = &mut b.special_active.as_mut().unwrap().state {
        st.charge = 0.5;
    }
    let s = ActorSample::from_actor(&b, &wr, WeaponKind::Shooter, 0, 0.0);
    assert_eq!(s.spst, 32, "0.5·63 = 31.5 rounds to 32");
    // a ghost packs 0 (specials.js L2125 `s.ghost` guard)
    let mut c = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut c,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    let s = ActorSample::from_actor(&c, &wr, WeaponKind::Shooter, 0, 0.0);
    assert_eq!(s.spst, 0, "ghosts never pack spst");
}

#[test]
fn tr35_2_sp_kit_records_are_byte_exact_on_the_wire() {
    // JS `recKit` writes `['k', nid, 'sp', data]` with the mixed array
    // verbatim (netmatch.js L134-137; specials.js L42): numbers rounded as
    // usual, strings as JSON strings. The `subs` kind-string lift (wire.rs)
    // must NOT touch the `sp` channel's strings.
    let idx = order_index("barrage");
    let start = TickEvent::Kit(KitRecord {
        nid: 0,
        kind: "sp".to_string(),
        sub_kind: None,
        data: vec![KitVal::Num(0.0), KitVal::Num(idx as f32)],
    });
    assert_eq!(
        encode_record(&start),
        format!("[\"k\",0,\"sp\",[0,{idx}]]"),
        "JS L258: rec(a, [0, SPECIAL_ORDER.indexOf(id)])"
    );
    let end = TickEvent::Kit(KitRecord {
        nid: 1,
        kind: "sp".to_string(),
        sub_kind: None,
        data: vec![KitVal::Num(1.0), KitVal::Str("splat".to_string())],
    });
    assert_eq!(encode_record(&end), "[\"k\",1,\"sp\",[1,\"splat\"]]");
    let spawn = TickEvent::Kit(KitRecord {
        nid: 2,
        kind: "sp".to_string(),
        sub_kind: None,
        data: vec![
            KitVal::Num(2.0),
            KitVal::Str("mi".to_string()),
            KitVal::Num(7.0),
            KitVal::Num(1.25),
            KitVal::Num(-2.5),
        ],
    });
    assert_eq!(
        encode_record(&spawn),
        "[\"k\",2,\"sp\",[2,\"mi\",7,1.25,-2.5]]"
    );
    for e in [&start, &end, &spawn] {
        let json = encode_record(e);
        let back = decode_record(parse_json(&json).unwrap().as_array().unwrap())
            .expect("a decodable kit record");
        assert_eq!(&back, e, "round-trip must be byte-identical: {json}");
    }
    // the `subs` lift stays scoped to `subs`: an `sp` payload string at any
    // index survives as `KitVal::Str` (the decoded `sub_kind` is `None`).
    let TickEvent::Kit(k) = decode_record(
        parse_json("[\"k\",0,\"sp\",[2,\"mi\",7,1,2,3]]")
            .unwrap()
            .as_array()
            .unwrap(),
    )
    .unwrap() else {
        panic!("the kit arm must decode");
    };
    assert_eq!(k.sub_kind, None);
    assert_eq!(k.data[1], KitVal::Str("mi".to_string()));
    assert_eq!(k.data[2], KitVal::Num(7.0));
    // and a real subs record still round-trips with its lifted kind
    let subs = TickEvent::Kit(KitRecord {
        nid: 3,
        kind: "subs".to_string(),
        sub_kind: Some("beacon".to_string()),
        data: vec![KitVal::Num(0.0), KitVal::Num(9.0), KitVal::Num(0.0)],
    });
    let json = encode_record(&subs);
    assert!(
        json.contains("\"beacon\""),
        "the kind rides the wire: {json}"
    );
    let back = decode_record(parse_json(&json).unwrap().as_array().unwrap()).unwrap();
    assert_eq!(back, subs);
}

/// A [`Transport`] stub recording framed sends (the replication tests' `Rec`).
#[derive(Default)]
struct Rec {
    out: Vec<(Option<String>, String)>,
}
impl Transport for Rec {
    fn connect(&mut self, _code: &str, _name: &str, _create: bool) {}
    fn poll_events(&mut self) -> Vec<TransportEvent> {
        Vec::new()
    }
    fn send_raw(&mut self, s: &str) -> bool {
        if let Some(rest) = s.strip_prefix("b|") {
            self.out.push((None, rest.to_string()));
        } else if let Some(rest) = s.strip_prefix("s|")
            && let Some((to, body)) = rest.split_once('|')
        {
            self.out.push((Some(to.to_string()), body.to_string()));
        } else {
            self.out.push((None, s.to_string()));
        }
        true
    }
    fn is_open(&self) -> bool {
        true
    }
    fn rtt(&self) -> f64 {
        0.0
    }
    fn close(&mut self) {}
}

#[test]
fn tr35_2_host_drain_rewrites_op2_gid_to_the_nid_form() {
    // specials.js L42 `netRec(a,'sp',d)` → replication: each *local* actor's
    // `sp_records` drain into `['k', nid, 'sp', data]`; op-2 gids were
    // stamped in the sim-side slot form (`slot*100_000+seq`, D-T34-5) and
    // the rewrite maps them to the wire form (`nid*100_000+seq`, registry.js
    // L64-65). Remote actors' queues are never touched.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.weapons;
    let sb = &f.tuning.subs;
    let roster = vec![
        RosterEntry {
            nid: 0,
            owner: "H".into(),
            bot: false,
            team: 0,
            slot: 0,
            name: "p0".into(),
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
        },
        RosterEntry {
            nid: 1,
            owner: "G".into(),
            bot: false,
            team: 1,
            slot: 0,
            name: "p1".into(),
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
        },
        RosterEntry {
            nid: 2,
            owner: "H".into(),
            bot: true,
            team: 1,
            slot: 1,
            name: "p2".into(),
            weapon: "shooter".into(),
            sub: None,
            special: None,
            style: None,
        },
    ];
    let cfg = StartCfg {
        roster,
        map: "tidewater".into(),
        time: "day".into(),
        duration: 120,
        difficulty: "normal".into(),
        palette: 0,
        mode: "turf".into(),
        host: "H".into(),
        id: "m1".into(),
    };
    let mut m = Match::with_roster(60.0, 7, &sim, t, &f.tuning.match_config, 1);
    let mut bot = Actor::new(1, 1, t);
    let pad = sim.spawn_pads[1];
    bot.spawn_at(vec3(pad.x, pad.y, pad.z), std::f32::consts::PI, &sim, t);
    bot.invuln = 0.0;
    m.actors.push(bot);
    m.runners.push(WeaponRunner::new());
    m.kinds.push(WeaponKind::Shooter);
    let mut r = crate::net::replication::Replication::new(&cfg, "H", "H");
    r.attach_bot(2, &m.actors[2], &f.tuning.difficulty.easy, 7);
    r.bind_match(&mut m);
    for _ in 0..(crate::match_::INTRO_TIME * 60.0) as usize + 2 {
        m.step_masked(FIXED_DT, &[], Some(r.remote_mask()), &sim, t, w, sb, None);
    }
    assert!(m.playing());

    // slot form on the bot (slot 1 ⇒ 100_005) and on the remote actor.
    m.actors[2].sp_records.push(vec![
        KitVal::Num(2.0),
        KitVal::Str("mi".to_string()),
        KitVal::Num(100_005.0),
        KitVal::Num(1.0),
    ]);
    m.actors[2]
        .sp_records
        .push(vec![KitVal::Num(0.0), KitVal::Num(3.0)]);
    m.actors[1]
        .sp_records
        .push(vec![KitVal::Num(0.0), KitVal::Num(3.0)]);

    let mut rec = Rec::default();
    // The drain runs *after* `send_tick` in one `update` (JS ordering: the
    // records ride the *next* tick, replication.rs L438-441), so spin past a
    // second 1/20 s tick boundary and read the last tick frame.
    for _ in 0..5 {
        r.update(FIXED_DT, &mut m, &sim, t, w, sb, &mut rec, None);
    }

    let payload = &rec
        .out
        .iter()
        .rev()
        .find(|(_, p)| {
            parse_json(p).and_then(|v| v.field("k").and_then(JVal::as_str).map(String::from))
                == Some("t".to_string())
        })
        .expect("a tick frame")
        .1;
    let (tick, _) = decode_tick(&parse_json(payload).unwrap());
    let kits: Vec<&KitRecord> = tick
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            TickEvent::Kit(k) if k.kind == "sp" => Some(k),
            _ => None,
        })
        .collect();
    assert_eq!(kits.len(), 2, "only the local actor's records drain");
    assert_eq!(kits[0].nid, 2);
    assert_eq!(
        kits[0].data,
        vec![
            KitVal::Num(2.0),
            KitVal::Str("mi".to_string()),
            KitVal::Num(200_005.0), // nid 2 × 100_000 + seq 5 (D-T34-5)
            KitVal::Num(1.0),
        ],
        "the op-2 gid is rewritten slot→nid"
    );
    assert_eq!(
        kits[1].data,
        vec![KitVal::Num(0.0), KitVal::Num(3.0)],
        "op-0 records pass through untouched"
    );
    assert!(m.actors[2].sp_records.is_empty(), "the queue drains");
    assert_eq!(
        m.actors[1].sp_records.len(),
        1,
        "remote queues are left alone"
    );
}

// ---------------------------------------------------------------------------
// lifecycle (specials.js `start` / `tick` / `body` / `end` / `onSplat`)
// ---------------------------------------------------------------------------

#[test]
fn tr35_lifecycle_start_records_and_timer_expiry() {
    // JS L258 `rec(a, [0, indexOf(id)])`; L341 `s.t >= s.dur → end('time')`
    // with the L367 record `[1, reason]`; the active clears (L369).
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "barrage", 1000.0);
    start(&mut a, "barrage");
    let dur = specials().barrage.duration;
    assert!(dur > 0.0, "barrage is a timed kind (config.js L416)");
    let s = a.special_active.as_ref().unwrap();
    assert_eq!(s.id, "barrage");
    assert_eq!(s.kind, "barrage");
    assert_eq!(s.dur, dur);
    assert!(!s.body, "barrage runs the generic tick (JS L275)");
    assert_eq!(
        a.sp_records[0],
        vec![KitVal::Num(0.0), KitVal::Num(order_index("barrage") as f32)]
    );
    let frames = (dur * 60.0).ceil() as usize + 1;
    for _ in 0..frames {
        tick(&mut a, FIXED_DT, &ActorInput::default());
    }
    assert!(a.special_active.is_none(), "JS L341: expired at the cap");
    assert_eq!(
        a.sp_records[1],
        vec![KitVal::Num(1.0), KitVal::Str("time".to_string())],
        "JS L367: the end records [1,'time']"
    );
    // a second end is a no-op (JS L366 `s.ended` guard via the None active)
    end(&mut a, "manual");
    assert_eq!(a.sp_records.len(), 2);
    // unknown ids are ignored (JS `SPECIALS[id]` undefined, L254)
    start(&mut a, "nope");
    assert!(a.special_active.is_none());
    assert!(def_by_id("nope").is_none());
    assert_eq!(order_index("nope"), -1);
}

#[test]
fn tr35_lifecycle_body_kind_expiry() {
    // JS L275: `s.body` kinds run `body` (L344-355), not `tick` — `tick`
    // early-returns; `body` advances the timer and ends at `dur` (sonar's
    // dur is `def.cast`, specials.js L1264).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "sonar", 1000.0);
    start(&mut a, "sonar");
    assert!(a.special_active.as_ref().unwrap().body);
    let frames = (specials().sonar.cast * 60.0).ceil() as usize + 1;
    tick(&mut a, FIXED_DT, &ActorInput::default()); // must not advance: the body guard
    assert_eq!(a.special_active.as_ref().unwrap().t, 0.0);
    for _ in 0..frames {
        body(&mut a, FIXED_DT, &sim, t, &ActorInput::default());
    }
    assert!(a.special_active.is_none());
    assert_eq!(
        a.sp_records[1],
        vec![KitVal::Num(1.0), KitVal::Str("time".to_string())]
    );
}

#[test]
fn tr35_lifecycle_start_special_handoff() {
    // JS `_startSpecial` (actor.js L837-859): spend the gauge, refill the
    // tank (L841), `stats.specials++` (L840), emit `special:use` (L845), and
    // hand non-slam/storm ids to `SpecialSystem.start` (L859 → specials.js
    // L254, which records `[0, idx]`).
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "barrage", 1000.0);
    a.ink = 10.0;
    a.start_special(t);
    let evs = a.drain_events();
    assert!(
        evs.contains(&ActorEvent::SpecialUse {
            id: "barrage".to_string()
        }),
        "JS L845"
    );
    assert_eq!(a.special, 0.0, "JS L839: the gauge is spent");
    assert_eq!(a.ink, t.ink_max, "JS L841: the tank refills");
    assert_eq!(a.specials_used, 1);
    assert!(a.special_active.is_some());
    assert_eq!(
        a.sp_records[0],
        vec![KitVal::Num(0.0), KitVal::Num(order_index("barrage") as f32)]
    );
    // JS L838: an unknown id falls back to `slam`
    let mut b = actor_with_special(t, "nope", 1000.0);
    b.special_id = Some("nope".to_string());
    b.start_special(t);
    assert_eq!(b.special_active.as_ref().unwrap().id, "slam");
}

// ---------------------------------------------------------------------------
// net ghost (specials.js L267-327, L287-298)
// ---------------------------------------------------------------------------

#[test]
fn tr35_net_ghost_ops() {
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    // op 0: `_startGhost` (L277-285) — a ghost active, no records (L367
    // guard keeps ghost starts silent).
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    let s = a.special_active.as_ref().expect("the ghost active");
    assert!(s.ghost);
    assert_eq!(s.id, "crab");
    assert_eq!(s.kind, "crab");
    assert!(a.sp_records.is_empty(), "ghosts never record");
    // op 0 again: the previous ghost closes first (JS L278 `end('net')`)
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("booyah") as f32)],
        &mut sys,
        &f,
    );
    let s = a.special_active.as_ref().unwrap();
    assert_eq!(s.id, "booyah");
    assert!(s.ghost);
    assert!(a.sp_records.is_empty());
    // a bad index is ignored (JS `SPECIAL_ORDER[d[1]]` undefined → L278
    // early-return before any state change would apply)
    net_ghost(&mut a, &[KitVal::Num(0.0), KitVal::Num(-1.0)], &mut sys, &f);
    net_ghost(&mut a, &[KitVal::Num(0.0), KitVal::Num(99.0)], &mut sys, &f);
    assert_eq!(a.special_active.as_ref().unwrap().id, "booyah");
    // op 1: the ghost end (L270) with the payload reason
    net_ghost(
        &mut a,
        &[KitVal::Num(1.0), KitVal::Str("time".to_string())],
        &mut sys,
        &f,
    );
    assert!(a.special_active.is_none());
    assert!(a.sp_records.is_empty(), "ghost ends never record");
    // op 1 on a *live* active is ignored (JS L270 `s.ghost` guard — the
    // owner never replays its own records)
    start(&mut a, "barrage");
    net_ghost(
        &mut a,
        &[KitVal::Num(1.0), KitVal::Str("time".to_string())],
        &mut sys,
        &f,
    );
    assert!(a.special_active.is_some());
    end(&mut a, "manual");
    // op 2: `_ghostObj` (L306-322) — registry insert + gid dedup
    net_ghost(
        &mut a,
        &[
            KitVal::Num(2.0),
            KitVal::Str("mi".to_string()),
            KitVal::Num(7.0),
            KitVal::Num(1.0),
            KitVal::Num(2.0),
            KitVal::Num(3.0),
        ],
        &mut sys,
        &f,
    );
    assert_eq!(sys.world.len(), 1);
    let w = &sys.world[0];
    assert!(w.ghost && w.gid == 7 && w.team == a.team);
    // JS L779: the missile *starts* above the thrower (`a.pos + 1.6y`, the
    // `1.6` hardcoded); `V(d, 3)` is the ground target, not the spawn pos.
    assert_eq!(w.pos, a.pos + vec3(0.0, 1.6, 0.0));
    let world::ObjectBody::Missile(mi) = &w.body else {
        panic!("the `mi` ghost carries a missile body");
    };
    assert_eq!(mi.to, vec3(1.0, 2.0, 3.0));
    net_ghost(
        &mut a,
        &[
            KitVal::Num(2.0),
            KitVal::Str("mi".to_string()),
            KitVal::Num(7.0),
            KitVal::Num(9.0),
            KitVal::Num(9.0),
            KitVal::Num(9.0),
        ],
        &mut sys,
        &f,
    );
    assert_eq!(sys.world.len(), 1, "JS L309: the gid dedup");
    // unknown object kinds are dropped (JS L311-317 only lists the visual
    // tags `mi/tw/sk/bu/ts/sh/or`)
    net_ghost(
        &mut a,
        &[
            KitVal::Num(2.0),
            KitVal::Str("xx".to_string()),
            KitVal::Num(8.0),
        ],
        &mut sys,
        &f,
    );
    assert_eq!(sys.world.len(), 1);
    // ops 3/4 are batch-2 stubs — they must not panic or mutate
    net_ghost(
        &mut a,
        &[KitVal::Num(3.0), KitVal::Num(7.0), KitVal::Num(0.0)],
        &mut sys,
        &f,
    );
    net_ghost(
        &mut a,
        &[KitVal::Num(4.0), KitVal::Str("cg".to_string())],
        &mut sys,
        &f,
    );
    assert!(a.special_active.is_none());
}

#[test]
fn tr35_ghost_tick_expiry_and_death() {
    // JS `_ghostTick` (L287-298): the timer runs, `!a.alive → end('splat')`
    // (L291), `s.t >= s.dur → end('time')` (L297) — ghost ends never record.
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    // a live actor never ticks the ghost end early
    ghost_tick(&mut a, FIXED_DT);
    assert!(a.special_active.as_ref().unwrap().ghost);
    a.alive = false;
    ghost_tick(&mut a, FIXED_DT);
    assert!(a.special_active.is_none(), "JS L291: death ends the ghost");
    assert!(a.sp_records.is_empty());
    // duration expiry
    let mut b = actor_with_special(t, "crab", 1000.0);
    net_ghost(
        &mut b,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    let frames = (specials().crab.duration * 60.0).ceil() as usize + 1;
    for _ in 0..frames {
        ghost_tick(&mut b, FIXED_DT);
    }
    assert!(b.special_active.is_none(), "JS L297: the ghost expired");
    assert!(b.sp_records.is_empty());
    // a non-ghost active is untouched by `_ghostTick`
    start(&mut b, "barrage");
    ghost_tick(&mut b, FIXED_DT);
    assert!(b.special_active.is_some());
}

// ---------------------------------------------------------------------------
// playback special block (netmatch.js L376-385)
// ---------------------------------------------------------------------------

#[test]
fn tr35_playback_special_flag_builds_and_clears_the_placeholder() {
    // JS L382: flag on with no active ⇒ `{ id, net: true }` placeholder;
    // L384: flag off with a non-ghost active ⇒ dropped outright.
    let f = fix();
    let t = &f.tuning.player;
    let mut s = sample(0.0, 0.0);
    s.f |= flags::SPECIAL;
    s.sp = 777.0;
    s.spx = order_index("barrage") as f32 + 1.0;
    let mut pb = ready_playback(&s);
    let mut a = Actor::new(1, 0, t);
    pb.apply(0, &mut a, FIXED_DT);
    // L376: the gauge rides `S.sp`; L379: the id from `spx`
    assert_eq!(a.special, 777.0);
    assert_eq!(a.special_id.as_deref(), Some("barrage"));
    let act = a.special_active.as_ref().expect("the placeholder");
    assert!(!act.ghost, "JS L382: the placeholder is not a ghost");
    assert!(
        !act.has_def,
        "the placeholder has no def (specials.js L2125)"
    );
    assert_eq!(act.id, "barrage");
    // flag off: the placeholder is dropped outright (JS L384, no grace)
    let mut s2 = sample(0.0, 0.0);
    s2.f &= !flags::SPECIAL;
    let mut pb2 = ready_playback(&s2);
    pb2.apply(0, &mut a, FIXED_DT);
    assert!(a.special_active.is_none(), "JS L384: flag-off clears it");
}

#[test]
fn tr35_playback_ghost_gets_the_off_grace() {
    // JS L383: a *ghost* active survives up to 0.6 s of the `F.special`
    // flag being off (its end record may still be in flight), then ends
    // `'net'` — and ghost ends never record.
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    let mut pb = ready_playback(&sample(0.0, 0.0)); // no SPECIAL flag
    // 0.5 s of flag-off: inside the 0.6 s grace (JS L383)
    for _ in 0..30 {
        pb.apply(0, &mut a, FIXED_DT);
    }
    assert!(a.special_active.is_some(), "grace: 0.5 s is not past 0.6 s");
    // past the window the ghost ends `'net'`
    for _ in 0..10 {
        pb.apply(0, &mut a, FIXED_DT);
    }
    assert!(a.special_active.is_none(), "JS L383: the ghost ended 'net'");
    assert!(a.sp_records.is_empty(), "ghost ends never record");
}

#[test]
fn tr35_playback_spst_feeds_the_ghost_crab_snapshot() {
    // JS L385: `specialNetApply(a, S.spst)` — the ghost crab takes the
    // packed roll / firing / hull byte (specials.js L2131-2134).
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    let mut s = sample(0.0, 0.0);
    s.f |= flags::SPECIAL;
    s.spst = (63 << 2) | (1 << 1) | 1; // hull byte 63, firing, roll
    let mut pb = ready_playback(&s);
    pb.apply(0, &mut a, FIXED_DT);
    let act = a.special_active.as_ref().unwrap();
    assert!(act.ghost, "the flag-on branch keeps the existing ghost");
    let KindState::Crab(st) = &act.state else {
        panic!("the crab ghost must carry its kind state");
    };
    let net = st.net.expect("the spst snapshot");
    assert!(net.roll && net.firing);
    assert!(
        (net.hull - 63.0 / 255.0 * std::f32::consts::TAU).abs() < 1e-6,
        "JS L2134: hull = byte/255·TAU"
    );
}

// ---------------------------------------------------------------------------
// slam / storm phase machines (actor.js L861-898)
// ---------------------------------------------------------------------------

#[test]
fn tr35_slam_phase_machine() {
    // JS L850-851 (leap) → L879-888 (Rise → Hang → Fall) → L893-896
    // (impact: `special:slam` event, active gone, 0.3 s grace). Slam actives
    // never record (specials.js L365/L367 guard).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "slam", 1000.0);
    let pad = sim.spawn_pads[0];
    a.spawn_at(vec3(pad.x, pad.y, pad.z), 0.0, &sim, t);
    a.vel = vec3(6.0, 0.0, 0.0);
    a.start_special(t);
    let evs = a.drain_events();
    assert!(evs.contains(&ActorEvent::SpecialUse {
        id: "slam".to_string()
    }));
    let s = a.special_active.as_ref().unwrap();
    assert!(s.armor, "JS L850: armour through the slam");
    assert!(s.body, "JS L850: the special owns the body");
    assert_eq!(s.phase, SpecialPhase::Rise);
    assert_eq!(a.vel.y, 11.5, "JS L851: the leap");
    assert!(
        (a.vel.x - 1.8).abs() < 1e-5,
        "JS L851: lateral ×0.3 (got {})",
        a.vel.x
    );
    assert!(!a.grounded);
    let mut saw_hang = false;
    let mut saw_fall = false;
    let input = ActorInput::default();
    for _ in 0..900 {
        a.update_special(FIXED_DT, &input, &sim, t);
        let phase = a.special_active.as_ref().map(|s| s.phase);
        if phase == Some(SpecialPhase::Hang) {
            saw_hang = true;
        }
        if phase == Some(SpecialPhase::Fall) {
            saw_fall = true;
        }
        if phase.is_none() {
            break;
        }
    }
    assert!(saw_hang && saw_fall, "Rise → Hang → Fall (JS L882-886)");
    assert!(a.special_active.is_none(), "JS L894: the active is gone");
    let evs = a.drain_events();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            ActorEvent::SpecialSlam { radius, .. }
                if (*radius - specials().slam.radius).abs() < 1e-6
        )),
        "JS L914: the impact event carries the tuning radius"
    );
    assert_eq!(a.invuln, 0.3, "JS L896: the end grace");
    assert!(a.sp_records.is_empty(), "slam never records (L365 guard)");
    // `end()` skips slam/storm (specials.js L364)
    let mut b = actor_with_special(t, "slam", 1000.0);
    b.spawn_at(vec3(pad.x, pad.y, pad.z), 0.0, &sim, t);
    b.start_special(t);
    end(&mut b, "manual");
    assert!(
        b.special_active.is_some(),
        "JS L364: slam is immune to end()"
    );

    // Match level (batch 2): the landing runs `_slamImpact` — the paint is
    // credited through `addTurfNoSpecial` (never recharges the gauge, JS
    // L910) and the `special:slam` marker reaches the FORWARD bus.
    let w = &f.tuning.weapons;
    let sb = &f.tuning.subs;
    let mut m = tr35_match(&f, 11, 1);
    m.actors[0].set_special("slam");
    m.actors[0].special = m.actors[0].special_cost;
    let cov0 = m.paint.coverage()[0];
    let press = [ActorInput {
        special: true,
        ..ActorInput::default()
    }];
    m.step(FIXED_DT, &press, &sim, t, w, sb, None);
    let mut frames = 0;
    while !m
        .events
        .iter()
        .any(|e| matches!(e, MatchEvent::SpecialSlam { .. }))
    {
        m.step(FIXED_DT, &[], &sim, t, w, sb, None);
        frames += 1;
        assert!(
            frames < 300,
            "the slam lands within rise 0.55 + hang 0.25 + fall (JS L877-893)"
        );
    }
    assert!(
        m.events.iter().any(|e| matches!(
            e,
            MatchEvent::SpecialSlam { owner: (0, 0), radius, .. }
                if (*radius - specials().slam.radius).abs() < 1e-6
        )),
        "JS actor.js L914 → netmatch.js L47: the FORWARD bus event"
    );
    assert!(m.actors[0].turf > 0.0, "the impact paint is credited");
    assert_eq!(
        m.actors[0].special, 0.0,
        "JS L910: special ink never recharges the spent gauge"
    );
    assert!(
        m.paint.coverage()[0] > cov0,
        "the slam grew team 0 turf (JS L903-910)"
    );
}

#[test]
fn tr35_storm_throw_and_glide() {
    // JS L855-857: the storm builds a body-owning active in the `Throw`
    // phase and emits the cloud throw; L868-875: the windup glides with the
    // `exp(-6·dt)` damping, control returning after 0.35 s.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "storm", 1000.0);
    let pad = sim.spawn_pads[0];
    a.spawn_at(vec3(pad.x, pad.y, pad.z), 0.0, &sim, t);
    a.start_special(t);
    let evs = a.drain_events();
    assert!(evs.contains(&ActorEvent::SpecialStorm), "JS L857");
    let s = a.special_active.as_ref().unwrap();
    assert!(s.body);
    assert_eq!(s.phase, SpecialPhase::Throw);
    a.vel = vec3(10.0, 0.0, 0.0);
    a.grounded = true;
    let input = ActorInput::default();
    a.update_special(FIXED_DT, &input, &sim, t);
    let want = 10.0 * (-6.0 * FIXED_DT).exp(); // JS L869
    assert!(
        (a.vel.x - want).abs() < 1e-5,
        "the xz glide decays exp(-6·dt): {} vs {}",
        a.vel.x,
        want
    );
    assert!(
        (a.vel.y).abs() < 1e-3,
        "grounded: the y is pinned (JS L871): {}",
        a.vel.y
    );
    for _ in 0..30 {
        a.update_special(FIXED_DT, &input, &sim, t);
    }
    assert!(
        a.special_active.is_none(),
        "JS L868: control returns after the 0.35 s windup"
    );
    assert!(a.sp_records.is_empty(), "storm never records (L365 guard)");

    // Direct throw geometry (JS `throwStorm`, weapons.js L1467-1479): the
    // bomb spawns at `pos.y + 1.45` (L1473), `fuse: -1` (it never arms —
    // clouds on landing instead), kind `storm` and the unit-XZ heading on
    // `dir` (L1477); the `recBomb` (L1478) rides the `BombSpawn` event.
    let mut ps = crate::weapon::ProjectileSim::new(5);
    let mut a2 = actor_with_special(t, "storm", 1000.0);
    a2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    a2.aim_pitch = -0.58; // pitch clamps to −0.3 (JS L1471): a short lob
    ps.throw_storm(&a2);
    assert_eq!(ps.bombs.len(), 1);
    let b = &ps.bombs[0];
    assert_eq!(b.kind, "storm", "JS L1475: the storm kind");
    assert_eq!(b.fuse, -1.0, "JS L1475: a storm bomb never arms");
    assert!(
        (b.pos - vec3(a2.pos.x, a2.pos.y + 1.45, a2.pos.z)).length() < 1e-5,
        "JS L1473: the throw point (got {:?}, actor at {:?})",
        b.pos,
        a2.pos
    );
    let d = b.dir.expect("JS L1477: storm carries the throw heading");
    assert!(
        (d - vec3(0.0, 0.0, 1.0)).length() < 1e-5,
        "aim yaw 0 ⇒ unit +z (got {d:?})"
    );
    let evs = ps.drain_events();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            crate::weapon::SimEvent::BombSpawn { kind, .. } if kind == "storm"
        )),
        "JS L1478 `recBomb`: the spawn rides the wire event"
    );

    // Match level: the bomb clouds (landing or the 1.1 s age cap, JS L1729 /
    // L1740) and the cloud enters the specials world registry.
    let w = &f.tuning.weapons;
    let sb = &f.tuning.subs;
    let mut m = tr35_match(&f, 7, 1);
    m.actors[0].set_special("storm");
    m.actors[0].special = m.actors[0].special_cost;
    m.actors[0].aim_pitch = -0.58;
    let press = [ActorInput {
        special: true,
        ..ActorInput::default()
    }];
    m.step(FIXED_DT, &press, &sim, t, w, sb, None);
    assert_eq!(m.projectiles.bombs.len(), 1, "the press threw the bomb");
    let mut frames = 0;
    while m.specials.world.is_empty() {
        m.step(FIXED_DT, &[], &sim, t, w, sb, None);
        frames += 1;
        assert!(
            frames < 200,
            "the bomb clouds by the 1.1 s age cap (JS L1740)"
        );
    }
    let obj = &m.specials.world[0];
    assert_eq!(obj.kind, "cloud", "JS `_spawnCloud` L1539");
    assert_eq!(obj.team, 0);
    let cloud = match &obj.body {
        world::ObjectBody::Cloud(c) => *c,
        other => panic!("the registry entry must carry a Cloud body, got {other:?}"),
    };
    assert!(
        (cloud.dur - specials().storm.duration).abs() < 1e-6,
        "JS L1539: `dur: SPECIALS.storm.duration` = 6.5"
    );
    assert!(
        (obj.pos.y - (cloud.ground_y + 4.6)).abs() < 1e-4,
        "JS L1535: the cloud hangs at groundY + 4.6"
    );
    let (z0, x0) = (obj.pos.z, obj.pos.x);
    for _ in 0..10 {
        m.step(FIXED_DT, &[], &sim, t, w, sb, None);
    }
    let obj = &m.specials.world[0];
    let want = specials().storm.drift_speed * FIXED_DT * 10.0;
    assert!(
        ((obj.pos.z - z0) - cloud.dir.z * want).abs() < 1e-3
            && ((obj.pos.x - x0) - cloud.dir.x * want).abs() < 1e-3,
        "JS L1768: drift `dir * driftSpeed * dt` (1.1 m/s)"
    );
    assert!(
        m.actors[0].turf > 0.0,
        "JS L1779: the rain credits the owner's turf"
    );
    assert_eq!(
        m.actors[0].special, 0.0,
        "JS L1779 `addTurfNoSpecial`: the rain never recharges"
    );
}

// ---------------------------------------------------------------------------
// world objects + `remaining` + damage gates + net state
// ---------------------------------------------------------------------------

#[test]
fn tr35_spawn_records_the_slot_form_gid() {
    // JS `_spawn` (L300-305): stamp `gid = netId(a)` and record
    // `[2, kind, gid, ...data]`; the sim-side gid is the slot form
    // (`slot*100_000+seq`, D-T34-5) — the replication layer rewrites it.
    let f = fix();
    let t = &f.tuning.player;
    let mut a = Actor::new(0, 1, t);
    let mut sys = SpecialSystem::new();
    let obj = world::WorldObject::new("mi", a.team, Vec3::ZERO);
    spawn(&mut a, &mut sys, obj, &[1.0, 2.0, 3.0]);
    assert_eq!(sys.world.len(), 1);
    assert_eq!(sys.world[0].gid, 100_001, "slot 1, seq 1");
    assert!(!sys.world[0].ghost);
    assert_eq!(
        a.sp_records[0],
        vec![
            KitVal::Num(2.0),
            KitVal::Str("mi".to_string()),
            KitVal::Num(100_001.0),
            KitVal::Num(1.0),
            KitVal::Num(2.0),
            KitVal::Num(3.0),
        ],
        "JS L303: [2, kind, gid, ...data]"
    );
    // the gid sequence advances
    let obj2 = world::WorldObject::new("tw", a.team, Vec3::ZERO);
    spawn(&mut a, &mut sys, obj2, &[]);
    assert_eq!(sys.world[1].gid, 100_002);
}

#[test]
fn tr35_remaining_matches_the_js_ladder() {
    // JS `remaining` (L382-390): the generic `max(0, 1-t/dur)`; booyah
    // `thrown ? 0 : 1`; blower count-scaled; crab the hp/t min.
    let f = fix();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "barrage", 1000.0);
    assert_eq!(remaining(&a), 0.0, "JS L384: no active ⇒ 0");
    start(&mut a, "barrage");
    assert_eq!(remaining(&a), 1.0);
    let dur = specials().barrage.duration;
    a.special_active.as_mut().unwrap().t = dur / 2.0;
    assert!((remaining(&a) - 0.5).abs() < 1e-6);
    a.special_active.as_mut().unwrap().t = dur * 2.0;
    assert_eq!(remaining(&a), 0.0, "clamped at 0");
    // booyah
    let mut b = actor_with_special(t, "booyah", 1000.0);
    start(&mut b, "booyah");
    assert_eq!(remaining(&b), 1.0, "JS L383: held orb reads full");
    if let KindState::Booyah(st) = &mut b.special_active.as_mut().unwrap().state {
        st.thrown = true;
    }
    assert_eq!(remaining(&b), 0.0, "JS L383: thrown ⇒ empty");
    // blower: `(1 - count/max) · (1 - t/dur)`
    let mut c = actor_with_special(t, "blower", 1000.0);
    start(&mut c, "blower");
    let max = specials().blower.max as f32;
    if let KindState::Blower(st) = &mut c.special_active.as_mut().unwrap().state {
        st.count = max / 2.0;
    }
    assert!((remaining(&c) - 0.5).abs() < 1e-6, "JS L386");
    // crab: `min(1 - t/dur, hp/def.hp)`
    let mut d = actor_with_special(t, "crab", 1000.0);
    start(&mut d, "crab");
    if let KindState::Crab(st) = &mut d.special_active.as_mut().unwrap().state {
        st.hp = specials().crab.hp as f32 / 2.0;
    }
    assert!((remaining(&d) - 0.5).abs() < 1e-6, "JS L387");
}

#[test]
fn tr35_filter_damage_gates() {
    let f = fix();
    let t = &f.tuning.player;
    // no special, no shield ⇒ unfiltered (JS L428 `return amount`)
    let mut plain = Actor::new(0, 0, t);
    assert_eq!(
        filter_damage(&mut plain, 20.0, Some((1, 0)), Some(vec3(1.0, 0.0, 0.0))),
        None
    );
    // bubbler shield: absorbed with a capped knock (JS L421-427)
    let mut bub = actor_with_special(t, "bubbler", 1000.0);
    start(&mut bub, "bubbler");
    assert!(
        bub.status.shield > 0.0,
        "IMPL.bubbler.start grants the shield"
    );
    bub.pos = Vec3::ZERO;
    let d = &specials().bubbler;
    let k = (20.0 * d.knock_per_damage).min(d.knock_max);
    let out = filter_damage(&mut bub, 20.0, Some((1, 0)), Some(vec3(10.0, 0.0, 0.0)));
    assert_eq!(out, Some(0.0), "the shield absorbs");
    assert!(
        (bub.vel.x + k).abs() < 1e-4,
        "shoved away from the attacker"
    );
    // kraken: any hit is knocked aside for 0 damage (JS L414)
    let mut kra = actor_with_special(t, "kraken", 1000.0);
    start(&mut kra, "kraken");
    kra.pos = Vec3::ZERO;
    let kd = &specials().kraken;
    let kk = (50.0 * kd.knock_per_damage).min(4.0); // the 4 cap is JS L414
    let out = filter_damage(&mut kra, 50.0, Some((1, 0)), Some(vec3(10.0, 0.0, 0.0)));
    assert_eq!(out, Some(0.0));
    assert!((kra.vel.x + kk).abs() < 1e-4, "JS L430-438: the shove");
    // stamp: mid-guard front hits deflect, rear hits pass (JS L417-420)
    let mut st = actor_with_special(t, "stamp", 1000.0);
    start(&mut st, "stamp"); // face_yaw = a.yaw = 0 ⇒ front is +z
    if let KindState::Stamp(sg) = &mut st.special_active.as_mut().unwrap().state {
        sg.guard = 0.5;
    }
    let out = filter_damage(&mut st, 15.0, Some((1, 0)), Some(vec3(0.0, 0.0, 5.0)));
    assert_eq!(out, Some(0.0), "JS L418: the front arc deflects");
    let out = filter_damage(&mut st, 15.0, Some((1, 0)), Some(vec3(0.0, 0.0, -5.0)));
    assert_eq!(out, None, "a rear hit falls through to the shield check");
    // self damage carries no direction (JS L417 `attacker !== v`): the
    // knock falls back to the facing (yaw 0 ⇒ shove toward −z, JS L435).
    let mut self_hit = actor_with_special(t, "kraken", 1000.0);
    start(&mut self_hit, "kraken");
    self_hit.pos = Vec3::ZERO;
    self_hit.grounded = true;
    let out = filter_damage(&mut self_hit, 10.0, Some((0, 0)), Some(vec3(1.0, 0.0, 0.0)));
    assert_eq!(out, Some(0.0));
    let sk = (10.0 * kd.knock_per_damage).min(4.0);
    assert_eq!(
        self_hit.vel.x, 0.0,
        "JS L433: a self hit has no attacker direction"
    );
    assert!(
        (self_hit.vel.z + sk).abs() < 1e-4,
        "JS L435: the facing fallback shoves along −z"
    );
    assert!(
        (self_hit.vel.y - (1.4 + sk * 0.18)).abs() < 1e-4,
        "JS L436: the grounded knock lifts (1.4 + k·0.18)"
    );
    assert!(!self_hit.grounded);
    // crab: hull absorbs, rear/above hits reach the rider (JS L2014-2026)
    let mut crab_a = actor_with_special(t, "crab", 1000.0);
    start(&mut crab_a, "crab");
    crab_a.pos = Vec3::ZERO;
    let hp0 = crab_a.special_active.as_ref().unwrap().crab_hp();
    let out = filter_damage(&mut crab_a, 10.0, Some((1, 0)), Some(vec3(0.0, 0.0, 5.0)));
    assert_eq!(out, Some(0.0), "the hull eats a front hit");
    let hp1 = crab_a.special_active.as_ref().unwrap().crab_hp();
    assert!(
        (hp0 - hp1 - 10.0).abs() < 1e-4,
        "JS L2023: hull hp -= amount"
    );
    let out = filter_damage(&mut crab_a, 10.0, Some((1, 0)), Some(vec3(0.0, 0.0, -5.0)));
    assert_eq!(out, Some(10.0), "JS L2020: a rear hit reaches the rider");
}

#[test]
fn tr35_net_state_crab_round_trip() {
    // JS `specialNetState` (L2124-2130): crab `roll | firing<<1 | hull<<2`;
    // the ghost side unpacks the same bits (L2131-2136).
    let f = fix();
    let t = &f.tuning.player;
    let mut owner = actor_with_special(t, "crab", 1000.0);
    start(&mut owner, "crab");
    {
        let s = owner.special_active.as_mut().unwrap();
        s.firing = 0.5;
        if let KindState::Crab(st) = &mut s.state {
            st.roll = true;
            st.hull = std::f32::consts::TAU * 0.25;
        }
    }
    let v = net_state(&owner);
    assert_eq!(
        v,
        (64 << 2) | (1 << 1) | 1,
        "JS L2127: hull byte round(0.25·255)=64"
    );
    // slam/storm actives have no def ⇒ 0 (JS L2125)
    let mut slam = actor_with_special(t, "slam", 1000.0);
    slam.special_active = Some(Box::new(SpecialActive::net_placeholder("slam")));
    assert_eq!(net_state(&slam), 0);
    // the ghost unpack
    let mut ghost = actor_with_special(t, "crab", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut ghost,
        &[KitVal::Num(0.0), KitVal::Num(order_index("crab") as f32)],
        &mut sys,
        &f,
    );
    net_apply(&mut ghost, v);
    let KindState::Crab(st) = &ghost.special_active.as_ref().unwrap().state else {
        panic!("crab kind state");
    };
    let net = st.net.unwrap();
    assert!(net.roll && net.firing);
    assert!((net.hull - 64.0 / 255.0 * std::f32::consts::TAU).abs() < 1e-6);
    // a live (non-ghost) active ignores the apply (JS L2132 `s.ghost` guard)
    net_apply(&mut owner, 0);
    let KindState::Crab(st) = &owner.special_active.as_ref().unwrap().state else {
        panic!("crab kind state");
    };
    assert!(st.net.is_none(), "the owner never takes a snapshot");
    // booyah: `(v & 63)/63` with the `netCharge` latch (JS L2135)
    let mut boo = actor_with_special(t, "booyah", 1000.0);
    net_ghost(
        &mut boo,
        &[KitVal::Num(0.0), KitVal::Num(order_index("booyah") as f32)],
        &mut sys,
        &f,
    );
    net_apply(&mut boo, 21);
    assert!((boo.special_active.as_ref().unwrap().booyah_charge() - 21.0 / 63.0).abs() < 1e-6);
}

#[test]
fn tr35_determinism_two_matches_same_seed() {
    // The special wiring must not introduce nondeterminism: two matches on
    // one seed, identical inputs (a special press included), must produce
    // identical event buses and identical record queues.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.weapons;
    let sb = &f.tuning.subs;
    let mut m1 = Match::with_roster(60.0, 13, &sim, t, &f.tuning.match_config, 2);
    let mut m2 = Match::with_roster(60.0, 13, &sim, t, &f.tuning.match_config, 2);
    let mut inputs = vec![ActorInput::default(); m1.actors.len()];
    let mut guard = 0;
    while !m1.playing() {
        m1.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
        m2.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
        guard += 1;
        assert!(guard < 1000, "the intro must end");
    }
    // fill the gauge and press special on actor 0 for one frame
    m1.actors[0].special = m1.actors[0].special_cost;
    m2.actors[0].special = m2.actors[0].special_cost;
    inputs[0].special = true;
    m1.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
    m2.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
    inputs[0].special = false;
    for _ in 0..300 {
        m1.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
        m2.step(FIXED_DT, &inputs, &sim, t, w, sb, None);
    }
    assert_eq!(m1.events, m2.events, "the event buses must agree");
    assert!(
        m1.events
            .iter()
            .any(|e| matches!(e, MatchEvent::SpecialUse { .. })),
        "the press must reach the bus"
    );
    assert_eq!(
        m1.actors[0].sp_records, m2.actors[0].sp_records,
        "the record queues must agree"
    );
    assert_eq!(m1.actors[0].form, m2.actors[0].form);
    assert_eq!(m1.actors[0].special, m2.actors[0].special);
}

// ---------------------------------------------------------------------------
// Task 35 batch 2 — the aura family: slam impact, storm cloud, bubbler
// shield, sonar reveal, kraken trample (per-value ladders)
// ---------------------------------------------------------------------------

/// A live match past the intro on the embedded Tidewater layout.
fn tr35_match(f: &Fix, seed: u64, team_size: usize) -> Match {
    let sim = f.sim();
    let t = &f.tuning.player;
    let w = &f.tuning.weapons;
    let sb = &f.tuning.subs;
    let mut m = Match::with_roster(60.0, seed, &sim, t, &f.tuning.match_config, team_size);
    let mut guard = 0;
    while !m.playing() {
        m.step(FIXED_DT, &[], &sim, t, w, sb, None);
        guard += 1;
        assert!(guard < 1000, "the intro must end");
    }
    m
}

#[test]
fn tr35_b2_slam_impact_paint_and_damage_ladder() {
    // JS `Actor._slamImpact` (actor.js L900-924): the paint is credited
    // through `addTurfNoSpecial` (L910 — the gauge never recharges), every
    // enemy inside `radius` with LOS takes the kill-radius ladder (L916-923:
    // flat `damageMax` inside `killRadius`, then the *linear* 30 % tail).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().slam;
    assert_eq!(
        (d.radius, d.kill_radius, d.damage_max, d.damage_min),
        (5.2, 3.2, 180.0, 55.0)
    );
    let build = |seed: u64| {
        let mut s0 = actor_with_special(t, "slam", 1000.0);
        s0.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
        s0.special = 500.0;
        let mut e0 = Actor::new(1, 0, t);
        e0.spawn_at(vec3(2.0, 0.05, 0.0), std::f32::consts::PI, &sim, t);
        e0.invuln = 0.0;
        let mut e1 = Actor::new(1, 1, t);
        e1.spawn_at(vec3(4.0, 0.05, 0.0), std::f32::consts::PI, &sim, t);
        e1.invuln = 0.0;
        (
            vec![s0, e0, e1],
            crate::paint::PaintGrid::new(&f.world),
            crate::weapon::ProjectileSim::new(seed),
            Rng::new(seed),
        )
    };
    let (mut actors, mut paint, mut ps, mut rng) = build(99);
    let c = actors[0].pos;
    slam_impact(
        &mut rng,
        &mut ps,
        &mut actors,
        0,
        c,
        &f.world,
        &mut paint,
        None,
        t,
    );
    // JS L920: `dist < killRadius` ⇒ flat damageMax — 2 m kills the 100 hp
    // enemy outright.
    assert!(
        !actors[1].alive,
        "JS L920: 180 inside killRadius 3.2 splats"
    );
    // 4 m: `55 + (180−55)·0.3·(1 − (4−3.2)/(5.2−3.2))` = 55 + 37.5·0.6 = 77.5.
    let want: f32 = 55.0 + (180.0 - 55.0) * 0.3 * (1.0 - (4.0 - 3.2) / (5.2 - 3.2));
    assert!((want - 77.5).abs() < 1e-3, "the ladder formula: {want}");
    assert!(
        (actors[2].hp - (100.0 - want)).abs() < 1e-3,
        "JS L920: the linear tail — hp {} vs {}",
        actors[2].hp,
        100.0 - want
    );
    assert!(
        actors[0].turf > 0.0,
        "the 1 + 9 splats credit turf (JS L903-910)"
    );
    assert_eq!(
        actors[0].special, 500.0,
        "JS L910: `addTurfNoSpecial` — the impact never recharges"
    );
    // The 37 draws (JS L903-908 order) come off the match stream: the same
    // seed must replay the identical area.
    let (mut actors2, mut paint2, mut ps2, mut rng2) = build(99);
    let c2 = actors2[0].pos;
    slam_impact(
        &mut rng2,
        &mut ps2,
        &mut actors2,
        0,
        c2,
        &f.world,
        &mut paint2,
        None,
        t,
    );
    assert_eq!(
        actors[0].turf, actors2[0].turf,
        "the slam draws are deterministic off the fixed seed"
    );
    assert_eq!(actors[2].hp, actors2[2].hp);
}

#[test]
fn tr35_b2_slam_ghost_is_a_bare_active() {
    // JS `_startGhost` L277-285: `slam` has neither a GHOST nor an IMPL
    // entry (specials.js L2049-2121) → a bare ghost active. The actor's
    // phase machine never engages for a ghost: no leap, no impact, no
    // paint, no damage — the slot only mirrors the "special running" flag.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let mut a = actor_with_special(t, "slam", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    let pos = a.pos;
    let vel = a.vel;
    let mut foe = Actor::new(1, 0, t);
    foe.spawn_at(vec3(0.5, 0.05, 0.0), std::f32::consts::PI, &sim, t);
    foe.invuln = 0.0;
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut a,
        &[KitVal::Num(0.0), KitVal::Num(order_index("slam") as f32)],
        &mut sys,
        &f,
    );
    let s = a.special_active.as_ref().expect("the ghost active");
    assert!(s.ghost && s.id == "slam", "JS L282: the bare ghost start");
    assert_eq!(a.pos, pos, "no leap: the phase machine never engages");
    assert_eq!(a.vel, vel);
    assert!(a.sp_records.is_empty(), "ghosts never record");
    let mut actors = [a, foe];
    for _ in 0..120 {
        ghost_tick(&mut actors[0], FIXED_DT);
    }
    assert!(
        actors[0].special_active.is_some(),
        "JS L297: `s.dur &&` — a dur-0 ghost slam never self-expires"
    );
    // JS L364 makes even `end()` slam-immune: the ghost rides until the
    // playback flag-off grace clears it (the generic L383 path, covered by
    // `tr35_playback_ghost_gets_the_off_grace`).
    end(&mut actors[0], "net");
    assert!(
        actors[0].special_active.is_some(),
        "JS L364: slam is immune to end(), ghosts included"
    );
    let evs = actors[0].drain_events();
    assert!(
        !evs.iter()
            .any(|e| matches!(e, ActorEvent::SpecialSlam { .. })),
        "a ghost slam never emits the impact marker"
    );
    assert_eq!(actors[0].turf, 0.0, "a ghost slam paints nothing");
    assert_eq!(actors[1].hp, 100.0, "a ghost slam judges no damage");
    assert!(actors[1].alive);
}

#[test]
fn tr35_b2_storm_cloud_rain_dps_and_expiry() {
    // JS `_updateClouds` (weapons.js L1760-1795): drift always; rain + dps
    // only while `t < dur − 0.3`; the cloud pops at `t >= dur`. dps 34·dt
    // hits *local* enemies only (L1783 `e.remote` skip), the rain paints
    // every 0.045 s and credits `addTurfNoSpecial` (L1779).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let sp = &specials().storm;
    assert_eq!(
        (sp.duration, sp.radius, sp.dps, sp.drift_speed),
        (6.5, 3.4, 34.0, 1.1)
    );
    // Center on a spawn pad: open sky, so the cloud's LOS ladder runs
    // (the origin sits under indoor geometry and would block the rays).
    let pp = f.layout.stage.spawn_pads[0];
    let pad = vec3(pp[0], pp[1], pp[2]);
    let spawn = world::CloudSpawn {
        owner_slot: 0,
        team: 0,
        pos: pad,
        dir: vec3(0.0, 0.0, 1.0),
        ghost: false,
    };
    let mut objects = vec![world::make_cloud(&f.world, &spawn, sp.duration)];
    let obj = &objects[0];
    let cloud = match &obj.body {
        world::ObjectBody::Cloud(c) => *c,
        other => panic!("expected a Cloud body, got {other:?}"),
    };
    assert_eq!(obj.kind, "cloud");
    assert!(
        (obj.pos.y - (cloud.ground_y + 4.6)).abs() < 1e-6,
        "JS L1511/L1535: raycast floor, hang at groundY + 4.6"
    );
    let mut owner = actor_with_special(t, "storm", 1000.0);
    owner.spawn_at(pad + vec3(-2.0, 0.05, -2.0), 0.0, &sim, t);
    owner.special = 500.0;
    let mut vic = Actor::new(1, 0, t);
    vic.spawn_at(pad + vec3(0.3, 0.05, 0.3), std::f32::consts::PI, &sim, t);
    vic.invuln = 0.0;
    let mut rem = Actor::new(1, 1, t);
    rem.spawn_at(pad + vec3(-0.3, 0.05, -0.3), std::f32::consts::PI, &sim, t);
    rem.invuln = 0.0;
    let mut actors = vec![owner, vic, rem];
    let remote = [false, false, true];
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng = Rng::new(7);
    world::step_clouds(
        &mut objects,
        FIXED_DT,
        &mut actors,
        &f.world,
        &mut paint,
        &mut rng,
        Some(&remote[..]),
        t,
    );
    // JS L1789: `damage(dps·dt)` — 34/60 this frame, local victim only.
    assert!(
        (actors[1].hp - (100.0 - 34.0 * FIXED_DT)).abs() < 1e-3,
        "JS L1789: dps 34·dt on the local enemy (hp {})",
        actors[1].hp
    );
    assert_eq!(actors[2].hp, 100.0, "JS L1783: remote victims are skipped");
    // JS L1768: the drift is `dir · 1.1 · dt` every frame.
    assert!(
        (objects[0].pos.z - (pad.z + 1.1 * FIXED_DT)).abs() < 1e-5,
        "JS L1768: drift 1.1 m/s along dir (got z {})",
        objects[0].pos.z
    );
    // JS L1775-1780: rain_t starts at 0 ⇒ the first drop fires immediately.
    assert!(
        actors[0].turf > 0.0,
        "JS L1779: the rain credits the owner's turf"
    );
    assert_eq!(
        actors[0].special, 500.0,
        "JS L1779: the rain never recharges"
    );
    if let world::ObjectBody::Cloud(c) = &objects[0].body {
        assert!(
            (c.rain_t - (0.045 - FIXED_DT)).abs() < 1e-5,
            "JS L1775: one drop per 0.045 s window"
        );
    }
    // JS L1771: the last 0.3 s is quiet — drift runs, rain + dps do not.
    if let world::ObjectBody::Cloud(c) = &mut objects[0].body {
        c.t = 6.5 - 0.2;
    }
    let hp = actors[1].hp;
    let turf = actors[0].turf;
    world::step_clouds(
        &mut objects,
        FIXED_DT,
        &mut actors,
        &f.world,
        &mut paint,
        &mut rng,
        Some(&remote[..]),
        t,
    );
    assert_eq!(actors[1].hp, hp, "JS L1771: no dps inside the 0.3 s tail");
    assert_eq!(
        actors[0].turf, turf,
        "JS L1771: no rain inside the 0.3 s tail"
    );
    // JS L1793: the cloud pops at `t >= dur`.
    if let world::ObjectBody::Cloud(c) = &mut objects[0].body {
        c.t = 6.5 - 0.5 * FIXED_DT;
    }
    world::step_clouds(
        &mut objects,
        FIXED_DT,
        &mut actors,
        &f.world,
        &mut paint,
        &mut rng,
        Some(&remote[..]),
        t,
    );
    assert!(objects.is_empty(), "JS L1793: `clouds.splice` at t ≥ dur");
    // A ghost cloud rains nothing (L1779 `!c.ghost` half of the guard) but
    // still hurts this screen's locals (L1782 runs regardless).
    let gspawn = world::CloudSpawn {
        owner_slot: 0,
        team: 0,
        pos: pad,
        dir: vec3(0.0, 0.0, 1.0),
        ghost: true,
    };
    let mut gobjects = vec![world::make_cloud(&f.world, &gspawn, sp.duration)];
    let mut owner2 = actor_with_special(t, "storm", 1000.0);
    owner2.spawn_at(pad + vec3(-2.0, 0.05, -2.0), 0.0, &sim, t);
    let mut vic2 = Actor::new(1, 0, t);
    vic2.spawn_at(pad + vec3(0.3, 0.05, 0.3), std::f32::consts::PI, &sim, t);
    vic2.invuln = 0.0;
    let mut gactors = vec![owner2, vic2];
    world::step_clouds(
        &mut gobjects,
        FIXED_DT,
        &mut gactors,
        &f.world,
        &mut paint,
        &mut rng,
        None,
        t,
    );
    assert_eq!(
        gactors[0].turf, 0.0,
        "JS L1779: a ghost cloud paints nothing"
    );
    assert!(
        gactors[1].hp < 100.0,
        "JS L1782: a ghost cloud still rains dps on locals"
    );
}

#[test]
fn tr35_b2_bubbler_shield_ladder() {
    // JS `giveShield` L486-488 (max + owner flag) and the `update` ladder
    // L542-563: decay, dead/no-shield drops, owner-only sharing inside
    // `shareRange` 1.5 with the remaining time and `owner = false`.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().bubbler;
    assert_eq!((d.duration, d.share_range), (6.5, 1.5));
    // giveShield takes the max of what is left (JS L487).
    let mut a = actor_with_special(t, "bubbler", 1000.0);
    a.status.shield = 2.0;
    give_shield(&mut a, 6.5, true);
    assert_eq!(a.status.shield, 6.5, "JS L487: max(2, 6.5)");
    assert!(a.status.shield_owner, "JS L488: the owner flag is stamped");
    give_shield(&mut a, 3.0, false);
    assert_eq!(
        a.status.shield, 6.5,
        "JS L487: max(6.5, 3) keeps the longer"
    );
    assert!(
        !a.status.shield_owner,
        "JS L488: the flag is overwritten per grant"
    );
    // IMPL.bubbler.start grants the full field (L1257).
    let mut owner = actor_with_special(t, "bubbler", 1000.0);
    owner.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut owner, "bubbler");
    assert!(
        (owner.status.shield - 6.5).abs() < 1e-6,
        "JS L1257: giveShield(def.duration)"
    );
    assert!(owner.status.shield_owner);
    // The sharing ladder: mate1 inside 1.5 shares; mate2 (1.4 from mate1 but
    // 2.4 from the owner) does not — a shared shield never re-shares (L556
    // `a._shieldOwner`); the enemy team is skipped (L558).
    let mut mate1 = Actor::new(0, 1, t);
    mate1.spawn_at(vec3(1.0, 0.05, 0.0), 0.0, &sim, t);
    let mut mate2 = Actor::new(0, 2, t);
    mate2.spawn_at(vec3(2.4, 0.05, 0.0), 0.0, &sim, t);
    let mut foe = Actor::new(1, 0, t);
    foe.spawn_at(vec3(0.5, 0.05, 0.0), 0.0, &sim, t);
    let mut actors = vec![owner, mate1, mate2, foe];
    update_shields(&mut actors, FIXED_DT);
    assert!(
        (actors[0].status.shield - (6.5 - FIXED_DT)).abs() < 1e-5,
        "JS L545: the field decays by dt"
    );
    // JS L545/L559-560: the owner shares its *decayed* value (6.5 − dt) and
    // the mate's own loop iteration decays it once more in the same frame
    // (roster order owner → mate) — the observable is 6.5 − 2·dt.
    assert!(
        (actors[1].status.shield - (6.5 - 2.0 * FIXED_DT)).abs() < 1e-5,
        "JS L560: the mate takes the remaining time, then decays: {}",
        actors[1].status.shield
    );
    assert!(
        !actors[1].status.shield_owner,
        "JS L560: shared shields are not owner fields"
    );
    assert_eq!(
        actors[2].status.shield, 0.0,
        "JS L556: a shared shield never re-shares"
    );
    assert_eq!(actors[3].status.shield, 0.0, "JS L558: enemies never share");
    // An owner-flagged field without a bubbler active never shares (L556
    // `a.specialActive?.id === 'bubbler'`).
    let mut fake = Actor::new(0, 0, t);
    fake.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    fake.status.shield = 5.0;
    fake.status.shield_owner = true;
    start(&mut fake, "slam"); // an active, but not the bubbler's
    let mut near = Actor::new(0, 1, t);
    near.spawn_at(vec3(0.5, 0.05, 0.0), 0.0, &sim, t);
    let mut actors = vec![fake, near];
    update_shields(&mut actors, FIXED_DT);
    assert_eq!(
        actors[1].status.shield, 0.0,
        "JS L556: the id check gates sharing"
    );
    // A dead carrier drops the field (L544) and shares nothing.
    let mut dead = Actor::new(0, 0, t);
    dead.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    dead.status.shield = 5.0;
    dead.status.shield_owner = true;
    start(&mut dead, "bubbler");
    dead.alive = false;
    let mut near2 = Actor::new(0, 1, t);
    near2.spawn_at(vec3(0.5, 0.05, 0.0), 0.0, &sim, t);
    let mut actors = vec![dead, near2];
    update_shields(&mut actors, FIXED_DT);
    assert_eq!(actors[0].status.shield, 0.0, "JS L544: dead ⇒ drop");
    assert_eq!(
        actors[1].status.shield, 0.0,
        "JS L544: a dead owner never shares"
    );
    // The ghost consumption runs the same grant (no GHOST.bubbler entry ⇒
    // the L282 fallback to `IMPL.bubbler.start`).
    let mut ghost = actor_with_special(t, "bubbler", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut ghost,
        &[KitVal::Num(0.0), KitVal::Num(order_index("bubbler") as f32)],
        &mut sys,
        &f,
    );
    assert!(ghost.special_active.as_ref().is_some_and(|s| s.ghost));
    assert!(
        (ghost.status.shield - 6.5).abs() < 1e-6,
        "JS L282: the ghost start runs IMPL.bubbler.start"
    );
}

#[test]
fn tr35_b2_sonar_reveal_ladder() {
    // JS `IMPL.sonar.start` L1263-1271: the active runs the 0.6 s `cast`
    // window; every other-team actor gets a fresh `duration` reveal — a
    // plain assignment (L1268, not max) — plus the echo-orb track (L1269).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().sonar;
    assert_eq!(
        (d.duration, d.cast, d.slow, d.ink_mul),
        (8.0, 0.6, 0.8, 1.4)
    );
    let mut caster = actor_with_special(t, "sonar", 1000.0);
    caster.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut caster, "sonar");
    {
        let s = caster.special_active.as_ref().unwrap();
        assert!(s.body, "JS L1264: the cast owns the body");
        assert!(
            (s.dur - 0.6).abs() < 1e-6,
            "JS L1264: the active runs `cast`, not `duration`"
        );
    }
    let mut mate = Actor::new(0, 1, t);
    mate.spawn_at(vec3(1.0, 0.05, 0.0), 0.0, &sim, t);
    mate.status.reveal = 3.0;
    let mut e1 = Actor::new(1, 0, t);
    e1.spawn_at(vec3(2.0, 0.05, 0.0), 0.0, &sim, t);
    e1.status.reveal = 100.0; // a stale long reveal must be *overwritten*
    e1.status.track = 9.0;
    e1.status.track_team = Some(0);
    let mut e2 = Actor::new(1, 1, t);
    e2.spawn_at(vec3(3.0, 0.05, 0.0), 0.0, &sim, t);
    let mut actors = vec![caster, mate, e1, e2];
    sonar::apply_reveal(0, &mut actors);
    assert!(
        (actors[2].status.reveal - 8.0).abs() < 1e-6,
        "JS L1268: assignment, not max (was 100)"
    );
    assert_eq!(actors[2].status.reveal_team, Some(0));
    assert!(
        (actors[2].status.track - 9.0).abs() < 1e-6,
        "JS L1269 → subs.js L537-542: same-team track keeps the max"
    );
    assert!(
        (actors[3].status.reveal - 8.0).abs() < 1e-6,
        "JS L1268: every enemy takes the fresh reveal"
    );
    assert!(
        (actors[3].status.track - 8.0).abs() < 1e-6,
        "JS L1269: a fresh track is `time`"
    );
    assert_eq!(actors[3].status.track_team, Some(0));
    assert!(
        (actors[1].status.reveal - 3.0).abs() < 1e-6,
        "JS L1267: teammates are skipped"
    );
    assert!(actors[0].special_active.as_ref().unwrap().sonar_revealed());
    // A re-cast never stacks the reveal (assignment again).
    sonar::apply_reveal(0, &mut actors);
    assert!(
        (actors[2].status.reveal - 8.0).abs() < 1e-6,
        "JS L1268: the second cast re-assigns 8, not 16"
    );
    // The ghost consumption runs the same start (GHOST.sonar === IMPL, L2051).
    let mut ghost = actor_with_special(t, "sonar", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut ghost,
        &[KitVal::Num(0.0), KitVal::Num(order_index("sonar") as f32)],
        &mut sys,
        &f,
    );
    let s = ghost.special_active.as_ref().unwrap();
    assert!(s.ghost && s.body);
    assert!(
        (s.dur - 0.6).abs() < 1e-6,
        "JS L282: the ghost runs IMPL.sonar.start"
    );
}

#[test]
fn tr35_b2_sonar_slow_and_ink_burn() {
    // JS actor.js L449: a revealed kid's target speed takes `sonar.slow`
    // (0.8); L375: a revealed kid's ink spend takes `sonar.inkMul` (1.4).
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    // The slow: identical actors, one revealed, running along their facing
    // (+z, yaw 0) — the damp target scales by exactly 0.8.
    let mut plain = Actor::new(0, 0, t);
    plain.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    let mut rev = Actor::new(0, 1, t);
    rev.spawn_at(vec3(0.0, 0.05, 3.0), 0.0, &sim, t);
    rev.status.reveal = 8.0;
    let inp = ActorInput {
        move_dir: vec3(0.0, 0.0, 1.0),
        ..ActorInput::default()
    };
    let paint = crate::paint::PaintGrid::new(&f.world);
    let sb = &f.tuning.subs;
    for _ in 0..30 {
        plain.step(FIXED_DT, &inp, &sim, &paint, t, sb, None, None);
        rev.step(FIXED_DT, &inp, &sim, &paint, t, sb, None, None);
    }
    assert!(
        plain.vel.z > 1.0,
        "the plain kid is running: {}",
        plain.vel.z
    );
    assert!(
        (rev.vel.z - 0.8 * plain.vel.z).abs() < 1e-4,
        "JS L449: the revealed speed target scales by sonar.slow ({} vs {})",
        rev.vel.z,
        plain.vel.z
    );
    // The ink burn: two seeded matches, one press frame — the revealed
    // spend is exactly `ink_mul` times the unrevealed one (JS L375, the
    // match-side increment ladder at match_.rs L846-848).
    let w = &f.tuning.weapons;
    let mut ma = tr35_match(&f, 21, 1);
    let mut mb = tr35_match(&f, 21, 1);
    mb.actors[0].status.reveal = 8.0;
    let fire = [ActorInput {
        fire: true,
        ..ActorInput::default()
    }];
    ma.step(FIXED_DT, &fire, &sim, t, w, sb, None);
    mb.step(FIXED_DT, &fire, &sim, t, w, sb, None);
    let spent_a = 100.0 - ma.actors[0].ink;
    assert!(spent_a > 0.0, "the shooter paid ink on the press frame");
    assert!(
        (mb.actors[0].ink - (100.0 - 1.4 * spent_a)).abs() < 1e-3,
        "JS L375: revealed spend ×1.4 — ink {} vs the expected {}",
        mb.actors[0].ink,
        100.0 - 1.4 * spent_a
    );
}

#[test]
fn tr35_b2_kraken_trample_ladder() {
    // JS `IMPL.kraken` (specials.js L1410-1469): the start flags, the
    // jump-attack trigger (hop / dive), the hop boost, the landing blast
    // and the movement trail.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().kraken;
    assert_eq!(
        (
            d.speed,
            d.hop_vel,
            d.attack_vel,
            d.attack_fwd,
            d.cooldown,
            d.radius,
            d.damage,
            d.paint_radius
        ),
        (7.2, 10.5, 8.5, 6.5, 0.65, 2.3, 200.0, 1.15)
    );
    // start (L1412-1413).
    let mut a = actor_with_special(t, "kraken", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "kraken");
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(s.no_squid && s.ink_proof && s.face_move, "JS L1412");
        assert_eq!(s.speed, Some(7.2), "JS L1412: `speed: def.speed`");
        assert_eq!(s.accel, Some(45.0), "JS L1412: `accel: 45` (D-T35-4)");
    }
    assert_eq!(a.form, crate::actor::Form::Kid, "JS L1413: forced kid form");
    // The weapon hook owns the trigger (L1434 `return true`).
    a.aim_yaw = 0.0;
    a.grounded = true;
    a.fire_gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        sub: false,
        sub_released: false,
    };
    assert!(
        weapon_hook_1(&mut a, FIXED_DT, &f),
        "JS L1434: the jump-attack skips the weapon runner"
    );
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Kraken(st) = &s.state else {
            panic!("kraken state");
        };
        assert!(
            (st.cd - 0.65).abs() < 1e-6,
            "JS L1425: `s.cd = def.cooldown`"
        );
        assert!(st.attack, "JS L1426: the attack is airborne");
    }
    assert!(
        (a.vel.y - 8.5).abs() < 1e-6,
        "JS L1430: `vel.y = def.attackVel` (got {})",
        a.vel.y
    );
    assert!(
        (a.vel.z - 3.25).abs() < 1e-6,
        "JS L1429: half `attackFwd` along the aim (yaw 0 ⇒ +z)"
    );
    assert!(!a.grounded, "JS L1430: the hop leaves the ground");
    // The cd decays even without a press (L1424).
    a.fire_gate.pressed = false;
    assert!(
        weapon_hook_1(&mut a, FIXED_DT, &f),
        "JS L1434: the hook keeps owning the trigger"
    );
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Kraken(st) = &s.state else {
            panic!("kraken state");
        };
        assert!(
            (st.cd - (0.65 - FIXED_DT)).abs() < 1e-5,
            "JS L1424: `s.cd -= dt`"
        );
    }
    // The airborne dive (L1431): `vel.y = min(vel.y, −16)`.
    {
        let s = a.special_active.as_mut().unwrap();
        let KindState::Kraken(st) = &mut s.state else {
            panic!("kraken state");
        };
        st.cd = 0.0;
    }
    a.grounded = false;
    a.vel.y = 5.0;
    a.fire_gate.pressed = true;
    assert!(weapon_hook_1(&mut a, FIXED_DT, &f));
    assert!(
        (a.vel.y + 16.0).abs() < 1e-6,
        "JS L1431: the dive clamps to −16 (got {})",
        a.vel.y
    );
    // The hop boost (L1438): a rising, non-attack airborne edge takes
    // `max(vel.y, hopVel)`.
    let mut b = actor_with_special(t, "kraken", 1000.0);
    b.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut b, "kraken");
    b.grounded = false;
    b.vel.y = 5.0;
    let mut ps = crate::weapon::ProjectileSim::new(3);
    let mut subs = crate::sub::SubSystem::new(3);
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng = Rng::new(5);
    let mut actors = vec![b];
    kraken::tick(
        &mut rng,
        &mut ps,
        &mut actors,
        0,
        FIXED_DT,
        &f.world,
        &mut paint,
        &mut subs,
        None,
        t,
    );
    assert!(
        (actors[0].vel.y - 10.5).abs() < 1e-6,
        "JS L1438: the hop takes `max(vel.y, hopVel)` (got {})",
        actors[0].vel.y
    );
    // The landing blast (L1439-1451): attack + air→ground edge ⇒ the
    // `special:slam` marker (radius 2.3), the owner-side paint credit and
    // `blast(…, radius, damage, damage·0.6, 'kraken', radius·0.7)`.
    let mut c = actor_with_special(t, "kraken", 1000.0);
    c.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut c, "kraken");
    c.special = 500.0;
    {
        let s = c.special_active.as_mut().unwrap();
        let KindState::Kraken(st) = &mut s.state else {
            panic!("kraken state");
        };
        st.air = true;
        st.attack = true;
    }
    c.grounded = true;
    let mut foe = Actor::new(1, 0, t);
    foe.spawn_at(vec3(1.0, 0.05, 0.0), std::f32::consts::PI, &sim, t);
    foe.invuln = 0.0;
    let mut actors = vec![c, foe];
    kraken::tick(
        &mut rng,
        &mut ps,
        &mut actors,
        0,
        FIXED_DT,
        &f.world,
        &mut paint,
        &mut subs,
        None,
        t,
    );
    // blast centre `c + 0.4y` vs the foe's `pos + 0.8y`: d = √(1 + 0.16) ≈
    // 1.077 ≤ killRadius 1.61 ⇒ flat `damage` 200 ⇒ the 100 hp foe splats.
    assert!(!actors[1].alive, "JS L1449: 200 inside killRadius 2.3·0.7");
    assert!(
        ps.drain_events().iter().any(|e| matches!(
            e,
            crate::weapon::SimEvent::Hit { damage, killed: true, .. }
                if (*damage - 200.0).abs() < 1e-3
        )),
        "JS L1449: the blast's flat damage rides the hit event"
    );
    let evs = actors[0].drain_events();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            ActorEvent::SpecialSlam { radius, .. } if (*radius - 2.3).abs() < 1e-6
        )),
        "JS L1448: the FORWARD slam marker carries def.radius"
    );
    assert!(
        actors[0].turf > 0.0,
        "JS L1444: the landing paint is credited"
    );
    assert_eq!(
        actors[0].special, 500.0,
        "JS L1444: `addTurfNoSpecial` never recharges"
    );
    // The movement trail (L1452-1456): ground speed accrues, >0.5 m paints
    // a `paintRadius` splat and resets the accumulator. A fresh paint grid:
    // the blast section already inked this spot, and `splat` credits only
    // *new* area.
    let mut paint2 = crate::paint::PaintGrid::new(&f.world);
    let mut e = actor_with_special(t, "kraken", 1000.0);
    e.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut e, "kraken");
    e.grounded = true;
    e.vel = vec3(10.0, 0.0, 0.0);
    {
        let s = e.special_active.as_mut().unwrap();
        let KindState::Kraken(st) = &mut s.state else {
            panic!("kraken state");
        };
        st.trail = 0.49;
    }
    let mut actors = vec![e];
    kraken::tick(
        &mut rng,
        &mut ps,
        &mut actors,
        0,
        FIXED_DT,
        &f.world,
        &mut paint2,
        &mut subs,
        None,
        t,
    );
    assert!(
        actors[0].turf > 0.0,
        "JS L1455: the trail splat credits turf"
    );
    {
        let s = actors[0].special_active.as_ref().unwrap();
        let KindState::Kraken(st) = &s.state else {
            panic!("kraken state");
        };
        assert_eq!(st.trail, 0.0, "JS L1455: the accumulator resets on paint");
    }
    // A non-kraken active never takes the weapon hook (L359-362).
    let mut bub = actor_with_special(t, "bubbler", 1000.0);
    start(&mut bub, "bubbler");
    assert!(
        !weapon_hook_1(&mut bub, FIXED_DT, &f),
        "JS L361: only the trigger-taking kinds skip the runner"
    );
    // The ghost consumption runs IMPL.kraken.start (GHOST.kraken has only
    // `tick`, L2103 → the L282 fallback).
    let mut ghost = actor_with_special(t, "kraken", 1000.0);
    let mut sys = SpecialSystem::new();
    net_ghost(
        &mut ghost,
        &[KitVal::Num(0.0), KitVal::Num(order_index("kraken") as f32)],
        &mut sys,
        &f,
    );
    assert_eq!(ghost.form, crate::actor::Form::Kid, "JS L1413 via L282");
    assert_eq!(
        ghost.special_active.as_ref().unwrap().speed,
        Some(7.2),
        "JS L282: the ghost start runs IMPL.kraken.start"
    );
}

// ---------------------------------------------------------------------------
// batch 3 — the weapon-trigger family, per-kind value ladders
// ---------------------------------------------------------------------------

#[test]
fn tr35_b3_strike_ladder() {
    // JS `IMPL.strike` (specials.js L1280-1327), tuning config.js L428-429.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().strike;
    assert_eq!(
        (d.aim_time, d.flight, d.radius, d.duration, d.dps, d.pull),
        (7.0, 2.2, 5.5, 4.5, 62.0, 1.6)
    );
    let mut a = actor_with_special(t, "strike", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "strike");
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(s.body, "JS L1282: strike is a body kind");
        assert_eq!(s.dur, 0.0, "JS L1282: the aim ladder ends it, not `dur`");
        let KindState::Strike(st) = &s.state else {
            panic!("strike state");
        };
        assert!(st.aiming && !st.confirm && st.auto_t.is_none());
        // JS L1283: the raw aim point 16 m along the aim yaw.
        assert_eq!(st.target, vec3(0.0, 0.0, 16.0));
    }
    // The roster-side half (JS L1284-1285): bounds clamp, bot target, autoT.
    let mut enemy = Actor::new(1, 1, t);
    enemy.spawn_at(vec3(3.0, 0.05, 6.0), 0.0, &sim, t);
    let mut actors = vec![a, enemy];
    let sys = SpecialSystem::new();
    let paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng = Rng::new(11);
    strike::begin(0, &mut actors, &sys, &f.world, &paint, &mut rng);
    let (target, auto_t) = {
        let s = actors[0].special_active.as_ref().unwrap();
        let KindState::Strike(st) = &s.state else {
            panic!("strike state");
        };
        (
            st.target,
            st.auto_t.expect("JS L1285: bot-style actors draw autoT"),
        )
    };
    let b = &f.world.bounds;
    assert!(
        target.x >= b.min_x + 2.0 - 1e-3 && target.x <= b.max_x - 2.0 + 1e-3,
        "JS L1284: clamped into bounds ± 2 (x {})",
        target.x
    );
    assert!(
        target.z >= b.min_z + 2.0 - 1e-3 && target.z <= b.max_z - 2.0 + 1e-3,
        "JS L1284: clamped into bounds ± 2 (z {})",
        target.z
    );
    assert_eq!(target.y, 0.0, "JS L1325: the target rides the y = 0 plane");
    assert!(
        (0.7..1.3).contains(&auto_t),
        "JS L1285: `0.7 + rand*0.6` (got {auto_t})"
    );
    // The auto-confirm timer raises the launch flag (JS L1291-1292).
    {
        let s = actors[0].special_active.as_mut().unwrap();
        s.t = auto_t;
        strike::body(s, FIXED_DT);
    }
    assert!(
        actors[0].peek_strike_launch(),
        "JS L1292: the launch flag is raised for the match"
    );
    // The launch (JS L1294-1301): missile + record + end('launch').
    let mut sys = SpecialSystem::new();
    strike::launch(0, &mut actors, &mut sys, &f.world);
    assert_eq!(sys.world.len(), 1, "JS L1298: `new Missile(this, a, to)`");
    let w = &sys.world[0];
    assert_eq!(w.kind, "mi");
    assert!(!w.ghost && w.gid != 0);
    let world::ObjectBody::Missile(mi) = &w.body else {
        panic!("missile body");
    };
    assert_eq!(mi.flight, 2.2, "JS L780: `flight = SPECIALS.strike.flight`");
    assert_eq!(
        w.pos,
        actors[0].pos + vec3(0.0, 1.6, 0.0),
        "JS L779: the missile starts above the thrower"
    );
    let a = &actors[0];
    assert!(
        a.special_active.is_none(),
        "JS L1300: launch ends the special"
    );
    assert_eq!(a.sp_records.len(), 3, "start + the missile put + the end");
    assert_eq!(
        a.sp_records[0],
        vec![KitVal::Num(0.0), KitVal::Num(order_index("strike") as f32)]
    );
    assert_eq!(a.sp_records[1][1], KitVal::Str("mi".to_string()));
    assert_eq!(a.sp_records[1].len(), 3 + 3, "JS L1298: the `to` triple");
    assert_eq!(
        a.sp_records[2],
        vec![KitVal::Num(1.0), KitVal::Str("launch".to_string())]
    );
}

#[test]
fn tr35_b3_zooka_volleys() {
    // JS `IMPL.zooka` (specials.js L1330-1348), tuning config.js L431-432.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().zooka;
    assert_eq!(
        (
            d.duration,
            d.interval,
            d.speed,
            d.range,
            d.damage,
            d.height,
            d.radius,
            d.paint_every,
            d.paint_radius
        ),
        (6.0, 1.0, 34.0, 44.0, 180.0, 2.8, 0.55, 0.7, 0.95)
    );
    let mut a = actor_with_special(t, "zooka", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "zooka");
    {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(
            s.speed,
            Some(4.2),
            "JS L1331: walk speed (hardcoded, D-T35-4)"
        );
        assert!(s.aim_face && s.no_squid, "JS L1331");
        let KindState::Zooka(st) = &s.state else {
            panic!("zooka state");
        };
        assert_eq!(
            st.cd, 0.25,
            "JS L1331: initial cooldown (hardcoded, D-T35-4)"
        );
    }
    let mut sys = SpecialSystem::new();
    let mut rng = Rng::new(4);
    a.fire_gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        sub: false,
        sub_released: false,
    };
    // 20 held-trigger frames: the 0.25 s cooldown expires once, the 1.0 s
    // interval re-arms (JS L1333-1335).
    for _ in 0..20 {
        assert!(
            weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng),
            "JS L1345: the special owns the trigger"
        );
    }
    assert_eq!(sys.world.len(), 1, "JS L1338: one twister per interval");
    let w = &sys.world[0];
    assert_eq!(w.kind, "tw");
    assert!(!w.ghost && w.gid != 0);
    let world::ObjectBody::Twister(tw) = &w.body else {
        panic!("twister body");
    };
    assert!(
        (tw.vel - vec3(0.0, 0.0, 34.0)).length() < 1e-4,
        "JS L884: `dir * def.speed` (yaw 0 ⇒ +z, got {:?})",
        tw.vel
    );
    assert!(
        (tw.life - 44.0 / 34.0).abs() < 1e-5,
        "JS L885: `life = range / speed`"
    );
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Zooka(st) = &s.state else {
            panic!("zooka state");
        };
        // The 0.25 s start cooldown expires on frame 15 or 16 (f32 rounding of
        // 15 × FIXED_DT against the exact 0.25), so the post-re-arm decay is
        // 4–5 frames either way.
        assert!(
            (st.cd - (1.0 - 4.0 * FIXED_DT)).abs() < 1e-5
                || (st.cd - (1.0 - 5.0 * FIXED_DT)).abs() < 1e-5,
            "JS L1335: re-armed to `def.interval` (cd {})",
            st.cd
        );
        assert!(
            (s.firing - 0.45).abs() < 1e-6,
            "JS L1335: the fire pose (hardcoded 0.45, D-T35-4)"
        );
    }
    // Record `[2, 'tw', gid, ...v3(m), ...v3(dir)]` (JS L1337).
    assert_eq!(a.sp_records.len(), 2, "start + the twister put");
    let rec = &a.sp_records[1];
    assert_eq!(
        &rec[..3],
        &[
            KitVal::Num(2.0),
            KitVal::Str("tw".to_string()),
            KitVal::Num(w.gid as f32)
        ]
    );
    assert_eq!(rec.len(), 3 + 6, "JS L1337: muzzle + dir triples");
}

#[test]
fn tr35_b3_wail_place() {
    // JS `IMPL.wail` (specials.js L1351-1407), tuning config.js L434-435.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().wail;
    assert_eq!(
        (
            d.charge,
            d.blast,
            d.radius,
            d.range,
            d.dps,
            d.hold_time,
            d.hold_speed
        ),
        (1.3, 3.2, 1.5, 72.0, 260.0, 6.0, 3.2)
    );
    let mut a = actor_with_special(t, "wail", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "wail");
    {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(s.speed, Some(3.2), "JS L1353: `speed: def.holdSpeed`");
        assert_eq!(s.dur, 0.0, "JS L1353: the placement ends it");
        assert!(s.aim_face && s.no_squid, "JS L1353");
    }
    // `dirOf` clamps the pitch to ±0.3 rad (JS L1373, hardcoded D-T35-4).
    a.aim_pitch = 1.9;
    let dir = wail::dir_of(&a);
    assert!(
        (dir.y - 0.3_f32.sin()).abs() < 1e-5,
        "JS L1373: pitch clamped to 0.3 (got {})",
        dir.y
    );
    a.aim_pitch = 0.0;
    // The bot auto-place timer (JS L1364): `0.45 + rand*0.5`.
    let mut rng = Rng::new(9);
    wail::begin(&mut a, &mut rng);
    let auto_t = {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Wail(st) = &s.state else {
            panic!("wail state");
        };
        st.auto_t.expect("JS L1364: the bot draw")
    };
    assert!(
        (0.45..0.95).contains(&auto_t),
        "JS L1364: autoT ∈ [0.45, 0.95) (got {auto_t})"
    );
    // The timer expiry places the speaker and ends with 'place' (JS L1369-1370).
    let mut sys = SpecialSystem::new();
    {
        let s = a.special_active.as_mut().unwrap();
        s.t = auto_t;
    }
    assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    assert_eq!(sys.world.len(), 1, "JS L1397: the speaker world object");
    let w = &sys.world[0];
    assert_eq!(w.kind, "sk");
    assert!(!w.ghost && w.gid != 0);
    let world::ObjectBody::Speaker(sp) = &w.body else {
        panic!("speaker body");
    };
    assert_eq!(
        (sp.range, sp.radius),
        (72.0, 1.5),
        "JS L966-967: def values"
    );
    assert_eq!(sp.phase, 0, "JS L964: the charge phase starts");
    assert!(a.special_active.is_none(), "JS L1398: `end(a, 'place')`");
    assert_eq!(
        a.sp_records.last().unwrap(),
        &[KitVal::Num(1.0), KitVal::Str("place".to_string())]
    );
    assert_eq!(a.sp_records[1][1], KitVal::Str("sk".to_string()));
}

#[test]
fn tr35_b3_blower_bubble_ladder() {
    // JS `IMPL.blower` (specials.js L1472-1509), tuning config.js L440-441.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().blower;
    assert_eq!(
        (
            d.duration,
            d.max,
            d.inflate,
            d.r_min,
            d.r_max,
            d.drift,
            d.life,
            d.pop_damage,
            d.blast_mul,
            d.damage_max,
            d.damage_min
        ),
        (9.0, 3, 1.0, 0.9, 2.1, 1.4, 9.0, 55.0, 1.9, 180.0, 45.0)
    );
    let mut a = actor_with_special(t, "blower", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "blower");
    assert!(
        a.special_active.as_ref().unwrap().aim_face && a.special_active.as_ref().unwrap().no_squid,
        "JS L1473"
    );
    let mut sys = SpecialSystem::new();
    let mut rng = Rng::new(2);
    a.fire_gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        sub: false,
        sub_released: false,
    };
    // Press: a fresh held bubble in the registry, *no record yet* (JS L1477).
    assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    assert_eq!(sys.world.len(), 1);
    let gid1 = sys.world[0].gid;
    assert_eq!(sys.world[0].kind, "bu");
    assert!(!sys.world[0].ghost && gid1 != 0);
    assert_eq!(
        a.sp_records.len(),
        1,
        "JS L303: gid-less held bubble unrecorded"
    );
    {
        let world::ObjectBody::Bubble(b) = &sys.world[0].body else {
            panic!("bubble body");
        };
        assert!(b.held, "JS L1030: held on the muzzle");
        assert_eq!(b.life, 9.0, "JS L1026: `life = def.life`");
        assert!(b.ph < 6.0, "JS L1027: `ph = random()*6`");
        assert!(
            b.r > 0.9 && b.r < 0.95,
            "JS L1484: ease-out inflation from rMin (r {})",
            b.r
        );
    }
    // Trigger-up releases it and records `[2,'bu',gid,...v3(pos),r2(r),...v3(f)]`.
    a.fire_gate.fire = false;
    a.fire_gate.pressed = false;
    assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    {
        let world::ObjectBody::Bubble(b) = &sys.world[0].body else {
            panic!("bubble body");
        };
        assert!(!b.held, "JS L1033: released");
        assert!(
            (b.vel.z - 1.4).abs() < 1e-5 && b.vel.y == 0.25,
            "JS L1034-1036: drift 1.4 forward + 0.25 lift (hardcoded, D-T35-4)"
        );
    }
    assert_eq!(a.sp_records.len(), 2);
    let rec = &a.sp_records[1];
    assert_eq!(
        &rec[..3],
        &[
            KitVal::Num(2.0),
            KitVal::Str("bu".to_string()),
            KitVal::Num(gid1 as f32)
        ]
    );
    assert_eq!(rec.len(), 3 + 3 + 1 + 3, "JS L1502: pos + r + fwd");
    // Two more press/release cycles fill the ladder: max 3, done tail.
    for _ in 0..2 {
        a.fire_gate.fire = true;
        a.fire_gate.pressed = true;
        assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
        a.fire_gate.fire = false;
        a.fire_gate.pressed = false;
        assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    }
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Blower(st) = &s.state else {
            panic!("blower state");
        };
        assert_eq!(st.count, 3.0, "JS L1493: three bubbles released");
        assert!(st.done, "JS L1493: `count >= def.max`");
        assert!(
            (s.dur - (s.t + 0.6)).abs() < 1e-4,
            "JS L1494: the 0.6 s tail (hardcoded, D-T35-4)"
        );
    }
    assert_eq!(sys.world.len(), 3);
    // A fourth press creates nothing (JS L1477: `count < def.max`).
    a.fire_gate.fire = true;
    a.fire_gate.pressed = true;
    assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    assert_eq!(sys.world.len(), 3, "JS L1477: `max` caps the bubbles");
    // A held bubble at `end` routes through `sp_let` → `drain_let` (JS L1505).
    let mut b2 = actor_with_special(t, "blower", 1000.0);
    b2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut b2, "blower");
    b2.fire_gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        sub: false,
        sub_released: false,
    };
    assert!(weapon_hook_at(&mut b2, FIXED_DT, &mut sys, &f, &mut rng));
    end(&mut b2, "time");
    assert!(
        b2.sp_let.is_some(),
        "JS L1505: the held bubble latches for the match drain"
    );
    let before = b2.sp_records.len();
    blower::drain_let(&mut b2, &mut sys);
    assert!(b2.sp_let.is_none(), "the latch is consumed");
    assert_eq!(
        b2.sp_records.len(),
        before + 1,
        "JS L1502: the release records"
    );
}

#[test]
fn tr35_b3_booyah_cheer_and_throw() {
    // JS `IMPL.booyah` (specials.js L1717-1769) + `SpecialSystem.cheer`
    // (L510-531), tuning config.js L456-457.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().booyah;
    assert_eq!(
        (
            d.charge,
            d.cheer,
            d.cheer_special,
            d.auto_throw,
            d.move_speed,
            d.throw_speed,
            d.fuse,
            d.radius,
            d.kill_radius,
            d.damage_max,
            d.damage_min
        ),
        (4.5, 0.12, 12.0, 2.5, 1.8, 19.6, 1.5, 8.4, 4.6, 220.0, 60.0)
    );
    // The charge ladder: `+dt/charge` per frame, capped at 1 (JS L1729).
    let mut a = actor_with_special(t, "booyah", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "booyah");
    assert_eq!(
        a.special_active.as_ref().unwrap().speed,
        Some(1.8),
        "JS L1718: `speed: def.moveSpeed`"
    );
    let mut sys = SpecialSystem::new();
    let mut rng = Rng::new(6);
    for _ in 0..60 {
        assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    }
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Booyah(st) = &s.state else {
            panic!("booyah state");
        };
        assert!(
            (st.charge - 60.0 * FIXED_DT / 4.5).abs() < 1e-4,
            "JS L1729: one second = 1/4.5 of the orb (charge {})",
            st.charge
        );
    }
    // Cheers (JS L510-531): a teammate's orb +0.12, the cheater's own gauge
    // +12, rate-limited to one per 0.4 s.
    let mut cheerer = actor_with_special(t, "booyah", 1000.0);
    cheerer.special = 0.0;
    let mut holder = actor_with_special(t, "booyah", 1000.0);
    start(&mut holder, "booyah");
    {
        let s = holder.special_active.as_mut().unwrap();
        let KindState::Booyah(st) = &mut s.state else {
            panic!("booyah state");
        };
        st.charge = 0.5;
    }
    let mut actors = vec![cheerer, holder];
    cheer(0, &mut actors, 1.0);
    {
        let s = actors[1].special_active.as_ref().unwrap();
        let KindState::Booyah(st) = &s.state else {
            panic!("booyah state");
        };
        assert!((st.charge - 0.62).abs() < 1e-5, "JS L520: `+def.cheer`");
        assert_eq!(st.cheered, 0.35, "JS L521: the cheer flash");
    }
    assert!(
        (actors[0].special - 12.0).abs() < 1e-5,
        "JS L527: `+def.cheerSpecial` on the cheater's gauge"
    );
    cheer(0, &mut actors, 1.2);
    assert!(
        (actors[1].special_active.as_ref().unwrap().booyah_charge() - 0.62).abs() < 1e-5,
        "JS L511: the 0.4 s cheer rate limit"
    );
    cheer(0, &mut actors, 1.5);
    assert!(
        (actors[0].special - 24.0).abs() < 1e-5,
        "JS L527: a second cheer tops up again"
    );
    // A thrown orb is not cheerable (JS L519 `s.thrown`).
    {
        let s = actors[1].special_active.as_mut().unwrap();
        let KindState::Booyah(st) = &mut s.state else {
            panic!("booyah state");
        };
        st.thrown = true;
    }
    cheer(0, &mut actors, 2.0);
    assert!(
        (actors[0].special - 24.0).abs() < 1e-5,
        "JS L525: nothing helped ⇒ no self top-up"
    );
    // `tick` decays the cheer flash (JS L1746).
    {
        let s = actors[1].special_active.as_mut().unwrap();
        let KindState::Booyah(st) = &mut s.state else {
            panic!("booyah state");
        };
        st.thrown = false;
        st.cheered = 0.35;
    }
    booyah::tick(actors[1].special_active.as_mut().unwrap(), FIXED_DT);
    {
        let s = actors[1].special_active.as_ref().unwrap();
        let KindState::Booyah(st) = &s.state else {
            panic!("booyah state");
        };
        assert!(
            (st.cheered - (0.35 - FIXED_DT)).abs() < 1e-5,
            "JS L1746: `cheered -= dt`"
        );
    }
    drop(actors);
    // The full-charge click throws (JS L1731-1733): orb + record + end.
    {
        let s = a.special_active.as_mut().unwrap();
        let KindState::Booyah(st) = &mut s.state else {
            panic!("booyah state");
        };
        st.charge = 1.0;
        st.full_t = 0.1; // JS L1732: the 0.05 s minimum hold (hardcoded)
    }
    a.fire_gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        sub: false,
        sub_released: false,
    };
    assert!(weapon_hook_at(&mut a, FIXED_DT, &mut sys, &f, &mut rng));
    assert_eq!(sys.world.len(), 1, "JS L1753-1760: the orb is away");
    let w = &sys.world[0];
    assert_eq!(w.kind, "or");
    assert!(!w.ghost && w.gid != 0);
    let world::ObjectBody::Orb(o) = &w.body else {
        panic!("orb body");
    };
    assert_eq!(o.fuse, 1.5, "JS L1199: `fuse = def.fuse`");
    assert_eq!(
        w.pos,
        a.pos + vec3(0.0, 1.35, 0.0),
        "JS L1755: the throw origin (hardcoded 1.35, D-T35-4)"
    );
    assert!(
        (o.vel.z - (0.28_f32.cos() * 19.6)).abs() < 1e-3
            && (o.vel.y - (0.28_f32.sin() * 19.6 + 1.5)).abs() < 1e-3,
        "JS L1756: the shared lob arc at `throwSpeed` (got {:?})",
        o.vel
    );
    assert!(a.special_active.is_none(), "JS L1761: `end(a, 'throw')`");
    assert_eq!(
        a.sp_records.last().unwrap(),
        &[KitVal::Num(1.0), KitVal::Str("throw".to_string())]
    );
    assert_eq!(a.sp_records[1][1], KitVal::Str("or".to_string()));
    assert_eq!(a.sp_records[1].len(), 3 + 6, "JS L1759: from + vel triples");
    // The autoThrow cap throws without a click (JS L1731).
    let mut b2 = actor_with_special(t, "booyah", 1000.0);
    b2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut b2, "booyah");
    {
        let s = b2.special_active.as_mut().unwrap();
        let KindState::Booyah(st) = &mut s.state else {
            panic!("booyah state");
        };
        st.charge = 1.0;
        st.full_t = 2.6; // > def.autoThrow
    }
    assert!(weapon_hook_at(&mut b2, FIXED_DT, &mut sys, &f, &mut rng));
    assert_eq!(sys.world.len(), 2, "JS L1731: autoThrow fires the orb");
    assert!(b2.special_active.is_none());
}

#[test]
fn tr35_b3_barrage_gaps_and_subs() {
    // JS `IMPL.barrage` (specials.js L1247-1253) + the runner's volley gate
    // (weapons.js L132-155), tuning config.js L416-420.
    let f = fix();
    let t = &f.tuning.player;
    let want = [
        ("barrage", "bomb", 0.3),
        ("barrage_sticky", "sticky", 0.4),
        ("barrage_burst", "burst", 0.2),
        ("barrage_seeker", "seeker", 0.5),
        ("barrage_mist", "mist", 0.45),
    ];
    for (id, bomb, gap) in want {
        let Some(SpecialEntry::Barrage(d)) = def_by_id(id) else {
            panic!("{id} is a barrage");
        };
        assert_eq!(d.gap, gap, "config.js L416-420: `{id}` gap");
        assert_eq!(d.duration, 6.5, "config.js L416-420: `{id}` duration");
        assert_eq!(d.kind, "barrage", "JS L255: the shared dispatch kind");
        assert_eq!(d.bomb, bomb, "config.js L416-420: `{id}` bomb sub");
    }
    // `start` arms the volley state (JS L1249).
    let mut a = actor_with_special(t, "barrage_sticky", 1000.0);
    a.sub = crate::sub::SubKind::Scan;
    start(&mut a, "barrage_sticky");
    {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(s.dur, 6.5, "JS L256: `dur = def.duration`");
        let KindState::Barrage(st) = &s.state else {
            panic!("barrage state");
        };
        assert_eq!(st.bomb, "sticky", "JS L1249: `s.bomb = SUBS[def.bomb]`");
        assert_eq!(st.next_throw, 0.0, "JS L1249: ready from the start");
    }
    assert_eq!(
        barrage::effective_sub(&a),
        crate::sub::SubKind::Sticky,
        "JS weapons.js L136-137: the barrage bomb overrides the loadout"
    );
    end(&mut a, "time");
    assert!(a.sp_reset_aim, "JS L1252: the runner's aim state clears");
    assert_eq!(
        barrage::effective_sub(&a),
        crate::sub::SubKind::Scan,
        "JS L133: back to the loadout sub"
    );
    // A ghost barrage carries the state but never runs the runner (JS L132).
    let mut sys = SpecialSystem::new();
    let mut g = actor_with_special(t, "barrage_mist", 1000.0);
    g.sub = crate::sub::SubKind::Scan;
    net_ghost(
        &mut g,
        &[
            KitVal::Num(0.0),
            KitVal::Num(order_index("barrage_mist") as f32),
        ],
        &mut sys,
        &f,
    );
    {
        let s = g.special_active.as_ref().unwrap();
        assert!(s.ghost);
        assert_eq!(s.dur, 6.5, "JS L280: the ghost takes the def duration");
        let KindState::Barrage(st) = &s.state else {
            panic!("barrage ghost state");
        };
        assert_eq!(st.bomb, "mist", "JS L282: the IMPL fallback arms the bomb");
    }
    assert_eq!(
        barrage::effective_sub(&g),
        crate::sub::SubKind::Scan,
        "JS L132: ghost actives never override the loadout"
    );
}

#[test]
fn tr35_b3_ghost_kinds_and_bubble_roundtrip() {
    // JS `_ghostObj` (specials.js L306-322) for `tw` / `sk` / `bu` / `or`,
    // the op-3 bubble pop (`netGhost` L273 + `Bubble.netEvent` L1080) and
    // `netHurtObj` (L324-327).
    let f = fix();
    let t = &f.tuning.player;
    let a = actor_with_special(t, "blower", 1000.0);
    let mut actors = vec![a];
    let mut sys = SpecialSystem::new();
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng = Rng::new(0x9E7);
    let mut subs = crate::sub::SubSystem::new(1);
    let mut events = Vec::new();
    let mut ctx = world::WorldCtx {
        actors: &mut actors,
        collision: &f.world,
        paint: &mut paint,
        rng: &mut rng,
        subs: &mut subs,
        sb: &f.tuning.subs,
        t: &f.tuning.player,
        events: &mut events,
        remote: None,
        time: 0.0,
    };
    let k = |v: f32| KitVal::Num(v);
    let s = |v: &str| KitVal::Str(v.to_string());
    // op 2 `tw` (JS L312-313): pos = V(d,3), dir = V(d,6).
    super::net_ghost(
        0,
        &[
            k(2.0),
            s("tw"),
            k(11.0),
            k(1.0),
            k(2.0),
            k(3.0),
            k(0.0),
            k(0.0),
            k(1.0),
        ],
        &mut sys,
        &mut ctx,
    );
    {
        let w = &sys.world[0];
        assert!(w.ghost && w.gid == 11 && w.kind == "tw");
        assert_eq!(w.pos, vec3(1.0, 2.0, 3.0));
        let world::ObjectBody::Twister(tw) = &w.body else {
            panic!("twister body");
        };
        assert_eq!(tw.vel, vec3(0.0, 0.0, 34.0), "JS L884: dir × zooka.speed");
        assert!(
            (tw.life - 44.0 / 34.0).abs() < 1e-5,
            "JS L885: range / speed"
        );
    }
    // The gid dedup (JS L309).
    super::net_ghost(
        0,
        &[
            k(2.0),
            s("tw"),
            k(11.0),
            k(9.0),
            k(9.0),
            k(9.0),
            k(1.0),
            k(0.0),
            k(0.0),
        ],
        &mut sys,
        &mut ctx,
    );
    assert_eq!(sys.world.len(), 1, "JS L309: a known gid is skipped");
    // op 2 `sk` (JS L313).
    super::net_ghost(
        0,
        &[
            k(2.0),
            s("sk"),
            k(12.0),
            k(4.0),
            k(0.0),
            k(5.0),
            k(0.0),
            k(0.0),
            k(1.0),
        ],
        &mut sys,
        &mut ctx,
    );
    {
        let w = &sys.world[1];
        assert_eq!(w.kind, "sk");
        assert_eq!(w.pos, vec3(4.0, 0.0, 5.0));
        let world::ObjectBody::Speaker(sp) = &w.body else {
            panic!("speaker body");
        };
        assert_eq!(
            (sp.range, sp.radius),
            (72.0, 1.5),
            "JS L966-967: def values"
        );
        assert_eq!(sp.phase, 0);
    }
    // op 2 `bu` (JS L314): pos = V(d,3), r = d[6], release(V(d,7)).
    super::net_ghost(
        0,
        &[
            k(2.0),
            s("bu"),
            k(13.0),
            k(0.0),
            k(1.0),
            k(0.0),
            k(2.0),
            k(0.0),
            k(0.0),
            k(1.0),
        ],
        &mut sys,
        &mut ctx,
    );
    {
        let w = &sys.world[2];
        assert_eq!(w.kind, "bu");
        assert_eq!(w.pos, vec3(0.0, 1.0, 0.0));
        let world::ObjectBody::Bubble(b) = &w.body else {
            panic!("bubble body");
        };
        assert!((b.r - 2.0).abs() < 1e-6, "JS L314: r = d[6]");
        assert!(!b.held, "JS L314: the ghost arrives released");
        assert!(
            (b.vel.z - 1.4).abs() < 1e-5 && b.vel.y == 0.25,
            "JS L1034-1036: drift + lift from the record fwd"
        );
    }
    // op 2 `or` (JS L317): from = V(d,3), vel = V(d,6).
    super::net_ghost(
        0,
        &[
            k(2.0),
            s("or"),
            k(14.0),
            k(1.0),
            k(2.0),
            k(1.0),
            k(0.0),
            k(5.0),
            k(5.0),
        ],
        &mut sys,
        &mut ctx,
    );
    {
        let w = &sys.world[3];
        assert_eq!(w.kind, "or");
        assert_eq!(w.pos, vec3(1.0, 2.0, 1.0));
        let world::ObjectBody::Orb(o) = &w.body else {
            panic!("orb body");
        };
        assert_eq!(o.fuse, 1.5, "JS L1199: def.fuse");
        assert_eq!(o.vel, vec3(0.0, 5.0, 5.0));
    }
    // op 3 (JS L273 + `Bubble.netEvent` L1080): the `x` event pops the ghost.
    super::net_ghost(0, &[k(3.0), k(13.0), s("x")], &mut sys, &mut ctx);
    assert!(sys.world[2].dead, "JS L1082: the ghost bubble pops");
    // Ghost ends never record (JS L1084 owner guard) — nothing for the proxy.
    // The dead ghost is pruned by the next object step (JS L566-569).
    world::step_special_objects(&mut sys.world, FIXED_DT, &mut ctx);
    assert_eq!(sys.world.len(), 3, "JS L569: the dead ghost is spliced out");
    // `netHurtObj` (JS L324-327): only the *owner-side* bubble is hurtable.
    let mut obj = world::make_bubble(0, 0, false, ctx.rng);
    obj.gid = 21;
    if let world::ObjectBody::Bubble(b) = &mut obj.body {
        world::bubble_release(b, vec3(0.0, 0.0, 1.0));
    }
    sys.world.push(obj);
    // Negative dmg = own-team ink → the charge path (JS L326/L479-481).
    net_hurt_obj(&mut sys, 21, -20.0, &mut ctx);
    let wi = sys.world.iter().position(|w| w.gid == 21).unwrap();
    {
        let world::ObjectBody::Bubble(b) = &sys.world[wi].body else {
            panic!("bubble body");
        };
        assert!(
            (b.charge - 20.0).abs() < 1e-5,
            "JS L480: own-team ink charges the released bubble"
        );
        assert!(!sys.world[wi].dead, "JS L481: below popDamage it survives");
    }
    // Positive dmg = enemy ink → the soak path (JS L482, floor rMin·0.75).
    net_hurt_obj(&mut sys, 21, 50.0, &mut ctx);
    {
        let world::ObjectBody::Bubble(b) = &sys.world[wi].body else {
            panic!("bubble body");
        };
        assert!(
            (b.r - 0.7).abs() < 1e-5,
            "JS L482: r −= dmg·0.004 (rMin 0.9 − 0.2, got {})",
            b.r
        );
        assert!((b.charge - 20.0).abs() < 1e-5, "enemy ink never charges");
    }
    // `charge >= popDamage` (55) pops it: dead + the owner records [3,gid,'x'].
    net_hurt_obj(&mut sys, 21, -40.0, &mut ctx);
    assert!(sys.world[wi].dead, "JS L481: popDamage 55 explodes it");
    assert_eq!(
        actors[0].sp_records.last().unwrap(),
        &[
            KitVal::Num(3.0),
            KitVal::Num(21.0),
            KitVal::Str("x".to_string())
        ],
        "JS L1084: `rec(this.owner, [3, gid, 'x'])`"
    );
}

// ---------------------------------------------------------------------------
// TR-35.1 — Task 35 batch 4: the four body / trigger kinds' core-mechanic
// timing ladders (jetpack hover + blast, zipcaster tether + impact, crab
// gatling + hull, stamp swing + flip + slam), fixed seed, value-by-value
// against the JS anchors (spec §6).
// ---------------------------------------------------------------------------

#[test]
fn tr35_b4_jetpack_launch_hover_boost_blast() {
    // JS `IMPL.jetpack` (specials.js L1512-1595), tuning config.js L440-442.
    // The open `wall_world` deck (not tidewater) keeps the vertical launch
    // clear of the stage's overhead geometry.
    let t_all = crate::embedded_tuning();
    let t = &t_all.player;
    let d = &specials().jetpack;
    let world = wall_world();
    let layout = crate::embedded_tidewater();
    let sim = crate::actor::SimWorld::new(&world, &layout);
    // Rust tuple traits stop at 12 elements — the table is asserted in two
    // halves.
    assert_eq!(
        (
            d.duration,
            d.height,
            d.speed,
            d.accel,
            d.boost,
            d.boost_gap,
            d.interval
        ),
        (7.0, 3.8, 5.5, 11.0, 9.0, 0.9, 0.55)
    );
    assert_eq!(
        (
            d.proj_speed,
            d.range,
            d.direct_damage,
            d.splash_max,
            d.splash_min,
            d.splash_radius,
            d.paint_radius
        ),
        (30.0, 34.0, 125.0, 70.0, 30.0, 2.4, 1.5)
    );
    let mut a = actor_with_special(t, "jetpack", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    let y0 = a.pos.y;
    start(&mut a, "jetpack");
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(
            s.body && s.jump_back,
            "JS L1513-1514: body-owned + jumpBack"
        );
        assert_eq!(a.vel.y, 7.0, "JS L1516: launch impulse (D-T35-4)");
        assert!(!a.grounded, "JS L1513");
        assert_eq!(a.sp_marker.as_deref(), Some("jetpack"), "JS L1515");
        let KindState::Jetpack(st) = &s.state else {
            panic!("jetpack state");
        };
        assert_eq!(st.origin.y, y0);
        assert_eq!(st.cd, 0.3, "JS L1514: start cooldown (D-T35-4)");
    }
    // Frame 1 (JS actor.js L274-277: the body branch replaces the controller):
    // steer toward +z at `accel · dt`, the hover pull, exhaust paint raised.
    let inp = ActorInput {
        move_dir: vec3(0.0, 0.0, 1.0),
        fire: true,
        ..Default::default()
    };
    crate::special::body(&mut a, FIXED_DT, &sim, t, &inp);
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(
            (a.vel.z - 11.0 * FIXED_DT).abs() < 1e-6,
            "JS L1534: accel step (got {})",
            a.vel.z
        );
        // JS L1552-1553: the hover target clamps to `vy = 6`, blended at
        // `dt · 6` (ascending toward `floor + height`).
        let want = (6.0 - 7.0) * (FIXED_DT * 6.0);
        assert!(
            (a.vel.y - (7.0 + want)).abs() < 1e-5,
            "JS L1552: hover blend (got {})",
            a.vel.y
        );
        assert!(
            (a.pos.y - (y0 + a.vel.y * FIXED_DT)).abs() < 1e-5,
            "JS L1555: integrate"
        );
        assert_eq!(s.face_yaw, Some(a.aim_yaw), "JS L1559: face the aim");
        let KindState::Jetpack(st) = &s.state else {
            panic!("jetpack state");
        };
        assert!((st.cd - (0.3 - FIXED_DT)).abs() < 1e-6);
        assert!(a.sp_jet_paint, "JS L1576: the trail fires on frame one");
        assert_eq!(st.paint_t, 0.28, "JS L1576: trail period (D-T35-4)");
        assert!(!a.sp_jet_fire, "JS L1562: still on the 0.3 s cooldown");
    }
    a.sp_jet_paint = false;
    // The 0.3 s start cooldown expires on frame 18 (±1 for the f32 decay).
    let mut fire_frame = 0usize;
    for k in 2..=20 {
        crate::special::body(&mut a, FIXED_DT, &sim, t, &inp);
        if std::mem::take(&mut a.sp_jet_fire) {
            assert_eq!(fire_frame, 0, "one blast per `interval`");
            fire_frame = k;
            let s = a.special_active.as_ref().unwrap();
            let KindState::Jetpack(st) = &s.state else {
                panic!("jetpack state");
            };
            assert_eq!(st.cd, d.interval, "JS L1563: re-armed to `def.interval`");
            assert_eq!(s.firing, 0.4, "JS L1563: the fire pose (D-T35-4)");
        }
        a.sp_jet_paint = false;
    }
    assert!(
        (17..=19).contains(&fire_frame),
        "JS L1562: the blast rides the cd edge, got frame {fire_frame}"
    );
    // A jump edge fires the boost (JS L1541-1550): `vel.y = def.boost`, then
    // the reduced-gravity arc until `boostT` runs out.
    let inp_jump = ActorInput {
        move_dir: vec3(0.0, 0.0, 1.0),
        fire: true,
        jump: true,
        ..Default::default()
    };
    crate::special::body(&mut a, FIXED_DT, &sim, t, &inp_jump);
    {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Jetpack(st) = &s.state else {
            panic!("jetpack state");
        };
        assert_eq!(st.boost_cd, 0.9, "JS L1545: `def.boostGap`");
        assert_eq!(st.boost_t, 0.35, "JS L1545: boost length (D-T35-4)");
    }
    // JS L1546/L1554: `vel.y = def.boost`, then the 0.35-gravity arc.
    assert!(
        (a.vel.y - (9.0 - t.gravity * 0.35 * FIXED_DT)).abs() < 1e-5,
        "JS L1554: reduced gravity, got {}",
        a.vel.y
    );
    // The match drain (JS L1566-1567 `fireCustom … type:'blast'` + the
    // `burst` override).
    let mut ps = crate::weapon::ProjectileSim::new(5);
    ps.fire_jetpack_blast(&a, d);
    assert_eq!(ps.list.len(), 1);
    {
        let p = &ps.list[0];
        assert!(
            (p.vel - vec3(0.0, 0.0, 30.0)).length() < 1e-5,
            "JS L1566: `speed: def.projSpeed` along the aim, got {:?}",
            p.vel
        );
        assert_eq!(p.damage, 125.0, "JS L1566: `damage: def.directDamage`");
        assert!(p.blast, "JS L1566: `type: 'blast'`");
        assert!(
            (p.life - 34.0 / 30.0).abs() < 1e-6,
            "JS L1203: `life = range / speed`"
        );
        let b = p.burst.as_ref().expect("JS L1567: the burst override");
        assert_eq!(
            (
                b.radius,
                b.splash_radius,
                b.splash_damage_max,
                b.splash_damage_min
            ),
            (2.4, 2.4, 70.0, 30.0),
            "JS L1567: radius / splashRadius / dmgMax / dmgMin"
        );
        assert_eq!(b.paint, Some(1.5), "JS L1567: `paint: def.paintRadius`");
    }
    // JS L1577-1578: the exhaust splat (one seed draw, turf only).
    let mut paint = crate::paint::PaintGrid::new(&world);
    let mut rng = Rng::new(0x5EED);
    let sp0 = a.special;
    jetpack::paint_trail(&mut a, &world, &mut paint, &mut rng);
    assert!(a.turf > 0.0, "JS L1578: the trail paints");
    assert_eq!(
        a.special, sp0,
        "JS L53: special ink never charges the gauge"
    );
    // JS L374: the end routes the home super jump (marker action `true`).
    a.pos.z += 10.0; // beyond `MARKER_HOME_DIST` (JS L374: 2.5)
    crate::special::end(&mut a, "time");
    assert!(a.special_active.is_none());
    assert_eq!(
        a.sp_marker_action,
        Some(true),
        "JS L374: the home jump started ⇒ the marker turns back"
    );
    assert!(
        a.super_jump.is_some(),
        "JS L374: `superJump(origin, instant, home)`"
    );
    assert_eq!(
        a.sp_records.last().map(|r| (r[0].clone(), r[1].clone())),
        Some((KitVal::Num(1.0), KitVal::Str("time".to_string()))),
        "JS L367: the end records `[1,'time']`"
    );
    // The match-side drain routes the action onto the home marker (JS L714).
    let mut sys = SpecialSystem::new();
    sys.world.push(world::make_marker(
        a.slot,
        a.team,
        "jetpack",
        vec3(0.0, 0.0, 0.0),
        false,
    ));
    crate::special::drain_marker_action(&mut a, &mut sys);
    let wi = world::find_marker(&sys.world, a.slot).unwrap();
    if let world::ObjectBody::Marker(mk) = &sys.world[wi].body {
        assert!(mk.back, "JS L714: `returning()` marks the home trip");
    } else {
        panic!("marker body");
    }
}

/// A flat deck with a vertical wall at `z = 5` (the zipcaster raycast target).
fn wall_world() -> CollisionWorld {
    let common = crate::geometry::BrushCommon {
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
        mural: Vec::<crate::geometry::Mural>::new(),
        oct: None,
        only_in: None,
        not_in: None,
    };
    let deck = crate::geometry::Brush::Box {
        min: [-10.0, -1.0, -10.0],
        max: [10.0, 0.0, 10.0],
        common: common.clone(),
    };
    let wall = crate::geometry::Brush::Box {
        min: [-5.0, 0.0, 5.0],
        max: [5.0, 5.0, 5.5],
        common,
    };
    CollisionWorld::from_brushes(
        &[deck, wall],
        crate::geometry::Bounds {
            min_x: -30.0,
            max_x: 30.0,
            min_z: -30.0,
            max_z: 30.0,
        },
    )
}

#[test]
fn tr35_b4_zipcaster_tether_zip_hang_impact() {
    // JS `IMPL.zipcaster` (specials.js L1772-1905), tuning config.js L462-463.
    let t_all = crate::embedded_tuning();
    let t = &t_all.player;
    let d = &specials().zipcaster;
    assert_eq!(
        (
            d.duration,
            d.range,
            d.speed,
            d.hang,
            d.cooldown,
            d.impact_direct,
            d.impact_splash,
            d.impact_direct_r,
            d.impact_radius
        ),
        (8.5, 21.0, 22.0, 1.4, 0.35, 100.0, 30.0, 1.0, 2.4)
    );
    let world = wall_world();
    let layout = crate::embedded_tidewater();
    let sim = crate::actor::SimWorld::new(&world, &layout);
    let mut paint = crate::paint::PaintGrid::new(&world);
    let mut rng = Rng::new(0x2121);
    let mut a = actor_with_special(t, "zipcaster", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    let y0 = a.pos.y;
    let sp0 = a.special;
    start(&mut a, "zipcaster");
    assert!(a.special_active.as_ref().unwrap().jump_back, "JS L1774");
    assert_eq!(a.sp_marker.as_deref(), Some("zipcaster"), "JS L1775");
    // The sub press fires the tether (JS L1786-1792): the ray leaves
    // `pos.y + 1.2` (JS L1794, D-T35-4) along the aim, latches on the wall
    // face at `z = 5` with normal `(0, 0, −1)` ⇒ `wall = |n.y| < 0.55`
    // (JS L1800).
    let mut gate = crate::actor::FireGate {
        sub: true,
        pressed: true,
        ..Default::default()
    };
    let mut s = a.special_active.take().unwrap();
    zipcaster::weapon(
        &mut a, &mut s, FIXED_DT, &mut gate, &world, &mut paint, &mut rng, t,
    );
    a.special_active = Some(s);
    assert!(!gate.sub, "JS L1789: the sub trigger is swallowed");
    let to = {
        let s = a.special_active.as_ref().unwrap();
        assert!(s.body && s.hang == 0.0, "JS L1806: the zip owns the body");
        let KindState::Zipcaster(st) = &s.state else {
            panic!("zipcaster state");
        };
        assert_eq!(st.cd, d.cooldown, "JS L1796: `def.cooldown`");
        let z = st.zip.expect("the tether latched");
        assert_eq!(z.anchor, vec3(0.0, y0 + 1.2, 5.0), "JS L1794-1795: ray hit");
        assert_eq!(z.n, vec3(0.0, 0.0, -1.0));
        assert!(z.wall, "JS L1800: |n.y| = 0 < 0.55 ⇒ a wall anchor");
        // JS L1801-1802: `point + n·(radius + 0.08)`, then `y −= 1.0`.
        assert!((z.to.x - 0.0).abs() < 1e-5);
        assert!((z.to.y - (y0 + 0.2)).abs() < 1e-5, "to {:?}", z.to);
        assert!((z.to.z - (5.0 - (t.radius + 0.08))).abs() < 1e-5);
        z.to
    };
    // `[4,'zf', ...v3(anchor), r2(n)]` (JS L1805) + the latch paint (L1807).
    let k = |v: f32| KitVal::Num(v);
    let str_ = |v: &str| KitVal::Str(v.to_string());
    assert_eq!(
        a.sp_records[1],
        vec![
            k(4.0),
            str_("zf"),
            k(0.0),
            k(y0 + 1.2),
            k(5.0),
            k(0.0),
            k(0.0),
            k(-1.0)
        ],
        "JS L1805: the anchor + normal record"
    );
    assert!(a.turf > 0.0, "JS L1807: the latch paints");
    assert_eq!(
        a.special, sp0,
        "JS L53: special ink never charges the gauge"
    );
    // JS actor.js L276: a body-owned frame never reaches the weapon ladder —
    // the cd stays frozen and a fresh sub press is not consumed.
    {
        let mut sys = SpecialSystem::new();
        let mut gate2 = crate::actor::FireGate {
            sub: true,
            pressed: true,
            ..Default::default()
        };
        let mut ps = crate::weapon::ProjectileSim::new(5);
        let mut paint2 = crate::paint::PaintGrid::new(&world);
        let r = crate::special::weapon_hook(
            &mut a,
            FIXED_DT,
            &mut sys,
            &world,
            &mut rng,
            &mut paint2,
            &mut ps,
            t,
            &ActorInput::default(),
            &mut gate2,
        );
        assert!(r, "JS actor.js L276: the runner is skipped on a body frame");
        assert!(gate2.sub, "the ladder never ran ⇒ the sub press survives");
        let s = a.special_active.as_ref().unwrap();
        let KindState::Zipcaster(st) = &s.state else {
            panic!("zipcaster state");
        };
        assert_eq!(st.cd, d.cooldown, "the cd never decremented mid-flight");
        assert_eq!(
            a.sp_records.len(),
            2,
            "no new record from the skipped ladder"
        );
    }
    // The pull (JS L1834): `vel = dv · speed / dist` — 22 m/s toward `to`.
    let inp = ActorInput::default();
    let mut frames = 0usize;
    for k in 1..=30 {
        crate::special::body(&mut a, FIXED_DT, &sim, t, &inp);
        if k == 2 {
            assert!(
                (a.vel.length() - 22.0).abs() < 1e-4,
                "JS L1834: the zip runs at `def.speed` (got {})",
                a.vel.length()
            );
        }
        if a.sp_zip_impact.is_some() {
            frames = k;
            break;
        }
    }
    assert!(frames > 0, "the tether must arrive within 30 frames");
    // JS L1827: arrival snaps to `to` (4.5 m at 0.3667 m/frame ≈ frame 12).
    assert!((11..=13).contains(&frames), "zip duration, got {frames}");
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(!s.body, "JS L1813/L1830: the body releases on arrival");
        assert!((a.pos - to).length() < 1e-5, "JS L1829: snap to the anchor");
        assert_eq!(a.vel, Vec3::ZERO, "JS L1829: `vel.set(0, 0, 0)`");
        assert_eq!(s.hang, d.hang, "JS L1831: a wall arrival clings");
        assert_eq!(s.hang_n, Some(vec3(0.0, 0.0, -1.0)), "JS L1831");
        assert!(!a.grounded, "JS L1832");
        assert_eq!(s.face_yaw, None, "JS L1830");
    }
    assert!(
        matches!(a.sp_zip_impact, Some(crate::actor::ZipImpact::Land(true))),
        "JS L1833: the land impact rides the wall flag"
    );
    // The hang release (JS L1882-1888): jump ⇒ `vel = (n.x·4, 8, n.z·4)`.
    let inp_jump = ActorInput {
        jump: true,
        ..Default::default()
    };
    crate::special::tick(&mut a, FIXED_DT, &inp_jump);
    {
        assert_eq!(
            a.vel,
            vec3(0.0, 8.0, -4.0),
            "JS L1887: the wall jump-off (D-T35-4)"
        );
        assert_eq!(a.special_active.as_ref().unwrap().hang, 0.0);
        assert!(!a.grounded);
    }
    // The slam scan (JS L1818-1826): an enemy within `radius·2 + 0.2` XZ and
    // `|dy| < 1.4` is the direct victim.
    let mut near_enemy = Actor::new(3, 1, t);
    near_enemy.spawn_at(vec3(0.3, 0.05, 4.8), 0.0, &sim, t);
    let scan = [a.clone(), near_enemy.clone()];
    assert_eq!(
        zipcaster::slam_scan(&scan, &a, t),
        Some(1),
        "JS L1820: hypot(0.3, 0.26) < radius·2 + 0.2 = 0.96"
    );
    // The impact ladder (JS L1846-1863) against the roster: the 1 m direct
    // core hurts for `impactDirect`, the ring for `impactSplash`.
    let mut e1 = Actor::new(1, 1, t);
    e1.spawn_at(vec3(0.0, 0.05, 4.3), 0.0, &sim, t);
    let mut e2 = Actor::new(1, 2, t);
    e2.spawn_at(vec3(0.0, 0.05, 2.0), 0.0, &sim, t);
    // `spawn_at` arms the spawn invulnerability (JS `_spawn`); clear it so the
    // impact damage actually lands (`apply_hit` blocks while `invuln > 0`).
    e1.set_invuln(0.0);
    e2.set_invuln(0.0);
    let turf0 = a.turf;
    let mut actors = [a, e1, e2];
    let mut subs = crate::sub::SubSystem::new(3);
    let mut ps = crate::weapon::ProjectileSim::new(5);
    let imp = actors[0].sp_zip_impact.take().expect("the land impact");
    zipcaster::impact(
        &mut actors,
        0,
        imp,
        &world,
        &mut paint,
        &mut rng,
        &mut ps,
        &mut subs,
        None,
        t,
    );
    // `[4,'zi', ...v3(c), hang]` (JS L1848): c = pos + 0.7y, hang = 1.
    let rec = actors[0].sp_records.last().unwrap();
    assert_eq!(&rec[..2], &[k(4.0), str_("zi")]);
    assert!((rec[2].as_f32().unwrap() - 0.0).abs() < 1e-3);
    assert!(
        (rec[3].as_f32().unwrap() - 0.9).abs() < 1e-3,
        "c.y = 0.2 + 0.7"
    );
    assert!((rec[4].as_f32().unwrap() - to.z).abs() < 1e-2);
    assert_eq!(rec[5], k(1.0), "JS L1848: the hang flag");
    let evs = ps.drain_events();
    assert_eq!(evs.len(), 2, "one direct + one splash victim");
    for ev in &evs {
        let crate::weapon::SimEvent::Hit {
            victim_slot,
            damage,
            killed,
            ..
        } = *ev
        else {
            panic!("hit event, got {ev:?}");
        };
        if victim_slot == 1 {
            assert_eq!((damage, killed), (100.0, true), "JS L1852: `impactDirect`");
        } else {
            assert_eq!((damage, killed), (30.0, false), "JS L1853: `impactSplash`");
        }
    }
    assert_eq!(actors[2].hp, 70.0, "the splash victim survives at 70 hp");
    assert!(!actors[1].alive, "the direct victim is splatted");
    assert!(actors[0].turf > turf0, "JS L1859: the impact paint");
}

#[test]
fn tr35_b4_crab_gatling_cannon_hull_break() {
    // JS `IMPL.crab` (specials.js L1908-2043), tuning config.js L465-467.
    let f = fix();
    let sim = f.sim();
    let t = &f.tuning.player;
    let d = &specials().crab;
    // Rust tuple traits stop at 12 elements — the table is asserted in two
    // halves.
    assert_eq!(
        (
            d.duration,
            d.hp,
            d.speed,
            d.roll_speed,
            d.turn_rate,
            d.gun_interval,
            d.gun_damage,
            d.gun_speed,
            d.gun_range,
            d.gun_spread
        ),
        (9.0, 460, 3.0, 9.5, 1.5, 0.075, 18.0, 36.0, 22.0, 3.5)
    );
    assert_eq!(
        (
            d.cannon_gap,
            d.cannon_speed,
            d.cannon_radius,
            d.cannon_damage_max,
            d.cannon_damage_min,
            d.roll_armor
        ),
        (1.15, 17.0, 3.0, 150.0, 40.0, 0.35)
    );
    let k = |v: f32| KitVal::Num(v);
    let str_ = |v: &str| KitVal::Str(v.to_string());
    let crab_hp = |a: &Actor| a.special_active.as_ref().unwrap().crab_hp();
    let mut a = actor_with_special(t, "crab", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "crab");
    {
        let s = a.special_active.as_ref().unwrap();
        assert!(
            s.no_squid && s.ink_proof,
            "JS L1912: swim disabled, ink-proof hull"
        );
        assert_eq!(s.speed, Some(3.0), "JS L1911: `speed: def.speed`");
        assert_eq!(s.face_yaw, Some(a.yaw), "JS L1911: `faceYaw: a.yaw`");
        assert_eq!(a.form, crate::actor::Form::Kid, "JS L1913: the crab form");
        let KindState::Crab(st) = &s.state else {
            panic!("crab state");
        };
        assert_eq!(st.hp, 460.0, "JS L1911: `hp: def.hp`");
        assert_eq!(st.hull, a.yaw, "JS L1911: the hull starts at the facing");
        assert_eq!(st.ccd, 0.4, "JS L1912: start cannon cooldown (D-T35-4)");
    }
    // The gatling frame (JS L1955-1974): the hull slews toward the aim at
    // `def.turnRate`, then one spread-jittered round fires per cooldown slot.
    a.aim_yaw = 1.0;
    let mut sys = SpecialSystem::new();
    let mut ps = crate::weapon::ProjectileSim::new(5);
    let mut rng = Rng::new(0xCAB);
    // JS L1970: exactly two draws per round, in order.
    let (r1, r2) = {
        let mut r = rng; // `Rng` is `Copy` — this snapshots the stream
        (r.next_f32(), r.next_f32())
    };
    let inp = ActorInput::default();
    let mut gate = crate::actor::FireGate {
        fire: true,
        pressed: true,
        ..Default::default()
    };
    {
        let mut s = a.special_active.take().unwrap();
        crab::weapon(
            &mut a, &mut s, FIXED_DT, &inp, &mut gate, &mut sys, &mut rng, &mut ps,
        );
        a.special_active = Some(s);
    }
    let hull1 = {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(s.firing, 0.3, "JS L1965: the fire pose (D-T35-4)");
        let KindState::Crab(st) = &s.state else {
            panic!("crab state");
        };
        assert!(
            (st.hull - 1.5 * FIXED_DT).abs() < 1e-6,
            "JS L1957: hull slew at `def.turnRate`, got {}",
            st.hull
        );
        assert_eq!(
            s.face_yaw,
            Some(st.hull),
            "JS L1959: the hull owns the facing"
        );
        st.hull
    };
    assert_eq!(ps.list.len(), 1, "JS L1967: one round per cooldown slot");
    {
        let p = &ps.list[0];
        let deg = std::f32::consts::PI / 180.0;
        let sp = (r1 - 0.5) * 2.0 * 3.5 * deg;
        let sp2 = (r2 - 0.5) * 3.5 * deg;
        let exp = vec3((hull1 + sp).sin(), sp2.sin(), (hull1 + sp).cos()) * 36.0;
        assert!(
            (p.vel - exp).length() < 1e-4,
            "JS L1969-1972: spread cone around the hull aim, got {:?}",
            p.vel
        );
        assert_eq!(p.damage, 18.0, "JS L1972: `damage: def.gunDamage`");
        assert_eq!(p.straight, 0.4, "JS L1972: `straight: 0.4`");
        assert_eq!(p.radius, 0.55, "JS L1972: `radius: 0.55`");
        assert_eq!(p.trail_every, 1.6, "JS L1972: `trailEvery: 1.6`");
        assert!(
            (p.life - (22.0 / 36.0 + 0.6)).abs() < 1e-6,
            "JS L1203: `life = range / speed + 0.6`"
        );
        assert!(!p.blast, "JS L1972: `type: 'shot'`");
        assert!(p.burst.is_none(), "the gatling round has no burst");
    }
    // Catch-up (JS L1967 `guard++ < 3`): a deep negative gcd fires at most
    // three rounds in one frame.
    {
        let mut s = a.special_active.take().unwrap();
        {
            let KindState::Crab(st) = &mut s.state else {
                panic!("crab state");
            };
            st.gcd = -0.25;
        }
        crab::weapon(
            &mut a, &mut s, FIXED_DT, &inp, &mut gate, &mut sys, &mut rng, &mut ps,
        );
        a.special_active = Some(s);
    }
    assert_eq!(
        ps.list.len(),
        4,
        "JS L1967: the catch-up burst is capped at 3 rounds (D-T35-4)"
    );
    // The cannon (JS L1976-1986): sub press edge on a ready cooldown spawns
    // the mortar shell with the lead pitch into the world registry.
    let mut gate2 = crate::actor::FireGate {
        sub: true,
        pressed: true,
        ..Default::default()
    };
    {
        let mut s = a.special_active.take().unwrap();
        {
            let KindState::Crab(st) = &mut s.state else {
                panic!("crab state");
            };
            st.ccd = 0.0;
        }
        crab::weapon(
            &mut a, &mut s, FIXED_DT, &inp, &mut gate2, &mut sys, &mut rng, &mut ps,
        );
        a.special_active = Some(s);
    }
    assert_eq!(ps.list.len(), 4, "the shell is a world object, not a round");
    assert_eq!(
        sys.world.len(),
        1,
        "JS L1985: the shell enters the registry"
    );
    assert_eq!(sys.world[0].kind, "sh", "JS L1157: the shell wire tag");
    let hull3 = {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Crab(st) = &s.state else {
            panic!("crab state");
        };
        assert_eq!(st.ccd, 1.15, "JS L1977: `ccd = def.cannonGap`");
        st.hull
    };
    {
        // JS L1980: `lp = clamp(aimPitch + 0.45, 0.1, 1.0)` (D-T35-4).
        let lp = 0.45_f32;
        let exp_vel = vec3(hull3.sin() * lp.cos(), lp.sin(), hull3.cos() * lp.cos()) * 17.0;
        if let world::ObjectBody::Shell(sh) = &sys.world[0].body {
            assert!(
                (sh.vel - exp_vel).length() < 1e-4,
                "JS L1979-1983: lead pitch at `def.cannonSpeed`, got {:?}",
                sh.vel
            );
        } else {
            panic!("shell body");
        }
        // JS L1986/L300: `[2,'sh',gid, ...v3(muzzle), ...v3(vel)]`.
        let (c3, s3) = (hull3.cos(), hull3.sin());
        let m = vec3(
            a.pos.x + 0.5 * c3 - 0.2 * s3,
            a.pos.y + 1.1,
            a.pos.z - 0.5 * s3 - 0.2 * c3,
        );
        let exp_rec: Vec<KitVal> = vec![k(2.0), str_("sh"), k(1.0)]
            .into_iter()
            .chain(world::v3(m).into_iter().map(k))
            .chain(world::v3(exp_vel).into_iter().map(k))
            .collect();
        assert_eq!(
            *a.sp_records.last().unwrap(),
            exp_rec,
            "JS L1986: the spawn record"
        );
    }
    // Roll (JS L1948-1953/L1960): swim rolls the ball, swaps to `rollSpeed`
    // and the trigger is owned but never fires.
    let inp_roll = ActorInput {
        squid: true,
        ..Default::default()
    };
    {
        let mut s = a.special_active.take().unwrap();
        let r = crab::weapon(
            &mut a, &mut s, FIXED_DT, &inp_roll, &mut gate, &mut sys, &mut rng, &mut ps,
        );
        a.special_active = Some(s);
        assert!(r, "JS L1987: the crab owns the trigger");
    }
    {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(s.speed, Some(9.5), "JS L1950: `speed = def.rollSpeed`");
        let KindState::Crab(st) = &s.state else {
            panic!("crab state");
        };
        assert!(st.roll, "JS L1948: swim rolls the crab");
    }
    assert_eq!(ps.list.len(), 4, "JS L1960: a rolled crab cannot shoot");
    // The damage ladder (JS L2014-2026): roll armour, then front absorb and
    // the exposed behind / above sectors.
    let ap = a.pos;
    let hp0 = crab_hp(&a);
    assert_eq!(
        crab::hurt(&mut a, 20.0, Some(vec3(0.0, ap.y, ap.z + 5.0))),
        0.0,
        "JS L2026: the hull absorbs"
    );
    assert_eq!(
        crab_hp(&a),
        hp0 - 7.0,
        "JS L2023: `hp -= amount * rollArmor` (20 · 0.35)"
    );
    let inp_roll_off = ActorInput::default();
    let mut gate_off = crate::actor::FireGate::default();
    {
        let mut s = a.special_active.take().unwrap();
        crab::weapon(
            &mut a,
            &mut s,
            FIXED_DT,
            &inp_roll_off,
            &mut gate_off,
            &mut sys,
            &mut rng,
            &mut ps,
        );
        a.special_active = Some(s);
    }
    assert_eq!(
        a.special_active.as_ref().unwrap().speed,
        Some(3.0),
        "JS L1950: unrolled"
    );
    let hp1 = crab_hp(&a);
    assert_eq!(
        crab::hurt(&mut a, 30.0, Some(vec3(0.0, ap.y, ap.z + 5.0))),
        0.0
    );
    assert_eq!(crab_hp(&a), hp1 - 30.0, "JS L2023: no armour when unrolled");
    assert_eq!(
        crab::hurt(&mut a, 40.0, Some(vec3(0.0, ap.y, ap.z - 5.0))),
        40.0,
        "JS L2020: from behind the rider takes it (`front < -0.45`, D-T35-4)"
    );
    assert_eq!(
        crab_hp(&a),
        hp1 - 30.0,
        "the hull is untouched by a rider hit"
    );
    assert_eq!(
        crab::hurt(&mut a, 50.0, Some(vec3(0.0, ap.y + 3.0, ap.z))),
        50.0,
        "JS L2020: from above (`ap.y > pos.y + 2.4`, D-T35-4)"
    );
    // The hull break (JS L2012 `s.hp <= 0 → end('broken')` + L2040 hop).
    {
        let mut s = a.special_active.take().unwrap();
        {
            let KindState::Crab(st) = &mut s.state else {
                panic!("crab state");
            };
            st.hp = 10.0;
        }
        a.special_active = Some(s);
    }
    assert_eq!(
        crab::hurt(&mut a, 10.0, Some(vec3(0.0, ap.y, ap.z + 5.0))),
        0.0
    );
    crate::special::tick(&mut a, FIXED_DT, &ActorInput::default());
    assert!(
        a.special_active.is_none(),
        "JS L2012: the break ends the special"
    );
    assert_eq!(a.vel.y, 5.0, "JS L2040: the dismount hop (D-T35-4)");
    assert!(!a.grounded, "JS L2040");
    assert_eq!(
        a.sp_marker_action,
        Some(false),
        "JS L373: crab is not `jumpBack`"
    );
    assert_eq!(
        a.sp_records.last().map(|r| (r[0].clone(), r[1].clone())),
        Some((KitVal::Num(1.0), KitVal::Str("broken".to_string()))),
        "JS L367: the end records `[1,'broken']`"
    );
    // The rolled footprint paint (JS L2005): `legT > 0.08` on grounded roll
    // above 2 m/s raises the drain flag; the splat credits turf only.
    let mut b = Actor::new(1, 0, t);
    b.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut b, "crab");
    {
        let mut s = b.special_active.take().unwrap();
        {
            let KindState::Crab(st) = &mut s.state else {
                panic!("crab state");
            };
            st.roll = true;
        }
        b.special_active = Some(s);
    }
    b.grounded = true;
    b.vel.z = 3.0;
    let mut painted_at = 0usize;
    for fr in 1..=8 {
        let mut s = b.special_active.take().unwrap();
        crab::tick(&mut b, &mut s, FIXED_DT);
        b.special_active = Some(s);
        if std::mem::take(&mut b.sp_crab_paint) {
            assert_eq!(painted_at, 0, "one footprint per leg period");
            painted_at = fr;
        }
    }
    assert!(
        (5..=6).contains(&painted_at),
        "JS L2005: `legT > 0.08` (D-T35-4), got frame {painted_at}"
    );
    let mut paint = crate::paint::PaintGrid::new(&f.world);
    let mut rng2 = Rng::new(0x5EED);
    let sp0 = b.special;
    crab::paint_footprint(&mut b, &f.world, &mut paint, &mut rng2, false);
    assert!(b.turf > 0.0, "JS L2005: the rolled tracks paint");
    assert_eq!(
        b.special, sp0,
        "JS L53: special ink never charges the gauge"
    );
    // The ghost-side muted drain (JS L296 `ghostMute` → L2118 → L2005 → L53):
    // the seed `Math.random()` is evaluated at the call site *before*
    // `PaintSystem.splat` returns 0 under the mute (paint.js L470), so the
    // draw is consumed even though nothing paints.
    let mut probe = Rng::new(0x5EED);
    let _d1 = probe.next_f32();
    let d2 = probe.next_f32();
    let mut rng3 = Rng::new(0x5EED);
    let turf0 = b.turf;
    crab::paint_footprint(&mut b, &f.world, &mut paint, &mut rng3, true);
    assert_eq!(
        rng3.next_f32(),
        d2,
        "the muted footprint still consumes the seed draw"
    );
    assert_eq!(b.turf, turf0, "the muted footprint paints nothing");
}

#[test]
fn tr35_b4_stamp_swing_flip_guard_throw_slam() {
    // JS `IMPL.stamp` (specials.js L1598-1714), tuning config.js L450-453.
    let t_all = crate::embedded_tuning();
    let t = &t_all.player;
    let d = &specials().stamp;
    // Rust tuple traits stop at 12 elements — the table is asserted in two
    // halves.
    assert_eq!(
        (
            d.duration,
            d.interval,
            d.reach,
            d.radius,
            d.damage,
            d.lunge,
            d.move_speed,
            d.throw_speed,
            d.throw_radius,
            d.throw_damage_max,
            d.throw_damage_min,
            d.turn_rate
        ),
        (
            7.5, 0.42, 1.6, 1.55, 200.0, 2.6, 7.2, 25.0, 3.2, 200.0, 60.0, 1.7
        )
    );
    assert_eq!(
        (
            d.turn_rate_still,
            d.strafe,
            d.guard_time,
            d.deflect_arc,
            d.deflect_r,
            d.bomb_clear_r,
            d.flip_time,
            d.flip_reach,
            d.flip_radius,
            d.flip_back_reach,
            d.flip_back_radius
        ),
        (6.0, 0.35, 0.45, 70.0, 2.6, 3.2, 0.5, 2.7, 1.9, 1.2, 1.6)
    );
    let world = wall_world();
    let layout = crate::embedded_tidewater();
    let sim = crate::actor::SimWorld::new(&world, &layout);
    let k = |v: f32| KitVal::Num(v);
    let str_ = |v: &str| KitVal::Str(v.to_string());
    let mut a = actor_with_special(t, "stamp", 1000.0);
    a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut a, "stamp");
    {
        let s = a.special_active.as_ref().unwrap();
        assert_eq!(s.speed, Some(7.2), "JS L1600: `speed: def.moveSpeed`");
        assert!(s.no_squid, "JS L1600: the stamp never swims");
        assert_eq!(s.face_yaw, Some(a.yaw), "JS L1600: `faceYaw: a.yaw`");
        let KindState::Stamp(st) = &s.state else {
            panic!("stamp state");
        };
        assert_eq!(st.pending, -1.0, "JS L1600: `pending: -1` (none)");
        assert_eq!(st.body_yaw, a.yaw, "JS L1600: `bodyYaw: a.yaw`");
    }
    // The body-yaw slew (JS L1605-1609): stopped turns at `turnRateStill`.
    a.aim_yaw = std::f32::consts::FRAC_PI_2;
    let mut intent = ActorInput {
        move_dir: vec3(0.0, 0.0, 1.0),
        ..Default::default()
    };
    {
        let mut s = a.special_active.take().unwrap();
        stamp::move_hook(&a, &mut s, FIXED_DT, &mut intent);
        a.special_active = Some(s);
    }
    let by1 = {
        let s = a.special_active.as_ref().unwrap();
        let KindState::Stamp(st) = &s.state else {
            panic!("stamp state");
        };
        assert!(
            (st.body_yaw - 6.0 * FIXED_DT).abs() < 1e-6,
            "JS L1607: stopped slew is `turnRateStill` (D-T35-4), got {}",
            st.body_yaw
        );
        assert_eq!(
            s.face_yaw,
            Some(st.body_yaw),
            "JS L1609: the body owns the facing"
        );
        st.body_yaw
    };
    {
        // JS L1614-1616: the along/strafe shuffle (sideways input scaled by
        // `def.strafe`, D-T35-4).
        let f = vec3(by1.sin(), 0.0, by1.cos());
        let along = f.z; // mv = (0, 0, 1)
        let want_x = f.x * along + (0.0 - f.x * along) * 0.35;
        let want_z = f.z * along + (1.0 - f.z * along) * 0.35;
        assert!(
            (intent.move_dir.x - want_x).abs() < 1e-6,
            "JS L1615: strafe scale"
        );
        assert!((intent.move_dir.z - want_z).abs() < 1e-6, "JS L1616");
    }
    // The grounded swing (JS L1641-1647): `pending = 0.15` wind-up, then the
    // queued smash + lunge along the body facing.
    let mut sys = SpecialSystem::new();
    let gate_fire = crate::actor::FireGate {
        fire: true,
        pressed: true,
        ..Default::default()
    };
    let mut slam_frame = 0usize;
    for fr in 1..=12 {
        let mut s = a.special_active.take().unwrap();
        let (r, reason) = stamp::weapon(&mut a, &mut s, FIXED_DT, &gate_fire, &mut sys);
        a.special_active = Some(s);
        assert!(r, "JS L1656: the stamp owns the trigger");
        assert_eq!(reason, None, "the swing never ends the special");
        if fr == 1 {
            let s = a.special_active.as_ref().unwrap();
            let KindState::Stamp(st) = &s.state else {
                panic!("stamp state");
            };
            assert_eq!(s.firing, 0.5, "JS L1646: the swing pose (D-T35-4)");
            assert_eq!(st.cd, 0.42, "JS L1643: `cd = def.interval`");
            assert_eq!(st.guard, 0.45, "JS L1644: `guard = def.guardTime`");
        }
        // Drain the queue the way the match runner does, so a second frame
        // never re-reports the same smash (JS L1651 pushes once per swing).
        let drained = std::mem::take(&mut a.sp_stamp_slam);
        if !drained.is_empty() {
            assert_eq!(slam_frame, 0, "one smash per swing");
            assert_eq!(
                drained,
                vec![(1.55, 1.6)],
                "JS L1651: `(def.radius, def.reach)`"
            );
            slam_frame = fr;
        }
    }
    assert!(
        (9..=11).contains(&slam_frame),
        "JS L1642/L1649: the 0.15 s wind-up (D-T35-4), got frame {slam_frame}"
    );
    {
        let f = vec3(by1.sin(), 0.0, by1.cos());
        assert!(
            (a.vel.x - f.x * 2.6).abs() < 1e-6 && (a.vel.z - f.z * 2.6).abs() < 1e-6,
            "JS L1652-1653: the lunge shove along the body facing, got {:?}",
            a.vel
        );
    }
    // The guard sweep (JS L1672-1681): enemy shots in the front arc are
    // knocked out; behind-arc and own-team shots survive.
    let mut ps = crate::weapon::ProjectileSim::new(5);
    let mk_shot = |z: f32, team: usize| crate::weapon::Projectile {
        pos: vec3(0.0, a.pos.y + 1.0, z),
        prev: vec3(0.0, a.pos.y + 1.0, z),
        start: vec3(0.0, a.pos.y + 1.0, z),
        vel: vec3(0.0, 0.0, if z > 0.0 { -10.0 } else { 10.0 }),
        team,
        owner_slot: if team == 0 { 0 } else { 1 },
        age: 0.0,
        life: 2.0,
        straight: 0.0,
        radius: 0.55,
        size: 0.14,
        damage: 18.0,
        grav: 0.0,
        drag: 0.0,
        trail: 0.0,
        trail_every: 0.0,
        trail_radius: 0.35,
        seed: 0.5,
        blast: false,
        pellet: None,
        burst: None,
        ghost: false,
        below_falloff: 0.0,
        below_falloff_max: 0.0,
        volley: None,
        dmg_far: None,
        delay: 0.0,
        sp: None,
    };
    ps.list.push(mk_shot(1.0, 1)); // front enemy shot — deflected
    ps.list.push(mk_shot(-1.5, 1)); // behind the arc — survives
    ps.list.push(mk_shot(1.0, 0)); // own team — ignored
    let mut subs = crate::sub::SubSystem::new(5);
    stamp::tick_full(&mut a, FIXED_DT, &mut ps, &mut subs);
    assert_eq!(
        ps.list.len(),
        2,
        "JS L1679: the front-arc shot is knocked out"
    );
    // `remove_shot` swap-removes (JS splices): the own-team tail lands at
    // index 0 and the behind shot keeps its slot at index 1.
    assert_eq!(
        ps.list[0].team, 0,
        "JS L1676: own-team shots are never touched"
    );
    assert!(
        (ps.list[1].pos.z - (-1.5)).abs() < 1e-6,
        "JS L1678: the behind shot passes the arc gate"
    );
    // The airborne flip (JS L1634-1640 / L1661-1664): back smash at
    // `t >= flipTime·0.3`, front smash at `t >= flipTime·0.75`, ends at
    // `flipTime`.
    let mut b = actor_with_special(t, "stamp", 1000.0);
    b.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut b, "stamp");
    b.grounded = false;
    {
        let mut s = b.special_active.take().unwrap();
        let (_, reason) = stamp::weapon(&mut b, &mut s, FIXED_DT, &gate_fire, &mut sys);
        b.special_active = Some(s);
        assert_eq!(reason, None, "the flip never ends the special");
    }
    {
        let s = b.special_active.as_ref().unwrap();
        let KindState::Stamp(st) = &s.state else {
            panic!("stamp state");
        };
        assert!(st.flip.is_some(), "JS L1635: airborne fire flips");
        assert_eq!(st.guard, 0.5, "JS L1636: `guard = def.flipTime`");
        assert_eq!(st.cd, 0.6, "JS L1637: `cd = flipTime + 0.1` (D-T35-4)");
        assert_eq!(s.firing, 0.5, "JS L1646: the flip pose");
    }
    let mut slams: Vec<(f32, f32)> = Vec::new();
    let mut flip_end = 0usize;
    let mut ps_b = crate::weapon::ProjectileSim::new(5);
    let mut subs_b = crate::sub::SubSystem::new(5);
    for fr in 1..=32 {
        stamp::tick_full(&mut b, FIXED_DT, &mut ps_b, &mut subs_b);
        slams.extend(std::mem::take(&mut b.sp_stamp_slam));
        let s = b.special_active.as_ref().unwrap();
        let KindState::Stamp(st) = &s.state else {
            panic!("stamp state");
        };
        if st.flip.is_none() && flip_end == 0 {
            flip_end = fr;
        }
    }
    assert_eq!(
        slams,
        vec![(1.6, -1.2), (1.9, 2.7)],
        "JS L1662-1663: back (`flipBackRadius, -flipBackReach`) then front"
    );
    assert!(
        (29..=31).contains(&flip_end),
        "JS L1664: the flip ends at `flipTime = 0.5` s, got frame {flip_end}"
    );
    // The throw (JS L1623-1632): the sub press spawns the flying stamp and
    // ends the special.
    let mut c = actor_with_special(t, "stamp", 1000.0);
    c.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut c, "stamp");
    let mut sys_c = SpecialSystem::new();
    let gate_sub = crate::actor::FireGate {
        sub: true,
        pressed: true,
        ..Default::default()
    };
    {
        let mut s = c.special_active.take().unwrap();
        let (r, reason) = stamp::weapon(&mut c, &mut s, FIXED_DT, &gate_sub, &mut sys_c);
        c.special_active = Some(s);
        assert!(r, "JS L1631: the stamp owns the trigger");
        assert_eq!(
            reason,
            Some("throw"),
            "JS L1630: the throw ends the special"
        );
    }
    assert_eq!(
        sys_c.world.len(),
        1,
        "JS L1629: the thrown stamp enters the world"
    );
    assert_eq!(
        sys_c.world[0].kind, "ts",
        "JS L1105: the thrown-stamp wire tag"
    );
    assert_eq!(
        sys_c.world[0].pos,
        vec3(0.0, c.pos.y + 1.5, 0.0),
        "JS L1628: throw origin `pos + 1.5y` (D-T35-4)"
    );
    if let world::ObjectBody::TStamp(ts) = &sys_c.world[0].body {
        assert_eq!(
            ts.vel,
            vec3(0.0, 2.5, 25.0),
            "JS L1108: dir·throwSpeed + the 2.5 lift (D-T35-4)"
        );
    } else {
        panic!("tstamp body");
    }
    {
        // JS L1629/L300: `[2,'ts',gid, ...v3(from), ...v3(dir)]`.
        let exp_rec: Vec<KitVal> = vec![k(2.0), str_("ts"), k(1.0)]
            .into_iter()
            .chain(world::v3(vec3(0.0, c.pos.y + 1.5, 0.0)).into_iter().map(k))
            .chain(world::v3(vec3(0.0, 0.0, 1.0)).into_iter().map(k))
            .collect();
        assert_eq!(
            *c.sp_records.last().unwrap(),
            exp_rec,
            "JS L1629: the spawn record"
        );
    }
    crate::special::end(&mut c, "throw");
    assert!(
        c.special_active.is_none(),
        "JS L1630: the throw clears the active"
    );
    assert_eq!(
        c.sp_marker_action,
        Some(false),
        "JS L373: stamp is not `jumpBack`"
    );
    assert!(c.super_jump.is_none(), "a thrown stamp never jumps home");
    // The smash (JS L1694-1712): ground probe, two splats, the record and
    // the flat-damage disc with LOS.
    let mut c2 = actor_with_special(t, "stamp", 1000.0);
    c2.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
    start(&mut c2, "stamp");
    let mut e = Actor::new(1, 1, t);
    e.spawn_at(vec3(0.3, 0.05, 1.6), 0.0, &sim, t);
    // Clear the spawn invulnerability so the slam damage lands (JS `_spawn`
    // arms it; `apply_hit` blocks while `invuln > 0`).
    e.set_invuln(0.0);
    let turf0 = c2.turf;
    let sp0 = c2.special;
    let mut actors = [c2, e];
    let mut paint = crate::paint::PaintGrid::new(&world);
    let mut rng = Rng::new(0x57);
    let mut ps2 = crate::weapon::ProjectileSim::new(5);
    let mut subs2 = crate::sub::SubSystem::new(5);
    stamp::slam(
        &mut actors,
        0,
        1.55,
        1.6,
        &world,
        &mut paint,
        &mut rng,
        &mut ps2,
        &mut subs2,
        None,
        t,
    );
    // JS L1697: the probe from `pos + 1.2y` down 3 m lands on the deck at
    // `y = 0`, so `at = (0, 0, 1.6)`.
    let rec = actors[0].sp_records.last().unwrap();
    assert_eq!(
        &rec[..2],
        &[k(4.0), str_("sl")],
        "JS L1701: the slam record tag"
    );
    assert_eq!(
        &rec[2..],
        &[k(0.0), k(0.0), k(1.6), k(1.55)],
        "JS L1701: `...v3(at), r2(radius)`"
    );
    let evs = ps2.drain_events();
    assert_eq!(
        evs.len(),
        1,
        "JS L1704-1710: one victim in the disc with LOS"
    );
    let crate::weapon::SimEvent::Hit {
        victim_slot,
        damage,
        killed,
        ..
    } = evs[0]
    else {
        panic!("hit event, got {evs:?}");
    };
    assert_eq!(
        (victim_slot, damage, killed),
        (1, 200.0, true),
        "JS L1709: flat `def.damage` in the splash disc"
    );
    assert!(!actors[1].alive, "the 200 damage splats the 100 hp enemy");
    assert!(
        actors[0].turf > turf0,
        "JS L1699-1700: the two splats credit turf"
    );
    assert_eq!(
        actors[0].special, sp0,
        "JS L53: special ink never charges the gauge"
    );
}

#[test]
fn tr35_b4_determinism_same_seed_identical_streams() {
    // The batch-4 kinds draw RNG only through the shared xorshift stream
    // (the gatling spread pairs, JS L1970): the same seed must replay the
    // same records, body state, projectile stream and world registry.
    let run = || {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let mut a = actor_with_special(t, "crab", 1000.0);
        a.spawn_at(vec3(0.0, 0.05, 0.0), 0.0, &sim, t);
        a.aim_yaw = 0.7;
        a.aim_pitch = 0.1;
        start(&mut a, "crab");
        let mut sys = SpecialSystem::new();
        let mut ps = crate::weapon::ProjectileSim::new(5);
        let mut paint = crate::paint::PaintGrid::new(&f.world);
        let mut rng = Rng::new(0xBEEF);
        let mut gate = crate::actor::FireGate::default();
        let inp = ActorInput::default();
        for fr in 1..=40 {
            gate.fire = true;
            gate.pressed = fr == 1;
            // JS L1976: the cannon needs the sub press edge *and* a ready
            // cooldown (`ccd` starts at 0.4, so frame 26 is the first hit).
            gate.sub = fr == 26;
            gate.sub_released = fr == 27;
            let _ = crate::special::weapon_hook(
                &mut a, FIXED_DT, &mut sys, &f.world, &mut rng, &mut paint, &mut ps, t, &inp,
                &mut gate,
            );
            crate::special::tick(&mut a, FIXED_DT, &inp);
        }
        (
            a.sp_records.clone(),
            a.pos,
            a.vel,
            ps.list.len(),
            sys.world.len(),
        )
    };
    let r1 = run();
    let r2 = run();
    assert_eq!(r1.0, r2.0, "identical seeds ⇒ identical special records");
    assert_eq!(r1.1, r2.1, "identical seeds ⇒ identical positions");
    assert_eq!(r1.2, r2.2, "identical seeds ⇒ identical velocities");
    assert_eq!(r1.3, r2.3, "identical seeds ⇒ identical projectile streams");
    assert_eq!(r1.4, r2.4, "identical seeds ⇒ identical world registries");
    assert!(
        r1.3 > 5,
        "the gatling stream must be busy, got {} rounds",
        r1.3
    );
    assert_eq!(r1.4, 1, "the frame-26 sub press fired exactly one shell");
}

// ---------------------------------------------------------------------------
// Review P0-1 regression: JS `shouldApplyHit` (netmatch.js L711-717) checks
// `attacker.remote → 'drop'` *before* the victim, so a ghost world object's
// blast (`ts` / `sh` / `or` op-2 spawns, the ghost bubble `netEvent 'x'`)
// must never hurt anyone on the receiving screen.
// ---------------------------------------------------------------------------

#[test]
fn tr35_b4_review_ghost_blast_attacker_drop() {
    let t_all = crate::embedded_tuning();
    let t = &t_all.player;
    let world = wall_world();
    let mut paint = crate::paint::PaintGrid::new(&world);
    let d_ts = &specials().stamp;

    // Roster: [0] the team-1 owner (slot 1 — the ghost's owner proxy),
    // [1] a team-0 local victim in the blast disc.
    let roster = || {
        let mut owner = Actor::new(1, 1, t);
        owner.pos = vec3(0.0, 0.0, -4.0);
        let mut victim = Actor::new(0, 0, t);
        victim.pos = vec3(0.0, 0.0, 0.5);
        [owner, victim]
    };

    // 1. Receiving screen: the ghost `ts` (op-2 ghost spawn) booms on the
    //    deck next to the local victim — every hit drops (JS L714).
    {
        let mut actors = roster();
        let hp0 = actors[1].hp;
        let remote = [true, false];
        let mut sys = SpecialSystem::new();
        sys.world.push(world::make_tstamp(
            1,
            1,
            true,
            vec3(0.0, 0.3, 0.5),
            vec3(0.0, -1.0, 0.0),
        ));
        let mut events = Vec::new();
        let mut rng = Rng::new(0x9);
        let mut subs = crate::sub::SubSystem::new(1);
        {
            let mut ctx = world::WorldCtx {
                actors: &mut actors,
                collision: &world,
                paint: &mut paint,
                rng: &mut rng,
                subs: &mut subs,
                sb: &t_all.subs,
                t,
                events: &mut events,
                remote: Some(&remote),
                time: 0.0,
            };
            world::step_special_objects(&mut sys.world, FIXED_DT, &mut ctx);
        }
        assert!(sys.world.is_empty(), "the ghost stamp boomed on the deck");
        assert!(
            events.iter().all(|e| !matches!(
                e,
                crate::weapon::SimEvent::Hit { .. } | crate::weapon::SimEvent::HitSend { .. }
            )),
            "JS netmatch.js L711-717: attacker.remote → 'drop', got {events:?}"
        );
        assert_eq!(actors[1].hp, hp0, "the local victim is untouched");
        assert!(actors[1].alive);
    }
    // 2. Owner screen (attacker local): the same boom routes `Hit` for the
    //    local victim and `HitSend` for the remote one (JS L715).
    {
        let mut owner = Actor::new(1, 1, t);
        owner.pos = vec3(0.0, 0.0, -4.0);
        let mut v_local = Actor::new(0, 0, t);
        v_local.pos = vec3(0.0, 0.0, 0.5);
        let mut v_remote = Actor::new(0, 2, t);
        v_remote.pos = vec3(0.0, 0.0, 0.9);
        let rhp0 = v_remote.hp;
        let remote = [false, false, true];
        let mut actors = [owner, v_local, v_remote];
        let mut sys = SpecialSystem::new();
        sys.world.push(world::make_tstamp(
            1,
            1,
            false,
            vec3(0.0, 0.3, 0.5),
            vec3(0.0, -1.0, 0.0),
        ));
        let mut events = Vec::new();
        let mut rng = Rng::new(0x9);
        let mut subs = crate::sub::SubSystem::new(1);
        {
            let mut ctx = world::WorldCtx {
                actors: &mut actors,
                collision: &world,
                paint: &mut paint,
                rng: &mut rng,
                subs: &mut subs,
                sb: &t_all.subs,
                t,
                events: &mut events,
                remote: Some(&remote),
                time: 0.0,
            };
            world::step_special_objects(&mut sys.world, FIXED_DT, &mut ctx);
        }
        assert_eq!(events.len(), 2, "one local hit + one send");
        let crate::weapon::SimEvent::Hit {
            victim_slot,
            damage,
            killed,
            ..
        } = events[0]
        else {
            panic!("hit event, got {:?}", events[0]);
        };
        assert_eq!(
            (victim_slot, damage, killed),
            (0, d_ts.throw_damage_max, true),
            "JS L72/L73: flat inside `killRadius`, local victim takes the damage"
        );
        let crate::weapon::SimEvent::HitSend {
            victim_slot,
            damage,
            ..
        } = events[1]
        else {
            panic!("hit-send event, got {:?}", events[1]);
        };
        assert_eq!(
            (victim_slot, damage),
            (2, d_ts.throw_damage_max),
            "JS netmatch.js L715: victim.remote → 'send'"
        );
        assert_eq!(actors[1].hp, 0.0, "the kill clamps hp at 0");
        assert!(!actors[1].alive, "the 200 damage splats the 100 hp victim");
        assert_eq!(actors[2].hp, rhp0, "a remote victim is never hurt here");
    }
    // 3. The ghost bubble `netEvent 'x'` route (JS L1080 → `explode(null)`,
    //    the sim's op-3 arm): same attacker gate.
    let mk_bubble = |ghost: bool| world::WorldObject {
        kind: "bu".to_string(),
        gid: 7,
        team: 1,
        pos: vec3(0.0, 0.6, 0.5),
        ghost,
        dead: false,
        body: world::ObjectBody::Bubble(world::Bubble {
            owner_slot: 1,
            t: 0.0,
            life: 1.0,
            r: 2.0,
            vel: Vec3::ZERO,
            charge: 0.0,
            held: false,
            ph: 0.0,
        }),
    };
    {
        let mut actors = roster();
        let hp0 = actors[1].hp;
        let remote = [true, false];
        let mut objects = vec![mk_bubble(true)];
        let mut events = Vec::new();
        let mut rng = Rng::new(0x11);
        let mut subs = crate::sub::SubSystem::new(1);
        {
            let mut ctx = world::WorldCtx {
                actors: &mut actors,
                collision: &world,
                paint: &mut paint,
                rng: &mut rng,
                subs: &mut subs,
                sb: &t_all.subs,
                t,
                events: &mut events,
                remote: Some(&remote),
                time: 0.0,
            };
            world::bubble_explode(0, None, &mut objects, &mut ctx);
        }
        assert!(
            events.iter().all(|e| !matches!(
                e,
                crate::weapon::SimEvent::Hit { .. } | crate::weapon::SimEvent::HitSend { .. }
            )),
            "JS L714: the ghost bubble's blast drops too, got {events:?}"
        );
        assert_eq!(actors[1].hp, hp0, "the local victim survives the ghost pop");
    }
    {
        let mut actors = roster();
        let hp0 = actors[1].hp;
        let remote = [false, false];
        let mut objects = vec![mk_bubble(false)];
        let mut events = Vec::new();
        let mut rng = Rng::new(0x11);
        let mut subs = crate::sub::SubSystem::new(1);
        {
            let mut ctx = world::WorldCtx {
                actors: &mut actors,
                collision: &world,
                paint: &mut paint,
                rng: &mut rng,
                subs: &mut subs,
                sb: &t_all.subs,
                t,
                events: &mut events,
                remote: Some(&remote),
                time: 0.0,
            };
            world::bubble_explode(0, None, &mut objects, &mut ctx);
        }
        assert!(
            matches!(
                events[0],
                crate::weapon::SimEvent::Hit { victim_slot: 0, .. }
            ),
            "JS L716: local attacker + local victim → 'local', got {events:?}"
        );
        assert!(actors[1].hp < hp0, "the owner screen's blast lands");
    }
}
