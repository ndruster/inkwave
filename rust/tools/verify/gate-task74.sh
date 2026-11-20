#!/usr/bin/env bash
# Task 74 — M6 finale MATRIX gate.
# Scope: (1) nine-stage gate on the final workspace; (2) ALL 10 crosschecks
# (M5 regression baseline); (3) full-artifact determinism via run-all.sh
# double run; (4) audit that every M6 task gate log (71-73) latest == PASS;
# (5) upstream/sim guards. Template: gate-task70.sh (M5 finale). TRUNK
# DEGRADED (D-T40-1 SIGKILL) is non-blocking; a real non-SIGKILL failure fails.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task74-m6-matrix-${TS}.log
: > "$LOG"

stage() {
  local name="$1"; shift
  echo "=== $name ===" >> "$LOG"
  local raw="/tmp/gate74-raw-$$.log"
  : > "$raw"
  "$@" >> "$raw" 2>&1
  local rc=$?
  echo "${name^^}_EXIT=$rc" >> "$LOG"
  grep -v "libselinux.so.1" "$raw" >> "$LOG" 2>/dev/null || true
  rm -f "$raw"
  return $rc
}

# 1-5: nine-stage core
stage fmt cargo fmt --all -- --check || true
stage clippy cargo clippy --workspace --all-targets -- -D warnings || true
stage test cargo test --workspace || true
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown || true
stage tree cargo tree -p inkwave_sim || true

# 6: determinism — full-artifact re-extraction (run-all.sh) leaves assets byte-identical
echo "=== determinism: run-all.sh double-run ===" >> "$LOG"
RAW=/tmp/gate74-det-$$.log
: > "$RAW"
hash_assets() { find assets -type f -name '*.json' -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1; }
BEFORE=$(hash_assets)
(cd tools/extract && bash run-all.sh >> "$RAW" 2>&1)
RUN1=$?
MID=$(hash_assets)
(cd tools/extract && bash run-all.sh >> "$RAW" 2>&1)
RUN2=$?
AFTER=$(hash_assets)
if [ "$BEFORE" = "$MID" ] && [ "$MID" = "$AFTER" ] && [ "$RUN1" -eq 0 ] && [ "$RUN2" -eq 0 ]; then
  echo "DETERM=BYTE_IDENTICAL" >> "$LOG"
  DET_RC=0
else
  echo "DETERM=MISMATCH before=$BEFORE mid=$MID after=$AFTER" >> "$LOG"
  DET_RC=1
fi
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "DETERM_EXIT=$DET_RC" >> "$LOG"
rm -f "$RAW"

# 7: ALL 10 crosschecks (M5 regression baseline + M6 data planes)
echo "=== xchk: all 10 crosschecks ===" >> "$LOG"
RAW=/tmp/gate74-xc-$$.log
: > "$RAW"
XC_RC=0
for xc in \
  settings-crosscheck.mjs \
  kit-sfx-crosscheck.mjs \
  audio-bus-crosscheck.mjs \
  environment-crosscheck.mjs \
  lobbyset-crosscheck.mjs \
  postprocess-crosscheck.mjs \
  screenfx-crosscheck.mjs \
  props-crosscheck.mjs \
  boss-crosscheck.mjs \
  locker-crosscheck.mjs; do
  node "tools/$xc" >> "$RAW" 2>&1
  rc=$?
  echo "XCHK_${xc%.mjs}_EXIT=$rc" >> "$LOG"
  [ "$rc" -ne 0 ] && XC_RC=1
done
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
echo "XCHK_EXIT=$XC_RC" >> "$LOG"
rm -f "$RAW"

# 8: trunk release build — ONE attempt (D-T40-1 SIGKILL guard)
echo "=== trunk build (one attempt, SIGKILL false-green guard, D-T40-1) ===" >> "$LOG"
RAW=/tmp/gate74-trunk-$$.log
: > "$RAW"
trunk build --release >> "$RAW" 2>&1
TRUNK_RC=$?
# P2-3: count only the wasm-bindgen SIGKILL signal in the error causal chain,
# not an arbitrary "signal: 9" string anywhere in the build log.
SIGKILL_HITS=$(grep -c "returned a bad status: signal: 9" "$RAW" 2>/dev/null || true)
[ -z "$SIGKILL_HITS" ] && SIGKILL_HITS=0
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
echo "TRUNK_EXIT=$TRUNK_RC" >> "$LOG"

# 9: headless — theme/tier boot combos + boss 600 frames
echo "=== headless ===" >> "$LOG"
RAW=/tmp/gate74-hl-$$.log
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

# Upstream discipline guards
echo "=== upstream tree guards ===" >> "$LOG"
JS_STATUS=$(git status --short ../src/ ../server/ ../tools/ ../docs/ 2>/dev/null)
if [ -z "$JS_STATUS" ]; then echo "JS_TREE_CLEAN" >> "$LOG"; else echo "$JS_STATUS" >> "$LOG"; fi
SIM_DIFF=$(git status --short crates/inkwave_sim/ 2>/dev/null)
if [ -z "$SIM_DIFF" ]; then echo "SIM_TREE_CLEAN" >> "$LOG"; else echo "$SIM_DIFF" >> "$LOG"; fi

# M6 gate-log audit: every M6 task (71-73) LATEST log must be PASS.
echo "=== M6 gate-log verdict audit (tasks 71-73) ===" >> "$LOG"
AUDIT_RC=0
for t in 71 72 73; do
  logs=$(ls tools/verify/logs/task${t}-gates-*.log 2>/dev/null | sort)
  latest=$(echo "$logs" | tail -1)
  found_pass=0
  if [ -n "$latest" ] && grep -q '^GATE_VERDICT=PASS$' "$latest"; then found_pass=1; fi
  if [ "$found_pass" -eq 1 ]; then
    echo "M6_TASK_${t}_VERDICT=PASS ($latest)" >> "$LOG"
  else
    echo "M6_TASK_${t}_VERDICT=MISSING_OR_FAIL" >> "$LOG"
    AUDIT_RC=1
  fi
done
echo "M6_AUDIT_EXIT=$AUDIT_RC" >> "$LOG"

echo "=== verdict ===" >> "$LOG"
FAILED=0
for key in FMT CLIPPY TEST WASM TREE DETERM XCHK HEADLESS HEADLESS_BOSS RELEASE_BUILD M6_AUDIT; do
  val=$(grep "^${key}_EXIT=" "$LOG" | tail -1 | cut -d= -f2)
  if [ -z "$val" ] || [ "$val" != "0" ]; then
    echo "STAGE_FAIL $key=${val:-missing}" >> "$LOG"
    FAILED=1
  fi
done
if [ "$TRUNK_RC" -ne 0 ]; then
  if [ "$SIGKILL_HITS" -gt 0 ]; then
    echo "TRUNK=DEGRADED: rc=$TRUNK_RC, signal:9 hits=$SIGKILL_HITS (D-T40-1 precedent, non-blocking)" >> "$LOG"
  else
    echo "STAGE_FAIL TRUNK=$TRUNK_RC" >> "$LOG"
    FAILED=1
  fi
fi
grep -q '^JS_TREE_CLEAN$' "$LOG" || { echo "STAGE_FAIL JS_TREE_DIRTY" >> "$LOG"; FAILED=1; }
grep -q '^SIM_TREE_CLEAN$' "$LOG" || { echo "STAGE_FAIL SIM_TREE_DIRTY" >> "$LOG"; FAILED=1; }
if [ "$FAILED" -eq 0 ]; then
  echo "M6_MATRIX_VERDICT=PASS" >> "$LOG"
else
  echo "M6_MATRIX_VERDICT=FAIL" >> "$LOG"
fi
echo "MATRIX_DONE" >> "$LOG"
echo "Matrix log: $LOG"
tail -4 "$LOG"
exit "$FAILED"
