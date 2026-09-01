#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
source "$PROJECT_ROOT/scripts/lib/local-development-secrets.sh"
TEST_DIR="$(mktemp -d)"
NEXT_BIN="$PROJECT_ROOT/apps/ui/node_modules/.bin/next"
CREATED_NEXT_BIN=false

cleanup() {
  rm -rf "$TEST_DIR"
  if [ "$CREATED_NEXT_BIN" = true ]; then
    rm -f "$NEXT_BIN"
    rmdir "$PROJECT_ROOT/apps/ui/node_modules/.bin" 2>/dev/null || true
    rmdir "$PROJECT_ROOT/apps/ui/node_modules" 2>/dev/null || true
  fi
}

trap cleanup EXIT

mkdir -p "$TEST_DIR/bin"

cat > "$TEST_DIR/bin/doppler" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

preserve=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    run) shift ;;
    --preserve-env=*) preserve="${1#*=}"; shift ;;
    --) shift; break ;;
    *) echo "unexpected doppler argument: $1" >&2; exit 1 ;;
  esac
done

[ "$preserve" = "PORT_PREFIX,AUTH_MODE" ] || {
  echo "caller selectors were not preserved through Doppler" >&2
  exit 1
}

caller_prefix="$PORT_PREFIX"
caller_auth_mode="$AUTH_MODE"
export PORT_PREFIX=93 AUTH_MODE=full OPENAI_API_KEY=injected-openai ANTHROPIC_API_KEY=injected-anthropic
export PORT_PREFIX="$caller_prefix" AUTH_MODE="$caller_auth_mode"
exec "$@"
EOF

cat > "$TEST_DIR/bin/just" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$PORT_PREFIX|$AUTH_MODE|${OPENAI_API_KEY:-}|${ANTHROPIC_API_KEY:-}|${SECRETS_ENCRYPTION_KEY:-}|${SECRETS_ENCRYPTION_KEY_PREVIOUS:-}|$*" > "$AGENT_START_CAPTURE"
EOF

for command_name in sqlx pnpm caddy nats-server pg_ctl valkey-server; do
  cat > "$TEST_DIR/bin/$command_name" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
done
chmod +x "$TEST_DIR/bin/"*

if [ ! -x "$NEXT_BIN" ]; then
  mkdir -p "$(dirname "$NEXT_BIN")"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$NEXT_BIN"
  chmod +x "$NEXT_BIN"
  CREATED_NEXT_BIN=true
fi

capture="$TEST_DIR/capture"
PATH="$TEST_DIR/bin:$PATH" \
  PORT_PREFIX=271 \
  AUTH_MODE=none \
  SECRETS_ENCRYPTION_KEY='test-only-not-a-secret' \
  AGENT_START_CAPTURE="$capture" \
  "$PROJECT_ROOT/scripts/start-agent-dev.sh"

expected='271|none|injected-openai|injected-anthropic|test-only-not-a-secret||start-all --no-watch'
actual="$(<"$capture")"
[ "$actual" = "$expected" ] || {
  echo "FAIL: expected '$expected', got '$actual'" >&2
  exit 1
}

env -u SECRETS_ENCRYPTION_KEY \
  -u SECRETS_ENCRYPTION_KEY_PREVIOUS \
  PATH="$TEST_DIR/bin:$PATH" \
  LOCAL_DEVELOPMENT_SECRETS_DIR="$TEST_DIR/secrets" \
  PORT_PREFIX=272 \
  AUTH_MODE=none \
  AGENT_START_CAPTURE="$capture" \
  "$PROJECT_ROOT/scripts/start-agent-dev.sh"

actual="$(<"$capture")"
IFS='|' read -r prefix auth openai anthropic generated previous command <<< "$actual"
[ "$prefix|$auth|$openai|$anthropic|$previous|$command" = \
  "272|none|injected-openai|injected-anthropic|$LEGACY_LOCAL_SECRETS_ENCRYPTION_KEY|start-all --no-watch" ] || {
  echo "FAIL: private-key startup contract mismatch: '$actual'" >&2
  exit 1
}
[ "$generated" != "$LEGACY_LOCAL_SECRETS_ENCRYPTION_KEY" ]
[ "$(stat -c '%a' "$TEST_DIR/secrets/secrets-encryption-key-272")" = 600 ]

env -u SECRETS_ENCRYPTION_KEY -u SECRETS_ENCRYPTION_KEY_PREVIOUS \
  PATH="$TEST_DIR/bin:$PATH" LOCAL_DEVELOPMENT_SECRETS_DIR="$TEST_DIR/secrets" \
  PORT_PREFIX=272 AUTH_MODE=none AGENT_START_CAPTURE="$capture" \
  "$PROJECT_ROOT/scripts/start-agent-dev.sh"
restarted="$(<"$capture")"
[ "$restarted" = "$actual" ] || {
  echo "FAIL: local encryption key changed across restarts" >&2
  exit 1
}

legacy_key_definitions="$(rg -lF "$LEGACY_LOCAL_SECRETS_ENCRYPTION_KEY" \
  "$PROJECT_ROOT/scripts" | wc -l | tr -d ' ')"
[ "$legacy_key_definitions" = "1" ] || {
  echo "FAIL: expected one legacy local encryption key definition, found $legacy_key_definitions" >&2
  exit 1
}

if PORT_PREFIX=655 AUTH_MODE=none "$PROJECT_ROOT/scripts/start-agent-dev.sh" >/dev/null 2>&1; then
  echo "FAIL: invalid port prefix was accepted" >&2
  exit 1
fi

if PORT_PREFIX=271 AUTH_MODE=production "$PROJECT_ROOT/scripts/start-agent-dev.sh" >/dev/null 2>&1; then
  echo "FAIL: invalid auth mode was accepted" >&2
  exit 1
fi

rg -qF 'PORT_PREFIX=271 AUTH_MODE=none ./scripts/start-agent-dev.sh' "$PROJECT_ROOT/AGENTS.md"

canonical_references="$(rg -oF './scripts/start-agent-dev.sh' \
  "$PROJECT_ROOT/AGENTS.md" \
  "$PROJECT_ROOT/.agents/skills/manual-ui-testing/SKILL.md" \
  "$PROJECT_ROOT/.agents/skills/ship/SKILL.md" \
  "$PROJECT_ROOT/scripts/init-cloud-env.sh" | wc -l | tr -d ' ')"
[ "$canonical_references" = "1" ] || {
  echo "FAIL: expected one canonical agent startup command, found $canonical_references" >&2
  exit 1
}

if rg -n 'doppler run -- (just )?start-(all|dev)' \
  "$PROJECT_ROOT/AGENTS.md" \
  "$PROJECT_ROOT/.agents/skills/manual-ui-testing/SKILL.md" \
  "$PROJECT_ROOT/.agents/skills/ship/SKILL.md" \
  "$PROJECT_ROOT/scripts/init-cloud-env.sh"; then
  echo "FAIL: stale direct Doppler startup command found" >&2
  exit 1
fi

if rg -n '/healthz' \
  "$PROJECT_ROOT/AGENTS.md" \
  "$PROJECT_ROOT/.agents/skills/manual-ui-testing/SKILL.md"; then
  echo "FAIL: stale health endpoint found in agent startup guidance" >&2
  exit 1
fi

echo "agent development startup contract checks passed"
