# API Specification

## Abstract

This document defines the HTTP API endpoints for Everruns v0.2.0 (M2).

## Requirements

### Endpoint Reference

All endpoints are prefixed with `/v1/`. For the complete endpoint listing with request/response schemas, see the [OpenAPI spec](../scripts/export-openapi.sh) or the live spec at `/api-doc/openapi.json`.

See [authentication.md](authentication.md) for full authentication specification.

### Agents

#### Agent Preview

The preview endpoint computes the final agent shape without persisting anything. Useful for UI to show users what their agent will look like at runtime.

**Request:**
```json
POST /v1/agents/preview
{
  "system_prompt": "You are a helpful assistant.",
  "capabilities": [
    { "ref": "current_time" },
    { "ref": "mcp:01234567-89ab-cdef-0123-456789abcdef" }
  ]
}
```

**Response:**
```json
{
  "system_prompt": "## Current Time\nYou have access to...\n\nYou are a helpful assistant.",
  "tools": [
    {
      "name": "get_current_time",
      "description": "Get the current date and time",
      "parameters": { ... }
    }
  ]
}
```

The response shows:
- `system_prompt`: Final prompt with capability additions prepended
- `tools`: All tool definitions from enabled capabilities (including MCP servers)

**Input Validation:**

All agent create/update/import endpoints enforce input size limits as last-resort protection against abuse. See [models.md](models.md#agent) for limit details. Validation failures return `400 Bad Request` with generic message "Input exceeds allowed limits".

### Sessions

#### Create Session

**Request:**
```json
POST /v1/sessions
{
  "agent_id": "agent_01234567-...",
  "title": "Optional title",
  "tags": ["optional", "tags"],
  "model_id": "optional-model-override",
  "capabilities": [
    { "ref": "current_time" },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } }
  ]
}
```

The `agent_id` field is required and specifies which agent will work in this session.

**Session Capabilities:**

The optional `capabilities` field allows setting session-level capabilities that are **additive** to the agent's capabilities. When building the RuntimeAgent:
1. Agent capabilities are applied first
2. Session capabilities are applied after (additive)

This enables temporarily extending an agent's capabilities for specific sessions without modifying the agent configuration.

#### List Sessions

Supports optional filtering by agent:

```
GET /v1/sessions?agent_id=agent_01234567-...
```

Without the `agent_id` query parameter, returns all sessions in the organization.

#### Cancel Turn

Cancels the currently executing turn in a session. This stops the workflow execution and emits appropriate events.

**Request:** No body required.

**Response:** `200 OK` on success, `400 Bad Request` if session is not active.

**Events emitted:**
1. `turn.cancelled` - Immediately when cancel is requested
2. `input.message` - "User requested to cancel the work."
3. `output.message.completed` - "Work was cancelled by user." (emitted by worker when it stops)
4. `session.idled` - When the session transitions to idle (emitted by worker)

The user message is emitted immediately by the API, while the agent message and session.idled events are emitted by the worker after it detects the cancellation and stops execution. This ensures the agent message appears after any in-flight events.

### Events

See [events.md](events.md) for full event specification.

**SSE/Events query parameters** (`/v1/sessions/{id}/sse` and `/v1/sessions/{id}/events`):

| Parameter | Type | Description |
|-----------|------|-------------|
| `since_id` | EventId | Resume after this event ID |
| `types` | string[] | Positive filter: only return matching event types. Empty = all. Repeated key format: `?types=a&types=b` |
| `exclude` | string[] | Negative filter: remove matching event types. Applied after `types`. Repeated key format: `?exclude=a&exclude=b` |

When both `types` and `exclude` are provided, `types` narrows first, then `exclude` removes from that set. Both accept only known event types (max 25 per parameter).

### OpenAPI Spec Generation

The OpenAPI spec is generated from Rust code using `utoipa` derive macros. See [openapi.rs](../crates/server/src/openapi.rs) for the spec definition.

```bash
# Generate spec to stdout
cargo run --bin export-openapi

# Or use the convenience script
./scripts/export-openapi.sh
```

### Response Formats

All endpoints return JSON. Event streaming uses Server-Sent Events (SSE) with `text/event-stream` content type.

### Pagination

Endpoints that return lists support pagination via query parameters:

| Parameter | Type | Default | Max | Description |
|-----------|------|---------|-----|-------------|
| `offset` | integer | 0 | - | Number of items to skip |
| `limit` | integer | 20 | 100 | Maximum items to return |

**Paginated Response Format:**

```json
{
  "data": [...],
  "total": 150,
  "offset": 0,
  "limit": 20
}
```

| Field | Type | Description |
|-------|------|-------------|
| `data` | array | Items for the current page |
| `total` | integer | Total count across all pages |
| `offset` | integer | Current offset (echoed from request) |
| `limit` | integer | Current limit (echoed from request) |

Currently only `GET /v1/sessions` uses offset/limit pagination. Most list endpoints return all items wrapped in `{"data": [...], "total": N}`.

**Exception:** The `/v1/capabilities` endpoint uses `items` instead of `data` for historical reasons.

### Error Responses

```json
{
  "error": "Error message",
  "status": 400
}
```

Standard HTTP status codes:
- `400` - Bad Request (invalid input)
- `401` - Unauthorized (missing/invalid auth)
- `403` - Forbidden (insufficient permissions)
- `404` - Not Found
- `422` - Unprocessable Entity (validation error)
- `500` - Internal Server Error

### Error Handling Guidelines

- **Never expose internal error details.** Return generic `500 Internal Server Error` with "Internal server error".
- **Always log server-side:** `tracing::error!()` before returning generic response.
- Only return safe, user-facing messages: "Not found", "Invalid request", "Internal server error".

Example:
```rust
let result = state.db.some_operation().await.map_err(|e| {
    tracing::error!("Failed to perform operation: {}", e);
    StatusCode::INTERNAL_SERVER_ERROR
})?;
```
