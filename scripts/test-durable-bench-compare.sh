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

# Repeated runs: the gate compares the median run, so one slow sample among
# three does not fail it and one fast sample does not hide a real drop.
{ row steady 500; row other 990; row steady 950; row other 1010; row steady 900; row other 1000; } >"$TMP/one_slow.jsonl"
out="$(DURABLE_BENCH_MEDIAN_OUT="$TMP/median.jsonl" "$COMPARE" "$TMP/one_slow.jsonl" "$TMP/baseline.jsonl")" ||
  fail "one slow run out of three must pass: $out"
grep -q '| b | steady | 900 | 1000 | -10% |' <<<"$out" || fail "the median run is compared: $out"
grep -q 'median of up to 3 runs' <<<"$out" || fail "repeated runs are announced: $out"
[ "$(wc -l <"$TMP/median.jsonl")" -eq 2 ] || fail "one median row per scenario is written"
[ "$(jq -r 'select(.scenario == "other") | .tasks_per_sec' "$TMP/median.jsonl")" = "1000" ] ||
  fail "the median file holds the median run"
[ "$(jq -r '.scenario' "$TMP/median.jsonl" | head -1)" = "steady" ] || fail "scenarios keep their first-seen order"

{ row steady 600; row steady 990; row steady 650; } >"$TMP/one_fast.jsonl"
if out="$("$COMPARE" "$TMP/one_fast.jsonl" "$TMP/baseline.jsonl")"; then
  fail "two slow runs out of three must fail"
fi
{ row steady 600; row steady 990; } >"$TMP/two_runs.jsonl"
if "$COMPARE" "$TMP/two_runs.jsonl" "$TMP/baseline.jsonl" >/dev/null; then
  fail "an even count compares the lower middle run"
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
