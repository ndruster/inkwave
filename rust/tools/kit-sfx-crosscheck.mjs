// M5 Task 60 — independent kit-SFX crosscheck.
// The extractor imports the live ESM registry; this checker uses a SECOND,
// independent source-level scan: regexes over the 9 kit modules for
// `def('name', ...)` / `SFX.name = ...` registrations, compared against
// `assets/audio/kit-sfx.json`. No import of the extractor, no artifact writes.
// Usage: cd rust && node tools/kit-sfx-crosscheck.mjs
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..', '..');
const files = [
  'src/game/kits/brolly.js',
  'src/game/kits/bow.js',
  'src/game/kits/bow-sfx.js',
  'src/game/kits/blade.js',
  'src/game/kits/blade-fx.js',
  'src/game/kits/mitts.js',
  'src/game/kits/mitts-assets.js',
  'src/game/kits/shaker.js',
  'src/game/kits/waddle.js',
  'src/game/kits/torpedo.js',
  'src/game/kits/tracer.js',
  'src/game/kits/boomerang.js',
];
const found = new Set();
for (const f of files) {
  const s = fs.readFileSync(path.join(root, f), 'utf8');
  for (const m of s.matchAll(/\bdef\(\s*['"]([a-z0-9_]+)['"]\s*,/g)) found.add(m[1]);
  for (const m of s.matchAll(/\bSFX\.([a-z0-9_]+)\s*=\s*\{/g)) found.add(m[1]);
}
const doc = JSON.parse(fs.readFileSync(path.join(root, 'rust/assets/audio/kit-sfx.json'), 'utf8'));
const names = new Set(doc.cues.map((c) => c.name));
const missing = [...found].filter((n) => !names.has(n));
const extra = [...names].filter((n) => !found.has(n));
if (missing.length || extra.length) {
  throw new Error(`KIT_SFX_CROSSCHECK=FAIL missing=${missing.join(',')} extra=${extra.join(',')}`);
}
if (doc.cues.length !== 53) throw new Error(`expected 53 kit cues, got ${doc.cues.length}`);
console.log(`KIT_SFX_CROSSCHECK=PASS names=${doc.cues.length}`);
