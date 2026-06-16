# MCP Server Specification

> Part of the [MCP spec family](mcp.md). This document covers MCP server registration, CRUD API, tool naming, discovery, and execution. For MCP-client support in the in-process runtime (shared `everruns-mcp` crate, HTTP + optional stdio transport, pluggable auth), see [runtime-mcp.md](runtime-mcp.md).

## Abstract

This document defines the data model and API for MCP (Model Context Protocol) servers in Everruns. MCP servers extend agent capabilities by providing external tools through a standardized protocol. MCP servers appear as "virtual capabilities" in the capability system, allowing agents to use MCP tools alongside built-in capabilities.

## Requirements

### McpServer

Configuration for an organization-managed remote MCP server connection. Organization MCP servers are always remote HTTP (Streamable HTTP); stdio is rejected on create/update and is only available to single-tenant runtime/CLI hosts (see [runtime-mcp.md](runtime-mcp.md)).

See `crates/core/src/mcp_server.rs` for the full `McpServer` struct definition.

**Input Validation Limits:**

| Field | Max Size | Notes |
|-------|----------|-------|
| `name` | 255 chars | Must be unique, non-empty, snake_case recommended |
| `description` | 10 KB | Optional description |
| `url` | 2 KB | Valid HTTP/HTTPS URL |
| `headers` | 100 entries | Maximum header entries |

### Transport Types

Supported transport types:

| Type | Description |
|------|-------------|
| `http` | HTTP-based MCP transport (Streamable HTTP) |
| `stdio` | Local-process transport — runtime/CLI hosts only; rejected by the hosted control plane (see [runtime-mcp.md](runtime-mcp.md)) |

Future transport types (not yet implemented):
- `websocket` - WebSocket-based transport

### Scoped `mcpServers`

In addition to organization-managed MCP server records, harnesses, agents, and
sessions may embed remote MCP server config directly in a `mcpServers` object.
This is intended for session-local or agent-local MCP wiring without creating an
organization-global MCP server.

The shape matches the remote-server subset of `.mcp.json`:

```json
{
  "mcpServers": {
    "docs": {
      "type": "http",
      "url": "https://example.com/mcp",
      "headers": {
        "Authorization": "Bearer test-token"
      }
    }
  }
}
```

Constraints:
- In the hosted control plane, only remote HTTP scoped servers are accepted; stdio scoped servers are rejected by validation. Single-tenant runtime/CLI hosts may also use stdio (see [runtime-mcp.md](runtime-mcp.md)).
- Names must be unique after Everruns MCP tool-name sanitization.
- URLs use the same SSRF-safe validation as organization-managed MCP servers.
- Scoped servers are not stored as org capabilities and do not appear in org MCP CRUD APIs.
- Scoped headers are literal values; `${ENV}` placeholders are not expanded.

Merge order:
1. Harness `mcpServers`
2. Agent `mcpServers`
3. Session `mcpServers`

Later layers override earlier layers by logical server name.

Runtime behavior:
- Effective scoped servers are resolved before organization-managed MCP servers.
- Scoped tool discovery is live at preview/runtime; there is no persisted org-level cache row.
- Scoped tool names use the same `mcp_{server_name}__{tool_name}` prefixing as org MCP servers.

### Status Values

| Status | Description |
|--------|-------------|
| `active` | Server is enabled and available for use |
| `disabled` | Server is disabled and not used |

### API Endpoints

#### POST /v1/mcp-servers

Create a new MCP server configuration.

**Request Body:**
```json
{
  "name": "atlassian_mcp",
  "description": "Atlassian MCP Server for Jira and Confluence",
  "url": "https://mcp.atlassian.com/v1/mcp",
  "transport_type": "http",
  "api_key": "optional-api-key",
  "headers": {
    "X-Custom-Header": "value"
  }
}
```

**Response:** `201 Created` with McpServer object

#### GET /v1/mcp-servers

List all MCP servers.

**Response:** `200 OK`
```json
{
  "data": [McpServer, ...]
}
```

#### GET /v1/mcp-servers/{server_id}

Get a specific MCP server by ID.

**Response:** `200 OK` with McpServer object, or `404 Not Found`

#### PATCH /v1/mcp-servers/{server_id}

Update an MCP server. Only provided fields are updated.

**Request Body:**
```json
{
  "description": "Updated description",
  "status": "disabled",
  "api_key": "new-api-key"
}
```

**Response:** `200 OK` with updated McpServer object

#### DELETE /v1/mcp-servers/{server_id}

Delete an MCP server.

**Response:** `204 No Content` on success, `404 Not Found` if not exists

### Security Considerations

1. **API Key Encryption**: API keys are encrypted at rest using envelope encryption (see `specs/encryption.md`)
2. **API Key Not Exposed**: The `api_key_encrypted` field is never returned in API responses
3. **Unique Names**: Server names must be unique to prevent configuration conflicts
4. **SSRF Protection**: MCP server URLs are validated on create/update (static check) and re-validated with DNS resolution before each tool call and `tools/list` fetch (`validate_url_dns_pinned`). Private IPs, loopback, link-local, and cloud metadata endpoints are blocked; the DNS-pinned check also prevents DNS-rebinding attacks by verifying every resolved IP on each outbound request (TM-TOOL-018).

## MCP as Virtual Capabilities

MCP servers integrate into the capability system as "virtual capabilities". This allows agents to select MCP servers alongside built-in capabilities using the same UI.

### Capability ID Format

MCP capabilities use a prefixed ID format:
```
mcp:{server_uuid}
```

Example: `mcp:01933b5a-0000-7000-8000-000000000501`

When this capability is enabled on an agent, tools from this MCP server become available with prefixed names (e.g., `mcp_microsoft_learn__search`).

### Tool Name Prefixing

To avoid naming conflicts, MCP tools are prefixed with the server name using a **double underscore** separator:
```
mcp_{server_name}__{tool_name}
```

The double underscore (`__`) separator is used instead of single underscore because server names can contain underscores (e.g., `microsoft_learn`). This allows unambiguous parsing of the tool name back to its components.

**Examples:**
- Server `microsoft_learn` with tool `search` → `mcp_microsoft_learn__search`
- Server `github` with tool `search_repos` → `mcp_github__search_repos`
- Server `atlassian_jira` with tool `create_issue` → `mcp_atlassian_jira__create_issue`

**Parsing:** To extract server name and tool name:
1. Verify the name starts with `mcp_`
2. Find the double underscore separator `__`
3. Server name is between `mcp_` and `__`
4. Tool name is everything after `__`

### Tool Discovery

Tools are discovered from MCP servers via the `tools/list` JSON-RPC method:

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list"
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [
      {
        "name": "search",
        "description": "Search for content",
        "inputSchema": { "type": "object", ... }
      }
    ]
  }
}
```

### Tool Caching

Tools are cached per server (`cached_tools` + `tools_cached_at`) and served with
a bounded stale-while-revalidate strategy so recent stale caches avoid blocking
agent tool resolution on an upstream `tools/list`, while untrusted tool
definitions cannot remain registered indefinitely after refresh failures:

- **Fresh** (within the 1h TTL): cached tools are returned directly.
- **Stale but within the maximum stale lifetime** (older than the TTL, under 24h,
  with a prior successful fetch): cached tools are returned immediately and a
  refresh is kicked off in the background. OAuth servers are excluded — they
  cannot self-refresh without a user connection token, so they take the blocking
  path instead of spawning a no-op background refresh.
- **Expired stale** (24h or older), **cold** (never fetched), or **forced**: the
  caller blocks on a refresh. Batch agent tool resolution omits expired stale
  tools if that refresh fails rather than registering the old definitions.
- The 24h max-stale window also bounds **OAuth** servers: since they can't
  self-refresh without a user connection token, their cached tools are served
  only while inside the window. Past 24h the blocking refresh fails (and batch
  resolution omits them) until the user reconnects — so revoked/poisoned OAuth
  tool metadata can't be served indefinitely either.
- Concurrent refreshes for the same server are coalesced (single-flight), so a
  burst of agent runs triggers at most one upstream fetch rather than a herd.
- Force refresh is available via API.

### Tool Execution

MCP tools are executed via the `tools/call` JSON-RPC method:

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "search",
    "arguments": { "query": "Azure functions" }
  }
}
```

**Response (Plain JSON):**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      { "type": "text", "text": "Search results..." }
    ],
    "isError": false
  }
}
```

**Response (Server-Sent Events format):**

Some MCP servers (e.g., Microsoft Learn) return responses in SSE format:
```
event: message
data: {"jsonrpc":"2.0","id":1,"result":{"content":[...],"isError":false}}
```

The executor automatically detects and parses both formats:
- If response starts with `event:` or contains `\ndata:`, parse as SSE
- Extract JSON from the `data:` line
- Otherwise, parse as plain JSON

### MCP Content Types

MCP tool results can contain various content types:

| Type | Fields | Description |
|------|--------|-------------|
| `text` | `text` | Plain text content |
| `image` | `data`, `mime_type` | Base64-encoded image |
| `resource` | `uri`, `mime_type`, `text` | External resource reference |

The executor converts MCP content to the internal format:
- Single text content → simplified `{ "result": "text" }`
- Multiple text/resource blocks → `{ "content": [...] }`
- **Image content → extracted as `ToolResultImage`** and sent to the LLM as native image content blocks (not embedded in JSON). This enables the LLM to visually analyze images returned by MCP tools.
- Image-only responses → `{ "result": "[N image(s)]" }` with images as separate `ToolResult.images`
- Errors → `{ "error": "message" }`

### LLM Provider Compatibility

**Anthropic Provider:**
- Text content blocks must be non-empty (Anthropic API rejects empty strings)
- Assistant messages with tool calls but empty text are valid - the empty text is filtered out
- The conversion layer automatically filters empty text blocks before sending to the API

**OpenAI Provider:**
- Empty text content is allowed
- No special filtering required

### UI Integration

In the capability selector UI, MCP capabilities are displayed with:
- An "MCP" badge to distinguish them from built-in capabilities
- Server name as the capability name
- Server description as the capability description
- List of available tools

## Seed Data

The following MCP server is seeded by default:

| Name | URL | Description |
|------|-----|-------------|
| `microsoft_learn` | `https://learn.microsoft.com/api/mcp` | Microsoft Learn documentation server |

A demo agent "Microsoft Learn Assistant" is also seeded, configured to use this MCP server.

## Implementation Details

### Crate Structure

| Crate | Responsibility |
|-------|----------------|
| `everruns-core` | MCP types (`McpServer`, `McpToolDefinition`), tool name helpers (`mcp_tool_name`, `parse_mcp_tool_name`, `is_mcp_tool`) |
| `everruns-server` | API routes, gRPC services, database operations |
| `everruns-worker` | `McpToolExecutor` for HTTP calls, `CompositeToolExecutor` for routing |

### Key Components

**McpToolExecutor** (`crates/worker/src/mcp_executor.rs`):
- Executes MCP tools by calling remote HTTP endpoints
- Parses tool names to extract server prefix and original tool name
- Caches server info for efficiency
- Handles both plain JSON and SSE response formats

**CompositeToolExecutor** (`crates/worker/src/mcp_executor.rs`):
- Routes tool calls to appropriate executor
- MCP tools (prefixed with `mcp_`) → McpToolExecutor
- Built-in tools → ToolRegistry

**gRPC Protocol** (`crates/internal-protocol/proto/worker.proto`):
- `GetTurnContext` returns `mcp_tool_definitions` with prefixed tool names
- `GetMcpServerByPrefix` resolves server info by name prefix

### Error Handling

| Error | Response |
|-------|----------|
| Invalid tool name format | `400 Bad Request` with error message |
| MCP server not found | `404 Not Found` |
| MCP server unreachable | `502 Bad Gateway` with timeout after 60s |
| MCP tool returns error | Success with `{ "error": "message" }` in result |
| JSON-RPC error | Error propagated with code and message |
