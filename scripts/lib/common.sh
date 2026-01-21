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
# Example: check_npm_deps "UI" "just ui-install"
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

# Backward-compatible wrappers
check_ui_deps() {
  check_npm_deps "UI" "just ui-install"
}

check_docs_deps() {
  check_npm_deps "Docs" "just docs-install"
}
