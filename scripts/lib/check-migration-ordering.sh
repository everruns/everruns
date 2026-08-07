#!/usr/bin/env bash
# Check that migration files in crates/server/migrations/ are strictly
# sequential (001, 002, ..., N) with no gaps and no duplicates, and that
# every .sql file uses the required NNN_<description>.sql naming
# (zero-padded 3-digit prefix).
#
# Spec: knowledge/operations/migrations.md ("Sequential Ordering Validation")
# Used by: scripts/lib/pre-push.sh, scripts/lib/pre-pr.sh, /ship skill.
#
# Exits 0 on success, 1 on violation. Always exits non-zero (never silently
# skips) when the migrations directory is missing or empty, since this repo
# is expected to always carry migrations.
#
# Usage: bash scripts/lib/check-migration-ordering.sh

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MIGRATION_DIR="$PROJECT_ROOT/crates/server/migrations"

if [ ! -d "$MIGRATION_DIR" ]; then
  echo "error: migration directory not found: $MIGRATION_DIR"
  exit 1
fi

shopt -s nullglob
sql_files=("$MIGRATION_DIR"/*.sql)
shopt -u nullglob

if [ "${#sql_files[@]}" -eq 0 ]; then
  echo "error: no .sql files found in $MIGRATION_DIR"
  exit 1
fi

EXPECTED=1
LAST_FILE=""
for f in "${sql_files[@]}"; do
  name=$(basename "$f")
  if ! [[ "$name" =~ ^([0-9]{3})_.+\.sql$ ]]; then
    echo "error: $name does not match required NNN_<description>.sql format"
    echo "fix: use a zero-padded 3-digit prefix (see knowledge/operations/migrations.md)"
    exit 1
  fi
  prefix="${BASH_REMATCH[1]}"
  num=$((10#$prefix))
  if [ "$num" != "$EXPECTED" ]; then
    echo "migration ordering broken at $name: expected $(printf '%03d' "$EXPECTED"), got $prefix"
    if [ -n "$LAST_FILE" ]; then
      echo "previous file: $(basename "$LAST_FILE")"
    fi
    echo "fix: renumber to the next available number (see knowledge/operations/migrations.md)"
    exit 1
  fi
  LAST_FILE="$f"
  EXPECTED=$((EXPECTED + 1))
done

echo "migrations sequential (001..$(printf '%03d' $((EXPECTED - 1))))"
