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
echo "1/6 Rust formatting"
if cargo fmt --check 2>/dev/null; then
  pass "cargo fmt"
else
  fail "cargo fmt — run: cargo fmt"
fi

# 2. Clippy
echo "2/6 Rust linting"
if cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then
  pass "clippy"
else
  fail "clippy — run: cargo clippy --fix --allow-dirty"
fi

# 3. Cargo.lock freshness
echo "3/6 Cargo.lock freshness"
if cargo fetch --locked 2>/dev/null; then
  pass "Cargo.lock up to date"
else
  fail "Cargo.lock outdated — run: cargo fetch"
fi

# 4. UI formatting (skip if node_modules missing)
echo "4/6 UI formatting"
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
echo "5/6 UI linting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && npm run lint 2>/dev/null); then
    pass "UI lint"
  else
    fail "UI lint — run: cd apps/ui && npm run lint -- --fix"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

# 6. Commit author attribution check
echo "6/6 Commit author attribution"
AUTHOR_NAME=$(git config user.name 2>/dev/null || echo "")
AUTHOR_EMAIL=$(git config user.email 2>/dev/null || echo "")
BOT_PATTERN="(claude|cursor|copilot|github-actions|bot|ai-agent|openai|anthropic|gpt)"
if echo "$AUTHOR_NAME" | grep -iEq "$BOT_PATTERN"; then
  fail "git user.name looks like a bot: '$AUTHOR_NAME' — commits must use a real user"
elif echo "$AUTHOR_EMAIL" | grep -iEq "$BOT_PATTERN"; then
  fail "git user.email looks like a bot: '$AUTHOR_EMAIL' — commits must use a real user"
else
  pass "commit author: $AUTHOR_NAME <$AUTHOR_EMAIL>"
fi

echo ""
if [ $FAILED -ne 0 ]; then
  echo "❌ Pre-push checks failed. Fix issues above."
  echo "   Auto-fix: just fmt"
  exit 1
fi
echo "✅ All pre-push checks passed."
