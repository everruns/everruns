#!/usr/bin/env bash
# Infrastructure services: start, stop, reset, logs
# Runs PostgreSQL via pg_ctl and Valkey/Redis as native processes.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-}"

apply_port_prefix_defaults

# --- Paths & config ---
# Store data under $PROJECT_ROOT/.local/data/ so it persists across reboots
# and is isolated per repo/worktree. The .local/ dir is gitignored.

LOCAL_DATA_DIR="${PROJECT_ROOT}/.local/data"
PGDATA="${PGDATA:-${LOCAL_DATA_DIR}/pgdata-${DB_PORT}}"
PG_LOGFILE="${PGDATA}/pg.log"
VALKEY_DATA_DIR="${LOCAL_DATA_DIR}/valkey-${VALKEY_PORT}"
VALKEY_PIDFILE="${VALKEY_DATA_DIR}/valkey.pid"
VALKEY_LOGFILE="${VALKEY_DATA_DIR}/valkey.log"
PG_USER="${PG_USER:-everruns}"
PG_DB="${PG_DB:-everruns}"

# Run a command as an unprivileged user when we're root.
# PostgreSQL refuses to run initdb/pg_ctl as root, so we use the 'postgres'
# system user (creating it if needed).
run_pg() {
  if [ "$(id -u)" -eq 0 ]; then
    if ! id postgres &>/dev/null; then
      useradd -r -s /bin/bash postgres 2>/dev/null || true
    fi
    su -s /bin/bash postgres -c "export PATH=$PG_BIN:\$PATH; $*"
  else
    eval "$@"
  fi
}

# Detect pg_ctl location
detect_pg_bin() {
  if command -v pg_ctl &> /dev/null; then
    PG_BIN="$(dirname "$(command -v pg_ctl)")"
    return 0
  fi
  if [ -d "/usr/lib/postgresql" ]; then
    local version
    version=$(ls /usr/lib/postgresql 2>/dev/null | sort -V | tail -1)
    if [ -n "$version" ] && [ -f "/usr/lib/postgresql/$version/bin/pg_ctl" ]; then
      PG_BIN="/usr/lib/postgresql/$version/bin"
      return 0
    fi
  fi
  echo "❌ pg_ctl not found. Install PostgreSQL (apt-get install postgresql)."
  return 1
}

# Detect valkey-server or redis-server
detect_valkey_bin() {
  if command -v valkey-server &> /dev/null; then
    VALKEY_SERVER="valkey-server"
    VALKEY_CLI="valkey-cli"
    return 0
  fi
  if command -v redis-server &> /dev/null; then
    VALKEY_SERVER="redis-server"
    VALKEY_CLI="${VALKEY_CLI_CMD:-redis-cli}"
    if command -v valkey-cli &> /dev/null; then
      VALKEY_CLI="valkey-cli"
    fi
    return 0
  fi
  echo "⚠️  No valkey-server or redis-server found. Valkey will be skipped."
  return 1
}

# --- PostgreSQL ---

pg_is_running() {
  run_pg "$PG_BIN/pg_ctl -D $PGDATA status" &>/dev/null
}

start_postgres() {
  detect_pg_bin || return 1

  if pg_is_running; then
    echo "   ✅ PostgreSQL already running (port ${DB_PORT})"
    return 0
  fi

  if [ ! -d "$PGDATA/base" ]; then
    echo "   Initializing PostgreSQL cluster at $PGDATA..."
    mkdir -p "$PGDATA"
    # Ensure the data dir is owned by the right user
    if [ "$(id -u)" -eq 0 ]; then
      chown postgres:postgres "$PGDATA"
    fi
    run_pg "$PG_BIN/initdb -D $PGDATA --auth=trust -U $PG_USER" >/dev/null 2>&1

    # Configure for local connections on the right port
    cat >> "$PGDATA/postgresql.conf" <<EOF
listen_addresses = 'localhost'
port = ${DB_PORT}
unix_socket_directories = '$PGDATA'
EOF
    # Fix ownership of config we just appended
    if [ "$(id -u)" -eq 0 ]; then
      chown postgres:postgres "$PGDATA/postgresql.conf"
    fi
  fi

  echo "   Starting PostgreSQL on port ${DB_PORT}..."
  if [ "$(id -u)" -eq 0 ]; then
    touch "$PG_LOGFILE"
    chown postgres:postgres "$PG_LOGFILE"
  fi
  run_pg "$PG_BIN/pg_ctl -D $PGDATA -l $PG_LOGFILE -o '-p ${DB_PORT}' start" >/dev/null 2>&1

  # Wait for ready
  for i in {1..15}; do
    if run_pg "$PG_BIN/pg_isready -h localhost -p ${DB_PORT} -U $PG_USER -q -t 2" 2>/dev/null; then
      # Create database if missing (cut first column to avoid matching the owner name)
      if ! run_pg "psql -h localhost -p ${DB_PORT} -U $PG_USER -lqt" 2>/dev/null | cut -d'|' -f1 | grep -qw "$PG_DB"; then
        run_pg "$PG_BIN/createdb -h localhost -p ${DB_PORT} -U $PG_USER $PG_DB" 2>/dev/null || true
      fi
      echo "   ✅ PostgreSQL started on localhost:${DB_PORT}"
      return 0
    fi
    sleep 1
  done

  echo "   ❌ PostgreSQL failed to start (see $PG_LOGFILE)"
  return 1
}

stop_postgres() {
  detect_pg_bin || return 0
  if pg_is_running; then
    run_pg "$PG_BIN/pg_ctl -D $PGDATA stop -m fast" >/dev/null 2>&1
    echo "   ✅ PostgreSQL stopped"
  fi
}

# --- Valkey/Redis ---

valkey_is_running() {
  if [ -f "$VALKEY_PIDFILE" ]; then
    local pid
    pid=$(cat "$VALKEY_PIDFILE" 2>/dev/null || true)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
    return $?
  fi
  return 1
}

start_valkey() {
  if ! detect_valkey_bin; then
    return 1
  fi

  if valkey_is_running; then
    echo "   ✅ Valkey already running (port ${VALKEY_PORT})"
    return 0
  fi

  echo "   Starting Valkey on port ${VALKEY_PORT}..."
  mkdir -p "$VALKEY_DATA_DIR"
  $VALKEY_SERVER \
    --port "$VALKEY_PORT" \
    --daemonize yes \
    --pidfile "$VALKEY_PIDFILE" \
    --logfile "$VALKEY_LOGFILE" \
    --save "" >/dev/null 2>&1

  for i in {1..10}; do
    if $VALKEY_CLI -p "$VALKEY_PORT" ping 2>/dev/null | grep -q PONG; then
      echo "   ✅ Valkey started on localhost:${VALKEY_PORT}"
      return 0
    fi
    sleep 1
  done

  echo "   ⚠️  Valkey did not start (see $VALKEY_LOGFILE)"
  return 1
}

stop_valkey() {
  if [ -f "$VALKEY_PIDFILE" ]; then
    local pid
    pid=$(cat "$VALKEY_PIDFILE" 2>/dev/null || true)
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      sleep 1
      echo "   ✅ Valkey stopped"
    fi
    rm -f "$VALKEY_PIDFILE"
  fi
}

# --- Commands ---

case "$cmd" in
  start)
    echo "🚀 Starting infrastructure services..."
    start_postgres
    start_valkey || true
    echo ""
    echo "✅ Services started!"
    echo "   - Postgres: localhost:${DB_PORT}"
    echo "   - Valkey:   localhost:${VALKEY_PORT}"
    ;;

  stop)
    echo "🛑 Stopping infrastructure services..."
    stop_valkey
    stop_postgres
    echo "✅ Services stopped!"
    ;;

  reset)
    echo "🔄 Resetting infrastructure services..."
    stop_valkey
    stop_postgres
    rm -rf "$PGDATA"
    rm -rf "$VALKEY_DATA_DIR"
    echo "✅ Services reset (data removed)!"
    ;;

  logs)
    echo "=== PostgreSQL log ==="
    tail -50 "$PG_LOGFILE" 2>/dev/null || echo "(no log)"
    echo ""
    echo "=== Valkey log ==="
    tail -50 "$VALKEY_LOGFILE" 2>/dev/null || echo "(no log)"
    ;;

  *)
    echo "Usage: $0 {start|stop|reset|logs}"
    exit 1
    ;;
esac
