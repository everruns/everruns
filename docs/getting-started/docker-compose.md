---
title: Docker Compose
description: Step-by-step guide to deploying the complete Everruns platform with Docker Compose, including the control plane, workers, UI, and PostgreSQL database.
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

### 2. Generate Encryption Key

Everruns encrypts LLM API keys at rest. Generate a key:

```bash
python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"
```

### 3. Create Environment File

Create a `.env` file with your encryption key and optional LLM API keys:

```bash
# .env
SECRETS_ENCRYPTION_KEY=kek-v1:<your-generated-key>

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
- Control plane API
- 3 worker instances
- Next.js UI
- Caddy reverse proxy

### 5. Access the Platform

| Service | URL |
|---------|-----|
| **Web UI** | http://localhost:9300 |
| **API** | http://localhost:9300/api/... |
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

```bash
# Create a session (agent_id in request body)
curl -X POST http://localhost:9300/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "{agent_id}"}'

# Send a message
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/messages \
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

### Database Connection Issues

If services fail to connect to PostgreSQL:

```bash
# Check postgres health
docker compose ps postgres

# View postgres logs
docker compose logs postgres
```

### Migration Failures

Migrations are auto-applied when the API server starts. If migrations fail:

```bash
# Check API logs for migration errors
docker compose logs api

# Restart API to retry migrations
docker compose restart api
```

To check migration status before deployment:

```bash
docker compose exec api everruns-admin migrate-info
```

### Worker Not Processing

Verify workers can reach the control plane:

```bash
# Check worker logs
docker compose logs worker-1

# Verify gRPC connection
docker compose exec worker-1 /bin/sh -c "echo" || echo "Cannot exec (distroless image)"
```

## Next Steps

- [API Reference](/api/) - Full API documentation
- [Capabilities](/features/capabilities/) - Extend agent functionality
- [Environment Variables](/sre/environment-variables/) - Advanced configuration
