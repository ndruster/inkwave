// Task 56 TR-56.1 independent crosscheck.
//
// Deliberately does NOT import extract-lobbyset.mjs: it re-parses the
// READ-ONLY JS sources with plain regexes (a different code path) and
// recomputes the eight lineup marks from the bearing formula independently,
// then reconciles everything against rust/assets/lobbyset.json. Any upstream
// drift or extractor regression fails the gate's XCHK stage.
import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools
const repoRoot = resolve(here, '../..');                   // repository root
const doc = JSON.parse(readFileSync(resolve(here, '../assets/lobbyset.json'), 'utf8'));
const set = readFileSync(resolve(repoRoot, 'src/game/lobbySet.js'), 'utf8');
const mats = readFileSync(resolve(repoRoot, 'src/game/lobbySet-mats.js'), 'utf8');
const show = readFileSync(resolve(repoRoot, 'src/game/showcase.js'), 'utf8');

let checks = 0;
function eq(actual, expected, label) {
  if (actual !== expected) throw new Error(`${label}: got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  checks++;
}
function near(actual, expected, label) {
  if (!Number.isFinite(actual) || Math.abs(actual - expected) > 1e-9) {
    throw new Error(`${label}: got ${actual}, expected ${expected}`);
  }
  checks++;
}
function one(src, re, label) {
  const m = re.exec(src);
  if (!m) throw new Error(`${label}: no match — upstream drift?`);
  return m;
}
function num(src, re, label) { return Number(one(src, re, label)[1]); }
function vec(src, name, label) {
  // Object literal (`pos: V(...)`) or assignment (`const p = V(...)`).
  const m = one(src, new RegExp(`${name}\\s*[:=]\\s*V\\(([^)]*)\\)`), label);
  return m[1].split(',').map((s) => Number(s.trim()));
}

// ---- ALLEY: every scalar + the three sub-tables ---------------------------
const alley = one(set, /export const ALLEY = \{([\s\S]*?)\n\};/, 'ALLEY block')[1];
for (const key of ['xL', 'xR', 'zNear', 'zMouth', 'hL', 'hR']) {
  near(doc.alley[key], num(alley, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `ALLEY.${key}`), `alley.${key}`);
}
for (const [table, keys] of Object.entries({
  dock: ['x0', 'x1', 'z0', 'z1', 'h'],
  stair: ['x0', 'x1', 'z0', 'n'],
  street: ['z0', 'z1', 'curb'],
})) {
  const body = one(alley, new RegExp(`${table}:\\s*\\{([^}]*)\\}`), `ALLEY.${table}`)[1];
  for (const key of keys) {
    near(doc.alley[table][key], num(body, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `ALLEY.${table}.${key}`), `alley.${table}.${key}`);
  }
}

// ---- CAM: pos/target vectors + frustum ------------------------------------
const camLine = one(set, /const CAM = \{([^}]*)\}/, 'CAM')[1];
const camPos = vec(camLine, 'pos', 'CAM.pos');
const camTarget = vec(camLine, 'target', 'CAM.target');
for (let i = 0; i < 3; i++) {
  near(doc.cam.pos[i], camPos[i], `cam.pos[${i}]`);
  near(doc.cam.target[i], camTarget[i], `cam.target[${i}]`);
}
for (const key of ['fov', 'near', 'far']) {
  near(doc.cam[key], num(camLine, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `CAM.${key}`), `cam.${key}`);
}

// ---- QUALITY: three rows, every field -------------------------------------
const qBlock = one(set, /const QUALITY = \{([\s\S]*?)\n\};/, 'QUALITY')[1];
const qRows = [...qBlock.matchAll(/(low|medium|high):\s*\{\s*refl:\s*([\d.]+),\s*shadow:\s*(\d+),\s*steam:\s*(true|false),\s*extraLights:\s*(true|false)\s*\}/g)];
eq(qRows.length, 3, 'QUALITY row count');
for (const [, name, refl, shadow, steam, extraLights] of qRows) {
  const r = doc.quality[name];
  if (!r) throw new Error(`quality.${name} missing from lobbyset.json`);
  near(r.refl, Number(refl), `quality.${name}.refl`);
  eq(r.shadow, Number(shadow), `quality.${name}.shadow`);
  eq(r.steam, steam === 'true', `quality.${name}.steam`);
  eq(r.extraLights, extraLights === 'true', `quality.${name}.extraLights`);
}

// ---- 8 spots: independent bearing recomputation --------------------------
// JS: bear([deg,z]) = [CAM.x + (CAM.z - z) * tan(deg*pi/180), z]
//     faceCam(x,z,k=0.72) = atan2(CAM.x - x, CAM.z - z) * k
// Crosscheck reads the literal degree/depth pairs with regexes and computes
// the world positions itself (the extractor instead evals the JS slices).
const frontLine = one(set, /const FRONT = \[([^\n]*?)\]\.map\(bear\);/, 'FRONT')[1];
const backLine = one(set, /const BACK = \[([^\n]*?)\]\.map\(bear\);/, 'BACK')[1];
const pairs = (line) => [...line.matchAll(/\[\s*(-?[\d.]+)\s*,\s*(-?[\d.]+)\s*\]/g)]
  .map((m) => [Number(m[1]), Number(m[2])]);
const frontPairs = pairs(frontLine);
const backPairs = pairs(backLine);
eq(frontPairs.length, 4, 'FRONT mark count');
eq(backPairs.length, 4, 'BACK mark count');
// Read the dock height from the JS source too, not from the artifact.
const dockBody = one(alley, /dock:\s*\{([^}]*)\}/, 'ALLEY.dock')[1];
const dockH = num(dockBody, /\bh:\s*(-?[\d.]+)/, 'ALLEY.dock.h');
const want = [
  ...frontPairs.map(([deg, z]) => {
    const x = camPos[0] + (camPos[2] - z) * Math.tan((deg * Math.PI) / 180);
    return { pos: [x, 0, z], yaw: Math.atan2(camPos[0] - x, camPos[2] - z) * 0.72, team: 0 };
  }),
  ...backPairs.map(([deg, z]) => {
    const x = camPos[0] + (camPos[2] - z) * Math.tan((deg * Math.PI) / 180);
    return { pos: [x, dockH, z], yaw: Math.atan2(camPos[0] - x, camPos[2] - z) * 0.6, team: 1 };
  }),
];
eq(doc.spots.length, 8, 'spot count');
for (let i = 0; i < 8; i++) {
  eq(doc.spots[i].team, want[i].team, `spots[${i}].team`);
  for (let k = 0; k < 3; k++) near(doc.spots[i].pos[k], want[i].pos[k], `spots[${i}].pos[${k}]`);
  near(doc.spots[i].yaw, want[i].yaw, `spots[${i}].yaw`);
}

// ---- hubSpot / hubCamera ---------------------------------------------------
const hub = one(set, /this\.hubSpot = \{\s*pos:\s*V\(([^)]*)\),\s*yaw:\s*(-?[\d.]+)\s*\}/, 'hubSpot');
const hubPos = hub[1].split(',').map((s) => Number(s.trim()));
for (let i = 0; i < 3; i++) near(doc.hubSpot.pos[i], hubPos[i], `hubSpot.pos[${i}]`);
near(doc.hubSpot.yaw, Number(hub[2]), 'hubSpot.yaw');
const hubCam = one(set, /this\.hubCamera = \{([^}]*)\}/, 'hubCamera')[1];
const hubCamPos = vec(hubCam, 'pos', 'hubCamera.pos');
const hubCamTarget = vec(hubCam, 'target', 'hubCamera.target');
for (let i = 0; i < 3; i++) {
  near(doc.hubCamera.pos[i], hubCamPos[i], `hubCamera.pos[${i}]`);
  near(doc.hubCamera.target[i], hubCamTarget[i], `hubCamera.target[${i}]`);
}
for (const key of ['fov', 'near', 'far']) {
  near(doc.hubCamera[key], num(hubCam, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `hubCamera.${key}`), `hubCamera.${key}`);
}

// ---- showcase constants / mode sets / pedestal ----------------------------
const scalar = (name) => num(show, new RegExp(`const ${name} = ([\\d.]+);`), `showcase.${name}`);
for (const [jsKey, docKey] of [
  ['CHAR_H', 'CHAR_H'], ['LOAD_FOCUS_Y', 'LOAD_FOCUS_Y'], ['PLATE_H', 'PLATE_H'],
  ['SET_FADE', 'SET_FADE'], ['GLIDE', 'GLIDE'],
]) near(doc.pedestal[docKey], scalar(jsKey), `pedestal.${docKey}`);
eq(doc.pedestal.HUB_ID, one(show, /const HUB_ID = '([^']+)'/, 'HUB_ID')[1], 'pedestal.HUB_ID');
const setNames = (name) => one(show, new RegExp(`const ${name} = new Set\\(\\[([^\\]]*)\\]\\)`), name)[1]
  .match(/'([^']+)'/g).map((s) => s.slice(1, -1)).sort();
for (const [jsName, docName] of [['PEDESTAL', 'pedestal'], ['OVERLAY', 'overlay'], ['SET_MODES', 'set']]) {
  const wantSet = setNames(jsName);
  eq(doc.modes[docName].length, wantSet.length, `modes.${docName} length`);
  for (let i = 0; i < wantSet.length; i++) eq(doc.modes[docName][i], wantSet[i], `modes.${docName}[${i}]`);
}
const slots = one(show, /const SLOTS = \[([\s\S]*?)\n\];/, 'SLOTS')[1];
const slotRows = [...slots.matchAll(/\{\s*x:\s*(-?[\d.]+),\s*z:\s*(-?[\d.]+),\s*R:\s*([\d.]+),\s*top:\s*([\d.]+),\s*yaw:\s*(-?[\d.]+)\s*\}/g)];
eq(slotRows.length, 4, 'JS SLOTS row count');
eq(doc.pedestal.SLOTS.length, 4, 'pedestal.SLOTS length');
for (let i = 0; i < 4; i++) {
  const [, x, z, R, top, yaw] = slotRows[i];
  const row = doc.pedestal.SLOTS[i];
  near(row.x, Number(x), `SLOTS[${i}].x`);
  near(row.z, Number(z), `SLOTS[${i}].z`);
  near(row.R, Number(R), `SLOTS[${i}].R`);
  near(row.top, Number(top), `SLOTS[${i}].top`);
  near(row.yaw, Number(yaw), `SLOTS[${i}].yaw`);
}
const ped = one(show, /const PED = \{([\s\S]*?)\};/, 'PED')[1];
const drum = one(show, /const DRUM = \{([\s\S]*?)\};/, 'DRUM')[1];
for (const [name, body, row] of [['PED', ped, doc.pedestal.PED], ['DRUM', drum, doc.pedestal.DRUM]]) {
  for (const key of ['bevel', 'bottom', 'ink']) {
    near(row[key], num(body, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `${name}.${key}`), `pedestal.${name}.${key}`);
  }
  for (const [sub, keys] of [['groove', ['y', 'h', 'd']], ['flange', ['y', 'out', 'b']]]) {
    const subBody = one(body, new RegExp(`${sub}:\\s*\\{([^}]*)\\}`), `${name}.${sub}`)[1];
    for (const k of keys) {
      near(row[sub][k], num(subBody, new RegExp(`\\b${k}:\\s*(-?[\\d.]+)`), `${name}.${sub}.${k}`), `pedestal.${name}.${sub}.${k}`);
    }
  }
}
near(doc.pedestal.PED.R, num(ped, /\bR:\s*(-?[\d.]+)/, 'PED.R'), 'pedestal.PED.R');
// Key list comes from the JS source (not Object.keys(artifact)) so a missing
// artifact field fails the check instead of silently shrinking the loop.
const swim = one(show, /const SWIM = \{([^}]*)\};/, 'SWIM')[1];
const swimKeys = [...swim.matchAll(/(\w+):\s*(-?[\d.]+)/g)].map((m) => m[1]);
eq(swimKeys.length, Object.keys(doc.pedestal.SWIM).length, 'SWIM key count');
for (const key of swimKeys) {
  near(doc.pedestal.SWIM[key], num(swim, new RegExp(`\\b${key}:\\s*(-?[\\d.]+)`), `SWIM.${key}`), `pedestal.SWIM.${key}`);
}

// ---- emotes ---------------------------------------------------------------
const emoteBlock = one(show, /const LOB_EMOTES = \{([\s\S]*?)\n\};/, 'LOB_EMOTES')[1];
const emoteRows = [...emoteBlock.matchAll(/(\w+):\s*\{\s*dance:\s*'([^']+)',\s*v:\s*([\d.]+),\s*(?:t0:\s*([\d.]+),\s*)?dur:\s*([\d.]+),\s*hop:\s*([\d.]+)(?:,\s*trig:\s*'([^']+)')?\s*\}/g)];
eq(emoteRows.length, 4, 'LOB_EMOTES row count');
eq(Object.keys(doc.emotes).length, 4, 'emotes key count');
for (const [, name, dance, v, t0, dur, hop, trig] of emoteRows) {
  const e = doc.emotes[name];
  if (!e) throw new Error(`emotes.${name} missing from lobbyset.json`);
  eq(e.dance, dance, `emotes.${name}.dance`);
  near(e.v, Number(v), `emotes.${name}.v`);
  near(e.dur, Number(dur), `emotes.${name}.dur`);
  near(e.hop, Number(hop), `emotes.${name}.hop`);
  eq(e.t0 ?? null, t0 === undefined ? null : Number(t0), `emotes.${name}.t0`);
  eq(e.trig ?? null, trig ?? null, `emotes.${name}.trig`);
}

// ---- Task 67 materials: independent re-parse (different regex path) -------
const M = doc.materials;
if (!M) throw new Error('materials section missing from lobbyset.json');
// Reflection: ground material line + Fresnel/k/mix GLSL lines.
const gLine = mats.split('\n').find((l) => l.includes('envMapIntensity: 1'));
near(M.reflection.env_map_intensity, num(gLine, /envMapIntensity:\s*([\d.]+)/, 'refl.envI'), 'materials.reflection.env_map_intensity');
const fLine = mats.split('\n').find((l) => l.includes('float F = 0.02'));
near(M.reflection.fresnel_f0, num(fLine, /float F =\s*([\d.]+)/, 'refl.F0'), 'materials.reflection.fresnel_f0');
near(M.reflection.fresnel_scale, num(fLine, /\+\s*([\d.]+)\s*\*\s*pow/, 'refl.Fscale'), 'materials.reflection.fresnel_scale');
near(M.reflection.fresnel_pow, num(fLine, /NdV,\s*([\d.]+)\)/, 'refl.Fpow'), 'materials.reflection.fresnel_pow');
const kLine = mats.split('\n').find((l) => l.includes('float k = clamp'));
near(M.reflection.k_damp, num(kLine, /gDamp\s*\*\s*([\d.]+)/, 'refl.kDamp'), 'materials.reflection.k_damp');
near(M.reflection.k_ink, num(kLine, /gInk\s*\*\s*([\d.]+)/, 'refl.kInk'), 'materials.reflection.k_ink');
const wm = (one(mats, /F \* mix\(([^)]*)\)/, 'refl.waterMix')[1].match(/-?[\d.]+/g) || []).map(Number);
eq(M.reflection.water_mix.length, wm.length, 'materials.reflection.water_mix length');
for (let i = 0; i < wm.length; i++) near(M.reflection.water_mix[i], wm[i], `materials.reflection.water_mix[${i}]`);
// Neon: team colors from makeUniforms line, core color + intensity vec4.
const teamLine = mats.split('\n').find((l) => l.includes('uTeamA:'));
eq(M.neon.team_a, one(teamLine, /uTeamA:[^']*'([^']+)'/, 'teamA')[1], 'materials.neon.team_a');
eq(M.neon.team_b, one(teamLine, /uTeamB:[^']*'([^']+)'/, 'teamB')[1], 'materials.neon.team_b');
const core = one(mats, /uColC: \{ value: new THREE\.Color\(([^)]*)\)/, 'neon core')[1].split(',').map(Number);
const inten = one(mats, /uI: \{ value: new THREE\.Vector4\(([^)]*)\)/, 'neon I')[1].split(',').map(Number);
eq(M.neon.core_color.length, core.length, 'materials.neon.core_color length');
eq(M.neon.intensity.length, inten.length, 'materials.neon.intensity length');
for (let i = 0; i < core.length; i++) near(M.neon.core_color[i], core[i], `materials.neon.core_color[${i}]`);
for (let i = 0; i < inten.length; i++) near(M.neon.intensity[i], inten[i], `materials.neon.intensity[${i}]`);
// Steam: cards/geo from lobbySet.js, uniforms/GLSL from mats.
const fxBody = one(set, /_fx\(\)\s*\{[\s\S]*?steam: three crossed cards[\s\S]*?(pos\.push\([^\n]*seed\.push\(i \* [\d.]+[^\n]*\n)/, 'steam fx')[1];
const sGeo = fxBody;
near(M.steam.cards, num(set, /for \(let i = 0; i < (\d+); i\+\+\) \{\s*\n\s*const a = \(i \/ \d+\)/, 'steam cards'), 'materials.steam.cards');
near(M.steam.x, num(sGeo, /pos\.push\(([-\d.]+)/, 'steam x'), 'materials.steam.x');
near(M.steam.card_height, num(sGeo, /,\s*v \*\s*([\d.]+),/, 'steam H'), 'materials.steam.card_height');
near(M.steam.z, num(sGeo, /v \*\s*[\d.]+,\s*([-\d.]+)\s*\+/, 'steam z'), 'materials.steam.z');
near(M.steam.seed_step, num(sGeo, /seed\.push\(i \*\s*([\d.]+)/, 'steam seed step'), 'materials.steam.seed_step');
const sCol = one(mats, /uCol: \{ value: new THREE\.Color\(([^)]*)\)/, 'steam color')[1].split(',').map(Number);
const sNoise = one(mats, /vec2 p = vUv \* vec2\(([^)]*)\)/, 'steam noise')[1].split(',').map(Number);
const sEdge = (one(mats, /float a = smoothstep\(([^)]*)\)/, 'steam edge')[1].match(/[\d.]+/g) || []).map(Number);
near(M.steam.time_scale, num(mats, /float t = uTime \*\s*([\d.]+)/, 'steam time'), 'materials.steam.time_scale');
near(M.steam.alpha_scale, num(mats, /\*\s*([\d.]+)\s*;\s*\n\s*vec3 c = mix\(uCol/, 'steam alpha'), 'materials.steam.alpha_scale');
eq(M.steam.color.length, sCol.length, 'materials.steam.color length');
for (let i = 0; i < sCol.length; i++) near(M.steam.color[i], sCol[i], `materials.steam.color[${i}]`);
eq(M.steam.noise_scale.length, sNoise.length, 'materials.steam.noise_scale length');
for (let i = 0; i < sNoise.length; i++) near(M.steam.noise_scale[i], sNoise[i], `materials.steam.noise_scale[${i}]`);
eq(M.steam.alpha_edge.length, sEdge.length, 'materials.steam.alpha_edge length');
for (let i = 0; i < sEdge.length; i++) near(M.steam.alpha_edge[i], sEdge[i], `materials.steam.alpha_edge[${i}]`);
// Lobby PMREM: fromScene args + normalize + envIntensity + env card k.
const fsArgs = one(set, /fromScene\(this\._envScene,\s*([\d.]+,\s*[\d.]+,\s*[\d.]+,\s*\{ size:\s*[\d.]+,\s*position: V\([^)]*\))\s*\}\)/, 'pmrem args')[1]
  .match(/-?[\d.]+/g).map(Number);
near(M.pmrem.sigma, fsArgs[0], 'materials.pmrem.sigma');
near(M.pmrem.near, fsArgs[1], 'materials.pmrem.near');
near(M.pmrem.far, fsArgs[2], 'materials.pmrem.far');
near(M.pmrem.size, fsArgs[3], 'materials.pmrem.size');
eq(M.pmrem.position.length, 3, 'materials.pmrem.position length');
for (let i = 0; i < 3; i++) near(M.pmrem.position[i], fsArgs[4 + i], `materials.pmrem.position[${i}]`);
near(M.pmrem.team_normalize, num(set, /multiplyScalar\(\s*([\d.]+)\s*\/\s*Math\.max/, 'pmrem norm'), 'materials.pmrem.team_normalize');
near(M.pmrem.environment_intensity, num(set, /this\.environmentIntensity =\s*([\d.]+)/, 'envIntensity'), 'materials.pmrem.environment_intensity');
for (const [key, needle] of [
  ['door', "'#ffb366',"],
  ['neon', "'#ffffff', 4);"],
  ['bulbs', "'#ffcf8a',"],
  ['vending', "'#cfefff',"],
]) {
  const l = set.split('\n').find((x) => x.includes(needle));
  near(M.pmrem.env_card_k[key], num(l, /'(?:[^']*)',\s*([\d.]+)\)/, `env k ${key}`), `materials.pmrem.env_card_k.${key}`);
}

// ---- upstream pin ---------------------------------------------------------
const pinned = execSync(
  'git log -1 --format=%H -- src/game/lobbySet.js src/game/lobbySet-mats.js src/game/showcase.js',
  { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] },
).toString().trim();
eq(doc.source.commit, pinned, 'source.commit');
eq(doc.source.files.length, 3, 'source.files length');
eq(doc.schema, 1, 'schema');

console.log(`lobbyset-crosscheck: ${checks} checks OK (ALLEY/CAM/3 quality rows/8 bearing spots/hub + showcase constants/SLOTS/PED/DRUM/SWIM/mode sets/emotes + Task67 materials)`);
