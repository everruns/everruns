---
title: Docker Compose
description: "Deploy the Everruns platform with Docker Compose: control plane, workers, UI, and PostgreSQL."
---

Deploy the complete Everruns platform using Docker Compose. This guide sets up the control plane, workers, UI, and database in a single command.

## Prerequisites

- Docker Engine 20.10+
- Docker Compose v2.0+
- 4GB available RAM

## Quick Start

### 1. Download Docker Compose File

```bash
# Create directory and download docker-compose file
mkdir everruns && cd everruns
curl -o docker-compose.yaml https://raw.githubusercontent.com/everruns/everruns/main/examples/docker-compose-full.yaml
```

### 2. Generate Secrets

Everruns encrypts LLM API keys at rest, signs login sessions, and authenticates
workers to the control plane. Generate one value for each:

```bash
python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"  # encryption key
openssl rand -hex 32  # JWT secret
openssl rand -hex 32  # worker token
```

### 3. Create Environment File

Create a `.env` file next to `docker-compose.yaml`. The first five values are
required: Compose refuses to start without `AUTH_JWT_SECRET`, and the server
refuses to start without `WORKER_GRPC_AUTH_TOKEN`.

```bash
# .env
SECRETS_ENCRYPTION_KEY=kek-v1:<your-generated-key>
AUTH_JWT_SECRET=<jwt-secret>
WORKER_GRPC_AUTH_TOKEN=<worker-token>
AUTH_ADMIN_EMAIL=admin@example.com
AUTH_ADMIN_PASSWORD=<choose-a-password>

# Optional: Add API keys here to skip UI configuration
DEFAULT_OPENAI_API_KEY=sk-...
DEFAULT_ANTHROPIC_API_KEY=sk-ant-...
DEFAULT_GEMINI_API_KEY=AIza...
```

### 4. Start Services

```bash
docker compose pull  # Fetch latest images
docker compose up -d
```

The published compose file defaults to app entry point on `9300`. If that port is busy, override before startup:

```bash
EXAMPLE_PROXY_PORT=10300 docker compose up -d
```

This starts:
- PostgreSQL database
- Valkey (rate limiting) and NATS (event delivery and task notifications)
- Control plane (`server`): HTTP API and worker gRPC
- 3 worker instances
- Next.js UI
- Caddy reverse proxy
- VictoriaMetrics, scraping server metrics at http://localhost:8428/vmui

Sign in to the UI with `AUTH_ADMIN_EMAIL` and `AUTH_ADMIN_PASSWORD`.

### 5. Access the Platform

| Service | URL |
|---------|-----|
| **Web UI** | http://localhost:9300 |
| **API** | http://localhost:9300/api/... |
| **OAuth** | http://localhost:9300/oauth/... |
| **MCP** | http://localhost:9300/mcp |
| **OAuth Metadata** | http://localhost:9300/.well-known/oauth-authorization-server |
| **Health Check** | http://localhost:9300/health |

## Configuration

### Run Multiple Copies

If you want multiple Everruns compose stacks on the same machine, set both a Compose project name and host-port overrides:

```bash
COMPOSE_PROJECT_NAME=everruns-demo-2 \
EXAMPLE_PROXY_PORT=10300 \
docker compose up -d
```

### Configure LLM Provider

If you didn't set LLM API keys (`DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY`, or `DEFAULT_GEMINI_API_KEY`) in your `.env` file, configure via UI:

1. Open http://localhost:9300
2. Navigate to **Settings** > **Providers**
3. Add your OpenAI or Anthropic API key
4. Save and verify connection

### Create Your First Agent

1. Go to **Agents** in the UI
2. Click **Create Agent**
3. Set a name and system prompt
4. Select your configured LLM provider
5. Save the agent

### Start a Session

The API requires a token. Create a personal access token under **Settings** >
**Personal Access Tokens** in the UI (it starts with `evr_pat_` and is shown
once), or from the command line as described in
[Authentication](/sre/runbooks/authentication/#create-personal-access-token).

```bash
export EVERRUNS_TOKEN=evr_pat_...

# Create a session (agent_id in request body)
curl -X POST http://localhost:9300/api/v1/sessions \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "{agent_id}"}'

# Send a message
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/messages \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]}}'
```

## Scaling Workers

Add more workers by scaling the worker services:

```bash
# Scale to 5 workers
docker compose up -d --scale worker-1=1 --scale worker-2=1 --scale worker-3=3
```

Or modify `docker-compose.yaml` to add more worker services.

## Monitoring

### View Logs

```bash
# All services
docker compose logs -f

# Specific service
docker compose logs -f server
docker compose logs -f worker-1
```

### Distributed Tracing

Set `OTEL_EXPORTER_OTLP_ENDPOINT` to export traces to any OTLP-compatible backend (Grafana Tempo, Datadog, etc.).

## Stopping Services

```bash
# Stop all services
docker compose down

# Stop and remove volumes (deletes data)
docker compose down -v
```

## Troubleshooting

### Check Service Health

```bash
docker compose ps
```

`server` should report `healthy`. Its healthcheck runs `everruns-server --health-check`,
which calls `/health` inside the container. Images released before that flag existed
always show `unhealthy` even when the API works; check `curl http://localhost:9300/health` instead.

### Database Connection Issues

If services fail to connect to PostgreSQL:

```bash
# Check postgres health
docker compose ps postgres

# View postgres logs
docker compose logs postgres
```

### Migration Failures

Migrations are auto-applied when the server starts. If migrations fail:

```bash
# Check server logs for migration errors
docker compose logs server

# Restart the server to retry migrations
docker compose restart server
```

To inspect migration status, use the [admin container](/sre/admin-container/).

### Worker Not Processing

Verify workers can reach the control plane:

```bash
# Check worker logs
docker compose logs worker-1
```

A worker that logs authentication errors has a different `WORKER_GRPC_AUTH_TOKEN`
from the server. Both read it from the same `.env`, so recreate them after
changing it: `docker compose up -d --force-recreate`.

## Next Steps

- [API Reference](/api/) - Full API documentation
- [Capabilities](/features/capabilities/) - Extend agent functionality
- [Environment Variables](/sre/environment-variables/) - Advanced configuration
