#!/usr/bin/env bash
# Service operations: control-plane, worker, watch-*, start-dev, start-all, stop-all

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
shift || true

# Parse flags
NO_WATCH=false
NO_DOCKER=false
NO_UI=false
for arg in "$@"; do
  case "$arg" in
    --no-watch) NO_WATCH=true ;;
    --no-docker) NO_DOCKER=true ;;
    --no-ui) NO_UI=true ;;
  esac
done

# require_command is defined in common.sh (sourced above)

case "$cmd" in
  control-plane)
    echo "🌐 Starting control-plane server..."
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    cargo run -p everruns-control-plane
    ;;

  worker)
    echo "⚙️  Starting worker..."
    cargo run -p everruns-worker
    ;;

  watch-control-plane)
    echo "👀 Starting control-plane server with auto-reload..."
    require_command cargo-watch "Run: just init"
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    cargo watch -w crates -x 'run -p everruns-control-plane'
    ;;

  watch-worker)
    echo "👀 Starting worker with auto-reload..."
    require_command cargo-watch "Run: just init"
    cargo watch -w crates -x 'run -p everruns-worker'
    ;;

  start-dev)
    echo "🚀 Starting Everruns in DEV MODE (in-memory storage, no database required)..."
    echo ""

    if [ "$NO_WATCH" = false ]; then
      require_command cargo-watch "Run: just init (or use --no-watch)"
    fi
    require_command npm "Install Node.js/npm to start the UI."

    # Track child PIDs for cleanup
    CHILD_PIDS=()
    CLEANUP_DONE=false

    cleanup() {
      # Prevent multiple cleanup runs
      if [ "$CLEANUP_DONE" = true ]; then
        return
      fi
      CLEANUP_DONE=true

      # Ignore signals during cleanup to prevent interruption
      trap '' SIGINT SIGTERM

      stty sane 2>/dev/null || true
      echo ""
      echo "🛑 Stopping services..."

      # Send SIGTERM to tracked PIDs first
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -TERM "$pid" 2>/dev/null || true
        fi
      done

      # Kill by name (catches any child processes we didn't track directly)
      pkill -TERM -f "cargo-watch" 2>/dev/null || true
      pkill -TERM -f "everruns-control-plane" 2>/dev/null || true
      pkill -TERM -f "next dev" 2>/dev/null || true
      pkill -TERM -f "next-router-worker" 2>/dev/null || true

      # Give processes time to terminate gracefully
      sleep 1

      # Force kill any remaining processes
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -KILL "$pid" 2>/dev/null || true
        fi
      done
      pkill -KILL -f "cargo-watch" 2>/dev/null || true
      pkill -KILL -f "everruns-control-plane" 2>/dev/null || true
      pkill -KILL -f "next dev" 2>/dev/null || true
      pkill -KILL -f "next-router-worker" 2>/dev/null || true

      # Restore terminal state
      stty sane 2>/dev/null || true
      echo "✅ Services stopped"
      exit 0
    }

    trap cleanup SIGINT SIGTERM

    # Enable dev mode
    export DEV_MODE=true
    export DEPLOYMENT_GRADE=dev
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    export RUST_LOG=${RUST_LOG:-info}

    # Set encryption key if not provided (standard dev key from .env.example)
    if [ -z "${SECRETS_ENCRYPTION_KEY:-}" ]; then
      export SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY="
      echo "   ✅ Using default encryption key"
    fi

    # Configure LLM API keys
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

    # Disable OpenTelemetry in dev mode
    if [ -z "${OTEL_SDK_DISABLED:-}" ]; then
      export OTEL_SDK_DISABLED=true
      echo "   ℹ️  OpenTelemetry disabled (no collector in dev mode)"
    fi

    # Start control-plane
    if [ "$NO_WATCH" = true ]; then
      echo "1️⃣  Starting control-plane (DEV MODE)..."
      cargo run -p everruns-control-plane &
    else
      echo "1️⃣  Starting control-plane (DEV MODE) with auto-reload..."
      cargo watch -w crates -x 'run -p everruns-control-plane' &
    fi
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Wait for control-plane
    echo "2️⃣  Waiting for control-plane to be ready..."
    for i in {1..30}; do
      if curl -s http://localhost:9000/health > /dev/null 2>&1; then
        echo "   ✅ Control-plane is ready"
        break
      fi
      sleep 2
    done

    echo "3️⃣  Worker: Running in-process with control-plane (no separate worker needed)"

    # Check UI dependencies
    echo "4️⃣  Checking UI dependencies..."
    check_ui_deps || true

    # Start UI
    echo "5️⃣  Starting UI server..."
    cd "$PROJECT_ROOT/apps/ui"
    npm run dev &
    UI_PID=$!
    CHILD_PIDS+=("$UI_PID")
    cd "$PROJECT_ROOT"
    sleep 5
    echo "   ✅ UI is starting (PID: $UI_PID)"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ DEV MODE started (fully functional, in-memory storage)!"
    echo ""
    if [ "$NO_WATCH" = true ]; then
      echo "   🌐 Control-plane: http://localhost:9000"
    else
      echo "   🌐 Control-plane: http://localhost:9000 (auto-reload)"
    fi
    echo "   📖 API Docs:      http://localhost:9000/swagger-ui/"
    echo "   ⚙️  Worker:        Running in-process (no separate process)"
    echo "   🖥️  UI:            http://localhost:9100 (hot reload)"
    echo ""
    echo "⚠️  DEV MODE notes:"
    echo "   - Data is stored in memory (lost on restart)"
    echo "   - No PostgreSQL or Docker required"
    echo "   - Worker runs in-process with control-plane"
    echo ""
    if [ "$NO_WATCH" = false ]; then
      echo "👀 Edit code in crates/ and services will auto-restart"
    fi
    echo "💡 Press Ctrl+C to stop services"
    echo ""

    wait
    ;;

  start-all)
    echo "🚀 Starting Everruns development environment..."
    echo ""

    if [ "$NO_WATCH" = false ]; then
      require_command cargo-watch "Run: just init (or use --no-watch)"
    fi
    require_command sqlx "Run: just init"
    if [ "$NO_UI" = false ]; then
      require_command npm "Install Node.js/npm to start the UI (or use --no-ui)"
    fi

    CHILD_PIDS=()
    JAEGER_STARTED=false
    CLEANUP_DONE=false

    cleanup() {
      # Prevent multiple cleanup runs
      if [ "$CLEANUP_DONE" = true ]; then
        return
      fi
      CLEANUP_DONE=true

      # Ignore signals during cleanup to prevent interruption
      trap '' SIGINT SIGTERM

      stty sane 2>/dev/null || true
      echo ""
      echo "🛑 Stopping services..."

      # Send SIGTERM to tracked PIDs first
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -TERM "$pid" 2>/dev/null || true
        fi
      done

      # Kill by name (catches any child processes we didn't track directly)
      pkill -TERM -f "cargo-watch" 2>/dev/null || true
      pkill -TERM -f "everruns-control-plane" 2>/dev/null || true
      pkill -TERM -f "everruns-worker" 2>/dev/null || true
      pkill -TERM -f "next dev" 2>/dev/null || true
      pkill -TERM -f "next-router-worker" 2>/dev/null || true

      # Give processes time to terminate gracefully
      sleep 1

      # Force kill any remaining processes
      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -KILL "$pid" 2>/dev/null || true
        fi
      done
      pkill -KILL -f "cargo-watch" 2>/dev/null || true
      pkill -KILL -f "everruns-control-plane" 2>/dev/null || true
      pkill -KILL -f "everruns-worker" 2>/dev/null || true
      pkill -KILL -f "next dev" 2>/dev/null || true
      pkill -KILL -f "next-router-worker" 2>/dev/null || true

      # Restore terminal state
      stty sane 2>/dev/null || true
      echo "✅ Services stopped (Docker still running if started)"
      exit 0
    }

    trap cleanup SIGINT SIGTERM

    # Check PostgreSQL
    echo "1️⃣  Checking PostgreSQL..."
    if [ "$NO_DOCKER" = true ]; then
      # No Docker mode - require local PostgreSQL
      if check_port_open localhost 5432; then
        echo "   ✅ Local PostgreSQL is ready"
        export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
      else
        echo "   ❌ PostgreSQL not running on localhost:5432"
        echo "   Start PostgreSQL or remove --no-docker flag"
        exit 1
      fi
      export OTEL_SDK_DISABLED=true
      echo "   ℹ️  OpenTelemetry disabled (--no-docker)"
    elif check_port_open localhost 5432; then
      echo "   ✅ Local PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://postgres:postgres@localhost/everruns}
      if resolve_docker_compose 2>/dev/null; then
        if ! docker ps 2>/dev/null | grep -q jaeger; then
          echo "   ℹ️  Starting Jaeger for tracing..."
          ensure_docker_daemon || true
          cd "$PROJECT_ROOT/local"
          "${DOCKER_COMPOSE[@]}" up -d jaeger 2>/dev/null && JAEGER_STARTED=true
          cd "$PROJECT_ROOT"
        else
          JAEGER_STARTED=true
        fi
      fi
    elif command -v docker &> /dev/null && docker ps 2>/dev/null | grep -q postgres; then
      echo "   ✅ Docker PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
      if ! docker ps 2>/dev/null | grep -q jaeger; then
        echo "   ℹ️  Starting Jaeger for tracing..."
        if resolve_docker_compose 2>/dev/null; then
          cd "$PROJECT_ROOT/local"
          "${DOCKER_COMPOSE[@]}" up -d jaeger 2>/dev/null && JAEGER_STARTED=true
          cd "$PROJECT_ROOT"
        fi
      else
        JAEGER_STARTED=true
      fi
    else
      echo "   ⚠️  PostgreSQL not found. Starting via Docker..."
      if resolve_docker_compose; then
        ensure_docker_daemon || exit 1
        cd "$PROJECT_ROOT/local"
        "${DOCKER_COMPOSE[@]}" up -d postgres jaeger
        cd "$PROJECT_ROOT"
        sleep 3
        until docker exec everruns-postgres pg_isready -U everruns -d everruns > /dev/null 2>&1; do
          echo "   Waiting for Postgres to be ready..."
          sleep 1
        done
        export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
        echo "   ✅ Docker PostgreSQL and Jaeger started"
        JAEGER_STARTED=true
      else
        echo "   ❌ No PostgreSQL available. Start PostgreSQL or install Docker."
        exit 1
      fi
    fi

    if [ "$JAEGER_STARTED" = false ]; then
      echo "   ⚠️  Jaeger not available, disabling OpenTelemetry tracing"
      export OTEL_SDK_DISABLED=true
    fi

    # Run migrations
    echo "2️⃣  Running database migrations..."
    sqlx migrate run --source "$PROJECT_ROOT/crates/control-plane/migrations"
    echo "   ✅ Migrations complete"

    # Configure LLM API keys
    echo "3️⃣  Configuring LLM API keys from environment..."
    if [ -n "${OPENAI_API_KEY:-}" ]; then
      export DEFAULT_OPENAI_API_KEY="$OPENAI_API_KEY"
      echo "   ✅ OpenAI API key configured"
    else
      echo "   ⚠️  OPENAI_API_KEY not set"
    fi
    if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
      export DEFAULT_ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"
      echo "   ✅ Anthropic API key configured"
    else
      echo "   ⚠️  ANTHROPIC_API_KEY not set"
    fi

    # Start API
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-http://localhost:9100}
    export DEPLOYMENT_GRADE=dev
    export RUST_LOG=${RUST_LOG:-info}
    if [ "$NO_WATCH" = true ]; then
      echo "4️⃣  Starting API server..."
      cargo run -p everruns-control-plane &
    else
      echo "4️⃣  Starting API server with auto-reload..."
      cargo watch -w crates -x 'run -p everruns-control-plane' &
    fi
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Wait for API
    echo "5️⃣  Waiting for API to be ready..."
    for i in {1..30}; do
      if curl -s http://localhost:9000/health > /dev/null 2>&1; then
        echo "   ✅ API is ready"
        break
      fi
      sleep 2
    done

    # Start Worker with restart-on-crash logic
    # Restarts every 5 seconds for up to 90 seconds if worker fails to connect
    start_worker_with_restart() {
      local start_time=$(date +%s)
      local max_duration=90
      local retry_delay=5

      while true; do
        local now=$(date +%s)
        local elapsed=$((now - start_time))

        if [ $elapsed -ge $max_duration ]; then
          echo "   ❌ Worker failed to start after ${max_duration}s, giving up"
          return 1
        fi

        if [ "$NO_WATCH" = true ]; then
          cargo run -p everruns-worker
        else
          cargo watch -w crates -x 'run -p everruns-worker'
        fi
        local exit_code=$?

        # Check if we should retry
        now=$(date +%s)
        elapsed=$((now - start_time))
        if [ $elapsed -ge $max_duration ]; then
          echo "   ❌ Worker exited after ${max_duration}s total, not restarting"
          return $exit_code
        fi

        if [ $exit_code -ne 0 ]; then
          echo "   ⚠️  Worker exited with code $exit_code, restarting in ${retry_delay}s..."
          sleep $retry_delay
        else
          # Clean exit, don't restart
          return 0
        fi
      done
    }

    if [ "$NO_WATCH" = true ]; then
      echo "6️⃣  Starting worker..."
    else
      echo "6️⃣  Starting worker with auto-reload..."
    fi
    start_worker_with_restart &
    WORKER_PID=$!
    CHILD_PIDS+=("$WORKER_PID")
    sleep 2
    echo "   ✅ Worker is starting (PID: $WORKER_PID)"

    if [ "$NO_UI" = false ]; then
      # Check UI dependencies
      echo "7️⃣  Checking UI dependencies..."
      check_ui_deps || true

      # Start UI
      echo "8️⃣  Starting UI server..."
      cd "$PROJECT_ROOT/apps/ui"
      npm run dev &
      UI_PID=$!
      CHILD_PIDS+=("$UI_PID")
      cd "$PROJECT_ROOT"
      sleep 5
      echo "   ✅ UI is starting (PID: $UI_PID)"
    else
      echo "7️⃣  Skipping UI (--no-ui)"
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if [ "$NO_WATCH" = true ]; then
      echo "✅ All services started!"
    else
      echo "✅ All services started with auto-reload!"
    fi
    echo ""
    if [ "$NO_WATCH" = true ]; then
      echo "   🌐 API:         http://localhost:9000"
      echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
      echo "   ⚙️ Worker:      running"
    else
      echo "   🌐 API:         http://localhost:9000 (auto-reload)"
      echo "   📖 API Docs:    http://localhost:9000/swagger-ui/"
      echo "   ⚙️ Worker:      running (auto-reload)"
    fi
    if [ "$NO_UI" = false ]; then
      echo "   🖥️ UI:          http://localhost:9100 (hot reload)"
    fi
    if [ "$JAEGER_STARTED" = true ]; then
      echo "   🔍 Jaeger UI:   http://localhost:16686"
    elif [ "$NO_DOCKER" = false ]; then
      echo "   🔍 Jaeger:      disabled (no Docker)"
    fi
    echo ""
    if [ "$NO_WATCH" = false ]; then
      echo "👀 Edit code in crates/ and services will auto-restart"
    fi
    echo "💡 Press Ctrl+C to stop services"
    echo ""

    wait
    ;;

  stop-all)
    echo "🛑 Stopping all Everruns services..."

    pkill -f "everruns-control-plane" 2>/dev/null || true
    pkill -f "everruns-worker" 2>/dev/null || true
    pkill -f "next dev" 2>/dev/null || true

    if resolve_docker_compose 2>/dev/null; then
      cd "$PROJECT_ROOT/local"
      "${DOCKER_COMPOSE[@]}" down
      cd "$PROJECT_ROOT"
    fi

    echo "✅ All services stopped!"
    ;;

  *)
    echo "Usage: $0 {control-plane|worker|watch-control-plane|watch-worker|start-dev|start-all|stop-all} [options]"
    echo ""
    echo "Options:"
    echo "  --no-watch    Don't use cargo-watch (faster startup, no auto-reload)"
    echo "  --no-docker   Don't use Docker (requires local PostgreSQL for start-all)"
    echo "  --no-ui       Don't start the UI server"
    exit 1
    ;;
esac
