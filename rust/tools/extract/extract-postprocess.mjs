// Task M5-T65: extract the post-processing chain parameters from the READ-ONLY
// JS/CSS tree. Source files stay byte-clean. Output is deterministic
// (stable-json): rust/assets/postprocess.json.
//
// Values are located by source-fragment assertions then parsed with narrow
// regexes. `tools/postprocess-crosscheck.mjs` re-parses every constant via an
// independent line-oriented path.
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));            // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/postprocess.json');
const SRC_FILES = [
  'src/core/renderer.js',
  'src/config.js',
  'styles/hud.css',
];
const commit = upstreamCommit(repoRoot, SRC_FILES);

const read = (p) => readFileSync(resolve(repoRoot, p), 'utf8');
const renderer = read('src/core/renderer.js');
const config = read('src/config.js');
const css = read('styles/hud.css');

function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`postprocess extract: ${where}: missing ${needle}`);
}
function line(src, needle, where) {
  const hit = src.split('\n').find((l) => l.includes(needle));
  if (!hit) throw new Error(`postprocess extract: ${where}: no line with ${needle}`);
  return hit;
}
function numAfter(s, needle) {
  const i = s.indexOf(needle);
  if (i < 0) throw new Error(`postprocess extract: no ${needle} in ${s}`);
  const m = s.slice(i + needle.length).match(/-?\d+(?:\.\d+)?/);
  if (!m) throw new Error(`postprocess extract: no number after ${needle} in ${s}`);
  return Number(m[0]);
}
function triple(s) {
  const m = s.match(/new THREE\.(?:Vector3|Color)\(([^)]*)\)/);
  const src = m ? m[1] : s;
  return src.split(',').map((x) => Number(x.trim()));
}

// ---- pass order: parse the ACTUAL `comp.addPass(...)` call sequence -------
assertContains(renderer, 'MSAA HDR target → optional GTAO → bloom → grade/vignette → output', 'pass order header');
const addPassArgToName = {
  'this.renderPass': 'render_hdr',
  ao: 'gtao',
  'this.bloom': 'bloom',
  'this.grade': 'grade_vignette',
  'this.extraPass': 'screenfx',
  'new OutputPass()': 'output',
  'new SMAAPass()': 'smaa',
};
const addPassCalls = [...renderer.matchAll(/comp\.addPass\((new \w+\(\)|\w+(?:\.\w+)?)\)/g)].map((m) => m[1].trim());
const passOrder = addPassCalls.map((arg) => {
  const name = addPassArgToName[arg];
  if (!name) throw new Error(`postprocess extract: unmapped addPass arg ${arg}`);
  return name;
});
const expectedOrder = 'render_hdr,gtao,bloom,grade_vignette,screenfx,output,smaa';
if (passOrder.join(',') !== expectedOrder) {
  throw new Error(`postprocess extract: pass order drifted: ${passOrder.join(',')}`);
}

// ---- grade uniform defaults (renderer.js L17-32) --------------------------
const gradeShaderAt = renderer.indexOf('const GradeShader = {');
const gradeBlock = renderer.slice(
  gradeShaderAt,
  renderer.indexOf('vertexShader:', gradeShaderAt),
);
for (const n of [
  'uSat: { value: 1.08 }', 'uVib: { value: 0.12 }', 'uContrast: { value: 1.07 }',
  'uShadowTint: { value: new THREE.Vector3(0.975, 0.99, 1.035) }',
  'uHighTint: { value: new THREE.Vector3(1.025, 1.0, 0.972) }',
  'uLift: { value: 0.0 }', 'uExposure: { value: 1.0 }',
  'uVignette: { value: 0.22 }', 'uHurt: { value: 0 }',
  'uHurtColor: { value: new THREE.Color(1, 0.2, 0.3) }',
  'uFlash: { value: 0 }', 'uAspect: { value: 1.7 }',
]) assertContains(gradeBlock, n, 'renderer.js grading');

const gradeDefaults = {
  sat: numAfter(gradeBlock, 'uSat: { value: '),
  vib: numAfter(gradeBlock, 'uVib: { value: '),
  contrast: numAfter(gradeBlock, 'uContrast: { value: '),
  shadow_tint: triple(line(gradeBlock, 'uShadowTint:', 'uShadowTint')),
  high_tint: triple(line(gradeBlock, 'uHighTint:', 'uHighTint')),
  lift: numAfter(gradeBlock, 'uLift: { value: '),
  exposure: numAfter(gradeBlock, 'uExposure: { value: '),
  vignette: numAfter(gradeBlock, 'uVignette: { value: '),
  hurt: numAfter(gradeBlock, 'uHurt: { value: '),
  hurt_color: triple(line(gradeBlock, 'uHurtColor:', 'uHurtColor')),
  flash: numAfter(gradeBlock, 'uFlash: { value: '),
  aspect: numAfter(gradeBlock, 'uAspect: { value: '),
};

// ---- GTAO constants (renderer.js L128-136) --------------------------------
const blendIntensity = numAfter(line(renderer, '.blendIntensity =', 'gtao blend'), '.blendIntensity = ');
const gtaoBlock = line(renderer, 'updateGtaoMaterial(', 'gtao uniforms');
const pdBlock = line(renderer, 'updatePdMaterial(', 'denoise uniforms');
assertContains(gtaoBlock, 'radius', 'gtao radius');
assertContains(pdBlock, 'lumaPhi', 'denoise lumaPhi');
function uniformNum(block, key) {
  const hit = line(block, `${key}:`, `uniform ${key}`);
  return numAfter(hit, `${key}: `);
}
const gtao = {
  blend_intensity: blendIntensity,
  radius: uniformNum(gtaoBlock, 'radius'),
  distance_exponent: uniformNum(gtaoBlock, 'distanceExponent'),
  thickness: uniformNum(gtaoBlock, 'thickness'),
  scale: uniformNum(gtaoBlock, 'scale'),
  samples: uniformNum(gtaoBlock, 'samples'),
  distance_falloff: uniformNum(gtaoBlock, 'distanceFallOff'),
  half_res_apple_gpu: true, // renderer.js L3 comment; pinned by assertion below
};
assertContains(renderer, 'GTAO runs at half res', 'gtao half-res');
const denoise = {
  luma_phi: uniformNum(pdBlock, 'lumaPhi'),
  depth_phi: uniformNum(pdBlock, 'depthPhi'),
  normal_phi: uniformNum(pdBlock, 'normalPhi'),
  radius: uniformNum(pdBlock, 'radius'),
  rings: uniformNum(pdBlock, 'rings'),
  samples: uniformNum(pdBlock, 'samples'),
};

// ---- SMAA (renderer.js L148-149) ------------------------------------------
assertContains(renderer, 'new SMAAPass()', 'smaa pass');
assertContains(renderer, 'q.msaa && !this.samples', 'smaa gate');

// ---- tilt-shift CSS (hud.css L776-781; NOT a GPU pass) ---------------------
const tiltLine = line(css, '.iw-dio__tilt {', 'tilt shift');
const blurPx = numAfter(tiltLine, 'blur(');
const sat = numAfter(tiltLine, 'saturate(');
const topLine = line(css, '.iw-dio__tilt--top', 'tilt top');
const topHeight = numAfter(topLine, 'height: ');
const botLine = line(css, '.iw-dio__tilt--bot', 'tilt bottom');
const botHeight = numAfter(botLine, 'height: ');
const tiltShift = {
  blur_px: blurPx,
  saturate: sat,
  top_height_pct: topHeight,
  bottom_height_pct: botHeight,
};

// ---- QUALITY 四档开关组合 (config.js L585-591) -----------------------------
const quality = {};
for (const tier of ['low', 'medium', 'high', 'ultra']) {
  const tierLine = config.split('\n').find((l) => new RegExp(`^\\s*${tier}:\\s*\\{`).test(l));
  if (!tierLine) throw new Error(`postprocess extract: quality ${tier}: no line`);
  quality[tier] = {
    msaa: numAfter(tierLine, 'msaa: '),
    bloom: /bloom:\s*true/.test(tierLine),
    ao: /ao:\s*true/.test(tierLine),
  };
}
if (quality.low.msaa !== 0 || quality.low.bloom || quality.low.ao) {
  throw new Error('postprocess extract: low tier must have all post disabled');
}

const doc = {
  schema: 1,
  source: { commit, files: SRC_FILES },
  pass_order: passOrder,
  grade_defaults: gradeDefaults,
  gtao: { ...gtao, denoise },
  smaa: { gate: 'msaa>0 && !multisampled', pass: 'SMAAPass' },
  tilt_shift: tiltShift,
  quality,
};

writeFileSync(outPath, stableStringify(doc) + '\n');
console.log(`extract-postprocess: ${commit.slice(0, 8)} -> ${resolve(repoRoot, 'rust/assets/postprocess.json')}`);
