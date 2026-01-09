#!/usr/bin/env bash
set -euo pipefail

# Patch API keys for LLM providers from environment variables
# Usage: ./scripts/patch-provider-keys.sh [--api-url URL]
#
# This script creates database providers with API keys for config-based providers.
# Database providers take priority over config providers with the same name.
#
# Environment Variables:
#   OPENAI_API_KEY    - API key for OpenAI provider
#   ANTHROPIC_API_KEY - API key for Anthropic provider
#   API_URL           - API base URL (default: http://localhost:9000)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Load .env file if it exists
if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  source "$PROJECT_ROOT/.env"
  set +a
fi

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
    echo "curl is required but not installed"
    exit 1
  fi
  if ! command -v jq &> /dev/null; then
    echo "jq is required but not installed"
    echo "   Install with: apt-get install jq (or brew install jq)"
    exit 1
  fi
}

# Wait for API to be healthy
wait_for_api() {
  local max_attempts=30
  local attempt=0

  echo "Waiting for API to be healthy at $API_URL..."

  while [[ $attempt -lt $max_attempts ]]; do
    if curl -s "$API_URL/health" > /dev/null 2>&1; then
      echo "   API is healthy"
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done

  echo "API not healthy after $max_attempts seconds"
  exit 1
}

# Get provider by name
get_provider_by_name() {
  local name="$1"
  curl -s "$API_URL/v1/llm-providers" | jq -r ".data[] | select(.name == \"$name\")"
}

# Check if provider exists and is not readonly (i.e., is a database provider)
provider_is_db() {
  local name="$1"
  local provider
  provider=$(get_provider_by_name "$name")

  if [[ -z "$provider" || "$provider" == "null" ]]; then
    return 1
  fi

  local readonly
  readonly=$(echo "$provider" | jq -r '.readonly // false')

  [[ "$readonly" == "false" ]]
}

# Get provider ID by name
get_provider_id_by_name() {
  local name="$1"
  curl -s "$API_URL/v1/llm-providers" | jq -r ".data[] | select(.name == \"$name\") | .id"
}

# Get provider details by name
get_provider_details() {
  local name="$1"
  curl -s "$API_URL/v1/llm-providers" | jq -r ".data[] | select(.name == \"$name\")"
}

# Create a database provider with the same name as a config provider, but with API key
create_provider_with_key() {
  local name="$1"
  local provider_type="$2"
  local api_key="$3"

  local payload
  payload=$(jq -n \
    --arg name "$name" \
    --arg provider_type "$provider_type" \
    --arg api_key "$api_key" \
    '{
      name: $name,
      provider_type: $provider_type,
      api_key: $api_key
    }'
  )

  local response
  response=$(curl -s -X POST "$API_URL/v1/llm-providers" \
    -H "Content-Type: application/json" \
    -d "$payload")

  echo "$response"
}

# Update an existing database provider's API key
update_provider_key() {
  local provider_id="$1"
  local api_key="$2"

  local payload
  payload=$(jq -n \
    --arg api_key "$api_key" \
    '{api_key: $api_key}'
  )

  local response
  response=$(curl -s -X PATCH "$API_URL/v1/llm-providers/$provider_id" \
    -H "Content-Type: application/json" \
    -d "$payload")

  echo "$response"
}

# Patch a provider's API key - creates DB provider if needed
patch_provider_key() {
  local name="$1"
  local provider_type="$2"
  local api_key="$3"

  if [[ -z "$api_key" ]]; then
    echo "   Skipping $name (no API key set)"
    return 0
  fi

  # Check if a database provider already exists
  if provider_is_db "$name"; then
    # Update existing database provider
    local provider_id
    provider_id=$(get_provider_id_by_name "$name")
    echo "   Updating API key for $name..."
    local response
    response=$(update_provider_key "$provider_id" "$api_key")

    local updated_id
    updated_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -n "$updated_id" ]]; then
      echo "      Updated API key"
    else
      echo "      Failed to update: $response"
    fi
  else
    # Create a new database provider with the API key
    # This will take priority over the readonly config provider
    echo "   Creating database provider for $name with API key..."
    local response
    response=$(create_provider_with_key "$name" "$provider_type" "$api_key")

    local created_id
    created_id=$(echo "$response" | jq -r '.id // empty')

    if [[ -n "$created_id" ]]; then
      echo "      Created provider with API key (overrides config)"
    else
      echo "      Failed to create: $response"
    fi
  fi
}

# Main execution
main() {
  echo "Patching LLM provider API keys from environment..."
  echo ""

  check_tools
  wait_for_api

  echo ""
  echo "Patching providers:"

  # Patch OpenAI provider
  patch_provider_key "OpenAI" "openai" "${OPENAI_API_KEY:-}"

  # Patch Anthropic provider
  patch_provider_key "Anthropic" "anthropic" "${ANTHROPIC_API_KEY:-}"

  echo ""
  echo "Done!"
}

main
