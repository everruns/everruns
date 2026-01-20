---
name: no-docker-setup
description: Set up full production-like backend (PostgreSQL + API + Worker) without Docker. Use for testing durable workflows, database persistence, or when DEV_MODE is insufficient.
---

# No-Docker Setup

**Full production-like backend** without Docker: PostgreSQL + API + Worker.

## When to Use

| Mode | Use Case |
|------|----------|
| `just start-dev` | Quick testing, UI work, in-memory (no persistence) |
| **This skill** | Durable workflows, database testing, persistence needed |
| `just start-all` | Full setup with Docker (easiest if Docker available) |

## What It Starts

```
PostgreSQL (port 5432) → API (port 9000) → Worker (port 9001)
     ↓                       ↓                  ↓
  Persistent DB         HTTP + gRPC        Durable workflows
```

All components run as separate processes, just like production.

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

## Fixed Configuration

| Component | Value |
|-----------|-------|
| PostgreSQL | `/tmp/pgdata` (fresh each run) |
| Database URL | `postgres://everruns:everruns@localhost:5432/everruns` |
| API | `http://localhost:9000` |
| Worker gRPC | `localhost:9001` |

## Log Files

| Service | Location |
|---------|----------|
| API | `/tmp/api.log` |
| Worker | `/tmp/worker.log` |
| PostgreSQL | `/tmp/pgdata/pg.log` |

## Testing

```bash
# Health check
curl http://localhost:9000/health

# Run tests
cargo test
```

## Troubleshooting

```bash
# Check logs
cat /tmp/api.log
cat /tmp/worker.log

# Check ports
lsof -i :5432  # PostgreSQL
lsof -i :9000  # API
lsof -i :9001  # Worker gRPC
```
