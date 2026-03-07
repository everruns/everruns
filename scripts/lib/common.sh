#!/usr/bin/env bash
# Common functions for dev scripts

set -euo pipefail

# Get project root (two levels up from lib/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"

cd "$PROJECT_ROOT"

# Load .env file if it exists
if [ -f .env ]; then
  set -a
  source .env
  set +a
fi

# Auto-activate sccache if installed (optional compile cache, no-op if missing)
if [ -z "${RUSTC_WRAPPER:-}" ] && [ -f "$SCRIPT_DIR/sccache.sh" ]; then
  # shellcheck disable=SC1091
  source "$SCRIPT_DIR/sccache.sh"
  activate_sccache 2>/dev/null || true
fi

# Check if a command is available, exit with hint if not
require_command() {
  local cmd="$1"
  local hint="$2"

  if ! command -v "$cmd" &> /dev/null; then
    echo "❌ $cmd not installed. $hint"
    exit 1
  fi
}

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

  echo "❌ Docker Compose not found. Install Docker Desktop/Colima or the docker-compose plugin."
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

# Check if npm dependencies need to be installed/updated
# Usage: check_npm_deps <app_name> <install_command>
# Example: check_npm_deps "UI" "cd apps/ui && npm install"
check_npm_deps() {
  local app_name="$1"
  local install_cmd="$2"
  local app_name_lower
  app_name_lower=$(echo "$app_name" | tr '[:upper:]' '[:lower:]')
  local app_dir="$PROJECT_ROOT/apps/$app_name_lower"

  if [ ! -d "$app_dir/node_modules" ]; then
    echo "   ⚠️  $app_name dependencies not installed. Run: $install_cmd"
    return 1
  fi

  if [ -f "$app_dir/package-lock.json" ] && [ -f "$app_dir/node_modules/.package-lock.json" ]; then
    if [ "$app_dir/package-lock.json" -nt "$app_dir/node_modules/.package-lock.json" ]; then
      echo "   ⚠️  $app_name dependencies outdated. Run: $install_cmd"
      return 1
    fi
  fi

  return 0
}

# Check if something is listening on a port (no external tools needed)
check_port_open() {
  local host="${1:-localhost}"
  local port="${2:-5432}"
  (echo > "/dev/tcp/$host/$port") 2>/dev/null
}

# Check if PostgreSQL is actually responding (not just port open)
# Uses pg_isready if available, falls back to psql connection test
check_postgres_ready() {
  local host="${1:-localhost}"
  local port="${2:-5432}"
  local user="${3:-postgres}"

  # First check if port is open
  if ! check_port_open "$host" "$port"; then
    return 1
  fi

  # Try pg_isready if available (preferred)
  if command -v pg_isready &> /dev/null; then
    pg_isready -h "$host" -p "$port" -U "$user" -q -t 2 2>/dev/null
    return $?
  fi

  # Fall back to psql connection test
  if command -v psql &> /dev/null; then
    PGCONNECT_TIMEOUT=2 psql -h "$host" -p "$port" -U "$user" -c "SELECT 1" &>/dev/null
    return $?
  fi

  # No pg_isready or psql available — probe the PostgreSQL wire protocol.
  # Send a minimal SSLRequest packet and check for a valid response (S or N).
  # This catches stale port forwards (e.g., Colima SSH mux) that accept TCP
  # but have no PostgreSQL behind them.
  local response
  response=$(printf '\x00\x00\x00\x08\x04\xd2\x16\x2f' | \
    nc -w 2 "$host" "$port" 2>/dev/null | head -c1 | od -An -tx1 | tr -d ' ')
  # PostgreSQL responds with 'S' (0x53) or 'N' (0x4e) to SSLRequest
  if [ "$response" = "53" ] || [ "$response" = "4e" ]; then
    return 0
  fi
  return 1
}


print_doppler_secret_hint() {
  local missing=()

  [ -n "${OPENAI_API_KEY:-}" ] || missing+=("OPENAI_API_KEY")
  [ -n "${ANTHROPIC_API_KEY:-}" ] || missing+=("ANTHROPIC_API_KEY")

  if [ ${#missing[@]} -eq 0 ]; then
    return 0
  fi

  if command -v doppler &> /dev/null; then
    echo "   ⚠️  Missing env: ${missing[*]}"
    echo "   ℹ️  Cloud agents use Doppler for secrets."
    echo "   ℹ️  Re-run with: doppler run -- <command>"
    echo "   ℹ️  Quickcheck: doppler run -- env | rg 'OPENAI_API_KEY|ANTHROPIC_API_KEY'"
  fi
}

# Backward-compatible wrappers
check_ui_deps() {
  check_npm_deps "UI" "cd apps/ui && npm install"
}

check_docs_deps() {
  check_npm_deps "Docs" "cd apps/docs && npm install"
}
