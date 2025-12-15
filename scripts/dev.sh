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
    docker compose up -d
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
    echo "🌐 Starting API server (Temporal mode)..."
    AGENT_RUNNER_MODE=temporal cargo run -p everruns-api --features temporal
    ;;

  worker)
    echo "⚙️  Starting worker (Temporal mode)..."
    AGENT_RUNNER_MODE=temporal cargo run -p everruns-worker --features temporal
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

    # Start API in background (with temporal feature and mode enabled)
    echo "4️⃣  Starting API server (Temporal mode)..."
    AGENT_RUNNER_MODE=temporal cargo run -p everruns-api --features temporal &
    API_PID=$!
    sleep 3

    # Check if API is running
    if curl -s http://localhost:9000/health > /dev/null 2>&1; then
      echo "   ✅ API is running (PID: $API_PID)"
    else
      echo "   ⚠️  API may still be starting..."
    fi

    # Start Worker in background (with temporal feature and mode enabled)
    echo "5️⃣  Starting Temporal worker..."
    AGENT_RUNNER_MODE=temporal cargo run -p everruns-worker --features temporal &
    WORKER_PID=$!
    sleep 2
    echo "   ✅ Worker is starting (PID: $WORKER_PID)"

    # Start UI in background
    echo "6️⃣  Starting UI server..."
    cd apps/ui
    npm run dev &
    UI_PID=$!
    cd "$PROJECT_ROOT"
    sleep 5
    echo "   ✅ UI is starting (PID: $UI_PID)"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ All services started!"
    echo ""
    echo "   🌐 API:         http://localhost:9000"
    echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
    echo "   ⚙️  Worker:      running (Temporal workflows)"
    echo "   🖥️  UI:          http://localhost:3000"
    echo "   ⏱️  Temporal UI: http://localhost:8080"
    echo ""
    echo "💡 To stop all services: ./scripts/dev.sh stop-all"
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

  smoke-test)
    echo "🧪 Running smoke tests..."
    "$SCRIPT_DIR/smoke-test.sh" "$@"
    ;;

  logs)
    cd harness
    docker compose logs -f
    ;;

  init)
    echo "🔧 Installing development dependencies..."
    if ! command -v sqlx &> /dev/null; then
      echo "  Installing sqlx-cli..."
      cargo install sqlx-cli --no-default-features --features postgres
    else
      echo "  sqlx-cli already installed"
    fi
    if ! command -v cargo-deny &> /dev/null; then
      echo "  Installing cargo-deny..."
      cargo install cargo-deny --locked
    else
      echo "  cargo-deny already installed"
    fi
    echo "✅ Development dependencies ready!"
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
  init        Install development dependencies (sqlx-cli, cargo-deny)
  start       Start Docker services (Postgres, Temporal)
  stop        Stop Docker services
  start-all   Start everything (Docker, migrations, API, UI)
  stop-all    Stop all services (API, UI, Docker)
  reset       Stop and remove all Docker volumes
  migrate     Run database migrations
  build       Build all crates
  test        Run tests
  check       Run format, lint, and test checks
  api         Start the API server
  worker      Start the worker
  ui          Start the UI development server
  ui-build    Build the UI for production
  ui-install  Install UI dependencies
  smoke-test  Run smoke tests (use --with-ui for UI tests)
  logs        View Docker service logs
  clean       Clean build artifacts and Docker volumes
  help        Show this help message

Examples:
  $0 init            # First-time setup (install tools)
  $0 start-all       # Start everything and run
  $0 smoke-test      # Run API smoke tests
  $0 smoke-test --with-ui  # Run API + UI smoke tests
  $0 stop-all        # Stop everything
EOF
    ;;
esac
