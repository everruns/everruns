#!/usr/bin/env bash
set -euo pipefail

# Patch API keys for LLM providers from environment variables
# Usage: ./scripts/patch-provider-keys.sh [--api-url URL]
#
# Providers are created by database migration (003_default_providers.sql).
# This script just updates their API keys.
#
# Environment Variables:
#   OPENAI_API_KEY    - API key for OpenAI provider
#   ANTHROPIC_API_KEY - API key for Anthropic provider
#   API_URL           - API base URL (default: http://localhost:9000)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Well-known provider UUIDs from migration 003_default_providers.sql
OPENAI_PROVIDER_ID="01933b5a-0000-7000-8000-000000000001"
ANTHROPIC_PROVIDER_ID="01933b5a-0000-7000-8000-000000000002"

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

# Update a provider's API key
update_provider_key() {
  local provider_id="$1"
  local provider_name="$2"
  local api_key="$3"

  if [[ -z "$api_key" ]]; then
    echo "   Skipping $provider_name (no API key set)"
    return 0
  fi

  echo "   Updating API key for $provider_name..."

  local payload
  payload=$(jq -n \
    --arg api_key "$api_key" \
    '{api_key: $api_key}'
  )

  local response
  local http_code
  response=$(curl -s -w "\n%{http_code}" -X PATCH "$API_URL/v1/llm-providers/$provider_id" \
    -H "Content-Type: application/json" \
    -d "$payload")

  http_code=$(echo "$response" | tail -n1)
  response=$(echo "$response" | sed '$d')

  if [[ "$http_code" == "200" ]]; then
    echo "      Updated API key"
  elif [[ "$http_code" == "404" ]]; then
    echo "      Provider not found (run migrations first)"
  else
    echo "      Failed to update (HTTP $http_code): $response"
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

  # Update OpenAI provider
  update_provider_key "$OPENAI_PROVIDER_ID" "OpenAI" "${OPENAI_API_KEY:-}"

  # Update Anthropic provider
  update_provider_key "$ANTHROPIC_PROVIDER_ID" "Anthropic" "${ANTHROPIC_API_KEY:-}"

  echo ""
  echo "Done!"
}

main
