#!/usr/bin/env bash
# Exercise scripts/scan_actions_log_secrets.py against log fixtures.
#
# The fixtures are the measurement that matters. A log scanner that cries wolf
# gets muted, so the benign half deliberately carries this repository's own CI
# values -- the hardcoded POSTGRES_PASSWORD, the throwaway
# SECRETS_ENCRYPTION_KEY -- and the leak half carries the shape that actually
# reached a public log in run 35305647248.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

expect() {
  local name="$1" expected="$2" file="$3"
  local output status
  set +e
  output="$(python3 scripts/scan_actions_log_secrets.py "$file" 2>&1)"
  status=$?
  set -e
  if [ "$expected" = "flag" ] && [ "$status" -eq 0 ]; then
    echo "FAIL $name: expected a finding, got none"
    failures=$((failures + 1))
  elif [ "$expected" = "quiet" ] && [ "$status" -ne 0 ]; then
    echo "FAIL $name: expected no finding, got:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
  else
    echo "ok   $name"
  fi
  # The scanner runs in a public log; a value must never reach its output.
  if grep -qE 'apikey_[A-Za-z0-9]{24,}|sk-ant-[A-Za-z0-9]|dp\.st\.|ghp_[A-Za-z0-9]{20,}' <<<"$output"; then
    echo "FAIL $name: scanner echoed a credential value into its own output"
    failures=$((failures + 1))
  fi
}

# --- leaks the scanner must catch -------------------------------------------

cat > "$WORK/env-block.log" <<'LOG'
2026-09-18T04:06:16.5269100Z env:
2026-09-18T04:06:16.5270198Z   DOPPLER_TOKEN: ***
2026-09-18T04:06:16.5271248Z   TYPESAFE_API_KEY: apikey_EXAMPLEEXAMPLEEXAMPLEEXAMPLE0000_EXAMPLE1111
2026-09-18T04:06:16.5272756Z ##[endgroup]
LOG
expect "env-block: the run 35305647248 shape" flag "$WORK/env-block.log"

# Credential-shaped literals are assembled here rather than written out. A
# committed file full of them trips GitHub push protection -- it blocked this
# very file on the Slack sample -- and getting into the habit of clicking
# "allow secret" is a worse outcome than a little indirection. Each split falls
# inside the prefix its rule matches on, so the literal never appears whole.
stamp="2026-09-18T04:06:16Z"
{
  echo "$stamp calling with sk""-proj-EXAMPLEEXAMPLEEXAMPLEEXAMPLE0000"
  echo "$stamp header x-api-key: sk""-ant-api03-EXAMPLEEXAMPLEEXAMPLEEXAMPLE"
  echo "$stamp remote rejected gh""p_EXAMPLEEXAMPLEEXAMPLEEXAMPLEEXAMPLE12"
  echo "$stamp DOPPLER_TOKEN=dp"".st.dev.EXAMPLEEXAMPLEEXAMPLEEXAMPLE00"
  echo "$stamp slack said xox""b-0000000000-EXAMPLEEXAMPLEE"
  echo "$stamp profile AKI""AIOSFODNN7EXAMPLE"
  echo "$stamp gitlab glp""at-EXAMPLEEXAMPLEEXAMPLEEXAMPLE00"
  echo "$stamp authorization: Bearer evr""_pat_EXAMPLEEXAMPLE0000"
} > "$WORK/prefixes.log"
expect "prefix rules: eight credential formats" flag "$WORK/prefixes.log"

# A secret outside an env block, which the static workflow guard cannot see.
cat > "$WORK/echoed-by-a-test.log" <<'LOG'
2026-09-18T04:06:16Z test failed: upstream rejected apikey_EXAMPLEEXAMPLEEXAMPLEEXAMPLE0000
LOG
expect "prefix: a credential a failing test echoed" flag "$WORK/echoed-by-a-test.log"

# The allowlist is keyed on the value, so the same variable carrying anything
# else still reports. Without this the scanner would go blind on rotation.
cat > "$WORK/rotated-value.log" <<'LOG'
2026-09-18T04:06:16Z env:
2026-09-18T04:06:16Z   SECRETS_ENCRYPTION_KEY: kek-v1:EXAMPLEEXAMPLEEXAMPLEEXAMPLEEXAMPLEEXAMPLE0=
2026-09-18T04:06:16Z ##[endgroup]
LOG
expect "env-block: an allowlisted name with an unknown value" flag "$WORK/rotated-value.log"

# --- benign logs the scanner must stay quiet on -----------------------------

# The local-development encryption key is read out of ci.yml rather than
# written here: scripts/test-agent-dev-startup.sh asserts that literal appears
# in exactly one file under scripts/, and a second copy fails it. Reading it
# also makes the fixture prove the real thing -- that the allowlist is derived
# from the workflows -- instead of restating a value that could drift.
local_dev_key="$(grep -ohE 'SECRETS_ENCRYPTION_KEY: kek-v1:\S+' .github/workflows/ci.yml \
  | head -1 | sed 's/^SECRETS_ENCRYPTION_KEY: //')"
if [ -z "$local_dev_key" ]; then
  echo "FAIL: no SECRETS_ENCRYPTION_KEY literal found in ci.yml to build the fixture from"
  exit 1
fi
{
  echo "$stamp env:"
  echo "$stamp   POSTGRES_PASSWORD: everruns"
  echo "$stamp   SECRETS_ENCRYPTION_KEY: ${local_dev_key}"
  echo "$stamp   WORKER_GRPC_AUTH_TOKEN: test-grpc-token-for-ci"
  echo "$stamp   EVERRUNS_API_KEY: sdk-compat-test-key"
  echo "$stamp   STORAGE_S3_SECRET_ACCESS_KEY: everruns-secret"
  echo "$stamp   DOPPLER_TOKEN: ***"
  echo "$stamp ##[endgroup]"
} > "$WORK/repo-ci-values.log"
expect "env-block: this repo's committed CI values" quiet "$WORK/repo-ci-values.log"

# Documented API examples are the other way a credential shape reaches a public
# log: a contract test that fails dumps the whole catalog into its assertion
# output, examples and all, as run 35396473086 did with an `sk-` prefixed MCP
# placeholder. Catching that here -- by running the real scanner over the
# committed examples -- fixes it at the source instead of leaving an hourly
# alarm nobody can act on. The reported line number indexes the sorted example
# list, so `jq -r '[.. | objects | select(has("example")) | .example
# | select(type == "string")] | unique[]' docs/api/openapi.json | sed -n '<n>p'`
# names the offending placeholder.
jq -r '[.. | objects | select(has("example")) | .example | select(type == "string")]
       | unique[]' docs/api/openapi.json \
  | sed "s/^/$stamp /" > "$WORK/openapi-examples.log"
if [ ! -s "$WORK/openapi-examples.log" ]; then
  echo "FAIL: no string examples found in docs/api/openapi.json to build the fixture from"
  exit 1
fi
expect "documented OpenAPI examples carry no credential shape" quiet "$WORK/openapi-examples.log"

cat > "$WORK/ordinary-build.log" <<'LOG'
2026-09-18T04:06:16Z env:
2026-09-18T04:06:16Z   CACHE_KEY: debug-ubuntu-latest
2026-09-18T04:06:16Z   CARGO_HOME: /home/runner/.cargo
2026-09-18T04:06:16Z   CARGO_TERM_COLOR: always
2026-09-18T04:06:16Z ##[endgroup]
2026-09-18T04:06:16Z      Running tests/classifier.rs (target/debug/deps/classifier-768afccfcf4ba8b9)
2026-09-18T04:06:16Z pulled sha256:9b2c1f0a7d3e4c5b6a8f9e0d1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6978859473
2026-09-18T04:06:16Z test debug_never_renders_the_api_key ... ok
LOG
expect "build noise: cache keys, dep hashes, digests" quiet "$WORK/ordinary-build.log"

if [ "$failures" -ne 0 ]; then
  echo "$failures fixture(s) failed"
  exit 1
fi
echo "actions-log secret scan: all fixtures pass"
