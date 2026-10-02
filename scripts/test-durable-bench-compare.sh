#!/usr/bin/env bash
# Tests for scripts/lib/durable-bench-compare.sh: the weekly durable bench
# gate passes within the allowed drop, fails past it, and reports without a
# baseline.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPARE="$PROJECT_ROOT/scripts/lib/durable-bench-compare.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

row() {
  printf '{"bench":"b","scenario":"%s","moniker":"m","smoke":false,"tasks":10,"tasks_per_sec":%s,"s2s_p50_ms":1,"s2s_p99_ms":2,"e2e_p50_ms":1,"e2e_p99_ms":2}\n' "$1" "$2"
}

{ row steady 1000; row other 1000; } >"$TMP/baseline.jsonl"
{ row steady 800; row brand_new 5; } >"$TMP/within.jsonl"
{ row steady 600; } >"$TMP/regressed.jsonl"

fail() {
  echo "FAIL: $1"
  exit 1
}

out="$("$COMPARE" "$TMP/within.jsonl" "$TMP/baseline.jsonl")" || fail "a 20% drop must pass"
grep -q '| b | steady | 800 | 1000 | -20% |' <<<"$out" || fail "row for steady scenario missing: $out"
grep -q '| b | brand_new | 5 | new |' <<<"$out" || fail "a new scenario is reported as new"

if out="$("$COMPARE" "$TMP/regressed.jsonl" "$TMP/baseline.jsonl")"; then
  fail "a 40% drop must fail"
fi
grep -q 'REGRESSED' <<<"$out" || fail "a regression is flagged in the table"

DURABLE_BENCH_MAX_DROP_PCT=50 "$COMPARE" "$TMP/regressed.jsonl" "$TMP/baseline.jsonl" >/dev/null ||
  fail "the threshold is configurable"

out="$("$COMPARE" "$TMP/within.jsonl" "$TMP/missing.jsonl")" || fail "no baseline must not fail"
grep -q 'No baseline' <<<"$out" || fail "missing baseline is reported"

: >"$TMP/empty.jsonl"
if "$COMPARE" "$TMP/empty.jsonl" "$TMP/baseline.jsonl" >/dev/null; then
  fail "an empty run must fail"
fi

echo "durable bench compare: all checks passed"
