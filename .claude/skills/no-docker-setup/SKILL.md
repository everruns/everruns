---
name: no-docker-setup
description: Set up full production-like backend (PostgreSQL + Caddy + API + Worker) without Docker. Use for testing durable workflows, database persistence, SSE through proxy, or when DEV_MODE is insufficient.
---

# No-Docker Setup

**Full production-like backend** without Docker: PostgreSQL + Caddy + API + Worker.

## When to Use

| Mode | Use Case |
|------|----------|
| `just start-dev --no-watch` | Quick testing, UI work, in-memory (no persistence) |
| **This skill** | Durable workflows, database testing, SSE proxy testing |
| `just start-all` | Full setup with Docker (easiest if Docker available) |

## What It Does

1. Sets up fresh PostgreSQL cluster at `/tmp/pgdata`
2. Installs and starts Caddy reverse proxy on `:9300`
   - Routes `/api/*` to `:9000` (strips prefix)
   - Uses h2c transport for multiplexed SSE streams
3. Runs `just start-all --no-watch --no-docker --no-ui`
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
Client (:9300) → Caddy (h2c proxy) → API (:9000) → Worker (:9001)
                                        ↓
                                  PostgreSQL (:5432)
```

### SSE Through Proxy

Caddy is configured for optimal SSE streaming:
- `flush_interval -1`: No response buffering
- `transport http { versions h2c 2 }`: HTTP/2 cleartext to backend
- `read_timeout 0`: No timeout for long-lived SSE streams

This matches the production docker-compose-full.yaml setup and enables
testing SSE connection cycling behavior through a real proxy.

## Testing

```bash
# Via Caddy proxy (matches production)
curl http://localhost:9300/api/health

# Direct API access (bypasses proxy)
curl http://localhost:9000/health

# Load test through Caddy
API_URL=http://localhost:9300/api just load-test quick
```
