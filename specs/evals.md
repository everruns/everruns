# Evals

<!-- Design Decisions:
  - Evals are user-facing: org-scoped, visible in UI, not internal testing infrastructure
  - Each eval case creates a real session — same behavior as production, debuggable
  - Scorers return 0.0–1.0 (not binary) to support nuanced grading
  - Eval runs are durable workflows — reuse existing engine, no parallel infra
  - Multi-turn cases supported via sequential InputMessage delivery
  - LLM-as-judge scorer deferred to Phase 2 (requires model call inside scoring)
  - No dataset management — evals are small, curated collections
  - No cross-org visibility — evals are org-scoped like all other entities
  - EvalTarget replaces harness_id + agent_id — unified session setup contract
  - EvalTarget::Session mirrors CreateSessionRequest; EvalTarget::App references a deployed app
  - Resolution order: EvalRun.target → EvalCase.target → Eval.target → org default harness
  - EvalCaseResult stores both target (live reference) and target_snapshot (frozen copy at execution time)
-->

## Abstract

Evals let users define, run, and track behavioral tests for their agents. An Eval is a named collection of cases — each case sends messages to a fresh session and scores the result. Users compare runs across models, track regressions after prompt changes, and gate App publishes on pass rates.

Each eval case creates a real session with the target agent and harness. Failed cases are debuggable by clicking into the session and seeing the full conversation, tool calls, and events.

## Concepts

### EvalTarget

Defines how to instantiate a session for eval cases. Two variants:

- **`session`**: Mirrors `CreateSessionRequest` — `harness_id` and `harness_name` are optional but mutually exclusive (if both are provided, validation fails; if neither, the org default harness is used). Other fields: `agent_id`, `model_id`, `system_prompt`, `max_iterations`. Full control over session setup.
- **`app`**: Reference to a deployed App — `app_id`. Session created via the app's configuration.

**Resolution order**: `EvalRun.target` → `EvalCase.target` → `Eval.target` → org default harness.

All three levels (Eval, EvalCase, EvalRun) have an optional `target` field. The first non-null target in the resolution chain is used. This allows running the same eval cases against different targets per run, or defining per-case overrides for heterogeneous test suites.

### Eval

Top-level entity. Contains cases that define expected behaviors.

- Org-scoped, follows standard building-block lifecycle (`active → archived → deleted`)
- Optional `target` (EvalTarget) — the default session setup for all cases
- Optional `model_override` for baseline model selection
- Tagged for organization and filtering

### EvalCase

A single test within an eval. Defines input messages, scoring criteria, and execution bounds.

- Each case has a `description` explaining what behavior it measures (self-documenting)
- Optional `target` (EvalTarget) — per-case override
- `conversation`: one or more input messages sent sequentially (multi-turn support)
- `post`: optional verification messages sent after conversation completes and session idles, before scoring (e.g. running test scripts for SWE-bench)
- `scorers`: list of scoring rules applied after execution completes
- `max_turns`: optional bound on agent turns (default: 10)
- `timeout_seconds`: per-case timeout (default: 120)
- Tagged independently from parent eval for subset runs

### EvalRun

A single execution of all (or a tagged subset of) cases in an eval.

- Creates one session per case
- Optional `target` (EvalTarget) — per-run override (e.g., test same cases against a different app)
- Supports model override per run (compare models without editing the eval)
- Tracks aggregate metrics: pass rate, average score, turns, latency, tokens
- Triggered by user action, API call, or scheduled task
- Status: `pending → running → completed | failed | cancelled`

### EvalCaseResult

The outcome of a single case within a run.

- Links to the actual session created for this case (browsable in UI)
- `target`: resolved EvalTarget at execution time (always concrete, never NULL)
- `target_snapshot`: identical frozen copy for immutability (both set at run creation, both equal initially; `target_snapshot` must never be overwritten)
- Contains per-scorer scores with pass/fail, value (0.0–1.0), and reason
- Captures efficiency metrics: turn count, latency, token usage

### Scorer

A rule that grades agent output after execution. Embedded in EvalCase, not a standalone entity.

- Returns `{ pass: bool, value: f64, reason: String }` where value is 0.0–1.0
- Case passes when ALL scorers pass
- Case-level score = weighted average of scorer values

## Scorer Types

| Type | Config | What It Checks |
|------|--------|----------------|
| `contains` | `{ text: String }` | Final assistant message contains substring |
| `not_contains` | `{ text: String }` | Final assistant message does NOT contain substring |
| `regex` | `{ pattern: String }` | Final assistant message matches regex pattern |
| `tool_called` | `{ tool: String, min: Option<u32> }` | Agent called named tool at least `min` times (default 1) |
| `tool_not_called` | `{ tool: String }` | Agent did NOT call named tool |
| `tool_call_count` | `{ min: Option<u32>, max: Option<u32> }` | Total tool calls within range |
| `turns_within` | `{ max: u32 }` | Completed within N turns |
| `file_contains` | `{ path: String, text: String }` | Session filesystem file contains substring |
| `json_schema` | `{ schema: Value }` | Final assistant message parses as JSON matching schema |
| `llm_judge` | `{ rubric: String, model: Option<String> }` | LLM grades output against rubric (Phase 2) |

## Data Model

### Eval

See `crates/core/src/eval.rs` for full field definitions.

| Field | Type | Description |
|-------|------|-------------|
| `id` / `public_id` | UUID / EvalId | Dual-ID pattern (`eval_` prefix) |
| `org_id` | i64 | Owning organization |
| `name` | String | Display name |
| `description` | Option\<String\> | Optional description |
| `target` | Option\<EvalTarget\> | Session setup target (JSONB) |
| `model_override` | Option\<String\> | Optional default model for runs |
| `tags` | Vec\<String\> | Organization tags |
| `status` | EvalStatus | `active`, `archived`, `deleted` |
| `created_at` | DateTime | Creation timestamp |
| `updated_at` | DateTime | Last update timestamp |
| `archived_at` | Option\<DateTime\> | Archive timestamp |
| `deleted_at` | Option\<DateTime\> | Deletion timestamp |

### EvalCase

| Field | Type | Description |
|-------|------|-------------|
| `id` / `public_id` | UUID / EvalCaseId | Dual-ID pattern (`evalcase_` prefix) |
| `eval_id` | UUID | FK to parent eval |
| `name` | String | Case name |
| `description` | Option\<String\> | What behavior this measures |
| `target` | Option\<EvalTarget\> | Per-case target override (JSONB) |
| `tags` | Vec\<String\> | Tags for subset runs |
| `conversation` | Vec\<InputMessage\> | Input messages (sequential) |
| `post` | Option\<Vec\<InputMessage\>\> | Post-conversation verification messages (sent after session idles, before scoring) |
| `scorers` | Vec\<Scorer\> | Scoring rules (JSONB) |
| `max_turns` | Option\<u32\> | Turn limit (default: 10) |
| `timeout_seconds` | Option\<u32\> | Timeout (default: 120) |
| `position` | i32 | Display order |
| `created_at` | DateTime | Creation timestamp |
| `updated_at` | DateTime | Last update timestamp |

### EvalRun

| Field | Type | Description |
|-------|------|-------------|
| `id` / `public_id` | UUID / EvalRunId | Dual-ID pattern (`evalrun_` prefix) |
| `eval_id` | UUID | FK to parent eval |
| `org_id` | i64 | Owning organization |
| `target` | Option\<EvalTarget\> | Per-run target override (JSONB) |
| `model_override` | Option\<String\> | Model override for this run |
| `filter_tags` | Option\<Vec\<String\>\> | Only run cases matching these tags |
| `status` | EvalRunStatus | `pending`, `running`, `completed`, `failed`, `cancelled` |
| `triggered_by` | String | `user`, `schedule`, `publish_gate` |
| `started_at` | Option\<DateTime\> | When execution started |
| `completed_at` | Option\<DateTime\> | When execution finished |
| `summary` | Option\<RunSummary\> | Aggregate metrics (JSONB, set on completion) |
| `created_at` | DateTime | Creation timestamp |
| `updated_at` | DateTime | Last update timestamp |

### RunSummary (embedded JSONB)

| Field | Type | Description |
|-------|------|-------------|
| `total` | u32 | Total cases in run |
| `passed` | u32 | Cases that passed |
| `failed` | u32 | Cases that failed |
| `errored` | u32 | Cases that errored or timed out |
| `pass_rate` | f64 | passed / total |
| `avg_score` | f64 | Average case score |
| `avg_turns` | f64 | Average turns per case |
| `avg_latency_ms` | u64 | Average case latency |
| `total_input_tokens` | u64 | Total input tokens |
| `total_output_tokens` | u64 | Total output tokens |

### EvalCaseResult

| Field | Type | Description |
|-------|------|-------------|
| `id` / `public_id` | UUID / EvalResultId | Dual-ID pattern (`evalresult_` prefix) |
| `eval_run_id` | UUID | FK to parent run |
| `eval_case_id` | UUID | FK to the case |
| `session_id` | Option\<UUID\> | FK to session created for this case |
| `target` | EvalTarget | Resolved target at execution time (JSONB, always concrete) |
| `target_snapshot` | EvalTarget | Frozen copy of resolved target (JSONB, immutable) |
| `status` | CaseResultStatus | `pending`, `running`, `passed`, `failed`, `errored`, `timeout` |
| `scores` | Option\<Map\<String, Score\>\> | Per-scorer results (JSONB) |
| `turns` | Option\<u32\> | Turn count |
| `latency_ms` | Option\<u64\> | Execution time |
| `tokens` | Option\<TokenUsage\> | Token usage |
| `error_message` | Option\<String\> | Error details if errored |
| `created_at` | DateTime | Creation timestamp |
| `updated_at` | DateTime | Last update timestamp |

### Score (embedded JSONB)

| Field | Type | Description |
|-------|------|-------------|
| `pass` | bool | Whether this scorer passed |
| `value` | f64 | Score 0.0–1.0 |
| `reason` | String | Human-readable explanation |

## ID Schema

| Entity | Prefix | Example |
|--------|--------|---------|
| Eval | `eval` | `eval_01933b5a000070008000000000000001` |
| EvalCase | `evalcase` | `evalcase_01933b5a000070008000000000000001` |
| EvalRun | `evalrun` | `evalrun_01933b5a000070008000000000000001` |
| EvalCaseResult | `evalresult` | `evalresult_01933b5a000070008000000000000001` |

## API

All endpoints under `/v1/evals`. See `crates/server/src/api/evals.rs`.

### Eval CRUD

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/evals` | Create eval |
| `GET` | `/v1/evals` | List evals (paginated, filterable by status) |
| `GET` | `/v1/evals/{eval_id}` | Get eval with case count and last run summary |
| `PATCH` | `/v1/evals/{eval_id}` | Update eval |
| `DELETE` | `/v1/evals/{eval_id}` | Archive eval |

### Case Management

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/evals/{eval_id}/cases` | Add case |
| `GET` | `/v1/evals/{eval_id}/cases` | List cases |
| `GET` | `/v1/evals/{eval_id}/cases/{case_id}` | Get case |
| `PATCH` | `/v1/evals/{eval_id}/cases/{case_id}` | Update case |
| `DELETE` | `/v1/evals/{eval_id}/cases/{case_id}` | Remove case |

### Run Management

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/evals/{eval_id}/runs` | Trigger run (body: optional `model_override`, `filter_tags`) |
| `GET` | `/v1/evals/{eval_id}/runs` | List runs with summaries |
| `GET` | `/v1/evals/{eval_id}/runs/{run_id}` | Get run with case results |
| `POST` | `/v1/evals/{eval_id}/runs/{run_id}/cancel` | Cancel running eval |

## Execution Flow

```
POST /v1/evals/{eval_id}/runs
  │
  ├─ Create EvalRun (status: pending, target from request body)
  ├─ For each case:
  │    ├─ Resolve target: run.target → case.target → eval.target → org default
  │    └─ Create EvalCaseResult (status: pending, target + target_snapshot = resolved target)
  ├─ Spawn background execution task
  │
  │  For each case (bounded concurrency = 5):
  │    1. Update CaseResult status → running
  │    2. Create session from resolved EvalTarget (tags: ["eval"])
  │    3. For each message in case.conversation:
  │       a. POST message to session
  │       b. Wait for session idle
  │    4. For each message in case.post (if present):
  │       a. POST message to session
  │       b. Wait for session idle
  │    5. Fetch session events (tool.completed, turn.completed)
  │    6. Fetch final assistant messages
  │    7. Run scorers → produce Score per scorer
  │    8. Update CaseResult (status, scores, turns, latency, tokens)
  │
  ├─ Aggregate results → RunSummary
  ├─ Update EvalRun (status: completed, summary)
  └─ Return
```

Sessions created by eval runs are tagged `eval` and reference the eval run for filtering.

## UI

### Navigation

Add "Evals" to the Building Blocks section in the sidebar, between "Skills" and "Capabilities". Icon: `FlaskConical` from lucide-react.

### Evals List Page (`/evals`)

Card grid layout (matching agents page pattern):
- Each card shows: name, agent name, harness name, tags, last run status (pass/fail badge), last run pass rate, last run date
- "New Eval" button in header
- Filter: status (active/archived), tags

### Eval Detail Page (`/evals/[evalId]`)

Two tabs: **Cases** and **Runs**.

**Cases tab:**
- Table: name, tags, description (truncated), last result (pass/fail/score badge)
- "Add Case" button
- Click row → case detail dialog or inline expand

**Runs tab:**
- Table: date, model, status, pass rate, avg score, avg turns, total tokens
- "Run Eval" button (with optional model override dropdown)
- Click row → run detail page

### Run Detail Page (`/evals/[evalId]/runs/[runId]`)

Header: run metadata (model, status, triggered by, duration).

Summary cards row: pass rate (large number), avg score, avg turns, avg latency, total tokens.

Results table: case name, status (pass/fail/error badge), score, turns, latency, tokens, session link (icon button → opens session in new tab).

Click case row → expand to show per-scorer results with pass/value/reason.

### Run Comparison (Phase 2)

Select two runs from the runs tab → side-by-side view:
- Summary metrics with delta indicators (green up / red down)
- Per-case comparison table with score deltas
- Highlights regressions (cases that passed before but fail now)

### Create Eval Form (`/evals/new`)

Fields: name, description, target type selector (session / app), target fields (varies by type), model override (optional), tags.

Target type selector:
- **Session**: harness (optional select, defaults to org default), agent (optional select), model (optional), system prompt (optional textarea)
- **App**: app ID (text input)

Cases added after creation on the detail page (simpler flow, avoids complex nested form).

### Add Case Dialog

Fields: name, description, tags, messages (textarea per message, add more button), max turns, timeout.

Scorers section: add scorer dropdown → type-specific config form. Each scorer shows a card with type, config, weight, remove button.

## Phases

| Phase | Scope | Entities |
|-------|-------|----------|
| **1** | Core entities, deterministic scorers, API, basic UI, eval runner workflow | Eval, EvalCase, EvalRun, EvalCaseResult |
| **2** | `llm_judge` scorer, run comparison UI, tag-based partial runs, "create case from session" | — |
| **3** | Scheduled eval runs (cron integration), App publish gates, cost estimation | — |

This spec covers Phase 1. Phase 2 and 3 are outlined for direction but not specified in detail.

## Non-Goals

- **Dataset management** — Evals are small curated collections, not large datasets
- **Fine-tuning loops** — Evals measure, they don't auto-fix
- **Cross-org benchmarking** — No public leaderboard or shared evals
- **Provider benchmarking** — Internal concern, not user-facing
- **Eval marketplace** — No sharing evals across organizations
