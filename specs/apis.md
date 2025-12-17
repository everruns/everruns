# API Specification

## Abstract

This document defines the HTTP API endpoints for Everruns v0.2.0 (M2).

## Requirements

### Base URL

All endpoints are prefixed with `/v1/`.

### Health

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Server health check (includes version and runner mode) |

### Agents

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents` | Create agent |
| GET | `/v1/agents` | List agents (paginated) |
| GET | `/v1/agents/{id}` | Get agent by ID |
| PATCH | `/v1/agents/{id}` | Update agent |
| DELETE | `/v1/agents/{id}` | Archive agent (soft delete) |

### Sessions

Sessions are instances of agentic loop execution tied to an agent.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents/{agent_id}/sessions` | Create session |
| GET | `/v1/agents/{agent_id}/sessions` | List sessions (paginated) |
| GET | `/v1/agents/{agent_id}/sessions/{session_id}` | Get session |
| PATCH | `/v1/agents/{agent_id}/sessions/{session_id}` | Update session |
| DELETE | `/v1/agents/{agent_id}/sessions/{session_id}` | Delete session |

### Messages

Messages store all conversation content (user, assistant, tool calls, tool results).

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/messages` | Create message (triggers workflow) |
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/messages` | List messages |

### Events

Server-Sent Events (SSE) for real-time UI updates.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/events` | Stream events (SSE) |

### LLM Providers

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/llm-providers` | Create LLM provider |
| GET | `/v1/llm-providers` | List LLM providers |
| GET | `/v1/llm-providers/{id}` | Get LLM provider |
| PATCH | `/v1/llm-providers/{id}` | Update LLM provider |
| DELETE | `/v1/llm-providers/{id}` | Delete LLM provider |

### LLM Models

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/llm-providers/{provider_id}/models` | Create model for provider |
| GET | `/v1/llm-providers/{provider_id}/models` | List provider models |
| GET | `/v1/llm-models` | List all models |
| GET | `/v1/llm-models/{id}` | Get model |
| PATCH | `/v1/llm-models/{id}` | Update model |
| DELETE | `/v1/llm-models/{id}` | Delete model |

### API Documentation

| Method | Path | Description |
|--------|------|-------------|
| GET | `/swagger-ui/` | Swagger UI for OpenAPI docs |
| GET | `/api-doc/openapi.json` | OpenAPI specification |

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
