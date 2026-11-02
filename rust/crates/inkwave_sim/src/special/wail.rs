//! `IMPL.wail` (specials.js L1351-1407) — Howl Box: hold the speaker up
//! (walk at `holdSpeed`), click to set it down; the speaker world object
//! does the blasting. Batch 3: the bot auto-place timer (`autoT`, L1364) and
//! the placement ladder (L1367-1399). The held prop / guide beam (L1355-1363,
//! L1376-1389) and the end cleanup (L1400-1406) are presentation.

use glam::{Vec3, vec3};

use crate::actor::{Actor, SpecialActive};
use crate::collision::CollisionWorld;
use crate::special::specials;
use crate::special::{KindState, SpecialSystem, spawn, world};
use crate::weapon::Rng;

/// Kind state (JS `s.autoT`, L1364: the bot auto-place timer).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.autoT` (L1364): `None` = human (the fire-press ladder ends it);
    /// the sim draws it for every actor in [`begin`] (no local player).
    pub auto_t: Option<f32>,
}

/// JS `IMPL.wail.start` (L1353-1365): `Object.assign(s, { speed:
/// s.def.holdSpeed, noSquid: true, aimFace: true, dur: 0 })`. The bot
/// `autoT` draw is the roster-side half — [`begin`] (the ghost start runs
/// this same arm through `start_ghost`, and JS's `GHOST.wail.start` *calls*
/// `IMPL.wail.start`, L2053 — but the `a.bot` gate is false on a proxy, so
/// the ghost never gets an `autoT`; the sim keeps the draw in `begin`, which
/// only the live `special:use` peek runs).
pub fn start(_a: &mut Actor, s: &mut SpecialActive) {
    s.speed = Some(specials().wail.hold_speed);
    s.no_squid = true;
    s.aim_face = true;
    s.dur = 0.0;
    s.state = KindState::Wail(State { auto_t: None });
}

/// JS `IMPL.wail.start` L1364: `if (a.bot) s.autoT = 0.45 + Math.random() *
/// 0.5` — run by the match's `special:use` peek for live actors (every sim
/// actor is bot-style).
pub fn begin(a: &mut Actor, rng: &mut Rng) {
    let Some(s) = a.special_active.as_mut() else {
        return;
    };
    if let Some(st) = match &mut s.state {
        KindState::Wail(st) => Some(st),
        _ => None,
    } {
        st.auto_t = Some(0.45 + rng.next_f32() * 0.5);
    }
}

/// JS `IMPL.wail.dirOf` (L1372-1375): the aim direction with the pitch
/// clamped to `±0.3` rad (the clamp is hardcoded in JS, D-T35-4).
#[must_use]
pub fn dir_of(a: &Actor) -> Vec3 {
    let p = a.aim_pitch.clamp(-0.3, 0.3);
    vec3(
        a.aim_yaw.sin() * p.cos(),
        p.sin(),
        a.aim_yaw.cos() * p.cos(),
    )
}

/// JS `IMPL.wail.weapon` (L1367-1371): the hold pose timer runs every frame;
/// a click after 0.2 s (or the bot timer, or the `holdTime` cap) sets the
/// speaker down — [`place`]. Always `true`: the special owns the trigger.
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    sys: &mut SpecialSystem,
    collision: &CollisionWorld,
    dt: f32,
) -> (bool, Option<&'static str>) {
    let _ = dt;
    s.firing = 0.2; // JS L1368 (set, not decayed — the hold pose)
    let Some(st) = (match &s.state {
        KindState::Wail(st) => Some(st.clone()),
        _ => None,
    }) else {
        return (true, None);
    };
    let d = &specials().wail;
    // JS L1369: `(inp.firePressed && s.t > 0.2) || (s.autoT != null &&
    // s.t >= s.autoT) || s.t >= s.def.holdTime`.
    let place = (a.fire_gate.pressed && s.t > 0.2)
        || st.auto_t.is_some_and(|at| s.t >= at)
        || s.t >= d.hold_time;
    if place {
        place_speaker(a, sys, collision);
        return (true, Some("place")); // JS L1398: `this.end(a, 'place')`
    }
    (true, None)
}

/// JS `IMPL.wail.place` (L1390-1399): snap 1.3 m ahead of the actor to the
/// ground below (a down raycast from `pos.y + 1.2`, 4 m, grates skipped —
/// only a fairly flat surface, `normal.y > 0.6`, takes the box), spawn the
/// speaker aimed along [`dir_of`]. The `throw` trigger (L1395) is
/// presentation.
fn place_speaker(a: &mut Actor, sys: &mut SpecialSystem, collision: &CollisionWorld) {
    let f = vec3(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
    let at = a.pos + f * 1.3;
    let g = collision.raycast(vec3(at.x, a.pos.y + 1.2, at.z), Vec3::NEG_Y, 4.0, true);
    let pos = if g.hit && g.normal.y > 0.6 {
        g.point
    } else {
        a.pos
    };
    let dir = dir_of(a);
    // JS L1397: `[2, 'sk', gid, ...v3(pos), ...v3(dir)]`.
    let obj = world::make_speaker(a.slot, a.team, false, pos, dir);
    let data = world::v3(pos)
        .into_iter()
        .chain(world::v3(dir))
        .collect::<Vec<_>>();
    spawn(a, sys, obj, &data);
}

/// JS `IMPL.wail.tick` (L1376-1389): the held-prop visuals only — out of
/// sim scope.
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.wail.end` (L1400-1406): visual cleanup only — out of sim scope.
pub fn end(_a: &mut Actor, _reason: &str) {}
