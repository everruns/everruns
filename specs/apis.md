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

### Capabilities

Capabilities are modular functionality units that can be enabled on agents. See [capabilities.md](capabilities.md) for full specification.

Capabilities are managed as part of the agent resource. When creating or updating an agent, you can specify the capabilities to enable. The agent response includes the list of enabled capabilities.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List all available capabilities |
| GET | `/v1/capabilities/{capability_id}` | Get capability details |

**Request/Response Examples:**

List capabilities:
```json
GET /v1/capabilities
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

The `ApiDoc` struct is shared between:
- `main.rs` - serves spec at `/api-doc/openapi.json` and Swagger UI
- `bin/export_openapi.rs` - exports spec to stdout for static generation

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
- `401` - Unauthorized
- `403` - Forbidden
- `404` - Not found
- `500` - Internal error
