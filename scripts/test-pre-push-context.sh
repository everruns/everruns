#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
source "$PROJECT_ROOT/scripts/lib/pre-push-context.sh"

assert_true() {
  local description="$1"
  shift
  if ! "$@"; then
    echo "FAIL: $description" >&2
    exit 1
  fi
}

assert_false() {
  local description="$1"
  shift
  if "$@"; then
    echo "FAIL: $description" >&2
    exit 1
  fi
}

PRE_PUSH_CHANGED_FILES=$'docs/guide.md\ncrates/server/src/lib.rs'
assert_true "Rust source selects Rust checks" pre_push_rust_changed
assert_false "server source does not select the core API guard" pre_push_core_api_changed
assert_false "non-UI changes do not select UI checks" pre_push_ui_changed
assert_false "source-only changes do not fetch the Cargo graph" pre_push_cargo_graph_changed

PRE_PUSH_CHANGED_FILES=$'apps/ui/src/app/page.tsx'
assert_true "UI source selects UI checks" pre_push_ui_changed
assert_false "UI source does not select Rust checks" pre_push_rust_changed

PRE_PUSH_CHANGED_FILES=$'crates/core/src/lib.rs'
assert_true "core source selects the core API guard" pre_push_core_api_changed

PRE_PUSH_CHANGED_FILES=$'integrations/example/Cargo.toml'
assert_true "nested manifests select Rust checks" pre_push_rust_changed
assert_true "nested manifests refresh the Cargo graph" pre_push_cargo_graph_changed

PRE_PUSH_CHANGED_FILES=$'scripts/lib/pre-push.sh'
assert_true "pre-push workflow changes exercise Rust checks" pre_push_rust_changed
assert_true "pre-push workflow changes exercise UI checks" pre_push_ui_changed
assert_true "pre-push workflow changes exercise the core API guard" pre_push_core_api_changed

common_git_dir="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
cache_key="$(printf '%s' "$common_git_dir" | cksum | awk '{print $1}')"
temp_root="${TMPDIR:-/tmp}"
expected_target="${temp_root%/}/everruns-build-$(id -u)/$cache_key/pre-push-target"
actual_target="$(pre_push_shared_target_dir)"
if [ "$actual_target" != "$expected_target" ]; then
  echo "FAIL: shared target mismatch: $actual_target != $expected_target" >&2
  exit 1
fi

cache_test_root="$(mktemp -d)"
mkdir "$cache_test_root/everruns-build-$(id -u)"
chmod 0777 "$cache_test_root/everruns-build-$(id -u)"
if TMPDIR="$cache_test_root" pre_push_shared_target_dir >/dev/null 2>&1; then
  echo "FAIL: insecure pre-created cache root was accepted" >&2
  exit 1
fi
rm -rf "$cache_test_root"

cache_test_root="$(mktemp -d)"
mkdir "$cache_test_root/attacker-target"
ln -s "$cache_test_root/attacker-target" "$cache_test_root/everruns-build-$(id -u)"
if TMPDIR="$cache_test_root" pre_push_shared_target_dir >/dev/null 2>&1; then
  echo "FAIL: symlinked cache root was accepted" >&2
  exit 1
fi
rm -rf "$cache_test_root"

(
  CARGO_TARGET_DIR="/tmp/everruns-custom-target"
  configure_pre_push_build_cache
  if [ "$CARGO_TARGET_DIR" != "/tmp/everruns-custom-target" ]; then
    echo "FAIL: explicit CARGO_TARGET_DIR was replaced" >&2
    exit 1
  fi
)

temp_repo="$(mktemp -d)"
trap 'rm -rf "$temp_repo"' EXIT
(
  cd "$temp_repo"
  git init -q
  git config user.name "Mykhailo Chalyi"
  git config user.email "mike@chaliy.name"
  mkdir -p crates/example/src apps/ui
  printf '%s\n' 'fn original() {}' > crates/example/src/lib.rs
  printf '%s\n' '{}' > apps/ui/package.json
  git add crates/example/src/lib.rs apps/ui/package.json
  git commit -qm "test: establish comparison base"
  git update-ref refs/remotes/origin/main HEAD
  rm crates/example/src/lib.rs
  printf '%s\n' 'export {}' > apps/ui/new.ts

  changed_files="$(pre_push_collect_changed_files)"
  if [ "$changed_files" != $'apps/ui/new.ts\ncrates/example/src/lib.rs' ]; then
    echo "FAIL: changed-file collection missed deleted or untracked files: $changed_files" >&2
    exit 1
  fi
)

echo "Pre-push context tests passed."
