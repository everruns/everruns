---
title: API Overview
description: Overview of the Everruns REST API
---

The Everruns API provides a RESTful interface for managing agents, sessions, and messages.

## Base URL

All API endpoints are versioned under the `/v1/` prefix:

```
https://your-domain.com/v1/
```

## Authentication

API authentication is configured per deployment. See your deployment's authentication documentation for details.

## Interactive Documentation

Full interactive API documentation is available via Swagger UI:

- **Swagger UI**: `https://your-domain.com/swagger-ui/`
- **OpenAPI Spec**: `https://your-domain.com/api-doc/openapi.json`

## Core Endpoints

### Agents

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/agents` | List all agents |
| POST | `/v1/agents` | Create a new agent |
| GET | `/v1/agents/{id}` | Get agent details |
| PATCH | `/v1/agents/{id}` | Update an agent |
| DELETE | `/v1/agents/{id}` | Delete an agent |

### Sessions

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/agents/{id}/sessions` | List sessions for an agent |
| POST | `/v1/agents/{id}/sessions` | Create a new session |
| GET | `/v1/sessions/{id}` | Get session details |
| DELETE | `/v1/sessions/{id}` | Delete a session |

### Messages

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/sessions/{id}/messages` | List messages in a session |
| POST | `/v1/sessions/{id}/messages` | Send a message |
| GET | `/v1/sessions/{id}/events` | Stream session events (SSE) |

### Capabilities

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/capabilities` | List available capabilities |

### LLM Providers

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v1/llm-providers` | List configured providers |
| POST | `/v1/llm-providers` | Add a provider |
| DELETE | `/v1/llm-providers/{id}` | Remove a provider |

## Event Streaming

Session events are streamed via Server-Sent Events (SSE). Connect to the events endpoint to receive real-time updates:

```bash
curl -N https://your-domain.com/v1/sessions/{id}/events
```

Event types include:
- `message.created` - New message added
- `message.delta` - Streaming content update
- `tool.call` - Tool invocation
- `tool.result` - Tool execution result
- `session.completed` - Session finished processing

## Error Responses

All errors follow a consistent format:

```json
{
  "error": {
    "message": "Description of the error",
    "code": "ERROR_CODE"
  }
}
```

Common HTTP status codes:
- `400` - Bad Request (invalid input)
- `404` - Not Found
- `500` - Internal Server Error
