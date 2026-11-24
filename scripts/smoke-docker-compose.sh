#!/usr/bin/env bash
# Run the Docker Compose quickstart (docs/getting-started/docker-compose.md)
# end to end and fail on the first step a reader would get stuck on.
#
# Steps mirror the guide: generate secrets, write .env, pull, up, sign in,
# create a personal access token, create an agent, start a session, send a
# message, wait for the turn to complete, and check which data plane ran it
# (v2, shards, by default; v1 for image tags published before the agentd image). The agent runs on LlmSim so no
# LLM API key is needed. Startup no longer creates provider rows for built-in
# drivers (only connected providers are listed), so the smoke connects LlmSim
# through the API and enables an `llmsim-default` model under it; images that
# still seed the provider and its disabled model are handled the same way.
#
# Decision: this runs published images (EVERRUNS_TAG, default `latest`), not
# PR-built binaries. CI binaries are linked against the runner's newer glibc
# and cannot run in the distroless base (see docker/Dockerfile.prebuilt), so
# the smoke checks the compose file and guide against what a reader pulls.
#
# Usage: scripts/smoke-docker-compose.sh [compose-file]
#   EVERRUNS_TAG=v0.34.0 scripts/smoke-docker-compose.sh
#   KEEP_STACK=1 leaves the stack running for inspection.
#   SMOKE_PROFILES= (empty) runs the lean default set instead of the guide's
#   `scale,metrics` profiles.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_SRC="${1:-$ROOT/infra/docker-compose/docker-compose.yaml}"
PORT="${EXAMPLE_PROXY_PORT:-9300}"
BASE="http://localhost:$PORT"
# shellcheck source=lib/smoke-turn.sh
. "$ROOT/scripts/lib/smoke-turn.sh"

WORKDIR="$(mktemp -d)"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-everruns-smoke}"

step() { printf '\n==> %s\n' "$*"; }
fail() {
  echo "::error title=Compose smoke::$*" >&2
  docker compose ps -a >&2 || true
  docker compose logs --tail 80 server worker-1 agentd executor edge >&2 || true
  exit 1
}
cleanup() {
  if [ "${KEEP_STACK:-}" != "1" ]; then
    (cd "$WORKDIR" && docker compose down -v --remove-orphans >/dev/null 2>&1) || true
    rm -rf "$WORKDIR"
  else
    echo "Stack left running in $WORKDIR"
  fi
}
trap cleanup EXIT

cd "$WORKDIR"
cp "$COMPOSE_SRC" docker-compose.yaml

DATA_PLANE="$(smoke_data_plane "${EVERRUNS_TAG:-latest}")"

step "Generate secrets and write .env (guide steps 2-3, data plane $DATA_PLANE)"
{
  echo "SECRETS_ENCRYPTION_KEY=$(python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())")"
  echo "AUTH_JWT_SECRET=$(openssl rand -hex 32)"
  echo "WORKER_GRPC_AUTH_TOKEN=$(openssl rand -hex 32)"
  echo "AUTH_ADMIN_EMAIL=admin@example.com"
  echo "AUTH_ADMIN_PASSWORD=$(openssl rand -hex 16)"
  echo "EVERRUNS_DP_TOKEN=$(openssl rand -hex 32)"
  echo "EVERRUNS_DP_KEY=$(openssl rand -base64 32)"
  if [ "$DATA_PLANE" = v1 ]; then
    echo "EVERRUNS_DATA_PLANE=v1"
    echo "DATA_PLANE_REPLICAS=0"
    echo "DATA_PLANE_PULL_POLICY=never"
  fi
  echo "COMPOSE_PROFILES=${SMOKE_PROFILES-scale,metrics}"
} > .env
# shellcheck disable=SC1091
. ./.env

step "Pull and start services (guide step 4)"
docker compose pull --quiet
docker compose up -d

step "Wait for /health through the proxy"
for _ in $(seq 1 90); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then break; fi
  sleep 2
done
curl -fsS "$BASE/health" || fail "GET $BASE/health never succeeded"
echo
# The UI container starts alongside the server, so /health can pass while
# Caddy still answers 502 for the UI; poll instead of checking once.
for _ in $(seq 1 60); do
  ui_code="$(curl -sS -o /dev/null -w '%{http_code}' "$BASE/" || true)"
  case "$ui_code" in 2??|3??) break ;; esac
  sleep 2
done
case "$ui_code" in 2??|3??) ;; *) fail "UI returned HTTP $ui_code" ;; esac

step "Check container healthchecks"
server_image="$(docker compose config --images | grep everruns-server | head -1)"
probe_out="$(docker run --rm --entrypoint /app/everruns-server "$server_image" --health-check 2>&1 || true)"
if grep -q "unexpected argument" <<<"$probe_out"; then
  # Images released before --health-check existed always report unhealthy.
  echo "::warning title=Compose smoke::$server_image predates --health-check; skipping health status check"
else
  status=""
  for _ in $(seq 1 30); do
    status="$(docker inspect --format '{{.State.Health.Status}}' "$(docker compose ps -q server)")"
    [ "$status" = "healthy" ] && break
    sleep 2
  done
  [ "$status" = "healthy" ] || fail "server container health is '$status'"
fi
echo "ok"

step "Check the selected profiles"
running="$(docker compose ps --services --status running)"
case ",$COMPOSE_PROFILES," in
  *,scale,*) grep -qx worker-3 <<<"$running" || fail "scale profile set but worker-3 is not running" ;;
  *) ! grep -qx worker-3 <<<"$running" || fail "worker-3 is running without the scale profile" ;;
esac
case ",$COMPOSE_PROFILES," in
  *,metrics,*) grep -qx victoriametrics <<<"$running" || fail "metrics profile set but victoriametrics is not running" ;;
  *) ! grep -qx victoriametrics <<<"$running" || fail "victoriametrics is running without the metrics profile" ;;
esac
echo "ok (${COMPOSE_PROFILES:-default set})"

step "Sign in and create a personal access token"
management_api="$(smoke_management_api_url "$BASE")" || fail "could not select the management API"
access_token="$(curl -fsS -X POST "$management_api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"$AUTH_ADMIN_EMAIL\",\"password\":\"$AUTH_ADMIN_PASSWORD\"}" | jq -r .access_token)"
[ -n "$access_token" ] && [ "$access_token" != null ] || fail "login returned no access_token"
EVERRUNS_TOKEN="$(curl -fsS -X POST "$management_api/auth/personal-access-tokens" \
  -H "Authorization: Bearer $access_token" \
  -H "Content-Type: application/json" \
  -d '{"name":"compose-smoke"}' | jq -r .token)"
case "$EVERRUNS_TOKEN" in evr_pat_*) ;; *) fail "PAT does not start with evr_pat_" ;; esac

step "Run a turn on LlmSim (guide: Start a Session)"
smoke_turn "$BASE" "$EVERRUNS_TOKEN" compose-smoke >/dev/null

step "Check the data plane that ran it ($DATA_PLANE)"
smoke_check_plane "$DATA_PLANE" docker compose exec -T postgres psql -U everruns -d everruns -tAc

step "Compose quickstart smoke passed"
