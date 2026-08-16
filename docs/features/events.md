---
title: Events
description: Real-time SSE streaming for full session observability. Event categories, structure, and the wire protocol.
---

Every action during a session, user input, LLM responses, tool calls, lifecycle transitions, emits an **event**. The event log is the source of truth for session state; the SSE stream is a live tail of that log.

For *why* the platform is shaped this way, see [Events as the primary store](/explanation/events/). For the full catalog of event types and payloads, see the [Event Reference](/event-reference/).

## Event categories

| Category | Examples | Description |
|---|---|---|
| **Input** | `input.message` | User messages submitted to the session |
| **Output** | `output.message.started`, `output.message.delta`, `output.message.completed` | Agent response lifecycle |
| **Turn** | `turn.started`, `turn.completed`, `turn.failed`, `turn.cancelled` | Turn lifecycle |
| **Thinking** | `reason.thinking.*` | Extended thinking content (Claude, GPT-5.x, o-series) |
| **Atom** | `reason.*`, `act.*`, `tool.*` | Internal execution phases |
| **LLM** | `llm.generation` | Full LLM API call details |
| **Session** | `session.started`, `session.activated`, `session.idled` | Session state changes |
| **Subagent** | `subagent.*` | Subagent lifecycle |

## Event structure

```json
{
  "id": "event_01933b5a00007000800000000000001",
  "type": "turn.completed",
  "ts": "2024-01-15T10:30:00.000Z",
  "session_id": "session_01933b5a00007000800000000000002",
  "sequence": 42,
  "context": {
    "turn_id": "turn_...",
    "input_message_id": "message_...",
    "trace_id": "turn_...",
    "span_id": "abc123",
    "parent_span_id": "def456"
  },
  "data": { /* type-specific payload */ }
}
```

| Field | Type | Description |
|---|---|---|
| `id` | string | Unique event ID (UUIDv7) |
| `type` | string | Event type in dot notation |
| `ts` | string | ISO 8601 with millisecond precision |
| `session_id` | string | Session this event belongs to |
| `sequence` | integer | Monotonic per-session sequence (ordering source of truth) |
| `context` | object | Correlation IDs for tracing |
| `data` | object | Event-specific payload |

Ordering is by `sequence`, not `ts`. Two events with the same wall-clock time still have a strict order.

## Common patterns

### Started → completed → failed

Long-running operations follow a lifecycle:

```
turn.started → turn.completed
            ↘ turn.failed
            ↘ turn.cancelled
```

These boundaries are what your UI uses to manage state and surface errors.

### Delta streaming

Streaming content uses delta events with accumulated state:

```json
{
  "type": "output.message.delta",
  "data": {
    "turn_id": "turn_...",
    "delta": "Hello",
    "accumulated": "Hello"
  }
}
```

Deltas are batched at ~100ms to reduce volume.

## Forward-compatibility

Events follow a defined contract. Consumers must tolerate evolution:

| Change | Allowed |
|---|---|
| New event types | yes |
| New optional fields | yes |
| New enum values | yes |
| Removing or retyping fields | no |

Your deserializer should ignore unknown fields, ignore unknown event types, and treat optional fields as optional.

## Consume the stream

- [Stream events with the SDK](/how-to/stream-events/), the convenient path (Python, Rust, TypeScript).
- [Consume events via raw SSE](/how-to/consume-events-via-sse/), protocol-level, when the SDK isn't available.

## See also

- [Event Reference](/event-reference/), complete event type catalog with payloads.
- [Events as the primary store](/explanation/events/), design rationale.
