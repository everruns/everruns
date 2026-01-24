---
title: AG-UI Protocol
description: Stream agent events using the AG-UI protocol for CopilotKit compatibility
---

# AG-UI Protocol

Everruns supports the [AG-UI protocol](https://docs.ag-ui.com) for streaming agent events to UI clients. This enables integration with [CopilotKit](https://www.copilotkit.ai) and other AG-UI-compatible frameworks.

## Overview

AG-UI (Agent User Interaction Protocol) is an open, event-based protocol that standardizes how AI agents communicate with user interfaces. Everruns provides AG-UI as a secondary API alongside its native SSE endpoint.

## Endpoint

```
GET /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/ag-ui/sse
```

## Quick Start

### 1. Create an Agent and Session

```bash
ORG="org_00000000000000000000000000000001"

# Create agent
AGENT_ID=$(curl -s -X POST "http://localhost:9000/v1/orgs/$ORG/agents" \
  -H "Content-Type: application/json" \
  -d '{"name":"My Agent","system_prompt":"You are helpful."}' | jq -r '.id')

# Create session
SESSION_ID=$(curl -s -X POST "http://localhost:9000/v1/orgs/$ORG/agents/$AGENT_ID/sessions" \
  -H "Content-Type: application/json" \
  -d '{}' | jq -r '.id')
```

### 2. Connect to AG-UI SSE

```bash
curl -N "http://localhost:9000/v1/orgs/$ORG/agents/$AGENT_ID/sessions/$SESSION_ID/ag-ui/sse"
```

### 3. Send a Message

In another terminal:

```bash
curl -X POST "http://localhost:9000/v1/orgs/$ORG/agents/$AGENT_ID/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"content":[{"type":"text","text":"Hello!"}]}'
```

You'll see AG-UI events streaming in the first terminal:

```
event: RUN_STARTED
data: {"type":"RUN_STARTED","threadId":"session_...","runId":"turn_..."}

event: TEXT_MESSAGE_START
data: {"type":"TEXT_MESSAGE_START","messageId":"msg_...","role":"assistant"}

event: TEXT_MESSAGE_CONTENT
data: {"type":"TEXT_MESSAGE_CONTENT","messageId":"msg_...","delta":"Hello"}

event: TEXT_MESSAGE_CONTENT
data: {"type":"TEXT_MESSAGE_CONTENT","messageId":"msg_...","delta":"! How"}

event: TEXT_MESSAGE_END
data: {"type":"TEXT_MESSAGE_END","messageId":"msg_..."}

event: RUN_FINISHED
data: {"type":"RUN_FINISHED","threadId":"session_...","runId":"turn_..."}
```

## Event Types

### Lifecycle Events

| Event | Description |
|-------|-------------|
| `RUN_STARTED` | Agent turn has started processing |
| `RUN_FINISHED` | Agent turn completed successfully |
| `RUN_ERROR` | Agent turn failed with error |

### Message Events

| Event | Description |
|-------|-------------|
| `TEXT_MESSAGE_START` | Assistant message started streaming |
| `TEXT_MESSAGE_CONTENT` | Incremental text content (delta) |
| `TEXT_MESSAGE_END` | Assistant message completed |

### Tool Events

| Event | Description |
|-------|-------------|
| `TOOL_CALL_START` | Tool invocation started |
| `TOOL_CALL_ARGS` | Tool arguments (sent immediately) |
| `TOOL_CALL_END` | Tool call definition complete |
| `TOOL_CALL_RESULT` | Tool execution result |

### Thinking Events

For models with extended thinking (e.g., Claude with `reasoning_effort`):

| Event | Description |
|-------|-------------|
| `THINKING_TEXT_MESSAGE_START` | Chain-of-thought started |
| `THINKING_TEXT_MESSAGE_CONTENT` | Reasoning content delta |
| `THINKING_TEXT_MESSAGE_END` | Thinking phase completed |

## JavaScript Integration

```javascript
const eventSource = new EventSource(
  '/v1/orgs/org_.../agents/agent_.../sessions/session_.../ag-ui/sse'
);

// Handle streaming text
eventSource.addEventListener('TEXT_MESSAGE_CONTENT', (event) => {
  const data = JSON.parse(event.data);
  appendToMessage(data.delta);
});

// Handle completion
eventSource.addEventListener('RUN_FINISHED', (event) => {
  console.log('Turn completed');
});

// Handle errors
eventSource.addEventListener('RUN_ERROR', (event) => {
  const data = JSON.parse(event.data);
  showError(data.message);
});
```

## CopilotKit Demo

A working demo application is available at `examples/copilotkit-demo/`:

```bash
cd examples/copilotkit-demo
npm install
npm run dev
```

This provides a chat interface with a live event visualization panel.

## Comparison with Native SSE

| Feature | Native `/sse` | AG-UI `/ag-ui/sse` |
|---------|---------------|-------------------|
| Event format | Everruns native | AG-UI protocol |
| All events | Yes | UI-focused subset |
| Internal events | Included | Excluded |
| Use case | Full observability | UI integration |

Use the native `/sse` endpoint for full event access and observability. Use `/ag-ui/sse` for CopilotKit integration and UI-focused streaming.

## Learn More

- [AG-UI Protocol Documentation](https://docs.ag-ui.com)
- [CopilotKit Documentation](https://docs.copilotkit.ai)
- [Everruns Events Specification](/specs/events.md)
