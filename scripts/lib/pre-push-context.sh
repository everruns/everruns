#!/usr/bin/env bash

# Scope expensive pre-push checks to the files that can affect them. The cheap
# repository guards still run every time.

pre_push_collect_changed_files() {
  local base

  if git rev-parse --verify -q '@{upstream}' >/dev/null 2>&1; then
    base="$(git merge-base HEAD '@{upstream}')"
  elif git rev-parse --verify -q 'origin/main' >/dev/null 2>&1; then
    base="$(git merge-base HEAD origin/main)"
  else
    return 1
  fi

  {
    git diff --name-only --diff-filter=ACDMRTUXB "$base" HEAD
    git diff --name-only --diff-filter=ACDMRTUXB
    git diff --cached --name-only --diff-filter=ACDMRTUXB
    git ls-files --others --exclude-standard
  } | sed '/^$/d' | LC_ALL=C sort -u
}

pre_push_changed_files_match() {
  local pattern="$1"
  printf '%s\n' "$PRE_PUSH_CHANGED_FILES" | grep -Eq -- "$pattern"
}

pre_push_workflow_changed() {
  pre_push_changed_files_match '^(justfile|scripts/lib/pre-push(-context)?\.sh)$'
}

pre_push_rust_changed() {
  pre_push_workflow_changed ||
    pre_push_changed_files_match '(^|/)Cargo\.(toml|lock)$|(^|/)[^/]+\.rs$|^rust-toolchain(\.toml)?$|^\.cargo/'
}

pre_push_cargo_graph_changed() {
  pre_push_workflow_changed ||
    pre_push_changed_files_match '(^|/)Cargo\.(toml|lock)$|^rust-toolchain(\.toml)?$|^\.cargo/'
}

pre_push_ui_changed() {
  pre_push_workflow_changed || pre_push_changed_files_match '^apps/ui/'
}

pre_push_core_api_changed() {
  pre_push_workflow_changed ||
    pre_push_changed_files_match '^crates/(core|engine)/|^scripts/lib/check-core-public-api\.sh$|^Cargo\.(toml|lock)$|^rust-toolchain(\.toml)?$'
}

# One stat field, on either stat. `stat -c` is GNU and `stat -f` is BSD, and
# `just pre-push` runs on macOS as well as Linux, so asking for only one of them
# would fail the ownership check on the other and refuse every cache root.
pre_push_stat_field() {
  local gnu_format="$1" bsd_format="$2" path="$3"
  stat -c "$gnu_format" "$path" 2>/dev/null ||
    stat -f "$bsd_format" "$path" 2>/dev/null
}

pre_push_shared_target_dir() {
  local cache_key cache_root common_git_dir mode owner_id temp_root

  common_git_dir="$(git rev-parse --git-common-dir)"
  if [[ "$common_git_dir" != /* ]]; then
    common_git_dir="$PROJECT_ROOT/$common_git_dir"
  fi
  common_git_dir="$(cd "$common_git_dir" && pwd -P)"
  cache_key="$(printf '%s' "$common_git_dir" | cksum | awk '{print $1}')"
  temp_root="${TMPDIR:-/tmp}"
  cache_root="${temp_root%/}/everruns-build-$(id -u)"

  # This directory is a security boundary: Cargo can execute artifacts from
  # its target directory. Refuse attacker-created paths instead of repairing
  # their ownership or permissions.
  if [ ! -e "$cache_root" ] && [ ! -L "$cache_root" ]; then
    # A concurrent pre-push may win creation; validation below decides whether
    # the resulting path is safe.
    (umask 077 && mkdir "$cache_root") 2>/dev/null || true
  fi
  owner_id="$(pre_push_stat_field '%u' '%u' "$cache_root")" || return 1
  mode="$(pre_push_stat_field '%a' '%Lp' "$cache_root")" || return 1
  if [ -L "$cache_root" ] || [ ! -d "$cache_root" ] ||
    [ "$owner_id" != "$(id -u)" ] || [ "$mode" != "700" ]; then
    echo "Refusing insecure pre-push cache root: $cache_root" >&2
    return 1
  fi

  (umask 077 && mkdir -p "$cache_root/$cache_key/pre-push-target") || return 1
  printf '%s\n' "$cache_root/$cache_key/pre-push-target"
}

configure_pre_push_build_cache() {
  if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    CARGO_TARGET_DIR="$(pre_push_shared_target_dir)"
    export CARGO_TARGET_DIR
  fi

  # Pre-push artifacts are reused across worktrees, so incremental state only
  # consumes space without improving the validation result.
  : "${CARGO_INCREMENTAL:=0}"
  export CARGO_INCREMENTAL
}
