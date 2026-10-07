// Deterministic JSON serialisation for the INKWAVE data extraction pipeline:
// object keys are recursively sorted so re-running an extractor against the
// same upstream commit produces byte-identical output (TR-3.4). Arrays keep
// their order; -0 is normalised; no timestamps are ever emitted.
export function sortKeys(value) {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) {
      const v = value[key];
      if (v === undefined || typeof v === 'function') continue;
      out[key] = sortKeys(v === -0 ? 0 : v);
    }
    return out;
  }
  return value;
}

export function stableStringify(value, indent = 2) {
  return JSON.stringify(sortKeys(value), null, indent) + '\n';
}
