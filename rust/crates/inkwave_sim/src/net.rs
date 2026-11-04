//! Task 15 — PROTO v1 boundary: network *types and documentation only, no
//! transport* (spec FR-15 / AC-10; PORT_MAP maps `src/net/netmatch.js` here,
//! the relay clients `session.js`/`transport.js`/`mock.js` to `inkwave::net`
//! in M2+).
//!
//! Every type below mirrors a wire concept of the upstream netcode so the sim
//! layer can one day be replicated *as-is*:
//!
//! - [`ActorSample`] — the 24-key object `unpackActor` builds from the
//!   24-slot `packActor` array (netmatch.js L723-757; the array's slot 0 is
//!   the actor's `nid`, which becomes the tick's routing key — modelled as
//!   [`Tick::actors`] pairs), built losslessly from [`crate::actor::Actor`] +
//!   [`crate::weapon::WeaponRunner`] by [`ActorSample::from_actor`]. The JS
//!   packer rounds (`r2`/`r3`/`Math.round`); that quantisation is a
//!   *transport* concern, so the Rust type keeps full f32 precision and the
//!   wire layer rounds at serialise time.
//! - [`Tick`] — the `{k:'t', ts, a:[…], e:[…]}` owner tick (netmatch.js
//!   `_sendTick` L190-203, 20 Hz).
//! - [`TickEvent`] — the owner event timeline records `['s' …] / ['p' …] /
//!   ['b' …] / ['tr' …] / ['ev' …] / ['k' …] / ['z' …]` (recSplat L109-114,
//!   recProj L116-122, recBomb L124-128, recKit L134-137, `_onLocalEvent`
//!   L139-143, recZone L148; boss `['bm'|'bc']` (L494/L496) are out of M1
//!   scope and kept as commented placeholders).
//! - [`DirectMsg`] — the point-to-point messages that bypass the playback
//!   timeline: `{k:'hit'}` / `{k:'dh'}` / `{k:'bhit'}` (sendHit L163-167,
//!   sendDevHit L157-160, sendBossHit L150-154).
//! - [`HostMsg`] — host-broadcast control: `{k:'st'}` state, `{k:'res'}`
//!   result, `{k:'end'}` (L80, L650-666). `{k:'own'}` ownership exists in
//!   the receive switch but is a reserved stub the host never sends — see
//!   [`HostMsg::Ownership`].
//! - [`ControlFrame`] — the relay's JSON control frames `welcome / join /
//!   leave / err / pong` and the `b|` / `s|to|` / `m|from|` envelope prefixes
//!   (transport.js L1-3 & L100-102, server/src/index.js L10-16).
//!
//! Field-by-field provenance lives on each field's doc comment; the prose
//! contract (ownership, playback clock, Hermite path, corrections, ink
//! replay) is in `docs/NET.md` — this module only fixes the *data shapes*.
//!
//! # Relay frame format (PROTO v1, unchanged for the Rust port)
//!
//! One WebSocket per player to a Cloudflare Durable Object relay, one room
//! object per 5-character code (`server/`). The relay never parses game
//! payloads; it only routes envelopes:
//!
//! ```text
//! out  "b|<json>"          broadcast to everyone else   (Transport.broadcast)
//! out  "s|<to>|<json>"     to one member                (Transport.sendTo)
//! out  "ping"              liveness, answered by the runtime with "pong"
//! out  {"t":"lock","v":b}  host: refuse joins while a match runs
//! in   "m|<from>|<json>"   fan-out of a peer payload    (Transport.onMessage)
//! in   {"t":"welcome","id","host","members":[{id,name}]}
//! in   {"t":"join","m":{id,name}}   {"t":"leave","id","host"}
//! in   {"t":"err","e":"…"} (then close)  {"t":"pong","c"}
//! ```
//!
//! URL: `ws(s)://<relay>/room/<code>?name=<n>&v=1[&create=1]`
//! (transport.js L44). A sweep drops sockets silent for 20 s during a match,
//! 150 s in the lobby (`SILENT_MATCH`/`SILENT_LOBBY`, server/src/index.js
//! L28; the "10 s" in NET.md "Relay" is stale).

use serde::{Deserialize, Serialize};

pub mod matchbuild;
pub mod playback;
pub mod replication;
pub mod session;
pub mod transport;
pub mod wire;

use self::wire::JVal;

/// Wire protocol revision — mirrors `PROTO = 1` (transport.js L5). The Rust
/// port reuses the same relay; the protocol does not change (PORT_MAP L71).
pub const PROTO: u8 = 1;

/// Owner tick rate: `TICK = 1/20` (netmatch.js L30). Owners simulate at full
/// frame rate and stream every 1/20 s.
pub const TICK_HZ: f32 = 20.0;

/// Relay member id: a 4-character base36 string minted by the room object
/// (`Math.random().toString(36).slice(2, 6).toUpperCase()`,
/// server/src/index.js L78) and carried verbatim in every frame
/// (`welcome.id`, `m|<from>|`, `s|<to>|`; transport.js L59/L65). Strings,
/// not numbers — PROTO v1 does not change.
pub type SessionId = String;

/// Identity of a squidkid on the wire: the roster index assigned by the host
/// (`nid`, session.js L272-287) and the relay session id of the player who
/// *owns* its simulation (`owner`, match.js L110 — `a.owner = r.owner`, a
/// member id string). Nobody else ever simulates someone else's squidkid
/// (NET.md "Ownership").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId {
    /// Roster slot id (`a.nid`).
    pub nid: u8,
    /// Session id of the owning player (the host owns the bots).
    pub owner: SessionId,
}

// ---------------------------------------------------------------------------
// Actor tick view (packActor / unpackActor, netmatch.js L723-757)
// ---------------------------------------------------------------------------

/// State bits packed into one `u32` per actor per tick — the `F` table
/// (netmatch.js L41-45, exported as `NET_FLAGS` at L825). Rust keeps the
/// same bit values so a future wire layer can emit the identical integer.
#[allow(missing_docs)]
pub mod flags {
    pub const ALIVE: u32 = 1 << 0;
    pub const SQUID: u32 = 1 << 1;
    pub const SUB: u32 = 1 << 2;
    pub const CLIMB: u32 = 1 << 3;
    pub const GROUNDED: u32 = 1 << 4;
    /// `groundTeam === 1` — **own team's** paint under the feet. The code is
    /// relative to the actor (`t - 1 === a.team ? 1 : 2`, actor.js L401),
    /// not an absolute team id (netmatch.js L731).
    pub const GT1: u32 = 1 << 5;
    /// `groundTeam === 2` — **enemy** paint (slows and hurts, actor.js L295).
    pub const GT2: u32 = 1 << 6;
    pub const CHARGING: u32 = 1 << 7;
    pub const ROLLING: u32 = 1 << 8;
    pub const STREAMING: u32 = 1 << 9;
    pub const DODGE: u32 = 1 << 10;
    pub const SUB_AIM: u32 = 1 << 11;
    pub const FIRING: u32 = 1 << 12;
    pub const SPECIAL: u32 = 1 << 13;
    pub const SJ_CHARGE: u32 = 1 << 14;
    pub const SJ_FLIGHT: u32 = 1 << 15;
    pub const FLICK: u32 = 1 << 16;
    pub const SLOSH: u32 = 1 << 17;
    pub const INVULN: u32 = 1 << 18;
    pub const ENEMY: u32 = 1 << 19;
}

/// One actor's network tick — the object `unpackActor` builds (netmatch.js
/// L753-755), with the same field order and semantics. Constructed from the
/// sim state by [`ActorSample::from_actor`]; TR-15.2 requires the mapping to
/// be lossless (full f32, no wire rounding).
///
/// Task 35: the special slots (`sp`, `spx`, `spst`) and `F.special` are live
/// (netmatch.js L738/L748/L750); `ks` remains the brolly-only kit pose.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ActorSample {
    /// Sender clock timestamp of the tick, seconds (`d.ts` → `s.t`; the JS
    /// clock is `performance.now()/1000`, the Rust layer will use its own
    /// monotonic seconds).
    pub t: f32,
    /// `pos.x` (JS `r2(a.pos.x)`).
    pub x: f32,
    /// Visual Y: `pos.y + smoothY` — the owner's smoothed feet height,
    /// *not* the raw physics Y (JS L745 `a.pos.y + (a.smoothY || 0)`; the
    /// Rust sim exposes it as [`crate::actor::Actor::visual_y`]).
    pub y: f32,
    /// `pos.z`.
    pub z: f32,
    /// `vel.x` (owner velocity — the Hermite tangents, NET.md "Path").
    pub vx: f32,
    /// `vel.y`.
    pub vy: f32,
    /// `vel.z`.
    pub vz: f32,
    /// Body yaw, radians (JS `r3(a.yaw)`).
    pub yaw: f32,
    /// Aim yaw (controller-written, [`crate::actor::Actor::aim_yaw`]).
    pub aim_yaw: f32,
    /// Aim pitch ([`crate::actor::Actor::aim_pitch`]).
    pub aim_pitch: f32,
    /// Packed [`flags`] word (`f: s[10]`).
    pub f: u32,
    /// Hit points (JS `Math.round(a.hp)`; M1 keeps full precision).
    pub hp: f32,
    /// Ink (JS `Math.round(a.ink)`).
    pub ink: f32,
    /// Special gauge `sp` (s[13], JS `Math.round(a.special)`): the raw gauge
    /// in turf points; the wire layer rounds at serialise time.
    pub sp: f32,
    /// Weapon charge `ch` (s[14], `wr.streaming ? burstFrac : charge`,
    /// linearly interpolated on playback; JS L748): Task 31 leaves every
    /// wired weapon non-streaming, so twins/blaster/brolly all pack 0 here.
    pub ch: f32,
    /// Turf credited to this actor (JS `Math.round(a.stats.turf)` →
    /// [`crate::actor::Actor::turf`]).
    pub turf: f32,
    /// Teleport counter (JS `a.netTp`, actor.js L169: bumped on respawn so
    /// proxies *cut* instead of gliding across the map, NET.md "Path").
    /// The Rust sim has no `netTp` field yet (M1 is offline); the counter is
    /// derived here from respawn events by the future transport layer, and
    /// [`ActorSample::from_actor`] takes it as a parameter.
    pub tp: u32,
    /// Wall normal x (JS `a.climbing ? a.wallN.x : 0`): M1 has no climb — 0.
    pub wx: f32,
    /// Wall normal y: M1 — 0.
    pub wy: f32,
    /// Wall normal z: packActor writes **0** when not climbing (JS L749
    /// `n ? r2(n.z) : 0`); M1 has no climb — 0. (The `1` you see in JS is
    /// only `blankSample`'s playback-side default, L757.)
    pub wz: f32,
    /// Weapon lock timer `lock` (s[20], `r2(wr.lockT)`): `lockT` is a
    /// dualies-only field (spec D-5) — twins/blaster/brolly all pack 0.
    pub lock: f32,
    /// Kit pose word `ks` (s[21], `MAIN_KITS[kind].netState(wr)`): the brolly
    /// canopy pose (`brolly_net_state`, brolly.js L636-641); every other kind
    /// — and every kit without a `netState` — packs 0.
    pub ks: u32,
    /// Special id index `spx` (s[22], JS `SPECIAL_ORDER.indexOf(a.specialId)
    /// + 1`, netmatch.js L750): the *equipped* special, 1-based; `0` for an
    /// unknown id (JS `-1 + 1`). The proxy restores `specialId` from it
    /// (applyRemote L379) so the `F.special` placeholder knows what to name.
    pub spx: f32,
    /// Special net state `spst` (s[23], `specialNetState(a)`, specials.js
    /// L2124-2130): the crab roll/firing/hull byte or the booyah charge
    /// 0..63; `0` for every other kind and for ghosts without a def.
    pub spst: u32,
}

impl ActorSample {
    /// Build the tick view from sim state — the `packActor` mapping
    /// (netmatch.js L723-751), lossless (no `r2`/`r3` rounding; that belongs
    /// to the wire layer). `teleport_count` is the `netTp` counter (see the
    /// [`ActorSample::tp`] field doc). `kind` is the actor's main weapon kind
    /// (JS `MAIN_KITS[a.weapon?.kind]?.netState?.(wr)`, L750): it selects the
    /// `ks` kit-pose slot (brolly only; spec D-1).
    #[must_use]
    pub fn from_actor(
        a: &crate::actor::Actor,
        wr: &crate::weapon::WeaponRunner,
        kind: crate::weapon::WeaponKind,
        teleport_count: u32,
        t: f32,
    ) -> Self {
        let mut f = 0u32;
        if a.alive {
            f |= flags::ALIVE;
        }
        if a.form == crate::actor::Form::Squid {
            f |= flags::SQUID;
        }
        if a.submerged {
            f |= flags::SUB;
        }
        // M1 sim has no wall climb (actor.rs module doc): CLIMB stays 0,
        // matching `a.climbing === false`.
        if a.grounded {
            f |= flags::GROUNDED;
        }
        match a.ground_team {
            1 => f |= flags::GT1,
            2 => f |= flags::GT2,
            _ => {}
        }
        // `firingT > 0` (JS L737) maps to a live flag. Task 31: the
        // charger/spinner spin-up window sets JS `wr.charging` (weapons.js
        // L189/L466) → `F.charging` (netmatch.js L732), and the bucket
        // wind-up sets `wr.flick >= 0` (weapons.js L444) → `F.flick`
        // (netmatch.js L740). Rolling/streaming/dodge/subAim/sj/slosh
        // states stay 0 for the wired kinds.
        if wr.firing() {
            f |= flags::FIRING;
        }
        if wr.charging_flag() {
            f |= flags::CHARGING;
        }
        if wr.flick_flag() {
            f |= flags::FLICK;
        }
        if a.invuln > 0.0 {
            f |= flags::INVULN;
        }
        if a.on_enemy {
            f |= flags::ENEMY;
        }
        // JS L738: any running special (including the net placeholder) sets
        // `F.special`; the proxy side reacts in `applyRemote` (L379-385).
        if a.special_active.is_some() {
            f |= flags::SPECIAL;
        }
        Self {
            t,
            x: a.pos.x,
            y: a.visual_y(),
            z: a.pos.z,
            vx: a.vel.x,
            vy: a.vel.y,
            vz: a.vel.z,
            yaw: a.yaw,
            aim_yaw: a.aim_yaw,
            aim_pitch: a.aim_pitch,
            f,
            hp: a.hp,
            ink: a.ink,
            // JS L748 `Math.round(a.special)` — full precision here, the
            // wire rounds at serialise time.
            sp: a.special,
            // JS L748 `wr.streaming ? wr.burstFrac : wr.charge` — `streaming`
            // is set only by the retired splatling (weapons.js L381-426), so
            // every wired kind (charger/spinner/bucket included) ships
            // `wr.charge` here.
            ch: if wr.streaming() {
                wr.burst_frac()
            } else {
                wr.charge()
            },
            turf: a.turf,
            tp: teleport_count,
            wx: 0.0,
            wy: 0.0,
            wz: 0.0,
            // JS L750 `r2(wr.lockT || 0)` — `lockT` is dualies-only (spec
            // D-5): twins/blaster/brolly read 0; the wire layer applies `r2`.
            lock: wr.lock_t(),
            // JS L750 `MAIN_KITS[a.weapon?.kind]?.netState?.(wr) | 0`
            // (brolly.js L636-641); no other wired kind has a `netState`.
            ks: if kind == crate::weapon::WeaponKind::Brolly {
                wr.brolly_net_state()
            } else {
                0
            },
            // JS L750: the *equipped* id's `SPECIAL_ORDER` index + 1
            // (unknown ⇒ `-1 + 1 = 0`).
            spx: a
                .special_id
                .as_deref()
                .map_or(0.0, |id| (crate::special::order_index(id) + 1) as f32),
            spst: crate::special::net_state(a),
        }
    }

    /// Decode the packed flag word back into booleans (the playback side of
    /// `unpackActor` + `hermite`, which reads `f & F.*`).
    #[must_use]
    pub fn flag(&self, bit: u32) -> bool {
        self.f & bit != 0
    }
}

// ---------------------------------------------------------------------------
// Tick & event timeline
// ---------------------------------------------------------------------------

/// The owner tick `{k:'t', ts, a:[…], e:[…], B?, c?}` (`_sendTick`, netmatch.js
/// L190-203). The host additionally rides `c` (its clock snapshot —
/// [`HostMsg::Clock`]) and `B` (boss snapshot, Task 41) on this frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    /// Sender clock time, `r3(now())`.
    pub ts: f32,
    /// `(nid, sample)` per actor the sender owns (JS: `!x.remote`; slot 0 of
    /// the packed array is the `nid` the playback side routes by —
    /// `byNid.get(s[0])`, netmatch.js L247).
    pub actors: Vec<(u8, ActorSample)>,
    /// Event records produced since the last tick, in order (`msg.e`); each
    /// carries its own sender-clock timestamp (JS `_rec` prefixes `r3(now())`,
    /// netmatch.js L107).
    pub events: Vec<TimedEvent>,
    /// Boss snapshot `msg.B` — the host rides `boss.pack(bossN % 10 === 0)`
    /// on every tick while a live boss sim exists (netmatch.js L195-196).
    /// `None` (every non-boss match) ⇒ the `B` key is **absent** on the wire,
    /// so turf/zone tick bytes are unchanged from the pre-Task-41 shape
    /// (PROTO v1 compat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<BossSnap>,
}

/// One live crablet inside a [`BossSnap`] — `pack`'s crab tuple
/// `[id, r2(x), r2(y), r2(z), r2(yaw), Math.round(hp)]` (boss.js L426).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BossCrab {
    /// Monotonic host id (`crabSeq++`, bossBrain.js L336).
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// Integer HP on the wire (`Math.round(c.hp)`, boss.js L426).
    pub hp: u32,
}

/// The boss snapshot `msg.B` — `Boss.pack(full)`'s heterogeneous array
/// (boss.js L422-431), decoded by `Boss.unpack` (L432-434). Index layout
/// (the wire order `encode_boss_snap`/`decode_boss_snap` must reproduce):
/// `[r3 bt, r2 x, r2 y, r2 z, r3 yaw, r2 vx, r2 vz, round(hp*10)/10, phase,
/// f, mv, anim, r2 turn, flashN, weakN, killer, crabs, round(maxHp), move?]`
/// — `f` bits: 1 stunned / 2 dead / 4 invuln / 8 visible (L424); `mv` is the
/// active move's `t0` or `-1` (L427); `move` (index 18) rides only on the
/// every-10th **full** snapshot while a move is live (L429, netmatch.js L196).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BossSnap {
    /// Boss clock time (index 0, `r3`), monotonic on screen (`_follow` L447).
    pub bt: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub vx: f32,
    pub vz: f32,
    /// One-decimal HP (`Math.round(hp * 10) / 10`, boss.js L427).
    pub hp: f32,
    /// Phase 0..3 (L427).
    pub phase: u32,
    /// State bits: 1 stunned, 2 dead, 4 invuln, 8 visible (L424).
    pub f: u32,
    /// Active move's `t0`, or `-1` when idle (L427-428).
    pub mv: f32,
    /// Set-piece anim: 0 none, 1 intro, 2 roar (boss.js L43, L428).
    pub anim: u32,
    /// Turn rate (`r2`, L428).
    pub turn: f32,
    /// Cumulative hit-flash counter (`flashN`, L428; guest compares at L455).
    pub flash: u32,
    /// Cumulative weak-hit counter (`weakN`, L428; guest compares at L456).
    pub weak: u32,
    /// Nid of the killer, `-1` while alive (L428; boss.js L42/L364).
    pub killer: i32,
    /// Live crablets (dead ones are skipped by `pack`, L426).
    pub crabs: Vec<BossCrab>,
    /// Integer max HP (`Math.round(maxHp)`, L428).
    pub max_hp: u32,
    /// The active move record `{id, t0, s, d, p}` (bossBrain.js L291) —
    /// present only on full snapshots (`s[18]`, L429); kept verbatim as
    /// [`JVal`] so its field order survives the round trip. (`move` is a
    /// Rust keyword; the JS name rides the serde rename.)
    #[serde(default, rename = "move")]
    pub move_rec: Option<JVal>,
}

/// Splat record `['s', x, y, z, radius, team, seed, kind, sx, sy, sz, amt]`
/// (`recSplat`, netmatch.js L109-114). Ink is replicated splat-for-splat
/// from whoever painted it, replayed with the seeded shape so every screen
/// shows the same turf (NET.md "Ink and hits"). Mirrors
/// [`crate::paint::InkSplat`] + [`crate::paint::SplatOpts`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplatRecord {
    /// Center x (`r2(c.x)`).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Radius (`r2(radius)`).
    pub radius: f32,
    /// Painting team (0/1).
    pub team: u8,
    /// Deterministic blob seed (`r3(o.seed)`) — the sim always passes one
    /// (`crate::paint::SplatOpts::seed`).
    pub seed: f32,
    /// Splat kind on the wire as its **name string** (`o.kind ?? 0`,
    /// netmatch.js L112; replayed through the JS `K` table, paint.js L35/
    /// L455, and re-set as a string at L470). `None` ⇒ JS `0` (kind inferred
    /// from radius/stretch on replay).
    pub kind: Option<String>,
    /// Stretch direction (JS `st ? r3(st.*) : 0` — zero when `None`).
    pub stretch: [f32; 3],
    /// Stretch amount (`r2(o.stretchAmt ?? 1)`, 0 when no stretch).
    pub stretch_amt: f32,
}

impl SplatRecord {
    /// Build the replication record from a sim splat — the `recSplat`
    /// mapping (netmatch.js L112-113): `None` stretch ⇒ zero direction and
    /// amount 0 (JS writes `0`, not `1`, when `st` is falsy); `None` kind
    /// ⇒ JS `0` (modelled as `None`).
    #[must_use]
    pub fn from_ink_splat(s: &crate::paint::InkSplat) -> Self {
        let o = &s.opts;
        Self {
            x: s.center.x,
            y: s.center.y,
            z: s.center.z,
            radius: s.radius,
            team: s.team as u8,
            seed: o.seed,
            kind: o.kind.map(|k| k.name().to_string()),
            stretch: o.stretch.map(|v| [v.x, v.y, v.z]).unwrap_or([0.0; 3]),
            stretch_amt: match o.stretch {
                Some(_) => o.stretch_amt.unwrap_or(1.0),
                None => 0.0,
            },
        }
    }
}

/// Animation trigger record `['tr', nid, name, data]` (netmatch.js L92,
/// packTrig L789-794). `name` is the character trigger name (a string; e.g.
/// `'spawn'`, `'fire'`), `data` a small payload.
///
/// The M1 "number-only" Deviation is closed by [`TrigData`], which models all
/// three `packTrig` branches (null / number / plain object) — see
/// [`crate::net::wire::pack_trig`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerRecord {
    pub nid: u8,
    pub name: String,
    /// The payload before `packTrig` (the raw trigger `data` argument).
    pub data: TrigData,
}

/// A `packTrig` payload (netmatch.js L789-794). Three shapes on the wire:
/// `null`/non-numeric ⇒ the bare token `0`; a number ⇒ `r3(d)`; a plain object
/// ⇒ a key-ordered map of its **number-valued** keys, each `r3`'d (used by the
/// dualies hand triggers `{amp}` / `{hand}`). The object variant keeps insertion
/// order so the wire bytes match JS `JSON.stringify`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrigData {
    /// Serialized as the bare token `0`.
    Zero,
    /// A number (`r3` applied on the wire).
    Num(f32),
    /// A plain object; `(key, value)` pairs in insertion order (only number
    /// values survive `packTrig`, so the model holds just those).
    Obj(Vec<(String, f32)>),
}

/// Forwarded gameplay event `['ev', name, packed]` (`_onLocalEvent`,
/// netmatch.js L139-143; the `FORWARD` list, L47). The packed payload maps
/// actors to `{n: nid}`, vectors to `[x,y,z]`, and passes numbers/strings/
/// booleans through (packEvent L803-813).
///
/// The M1 "name-only" Deviation is closed by [`EventRecord`], which pairs the
/// name with the verbatim packed payload ([`JVal`]) the replay side unpacks
/// per name (`_playEvent` L573-601).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForwardEvent {
    /// JS `'actor:jump'`.
    ActorJump,
    /// JS `'superjump'` — M1 sim: no super jump yet (actor.rs module doc).
    SuperJump,
    /// JS `'superjump:land'` — M1: none.
    SuperJumpLand,
    /// JS `'special:use'` — M1: no specials.
    SpecialUse,
    /// JS `'special:slam'` — M1: no specials.
    SpecialSlam,
    /// JS `'weapon:dodge'` — M1: Spritzer has no dodge.
    WeaponDodge,
    /// JS `'weapon:fire'` (drives ghost projectiles on replay, L598).
    WeaponFire,
    /// JS `'splatted'` (victim/attacker/cause, L580).
    Splatted,
    /// JS `'respawn'` (L581).
    Respawn,
}

impl ForwardEvent {
    /// The wire name from the `FORWARD` table (netmatch.js L47). This is the
    /// exact string `['ev', name, …]` carries.
    #[must_use]
    pub fn js_wire_name(&self) -> &'static str {
        match self {
            ForwardEvent::ActorJump => "actor:jump",
            ForwardEvent::SuperJump => "superjump",
            ForwardEvent::SuperJumpLand => "superjump:land",
            ForwardEvent::SpecialUse => "special:use",
            ForwardEvent::SpecialSlam => "special:slam",
            ForwardEvent::WeaponDodge => "weapon:dodge",
            ForwardEvent::WeaponFire => "weapon:fire",
            ForwardEvent::Splatted => "splatted",
            ForwardEvent::Respawn => "respawn",
        }
    }

    /// Parse a wire name back to the enum (`None` for names outside `FORWARD`).
    #[must_use]
    pub fn from_js_name(s: &str) -> Option<Self> {
        Some(match s {
            "actor:jump" => ForwardEvent::ActorJump,
            "superjump" => ForwardEvent::SuperJump,
            "superjump:land" => ForwardEvent::SuperJumpLand,
            "special:use" => ForwardEvent::SpecialUse,
            "special:slam" => ForwardEvent::SpecialSlam,
            "weapon:dodge" => ForwardEvent::WeaponDodge,
            "weapon:fire" => ForwardEvent::WeaponFire,
            "splatted" => ForwardEvent::Splatted,
            "respawn" => ForwardEvent::Respawn,
            _ => return None,
        })
    }
}

/// Forwarded gameplay event `['ev', name, packed]` — the name plus the
/// `packEvent(e)` object (netmatch.js L803-813) carried **verbatim** as a
/// [`JVal`]: the replay side (`_playEvent` L573-601) unpacks it per name, so
/// the transport layer never needs to interpret its heterogeneous contents.
/// The M1 "name-only" Deviation is closed here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub name: ForwardEvent,
    /// The packed payload (`{n: nid}` for actors, `[x,y,z]` for vectors,
    /// numbers/strings/booleans passed through), order-preserved on the wire.
    pub payload: JVal,
}

/// One element of a [`KitRecord`]'s payload — the JS `data` arrays are
/// *mixed*: numbers everywhere, plus strings at fixed indices (the subs kind
/// string at `data[2]`, subs.js L77/L103; the specials channel's reason /
/// object kind strings, specials.js L258/L268/L270/L296/L303/L314/L320).
/// The wire writes `Num` as a rounded number and `Str` as a JSON string, so
/// both shapes stay byte-identical to JS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum KitVal {
    /// A number (JS numeric array element).
    Num(f32),
    /// A string (JS string array element).
    Str(String),
}

impl KitVal {
    /// JS numeric read (`d[i] | 0` / `+d[i]`): `None` for a string element.
    #[must_use]
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Num(v) => Some(*v),
            Self::Str(_) => None,
        }
    }

    /// JS string read: `None` for a number (JS `typeof` guards).
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            Self::Num(_) => None,
        }
    }

    /// f64 view for the wire layer's rounding helpers.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        self.as_f32().map(f64::from)
    }

    /// Numeric read with a `0.0` default (JS `d[i] || 0`).
    #[must_use]
    pub fn num_or(&self, default: f32) -> f32 {
        self.as_f32().unwrap_or(default)
    }
}

/// Kit world-object spawn `['k', nid, kind, data]` (recKit, netmatch.js
/// L134-137): a fist/arrow/canopy/thrown sub replayed by the kit's
/// `ghost(actor, data)`, visual-only, plus the specials channel
/// (`kind == "sp"`, specials.js `rec` → `netGhost`, L267-276).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitRecord {
    pub nid: u8,
    /// Kit kind string (JS `kind`, e.g. `'mitts'` / `'sp'`).
    pub kind: String,
    /// Sub-system kind string for `kind == "subs"` records — JS packs the
    /// mixed array `['k', nid, 'subs', [op, gid, kindStr, …]]` (subs.js
    /// L77/L103); the wire layer writes it back at `data[2]` and the decode
    /// lifts it out again (the vector keeps the JS index layout with a
    /// `Num(0.0)` placeholder there). `None` for the main-weapon kits and
    /// the specials channel.
    #[serde(default)]
    pub sub_kind: Option<String>,
    /// The mixed payload the kit/special packs (JS `data`).
    pub data: Vec<KitVal>,
}

/// Zone Control decision record `['z', e]` (recZone, netmatch.js L148): the
/// host puts every rules decision (capture / control / penalty / rotation /
/// overtime / end, plus a count snapshot twice a second) on its event
/// timeline so they land in step with the paint that caused them (NET.md
/// "Zone Control"). M1 is Turf War only — placeholder shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneRecord {
    /// The zones.js `netEvent` payload: a **mixed array** — a 2-letter tag
    /// string first, then numbers (`['zz', zoneId, team]`, `['zo', o]`,
    /// `['zp', t, p, start, end]`, `['zs', …counts…]`, `['za', i, final]`,
    /// `['zf']`, `['zt', L]`, `['ze', winner, reason, …]`; zones.js
    /// L193-L365). The wire layer carries the raw JS array verbatim as a
    /// [`JVal`] (recZone wraps it as `['z', e]`, netmatch.js L148), so the
    /// M1 "strings-only" placeholder is closed here.
    pub payload: JVal,
}

/// Projectile ghost record `['p', nid, type, wid, pos, vel, delay, life,
/// straight, radius, size, grav, drag, trailEvery, head, vis, tail0, tailK,
/// wob, wobF, nose, sats]` (recProj, netmatch.js L116-122): a visual-only copy
/// of someone else's shot — it never paints or damages (NET.md "Ink and
/// hits"). M1 fires Spritzer rounds through [`crate::weapon::ProjectileSim`];
/// the record carries the full 26-slot PROTO v1 shape (the M1 "subset"
/// Deviation is closed by [`crate::net::wire::encode_record`]). Slots the JS
/// packer passes through unrounded (`grav`/`drag`/`trailEvery`/`tail0`/
/// `tailK`/`wob`/`wobF`/`nose`/`sats`) keep the JS defaults resolved by the
/// caller (`vis ?? 0.1`, `tail0 ?? 0.8`, `tailK ?? 1.3`, `wob ?? 0.035`,
/// `wobF ?? 26`, `nose ?? 0.3`, `sats ?? 3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectileRecord {
    /// Owner nid (`o.nid`).
    pub nid: u8,
    /// Projectile type on the wire as its **name string** (`p.type`:
    /// `'shot' | 'slosh' | 'blast' | 'drop'`, weapons.js L1034/L1132/L1178/
    /// L1226 — JS assigns the string literal, not an id).
    pub ptype: String,
    /// Weapon id (`p.wid || 0`): the weapon's string id (`w.id`, weapons.js
    /// L1073); `None` ⇒ JS `0` (projectiles with no `wid`).
    pub wid: Option<String>,
    /// Spawn position `r2(p.pos.*)`.
    pub pos: [f32; 3],
    /// Spawn velocity `r2(p.vel.*)`.
    pub vel: [f32; 3],
    /// `r3(p.delay || 0)` — fuse before the shot becomes visible.
    pub delay: f32,
    /// `r3(p.life)` — ballistic lifetime the ghost integrates.
    pub life: f32,
    /// `r3(p.straight)` — straight-line blend.
    pub straight: f32,
    /// `r2(p.radius)` — paint radius (visual only on ghosts).
    pub radius: f32,
    /// `r2(p.size)` — ghost mesh scale.
    pub size: f32,
    /// `p.grav` — gravity, passed through unrounded.
    pub grav: f32,
    /// `p.drag` — air drag, passed through unrounded.
    pub drag: f32,
    /// `p.trailEvery || 0` — trail-spawn cadence, passed through.
    pub trail_every: f32,
    /// `p.head ? 1 : 0` — the projectile's head sprite on the wire as 1/0.
    pub head: bool,
    /// `r3(p.vis ?? 0.1)` — visibility blend.
    pub vis: f32,
    /// `p.tail0 ?? 0.8` — trail length start, passed through.
    pub tail0: f32,
    /// `p.tailK ?? 1.3` — trail length curve, passed through.
    pub tail_k: f32,
    /// `p.wob ?? 0.035` — wobble amplitude, passed through.
    pub wob: f32,
    /// `p.wobF ?? 26` — wobble frequency, passed through.
    pub wob_f: f32,
    /// `p.nose ?? 0.3` — nose offset, passed through.
    pub nose: f32,
    /// `p.sats ?? 3` — satellite count, passed through.
    pub sats: f32,
}

/// Bomb record `['b', nid, kind, pos, vel]` (recBomb, netmatch.js L124-128).
/// `kind` is the bomb's string kind (`'bomb' | 'storm'`, weapons.js
/// L1461/L1477). M1: no thrown bombs — placeholder shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BombRecord {
    pub nid: u8,
    pub kind: String,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
}

/// One entry on the owner's event timeline (`this.out`, netmatch.js L107:
/// every record is prefixed with its `r3(now())` timestamp so playback
/// replays it on the sender's timeline, in step with the actor samples).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedEvent {
    /// Record timestamp on the sender clock.
    pub ts: f32,
    pub kind: TickEvent,
}

/// The event variants that ride the tick (`Tick::events`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TickEvent {
    /// Ink splat replication (JS `'s'`).
    Splat(SplatRecord),
    /// Ghost projectile (JS `'p'`).
    Proj(ProjectileRecord),
    /// Ghost bomb (JS `'b'`) — M1 placeholder.
    Bomb(BombRecord),
    /// Animation trigger (JS `'tr'`).
    Trigger(TriggerRecord),
    /// Forwarded gameplay event (JS `'ev'`) — name + verbatim packed payload.
    Event(EventRecord),
    /// Kit object spawn (JS `'k'`) — M1 placeholder.
    Kit(KitRecord),
    /// Zone Control decision (JS `'z'`) — M1 placeholder (Turf War only).
    Zone(ZoneRecord),
    /// Boss move record `['bm', rec]` (recBoss, bossBrain.js L294; host
    /// timeline; guests feed it to `Boss.onMove` — netmatch.js L494,
    /// boss.js L436). `rec` is the `{id, t0, s, d, p}` object (bossBrain.js
    /// L291) carried **verbatim** as [`JVal`] so its field order survives the
    /// round trip (the hazards replay reads it by name, like JS `hz.add`).
    BossMove(JVal),
    /// Crablet burst `['bc', id, x, y, z, killed?1:0]` (boss.js L386; guests
    /// pop the crab at the position — netmatch.js L496 `_crabBurst`).
    BossCrabBurst {
        id: u32,
        x: f32,
        y: f32,
        z: f32,
        /// `!!e[6]` on the guest side (netmatch.js L496).
        killed: bool,
    },
}

// ---------------------------------------------------------------------------
// Point-to-point messages (bypass the playback timeline)
// ---------------------------------------------------------------------------

/// Direct peer→peer messages sent via `s|to|` (never queued on the tick
/// timeline). Hits are decided by the shooter's screen and applied by the
/// victim's owner (NET.md "Ink and hits").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DirectMsg {
    /// `{k:'hit', v, a, d, w}` — shooter-authoritative damage (sendHit,
    /// netmatch.js L163-167). Applied by the victim's owner.
    Hit {
        /// Victim nid.
        v: u8,
        /// Attacker nid.
        a: u8,
        /// Damage (`r2(dmg)`).
        d: f32,
        /// Weapon id on the wire as a **string** (`wid` — every JS caller
        /// passes one: `'roller'`, `'bomb'`, `p.weaponId || p.wid || p.type`;
        /// weapons.js L265/L1504/L1608, forwarded verbatim at L634). `None`
        /// ⇒ the key is absent on the wire.
        w: Option<String>,
    },
    /// `{k:'dh', kind, id, d}` — a hit landed locally on a *ghost* device;
    /// forwarded to the device's owner, whose real copy takes it (sendDevHit
    /// L157-160 → `netHurt`). M1: no kit devices — placeholder shape.
    DeviceHit { kind: String, id: u32, d: f32 },
    /// `{k:'bhit', a, d, weak, w, c}` — a guest's hit on the boss (or a
    /// crablet), applied by the host that runs it (sendBossHit L150-154).
    /// M1: no boss — placeholder shape.
    BossHit {
        a: u8,
        d: f32,
        /// On the wire as `1`/`0` (`weak ? 1 : 0`, L153), not a JSON bool.
        weak: bool,
        /// Weapon id string (`wid`; `'crab'` for crablet hits, boss.js
        /// L296/L374).
        w: String,
        /// Crablet id, `-1` for boss hits (L150 default). Raw roster integer
        /// on the wire (`c: crab`, L153) — widened from `i8` because
        /// `crabSeq++` (bossBrain.js L336) is unbounded and 255 would alias
        /// the `-1` body sentinel.
        c: i32,
    },
}

/// Host-broadcast authoritative messages (`b|`). The host runs the bots, the
/// match clock and the final judge (NET.md intro); guests obey these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostMsg {
    /// `{k:'st', s, t}` — match state + time (netmatch.js L80, host-only).
    /// Guests apply it in `_hostState` (L644-649): the time is **hard-set**
    /// (`m.time = d.t`) and the state is re-entered on change. This is
    /// distinct from [`HostMsg::Clock`], the gentle per-tick steering.
    State {
        /// JS `match.state` string (`'playing'` etc.); Rust [`crate::match_::Phase`].
        phase: String,
        /// `d.t` — `None` when the frame carries no `t` key; JS guards the
        /// clock write with `typeof d.t === 'number'` (L647), so a missing
        /// `t` must leave the guest clock untouched.
        t: Option<f32>,
    },
    /// The host clock snapshot riding the owner tick (`msg.c = [state,
    /// r2(time)]`, `_sendTick` L197-200, every 0.5 s), consumed by
    /// `_hostClock` (L639-643): while both sides are `playing` and the local
    /// clock is off by >0.2 s, nudge it halfway (`m.time += (t - m.time) *
    /// 0.5`). Not a standalone wire message — the host's [`Tick`] carries it
    /// as its `c` field; modelled as a variant so the type exists here.
    Clock { phase: String, t: f32 },
    /// `{k:'res', cov, win, mode?, bo?, st:[…]}` — the final result
    /// (sendResult, netmatch.js L650-655). The host's final count is the
    /// result on every screen (NET.md "Ink and hits"). Zone Control adds the
    /// `mode:'zones'` plus `zc/zp/zr/zo/zl` extras (JS L652-653, Task 36
    /// batch 3); Boss Battle adds `mode:'boss'` + `bo` (Task 41).
    Result {
        /// `result.coverage` fractions `[team0, team1]`.
        coverage: [f32; 2],
        /// `result.winner` team index.
        winner: u8,
        /// Per-actor stat rows `[nid, turf, splats, deaths, bossDmg,
        /// weakHits]` (JS L654 — always 6 wide; `_result` L659 tolerates
        /// short rows with 0 defaults).
        stats: Vec<ActorStatsRow>,
        /// Zone Control settlement (`mode:'zones'` + `zc/zp/zr/zo/zl`, JS
        /// L652-653); `None` = turf — JS drops the `undefined` `mode`/`bo`
        /// keys from `JSON.stringify`, so the turf wire is byte-identical
        /// to the pre-Task-36 shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zones: Option<ZonesResult>,
        /// Boss settlement `bo = result.boss` — `{name, win, time, hp, maxHp,
        /// phase}` (bossMode.js L50), carried verbatim as [`JVal`] (JS
        /// `_result` L661 rebuilds it without inspecting fields). `Some` ⇒
        /// `mode:'boss'` on the wire (the two always travel together, JS
        /// L652); sits between `mode` and the zones extras, matching the JS
        /// object-literal order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bo: Option<JVal>,
    },
    /// `{k:'end'}` — results shown, everyone returns to the lobby (sendEnd,
    /// netmatch.js L666).
    End,
    /// `{k:'own', map}` — ownership re-assignment map. **Reserved:** the
    /// receive case exists (`onMessage` L219 → `_ownership(d.map)`), but
    /// `_ownership` is an empty stub (`/* reserved: explicit transfers */`,
    /// netmatch.js L709) and the host **never sends** `{k:'own'}`. Adoption
    /// after a host leaves happens locally in `_adopt` (L692-707), which
    /// continues the teleport counter (`a.netTp = a.net.tp || 0`, L702).
    /// Shape kept so the wire contract stays complete.
    Ownership {
        /// `nid → owner session id` map.
        map: Vec<(u8, SessionId)>,
    },
}

/// One row of the result stats array (JS L654).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorStatsRow {
    pub nid: u8,
    /// `Math.round(a.stats.turf)`.
    pub turf: i32,
    pub splats: u32,
    pub deaths: u32,
    /// `Math.round(a.stats.bossDmg || 0)` (JS L654; Task 41). `0` on every
    /// non-boss row — `_result` L659 defaults short rows to 0.
    #[serde(default)]
    pub boss_dmg: i32,
    /// `a.stats.weakHits || 0` (JS L654; Task 41).
    #[serde(default)]
    pub weak_hits: u32,
}

/// The Zone Control extras on the host result wire (`sendResult` L652-653:
/// `zc` counts, `zp` penalty, `zr` reason, `zo` overtime `1/0`, `zl` log).
/// Guests build their [`crate::match_::MatchResult::zones`] verbatim from
/// these — JS `_result` L662 does exactly that (`counts: d.zc, penalty:
/// d.zp, reason: d.zr, overtime: !!d.zo, log: d.zl || []`), so the host's
/// count is the settlement on every screen (no guest-local replay drift).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZonesResult {
    /// `result.counts` — `[Math.ceil(count0), Math.ceil(count1)]`
    /// (match.js L257).
    pub counts: [i64; 2],
    /// `result.penalty` — the raw per-team penalty (JS `[...Z.penalty]`).
    pub penalty: [f32; 2],
    /// `result.reason` (JS `Z.reason`; `None` = the key is absent on the
    /// wire, like `undefined` in `JSON.stringify`).
    pub reason: Option<String>,
    /// `result.overtime` (wire `zo: 1/0`, JS `Z.overtime`).
    pub overtime: bool,
    /// `result.log` — the penalty log entries (wire `zl`, JS `Z.log`).
    pub log: Vec<crate::match_::ZonePenaltyLog>,
}

// ---------------------------------------------------------------------------
// Relay control frames (transport.js / server/src/index.js)
// ---------------------------------------------------------------------------

/// JSON control frames exchanged with the room relay (the relay itself only
/// routes `b|`/`s|`/`m|` envelopes and answers `ping`; server/src/index.js
/// L111-127). Shapes mirror transport.js L63-66 and server/src/index.js
/// L14-16 & L81-83. All ids are [`SessionId`] strings (base36, L78).
///
/// `Eq` is intentionally not derived: `Pong.c` is an f64 timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlFrame {
    /// `{"t":"welcome","id","host","members":[{id,name}]}` — sent on connect;
    /// resolves `Transport.connect` (transport.js L65; server L81).
    Welcome {
        id: SessionId,
        host: SessionId,
        members: Vec<(SessionId, String)>,
    },
    /// `{"t":"join","m":{id,name}}` — fan-out when a member joins (server
    /// L82-83; session.js L146).
    Join { id: SessionId, name: String },
    /// `{"t":"leave","id","host"}` — member dropped; `host` is the current
    /// oldest-member id (re-elected when the old host left; server L139-141;
    /// session.js L153-157).
    Leave { id: SessionId, host: SessionId },
    /// `{"t":"err","e":"…"}` — join refusal (`Room not found` / `Room is
    /// full` / `Match in progress`), then the socket closes (server L66-73).
    Err { e: String },
    /// `{"t":"pong","c"}` — RTT echo (server L125). `c` is the client's
    /// `performance.now()` — a float millisecond stamp, not an integer.
    Pong { c: f64 },
    /// `{"t":"lock","v"}` — host-only: refuse new joins while a match runs
    /// (transport.js L102; server L126).
    Lock { v: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Actor, Form, SimWorld};
    use crate::collision::CollisionWorld;
    use crate::geometry::StageLayout;
    use crate::match_::Match;
    use crate::weapon::{ProjectileSim, WeaponRunner};

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
        fn sim(&self) -> SimWorld<'_> {
            SimWorld::new(&self.world, &self.layout)
        }
    }

    #[test]
    fn flags_match_upstream_table() {
        // netmatch.js L41-45: exact bit values, all 20 entries.
        assert_eq!(flags::ALIVE, 1);
        assert_eq!(flags::SQUID, 2);
        assert_eq!(flags::SUB, 4);
        assert_eq!(flags::CLIMB, 8);
        assert_eq!(flags::GROUNDED, 16);
        assert_eq!(flags::GT1, 32);
        assert_eq!(flags::GT2, 64);
        assert_eq!(flags::CHARGING, 128);
        assert_eq!(flags::ROLLING, 256);
        assert_eq!(flags::STREAMING, 512);
        assert_eq!(flags::DODGE, 1024);
        assert_eq!(flags::SUB_AIM, 2048);
        assert_eq!(flags::FIRING, 4096);
        assert_eq!(flags::SPECIAL, 8192);
        assert_eq!(flags::SJ_CHARGE, 16384);
        assert_eq!(flags::SJ_FLIGHT, 32768);
        assert_eq!(flags::FLICK, 65536);
        assert_eq!(flags::SLOSH, 131072);
        assert_eq!(flags::INVULN, 262144);
        assert_eq!(flags::ENEMY, 524288);
    }

    #[test]
    fn sample_is_built_losslessly_from_sim_state() {
        let f = fix();
        let t = &f.tuning.player;
        let mut a = Actor::new(0, 0, t);
        // Drive the actor into a distinctive state (all public fields).
        a.pos = glam::vec3(1.234_567_8, 2.0, -3.456_789);
        a.vel = glam::vec3(0.5, -9.81, 1.5);
        a.yaw = 0.712_345_6;
        a.aim_yaw = -1.1;
        a.aim_pitch = 0.33;
        a.form = Form::Squid;
        a.submerged = true;
        a.grounded = true;
        a.ground_team = 2;
        a.on_enemy = true;
        a.hp = 61.25;
        a.ink = 80.0;
        a.turf = 4096.5;
        a.invuln = 1.0;

        // The firing flag comes from the real runner path (JS `wr.firingT > 0`):
        // one gated Spritzer update opens the muzzle-flash window.
        let mut wr = WeaponRunner::new();
        let gate = crate::actor::FireGate {
            fire: true,
            ..Default::default()
        };
        let mut proj = ProjectileSim::new(1);
        let mut pg = crate::paint::PaintGrid::new(&f.world);
        let mut foes = crate::weapon::FoeScan::empty();
        wr.update(
            crate::actor::FIXED_DT,
            &gate,
            &mut a,
            crate::weapon::WeaponKind::Shooter,
            &f.tuning.weapons,
            &f.tuning.subs,
            glam::Vec3::ZERO,
            &f.world,
            &mut proj,
            &mut foes,
            &mut pg,
            t,
            None,
            false,
            0.0,
            &mut Vec::new(),
            &mut Vec::new(),
            false,
            false,
        );
        assert!(wr.firing(), "a shot must open the firing window");

        let s = ActorSample::from_actor(&a, &wr, crate::weapon::WeaponKind::Shooter, 7, 12.345);
        // Bit-exact float mapping (TR-15.2 "lossless").
        assert_eq!(s.x, a.pos.x);
        assert_eq!(s.y, a.visual_y());
        assert_eq!(s.z, a.pos.z);
        assert_eq!((s.vx, s.vy, s.vz), (a.vel.x, a.vel.y, a.vel.z));
        assert_eq!(
            (s.yaw, s.aim_yaw, s.aim_pitch),
            (a.yaw, a.aim_yaw, a.aim_pitch)
        );
        assert_eq!(s.hp, a.hp);
        assert_eq!(s.ink, a.ink);
        assert_eq!(s.turf, a.turf);
        assert_eq!(s.tp, 7);
        assert_eq!(s.t, 12.345);
        // Flag word mirrors the JS `F` composition (netmatch.js L726-743).
        assert!(s.flag(flags::ALIVE));
        assert!(s.flag(flags::SQUID));
        assert!(s.flag(flags::SUB));
        assert!(s.flag(flags::GROUNDED));
        assert!(s.flag(flags::GT2));
        assert!(!s.flag(flags::GT1));
        assert!(s.flag(flags::ENEMY));
        assert!(s.flag(flags::INVULN));
        assert!(s.flag(flags::FIRING));
        assert!(!s.flag(flags::CLIMB), "M1 has no wall climb");
        // M1-constant slots (packActor writes 0 for all of these in M1).
        assert_eq!(s.sp, 0.0);
        assert_eq!(s.ch, 0.0);
        assert_eq!(s.lock, 0.0);
        assert_eq!(s.ks, 0);
        assert_eq!(s.wz, 0.0, "packActor: not climbing ⇒ 0 (JS L749)");
    }

    #[test]
    fn sample_survives_serde_round_trip() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let w = &f.tuning.weapons;
        let sb = &f.tuning.subs;
        let mut m = Match::with_roster(180.0, 42, &sim, t, &f.tuning.match_config, 2);
        // Step past the intro (4.2 s) with the trigger held so at least one
        // runner is firing and positions/velocities are non-trivial.
        let mut inputs = vec![crate::actor::ActorInput::default(); 4];
        for _ in 0..340 {
            inputs[0].fire = m.playing();
            m.step(crate::actor::FIXED_DT, &inputs, &sim, t, w, sb, None);
        }
        let mut firing_seen = 0;
        for (i, (a, wr)) in m.actors.iter().zip(&m.runners).enumerate() {
            let s = ActorSample::from_actor(a, wr, m.kinds[i], i as u32, m.time);
            firing_seen += usize::from(s.flag(flags::FIRING));
            let json = serde_json::to_string(&s).unwrap();
            let back: ActorSample = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back, "actor {i} must round-trip bit-exact");
        }
        assert!(firing_seen > 0, "a held trigger must show FIRING");
    }

    #[test]
    fn tick_and_messages_survive_serde_round_trip() {
        let f = fix();
        let sim = f.sim();
        let t = &f.tuning.player;
        let m = Match::with_roster(180.0, 7, &sim, t, &f.tuning.match_config, 2);
        let actors = m
            .actors
            .iter()
            .zip(&m.runners)
            .enumerate()
            .map(|(i, (a, wr))| (i as u8, ActorSample::from_actor(a, wr, m.kinds[i], 0, 0.0)))
            .collect();
        let tick = Tick {
            ts: 1.5,
            actors,
            events: vec![
                TimedEvent {
                    ts: 1.4,
                    kind: TickEvent::Splat(SplatRecord {
                        x: 1.0,
                        y: 0.0,
                        z: -2.0,
                        radius: 0.9,
                        team: 1,
                        seed: 0.123,
                        kind: None,
                        stretch: [0.0, 0.0, 0.0],
                        stretch_amt: 0.0,
                    }),
                },
                TimedEvent {
                    ts: 1.45,
                    kind: TickEvent::Event(EventRecord {
                        name: ForwardEvent::Splatted,
                        payload: JVal::Obj(vec![("cause".into(), JVal::Str("splat".into()))]),
                    }),
                },
                TimedEvent {
                    ts: 1.46,
                    kind: TickEvent::Trigger(TriggerRecord {
                        nid: 3,
                        name: "fire".into(),
                        data: TrigData::Num(0.5),
                    }),
                },
                // Task 41: the boss records ride the same timeline.
                TimedEvent {
                    ts: 1.47,
                    kind: TickEvent::BossMove(JVal::Obj(vec![
                        ("id".into(), JVal::Str("slam".into())),
                        ("t0".into(), JVal::Num(1.4)),
                    ])),
                },
                TimedEvent {
                    ts: 1.48,
                    kind: TickEvent::BossCrabBurst {
                        id: 4,
                        x: 1.23,
                        y: 0.0,
                        z: -2.34,
                        killed: true,
                    },
                },
            ],
            b: Some(BossSnap {
                bt: 12.345,
                x: 1.5,
                y: 0.0,
                z: -2.5,
                yaw: 0.7,
                vx: 0.5,
                vz: -0.5,
                hp: 1234.5,
                phase: 2,
                f: 8,
                mv: -1.0,
                anim: 0,
                turn: 0.12,
                flash: 7,
                weak: 3,
                killer: -1,
                crabs: vec![BossCrab {
                    id: 4,
                    x: 1.0,
                    y: 0.0,
                    z: -2.0,
                    yaw: 1.5,
                    hp: 23,
                }],
                max_hp: 14400,
                move_rec: None,
            }),
        };
        let json = serde_json::to_string(&tick).unwrap();
        assert_eq!(tick, serde_json::from_str::<Tick>(&json).unwrap());

        let direct = vec![
            DirectMsg::Hit {
                v: 2,
                a: 0,
                d: 12.5,
                w: Some("spritzer".into()),
            },
            DirectMsg::DeviceHit {
                kind: "curtain".into(),
                id: 9,
                d: 4.0,
            },
            DirectMsg::BossHit {
                a: 1,
                d: 20.0,
                weak: true,
                w: "crab".into(),
                c: 3,
            },
        ];
        let json = serde_json::to_string(&direct).unwrap();
        assert_eq!(
            direct,
            serde_json::from_str::<Vec<DirectMsg>>(&json).unwrap()
        );

        let host = vec![
            HostMsg::State {
                phase: "playing".into(),
                t: Some(90.0),
            },
            HostMsg::Clock {
                phase: "playing".into(),
                t: 89.75,
            },
            HostMsg::Result {
                coverage: [0.61, 0.39],
                winner: 0,
                stats: vec![ActorStatsRow {
                    nid: 0,
                    turf: 120,
                    splats: 2,
                    deaths: 1,
                    boss_dmg: 4500,
                    weak_hits: 12,
                }],
                zones: None,
                bo: Some(JVal::Obj(vec![(
                    "name".into(),
                    JVal::Str("HULLBREAKER".into()),
                )])),
            },
            HostMsg::End,
            HostMsg::Ownership {
                map: vec![(4, "A1B2".into()), (5, "A1B2".into())],
            },
        ];
        let json = serde_json::to_string(&host).unwrap();
        assert_eq!(host, serde_json::from_str::<Vec<HostMsg>>(&json).unwrap());

        let frames = vec![
            ControlFrame::Welcome {
                id: "K3XZ".into(),
                host: "A1B2".into(),
                members: vec![("A1B2".into(), "a".into()), ("K3XZ".into(), "b".into())],
            },
            ControlFrame::Join {
                id: "Q7M1".into(),
                name: "c".into(),
            },
            ControlFrame::Leave {
                id: "Q7M1".into(),
                host: "A1B2".into(),
            },
            ControlFrame::Err {
                e: "Room is full".into(),
            },
            ControlFrame::Pong { c: 12345.75 },
            ControlFrame::Lock { v: true },
        ];
        let json = serde_json::to_string(&frames).unwrap();
        assert_eq!(
            frames,
            serde_json::from_str::<Vec<ControlFrame>>(&json).unwrap()
        );
    }

    #[test]
    fn splat_record_mirrors_paint_splat() {
        // The record must be constructible from a sim `InkSplat` without
        // loss of the fields the replay needs (seeded shape, stretch).
        use crate::paint::{InkSplat, SplatOpts};
        let splat = InkSplat {
            center: glam::vec3(1.0, 2.0, 3.0),
            radius: 1.25,
            team: 1,
            opts: SplatOpts {
                seed: 0.75,
                stretch: Some(glam::vec3(0.5, 0.0, 0.5)),
                stretch_amt: Some(1.5),
                kind: Some(crate::paint::SplatKind::Blast),
            },
            seq: 1,
            replayed: false,
        };
        let rec = SplatRecord::from_ink_splat(&splat);
        assert_eq!(rec.x, 1.0);
        assert_eq!(rec.y, 2.0);
        assert_eq!(rec.z, 3.0);
        assert_eq!(rec.radius, 1.25);
        assert_eq!(rec.team, 1);
        assert_eq!(rec.seed, 0.75);
        assert_eq!(rec.kind, Some("blast".into()));
        assert_eq!(rec.stretch, [0.5, 0.0, 0.5]);
        assert_eq!(rec.stretch_amt, 1.5);
        let json = serde_json::to_string(&rec).unwrap();
        assert_eq!(rec, serde_json::from_str::<SplatRecord>(&json).unwrap());

        // No stretch ⇒ JS writes 0 for direction *and* amount (L113).
        let bare = InkSplat {
            center: glam::Vec3::ZERO,
            radius: 0.5,
            team: 0,
            opts: SplatOpts {
                seed: 0.25,
                ..Default::default()
            },
            seq: 2,
            replayed: false,
        };
        let rec = SplatRecord::from_ink_splat(&bare);
        assert_eq!(rec.stretch, [0.0; 3]);
        assert_eq!(rec.stretch_amt, 0.0);
        assert_eq!(rec.kind, None, "JS `o.kind ?? 0`");
    }
}
