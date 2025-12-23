#!/usr/bin/env bash
set -euo pipefail

# Seed agents from YAML configuration into the local development database
# Usage: ./scripts/seed-agents.sh [--api-url URL]
#
# Requires mikefarah/yq (Go version): https://github.com/mikefarah/yq

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SEED_FILE="$PROJECT_ROOT/harness/seed-agents.yaml"

API_URL="${API_URL:-http://localhost:9000}"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --api-url)
      API_URL="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1"
      exit 1
      ;;
  esac
done

# Check for required tools
check_tools() {
  if ! command -v curl &> /dev/null; then
    echo "❌ curl is required but not installed"
    exit 1
  fi
  if ! command -v jq &> /dev/null; then
    echo "❌ jq is required but not installed"
    echo "   Install with: apt-get install jq (or brew install jq)"
    exit 1
  fi
  if ! command -v yq &> /dev/null; then
    echo "❌ yq is required but not installed"
    echo "   Install mikefarah/yq (Go version):"
    echo "     brew install yq"
    echo "     go install github.com/mikefarah/yq/v4@latest"
    echo "     or download from: https://github.com/mikefarah/yq/releases"
    exit 1
  fi

  # Detect yq variant and require mikefarah/yq
  local version_output
  version_output=$(yq --version 2>&1 || true)

  if echo "$version_output" | grep -qi "mikefarah\|https://github.com/mikefarah/yq"; then
    echo "   Using mikefarah/yq (Go)"
  elif echo "$version_output" | grep -qE "version v?[0-9]+\.[0-9]+"; then
    echo "   Using mikefarah/yq (Go)"
  else
    echo "❌ Wrong yq version installed!"
    echo ""
    echo "   Found: kislyuk/yq (Python wrapper)"
    echo "   Required: mikefarah/yq (Go version)"
    echo ""
    echo "   To fix, uninstall Python yq and install Go yq:"
    echo "     pip uninstall yq"
    echo "     brew install yq"
    echo ""
    echo "   Or install Go yq with a different name and set YQ_PATH:"
    echo "     go install github.com/mikefarah/yq/v4@latest"
    echo ""
    echo "   Download: https://github.com/mikefarah/yq/releases"
    exit 1
  fi
}

# Wait for API to be healthy
wait_for_api() {
  local max_attempts=30
  local attempt=0

  echo "⏳ Waiting for API to be healthy at $API_URL..."

  while [[ $attempt -lt $max_attempts ]]; do
    if curl -s "$API_URL/health" > /dev/null 2>&1; then
      echo "   ✅ API is healthy"
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done

  echo "❌ API not healthy after $max_attempts seconds"
  exit 1
}

# Get existing providers and return their names
get_existing_provider_names() {
  curl -s "$API_URL/v1/llm-providers" | jq -r '.data[].name'
}

# Check if provider with name already exists
provider_exists() {
  local name="$1"
  local existing_names
  existing_names=$(get_existing_provider_names)

  echo "$existing_names" | grep -Fxq "$name"
}

# Get provider ID by name
get_provider_id_by_name() {
  local name="$1"
  curl -s "$API_URL/v1/llm-providers" | jq -r ".data[] | select(.name == \"$name\") | .id"
}

# Get existing model IDs for a provider
get_existing_model_ids() {
  local provider_id="$1"
  curl -s "$API_URL/v1/llm-providers/$provider_id/models" | jq -r '.[].model_id'
}

# Create a provider from JSON payload
create_provider() {
  local payload="$1"
  local response

  response=$(curl -s -X POST "$API_URL/v1/llm-providers" \
    -H "Content-Type: application/json" \
    -d "$payload")

  echo "$response"
}

# Create a model under a provider
create_model() {
  local provider_id="$1"
  local payload="$2"
  local response

  response=$(curl -s -X POST "$API_URL/v1/llm-providers/$provider_id/models" \
    -H "Content-Type: application/json" \
    -d "$payload")

  echo "$response"
}

# Get existing agents and return their names as JSON array
get_existing_agent_names() {
  curl -s "$API_URL/v1/agents" | jq -r '.items[].name'
}

# Check if agent with name already exists
agent_exists() {
  local name="$1"
  local existing_names
  existing_names=$(get_existing_agent_names)

  echo "$existing_names" | grep -Fxq "$name"
}

# Create an agent from JSON payload (uses idempotent PUT endpoint)
# Returns: "STATUS_CODE|RESPONSE_BODY"
create_agent() {
  local payload="$1"
  local http_code
  local response

  # Get both HTTP status code and response body
  response=$(curl -s -w "\n%{http_code}" -X PUT "$API_URL/v1/agents" \
    -H "Content-Type: application/json" \
    -d "$payload")

  # Extract status code (last line) and body (everything else)
  http_code=$(echo "$response" | tail -n1)
  response=$(echo "$response" | sed '$d')

  echo "${http_code}|${response}"
}

# Set capabilities for an agent
set_agent_capabilities() {
  local agent_id="$1"
  local capabilities="$2"

  if [[ -n "$capabilities" && "$capabilities" != "null" && "$capabilities" != "[]" ]]; then
    curl -s -X PUT "$API_URL/v1/agents/$agent_id/capabilities" \
      -H "Content-Type: application/json" \
      -d "$capabilities" > /dev/null
  fi
}

# Seed providers from YAML file
seed_providers() {
  if [[ ! -f "$SEED_FILE" ]]; then
    echo "❌ Seed file not found: $SEED_FILE"
    exit 1
  fi

  echo "📖 Reading seed providers from $SEED_FILE"

  # Get number of providers in YAML
  local provider_count
  provider_count=$(yq '.providers | length' "$SEED_FILE")

  if [[ "$provider_count" -eq 0 || "$provider_count" == "null" ]]; then
    echo "   No providers defined in seed file"
    return 0
  fi

  echo "   Found $provider_count provider(s) to seed"

  local created=0
  local skipped=0

  # Process each provider
  for i in $(seq 0 $((provider_count - 1))); do
    local name
    local provider_type
    local api_key_env
    local is_default
    local api_key=""

    # Extract provider fields
    name=$(yq ".providers[$i].name" "$SEED_FILE")
    provider_type=$(yq ".providers[$i].provider_type" "$SEED_FILE")
    api_key_env=$(yq ".providers[$i].api_key_env // \"\"" "$SEED_FILE")
    is_default=$(yq ".providers[$i].is_default // false" "$SEED_FILE")

    # Get API key from environment variable if specified
    if [[ -n "$api_key_env" && "$api_key_env" != "null" && "$api_key_env" != "" ]]; then
      api_key="${!api_key_env:-}"
    fi

    # Check if provider already exists
    if provider_exists "$name"; then
      echo "   ⏭️  Skipping provider '$name' (already exists)"
      skipped=$((skipped + 1))

      # Still seed models for existing provider
      local provider_id
      provider_id=$(get_provider_id_by_name "$name")
      if [[ -n "$provider_id" ]]; then
        seed_models_for_provider "$i" "$provider_id"
      fi
      continue
    fi

    # Build create payload
    local payload
    if [[ -n "$api_key" ]]; then
      payload=$(jq -n \
        --arg name "$name" \
        --arg provider_type "$provider_type" \
        --argjson is_default "$is_default" \
        --arg api_key "$api_key" \
        '{
          name: $name,
          provider_type: $provider_type,
          is_default: $is_default,
          api_key: $api_key
        }'
      )
    else
      payload=$(jq -n \
        --arg name "$name" \
        --arg provider_type "$provider_type" \
        --argjson is_default "$is_default" \
        '{
          name: $name,
          provider_type: $provider_type,
          is_default: $is_default
        }'
      )
    fi

    # Create the provider
    echo "   🌱 Creating provider '$name'..."
    local response
    response=$(create_provider "$payload")

    local provider_id
    provider_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -z "$provider_id" ]]; then
      echo "      ❌ Failed to create provider: $response"
      continue
    fi

    if [[ -n "$api_key" ]]; then
      echo "      ✅ Created with API key from \$$api_key_env"
    else
      echo "      ✅ Created (no API key - set \$$api_key_env to configure)"
    fi

    created=$((created + 1))

    # Seed models for this provider
    seed_models_for_provider "$i" "$provider_id"
  done

  echo ""
  echo "📊 Provider seeding complete: $created created, $skipped skipped"
}

# Seed models for a specific provider
seed_models_for_provider() {
  local provider_index="$1"
  local provider_id="$2"

  local model_count
  model_count=$(yq ".providers[$provider_index].models | length" "$SEED_FILE")

  if [[ "$model_count" -eq 0 || "$model_count" == "null" ]]; then
    return 0
  fi

  # Get existing model IDs for this provider
  local existing_models
  existing_models=$(get_existing_model_ids "$provider_id")

  for j in $(seq 0 $((model_count - 1))); do
    local model_id
    local display_name
    local context_window
    local is_default

    model_id=$(yq ".providers[$provider_index].models[$j].model_id" "$SEED_FILE")
    display_name=$(yq ".providers[$provider_index].models[$j].display_name" "$SEED_FILE")
    context_window=$(yq ".providers[$provider_index].models[$j].context_window // 0" "$SEED_FILE")
    is_default=$(yq ".providers[$provider_index].models[$j].is_default // false" "$SEED_FILE")

    # Check if model already exists
    if echo "$existing_models" | grep -Fxq "$model_id"; then
      echo "      ⏭️  Skipping model '$display_name' (already exists)"
      continue
    fi

    # Build model payload
    local payload
    if [[ "$context_window" -gt 0 ]]; then
      payload=$(jq -n \
        --arg model_id "$model_id" \
        --arg display_name "$display_name" \
        --argjson context_window "$context_window" \
        --argjson is_default "$is_default" \
        '{
          model_id: $model_id,
          display_name: $display_name,
          context_window: $context_window,
          is_default: $is_default
        }'
      )
    else
      payload=$(jq -n \
        --arg model_id "$model_id" \
        --arg display_name "$display_name" \
        --argjson is_default "$is_default" \
        '{
          model_id: $model_id,
          display_name: $display_name,
          is_default: $is_default
        }'
      )
    fi

    # Create the model
    local response
    response=$(create_model "$provider_id" "$payload")

    local created_model_id
    created_model_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -z "$created_model_id" ]]; then
      echo "      ❌ Failed to create model '$display_name': $response"
      continue
    fi

    echo "      🔧 Created model '$display_name' ($model_id)"
  done
}

# Seed agents from YAML file
seed_agents() {
  if [[ ! -f "$SEED_FILE" ]]; then
    echo "❌ Seed file not found: $SEED_FILE"
    exit 1
  fi

  echo "📖 Reading seed agents from $SEED_FILE"

  # Get number of agents in YAML (mikefarah/yq syntax)
  local agent_count
  agent_count=$(yq '.agents | length' "$SEED_FILE")

  if [[ "$agent_count" -eq 0 ]]; then
    echo "   No agents defined in seed file"
    return 0
  fi

  echo "   Found $agent_count agent(s) to seed"

  local created=0
  local skipped=0

  # Process each agent
  for i in $(seq 0 $((agent_count - 1))); do
    local name
    local description
    local system_prompt
    local tags
    local capabilities

    # Extract agent fields using mikefarah/yq syntax
    name=$(yq ".agents[$i].name" "$SEED_FILE")
    description=$(yq ".agents[$i].description // \"\"" "$SEED_FILE")
    system_prompt=$(yq ".agents[$i].system_prompt" "$SEED_FILE")
    tags=$(yq -o=json -I=0 ".agents[$i].tags // []" "$SEED_FILE")
    capabilities=$(yq -o=json -I=0 ".agents[$i].capabilities // []" "$SEED_FILE")

    # Build create payload
    local payload
    payload=$(jq -n \
      --arg name "$name" \
      --arg description "$description" \
      --arg system_prompt "$system_prompt" \
      --argjson tags "$tags" \
      '{
        name: $name,
        system_prompt: $system_prompt,
        tags: $tags
      } + (if $description != "" then {description: $description} else {} end)'
    )

    # Create the agent using idempotent PUT endpoint
    echo "   🌱 Seeding agent '$name'..."
    local result
    result=$(create_agent "$payload")

    local http_code
    local response
    http_code=$(echo "$result" | cut -d'|' -f1)
    response=$(echo "$result" | cut -d'|' -f2-)

    local agent_id
    agent_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -z "$agent_id" ]]; then
      echo "      ❌ Failed to create agent: $response"
      continue
    fi

    # Check HTTP status to determine if agent was created or already existed
    if [[ "$http_code" == "200" ]]; then
      echo "      ⏭️  Already exists (skipped)"
      skipped=$((skipped + 1))
      continue
    fi

    # Set capabilities if defined (only for newly created agents)
    if [[ "$capabilities" != "[]" && "$capabilities" != "null" ]]; then
      local cap_payload
      cap_payload=$(echo "$capabilities" | jq '{capabilities: .}')
      set_agent_capabilities "$agent_id" "$cap_payload"
      echo "      ✅ Created with capabilities: $capabilities"
    else
      echo "      ✅ Created (no capabilities)"
    fi

    created=$((created + 1))
  done

  echo ""
  echo "📊 Seeding complete: $created created, $skipped skipped"
}

# Main execution
main() {
  echo "🌱 Seeding development database..."
  echo ""

  check_tools
  wait_for_api

  echo ""
  seed_providers

  echo ""
  seed_agents

  echo ""
  echo "✅ Done!"
}

main
