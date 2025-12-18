# Environment Variables

## API_PREFIX

Optional prefix for all API routes.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Empty (no prefix) |

**Example:**

```bash
# Routes at /api/v1/agents
API_PREFIX=/api
```

**Notes:**
- `/health`, `/swagger-ui`, and `/api-doc/openapi.json` are not affected by this prefix
- All API routes including auth (`/v1/auth/*`) are affected by this prefix
- OAuth callback URLs automatically include this prefix when using defaults
- Use when running behind a reverse proxy or API gateway that expects a path prefix

## CORS_ALLOWED_ORIGINS

Comma-separated list of allowed origins for cross-origin requests. Only needed when the UI is served from a different domain than the API.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (CORS disabled) |

**Example:**

```bash
# Allow requests from a different frontend origin
CORS_ALLOWED_ORIGINS=https://app.example.com

# Multiple origins
CORS_ALLOWED_ORIGINS=https://app.example.com,https://admin.example.com
```

**Notes:**
- Not needed for local development (Next.js proxy handles `/api/*` requests)
- Not needed in production if using a reverse proxy on the same domain
- If set, credentials are allowed (`Access-Control-Allow-Credentials: true`)
- Wildcard (`*`) is not supported when using credentials

## UI API Proxy Architecture

The UI makes all API requests to `/api/*` paths. These are handled differently in each environment:

**Local Development:**
- Next.js rewrites proxy `/api/*` to `http://localhost:9000/*`
- Example: `/api/v1/agents` → `http://localhost:9000/v1/agents`
- No CORS needed (same-origin)

**Production (recommended):**
- Configure your reverse proxy (nginx, Caddy, etc.) to route `/api/*` to the API server
- Strip the `/api` prefix when forwarding
- Example nginx config:
  ```nginx
  location /api/ {
    proxy_pass http://api-server:9000/;
  }
  ```
- No CORS needed (same-origin)

## Observability

### OBSERVABILITY_ENABLED

Global toggle for observability integrations.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Auto-detect (enabled if any backend configured) |

**Example:**

```bash
# Disable observability even if backends are configured
OBSERVABILITY_ENABLED=false
```

### LANGFUSE_PUBLIC_KEY

Langfuse public API key for authentication.

| Property | Value |
|----------|-------|
| **Required** | Yes (for Langfuse) |
| **Default** | Not set |

**Example:**

```bash
LANGFUSE_PUBLIC_KEY=pk-lf-abc123...
```

### LANGFUSE_SECRET_KEY

Langfuse secret API key for authentication.

| Property | Value |
|----------|-------|
| **Required** | Yes (for Langfuse) |
| **Default** | Not set |

**Example:**

```bash
LANGFUSE_SECRET_KEY=sk-lf-xyz789...
```

### LANGFUSE_HOST

Langfuse API endpoint URL.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `https://cloud.langfuse.com` |

**Example:**

```bash
# EU region
LANGFUSE_HOST=https://cloud.langfuse.com

# US region
LANGFUSE_HOST=https://us.cloud.langfuse.com

# Self-hosted
LANGFUSE_HOST=https://langfuse.example.com
```

### LANGFUSE_RELEASE

Application release/version tag for tracing.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set |

**Example:**

```bash
LANGFUSE_RELEASE=v1.2.3
```

### LANGFUSE_FLUSH_INTERVAL_MS

Interval in milliseconds between batch flushes.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `5000` |

**Example:**

```bash
LANGFUSE_FLUSH_INTERVAL_MS=1000
```

### LANGFUSE_MAX_BATCH_SIZE

Maximum number of events per batch before forced flush.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `100` |

**Example:**

```bash
LANGFUSE_MAX_BATCH_SIZE=50
```

**Notes:**
- See [Observability Runbook](runbooks/observability.md) for full configuration guide
