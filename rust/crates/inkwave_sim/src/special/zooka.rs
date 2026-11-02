//! `IMPL.zooka` (specials.js L1330-1348) — Twister Zooka: the main weapon
//! is replaced by a twister launcher firing every `interval` s. Batch 3:
//! the volley ladder in [`weapon`] (trigger-owning through
//! [`crate::special::weapon_hook`]). The JS `_swapWeapon` / `_restoreWeapon`
//! pair (L1331/L1347) only re-points the *presentation* weapon model — the
//! runner's own update is skipped every frame while the special owns the
//! trigger (L373), so the sim never sees a gameplay difference (recorded
//! deviation).

use crate::actor::{Actor, SpecialActive};
use crate::special::{KindState, SpecialSystem, spawn, world};
use crate::weapon::ProjectileSim;

/// JS L1331: the initial fire cooldown (`s.cd = 0.25`) — hardcoded in JS,
/// not tuning (D-T35-4).
const START_CD: f32 = 0.25;
/// JS L1331: the walk speed override (`s.speed = 4.2`) — hardcoded (D-T35-4).
const SPEED: f32 = 4.2;
/// JS L1335: the fire-pose timer (`s.firing = 0.45`) — hardcoded (D-T35-4).
const FIRING: f32 = 0.45;

/// Kind state (JS `s.cd`, L1331).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    /// JS `s.cd` (L1331): seconds until the next twister.
    pub cd: f32,
}

/// JS `IMPL.zooka.start` (L1331).
pub fn start(_a: &mut Actor, s: &mut SpecialActive) {
    s.speed = Some(SPEED); // JS L1331 `s.speed = 4.2` (D-T35-4)
    s.aim_face = true;
    s.no_squid = true;
    s.state = KindState::Zooka(State { cd: START_CD });
}

/// JS `IMPL.zooka.weapon` (L1332-1346): the cooldown decays every frame; a
/// held trigger at `cd <= 0` launches a twister from the muzzle along the
/// aim and re-arms the cooldown. Always `true` — the special owns the
/// trigger (the runner is skipped). The `zooka_fire` sound, the recoil emit,
/// the rumble and the `weapon:fire` event (L1339-1343) are presentation.
pub fn weapon(
    a: &mut Actor,
    s: &mut SpecialActive,
    sys: &mut SpecialSystem,
    dt: f32,
) -> (bool, Option<&'static str>) {
    let Some(st) = (match &mut s.state {
        KindState::Zooka(st) => Some(st),
        _ => None,
    }) else {
        return (true, None);
    };
    st.cd -= dt; // JS L1333 (decays even when nothing fires)
    if a.fire_gate.fire && st.cd <= 0.0 {
        st.cd = crate::special::specials().zooka.interval; // JS L1335
        s.firing = FIRING;
        // JS L1336-1337: the muzzle + aim line from the projectile system.
        let m = ProjectileSim::muzzle(a);
        let dir = ProjectileSim::aim_from(a, m, None);
        // JS L1338: `new Twister(this, a, m, dir)` recorded as
        // `[2, 'tw', gid, ...v3(m), ...v3(dir)]`.
        let obj = world::make_twister(a.slot, a.team, false, m, dir);
        let data = world::v3(m)
            .into_iter()
            .chain(world::v3(dir))
            .collect::<Vec<_>>();
        spawn(a, sys, obj, &data);
    }
    (true, None) // JS L1345
}

/// JS `IMPL.zooka` has no `tick` (the volleys run in `weapon`).
pub fn tick(_s: &mut SpecialActive, _dt: f32) {}

/// JS `IMPL.zooka.end` (L1347): `this._restoreWeapon(a)` — presentation only
/// in the sim (module docs).
pub fn end(_a: &mut Actor, _reason: &str) {}
