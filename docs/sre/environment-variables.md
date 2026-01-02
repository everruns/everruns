---
title: Environment Variables
description: Configuration environment variables for Everruns
---

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

## LLM Provider API Keys

LLM provider API keys (OpenAI, Anthropic, Azure OpenAI) are **not** configured via environment variables. Instead, they are stored encrypted in the database and managed via the Settings > Providers UI.

| Property | Value |
|----------|-------|
| **Storage** | Database (encrypted with AES-256-GCM) |
| **Configuration** | Settings > Providers UI or `/v1/llm-providers` API |
| **Supported Providers** | OpenAI, Anthropic, Azure OpenAI |

**Required for encryption:**

The `SECRETS_ENCRYPTION_KEY` environment variable must be set for the API and Worker to encrypt/decrypt API keys:

```bash
# Generate a new key
python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"

# Set in environment
SECRETS_ENCRYPTION_KEY=kek-v1:your-generated-key-here
```

**Note:** Environment variables like `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` are NOT used by the system. All API keys must be configured through the database.

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

## Worker Configuration

### GRPC_ADDRESS

Address of the control-plane gRPC server for worker communication.

| Property | Value |
|----------|-------|
| **Required** | No (worker only) |
| **Default** | `127.0.0.1:9001` |

**Example:**

```bash
GRPC_ADDRESS=127.0.0.1:9001
```

**Notes:**
- Workers communicate with the control-plane via gRPC for all database operations
- The control-plane exposes both HTTP (port 9000) and gRPC (port 9001) interfaces
- Workers are stateless and do not connect directly to the database

### TEMPORAL_ADDRESS

Address of the Temporal server.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `localhost:7233` |

**Example:**

```bash
TEMPORAL_ADDRESS=localhost:7233
```

### TEMPORAL_NAMESPACE

Temporal namespace for workflows.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `default` |

### TEMPORAL_TASK_QUEUE

Temporal task queue name for agent run workflows.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `everruns-agent-runs` |
