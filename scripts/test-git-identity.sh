#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck disable=SC1091
source "$PROJECT_ROOT/scripts/lib/common.sh"

assert_eq() {
  local expected="$1"
  local actual="$2"
  local label="$3"

  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $label expected '$expected', got '$actual'" >&2
    exit 1
  fi
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"

  if [[ "$haystack" != *"$needle"* ]]; then
    echo "FAIL: $label expected to contain '$needle', got '$haystack'" >&2
    exit 1
  fi
}

with_isolated_git_env() {
  local dir="$1"

  export HOME="$dir/home"
  export XDG_CONFIG_HOME="$dir/xdg"
  export GIT_CONFIG_GLOBAL="$dir/global.gitconfig"
  export GIT_CONFIG_NOSYSTEM=1
  mkdir -p "$HOME" "$XDG_CONFIG_HOME"
  : > "$GIT_CONFIG_GLOBAL"
}

make_repo() {
  local dir="$1"

  git init -q -b main "$dir/repo"
  cd "$dir/repo"
  git config commit.gpgsign false
  git config user.name "Human Base"
  git config user.email "human-base@example.com"
  touch README.md
  git add README.md
  git commit -q -m "chore: base"
  git update-ref refs/remotes/origin/main HEAD
}

test_human_git_config_without_env_uses_git_identity() {
  local tmp
  tmp="$(mktemp -d)"
  (
    with_isolated_git_env "$tmp"
    make_repo "$tmp"
    git config user.name "Human Example"
    git config user.email "human@example.com"
    unset GIT_USER_NAME GIT_USER_EMAIL || true

    resolve_commit_git_identity

    assert_eq "Human Example" "$RESOLVED_GIT_AUTHOR_NAME" "human name"
    assert_eq "human@example.com" "$RESOLVED_GIT_AUTHOR_EMAIL" "human email"
    assert_eq "git" "$RESOLVED_GIT_AUTHOR_SOURCE" "human source"
  )
  rm -rf "$tmp"
}

test_agent_git_config_requires_env_override() {
  local tmp output
  tmp="$(mktemp -d)"
  (
    with_isolated_git_env "$tmp"
    make_repo "$tmp"
    git config user.name "Claude"
    git config user.email "claude@example.com"
    unset GIT_USER_NAME GIT_USER_EMAIL || true

    if output="$(resolve_commit_git_identity 2>&1)"; then
      echo "FAIL: resolve_commit_git_identity should fail for agent-like git config without env" >&2
      exit 1
    fi

    assert_contains "$output" "set GIT_USER_NAME and GIT_USER_EMAIL" "agent override message"
  )
  rm -rf "$tmp"
}

test_agent_git_config_uses_human_env_override() {
  local tmp
  tmp="$(mktemp -d)"
  (
    with_isolated_git_env "$tmp"
    make_repo "$tmp"
    git config user.name "Cursor"
    git config user.email "cursor@example.com"
    export GIT_USER_NAME="Mykhailo Chalyi"
    export GIT_USER_EMAIL="mike@chaliy.name"

    resolve_commit_git_identity

    assert_eq "Mykhailo Chalyi" "$RESOLVED_GIT_AUTHOR_NAME" "env override name"
    assert_eq "mike@chaliy.name" "$RESOLVED_GIT_AUTHOR_EMAIL" "env override email"
    assert_eq "env" "$RESOLVED_GIT_AUTHOR_SOURCE" "env override source"
  )
  rm -rf "$tmp"
}

test_agent_like_outgoing_commit_is_detected() {
  local tmp offender
  tmp="$(mktemp -d)"
  (
    with_isolated_git_env "$tmp"
    make_repo "$tmp"
    git checkout -q -b fix/test
    printf 'change\n' >> README.md
    git add README.md
    GIT_AUTHOR_NAME="Claude" \
      GIT_AUTHOR_EMAIL="claude@example.com" \
      GIT_COMMITTER_NAME="Claude" \
      GIT_COMMITTER_EMAIL="claude@example.com" \
      git commit -q -m "fix: agent authored"

    offender="$(find_agent_like_outgoing_commit)"

    assert_contains "$offender" "Claude" "offending author name"
    assert_contains "$offender" "claude@example.com" "offending author email"
  )
  rm -rf "$tmp"
}

test_human_outgoing_commit_is_allowed() {
  local tmp
  tmp="$(mktemp -d)"
  (
    with_isolated_git_env "$tmp"
    make_repo "$tmp"
    git checkout -q -b fix/test
    printf 'change\n' >> README.md
    git add README.md
    git commit -q -m "fix: human authored"

    if find_agent_like_outgoing_commit >/dev/null; then
      echo "FAIL: human-authored outgoing commit should not be flagged" >&2
      exit 1
    fi
  )
  rm -rf "$tmp"
}

test_human_git_config_without_env_uses_git_identity
test_agent_git_config_requires_env_override
test_agent_git_config_uses_human_env_override
test_agent_like_outgoing_commit_is_detected
test_human_outgoing_commit_is_allowed

echo "git identity checks passed"
