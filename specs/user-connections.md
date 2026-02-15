# User Connections Specification

## Abstract

User connections allow users to link external service accounts (GitHub, GitLab, etc.) to their Everruns account, independent of their authentication provider. A user who logged in via Google or email/password can still connect their GitHub account for repo access. Connected tokens are auto-injected into agent sessions, enabling tools like `csb_git_clone` to operate as the user without exposing credentials to the LLM.

## Requirements

### UserConnection

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Internal primary key |
| `user_id` | UUID | FK to users.id |
| `provider` | string | `github`, `gitlab`, `bitbucket` |
| `provider_user_id` | string | Provider's user ID |
| `provider_username` | string | Display name (e.g., `octocat`) |
| `access_token_encrypted` | bytes | Encrypted with envelope encryption |
| `refresh_token_encrypted` | bytes? | For providers that support refresh |
| `scopes` | string? | Granted scopes (e.g., `repo,read:user`) |
| `expires_at` | timestamp? | NULL = no expiry (GitHub OAuth App tokens) |
| `created_at` | timestamp | |
| `updated_at` | timestamp | |

No unique DB constraint on (user_id, provider). Uniqueness enforced in application code. This allows future multi-account support per provider if needed.

### Connection Methods

#### 1. GitHub OAuth App (Browser Flow)

A **separate** GitHub OAuth App from the login OAuth App. Different client_id/secret, different scopes.

- Login OAuth App scopes: `user:email read:user` (profile only)
- Connection OAuth App scopes: `repo read:user` (repo access + profile for username)

**Flow:**
1. User clicks "Connect GitHub" in settings UI
2. `GET /v1/user/connections/github/authorize` → redirect to GitHub
3. GitHub redirects back → `GET /v1/user/connections/github/callback?code=...&state=...`
4. Server exchanges code for access_token
5. Server calls GitHub API to get username/user_id
6. Server encrypts token, upserts into `user_connections`
7. Redirect to UI settings page with success

**Token behavior:** GitHub OAuth App tokens do not expire by default. No refresh flow needed for v1.

#### 2. Manual PAT (Fallback)

For GitHub Enterprise, fine-grained tokens, or users who prefer explicit control:

```
POST /v1/user/connections
{ "provider": "github", "access_token": "ghp_xxxxxxxxxxxx" }
```

Server validates the token against `GET https://api.github.com/user`, extracts username/id, stores encrypted.

### Scoping

Connections are **user-scoped**. The token represents the user's identity on the external service. It's usable in any org the user belongs to.

### Session Injection

When a session starts and the `codesandbox` capability is active:

1. Resolve user from auth context (session creator)
2. Query `user_connections` for user's `github` connection
3. If found, decrypt token and set as session secret: `GITHUB_TOKEN`
4. Also inject `GITHUB_USERNAME` and `GITHUB_EMAIL` as session KV values

Agent tools read `GITHUB_TOKEN` from session secrets internally. The token value never appears in tool arguments, tool results, or message history.

### API Endpoints

#### GET /v1/user/connections

List user's connected accounts. Token values never returned.

**Response:**
```json
{
  "data": [
    {
      "provider": "github",
      "provider_username": "octocat",
      "scopes": "repo,read:user",
      "connected_at": "2026-02-15T10:00:00Z"
    }
  ]
}
```

#### POST /v1/user/connections

Add connection via manual token.

**Request:**
```json
{
  "provider": "github",
  "access_token": "ghp_xxxxxxxxxxxx"
}
```

**Response:** `201 Created` with connection info (no token in response).

#### DELETE /v1/user/connections/{provider}

Disconnect. Deletes the stored token.

**Response:** `204 No Content`

#### GET /v1/user/connections/github/authorize

Start GitHub OAuth flow. Returns redirect to GitHub.

**Query params:**
- `redirect_uri` (optional): Where to redirect after callback (default: UI settings page)

#### GET /v1/user/connections/github/callback

GitHub OAuth callback. Exchanges code, stores token, redirects to UI.

### Environment Variables

| Variable | Description |
|----------|-------------|
| `GITHUB_CONNECTION_CLIENT_ID` | GitHub OAuth App client ID (for connections, NOT login) |
| `GITHUB_CONNECTION_CLIENT_SECRET` | GitHub OAuth App client secret |
| `GITHUB_CONNECTION_REDIRECT_URI` | Callback URL (default: `{AUTH_BASE_URL}/v1/user/connections/github/callback`) |

### Security

- Tokens encrypted at rest via AES-256-GCM envelope encryption (same as MCP server API keys, session secrets)
- Token never returned in API responses
- Token never appears in LLM message history or tool results
- `csb_git_clone` tool uses credential helper approach: writes a temporary script inside the sandbox VM that supplies the token, avoiding it appearing in command lines or process lists
- Revoking a connection immediately prevents new sessions from getting the token; existing sessions retain their injected copy until session ends

### Error Handling

| Scenario | Response |
|----------|----------|
| Provider not supported | 400: "Unsupported provider: {name}" |
| Token validation fails | 400: "Invalid token: could not authenticate with {provider}" |
| Already connected (app-level check) | 409: "Already connected to {provider}. Disconnect first." |
| OAuth state mismatch | 400: "Invalid OAuth state" |
| Connection not found on delete | 404 |
| Missing connection config | 500 (logged): GitHub connection OAuth App not configured |

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Separate OAuth App? | Yes | Different scopes (repo vs profile). Separation of concerns. User sees distinct authorization prompts. |
| User-scoped not org-scoped? | Yes | Token represents user's GitHub identity, not org's. Different org members have different repo access. |
| No DB unique constraint? | Yes | Enforce in app code. Allows future multi-account per provider without migration. |
| Auto-inject into sessions? | Yes | Seamless UX. User connects once, all sessions get access. |
| Credential helper not URL-embedded token? | Yes | Token never appears in command line, process list, or exec output. Safer. |
| Generic `user_connections` not `github_connections`? | Yes | Same pattern works for GitLab, Bitbucket. Provider-generic table + provider-specific OAuth code. |
