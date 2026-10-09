#!/usr/bin/env bash
# Layering guard: the server's domain, storage, services and records layers
# must not reach up into the HTTP layer (`crate::api`).
#
# Why: `api` is the transport. Shared shapes it used to own (the error body,
# list wrapper and pagination, validation, channel ingress resolution, metric
# names, request/response DTOs used by domain services, the command catalog)
# now live in a layer-neutral home, and `api` re-exports them so OpenAPI schema
# names, JSON shapes and the `everruns_server::api::*` paths SaaS imports stay
# unchanged. A lower layer importing `api` again re-creates the cycle this
# split removed: a domain change starts depending on HTTP handler modules, and
# moving a handler breaks a service.
#
# Fix a violation by moving the shared item down (into `records/`, a domain's
# `types.rs`, `services/`, or a crate-root module such as `metrics_names`) and
# re-exporting it from the `api` module with `pub use`, never by importing
# `api` from below.
#
# The rule is absolute: there is no allowlist, so every reference fails,
# including one in a comment or a test. Name the HTTP layer as `api::...` in
# prose instead of a `crate::api` path.
#
# Caught forms, in any `.rs` file under the guarded trees:
#   crate::api::...            crate::api (bare)
#   super::super::api::...     (any depth of super::)
#   everruns_server::api::...  (an absolute path from inside the crate's tests)
#   use crate::{ ..., api, ... };   (grouped, single- or multi-line)
#
# Usage: check-server-api-layering.sh [SERVER_SRC_DIR]
#   SERVER_SRC_DIR defaults to crates/server/src (used by the shell test).
#
# Used by: scripts/lib/pre-push.sh, the `server-api-layering` CI job.
# Exits 0 on success, 1 on violation. Never silently skips.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SRC="${1:-crates/server/src}"
LAYERS=(domains storage services records)

TREES=()
for layer in "${LAYERS[@]}"; do
  if [ ! -d "$SRC/$layer" ]; then
    echo "Server API layering guard: missing $SRC/$layer; the guard would check nothing." >&2
    exit 1
  fi
  TREES+=("$SRC/$layer")
done

PATTERN='crate::api\b|(super::)+api\b|everruns_server::api\b'

FAILED=0

if matches=$(grep -rnE "$PATTERN" "${TREES[@]}" --include='*.rs'); then
  echo "Lower server layers must not reference the HTTP layer (crate::api):"
  echo "$matches" | sed 's/^/  /'
  FAILED=1
fi

# Grouped imports (`use crate::{api, ...}`), which can span lines.
grouped=$(find "${TREES[@]}" -type f -name '*.rs' -print0 \
  | xargs -0 perl -0ne '
      while (/\buse\s+crate::\{([^;]*?)\};/gs) {
        if ($1 =~ /(?<![\w:])api\b/) { print "$ARGV\n"; last; }
      }' \
  | sort -u)
if [ -n "$grouped" ]; then
  echo "Lower server layers must not import the HTTP layer via a grouped use crate::{api, ...}:"
  echo "$grouped" | sed 's/^/  /'
  FAILED=1
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Server API layering guard failed. Move the shared item out of api and re-export it there with pub use."
  exit 1
fi

files=$(find "${TREES[@]}" -type f -name '*.rs' | wc -l | tr -d ' ')
echo "Server API layering guard passed: $files files under ${LAYERS[*]} do not reference crate::api."
