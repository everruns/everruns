#!/usr/bin/env bash
# Verify a release-prep PR squashed all feature migrations.
#
# Run this when a PR bumps `[workspace.package].version` (i.e., the PR is a
# release prep). The CI job in `.github/workflows/ci.yml` triggers it on that
# signal, so feature-named migrations cannot leak into a release tag the way
# `016_eval_case_result_metadata.sql` and `017_eval_artifacts.sql` did in
# v0.8.15. See `specs/release-process.md` ("Migration Squashing").
#
# Rule: every file under crates/server/migrations/ matching `[0-9]+*.sql` must
# be either a foundational baseline migration or a `NNN_vX.Y.Z.sql` release
# migration. Feature-named migrations are forbidden in a release-prep PR.

set -euo pipefail

ROOT="${1:-$(git rev-parse --show-toplevel)}"
MIG_DIR="$ROOT/crates/server/migrations"

if [ ! -d "$MIG_DIR" ]; then
  echo "no migrations dir at $MIG_DIR — nothing to check"
  exit 0
fi

# Foundational baseline migrations exist before the version-named convention
# was adopted. They are the only allowed non-version names.
FOUNDATIONAL=("001_base_schema.sql" "002_durable_execution.sql")

bad=()
shopt -s nullglob
for f in "$MIG_DIR"/[0-9]*.sql; do
  name=$(basename "$f")
  is_foundational=false
  for fnd in "${FOUNDATIONAL[@]}"; do
    if [ "$name" = "$fnd" ]; then
      is_foundational=true
      break
    fi
  done
  if $is_foundational; then
    continue
  fi
  if [[ "$name" =~ ^[0-9]{3}_v[0-9]+\.[0-9]+\.[0-9]+\.sql$ ]]; then
    continue
  fi
  bad+=("$name")
done

if [ ${#bad[@]} -gt 0 ]; then
  {
    echo "ERROR: release prep left these feature migrations un-squashed:"
    printf '  - %s\n' "${bad[@]}"
    echo
    echo "Fold them into the release migration per specs/release-process.md"
    echo "(Migration Squashing). The /prepare-release command does this; if you"
    echo "ran it manually, re-run the squash step now."
  } >&2
  exit 1
fi

echo "OK: every migration is either foundational or NNN_vX.Y.Z.sql"
