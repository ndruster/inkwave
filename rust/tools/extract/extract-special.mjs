// Task 53 (D-T53-1) special-prop extractor: bakes the five boss/special world
// prop geometry sets (src/game/special-props.js getSpecialProp) into
//
//   rust/assets/props/special.bin.gz   (same 'IKP1' archive format, props.schema.md)
//   rust/assets/props/special.json     (manifest: kind → named part → template)
//
// Each kind's part groups (body / ink / glow / cone / legs[i].body / gun.body /
// cannon.ink / ballBody …) are already merged BufferGeometries with baked vertex
// colours (special-props.js Parts.build → mergeGeometries); they are welded +
// content-hash deduplicated into the template table with the same quantisation as
// extract-props.mjs. The instance table is empty: specials are placed by the
// runtime (Task 54 / 57 consumers). Bucket per part role mirrors propMesh()
// (specials.js L202-208): body → 'gloss' (plastic), ink → 'paint' (team-tinted at
// runtime), glow → 'glow'. Anchor points / dimensions ride the manifest.
//
// Deterministic (TR-3.4); the JS tree is never modified.
//   node extract-special.mjs
import { mkdirSync, writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { upstreamCommit } from './upstream-commit.mjs';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '../../..');
const outDir = resolve(here, '../../assets/props');

globalThis.document = {
  createElement: (tag) => {
    if (tag !== 'canvas') return {};
    const cv = { width: 0, height: 0 };
    cv.getContext = () => new Proxy({ canvas: cv }, {
      get: (t, k) => (k in t ? t[k]
        : k === 'measureText' ? () => ({ width: 0 })
          : k === 'createLinearGradient' || k === 'createRadialGradient' ? () => ({ addColorStop() {} })
            : () => undefined),
      set: (t, k, v) => { t[k] = v; return true; },
    });
    return cv;
  },
  fonts: { load: () => Promise.resolve() },
};

const THREE = await import(resolve(repoRoot, 'node_modules/three/build/three.module.js'));
const { SPECIAL_PROP_KINDS, getSpecialProp } = await import(resolve(repoRoot, 'src/game/special-props.js'));

const clamp8 = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));

// weld + quantise one merged geometry (identity transform, colours already baked)
function bakeGeo(g) {
  const P = g.attributes.position, N = g.attributes.normal, C = g.attributes.color;
  const recs = new Map(), verts = [], idx = [];
  const pushV = (i) => {
    const q = [
      Math.round(P.getX(i) * 1e3), Math.round(P.getY(i) * 1e3), Math.round(P.getZ(i) * 1e3),
      N ? Math.round(N.getX(i) * 1e3) : 0, N ? Math.round(N.getY(i) * 1e3) : 1000, N ? Math.round(N.getZ(i) * 1e3) : 0,
      C ? clamp8(C.getX(i)) : 255, C ? clamp8(C.getY(i)) : 255, C ? clamp8(C.getZ(i)) : 255,
    ];
    const key = q.join(',');
    let j = recs.get(key);
    if (j === undefined) { j = verts.length; recs.set(key, j); verts.push(q); }
    return j;
  };
  if (g.index) {
    const I = g.index.array;
    for (let i = 0; i < I.length; i += 3) idx.push(pushV(I[i]), pushV(I[i + 1]), pushV(I[i + 2]));
  } else {
    for (let i = 0; i < P.count; i += 3) idx.push(pushV(i), pushV(i + 1), pushV(i + 2));
  }
  return { verts, idx };
}

const bucketOf = (name) => (/ink/i.test(name) ? 'paint' : /glow/i.test(name) ? 'glow' : 'gloss');
const vec = (v) => (v && v.isVector3 ? [v.x, v.y, v.z] : undefined);

const store = new Map();   // hash -> {tid, bucketKey, verts, idx}
const list = [];
function addTemplate(bucketKey, verts, idx) {
  const h = createHash('sha256').update(bucketKey + '|' + verts.map((v) => v.join(',')).join(';') + '|' + idx.join(',')).digest('hex');
  let id = store.get(h);
  if (id === undefined) { id = list.length; store.set(h, id); list.push({ bucketKey, verts, idx }); }
  return id;
}

const kinds = {};
for (const kind of SPECIAL_PROP_KINDS) {
  const d = getSpecialProp(kind);
  const parts = [];
  const extras = {};
  const collect = (name, g) => {
    const bk = bucketOf(name);
    const { verts, idx } = bakeGeo(g);
    const tid = addTemplate(bk, verts, idx);
    parts.push({ name, bucket: bk, tid, verts: verts.length, tris: idx.length / 3 });
  };
  for (const [k, v] of Object.entries(d)) {
    if (v && v.isBufferGeometry) collect(k, v);
  }
  if (d.legs) d.legs.forEach((l, i) => {
    if (l.body) collect(`legs${i}.body`, l.body);
    if (l.ink) collect(`legs${i}.ink`, l.ink);
    extras[`legs${i}`] = { at: vec(l.at), side: l.side, index: l.index };
  });
  for (const grp of ['gun', 'cannon']) {
    if (!d[grp]) continue;
    if (d[grp].body) collect(`${grp}.body`, d[grp].body);
    if (d[grp].ink) collect(`${grp}.ink`, d[grp].ink);
    extras[grp] = { at: vec(d[grp].at) };
  }
  // anchors + dimensions (everything non-geometry the consumers place by)
  for (const [k, v] of Object.entries(d)) {
    if (v && v.isBufferGeometry) continue;
    const w = vec(v);
    if (w) { extras[k] = w; continue; }
    if (Array.isArray(v) && v.every((p) => p && p.isVector3)) { extras[k] = v.map(vec); continue; }
    if (Array.isArray(v) && v.every((p) => p && p.isVector3 === undefined && typeof p === 'object')) continue; // legs handled above
    if (typeof v === 'number' || typeof v === 'string') extras[k] = v;
  }
  kinds[kind] = { parts, extras };
}

// ---------------------------------------------------------------- bin (same IKP1 writer, empty tables)
const bucketKeys = [...new Set(list.map((t) => t.bucketKey))].sort();
const bucketId = new Map(bucketKeys.map((k, i) => [k, i]));
const chunks = [];
const push = (buf) => chunks.push(buf);
const u32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeUInt32LE(v); push(x); };
const u16 = (v) => { const x = Buffer.allocUnsafe(2); x.writeUInt16LE(v); push(x); };
const i16 = (v) => { const x = Buffer.allocUnsafe(2); x.writeInt16LE(v); push(x); };
const i32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeInt32LE(Math.round(v)); push(x); };
const f32 = (v) => { const x = Buffer.allocUnsafe(4); x.writeFloatLE(v); push(x); };
const u8 = (v) => push(Buffer.from([v]));

u32(0x31504B49); u16(1); u16(bucketKeys.length);
for (const k of bucketKeys) { const nb = Buffer.from(k, 'utf8'); u8(nb.length); push(nb); u8(0); }
u32(list.length);
for (const t of list) {
  u32(t.verts.length); u32(t.idx.length);
  for (const v of t.verts) { i32(v[0]); i32(v[1]); i32(v[2]); i16(v[3]); i16(v[4]); i16(v[5]); u8(v[6]); u8(v[7]); u8(v[8]); }
  for (const j of t.idx) u32(j);
}
u32(0);   // instances
u32(0);   // spins
u32(0);   // blinks
u32(0);   // decor instances
const bin = Buffer.concat(chunks);
const gz = gzipSync(bin, { level: 9 });

mkdirSync(outDir, { recursive: true });
writeFileSync(resolve(outDir, 'special.bin.gz'), gz);
const commit = upstreamCommit(repoRoot, [
  'src/game/special-props.js',
  'src/config.js',
]);
let verts = 0, tris = 0;
for (const t of list) { verts += t.verts.length; tris += t.idx.length / 3; }
const manifest = {
  meta: {
    bins: 'special.bin.gz', binBytes: gz.length, rawBytes: bin.length,
    binSha256: createHash('sha256').update(gz).digest('hex'),
    templates: list.length, verts, tris,
    buckets: bucketKeys.map((k) => ({ name: k, ns: 0 })),
  },
  kinds,
  source: { commit },
};
writeFileSync(resolve(outDir, 'special.json'), stableStringify(manifest));
console.log(`extract-special: kinds=${SPECIAL_PROP_KINDS.length} templates=${list.length} verts=${verts} tris=${tris} bin=${gz.length}B(gz)/${bin.length}B`);
