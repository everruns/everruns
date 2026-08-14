#!/usr/bin/env bash
# Pre-push checks: fast local validation to catch CI failures early (~30s).
# Runs formatting, linting, lockfile, migration, test/example enumeration, knowledge,
# architecture isolation guards, and attribution checks.
# Usage: just pre-push (or: bash scripts/lib/pre-push.sh)

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

FAILED=0
fail() { echo "   ❌ $1"; FAILED=1; }
pass() { echo "   ✅ $1"; }

echo "🔒 Running pre-push checks..."
echo ""

# 1. Rust formatting
echo "1/23 Rust formatting"
if cargo fmt --check 2>/dev/null; then
  pass "cargo fmt"
else
  fail "cargo fmt — run: cargo fmt"
fi

# 2. Clippy
echo "2/23 Rust linting"
if cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then
  pass "clippy"
else
  fail "clippy — run: cargo clippy --fix --allow-dirty"
fi

# 3. Cargo.lock freshness
echo "3/23 Cargo.lock freshness"
if cargo fetch --locked 2>/dev/null; then
  pass "Cargo.lock up to date"
else
  fail "Cargo.lock outdated — run: cargo fetch"
fi

# 4. UI formatting (skip if node_modules missing)
echo "4/23 UI formatting"
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
echo "5/23 UI linting"
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
echo "6/23 Migration ordering"
if MIGRATION_ORDERING_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-migration-ordering.sh" 2>&1
)"; then
  pass "$MIGRATION_ORDERING_OUTPUT"
else
  printf '%s\n' "$MIGRATION_ORDERING_OUTPUT" | sed 's/^/   /'
  fail "migration ordering check failed"
fi

# 7. Migration immutability check
echo "7/23 Migration immutability"
if MIGRATION_IMMUTABILITY_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-migration-immutability.sh" 2>&1
)"; then
  pass "existing migrations unchanged"
else
  printf '%s\n' "$MIGRATION_IMMUTABILITY_OUTPUT" | sed 's/^/   /'
  fail "migration immutability check failed"
fi

# 8. Server integration test enumeration
echo "8/23 Server test enumeration"
if SERVER_TEST_ENUM_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-server-test-enumeration.sh" 2>&1
)"; then
  pass "$SERVER_TEST_ENUM_OUTPUT"
else
  printf '%s\n' "$SERVER_TEST_ENUM_OUTPUT" | sed 's/^/   /'
  fail "server test enumeration check failed"
fi

# 9. Public everruns example inventory and compile check
echo "9/23 Public everruns examples"
if EVERRUNS_EXAMPLES_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-everruns-examples.sh" 2>&1
)"; then
  pass "public everruns examples use the facade and compile"
else
  printf '%s\n' "$EVERRUNS_EXAMPLES_OUTPUT" | sed 's/^/   /'
  fail "public everruns example check failed"
fi

# 10. Knowledge bundle conformance
echo "10/23 Knowledge bundle conformance"
if KNOWLEDGE_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/test-knowledge-okf.sh" 2>&1
)"; then
  pass "$KNOWLEDGE_OUTPUT"
else
  printf '%s\n' "$KNOWLEDGE_OUTPUT" | sed 's/^/   /'
  fail "knowledge bundle conformance check failed"
fi

# 11. Published crate documentation contract
echo "11/23 Published crate documentation"
if PUBLISHED_DOCS_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/test-published-crate-docs.sh" 2>&1
)"; then
  pass "$PUBLISHED_DOCS_OUTPUT"
else
  printf '%s\n' "$PUBLISHED_DOCS_OUTPUT" | sed 's/^/   /'
  fail "published crate documentation check failed"
fi

# 12. Capability contract architecture guard (EVE-873)
echo "12/23 Capability contract guard"
if CAPABILITY_CONTRACT_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-capability-contract.sh" 2>&1
)"; then
  pass "$CAPABILITY_CONTRACT_OUTPUT"
else
  printf '%s\n' "$CAPABILITY_CONTRACT_OUTPUT" | sed 's/^/   /'
  fail "capability contract guard failed"
fi

# 13. Test-support isolation guard (EVE-875)
echo "13/23 Test-support isolation guard"
if TEST_SUPPORT_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-test-support-isolation.sh" 2>&1
)"; then
  pass "$TEST_SUPPORT_ISOLATION_OUTPUT"
else
  printf '%s\n' "$TEST_SUPPORT_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "test-support isolation guard failed"
fi

# 14. Observability isolation guard (EVE-876)
echo "14/23 Observability isolation guard"
if OBSERVABILITY_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-observability-isolation.sh" 2>&1
)"; then
  pass "$OBSERVABILITY_ISOLATION_OUTPUT"
else
  printf '%s\n' "$OBSERVABILITY_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "observability isolation guard failed"
fi

# 15. Agent-record isolation guard (EVE-877, EVE-881, EVE-882, EVE-878,
#     EVE-879: Agent, Harness, Session, eval/observer/feature-management
#     records, and connector/OAuth/email infrastructure)
echo "15/23 Agent-record isolation guard"
if AGENT_RECORD_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-agent-record-isolation.sh" 2>&1
)"; then
  pass "$AGENT_RECORD_ISOLATION_OUTPUT"
else
  printf '%s\n' "$AGENT_RECORD_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "agent-record isolation guard failed"
fi

# 16. Provider isolation guard (EVE-874)
echo "16/23 Provider isolation guard"
if PROVIDER_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-provider-isolation.sh" 2>&1
)"; then
  pass "$PROVIDER_ISOLATION_OUTPUT"
else
  printf '%s\n' "$PROVIDER_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "provider isolation guard failed"
fi

# 17. Core kernel dependency guard (EVE-903)
echo "17/23 Core kernel dependency guard"
if CORE_KERNEL_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-core-kernel-dependencies.sh" 2>&1
)"; then
  pass "$CORE_KERNEL_OUTPUT"
else
  printf '%s\n' "$CORE_KERNEL_OUTPUT" | sed 's/^/   /'
  fail "core kernel dependency guard failed"
fi

# 18. Core public API/semver and documentation guard (EVE-906)
echo "18/23 Core public API guard"
if CORE_PUBLIC_API_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-core-public-api.sh" 2>&1
)"; then
  pass "$CORE_PUBLIC_API_OUTPUT"
else
  printf '%s\n' "$CORE_PUBLIC_API_OUTPUT" | sed 's/^/   /'
  fail "core public API guard failed"
fi

# 19. Effectful environment capability isolation guard (EVE-883)
echo "19/23 Environment capability isolation guard"
if ENVIRONMENT_CAPABILITY_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-environment-capability-isolation.sh" 2>&1
)"; then
  pass "$ENVIRONMENT_CAPABILITY_ISOLATION_OUTPUT"
else
  printf '%s\n' "$ENVIRONMENT_CAPABILITY_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "environment capability isolation guard failed"
fi

# 20. Hosted-capability isolation guard (EVE-885)
echo "20/23 Hosted-capability isolation guard"
if HOSTED_CAPABILITY_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-hosted-capability-isolation.sh" 2>&1
)"; then
  pass "$HOSTED_CAPABILITY_ISOLATION_OUTPUT"
else
  printf '%s\n' "$HOSTED_CAPABILITY_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "hosted-capability isolation guard failed"
fi

# 21. Portable policy built-ins isolation guard (EVE-884)
echo "21/23 Portable built-ins isolation guard"
if PORTABLE_BUILTINS_ISOLATION_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-portable-builtins-isolation.sh" 2>&1
)"; then
  pass "$PORTABLE_BUILTINS_ISOLATION_OUTPUT"
else
  printf '%s\n' "$PORTABLE_BUILTINS_ISOLATION_OUTPUT" | sed 's/^/   /'
  fail "portable built-ins isolation guard failed"
fi

# 22. Scale Engine architecture boundary
echo "22/23 Scale Engine boundary guard"
if SCALE_ENGINE_OUTPUT="$(
  bash "$PROJECT_ROOT/scripts/lib/check-scale-engine-boundary.sh" 2>&1
)"; then
  pass "$SCALE_ENGINE_OUTPUT"
else
  printf '%s\n' "$SCALE_ENGINE_OUTPUT" | sed 's/^/   /'
  fail "Scale Engine boundary guard failed"
fi

# 23. Commit author attribution check
echo "23/23 Commit author attribution"
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
