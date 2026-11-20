// Task 69 TR-69.1 independent crosscheck.
//
// Deliberately does NOT import extract-locker.mjs. It imports the READ-ONLY
// character-style.js ESM module directly (a different execution path than the
// extractor's slice+eval) and re-parses the tables/tabs from source with
// independent regexes, then reconciles against rust/assets/locker.json.
import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '../..');
const doc = JSON.parse(readFileSync(resolve(here, '../assets/locker.json'), 'utf8'));

let checks = 0;
function eq(actual, expected, label) {
  if (actual !== expected) throw new Error(`${label}: got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  checks++;
}
function near(actual, expected, label) {
  if (!Number.isFinite(actual) || Math.abs(actual - expected) > 1e-9) throw new Error(`${label}: got ${actual}, expected ${expected}`);
  checks++;
}

// Import the real module directly (independent of the extractor's slice+eval).
const S = await import(`file://${resolve(repoRoot, 'src/game/character-style.js')}`);

// ---- Tables: direct module values vs artifact ----------------------------
eq(doc.tables.skin.tones.length, S.SKIN_TONES.length, 'skin tones length');
eq(doc.tables.skin.names.length, S.SKIN_NAMES.length, 'skin names length');
for (let i = 0; i < S.SKIN_TONES.length; i++) {
  eq(doc.tables.skin.tones[i], S.SKIN_TONES[i], `skin tone ${i}`);
  eq(doc.tables.skin.names[i], S.SKIN_NAMES[i], `skin name ${i}`);
}
eq(doc.tables.outfit.items.length, S.OUTFITS.length, 'outfit length');
eq(doc.tables.outfit.names.length, S.OUTFIT_NAMES.length, 'outfit names length');
for (let i = 0; i < S.OUTFITS.length; i++) {
  eq(doc.tables.outfit.names[i], S.OUTFIT_NAMES[i], `outfit name ${i}`);
  for (const k of ['shirt', 'shorts', 'shoe', 'sole', 'sock', 'strap', 'pattern']) {
    if (k === 'pattern') near(doc.tables.outfit.items[i][k], S.OUTFITS[i][k], `outfit ${i}.pattern`);
    else eq(doc.tables.outfit.items[i][k], S.OUTFITS[i][k], `outfit ${i}.${k}`);
  }
}
eq(doc.tables.iris.gradients.length, S.IRIS.length, 'iris length');
for (let i = 0; i < S.IRIS.length; i++) {
  eq(doc.tables.iris.names[i], S.IRIS_NAMES[i], `iris name ${i}`);
  for (let k = 0; k < 2; k++) eq(doc.tables.iris.gradients[i][k], S.IRIS[i][k], `iris ${i}[${k}]`);
}
eq(doc.tables.hair.count, S.HAIR_STYLES, 'hair count');
eq(doc.tables.hair.names.length, S.HAIR_STYLE_NAMES.length, 'hair names length');
for (let i = 0; i < S.HAIR_STYLE_NAMES.length; i++) eq(doc.tables.hair.names[i], S.HAIR_STYLE_NAMES[i], `hair name ${i}`);
eq(doc.tables.hat.kinds.length, S.HATS.length, 'hat length');
for (let i = 0; i < S.HATS.length; i++) {
  eq(doc.tables.hat.kinds[i], S.HATS[i], `hat kind ${i}`);
  eq(doc.tables.hat.names[i], S.HAT_NAMES[i], `hat name ${i}`);
}
eq(doc.tables.brows.kinds.length, S.BROWS.length, 'brows length');
for (let i = 0; i < S.BROWS.length; i++) {
  eq(doc.tables.brows.kinds[i], S.BROWS[i], `brow kind ${i}`);
  eq(doc.tables.brows.names[i], S.BROW_NAMES[i], `brow name ${i}`);
}
eq(doc.tables.presets.length, S.PRESETS.length, 'preset length');
for (let i = 0; i < S.PRESETS.length; i++) {
  eq(doc.tables.presets[i].id, S.PRESETS[i].id, `preset ${i}.id`);
  eq(doc.tables.presets[i].name, S.PRESETS[i].name, `preset ${i}.name`);
  for (const k of ['hair', 'skin', 'outfit', 'eyes', 'hat', 'brows']) {
    eq(doc.tables.presets[i].style[k], S.PRESETS[i].style[k], `preset ${i}.${k}`);
  }
}

// ---- resolveStyle / randomStyle / styleSwatch: re-call the real module ----
for (const [label, got, want] of [
  ['resolved_default', doc.reference.resolved_default, S.resolveStyle({}, 0)],
  ['resolved_partial', doc.reference.resolved_partial, S.resolveStyle({ outfit: 13 }, 12345)],
  ['resolved_hair_skin', doc.reference.resolved_hair_skin, S.resolveStyle({ hair: 5, skin: 3 }, 12345)],
  ['resolved_high_seed', doc.reference.resolved_high_seed, S.resolveStyle({}, 2147483648)],
]) {
  for (const k of ['hair', 'skin', 'outfit', 'eyes', 'hat', 'brows']) eq(got[k], want[k], `${label}.${k}`);
}
const sw = S.styleSwatch({ hair: 0, skin: 0, outfit: 0, eyes: 0, hat: 0, brows: 0 });
for (const k of ['skin', 'shirt', 'shorts', 'shoe', 'sock', 'strap']) eq(doc.reference.swatch[k], sw[k], `swatch.${k}`);
for (let k = 0; k < 2; k++) eq(doc.reference.swatch.eyes[k], sw.eyes[k], `swatch.eyes[${k}]`);
let seq = 0;
const rng = () => [0.0, 0.99, 0.5, 0.2, 0.8, 0.33, 0.34, 0.1][seq++ % 8];
const rnd = S.randomStyle(rng);
for (const k of ['hair', 'skin', 'outfit', 'eyes', 'hat', 'brows']) eq(doc.reference.random_out[k], rnd[k], `random.${k}`);

// ---- APPEND-ONLY wrap/seed derivation: independent regex from source -----
const styleSrc = readFileSync(resolve(repoRoot, 'src/game/character-style.js'), 'utf8');
const wrapLine = styleSrc.split('\n').find((l) => l.includes('const wrap ='));
if (!wrapLine.includes('((Math.round(v) % n) + n) % n')) throw new Error('wrap formula drift');
checks++;
const resolveLine = styleSrc.split('\n').find((l) => l.includes('hair = wrap(st.hair'));
for (const token of ['seed % HAIR_STYLES', '(seed >> 3) % SKIN_TONES.length', '(hair + skin + (seed >> 6)) % OUTFITS.length', '(seed >> 9) % IRIS.length']) {
  if (!resolveLine.includes(token) && !styleSrc.includes(token)) throw new Error(`resolve derivation drift: ${token}`);
  checks++;
}
if (!styleSrc.includes('rng() < 0.34')) throw new Error('randomStyle hat probability drift');
checks++;

// ---- LOCKER_TABS + causes + network sync from menus/showcase -------------
const menusSrc = readFileSync(resolve(repoRoot, 'src/ui/menus.js'), 'utf8');
const showSrc = readFileSync(resolve(repoRoot, 'src/game/showcase.js'), 'utf8');
const tabsBlock = menusSrc.slice(menusSrc.indexOf('const LOCKER_TABS = ['), menusSrc.indexOf('\n];', menusSrc.indexOf('const LOCKER_TABS = [')));
const tabs = [...tabsBlock.matchAll(/\{ id: '(\w+)', label: '([^']+)'/g)].map((m) => ({ id: m[1], label: m[2] }));
eq(doc.ui.tabs.length, tabs.length, 'tabs length');
for (let i = 0; i < tabs.length; i++) {
  eq(doc.ui.tabs[i].id, tabs[i].id, `tab ${i}.id`);
  eq(doc.ui.tabs[i].label, tabs[i].label, `tab ${i}.label`);
}
for (const cause of doc.ui.set_style_causes) {
  if (!showSrc.includes(`'${cause}'`)) throw new Error(`setStyle cause missing upstream: ${cause}`);
  checks++;
}
if (!menusSrc.includes('net.setMe(ch)') || !menusSrc.includes('ch.style = { ...style }')) throw new Error('_syncMe style sync drift');
checks++;

// ---- upstream pin --------------------------------------------------------
const pinned = execSync(
  'git log -1 --format=%H -- src/game/character-style.js src/ui/menus.js src/game/showcase.js',
  { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] },
).toString().trim();
eq(doc.source.commit, pinned, 'source.commit');
eq(doc.source.files.length, 3, 'source.files length');
eq(doc.schema, 1, 'schema');

console.log(`locker-crosscheck: ${checks} checks OK (9 skin/10 outfit/8 iris/8 hair/4 hat/4 brows/10 presets, resolve/random/swatch, 4 tabs/6 causes, network sync, upstream pin)`);
