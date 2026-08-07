#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck disable=SC1091
source "$PROJECT_ROOT/scripts/lib/common.sh"

assert_eq() {
  local expected="$1"
  local actual="$2"
  local label="$3"

  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $label expected '$expected', got '$actual'" >&2
    exit 1
  fi
}

unset AUTH_MODE DEPLOYMENT_GRADE
configure_local_startup_auth
assert_eq "none" "$AUTH_MODE" "default auth mode"
assert_eq "dev" "$DEPLOYMENT_GRADE" "local deployment grade"

for service in api worker ui-proxy caddy; do
  inherited="$(SERVICE_NAME="$service" bash -c 'printf "%s:%s:%s" "$SERVICE_NAME" "$AUTH_MODE" "$DEPLOYMENT_GRADE"')"
  assert_eq "$service:none:dev" "$inherited" "$service environment"
done

for configured_mode in none admin full external; do
  AUTH_MODE="$configured_mode"
  configure_local_startup_auth
  assert_eq "$configured_mode" "$AUTH_MODE" "preserve explicit $configured_mode mode"
done

AUTH_MODE=none
verify_api_auth_mode '{"version":"test","auth_mode":"None"}' >/dev/null
if verify_api_auth_mode '{"version":"test","auth_mode":"Full"}' >/dev/null 2>&1; then
  echo "FAIL: API/UI auth disagreement was not detected" >&2
  exit 1
fi

echo "startup auth mode checks passed"
