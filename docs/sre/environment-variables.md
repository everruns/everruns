---
title: Environment Variables
description: Configuration environment variables for Everruns
---

## DEV_MODE

Enable development mode with in-memory storage. No PostgreSQL required.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `false` |

**Example:**

```bash
# Start in dev mode (no database required)
DEV_MODE=true ./target/debug/everruns-control-plane

# Or with 1
DEV_MODE=1 ./target/debug/everruns-control-plane
```

**Notes:**
- When enabled, uses in-memory storage instead of PostgreSQL
- All data is lost when the server stops
- gRPC server and worker communication are disabled
- Stale task reclamation is disabled
- Useful for quick local development and testing
- Not suitable for production or multi-instance deployments

**Limitations in dev mode:**
- No persistence (data is lost on restart)
- No worker support (all execution happens in-process)
- No distributed tracing of worker activities
- Single-instance only

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

LLM provider API keys (OpenAI, Anthropic, Azure OpenAI) are primarily stored encrypted in the database and managed via the Settings > Providers UI.

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

### Default API Keys (Development Convenience)

For development, you can set default API keys via environment variables. These are used as fallbacks when providers don't have keys configured in the database.

| Variable | Description |
|----------|-------------|
| `DEFAULT_OPENAI_API_KEY` | Fallback API key for OpenAI providers |
| `DEFAULT_ANTHROPIC_API_KEY` | Fallback API key for Anthropic providers |

**Example:**

```bash
# Set in .env or environment
DEFAULT_OPENAI_API_KEY=sk-...
DEFAULT_ANTHROPIC_API_KEY=sk-ant-...
```

**Notes:**
- Database-stored keys always take priority over environment variables
- These are intended for development convenience, not production use
- The `./scripts/dev.sh start-all` command automatically sets these from `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` if present

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

## OpenTelemetry Configuration

Everruns supports distributed tracing via OpenTelemetry with OTLP export. Traces follow the [Gen-AI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) for LLM operations.

### OTEL_EXPORTER_OTLP_ENDPOINT

OTLP endpoint for trace export (e.g., Jaeger, Grafana Tempo, or any OTLP-compatible backend).

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (tracing disabled) |

**Example:**

```bash
# For local Jaeger
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# For production Tempo
OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo.monitoring:4317
```

**Notes:**
- When set, traces are exported via OTLP/gRPC
- For local development, Jaeger is included in `docker-compose.yml`
- Without this variable, only console logging is enabled

### OTEL_SERVICE_NAME

Service name for traces.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `everruns-control-plane` (API), `everruns-worker` (Worker) |

**Example:**

```bash
OTEL_SERVICE_NAME=everruns-prod-api
```

### OTEL_SERVICE_VERSION

Service version for traces.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Cargo package version |

### OTEL_ENVIRONMENT

Deployment environment label.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set |

**Example:**

```bash
OTEL_ENVIRONMENT=production
```

### OTEL_RECORD_CONTENT

Enable recording of LLM input/output content in traces. **Warning:** May contain sensitive data.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `false` |

**Example:**

```bash
# Enable content recording (use with caution)
OTEL_RECORD_CONTENT=true
```

**Notes:**
- When enabled, `gen_ai.input.messages` and `gen_ai.output.messages` are recorded
- Disabled by default for privacy and data size concerns
- Only enable in development or when debugging specific issues

## Local Development with Jaeger

The `harness/docker-compose.yml` includes Jaeger for local trace visualization:

```bash
# Start all services including Jaeger
./scripts/dev.sh start

# Set OTLP endpoint for API and Worker
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# View traces at
open http://localhost:16686
```

### Jaeger Ports

| Port | Description |
|------|-------------|
| 4317 | OTLP gRPC receiver |
| 4318 | OTLP HTTP receiver |
| 16686 | Jaeger UI |

### Gen-AI Trace Attributes

LLM calls include the following OpenTelemetry attributes:

| Attribute | Description |
|-----------|-------------|
| `gen_ai.operation.name` | Operation type (`chat`, `embeddings`) |
| `gen_ai.provider.name` | Provider (`openai`, `anthropic`) |
| `gen_ai.request.model` | Requested model name |
| `gen_ai.request.max_tokens` | Maximum tokens requested |
| `gen_ai.request.temperature` | Sampling temperature |
| `gen_ai.usage.input_tokens` | Prompt tokens used |
| `gen_ai.usage.output_tokens` | Completion tokens used |
| `gen_ai.response.finish_reasons` | Why generation stopped |
| `server.address` | API endpoint URL |
