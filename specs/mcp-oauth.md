# MCP OAuth Specification

## Abstract

This document defines the OAuth 2.1 authentication system for MCP servers in Everruns. MCP OAuth enables users to authenticate with external MCP servers that require OAuth authorization, following the MCP Authorization specification (2025-06-18) which is based on OAuth 2.1, PKCE, and RFC 9728 (Protected Resource Metadata).

## Requirements

### Overview

MCP servers can require OAuth authentication to access their tools. When a user attempts to use tools from an OAuth-protected MCP server, they must first authorize access. The system handles:

1. **Discovery**: Automatic discovery of OAuth configuration via Protected Resource Metadata (RFC 9728)
2. **Authorization**: OAuth 2.1 authorization flow with PKCE
3. **Token Storage**: Per-user encrypted token storage (access + refresh tokens)
4. **Token Refresh**: Automatic token refresh when tokens expire
5. **Multi-user**: Each user has their own tokens for each MCP server

### MCP OAuth Configuration

MCP servers can be configured with OAuth settings. OAuth configuration is discovered automatically or configured manually.

| Field | Type | Description |
|-------|------|-------------|
| `auth_type` | enum | Authentication type: `none`, `api_key`, `oauth` |
| `oauth_authorization_url` | string? | Authorization endpoint URL |
| `oauth_token_url` | string? | Token endpoint URL |
| `oauth_client_id` | string? | OAuth client ID (public) |
| `oauth_client_secret_encrypted` | bytes? | Encrypted OAuth client secret |
| `oauth_scopes` | string[]? | Required OAuth scopes |
| `oauth_resource_metadata_url` | string? | RFC 9728 metadata URL (auto-discovered) |

### McpServerAuthType Enum

| Value | Description |
|-------|-------------|
| `none` | No authentication required |
| `api_key` | Static API key authentication (existing behavior) |
| `oauth` | OAuth 2.1 authentication |

### McpUserToken

Per-user OAuth tokens for MCP servers.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `mcp_server_id` | UUID | Reference to MCP server |
| `user_id` | UUID | Reference to user |
| `access_token_encrypted` | bytes | Encrypted OAuth access token |
| `refresh_token_encrypted` | bytes? | Encrypted OAuth refresh token |
| `token_type` | string | Token type (typically "Bearer") |
| `scope` | string? | Granted scopes |
| `expires_at` | timestamp? | Access token expiration time |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Constraints:**
- Unique index on `(mcp_server_id, user_id)` - one token per user per server

### McpOAuthState

Temporary storage for OAuth authorization state (PKCE, CSRF protection).

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | State identifier (used as `state` parameter) |
| `mcp_server_id` | UUID | Reference to MCP server |
| `user_id` | UUID | Reference to user |
| `code_verifier` | string | PKCE code verifier |
| `redirect_uri` | string | Redirect URI for callback |
| `return_url` | string? | UI URL to return to after auth |
| `created_at` | timestamp | Creation time |
| `expires_at` | timestamp | State expiration (10 minutes) |

### API Endpoints

#### GET /v1/mcp-servers/{server_id}/oauth/status

Get OAuth authorization status for the current user.

**Response:** `200 OK`
```json
{
  "auth_type": "oauth",
  "authorized": true,
  "expires_at": "2025-01-18T12:00:00Z",
  "scopes": ["repo", "user"]
}
```

Or if not authorized:
```json
{
  "auth_type": "oauth",
  "authorized": false,
  "authorization_url": "https://api.everruns.com/v1/mcp-servers/{server_id}/oauth/authorize"
}
```

#### GET /v1/mcp-servers/{server_id}/oauth/authorize

Initiate OAuth authorization flow. Redirects user to OAuth provider.

**Query Parameters:**
- `return_url` (optional): URL to redirect back to after authorization

**Response:** `302 Found` - Redirects to OAuth provider's authorization endpoint

#### GET /v1/oauth/callback

OAuth callback handler. Exchanges authorization code for tokens.

**Query Parameters:**
- `code`: Authorization code from OAuth provider
- `state`: State parameter for CSRF protection

**Response:** `302 Found` - Redirects to `return_url` or default UI page

#### DELETE /v1/mcp-servers/{server_id}/oauth/token

Revoke OAuth authorization for the current user.

**Response:** `204 No Content`

#### PATCH /v1/mcp-servers/{server_id}

Update MCP server OAuth configuration.

**Request Body:**
```json
{
  "auth_type": "oauth",
  "oauth_client_id": "client-id",
  "oauth_client_secret": "client-secret",
  "oauth_scopes": ["repo", "user"]
}
```

### OAuth Flow

#### 1. Protected Resource Metadata Discovery (RFC 9728)

When an MCP server is configured with OAuth or returns 401, discover OAuth endpoints:

```http
GET /.well-known/oauth-protected-resource HTTP/1.1
Host: api.githubcopilot.com
```

**Response:**
```json
{
  "resource": "https://api.githubcopilot.com/mcp/",
  "authorization_servers": ["https://github.com/login/oauth"]
}
```

#### 2. Authorization Server Metadata Discovery (RFC 8414)

Fetch OAuth endpoints from authorization server:

```http
GET /.well-known/oauth-authorization-server HTTP/1.1
Host: github.com
```

**Response:**
```json
{
  "issuer": "https://github.com",
  "authorization_endpoint": "https://github.com/login/oauth/authorize",
  "token_endpoint": "https://github.com/login/oauth/access_token",
  "scopes_supported": ["repo", "user", "read:org"]
}
```

#### 3. Authorization Request with PKCE

Generate PKCE parameters and redirect to authorization endpoint:

```
code_verifier = random_string(43-128 chars)
code_challenge = BASE64URL(SHA256(code_verifier))
```

**Authorization URL:**
```
https://github.com/login/oauth/authorize?
  client_id=<client_id>&
  redirect_uri=https://api.everruns.com/v1/oauth/callback&
  response_type=code&
  scope=repo%20user&
  state=<random_state>&
  code_challenge=<code_challenge>&
  code_challenge_method=S256&
  resource=https%3A%2F%2Fapi.githubcopilot.com%2Fmcp%2F
```

#### 4. Token Exchange

Exchange authorization code for tokens:

```http
POST https://github.com/login/oauth/access_token HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Accept: application/json

grant_type=authorization_code&
client_id=<client_id>&
client_secret=<client_secret>&
code=<authorization_code>&
redirect_uri=https://api.everruns.com/v1/oauth/callback&
code_verifier=<code_verifier>&
resource=https%3A%2F%2Fapi.githubcopilot.com%2Fmcp%2F
```

**Response:**
```json
{
  "access_token": "gho_xxxxx",
  "token_type": "Bearer",
  "expires_in": 3600,
  "refresh_token": "ghr_xxxxx",
  "scope": "repo user"
}
```

#### 5. Token Refresh

Automatically refresh tokens before expiration:

```http
POST https://github.com/login/oauth/access_token HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Accept: application/json

grant_type=refresh_token&
client_id=<client_id>&
client_secret=<client_secret>&
refresh_token=<refresh_token>&
resource=https%3A%2F%2Fapi.githubcopilot.com%2Fmcp%2F
```

### Token Usage in MCP Requests

When executing MCP tools, the worker:

1. Checks if the MCP server requires OAuth (`auth_type = oauth`)
2. Retrieves the user's access token from `mcp_user_tokens`
3. If token is expired/missing, returns an error requesting authorization
4. Includes token in MCP request:

```http
POST https://api.githubcopilot.com/mcp/ HTTP/1.1
Authorization: Bearer gho_xxxxx
Content-Type: application/json

{"jsonrpc": "2.0", "method": "tools/call", ...}
```

### Error Handling

| Error | HTTP Status | Description |
|-------|-------------|-------------|
| `oauth_required` | 401 | User needs to authorize with OAuth |
| `token_expired` | 401 | Token expired and refresh failed |
| `invalid_grant` | 401 | Authorization code or refresh token invalid |
| `oauth_config_missing` | 500 | MCP server OAuth not configured |

**Error Response:**
```json
{
  "error": "oauth_required",
  "message": "OAuth authorization required for this MCP server",
  "authorization_url": "https://api.everruns.com/v1/mcp-servers/{server_id}/oauth/authorize"
}
```

### Security Considerations

1. **Token Encryption**: All tokens encrypted at rest using envelope encryption (see `specs/encryption.md`)
2. **PKCE**: Always use S256 code challenge method
3. **State Parameter**: Random, unguessable state for CSRF protection
4. **Short-lived State**: OAuth state expires after 10 minutes
5. **Secure Redirect**: Validate redirect URIs match configured values
6. **Token Refresh**: Prefer refresh tokens over long-lived access tokens
7. **User Isolation**: Each user has separate tokens, never shared

### Database Schema

#### mcp_user_tokens table

```sql
CREATE TABLE mcp_user_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token_encrypted BYTEA NOT NULL,
    refresh_token_encrypted BYTEA,
    token_type TEXT NOT NULL DEFAULT 'Bearer',
    scope TEXT,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(mcp_server_id, user_id)
);
CREATE INDEX idx_mcp_user_tokens_user_id ON mcp_user_tokens(user_id);
CREATE INDEX idx_mcp_user_tokens_expires_at ON mcp_user_tokens(expires_at);
```

#### mcp_oauth_states table

```sql
CREATE TABLE mcp_oauth_states (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_verifier TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    return_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_mcp_oauth_states_expires_at ON mcp_oauth_states(expires_at);
```

#### mcp_servers table additions

```sql
ALTER TABLE mcp_servers ADD COLUMN auth_type TEXT NOT NULL DEFAULT 'none';
ALTER TABLE mcp_servers ADD COLUMN oauth_authorization_url TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_token_url TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_client_id TEXT;
ALTER TABLE mcp_servers ADD COLUMN oauth_client_secret_encrypted BYTEA;
ALTER TABLE mcp_servers ADD COLUMN oauth_scopes JSONB;
ALTER TABLE mcp_servers ADD COLUMN oauth_resource_metadata_url TEXT;
```

### Encrypted Columns Registry Update

Add to encryption.rs:
```rust
EncryptedColumn {
    table: "mcp_servers",
    column: "oauth_client_secret_encrypted",
    id_column: "id",
},
EncryptedColumn {
    table: "mcp_user_tokens",
    column: "access_token_encrypted",
    id_column: "id",
},
EncryptedColumn {
    table: "mcp_user_tokens",
    column: "refresh_token_encrypted",
    id_column: "id",
},
```

### UI Integration

#### MCP Server Configuration

In the MCP server edit form, show OAuth configuration:

1. **Auth Type Selector**: None / API Key / OAuth
2. **OAuth Fields** (when OAuth selected):
   - Client ID (required)
   - Client Secret (password field)
   - Scopes (comma-separated or tags)
   - Auto-discover checkbox (fetches from metadata)

#### User Authorization Flow

When user accesses an OAuth-protected MCP server:

1. **Status Badge**: Show "Connected" or "Authorization Required" on MCP server card
2. **Connect Button**: Opens authorization flow in popup/new tab
3. **After Authorization**: Badge updates to "Connected", tools become available

#### Session Tool Execution

When a tool call fails with `oauth_required`:

1. Show inline message: "This tool requires authorization"
2. Provide "Authorize" button that opens OAuth flow
3. After authorization, retry the tool call automatically

### GitHub Copilot MCP Server Example

Configuration for GitHub Copilot MCP server:

```json
{
  "name": "github_copilot",
  "description": "GitHub Copilot MCP Server",
  "url": "https://api.githubcopilot.com/mcp/",
  "transport_type": "http",
  "auth_type": "oauth",
  "oauth_client_id": "Iv1.xxxxx",
  "oauth_scopes": ["repo", "user", "read:org"]
}
```

The OAuth endpoints are discovered automatically from:
- `https://api.githubcopilot.com/.well-known/oauth-protected-resource`

### Implementation Crates

| Crate | Responsibility |
|-------|----------------|
| `everruns-core` | OAuth types (`McpServerAuthType`, token models) |
| `everruns-control-plane` | OAuth routes, token storage, metadata discovery |
| `everruns-worker` | Token retrieval, authenticated MCP requests |

### gRPC Protocol Updates

Add to `worker.proto`:

```protobuf
message McpServerInfo {
  // ... existing fields ...
  string auth_type = 10;  // "none", "api_key", "oauth"
}

message GetMcpUserTokenRequest {
  string mcp_server_id = 1;
  string user_id = 2;
}

message GetMcpUserTokenResponse {
  string access_token = 1;
  string token_type = 2;
  google.protobuf.Timestamp expires_at = 3;
}
```

### Testing Strategy

1. **Unit Tests**:
   - PKCE code generation and verification
   - Token encryption/decryption
   - OAuth URL construction
   - State parameter validation

2. **Integration Tests**:
   - Full OAuth flow with mock OAuth server
   - Token refresh flow
   - MCP tool execution with OAuth

3. **E2E Tests** (Playwright):
   - OAuth authorization UI flow
   - MCP server configuration with OAuth
   - Tool execution requiring authorization
