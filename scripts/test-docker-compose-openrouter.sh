#!/usr/bin/env bash
# The quickstart's environment-key option must reach the server, not workers/UI.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUTH_JWT_SECRET=compose-config-test \
SECRETS_ENCRYPTION_KEY=compose-config-test \
WORKER_GRPC_AUTH_TOKEN=compose-config-test \
DEFAULT_OPENROUTER_API_KEY=compose-openrouter-fixture \
SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV=true \
  docker compose -f "$ROOT/examples/docker-compose-full.yaml" config --format json |
  python3 -c '
import json, sys
services = json.load(sys.stdin)["services"]
assert services["server"]["environment"].get("DEFAULT_OPENROUTER_API_KEY") == "compose-openrouter-fixture", "OpenRouter environment key never reaches the server"
assert services["server"]["environment"].get("SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV") == "true", "Environment keys cannot be materialized for the default organization"
for name, service in services.items():
    if name != "server":
        assert "DEFAULT_OPENROUTER_API_KEY" not in service.get("environment", {}), name
print("Compose forwards the OpenRouter default credential only to the server")
'

# An omitted or empty opt-in must keep host credentials out of the default org.
AUTH_JWT_SECRET=compose-config-test \
SECRETS_ENCRYPTION_KEY=compose-config-test \
WORKER_GRPC_AUTH_TOKEN=compose-config-test \
SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV= \
  docker compose -f "$ROOT/examples/docker-compose-full.yaml" config --format json |
  python3 -c '
import json, sys
server = json.load(sys.stdin)["services"]["server"]
assert server["environment"]["SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV"] == "false", "Default provider-key seeding must remain disabled"
print("Compose leaves provider-key materialization disabled without explicit opt-in")
'

# Guard rails must fail before opening a browser, creating containers, or saving a key.
node --check "$ROOT/scripts/e2e-compose-openrouter.mjs"
python3 - "$ROOT" <<'PY'
import os, pathlib, subprocess, sys
script = pathlib.Path(sys.argv[1]) / 'scripts/e2e-compose-openrouter.mjs'
env = os.environ.copy()
env.pop('OPENROUTER_API_KEY', None)
env.pop('EVERRUNS_E2E_ALLOW_PROVIDER_STORAGE', None)
missing = subprocess.run(['node', str(script)], env=env, capture_output=True, text=True)
assert missing.returncode != 0
assert 'OPENROUTER_API_KEY is required' in missing.stderr
fixture = 'private-inference-fixture-never-log'
env['OPENROUTER_API_KEY'] = fixture
consent = subprocess.run(['node', str(script)], env=env, capture_output=True, text=True)
assert consent.returncode != 0
assert 'Explicit consent' in consent.stderr
assert fixture not in consent.stdout + consent.stderr
print('Real E2E refuses missing key or missing storage consent without exposing the key')
PY
