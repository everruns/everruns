---
type: Specification
title: "MCP Server Specification"
description: "MCP client remote server registration, CRUD API, tool naming, execution."
tags:
  - everruns
  - integrations
---
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
| `stdio` | Local-process transport, runtime/CLI hosts only; rejected by the hosted control plane (see [runtime-mcp.md](runtime-mcp.md)) |

Future transport types (not yet implemented):
- `websocket` - WebSocket-based transport

### Multi-era protocol support

Everruns' MCP **client** speaks three protocol eras over one HTTP code path, so
it interoperates with servers of any era without operator action:

| Version | Status | Connection model |
|---------|--------|------------------|
| `2025-03-26` | Superseded | Stateful: `initialize` handshake, `Mcp-Session-Id` echoed on every request, `notifications/initialized` |
| `2025-06-18` | Superseded | Stateful (as above) |
| `2026-07-28` | Current | Stateless: no handshake; protocol version + client info ride in `_meta` per request; routable headers let edge infra route without parsing the body |

Eras are referred to by version date throughout. Labels like "stable" and "RC"
were used while `2026-07-28` was in review; they outlived their meaning when it
shipped as a final spec, so they survive only as deserialization aliases.

On every request the client emits `_meta` and the SEP-2243 routable headers
(`MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`). These are additive and
ignored by 2025-era servers, so the same request shape works across all eras.
`_meta` carries what the `initialize` handshake used to negotiate once per
connection:

| `_meta` key | Value |
|---|---|
| `io.modelcontextprotocol/protocolVersion` | the negotiated version in force for this request |
| `io.modelcontextprotocol/clientInfo` | `{name, version}` |
| `io.modelcontextprotocol/clientCapabilities` | `{}` |

The capabilities object is a real declaration, not a placeholder. `sampling` and
`roots` are always absent: each requires a model or a filesystem view to answer a
request mid-call, which this transport cannot reach. `elicitation` is declared —
in `url` mode only — when the host supplied a consent handler, and absent
otherwise. Under MRTR a server **MUST NOT** ask for an input type the client did
not declare, so an accurate declaration is what stops servers blocking on prompts
nobody can answer.

#### Cacheable list results

A `2026-07-28` server returns `ttlMs` and `cacheScope` on `tools/list`. The
client caches the tool list for exactly the TTL the server declared; a missing,
zero, or negative `ttlMs` means "immediately stale" and is not cached at all, so
2025-era servers behave as before.

`cacheScope` decides the cache key. A `public` result is stored
credential-independently and shared across authorization contexts, which is what
the scope exists to permit. A `private` result, or one with no recognized scope,
the conservative default, keeps the credential hash in its key and therefore
never crosses authorization contexts.

#### Multi round-trip requests (MRTR)

A `2026-07-28` server may answer `tools/call` with
`resultType: "input_required"` rather than a result. The client handles both
shapes (`crates/mcp/src/http.rs::resolve_input_required`):

- **No `inputRequests`**: the server only needs the round trip, having stashed
  context in `requestState`. Retry once, echoing `requestState` verbatim under a
  *different* JSON-RPC id, since MRTR treats the retry as an independent request.
- **A URL mode `elicitation/create`**: handled, see [URL mode
  elicitation](#url-mode-elicitation) below.
- **Anything else** (form mode elicitation, sampling, roots): the client
  declares none of these. Fail with an error naming the requested keys rather
  than returning the empty result a caller would misread as success.

Bounded at two rounds (`MAX_INPUT_REQUIRED_ROUNDS`): a server may keep asking,
but each round costs a human interaction and holds the turn open, so looping
would only burn the call timeout.

#### URL mode elicitation

A server that needs a secret, a third-party authorization, or a payment must not
ask the client for it. It sends a URL mode `elicitation/create` inside the MRTR
`input_required` result, and the client gets a human to complete the interaction
out of band. Everruns' half:

- **Declared only when answerable.** The host injects a
  `UrlElicitationHandler` (`crates/mcp/src/elicitation.rs`); without one the
  client declares no `elicitation` capability and a compliant server cannot ask.
  Unattended runs inject nothing, so a background worker never stalls on a
  prompt nobody can answer.
- **The URL is validated before anyone sees it** (`validate_elicitation_url`):
  `https` only, with loopback `http` allowed for local development, so a consent
  surface is never handed a `javascript:`/`file:`/`data:` URL. The host also
  receives the host name and a Punycode flag so it can highlight the domain and
  warn on ambiguous ones.
- **Never fetched.** The client must not pre-fetch the URL or its metadata, and
  does not.
- **Consent, then retry.** `accept` retries the call with
  `inputResponses: {<key>: {action: "accept"}}` plus the echoed `requestState`;
  the server decides whether the out-of-band interaction finished. `decline` and
  `cancel` end the call with an error naming the host, because there is nothing
  to send that would let the server proceed.

This is 2026-07-28 only. In 2025-era servers elicitation is a server-initiated
request over a server→client stream this transport does not open, so the
handshake declares nothing regardless of host capabilities.

#### How the session hosts answer

A turn cannot block on a browser, so consent is collected across a pause rather
than inside the call. The worker host injects `ConsentingUrlElicitations`
(`crates/mcp/src/elicitation.rs`), which answers `accept` only when a human
already consented, and otherwise stands the elicitation down:

1. **First call.** No consent is recorded, so the handler cancels and the
   executor returns a structured tool result (`code:
   "url_elicitation_required"`, carrying `url`, `url_host`, `url_is_punycode`,
   `server`, `tool`, and `retry_tool`) — an expected, user-actionable state
   alongside the existing `credential_required` / `connection_required`
   affordances, never a transport failure.
2. **Pause.** `UrlElicitationHook`
   (`crates/engine/src/execution/act_hooks.rs`) recognises that payload, sets
   `waiting_for_url_elicitation`, and emits a synthetic
   `confirm_url_elicitation` client-side tool call. The session parks in
   `waiting_for_tool_results`, reusing the client-side tool machinery
   (`knowledge/execution/client-side-tools.md`).
3. **Consent.** The UI renders the card (`url-elicitation-tool-call.tsx`): the
   full URL with its domain highlighted, a Punycode warning where it applies,
   and no link that opens without a click. The decision posts to
   `POST /v1/sessions/{id}/mcp-elicitation-consent`, which reads the server,
   tool and domain back out of the emitted event — the browser does not get to
   say what was consented to — records a `StoredConsent` in session storage on
   an accept, completes the synthetic call, and resumes the turn.

   The decision also goes in as a user turn. A tool result cannot carry it: the
   synthetic call came from the engine, so nothing in the transcript claims that
   tool call and the lone result is dropped before the provider request is
   built. Without the spoken line the model never learns consent was given and
   asks for the link a second time.
4. **Retry.** The model calls the tool again; the server elicits again with a
   fresh `requestState`; this time the handler finds the consent and answers
   `accept`, so the server can check whether the out-of-band interaction
   completed.

Three properties of the consent record matter, all enforced in
`StoredConsent::grant_for` and `SessionElicitationConsents`
(`crates/worker/src/mcp_elicitation_consent.rs`):

- **Single use.** It is deleted before it is honoured, so one consent authorises
  exactly one `accept` and the next elicitation asks again.
- **Bound to the domain the user saw.** A server that elicits `pay.example.com`,
  waits for the click, then elicits somewhere else on the retry gets no reuse of
  the consent.
- **Durable and session-scoped**, because the retry may run in a different
  worker process than the call that asked.

Whether the turn pauses at all is a client capability question, so it rides a
session hint: `url_elicitation` (the UI declares it alongside `setup_connection`
at session creation). A client that declares neither — the CLI, an SDK caller —
keeps the older relay behaviour: the turn continues, the elicitation reaches the
user through the tool result, and they re-run the tool themselves. The
in-process runtime host still injects `RelayUrlElicitations` for the same
reason.

Not yet built: a per-server opt-in policy — today any server the operator
configured may elicit, gated only by the host's handler.

#### `protocol_mode`

Each server (org-managed `McpServer` and scoped `ScopedMcpServer`) carries a
`protocol_mode` policy. It defaults to `auto` and is omitted from serialized
config when `auto`, so existing configuration is byte-identical.

| Mode | Behavior |
|------|----------|
| `auto` (default) | Probe and adapt. Tries the stateless `2026-07-28` path first; on a response that signals the server needs a session (e.g. HTTP 400 mentioning `Mcp-Session-Id`, a "not initialized" JSON-RPC error, or an explicit unsupported/invalid protocol-version rejection), transparently runs the stateful handshake and retries once. The verdict (era + session id) is cached per server for a short TTL so a `tools/list` + `tools/call` pair negotiates once. If the fallback also fails, the error reports both failures. |
| `2025-03-26` | Pin `2025-03-26`; always handshake first. |
| `2025-06-18` | Pin `2025-06-18`; always handshake first. |
| `2026-07-28` | Pin `2026-07-28`; never handshake. |

The pre-release values `legacy`, `stable`, and `rc` still deserialize onto
`2025-03-26`, `2025-06-18`, and `2026-07-28` respectively, so stored config
keeps loading without a migration. They are never emitted.

Pinning exists to work around a server that mis-signals its era; `auto` is
correct for essentially all servers. Layering follows the normal
harness→agent→session last-wins merge, so a session can override an inherited
pin.

Persistence: for org-managed servers `protocol_mode` lives in the existing
`settings` JSONB column (no schema migration); for scoped servers it is part of
the embedded `mcpServers` object and propagates to the worker over gRPC
(`McpServerInfo.protocol_mode`).

Error codes are normalized across eras: `2026-07-28` renumbered the older
MCP-specific `-32002` onto the standard JSON-RPC `-32602`, so the client maps
`-32002 → -32602` before surfacing or classifying an error
(`normalize_mcp_error_code`).

The negotiation engine lives in `everruns-mcp` (`protocol.rs` for the pure
pieces, `http.rs` for the egress-bound orchestration); see
[runtime-mcp.md](runtime-mcp.md). Server-side adoption of `2026-07-28` on
Everruns' own `/mcp` endpoint (accepting `_meta`/session-less requests, emitting
`ttlMs`/`cacheScope`) is tracked separately in [mcp.md](mcp.md).

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

1. **API Key Encryption**: API keys are encrypted at rest using envelope encryption (see `knowledge/security/encryption.md`)
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
  refresh is kicked off in the background. OAuth servers are excluded, they
  cannot self-refresh without a user connection token, so they take the blocking
  path instead of spawning a no-op background refresh.
- **Expired stale** (24h or older), **cold** (never fetched), or **forced**: the
  caller blocks on a refresh. Batch agent tool resolution omits expired stale
  tools if that refresh fails rather than registering the old definitions.
- The 24h max-stale window also bounds **OAuth** servers: since they can't
  self-refresh without a user connection token, their cached tools are served
  only while inside the window. Past 24h the blocking refresh fails (and batch
  resolution omits them) until the user reconnects, so revoked/poisoned OAuth
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

### Agent-bound tool-parameter credentials

Some MCP servers require a credential as a tool argument rather than an HTTP
authorization header. Those values are Agent-scoped runtime bindings, not
session secrets and not part of the Agent blueprint. A binding identifies one
attached server endpoint, tool, and top-level parameter. Its value is accepted
only by the write-only Agent Credentials API/UI, encrypted with the control
plane encryption service, and never returned by an API.

Before reason, the bound parameter is removed from the model-visible tool
schema. After the original model tool call is persisted, the MCP executor
rejects an attempted override and injects the decrypted value into a cloned
outbound argument object. The model, events, narration, and worker logs retain
only the original credential-free call. A missing value returns the structured
`credential_required` result with the Agent setup URL.

Bindings are scoped by organization and Agent and are resolved from the
session's Agent identity. They therefore work for shared sessions and
session-per-invocation triggers without granting the same credential to other
Agents. Rotation applies to future calls immediately; deletion revokes the
binding. An endpoint mismatch fails closed as unconfigured.

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
| `everruns-core` | Neutral MCP wire/config types (`McpServer`, `McpToolDefinition`) and transport-independent tool-name helpers (`mcp_tool_name`, `parse_mcp_tool_name`, `is_mcp_tool`) |
| `everruns-mcp` | MCP client transports plus virtual-capability IDs and adapter (`McpCapability`) |
| `everruns-server` | API routes, gRPC services, database operations |
| `everruns-worker` | Runtime adapter and scoped server resolution injected into `everruns-mcp` |

### Key Components

**McpToolExecutor** (`crates/mcp/src/executor.rs`):
- Executes MCP tools by calling remote HTTP endpoints
- Parses tool names to extract server prefix and original tool name
- Caches server info for efficiency
- Handles both plain JSON and SSE response formats

**CompositeToolExecutor** (`crates/mcp/src/executor.rs`):
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
