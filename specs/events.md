# Events Specification

## Abstract

Events are the core communication protocol in Everruns. They provide observability into session execution, enable SSE streaming, and serve as the source of truth for conversation data. All events follow a standard schema and are persisted to the events table.

## Public Contract

Events are a **public API contract**. See [specs/events-contract.md](events-contract.md) for:
- Forward compatibility guarantees (what changes are breaking vs non-breaking)
- Consumer guidelines for handling events
- Server responsibilities for filtering unsupported events

**Key points:**
- Unknown event types are handled internally as `EventData::Unsupported` and filtered before API responses
- New optional fields may be added to events without breaking consumers
- New event types may be added without breaking consumers

## Standard Event Schema

Every event MUST conform to this schema:

```json
{
  "id": "01937abc-def0-7000-8000-000000000001",
  "type": "input.message",
  "ts": "2024-01-15T10:30:00.000Z",
  "session_id": "01937abc-def0-7000-8000-000000000002",
  "context": {
    "turn_id": "01937abc-def0-7000-8000-000000000003",
    "input_message_id": "01937abc-def0-7000-8000-000000000004",
    "exec_id": "01937abc-def0-7000-8000-000000000005",
    "trace_id": "turn_01937abc",
    "span_id": "exec_01937abc",
    "parent_span_id": "turn_01937abc"
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
| `type` | string | Yes | Event type in dot notation (e.g., `input.message`, `reason.started`) |
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
| `trace_id` | string | No | OTel-style trace ID (typically the turn_id string) |
| `span_id` | string | No | OTel-style span ID for this event |
| `parent_span_id` | string | No | Parent span ID for hierarchical linking |

### Hierarchical Tracing (OTel-Style)

Events within an agent turn form a hierarchical trace structure using OpenTelemetry-style span relationships. This enables observability tools (Braintrust, Jaeger, etc.) to visualize the execution as a tree.

**Trace Structure:**

```
turn (root span)
├── reason (child of turn)
│   └── llm.generation (child of reason)
├── act (child of turn)
│   ├── tool (child of act)
│   └── tool (child of act)
├── reason (child of turn)
│   └── llm.generation (child of reason)
└── ...
```

**Span ID Relationships:**

| Event Type | span_id | parent_span_id | trace_id |
|------------|---------|----------------|----------|
| `turn.started/completed` | turn_id | `null` (root) | turn_id |
| `reason.started/completed` | reason_span_id | turn_id | turn_id |
| `llm.generation` | llm_span_id | reason_span_id | turn_id |
| `act.started/completed` | act_span_id | turn_id | turn_id |
| `tool.started/completed` | tool_span_id | act_span_id | turn_id |

**Key Properties:**

1. **trace_id**: Groups all events within a single turn into one trace. Always equals the turn_id string (prefixed format, e.g., `turn_abc123`).

2. **span_id**: Uniquely identifies each event within the trace. Started/completed event pairs share the same span_id so observability tools merge them into a single span with timing.

3. **parent_span_id**: Links this span to its parent in the hierarchy. Root spans (turn events) have `null` parent. Child spans reference their immediate parent.

**ID Format Consistency:**

All IDs (`trace_id`, `span_id`, `parent_span_id`) use the prefixed string format (e.g., `turn_xyz`, `exec_abc`) rather than raw UUIDs. This ensures spans are correctly linked in observability tools.

## Event Naming Patterns

### Started-Completed-Failed Pattern

Operations that have a lifecycle follow a consistent **Started-Completed-Failed** pattern:

```
{operation}.started   → Operation began
{operation}.completed → Operation finished successfully
{operation}.failed    → Operation failed (optional, some use completed with success=false)
```

This pattern provides:
- **Observability**: Clear boundaries for timing and tracing
- **Error handling**: Explicit failure states for error tracking
- **UI feedback**: Start/end signals for loading indicators

**Examples:**

| Operation | Started | Completed | Failed |
|-----------|---------|-----------|--------|
| Turn | `turn.started` | `turn.completed` | `turn.failed` |
| Reason | `reason.started` | `reason.completed` | (uses `success=false` in completed) |
| Act | `act.started` | `act.completed` | (uses `success=false` in completed) |
| Tool Call | `tool.call_started` | `tool.call_completed` | (uses `status` field) |
| Thinking | `reason.thinking.started` | `reason.thinking.completed` | (implicit via `reason.completed`) |

**Note:** Some operations use a `success` or `status` field in the completed event rather than a separate failed event. This is a design choice based on the operation's semantics.

### Delta Pattern for Streaming

Streaming content uses a **Delta** pattern with accumulated state:

```
{operation}.delta → Incremental update with delta and accumulated fields
```

Delta events include:
- `delta`: New content since last event
- `accumulated`: Total content so far (convenience for UI)

**Examples:**

| Content Type | Delta Event |
|--------------|-------------|
| Response text | `text.delta` |
| Thinking content | `reason.thinking.delta` |

### Namespace Hierarchy

Event types use dot notation to indicate hierarchy:

```
{category}.{subcategory}.{action}
```

Examples:
- `message.user` - User message event
- `reason.started` - Reasoning phase started
- `reason.thinking.started` - Thinking within reasoning started
- `tool.call_completed` - Tool call within tool execution completed

## Event Categories

### Input Events

Input events represent user messages submitted to the session.

#### `input.message`

User message submitted to the session. Emitted when the API stores a new user message.

```json
{
  "id": "...",
  "type": "input.message",
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

### Output Events

Output events represent agent responses. They follow a lifecycle pattern: `started` → `delta*` → `completed`.

#### `output.message.started`

Emitted when the LLM starts generating a response. UI can show a "thinking" indicator until `output.message.delta` or `output.message.completed` events arrive.

```json
{
  "id": "...",
  "type": "output.message.started",
  "ts": "...",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "model": "gpt-4o"
  }
}
```

#### `output.message.delta`

Streaming text update during LLM generation. Events are batched (~100ms) to reduce volume while providing real-time feedback. UI should accumulate deltas or use the `accumulated` field until `output.message.completed` arrives with the final text.

```json
{
  "id": "...",
  "type": "output.message.delta",
  "ts": "...",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "delta": "Hello, ",
    "accumulated": "Hello, "
  }
}
```

#### `output.message.completed`

Agent response message. Emitted when LLM generation completes.

```json
{
  "id": "...",
  "type": "output.message.completed",
  "ts": "...",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "message": {
      "id": "01937abc-...",
      "role": "agent",
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

**Streaming Timeline:**

```
User sends message
       │
       ▼
┌──────────────────────────┐
│ output.message.started   │  ← UI shows thinking indicator
└───────────┬──────────────┘
            │
       ┌────┴────┐
       ▼         ▼
output.message.delta  output.message.delta  ← UI shows streaming text (batched ~100ms)
       │         │
       └────┬────┘
            │
            ▼
┌──────────────────────────┐
│ output.message.completed │  ← UI shows final message, stops streaming
└──────────────────────────┘
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

Turn execution failed. This event is emitted when:
- The LLM call fails (e.g., API key not configured, rate limit exceeded)
- Max iterations exceeded
- Other unrecoverable errors during turn execution

When a turn fails, an `output.message.completed` event with a user-friendly error message is also emitted so users see feedback in the chat.

```json
{
  "type": "turn.failed",
  "session_id": "...",
  "context": {
    "turn_id": "..."
  },
  "data": {
    "turn_id": "...",
    "error": "An error occurred while processing your request.",
    "error_code": "llm_error"
  }
}
```

**Error Codes:**
| Code | Description |
|------|-------------|
| `llm_error` | LLM call failed (API key missing, rate limit, network error) |
| `max_iterations` | Maximum iterations exceeded |

#### `turn.cancelled`

Turn execution was cancelled by user request. This event is emitted immediately when the cancel endpoint is called.

```json
{
  "type": "turn.cancelled",
  "session_id": "...",
  "context": {
    "turn_id": "..."
  },
  "data": {
    "turn_id": "...",
    "reason": "User requested cancellation",
    "usage": {
      "input_tokens": 150,
      "output_tokens": 30
    }
  }
}
```

**Fields:**
| Field | Type | Description |
|-------|------|-------------|
| `turn_id` | UUID | The cancelled turn identifier |
| `reason` | string | Optional reason for cancellation |
| `usage` | TokenUsage | Optional token usage up to cancellation point |

**Cancellation Flow:**
1. User clicks cancel in UI
2. API emits `turn.cancelled` event immediately
3. API emits `input.message` with "User requested to cancel the work."
4. Worker detects cancellation and stops execution
5. Worker emits `output.message.completed` with "Work was cancelled by user."
6. Worker emits `session.idled` event

### Atom Lifecycle Events

Atom events provide observability into the execution pipeline.

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

For failed reasoning (LLM call error):

```json
{
  "type": "reason.completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "success": false,
    "text_preview": null,
    "has_tool_calls": false,
    "tool_call_count": 0,
    "error": "LLM error: API key is required"
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
      { "id": "call_123", "name": "get_weather", "display_name": "Get Weather" }
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

#### `tool.started` / `tool.completed`

Individual tool execution within ActAtom.

```json
{
  "type": "tool.started",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call": {
      "id": "call_123",
      "name": "get_weather",
      "arguments": { "city": "Tokyo" }
    },
    "display_name": "Get Weather"
  }
}
```

```json
{
  "type": "tool.completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call_id": "call_123",
    "tool_name": "get_weather",
    "display_name": "Get Weather",
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
  "type": "tool.completed",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "tool_call_id": "call_456",
    "tool_name": "search_db",
    "display_name": "Search Database",
    "success": false,
    "status": "error",
    "error": "Connection timeout"
  }
}
```

### LLM Events

LLM events provide visibility into the actual LLM API calls.

#### `llm.generation`

Emitted after each LLM API call to provide full visibility into the messages sent to the model and the response received. This is useful for debugging, auditing, and understanding the exact prompts and responses.

**Metadata fields** (aligned with gen-ai OTel semantic conventions):
- `model` - Model name used for generation
- `provider` - LLM provider (openai, anthropic, etc.)
- `usage` - Token usage (input_tokens, output_tokens)
- `duration_ms` - Request duration in milliseconds
- `time_to_first_token_ms` - Time to first token (streaming latency)
- `success` - Whether the generation succeeded
- `error` - Error message if failed
- `finish_reasons` - Array of finish reasons (e.g., `["stop"]`, `["tool_calls"]`)
- `response_id` - Provider's response ID for correlation

```json
{
  "type": "llm.generation",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "messages": [
      {
        "id": "...",
        "role": "system",
        "content": [{ "type": "text", "text": "You are a helpful assistant." }],
        "created_at": "..."
      },
      {
        "id": "...",
        "role": "user",
        "content": [{ "type": "text", "text": "What's the weather in Tokyo?" }],
        "created_at": "..."
      }
    ],
    "output": {
      "text": "I'll check the weather for you.",
      "tool_calls": [
        {
          "id": "call_123",
          "name": "get_weather",
          "arguments": { "city": "Tokyo" }
        }
      ]
    },
    "metadata": {
      "model": "gpt-4o",
      "provider": "openai",
      "usage": {
        "input_tokens": 150,
        "output_tokens": 45
      },
      "duration_ms": 1200,
      "time_to_first_token_ms": 180,
      "success": true,
      "finish_reasons": ["stop"],
      "response_id": "chatcmpl-abc123"
    }
  }
}
```

For failed generations:

```json
{
  "type": "llm.generation",
  "session_id": "...",
  "context": { "turn_id": "...", "exec_id": "..." },
  "data": {
    "messages": [...],
    "output": {
      "text": null,
      "tool_calls": []
    },
    "metadata": {
      "model": "gpt-4o",
      "provider": "openai",
      "duration_ms": 500,
      "success": false,
      "error": "Rate limit exceeded"
    }
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

#### `session.activated`

Session became active (turn started). Emitted when a new turn begins.

```json
{
  "type": "session.activated",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "input_message_id": "..."
  }
}
```

#### `session.idled`

Session became idle (turn completed). Contains cumulative session usage for real-time UI updates.

```json
{
  "type": "session.idled",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "iterations": 3,
    "usage": {
      "input_tokens": 500,
      "output_tokens": 150,
      "cache_read_tokens": 100
    }
  }
}
```

**Usage Field:** Contains cumulative session token usage at this point.

### Extended Thinking Events

Extended thinking events provide visibility into the model's chain-of-thought reasoning when using models with extended thinking capabilities (e.g., Anthropic Claude with `reasoning_effort` configured).

#### `reason.thinking.started`

Emitted when the LLM starts generating a response with extended thinking enabled (`reasoning_effort` is set). This event is only emitted when using models that support extended thinking. UI can show a "thinking" indicator until `output.message.delta` or `output.message.completed` events arrive.

**Note:** This event is NOT emitted when `reasoning_effort` is not configured, even if the model supports extended thinking.

```json
{
  "type": "reason.thinking.started",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "model": "claude-opus-4-5"
  }
}
```

#### `reason.thinking.delta`

Streaming thinking/reasoning content from extended thinking models. These events contain the model's chain-of-thought reasoning before producing the final response. Events are batched (~100ms) similar to `output.message.delta`.

```json
{
  "type": "reason.thinking.delta",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "delta": "Let me analyze this step by step...",
    "accumulated": "Let me analyze this step by step..."
  }
}
```

#### `reason.thinking.completed`

Emitted when the model finishes its chain-of-thought reasoning and transitions to producing the final response. Contains the complete thinking content.

```json
{
  "type": "reason.thinking.completed",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "turn_id": "...",
    "thinking": "Let me analyze this step by step...\n\n1. First consideration...\n2. Second consideration..."
  }
}
```

**Note:** Extended thinking events (`reason.thinking.*`) are only emitted when using models that support extended thinking mode. The thinking content is separate from the main response text and typically shown in a collapsible section in the UI. Thinking content is also persisted in the `output.message.completed` event's `thinking` field for multi-turn context.

**Extended Thinking Timeline Example:**

```
User sends message
       │
       ▼
┌──────────────────────────┐
│ reason.thinking.started  │  ← UI shows thinking indicator
└────────┬─────────────────┘
         │
         ▼
  ┌────────────────────────┐
  │ reason.thinking.delta  │  ← UI shows streaming reasoning
  └────────┬───────────────┘
           │
           ▼
  ┌──────────────────────────┐
  │ reason.thinking.completed│  ← Thinking phase done
  └────────┬─────────────────┘
           │
      ┌────┴────┐
      ▼         ▼
output.message.delta  output.message.delta  ← UI shows streaming text
      │         │
      └────┬────┘
           │
           ▼
  ┌──────────────────────────┐
  │ output.message.completed │  ← UI shows final message
  └──────────────────────────┘
```

**Real-time Usage Tracking Pattern:**

The UI uses a combination of events for real-time usage display:

1. `session.idled` - Sets the baseline (cumulative from backend)
2. `llm.generation` - Adds tokens during turn execution (real-time increments)
3. `session.idled` - Resets to final cumulative value when turn completes

```
Timeline during a turn:
──────────────────────────────────────────────────────────────────
│ session.idled │ llm.generation │ llm.generation │ session.idled │
│   (baseline)  │    (+tokens)   │    (+tokens)   │  (final set)  │
──────────────────────────────────────────────────────────────────
      500 in    →     650 in     →     800 in     →     800 in
      100 out         130 out          175 out          175 out
```

This approach provides real-time feedback as tokens are consumed during LLM calls, while self-correcting to the accurate cumulative value when each turn ends.

## Event Type Registry

| Event Type | Category | Description |
|------------|----------|-------------|
| `input.message` | Input | User input message |
| `output.message.started` | Output | LLM generation started (thinking indicator) |
| `output.message.delta` | Output | Incremental text update during streaming |
| `output.message.completed` | Output | Agent response |
| `turn.started` | Turn | Turn execution started |
| `turn.completed` | Turn | Turn completed |
| `turn.failed` | Turn | Turn failed |
| `turn.cancelled` | Turn | Turn cancelled by user |
| `reason.started` | Atom | ReasonAtom started |
| `reason.completed` | Atom | ReasonAtom completed |
| `act.started` | Atom | ActAtom started |
| `act.completed` | Atom | ActAtom completed |
| `tool.started` | Atom | Individual tool started |
| `tool.completed` | Atom | Individual tool completed (includes result) |
| `llm.generation` | LLM | Full LLM API call with messages and response |
| `reason.thinking.started` | Thinking | Extended thinking started (thinking indicator) |
| `reason.thinking.delta` | Thinking | Incremental reasoning content from extended thinking models |
| `reason.thinking.completed` | Thinking | Extended thinking completed |
| `session.started` | Session | Session execution started |
| `session.activated` | Session | Session became active (turn started) |
| `session.idled` | Session | Session became idle (turn completed, includes usage) |

## Database Storage

Events are stored in the `events` table:

```sql
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    context JSONB NOT NULL DEFAULT '{}',
    data JSONB NOT NULL DEFAULT '{}',
    metadata JSONB,
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);
```

The `data` column contains the event-specific payload. The `event_type` column is denormalized for efficient filtering. The `context` column holds correlation data (turn_id, input_message_id, exec_id). The `metadata` and `tags` columns provide additional filtering and categorization capabilities.

## Storage Guarantees

The event store provides three key guarantees:

### 1. Append-Only Immutability

Events are **immutable** once written. The database enforces this via triggers:

```sql
CREATE TRIGGER events_append_only_update
    BEFORE UPDATE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();

CREATE TRIGGER events_append_only_delete
    BEFORE DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION prevent_event_mutation();
```

**Behavior:**
- `UPDATE` on `events` → Error: "events are append-only: UPDATE operations are not allowed"
- `DELETE` on `events` → Error: "events are append-only: DELETE operations are not allowed"
- `INSERT` → Allowed (append-only)

**Rationale:** Event sourcing requires immutable history. Allowing mutations would break replay, audit trails, and data integrity guarantees.

### 2. Atomic Per-Session Sequence Allocation

Each event within a session is assigned a monotonically increasing sequence number. Sequences are allocated atomically to prevent race conditions under concurrent writes.

**Implementation:**

A dedicated `event_sequences` table tracks the next sequence per session:

```sql
CREATE TABLE event_sequences (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    next_sequence INTEGER NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

The `allocate_event_sequence(session_id)` function atomically allocates the next sequence:

```sql
INSERT INTO event_sequences (session_id, next_sequence, updated_at)
VALUES (p_session_id, 2, NOW())
ON CONFLICT (session_id) DO UPDATE
SET next_sequence = event_sequences.next_sequence + 1,
    updated_at = NOW()
RETURNING next_sequence - 1;
```

**Guarantees:**
- No sequence gaps within a session (barring transaction rollbacks)
- No duplicate sequences within a session
- Race-free under concurrent inserts (uses PostgreSQL's atomic upsert)
- Sequences start at 1 for each session

**Previous Approach (Deprecated):**
The old `MAX(sequence)+1` approach had race conditions when multiple writers inserted events concurrently for the same session.

### 3. Event Type Consistency Validation

The `event_type` field must match the type indicated by the event's `data` payload. This is validated at the service layer before storage.

**Validation Rule:**
```
request.event_type == request.data.event_type()
```

**Exemption:** Raw/legacy events (where `data.event_type() == "unknown"`) are exempt from this check to support backward compatibility.

**Error on Mismatch:**
```
"event type mismatch: declared 'input.message' but data indicates 'output.message.completed'"
```

**Rationale:** Prevents drift between the declared event type and the actual payload, which would cause incorrect filtering, routing, and processing.

## Message Reconstruction

Messages are reconstructed from events for the conversation view. The following event types contribute to message reconstruction:

| Event Type | Role | Content Source |
|------------|------|----------------|
| `input.message` | `user` | `data.message.content` |
| `output.message.completed` | `agent` | `data.message.content` (may include tool calls) |
| `tool.completed` | `tool` | `data.result` (tool execution results) |

**Note:** Tool calls are embedded in `output.message.completed` events via `ContentPart::ToolCall`. Tool results come from `tool.completed` events, not a separate message type.

## SSE Streaming

Events are streamed to clients via Server-Sent Events (SSE):

```
event: input.message
data: {"id":"...","type":"input.message","ts":"...","session_id":"...","context":{},"data":{...}}

event: reason.started
data: {"id":"...","type":"reason.started","ts":"...","session_id":"...","context":{...},"data":{...}}
```

The SSE `event` field matches the `type` field in the event payload.

### SSE Connection Lifecycle

SSE streams include special lifecycle events for connection management:

| Event | Description |
|-------|-------------|
| `connected` | Sent immediately when the stream is established. Data: `{"status":"connected"}` |
| `disconnecting` | Sent before graceful close. Data: `{"reason":"connection_cycle","retry_ms":100}` |

### Connection Cycling

To prevent stale connections through proxies and load balancers, SSE connections are automatically cycled:

| Stream Type | Cycle Interval | Backoff Range |
|-------------|----------------|---------------|
| Session events (realtime) | 5 minutes | 100ms → 500ms |
| Durable monitoring | 10 minutes | 1000ms → 20000ms |

Before closing, the server sends a `disconnecting` event so clients can reconnect immediately using `since_id`. This ensures no events are missed.

### Retry Hints

Each SSE event includes a `retry:` field (in milliseconds) that hints reconnection timing:

| Situation | Session Events | Durable Monitoring |
|-----------|---------------|-------------------|
| Active (new events) | 100ms | 1000ms |
| Idle (backoff max) | 500ms | 20000ms |
| After `disconnecting` | 100ms | 1000ms |

The EventSource API automatically uses this hint.

### SDK Implementation Requirements

SDKs MUST:
1. Track the last received event ID (`lastEventId`)
2. Handle `disconnecting` event by reconnecting with `since_id` parameter
3. Handle `onerror` with exponential backoff (EventSource default behavior)
4. Use `retry:` hint for reconnection timing

Example client implementation:

```javascript
eventSource.addEventListener('disconnecting', (event) => {
  const data = JSON.parse(event.data);
  eventSource.close();
  setTimeout(() => reconnect(lastEventId), data.retry_ms);
});
```

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
WHERE event_type IN ('input.message', 'output.message.completed');
```

### Turn Events Filter

```sql
CREATE INDEX idx_events_turns ON events(session_id, sequence)
WHERE event_type IN ('turn.started', 'turn.completed', 'turn.failed');
```

### Tool Events Filter

```sql
CREATE INDEX idx_events_tool_calls ON events(session_id, sequence)
WHERE event_type IN ('tool.started', 'tool.completed');
```

## Event Listeners

Event listeners provide a pluggable mechanism for observability backends to react to events without modifying business logic.

### EventListener Trait

```rust
#[async_trait]
pub trait EventListener: Send + Sync {
    /// Called after an event is persisted
    async fn on_event(&self, event: &Event);

    /// Optional: filter which event types to receive
    fn event_types(&self) -> Option<Vec<&'static str>> { None }

    /// Human-readable name for logging
    fn name(&self) -> &'static str { "EventListener" }
}
```

### Listener Registration

Listeners are registered with `EventService` at startup:

```rust
let otel_listener = Arc::new(OtelEventListener::new());
let event_service = EventService::with_listeners(db, vec![otel_listener]);
```

### Built-in Listeners

| Listener | Event Types | Purpose |
|----------|-------------|---------|
| `OtelEventListener` | `llm.generation`, `tool.*`, `turn.*` | Generate OpenTelemetry spans |

### Execution Model

1. Event is persisted to database
2. All registered listeners are notified sequentially
3. Listeners should not block; spawn background tasks for heavy processing
4. Listener failures do not affect event persistence

### Error Isolation

Listeners are executed in isolation to ensure misbehaving listeners cannot disrupt event processing:

- **Panic isolation**: Each listener runs in a separate tokio task. If a listener panics, the panic is caught and logged, but other listeners continue to execute.
- **No error propagation**: Listener errors/panics never propagate to the event emitter or affect event persistence.
- **Logging**: Panics are logged with `tracing::error!` including the listener name for debugging.
- **Sequential execution**: Despite isolation, listeners are awaited sequentially to preserve ordering semantics.

**Rationale:** Event listeners are pluggable integrations (OTel, metrics, audit logs) that should not affect core event processing. A bug in an observability integration should never break the application.

**Implementation:**
```rust
// Each listener is spawned in isolation
let handle = tokio::spawn(async move {
    listener.on_event(&event).await;
});

// Panics are caught and logged, not propagated
if let Err(e) = handle.await {
    tracing::error!(listener = name, error = %e, "EventListener panicked");
}
```

### Custom Listeners

Custom listeners can be implemented for:
- Metrics collection (Prometheus, StatsD)
- Analytics pipelines (event forwarding to data warehouses)
- Audit logging
- Real-time alerting
