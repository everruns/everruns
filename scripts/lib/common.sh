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

# Check if UI dependencies need to be installed/updated
check_ui_deps() {
  local ui_dir="$PROJECT_ROOT/apps/ui"

  if [ ! -d "$ui_dir/node_modules" ]; then
    echo "   ⚠️  UI dependencies not installed. Run: just ui-install"
    return 1
  fi

  if [ -f "$ui_dir/package-lock.json" ] && [ -f "$ui_dir/node_modules/.package-lock.json" ]; then
    if [ "$ui_dir/package-lock.json" -nt "$ui_dir/node_modules/.package-lock.json" ]; then
      echo "   ⚠️  UI dependencies outdated. Run: just ui-install"
      return 1
    fi
  fi

  return 0
}

# Check if docs dependencies need to be installed/updated
check_docs_deps() {
  local docs_dir="$PROJECT_ROOT/apps/docs"

  if [ ! -d "$docs_dir/node_modules" ]; then
    echo "   ⚠️  Docs dependencies not installed. Run: just docs-install"
    return 1
  fi

  if [ -f "$docs_dir/package-lock.json" ] && [ -f "$docs_dir/node_modules/.package-lock.json" ]; then
    if [ "$docs_dir/package-lock.json" -nt "$docs_dir/node_modules/.package-lock.json" ]; then
      echo "   ⚠️  Docs dependencies outdated. Run: just docs-install"
      return 1
    fi
  fi

  return 0
}
