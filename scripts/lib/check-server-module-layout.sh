#!/usr/bin/env bash
# Layout guard: the server crate's module tree is the folder tree.
#
# Rule, two parts:
#   1. A module with children is a folder with `mod.rs`. A `foo.rs` file next
#      to a `foo/` folder is not allowed: move it to `foo/mod.rs`.
#   2. No path attributes on modules. A test file is a child module
#      (`foo/tests.rs` declared as `mod tests;` in `foo/mod.rs`), not a
#      `foo_tests.rs` sibling pulled in with a path attribute.
#
# Why: with both rules a module path names exactly one file, and a file's
# location names its module. Path attributes and `foo.rs` + `foo/` pairs let the
# two drift apart (overflow folders loaded from the parent, `*_tests.rs`
# siblings, a child file living in a different folder from its parent), so
# finding a module's code meant reading the attributes first.
#
# The rule is absolute: there is no allowlist. Every match fails, including one
# in a comment; name the attribute as "path attribute" in prose instead.
#
# Caught forms, in any `.rs` file under the guarded tree:
#   #[path = "..."]                      the attribute itself
#   #[cfg_attr(..., path = "...")]       the conditional form
#   foo.rs beside foo/                   any non-mod.rs file with a same-named folder,
#                                        whatever that folder holds
#
# Usage: check-server-module-layout.sh [SERVER_SRC_DIR]
#   SERVER_SRC_DIR defaults to crates/server/src (used by the shell test).
#
# Used by: scripts/lib/pre-push.sh, the `server-module-layout` CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SRC="${1:-crates/server/src}"

if [ ! -f "$SRC/lib.rs" ]; then
  echo "Server module layout guard: missing $SRC/lib.rs; the guard would check nothing." >&2
  exit 1
fi

FAILED=0

PATTERN='#!?\[[[:space:]]*path\b|cfg_attr[[:space:]]*\(.*\bpath[[:space:]]*='
if matches=$(grep -rnE "$PATTERN" "$SRC" --include='*.rs'); then
  echo "Server modules must not use path attributes (make the file a child module instead):"
  echo "$matches" | sed 's/^/  /'
  FAILED=1
fi

pairs=$(find "$SRC" -type f -name '*.rs' ! -name 'mod.rs' -print \
  | while IFS= read -r file; do
      if [ -d "${file%.rs}" ]; then
        printf '%s\n' "$file"
      fi
    done \
  | sort)
if [ -n "$pairs" ]; then
  echo "A module with a folder must be that folder's mod.rs (move foo.rs to foo/mod.rs):"
  while IFS= read -r file; do
    echo "  $file beside ${file%.rs}/"
  done <<<"$pairs"
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Server module layout guard failed."
  exit 1
fi

files=$(find "$SRC" -type f -name '*.rs' | wc -l | tr -d ' ')
echo "Server module layout guard passed: $files files under $SRC, no path attributes, no foo.rs beside foo/."
