#!/usr/bin/env bash
# Setup operations: init, upload-agents, seed

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
shift || true

case "$cmd" in
  init)
    echo "🔧 Installing all development dependencies..."
    echo ""

    require_command() {
      local cmd="$1"
      local hint="$2"

      if ! command -v "$cmd" &> /dev/null; then
        echo "❌ $cmd not installed. $hint"
        exit 1
      fi
    }

    echo "🧪 Preflight checks..."

    # Rust tools
    echo "📦 Rust tools:"
    if ! command -v sqlx &> /dev/null; then
      echo "  Installing sqlx-cli..."
      cargo install sqlx-cli --no-default-features --features postgres
    else
      echo "  ✅ sqlx-cli already installed"
    fi
    if ! command -v cargo-deny &> /dev/null; then
      echo "  Installing cargo-deny..."
      cargo install cargo-deny --locked
    else
      echo "  ✅ cargo-deny already installed"
    fi
    if ! command -v cargo-watch &> /dev/null; then
      echo "  Installing cargo-watch (for auto-reload)..."
      cargo install cargo-watch
    else
      echo "  ✅ cargo-watch already installed"
    fi

    # UI dependencies
    echo ""
    echo "🖥️  UI setup:"
    if ! command -v npm &> /dev/null; then
      echo "  ⚠️  npm not found! Please install Node.js/npm to use the UI."
      echo "     You can install it from: https://nodejs.org/"
      exit 1
    else
      echo "  ✅ npm found: $(npm --version)"
    fi
    echo "  📦 Installing UI dependencies..."
    cd "$PROJECT_ROOT/apps/ui"
    npm install
    echo "  🎭 Installing Playwright browsers..."
    npx playwright install chromium || echo "  ⚠️  Playwright browser install failed (may work in CI)"
    cd "$PROJECT_ROOT"

    # Docs dependencies
    echo ""
    echo "📚 Docs setup:"
    echo "  📦 Installing docs dependencies..."
    cd "$PROJECT_ROOT/apps/docs"
    npm install
    cd "$PROJECT_ROOT"

    echo ""
    echo "✅ All development dependencies ready!"
    ;;

  upload-agents)
    echo "📤 Uploading example agents..."
    API_URL="${API_URL:-http://localhost:9000}"
    EXAMPLES_DIR="$PROJECT_ROOT/examples/agents"

    # Check API is healthy
    if ! curl -s "$API_URL/health" > /dev/null 2>&1; then
      echo "❌ API not reachable at $API_URL"
      echo "   Start the control-plane first: just control-plane"
      exit 1
    fi

    # Check for jq
    if ! command -v jq &> /dev/null; then
      echo "❌ jq is required but not installed"
      echo "   Install with: apt-get install jq (or brew install jq)"
      exit 1
    fi

    # Build CLI if needed
    if [[ -f "$PROJECT_ROOT/target/release/everruns" ]]; then
      CLI_PATH="$PROJECT_ROOT/target/release/everruns"
    elif [[ -f "$PROJECT_ROOT/target/debug/everruns" ]]; then
      CLI_PATH="$PROJECT_ROOT/target/debug/everruns"
    else
      echo "📦 Building everruns CLI..."
      cargo build -p everruns-cli --release
      CLI_PATH="$PROJECT_ROOT/target/release/everruns"
    fi

    # Get existing agent names
    existing_agents=$(curl -s "$API_URL/v1/agents" | jq -r '.data[].name' 2>/dev/null || echo "")

    # Upload all example agents
    uploaded=0
    skipped=0
    for agent_file in "$EXAMPLES_DIR"/*.md; do
      if [[ ! -f "$agent_file" ]]; then
        continue
      fi

      display_name=$(grep -A1 "^---" "$agent_file" | grep "^name:" | sed 's/name:[[:space:]]*"\?\([^"]*\)"\?/\1/' | tr -d '"')

      if echo "$existing_agents" | grep -Fxq "$display_name"; then
        echo "   ⏭️  Skipping '$display_name' (already exists)"
        skipped=$((skipped + 1))
        continue
      fi

      echo "   🌱 Creating '$display_name'..."
      if $CLI_PATH --api-url "$API_URL" agents create --file "$agent_file" --quiet 2>/dev/null; then
        echo "      ✅ Created"
        uploaded=$((uploaded + 1))
      else
        echo "      ❌ Failed to create"
      fi
    done

    echo ""
    echo "📊 Upload complete: $uploaded created, $skipped skipped"
    ;;

  seed)
    exec "$PROJECT_ROOT/scripts/patch-provider-keys.sh" "$@"
    ;;

  *)
    echo "Usage: $0 {init|upload-agents|seed}"
    exit 1
    ;;
esac
