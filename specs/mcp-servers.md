# MCP Server Specification

## Abstract

This document defines the data model and API for MCP (Model Context Protocol) servers in Everruns. MCP servers extend agent capabilities by providing external tools and resources through a standardized protocol.

## Requirements

### McpServer

Configuration for a remote MCP server connection. Currently supports only HTTP (Streamable HTTP) transport.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Unique display name |
| `description` | string? | Optional description of the MCP server |
| `url` | string | Server endpoint URL |
| `transport_type` | enum | Transport type: `http` |
| `status` | enum | `active` or `disabled` |
| `api_key_set` | boolean | Whether API key is configured |
| `api_key_encrypted` | bytes? | Encrypted API key (not exposed via API) |
| `headers` | map[string]string | Additional HTTP headers for authentication |
| `settings` | object | Server-specific settings (reserved for future use) |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Input Validation Limits:**

| Field | Max Size | Notes |
|-------|----------|-------|
| `name` | 255 chars | Must be unique, non-empty |
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
  "name": "atlassian-mcp-server",
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

### Usage with Agents

MCP servers can be associated with agents to extend their tool capabilities. When an agent session starts, the platform can:

1. Connect to configured MCP servers
2. Retrieve available tools from each server
3. Make tools available to the agent's LLM

Note: Agent-MCP server association is planned for a future release.
