//! `IMPL.blower` (specials.js L1472-1509) — Bubble Blower: blow up to
//! `max` bubbles (a world-object each); shooting them pops the bubbles.
//! Batch 3: the inflation ladder in [`weapon`] (trigger-owning through
//! [`crate::special::weapon_hook`]) and the held-bubble release on [`end`]
//! (routed through [`crate::actor::Actor::sp_let`] — the release needs the
//! registry, which only the match holds). The JS `_swapWeapon` /
//! `_restoreWeapon` pair (L1473/L1507) only re-points the presentation
//! weapon model — the runner is skipped while the special owns the trigger
//! (recorded deviation). The inflate loop sound (L1479/L1489/L1493) is
//! presentation.

use glam::{Vec3, vec3};

use crate::actor::{Actor, SpecialActive};
use crate::special::specials;
use crate::special::{KindState, SpecialSystem, world};
use crate::weapon::{ProjectileSim, Rng};

/// JS L1490: the extra hold after full inflation before the bubble detaches
/// (`curT > inflate + 0.6`) — hardcoded in JS (D-T35-4).
const OVERINFLATE: f32 = 0.6;
/// JS L1487: the muzzle standoff along the aim (`r + 0.2`) — hardcoded
/// (D-T35-4).
const STANDOFF: f32 = 0.2;
/// JS L1488: the held bubble's floor lift (`a.pos.y + r * 0.7`) — hardcoded
/// (D-T35-4).
const FLOOR_LIFT: f32 = 0.7;
/// JS L1494: the tail after the last bubble (`dur = t + 0.6`) — hardcoded
/// (D-T35-4).
const DONE_TAIL: f32 = 0.6;

/// Kind state (JS `s.count`, `s.cur`, `s.curT`, `s.done`, `s.needRelease`,
/// L1473+).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.count` (L1473): bubbles released so far.
    pub count: f32,
    /// JS `s.cur` (L1473/1477): the world gid of the bubble being blown
    /// (`None` = nothing in hand). The object itself lives in the specials
    /// registry (`held`, muzzle-followed) from creation to release.
    pub cur: Option<u32>,
    /// JS `s.curT` (L1478/1482): seconds the current bubble has been blown.
    pub cur_t: f32,
    /// JS `s.done` (L1494): all bubbles released (`dur = t + 0.6`).
    pub done: bool,
    /// JS `s.needRelease` (L1476/1491): the trigger must be re-pressed.
    pub need_release: bool,
}

/// JS `IMPL.blower.start` (L1473).
pub fn start(_a: &mut Actor, s: &mut SpecialActive) {
    s.aim_face = true;
    s.no_squid = true;
    s.state = KindState::Blower(State::default());
}

/// JS `IMPL.blower.weapon` (L1474-1498): the inflation ladder. A press with
/// an empty hand (and bubbles left) opens a fresh bubble in the registry;
/// while held it grows on the ease-out curve and tracks the muzzle; letting
/// go (or over-inflating) releases it forward. Always `true`: the special
/// owns the trigger.
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    sys: &mut SpecialSystem,
    rng: &mut Rng,
    dt: f32,
) -> (bool, Option<&'static str>) {
    let d = &specials().blower;
    let Some(st) = (match &mut s.state {
        KindState::Blower(st) => Some(st),
        _ => None,
    }) else {
        return (true, None);
    };
    let f = vec3(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
    if !a.fire_gate.fire {
        st.need_release = false; // JS L1476
    }
    // JS L1477-1478: a fresh bubble — one `ph` draw (`make_bubble`), pushed
    // into the registry with a gid but *no record yet* (the record rides the
    // release, L1502).
    if a.fire_gate.fire
        && st.cur.is_none()
        && st.count < d.max as f32
        && !st.done
        && !st.need_release
    {
        let mut obj = world::make_bubble(a.slot, a.team, false, rng);
        let gid = sys.next_gid(a.slot);
        obj.gid = gid;
        sys.world.push(obj);
        st.cur = Some(gid);
        st.cur_t = 0.0;
    }
    if let Some(gid) = st.cur {
        st.cur_t += dt;
        s.firing = 0.3; // JS L1482 (set, not decayed — the blow pose)
        // JS L1483-1484: `k = clamp(curT/inflate, 0, 1)`,
        // `r = lerp(rMin, rMax, 1 - (1-k)²)` (ease-out).
        let k = (st.cur_t / d.inflate).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - k) * (1.0 - k);
        let r = d.r_min + (d.r_max - d.r_min) * ease;
        // JS L1485-1488: the held bubble rides the muzzle.
        let m = ProjectileSim::muzzle(a);
        let mut pos = m + f * (r + STANDOFF);
        pos.y = pos.y.max(a.pos.y + r * FLOOR_LIFT);
        if let Some(wi) = sys.world.iter().position(|w| !w.ghost && w.gid == gid) {
            sys.world[wi].pos = pos;
            if let world::ObjectBody::Bubble(b) = &mut sys.world[wi].body {
                b.r = r;
            }
        }
        // JS L1490-1495: release on trigger-up or over-inflation.
        if !a.fire_gate.fire || st.cur_t > d.inflate + OVERINFLATE {
            if a.fire_gate.fire {
                st.need_release = true;
            }
            // JS L1492: `let` runs on the just-updated `b.pos` / `b.r`.
            blow_let(a, sys, gid, f, pos, r);
            st.cur = None;
            st.count += 1.0;
            if st.count >= d.max as f32 {
                st.done = true;
                s.dur = s.t + DONE_TAIL; // JS L1494
            }
        }
    }
    (true, None) // JS L1497
}

/// JS `IMPL.blower.let(a, b, f)` (L1500-1503): release the bubble forward
/// and record `[2, 'bu', gid, ...v3(pos), r2(r), ...v3(f)]` — the other
/// screens build it already blown.
fn blow_let(a: &mut Actor, sys: &mut SpecialSystem, gid: u32, f: Vec3, pos: Vec3, r: f32) {
    if let Some(wi) = sys.world.iter().position(|w| !w.ghost && w.gid == gid)
        && let world::ObjectBody::Bubble(b) = &mut sys.world[wi].body
    {
        world::bubble_release(b, f);
    }
    if gid != 0 {
        let mut rec = vec![
            crate::net::KitVal::Num(2.0),
            crate::net::KitVal::Str("bu".to_string()),
            crate::net::KitVal::Num(gid as f32),
        ];
        rec.extend(world::v3(pos).into_iter().map(crate::net::KitVal::Num));
        rec.push(crate::net::KitVal::Num(world::r2(r)));
        rec.extend(world::v3(f).into_iter().map(crate::net::KitVal::Num));
        a.sp_records.push(rec);
    }
}

/// JS `IMPL.blower` has no `tick` (the inflation runs in `weapon`).
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.blower.end` (L1504-1508): `if (s.cur) let(a, s.cur, _fwd(a))` —
/// the release needs the registry, so the match drains [`Actor::sp_let`]
/// right after the special tick and runs [`blow_let`]'s body. The
/// `_restoreWeapon` half is presentation (module docs).
pub fn end(a: &mut Actor, _reason: &str) {
    let cur = a.special_active.as_ref().and_then(|s| match &s.state {
        KindState::Blower(st) => st.cur,
        _ => None,
    });
    if let Some(gid) = cur {
        let f = vec3(a.aim_yaw.sin(), 0.0, a.aim_yaw.cos());
        a.sp_let = Some((gid, f));
    }
}

/// The match-side drain of [`end`]'s held bubble (JS `IMPL.blower.let`,
/// L1500-1503): release the registry bubble and record it.
pub fn drain_let(a: &mut Actor, sys: &mut SpecialSystem) {
    let Some((gid, f)) = a.sp_let.take() else {
        return;
    };
    let Some(wi) = sys
        .world
        .iter()
        .position(|w| !w.ghost && w.gid == gid && w.kind == "bu")
    else {
        return;
    };
    let (pos, r) = match &sys.world[wi].body {
        world::ObjectBody::Bubble(b) => (sys.world[wi].pos, b.r),
        _ => return,
    };
    blow_let(a, sys, gid, f, pos, r);
}
