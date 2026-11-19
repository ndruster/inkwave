// Task M5-T65: independent crosscheck of assets/postprocess.json.
// Does NOT import extract-postprocess.mjs. Every constant is re-parsed from
// the READ-ONLY JS/CSS tree via a second, line-oriented path.
import { upstreamCommit } from './extract/upstream-commit.mjs';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));                  // rust/tools
const repoRoot = resolve(here, '../..');
const doc = JSON.parse(readFileSync(resolve(here, '../assets/postprocess.json'), 'utf8'));
const read = (p) => readFileSync(resolve(repoRoot, p), 'utf8').split('\n');
const renderer = read('src/core/renderer.js');
const config = read('src/config.js');
const css = read('styles/hud.css');

let checks = 0;
function eq(got, want, label) {
  checks += 1;
  if (got !== want) throw new Error(`postprocess crosscheck: ${label}: got ${got}, JS=${want}`);
}
function near(got, want, label) {
  checks += 1;
  if (Math.abs(got - want) > 1e-9) throw new Error(`postprocess crosscheck: ${label}: got ${got}, JS=${want}`);
}
function find(lines, needle, label) {
  const hit = lines.find((l) => l.includes(needle));
  if (!hit) throw new Error(`postprocess crosscheck: ${label}: no line for ${needle}`);
  return hit;
}
function value(line, key) {
  const i = line.indexOf(key);
  if (i < 0) throw new Error(`postprocess crosscheck: no ${key} in ${line}`);
  const m = line.slice(i + key.length).match(/-?\d+(?:\.\d+)?/);
  if (!m) throw new Error(`postprocess crosscheck: no number after ${key}`);
  return Number(m[0]);
}
function vec3(line) {
  const m = line.match(/new THREE\.(?:Vector3|Color)\(([^)]*)\)/);
  if (!m) throw new Error(`postprocess crosscheck: not a vec3: ${line}`);
  return m[1].split(',').map((x) => Number(x.trim()));
}

// Source pin.
eq(doc.source.commit, upstreamCommit(repoRoot, [
  'src/core/renderer.js', 'src/config.js', 'styles/hud.css',
]), 'source.commit');

// Pass order: independently re-parse the addPass call sequence from JS.
const argNames = {
  'this.renderPass': 'render_hdr', ao: 'gtao', 'this.bloom': 'bloom',
  'this.grade': 'grade_vignette', 'this.extraPass': 'screenfx',
  'new OutputPass()': 'output', 'new SMAAPass()': 'smaa',
};
const jsOrder = [...renderer.join('\n').matchAll(/comp\.addPass\((new \w+\(\)|\w+(?:\.\w+)?)\)/g)]
  .map((m) => argNames[m[1].trim()]);
if (jsOrder.some((n) => !n)) throw new Error('postprocess crosscheck: unmapped addPass call');
eq(doc.pass_order.join(','), jsOrder.join(','), 'pass order');
eq(jsOrder.join(','), 'render_hdr,gtao,bloom,grade_vignette,screenfx,output,smaa', 'JS pass order expected');

// Grade 12 uniforms.
const g = doc.grade_defaults;
const uSat = find(renderer, 'uSat: { value:', 'uSat');
near(g.sat, value(uSat, 'uSat: { value: '), 'sat');
near(g.vib, value(find(renderer, 'uVib: { value:', 'uVib'), 'uVib: { value: '), 'vib');
near(g.contrast, value(find(renderer, 'uContrast: { value:', 'uContrast'), 'uContrast: { value: '), 'contrast');
vec3(find(renderer, 'uShadowTint:', 'uShadowTint')).forEach((v, i) => near(g.shadow_tint[i], v, `shadow_tint[${i}]`));
vec3(find(renderer, 'uHighTint:', 'uHighTint')).forEach((v, i) => near(g.high_tint[i], v, `high_tint[${i}]`));
near(g.lift, value(find(renderer, 'uLift: { value:', 'uLift'), 'uLift: { value: '), 'lift');
near(g.exposure, value(find(renderer, 'uExposure: { value:', 'uExposure'), 'uExposure: { value: '), 'exposure');
near(g.vignette, value(find(renderer, 'uVignette: { value:', 'uVignette'), 'uVignette: { value: '), 'vignette');
near(g.hurt, value(find(renderer, 'uHurt: { value:', 'uHurt'), 'uHurt: { value: '), 'hurt');
vec3(find(renderer, 'uHurtColor:', 'uHurtColor')).forEach((v, i) => near(g.hurt_color[i], v, `hurt_color[${i}]`));
near(g.flash, value(find(renderer, 'uFlash: { value:', 'uFlash'), 'uFlash: { value: '), 'flash');
near(g.aspect, value(find(renderer, 'uAspect: { value:', 'uAspect'), 'uAspect: { value: '), 'aspect');

// GTAO + denoise.
const ao = doc.gtao;
near(ao.blend_intensity, value(find(renderer, '.blendIntensity =', 'blend'), '.blendIntensity = '), 'blend intensity');
const gaoLine = find(renderer, 'updateGtaoMaterial(', 'gtao uniforms');
for (const [key, name] of [
  ['radius', 'radius'], ['distance_exponent', 'distanceExponent'], ['thickness', 'thickness'],
  ['scale', 'scale'], ['samples', 'samples'], ['distance_falloff', 'distanceFallOff'],
]) near(ao[key], value(gaoLine, `${name}: `), `gtao ${name}`);
eq(ao.half_res_apple_gpu, renderer.some((l) => l.includes('GTAO runs at half res')), 'gtao half-res');
const pdLine = find(renderer, 'updatePdMaterial(', 'denoise uniforms');
for (const [key, name] of [
  ['luma_phi', 'lumaPhi'], ['depth_phi', 'depthPhi'], ['normal_phi', 'normalPhi'],
  ['radius', 'radius'], ['rings', 'rings'], ['samples', 'samples'],
]) near(ao.denoise[key], value(pdLine, `${name}: `), `denoise ${name}`);

// SMAA gate.
eq(doc.smaa.pass, 'SMAAPass', 'smaa pass');
eq(renderer.some((l) => l.includes('q.msaa && !this.samples') && l.includes('SMAAPass')), true, 'smaa gate in JS');

// tilt-shift CSS (D-T65-1: DOM/CSS, not GPU).
const tilt = doc.tilt_shift;
const tiltLine = find(css, '.iw-dio__tilt {', 'tilt');
near(tilt.blur_px, value(tiltLine, 'blur('), 'tilt blur');
near(tilt.saturate, value(tiltLine, 'saturate('), 'tilt saturate');
near(tilt.top_height_pct, value(find(css, '.iw-dio__tilt--top', 'tilt top'), 'height: '), 'tilt top');
near(tilt.bottom_height_pct, value(find(css, '.iw-dio__tilt--bot', 'tilt bot'), 'height: '), 'tilt bottom');

// QUALITY 四档开关.
for (const tier of ['low', 'medium', 'high', 'ultra']) {
  const qline = config.find((l) => new RegExp(`^\\s*${tier}:\\s*\\{`).test(l));
  if (!qline) throw new Error(`postprocess crosscheck: quality ${tier} line missing`);
  eq(doc.quality[tier].msaa, value(qline, 'msaa: '), `${tier} msaa`);
  eq(doc.quality[tier].bloom, /bloom:\s*true/.test(qline), `${tier} bloom`);
  eq(doc.quality[tier].ao, /ao:\s*true/.test(qline), `${tier} ao`);
}
eq(doc.quality.low.msaa + Number(doc.quality.low.bloom) + Number(doc.quality.low.ao), 0, 'low all-off');

console.log(`postprocess crosscheck: OK (${checks} assertions, commit ${doc.source.commit.slice(0, 8)})`);
