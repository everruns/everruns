#!/usr/bin/env bash
# Smoke for agentd on its own (infra/agentd/compose.yaml): start the install
# with an API key, push a model key bound to a stand-in OpenAI-compatible
# model and an agent, run a turn, read it back from the stream, then restart
# the container and check the agent and the session are still there.
#
# Usage: AGENTD_TAG=latest scripts/smoke-agentd-standalone.sh
# A tag published before `agentd serve` existed is skipped, not failed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${AGENTD_TAG:-latest}"
IMAGE="ghcr.io/everruns/agentd:$TAG"
PORT="${AGENTD_PORT:-17700}"
WORK="$(mktemp -d)"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-agentd-smoke}"
fail() { echo "::error title=agentd standalone smoke::$*" >&2; exit 1; }

docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull -q "$IMAGE" >/dev/null
if ! docker run --rm "$IMAGE" serve --help >/dev/null 2>&1; then
  echo "::notice title=agentd standalone smoke::$IMAGE has no \`serve\`; skipped"
  exit 0
fi

SECRET="$(openssl rand -hex 24)"
mkdir -p "$WORK/keys"
printf 'smoke:%s\n' "$SECRET" > "$WORK/keys/api-keys"
chmod 0644 "$WORK/keys/api-keys"

# The stand-in model: answers every chat completion with "pong" and records
# the authorization header it was sent.
cat > "$WORK/model.py" <<'PY'
import http.server, json
SEEN = []
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        SEEN.append(self.headers.get("authorization", ""))
        chunks = [
            {"id": "1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "pong"}, "finish_reason": None}]},
            {"id": "1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
             "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}},
        ]
        body = "".join(f"data: {json.dumps(c)}\n\n" for c in chunks) + "data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body.encode())
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(json.dumps(SEEN).encode())
http.server.ThreadingHTTPServer(("0.0.0.0", 8080), H).serve_forever()
PY
cat > "$WORK/override.yaml" <<YAML
services:
  model:
    image: python:3.13-alpine
    command: ["python", "-u", "/model.py"]
    volumes:
      - $WORK/model.py:/model.py:ro
YAML

compose() {
  AGENTD_TAG="$TAG" AGENTD_PORT="$PORT" AGENTD_KEYS_DIR="$WORK/keys" AGENTD_GROUP_LABEL=org \
    docker compose -f "$ROOT/infra/agentd/compose.yaml" -f "$WORK/override.yaml" "$@"
}
cleanup() {
  if [ "${KEEP:-}" != 1 ]; then
    compose logs agentd --tail 80 || true
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

BASE="http://127.0.0.1:$PORT"
api() { # method path [body] -> body; fails on a non-2xx status
  local method="$1" path="$2" body="${3:-}"
  local args=(-sS -X "$method" -H "Authorization: Bearer $SECRET" -H 'content-type: application/json' --max-time 90)
  [ -n "$body" ] && args+=(--data "$body")
  curl -f "${args[@]}" "$BASE$path"
}
status() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer ${2:-}" "$BASE$1"; }
until_ok() { # method path body: retry while the shard elects
  for _ in $(seq 1 120); do
    if out="$(api "$@" 2>/dev/null)"; then echo "$out"; return 0; fi
    sleep 1
  done
  fail "$1 $2 never succeeded"
}

echo "==> up ($IMAGE)"
compose up -d agentd model
for _ in $(seq 1 60); do curl -fsS "$BASE/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "$BASE/health" >/dev/null || fail "no /health"

echo "==> auth"
[ "$(status /v1/agents)" = 401 ] || fail "anonymous call not refused"
[ "$(status /v1/agents not-the-key-not-the-key-not-the-key)" = 401 ] || fail "wrong key not refused"

echo "==> push a model key and an agent"
until_ok PUT /v1/secrets/model-key \
  '{"value":"key-of-the-smoke","bind":{"model":"Standin","provider_type":"openai","base_url":"http://model:8080/v1","api":"chat_completions"}}' >/dev/null
until_ok PUT /v1/agents/helper \
  '{"name":"helper","instructions":"Answer.","model":{"provider":"Standin","model":"standin-large"}}' | grep -q '"revision":1' \
  || fail "agent not created at revision 1"

echo "==> a session and a turn"
session="$(until_ok POST /v1/sessions '{"agent_id":"helper","labels":{"org":"smoke"}}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
reply="$(api POST "/v1/sessions/$session/messages?wait=true" '{"text":"ping"}')"
grep -q '"status":"completed"' <<<"$reply" || fail "turn did not complete: $reply"
grep -q pong <<<"$reply" || fail "no reply from the model: $reply"
seen="$(compose exec -T model wget -qO- http://127.0.0.1:8080/)"
grep -q 'Bearer key-of-the-smoke' <<<"$seen" || fail "the model was not sent the pushed key: $seen"

echo "==> the stream"
found=""
cursor=""
for _ in $(seq 1 20); do
  page="$(api GET "/v1/stream?label=org=smoke&wait=5&after=$cursor")"
  if grep -q '"type":"Usage"' <<<"$page"; then found=1; break; fi
  cursor="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["cursor"])' <<<"$page")"
done
[ -n "$found" ] || fail "no Usage record on the stream"

echo "==> restart keeps agents and sessions"
compose restart agentd
for _ in $(seq 1 60); do curl -fsS "$BASE/health" >/dev/null 2>&1 && break; sleep 1; done
until_ok GET /v1/agents/helper '' | grep -q '"revision":1' || fail "agent lost on restart"
until_ok GET "/v1/sessions?label=org=smoke" '' | grep -q "$session" || fail "session lost on restart"

echo "agentd standalone smoke passed"
