// M3 tuning extractor (Task 3 M1 slice, extended by Task 28 for M3; TR-28.1/TR-28.2),
// extended by Task 41 with the HULLBREAKER boss data tables (schema v3, TR-41.1).
//
// Imports the upstream tuning module verbatim (src/config.js — never modified,
// AC-1/TR-2.4) and emits the full gameplay tuning set into
// rust/assets/tuning.json (schema inkwave.tuning.v3):
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

// SPEED / TURN / COOL / COOL_RND (bossBrain.js L12-15) — private, sliced.
const srcSPEED = sliceBlock(bossBrainSrc, 'bossBrain.js', 'SPEED', 'const SPEED = [', '];');
const srcTURN = sliceBlock(bossBrainSrc, 'bossBrain.js', 'TURN', 'const TURN = [', '];');
const srcCOOL = sliceBlock(bossBrainSrc, 'bossBrain.js', 'COOL', 'const COOL = [', '];');
const srcCOOL_RND = sliceBlock(bossBrainSrc, 'bossBrain.js', 'COOL_RND', 'const COOL_RND = [', '];');
assertContains(srcSPEED, 'const SPEED = [0, 2.9, 3.3, 3.8];', 'bossBrain.js SPEED');
assertContains(srcTURN, 'const TURN = [0, 1.25, 1.45, 1.75];', 'bossBrain.js TURN');
assertContains(srcCOOL, 'const COOL = [0, 1.05, 0.8, 0.6];', 'bossBrain.js COOL');
assertContains(srcCOOL_RND, 'const COOL_RND = [0, 0.8, 0.6, 0.45];', 'bossBrain.js COOL_RND');
const { SPEED, TURN, COOL, COOL_RND } = new Function(
  [srcSPEED, srcTURN, srcCOOL, srcCOOL_RND, 'return { SPEED, TURN, COOL, COOL_RND };'].join('\n'),
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
  brain: { speed: SPEED, turn: TURN, cool: COOL, coolRnd: COOL_RND },
  nav: { wallBody: WALL_BODY, floorBody: FLOOR_BODY, padr: PAD_MARGIN },
  hit: HIT.map(([socket, r, weak]) => ({ socket, r, weak })),
};

const tuning = {
  schema: 'inkwave.tuning.v3',
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
