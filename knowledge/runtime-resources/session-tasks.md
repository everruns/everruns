---
type: Specification
title: "Session Tasks"
description: "Session task registry for background work."
tags:
  - everruns
  - runtime-resources
---
# Session Tasks

Status: implemented (v1). This spec is the contract for the task registry.
Phased-out pieces and deviations are listed under Implementation notes below.

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
`knowledge/runtime-resources/session-resources.md` and `knowledge/runtime-resources/leased-resources.md`; tasks reference
the resources they hold.

Prior art informing the shape: A2A Tasks and the MCP Tasks extension (state
classes, `input-required` as a resumable state, deferred results), Temporal
(record vs append-only journal, cooperative cancel vs forced terminate,
attempt fencing), and id-reconciled status parts in streaming UIs
(snapshot-then-delta).

## Domain model

### Task record

One row per task in `session_tasks` (PostgreSQL; in-memory map in dev mode).
IDs use the `task_` prefix per `knowledge/foundations/id-schema.md`.

| Field | Meaning |
|---|---|
| `id` | `task_*` public ID |
| `session_id` | Owning session; `ON DELETE CASCADE` |
| `kind` | `subagent`, `session`, `agent_handoff`, `external_agent`, `background_tool`, `monitor`, … |
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

- Messages persist to the `session_task_messages` table (the queryable
  thread) and are mirrored as `task.message.sent` / `task.message.received`
  events on the session stream for live UIs.
- Answering an input request is an inbound message with `in_reply_to` set;
  delivery clears `input_request` and returns the task to `running`.
- For subagents the channel carries only cross-boundary messages — the child
  transcript is not mirrored; `links.child_session_id` points at the full
  conversation.
- Schema-bound local delegation children can post structured progress with a child-only
  `report_task_progress` tool. Valid calls append outbound messages containing a
  single `data` part; invalid payloads return a validation error so the child
  can retry.

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
| `session` | create a detached peer session | n/a | `cancel` cooperatively cancels the peer session (standard send/cancel path) and settles the tracking task `canceled` — cancel means cancel, not detach-only (EVE-766) |
| `monitor` | created by `spawn_background` with a `schedule` arg | n/a (schedule-driven) | schedule fire → probe runs (or placeholder message); one-shot → `succeeded`; recurring stays `running` |

A monitor is a long-lived task (`running` until canceled or exhausted).
`spawn_background` with a `schedule` argument creates a `monitor` task linked
to the backing session schedule via `spec["schedule_id"]`. The monitor spec
also stores `spec["tool"]` and `spec["arguments"]` — the probe tool to run on
each fire.

On each fire the session scheduler executes the probe tool directly (using a
small built-in registry of context-free probe tools) and records the result as
an outbound message on the monitor's thread. Because the probe runs autonomously, no agent turn is
started — the session LLM is not involved.

A plain `"Monitor fired at …"` placeholder is recorded and the normal
scheduled session turn runs instead when any of the following are true:

- No tool registry is available at startup.
- `spec["tool"]` is absent or empty.
- The named tool is not in the built-in probe registry. Network, filesystem,
  storage, and other context-sensitive tools are intentionally excluded until
  probe execution can use the same fully populated `ToolContext` as worker/API
  executor paths.
- The tool returns `InternalError` or `ConnectionRequired` (non-observable outcomes).

One-shot monitors transition to `succeeded` after their single fire; recurring
monitors stay `running` until `cancel_task` is called, which cancels the linked
schedule and transitions the task to `canceled`.

If the underlying schedule is canceled or deleted directly (not via `cancel_task`
or the API cancel endpoint), the session scheduler's periodic reconciliation sweep
(~60 s cadence) detects the orphaned monitor and transitions it to `canceled`
(`state_detail: "schedule canceled"`). Prefer `cancel_task` or `POST …/cancel`
when both atomicity and immediacy matter.

## Results and artifacts

Results are modeled apart from status:

- **Machine result**: what a synchronous call would have returned, at
  `/.tasks/{task_id}/result.json` (one convention replacing `/.agent-runs/`
  and `/.background/`). Logs and large output live under the same directory.
- **Human summary**: short `summary` on the record.
- **Artifacts**: typed links (file, PR, child session) on the record.

Task kinds may require a structured result by storing `spec.result_schema` as a
JSON Schema. Local child-session kinds (`subagent`, `session`, and
`agent_handoff`) inject the shared child-only `report_result` tool. It validates
its arguments, writes the JSON object to `/.tasks/{task_id}/result.json`, and
updates the task record. If a schema-bound local child reaches a successful
terminal status without a recorded `result_path`, the task settles as `failed`
with `error.kind = "no_result"`.

External A2A tasks cannot receive local tools. For `external_agent`, the first
structured data artifact on the terminal A2A task is validated against
`spec.result_schema`. A valid value uses the same task-result path; a mismatch
settles as `failed`/`schema_mismatch`, and absent data settles as
`failed`/`no_result`. Tasks without `spec.result_schema` keep their existing
summary-only or legacy snapshot behavior.

Local child-session tasks may also declare `spec.message_schema`. That schema
injects the shared child-only `report_task_progress` tool; valid calls are
recorded as outbound task messages with `data` content, and background tasks
use `wake_policy: on_activity` so those messages wake the parent session.
External A2A spawning rejects `message_schema` explicitly.

### Retention (TTL)

Terminal task records are pruned on a global TTL (EVE-580). The retention pass
runs inside the existing `session_task_reaper` durable activity (see Durability
and recovery): on each tick, after the orphan reconciliation, it deletes a
bounded batch of tasks in a terminal state (`succeeded`/`failed`/`canceled`)
whose `finished_at` is older than `now - TTL`, together with their
`session_task_messages` and their `result_path` artifact subtree
(`/.tasks/{task_id}`). Live tasks (`queued`/`running`/`awaiting_input`) and
terminal tasks newer than the TTL are never touched — the prune predicate is
strictly `state IN (terminal) AND finished_at < cutoff`.

- **Configuration**: a single global TTL via `SESSION_TASK_RETENTION_TTL_SECONDS`
  (default 30 days), seeded into the reaper activity input through the same
  bootstrap path as the reaper interval (`SessionTaskReaperInput::from_env`).
  `0` disables retention (records live forever — the pre-EVE-580 behavior).
  A per-org override is a follow-up, not this issue; the global TTL is the
  documented extension point.
- **Bounded work**: each pass prunes at most `retention_limit` tasks (default
  100), draining a backlog across successive ticks so a large backlog cannot
  wedge the tick or blow memory (mirrors the orphan-scan and blob-GC bounds).
- **Artifact cleanup and ordering**: rows (and cascading messages) are deleted
  first; `result_path` artifacts are removed afterwards through the existing
  session-file deletion seam (`delete_session_file_recursive`, which clears
  backing blobs on the object-storage backend). A crash between the two can at
  worst leak a dangling blob — reclaimed by blob GC (`crates/server/src/blob_gc.rs`)
  — rather than leave a row pointing at a deleted artifact. Artifact deletion
  is best-effort and never fails the prune.
- **Tenant scoping**: the query is global/by-age, but every delete is keyed on
  the task's own primary key, so it cannot cross-delete between orgs (TM-TENANT).

Source: `crates/worker/src/session_task_reaper.rs` (pass + config),
`crates/server/src/storage/backend.rs`
(`prune_terminal_session_tasks_with_artifacts`), backed by a partial index on
terminal `finished_at` (migration 075).

## Events and UI

Two snapshot-carrying event types on the existing session event stream
(`knowledge/execution/events.md`), plus the message events above:

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

A cross-session **Work view** (EVE-756) reuses the same snapshot-driven chips
and the per-session task detail card over the org-scoped `GET /v1/tasks`,
grouping tasks by `root_session_id` into a delegation tree (root session →
owning sessions → tasks). It has no org-wide event stream, so it reconciles the
same `task.created`/`task.updated` snapshots by `task_id` across one per-session
SSE subscription per owning session.

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

Delegation spawning uses the unified `spawn_agent` surface. Known delegation
providers share one dispatcher; its `target.type` enum reflects the active
providers (`subagent`, `agent`, and/or `external_a2a`). Every spawn creates a
task and returns its `task_id`. Blocking
(foreground) spawns also create task records: same object, and the UI shows it
live while the parent turn waits; background is a mode, not a different entity.
All delegation providers parse the shared `background | foreground` execution
vocabulary natively; the dispatcher owns one model-facing enum and performs no
provider-specific mode translation.

Naming cleanup: `task` parameters that carry instruction text were renamed to
`instructions` so "task" unambiguously means the lifecycle object. The
`sessions.subagent_task`, `subagent_name`, and `subagent_status` DB columns
were retired in migration 062.

## Wake-ups

Waking the parent is a registry-level delivery policy on outbound activity,
not per-capability code. `wake_policy` on the task selects: wake on terminal
transition, wake on any `post`/`request_input`, or silent (parent polls). This
replaces the A2A synthetic wake-up message and pre-empts the subagent Phase 1b
variant.

There are two delivery paths, selected by whether the parent has an active turn:

- **Idle parent → between-turn steering.** The waker injects a synthetic user
  message and starts (or steers) a turn. Unchanged.
- **Active parent → mid-turn injection.** The wake payload (task snapshot plus
  the outbound message / `report_progress` data) is enqueued and consumed at the
  parent's next agentic-loop iteration boundary — before the next LLM call —
  appearing as injected context alongside the reloaded conversation, so the
  parent reacts within the same turn instead of after it idles.

The mid-turn queue lives behind the registry seam (`SessionWakeQueue` in
`everruns-core`, fed via the `TaskTransitionObserver` seam by an
`ObservingTaskRegistry` decorator over any registry, so runtime and server
share it). The turn loop drains it at each reason iteration boundary.

**Exactly-once.** The queue is the single source of truth for an undelivered
wake: each real transition enqueues one entry (the registry fires each
transition once per observer), and `drain` atomically removes a session's
entries under one lock — that removal *is* the claim. A wake is therefore
delivered mid-turn (drained by a running turn's next iteration) **xor** queued
for the next turn (drained by that turn's first iteration), never both. Turn
cancellation, seal, and max-iterations leave undrained wakes in the queue for
the next turn (between-turn fallback); a completion landing mid-loop is visible
to the very next iteration. Within a process the queue gives exactly-once;
durable exactly-once across a worker restart is a property of the *persistent*
transition source (the durable signal store), fenced by task `attempt`
(`expected_attempt`), not of the in-memory queue.

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
GET  /v1/tasks                                  — org-scoped list (state/kind/created_after/root_session_id, newest-first, bounded limit)
GET  /v1/sessions/{session_id}/tasks            — list (filter by state/kind)
GET  /v1/sessions/{session_id}/tasks/{task_id}  — snapshot + recent thread
POST /v1/sessions/{session_id}/tasks/{task_id}/messages — inbound message
POST /v1/sessions/{session_id}/tasks/{task_id}/cancel   — cancel intent
GET    /v1/sessions/{session_id}/tasks/{task_id}/push-configs             — list per-task push configs
POST   /v1/sessions/{session_id}/tasks/{task_id}/push-configs             — create a per-task push config
DELETE /v1/sessions/{session_id}/tasks/{task_id}/push-configs/{config_id} — delete a per-task push config
```

`GET /v1/tasks` (EVE-583) is the cross-session, org-scoped query for
ops/observability: it lists tasks across every session in the caller's org,
newest-first, with optional `state`, `kind`, and `created_after` (RFC3339) age
filters and a bounded `limit` (default 100, max 500). The org is taken from the
authenticated caller, never from input; scoping is a semijoin on
`sessions.org_id` (the authoritative tenant boundary), backed by the
`session_tasks (created_at DESC)` index.

The optional `root_session_id` filter (EVE-680) narrows the list to a single
delegation tree — the root session's own tasks plus every descendant's. A
session's tree root is denormalized onto `sessions.root_session_id` (a top-level
session is its own root; a subagent child inherits its parent's root, set at
session creation and backfilled by migration 094). A detached `session` task's
peer is created with an internal, org-validated root override, so detached task
chains stay grouped with and spend against the origin tree; ordinary forks do
not set this override. The root is mirrored onto
`session_tasks.root_session_id` at task creation, so the whole tree is one
indexed lookup with no parent-chain walk. The filter parses to a session id and
stays inside the org semijoin, so it never crosses the tenant boundary.

Message posts and cancels both invoke the kind's `TaskExecutor` best-effort
after the durable registry write: the message is recorded and the cancel intent
is set regardless of executor outcome. A delivery or cancel error is logged at
`warn` and never fails the HTTP call. Executor calls from the API run under the
session's effective network ACL (harness chain → agent → session overlay fold);
if the ACL cannot be computed the executor call is skipped with a `warn`.

Specifically:
- `POST …/messages`: rejected with 400 for `subagent`- and `agent_handoff`-kind
  tasks — both are steered exclusively by the parent agent (via the
  `message_task` tool); `links.child_session_id` exposes the child session for
  direct addressing. For all other kinds the message is recorded durably, the task is re-fetched (it
  may have transitioned to `running` if the message answered an input request),
  and `executor.deliver` is called. For kinds without a registered executor the
  message is still recorded (delivery = no-op).
- `POST …/cancel`: sets `cancel_requested_at`, then calls `executor.cancel`.
  `MonitorTaskExecutor.cancel` disables the linked schedule and transitions
  the task to `canceled` — so API cancel of a monitor task now atomically
  cancels the schedule too. The response reflects the task state after the
  executor runs (re-fetched), so a monitor task is returned as `canceled`.

### Webhooks and per-task push configs

Two independent delivery surfaces exist, both signed with HMAC-SHA256 when a
secret is set (`X-Everruns-Signature: sha256=<hex>`) and both delivered through
the same SSRF-guarded egress path (`build_task_webhook_request`, which pins DNS
to the create-time resolution to defeat rebinding; EVE-625):

- Organization task webhooks (`organization_task_webhooks`, EVE-579): org-scoped,
  fire only on terminal transitions (succeeded / failed / canceled). Managed via
  `/v1/task-webhooks`. Unchanged by EVE-682.
- Per-task push configs (`session_task_push_configs`, EVE-682): scoped to a
  single task via `{ url, secret?, event_filter? }`, modeled on A2A
  `TaskPushNotificationConfig`. `event_filter` is a subset of `terminal`
  (default), `awaiting_input`, `message`; a config opts into non-terminal
  delivery by listing those events. The table has no `org_id` — a config is
  reachable only through its owning session, so the endpoint authorizes by
  verifying the task's session belongs to the caller's org (the notifier resolves
  the same boundary server-internally via `get_session_unscoped`). Responses
  return a `tpc_<id>` public id and a `has_secret` boolean; the stored secret is
  never echoed. URLs are `validate_safe_url`-checked before persistence.

Per-task configs can be created two ways, sharing one delivery path: via the
`push-configs` endpoint (persisted to the table) or at subagent spawn time via
the `push_configs` spawn arg (embedded in the task spec under `push_configs`;
the notifier reads both). The registry fires the notifier on terminal,
awaiting_input, and outbound-message transitions; per-config `event_filter`
selects which land. Delivery is best-effort — failures are logged, never fatal,
and never block the task update.

### Transition observers (in-process, EVE-729)

Webhook delivery is one consumer of a lower-level seam: a `SessionTaskRegistry`
fires each real transition (terminal / awaiting_input / outbound message) once to
every registered `TaskTransitionObserver`. The trait and its `TaskTransition`
enum live in `everruns-core` (`task_observer`) so `everruns-host` embedders
can observe task transitions in process — with the same filter semantics — without
HTTP or a dependency on the control-plane server. The server's webhook dispatcher
(`DirectTaskWebhookNotifier`) is one implementation registered via
`with_transition_observer`; embedders register their own. Because both share the
registry's single transition-detection path, an in-process observer receives
exactly the transitions the webhook path fires (asserted by the parity test in
`crates/server/src/storage/session_task_store.rs`). Dispatch is best-effort and
off the task-update path: each observer runs on its own detached task, so one
slow observer never blocks task updates or another observer.

## Migration

No backward compatibility is required; data migrates forward once:

- `session_resources` rows with kind `subagent`, `agent_run`,
  `background_run`, `agent_handoff` backfill into `session_tasks`
  (`agent_run` → `external_agent`; the A2A `AgentRunRecord` maps
  field-for-field). Remaining kinds stay as resources.
- `subagent_results` folds into `session_tasks` and is dropped.
- `sessions.subagent_status`/`subagent_task`/`subagent_name` were dropped
  (migration 062); `parent_session_id` stays as delegation tree metadata.
- `subagent.*` event types are superseded by `task.*` events.
- `get_subagents`, `get_agent_runs`, `wait_agent`, `message_agent`,
  `message_subagent`, `cancel_agent` — **retired (done)**. These per-kind tools have
  been removed; all listing, waiting, messaging, and cancellation now routes
  through the generic tools (`list_tasks`, `get_task`, `message_task`,
  `cancel_task`, `wait_task`).
- Direct delegation entry points for subagents and first-party handoffs are
  retired; use `spawn_agent(target.type = "subagent" | "agent")` instead.
- `GET /v1/sessions/{id}/resources` keeps serving infrastructure resources.

## Implementation notes (v1)

- `monitor` kind is first-class as of this implementation. `spawn_background`
  with a `schedule` argument creates a `monitor` task (kind = "monitor") linked
  to the session schedule via `spec["schedule_id"]`. The `session_scheduler`
  server loop finds matching monitors on each schedule fire, records an outbound
  message on their thread, and completes one-shot monitors (cron_expression
  absent) to `succeeded`. Recurring monitors stay `running` until
  `cancel_task`, which cancels the linked schedule via `MonitorTaskExecutor`.
  `TASK_KIND_MONITOR` and the other `TASK_KIND_*` constants are now re-exported
  from `everruns-core`.

- Storage: `session_tasks` + `session_task_messages` (migration 053);
  PostgreSQL and in-memory backends both route updates through
  `apply_task_update` in `crates/core/src/session_task.rs`. gRPC workers get
  the registry via task RPCs in the internal worker protocol; task and message
  payloads travel as native protobuf messages (EVE-642), serialized once by
  protobuf framing rather than JSON-encoded into byte fields. The proto↔core
  conversions live in `everruns-internal-protocol`; everruns-core stays the
  source of truth for lifecycle invariants.
- Session-resource dual-write retired (migration 054): subagent, background_run,
  and agent_handoff no longer register in `session_resources`. A2A agent runs
  (`external_agent` tasks) now store their run records in session storage KV
  (`agent_run:{run_id}` keys). Legacy `subagent.*` events are still
  emitted for external consumers, but the CLI now renders the `task.*`
  lifecycle instead; retiring the legacy emission awaits a compatibility
  decision (knowledge/execution/events.md). The `task` → `instructions` parameter rename is done for
  model-facing delegation parameters. The
  `sessions.subagent_*` columns (`subagent_name`, `subagent_task`, `subagent_status`)
  were retired in migration 062; `parent_session_id` is kept as delegation tree
  metadata and is now set at session creation time.
- Durability: `attempt`/`worker_id`/`heartbeat_at` are stored. The orphan
  reconciler (`session_task_reaper` durable activity, every 60 s) finds tasks
  with stale heartbeats (`heartbeat_at IS NOT NULL AND heartbeat_at < now -
  5m`) and, for re-attachable kinds, re-attaches them; otherwise fails them via
  the registry using `FOR UPDATE SKIP LOCKED` on the PG backend. Tasks with
  `NULL heartbeat_at` (foreground subagent tasks) are excluded (covered by
  EVE-535 spawn handles). The reconciler now runs on gRPC workers via the
  `ListOrphanedSessionTasks` RPC (added to the internal worker protocol);
  `session_task_reaper` is included in the gRPC task worker's default
  activity types. Stale-attempt fencing is built: `SessionTaskUpdate` carries
  an optional `expected_attempt` field; `apply_task_update` silently drops any
  update where `expected_attempt` is set but does not match `task.attempt`.
  When the reaper fails an orphan it sets `increment_attempt`, bumping
  `task.attempt` so the superseded executor's heartbeats, state writes, and
  message posts (`NewTaskMessage.expected_attempt`, enforced in
  `record_message`) are rejected. Writers that do not track attempts
  (e.g. `cancel_task` from the API) leave `expected_attempt: None` and are
  unaffected.
- Re-attach: `TaskExecutor` now has `fn can_reattach(&self) -> bool` (default
  `false`). Kinds returning `true` must implement `start` to resume idempotently
  with the new attempt. The reaper re-attaches up to `max_attempts` (default 3,
  configurable in `SessionTaskReaperInput`): it atomically supersedes with
  `increment_attempt` + `expected_attempt` fence, builds a minimal `ToolContext`
  (storage_store + registry + egress), calls `executor.start(&updated_task,
  &ctx)`, and on `start` error falls back to orphaned-fail with the attempt
  already bumped. On attempt ≥ max_attempts the task is failed as orphaned
  immediately. `ExternalAgentTaskExecutor` (`external_agent` kind) implements
  `can_reattach → true`: loads the `AgentRunRecord` from session storage; if
  terminal mirrors the state and returns; if `remote_task_id` is absent returns
  an error (caller falls back to orphaned); otherwise rebuilds the A2A client
  and resumes `background_monitor` with `heartbeat_attempt = task.attempt`.
  The background poll loop now heartbeats the registry on every poll iteration
  when `heartbeat_attempt` is provided, fencing stale writes from superseded
  executors. All other kinds still fail immediately as orphaned.
- Wake-ups: `wake_policy` is enforced at the registry level
  (`DbSessionTaskRegistry`). `OnTerminal` wakes on any transition into
  `succeeded`/`failed`/`canceled`. `OnActivity` additionally wakes on
  `awaiting_input` entry and outbound messages. `Silent` never wakes.
  A2A's legacy `wake_parent` call is gated on `session_task_registry.is_none()`
  (backward compat for sessions without a registry).
- Mid-turn delivery (EVE-681, part A): `SessionWakeQueue`
  (`crates/core/src/wake_queue.rs`) is a per-session queue behind the registry
  seam; `ObservingTaskRegistry` (`crates/core/src/task_observer.rs`) is a
  storage-agnostic decorator that fans qualifying transitions to observers
  (the reusable form of `DbSessionTaskRegistry`'s inline fan-out). The
  `everruns-host` in-process loop wraps its injected registry with this
  decorator + queue and drains the queue at every reason iteration boundary
  (`InProcessRuntime::drain_and_inject_wakes`), injecting each wake as a user
  message before the LLM call and continuing a would-idle turn while wakes are
  pending. `wake_text_for` renders the same terminal/awaiting_input/message text
  the between-turn waker uses, so both paths agree on *when* a wake fires. The
  server durable-worker path (draining the queue at
  `unified_worker::schedule_next_activity`'s signal-consume boundary, persisted
  through the durable signal store, with exactly-once across worker restart) is
  **not** wired in part A — it needs live Postgres/NATS/gRPC to validate and is
  deferred to a reviewed follow-up. The cross-session Work view (part B, EVE-756)
  builds on EVE-680's `root_session_id` and is now implemented in `apps/ui`
  (grouped delegation tree over `GET /v1/tasks`, reusing the per-session chips
  and task detail).
- Background tool cancellation is cooperative: runs with a task record
  heartbeat every ~2s and poll `cancel_requested_at`, winding down to
  `canceled` when set (works across worker processes).
- `LLMSIM_DEMO=tasks` drives the full lifecycle end-to-end without an LLM
  key (see `crates/llmsim/src/lib.rs`).

## Recurrence and task definitions (decision: defer — EVE-584)

**Question:** does the system need a first-class `TaskDefinition` (a reusable
*template* + *recurrence* primitive that instantiates fresh task instances on a
cadence), or is the existing schedule + monitor composition sufficient?

**Decision: no dedicated primitive in v1 — recurrence is expressed by
composition.** Two existing primitives already cover recurring background work:

- A recurring **session schedule** (`cron_expression`, via the `session_schedule`
  capability — `docs/capabilities/session-schedules.md`) fires on a cadence.
- A **monitor** task (`spawn_background` with a `schedule` arg) binds that
  schedule to a long-lived task: each fire runs the probe tool and records the
  result on the task thread (recurring monitors stay `running`; one-shot ones go
  `succeeded`). A bare recurring schedule without a monitor simply delivers a
  scheduled turn to the session.

A separate `TaskDefinition` primitive would add a parallel concept plus
schema/API/UI surface. Its only capability beyond the monitor model is
"**each fire yields a distinct task instance** with its own lifecycle, result,
and retention" (versus one long-lived monitor that accumulates messages), plus
reusable templates instantiated across sessions. There is no concrete demand for
either today, so building it now would be speculative surface area.

**Revisit if** any of these appear: (1) a need for each recurrence to produce an
independent task instance (own result/retention/success-history) rather than a
single long-lived monitor; (2) reusable, parameterized task templates
instantiated across sessions or agents; (3) a need to define the cadence
independently of a specific session's monitor wiring. Until then, recurrence
stays as `schedule + monitor` composition.

## Out of scope (v1)

- Per-org retention TTL overrides (global TTL ships first; see Retention).
- A dedicated task-definition / recurrence primitive — see the decision above;
  recurrence ships as schedule + monitor composition.
