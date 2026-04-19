#!/usr/bin/env bash
set -euo pipefail

base_url="${1:?usage: resolve-harness-id.sh <base_url> <api_key> [harness_name] [timeout_seconds]}"
api_key="${2:?usage: resolve-harness-id.sh <base_url> <api_key> [harness_name] [timeout_seconds]}"
harness_name="${3:-generic}"
timeout_seconds="${4:-30}"

deadline=$((SECONDS + timeout_seconds))

while (( SECONDS < deadline )); do
  if response="$(curl -fsS -H "Authorization: Bearer $api_key" "$base_url/v1/harnesses/$harness_name" 2>/dev/null)"; then
    python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$response"
    exit 0
  fi

  sleep 1
done

echo "Timed out waiting for harness '$harness_name' at $base_url/v1/harnesses/$harness_name" >&2
exit 1
