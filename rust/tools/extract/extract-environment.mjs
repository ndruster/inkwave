// Task M4-T55 batch A: extract the environment / post-process / quality data
// tables from the READ-ONLY JS tree (JS tree stays byte-clean).
//
// Same discipline as extract-tuning.mjs / extract-character.mjs: slice the
// literal blocks out of the sources and evaluate them standalone (no hand
// copied numbers), with assertContains guards that fail on upstream drift.
// Output is deterministic (stable-json): rust/assets/environment.json
//
//   src/world/environment.js  THEMES (day / sunset / golden)
//   src/core/renderer.js      BLOOM + GradeShader.uniforms defaults
//   src/config.js             DEFAULT_SETTINGS / QUALITY / TIMES / MAPS /
//                             mapTheme()
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');                // repository root
const outPath = resolve(here, '../../assets/environment.json');

const commit = upstreamCommit(repoRoot, [
  'src/world/environment.js',
  'src/core/renderer.js',
  'src/config.js',
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
const evalBlock = (block, name, extra = '') =>
  new Function(extra.args || '', `${block.replace(/^export\s+/, '')}\nreturn ${name};`)(...(extra.values || []));

const envSrc = readFileSync(resolve(repoRoot, 'src/world/environment.js'), 'utf8');
const rendererSrc = readFileSync(resolve(repoRoot, 'src/core/renderer.js'), 'utf8');
const configSrc = readFileSync(resolve(repoRoot, 'src/config.js'), 'utf8');

// ---- THEMES (environment.js L37-108) ------------------------------------
const srcThemes = sliceBlock(envSrc, 'environment.js', 'THEMES', 'const THEMES = {', '\n};');
for (const needle of [
  'day: {', 'sunset: {', 'golden: {',
  'sunIntensity: 3.3,', 'night: 1,', 'shafts: 1,',
  'fog: [25, 900],', 'bloom: [0.4, 0.55, 1.7],',
]) assertContains(srcThemes, needle, 'environment.js THEMES');
const THEMES = evalBlock(srcThemes, 'THEMES');

// ---- BLOOM + GradeShader defaults (renderer.js L15-62) ------------------
const srcBloom = sliceBlock(rendererSrc, 'renderer.js', 'BLOOM', 'const BLOOM = [', '];');
assertContains(srcBloom, '0.28, 0.45, 2.4', 'renderer.js BLOOM');
const BLOOM = evalBlock(srcBloom, 'BLOOM');

const srcGrade = sliceBlock(rendererSrc, 'renderer.js', 'GradeShader', 'const GradeShader = {', '\n};');
for (const needle of [
  'uSat: { value: 1.08 }', 'uVignette: { value: 0.22 }',
  'uShadowTint: { value: new THREE.Vector3(0.975, 0.99, 1.035) }',
  'uExposure: { value: 1.0 }',
]) assertContains(srcGrade, needle, 'renderer.js GradeShader');
class Vector3 { constructor(x, y, z) { this.a = [x, y, z]; } toArray() { return this.a; } }
class Color { constructor(r, g, b) { this.a = [r, g, b]; } toArray() { return this.a; } }
const THREE = { Vector3, Color };
const gradeUniforms = new Function('THREE', `${srcGrade}\nreturn GradeShader.uniforms;`)(THREE);
const gradeDefaults = {};
for (const [k, v] of Object.entries(gradeUniforms)) {
  const val = v.value;
  if (typeof val === 'number') gradeDefaults[k] = val;
  else if (val && typeof val.toArray === 'function') gradeDefaults[k] = val.toArray();
  // tDiffuse (null) and other sampler uniforms are deliberately omitted.
}
for (const k of ['uSat', 'uVib', 'uContrast', 'uLift', 'uExposure', 'uVignette', 'uHurt', 'uFlash', 'uAspect',
  'uShadowTint', 'uHighTint', 'uHurtColor']) {
  if (!(k in gradeDefaults)) throw new Error(`renderer.js GradeShader: missing uniform ${k}`);
}

// ---- settings / quality (config.js L562-591) ----------------------------
const srcSettings = sliceBlock(configSrc, 'config.js', 'DEFAULT_SETTINGS', 'export const DEFAULT_SETTINGS = {', '\n};');
assertContains(srcSettings, "quality: 'high',", 'config.js DEFAULT_SETTINGS');
assertContains(srcSettings, 'shadows: true,', 'config.js DEFAULT_SETTINGS');
const DEFAULT_SETTINGS = evalBlock(srcSettings, 'DEFAULT_SETTINGS');

const srcQuality = sliceBlock(configSrc, 'config.js', 'QUALITY', 'export const QUALITY = {', '\n};');
for (const needle of [
  'pixelRatio: 0.75,', 'pixelRatio: 1.0,', 'pixelRatio: 1.5,', 'pixelRatio: 2.0,',
  'shadowSize: 4096,', 'ao: true,',
]) assertContains(srcQuality, needle, 'config.js QUALITY');
const QUALITY = evalBlock(srcQuality, 'QUALITY');

// ---- TIMES / MAPS / mapTheme (config.js L512-525) -----------------------
const srcTimes = sliceBlock(configSrc, 'config.js', 'TIMES', 'export const TIMES =', '\n');
assertContains(srcTimes, "['day', 'dusk']", 'config.js TIMES');
const TIMES = evalBlock(srcTimes, 'TIMES');

const srcMapTheme = sliceBlock(configSrc, 'config.js', 'mapTheme', 'export const mapTheme =', '\n');
assertContains(srcMapTheme, 'map.times[time]', 'config.js mapTheme');
const mapTheme = evalBlock(srcMapTheme, 'mapTheme');

const srcMaps = sliceBlock(configSrc, 'config.js', 'MAPS', 'export const MAPS = [', '\n];');
assertContains(srcMaps, "id: 'halyard'", 'config.js MAPS');
assertContains(srcMaps, 'dusk: \'sunset\'', 'config.js MAPS');
const MAPS = evalBlock(srcMaps, 'MAPS');

// Evaluate the real mapTheme() for every stage × time so Rust never
// re-implements the fallback chain (≡ "no hand-copied numbers" rule).
const mapThemes = {};
for (const m of MAPS) {
  mapThemes[m.id] = { theme: m.theme || 'day', flags: {} };
  for (const t of TIMES) mapThemes[m.id][t] = mapTheme(m, t);
  for (const k of ['onlineOnly', 'noBots', 'noBoss']) if (m[k]) mapThemes[m.id].flags[k] = true;
}
for (const [id, row] of Object.entries(mapThemes)) {
  if (!(row.day in THEMES) || !(row.dusk in THEMES)) {
    throw new Error(`config.js MAPS: ${id} maps to a theme outside THEMES: ${row.day}/${row.dusk}`);
  }
}

const doc = {
  schema: 1,
  source: {
    commit,
    files: [
      'src/world/environment.js',
      'src/core/renderer.js',
      'src/config.js',
    ],
  },
  themes: THEMES,
  gradeDefaults,
  bloomDefault: BLOOM,
  defaultSettings: DEFAULT_SETTINGS,
  quality: QUALITY,
  times: TIMES,
  mapThemes,
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('environment.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(`wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, themes=${Object.keys(THEMES).length}, maps=${MAPS.length}`);
