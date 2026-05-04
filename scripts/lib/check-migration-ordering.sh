#!/usr/bin/env bash
# Check that migration files in crates/server/migrations/ are strictly
# sequential (001, 002, ..., N) with no gaps and no duplicates.
#
# Spec: specs/migrations.md ("Sequential Ordering Validation")
# Used by: scripts/lib/pre-push.sh, scripts/lib/pre-pr.sh, /ship skill.
#
# Exits 0 on success, 1 on violation.
# Usage: bash scripts/lib/check-migration-ordering.sh

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MIGRATION_DIR="$PROJECT_ROOT/crates/server/migrations"

if [ ! -d "$MIGRATION_DIR" ]; then
  echo "skipped: $MIGRATION_DIR not found"
  exit 0
fi

EXPECTED=1
LAST_FILE=""
for f in "$MIGRATION_DIR"/[0-9]*.sql; do
  [ -e "$f" ] || continue
  NUM=$(basename "$f" | grep -oE '^[0-9]+' | sed 's/^0*//')
  : "${NUM:=0}"
  if [ "$NUM" != "$EXPECTED" ]; then
    echo "migration ordering broken at $(basename "$f"): expected $(printf '%03d' "$EXPECTED"), got $(printf '%03d' "$NUM")"
    if [ -n "$LAST_FILE" ]; then
      echo "previous file: $(basename "$LAST_FILE")"
    fi
    echo "fix: renumber to the next available number (see specs/migrations.md)"
    exit 1
  fi
  LAST_FILE="$f"
  EXPECTED=$((EXPECTED + 1))
done

echo "migrations sequential (001..$(printf '%03d' $((EXPECTED - 1))))"
