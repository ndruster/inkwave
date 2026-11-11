// Task M4-T56 batch B: extract the 3D lobby set / showcase layout data from
// the READ-ONLY JS tree (JS tree stays byte-clean).
//
// Same discipline as extract-environment.mjs: slice the literal blocks out of
// the sources and evaluate them standalone with a tiny THREE.Vector3 stub.
// assertContains guards pin the surrounding source lines so upstream drift
// fails the pipeline instead of silently changing the artifact.
// Output is deterministic (stable-json): rust/assets/lobbyset.json
import { execSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/lobbyset.json');
let commit;
try {
  // Pin the *upstream JS data* revision (see extract-environment.mjs): the
  // gate's DETERM re-run must be byte-identical across Rust-side commits.
  commit = execSync(
    'git log -1 --format=%H -- src/game/lobbySet.js src/game/showcase.js',
    { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] },
  ).toString().trim();
} catch { commit = 'unknown'; }

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

const doc = {
  schema: 1,
  source: { commit, files: ['src/game/lobbySet.js', 'src/game/showcase.js'] },
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
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('lobbyset.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(`wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, spots=${spots.length}, emotes=${Object.keys(LOB_EMOTES).length}`);
