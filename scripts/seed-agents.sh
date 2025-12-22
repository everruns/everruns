#!/usr/bin/env bash
set -euo pipefail

# Seed agents from YAML configuration into the local development database
# Usage: ./scripts/seed-agents.sh [--api-url URL]
#
# Supports both mikefarah/yq (Go) and kislyuk/yq (Python) versions

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SEED_FILE="$PROJECT_ROOT/harness/seed-agents.yaml"

API_URL="${API_URL:-http://localhost:9000}"

# yq variant: "go" for mikefarah/yq, "python" for kislyuk/yq
YQ_VARIANT=""

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

# Detect which yq variant is installed
detect_yq_variant() {
  local version_output
  version_output=$(yq --version 2>&1 || true)

  if echo "$version_output" | grep -qi "mikefarah\|https://github.com/mikefarah/yq"; then
    YQ_VARIANT="go"
    echo "   Using mikefarah/yq (Go)"
  elif echo "$version_output" | grep -qE "^yq [0-9]" || echo "$version_output" | grep -qi "kislyuk"; then
    YQ_VARIANT="python"
    echo "   Using kislyuk/yq (Python)"
  else
    # Try to detect by behavior - Go yq outputs version differently
    if yq --version 2>&1 | grep -qE "version v?[0-9]+\.[0-9]+"; then
      YQ_VARIANT="go"
      echo "   Using mikefarah/yq (Go)"
    else
      YQ_VARIANT="python"
      echo "   Using kislyuk/yq (Python) [assumed]"
    fi
  fi
}

# Wrapper for yq that handles both variants
# Usage: yq_read <expression> <file>
yq_read() {
  local expr="$1"
  local file="$2"

  if [[ "$YQ_VARIANT" == "go" ]]; then
    yq -r "$expr" "$file"
  else
    yq -r "$expr" "$file"
  fi
}

# Wrapper for yq JSON output
# Usage: yq_json <expression> <file>
yq_json() {
  local expr="$1"
  local file="$2"

  if [[ "$YQ_VARIANT" == "go" ]]; then
    yq -o=json -I=0 "$expr" "$file"
  else
    yq -c "$expr" "$file"
  fi
}

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
    echo "   Install with:"
    echo "     Go version:     brew install yq (or go install github.com/mikefarah/yq/v4@latest)"
    echo "     Python version: pip install yq"
    exit 1
  fi

  detect_yq_variant
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

# Create an agent from JSON payload
create_agent() {
  local payload="$1"
  local response

  response=$(curl -s -X POST "$API_URL/v1/agents" \
    -H "Content-Type: application/json" \
    -d "$payload")

  echo "$response"
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

# Seed agents from YAML file
seed_agents() {
  if [[ ! -f "$SEED_FILE" ]]; then
    echo "❌ Seed file not found: $SEED_FILE"
    exit 1
  fi

  echo "📖 Reading seed agents from $SEED_FILE"

  # Get number of agents in YAML
  local agent_count
  agent_count=$(yq_read '.agents | length' "$SEED_FILE")

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

    # Extract agent fields using yq wrapper
    name=$(yq_read ".agents[$i].name" "$SEED_FILE")
    description=$(yq_read ".agents[$i].description // \"\"" "$SEED_FILE")
    system_prompt=$(yq_read ".agents[$i].system_prompt" "$SEED_FILE")
    tags=$(yq_json ".agents[$i].tags // []" "$SEED_FILE")
    capabilities=$(yq_json ".agents[$i].capabilities // []" "$SEED_FILE")

    # Check if agent already exists
    if agent_exists "$name"; then
      echo "   ⏭️  Skipping '$name' (already exists)"
      skipped=$((skipped + 1))
      continue
    fi

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

    # Create the agent
    echo "   🌱 Creating agent '$name'..."
    local response
    response=$(create_agent "$payload")

    local agent_id
    agent_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -z "$agent_id" ]]; then
      echo "      ❌ Failed to create agent: $response"
      continue
    fi

    # Set capabilities if defined
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
  echo "🌱 Seeding development agents..."
  echo ""

  check_tools
  wait_for_api
  seed_agents

  echo ""
  echo "✅ Done!"
}

main
