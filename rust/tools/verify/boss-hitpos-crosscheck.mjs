// Task 42 batch A — independent evidence chain for `boss.hit[].pos` in
// rust/assets/tuning.json (schema inkwave.tuning.v5).
//
// extract-tuning.mjs derives hitPos through a static shortcut: the rest bone
// chain is pure translation, so the socket world point collapses to the
// sockets-table literal p (spec Task 42 §1). This script re-derives the same
// ten points the *dynamic* way instead: it imports the real THREE module and
// the real bossModelGeo.js ESM surface (no slicing of the dependency blocks),
// runs buildRig(), mounts the rig under a Group, calls updateMatrixWorld(true)
// and replays BossModel.getSocket semantics exactly:
//   off = p - rest[bone]                       (src/boss/bossModel.js L56)
//   world = off.applyMatrix4(bone.matrixWorld) (src/boss/bossModel.js L151-154)
// The three constructor overrides (src/boss/bossModel.js L57-59) keep their
// full `.sub(rig.rest.*)` term and go through the same matrixWorld composite.
// Every component must match tuning.json within 1e-4; regular sockets also
// assert the literal point p itself. Prints the expected table for the
// inkwave_sim anchor tests, then `ok`. Any mismatch throws (exit code != 0).
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as THREE from 'three';
import { buildRig, CONT, ARMS, HEAD, cont, V3 } from '../../../src/boss/bossModelGeo.js';

const here = dirname(fileURLToPath(import.meta.url));   // rust/tools/verify
const repoRoot = resolve(here, '../../..');

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

const geoSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModelGeo.js'), 'utf8');
const modelSrc = readFileSync(resolve(repoRoot, 'src/boss/bossModel.js'), 'utf8');
const tuning = JSON.parse(readFileSync(resolve(repoRoot, 'rust/assets/tuning.json'), 'utf8'));
if (tuning.schema !== 'inkwave.tuning.v5') throw new Error(`tuning.json schema ${tuning.schema} != inkwave.tuning.v5`);

// The sockets table (src/boss/bossModelGeo.js L822-829) is local to
// buildParts(); eval it against the REAL module exports (V3/cont/ARMS/HEAD/
// CONT) — only the literal block itself is sliced.
const srcSOCKETS = sliceBlock(geoSrc, 'bossModelGeo.js', 'sockets', 'const sockets = {', '\n  };');
assertContains(srcSOCKETS, "clawL: ['clawL', ARMS[0].W.clone().addScaledVector(ARMS[0].D, 1.05)]", 'bossModelGeo.js sockets');
const sockets = new Function('V3', 'cont', 'ARMS', 'HEAD', 'CONT', [
  'const { hl, hw, hh } = CONT;',
  srcSOCKETS,
  'return sockets;',
].join('\n'))(V3, cont, ARMS, HEAD, CONT);

// The three constructor overrides (src/boss/bossModel.js L57-59), full off
// expressions with `this.rig` rebound to the real rig below.
const OVERRIDE_PIN = {
  shellF: 'new THREE.Vector3(0, 0.05, 1.5)',   // JS src/boss/bossModel.js L57
  shellR: 'new THREE.Vector3(0, 0.1, -1.55)',  // JS src/boss/bossModel.js L58
  body: 'new THREE.Vector3(0, 2.2, 2.3)',      // JS src/boss/bossModel.js L59
};
function overrideOffExpr(name) {
  const i = modelSrc.indexOf(`this._sock.${name} = {`);
  if (i < 0) throw new Error(`bossModel.js: _sock.${name} override not found`);
  const line = modelSrc.slice(i, modelSrc.indexOf('\n', i));
  assertContains(line, OVERRIDE_PIN[name], `bossModel.js _sock.${name}`);
  const a = line.indexOf('off: ');
  const b = line.lastIndexOf(' };');
  if (a < 0 || b <= a) throw new Error(`bossModel.js: cannot parse _sock.${name} off expression`);
  return line.slice(a + 5, b).replaceAll('this.rig', 'rig');
}
const OVERRIDE_BONE = { shellF: 'shell', shellR: 'shell', body: 'head' };

// Real rig, real bone chain, rest pose (nothing animated).
const rig = buildRig();
const host = new THREE.Group();
host.add(rig.by.base);
host.updateMatrixWorld(true);

const EXPECTED_ORDER = ['shellF', 'shellR', 'body', 'clawL', 'clawR', 'eyeL', 'eyeR', 'belly', 'crackL', 'crackR']; // JS src/boss/bossModel.js L14-18
const TOL = 1e-4;
const rows = [];
const hit = tuning.boss.hit;
if (hit.length !== 10) throw new Error(`tuning.json: boss.hit has ${hit.length} entries, expected 10`);

for (let i = 0; i < 10; i++) {
  const { socket, pos } = hit[i];
  if (socket !== EXPECTED_ORDER[i]) throw new Error(`hit[${i}]: socket ${socket} != ${EXPECTED_ORDER[i]}`);
  if (!Array.isArray(pos) || pos.length !== 3) throw new Error(`hit[${i}]: missing pos`);

  let boneName;
  let off;
  if (socket in OVERRIDE_BONE) {
    boneName = OVERRIDE_BONE[socket];
    off = new Function('THREE', 'CONT', 'rig', `return (${overrideOffExpr(socket)});`)(THREE, CONT, rig);
  } else {
    const entry = sockets[socket];
    if (!entry) throw new Error(`sockets table: no entry for ${socket}`);
    [boneName, off] = [entry[0], entry[1].clone().sub(rig.rest[entry[0]])];
    // static-route cross-check: the artifact must equal the literal point p
    const p = entry[1];
    for (let k = 0; k < 3; k++) {
      if (Math.abs(p.toArray()[k] - pos[k]) > TOL) {
        throw new Error(`hit[${i}] ${socket}: literal p ${p.toArray()} != tuning pos ${pos}`);
      }
    }
  }
  const bone = rig.by[boneName];
  if (!bone) throw new Error(`rig: no bone ${boneName} for socket ${socket}`);
  const world = off.applyMatrix4(bone.matrixWorld).toArray();
  for (let k = 0; k < 3; k++) {
    if (Math.abs(world[k] - pos[k]) > TOL) {
      throw new Error(`hit[${i}] ${socket}: getSocket rest world ${world} != tuning pos ${pos} (tol ${TOL})`);
    }
  }
  rows.push(`${socket.padEnd(7)} bone=${boneName.padEnd(7)} world=[${world.join(', ')}]`);
}

console.log(`boss-hitpos-crosscheck: 10/10 sockets within ${TOL} (rig bones=${rig.bones.length}, schema=${tuning.schema})`);
for (const r of rows) console.log('  ' + r);
console.log('ok');
