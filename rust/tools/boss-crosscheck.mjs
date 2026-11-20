// Task 68 TR-68.1 independent crosscheck.
//
// Deliberately does NOT import extract-boss.mjs. It re-imports the READ-ONLY
// bossModelGeo.js directly and recomputes the rig/tri budgets with its own
// serialization, re-parses HIT/DUR/PH from the JS sources with independent
// regexes, and reconciles against rust/assets/boss.json. Any upstream drift or
// extractor regression fails the gate's XCHK stage.
import { execSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools
const repoRoot = resolve(here, '../..');                   // repository root
const doc = JSON.parse(readFileSync(resolve(here, '../assets/boss.json'), 'utf8'));

let checks = 0;
function eq(actual, expected, label) {
  if (actual !== expected) throw new Error(`${label}: got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  checks++;
}
function near(actual, expected, label) {
  if (!Number.isFinite(actual) || Math.abs(actual - expected) > 1e-9) throw new Error(`${label}: got ${actual}, expected ${expected}`);
  checks++;
}

// ---- Rig: recompute bone tree from the real module, independent walk ------
const { buildRig, buildParts, QUALITY_K } = await import(
  `file://${resolve(repoRoot, 'src/boss/bossModelGeo.js')}`
);
const rig = buildRig();
eq(doc.rig.bone_count, rig.bones.length, 'rig.bone_count');
eq(rig.bones.length, 100, 'live rig has 100 bones');
eq(doc.rig.bones.length, rig.bones.length, 'rig.bones length');
// Crosscheck walks by index and verifies name + parent against the artifact.
const Q = 1e6;
const parts = [];
for (let i = 0; i < rig.bones.length; i++) {
  const b = rig.bones[i];
  const row = doc.rig.bones[i];
  eq(row.name, b.name, `rig.bones[${i}].name`);
  eq(row.parent, b.parent ? b.parent.name : '', `rig.bones[${i}].parent`);
  const rest = rig.rest[b.name];
  const restQ = [rest.x, rest.y, rest.z].map((v) => Math.round(v * Q));
  eq(row.rest_q.length, 3, `rig.bones[${i}].rest_q length`);
  for (let k = 0; k < 3; k++) eq(row.rest_q[k], restQ[k], `rig.bones[${i}].rest_q[${k}]`);
  parts.push(`${b.name}|${b.parent ? b.parent.name : ''}|${restQ.join(',')}`);
}
const sha = createHash('sha256').update(parts.join('\n')).digest('hex');
eq(doc.rig.bone_tree_sha256, sha, 'rig.bone_tree_sha256');
// Chains recomputed from the live rig.
eq(doc.rig.chains.length, rig.chains.length, 'rig.chains length');
for (let i = 0; i < rig.chains.length; i++) eq(doc.rig.chains[i], rig.chains[i].name, `rig.chains[${i}]`);

// ---- Tri budgets: recompute from the real buildParts ---------------------
for (const tier of ['low', 'medium', 'high', 'ultra']) {
  const bp = buildParts(buildRig(), tier);
  const buckets = Object.fromEntries(Object.entries(bp.buckets).map(([k, v]) => [k, v.tris]));
  const total = Object.values(buckets).reduce((a, b) => a + b, 0);
  const row = doc.tri_budget[tier];
  if (!row) throw new Error(`tri_budget.${tier} missing`);
  near(row.scale, QUALITY_K[tier], `tri_budget.${tier}.scale`);
  eq(Object.keys(row.buckets).length, Object.keys(buckets).length, `tri_budget.${tier} bucket count`);
  for (const [k, v] of Object.entries(buckets)) eq(row.buckets[k], v, `tri_budget.${tier}.${k}`);
  eq(row.total, total, `tri_budget.${tier}.total`);
  if (total > 150000) throw new Error(`${tier} over 150k budget`);
  checks++;
}

// ---- HIT table: independent regex from bossModel.js ---------------------
const modelSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModel.js'), 'utf8');
const hitStart = modelSrc.indexOf('const HIT = [');
const hitEnd = modelSrc.indexOf('];', hitStart);
const hitBlock = modelSrc.slice(hitStart, hitEnd + 2);
const hitRows = [...hitBlock.matchAll(/\['(\w+)',\s*([\d.]+),\s*(true|false)\]/g)];
eq(hitRows.length, 10, 'HIT row count');
eq(doc.hit.length, hitRows.length, 'doc.hit length');
for (let i = 0; i < hitRows.length; i++) {
  const [, socket, radius, weak] = hitRows[i];
  eq(doc.hit[i].socket, socket, `hit[${i}].socket`);
  near(doc.hit[i].radius, Number(radius), `hit[${i}].radius`);
  eq(doc.hit[i].weak, weak === 'true', `hit[${i}].weak`);
}

// ---- Animation DUR / PH / clip priority from bossAnim.js ----------------
const animSrc = readFileSync(resolve(repoRoot, 'src/boss/bossAnim.js'), 'utf8');
const durLine = animSrc.split('\n').find((l) => l.includes('const DUR = {'));
for (const move of doc.animation.moves) {
  const m = durLine.match(new RegExp(`${move}:\\s*\\[([\\d.]+),\\s*([\\d.]+),\\s*([\\d.]+)\\]`));
  if (!m) throw new Error(`DUR missing ${move}`);
  for (let i = 0; i < 3; i++) near(doc.animation.dur[move][i], Number(m[i + 1]), `dur.${move}[${i}]`);
}
const phLine = animSrc.split('\n').find((l) => l.includes('const PH = {'));
for (const k of ['tele', 'act', 'rec']) {
  const m = phLine.match(new RegExp(`${k}:\\s*(\\d+)`));
  eq(doc.animation.ph[k], Number(m[1]), `ph.${k}`);
}
eq(doc.animation.clip_priority.length, 7, 'clip priority length');
eq(doc.animation.moves.length, 6, 'moves length');

// ---- Material slots: 5 buckets from bossModelGeo.js ---------------------
const geoSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModelGeo.js'), 'utf8');
const bucketRow = geoSrc.split('\n').find((l) => l.includes('new Bucket(\'steel\')'));
const slots = [...bucketRow.matchAll(/new Bucket\('(\w+)'\)/g)].map((m) => m[1]);
eq(doc.materials.slots.length, slots.length, 'material slots length');
for (let i = 0; i < slots.length; i++) eq(doc.materials.slots[i], slots[i], `material slot ${i}`);
const matsSrc = readFileSync(resolve(repoRoot, 'src/boss/bossMats.js'), 'utf8');
eq(doc.materials.stencil_atlas, matsSrc.includes('function makeStencilTexture'), 'stencil atlas flag');
eq(doc.materials.stencil_size, Number(matsSrc.split('\n').find((l) => l.includes('cv.width = cv.height =')).match(/cv\.height =\s*(\d+)/)[1]), 'stencil size');

// ---- upstream pin --------------------------------------------------------
const pinned = execSync(
  'git log -1 --format=%H -- src/boss/bossModelGeo.js src/boss/bossModel.js src/boss/bossAnim.js src/boss/bossMats.js src/boss/bossModelFx.js',
  { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] },
).toString().trim();
eq(doc.source.commit, pinned, 'source.commit');
eq(doc.source.files.length, 5, 'source.files length');
eq(doc.schema, 1, 'schema');

console.log(`boss-crosscheck: ${checks} checks OK (100-bone tree+SHA256, 4-tier tri budgets, 10 HIT, 6-move DUR/PH, 5 material slots, upstream pin)`);
