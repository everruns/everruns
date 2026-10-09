#!/usr/bin/env bash
# Exercise scripts/lib/check-server-api-layering.sh against scratch trees.
#
# The guard is only worth having if it bites on every import form a lower
# layer could use to reach `crate::api` (a path, a bare module, a relative
# `super::` climb, a grouped `use crate::{api, ...}` across lines) and stays
# quiet on look-alikes such as `crate::api_keys` or a local `api_` identifier.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
GUARD="$(pwd)/scripts/lib/check-server-api-layering.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

# A scratch project root with the four guarded layers and one clean file each.
scratch() {
  local dir="$WORK/root"
  rm -rf "$dir"
  mkdir -p "$dir/scripts/lib"
  cp "$GUARD" "$dir/scripts/lib/check-server-api-layering.sh"
  for layer in domains storage services records; do
    mkdir -p "$dir/crates/server/src/$layer"
    printf 'use crate::records::common::Pagination;\n' > "$dir/crates/server/src/$layer/clean.rs"
  done
  printf '%s\n' "$dir"
}

expect() {
  local name="$1" expected="$2" dir="$3" needle="${4:-}"
  local output status
  set +e
  output="$(cd "$dir" && bash scripts/lib/check-server-api-layering.sh 2>&1)"
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
expect "clean layers pass" pass "$ROOT"

ROOT="$(scratch)"
printf 'use crate::api::common::ErrorResponse;\n' > "$ROOT/crates/server/src/domains/bad.rs"
expect "use crate::api path fails" fail "$ROOT" "domains/bad.rs:1"

ROOT="$(scratch)"
printf 'fn f() { let _ = crate::api::validation::MAX; }\n' > "$ROOT/crates/server/src/storage/bad.rs"
expect "inline crate::api path fails" fail "$ROOT" "storage/bad.rs:1"

ROOT="$(scratch)"
printf 'use crate::api;\n' > "$ROOT/crates/server/src/services/bad.rs"
expect "bare crate::api module fails" fail "$ROOT" "services/bad.rs:1"

ROOT="$(scratch)"
printf 'use super::super::api::common::Pagination;\n' > "$ROOT/crates/server/src/records/bad.rs"
expect "super::super::api fails" fail "$ROOT" "records/bad.rs:1"

ROOT="$(scratch)"
printf 'use everruns_server::api::sessions::CreateSessionRequest;\n' \
  > "$ROOT/crates/server/src/domains/bad_tests.rs"
expect "everruns_server::api fails" fail "$ROOT" "domains/bad_tests.rs:1"

ROOT="$(scratch)"
printf 'use crate::{\n    api,\n    records::Agent,\n};\n' > "$ROOT/crates/server/src/domains/grouped.rs"
expect "multi-line grouped use crate::{api} fails" fail "$ROOT" "domains/grouped.rs"

ROOT="$(scratch)"
printf 'use crate::{api::common::ListResponse, records::Agent};\n' \
  > "$ROOT/crates/server/src/services/grouped.rs"
expect "single-line grouped use crate::{api::...} fails" fail "$ROOT" "services/grouped.rs"

ROOT="$(scratch)"
mkdir -p "$ROOT/crates/server/src/domains/nested/deep"
printf '// See crate::api::sessions for the handler.\n' \
  > "$ROOT/crates/server/src/domains/nested/deep/comment.rs"
expect "comment naming crate::api in a nested file fails" fail "$ROOT" "nested/deep/comment.rs:1"

ROOT="$(scratch)"
printf 'use crate::api_keys::ApiKey;\nuse crate::{records::api_tokens, storage};\nfn api() {}\nfn g() { let _ = self::api(); }\n' \
  > "$ROOT/crates/server/src/domains/lookalike.rs"
printf '// The HTTP layer (`api::sessions`) re-exports these.\n' \
  > "$ROOT/crates/server/src/records/prose.rs"
expect "look-alikes and api:: prose pass" pass "$ROOT"

ROOT="$(scratch)"
rm -rf "$ROOT/crates/server/src/records"
expect "a missing layer fails instead of checking nothing" fail "$ROOT" "missing"

if [ "$failures" -ne 0 ]; then
  echo "$failures server API layering guard case(s) failed."
  exit 1
fi
echo "Server API layering guard tests passed."
