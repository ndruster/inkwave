#!/usr/bin/env bash
# Nine-stage gate for Task 56 — 3D showcase lobby / state layer.
# Cumulative with gate-task55.sh:
#   DETERM += assets/lobbyset.json (and re-runs extract-lobbyset.mjs)
#   XCHK   += lobbyset-crosscheck.mjs (independent regex re-parse + bearing
#             recompute) and the ui::showcase_state unit tests
#   HEADLESS keeps the 4 (theme,tier) boot combinations + boss regression;
#            the showcase scene is gated off in headless (no WorldPlugin),
#            so TR-56.1 also runs the pure state/data layer in `cargo test`.
# Verdict is computed from every recorded *_EXIT, with the D-T40-1 TRUNK
# DEGRADED precedent as the only non-blocking failure.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task56-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate56-raw-$$.log"
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
# 3. workspace tests (TR-56.1 state anchors + all M1-M4 regressions)
stage test cargo test --workspace || true
# 4. wasm32 check
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
# 5. sim dependency surface unchanged (NFR-M4-1)
stage tree cargo tree -p inkwave_sim || true

# 6. determinism: rerun both extractors; every data artifact stays byte-identical.
echo "=== determinism: environment + lobbyset + char assets ===" >> "$LOG"
RAW=/tmp/gate56-det-$$.log
: > "$RAW"
FILES="assets/environment.json assets/lobbyset.json assets/char/char.bin.gz assets/char/char.json assets/char/char-poses.json crates/inkwave/tests/fixtures/char-pose-ref.json"
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-environment.mjs >> "$RAW" 2>&1
DET1=$?
node extract-lobbyset.mjs >> "$RAW" 2>&1
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

# 7. independent crosschecks: JS re-parse for environment + lobby set,
#    props tie-out, char pinned replay, and the Task 55/56 Rust unit tests.
echo "=== xchk: environment + lobbyset + props + char-pose regressions ===" >> "$LOG"
RAW=/tmp/gate56-xc-$$.log
: > "$RAW"
node tools/environment-crosscheck.mjs >> "$RAW" 2>&1
XC1=$?
node tools/lobbyset-crosscheck.mjs >> "$RAW" 2>&1
XC2=$?
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC3=$?
cargo test -p inkwave --bin inkwave environment:: -- --nocapture >> "$RAW" 2>&1
XC4=$?
cargo test -p inkwave --bin inkwave ui::showcase_state -- --nocapture >> "$RAW" 2>&1
XC5=$?
cargo test -p inkwave --bin inkwave char::pinned -- --nocapture >> "$RAW" 2>&1
XC6=$?
XC_RC=$((XC1 | XC2 | XC3 | XC4 | XC5 | XC6))
echo "XCHK_ENV_EXIT=$XC1" >> "$LOG"
echo "XCHK_LOBBYSET_EXIT=$XC2" >> "$LOG"
echo "XCHK_PROPS_EXIT=$XC3" >> "$LOG"
echo "XCHK_ENVTEST_EXIT=$XC4" >> "$LOG"
echo "XCHK_SHOWCASE_EXIT=$XC5" >> "$LOG"
echo "XCHK_POSE_EXIT=$XC6" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

# 8. trunk release build (retry<=3, SIGKILL false-green guard, D-T40-1)
echo "=== trunk build (retry<=3, SIGKILL false-green guard) ===" >> "$LOG"
TRUNK_RC=1
SIGKILL_HITS=0
for attempt in 1 2 3; do
  RAW=/tmp/gate56-trunk-$$.log
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

# 9. headless: 4 (theme,tier) boot combinations + boss regression, 600 frames
#    each. The 3D lobby only exists on the windowed branch, so the pure state
#    layer is covered by XCHK_SHOWCASE_EXIT and the workspace test stage.
echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate56-hl-$$.log
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
