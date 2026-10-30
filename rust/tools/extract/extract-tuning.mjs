// M3 tuning extractor (Task 3 M1 slice, extended by Task 28 for M3; TR-28.1/TR-28.2).
//
// Imports the upstream tuning module verbatim (src/config.js — never modified,
// AC-1/TR-2.4) and emits the full gameplay tuning set into
// rust/assets/tuning.json (schema inkwave.tuning.v2):
//   PLAYER, WEAPONS (the 12 roster weapons per WEAPON_ORDER — the retired
//   dualies/slosher/splatling defs are NOT emitted, TR-28.2), WEAPON_ORDER,
//   weaponRange() evaluated per weapon, SUBS (15, per SUB_ORDER),
//   SPECIALS (19, per SPECIAL_ORDER), ZONES, MATCH, TEAM_PALETTES,
//   DIFFICULTY (easy/normal/hard).
// `spritzer` is kept as the M1 alias of WEAPONS.shooter until the Task 29
// runner refactor migrates its consumers.
//
// Output goes through stableStringify (sorted keys, no timestamps) so a re-run
// against the same upstream commit is byte-identical.
import { mkdirSync, writeFileSync } from 'node:fs';
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

const tuning = {
  schema: 'inkwave.tuning.v2',
  source: { commit, path: 'src/config.js' },
  player: PLAYER,
  spritzer: WEAPONS.shooter, // M1 alias of weapons.shooter (removed in Task 29)
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
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(tuning));
console.log(
  `tuning.json <- src/config.js @ ${commit} ` +
  `(${WEAPON_ORDER.length} weapons, ${SUB_ORDER.length} subs, ${SPECIAL_ORDER.length} specials)`,
);
