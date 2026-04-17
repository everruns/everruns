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

### 3. Sandbox Python version — BLOCKING

Default Daytona snapshot uses Python 3.14. Most SWE-bench repos (2018-2022 code) require Python 3.8-3.10 for their C extensions and build systems. This is the primary remaining blocker. Confirmed: astropy 4.3 `pip install -e .` fails on Python 3.14 because `setuptools.dep_util` was removed and C extensions use incompatible NumPy ABI.

Options:
- **Daytona snapshots** with specific Python versions (one per SWE-bench `version` field)
- **pyenv in post script** — `pyenv install 3.9.x && pyenv shell 3.9.x` before running tests
- **Conda environments** — mirror the official SWE-bench harness approach (requires conda in snapshot)

### 4. ~~No `command` scorer~~ — RESOLVED by `post` messages (#1248)

EvalCase now has an optional `post: Vec<EvalInputMessage>` field sent after the conversation completes. A `post` message tells the agent to run a verification script that prints a structured `swe-result: pass` / `swe-result: fail` line, and a standard `contains` scorer matches it. No new scorer type needed.

### 5. ~~Test patch application~~ — RESOLVED

`git apply` of the gold test_patch was failing because the patch content was truncated (missing trailing context lines). Root cause: WebFetch summarization dropped context. Fix: use the HuggingFace datasets API (`datasets-server.huggingface.co/rows`) to get the exact, byte-accurate test_patch. Confirmed working after using the correct patch content.

### 6. Post script dependencies

The post verification script needs all test dependencies installed: `pytest`, `numpy`, `pyerfa`, `hypothesis`, and the package itself (`pip install -e .`). Different SWE-bench repos need different deps. The official harness uses per-repo conda environment specs.

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

1. ~~**EvalRun workflow**~~ - DONE (#1248), but not durable
2. **Dataset loader** - script to bulk-create 300 EvalCases from HuggingFace dataset
3. **Sandbox snapshots** - pre-bake Python 3.8/3.9/3.10 snapshots per SWE-bench repo
4. ~~**EvalRun-level overrides**~~ - DONE (#1239: EvalTarget can be set at Eval, EvalCase, or EvalRun level)
5. **Robust test_patch application** - use `git apply --3way` or fall back to `patch -p1` with fuzz
6. **Durable eval runs** - replace `tokio::spawn` fire-and-forget with the durable execution engine

## End-to-end example (current state)

Single `astropy__astropy-12907` case with Claude Sonnet + `coding-daytona` harness.

**What works:**
- Eval workflow runs end-to-end (create → execute → score → report)
- Agent finds the correct one-line fix (matches gold patch) in every successful conversation turn
- Post messages send the hidden test_patch + verification script
- `git apply` of the test_patch succeeds (when using correct patch from HuggingFace API)
- Scorer checks for `swe-result: pass` in final message

**What fails (and why):**
- `pip install -e .` fails because Python 3.14 can't build astropy 4.3 C extensions
- So `python -m pytest` can't import astropy → score = 0.0
- The fix is correct but unverifiable in this environment

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
