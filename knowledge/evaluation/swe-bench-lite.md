---
type: Specification
title: "SWE-bench Lite on Everruns"
description: "SWE-bench Lite evaluation harness."
tags:
  - everruns
  - evaluation
---
# SWE-bench Lite on Everruns

Status: **tooling ready**: loader, runner, scorer scripts in `evals/swe-bench/`

## What is SWE-bench Lite?

300 curated real-world GitHub issues from popular Python repos (django, flask, sympy, scikit-learn, astropy, etc.). Each instance: problem statement, repo + commit, gold patch, test script. Metric: does the agent's patch make failing tests pass?

## Mapping to Everruns primitives

| SWE-bench concept | Everruns primitive |
|---|---|
| 300 task instances | EvalCases (one per instance) |
| "Fix this issue" prompt | EvalCase `conversation` (single message) |
| Test validation | External SWE-bench Docker harness → score write-back API |
| Repo checkout + env setup | Harness capability (Daytona / E2B) |
| Agent strategy | Agent config (model, instructions, capabilities) |
| Benchmark run | EvalRun across an Eval |

## First case result: `astropy__astropy-12907`

**Instance**: Modeling's `separability_matrix` bug with nested CompoundModels.

**Result**: Agent found the correct fix in ~90s.
- Created Daytona sandbox
- Cloned repo, checked out correct commit
- Read source code (`separable.py`)
- Identified the one-line fix: `cright[-right.shape[0]:, -right.shape[1]:] = 1` -> `= right`
- This matches the gold patch exactly
- Could not run verification: sandbox Python 3.14 is too new for 2022 astropy C extensions

**Exported messages**: `evals/swe-bench/examples/astropy-12907-session-export.jsonl` (22 messages, 10 tool calls)

## Findings and blockers

### 1. ~~Eval workflow not implemented~~, FIXED (#1248)

`EvalService::create_run()` now dispatches a background task that creates sessions, sends conversation + post messages, and runs scorers. Fire-and-forget (not durable yet); server crash mid-run leaves run stuck at `running`.

### 2. ~~Tool resolution: agent capabilities override harness~~, IRRELEVANT

Using the built-in `coding-daytona` harness (which extends `generic` with the `daytona` capability) removes the need for a custom agent. No custom agent needed at all.

### 3. ~~Sandbox Python version~~, BYPASSED via external scoring

Default Daytona snapshot uses Python 3.14; SWE-bench repos need 3.8-3.10. **No longer blocking**: we don't run tests in the sandbox. The agent only produces a patch; the official SWE-bench Docker harness (which has all correct environments pre-built) scores externally. Scores are written back via the bulk PATCH API.

### 4. ~~No `command` scorer~~, RESOLVED by `post` messages (#1248)

EvalCase now has an optional `post: Vec<EvalInputMessage>` field sent after the conversation completes. A `post` message tells the agent to run a verification script that prints a structured `swe-result: pass` / `swe-result: fail` line, and a standard `contains` scorer matches it. No new scorer type needed.

### 5. ~~Test patch application~~, RESOLVED

`git apply` of the gold test_patch was failing because the patch content was truncated (missing trailing context lines). Root cause: WebFetch summarization dropped context. Fix: use the HuggingFace datasets API (`datasets-server.huggingface.co/rows`) to get the exact, byte-accurate test_patch. Confirmed working after using the correct patch content.

### 6. ~~Post script dependencies~~, BYPASSED via external scoring

No longer needed, the agent doesn't run tests in the sandbox. External Docker harness handles all dependencies.

### 7. Agent reliability with sandbox creation

The Daytona `list_snapshots` API returns a format the integration doesn't expect (parsing error). The agent tries `snapshot: "python"` which doesn't exist. Combined with sandbox name collisions from previous runs, the agent often wastes its first few tool calls on errors and sometimes gives up without completing the fix.

## Test matrix

| # | Environment | Model | Strategy | Signal |
|---|---|---|---|---|
| 1 | Daytona | Claude Sonnet | Direct fix | Baseline |
| 2 | E2B | Claude Sonnet | Direct fix | Env comparison |
| 3 | Daytona | GPT-4o | Direct fix | Model comparison |
| 4 | Daytona | Claude Sonnet | Exploration-first | Strategy comparison |
| 5 | Daytona | Claude Opus | Direct fix | Model ceiling |

## What to build next

1. ~~**EvalRun workflow**~~, DONE (#1248), but not durable
2. ~~**Dataset loader**~~, DONE (`evals/swe-bench/swe_bench/loader.py`)
3. ~~**Artifact collection**~~, DONE (EVE-327, #1334)
4. ~~**Score write-back API**~~, DONE (EVE-328)
5. ~~**EvalRun-level overrides**~~, DONE (#1239: EvalTarget can be set at Eval, EvalCase, or EvalRun level)
6. ~~**Runner + scorer scripts**~~, DONE (`evals/swe-bench/swe_bench/runner.py`, `scorer.py`)
7. **Durable eval runs**: replace `tokio::spawn` fire-and-forget with the durable execution engine

## Tooling

Scripts live in `evals/swe-bench/`. See `evals/swe-bench/README.md` for full usage.

```bash
# Integration test (2 cases)
python -m swe_bench.loader --integration -o manifest.json
python -m swe_bench.runner --eval-id eval_xxx
python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --predictions predictions.jsonl

# Full benchmark (300 cases)
python -m swe_bench.loader -o manifest.json
python -m swe_bench.runner --eval-id eval_xxx --model claude-sonnet-4-20250514
python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --predictions predictions.jsonl
```

### Flow

1. **Loader** pulls HuggingFace dataset → creates Eval + EvalCases via API. Each case instructs agent to fix the issue and write `git diff > /workspace/fix.patch`. Cases declare `artifacts: [{name: "patch", path: "/workspace/fix.patch"}]`.
2. **Runner** triggers an EvalRun, polls until completion, exports `predictions.jsonl` via the artifact NDJSON endpoint.
3. **Scorer** feeds `predictions.jsonl` to the official SWE-bench Docker harness, then bulk-PATCHes scores back via the write-back API. RunSummary updates automatically.

### Why external scoring?

The sandbox uses Python 3.14; SWE-bench repos need 3.8-3.10 with repo-specific conda envs. The official Docker harness has all environments pre-built. Rather than replicating that inside Everruns, we produce patches and score externally.
