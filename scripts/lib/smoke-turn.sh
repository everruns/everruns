# Shared steps of the self-hosted smokes (Docker Compose, everruns docker up,
# Helm): pick the data plane the images support, run one agent turn on
# LlmSim, and check which data plane ran it. Source it; the caller defines
# `fail` and `step`.

# The data plane to smoke: DATA_PLANE when set, else v2 when the shard image
# is published for $1 (it is from the release that made v2 the default), else
# v1. Older tags have no shard image, so their smokes keep checking v1.
smoke_data_plane() {
  local tag="$1"
  if [ -n "${DATA_PLANE:-}" ]; then
    echo "$DATA_PLANE"
  elif docker manifest inspect "ghcr.io/everruns/agentd:$tag" >/dev/null 2>&1; then
    echo v2
  else
    echo "::notice title=data plane::ghcr.io/everruns/agentd:$tag is not published; smoking v1" >&2
    echo v1
  fi
}

# Sign in with a password and print a personal access token. $1 base URL,
# $2 email, $3 password.
smoke_token() {
  local base="$1" email="$2" password="$3" access token
  access="$(curl -fsS -X POST "$base/api/v1/auth/login" -H "Content-Type: application/json" \
    -d "{\"email\":\"$email\",\"password\":\"$password\"}" | jq -r .access_token)" \
    || fail "sign-in failed"
  [ -n "$access" ] && [ "$access" != null ] || fail "login returned no access_token"
  token="$(curl -fsS -X POST "$base/api/v1/auth/personal-access-tokens" \
    -H "Authorization: Bearer $access" -H "Content-Type: application/json" \
    -d '{"name":"smoke"}' | jq -r .token)" || fail "could not create a personal access token"
  case "$token" in evr_pat_*) ;; *) fail "PAT does not start with evr_pat_" ;; esac
  echo "$token"
}

# Connect LlmSim, create an agent on it, start a session, send a message and
# wait for the turn. $1 base URL, $2 token, $3 a name for the agent. Prints
# the session id.
smoke_turn() {
  local base="$1" token="$2" name="$3"
  local auth=(-H "Authorization: Bearer $token" -H "Content-Type: application/json")
  # LlmSim's default model name (crates/server/src/setup/seed/models.rs).
  local model_name="llmsim-default"
  local provider_id model agent_id session_id events=""

  provider_id="$(curl -fsS "$base/api/v1/providers" "${auth[@]}" \
    | jq -r '[.data[] | select(.provider_type == "llmsim")][0].id // empty')" \
    || fail "could not list providers"
  if [ -z "$provider_id" ]; then
    provider_id="$(curl -fsS -X POST "$base/api/v1/providers" "${auth[@]}" \
      -d '{"name":"LlmSim","provider_type":"llmsim"}' | jq -r .id)" \
      || fail "could not connect the LlmSim provider"
  fi
  case "$provider_id" in provider_*) ;; *) fail "LlmSim provider id is '$provider_id'" ;; esac
  model="$(curl -fsS "$base/api/v1/models?limit=500" "${auth[@]}" \
    | jq -r --arg p "$provider_id" --arg m "$model_name" \
      '[.data[] | select(.provider_id == $p and .model_id == $m)][0].id // empty')" \
    || fail "could not list models"
  if [ -z "$model" ]; then
    model="$(curl -fsS -X POST "$base/api/v1/providers/$provider_id/models" "${auth[@]}" \
      -d "{\"model_id\":\"$model_name\",\"display_name\":\"LlmSim Default\",\"capabilities\":[\"chat\",\"tools\"],\"enabled\":true}" \
      | jq -r .id)" || fail "could not create the LlmSim model"
  fi
  case "$model" in model_*) ;; *) fail "LlmSim model id is '$model'" ;; esac
  curl -fsS -X PATCH "$base/api/v1/models/$model" "${auth[@]}" -d '{"enabled":true}' >/dev/null \
    || fail "could not enable the LlmSim model"
  agent_id="$(curl -fsS -X POST "$base/api/v1/agents" "${auth[@]}" \
    -d "{\"name\":\"$name\",\"system_prompt\":\"You are helpful.\",\"default_model_id\":\"$model\"}" | jq -r .id)"
  case "$agent_id" in agent_*) ;; *) fail "agent create returned '$agent_id'" ;; esac

  session_id="$(curl -fsS -X POST "$base/api/v1/sessions" "${auth[@]}" \
    -d "{\"agent_id\": \"$agent_id\"}" | jq -r .id)"
  case "$session_id" in session_*) ;; *) fail "session create returned '$session_id'" ;; esac
  curl -fsS -X POST "$base/api/v1/sessions/$session_id/messages" "${auth[@]}" \
    -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]}}' >/dev/null \
    || fail "send message failed"

  for _ in $(seq 1 60); do
    events="$(curl -fsS "$base/api/v1/sessions/$session_id/events?limit=100" "${auth[@]}" | jq -r '.data[].type')"
    grep -qx turn.completed <<<"$events" && break
    grep -qx turn.failed <<<"$events" && fail "turn failed: $(tr '\n' ' ' <<<"$events")"
    sleep 2
  done
  grep -qx turn.completed <<<"$events" || fail "no turn.completed; events: $(tr '\n' ' ' <<<"$events")"
  grep -qx output.message.completed <<<"$events" || fail "turn completed without an assistant message"
  echo "$session_id"
}

# Check the data plane that ran the turns: $1 v1 or v2, then a command that
# runs `psql` against the install's database and takes SQL as its last
# argument. A v2 organization's sessions live in the shard, never in the
# server's `sessions` table; a v1 install places no organization.
smoke_check_plane() {
  local plane="$1"; shift
  local placed sessions
  placed="$("$@" "SELECT count(*) FROM dp_orgs" | tr -d '[:space:]')" || fail "could not read dp_orgs"
  sessions="$("$@" "SELECT count(*) FROM sessions" | tr -d '[:space:]')" || fail "could not read sessions"
  case "$plane" in
    v2)
      [ "${placed:-0}" -ge 1 ] || fail "v2: no organization was placed on the data plane"
      [ "${sessions:-0}" = 0 ] || fail "v2: $sessions session(s) ran on the workers (v1), not on the shard"
      ;;
    v1)
      [ "${placed:-0}" = 0 ] || fail "v1: $placed organization(s) were placed on a data plane"
      [ "${sessions:-0}" -ge 1 ] || fail "v1: the session is not in the server's sessions table"
      ;;
  esac
  echo "ok: the turn ran on $plane (placed orgs: $placed, server sessions: $sessions)"
}
