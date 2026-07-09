# Subagents Specification

Each spawned subagent is also tracked as a session task (`kind = subagent`,
`links.child_session_id` pointing at the child) with lifecycle `task.*`
events and a message channel — see
[`specs/session-tasks.md`](./session-tasks.md). The generic `list_tasks`,
`get_task`, `message_task`, and `cancel_task` tools work on subagents via
the `SubagentTaskExecutor`.

<!-- Design Decisions:
  - 1 delegation tool: spawn_agent(target.type = "subagent")
  - Background execution is the default: spawn returns immediately with a task_id;
    a detached watcher settles the task and the OnTerminal wake policy notifies the parent
  - mode: "foreground" blocks the parent tool call until the child idles (original Phase 1 behavior)
  - No nesting: subagents cannot spawn subagents
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
| No nesting | Prevents runaway resource consumption and simplifies reasoning about execution depth. |
| Human-readable names | "Test Runner" is more natural than `test-runner` in conversation. |
| Inherit parent config | Subagent uses same harness, agent, and model. No capability escalation. |
| Generic lifecycle tools | `list_tasks`, `get_task`, `message_task`, `cancel_task` work for all task kinds including subagents. |

## Data Model

### Session Extensions

The `Session` entity is extended with a subagent nesting guard:

| Field | Type | Description |
|-------|------|-------------|
| `parent_session_id` | SessionId? | Parent session (null for top-level sessions) |

`subagent_name`, `subagent_task`, and `subagent_status` were retired in
migration 059. Lifecycle state is now tracked via `SessionTask` records
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
| `Sealed` | Durable engine deliberately stopped the child's turn to prevent waste (no forward progress, or budget exhausted; see `SealReason`). Terminal and non-retryable — distinct from `Failed` so the parent can decide what to do next. The seal reason is surfaced in the child's final assistant message / spawn `result`. |

Terminal subagent statuses are derived from the child's terminal **turn event** (`turn.completed` / `turn.failed` / `turn.cancelled` / `turn.sealed`), not from the bare `idle` session status — a failed or sealed turn also leaves the session `idle`, so `idle` alone never settles a subagent.

### Database Migration

See `crates/server/migrations/009_subagents.sql` for schema changes.

## Tools

### `spawn_agent` (`target.type = "subagent"`)

Sessions with `subagents` enabled advertise `subagent` in the shared
`spawn_agent` dispatcher's `target.type` enum. The dispatcher creates a child
session with `parent_session_id` set, creates a `TASK_KIND_SUBAGENT` task,
defaults to background mode, and supports `mode: "foreground"` for blocking
execution. The dispatcher can advertise other active known delegation providers
alongside `subagent` (for example first-party handoff or external A2A).

The retired direct creation entry point is no longer advertised to models.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name for the subagent |
| `instructions` | string | Yes | Instructions sent as first user message |
| `target.type` | string | Yes | Must be `subagent` |
| `mode` | string | No | `background` (default) or `foreground` |
| `blueprint` | string | No | Optional blueprint ID for a specialist child agent. |
| `config` | object | No | Blueprint-specific configuration. Only valid with `blueprint`. |
| `result_schema` | object | No | JSON Schema for the child session's final machine result. |

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

**Behavior (both modes):**
1. Creates child session with `parent_session_id` set to current session
2. Inherits the parent session locale when present
3. Creates a `TASK_KIND_SUBAGENT` task on the parent session linked to the child session

**Foreground:**
4. Sends `instructions` as first user message
5. Blocks on `wait_for_idle`
6. On child terminal turn status: returns last assistant message, task state → `succeeded`/`failed`/`canceled`
7. On child failure: returns error, task state → `failed`

**Background:**
4. Detaches a watcher (same pattern as `spawn_background` runs) and returns immediately; the watcher sends `instructions` as the first user message — deferred so local hosts, where `send_message` runs the child turn synchronously, do not block the spawn call
5. The task is created with `wake_policy: on_terminal`; the watcher heartbeats the task registry (attempt-fenced) so the session task reaper can fail an orphaned watcher after worker loss
6. The watcher waits in slices until the child reaches a terminal turn status (overall cap 6 h), then settles the task and the durable spawn handle; the registry-level wake policy delivers the completion message to the parent (specs/session-tasks.md, Wake-ups)
7. Local/embedded hosts (everruns-runtime) may report a bare `idle` after their synchronous turn — the watcher settles it as `completed`; hosted adapters never return bare `idle`
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
> See [events.md](events.md). The flow below predates the migration and is kept
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

The child session does **not** inherit:
- Message history (clean context window)
- Session-level capabilities (only agent capabilities apply)
- Active turn state

## Nesting Prevention

Subagent delegation checks `parent_session.parent_session_id`. If the current session already has a parent (is itself a subagent), the tool returns a `ToolError`:

```
"Subagents cannot spawn subagents. Only top-level sessions can create subagents."
```

This is a hard constraint enforced at the tool execution layer, not a configuration option.

**Rationale:** Unbounded nesting creates exponential resource consumption and makes debugging impractical. A single level of delegation covers the vast majority of use cases.

## Security Considerations

| Concern | Mitigation |
|---------|------------|
| Capability escalation | Subagent inherits parent capabilities exactly; no additional capabilities |
| Context isolation | Separate message history; child cannot read parent messages |
| Resource exhaustion | Foreground: 300s timeout on `wait_for_idle`. Background: 6h overall watcher cap; max iterations per child session; orphaned watchers are failed by the session task reaper on stale heartbeats |
| Runaway nesting | Hard block on subagents spawning subagents |
| Org boundary | Child session inherits org_id; standard multitenancy enforcement applies |

## UI

Dedicated **Subagents** tab in the session view with master-detail layout:

- **Master list**: All subagents with name, status badge, task preview
- **Detail view**: Full conversation history of selected subagent
- Status badges use existing session status styling conventions
- Real-time updates via subagent SSE events

The `subagents` feature string is contributed when the subagent tools are available, controlling tab visibility (see [capabilities.md](capabilities.md#capability-features)).

## Phase 1b (Future)

Background mode shipped (default; see `spawn_agent` above) — completion is
delivered through the task registry's `OnTerminal` wake policy rather than a
subagent-specific mechanism. Remaining candidates:

| Feature | Description |
|---------|-------------|
| Subagent results table | Durable tracking of subagent outcomes across sessions |
| Max iterations config | Per-subagent iteration limit (separate from session default) |
| Parallel spawn | Spawn multiple subagents in a single tool call |
| Steering messages | Completion notifications injected mid-turn into parent context |
