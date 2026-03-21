#!/usr/bin/env bash
# Common functions for dev scripts

set -euo pipefail

# Get project root (two levels up from lib/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
RUN_STATE_DIR="${TMPDIR:-/tmp}/everruns-run-$(printf '%s' "$PROJECT_ROOT" | cksum | awk '{print $1}')"

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

apply_port_prefix_defaults() {
  if [ -n "${PORT_PREFIX:-}" ]; then
    : "${PROXY_PORT:=${PORT_PREFIX}00}"
    : "${API_PORT:=${PORT_PREFIX}01}"
    : "${WORKER_GRPC_PORT:=${PORT_PREFIX}02}"
    : "${CADDY_ADMIN_PORT:=${PORT_PREFIX}03}"
    : "${UI_PORT:=${PORT_PREFIX}05}"
    : "${OTEL_GRPC_PORT:=${PORT_PREFIX}17}"
    : "${OTEL_HTTP_PORT:=${PORT_PREFIX}18}"
    : "${VALKEY_PORT:=${PORT_PREFIX}79}"
    : "${DB_PORT:=${PORT_PREFIX}32}"
  else
    : "${PROXY_PORT:=9300}"
    : "${API_PORT:=9301}"
    : "${WORKER_GRPC_PORT:=9001}"
    : "${CADDY_ADMIN_PORT:=2019}"
    : "${UI_PORT:=9305}"
    : "${OTEL_GRPC_PORT:=4317}"
    : "${OTEL_HTTP_PORT:=4318}"
    : "${VALKEY_PORT:=6379}"
    : "${DB_PORT:=9332}"
  fi

  # COMPOSE_PROJECT_NAME is still used by example.just for the Docker-based example deployment
  : "${COMPOSE_PROJECT_NAME:=everruns${PORT_PREFIX:+-$PORT_PREFIX}}"

  export PORT_PREFIX API_PORT WORKER_GRPC_PORT CADDY_ADMIN_PORT UI_PORT PROXY_PORT OTEL_GRPC_PORT OTEL_HTTP_PORT VALKEY_PORT DB_PORT COMPOSE_PROJECT_NAME RUN_STATE_DIR
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

GIT_AGENT_IDENTITY_PATTERN="(claude|cursor|copilot|github-actions|bot|ai-agent|openai|anthropic|gpt)"

git_identity_looks_agent_like() {
  local value="${1:-}"

  if [ -z "$value" ]; then
    return 1
  fi

  printf '%s\n' "$value" | grep -iEq "$GIT_AGENT_IDENTITY_PATTERN"
}

# Resolve the commit identity to use.
# Prefer existing human git config. If git config is agent-like or incomplete,
# fall back to GIT_USER_NAME/GIT_USER_EMAIL and reject agent-like values there too.
resolve_commit_git_identity() {
  local current_name current_email
  current_name="$(git config user.name 2>/dev/null || true)"
  current_email="$(git config user.email 2>/dev/null || true)"

  RESOLVED_GIT_AUTHOR_NAME=""
  RESOLVED_GIT_AUTHOR_EMAIL=""
  RESOLVED_GIT_AUTHOR_SOURCE=""

  if [ -n "$current_name" ] && [ -n "$current_email" ] && \
    ! git_identity_looks_agent_like "$current_name" && \
    ! git_identity_looks_agent_like "$current_email"; then
    RESOLVED_GIT_AUTHOR_NAME="$current_name"
    RESOLVED_GIT_AUTHOR_EMAIL="$current_email"
    RESOLVED_GIT_AUTHOR_SOURCE="git"
  else
    if [ -z "${GIT_USER_NAME:-}" ] || [ -z "${GIT_USER_EMAIL:-}" ]; then
      echo "git commit identity is missing or agent-like; set GIT_USER_NAME and GIT_USER_EMAIL to a real user before committing" >&2
      return 1
    fi

    RESOLVED_GIT_AUTHOR_NAME="$GIT_USER_NAME"
    RESOLVED_GIT_AUTHOR_EMAIL="$GIT_USER_EMAIL"
    RESOLVED_GIT_AUTHOR_SOURCE="env"
  fi

  if git_identity_looks_agent_like "$RESOLVED_GIT_AUTHOR_NAME" || \
    git_identity_looks_agent_like "$RESOLVED_GIT_AUTHOR_EMAIL"; then
    echo "resolved git commit identity looks agent-like: '$RESOLVED_GIT_AUTHOR_NAME <$RESOLVED_GIT_AUTHOR_EMAIL>'" >&2
    return 1
  fi
}

configure_commit_git_identity_if_needed() {
  resolve_commit_git_identity || return 1
  git config user.name "$RESOLVED_GIT_AUTHOR_NAME"
  git config user.email "$RESOLVED_GIT_AUTHOR_EMAIL"
}

git_outgoing_commit_range() {
  if git rev-parse --verify -q '@{upstream}' >/dev/null 2>&1; then
    printf '%s\n' '@{upstream}..HEAD'
    return 0
  fi

  if git rev-parse --verify -q 'origin/main' >/dev/null 2>&1; then
    printf '%s\n' 'origin/main..HEAD'
    return 0
  fi

  return 1
}

find_agent_like_outgoing_commit() {
  local range sha name email

  if ! range="$(git_outgoing_commit_range)"; then
    return 1
  fi

  while IFS=$'\t' read -r sha name email; do
    [ -n "$sha" ] || continue

    if git_identity_looks_agent_like "$name" || git_identity_looks_agent_like "$email"; then
      printf '%s\t%s\t%s\n' "$sha" "$name" "$email"
      return 0
    fi
  done < <(git log --format='%H%x09%an%x09%ae' "$range")

  return 1
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


# Ensure the target database exists on a running PostgreSQL instance.
# Usage: ensure_postgres_db <host> <port> <user> <dbname>
ensure_postgres_db() {
  local host="${1:-localhost}"
  local port="${2:-5432}"
  local user="${3:-everruns}"
  local db="${4:-everruns}"

  if ! command -v psql &> /dev/null; then
    return 0  # can't check, let the app fail with a clear error
  fi

  # Check if DB exists (cut first column to avoid matching the owner name)
  if psql -h "$host" -p "$port" -U "$user" -lqt 2>/dev/null | cut -d'|' -f1 | grep -qw "$db"; then
    return 0
  fi

  echo "   📦 Creating database '$db'..."
  if command -v createdb &> /dev/null; then
    createdb -h "$host" -p "$port" -U "$user" "$db" 2>/dev/null || true
  else
    psql -h "$host" -p "$port" -U "$user" -c "CREATE DATABASE \"$db\"" 2>/dev/null || true
  fi
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
