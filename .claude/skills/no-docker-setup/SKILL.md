---
name: no-docker-setup
description: Set up full production-like backend (PostgreSQL + API + Worker) without Docker. Use for testing durable workflows, database persistence, or when DEV_MODE is insufficient.
---

# No-Docker Setup

**Full production-like backend** without Docker: PostgreSQL + API + Worker.

## When to Use

| Mode | Use Case |
|------|----------|
| `just start-dev --no-watch` | Quick testing, UI work, in-memory (no persistence) |
| **This skill** | Durable workflows, database testing, persistence needed |
| `just start-all` | Full setup with Docker (easiest if Docker available) |

## What It Does

1. Sets up fresh PostgreSQL cluster at `/tmp/pgdata`
2. Runs `just start-all --no-watch --no-docker --no-ui`
   - Runs database migrations
   - Starts API server (port 9000)
   - Starts Worker (port 9001)

## Quick Start

```bash
# Prerequisites: PostgreSQL 16+, jq, API key
sudo -E .claude/skills/no-docker-setup/scripts/start.sh
```

## Prerequisites

1. **PostgreSQL 16+** - `apt-get install postgresql-16`
2. **jq** - `apt-get install jq`
3. **API Key** - `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`
4. **Root access** - For PostgreSQL cluster initialization

Note: `sqlx` CLI is auto-installed if missing.

## Architecture

```
PostgreSQL (port 5432) → API (port 9000) → Worker (port 9001)
     ↓                       ↓                  ↓
  /tmp/pgdata           HTTP + gRPC        Durable workflows
```

## Alternative: Manual Setup

If you already have PostgreSQL running:

```bash
export DATABASE_URL="postgres://user:pass@localhost:5432/everruns"
just start-all --no-watch --no-docker --no-ui
```

## Testing

```bash
curl http://localhost:9000/health
cargo test
```
