#!/usr/bin/env bash
# Nine-stage gate for Task 71 — fullscreen grade + screenfx composite pass body.
# Template: gate-task70.sh (finale matrix) / gate-task69.sh (regular task). Deltas:
#   DETERM: postprocess + screenfx extraction double-run keeps both JSONs
#           byte-identical (the pass body consumes those two data planes);
#   XCHK:   task71专项 (6 anchors: identity default, low/post-off gate,
#           grade-unconditional-when-idle, composite forwarding, 12-uniform
#           pin, WGSL binding/struct interface) + postprocess/screenfx
#           crosschecks (the two data planes this task binds) + standing
#           environment/lobbyset/boss/locker/props crosschecks.
# Verdict from every recorded *_EXIT; TRUNK D-T40-1 DEGRADED is the only
# non-blocking failure. GPU visuals are a user-machine review item (D-M6-1..4).
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task71-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate71-raw-$$.log"
  : > "$raw"
  "$@" >> "$raw" 2>&1
  local rc=$?
  echo "${name^^}_EXIT=$rc" >> "$LOG"
  grep -v "libselinux.so.1" "$raw" >> "$LOG" 2>/dev/null || true
  rm -f "$raw"
  return $rc
}

stage fmt cargo fmt --all -- --check || true
stage clippy cargo clippy --workspace --all-targets -- -D warnings || true
stage test cargo test --workspace || true
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
stage tree cargo tree -p inkwave_sim || true

echo "=== determinism: postprocess + screenfx extraction double-run ===" >> "$LOG"
RAW=/tmp/gate71-det-$$.log
: > "$RAW"
FILES="assets/postprocess.json assets/screenfx.json"
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-postprocess.mjs >> "$RAW" 2>&1
DET1=$?
node extract-screenfx.mjs >> "$RAW" 2>&1
DET2=$?
cd ../..
AFTER=$(sha256sum $FILES)
if [ "$BEFORE" = "$AFTER" ] && [ "$DET1" -eq 0 ] && [ "$DET2" -eq 0 ]; then
  echo "DETERM=BYTE_IDENTICAL" >> "$LOG"
  DET_RC=0
else
  echo "DETERM=MISMATCH" >> "$LOG"
  diff <(echo "$BEFORE") <(echo "$AFTER") >> "$LOG" 2>&1
  DET_RC=1
fi
echo "$AFTER" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "DETERM_EXIT=$DET_RC" >> "$LOG"
rm -f "$RAW"

echo "=== xchk: task71 + postprocess + screenfx + environment + lobbyset + boss + locker + props ===" >> "$LOG"
RAW=/tmp/gate71-xc-$$.log
: > "$RAW"
TASK71_RAW=/tmp/gate71-task71-$$.log
cargo test -p inkwave --bin inkwave task71 -- --nocapture > "$TASK71_RAW" 2>&1
XC1=$?
TASK71_COUNT=$(grep -o 'running [0-9]\+ tests' "$TASK71_RAW" | tail -1 | awk '{print $2}')
if [ -z "$TASK71_COUNT" ] || [ "$TASK71_COUNT" -lt 6 ]; then
  echo "XCHK_TASK71_COUNT_BAD=${TASK71_COUNT:-missing}" >> "$LOG"
  XC1=1
fi
cat "$TASK71_RAW" >> "$RAW"
rm -f "$TASK71_RAW"
node tools/postprocess-crosscheck.mjs >> "$RAW" 2>&1
XC2=$?
node tools/screenfx-crosscheck.mjs >> "$RAW" 2>&1
XC3=$?
node tools/environment-crosscheck.mjs >> "$RAW" 2>&1
XC4=$?
node tools/lobbyset-crosscheck.mjs >> "$RAW" 2>&1
XC5=$?
node tools/boss-crosscheck.mjs >> "$RAW" 2>&1
XC6=$?
node tools/locker-crosscheck.mjs >> "$RAW" 2>&1
XC7=$?
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC8=$?
XC_RC=$((XC1 | XC2 | XC3 | XC4 | XC5 | XC6 | XC7 | XC8))
echo "XCHK_TASK71_EXIT=$XC1" >> "$LOG"
echo "XCHK_POSTPROCESS_EXIT=$XC2" >> "$LOG"
echo "XCHK_SCREENFX_EXIT=$XC3" >> "$LOG"
echo "XCHK_ENV_EXIT=$XC4" >> "$LOG"
echo "XCHK_LOBBYSET_EXIT=$XC5" >> "$LOG"
echo "XCHK_BOSS_EXIT=$XC6" >> "$LOG"
echo "XCHK_LOCKER_EXIT=$XC7" >> "$LOG"
echo "XCHK_PROPS_EXIT=$XC8" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

echo "=== trunk build (one attempt, SIGKILL false-green guard, D-T40-1) ===" >> "$LOG"
RAW=/tmp/gate71-trunk-$$.log
: > "$RAW"
trunk build --release >> "$RAW" 2>&1
TRUNK_RC=$?
SIGKILL_HITS=$(grep -c "signal: 9" "$RAW" 2>/dev/null || true)
[ -z "$SIGKILL_HITS" ] && SIGKILL_HITS=0
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
echo "TRUNK_EXIT=$TRUNK_RC" >> "$LOG"

echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate71-hl-$$.log
: > "$RAW"
cargo build --release -p inkwave >> "$RAW" 2>&1
BUILD_RC=$?
HL_RC=0
for spec in day:low sunset:low golden:high day:ultra; do
  theme=${spec%%:*}; tier=${spec##*:}
  INKWAVE_THEME=$theme INKWAVE_QUALITY=$tier target/release/inkwave --headless --frames 600 >> "$RAW" 2>&1
  COMBO_RC=$?
  echo "HEADLESS_COMBO ${theme}/${tier} EXIT=$COMBO_RC" >> "$LOG"
  [ "$COMBO_RC" -ne 0 ] && HL_RC=1
done
target/release/inkwave --headless --mode boss --frames 600 >> "$RAW" 2>&1
HL_BOSS_RC=$?
echo "RELEASE_BUILD_EXIT=$BUILD_RC" >> "$LOG"
echo "HEADLESS_EXIT=$HL_RC" >> "$LOG"
echo "HEADLESS_BOSS_EXIT=$HL_BOSS_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"

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
    echo "TRUNK=DEGRADED: 1 attempt, rc=$TRUNK_RC, signal:9 hits=$SIGKILL_HITS" >> "$LOG"
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
