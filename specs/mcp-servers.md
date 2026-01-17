# MCP Server Specification

## Abstract

This document defines the data model and API for MCP (Model Context Protocol) servers in Everruns. MCP servers extend agent capabilities by providing external tools through a standardized protocol. MCP servers appear as "virtual capabilities" in the capability system, allowing agents to use MCP tools alongside built-in capabilities.

## Requirements

### McpServer

Configuration for a remote MCP server connection. Currently supports only HTTP (Streamable HTTP) transport.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Unique name (used as tool prefix) |
| `description` | string? | Optional description of the MCP server |
| `url` | string | Server endpoint URL |
| `transport_type` | enum | Transport type: `http` |
| `status` | enum | `active` or `disabled` |
| `api_key_set` | boolean | Whether API key is configured |
| `api_key_encrypted` | bytes? | Encrypted API key (not exposed via API) |
| `headers` | map[string]string | Additional HTTP headers for authentication |
| `settings` | object | Server-specific settings (reserved for future use) |
| `cached_tools` | json | Cached tool definitions from server |
| `tools_cached_at` | timestamp? | When tools were last cached |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Input Validation Limits:**

| Field | Max Size | Notes |
|-------|----------|-------|
| `name` | 255 chars | Must be unique, non-empty, snake_case recommended |
| `description` | 10 KB | Optional description |
| `url` | 2 KB | Valid HTTP/HTTPS URL |
| `headers` | 100 entries | Maximum header entries |

### Transport Types

Currently supported transport types:

| Type | Description |
|------|-------------|
| `http` | HTTP-based MCP transport (Streamable HTTP) |

Future transport types (not yet implemented):
- `stdio` - Local process with stdio communication
- `websocket` - WebSocket-based transport

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

Tools are cached with hybrid TTL strategy:
- Tools are fetched on first access or when cache is stale (24h TTL)
- Background refresh for commonly used servers
- Force refresh available via API

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
- Multiple content blocks → `{ "content": [...] }`
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
| `everruns-control-plane` | API routes, gRPC services, database operations |
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
