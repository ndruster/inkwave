#!/usr/bin/env bash
# TR-4.1/4.2 evidence: build the Rust collision dump and diff it against an
# independent Python reimplementation of level.js/physics.js geometry.
# Exit 0 iff every ground height / ray hit / face id matches within 1 mm.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
DUMP="$(mktemp -t inkwave-collision-dump-XXXXXX.json)"
trap 'rm -f "$DUMP"' EXIT

cargo run --manifest-path "$RUST_DIR/Cargo.toml" --example collision_dump -p inkwave_sim --quiet > "$DUMP"
python3 "$HERE/collision_crosscheck.py" "$DUMP" "$RUST_DIR/assets/maps/tidewater.json"
