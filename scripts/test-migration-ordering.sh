#!/usr/bin/env bash
# Exercise scripts/lib/check-migration-ordering.sh against fixture directories.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$PROJECT_ROOT/scripts/lib/check-migration-ordering.sh"
LEGACY_MAX="$(sed -n 's/^LEGACY_MAX=//p' "$CHECK")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0

fixture() {
  local dir="$tmp/$1"
  shift
  mkdir -p "$dir"
  for i in $(seq 1 "$LEGACY_MAX"); do
    : >"$dir/$(printf '%03d' "$i")_legacy.sql"
  done
  for name in "$@"; do
    : >"$dir/$name"
  done
  printf '%s\n' "$dir"
}

expect() {
  local want="$1" label="$2" dir="$3" pattern="${4:-}" out
  if out="$(bash "$CHECK" "$dir" 2>&1)"; then got=pass; else got=fail; fi
  if [ "$got" != "$want" ]; then
    echo "FAIL: $label: expected $want, got $got"
    echo "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  elif [ -n "$pattern" ] && ! grep -q "$pattern" <<<"$out"; then
    echo "FAIL: $label: output lacks '$pattern'"
    echo "$out" | sed 's/^/  /'
    failures=$((failures + 1))
  else
    echo "ok: $label"
  fi
}

expect pass "legacy history only" "$(fixture legacy)"
expect pass "timestamped after legacy" \
  "$(fixture ts 20261011210512_add_widgets.sql 20261012080000_more.sql)"
expect fail "two timestamped files share a version" \
  "$(fixture dup 20261011210512_a.sql 20261011210512_b.sql)" "share version"
expect fail "new file with the retired NNN_ numbering" \
  "$(fixture retired "$(printf '%03d' $((LEGACY_MAX + 1)))_new.sql")" "retired NNN_"
expect fail "legacy duplicate" \
  "$(fixture legacydup "$(printf '%03d' "$LEGACY_MAX")_other.sql")" "share version"
expect fail "invalid timestamp" "$(fixture badts 20261340000000_x.sql)" "not a valid UTC timestamp"
expect fail "uppercase description" "$(fixture upper 20261011210512_AddWidgets.sql)" "does not match"
expect fail "too-short prefix" "$(fixture short 2026101121_x.sql)" "does not match"

gap="$(fixture gap)"
rm "$gap/005_legacy.sql"
expect fail "gap in legacy history" "$gap" "legacy migration history broken"

expect pass "repository migrations" "$PROJECT_ROOT/crates/server/migrations"

if [ "$failures" -ne 0 ]; then
  echo "$failures migration ordering test(s) failed"
  exit 1
fi
echo "migration ordering tests passed"
