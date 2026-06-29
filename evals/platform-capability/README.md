# Platform Capability Evaluation

Evaluates whether an Everruns agent equipped with the
[`platform_management`](../../crates/core/src/capabilities/platform_management.rs)
capability can turn natural-language requests into the **correct platform
operations** — managing agents, harnesses, apps, channels, and sessions — and
behave safely around destructive requests.

This is the platform-control analogue of [`evals/swe-bench`](../swe-bench): a
curated dataset plus a loader and runner that drive the first-class Everruns
[evals system](../../specs/evals.md) over the `/v1/evals` HTTP API. The server
spins up a real session per case, runs the conversation, and scores the result —
so every case is debuggable by clicking into its session in the UI.

## What "platform capability" means here

The `platform_management` capability gives an agent these tools:

| Area | Read | Write |
|------|------|-------|
| Agents | `read_agents` | `manage_agents` (create/update/delete) |
| Harnesses | `read_harnesses` | `manage_harnesses` (create/update/delete/copy) |
| Apps | `read_apps` | `manage_apps` (create/update/delete/destroy/publish/unpublish) |
| App channels | — | `manage_app_channels` (add/update/delete) |
| Sessions | `read_sessions`, `session_context_report` | `manage_sessions` (create/delete) |
| Session I/O | `session_read_messages`, `session_read_response` | `session_send_message` |
| Capabilities | `read_capabilities` | — |

The built-in **`platform-chat`** harness carries this capability (it inherits
`generic` and adds `platform_management`), so it is the default eval target.

## Architecture

```
dataset.yaml          loader.py              runner.py
    │                     │                      │
    ▼                     ▼                      ▼
runner-agnostic   POST /v1/evals          POST /v1/evals/{id}/runs
intent + scorers  POST .../cases          poll GET .../runs/{id}
                  (create Eval+Cases)     print summary + per-case table
```

`dataset.yaml` is the durable artifact and is intentionally **runner-agnostic**:
it describes intent and expected behaviour, not how execution happens. Today the
loader replays it through the internal eval engine; the same prompts could later
be driven by an external runner (e.g. Mira) and the results imported via
`POST /v1/evals/import` (see [`proposals/mira-results-publishing.md`](../../proposals/mira-results-publishing.md)).

## Setup

```bash
cd evals/platform-capability
pip install -e .

export EVERRUNS_API_URL=http://localhost:9300/api
export EVERRUNS_API_KEY=dev          # or "Bearer evr_pat_..." for a PAT
```

The `evals` feature flag must be enabled on the server (it gates `/v1/evals`).

## Quick start

```bash
# Load the full suite (creates one Eval with all cases)
python -m platform_capability.loader -o manifest.json

# Trigger a run and watch it complete
python -m platform_capability.runner --eval-id eval_xxx

# Compare a different model on the same cases
python -m platform_capability.runner --eval-id eval_xxx --model claude-sonnet-4-6
```

Load or run a subset by tag (`agents`, `harnesses`, `apps`, `sessions`,
`capabilities`, `safety`, `multi-turn`, `read`, `write`):

```bash
python -m platform_capability.loader --tag safety
python -m platform_capability.runner --eval-id eval_xxx --tag safety
```

## How it works

### Case design

Each case sends one or more natural-language messages to a fresh
`platform-chat` session and scores what the agent did. Scoring leans on
`tool_called`: for a capability eval the primary signal is **"did the agent pick
the right tool?"**, not the exact wording of its reply. `contains`/`regex`
checks carry lower weight so phrasing differences do not cause false negatives.
A case passes only when **all** its scorers pass; the case score is the
weighted average of scorer values (see [`specs/evals.md`](../../specs/evals.md)).

Cases that mutate state are **self-contained**: a multi-turn case creates the
entity it then updates/reads, so cases do not depend on entities pre-existing in
the org.

### Categories

- **Read** — list/find agents, harnesses, apps, sessions, capabilities.
- **Write** — create/update/rename/archive agents; create/copy harnesses;
  create/publish apps and add channels; create sessions.
- **Session I/O** — create a session, message it, and report the reply.
- **Multi-step** — discover a capability then use it; create then confirm.
- **Safety** — vague bulk-delete and hard-destroy requests must **not** trigger
  blind mutation (`tool_not_called`); off-platform questions must not cause
  spurious platform changes.

### Scoring caveat

Live pass rates reflect the built-in scorers only — they check tool selection
and response content, not deep semantic correctness. For nuanced grading,
`llm_judge` (eval spec, Phase 2) or an external runner can layer on top.

## Re-running: clean up created entities

Write cases create **real** entities, and names are unique per org, so a second
run can collide on `create`. Run against a disposable/scratch org, or archive
the eval-created entities (their names are prefixed `eval-` or `Eval `) between
runs. Read-only and safety cases are safe to re-run as-is.
