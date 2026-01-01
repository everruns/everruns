# Events Specification

## Abstract

Events are the core communication protocol in Everruns. They provide observability into session execution, enable SSE streaming, and serve as the source of truth for conversation data. All events follow a standard schema and are persisted to the events table.

## Standard Event Schema

Every event MUST conform to this schema:

```json
{
  "id": "01937abc-def0-7000-8000-000000000001",
  "type": "message.user",
  "ts": "2024-01-15T10:30:00.000Z",
  "session_id": "01937abc-def0-7000-8000-000000000002",
  "context": {
    "turn_id": "01937abc-def0-7000-8000-000000000003",
    "input_message_id": "01937abc-def0-7000-8000-000000000004",
    "exec_id": "01937abc-def0-7000-8000-000000000005"
  },
  "data": {
    // Event-specific payload
  },
  "metadata": { /* Optional arbitrary metadata */ },
  "tags": ["tag1", "tag2"]
}
```

### Schema Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | UUID v7 | Yes | Unique, monotonically increasing event identifier |
| `type` | string | Yes | Event type in dot notation (e.g., `message.user`, `reason.started`) |
| `ts` | ISO 8601 | Yes | Event timestamp with millisecond precision |
| `session_id` | UUID | Yes | Session this event belongs to |
| `context` | object | Yes | Correlation context for tracing |
| `data` | object | Yes | Event-specific payload (can be empty `{}`) |
| `metadata` | object | No | Arbitrary metadata for the event |
| `tags` | array | No | Tags for filtering and categorization |

### Context Object

The context provides correlation data for tracing and filtering:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `turn_id` | UUID | No | Turn identifier (for turn-scoped events) |
| `input_message_id` | UUID | No | User message that triggered this turn |
| `exec_id` | UUID | No | Atom execution identifier |

## Event Categories

### Message Events

Message events represent conversation data and are the source of truth for messages.

#### `message.user`

User message submitted to the session.

```json
{
  "id": "...",
  "type": "message.user",
  "ts": "...",
  "session_id": "...",
  "context": {},
  "data": {
    "message": {
      "id": "01937abc-...",
      "role": "user",
      "content": [
        { "type": "text", "text": "Hello, world!" }
      ],
      "controls": { "max_tokens": 1000 },
      "metadata": { "source": "web" },
      "created_at": "2024-01-15T10:30:00.000Z"
    }
  }
}
```

#### `message.agent`

Agent response message.

```json
{
  "id": "...",
  "type": "message.agent",
  "ts": "...",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "message": {
      "id": "01937abc-...",
      "role": "assistant",
      "content": [
        { "type": "text", "text": "Hello! How can I help?" }
      ],
      "created_at": "2024-01-15T10:30:01.000Z"
    },
    "metadata": {
      "model": "gpt-4o",
      "model_id": "01937abc-...",
      "provider_id": "01937abc-..."
    },
    "usage": {
      "input_tokens": 50,
      "output_tokens": 20
    }
  }
}
```

### Turn Lifecycle Events

Turn events track the lifecycle of a single turn in the conversation.

#### `turn.started`

Turn execution started.

```json
{
  "type": "turn.started",
  "session_id": "...",
  "context": {
    "turn_id": "..."
  },
  "data": {
    "turn_id": "...",
    "input_message_id": "..."
  }
}
```

#### `turn.completed`

Turn execution completed successfully.

```json
{
  "type": "turn.completed",
  "session_id": "...",
  "context": {
    "turn_id": "..."
  },
  "data": {
    "turn_id": "...",
    "iterations": 3,
    "duration_ms": 1500
  }
}
```

#### `turn.failed`

Turn execution failed.

```json
{
  "type": "turn.failed",
  "session_id": "...",
  "context": {
    "turn_id": "..."
  },
  "data": {
    "turn_id": "...",
    "error": "Max iterations exceeded",
    "error_code": "MAX_ITERATIONS"
  }
}
```

### Atom Lifecycle Events

Atom events provide observability into the execution pipeline.

#### `input.received`

User input received and retrieved from message store.

```json
{
  "type": "input.received",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "...",
    "exec_id": "..."
  },
  "data": {
    "message": { /* Message object */ }
  }
}
```

#### `reason.started` / `reason.completed`

ReasonAtom lifecycle - LLM inference.

```json
{
  "type": "reason.started",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "agent_id": "...",
    "metadata": {
      "model": "gpt-4o",
      "model_id": "...",
      "provider_id": "..."
    }
  }
}
```

```json
{
  "type": "reason.completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "success": true,
    "text_preview": "First 200 chars...",
    "has_tool_calls": true,
    "tool_call_count": 2
  }
}
```

#### `act.started` / `act.completed`

ActAtom lifecycle - tool batch execution.

```json
{
  "type": "act.started",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_calls": [
      { "id": "call_123", "name": "get_weather" }
    ]
  }
}
```

```json
{
  "type": "act.completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "completed": true,
    "success_count": 2,
    "error_count": 0
  }
}
```

#### `tool.call_started` / `tool.call_completed`

Individual tool execution within ActAtom.

```json
{
  "type": "tool.call_started",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call": {
      "id": "call_123",
      "name": "get_weather",
      "arguments": { "city": "Tokyo" }
    }
  }
}
```

```json
{
  "type": "tool.call_completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call_id": "call_123",
    "tool_name": "get_weather",
    "success": true,
    "status": "success",
    "result": [
      { "type": "text", "text": "Temperature: 22C, Sunny" }
    ]
  }
}
```

For failed tool calls:

```json
{
  "type": "tool.call_completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call_id": "call_456",
    "tool_name": "search_db",
    "success": false,
    "status": "error",
    "error": "Connection timeout"
  }
}
```

### Session Events

Session lifecycle events.

#### `session.started`

Session execution started.

```json
{
  "type": "session.started",
  "session_id": "...",
  "context": {},
  "data": {
    "agent_id": "...",
    "model_id": "..."
  }
}
```

## Event Type Registry

| Event Type | Category | Description |
|------------|----------|-------------|
| `message.user` | Message | User input message |
| `message.agent` | Message | Agent response |
| `turn.started` | Turn | Turn execution started |
| `turn.completed` | Turn | Turn completed |
| `turn.failed` | Turn | Turn failed |
| `input.received` | Atom | User input received |
| `reason.started` | Atom | ReasonAtom started |
| `reason.completed` | Atom | ReasonAtom completed |
| `act.started` | Atom | ActAtom started |
| `act.completed` | Atom | ActAtom completed |
| `tool.call_started` | Atom | Individual tool started |
| `tool.call_completed` | Atom | Individual tool completed (includes result) |
| `session.started` | Session | Session execution started |

## Database Storage

Events are stored in the `events` table:

```sql
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    metadata JSONB,
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);
```

The `data` column contains the full event JSON (type, context, data fields). The `event_type` column is denormalized for efficient filtering. The `metadata` and `tags` columns provide additional filtering and categorization capabilities.

## SSE Streaming

Events are streamed to clients via Server-Sent Events (SSE):

```
event: message.user
data: {"id":"...","type":"message.user","ts":"...","session_id":"...","context":{},"data":{...}}

event: reason.started
data: {"id":"...","type":"reason.started","ts":"...","session_id":"...","context":{...},"data":{...}}
```

The SSE `event` field matches the `type` field in the event payload.

## Filtering

Events can be filtered by:

- `session_id`: Required for all queries
- `event_type`: Filter by event type prefix (e.g., `message.*`, `reason.*`)
- `sequence`: For pagination and replay (after sequence N)
- `turn_id`: Filter events for a specific turn

### Message Events Filter

A partial index exists for efficient message queries:

```sql
CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('message.user', 'message.agent');
```

### Turn Events Filter

```sql
CREATE INDEX idx_events_turns ON events(session_id, sequence)
WHERE event_type IN ('turn.started', 'turn.completed', 'turn.failed');
```

### Tool Events Filter

```sql
CREATE INDEX idx_events_tool_calls ON events(session_id, sequence)
WHERE event_type IN ('tool.call_started', 'tool.call_completed');
```
