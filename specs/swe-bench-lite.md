# SWE-bench Lite on Everruns

Status: **end-to-end eval workflow validated** (April 2026 rebase)

## What is SWE-bench Lite?

300 curated real-world GitHub issues from popular Python repos (django, flask, sympy, scikit-learn, astropy, etc.). Each instance: problem statement, repo + commit, gold patch, test script. Metric: does the agent's patch make failing tests pass?

## Mapping to Everruns primitives

| SWE-bench concept | Everruns primitive |
|---|---|
| 300 task instances | EvalCases (one per instance) |
| "Fix this issue" prompt | EvalCase `conversation` (single message) |
| Test validation | Scorer (needs new `command` scorer type) |
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

**Exported messages**: `swe-bench-lite-astropy-12907-export.jsonl` (22 messages, 10 tool calls)

## Findings and blockers

### 1. ~~Eval workflow not implemented~~ — FIXED (#1248)

`EvalService::create_run()` now dispatches a background task that creates sessions, sends conversation + post messages, and runs scorers. Fire-and-forget (not durable yet); server crash mid-run leaves run stuck at `running`.

### 2. ~~Tool resolution: agent capabilities override harness~~ — IRRELEVANT

Using the built-in `coding-daytona` harness (which extends `generic` with the `daytona` capability) removes the need for a custom agent. No custom agent needed at all.

### 3. Sandbox Python version

Still an issue. Default Daytona snapshot uses Python 3.14. Many SWE-bench repos won't build on modern Python. Fix: per-repo Daytona snapshots with specific Python versions, or install pyenv in the conversation setup.

### 4. ~~No `command` scorer~~ — RESOLVED by `post` messages (#1248)

EvalCase now has an optional `post: Vec<EvalInputMessage>` field sent after the conversation completes. A `post` message tells the agent to run a verification script that prints a structured `swe-result: pass` / `swe-result: fail` line, and a standard `contains` scorer matches it. No new scorer type needed.

### 5. Test patch application

The gold `test_patch` from SWE-bench sometimes fails `git apply` on the freshly cloned repo — likely due to shallow clone or line ending mismatches. Needs handling in the post script: try `git apply --3way` or `patch -p1`.

## Test matrix

| # | Environment | Model | Strategy | Signal |
|---|---|---|---|---|
| 1 | Daytona | Claude Sonnet | Direct fix | Baseline |
| 2 | E2B | Claude Sonnet | Direct fix | Env comparison |
| 3 | Daytona | GPT-4o | Direct fix | Model comparison |
| 4 | Daytona | Claude Sonnet | Exploration-first | Strategy comparison |
| 5 | Daytona | Claude Opus | Direct fix | Model ceiling |

## What to build next

1. ~~**EvalRun workflow**~~ - DONE (#1248), but not durable
2. **Dataset loader** - script to bulk-create 300 EvalCases from HuggingFace dataset
3. **Sandbox snapshots** - pre-bake Python 3.8/3.9/3.10 snapshots per SWE-bench repo
4. ~~**EvalRun-level overrides**~~ - DONE (#1239: EvalTarget can be set at Eval, EvalCase, or EvalRun level)
5. **Robust test_patch application** - use `git apply --3way` or fall back to `patch -p1` with fuzz
6. **Durable eval runs** - replace `tokio::spawn` fire-and-forget with the durable execution engine

## End-to-end example (current, working)

Single `astropy__astropy-12907` case with Claude Sonnet + `coding-daytona` harness, ran in ~2.5 min. Agent correctly identified and applied the one-line fix matching the gold patch. Post script attempted gold test_patch + pytest; `git apply` failed (issue #5 above). Score: 0.0. The workflow itself works end-to-end.

```
POST /api/v1/evals
  { name, target: { type: "session", harness_name: "coding-daytona" }, tags: ["swe-bench"] }

POST /api/v1/evals/{eval_id}/cases
  {
    name: "astropy__astropy-12907",
    conversation: [{ content: "<problem_statement + setup instructions>" }],
    post: [{ content: "<verification script with hidden test_patch>" }],
    scorers: [{ type: "contains", text: "swe-result: pass", weight: 1.0 }],
    max_turns: 40, timeout_seconds: 900
  }

POST /api/v1/evals/{eval_id}/runs
  { model_override: "claude-sonnet-4-20250514" }
```
