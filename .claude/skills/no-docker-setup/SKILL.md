---
name: no-docker-setup
description: Set up full production-like backend (PostgreSQL + Valkey + Caddy + API + Worker) without Docker. Use for testing durable workflows, database persistence, SSE through proxy, or when DEV_MODE is insufficient.
---

# No-Docker Setup

**Full production-like backend** without Docker: PostgreSQL + Valkey + Caddy + API + Worker.

## When to Use

| Mode | Use Case |
|------|----------|
| `just start-dev --no-watch` | Quick testing, UI work, in-memory (no persistence) |
| **This skill** | Durable workflows, database testing, SSE proxy testing |
| `just start-all` | Full setup with Docker (easiest if Docker available) |

## What It Does

1. Sets up fresh PostgreSQL cluster at `/tmp/pgdata`
2. Installs and starts Valkey for distributed rate limiting (falls back to redis-server if available)
3. Installs and starts Caddy reverse proxy on `:9300`
   - Routes `/api/*` to `:9000` (strips prefix)
   - Disables response buffering for SSE streaming
4. Runs `just start-all --no-watch --no-docker --no-ui`
   - API server auto-applies migrations on startup
   - Starts API server (port 9000)
   - Starts Worker (port 9001)

## Quick Start

```bash
# Full setup with Caddy proxy (production-like)
sudo -E .claude/skills/no-docker-setup/scripts/start.sh

# Without Caddy (direct API access on :9000)
sudo -E .claude/skills/no-docker-setup/scripts/start.sh --no-caddy
```

## Prerequisites

1. **PostgreSQL 16+** - `apt-get install postgresql-16`
2. **jq** - `apt-get install jq`
3. **API Key** - `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`
4. **Root access** - For PostgreSQL cluster initialization

Caddy is auto-installed if not present.

## Architecture

```
Client (:9300) → Caddy (proxy) → API (:9000) → Worker (:9001)
                                     ↓
                               PostgreSQL (:5432)
                                     ↓
                               Valkey (:6379, optional)
```

### SSE Through Proxy

Caddy is configured for SSE streaming:
- `flush_interval -1`: No response buffering (required for SSE)

The server handles 5-min connection cycling with `disconnecting` events.
The SDK reconnects transparently without consuming retry budget.

## Testing

```bash
# Via Caddy proxy (matches production)
curl http://localhost:9300/api/health

# Direct API access (bypasses proxy)
curl http://localhost:9000/health

# Load test through Caddy
API_URL=http://localhost:9300/api just load-test quick
```
