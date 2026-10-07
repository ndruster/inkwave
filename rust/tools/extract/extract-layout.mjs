// Tidewater Plaza layout extractor (Task 3 / TR-3.3, TR-3.4).
//
// Imports the evaluated upstream layout (mapkit B/R/O/OCT/ARC + tidewater's own
// arcBand/arcSub/chainBand/chainSub helpers have already produced plain
// box/obox/ramp defs), performs the half -> 180 deg mirror expansion that
// level.js does at runtime, and emits the raw primitive description the Rust
// collision world (Task 4) is built from:
//   rust/assets/maps/tidewater.json
//
// mirrorDef is replicated verbatim from src/world/level.js (that module imports
// three, which the extractor must not load); keep it in lockstep on upstream
// sync. The JS tree is never modified (AC-1/TR-2.4). Output is deterministic
// (stableStringify): same upstream commit => byte-identical JSON.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import { LAYOUT } from '../../../src/world/stages/tidewater/layout.js';
import { PLAYER } from '../../../src/config.js';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));      // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outPath = resolve(here, '../../assets/maps/tidewater.json');

let commit;
try {
  commit = execSync('git rev-parse HEAD', { cwd: repoRoot, stdio: ['ignore', 'pipe', 'ignore'] })
    .toString().trim();
} catch {
  commit = 'unknown';
}

// --- level.js mirrorDef (180 deg rotation about the Y axis); keep identical.
function mirrorDef(d) {
  const mural = d.mural ? d.mural.map((m) => ({ ...m, n: [-m.n[0], m.n[1], -m.n[2]] })) : undefined;
  const noPaint = d.noPaint ? d.noPaint.map((m) => [-m[0], m[1], -m[2]]) : undefined;
  if (d.kind === 'box') {
    return { ...d, mural, noPaint, min: [-d.max[0], d.min[1], -d.max[2]], max: [-d.min[0], d.max[1], -d.min[2]] };
  }
  if (d.kind === 'obox') return { ...d, mural, noPaint, center: [-d.center[0], d.center[1], -d.center[2]] };
  return { ...d, mural, noPaint, low: [-d.low[0], d.low[1], -d.low[2]], high: [-d.high[0], d.high[1], -d.high[2]] };
}

// Normalise one raw def into the documented schema: every flag is explicit,
// defaults match level.js _addBlock so Rust never has to guess.
function normBrush(d) {
  const common = {
    tag: d.tag ?? null,
    color: d.color ?? '#dddddd',
    pattern: d.pattern ?? 0,
    paint: d.paint !== false && !d.grate && !d.rail,
    solid: d.solid !== false,
    // level.js: b.grate = !!d.grate || !!d.rail (rails are a collision-only grate subtype)
    grate: !!d.grate || !!d.rail,
    rail: !!d.rail,
    roof: !!d.roof,
    perch: !!d.perch,
    noNav: !!d.noNav,
    hidden: !!d.hidden || !!d.rail,
    bevel: d.bevel ?? null,
    noPaint: (d.noPaint ?? []).map((n) => [n[0], n[1], n[2]]),
    mural: (d.mural ?? []).map((m) => ({ id: m.id, n: [m.n[0], m.n[1], m.n[2]] })),
    oct: d.oct ? [d.oct[0], d.oct[1], d.oct[2]] : null,
  };
  if (d.kind === 'box') {
    return { kind: 'box', min: d.min, max: d.max, ...common };
  }
  if (d.kind === 'obox') {
    return { kind: 'obox', center: d.center, size: d.size, rotY: d.rotY, ...common };
  }
  return {
    kind: 'ramp', low: d.low, high: d.high, width: d.width,
    thickness: d.thickness ?? 0.6, thin: !!d.thin, ...common,
  };
}

const L = LAYOUT;
const mirrored = L.half.map(mirrorDef);
// defs order matches level.js _build exactly (block ids are positional).
const primitives = [...L.single, ...L.half, ...mirrored].map(normBrush);
const countKind = (kind) => primitives.filter((p) => p.kind === kind).length;

const doc = {
  schema: 'inkwave.stage_layout.v1',
  source: { commit, path: 'src/world/stages/tidewater/layout.js' },
  stage: {
    id: L.id,
    bounds: L.bounds,
    spawnPads: L.spawnPads,
    spawnBarrier: L.spawnBarrier,
    waterY: PLAYER.waterY,
  },
  primitives,
  meta: {
    // node-side statistics, cross-checked by the Rust parser (TR-3.3).
    primitiveCounts: {
      box: countKind('box'),
      obox: countKind('obox'),
      ramp: countKind('ramp'),
      total: primitives.length,
    },
    sourceCounts: {
      single: L.single.length,
      half: L.half.length,
      halfMirrored: mirrored.length,
    },
    bounds: L.bounds,
    spawnPads: L.spawnPads,
    spawnBarrier: L.spawnBarrier,
    waterY: PLAYER.waterY,
    // Tidewater's reserved PATTERN slots (surfaces.js).
    surfaceSlots: { herringbone: 28, terrazzo: 29, stucco: 30 },
  },
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(doc));
console.log(`maps/tidewater.json <- ${L.id} layout @ ${commit}: ${primitives.length} primitives `
  + `(${countKind('box')} box, ${countKind('obox')} obox, ${countKind('ramp')} ramp)`);
