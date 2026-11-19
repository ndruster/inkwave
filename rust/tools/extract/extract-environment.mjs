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
import { createHash } from 'node:crypto';
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

// ---- Task 66: cloud / PMREM / reflection / sea BAKE constants -------------
// Scalars are located by source-fragment assertions then narrow regexes, so
// upstream drift fails the pipeline instead of silently changing the artifact.
function lineOf(src, needle, where) {
  const hit = src.split('\n').find((l) => l.includes(needle));
  if (!hit) throw new Error(`${where}: no line with ${needle}`);
  return hit;
}
function numAfter(s, needle) {
  const i = s.indexOf(needle);
  if (i < 0) throw new Error(`no ${needle} in ${s}`);
  const m = s.slice(i + needle.length).match(/-?\d+(?:\.\d+)?/);
  if (!m) throw new Error(`no number after ${needle} in ${s}`);
  return Number(m[0]);
}

// Cloud bake: CLOUD_W/H, default seed/cov, GLSL step constants.
assertContains(envSrc, 'const CLOUD_W = 2048, CLOUD_H = 640;', 'environment.js cloud size');
const cloudSize = lineOf(envSrc, 'const CLOUD_W =', 'CLOUD_W');
const cloudSeedLine = lineOf(envSrc, 'uSeed: { value: 3.0 }', 'cloud uSeed');
const cloud = {
  width: numAfter(cloudSize, 'CLOUD_W = '),
  height: numAfter(cloudSize, 'CLOUD_H = '),
  seed: numAfter(cloudSeedLine, 'uSeed: { value: '),
  coverage: numAfter(lineOf(envSrc, 'uCov: { value: 0.46 }', 'cloud uCov'), 'uCov: { value: '),
  scan_bands: numAfter(lineOf(envSrc, 'const strips = 10', 'cloud strips'), 'strips = '),
};
for (const needle of ['HB = 1150', 'CELL = 2700', 'RMAX = 55000', 'TOPMAX = 3300', 'dt = 16']) {
  assertContains(envSrc, needle, 'environment.js cloud GLSL constants');
}

// PMREM + env intensity default.
assertContains(envSrc, 'new THREE.PMREMGenerator(', 'environment.js PMREM');
const envKLine = lineOf(envSrc, 'scene.environmentIntensity = T.envK ?? 0.66', 'envK default');
const pmrem = {
  size: numAfter(lineOf(envSrc, '{ size: 256 }', 'PMREM size'), 'size: '),
  env_k_default: numAfter(envKLine, 'T.envK ?? '),
};

// Far reflection (cube 512, CubeCamera near/far) + planar reflection tiers.
const farLine = lineOf(envSrc, 'new THREE.WebGLCubeRenderTarget(512', 'far cube');
const cubeCamLine = lineOf(envSrc, 'new THREE.CubeCamera(2, 30000', 'CubeCamera');
const far = {
  cube_size: numAfter(farLine, 'WebGLCubeRenderTarget('),
  cube_near: numAfter(cubeCamLine, 'CubeCamera('),
  cube_far: numAfter(cubeCamLine, 'CubeCamera(2, '),
};
const reflLine = lineOf(envSrc, "const scale = !this.reflections || q === 'low' ? 0 :", 'planar reflection scale');
for (const needle of ["q === 'medium' ? 0.28", "q === 'ultra' ? 0.5", ': 0.4)']) {
  assertContains(reflLine, needle, 'environment.js reflection tier');
}
const reflection = {
  tier_scale: { low: 0, medium: 0.28, high: 0.4, ultra: 0.5 },
};

// Sea wave texture (makeWaveTexture size/seed/wave count/wind/freq/amp).
const waveDef = lineOf(envSrc, 'function makeWaveTexture(size = 256, seed = 11)', 'makeWaveTexture');
const waveCountLine = lineOf(envSrc, 'for (let i = 0; i < 44; i++)', 'wave count');
const windLine = lineOf(envSrc, 'const wind = 0.55;', 'wave wind');
const freqLine = lineOf(envSrc, 'Math.pow(rnd(), 1.5) * 26', 'wave freq');
const ampLine = lineOf(envSrc, '1 / Math.pow(kl, 1.3)', 'wave amp');
const sea = {
  wave_texture_size: numAfter(waveDef, 'size = '),
  wave_seed: numAfter(waveDef, 'seed = '),
  wave_count: numAfter(waveCountLine, 'i < '),
  wind: numAfter(windLine, 'wind = '),
  freq_rnd_pow: numAfter(freqLine, 'rnd(), '),
  freq_max: numAfter(freqLine, '* '),
  amp_pow: numAfter(ampLine, 'kl, '),
};

// Swell: three (k, phase, speed, amplitude) components parsed from the actual
// GLSL lines (environment.js L477-479) — NO hand-copied constants.
const swellLineRe = /k\s*=\s*vec2\(([-\d.]+),\s*([-\d.]+)\);\s*ph\s*=\s*dot\(p,\s*k\)\s*\+\s*t\s*\*\s*([\d.]+)(?:\s*\+\s*([\d.]+))?;\s*h\s*\+=\s*([\d.]+)\s*\*\s*sin\(ph\)/g;
const swellMatches = [...envSrc.matchAll(swellLineRe)];
if (swellMatches.length !== 3) {
  throw new Error(`postprocess extract: expected 3 swell components, got ${swellMatches.length}`);
}
const swell = {
  components: swellMatches.map((m) => ({
    k: [Number(m[1]), Number(m[2])],
    speed: Number(m[3]),
    phase: m[4] ? Number(m[4]) : 0.0,
    amp: Number(m[5]),
  })),
  amp_min: numAfter(lineOf(envSrc, 'mix(0.035, 0.16, smoothstep(3.0, 70.0', 'swellAmp'), 'mix('),
  amp_max: numAfter(lineOf(envSrc, 'mix(0.035, 0.16, smoothstep(3.0, 70.0', 'swellAmp'), 'mix(0.035, '),
  deck_min: numAfter(lineOf(envSrc, 'smoothstep(3.0, 70.0, dDeck)', 'swellAmp step'), 'smoothstep('),
  deck_max: numAfter(lineOf(envSrc, 'smoothstep(3.0, 70.0, dDeck)', 'swellAmp step'), 'smoothstep(3.0, '),
};

// Task 66 TR-66.1: bit-exact reference reimplementation of the upstream
// `makeWaveTexture` wave constructor (environment.js L1262-1273), used ONLY
// to produce a SHA256 golden that the Rust bake must match. This is an
// independent JS reference path — not self-verification of the Rust code.
function mulberry32(a) {
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
function referenceWaveDescriptors(spec) {
  const rnd = mulberry32(spec.wave_seed);
  const waves = [];
  for (let i = 0; i < spec.wave_count; i++) {
    const f = 2 + Math.pow(rnd(), spec.freq_rnd_pow) * spec.freq_max;
    const ang = spec.wind + (rnd() - 0.5) * 2.4;
    let kx = Math.round(Math.cos(ang) * f), ky = Math.round(Math.sin(ang) * f);
    if (kx === 0 && ky === 0) kx = 2;
    const kl = Math.hypot(kx, ky);
    const amp = 1 / Math.pow(kl, spec.amp_pow);
    const phase = rnd() * Math.PI * 2;
    waves.push([kx, ky, amp, phase]);
  }
  return waves;
}
// Canonical binary form shared with the Rust bake test: per wave,
// kx(i32 LE) + ky(i32 LE) + round(amp·1e9)(i64 LE) + round(phase·1e9)(i64 LE)
// = 24 bytes. Amp/phase are quantized to 1e-9 so the golden pins the FORMULA
// (RNG/exponents/constants) without depending on libm's last-bit pow rounding.
const WAVE_Q = 1e9;
const _waves = referenceWaveDescriptors(sea);
const _buf = Buffer.alloc(_waves.length * 24);
{
  let o = 0;
  for (const [kx, ky, amp, phase] of _waves) {
    _buf.writeInt32LE(kx, o); o += 4;
    _buf.writeInt32LE(ky, o); o += 4;
    _buf.writeBigInt64LE(BigInt(Math.round(amp * WAVE_Q)), o); o += 8;
    _buf.writeBigInt64LE(BigInt(Math.round(phase * WAVE_Q)), o); o += 8;
  }
}
const waveGolden = createHash('sha256').update(_buf).digest('hex');

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
  bake: {
    cloud,
    pmrem,
    far,
    reflection,
    sea,
    swell,
    wave_sha256: waveGolden,
  },
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('environment.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(`wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, themes=${Object.keys(THEMES).length}, maps=${MAPS.length}`);
