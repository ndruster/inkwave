// M3 tuning extractor (Task 3 M1 slice, extended by Task 28 for M3; TR-28.1/TR-28.2),
// extended by Task 41 with the HULLBREAKER boss data tables (schema v3, TR-41.1),
// extended by Task 42 batch A with the boss nav plan constants and the
// boss.hit rest-model-space sphere positions (schema v4),
// extended by Task 43 batch A with the brain PREF table (schema v5).
//
// Imports the upstream tuning module verbatim (src/config.js — never modified,
// AC-1/TR-2.4) and emits the full gameplay tuning set into
// rust/assets/tuning.json (schema inkwave.tuning.v5):
//   PLAYER, WEAPONS (the 12 roster weapons per WEAPON_ORDER — the retired
//   dualies/slosher/splatling defs are NOT emitted, TR-28.2), WEAPON_ORDER,
//   weaponRange() evaluated per weapon, SUBS (15, per SUB_ORDER),
//   SPECIALS (19, per SPECIAL_ORDER), ZONES, MATCH, TEAM_PALETTES,
//   DIFFICULTY (easy/normal/hard), BOSS (the Task 41 boss section).
// The retired M1 `spritzer` top-level alias was removed in Task 29 — the
// consumers read `weapons.shooter` directly.
//
// Output goes through stableStringify (sorted keys, no timestamps) so a re-run
// against the same upstream commit is byte-identical.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
// Real three (already a project dependency) for the Task 42 hitPos static
// evaluation — the boss modules themselves are still NOT imported (they pull
// browser globals); only their literal blocks are sliced and eval'd below.
import * as THREE from 'three';
import {
  PLAYER, WEAPONS, WEAPON_ORDER, weaponRange, SUBS, SUB_ORDER,
  SPECIALS, SPECIAL_ORDER, ZONES, MATCH, TEAM_PALETTES, DIFFICULTY,
} from '../../../src/config.js';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));      // rust/tools/extract
const repoRoot = resolve(here, '../../..');              // repository root
const outPath = resolve(here, '../../assets/tuning.json');

let commit;
try {
  commit = execSync('git rev-parse HEAD', { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] })
    .toString().trim();
} catch {
  commit = 'unknown';
}

// Project `table` down to `order` (throwing on unknown ids so an upstream
// rename fails the pipeline instead of silently dropping content).
function pick(table, order, label) {
  const out = {};
  for (const id of order) {
    if (!table[id]) throw new Error(`${label}: missing "${id}"`);
    out[id] = table[id];
  }
  return out;
}

// Effective reach per weapon (m) — the weaponRange() switch (config.js
// L307-315) evaluated at extraction time, so the Rust side never re-implements
// the branch logic.
const ranges = {};
for (const id of WEAPON_ORDER) ranges[id] = weaponRange(WEAPONS[id]);

// --- Task 41: the HULLBREAKER boss data tables ------------------------------
// The boss modules pull THREE / browser globals through their ESM imports, so
// they are NOT imported here; instead we slice the exact literal blocks out of
// the sources and eval them standalone (the pattern proven in
// rust/tools/net-fixtures/gen.mjs). assertContains guards pin the surrounding
// source lines, so upstream drift fails the pipeline instead of silently
// changing the artifact.
function sliceBlock(src, file, name, start, end) {
  const i = src.indexOf(start);
  if (i < 0) throw new Error(`${file}: sliceBlock(${name}): start marker not found: ${start}`);
  const j = src.indexOf(end, i);
  if (j < 0) throw new Error(`${file}: sliceBlock(${name}): end marker not found after start: ${end}`);
  return src.slice(i, j + end.length);
}
function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`expected fragment not found in ${where}: ${needle}`);
}

const bossJsSrc = readFileSync(resolve(repoRoot, 'src/boss/boss.js'), 'utf8');
const bossModeSrc = readFileSync(resolve(repoRoot, 'src/boss/bossMode.js'), 'utf8');
const bossHzSrc = readFileSync(resolve(repoRoot, 'src/boss/bossHazards.js'), 'utf8');
const bossModelSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModel.js'), 'utf8');
const bossBrainSrc = readFileSync(resolve(repoRoot, 'src/boss/bossBrain.js'), 'utf8');
const bossNavSrc = readFileSync(resolve(repoRoot, 'src/boss/bossNav.js'), 'utf8');
const bossGeoSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModelGeo.js'), 'utf8');

// BOSS (boss.js L15-25).
const srcBOSS = sliceBlock(bossJsSrc, 'boss.js', 'BOSS', 'const BOSS = {', '\n};');
assertContains(srcBOSS, "name: 'HULLBREAKER',", 'boss.js BOSS');
assertContains(srcBOSS, 'hpPerUnit: 18000,', 'boss.js BOSS');
assertContains(srcBOSS, 'phases: [0.66, 0.33],', 'boss.js BOSS');
assertContains(srcBOSS, 'introAt: 1.8, introLen: 3.4, roarLen: 1.9,', 'boss.js BOSS');
const BOSS = new Function(`${srcBOSS}\nreturn BOSS;`)();

// BOSS_MODE + squadUnits (bossMode.js L7-15).
const srcBOSS_MODE = sliceBlock(bossModeSrc, 'bossMode.js', 'BOSS_MODE', 'const BOSS_MODE = {', '\n};');
const srcSQUAD = sliceBlock(bossModeSrc, 'bossMode.js', 'squadUnits', 'function squadUnits', '\n');
assertContains(srcBOSS_MODE, 'duration: 240,', 'bossMode.js BOSS_MODE');
assertContains(srcBOSS_MODE, 'squad: 8,', 'bossMode.js BOSS_MODE');
assertContains(srcSQUAD, 'a.isBot && !a.isLocal ? 0.6 : 1', 'bossMode.js squadUnits');
const { BOSS_MODE, squadUnits } = new Function(
  `${srcBOSS_MODE}\n${srcSQUAD}\nreturn { BOSS_MODE, squadUnits };`,
)();

// MOVES / PACE / MIN_TELE / HZ / SAFE_PAD / REACH_N (bossHazards.js L17-36).
// The design spec's `reachStep` is the JS `SAFE_PAD` (bossHazards.js L35).
const srcMOVES = sliceBlock(bossHzSrc, 'bossHazards.js', 'MOVES', 'const MOVES = {', '\n};');
const srcPACE = sliceBlock(bossHzSrc, 'bossHazards.js', 'PACE', 'const PACE = [', '];');
const srcMIN_TELE = sliceBlock(bossHzSrc, 'bossHazards.js', 'MIN_TELE', 'const MIN_TELE =', '\n');
const srcHZ = sliceBlock(bossHzSrc, 'bossHazards.js', 'HZ', 'const HZ = {', '\n};');
const srcSAFE_PAD = sliceBlock(bossHzSrc, 'bossHazards.js', 'SAFE_PAD', 'const SAFE_PAD =', '\n');
const srcREACH_N = sliceBlock(bossHzSrc, 'bossHazards.js', 'REACH_N', 'const REACH_N =', '\n');
assertContains(srcMOVES, 'slam: { tele: 1.15, act: 1.9, rec: 1.0 },', 'bossHazards.js MOVES');
assertContains(srcMOVES, 'frenzy: { tele: 1.2, act: 3.0, rec: 1.7 },', 'bossHazards.js MOVES');
assertContains(srcPACE, 'const PACE = [1, 1, 0.88, 0.78];', 'bossHazards.js PACE');
assertContains(srcHZ, 'chargeSpeed: [0, 13, 14.5, 16],', 'bossHazards.js HZ');
assertContains(srcHZ, 'crabHp: 30,', 'bossHazards.js HZ');
assertContains(srcSAFE_PAD, 'const SAFE_PAD = 0.6;', 'bossHazards.js SAFE_PAD');
assertContains(srcREACH_N, 'const REACH_N = 96;', 'bossHazards.js REACH_N');
const { MOVES, PACE, MIN_TELE, HZ, SAFE_PAD, REACH_N } = new Function(
  [srcMOVES, srcPACE, srcMIN_TELE, srcHZ, srcSAFE_PAD, srcREACH_N,
   'return { MOVES, PACE, MIN_TELE, HZ, SAFE_PAD, REACH_N };'].join('\n'),
)();

// HIT (bossModel.js L14-18) — not exported, sliced.
const srcHIT = sliceBlock(bossModelSrc, 'bossModel.js', 'HIT', 'const HIT = [', '];');
assertContains(srcHIT, "['shellF', 1.8, false]", 'bossModel.js HIT');
assertContains(srcHIT, "['crackR', 0.7, true]", 'bossModel.js HIT');
const HIT = new Function(`${srcHIT}\nreturn HIT;`)();

// SPEED / TURN / COOL / COOL_RND / PREF (bossBrain.js L12-16) — private, sliced.
const srcSPEED = sliceBlock(bossBrainSrc, 'bossBrain.js', 'SPEED', 'const SPEED = [', '];');
const srcTURN = sliceBlock(bossBrainSrc, 'bossBrain.js', 'TURN', 'const TURN = [', '];');
const srcCOOL = sliceBlock(bossBrainSrc, 'bossBrain.js', 'COOL', 'const COOL = [', '];');
const srcCOOL_RND = sliceBlock(bossBrainSrc, 'bossBrain.js', 'COOL_RND', 'const COOL_RND = [', '];');
const srcPREF = sliceBlock(bossBrainSrc, 'bossBrain.js', 'PREF', 'const PREF = [', '];');
assertContains(srcSPEED, 'const SPEED = [0, 2.9, 3.3, 3.8];', 'bossBrain.js SPEED');
assertContains(srcTURN, 'const TURN = [0, 1.25, 1.45, 1.75];', 'bossBrain.js TURN');
assertContains(srcCOOL, 'const COOL = [0, 1.05, 0.8, 0.6];', 'bossBrain.js COOL');
assertContains(srcCOOL_RND, 'const COOL_RND = [0, 0.8, 0.6, 0.45];', 'bossBrain.js COOL_RND');
assertContains(srcPREF, 'const PREF = [0, 10, 9, 8];', 'bossBrain.js PREF');
const { SPEED, TURN, COOL, COOL_RND, PREF } = new Function(
  [srcSPEED, srcTURN, srcCOOL, srcCOOL_RND, srcPREF,
   'return { SPEED, TURN, COOL, COOL_RND, PREF };'].join('\n'),
)();

// WALL_BODY / FLOOR_BODY / PAD_MARGIN (bossNav.js L10-15) — the design spec's
// `padr` is the JS `PAD_MARGIN` (bossNav.js L15).
const srcWALL_BODY = sliceBlock(bossNavSrc, 'bossNav.js', 'WALL_BODY', 'const WALL_BODY = [', '];');
const srcFLOOR_BODY = sliceBlock(bossNavSrc, 'bossNav.js', 'FLOOR_BODY', 'const FLOOR_BODY = [', '];');
const srcPAD_MARGIN = sliceBlock(bossNavSrc, 'bossNav.js', 'PAD_MARGIN', 'const PAD_MARGIN = [', '];');
assertContains(srcWALL_BODY, '[{ z: 2.3, r: 2.5 }, { z: -1.8, r: 2.6 }]', 'bossNav.js WALL_BODY');
assertContains(srcFLOOR_BODY, '[{ z: 2.2, r: 1.45 }, { z: 0, r: 1.5 }, { z: -2.3, r: 1.45 }]', 'bossNav.js FLOOR_BODY');
assertContains(srcPAD_MARGIN, 'const PAD_MARGIN = [3.5, 1.0];', 'bossNav.js PAD_MARGIN');
const { WALL_BODY, FLOOR_BODY, PAD_MARGIN } = new Function(
  [srcWALL_BODY, srcFLOOR_BODY, srcPAD_MARGIN, 'return { WALL_BODY, FLOOR_BODY, PAD_MARGIN };'].join('\n'),
)();

// PLAN_WALL / PLAN_FLOOR / STRIDE (bossNav.js L12-14) — Task 42 batch A.
const srcPLAN_WALL = sliceBlock(bossNavSrc, 'bossNav.js', 'PLAN_WALL', 'const PLAN_WALL =', '\n');
const srcPLAN_FLOOR = sliceBlock(bossNavSrc, 'bossNav.js', 'PLAN_FLOOR', 'const PLAN_FLOOR =', '\n');
const srcSTRIDE = sliceBlock(bossNavSrc, 'bossNav.js', 'STRIDE', 'const STRIDE =', '\n');
assertContains(srcPLAN_WALL, 'const PLAN_WALL = 2.9;', 'bossNav.js PLAN_WALL');
assertContains(srcPLAN_FLOOR, 'const PLAN_FLOOR = 1.8;', 'bossNav.js PLAN_FLOOR');
assertContains(srcSTRIDE, 'const STRIDE = 1.35;', 'bossNav.js STRIDE');
const { PLAN_WALL, PLAN_FLOOR, STRIDE } = new Function(
  [srcPLAN_WALL, srcPLAN_FLOOR, srcSTRIDE, 'return { PLAN_WALL, PLAN_FLOOR, STRIDE };'].join('\n'),
)();

// --- Task 42 batch A: rest-model-space hit sphere positions (boss.hit[].pos).
// Static route (spec Task 42 §1, verified against the sources): buildRig's
// add() only sets bone.position (bossModelGeo.js L320-327, no quaternion), so
// the rest bone chain is pure translation; getSocket computes
// off.applyMatrix4(bone.matrixWorld) with off = p - rest[bone]
// (bossModel.js L56 + L151-154), which collapses to the sockets-table literal
// point p. The three sockets overridden in the constructor (shellF / shellR /
// body, bossModel.js L57-59) likewise reduce to their literal expression with
// the trailing `.sub(this.rig.rest.*)` cancelled by rest[bone]. The geo
// dependency blocks are sliced from bossModelGeo.js and eval'd against the
// real THREE; the independent evidence chain (real buildRig + real
// matrixWorld) lives in rust/tools/verify/boss-hitpos-crosscheck.mjs.
const deExport = (s) => s.replace(/^export /, '');
const srcV3 = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'V3', 'export const V3 =', '\n');
assertContains(srcV3, 'new THREE.Vector3(x, y, z)', 'bossModelGeo.js V3');
const srcM = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'M', 'const M = () =>', '\n');
assertContains(srcM, 'new THREE.Matrix4()', 'bossModelGeo.js M');
const srcTRS = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'TRS', 'const TRS =', '\n');
assertContains(srcTRS, "setFromEuler(new THREE.Euler(rx, ry, rz, 'YXZ'))", 'bossModelGeo.js TRS');
const srcCONT = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'CONT', 'export const CONT = {', '};');
assertContains(srcCONT, 'pos: V3(0, 3.0, -1.3), pitch: 0.12, hl: 3.03, hw: 1.22, hh: 1.3', 'bossModelGeo.js CONT');
const srcMC = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'MC', 'export const MC =', ');');
assertContains(srcMC, 'TRS(CONT.pos.x, CONT.pos.y, CONT.pos.z, CONT.pitch, 0, 0)', 'bossModelGeo.js MC');
const srccont = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'cont', 'export const cont =', ');');
assertContains(srccont, 'V3(x, y, z).applyMatrix4(MC)', 'bossModelGeo.js cont');
const srcARMS = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'ARMS', 'export const ARMS = [', '];');
assertContains(srcARMS, "name: 'L', crusher: true, S: V3(0.84, 1.81, 2.62)", 'bossModelGeo.js ARMS');
assertContains(srcARMS, "D: V3(0.06, -0.2, 1).normalize()", 'bossModelGeo.js ARMS');
assertContains(srcARMS, "D: V3(-0.03, -0.06, 1).normalize()", 'bossModelGeo.js ARMS');
const srcHEAD = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'HEAD', 'export const HEAD = {', '\n};');
assertContains(srcHEAD, 'stalk: [V3(0.38, 2.82, 2.9), V3(0.52, 3.24, 3.02), V3(0.64, 3.64, 3.1)]', 'bossModelGeo.js HEAD');
// The sockets table (bossModelGeo.js L822-829) is local to buildParts; `hl`,
// `hw`, `hh` there are the CONT destructure (bossModelGeo.js L386).
const srcSOCKETS = sliceBlock(bossGeoSrc, 'bossModelGeo.js', 'sockets', 'const sockets = {', '\n  };');
assertContains(srcSOCKETS, "clawL: ['clawL', ARMS[0].W.clone().addScaledVector(ARMS[0].D, 1.05)]", 'bossModelGeo.js sockets');
assertContains(srcSOCKETS, "belly: ['belly0', V3(0, 1.25, 2.25)]", 'bossModelGeo.js sockets');
assertContains(srcSOCKETS, "crackR: ['abdomen', cont(-hw + 0.1, 0.5, -0.22)]", 'bossModelGeo.js sockets');
const { sockets, CONT } = new Function('THREE', [
  deExport(srcV3), srcM, srcTRS, deExport(srcCONT), deExport(srcMC), deExport(srccont),
  deExport(srcARMS), deExport(srcHEAD),
  'const { hl, hw, hh } = CONT;',
  srcSOCKETS,
  'return { sockets, CONT };',
].join('\n'))(THREE);

// The three constructor overrides (bossModel.js L57-59): parse the off
// expression off the source line and cancel the trailing `.sub(...)` (the
// pure-translation argument above), so no number is retyped here.
function restSocketPoint(name, pin) {
  const i = bossModelSrc.indexOf(`this._sock.${name} = {`);
  if (i < 0) throw new Error(`bossModel.js: _sock.${name} override not found`);
  const line = bossModelSrc.slice(i, bossModelSrc.indexOf('\n', i));
  assertContains(line, pin, `bossModel.js _sock.${name}`);
  const a = line.indexOf('off: ');
  const b = line.lastIndexOf(' };');
  if (a < 0 || b <= a) throw new Error(`bossModel.js: cannot parse _sock.${name} off expression`);
  const off = line.slice(a + 5, b);
  const s = off.lastIndexOf('.sub(this.rig.rest.');
  if (s < 0) throw new Error(`bossModel.js: _sock.${name} off lacks .sub(this.rig.rest.*)`);
  return off.slice(0, s);
}
const special = new Function('THREE', 'CONT', [
  `return {`,
  `  shellF: (${restSocketPoint('shellF', 'new THREE.Vector3(0, 0.05, 1.5)')}),`,
  `  shellR: (${restSocketPoint('shellR', 'new THREE.Vector3(0, 0.1, -1.55)')}),`,
  `  body: (${restSocketPoint('body', 'new THREE.Vector3(0, 2.2, 2.3)')}),`,
  `};`,
].join('\n'))(THREE, CONT);

// Rest point per HIT socket: the constructor overrides win over the sockets
// table (bossModel.js L56 loop runs before L57-59).
for (const [socket] of HIT) {
  const p = special[socket] ?? sockets[socket]?.[1];
  if (!p || typeof p.x !== 'number') throw new Error(`boss.hit: no rest point for socket ${socket}`);
}

const boss = {
  boss: BOSS,
  mode: BOSS_MODE,
  // squadUnits() evaluated for one human / one non-local bot (bossMode.js L15).
  squadUnits: { human: squadUnits([{ isBot: false }]), bot: squadUnits([{ isBot: true, isLocal: false }]) },
  moves: MOVES,
  pace: PACE,
  minTele: MIN_TELE,
  hz: HZ,
  reachStep: SAFE_PAD,
  reachN: REACH_N,
  brain: { speed: SPEED, turn: TURN, cool: COOL, coolRnd: COOL_RND, pref: PREF },
  nav: {
    wallBody: WALL_BODY, floorBody: FLOOR_BODY, padr: PAD_MARGIN,
    planWall: PLAN_WALL, planFloor: PLAN_FLOOR, stride: STRIDE,
  },
  hit: HIT.map(([socket, r, weak]) => {
    const p = special[socket] ?? sockets[socket][1];
    return { socket, r, weak, pos: [p.x, p.y, p.z] };
  }),
};

const tuning = {
  schema: 'inkwave.tuning.v5',
  source: { commit, path: 'src/config.js + src/boss/*.js' },
  player: PLAYER,
  weaponOrder: WEAPON_ORDER,
  weapons: pick(WEAPONS, WEAPON_ORDER, 'WEAPON_ORDER'),
  weaponRange: ranges,
  subOrder: SUB_ORDER,
  subs: pick(SUBS, SUB_ORDER, 'SUB_ORDER'),
  specialOrder: SPECIAL_ORDER,
  specials: pick(SPECIALS, SPECIAL_ORDER, 'SPECIAL_ORDER'),
  zones: ZONES,
  match: MATCH,
  teamPalettes: TEAM_PALETTES,
  difficulty: DIFFICULTY,
  boss,
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(tuning));
console.log(
  `tuning.json <- src/config.js + src/boss/*.js @ ${commit} ` +
  `(${WEAPON_ORDER.length} weapons, ${SUB_ORDER.length} subs, ${SPECIAL_ORDER.length} specials, boss)`,
);
