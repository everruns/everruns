# AG-UI Protocol Specification

## Abstract

AG-UI (Agent User Interaction Protocol) is an open, lightweight, event-based protocol for streaming agent UI events. Everruns provides AG-UI compatibility as a **secondary API** alongside the primary SSE endpoint, enabling integration with CopilotKit and other AG-UI-compatible clients.

## Protocol Overview

AG-UI defines a standardized event format for bidirectional communication between agentic backends and user-facing frontends. See https://docs.ag-ui.com for the full specification.

## Endpoint

```
GET /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/ag-ui/sse
```

### Query Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `since_id` | UUID | Optional. Only return events after this event ID |

### Response

Server-Sent Events (SSE) stream with AG-UI formatted events.

```
event: RUN_STARTED
data: {"type":"RUN_STARTED","threadId":"session_...","runId":"turn_...","timestamp":1234567890.123}

event: TEXT_MESSAGE_START
data: {"type":"TEXT_MESSAGE_START","messageId":"msg_...","role":"assistant","timestamp":1234567890.123}

event: TEXT_MESSAGE_CONTENT
data: {"type":"TEXT_MESSAGE_CONTENT","messageId":"msg_...","delta":"Hello","timestamp":1234567890.123}

event: TEXT_MESSAGE_END
data: {"type":"TEXT_MESSAGE_END","messageId":"msg_...","timestamp":1234567890.123}

event: RUN_FINISHED
data: {"type":"RUN_FINISHED","threadId":"session_...","runId":"turn_...","timestamp":1234567890.123}
```

## Event Types

### Lifecycle Events

| Event | Description | Fields |
|-------|-------------|--------|
| `RUN_STARTED` | Turn/run started | `threadId`, `runId`, `timestamp` |
| `RUN_FINISHED` | Turn/run completed | `threadId`, `runId`, `timestamp` |
| `RUN_ERROR` | Turn/run failed | `message`, `code`, `timestamp` |

### Text Message Events

| Event | Description | Fields |
|-------|-------------|--------|
| `TEXT_MESSAGE_START` | Message started | `messageId`, `role`, `timestamp` |
| `TEXT_MESSAGE_CONTENT` | Content delta | `messageId`, `delta`, `timestamp` |
| `TEXT_MESSAGE_END` | Message completed | `messageId`, `timestamp` |

### Tool Call Events

| Event | Description | Fields |
|-------|-------------|--------|
| `TOOL_CALL_START` | Tool call initiated | `toolCallId`, `toolCallName`, `parentMessageId`, `timestamp` |
| `TOOL_CALL_ARGS` | Tool arguments | `toolCallId`, `delta`, `timestamp` |
| `TOOL_CALL_END` | Tool call definition complete | `toolCallId`, `timestamp` |
| `TOOL_CALL_RESULT` | Tool result received | `toolCallId`, `result`, `timestamp` |

### Thinking Events (Extended Reasoning)

| Event | Description | Fields |
|-------|-------------|--------|
| `THINKING_TEXT_MESSAGE_START` | Thinking started | `messageId`, `timestamp` |
| `THINKING_TEXT_MESSAGE_CONTENT` | Thinking content | `messageId`, `delta`, `timestamp` |
| `THINKING_TEXT_MESSAGE_END` | Thinking completed | `messageId`, `timestamp` |

## ID Mapping

| AG-UI ID | Everruns Source | Description |
|----------|-----------------|-------------|
| `threadId` | `session_id` | Persists across turns |
| `runId` | `turn_id` | Unique per turn |
| `messageId` | Generated | `msg_{turn_id}` for outputs |
| `toolCallId` | `tool_call.id` | From tool call |

## Event Mapping from Everruns

| Everruns Event | AG-UI Event(s) |
|----------------|----------------|
| `turn.started` | `RUN_STARTED` |
| `turn.completed` | `RUN_FINISHED` |
| `turn.failed` | `RUN_ERROR` |
| `turn.cancelled` | `RUN_ERROR` (code: "cancelled") |
| `output.message.started` | `TEXT_MESSAGE_START` |
| `output.message.delta` | `TEXT_MESSAGE_CONTENT` |
| `output.message.completed` | `TEXT_MESSAGE_END` |
| `tool.started` | `TOOL_CALL_START` + `TOOL_CALL_ARGS` + `TOOL_CALL_END` |
| `tool.completed` | `TOOL_CALL_RESULT` |
| `reason.thinking.started` | `THINKING_TEXT_MESSAGE_START` |
| `reason.thinking.delta` | `THINKING_TEXT_MESSAGE_CONTENT` |
| `reason.thinking.completed` | `THINKING_TEXT_MESSAGE_END` |

### Events NOT Mapped

Internal/observability events are not emitted to AG-UI:
- `reason.started`, `reason.completed`
- `act.started`, `act.completed`
- `llm.generation`
- `session.started`, `session.activated`, `session.idled`
- `input.message`

## Serialization

- Event types use `SCREAMING_SNAKE_CASE`
- Field names use `camelCase`
- Timestamps are Unix seconds with millisecond precision (float)
- All events include `type` field

## Primary vs Secondary API

| Aspect | Primary (`/sse`) | Secondary (`/ag-ui/sse`) |
|--------|------------------|--------------------------|
| Format | Everruns native | AG-UI protocol |
| All events | Yes | Subset (UI-focused) |
| Internal events | Included | Excluded |
| Compatibility | Everruns clients | CopilotKit ecosystem |

## Example Usage

### JavaScript EventSource

```javascript
const eventSource = new EventSource(
  '/v1/orgs/org_.../agents/agent_.../sessions/session_.../ag-ui/sse'
);

eventSource.addEventListener('TEXT_MESSAGE_CONTENT', (event) => {
  const data = JSON.parse(event.data);
  console.log('Content:', data.delta);
});

eventSource.addEventListener('RUN_ERROR', (event) => {
  const data = JSON.parse(event.data);
  console.error('Error:', data.message);
});
```

### curl

```bash
curl -N "http://localhost:9000/v1/orgs/org_.../agents/agent_.../sessions/session_.../ag-ui/sse"
```

## CopilotKit Integration

The AG-UI endpoint is compatible with CopilotKit's event format. See `examples/copilotkit-demo/` for a working integration example.

## References

- AG-UI Protocol: https://docs.ag-ui.com
- CopilotKit: https://www.copilotkit.ai
- Everruns Events Spec: [specs/events.md](events.md)
