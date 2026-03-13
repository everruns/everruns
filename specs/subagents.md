# Subagents Specification

<!-- Design Decisions:
  - 3 tools only: spawn_subagent, get_subagents, message_subagent
  - Foreground execution blocks parent tool call (Phase 1); background mode deferred
  - No nesting: subagents cannot spawn subagents
  - Human-readable names by default ("Test Runner" not "test-runner")
  - Case-insensitive name matching for get/message operations
  - Completion notifications use steering messages (injected mid-turn)
  - message_subagent unifies cancel + resume + mid-execution steering
  - Subagent inherits parent's harness and agent configuration
  - UI: dedicated Subagents tab with master-detail layout
-->

## Abstract

Subagents allow a host agent to delegate tasks to child sessions that run in their own context window. Each subagent is a full session with isolated message history, enabling parallel workstreams and separation of concerns within a single parent conversation.

Inspired by Claude Code's Agent tool, Cursor's sub-agents, and OpenAI Codex's multi-agent patterns.

## Design Principles

| Principle | Rationale |
|-----------|-----------|
| Exactly 3 tools | Minimal surface area. spawn/get/message covers full lifecycle. |
| Foreground-first | Simpler mental model: agent calls tool, blocks, gets result. Background deferred to Phase 1b. |
| No nesting | Prevents runaway resource consumption and simplifies reasoning about execution depth. |
| Human-readable names | "Test Runner" is more natural than `test-runner` in conversation. Case-insensitive matching for ergonomics. |
| Inherit parent config | Subagent uses same harness, agent, and model. No capability escalation. |
| Steering via messages | `message_subagent` unifies cancel, resume, and mid-execution guidance into one tool. |

## Data Model

### Session Extensions

The `Session` entity is extended with subagent-specific fields:

| Field | Type | Description |
|-------|------|-------------|
| `parent_session_id` | SessionId? | Parent session (null for top-level sessions) |
| `subagent_name` | String? | Human-readable name ("Test Runner") |
| `subagent_task` | String? | Original task description |
| `subagent_status` | SubagentStatus? | Current lifecycle status |

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
| `Cancelled` | Parent cancelled via `message_subagent(cancel: true)` |
| `MaxIterationsReached` | Child hit iteration limit |

### Database Migration

See `crates/server/migrations/008_subagents.sql` for schema changes.

## Tools

### spawn_subagent

Creates a child session and sends the task as the first user message. In foreground mode, blocks until the child idles.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name for the subagent |
| `task` | string | Yes | Task description sent as first user message |

**Returns:** Last assistant message from the child session (the subagent's response to the task).

**Behavior:**
1. Creates child session with `parent_session_id` set to current session
2. Sets `subagent_name`, `subagent_task`, `subagent_status = Spawning`
3. Sends `task` as first user message → status transitions to `Running`
4. Blocks on `wait_for_idle` (foreground mode)
5. On child idle: returns last assistant message, status → `Completed`
6. On child failure: returns error, status → `Failed`

### get_subagents

Lists or retrieves detail for subagents spawned by the current session.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name_or_id` | string | No | Filter by name (case-insensitive) or session ID |
| `status_filter` | string | No | Filter by SubagentStatus value |

**Returns:** Array of subagent summaries (name, status, task, created_at), or single subagent detail when `name_or_id` matches exactly one.

### message_subagent

Sends a follow-up message to an existing subagent. Unifies steering, resuming, and cancellation.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name_or_id` | string | Yes | Subagent name (case-insensitive) or session ID |
| `message` | string | Yes* | Message content to send (* not required when `cancel: true`) |
| `cancel` | bool | No | If true, cancel the subagent instead of messaging |

**Returns:** Last assistant message after the subagent processes the message, or cancellation confirmation.

**Cancel behavior:** Sets `subagent_status = Cancelled`, cancels any active turn in the child session.

## Events

Four SSE event types follow the existing event naming patterns (see [events.md](events.md)):

| Event Type | Category | Description |
|------------|----------|-------------|
| `subagent.spawned` | Subagent | Child session created and task sent |
| `subagent.completed` | Subagent | Child session finished task successfully |
| `subagent.failed` | Subagent | Child session failed |
| `subagent.cancelled` | Subagent | Child session cancelled by parent |

All subagent events are emitted on the **parent** session's event stream. Event data includes `subagent_name`, `subagent_session_id`, and status-specific fields (e.g., `error` for failed).

## Execution Flow

```
Parent Agent                          System                           Child Session
     │                                  │                                  │
     │  spawn_subagent("Runner", task)  │                                  │
     │─────────────────────────────────>│                                  │
     │                                  │  create session(parent_id=...)   │
     │                                  │─────────────────────────────────>│
     │                                  │  send task as user message       │
     │                                  │─────────────────────────────────>│
     │                                  │  emit subagent.spawned           │
     │                                  │                                  │
     │          (blocked)               │         agentic loop             │
     │          wait_for_idle           │<────────────────────────────────>│
     │                                  │                                  │
     │                                  │  child idles                     │
     │                                  │<─────────────────────────────────│
     │                                  │  emit subagent.completed         │
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
