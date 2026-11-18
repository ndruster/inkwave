// Task 59 — independent settings crosscheck.
// The extractor (`extract-environment.mjs`) imports config.js to write
// `assets/environment.json:defaultSettings`. This checker uses a SECOND,
// independent parser (regex slices of the same source block, no import of
// the extractor) and compares scalar key/value pairs against the extracted
// document. It is intentionally read-only: no artifact is rewritten.
// Usage: cd rust && node tools/settings-crosscheck.mjs
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..', '..');
const src = fs.readFileSync(path.join(root, 'src/config.js'), 'utf8');
const doc = JSON.parse(fs.readFileSync(path.join(root, 'rust/assets/environment.json'), 'utf8'));
const m = src.match(/export const DEFAULT_SETTINGS\s*=\s*\{([\s\S]*?)\n\};/);
if (!m) throw new Error('DEFAULT_SETTINGS block not found in src/config.js');
const body = m[1];
const pairs = {};
for (const raw of body.split('\n')) {
  const line = raw.replace(/\/\/.*$/, '').trim();
  if (!line) continue;
  // Some upstream lines pack three keys (`master: 0.8, music: 0.6, ...`),
  // so scan key/value pairs instead of assuming one key per line.
  const re = /(\w+)\s*:\s*('[^']*'|"[^"]*"|[^,]+?)\s*(?:,|$)/g;
  let hit;
  let found = 0;
  while ((hit = re.exec(line)) !== null) {
    const value = hit[2].trim();
    // Numeric / boolean / quoted-string literals only — Function is a
    // literal evaluator, NOT a JS-module import (the extractor imports).
    pairs[hit[1]] = Function(`return (${value})`)();
    found++;
  }
  if (!found) throw new Error(`unparsed DEFAULT_SETTINGS line: ${raw}`);
}
const saved = doc.defaultSettings;
if (!saved) throw new Error('environment.json has no defaultSettings');
let checked = 0;
for (const [k, v] of Object.entries(pairs)) {
  if (!(k in saved)) throw new Error(`environment.json missing ${k}`);
  if (typeof v === 'object' && v !== null) continue; // nested tables: not scalars
  if (saved[k] !== v) {
    throw new Error(`DEFAULT_SETTINGS.${k}: upstream ${JSON.stringify(v)} != extracted ${JSON.stringify(saved[k])}`);
  }
  checked++;
}
// The extracted table must contain exactly the upstream scalar keys (21),
// nothing dropped and nothing invented.
if (checked !== 21) throw new Error(`expected 21 scalar keys, checked ${checked}`);
const extra = Object.keys(saved).filter(k => !(k in pairs));
if (extra.length) throw new Error(`environment.json has extra defaultSettings keys: ${extra}`);
console.log(`SETTINGS_CROSSCHECK=PASS keys=${checked}`);
