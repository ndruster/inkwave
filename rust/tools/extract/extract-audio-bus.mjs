// Task M5-T63: extract the audio bus topology / dynamics / spatial / IR
// constants from the READ-ONLY JS tree. No upstream file is modified.
// The crosscheck (`tools/audio-bus-crosscheck.mjs`) re-parses the same sources
// with a different code path; any upstream drift fails the gate.
import { upstreamCommit } from './upstream-commit.mjs';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));       // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/audio/bus.json');
const commit = upstreamCommit(repoRoot, [
  'src/audio/audio.js',
  'src/audio/music.js',
]);

const audio = readFileSync(resolve(repoRoot, 'src/audio/audio.js'), 'utf8');
const music = readFileSync(resolve(repoRoot, 'src/audio/music.js'), 'utf8');

function match(src, re, label) {
  const m = re.exec(src);
  if (!m) throw new Error(`audio-bus: ${label} not found (upstream drift?)`);
  return m;
}
const num = (src, re, label) => Number(match(src, re, label)[1]);
const text = (src, re, label) => match(src, re, label)[1];

// ---- topology (audio.js L113-139) ----------------------------------------
const initBlock = match(audio, /const g = \(v\) => \{[\s\S]*?this\.ready = true;/, 'engine.init block')[0];
const revSend = num(initBlock, /this\.revSend = g\(([\d.]+)\)/, 'revSend');
const revRet = num(initBlock, /this\.revRet = g\(([\d.]+)\)/, 'revRet');
const sfxIn = num(initBlock, /this\.sfxIn = g\(([\d.]+)\)/, 'sfxIn');
const hpFreq = num(initBlock, /hp\.frequency\.value = ([\d.]+)/, 'highpass.frequency');
const hpQ = num(initBlock, /hp\.Q\.value = ([\d.]+)/, 'highpass.Q');

function compressor(prefix, label) {
  return {
    threshold: num(initBlock, new RegExp(`${prefix}\\.threshold\\.value = (-?[\\d.]+)`), `${label}.threshold`),
    knee: num(initBlock, new RegExp(`${prefix}\\.knee\\.value = (-?[\\d.]+)`), `${label}.knee`),
    ratio: num(initBlock, new RegExp(`${prefix}\\.ratio\\.value = (-?[\\d.]+)`), `${label}.ratio`),
    attack: num(initBlock, new RegExp(`${prefix}\\.attack\\.value = (-?[\\d.]+)`), `${label}.attack`),
    release: num(initBlock, new RegExp(`${prefix}\\.release\\.value = (-?[\\d.]+)`), `${label}.release`),
  };
}
const glue = compressor('glue', 'glue');
const limiter = compressor('lim', 'limiter');

const edges = [
  ['sfxIn', 'sfxBus'], ['sfxBus', 'master'],
  ['musicBus', 'duck'], ['duck', 'master'],
  ['revSend', 'conv'], ['conv', 'revRet'], ['revRet', 'sfxBus'],
  ['master', 'highpass'], ['highpass', 'glue'], ['glue', 'limiter'], ['limiter', 'destination'],
];
const connectChecks = [
  '.connect(this.sfxBus)', '.connect(this.master)', '.connect(this.duckG)',
  '.connect(this.revSend)', '.connect(this.conv)', '.connect(this.revRet)',
  '.connect(hp)', '.connect(glue)', '.connect(lim)', '.connect(ctx.destination)',
];
for (const needle of connectChecks) {
  if (!audio.includes(needle)) throw new Error(`audio-bus: missing connect ${needle}`);
}

// ---- spatial / send (audio.js L28-30, L174-241) --------------------------
const taperExponent = num(audio, /const taper = \(v\) => Math\.pow\([\s\S]*?,\s*([\d.]+)\);/, 'taper exponent');
const distCutLine = match(audio, /^const distCut = .*$/m, 'distCut line')[0];
const distCutNear = num(distCutLine, /d < ([\d.]+)/, 'distCut.near');
const distCutCeiling = num(distCutLine, /\? ([\d.]+)/, 'distCut.ceiling');
const distCutFloor = num(distCutLine, /Math\.max\(([\d.]+),/, 'distCut.floor');
const distCutHalf = num(distCutLine, /\(d - [\d.]+\) \/ ([\d.]+)/, 'distCut.halfLife');

const pannerRef = num(audio, /_panner\(pos, ref = ([\d.]+)\)/, 'panner.ref');
const pannerRolloff = num(audio, /p\.rolloffFactor = ([\d.]+)/, 'panner.rolloff');
const pannerMaxMin = num(audio, /p\.maxDistance = Math\.max\((\d+), ref \* \d+\)/, 'panner.maxMin');
const pannerMaxMul = num(audio, /p\.maxDistance = Math\.max\(\d+, ref \* (\d+)\)/, 'panner.maxMul');
const coneInner = num(audio, /p\.coneInnerAngle = (\d+)/, 'panner.coneInner');
const coneOuter = num(audio, /p\.coneOuterAngle = (\d+)/, 'panner.coneOuter');
const modelLine = match(audio, /^\s*p\.panningModel = .*$/m, 'panner.model line')[0];
const model = match(modelLine, /'([^']+)' : '([^']+)'/, 'panner.model');

const send = match(audio, /_sendScale\(d\) \{ return Math\.sqrt\(([\d.]+) \/ \(([\d.]+) \+ ([\d.]+) \* Math\.max\(0, d - ([\d.]+)\)\)\); \}/, '_sendScale');
const revExtraLine = match(audio, /^\s*voice\.rev \+= Math\.min\(.*$/m, 'reverb distance line')[0];
const revExtraCap = num(revExtraLine, /Math\.min\(([\d.]+)/, 'reverb extra cap');
const revExtraStart = num(revExtraLine, /\(d - ([\d.]+)\)/, 'reverb extra start');
const revExtraDiv = num(revExtraLine, /\(d - [\d.]+\) \/ ([\d.]+)\)/, 'reverb extra divisor');
const defaultReverb = num(audio, /rev: def\.reverb \?\? ([\d.]+)/, 'default reverb');
const lpSmooth = num(audio, /voice\.lp\.frequency\.setTargetAtTime\([^,]+, t, ([\d.]+)\)/, 'lp smoothing');
const sendSmooth = num(audio, /voice\.send\.gain\.setTargetAtTime\([^,]+, t, ([\d.]+)\)/, 'send smoothing');
const listenerSmooth = num(audio, /l\.positionX\.setTargetAtTime\([^,]+, t, ([\d.]+)\)/, 'listener smoothing');

// ---- plaza IR (audio.js L124, music.js L98-122) --------------------------
const irCall = match(audio, /makeImpulse\(ctx, ([\d.]+), ([\d.]+), \{ pre: ([\d.]+), seed: (\d+) \}\)/, 'plaza IR call');
const irBlock = match(music, /export function makeImpulse\(ctx, seconds = [\d.]+, decay = [\d.]+, opt = \{\}\) \{[\s\S]*?\n\}/, 'makeImpulse')[0];
const bright = num(irBlock, /bright = opt\.bright \?\? ([\d.]+)/, 'IR bright');
const dark = num(irBlock, /dark = opt\.dark \?\? ([\d.]+)/, 'IR dark');
const envDecayMul = num(irBlock, /Math\.exp\(-x \* decay \* ([\d.]+)\)/, 'IR decay multiplier');
const attackSec = num(irBlock, /\(i - pre\) \/ \(([\d.]+) \* sr\)/, 'IR attack');
const darkenRate = num(irBlock, /Math\.exp\(-x \* ([\d.]+)\)/, 'IR darken rate');
const channelSeedStep = num(irBlock, /\(opt\.seed \?\? \d+\) \+ ch \* (\d+)/, 'IR channel seed step');
const tapsSource = text(irBlock, /const taps = opt\.taps \?\? (\[\[[\s\S]*?\]\]);/, 'IR taps');
const taps = [...tapsSource.matchAll(/\[\s*([\d.]+)\s*,\s*([\d.]+)\s*\]/g)]
  .map((m) => [Number(m[1]), Number(m[2])]);
if (taps.length !== 5) throw new Error(`audio-bus: expected 5 IR taps, got ${taps.length}`);
const tapTailGain = num(irBlock, /d\[i \+ 1\] \+= amp \* ([\d.]+)/, 'IR tap tail gain');
const rightOffset = num(irBlock, /\(tt \+ \(ch \? ([\d.]+) : 0\)\) \* sr/, 'IR right-ear offset');

const doc = {
  schema: 1,
  source: { commit, files: ['src/audio/audio.js', 'src/audio/music.js'] },
  graph: {
    taper_exponent: taperExponent,
    sfx_in: sfxIn,
    rev_send: revSend,
    rev_return: revRet,
    highpass: { frequency: hpFreq, q: hpQ },
    glue,
    limiter,
    edges: edges.map(([from, to]) => ({ from, to })),
  },
  spatial: {
    panning_model: model[1],
    fallback_model: model[2],
    distance_model: 'inverse',
    ref_distance: pannerRef,
    max_distance_min: pannerMaxMin,
    max_distance_factor: pannerMaxMul,
    rolloff: pannerRolloff,
    cone_inner: coneInner,
    cone_outer: coneOuter,
    dist_cut: { near: distCutNear, ceiling: distCutCeiling, floor: distCutFloor, half_life: distCutHalf, q: 0.5 },
    send: { numerator: Number(send[1]), base: Number(send[2]), rolloff: Number(send[3]), ref: Number(send[4]) },
    reverb_distance: { cap: revExtraCap, start: revExtraStart, divisor: revExtraDiv },
    default_reverb: defaultReverb,
    smoothing: { listener: listenerSmooth, lowpass: lpSmooth, send: sendSmooth },
  },
  impulse: {
    seconds: Number(irCall[1]),
    decay: Number(irCall[2]),
    pre: Number(irCall[3]),
    seed: Number(irCall[4]),
    bright,
    dark,
    decay_multiplier: envDecayMul,
    attack_seconds: attackSec,
    darken_rate: darkenRate,
    channel_seed_step: channelSeedStep,
    taps: taps.map(([t, a]) => ({ time: t, amp: a })),
    tap_tail_gain: tapTailGain,
    right_offset: rightOffset,
  },
};
const out = stableStringify(doc) + '\n';
if (/\bNaN\b|\bInfinity\b/.test(out)) throw new Error('bus.json contains NaN/Infinity');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, out);
console.log(`wrote ${outPath}: ${(out.length / 1024).toFixed(1)} KB, edges=${edges.length}, ir_taps=${taps.length}`);
