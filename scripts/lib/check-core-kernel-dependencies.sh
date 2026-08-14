#!/usr/bin/env bash
# Architecture guard (EVE-903): everruns-core is the transport-free execution
# kernel. Concrete TLS selection and HTTP provider drivers live in their owners.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0
fail() { echo "$1"; FAILED=1; }

METADATA=$(cargo metadata --no-deps --format-version 1)
CORE_PACKAGE=$(printf '%s' "$METADATA" | jq -c '.packages[] | select(.name == "everruns-core")')

DEFAULT_FEATURES=$(printf '%s' "$CORE_PACKAGE" | jq -c '.features.default')
if [ "$DEFAULT_FEATURES" != '[]' ]; then
  fail "everruns-core default features must be intentionally empty; found $DEFAULT_FEATURES"
fi
if printf '%s' "$CORE_PACKAGE" | jq -e '.features | has("llm-tests")' >/dev/null; then
  fail "everruns-core must not expose the dead llm-tests feature"
fi

EXPECTED_NORMAL='anyhow
async-trait
base64
chrono
cron
everruns-capability
everruns-provider
futures
globset
inventory
jsonschema
rand
regex
serde
serde_json
serde_yaml
sha2
thiserror
tokio
tokio-util
toml
tracing
tree-sitter
tree-sitter-python
tree-sitter-rust
tree-sitter-typescript
url
utoipa
uuid'
ACTUAL_NORMAL=$(printf '%s' "$CORE_PACKAGE" | jq -r '.dependencies[] | select((.kind // "normal") == "normal") | .name' | sort -u)
if [ "$ACTUAL_NORMAL" != "$EXPECTED_NORMAL" ]; then
  fail "everruns-core direct dependency set changed; re-audit every entry and update this guard intentionally:"
  diff -u <(printf '%s\n' "$EXPECTED_NORMAL") <(printf '%s\n' "$ACTUAL_NORMAL") || true
fi

OPTIONAL=$(printf '%s' "$CORE_PACKAGE" | jq -r '.dependencies[] | select((.kind // "normal") == "normal" and .optional) | .name' | sort -u)
EXPECTED_OPTIONAL='tree-sitter
tree-sitter-python
tree-sitter-rust
tree-sitter-typescript
utoipa'
if [ "$OPTIONAL" != "$EXPECTED_OPTIONAL" ]; then
  fail "everruns-core optional dependency set changed; expected only OpenAPI and structural-outline opt-ins:"
  diff -u <(printf '%s\n' "$EXPECTED_OPTIONAL") <(printf '%s\n' "$OPTIONAL") || true
fi

DEV_DEPENDENCIES=$(printf '%s' "$CORE_PACKAGE" | jq -r '.dependencies[] | select(.kind == "dev") | .name' | sort -u)
EXPECTED_DEV='insta
tempfile
tokio'
if [ "$DEV_DEPENDENCIES" != "$EXPECTED_DEV" ]; then
  fail "everruns-core dev dependency set changed; re-audit test-only dependencies:"
  diff -u <(printf '%s\n' "$EXPECTED_DEV") <(printf '%s\n' "$DEV_DEPENDENCIES") || true
fi

if printf '%s' "$CORE_PACKAGE" | jq -e '.dependencies[] | select(.kind == "build")' >/dev/null; then
  fail "everruns-core must not add build dependencies without an explicit audit"
fi

if ! printf '%s' "$CORE_PACKAGE" | jq -e '.dependencies[] | select(.name == "everruns-provider" and .uses_default_features == false)' >/dev/null; then
  fail "everruns-core must consume everruns-provider with default features disabled"
fi

SOURCE_PATTERN='(rustls|reqwest|hyper|sqlx|opentelemetry|tracing_opentelemetry)::|tokio::(net|process)|OpenAIProtocolChatDriver|OpenResponsesProtocolChatDriver'
if matches=$(grep -rnE "$SOURCE_PATTERN" crates/core/src --include='*.rs' 2>/dev/null); then
  fail "everruns-core sources reference concrete TLS/transport/database/exporter/runtime APIs:"
  echo "$matches"
fi

CORE_TREE=$(cargo tree -p everruns-core --edges normal,build --prefix none 2>/dev/null)
FORBIDDEN_TREE='^(rustls|rustls-webpki|reqwest|hyper|hyper-util|hyper-rustls|h2|tower-http|eventsource-stream|sqlx|opentelemetry|opentelemetry_sdk|opentelemetry-otlp|opentelemetry-http|opentelemetry-proto|tracing-opentelemetry|tree-sitter|tree-sitter-rust|tree-sitter-typescript|tree-sitter-python|utoipa|mlua|bashkit|deno_core|wasmtime) '
if echo "$CORE_TREE" | grep -qE "$FORBIDDEN_TREE"; then
  fail "everruns-core default tree contains a forbidden implementation dependency:"
  echo "$CORE_TREE" | grep -E "$FORBIDDEN_TREE" | sort -u
fi

TOKIO_FEATURES=$(cargo tree -p everruns-core --edges normal,build,features -i tokio 2>/dev/null || true)
if echo "$TOKIO_FEATURES" | grep -qE 'tokio feature "(net|process|fs|io-util|signal|rt-multi-thread|full)"'; then
  fail "everruns-core enables a non-kernel Tokio feature:"
  echo "$TOKIO_FEATURES" | grep -E 'tokio feature "(net|process|fs|io-util|signal|rt-multi-thread|full)"' | sort -u
fi

OPENAPI_TREE=$(cargo tree -p everruns-core --features openapi --edges normal,build --prefix none 2>/dev/null)
if ! echo "$OPENAPI_TREE" | grep -qE '^utoipa '; then
  fail "everruns-core openapi opt-in must activate utoipa"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Core kernel dependency guard failed (EVE-903)."
  exit 1
fi

echo "Core kernel dependency guard passed: defaults empty; TLS/HTTP/process/DB/interpreter/exporter deps absent; opt-ins explicit."
