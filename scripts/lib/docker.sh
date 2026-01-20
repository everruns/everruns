#!/usr/bin/env bash
# Docker operations: start, stop, reset, logs, migrate

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"

case "$cmd" in
  start)
    echo "🚀 Starting Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    "${DOCKER_COMPOSE[@]}" up -d
    echo "✅ Services started!"
    echo "   - Postgres: localhost:5432"
    echo "   - Jaeger UI: http://localhost:16686"
    echo "   - OTLP gRPC: localhost:4317"
    ;;

  stop)
    echo "🛑 Stopping Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    "${DOCKER_COMPOSE[@]}" down
    echo "✅ Services stopped!"
    ;;

  reset)
    echo "🔄 Resetting Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    "${DOCKER_COMPOSE[@]}" down -v
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
    echo "Usage: $0 {start|stop|reset|logs|migrate}"
    exit 1
    ;;
esac
