// Task 68: extract the HULLBREAKER high-fidelity MODEL metadata from the
// READ-ONLY JS tree (JS tree stays byte-clean). Unlike the pure-regex extractors,
// this one imports the actual `bossModelGeo.js` (three is available in
// node_modules) and CALLS buildRig()/buildParts() to obtain the deterministic
// 100-bone skeleton and the real per-tier triangle budgets. Sim/hitShapes
// geometry is untouched (M4 Task41/46 contract) — this is presentation metadata
// only. Output: rust/assets/boss.json (stable-json).
import { upstreamCommit } from './upstream-commit.mjs';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/boss.json');
const MODEL_FILES = [
  'src/boss/bossModelGeo.js',
  'src/boss/bossModel.js',
  'src/boss/bossAnim.js',
  'src/boss/bossMats.js',
  'src/boss/bossModelFx.js',
];
const commit = upstreamCommit(repoRoot, MODEL_FILES);

function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`expected fragment not found in ${where}: ${needle}`);
}
function numAfter(s, needle) {
  const i = s.indexOf(needle);
  if (i < 0) throw new Error(`no ${needle} in ${s}`);
  const m = s.slice(i + needle.length).match(/-?\d+(?:\.\d+)?/);
  if (!m) throw new Error(`no number after ${needle} in ${s}`);
  return Number(m[0]);
}
function lineOf(src, needle, where) {
  const hit = src.split('\n').find((l) => l.includes(needle));
  if (!hit) throw new Error(`${where}: no line with ${needle}`);
  return hit;
}
function nums(s) {
  return (s.match(/-?\d+(?:\.\d+)?/g) || []).map(Number);
}

const geoSrc = readFileSync(resolve(repoRoot, MODEL_FILES[0]), 'utf8');
const modelSrc = readFileSync(resolve(repoRoot, MODEL_FILES[1]), 'utf8');
const animSrc = readFileSync(resolve(repoRoot, MODEL_FILES[2]), 'utf8');
const matsSrc = readFileSync(resolve(repoRoot, MODEL_FILES[3]), 'utf8');

// ---- Rig: import the REAL module and call buildRig() ----------------------
const { buildRig, buildParts, QUALITY_K } = await import(
  new URL(`../../../${MODEL_FILES[0]}`, import.meta.url).href
);
const rig = buildRig();
if (rig.bones.length !== 100) throw new Error(`expected 100 bones, got ${rig.bones.length}`);

// Canonical bone tree: each bone's name, parent name (or ''), rest position.
// Rest position is read from rig.rest[name] (model-space), quantized to 1e-6
// before serialization so the cross-language SHA256 is libm-independent (rest
// positions come from literal V3 math, no pow).
const boneRows = rig.bones.map((b) => {
  const parent = b.parent ? b.parent.name : '';
  const rest = rig.rest[b.name];
  if (!rest) throw new Error(`bone ${b.name} missing rest`);
  return { name: b.name, parent, rest: [rest.x, rest.y, rest.z] };
});
const Q = 1e6;
// Store the quantized rest position alongside each bone in the artifact so the
// Rust data plane can RECOMPUTE the same SHA256 (no hardcoded golden, no
// self-verification): canonical line = name|parent|qx,qy,qz (i64, 1e6 scale).
const canonical = boneRows.map(
  (b) => `${b.name}|${b.parent}|${b.rest.map((v) => Math.round(v * Q)).join(',')}`
).join('\n');
const boneTreeSha256 = createHash('sha256').update(canonical).digest('hex');
for (const b of boneRows) b.rest_q = b.rest.map((v) => Math.round(v * Q));

// Chains (verlet whips) — deterministic names from the real rig.
const chains = rig.chains.map((c) => c.name);

// ---- Parts: real per-tier triangle budgets --------------------------------
const tiers = ['low', 'medium', 'high', 'ultra'];
const triBudget = {};
for (const tier of tiers) {
  const parts = buildParts(buildRig(), tier);
  const buckets = {};
  let total = 0;
  for (const [name, bucket] of Object.entries(parts.buckets)) {
    buckets[name] = bucket.tris;
    total += bucket.tris;
  }
  triBudget[tier] = { scale: QUALITY_K[tier], buckets, total };
}
for (const tier of tiers) {
  if (triBudget[tier].total > 150000) throw new Error(`${tier} exceeds 150k tri budget`);
}

// ---- HIT table: regex-parse the literal array from bossModel.js L14-18 ----
const hitStart = modelSrc.indexOf('const HIT = [');
const hitEnd = modelSrc.indexOf('];', hitStart);
if (hitStart < 0 || hitEnd < 0) throw new Error('HIT table markers not found');
const hitBlock = modelSrc.slice(hitStart, hitEnd + 2);
const hitRows = [...hitBlock.matchAll(/\['(\w+)',\s*([\d.]+),\s*(true|false)\]/g)]
  .map((m) => ({ socket: m[1], radius: Number(m[2]), weak: m[3] === 'true' }));
if (hitRows.length !== 10) throw new Error(`expected 10 HIT rows, got ${hitRows.length}`);
assertContains(modelSrc, 'h.socket === \'belly\'', 'belly active condition');
assertContains(modelSrc, 'A.sp.tear.x > 0.5', 'crack active condition');

// ---- Animation constants: DUR / PH / clip priority from bossAnim.js -------
const durLine = lineOf(animSrc, 'const DUR = {', 'DUR');
const durObj = {};
for (const m of durLine.matchAll(/(\w+):\s*\[([\d.]+),\s*([\d.]+),\s*([\d.]+)\]/g)) {
  durObj[m[1]] = [Number(m[2]), Number(m[3]), Number(m[4])];
}
const MOVES = ['slam', 'barrage', 'sweep', 'charge', 'crablets', 'frenzy'];
if (MOVES.some((k) => !durObj[k])) throw new Error('DUR missing a move');
const phLine = lineOf(animSrc, 'const PH = {', 'PH');
const ph = { tele: numAfter(phLine, 'tele: '), act: numAfter(phLine, 'act: '), rec: numAfter(phLine, 'rec: ') };
// clip selection priority — pinned as an ordered literal list from L484-495.
const clipPriority = ['dead', 'intro', 'stun', 'roar', 'move:phase', 'recover', 'loco'];
for (const needle of ['st.dead', "st.move === 'intro'", 'st.stunned', "st.move === 'roar'", 'st.move && CLIPS[st.move]', 'this.recoverT < 1.0', "key = 'loco'"]) {
  assertContains(animSrc, needle, 'clip selection');
}

// ---- Material slots: 5 skinned buckets + stencil atlas --------------------
const bucketLine = lineOf(matsSrc, 'makeBossMaterials', 'boss materials');
assertContains(matsSrc, 'makeStencilTexture', 'stencil atlas');
const stencilSize = numAfter(lineOf(matsSrc, 'cv.width = cv.height = 1024', 'stencil size'), 'cv.height = ');
const materialSlots = ['steel', 'cara', 'flesh', 'eye', 'junk'];
for (const s of materialSlots) assertContains(geoSrc, `new Bucket('${s}')`, 'material bucket');

const doc = {
  schema: 1,
  source: { commit, files: MODEL_FILES },
  rig: {
    bone_count: rig.bones.length,
    bone_tree_sha256: boneTreeSha256,
    bones: boneRows.map((b) => ({ name: b.name, parent: b.parent, rest_q: b.rest_q })),
    chains,
  },
  tri_budget: triBudget,
  hit: hitRows,
  animation: { moves: MOVES, dur: durObj, ph, clip_priority: clipPriority },
  materials: { slots: materialSlots, stencil_atlas: true, stencil_size: stencilSize, skin_bone_count: rig.bones.length },
};
const out = stableStringify(doc) + '\n';
if (out.includes('NaN') || out.includes('Infinity')) throw new Error('boss.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(
  `wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, bones=${rig.bones.length}, ` +
  `tri high=${triBudget.high.total}, hit=${hitRows.length}`
);
