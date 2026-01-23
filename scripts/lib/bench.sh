#!/usr/bin/env bash
# Benchmark operations: durable-bench, durable-bench-db

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"
shift || true

parse_bench_args() {
  SAVE_ARG=""
  MONIKER_ARG=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --save) SAVE_ARG="--save" ;;
      --moniker) MONIKER_ARG="--moniker $2"; shift ;;
      *) [ -z "$MONIKER_ARG" ] && [ -n "$SAVE_ARG" ] && MONIKER_ARG="--moniker $1" ;;
    esac
    shift || true
  done
}

run_benchmarks() {
  echo ""
  echo "Running concurrent_workers..."
  cargo bench -p everruns-durable --bench concurrent_workers -- $SAVE_ARG $MONIKER_ARG
  echo ""
  echo "Running workflow_throughput..."
  cargo bench -p everruns-durable --bench workflow_throughput -- $SAVE_ARG $MONIKER_ARG
  echo ""
  echo "Running cold_start_latency..."
  cargo bench -p everruns-durable --bench cold_start_latency -- $SAVE_ARG $MONIKER_ARG
  echo ""
}

case "$cmd" in
  memory)
    parse_bench_args "$@"

    if [ -n "$SAVE_ARG" ]; then
      echo "📊 Running durable execution benchmarks (with checkpointing)..."
      [ -n "$MONIKER_ARG" ] && echo "   Moniker: ${MONIKER_ARG#--moniker }"
    else
      echo "📊 Running durable execution benchmarks..."
    fi

    run_benchmarks

    if [ -n "$SAVE_ARG" ]; then
      echo "✅ Benchmarks complete with checkpoints saved!"
      echo "   Reports:     target/benchmark-reports/"
      echo "   Checkpoints: crates/durable/benches/checkpoints/"
    else
      echo "✅ Benchmarks complete!"
      echo "   Reports: target/benchmark-reports/"
      echo "   💡 Tip: Use --save to save checkpoints for comparison"
    fi
    ;;

  db)
    parse_bench_args "$@"

    if [ -n "$SAVE_ARG" ]; then
      echo "📊 Running durable execution benchmarks (PostgreSQL, with checkpointing)..."
      [ -n "$MONIKER_ARG" ] && echo "   Moniker: ${MONIKER_ARG#--moniker }"
    else
      echo "📊 Running durable execution benchmarks (PostgreSQL)..."
    fi
    echo ""

    DB_HOST="${DB_HOST:-localhost}"
    DB_PORT="${DB_PORT:-5432}"
    DB_NAME="${DB_NAME:-everruns_test}"
    USING_DOCKER_POSTGRES=false

    echo "1️⃣  Checking PostgreSQL connection..."

    if command -v docker &> /dev/null && docker ps 2>/dev/null | grep -q everruns-postgres; then
      echo "   Found Docker PostgreSQL container"
      USING_DOCKER_POSTGRES=true
      DB_USER="everruns"
      DB_PASS="everruns"
      echo "   Waiting for PostgreSQL to be ready..."
      until docker exec everruns-postgres pg_isready -U everruns -d everruns > /dev/null 2>&1; do
        sleep 1
      done
      echo "   ✅ PostgreSQL is ready"
    elif check_postgres_ready "$DB_HOST" "$DB_PORT" "${DB_USER:-postgres}"; then
      echo "   ✅ Local PostgreSQL is ready"
      DB_USER="${DB_USER:-postgres}"
      DB_PASS="${DB_PASS:-postgres}"
    else
      echo "   ⚠️  PostgreSQL not found. Starting via Docker..."
      if resolve_docker_compose; then
        ensure_docker_daemon || exit 1
        cd "$PROJECT_ROOT/local"
        "${DOCKER_COMPOSE[@]}" up -d postgres
        cd "$PROJECT_ROOT"
        sleep 3
        until docker exec everruns-postgres pg_isready -U everruns -d everruns > /dev/null 2>&1; do
          echo "   Waiting for Postgres to be ready..."
          sleep 1
        done
        echo "   ✅ Docker PostgreSQL started"
        USING_DOCKER_POSTGRES=true
        DB_USER="everruns"
        DB_PASS="everruns"
      else
        echo "   ❌ No PostgreSQL available. Start PostgreSQL or install Docker."
        exit 1
      fi
    fi

    # Set DATABASE_URL for benchmarks
    if [ "$USING_DOCKER_POSTGRES" = true ]; then
      export BENCHMARK_DATABASE_URL="postgres://${DB_USER}:${DB_PASS}@${DB_HOST}:${DB_PORT}/${DB_NAME}"
    else
      export BENCHMARK_DATABASE_URL="postgres://${DB_USER}:${DB_PASS}@${DB_HOST}:${DB_PORT}/${DB_NAME}"
    fi

    echo "2️⃣  Running benchmarks against PostgreSQL..."
    run_benchmarks

    if [ -n "$SAVE_ARG" ]; then
      echo "✅ PostgreSQL benchmarks complete with checkpoints saved!"
    else
      echo "✅ PostgreSQL benchmarks complete!"
    fi
    echo "   Reports: target/benchmark-reports/"
    ;;

  *)
    echo "Usage: $0 {memory|db} [--save] [moniker]"
    exit 1
    ;;
esac
