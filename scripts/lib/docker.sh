#!/usr/bin/env bash
# Docker operations: start, stop, reset, logs, migrate

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
profile="${2:-}"

# Helper to get compose profile args
get_profile_args() {
  local p="$1"
  case "$p" in
    phoenix)
      echo "--profile phoenix"
      ;;
    jaeger|"")
      # Default to Jaeger
      echo "--profile default"
      ;;
    *)
      echo "❌ Unknown profile: $p. Use 'jaeger' or 'phoenix'." >&2
      exit 1
      ;;
  esac
}

case "$cmd" in
  start)
    profile_args=$(get_profile_args "$profile")
    echo "🚀 Starting Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    # shellcheck disable=SC2086
    "${DOCKER_COMPOSE[@]}" $profile_args up -d
    echo "✅ Services started!"
    echo "   - Postgres: localhost:5432"
    if [ "$profile" = "phoenix" ]; then
      echo "   - Phoenix UI: http://localhost:6006"
    else
      echo "   - Jaeger UI: http://localhost:16686"
    fi
    echo "   - OTLP gRPC: localhost:4317"
    ;;

  stop)
    echo "🛑 Stopping Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    # Stop all profiles
    "${DOCKER_COMPOSE[@]}" --profile default --profile phoenix down
    echo "✅ Services stopped!"
    ;;

  reset)
    echo "🔄 Resetting Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    # Reset all profiles
    "${DOCKER_COMPOSE[@]}" --profile default --profile phoenix down -v
    echo "✅ Services reset!"
    ;;

  logs)
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    "${DOCKER_COMPOSE[@]}" logs -f
    ;;

  migrate)
    echo "🔧 Running database migrations..."
    export DATABASE_URL=${DATABASE_URL:-postgres://everruns:everruns@localhost:5432/everruns}
    sqlx migrate run --source "$PROJECT_ROOT/crates/control-plane/migrations"
    echo "✅ Migrations complete!"
    ;;

  *)
    echo "Usage: $0 {start|stop|reset|logs|migrate} [profile]"
    echo ""
    echo "Commands:"
    echo "  start [profile]  Start Docker services (default: jaeger)"
    echo "  stop             Stop all Docker services"
    echo "  reset            Stop and remove all volumes"
    echo "  logs             Follow service logs"
    echo "  migrate          Run database migrations"
    echo ""
    echo "Profiles:"
    echo "  jaeger   Use Jaeger for tracing (default)"
    echo "  phoenix  Use Arize Phoenix for LLM observability"
    echo ""
    echo "Examples:"
    echo "  $0 start           # Start with Jaeger"
    echo "  $0 start phoenix   # Start with Phoenix"
    exit 1
    ;;
esac
