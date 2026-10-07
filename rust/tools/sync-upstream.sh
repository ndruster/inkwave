#!/usr/bin/env bash
#
# sync-upstream.sh — track new commits in upstream INKWAVE for the Rust port.
#
# The JavaScript tree in this repo is an untouched fork of
# https://github.com/jaydendavisnc/inkwave (git remote `upstream`). Rust code
# cannot `git merge` JS changes, so every upstream change is ported by hand:
# this script fetches upstream and lists what arrived since the last recorded
# baseline; the actual ports are tracked in rust/SYNC_LEDGER.md.
#
# Usage:
#   tools/sync-upstream.sh check             # fetch + list unported commits
#   tools/sync-upstream.sh update-baseline   # advance baseline to upstream/main
#
# Environment:
#   INKWAVE_UPSTREAM_REMOTE  git remote name to use (default: upstream)
#
# The remote itself is HTTPS by default (works for a public repo without a
# key). On a machine where github.com HTTPS is unreachable, either configure a
# repo-local URL rewrite (e.g. over SSH) or export the variable above.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
BASELINE_FILE="$RUST_DIR/.upstream-baseline"
REMOTE="${INKWAVE_UPSTREAM_REMOTE:-upstream}"
BRANCH="main"

cd "$REPO_ROOT"

[[ -f "$BASELINE_FILE" ]] || {
  echo "error: baseline file missing: $BASELINE_FILE" >&2
  exit 2
}
BASELINE="$(tr -d '[:space:]' <"$BASELINE_FILE")"
[[ ${#BASELINE} -eq 40 ]] || {
  echo "error: baseline is not a 40-char commit hash: '$BASELINE'" >&2
  exit 2
}
git cat-file -e "$BASELINE^{commit}" 2>/dev/null || {
  echo "error: baseline commit $BASELINE not found in this clone (run: git fetch $REMOTE)" >&2
  exit 2
}

cmd="${1:-check}"
case "$cmd" in
  check)
    echo "== fetching $REMOTE/$BRANCH =="
    if ! git fetch "$REMOTE" --no-tags --quiet; then
      echo "warning: fetch failed (network?); showing commits known locally" >&2
    fi
    git cat-file -e "$REMOTE/$BRANCH^{commit}" 2>/dev/null || {
      echo "error: $REMOTE/$BRANCH not available locally and fetch failed" >&2
      exit 3
    }
    TIP="$(git rev-parse "$REMOTE/$BRANCH")"
    echo "baseline : $BASELINE"
    echo "upstream : $TIP ($REMOTE/$BRANCH)"
    if [[ "$BASELINE" == "$TIP" ]]; then
      echo "result   : up to date — no upstream commits to port"
      exit 0
    fi
    RANGE="$BASELINE..$TIP"
    N="$(git rev-list --count "$RANGE")"
    echo "result   : $N commit(s) to triage — port the impactful ones and track them in rust/SYNC_LEDGER.md"
    echo
    git log --reverse --date=short --format='%h %ad %s' "$RANGE" |
      while IFS= read -r line; do
        sha="${line%% *}"
        full="$(git rev-parse "$sha")"
        echo "● $line"
        # surface impacted areas (top-level dirs / files), one line each
        git diff-tree --no-commit-id --name-only -r "$full" |
          awk -F/ '{print "    " $1}' | sort | uniq -c | sort -rn | sed 's/^/   /'
      done
    echo
    echo "next: port each change (see rust/PORT_MAP.md), mark it in rust/SYNC_LEDGER.md,"
    echo "      then: $0 update-baseline"
    ;;

  update-baseline)
    if ! git fetch "$REMOTE" --no-tags --quiet; then
      echo "warning: fetch failed; updating to locally known $REMOTE/$BRANCH" >&2
    fi
    NEW="$(git rev-parse "$REMOTE/$BRANCH")"
    if [[ "$NEW" == "$BASELINE" ]]; then
      echo "already at baseline $BASELINE"
      exit 0
    fi
    echo "$NEW" >"$BASELINE_FILE"
    echo "baseline advanced:"
    echo "  old: $BASELINE"
    echo "  new: $NEW"
    echo "remember to add a row to rust/SYNC_LEDGER.md for every ported commit."
    ;;

  *)
    echo "usage: $0 {check|update-baseline}" >&2
    exit 64
    ;;
esac
