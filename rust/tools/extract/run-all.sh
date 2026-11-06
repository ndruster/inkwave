#!/usr/bin/env bash
# Regenerate every Rust-side data artifact from the upstream JS tree (Task 3).
# Deterministic: running twice against the same upstream commit must leave the
# working tree clean (TR-3.4). Requires node >= 20 (ESM, no dependencies).
set -euo pipefail
cd "$(dirname "$0")"
node extract-tuning.mjs
node extract-texlib.mjs
node extract-layout.mjs
node extract-layout.mjs kelpline
node extract-layout.mjs halyard
node extract-layout.mjs saltpan
node extract-layout.mjs crossmarket
node extract-layout.mjs lockgate
node extract-layout.mjs terraces
node extract-layout.mjs cargo
# props archives + manifests (Task 53 / D-T53-1: dressing bake + decor + lamps + murals)
for stage in tidewater kelpline halyard saltpan crossmarket lockgate terraces cargo; do
  node extract-props.mjs "$stage"
done
node extract-special.mjs
