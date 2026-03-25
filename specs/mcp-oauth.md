# MCP OAuth Specification

> Part of the [MCP spec family](mcp.md). This document covers OAuth 2.1 endpoints, dynamic client registration, PKCE, and token lifecycle for inbound MCP clients.

## Abstract

MCP OAuth enables third-party MCP clients to authenticate with Everruns using OAuth 2.1 with PKCE. This allows tools like Claude Desktop, Cursor, and other MCP-compatible clients to connect to Everruns as an MCP server with proper user authorization.

The module is **backend-agnostic** — it works identically with BuiltinAuthBackend (OSS) and external providers like PropelAuth (SaaS). It follows the same pattern as CLI auth: the authorize endpoint requires an authenticated user via the `AuthUser` extractor, delegating identity verification to whatever auth backend is configured.

## Design Decisions

1. **Backend-agnostic**: MCP OAuth sits alongside the auth backend, not inside it. No changes to `AuthBackend` trait.
2. **Same pattern as CLI auth**: The authorize endpoint requires `AuthUser` — works with any backend.
3. **OAuth 2.1 + PKCE**: Authorization code grant with mandatory PKCE (S256). No implicit grant.
4. **Dynamic client registration**: Per RFC 7591 — MCP clients register themselves at runtime.
5. **MCP OAuth tokens are JWTs**: Signed with the same `AUTH_JWT_SECRET`, distinguished by `token_type: "mcp_access"` claim.
6. **Scoped to org**: Authorization grants are scoped to a specific organization.

## OAuth 2.1 Flow

```
MCP Client → GET /.well-known/oauth-authorization-server
           ← Server metadata (endpoints, PKCE support)

MCP Client → POST /v1/oauth/register
           ← client_id, client_secret (dynamic registration)

MCP Client → GET /v1/oauth/authorize?client_id=...&code_challenge=...&state=...&redirect_uri=...
           → User authenticates (via whatever auth backend is configured)
           → User consents
           ← Redirect to redirect_uri with ?code=...&state=...

MCP Client → POST /v1/oauth/token (grant_type=authorization_code, code=..., code_verifier=...)
           ← { access_token, token_type, expires_in, refresh_token }

MCP Client → POST /mcp (with Authorization: Bearer <access_token>)
           ← MCP JSON-RPC responses
```

## Endpoints

### GET /.well-known/oauth-authorization-server

OAuth 2.0 Authorization Server Metadata (RFC 8414). No auth required.

**Response:**
```json
{
  "issuer": "https://app.example.com",
  "authorization_endpoint": "https://app.example.com/api/v1/oauth/authorize",
  "token_endpoint": "https://app.example.com/api/v1/oauth/token",
  "registration_endpoint": "https://app.example.com/api/v1/oauth/register",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["client_secret_post"],
  "scopes_supported": ["mcp"]
}
```

### POST /v1/oauth/register

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

### GET /v1/oauth/authorize

Authorization endpoint. **Requires authenticated user** (via `AuthUser` extractor).

**Query Parameters:**
- `client_id` (required)
- `redirect_uri` (required, must match registered URI)
- `response_type=code` (required)
- `code_challenge` (required, S256)
- `code_challenge_method=S256` (required)
- `state` (required)
- `scope=mcp` (optional, defaults to `mcp`)

**Flow:**
1. `AuthUser` extractor fires — delegates to configured auth backend
2. Not authenticated → redirect to login with `return_to` back to authorize URL
3. Authenticated → validate client_id, redirect_uri, generate authorization code
4. Redirect to `redirect_uri?code=...&state=...`

Authorization codes: random 32-byte hex, 5-minute TTL, one-time use, stored with PKCE challenge.

### POST /v1/oauth/token

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
  "expires_in": 3600,
  "refresh_token": "<opaque>"
}
```

## MCP OAuth Access Tokens

JWTs signed with `AUTH_JWT_SECRET`, with distinct claims:

```json
{
  "sub": "<user_id>",
  "email": "<email>",
  "name": "<name>",
  "roles": ["user"],
  "token_type": "mcp_access",
  "client_id": "<oauth_client_id>",
  "scope": "mcp",
  "exp": 1711234567,
  "iat": 1711230967
}
```

The `token_type: "mcp_access"` claim distinguishes these from regular access tokens. They are validated by the existing `BuiltinAuthBackend.validate_token()` path — the JWT service validates the signature and expiry, and the `token_type` field is informational (not enforced at the middleware level, since MCP tokens grant the same access as regular sessions).

## Database Schema

### oauth_clients

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

### oauth_authorization_codes

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

### oauth_refresh_tokens

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

## Integration

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
1. `POST /v1/oauth/register` — no auth needed, backend not involved
2. `GET /v1/oauth/authorize` — uses `AuthUser` extractor, which delegates to any backend
3. `POST /v1/oauth/token` — validates code + PKCE, creates JWT. Backend not involved.
4. Token validation — standard JWT validation via `validate_token()`

External backends only need to ensure their login page handles `redirect_to` query parameter (already required for CLI auth).

## Security Considerations

1. **PKCE mandatory**: S256 only. No plain challenge method.
2. **Authorization codes**: Stored hashed (SHA-256), 5-min TTL, one-time use.
3. **Client secrets**: Stored hashed (SHA-256), shown only at registration.
4. **Redirect URI validation**: Exact match against registered URIs.
5. **Refresh tokens**: Stored hashed, 30-day TTL, rotation on use.
6. **No open redirects**: Redirect URIs must be pre-registered.
7. **CSRF protection**: State parameter required and validated by client.

## Implementation

See `crates/server/src/auth/mcp_oauth.rs` for the implementation.
