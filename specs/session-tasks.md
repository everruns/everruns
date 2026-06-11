# Session Tasks

Status: design proposal. Not implemented yet. When implemented, this spec
becomes the contract for the task registry and the tracking sections of
`specs/subagents.md`, `specs/a2a-capability.md`, and
`specs/background-execution.md` re-point here.

A session does work in the foreground (turns) and in the background (**tasks**).
A task is any asynchronous work owned by a session: a subagent, a delegated
external agent, a background tool run, a monitor. Tasks have one uniform
lifecycle, report progress, exchange messages with the session, may pause for
input, and end with a result. Tasks may hold **resources** (sandboxes, browser
sessions) — infrastructure that is leased and released, never "completed".

## Motivation

Background work is tracked today by three overlapping mechanisms with no shared
contract:

- `session_resources` mixes work (subagents, agent runs, background runs) with
  infrastructure (sandboxes, browser sessions) under one watered-down status
  enum (`active|completed|failed|released`); each kind smuggles its real state
  machine into `metadata`.
- Results follow three conventions: `/.agent-runs/{id}/result.json`,
  `/.background/{run_id}/`, and last-assistant-message plus a denormalized
  `subagent_results` table for subagents.
- Wake-ups are hand-rolled per capability (A2A injects a synthetic session
  message; subagents specced a second mechanism in their Phase 1b).
- Registry writes emit no events; the UI polls. The registry is visibility
  only: if a worker dies, entries linger until manual cleanup.
- There is no "needs input" state and no cooperative cancellation.

This spec replaces the work half of `session_resources` with a first-class
task model. The resource half (leased infrastructure) stays in
`specs/session-resources.md` and `specs/leased-resources.md`; tasks reference
the resources they hold.

Prior art informing the shape: A2A Tasks and the MCP Tasks extension (state
classes, `input-required` as a resumable state, deferred results), Temporal
(record vs append-only journal, cooperative cancel vs forced terminate,
attempt fencing), and id-reconciled status parts in streaming UIs
(snapshot-then-delta).

## Domain model

### Task record

One row per task in `session_tasks` (PostgreSQL; in-memory map in dev mode).
IDs use the `task_` prefix per `specs/id-schema.md`.

| Field | Meaning |
|---|---|
| `id` | `task_*` public ID |
| `session_id` | Owning session; `ON DELETE CASCADE` |
| `kind` | `subagent`, `external_agent`, `background_tool`, `monitor`, … |
| `display_name` | Human label ("Test Runner") |
| `spec` | Kind-specific input (instructions, tool args, external agent id) |
| `state` | See state machine below |
| `state_detail` | Short free text ("polling remote task", "iteration 4/10") |
| `progress` | `{current?, total?, unit?, label?}` — today's `BackgroundProgress` |
| `input_request` | Structured ask when `awaiting_input`; cleared on answer |
| `cancel_requested_at` | Cooperative cancel intent — a flag, not a state |
| `summary` | Human-readable outcome |
| `result_path` | Session VFS: `/.tasks/{task_id}/result.json` |
| `artifacts` | `[{name, type, path\|url}]` — files, PRs, child session links |
| `error` | `{kind, message}` — timeout/rejection are error kinds, not states |
| `attempt`, `worker_id`, `heartbeat_at` | Liveness and stale-attempt fencing |
| `links` | `{child_session_id?, remote_task_id?, resource_ids: []}` |
| `wake_policy` | When outbound activity wakes the parent (see Wake-ups) |
| `created_at`, `started_at`, `finished_at`, `updated_at` | Timestamps |

### State machine

Six states in three classes. Everything else is data, not state.

```
              ┌─────────────────────────────┐
queued ──► running ◄──► awaiting_input      │ active / interrupted
              │               │             │
              ▼               ▼             ▼
         succeeded         failed       canceled    terminal
```

- **Active**: `queued` (accepted, not started), `running`.
- **Interrupted**: `awaiting_input` — the task asked the session (agent or
  human) for something and is resumable. First-class, not an error. Subsumes
  human-in-the-loop approvals and A2A `input-required`/`auth-required`.
- **Terminal**: `succeeded`, `failed`, `canceled`.

Cancellation is cooperative and idempotent: `cancel_task` sets
`cancel_requested_at` and asks the executor to wind down; the task may still
end `succeeded` or `failed`. Remote state machines map onto this one (A2A:
`submitted→queued`, `working→running`, `input/auth-required→awaiting_input`,
`rejected`/`timed_out`→`failed` with `error.kind`).

### Messages

Tasks and the session communicate through a bidirectional, persisted message
channel — the generalization of today's `message_subagent`/`message_agent` and
the A2A/subagent wake-up messages.

```
TaskMessage {
  id, direction: inbound | outbound,   // inbound = session→task
  content: [Part],                     // text/file/data parts
  in_reply_to: Option<InputRequestId>, // set when answering an input request
  created_at,
}
```

- Messages persist to the session event journal (`task.message.sent` /
  `task.message.received`), giving each task a replayable thread.
- Answering an input request is an inbound message with `in_reply_to` set;
  delivery clears `input_request` and returns the task to `running`.
- For subagents the channel carries only cross-boundary messages — the child
  transcript is not mirrored; `links.child_session_id` points at the full
  conversation.

## Contracts

Two traits, one per direction of control. The registry owns the record,
lifecycle invariants, events, durability, and recovery; capabilities plug in
executors.

```rust
/// Registry → executor: control plane for a task kind.
trait TaskExecutor {
    fn kind(&self) -> &str;
    async fn start(&self, task, ctx) -> Result<()>;     // begin, or re-attach after worker loss
    async fn deliver(&self, task, msg, ctx) -> Result<()>; // inbound messages + input answers
    async fn cancel(&self, task, ctx) -> Result<()>;    // cooperative
    async fn reconcile(&self, task, ctx) -> Result<()>; // polled kinds; reports via sink
}

/// Running work → registry: report plane. Generalizes BackgroundEventSink.
trait TaskSink {
    async fn state(&self, state, detail);
    async fn progress(&self, progress);
    async fn output(&self, stream, delta);       // high-frequency, ephemeral
    async fn post(&self, msg);                   // outbound message; may wake parent
    async fn request_input(&self, req);          // → awaiting_input
    async fn artifact(&self, artifact);
}
```

The asymmetry is deliberate: the registry calls into the executor; the running
work pushes into the sink. In-process work holds the sink directly; remote
kinds push from inside `reconcile` as polling translates remote state.

`state`, `progress`, and `request_input` mutate the task record (UI chips
update); `post` and `output` are content (thread and stream). This keeps
`task.updated` snapshots small while threads and output grow elsewhere.

### Kind mappings

| Kind | `start` | `deliver` | outbound translation |
|---|---|---|---|
| `subagent` | create child session, send instructions | `send_message(child)` | child question → `request_input`; final message → `post` + terminal state |
| `external_agent` | A2A `message/send` | `message/send` with `remote_task_id` | `reconcile` polls `tasks/get`; remote artifacts → `artifact` |
| `background_tool` | run `execute_background` with the sink | rarely used | direct sink calls (existing `BackgroundEventSink` is a strict subset) |
| `monitor` (future) | start watch loop | adjust what's watched | observation → `post`/`artifact`; triggered condition → `request_input` |

A monitor is a long-lived task (`running` until canceled). If recurrence or
schedules are needed later, add a separate definition entity that spawns tasks
(Temporal's definition/execution split) without touching the task model.

## Results and artifacts

Results are modeled apart from status:

- **Machine result**: what a synchronous call would have returned, at
  `/.tasks/{task_id}/result.json` (one convention replacing `/.agent-runs/`
  and `/.background/`). Logs and large output live under the same directory.
- **Human summary**: short `summary` on the record.
- **Artifacts**: typed links (file, PR, child session) on the record.

Retention/expiry of task results is explicitly out of scope for v1; the
columns (`finished_at`, `artifacts`) are designed so a TTL policy can be added
without schema change.

## Events and UI

Two snapshot-carrying event types on the existing session event stream
(`specs/events.md`), plus the message events above:

- `task.created` — full task snapshot.
- `task.updated` — full task snapshot on every state, progress, or
  `state_detail` transition. Consumers never need a follow-up read.

Riding the events table gives persistence, per-session ordering, and
`since_id` SSE replay with no new infrastructure. UIs follow
snapshot-then-delta: fetch `GET /v1/sessions/{id}/tasks`, then reconcile
`task.updated` by `task_id`. High-frequency `output` deltas stay ephemeral
(NATS / VFS tail) and render only on the task detail view.

UI shape: a session activity rail — one chip per task (kind icon, name, state
badge, progress, live `state_detail`), an inline form when `awaiting_input`,
cancel, and on completion the summary plus artifact links. Chips render purely
from the snapshot, so new kinds appear with no frontend work. The current
resources tab becomes tasks + resources.

## Agent-facing tools

Generic tools replace the per-kind query/messaging tools:

```
list_tasks / get_task   — state, detail, progress, summary, result_path,
                          recent thread; get_task supports an output cursor
                          (only new output since last check)
message_task            — inbound message; subsumes message_subagent,
                          message_agent, and input answers (in_reply_to)
cancel_task             — sets cancel intent
wait_task               — generic foreground wait; subsumes wait_agent
```

Spawning stays per-capability (`spawn_subagent`, `spawn_agent`,
`spawn_background`) because input schemas genuinely differ — but every spawn
creates a task and returns its `task_id`. Blocking (foreground) spawns also
create task records: same object, and the UI shows it live while the parent
turn waits; background is a mode, not a different entity.

Naming cleanup: today `task` parameters carry instruction text
(`subagent_task`, `spawn_subagent(task:)`). These rename to `instructions` so
"task" unambiguously means the lifecycle object.

## Wake-ups

Waking the parent is a registry-level delivery policy on outbound activity,
not per-capability code. `wake_policy` on the task selects: wake on terminal
transition, wake on any `post`/`request_input`, or silent (parent polls).
Delivery uses the existing steering-message mechanism. This replaces the A2A
synthetic wake-up message and pre-empts the subagent Phase 1b variant.

## Durability and recovery

Unlike the v1 `session_resources` registry, tasks are durable:

- Executors heartbeat through the sink; `worker_id`/`heartbeat_at` record
  liveness.
- A background reconciler (same pattern as leased-resource cleanup:
  claim with `FOR UPDATE SKIP LOCKED`) finds tasks with stale heartbeats and
  either re-attaches (`start` on a new worker, `attempt + 1`) or marks them
  `failed` with `error.kind = "orphaned"`.
- Stale attempts are fenced: sink writes carry `attempt` and writes from a
  superseded attempt are rejected.

## Relationship to session resources

Complementary, not merged. `session_resources` narrows back to leased
infrastructure (sandbox, browser_session, voice_connection, sprite) with its
existing lease/cleanup semantics. A task lists the resources it holds in
`links.resource_ids`; releasing a session's resources on task failure becomes
a registry policy rather than per-capability cleanup.

## API

```
GET  /v1/sessions/{session_id}/tasks            — list (filter by state/kind)
GET  /v1/sessions/{session_id}/tasks/{task_id}  — snapshot + recent thread
POST /v1/sessions/{session_id}/tasks/{task_id}/messages — inbound message
POST /v1/sessions/{session_id}/tasks/{task_id}/cancel   — cancel intent
```

Webhooks on terminal transitions are out of scope for v1.

## Migration

No backward compatibility is required; data migrates forward once:

- `session_resources` rows with kind `subagent`, `agent_run`,
  `background_run`, `agent_handoff` backfill into `session_tasks`
  (`agent_run` → `external_agent`; the A2A `AgentRunRecord` maps
  field-for-field). Remaining kinds stay as resources.
- `subagent_results` folds into `session_tasks` and is dropped.
- `sessions.subagent_status`/`subagent_task` become derivable from the task
  record; `parent_session_id` stays.
- `subagent.*` event types are superseded by `task.*` events.
- `get_subagents`, `get_agent_runs`, `wait_agent`, `message_agent`,
  `message_subagent` retire in favor of the generic tools.
- `GET /v1/sessions/{id}/resources` keeps serving infrastructure resources.

## Out of scope (v1)

- Webhooks / push notifications on task transitions.
- Result retention and TTL policies.
- Task definitions / recurrence (monitors ship as long-lived tasks first).
- Cross-session or org-scoped task queries.
