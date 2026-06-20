# Subagents Specification

Each spawned subagent is also tracked as a session task (`kind = subagent`,
`links.child_session_id` pointing at the child) with lifecycle `task.*`
events and a message channel — see
[`specs/session-tasks.md`](./session-tasks.md). The generic `list_tasks`,
`get_task`, `message_task`, and `cancel_task` tools work on subagents via
the `SubagentTaskExecutor`.

<!-- Design Decisions:
  - 1 creation tool: spawn_subagent (get/message/cancel handled by generic session_tasks tools)
  - Foreground execution blocks parent tool call (Phase 1); background mode deferred
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
| Single creation tool | Minimal surface area. `spawn_subagent` covers creation; lifecycle is managed via generic task tools. |
| Foreground-first | Simpler mental model: agent calls tool, blocks, gets result. Background deferred to Phase 1b. |
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
```

| Status | Description |
|--------|-------------|
| `Spawning` | Session created, task not yet sent |
| `Running` | Task sent, agentic loop active |
| `Completed` | Child session idled after processing task |
| `Failed` | Child session encountered an unrecoverable error |
| `Cancelled` | Parent cancelled via `cancel_task` |
| `MaxIterationsReached` | Child hit iteration limit |

### Database Migration

See `crates/server/migrations/009_subagents.sql` for schema changes.

## Tools

### spawn_subagent

Creates a child session and sends the instructions as the first user message. In foreground mode, blocks until the child idles. Returns a `task_id` that can be used with the generic session task tools.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name for the subagent |
| `instructions` | string | Yes | Instructions sent as first user message |

**Returns:** Last assistant message from the child session plus a `task_id` for the session task record.

**Behavior:**
1. Creates child session with `parent_session_id` set to current session
2. Inherits the parent session locale when present
3. Creates a `TASK_KIND_SUBAGENT` task on the parent session linked to the child session
4. Sends `instructions` as first user message
5. Blocks on `wait_for_idle` (foreground mode)
6. On child idle: returns last assistant message, task state → `succeeded`
7. On child failure: returns error, task state → `failed`

### Monitoring and steering subagents

Use the generic `session_tasks` tools after spawning. The `task_id` is returned by `spawn_subagent`.

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
     │  spawn_subagent("Runner", instructions)  │                          │
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

`spawn_subagent` checks `parent_session.parent_session_id`. If the current session already has a parent (is itself a subagent), the tool returns a `ToolError`:

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
| Resource exhaustion | 300s timeout on `wait_for_idle`; max iterations per child session |
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

| Feature | Description |
|---------|-------------|
| Background mode | `spawn_subagent` returns immediately; completion via steering message injected into parent turn |
| Subagent results table | Durable tracking of subagent outcomes across sessions |
| Max iterations config | Per-subagent iteration limit (separate from session default) |
| Parallel spawn | Spawn multiple subagents in a single tool call |
| Steering messages | Completion notifications injected mid-turn into parent context |
