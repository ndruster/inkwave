// Task 53 (TR-53.1) props crosscheck: the baked per-stage manifests
// (rust/assets/props/<stage>.json + .bin.gz, consumed by the Rust props loader)
// are re-verified against the LIVE JS source of truth, independent of the
// extractor's bake path:
//
//  1. placement table  ≡ dressingFor(stage) entry-by-entry — type / pos / rotY /
//     scale exact, modeFlags recomputed from onlyIn/notIn AND cross-checked
//     against inMode() membership (variants.js L6);
//  2. per-placement tris recomputed from raw PropKit geometry (bucket snapshot
//     diff, blob skipped, spin templates via _tplGeo, blink = 84, cloth flag =
//     16 / banner = 80 static-pose tris — D-T53-6④) — no welding/quantisation
//     involved, so this catches bake-path regressions that change triangle
//     counts (empty-part skips, bucket filtering, cloth seg counts);
//  3. binary archive tie-out: magic/version, bucket table, template/vert/tri
//     meta, instance-table rows (pos mm / rotY / scale f32 / modeFlags) match
//     the placement they belong to, Σ instance+spin+blink tris per placement
//     equals the manifest count, section counts match manifest counts,
//     binSha256 matches the file on disk;
//  4. lamps table recomputed through the real Level.groundHeight (colliders
//     collected exactly as main.js L228-242 does for the turf build);
//  5. murals table ≡ createMuralTexture(stage).userData.murals (non-null rows).
//
// Exit 0 on success; every mismatch is printed with stage + placement index.
//   node rust/tools/props-crosscheck.mjs [stage]
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));          // rust/tools
const repoRoot = resolve(here, '../..');                       // inkwave repo root
const propsDir = resolve(here, '../assets/props');

const STAGES = ['tidewater', 'kelpline', 'halyard', 'saltpan', 'crossmarket', 'lockgate', 'terraces', 'cargo'];
const stageArg = process.argv[2] ?? null;
if (stageArg && !STAGES.includes(stageArg)) {
  console.error(`props-crosscheck: unknown stage "${stageArg}"`);
  process.exit(1);
}

// ---------------------------------------------------------------- DOM stub (same as extractor: canvas 2D no-op)
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
      return () => undefined;
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
const { createMuralTexture } = await import(resolve(repoRoot, 'src/world/murals.js'));

const errors = [];
const fail = (stage, what) => errors.push(`[${stage}] ${what}`);

// ---------------------------------------------------------------- bin reader (props.schema.md IKP1 v1)
function readBin(gz) {
  const b = gunzipSync(gz);
  let o = 0;
  const u8 = () => b[o++];
  const u16 = () => { const v = b.readUInt16LE(o); o += 2; return v; };
  const i16 = () => { const v = b.readInt16LE(o); o += 2; return v; };
  const u32 = () => { const v = b.readUInt32LE(o); o += 4; return v; };
  const i32 = () => { const v = b.readInt32LE(o); o += 4; return v; };
  const f32 = () => { const v = b.readFloatLE(o); o += 4; return v; };
  const magic = u32();
  if (magic !== 0x31504B49) throw new Error(`bad magic 0x${magic.toString(16)}`);
  const version = u16();
  if (version !== 1) throw new Error(`bad version ${version}`);
  const buckets = [];
  for (let i = 0, n = u16(); i < n; i++) {
    const len = u8();
    const name = b.subarray(o, o + len).toString('utf8'); o += len;
    buckets.push({ name, ns: u8() });
  }
  const templates = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const vc = u32(), ic = u32();
    o += vc * 21;                       // i32×3 (12B) + i16×3 (6B) + u8×3 (3B)
    o += ic * 4;                        // u32 indices
    templates.push({ verts: vc, tris: ic / 3 });
  }
  const instances = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const tid = u32(), bucket = u8();
    const pos = [i32(), i32(), i32()];
    const rotY = f32(), scale = f32(), modeFlags = u8();
    instances.push({ tid, bucket, pos, rotY, scale, modeFlags });
  }
  const spins = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const tid = u32(), bucket = u8();
    const m = []; for (let j = 0; j < 12; j++) m.push(f32());
    spins.push({ tid, bucket, m, speed: f32(), phase: f32(), axis: u8() });
  }
  const blinks = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const pos = [i32(), i32(), i32()];
    const scale = f32();
    const col = [u8(), u8(), u8()];
    blinks.push({ pos, scale, col, rate: f32(), phase: f32(), lo: f32(), hi: f32() });
  }
  const decors = [];
  for (let i = 0, n = u32(); i < n; i++) {
    const tid = u32(), bucket = u8();
    const pos = [i32(), i32(), i32()];
    decors.push({ tid, bucket, pos, scale: f32() });
  }
  if (o !== b.length) throw new Error(`trailing bytes: read ${o} of ${b.length}`);
  return { buckets, templates, instances, spins, blinks, decors };
}

// ---------------------------------------------------------------- helpers
const modeFlagsOf = (it) => (it.onlyIn === 'turf' ? 1 : it.onlyIn === 'zones' ? 2 : it.notIn === 'turf' ? 3 : it.notIn === 'zones' ? 4 : 0);
// flag → expected [turf, zones] membership (variants.js inMode semantics)
const FLAG_MEMBERSHIP = { 0: [true, true], 1: [true, false], 2: [false, true], 3: [false, true], 4: [true, false] };
const rawTris = (g) => {
  const P = g.attributes.position;
  if (!P || P.count === 0) return 0;
  return Math.floor((g.index ? g.index.count : P.count) / 3);
};
// cloth static-pose replicas (extract-props.mjs flagGeo/bannerGeo):
// PlaneGeometry(w,h,2,4) → 2×4×2 tris; PlaneGeometry(0.8,1.9,4,10) → 4×10×2.
const CLOTH_FLAG_TRIS = 2 * 4 * 2;
const CLOTH_BANNER_TRIS = 4 * 10 * 2;
const BLINK_TRIS = 84;                 // unit sphere used by blink bulbs (props.js)
const fround = (v) => Math.fround(v);

async function crosscheckStage(stage) {
  const manifest = JSON.parse(readFileSync(resolve(propsDir, `${stage}.json`), 'utf8'));
  const gz = readFileSync(resolve(propsDir, `${stage}.bin.gz`));
  const bin = readBin(gz);

  // 3a. binSha256 + meta vs bin tables
  const sha = createHash('sha256').update(gz).digest('hex');
  if (sha !== manifest.meta.binSha256) fail(stage, `binSha256 mismatch: file ${sha} vs manifest ${manifest.meta.binSha256}`);
  if (bin.buckets.length !== manifest.meta.buckets.length) fail(stage, `bucket count ${bin.buckets.length} vs manifest ${manifest.meta.buckets.length}`);
  else bin.buckets.forEach((bk, i) => {
    const m = manifest.meta.buckets[i];
    if (bk.name !== m.name || bk.ns !== m.ns) fail(stage, `bucket[${i}] ${JSON.stringify(bk)} vs manifest ${JSON.stringify(m)}`);
  });
  if (bin.templates.length !== manifest.meta.templates) fail(stage, `template count ${bin.templates.length} vs manifest ${manifest.meta.templates}`);
  let tv = 0, tt = 0;
  for (const t of bin.templates) { tv += t.verts; tt += t.tris; }
  if (tv !== manifest.meta.verts) fail(stage, `Σ template verts ${tv} vs manifest ${manifest.meta.verts}`);
  if (tt !== manifest.meta.tris) fail(stage, `Σ template tris ${tt} vs manifest ${manifest.meta.tris}`);
  if (bin.spins.length !== manifest.counts.spin) fail(stage, `spin count ${bin.spins.length} vs manifest ${manifest.counts.spin}`);
  if (bin.blinks.length !== manifest.counts.blink) fail(stage, `blink count ${bin.blinks.length} vs manifest ${manifest.counts.blink}`);

  // 1+2. live dressingFor + PropKit re-bake-free triangle count
  const kit = new PropKit(null, { headless: true, quality: 'high' });
  const items = dressingFor(stage);
  if (items.length !== manifest.placements.length || items.length !== manifest.meta.placements) {
    fail(stage, `placements length ${items.length} vs manifest ${manifest.placements.length}/${manifest.meta.placements}`);
    return;
  }
  const colliders = [];
  let instSeen = 0;
  for (let pi = 0; pi < items.length; pi++) {
    const it = items[pi];
    const pl = manifest.placements[pi];
    const pos = it.pos || [0, 0, 0], rotY = it.rotY || 0, scale = it.scale ?? 1;
    const flag = modeFlagsOf(it);
    if (pl.i !== pi) fail(stage, `placement[${pi}] i=${pl.i}`);
    if (pl.type !== it.type) fail(stage, `placement[${pi}] type ${pl.type} vs ${it.type}`);
    if (pl.pos.length !== pos.length || pl.pos.some((v, j) => v !== pos[j])) fail(stage, `placement[${pi}] pos ${JSON.stringify(pl.pos)} vs ${JSON.stringify(pos)}`);
    if (pl.rotY !== rotY) fail(stage, `placement[${pi}] rotY ${pl.rotY} vs ${rotY}`);
    if (pl.scale !== scale) fail(stage, `placement[${pi}] scale ${pl.scale} vs ${scale}`);
    if (pl.modeFlags !== flag) fail(stage, `placement[${pi}] modeFlags ${pl.modeFlags} vs recomputed ${flag}`);
    const [mt, mz] = FLAG_MEMBERSHIP[flag];
    if (inMode(it, 'turf') !== mt || inMode(it, 'zones') !== mz) fail(stage, `placement[${pi}] modeFlags ${flag} inconsistent with inMode(${inMode(it, 'turf')},${inMode(it, 'zones')})`);

    // live geometry diff (mirror of the extractor's add loop, raw tris only)
    const snap = new Map();
    for (const [k, a] of kit._buckets) snap.set(k, a.length);
    const s0 = kit._spin.length, b0 = kit._blink.length, f0 = kit._flags.length, n0 = kit._banners.length;
    const r = kit.add(it.type, it);
    if (r && r.colliders && inMode(it, 'turf')) colliders.push(...r.colliders);
    let tris = 0;
    for (const [k, a] of kit._buckets) {
      if (k.split('~')[0] === 'blob') continue;
      const from = snap.get(k) || 0;
      for (let j = from; j < a.length; j++) tris += rawTris(a[j].g);
    }
    for (const rec of kit._spin.slice(s0)) tris += rawTris(kit._tplGeo(rec.kind));
    const nBlink = kit._blink.length - b0;
    tris += nBlink * BLINK_TRIS;
    const nFlag = kit._flags.length - f0, nBanner = kit._banners.length - n0;
    tris += nFlag * CLOTH_FLAG_TRIS + nBanner * CLOTH_BANNER_TRIS;
    if (pl.tris !== tris) fail(stage, `placement[${pi}] (${it.type}) tris ${pl.tris} vs live ${tris}`);

    // 3b. instance-table tie-out for this placement's range
    if (pl.instStart !== instSeen) fail(stage, `placement[${pi}] instStart ${pl.instStart} vs running ${instSeen}`);
    instSeen += pl.instCount;
    let rangeTris = 0;
    for (let j = pl.instStart; j < pl.instStart + pl.instCount; j++) {
      const inst = bin.instances[j];
      if (!inst) { fail(stage, `placement[${pi}] instance[${j}] missing`); break; }
      if (inst.pos[0] !== Math.round(pos[0] * 1e3) || inst.pos[1] !== Math.round(pos[1] * 1e3) || inst.pos[2] !== Math.round(pos[2] * 1e3)) {
        fail(stage, `placement[${pi}] instance[${j}] pos mm ${inst.pos} vs ${JSON.stringify(pos)}`);
      }
      if (inst.rotY !== fround(rotY) || inst.scale !== fround(scale)) fail(stage, `placement[${pi}] instance[${j}] rotY/scale ${inst.rotY}/${inst.scale}`);
      if (inst.modeFlags !== flag) fail(stage, `placement[${pi}] instance[${j}] modeFlags ${inst.modeFlags} vs ${flag}`);
      rangeTris += bin.templates[inst.tid].tris;
    }
    // spin records of this placement carry world base matrices; verify their
    // template tris + the blink budget close the placement's tris exactly
    // (cloth rides the instance table, so rangeTris already includes it).
    const spinTris = (() => {
      let t = 0;
      for (const rec of kit._spin.slice(s0)) t += rawTris(kit._tplGeo(rec.kind));
      return t;
    })();
    const clothTris = nFlag * CLOTH_FLAG_TRIS + nBanner * CLOTH_BANNER_TRIS;
    if (rangeTris + spinTris + nBlink * BLINK_TRIS !== pl.tris) {
      fail(stage, `placement[${pi}] bin instance tris ${rangeTris} + spin ${spinTris} + blink ${nBlink * BLINK_TRIS} ≠ manifest tris ${pl.tris} (cloth ${clothTris} rides instances)`);
    }
  }
  if (instSeen !== bin.instances.length) fail(stage, `Σ instCount ${instSeen} vs bin instance table ${bin.instances.length}`);
  // global spin tris: bin spin section template tris must equal the live sum
  let binSpinTris = 0;
  for (const s of bin.spins) binSpinTris += bin.templates[s.tid].tris;
  let liveSpinTris = 0;
  for (const rec of kit._spin) liveSpinTris += rawTris(kit._tplGeo(rec.kind));
  if (binSpinTris !== liveSpinTris) fail(stage, `Σ spin template tris bin ${binSpinTris} vs live ${liveSpinTris}`);

  // 4. lamps table through the real Level
  const level = new Level(layoutFor(MAP_LAYOUTS[stage], 'turf'), colliders);
  const lampPts = level.layout.decor?.lamps || [];
  const lamps = [...lampPts, ...lampPts.map(([x, z]) => [-x, -z])].slice(0, 12)
    .map(([x, z]) => [x + 0.92 * (x > 0 ? -1 : 1), Math.max(0, level.groundHeight(x, z)) + 4.9, z]);
  if (lamps.length !== manifest.lamps.length) fail(stage, `lamps count ${lamps.length} vs manifest ${manifest.lamps.length}`);
  else lamps.forEach((l, i) => {
    const m = manifest.lamps[i];
    if (l.some((v, j) => Math.abs(v - m[j]) > 1e-9)) fail(stage, `lamps[${i}] ${JSON.stringify(l)} vs ${JSON.stringify(m)}`);
  });

  // 5. murals ≡ tex.userData.murals
  const tex = await createMuralTexture(stage);
  const murals = (tex.userData.murals || []).map((m, id) => (m ? id : null)).filter((v) => v !== null);
  if (murals.length !== manifest.murals.length) fail(stage, `murals count ${murals.length} vs manifest ${manifest.murals.length}`);
  else murals.forEach((id, i) => {
    const m = manifest.murals[i];
    if (m.id !== id) fail(stage, `murals[${i}] id ${m.id} vs live ${id}`);
  });

  console.log(`props-crosscheck ${stage}: placements=${items.length} instances=${bin.instances.length} tris(manifest Σ placements)=${manifest.placements.reduce((a, p) => a + p.tris, 0)} ok`);
}

for (const stage of (stageArg ? [stageArg] : STAGES)) await crosscheckStage(stage);

if (errors.length) {
  for (const e of errors) console.error('MISMATCH ' + e);
  console.error(`props-crosscheck: ${errors.length} mismatch(es)`);
  process.exit(1);
}
console.log('props-crosscheck: OK');
