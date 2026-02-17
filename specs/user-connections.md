# User Connections Specification

## Abstract

User connections allow users to link external service accounts (GitHub, GitLab, etc.) to their Everruns account, independent of their authentication provider. A user who logged in via Google or email/password can still connect their GitHub account for repo access. Connection tokens are resolved lazily at tool execution time, enabling tools like `git_clone` to operate as the user without exposing credentials to the LLM.

## Requirements

### UserConnection

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Internal primary key |
| `user_id` | UUID | FK to users.id |
| `provider` | string | `github`, `gitlab`, `bitbucket` |
| `provider_user_id` | string | Provider's user/account ID |
| `provider_username` | string | Display name (e.g., `octocat`) |
| `access_token_encrypted` | bytes? | Encrypted OAuth token (NULL for GitHub App) |
| `refresh_token_encrypted` | bytes? | For providers that support refresh |
| `scopes` | string? | Granted permissions (e.g., `contents: read`) |
| `installation_id` | bigint? | GitHub App installation ID |
| `expires_at` | timestamp? | NULL = no expiry |
| `created_at` | timestamp | |
| `updated_at` | timestamp | |

No unique DB constraint on (user_id, provider). Uniqueness enforced in application code. This allows future multi-account support per provider if needed.

### Connection Method: GitHub App

A **GitHub App** (not an OAuth App) provides granular, per-repo permissions with short-lived tokens.

| Property | OAuth App (old) | GitHub App (current) |
|----------|----------------|---------------------|
| Token lifetime | Forever | 1 hour (installation access token) |
| Scope | `repo` = all repos | Per-repo (user selects which repos to install on) |
| Permissions | Coarse (`repo` = read+write+admin) | Granular (e.g., `contents: read` for clone only) |
| Revocation | User must manually revoke | Token expires automatically |
| Blast radius | All user's repos, forever | Selected repos only, 1 hour window |

**Flow:**
1. User clicks "Install" in Settings > Connections
2. `GET /v1/user/connections/github/authorize` → redirect to GitHub App installation page
3. User selects which repos to grant access to
4. GitHub redirects back → `GET /v1/user/connections/github/callback?installation_id=...&setup_action=install`
5. Server verifies installation via GitHub API (`GET /app/installations/{id}`)
6. Server stores `installation_id` in `user_connections`
7. Redirect to UI settings page with success

**Token minting:** At tool execution time, the server:
1. Reads `installation_id` from `user_connections`
2. Creates a JWT signed with the App's RSA private key (RS256, 10min TTL)
3. Calls `POST /app/installations/{id}/access_tokens` to mint a 1-hour token
4. Returns the token to the tool

**Permissions requested in App manifest:**
- Clone-only: `contents: read`
- Clone+push: `contents: write` (includes read)
- No admin, no webhooks, no issues, no PRs — unless needed

### Scoping

Connections are **user-scoped**. The installation represents the user's/org's grant of access. It's usable in any Everruns org the user belongs to.

**Visibility:** Connections are private to the user who created them. Other org members cannot list, view, or manage another user's connections via the API. The `GET /v1/user/connections` endpoint only returns the authenticated user's own connections.

**Token resolution:** Although connections are private, the lazy token resolver (`UserConnectionResolver`) can resolve tokens across org members for tool execution. When a tool needs a GitHub token, the resolver finds any org member who has connected GitHub. This means a user's installation may be used to serve tool requests in sessions belonging to the same org, but the connection itself remains invisible to other users.

### Lazy Token Resolution

Connection tokens are resolved lazily at tool execution time via `UserConnectionResolver`:

1. Tool (e.g. `git_clone`) requests token via `context.connection_resolver`
2. For GitHub App: resolver reads `installation_id`, mints a fresh 1h token via GitHub API
3. For legacy OAuth: resolver decrypts stored `access_token_encrypted`
4. If no connection exists, tool returns guidance: "connect GitHub in Settings > Connections"

Benefits:
- Tokens are always fresh (1h TTL, minted on demand)
- Sessions created before connecting still get tokens
- No long-lived secrets stored for GitHub
- Token scope is limited to installed repos only

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
      "scopes": "contents: write, metadata: read",
      "connected_at": "2026-02-15T10:00:00Z"
    }
  ]
}
```

#### DELETE /v1/user/connections/{provider}

Disconnect. Deletes the stored installation_id/token.

**Response:** `204 No Content`

#### GET /v1/user/connections/github/authorize

Start GitHub App installation flow. Redirects to `https://github.com/apps/{slug}/installations/new`.

#### GET /v1/user/connections/github/callback

GitHub App installation callback. Receives `installation_id`, verifies it, stores it, redirects to UI.

**Query params (from GitHub):**
- `installation_id`: The numeric installation ID
- `setup_action`: `install` or `update`
- `state`: CSRF token (if passed during redirect)

### Environment Variables

| Variable | Description |
|----------|-------------|
| `GITHUB_APP_ID` | GitHub App ID (numeric, from App settings page) |
| `GITHUB_APP_PRIVATE_KEY` | PEM-encoded RSA private key for JWT signing |
| `GITHUB_APP_SLUG` | App slug for installation URL (default: `everruns`) |
| `GITHUB_APP_SETUP_URL` | Post-install callback URL (default: `{AUTH_BASE_URL}{API_PREFIX}/v1/user/connections/github/callback`) |

### GitHub App Setup

Create a GitHub App:

1. Go to **GitHub → Settings → Developer settings → GitHub Apps → New GitHub App**
2. Configure:
   - **GitHub App name:** `Everruns`
   - **Homepage URL:** `https://everruns.com`
   - **Setup URL:** `https://<domain>/v1/user/connections/github/callback` (or `http://localhost:9300/api/v1/user/connections/github/callback` for local dev)
   - **Redirect on update:** checked
   - **Webhook:** uncheck "Active" (not needed)
   - **Repository permissions:** `Contents: Read & write` (or `Read-only` for clone-only)
   - **Where can this GitHub App be installed?** `Any account`
3. After creation:
   - Copy **App ID** → `GITHUB_APP_ID`
   - Note the **slug** from the URL → `GITHUB_APP_SLUG`
   - Generate a **private key** (.pem file) → `GITHUB_APP_PRIVATE_KEY`

### Security

- No long-lived tokens stored for GitHub (only `installation_id`, which is not a secret)
- Installation tokens are minted on demand with 1h TTL
- App private key stored as server-side env var, never in database
- Token never returned in API responses
- Token never appears in LLM message history or tool results
- Git tools use credential helper approach: writes a temporary script inside the sandbox that supplies the token, avoiding it appearing in command lines or process lists
- Revoking (uninstalling) the GitHub App immediately prevents new token minting
- Other providers (GitLab) still use encrypted OAuth tokens at rest

### Error Handling

| Scenario | Response |
|----------|----------|
| Already connected (app-level check) | Upsert replaces existing connection |
| Installation verification failed | 400: "GitHub App installation verification failed" |
| Connection not found on delete | 404 |
| Missing GitHub App config | 500 (logged): "GitHub App not configured" |

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| GitHub App instead of OAuth App? | Yes | Short-lived tokens (1h vs forever), per-repo permissions, smaller blast radius. See security comparison table above. |
| User-scoped not org-scoped? | Yes | Installation represents user's GitHub identity/access, not Everruns org's. Different org members have different repo access. Connections are private. |
| Store installation_id not token? | Yes | Tokens minted on demand via App private key. No long-lived secrets in database. |
| No DB unique constraint? | Yes | Enforce in app code. Allows future multi-account per provider without migration. |
| Auto-inject into sessions? | Yes | Seamless UX. User installs once, all sessions get access. |
| Credential helper not URL-embedded token? | Yes | Token never appears in command line, process list, or exec output. Safer. |
| Generic `user_connections` not `github_connections`? | Yes | Same pattern works for GitLab, Bitbucket. Provider-generic table + provider-specific connection code. |
| Backward-compatible resolver? | Yes | Resolver checks `installation_id` first (GitHub App path), falls back to `access_token_encrypted` (legacy OAuth path). Supports migration period. |
