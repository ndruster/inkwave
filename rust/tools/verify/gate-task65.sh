#!/usr/bin/env bash
# Nine-stage gate for Task 65 — post-processing chain (GTAO/grade/SMAA/tilt-shift).
# Template: gate-task64.sh. Deltas:
#   DETERM: extract-postprocess.mjs double-run keeps assets/postprocess.json
#           byte-identical (source.commit pinned to renderer/config/hud.css).
#   XCHK:   task65专项 (4 anchors: grade 12 uniforms, four-tier switches +
#           low all-off, GTAO/SMAA/tilt params, pass order) + postprocess
#           crosscheck (53 assertions) + environment crosscheck (97 checks,
#           uHurtColor extended) + standing lobbyset/props/screenfx crosschecks.
# Verdict from every recorded *_EXIT; TRUNK D-T40-1 DEGRADED is the only
# non-blocking failure.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task65-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate65-raw-$$.log"
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
# 3. workspace tests (all M1-M5 regressions + Task 59-64 anchors)
stage test cargo test --workspace || true
# 4. wasm32 check
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
# 5. sim dependency surface unchanged (NFR-M4-1)
stage tree cargo tree -p inkwave_sim || true

# 6. determinism: postprocess source (postprocess.json) is re-extracted twice
#    and stays byte-identical (source.commit pinned).
echo "=== determinism: postprocess extraction double-run ===" >> "$LOG"
RAW=/tmp/gate65-det-$$.log
: > "$RAW"
FILES="assets/postprocess.json"
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-postprocess.mjs >> "$RAW" 2>&1
DET1=$?
node extract-postprocess.mjs >> "$RAW" 2>&1
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

# 7. crosschecks: task65 anchors (count guard), postprocess (53) + environment
#    (97) crosschecks and the standing screenfx/lobbyset/props crosschecks.
echo "=== xchk: task65 + postprocess + environment + screenfx + lobbyset + props ===" >> "$LOG"
RAW=/tmp/gate65-xc-$$.log
: > "$RAW"
TASK65_RAW=/tmp/gate65-task65-$$.log
cargo test -p inkwave --bin inkwave task65 -- --nocapture > "$TASK65_RAW" 2>&1
XC1=$?
TASK65_COUNT=$(grep -o 'running [0-9]\+ tests' "$TASK65_RAW" | tail -1 | awk '{print $2}')
if [ -z "$TASK65_COUNT" ] || [ "$TASK65_COUNT" -lt 4 ]; then
  echo "XCHK_TASK65_COUNT_BAD=${TASK65_COUNT:-missing}" >> "$LOG"
  XC1=1
fi
cat "$TASK65_RAW" >> "$RAW"
rm -f "$TASK65_RAW"
node tools/postprocess-crosscheck.mjs >> "$RAW" 2>&1
XC2=$?
node tools/environment-crosscheck.mjs >> "$RAW" 2>&1
XC3=$?
node tools/screenfx-crosscheck.mjs >> "$RAW" 2>&1
XC4=$?
node tools/lobbyset-crosscheck.mjs >> "$RAW" 2>&1
XC5=$?
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC6=$?
XC_RC=$((XC1 | XC2 | XC3 | XC4 | XC5 | XC6))
echo "XCHK_TASK65_EXIT=$XC1" >> "$LOG"
echo "XCHK_POSTPROCESS_EXIT=$XC2" >> "$LOG"
echo "XCHK_ENV_EXIT=$XC3" >> "$LOG"
echo "XCHK_SCREENFX_EXIT=$XC4" >> "$LOG"
echo "XCHK_LOBBYSET_EXIT=$XC5" >> "$LOG"
echo "XCHK_PROPS_EXIT=$XC6" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

# 8. trunk release build — ONE attempt (D-T40-1 SIGKILL guard).
echo "=== trunk build (one attempt, SIGKILL false-green guard, D-T40-1) ===" >> "$LOG"
RAW=/tmp/gate65-trunk-$$.log
: > "$RAW"
trunk build --release >> "$RAW" 2>&1
TRUNK_RC=$?
SIGKILL_HITS=$(grep -c "signal: 9" "$RAW" 2>/dev/null || true)
[ -z "$SIGKILL_HITS" ] && SIGKILL_HITS=0
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
echo "TRUNK_EXIT=$TRUNK_RC" >> "$LOG"

# 9. headless: 4 (theme,tier) boot combinations + boss regression, 600 frames
echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate65-hl-$$.log
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
