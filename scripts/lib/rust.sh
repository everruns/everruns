#!/usr/bin/env bash
# Rust operations: build, test, check, pre-pr, clean

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"

case "$cmd" in
  build)
    echo "🔨 Building Everrun..."
    cargo build
    echo "✅ Build complete!"
    ;;

  test)
    echo "🧪 Running tests..."
    cargo test
    echo "✅ Tests complete!"
    ;;

  check)
    echo "🔍 Running code quality checks..."
    echo "  - Formatting..."
    cargo fmt --check
    echo "  - Linting..."
    cargo clippy --all-targets -- -D warnings
    echo "  - Tests..."
    cargo test
    echo "✅ All checks passed!"
    ;;

  pre-pr)
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

    # 1. Rust formatting
    echo "1️⃣  Checking Rust formatting..."
    if cargo fmt --check; then
      echo "   ✅ Rust formatting OK"
    else
      echo "   ❌ Rust formatting failed. Run: cargo fmt"
      FAILED=1
    fi
    echo ""

    # 2. Rust linting
    echo "2️⃣  Running Clippy..."
    if cargo clippy --all-targets --all-features -- -D warnings; then
      echo "   ✅ Clippy passed"
    else
      echo "   ❌ Clippy failed"
      FAILED=1
    fi
    echo ""

    # 3. Rust tests
    echo "3️⃣  Running Rust tests..."
    if cargo test --all-features --lib --bins; then
      echo "   ✅ Rust tests passed"
    else
      echo "   ❌ Rust tests failed"
      FAILED=1
    fi
    echo ""

    # 4. UI lint
    echo "4️⃣  Running UI lint..."
    cd "$PROJECT_ROOT/apps/ui"
    if npm run lint; then
      echo "   ✅ UI lint passed"
    else
      echo "   ❌ UI lint failed"
      FAILED=1
    fi
    cd "$PROJECT_ROOT"
    echo ""

    # 5. UI build
    echo "5️⃣  Building UI..."
    cd "$PROJECT_ROOT/apps/ui"
    if npm run build; then
      echo "   ✅ UI build passed"
    else
      echo "   ❌ UI build failed"
      FAILED=1
    fi
    cd "$PROJECT_ROOT"
    echo ""

    # 6. OpenAPI spec freshness
    echo "6️⃣  Checking OpenAPI spec freshness..."
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

    # 7. Docs build
    echo "7️⃣  Building docs..."
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
    ;;

  clean)
    echo "🧹 Cleaning build artifacts..."
    cargo clean
    echo "✅ Cargo clean complete!"
    ;;

  *)
    echo "Usage: $0 {build|test|check|pre-pr|clean}"
    exit 1
    ;;
esac
