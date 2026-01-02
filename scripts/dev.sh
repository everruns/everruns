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
    sqlx migrate run --source crates/everruns-storage/migrations
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
    echo "🚀 Starting complete Everruns development environment..."
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
      echo "✅ Services stopped (Docker still running)"
      exit 0
    }

    # Set up signal handler for Ctrl+C
    trap cleanup SIGINT SIGTERM

    # Start Docker services
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
    sqlx migrate run --source crates/everruns-storage/migrations
    echo "   ✅ Migrations complete"

    # Wait for Temporal (needed before API/worker connect)
    wait_for_temporal

    # Start API in background with auto-reload (Temporal mode)
    echo "5️⃣  Starting API server with auto-reload (Temporal mode)..."
    export AGENT_RUNNER_MODE=temporal
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
    echo "6️⃣  Seeding development agents..."
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

      # Check if yq and jq are available for seeding
      if command -v yq &> /dev/null && command -v jq &> /dev/null; then
        "$SCRIPT_DIR/seed-agents.sh" 2>&1 | sed 's/^/   /'
      else
        echo "   ⚠️  Skipping seed: yq and jq required (install with: pip install yq && apt-get install jq)"
      fi
    ) &
    SEED_PID=$!

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
  start-all   Start everything with auto-reload (Docker, API, Worker, UI, seed)
  stop-all    Stop all services (API, UI, Docker)
  reset       Stop and remove all Docker volumes
  migrate     Run database migrations
  seed        Seed development agents from harness/seed-agents.yaml
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
  docs        Start the docs development server
  docs-build  Build the docs for production
  docs-install Install docs dependencies
  logs        View Docker service logs
  clean       Clean build artifacts and Docker volumes
  help        Show this help message

Examples:
  $0 init            # First-time setup (install all dependencies)
  $0 start-all       # Start everything with auto-reload
  $0 watch-api       # Just run API with auto-reload
  $0 docs            # Start docs dev server
  $0 stop-all        # Stop everything
EOF
    ;;
esac
