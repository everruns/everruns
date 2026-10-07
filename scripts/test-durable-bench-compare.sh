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

# DB statements per turn gate (server turn latency rows).
db_row() {
  printf '{"bench":"s","scenario":"%s","moniker":"m","smoke":false,"tasks":10,"tasks_per_sec":10,"s2s_p50_ms":1,"s2s_p99_ms":2,"db_statements_per_turn":%s,"db_ms_per_turn":5}\n' "$1" "$2"
}
{ db_row c1 100; db_row c8 100; } >"$TMP/db_baseline.jsonl"
{ db_row c1 110; db_row c8 60; } >"$TMP/db_within.jsonl"
{ db_row c1 130; } >"$TMP/db_grown.jsonl"

out="$("$COMPARE" "$TMP/db_within.jsonl" "$TMP/db_baseline.jsonl")" || fail "10% more statements must pass: $out"
grep -q '| s | c1 | 110 | 100 | 10% |' <<<"$out" || fail "statements row missing: $out"
grep -q '| s | c8 | 60 | 100 | -40% |' <<<"$out" || fail "fewer statements must pass: $out"

if out="$("$COMPARE" "$TMP/db_grown.jsonl" "$TMP/db_baseline.jsonl")"; then
  fail "30% more statements per turn must fail"
fi
grep -q '| s | c1 | 130 | 100 | 30% REGRESSED |' <<<"$out" || fail "statement growth is flagged: $out"

DURABLE_BENCH_MAX_STATEMENT_GROWTH_PCT=50 "$COMPARE" "$TMP/db_grown.jsonl" "$TMP/db_baseline.jsonl" >/dev/null ||
  fail "the statement threshold is configurable"

# A baseline without the field (older rows) reports throughput only.
out="$("$COMPARE" "$TMP/db_grown.jsonl" "$TMP/baseline.jsonl")" || true
grep -q 'Database gate' <<<"$out" && fail "no database table without baseline statements"

echo "durable bench compare: all checks passed"
