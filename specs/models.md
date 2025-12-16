# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable AI agent execution platform.

## Requirements

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Display name |
| `description` | string? | Optional description |
| `system_prompt` | string | System prompt for the LLM |
| `default_model_id` | UUID? | Reference to llm_models table |
| `tags` | string[] | Tags for organization/filtering |
| `status` | enum | `active` or `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### Session

An instance of agentic loop execution. Multiple sessions can exist concurrently for a single agent.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `agent_id` | UUID v7 | Parent agent reference |
| `title` | string? | Session title (user-provided or auto-generated) |
| `tags` | string[] | Tags for organization/filtering |
| `model_id` | UUID? | Override model (null = use agent default) |
| `status` | enum | `pending`, `running`, `completed`, `failed` |
| `created_at` | timestamp | Creation time |
| `started_at` | timestamp? | Execution start time |
| `finished_at` | timestamp? | Completion time |

Status transitions: `pending` → `running` → `completed` | `failed`

### Message

The primary conversation data. Stores all conversation content including user messages, assistant responses, tool calls, and tool results.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session (auto-increment per session) |
| `role` | enum | `user`, `assistant`, `tool_call`, `tool_result`, `system` |
| `content` | JSON | Message content (schema depends on role) |
| `tool_call_id` | string? | For tool_result, references the tool_call id |
| `created_at` | timestamp | Creation time |

**Content schemas by role:**

```json
// role=user, assistant, or system
{
  "text": "Hello, how are you?"
}

// role=tool_call (assistant requesting tool execution)
{
  "id": "call_abc123",
  "name": "search",
  "arguments": { "query": "test" }
}

// role=tool_result (result of tool execution)
{
  "result": { "matches": [...] },
  "error": null
}
```

### Event

SSE notification stream for real-time UI updates. **Not the primary data store** - use Messages for that.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session |
| `event_type` | string | Type of notification (see below) |
| `data` | JSON | Event-specific payload |
| `created_at` | timestamp | Event time |

**Event Types:**

1. **Step Events** - Workflow progress notifications
   - `step.started` - Started processing (e.g., LLM call began)
   - `step.generating` - Generation in progress (streaming delta)
   - `step.generated` - Generation complete
   - `step.error` - Step failed

2. **Message Events** - Notifications about messages
   - `message.created` - A new message was created
   - `message.delta` - Streaming content update

3. **Tool Events** - Tool execution notifications
   - `tool.started` - Tool execution began
   - `tool.completed` - Tool execution finished

4. **Session Events** - Session lifecycle
   - `session.started` - Session began processing
   - `session.completed` - Session finished successfully
   - `session.failed` - Session encountered error

## Flow Example

```
User sends: "How much is 2+2?"

1. POST /v1/agents/{id}/sessions/{id}/messages
   → Creates Message(role=user, content: { text: "How much is 2+2?" })
   → Triggers session workflow

2. Workflow starts
   → Updates Session(status=running)
   → Emits Event(session.started)

3. LLM call starts
   → Emits Event(step.started)

4. LLM streaming response
   → Emits Event(step.generating, data: { delta: "The answer" })
   → Emits Event(step.generating, data: { delta: " is 4" })

5. LLM complete
   → Creates Message(role=assistant, content: { text: "The answer is 4" })
   → Emits Event(step.generated)
   → Emits Event(message.created, data: { message_id: "..." })

6. Session complete
   → Updates Session(status=completed, finished_at=now())
   → Emits Event(session.completed)
```

## Design Decisions

| Question | Decision |
|----------|----------|
| What stores conversation? | **Messages** table - primary data |
| What are Events for? | SSE notifications for real-time UI updates |
| Where are tool calls stored? | Messages with role=tool_call |
| Where are tool results stored? | Messages with role=tool_result |
| Session status? | Explicit status field (pending, running, completed, failed) |
