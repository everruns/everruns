# Everruns

Durable AI agent execution platform. Run long-running LLM agents reliably - if the service restarts, agents resume from where they left off.

## Overview

Everruns is a service that runs AI agents in the most reliable way possible. Each step in an agent run is persisted, so if the service restarts, it picks up from the last step. Built on Temporal for durable execution.

### Key Features

- **Durable execution**: Agent runs survive restarts via Temporal workflows
- **Real-time streaming**: AG-UI protocol compatible event stream
- **Management UI**: Dashboard for agents, runs, and chat

### Architecture

- **API Service**: HTTP API with SSE streaming
- **Worker Service**: Temporal workflows and activities
- **Management UI**: Next.js dashboard
- **PostgreSQL**: Metadata and event storage
- **Temporal**: Durable workflow orchestration

## Quick Start

### Prerequisites

- Docker & Docker Compose
- Rust stable toolchain
- Node.js 18+ (for UI)
- OpenAI API key (for LLM calls)

### Start Everything

```bash
# Install all dependencies (first time only)
./scripts/dev.sh init

# Start all services
./scripts/dev.sh start-all
```

Services available at:
- **UI**: http://localhost:9100
- **API**: http://localhost:9000
- **API Docs**: http://localhost:9000/swagger-ui/
- **Temporal UI**: http://localhost:8080

### Verify Installation

```bash
./scripts/dev.sh smoke-test
```

### Stop Services

```bash
./scripts/dev.sh stop-all
```

## Usage

### Create an Agent

1. Open http://localhost:9100/agents
2. Click "New Agent"
3. Enter name and select model (e.g., `gpt-5.1`)
4. Create a version with a system prompt

### Chat with an Agent

1. Open http://localhost:9100/chat
2. Select your agent
3. Send a message and watch the response stream

### API Example

```bash
# Create an agent
curl -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "My Agent", "default_model_id": "gpt-5.1"}'

# Create a thread and start a run
curl -X POST http://localhost:9000/v1/threads -H "Content-Type: application/json" -d '{}'
curl -X POST http://localhost:9000/v1/runs \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "...", "agent_version": 1, "thread_id": "..."}'

# Stream events
curl http://localhost:9000/v1/runs/{run_id}/events
```

## Development

```bash
./scripts/dev.sh <command>

# Lifecycle
start-all     # Start everything
stop-all      # Stop all services
start         # Start Docker services only
stop          # Stop Docker services

# Services
api           # Start API server
ui            # Start UI dev server

# Database
migrate       # Run migrations
reset         # Reset database

# Quality
check         # Run format, lint, tests
smoke-test    # Run smoke tests
```

## Project Structure

```
everruns/
├── apps/ui/              # Next.js Management UI
├── crates/
│   ├── everruns-api/     # HTTP API (axum)
│   ├── everruns-worker/  # Temporal worker
│   ├── everruns-contracts/  # DTOs and events
│   └── everruns-storage/ # Database layer
├── harness/              # Docker Compose
├── specs/                # Specifications
└── scripts/              # Dev scripts
```

## Configuration

Create a `.env` file:

```bash
OPENAI_API_KEY=sk-...
DATABASE_URL=postgres://everruns:everruns@localhost:5432/everruns
```

## License

MIT
