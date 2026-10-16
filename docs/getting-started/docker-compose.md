---
title: Docker Compose
description: "Deploy the Everruns platform with Docker Compose: control plane, workers, UI, and PostgreSQL."
---

Deploy the complete Everruns platform using Docker Compose. This guide sets up the control plane, workers, UI, and database in a single command.

## Prerequisites

- Docker Engine 20.10+
- Docker Compose v2.23.1+ (the file uses inline `configs.content`)
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

# Optional, single-tenant admin mode: encrypt these keys into the default org.
# The explicit seeding opt-in is required to skip UI credential configuration.
SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV=true
DEFAULT_OPENAI_API_KEY=sk-...
DEFAULT_ANTHROPIC_API_KEY=sk-ant-...
DEFAULT_GEMINI_API_KEY=AIza...
DEFAULT_OPENROUTER_API_KEY=sk-or-...
```

The optional environment-key path stores keys encrypted in the default
organization's database. Production startup otherwise ignores `DEFAULT_*` keys
for tenant execution. Enable seeding only for a single-tenant deployment with
admin auth; built-in signup or OAuth self-provisioning prevents this opt-in from
sharing host credentials. Leave it unset and use the UI for other deployments.

### 4. Start Services

```bash
docker compose pull  # Fetch latest images
docker compose up -d
```

The published compose file defaults to app entry point on `9300`. If that port is busy, override before startup:

```bash
EXAMPLE_PROXY_PORT=10300 VICTORIAMETRICS_PORT=18428 docker compose up -d
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
VICTORIAMETRICS_PORT=18428 \
docker compose up -d
```

This isolates Compose networks and volumes. Override **both** published ports:
VictoriaMetrics also binds a host port. To restrict a local test to this machine,
use `EXAMPLE_PROXY_PORT=127.0.0.1:10300` and
`VICTORIAMETRICS_PORT=127.0.0.1:18428`.

The downloaded file follows `main`, while its application images default to the
published `latest` tag. These can be different revisions. Use `EVERRUNS_TAG=vX.Y.Z`
to select a published release; `docker compose config --images` shows the selected
images. Run `docker compose pull` before recreating services after a tag change.

### Configure LLM Provider

If you didn't set LLM API keys (`DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY`, `DEFAULT_GEMINI_API_KEY`, or `DEFAULT_OPENROUTER_API_KEY`) in your `.env` file, configure via UI:

1. Open http://localhost:9300
2. Open **Models** > **Providers** (also available from Settings)
3. Click **Set key** on your provider, such as OpenRouter, and enter an inference
   API key. Use an existing limited key, never an OpenRouter management key.
4. Wait for the key verification result, then click **Save Credentials**
5. Sync the provider's models if needed, enable a model that supports chat and tools,
   and select it under **Defaults**. For OpenRouter, `openai/gpt-6-luna` is a
   suitable lightweight choice.

A fresh **Chat** shows **No intelligence available** until a usable model is
configured. Provider credentials are encrypted using `SECRETS_ENCRYPTION_KEY`;
keep that key stable while retaining the database volume.

### Create Your First Agent

1. Go to **Agents** in the UI
2. Click **Create Agent**
3. Set a name and system prompt
4. Select your configured LLM provider
5. Save the agent

You can also create an agent through **Chat**. Ask, for example:
"Create an agent named `math-tutor` with the system prompt `You are a concise math
tutor. Explain arithmetic in one sentence.` Use my configured default model."
Open the returned agent link to verify it was saved. To run it, open **Playground**,
choose the agent, leave **Dry run** unchecked, start a chat, and ask
"What is 7 multiplied by 8?" A successful response should explain that the result
is 56.

### Repeatable real-provider E2E (repository checkout)

The opt-in test downloads the same Compose file, starts a unique project with
loopback ports and disposable volumes, signs in through the UI, verifies and saves
an OpenRouter key through the UI, creates an agent through Chat, and runs the
persisted agent in Playground. It checks durable events and the saved answer,
records the real browser journey, and removes its project and volumes on exit.
It makes paid requests; use a limited inference key and a low-cost model. No
simulated responses satisfy this test.

```bash
cd apps/ui
pnpm install --frozen-lockfile
pnpm exec playwright install chromium
cd ../..

# Supply OPENROUTER_API_KEY securely in the environment, without shell tracing.
# This opt-in permits encrypted credential storage in the disposable test stack.
EVERRUNS_E2E_ALLOW_PROVIDER_STORAGE=1 node scripts/e2e-compose-openrouter.mjs
```

The test prints its evidence directory, containing a WebM video, `report.json`, and
`commands.log`. Secret values are excluded from command output, inputs stay
masked, and no trace, HAR, or browser authentication state is saved. To verify
startup and UI login without a key, run
`node scripts/e2e-compose-openrouter.mjs --setup-only`; this does **not** verify
inference, agent creation, or agent execution. Set `EVERRUNS_TAG` to pin published
images, or set `COMPOSE_E2E_FILE` to your local
[Compose file](https://github.com/everruns/everruns/blob/main/examples/docker-compose-full.yaml)
to test a configuration change. See `node scripts/e2e-compose-openrouter.mjs --help` for options.

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
