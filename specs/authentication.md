# Authentication Specification

## Abstract

This document defines the authentication system for Everruns, supporting flexible authentication modes for different deployment scenarios.

## Requirements

### Authentication Modes

Everruns supports three authentication modes:

1. **None** (`AUTH_MODE=none`): No authentication required. All requests are allowed with anonymous user context. Suitable for local development.

2. **Admin** (`AUTH_MODE=admin`): Single admin user via environment variables. Suitable for local development with basic access control.

3. **Full** (`AUTH_MODE=full`): Complete authentication with user registration, OAuth, and API keys. Suitable for production deployments.

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
Authorization: <api_key>
Authorization: ApiKey <api_key>
```

- API keys prefixed with `evr_` for identification
- Full key shown only at creation, stored hashed (SHA-256)
- Supports scopes and expiration
- Used for programmatic access

#### 3. Cookie-based Session

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
| `AUTH_MODE` | Authentication mode: `none`, `admin`, `full` | `none` |
| `AUTH_BASE_URL` | Base URL for OAuth callbacks | `http://localhost:9000` |
| `AUTH_ADMIN_EMAIL` | Admin user email (admin mode) | - |
| `AUTH_ADMIN_PASSWORD` | Admin user password (admin mode) | - |
| `AUTH_JWT_SECRET` | JWT signing secret (required for admin/full) | - |
| `AUTH_JWT_ACCESS_TOKEN_LIFETIME` | Access token lifetime in seconds | `900` (15 min) |
| `AUTH_JWT_REFRESH_TOKEN_LIFETIME` | Refresh token lifetime in seconds | `2592000` (30 days) |
| `AUTH_DISABLE_PASSWORD` | Disable password authentication | `false` |
| `AUTH_DISABLE_SIGNUP` | Disable user registration | `false` |
| `AUTH_GOOGLE_CLIENT_ID` | Google OAuth client ID | - |
| `AUTH_GOOGLE_CLIENT_SECRET` | Google OAuth client secret | - |
| `AUTH_GOOGLE_REDIRECT_URI` | Google OAuth redirect URI | `{base_url}/v1/auth/callback/google` |
| `AUTH_GOOGLE_ALLOWED_DOMAINS` | Comma-separated allowed email domains | - |
| `AUTH_GITHUB_CLIENT_ID` | GitHub OAuth client ID | - |
| `AUTH_GITHUB_CLIENT_SECRET` | GitHub OAuth client secret | - |
| `AUTH_GITHUB_REDIRECT_URI` | GitHub OAuth redirect URI | `{base_url}/v1/auth/callback/github` |
| `CORS_ALLOWED_ORIGINS` | Comma-separated allowed CORS origins (only if cross-origin) | Not set |

### Database Schema

#### users table additions

```sql
ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN email_verified BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN auth_provider TEXT;  -- 'google', 'github', or NULL for password
ALTER TABLE users ADD COLUMN auth_provider_id TEXT;
```

#### api_keys table

```sql
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    scopes JSONB NOT NULL DEFAULT '["*"]'::jsonb,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### refresh_tokens table

```sql
CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Security Considerations

1. **JWT Secret**: Must be a secure random string (minimum 32 bytes recommended)
2. **Cookie Security**: Refresh tokens use HTTP-only, Secure (in production), SameSite=Lax cookies
3. **API Key Storage**: Only hash is stored, full key shown once at creation
4. **Password Storage**: Argon2id with secure defaults
5. **Token Revocation**: Refresh tokens can be revoked by deleting from database

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

The UI fetches authentication configuration from `GET /v1/auth/config` on startup:

```typescript
interface AuthConfigResponse {
  mode: "none" | "admin" | "full";
  password_auth_enabled: boolean;
  oauth_providers: string[];  // ["google", "github"]
  signup_enabled: boolean;
}
```

### Conditional Rendering

Based on `mode`:

- **none**: Skip authentication entirely, show app directly
- **admin/full**: Require login before accessing protected routes

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
4. If not authenticated, redirect to `/login`
5. After login, cookies are set automatically (HTTP-only)
6. Subsequent requests include cookies via `credentials: "include"`

### OAuth Flow

1. User clicks OAuth button (e.g., "Continue with Google")
2. Browser redirects to `GET /v1/auth/oauth/{provider}`
3. API redirects to provider's authorization page
4. After user authorizes, provider redirects to callback
5. API handles callback, sets cookies, redirects to `/`

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

```typescript
// All requests go through /api prefix and include credentials for cookie-based auth
fetch('/api/v1/agents', {
  credentials: "include",
  headers: { "Content-Type": "application/json" }
});
```

The `/api` prefix is stripped by the proxy (Next.js in dev, reverse proxy in prod) before reaching the backend.
