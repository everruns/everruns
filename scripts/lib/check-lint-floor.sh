#!/usr/bin/env bash
# Ratchet membership of the clippy lint floor.
#
# Why: `[workspace.lints.clippy]` denies `unwrap_used` and `expect_used`, but
# Cargo only applies those to a crate that opts in with `[lints] workspace =
# true`. Crates still carrying production unwrap/expect debt are deliberately
# not opted in — denying there would fail the build on ~1000 existing sites, and
# warning there would drown every other warning in the build.
#
# That leaves opting in voluntary, which means a new crate silently skips the
# floor and grows its own debt. This guard makes membership one-way: a crate may
# join the floor, a new crate must, and none may leave.
#
# The allowlist is the debt list: every crate named there is exempt because it
# has production debt today, not because exemption is a choice.
#
# Usage:
#   check-lint-floor.sh            # verify (pre-push, CI)
#   check-lint-floor.sh --report   # show who is on the floor and who owes
#
# Used by: scripts/lib/pre-push.sh, the `lint-floor` CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

ALLOWLIST="scripts/lib/lint-floor-allowlist.txt"

if [ ! -f "$ALLOWLIST" ]; then
  echo "Lint floor allowlist missing: $ALLOWLIST"
  exit 1
fi

if ! grep -q '^\[workspace.lints.clippy\]' Cargo.toml; then
  echo "  - Cargo.toml: [workspace.lints.clippy] is gone; the lint floor is not defined"
  echo "Lint floor guard failed."
  exit 1
fi
for lint in unwrap_used expect_used; do
  if ! grep -qE "^${lint} = \"deny\"" Cargo.toml; then
    echo "  - Cargo.toml: workspace lint '$lint' is no longer denied"
    echo "Lint floor guard failed."
    exit 1
  fi
done

# Workspace member directories, relative to the repository root.
members() {
  cargo metadata --no-deps --format-version 1 \
    | python3 -c '
import json, pathlib, sys
root = pathlib.Path.cwd()
for package in json.load(sys.stdin)["packages"]:
    print(pathlib.Path(package["manifest_path"]).parent.relative_to(root))
' | sort
}

opted_in() {
  # `[lints]` followed by `workspace = true`, tolerating comments between them.
  awk '
    /^\[lints\]/ { inlints = 1; next }
    /^\[/        { inlints = 0 }
    inlints && /^workspace[[:space:]]*=[[:space:]]*true/ { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$1/Cargo.toml"
}

declare -A EXEMPT=()
while read -r crate; do
  case "$crate" in
    ''|'#'*) continue ;;
  esac
  EXEMPT["$crate"]=1
done < "$ALLOWLIST"

FAILED=0
ON_FLOOR=0
declare -A SEEN=()

while read -r dir; do
  [ -n "$dir" ] || continue
  if opted_in "$dir"; then
    ON_FLOOR=$((ON_FLOOR + 1))
    if [ -n "${EXEMPT[$dir]:-}" ]; then
      SEEN["$dir"]=1
      echo "  - $dir: on the lint floor but still listed in $ALLOWLIST"
      echo "      remove its entry to bank the win"
      FAILED=1
    fi
  elif [ -n "${EXEMPT[$dir]:-}" ]; then
    SEEN["$dir"]=1
  else
    echo "  - $dir: not on the clippy lint floor"
    echo "      add '[lints]' with 'workspace = true' to $dir/Cargo.toml, and"
    echo "      '#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]'"
    echo "      to each of its target roots (src/lib.rs, src/main.rs, tests/*.rs)"
    FAILED=1
  fi
done < <(members)

if [ "${1:-}" = "--report" ]; then
  echo "On the lint floor: $ON_FLOOR"
  echo "Exempt (production unwrap/expect debt): ${#EXEMPT[@]}"
fi

for crate in "${!EXEMPT[@]}"; do
  if [ -z "${SEEN[$crate]:-}" ]; then
    echo "  - $ALLOWLIST: stale entry for a crate that is not a workspace member: $crate"
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Lint floor guard failed."
  exit 1
fi

echo "Lint floor guard passed: $ON_FLOOR crates deny unwrap/expect, ${#EXEMPT[@]} exempt with recorded debt."
