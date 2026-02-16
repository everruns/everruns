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

### Connection Method: GitHub OAuth App



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

### Scoping

Connections are **user-scoped**. The token represents the user's identity on the external service. It's usable in any org the user belongs to.

**Visibility:** Connections are private to the user who created them. Other org members cannot list, view, or manage another user's connections via the API. The `GET /v1/user/connections` endpoint only returns the authenticated user's own connections.

**Token resolution:** Although connections are private, the lazy token resolver (`UserConnectionResolver`) can resolve tokens across org members for tool execution. When a tool needs a GitHub token, the resolver finds any org member who has connected GitHub. This means a user's token may be used to serve tool requests in sessions belonging to the same org, but the connection itself remains invisible to other users.

### Lazy Token Resolution

Connection tokens are resolved lazily at tool execution time via `UserConnectionResolver`:

1. Tool (e.g. `csb_git_clone`) requests token via `context.connection_resolver`
2. Resolver joins `sessions → org_members → user_connections` to find the token
3. Token is decrypted and returned directly to the tool
4. If no connection exists, tool returns guidance: "connect GitHub in Settings > Connections"

Benefits over eager session injection:
- Tokens are always fresh (reconnect mid-session works)
- Sessions created before connecting still get tokens
- No stale secrets in session storage

The token value never appears in tool arguments, tool results, or message history.

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
| `GITHUB_CONNECTION_REDIRECT_URI` | Callback URL (default: `{AUTH_BASE_URL}{API_PREFIX}/v1/user/connections/github/callback`) |

### GitHub OAuth App Setup

Create a **separate** GitHub OAuth App for connections (not the login OAuth App):

1. Go to **GitHub → Settings → Developer settings → OAuth Apps → New OAuth App**
2. Configure:
   - **Application name:** `Everruns`
   - **Homepage URL:** `https://everruns.com` (or `http://localhost:9300` for local dev)
   - **Authorization callback URL:** `http://localhost:9300/api/v1/user/connections/github/callback` (local) or `https://<domain>/v1/user/connections/github/callback` (production)
3. Copy **Client ID** → `GITHUB_CONNECTION_CLIENT_ID`
4. Generate **Client Secret** → `GITHUB_CONNECTION_CLIENT_SECRET`

The app requests `repo read:user` scopes (hardcoded in `GitHubConnectionService`).

### Security

- Tokens encrypted at rest via AES-256-GCM envelope encryption (same as MCP server API keys, session secrets)
- Token never returned in API responses
- Token never appears in LLM message history or tool results
- `csb_git_clone` tool uses credential helper approach: writes a temporary script inside the sandbox VM that supplies the token, avoiding it appearing in command lines or process lists
- Revoking a connection immediately prevents new sessions from getting the token; existing sessions retain their injected copy until session ends

### Error Handling

| Scenario | Response |
|----------|----------|
| Already connected (app-level check) | 409: "Already connected to {provider}. Disconnect first." |
| OAuth state mismatch | 400: "Invalid OAuth state" |
| Connection not found on delete | 404 |
| Missing connection config | 500 (logged): GitHub connection OAuth App not configured |

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Separate OAuth App? | Yes | Different scopes (repo vs profile). Separation of concerns. User sees distinct authorization prompts. |
| User-scoped not org-scoped? | Yes | Token represents user's GitHub identity, not org's. Different org members have different repo access. Connections are private — other org members cannot see them. |
| No DB unique constraint? | Yes | Enforce in app code. Allows future multi-account per provider without migration. |
| Auto-inject into sessions? | Yes | Seamless UX. User connects once, all sessions get access. |
| Credential helper not URL-embedded token? | Yes | Token never appears in command line, process list, or exec output. Safer. |
| Generic `user_connections` not `github_connections`? | Yes | Same pattern works for GitLab, Bitbucket. Provider-generic table + provider-specific OAuth code. |
