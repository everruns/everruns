#!/usr/bin/env bash
# Check migration filenames in crates/server/migrations/.
#
# New migrations are named by UTC creation time, `YYYYMMDDHHMMSS_<description>.sql`
# (sqlx's own default, created by `just new-migration <name>`). Parallel branches
# cannot pick the same version, so there is nothing to renumber after a rebase.
# sqlx applies any version missing from `_sqlx_migrations`, in version order,
# so a branch merged later with an older timestamp still applies.
#
# The baseline (BASELINE) replaced the sequential history 001..LEGACY_MAX on
# 2026-10-11 and must stay the lowest version. That history lives on, frozen,
# in legacy/ (exactly 001..LEGACY_MAX) for adopting databases that ran it.
# A NNN_ file in the main directory is a migration written against the retired
# convention and is rejected.
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
LEGACY_MAX=222
BASELINE=20261011000000
LEGACY_DIR="$MIGRATION_DIR/legacy"

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
timestamped=0
lowest=""
declare -A seen=()

for f in "${sql_files[@]}"; do
  name=$(basename "$f")
  if [[ "$name" =~ ^([0-9]{3})_.+\.sql$ ]]; then
    echo "error: $name uses the retired NNN_ numbering"
    echo "fix: rename it with \`just new-migration <description>\` (YYYYMMDDHHMMSS_<description>.sql)"
    status=1
    continue
  elif [[ "$name" =~ ^([0-9]{14})_[a-z0-9_]+\.sql$ ]]; then
    ts="${BASH_REMATCH[1]}"
    version=$((10#$ts))
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
  if [ -z "$lowest" ] || [ "$version" -lt "$lowest" ]; then lowest=$version; fi
done

if [ "$lowest" != "$BASELINE" ]; then
  echo "error: the baseline ${BASELINE}_*.sql must exist and be the lowest version (found ${lowest:-none})"
  status=1
fi

expected=1
if [ -d "$LEGACY_DIR" ]; then
  for f in "$LEGACY_DIR"/*.sql; do
    name=$(basename "$f")
    if ! [[ "$name" =~ ^([0-9]{3})_.+\.sql$ ]] || [ "$((10#${BASH_REMATCH[1]}))" != "$expected" ]; then
      echo "error: legacy history broken at legacy/$name: expected $(printf '%03d' "$expected")_*.sql"
      status=1
      break
    fi
    expected=$((expected + 1))
  done
fi
if [ "$status" -eq 0 ] && [ "$expected" -ne $((LEGACY_MAX + 1)) ]; then
  echo "error: legacy/ must hold exactly 001..$(printf '%03d' "$LEGACY_MAX"), found $((expected - 1)) files"
  status=1
fi

if [ "$status" -ne 0 ]; then
  echo "see knowledge/operations/migrations.md"
  exit 1
fi

echo "migrations ok ($timestamped timestamped from the baseline, versions unique; legacy/ 001..$(printf '%03d' "$LEGACY_MAX") frozen)"
