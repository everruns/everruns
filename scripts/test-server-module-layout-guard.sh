#!/usr/bin/env bash
# Exercise scripts/lib/check-server-module-layout.sh against scratch trees.
#
# The guard is only worth having if it bites on every way the module tree can
# drift from the folder tree (a path attribute, its cfg_attr form, a foo.rs
# beside a foo/ folder at any depth, even one holding only assets) and stays
# quiet on the layouts the rule asks for and on look-alikes such as a
# `#[path_like]` attribute or a `foo_tests.rs` file with no folder.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
GUARD="$(pwd)/scripts/lib/check-server-module-layout.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

# A scratch project root with a conforming server tree.
scratch() {
  local dir="$WORK/root"
  rm -rf "$dir"
  mkdir -p "$dir/scripts/lib" "$dir/crates/server/src/api/sessions"
  cp "$GUARD" "$dir/scripts/lib/check-server-module-layout.sh"
  local src="$dir/crates/server/src"
  printf 'pub mod api;\nmod leaf;\n' > "$src/lib.rs"
  printf 'pub mod sessions;\n' > "$src/api/mod.rs"
  printf '#[cfg(test)]\nmod tests;\n' > "$src/api/sessions/mod.rs"
  printf 'use super::*;\n' > "$src/api/sessions/tests.rs"
  printf 'fn f() -> std::path::PathBuf { std::path::PathBuf::from("x") }\n' > "$src/leaf.rs"
  printf '%s\n' "$dir"
}

expect() {
  local name="$1" expected="$2" dir="$3" needle="${4:-}"
  local output status
  set +e
  output="$(cd "$dir" && bash scripts/lib/check-server-module-layout.sh 2>&1)"
  status=$?
  set -e
  if [ "$expected" = "pass" ] && [ "$status" -ne 0 ]; then
    echo "FAIL $name: expected the guard to pass, got:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  if [ "$expected" = "fail" ] && [ "$status" -eq 0 ]; then
    echo "FAIL $name: expected the guard to fail, it passed:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  if [ -n "$needle" ] && ! grep -qF "$needle" <<<"$output"; then
    echo "FAIL $name: expected output to mention '$needle', got:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

ROOT="$(scratch)"
expect "conforming tree passes" pass "$ROOT"

ROOT="$(scratch)"
printf '#[cfg(test)]\n#[path = "leaf_tests.rs"]\nmod tests;\n' >> "$ROOT/crates/server/src/leaf.rs"
printf 'use super::*;\n' > "$ROOT/crates/server/src/leaf_tests.rs"
expect "path attribute fails" fail "$ROOT" "src/leaf.rs:3"

ROOT="$(scratch)"
printf '#[path="other.rs"] mod other;\n' >> "$ROOT/crates/server/src/api/sessions/tests.rs"
expect "compact path attribute in a nested file fails" fail "$ROOT" "sessions/tests.rs:2"

ROOT="$(scratch)"
printf '#[cfg_attr(test, path = "leaf_alt.rs")]\nmod alt;\n' >> "$ROOT/crates/server/src/leaf.rs"
expect "cfg_attr path fails" fail "$ROOT" "src/leaf.rs:2"

ROOT="$(scratch)"
mkdir -p "$ROOT/crates/server/src/leaf"
printf 'fn g() {}\n' > "$ROOT/crates/server/src/leaf/child.rs"
expect "foo.rs beside foo/ fails" fail "$ROOT" "src/leaf.rs beside"

ROOT="$(scratch)"
mkdir -p "$ROOT/crates/server/src/api/sessions/deep/inner"
printf 'mod inner;\n' > "$ROOT/crates/server/src/api/sessions/deep.rs"
printf 'fn h() {}\n' > "$ROOT/crates/server/src/api/sessions/deep/inner.rs"
expect "nested foo.rs beside foo/ fails" fail "$ROOT" "sessions/deep.rs beside"

ROOT="$(scratch)"
mkdir -p "$ROOT/crates/server/src/leaf"
printf '<html></html>\n' > "$ROOT/crates/server/src/leaf/app.html"
expect "foo.rs beside an asset-only foo/ fails" fail "$ROOT" "src/leaf.rs beside"

ROOT="$(scratch)"
printf 'use super::*;\n' > "$ROOT/crates/server/src/api/other_tests.rs"
printf '// A path attribute used to load this file.\n#[path_like]\nfn k() {}\n' \
  >> "$ROOT/crates/server/src/leaf.rs"
expect "look-alikes and prose pass" pass "$ROOT"

ROOT="$(scratch)"
rm -f "$ROOT/crates/server/src/lib.rs"
expect "a missing crate root fails instead of checking nothing" fail "$ROOT" "missing"

if [ "$failures" -ne 0 ]; then
  echo "$failures server module layout guard case(s) failed."
  exit 1
fi
echo "Server module layout guard tests passed."
