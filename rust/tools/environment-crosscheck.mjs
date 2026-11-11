// Task 55 TR-55.1 / D-T55-A independent crosscheck.
//
// Deliberately does NOT import extract-environment.mjs: it re-parses the
// READ-ONLY JS sources with plain regexes (a different code path) and
// reconciles them against rust/assets/environment.json. Any upstream drift
// or extractor regression fails the gate's XCHK stage.
import { readFileSync } from 'node:fs';
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

console.log(`environment-crosscheck: ${checks} checks OK (4 quality rows, 3 themes, 8 map themes, grade/bloom defaults)`);
