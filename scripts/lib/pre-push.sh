#!/usr/bin/env bash
# Pre-push checks: fast local validation to catch CI failures early (~30s).
# Runs formatting, linting, and lockfile checks.
# Usage: just pre-push (or: bash scripts/lib/pre-push.sh)

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

FAILED=0
fail() { echo "   ❌ $1"; FAILED=1; }
pass() { echo "   ✅ $1"; }

echo "🔒 Running pre-push checks..."
echo ""

# 1. Rust formatting
echo "1/7 Rust formatting"
if cargo fmt --check 2>/dev/null; then
  pass "cargo fmt"
else
  fail "cargo fmt — run: cargo fmt"
fi

# 2. Clippy
echo "2/7 Rust linting"
if cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then
  pass "clippy"
else
  fail "clippy — run: cargo clippy --fix --allow-dirty"
fi

# 3. Cargo.lock freshness
echo "3/7 Cargo.lock freshness"
if cargo fetch --locked 2>/dev/null; then
  pass "Cargo.lock up to date"
else
  fail "Cargo.lock outdated — run: cargo fetch"
fi

# 4. UI formatting (skip if node_modules missing)
echo "4/7 UI formatting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && npm run format:check 2>/dev/null); then
    pass "UI format"
  else
    fail "UI format — run: cd apps/ui && npm run format"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

# 5. UI linting (skip if node_modules missing)
echo "5/7 UI linting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && npm run lint 2>/dev/null); then
    pass "UI lint"
  else
    fail "UI lint — run: cd apps/ui && npm run lint -- --fix"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

# 6. Migration ordering check
echo "6/7 Migration ordering"
MIGRATION_DIR="$PROJECT_ROOT/crates/server/migrations"
if [ -d "$MIGRATION_DIR" ]; then
  EXPECTED=1
  ORDER_OK=true
  for f in "$MIGRATION_DIR"/[0-9]*.sql; do
    NUM=$(basename "$f" | grep -oE '^[0-9]+' | sed 's/^0*//')
    if [ "$NUM" != "$EXPECTED" ]; then
      ORDER_OK=false
      break
    fi
    EXPECTED=$((EXPECTED + 1))
  done
  if $ORDER_OK; then
    pass "migrations sequential (001..$((EXPECTED - 1)))"
  else
    fail "migration ordering broken at $(basename "$f") — expected $(printf '%03d' "$EXPECTED"), got $(printf '%03d' "$NUM")"
  fi
else
  echo "   ⏭️  skipped (no migrations dir)"
fi

# 7. Commit author attribution check
echo "7/7 Commit author attribution"
if ! resolve_commit_git_identity; then
  fail "commit identity invalid — fix git config or set GIT_USER_NAME/GIT_USER_EMAIL to a real user"
elif OFFENDING_COMMIT="$(find_agent_like_outgoing_commit)"; then
  IFS=$'\t' read -r OFFENDING_SHA OFFENDING_NAME OFFENDING_EMAIL <<< "$OFFENDING_COMMIT"
  fail "outgoing commit $OFFENDING_SHA has agent-like author '$OFFENDING_NAME <$OFFENDING_EMAIL>'"
else
  pass "commit author ($RESOLVED_GIT_AUTHOR_SOURCE): $RESOLVED_GIT_AUTHOR_NAME <$RESOLVED_GIT_AUTHOR_EMAIL>"
fi

echo ""
if [ $FAILED -ne 0 ]; then
  echo "❌ Pre-push checks failed. Fix issues above."
  echo "   Auto-fix: just fmt"
  exit 1
fi
echo "✅ All pre-push checks passed."
