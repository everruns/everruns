#!/bin/bash
# Export OpenAPI specification
#
# Usage: ./scripts/export-openapi.sh [output-file]
#
# Generates the OpenAPI spec using the export-openapi binary.
# No running API server required.
#
# Output defaults to docs/api/openapi.json

set -e

OUTPUT_FILE="${1:-docs/api/openapi.json}"

echo "Generating OpenAPI spec..."

# Build and run the export-openapi binary
cargo run --bin export-openapi --release 2>/dev/null > "$OUTPUT_FILE"

echo "OpenAPI spec saved to $OUTPUT_FILE"
echo "Spec version: $(jq -r '.info.version' "$OUTPUT_FILE")"
echo "Endpoints: $(jq '.paths | keys | length' "$OUTPUT_FILE")"
