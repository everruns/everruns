---
title: Authentication Configuration
description: Configuring and managing authentication for Everruns
---

This runbook covers configuring and managing authentication for Everruns.

## Authentication Modes

### 1. No Authentication (Development)

Use for local development when authentication isn't needed:

```bash
export AUTH_MODE=none
```

All requests will be allowed with full admin access.

### 2. Admin Mode (Simple Development)

Use for local development with basic access control:

```bash
export AUTH_MODE=admin
export AUTH_ADMIN_EMAIL=admin@example.com
export AUTH_ADMIN_PASSWORD=your-secure-password
export AUTH_JWT_SECRET=$(openssl rand -hex 32)
```

Only the admin user can authenticate.

### 3. Full Authentication (Production)

Use for production deployments:

```bash
export AUTH_MODE=full
export AUTH_BASE_URL=https://your-domain.com
export AUTH_JWT_SECRET=$(openssl rand -hex 32)

# Optional: Configure OAuth
export AUTH_GOOGLE_CLIENT_ID=your-google-client-id
export AUTH_GOOGLE_CLIENT_SECRET=your-google-client-secret
export AUTH_GITHUB_CLIENT_ID=your-github-client-id
export AUTH_GITHUB_CLIENT_SECRET=your-github-client-secret
```

## Environment Variables Reference

### Core Settings

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_MODE` | No | `none`, `admin`, or `full` (default: `none`) |
| `AUTH_BASE_URL` | For OAuth | Base URL for callbacks (default: `http://localhost:9000`) |
| `AUTH_JWT_SECRET` | For admin/full | JWT signing secret (min 32 chars recommended) |

### Admin Mode Settings

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_ADMIN_EMAIL` | Yes (admin mode) | Admin user email |
| `AUTH_ADMIN_PASSWORD` | Yes (admin mode) | Admin user password |

### JWT Settings

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_JWT_ACCESS_TOKEN_LIFETIME` | No | Access token lifetime in seconds (default: 900) |
| `AUTH_JWT_REFRESH_TOKEN_LIFETIME` | No | Refresh token lifetime in seconds (default: 2592000) |

### Feature Toggles

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_DISABLE_PASSWORD` | No | Set to `true` to disable password login |
| `AUTH_DISABLE_SIGNUP` | No | Set to `true` to disable user registration |

### Google OAuth

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_GOOGLE_CLIENT_ID` | For Google OAuth | Google OAuth client ID |
| `AUTH_GOOGLE_CLIENT_SECRET` | For Google OAuth | Google OAuth client secret |
| `AUTH_GOOGLE_REDIRECT_URI` | No | Custom redirect URI |
| `AUTH_GOOGLE_ALLOWED_DOMAINS` | No | Comma-separated allowed email domains |

### GitHub OAuth

| Variable | Required | Description |
|----------|----------|-------------|
| `AUTH_GITHUB_CLIENT_ID` | For GitHub OAuth | GitHub OAuth client ID |
| `AUTH_GITHUB_CLIENT_SECRET` | For GitHub OAuth | GitHub OAuth client secret |
| `AUTH_GITHUB_REDIRECT_URI` | No | Custom redirect URI |

## Common Tasks

### Generate JWT Secret

```bash
# Using OpenSSL
openssl rand -hex 32

# Using Python
python3 -c "import secrets; print(secrets.token_hex(32))"
```

### Verify Authentication is Working

```bash
# Check auth config endpoint
curl http://localhost:9000/v1/auth/config

# Should return:
# {"mode":"none","password_auth_enabled":false,"oauth_providers":[],"signup_enabled":false}

# For admin mode:
curl -X POST http://localhost:9000/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"your-password"}'
```

### Create API Key

```bash
# Login first to get access token
TOKEN=$(curl -s -X POST http://localhost:9000/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"user@example.com","password":"password"}' | jq -r '.access_token')

# Create API key
curl -X POST http://localhost:9000/v1/auth/api-keys \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"my-api-key"}'

# Response includes full key - save it, it's shown only once
```

### Revoke API Key

```bash
curl -X DELETE http://localhost:9000/v1/auth/api-keys/{key_id} \
  -H "Authorization: Bearer $TOKEN"
```

### Force Logout User

Delete their refresh tokens from database:

```sql
DELETE FROM refresh_tokens WHERE user_id = 'user-uuid-here';
```

## Troubleshooting

### "Authentication required" when AUTH_MODE=none

- Verify `AUTH_MODE` environment variable is set correctly
- Restart the server after changing environment variables

### JWT Validation Fails

- Ensure `AUTH_JWT_SECRET` hasn't changed
- Check token hasn't expired
- Verify the token is for the correct environment

### OAuth Redirect Fails

- Verify `AUTH_BASE_URL` matches the OAuth app configuration
- Check that redirect URI in provider matches `{AUTH_BASE_URL}{API_PREFIX}/v1/auth/callback/{provider}`
- If using `API_PREFIX`, ensure it's included in OAuth provider redirect URI configuration
- Ensure client ID and secret are correct

### Password Login Returns Unauthorized

- In admin mode: check `AUTH_ADMIN_EMAIL` and `AUTH_ADMIN_PASSWORD` match
- In full mode with password disabled: check `AUTH_DISABLE_PASSWORD` isn't set
- Verify user exists and password is correct

## Database Migration

Authentication requires migration `003_authentication.sql`:

```bash
# Run migrations
sqlx migrate run

# Or via the dev script
./scripts/dev.sh migrate
```

## Health Check

The `/health` endpoint shows current auth mode:

```bash
curl http://localhost:9000/health
# {"status":"ok","version":"0.2.0","auth_mode":"None"}
```

## Security Best Practices

1. **Never commit secrets**: Use environment variables or secret management
2. **Rotate JWT secret**: Change `AUTH_JWT_SECRET` periodically (invalidates all tokens)
3. **Use HTTPS**: Always use HTTPS in production for OAuth callbacks
4. **Limit OAuth domains**: Use `AUTH_GOOGLE_ALLOWED_DOMAINS` to restrict access
5. **Monitor API key usage**: Track `last_used_at` for suspicious activity
6. **Set token expiration**: Use shorter `AUTH_JWT_ACCESS_TOKEN_LIFETIME` for higher security
