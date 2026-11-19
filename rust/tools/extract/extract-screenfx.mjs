// Task M5-T64: extract the screen-FX composite overlay parameters from the
// READ-ONLY JS tree. The source files stay byte-clean. Output is deterministic
// (stable-json): rust/assets/screenfx.json.
//
// Values are located by source-fragment assertions then parsed with narrow
// regexes; upstream drift fails the pipeline instead of silently changing the
// artifact. `tools/screenfx-crosscheck.mjs` re-parses the same constants via an
// independent, line-oriented path.
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));            // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/screenfx.json');
const SRC_FILES = [
  'src/fx/screenfx.js',
  'src/ui/hud-boss.js',
  'src/ui/hud.js',
  'src/game/minimap.js',
];
const commit = upstreamCommit(repoRoot, SRC_FILES);

const read = (p) => readFileSync(resolve(repoRoot, p), 'utf8');
const sfx = read('src/fx/screenfx.js');
const boss = read('src/ui/hud-boss.js');
const hud = read('src/ui/hud.js');
const mini = read('src/game/minimap.js');

function assertContains(src, needle, where) {
  if (!src.includes(needle)) throw new Error(`screenfx extract: ${where}: missing ${needle}`);
}
function line(src, needle, where) {
  const hit = src.split('\n').find((l) => l.includes(needle));
  if (!hit) throw new Error(`screenfx extract: ${where}: no line with ${needle}`);
  return hit;
}
function nums(s) {
  const m = s.match(/-?\d+(?:\.\d+)?/g);
  if (!m) throw new Error(`screenfx extract: no numbers in ${s}`);
  return m.map(Number);
}

// ---- composite pass placement (CONTRACTS: grade → screenfx → output) ------
assertContains(sfx, 'R.setExtraPass(this.pass)', 'screenfx.js setExtraPass');
assertContains(sfx, 'name: \'InkwaveScreenFX\'', 'screenfx.js material name');

// ---- uniform defaults (screenfx.js L544-558) ------------------------------
const materialAt = sfx.indexOf("name: 'InkwaveScreenFX'");
const uStart = sfx.indexOf('uniforms: {', materialAt);
const uBlock = sfx.slice(uStart, sfx.indexOf('vertexShader:', uStart));
for (const n of [
  'uRes: { value: new THREE.Vector2(1600, 900)', 'uAspect: { value: 16 / 9',
  'uHeart: { value: new THREE.Vector4(0.6, 0.02, 0.06, 0)', 'uHurt: { value: 0 }',
  'uCharge: { value: new THREE.Vector4(1, 1, 1, 0)', 'uSat: { value: 1 }',
  'uKill: { value: new THREE.Vector4(1, 1, 1, 0)',
]) assertContains(uBlock, n, 'screenfx.js uniforms');
assertContains(sfx.slice(materialAt, uStart), 'defines: { TAPS: 8 }', 'screenfx.js defines');

// ---- decay / heartbeat / hurt constants ----------------------------------
const decayLine = line(sfx, 'const k = Math.exp(-dt * 6)', '_decayAll');
const decayRate = nums(decayLine).at(-1); // 6
const lowKLine = line(sfx, 'const lowK = alive ? clamp((0.5 - hp) / 0.38', 'lowK');
const lowKMatch = lowKLine.match(/\(0\.5 - hp\) \/ ([\d.]+)/);
const lowKDenom = Number(lowKMatch[1]);
const bpmLine = line(sfx, 'const bpm = 80 + 70 * lowK', 'heartbeat bpm');
const bpmBase = nums(bpmLine)[0], bpmScale = nums(bpmLine)[1];
const hurtDamp = Number(line(sfx, 's.hurt = damp(s.hurt, lowK, 4, dt)', 'hurt damp').match(/lowK, ([\d.]+), dt/)[1]);
const heartDamp = Number(line(sfx, 's.heart = damp(s.heart, 0, 6, dt)', 'heart damp').match(/s\.heart, 0, ([\d.]+), dt/)[1]);
const dmgAccLine = line(sfx, 's.dmgT = 0.06', 'damage window');
const dmgWindow = Number(dmgAccLine.match(/s\.dmgT = ([\d.]+)/)[1]);

// ---- pass_enabled thresholds (screenfx.js L862-866) -----------------------
const passStart = sfx.indexOf('const any = lensOn');
const passEnd = sfx.indexOf('this.pass.enabled = any');
if (passStart < 0 || passEnd < 0 || passEnd < passStart) {
  throw new Error('screenfx extract: pass enabled block markers not found');
}
const passBlock = sfx.slice(passStart, passEnd);
assertContains(passBlock, 's.flood > 0.001', 'pass enabled flood term');
// Per-term visibility thresholds: each `s.<field> ... > <threshold>` term.
// `lensOn` has no numeric threshold and is tracked separately.
const passTerms = {};
for (const raw of passBlock.replace(/\n/g, ' ').split('||')) {
  const term = raw.trim();
  const tm = term.match(/>\s*(0\.\d+)/);
  const nm = term.match(/s\.(\w+)/);
  if (tm && nm) passTerms[nm[1]] = Number(tm[1]);
}
for (const key of ['speed', 'stretch', 'punch', 'chroma', 'flood', 'sat']) {
  if (!(key in passTerms)) throw new Error(`screenfx extract: pass term ${key} missing`);
}
const passThresholds = [...new Set(Object.values(passTerms))].sort((a, b) => a - b);

// ---- damage-number pool (hud-boss.js L402-440) ---------------------------
const comboLine = line(boss, 'now - c.last > 0.45', 'bnum combo window');
const bnumLife = Number(line(boss, 'age > 1.05', 'bnum life').match(/age > ([\d.]+)/)[1]);
const bnumOut = Number(line(boss, 'age > 0.45 && !c.out', 'bnum out').match(/age > ([\d.]+)/)[1]);
const bnumCap = Number(line(boss, 'S.nums.length > 6', 'bnum cap').match(/length > (\d+)/)[1]);
const bnumWeakScale = Number(line(boss, "(c.weak ? 0.2 : 0)", 'bnum weak scale').match(/\? ([\d.]+)/)[1]);
const critLine = line(boss, 'setTimeout(() => el.remove(), 900)', 'CRIT lifetime');
const critMs = Number(critLine.match(/remove\(\), (\d+)\)/)[1]);
const comboWindow = Number(comboLine.match(/> ([\d.]+)/)[1]);

// ---- minimap dots / death marks / FX lives -------------------------------
const dotCount = Number(line(hud, 'this.mapDots = Array.from({ length: 8 }', 'minimap dots').match(/length: (\d+)/)[1]);
const deathCount = Number(line(hud, 'this.mapDms = Array.from({ length: 12 }', 'minimap deaths').match(/length: (\d+)/)[1]);
const fxCap = Number(line(mini, 'fxList.length > 40', 'minimap fx cap').match(/> (\d+)/)[1]);
const fxLife = {};
for (const [key, needle] of [
  ['slam', "kind: 'slam'"], ['boom', "kind: 'boom'"],
  ['jump', "kind: 'jump'"], ['spawn', "kind: 'spawn'"],
]) {
  const l = line(mini, needle, `minimap fx ${key}`);
  fxLife[key] = Number(l.match(/life: ([\d.]+)/)[1]);
}
fxLife.land = Number(line(mini, 'f.life = Math.min(f.life, f.t + 0.35)', 'minimap fx land').match(/\+ ([\d.]+)/)[1]);

const doc = {
  schema: 1,
  source: { commit, files: SRC_FILES },
  composite: {
    material_name: 'InkwaveScreenFX',
    taps: 8,
    uniform_defaults: {
      res: [1600, 900],
      aspect: 16 / 9,
      heart: [0.6, 0.02, 0.06, 0.0],
      hurt: 0,
      charge: [1, 1, 1, 0],
      sat: 1,
      kill: [1, 1, 1, 0],
    },
    pass_thresholds: passThresholds,
    pass_terms: passTerms,
    pass_order: ['grade', 'screenfx', 'output'],
  },
  decay: {
    rate: decayRate,
    low_k_denom: lowKDenom,
    bpm_base: bpmBase,
    bpm_scale: bpmScale,
    hurt_damp: hurtDamp,
    heart_damp: heartDamp,
    damage_window: dmgWindow,
  },
  damage_number: {
    pool_cap: bnumCap,
    combo_window: comboWindow,
    life: bnumLife,
    out_after: bnumOut,
    weak_scale_bonus: bnumWeakScale,
    crit_ms: critMs,
  },
  minimap: {
    dot_count: dotCount,
    death_mark_count: deathCount,
    fx_cap: fxCap,
    fx_life: fxLife,
  },
};

writeFileSync(outPath, stableStringify(doc) + '\n');
console.log(`extract-screenfx: ${commit.slice(0, 8)} -> ${resolve(repoRoot, 'rust/assets/screenfx.json')}`);
