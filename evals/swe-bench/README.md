# SWE-bench Lite Evaluation

Run [SWE-bench Lite](https://www.swebench.com/) (300 real-world GitHub issues) against Everruns agents.

## Architecture

```
loader.py          runner.py              scorer.py
    │                   │                      │
    ▼                   ▼                      ▼
HuggingFace ──►  Everruns API  ──►  predictions.jsonl  ──►  SWE-bench Docker  ──►  PATCH scores back
 dataset        (create eval,       (artifact export)       harness                 to Everruns
                 trigger run,
                 poll status)
```

Three scripts, three steps:

1. **`loader`**: Pulls SWE-bench Lite from HuggingFace, creates an Eval with one case per instance
2. **`runner`**: Triggers an eval run, polls until completion, exports `predictions.jsonl`
3. **`scorer`**: Runs the official SWE-bench Docker harness, writes pass/fail scores back via the bulk PATCH API

## Setup

```bash
cd evals/swe-bench
pip install -e .

# For scoring (requires Docker)
pip install -e ".[scorer]"
```

Set environment variables (or pass as flags):

```bash
export EVERRUNS_API_URL=http://localhost:9300/api
export EVERRUNS_API_KEY=dev
```

## Quick Start, Integration Test (2 cases)

```bash
# Load 2 representative instances
python -m swe_bench.loader --integration -o manifest.json

# Trigger run (uses eval ID from loader output)
python -m swe_bench.runner --eval-id eval_xxx

# Score (after run completes)
python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --predictions predictions-evalrun_xxx.jsonl
```

## Full Run (300 cases)

```bash
# Load all 300 instances
python -m swe_bench.loader -o manifest.json

# Run with a specific model
python -m swe_bench.runner --eval-id eval_xxx --model claude-sonnet-4-20250514

# Score
python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --predictions predictions-evalrun_xxx.jsonl
```

## Commands

### loader

```
python -m swe_bench.loader [OPTIONS]

Options:
  --integration       Load only 2 instances (astropy-12907, django-11179)
  --limit N           Load first N instances
  --harness NAME      Harness name (default: coding-daytona)
  --name TEXT         Custom eval name
  --tag TEXT          Extra tag
  -o, --output FILE   Write manifest JSON
```

### runner

```
python -m swe_bench.runner --eval-id EVAL_ID [OPTIONS]

Options:
  --model MODEL       Model override (e.g. claude-sonnet-4-20250514)
  --harness NAME      Harness override for this run
  --poll-interval N   Seconds between polls (default: 15)
  -o, --output FILE   Custom predictions output path
```

### scorer

```
python -m swe_bench.scorer [OPTIONS]

Options:
  --eval-id ID        Eval ID (required for write-back)
  --run-id ID         Run ID (required for write-back)
  --predictions FILE  Path to predictions.jsonl (runs Docker harness)
  --results FILE      Path to pre-computed results (skip harness)
  --max-workers N     Docker parallelism (default: 4)
  --dry-run           Score without writing back
  -o, --output FILE   Save report JSON
```

## How It Works

### Case Design

Each case gives the agent:
- The problem statement from the SWE-bench instance
- Instructions to checkout the correct commit, fix the bug, and write `git diff` to `/workspace/fix.patch`

The `artifacts` field on each case tells the eval runner to collect `/workspace/fix.patch` after the agent finishes. A lightweight `file_contains` scorer checks that the patch file exists and contains a diff (sanity check only, real scoring happens externally).

### External Scoring

SWE-bench scoring requires repo-specific Python environments (3.8–3.10) with exact dependency sets. Rather than replicating this inside the agent sandbox, we:

1. Export all patches via `GET /runs/{id}/artifacts` → `predictions.jsonl`
2. Feed them to the official SWE-bench Docker harness (which has all environments pre-built)
3. Write real pass/fail scores back via `PATCH /runs/{id}/scores`

This means live pass rates during the run are based on the sanity scorer only. Real scores arrive after external scoring completes.

### Comparing Models

Run the same eval with different models:

```bash
python -m swe_bench.runner --eval-id eval_xxx --model claude-sonnet-4-20250514
python -m swe_bench.runner --eval-id eval_xxx --model gpt-4o
```

Each run produces its own predictions and scores. Compare in the Everruns UI.
