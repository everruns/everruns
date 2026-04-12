---
title: Events
description: Real-time SSE event streaming for full session observability. Monitor agent execution, tool calls, token usage, and all lifecycle events as they happen.
---

Events are the core communication protocol in Everruns. They provide real-time visibility into session execution via Server-Sent Events (SSE) streaming.

## Overview

Every action during a session - from user messages to LLM responses to tool executions - emits events. This enables:

- **Real-time UI updates**: Stream agent responses as they're generated
- **Observability**: Track every step of agent execution
- **Debugging**: Full visibility into LLM calls, tool execution, and errors
- **Integration**: Build custom UIs or monitoring tools on the event stream

## Quick Start

### SSE Streaming

Subscribe to real-time events via Server-Sent Events:

```bash
curl -N "https://api.everruns.com/v1/sessions/{session_id}/sse" \
  -H "Authorization: Bearer $API_KEY"
```

### Polling

Alternatively, poll for events with pagination:

```bash
curl "https://api.everruns.com/v1/sessions/{session_id}/events?since_id={last_event_id}" \
  -H "Authorization: Bearer $API_KEY"
```

## SSE Connection Management

### Connection Lifecycle Events

SSE streams include special lifecycle events for connection management:

| Event | Description |
|-------|-------------|
| `connected` | Sent immediately when the stream is established |
| `disconnecting` | Sent before the server gracefully closes the connection |

### Heartbeat Comments

All SSE streams send periodic heartbeat comments every 30 seconds to allow clients to detect stale connections:

```
: heartbeat
```

Heartbeat comments are standard SSE comments (lines starting with `:`) — they are **invisible to event parsers** and do not appear as events. They simply reset the TCP read timer so clients can distinguish between "connection alive but idle" and "connection dead."

**For SDK/client developers:** Set a read timeout of 45 seconds (1.5x the 30s heartbeat interval). If no data (events or heartbeats) arrives within 45s, treat the connection as stale and reconnect with `since_id`. The server sends heartbeats during all phases: idle, model thinking, tool execution, and active streaming.

### Connection Cycling

To prevent stale connections through proxies and load balancers, SSE connections are automatically cycled:

| Stream Type | Cycle Interval |
|-------------|----------------|
| Session events | 5 minutes |
| Durable monitoring | 10 minutes |

Before closing, the server sends a `disconnecting` event:

```json
{
  "event": "disconnecting",
  "data": "{\"reason\":\"connection_cycle\",\"retry_ms\":100}"
}
```

Clients should reconnect immediately using the `since_id` of the last received event. This ensures no events are missed during the transition.

### Retry Hints

Each SSE event includes a `retry:` field that hints how long clients should wait before reconnecting if the connection is unexpectedly lost:

| Situation | Retry Hint |
|-----------|------------|
| Active streaming (new events) | 100ms |
| Idle (no new events) | Increases with backoff up to 500ms |
| After `disconnecting` event | 100ms (immediate reconnect) |

The EventSource API automatically uses this hint for reconnection timing.

### Resuming Streams

Use the `since_id` query parameter to resume from the last received event:

```bash
curl -N "https://api.everruns.com/v1/sessions/{session_id}/sse?since_id={last_event_id}" \
  -H "Authorization: Bearer $API_KEY"
```

Event IDs are UUID v7 and serve as unique identifiers. Ordering is based on a dedicated sequence number, ensuring reliable event delivery with no duplicates or gaps on reconnection.

### JavaScript Example

```javascript
function connectSSE(sessionId, lastEventId) {
  const url = new URL(`/v1/sessions/${sessionId}/sse`, API_BASE);
  if (lastEventId) {
    url.searchParams.set('since_id', lastEventId);
  }

  const eventSource = new EventSource(url, { withCredentials: true });

  eventSource.addEventListener('connected', () => {
    console.log('SSE connected');
  });

  eventSource.addEventListener('disconnecting', (event) => {
    const data = JSON.parse(event.data);
    console.log('SSE disconnecting, reconnecting in', data.retry_ms, 'ms');
    eventSource.close();
    setTimeout(() => connectSSE(sessionId, lastEventId), data.retry_ms);
  });

  eventSource.addEventListener('input.message', (event) => {
    const eventData = JSON.parse(event.data);
    lastEventId = eventData.id;
    // Handle event...
  });

  // Handle other event types...

  eventSource.onerror = () => {
    eventSource.close();
    setTimeout(() => connectSSE(sessionId, lastEventId), 2000);
  };
}
```

## Event Categories

Events are organized into categories based on what they represent:

| Category | Events | Description |
|----------|--------|-------------|
| **Input** | `input.message` | User messages submitted to the session |
| **Output** | `output.message.*` | Agent response lifecycle (started, delta, completed) |
| **Turn** | `turn.*` | Turn lifecycle (started, completed, failed, cancelled) |
| **Thinking** | `reason.thinking.*` | Extended thinking content (for Claude models) |
| **Atom** | `reason.*`, `act.*`, `tool.*` | Internal execution phases |
| **LLM** | `llm.generation` | Full LLM API call details |
| **Session** | `session.*` | Session state changes |
| **Subagent** | `subagent.*` | Subagent lifecycle (spawned, completed, failed, cancelled) |

## Event Structure

Every event follows this schema:

```json
{
  "id": "event_01933b5a00007000800000000000001",
  "type": "turn.completed",
  "ts": "2024-01-15T10:30:00.000Z",
  "session_id": "session_01933b5a00007000800000000000002",
  "sequence": 42,
  "context": {
    "turn_id": "turn_01933b5a00007000800000000000003",
    "input_message_id": "message_01933b5a00007000800000000000004",
    "trace_id": "turn_01933b5a00007000800000000000003",
    "span_id": "abc123",
    "parent_span_id": "def456"
  },
  "data": { /* type-specific payload */ }
}
```

### Core Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique event ID (UUIDv7 with `event_` prefix) |
| `type` | string | Event type in dot notation |
| `ts` | string | ISO 8601 timestamp with millisecond precision |
| `session_id` | string | Session this event belongs to |
| `sequence` | integer | Monotonic sequence within session (for ordering) |
| `context` | object | Correlation context for tracing |
| `data` | object | Event-specific payload |

### Event Context

The context object provides correlation IDs for tracing:

| Field | Description |
|-------|-------------|
| `turn_id` | Turn this event belongs to |
| `input_message_id` | User message that triggered the turn |
| `exec_id` | Atom execution identifier |
| `trace_id` | OpenTelemetry-style trace ID |
| `span_id` | This event's span ID |
| `parent_span_id` | Parent span for hierarchy |

## Common Patterns

### Started-Completed-Failed Pattern

Long-running operations follow a lifecycle pattern:

```
turn.started → turn.completed
            ↘ turn.failed
            ↘ turn.cancelled
```

This provides clear boundaries for UI state management and error handling.

### Delta Pattern for Streaming

Streaming content uses delta events with accumulated state:

```json
{
  "type": "output.message.delta",
  "data": {
    "turn_id": "turn_...",
    "delta": "Hello",           // New content since last delta
    "accumulated": "Hello"       // Total content so far
  }
}
```

Delta events are batched (~100ms) to reduce volume while maintaining real-time feel.

## Forward Compatibility

Events follow semantic versioning. The contract is defined in `specs/events.md`.

### What Won't Break

- Adding new event types
- Adding optional fields to existing events
- Adding new enum values

### Consumer Guidelines

1. **Ignore unknown fields**: Your deserializer should not fail on unknown fields
2. **Handle optional fields**: Check for presence before accessing
3. **Don't rely on field ordering**: JSON field order is not guaranteed

All events in API responses are well-defined types. The server filters out any internal or unsupported events before transmission.

## Related Resources

- [Event Reference](/event-reference/) - Complete reference for all event types
- [specs/events.md](https://github.com/everruns/everruns/blob/main/specs/events.md) - Internal specification (includes contract & compatibility)
