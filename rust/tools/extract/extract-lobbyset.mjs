// Task M4-T56 batch B: extract the 3D lobby set / showcase layout data from
// the READ-ONLY JS tree (JS tree stays byte-clean).
//
// Same discipline as extract-environment.mjs: slice the literal blocks out of
// the sources and evaluate them standalone with a tiny THREE.Vector3 stub.
// assertContains guards pin the surrounding source lines so upstream drift
// fails the pipeline instead of silently changing the artifact.
// Output is deterministic (stable-json): rust/assets/lobbyset.json
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/lobbyset.json');
const commit = upstreamCommit(repoRoot, [
  'src/game/lobbySet.js',
  'src/game/lobbySet-mats.js',
  'src/game/showcase.js',
]);

function sliceBlock(src, file, name, start, end) {
  const i = src.indexOf(start);
  if (i < 0) throw new Error(`${file}: sliceBlock(${name}): start marker not found: ${start}`);
  const j = src.indexOf(end, i);
  if (j < 0) throw new Error(`${file}: sliceBlock(${name}): end marker not found: ${end}`);
  return src.slice(i, j + end.length);
}
function sliceBetween(src, file, name, start, endExclusive) {
  const i = src.indexOf(start);
  const j = src.indexOf(endExclusive, i);
  if (i < 0 || j < 0) throw new Error(`${file}: sliceBetween(${name}) markers not found`);
  return src.slice(i, j);
}
function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`expected fragment not found in ${where}: ${needle}`);
}

const setSrc = readFileSync(resolve(repoRoot, 'src/game/lobbySet.js'), 'utf8');
const matsSrc = readFileSync(resolve(repoRoot, 'src/game/lobbySet-mats.js'), 'utf8');
const showSrc = readFileSync(resolve(repoRoot, 'src/game/showcase.js'), 'utf8');

// ---- ALLEY (lobbySet.js L38-43) -----------------------------------------
const srcAlley = sliceBlock(setSrc, 'lobbySet.js', 'ALLEY', 'export const ALLEY = {', '\n};');
for (const n of ['xL: -4', 'zMouth: -34', 'dock: {', 'street: {']) assertContains(srcAlley, n, 'lobbySet.js ALLEY');
const ALLEY = new Function(`${srcAlley.replace(/^export\s+/, '')}\nreturn ALLEY;`)();

// ---- CAM / FRONT / BACK / faceCam (lobbySet.js L44-51) -------------------
const srcLayout = sliceBetween(setSrc, 'lobbySet.js', 'layout', 'const CAM = {', 'const QUALITY = {');
for (const n of ['const bear =', 'const FRONT =', 'const BACK =', 'const faceCam =']) assertContains(srcLayout, n, 'lobbySet.js layout');
class Vector3 {
  constructor(x, y, z) { this.x = x; this.y = y; this.z = z; this.a = [x, y, z]; }
  clone() { return new Vector3(...this.a); }
  toArray() { return this.a.slice(); }
}
const THREE = { Vector3 };
const layoutPrelude = 'const V = (x, y, z) => new THREE.Vector3(x, y, z);\n';
const L = new Function('THREE', `${layoutPrelude}${srcLayout}\nreturn { CAM, FRONT, BACK, faceCam, bear };`)(THREE);
const v3 = (v) => (Array.isArray(v) ? v : v.toArray());

// ---- hubSpot / camera / hubCamera (lobbySet.js L68-70) -------------------
// These live inside the LobbySet constructor, so evaluate that slice against a
// `this` stub with the real CAM object rather than hand-copying the literals.
const srcHub = sliceBetween(setSrc, 'lobbySet.js', 'hub', 'this.hubSpot = {', 'this._lanes =');
for (const n of ['this.hubSpot = {', 'this.camera = {', 'this.hubCamera = {']) assertContains(srcHub, n, 'lobbySet.js hub lines');
const hubFn = new Function('THREE', 'CAM', `${layoutPrelude}${srcHub}\nreturn { hubSpot: this.hubSpot, camera: this.camera, hubCamera: this.hubCamera };`);
const H = hubFn.call({}, THREE, L.CAM);
for (const row of [H.hubSpot, H.hubCamera]) {
  if (!v3(row.pos).every(Number.isFinite)) throw new Error('non-finite hub position');
}

// ---- QUALITY (lobbySet.js L53-57) ---------------------------------------
const srcQuality = sliceBlock(setSrc, 'lobbySet.js', 'QUALITY', 'const QUALITY = {', '\n};');
for (const n of ['refl: 0.5', 'shadow: 1536', 'steam: false', 'extraLights: true']) assertContains(srcQuality, n, 'lobbySet.js QUALITY');
const QUALITY = new Function(`${srcQuality}\nreturn QUALITY;`)();

// Evaluate the real bearing mapping (JS class L67) so Rust never
// re-implements the camera-relative mark placement.
const dockH = ALLEY.dock.h;
const spots = [
  ...L.FRONT.map(([x, z]) => ({ pos: [x, 0, z], yaw: L.faceCam(x, z), team: 0 })),
  ...L.BACK.map(([x, z]) => ({ pos: [x, dockH, z], yaw: L.faceCam(x, z, 0.6), team: 1 })),
];
if (spots.length !== 8) throw new Error(`expected 8 lobby spots, got ${spots.length}`);
for (const s of spots) {
  if (!s.pos.every(Number.isFinite) || !Number.isFinite(s.yaw)) throw new Error('non-finite spot');
}

// ---- showcase constants (showcase.js L49-77) ----------------------------
const srcShow = sliceBetween(showSrc, 'showcase.js', 'constants', 'const PED = {', '// lobby emotes');
for (const n of ['const PEDESTAL = new Set(', 'const SET_MODES = new Set(', 'PLATE_H = 1.64', 'const SWIM = {']) assertContains(srcShow, n, 'showcase.js constants');
const S = new Function(`${srcShow}\nreturn { PED, DRUM, SLOTS, CHAR_H, LOAD_FOCUS_Y, PEDESTAL, OVERLAY, SET_MODES, SET_FADE, GLIDE, HUB_ID, PLATE_H, SWIM };`)();
const setToArray = (s) => [...s].sort();

const srcEmotes = sliceBlock(showSrc, 'showcase.js', 'LOB_EMOTES', 'const LOB_EMOTES = {', '\n};');
const LOB_EMOTES = new Function(`${srcEmotes}\nreturn LOB_EMOTES;`)();

// ---- Task 67: lobby material parameters (lobbySet-mats.js / lobbySet.js) --
// Every number below is parsed from the actual JS source via regex. No
// hand-copied constants; assertContains needles pin the surrounding literals.
function lineOf(src, needle, where) {
  const hit = src.split('\n').find((l) => l.includes(needle));
  if (!hit) throw new Error(`${where}: no line with ${needle}`);
  return hit;
}
function nums(s) {
  return (s.match(/-?\d+(?:\.\d+)?/g) || []).map(Number);
}
function grabNums(src, re, where) {
  const m = src.match(re);
  if (!m) throw new Error(`${where}: regex failed for ${re}`);
  return nums(m[1]);
}
function one(src, re, where) {
  const a = grabNums(src, re, where);
  if (a.length !== 1) throw new Error(`${where}: expected 1 number, got ${a.length}`);
  return a[0];
}

// Wet ground planar reflection — lobbySet-mats.js L175 / L290-292.
const groundLine = lineOf(matsSrc, 'envMapIntensity: 1 })', 'ground mat');
const fresnelLine = lineOf(matsSrc, 'float F = 0.02 + 0.98 * pow', 'ground Fresnel');
const kLine = lineOf(matsSrc, 'float k = clamp(gWater', 'ground k');
const mixLine = lineOf(matsSrc, 'mix(reflectedLight.indirectSpecular', 'ground mix');
const reflection = {
  env_map_intensity: one(groundLine, /envMapIntensity:\s*([\d.]+)/, 'ground envIntensity'),
  fresnel_f0: one(fresnelLine, /float F =\s*([\d.]+)/, 'ground Fresnel F0'),
  fresnel_scale: one(fresnelLine, /\+\s*([\d.]+)\s*\*\s*pow/, 'ground Fresnel scale'),
  fresnel_pow: one(fresnelLine, /pow\(1\.0 - NdV,\s*([\d.]+)\)/, 'ground Fresnel pow'),
  k_damp: one(kLine, /gDamp\s*\*\s*([\d.]+)/, 'ground k damp'),
  k_ink: one(kLine, /gInk\s*\*\s*([\d.]+)/, 'ground k ink'),
  water_mix: grabNums(mixLine, /F \* mix\(([^)]*)\)/, 'ground water mix'),
};
assertContains(matsSrc, 'float F = 0.02 + 0.98 * pow(1.0 - NdV, 5.0);', 'ground Fresnel literal');
assertContains(matsSrc, 'float k = clamp(gWater + gDamp * 0.5 + gInk * 0.5, 0.0, 1.0);', 'ground k literal');
if (reflection.water_mix.length !== 2) throw new Error('ground water mix must be 2 numbers');

// Neon — lobbySet-mats.js L17 (team colors), L321 (core color + intensity).
const neonULine = lineOf(matsSrc, 'uTeamA:', 'neon team uniforms');
const teamHexA = neonULine.match(/uTeamA:[^']*'([^']+)'/)[1];
const teamHexB = neonULine.match(/uTeamB:[^']*'([^']+)'/)[1];
const neonLine = lineOf(matsSrc, 'uColC: { value: new THREE.Color', 'neon uniforms');
const neon = {
  core_color: grabNums(neonLine, /uColC: \{ value: new THREE\.Color\(([^)]*)\)/, 'neon core color'),
  intensity: grabNums(neonLine, /uI: \{ value: new THREE\.Vector4\(([^)]*)\)/, 'neon intensity vector'),
  team_a: teamHexA,
  team_b: teamHexB,
};
if (neon.core_color.length !== 3 || neon.intensity.length !== 4) throw new Error('neon vec size');

// Steam — lobbySet-mats.js L424/L433/L437 + lobbySet.js L589-591.
const steamU = lineOf(matsSrc, 'uCol: { value: new THREE.Color', 'steam uniforms');
const steamP = lineOf(matsSrc, 'vec2 p = vUv * vec2', 'steam noise');
const steamA = lineOf(matsSrc, 'float a = smoothstep(0.4, 0.85', 'steam alpha');
const steamCards = lineOf(setSrc, 'for (let i = 0; i < 3', 'steam cards');
const steamGeo = lineOf(setSrc, 'seed.push(i *', 'steam geo');
const steam = {
  cards: one(steamCards, /for \(let i = 0; i <\s*(\d+)/, 'steam card count'),
  card_height: one(steamGeo, /,\s*v \*\s*([\d.]+),/, 'steam card height'),
  x: one(steamGeo, /pos\.push\(([-\d.]+)/, 'steam x'),
  z: one(steamGeo, /v \*\s*[\d.]+,\s*([-\d.]+)\s*\+/, 'steam z'),
  seed_step: one(steamGeo, /seed\.push\(i \*\s*([\d.]+)/, 'steam seed step'),
  color: grabNums(steamU, /uCol: \{ value: new THREE\.Color\(([^)]*)\)/, 'steam color'),
  time_scale: one(lineOf(matsSrc, 'float t = uTime * 0.35', 'steam time'), /uTime \*\s*([\d.]+)/, 'steam time scale'),
  noise_scale: grabNums(steamP, /vec2\(([^)]*)\)/, 'steam noise scale'),
  alpha_edge: grabNums(steamA, /smoothstep\(([^)]*)\)/, 'steam alpha smoothstep'),
  alpha_scale: one(steamA, /\*\s*([\d.]+)\s*;/, 'steam alpha scale'),
};
if (steam.color.length !== 3 || steam.noise_scale.length !== 2 || steam.alpha_edge.length !== 2) {
  throw new Error('steam vec size');
}
assertContains(matsSrc, 'float t = uTime * 0.35 + vSeed * 7.0;', 'steam time literal');
assertContains(matsSrc, 'float a = smoothstep(0.4, 0.85, n) * edge * 0.6;', 'steam alpha literal');

// Lobby PMREM — lobbySet.js L156/L160/L73 + env card k literals L143-147.
const envRTLine = lineOf(setSrc, 'this._pmrem.fromScene(', 'lobby PMREM');
const envNormLine = lineOf(setSrc, 'multiplyScalar(4 / Math.max', 'lobby PMREM normalize');
const envIntLine = lineOf(setSrc, 'this.environmentIntensity =', 'lobby envIntensity');
const envSceneArgs = grabNums(envRTLine, /fromScene\(this\._envScene,\s*([\d.]+,\s*[\d.]+,\s*[\d.]+,\s*\{ size:\s*[\d.]+,\s*position: V\([^)]*\))\s*\}\)/, 'lobby PMREM fromScene args');
// fromScene args: [sigma=0.04, near=0.1, far=100, size=128, posx, posy, posz]
const pmrem = {
  sigma: envSceneArgs[0],
  near: envSceneArgs[1],
  far: envSceneArgs[2],
  size: envSceneArgs[3],
  position: envSceneArgs.slice(4, 7),
  team_normalize: one(envNormLine, /multiplyScalar\(\s*([\d.]+)\s*\//, 'lobby PMREM normalize'),
  environment_intensity: one(envIntLine, /environmentIntensity =\s*([\d.]+)/, 'lobby envIntensity'),
  env_card_k: {
    door: one(lineOf(setSrc, "'#ffb366', 5)", 'env door k'), /'(?:[^']*)',\s*([\d.]+)\)/, 'env door k'),
    neon: one(lineOf(setSrc, "'#ffffff', 4)", 'env neon k'), /'(?:[^']*)',\s*([\d.]+)\)/, 'env neon k'),
    bulbs: one(lineOf(setSrc, "'#ffcf8a', 3)", 'env bulbs k'), /'(?:[^']*)',\s*([\d.]+)\)/, 'env bulbs k'),
    vending: one(lineOf(setSrc, "'#cfefff', 2.5)", 'env vending k'), /'(?:[^']*)',\s*([\d.]+)\)/, 'env vending k'),
  },
};
assertContains(setSrc, 'fromScene(this._envScene, 0.04, 0.1, 100, { size: 128, position: V(0.8, 1.4, -1.2) })', 'lobby PMREM literal');
if (envSceneArgs.length !== 7) throw new Error(`lobby PMREM fromScene: expected 7 args, got ${envSceneArgs.length}`);

const materials = { reflection, neon, steam, pmrem };

const doc = {
  schema: 1,
  source: { commit, files: ['src/game/lobbySet.js', 'src/game/lobbySet-mats.js', 'src/game/showcase.js'] },
  alley: ALLEY,
  cam: { pos: v3(L.CAM.pos), target: v3(L.CAM.target), fov: L.CAM.fov, near: L.CAM.near, far: L.CAM.far },
  quality: QUALITY,
  spots,
  hubSpot: { pos: v3(H.hubSpot.pos), yaw: H.hubSpot.yaw },
  hubCamera: { pos: v3(H.hubCamera.pos), target: v3(H.hubCamera.target), fov: H.hubCamera.fov, near: H.hubCamera.near, far: H.hubCamera.far },
  pedestal: {
    PED: S.PED, DRUM: S.DRUM, SLOTS: S.SLOTS,
    CHAR_H: S.CHAR_H, LOAD_FOCUS_Y: S.LOAD_FOCUS_Y, PLATE_H: S.PLATE_H,
    SET_FADE: S.SET_FADE, GLIDE: S.GLIDE, HUB_ID: S.HUB_ID, SWIM: S.SWIM,
  },
  modes: {
    pedestal: setToArray(S.PEDESTAL),
    overlay: setToArray(S.OVERLAY),
    set: setToArray(S.SET_MODES),
  },
  emotes: LOB_EMOTES,
  materials,
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('lobbyset.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(`wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, spots=${spots.length}, emotes=${Object.keys(LOB_EMOTES).length}`);
