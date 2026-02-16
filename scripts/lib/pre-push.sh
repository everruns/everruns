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
echo "1/5 Rust formatting"
if cargo fmt --check 2>/dev/null; then
  pass "cargo fmt"
else
  fail "cargo fmt — run: cargo fmt"
fi

# 2. Clippy
echo "2/5 Rust linting"
if cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then
  pass "clippy"
else
  fail "clippy — run: cargo clippy --fix --allow-dirty"
fi

# 3. Cargo.lock freshness
echo "3/5 Cargo.lock freshness"
if cargo fetch --locked 2>/dev/null; then
  pass "Cargo.lock up to date"
else
  fail "Cargo.lock outdated — run: cargo fetch"
fi

# 4. UI formatting (skip if node_modules missing)
echo "4/5 UI formatting"
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
echo "5/5 UI linting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && npm run lint 2>/dev/null); then
    pass "UI lint"
  else
    fail "UI lint — run: cd apps/ui && npm run lint -- --fix"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

echo ""
if [ $FAILED -ne 0 ]; then
  echo "❌ Pre-push checks failed. Fix issues above."
  echo "   Auto-fix: just fmt"
  exit 1
fi
echo "✅ All pre-push checks passed."
