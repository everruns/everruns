# AG-UI Protocol Specification

## Abstract

Everruns exposes agents via the AG-UI protocol (https://docs.ag-ui.com). This enables compatibility with CopilotKit and other AG-UI clients.

## Requirements

### Transport

- **Protocol**: Server-Sent Events (SSE) over HTTP
- **Endpoint**: `GET /v1/runs/{run_id}/events`
- **CopilotKit Endpoint**: `POST /v1/ag-ui`
- **Content-Type**: `text/event-stream`

### Event Types

Everruns implements the standard AG-UI event types:

- `RunStarted`, `RunFinished`, `RunError` - Lifecycle events
- `TextMessageStart`, `TextMessageContent`, `TextMessageEnd` - Message streaming
- `ToolCallStart`, `ToolCallArgs`, `ToolCallEnd`, `ToolCallResult` - Tool execution

See the [AG-UI specification](https://docs.ag-ui.com) for complete event schemas.

### Event Persistence

- Events stored in `run_events` table with sequence numbers
- Clients can replay from any point using `Last-Event-ID` header
- Events are immutable once persisted
