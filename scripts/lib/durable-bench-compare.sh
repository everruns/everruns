#!/usr/bin/env bash
# Compare a durable benchmark run with the committed baseline.
#
# Why: two of the three PostgreSQL benches sat broken on main for months, and
# nobody could say whether the engine had got faster or slower because there
# was no baseline anyone trusted. The weekly bench workflow runs every bench
# with `--summary`, then this script prints a markdown table against
# crates/durable/benches/baseline.jsonl and fails when throughput dropped.
#
# Only throughput gates. Shared CI runners make tail latency too noisy to fail
# on, so latency is reported, not enforced.
#
# Usage:
#   durable-bench-compare.sh <current.jsonl> [baseline.jsonl]
#
# Environment:
#   DURABLE_BENCH_MAX_DROP_PCT  allowed throughput drop per scenario (default 30)
#
# Exits 0 when no scenario regressed (or there is no baseline yet), 1 otherwise.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
current="${1:?usage: durable-bench-compare.sh <current.jsonl> [baseline.jsonl]}"
baseline="${2:-$PROJECT_ROOT/crates/durable/benches/baseline.jsonl}"
max_drop="${DURABLE_BENCH_MAX_DROP_PCT:-30}"

if [ ! -s "$current" ]; then
  echo "error: no results in $current"
  exit 1
fi

if [ ! -s "$baseline" ]; then
  echo "No baseline at $baseline yet; reporting this run only."
  echo
  echo "| Bench | Scenario | Tasks | Tasks/s | S2S p50 ms | S2S p99 ms |"
  echo "| --- | --- | ---: | ---: | ---: | ---: |"
  jq -r '"| \(.bench) | \(.scenario) | \(.tasks) | \(.tasks_per_sec) | \(.s2s_p50_ms) | \(.s2s_p99_ms) |"' "$current"
  exit 0
fi

# One row per scenario in the current run, joined with its baseline row.
report="$(
  jq -n -r --slurpfile cur "$current" --slurpfile base "$baseline" --argjson max "$max_drop" '
    ($base | map({key: "\(.bench)/\(.scenario)", value: .}) | from_entries) as $b
    | $cur[]
    | . as $c
    | $b["\($c.bench)/\($c.scenario)"] as $old
    | if $old == null then
        "| \($c.bench) | \($c.scenario) | \($c.tasks_per_sec) | new | | \($c.s2s_p99_ms) | |"
      else
        (if $old.tasks_per_sec == 0 then 0
         else (($c.tasks_per_sec - $old.tasks_per_sec) / $old.tasks_per_sec * 100) end) as $pct
        | (if $pct < -$max then " REGRESSED" else "" end) as $flag
        | "| \($c.bench) | \($c.scenario) | \($c.tasks_per_sec) | \($old.tasks_per_sec) | \($pct * 10 | round / 10)%\($flag) | \($c.s2s_p99_ms) | \($old.s2s_p99_ms) |"
      end
  '
)"

echo "Throughput gate: a scenario fails when it drops more than ${max_drop}% below the baseline."
echo
echo "| Bench | Scenario | Tasks/s | Baseline | Change | S2S p99 ms | Baseline |"
echo "| --- | --- | ---: | ---: | ---: | ---: | ---: |"
echo "$report"

if grep -q "REGRESSED" <<<"$report"; then
  echo
  echo "Throughput regressed. If the change is expected, refresh the baseline from this run's summary artifact."
  exit 1
fi
