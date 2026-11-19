#!/usr/bin/env bash
# Nine-stage gate for Task 62 — 3D lobby preview / nameplates / emote.
# Template: gate-task61.sh. Deltas:
#   DETERM: extract-lobbyset.mjs double-run keeps the emote/spot/camera source
#           (lobbyset.json) byte-identical.
#   XCHK:   all showcase tests (old 11 + Task62 state/projection/scene anchors)
#           + lobbyset crosscheck (t0/trig already re-parsed from JS) + the
#           standing environment/props/camera crosschecks.
# Verdict from every recorded *_EXIT; TRUNK D-T40-1 DEGRADED is the only
# non-blocking failure.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task62-gates-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate62-raw-$$.log"
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
# 3. workspace tests (all M1-M4 regressions + Task 59 anchors)
stage test cargo test --workspace || true
# 4. wasm32 check
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
# 5. sim dependency surface unchanged (NFR-M4-1)
stage tree cargo tree -p inkwave_sim || true

# 6. determinism: the emote/spot/camera source (lobbyset.json) is re-extracted
#    twice and stays byte-identical (source.commit pinned).
echo "=== determinism: lobbyset camera table double-run ===" >> "$LOG"
RAW=/tmp/gate62-det-$$.log
: > "$RAW"
FILES="assets/lobbyset.json"
BEFORE=$(sha256sum $FILES)
cd tools/extract
node extract-lobbyset.mjs >> "$RAW" 2>&1
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

# 7. crosschecks: every showcase/state/scene anchor, camera consumers and the
#    standing environment/lobbyset/props crosschecks.
echo "=== xchk: showcase + task62 + camera consumers + environment + lobbyset + props ===" >> "$LOG"
RAW=/tmp/gate62-xc-$$.log
: > "$RAW"
cargo test -p inkwave --bin inkwave showcase -- --nocapture >> "$RAW" 2>&1
XC1=$?
TASK62_RAW=/tmp/gate62-task62-$$.log
cargo test -p inkwave --bin inkwave task62 -- --nocapture > "$TASK62_RAW" 2>&1
XC2=$?
TASK62_COUNT=$(grep -o 'running [0-9]\+ tests' "$TASK62_RAW" | tail -1 | awk '{print $2}')
if [ -z "$TASK62_COUNT" ] || [ "$TASK62_COUNT" -lt 5 ]; then
  echo "XCHK_TASK62_COUNT_BAD=${TASK62_COUNT:-missing}" >> "$LOG"
  XC2=1
fi
cat "$TASK62_RAW" >> "$RAW"
rm -f "$TASK62_RAW"
cargo test -p inkwave --bin inkwave camera -- --nocapture >> "$RAW" 2>&1
XC3=$?
cargo test -p inkwave --bin inkwave vfov_matches -- --nocapture >> "$RAW" 2>&1
XC4=$?
node tools/environment-crosscheck.mjs >> "$RAW" 2>&1
XC5=$?
node tools/lobbyset-crosscheck.mjs >> "$RAW" 2>&1
XC6=$?
node tools/props-crosscheck.mjs >> "$RAW" 2>&1
XC7=$?
XC_RC=$((XC1 | XC2 | XC3 | XC4 | XC5 | XC6 | XC7))
echo "XCHK_SHOWCASE_EXIT=$XC1" >> "$LOG"
echo "XCHK_TASK62_EXIT=$XC2" >> "$LOG"
echo "XCHK_CAMERA_EXIT=$XC3" >> "$LOG"
echo "XCHK_VFOV_EXIT=$XC4" >> "$LOG"
echo "XCHK_ENV_EXIT=$XC5" >> "$LOG"
echo "XCHK_LOBBYSET_EXIT=$XC6" >> "$LOG"
echo "XCHK_PROPS_EXIT=$XC7" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

# 8. trunk release build — ONE attempt (≡ M4 closing matrix gate-task57.sh;
#    the sandbox wasm-bindgen OOM SIGKILL is environmental, D-T40-1, and a
#    retry cannot change it — 3 attempts cost ~75 min for the same result).
echo "=== trunk build (one attempt, SIGKILL false-green guard, D-T40-1) ===" >> "$LOG"
RAW=/tmp/gate62-trunk-$$.log
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
RAW=/tmp/gate62-hl-$$.log
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
