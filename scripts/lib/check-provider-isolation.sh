#!/usr/bin/env bash
# Architecture guard (EVE-874): official wire-protocol provider crates build on
# the provider SPI (`everruns-provider`) and the neutral capability contract
# alone — never on the monolithic kernel or product composition crates:
#
# 1. Provider crate sources (src/ AND tests/ — dev code included) must not
#    import everruns_core, everruns_host, everruns_platform, or
#    everruns_server.
# 2. Provider crate manifests must not declare a direct everruns-core /
#    everruns-host / everruns-platform / everruns-server dependency on any
#    edge kind (normal, build, or dev).
# 3. `cargo tree` for each provider crate must be free of those crates on
#    every edge kind, so provider-only builds never compile the kernel.
# 4. Provider-only shipped trees must not pull core's heavy feature subtrees:
#    no sqlx, utoipa, inventory, axum, or tonic in normal edges. (The
#    provider SPI's `sqlx`/`openapi` features are intentional, documented
#    opt-ins that provider crates leave off.)
# 5. Server and platform code never resolves credentials from the process
#    environment (the fail-closed Key Resolution Contract in
#    knowledge/foundations/llm-drivers.md): no EnvCredentialProvider, no
#    `from_env`, no `provider_from_env` on any org-scoped path. Drivers may
#    *declare* their vendor's variable names — declaring reads nothing — but
#    only standalone/CLI/dev entrypoints may pair a declaration with a lookup.
# 6. Drivers never read credential env vars themselves; the names they declare
#    are inert data.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0

DRIVER_LAYOUT_NAMES=(
  anthropic
  bedrock
  fireworks
  gemini
  llmsim
  mai
  meta
  openai
  openrouter
)
PROVIDER_DIRS=(
  crates/drivers/openai
  crates/drivers/anthropic
  crates/drivers/openrouter
  crates/drivers/gemini
  crates/drivers/bedrock
  crates/drivers/mai
  crates/drivers/fireworks
  crates/drivers/meta
)
PROVIDER_CRATES=(
  everruns-openai
  everruns-anthropic
  everruns-openrouter
  everruns-gemini
  everruns-bedrock
  everruns-mai
  everruns-fireworks
  everruns-meta
)
FORBIDDEN_TREE='^(everruns-core|everruns-host|everruns-platform|everruns-server) '
HEAVY_TREE='^(sqlx|utoipa|inventory|axum|tonic) '

# Keep model drivers physically grouped without turning the directory into a
# package or shared version boundary.
for driver in "${DRIVER_LAYOUT_NAMES[@]}"; do
  if [ ! -f "crates/drivers/$driver/Cargo.toml" ]; then
    echo "Missing driver package: crates/drivers/$driver/Cargo.toml"
    FAILED=1
  fi
  if [ -e "crates/$driver" ]; then
    echo "Driver packages belong under crates/drivers, not crates/$driver"
    FAILED=1
  fi
done

# 1. No kernel/product imports anywhere in provider crates (tests included —
#    dev-only coupling is exactly what EVE-874 removed).
SOURCE_PATTERN='(everruns_core|everruns_host|everruns_platform|everruns_server)::'
if matches=$(grep -rnE "$SOURCE_PATTERN" "${PROVIDER_DIRS[@]}" --include='*.rs' 2>/dev/null); then
  echo "Provider protocol crates must not import core/host/platform/server (EVE-874):"
  echo "$matches"
  FAILED=1
fi

# 2. Manifests: no direct dependency declarations on the forbidden crates
#    (any section — [dependencies], [dev-dependencies], [build-dependencies]).
for dir in "${PROVIDER_DIRS[@]}"; do
  if matches=$(grep -nE '^[[:space:]]*everruns-(core|host|platform|server)[[:space:]]*[.=]' "$dir/Cargo.toml" 2>/dev/null); then
    echo "$dir/Cargo.toml must not declare a core/host/platform/server dependency (EVE-874):"
    echo "$matches"
    FAILED=1
  fi
done

# 3. Dependency trees: forbidden crates absent on every edge kind, so
#    `cargo test -p <provider>` never builds the kernel.
for crate in "${PROVIDER_CRATES[@]}"; do
  tree=$(cargo tree -p "$crate" --edges normal,build,dev --prefix none 2>/dev/null)
  if echo "$tree" | grep -qE "$FORBIDDEN_TREE"; then
    echo "$crate must not depend on core/host/platform/server (any edge kind):"
    echo "$tree" | grep -E "$FORBIDDEN_TREE" | sort -u
    FAILED=1
  fi
done

# 4. Shipped (normal-edge) trees: no heavy core feature subtree leaks into
#    provider-only builds.
for crate in "${PROVIDER_CRATES[@]}"; do
  tree=$(cargo tree -p "$crate" --edges normal --prefix none 2>/dev/null)
  if echo "$tree" | grep -qE "$HEAVY_TREE"; then
    echo "$crate must not ship heavy core feature subtrees (sqlx/utoipa/inventory/axum/tonic):"
    echo "$tree" | grep -E "$HEAVY_TREE" | sort -u
    FAILED=1
  fi
done

# 5. Org-scoped code never pairs a driver's declared variable names with a real
#    environment lookup. The names are the drivers'; the lookup belongs to
#    standalone/CLI/dev entrypoints only.
# Credential-specific only: `everruns_<driver>::from_env(` is a provider
# constructor, while unrelated `Type::from_env` helpers (deployment feature
# flags) are not credential resolution and stay allowed.
ENV_CREDENTIAL_PATTERN='(EnvCredentialProvider|provider_from_env|everruns_[a-z_]+::from_env[[:space:]]*\()'
SERVER_DIRS=(crates/server/src crates/platform/src)
for dir in "${SERVER_DIRS[@]}"; do
  if matches=$(grep -rnE "$ENV_CREDENTIAL_PATTERN" "$dir" --include='*.rs' 2>/dev/null); then
    echo "Server/platform code must not resolve credentials from the environment:"
    echo "$matches"
    FAILED=1
  fi
done

# 6. Drivers declare credential variable names; they never read them. A driver
#    reading its own key from env is the shape the contract has always banned.
# A real lookup is `env::var("NAME")`. A declaration is `.env("NAME")`, which
# reads nothing, so the `var` is what distinguishes them.
CREDENTIAL_ENV_READ='env::var(_os)?[[:space:]]*\([[:space:]]*"[A-Z_]*(API_KEY|SECRET|TOKEN|BASE_URL|ENDPOINT|ACCESS_KEY)'
for dir in "${PROVIDER_DIRS[@]}"; do
  if matches=$(grep -rnE "$CREDENTIAL_ENV_READ" "$dir/src" --include='*.rs' 2>/dev/null); then
    echo "$dir must declare its credential variables, never read them:"
    echo "$matches"
    FAILED=1
  fi
done

if [ "$FAILED" -ne 0 ]; then
  echo "Provider isolation guard failed. Wire-protocol crates build on everruns-provider alone (EVE-874), and credentials never reach org-scoped paths from the environment."
  exit 1
fi

echo "Provider isolation guard passed: provider crates depend on the provider SPI, not core/host/platform/server; credential env resolution stays out of server/platform."
