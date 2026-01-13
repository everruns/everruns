#!/usr/bin/env bash
set -euo pipefail

# Development helper script for Everrun

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Resolve Docker Compose command (plugin or standalone)
DOCKER_COMPOSE=()
resolve_docker_compose() {
  if docker compose version &> /dev/null; then
    DOCKER_COMPOSE=(docker compose)
    return 0
  fi

  if command -v docker-compose &> /dev/null && docker-compose version &> /dev/null; then
    DOCKER_COMPOSE=(docker-compose)
    return 0
  fi

  return 1
}

ensure_docker_daemon() {
  local info_output
  if info_output=$(docker info 2>&1); then
    return 0
  fi

  echo "❌ Docker daemon not running or not accessible. Start Docker (Docker Desktop/Colima) and retry."
  echo "   Details: $info_output"
  return 1
}

# Load .env file if it exists
if [ -f .env ]; then
  set -a
  source .env
  set +a
fi

command="${1:-help}"

case "$command" in
  start)
    echo "🚀 Starting Everrun development environment..."
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi
    cd harness
    "${DOCKER_COMPOSE[@]}" up -d
    echo "✅ Services started!"
    echo "   - Postgres: localhost:5432"
    echo "   - Jaeger UI: http://localhost:16686"
    echo "   - OTLP gRPC: localhost:4317"
    ;;

  stop)
    echo "🛑 Stopping Everrun development environment..."
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi
    cd harness
    "${DOCKER_COMPOSE[@]}" down
    echo "✅ Services stopped!"
    ;;

  reset)
    echo "🔄 Resetting Everrun development environment..."
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi
    cd harness
    "${DOCKER_COMPOSE[@]}" down -v
    echo "✅ Services reset!"
    ;;

  migrate)
    echo "🔧 Running database migrations..."
    export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
    sqlx migrate run --source crates/control-plane/migrations
    echo "✅ Migrations complete!"
    ;;

  upload-agents)
    echo "📤 Uploading seed agents..."
    API_URL="${API_URL:-http://localhost:9000}"
    EXAMPLES_DIR="$PROJECT_ROOT/examples/agents"

    # Agents to upload (core demo agents)
    SEED_AGENTS=("dad-jokes-agent" "research-agent")

    # Check API is healthy
    if ! curl -s "$API_URL/health" > /dev/null 2>&1; then
      echo "❌ API not reachable at $API_URL"
      echo "   Start the API first: ./scripts/dev.sh api"
      exit 1
    fi

    # Check for jq (needed to parse agent names)
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

    # Get existing agent names to prevent duplicates
    existing_agents=$(curl -s "$API_URL/v1/agents" | jq -r '.data[].name' 2>/dev/null || echo "")

    # Upload specified agents (skip if already exists by name)
    uploaded=0
    skipped=0
    for agent_name in "${SEED_AGENTS[@]}"; do
      agent_file="$EXAMPLES_DIR/${agent_name}.md"
      if [[ ! -f "$agent_file" ]]; then
        echo "   ⚠️  Agent file not found: $agent_file"
        continue
      fi

      # Extract display name from markdown front matter
      display_name=$(grep -A1 "^---" "$agent_file" | grep "^name:" | sed 's/name:[[:space:]]*"\?\([^"]*\)"\?/\1/' | tr -d '"')

      # Check if agent with this name already exists
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
    # Patch API keys for providers from environment variables
    # Note: Providers and models are now loaded from config/providers.toml
    exec "$PROJECT_ROOT/scripts/patch-provider-keys.sh" "$@"
    ;;

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

  api)
    echo "🌐 Starting API server..."
    # Allow CORS from UI (localhost:9100) for SSE connections
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    cargo run -p everruns-control-plane
    ;;

  worker)
    echo "⚙️  Starting worker..."
    cargo run -p everruns-worker
    ;;

  watch-api)
    echo "👀 Starting API server with auto-reload..."
    if ! command -v cargo-watch &> /dev/null; then
      echo "❌ cargo-watch not installed. Run: cargo install cargo-watch"
      exit 1
    fi
    # Allow CORS from UI (localhost:9100) for SSE connections
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    cargo watch -w crates -x 'run -p everruns-control-plane'
    ;;

  watch-worker)
    echo "👀 Starting worker with auto-reload..."
    if ! command -v cargo-watch &> /dev/null; then
      echo "❌ cargo-watch not installed. Run: cargo install cargo-watch"
      exit 1
    fi
    cargo watch -w crates -x 'run -p everruns-worker'
    ;;

  ui)
    echo "🖥️  Starting UI development server..."
    cd apps/ui
    npm run dev
    ;;

  ui-build)
    echo "🔨 Building UI for production..."
    cd apps/ui
    npm run build
    echo "✅ UI build complete!"
    ;;

  ui-install)
    echo "📦 Installing UI dependencies..."
    cd apps/ui
    npm install
    echo "✅ UI dependencies installed!"
    ;;

  e2e)
    echo "🎭 Running UI e2e tests..."
    cd apps/ui

    # Set chromium path for restricted environments
    CHROMIUM_PATH="/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
    if [ -f "$CHROMIUM_PATH" ]; then
      export PLAYWRIGHT_CHROMIUM_PATH="$CHROMIUM_PATH"
    fi

    npm run e2e
    echo "✅ E2E tests complete!"
    ;;

  e2e-screenshots)
    echo "📸 Taking UI screenshots..."
    cd apps/ui

    # Set chromium path for restricted environments
    CHROMIUM_PATH="/root/.cache/ms-playwright/chromium-1194/chrome-linux/chrome"
    if [ -f "$CHROMIUM_PATH" ]; then
      export PLAYWRIGHT_CHROMIUM_PATH="$CHROMIUM_PATH"
    fi

    npm run e2e:screenshots
    echo "✅ Screenshots saved to apps/ui/e2e/screenshots/"
    ;;

  docs)
    echo "📚 Starting docs development server..."
    cd apps/docs
    npm run dev
    ;;

  docs-build)
    echo "🔨 Building docs for production..."
    cd apps/docs
    npm run check && npm run build
    echo "✅ Docs build complete!"
    ;;

  docs-install)
    echo "📦 Installing docs dependencies..."
    cd apps/docs
    npm install
    echo "✅ Docs dependencies installed!"
    ;;

  start-dev)
    echo "🚀 Starting Everruns in DEV MODE (in-memory storage, no database required)..."
    echo ""

    # Required tool checks
    require_command() {
      local cmd="$1"
      local hint="$2"

      if ! command -v "$cmd" &> /dev/null; then
        echo "❌ $cmd not installed. $hint"
        exit 1
      fi
    }

    require_command cargo-watch "Run: ./scripts/dev.sh init"
    require_command npm "Install Node.js/npm to start the UI (see README.md)."

    # Track child PIDs for cleanup
    CHILD_PIDS=()

    # Cleanup function to kill child processes on exit
    cleanup() {
      echo ""
      echo "🛑 Stopping services..."
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill "$pid" 2>/dev/null || true
        fi
      done
      pkill -f "cargo-watch" 2>/dev/null || true
      pkill -f "everruns-control-plane" 2>/dev/null || true
      pkill -f "next dev" 2>/dev/null || true
      echo "✅ Services stopped"
      exit 0
    }

    trap cleanup SIGINT SIGTERM

    # Enable dev mode
    export DEV_MODE=true
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}

    # Start API in background with auto-reload (dev mode)
    echo "1️⃣  Starting API server (DEV MODE) with auto-reload..."
    cargo watch -w crates -x 'run -p everruns-control-plane' &
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Check if API is running
    if curl -s http://localhost:9000/health > /dev/null 2>&1; then
      echo "   ✅ API is running (PID: $API_PID)"
    else
      echo "   ⚠️  API compiling (will auto-reload on changes)..."
    fi

    # Wait for API to be ready
    echo "2️⃣  Waiting for API to be ready..."
    for i in {1..30}; do
      if curl -s http://localhost:9000/health > /dev/null 2>&1; then
        echo "   ✅ API is ready"
        break
      fi
      sleep 2
    done

    # Note: Worker is NOT started in dev mode (execution happens in-process)
    echo "3️⃣  Worker: Not needed in DEV MODE (execution happens in-process)"

    # Start UI in background
    echo "4️⃣  Starting UI server..."
    cd apps/ui
    npm run dev &
    UI_PID=$!
    CHILD_PIDS+=("$UI_PID")
    cd "$PROJECT_ROOT"
    sleep 5
    echo "   ✅ UI is starting (PID: $UI_PID)"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ DEV MODE started (in-memory storage)!"
    echo ""
    echo "   🌐 API:         http://localhost:9000 (auto-reload)"
    echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
    echo "   🖥️  UI:          http://localhost:9100 (hot reload)"
    echo ""
    echo "⚠️  DEV MODE limitations:"
    echo "   - Data is stored in memory (lost on restart)"
    echo "   - No PostgreSQL or Docker required"
    echo "   - No separate worker process"
    echo ""
    echo "👀 Edit code in crates/ and services will auto-restart"
    echo "💡 Press Ctrl+C to stop services"
    echo ""

    # Wait for processes
    wait
    ;;

  start-all)
    echo "🚀 Starting Everruns development environment..."
    echo ""

    # Required tool checks and helpers
    require_command() {
      local cmd="$1"
      local hint="$2"

      if ! command -v "$cmd" &> /dev/null; then
        echo "❌ $cmd not installed. $hint"
        exit 1
      fi
    }

    require_command cargo-watch "Run: ./scripts/dev.sh init"
    require_command sqlx "Run: ./scripts/dev.sh init"
    require_command npm "Install Node.js/npm to start the UI (see README.md)."

    # Track child PIDs for cleanup
    CHILD_PIDS=()

    # Cleanup function to kill child processes on exit
    cleanup() {
      echo ""
      echo "🛑 Stopping services..."
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill "$pid" 2>/dev/null || true
        fi
      done
      # Also kill by name in case PIDs were replaced
      pkill -f "cargo-watch" 2>/dev/null || true
      pkill -f "everruns-control-plane" 2>/dev/null || true
      pkill -f "everruns-worker" 2>/dev/null || true
      pkill -f "next dev" 2>/dev/null || true
      echo "✅ Services stopped (Docker still running if started)"
      exit 0
    }

    # Set up signal handler for Ctrl+C
    trap cleanup SIGINT SIGTERM

    # Check PostgreSQL (can be local or Docker)
    echo "1️⃣  Checking PostgreSQL..."

    # Try local postgres first, then Docker
    if pg_isready -h localhost -p 5432 > /dev/null 2>&1; then
      echo "   ✅ Local PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://postgres:postgres@localhost/everruns}
    elif command -v docker &> /dev/null && docker ps 2>/dev/null | grep -q postgres; then
      echo "   ✅ Docker PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
    else
      echo "   ⚠️  PostgreSQL not found. Starting via Docker..."
      if resolve_docker_compose; then
        ensure_docker_daemon || exit 1
        cd "$PROJECT_ROOT/harness"
        "${DOCKER_COMPOSE[@]}" up -d postgres
        cd "$PROJECT_ROOT"
        sleep 3
        until docker exec everruns-postgres pg_isready -U everruns -d everruns > /dev/null 2>&1; do
          echo "   Waiting for Postgres to be ready..."
          sleep 1
        done
        export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
        echo "   ✅ Docker PostgreSQL started"
      else
        echo "   ❌ No PostgreSQL available. Start PostgreSQL or install Docker."
        exit 1
      fi
    fi

    # Run migrations
    echo "2️⃣  Running database migrations..."
    sqlx migrate run --source crates/control-plane/migrations
    echo "   ✅ Migrations complete"

    # Set default LLM API keys from environment (for development convenience)
    # These are used as fallback when providers don't have keys in the database
    echo "3️⃣  Configuring LLM API keys from environment..."
    if [ -n "${OPENAI_API_KEY:-}" ]; then
      export DEFAULT_OPENAI_API_KEY="$OPENAI_API_KEY"
      echo "   ✅ OpenAI API key configured"
    else
      echo "   ⚠️  OPENAI_API_KEY not set (OpenAI models may not work)"
    fi
    if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
      export DEFAULT_ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"
      echo "   ✅ Anthropic API key configured"
    else
      echo "   ⚠️  ANTHROPIC_API_KEY not set (Anthropic models may not work)"
    fi

    # Start API in background with auto-reload
    echo "4️⃣  Starting API server with auto-reload..."
    # Allow CORS from UI (localhost:9100) for SSE connections
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    cargo watch -w crates -x 'run -p everruns-control-plane' &
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Check if API is running
    if curl -s http://localhost:9000/health > /dev/null 2>&1; then
      echo "   ✅ API is running with auto-reload (PID: $API_PID)"
    else
      echo "   ⚠️  API compiling (will auto-reload on changes)..."
    fi

    # Wait for API to be ready, then upload seed agents
    echo "5️⃣  Waiting for API to be ready..."
    for i in {1..30}; do
      if curl -s http://localhost:9000/health > /dev/null 2>&1; then
        echo "   ✅ API is ready"
        break
      fi
      sleep 2
    done

    # Upload seed agents from markdown files
    echo "6️⃣  Uploading seed agents..."
    if "$0" upload-agents 2>/dev/null; then
      echo "   ✅ Seed agents uploaded"
    else
      echo "   ⚠️  Agent upload failed"
    fi

    # Start Worker in background with auto-reload (Temporal mode)
    echo "7️⃣  Starting Temporal worker with auto-reload..."
    cargo watch -w crates -x 'run -p everruns-worker' &
    WORKER_PID=$!
    CHILD_PIDS+=("$WORKER_PID")
    sleep 2
    echo "   ✅ Worker is starting with auto-reload (PID: $WORKER_PID)"

    # Start UI in background
    echo "8️⃣  Starting UI server..."
    cd apps/ui
    npm run dev &
    UI_PID=$!
    CHILD_PIDS+=("$UI_PID")
    cd "$PROJECT_ROOT"
    sleep 5
    echo "   ✅ UI is starting (PID: $UI_PID)"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ All services started with auto-reload!"
    echo ""
    echo "   🌐 API:         http://localhost:9000 (auto-reload)"
    echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
    echo "   ⚙️ Worker:      running (auto-reload)"
    echo "   🖥️ UI:          http://localhost:9100 (hot reload)"
    echo "   🔍 Jaeger UI:   http://localhost:16686"
    echo ""
    echo "👀 Edit code in crates/ and services will auto-restart"
    echo "💡 Press Ctrl+C to stop services"
    echo ""

    # Wait for processes
    wait
    ;;

  stop-all)
    echo "🛑 Stopping all Everruns services..."

    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi

    # Kill any running cargo/node processes for this project
    pkill -f "everruns-control-plane" 2>/dev/null || true
    pkill -f "everruns-worker" 2>/dev/null || true
    pkill -f "next dev" 2>/dev/null || true

    # Stop Docker services
    cd harness
    "${DOCKER_COMPOSE[@]}" down

    echo "✅ All services stopped!"
    ;;

  logs)
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi
    cd harness
    "${DOCKER_COMPOSE[@]}" logs -f
    ;;

  init)
    echo "🔧 Installing all development dependencies..."
    echo ""

    # Preflight checks
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
    cd apps/ui
    npm install
    echo "  🎭 Installing Playwright browsers..."
    npx playwright install chromium || echo "  ⚠️  Playwright browser install failed (may work in CI)"
    cd "$PROJECT_ROOT"

    # Docs dependencies
    echo ""
    echo "📚 Docs setup:"
    echo "  📦 Installing docs dependencies..."
    cd apps/docs
    npm install
    cd "$PROJECT_ROOT"

    echo ""
    echo "✅ All development dependencies ready!"
    ;;

  pre-pr)
    echo "🔍 Running pre-PR checks..."
    echo ""
    FAILED=0

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
    if cargo test --all-features; then
      echo "   ✅ Rust tests passed"
    else
      echo "   ❌ Rust tests failed"
      FAILED=1
    fi
    echo ""

    # 4. UI lint
    echo "4️⃣  Running UI lint..."
    cd apps/ui
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
    cd apps/ui
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
      if diff -q docs/api/openapi.json "$TEMP_SPEC" > /dev/null 2>&1; then
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
    cd apps/docs
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
    echo "🧹 Cleaning build artifacts and Docker volumes..."
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
      exit 1
    fi
    cargo clean
    cd harness
    "${DOCKER_COMPOSE[@]}" down -v
    echo "✅ Clean complete!"
    ;;

  durable-bench)
    echo "📊 Running durable execution benchmarks..."
    echo ""
    echo "Running concurrent_workers..."
    cargo bench -p everruns-durable --bench concurrent_workers
    echo ""
    echo "Running workflow_throughput..."
    cargo bench -p everruns-durable --bench workflow_throughput
    echo ""
    echo "✅ Benchmarks complete!"
    echo "   Reports: target/benchmark-reports/"
    ;;

  durable-bench-save)
    echo "📊 Running durable execution benchmarks (with checkpointing)..."
    MONIKER_ARG=""
    if [ -n "${2:-}" ]; then
      MONIKER_ARG="--moniker $2"
      echo "   Using moniker: $2"
    fi
    echo ""
    echo "Running concurrent_workers..."
    cargo bench -p everruns-durable --bench concurrent_workers -- --save $MONIKER_ARG
    echo ""
    echo "Running workflow_throughput..."
    cargo bench -p everruns-durable --bench workflow_throughput -- --save $MONIKER_ARG
    echo ""
    echo "✅ Benchmarks complete with checkpoints saved!"
    echo "   Reports:     target/benchmark-reports/"
    echo "   Checkpoints: crates/durable/benches/checkpoints/"
    ;;

  help|*)
    cat <<EOF
Everrun Development Helper

Usage: $0 <command>

Commands:
  init        Install all development dependencies (Rust tools + UI + Docs)
  start       Start Docker services (Postgres, Jaeger)
  stop        Stop Docker services
  start-dev   Start in DEV MODE (in-memory storage, no Docker/PostgreSQL required)
  start-all   Start everything with auto-reload (Docker, API, Worker, UI)
  stop-all    Stop all services (API, UI, Docker)
  reset       Stop and remove all Docker volumes
  migrate     Run database migrations
  upload-agents Upload seed agents (dad-jokes, research) using CLI
  build       Build all crates
  test        Run tests
  check       Run format, lint, and test checks
  pre-pr      Run all pre-PR checks (fmt, clippy, tests, UI, OpenAPI, docs)
  api         Start the API server
  worker      Start the worker
  watch-api   Start API with auto-reload on code changes
  watch-worker Start worker with auto-reload on code changes
  ui          Start the UI development server
  ui-build    Build the UI for production
  ui-install  Install UI dependencies
  e2e         Run UI e2e tests (Playwright)
  e2e-screenshots  Take UI screenshots for visual verification
  docs        Start the docs development server
  docs-build  Build the docs for production
  docs-install Install docs dependencies
  logs        View Docker service logs
  clean       Clean build artifacts and Docker volumes
  durable-bench      Run durable execution benchmarks
  durable-bench-save Run benchmarks with checkpoint saving (optional: moniker)
  help        Show this help message

Examples:
  $0 init                  # First-time setup (install all dependencies)
  $0 start-dev             # Start DEV MODE (no Docker/PostgreSQL needed)
  $0 start-all             # Start everything with auto-reload (requires PostgreSQL)
  $0 pre-pr                # Run all checks before creating a PR
  $0 watch-api             # Just run API with auto-reload
  $0 docs                  # Start docs dev server
  $0 durable-bench         # Run benchmarks
  $0 durable-bench-save    # Run benchmarks with checkpointing
  $0 durable-bench-save ci-4cpu-8gb  # With custom moniker
  $0 stop-all              # Stop everything
EOF
    ;;
esac
