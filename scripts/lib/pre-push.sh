#!/usr/bin/env bash
# Pre-push checks: fast local validation to catch CI failures early (~30s).
# Runs formatting, linting, lockfile, migration, test/example enumeration, knowledge, and attribution checks.
# Usage: just pre-push (or: bash scripts/lib/pre-push.sh)

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

FAILED=0
fail() { echo "   ❌ $1"; FAILED=1; }
pass() { echo "   ✅ $1"; }

echo "🔒 Running pre-push checks..."
echo ""

# 1. Rust formatting
echo "1/12 Rust formatting"
if cargo fmt --check 2>/dev/null; then
  pass "cargo fmt"
else
  fail "cargo fmt — run: cargo fmt"
fi

# 2. Clippy
echo "2/12 Rust linting"
if cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then
  pass "clippy"
else
  fail "clippy — run: cargo clippy --fix --allow-dirty"
fi

# 3. Cargo.lock freshness
echo "3/12 Cargo.lock freshness"
if cargo fetch --locked 2>/dev/null; then
  pass "Cargo.lock up to date"
else
  fail "Cargo.lock outdated — run: cargo fetch"
fi

# 4. UI formatting (skip if node_modules missing)
echo "4/12 UI formatting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && pnpm run format:check 2>/dev/null); then
    pass "UI format"
  else
    fail "UI format — run: cd apps/ui && pnpm run format"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

# 5. UI linting (skip if node_modules missing)
echo "5/12 UI linting"
if [ -d "$PROJECT_ROOT/apps/ui/node_modules" ]; then
  if (cd "$PROJECT_ROOT/apps/ui" && pnpm run lint 2>/dev/null); then
    pass "UI lint"
  else
    fail "UI lint — run: cd apps/ui && pnpm run lint -- --fix"
  fi
else
  echo "   ⏭️  skipped (no node_modules)"
fi

# 6. Migration ordering check
echo "6/12 Migration ordering"
if MIGRATION_ORDERING_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-migration-ordering.sh" 2>&1
)"; then
  pass "$MIGRATION_ORDERING_OUTPUT"
else
  printf '%s\n' "$MIGRATION_ORDERING_OUTPUT" | sed 's/^/   /'
  fail "migration ordering check failed"
fi

# 7. Migration immutability check
echo "7/12 Migration immutability"
if MIGRATION_IMMUTABILITY_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-migration-immutability.sh" 2>&1
)"; then
  pass "existing migrations unchanged"
else
  printf '%s\n' "$MIGRATION_IMMUTABILITY_OUTPUT" | sed 's/^/   /'
  fail "migration immutability check failed"
fi

# 8. Server integration test enumeration
echo "8/12 Server test enumeration"
if SERVER_TEST_ENUM_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-server-test-enumeration.sh" 2>&1
)"; then
  pass "$SERVER_TEST_ENUM_OUTPUT"
else
  printf '%s\n' "$SERVER_TEST_ENUM_OUTPUT" | sed 's/^/   /'
  fail "server test enumeration check failed"
fi

# 9. Public everruns example inventory and compile check
echo "9/12 Public everruns examples"
if EVERRUNS_EXAMPLES_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-everruns-examples.sh" 2>&1
)"; then
  pass "public everruns examples use the facade and compile"
else
  printf '%s\n' "$EVERRUNS_EXAMPLES_OUTPUT" | sed 's/^/   /'
  fail "public everruns example check failed"
fi

# 10. Knowledge bundle conformance
echo "10/12 Knowledge bundle conformance"
if KNOWLEDGE_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/test-knowledge-okf.sh" 2>&1
)"; then
  pass "$KNOWLEDGE_OUTPUT"
else
  printf '%s\n' "$KNOWLEDGE_OUTPUT" | sed 's/^/   /'
  fail "knowledge bundle conformance check failed"
fi

# 11. Published crate documentation contract
echo "11/12 Published crate documentation"
if PUBLISHED_DOCS_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/test-published-crate-docs.sh" 2>&1
)"; then
  pass "$PUBLISHED_DOCS_OUTPUT"
else
  printf '%s\n' "$PUBLISHED_DOCS_OUTPUT" | sed 's/^/   /'
  fail "published crate documentation check failed"
fi

# 12. Commit author attribution check
echo "12/12 Commit author attribution"
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
