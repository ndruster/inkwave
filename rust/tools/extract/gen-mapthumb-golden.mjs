// Task 53 (D-T53-5) mapThumb golden fixtures: run the REAL src/world/mapThumb.js
// over all eight stages × three themes and freeze the SVG strings into
// rust/assets/props/thumb/<stage>-<theme>.svg for the Rust byte-compare test.
// Deterministic (pure data → string; no DOM, no three.js). The JS tree is read,
// never modified.
//
//   node gen-mapthumb-golden.mjs
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));        // rust/tools/extract
const repoRoot = resolve(here, '../../..');
const outDir = resolve(here, '../../assets/props/thumb');

const { layoutThumbSVG } = await import(resolve(repoRoot, 'src/world/mapThumb.js'));
const { MAP_LAYOUTS } = await import(resolve(repoRoot, 'src/world/maps.js'));

const STAGES = ['tidewater', 'kelpline', 'halyard', 'saltpan', 'crossmarket', 'lockgate', 'terraces', 'cargo'];
const THEMES = ['day', 'sunset', 'golden'];

mkdirSync(outDir, { recursive: true });
for (const stage of STAGES) {
  const layout = MAP_LAYOUTS[stage];
  if (!layout) throw new Error(`gen-mapthumb-golden: no MAP_LAYOUTS[${stage}]`);
  for (const theme of THEMES) {
    const svg = layoutThumbSVG(layout, theme);
    writeFileSync(resolve(outDir, `${stage}-${theme}.svg`), svg);
    console.log(`gen-mapthumb-golden ${stage}/${theme}: ${svg.length} chars`);
  }
}
