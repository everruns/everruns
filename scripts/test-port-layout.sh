#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck disable=SC1091
source "$PROJECT_ROOT/scripts/lib/common.sh"

assert_eq() {
  local expected="$1"
  local actual="$2"
  local label="$3"

  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $label expected '$expected', got '$actual'" >&2
    exit 1
  fi
}

check_default_ports() {
  unset PORT_PREFIX API_PORT WORKER_GRPC_PORT CADDY_ADMIN_PORT UI_PORT PROXY_PORT OTEL_GRPC_PORT OTEL_HTTP_PORT VALKEY_PORT DB_PORT COMPOSE_PROJECT_NAME
  apply_port_prefix_defaults

  assert_eq "9300" "$PROXY_PORT" "default proxy port"
  assert_eq "9301" "$API_PORT" "default api port"
  assert_eq "9001" "$WORKER_GRPC_PORT" "default worker gRPC port"
  assert_eq "2019" "$CADDY_ADMIN_PORT" "default caddy admin port"
  assert_eq "9305" "$UI_PORT" "default ui port"
  assert_eq "4317" "$OTEL_GRPC_PORT" "default OTLP gRPC port"
  assert_eq "4318" "$OTEL_HTTP_PORT" "default OTLP HTTP port"
  assert_eq "6379" "$VALKEY_PORT" "default valkey port"
  assert_eq "9332" "$DB_PORT" "default db port"
  assert_eq "everruns" "$COMPOSE_PROJECT_NAME" "default compose project"
}

check_prefixed_ports() {
  export PORT_PREFIX=271
  unset API_PORT WORKER_GRPC_PORT CADDY_ADMIN_PORT UI_PORT PROXY_PORT OTEL_GRPC_PORT OTEL_HTTP_PORT VALKEY_PORT DB_PORT COMPOSE_PROJECT_NAME
  apply_port_prefix_defaults

  assert_eq "27100" "$PROXY_PORT" "prefixed proxy port"
  assert_eq "27101" "$API_PORT" "prefixed api port"
  assert_eq "27102" "$WORKER_GRPC_PORT" "prefixed worker gRPC port"
  assert_eq "27103" "$CADDY_ADMIN_PORT" "prefixed caddy admin port"
  assert_eq "27105" "$UI_PORT" "prefixed ui port"
  assert_eq "27117" "$OTEL_GRPC_PORT" "prefixed OTLP gRPC port"
  assert_eq "27118" "$OTEL_HTTP_PORT" "prefixed OTLP HTTP port"
  assert_eq "27179" "$VALKEY_PORT" "prefixed valkey port"
  assert_eq "27132" "$DB_PORT" "prefixed db port"
  assert_eq "everruns-271" "$COMPOSE_PROJECT_NAME" "prefixed compose project"
}

check_example_ports() {
  local compose_file="$PROJECT_ROOT/examples/docker-compose-full.yaml"
  local local_compose_file="$PROJECT_ROOT/local/docker-compose.yml"

  if rg -q '^    container_name:' "$compose_file"; then
    echo "FAIL: example compose should not hardcode container_name" >&2
    exit 1
  fi
  rg -q 'PORT: "9301"' "$compose_file"
  rg -q 'PORT: "9305"' "$compose_file"
  rg -q 'reverse_proxy server:9301' "$compose_file"
  rg -q 'reverse_proxy ui:9305' "$compose_file"
  rg -q '\$\{EXAMPLE_PROXY_PORT:-9300\}:9300' "$compose_file"
  rg -q '\$\{VALKEY_PORT:-6379\}:6379' "$local_compose_file"
  rg -q '\$\{OTEL_GRPC_PORT:-4317\}:4317' "$local_compose_file"
  rg -q '\$\{OTEL_HTTP_PORT:-4318\}:4318' "$local_compose_file"
}

check_caddyfile() {
  if ! command -v caddy >/dev/null 2>&1; then
    echo "SKIP: caddy not installed, skipping Caddyfile validation"
    return 0
  fi

  caddy validate --config "$PROJECT_ROOT/local/Caddyfile" --adapter caddyfile >/dev/null

  local example_caddyfile
  example_caddyfile="$(mktemp)"
  python3 - <<'PY' "$PROJECT_ROOT/examples/docker-compose-full.yaml" "$example_caddyfile"
from pathlib import Path
import re
import sys

compose_file = Path(sys.argv[1])
output_file = Path(sys.argv[2])
text = compose_file.read_text()
match = re.search(r"caddyfile:\n\s+content: \|\n(?P<body>(?:\s{6}.*\n)+)", text)
if not match:
    raise SystemExit("FAIL: example compose Caddyfile not found")

body = []
for line in match.group("body").splitlines():
    if not line.startswith("      "):
        raise SystemExit("FAIL: example compose Caddyfile indentation changed unexpectedly")
    body.append(line[6:])

output_file.write_text("\n".join(body) + "\n")
PY
  caddy validate --config "$example_caddyfile" --adapter caddyfile >/dev/null
  rm -f "$example_caddyfile"
}

check_default_ports
check_prefixed_ports
check_example_ports
check_caddyfile

echo "port layout checks passed"
