#!/usr/bin/env bash
# SeaweedFS: local S3-compatible object storage for the optional blob backend
# (knowledge/runtime-resources/object-storage.md).
#
# Runs the `weed` binary as a native process — no Docker — mirroring how
# infra.sh runs PostgreSQL/Valkey/NATS. SeaweedFS speaks the S3 API, so the
# server uses the same S3 client code path as against AWS S3.
#
# This is opt-in (not part of `just start-all`); enable the backend with:
#   STORAGE_BLOB_BACKEND=s3
#   STORAGE_S3_BUCKET=everruns-dev
#   STORAGE_S3_ENDPOINT=http://127.0.0.1:8333
#   STORAGE_S3_REGION=us-east-1
#   STORAGE_S3_ACCESS_KEY_ID=everruns
#   STORAGE_S3_SECRET_ACCESS_KEY=everruns-secret
#   STORAGE_S3_ALLOW_HTTP=true

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cmd="${1:-start}"

# Data persists under $PROJECT_ROOT/.local/data/ (gitignored), isolated per repo.
LOCAL_DATA_DIR="${PROJECT_ROOT}/.local/data"
SEAWEEDFS_S3_PORT="${SEAWEEDFS_S3_PORT:-8333}"
SEAWEEDFS_MASTER_PORT="${SEAWEEDFS_MASTER_PORT:-9333}"
SEAWEEDFS_DATA_DIR="${LOCAL_DATA_DIR}/seaweedfs-${SEAWEEDFS_S3_PORT}"
SEAWEEDFS_PIDFILE="${SEAWEEDFS_DATA_DIR}/seaweedfs.pid"
SEAWEEDFS_LOGFILE="${SEAWEEDFS_DATA_DIR}/seaweedfs.log"
SEAWEEDFS_S3_CONFIG="${PROJECT_ROOT}/local/seaweedfs-s3-config.json"
SEAWEEDFS_BUCKET="${STORAGE_S3_BUCKET:-everruns-dev}"

detect_seaweedfs_bin() {
  if command -v weed &> /dev/null; then
    SEAWEED="weed"
    return 0
  fi
  echo "❌ 'weed' (SeaweedFS) not found."
  echo "   Install from https://github.com/seaweedfs/seaweedfs/releases"
  echo "   or: brew install seaweedfs"
  return 1
}

seaweedfs_is_running() {
  if [ -f "$SEAWEEDFS_PIDFILE" ]; then
    local pid
    pid=$(cat "$SEAWEEDFS_PIDFILE" 2>/dev/null || true)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
    return $?
  fi
  return 1
}

port_open() {
  (exec 3<>"/dev/tcp/localhost/$1") 2>/dev/null && exec 3>&- && return 0
  return 1
}

create_bucket() {
  # Idempotent: a no-op if the bucket already exists.
  for _ in {1..10}; do
    if printf 's3.bucket.create -name %s\n' "$SEAWEEDFS_BUCKET" \
      | "$SEAWEED" shell -master "localhost:${SEAWEEDFS_MASTER_PORT}" >/dev/null 2>&1; then
      echo "   ✅ bucket '${SEAWEEDFS_BUCKET}' ready"
      return 0
    fi
    sleep 1
  done
  echo "   ⚠️  could not create bucket '${SEAWEEDFS_BUCKET}' (see $SEAWEEDFS_LOGFILE)"
  return 1
}

start_seaweedfs() {
  detect_seaweedfs_bin || return 1

  if seaweedfs_is_running; then
    echo "   ✅ SeaweedFS already running (S3 port ${SEAWEEDFS_S3_PORT})"
    create_bucket
    return 0
  fi

  echo "   Starting SeaweedFS (S3 :${SEAWEEDFS_S3_PORT}, master :${SEAWEEDFS_MASTER_PORT})..."
  mkdir -p "$SEAWEEDFS_DATA_DIR"
  # All-in-one server (master + volume + filer) with the S3 gateway enabled.
  "$SEAWEED" server \
    -dir="$SEAWEEDFS_DATA_DIR" \
    -master.port="$SEAWEEDFS_MASTER_PORT" \
    -s3 \
    -s3.port="$SEAWEEDFS_S3_PORT" \
    -s3.config="$SEAWEEDFS_S3_CONFIG" \
    > "$SEAWEEDFS_LOGFILE" 2>&1 &
  echo $! > "$SEAWEEDFS_PIDFILE"
  disown

  for _ in {1..20}; do
    if ! seaweedfs_is_running; then
      echo "   ❌ SeaweedFS exited during startup (see $SEAWEEDFS_LOGFILE)"
      return 1
    fi
    if port_open "$SEAWEEDFS_S3_PORT"; then
      echo "   ✅ SeaweedFS started (S3 http://localhost:${SEAWEEDFS_S3_PORT})"
      create_bucket
      return 0
    fi
    sleep 1
  done

  echo "   ⚠️  SeaweedFS S3 port did not open (see $SEAWEEDFS_LOGFILE)"
  return 1
}

stop_seaweedfs() {
  if [ -f "$SEAWEEDFS_PIDFILE" ]; then
    local pid
    pid=$(cat "$SEAWEEDFS_PIDFILE" 2>/dev/null || true)
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      sleep 1
      echo "   ✅ SeaweedFS stopped"
    fi
    rm -f "$SEAWEEDFS_PIDFILE"
  fi
}

case "$cmd" in
  start)
    echo "🚀 Starting SeaweedFS object storage..."
    start_seaweedfs
    ;;
  stop)
    echo "🛑 Stopping SeaweedFS..."
    stop_seaweedfs
    ;;
  reset)
    echo "🔄 Resetting SeaweedFS..."
    stop_seaweedfs
    # Safety: only delete data dirs under the repo's .local/ directory.
    case "$SEAWEEDFS_DATA_DIR" in
      "$LOCAL_DATA_DIR"*) rm -rf "$SEAWEEDFS_DATA_DIR" ;;
      *) echo "   ⚠️  $SEAWEEDFS_DATA_DIR is outside $LOCAL_DATA_DIR — skipping removal" ;;
    esac
    echo "✅ SeaweedFS reset (data removed)!"
    ;;
  bucket)
    detect_seaweedfs_bin && create_bucket
    ;;
  logs)
    tail -50 "$SEAWEEDFS_LOGFILE" 2>/dev/null || echo "(no log)"
    ;;
  *)
    echo "Usage: $0 {start|stop|reset|bucket|logs}"
    exit 1
    ;;
esac
