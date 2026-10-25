#!/usr/bin/env bash
# Refresh the vendored Daytona OpenAPI specs that generate the Daytona client
# (crates/integrations/build.rs). Run it when Daytona ships an API change, or
# when the live drift test (`daytona_live_api_test::test_live_openapi_matches_vendored`)
# fails.
#
# - Management API: public, served at https://app.daytona.io/api-json.
# - Toolbox API: served by the daemon inside every sandbox at
#   /swagger/doc.json, so this script creates a throwaway sandbox (needs
#   DAYTONA_API_KEY) and deletes it again. The daemon publishes Swagger 2.0;
#   it is converted to OpenAPI 3.0 here because the generator reads 3.0 only.
#
# Both files are written with sorted keys and two-space indent so a refresh
# shows up as a readable diff.
#
# Usage: doppler run -- scripts/update-daytona-openapi.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/crates/integrations/openapi/daytona"
API_BASE="${DAYTONA_API_BASE:-https://app.daytona.io/api}"
TOOLBOX_BASE="${DAYTONA_TOOLBOX_BASE:-https://proxy.app.daytona.io/toolbox}"
: "${DAYTONA_API_KEY:?DAYTONA_API_KEY is required to read the toolbox spec from a sandbox}"

work="$(mktemp -d)"
sandbox_id=""
cleanup() {
  if [ -n "$sandbox_id" ]; then
    curl -fsS -X DELETE "$API_BASE/sandbox/$sandbox_id" \
      -H "Authorization: Bearer $DAYTONA_API_KEY" -o /dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

normalize() {
  python3 -c 'import json,sys; json.dump(json.load(open(sys.argv[1])), open(sys.argv[2], "w"), indent=2, sort_keys=True); open(sys.argv[2], "a").write("\n")' "$1" "$2"
}

curl -fsS "${API_BASE%/api}/api-json" -o "$work/api.json"

sandbox_id="$(curl -fsS -X POST "$API_BASE/sandbox" \
  -H "Authorization: Bearer $DAYTONA_API_KEY" -H 'Content-Type: application/json' \
  -d '{"labels":{"everruns.purpose":"openapi-refresh"},"autoDeleteInterval":10}' |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"

for _ in $(seq 1 60); do
  if curl -fsS "$TOOLBOX_BASE/$sandbox_id/swagger/doc.json" \
    -H "Authorization: Bearer $DAYTONA_API_KEY" -o "$work/toolbox-swagger.json" 2>/dev/null; then
    break
  fi
  sleep 2
done
[ -s "$work/toolbox-swagger.json" ] || { echo "toolbox spec not reachable" >&2; exit 1; }

npx -y swagger2openapi@7.0.8 "$work/toolbox-swagger.json" -o "$work/toolbox.json"

mkdir -p "$OUT"
normalize "$work/api.json" "$OUT/api.json"
normalize "$work/toolbox.json" "$OUT/toolbox.json"
echo "Updated $OUT (toolbox $(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["info"]["version"])' "$OUT/toolbox.json"))"
