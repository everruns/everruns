#!/bin/bash
# Export OpenAPI specification
#
# Usage: ./scripts/export-openapi.sh [output-file]
#
# Generates the OpenAPI spec using the export-openapi binary.
# No running API server required.
#
# Output defaults to docs/api/openapi.json

set -euo pipefail

OUTPUT_FILE="${1:-docs/api/openapi.json}"

echo "Generating OpenAPI spec..."

# Generate to a temporary file and move it into place only on success.
#
# The release build takes minutes, so an interrupted run is routine. Redirecting
# the binary's stdout straight at $OUTPUT_FILE truncates the committed spec the
# moment the shell opens it, which leaves an empty docs/api/openapi.json behind
# on Ctrl-C, a timeout, or a build failure.
TEMP_FILE="$(mktemp "${TMPDIR:-/tmp}/openapi-export.XXXXXX.json")"
trap 'rm -f "$TEMP_FILE"' EXIT

cargo run --bin export-openapi --release 2>/dev/null > "$TEMP_FILE"

# A half-written spec is worse than none: it would be committed and could pass
# a freshness diff against another half-written copy. Require valid JSON that
# actually carries paths before replacing the checked-in file.
if ! jq -e '.paths | length > 0' "$TEMP_FILE" > /dev/null 2>&1; then
  echo "Generated spec is empty or invalid — leaving $OUTPUT_FILE untouched." >&2
  exit 1
fi

mv "$TEMP_FILE" "$OUTPUT_FILE"
trap - EXIT

echo "OpenAPI spec saved to $OUTPUT_FILE"
echo "Spec version: $(jq -r '.info.version' "$OUTPUT_FILE")"
echo "Endpoints: $(jq '.paths | keys | length' "$OUTPUT_FILE")"
