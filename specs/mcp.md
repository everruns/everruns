# MCP (Model Context Protocol) Specification

## Abstract

This is the umbrella specification for MCP in Everruns. MCP enables agents to use external tools hosted on remote servers via a standardized protocol. Everruns acts as both an **MCP client** (calling remote MCP servers) and an **MCP server** (exposing agent tools to external MCP clients like Claude Desktop or Cursor).

This spec covers:

- **Protocol fundamentals** — JSON-RPC, transports, content types
- **MCP client** — connecting to remote MCP servers, tool discovery, execution, caching
- **MCP server** — exposing Everruns as an MCP endpoint for external clients
- **OAuth** — authenticating external MCP clients via OAuth 2.1 + PKCE
- **Capabilities integration** — MCP servers as virtual capabilities
- **Security** — SSRF, auth, encryption, threat mitigations

For detailed sub-topics, see:

- [`specs/mcp-servers.md`](mcp-servers.md) — MCP server registration, CRUD API, tool naming, execution, caching
- [`specs/mcp-oauth.md`](mcp-oauth.md) — OAuth 2.1 endpoints, dynamic client registration, PKCE, token lifecycle

## Protocol Overview

Everruns implements the [Model Context Protocol](https://spec.modelcontextprotocol.io) for tool interoperability. The protocol uses JSON-RPC 2.0 over HTTP.

### Transports

| Transport | Status | Description |
|-----------|--------|-------------|
| `http` | Supported | Streamable HTTP (POST with JSON-RPC body) |
| `stdio` | Planned | Local process with stdio communication |
| `websocket` | Planned | WebSocket-based transport |

### JSON-RPC Methods

| Method | Direction | Description |
|--------|-----------|-------------|
| `tools/list` | Client → Server | Discover available tools |
| `tools/call` | Client → Server | Execute a tool |

### Content Types

MCP tools exchange structured content blocks:

| Type | Fields | Description |
|------|--------|-------------|
| `text` | `text` | Plain text |
| `image` | `data`, `mime_type` | Base64-encoded image |
| `resource` | `uri`, `mime_type`, `text` | External resource reference |

### Response Formats

Servers may respond in plain JSON or Server-Sent Events (SSE):

- **Plain JSON**: Standard JSON-RPC response body
- **SSE**: Lines prefixed with `event:` / `data:` — executor auto-detects and extracts

## Architecture

### Roles

**Everruns as MCP Client** — connects to remote MCP servers registered by users, discovers their tools, and makes them available to agents. See [`specs/mcp-servers.md`](mcp-servers.md).

**Everruns as MCP Server** — exposes agent tools to external MCP clients (Claude Desktop, Cursor, etc.) via the `/mcp` endpoint, authenticated with OAuth 2.1. See [`specs/mcp-oauth.md`](mcp-oauth.md).

### Crate Map

| Crate | Module | Responsibility |
|-------|--------|----------------|
| `everruns-core` | `mcp_server.rs` | Domain types (`McpServer`, `McpToolDefinition`, content types), tool name helpers |
| `everruns-core` | `capabilities/mcp.rs` | Virtual capability wrapper, tool-to-definition conversion |
| `everruns-server` | `api/mcp_servers.rs` | HTTP CRUD routes for MCP server management |
| `everruns-server` | `services/mcp_server.rs` | Business logic, tool caching, permission policies |
| `everruns-server` | `storage/repositories/mcp_servers.rs` | PostgreSQL persistence |
| `everruns-server` | `auth/mcp_oauth.rs` | OAuth 2.1 endpoints (register, authorize, token) |
| `everruns-worker` | `mcp_executor.rs` | HTTP tool execution, SSE parsing, image extraction |
| `everruns-internal-protocol` | `worker.proto` | gRPC for tool context and server resolution |

### Tool Naming Convention

MCP tools are prefixed to avoid collisions with built-in tools:

```
mcp_{server_name}__{tool_name}
```

Double underscore (`__`) separates server name from tool name because server names can contain single underscores (e.g., `microsoft_learn`).

### Capability ID Format

```
mcp:{server_uuid}
```

Enabling this capability on an agent makes all tools from the MCP server available.

## Authentication Modes (Outbound)

When Everruns calls a remote MCP server, three auth modes are supported:

| Mode | Description |
|------|-------------|
| `none` | No authentication |
| `api_key` | API key sent as `Authorization: Bearer` header. Encrypted at rest (see `specs/encryption.md`). |
| `oauth` | OAuth token obtained via provider-specific flow, stored as encrypted session secret. |

## Authentication (Inbound — MCP OAuth)

External MCP clients authenticate via OAuth 2.1 with mandatory PKCE (S256). See [`specs/mcp-oauth.md`](mcp-oauth.md) for full details.

Key points:

- Dynamic client registration (RFC 7591)
- Authorization code grant with PKCE (RFC 7636)
- Backend-agnostic — works with any `AuthBackend` implementation
- JWTs with `token_type: "mcp_access"` claim

## Security

| Threat | Mitigation |
|--------|------------|
| SSRF via MCP server URL | URLs validated on create/update and re-validated at execution time. Private IPs, loopback, link-local, and cloud metadata endpoints blocked. |
| API key exposure | Encrypted at rest (envelope encryption). Never returned in API responses. |
| OAuth code interception | PKCE mandatory (S256). Codes hashed (SHA-256), 5-min TTL, one-time use. |
| Client secret leak | Stored hashed. Shown only at registration. |
| Open redirect | Redirect URIs must exactly match pre-registered values. |
| Tool name collision | Double-underscore prefix scheme ensures unambiguous tool routing. |
| Unauthorized MCP access | Permission policies: `MCP_SERVER_VIEW`, `MCP_SERVER_MANAGE`, `MCP_SERVER_DANGEROUS`. |

See `specs/threat-model.md` for the full threat model.

## LLM Provider Compatibility

| Provider | Notes |
|----------|-------|
| Anthropic | Empty text content blocks filtered before sending (API rejects empty strings) |
| OpenAI | No special handling needed |

Image content from MCP tools is extracted as native `ToolResultImage` blocks for multimodal LLM consumption.
