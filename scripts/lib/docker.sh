#!/usr/bin/env bash
# Docker operations: start, stop, reset, logs

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"

apply_port_prefix_defaults

case "$cmd" in
  start)
    echo "🚀 Starting Everrun development environment..."
    resolve_docker_compose || exit 1
    cd "$PROJECT_ROOT/local"
    "${DOCKER_COMPOSE[@]}" up -d
    echo "✅ Services started!"
    echo "   - Postgres: localhost:${DB_PORT}"
    echo "   - Valkey:   localhost:${VALKEY_PORT}"
    echo "   - OTLP gRPC: localhost:${OTEL_GRPC_PORT}"
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

  *)
    echo "Usage: $0 {start|stop|reset|logs}"
    exit 1
    ;;
esac
