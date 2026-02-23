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

For the complete list of event types and their data schemas, see [event type constants](../crates/core/src/events.rs) and `EventData` enum variants.

### Event Category Overview

| Category | Events | Purpose |
|----------|--------|---------|
| Input | `input.message` | User messages |
| Output | `output.message.{started,delta,completed}` | Agent response lifecycle |
| Turn | `turn.{started,completed,failed,cancelled}` | Turn lifecycle |
| Atom | `reason.*`, `act.*` | Execution pipeline observability |
| Tool | `tool.{started,completed}` | Individual tool execution |
| LLM | `llm.generation` | Full LLM API call visibility |
| Session | `session.{started,activated,idled}` | Session lifecycle |
| Thinking | `reason.thinking.{started,delta,completed}` | Extended thinking (Anthropic) |

### Representative Example

All events follow the standard schema. Here's an `output.message.completed` event as a representative example:

```json
{
  "id": "01937abc-def0-7000-8000-000000000001",
  "type": "output.message.completed",
  "ts": "2024-01-15T10:30:01.000Z",
  "session_id": "...",
  "context": { "turn_id": "...", "input_message_id": "..." },
  "data": {
    "message": {
      "id": "01937abc-...",
      "role": "agent",
      "content": [{ "type": "text", "text": "Hello! How can I help?" }],
      "created_at": "2024-01-15T10:30:01.000Z"
    },
    "metadata": { "model": "gpt-4o", "model_id": "...", "provider_id": "..." },
    "usage": { "input_tokens": 50, "output_tokens": 20 }
  }
}
```

### Output Streaming Timeline

```
User sends message
       │
       ▼
output.message.started    ← UI shows thinking indicator
       │
output.message.delta*     ← UI shows streaming text (batched ~100ms)
       │
output.message.completed  ← UI shows final message, stops streaming
```

### Turn Failure

`turn.failed` is emitted when LLM call fails, max iterations exceeded, or other unrecoverable errors. An `output.message.completed` with a user-friendly error is also emitted.

**Error Codes:** `llm_error`, `max_iterations`

### Turn Cancellation Flow

1. User clicks cancel in UI
2. API emits `turn.cancelled` immediately
3. API emits `input.message` with "User requested to cancel the work."
4. Worker detects cancellation and stops execution
5. Worker emits `output.message.completed` with "Work was cancelled by user."
6. Worker emits `session.idled`

### LLM Generation Events

`llm.generation` metadata fields align with gen-ai OTel semantic conventions: `model`, `provider`, `usage`, `duration_ms`, `time_to_first_token_ms`, `success`, `error`, `finish_reasons`, `response_id`.

### Extended Thinking Events

Extended thinking events (`reason.thinking.*`) are only emitted when using models with extended thinking capabilities (e.g., Anthropic Claude with `reasoning_effort` configured). The thinking content is separate from the main response text.

```
reason.thinking.started    ← UI shows thinking indicator
reason.thinking.delta*     ← UI shows streaming reasoning
reason.thinking.completed  ← Thinking done, transitions to response
output.message.delta*      ← UI shows streaming response text
output.message.completed   ← Final message
```

**Real-time Usage Tracking Pattern:**

The UI uses a combination of events for real-time usage display:

1. `session.idled` - Sets the baseline (cumulative from backend)
2. `llm.generation` - Adds tokens during turn execution (real-time increments)
3. `session.idled` - Resets to final cumulative value when turn completes

This approach provides real-time feedback as tokens are consumed during LLM calls, while self-correcting to the accurate cumulative value when each turn ends.

## Database Storage

See [migrations](../crates/server/migrations/) for the events table schema. Key columns: `data` (event-specific payload as JSONB), `event_type` (denormalized for filtering), `context` (correlation data: turn_id, input_message_id, exec_id).

## Storage Guarantees

The event store provides three key guarantees:

### 1. Append-Only Immutability

Events are **immutable** once written. The database enforces this via triggers that block UPDATE and DELETE. Only INSERT is allowed.

**Rationale:** Event sourcing requires immutable history. Allowing mutations would break replay, audit trails, and data integrity guarantees.

### 2. Atomic Per-Session Sequence Allocation

Each event within a session is assigned a monotonically increasing sequence number. Sequences are allocated atomically via the `allocate_event_sequence()` function using PostgreSQL's atomic upsert on a dedicated `event_sequences` table. See [migrations](../crates/server/migrations/) for the implementation.

**Guarantees:**
- No sequence gaps within a session (barring transaction rollbacks)
- No duplicate sequences within a session
- Race-free under concurrent inserts
- Sequences start at 1 for each session

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

Messages are reconstructed from events: `input.message` → user, `output.message.completed` → agent, `tool.completed` → tool results. Tool calls are embedded in `output.message.completed` via `ContentPart::ToolCall`.

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

### Query Parameter Filters

Both the SSE (`/v1/sessions/{id}/sse`) and JSON (`/v1/sessions/{id}/events`) endpoints accept:

| Parameter | Type | Description |
|-----------|------|-------------|
| `since_id` | EventId | Resume after this event ID (UUID v7 monotonic) |
| `types` | string[] | **Positive filter**: only return events matching these types. Empty = all types. |
| `exclude` | string[] | **Negative filter**: remove matching types from the result. |

**Semantics when both are provided:** `types` narrows first, then `exclude` removes from that set.

**Examples:**
- Only turn lifecycle: `?types=turn.started&types=turn.completed`
- Everything except deltas: `?exclude=output.message.delta&exclude=reason.thinking.delta`
- Turn events but not failures: `?types=turn.started&types=turn.completed&types=turn.failed&exclude=turn.failed`

**Validation:**
- Both parameters accept only known event types (see Event Type Registry). Unknown types return 400.
- Maximum 25 values per parameter to prevent abuse.

Partial indexes exist for efficient filtering by message events, turn events, and tool events. See [migrations](../crates/server/migrations/) for index definitions.

## Event Listeners

Event listeners provide a pluggable mechanism for observability backends to react to events without modifying business logic.

### EventListener Trait

See [EventListener trait](../crates/core/src/event_listeners.rs) for the trait definition. Listeners are registered with `EventService` at startup.

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

Each listener is spawned in a separate tokio task. Panics are caught and logged, not propagated.

### Custom Listeners

Custom listeners can be implemented for:
- Metrics collection (Prometheus, StatsD)
- Analytics pipelines (event forwarding to data warehouses)
- Audit logging
- Real-time alerting
