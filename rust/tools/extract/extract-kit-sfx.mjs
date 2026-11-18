// M5 Task 60 batch A: extract the kit-weapon SFX cue table from the READ-ONLY
// upstream JS tree (JS tree stays byte-clean).
//
// `src/audio/audio.js` owns the `SFX` registry (L399 `export const SFX = {}`);
// `src/game/kits/index.js` imports the 9 kit modules that self-register their
// cues. Both are pure ESM and import cleanly in Node. The base 148 cues come
// from `SFX_NAMES` (audio.js L2501); everything else is a kit cue.
//
// M4 docs said "42 kit SFX"; the actual upstream tree registers 53 — the
// extractor pins the LIVE table, never a hand-copied name list.
// Output: rust/assets/audio/kit-sfx.json (stable-json, deterministic).
import { upstreamCommit } from './upstream-commit.mjs';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pathToFileURL } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/audio/kit-sfx.json');

const kitFiles = [
  'src/audio/audio.js',
  'src/game/kits/index.js',
  'src/game/kits/brolly.js',
  'src/game/kits/bow.js',
  'src/game/kits/bow-sfx.js',
  'src/game/kits/blade.js',
  'src/game/kits/blade-fx.js',
  'src/game/kits/mitts.js',
  'src/game/kits/mitts-assets.js',
  'src/game/kits/shaker.js',
  'src/game/kits/waddle.js',
  'src/game/kits/torpedo.js',
  'src/game/kits/tracer.js',
  'src/game/kits/boomerang.js',
];
const commit = upstreamCommit(repoRoot, kitFiles);

const { SFX, SFX_NAMES } = await import(pathToFileURL(resolve(repoRoot, 'src/audio/audio.js')).href);
await import(pathToFileURL(resolve(repoRoot, 'src/game/kits/index.js')).href);
const base = new Set(SFX_NAMES);
const kitCues = Object.entries(SFX)
  .filter(([name]) => !base.has(name))
  .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  .map(([name, def]) => {
    // Only JSON-safe scalar metadata. The `build` / `loop` functions are code
    // (the Rust port synthesises them in sfx_table.rs), not data.
    const meta = {};
    for (const k of ['gain', 'max', 'jitter', 'reverb', 'minGap', 'type', 'pitch', 'dur']) {
      if (k in def && typeof def[k] !== 'function') meta[k] = def[k];
    }
    return { name, ...meta };
  });

const doc = {
  meta: {
    source: {
      kind: 'read-only-js-tree',
      commit,
      // Pinned to the live upstream tree; a single file drift changes `commit`.
      files: kitFiles,
      baseCount: SFX_NAMES.length,
      kitCount: kitCues.length,
    },
    note: 'Kit SFX self-register into SFX via src/game/kits/index.js. M4 docs said 42; live upstream registers 53. Functions (build/loop) are code, not data — the Rust synthesiser stays in sfx_table.rs.',
  },
  cues: kitCues,
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(doc) + '\n');
console.log(`extract-kit-sfx: ${kitCues.length} kit cues (base ${SFX_NAMES.length}) → ${outPath}`);
