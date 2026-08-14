#!/usr/bin/env bash
# Public-boundary guard (EVE-906): snapshot the published core API, reject
# compatibility owners/concrete backends, and ratchet documentation debt.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

SNAPSHOT="$PROJECT_ROOT/crates/core/public-api.txt"
TOOLCHAIN="${CORE_API_TOOLCHAIN:-nightly}"
MISSING_DOCS_BUDGET=1152
FAILED=0

fail() {
  echo "$1" >&2
  FAILED=1
}

if grep -R -n -E --include='*.rs' \
  '^pub use (everruns_provider|everruns_capability|crate::(compact|driver_registry|error|execution_phase|llm_retry|model|model_profiles|model_spec|provider|runtime_provider|tool_types|typed_id))' \
  crates/core/src; then
  fail "everruns-core publicly re-exports a provider/capability compatibility owner"
fi

if grep -R -n -E --include='*.rs' '\bResolvedModel\b' crates/core/src; then
  fail "everruns-core still contains the legacy credential-bearing ResolvedModel path"
fi

if grep -n -E \
  '^pub mod (in_memory|openai_protocol|openresponses_protocol|model_discovery|runtime_provider|driver_registry|typed_id|tool_types);' \
  crates/core/src/lib.rs; then
  fail "everruns-core exposes a forbidden implementation/compatibility module"
fi

if ! grep -q -E '^#!\[cfg_attr\(not\(test\), forbid\(unsafe_code\)\)\]' crates/core/src/lib.rs; then
  fail "everruns-core must forbid unsafe code in the published library"
fi
if ! grep -q -E '^#!\[deny\(rustdoc::broken_intra_doc_links\)\]' crates/core/src/lib.rs; then
  fail "everruns-core must deny broken rustdoc links"
fi

API_JSON="$PROJECT_ROOT/target/doc/everruns_core.json"
API_CURRENT="$(mktemp "${TMPDIR:-/tmp}/everruns-core-api.XXXXXX")"
DOC_LOG="$(mktemp "${TMPDIR:-/tmp}/everruns-core-docs.XXXXXX")"
trap 'rm -f "$API_CURRENT" "$DOC_LOG"' EXIT

if ! cargo "+$TOOLCHAIN" rustdoc -p everruns-core --lib -- \
  -Z unstable-options --output-format json >"$DOC_LOG" 2>&1; then
  cat "$DOC_LOG" >&2
  fail "everruns-core rustdoc JSON generation failed with $TOOLCHAIN"
else
  {
    echo '# Generated from rustdoc JSON by scripts/lib/check-core-public-api.sh.'
    echo '# path<TAB>kind; update intentionally with UPDATE_CORE_PUBLIC_API=1.'
    jq -r '.paths | to_entries[] | select(.value.path[0] == "everruns_core") | [(.value.path|join("::")), .value.kind] | @tsv' \
      "$API_JSON" | LC_ALL=C sort -u
  } >"$API_CURRENT"

  if [ "${UPDATE_CORE_PUBLIC_API:-0}" = "1" ]; then
    cp "$API_CURRENT" "$SNAPSHOT"
    echo "Updated $SNAPSHOT"
  elif ! diff -u "$SNAPSHOT" "$API_CURRENT"; then
    fail "everruns-core public API changed; audit it, then run UPDATE_CORE_PUBLIC_API=1 bash scripts/lib/check-core-public-api.sh"
  fi
fi

MISSING_DOCS_LOG="$(mktemp "${TMPDIR:-/tmp}/everruns-core-missing-docs.XXXXXX")"
trap 'rm -f "$API_CURRENT" "$DOC_LOG" "$MISSING_DOCS_LOG"' EXIT
if ! RUSTFLAGS='-W missing-docs' cargo check -p everruns-core --lib \
  --message-format=short > /dev/null 2>"$MISSING_DOCS_LOG"; then
  cat "$MISSING_DOCS_LOG" >&2
  fail "everruns-core missing-docs audit did not compile"
else
  MISSING_DOCS_COUNT=$(grep -c -E '^crates/core/.*missing documentation' "$MISSING_DOCS_LOG" || true)
  MISSING_DOCS_COUNT=${MISSING_DOCS_COUNT:-0}
  if [ "$MISSING_DOCS_COUNT" -gt "$MISSING_DOCS_BUDGET" ]; then
    fail "everruns-core missing-doc warnings increased: $MISSING_DOCS_COUNT > $MISSING_DOCS_BUDGET"
  fi
fi

if [ "$FAILED" -ne 0 ]; then
  echo "Core public API guard failed (EVE-906)." >&2
  exit 1
fi

echo "Core public API guard passed: rustdoc snapshot stable; compatibility paths absent; missing-doc debt ${MISSING_DOCS_COUNT}/${MISSING_DOCS_BUDGET}."
