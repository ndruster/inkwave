#!/usr/bin/env bash
# Nine-stage gate for Task 54 — full character spring-pose binding (D-T54-4).
# Cumulative with gate-task53.sh:
#   DETERM  += char.bin.gz + char.json + char-poses.json + char-pose-ref.json
#   XCHK    += the JS reference harness replay (char::pinned) on top of the
#              live props crosscheck
#   HEADLESS += turf + boss, 600 frames each (8+1 rig trees per frame)
# Verdict is computed from every recorded *_EXIT, with the D-T40-1 TRUNK
# DEGRADED precedent as the only non-blocking failure.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task54-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate54-raw-$$.log"
  : > "$raw"
  "$@" >> "$raw" 2>&1
  local rc=$?
  echo "${name^^}_EXIT=$rc" >> "$LOG"
  grep -v "libselinux.so.1" "$raw" >> "$LOG" 2>/dev/null || true
  rm -f "$raw"
  return $rc
}

# 1. fmt
stage fmt cargo fmt --all -- --check || true
# 2. clippy (all targets, warnings are errors)
stage clippy cargo clippy --workspace --all-targets -- -D warnings || true
# 3. workspace tests (TR-54.1 pinned replay, TR-54.2 draw-call budget,
#    form-switch state machine, bridge 30-frame determinism, + all M1-M3)
stage test cargo test --workspace || true
# 4. wasm32 check
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
# 5. sim dependency surface must stay unchanged (NFR-M4-1)
stage tree cargo tree -p inkwave_sim || true

# 6. determinism: rerun BOTH char extractors, all four products must be
#    byte-identical (D-T54-1 / D-T54-2).
echo "=== determinism: char assets + pose reference fixture ===" >> "$LOG"
RAW=/tmp/gate54-det-$$.log
: > "$RAW"
FILES="assets/char/char.bin.gz assets/char/char.json assets/char/char-poses.json crates/inkwave/tests/fixtures/char-pose-ref.json"
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-character.mjs >> "$RAW" 2>&1
node char-pose-reference.mjs >> "$RAW" 2>&1
DET_RC=$?
cd ../..
AFTER=$(sha256sum $FILES)
if [ "$BEFORE" = "$AFTER" ] && [ "$DET_RC" -eq 0 ]; then
  echo "DETERM=BYTE_IDENTICAL" >> "$LOG"
else
  echo "DETERM=MISMATCH" >> "$LOG"
  diff <(echo "$BEFORE") <(echo "$AFTER") >> "$LOG" 2>&1
  DET_RC=1
fi
echo "$AFTER" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "DETERM_EXIT=$DET_RC" >> "$LOG"
rm -f "$RAW"

# 7. independent crosscheck: live dressingFor/PropKit (Task 53 regression) +
#    the JS character.js reference fixture replayed by the Rust pose core.
echo "=== xchk: props-crosscheck + char-pose-reference pinned replay ===" >> "$LOG"
RAW=/tmp/gate54-xc-$$.log
: > "$RAW"
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC1=$?
cargo test -p inkwave --bin inkwave char::pinned -- --nocapture >> "$RAW" 2>&1
XC2=$?
XC_RC=$((XC1 | XC2))
echo "XCHK_PROPS_EXIT=$XC1" >> "$LOG"
echo "XCHK_POSE_EXIT=$XC2" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

# 8. trunk release build (retry<=3, SIGKILL false-green guard, D-T40-1)
echo "=== trunk build (retry<=3, SIGKILL false-green guard) ===" >> "$LOG"
TRUNK_RC=1
SIGKILL_HITS=0
for attempt in 1 2 3; do
  RAW=/tmp/gate54-trunk-$$.log
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

# 9. headless (release): 600 frames of turf + boss, every frame drives all
#    rig trees through CharAnim; any panic/NaN fails the process.
echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate54-hl-$$.log
: > "$RAW"
cargo build --release -p inkwave >> "$RAW" 2>&1
BUILD_RC=$?
target/release/inkwave --headless --frames 600 >> "$RAW" 2>&1
HL_RC=$?
target/release/inkwave --headless --mode boss --frames 600 >> "$RAW" 2>&1
HL_BOSS_RC=$?
echo "RELEASE_BUILD_EXIT=$BUILD_RC" >> "$LOG"
echo "HEADLESS_EXIT=$HL_RC" >> "$LOG"
echo "HEADLESS_BOSS_EXIT=$HL_BOSS_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"

# Upstream discipline (NFR-M4-1 / NFR-M4-4)
echo "=== upstream tree guards ===" >> "$LOG"
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

echo "=== verdict ===" >> "$LOG"
FAILED=0
for key in FMT CLIPPY TEST WASM TREE DETERM XCHK HEADLESS HEADLESS_BOSS RELEASE_BUILD; do
  val=$(grep "^${key}_EXIT=" "$LOG" | tail -1 | cut -d= -f2)
  if [ -z "$val" ] || [ "$val" != "0" ]; then
    echo "STAGE_FAIL $key=${val:-missing}" >> "$LOG"
    FAILED=1
  fi
done
if [ "$TRUNK_RC" -ne 0 ]; then
  if [ "$SIGKILL_HITS" -gt 0 ]; then
    echo "TRUNK=DEGRADED: 3 attempts, rc=$TRUNK_RC, signal:9 hits=$SIGKILL_HITS" >> "$LOG"
    echo "(wasm-bindgen OOM SIGKILL in the sandbox — environmental, D-T40-1 precedent)" >> "$LOG"
  else
    echo "STAGE_FAIL TRUNK=$TRUNK_RC" >> "$LOG"
    FAILED=1
  fi
fi
grep -q '^JS_TREE_CLEAN$' "$LOG" || { echo "STAGE_FAIL JS_TREE_DIRTY" >> "$LOG"; FAILED=1; }
grep -q '^SIM_TREE_CLEAN$' "$LOG" || { echo "STAGE_FAIL SIM_TREE_DIRTY" >> "$LOG"; FAILED=1; }
if [ "$FAILED" -eq 0 ]; then
  echo "GATE_VERDICT=PASS" >> "$LOG"
else
  echo "GATE_VERDICT=FAIL" >> "$LOG"
fi
echo "GATES_DONE" >> "$LOG"
echo "Gate log: $LOG"
tail -4 "$LOG"
exit "$FAILED"
