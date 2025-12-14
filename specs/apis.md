# API Specification

## Abstract

This document defines the HTTP API endpoints for Everruns.

## Requirements

### Base URL

All endpoints are prefixed with `/v1/`.

### Health

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Server health check |

### Agents

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents` | Create agent |
| GET | `/v1/agents` | List agents (paginated) |
| GET | `/v1/agents/{id}` | Get agent by ID |
| PATCH | `/v1/agents/{id}` | Update agent |
| DELETE | `/v1/agents/{id}` | Delete agent (soft delete) |

### Threads

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/threads` | Create thread |
| GET | `/v1/threads/{id}` | Get thread with messages |

### Messages

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/threads/{id}/messages` | Add message to thread |

### Runs

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/runs` | Create and start run |
| GET | `/v1/runs/{id}` | Get run status |
| POST | `/v1/runs/{id}/cancel` | Cancel running run |
| GET | `/v1/runs/{id}/events` | Stream events (SSE) |

### AG-UI Endpoint

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/ag-ui` | AG-UI protocol endpoint for CopilotKit |

Query parameters: `agent_id`, `thread_id` (optional)

### Response Formats

All endpoints return JSON. Event streaming uses Server-Sent Events (SSE) with `text/event-stream` content type.

### Error Responses

```json
{
  "error": "Error message",
  "status": 400
}
```

Standard HTTP status codes:
- `200` - Success
- `201` - Created
- `204` - No content
- `400` - Bad request
- `404` - Not found
- `500` - Internal error
