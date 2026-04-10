# Authentication Specification

## Abstract

This document defines the authentication system for Everruns, supporting flexible authentication modes for different deployment scenarios.

## Requirements

### Authentication Modes

Everruns supports four authentication modes:

1. **None** (`AUTH_MODE=none`): No authentication required. All requests use a well-known anonymous user (`ANONYMOUS_USER_ID = 00000000-0000-0000-0000-000000000001`). This is a real database user seeded at startup, so all code paths (org membership, API keys, etc.) work uniformly without special-casing. The anonymous user has admin role and belongs to the default organization. Suitable for local development.

2. **Admin** (`AUTH_MODE=admin`): Single admin user via environment variables. Suitable for local development with basic access control.

3. **Full** (`AUTH_MODE=full`): Complete authentication with user registration, OAuth, and API keys. Suitable for production deployments.

4. **External** (`AUTH_MODE=external`): Authentication managed by a third-party provider (e.g., PropelAuth, Auth0, Clerk). The external provider handles login, registration, and user management. Built-in password auth, OAuth, and signup are disabled. API key authentication is supported. Suitable for SaaS deployments with external identity providers.

### Authentication Methods

When authentication is enabled, the following methods are supported:

#### 1. Bearer Token (JWT)

```
Authorization: Bearer <access_token>
```

- Access tokens are short-lived (default: 15 minutes)
- Refresh tokens stored in database for revocation
- Tokens include user ID, email, name, and roles

#### 2. API Key

```
Authorization: Bearer <api_key>
```

- API keys prefixed with `evr_` for identification — the `evr_` prefix distinguishes API keys from JWTs within the `Bearer` scheme
- Auth scheme matching is case-insensitive per RFC 7235 (`bearer`, `BEARER`, `Bearer` all accepted)
- Full key shown only at creation, stored hashed (SHA-256)
- Supports scopes and expiration
- Used for programmatic access
- Legacy formats (`Authorization: evr_...`, `Authorization: ApiKey evr_...`) are still accepted for backward compatibility
- `metadata` JSONB column stores creation context: `source` (cli_login, web_ui, api), `hostname`, `os`, `ip`

#### 3. CLI Login (localhost OAuth callback)

Interactive CLI authentication via `everruns login`. Flow:

1. CLI calls `POST /v1/auth/cli/start` with `redirect_port`
2. Server creates a pending `cli_auth_sessions` row (state + exchange_code, 5-min TTL)
3. Server returns `auth_url` (login page with redirect to `/v1/auth/cli/callback?state=...`)
4. CLI opens browser and starts one-shot localhost HTTP server
5. User logs in, server calls `/v1/auth/cli/callback` which associates user and redirects to `localhost:{port}/callback?code=...`
6. CLI receives code, redirects browser to `/cli/login-success` (branded success page)
7. CLI calls `POST /v1/auth/cli/exchange` with code + hostname + os
8. Server creates API key with metadata, returns key + user + orgs
9. CLI prompts for org selection (if multiple), stores credentials in the platform config file (`~/.config/everruns/credentials.json` on Linux, `~/Library/Application Support/everruns/credentials.json` on macOS)

Endpoints: `POST /v1/auth/cli/start`, `GET /v1/auth/cli/callback`, `POST /v1/auth/cli/exchange`, `GET /cli/login-success`

See `crates/server/src/auth/cli_auth.rs` for implementation.

#### 4. Cookie-based Session

- `access_token` cookie with JWT
- `refresh_token` cookie (HTTP-only, secure)
- Suitable for web UI authentication

### OAuth Providers

When configured, supports OAuth2 with:

- **Google**: OpenID Connect with email profile
- **GitHub**: OAuth2 with user:email and read:user scopes

Account linking by email is supported (same email = same account).

### Password Requirements

- Minimum 8 characters
- Hashed with Argon2id (default parameters)

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AUTH_MODE` | Authentication mode: `none`, `admin`, `full`, `external` | `none` |
| `AUTH_BASE_URL` | Base URL for OAuth callbacks (include path prefix if behind reverse proxy) | `http://localhost:9300/api` |
| `AUTH_ADMIN_EMAIL` | Admin user email (admin mode) | - |
| `AUTH_ADMIN_PASSWORD` | Admin user password (admin mode) | - |
| `AUTH_JWT_SECRET` | JWT signing secret (required for admin/full) | - |
| `AUTH_JWT_ACCESS_TOKEN_LIFETIME` | Access token lifetime in seconds | `900` (15 min) |
| `AUTH_JWT_REFRESH_TOKEN_LIFETIME` | Refresh token lifetime in seconds | `2592000` (30 days) |
| `AUTH_DISABLE_PASSWORD` | Disable password authentication | `false` |
| `AUTH_DISABLE_SIGNUP` | Disable user registration | `false` |
| `AUTH_GOOGLE_CLIENT_ID` | Google OAuth client ID | - |
| `AUTH_GOOGLE_CLIENT_SECRET` | Google OAuth client secret | - |
| `AUTH_GOOGLE_REDIRECT_URI` | Google OAuth redirect URI | `{base_url}{api_prefix}/v1/auth/callback/google` |
| `AUTH_GOOGLE_ALLOWED_DOMAINS` | Comma-separated allowed email domains | - |
| `AUTH_GITHUB_CLIENT_ID` | GitHub OAuth client ID | - |
| `AUTH_GITHUB_CLIENT_SECRET` | GitHub OAuth client secret | - |
| `AUTH_GITHUB_REDIRECT_URI` | GitHub OAuth redirect URI | `{base_url}{api_prefix}/v1/auth/callback/github` |
| `GITHUB_CONNECTION_CLIENT_ID` | GitHub OAuth App client ID (for connections, NOT login) | - |
| `GITHUB_CONNECTION_CLIENT_SECRET` | GitHub OAuth App client secret (for connections) | - |
| `GITHUB_CONNECTION_REDIRECT_URI` | Callback URL for connections OAuth | `{AUTH_BASE_URL}/v1/user/connections/github/callback` |
| `CORS_ALLOWED_ORIGINS` | Comma-separated allowed CORS origins (only if cross-origin) | Not set |

### Database Schema

See `crates/server/migrations/001_base_schema.sql` for `users`, `api_keys`, and `refresh_tokens` table DDL.

#### Anonymous User (seeded at startup)

For `auth=none` mode, a well-known anonymous user is seeded via `crates/server/src/seed.rs`. Constants in `crates/core/src/organization.rs`: `ANONYMOUS_USER_ID`, `ANONYMOUS_USER_EMAIL`, `ANONYMOUS_USER_NAME`. The anonymous user has admin role and belongs to the default organization.

### Security Considerations

1. **JWT Secret**: Must be a secure random string (minimum 32 bytes recommended)
2. **Cookie Security**: Refresh tokens use HTTP-only, Secure (in production), SameSite=Strict cookies with `Path=/` so the cookie is sent through the UI's `/api` proxy
3. **API Key Storage**: Only hash is stored, full key shown once at creation
4. **Password Storage**: Argon2id with secure defaults
5. **Token Revocation**: Refresh tokens can be revoked by deleting from database
6. **Token Auto-Refresh**: The UI API client intercepts 401 responses and attempts a silent token refresh before failing (skips `/v1/auth/` endpoints to avoid loops; concurrent 401s are deduplicated into a single refresh request)

### Error Responses

```json
{
  "error": "Unauthorized"
}
```

- `401 Unauthorized`: Missing or invalid credentials
- `403 Forbidden`: Valid credentials but insufficient permissions

## UI Integration

### Configuration Discovery

The UI fetches authentication configuration from `GET /v1/auth/config` on startup (returns mode, password/OAuth/signup status).

### Conditional Rendering

Based on `mode`:

- **none**: Skip authentication entirely, show app directly
- **admin/full**: Require login before accessing protected routes
- **external**: Auth required (like full), but login/signup managed by external provider
- Protected routes fail closed while auth bootstrap state is unknown: if `/v1/auth/config` fails, or `/v1/auth/me` fails for reasons other than `401 Unauthorized`, the UI shows a blocking auth-unavailable state instead of rendering the app shell or redirecting to login

### UI Components

| Component | Path | Description |
|-----------|------|-------------|
| Login Page | `/login` | Email/password form + OAuth buttons |
| Register Page | `/register` | User registration (if `signup_enabled`) |
| User Menu | Sidebar | Profile, API keys link, logout |
| API Keys | `/settings#api-keys` | Create, list, delete API keys |

### Authentication Flow

1. App loads, fetches `/v1/auth/config`
2. If `mode === "none"`, render app without auth
3. Otherwise, check if user is authenticated via `/v1/auth/me`
4. If auth bootstrap fails (`/v1/auth/config` error or non-401 `/v1/auth/me` error), block protected routes with an auth-unavailable state
5. If `/v1/auth/me` returns `401 Unauthorized`, redirect to `/login?return_to=<current_path>` (preserving the user's location)
6. After login, cookies are set automatically (HTTP-only) and the user is redirected back to `return_to` (default: `/dashboard`)
7. Subsequent requests include cookies via `credentials: "include"`
8. On 401 response, the API client silently attempts `POST /v1/auth/refresh` (using the HttpOnly `refresh_token` cookie) and retries the request

### Token Refresh

The `POST /v1/auth/refresh` endpoint accepts the refresh token from two sources (checked in order):

1. **JSON body** `{ "refresh_token": "..." }` — for programmatic clients
2. **HttpOnly cookie** `refresh_token` — primary flow for browser clients (cookie is set at login with `Path=/`)

On success, the old refresh token is deleted (rotation) and a new token pair is returned (both in JSON body and Set-Cookie headers).

### OAuth Flow

1. User clicks OAuth button (e.g., "Continue with Google")
2. If a `return_to` URL is present, the UI persists it in `sessionStorage` (key: `everruns_return_to`) so it survives the redirect chain
3. Browser redirects to `GET /v1/auth/oauth/{provider}`
4. API redirects to provider's authorization page
5. After user authorizes, provider redirects to callback
6. API handles callback, sets cookies, redirects to `/`
7. The main layout checks `sessionStorage` for a pending `return_to` and redirects the user back

### Protected Routes

All routes under `/(main)/*` are protected:
- `/dashboard`
- `/agents`
- `/settings`

Auth pages under `/(auth)/*` are public:
- `/login`
- `/register`

### State Management

Authentication state is managed via:

1. **AuthProvider** - React Context providing auth state
2. **React Query** - Caching auth config and user info
3. **HTTP-only Cookies** - Secure token storage (managed by server)

### API Client Configuration

All requests go through `/api` prefix with `credentials: "include"` for cookie-based auth. The `/api` prefix is stripped by the proxy (Next.js in dev, reverse proxy in prod) before reaching the backend.

## Pluggable Authentication Backend

The auth system uses a trait-based pluggable backend so external auth providers (PropelAuth, Auth0, Keycloak) can be used without modifying OSS code. OSS ships `BuiltinAuthBackend` as the default.

### AuthBackend Trait

See `crates/server/src/auth/backend.rs` for the `AuthBackend` trait. Key methods: `validate_token()`, `validate_api_key()`, `auth_routes()`, `auth_config_response()`.

### BuiltinAuthBackend (OSS Default)

See `crates/server/src/auth/builtin.rs`. Wraps JWT + password + API key logic. Auth route handlers use `BuiltinAuthBackend` as axum state directly with `FromRef<BuiltinAuthBackend> for AuthState`.

### AuthState

`AuthState` holds `Arc<dyn AuthBackend>`. The `extract_auth_user()` middleware delegates to `backend.validate_token()` and `backend.validate_api_key()`. OSS convenience: `AuthState::builtin(config, db)`.

### External Identity Support

Migration `004_external_identity.sql` adds nullable `external_id` columns to `users` and `organizations` tables, mapping external provider IDs to internal IDs. OSS: unused (NULL). SaaS: populated by auth backend sync.

See `crates/server/src/storage/` for lookup/upsert methods: `get_user_by_external_id()`, `get_organization_by_external_id()`, `upsert_org_by_external_id()`, `ensure_membership()`.

### UI Context Exports

`AuthContext` and `AuthContextValue` are exported from `providers/auth-provider.tsx` so SaaS wrappers can provide custom auth context values without forking the component.
