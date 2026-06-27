# MCP (Model Context Protocol) Specification

## Abstract

Everruns exposes an MCP server endpoint (`/mcp`) so external MCP clients — Claude Desktop, Cursor, VS Code, etc. — can interact with Everruns agents and tools over the [Model Context Protocol](https://spec.modelcontextprotocol.io). Authentication uses the same mechanisms as other API routes (`ResolvedOrg` extractor — API keys, JWT/cookie sessions), including OAuth 2.1-issued JWTs with mandatory PKCE for MCP client registration flows.

The endpoint is always mounted (no feature flag); access is gated per request by authentication and per-call org resolution, not by deployment- or org-level opt-in.

Routing is intentionally split:

- REST API lives under `/api/*`
- MCP OAuth lives at `/oauth/*` (authorize, token, register)
- MCP JSON-RPC lives at `/mcp`
- OAuth discovery metadata lives at `/.well-known/oauth-authorization-server`
- Protected-resource metadata lives at `/.well-known/oauth-protected-resource/mcp` (RFC 9728 §3.1 path-derived for the `/mcp` resource)

Everruns also acts as an **MCP client** (connecting to remote MCP servers). That side is covered in [`specs/mcp-servers.md`](mcp-servers.md), with the in-process runtime path (shared `everruns-mcp` crate, HTTP + optional stdio transport, pluggable auth) in [`specs/runtime-mcp.md`](runtime-mcp.md).

## Protocol

JSON-RPC 2.0 over Streamable HTTP (`POST /mcp` with JSON-RPC body).

Everruns supports MCP protocol versions `2025-06-18` and `2025-03-26`.

- `initialize` negotiates `protocolVersion` from the request body. When the client omits a version, Everruns falls back to `2025-03-26`.
- All non-`initialize` requests may send `MCP-Protocol-Version`. When omitted, Everruns falls back to `2025-03-26`.
- Requests that send an unsupported `MCP-Protocol-Version` are rejected with HTTP `400 Bad Request` and a JSON-RPC error payload.

### Supported Methods

| Method | Description |
|--------|-------------|
| `initialize` | Initialize MCP session and negotiate capabilities |
| `ping` | Health check / keep-alive |
| `tools/list` | Discover available tools |
| `tools/call` | Execute a tool |
| `resources/list` | Discover static Everruns resources |
| `resources/read` | Read a static Everruns resource by URI |

### Entity Cards

Under negotiated protocol `2025-06-18`, Everruns exposes per-entity card
tools (`agent_get_card`, future `session_get_card`, …) that return an MCP
`resource` content block of MIME type `text/html` at the `ui://` scheme
along with a one-line text summary. MCP-Apps-aware hosts render the HTML
in a sandboxed iframe; hosts that ignore embedded resources still see the
summary. See [`specs/mcp-cards.md`](mcp-cards.md) for the standard,
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

## Architecture

### Crate Map

| Crate | Module | Responsibility |
|-------|--------|----------------|
| `everruns-server` | `auth/mcp_oauth.rs` | OAuth 2.1 endpoints (register, authorize, token) |
| `everruns-server` | `api/mcp_servers.rs` | HTTP CRUD routes for MCP server management |
| `everruns-server` | `services/mcp_server.rs` | Business logic, tool caching, permission policies |
| `everruns-server` | `storage/repositories/mcp_servers.rs` | PostgreSQL persistence |
| `everruns-core` | `mcp_server.rs` | Domain types (`McpServer`, `McpToolDefinition`, content types), tool name helpers |
| `everruns-core` | `capabilities/mcp.rs` | Virtual capability wrapper, tool-to-definition conversion |
| `everruns-worker` | `mcp_executor.rs` | HTTP tool execution, SSE parsing, image extraction |
| `everruns-internal-protocol` | `worker.proto` | gRPC for tool context and server resolution |

## OAuth 2.1 Authentication

External MCP clients authenticate via OAuth 2.1 with mandatory PKCE (S256). The module is **backend-agnostic** — it works identically with BuiltinAuthBackend (OSS) and external providers like PropelAuth (SaaS). It follows the same pattern as CLI auth: the authorize endpoint requires an authenticated user via the `AuthUser` extractor, delegating identity verification to whatever auth backend is configured.

### Design Decisions

1. **Backend-agnostic**: MCP OAuth sits alongside the auth backend, not inside it. No changes to `AuthBackend` trait.
2. **Same pattern as CLI auth**: The authorize endpoint requires `AuthUser` — works with any backend.
3. **OAuth 2.1 + PKCE**: Authorization code grant with mandatory PKCE (S256). No implicit grant.
4. **Dynamic client registration**: Per RFC 7591 — MCP clients register themselves at runtime.
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
  "scopes_supported": ["mcp"]
}
```

#### GET /.well-known/oauth-protected-resource/mcp

Protected Resource Metadata for MCP-aware OAuth clients. No auth required.

The path-specific URL is derived per [RFC 9728 §3.1](https://datatracker.ietf.org/doc/html/rfc9728#section-3.1) for the `/mcp` resource: `{root}/mcp` → `{root}/.well-known/oauth-protected-resource/mcp`. The root `/.well-known/oauth-protected-resource` URL is intentionally **not** served — it would be the PRM for a resource at `/`, which Everruns does not expose. Real MCP providers (e.g. Atlassian) only serve the path-specific URL, so OSS canonicalises on the same shape.

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
  "redirect_uris": ["http://localhost:12345/callback"]
}
```

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

All other schemes — including `javascript:`, `data:`, `file:`, `vbscript:`, custom app schemes, protocol-relative `//host/...`, and unparseable/relative URIs — are rejected with `400 invalid_redirect_uri`. The same check is enforced again at `GET /oauth/authorize` and `POST /oauth/authorize` confirmation as defense in depth, so a previously registered unsafe URI cannot become an open-redirect target. See `crates/server/src/auth/mcp_oauth.rs::validate_redirect_uri` for the canonical policy.

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

Token exchange. No cookie/session auth — uses client credentials.

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

- The general `/api/*` path (`AuthUser` → `AuthBackend::validate_token` → `JwtService::validate_access_token`) **rejects** `mcp_access` tokens — a token authorized only for `/mcp` cannot act as a full user token on the REST API.
- The `/mcp` endpoint uses a separate `McpAuthUser`/`McpResolvedOrg` extractor (`AuthBackend::validate_mcp_token` → `JwtService::validate_mcp_access_token`) that **accepts only** `mcp_access` tokens bound to the exact `/mcp` resource, and rejects regular session/access tokens and cookie sessions. Personal access tokens (`evr_pat_`) remain accepted on `/mcp` as intentional programmatic credentials.

Acting-as-user is preserved (claims still carry the user's identity and roles); only the unbounded-resource scope is removed. The access-token lifetime is the configured JWT access-token lifetime; MCP refresh tokens are opaque, stored hashed, and rotated by the OAuth flow (rotation re-mints an `mcp_access` token with the same audience).

### Database Schema

#### oauth_clients

```sql
CREATE TABLE oauth_clients (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_id TEXT NOT NULL UNIQUE,
    client_secret_hash TEXT NOT NULL,
    client_name TEXT NOT NULL,
    redirect_uris JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### oauth_authorization_codes

```sql
CREATE TABLE oauth_authorization_codes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    code_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL DEFAULT 'S256',
    scope TEXT NOT NULL DEFAULT 'mcp',
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### oauth_refresh_tokens

```sql
CREATE TABLE oauth_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    token_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'mcp',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

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

The well-known endpoint is also merged — the API prefix is handled by constructing the full URL in the metadata response.

### External Auth Backend (PropelAuth)

Works automatically because:
1. `POST /oauth/register` — no auth needed, backend not involved
2. `GET /oauth/authorize` — resolves user from cookie, redirects to login if needed
3. `POST /oauth/token` — validates code + PKCE, mints a resource-bound `mcp_access` JWT via `JwtService::generate_mcp_access_token()`. Backend not involved.
4. Token validation — the `/mcp` endpoint calls `AuthBackend::validate_mcp_token(token, resource)`, NOT the general `validate_token()`. External backends that issue their own MCP tokens must override `validate_mcp_token` to enforce the same audience binding (the OSS default fails closed); the mint + validate split is exposed on `JwtService`/`AuthBackend` for this purpose (TM-MCP-006).

External backends only need to ensure their login page honors the shared `return_to` query parameter (see [Login Page Contract](authentication.md#login-page-contract) in the authentication spec). `return_to` is the single public auth-resume parameter across app, MCP OAuth, and CLI flows — there is no separate `redirect_to`.

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

See `specs/threat-model.md` for the full threat model.

## Multi-Organization Support

MCP clients authenticate via OAuth 2.1 Bearer tokens, which don't carry org context (unlike browser sessions that use the `everruns_org` cookie). Two mechanisms enable multi-org access:

### Tier 0 Tools

| Tool | Description |
|------|-------------|
| `me` | Returns current user profile and default organization context |
| `list_organizations` | Lists all orgs the user belongs to, with roles |

### Per-Call `organization_id` Override

All org-scoped tools (`agent_run`, `session_send_message`, `session_get_status`, `agent_get_card`, `discover`, `query`, `execute`) accept an optional `organization_id` parameter (format: `org_{32-hex}`). When provided:

1. User membership is validated against the database (not stale JWT claims)
2. A `ResolvedOrg` is constructed for the target org
3. The tool executes in that org's context

When omitted, the default org is used (first org from the user's membership list).

### Design Decisions

1. **Stateless per-call override** — no session state needed. Each tool call independently targets an org.
2. **DB-validated membership** — JWT org claims may be stale; always check DB for fresh membership.
3. **`discover` accepts `organization_id` for consistency** — catalog search itself is effectively org-agnostic today, but the argument keeps org-scoped routing uniform across tools and leaves room for future org-specific catalog visibility.
4. **No `switch_organization` tool** — the MCP transport is stateless, so there is no server-side "current org" to switch. Tool descriptions tell clients to call `list_organizations` and pass `organization_id` directly on org-scoped calls.

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
can reason about — for example, `tool_not_found` ships a fixed `hint`
pointing the caller at `tools/list`. As more tool implementations
construct `McpExecuteError` directly (instead of going through the
prose-string classifier), more occurrences will populate the
optional fields with case-specific values.

The legacy `content[0].text` channel is preserved verbatim for MCP
clients that predate the envelope — `structuredContent` is additive,
not a replacement.

### `McpErrorCode` (closed vocabulary)

The full enum, per-variant default `category`/`retryable`, and
human-readable meanings live in
[`crates/core/src/mcp_server.rs`](../crates/core/src/mcp_server.rs)
(`pub enum McpErrorCode` near line 477). The defaults there are
authoritative; this spec captures the contract around them.

`category` and `retryable` are defaults, not invariants — every
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

## Implementation

See `crates/server/src/auth/mcp_oauth.rs` for the OAuth implementation.
See `crates/server/src/api/mcp_endpoint/mod.rs` for the MCP endpoint and multi-org tool handlers.
See `crates/core/src/mcp_server.rs` for the `McpExecuteError` /
`McpErrorCode` / `McpErrorCategory` types backing the structured
error envelope.
