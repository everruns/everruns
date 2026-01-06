#!/bin/bash
# Export OpenAPI specification from the running API
#
# Usage: ./scripts/export-openapi.sh [output-file]
#
# Requires the API to be running at localhost:9000
# Output defaults to docs/api/openapi.json

set -e

API_URL="${API_URL:-http://localhost:9000}"
OUTPUT_FILE="${1:-docs/api/openapi.json}"

echo "Fetching OpenAPI spec from $API_URL/api-doc/openapi.json..."

# Check if API is running
if ! curl -s --fail "$API_URL/health" > /dev/null 2>&1; then
    echo "Error: API is not running at $API_URL"
    echo "Start the API first with: ./scripts/dev.sh api"
    exit 1
fi

# Fetch and save the spec
curl -s "$API_URL/api-doc/openapi.json" | jq '.' > "$OUTPUT_FILE"

echo "OpenAPI spec saved to $OUTPUT_FILE"
echo "Spec version: $(jq -r '.info.version' "$OUTPUT_FILE")"
echo "Endpoints: $(jq '.paths | keys | length' "$OUTPUT_FILE")"
