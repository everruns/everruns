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

### MCP Endpoint

| Method | Path | Description |
|--------|------|-------------|
| POST | `/mcp` | MCP server (Streamable HTTP transport, JSON-RPC 2.0) |

Exposes Everruns as an MCP server. Tier 1 tools (`agent_run`, `session_send_message`, `session_get_status`) handle the agent conversation loop via direct service calls. Tier 2 tools (`discover`, `execute`) are backed by a bashkit `ScriptedTool` with all API operations registered as builtins — run `discover --categories` to list them, or call them directly in bash scripts via `execute`. See `crates/server/src/api/mcp_endpoint/mod.rs` for implementation.

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

### Organizations

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/orgs` | List organizations for current user |
| POST | `/v1/orgs` | Create organization |
| GET | `/v1/orgs/{org}` | Get organization details |
| PATCH | `/v1/orgs/{org}` | Update organization name and harness defaults |

Organization details include two org-scoped harness settings:
- `default_harness_id` - the harness the UI should preselect for normal new-session flows
- `base_harness_id` - the harness used when `POST /v1/sessions` omits `harness_id`

New organizations initialize these to the built-in `Generic` and `Base` harnesses respectively. Initialization fills missing values but does not overwrite user-selected values.

### Agents

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/agents` | Create agent (optional client-supplied `id`) |
| GET | `/v1/agents` | List active agents by default (`include_archived=true` to include archived) |
| GET | `/v1/agents/{id}` | Get agent by ID |
| PUT | `/v1/agents/{id}` | Upsert agent (create 201, update 200) |
| PATCH | `/v1/agents/{id}` | Update agent |
| DELETE | `/v1/agents/{id}` | Archive agent (soft delete) |
| POST | `/v1/agents/{id}/delete` | Dangerous delete of archived agent |
| POST | `/v1/agents/import` | Import agent from file content |
| GET | `/v1/agents/{id}/export` | Export agent as Markdown |
| POST | `/v1/agents/preview` | Preview final agent shape |
| POST | `/v1/agents/{id}/copy` | Copy agent (new ID, "{name} (copy)") |

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

### Harnesses

Harnesses define the base environment and capabilities for sessions. See [harness-types.md](harness-types.md) for built-in types.

Harness create/update payloads include optional `parent_harness_id`. When present, preview and runtime resolve the effective harness from parent to child before applying any agent or session layers.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/harnesses` | Create harness |
| GET | `/v1/harnesses` | List active harnesses by default (`include_archived=true` to include archived) |
| GET | `/v1/harnesses/{id}` | Get harness |
| PATCH | `/v1/harnesses/{id}` | Update harness |
| DELETE | `/v1/harnesses/{id}` | Archive harness |
| POST | `/v1/harnesses/{id}/delete` | Dangerous delete of archived harness |
| POST | `/v1/harnesses/{id}/copy` | Copy harness (new ID, "{name} (copy)") |
| POST | `/v1/harnesses/preview` | Preview merged system prompt + tools |

### Building Block Lifecycle API Rules

Applies to agents, harnesses, skills, MCP servers, and apps unless an entity-specific spec overrides it.

- `DELETE /v1/{resource}/{id}` archives the entity. Archive is the default destructive-looking action in UI, but it is not permanent.
- Dangerous delete is a separate explicit endpoint (`POST /v1/{resource}/{id}/delete`) and requires the dangerous permission for that resource.
- Dangerous delete only succeeds from `archived`.
- Detail APIs return archived entities, but return `404` for deleted entities.
- List APIs exclude archived and deleted items by default. `include_archived=true` includes archived items, but deleted items stay hidden.
- Archived or deleted entities cannot be assigned in create/update APIs.
- Archived or deleted entities cannot be edited.
- Historical reference surfaces may still expose the foreign-key ID for a deleted entity so callers can render tombstones such as `<Deleted Harness>`.

### Sessions

Sessions are top-level entities under organizations. Each session has an agent assigned to work in it.

See `specs/localization.md` for locale/timezone precedence and execution-context rules.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/sessions` | Create session |
| POST | `/v1/sessions/chat` | Get or create global chat session |
| GET | `/v1/sessions` | List sessions (paginated) |
| GET | `/v1/sessions/{session_id}` | Get session |
| PATCH | `/v1/sessions/{session_id}` | Update session |
| DELETE | `/v1/sessions/{session_id}` | Delete session |
| POST | `/v1/sessions/{session_id}/cancel` | Cancel current turn |

#### Create Session

**Request:**
```json
POST /v1/sessions
{
  "harness_id": "harness_01234567-...",
  "agent_id": "agent_01234567-...",
  "title": "Optional title",
  "locale": "uk-UA",
  "tags": ["optional", "tags"],
  "model_id": "optional-model-override",
  "capabilities": [
    { "ref": "current_time" },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } }
  ],
  "hints": {
    "setup_connection": true,
    "rich_media": true
  }
}
```

The `harness_id` field is optional. When omitted, the server uses the organization's `base_harness_id`. New organizations default that setting to the built-in `Base` harness.

The `agent_id` field is optional and specifies which agent will work in this session when present.

The optional `locale` field sets the session's default locale for agent responses and regional formatting. It is persisted on the session and reused across worker turns.
The session API should also support an optional `timezone` field with an IANA timezone value. This is the durable fallback for unattended execution and scheduled/background turns.

**Session Capabilities:**

The optional `capabilities` field allows setting session-level capabilities that are **additive** to the agent's capabilities. When building the RuntimeAgent:
1. Agent capabilities are applied first
2. Session capabilities are applied after (additive)

This enables temporarily extending an agent's capabilities for specific sessions without modifying the agent configuration.

**Client Hints:**

The optional `hints` field sets session-level **client hints** — generic key-value pairs that tell the server what the client can handle. Unlike capabilities (agent-scoped feature flags for tools), hints are client-scoped signals that influence server behavior without coupling to a fixed schema.

- Any key-value pair is valid; unknown keys are ignored by the server.
- Session hints are defaults for every turn in this session.
- Per-message `controls.hints` override session hints key-by-key (shallow merge).
- Resolution: `effective_hints = session.hints ∪ message.controls.hints` (message wins).

Examples of hint keys:

| Key | Value | Meaning |
| --- | ----- | ------- |
| `setup_connection` | `true` | Client can handle inline connection setup flow |
| `rich_media` | `true` | Client can render images, audio, etc. |
| `file_upload` | `true` | Client supports file upload tool calls |
| `max_image_width` | `1024` | Client display constraint |

Session creation and any other assignment flow must reject archived or deleted harnesses/agents with a client error. Existing sessions are preserved when dependencies are archived or deleted, but the next execution atom must fail gracefully with a user-visible explanation.

#### Get or Create Chat Session

Returns the calling user's singleton global chat session. Creates one with the Platform Chat harness if none exists. Uses tag-based lookup (`global-chat` + `user:{user_id}`) for per-user singleton management.

**Request:** `POST /v1/sessions/chat` (no body required)

**Response:** `200 OK` with the `Session` object.

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

### Messages

Messages store all conversation content (user, agent, tool calls, tool results).

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/sessions/{session_id}/messages` | Create message (triggers workflow) |
| GET | `/v1/sessions/{session_id}/messages` | List messages |

`CreateMessageRequest.metadata.locale` and `CreateMessageRequest.metadata.timezone` are reserved for per-turn execution-context overrides. They take precedence over session and user defaults for the triggered turn only.

`CreateMessageRequest.controls.hints` allows per-message client hint overrides. These are shallow-merged with session-level hints (message wins per key). See the Client Hints section under Create Session.

### Images

Org-scoped image storage for message attachments. Images are stored with optional session metadata.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/images` | Upload image (multipart/form-data) |
| GET | `/v1/images` | List images (paginated) |
| GET | `/v1/images/{id}` | Get image data |
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
- Request body limit: 101MB (100MB file + 1MB multipart overhead)
- Allowed types: image/png, image/jpeg, image/gif, image/webp
- Thumbnails generated automatically (max 200x200 pixels)

**Usage in Messages:**

Images can be attached to messages using the `image_file` content part type:

```json
POST /v1/sessions/{session_id}/messages
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
| GET | `/v1/sessions/{session_id}/fs` | List root directory |
| GET | `/v1/sessions/{session_id}/fs/{path}` | Read file or list directory |
| POST | `/v1/sessions/{session_id}/fs/{path}` | Create file or directory |
| PUT | `/v1/sessions/{session_id}/fs/{path}` | Update file content |
| DELETE | `/v1/sessions/{session_id}/fs/{path}` | Delete file |
| DELETE | `/v1/sessions/{session_id}/fs/{path}?recursive=true` | Delete directory recursively |
| POST | `/v1/sessions/{session_id}/fs/_/stat` | Get file metadata |
| POST | `/v1/sessions/{session_id}/fs/_/move` | Move/rename file |
| POST | `/v1/sessions/{session_id}/fs/_/copy` | Copy file |
| POST | `/v1/sessions/{session_id}/fs/_/grep` | Search files by content |

**Note:** Paths starting with `_` are reserved for system actions and cannot be used for file creation or updates.

### Events

Server-Sent Events (SSE) for real-time UI updates and event listing.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/sessions/{session_id}/sse` | Stream events (SSE) |
| GET | `/v1/sessions/{session_id}/events` | List events (JSON) |

**Query Parameters** (both endpoints):

| Parameter | Type | Description |
|-----------|------|-------------|
| `since_id` | EventId | Resume after this event ID |
| `types` | string[] | Positive filter: only return matching event types. Empty = all. Repeated key format: `?types=a&types=b` |
| `exclude` | string[] | Negative filter: remove matching event types. Applied after `types`. Repeated key format: `?exclude=a&exclude=b` |

When both `types` and `exclude` are provided, `types` narrows first, then `exclude` removes from that set. Both accept only known event types (max 25 per parameter). See [events.md](events.md) for full filtering semantics.

**Pagination Parameters** (list endpoint only):

| Parameter | Type | Description |
|-----------|------|-------------|
| `limit` | integer (1-1000) | Max events to return. Enables backward pagination (last N events). |
| `before_sequence` | integer | Cursor: only return events with sequence < this value. Requires `limit`. |

When `limit` is provided:
- Returns the last N events (oldest→newest within batch)
- Response includes `X-Total-Count` header with count of non-delta events
- Turn boundary snapping: batch start snaps to nearest `turn.started` event
- Without `limit`, all events are returned (backward compatible)

### LLM Provider Configuration

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/llm-providers` | List providers |
| POST | `/v1/llm-providers` | Create provider |
| GET | `/v1/llm-providers/{id}` | Get provider |
| PATCH | `/v1/llm-providers/{id}` | Update provider (API key, base URL) |
| DELETE | `/v1/llm-providers/{id}` | Delete provider |
| POST | `/v1/llm-providers/{id}/sync-models` | Sync models from provider API |
| GET | `/v1/llm-providers/{id}/models` | List models for provider |
| POST | `/v1/llm-providers/{id}/models` | Create model for provider |
| GET | `/v1/llm-models` | List all models |
| GET | `/v1/llm-models/{id}` | Get model |
| PATCH | `/v1/llm-models/{id}` | Update model |
| DELETE | `/v1/llm-models/{id}` | Delete model |

#### Model Sync

The sync endpoint discovers available models from a provider's API:

**Request:**
```
POST /v1/llm-providers/{id}/sync-models
```

**Response (success):**
```json
{
  "status": "success",
  "created": 5,
  "updated": 10,
  "stale": 2
}
```

**Response (not supported):**
```json
{
  "status": "not_supported"
}
```

Providers with custom base URLs or providers that don't support model listing return `not_supported`.

#### List Models Query Parameters

`GET /v1/llm-models` supports:
- `source` - Filter by source: `manual`, `discovered`, `predefined`
- `include_stale` - Include stale models (default: `true`)
- `favorites_only` - Only return favorites (default: `false`)

### User Connections

User-scoped external service accounts (e.g., GitHub) for repo access. See [user-connections.md](user-connections.md) for full specification.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/user/connections` | List user's connected accounts |
| DELETE | `/v1/user/connections/{provider}` | Disconnect (delete stored token) |
| GET | `/v1/user/connections/github/authorize` | Start GitHub OAuth flow |
| GET | `/v1/user/connections/github/callback` | GitHub OAuth callback |

### Users

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/users` | List users in current organization (supports `?search=` query) |
| PATCH | `/v1/users/me` | Update current user's profile (`name`, `locale`, `timezone`). |
| DELETE | `/v1/users/me` | Delete current user's account and all associated data |
| GET | `/v1/users/me/export` | Export current user's data (GDPR data portability) |
| POST | `/v1/users/me/switch-org` | Switch current organization context |

`PATCH /v1/users/me` should use patch semantics:
- `name`: optional display name
- `locale`: optional BCP 47 locale
- `timezone`: optional IANA timezone

These values are durable user defaults. They do not override explicit per-message or per-request execution context.

### Agent Capabilities

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List available capabilities |
| GET | `/v1/capabilities/{capability_id}` | Get capability details |

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

The spec is defined in `crates/server/src/openapi.rs`:

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
| POST | `/v1/durable/workers/{id}/drain` | Drain worker (stop accepting tasks) |
| POST | `/v1/durable/workers/{id}/resume` | Resume draining worker |
| GET | `/v1/durable/workflows` | List workflows |
| GET | `/v1/durable/workflows/{id}` | Get workflow details |
| GET | `/v1/durable/workflows/{id}/events` | Get workflow event history |
| GET | `/v1/durable/tasks` | List task queue |
| POST | `/v1/durable/tasks` | Enqueue standalone task (generic queue) |
| GET | `/v1/durable/metrics/timeseries` | Get metrics time series (ring buffer) |
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
| `GET /v1/sessions` | 20 | Ordered by `created_at DESC`, optional `agent_id` filter |

**Non-Paginated List Endpoints:**

These endpoints return all items wrapped in `{"data": [...], "total": N}`:
- `GET /v1/agents` - Returns all agents
- `GET /v1/sessions/{session_id}/messages` - Returns all messages
- `GET /v1/sessions/{session_id}/events` - Returns all events (supports optional `limit`/`before_sequence` pagination)
- `GET /v1/llm-providers` - Returns all providers
- `GET /v1/llm-models` - Returns all models
- `GET /v1/durable/workers` - Returns all workers
- `GET /v1/durable/workflows` - Returns all workflows
- `GET /v1/durable/workflows/{id}/events` - Returns workflow events
- `GET /v1/durable/tasks` - Returns all tasks
- `GET /v1/durable/metrics/timeseries` - Returns metrics ring buffer (max 360 points)
- `GET /v1/durable/dlq` - Returns all DLQ entries
- `GET /v1/durable/circuit-breakers` - Returns all circuit breakers
- `GET /v1/user/connections` - Returns all user connections (array, no wrapper)

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

### Search

All entity list endpoints support an optional `?search=` query parameter for tokenized multi-word search across name and description fields.

**Behavior:**
- The query is split on whitespace into tokens; each token must match somewhere in the concatenated searchable fields (case-insensitive).
- Example: `?search=customer+bot` matches an agent named "Customer Support Bot" because both "customer" and "bot" appear.
- Tokens are capped at 8 to prevent performance degradation from excessively long queries (e.g. pasting a poem).
- LIKE wildcards (`%`, `_`, `\`) in user input are escaped so they are treated as literal characters.
- Empty or whitespace-only search values are treated as no filter.

**Supported endpoints:**

| Endpoint | Fields searched |
|----------|---------------|
| `GET /v1/agents?search=` | `name`, `description` |
| `GET /v1/sessions?search=` | `title` |
| `GET /v1/harnesses?search=` | `name`, `description` |
| `GET /v1/skills?search=` | `name`, `description` |
| `GET /v1/apps?search=` | `name`, `description` |
| `GET /v1/mcp-servers?search=` | `name`, `description` |

**Convention:** When adding new entity types, always include `?search=` support on the list endpoint. Search should match against `name` and `description` at minimum (case-insensitive, tokenized). Empty or whitespace-only search values are treated as no filter.

### Resource Config Endpoints

Each resource exposes a config endpoint returning `ResourceConfigResponse` with the caller's evaluated policy results. UI uses these on load to show/hide controls.

```
GET /v1/{resource}/config → ResourceConfigResponse
```

| Endpoint | Description |
|----------|-------------|
| `GET /v1/harnesses/config` | Harness policies |
| `GET /v1/agents/config` | Agent policies |
| `GET /v1/apps/config` | App policies |
| `GET /v1/sessions/config` | Session policies |
| `GET /v1/mcp-servers/config` | MCP server policies |
| `GET /v1/llm-providers/config` | LLM provider policies |
| `GET /v1/llm-models/config` | LLM model policies |
| `GET /v1/skills/config` | Skill policies |

**Response:**
```json
{
  "policies": {
    "harness.manage": true,
    "harness.dangerous": false
  }
}
```

See `specs/permissions.md` for the full policy model and `ResourceConfigResponse` details.

### Rate Limiting

Global per-IP rate limiting applies to all authenticated API routes (excludes `/health` and `/metrics`). See `crates/server/src/auth/rate_limit.rs` for implementation.

| Scope | Default Limit | Env Var |
|-------|---------------|---------|
| Global API | 120 req/min per IP | `RATE_LIMIT_API_REQUESTS_PER_MINUTE` |
| Login | 10 req/min per IP | — |
| Register | 5 req/min per IP | — |
| Token refresh | 30 req/min per IP | — |

Set `RATE_LIMIT_API_REQUESTS_PER_MINUTE=0` to disable global API rate limiting. Auth endpoint limits are not configurable. Returns `429 Too Many Requests` when exceeded.

### Resource Limits

Configurable limits on resource creation. See `crates/server/src/server.rs` for `ResourceLimits`.

| Resource | Default | Env Var |
|----------|---------|---------|
| Orgs per user | 5 | `RESOURCE_LIMIT_MAX_ORGS_PER_USER` |
| Members per org | 50 | `RESOURCE_LIMIT_MAX_MEMBERS_PER_ORG` |
| API keys per user per org | 10 | `RESOURCE_LIMIT_MAX_API_KEYS_PER_USER_PER_ORG` |

Returns `409 Conflict` when a limit is exceeded.

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
- `409` - Conflict (resource limit exceeded)
- `422` - Unprocessable Entity (validation error)
- `429` - Too Many Requests (rate limited)
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
