#!/usr/bin/env bash
# Check that every crate publishing integration plugins is named in the hosted
# catalog, and that plugin registration stays an explicit list.
#
# Why: capabilities and connectors used to register through `inventory::submit!`,
# with `extern crate` lines in everruns-server and everruns-worker forcing the
# integration crates to link so their submissions survived. Presence in a
# registry was a linker side effect — dropping a line removed an integration
# with no compile error, the list was maintained in four places, and a registry
# built by one binary differed from the same registry built by another.
#
# Now each crate publishes `CAPABILITY_PLUGINS` / `CONNECTOR_PLUGINS` consts and
# crates/integrations-catalog names every one. The compiler checks that a named
# crate exists; nothing but this script checks that a crate which publishes
# plugins was named at all.
#
# Used by: scripts/lib/pre-push.sh, the `integration-catalog` CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

CATALOG="crates/integrations-catalog/src/lib.rs"
FAILED=0

if [ ! -f "$CATALOG" ]; then
  echo "Integration catalog missing: $CATALOG"
  exit 1
fi

# 1. Every crate publishing plugin consts must be named in the catalog.
while IFS= read -r manifest; do
  crate_dir="$(dirname "$manifest")"
  # Exclusions, each for a reason rather than convenience:
  #   integrations-catalog  the catalog itself
  #   platform              its own capabilities are registered by platform,
  #                         which the catalog sits above
  #   test-support          test doubles; tests register them explicitly and
  #                         they must never reach a hosted registry
  case "$crate_dir" in
    crates/integrations-catalog|crates/platform|crates/test-support) continue ;;
  esac
  if ! grep -rqE '^\s*pub const (CAPABILITY|CONNECTOR)_PLUGINS' "$crate_dir/src" 2>/dev/null; then
    continue
  fi
  package="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -1)"
  if ! grep -q "crate_name: \"$package\"" "$CATALOG"; then
    echo "Publishes plugins but is not named in the catalog: $package ($crate_dir)"
    echo "  add a CatalogEntry to $CATALOG, or stop publishing the const"
    FAILED=1
  fi
done < <(find crates integrations -mindepth 2 -maxdepth 2 -name Cargo.toml)

# 2. Capability and connector registration stays an explicit list.
if matches=$(grep -rn -B1 'inventory::submit!' --include='*.rs' crates integrations 2>/dev/null \
    | grep -E '(IntegrationPlugin|ConnectorPlugin)' || true); then
  if [ -n "$matches" ]; then
    echo "Capability/connector plugins must be published as consts, not submitted to inventory:"
    printf '%s\n' "$matches"
    FAILED=1
  fi
fi

# 3. No binary force-links an integration crate to make registration happen.
if matches=$(grep -rn '^extern crate everruns' --include='*.rs' crates 2>/dev/null || true); then
  if [ -n "$matches" ]; then
    echo "Force-linking integration crates is what the catalog replaced:"
    printf '%s\n' "$matches"
    FAILED=1
  fi
fi

if [ "$FAILED" -eq 0 ]; then
  echo "Integration catalog is complete."
fi
exit "$FAILED"
