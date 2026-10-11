#!/usr/bin/env bash
# Check migration filenames in crates/server/migrations/.
#
# New migrations are named by UTC creation time, `YYYYMMDDHHMMSS_<description>.sql`
# (sqlx's own default, created by `just new-migration <name>`). Parallel branches
# cannot pick the same version, so there is nothing to renumber after a rebase.
# sqlx applies any version missing from `_sqlx_migrations`, in version order,
# so a branch merged later with an older timestamp still applies.
#
# Files 001..LEGACY_MAX are the frozen sequential history from before the switch
# and must stay exactly that range. Any other 3-digit file is a migration written
# against the old convention and is rejected.
#
# Every version must be unique: two files with one version fail at startup.
#
# Spec: knowledge/operations/migrations.md
# Used by: CI (job `Migration Immutability`), scripts/lib/pre-push.sh, scripts/lib/pre-pr.sh.
#
# Usage: bash scripts/lib/check-migration-ordering.sh [migrations-dir]

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MIGRATION_DIR="${1:-$PROJECT_ROOT/crates/server/migrations}"
LEGACY_MAX=220

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

status=0
expected_legacy=1
timestamped=0
declare -A seen=()

for f in "${sql_files[@]}"; do
  name=$(basename "$f")
  if [[ "$name" =~ ^([0-9]{3})_.+\.sql$ ]]; then
    num=$((10#${BASH_REMATCH[1]}))
    if [ "$num" -gt "$LEGACY_MAX" ]; then
      echo "error: $name uses the retired NNN_ numbering"
      echo "fix: rename it with \`just new-migration <description>\` (YYYYMMDDHHMMSS_<description>.sql)"
      status=1
      continue
    fi
    if [ "$num" != "$expected_legacy" ]; then
      echo "error: legacy migration history broken at $name: expected $(printf '%03d' "$expected_legacy")"
      status=1
    fi
    expected_legacy=$((num + 1))
    version=$num
  elif [[ "$name" =~ ^([0-9]{14})_[a-z0-9_]+\.sql$ ]]; then
    version=$((10#${BASH_REMATCH[1]}))
    ts="${BASH_REMATCH[1]}"
    if ! date -u -d "${ts:0:8} ${ts:8:2}:${ts:10:2}:${ts:12:2}" >/dev/null 2>&1 \
      || [ "${ts:0:4}" -lt 2026 ]; then
      echo "error: $name: ${ts} is not a valid UTC timestamp (YYYYMMDDHHMMSS)"
      status=1
    fi
    timestamped=$((timestamped + 1))
  else
    echo "error: $name does not match YYYYMMDDHHMMSS_<description>.sql"
    echo "fix: create migrations with \`just new-migration <description>\` (lowercase, digits, underscores)"
    status=1
    continue
  fi

  if [ -n "${seen[$version]:-}" ]; then
    echo "error: $name and ${seen[$version]} share version $version"
    echo "fix: give the newer one a fresh timestamp with \`just new-migration\`"
    status=1
  fi
  seen[$version]="$name"
done

if [ "$expected_legacy" -ne $((LEGACY_MAX + 1)) ] && [ "$status" -eq 0 ]; then
  echo "error: legacy history must be 001..$(printf '%03d' "$LEGACY_MAX"), found 001..$(printf '%03d' $((expected_legacy - 1)))"
  status=1
fi

if [ "$status" -ne 0 ]; then
  echo "see knowledge/operations/migrations.md"
  exit 1
fi

echo "migrations ok (legacy 001..$(printf '%03d' "$LEGACY_MAX"), $timestamped timestamped, versions unique)"
