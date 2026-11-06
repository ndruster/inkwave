// Task 53 (D-T53-1) prop extractor: bakes every stage's set dressing (src/world/
// props.js PropKit + dressing.js placements + decor.js Decor + murals.js placement
// table) into a per-stage binary template/instance archive + stable JSON manifest:
//
//   rust/assets/props/<stage>.bin.gz   (LE binary, format: props.schema.md)
//   rust/assets/props/<stage>.json     (manifest: meta / placements / lamps / murals / counts)
//
// Harness = the REAL JS modules under node: PropKit(headless) never touches the DOM
// (props.js L3103), Decor/murals only need a no-op canvas 2D stub (the atlas pixels
// are not ported — D-T53-6③, we read the mural placement table from userData.murals).
// Each dressingFor(id) placement is baked exactly as mergeParts() would (AO falloff,
// vertex colour, negative-determinant winding flip — props.js L1087-1132), then
// undone into PROP-LOCAL space (inverse of the begin matrix), welded on a 1 mm /
// 1e-3-normal / 8-bit-colour grid and content-hash deduplicated into a template
// table; instances carry (templateId, bucket, pos mm, rotY, scale, modeFlags) so
// the Rust runtime re-applies the placement transform.
//
// Deviations from the JS runtime (all declared in the approved spec D-T53-6):
//  - spin/blink records are stored in WORLD space (self-contained; JS update()
//    composes r.base × rotY/Z(time·speed+phase) on the world matrix anyway);
//  - flags/banners bake as static cloth geometry (cloth shader not ported);
//  - blob buckets are skipped entirely (shadow-catcher decals, not ported);
//  - quantisation: pos i32 mm, normal i16 ×1e3, colour u8 (D-T53-6⑥ — i32 pos
//    because pieredge/halyard local geometry exceeds the i16 mm range);
//  - prop colliders are collected ONLY to reproduce the JS groundHeight context
//    for Decor + the uLamps table (D-T53-6①: they do not ride the archive).
//
// Deterministic (TR-3.4): same upstream commit ⇒ byte-identical .bin.gz + .json.
// The JS tree is never modified (AC-1/TR-2.4).
//
//   node extract-props.mjs [stage]     (stage defaults to: all eight)
import { mkdirSync, writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));      // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outDir = resolve(here, '../../assets/props');

const STAGES = ['tidewater', 'kelpline', 'halyard', 'saltpan', 'crossmarket', 'lockgate', 'terraces', 'cargo'];
const stageArg = process.argv[2] ?? null;
if (stageArg && !STAGES.includes(stageArg)) {
  console.error(`extract-props: unknown stage "${stageArg}" (expected one of: ${STAGES.join(', ')})`);
  process.exit(1);
}

// ---------------------------------------------------------------- DOM stub (canvas 2D, no pixels)
// Decor + murals.js draw into document.createElement('canvas') contexts; the
// extractor only needs the calls to exist (emblem/mural pixels are not ported).
function makeCtx(canvas) {
  const store = { canvas };
  return new Proxy(store, {
    get(t, k) {
      if (k in t) return t[k];
      if (k === 'measureText') return () => ({ width: 0 });
      if (k === 'createLinearGradient' || k === 'createRadialGradient' || k === 'createConicGradient') {
        return () => ({ addColorStop() {} });
      }
      if (k === 'getImageData') return (x, y, w, h) => ({ width: w, height: h, data: new Uint8ClampedArray(Math.max(1, w * h * 4)) });
      return () => undefined;   // every other canvas method: no-op
    },
    set(t, k, v) { t[k] = v; return true; },
  });
}
globalThis.document = {
  createElement: (tag) => {
    if (tag !== 'canvas') return {};
    const cv = { width: 0, height: 0 };
    cv.getContext = () => makeCtx(cv);
    return cv;
  },
  fonts: { load: () => Promise.resolve() },
};

const THREE = await import(resolve(repoRoot, 'node_modules/three/build/three.module.js'));
const { PropKit } = await import(resolve(repoRoot, 'src/world/props.js'));
const { dressingFor } = await import(resolve(repoRoot, 'src/world/dressing.js'));
const { MAP_LAYOUTS } = await import(resolve(repoRoot, 'src/world/maps.js'));
const { layoutFor, inMode } = await import(resolve(repoRoot, 'src/world/variants.js'));
const { Level } = await import(resolve(repoRoot, 'src/world/level.js'));
const { Decor } = await import(resolve(repoRoot, 'src/world/decor.js'));
const { createMuralTexture } = await import(resolve(repoRoot, 'src/world/murals.js'));

const TAU = Math.PI * 2;
const _m = new THREE.Matrix4(), _nm = new THREE.Matrix3(), _v = new THREE.Vector3();
const WHITE = new THREE.Color(1, 1, 1);
const TEAM_COLORS = [new THREE.Color('#ff8a14'), new THREE.Color('#2f5bff')];   // props.js L3102 defaults

const clamp8 = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));

// ---------------------------------------------------------------- cloth geometry replicas
// props.js flagGeo()/bannerGeo() minus the uv/flex/wave attributes (cloth shader
// is not ported — the bake is the static pose the JS frame t=0 would show).
function flagGeo() {
  const W = 0.28, Hf = 0.34;
  const g = new THREE.PlaneGeometry(W, Hf, 2, 4);
  const P = g.attributes.position, n = P.count;
  for (let i = 0; i < n; i++) {
    const y = P.getY(i) - Hf / 2, t = -y / Hf;
    P.setXY(i, P.getX(i) * (1 - t), y);
  }
  g.computeVertexNormals();
  return g;
}
function bannerGeo(w, h) {
  const g = new THREE.PlaneGeometry(w, h, 4, 10);
  const P = g.attributes.position, n = P.count;
  for (let i = 0; i < n; i++) P.setY(i, P.getY(i) - h / 2);
  return g;
}
const CLOTH_FLAG = flagGeo();
const CLOTH_BANNER = bannerGeo(0.8, 1.9);

// ---------------------------------------------------------------- template store
// A template = quantised, welded vertex list + index list, keyed by content hash
// (bucket included: identical geometry in two buckets stays two templates, which
// mirrors how mergeParts() would have emitted two meshes).
class TemplateStore {
  constructor() { this.map = new Map(); this.list = []; }
  add(bucketKey, verts, idx) {
    const h = createHash('sha256').update(bucketKey + '|' + verts.map((v) => v.join(',')).join(';') + '|' + idx.join(',')).digest('hex');
    let id = this.map.get(h);
    if (id === undefined) {
      id = this.list.length;
      this.map.set(h, id);
      this.list.push({ bucketKey, verts, idx });
    }
    return id;
  }
}

// mergeParts() semantics (props.js L1087-1132) baked into prop-local (or world)
// space: parts = [{g, m, c, ao}], undo = matrix that undoes the placement begin
// transform (identity ⇒ keep world). Returns {verts, idx} quantised, or null.
function bakeParts(parts, undo) {
  const recs = new Map();
  const verts = [];
  const idx = [];
  for (const p of parts) {
    const g = p.g, P = g.attributes.position, N = g.attributes.normal, C = g.attributes.color;
    if (!P || P.count === 0) continue;
    const lm = _m.copy(undo).multiply(p.m);
    const le = lm.elements;
    _nm.getNormalMatrix(lm);
    const ne = _nm.elements;
    const flip = lm.determinant() < 0;
    const wm = p.m.elements;
    const cr = p.c.r, cg = p.c.g, cb = p.c.b;
    const pushV = (i) => {
      const vx = P.getX(i), vy = P.getY(i), vz = P.getZ(i);
      let ao = 1;
      if (p.ao != null) {
        const wy = wm[1] * vx + wm[5] * vy + wm[9] * vz + wm[13];
        const hh = Math.max(0, wy - p.ao);
        const t = Math.min(1, hh / 0.55);
        ao = 0.72 + 0.28 * t * t * (3 - 2 * t);
      }
      const lx = le[0] * vx + le[4] * vy + le[8] * vz + le[12];
      const ly = le[1] * vx + le[5] * vy + le[9] * vz + le[13];
      const lz = le[2] * vx + le[6] * vy + le[10] * vz + le[14];
      const nx0 = N ? N.getX(i) : 0, ny0 = N ? N.getY(i) : 1, nz0 = N ? N.getZ(i) : 0;
      let tx = nx0 * ne[0] + ny0 * ne[3] + nz0 * ne[6];
      let ty = nx0 * ne[1] + ny0 * ne[4] + nz0 * ne[7];
      let tz = nx0 * ne[2] + ny0 * ne[5] + nz0 * ne[8];
      const tl = Math.hypot(tx, ty, tz) || 1;
      const kx = C ? C.getX(i) : 1, ky = C ? C.getY(i) : 1, kz = C ? C.getZ(i) : 1;
      const q = [
        Math.round(lx * 1e3), Math.round(ly * 1e3), Math.round(lz * 1e3),
        Math.round((tx / tl) * 1e3), Math.round((ty / tl) * 1e3), Math.round((tz / tl) * 1e3),
        clamp8(cr * kx * ao), clamp8(cg * ky * ao), clamp8(cb * kz * ao),
      ];
      const key = q.join(',');
      let j = recs.get(key);
      if (j === undefined) { j = verts.length; recs.set(key, j); verts.push(q); }
      return j;
    };
    if (g.index) {
      const I = g.index.array;
      for (let i = 0; i < I.length; i += 3) {
        idx.push(pushV(I[i]), pushV(flip ? I[i + 2] : I[i + 1]), pushV(flip ? I[i + 1] : I[i + 2]));
      }
    } else {
      for (let i = 0; i < P.count; i += 3) {
        idx.push(pushV(i), pushV(flip ? i + 2 : i + 1), pushV(flip ? i + 1 : i + 2));
      }
    }
  }
  return verts.length ? { verts, idx } : null;
}

// world AABB of a baked (local) template under the placement matrix
function worldBBox(entry, M, box) {
  const e = M.elements;
  for (const v of entry.verts) {
    const x = v[0] / 1e3, y = v[1] / 1e3, z = v[2] / 1e3;
    const wx = e[0] * x + e[4] * y + e[8] * z + e[12];
    const wy = e[1] * x + e[5] * y + e[9] * z + e[13];
    const wz = e[2] * x + e[6] * y + e[10] * z + e[14];
    box.min[0] = Math.min(box.min[0], wx); box.min[1] = Math.min(box.min[1], wy); box.min[2] = Math.min(box.min[2], wz);
    box.max[0] = Math.max(box.max[0], wx); box.max[1] = Math.max(box.max[1], wy); box.max[2] = Math.max(box.max[2], wz);
  }
}

const modeFlagsOf = (it) => (it.onlyIn === 'turf' ? 1 : it.onlyIn === 'zones' ? 2 : it.notIn === 'turf' ? 3 : it.notIn === 'zones' ? 4 : 0);

// ---------------------------------------------------------------- per-stage bake
function extractStage(stage) {
  const kit = new PropKit(null, { headless: true, quality: 'high' });
  const items = dressingFor(stage);
  const store = new TemplateStore();
  const instances = [];   // {tid, bucketKey, pos, rotY, scale, modeFlags}
  const spins = [];       // world-space {tid, bucketKey, m[12], speed, phase, axis}
  const blinks = [];      // world-space {pos, scale, col, rate, phase, lo, hi}
  const decorInsts = [];  // {tid, bucketKey, pos, scale}
  const placements = [];
  const colliders = [];
  const spinTpl = {};     // kind -> {tid, bucketKey}
  let nFlags = 0, nBanners = 0;

  const bucketLens = () => { const s = new Map(); for (const [k, a] of kit._buckets) s.set(k, a.length); return s; };

  for (let pi = 0; pi < items.length; pi++) {
    const it = items[pi];
    const pos = it.pos || [0, 0, 0], rotY = it.rotY || 0, scale = it.scale ?? 1;
    const M = new THREE.Matrix4().compose(
      new THREE.Vector3(pos[0], pos[1], pos[2]),
      new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), rotY),
      new THREE.Vector3(scale, scale, scale));
    const Minv = M.clone().invert();
    const modeFlags = modeFlagsOf(it);

    const snap = bucketLens();
    const s0 = kit._spin.length, b0 = kit._blink.length, f0 = kit._flags.length, n0 = kit._banners.length;
    const r = kit.add(it.type, it);
    if (r && r.colliders && inMode(it, 'turf')) colliders.push(...r.colliders);

    const box = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
    const instStart = instances.length;
    let tris = 0;
    for (const [bucketKey, arr] of kit._buckets) {
      const from = snap.get(bucketKey) || 0;
      if (arr.length === from) continue;
      const key = bucketKey.split('~')[0];
      if (key === 'blob') continue;                     // shadow-catcher decals: not ported
      const baked = bakeParts(arr.slice(from), Minv);
      if (!baked) continue;
      const tid = store.add(bucketKey, baked.verts, baked.idx);
      instances.push({ tid, bucketKey, pos, rotY, scale, modeFlags });
      tris += baked.idx.length / 3;
      worldBBox(store.list[tid], M, box);
    }
    // spinners — template per kind baked once (local geometry), record matrix in WORLD space
    for (const rec of kit._spin.slice(s0)) {
      if (!spinTpl[rec.kind]) {
        const g = kit._tplGeo(rec.kind);
        const bk = rec.kind === 'fan' ? 'paint' : 'metal';
        const baked = bakeParts([{ g, m: new THREE.Matrix4(), c: WHITE, ao: null }], new THREE.Matrix4());
        spinTpl[rec.kind] = { tid: store.add(bk, baked.verts, baked.idx), bucketKey: bk };
      }
      const t = spinTpl[rec.kind];
      const e = rec.base.elements;
      spins.push({
        tid: t.tid, bucketKey: t.bucketKey,
        m: [e[0], e[4], e[8], e[12], e[1], e[5], e[9], e[13], e[2], e[6], e[10], e[14]],
        speed: rec.speed, phase: rec.phase, axis: rec.kind === 'fan' ? 1 : 0,
      });
      tris += store.list[t.tid].idx.length / 3;
    }
    // blink lights — world position + uniform scale (sphere: rotation is irrelevant)
    for (const rec of kit._blink.slice(b0)) {
      const e = rec.m.elements;
      blinks.push({
        pos: [e[12], e[13], e[14]],
        scale: Math.hypot(e[0], e[1], e[2]),
        col: [clamp8(rec.color.r), clamp8(rec.color.g), clamp8(rec.color.b)],
        rate: rec.rate, phase: rec.phase, lo: rec.lo, hi: rec.hi,
      });
      tris += 84;
    }
    // flags/banners — static cloth pose baked into the placement's local space (bucket paint)
    for (const [slice0, list, geo] of [[f0, kit._flags, CLOTH_FLAG], [n0, kit._banners, CLOTH_BANNER]]) {
      for (const rec of list.slice(slice0)) {
        const c = rec.team != null ? TEAM_COLORS[rec.team].clone().lerp(WHITE, rec.tint || 0) : rec.color;
        const baked = bakeParts([{ g: geo, m: new THREE.Matrix4().multiplyMatrices(Minv, rec.m), c, ao: null }], new THREE.Matrix4());
        if (!baked) continue;
        const tid = store.add('paint', baked.verts, baked.idx);
        instances.push({ tid, bucketKey: 'paint', pos, rotY, scale, modeFlags });
        tris += baked.idx.length / 3;
        worldBBox(store.list[tid], M, box);
        if (list === kit._flags) nFlags++; else nBanners++;
      }
    }
    placements.push({
      i: pi, type: it.type, pos, rotY, scale, modeFlags,
      instStart, instCount: instances.length - instStart, tris,
      bbox: [box.min.map((v) => (v === Infinity ? 0 : v)), box.max.map((v) => (v === -Infinity ? 0 : v))],
    });
  }

  // ---------------------------------------------------------------- Decor (world space)
  const level = new Level(layoutFor(MAP_LAYOUTS[stage], 'turf'), colliders);
  const decor = new Decor(new THREE.Scene(), level);
  decor.group.updateMatrixWorld(true);
  const DECOR_BUCKET = {
    'spawnPad:base': 'metal', 'lamps:body': 'metal', 'lamps:glass': 'glass',
    'palms:trunk': 'wood', 'palms:fronds': 'foliage', 'flags:poles': 'metal', 'flags:cloth': 'paint',
  };
  const I4 = new THREE.Matrix4();
  let decorMeshes = 0;
  const padBase = { baked: null };
  decor.group.traverse((o) => {
    if (!o.isMesh || !(o.name in DECOR_BUCKET)) return;   // pad face/barrier (ShaderMaterial) skipped
    const bk = DECOR_BUCKET[o.name];
    if (o.name === 'spawnPad:base') {
      if (!padBase.baked) padBase.baked = bakeParts([{ g: o.geometry, m: I4.clone(), c: WHITE, ao: null }], I4.clone());
      const tid = store.add(bk, padBase.baked.verts, padBase.baked.idx);
      const p = o.parent.position;
      decorInsts.push({ tid, bucketKey: bk, pos: [p.x, p.y, p.z], scale: 1 });
      decorMeshes++;
      return;
    }
    if (o.isInstancedMesh) {                              // flags:cloth — one bake per team instance
      for (let i = 0; i < o.count; i++) {
        const im = new THREE.Matrix4();
        o.getMatrixAt(i, im);
        im.premultiply(o.matrixWorld);
        const c = TEAM_COLORS[decor.flags[i].team];
        const baked = bakeParts([{ g: o.geometry, m: im, c, ao: null }], I4.clone());
        if (!baked) continue;
        decorInsts.push({ tid: store.add(bk, baked.verts, baked.idx), bucketKey: bk, pos: [0, 0, 0], scale: 1 });
      }
      decorMeshes++;
      return;
    }
    const baked = bakeParts([{ g: o.geometry, m: o.matrixWorld.clone(), c: WHITE, ao: null }], I4.clone());
    if (!baked) return;
    decorInsts.push({ tid: store.add(bk, baked.verts, baked.idx), bucketKey: bk, pos: [0, 0, 0], scale: 1 });
    decorMeshes++;
  });

  // uLamps table — setLevelLamps() formula (levelMaterial.js L28-36) baked final
  const lampPts = level.layout.decor?.lamps || [];
  const lamps = [...lampPts, ...lampPts.map(([x, z]) => [-x, -z])].slice(0, 12)
    .map(([x, z]) => [x + 0.92 * (x > 0 ? -1 : 1), Math.max(0, level.groundHeight(x, z)) + 4.9, z]);

  return { kit, stage, items, placements, instances, spins, blinks, decorInsts, store, lamps, counts: { spin: spins.length, blink: blinks.length, flags: nFlags, banners: nBanners, decorMeshes } };
}

// ---------------------------------------------------------------- bin writer (LE, props.schema.md)
function writeBin(b) {
  const bucketKeys = [...new Set([
    ...b.store.list.map((t) => t.bucketKey),
    ...b.instances.map((i) => i.bucketKey),
    ...b.spins.map((s) => s.bucketKey),
    ...b.decorInsts.map((d) => d.bucketKey),
  ])].sort();
  const bucketId = new Map(bucketKeys.map((k, i) => [k, i]));
  const chunks = [];
  const push = (buf) => chunks.push(buf);
  const u32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeUInt32LE(v); push(x); };
  const u16 = (v) => { const x = Buffer.allocUnsafe(2); x.writeUInt16LE(v); push(x); };
  const i16 = (v) => { const x = Buffer.allocUnsafe(2); x.writeInt16LE(v); push(x); };
  const i32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeInt32LE(Math.round(v)); push(x); };
  const f32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeFloatLE(v); push(x); };
  const u8 = (v) => push(Buffer.from([v]));

  u32(0x31504B49);            // 'IKP1'
  u16(1);                    // version
  u16(bucketKeys.length);
  for (const k of bucketKeys) {
    const [name, flag] = k.split('~');
    const nb = Buffer.from(name, 'utf8');
    u8(nb.length); push(nb); u8(flag === 'ns' ? 1 : 0);
  }
  u32(b.store.list.length);
  for (const t of b.store.list) {
    u32(t.verts.length); u32(t.idx.length);
    for (const v of t.verts) {
      i32(v[0]); i32(v[1]); i32(v[2]);
      i16(v[3]); i16(v[4]); i16(v[5]);
      u8(v[6]); u8(v[7]); u8(v[8]);
    }
    for (const j of t.idx) u32(j);
  }
  u32(b.instances.length);
  for (const i of b.instances) {
    u32(i.tid); u8(bucketId.get(i.bucketKey));
    i32(i.pos[0] * 1e3); i32(i.pos[1] * 1e3); i32(i.pos[2] * 1e3);
    f32(i.rotY); f32(i.scale); u8(i.modeFlags);
  }
  u32(b.spins.length);
  for (const s of b.spins) {
    u32(s.tid); u8(bucketId.get(s.bucketKey));
    for (const v of s.m) f32(v);
    f32(s.speed); f32(s.phase); u8(s.axis);
  }
  u32(b.blinks.length);
  for (const l of b.blinks) {
    i32(l.pos[0] * 1e3); i32(l.pos[1] * 1e3); i32(l.pos[2] * 1e3);
    f32(l.scale); u8(l.col[0]); u8(l.col[1]); u8(l.col[2]);
    f32(l.rate); f32(l.phase); f32(l.lo); f32(l.hi);
  }
  u32(b.decorInsts.length);
  for (const d of b.decorInsts) {
    u32(d.tid); u8(bucketId.get(d.bucketKey));
    i32(d.pos[0] * 1e3); i32(d.pos[1] * 1e3); i32(d.pos[2] * 1e3);
    f32(d.scale);
  }
  return { bin: Buffer.concat(chunks), buckets: bucketKeys.map((k) => ({ name: k.split('~')[0], ns: k.includes('~ns') ? 1 : 0 })) };
}

// ---------------------------------------------------------------- main
mkdirSync(outDir, { recursive: true });
let commit = 'unknown';
try {
  commit = execSync('git rev-parse HEAD', { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
} catch { /* keep 'unknown' */ }

for (const stage of (stageArg ? [stageArg] : STAGES)) {
  const b = extractStage(stage);
  const { bin, buckets } = writeBin(b);
  const gz = gzipSync(bin, { level: 9 });
  const binName = `${stage}.bin.gz`;
  writeFileSync(resolve(outDir, binName), gz);
  const tex = await createMuralTexture(stage);
  const murals = (tex.userData.murals || [])
    .map((m, id) => (m ? { id, rect: m.rect, place: m.place, fx: m.fx } : null))
    .filter(Boolean);
  let verts = 0, tris = 0;
  for (const t of b.store.list) { verts += t.verts.length; tris += t.idx.length / 3; }
  const manifest = {
    meta: {
      stage,
      placements: b.placements.length,
      templates: b.store.list.length,
      verts, tris,
      bin: binName, binBytes: gz.length, rawBytes: bin.length,
      binSha256: createHash('sha256').update(gz).digest('hex'),
      buckets,
    },
    placements: b.placements,
    lamps: b.lamps,
    murals,
    counts: b.counts,
    source: { commit },
  };
  writeFileSync(resolve(outDir, `${stage}.json`), stableStringify(manifest));
  console.log(`extract-props ${stage}: placements=${b.placements.length} templates=${b.store.list.length} verts=${verts} tris=${tris} instances=${b.instances.length} spin=${b.counts.spin} blink=${b.counts.blink} decor=${b.counts.decorMeshes} bin=${gz.length}B(gz)/${bin.length}B`);
}
