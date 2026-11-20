// Task 69: extract the locker (character style catalog) from the READ-ONLY JS
// tree. Upstream has no locker.js — the face is character-style.js (tables +
// resolve/random/swatch), menus.js (LOCKER_TABS + setStyle causes + _syncMe),
// showcase.js (3D preview). This extractor slices/evaluates the literal tables
// so no number is hand-copied (NFR-M5-6). Output: rust/assets/locker.json.
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/locker.json');
const SRC_FILES = [
  'src/game/character-style.js',
  'src/ui/menus.js',
  'src/game/showcase.js',
];
const commit = upstreamCommit(repoRoot, SRC_FILES);

function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`expected fragment not found in ${where}: ${needle}`);
}
function sliceBlock(src, name, start, end) {
  const i = src.indexOf(start);
  if (i < 0) throw new Error(`${name}: start marker not found: ${start}`);
  const j = src.indexOf(end, i);
  if (j < 0) throw new Error(`${name}: end marker not found: ${end}`);
  return src.slice(i, j + end.length);
}

const styleSrc = readFileSync(resolve(repoRoot, SRC_FILES[0]), 'utf8');
const menusSrc = readFileSync(resolve(repoRoot, SRC_FILES[1]), 'utf8');
const showSrc = readFileSync(resolve(repoRoot, SRC_FILES[2]), 'utf8');

// Evaluate the literal table block (SKIN..PRESETS + wrap/resolveStyle/randomStyle/
// styleSwatch) standalone — it has no imports.
const tableStart = styleSrc.indexOf('export const SKIN_TONES');
const tableEnd = styleSrc.indexOf('export function applyStyleUniforms');
if (tableStart < 0 || tableEnd < 0) throw new Error('character-style.js table markers not found');
let tableSrc = styleSrc.slice(tableStart, tableEnd).replace(/export /g, '');
// styleSwatch's last statement is fine; append a return of everything we need.
const T = new Function(`${tableSrc}\nreturn {
  SKIN_TONES, SKIN_NAMES, OUTFITS, OUTFIT_NAMES, IRIS, IRIS_NAMES,
  HAIR_STYLE_NAMES, HAIR_STYLES, HATS, HAT_NAMES, BROWS, BROW_NAMES, PRESETS,
  wrap, resolveStyle, randomStyle, styleSwatch,
};`)();

// Table lengths / name alignment guards.
const aligned = [
  ['SKIN_TONES', T.SKIN_TONES, T.SKIN_NAMES],
  ['OUTFITS', T.OUTFITS, T.OUTFIT_NAMES],
  ['IRIS', T.IRIS, T.IRIS_NAMES],
  ['HATS', T.HATS, T.HAT_NAMES],
  ['BROWS', T.BROWS, T.BROW_NAMES],
];
for (const [name, table, names] of aligned) {
  if (table.length !== names.length) throw new Error(`${name}/${name}_NAMES length mismatch`);
}
if (T.HAIR_STYLES !== T.HAIR_STYLE_NAMES.length) throw new Error('HAIR_STYLES mismatch');
if (T.PRESETS.length !== 10) throw new Error('expected 10 presets');
for (const o of T.OUTFITS) {
  for (const k of ['shirt', 'shorts', 'shoe', 'sole', 'sock', 'strap', 'pattern']) {
    if (!(k in o)) throw new Error(`outfit missing ${k}`);
  }
}
for (const p of T.PRESETS) {
  for (const k of ['hair', 'skin', 'outfit', 'eyes', 'hat', 'brows']) {
    if (!(k in p.style)) throw new Error(`preset ${p.id} missing style.${k}`);
  }
}

// resolveStyle / randomStyle / styleSwatch behaviour pinned by reference calls.
const resolvedDefault = T.resolveStyle({}, 0);
const resolvedPartial = T.resolveStyle({ outfit: 13 }, 12345); // wraps 13 % 10 = 3
// P1 regression anchors: (a) outfit derivation MUST use the already-resolved
// hair/skin locals, not seed-recomputed ones; (b) JS `>>` is sign-propagating
// on int32 seeds (>= 2^31 must differ from a logical shift).
const resolvedHairSkin = T.resolveStyle({ hair: 5, skin: 3 }, 12345);
const resolvedHighSeed = T.resolveStyle({}, 2147483648); // 2^31, bit 31 set
const swatch = T.styleSwatch({ hair: 0, skin: 0, outfit: 0, eyes: 0, hat: 0, brows: 0 });
// randomStyle with a deterministic rng sequence.
let seq = 0;
const rng = () => [0.0, 0.99, 0.5, 0.2, 0.8, 0.33, 0.34, 0.1][seq++ % 8];
const randomOut = T.randomStyle(rng);

// LOCKER_TABS + setStyle causes from menus.js / showcase.js.
const tabsBlock = sliceBlock(menusSrc, 'LOCKER_TABS', 'const LOCKER_TABS = [', '\n];');
const tabs = [...tabsBlock.matchAll(/\{ id: '(\w+)', label: '([^']+)'/g)].map((m) => ({ id: m[1], label: m[2] }));
if (tabs.length !== 4) throw new Error(`expected 4 locker tabs, got ${tabs.length}`);
for (const cause of ['hair', 'eyes', 'skin', 'outfit', 'preset', 'random']) {
  assertContains(showSrc, `'${cause}'`, `showcase setStyle cause ${cause}`);
}
assertContains(menusSrc, 'net.setMe(ch)', '_syncMe network sync');
assertContains(menusSrc, 'ch.style = { ...style }', '_syncMe style diff');

const doc = {
  schema: 1,
  source: { commit, files: SRC_FILES },
  tables: {
    skin: { tones: T.SKIN_TONES, names: T.SKIN_NAMES },
    outfit: { items: T.OUTFITS, names: T.OUTFIT_NAMES },
    iris: { gradients: T.IRIS, names: T.IRIS_NAMES },
    hair: { count: T.HAIR_STYLES, names: T.HAIR_STYLE_NAMES },
    hat: { kinds: T.HATS, names: T.HAT_NAMES },
    brows: { kinds: T.BROWS, names: T.BROW_NAMES },
    presets: T.PRESETS,
  },
  reference: {
    resolved_default: resolvedDefault,
    resolved_partial: resolvedPartial,
    resolved_hair_skin: resolvedHairSkin,
    resolved_high_seed: resolvedHighSeed,
    swatch: swatch,
    random_out: randomOut,
  },
  ui: { tabs, set_style_causes: ['hair', 'eyes', 'skin', 'outfit', 'preset', 'random'] },
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('locker.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(
  `wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, ` +
  `skin=${T.SKIN_TONES.length} outfit=${T.OUTFITS.length} iris=${T.IRIS.length} presets=${T.PRESETS.length}`
);
