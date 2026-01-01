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
  "context": {
    "session_id": "01937abc-def0-7000-8000-000000000002"
  },
  "data": {
    // Event-specific payload
  }
}
```

### Schema Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | UUID v7 | Yes | Unique, monotonically increasing event identifier |
| `type` | string | Yes | Event type in dot notation (e.g., `message.user`, `reason.started`) |
| `ts` | ISO 8601 | Yes | Event timestamp with millisecond precision |
| `context` | object | Yes | Correlation context for tracing |
| `data` | object | Yes | Event-specific payload (can be empty `{}`) |

### Context Object

The context provides correlation data for tracing and filtering:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `session_id` | UUID | Yes | Session this event belongs to |
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
  "context": {
    "session_id": "..."
  },
  "data": {
    "message_id": "01937abc-...",
    "content": [
      { "type": "text", "text": "Hello, world!" }
    ],
    "controls": { "max_tokens": 1000 },
    "metadata": { "source": "web" },
    "tags": ["important"]
  }
}
```

#### `message.assistant`

Assistant response message.

```json
{
  "id": "...",
  "type": "message.assistant",
  "ts": "...",
  "context": {
    "session_id": "...",
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "message_id": "01937abc-...",
    "content": [
      { "type": "text", "text": "Hello! How can I help?" }
    ],
    "model": "gpt-4o",
    "usage": {
      "input_tokens": 50,
      "output_tokens": 20
    }
  }
}
```

#### `message.tool_call`

Tool call request from the assistant.

```json
{
  "id": "...",
  "type": "message.tool_call",
  "ts": "...",
  "context": {
    "session_id": "...",
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "message_id": "01937abc-...",
    "tool_calls": [
      {
        "id": "call_123",
        "name": "get_weather",
        "arguments": { "city": "Tokyo" }
      }
    ]
  }
}
```

#### `message.tool_result`

Tool execution result.

```json
{
  "id": "...",
  "type": "message.tool_result",
  "ts": "...",
  "context": {
    "session_id": "...",
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "message_id": "01937abc-...",
    "tool_call_id": "call_123",
    "tool_name": "get_weather",
    "content": [
      { "type": "text", "text": "Temperature: 22C, Sunny" }
    ],
    "is_error": false
  }
}
```

### Atom Lifecycle Events

Atom events provide observability into the execution pipeline.

#### `input.started` / `input.completed`

InputAtom lifecycle - retrieving user message.

```json
{
  "type": "input.started",
  "context": {
    "session_id": "...",
    "turn_id": "...",
    "input_message_id": "...",
    "exec_id": "..."
  },
  "data": {}
}
```

```json
{
  "type": "input.completed",
  "context": { ... },
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
  "context": { ... },
  "data": {
    "agent_id": "...",
    "model": "gpt-4o"
  }
}
```

```json
{
  "type": "reason.completed",
  "context": { ... },
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
  "context": { ... },
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
  "context": { ... },
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
  "context": { ... },
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
  "context": { ... },
  "data": {
    "tool_call_id": "call_123",
    "tool_name": "get_weather",
    "success": true,
    "status": "success"
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
  "context": {
    "session_id": "..."
  },
  "data": {
    "agent_id": "...",
    "model_id": "..."
  }
}
```

#### `session.completed`

Session execution completed successfully.

```json
{
  "type": "session.completed",
  "context": {
    "session_id": "..."
  },
  "data": {
    "duration_ms": 1500
  }
}
```

#### `session.failed`

Session execution failed.

```json
{
  "type": "session.failed",
  "context": {
    "session_id": "..."
  },
  "data": {
    "error": "LLM provider unavailable",
    "error_code": "PROVIDER_ERROR"
  }
}
```

## Event Type Registry

| Event Type | Category | Description |
|------------|----------|-------------|
| `message.user` | Message | User input message |
| `message.assistant` | Message | Assistant response |
| `message.tool_call` | Message | Tool call request |
| `message.tool_result` | Message | Tool execution result |
| `input.started` | Atom | InputAtom started |
| `input.completed` | Atom | InputAtom completed |
| `reason.started` | Atom | ReasonAtom started |
| `reason.completed` | Atom | ReasonAtom completed |
| `act.started` | Atom | ActAtom started |
| `act.completed` | Atom | ActAtom completed |
| `tool.call_started` | Atom | Individual tool started |
| `tool.call_completed` | Atom | Individual tool completed |
| `session.started` | Session | Session execution started |
| `session.completed` | Session | Session completed |
| `session.failed` | Session | Session failed |

## Database Storage

Events are stored in the `events` table:

```sql
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);
```

The `data` column contains the full event JSON (type, context, data fields). The `event_type` column is denormalized for efficient filtering.

## SSE Streaming

Events are streamed to clients via Server-Sent Events (SSE):

```
event: message.user
data: {"id":"...","type":"message.user","ts":"...","context":{...},"data":{...}}

event: reason.started
data: {"id":"...","type":"reason.started","ts":"...","context":{...},"data":{...}}
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
WHERE event_type IN ('message.user', 'message.assistant', 'message.tool_call', 'message.tool_result');
```
