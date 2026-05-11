#!/usr/bin/env bash
# Reject rewrites of already-merged SQL migrations.
# New migrations are append-only: add the next NNN_*.sql file instead.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BASE_REF="${MIGRATION_IMMUTABILITY_BASE_REF:-origin/main}"
MIGRATION_PATHSPEC="crates/server/migrations/*.sql"

if ! git rev-parse --verify -q "$BASE_REF" >/dev/null; then
  if git remote get-url origin >/dev/null 2>&1; then
    git fetch --quiet origin main:refs/remotes/origin/main || true
  fi
fi

if ! git rev-parse --verify -q "$BASE_REF" >/dev/null; then
  echo "Could not resolve migration immutability base ref: $BASE_REF" >&2
  echo "Fetch origin/main or set MIGRATION_IMMUTABILITY_BASE_REF." >&2
  exit 1
fi

MERGE_BASE="$(git merge-base HEAD "$BASE_REF")"
readonly MERGE_BASE

if ! violations="$(
  git diff --name-status --find-renames --diff-filter=MDR \
    "$MERGE_BASE" HEAD -- "$MIGRATION_PATHSPEC"
)"; then
  echo "Failed to compare migration history against $BASE_REF." >&2
  exit 1
fi

if [ -n "$violations" ]; then
  # Approved repairs for merged duplicate migration versions. Keep each
  # exception scoped to the exact rename that restores sequential ordering.
  violations="$(printf '%s\n' "$violations" | grep -v $'^R100\tcrates/server/migrations/027_session_stats_indexes.sql\tcrates/server/migrations/028_session_stats_indexes.sql$' || true)"
  violations="$(printf '%s\n' "$violations" | grep -v $'^R100\tcrates/server/migrations/041_reporting_saved_reports_exports.sql\tcrates/server/migrations/042_reporting_saved_reports_exports.sql$' || true)"
fi

if [ -n "$violations" ]; then
  echo "Existing migrations are immutable once merged to main." >&2
  echo "Add a new sequential migration instead of modifying, deleting, or renaming an existing one." >&2
  echo "" >&2
  echo "$violations" >&2
  exit 1
fi

echo "migration history immutable"
