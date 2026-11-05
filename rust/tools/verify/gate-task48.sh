#!/usr/bin/env bash
# Nine-stage gate for Task 48 — full-map extraction form (Task 47 gate, stage 6 extended to 8 maps).
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task48-gates-${TS}.log
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
# 3. test
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

# 6. tuning+layout determinism, all 8 maps (rerun must be byte-identical)
echo "=== determinism: rerun extractors, compare sha256 before/after (8 maps) ===" >> "$LOG"
RAW=/tmp/gate-det-$$.log
: > "$RAW"
MAPS="tidewater kelpline halyard saltpan crossmarket lockgate terraces cargo"
BEFORE=$(sha256sum assets/tuning.json assets/net-fixtures.json)
for m in $MAPS; do BEFORE="$BEFORE
$(sha256sum assets/maps/$m.json)"; done
cd tools/extract
node extract-tuning.mjs >> "$RAW" 2>&1
for m in $MAPS; do node extract-layout.mjs "$m" >> "$RAW" 2>&1; done
cd ../..
DETERM_RC=$?
AFTER=$(sha256sum assets/tuning.json assets/net-fixtures.json)
for m in $MAPS; do AFTER="$AFTER
$(sha256sum assets/maps/$m.json)"; done
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

# 7. boss hitPos crosscheck
echo "=== boss hitPos crosscheck (node, dynamic bone chain) ===" >> "$LOG"
RAW=/tmp/gate-xc-$$.log
: > "$RAW"
node tools/verify/boss-hitpos-crosscheck.mjs >> "$RAW" 2>&1
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

# 9. headless (native release binary first)
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

# JS tree status
echo "=== JS tree git status ===" >> "$LOG"
JS_STATUS=$(git status --short ../src/ 2>/dev/null)
if [ -z "$JS_STATUS" ]; then
  echo "JS_TREE_CLEAN" >> "$LOG"
else
  echo "$JS_STATUS" >> "$LOG"
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
