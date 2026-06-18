#!/usr/bin/env bash
# SessionStart hook: override agent-like git identity with real user.
# Runs automatically when a Claude Code session starts.

set -euo pipefail

# Only act inside a git repo
git rev-parse --git-dir >/dev/null 2>&1 || exit 0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$REPO_ROOT/scripts/lib/common.sh"

configure_commit_git_identity_if_needed 2>/dev/null || true

# Do not change commit.gpgsign by default. Contributors may have valid global
# or repository-local signing policies, and an automatic SessionStart hook must
# not silently weaken those source-control integrity settings. Cloud sessions
# that intentionally need to bypass a broken inherited signing setup can opt in.
if [ "${EVERRUNS_DISABLE_COMMIT_SIGNING:-}" = "1" ]; then
  git -C "$REPO_ROOT" config --local commit.gpgsign false 2>/dev/null || true
fi
