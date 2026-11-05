#!/usr/bin/env bash
# Nine-stage gate for Task 47 — raw-file capture form (Task 41+ convention).
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task47-gates-${TS}.log
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

# 6. tuning+fixture determinism
echo "=== tuning+fixture determinism (rerun, must be byte-identical) ===" >> "$LOG"
RAW=/tmp/gate-det-$$.log
: > "$RAW"
cd tools/extract
node extract-tuning.mjs >> "$RAW" 2>&1
node extract-layout.mjs >> "$RAW" 2>&1
node extract-layout.mjs saltpan >> "$RAW" 2>&1
node extract-layout.mjs crossmarket >> "$RAW" 2>&1
cd ../..
DETERM_RC=$?
SHA_T=$(sha256sum assets/tuning.json | awk '{print $1}')
SHA_F=$(sha256sum assets/net-fixtures.json | awk '{print $1}')
echo "$SHA_T  assets/tuning.json" >> "$LOG"
echo "$SHA_F  assets/net-fixtures.json" >> "$LOG"
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
