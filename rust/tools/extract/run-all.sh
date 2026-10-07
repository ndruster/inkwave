#!/usr/bin/env bash
# Regenerate every Rust-side data artifact from the upstream JS tree (Task 3).
# Deterministic: running twice against the same upstream commit must leave the
# working tree clean (TR-3.4). Requires node >= 20 (ESM, no dependencies).
set -euo pipefail
cd "$(dirname "$0")"
node extract-tuning.mjs
node extract-layout.mjs
