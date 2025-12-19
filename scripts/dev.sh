#!/usr/bin/env bash
set -euo pipefail

# Development helper script for Everrun

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

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
    cd harness
    docker compose up -d
    echo "✅ Services started!"
    echo "   - Postgres: localhost:5432"
    echo "   - Temporal: localhost:7233"
    echo "   - Temporal UI: http://localhost:8080"
    ;;

  stop)
    echo "🛑 Stopping Everrun development environment..."
    cd harness
    docker compose down
    echo "✅ Services stopped!"
    ;;

  reset)
    echo "🔄 Resetting Everrun development environment..."
    cd harness
    docker compose down -v
    echo "✅ Services reset!"
    ;;

  migrate)
    echo "🔧 Running database migrations..."
    export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
    sqlx migrate run --source crates/everruns-storage/migrations
    echo "✅ Migrations complete!"
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
    cargo run -p everruns-api
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
    export AGENT_RUNNER_MODE=${AGENT_RUNNER_MODE:-temporal}
    cargo watch -w crates -x 'run -p everruns-api'
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

  start-all)
    echo "🚀 Starting complete Everruns development environment..."
    echo ""

    # Check for cargo-watch
    if ! command -v cargo-watch &> /dev/null; then
      echo "❌ cargo-watch not installed. Run: ./scripts/dev.sh init"
      exit 1
    fi

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
      pkill -f "everruns-api" 2>/dev/null || true
      pkill -f "everruns-worker" 2>/dev/null || true
      pkill -f "next dev" 2>/dev/null || true
      echo "✅ Services stopped (Docker still running)"
      exit 0
    }

    # Set up signal handler for Ctrl+C
    trap cleanup SIGINT SIGTERM

    # Start Docker services
    echo "1️⃣  Starting Docker services..."
    cd "$PROJECT_ROOT/harness"
    docker compose up -d
    echo "   ✅ Docker services started"
    cd "$PROJECT_ROOT"

    # Wait for Postgres to be ready
    echo "2️⃣  Waiting for Postgres..."
    sleep 3
    until docker exec everruns-postgres pg_isready -U everruns -d everruns > /dev/null 2>&1; do
      echo "   Waiting for Postgres to be ready..."
      sleep 1
    done
    echo "   ✅ Postgres is ready"

    # Run migrations
    echo "3️⃣  Running database migrations..."
    export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
    sqlx migrate run --source crates/everruns-storage/migrations
    echo "   ✅ Migrations complete"

    # Start API in background with auto-reload (Temporal mode)
    echo "4️⃣  Starting API server with auto-reload (Temporal mode)..."
    export AGENT_RUNNER_MODE=temporal
    cargo watch -w crates -x 'run -p everruns-api' &
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Check if API is running
    if curl -s http://localhost:9000/health > /dev/null 2>&1; then
      echo "   ✅ API is running with auto-reload (PID: $API_PID)"
    else
      echo "   ⚠️  API compiling (will auto-reload on changes)..."
    fi

    # Start Worker in background with auto-reload (Temporal mode)
    echo "5️⃣  Starting Temporal worker with auto-reload..."
    cargo watch -w crates -x 'run -p everruns-worker' &
    WORKER_PID=$!
    CHILD_PIDS+=("$WORKER_PID")
    sleep 2
    echo "   ✅ Worker is starting with auto-reload (PID: $WORKER_PID)"

    # Start UI in background
    echo "6️⃣  Starting UI server..."
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
    echo "   ⚙️  Worker:      running (auto-reload)"
    echo "   🖥️  UI:          http://localhost:9100 (hot reload)"
    echo "   ⏱️  Temporal UI: http://localhost:8080"
    echo ""
    echo "👀 Edit code in crates/ and services will auto-restart"
    echo "💡 Press Ctrl+C to stop services (Docker will keep running)"
    echo ""

    # Wait for processes
    wait
    ;;

  stop-all)
    echo "🛑 Stopping all Everruns services..."

    # Kill any running cargo/node processes for this project
    pkill -f "everruns-api" 2>/dev/null || true
    pkill -f "everruns-worker" 2>/dev/null || true
    pkill -f "next dev" 2>/dev/null || true

    # Stop Docker services
    cd harness
    docker compose down

    echo "✅ All services stopped!"
    ;;

  logs)
    cd harness
    docker compose logs -f
    ;;

  init)
    echo "🔧 Installing all development dependencies..."
    echo ""

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
    cd "$PROJECT_ROOT"

    echo ""
    echo "✅ All development dependencies ready!"
    ;;

  clean)
    echo "🧹 Cleaning build artifacts and Docker volumes..."
    cargo clean
    cd harness
    docker compose down -v
    echo "✅ Clean complete!"
    ;;

  help|*)
    cat <<EOF
Everrun Development Helper

Usage: $0 <command>

Commands:
  init        Install all development dependencies (Rust tools + UI)
  start       Start Docker services (Postgres, Temporal)
  stop        Stop Docker services
  start-all   Start everything with auto-reload (Docker, API, Worker, UI)
  stop-all    Stop all services (API, UI, Docker)
  reset       Stop and remove all Docker volumes
  migrate     Run database migrations
  build       Build all crates
  test        Run tests
  check       Run format, lint, and test checks
  api         Start the API server
  worker      Start the worker
  watch-api   Start API with auto-reload on code changes
  watch-worker Start worker with auto-reload on code changes
  ui          Start the UI development server
  ui-build    Build the UI for production
  ui-install  Install UI dependencies
  logs        View Docker service logs
  clean       Clean build artifacts and Docker volumes
  help        Show this help message

Examples:
  $0 init            # First-time setup (install all dependencies)
  $0 start-all       # Start everything with auto-reload
  $0 watch-api       # Just run API with auto-reload
  $0 stop-all        # Stop everything
EOF
    ;;
esac
