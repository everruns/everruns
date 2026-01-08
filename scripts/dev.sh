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

ensure_protoc() {
  if command -v protoc &> /dev/null; then
    return 0
  fi

  echo "ℹ️  protoc not found. Attempting installation..."
  if [[ "$OSTYPE" == "darwin"* ]] && command -v brew &> /dev/null; then
    brew install protobuf || true
  elif command -v apt-get &> /dev/null; then
    sudo apt-get update && sudo apt-get install -y protobuf-compiler || true
  fi

  if command -v protoc &> /dev/null; then
    echo "   ✅ protoc installed: $(protoc --version)"
    return 0
  fi

  echo "❌ protoc is required (Protocol Buffers compiler). Install manually, e.g.:"
  echo "   macOS:   brew install protobuf"
  echo "   Debian:  sudo apt-get install -y protobuf-compiler"
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
    echo "   - Temporal: localhost:7233"
    echo "   - Temporal UI: http://localhost:8080"
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

  seed)
    echo "🌱 Seeding development database..."
    "$SCRIPT_DIR/seed-agents.sh"
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
    export AGENT_RUNNER_MODE=${AGENT_RUNNER_MODE:-temporal}
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

  start-all)
    # Parse arguments: --temporal for Temporal mode (default is Durable)
    USE_TEMPORAL=false
    shift  # Remove 'start-all' from args
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --temporal)
          USE_TEMPORAL=true
          shift
          ;;
        *)
          echo "Unknown option: $1"
          echo "Usage: $0 start-all [--temporal]"
          exit 1
          ;;
      esac
    done

    if [ "$USE_TEMPORAL" = true ]; then
      echo "🚀 Starting Everruns development environment (Temporal mode)..."
    else
      echo "🚀 Starting Everruns development environment (Durable mode)..."
    fi
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

    check_port() {
      local host="$1"
      local port="$2"

      if command -v nc &> /dev/null; then
        nc -z "$host" "$port" &> /dev/null
        return $?
      fi

      if command -v python3 &> /dev/null; then
        python3 - <<PY > /dev/null 2>&1
import socket, sys
s = socket.socket()
s.settimeout(1)
try:
    s.connect(("$host", $port))
    sys.exit(0)
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
        return $?
      fi

      return 1
    }

    wait_for_temporal() {
      local max_attempts=60
      local attempt=0

      echo "4️⃣  Waiting for Temporal..."
      while [[ $attempt -lt $max_attempts ]]; do
        if check_port "localhost" "7233"; then
          echo "   ✅ Temporal is ready"
          return 0
        fi
        attempt=$((attempt + 1))
        if (( attempt % 5 == 0 )); then
          echo "   Waiting for Temporal to be ready..."
        fi
        sleep 1
      done

      echo "   ❌ Temporal did not become ready. Check docker logs and retry."
      cleanup
      exit 1
    }

    # Check for required tools early
    if [ "$USE_TEMPORAL" = true ]; then
      require_command docker "Install Docker Desktop/Colima and ensure the daemon is running."
      ensure_docker_daemon || exit 1
      if ! resolve_docker_compose; then
        echo "❌ Docker Compose plugin or docker-compose binary is required (try updating Docker Desktop or install docker-compose)."
        exit 1
      fi
      if ! command -v nc &> /dev/null && ! command -v python3 &> /dev/null; then
        echo "❌ Need either 'nc' or 'python3' available to check Temporal readiness."
        exit 1
      fi
    fi
    ensure_protoc || exit 1
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

    if [ "$USE_TEMPORAL" = true ]; then
      # Start Docker services (Temporal mode needs Docker for Temporal + Postgres)
      echo "1️⃣  Starting Docker services..."
      cd "$PROJECT_ROOT/harness"
      "${DOCKER_COMPOSE[@]}" up -d
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
      sqlx migrate run --source crates/control-plane/migrations
      echo "   ✅ Migrations complete"

      # Wait for Temporal (needed before API/worker connect)
      wait_for_temporal
    else
      # Durable mode: Just need Postgres (can be local or Docker)
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
    fi

    # Set runner mode
    if [ "$USE_TEMPORAL" = true ]; then
      export RUNNER_MODE=temporal
      MODE_NAME="Temporal"
    else
      export RUNNER_MODE=durable
      MODE_NAME="Durable"
    fi

    # Start API in background with auto-reload
    echo "3️⃣  Starting API server with auto-reload ($MODE_NAME mode)..."
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

    # Seed development agents (runs in background, waits for API)
    echo "4️⃣  Seeding development agents..."
    (
      # Wait for API to be healthy before seeding
      max_attempts=60
      attempt=0
      while [[ $attempt -lt $max_attempts ]]; do
        if curl -s http://localhost:9000/health > /dev/null 2>&1; then
          break
        fi
        attempt=$((attempt + 1))
        sleep 1
      done

      "$SCRIPT_DIR/seed-agents.sh" 2>&1 | sed 's/^/   /'
    ) &
    SEED_PID=$!

    # Start Worker in background with auto-reload
    echo "5️⃣  Starting $MODE_NAME worker with auto-reload..."
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
    echo "✅ All services started with auto-reload ($MODE_NAME mode)!"
    echo ""
    echo "   🌐 API:         http://localhost:9000 (auto-reload)"
    echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
    echo "   ⚙️ Worker:      running (auto-reload, $MODE_NAME)"
    echo "   🖥️ UI:          http://localhost:9100 (hot reload)"
    if [ "$USE_TEMPORAL" = true ]; then
      echo "   ⏱️ Temporal UI: http://localhost:8080"
    fi
    echo "   🔍 Jaeger UI:   http://localhost:16686"
    echo ""
    echo "👀 Edit code in crates/ and services will auto-restart"
    echo "💡 Press Ctrl+C to stop services"
    if [ "$USE_TEMPORAL" = false ]; then
      echo "💡 Use --temporal flag for Temporal mode: ./scripts/dev.sh start-all --temporal"
    fi
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

    # Preflight checks (align with start-all expectations)
    require_command() {
      local cmd="$1"
      local hint="$2"

      if ! command -v "$cmd" &> /dev/null; then
        echo "❌ $cmd not installed. $hint"
        exit 1
      fi
    }

    echo "🧪 Preflight checks..."
    require_command docker "Install Docker Desktop/Colima and ensure the daemon is running."
    ensure_docker_daemon || exit 1
    if ! resolve_docker_compose; then
      echo "❌ Docker Compose plugin or docker-compose binary is required (try updating Docker Desktop or install docker-compose)."
      exit 1
    fi
    if ! command -v nc &> /dev/null && ! command -v python3 &> /dev/null; then
      echo "ℹ️  Neither 'nc' nor 'python3' found. Attempting to install 'nc'..."
      if [[ "$OSTYPE" == "darwin"* ]] && command -v brew &> /dev/null; then
        brew install netcat || true
      elif command -v apt-get &> /dev/null; then
        sudo apt-get update && sudo apt-get install -y netcat-openbsd || true
      fi
      if ! command -v nc &> /dev/null && ! command -v python3 &> /dev/null; then
        echo "❌ Need either 'nc' or 'python3' available to check Temporal readiness."
        echo "   Please install netcat (nc) or Python 3 and rerun."
        exit 1
      fi
    fi
    ensure_protoc || exit 1

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

  help|*)
    cat <<EOF
Everrun Development Helper

Usage: $0 <command>

Commands:
  init        Install all development dependencies (Rust tools + UI + Docs)
  start       Start Docker services (Postgres, Temporal)
  stop        Stop Docker services
  start-all   Start everything with auto-reload (Durable mode by default)
              Use --temporal flag for Temporal mode
  stop-all    Stop all services (API, UI, Docker)
  reset       Stop and remove all Docker volumes
  migrate     Run database migrations
  seed        Seed development agents from harness/seed-agents.yaml
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
  docs        Start the docs development server
  docs-build  Build the docs for production
  docs-install Install docs dependencies
  logs        View Docker service logs
  clean       Clean build artifacts and Docker volumes
  help        Show this help message

Examples:
  $0 init                  # First-time setup (install all dependencies)
  $0 start-all             # Start everything in Durable mode (default)
  $0 start-all --temporal  # Start everything in Temporal mode
  $0 pre-pr                # Run all checks before creating a PR
  $0 watch-api             # Just run API with auto-reload
  $0 docs                  # Start docs dev server
  $0 stop-all              # Stop everything
EOF
    ;;
esac
