//! `IMPL.booyah` (specials.js L1717-1769) — Cheer Orb: hold the orb, the
//! charge fills over `charge` s (teammate cheers add `cheered` through
//! [`crate::special::cheer`]), throw once full. Batch 3: the charge ladder
//! and the throw (trigger-owning through [`crate::special::weapon_hook`]).
//! The ball / halo meshes (L1722-1725), the charge loop sound and the
//! `raise` arm pose (L1737) are presentation.

use glam::vec3;

use crate::actor::{Actor, SpecialActive};
use crate::special::specials;
use crate::special::{KindState, SpecialSystem, spawn, world};
use crate::weapon::throw_velocity;

/// JS L1732: the minimum hold before a click throws (`fullT > 0.05`) —
/// hardcoded in JS (D-T35-4).
const THROW_MIN: f32 = 0.05;
/// JS L1755: the throw origin lift (`pos.y + 1.35`) — hardcoded (D-T35-4).
const THROW_LIFT: f32 = 1.35;

/// Kind state (JS `s.charge`, `s.fullT`, `s.thrown`, `s.cheered`,
/// `s.netCharge`, L1720 / L2135).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.charge` (L1720): 0..1 orb charge.
    pub charge: f32,
    /// JS `s.fullT` (L1720): seconds spent fully charged.
    pub full_t: f32,
    /// JS `s.thrown` (L1720): the orb is away (the special is over).
    pub thrown: bool,
    /// JS `s.cheered` (L1720): cheer-pulse decay timer.
    pub cheered: f32,
    /// JS `s.netCharge` (L2135): the ghost side takes the owner's charge
    /// from the `spst` channel instead of the timer.
    pub net_charge: bool,
}

/// JS `IMPL.booyah.start` (L1718-1726).
pub fn start(_a: &mut Actor, s: &mut SpecialActive) {
    s.speed = Some(specials().booyah.move_speed);
    s.no_squid = true;
    s.aim_face = true;
    s.state = KindState::Booyah(State::default());
    // JS L1720: `bomb: {kind: 'booyah', throwSpeed}` — the throw-arc preview
    // is presentation (the orb itself rides [`weapon`]).
}

/// JS `IMPL.booyah.weapon` (L1727-1739): the charge fills at `1/charge` per
/// second (cheers add through [`crate::special::cheer`]); once full, a click
/// after 0.05 s — or the `autoThrow` cap — throws the orb and ends with
/// `'throw'`. Always `true`: the special owns the trigger.
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    sys: &mut SpecialSystem,
    dt: f32,
) -> (bool, Option<&'static str>) {
    let Some(st) = (match &mut s.state {
        KindState::Booyah(st) => Some(st),
        _ => None,
    }) else {
        return (true, None);
    };
    let d = &specials().booyah;
    st.charge = (st.charge + dt / d.charge).min(1.0); // JS L1729
    if st.charge >= 1.0 {
        st.full_t += dt; // JS L1731
        if (a.fire_gate.pressed && st.full_t > THROW_MIN) || st.full_t > d.auto_throw {
            // JS L1753-1761 (`throwIt`): the orb leaves on the shared lob
            // arc at `throwSpeed`.
            st.thrown = true;
            let from = vec3(a.pos.x, a.pos.y + THROW_LIFT, a.pos.z);
            let vel = throw_velocity(a, d.throw_speed);
            // Record: `[2, 'or', gid, ...v3(from), ...v3(vel)]`.
            let obj = world::make_orb(a.slot, a.team, false, from, vel);
            let data = world::v3(from)
                .into_iter()
                .chain(world::v3(vel))
                .collect::<Vec<_>>();
            spawn(a, sys, obj, &data);
            // JS L1761: `this.end(a, 'throw')` — run after the put-back.
            return (true, Some("throw"));
        }
    }
    (true, None) // JS L1738
}

/// JS `IMPL.booyah.tick` (L1740-1752): ball visuals + `s.cheered` decay.
pub fn tick(s: &mut SpecialActive, dt: f32) {
    if let KindState::Booyah(st) = &mut s.state {
        st.cheered = (st.cheered - dt).max(0.0); // JS L1746
    }
    // The cheer-credit ladder (JS `SpecialSystem.cheer`, L510-531) runs in
    // [`crate::special::cheer`] from the match's per-actor `intent.cheer`
    // check (L535).
}

/// JS `IMPL.booyah.end` (L1763-1768): mesh removal + the sound stop are
/// presentation.
pub fn end(_a: &mut Actor, _reason: &str) {}
