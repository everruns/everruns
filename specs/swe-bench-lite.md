# SWE-bench Lite on Everruns

Status: **analysis complete, first case validated**

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

### 1. Eval workflow not implemented

`EvalService::create_run()` creates records but never dispatches a `RunEvalWorkflow`. Eval runs stay at `pending` forever. Workaround: create sessions directly via API.

### 2. Tool resolution: agent capabilities override harness

In `act_activity`, when `agent_id` is present, only agent capabilities are loaded (not harness). Fix: put capabilities on the agent, not just the harness. Long-term: merge both.

### 3. Sandbox Python version

Daytona default snapshot uses Python 3.14. Many SWE-bench repos (older commits from 2018-2022) won't build on modern Python. Fix: use Daytona snapshots with specific Python versions, or install pyenv in setup.

### 4. No `command` scorer

Current scorers check message content or file content. SWE-bench needs: "run test script, check exit code 0." Options:
- Build a `command` scorer that executes in the session's sandbox
- External validation (extract patches, run SWE-bench harness separately)
- `file_contains` workaround (agent writes results to file)

## Test matrix

| # | Environment | Model | Strategy | Signal |
|---|---|---|---|---|
| 1 | Daytona | Claude Sonnet | Direct fix | Baseline |
| 2 | E2B | Claude Sonnet | Direct fix | Env comparison |
| 3 | Daytona | GPT-4o | Direct fix | Model comparison |
| 4 | Daytona | Claude Sonnet | Exploration-first | Strategy comparison |
| 5 | Daytona | Claude Opus | Direct fix | Model ceiling |

## What to build next

1. **EvalRun workflow** - implement `RunEvalWorkflow` in durable engine
2. **Dataset loader** - script to bulk-create 300 EvalCases from HuggingFace dataset
3. **`command` scorer** - run shell command in sandbox, pass on exit code 0
4. **Sandbox snapshots** - pre-bake Python 3.8/3.9/3.10 snapshots for older repos
5. **EvalRun-level overrides** - allow `agent_id`/`harness_id`/`model_override` per run (not just per eval)
6. **Merge harness + agent capabilities** at runtime (not either/or)
