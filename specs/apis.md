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

### Authentication

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/auth/config` | Get authentication configuration |
| POST | `/v1/auth/login` | Login with email/password |
| POST | `/v1/auth/register` | Register new user |
| POST | `/v1/auth/refresh` | Refresh access token |
| POST | `/v1/auth/logout` | Logout (clear cookies) |
| GET | `/v1/auth/oauth/{provider}` | Redirect to OAuth provider |
| GET | `/v1/auth/callback/{provider}` | OAuth callback |
| GET | `/v1/auth/me` | Get current user info |
| GET | `/v1/auth/api-keys` | List user's API keys |
| POST | `/v1/auth/api-keys` | Create API key |
| DELETE | `/v1/auth/api-keys/{key_id}` | Delete API key |

See [authentication.md](authentication.md) for full authentication specification.

### Agents

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents` | Create agent |
| GET | `/v1/agents` | List agents (paginated) |
| GET | `/v1/agents/{id}` | Get agent by ID |
| PATCH | `/v1/agents/{id}` | Update agent |
| DELETE | `/v1/agents/{id}` | Archive agent (soft delete) |
| POST | `/v1/agents/import` | Import agent from file content |
| GET | `/v1/agents/{id}/export` | Export agent as Markdown |

**Input Validation:**

All agent create/update/import endpoints enforce input size limits as last-resort protection against abuse. See [models.md](models.md#agent) for limit details. Validation failures return `400 Bad Request` with generic message "Input exceeds allowed limits".

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

### Images

Global image storage for message attachments. Images are stored with optional session metadata.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/images` | Upload image (multipart/form-data) |
| GET | `/v1/images` | List images (paginated) |
| GET | `/v1/images/{id}` | Get image metadata |
| GET | `/v1/images/{id}/data` | Get full image data |
| GET | `/v1/images/{id}/thumbnail` | Get thumbnail (200x200 max) |
| DELETE | `/v1/images/{id}` | Delete image |

**Upload Request (multipart/form-data):**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | file | Yes | Image file (PNG, JPEG, GIF, WebP) |
| `session_id` | string | No | Optional session ID for metadata |

**Upload Response:**

```json
{
  "id": "01933b5a-0000-7000-8000-000000000001",
  "filename": "screenshot.png",
  "content_type": "image/png",
  "size_bytes": 102400,
  "has_thumbnail": true,
  "created_at": "2024-01-15T10:30:00Z"
}
```

**Constraints:**
- Maximum file size: 100MB
- Allowed types: image/png, image/jpeg, image/gif, image/webp
- Thumbnails generated automatically (max 200x200 pixels)

**Usage in Messages:**

Images can be attached to messages using the `image_file` content part type:

```json
POST /v1/agents/{agent_id}/sessions/{session_id}/messages
{
  "message": {
    "content": [
      { "type": "text", "text": "What's in this image?" },
      { "type": "image_file", "image_id": "01933b5a-...", "filename": "photo.png" }
    ]
  }
}
```

Images are sent to the LLM when processing messages. The system automatically resolves `image_file` references and converts them to the provider-specific format (OpenAI Vision or Anthropic Vision).

### Session Filesystem

Virtual filesystem scoped to each session. See [session-filesystem.md](session-filesystem.md) for full specification.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/fs` | List root directory |
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}` | Read file or list directory |
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}` | Create file or directory |
| PUT | `/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}` | Update file content |
| DELETE | `/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}` | Delete file |
| DELETE | `/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}?recursive=true` | Delete directory recursively |
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/fs/_/stat` | Get file metadata |
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/fs/_/move` | Move/rename file |
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/fs/_/copy` | Copy file |
| POST | `/v1/agents/{agent_id}/sessions/{session_id}/fs/_/grep` | Search files by content |

**Note:** Paths starting with `_` are reserved for system actions and cannot be used for file creation or updates.

### Events

Server-Sent Events (SSE) for real-time UI updates and event listing.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/sse` | Stream events (SSE) |
| GET | `/v1/agents/{agent_id}/sessions/{session_id}/events` | List events (JSON) |

### LLM Provider Configuration

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/llm-providers` | List providers |
| GET | `/v1/llm-providers/{id}` | Get provider |
| PATCH | `/v1/llm-providers/{id}` | Update provider (API key, base URL) |
| GET | `/v1/llm-models` | List models |
| GET | `/v1/llm-models/{id}` | Get model |

### Agent Capabilities

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List available capabilities |

Capabilities are modular functionality units that can be enabled on agents. They provide:
- **Tool groups**: Sets of related tools (e.g., `session_file_system` provides read/write/grep tools)
- **System prompt additions**: Context injected into the agent's prompt
- **Documentation**: User-facing descriptions of what the capability provides

#### Response Format

```json
{
  "items": [
    {
      "id": "current_time",
      "name": "Current Time",
      "description": "Tool to get current date and time",
      "status": "available",
      "icon": "clock",
      "category": "Utilities"
    }
  ],
  "total": 5
}
```

Create agent with capabilities:
```json
POST /v1/agents
{
  "name": "Research Assistant",
  "system_prompt": "You are a helpful research assistant.",
  "capabilities": ["current_time", "web_fetch"]
}
```

Update agent capabilities:
```json
PATCH /v1/agents/{agent_id}
{
  "capabilities": ["current_time", "web_fetch", "session_file_system"]
}
```

Agent response includes capabilities:
```json
GET /v1/agents/{agent_id}
{
  "id": "...",
  "name": "Research Assistant",
  "system_prompt": "You are a helpful research assistant.",
  "capabilities": ["current_time", "web_fetch"],
  "status": "active",
  ...
}
```

### API Documentation

| Method | Path | Description |
|--------|------|-------------|
| GET | `/swagger-ui/` | Swagger UI for OpenAPI docs |
| GET | `/api-doc/openapi.json` | OpenAPI specification |

### OpenAPI Spec Generation

The OpenAPI spec is generated from Rust code using `utoipa` derive macros.

#### Export Binary

A standalone binary generates the spec without running the full server:

```bash
# Generate spec to stdout
cargo run --bin export-openapi

# Or use the convenience script
./scripts/export-openapi.sh
```

The binary is useful for:
- CI/CD pipelines that need the spec without running services
- Documentation builds (e.g., Astro Starlight with starlight-openapi)
- Static spec export for external tools

#### Implementation

The spec is defined in `crates/control-plane/src/openapi.rs`:

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        api::agents::create_agent,
        api::agents::list_agents,
        // ... all API endpoints
    ),
    components(schemas(...)),
    tags(...)
)]
pub struct ApiDoc;

impl ApiDoc {
    pub fn to_json() -> String {
        Self::openapi()
            .to_pretty_json()
            .expect("Failed to serialize OpenAPI spec")
    }
}
```

### Durable Execution Admin

Administrative endpoints for monitoring and managing the durable execution engine.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/durable/workers` | List registered workers |
| GET | `/v1/durable/workflows` | List workflows |
| GET | `/v1/durable/workflows/{id}` | Get workflow details |
| GET | `/v1/durable/workflows/{id}/events` | Get workflow event history |
| GET | `/v1/durable/tasks` | List task queue |
| GET | `/v1/durable/dlq` | List dead letter queue |
| GET | `/v1/durable/circuit-breakers` | List circuit breakers |

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

**Endpoints with Pagination:**

| Endpoint | Default Limit | Notes |
|----------|---------------|-------|
| `GET /v1/agents/{agent_id}/sessions` | 20 | Ordered by `created_at DESC` |

**Non-Paginated List Endpoints:**

These endpoints return all items wrapped in `{"data": [...], "total": N}`:
- `GET /v1/agents` - Returns all agents
- `GET /v1/agents/{agent_id}/sessions/{session_id}/messages` - Returns all messages
- `GET /v1/agents/{agent_id}/sessions/{session_id}/events` - Returns all events
- `GET /v1/llm-providers` - Returns all providers
- `GET /v1/llm-models` - Returns all models
- `GET /v1/durable/workers` - Returns all workers
- `GET /v1/durable/workflows` - Returns all workflows
- `GET /v1/durable/workflows/{id}/events` - Returns workflow events
- `GET /v1/durable/tasks` - Returns all tasks
- `GET /v1/durable/dlq` - Returns all DLQ entries
- `GET /v1/durable/circuit-breakers` - Returns all circuit breakers

**Exception:** The `/v1/capabilities` endpoint uses `items` instead of `data` for historical reasons.

**Example Usage:**

```bash
# First page (default)
GET /v1/agents/{id}/sessions

# Second page
GET /v1/agents/{id}/sessions?offset=20&limit=20

# Custom page size
GET /v1/agents/{id}/sessions?limit=10
```

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
