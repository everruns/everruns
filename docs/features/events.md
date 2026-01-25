---
title: Events
description: Real-time event streaming for session observability
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
curl -N "https://api.everruns.com/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/sse" \
  -H "Authorization: Bearer $API_KEY"
```

### Polling

Alternatively, poll for events with pagination:

```bash
curl "https://api.everruns.com/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/events?since_id={last_event_id}" \
  -H "Authorization: Bearer $API_KEY"
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

Events follow semantic versioning. The contract is defined in `specs/events-contract.md`.

### What Won't Break

- Adding new event types
- Adding optional fields to existing events
- Adding new enum values

### Consumer Guidelines

1. **Ignore unknown fields**: Your deserializer should not fail on unknown fields
2. **Handle optional fields**: Check for presence before accessing
3. **Don't rely on field ordering**: JSON field order is not guaranteed

All events in API responses are well-defined types. The server filters out any internal or unsupported events before transmission.

## OpenAPI Schema

Full event schemas are documented in the OpenAPI specification:

- [Event Schema](/api/schemas/Event)
- [EventData Variants](/api/schemas/EventData)
- [EventContext](/api/schemas/EventContext)

## Related Resources

- [Event Reference](/features/event-reference) - Complete reference for all event types
- [specs/events.md](https://github.com/everruns/everruns/blob/main/specs/events.md) - Internal specification
- [specs/events-contract.md](https://github.com/everruns/everruns/blob/main/specs/events-contract.md) - Contract specification
