// Stage layout extractor (Task 3 / TR-3.3, TR-3.4; Task 37 §1 generalised to
// every extracted stage): `node extract-layout.mjs [stage]`, stage defaults to
// 'tidewater' and must be one of the STAGES below.
//
// Imports the evaluated upstream layout (mapkit B/R/O/OCT/ARC + each stage's
// own helpers have already produced plain box/obox/ramp defs), performs the
// half -> 180 deg mirror expansion that level.js does at runtime, and emits
// the raw primitive description the Rust collision world (Task 4) is built
// from:
//   rust/assets/maps/<stage>.json
//
// mirrorDef is replicated verbatim from src/world/level.js (that module imports
// three, which the extractor must not load); keep it in lockstep on upstream
// sync. The JS tree is never modified (AC-1/TR-2.4). Output is deterministic
// (stableStringify): same upstream commit => byte-identical JSON.
//
// Task 36 §1: Zone Control zones ride the layout JSON. The merge follows
// src/world/maps.js L118-119 — a layout's own `zones` wins, otherwise the
// hand-drawn ZONE_DEFS entry (src/world/zones-data.js) is attached verbatim
// (poly/polys/y0/y1, coordinates untouched). Stages without zones get no key.
//
// Task 37 §1: variant flags ride every primitive (`onlyIn`/`notIn`, the
// src/world/variants.js L2-3 mode tags) and meta.variantCounts echoes the
// per-mode primitive counts the Rust for_mode filter must reproduce.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import { PLAYER } from '../../../src/config.js';
import { ZONE_DEFS } from '../../../src/world/zones-data.js';
import { stableStringify } from './stable-json.mjs';

const here = dirname(fileURLToPath(import.meta.url));      // rust/tools/extract
const repoRoot = resolve(here, '../../..');

// Per-stage reserved PATTERN slots (each stage's surfaces.js SURF map:
// tidewater surfaces.js L19, kelpline surfaces.js L17, saltpan surfaces.js L17,
// crossmarket surfaces.js L16, lockgate surfaces.js L15, terraces surfaces.js L21,
// cargo surfaces.js L18). Halyard has no surfaces.js — it draws only on the
// shared PATTERN 0-27, so it reserves no slots.
const STAGES = {
  tidewater: { herringbone: 28, terrazzo: 29, stucco: 30 },
  kelpline: { tarmac: 31, quay: 32, chequer: 33 },
  saltpan: { salt: 34, mud: 35, timber: 36 },
  crossmarket: { setts: 37, ashlar: 38, tramway: 39 },
  lockgate: { setts: 40, engbrick: 41, hoofsteps: 42 },
  terraces: { calce: 43, cotto: 44, pebble: 45 },
  cargo: { tarmac: 46, quay: 47, chequer: 48 },
  halyard: {},
};

const stage = process.argv[2] ?? 'tidewater';
if (!(stage in STAGES)) {
  console.error(
    `extract-layout: unknown stage "${stage}" (expected one of: ${Object.keys(STAGES).join(', ')})`,
  );
  process.exit(1);
}
// Halyard is the one stage with no stages/<id>/ folder: its layout object is the
// HALYARD export of src/world/maps.js (L30-114, imports mapkit/stages-index/
// zones-data only — no three, safe to load under node). Its `intro`/`decor`/
// `water` fields are presentation-only and do not ride the v1 schema.
let L;
let sourcePath;
if (stage === 'halyard') {
  ({ HALYARD: L } = await import('../../../src/world/maps.js'));
  sourcePath = 'src/world/maps.js';
} else {
  ({ LAYOUT: L } = await import(`../../../src/world/stages/${stage}/layout.js`));
  sourcePath = `src/world/stages/${stage}/layout.js`;
}
const outPath = resolve(here, `../../assets/maps/${stage}.json`);

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
    // variant mode tags (src/world/variants.js L2-3); mirrorDef's `...d` spread
    // carries them onto the mirrored twin, so flags travel with each piece.
    // `||` (not `??`) matches the JS truthiness in inMode (variants.js L6):
    // an empty-string tag is absent, and the Rust Option<str> port never sees `Some("")`.
    onlyIn: d.onlyIn || null,
    notIn: d.notIn || null,
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

// maps.js L118-119 merge: a layout that already carries `zones` wins; otherwise
// attach the hand-drawn ZONE_DEFS entry for this stage verbatim.
const zones = L.zones ?? ZONE_DEFS[L.id];
const mirrored = L.half.map(mirrorDef);
// defs order matches level.js _build exactly (block ids are positional).
const primitives = [...L.single, ...L.half, ...mirrored].map(normBrush);
const countKind = (kind) => primitives.filter((p) => p.kind === kind).length;

// src/world/variants.js L6 `inMode` — verbatim port; the runtime applies it
// through layoutFor (variants.js L16-18) when main.js L242 builds the level.
const inMode = (it, mode) => (!it.onlyIn || it.onlyIn === mode) && (!it.notIn || it.notIn !== mode);
// per-mode counts over the union list (what Rust StageLayout::for_mode filters).
const variantCounts = (mode) => {
  const list = primitives.filter((p) => inMode(p, mode));
  const ck = (kind) => list.filter((p) => p.kind === kind).length;
  return { box: ck('box'), obox: ck('obox'), ramp: ck('ramp'), total: list.length };
};

const doc = {
  schema: 'inkwave.stage_layout.v1',
  source: { commit, path: sourcePath },
  stage: {
    id: L.id,
    bounds: L.bounds,
    spawnPads: L.spawnPads,
    spawnBarrier: L.spawnBarrier,
    waterY: PLAYER.waterY,
  },
  primitives,
  // Zone Control zones (zones-data.js L14-51 via the maps.js L118-119 merge);
  // stages without a zone entry simply get no `zones` key.
  ...(zones ? { zones } : {}),
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
    // This stage's reserved PATTERN slots (surfaces.js SURF).
    surfaceSlots: STAGES[stage],
    // per-mode primitive counts (variants.js L6 filter; Rust for_mode cross-check).
    variantCounts: { turf: variantCounts('turf'), zones: variantCounts('zones') },
  },
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, stableStringify(doc));
const vc = doc.meta.variantCounts;
console.log(`maps/${stage}.json <- ${L.id} layout @ ${commit}: ${primitives.length} primitives `
  + `(${countKind('box')} box, ${countKind('obox')} obox, ${countKind('ramp')} ramp); `
  + `turf ${vc.turf.total}, zones ${vc.zones.total}`);
