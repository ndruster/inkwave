// Task 17 — net wire gold-standard fixtures.
//
// Byte-exact PROTO v1 samples captured from the **real upstream packers** so
// the Rust `inkwave_sim::net::wire` codec can be pinned to them. To keep the
// fork's "zero changes to the JS tree" rule (spec Constraints), we do NOT edit
// `src/net/netmatch.js` to export its private helpers; instead we read the
// file, slice out the exact source of the packing functions, and `eval` them
// with the same module-scope constants they close over (`r2`, `r3`, `F`,
// `SPECIAL_ORDER`, a `MAIN_KITS`/`specialNetState` stub that reproduces the
// M1 "no kits/specials ⇒ 0" path). The sliced text is asserted against
// upstream on every run, so a drift in the packers fails the generator.
//
// Inputs are **exact decimals** (no Math.fround): the Rust side stores these
// as f32 and re-interpretation through the shortest round-trip decimal (see
// `wire::dec`) recovers the identical f64 the JS packers operate on, so the
// emitted `wire` strings are `JSON.stringify` of the real packer output — the
// ground truth Rust must reproduce byte-for-byte. `unpacked` is the real
// `unpackActor`/`unpackTrig`/`unpackEvent` output, the ground truth Rust
// decode must match.
//
// Run: `node rust/tools/net-fixtures/gen.mjs` → `rust/assets/net-fixtures.json`
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { SPECIAL_ORDER } from '../../../src/config.js';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..');
const netmatchSrc = readFileSync(join(repoRoot, 'src', 'net', 'netmatch.js'), 'utf8');
const sessionSrc = readFileSync(join(repoRoot, 'src', 'net', 'session.js'), 'utf8');
const transportSrc = readFileSync(join(repoRoot, 'src', 'net', 'transport.js'), 'utf8');
const serverSrc = readFileSync(join(repoRoot, 'server', 'src', 'index.js'), 'utf8');

// --- slice the real packers out of netmatch.js -----------------------------
// Each entry: [exportedName, startMarker, endMarker]. We take the substring
// from the start marker up to (not including) the end marker and assert it is
// found, so the slice can't silently grab the wrong region.
function slice(name, start, end) {
  const i = netmatchSrc.indexOf(start);
  if (i < 0) throw new Error(`slice(${name}): start marker not found: ${start}`);
  const j = netmatchSrc.indexOf(end, i);
  if (j < 0) throw new Error(`slice(${name}): end marker not found after start: ${end}`);
  return netmatchSrc.slice(i, j);
}

const srcR2 = slice('r2', 'const r2 =', '\n');
const srcR3 = slice('r3', 'const r3 =', '\n');
// the F table spans several lines; grab up to the closing brace of the object.
const srcF = slice('F', 'const F = {', 'invuln: 262144, enemy: 524288,\n};') + 'invuln: 262144, enemy: 524288,\n};';
const srcPackActor = slice('packActor', 'function packActor(a) {', '\nfunction unpackActor');
const srcUnpackActor = slice('unpackActor', 'function unpackActor(s, ts) {', '\nfunction blankSample');
const srcBlankSample = slice('blankSample', 'function blankSample()', '\nfunction copySample');
const srcCopySample = slice('copySample', 'function copySample(s, o)', '\n');
const srcPackTrig = slice('packTrig', 'function packTrig(d) {', '\nfunction unpackTrig');
const srcUnpackTrig = slice('unpackTrig', 'function unpackTrig(d) {', '\nfunction packEvent');
const srcPackEvent = slice('packEvent', 'function packEvent(e) {', '\nfunction unpackEvent');
const srcUnpackEvent = slice('unpackEvent', 'function unpackEvent(d, nm) {', '\nexport {');

// `recSplat`/`recProj` bodies are methods on the class; we inline the exact
// record-construction expressions they build (netmatch.js L112-121). They are
// pure given (c, radius, team, o) / (p), so we reproduce them as standalone
// functions and assert the source fragments still match upstream.
function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`expected fragment not found in ${where}: ${needle}`);
}
// record constructors (recSplat / recProj) — sliced via the packers above; the
// remaining hand-replicated records and JSON messages are anchored here so any
// drift in upstream's wire shapes fails the generator (and thus the build).
assertContains(netmatchSrc, "['s', r2(c.x), r2(c.y), r2(c.z), r2(radius), team, r3(o.seed ?? Math.random()), o.kind ?? 0,");
assertContains(netmatchSrc, "['p', o.nid, p.type, p.wid || 0, r2(p.pos.x), r2(p.pos.y), r2(p.pos.z), r2(p.vel.x), r2(p.vel.y), r2(p.vel.z),");
assertContains(netmatchSrc, "['b', o.nid, b.kind, r2(b.pos.x), r2(b.pos.y), r2(b.pos.z), r2(b.vel.x), r2(b.vel.y), r2(b.vel.z)]");
assertContains(netmatchSrc, "['k', a.nid, kind, data]");
assertContains(netmatchSrc, "['z', e]");
assertContains(netmatchSrc, "['ev', name, packEvent(e)]");
// direct / host messages.
assertContains(netmatchSrc, "{ k: 'hit', v: victim.nid, a: attacker.nid, d: r2(dmg), w: wid }");
assertContains(netmatchSrc, "{ k: 'dh', kind, id, d: r2(dmg) }");
assertContains(netmatchSrc, "{ k: 'bhit', a: attacker.nid, d: r2(d), weak: weak ? 1 : 0, w, c: crab }");
assertContains(netmatchSrc, "{ k: 'st', s: state, t: r2(m.time) }");
assertContains(netmatchSrc, "{ k: 'res', cov: result.coverage, win: result.winner, mode: result.mode, bo: result.boss,");
assertContains(netmatchSrc, "st: this.match.actors.map((a) => [a.nid, Math.round(a.stats.turf), a.stats.splats, a.stats.deaths, Math.round(a.stats.bossDmg || 0), a.stats.weakHits || 0])");
assertContains(netmatchSrc, "{ k: 'end' }");
// tick envelope (`_sendTick`): key order k,ts,a then optional e / B / c.
assertContains(netmatchSrc, "const msg = { k: 't', ts: r3(now()), a };");
assertContains(netmatchSrc, "if (this.out.length) { msg.e = this.out; this.out = []; }");
assertContains(netmatchSrc, "msg.c = [this.match.state, r2(this.match.time)]");
// `_rec` prefixes each event record with r3(now()) — timestamp FIRST.
assertContains(netmatchSrc, "_rec(e) { this.out.push([r3(now()), ...e]); }");
// session messages (src/net/session.js).
assertContains(sessionSrc, "{ k: 'lobby', l: this._wireLobby() }");
assertContains(sessionSrc, "{ map: l.map, time: l.time, duration: l.duration, bots: l.bots, difficulty: l.difficulty, palette: l.palette, mode: l.mode, players: l.players.map(({ id, name, team, weapon, sub, special, style, ready, ping }) => ({ id, name, team, weapon, sub, special, style, ready, ping })) }");
assertContains(sessionSrc, "{ k: 'emote', n: String(name).slice(0, 16) }");
assertContains(sessionSrc, "{ k: 'ready', id: cfg.id }");
assertContains(sessionSrc, "{ k: 'go', id: this._startCfg.id }");
assertContains(sessionSrc, "roster.push({ nid: nid++, owner: p.id, bot: false, team, slot: slot++, name: p.name, weapon: p.weapon, sub: subOf(p.sub), special: specialOf(p.special), style: p.style });");
assertContains(sessionSrc, "{ k: 'start', roster, map: l.map, time: l.time,");
// relay control frames (server/src/index.js) and transport envelopes.
assertContains(serverSrc, "{ t: 'welcome', id, host: all[0].id, members: all.map(({ id, name }) => ({ id, name })) }");
assertContains(serverSrc, "{ t: 'join', m: { id, name } }");
assertContains(serverSrc, "{ t: 'leave', id: a.id, host }");
assertContains(serverSrc, "{ t: 'err', e }");
assertContains(serverSrc, "{ t: 'pong', c: o.c }");
assertContains(transportSrc, "{ t: 'lock', v: !!v }");
assertContains(transportSrc, "this._raw('b|' + JSON.stringify(obj))");
assertContains(transportSrc, "this._raw('s|' + id + '|' + JSON.stringify(obj))");

// Build a module scope with the sliced helpers + the constants they close over.
const MAIN_KITS = {}; // M1: no kit weapons ⇒ `MAIN_KITS[kind]?.netState?.(wr) | 0` === 0
const specialNetState = () => 0; // M1: no specials ⇒ 0 (specials.js L2126 for null specialActive)
const THREE = { Vector3: class { constructor(x = 0, y = 0, z = 0) { this.x = x; this.y = y; this.z = z; this.isVector3 = true; } } };
const scope = { r2: null, r3: null, F: null, packActor: null, unpackActor: null, blankSample: null, copySample: null, packTrig: null, unpackTrig: null, packEvent: null, unpackEvent: null, MAIN_KITS, specialNetState, SPECIAL_ORDER, THREE };
const boot = new Function(
  'MAIN_KITS', 'specialNetState', 'SPECIAL_ORDER', 'THREE',
  [srcR2, srcR3, srcF, srcPackActor, srcUnpackActor, srcBlankSample, srcCopySample, srcPackTrig, srcUnpackTrig, srcPackEvent, srcUnpackEvent,
   'return { r2, r3, F, packActor, unpackActor, blankSample, copySample, packTrig, unpackTrig, packEvent, unpackEvent };'].join('\n')
);
Object.assign(scope, boot(MAIN_KITS, specialNetState, SPECIAL_ORDER, THREE));
const { r2, r3, packActor, unpackActor, packTrig, unpackTrig, packEvent, unpackEvent } = scope;

// splat record — the exact expression from recSplat (seed passed explicitly,
// never Math.random, so the fixture is deterministic).
function recSplat(c, radius, team, o) {
  const st = o.stretch;
  return ['s', r2(c.x), r2(c.y), r2(c.z), r2(radius), team, r3(o.seed ?? Math.random()), o.kind ?? 0,
    st ? r3(st.x) : 0, st ? r3(st.y) : 0, st ? r3(st.z) : 0, st ? r2(o.stretchAmt ?? 1) : 0];
}
// proj record — the exact 26-slot expression from recProj (L119-121).
function recProj(p) {
  const o = { nid: p._nid };
  return ['p', o.nid, p.type, p.wid || 0, r2(p.pos.x), r2(p.pos.y), r2(p.pos.z), r2(p.vel.x), r2(p.vel.y), r2(p.vel.z),
    r3(p.delay || 0), r3(p.life), r3(p.straight), r2(p.radius), r2(p.size), p.grav, p.drag, p.trailEvery || 0, p.head ? 1 : 0,
    r3(p.vis ?? 0.1), p.tail0 ?? 0.8, p.tailK ?? 1.3, p.wob ?? 0.035, p.wobF ?? 26, p.nose ?? 0.3, p.sats ?? 3];
}
// bomb record — recBomb (L127).
function recBomb(b) {
  return ['b', b._nid, b.kind, r2(b.pos.x), r2(b.pos.y), r2(b.pos.z), r2(b.vel.x), r2(b.vel.y), r2(b.vel.z)];
}
// kit record — recKit (L136): ['k', nid, kind, data] (data passed through raw).
function recKit(nid, kind, data) {
  return ['k', nid, kind, data];
}
// zone record — recZone (L148): ['z', e] where e is the zones.js netEvent array.
function recZone(e) {
  return ['z', e];
}

// Exact-decimal inputs (see header): the Rust `dec()` re-interpretation
// recovers these same f64 values from the f32 storage, so both sides round
// identically.
const V3 = (x, y, z) => new THREE.Vector3(x, y, z);

// An actor input in the shape packActor reads (netmatch.js L723-750). Every
// numeric field is an exact decimal the Rust f32 side round-trips through
// `wire::dec`.
function mkActor(over = {}) {
  return {
    nid: 0, pos: V3(0, 0, 0), vel: V3(0, 0, 0), yaw: 0, aimYaw: 0, aimPitch: 0,
    hp: 100, ink: 100, special: 0, stats: { turf: 0 }, netTp: 0,
    alive: true, form: 'kid', submerged: false, climbing: false, grounded: true,
    groundTeam: 0, onEnemy: false, invuln: 0, specialActive: null, superJumpState: null,
    specialId: undefined, weapon: { kind: 'shooter' },
    smoothY: 0, wallN: null,
    weaponRunner: { charging: false, rolling: false, streaming: false, dodge: false, aimingSub: false, firingT: 0, flick: -1, slosh: -1, lockT: 0, charge: 0, burstFrac: 0 },
    ...over,
  };
}

// --- fixture cases ---------------------------------------------------------
const actorCases = {
  kid_idle: mkActor(),
  squid_grounded_enemy: mkActor({ form: 'squid', submerged: true, groundTeam: 2, onEnemy: true, pos: V3(1.2345678, 2.0, -3.456789), vel: V3(0.5, -9.81, 1.5), yaw: 0.7123456, aimYaw: -1.1, aimPitch: 0.33, hp: 61.25, ink: 80, turf: 0 }),
  firing_invuln_gt1: mkActor({ weaponRunner: { ...mkActor().weaponRunner, firingT: 0.05 }, invuln: 1.0, groundTeam: 1, stats: { turf: 4096.5 } }),
  respawn_tp_jump: mkActor({ netTp: 7, alive: false, hp: 0 }),
  full_flags: mkActor({
    form: 'squid', submerged: true, climbing: true, groundTeam: 2, onEnemy: true, invuln: 1,
    wallN: V3(0, 1, 0),
    weaponRunner: { charging: true, rolling: true, streaming: true, dodge: true, aimingSub: true, firingT: 0.05, flick: 0.5, slosh: 0.5, lockT: 1.2, charge: 0.7, burstFrac: 0.3 },
    specialActive: { def: 1 }, superJumpState: { phase: 'flight' },
  }),
  // rounding boundaries: r2 half-up, r3 half-up, -0, .005 / .0005 edges.
  round_edges: mkActor({
    pos: V3(-0.0, 0.125, -0.125), // -0, .125 (r2 half → 0.13), -.125 (r2 half → -0.12 toward +inf)
    vel: V3(0.0005, -0.0005, 0.005), // r3 half edges
    yaw: 0.0005, aimYaw: -0.0005, aimPitch: 0.125,
    hp: 61.5, ink: 0.5, // Math.round half-up: 62, 1 (JS rounds .5 toward +inf)
  }),
};
// turf is under stats.turf; fix the two that used a top-level `turf` key.
actorCases.squid_grounded_enemy.stats = { turf: 4096.5 };
actorCases.firing_invuln_gt1.stats = { turf: 4096.5 };

const fixtures = { actors: {}, tick: {}, events: {}, trig: {}, ev: {}, msgs: {} };

for (const [name, a] of Object.entries(actorCases)) {
  const wire = packActor(a);
  fixtures.actors[name] = {
    input: a,
    wire,
    wire_str: JSON.stringify(wire),
    unpacked: unpackActor(wire, r3(12.345)),
  };
}

// --- tick envelope (host rides c; a tick with events rides e) --------------
{
  const actors = [packActor(actorCases.kid_idle), packActor(actorCases.squid_grounded_enemy)];
  const ts = r3(12.345);
  const guest = { k: 't', ts, a: actors };
  const host = { k: 't', ts, a: actors, c: ['playing', r2(90.0)] };
  // A tick carrying events: `_rec` prefixes each record with r3(now()) (L107),
  // so `msg.e` entries are `[ts, 's'|'tr'|…, …]` — timestamp FIRST.
  const withEvents = { k: 't', ts, a: actors, e: [[r3(12.34), ...recSplat(V3(0, 0, 0), 0.9, 1, { seed: 0.123 })], [r3(12.345), 'tr', 0, 'fire', packTrig(null)]] };
  fixtures.tick.guest = { wire_str: JSON.stringify(guest), ts, actorNames: ['kid_idle', 'squid_grounded_enemy'] };
  fixtures.tick.host = { wire_str: JSON.stringify(host), ts, clock: ['playing', r2(90.0)], actorNames: ['kid_idle', 'squid_grounded_enemy'] };
  fixtures.tick.with_events = { wire_str: JSON.stringify(withEvents), ts, actorNames: ['kid_idle', 'squid_grounded_enemy'] };
}

// --- event records ---------------------------------------------------------
{
  const splat = recSplat(V3(1.2345678, 2.0, -3.456789), 0.9, 1, { seed: 0.123456 });
  const splatStretch = recSplat(V3(0, 0, 0), 1.0, 0, { seed: 0.5, stretch: V3(0.5, 0, 0.5), stretchAmt: 1.5, kind: 'blast' });
  const proj = recProj({ _nid: 3, type: 'shot', wid: 'shooter', pos: V3(1, 2, 3), vel: V3(4, 5, 6), delay: 0, life: 1.2345, straight: 0.13, radius: 0.9, size: 0.4, grav: 1, drag: 0.02, trailEvery: 0.3, head: false, vis: 0.1, tail0: 0.8, tailK: 1.3, wob: 0.035, wobF: 26, nose: 0.3, sats: 3 });
  const proj2 = recProj({ _nid: 0, type: 'blast', wid: 'splasher', pos: V3(-2.5, 1.5, 0), vel: V3(0, 7.2, -3), delay: 0.25, life: 0.8, straight: 0, radius: 1.25, size: 0.6, grav: 22, drag: 0.05, trailEvery: 0, head: true, vis: 0.2, tail0: 1, tailK: 2, wob: 0, wobF: 12, nose: 0.5, sats: 0 });
  const bomb = recBomb({ _nid: 1, kind: 'bomb', pos: V3(0.5, 1.5, -2.5), vel: V3(3, 4, -1) });
  const kit = recKit(2, 'mitts', [0.5, 1.25, 0]);
  const zone = recZone(['zc', 1, 0]);
  fixtures.events.splat = { wire: splat, wire_str: JSON.stringify(splat) };
  fixtures.events.splat_stretch = { wire: splatStretch, wire_str: JSON.stringify(splatStretch) };
  fixtures.events.proj = { wire: proj, wire_str: JSON.stringify(proj) };
  fixtures.events.proj_blast = { wire: proj2, wire_str: JSON.stringify(proj2) };
  fixtures.events.bomb = { wire: bomb, wire_str: JSON.stringify(bomb) };
  fixtures.events.kit = { wire: kit, wire_str: JSON.stringify(kit) };
  fixtures.events.zone = { wire: zone, wire_str: JSON.stringify(zone) };
}

// --- triggers (number / null / object branches of packTrig) ----------------
{
  const cases = { num: 0.123456, null: null, obj: { amp: 0.123456, hand: 1 }, obj_nofloat: { tag: 'x', amp: 0.5 } };
  for (const [k, v] of Object.entries(cases)) {
    const p = packTrig(v);
    fixtures.trig[k] = { input: v, wire: p, wire_str: JSON.stringify(p), unpacked: unpackTrig(p) };
  }
}

// --- forwarded events (packEvent: actor/vector/number/string/bool) ---------
{
  const victim = mkActor({ nid: 2, character: {} });
  const attacker = mkActor({ nid: 0, character: {} });
  const e = { victim, attacker, cause: 'shooter', pos: V3(1.5, 2.5, -3.5), amount: 0.123456, swim: true };
  const p = packEvent(e);
  const nm = { byNid: new Map([[victim.nid, victim], [attacker.nid, attacker]]) };
  fixtures.ev.splatted = { wire: p, wire_str: JSON.stringify(p), unpacked_keys: Object.keys(unpackEvent(p, nm)) };
  // respawn: payload carries only the actor (→ {n: nid}).
  const rp = packEvent({ actor: victim });
  fixtures.ev.respawn = { wire: rp, wire_str: JSON.stringify(rp), unpacked_keys: Object.keys(unpackEvent(rp, nm)) };
  // weapon:fire: a vector + numbers, no actor field the replay resolves.
  const wf = packEvent({ pos: V3(1, 2, 3), dir: V3(0, 0, -1), spread: 0.035, type: 'shot' });
  fixtures.ev.weapon_fire = { wire: wf, wire_str: JSON.stringify(wf) };
}

// --- direct / host / session JSON messages ---------------------------------
{
  const hit = { k: 'hit', v: 2, a: 0, d: r2(12.345), w: 'shooter' };
  const hitNoW = { k: 'hit', v: 2, a: 0, d: r2(36) };
  const dh = { k: 'dh', kind: 'subs', id: 9, d: r2(4.0) };
  const bhit = { k: 'bhit', a: 1, d: r2(20.0), weak: 1, w: 'crab', c: 3 };
  const bhitBoss = { k: 'bhit', a: 1, d: r2(30.0), weak: 0, w: 'shooter', c: -1 };
  const st = { k: 'st', s: 'playing', t: r2(90.0) };
  const stFinish = { k: 'st', s: 'finish', t: r2(0.0) };
  // turf result: sendResult spreads mode/bo = undefined → JSON.stringify drops them.
  const res = { k: 'res', cov: [0.61, 0.39], win: 0, mode: undefined, bo: undefined, st: [[0, 120, 2, 1, 0, 0], [1, 80, 1, 0, 0, 0]] };
  const end = { k: 'end' };
  // `own`/`me` have no single send-side literal to anchor: `own`'s sender is
  // the reserved `_ownership()` stub (netmatch.js L709), and `me` is sent as
  // `{ k: 'me', ...o }` spreads (session.js L110/212/388). Both ride the JVal
  // passthrough path, so their byte shape is low-risk; the forms here follow
  // the receive-side contract (netmatch.js L219 / session.js handlers).
  const own = { k: 'own', map: { 4: 'A1B2', 5: 'A1B2' } };
  const me = { k: 'me', name: 'ab', weapon: 'shooter', sub: null, special: null, style: null, ping: 42 };
  const lobby = { k: 'lobby', l: { map: 'tidewater', time: 'day', duration: 180, bots: true, difficulty: 'normal', palette: 0, mode: 'turf', players: [{ id: 'A1B2', name: 'ab', team: 0, weapon: 'shooter', sub: null, special: null, style: null, ready: true, ping: 42 }] } };
  // a second lobby: two players, one not ready, different settings (exercises
  // the roster array + null sub/special/style passthrough on both sides).
  const lobby2 = { k: 'lobby', l: { map: 'tidewater', time: 'dusk', duration: 120, bots: false, difficulty: 'hard', palette: 2, mode: 'turf', players: [{ id: 'A1B2', name: 'ab', team: 0, weapon: 'shooter', sub: null, special: null, style: null, ready: true, ping: 15 }, { id: 'K3XZ', name: 'cd', team: 1, weapon: 'spritzer', sub: null, special: null, style: null, ready: false, ping: 88 }] } };
  const start = { k: 'start', roster: [{ nid: 0, owner: 'A1B2', bot: false, team: 0, slot: 0, name: 'ab', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 1, owner: 'A1B2', bot: true, team: 0, slot: 1, name: 'Bot', weapon: 'shooter', sub: null, special: null, style: null }], map: 'tidewater', time: 'day', duration: 180, difficulty: 'normal', palette: 0, mode: 'turf', host: 'A1B2', id: 'abc123' };
  // a second start: a full 4v4 roster across two owners (nid ordering team0→team1).
  const start2 = { k: 'start', roster: [{ nid: 0, owner: 'A1B2', bot: false, team: 0, slot: 0, name: 'ab', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 1, owner: 'K3XZ', bot: false, team: 0, slot: 1, name: 'cd', weapon: 'spritzer', sub: null, special: null, style: null }, { nid: 2, owner: 'A1B2', bot: true, team: 0, slot: 2, name: 'B0', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 3, owner: 'A1B2', bot: true, team: 0, slot: 3, name: 'B1', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 4, owner: 'A1B2', bot: true, team: 1, slot: 0, name: 'B2', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 5, owner: 'A1B2', bot: true, team: 1, slot: 1, name: 'B3', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 6, owner: 'A1B2', bot: true, team: 1, slot: 2, name: 'B4', weapon: 'shooter', sub: null, special: null, style: null }, { nid: 7, owner: 'A1B2', bot: true, team: 1, slot: 3, name: 'B5', weapon: 'shooter', sub: null, special: null, style: null }], map: 'tidewater', time: 'day', duration: 180, difficulty: 'normal', palette: 0, mode: 'turf', host: 'A1B2', id: 'xyz789' };
  const ready = { k: 'ready', id: 'abc123' };
  const go = { k: 'go', id: 'abc123' };
  const emote = { k: 'emote', n: 'booyah' };
  fixtures.msgs = {
    hit: JSON.stringify(hit), hit_no_w: JSON.stringify(hitNoW),
    dh: JSON.stringify(dh), bhit: JSON.stringify(bhit), bhit_boss: JSON.stringify(bhitBoss),
    st: JSON.stringify(st), st_finish: JSON.stringify(stFinish),
    res: JSON.stringify(res), end: JSON.stringify(end), own: JSON.stringify(own),
    me: JSON.stringify(me), lobby: JSON.stringify(lobby), lobby2: JSON.stringify(lobby2),
    start: JSON.stringify(start), start2: JSON.stringify(start2),
    ready: JSON.stringify(ready), go: JSON.stringify(go), emote: JSON.stringify(emote),
  };
}

// --- relay control frames (server/src/index.js L14-16, L81-83) -------------
{
  const welcome = { t: 'welcome', id: 'K3XZ', host: 'A1B2', members: [{ id: 'A1B2', name: 'ab' }, { id: 'K3XZ', name: 'cd' }] };
  const join = { t: 'join', m: { id: 'Q7M1', name: 'ef' } };
  const leave = { t: 'leave', id: 'Q7M1', host: 'A1B2' };
  const err = { t: 'err', e: 'Room is full' };
  const pong = { t: 'pong', c: 12345.75 };
  const lock = { t: 'lock', v: true };
  const lockOff = { t: 'lock', v: false };
  fixtures.control = {
    welcome: JSON.stringify(welcome), join: JSON.stringify(join), leave: JSON.stringify(leave),
    err: JSON.stringify(err), pong: JSON.stringify(pong), lock: JSON.stringify(lock), lock_off: JSON.stringify(lockOff),
  };
}

const out = join(repoRoot, 'rust', 'assets', 'net-fixtures.json');
writeFileSync(out, JSON.stringify({ generated_from: 'src/net/netmatch.js (sliced packers)', fixtures }, null, 2) + '\n');
console.log('wrote', out);
console.log('actor cases:', Object.keys(fixtures.actors).join(', '));
