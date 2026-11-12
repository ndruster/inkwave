// Task 54 batch A (D-T54-1): character asset extraction — full rig + geometry + pose tables.
// Real-module harness (per extract-props.mjs): node imports the actual game modules
// (src/game/character.js + character-geo.js + character-weapons.js + character-lod.js +
// character-style.js), instantiates a real Character with the canonical rookie style, walks the
// scene graph, and quantizes + welds + content-hash-dedups every part geometry into a template
// store (口径 ≡ extract-props: pos i32 mm, normal i16×1e3, color u8, 1e-3 weld grid). Instances
// are {bone, bone-local 3x4 matrix}. Pose tables are sliced verbatim from character.js source
// (literal blocks + assertContains, no hand-typed values) into char-poses.json.
//
// Outputs (deterministic; run twice -> byte-identical):
//   rust/assets/char/char.bin.gz     template + instance store (schema: rust/assets/char/char.schema.md)
//   rust/assets/char/char.json       manifest: rig tables, per-part meta, weapon frames, counts, sha256
//   rust/assets/char/char-poses.json pose/hold/fidget/expression/style tables sliced from JS source
//
// Documented deviations from D-T54-1:
//  - spring hz·ζ literals stay at their call sites (batch B ports the formulas from source directly).
//  - outfit-coupled parts are NOT extracted; canonical style = rookie preset (outfit 0 base tee).
//  - skinned kid/hair/eyes geometry is emitted in kid space, rigid-mounted at the '@kid' pseudo
//    bone (identity matrix); batch C decides the re-rig strategy.
//  - weapon "子帧" = static frame hierarchy (pivot/off/part pivots/drum/spin/muzzle/handR/handL);
//    dynamic pump/trig motion stays in animateWeapon (batch B/C).

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { upstreamCommit } from './upstream-commit.mjs';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { stableStringify } from './stable-json.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');
const OUT_DIR = resolve(repoRoot, 'rust/assets/char');
const CHAR_JS = resolve(repoRoot, 'src/game/character.js');
const STYLE_JS = resolve(repoRoot, 'src/game/character-style.js');

// ---- headless stubs (defensive; the character chain is DOM-free but ctx.js may be extended) ----
globalThis.document = globalThis.document || {
  createElement: (t) => (t === 'canvas'
    ? { width: 0, height: 0, getContext: () => new Proxy({}, { get: () => () => ({ addColorStop() {} }) }) }
    : {}),
};
globalThis.innerWidth = globalThis.innerWidth || 1920;
globalThis.innerHeight = globalThis.innerHeight || 1080;

// ---- real module imports (no mocks) ----
const THREE = await import(resolve(repoRoot, 'node_modules/three/build/three.module.js'));
const geo = await import(resolve(repoRoot, 'src/game/character-geo.js'));
const W = await import(resolve(repoRoot, 'src/game/character-weapons.js'));
const LOD = await import(resolve(repoRoot, 'src/game/character-lod.js'));
const STYLE = await import(resolve(repoRoot, 'src/game/character-style.js'));
const C = await import(resolve(repoRoot, 'src/game/character.js'));

// ---- literal-block slicing + eval (extract-tuning.mjs discipline: markers pinned, no hand copy) ----
const srcChar = readFileSync(CHAR_JS, 'utf8');
const srcStyle = readFileSync(STYLE_JS, 'utf8');

function sliceBlock(src, startMarker, endMarker, label) {
  const i = src.indexOf(startMarker);
  if (i < 0) throw new Error(`marker missing (start): ${label} :: ${startMarker}`);
  const j = src.indexOf(endMarker, i);
  if (j < 0) throw new Error(`marker missing (end): ${label} :: ${endMarker}`);
  return src.slice(i, j + endMarker.length);
}
function assertContains(src, needle, label) {
  if (!src.includes(needle)) throw new Error(`assert failed: ${label} :: ${needle}`);
}

// pose channel bank (character.js L175-189): pure S() index arithmetic -> real channel numbers
const bankBlock = sliceBlock(srcChar, 'let _k = 0;', 'const PN = _k;', 'channel bank');
const CHANNEL_NAMES = [...bankBlock.matchAll(/(?:const|,)\s*([A-Z_0-9]+) = S[,(]/g)].map((m) => m[1]);
const channels = new Function(`${bankBlock}\nreturn {${CHANNEL_NAMES.join(',')}, PN};`)();
if (CHANNEL_NAMES.length !== 66 || channels.PN !== 126) throw new Error(`unexpected channel layout: PN=${channels.PN} names=${CHANNEL_NAMES.length}`);

// rig constants (L166-170) — STANCE_IDLE depends on HIPW
const rigConstBlock = sliceBlock(srcChar, 'const ANKLE_H = REST.footL.y;', 'new THREE.Vector3(0, 0.164, 0.014);', 'rig constants');
const rigConst = new Function('REST', 'THREE', `${rigConstBlock}\nreturn { ANKLE_H, HIPW, BALL_Z, HEEL_Z, HEAD_CTR: [HEAD_CTR.x, HEAD_CTR.y, HEAD_CTR.z] };`)(geo.REST, THREE);

// helpers: slice a `NAME = { ... };` block (ends at the next line-initial '};') or a whole line
function sliceAssign(src, marker, label) {
  const i = src.indexOf(marker);
  if (i < 0) throw new Error(`marker missing (start): ${label} :: ${marker}`);
  const j = src.indexOf('\n};', i);
  if (j < 0) throw new Error(`marker missing (end): ${label}`);
  return src.slice(i, j + 3);
}
function sliceToEol(src, marker, label) {
  const i = src.indexOf(marker);
  if (i < 0) throw new Error(`marker missing: ${label} :: ${marker}`);
  const j = src.indexOf('\n', i);
  if (j < 0) throw new Error(`no EOL after: ${label}`);
  return src.slice(i, j);
}

// HOLD full-kind table (base L233-276 + per-kind extension blocks L279-477 + the fire line),
// ANIM_OF/NO_TWIRL (L437-441), HOLD_HERO (L497), AIM_PIVOT (L232), BOW_CANT (L478).
assertContains(srcChar, "carry: { p: [-0.19, 0.76, 0.14], r: [0.62, 0.12, -0.16] }", 'HOLD.shooter.carry');
assertContains(srcChar, "aim: { p: [-0.04, -0.075, 0.27], r: [0, 0.035, 0] }", 'HOLD.shooter.aim');
assertContains(srcChar, "lock: { p: [-0.07, -0.05, 0.31], r: [0, 0.02, -0.06] }", 'HOLD.dualies.lock');
const holdBase = sliceAssign(srcChar, 'const HOLD = {', 'HOLD base');
const holdExt = ['HOLD.dualies = {', 'HOLD.slosher = {', 'HOLD.splatling = {', 'HOLD.bucket = {', 'HOLD.spinner = {', 'HOLD.sp_zooka = {', 'HOLD.blade = {', 'HOLD.mitts = {', 'HOLD.bow = {'].map((m) => sliceAssign(srcChar, m, m));
const fireLine = sliceToEol(srcChar, "HOLD.shooter.fire = 'recoil';", 'fire line');
const animOfLine = sliceToEol(srcChar, 'const ANIM_OF = {', 'ANIM_OF');
const noTwirlLine = sliceToEol(srcChar, 'const NO_TWIRL = {', 'NO_TWIRL');
const brollyLine = sliceToEol(srcChar, "ANIM_OF.brolly = 'shooter';", 'ANIM_OF.brolly');
const mittsTwirlLine = sliceToEol(srcChar, 'NO_TWIRL.mitts = true;', 'NO_TWIRL.mitts');
const bowAnimLine = sliceToEol(srcChar, "ANIM_OF.bow = 'charger';", 'ANIM_OF.bow');
const aimPivotLine = sliceToEol(srcChar, 'const AIM_PIVOT = new THREE.Vector3(', 'AIM_PIVOT');
const holdHeroLine = sliceToEol(srcChar, 'const HOLD_HERO = {', 'HOLD_HERO');
const bowCantLine = sliceToEol(srcChar, 'const BOW_CANT = -0.85;', 'BOW_CANT');
const holdEval = new Function('THREE', `
${holdBase}
${holdExt.slice(0, 3).join('\n')}
${fireLine}
${holdExt.slice(3).join('\n')}
${animOfLine}
${noTwirlLine}
${brollyLine}
${mittsTwirlLine}
${bowAnimLine}
${aimPivotLine}
${holdHeroLine}
${bowCantLine}
return { HOLD, ANIM_OF, NO_TWIRL, AIM_PIVOT, HOLD_HERO, BOW_CANT };
`)(THREE);
for (const k of ['shooter', 'blaster', 'charger', 'roller', 'dualies', 'slosher', 'splatling', 'bucket', 'spinner', 'sp_zooka', 'blade', 'mitts', 'bow']) {
  if (!holdEval.HOLD[k]) throw new Error(`HOLD missing kind: ${k}`);
}

// stance / shuffle / XCH / fidgets / emote+dance keys / slosh keys / misc (L364-505)
const stanceBlock = sliceBlock(srcChar, 'const STANCE_IDLE', '[0.03, -0.02, 0.45]];', 'stance+shuffle');
const xchLine = sliceToEol(srcChar, 'const XCH = [BROW, BROWY, EYE, MCURVE, MWIDTH, MOPEN, MTILT, SQUINT];', 'XCH');
const fidgetsLine = sliceToEol(srcChar, "const FIDGETS = ['goggles'", 'FIDGETS');
const fidgetLenLine = sliceToEol(srcChar, 'const FIDGET_LEN = [', 'FIDGET_LEN');
const fidgetKBlock = sliceBlock(srcChar, 'const K_GOG_T = [', 'K_MENU_V = [0, 0, 0.5, 0.5, -0.4, -0.4, 0.05, 0];', 'fidget K tables');
const emDvBlock = sliceBlock(srcChar, 'const K_EM_ST = [', 'K_DV_SX = [1.5, 1.2, 0.83, 1.07, 0.98, 1];', 'emote/dance keys');
const kSlBlock = sliceBlock(srcChar, 'const K_SL_T = [', 'K_SL_W = [0, 0.32, -0.08, -0.14, 0];', 'slosh keys');
const fingerLine = sliceToEol(srcChar, 'const FINGER_SPREAD = [', 'FINGER_SPREAD');
const danceLine = sliceToEol(srcChar, 'const DANCE_VARIANTS = { victory: 3', 'DANCE_VARIANTS');
assertContains(srcChar, 'const STANCE_LOCK = [0.165, 0.035, 0.42, -0.165, -0.035, -0.42];', 'STANCE_LOCK');
const keyNames = [...(fidgetKBlock + emDvBlock + kSlBlock).matchAll(/(?:const|,)\s*([A-Z_0-9]+) = \[/g)].map((m) => m[1]);
const stanceEval = new Function('HIPW', 'CHAN', `
const {${CHANNEL_NAMES.join(',')}} = CHAN;
${stanceBlock}
${xchLine}
${fidgetsLine}
${fidgetLenLine}
${fidgetKBlock}
${emDvBlock}
${kSlBlock}
${fingerLine}
${danceLine}
return { STANCE_IDLE, STANCE_LOCK, SHUFFLES, XCH, FIDGETS, FIDGET_LEN, ${keyNames.join(', ')}, FINGER_SPREAD, DANCE_VARIANTS };
`)(rigConst.HIPW, channels);

// blink curve: function source verbatim + the constructor's default blink params (L398-404, L605)
const blinkSrc = sliceBlock(srcChar, 'function blinkCurve(b, t) {', '\n  return 0;\n}', 'blinkCurve');
const blDefaults = srcChar.match(/this\.bl = \{ t: -1, c: 0\.08, h: 0\.03, o: 0\.16[^;]*;/);
if (!blDefaults) throw new Error('blink defaults missing');

// expression tables (L371-392): X_* / E_* deltas written through the XCH channels
const exprBlock = sliceBlock(srcChar, 'const X_FOCUS = [', 'E_BLUSH = [0, 0, 0.45];', 'expression tables');
const exprNames = [...exprBlock.matchAll(/(?:const|,)\s*([A-Z_0-9]+) = \[/g)].map((m) => m[1]);
const expressions = new Function(`${exprBlock}\nreturn {${exprNames.join(',')}};`)();

// style catalog (character-style.js): literal blocks sliced + eval'd, cross-checked vs real exports
assertContains(srcStyle, 'export const HAIR_STYLES = HAIR_STYLE_NAMES.length;', 'HAIR_STYLES');
const decl = (s) => s.replace(/^export /, '');
const styleEval = new Function(`
${decl(sliceBlock(srcStyle, 'export const SKIN_TONES = [', '\n];', 'SKIN_TONES'))}
${decl(sliceBlock(srcStyle, 'export const OUTFITS = [', '\n];', 'OUTFITS'))}
${decl(sliceBlock(srcStyle, 'export const IRIS = [', '\n];', 'IRIS'))}
${decl(sliceToEol(srcStyle, 'export const SKIN_NAMES = [', 'SKIN_NAMES'))}
${decl(sliceToEol(srcStyle, 'export const OUTFIT_NAMES = [', 'OUTFIT_NAMES'))}
${decl(sliceToEol(srcStyle, 'export const IRIS_NAMES = [', 'IRIS_NAMES'))}
${decl(sliceToEol(srcStyle, 'export const HAIR_STYLE_NAMES = [', 'HAIR_STYLE_NAMES'))}
return { SKIN_TONES, OUTFITS, IRIS, SKIN_NAMES, OUTFIT_NAMES, IRIS_NAMES, HAIR_STYLE_NAMES };
`)(THREE);
// anti-drift: sliced tables must equal the real exported values (JSON-compare; no hand copy)
for (const n of ['SKIN_TONES', 'OUTFITS', 'IRIS', 'SKIN_NAMES', 'OUTFIT_NAMES', 'IRIS_NAMES', 'HAIR_STYLE_NAMES']) {
  if (stableStringify(styleEval[n]) !== stableStringify(STYLE[n])) throw new Error(`style drift: ${n}`);
}

// ---- canonical character instance (rookie preset style) ----
const STYLE0 = { hair: 0, skin: 0, outfit: 0, eyes: 0, hat: 0, brows: 0 };
const ch = new C.Character({ name: 'Extract', color: '#ff8a14', weapon: 'shooter', style: STYLE0 });
ch.root.updateMatrixWorld(true);

// ---- template store (quantization 口径 ≡ extract-props) ----
const Q_POS = 1000, Q_NOR = 1000;
class TemplateStore {
  constructor() { this.templates = []; this.map = new Map(); this.instances = []; this.totalVerts = 0; this.totalTris = 0; }
  add(g) {
    const pos = g.attributes.position.array;
    const nrm = g.attributes.normal ? g.attributes.normal.array : null;
    const col = g.attributes.color ? g.attributes.color.array : null;
    const n = g.attributes.position.count;
    const idx = g.index ? g.index.array : null;
    const weld = new Map(), remap = new Array(n), tq = new Array(n);
    for (let i = 0; i < n; i++) {
      tq[i] = [
        Math.round(pos[i * 3] * Q_POS), Math.round(pos[i * 3 + 1] * Q_POS), Math.round(pos[i * 3 + 2] * Q_POS),
        nrm ? Math.round(nrm[i * 3] * Q_NOR) : 0, nrm ? Math.round(nrm[i * 3 + 1] * Q_NOR) : 1000, nrm ? Math.round(nrm[i * 3 + 2] * Q_NOR) : 0,
        col ? Math.round(Math.min(1, Math.max(0, col[i * 3])) * 255) : 255,
        col ? Math.round(Math.min(1, Math.max(0, col[i * 3 + 1])) * 255) : 255,
        col ? Math.round(Math.min(1, Math.max(0, col[i * 3 + 2])) * 255) : 255,
      ];
      const key = tq[i].join(',');
      let w = weld.get(key);
      if (w === undefined) { w = weld.size; weld.set(key, w); }
      remap[i] = w;
    }
    const m = idx ? idx.length : n;
    const outIdx = new Array(m);
    for (let i = 0; i < m; i++) outIdx[i] = remap[idx ? idx[i] : i];
    const hash = createHash('sha256');
    for (const k of weld.keys()) hash.update(k + '|');
    for (const i of outIdx) hash.update(String(i) + ',');
    const hk = hash.digest('hex');
    let id = this.map.get(hk);
    if (id === undefined) {
      id = this.templates.length;
      this.map.set(hk, id);
      const verts = [...weld.keys()].map((k) => k.split(',').map(Number));
      this.templates.push({ verts, idx: outIdx });
      this.totalVerts += verts.length;
      this.totalTris += Math.floor(outIdx.length / 3);
    }
    return id;
  }
  buildBin() {
    const chunks = [];
    const header = Buffer.alloc(16);
    header.writeUInt32LE(0x31434B49, 0); // 'IKC1' as LE bytes
    header.writeUInt32LE(1, 4);          // version
    header.writeUInt32LE(this.templates.length, 8);
    header.writeUInt32LE(this.instances.length, 12);
    chunks.push(header);
    for (const t of this.templates) {
      const n = t.verts.length, ti = t.idx.length;
      const buf = Buffer.alloc(12 + n * 12 + n * 6 + n * 3 + ti * 4);
      let o = 0;
      buf.writeInt32LE(n, o); o += 4;
      buf.writeInt32LE(ti, o); o += 4;
      buf.writeInt32LE(ti / 3, o); o += 4;
      for (const v of t.verts) { buf.writeInt32LE(v[0], o); buf.writeInt32LE(v[1], o + 4); buf.writeInt32LE(v[2], o + 8); o += 12; }
      for (const v of t.verts) { buf.writeInt16LE(v[3], o); buf.writeInt16LE(v[4], o + 2); buf.writeInt16LE(v[5], o + 4); o += 6; }
      for (const v of t.verts) { buf.writeUInt8(v[6], o); buf.writeUInt8(v[7], o + 1); buf.writeUInt8(v[8], o + 2); o += 3; }
      for (const i of t.idx) { buf.writeUInt32LE(i, o); o += 4; }
      chunks.push(buf);
    }
    for (const ins of this.instances) {
      const buf = Buffer.alloc(4 + 2 + 4 + 48);
      let o = 0;
      buf.writeUInt32LE(ins.template, o); o += 4;
      buf.writeUInt16LE(ins.boneId, o); o += 2;
      buf.writeUInt32LE(ins.partId, o); o += 4;
      for (const f of ins.matrix) { buf.writeFloatLE(f, o); o += 4; }
      chunks.push(buf);
    }
    return Buffer.concat(chunks);
  }
}
const store = new TemplateStore();

// ---- bone table (real bones + '@kid'/'@model' pseudo mounts) ----
const BONES = [...geo.BONE_NAMES, '@kid', '@model'];
const boneIdx = new Map(BONES.map((n, i) => [n, i]));

function relMatrix(obj, ref) {
  obj.updateWorldMatrix(true, false);
  ref.updateWorldMatrix(true, false);
  const inv = new THREE.Matrix4().copy(ref.matrixWorld).invert();
  const m = new THREE.Matrix4().multiplyMatrices(inv, obj.matrixWorld);
  const e = m.elements;
  return [e[0], e[4], e[8], e[12], e[1], e[5], e[9], e[13], e[2], e[6], e[10], e[14]];
}
const v3 = (v) => [v.x, v.y, v.z];
const q4 = (q) => [q.x, q.y, q.z, q.w];
// JSON-serialize any object/array, flattening THREE.Vector3 values to [x, y, z]
const ser = (o) => JSON.parse(JSON.stringify(o, (k, v) => (v && v.isVector3) ? [v.x, v.y, v.z] : v));

// ---- part registry ----
const partMeta = new Map();
function addPart(key, geometry, boneName, matrix, { tier, matClass, shadow = true, extra = null }) {
  if (partMeta.has(key)) throw new Error(`duplicate part key: ${key}`);
  const template = store.add(geometry);
  const t = store.templates[template];
  store.instances.push({ template, boneId: boneIdx.get(boneName), partId: 0, matrix });
  partMeta.set(key, { key, id: store.instances.length - 1, tier, bone: boneName, matClass, shadow, template, verts: t.verts.length, tris: t.idx.length / 3, matrix, extra });
}

const TIER_NAMES = ['hero', 'game', 'far'];
const IDENT = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0];

// ---- 1) kid parts, 3 tiers via the real _tierParts path; hair iterated per style ----
for (let t = 0; t < 3; t++) {
  for (let h = 0; h < geo.HAIR_STYLE_NAMES.length; h++) {
    ch.style = { ...STYLE0, hair: h };
    const parts = ch._tierParts(t);
    const hp = parts.find((p) => p.key === 'hair');
    addPart(`hair${h}:${TIER_NAMES[t]}`, hp.geo, '@kid', IDENT, { tier: TIER_NAMES[t], matClass: 'hair', shadow: hp.shadow });
    if (h === 0) {
      for (const p of parts) {
        if (p.key === 'hair') continue;
        addPart(`${p.key}:${TIER_NAMES[t]}`, p.geo, '@kid', IDENT, { tier: TIER_NAMES[t], matClass: p.mat, shadow: p.shadow });
      }
    }
  }
}
ch.style = { ...STYLE0 };

// ---- 2) tank (glass + liquid surface), 3 tiers; mounted at the 'tank' bone ----
const tankBone = ch.bones.tank;
const tankMatGlass = relMatrix(ch.tank.glass, tankBone);
const tankMatFill = relMatrix(ch.tank.fill, tankBone);
for (let t = 0; t < 3; t++) {
  const T = geo.getKidShared(TIER_NAMES[t]).tank;
  addPart(`tankGlass:${TIER_NAMES[t]}`, T.glass, 'tank', tankMatGlass, { tier: TIER_NAMES[t], matClass: 'glass', shadow: false });
  addPart(`tankFill:${TIER_NAMES[t]}`, T.fill, 'tank', tankMatFill, { tier: TIER_NAMES[t], matClass: 'fill', shadow: false, extra: { fillBottom: T.fillBottom, fillHeight: T.fillHeight, tilt: T.tilt, offset: v3(T.offset), center: v3(T.center), offsetTank: v3(T.offsetTank) } });
}

// ---- 3) squid parts, 3 tiers; mounted at '@model' (rigid, not skinned) ----
const gameSquid = geo.getKidShared('game').squid;
const farSquid = geo.getKidShared('far').squid;
for (const [slot, mesh] of [['body', ch.squid.body], ['dark', ch.squid.dark], ['eyes', ch.squid.eyes]]) {
  const m = relMatrix(mesh, ch.model);
  for (let t = 0; t < 3; t++) {
    const S = t === 0 ? geo.getKidShared('hero').squid : t === 1 ? gameSquid : farSquid;
    let g = S[slot];
    if (t === 2 && g === gameSquid[slot]) g = LOD.farGeometry(g, 1500); // mirrors _tierProps far rule
    addPart(`squid${slot[0].toUpperCase()}${slot.slice(1)}:${TIER_NAMES[t]}`, g, '@model', m, { tier: TIER_NAMES[t], matClass: slot === 'body' ? 'squid' : slot === 'dark' ? 'dark' : 'eye', shadow: slot !== 'eyes' });
  }
}

// ---- 4) bomb (idle sub prop), mounted at 'handL' via the real bomb group ----
const bombDef = W.getSubDef('bomb');
for (const mesh of ch.bomb.group.children) {
  if (!mesh.isMesh) continue;
  const slot = mesh.geometry === bombDef.body ? 'body' : 'ink';
  addPart(`bomb:${slot}`, mesh.geometry, 'handL', relMatrix(mesh, ch.bones.handL), { tier: 'near', matClass: slot === 'ink' ? 'ink' : 'plastic', shadow: true, extra: { inHandL: { pos: v3(bombDef.inHandL.pos), quat: q4(bombDef.inHandL.quat) }, grip: bombDef.grip ? { pos: v3(bombDef.grip.pos), handZ: v3(bombDef.grip.handZ), handY: v3(bombDef.grip.handY) } : null } });
}

// ---- 5) weapons, per kind: geometry + static frame hierarchy (pivot/off/part pivots/drum/spin/muzzle/hands) ----
// kit weapons (B2 scenario): mitts and bow self-register on import; blade and
// brolly register manually (kits/blade.js / kits/brolly.js themselves pull in
// bots/ui/physics — too heavy here; the -model.js modules are standalone).
await import(resolve(repoRoot, 'src/game/kits/mitts-model.js'));
await import(resolve(repoRoot, 'src/game/kits/bow-model.js'));
const bladeModel = await import(resolve(repoRoot, 'src/game/kits/blade-model.js'));
W.registerWeaponModel('blade', bladeModel.buildBlade, bladeModel.animateBlade);
const brollyModel = await import(resolve(repoRoot, 'src/game/kits/brolly-model.js'));
W.registerWeaponModel('brolly', brollyModel.brollyDef, brollyModel.animateBrolly);
const KINDS = [...W.WEAPON_KINDS, ...W.SPECIAL_WEAPON_KINDS];
const weapons = {};
for (const kind of KINDS) {
  ch.setWeapon(kind);
  ch.root.updateMatrixWorld(true);
  const w = ch.weapons[kind];
  const d = w.def;
  const handR = ch.bones.handR, handL = ch.bones.handL;
  const rec = { frames: {}, parts: {}, instances: [] };
  rec.frames.pivot = v3(w.pivot.position);
  rec.frames.off = { pos: v3(w.off.position), quat: q4(w.off.quaternion) };
  rec.frames.inHand = { pos: v3(d.inHand.pos), quat: q4(d.inHand.quat) };
  rec.frames.gripR = ser(d.gripR);
  rec.frames.gripL = ser(d.gripL);
  rec.frames.handR = { pos: v3(d.handR.pos), quat: q4(d.handR.quat) };
  rec.frames.handL = { pos: v3(d.handL.pos), quat: q4(d.handL.quat) };
  if (d.inHandL) rec.frames.inHandL = { pos: v3(d.inHandL.pos), quat: q4(d.inHandL.quat) };
  if (d.muzzle) rec.frames.muzzle = v3(d.muzzle);
  if (d.twirl) rec.frames.twirl = v3(d.twirl);
  if (d.drumAt) rec.frames.drumAt = v3(d.drumAt);
  if (d.spinAt) rec.frames.spinAt = v3(d.spinAt);
  if (d.drumR !== undefined) rec.frames.drumR = d.drumR;
  if (d.drawLen !== undefined) rec.frames.drawLen = d.drawLen;
  rec.frames.dual = !!d.dual;
  rec.frames.mirrorDual = !!d.mirrorDual;
  for (const [name, pd] of Object.entries(d.parts || {})) {
    rec.parts[name] = { pivot: v3(pd.pivot), mat: pd.mat, lamp: pd.lamp ? { color: pd.lamp.color, emissive: pd.lamp.emissive, intensity: pd.lamp.intensity, roughness: pd.lamp.roughness } : null };
  }
  const matClassOf = (g, partName) => {
    if (partName) { const pd = d.parts[partName]; return pd.mat === 'ink' ? 'ink' : pd.mat === 'lamp' ? 'lamp' : 'plastic'; }
    if (g === d.inkStatic || g === d.ink || g === d.drum) return 'ink';
    if (g === d.glow) return 'glow';
    return 'plastic';
  };
  // right hand: structure-aware walk via the instance's direct refs (disambiguates shared geoms,
  // e.g. charger 'ports' reuses d.body geometry)
  const emit = (g, slot, mtxObj, bone, ref, tier) => {
    if (!g) return;
    const key = `weapon:${kind}:${slot}${bone === 'handL' ? '@L' : ''}`;
    if (partMeta.has(key)) return;
    addPart(key, g, bone, relMatrix(mtxObj, ref), { tier, matClass: matClassOf(g, slot.startsWith('part:') ? slot.slice(5) : null), shadow: true });
    rec.instances.push(key);
  };
  // structure-aware walk of a weapon instance (right hand or the dual left-hand instance)
  const walk = (inst, bone, ref) => {
    emit(inst.body.geometry, 'body', inst.body, bone, ref, 'near');
    emit(inst.ink.geometry, 'ink', inst.ink, bone, ref, 'near');
    emit(inst.def.body, 'farBody', inst.bodyFar, bone, ref, 'far');
    emit(inst.def.ink, 'farInk', inst.inkFar, bone, ref, 'far');
    for (const [name, grp] of Object.entries(inst.parts || {})) {
      const m = grp.userData.mesh;
      emit(m.geometry, `part:${name}`, m, bone, ref, 'near');
    }
    if (inst.drum) {
      const dm = inst.drum.userData.ink;
      emit(dm.geometry, 'drum', dm, bone, ref, 'near');
      emit(inst.drum.children[1].geometry, 'drumCaps', inst.drum.children[1], bone, ref, 'near');
    }
    if (inst.spin) emit(inst.spin.children[0].geometry, 'spin', inst.spin.children[0], bone, ref, 'near');
    if (inst.glow) emit(inst.glow.geometry, 'glow', inst.glow, bone, ref, 'near');
  };
  walk(w, 'handR', handR);
  // runtime-far decimated variants (mirrors _animWeapon farT swap; mounted at the off frame)
  emit(LOD.farGeometry(d.body, 700), 'farDecim:body', w.off, 'handR', handR, 'far');
  emit(LOD.farGeometry(d.ink, 300), 'farDecim:ink', w.off, 'handR', handR, 'far');
  // dual left hand (dualies): a full second instance under handL
  if (w.left) {
    rec.frames.left = { pivot: v3(w.left.pivot.position), off: { pos: v3(w.left.off.position), quat: q4(w.left.off.quaternion) } };
    walk(w.left, 'handL', handL);
  }
  // twins mirror: pivotL holds [bodyL(d.body), inkL(d.ink)] meshes under a mir scale(-1,1,1)
  if (w.pivotL) {
    w.pivotL.traverse((o) => {
      if (!o.isMesh) return;
      const slot = o.geometry === d.body ? 'farBody' : o.geometry === d.ink ? 'farInk' : null;
      if (slot) emit(o.geometry, slot, o, 'handL', handL, 'far');
    });
  }
  weapons[kind] = rec;
}
ch.setWeapon('shooter');
ch.root.updateMatrixWorld(true);

// ---- 6) rig tables (real module values) ----
const restPos = geo.getRestPositions(STYLE0);
const boneInverses = geo.getBoneInverses(STYLE0).map((m) => [...m.elements]);
const hairRest = {};
for (let h = 0; h < geo.HAIR_STYLE_NAMES.length; h++) {
  const H = geo.getHairStyle({ ...STYLE0, hair: h }, 'hero');
  hairRest[h] = { name: H.name, rest: ser(H.rest), meta: ser(H.meta) };
}

// ---- finalize part ids + bin ----
const partKeys = [...partMeta.keys()].sort();
const partId = new Map(partKeys.map((k, i) => [k, i]));
for (const k of partKeys) store.instances[partMeta.get(k).id].partId = partId.get(k);
const bin = store.buildBin();
const binGz = gzipSync(bin, { level: 9 });
const binSha = createHash('sha256').update(binGz).digest('hex');

// ---- manifest ----
// Character geometry/pose/style modules plus the two wired kit models.
const commit = upstreamCommit(repoRoot, [
  'src/game/character.js',
  'src/game/character-geo.js',
  'src/game/character-weapons.js',
  'src/game/character-lod.js',
  'src/game/character-style.js',
  'src/game/character-face.js',
  'src/game/character-hair.js',
  'src/game/character-mats.js',
  'src/game/character-outfit.js',
  'src/game/kits/mitts-model.js',
  'src/game/kits/blade-model.js',
  'src/game/kits/bow-model.js',
  'src/game/kits/brolly-model.js',
]);
mkdirSync(OUT_DIR, { recursive: true });
writeFileSync(resolve(OUT_DIR, 'char.bin.gz'), binGz);

const manifest = {
  meta: {
    source: { commit, files: ['src/game/character.js', 'src/game/character-geo.js', 'src/game/character-weapons.js', 'src/game/character-lod.js', 'src/game/character-style.js', 'src/game/character-face.js', 'src/game/character-hair.js', 'src/game/character-mats.js', 'src/game/character-outfit.js'] },
    bin: 'char.bin.gz', binBytes: binGz.length, rawBytes: bin.length, binSha256: binSha,
    counts: {
      templates: store.templates.length, verts: store.totalVerts, tris: store.totalTris,
      instances: store.instances.length, bones: BONES.length, parts: partKeys.length,
      weaponKinds: KINDS.length, tiers: TIER_NAMES.length, hairStyles: geo.HAIR_STYLE_NAMES.length,
    },
    style: STYLE0,
    quant: { pos: 'i32 mm', normal: 'i16 x1e3', color: 'u8', weld: '1e-3 grid + content-hash dedup (≡ extract-props)', matrix: 'bone-local 3x4 row-major f32' },
  },
  rig: {
    boneNames: geo.BONE_NAMES, boneParent: geo.BONE_PARENT, boneIndex: geo.BONE_INDEX,
    hairMax: geo.HAIR_MAX, hairSegs: geo.HAIR_SEGS,
    aimPivot: v3(holdEval.AIM_PIVOT), fistOffset: v3(W.FIST_OFFSET), gripHoleL: v3(W.GRIP_HOLE_L),
    rest: ser(restPos),
    boneInverses, hairRest, pseudoBones: ['@kid', '@model'],
  },
  parts: partKeys.map((k) => {
    const p = partMeta.get(k);
    return { key: p.key, id: partId.get(k), tier: p.tier, bone: p.bone, matClass: p.matClass, shadow: p.shadow, template: p.template, verts: p.verts, tris: p.tris, matrix: p.matrix, extra: p.extra };
  }),
  weapons,
  weaponKindList: { kinds: W.WEAPON_KINDS, specialWeaponKinds: W.SPECIAL_WEAPON_KINDS },
  specialRefs: {
    specialBinKinds: ['crab', 'jetpack', 'kraken', 'missile', 'speaker'],
    specialBinNote: 'rust/assets/props/special.bin.gz holds these held props — NOT re-extracted here (name refs only).',
    subKinds: W.SUB_KINDS,
    subNote: 'only the idle "bomb" sub prop is extracted; other SUB_KINDS props are batch-C scope.',
    kitWeapons: ['blade', 'brolly', 'mitts', 'bow'],
    kitNote: 'kit models register via src/game/kits/*.js (not imported by character.js); HOLD entries still emitted in char-poses.json. Upstream has no "bowl" weapon (no config entry / kit file / HOLD) — the name only survives in a blade-model.js comment and char.schema.md batch-C note.',
  },
};
writeFileSync(resolve(OUT_DIR, 'char.json'), stableStringify(manifest) + '\n');

// ---- char-poses.json ----
const poses = {
  meta: {
    source: { commit, files: ['src/game/character.js', 'src/game/character-style.js'] },
    note: 'literal-block slices + eval (assertContains pinned; no hand-typed values). Channel indices computed from the real pose bank layout.',
  },
  poseChannels: channels,
  rigConst,
  XCH: stanceEval.XCH,
  AIM_PIVOT: v3(holdEval.AIM_PIVOT),
  HOLD: holdEval.HOLD,
  HOLD_HERO: holdEval.HOLD_HERO,
  ANIM_OF: holdEval.ANIM_OF,
  NO_TWIRL: holdEval.NO_TWIRL,
  BOW_CANT: holdEval.BOW_CANT,
  STANCE_IDLE: stanceEval.STANCE_IDLE,
  STANCE_LOCK: stanceEval.STANCE_LOCK,
  SHUFFLES: stanceEval.SHUFFLES,
  FIDGETS: stanceEval.FIDGETS,
  FIDGET_LEN: stanceEval.FIDGET_LEN,
  ...Object.fromEntries(keyNames.map((n) => [n, stanceEval[n]])),
  FINGER_SPREAD: stanceEval.FINGER_SPREAD,
  DANCE_VARIANTS: stanceEval.DANCE_VARIANTS,
  blinkCurve: { source: blinkSrc, defaults: { c: 0.08, h: 0.03, o: 0.16, lag: 0.008, amp: 1, dbl: false, squeeze: 0 }, defaultsSource: blDefaults[0] },
  expressions,
  style: {
    SKIN_TONES: styleEval.SKIN_TONES,
    SKIN_NAMES: styleEval.SKIN_NAMES,
    OUTFITS: styleEval.OUTFITS,
    OUTFIT_NAMES: styleEval.OUTFIT_NAMES,
    IRIS: styleEval.IRIS,
    IRIS_NAMES: styleEval.IRIS_NAMES,
    HAIR_STYLE_NAMES: styleEval.HAIR_STYLE_NAMES,
    HAIR_STYLES: STYLE.HAIR_STYLES,
  },
};
writeFileSync(resolve(OUT_DIR, 'char-poses.json'), stableStringify(poses) + '\n');

console.log(`char: templates=${store.templates.length} verts=${store.totalVerts} tris=${store.totalTris} instances=${store.instances.length} parts=${partKeys.length} weapons=${KINDS.length} bin=${binGz.length}B sha=${binSha.slice(0, 16)}`);
