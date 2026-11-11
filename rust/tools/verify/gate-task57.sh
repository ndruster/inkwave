#!/usr/bin/env bash
# Task 57 M4 closing matrix.
#
# Runs the commands published in rust/README.md §7 and the §"门禁速查"
# fmt/clippy/test/wasm/tree/xchk items, verifies determinism over ALL data
# artifacts (json / gz / svg / fixture), runs TRUNK once (D-T40-1 OOM is
# recorded as DEGRADED), then prints the Task 41..56 gate matrix.
#
# Net-smoke (README §3) needs the relay + browser and is an M2 legacy item;
# it is explicitly out of scope for the M4 closing matrix.
set -u
cd /root/work/project/inkwave/rust
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=tools/verify/logs/task57-m4-matrix-${TS}.log
: > "$LOG"
FAILED=0

stage() {
  local name="$1"; shift
  echo "=== $name: $* ===" >> "$LOG"
  local raw="/tmp/gate57-raw-$$.log"
  : > "$raw"
  "$@" >> "$raw" 2>&1
  local rc=$?
  echo "${name^^}_EXIT=$rc" >> "$LOG"
  grep -v "libselinux.so.1" "$raw" >> "$LOG" 2>/dev/null || true
  rm -f "$raw"
  [ "$rc" -ne 0 ] && { echo "STAGE_FAIL $name=$rc" >> "$LOG"; FAILED=1; }
  return 0
}

stage fmt cargo fmt --all -- --check
stage clippy cargo clippy --workspace --all-targets -- -D warnings
stage test cargo test --workspace
stage wasm cargo check -p inkwave_sim -p inkwave --target wasm32-unknown-unknown
stage tree cargo tree -p inkwave_sim

# ---- Determinism over ALL data artifacts ----------------------------------
# `run-all.sh` regenerates the json/gz products; the 24 SVG thumbnails and the
# pinned char-pose fixture are read-only goldens, so they are checked for
#"not drifted" rather than regenerated (they were covered by gate-task53.sh).
echo "=== determinism: bash tools/extract/run-all.sh over all artifacts ===" >> "$LOG"
LIST=/tmp/gate57-artifacts-$$.txt
find assets crates/inkwave/tests/fixtures -type f \
  \( -name '*.json' -o -name '*.bin.gz' -o -name '*.svg' \) -print0 | sort -z > "$LIST"
BEFORE=$(xargs -0 sha256sum < "$LIST")
RAW=/tmp/gate57-det-$$.log
: > "$RAW"
(cd tools/extract && bash run-all.sh) >> "$RAW" 2>&1
DET_RC=$?
AFTER=$(xargs -0 sha256sum < "$LIST")
rm -f "$LIST"
if [ "$BEFORE" = "$AFTER" ] && [ "$DET_RC" -eq 0 ]; then
  echo "DETERM=BYTE_IDENTICAL" >> "$LOG"
  DETERM_RC=0
else
  echo "DETERM=MISMATCH" >> "$LOG"
  diff <(echo "$BEFORE") <(echo "$AFTER") >> "$LOG" 2>&1
  DETERM_RC=1
  FAILED=1
fi
echo "RUN_ALL_EXIT=$DET_RC" >> "$LOG"
echo "DETERM_EXIT=$DETERM_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
ARTIFACT_COUNT=$(printf '%s' "$BEFORE" | grep -c . || true)
echo "ARTIFACT_COUNT=$ARTIFACT_COUNT" >> "$LOG"
# Defensive floor: the current registry is 59 artifacts (json / bin.gz / svg /
# pinned fixture). A much smaller count means the find scope broke or a product
# was deleted, and an empty BEFORE==AFTER would otherwise look "identical".
if [ "${ARTIFACT_COUNT:-0}" -lt 50 ]; then
  echo "STAGE_FAIL ARTIFACT_COUNT_TOO_LOW=$ARTIFACT_COUNT" >> "$LOG"
  FAILED=1
fi

stage xchk_environment node tools/environment-crosscheck.mjs
stage xchk_lobbyset node tools/lobbyset-crosscheck.mjs
stage xchk_props node tools/props-crosscheck.mjs
stage xchk_collision bash tools/verify/run-crosscheck.sh

# ---- TRUNK: actually run once in the closing matrix -----------------------
echo "=== TRUNK (closing matrix, one attempt; D-T40-1) ===" >> "$LOG"
RAW=/tmp/gate57-trunk-$$.log
: > "$RAW"
trunk build --release >> "$RAW" 2>&1
TRUNK_RC=$?
SIGKILL=$(grep -c 'signal: 9' "$RAW" 2>/dev/null || true)
[ -z "$SIGKILL" ] && SIGKILL=0
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
echo "TRUNK_EXIT=$TRUNK_RC" >> "$LOG"
if [ "$TRUNK_RC" -ne 0 ] && [ "$SIGKILL" -gt 0 ]; then
  echo "TRUNK=DEGRADED: rc=$TRUNK_RC, signal:9 hits=$SIGKILL (D-T40-1, environmental, non-blocking)" >> "$LOG"
elif [ "$TRUNK_RC" -ne 0 ]; then
  echo "STAGE_FAIL TRUNK=$TRUNK_RC" >> "$LOG"
  FAILED=1
fi

# ---- README §7 headless matrix: 4 combos + boss, 600 frames each ----------
stage release_build cargo build --release -p inkwave
for spec in day:low sunset:low golden:high day:ultra; do
  theme=${spec%%:*}; tier=${spec##*:}
  RAW=/tmp/gate57-hl-$$.log
  : > "$RAW"
  INKWAVE_THEME=$theme INKWAVE_QUALITY=$tier target/release/inkwave --headless --frames 600 >> "$RAW" 2>&1
  rc=$?
  echo "HEADLESS_COMBO ${theme}/${tier} EXIT=$rc" >> "$LOG"
  grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
  rm -f "$RAW"
  [ "$rc" -ne 0 ] && { echo "STAGE_FAIL headless_${theme}_${tier}=$rc" >> "$LOG"; FAILED=1; }
done
RAW=/tmp/gate57-boss-$$.log
: > "$RAW"
target/release/inkwave --headless --mode boss --frames 600 >> "$RAW" 2>&1
BOSS_RC=$?
echo "HEADLESS_BOSS_EXIT=$BOSS_RC" >> "$LOG"
grep -v "libselinux.so.1" "$RAW" >> "$LOG" 2>/dev/null || true
rm -f "$RAW"
[ "$BOSS_RC" -ne 0 ] && { echo "STAGE_FAIL headless_boss=$BOSS_RC" >> "$LOG"; FAILED=1; }

# ---- Upstream discipline (NFR-M4-1 / NFR-M4-4) ---------------------------
echo "=== upstream tree guards ===" >> "$LOG"
JS_STATUS=$(git status --short ../src/ ../server/ ../tools/ ../docs/ 2>/dev/null)
[ -z "$JS_STATUS" ] && echo "JS_TREE_CLEAN" >> "$LOG" || { echo "$JS_STATUS" >> "$LOG"; echo "STAGE_FAIL JS_TREE_DIRTY" >> "$LOG"; FAILED=1; }
SIM_DIFF=$(git diff --stat crates/inkwave_sim/ 2>/dev/null)
[ -z "$SIM_DIFF" ] && echo "SIM_TREE_CLEAN" >> "$LOG" || { echo "$SIM_DIFF" >> "$LOG"; echo "STAGE_FAIL SIM_TREE_DIRTY" >> "$LOG"; FAILED=1; }

# ---- Task 41..56 gate matrix (final log per task) -------------------------
# Legacy gate-task41..53 logs predate the auto-verdict block. A legacy log is
# accepted ONLY when: every mandatory stage exists and is zero; no hard-failure
# marker exists; no *_EXIT is non-zero except TRUNK; TRUNK non-zero has an
# explicit SIGKILL/DEGRADED record; and a first-pass DETERM MISMATCH is paired
# with the documented "commit-stamp refresh → BYTE_IDENTICAL (rerun)" evidence
# (Task 48/49/50/52/55 precedent).
exit_code() {
  grep -m1 "^$1=" "$2" | sed -E 's/^[A-Z_]+=([0-9]+).*/\1/'
}

legacy_ok() {
  local f="$1" n="$2" key v nz tr
  for key in FMT CLIPPY TEST WASM TREE HEADLESS; do
    v=$(exit_code "${key}_EXIT" "$f")
    [ "$v" = "0" ] || return 1
  done
  # DETERM and TRUNK stage records must EXIST (all Task 41..56 final logs have
  # them); a missing stage can no longer pass by simply having no failure line.
  for key in DETERM_EXIT TRUNK_EXIT; do
    v=$(exit_code "$key" "$f")
    [ -n "$v" ] || return 1
  done
  v=$(exit_code XCHK_EXIT "$f")
  if [ "$n" -ge 42 ]; then
    # XCHK became a standalone stage from Task 42 onward; Task 41's gate
    # script predates it and is the single documented historical exception.
    [ "$v" = "0" ] || return 1
  else
    [ -z "$v" ] || [ "$v" = "0" ] || return 1
  fi
  grep -qE 'GATE_VERDICT=FAIL|STAGE_FAIL|JS_TREE_DIRTY|SIM_TREE_DIRTY|HEADLESS_COMBO .*EXIT=[1-9]|HEADLESS_BOSS_EXIT=[1-9]|RELEASE_BUILD_EXIT=[1-9]' "$f" && return 1
  nz=$(grep -E '^[A-Z_]+_EXIT=[1-9]' "$f" | grep -v '^TRUNK_EXIT=' | wc -l)
  if [ "$nz" -gt 0 ]; then
    # Only the documented DETERM commit-stamp refresh is excusable.
    [ "$nz" -eq 1 ] && grep -q '^DETERM_EXIT=1' "$f" \
      && grep -q 'DETERM=BYTE_IDENTICAL (rerun)' "$f" || return 1
  fi
  tr=$(exit_code TRUNK_EXIT "$f")
  if [ "${tr:-0}" != "0" ]; then
    grep -qE 'signal: 9|sigkill_hits=[1-9]|TRUNK=DEGRADED' "$f" || return 1
  fi
  return 0
}

echo "=== M4 nine-stage gate matrix (final log per task) ===" >> "$LOG"
for n in $(seq 41 56); do
  f=$(ls -1 tools/verify/logs/task${n}-gates-*.log 2>/dev/null | sort | tail -1)
  if [ -z "$f" ]; then
    printf 'Task %-3s NO_LOG\n' "$n" >> "$LOG"
    echo "STAGE_FAIL gate_task${n}=NO_LOG" >> "$LOG"
    FAILED=1
    continue
  fi
  verdict=$(grep '^GATE_VERDICT=' "$f" | tail -1 | cut -d= -f2)
  if [ -z "$verdict" ]; then
    if legacy_ok "$f" "$n"; then verdict="LEGACY_SEGMENTS_OK"; else verdict="LEGACY_SEGMENTS_FAIL"; fi
  fi
  tr=$(exit_code TRUNK_EXIT "$f")
  stamp=$(grep -c 'DETERM=BYTE_IDENTICAL (rerun)' "$f" 2>/dev/null || true)
  printf 'Task %-3s %-44s verdict=%-21s trunk_exit=%-3s stamp_refresh=%s\n' \
    "$n" "$(basename "$f")" "$verdict" "${tr:-n/a}" "${stamp:-0}" >> "$LOG"
  case "$verdict" in
    PASS|LEGACY_SEGMENTS_OK) ;;
    *) echo "STAGE_FAIL gate_task$n=$verdict" >> "$LOG"; FAILED=1 ;;
  esac
done
echo "=== TRUNK in the closing matrix itself ===" >> "$LOG"
TRUNK_SUMMARY=$(grep -E '^TRUNK=DEGRADED|^TRUNK_EXIT=' "$LOG" 2>/dev/null || true)
echo "$TRUNK_SUMMARY" >> "$LOG"

echo "=== verdict ===" >> "$LOG"
if [ "$FAILED" -eq 0 ]; then
  echo "M4_MATRIX_VERDICT=PASS" >> "$LOG"
else
  echo "M4_MATRIX_VERDICT=FAIL" >> "$LOG"
fi
echo "MATRIX_DONE" >> "$LOG"
echo "Matrix log: $LOG"
tail -24 "$LOG"
exit "$FAILED"
