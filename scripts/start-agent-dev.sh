#!/usr/bin/env bash
# Canonical DB-backed local stack for coding agents. Operational guidance lives in AGENTS.md.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
source "$SCRIPT_DIR/lib/local-development-secrets.sh"

usage() {
  cat <<'EOF'
Usage: PORT_PREFIX=<1-654> AUTH_MODE=<none|admin|full|external> ./scripts/start-agent-dev.sh

See AGENTS.md for prerequisites, port layout, auth setup, persistence, verification, and shutdown.
EOF
}

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  usage
  exit 0
fi

if [ -n "${1:-}" ] && [ "${1:-}" != "--doppler-injected" ]; then
  usage >&2
  exit 2
fi

if ! [[ "${PORT_PREFIX:-}" =~ ^[1-9][0-9]{0,2}$ ]] || [ "$PORT_PREFIX" -gt 654 ]; then
  echo "PORT_PREFIX must be an integer from 1 through 654 (for example, 271)." >&2
  exit 2
fi

case "${AUTH_MODE:-}" in
  none|admin|full|external) ;;
  *)
    echo "AUTH_MODE must be explicit: none, admin, full, or external." >&2
    exit 2
    ;;
esac

# Maintainers keep provider keys in Doppler. Contributors outside the project have
# no Doppler access, so without a configured Doppler the stack starts from the
# caller's own environment (OPENAI_API_KEY, ANTHROPIC_API_KEY, ...) instead.
doppler_configured() {
  command -v doppler >/dev/null 2>&1 || return 1
  [ -n "${DOPPLER_TOKEN:-}" ] && return 0
  [ -n "$(doppler configure get token --plain 2>/dev/null || true)" ]
}

if [ "${1:-}" != "--doppler-injected" ]; then
  if doppler_configured; then
    # Doppler secrets normally win over the caller environment. Preserve the two
    # caller-owned selectors so a Doppler config cannot silently move or re-auth the stack.
    exec doppler run --preserve-env="PORT_PREFIX,AUTH_MODE" -- \
      "$PROJECT_ROOT/scripts/start-agent-dev.sh" --doppler-injected
  fi
  echo "Doppler is not configured; using provider keys from your environment." >&2
fi

for command_name in cargo node just sqlx pnpm caddy nats-server; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "$command_name is required. Run: just init" >&2
    exit 1
  fi
done

if ! command -v pg_ctl >/dev/null 2>&1 && [ ! -d /usr/lib/postgresql ]; then
  echo "PostgreSQL is required. Run: just init" >&2
  exit 1
fi

if ! command -v valkey-server >/dev/null 2>&1 && ! command -v redis-server >/dev/null 2>&1; then
  echo "Valkey (or Redis) is required. Run: just init" >&2
  exit 1
fi

if [ ! -x "$PROJECT_ROOT/apps/ui/node_modules/.bin/next" ]; then
  echo "UI dependencies are required. Run: just init" >&2
  exit 1
fi

if configure_local_development_encryption_key; then
  echo "Using the private per-prefix local-development encryption key."
fi

cd "$PROJECT_ROOT"
exec just start-all --no-watch
