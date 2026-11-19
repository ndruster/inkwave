// Task M5-T64: independent crosscheck of assets/screenfx.json.
// This file intentionally does NOT import extract-screenfx.mjs: every value is
// re-parsed from the READ-ONLY JS sources via a second, line-oriented path.
import { upstreamCommit } from './extract/upstream-commit.mjs';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));                  // rust/tools
const repoRoot = resolve(here, '../..');
const doc = JSON.parse(readFileSync(resolve(here, '../assets/screenfx.json'), 'utf8'));
const read = (p) => readFileSync(resolve(repoRoot, p), 'utf8').split('\n');
const sfx = read('src/fx/screenfx.js');
const boss = read('src/ui/hud-boss.js');
const hud = read('src/ui/hud.js');
const mini = read('src/game/minimap.js');

let checks = 0;
function eq(got, want, label) {
  checks += 1;
  if (got !== want) throw new Error(`screenfx crosscheck: ${label}: got ${got}, JS=${want}`);
}
function near(got, want, label) {
  checks += 1;
  if (Math.abs(got - want) > 1e-9) throw new Error(`screenfx crosscheck: ${label}: got ${got}, JS=${want}`);
}
function find(lines, needle, label) {
  const hit = lines.find((l) => l.includes(needle));
  if (!hit) throw new Error(`screenfx crosscheck: ${label}: no line for ${needle}`);
  return hit;
}
function vector(s, label) {
  const m = s.match(/new THREE\.Vector[24]\(([^)]*)\)/);
  if (!m) throw new Error(`screenfx crosscheck: ${label}: not a vector literal: ${s}`);
  return m[1].split(',').map((x) => Number(x.trim()));
}

// Source pin.
eq(doc.source.commit, upstreamCommit(repoRoot, [
  'src/fx/screenfx.js', 'src/ui/hud-boss.js', 'src/ui/hud.js', 'src/game/minimap.js',
]), 'source.commit');
eq(doc.source.files.join(','), 'src/fx/screenfx.js,src/ui/hud-boss.js,src/ui/hud.js,src/game/minimap.js', 'source.files');

// Composite placement (grade → screenfx → output).
eq(sfx.some((l) => l.includes('R.setExtraPass(this.pass)')), true, 'setExtraPass');
eq(doc.composite.material_name, find(sfx, "name: 'InkwaveScreenFX'", 'material').match(/name: '([^']+)'/)[1], 'material name');
eq(doc.composite.taps, Number(find(sfx, 'TAPS: 8', 'taps').match(/TAPS: (\d+)/)[1]), 'taps');
eq(doc.composite.pass_order.join(','), 'grade,screenfx,output', 'pass order');

// Uniform defaults.
const u = doc.composite.uniform_defaults;
near(u.aspect, 16 / 9, 'aspect');
near(u.hurt, Number(find(sfx, 'uHurt: { value:', 'uHurt').match(/uHurt: \{ value: (\d+)/)[1]), 'hurt');
const heart = vector(find(sfx, 'uHeart: { value:', 'uHeart'), 'heart');
heart.forEach((v, i) => near(u.heart[i], v, `heart[${i}]`));
near(u.heart[3], 0, 'heart alpha');
const res = find(sfx, 'uRes: { value:', 'uRes').match(/Vector2\((\d+), (\d+)\)/).slice(1).map(Number);
res.forEach((v, i) => eq(u.res[i], v, `res[${i}]`));
const charge = find(sfx, 'uCharge: { value:', 'uCharge').match(/Vector4\(([^)]*)\)/)[1].split(',').map(Number);
charge.forEach((v, i) => eq(u.charge[i], v, `charge[${i}]`));
const kill = find(sfx, 'uKill: { value:', 'uKill').match(/Vector4\(([^)]*)\)/)[1].split(',').map(Number);
kill.forEach((v, i) => eq(u.kill[i], v, `kill[${i}]`));
near(u.sat, Number(find(sfx, 'uSat: { value:', 'uSat').match(/uSat: \{ value: (\d+)/)[1]), 'sat');

// pass_enabled thresholds: independently collect all `> 0.xxx` terms.
const anyIdx = sfx.findIndex((l) => l.includes('const any = lensOn'));
const enabledIdx = sfx.findIndex((l) => l.includes('this.pass.enabled = any'));
const anyBlock = sfx.slice(anyIdx, enabledIdx + 1).join('\n');
const jsTerms = {};
for (const raw of anyBlock.replace(/\n/g, ' ').split('||')) {
  const term = raw.trim();
  const tm = term.match(/>\s*(0\.\d+)/);
  const nm = term.match(/s\.(\w+)/);
  if (tm && nm) jsTerms[nm[1]] = Number(tm[1]);
}
const jsThresholds = [...new Set(Object.values(jsTerms))].sort((a, b) => a - b);
eq(doc.composite.pass_thresholds.join(','), jsThresholds.join(','), 'pass thresholds');
const docTerms = doc.composite.pass_terms;
for (const key of Object.keys(jsTerms)) {
  near(docTerms[key], jsTerms[key], `pass term ${key}`);
}
eq(Object.keys(docTerms).length, Object.keys(jsTerms).length, 'pass term count');

// decay / heartbeat / hurt.
const d = doc.decay;
eq(d.rate, Number(find(sfx, 'Math.exp(-dt * 6)', 'decay').match(/\* (\d+)\)/)[1]), 'decay rate');
eq(d.low_k_denom, Number(find(sfx, 'const lowK =', 'lowK').match(/\(0\.5 - hp\) \/ ([\d.]+)/)[1]), 'lowK denom');
const bpm = find(sfx, 'const bpm =', 'bpm').match(/= (\d+) \+ (\d+) \* lowK/).slice(1).map(Number);
eq(d.bpm_base, bpm[0], 'bpm base');
eq(d.bpm_scale, bpm[1], 'bpm scale');
eq(d.hurt_damp, Number(find(sfx, 's.hurt = damp(', 'hurt damp').match(/lowK, (\d+), dt/)[1]), 'hurt damp');
eq(d.heart_damp, Number(find(sfx, 's.heart = damp(', 'heart damp').match(/s\.heart, 0, (\d+), dt/)[1]), 'heart damp');
eq(d.damage_window, Number(find(sfx, 's.dmgT = 0.06', 'damage window').match(/s\.dmgT = ([\d.]+)/)[1]), 'damage window');

// damage-number pool.
const bn = doc.damage_number;
eq(bn.combo_window, Number(find(boss, 'now - c.last > 0.45', 'combo').match(/> ([\d.]+)/)[1]), 'combo window');
eq(bn.life, Number(find(boss, 'age > 1.05', 'life').match(/> ([\d.]+)/)[1]), 'life');
eq(bn.out_after, Number(find(boss, 'age > 0.45 && !c.out', 'out').match(/> ([\d.]+)/)[1]), 'out after');
eq(bn.pool_cap, Number(find(boss, 'S.nums.length > 6', 'cap').match(/> (\d+)/)[1]), 'pool cap');
eq(bn.weak_scale_bonus, Number(find(boss, 'c.weak ? 0.2 : 0', 'weak').match(/\? ([\d.]+)/)[1]), 'weak bonus');
eq(bn.crit_ms, Number(find(boss, 'el.remove(), 900', 'crit').match(/, (\d+)\)/)[1]), 'crit ms');

// minimap dots / death marks / FX.
const mm = doc.minimap;
eq(mm.dot_count, Number(find(hud, 'this.mapDots = Array.from', 'dots').match(/length: (\d+)/)[1]), 'dot count');
eq(mm.death_mark_count, Number(find(hud, 'this.mapDms = Array.from', 'deaths').match(/length: (\d+)/)[1]), 'death mark count');
eq(mm.fx_cap, Number(find(mini, 'fxList.length > 40', 'fx cap').match(/> (\d+)/)[1]), 'fx cap');
for (const [key, needle] of [
  ['slam', "kind: 'slam'"], ['boom', "kind: 'boom'"],
  ['jump', "kind: 'jump'"], ['spawn', "kind: 'spawn'"],
]) {
  eq(mm.fx_life[key], Number(find(mini, needle, `fx ${key}`).match(/life: ([\d.]+)/)[1]), `fx ${key} life`);
}
eq(mm.fx_life.land, Number(find(mini, 'f.life = Math.min', 'fx land').match(/\+ ([\d.]+)/)[1]), 'fx land life');

console.log(`screenfx crosscheck: OK (${checks} assertions, commit ${doc.source.commit.slice(0, 8)})`);
