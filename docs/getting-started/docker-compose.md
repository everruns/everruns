---
title: Docker Compose Quickstart
description: Deploy Everruns with Docker Compose in minutes
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
```

### 4. Start Services

```bash
docker compose up -d
```

This starts:
- PostgreSQL database
- Control plane API
- 3 worker instances
- Next.js UI
- Caddy reverse proxy
- Jaeger tracing (optional)

### 5. Access the Platform

| Service | URL |
|---------|-----|
| **Web UI** | http://localhost:8080 |
| **Swagger API Docs** | http://localhost:8080/swagger-ui/ |
| **Health Check** | http://localhost:8080/health |
| **Jaeger Tracing** | http://localhost:16686 |

## Configuration

### Configure LLM Provider

If you didn't set `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY` in your `.env` file, configure via UI:

1. Open http://localhost:8080
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
# Create a session
curl -X POST http://localhost:8080/api/v1/agents/{agent_id}/sessions \
  -H "Content-Type: application/json"

# Send a message
curl -X POST http://localhost:8080/api/v1/sessions/{session_id}/messages \
  -H "Content-Type: application/json" \
  -d '{"content": [{"type": "text", "text": "Hello!"}]}'
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
docker compose logs -f api
docker compose logs -f worker-1
```

### Distributed Tracing

Jaeger UI is available at http://localhost:16686 for viewing request traces across services.

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

The `migrate` service runs once on startup. If it fails:

```bash
# Check migration logs
docker compose logs migrate

# Retry migrations
docker compose up migrate
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

- [API Reference](/api) - Full API documentation
- [Capabilities](/features/capabilities) - Extend agent functionality
- [Environment Variables](/sre/environment-variables) - Advanced configuration
