//! `IMPL.zipcaster` (specials.js L1772-1905) — Zipline: the sub button
//! fires a tether; the body-owned zip pulls you to the anchor and you hang
//! (`s.hang`) until the jump / push-away releases. The end routes the
//! `jumpBack` marker through [`crate::special::end`] (JS L374).

use crate::actor::{Actor, ActorInput, FireGate, SimWorld, SpecialActive, ZipImpact};
use crate::collision::CollisionWorld;
use crate::net::KitVal;
use crate::paint::{PaintGrid, SplatOpts};
use crate::sub::SubSystem;
use crate::tuning::PlayerTuning;
use crate::weapon::{ProjectileSim, Rng, SimEvent, apply_hit};
use glam::{Vec3, vec3};

/// JS L1794: tether origin (`pos.y + 1.2`) — hardcoded (D-T35-4).
const FROM_UP: f32 = 1.2;
/// JS L1800: the wall classification `|n.y| < 0.55` — hardcoded.
const WALL_NY: f32 = 0.55;
/// JS L1801: anchor push-off `PLAYER.radius + 0.08` — the `0.08` literal.
const ANCHOR_PUSH: f32 = 0.08;
/// JS L1802: wall anchors drop `1.0` — hardcoded.
const WALL_Y_DROP: f32 = 1.0;
/// JS L1807: latch paint `point + n * 0.1`, radius `1.0` — hardcoded.
const LATCH_PUSH: f32 = 0.1;
/// JS L1807: latch paint radius.
const LATCH_R: f32 = 1.0;
/// JS L1827: the flight timeout (`z.t > 1.6`) — hardcoded.
const FLIGHT_TIMEOUT: f32 = 1.6;
/// JS L1827: the stuck budget (`z.stuck > 0.15`) — hardcoded.
const STUCK_MAX: f32 = 0.15;
/// JS L1839: progress below `step * 0.3` counts as stuck — hardcoded.
const STUCK_FRAC: f32 = 0.3;
/// JS L1820: the slam scan `hypot(dx,dz) < PLAYER.radius*2 + 0.2` and
/// `|dy| < 1.4` — the `0.2` / `1.4` literals (D-T35-4).
const SLAM_XZ_BONUS: f32 = 0.2;
/// JS L1820: the slam vertical gate.
const SLAM_DY: f32 = 1.4;
/// JS L1847: impact centre `pos.y + 0.7` — hardcoded.
const IMPACT_UP: f32 = 0.7;
/// JS L1851: victim probe `e.pos.y + 0.8` — hardcoded.
const VICTIM_UP: f32 = 0.8;
/// JS L1853: splash reach margin `+ 0.3` — hardcoded.
const SPLASH_MARGIN: f32 = 0.3;
/// JS L1858: impact ground probe (`groundBelow(c, 3)`).
const IMPACT_PROBE: f32 = 3.0;
/// JS L1859: impact paint `+ 0.2y`, radius `impactRadius * 0.75`.
const IMPACT_PAINT_LIFT: f32 = 0.2;
/// JS L1859: impact paint radius fraction.
const IMPACT_PAINT_FRAC: f32 = 0.75;
/// JS L1883: the push-away dot gate (`> 0.5`) — hardcoded.
const AWAY_DOT: f32 = 0.5;
/// JS L1887: the wall jump-off `vel = (n.x * 4, 8, n.z * 4)` — hardcoded.
const PUSH_XZ: f32 = 4.0;
/// JS L1887: the wall jump-off vertical.
const PUSH_Y: f32 = 8.0;

/// JS `s.zip` (L1803): the tether flight.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ZipFlight {
    /// JS `z.to`: the pull target (anchor + push-off, wall-adjusted).
    pub to: Vec3,
    /// JS `z.n`: the surface normal at the anchor.
    pub n: Vec3,
    /// JS `z.wall`: `|n.y| < 0.55` — arrival clings instead of standing.
    pub wall: bool,
    /// JS `z.anchor`: the raw ray hit point (the `['4','zf'…]` record).
    pub anchor: Vec3,
    /// JS `z.t`: seconds of flight.
    pub t: f32,
    /// JS `z.stuck`: the stuck budget.
    pub stuck: f32,
}

/// Kind state (JS `s.origin`, `s.cd`, `s.subWas`, `s.zip`, `s.wisp`, L1774).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.origin` (L1774): the special's start position (`jumpBack`
    /// target).
    pub origin: Vec3,
    /// JS `s.cd` (L1774/1796): seconds until the tether can fire again.
    pub cd: f32,
    /// JS `s.subWas` (L1774): the sub-press edge tracker.
    pub sub_was: bool,
    /// JS `s.zip` (L1774/1803): the tether flight (`None` = idle).
    pub zip: Option<ZipFlight>,
    /// JS `s.wisp` (L1774): aura visual timer — out of sim scope (the aura /
    /// tether line / miss timer are camera-gated presentation, JS L1866-1878
    /// / L1890-1902).
    pub wisp: f32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            cd: 0.0,
            sub_was: false,
            zip: None,
            wisp: 0.0,
        }
    }
}

/// JS `IMPL.zipcaster.start` (L1773-1784).
pub fn start(a: &mut Actor, s: &mut SpecialActive) {
    s.jump_back = true; // JS L1774
    a.sp_marker = Some("zipcaster".to_string()); // JS L1775: `ReturnMarker`
    s.state = crate::special::KindState::Zipcaster(State {
        origin: a.pos,
        ..State::default()
    });
}

/// JS `IMPL.zipcaster.weapon` (L1786-1792): the sub press fires the tether
/// and swallows the sub trigger (the main weapon keeps working — the hook
/// returns `false`). The aura / line visuals of `fire` are presentation.
#[allow(clippy::too_many_arguments)]
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    gate: &mut FireGate,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    t: &PlayerTuning,
) {
    let d = &crate::special::specials().zipcaster;
    let crate::special::KindState::Zipcaster(st) = &mut s.state else {
        return;
    };
    st.cd -= dt;
    let press = gate.sub && !st.sub_was;
    st.sub_was = gate.sub;
    // JS L1789: the sub trigger is consumed by the tether.
    gate.sub = false;
    gate.sub_released = false;
    if press && st.cd <= 0.0 && st.zip.is_none() && fire(a, st, d, collision, paint, rng, t) {
        // JS L1806: the zip takes over the body this frame.
        s.body = true;
        s.hang = 0.0;
    }
}

/// JS `IMPL.zipcaster.fire` (L1793-1810). Returns whether the tether
/// latched (JS sets `s.body` / `s.hang` only on a hit, L1806).
#[allow(clippy::too_many_arguments)]
fn fire(
    a: &mut Actor,
    st: &mut State,
    d: &crate::tuning::Zipcaster,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    t: &PlayerTuning,
) -> bool {
    let from = vec3(a.pos.x, a.pos.y + FROM_UP, a.pos.z);
    // JS L1795 aims at the raw `a.aimPoint`; the sim has no aim point —
    // `aim_from(…, None)` (the weapon aim direction) is used (deviation).
    let dir = ProjectileSim::aim_from(a, from, None);
    st.cd = d.cooldown;
    let g = collision.raycast(from, dir, d.range, true);
    let Some(g) = g.hit.then_some(g) else {
        // JS L1799: the miss line is a visual; only the record rides the wire.
        let to = from + dir * d.range;
        let mut rec = vec![KitVal::Num(4.0), KitVal::Str("zm".to_string())];
        rec.extend(crate::special::world::v3(to).into_iter().map(KitVal::Num));
        a.sp_records.push(rec);
        return false;
    };
    let n = g.normal;
    let wall = n.y.abs() < WALL_NY;
    // JS L1801: `point + n * (PLAYER.radius + 0.08)`.
    let mut to = g.point + n * (t.radius + ANCHOR_PUSH);
    to.y = if wall { to.y - WALL_Y_DROP } else { g.point.y };
    st.zip = Some(ZipFlight {
        to,
        n,
        wall,
        anchor: g.point,
        t: 0.0,
        stuck: 0.0,
    });
    // JS L1805: `[4, 'zf', ...v3(anchor), r2(n.x), r2(n.y), r2(n.z)]`.
    let mut rec = vec![KitVal::Num(4.0), KitVal::Str("zf".to_string())];
    rec.extend(
        crate::special::world::v3(g.point)
            .into_iter()
            .map(KitVal::Num),
    );
    rec.push(KitVal::Num(crate::special::world::r2(n.x)));
    rec.push(KitVal::Num(crate::special::world::r2(n.y)));
    rec.push(KitVal::Num(crate::special::world::r2(n.z)));
    a.sp_records.push(rec);
    // JS L1807: latch paint (one seed draw, turf without gauge charge).
    let seed = rng.next_f32();
    let area = paint.splat(
        collision,
        g.point + n * LATCH_PUSH,
        LATCH_R,
        a.team,
        &SplatOpts {
            seed,
            stretch: None,
            stretch_amt: None,
            kind: None,
        },
    );
    a.add_turf_no_special(area);
    true
}

/// JS `IMPL.zipcaster.body` L1818-1826: the enemy slam scan. The match runs
/// this *before* the actor step (the body implementation has no roster
/// access); on a hit it clears the zip + facing, raises
/// [`ZipImpact::Slam`], and keeps `s.body` so this frame's body branch still
/// runs and releases the body (JS returns right after `impact`, L1824).
/// Returns the roster index of the slammed enemy.
#[must_use]
pub fn slam_scan(actors: &[Actor], a: &Actor, t: &PlayerTuning) -> Option<usize> {
    for (ei, e) in actors.iter().enumerate() {
        if e.team == a.team || !e.alive {
            continue; // JS L1819
        }
        let dx = e.pos.x - a.pos.x;
        let dz = e.pos.z - a.pos.z;
        if (dx * dx + dz * dz).sqrt() < t.radius * 2.0 + SLAM_XZ_BONUS
            && (e.pos.y - a.pos.y).abs() < SLAM_DY
        {
            return Some(ei); // JS L1820
        }
    }
    None
}

/// JS `IMPL.zipcaster.body` (L1811-1843): the zip pull. The enemy slam scan
/// (JS L1818-1826) runs in the match *before* the actor step (the body
/// implementation has no roster access); on a slam the match clears `zip`
/// and raises [`ZipImpact::Slam`], so this ladder sees `zip == None` and
/// just releases the body.
pub fn body(
    a: &mut Actor,
    s: &mut SpecialActive,
    dt: f32,
    world: &SimWorld,
    t: &PlayerTuning,
    input: &ActorInput,
) {
    let d = &crate::special::specials().zipcaster;
    let crate::special::KindState::Zipcaster(st) = &mut s.state else {
        return;
    };
    let Some(mut z) = st.zip else {
        s.body = false; // JS L1813
        return;
    };
    z.t += dt;
    let dv = z.to - a.pos;
    let dist = dv.length();
    let step = d.speed * dt;
    if dist <= step || z.t > FLIGHT_TIMEOUT || z.stuck > STUCK_MAX {
        // JS L1827-1833: arrival / timeout / stuck — stop, hang on walls,
        // raise the impact (the match drains it into the impact ladder).
        a.vel = Vec3::ZERO;
        if dist <= step {
            a.pos = z.to;
        }
        s.body = false;
        st.zip = None;
        s.face_yaw = None;
        if z.wall {
            s.hang = d.hang;
            s.hang_n = Some(z.n);
            a.grounded = false;
        }
        a.sp_zip_impact = Some(ZipImpact::Land(z.wall));
        return;
    }
    a.vel = dv * (d.speed / dist);
    let before = a.pos;
    let py = a.pos.y;
    a.pos += a.vel * dt;
    a.body_resolve(false, py, false, world, t);
    if a.pos.distance(before) < step * STUCK_FRAC {
        z.stuck += dt;
    } else {
        z.stuck = 0.0;
    }
    st.zip = Some(z);
    s.face_yaw = Some(a.vel.x.atan2(a.vel.z));
    // JS L1842: the body runs the tick ladder at the end of the frame.
    tick(a, s, dt, input);
}

/// JS `IMPL.zipcaster.tick` (L1864-1903): the hang release (jump or push
/// away from the wall). The aura / wisp / tether-line block (L1866-1878,
/// L1890-1902) is camera-gated presentation.
pub fn tick(a: &mut Actor, s: &mut SpecialActive, dt: f32, inp: &ActorInput) {
    let crate::special::KindState::Zipcaster(st) = &mut s.state else {
        return;
    };
    if st.zip.is_none() {
        s.face_yaw = None; // JS L1865
    }
    if s.hang > 0.0 {
        s.hang -= dt;
        // JS L1882-1888: jump (or push away from the wall) lets go.
        let mv = inp.move_dir;
        let away = s.hang_n.is_some_and(|n| mv.x * n.x + mv.z * n.z > AWAY_DOT);
        if inp.jump || away {
            s.hang = 0.0;
            let n = s.hang_n.unwrap_or(Vec3::Y);
            a.vel = vec3(n.x * PUSH_XZ, PUSH_Y, n.z * PUSH_XZ);
            a.grounded = false;
        }
    }
}

/// JS `IMPL.zipcaster.impact` (L1846-1863): the zipper mini-explosion — a
/// direct hit (the slammed enemy, or anyone within `impactDirectR`) takes
/// `impactDirect`, the rest of the blast `impactSplash`. The match drains
/// [`Actor::sp_zip_impact`] into this after the actor step.
#[allow(clippy::too_many_arguments)]
pub fn impact(
    actors: &mut [Actor],
    idx: usize,
    imp: ZipImpact,
    collision: &CollisionWorld,
    paint: &mut PaintGrid,
    rng: &mut Rng,
    ps: &mut ProjectileSim,
    subs: &mut SubSystem,
    remote: Option<&[bool]>,
    t: &PlayerTuning,
) {
    let d = &crate::special::specials().zipcaster;
    let (team, owner_slot) = (actors[idx].team, actors[idx].slot);
    let pos = actors[idx].pos;
    let c = vec3(pos.x, pos.y + IMPACT_UP, pos.z);
    let direct_ei = match imp {
        ZipImpact::Slam(ei) => Some(ei),
        ZipImpact::Land(_) => None,
    };
    // JS L1848: `[4, 'zi', ...v3(c), s.hang > 0 ? 1 : 0]` — `hang` is only
    // raised on a wall arrival, so the flag mirrors `z.wall` (a slam never
    // hangs).
    let flag = match imp {
        ZipImpact::Slam(_) => 0.0,
        ZipImpact::Land(w) => {
            if w {
                1.0
            } else {
                0.0
            }
        }
    };
    let mut rec = vec![KitVal::Num(4.0), KitVal::Str("zi".to_string())];
    rec.extend(crate::special::world::v3(c).into_iter().map(KitVal::Num));
    rec.push(KitVal::Num(flag));
    actors[idx].sp_records.push(rec);
    let mut hits: Vec<(usize, f32)> = Vec::new();
    for (ei, e) in actors.iter().enumerate() {
        if e.team == team || !e.alive {
            continue; // JS L1850
        }
        let v = vec3(e.pos.x, e.pos.y + VICTIM_UP, e.pos.z);
        let dist = v.distance(c);
        let hit_direct = Some(ei) == direct_ei || dist <= d.impact_direct_r;
        if !hit_direct && dist > d.impact_radius + SPLASH_MARGIN {
            continue; // JS L1853
        }
        if Some(ei) != direct_ei && !collision.los(c, v) {
            continue; // JS L1854
        }
        hits.push((
            ei,
            if hit_direct {
                d.impact_direct
            } else {
                d.impact_splash
            },
        ));
    }
    for (ei, dmg) in hits {
        // JS L1855 `applyHit(a, e, dmg, 'zipcaster')`: remote victims route
        // `'send'` like [`crate::special::slam_impact`].
        let (vs, vt) = (actors[ei].slot, actors[ei].team);
        let send = remote.is_some_and(|m| m.get(ei).copied().unwrap_or(false));
        if send {
            ps.push_event(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(&mut actors[ei], dmg, Some((team, owner_slot)), Some(pos), t);
            ps.push_event(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }
    subs.damage_area(c, d.impact_radius, d.impact_splash, team, false); // JS L1857
    // JS L1858-1859: one paint splat under the blast (one seed draw).
    let g = crate::special::world::ground_below(collision, c, IMPACT_PROBE);
    let base = g.unwrap_or(c);
    let seed = rng.next_f32();
    let area = paint.splat(
        collision,
        vec3(base.x, base.y + IMPACT_PAINT_LIFT, base.z),
        d.impact_radius * IMPACT_PAINT_FRAC,
        team,
        &SplatOpts {
            seed,
            stretch: None,
            stretch_amt: None,
            kind: None,
        },
    );
    actors[idx].add_turf_no_special(area);
}

/// JS `IMPL.zipcaster.end` (L1904): drop the cling (`s.hang = 0`). The
/// marker routing runs in [`crate::special::end`] (JS L373-375).
pub fn end(a: &mut Actor, _reason: &str) {
    if let Some(s) = a.special_active.as_mut() {
        s.hang = 0.0;
    }
}
