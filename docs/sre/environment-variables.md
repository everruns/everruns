---
title: Environment Variables
description: Complete reference for all Everruns environment variables, including database connections, authentication, encryption, and development mode configuration.
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
DEV_MODE=true ./target/debug/everruns-server

# Or with 1
DEV_MODE=1 ./target/debug/everruns-server
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

## DEPLOYMENT_GRADE

Deployment environment grade. Controls which features and capabilities are available.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `prod` (or `dev` if `DEV_MODE=true`) |

**Valid values:**

| Grade | Description |
|-------|-------------|
| `dev` | Development - all experimental features enabled |
| `poc` | Proof of concept / demo environment |
| `preview` | Preview/staging environment |
| `prod` | Production - only stable features |

**Example:**

```bash
# Run in development mode with experimental features
DEPLOYMENT_GRADE=dev ./target/debug/everruns-server

# Production mode (default)
DEPLOYMENT_GRADE=prod ./target/debug/everruns-server
```

**Notes:**
- If not set, falls back to `DEV_MODE`: if `DEV_MODE=true`, uses `dev`; otherwise uses `prod`
- Experimental capabilities (e.g., Docker Container) are only available in `dev` grade
- Experimental seed agents (e.g., Python Coder) are only created in `dev` grade
- Use `dev` for local development and testing experimental features
- Use `prod` for production deployments

## API_PREFIX

Path prefix for REST API routes.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `/api` |

**Example:**

```bash
# Routes at /api/v1/agents
API_PREFIX=/api
```

**Notes:**
- `/health`, `/api-doc/openapi.json`, `/mcp`, `/.well-known/*`, `/oauth/*`, and `/cli/login-success` stay at the server root
- REST API routes including auth (`/v1/auth/*`) are mounted under this prefix
- OAuth callback URLs use `AUTH_BASE_URL`, which should already include the API prefix
- Override only if you need a non-`/api` REST prefix behind a reverse proxy or gateway

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
- Not needed for local development (Caddy reverse proxy keeps UI and backend on one origin)
- Not needed in production if using a reverse proxy on the same domain
- If set, credentials are allowed (`Access-Control-Allow-Credentials: true`)
- Wildcard (`*`) is not supported when using credentials

## VALKEY_URL

Connection URL for Valkey (Redis-compatible) used for distributed rate limiting across control-plane instances.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (uses per-instance in-memory rate limiting) |

**Example:**

```bash
# Local Valkey
VALKEY_URL=redis://localhost:6379

# With authentication
VALKEY_URL=redis://user:password@valkey.example.com:6379

# TLS (managed cloud service)
VALKEY_URL=rediss://user:password@valkey.example.com:6380
```

**Notes:**
- When not set, rate limiting falls back to in-memory governor (per-instance, no coordination)
- With N instances behind a load balancer, per-instance rate limiting allows N× the intended budget per IP — set `VALKEY_URL` for coordinated limits
- Accepts `redis://`, `rediss://` (TLS), `valkey://`, `valkeys://` (TLS) schemes
- Fail-open: if Valkey is unreachable, requests are allowed (availability over strictness)
- Only used by control-plane (server); workers don't need this variable
- Uses sliding-window counters via Lua scripts for atomic rate limit checks

## NATS_URL

Connection URL for NATS with JetStream, used for push-based event delivery and task notifications.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (uses PG NOTIFY for task notifications, in-memory broadcast for SSE event delivery) |

**Example:**

```bash
# Local NATS
NATS_URL=nats://localhost:4222

# Cluster
NATS_URL=nats://nats1:4222,nats://nats2:4222,nats://nats3:4222
```

**Notes:**
- When not set, the system behaves exactly as before — all events persist to PG, SSE polls PG, task notifications use PG NOTIFY. Zero behavioral change.
- When set, enables two features:
  - **Ephemeral event delivery**: delta events (`output.message.delta`, `reason.thinking.delta`, `tool.output.delta`, `llm.generation`) skip PostgreSQL and flow only through NATS JetStream. SSE streams subscribe to NATS instead of polling PG.
  - **Task notifications**: `task.available.{activity_type}` subjects replace PG NOTIFY for push-based worker notification. Lower latency (~1ms vs ~30ms), supports multi-instance deployments.
- NATS JetStream must be enabled on the server (`--jetstream` flag)
- Fail-graceful: if NATS connection fails at startup, falls back to PG NOTIFY + in-memory delivery with a warning
- Only used by control-plane (server); workers communicate via gRPC and don't need NATS access
- Default port: 4222 (or `PORT_PREFIX22` with `PORT_PREFIX`)
- `just start-all` automatically starts NATS and exports `NATS_URL` if `nats-server` is installed

## LLM Provider API Keys

LLM provider API keys (OpenAI, Anthropic, Gemini) are primarily stored encrypted in the database and managed via the Settings > Providers UI.

| Property | Value |
|----------|-------|
| **Storage** | Database (encrypted with AES-256-GCM) |
| **Configuration** | Settings > Providers UI or `/v1/llm-providers` API |
| **Supported Providers** | OpenAI, Anthropic, Google Gemini |

**Required for encryption:**

The `SECRETS_ENCRYPTION_KEY` environment variable must be set for the control-plane API to encrypt/decrypt API keys. Workers receive decrypted API keys via gRPC and do not need this variable.

```bash
# Generate a new key
python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"

# Set in environment (control-plane only)
SECRETS_ENCRYPTION_KEY=kek-v1:your-generated-key-here
```

### Default API Keys (Development Convenience)

For development, you can set default API keys via environment variables on the **control-plane only**. These are used as fallbacks when providers don't have keys configured in the database.

| Variable | Description |
|----------|-------------|
| `DEFAULT_OPENAI_API_KEY` | Fallback API key for OpenAI providers |
| `DEFAULT_ANTHROPIC_API_KEY` | Fallback API key for Anthropic providers |
| `DEFAULT_GEMINI_API_KEY` | Fallback API key for Google Gemini providers |

**Example:**

```bash
# Set in .env or environment (control-plane only)
DEFAULT_OPENAI_API_KEY=sk-...
DEFAULT_ANTHROPIC_API_KEY=sk-ant-...
DEFAULT_GEMINI_API_KEY=AIza...
```

**Notes:**
- These variables are only used by the control-plane, not workers
- Workers receive API keys via gRPC from the control-plane
- Database-stored keys always take priority over environment variables
- These are intended for development convenience, not production use
- The `just start-all` command automatically sets these from `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `GEMINI_API_KEY` if present
- If no API key is configured for a provider, LLM calls will fail and users will see an error message in the chat: "I encountered an error while processing your request. Please try again later."

## UI API Proxy Architecture

The UI makes all REST API requests (including SSE) to `/api/*` paths. The backend serves those routes under `/api` directly. Root-level backend routes like `/oauth/*`, `/mcp`, and `/.well-known/*` bypass the UI and are proxied straight to the backend.

**Local Development:**
- Caddy on `:9300` routes `/api/*`, `/oauth/*`, `/mcp`, and `/.well-known/*` to backend at `:9301`
- Example: `/api/v1/agents` → `http://localhost:9301/api/v1/agents`
- Example: `/oauth/authorize?...` → `http://localhost:9301/oauth/authorize?...`
- Example: `/mcp` → `http://localhost:9301/mcp`
- Example: `/.well-known/oauth-authorization-server` → `http://localhost:9301/.well-known/oauth-authorization-server`
- SSE streaming works via `flush_interval -1` in Caddy config
- No CORS needed (same-origin through Caddy)

**Production:**
- Configure your reverse proxy (nginx, Caddy, etc.) to route `/api/*`, `/oauth/*`, `/mcp`, and `/.well-known/*` to the API server
- Disable response buffering for SSE endpoints
- Example Caddy config: see `local/Caddyfile`

## SSE Streaming Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `SSE_REALTIME_CYCLE_SECS` | `300` | Connection cycle interval for session event streams (seconds) |
| `SSE_MONITORING_CYCLE_SECS` | `600` | Connection cycle interval for durable monitoring streams (seconds) |
| `SSE_HEARTBEAT_INTERVAL_SECS` | `30` | Interval between heartbeat comments on all SSE streams (seconds) |
| `SSE_GLOBAL_MAX` | `10000` | Maximum total SSE connections across all users |
| `SSE_PER_SESSION_MAX` | `5` | Maximum SSE connections per session |
| `SSE_PER_ORG_MAX` | `1000` | Maximum SSE connections per organization |

**Notes:**
- Heartbeat comments (`: heartbeat\n\n`) are sent on all SSE streams to detect stale connections
- The heartbeat interval must be less than the SDK read timeout (default: 60s) with safety margin
- Connection cycling prevents stale connections through proxies and load balancers
- When running behind HTTP/1.1 proxies, increase `SSE_REALTIME_CYCLE_SECS` to reduce reconnection frequency

## Worker gRPC Configuration

### WORKER_GRPC_ADDRESS

Address of the control-plane gRPC server for worker communication.

| Property | Value |
|----------|-------|
| **Required** | No (worker only) |
| **Default** | `127.0.0.1:9001` |

**Example:**

```bash
WORKER_GRPC_ADDRESS=127.0.0.1:9001
```

**Notes:**
- Workers communicate with the control-plane via gRPC for all database operations
- The control-plane exposes both HTTP (local dev default `9301`) and gRPC (default `9001`) interfaces
- Workers are stateless and do not connect directly to the database

### WORKER_GRPC_AUTH_TOKEN

Bearer token for authenticating worker gRPC connections to the control-plane.

| Property | Value |
|----------|-------|
| **Required** | Yes (production); No (dev mode) |
| **Default** | Unset (auth disabled) |

**Example:**

```bash
WORKER_GRPC_AUTH_TOKEN=your-secret-token
```

**Notes:**
- Must be set on both the server and all workers (same value)
- When unset, gRPC auth is disabled (acceptable for local development only)
- Server panics on startup if unset when not in dev mode

### WORKER_GRPC_ADDR

Bind address for the server-side gRPC listener (control-plane only).

| Property | Value |
|----------|-------|
| **Required** | No (server only) |
| **Default** | `0.0.0.0:9001` |

**Example:**

```bash
WORKER_GRPC_ADDR=0.0.0.0:9001
```

### WORKER_GRPC_CONNECT_TIMEOUT

Timeout in seconds for worker initial connection to control-plane gRPC.

| Property | Value |
|----------|-------|
| **Required** | No (worker only) |
| **Default** | `30` |

**Example:**

```bash
WORKER_GRPC_CONNECT_TIMEOUT=60
```

### WORKER_GRPC_TLS_CERT

Path to PEM-encoded certificate file. On the server, this is the gRPC server certificate. On the worker, this is the client certificate presented during mTLS handshake.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (TLS disabled) |

**Example:**

```bash
WORKER_GRPC_TLS_CERT=/etc/everruns/grpc-cert.pem
```

**Notes:**
- Must be set together with `WORKER_GRPC_TLS_KEY`
- Server: enables TLS on the gRPC listener when both cert and key are set
- Worker: presents client certificate to the server when both cert and key are set (requires `WORKER_GRPC_TLS_CA_CERT`)

### WORKER_GRPC_TLS_KEY

Path to PEM-encoded private key file corresponding to `WORKER_GRPC_TLS_CERT`.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set |

**Example:**

```bash
WORKER_GRPC_TLS_KEY=/etc/everruns/grpc-key.pem
```

### WORKER_GRPC_TLS_CA_CERT

Path to PEM-encoded CA certificate bundle for verifying the remote peer.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set |

**Example:**

```bash
WORKER_GRPC_TLS_CA_CERT=/etc/everruns/grpc-ca.pem
```

**Notes:**
- Server: when set, requires workers to present valid client certificates signed by this CA (mutual TLS)
- Worker: when set, verifies the server's certificate against this CA and switches to `https://` transport
- For full mTLS, set on both server and worker alongside their respective cert/key pairs

### WORKER_GRPC_TLS_DOMAIN

Override the expected server domain name for TLS certificate verification (worker only).

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Derived from `WORKER_GRPC_ADDRESS` hostname |

**Example:**

```bash
WORKER_GRPC_TLS_DOMAIN=control-plane.internal
```

**Notes:**
- Useful when the server certificate CN/SAN differs from the connection hostname (e.g., connecting via IP but cert has a DNS name)

## OpenTelemetry Configuration

Everruns supports distributed tracing via OpenTelemetry with OTLP export. Traces follow the [Gen-AI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) for LLM operations.

### OTEL_EXPORTER_OTLP_ENDPOINT

OTLP endpoint for trace export (e.g., Grafana Tempo, Datadog, or any OTLP-compatible backend).

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (tracing disabled) |

**Example:**

```bash
# For local OTLP collector
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# For production Tempo
OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo.monitoring:4317
```

**Notes:**
- When set, traces are exported via OTLP/gRPC
- Connect to any OTLP-compatible backend for trace visualization
- Without this variable, only console logging is enabled

### OTEL_SERVICE_NAME

Service name for traces.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `everruns-server` (API), `everruns-worker` (Worker) |

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
# Standard OTel env var (preferred)
OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true

# Legacy alias (also works)
OTEL_RECORD_CONTENT=true
```

**Notes:**
- When enabled, `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result`, and thinking content are recorded
- Disabled by default for privacy and data size concerns
- Only enable in development or when debugging specific issues

## Local Development with OpenTelemetry

To visualize traces locally, point `OTEL_EXPORTER_OTLP_ENDPOINT` at any OTLP-compatible collector:

```bash
# Set OTLP endpoint for API and Worker
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# Start services
just start-all
```

### Gen-AI Trace Structure

Traces follow the agentic execution lifecycle with 13 event types:

```
invoke_agent {turn_id} (root span)
├── reason (LLM reasoning phase)
│   ├── thinking (extended thinking, if enabled)
│   └── chat {model} (LLM API call)
├── act (tool execution phase)
│   ├── execute_tool {name}
│   └── execute_tool {name}
├── reason (iteration 2)
│   └── chat {model}
└── ...
```

### Gen-AI Trace Attributes

All spans include OpenTelemetry attributes following the Gen-AI semantic conventions:

| Attribute | Span Types | Description |
|-----------|-----------|-------------|
| `gen_ai.operation.name` | All | Operation type (`invoke_agent`, `chat`, `execute_tool`, `reason`, `act`, `thinking`) |
| `gen_ai.system` | chat | Provider (`openai`, `anthropic`, `gemini`) |
| `gen_ai.request.model` | chat, thinking | Requested model name |
| `gen_ai.response.model` | chat | Model actually used |
| `gen_ai.response.id` | chat | Response identifier |
| `gen_ai.response.finish_reasons` | chat | Why generation stopped |
| `gen_ai.usage.input_tokens` | chat, reason, invoke_agent | Prompt tokens used |
| `gen_ai.usage.output_tokens` | chat, reason, invoke_agent | Completion tokens used |
| `gen_ai.usage.cache_read_tokens` | chat | Tokens read from prompt cache |
| `gen_ai.usage.cache_creation_tokens` | chat | Tokens written to prompt cache |
| `gen_ai.output.type` | chat | `text` or `tool_calls` |
| `gen_ai.conversation.id` | All | Session identifier |
| `gen_ai.tool.name` | execute_tool | Tool name |
| `gen_ai.tool.call.id` | execute_tool | Tool call identifier |
| `tool.success` | execute_tool | Whether tool succeeded |
| `turn.id` | invoke_agent | Turn identifier |
| `turn.iterations` | invoke_agent | Number of reason/act iterations |
| `error.type` | invoke_agent, chat, execute_tool | Error description (on failure) |
| `otel.status_code` | invoke_agent | `ERROR` on failure/cancellation |
| `duration_ms` | All | Span duration in milliseconds |
| `time_to_first_token_ms` | chat | Streaming latency |

## Braintrust Integration

Everruns supports sending LLM generation events to [Braintrust](https://www.braintrust.dev/) for observability, evaluation, and logging.

For setup instructions and configuration details, see the [Braintrust Integration Guide](/observability/braintrust/).

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_API_KEY` | Yes | - | API key from Braintrust settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name for organizing traces |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name lookup) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |
