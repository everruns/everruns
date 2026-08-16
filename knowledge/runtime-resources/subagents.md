---
type: Specification
title: "Subagents Specification"
description: "Subagent orchestration."
tags:
  - everruns
  - runtime-resources
---
# Subagents Specification

Each spawned subagent is also tracked as a session task (`kind = subagent`,
`links.child_session_id` pointing at the child) with lifecycle `task.*`
events and a message channel, see
[`knowledge/runtime-resources/session-tasks.md`](session-tasks.md). The generic `list_tasks`,
`get_task`, `message_task`, and `cancel_task` tools work on subagents via
the `SubagentTaskExecutor`.

<!-- Design Decisions:
  - 1 delegation tool: spawn_agent(target.type = "subagent")
  - Background execution is the default: spawn returns immediately with a task_id;
    a detached watcher settles the task and the OnTerminal wake policy notifies the parent
  - mode: "foreground" blocks the parent tool call until the child idles (original Phase 1 behavior)
  - Governed nesting: subagents may spawn subagents up to max_subagent_depth
  - Root-tree task caps: a root session has bounded live and total descendant
    subagent tasks
  - Human-readable names by default ("Test Runner" not "test-runner")
  - message_task / cancel_task unify steering and cancellation via the task registry
  - Subagent inherits parent's harness and agent configuration
  - UI: dedicated Subagents tab with master-detail layout
-->

## Abstract

Subagents allow a host agent to delegate tasks to child sessions that run in their own context window. Each subagent is a full session with isolated message history, enabling parallel workstreams and separation of concerns within a single parent conversation.

Inspired by Claude Code's Agent tool, Cursor's sub-agents, and OpenAI Codex's multi-agent patterns.

## Design Principles

| Principle | Rationale |
|-----------|-----------|
| Single delegation tool | Minimal surface area. `spawn_agent(target.type = "subagent")` covers creation; lifecycle is managed via generic task tools. |
| Background-first | Parallelism is the point of subagents: spawn returns a `task_id` immediately and the parent keeps working; the task's `OnTerminal` wake policy notifies it on completion. `mode: "foreground"` opts back into block-and-return for results the parent cannot proceed without. |
| Governed nesting | Allows coordinator -> worker -> specialist flows while bounding runaway delegation. |
| Human-readable names | "Test Runner" is more natural than `test-runner` in conversation. |
| Inherit parent config | Subagent uses same harness, agent, and model. No capability escalation. |
| Generic lifecycle tools | `list_tasks`, `get_task`, `message_task`, `cancel_task` work for all task kinds including subagents. |

## Data Model

### Session Extensions

The `Session` entity is extended with subagent tree metadata:

| Field | Type | Description |
|-------|------|-------------|
| `parent_session_id` | SessionId? | Parent session (null for top-level sessions). Used to compute delegation depth. |
| `root_session_id` | SessionId? | Root of this session's delegation tree (EVE-680). A top-level session is its own root; a subagent child inherits its parent's root. Denormalized (migration 094) so a whole tree is one indexed query; mirrored onto `session_tasks.root_session_id` and filterable via `GET /v1/tasks?root_session_id=`. |

`subagent_name`, `subagent_task`, and `subagent_status` were retired in
migration 062. Lifecycle state is now tracked via `SessionTask` records
(`TASK_KIND_SUBAGENT`) owned by the parent session; use `list_tasks` /
`get_task` to read subagent status.

See `crates/core/src/session.rs` for full field list.

### SubagentStatus

```
Spawning → Running → Completed
                   → Failed
                   → Cancelled
                   → MaxIterationsReached
                   → Sealed
```

| Status | Description |
|--------|-------------|
| `Spawning` | Session created, task not yet sent |
| `Running` | Task sent, agentic loop active |
| `Completed` | Child session idled after processing task |
| `Failed` | Child session encountered an unrecoverable error |
| `Cancelled` | Parent cancelled via `cancel_task` |
| `MaxIterationsReached` | Child hit iteration limit |
| `Sealed` | Durable engine deliberately stopped the child's turn to prevent waste (no forward progress, or budget exhausted; see `SealReason`). Terminal and non-retryable, distinct from `Failed` so the parent can decide what to do next. The seal reason is surfaced in the child's final assistant message / spawn `result`. |

Terminal subagent statuses are derived from the child's terminal **turn event** (`turn.completed` / `turn.failed` / `turn.cancelled` / `turn.sealed`), not from the bare `idle` session status, a failed or sealed turn also leaves the session `idle`, so `idle` alone never settles a subagent.

### Database Migration

See `crates/server/migrations/007_v0.8.6.sql` for schema changes.

## Tools

### `spawn_agent` (`target.type = "subagent"`)

Sessions with `subagents` enabled advertise `subagent` in the shared
`spawn_agent` dispatcher's `target.type` enum. The dispatcher creates a child
session with `parent_session_id` set, creates a `TASK_KIND_SUBAGENT` task,
defaults to background mode, and supports `mode: "foreground"` for blocking
execution. The dispatcher can advertise other active known delegation providers
alongside `subagent` (for example first-party handoff or external A2A).

The retired direct creation entry point is no longer advertised to models.
`background` and `foreground` are the native shared execution-mode vocabulary
for every delegation provider; the dispatcher does not rewrite them per target.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name for the subagent |
| `instructions` | string | Yes | Instructions sent as first user message |
| `target.type` | string | Yes | Must be `subagent` |
| `goal` | string | No | Objective stored on the spawned `Session.goal` and injected at system-prompt level |
| `lifetime` | string | No | `linked` (default) creates a lifecycle child; `detached` creates an independent top-level peer session |
| `seed` | string | No | For `detached`: `fresh` (default), `fork`, or `workspace` |
| `mode` | string | No | `background` (default) or `foreground` |
| `blueprint` | string | No | Optional blueprint ID for a specialist child agent. |
| `config` | object | No | Blueprint-specific configuration. Only valid with `blueprint`. |
| `result_schema` | object | No | JSON Schema for the child session's final machine result. |
| `message_schema` | object | No | JSON Schema for structured progress messages from the child session. |
| `push_configs` | array | No | Per-task webhook targets (EVE-682). Each entry `{ url, secret?, event_filter? }`; `event_filter` is a subset of `terminal` (default), `awaiting_input`, `message`. URLs are SSRF-validated at spawn; embedded in the task spec and delivered alongside endpoint-created configs. See knowledge/runtime-resources/session-tasks.md. |

**Returns (background):** `task_id` and `status: "running"` immediately; the final result lands on the task record (`summary`) and the parent is woken on the terminal transition.

**Returns (foreground):** Last assistant message from the child session plus a `task_id` for the session task record.

When `result_schema` is present, the child session receives a `report_result`
tool whose parameters are the declared schema. A valid `report_result` call
writes the JSON object to `/.tasks/{task_id}/result.json` in the parent task
workspace and records `result_path` on the task. A child that reaches a
successful terminal status without reporting the result settles the task as
`failed` with `error.kind = "no_result"`. Foreground spawns return the reported
JSON object inline; without `result_schema`, foreground spawns keep returning
the last assistant message.

When `message_schema` is present, the child session receives a
`report_task_progress` tool whose parameters are the declared schema. A valid
`report_task_progress` call appends an outbound task message with a `data` part to
the parent task's message thread. The name is deliberately distinct from the
channel-facing `report_progress` tool (progress-reporting reply mode) so the two
never collide in a single session toolset. Background subagent tasks with
`message_schema` use `wake_policy: on_activity` so progress messages wake the
parent; tasks without `message_schema` keep completion-only wake-ups.

The schema validation, result-file settlement, and child-only reporting tools
are shared delegation infrastructure. Subagents consume the same implementation
as first-party agent handoffs; target-specific code only owns lifecycle and
session creation.

**Behavior (both modes):**
1. Creates a session. `lifetime = linked` sets `parent_session_id` to the current
   session; `lifetime = detached` leaves `parent_session_id = NULL` and records
   lineage with `forked_from_session_id`.
2. Inherits the parent session locale when present
3. Stores `goal` on the spawned session when provided; the runtime exposes it in
   a `<session-goal>` system-prompt block.
4. Creates a parent-owned task linked to the child session:
   `TASK_KIND_SUBAGENT` for linked sessions, `TASK_KIND_SESSION` for detached
   peer sessions. Detached task wake policy defaults to `silent`.

**Foreground:**
4. Sends `instructions` as first user message
5. Blocks on `wait_for_idle`
6. On child terminal turn status: returns last assistant message, task state → `succeeded`/`failed`/`canceled`
7. On child failure: returns error, task state → `failed`

**Background:**
4. Detaches a watcher (same pattern as `spawn_background` runs) and returns immediately; the watcher sends `instructions` as the first user message, deferred so local hosts, where `send_message` runs the child turn synchronously, do not block the spawn call
5. The task is created with `wake_policy: on_terminal`, or `on_activity` when `message_schema` is present; the watcher heartbeats the task registry (attempt-fenced) so the session task reaper can fail an orphaned watcher after worker loss
6. The watcher waits in slices until the child reaches a terminal turn status (overall cap 6 h), then settles the task and the durable spawn handle; the registry-level wake policy delivers the completion message to the parent (knowledge/runtime-resources/session-tasks.md, Wake-ups)
7. Local/embedded hosts (everruns-host) may report a bare `idle` after their synchronous turn, the watcher settles it as `completed`; hosted adapters never return bare `idle`
8. `SubagentTaskExecutor::reconcile` (invoked from `wait_task`'s poll loop) probes the child's terminal turn status and settles the task if the watcher died, so `wait_task` converges even after worker loss

**Degradation:** background mode requires a session task registry (it is the only surface for the result). An explicit `mode: "background"` without one is a tool error; an unspecified mode degrades to foreground so embedders without background tracking keep blocking semantics.

### Monitoring and steering subagents

Use the generic `session_tasks` tools after spawning. The `task_id` is returned
by `spawn_agent(target.type = "subagent")`.

| Tool | Description |
|------|-------------|
| `list_tasks` with `kind: "subagent"` | List all subagent tasks and their status |
| `get_task` | Get detailed status and result for a specific subagent |
| `message_task` | Send a steering message or additional context to a running subagent |
| `cancel_task` | Request cooperative cancellation of a subagent |
| `wait_task` | Block until a subagent reaches a terminal or interrupted state |

## Events

> **Retired:** the dedicated `subagent.*` SSE events (`subagent.spawned`,
> `subagent.completed`, `subagent.failed`, `subagent.cancelled`) were removed in
> EVE-585. Subagents are modeled as Session Tasks, so their lifecycle now surfaces
> through the `task.*` events on the **parent** session's event stream
> (`task.created`, `task.updated`, `task.message.sent`, `task.message.received`).
> See [events.md](../execution/events.md). The flow below predates the migration and is kept
> for the orchestration shape; the emitted event names are now `task.*`.

## Execution Flow

```
Parent Agent                          System                           Child Session
     │                                  │                                  │
     │  spawn_agent(target.type="subagent")      │                          │
     │─────────────────────────────────>│                                  │
     │                                  │  create session(parent_id=...)   │
     │                                  │─────────────────────────────────>│
     │                                  │  send instructions as message    │
     │                                  │─────────────────────────────────>│
     │                                  │  emit task.created               │
     │                                  │                                  │
     │          (blocked)               │         agentic loop             │
     │          wait_for_idle           │<────────────────────────────────>│
     │                                  │                                  │
     │                                  │  child idles                     │
     │                                  │<─────────────────────────────────│
     │                                  │  emit task.updated (succeeded)   │
     │  return last assistant message   │                                  │
     │<─────────────────────────────────│                                  │
     │                                  │                                  │
```

### Inheritance

The child session inherits:
- **Harness**: Same harness as parent session
- **Agent**: Same agent configuration (model, system prompt, capabilities)
- **Organization**: Same org scope (multitenancy)
- **Budget pool**: Session-scoped budgets resolve to the delegation tree's
  `root_session_id`, so every descendant turn spends from the root session's
  shared budget. Ledger attribution still records the child session that spent.

The child session does **not** inherit:
- Message history (clean context window)
- Session-level capabilities (only agent capabilities apply)
- Active turn state

## Spawn Governance

Subagent delegation walks the current session's `parent_session_id` chain and allows the spawn only when the new child depth is less than or equal to `max_subagent_depth`. Top-level sessions are depth 0; direct children are depth 1.

Detached spawns are peer sessions, not subagents. They reset nesting depth
(`parent_session_id` remains null; lineage is recorded separately via the fork
lineage fields) and are **not** counted by the subagent descendant caps. They are
instead bounded by their own detached caps against the origin tree root (see
**Detached spawn caps** below), so a loop of detached spawns cannot escape
governance (EVE-767 / TM-DOS-030). Before session creation, the host resolves
the current session's human owner and requires `OrgSessionsManage`
(`SESSION_MANAGE`). Denial is returned as a `ToolError`; the model cannot choose
the caller or bypass the host-injected authority.

The default maximum depth is 2, enabling A -> B -> C while rejecting a depth-3 child. Setting `max_subagent_depth` to 0 restores the previous hard block on spawning subagents. The policy is resolved as platform default, then org override, then agent/capability override; the current authored override is exposed through the `subagents` capability config.

When the cap is exceeded, the tool returns a `ToolError` naming the attempted depth and configured cap.

Subagent spawning also counts existing descendant subagent tasks by walking the
root session's task tree (`TASK_KIND_SUBAGENT` records and their
`links.child_session_id`). The model-facing `spawn_agent` tool is serialized by
the act scheduler, so same-batch spawn calls cannot all pass cap admission using
the same stale count. A new spawn is rejected before child creation when it
would exceed either:

- `max_active_descendant_tasks` (default 16): non-terminal descendant tasks
  under the root session, including `queued`, `running`, and `awaiting_input`
  tasks.
- `max_total_descendant_tasks` (default 200): all descendant subagent task
  records under the root session, including terminal records until retention
  prunes them.

Both caps are configurable on the `subagents` capability. `max_depth` remains
an alias for `max_subagent_depth`; `max_concurrent_descendant_tasks` is an alias
for `max_active_descendant_tasks`. Cap errors return `ToolError` messages that
name the configured limit and attempted count.

**Rationale:** Bounded nesting covers coordinator/delegator patterns, while
root-tree task caps bound wide fan-out and repeated retry loops even when depth
is shallow.

### Detached spawn caps

A detached spawn (`lifetime = detached`) resets depth but is admission-capped
against the **origin** subagent-tree root before the peer session is created.
The host authority resolves the origin root from the spawning session's stored,
org-scoped `root_session_id`, then the gate counts detached (`TASK_KIND_SESSION`) tasks
anywhere under that root, a BFS that follows every task's
`links.child_session_id`, so detached spawns made by subagents deeper in the tree
or by other detached peers all count against the origin root. A new detached
spawn is rejected before session creation when it would exceed either:

- `max_active_detached_tasks` (default 8): non-terminal detached peer tasks under
  the origin root.
- `max_total_detached_tasks` (default 50): all detached peer task records under
  the origin root, including terminal records until retention prunes them.

These caps are independent of the subagent descendant caps (a detached peer does
not consume the subagent budget and vice-versa) and are configurable on the
`subagents` capability. Cap errors return `ToolError` messages naming the
configured limit and attempted count. The same authority call returns the
org-validated origin root, which is passed as an internal-only session-creation
override. Detached peers and detached chains spend from that root budget in
addition to consuming the independent count cap (see `knowledge/security/budgeting.md`).

## Security Considerations

| Concern | Mitigation |
|---------|------------|
| Capability escalation | Subagent inherits parent capabilities exactly; no additional capabilities |
| Detached session creation | Host-injected authority evaluates `SESSION_MANAGE` for the resolved session owner before creation |
| Cross-org budget linkage | Internal override is stripped from public HTTP and storage resolves it with `org_id` before canonicalizing the root |
| Context isolation | Separate message history; child cannot read parent messages |
| Resource exhaustion | Foreground: 300s timeout on `wait_for_idle`. Background: 6h overall watcher cap; max iterations per child session; orphaned watchers are failed by the session task reaper on stale heartbeats |
| Runaway nesting/fan-out | `max_subagent_depth` enforced by bounded parent-chain walk; root-tree active and total descendant task caps are enforced before child session creation; detached peer spawns reset depth but are separately capped against the origin root (`max_active_detached_tasks` / `max_total_detached_tasks`) so a detached fan-out loop cannot escape governance (TM-DOS-030) |
| Org boundary | Child session inherits org_id; standard multitenancy enforcement applies |

## UI

Dedicated **Subagents** tab in the session view with master-detail layout:

- **Master list**: All subagents with name, status badge, task preview
- **Detail view**: Full conversation history of selected subagent
- Status badges use existing session status styling conventions
- Real-time updates via subagent SSE events

The `subagents` feature string is contributed when the subagent tools are available, controlling tab visibility (see [capabilities.md](../execution/capabilities.md#capability-features)).

## Phase 1b (Future)

Background mode shipped (default; see `spawn_agent` above), completion is
delivered through the task registry's `OnTerminal` wake policy rather than a
subagent-specific mechanism. Remaining candidates:

| Feature | Description |
|---------|-------------|
| Subagent results table | Durable tracking of subagent outcomes across sessions |
| Max iterations config | Per-subagent iteration limit (separate from session default) |
| Parallel spawn | Spawn multiple subagents in a single tool call |
| Steering messages | Completion notifications injected mid-turn into parent context |
