#!/usr/bin/env bash
# Architecture guard (EVE-876): telemetry initialization and exporter
# implementations live behind `everruns-core/observability`.
# The neutral kernel owns only the observability contracts — the
# `EventListener` trait, event types, and the gen-AI span conventions:
#
# 1. `everruns-core` sources must not reference the OpenTelemetry SDK,
#    OTLP exporter, tracing-subscriber layers, or the tracing-opentelemetry
#    bridge. (tracing-subscriber remains a core dev-dependency for tests,
#    so the source guard covers src/ only.)
# 2. `everruns-core` must carry no exporter crate on its normal or build
#    dependency edges, so `cargo tree -p everruns-core` stays clean.
# 3. Framework builds (the `everruns` facade) and provider-only crates must
#    ship no OTLP/exporter subtree — default Framework builds stay offline.
# 4. Only `everruns-core host` may declare the exporter dependencies among library
#    crates; the feature stays off in default Framework and provider builds.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Keeps cargo's stderr, so "the guard could not run" never looks like
# "the guard found a violation". See guard-cargo.sh.
source "$SCRIPT_DIR/guard-cargo.sh"
source "$SCRIPT_DIR/core-feature-modules.sh"

FAILED=0

if [ -e crates/observability/Cargo.toml ] || [ ! -e crates/core/src/host/observability/mod.rs ]; then
  echo "Observability must remain an opt-in everruns-core host module, not a standalone crate"
  FAILED=1
fi

EXPORTER_CRATES_TREE='^(opentelemetry|opentelemetry_sdk|opentelemetry-otlp|opentelemetry-http|opentelemetry-proto|tracing-opentelemetry) '

# 1. Core sources: no exporter/subscriber references (contracts only).
SOURCE_PATTERN='(opentelemetry|opentelemetry_sdk|opentelemetry_otlp|tracing_opentelemetry|tracing_subscriber)::'
if matches=$(core_kernel_source_files | xargs grep -nE "$SOURCE_PATTERN" 2>/dev/null); then
  echo "everruns-core sources must not reference exporter/subscriber crates (EVE-876):"
  echo "$matches"
  FAILED=1
fi

# 2. Core: no exporter crate on normal/build edges.
CORE_TREE=$(guard_cargo_tree -p everruns-core --edges normal,build --prefix none)
if echo "$CORE_TREE" | grep -qE "$EXPORTER_CRATES_TREE"; then
  echo "everruns-core must not depend on OpenTelemetry/OTLP exporter crates:"
  echo "$CORE_TREE" | grep -E "$EXPORTER_CRATES_TREE" | sort -u
  FAILED=1
fi

# 3. Framework facade and provider-only crates: shipped dependency tree free
#    of the exporter subtree. Framework/host defaults subsume no-default;
#    drivers enable every vendor feature, matching the retired shim coverage.
CLEAN_CRATES=(
  everruns
  everruns-drivers
)
for crate in "${CLEAN_CRATES[@]}"; do
  if [ "$crate" = everruns-drivers ]; then
    tree=$(guard_cargo_tree -p "$crate" --all-features --edges normal --prefix none)
  else
    tree=$(guard_cargo_tree -p "$crate" --edges normal --prefix none)
  fi
  if echo "$tree" | grep -qE "$EXPORTER_CRATES_TREE"; then
    echo "$crate must not ship OpenTelemetry/OTLP exporter crates in its normal dependency tree:"
    echo "$tree" | grep -E "$EXPORTER_CRATES_TREE" | sort -u
    FAILED=1
  fi
done

# 4. Exporter dependency declarations live in everruns-core host only
#    (binaries get them transitively; app/bin crates may not re-declare them).
if matches=$(grep -rnE '^(opentelemetry|opentelemetry_sdk|opentelemetry-otlp|tracing-opentelemetry)[[:space:]]*[.=]' \
  crates/*/Cargo.toml integrations/*/Cargo.toml 2>/dev/null | grep -v '^crates/core/Cargo.toml'); then
  echo "Exporter dependencies are owned by everruns-core host's observability feature:"
  echo "$matches"
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Observability isolation guard failed. Telemetry init and exporters belong in everruns-core/observability."
  exit 1
fi

echo "Observability isolation guard passed: core carries contracts only; exporter deps stay behind everruns-core/observability."
