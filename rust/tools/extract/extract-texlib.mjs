// M4 Task 52 batch A texlib extractor (spec.md D-T52-1, TR-52.1).
//
// Emits rust/assets/texlib.json (schema inkwave.texlib.v1): the procedural
// texture-library metadata + the levelMaterial slot tables + the ink/lamp/
// material constants the Rust renderer needs. The GLSL bodies (prep/surf/
// TEXLIB_GLSL) are NOT carried here — they are hand-ported to WGSL/Rust
// (batches B/C/D) against the source line anchors pinned below.
//
//   layers: the 25 MATERIALS entries (texlib.js L211-1163, meta fields only,
//     prep/surf stripped) + the 21 stage layers appended exactly like
//     texlib.js L1167 ({...s.mat, name: s.name, group: s.group}).
//   slots: the levelMaterial.js L79-91 slot table — `map` (the eval'd literal,
//     50 entries incl. the trailing TL_SIDE), and the onWall/onTop tables
//     resolved to per-slot arrays (base tables + the STAGE_SURFACES loop).
//     uTL/uTLt/uTLs (L96-99) are built by the Rust side from these inputs.
//   ink / lamps / mat / tex: constants sliced from inkShading.js L22,
//     levelMaterial.js L11/L40-41/L50/L62 and texlib.js L1332/L185.
//
// Extraction pattern ≡ extract-tuning.mjs: literal blocks are sliced from the
// sources and eval'd standalone (assertContains pins the surrounding source
// lines, so upstream drift fails the pipeline instead of silently changing the
// artifact). `src/world/stages/surfaces.js` IS imported directly — it only
// pulls the seven stage surfaces files (no THREE / browser globals), same
// precedent as extract-tuning importing src/config.js.
//
// Output goes through stableStringify (sorted keys, no timestamps) so a re-run
// against the same upstream commit is byte-identical.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { upstreamCommit } from './upstream-commit.mjs';
import { STAGE_SURFACES, FIRST_STAGE_SLOT, LAST_STAGE_SLOT } from '../../../src/world/stages/surfaces.js';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));   // rust/tools/extract
const repoRoot = resolve(here, '../../..');             // repository root
const outPath = resolve(here, '../../assets/texlib.json');

const commit = upstreamCommit(repoRoot, [
  'src/world/texlib.js',
  'src/world/levelMaterial.js',
  'src/world/inkShading.js',
  'src/world/stages/surfaces.js',
  'src/world/maps.js',
]);

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
// Parse a bare number out of `label: <num>` inside an already-sliced block
// (so no constant is ever retyped here).
function numIn(block, label, where) {
  const i = block.indexOf(label);
  if (i < 0) throw new Error(`${where}: number not found after "${label}"`);
  const m = block.slice(i).match(/-?\d+(?:\.\d+)?(?:e-?\d+)?/);
  if (!m) throw new Error(`${where}: cannot parse number after "${label}"`);
  return Number(m[0]);
}
function strIn(block, label, where) {
  const i = block.indexOf(label);
  if (i < 0) throw new Error(`${where}: string not found after "${label}"`);
  const m = block.slice(i).match(/'([^']*)'/);
  if (!m) throw new Error(`${where}: cannot parse string after "${label}"`);
  return m[1];
}

const texlibSrc = readFileSync(resolve(repoRoot, 'src/world/texlib.js'), 'utf8');
const levelMatSrc = readFileSync(resolve(repoRoot, 'src/world/levelMaterial.js'), 'utf8');
const inkSrc = readFileSync(resolve(repoRoot, 'src/world/inkShading.js'), 'utf8');

// --- TEXLIB_MODE (texlib.js L85) — the mode/sym vocab the MATERIALS literal
// references through the destructured PLAIN/GRID/HEX (L86).
const srcMODE = sliceBlock(texlibSrc, 'texlib.js', 'TEXLIB_MODE', 'export const TEXLIB_MODE = {', ' };');
assertContains(srcMODE, 'plain: 0, grid: 1, hex: 2', 'texlib.js TEXLIB_MODE');
const MODE = new Function(`${srcMODE.replace(/^export /, '')}\nreturn TEXLIB_MODE;`)();
const { plain: PLAIN, grid: GRID, hex: HEX } = MODE;

// --- MATERIALS (texlib.js L211-1163): slice + eval, strip the GLSL bodies.
const srcMATERIALS = sliceBlock(texlibSrc, 'texlib.js', 'MATERIALS', 'const MATERIALS = [', '\n];');
assertContains(srcMATERIALS, "name: 'concrete', detail: 1.0, scale: 4.0", 'texlib.js MATERIALS');
assertContains(srcMATERIALS, "name: 'gel',", 'texlib.js MATERIALS');
assertContains(srcMATERIALS, 'stair: [0.28, 4, 0.5, 1]', 'texlib.js MATERIALS');
const MATERIALS = new Function('PLAIN', 'GRID', 'HEX', `${srcMATERIALS}\nreturn MATERIALS;`)(PLAIN, GRID, HEX);
if (MATERIALS.length !== 25) throw new Error(`texlib.js MATERIALS: expected 25 layers, got ${MATERIALS.length}`);

// The meta fields the artifact carries (everything except prep/surf, which the
// Rust side hand-ports against the source anchors). `group` defaults to 0
// exactly like the JS group builder (texlib.js L1367 `m.group || 0`).
function layerMeta(m) {
  return {
    name: m.name, group: m.group || 0, detail: m.detail, scale: m.scale,
    ...(m.tint !== undefined ? { tint: m.tint } : {}),
    ...(m.mask !== undefined ? { mask: m.mask } : {}),
    ...(m.stair !== undefined ? { stair: m.stair } : {}),
    alpha: m.alpha, mode: m.mode, sym: m.sym, hr: m.hr, ao: m.ao,
  };
}

// --- Stage layers, appended exactly like texlib.js L1167
// `for (const s of STAGE_SURFACES) MATERIALS.push({ ...s.mat, name: s.name, group: s.group });`
const stageCount = STAGE_SURFACES.length;
if (stageCount !== 21) throw new Error(`stages/surfaces.js: expected 21 stage surfaces, got ${stageCount}`);
const layers = [
  ...MATERIALS.map(layerMeta),
  ...STAGE_SURFACES.map((s) => layerMeta({ ...s.mat, name: s.name, group: s.group })),
];

// --- Slot table (levelMaterial.js L79-91). The `map` literal is eval'd with
// the same STAGE_SURFACES / FIRST_STAGE_SLOT / LAST_STAGE_SLOT bindings the
// module itself has (its import, L3), so the stage-slot fill and the concrete
// fallbacks reproduce exactly.
const srcMAP = sliceBlock(levelMatSrc, 'levelMaterial.js', 'map', "const map = ['concrete'", "'concrete'];");
assertContains(srcMAP, "'treads', 'stonestep', 'rampboard', 'gangdeck',", 'levelMaterial.js map');
assertContains(srcMAP, 'STAGE_SURFACES.find((s) => s.slot === FIRST_STAGE_SLOT + k)', 'levelMaterial.js map');
const map = new Function('STAGE_SURFACES', 'FIRST_STAGE_SLOT', 'LAST_STAGE_SLOT',
  `${srcMAP}\nreturn map;`)(STAGE_SURFACES, FIRST_STAGE_SLOT, LAST_STAGE_SLOT);
const SIDE = map.length - 1;
if (map.length !== 50) throw new Error(`levelMaterial.js map: expected 50 slots, got ${map.length}`);

// onWall/onTop: the base literals + the STAGE_SURFACES remap loop (L89-91),
// then resolved to per-slot arrays (the shader reads uTLt.z/w = onWall[i] ?? i
// / onTop[i] ?? i, L97).
const srcONW = sliceBlock(levelMatSrc, 'levelMaterial.js', 'onWall', 'const onWall = {', ' };');
assertContains(srcONW, '4: SIDE, 10: SIDE, 21: SIDE, 19: 18, 24: 18, 25: 23, 26: 17, 27: 11', 'levelMaterial.js onWall');
const srcONT = sliceBlock(levelMatSrc, 'levelMaterial.js', 'onTop', 'const onTop = {', ' };');
assertContains(srcONT, '20: 17', 'levelMaterial.js onTop');
const srcLOOP = sliceBlock(levelMatSrc, 'levelMaterial.js', 'stage remap loop', 'for (const s of STAGE_SURFACES) { if (s.onWall', '\n');
assertContains(srcLOOP, 'if (s.onTop != null) onTop[s.slot] = s.onTop; }', 'levelMaterial.js stage remap loop');
const { onWall, onTop } = new Function('SIDE', 'STAGE_SURFACES',
  `${srcONW}\n${srcONT}\n${srcLOOP}\nreturn { onWall, onTop };`)(SIDE, STAGE_SURFACES);
const onWallArr = map.map((_, i) => onWall[i] ?? i);
const onTopArr = map.map((_, i) => onTop[i] ?? i);

// --- ink constants (inkShading.js L22; levelMaterial.js L47-50/L62).
const srcRIP = sliceBlock(inkSrc, 'inkShading.js', 'INK_RIPPLES', 'export const INK_RIPPLES =', '\n');
assertContains(srcRIP, 'const INK_RIPPLES = 24;', 'inkShading.js INK_RIPPLES');
const INK_RIPPLES = new Function(`${srcRIP.replace(/^export /, '')}\nreturn INK_RIPPLES;`)();
const ink = {
  ripples: INK_RIPPLES,
  glow: numIn(levelMatSrc, 'uInkGlow: { value:', 'levelMaterial.js uInkGlow'),
  teamA: strIn(levelMatSrc, "uTeamA: { value: new THREE.Color('", 'levelMaterial.js uTeamA'),
  teamB: strIn(levelMatSrc, "uTeamB: { value: new THREE.Color('", 'levelMaterial.js uTeamB'),
  ppm: numIn(levelMatSrc, 'uPpm: { value: opts.ppm ||', 'levelMaterial.js uPpm'),
};
assertContains(levelMatSrc, "uInkGlow: { value: 0.06 }", 'levelMaterial.js uInkGlow');
assertContains(levelMatSrc, "uTeamA: { value: new THREE.Color('#ff8a14') }", 'levelMaterial.js uTeamA');
assertContains(levelMatSrc, "uTeamB: { value: new THREE.Color('#2f5bff') }", 'levelMaterial.js uTeamB');
assertContains(levelMatSrc, 'uPpm: { value: opts.ppm || 20 }', 'levelMaterial.js uPpm');

// --- lamps (levelMaterial.js L11).
const srcLAMP = sliceBlock(levelMatSrc, 'levelMaterial.js', 'lamps', 'const MAX_LAMPS =', '\n');
assertContains(srcLAMP, "MAX_LAMPS = 12, LAMP_R = 10, LAMP_HEX = '#ffc48a', LAMP_I = 16", 'levelMaterial.js lamps');
const lamps = new Function(`${srcLAMP}\nreturn { MAX_LAMPS, LAMP_R, LAMP_HEX, LAMP_I };`)();

// --- material baseline (levelMaterial.js L40-41) + texlib bake geometry
// (texlib.js L1332 size default / L4 header low-quality size / L185 2×2
// sub-sample loop).
const srcMAT = sliceBlock(levelMatSrc, 'levelMaterial.js', 'material baseline', 'roughness: 0.82', '\n  });');
assertContains(srcMAT, 'metalness: 0.0,', 'levelMaterial.js material baseline');
assertContains(srcMAT, 'clearcoat: 1.0, clearcoatRoughness: 0.08,', 'levelMaterial.js material baseline');
const mat = {
  roughness: numIn(srcMAT, 'roughness:', 'levelMaterial.js roughness'),
  metalness: numIn(srcMAT, 'metalness:', 'levelMaterial.js metalness'),
  clearcoat: numIn(srcMAT, 'clearcoat:', 'levelMaterial.js clearcoat'),
  clearcoatRoughness: numIn(srcMAT, 'clearcoatRoughness:', 'levelMaterial.js clearcoatRoughness'),
};
const srcSIZE = sliceBlock(texlibSrc, 'texlib.js', 'createTextureLibrary', 'export async function createTextureLibrary(renderer, { size =', '\n');
assertContains(srcSIZE, 'size = 512 } = {}', 'texlib.js createTextureLibrary');
assertContains(texlibSrc, '// 256 for low quality', 'texlib.js header');
assertContains(texlibSrc, 'for (int q = 0; q < 4 * uOne; q++)', 'texlib.js GEN_MAIN 2×2 sub-sample');
const tex = { sizeHigh: numIn(srcSIZE, 'size =', 'texlib.js size default'), sizeLow: 256, sub: 2 };

const doc = {
  schema: 'inkwave.texlib.v1',
  source: { commit, path: 'src/world/texlib.js + src/world/levelMaterial.js + src/world/inkShading.js + src/world/stages/surfaces.js' },
  layers,
  slots: { map, onWall: onWallArr, onTop: onTopArr },
  ink,
  lamps: { max: lamps.MAX_LAMPS, radius: lamps.LAMP_R, hex: lamps.LAMP_HEX, intensity: lamps.LAMP_I },
  mat,
  tex,
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(doc));
console.log(
  `texlib.json <- texlib.js + levelMaterial.js + inkShading.js + stages @ ${commit} ` +
  `(${layers.length} layers = 25 base + ${stageCount} stage, ${map.length} slots)`,
);
