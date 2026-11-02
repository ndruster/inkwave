//! `IMPL.strike` (specials.js L1280-1327) — Vortex Strike: aim a spot for
//! `aimTime` s (or confirm early), then launch a missile and end. Batch 3:
//! the full ladder — the bounds-clamped initial target, the bot target
//! picker (`botTarget`, L1303-1326), the auto-confirm timer and the missile
//! launch (L1294-1301). The body half runs through [`crate::special::body`]
//! (which raises the `launch` flag); the launch itself needs the specials
//! registry + collision, so the match runs [`launch`] the same frame through
//! [`crate::actor::Actor::peek_strike_launch`].

use glam::{Vec3, vec3};

use crate::actor::{Actor, SpecialActive};
use crate::collision::CollisionWorld;
use crate::paint::PaintGrid;
use crate::special::specials;
use crate::special::{KindState, SpecialSystem, spawn, world};
use crate::weapon::Rng;

/// Kind state (JS `s.aiming`, `s.confirm`, `s.autoT`, `s.target`, L1282-1285
/// + the sim-side `launch` flag, L1292).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.aiming` (L1282).
    pub aiming: bool,
    /// JS `s.confirm` (L1282): the owner clicked to launch early.
    pub confirm: bool,
    /// JS `s.autoT` (L1285): the auto-confirm time (`0.7 + rand*0.6`, drawn
    /// by [`begin`]; the sim has no local player, so every actor takes the
    /// JS `!isLocal || bot` branch).
    pub auto_t: Option<f32>,
    /// JS `s.target` (L1284): the aim point on the `y = 0` plane.
    pub target: Vec3,
    /// Sim-side latch (JS `launch` runs inline at L1292): [`body`] raises it,
    /// the match consumes it in [`launch`] the same frame.
    pub launch: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            aiming: false,
            confirm: false,
            auto_t: None,
            target: Vec3::ZERO,
            launch: false,
        }
    }
}

/// JS `IMPL.strike.start` (L1281-1287): the flags + the raw aim point. The
/// bounds clamp, the bot target and the `autoT` draw are the roster-side
/// half — [`begin`], run by the match on the same-frame `special:use` peek
/// (the ghost start never reaches either: `GHOST.strike.start` (L2050) is
/// the throw trigger only, blocked by the empty arm in `start_ghost`).
pub fn start(a: &mut Actor, s: &mut SpecialActive) {
    s.body = true;
    s.dur = 0.0; // JS L1282: `s.dur = 0` (the aim time ladder ends it)
    let tx = a.pos.x + a.aim_yaw.sin() * 16.0;
    let tz = a.pos.z + a.aim_yaw.cos() * 16.0;
    s.state = KindState::Strike(State {
        aiming: true,
        confirm: false,
        auto_t: None,
        target: Vec3::new(tx, 0.0, tz),
        launch: false,
    });
}

/// JS `IMPL.strike.start` L1284-1285 (the roster-side half): the stage-bounds
/// clamp, then `botTarget` and the `autoT` draw. JS runs these inline in
/// `start`; the sim defers them to the match's `special:use` peek because
/// they need the collision bounds, the paint grid and the roster. RNG order
/// matches JS: the `botTarget` draws first, then the `autoT` draw (L1285).
pub fn begin(
    a_idx: usize,
    actors: &mut [Actor],
    sys: &SpecialSystem,
    collision: &CollisionWorld,
    paint: &PaintGrid,
    rng: &mut Rng,
) {
    let a = &actors[a_idx];
    let Some(s) = a.special_active.as_ref() else {
        return;
    };
    if !matches!(s.state, KindState::Strike(_)) {
        return;
    }
    // JS L1284: clamp the raw aim point into `bounds ± 2`.
    let b = &collision.bounds;
    let clamped = vec3(
        a.pos.x.clamp(b.min_x + 2.0, b.max_x - 2.0),
        0.0,
        a.pos.z.clamp(b.min_z + 2.0, b.max_z - 2.0),
    );
    let team = a.team;
    // JS L1285: `!a.isLocal || a.bot` — every sim actor takes the bot path.
    // Phase 1 computes the target from the immutable roster, phase 2 writes
    // it back through the actor's own mutable borrow.
    let target = bot_target(actors, team, clamped, sys, collision, paint, rng);
    let auto_t = 0.7 + rng.next_f32() * 0.6; // JS L1285 (after the draws above)
    let Some(s) = actors[a_idx].special_active.as_mut() else {
        return;
    };
    if let Some(st) = match &mut s.state {
        KindState::Strike(st) => Some(st),
        _ => None,
    } {
        st.target = target;
        st.auto_t = Some(auto_t);
    }
}

/// JS `IMPL.strike.botTarget` (L1303-1326): the thickest live enemy cluster
/// (a strict `>` ladder), else — when there is none or the 0.35 draw fires —
/// the best of 24 paint-sampled ground points. The JS `!best || random() <
/// 0.35` short-circuits the draw when no cluster exists; the sim mirrors it
/// exactly (RNG-stream fidelity). Returns the final `y = 0` target (JS keeps
/// `s.target` untouched when nothing was found, L1325).
#[allow(clippy::too_many_arguments)]
fn bot_target(
    actors: &[Actor],
    team: usize,
    fallback: Vec3,
    sys: &SpecialSystem,
    collision: &CollisionWorld,
    paint: &PaintGrid,
    rng: &mut Rng,
) -> Vec3 {
    let mut best: Option<Vec3> = None;
    let mut bs = 0i32;
    for e in actors.iter() {
        if e.team == team || !e.alive {
            continue; // JS L1306
        }
        let mut n = 1i32;
        for o in actors.iter() {
            // JS L1307: `o !== e && o.team === e.team && o.alive &&
            // o.pos.distanceTo(e.pos) < 5`.
            if std::ptr::eq(o, e) {
                continue;
            }
            if o.team == e.team && o.alive && (o.pos - e.pos).length() < 5.0 {
                n += 1;
            }
        }
        if n > bs {
            // JS L1308: strict `>` — the first actor of a tied cluster wins.
            bs = n;
            best = Some(e.pos);
        }
    }
    let b = &collision.bounds;
    // JS L1311: the `Math.random()` draw only runs when a cluster exists
    // (the `!best ||` short-circuit).
    let resample = match best {
        None => true,
        Some(_) => rng.next_f32() < 0.35,
    };
    if resample {
        let mut bv = -1.0f32;
        let mut found: Option<Vec3> = None;
        let nodes = &sys.nav_nodes;
        let has_nodes = !nodes.is_empty();
        for _k in 0..24 {
            // JS L1314-1318: a nav node costs one draw (`nodes[(rand*n)|0]`),
            // the bounds-box sampler two (x, then z).
            let (x, z) = if has_nodes {
                let n =
                    nodes[((rng.next_f32() * nodes.len() as f32) as usize).min(nodes.len() - 1)];
                (n.x, n.z)
            } else {
                (
                    {
                        let u = rng.next_f32();
                        (b.min_x + 3.0) + ((b.max_x - 3.0) - (b.min_x + 3.0)) * u
                    },
                    {
                        let u = rng.next_f32();
                        (b.min_z + 3.0) + ((b.max_z - 3.0) - (b.min_z + 3.0)) * u
                    },
                )
            };
            let y = collision.ground_height(x, z, 30.0, false);
            if y == f32::NEG_INFINITY {
                continue; // JS L1319: open water (no ground within 30 m)
            }
            let s = paint.region_stats(collision, x, y, z, 4.0, team);
            // JS L1321: `st.n ? st.enemy * 1.5 + st.empty : -1`.
            let v = if s.n > 0 {
                s.enemy * 1.5 + s.empty
            } else {
                -1.0
            };
            if v > bv {
                // JS L1322: strict `>` — the first sample of a tied score wins.
                bv = v;
                found = Some(vec3(x, y, z));
            }
        }
        best = found; // JS replaces `best` wholesale (L1311-1324)
    }
    match best {
        Some(p) => vec3(p.x, 0.0, p.z), // JS L1325
        None => fallback,               // JS keeps `s.target` (L1325 guard)
    }
}

/// JS `IMPL.strike.body` (L1288-1293): the `_stand` physics runs in
/// [`crate::special::body`]; here the auto-confirm timer and the launch
/// gate. The launch itself is deferred to the match (it needs the registry +
/// collision) through the `launch` flag.
pub fn body(s: &mut SpecialActive, _dt: f32) {
    let Some(st) = (match &mut s.state {
        KindState::Strike(st) => Some(st),
        _ => None,
    }) else {
        return;
    };
    if !st.aiming {
        return; // JS L1290
    }
    if let Some(at) = st.auto_t
        && s.t >= at
    {
        st.confirm = true; // JS L1291
    }
    if st.confirm || s.t >= specials().strike.aim_time {
        st.launch = true; // JS L1292 → `IMPL.strike.launch`
    }
}

/// JS `IMPL.strike.launch` (L1294-1301): drop the aim, raycast the target
/// column down from `y = 40` (80 m, grates skipped), spawn the missile at
/// the ground hit (or the `y = 0` plane) and end with `'launch'`. The
/// `special:launch` emit (L1299) is presentation.
pub fn launch(
    a_idx: usize,
    actors: &mut [Actor],
    sys: &mut SpecialSystem,
    collision: &CollisionWorld,
) {
    let a = &mut actors[a_idx];
    let Some(s) = a.special_active.as_mut() else {
        return;
    };
    let Some(st) = (match &mut s.state {
        KindState::Strike(st) => Some(st),
        _ => None,
    }) else {
        return;
    };
    if !st.launch {
        return;
    }
    st.launch = false;
    st.aiming = false; // JS L1295
    let target = st.target;
    let (slot, team, from) = (a.slot, a.team, a.pos + vec3(0.0, 1.6, 0.0));
    // JS L1296: `raycast((tx, 40, tz), DOWN, 80, skipGrates = true)`.
    let g = collision.raycast(vec3(target.x, 40.0, target.z), Vec3::NEG_Y, 80.0, true);
    let to = if g.hit {
        g.point
    } else {
        vec3(target.x, 0.0, target.z)
    };
    // JS L1298: `new Missile(this, a, to)` — the ctor derives `from` from the
    // actor (`a.pos + 1.6y`, L779); the record carries `v3(to)`.
    let obj = world::make_missile(slot, team, false, from, to);
    spawn(a, sys, obj, &world::v3(to));
    crate::special::end(a, "launch"); // JS L1300
}

/// JS `IMPL.strike` has no `tick`.
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.strike` has no `end` arm.
pub fn end(_a: &mut Actor, _reason: &str) {}
