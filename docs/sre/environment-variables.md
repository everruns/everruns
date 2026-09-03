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
- `/mcp` is always mounted and authenticated; there is no `FEATURE_MCP_ENDPOINT` deployment variable or organization feature toggle
- REST API routes including auth (`/v1/auth/*`) are mounted under this prefix
- OAuth callback URLs use `AUTH_BASE_URL`, which defaults to `PUBLIC_APP_URL` plus `API_PREFIX` when unset
- Override only if you need a non-`/api` REST prefix behind a reverse proxy or gateway

## PUBLIC_APP_URL

Public browser origin for the Everruns app. In single-origin deployments, set this once and the server derives `FRONTEND_URL` and `AUTH_BASE_URL` from it.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `http://localhost:9300` |

**Example:**

```bash
PUBLIC_APP_URL=https://everruns.example.com
```

**Notes:**
- `FRONTEND_URL` defaults to `PUBLIC_APP_URL`
- `AUTH_BASE_URL` defaults to `PUBLIC_APP_URL` plus `API_PREFIX` (for example, `https://everruns.example.com/api`)
- Set `FRONTEND_URL` only when browser redirects must land on a different origin
- Set `AUTH_BASE_URL` only when OAuth callbacks use a different public API base

## AUTH_LOGIN_ORIGIN

Trusted browser origin that hosts the login page. Set this when an OSS-UI-based
app delegates `/login` to a central identity origin.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (same-origin relative `/login`) |

**Example:**

```bash
AUTH_LOGIN_ORIGIN=https://id.example.com
```

**Notes:**
- Supply only an HTTP(S) origin, with no credentials, path, query, or fragment
- Set the same value in the server and UI runtime environments
- The value is trusted deployment configuration; request/query input cannot override it
- `return_to` remains a relative path and is still sanitized against open redirects
- Configured absolute login redirects use full-page navigation

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

## HTTP_ADDR

Bind address for the server HTTP API.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `0.0.0.0:9000` |

**Example:**

```bash
HTTP_ADDR=0.0.0.0:9000
```

**Notes:**
- `ADDR` is supported as a legacy alias
- Container images already default to `0.0.0.0:9000`; most deployments do not need to set this

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
- With N instances behind a load balancer, per-instance rate limiting allows N× the intended budget per IP, set `VALKEY_URL` for coordinated limits
- Accepts `redis://`, `rediss://` (TLS), `valkey://`, `valkeys://` (TLS) schemes
- Fail-open: if Valkey is unreachable, requests are allowed (availability over strictness)
- Only used by control-plane (server); workers don't need this variable
- Uses sliding-window counters via Lua scripts for atomic rate limit checks

## DATABASE_UNPOOLED_URL

Direct PostgreSQL connection URL used only for session-scoped `LISTEN/NOTIFY` listeners.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (listeners reuse `DATABASE_URL` if it is a direct connection) |

**Example:**

```bash
# Query traffic through a pooler, listeners through a direct endpoint
DATABASE_URL=postgres://app:secret@ep-foo-pooler.us-east-1.aws.neon.tech/everruns?sslmode=require
DATABASE_UNPOOLED_URL=postgres://app:secret@ep-foo.us-east-1.aws.neon.tech/everruns?sslmode=require
```

**Notes:**
- Use this when `DATABASE_URL` points at Neon `-pooler`, PgBouncer, or another proxy that does not preserve session-scoped `LISTEN/NOTIFY` semantics.
- Listener paths include PostgreSQL-backed event wakeups, notification SSE, and PG task notification fallback when NATS is unavailable.
- If `DATABASE_URL` or `DATABASE_UNPOOLED_URL` appears to point at a pooled/proxied endpoint, startup now fails fast with guidance to set a direct listener URL.
- Ordinary query traffic still uses `DATABASE_URL`.

## Object Storage (S3-compatible blob backend)

Optional backend that offloads workspace-file and image *content bytes* to an
S3-compatible object store while keeping all metadata in PostgreSQL. Everruns
remains the proxy for every read/write, no presigned URLs are handed to
clients or workers. See [knowledge/runtime-resources/object-storage.md](https://github.com/everruns/everruns/blob/main/knowledge/runtime-resources/object-storage.md).

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `STORAGE_BLOB_BACKEND` | No | `db` | `db` keeps bytes inline in PostgreSQL (current behavior); `s3` offloads to object storage. |
| `STORAGE_S3_BUCKET` | When `s3` |, | Target bucket name. |
| `STORAGE_S3_REGION` | No |, | Bucket region / region label. |
| `STORAGE_S3_ENDPOINT` | No |, | Custom endpoint for S3-compatible stores (SeaweedFS, R2). Unset for AWS S3. |
| `STORAGE_S3_ACCESS_KEY_ID` | No |, | Static access key. Omit to use the AWS credential chain (IAM role/instance). |
| `STORAGE_S3_SECRET_ACCESS_KEY` | No |, | Static secret key. |
| `STORAGE_S3_PREFIX` | No | (empty) | Key prefix isolating multiple deployments within one bucket. |
| `STORAGE_S3_ALLOW_HTTP` | No | `false` | Allow plaintext HTTP (local/dev only, e.g. SeaweedFS over HTTP). |
| `STORAGE_S3_FORCE_PATH_STYLE` | No | `true` | Use path-style requests (required by SeaweedFS; harmless on AWS S3). |
| `STORAGE_BLOB_GC_INTERVAL_SECONDS` | No | `21600` (6h) | Interval between blob GC sweeps that reclaim orphaned objects. `0` disables GC. Only effective with the `s3` backend (inline `db` storage has no orphans). |
| `STORAGE_BLOB_GC_GRACE_SECONDS` | No | `86400` (24h) | Safety grace period; orphaned objects younger than this are never deleted (avoids racing in-flight creates). |
| `STORAGE_BLOB_GC_MAX_DELETES_PER_RUN` | No | `10000` | Per-sweep deletion cap to bound work; remaining orphans are reclaimed next sweep. |
| `STORAGE_BLOB_GC_MAX_LIST_PER_RUN` | No | `100000` | Per-sweep cap on objects listed per prefix, bounding GC memory; larger buckets are reconciled across sweeps in key-order windows. |

**Example (local SeaweedFS via `just seaweedfs`):**

```bash
STORAGE_BLOB_BACKEND=s3
STORAGE_S3_BUCKET=everruns-dev
STORAGE_S3_ENDPOINT=http://127.0.0.1:8333
STORAGE_S3_REGION=us-east-1
STORAGE_S3_ACCESS_KEY_ID=everruns
STORAGE_S3_SECRET_ACCESS_KEY=everruns-secret
STORAGE_S3_ALLOW_HTTP=true
```

Any S3-compatible store works (AWS S3, SeaweedFS, R2); only the endpoint
and credentials differ.

**Notes:**
- The backend is selected per process at startup; a deployment runs entirely on
  `db` or `s3`. Enabling `s3` offloads newly written content; pre-existing inline
  content is still served transparently.
- Tenant isolation is by object key (`workspaces/{workspace_id}/…`,
  `images/org-{org_id}/…`) plus the existing org/workspace authorization layer.

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
- When not set, the system behaves exactly as before, all events persist to PG, SSE polls PG, task notifications use PG NOTIFY. Zero behavioral change.
- When set, enables two features:
  - **Ephemeral event delivery**: delta events (`output.message.delta`, `reason.thinking.delta`, `tool.output.delta`, `llm.generation`) skip PostgreSQL and flow only through NATS JetStream. SSE streams subscribe to NATS instead of polling PG.
  - **Task notifications**: `task.available.{activity_type}` subjects replace PG NOTIFY for push-based worker notification. Lower latency (~1ms vs ~30ms), supports multi-instance deployments.
- When NATS event delivery is active, the server skips the legacy PostgreSQL event listener used only for SSE wakeups.
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
| **Configuration** | Settings > Providers UI or `/v1/providers` API |
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
| `DEFAULT_META_API_KEY` | Fallback API key for Meta Model API providers |

**Example:**

```bash
# Set in .env or environment (control-plane only)
DEFAULT_OPENAI_API_KEY=sk-...
DEFAULT_ANTHROPIC_API_KEY=sk-ant-...
DEFAULT_GEMINI_API_KEY=AIza...
DEFAULT_META_API_KEY=...
```

**Notes:**
- These variables are only used by the control-plane, not workers
- Workers receive API keys via gRPC from the control-plane
- Database-stored keys always take priority over environment variables
- These are intended for development convenience, not production use
- The `just start-all` command automatically sets these from `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `GEMINI_API_KEY` if present
- If no API key is configured for a provider, LLM calls will fail and users will see an error message in the chat: "I encountered an error while processing your request. Please try again later."

## System Email Delivery

System email delivery is an internal service used by product and operational flows. It is not an agent capability, public API, or UI setting.

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `EMAIL_PROVIDER` | Yes, when sending email in production | unset / disabled | Email provider. Supported values: `disabled`, `resend` |
| `RESEND_API_KEY` | Yes, when `EMAIL_PROVIDER=resend` | unset | Resend API key |
| `RESEND_API_BASE_URL` | No | `https://api.resend.com` | Resend API base URL override for tests or controlled deployments |

**Example:**

```bash
EMAIL_PROVIDER=resend
RESEND_API_KEY=re_...
```

**Notes:**
- Set these on the control-plane process that performs system email sends.
- The sender is fixed in code as `Everruns <no-replay@everruns.com>`.
- The Resend account must have `everruns.com` verified and enabled for sending.

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

### SERVER_GRPC_ADDRESS

Address of the server gRPC endpoint for worker communication.

| Property | Value |
|----------|-------|
| **Required** | No (worker only) |
| **Default** | `127.0.0.1:9001` |

**Example:**

```bash
SERVER_GRPC_ADDRESS=127.0.0.1:9001
```

**Notes:**
- Workers communicate with the server via gRPC for all database operations
- `WORKER_GRPC_ADDRESS` is supported as a legacy alias
- The server exposes both HTTP (default `9000`) and gRPC (default `9001`) interfaces
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

### SERVER_GRPC_BIND_ADDR

Bind address for the server-side gRPC listener.

| Property | Value |
|----------|-------|
| **Required** | No (server only) |
| **Default** | `0.0.0.0:9001` |

**Example:**

```bash
SERVER_GRPC_BIND_ADDR=0.0.0.0:9001
```

**Notes:**
- `WORKER_GRPC_ADDR` is supported as a legacy alias

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
| **Default** | Derived from `SERVER_GRPC_ADDRESS` hostname |

**Example:**

```bash
WORKER_GRPC_TLS_DOMAIN=control-plane.internal
```

**Notes:**
- Useful when the server certificate CN/SAN differs from the connection hostname (e.g., connecting via IP but cert has a DNS name)

## OpenTelemetry Configuration

Everruns supports distributed tracing via OpenTelemetry with OTLP export. Agent traces follow the OpenTelemetry [Gen-AI agent and inference conventions](https://github.com/open-telemetry/semantic-conventions-genai/tree/main/docs/gen-ai) and, on the same spans, the [OpenInference conventions](https://arize-ai.github.io/openinference/spec/semantic_conventions.html) read by Arize Phoenix, so one OTLP endpoint serves both families of backends.

### OTEL_EXPORTER_OTLP_ENDPOINT

OTLP endpoint for trace export (e.g., Grafana Tempo, Arize Phoenix, Datadog, or any OTLP-compatible backend).

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | Not set (tracing disabled) |

**Example:**

```bash
# For a local OTLP collector or Phoenix
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318

# For production Tempo
OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo.monitoring:4318
```

**Notes:**
- When set, traces are exported via OTLP over HTTP/protobuf (use the backend's HTTP port, typically 4318)
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
- When enabled, the chat span records `gen_ai.system_instructions`, `gen_ai.input.messages`, `gen_ai.output.messages`, and `gen_ai.tool.definitions` (plus the OpenInference `input.value`, `output.value`, and flattened `llm.input_messages.*`); tool spans record `gen_ai.tool.call.arguments` and `gen_ai.tool.call.result`; the turn root records the input message and final answer; the thinking span records the reasoning text
- Disabled by default for privacy and data size concerns
- Only enable in development or when debugging specific issues

### EVERRUNS_TRACE_CONVENTIONS

Which attribute vocabularies agent spans carry.

| Property | Value |
|----------|-------|
| **Required** | No |
| **Default** | `gen_ai,openinference` |

**Example:**

```bash
# Only the OpenTelemetry Gen-AI attributes (Tempo, Jaeger, Datadog, Langfuse)
EVERRUNS_TRACE_CONVENTIONS=gen_ai

# Only the OpenInference attributes (Arize Phoenix)
EVERRUNS_TRACE_CONVENTIONS=openinference
```

**Notes:**
- Span names, kinds, and hierarchy are the same under either vocabulary; only attributes differ
- Unknown values are ignored, and an empty selection falls back to both

## Local Development with OpenTelemetry

To visualize traces locally, point `OTEL_EXPORTER_OTLP_ENDPOINT` at any OTLP-compatible collector:

```bash
# Set OTLP endpoint for API and Worker
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318

# Start services
just start-all
```

To see traces in Arize Phoenix, run Phoenix locally and point the same variable at its OTLP/HTTP port (`http://localhost:6006`); spans render as AGENT, LLM, and TOOL spans with token counts and, when content capture is on, the messages.

### Gen-AI Trace Structure

Traces follow the agentic execution lifecycle with 13 event types; every span starts and ends at the timestamp of the event it records:

```
invoke_agent {agent name} (root span, INTERNAL)
├── reason (LLM reasoning phase)
│   └── chat {model} (LLM API call, CLIENT)
│       └── thinking (extended thinking, if enabled)
├── act (tool execution phase)
│   ├── execute_tool {name}
│   └── execute_tool {name}
├── reason (iteration 2)
│   └── chat {model}
└── ...
```

### Gen-AI Trace Attributes

Spans carry the OpenTelemetry Gen-AI attributes and the OpenInference attributes side by side (see `EVERRUNS_TRACE_CONVENTIONS`). The most useful ones:

| Attribute | Span Types | Description |
|-----------|-----------|-------------|
| `gen_ai.operation.name` | invoke_agent, chat, execute_tool | `invoke_agent`, `chat`, or `execute_tool` |
| `gen_ai.agent.id`, `gen_ai.agent.name`, `gen_ai.agent.description` | invoke_agent | Agent the turn runs as |
| `gen_ai.conversation.id` / `session.id` | All | Session identifier |
| `gen_ai.provider.name` / `llm.provider` | chat | Provider (`openai`, `anthropic`, `gcp.gemini`, `aws.bedrock`, ...) |
| `gen_ai.request.model`, `gen_ai.response.model` / `llm.model_name` | chat | Model name |
| `gen_ai.response.id` | chat | Provider response identifier |
| `gen_ai.response.finish_reasons` | chat | Why generation stopped (string array) |
| `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens` / `llm.token_count.*` | chat, invoke_agent | Token usage (turn root carries the cumulative total) |
| `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens` | chat, invoke_agent | Prompt-cache tokens |
| `gen_ai.request.temperature`, `gen_ai.request.max_tokens`, `gen_ai.request.reasoning.level`, `gen_ai.request.stream` / `llm.invocation_parameters` | chat | Request parameters |
| `gen_ai.response.time_to_first_chunk` | chat | Streaming latency in seconds |
| `gen_ai.conversation.compacted` | chat | Context was compacted before the call |
| `llm.cost.total` / `everruns.usage.cost_usd` | chat, invoke_agent | Cost in USD when known |
| `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.type`, `gen_ai.tool.description` / `tool.name`, `tool.description` | execute_tool | Tool identity |
| `openinference.span.kind` | All | `AGENT`, `LLM`, `TOOL`, or `CHAIN` |
| `error.type` | All | Low-cardinality error class on failure (error code, HTTP status, `timeout`, or `_OTHER`); the span status carries the message |
| `everruns.phase` | reason, act, thinking | Phase span marker |
| `everruns.turn.id`, `everruns.exec.id` | All | Everruns correlation ids |
| `everruns.turn.iterations`, `everruns.turn.tool_call_count`, `everruns.turn.llm_call_count` | invoke_agent | Turn counters |
| `everruns.tool.status` | execute_tool | `success`, `error`, `timeout`, or `cancelled` |

## Braintrust Integration

Everruns supports sending turn, reasoning, tool, and session lifecycle events to [Braintrust](https://www.braintrust.dev/) for observability, evaluation, and logging.

For setup instructions and configuration details, see the [Braintrust Integration Guide](/observability/braintrust/).

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_ENABLED` | No | enabled when API key is present | Explicit Braintrust on/off switch |
| `BRAINTRUST_API_KEY` | Yes | - | API key from Braintrust settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name for organizing traces |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name lookup) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |
| `BRAINTRUST_QUEUE_CAPACITY` | No | `1024` | Buffered event capacity before new exports are dropped |
| `BRAINTRUST_MAX_BATCH_SIZE` | No | `50` | Max events per Braintrust insert call |
| `BRAINTRUST_FLUSH_INTERVAL_MS` | No | `500` | Max delay before flushing a partial batch |
| `BRAINTRUST_REQUEST_TIMEOUT_MS` | No | `10000` | Per-request timeout for Braintrust insert calls |
| `BRAINTRUST_MAX_RETRIES` | No | `3` | Retries for `429`, `5xx`, and timeout/connect failures |
| `BRAINTRUST_RETRY_BASE_DELAY_MS` | No | `250` | Initial retry backoff |
| `BRAINTRUST_RETRY_MAX_DELAY_MS` | No | `5000` | Retry backoff cap |
| `BRAINTRUST_RECORD_CONTENT` | No | `false` | Export raw turn and LLM text content |
| `BRAINTRUST_RECORD_THINKING` | No | `none` | Extended thinking export mode: `none`, `summary`, `full` |
| `BRAINTRUST_TOOL_ARGS_MODE` | No | `redacted` | Tool argument export mode: `full`, `redacted`, `none` |
| `BRAINTRUST_TOOL_RESULTS_MODE` | No | `summary` | Tool result export mode: `full`, `summary`, `redacted`, `none` |
| `BRAINTRUST_DEBUG_PAYLOADS` | No | `false` | Print full outbound Braintrust payload JSON to local debug logs |
