#!/usr/bin/env bash
# Run the Docker Compose quickstart (docs/getting-started/docker-compose.md)
# end to end and fail on the first step a reader would get stuck on.
#
# Steps mirror the guide: generate secrets, write .env, pull, up, sign in,
# create a personal access token, create an agent, start a session, send a
# message, and wait for the turn to complete. The agent uses the seeded LlmSim
# model so no LLM API key is needed. LlmSim is seeded disabled, so the smoke
# enables it through the API first.
#
# Decision: this runs published images (EVERRUNS_TAG, default `latest`), not
# PR-built binaries. CI binaries are linked against the runner's newer glibc
# and cannot run in the distroless base (see docker/Dockerfile.prebuilt), so
# the smoke checks the compose file and guide against what a reader pulls.
#
# Usage: scripts/smoke-docker-compose.sh [compose-file]
#   EVERRUNS_TAG=v0.34.0 scripts/smoke-docker-compose.sh
#   KEEP_STACK=1 leaves the stack running for inspection.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_SRC="${1:-$ROOT/examples/docker-compose-full.yaml}"
PORT="${EXAMPLE_PROXY_PORT:-9300}"
BASE="http://localhost:$PORT"
# Seeded LlmSim model id (crates/server/src/seed/models.rs, LLMSIM_DEFAULT).
LLMSIM_MODEL="model_01933b5a000070008000000000000401"

WORKDIR="$(mktemp -d)"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-everruns-smoke}"

step() { printf '\n==> %s\n' "$*"; }
fail() {
  echo "::error title=Compose smoke::$*" >&2
  docker compose ps -a >&2 || true
  docker compose logs --tail 80 server worker-1 >&2 || true
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

step "Generate secrets and write .env (guide steps 2-3)"
{
  echo "SECRETS_ENCRYPTION_KEY=$(python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())")"
  echo "AUTH_JWT_SECRET=$(openssl rand -hex 32)"
  echo "WORKER_GRPC_AUTH_TOKEN=$(openssl rand -hex 32)"
  echo "AUTH_ADMIN_EMAIL=admin@example.com"
  echo "AUTH_ADMIN_PASSWORD=$(openssl rand -hex 16)"
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
ui_code="$(curl -sS -o /dev/null -w '%{http_code}' "$BASE/")"
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

step "Sign in and create a personal access token"
access_token="$(curl -fsS -X POST "$BASE/api/v1/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"$AUTH_ADMIN_EMAIL\",\"password\":\"$AUTH_ADMIN_PASSWORD\"}" | jq -r .access_token)"
[ -n "$access_token" ] && [ "$access_token" != null ] || fail "login returned no access_token"
EVERRUNS_TOKEN="$(curl -fsS -X POST "$BASE/api/v1/auth/personal-access-tokens" \
  -H "Authorization: Bearer $access_token" \
  -H "Content-Type: application/json" \
  -d '{"name":"compose-smoke"}' | jq -r .token)"
case "$EVERRUNS_TOKEN" in evr_pat_*) ;; *) fail "PAT does not start with evr_pat_" ;; esac
auth=(-H "Authorization: Bearer $EVERRUNS_TOKEN" -H "Content-Type: application/json")

step "Create an agent on the LlmSim model"
curl -fsS -X PATCH "$BASE/api/v1/models/$LLMSIM_MODEL" "${auth[@]}" -d '{"enabled":true}' >/dev/null \
  || fail "could not enable the LlmSim model"
agent_id="$(curl -fsS -X POST "$BASE/api/v1/agents" "${auth[@]}" \
  -d "{\"name\":\"compose-smoke\",\"system_prompt\":\"You are helpful.\",\"default_model_id\":\"$LLMSIM_MODEL\"}" | jq -r .id)"
case "$agent_id" in agent_*) ;; *) fail "agent create returned '$agent_id'" ;; esac

step "Start a session and send a message (guide: Start a Session)"
session_id="$(curl -fsS -X POST "$BASE/api/v1/sessions" "${auth[@]}" \
  -d "{\"agent_id\": \"$agent_id\"}" | jq -r .id)"
case "$session_id" in session_*) ;; *) fail "session create returned '$session_id'" ;; esac
curl -fsS -X POST "$BASE/api/v1/sessions/$session_id/messages" "${auth[@]}" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]}}' >/dev/null \
  || fail "send message failed"

step "Wait for a worker to complete the turn"
events=""
for _ in $(seq 1 60); do
  events="$(curl -fsS "$BASE/api/v1/sessions/$session_id/events?limit=100" "${auth[@]}" | jq -r '.data[].type')"
  grep -qx turn.completed <<<"$events" && break
  grep -qx turn.failed <<<"$events" && fail "turn failed: $(tr '\n' ' ' <<<"$events")"
  sleep 2
done
grep -qx turn.completed <<<"$events" || fail "no turn.completed; events: $(tr '\n' ' ' <<<"$events")"
grep -qx output.message.completed <<<"$events" || fail "turn completed without an assistant message"

step "Compose quickstart smoke passed"
