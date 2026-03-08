#!/usr/bin/env bash
# Service operations: server, worker, watch-*, start-dev, start-all, stop-all

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

apply_port_prefix_defaults
API_ADDR_DEFAULT="0.0.0.0:${API_PORT}"
PROXY_URL_DEFAULT="http://localhost:${PROXY_PORT}"
DB_URL_DEFAULT="postgres://everruns:everruns@localhost:${DB_PORT}/everruns"

# require_command is defined in common.sh (sourced above)

ui_dev_args=()
if [ -L "${PROJECT_ROOT:-$(pwd)}/apps/ui/node_modules" ]; then
  ui_dev_args+=(--webpack)
fi
if [ -n "${UI_DEV_ARGS:-}" ]; then
  read -r -a ui_dev_args_extra <<< "$UI_DEV_ARGS"
  ui_dev_args+=("${ui_dev_args_extra[@]}")
fi

run_ui_dev() {
  PORT="$UI_PORT" ./node_modules/.bin/next dev --port "$UI_PORT" "${ui_dev_args[@]}"
}

run_ui_start() {
  PORT="$UI_PORT" ./node_modules/.bin/next start --port "$UI_PORT"
}

case "$cmd" in
  server)
    echo "🌐 Starting server..."
    export ADDR=${ADDR:-$API_ADDR_DEFAULT}
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-$PROXY_URL_DEFAULT}
    export PUBLIC_APP_URL=${PUBLIC_APP_URL:-$PROXY_URL_DEFAULT}
    export APP_URL=${APP_URL:-$PUBLIC_APP_URL}
    cargo run -p everruns-server
    ;;

  worker)
    echo "⚙️  Starting worker..."
    cargo run -p everruns-worker
    ;;

  watch-server)
    echo "👀 Starting server with auto-reload..."
    require_command cargo-watch "Run: just init"
    export ADDR=${ADDR:-$API_ADDR_DEFAULT}
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-$PROXY_URL_DEFAULT}
    export PUBLIC_APP_URL=${PUBLIC_APP_URL:-$PROXY_URL_DEFAULT}
    export APP_URL=${APP_URL:-$PUBLIC_APP_URL}
    cargo watch -w crates -x 'run -p everruns-server'
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
    require_command caddy "Run: just init"

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
      pkill -TERM -f "caddy run" 2>/dev/null || true
      pkill -TERM -f "cargo-watch" 2>/dev/null || true
      pkill -TERM -f "everruns-server" 2>/dev/null || true
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
      pkill -KILL -f "caddy run" 2>/dev/null || true
      pkill -KILL -f "cargo-watch" 2>/dev/null || true
      pkill -KILL -f "everruns-server" 2>/dev/null || true
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
    export API_PORT UI_PORT PROXY_PORT
    export ADDR=${ADDR:-$API_ADDR_DEFAULT}
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-$PROXY_URL_DEFAULT}
    export PUBLIC_APP_URL=${PUBLIC_APP_URL:-$PROXY_URL_DEFAULT}
    export APP_URL=${APP_URL:-$PUBLIC_APP_URL}
    export AUTH_BASE_URL=${AUTH_BASE_URL:-${PROXY_URL_DEFAULT}/api}
    export FRONTEND_URL=${FRONTEND_URL:-$PROXY_URL_DEFAULT}
    export RUST_LOG=${RUST_LOG:-info}

    # Set encryption key if not provided (standard dev key from .env.example)
    if [ -z "${SECRETS_ENCRYPTION_KEY:-}" ]; then
      export SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY="
      echo "   ✅ Using default encryption key"
    fi

    print_doppler_secret_hint

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
    if [ -n "${GEMINI_API_KEY:-}" ]; then
      export DEFAULT_GEMINI_API_KEY="$GEMINI_API_KEY"
      echo "   ✅ Gemini API key configured"
    else
      echo "   ⚠️  GEMINI_API_KEY not set (Gemini models may not work)"
    fi

    # Disable OpenTelemetry in dev mode
    if [ -z "${OTEL_SDK_DISABLED:-}" ]; then
      export OTEL_SDK_DISABLED=true
      echo "   ℹ️  OpenTelemetry disabled (no collector in dev mode)"
    fi

    # Start server
    if [ "$NO_WATCH" = true ]; then
      echo "1️⃣  Starting server (DEV MODE)..."
      cargo run -p everruns-server &
    else
      echo "1️⃣  Starting server (DEV MODE) with auto-reload..."
      cargo watch -w crates -x 'run -p everruns-server' &
    fi
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Wait for server
    echo "2️⃣  Waiting for server to be ready..."
    for i in {1..30}; do
      if curl -s "http://localhost:${API_PORT}/health" > /dev/null 2>&1; then
        echo "   ✅ Server is ready"
        break
      fi
      sleep 2
    done

    echo "3️⃣  Worker: Running in-process with server (no separate worker needed)"

    # Check UI dependencies
    echo "4️⃣  Checking UI dependencies..."
    check_ui_deps || true

    # Start UI
    echo "5️⃣  Starting UI server..."
    cd "$PROJECT_ROOT/apps/ui"
    run_ui_dev &
    UI_PID=$!
    CHILD_PIDS+=("$UI_PID")
    cd "$PROJECT_ROOT"
    sleep 5
    echo "   ✅ UI is starting (PID: $UI_PID)"

    # Start Caddy reverse proxy
    echo "6️⃣  Starting reverse proxy (Caddy)..."
    caddy run --config "$PROJECT_ROOT/local/Caddyfile" --adapter caddyfile &
    CADDY_PID=$!
    CHILD_PIDS+=("$CADDY_PID")
    sleep 1
    echo "   ✅ Reverse proxy is running on :${PROXY_PORT} (PID: $CADDY_PID)"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ DEV MODE started (fully functional, in-memory storage)!"
    echo ""
    echo "   🌐 App:           http://localhost:${PROXY_PORT}"
    echo "   🔌 API:           http://localhost:${PROXY_PORT}/api/..."
    echo "   ⚙️  Worker:        Running in-process (no separate process)"
    echo ""
    echo "⚠️  DEV MODE notes:"
    echo "   - Data is stored in memory (lost on restart)"
    echo "   - No PostgreSQL or Docker required"
    echo "   - Worker runs in-process with server"
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
      require_command caddy "Run: just init"
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
      pkill -TERM -f "caddy run" 2>/dev/null || true
      pkill -TERM -f "cargo-watch" 2>/dev/null || true
      pkill -TERM -f "everruns-server" 2>/dev/null || true
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
      pkill -KILL -f "caddy run" 2>/dev/null || true
      pkill -KILL -f "cargo-watch" 2>/dev/null || true
      pkill -KILL -f "everruns-server" 2>/dev/null || true
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
      if check_postgres_ready localhost "$DB_PORT" everruns; then
        echo "   ✅ Local PostgreSQL is ready"
        export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
      else
        echo "   ❌ PostgreSQL not running or not responding on localhost:${DB_PORT}"
        echo "   Start PostgreSQL or remove --no-docker flag"
        exit 1
      fi
      export OTEL_SDK_DISABLED=true
      echo "   ℹ️  OpenTelemetry disabled (--no-docker)"
    elif check_postgres_ready localhost "$DB_PORT" postgres; then
      echo "   ✅ Local PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://postgres:postgres@localhost:${DB_PORT}/everruns}
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
      export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
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
        until check_postgres_ready localhost "$DB_PORT" everruns; do
          echo "   Waiting for Postgres to be ready..."
          sleep 1
        done
        export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
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

    # Start Valkey for distributed rate limiting
    if [ "$NO_DOCKER" = true ]; then
      if command -v valkey-cli &> /dev/null && valkey-cli ping 2>/dev/null | grep -q PONG; then
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Local Valkey ready (distributed rate limiting enabled)"
      elif command -v redis-cli &> /dev/null && redis-cli ping 2>/dev/null | grep -q PONG; then
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Local Redis-compatible server ready (distributed rate limiting enabled)"
      else
        echo "   ℹ️  Valkey not available — using per-instance rate limiting"
      fi
    elif [ -z "${VALKEY_URL:-}" ]; then
      if resolve_docker_compose 2>/dev/null; then
        if ! docker ps 2>/dev/null | grep -q valkey; then
          echo "   ℹ️  Starting Valkey for distributed rate limiting..."
          cd "$PROJECT_ROOT/local"
          "${DOCKER_COMPOSE[@]}" up -d valkey 2>/dev/null
          cd "$PROJECT_ROOT"
        fi
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Valkey started (distributed rate limiting enabled)"
      fi
    fi

    print_doppler_secret_hint

    # Configure LLM API keys
    echo "2️⃣  Configuring LLM API keys from environment..."
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
    if [ -n "${GEMINI_API_KEY:-}" ]; then
      export DEFAULT_GEMINI_API_KEY="$GEMINI_API_KEY"
      echo "   ✅ Gemini API key configured"
    else
      echo "   ⚠️  GEMINI_API_KEY not set"
    fi

    # Start API
    export API_PORT UI_PORT PROXY_PORT
    export ADDR=${ADDR:-$API_ADDR_DEFAULT}
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-$PROXY_URL_DEFAULT}
    export PUBLIC_APP_URL=${PUBLIC_APP_URL:-$PROXY_URL_DEFAULT}
    export APP_URL=${APP_URL:-$PUBLIC_APP_URL}
    export AUTH_BASE_URL=${AUTH_BASE_URL:-${PROXY_URL_DEFAULT}/api}
    export FRONTEND_URL=${FRONTEND_URL:-$PROXY_URL_DEFAULT}
    export DEPLOYMENT_GRADE=dev
    export RUST_LOG=${RUST_LOG:-info}
    if [ "$NO_WATCH" = true ]; then
      echo "3️⃣  Starting API server (auto-migrates on startup)..."
      cargo run -p everruns-server &
    else
      echo "3️⃣  Starting API server with auto-reload (auto-migrates on startup)..."
      cargo watch -w crates -x 'run -p everruns-server' &
    fi
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Wait for API
    echo "4️⃣  Waiting for API to be ready..."
    for i in {1..30}; do
      if curl -s "http://localhost:${API_PORT}/health" > /dev/null 2>&1; then
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
      run_ui_dev &
      UI_PID=$!
      CHILD_PIDS+=("$UI_PID")
      cd "$PROJECT_ROOT"
      sleep 5
      echo "   ✅ UI is starting (PID: $UI_PID)"

      # Start Caddy reverse proxy
      echo "9️⃣  Starting reverse proxy (Caddy)..."
      caddy run --config "$PROJECT_ROOT/local/Caddyfile" --adapter caddyfile &
      CADDY_PID=$!
      CHILD_PIDS+=("$CADDY_PID")
      sleep 1
      echo "   ✅ Reverse proxy is running on :${PROXY_PORT} (PID: $CADDY_PID)"
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
    if [ "$NO_UI" = false ]; then
      echo "   🌐 App:         http://localhost:${PROXY_PORT}"
      echo "   🔌 API:         http://localhost:${PROXY_PORT}/api/..."
    else
      echo "   🌐 API:         http://localhost:${API_PORT}"
    fi
    if [ "$NO_WATCH" = false ]; then
      echo "   ⚙️ Worker:      running (auto-reload)"
    else
      echo "   ⚙️ Worker:      running"
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

  start-production)
    echo "🚀 Starting Everruns in PRODUCTION MODE (release builds, no watchers)..."
    echo ""

    require_command sqlx "Run: just init"
    if [ "$NO_UI" = false ]; then
      require_command npm "Install Node.js/npm to build the UI (or use --no-ui)"
      require_command caddy "Run: just init"
    fi

    CHILD_PIDS=()
    JAEGER_STARTED=false
    CLEANUP_DONE=false

    cleanup() {
      if [ "$CLEANUP_DONE" = true ]; then
        return
      fi
      CLEANUP_DONE=true

      trap '' SIGINT SIGTERM

      stty sane 2>/dev/null || true
      echo ""
      echo "🛑 Stopping services..."

      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -TERM "$pid" 2>/dev/null || true
        fi
      done

      pkill -TERM -f "caddy run" 2>/dev/null || true
      pkill -TERM -f "everruns-server" 2>/dev/null || true
      pkill -TERM -f "everruns-worker" 2>/dev/null || true
      pkill -TERM -f "next-server" 2>/dev/null || true

      sleep 1

      for pid in "${CHILD_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -KILL "$pid" 2>/dev/null || true
        fi
      done
      pkill -KILL -f "caddy run" 2>/dev/null || true
      pkill -KILL -f "everruns-server" 2>/dev/null || true
      pkill -KILL -f "everruns-worker" 2>/dev/null || true
      pkill -KILL -f "next-server" 2>/dev/null || true

      stty sane 2>/dev/null || true
      echo "✅ Services stopped (Docker still running if started)"
      exit 0
    }

    trap cleanup SIGINT SIGTERM

    # Check PostgreSQL (same as start-all)
    echo "1️⃣  Checking PostgreSQL..."
    if [ "$NO_DOCKER" = true ]; then
      if check_postgres_ready localhost "$DB_PORT" everruns; then
        echo "   ✅ Local PostgreSQL is ready"
        export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
      else
        echo "   ❌ PostgreSQL not running or not responding on localhost:${DB_PORT}"
        echo "   Start PostgreSQL or remove --no-docker flag"
        exit 1
      fi
      export OTEL_SDK_DISABLED=true
      echo "   ℹ️  OpenTelemetry disabled (--no-docker)"
    elif check_postgres_ready localhost "$DB_PORT" postgres; then
      echo "   ✅ Local PostgreSQL is ready"
      export DATABASE_URL=${DATABASE_URL:-postgres://postgres:postgres@localhost:${DB_PORT}/everruns}
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
      export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
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
        until check_postgres_ready localhost "$DB_PORT" everruns; do
          echo "   Waiting for Postgres to be ready..."
          sleep 1
        done
        export DATABASE_URL=${DATABASE_URL:-$DB_URL_DEFAULT}
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

    # Start Valkey for distributed rate limiting
    if [ "$NO_DOCKER" = true ]; then
      if command -v valkey-cli &> /dev/null && valkey-cli ping 2>/dev/null | grep -q PONG; then
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Local Valkey ready (distributed rate limiting enabled)"
      elif command -v redis-cli &> /dev/null && redis-cli ping 2>/dev/null | grep -q PONG; then
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Local Redis-compatible server ready (distributed rate limiting enabled)"
      else
        echo "   ℹ️  Valkey not available — using per-instance rate limiting"
      fi
    elif [ -z "${VALKEY_URL:-}" ]; then
      if resolve_docker_compose 2>/dev/null; then
        if ! docker ps 2>/dev/null | grep -q valkey; then
          echo "   ℹ️  Starting Valkey for distributed rate limiting..."
          cd "$PROJECT_ROOT/local"
          "${DOCKER_COMPOSE[@]}" up -d valkey 2>/dev/null
          cd "$PROJECT_ROOT"
        fi
        export VALKEY_URL=${VALKEY_URL:-redis://localhost:6379}
        echo "   ✅ Valkey started (distributed rate limiting enabled)"
      fi
    fi

    print_doppler_secret_hint

    # Configure LLM API keys
    echo "2️⃣  Configuring LLM API keys from environment..."
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
    if [ -n "${GEMINI_API_KEY:-}" ]; then
      export DEFAULT_GEMINI_API_KEY="$GEMINI_API_KEY"
      echo "   ✅ Gemini API key configured"
    else
      echo "   ⚠️  GEMINI_API_KEY not set"
    fi

    # Build release binaries
    echo "3️⃣  Building release binaries (server + worker)..."
    cargo build --release -p everruns-server -p everruns-worker
    echo "   ✅ Release binaries built"

    # Build UI
    if [ "$NO_UI" = false ]; then
      echo "4️⃣  Building UI (production)..."
      check_ui_deps || true
      cd "$PROJECT_ROOT/apps/ui"
      npm run build
      cd "$PROJECT_ROOT"
      echo "   ✅ UI built"
    fi

    # Start server
    export API_PORT UI_PORT PROXY_PORT
    export ADDR=${ADDR:-$API_ADDR_DEFAULT}
    export CORS_ALLOWED_ORIGINS=${CORS_ALLOWED_ORIGINS:-$PROXY_URL_DEFAULT}
    export PUBLIC_APP_URL=${PUBLIC_APP_URL:-$PROXY_URL_DEFAULT}
    export APP_URL=${APP_URL:-$PUBLIC_APP_URL}
    export AUTH_BASE_URL=${AUTH_BASE_URL:-${PROXY_URL_DEFAULT}/api}
    export FRONTEND_URL=${FRONTEND_URL:-$PROXY_URL_DEFAULT}
    export DEPLOYMENT_GRADE=dev
    export RUST_LOG=${RUST_LOG:-info}

    echo "5️⃣  Starting server (release)..."
    "$PROJECT_ROOT/target/release/everruns-server" &
    API_PID=$!
    CHILD_PIDS+=("$API_PID")
    sleep 3

    # Wait for server
    echo "6️⃣  Waiting for server to be ready..."
    for i in {1..30}; do
      if curl -s "http://localhost:${API_PORT}/health" > /dev/null 2>&1; then
        echo "   ✅ Server is ready"
        break
      fi
      sleep 2
    done

    # Start worker
    echo "7️⃣  Starting worker (release)..."
    "$PROJECT_ROOT/target/release/everruns-worker" &
    WORKER_PID=$!
    CHILD_PIDS+=("$WORKER_PID")
    sleep 2
    echo "   ✅ Worker is running (PID: $WORKER_PID)"

    if [ "$NO_UI" = false ]; then
      # Start UI production server
      echo "8️⃣  Starting UI server (production)..."
      cd "$PROJECT_ROOT/apps/ui"
      run_ui_start &
      UI_PID=$!
      CHILD_PIDS+=("$UI_PID")
      cd "$PROJECT_ROOT"
      sleep 3
      echo "   ✅ UI is running (PID: $UI_PID)"

      # Start Caddy reverse proxy
      echo "9️⃣  Starting reverse proxy (Caddy)..."
      caddy run --config "$PROJECT_ROOT/local/Caddyfile" --adapter caddyfile &
      CADDY_PID=$!
      CHILD_PIDS+=("$CADDY_PID")
      sleep 1
      echo "   ✅ Reverse proxy is running on :${PROXY_PORT} (PID: $CADDY_PID)"
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ PRODUCTION MODE started (release builds, no watchers)!"
    echo ""
    if [ "$NO_UI" = false ]; then
      echo "   🌐 App:         http://localhost:${PROXY_PORT}"
      echo "   🔌 API:         http://localhost:${PROXY_PORT}/api/..."
    else
      echo "   🌐 API:         http://localhost:${API_PORT}"
    fi
    echo "   ⚙️ Worker:      running (release)"
    if [ "$JAEGER_STARTED" = true ]; then
      echo "   🔍 Jaeger UI:   http://localhost:16686"
    fi
    echo ""
    echo "💡 Press Ctrl+C to stop services"
    echo ""

    wait
    ;;

  stop-all)
    echo "🛑 Stopping all Everruns services..."

    pkill -f "caddy run" 2>/dev/null || true
    pkill -f "everruns-server" 2>/dev/null || true
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
    echo "Usage: $0 {server|worker|watch-server|watch-worker|start-dev|start-all|start-production|stop-all} [options]"
    echo ""
    echo "Options:"
    echo "  --no-watch    Don't use cargo-watch (faster startup, no auto-reload)"
    echo "  --no-docker   Don't use Docker (requires local PostgreSQL for start-all)"
    echo "  --no-ui       Don't start the UI server"
    exit 1
    ;;
esac
