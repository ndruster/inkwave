// Task 55 TR-55.1 / D-T55-A independent crosscheck.
//
// Deliberately does NOT import extract-environment.mjs: it re-parses the
// READ-ONLY JS sources with plain regexes (a different code path) and
// reconciles them against rust/assets/environment.json. Any upstream drift
// or extractor regression fails the gate's XCHK stage.
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools
const repoRoot = resolve(here, '../..');                   // repository root
const doc = JSON.parse(readFileSync(resolve(here, '../assets/environment.json'), 'utf8'));
const config = readFileSync(resolve(repoRoot, 'src/config.js'), 'utf8');
const envSrc = readFileSync(resolve(repoRoot, 'src/world/environment.js'), 'utf8');
const renderer = readFileSync(resolve(repoRoot, 'src/core/renderer.js'), 'utf8');

let checks = 0;
function eq(actual, expected, label) {
  if (actual !== expected) throw new Error(`${label}: got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  checks++;
}
function near(actual, expected, label) {
  if (Math.abs(actual - expected) > 1e-9) throw new Error(`${label}: got ${actual}, expected ${expected}`);
  checks++;
}
function matches(src, re, label) {
  const global = re.flags.includes('g') ? re : new RegExp(re.source, `${re.flags}g`);
  const out = [...src.matchAll(global)];
  if (out.length === 0) throw new Error(`${label}: no matches — upstream drift?`);
  return out;
}

// ---- QUALITY: four rows, every field ------------------------------------
const qre = /(low|medium|high|ultra):\s*\{\s*pixelRatio:\s*([\d.]+),\s*shadowSize:\s*(\d+),\s*msaa:\s*(\d+),\s*bloom:\s*(true|false),\s*ao:\s*(true|false),\s*paintAtlas:\s*(\d+),\s*particles:\s*([\d.]+)\s*\}/g;
const qRows = matches(config, qre, 'QUALITY rows');
eq(qRows.length, 4, 'QUALITY row count');
for (const row of qRows) {
  const [, name, pixelRatio, shadowSize, msaa, bloom, ao, paintAtlas, particles] = row;
  const r = doc.quality[name];
  if (!r) throw new Error(`QUALITY.${name} missing from environment.json`);
  near(r.pixelRatio, Number(pixelRatio), `${name}.pixelRatio`);
  eq(r.shadowSize, Number(shadowSize), `${name}.shadowSize`);
  eq(r.msaa, Number(msaa), `${name}.msaa`);
  eq(r.bloom, bloom === 'true', `${name}.bloom`);
  eq(r.ao, ao === 'true', `${name}.ao`);
  eq(r.paintAtlas, Number(paintAtlas), `${name}.paintAtlas`);
  near(r.particles, Number(particles), `${name}.particles`);
}

// ---- THEMES: source order is day / sunset / golden ----------------------
const themeOrder = ['day', 'sunset', 'golden'];
const intensities = matches(envSrc, /sunIntensity:\s*([\d.]+),/g, 'THEMES sunIntensity').map((m) => Number(m[1]));
const nights = matches(envSrc, /night:\s*(\d),/g, 'THEMES night').map((m) => Number(m[1]));
const fogs = matches(envSrc, /fog:\s*\[([\d.]+),\s*([\d.]+)\]/g, 'THEMES fog').map((m) => [Number(m[1]), Number(m[2])]);
eq(intensities.length, 3, 'sunIntensity count');
eq(nights.length, 3, 'night count');
eq(fogs.length, 3, 'fog count');
for (let i = 0; i < themeOrder.length; i++) {
  const t = doc.themes[themeOrder[i]];
  if (!t) throw new Error(`THEMES.${themeOrder[i]} missing from environment.json`);
  near(t.sunIntensity, intensities[i], `${themeOrder[i]}.sunIntensity`);
  eq(t.night, nights[i], `${themeOrder[i]}.night`);
  eq(t.fog.length, 2, `${themeOrder[i]}.fog length`);
  near(t.fog[0], fogs[i][0], `${themeOrder[i]}.fog[0]`);
  near(t.fog[1], fogs[i][1], `${themeOrder[i]}.fog[1]`);
}
// sunset's extra bloom triple — the only theme-level bloom override.
const themeBloom = matches(envSrc, /bloom:\s*\[([\d.]+),\s*([\d.]+),\s*([\d.]+)\]/g, 'THEMES bloom');
eq(themeBloom.length, 1, 'theme bloom override count');
eq(doc.themes.sunset.grade.bloom.length, 3, 'sunset.grade.bloom length');
for (let i = 0; i < 3; i++) near(doc.themes.sunset.grade.bloom[i], Number(themeBloom[0][i + 1]), `sunset.grade.bloom[${i}]`);
eq(doc.themes.day.grade.bloom, undefined, 'day has no theme bloom override');

// ---- renderer defaults ---------------------------------------------------
const rendererBloom = matches(renderer, /const BLOOM = \[([\d.]+),\s*([\d.]+),\s*([\d.]+)\]/, 'BLOOM');
for (let i = 0; i < 3; i++) near(doc.bloomDefault[i], Number(rendererBloom[0][i + 1]), `bloomDefault[${i}]`);
const gradeKeys = {
  uSat: 'uSat', uVib: 'uVib', uContrast: 'uContrast', uLift: 'uLift',
  uExposure: 'uExposure', uVignette: 'uVignette', uHurt: 'uHurt',
  uFlash: 'uFlash', uAspect: 'uAspect',
};
for (const [docKey, jsKey] of Object.entries(gradeKeys)) {
  const m = matches(renderer, new RegExp(`${jsKey}:\\s*\\{\\s*value:\\s*([\\d.]+)\\s*\\}`), `GradeShader.${jsKey}`);
  near(doc.gradeDefaults[docKey], Number(m[0][1]), `gradeDefaults.${docKey}`);
}
const vec = (name) => {
  const m = matches(renderer, new RegExp(`${name}:\\s*\\{\\s*value:\\s*new THREE\\.Vector3\\(([^)]*)\\)\\s*\\}`), `GradeShader.${name}`);
  return m[0][1].split(',').map((s) => Number(s.trim()));
};
for (const k of ['uShadowTint', 'uHighTint']) {
  const got = doc.gradeDefaults[k];
  const want = vec(k);
  eq(got.length, 3, `gradeDefaults.${k} length`);
  for (let i = 0; i < 3; i++) near(got[i], want[i], `gradeDefaults.${k}[${i}]`);
}
// Task 65 D-T65-2: uHurtColor is a THREE.Color (not Vector3); extend the
// 12th-uniform coverage that was previously skipped here.
{
  const m = matches(renderer, /uHurtColor:\s*\{\s*value:\s*new THREE\.Color\(([^)]*)\)\s*\}/, 'GradeShader.uHurtColor');
  const want = m[0][1].split(',').map((s) => Number(s.trim()));
  const got = doc.gradeDefaults.uHurtColor;
  eq(got.length, 3, 'gradeDefaults.uHurtColor length');
  for (let i = 0; i < 3; i++) near(got[i], want[i], `gradeDefaults.uHurtColor[${i}]`);
}

// ---- DEFAULT_SETTINGS + MAPS.times → mapThemes ---------------------------
eq(doc.defaultSettings.quality, matches(config, /quality:\s*'(high|medium|low|ultra)'/, 'DEFAULT_SETTINGS.quality')[0][1], 'default quality');
eq(doc.defaultSettings.shadows, true, 'default shadows');
eq(doc.defaultSettings.bloom, true, 'default bloom');
const mapRows = matches(config, /id:\s*'(\w+)',[^\n]*?times:\s*\{\s*day:\s*'(\w+)',\s*dusk:\s*'(\w+)'\s*\}/g, 'MAPS.times');
eq(mapRows.length, 8, 'MAPS.times row count');
for (const m of mapRows) {
  const [, id, day, dusk] = m;
  const row = doc.mapThemes[id];
  if (!row) throw new Error(`mapThemes.${id} missing from environment.json`);
  eq(row.day, day, `${id}.day theme`);
  eq(row.dusk, dusk, `${id}.dusk theme`);
  if (!(day in doc.themes) || !(dusk in doc.themes)) throw new Error(`${id}: theme outside THEMES`);
}

// ---- Task 66: cloud / PMREM / reflection / sea BAKE constants -------------
const bake = doc.bake;
if (!bake) throw new Error('environment.json missing bake section (Task 66)');
// Cloud: CLOUD_W/H + defaults + strips, independent numeric regex.
const cw = matches(envSrc, /const CLOUD_W = (\d+), CLOUD_H = (\d+);/, 'CLOUD_W/H')[0];
eq(bake.cloud.width, Number(cw[1]), 'bake.cloud.width');
eq(bake.cloud.height, Number(cw[2]), 'bake.cloud.height');
const cloudSeed = matches(envSrc, /u\.uSeed\.value = T\.cloudSeed \?\? ([\d.]+);/, 'cloud seed default')[0];
const cloudCov = matches(envSrc, /u\.uCov\.value = T\.cloudCov \?\? ([\d.]+);/, 'cloud cov default')[0];
near(bake.cloud.seed, Number(cloudSeed[1]), 'bake.cloud.seed');
near(bake.cloud.coverage, Number(cloudCov[1]), 'bake.cloud.coverage');
eq(bake.cloud.scan_bands, Number(matches(envSrc, /const strips = (\d+), h =/, 'cloud strips')[0][1]), 'bake.cloud.scan_bands');
// PMREM size + envK default.
eq(bake.pmrem.size, Number(matches(envSrc, /fromScene\([^)]*\{ size: (\d+) \}\)/, 'PMREM size')[0][1]), 'bake.pmrem.size');
near(bake.pmrem.env_k_default, Number(matches(envSrc, /environmentIntensity = T\.envK \?\? ([\d.]+)/, 'envK default')[0][1]), 'bake.pmrem.env_k_default');
// Far reflection cube + CubeCamera tiers.
const cube = matches(envSrc, /WebGLCubeRenderTarget\((\d+),/, 'far cube size')[0];
const cam = matches(envSrc, /new THREE\.CubeCamera\((\d+),\s*(\d+),/, 'CubeCamera')[0];
eq(bake.far.cube_size, Number(cube[1]), 'bake.far.cube_size');
eq(bake.far.cube_near, Number(cam[1]), 'bake.far.cube_near');
eq(bake.far.cube_far, Number(cam[2]), 'bake.far.cube_far');
// Reflection tier scales parsed from the actual _renderReflection ternary.
const reflLine = matches(envSrc, /const scale = !this\.reflections \|\| q === 'low' \? 0 :[^\n]+/, 'reflection scale')[0][0];
for (const [tier, re] of [
  ['low', /q === 'low' \? (\d+)/],
  ['medium', /q === 'medium' \? ([\d.]+)/],
  ['ultra', /q === 'ultra' \? ([\d.]+)/],
  ['high', /: ([\d.]+)\) \*/],
]) {
  near(bake.reflection.tier_scale[tier], Number(reflLine.match(re)[1]), `reflection tier ${tier}`);
}
// Sea wave texture + makeWaveTexture constants.
const waveDef = matches(envSrc, /function makeWaveTexture\(size = (\d+), seed = (\d+)\)/, 'wave texture def')[0];
eq(bake.sea.wave_texture_size, Number(waveDef[1]), 'bake.sea.wave_texture_size');
eq(bake.sea.wave_seed, Number(waveDef[2]), 'bake.sea.wave_seed');
eq(bake.sea.wave_count, Number(matches(envSrc, /for \(let i = 0; i < (\d+); i\+\+\) \{\s*const f = 2 \+/, 'wave count')[0][1]), 'bake.sea.wave_count');
near(bake.sea.wind, Number(matches(envSrc, /const wind = ([\d.]+);/, 'wave wind')[0][1]), 'bake.sea.wind');
near(bake.sea.freq_rnd_pow, Number(matches(envSrc, /Math\.pow\(rnd\(\), ([\d.]+)\) \* (\d+)/, 'wave freq')[0][1]), 'bake.sea.freq_rnd_pow');
eq(bake.sea.freq_max, Number(matches(envSrc, /Math\.pow\(rnd\(\), [\d.]+\) \* (\d+)/, 'wave freq')[0][1]), 'bake.sea.freq_max');
near(bake.sea.amp_pow, Number(matches(envSrc, /1 \/ Math\.pow\(kl, ([\d.]+)\)/, 'wave amp')[0][1]), 'bake.sea.amp_pow');
// Swell: three components pinned by their exact GLSL literals, INCLUDING the
// per-component amplitude (h += A * sin(ph)); independent 5-field regex.
const swellLines = matches(envSrc, /k\s*=\s*vec2\(([-\d.]+),\s*([-\d.]+)\);\s*ph\s*=\s*dot\(p,\s*k\)\s*\+\s*t\s*\*\s*([\d.]+)(?:\s*\+\s*([\d.]+))?;\s*h\s*\+=\s*([\d.]+)\s*\*\s*sin\(ph\)/g, 'swell components');
eq(swellLines.length, 3, 'swell component count');
for (let i = 0; i < 3; i++) {
  const [, kx, ky, speed, phase, ampI] = swellLines[i];
  const c = bake.swell.components[i];
  near(c.k[0], Number(kx), `swell[${i}].k.x`);
  near(c.k[1], Number(ky), `swell[${i}].k.y`);
  near(c.speed, Number(speed), `swell[${i}].speed`);
  near(c.phase, phase ? Number(phase) : 0, `swell[${i}].phase`);
  near(c.amp, Number(ampI), `swell[${i}].amp`);
}
const amp = matches(envSrc, /mix\(([\d.]+),\s*([\d.]+),\s*smoothstep\(([\d.]+),\s*([\d.]+),\s*dDeck\)\)/, 'swellAmp')[0];
near(bake.swell.amp_min, Number(amp[1]), 'swell.amp_min');
near(bake.swell.amp_max, Number(amp[2]), 'swell.amp_max');
near(bake.swell.deck_min, Number(amp[3]), 'swell.deck_min');
near(bake.swell.deck_max, Number(amp[4]), 'swell.deck_max');
// Wave SHA256 golden: independent reimplementation of the upstream
// makeWaveTexture wave constructor + canonical LE binary serialization.
function mulberry32(a) {
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
{
  const s = bake.sea;
  const rnd = mulberry32(s.wave_seed);
  const buf = Buffer.alloc(s.wave_count * 24);
  let o = 0;
  for (let i = 0; i < s.wave_count; i++) {
    const f = 2 + Math.pow(rnd(), s.freq_rnd_pow) * s.freq_max;
    const ang = s.wind + (rnd() - 0.5) * 2.4;
    let kx = Math.round(Math.cos(ang) * f), ky = Math.round(Math.sin(ang) * f);
    if (kx === 0 && ky === 0) kx = 2;
    const kl = Math.hypot(kx, ky);
    const amp = 1 / Math.pow(kl, s.amp_pow);
    const phase = rnd() * Math.PI * 2;
    buf.writeInt32LE(kx, o); o += 4;
    buf.writeInt32LE(ky, o); o += 4;
    buf.writeBigInt64LE(BigInt(Math.round(amp * 1e9)), o); o += 8;
    buf.writeBigInt64LE(BigInt(Math.round(phase * 1e9)), o); o += 8;
  }
  eq(bake.wave_sha256, createHash('sha256').update(buf).digest('hex'), 'bake.wave_sha256');
}

console.log(`environment-crosscheck: ${checks} checks OK (4 quality rows, 3 themes, 8 map themes, grade/bloom defaults, Task 66 bake constants)`);
