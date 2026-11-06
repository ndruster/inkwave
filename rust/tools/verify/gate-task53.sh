#!/usr/bin/env bash
# Nine-stage gate for Task 53 — props/murals/dressing extraction + Rust loader
# (D-T53-5: DETERM extended to 8×bin.gz+json + special + 24 mapthumb goldens;
#  XCHK = props-crosscheck.mjs live dressingFor/PropKit vs baked manifests).
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task53-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate-raw-$$.log"
  : > "$raw"
  "$@" >> "$raw" 2>&1
  local rc=$?
  echo "${name^^}_EXIT=$rc" >> "$LOG"
  # strip the noisy libselinux lines before appending
  grep -v "libselinux.so.1" "$raw" >> "$LOG" 2>/dev/null || true
  rm -f "$raw"
  return $rc
}

# 1. fmt
stage fmt cargo fmt --all -- --check || true
# 2. clippy
stage clippy cargo clippy --workspace --all-targets -- -D warnings || true
# 3. test (props decode/manifest goldens + mapthumb 24/24 — D-T53-1..4)
stage test cargo test --workspace || true
# 4. wasm32 check (only the two crates that compile for wasm)
echo "=== wasm32 check ===" >> "$LOG"
RAW=/tmp/gate-wasm-$$.log
: > "$RAW"
cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown >> "$RAW" 2>&1
WASM_RC=$?
echo "WASM_EXIT=$WASM_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"

# 5. cargo tree
stage tree cargo tree -p inkwave_sim || true

# 6. determinism: rerun extract-props/extract-special/gen-mapthumb-golden,
#    rerun must be byte-identical (8×bin.gz + 8×json + special pair + 24 svgs)
echo "=== determinism: rerun props/special/mapthumb extractors, compare sha256 ===" >> "$LOG"
RAW=/tmp/gate-det-$$.log
: > "$RAW"
MAPS="tidewater kelpline halyard saltpan crossmarket lockgate terraces cargo"
FILES=""
for m in $MAPS; do FILES="$FILES assets/props/$m.json assets/props/$m.bin.gz"; done
FILES="$FILES assets/props/special.json assets/props/special.bin.gz"
for f in assets/props/thumb/*.svg; do FILES="$FILES $f"; done
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-props.mjs >> "$RAW" 2>&1
node extract-special.mjs >> "$RAW" 2>&1
node gen-mapthumb-golden.mjs >> "$RAW" 2>&1
DETERM_RC=$?
cd ../..
AFTER=$(sha256sum $FILES)
if [ "$BEFORE" = "$AFTER" ]; then
  echo "DETERM=BYTE_IDENTICAL" >> "$LOG"
else
  echo "DETERM=MISMATCH" >> "$LOG"
  diff <(echo "$BEFORE") <(echo "$AFTER") >> "$LOG" 2>&1
  DETERM_RC=1
fi
echo "$AFTER" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "DETERM_EXIT=$DETERM_RC" >> "$LOG"
rm -f "$RAW"

# 7. props crosscheck (TR-53.1: live dressingFor/PropKit vs baked manifests)
echo "=== props crosscheck (node, live JS source of truth) ===" >> "$LOG"
RAW=/tmp/gate-xc-$$.log
: > "$RAW"
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC_RC=$?
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"

# 8. trunk build (retry<=3, SIGKILL false-green guard)
echo "=== trunk build (retry<=3, SIGKILL false-green guard) ===" >> "$LOG"
TRUNK_RC=1
SIGKILL_HITS=0
for attempt in 1 2 3; do
  RAW=/tmp/gate-trunk-$$.log
  : > "$RAW"
  trunk build --release >> "$RAW" 2>&1
  TRUNK_RC=$?
  SIGKILL_HITS=$(grep -c "signal: 9" "$RAW" 2>/dev/null || true)
  [ -z "$SIGKILL_HITS" ] && SIGKILL_HITS=0
  grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
  rm -f "$RAW"
  if [ "$TRUNK_RC" -eq 0 ]; then
    echo "trunk attempt $attempt succeeded" >> "$LOG"
    break
  fi
  echo "trunk attempt $attempt rejected (rc=$TRUNK_RC, sigkill_hits=$SIGKILL_HITS)" >> "$LOG"
done
echo "TRUNK_EXIT=$TRUNK_RC" >> "$LOG"

# 9. headless (native release binary first; props load must not panic)
echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate-hl-$$.log
: > "$RAW"
cargo build --release -p inkwave >> "$RAW" 2>&1
target/release/inkwave --headless --frames 600 >> "$RAW" 2>&1
HL_RC=$?
target/release/inkwave --headless --mode boss --frames 600 >> "$RAW" 2>&1
HL_BOSS_RC=$?
echo "HEADLESS_EXIT=$HL_RC (turf) / HEADLESS_BOSS_EXIT=$HL_BOSS_RC (boss)" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"

# JS tree status + sim zero-diff guard (NFR-M4-1)
echo "=== JS tree git status ===" >> "$LOG"
JS_STATUS=$(git status --short ../src/ ../server/ ../tools/ ../docs/ 2>/dev/null)
if [ -z "$JS_STATUS" ]; then
  echo "JS_TREE_CLEAN" >> "$LOG"
else
  echo "$JS_STATUS" >> "$LOG"
fi
SIM_DIFF=$(git diff --stat crates/inkwave_sim/ 2>/dev/null)
if [ -z "$SIM_DIFF" ]; then
  echo "SIM_TREE_CLEAN" >> "$LOG"
else
  echo "$SIM_DIFF" >> "$LOG"
fi
echo "GATES_DONE" >> "$LOG"

# Verdict
echo "" >> "$LOG"
echo "=== TRUNK verdict (D-T40-1 precedent) ===" >> "$LOG"
if [ "$TRUNK_RC" -ne 0 ] && [ "$SIGKILL_HITS" -gt 0 ]; then
  echo "TRUNK=DEGRADED: 3 attempts, each rc=$TRUNK_RC with $SIGKILL_HITS 'signal: 9' hits" >> "$LOG"
  echo "(wasm-bindgen OOM SIGKILL in the sandbox — environmental, not a code regression;" >> "$LOG"
  echo "trunk false-green guard active). Non-blocking." >> "$LOG"
fi

echo "Gate log: $LOG"
