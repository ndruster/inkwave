// Task M4-T54 batch B1 (D-T54-2): character pose reference harness.
// Runs the REAL src/game/character.js headless (same stub discipline as extract-character.mjs:
// real THREE from node_modules, ctx.js G object filled with minimal scene/physics/settings),
// instantiates the canonical rookie-style kid with a shooter, drives a scripted multi-phase
// scenario through Character.update(dt, AnimState) and dumps, per frame:
//   - every input (root.position, root.rotation.y, full AnimState, trigger calls)
//   - world position+quaternion of all 87 rig bones (after root.updateMatrixWorld(true))
//   - local transforms of the @kid / @model groups, the squid body groups, the tank surface,
//     and the animated weapon frames (pivot spin/scale, off, muzzle, bolt, trigger, can)
//   - the layout tables the Rust port needs without hand-copying numbers: channel indices
//     (sliced+eval'd from character.js, same technique as extract-character.mjs), spring
//     indices, timer indices, S_/X_/M_ constants, rig constants, hairMeta/hairIn/hairAx/A1/A2/
//     hairPh read off the live instance, tank T params, constructor rng-derived initial values.
// B2 is fully ported: face/fingers/jiggle/gaze/microexpr/dance/one-shot/
// lifePost/breathe/blink layers all RUN for real on both sides. Only the five
// methods in B2_NOOPS (below) are patched out before the scenario runs —
// render-side uniforms, locker/menu attention and squid blink — so the
// mulberry32 stream stays aligned. The fixture header `meta.b2Noops` is the
// authoritative list; the Rust core mirrors exactly those five no-ops.
//
// Output (deterministic; run twice -> byte-identical):
//   rust/crates/inkwave/tests/fixtures/char-pose-ref.json
//
// The Rust test `char_pose_reference_pinned` replays the embedded inputs through the pure-Rust
// port and compares per-bone world positions within 1e-4.

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');
const OUT = resolve(repoRoot, 'rust/crates/inkwave/tests/fixtures/char-pose-ref.json');
const CHAR_JS = resolve(repoRoot, 'src/game/character.js');

// ---- headless stubs (defensive; same as extract-character.mjs) ----
globalThis.document = globalThis.document || {
  createElement: (t) => (t === 'canvas'
    ? { width: 0, height: 0, getContext: () => new Proxy({}, { get: () => () => ({ addColorStop() {} }) }) }
    : {}),
};
globalThis.innerWidth = globalThis.innerWidth || 1920;
globalThis.innerHeight = globalThis.innerHeight || 1080;

// ---- real module imports (no mocks) ----
const THREE = await import(resolve(repoRoot, 'node_modules/three/build/three.module.js'));
const ctxMod = await import(resolve(repoRoot, 'src/core/ctx.js'));
const C = await import(resolve(repoRoot, 'src/game/character.js'));
const W = await import(resolve(repoRoot, 'src/game/character-weapons.js'));
// kit weapons for the J/K scenario (same registration as extract-character.mjs:
// mitts/bow self-register; blade/brolly register manually from the standalone
// -model.js modules)
await import(resolve(repoRoot, 'src/game/kits/mitts-model.js'));
await import(resolve(repoRoot, 'src/game/kits/bow-model.js'));
const bladeModel = await import(resolve(repoRoot, 'src/game/kits/blade-model.js'));
W.registerWeaponModel('blade', bladeModel.buildBlade, bladeModel.animateBlade);
const brollyModel = await import(resolve(repoRoot, 'src/game/kits/brolly-model.js'));
W.registerWeaponModel('brolly', brollyModel.brollyDef, brollyModel.animateBrolly);
const { G } = ctxMod;

// ---- minimal world context: inWorld = G.scene && root.parent === G.scene && G.physics ----
G.scene = new THREE.Scene();
G.settings = { quality: 'high' };
// flat ground only through the phys-null fallback in _ground (raycast never hits → gnd stays 9)
G.physics = { raycast: (_o, _d, _m, hit) => { hit.hit = false; return hit; }, raycastRay: () => null };
G.camera = null;
G.env = null;
G.fx = null;
G.renderer = null;

// ---- literal-block slicing + eval (extract-character.mjs discipline: markers pinned) ----
const srcChar = readFileSync(CHAR_JS, 'utf8');
function sliceBlock(src, startMarker, endMarker, label) {
  const i = src.indexOf(startMarker);
  if (i < 0) throw new Error(`marker missing (start): ${label} :: ${startMarker}`);
  const j = src.indexOf(endMarker, i);
  if (j < 0) throw new Error(`marker missing (end): ${label} :: ${endMarker}`);
  return src.slice(i, j + endMarker.length);
}
function sliceToEol(src, marker, label) {
  const i = src.indexOf(marker);
  if (i < 0) throw new Error(`marker missing: ${label} :: ${marker}`);
  const j = src.indexOf('\n', i);
  return src.slice(i, j);
}

// channel bank (character.js L175-189): pure S() index arithmetic -> real channel numbers
const bankBlock = sliceBlock(srcChar, 'let _k = 0;', 'const PN = _k;', 'channel bank');
const CHANNEL_NAMES = [...bankBlock.matchAll(/(?:const|,)\s*([A-Z_0-9]+) = S[,(]/g)].map((m) => m[1]);
const channels = new Function(`${bankBlock}\nreturn {${CHANNEL_NAMES.join(',')}, PN};`)();
if (CHANNEL_NAMES.length !== 66 || channels.PN !== 126) throw new Error(`unexpected channel layout: PN=${channels.PN} names=${CHANNEL_NAMES.length}`);

// spring bank (L206-216) + timer bank (L218-222): SPG()/TK() counters, values unused for indices
const springBlock = sliceBlock(srcChar, 'let _sk = 0; const SPG =', 'const SPN = _sk * 2;', 'spring bank');
const SPRING_NAMES = [...springBlock.matchAll(/(?:const|,)\s*([A-Z_0-9]+) = SPG[(),]/g)].map((m) => m[1]);
const springs = new Function(`${springBlock}\nreturn {${SPRING_NAMES.join(',')}, SPN};`)();
const timerBlock = sliceBlock(srcChar, 'let _tk = 0; const TK =', 'const TN = _tk;', 'timer bank');
const TIMER_NAMES = [...timerBlock.matchAll(/(?:const|,)\s*([A-Z_0-9]+) = TK\(\)/g)].map((m) => m[1]);
const timers = new Function(`${timerBlock}\nreturn {${TIMER_NAMES.join(',')}, TN};`)();
if (springs.SPN !== SPRING_NAMES.length * 2 || timers.TN !== TIMER_NAMES.length) throw new Error(`unexpected bank sizes: SPN=${springs.SPN}/${SPRING_NAMES.length} TN=${timers.TN}/${TIMER_NAMES.length}`);

// stepping-mode constants (L225); MODELR/SQY/... are ordinary channels already captured in the bank
const mLine = sliceToEol(srcChar, 'const M_GAIT = 0,', 'M_*');
const M = new Function(`${mLine}\nreturn {M_GAIT, M_CATCH, M_SETTLE};`)();

// rig constants (L166-170)
const geoMod = await import(resolve(repoRoot, 'src/game/character-geo.js'));
const rigConstBlock = sliceBlock(srcChar, 'const ANKLE_H = REST.footL.y;', 'new THREE.Vector3(0, 0.164, 0.014);', 'rig constants');
const rigConst = new Function('REST', 'THREE', `${rigConstBlock}\nreturn { ANKLE_H, HIPW, BALL_Z, HEEL_Z, HEAD_CTR: [HEAD_CTR.x, HEAD_CTR.y, HEAD_CTR.z] };`)(geoMod.REST, THREE);

// ---- canonical instance (rookie preset, same style as extract-character.mjs) ----
const STYLE0 = { hair: 0, skin: 0, outfit: 0, eyes: 0, hat: 0, brows: 0 };
const NAME = 'pin-test';
const ch = new C.Character({ name: NAME, color: '#ff8a14', weapon: 'shooter', style: STYLE0 });

// ---- B2 layer no-ops (Rust mirrors these exactly) ----
// Batch B2 enablement: dances, face, jiggle and fingers now RUN for real
// (Rust ports them); only the menu-side and render-side layers stay patched.
// `_poseDance` runs, so the scenario's dance segment drives setDance plus a
// raw danceVar override via emit's `extra` (both recorded per frame).
// `_updateMaterials` stays patched: the Rust port is a pure-data output
// pinned by unit tests, not by this transform replay. `_squidBlink` must
// stay a no-op: it would advance the blink clocks during the squid form
// (Rust skips it too). `_pickLook`/`_attendMenu` are unreachable in the
// harness (no G.actors, inWorld=true) but stay patched for stream safety.
const B2_NOOPS = [
  '_poseLocker', '_attendMenu', '_pickLook', '_squidBlink', '_updateMaterials',
];
for (const m of B2_NOOPS) ch[m] = () => {};

G.scene.add(ch.root);

// ---- capture the exact worldQuat the JS slosher branch reads (mid-update) ----
const _origAnimWeapon = ch._animWeapon.bind(ch);
let _wqCapture = null;
const WQ_CHAIN = ['hips', 'spine', 'chest', 'clavR', 'uArmR', 'fArmR', 'handR'];
let _chainCapture = null;
ch._animWeapon = (dt, s, w) => {
  const slo = w.def.kind === 'slosher' && w.off;
  _wqCapture = slo ? w.off.getWorldQuaternion(new THREE.Quaternion()).toArray() : null;
  _chainCapture = slo ? [ch.kid.quaternion.toArray(), ...WQ_CHAIN.map((n) => ch.bones[n].quaternion.toArray()), w.pivot.quaternion.toArray(), w.off.quaternion.toArray()] : null;
  _origAnimWeapon(dt, s, w);
};

// ---- per-frame output dump ----
const r9 = (v) => Math.round(v * 1e9) / 1e9;
const P = (o) => [r9(o.position.x), r9(o.position.y), r9(o.position.z)];
const Q = (o) => [r9(o.quaternion.x), r9(o.quaternion.y), r9(o.quaternion.z), r9(o.quaternion.w)];
const PS = (o) => [r9(o.position.x), r9(o.position.y), r9(o.position.z), r9(o.quaternion.x), r9(o.quaternion.y), r9(o.quaternion.z), r9(o.quaternion.w), r9(o.scale.x), r9(o.scale.y), r9(o.scale.z)];
const WPS = (o) => { const p = o.getWorldPosition(new THREE.Vector3()); const q = o.getWorldQuaternion(new THREE.Quaternion()); const s = o.getWorldScale(new THREE.Vector3()); return [r9(p.x), r9(p.y), r9(p.z), r9(q.x), r9(q.y), r9(q.z), r9(q.w), r9(s.x), r9(s.y), r9(s.z)]; };
const BONE_NAMES = geoMod.BONE_NAMES;

// ---- weapon header snapshot (taken at construction, before the scenario:
// the J section rotates weapons, so scenario-end `ch.weapon` ≠ shooter) ----
const WEAPON_HDR = {
  kind: ch.weaponKind, animKind: ch.animKind, hero: !!ch.weapon.hero,
  pivotPos: P(ch.weapon.pivot), offPos: P(ch.weapon.off), offQuat: Q(ch.weapon.off),
  muzzle: [r9(ch.weapon.muzzle.position.x), r9(ch.weapon.muzzle.position.y), r9(ch.weapon.muzzle.position.z)],
  parts: Object.keys(ch.weapon.parts),
  partPivots: Object.fromEntries(Object.entries(ch.weapon.parts).map(([k, p]) => [k, P(p)])),
};

function dump() {
  const bones = [];
  for (const n of BONE_NAMES) { const b = ch.bones[n]; bones.push(P(b), Q(b)); }
  const w = ch.weapon;
  return {
    bones,                                   // flat: 87 × [p(3), q(4)] interleaved pairs
    kid: PS(ch.kid), model: PS(ch.model),
    squidRoot: PS(ch.squidRoot), squidPivot: PS(ch.squid.pivot),
    tankFill: [r9(ch.tank.fill.position.y), r9(ch.tank.fill.rotation.x), r9(ch.tank.fill.scale.x), r9(ch.tank.fill.scale.z)],
    wpPivot: [r9(w.pivot.rotation.x), r9(w.pivot.scale.x)],
    wpOff: WPS(w.off), wpMuzzle: P(w.muzzle),
    // dynamic per-part locals (weapon families own different part sets)
    wpParts: Object.entries(w.parts).map(([n, p]) => [n, P(p), Q(p), [r9(p.scale.x), r9(p.scale.y), r9(p.scale.z)]]),
    wpPs: w.ps ? Array.from(w.ps).map(r9) : null,   // per-instance spring bank (kit weapons leave it uncreated)
    wpWq: _wqCapture ? _wqCapture.map(r9) : null,   // slosher worldQuat at _animWeapon call time
    wpChain: _chainCapture ? _chainCapture.map((q) => q.map(r9)) : null, // [kid, hips..handR, pivot, off] local quats at that instant
    wpLeft: w.left ? {
      pivot: [r9(w.left.pivot.rotation.x), r9(w.left.pivot.scale.x)],
      off: WPS(w.left.off),
      parts: Object.entries(w.left.parts).map(([n, p]) => [n, P(p), Q(p), [r9(p.scale.x), r9(p.scale.y), r9(p.scale.z)]]),
    } : null,
    // Task 58: kit weapon animation state (transform-side springs the wpParts
    // dump can't carry: bow draw / string spring, brolly trigger follow).
    wpKit: w.def.kind === 'bow' ? {
      kind: 'bow',
      draw: r9(w.bowDraw ?? 0),
      bw: w.bw ? { str: r9(w.bw.str), strV: r9(w.bw.strV), loose: !!w.bw.loose, lastRel: r9(w.bw.lastRel), glow: r9(w.bw.glow) } : null,
    } : w.def.kind === 'brolly' ? { kind: 'brolly', trig: r9(w.trig || 0) } : null,
  };
}

// ---- scenario ----
const DT = 1 / 60;
const frames = [];

function mkState(o = {}) {
  return Object.assign({
    grounded: true, vy: 0, speed: 0, localMove: { x: 0, z: 0 }, turnRate: 0,
    aimPitch: 0, aimYaw: 0, firing: false, charge: 0, subAim: false, rolling: false,
    lowInk: false, ink: 0.9, hp: 1, inEnemyInk: false, invuln: false, form: 'kid', special: null, runner: null,
  }, o);
}

const root = { x: 0, y: 0, z: 0, ry: 0 };
function emit(triggers, state, weapon = null, extra = null) {
  const f = { t: frames.length, trig: triggers, rp: [root.x, root.y, root.z], ry: root.ry, s: state };
  if (weapon) { f.setW = weapon; ch.setWeapon(weapon); }   // ≡ external weapon swap (no RNG)
  if (extra) Object.assign(f, extra);                      // record dance/danceVar on the frame
  frames.push(f);
  ch.root.position.set(root.x, root.y, root.z);
  ch.root.rotation.y = root.ry;
  // ≡ pinned.rs replay order: setDance, then a raw danceVar override, both
  // before triggers/update (character.js L1225-1235 / L1231).
  if (extra) {
    if (extra.dance !== undefined) ch.setDance(extra.dance);
    if (extra.danceVar !== undefined) ch.danceVar = extra.danceVar;
  }
  for (const [name, arg] of triggers) ch.trigger(name, arg);
  ch.update(DT, state);
  ch.root.updateMatrixWorld(true);
  f.o = dump();
}
function idle(n, extra = {}) {
  for (let i = 0; i < n; i++) emit([], mkState(extra));
}
function run(n, speedAt, { turnRate = 0, form = 'kid', ...rest } = {}) {
  for (let i = 0; i < n; i++) {
    const speed = speedAt(i, n);
    const fwd = [Math.sin(root.ry), 0, Math.cos(root.ry)];
    root.x += fwd[0] * speed * DT; root.z += fwd[2] * speed * DT;
    root.ry += turnRate * DT;
    emit([], mkState({ speed, turnRate, localMove: { x: 0, z: 1 }, form, ...rest }));
  }
}

// A: idle long enough to fire shuffles, weight shifts and a fidget (rng paths)
idle(300);
// B: accelerate to a full run
run(90, (i) => 7 * Math.min(1, i / 60));
// C: cruise
run(60, () => 7);
// D: hard stop + settle/shuffle recovery
run(45, (i) => 7 * (1 - i / 44));
idle(75);
// E: turn-in-motion (banking, predictive gait targets) then straighten
run(75, () => 4, { turnRate: 1.5 });
run(45, () => 4);
// F: running jump → airborne → land (spring impulses; _poseLand is a B2 no-op)
run(15, () => 5);
{
  let vy = 4.6, air = 0;
  emit([['jump', 5]], mkState({ speed: 5, localMove: { x: 0, z: 1 }, grounded: false, vy }));
  root.y += vy * DT; vy -= 15.5 * DT; air++;
  while (root.y > 0 && air < 200) {
    root.y += vy * DT; vy -= 15.5 * DT;
    root.z += 5 * DT * Math.cos(root.ry); root.x += 5 * DT * Math.sin(root.ry);
    if (root.y <= 0) { root.y = 0; emit([['land', Math.min(6, -vy)]], mkState({ speed: 5, localMove: { x: 0, z: 1 } })); }
    else emit([], mkState({ speed: 5, localMove: { x: 0, z: 1 }, grounded: false, vy }));
    air++;
  }
}
idle(90);
// G: kid shooting (recoil springs, bolt/can cycles, trigger squeeze) with aim sweep
{
  let ink = 0.9;
  for (let i = 0; i < 150; i++) {
    const firing = i % 10 < 5;
    const trig = [];
    if (i % 10 === 0) { trig.push(['shoot', 0]); ink = Math.max(0.2, ink - 0.03); }
    emit(trig, mkState({ firing, ink, aimPitch: 0.5 * Math.sin(i * 0.05), speed: 0 }));
  }
}
// H: kid → squid form switch, dry-hop run, then back to kid
idle(30);
run(120, (i) => (i > 60 ? 3 : 0), { form: 'squid' });
run(120, (i) => (i > 60 ? 2.5 : 0), { form: 'kid' });
// J: B2 one-shot pose layers + weapon-family rotation (roller flick/drum, brush
// swipe, bucket heave, slosher slosh + worldQuat aim sweep), throw/subAim,
// hits (blink/mx), dodge, spawn pop, special leap → slam.
{
  emit([], mkState(), 'roller');
  idle(20);
  emit([['flick', 1]], mkState({}));
  idle(45);
  emit([['flick', -1]], mkState({}));
  idle(30);
  for (let i = 0; i < 40; i++) emit(i % 8 === 0 ? [['shoot', 0]] : [], mkState({ firing: i % 8 < 4 }));
  idle(40);
  emit([], mkState(), 'brush');
  idle(20);
  emit([['flick', 1]], mkState({}));
  idle(60);
  emit([['flick', -1]], mkState({}));
  idle(40);
  emit([], mkState(), 'bucket');
  idle(20);
  emit([['flick', 1]], mkState({}));
  idle(60);
  emit([], mkState(), 'slosher');
  idle(30);
  emit([['slosh']], mkState({}));
  idle(50);
  for (let i = 0; i < 40; i++) emit(i === 12 ? [['slosh']] : [], mkState({ aimPitch: 0.6 * Math.sin(i * 0.1), aimYaw: 0.4 * Math.cos(i * 0.07) }));
  idle(40);
  emit([['throw']], mkState({ subAim: true }));
  for (let i = 0; i < 60; i++) emit([], mkState({ subAim: true, aimPitch: 0.3 }));
  for (let i = 0; i < 40; i++) emit([], mkState({ subAim: false }));
  emit([], mkState(), 'shooter');
  idle(30);
  emit([['hit', { x: 0.8, z: 0.3, amount: 1.2 }]], mkState({ hp: 0.8 }));
  idle(60);
  emit([['hit', { x: -0.6, z: 0.6, amount: 0.5 }]], mkState({ hp: 0.7 }));
  idle(80);
  emit([['dodge', { x: 0.3, z: 1, t: 0.45 }]], mkState({ speed: 3, localMove: { x: 0, z: 1 } }));
  for (let i = 0; i < 50; i++) emit([], mkState({ speed: 3, localMove: { x: 0, z: 1 } }));
  idle(40);
  emit([['spawn']], mkState({}));
  idle(90);
  emit([['special_leap']], mkState({ grounded: false, vy: 6 }));
  {
    let vy = 6, y = 0;
    for (let i = 0; i < 70; i++) {
      y += vy * DT; vy -= 15.5 * DT; if (y < 0) y = 0;
      root.y = y;
      const grounded = y <= 0;
      emit(i === 40 ? [['special_slam']] : [], mkState({ grounded, vy: grounded ? 0 : vy }));
      if (grounded && i > 45) break;
    }
    root.y = 0;
  }
  idle(80);
}
// K: kit weapons — mitts (dual pads, punch spring, left instance) + blade
// (poseBlade slash/heavy layers; animateBlade is material-only).
{
  emit([], mkState(), 'mitts');
  idle(30);
  for (let i = 0; i < 60; i++) emit(i % 12 === 0 ? [['shoot', 0]] : [], mkState({ firing: i % 12 < 6 }));
  idle(40);
  emit([], mkState(), 'blade');
  idle(30);
  emit([['blade_slash', 1]], mkState({}));
  idle(50);
  emit([['blade_slash', -1]], mkState({}));
  idle(40);
  emit([['blade_heavy']], mkState({ charge: 0.6 }));
  for (let i = 0; i < 70; i++) emit([], mkState({ charge: Math.max(0, 0.6 - i * 0.02) }));
  idle(40);
}
// K2: Task 58 — bow (poseBow draw/hold/cant + animateBow string spring, limb
// flex, nocked-arrow show window) and brolly (collar/trigger transformation
// surface; the canopy kit state machine has no Rust counterpart, so this
// pins the furled/shot surfaces). Note: poseBow's _bowCant begins at the
// ground value, swings upright in the air draw, back on landing.
{
  emit([], mkState(), 'bow');
  idle(30);
  // full draw → release → regen: pins the draw easing, the _bowHold
  // follow-through, the loose-string ring and the arrow hide/nock window
  for (let i = 0; i < 60; i++) emit([], mkState({ charge: Math.min(1, i * 0.03), aimPitch: 0.3 * Math.sin(i * 0.05) }));
  emit([['charge_release', 0]], mkState({ charge: 1 }));
  for (let i = 0; i < 45; i++) emit([], mkState({}));
  // tap shot: sub-ring1 draw, release straight after
  for (let i = 0; i < 20; i++) emit([], mkState({ charge: 0.2 }));
  emit([['charge_release', 0]], mkState({ charge: 0.2 }));
  for (let i = 0; i < 30; i++) emit([], mkState({}));
  for (let i = 0; i < 40; i++) emit([], mkState({ charge: Math.min(1, 0.1 + i * 0.05) }));
  emit([['charge_release', 0]], mkState({ charge: 1 }));
  idle(30);
  // cant: airborne while drawn (the bow swings upright), land (swings back)
  {
    let vy = 4.6;
    emit([['jump', 5]], mkState({ grounded: false, vy, charge: 1, speed: 5, localMove: { x: 0, z: 1 } }));
    root.y += vy * DT; vy -= 15.5 * DT;
    while (root.y > 0) {
      root.y += vy * DT; vy -= 15.5 * DT;
      root.z += 5 * DT * Math.cos(root.ry); root.x += 5 * DT * Math.sin(root.ry);
      if (root.y <= 0) { root.y = 0; emit([['land', Math.min(6, -vy)]], mkState({ charge: 1, speed: 5, localMove: { x: 0, z: 1 } })); }
      else emit([], mkState({ grounded: false, vy, charge: 1, speed: 5, localMove: { x: 0, z: 1 } }));
    }
  }
  idle(40);
}
{
  emit([], mkState(), 'brolly');
  idle(20);
  // trigger follow spring ≡ brolly-model.js L355-359; the collar sits at its
  // furled rest (no runner kit state in the harness)
  for (let i = 0; i < 60; i++) emit(i % 10 === 0 ? [['shoot', 0]] : [], mkState({}));
  idle(40);
}
// I: tail idle
idle(150);

// J: dances — one full period per variant so every window of each cycle is
// pinned. Each entry frame carries { dance, danceVar } (setDance + override
// before update, crossfading from the previous dance); a closing `null`
// frame pins the fade-out.
{
  emit([], mkState(), null, { dance: 'victory', danceVar: 0 }); idle(239);  // pump:     b=(t*2.1)%8 ≈ 229f
  emit([], mkState(), null, { dance: 'victory', danceVar: 1 }); idle(279);  // flourish: t%4.6     = 276f
  emit([], mkState(), null, { dance: 'victory', danceVar: 2 }); idle(199);  // hops:     b=(t*2.5)%8 = 192f
  emit([], mkState(), null, { dance: 'defeat', danceVar: 0 }); idle(359);   // slump:    t%6       = 360f
  emit([], mkState(), null, { dance: 'defeat', danceVar: 1 }); idle(389);   // sulk:     t%6.5     = 390f
  emit([], mkState(), null, { dance: 'defeat', danceVar: 2 }); idle(311);   // kick:     t%5.2     = 312f
  emit([], mkState(), null, { dance: 'menu_idle' }); idle(479);             // menu:     t%8       = 480f
  emit([], mkState(), null, { dance: 'lobby_pose' }); idle(419);            // lobby:    t%7       = 420f
  emit([], mkState(), null, { dance: null }); idle(60);
}

// ---- header: layout + live-instance params (no hand-copied numbers) ----
const chanIdx = {}; for (const n of CHANNEL_NAMES) chanIdx[n] = channels[n];
const sprIdx = {}; for (const n of SPRING_NAMES) sprIdx[n] = springs[n];
const timIdx = {}; for (const n of TIMER_NAMES) timIdx[n] = timers[n];

function hashStr(s) { let h = 2166136261; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); } return h >>> 0; }

const header = {
  meta: {
    harness: 'char-pose-reference.mjs', task: 'M4-T54 B2', frames: frames.length, dt: DT,
    name: NAME, seed: hashStr(NAME), style: STYLE0, color: '#ff8a14', weapon: 'shooter',
    b2Noops: B2_NOOPS.slice().sort(),
  },
  layout: {
    channels: chanIdx, channelNames: CHANNEL_NAMES, PN: channels.PN,
    springs: sprIdx, SPN: springs.SPN, timers: timIdx, TN: timers.TN,
    M, rigConst,
  },
  hair: {
    nStrands: ch.hairMeta.length,
    meta: ch.hairMeta.map((m) => ({ dir: [r9(m.dir.x), r9(m.dir.y), r9(m.dir.z)], G: r9(m.G), K: r9(m.K), len: r9(m.len) })),
    in: [...ch.hairIn].map(r9), ax: [...ch.hairAx].map(r9), a1: [...ch.hairA1].map(r9), a2: [...ch.hairA2].map(r9),
    ph: [...ch.hairPh].map(r9),
  },
  init: {
    shufT: r9(ch.shufT), shiftT: r9(ch.shiftT), nextFidget: r9(ch.nextFidget),
    blNext: r9(ch.bl.next), attT: r9(ch.att.t), mxT: r9(ch.mxT), sighT: r9(ch.sighT),
    gripT: r9(ch.gripT), earT: r9(ch.earT), lookT: r9(ch.lookT), danceOfs: r9(ch.danceOfs),
  },
  tank: {
    tilt: r9(ch.tank.tilt), h: r9(ch.tank.h), bottom: r9(ch.tank.bottom),
    offset: P(ch.tank.group), fillPos: P(ch.tank.fill), fillScale: [r9(ch.tank.fill.scale.x), r9(ch.tank.fill.scale.y), r9(ch.tank.fill.scale.z)],
  },
  weapon: WEAPON_HDR,
  boneNames: BONE_NAMES,
};

const out = JSON.stringify({ header, frames });
mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, out);

// ---- sanity: no NaN anywhere ----
if (out.includes('NaN')) throw new Error('fixture contains NaN');
console.log(`wrote ${OUT}: ${frames.length} frames, ${(out.length / 1e6).toFixed(2)} MB, bones=${BONE_NAMES.length}`);
