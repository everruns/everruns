#!/usr/bin/env bash
# Architecture guard (EVE-874): official wire-protocol provider crates build on
# the provider SPI (`everruns-contracts`) and the neutral capability contract
# alone — never on the monolithic kernel or product composition crates:
#
# 1. Provider crate sources (src/ AND tests/ — dev code included) must not
#    import everruns_core, everruns_host, everruns_platform, or
#    everruns_server.
# 2. Provider crate manifests must not declare a direct everruns-core /
#    everruns-host / everruns-platform / everruns-server dependency on any
#    edge kind (normal, build, or dev).
# 3. `cargo tree` with every vendor feature enabled must be free of those crates on
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

# Keeps cargo's stderr, so "the guard could not run" never looks like
# "the guard found a violation". See guard-cargo.sh.
source "$SCRIPT_DIR/guard-cargo.sh"

FAILED=0

DRIVER_LAYOUT_NAMES=(
  llmsim
  drivers
)
PROVIDER_DIRS=(
  crates/contracts
  crates/drivers/drivers
)
PROVIDER_CRATES=(
  everruns-contracts
  everruns-drivers
)
# Retired shim crates must not return as parallel implementations.
RETIRED_DRIVER_NAMES=(anthropic bedrock fireworks gemini mai meta openai openrouter)
for driver in "${RETIRED_DRIVER_NAMES[@]}"; do
  if [ -e "crates/drivers/$driver" ] || [ -e "crates/$driver" ]; then
    echo "Retired vendor package must remain a module of everruns-drivers: $driver"
    FAILED=1
  fi
done
FORBIDDEN_TREE='^(everruns-core|everruns-host|everruns-platform|everruns-capabilities|everruns-server) '
HEAVY_TREE='^(sqlx|utoipa|inventory|axum|tonic) '

# Keep model drivers physically grouped without turning the directory into a
# package or shared version boundary.
for driver in "${DRIVER_LAYOUT_NAMES[@]}"; do
  if [ ! -f "crates/drivers/$driver/Cargo.toml" ]; then
    echo "Missing driver package: crates/drivers/$driver/Cargo.toml"
    FAILED=1
  fi
  # `crates/drivers` is the grouping directory itself, so the multi-vendor
  # package that shares its name has no shadow path to check.
  if [ "$driver" != "drivers" ] && [ -e "crates/$driver" ]; then
    echo "Driver packages belong under crates/drivers, not crates/$driver"
    FAILED=1
  fi
done

# 1. No kernel/product imports anywhere in provider crates (tests included —
#    dev-only coupling is exactly what EVE-874 removed).
SOURCE_PATTERN='(everruns_core|everruns_host|everruns_platform|everruns_capabilities|everruns_server)::'
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
  tree=$(guard_cargo_tree -p "$crate" --all-features --edges normal,build,dev --prefix none)
  if echo "$tree" | grep -qE "$FORBIDDEN_TREE"; then
    echo "$crate must not depend on core/host/platform/server (any edge kind):"
    echo "$tree" | grep -E "$FORBIDDEN_TREE" | sort -u
    FAILED=1
  fi
done

# 4. Shipped (normal-edge) trees with every vendor feature enabled: no heavy
#    core feature subtree leaks into provider-only builds.
for crate in "${PROVIDER_CRATES[@]}"; do
  wire_features=(--all-features)
  if [ "$crate" = "everruns-contracts" ]; then
    # Contracts also own optional host codecs and sandbox registration. Exercise
    # every wire transport here; those host opt-ins are not driver dependencies.
    wire_features=(--no-default-features --features http,definition,responses-websocket)
  fi
  tree=$(guard_cargo_tree -p "$crate" "${wire_features[@]}" --edges normal --prefix none)
  if echo "$tree" | grep -qE "$HEAVY_TREE"; then
    echo "$crate must not ship heavy core feature subtrees (sqlx/utoipa/inventory/axum/tonic):"
    echo "$tree" | grep -E "$HEAVY_TREE" | sort -u
    FAILED=1
  fi
done

# Extension implementations depend on contracts rather than the hosted product.
# Include optional/build/dev declarations: an integration test must not quietly
# pull control-plane records back into a library host's dependency graph.
for manifest in integrations/*/Cargo.toml crates/integrations/Cargo.toml crates/ard/Cargo.toml crates/turbopuffer/Cargo.toml; do
  if matches=$(grep -nE '^[[:space:]]*everruns-(platform|capabilities)[[:space:]]*[.=]' "$manifest"); then
    echo "$manifest must use everruns-contracts extension SPIs, never platform:"
    echo "$matches"
    FAILED=1
  fi
done
if matches=$(grep -rnE 'everruns_(platform|capabilities)::' integrations crates/integrations crates/ard crates/turbopuffer --include='*.rs' 2>/dev/null); then
  echo "Extension implementations must not reference platform records:"
  echo "$matches"
  FAILED=1
fi

# Integrations implement the runtime SPI in `everruns_contracts::runtime` and
# never ship against core. Dev edges stay allowed: tests may drive a core host.
# The tree check covers optional, renamed, and transitive edges alike.
# crates/integrations is the feature-module crate new integrations land in.
integration_packages=()
for manifest in integrations/*/Cargo.toml crates/integrations/Cargo.toml; do
  integration_packages+=(-p "$(sed -nE 's/^name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$manifest" | head -n1)")
done
tree=$(guard_cargo_tree "${integration_packages[@]}" --all-features --edges normal,build --prefix none)
if echo "$tree" | grep -qE '^everruns-core '; then
  echo "Integrations must depend on everruns-contracts, never everruns-core (normal or build edge):"
  echo "$tree" | grep -E '^everruns-core ' | sort -u
  FAILED=1
fi

# 5. Org-scoped code never pairs a driver's declared variable names with a real
#    environment lookup. The names are the drivers'; the lookup belongs to
#    standalone/CLI/dev entrypoints only.
# Credential-specific only: `everruns_drivers::<vendor>::from_env(` (or the
# deprecated `everruns_<vendor>::from_env(` shim) is a provider
# constructor, while unrelated `Type::from_env` helpers (deployment feature
# flags) are not credential resolution and stay allowed.
# The AWS default credential chain (everruns-drivers `bedrock-default-credentials`:
# `BedrockAuth::default_chain`, `provider_from_default_chain`, the facade's
# `Bedrock::default_chain`) is ambient-credential resolution too: on a server
# it would sign tenant calls with the host's own IAM identity.
ENV_CREDENTIAL_PATTERN='(EnvCredentialProvider|provider_from_env|everruns_[a-z_]+(::[a-z_]+)?::from_env[[:space:]]*\(|default_chain[[:space:]]*\()'
SERVER_DIRS=(crates/server/src crates/capabilities/src crates/worker/src)
for manifest in crates/server/Cargo.toml crates/capabilities/Cargo.toml crates/worker/Cargo.toml; do
  if matches=$(grep -nE 'default-credentials|everruns/bedrock' "$manifest" 2>/dev/null); then
    echo "$manifest must not enable the AWS default credential chain:"
    echo "$matches"
    FAILED=1
  fi
done
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
  echo "Provider isolation guard failed. Wire-protocol crates build on everruns-contracts alone (EVE-874), and credentials never reach org-scoped paths from the environment."
  exit 1
fi

echo "Provider isolation guard passed: provider crates depend on the provider SPI, not core/host/platform/server; credential env resolution stays out of server/platform."
