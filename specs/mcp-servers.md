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

### Tool Name Prefixing

To avoid naming conflicts, MCP tools are prefixed with the server name:
```
mcp_{server_name}_{tool_name}
```

Example: If server `microsoft_learn` provides tool `search`, it becomes `mcp_microsoft_learn_search`.

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

**Response:**
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
