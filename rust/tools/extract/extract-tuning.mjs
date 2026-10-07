// M1 tuning extractor (Task 3 / TR-3.2, TR-3.4).
//
// Imports the upstream tuning module verbatim (src/config.js — never modified,
// AC-1/TR-2.4) and emits the M1 slice the simulation needs into
// rust/assets/tuning.json:
//   PLAYER (all physics/feel constants), WEAPONS.shooter (the Spritzer),
//   MATCH, TEAM_PALETTES, DIFFICULTY.easy.
//
// Output goes through stableStringify (sorted keys, no timestamps) so a re-run
// against the same upstream commit is byte-identical.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import { PLAYER, WEAPONS, MATCH, TEAM_PALETTES, DIFFICULTY } from '../../../src/config.js';
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

const tuning = {
  schema: 'inkwave.tuning.v1',
  source: { commit, path: 'src/config.js' },
  player: PLAYER,
  spritzer: WEAPONS.shooter,
  match: MATCH,
  teamPalettes: TEAM_PALETTES,
  difficulty: { easy: DIFFICULTY.easy },
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(tuning));
console.log(`tuning.json <- src/config.js @ ${commit}`);
