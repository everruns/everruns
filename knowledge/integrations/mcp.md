---
type: Specification
title: "MCP (Model Context Protocol) Specification"
description: "MCP server endpoint, OAuth 2.1 authentication, protocol, security."
tags:
  - everruns
  - integrations
---
# MCP (Model Context Protocol) Specification

## Abstract

Everruns exposes an MCP server endpoint (`/mcp`) so external MCP clients, Claude Desktop, Cursor, VS Code, etc., can interact with Everruns agents and tools over the [Model Context Protocol](https://spec.modelcontextprotocol.io). Authentication uses MCP-specific authentication and organization resolution: local anonymous identity is accepted only in `AUTH_MODE=none`, while deployed clients use personal access tokens or OAuth 2.1-issued MCP access tokens with mandatory PKCE and an exact resource audience. Regular browser session tokens are not accepted.

The endpoint is always mounted and is not part of the deployment or organization feature-flag
catalog. `FEATURE_MCP_ENDPOINT` is not a supported configuration variable, and stale
`mcp_endpoint` organization rows have no effect. Access is gated per request by MCP-specific
authentication, exact OAuth resource/audience binding, organization resolution, command policy,
and rate limiting rather than by availability opt-in.

Routing is intentionally split:

- REST API lives under `/api/*`
- MCP OAuth lives at `/oauth/*` (authorize, token, register)
- MCP JSON-RPC lives at `/mcp`
- OAuth discovery metadata lives at `/.well-known/oauth-authorization-server`
- Protected-resource metadata lives at `/.well-known/oauth-protected-resource/mcp` (RFC 9728 §3.1 path-derived for the `/mcp` resource)

Everruns also acts as an **MCP client** (connecting to remote MCP servers). That side is covered in [`knowledge/integrations/mcp-servers.md`](mcp-servers.md), with the in-process runtime path (shared `everruns-mcp` crate, HTTP + optional stdio transport, pluggable auth) in [`knowledge/integrations/runtime-mcp.md`](runtime-mcp.md). The client speaks the legacy (`2025-03-26`), current (`2025-06-18`), and 2026 stateless RC (`2026-07-28`) eras, auto-negotiated per server, see [`knowledge/integrations/mcp-servers.md`](mcp-servers.md) ("Multi-era protocol support").

> **Server-side RC adoption (follow-up).** This document describes Everruns' own `/mcp` *server* endpoint, which today negotiates `2025-06-18`/`2025-03-26`. Adopting the 2026 stateless RC on the server side, accepting session-less `_meta`-bearing requests, emitting `ttlMs`/`cacheScope` cache directives, and honoring the routable headers, is a separate workstream from the client-side multi-era support already shipped. When taken on, mind the `cacheScope` tenant-isolation requirement (per-user vs shared tool caches) given the per-user OAuth model below.

## Protocol

JSON-RPC 2.0 over Streamable HTTP (`POST /mcp` with JSON-RPC body).

Everruns supports MCP protocol versions `2026-07-28`, `2025-06-18`, and `2025-03-26`.

- `initialize` negotiates `protocolVersion` from the request body. When the client omits a version, Everruns falls back to `2025-03-26`. An exactly-supported version is echoed back; an unrecognized version negotiates down to the newest supported version that is not newer than the request (a client ahead of Everruns receives `2026-07-28`).
- All non-`initialize` requests may send `MCP-Protocol-Version`. When omitted, Everruns falls back to `2025-03-26`.
- Requests that send an unsupported `MCP-Protocol-Version` header are rejected with HTTP `400 Bad Request` and a JSON-RPC error payload.

### Statelessness (MCP 2026-07-28)

The endpoint is stateless request/response per JSON-RPC call, no `Mcp-Session-Id`, no sticky sessions, no server-side per-connection state. This already satisfies the `2026-07-28` stateless model: any request can hit any instance, with PostgreSQL as the shared source of truth. Adopting `2026-07-28` was protocol conformance, not re-architecture. Concretely:

- **`initialize` is optional.** `2026-07-28` removed the `initialize`/`initialized` handshake (SEP-2575). Everruns still accepts `initialize` for older clients because it creates no server state either way, and never requires a prior `initialize` for any method.
- **Client info rides in `_meta`.** Per-request client identity is read from `params._meta["io.modelcontextprotocol/clientInfo"]` (`{name, version}`) and used for telemetry only; nothing is stored.
- **Routing headers.** `2026-07-28` Streamable HTTP adds optional `Mcp-Method` and `Mcp-Name` request headers so gateways/load-balancers/rate-limiters can route on the operation without parsing the body. They are optional and the body stays authoritative; when present they must be singular and agree with the body (`Mcp-Method` vs the JSON-RPC `method`, and `Mcp-Name` vs `params.name` on `tools/call`), otherwise the request is rejected `400 Bad Request`. See [`knowledge/operations/production-deployment.md`](../operations/production-deployment.md#mcp-endpoint-scaling) for the proxy contract.
- **The richer tool shape** (`title`, `outputSchema`, `structuredContent`, entity-card tools) introduced in `2025-06-18` applies to `2025-06-18` and every later version, including `2026-07-28`; only the `2025-03-26` fallback omits it.
- **Result metadata.** `2026-07-28` requires every complete result to carry `resultType: "complete"`, and the listing/reading operations to carry caching hints. See [Cacheable results](#cacheable-results-2026-07-28).

### Cacheable results (2026-07-28)

Results are decorated at the dispatch layer, not inside each handler: the policy is per-method and per-era, and the handlers have many success paths (`crates/server/src/api/mcp_endpoint/caching.rs`).

| Method | `resultType` | `ttlMs` | `cacheScope` |
|--------|--------------|---------|--------------|
| `tools/list` | `complete` | 5 min | `public` |
| `resources/list` | `complete` | 5 min | `public` |
| `resources/read` | `complete` | 30 s | `private` |
| `tools/call` | `complete` (or `task`) |, |, |

`public` is only correct where the payload is identical for every caller, both list catalogs are static per protocol version, so a shared cache cannot leak between orgs. `resources/read` returns org-scoped data (agents, capabilities, harnesses, models) and is therefore `private`, since a cache keyed on the URI alone would serve one org's data to another. Tool results are never cacheable and carry `resultType` only; the Tasks extension's `resultType: "task"` takes precedence when a task handle is issued.

Hints are emitted only under the negotiated `2026-07-28` protocol. They would be ignored by 2025-era clients, but gating keeps each era's responses exactly what that era specifies.

The Tasks extension (server-directed long-running `tools/call` driven by `tasks/get`/`tasks/update`/`tasks/cancel`) is implemented as optional, additive interop alignment for the existing `agent_run` → `session_get_status` poll pattern. See [Tasks extension (2026-07-28)](#tasks-extension-2026-07-28) below.

### Supported Methods

| Method | Description |
|--------|-------------|
| `initialize` | Initialize MCP session and negotiate capabilities |
| `ping` | Health check / keep-alive |
| `tools/list` | Discover available tools |
| `tools/call` | Execute a tool |
| `resources/list` | Discover static Everruns resources |
| `resources/read` | Read a static Everruns resource by URI |
| `tasks/get` | (2026-07-28 Tasks extension) Poll a task handle for status + result |
| `tasks/update` | (2026-07-28 Tasks extension) Provide input to a task in `input_required` |
| `tasks/cancel` | (2026-07-28 Tasks extension) Request cancellation of a task |

`tasks/list` is intentionally not implemented, SEP-2663 removed it, and Everruns never exposed a server-side task list. The `tasks/*` methods are only routed under the negotiated `2026-07-28` protocol; a `2025-*` client that sends them gets `-32601 Method not found`.

### Entity Cards

Under negotiated protocol `2025-06-18`, Everruns exposes per-entity card
tools (`agent_get_card`, future `session_get_card`, …) that return an MCP
`resource` content block of MIME type `text/html` at the `ui://` scheme
along with a one-line text summary. MCP-Apps-aware hosts render the HTML
in a sandboxed iframe; hosts that ignore embedded resources still see the
summary. See [`knowledge/ui/mcp-cards.md`](../ui/mcp-cards.md) for the standard,
including the URI scheme, sandboxing requirements, and the planned
`postMessage` action protocol that backs future mutation buttons.

### Tool Metadata

`tools/list` returns only standard MCP tool fields.

- Under negotiated protocol `2025-06-18`, tool definitions may include `title`, `description`, `inputSchema`, `outputSchema`, and `annotations`.
- Under negotiated protocol `2025-03-26`, Everruns emits the older compatibility shape and omits newer `2025-06-18` fields such as `title` and `outputSchema`.
- `outputSchema` is attached only to tools with stable JSON outputs.
- Under negotiated protocol `2025-06-18`, when a tool has an `outputSchema` and returns valid JSON text, `tools/call` also returns `structuredContent` alongside the textual content block.

### Content Types

| Type | Fields | Description |
|------|--------|-------------|
| `text` | `text` | Plain text |
| `image` | `data`, `mime_type` | Base64-encoded image |
| `resource` | `uri`, `mime_type`, `text` | External resource reference |

## Tasks extension (2026-07-28)

Everruns' `/mcp` server implements the MCP Tasks extension
(`io.modelcontextprotocol/tasks`, SEP-2663) as optional, **additive** interop
alignment. It is not new capability: Everruns already runs long agent turns as
the poll pattern (`agent_run` returns a session id + hint, clients poll
`session_get_status`), all state Postgres-backed with no server-side session
memory. Tasks is the standardized vocabulary for that same pattern, so a
**task handle is a session id**. Field names follow final SEP-2663 and the
official Tasks extension overview: task handles use `ttlMs` and
`pollIntervalMs`.

The whole surface is gated: it activates only when the negotiated protocol is
`2026-07-28` **and** the client advertised the extension. `2025-*` clients (and
`2026-07-28` clients that did not opt in) see today's shapes byte-for-byte
unchanged.

### Capability negotiation

- **Server advertises** the extension in `initialize` capabilities under
  `capabilities.extensions["io.modelcontextprotocol/tasks"]`, only when the
  negotiated protocol is `2026-07-28`.
- **Client opts in** per request via
  `params._meta["io.modelcontextprotocol/clientCapabilities"].extensions["io.modelcontextprotocol/tasks"]`.
  Per SEP-2663 the server must never return a task to a client that did not
  declare support, this is what keeps the change back-compatible.

### Session ↔ task mapping

| Tasks extension | Everruns equivalent |
|-----------------|---------------------|
| task handle / `taskId` | `session_id` (Postgres-backed, instance-agnostic) |
| `tools/call` returns `CreateTaskResult` | `agent_run` / `session_send_message` return the task handle alongside their existing fields |
| `tasks/get` | `session_get_status` (status + events, surfaced under `result`) |
| `tasks/update` (provide input on `input_required`) | `session_send_message` |
| `tasks/cancel` | `cancel_session` (cooperative) |
| lifecycle state | derived from session status |
| `tasks/list` | not implemented (removed by SEP-2663) |

### Status mapping

Session status → task lifecycle state:

| Session status | Task status |
|----------------|-------------|
| `started`, `active` | `working` |
| `waiting_for_tool_results`, `paused` | `input_required` |
| `idle` | `completed` |

`failed` and `cancelled` are part of the SEP-2663 vocabulary but are not
persisted session statuses today (cancellation emits a turn event and the
session returns to `idle`), so they are unreachable from status alone. The
mapping helper is total against the vocabulary so callers with stronger
information can still report them.

### Task-handle shape and additivity

When active, `agent_run` / `session_send_message` merge `CreateTaskResult`
fields (`resultType: "task"`, `taskId`, `status`, `ttlMs`, `pollIntervalMs`)
into the tools/call `result`; the existing `content` / `structuredContent` are
untouched. `tasks/get` returns a `Task` object (`taskId`, `status`, `ttlMs`,
`pollIntervalMs`) with the full `session_get_status` payload under `result`.

**Structured result.** When the task's session reported a deterministic,
schema-bound result (`result.json`, produced by a task declared with a
`result_schema`, see [`knowledge/runtime-resources/subagents.md`](../runtime-resources/subagents.md) and
[`knowledge/runtime-resources/session-tasks.md`](../runtime-resources/session-tasks.md)), `tasks/get` adds that JSON under
`result.structured_result`, so Tasks clients get the machine result instead of
re-parsing last-message text. It is additive: the existing status snapshot
(session status, latest output, events) is unchanged, and a plain agent turn
that reported no structured result omits the field. Retrieval is scoped by the
same org `session_get_status` already validated, so tenant isolation is
preserved; when a session reported more than one structured result the most
recently updated one wins.

Implementation: `crates/server/src/api/mcp_endpoint/tasks.rs` (mapping helpers,
capability gating, task-handle shapes) and `mod.rs` (`handle_tasks_method` and
the `handle_tools_call` augmentation).

## Architecture

The Tier-2 `discover`, `query`, and `execute` tools share their catalog search,
Bashkit execution, positional rewriting, limits, and output/error formatting
with the built-in `platform` capability. The shared implementation is
`crates/server/src/services/platform_command_surface.rs`; MCP authentication and
organization selection remain endpoint-adapter concerns. This prevents the
agent-facing platform catalog from drifting from `/mcp`.

### Crate Map

| Crate | Module | Responsibility |
|-------|--------|----------------|
| `everruns-server` | `auth/mcp_oauth.rs` | OAuth 2.1 endpoints (register, authorize, token) |
| `everruns-server` | `api/mcp_servers.rs` | HTTP CRUD routes for MCP server management |
| `everruns-server` | `services/mcp_server.rs` | Business logic, tool caching, permission policies |
| `everruns-server` | `storage/repositories/mcp_servers.rs` | PostgreSQL persistence |
| `everruns-core` | `mcp_server.rs` | Domain types (`McpServer`, `McpToolDefinition`, content types), tool name helpers |
| `everruns-mcp` | `capability.rs` | Virtual capability wrapper, tool-to-definition conversion |
| `everruns-mcp` | `http.rs` | HTTP tool execution, SSE parsing, image extraction |
| `everruns-internal-protocol` | `worker.proto` | gRPC for tool context and server resolution |

## OAuth 2.1 Authentication

External MCP clients authenticate via OAuth 2.1 with mandatory PKCE (S256). The module is **backend-agnostic**: it works identically with BuiltinAuthBackend (OSS) and external providers like PropelAuth (SaaS). It follows the same pattern as CLI auth: the authorize endpoint requires an authenticated user via the `AuthUser` extractor, delegating identity verification to whatever auth backend is configured.

### Design Decisions

1. **Backend-agnostic**: MCP OAuth sits alongside the auth backend, not inside it. No changes to `AuthBackend` trait.
2. **Same pattern as CLI auth**: The authorize endpoint requires `AuthUser`, works with any backend.
3. **OAuth 2.1 + PKCE**: Authorization code grant with mandatory PKCE (S256). No implicit grant.
4. **Dynamic client registration**: Per RFC 7591, MCP clients register themselves at runtime.
5. **MCP OAuth tokens are standard JWTs**: Signed with the same `AUTH_JWT_SECRET`, using the standard `token_type: "access"` claim (same as regular access tokens).
6. **Scoped to org**: Authorization grants are scoped to a specific organization.

### Flow

```
MCP Client → GET {issuer}/.well-known/oauth-authorization-server
           ← Server metadata (endpoints, PKCE support)

MCP Client → POST /oauth/register
           ← client_id, client_secret (dynamic registration)

MCP Client → GET /oauth/authorize?client_id=...&code_challenge=...&state=...&redirect_uri=...
           → Not authenticated → 302 redirect to frontend login with return_to
           → Authenticated → show authorization confirmation
           → User approves → issue authorization code
           ← Redirect to redirect_uri with ?code=...&state=...

MCP Client → POST /oauth/token (grant_type=authorization_code, code=..., code_verifier=...)
           ← { access_token, token_type, expires_in, refresh_token }

MCP Client → POST /mcp (with Authorization: Bearer <access_token>)
           ← MCP JSON-RPC responses
```

### Endpoints

#### GET /.well-known/oauth-authorization-server

OAuth 2.0 Authorization Server Metadata (RFC 8414). No auth required.

The `issuer` is the backend root URL (e.g. `https://app.example.com`). OAuth endpoints live at the server root alongside `/mcp`.

**Response:**
```json
{
  "issuer": "https://app.example.com",
  "authorization_endpoint": "https://app.example.com/oauth/authorize",
  "token_endpoint": "https://app.example.com/oauth/token",
  "registration_endpoint": "https://app.example.com/oauth/register",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["client_secret_post"],
  "scopes_supported": ["mcp"],
  "authorization_response_iss_parameter_supported": true
}
```

`authorization_response_iss_parameter_supported` declares RFC 9207 support: every authorization response, success *and* error, carries an `iss` parameter naming the issuer, so a client talking to several authorization servers can detect a mix-up attack before it sends the code to a token endpoint. Advertising the capability obliges the server to always send it; a client that sees the flag and no `iss` must reject the response.

`registration_endpoint` stays advertised: DCR is deprecated as of `2026-07-28` in favor of Client ID Metadata Documents, but it is how every MCP client in the field registers today and the deprecation window is at least 12 months. CIMD (`client_id_metadata_document_supported`) is **not** implemented and deliberately not advertised, see [Not yet adopted](#not-yet-adopted-from-2026-07-28).

#### GET /.well-known/oauth-protected-resource/mcp

Protected Resource Metadata for MCP-aware OAuth clients. No auth required.

The path-specific URL is derived per [RFC 9728 §3.1](https://datatracker.ietf.org/doc/html/rfc9728#section-3.1) for the `/mcp` resource: `{root}/mcp` → `{root}/.well-known/oauth-protected-resource/mcp`. The root `/.well-known/oauth-protected-resource` URL is intentionally **not** served, it would be the PRM for a resource at `/`, which Everruns does not expose. Real MCP providers (e.g. Atlassian) only serve the path-specific URL, so OSS canonicalises on the same shape.

Everruns advertises `/mcp` as the protected resource and points clients at the same OAuth issuer and token endpoints used by the rest of the MCP flow.

### 401 WWW-Authenticate Header (RFC 9728)

Per [RFC 9728 §5.1](https://datatracker.ietf.org/doc/html/rfc9728#section-5.1) and the MCP 2025-06-18 auth spec, unauthenticated requests to `/mcp` return `401 Unauthorized` with a `WWW-Authenticate` header pointing at the protected-resource metadata document:

```
WWW-Authenticate: Bearer realm="mcp", resource_metadata="https://app.example.com/.well-known/oauth-protected-resource/mcp"
```

The `resource_metadata` URL is derived from the configured API base URL (stripping `/api` or `$API_PREFIX`) and uses the path-specific PRM URL for the `/mcp` resource. Implementation: `crates/server/src/api/mcp_endpoint/mod.rs` attaches a tower layer to the MCP router that injects the header on 401 responses and leaves other status codes untouched.

#### POST /oauth/register

Dynamic Client Registration (RFC 7591). No auth required.

**Request:**
```json
{
  "client_name": "Claude Desktop",
  "redirect_uris": ["http://localhost:12345/callback"],
  "application_type": "native"
}
```

`application_type` (OIDC, optional) is accepted as `native` or `web`; anything else is rejected. A client that declares `web` may not register an `http://` loopback callback, it has no local process to receive one. Unlike OIDC, an **omitted** `application_type` is treated as unstated rather than defaulting to `web`: every MCP client in the field today omits it and registers a loopback URI, so defaulting would reject all of them.

**Response:** `201 Created`
```json
{
  "client_id": "mcp_client_...",
  "client_secret": "mcp_secret_...",
  "client_name": "Claude Desktop",
  "redirect_uris": ["http://localhost:12345/callback"]
}
```

**Redirect URI policy.** Each `redirect_uris[*]` must parse as an absolute URL with no fragment and use one of the allowed schemes:

- `https://` to any host.
- `http://` only when the host is loopback: `localhost`, an IPv4 in `127.0.0.0/8`, or `[::1]`.

All other schemes, including `javascript:`, `data:`, `file:`, `vbscript:`, custom app schemes, protocol-relative `//host/...`, and unparseable/relative URIs, are rejected with `400 invalid_redirect_uri`. The same check is enforced again at `GET /oauth/authorize` and `POST /oauth/authorize` confirmation as defense in depth, so a previously registered unsafe URI cannot become an open-redirect target. See `crates/server/src/auth/mcp_oauth.rs::validate_redirect_uri` for the canonical policy.

#### GET /oauth/authorize

Authorization endpoint. Redirects to login when no session cookie is present.

**Query Parameters:**
- `client_id` (required)
- `redirect_uri` (required, must match registered URI)
- `response_type=code` (required)
- `code_challenge` (required, S256)
- `code_challenge_method=S256` (required)
- `state` (required)
- `scope=mcp` (optional, defaults to `mcp`)

**Flow:**
1. Check for valid session cookie
2. Not authenticated → 302 redirect to `{FRONTEND_URL}/login?return_to=/oauth/authorize?...`
3. User logs in → browser navigates back to `/oauth/authorize?...` (now with cookie)
4. Authenticated → validate client_id and redirect_uri, then render a confirmation page
5. User approves → generate authorization code
6. Redirect to `redirect_uri?code=...&state=...`

The confirmation page is rendered by the backend because `/oauth/*` routes are
root-level OAuth endpoints, not frontend application routes. It shows the
registered client name and client ID, signed-in user, redirect URI, requested
scope, and the concrete MCP access being granted. It deliberately shows no
organization: MCP OAuth tokens are user-scoped (the access token carries no
`org_id`) and resolve org per request, so naming a single org on the consent
page would misrepresent the grant. Canceling redirects back to the registered
redirect URI with `error=access_denied` and the original `state`.

Authorization codes: random 32-byte hex, 5-minute TTL, one-time use, stored with PKCE challenge.

#### POST /oauth/token

Token exchange. No cookie/session auth, uses client credentials.

**Request (authorization_code grant):**
```
grant_type=authorization_code
code=<authorization_code>
client_id=<client_id>
client_secret=<client_secret>
redirect_uri=<redirect_uri>
code_verifier=<pkce_verifier>
```

**Request (refresh_token grant):**
```
grant_type=refresh_token
refresh_token=<refresh_token>
client_id=<client_id>
client_secret=<client_secret>
```

**Response:**
```json
{
  "access_token": "<jwt>",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "<opaque>"
}
```

`expires_in` mirrors the actual JWT lifetime (`AUTH_JWT_ACCESS_TOKEN_LIFETIME`,
default 900 seconds). It is not a separate MCP-specific constant.

### Access Tokens

MCP OAuth access tokens are JWTs signed with `AUTH_JWT_SECRET`, but minted distinctly from browser/session/API tokens via `JwtService::generate_mcp_access_token()`. They are **resource-bound** (RFC 8707): `token_type="mcp_access"` and `aud` is the canonical `/mcp` resource URL.

```json
{
  "sub": "<user_id>",
  "email": "<email>",
  "name": "<name>",
  "roles": ["user"],
  "token_type": "mcp_access",
  "aud": "https://app.example.com/mcp",
  "exp": 1711234567,
  "iat": 1711230967
}
```

Audience binding (TM-MCP-006 / EVE-596) keeps the `/mcp` and `/api/*` surfaces isolated, preventing an OAuth confused-deputy:

- The general `/api/*` path (`AuthUser` → `AuthBackend::validate_token` → `JwtService::validate_access_token`) **rejects** `mcp_access` tokens, a token authorized only for `/mcp` cannot act as a full user token on the REST API.
- The `/mcp` endpoint uses a separate `McpAuthUser`/`McpResolvedOrg` extractor (`AuthBackend::validate_mcp_token` → `JwtService::validate_mcp_access_token`) that **accepts only** `mcp_access` tokens bound to the exact `/mcp` resource, and rejects regular session/access tokens and cookie sessions. Personal access tokens (`evr_pat_`) remain accepted on `/mcp` as intentional programmatic credentials.

Acting-as-user is preserved (claims still carry the user's identity and roles); only the unbounded-resource scope is removed. The access-token lifetime is the configured JWT access-token lifetime; MCP refresh tokens are opaque, stored hashed, and rotated by the OAuth flow (rotation re-mints an `mcp_access` token with the same audience).

### Database Schema

The OAuth tables, `oauth_clients`, `oauth_authorization_codes`, `oauth_refresh_tokens`, are defined
in [`crates/server/migrations/009_v0.8.8.sql`](../../crates/server/migrations/009_v0.8.8.sql).

The shapes that carry security intent: secrets and codes are stored **hashed**, never in plaintext;
authorization codes are single-use (`consumed`) and carry the PKCE challenge and method; and both
codes and refresh tokens expire. Authorization codes and refresh tokens cascade on user deletion, so
revoking a user revokes their grants.

### Route Mounting

MCP OAuth routes are mounted in `BuiltinAuthBackend::auth_routes()` alongside CLI auth routes:

```rust
fn auth_routes(&self) -> Option<Router> {
    let auth_routes = routes::routes(self.clone());
    let cli_routes = cli_auth_routes(cli_state);
    let mcp_oauth_routes = mcp_oauth::mcp_oauth_routes(mcp_oauth_state);
    Some(auth_routes.merge(cli_routes).merge(mcp_oauth_routes))
}
```

The well-known endpoint is also merged, the API prefix is handled by constructing the full URL in the metadata response.

### External Auth Backend (PropelAuth)

Works automatically because:
1. `POST /oauth/register`, no auth needed, backend not involved
2. `GET /oauth/authorize`, resolves user from cookie, redirects to login if needed
3. `POST /oauth/token`, validates code + PKCE, mints a resource-bound `mcp_access` JWT via `JwtService::generate_mcp_access_token()`. Backend not involved.
4. Token validation, the `/mcp` endpoint calls `AuthBackend::validate_mcp_token(token, resource)`, NOT the general `validate_token()`. External backends that issue their own MCP tokens must override `validate_mcp_token` to enforce the same audience binding (the OSS default fails closed); the mint + validate split is exposed on `JwtService`/`AuthBackend` for this purpose (TM-MCP-006).

External backends only need to ensure their login page honors the shared `return_to` query parameter (see [Login Page Contract](../security/authentication.md#login-page-contract) in the authentication spec). `return_to` is the single public auth-resume parameter across app, MCP OAuth, and CLI flows, there is no separate `redirect_to`.

## Security

| Threat | Mitigation |
|--------|------------|
| OAuth code interception | PKCE mandatory (S256). Codes hashed (SHA-256), 5-min TTL, one-time use. |
| Client secret leak | Stored hashed. Shown only at registration. |
| Open redirect | Redirect URIs must exactly match pre-registered values. |
| CSRF | State parameter required and validated by client. |
| Refresh token theft | Stored hashed, 30-day TTL, rotation on use. |
| MCP token confused-deputy | Access tokens are resource-bound (`token_type="mcp_access"`, `aud={root}/mcp`). `/api/*` rejects them and `/mcp` rejects regular session/access tokens (TM-MCP-006). |
| Unauthorized MCP access | Permission policies: `MCP_SERVER_VIEW`, `MCP_SERVER_MANAGE`, `MCP_SERVER_DANGEROUS`. |
| SSRF via MCP server URL | URLs validated on create/update and re-validated at execution time. Private IPs, loopback, link-local, and cloud metadata endpoints blocked. |
| API key exposure | Encrypted at rest (envelope encryption). Never returned in API responses. |
| Tool name collision | Double-underscore prefix scheme ensures unambiguous tool routing. |

See `knowledge/security/threat-model.md` for the full threat model.

## Multi-Organization Support

MCP clients authenticate via OAuth 2.1 Bearer tokens, which don't carry org context (unlike browser sessions that use the `everruns_org` cookie). Two mechanisms enable multi-org access:

### Tier 0 Tools

| Tool | Description |
|------|-------------|
| `me` | Returns current user profile and default organization context |
| `list_organizations` | Lists all orgs the user belongs to, with roles |

### Per-Call `organization_id` Override

All org-scoped tools (`agent_run`, `session_send_message`, `session_get_status`, `agent_get_card`, `discover`, `query`, `execute`) accept an optional `organization_id` parameter (format: `org_{32-hex}`). The `2026-07-28` Tasks methods (`tasks/get`, `tasks/update`, `tasks/cancel`) accept it as a request param too, resolved through the same path. When provided:

1. User membership is validated against the database (not stale JWT claims)
2. A `ResolvedOrg` is constructed for the target org
3. The tool executes in that org's context

When omitted, the default org is used (first org from the user's membership list).

### Design Decisions

1. **Stateless per-call override**: no session state needed. Each tool call independently targets an org.
2. **DB-validated membership**: JWT org claims may be stale; always check DB for fresh membership.
3. **`discover` accepts `organization_id` for consistency**: catalog search itself is effectively org-agnostic today, but the argument keeps org-scoped routing uniform across tools and leaves room for future org-specific catalog visibility.
4. **No `switch_organization` tool**: the MCP transport is stateless, so there is no server-side "current org" to switch. Tool descriptions tell clients to call `list_organizations` and pass `organization_id` directly on org-scoped calls.

## Error contract

`tools/call` failures use the MCP-standard error shape
(`isError: true` plus a `content[0].text` string) **and** carry a
typed `structuredContent` envelope so LLM toolcallers can branch on a
machine-readable code instead of regexing prose.

```json
{
  "result": {
    "content": [
      { "type": "text", "text": "Tool timed out after 30000ms" }
    ],
    "isError": true,
    "structuredContent": {
      "code": "tool_timeout",
      "category": "transient",
      "retryable": true,
      "message": "Tool timed out after 30000ms"
    }
  }
}
```

`retry_after_seconds`, `hint`, and `cause_chain` are all optional and
omitted from the wire shape when the server doesn't have a concrete
value to set. Today they are populated only on the cases the server
can reason about, for example, `tool_not_found` ships a fixed `hint`
pointing the caller at `tools/list`. As more tool implementations
construct `McpExecuteError` directly (instead of going through the
prose-string classifier), more occurrences will populate the
optional fields with case-specific values.

The legacy `content[0].text` channel is preserved verbatim for MCP
clients that predate the envelope, `structuredContent` is additive,
not a replacement.

### `McpErrorCode` (closed vocabulary)

The full enum, per-variant default `category`/`retryable`, and
human-readable meanings live in
[`crates/core/src/mcp_server.rs`](../../crates/core/src/mcp_server.rs)
(`pub enum McpErrorCode` near line 477). The defaults there are
authoritative; this spec captures the contract around them.

`category` and `retryable` are defaults, not invariants, every
occurrence may override them when the server has stronger
information. For example, an `internal` failure whose root cause is
known to be transient still ships `retryable: true`.

### Closed vocabulary rules

* Adding a new code is a spec change. Add the variant to
  `McpErrorCode` in `crates/core/src/mcp_server.rs` and update this
  spec's narrative if the new code changes the contract (new
  category, new retry semantics, new client guidance).
* SDKs deserialise any unrecognised code into `unknown` (serde
  `#[serde(other)]`); they must not crash on a value they don't know.
* The classifier `classify_mcp_execute_error` in the same module
  recovers a structured envelope from the legacy `Result<String,
  String>` error path. New tool implementations should construct
  `McpExecuteError` directly when they have a precise code; the
  classifier exists for forward-compat with the existing prose-string
  failures.

## Not yet adopted from 2026-07-28

Deliberate gaps, recorded so the next pass does not have to re-derive them:

- **Client ID Metadata Documents (CIMD).** The replacement for DCR. Implementing it means the authorization server fetches an arbitrary client-supplied HTTPS URL during `/oauth/authorize`, which is a new SSRF surface on an unauthenticated-ish path and needs a decision on whether any HTTPS origin may act as a client or only an allowlisted set. DCR keeps working throughout the deprecation window, so this is a scoped follow-up rather than a blocker.
- **MRTR on the server side.** Everruns' `/mcp` never answers `tools/call` with `resultType: "input_required"`; long-running work is expressed through the Tasks extension instead, which covers the same need for the tools we expose. (The *client* does handle receiving `input_required`, see [mcp-servers.md](mcp-servers.md).)
- **`subscriptions/listen`.** The consolidated notification stream. Task progress is polled through `tasks/get`, so nothing currently needs server push.
- **Roots, Sampling, Logging.** Deprecated in `2026-07-28` with a 12-month window. Everruns implements none of them, so there is nothing to remove.

## Implementation

See `crates/server/src/auth/mcp_oauth.rs` for the OAuth implementation.
See `crates/server/src/api/mcp_endpoint/caching.rs` for the cacheable-result decoration.
See `crates/server/src/api/mcp_endpoint/mod.rs` for the MCP endpoint and multi-org tool handlers.
See `crates/core/src/mcp_server.rs` for the `McpExecuteError` /
`McpErrorCode` / `McpErrorCategory` types backing the structured
error envelope.
