// Task M5-T63: independent crosscheck of assets/audio/bus.json.
// This file intentionally does NOT import extract-audio-bus.mjs: values are
// re-parsed from the READ-ONLY JS sources with a second, line-oriented path.
import { upstreamCommit } from './extract/upstream-commit.mjs';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));                  // rust/tools
const repoRoot = resolve(here, '../..');
const doc = JSON.parse(readFileSync(resolve(here, '../assets/audio/bus.json'), 'utf8'));
const audioLines = readFileSync(resolve(repoRoot, 'src/audio/audio.js'), 'utf8').split('\n');
const musicLines = readFileSync(resolve(repoRoot, 'src/audio/music.js'), 'utf8').split('\n');

let checks = 0;
function eq(got, want, label) {
  checks += 1;
  if (got !== want) throw new Error(`bus crosscheck: ${label}: got ${got}, JS=${want}`);
}
function near(got, want, label) {
  checks += 1;
  if (Math.abs(got - want) > 1e-9) throw new Error(`bus crosscheck: ${label}: got ${got}, JS=${want}`);
}
function line(lines, needle, label) {
  const hit = lines.find((l) => l.includes(needle));
  if (!hit) throw new Error(`bus crosscheck: missing JS line for ${label} (${needle})`);
  return hit;
}
function lastNumber(s) {
  const all = s.match(/-?\d+(?:\.\d+)?/g);
  if (!all) throw new Error(`bus crosscheck: no number in ${s}`);
  return Number(all.at(-1));
}
function afterEq(s, prop) {
  const re = prop ? new RegExp(`${prop}\\s*=\\s*(-?\\d+(?:\\.\\d+)?)`) : /=\s*(-?\d+(?:\.\d+)?)/;
  const m = s.match(re);
  if (!m) throw new Error(`bus crosscheck: no assignment in ${s}`);
  return Number(m[1]);
}
function quoted(s, index = 0) {
  const all = [...s.matchAll(/'([^']+)'/g)];
  return all[index]?.[1];
}

// Source pin.
eq(doc.source.commit, upstreamCommit(repoRoot, ['src/audio/audio.js', 'src/audio/music.js']), 'source.commit');
eq(doc.source.files.join(','), 'src/audio/audio.js,src/audio/music.js', 'source.files');

// Topology and gains (audio.js L113-139).
eq(doc.graph.edges.length, 11, 'edge count');
for (const connect of [
  '.connect(this.sfxBus)', '.connect(this.master)', '.connect(this.duckG)',
  '.connect(this.revSend)', '.connect(this.conv)', '.connect(this.revRet)',
  '.connect(hp)', '.connect(glue)', '.connect(lim)', '.connect(ctx.destination)',
  '.connect(this.revRet)',
]) {
  if (!audioLines.some((l) => l.includes(connect))) {
    throw new Error(`bus crosscheck: missing JS connect ${connect}`);
  }
}
near(doc.graph.rev_send, lastNumber(line(audioLines, 'this.revSend = g(', 'revSend')), 'revSend');
near(doc.graph.rev_return, lastNumber(line(audioLines, 'this.revRet = g(', 'revRet')), 'revRet');
near(doc.graph.sfx_in, lastNumber(line(audioLines, 'this.sfxIn = g(', 'sfxIn')), 'sfxIn');
near(doc.graph.taper_exponent, lastNumber(line(audioLines, 'const taper = ', 'taper')), 'taper');
const hpLine = line(audioLines, 'hp.frequency.value', 'highpass params');
near(doc.graph.highpass.frequency, afterEq(hpLine, 'hp\\.frequency\\.value'), 'hp.frequency');
near(doc.graph.highpass.q, afterEq(hpLine, 'hp\\.Q\\.value'), 'hp.q');
for (const [key, prefix] of [['glue', 'glue'], ['limiter', 'lim']]) {
  for (const prop of ['threshold', 'knee', 'ratio', 'attack', 'release']) {
    const js = afterEq(
      line(audioLines, `${prefix}.${prop}.value`, `${key}.${prop}`),
      `${prefix}\\.${prop}\\.value`,
    );
    near(doc.graph[key][prop], js, `${key}.${prop}`);
  }
}

// Listener / panner / send (audio.js L28-30, L174-241).
const distCut = line(audioLines, 'const distCut = ', 'distCut');
near(doc.spatial.dist_cut.near, Number(distCut.match(/d < ([\d.]+)/)?.[1]), 'distCut.near');
near(doc.spatial.dist_cut.ceiling, Number(distCut.match(/\? ([\d.]+)/)?.[1]), 'distCut.ceiling');
near(doc.spatial.dist_cut.floor, Number(distCut.match(/Math\.max\(([\d.]+)/)?.[1]), 'distCut.floor');
near(doc.spatial.dist_cut.half_life, Number(distCut.match(/\(d - [\d.]+\) \/ ([\d.]+)/)?.[1]), 'distCut.halfLife');
eq(doc.spatial.panning_model, quoted(line(audioLines, 'p.panningModel = ', 'panner model'), 0), 'HRTF model');
eq(doc.spatial.fallback_model, quoted(line(audioLines, 'p.panningModel = ', 'panner model'), 1), 'fallback model');
eq(doc.spatial.distance_model, 'inverse', 'distance model');
near(doc.spatial.ref_distance, lastNumber(line(audioLines, '_panner(pos, ref = ', 'panner ref')), 'panner ref');
near(doc.spatial.rolloff, lastNumber(line(audioLines, 'p.rolloffFactor = ', 'rolloff')), 'rolloff');
const maxLine = line(audioLines, 'p.maxDistance = Math.max(', 'max distance');
near(doc.spatial.max_distance_min, Number(maxLine.match(/Math\.max\((\d+), ref \* (\d+)\)/)?.[1]), 'max distance min');
near(doc.spatial.max_distance_factor, Number(maxLine.match(/Math\.max\((\d+), ref \* (\d+)\)/)?.[2]), 'max distance factor');
const sendLine = line(audioLines, '_sendScale(d) { return Math.sqrt(', 'sendScale');
const sendMatch = sendLine.match(/Math\.sqrt\(([\d.]+) \/ \(([\d.]+) \+ ([\d.]+) \* Math\.max\(0, d - ([\d.]+)\)\)\)/);
near(doc.spatial.send.numerator, Number(sendMatch[1]), 'send.numerator');
near(doc.spatial.send.base, Number(sendMatch[2]), 'send.base');
near(doc.spatial.send.rolloff, Number(sendMatch[3]), 'send.rolloff');
near(doc.spatial.send.ref, Number(sendMatch[4]), 'send.ref');
const revLine = line(audioLines, 'voice.rev += Math.min(', 'reverb distance');
const revMatch = revLine.match(/Math\.min\(([\d.]+), Math\.max\(0, \(d - ([\d.]+)\) \/ ([\d.]+)\)\)/);
near(doc.spatial.reverb_distance.cap, Number(revMatch[1]), 'reverb cap');
near(doc.spatial.reverb_distance.start, Number(revMatch[2]), 'reverb start');
near(doc.spatial.reverb_distance.divisor, Number(revMatch[3]), 'reverb divisor');
near(doc.spatial.default_reverb, lastNumber(line(audioLines, 'rev: def.reverb ?? ', 'default reverb')), 'default reverb');

// IR descriptor (music.js L98-122, audio.js L124).
const irCall = line(audioLines, 'makeImpulse(ctx, ', 'IR call').match(/makeImpulse\(ctx, ([\d.]+), ([\d.]+), \{ pre: ([\d.]+), seed: (\d+)/);
near(doc.impulse.seconds, Number(irCall[1]), 'IR seconds');
near(doc.impulse.decay, Number(irCall[2]), 'IR decay');
near(doc.impulse.pre, Number(irCall[3]), 'IR pre');
eq(doc.impulse.seed, Number(irCall[4]), 'IR seed');
const impNum = (needle, re) => {
  const m = line(musicLines, needle, `makeImpulse ${needle}`).match(re);
  if (!m) throw new Error(`bus crosscheck: makeImpulse pattern failed for ${needle}`);
  return Number(m[1]);
};
near(doc.impulse.bright, impNum('bright = opt.bright ??', /bright = opt\.bright \?\? ([\d.]+)/), 'IR bright');
near(doc.impulse.dark, impNum('dark = opt.dark ??', /dark = opt\.dark \?\? ([\d.]+)/), 'IR dark');
near(doc.impulse.decay_multiplier, impNum('Math.exp(-x * decay *', /Math\.exp\(-x \* decay \* ([\d.]+)\)/), 'IR decay multiplier');
near(doc.impulse.attack_seconds, impNum('Math.min(1, (i - pre) / (', /\(i - pre\) \/ \(([\d.]+) \* sr\)/), 'IR attack');
near(doc.impulse.darken_rate, impNum('Math.exp(-x * 4)', /Math\.exp\(-x \* ([\d.]+)\)/), 'IR darken');
eq(doc.impulse.channel_seed_step, impNum(') + ch *', /\(opt\.seed \?\? \d+\) \+ ch \* (\d+)/), 'IR channel seed step');
const tapLine = musicLines.find((l) => l.includes('const taps = opt.taps ??'));
const tapPairs = [...tapLine.matchAll(/\[\s*([\d.]+)\s*,\s*([\d.]+)\s*\]/g)].map((m) => [Number(m[1]), Number(m[2])]);
eq(tapPairs.length, 5, 'IR tap count');
for (let i = 0; i < 5; i += 1) {
  near(doc.impulse.taps[i].time, tapPairs[i][0], `IR tap ${i} time`);
  near(doc.impulse.taps[i].amp, tapPairs[i][1], `IR tap ${i} amp`);
}
near(doc.impulse.tap_tail_gain, impNum('d[i + 1] += amp *', /d\[i \+ 1\] \+= amp \* ([\d.]+)/), 'IR tap tail');
near(doc.impulse.right_offset, impNum('ch ? 0.0031 : 0', /\(tt \+ \(ch \? ([\d.]+) : 0\)\) \* sr/), 'IR right offset');

console.log(`audio-bus crosscheck: OK (${checks} assertions, commit ${doc.source.commit.slice(0, 8)})`);
