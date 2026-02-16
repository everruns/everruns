#!/usr/bin/env bash
# Pre-PR checks: validates code is ready for pull request

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

echo "🔍 Running pre-PR checks..."
echo ""
FAILED=0

# 0. Check branch is rebased on latest main
echo "0️⃣  Checking branch is rebased on main..."
if git fetch origin main --quiet 2>/dev/null; then
  MAIN_HEAD=$(git rev-parse origin/main 2>/dev/null)
  MERGE_BASE=$(git merge-base HEAD origin/main 2>/dev/null)
  if [ "$MAIN_HEAD" = "$MERGE_BASE" ]; then
    echo "   ✅ Branch is rebased on latest main"
  else
    echo "   ⚠️  Branch is not rebased on latest main"
    echo "      Run: git fetch origin main && git rebase origin/main"
    echo "      (This is a warning, not blocking)"
  fi
else
  echo "   ⚠️  Could not fetch origin/main (offline or no remote)"
fi
echo ""

# 1. Cargo.lock freshness
echo "1️⃣  Checking Cargo.lock freshness..."
if cargo fetch --locked 2>/dev/null; then
  echo "   ✅ Cargo.lock is up to date"
else
  echo "   ❌ Cargo.lock is outdated. Run: cargo fetch"
  FAILED=1
fi
echo ""

# 2. Rust formatting
echo "2️⃣  Checking Rust formatting..."
if cargo fmt --check; then
  echo "   ✅ Rust formatting OK"
else
  echo "   ❌ Rust formatting failed. Run: cargo fmt"
  FAILED=1
fi
echo ""

# 3. Rust linting
echo "3️⃣  Running Clippy..."
if cargo clippy --all-targets --all-features -- -D warnings; then
  echo "   ✅ Clippy passed"
else
  echo "   ❌ Clippy failed"
  FAILED=1
fi
echo ""

# 4. Rust tests
echo "4️⃣  Running Rust tests..."
if cargo test --all-features --lib --bins; then
  echo "   ✅ Rust tests passed"
else
  echo "   ❌ Rust tests failed"
  FAILED=1
fi
echo ""

# 5. UI formatting
echo "5️⃣  Checking UI formatting..."
cd "$PROJECT_ROOT/apps/ui"
if npm run format:check; then
  echo "   ✅ UI formatting OK"
else
  echo "   ❌ UI formatting failed. Run: cd apps/ui && npm run format"
  FAILED=1
fi
cd "$PROJECT_ROOT"
echo ""

# 6. UI lint
echo "6️⃣  Running UI lint..."
cd "$PROJECT_ROOT/apps/ui"
if npm run lint; then
  echo "   ✅ UI lint passed"
else
  echo "   ❌ UI lint failed"
  FAILED=1
fi
cd "$PROJECT_ROOT"
echo ""

# 7. UI build
echo "7️⃣  Building UI..."
cd "$PROJECT_ROOT/apps/ui"
if npm run build; then
  echo "   ✅ UI build passed"
else
  echo "   ❌ UI build failed"
  FAILED=1
fi
cd "$PROJECT_ROOT"
echo ""

# 8. OpenAPI spec freshness
echo "8️⃣  Checking OpenAPI spec freshness..."
TEMP_SPEC=$(mktemp)
if cargo run --bin export-openapi --release 2>/dev/null > "$TEMP_SPEC"; then
  if diff -q "$PROJECT_ROOT/docs/api/openapi.json" "$TEMP_SPEC" > /dev/null 2>&1; then
    echo "   ✅ OpenAPI spec is up to date"
  else
    echo "   ❌ OpenAPI spec is out of date!"
    echo "      Run: ./scripts/export-openapi.sh"
    FAILED=1
  fi
else
  echo "   ❌ Failed to generate OpenAPI spec"
  FAILED=1
fi
rm -f "$TEMP_SPEC"
echo ""

# 9. Docs build
echo "9️⃣  Building docs..."
cd "$PROJECT_ROOT/apps/docs"
if npm run check && npm run build; then
  echo "   ✅ Docs build passed"
else
  echo "   ❌ Docs build failed"
  FAILED=1
fi
cd "$PROJECT_ROOT"
echo ""

# Summary
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [ $FAILED -eq 0 ]; then
  echo "✅ All pre-PR checks passed!"
  echo "   Ready to create a pull request."
else
  echo "❌ Some checks failed. Please fix the issues above."
  exit 1
fi
