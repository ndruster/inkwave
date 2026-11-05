//! Sub weapons — every kind except the Splat Bomb (which lives in
//! [`crate::weapon`]'s bomb list): thrown and placed devices, clouds,
//! curtains, sprinklers, mines and jump beacons, plus the status effects
//! they put on players (tracking, poison).
//!
//! Faithful port of `src/game/subs.js` (`SubSystem`, 693 lines), batch one
//! of Task 34: the ten non-kit kinds (`bomb` rides [`crate::weapon`] —
//! `subs.js` never deploys it; `sticky` / `burst` / `seeker` / `scan` /
//! `curtain` / `sprinkler` / `mine` / `beacon` / `mist`). The five kit subs
//! (`shaker` / `waddle` / `torpedo` / `tracer` / `boomerang`) are batch two
//! (spec §5): [`SubKind::is_kit`] kinds deploy nothing here (D-T34-9).
//!
//! Keep in lockstep with upstream on sync:
//!   - `use` (L71-79) / `_throw` (L81-94) / `_place` (L96-105) / `_plant`
//!     (L107-119)               -> [`SubSystem::use_sub`]
//!   - `update` (L155-169) / `_step` (L170-184)  -> [`SubSystem::update`]
//!   - `netGhost` (L192-212)    -> [`SubSystem::net_ghost`]
//!   - `netHurt` (L214-217)     -> [`SubSystem::net_hurt`]
//!   - `_fly` (L219-258) / `_stick` (L260-276) / `_stuck` (L279-289)
//!     -> the `fly` / `stuck` state handlers
//!   - `_startRun` (L292-299) / `_run` (L300-351) -> the `run` state handler
//!   - `_startCloud` (L354-363) / `_cloud` (L365-378) -> `cloud`
//!   - `_startMist` (L381-399) / `_mist` (L400-414)   -> `mist`
//!   - `_startCurtain` (L417-436) / `_curtain` (L437-445) / `_spray`
//!     (L448-470) -> `curtain` / `spray`
//!   - `_mine` (L473-496) / `_mineBlast` (L497-512) -> `mine`
//!   - `_beacon` (L515-518) -> `beacon` (a no-op in the sim: the spin is
//!     presentation; the super-jump interaction is D-T34-2)
//!   - `track` (L537-542) / `poison` (L543-547)     -> [`Ctx::track`]
//!   - `_blast` (L550-567) / `_pelletBlast` (L568-583) -> [`Ctx::blast`]
//!   - `_paint` (L584-591) / `_paintUnder` (L592) / `_credit` (L594)
//!     -> [`Ctx::paint_at`] / [`Ctx::paint_under`] / [`Ctx::credit`]
//!   - `_hurt` (L597-607)       -> [`Ctx::hurt`]
//!   - `blockShot` (L609-632)   -> [`SubSystem::block_shot`]
//!   - `blockRay` (L633-649)    -> [`SubSystem::block_ray`]
//!   - `damageArea` (L650-658)  -> [`SubSystem::damage_area`]
//!   - `blockActor` (L660-672)  -> [`SubSystem::block_actor`]
//!   - `_destroy` (L674-683)    -> [`Ctx::destroy`] (mesh disposal is
//!     presentation-only)
//!
//! Presentation-only branches are dropped per the M3 range rule (spec §0):
//! prop meshes, the curtain shader, audio, `G.fx`, `nearCam`, rumble and the
//! `character.trigger` / non-forwarded `emit`s (`sub:use` rides
//! [`SimEvent::SubUse`] for the local client only; `sub:land` / `sub:cloud`
//! / `sub:arm` / `sub:destroyed` / `sub:cantuse` / `bomb:explode` / `shake`
//! / `actor:tracked` / `actor:poisoned` are not in the netmatch `FORWARD`
//! list (netmatch.js L47) and never reach the sim).
//!
//! Deviations from JS (recorded for review):
//!   - D-T34-2: the beacon super-jump interaction (`beaconsFor` /
//!     `jumpToBeacon` / `_landedOnBeacon`, L520-534) is not ported — the sim
//!     has no super jump; `uses` / `hp` / the per-player cap / shootability
//!     are.
//!   - D-T34-3 (retired in Task 35 batch 2): `it.sp` is now the real
//!     `!!owner.specialActive` stamp (subs.js L86/L113); `_credit` routes
//!     special-deployed ink through `addTurfNoSpecial` (L594).
//!   - D-T34-5: `gid` is `owner_slot * 100_000 + off` with `off` from the
//!     subsystem's own counter (JS `netId` uses the owner's `nid`,
//!     registry.js L64-65); the replication layer rewrites the `off` into a
//!     wire `nid`-based gid, so ghost records key on `(src_nid, gid)` and
//!     `net_hurt` on the owner-side local gid.
//!   - D-T34-8: the sprinkler / beacon shot-blocking capsule height is the
//!     constant [`DEVICE_H`] (JS `mesh.userData.lift * 2` needs the prop
//!     model, L626).
//!   - D-T34-9: kit sub kinds deploy nothing locally and no ghost is
//!     created (batch two).
//!   - The `sp` / `beepT` / `mesh` / `cloud` / `sheet` / `face` / `u` / `v`
//!     fields of JS items are presentation-only (mine visibility reads
//!     `face/u/v`, L481) and dropped.
//!   - RNG: the sim's draws follow the JS *order within each system's own
//!     stream (the determinism rule); presentation-only draws (prop `spinV`,
//!     the mist puff phases, L90/L388) are not consumed. A muted (ghost)
//!     step still consumes every draw — JS evaluates the `splat` arguments
//!     before `paint.js` L470 returns 0 under mute.

// Task 34 batch two: the five kit subs (`SUB_KITS`, kits/registry.js).
pub mod boomerang;
pub mod shaker;
pub mod torpedo;
pub mod tracer;
pub mod waddle;

use glam::{Vec3, vec3};

use crate::actor::{Actor, Form};
use crate::collision::CollisionWorld;
use crate::paint::{PaintGrid, SplatOpts};
use crate::tuning::{PlayerTuning, SubSet};
use crate::weapon::{
    DropSpec, ProjectileSim, Rng, SimEvent, apply_hit, segment_capsule_dist, throw_velocity,
};

/// JS `UP` (subs.js L25).
const UP: Vec3 = vec3(0.0, 1.0, 0.0);
/// JS `DOWN` (L25).
const DOWN: Vec3 = vec3(0.0, -1.0, 0.0);
/// JS `GRAV` (L28): thrown-sub flight gravity.
const GRAV: f32 = 24.0;
/// Sprinkler / beacon blocking-capsule height (D-T34-8; JS
/// `mesh.userData.lift * 2` = the prop model's scaled height, L626).
pub const DEVICE_H: f32 = 0.95;
/// `Math.PI * 2`.
const TAU: f32 = std::f32::consts::TAU;

/// JS `lerp` (ctx.js).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// JS `clamp` (ctx.js).
fn clamp(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

/// JS `r2` (L22): the two-decimal quantisation the net records use.
fn r2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// The fifteen `SUBS` kinds (config.js L319-…): the ten batch-one devices
/// plus the five kit subs (batch two, [`SubKind::is_kit`]). JS keys the
/// item's `kind` by the `SUBS` object key; the wire carries the same string
/// (`['k', nid, 'subs', [op, gid, kind, …]]`, subs.js L77).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubKind {
    /// `bomb` — Splat Bomb (deployed through [`crate::weapon`], never here).
    #[default]
    Bomb,
    /// `sticky` — Cling Charge.
    Sticky,
    /// `burst` — Burst Bomb.
    Burst,
    /// `seeker` — Skitter Bomb.
    Seeker,
    /// `scan` — Echo Orb.
    Scan,
    /// `curtain` — Drip Curtain.
    Curtain,
    /// `sprinkler` — Twirl Sprinkler.
    Sprinkler,
    /// `mine` — Lurk Mine (placed).
    Mine,
    /// `beacon` — Hop Beacon (placed).
    Beacon,
    /// `mist` — Murk Bomb.
    Mist,
    /// `shaker` — kit (batch two).
    Shaker,
    /// `waddle` — kit (batch two).
    Waddle,
    /// `torpedo` — kit (batch two).
    Torpedo,
    /// `tracer` — kit (batch two).
    Tracer,
    /// `boomerang` — kit (batch two).
    Boomerang,
}

impl SubKind {
    /// JS `THROWN[d.kind]` (bots.js L36, `THROWN.waddle = true` waddle.js
    /// L768, `THROWN.torpedo = true` torpedo.js L750, `THROWN.shaker = true`
    /// shaker.js L618, `THROWN[KIND] = true` boomerang.js L594): the lobbed
    /// / stickable subs (`bomb` / `sticky` / `burst` / `mist` / `seeker` /
    /// `waddle` / `torpedo` / `shaker` / `boomerang`).
    #[must_use]
    pub fn is_thrown(self) -> bool {
        matches!(
            self,
            Self::Bomb
                | Self::Sticky
                | Self::Burst
                | Self::Mist
                | Self::Seeker
                | Self::Waddle
                | Self::Torpedo
                | Self::Shaker
                | Self::Boomerang
        )
    }

    /// The JS `SUBS` key / wire string for this kind.
    #[must_use]
    pub fn id_str(self) -> &'static str {
        match self {
            Self::Bomb => "bomb",
            Self::Sticky => "sticky",
            Self::Burst => "burst",
            Self::Seeker => "seeker",
            Self::Scan => "scan",
            Self::Curtain => "curtain",
            Self::Sprinkler => "sprinkler",
            Self::Mine => "mine",
            Self::Beacon => "beacon",
            Self::Mist => "mist",
            Self::Shaker => "shaker",
            Self::Waddle => "waddle",
            Self::Torpedo => "torpedo",
            Self::Tracer => "tracer",
            Self::Boomerang => "boomerang",
        }
    }

    /// Wire `kind` string → enum (`SUBS[d[2]]`, subs.js L205).
    #[must_use]
    pub fn from_id_str(s: &str) -> Option<Self> {
        Some(match s {
            "bomb" => Self::Bomb,
            "sticky" => Self::Sticky,
            "burst" => Self::Burst,
            "seeker" => Self::Seeker,
            "scan" => Self::Scan,
            "curtain" => Self::Curtain,
            "sprinkler" => Self::Sprinkler,
            "mine" => Self::Mine,
            "beacon" => Self::Beacon,
            "mist" => Self::Mist,
            "shaker" => Self::Shaker,
            "waddle" => Self::Waddle,
            "torpedo" => Self::Torpedo,
            "tracer" => Self::Tracer,
            "boomerang" => Self::Boomerang,
            _ => return None,
        })
    }

    /// The five `SUB_KITS` kinds (batch two, D-T34-9).
    #[must_use]
    pub fn is_kit(self) -> bool {
        matches!(
            self,
            Self::Shaker | Self::Waddle | Self::Torpedo | Self::Tracer | Self::Boomerang
        )
    }
}

/// JS `it.state` (the `_step` switch, subs.js L171-183). `dead` is the
/// removal sentinel the update loop splices on (L164-167).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubState {
    /// `fly` — a thrown device in flight.
    Fly,
    /// `stuck` — a sticky charge clinging to a surface, charging its blast.
    Stuck,
    /// `run` — a skitter bomb skittering after its target.
    Run,
    /// `cloud` — an echo orb's sensing cloud.
    Cloud,
    /// `mist` — a murk bomb's poison mist.
    Mist,
    /// `curtain` — a live drip curtain (solid: blocks shots and players).
    Curtain,
    /// `spray` — a sprinkler spraying pulses.
    Spray,
    /// `mine` — a planted lurk mine.
    Mine,
    /// `beacon` — a planted hop beacon.
    Beacon,
    /// `dead` — removed at the end of this update pass.
    Dead,
}

/// One live sub device (JS `it`, the duck-typed item objects of `subs.js`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubItem {
    /// JS `it.kind`.
    pub kind: SubKind,
    /// JS `it.owner` — resolved to the actor-list index (stable per match).
    pub owner_idx: usize,
    /// The owner's identity slot for events / records (JS `a.slot`).
    pub owner_slot: usize,
    /// The owner's team (JS `a.team`; `it.team` mirrors it).
    pub owner_team: usize,
    /// JS `it.team`.
    pub team: usize,
    /// JS `it.state`.
    pub state: SubState,
    /// JS `it.pos`.
    pub pos: Vec3,
    /// JS `it.vel` (flight only; the skitter walks `pos` directly).
    pub vel: Vec3,
    /// JS `it.age` — seconds since the throw (the air-fuse reads it, L238).
    pub age: f32,
    /// JS `it.t` — seconds in the current state.
    pub t: f32,
    /// JS `it.born` (`G.time` at planting; the per-player cap sorts by it,
    /// L100).
    pub born: f32,
    /// JS `it.ghost` — a remote player's copy (paint muted, hits forwarded).
    pub ghost: bool,
    /// JS `it.gid` (D-T34-5).
    pub gid: u32,
    /// The owner's wire `nid` for a ghost (the `(src_nid, gid)` dedup key;
    /// `None` for local items).
    pub src_nid: Option<u8>,
    /// JS `it.dir` — the horizontal throw direction (skitter heading, L295;
    /// curtain facing, L421).
    pub dir: Vec3,
    /// JS `it.normal` — the mount surface normal (`stuck` / `spray` /
    /// planted devices).
    pub normal: Option<Vec3>,
    /// JS `it.n` — the curtain plane normal (its facing).
    pub n: Option<Vec3>,
    /// JS `it.tan` — the curtain plane's horizontal tangent.
    pub tan: Option<Vec3>,
    /// JS `it.hp` (shootable devices; curtains fade instead of dying).
    pub hp: f32,
    /// JS `it.uses` (beacon; only decremented by the super jump, D-T34-2).
    pub uses: u32,
    /// JS `it.fuse` — the cling charge countdown (`stuck`) or the mine's
    /// triggered delay (`Some` once armed, L483/L261).
    pub fuse: Option<f32>,
    /// JS `it.heading` — the skitter's travel heading (L295).
    pub heading: f32,
    /// JS `it.trail` — trail-splat accumulator, metres (L343).
    pub trail: f32,
    /// JS `it.stuckT` — wall-grinding timer (L325).
    pub stuck_t: f32,
    /// JS `it.dash` — the skitter's committed straight-line dash flag (L314).
    pub dash: bool,
    /// JS `it.target` — the skitter's victim (actor index).
    pub target: Option<usize>,
    /// JS `it.tagged` — the cloud's already-tracked victims (actor indices).
    pub tagged: Vec<usize>,
    /// JS `it.direct` — the murk bomb's direct-hit victim (poisoned for the
    /// whole mist, L231/L396).
    pub direct: Option<usize>,
    /// JS `it.pulseT` — the sprinkler's next-pulse countdown (L450).
    pub pulse_t: f32,
    /// JS `it.spin` — the sprinkler's rotor angle (drives the drop fan,
    /// L451/L460).
    pub spin: f32,
    /// JS `it.sp` (subs.js L86/L113 `sp: !!a.specialActive`, stamped at
    /// throw / plant): a device deployed during a special never charges the
    /// owner's meter (`_credit` L594 → `addTurfNoSpecial`). Task 35 batch 2
    /// retires D-T34-3 (it was always false while specials were unported).
    #[serde(default)]
    pub sp: bool,
}

/// The sub-weapon world (JS `G.subs`, `SubSystem`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubSystem {
    /// JS `this.items`.
    pub items: Vec<SubItem>,
    events: Vec<SimEvent>,
    rng: Rng,
    /// The `gid` `off` counter (JS `_seq` behind `netId`, registry.js
    /// L64-65; D-T34-5).
    seq: u32,
    /// JS `kits/shaker.js` `items` (batch two).
    #[serde(default)]
    shaker: Vec<shaker::ShakerItem>,
    /// JS `kits/waddle.js` `items` (batch two).
    #[serde(default)]
    waddle: Vec<waddle::WaddleItem>,
    /// JS `kits/torpedo.js` `items` (batch two).
    #[serde(default)]
    torpedo: Vec<torpedo::TorpedoItem>,
    /// JS `kits/torpedo.js` `drops` (batch two).
    #[serde(default)]
    torpedo_drops: Vec<torpedo::TorpedoDrop>,
    /// JS `kits/torpedo.js` per-burst-group `rec` arrays (the shared
    /// per-foe droplet hit counters, indexed by `TorpedoDrop::rec_idx`).
    /// JS hangs each `rec` on the drops of one burst; the sim keeps them in
    /// one list (never pruned — bounded by the match's torpedo bursts).
    #[serde(default)]
    drop_recs: Vec<Vec<(usize, u32)>>,
    /// JS `kits/tracer.js` `beams` (batch two).
    #[serde(default)]
    tracer: Vec<tracer::TracerItem>,
    /// JS `kits/boomerang.js` `items` (batch two).
    #[serde(default)]
    boomerang: Vec<boomerang::BoomerangItem>,
}

/// Empty subsystem for `#[serde(default)]` on [`crate::match_::Match`].
/// The match constructor always rebuilds it from the seed with its own
/// salt; this default only covers deserialising pre-Task-34 snapshots.
impl Default for SubSystem {
    fn default() -> Self {
        Self::new(0)
    }
}

impl SubSystem {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            items: Vec::new(),
            events: Vec::new(),
            rng: Rng::new(seed),
            seq: 0,
            shaker: Vec::new(),
            waddle: Vec::new(),
            torpedo: Vec::new(),
            torpedo_drops: Vec::new(),
            drop_recs: Vec::new(),
            tracer: Vec::new(),
            boomerang: Vec::new(),
        }
    }

    /// Take the queued sim events (the match layer drains them into the
    /// net records / turf credit / presentation channels).
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    /// JS `clear()` (L64-68): drop every item, plus the batch-two kit lists
    /// (each kit module's own `clear` — the kit files fold it into their
    /// `tick` / the subsystem splice; the sim clears them here).
    pub fn clear(&mut self) {
        self.items.clear();
        self.shaker.clear();
        self.waddle.clear();
        self.torpedo.clear();
        self.torpedo_drops.clear();
        self.drop_recs.clear();
        self.tracer.clear();
        self.boomerang.clear();
    }

    /// Assemble the per-frame working context from the subsystem fields
    /// (the JS `G.subs` globals the state machine closes over).
    #[allow(clippy::too_many_arguments)]
    fn ctx<'a>(
        &'a mut self,
        world: &'a CollisionWorld,
        actors: &'a mut [Actor],
        paint: &'a mut PaintGrid,
        projectiles: &'a mut ProjectileSim,
        t: &'a PlayerTuning,
        sb: &'a SubSet,
        remote: Option<&'a [bool]>,
        time: f32,
    ) -> Ctx<'a> {
        Ctx {
            items: &mut self.items,
            events: &mut self.events,
            rng: &mut self.rng,
            actors,
            paint,
            world,
            projectiles,
            t,
            sb,
            remote,
            seq: &mut self.seq,
            time,
            shaker: &mut self.shaker,
            waddle: &mut self.waddle,
            torpedo: &mut self.torpedo,
            torpedo_drops: &mut self.torpedo_drops,
            drop_recs: &mut self.drop_recs,
            tracer: &mut self.tracer,
            boomerang: &mut self.boomerang,
        }
    }

    // ------------------------------------------------------------------ deploy
    /// JS `SK?.blocked?.(a, sub)` (weapons.js L145): the kit subs that
    /// refuse a second throw while one is still out (torpedo.js L733,
    /// boomerang.js L603). The match layer computes this before the runner
    /// update and feeds it to `WeaponRunner::update`.
    #[must_use]
    pub fn kit_blocked(&self, kind: SubKind, actor_idx: usize) -> bool {
        match kind {
            SubKind::Torpedo => torpedo::blocked(&self.torpedo, actor_idx),
            SubKind::Boomerang => boomerang::blocked(&self.boomerang, actor_idx),
            _ => false,
        }
    }

    /// JS `use(a, sub)` (L71-79): throw or place a sub after the runner has
    /// paid the ink. Kit kinds deploy through the batch-two kit modules
    /// (`SUB_KITS[kind].use`, kits/registry.js); the splat bomb never
    /// reaches here (the runner routes `bomb` to
    /// [`crate::weapon::ProjectileSim::throw_bomb`], weapons.js L152).
    /// `kit_level` is the shaker's charge level (1..=3, shaker.js L359);
    /// the runner reads it off its own hold state — other kinds ignore it.
    #[allow(clippy::too_many_arguments)]
    pub fn use_sub(
        &mut self,
        actor_idx: usize,
        kit_level: u32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        projectiles: &mut ProjectileSim,
        t: &PlayerTuning,
        sb: &SubSet,
        remote: Option<&[bool]>,
        time: f32,
    ) {
        // JS `use(a, sub)` receives the *effective* sub (weapons.js L133):
        // under a live Bomb Barrage the thrown kind is the barrage's bomb
        // variant, not the loadout (`effective_sub` mirrors the runner).
        let kind = crate::special::barrage::effective_sub(&actors[actor_idx]);
        if kind == SubKind::Bomb {
            return; // the bomb path never lands here
        }
        if kind.is_kit() {
            let mut ctx = self.ctx(world, actors, paint, projectiles, t, sb, remote, time);
            match kind {
                SubKind::Shaker => shaker::use_shaker(&mut ctx, actor_idx, kit_level),
                SubKind::Waddle => waddle::use_waddle(&mut ctx, actor_idx),
                SubKind::Torpedo => torpedo::use_torpedo(&mut ctx, actor_idx),
                SubKind::Tracer => tracer::use_tracer(&mut ctx, actor_idx),
                SubKind::Boomerang => boomerang::use_boomerang(&mut ctx, actor_idx),
                _ => {}
            }
            return;
        }
        // JS `use` (subs.js L73): `sub.placed` — only the mine and beacon
        // configs carry the flag; the thrown kinds have none (undefined →
        // falsy → the throw path).
        let placed = match kind {
            SubKind::Mine => sb.mine.placed,
            SubKind::Beacon => sb.beacon.placed,
            _ => false,
        };
        let mut ctx = self.ctx(world, actors, paint, projectiles, t, sb, remote, time);
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        if placed {
            // JS `_place` (L96-105): raycast the floor under the player.
            let origin = vec3(a.pos.x, a.pos.y + 0.4, a.pos.z);
            let g = ctx.world.raycast(origin, DOWN, 3.0, false);
            if !g.hit {
                return;
            }
            // roof / rail / perch refusals (L98): the sim's block records
            // carry no such flags — treat every hit surface as plantable
            // (same reading as the Task 33 cling note).
            // Copy the yaw before the limit loop: `a` borrows `ctx.actors`
            // immutably and the loop mutates `ctx`.
            let yaw = a.yaw;
            // per-player limits (L100-101): the oldest beyond `max` goes off
            // (mine) or is removed (beacon).
            let max = if kind == SubKind::Mine {
                sb.mine.max
            } else {
                sb.beacon.max
            };
            loop {
                let mut oldest: Option<(usize, f32)> = None;
                for (j, it) in ctx.items.iter().enumerate() {
                    if it.owner_slot != slot
                        || it.owner_team != team
                        || it.kind != kind
                        || it.state == SubState::Dead
                    {
                        continue;
                    }
                    if oldest.is_none_or(|(_, b)| it.born < b) {
                        oldest = Some((j, it.born));
                    }
                }
                let Some((j, _)) = oldest else { break };
                let count = ctx
                    .items
                    .iter()
                    .filter(|it| {
                        it.owner_slot == slot
                            && it.owner_team == team
                            && it.kind == kind
                            && it.state != SubState::Dead
                    })
                    .count();
                if count < max as usize {
                    break;
                }
                if kind == SubKind::Mine {
                    ctx.mine_blast(j, false);
                } else {
                    ctx.destroy(j);
                }
            }
            let pos = g.point;
            let n = g.normal;
            let gid = ctx.gid_next(slot);
            ctx.plant(kind, actor_idx, slot, team, pos, n, gid, None, false, time);
            // netRec(a, 'subs', [1, gid, kind, pos, normal, yaw]) (L103)
            ctx.events.push(SimEvent::SubSpawn {
                owner_slot: slot,
                team,
                sub_kind: kind.id_str().to_string(),
                data: vec![
                    1.0,
                    gid as f32,
                    0.0,
                    r2(pos.x),
                    r2(pos.y),
                    r2(pos.z),
                    r2(n.x),
                    r2(n.y),
                    r2(n.z),
                    r2(yaw),
                ],
            });
            ctx.events.push(SimEvent::SubUse {
                owner_slot: slot,
                team,
                kind,
            });
        } else {
            // JS `use` throw path (L75-78).
            let speed = throw_speed(kind, sb);
            let pos = vec3(a.pos.x, a.pos.y + 1.35, a.pos.z);
            let vel = throw_velocity(a, speed);
            let gid = ctx.gid_next(slot);
            ctx.throw_item(kind, actor_idx, slot, team, pos, vel, gid, None, false);
            ctx.events.push(SimEvent::SubSpawn {
                owner_slot: slot,
                team,
                sub_kind: kind.id_str().to_string(),
                data: vec![
                    0.0,
                    gid as f32,
                    0.0,
                    r2(pos.x),
                    r2(pos.y),
                    r2(pos.z),
                    r2(vel.x),
                    r2(vel.y),
                    r2(vel.z),
                ],
            });
            ctx.events.push(SimEvent::SubUse {
                owner_slot: slot,
                team,
                kind,
            });
        }
    }

    // ------------------------------------------------------------------ online
    /// JS `netGhost(a, d)` (L192-212): a remote player's throw / placement /
    /// end record. The netmatch replay wraps `'k'` records in `mute++`
    /// (netmatch.js L490-491), so the whole call runs muted: paint is
    /// dropped (the draws still happen — JS evaluates the args first) and
    /// blasts hurt nothing.
    /// `nids` is the roster's wire-nid list (replication layer): the kit
    /// ghosts translate the wire `nid` bytes in their lock / walk / beam
    /// records back into actor-list indices (D-T34-5 pattern).
    #[allow(clippy::too_many_arguments)]
    pub fn net_ghost(
        &mut self,
        actor_idx: usize,
        nid: u8,
        sub_kind: &str,
        data: &[f32],
        nids: &[u8],
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        projectiles: &mut ProjectileSim,
        t: &PlayerTuning,
        sb: &SubSet,
        remote: Option<&[bool]>,
        time: f32,
    ) {
        if data.len() < 2 {
            return;
        }
        let (op, gid) = (data[0], data[1] as u32);
        let mut ctx = self.ctx(world, actors, paint, projectiles, t, sb, remote, time);
        let Some(kind) = SubKind::from_id_str(sub_kind) else {
            return;
        };
        if kind == SubKind::Bomb {
            return; // the bomb rides `['b']`
        }
        if kind.is_kit() {
            // Batch two: the kit's own `ghost` handles every op (the kit
            // records carry no kind slot at `data[2]` — `['k', nid,
            // <kit>, [op, gid, …]]`, netmatch.js L488 + each kit module).
            match kind {
                SubKind::Shaker => shaker::ghost(&mut ctx, actor_idx, nid, data),
                SubKind::Waddle => waddle::ghost(&mut ctx, actor_idx, nid, data, nids),
                SubKind::Torpedo => torpedo::ghost(&mut ctx, actor_idx, nid, data, nids),
                SubKind::Tracer => tracer::ghost(&mut ctx, actor_idx, nid, data, nids),
                SubKind::Boomerang => boomerang::ghost(&mut ctx, actor_idx, nid, data),
                _ => {}
            }
            return;
        }
        // op 2: the owner ended the device (L195-203).
        if op == 2.0 {
            let idx = ctx.items.iter().position(|x| {
                x.ghost && x.src_nid == Some(nid) && x.gid == gid && x.state != SubState::Dead
            });
            let Some(i) = idx else { return };
            let state = ctx.items[i].state;
            if ctx.items[i].kind == SubKind::Mine {
                ctx.mine_blast(i, true);
            } else if state == SubState::Stuck || state == SubState::Run {
                let (rad, dmax, dmin, prad) = blast_params(ctx.items[i].kind, ctx.sb);
                let n = ctx.items[i].normal.unwrap_or(UP);
                let c = ctx.items[i].pos + vec3(0.0, 0.2, 0.0);
                ctx.blast(i, c, rad, dmax, dmin, prad, n, true);
            } else {
                ctx.destroy(i);
            }
            ctx.items[i].state = SubState::Dead;
            return;
        }
        if data.len() < 3 {
            return;
        }
        // duplicate / already-live gid → drop (L205)
        if ctx
            .items
            .iter()
            .any(|x| x.ghost && x.src_nid == Some(nid) && x.gid == gid)
        {
            return;
        }
        let a = &ctx.actors[actor_idx];
        let (slot, team) = (a.slot, a.team);
        if op == 0.0 && data.len() >= 9 {
            let pos = vec3(data[3], data[4], data[5]);
            let vel = vec3(data[6], data[7], data[8]);
            ctx.throw_item(kind, actor_idx, slot, team, pos, vel, gid, Some(nid), true);
        } else if op == 1.0 && data.len() >= 10 {
            let pos = vec3(data[3], data[4], data[5]);
            let n = vec3(data[6], data[7], data[8]);
            // the ghost's floor hit for the ink under it (L209-210): the
            // sim skips `face/u/v` (presentation-only), so only the ray's
            // existence would matter — it does not.
            ctx.plant(
                kind,
                actor_idx,
                slot,
                team,
                pos,
                n,
                gid,
                Some(nid),
                true,
                time,
            );
        }
    }

    /// JS `netHurt(gid, dmg)` (L214-217): the owner's side of a hit made on
    /// one of its devices on another screen (`gid` already converted to the
    /// owner-side local id by the replication layer). `kind` is the wire
    /// `'dh'` kind: `"subs"` for the batch-one devices, a kit id for the
    /// batch-two lists (netmatch.js L211-217: `SUB_KITS[kind].netHurt` /
    /// `KIT_GHOSTS.torpedoArea.netHurt`).
    pub fn net_hurt(&mut self, kind: &str, gid: u32, dmg: f32) {
        match kind {
            "waddle" => waddle::net_hurt(&mut self.waddle, &mut self.events, gid, dmg),
            "torpedo" => torpedo::net_hurt(&mut self.torpedo, &mut self.events, gid, dmg),
            "torpedoArea" => {
                torpedo::net_area_hurt(&mut self.torpedo, &mut self.events, gid, dmg);
            }
            _ => {
                let idx = self
                    .items
                    .iter()
                    .position(|x| !x.ghost && x.gid == gid && x.state != SubState::Dead);
                if let Some(i) = idx {
                    hurt_item(&mut self.items, &mut self.events, i, dmg);
                }
            }
        }
    }

    /// Test-only: the cross-module replication tests (net/replication.rs)
    /// need to observe the ghost kit lists without going through the sim's
    /// private module layout.
    #[cfg(test)]
    pub(crate) fn waddle_items(&self) -> &[waddle::WaddleItem] {
        &self.waddle
    }

    /// TR-47.1 test-only: mutable waddle access so a threat scenario can pin
    /// a locked walker without running the sub physics.
    #[cfg(test)]
    pub(crate) fn waddle_items_mut(&mut self) -> &mut Vec<waddle::WaddleItem> {
        &mut self.waddle
    }

    /// Test-only, see [`Self::waddle_items`].
    #[cfg(test)]
    pub(crate) fn torpedo_items(&self) -> &[torpedo::TorpedoItem] {
        &self.torpedo
    }

    /// Test-only, see [`Self::waddle_items`].
    #[cfg(test)]
    pub(crate) fn boomerang_items(&self) -> &[boomerang::BoomerangItem] {
        &self.boomerang
    }

    /// JS `'splatted'` listeners (subs.js L60 + kits/boomerang.js L418): a
    /// splatted owner loses their live sprinkler and fizzles their live
    /// boomerang. The match calls this right after the actor pass so the
    /// `[2, gid]` end record lands in the same frame.
    pub fn owner_splatted_destroy(&mut self, actors: &[Actor]) {
        for (ai, a) in actors.iter().enumerate() {
            if !a.has_splat_event() {
                continue;
            }
            for it in &mut self.items {
                if it.owner_idx == ai
                    && it.kind == SubKind::Sprinkler
                    && it.state == SubState::Spray
                {
                    it.state = SubState::Dead;
                }
            }
            boomerang::owner_splatted(&mut self.boomerang, ai);
        }
    }

    // ------------------------------------------------------------------ per frame
    /// JS `update(dt)` + `_step` (L155-184): the batch-two kit `tick` loop
    /// first (the `SUB_KITS` registration order — shaker, waddle, torpedo,
    /// tracer, boomerang; kits/index.js), then the reverse walk, the
    /// per-item ghost mute wrap (`gm = it.ghost && nm`, L159-162) and the
    /// dead splice with the `[2, gid]` end record (L164-167).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        world: &CollisionWorld,
        actors: &mut [Actor],
        paint: &mut PaintGrid,
        projectiles: &mut ProjectileSim,
        t: &PlayerTuning,
        sb: &SubSet,
        remote: Option<&[bool]>,
        time: f32,
    ) {
        let mut ctx = self.ctx(world, actors, paint, projectiles, t, sb, remote, time);
        shaker::tick(&mut ctx, dt);
        waddle::tick(&mut ctx, dt);
        torpedo::tick(&mut ctx, dt);
        tracer::tick(&mut ctx, dt);
        boomerang::tick(&mut ctx, dt);
        let mut i = ctx.items.len();
        while i > 0 {
            i -= 1;
            ctx.items[i].age += dt;
            ctx.items[i].t += dt;
            let muted = ctx.items[i].ghost;
            ctx.step_item(i, dt, muted);
            if ctx.items[i].state == SubState::Dead {
                if !ctx.items[i].ghost {
                    let (slot, team, gid) = (
                        ctx.items[i].owner_slot,
                        ctx.items[i].owner_team,
                        ctx.items[i].gid,
                    );
                    ctx.events.push(SimEvent::SubEnd {
                        owner_slot: slot,
                        team,
                        sub_kind: None,
                        data: vec![2.0, gid as f32],
                    });
                }
                ctx.items.remove(i);
            }
        }
    }

    // ------------------------------------------------------------------ hooks
    /// JS `blockShot(prev, pos, team, dmg)` (L609-632): an enemy shot
    /// segment caught by a kit sub (batch two: waddle / torpedo), a
    /// curtain (absorbed, `dmg * shotMul`) or a sprinkler / beacon capsule
    /// (`dmg`). `muted` mirrors the JS mute wrap around a *ghost
    /// projectile's* `_step` (weapons.js L1572-1574): a remote player's
    /// visual round must not hurt our devices (their owner decides).
    #[allow(clippy::too_many_arguments)]
    pub fn block_shot(
        &mut self,
        prev: Vec3,
        pos: Vec3,
        team: usize,
        dmg: f32,
        muted: bool,
        sb: &SubSet,
    ) -> bool {
        block_shot_plain(
            &mut self.items,
            &mut self.events,
            &mut self.waddle,
            &mut self.torpedo,
            prev,
            pos,
            team,
            dmg,
            muted,
            sb,
        )
    }

    /// JS `blockRay(from, dir, len, team, dmg)` (L633-649): the distance to
    /// the first enemy curtain on a charger beam (or `len`). The curtain
    /// takes `dmg * shotMul` when the beam is cut. The batch-two kit hooks
    /// run first in `SUB_KITS` order (waddle, torpedo), each seeing the
    /// running `len` (JS L634).
    #[allow(clippy::too_many_arguments)]
    pub fn block_ray(
        &mut self,
        from: Vec3,
        dir: Vec3,
        len: f32,
        team: usize,
        dmg: f32,
        muted: bool,
        sb: &SubSet,
    ) -> f32 {
        let mut len = len.min(waddle::block_ray(
            &mut self.waddle,
            &mut self.events,
            from,
            dir,
            len,
            team,
            dmg,
            muted,
        ));
        len = len.min(torpedo::block_ray(
            &mut self.torpedo,
            &mut self.events,
            from,
            dir,
            len,
            team,
            dmg,
            muted,
        ));
        let width = sb.curtain.width;
        let height = sb.curtain.height;
        let shot_mul = sb.curtain.shot_mul;
        let mut best = len;
        let mut hit: Option<usize> = None;
        for (i, it) in self.items.iter().enumerate() {
            if it.team == team || it.state != SubState::Curtain {
                continue;
            }
            let n = it.n.unwrap_or(it.dir);
            let tan = it.tan.unwrap_or(vec3(-n.z, 0.0, n.x));
            let dn = dir.dot(n);
            if dn.abs() < 1e-4 {
                continue;
            }
            let t = (it.pos - from).dot(n) / dn;
            if t <= 0.0 || t >= best {
                continue;
            }
            let p = from + dir * t - it.pos;
            if p.dot(tan).abs() > width / 2.0 || p.y < -0.2 || p.y > height {
                continue;
            }
            best = t;
            hit = Some(i);
        }
        if let Some(i) = hit {
            hurt(&mut self.items, &mut self.events, i, dmg * shot_mul, muted);
        }
        best
    }

    /// JS `damageArea(c, radius, dmg, team)` (L650-658): a blast hurts enemy
    /// curtains / sprinklers / beacons within `radius + 0.4`. Muted (a
    /// ghost's blast): the owner's own blast hurts devices, `netHurt`
    /// carries it across. `G.specials.areaHit` is Task 35. Batch two: the
    /// kit `damageArea` hooks run first in `SUB_KITS` order (waddle,
    /// torpedo — JS L654 loops every kit); the torpedo hook takes the
    /// *plain* variant (no paint / actor context on this path: a caught
    /// locked torpedo is flagged `triggered` and bursts on its next
    /// `tick`, see `sub/torpedo.rs` D-T34-9 notes).
    pub fn damage_area(&mut self, c: Vec3, radius: f32, dmg: f32, team: usize, muted: bool) {
        if muted {
            return;
        }
        waddle::damage_area(
            &mut self.waddle,
            &mut self.events,
            c,
            radius,
            dmg,
            team,
            false,
        );
        torpedo::damage_area_plain(&mut self.torpedo, &mut self.events, c, radius, dmg, team);
        let targets: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.team != team
                    && matches!(
                        it.state,
                        SubState::Curtain | SubState::Spray | SubState::Beacon
                    )
                    && it.pos.distance(c) < radius + 0.4
            })
            .map(|(i, _)| i)
            .collect();
        for i in targets {
            hurt_item(&mut self.items, &mut self.events, i, dmg);
        }
    }

    /// JS `blockActor(a)` (L660-672): enemy players can't walk (or swim)
    /// through a curtain — push them back out to the side they're on and
    /// cancel the inward velocity. Called from `Actor::step` after
    /// integration (actor.js L332).
    pub fn block_actor(&self, a: &mut Actor, t: &PlayerTuning, sb: &SubSet) {
        for it in &self.items {
            if it.state != SubState::Curtain || it.team == a.team {
                continue;
            }
            let n = it.n.unwrap_or(it.dir);
            let tan = it.tan.unwrap_or(vec3(-n.z, 0.0, n.x));
            let d = a.pos - it.pos;
            let along = d.dot(tan);
            let across = d.dot(n);
            let r = t.radius + 0.12;
            if along.abs() > sb.curtain.width / 2.0 + t.radius
                || d.y < -1.2
                || d.y > sb.curtain.height
                || across.abs() > r
            {
                continue;
            }
            let side = if across >= 0.0 { 1.0 } else { -1.0 };
            a.pos += n * (side * r - across);
            let vn = a.vel.dot(n);
            if vn * side < 0.0 {
                a.vel -= n * vn;
            }
        }
    }
}

/// JS `_hurt` (L597-607) without the mute guard (the stand-alone entry
/// points — `net_hurt` / the block hooks — run unmuted by construction).
fn hurt_item(items: &mut [SubItem], events: &mut Vec<SimEvent>, i: usize, dmg: f32) {
    hurt(items, events, i, dmg, false);
}

/// JS `_hurt(it, dmg)` (L597-607).
fn hurt(items: &mut [SubItem], events: &mut Vec<SimEvent>, i: usize, dmg: f32, muted: bool) {
    if items[i].state == SubState::Dead || muted {
        return; // a ghost's hit: its owner's copy decides
    }
    if items[i].ghost {
        // netHurt(it.owner, 'subs', it.gid, dmg) → sendDevHit r2 (L602,
        // netmatch.js L157-159)
        events.push(SimEvent::DeviceHit {
            owner_slot: items[i].owner_slot,
            team: items[i].owner_team,
            kind: "subs".to_string(),
            gid: items[i].gid,
            d: r2(dmg),
        });
        if items[i].state == SubState::Curtain {
            items[i].hp -= dmg; // the curtain fades as it's hit (L603)
        }
        return;
    }
    items[i].hp -= dmg;
    if items[i].state == SubState::Curtain {
        return; // curtains fade instead; `_curtain` removes them at 0
    }
    if items[i].hp <= 0.0 {
        items[i].state = SubState::Dead; // `_destroy` (L674-683)
    }
}

/// JS `blockShot` (L609-632) as a free function: the batch-two kit hooks
/// first in `SUB_KITS` order (waddle, torpedo — the only kit subs with a
/// `blockShot`), then the curtain / sprinkler / beacon items loop. Shared
/// by [`SubSystem::block_shot`] and [`Ctx::probe_block_shot`] (the kit
/// touch / beam probes ride the same JS `G.subs.blockShot`).
#[allow(clippy::too_many_arguments)]
fn block_shot_plain(
    items: &mut [SubItem],
    events: &mut Vec<SimEvent>,
    waddle: &mut [waddle::WaddleItem],
    torpedo: &mut [torpedo::TorpedoItem],
    prev: Vec3,
    pos: Vec3,
    team: usize,
    dmg: f32,
    muted: bool,
    sb: &SubSet,
) -> bool {
    if waddle::block_shot(waddle, events, prev, pos, team, dmg, muted) {
        return true;
    }
    if torpedo::block_shot(torpedo, events, prev, pos, team, dmg, muted) {
        return true;
    }
    let width = sb.curtain.width;
    let height = sb.curtain.height;
    let shot_mul = sb.curtain.shot_mul;
    let mut hit: Option<(usize, f32)> = None;
    for (i, it) in items.iter().enumerate() {
        if it.team == team {
            continue;
        }
        match it.state {
            SubState::Curtain => {
                let n = it.n.unwrap_or(it.dir);
                let tan = it.tan.unwrap_or(vec3(-n.z, 0.0, n.x));
                let a = (prev - it.pos).dot(n);
                let b = (pos - it.pos).dot(n);
                if (a > 0.0) == (b > 0.0) {
                    continue;
                }
                let f = a / (a - b);
                let p = prev.lerp(pos, f) - it.pos;
                if p.dot(tan).abs() > width / 2.0 || p.y < -0.2 || p.y > height {
                    continue;
                }
                hit = Some((i, dmg * shot_mul));
                break;
            }
            SubState::Spray | SubState::Beacon => {
                let h = DEVICE_H;
                let n = it.normal.unwrap_or(UP);
                let base = it.pos + n * (h * 0.5);
                let bottom = vec3(base.x, base.y - h * 0.5, base.z);
                let res = segment_capsule_dist(prev, pos, bottom, 0.3, h);
                if res.dist < 0.34 {
                    hit = Some((i, dmg));
                    break;
                }
            }
            _ => {}
        }
    }
    let Some((i, d)) = hit else { return false };
    hurt(items, events, i, d, muted);
    true
}

/// JS `damageArea` (L650-658) without a [`Ctx`] (the splat-bomb fan inside
/// [`crate::weapon::ProjectileSim::explode_bomb`], reached through
/// `sub/torpedo.rs`'s `explode_bomb_fan` closure): the batch-two kit hooks
/// in `SUB_KITS` order (waddle, torpedo — the torpedo takes the *plain*
/// `triggered`-flag variant), then the curtain / sprinkler / beacon items
/// loop. `tracer` / `boomerang` are carried for the JS kit-loop shape;
/// neither has a `damageArea` hook.
#[allow(clippy::too_many_arguments)]
pub(crate) fn damage_area_fan_plain(
    items: &mut [SubItem],
    events: &mut Vec<SimEvent>,
    waddle: &mut [waddle::WaddleItem],
    torpedo: &mut [torpedo::TorpedoItem],
    _tracer: &mut [tracer::TracerItem],
    _boomerang: &mut [boomerang::BoomerangItem],
    c: Vec3,
    radius: f32,
    dmg: f32,
    team: usize,
    muted: bool,
) {
    if muted {
        return; // JS `netMuted()` guard (L651)
    }
    waddle::damage_area(waddle, events, c, radius, dmg, team, false);
    torpedo::damage_area_plain(torpedo, events, c, radius, dmg, team);
    let targets: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| {
            it.team != team
                && matches!(
                    it.state,
                    SubState::Curtain | SubState::Spray | SubState::Beacon
                )
                && it.pos.distance(c) < radius + 0.4
        })
        .map(|(i, _)| i)
        .collect();
    for i in targets {
        hurt_item(items, events, i, dmg);
    }
}

/// JS `it.sub.throwSpeed` for a thrown kind (the batch-two kits read their
/// own table entries; `tracer` has none — it fires as a beam).
fn throw_speed(kind: SubKind, sb: &SubSet) -> f32 {
    match kind {
        SubKind::Sticky => sb.sticky.throw_speed,
        SubKind::Burst => sb.burst.throw_speed,
        SubKind::Seeker => sb.seeker.throw_speed,
        SubKind::Scan => sb.scan.throw_speed,
        SubKind::Curtain => sb.curtain.throw_speed,
        SubKind::Sprinkler => sb.sprinkler.throw_speed,
        SubKind::Mist => sb.mist.throw_speed,
        SubKind::Shaker => sb.shaker.throw_speed,
        SubKind::Waddle => sb.waddle.throw_speed,
        SubKind::Torpedo => sb.torpedo.throw_speed,
        SubKind::Boomerang => sb.boomerang.throw_speed,
        _ => 0.0,
    }
}

/// The `(radius, damage_max, damage_min, paint_radius)` blast params of a
/// kind (JS `it.sub.radius / damageMax / damageMin / paintRadius` — the
/// `netGhost` op-2 blast, L200).
fn blast_params(kind: SubKind, sb: &SubSet) -> (f32, f32, f32, f32) {
    match kind {
        SubKind::Sticky => (
            sb.sticky.radius,
            sb.sticky.damage_max,
            sb.sticky.damage_min,
            sb.sticky.paint_radius,
        ),
        SubKind::Seeker => (
            sb.seeker.radius,
            sb.seeker.damage_max,
            sb.seeker.damage_min,
            sb.seeker.paint_radius,
        ),
        _ => (0.0, 0.0, 0.0, 0.0),
    }
}

/// JS `sub.inkCost` for a kind (the `SUBS` table). Kit kinds report their
/// cost for the HUD; the batch-one runner never spends it (D-T34-9).
#[must_use]
pub fn ink_cost(kind: SubKind, sb: &SubSet) -> f32 {
    match kind {
        SubKind::Bomb => sb.bomb.ink_cost,
        SubKind::Sticky => sb.sticky.ink_cost,
        SubKind::Burst => sb.burst.ink_cost,
        SubKind::Seeker => sb.seeker.ink_cost,
        SubKind::Scan => sb.scan.ink_cost,
        SubKind::Curtain => sb.curtain.ink_cost,
        SubKind::Sprinkler => sb.sprinkler.ink_cost,
        SubKind::Mine => sb.mine.ink_cost,
        SubKind::Beacon => sb.beacon.ink_cost,
        SubKind::Mist => sb.mist.ink_cost,
        SubKind::Shaker => sb.shaker.ink_cost,
        SubKind::Waddle => sb.waddle.ink_cost,
        SubKind::Torpedo => sb.torpedo.ink_cost,
        SubKind::Tracer => sb.tracer.ink_cost,
        SubKind::Boomerang => sb.boomerang.ink_cost,
    }
}

/// JS `track(e, team, time)` core (subs.js L537-542) on one actor status.
/// Free function since Task 35 batch 2: the sonar reveal ladder writes the
/// track straight onto the roster (specials.js L1269) without a [`Ctx`].
pub(crate) fn apply_track(st: &mut crate::actor::ActorStatus, team: usize, time: f32) {
    st.track = if st.track_team == Some(team) {
        st.track.max(time)
    } else {
        time.max(0.0)
    };
    st.track_team = Some(team);
}

/// The per-frame working context: every field is an independent borrow so
/// the state machine can mutate items, actors, paint and projectiles from
/// one function without fighting the borrow checker (the JS globals
/// `G.subs` / `G.actors` / `G.paint` / `G.projectiles` / `G.netm`).
pub(crate) struct Ctx<'a> {
    items: &'a mut Vec<SubItem>,
    events: &'a mut Vec<SimEvent>,
    rng: &'a mut Rng,
    actors: &'a mut [Actor],
    paint: &'a mut PaintGrid,
    world: &'a CollisionWorld,
    projectiles: &'a mut ProjectileSim,
    t: &'a PlayerTuning,
    sb: &'a SubSet,
    remote: Option<&'a [bool]>,
    seq: &'a mut u32,
    time: f32,
    // ---- batch two: the kit sub lists (JS `SUB_KITS[kind]` module-level
    // `items` / `drops` / `beams` arrays; kits/*.js).
    shaker: &'a mut Vec<shaker::ShakerItem>,
    waddle: &'a mut Vec<waddle::WaddleItem>,
    torpedo: &'a mut Vec<torpedo::TorpedoItem>,
    torpedo_drops: &'a mut Vec<torpedo::TorpedoDrop>,
    drop_recs: &'a mut Vec<Vec<(usize, u32)>>,
    tracer: &'a mut Vec<tracer::TracerItem>,
    boomerang: &'a mut Vec<boomerang::BoomerangItem>,
}

impl<'a> Ctx<'a> {
    /// The next device's `gid` (D-T34-5): `owner_slot * 100_000 + off`,
    /// `off` 1..=100_000 so a slot-0 item never gets the falsy `gid` 0
    /// (JS `netId` returns 0 offline and `gid ||` then falls back; the sim
    /// always mints a live id).
    fn gid_next(&mut self, owner_slot: usize) -> u32 {
        *self.seq += 1;
        let off = *self.seq % 100_000;
        let off = if off == 0 { 100_000 } else { off };
        (owner_slot * 100_000 + off as usize) as u32
    }

    // ------------------------------------------------------------------ throw / plant
    /// JS `_throw` (L81-94) minus the prop mesh / audio.
    #[allow(clippy::too_many_arguments)]
    fn throw_item(
        &mut self,
        kind: SubKind,
        owner_idx: usize,
        owner_slot: usize,
        owner_team: usize,
        pos: Vec3,
        vel: Vec3,
        gid: u32,
        src_nid: Option<u8>,
        ghost: bool,
    ) {
        let dir = vec3(vel.x, 0.0, vel.z).normalize_or_zero();
        self.items.push(SubItem {
            kind,
            owner_idx,
            owner_slot,
            owner_team,
            team: owner_team,
            state: SubState::Fly,
            pos,
            vel,
            age: 0.0,
            t: 0.0,
            born: 0.0,
            ghost,
            gid,
            src_nid,
            dir,
            normal: None,
            n: None,
            tan: None,
            hp: 1.0,
            uses: 0,
            fuse: None,
            heading: 0.0,
            trail: 0.0,
            stuck_t: 0.0,
            dash: false,
            target: None,
            tagged: Vec::new(),
            direct: None,
            pulse_t: 0.0,
            spin: 0.0,
            // JS L86: `sp: !!a.specialActive` — the thrower's live special.
            sp: self.actors[owner_idx].special_active.is_some(),
        });
    }

    /// JS `_plant` (L107-119) minus the mesh / `face,u,v` / audio.
    #[allow(clippy::too_many_arguments)]
    fn plant(
        &mut self,
        kind: SubKind,
        owner_idx: usize,
        owner_slot: usize,
        owner_team: usize,
        pos: Vec3,
        normal: Vec3,
        gid: u32,
        src_nid: Option<u8>,
        ghost: bool,
        time: f32,
    ) {
        let hp = if kind == SubKind::Beacon {
            self.sb.beacon.hp as f32
        } else {
            1.0 // `sub.hp || 1` — the mine has no hp (L113)
        };
        let uses = if kind == SubKind::Beacon {
            self.sb.beacon.uses
        } else {
            0
        };
        let state = if kind == SubKind::Mine {
            SubState::Mine
        } else {
            SubState::Beacon
        };
        self.items.push(SubItem {
            kind,
            owner_idx,
            owner_slot,
            owner_team,
            team: owner_team,
            state,
            pos,
            vel: Vec3::ZERO,
            age: 0.0,
            t: 0.0,
            born: time,
            ghost,
            gid,
            src_nid,
            dir: Vec3::ZERO,
            normal: Some(normal),
            n: None,
            tan: None,
            hp,
            uses,
            fuse: None,
            heading: 0.0,
            trail: 0.0,
            stuck_t: 0.0,
            dash: false,
            target: None,
            tagged: Vec::new(),
            direct: None,
            pulse_t: 0.0,
            spin: 0.0,
            // JS L113: `sp: !!a.specialActive`.
            sp: self.actors[owner_idx].special_active.is_some(),
        });
        if kind == SubKind::Mine {
            let i = self.items.len() - 1;
            self.paint_under(i, 1.1, ghost); // L116
        }
    }

    // ------------------------------------------------------------------ state machine
    /// JS `_step` (L170-184).
    fn step_item(&mut self, i: usize, dt: f32, muted: bool) {
        match self.items[i].state {
            SubState::Fly => self.fly(i, dt, muted),
            SubState::Stuck => self.stuck(i, dt, muted),
            SubState::Run => self.run(i, dt, muted),
            SubState::Cloud => self.cloud(i, dt, muted),
            SubState::Mist => self.mist(i, dt, muted),
            SubState::Curtain => self.curtain(i, dt),
            SubState::Spray => self.spray(i, dt),
            SubState::Mine => self.mine(i, dt, muted),
            SubState::Beacon => {} // JS `_beacon`: the spin is presentation
            SubState::Dead => {}
        }
    }

    /// JS `_fly` (L219-258).
    fn fly(&mut self, i: usize, dt: f32, muted: bool) {
        let kind = self.items[i].kind;
        self.items[i].vel.y -= GRAV * dt;
        let prev = self.items[i].pos;
        let vel = self.items[i].vel;
        self.items[i].pos += vel * dt;
        // direct hits on enemies (burst pellets, mist poisons for the whole
        // mist) — L223-236
        if matches!(kind, SubKind::Burst | SubKind::Mist) {
            let team = self.items[i].team;
            let pos = self.items[i].pos;
            let mut found: Option<(usize, f32)> = None;
            for (ei, e) in self.actors.iter().enumerate() {
                if e.team == team || !e.alive {
                    continue;
                }
                let h = if e.form == Form::Squid {
                    self.t.squid_height
                } else {
                    self.t.height
                };
                let res = segment_capsule_dist(prev, pos, e.pos, self.t.radius + 0.1, h);
                if res.dist < self.t.radius + 0.12 {
                    found = Some((ei, res.t));
                    break;
                }
            }
            if let Some((ei, t)) = found {
                let p = self.items[i].pos;
                self.items[i].pos = prev.lerp(p, t);
                if kind == SubKind::Burst {
                    self.pellet_blast(i, Some(ei), None, muted);
                    self.items[i].state = SubState::Dead;
                } else {
                    self.items[i].direct = Some(ei);
                    self.start_mist(i, muted);
                }
                return;
            }
        }
        // clouds burst in mid-air when their fuse runs out — L238
        if matches!(kind, SubKind::Scan | SubKind::Mist) && self.items[i].age > self.fuse_of(kind) {
            if kind == SubKind::Scan {
                self.start_cloud(i);
            } else {
                self.start_mist(i, muted);
            }
            return;
        }
        // world hit — L241-255 (`segment` with the default grate test)
        let pos = self.items[i].pos;
        let hit = self.world.segment(prev, pos, false);
        if hit.hit {
            let n = hit.normal;
            let floor = n.y > 0.6;
            match kind {
                SubKind::Sticky => {
                    let fuse = self.sb.sticky.fuse;
                    self.stick(i, hit.point, n, SubState::Stuck, Some(fuse));
                    return;
                }
                SubKind::Sprinkler => {
                    self.stick(i, hit.point, n, SubState::Spray, None);
                    return;
                }
                SubKind::Burst => {
                    self.pellet_blast(i, None, Some(hit.point), muted);
                    self.items[i].state = SubState::Dead;
                    return;
                }
                SubKind::Scan => {
                    self.items[i].pos = hit.point + n * 0.3;
                    self.start_cloud(i);
                    return;
                }
                SubKind::Mist => {
                    self.items[i].pos = hit.point + n * 0.3;
                    self.start_mist(i, muted);
                    return;
                }
                SubKind::Seeker if floor => {
                    self.start_run(i, hit.point);
                    return;
                }
                SubKind::Curtain if floor => {
                    self.start_curtain(i, hit.point, muted);
                    return;
                }
                _ => {
                    // bounce (seeker / curtain off walls, until they find a
                    // floor) — L252-254
                    self.items[i].pos = hit.point + n * 0.12;
                    let vn = self.items[i].vel.dot(n);
                    self.items[i].vel += n * (-vn * 1.4);
                    self.items[i].vel *= 0.55;
                }
            }
        }
        // sea — L256
        if self.items[i].pos.y < self.t.water_y - 1.8 {
            self.items[i].state = SubState::Dead;
        }
    }

    fn fuse_of(&self, kind: SubKind) -> f32 {
        match kind {
            SubKind::Scan => self.sb.scan.fuse,
            SubKind::Mist => self.sb.mist.fuse,
            _ => f32::MAX,
        }
    }

    /// JS `_stick` (L260-276) minus the mesh / audio / `sub:land` emit.
    fn stick(&mut self, i: usize, point: Vec3, normal: Vec3, state: SubState, fuse: Option<f32>) {
        self.items[i].state = state;
        self.items[i].t = 0.0;
        self.items[i].fuse = fuse;
        self.items[i].normal = Some(normal);
        self.items[i].pos = point + normal * 0.005;
        self.items[i].hp = if state == SubState::Spray {
            self.sb.sprinkler.hp as f32
        } else {
            1.0 // `sub.hp || 1` — the sticky has no hp
        };
        self.items[i].born = self.time;
        if state == SubState::Spray {
            // one sprinkler per player: a new one replaces the old (L269-273)
            let (os, ot) = (self.items[i].owner_slot, self.items[i].owner_team);
            for j in 0..self.items.len() {
                if j != i
                    && self.items[j].owner_slot == os
                    && self.items[j].owner_team == ot
                    && self.items[j].kind == SubKind::Sprinkler
                    && self.items[j].state == SubState::Spray
                {
                    self.items[j].state = SubState::Dead;
                }
            }
            self.items[i].pulse_t = 0.0;
            self.items[i].spin = 0.0;
        }
    }

    /// JS `_stuck` (L279-289) minus the blink / beep.
    fn stuck(&mut self, i: usize, _dt: f32, muted: bool) {
        let fuse = self.items[i].fuse.unwrap_or(0.0);
        if self.items[i].t >= fuse {
            let (rad, dmax, dmin, prad) = blast_params(SubKind::Sticky, self.sb);
            let n = self.items[i].normal.unwrap_or(UP);
            let pos = self.items[i].pos;
            self.blast(i, pos, rad, dmax, dmin, prad, n, muted);
            self.items[i].state = SubState::Dead;
        }
    }

    /// JS `_startRun` (L292-299).
    fn start_run(&mut self, i: usize, point: Vec3) {
        self.items[i].state = SubState::Run;
        self.items[i].t = 0.0;
        self.items[i].pos = point;
        let dir = self.items[i].dir;
        self.items[i].heading = dir.x.atan2(dir.z);
        self.items[i].trail = 0.0;
        self.items[i].stuck_t = 0.0;
        self.items[i].target = None;
        self.items[i].dash = false;
    }

    /// JS `_run` (L300-351) minus the mesh pose.
    fn run(&mut self, i: usize, dt: f32, muted: bool) {
        let s = &self.sb.seeker;
        let seek_range = s.seek_range;
        let speed0 = s.speed;
        let commit_dist = s.commit_dist;
        let turn_rate = s.turn_rate;
        let creep = s.creep;
        let trigger_dist = s.trigger_dist;
        let life = s.life;
        let trail_radius = s.trail_radius;
        // retarget (L302-310)
        let team = self.items[i].team;
        let pos = self.items[i].pos;
        if self.items[i].target.is_none_or(|ti| !self.actors[ti].alive) {
            let mut best: Option<usize> = None;
            let mut bd = seek_range;
            for (ei, e) in self.actors.iter().enumerate() {
                if e.team == team || !e.alive {
                    continue;
                }
                let d = e.pos.distance(pos);
                if d < bd {
                    bd = d;
                    best = Some(ei);
                }
            }
            if best != self.items[i].target {
                self.items[i].dash = false;
            }
            self.items[i].target = best;
        }
        // steering (L312-324)
        let mut speed = speed0;
        let mut heading = self.items[i].heading;
        let mut dash = self.items[i].dash;
        let pos = self.items[i].pos;
        if let Some(ti) = self.items[i].target {
            let (tp, tv) = (self.actors[ti].pos, self.actors[ti].vel);
            let (dx, dz) = (tp.x - pos.x, tp.z - pos.z);
            let dist = dx.hypot(dz);
            let mut d = dx.atan2(dz) - heading;
            while d > std::f32::consts::PI {
                d -= TAU;
            }
            while d < -std::f32::consts::PI {
                d += TAU;
            }
            if !dash && dist < commit_dist && d.abs() < 0.35 {
                dash = true;
            } else if dash && (d.abs() > std::f32::consts::FRAC_PI_2 || dist > commit_dist + 1.0) {
                dash = false;
            }
            if !dash {
                heading += clamp(d, -turn_rate * dt, turn_rate * dt);
                if tv.x.hypot(tv.z) < 2.5 {
                    speed *= 1.0 - creep * clamp((d.abs() - 0.7) / 0.6, 0.0, 1.0);
                }
            }
        }
        self.items[i].heading = heading;
        self.items[i].dash = dash;
        let (hx, hz) = (heading.sin(), heading.cos());
        let step = speed * dt;
        // walls: turn away along them (L328-333)
        let from = vec3(pos.x, pos.y + 0.2, pos.z);
        let w = self
            .world
            .raycast(from, vec3(hx, 0.0, hz), step + 0.3, true);
        let mut pos = pos;
        let mut stuck_t = self.items[i].stuck_t;
        if w.hit && w.normal.y.abs() < 0.5 {
            stuck_t += dt;
            let side = if hx * w.normal.z - hz * w.normal.x >= 0.0 {
                1.0
            } else {
                -1.0
            };
            self.items[i].heading += side * 6.0 * dt;
        } else {
            stuck_t = (stuck_t - dt).max(0.0);
            let nx = pos.x + hx * step;
            let nz = pos.z + hz * step;
            let gy = self.world.ground_height(nx, nz, pos.y + 0.6, false);
            if gy == f32::NEG_INFINITY {
                self.items[i].state = SubState::Dead;
                self.items[i].stuck_t = stuck_t;
                return;
            }
            pos = vec3(nx, gy, nz);
            self.items[i].pos = pos;
            let mut trail = self.items[i].trail + step;
            if trail > 0.35 {
                trail = 0.0;
                let seed = self.rng.next_f32();
                let area = if muted {
                    0.0
                } else {
                    self.paint.splat(
                        self.world,
                        vec3(pos.x, pos.y + 0.12, pos.z),
                        trail_radius,
                        team,
                        &SplatOpts {
                            seed,
                            stretch: None,
                            stretch_amt: None,
                            kind: None,
                        },
                    )
                };
                self.credit(i, area);
            }
            self.items[i].trail = trail;
        }
        self.items[i].stuck_t = stuck_t;
        // reached / expired / groundless (L346-350)
        let t = self.items[i].t;
        let reached = self.items[i]
            .target
            .is_some_and(|ti| self.actors[ti].pos.distance(self.items[i].pos) < trigger_dist);
        if reached || t > life || stuck_t > 0.6 {
            let (rad, dmax, dmin, prad) = (s.radius, s.damage_max, s.damage_min, s.paint_radius);
            let c = self.items[i].pos + vec3(0.0, 0.2, 0.0);
            self.blast(i, c, rad, dmax, dmin, prad, UP, muted);
            self.items[i].state = SubState::Dead;
        }
    }

    /// JS `_startCloud` (L354-363) minus the cloud mesh / audio / emit.
    fn start_cloud(&mut self, i: usize) {
        self.items[i].state = SubState::Cloud;
        self.items[i].t = 0.0;
        self.items[i].tagged.clear();
    }

    /// JS `_cloud` (L365-378).
    fn cloud(&mut self, i: usize, _dt: f32, _muted: bool) {
        let s = &self.sb.scan;
        let radius = s.radius;
        let track_time = s.track_time;
        let cloud_time = s.cloud_time;
        let grow = clamp(self.items[i].t / 0.25, 0.0, 1.0);
        let team = self.items[i].team;
        let pos = self.items[i].pos;
        let mut newly: Vec<usize> = Vec::new();
        for (ei, e) in self.actors.iter().enumerate() {
            if e.team == team || !e.alive || self.items[i].tagged.contains(&ei) {
                continue;
            }
            let c = vec3(e.pos.x, e.pos.y + 0.7, e.pos.z);
            if c.distance(pos) < radius * grow {
                newly.push(ei);
            }
        }
        for ei in newly {
            self.items[i].tagged.push(ei);
            self.track(ei, team, track_time);
        }
        if self.items[i].t >= cloud_time {
            self.items[i].state = SubState::Dead;
        }
    }

    /// JS `_startMist` (L381-399) minus the puff meshes / audio / emit.
    fn start_mist(&mut self, i: usize, _muted: bool) {
        self.items[i].state = SubState::Mist;
        self.items[i].t = 0.0;
        if let Some(di) = self.items[i].direct {
            let mist_time = self.sb.mist.mist_time;
            self.poison(di, mist_time); // a direct hit lasts the whole mist
        }
    }

    /// JS `_mist` (L400-414).
    fn mist(&mut self, i: usize, _dt: f32, _muted: bool) {
        let s = &self.sb.mist;
        let radius = s.radius;
        let mist_time = s.mist_time;
        let grow = clamp(self.items[i].t / 0.4, 0.0, 1.0);
        let team = self.items[i].team;
        let pos = self.items[i].pos;
        let targets: Vec<usize> = self
            .actors
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.team != team
                    && e.alive
                    && vec3(e.pos.x, e.pos.y + 0.6, e.pos.z).distance(pos) < radius * grow
            })
            .map(|(ei, _)| ei)
            .collect();
        for ei in targets {
            self.poison(ei, 0.6);
        }
        if self.items[i].t >= mist_time {
            self.items[i].state = SubState::Dead;
        }
    }

    /// JS `_startCurtain` (L417-436) minus the sheet mesh / audio / emit.
    fn start_curtain(&mut self, i: usize, point: Vec3, muted: bool) {
        self.items[i].state = SubState::Curtain;
        self.items[i].t = 0.0;
        self.items[i].hp = self.sb.curtain.hp as f32;
        self.items[i].pos = point;
        let dir = self.items[i].dir;
        self.items[i].n = Some(dir);
        self.items[i].tan = Some(vec3(-dir.z, 0.0, dir.x));
        self.paint_under(i, 1.4, muted);
    }

    /// JS `_curtain` (L437-445) minus the shader uniforms.
    fn curtain(&mut self, i: usize, dt: f32) {
        self.items[i].hp -= self.sb.curtain.decay * dt;
        if self.items[i].hp <= 0.0 {
            self.destroy(i);
        }
    }

    /// JS `_spray` (L448-470): the sprinkler's droplet pulses. A ghost never
    /// sprays (its owner's drops arrive as ghost rounds, L454). RNG order
    /// per drop: `[a_jitter, out, sp, radius, seed]` (the `vis` draw inside
    /// `spawnDrop` is presentation-only and not consumed).
    fn spray(&mut self, i: usize, dt: f32) {
        let s = &self.sb.sprinkler;
        let pulse = s.pulse;
        let spray_fade = s.spray_fade;
        let drops = s.drops;
        let drop_damage = s.drop_damage;
        self.items[i].pulse_t -= dt;
        self.items[i].spin += dt
            * (if self.items[i].t < spray_fade {
                9.0
            } else {
                4.5
            });
        if self.items[i].pulse_t > 0.0 || self.items[i].ghost {
            return;
        }
        self.items[i].pulse_t = pulse
            * (if self.items[i].t < spray_fade {
                1.0
            } else {
                2.0
            });
        let n = self.items[i].normal.unwrap_or(UP);
        // a tangent frame on the mounting surface (L461-463)
        let mut t1 = if n.x.abs() > 0.9 {
            vec3(0.0, 0.0, 1.0)
        } else {
            vec3(1.0, 0.0, 0.0)
        };
        t1 += n * -t1.dot(n);
        t1 = t1.normalize_or_zero();
        let t2 = n.cross(t1);
        let base = self.items[i].pos + n * 0.2;
        let spin = self.items[i].spin;
        let owner_idx = self.items[i].owner_idx;
        for k in 0..drops {
            let a = spin * 1.7 + (k as f32 / drops as f32) * TAU + self.rng.next_f32() * 0.4;
            let out = self.rng.next_f32() * 0.4 + 0.6;
            let sp = 3.2 + self.rng.next_f32() * 2.6;
            let radius = 0.5 + self.rng.next_f32() * 0.25;
            let seed = self.rng.next_f32();
            let v = (t1 * a.cos() + t2 * a.sin()) * out + n * 0.75;
            let spec = DropSpec {
                life: 1.2,
                straight: 0.0,
                size: 0.08,
                radius,
                damage: drop_damage,
                dmg_far: drop_damage,
                trail_start: 0.0,
                trail_every: 0.0,
                trail_radius: 0.3,
                grav: 22.0,
                drag: 0.4,
                delay: 0.0,
            };
            self.projectiles
                .push_drop(&self.actors[owner_idx], &spec, base, v * sp, seed);
        }
    }

    /// JS `_mine` (L473-496) minus the visibility / blink. A ghost mine never
    /// goes off by itself: its owner decides (the end record).
    fn mine(&mut self, i: usize, dt: f32, muted: bool) {
        if self.items[i].ghost {
            return;
        }
        let s = &self.sb.mine;
        if let Some(f) = self.items[i].fuse {
            let f = f - dt;
            self.items[i].fuse = Some(f);
            if f <= 0.0 {
                self.mine_blast(i, muted);
            }
            return;
        }
        if self.items[i].t < s.arm_time {
            return;
        }
        let team = self.items[i].team;
        let pos = self.items[i].pos;
        let trigger = s.trigger_radius;
        for e in self.actors.iter() {
            if e.team == team || !e.alive {
                continue;
            }
            if e.pos.distance(pos) < trigger {
                self.items[i].fuse = Some(s.delay);
                return;
            }
        }
    }

    /// JS `_mineBlast` (L497-512).
    fn mine_blast(&mut self, i: usize, muted: bool) {
        if self.items[i].state == SubState::Dead {
            return;
        }
        let s = &self.sb.mine;
        let (radius, damage, track_time, paint_radius) =
            (s.radius, s.damage, s.track_time, s.paint_radius);
        let team = self.items[i].team;
        let c = self.items[i].pos + vec3(0.0, 0.3, 0.0);
        self.paint_at(i, c, paint_radius, muted);
        let mut hits: Vec<usize> = Vec::new();
        for (ei, e) in self.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            let ec = vec3(e.pos.x, e.pos.y + 0.7, e.pos.z);
            if ec.distance(c) > radius || !self.world.los(c, ec) {
                continue;
            }
            hits.push(ei);
        }
        for ei in hits {
            self.apply_hit_routed(i, ei, damage, muted);
            self.track(ei, team, track_time);
        }
        self.damage_area(c, radius, 30.0, team, muted);
        self.items[i].state = SubState::Dead;
    }

    // ------------------------------------------------------------------ effects
    /// JS `track(e, team, time)` (L537-542). The `actor:tracked` emit is not
    /// in the netmatch `FORWARD` list (presentation only).
    pub fn track(&mut self, ei: usize, team: usize, time: f32) {
        apply_track(&mut self.actors[ei].status, team, time);
    }

    /// JS `poison(e, time)` (L543-547).
    fn poison(&mut self, ei: usize, time: f32) {
        let st = &mut self.actors[ei].status;
        st.poison = st.poison.max(time);
    }

    /// JS `_blast` (L550-567): paint + damage enemies (with LOS) around a
    /// blast, falloff `lerp(dmgMin, dmgMax, k*k)`, then `damageArea(60)`.
    #[allow(clippy::too_many_arguments)]
    fn blast(
        &mut self,
        i: usize,
        c: Vec3,
        radius: f32,
        dmg_max: f32,
        dmg_min: f32,
        paint_radius: f32,
        n: Vec3,
        muted: bool,
    ) {
        let team = self.items[i].team;
        self.paint_at(i, c + n * 0.2, paint_radius, muted);
        let mut hits: Vec<(usize, f32)> = Vec::new();
        for (ei, e) in self.actors.iter().enumerate() {
            if e.team == team || !e.alive {
                continue;
            }
            let ec = vec3(e.pos.x, e.pos.y + 0.7, e.pos.z);
            let d = ec.distance(c);
            if d > radius || !self.world.los(c + n * 0.3, ec) {
                continue;
            }
            let k = 1.0 - clamp((d - 0.8) / (radius - 0.8), 0.0, 1.0);
            hits.push((ei, lerp(dmg_min, dmg_max, k * k)));
        }
        for (ei, dmg) in hits {
            self.apply_hit_routed(i, ei, dmg, muted);
        }
        self.damage_area(c, radius, 60.0, team, muted);
    }

    /// JS `_pelletBlast` (L568-583): the burst bomb's pellet pop — direct
    /// damage to the victim it ran into, splash to the rest, `damageArea(25)`.
    fn pellet_blast(&mut self, i: usize, direct: Option<usize>, at: Option<Vec3>, muted: bool) {
        let s = &self.sb.burst;
        let (radius, direct_damage, splash_damage, paint_radius) =
            (s.radius, s.direct_damage, s.splash_damage, s.paint_radius);
        let team = self.items[i].team;
        let c = at.unwrap_or(self.items[i].pos);
        self.paint_at(i, vec3(c.x, c.y + 0.2, c.z), paint_radius, muted);
        if let Some(di) = direct {
            self.apply_hit_routed(i, di, direct_damage, muted);
        }
        let mut hits: Vec<usize> = Vec::new();
        for (ei, e) in self.actors.iter().enumerate() {
            if e.team == team || !e.alive || Some(ei) == direct {
                continue;
            }
            let ec = vec3(e.pos.x, e.pos.y + 0.7, e.pos.z);
            if ec.distance(c) > radius || !self.world.los(vec3(c.x, c.y + 0.3, c.z), ec) {
                continue;
            }
            hits.push(ei);
        }
        for ei in hits {
            self.apply_hit_routed(i, ei, splash_damage, muted);
        }
        self.damage_area(c, radius, 25.0, team, muted);
    }

    /// JS `_paint` (L584-591): one central splat + four ring splats, then
    /// the turf credit. RNG order: `[seed, (a, rr, radius, seed) × 4]`.
    /// Muted: the draws still happen (JS evaluates the args before
    /// `paint.js` L470 returns 0), the splats do not.
    fn paint_at(&mut self, i: usize, c: Vec3, r: f32, muted: bool) {
        let team = self.items[i].team;
        let mut area = 0.0;
        let seed = self.rng.next_f32();
        if !muted {
            area += self.paint.splat(
                self.world,
                c,
                r,
                team,
                &SplatOpts {
                    seed,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            );
        }
        for _ in 0..4 {
            let a = self.rng.next_f32() * TAU;
            let rr = r * (0.55 + self.rng.next_f32() * 0.45);
            let rad = 0.5 + self.rng.next_f32() * 0.4;
            let s2 = self.rng.next_f32();
            if muted {
                continue;
            }
            let p = vec3(c.x + a.cos() * rr, c.y + 0.3, c.z + a.sin() * rr);
            area += self.paint.splat(
                self.world,
                p,
                rad,
                team,
                &SplatOpts {
                    seed: s2,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            );
        }
        self.credit(i, area);
    }

    /// JS `_paintUnder` (L592).
    fn paint_under(&mut self, i: usize, r: f32, muted: bool) {
        let team = self.items[i].team;
        let pos = self.items[i].pos;
        let seed = self.rng.next_f32();
        let area = if muted {
            0.0
        } else {
            self.paint.splat(
                self.world,
                vec3(pos.x, pos.y + 0.2, pos.z),
                r,
                team,
                &SplatOpts {
                    seed,
                    stretch: None,
                    stretch_amt: None,
                    kind: None,
                },
            )
        };
        self.credit(i, area);
    }

    /// JS `_credit` (L594): turf for the thrower, routed by `it.sp`
    /// (Task 35: specials deployed during a special never charge —
    /// `addTurfNoSpecial`, subs.js L594 / weapons.js L845).
    fn credit(&mut self, i: usize, area: f32) {
        self.events.push(SimEvent::Turf {
            owner_slot: self.items[i].owner_slot,
            team: self.items[i].owner_team,
            area,
            no_special: self.items[i].sp,
        });
    }

    /// JS `applyHit` (weapons.js L1545-1554) routed through
    /// `shouldApplyHit` (netmatch.js L711-717): a ghost's blast drops its
    /// hits (the owner's screen decides), a hit on a proxy rides
    /// [`SimEvent::HitSend`], a local victim takes the damage.
    fn apply_hit_routed(&mut self, i: usize, vi: usize, dmg: f32, _muted: bool) {
        if self.items[i].ghost {
            // JS `shouldApplyHit`: the attacker of a ghost item is always the
            // remote owner → route 'drop' (the owner's screen decides the hit).
            return;
        }
        let atk_idx = self.items[i].owner_idx;
        if self
            .remote
            .is_some_and(|m| m.get(atk_idx).copied().unwrap_or(false))
        {
            return; // route 'drop'
        }
        let team = self.items[i].team;
        {
            let v = &self.actors[vi];
            if !v.alive || v.team == team {
                return; // JS applyHit L1546 guard
            }
        }
        let (vs, vt) = (self.actors[vi].slot, self.actors[vi].team);
        let send = self
            .remote
            .is_some_and(|m| m.get(vi).copied().unwrap_or(false));
        let (owner_slot, atk_pos) = (self.items[i].owner_slot, self.actors[atk_idx].pos);
        if send {
            self.events.push(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(
                &mut self.actors[vi],
                dmg,
                Some((team, owner_slot)),
                Some(atk_pos),
                self.t,
            );
            self.events.push(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }

    /// JS `damageArea` (L650-658) from inside the state machine (honours the
    /// caller's mute). Batch two: the kit `damageArea` hooks run first in
    /// `SUB_KITS` order (waddle, torpedo); the torpedo takes the *ctx*
    /// variant — with paint / actor context here, a caught locked torpedo
    /// bursts inline exactly like JS (`sub/torpedo.rs` D-T34-9 notes).
    fn damage_area(&mut self, c: Vec3, radius: f32, dmg: f32, team: usize, muted: bool) {
        if muted {
            return;
        }
        waddle::damage_area(self.waddle, self.events, c, radius, dmg, team, false);
        torpedo::damage_area_ctx(self, c, radius, dmg, team);
        let targets: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.team != team
                    && matches!(
                        it.state,
                        SubState::Curtain | SubState::Spray | SubState::Beacon
                    )
                    && it.pos.distance(c) < radius + 0.4
            })
            .map(|(j, _)| j)
            .collect();
        for j in targets {
            hurt(self.items, self.events, j, dmg, false);
        }
    }

    /// JS `G.subs.blockShot(prev, pos, team, dmg)` reached from inside the
    /// kit state machines (torpedo.js `touchDevice` L451-456 with `dmg = 0`
    /// — a pure touch probe — torpedo.js `dropStep` and tracer.js's beam
    /// device probe L294): the same fan as [`Ctx::probe_block_shot`]'s name
    /// suggests — kit hooks then items, over this frame's lists.
    fn probe_block_shot(
        &mut self,
        prev: Vec3,
        pos: Vec3,
        team: usize,
        dmg: f32,
        muted: bool,
    ) -> bool {
        block_shot_plain(
            self.items,
            self.events,
            self.waddle,
            self.torpedo,
            prev,
            pos,
            team,
            dmg,
            muted,
            self.sb,
        )
    }

    /// JS `hitRouted(ghost, owner, gid-team…, vi, dmg)` for the batch-two
    /// kit modules (`sub/*.rs` call sites): [`Ctx::apply_hit_routed`]'s
    /// routing with the attacker identity passed in (a kit sub's blast has
    /// no [`SubItem`] to read the owner off). A ghost kit sub drops its
    /// hits (the owner's screen decides, JS `shouldApplyHit`); a local
    /// attacker's hit on a remote victim rides [`SimEvent::HitSend`], on a
    /// local victim lands through [`apply_hit`] + [`SimEvent::Hit`].
    #[allow(clippy::too_many_arguments)]
    fn hit_routed(
        &mut self,
        ghost: bool,
        atk_idx: usize,
        owner_slot: usize,
        team: usize,
        vi: usize,
        dmg: f32,
    ) {
        if ghost {
            return; // JS `shouldApplyHit`: the owner's screen decides
        }
        if self
            .remote
            .is_some_and(|m| m.get(atk_idx).copied().unwrap_or(false))
        {
            return; // route 'drop'
        }
        {
            let v = &self.actors[vi];
            if !v.alive || v.team == team {
                return; // JS applyHit L1546 guard
            }
        }
        let (vs, vt) = (self.actors[vi].slot, self.actors[vi].team);
        let send = self
            .remote
            .is_some_and(|m| m.get(vi).copied().unwrap_or(false));
        let atk_pos = self.actors[atk_idx].pos;
        if send {
            self.events.push(SimEvent::HitSend {
                attacker_slot: owner_slot,
                victim_slot: vs,
                victim_team: vt,
                team,
                damage: dmg,
            });
        } else {
            let outcome = apply_hit(
                &mut self.actors[vi],
                dmg,
                Some((team, owner_slot)),
                Some(atk_pos),
                self.t,
            );
            self.events.push(SimEvent::Hit {
                attacker_slot: owner_slot,
                victim_slot: vs,
                team,
                damage: dmg,
                killed: outcome.killed,
            });
        }
    }

    /// JS `_destroy` (L674-683) minus the burst / audio / `sub:destroyed`
    /// emit (not in the netmatch `FORWARD` list).
    fn destroy(&mut self, i: usize) {
        if self.items[i].state == SubState::Dead {
            return;
        }
        self.items[i].state = SubState::Dead;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::FIXED_DT;
    use crate::geometry::{Bounds, Brush, BrushCommon, Mural};

    const DT: f32 = FIXED_DT;

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

    const B: Bounds = Bounds {
        min_x: -30.0,
        max_x: 30.0,
        min_z: -30.0,
        max_z: 30.0,
    };

    /// A flat 20×20 m deck with one thrower on the origin and a hand-driven
    /// `SubSystem` — the TR-34.1/34.3 per-kind probes run the sub state
    /// machine directly (no actor physics, no weapon runners).
    pub(super) struct Scene {
        pub(super) world: CollisionWorld,
        pub(super) t: PlayerTuning,
        pub(super) sb: SubSet,
        pub(super) subs: SubSystem,
        pub(super) actors: Vec<Actor>,
        pub(super) paint: PaintGrid,
        pub(super) ps: ProjectileSim,
        pub(super) time: f32,
    }

    impl Scene {
        pub(super) fn new() -> Self {
            let tuning = crate::embedded_tuning();
            let world =
                CollisionWorld::from_brushes(&[bx([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0])], B);
            let t = tuning.player;
            let mut thrower = Actor::new(0, 0, &t);
            thrower.pos = vec3(0.0, 0.05, 0.0);
            thrower.set_invuln(0.0);
            let paint = PaintGrid::new(&world);
            Self {
                world,
                t,
                sb: tuning.subs,
                subs: SubSystem::new(11),
                actors: vec![thrower],
                paint,
                ps: ProjectileSim::new(7),
                time: 0.0,
            }
        }

        /// JS `use(a, sub)` (subs.js L71-79) for the thrower.
        pub(super) fn deploy(&mut self, kind: SubKind) -> Vec<SimEvent> {
            self.actors[0].sub = kind;
            self.subs.use_sub(
                0,
                1,
                &self.world,
                &mut self.actors,
                &mut self.paint,
                &mut self.ps,
                &self.t,
                &self.sb,
                None,
                self.time,
            );
            self.subs.drain_events()
        }

        /// Live items of `kind`'s list: the kits ride their own vectors
        /// (batch two), the batch-one subs ride `items`.
        pub(super) fn kit_count(&self, kind: SubKind) -> usize {
            match kind {
                SubKind::Shaker => self.subs.shaker.len(),
                SubKind::Waddle => self.subs.waddle.len(),
                SubKind::Torpedo => self.subs.torpedo.len(),
                SubKind::Tracer => self.subs.tracer.len(),
                SubKind::Boomerang => self.subs.boomerang.len(),
                _ => self.subs.items.iter().filter(|it| it.kind == kind).count(),
            }
        }

        /// The `ghost` flag of the first item of a kit list.
        pub(super) fn kit_ghosted(&self, kind: SubKind) -> bool {
            match kind {
                SubKind::Shaker => self.subs.shaker.first().is_some_and(|it| it.ghost),
                SubKind::Waddle => self.subs.waddle.first().is_some_and(|it| it.ghost),
                SubKind::Torpedo => self.subs.torpedo.first().is_some_and(|it| it.ghost),
                SubKind::Tracer => self.subs.tracer.first().is_some_and(|it| it.ghost),
                SubKind::Boomerang => self.subs.boomerang.first().is_some_and(|it| it.ghost),
                _ => false,
            }
        }

        /// An enemy (team 1) standing at `z` on the deck.
        pub(super) fn add_enemy(&mut self, z: f32) {
            self.add_enemy_at(0.0, z);
        }

        /// An enemy (team 1) standing at `(x, z)` on the deck.
        pub(super) fn add_enemy_at(&mut self, x: f32, z: f32) {
            let slot = self.actors.len();
            let mut e = Actor::new(1, slot, &self.t);
            e.pos = vec3(x, 0.05, z);
            e.set_invuln(0.0);
            self.actors.push(e);
        }

        /// JS `update(dt)` + the dead splice (subs.js L155-169).
        pub(super) fn step(&mut self) -> Vec<SimEvent> {
            self.time += DT;
            self.subs.update(
                DT,
                &self.world,
                &mut self.actors,
                &mut self.paint,
                &mut self.ps,
                &self.t,
                &self.sb,
                None,
                self.time,
            );
            self.subs.drain_events()
        }
    }

    pub(super) fn hits(evs: &[SimEvent]) -> Vec<(usize, f32)> {
        evs.iter()
            .filter_map(|e| match e {
                SimEvent::Hit {
                    victim_slot,
                    damage,
                    ..
                } => Some((*victim_slot, *damage)),
                _ => None,
            })
            .collect()
    }

    fn spawn_of<'a>(evs: &'a [SimEvent], kind: &str) -> Option<&'a Vec<f32>> {
        evs.iter().find_map(|e| match e {
            SimEvent::SubSpawn { sub_kind, data, .. } if sub_kind == kind => Some(data),
            _ => None,
        })
    }

    #[test]
    fn tr34_1_ink_cost_matches_the_subs_table() {
        // JS `SUBS[k].inkCost` (the config.js table; spent by `use`,
        // subs.js L71-79).
        let s = Scene::new();
        let sb = &s.sb;
        assert_eq!(ink_cost(SubKind::Bomb, sb), 70.0);
        assert_eq!(ink_cost(SubKind::Sticky, sb), 70.0);
        assert_eq!(ink_cost(SubKind::Burst, sb), 40.0);
        assert_eq!(ink_cost(SubKind::Seeker, sb), 65.0);
        assert_eq!(ink_cost(SubKind::Scan, sb), 55.0);
        assert_eq!(ink_cost(SubKind::Curtain, sb), 55.0);
        assert_eq!(ink_cost(SubKind::Sprinkler, sb), 60.0);
        assert_eq!(ink_cost(SubKind::Mine, sb), 55.0);
        assert_eq!(ink_cost(SubKind::Beacon, sb), 70.0);
        assert_eq!(ink_cost(SubKind::Mist, sb), 60.0);
        // Kit kinds report their HUD cost but deploy nothing (D-T34-9).
        for k in [
            SubKind::Shaker,
            SubKind::Waddle,
            SubKind::Torpedo,
            SubKind::Tracer,
            SubKind::Boomerang,
        ] {
            assert!(ink_cost(k, sb) > 0.0, "{k:?}");
        }
        assert_eq!(ink_cost(SubKind::Sticky, sb), sb.sticky.ink_cost);
        assert_eq!(ink_cost(SubKind::Beacon, sb), sb.beacon.ink_cost);
    }

    #[test]
    fn tr34_1_thrown_kinds_launch_fly_items_with_recorded_ballistics() {
        // JS `_throw` (subs.js L81-94) + the `[0, gid, 0, r2 pos, r2 vel]`
        // spawn record (L77).
        let cases = [
            (SubKind::Sticky, 13.5),
            (SubKind::Burst, 16.0),
            (SubKind::Seeker, 9.0),
            (SubKind::Scan, 14.0),
            (SubKind::Curtain, 9.0),
            (SubKind::Sprinkler, 12.0),
            (SubKind::Mist, 13.5),
        ];
        for (kind, speed) in cases {
            let mut s = Scene::new();
            let evs = s.deploy(kind);
            assert_eq!(s.subs.items.len(), 1, "{kind:?}");
            let it = &s.subs.items[0];
            assert_eq!(it.state, SubState::Fly, "{kind:?}");
            assert!(!it.ghost);
            assert_eq!(it.gid, 1, "slot 0 mints off=1 (D-T34-5)");
            assert!(
                (it.pos - vec3(0.0, 1.4, 0.0)).length() < 1e-3,
                "{kind:?} hand-off point"
            );
            let want = throw_velocity(&s.actors[0], speed);
            assert_eq!(it.vel, want, "{kind:?} ballistic seed");
            let data = spawn_of(&evs, kind.id_str()).unwrap_or_else(|| panic!("{kind:?} spawn"));
            assert_eq!(data.len(), 9);
            assert_eq!(data[0], 0.0);
            assert_eq!(data[1], 1.0);
            assert_eq!(data[2], 0.0);
            assert_eq!(data[3], 0.0);
            assert_eq!(data[4], 1.4);
            assert_eq!(data[5], 0.0);
            assert_eq!(data[6], r2(want.x));
            assert_eq!(data[7], r2(want.y));
            assert_eq!(data[8], r2(want.z));
            assert!(
                evs.iter()
                    .any(|e| matches!(e, SimEvent::SubUse { kind: k2, .. } if *k2 == kind))
            );
        }
        // `throwSpeed` anchors (tuning.json `subs`).
        let s = Scene::new();
        assert_eq!(s.sb.sticky.throw_speed, 13.5);
        assert_eq!(s.sb.burst.throw_speed, 16.0);
        assert_eq!(s.sb.seeker.throw_speed, 9.0);
        assert_eq!(s.sb.scan.throw_speed, 14.0);
        assert_eq!(s.sb.curtain.throw_speed, 9.0);
        assert_eq!(s.sb.sprinkler.throw_speed, 12.0);
        assert_eq!(s.sb.mist.throw_speed, 13.5);
    }

    #[test]
    fn tr34_1_mine_plants_an_armed_device_with_ink_under_it() {
        // JS `_place`/`_plant` (subs.js L96-119): raycast the floor,
        // `hp || 1`, `_paintUnder(1.1)`; the record is
        // `[1, gid, 0, pos, normal, yaw]`.
        let mut s = Scene::new();
        let evs = s.deploy(SubKind::Mine);
        assert_eq!(s.subs.items.len(), 1);
        let it = &s.subs.items[0];
        assert_eq!(it.state, SubState::Mine);
        assert_eq!(it.gid, 1);
        assert_eq!(it.hp, 1.0, "`sub.hp || 1` (L113)");
        assert!(it.pos.distance(vec3(0.0, 0.0, 0.0)) < 1e-3);
        assert_eq!(it.normal, Some(vec3(0.0, 1.0, 0.0)));
        assert_eq!(it.born, s.time);
        let data = spawn_of(&evs, "mine").expect("the mine spawn");
        assert_eq!(data.len(), 10);
        assert_eq!(data[0], 1.0);
        assert_eq!(data[1], 1.0);
        assert_eq!(data[2], 0.0);
        assert_eq!(data[7], 1.0, "the floor normal");
        assert_eq!(data[9], 0.0, "yaw 0");
        assert!(s.paint.coverage()[0] > 0.0, "_paintUnder r=1.1");
        assert_eq!(s.sb.mine.arm_time, 0.9);
        assert_eq!(s.sb.mine.delay, 0.35);
        assert_eq!(s.sb.mine.damage, 45.0);
        assert_eq!(s.sb.mine.track_time, 8.0);
        assert_eq!(s.sb.mine.trigger_radius, 2.1);
        assert_eq!(s.sb.mine.max, 2, "TR-34.3 mine cap");
    }

    #[test]
    fn tr34_1_beacon_plants_a_shootable_device() {
        // JS `_plant` (subs.js L107-119): the beacon carries `sub.hp` and
        // `sub.uses` (the super jump spends them — D-T34-2).
        let mut s = Scene::new();
        let evs = s.deploy(SubKind::Beacon);
        let it = &s.subs.items[0];
        assert_eq!(it.state, SubState::Beacon);
        assert_eq!(it.gid, 1);
        assert_eq!(it.hp, 50.0);
        assert_eq!(it.uses, 2);
        assert!(it.pos.distance(vec3(0.0, 0.0, 0.0)) < 1e-3);
        let data = spawn_of(&evs, "beacon").expect("the beacon spawn");
        assert_eq!(data.len(), 10);
        assert_eq!(data[0], 1.0);
        assert_eq!(s.sb.beacon.max, 3, "TR-34.3 beacon cap");
        assert_eq!(s.sb.beacon.hp, 50);
        assert_eq!(s.sb.beacon.uses, 2);
    }

    #[test]
    fn tr34_1_bomb_deploy_nothing_kits_deploy() {
        // D-T34-9: the splat bomb rides `crate::weapon` (weapons.js L152
        // routes `sub === 'bomb'` to `throwBomb`), so `use_sub` never
        // deploys it. Task 34 batch two wired the five kits (weapons.js
        // L145): each `use_sub` now spawns a live item in its own list and
        // emits the kit `['k', …]` spawn record + `SubUse`.
        let mut s = Scene::new();
        let evs = s.deploy(SubKind::Bomb);
        assert!(s.subs.items.is_empty(), "bomb");
        assert!(evs.is_empty(), "bomb");

        for kind in [
            SubKind::Shaker,
            SubKind::Waddle,
            SubKind::Torpedo,
            SubKind::Tracer,
            SubKind::Boomerang,
        ] {
            let mut s = Scene::new();
            let evs = s.deploy(kind);
            assert!(s.kit_count(kind) >= 1, "{kind:?} spawns an item");
            let spawn = evs.iter().find_map(|e| match e {
                SimEvent::SubSpawn { sub_kind, .. } => Some(sub_kind.clone()),
                _ => None,
            });
            assert_eq!(
                spawn.as_deref(),
                Some(kind.id_str()),
                "{kind:?} spawn record"
            );
        }
        let s = Scene::new();
        assert_eq!(s.sb.bomb.ink_cost, 70.0);
        assert_eq!(s.sb.bomb.fuse, 0.95);
        assert_eq!(s.sb.bomb.radius, 3.1);
        assert_eq!(s.sb.bomb.damage_max, 180.0);
        assert_eq!(s.sb.bomb.damage_min, 35.0);
    }

    #[test]
    fn tr34_kit_ghost_round_trip() {
        // The batch-two wire path: the owner's `SubSpawn` record replays as
        // a ghost through `net_ghost` (the `['k', nid, <kit>, …]` arm). Slot
        // 0 / nid 0 pack the same gid (D-T34-5), as in the beacon round trip.
        for kind in [
            SubKind::Shaker,
            SubKind::Waddle,
            SubKind::Torpedo,
            SubKind::Tracer,
            SubKind::Boomerang,
        ] {
            let mut host = Scene::new();
            let evs = host.deploy(kind);
            let data = evs
                .iter()
                .find_map(|e| match e {
                    SimEvent::SubSpawn { sub_kind, data, .. } if *sub_kind == kind.id_str() => {
                        Some(data.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{kind:?} spawn record"));

            let mut guest = Scene::new();
            guest.subs.net_ghost(
                0,
                0,
                kind.id_str(),
                &data,
                &[0],
                &guest.world,
                &mut guest.actors,
                &mut guest.paint,
                &mut guest.ps,
                &guest.t,
                &guest.sb,
                None,
                0.0,
            );
            assert_eq!(guest.kit_count(kind), 1, "{kind:?} ghost spawns");
            assert!(guest.kit_ghosted(kind), "{kind:?} ghost flag set");
        }
    }

    #[test]
    fn tr34_1_sticky_sits_on_its_fuse_then_blasts() {
        // JS `_stick`/`_stuck` (subs.js L260-289): fuse 2.4 s ≈ 144 frames,
        // then `_blast` paints r=3.3.
        let mut s = Scene::new();
        s.deploy(SubKind::Sticky);
        let mut landed = None;
        for f in 0..120 {
            s.step();
            if s.subs.items[0].state == SubState::Stuck {
                landed = Some(f);
                break;
            }
        }
        assert!(landed.is_some(), "the charge sticks to the deck");
        assert_eq!(s.subs.items[0].fuse, Some(2.4));
        let mut gone = None;
        for f in 0..150 {
            s.step();
            if s.subs.items.is_empty() {
                gone = Some(f);
                break;
            }
        }
        let f = gone.expect("the sticky detonates on its fuse") as f32;
        assert!((f * DT - 2.4).abs() <= 3.0 * DT, "detonated at {f} frames");
        assert!(s.paint.coverage()[0] > 0.0, "_blast paints");
        assert_eq!(s.sb.sticky.fuse, 2.4);
        assert_eq!(s.sb.sticky.radius, 4.0);
    }

    #[test]
    fn tr34_1_burst_direct_hit_pops_for_direct_and_splash_damage() {
        // JS `_fly` direct check + `_pelletBlast` (subs.js L223-236,
        // L568-583): 60 to the victim it ran into, 35 to the rest inside
        // r=2.1 (tuning.json `burst`).
        let mut s = Scene::new();
        s.add_enemy(4.0);
        s.add_enemy(5.4);
        s.actors[0].aim_pitch = -0.28; // flat shot (the +0.28 lift cancels)
        s.deploy(SubKind::Burst);
        let mut all = Vec::new();
        for _ in 0..60 {
            let e = s.step();
            all.extend(e);
            if s.subs.items.is_empty() {
                break;
            }
        }
        assert!(s.subs.items.is_empty(), "the burst pops on the direct hit");
        let h = hits(&all);
        assert!(h.contains(&(1, 60.0)), "direct damage {h:?}");
        assert!(h.contains(&(2, 35.0)), "splash damage {h:?}");
        assert!(s.paint.coverage()[0] > 0.0);
        assert_eq!(s.sb.burst.direct_damage, 60.0);
        assert_eq!(s.sb.burst.splash_damage, 35.0);
        assert_eq!(s.sb.burst.radius, 2.1);
    }

    #[test]
    fn tr34_1_scan_land_cloud_tracks_the_enemy() {
        // JS `_startCloud`/`_cloud` (subs.js L354-378): a 0.9 s cloud that
        // `track(8)`s every enemy inside the growing r=4.2.
        let mut s = Scene::new();
        s.add_enemy(8.0);
        s.deploy(SubKind::Scan);
        let mut cloud = 0usize;
        for _ in 0..200 {
            s.step();
            if s.subs
                .items
                .first()
                .is_some_and(|it| it.state == SubState::Cloud)
            {
                cloud += 1;
            } else if cloud > 0 {
                break;
            }
        }
        assert!(cloud > 0, "the orb bursts into a cloud on landing");
        assert!(
            (cloud as f32 * DT - 0.9).abs() <= 3.0 * DT,
            "cloud for {cloud} frames"
        );
        assert!(s.subs.items.is_empty());
        assert_eq!(s.actors[1].status.track, 8.0);
        assert_eq!(s.actors[1].status.track_team, Some(0));
        assert_eq!(s.sb.scan.cloud_time, 0.9);
        assert_eq!(s.sb.scan.track_time, 8.0);
    }

    #[test]
    fn tr34_1_seeker_skitters_to_its_target_and_blasts() {
        // JS `_startRun`/`_run` (subs.js L292-351): skitter at 6.3 m/s,
        // detonate within `triggerDist` 1.2, blast r=2.8 with the 180→35
        // falloff.
        let mut s = Scene::new();
        s.add_enemy(8.0);
        s.deploy(SubKind::Seeker);
        let mut run = 0usize;
        let mut all = Vec::new();
        for _ in 0..400 {
            let e = s.step();
            all.extend(e);
            if s.subs
                .items
                .first()
                .is_some_and(|it| it.state == SubState::Run)
            {
                run += 1;
            } else if run > 0 {
                break;
            }
        }
        assert!(run > 0, "the skitter lands and runs");
        assert!(s.subs.items.is_empty());
        let h = hits(&all);
        assert_eq!(h.len(), 1, "{h:?}");
        assert_eq!(h[0].0, 1);
        assert!(
            h[0].1 > 35.0 && h[0].1 <= 180.0,
            "falloff damage {}",
            h[0].1
        );
        assert!(s.paint.coverage()[0] > 0.0, "trail + blast paint");
        assert_eq!(s.sb.seeker.life, 4.5);
        assert_eq!(s.sb.seeker.speed, 6.3);
    }

    #[test]
    fn tr34_1_curtain_fades_on_its_decay() {
        // JS `_startCurtain`/`_curtain` (subs.js L417-445): hp 170, decay
        // 19/s → 170/19 ≈ 8.95 s ≈ 537 frames.
        let mut s = Scene::new();
        s.deploy(SubKind::Curtain);
        let mut mounted = false;
        for _ in 0..120 {
            s.step();
            if s.subs.items[0].state == SubState::Curtain {
                mounted = true;
                break;
            }
        }
        assert!(mounted, "the curtain mounts on the floor");
        assert_eq!(s.subs.items[0].hp, 170.0);
        assert_eq!(s.subs.items[0].n, Some(vec3(0.0, 0.0, 1.0)));
        assert!(s.paint.coverage()[0] > 0.0, "_paintUnder r=1.4");
        let mut gone = None;
        for f in 0..600 {
            s.step();
            if s.subs.items.is_empty() {
                gone = Some(f);
                break;
            }
        }
        let f = gone.expect("the curtain runs dry") as f32;
        assert!(
            (f * DT - 170.0 / 19.0).abs() <= 2.0 * DT + 0.02,
            "faded at {f} frames"
        );
        assert_eq!(s.sb.curtain.hp, 170);
        assert_eq!(s.sb.curtain.decay, 19.0);
        assert_eq!(s.sb.curtain.shot_mul, 0.5);
    }

    #[test]
    fn tr34_1_sprinkler_pulses_drops_on_schedule() {
        // JS `_spray` (subs.js L448-470): `drops` (6) rounds every `pulse`
        // (0.3 s) while mounted; hp 70.
        let mut s = Scene::new();
        s.deploy(SubKind::Sprinkler);
        for _ in 0..120 {
            s.step();
            if s.subs.items[0].state == SubState::Spray {
                break;
            }
        }
        assert_eq!(s.subs.items[0].state, SubState::Spray);
        assert_eq!(s.subs.items[0].hp, 70.0);
        assert_eq!(s.subs.items[0].pulse_t, 0.0, "_stick seeds 0 (L274)");
        s.step();
        assert_eq!(s.ps.list.len(), 6, "the first pulse");
        assert!((s.subs.items[0].pulse_t - 0.3).abs() < 1e-4);
        let mut gap = None;
        for f in 1..40 {
            s.step();
            if s.ps.list.len() > 6 {
                gap = Some(f);
                break;
            }
        }
        let g = gap.expect("a second pulse");
        assert!((17..=19).contains(&g), "pulse gap {g} frames");
        assert_eq!(s.ps.list.len(), 12);
        assert_eq!(s.sb.sprinkler.drops, 6);
        assert_eq!(s.sb.sprinkler.pulse, 0.3);
        assert_eq!(s.sb.sprinkler.hp, 70);
        assert_eq!(s.sb.sprinkler.drop_damage, 8.0);
    }

    #[test]
    fn tr34_1_mine_arms_then_trips_and_tracks() {
        // JS `_mine`/`_mineBlast` (subs.js L473-512): arm 0.9 s, trigger
        // r=2.1, delay 0.35 s, damage 45 + `track(8)`.
        let mut s = Scene::new();
        s.deploy(SubKind::Mine);
        s.add_enemy(1.0);
        for _ in 0..48 {
            let e = s.step();
            assert!(hits(&e).is_empty(), "no blast before the arm time");
        }
        assert_eq!(s.subs.items[0].fuse, None, "0.8 s < arm_time 0.9 s");
        let mut all = Vec::new();
        let mut tripped = false;
        for _ in 0..60 {
            let e = s.step();
            all.extend(e);
            if !tripped && s.subs.items.first().is_some_and(|it| it.fuse.is_some()) {
                tripped = true;
            }
            if s.subs.items.is_empty() {
                break;
            }
        }
        assert!(tripped, "the proximity trips the fuse");
        assert!(s.subs.items.is_empty(), "the mine went off");
        let h = hits(&all);
        assert_eq!(h, vec![(1, 45.0)]);
        assert_eq!(s.actors[1].status.track, 8.0);
        assert_eq!(s.actors[1].status.track_team, Some(0));
        assert!(s.paint.coverage()[0] > 0.0);
    }

    #[test]
    fn tr34_1_mist_lingers_and_poisons() {
        // JS `_startMist`/`_mist` (subs.js L381-414): a 4.5 s mist that
        // poisons (0.6 s per tick, L411) every enemy inside the growing
        // r=3.3, while an enemy caught by the flying orb is poisoned for
        // the whole mistTime (L231 + L396). The fixture drives the sub
        // machine only, not the actor decay (actor.js L354), so `poison`
        // keeps the raw `max` value each channel wrote.
        let mut s = Scene::new();
        s.add_enemy(8.0); // on the flight line: the orb hits it directly
        s.add_enemy_at(2.5, 8.0); // off-axis: inside r=3.3, never hit in fly
        s.deploy(SubKind::Mist);
        let mut mist = 0usize;
        for _ in 0..400 {
            s.step();
            if s.subs
                .items
                .first()
                .is_some_and(|it| it.state == SubState::Mist)
            {
                mist += 1;
            } else if mist > 0 {
                break;
            }
        }
        assert!(mist > 0, "the murk bomb bursts into mist");
        assert!(
            (mist as f32 * DT - 4.5).abs() <= 3.0 * DT,
            "mist for {mist} frames"
        );
        assert_eq!(s.actors[1].status.poison, 4.5, "direct hit: full mistTime");
        assert_eq!(s.actors[2].status.poison, 0.6, "tick refresh inside r=3.3");
        assert_eq!(s.sb.mist.mist_time, 4.5);
        assert_eq!(s.sb.mist.radius, 3.3);
    }

    #[test]
    fn tr34_3_beacon_cap_evicts_the_oldest() {
        // JS `_place` (subs.js L100-101): a fourth beacon destroys the
        // oldest (`max 3`); the splice emits the `[2, gid]` end record
        // (L164-167).
        let mut s = Scene::new();
        let mut all = Vec::new();
        for _ in 0..4 {
            all.extend(s.deploy(SubKind::Beacon));
        }
        assert_eq!(s.subs.items.len(), 4, "the eviction is a Dead flag");
        assert_eq!(s.subs.items[0].gid, 1);
        assert_eq!(s.subs.items[0].state, SubState::Dead);
        let gids: Vec<u32> = s.subs.items[1..].iter().map(|it| it.gid).collect();
        assert_eq!(gids, vec![2, 3, 4]);
        let e = s.step();
        let gids: Vec<u32> = s.subs.items.iter().map(|it| it.gid).collect();
        assert_eq!(gids, vec![2, 3, 4]);
        assert!(
            e.iter()
                .any(|ev| matches!(ev, SimEvent::SubEnd { data, .. } if *data == [2.0, 1.0])),
            "{e:?}"
        );
        assert_eq!(
            all.iter()
                .filter(|e| matches!(e, SimEvent::SubSpawn { .. }))
                .count(),
            4
        );
    }

    #[test]
    fn tr34_3_mine_cap_detonates_the_oldest() {
        // JS `_place` (subs.js L100): a third mine sets off the oldest
        // (`max 2`) — a real blast, not a silent removal.
        let mut s = Scene::new();
        s.deploy(SubKind::Mine);
        s.deploy(SubKind::Mine);
        let cov1 = s.paint.coverage()[0];
        let e3 = s.deploy(SubKind::Mine);
        assert!(
            s.paint.coverage()[0] > cov1,
            "the eviction blast painted r=2"
        );
        assert_eq!(s.subs.items[0].state, SubState::Dead);
        assert!(e3.iter().any(|e| matches!(e, SimEvent::Turf { .. })));
        let e = s.step();
        let gids: Vec<u32> = s.subs.items.iter().map(|it| it.gid).collect();
        assert_eq!(gids, vec![2, 3]);
        assert!(
            e.iter()
                .any(|ev| matches!(ev, SimEvent::SubEnd { data, .. } if *data == [2.0, 1.0]))
        );
    }
}

// ============================================================ threat view (Task 47)
// JS `SUB_KITS[k].threats?.(L)` fan (bots.js `_threatScan` L1275): the
// read-only device descriptors the bot brain scores and acts on. The JS
// descriptors are live getter objects cached on the item (`it.thr`,
// waddle.js L664-685 / torpedo.js L709-726); the sim rebuilds a value view
// per scan and re-resolves the held one by [`ThreatRef`] every frame (a
// removed / dead device resolving to `None` is JS `!d.live`). The JS
// descriptor's `hp` / `hover` / `owner` / `obj` fields are never read by
// bots.js and are dropped.

/// Which device kind a [`ThreatView`] describes (JS `d.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreatKind {
    /// `waddle` — the walking bomb (JS `kits/waddle.js threatOf`).
    Waddle,
    /// `torpedo` — the homing torpedo (JS `kits/torpedo.js threatOf`).
    Torpedo,
}

/// JS `d.state` across both kits: the two kits' state enums share `fly` and
/// never overlap the rest (`_threatScore` / `_threatCtl` branch on the
/// string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreatState {
    /// `fly` — both kits' post-throw arc.
    Fly,
    /// `sense` — waddle only.
    Sense,
    /// `wake` — waddle only.
    Wake,
    /// `walk` — waddle only.
    Walk,
    /// `unfold` — torpedo only.
    Unfold,
    /// `launch` — torpedo only.
    Launch,
}

/// One device snapshot (JS `threatOf(it)` / `threatOf(t)`).
#[derive(Debug, Clone, Copy)]
pub struct ThreatView {
    /// JS `d.kind`.
    pub kind: ThreatKind,
    /// JS `it.owner` — actor-list index (part of the cross-frame identity).
    pub owner_idx: usize,
    /// JS `it.gid` (part of the cross-frame identity; unique per owner,
    /// D-T34-5 keying).
    pub gid: u32,
    /// JS `d.team`.
    pub team: usize,
    /// JS `d.pos`.
    pub pos: Vec3,
    /// JS `d.aimY` — the hit-centre height over `pos.y` (waddle `MID`,
    /// torpedo `0`).
    pub aim_y: f32,
    /// JS `d.radius`.
    pub radius: f32,
    /// JS `d.trigger` (waddle `s.triggerDist`; torpedo `HIT_R +
    /// PLAYER.radius`; the `|| 1.2` fallback sites never fire).
    pub trigger: f32,
    /// JS `d.speed` (waddle `s.speed`; torpedo `s.launchSpeed`).
    pub speed: f32,
    /// JS `d.senseRadius` — `Some` for waddles only (JS `undefined` torpedoes
    /// hit the `|| 7` / `|| 6.5` fallbacks).
    pub sense_radius: Option<f32>,
    /// JS `d.lockRange` — `Some` for torpedoes only.
    pub lock_range: Option<f32>,
    /// JS `d.ground`.
    pub ground: bool,
    /// JS `d.state`.
    pub state: ThreatState,
    /// JS `d.shootable`.
    pub shootable: bool,
    /// JS `d.locked`.
    pub locked: bool,
    /// JS `d.target` — state-gated. The getter's `alive(target)` term is
    /// redundant here: the only consumer is `d.target === a` and the reader
    /// is alive while it thinks.
    pub target: Option<usize>,
    /// JS `d.vel`.
    pub vel: Vec3,
    /// JS `d.left` — `None` for torpedoes (JS `undefined`; the
    /// `d.left !== undefined` branch skips).
    pub left: Option<f32>,
}

impl ThreatView {
    /// This view's cross-frame identity.
    #[must_use]
    pub fn r#ref(&self) -> ThreatRef {
        ThreatRef {
            kind: self.kind,
            owner_idx: self.owner_idx,
            gid: self.gid,
        }
    }
}

/// Cross-frame device identity (JS descriptor object identity via `it.thr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreatRef {
    /// Which kit list to look in.
    pub kind: ThreatKind,
    /// The owner's actor-list index.
    pub owner_idx: usize,
    /// The item's `gid` (unique per owner).
    pub gid: u32,
}

/// JS `kits/waddle.js threatOf` (L664-685). `MID` = `0.11 * WSCALE` = 0.198
/// (waddle.js L35); the `hop` term of the `vel` getter is absent per D-T34-7
/// (the sim's waddle never hops).
fn waddle_threat(sb: &SubSet, it: &waddle::WaddleItem) -> ThreatView {
    let s = &sb.waddle;
    let state = match it.state {
        waddle::WaddleState::Fly => ThreatState::Fly,
        waddle::WaddleState::Sense => ThreatState::Sense,
        waddle::WaddleState::Wake => ThreatState::Wake,
        waddle::WaddleState::Walk => ThreatState::Walk,
        waddle::WaddleState::Dead => ThreatState::Walk, // unreachable (`threats` filters dead)
    };
    let locked = matches!(
        it.state,
        waddle::WaddleState::Wake | waddle::WaddleState::Walk
    );
    let vel = if it.state == waddle::WaddleState::Walk && !it.air {
        vec3(it.heading.sin() * s.speed, 0.0, it.heading.cos() * s.speed)
    } else if it.state == waddle::WaddleState::Fly || it.air {
        it.vel
    } else {
        Vec3::ZERO
    };
    let left = if it.state == waddle::WaddleState::Walk {
        Some(
            (s.life - it.walk_t)
                .min((s.max_travel - it.travel) / s.speed)
                .max(0.0),
        )
    } else {
        Some(s.life)
    };
    ThreatView {
        kind: ThreatKind::Waddle,
        owner_idx: it.owner_idx,
        gid: it.gid,
        team: it.team,
        pos: it.pos,
        aim_y: 0.11 * 1.8,
        radius: s.radius,
        trigger: s.trigger_dist,
        speed: s.speed,
        sense_radius: Some(s.sense_radius),
        lock_range: None,
        ground: true,
        state,
        shootable: it.state != waddle::WaddleState::Fly,
        locked,
        target: if locked { it.target } else { None },
        vel,
        left,
    }
}

/// JS `kits/torpedo.js threatOf` (L709-726).
fn torpedo_threat(sb: &SubSet, player_radius: f32, t: &torpedo::TorpedoItem) -> ThreatView {
    let s = &sb.torpedo;
    let state = match t.state {
        torpedo::TorpedoState::Fly => ThreatState::Fly,
        torpedo::TorpedoState::Unfold => ThreatState::Unfold,
        torpedo::TorpedoState::Launch => ThreatState::Launch,
        torpedo::TorpedoState::Dead => ThreatState::Fly, // unreachable (`threats` filters dead)
    };
    let locked = t.locked();
    let vel = match t.state {
        torpedo::TorpedoState::Launch => t.dir * t.speed,
        torpedo::TorpedoState::Fly => t.vel,
        _ => Vec3::ZERO,
    };
    ThreatView {
        kind: ThreatKind::Torpedo,
        owner_idx: t.owner_idx,
        gid: t.gid,
        team: t.team,
        pos: t.pos,
        aim_y: 0.0,
        radius: s.radius,
        trigger: torpedo::HIT_R + player_radius,
        speed: s.launch_speed,
        sense_radius: None,
        lock_range: Some(s.lock_range),
        ground: false,
        state,
        shootable: true, // JS `live(t)` — the list already excludes dead
        locked,
        target: if locked { t.target } else { None },
        vel,
        left: None,
    }
}

impl SubSystem {
    /// JS `SUB_KITS[k].threats?.(L)` fan (bots.js L1275): every live device
    /// in kit registration order (waddle, then torpedo — the only kits with
    /// a `threats` hook). Ghosts are included (JS does not filter them).
    #[must_use]
    pub fn threats(&self, sb: &SubSet, player_radius: f32) -> Vec<ThreatView> {
        let mut out = Vec::new();
        for it in &self.waddle {
            if it.state != waddle::WaddleState::Dead {
                out.push(waddle_threat(sb, it));
            }
        }
        for t in &self.torpedo {
            if t.live() {
                out.push(torpedo_threat(sb, player_radius, t));
            }
        }
        out
    }

    /// Re-resolve a [`ThreatRef`] to a live view (JS reads the descriptor's
    /// `live` getter; a removed / dead device resolves to `None`).
    #[must_use]
    pub fn find_threat(&self, r: ThreatRef, sb: &SubSet, player_radius: f32) -> Option<ThreatView> {
        match r.kind {
            ThreatKind::Waddle => self
                .waddle
                .iter()
                .find(|it| {
                    it.owner_idx == r.owner_idx
                        && it.gid == r.gid
                        && it.state != waddle::WaddleState::Dead
                })
                .map(|it| waddle_threat(sb, it)),
            ThreatKind::Torpedo => self
                .torpedo
                .iter()
                .find(|t| t.owner_idx == r.owner_idx && t.gid == r.gid && t.live())
                .map(|t| torpedo_threat(sb, player_radius, t)),
        }
    }
}

#[cfg(test)]
#[path = "sub/shaker_tests.rs"]
mod shaker_tests;

#[cfg(test)]
#[path = "sub/waddle_tests.rs"]
mod waddle_tests;

#[cfg(test)]
#[path = "sub/torpedo_tests.rs"]
mod torpedo_tests;

#[cfg(test)]
#[path = "sub/tracer_tests.rs"]
mod tracer_tests;

#[cfg(test)]
#[path = "sub/boomerang_tests.rs"]
mod boomerang_tests;
