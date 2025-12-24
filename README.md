# Everruns

[![Website](https://img.shields.io/badge/Website-everruns.com-blue)](https://everruns.com)

> **Note:** This repository is in **Heavy Vibecoding PoC Mode**. Expect rapid changes, experimental features, and unconventional approaches as we explore ideas quickly.

Durable AI agent execution platform. Run long-running LLM agents reliably - if the service restarts, agents resume from where they left off.

## Overview

Everruns is a service that runs AI agents in the most reliable way possible. Each step in an agent run is persisted, so if the service restarts, it picks up from the last step. Built on Temporal for durable execution.

### Key Features

- **Durable execution**: Agent sessions survive restarts via Temporal workflows
- **Management UI**: Dashboard for agents, sessions, and chat
- **Indefinite sessions**: Sessions can receive messages continuously without terminating

### Data Model

- **Agent**: Configuration for an agentic loop (system prompt, model, etc.)
- **Session**: An instance of conversation with an agent
- **Message**: User messages and assistant responses within a session
- **LLM Provider**: External LLM service configuration (OpenAI, Anthropic, etc.)
- **Capability**: Modular functionality that can be enabled on agents (tools, behaviors)

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

1. Open http://localhost:9100/agents
2. Click on an agent, then "New Session"
3. Send a message and see the response

### API Example

```bash
# Create an agent
curl -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "My Agent", "system_prompt": "You are a helpful assistant."}'

# Create a session for the agent
curl -X POST http://localhost:9000/v1/agents/{agent_id}/sessions \
  -H "Content-Type: application/json" \
  -d '{"title": "My Chat"}'

# Send a message (triggers workflow execution)
curl -X POST http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/messages \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "role": "user",
      "content": [{"type": "text", "text": "Hello, how are you?"}]
    }
  }'

# Get messages (poll for response)
curl http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/messages

# Stream events (real-time updates)
curl http://localhost:9000/v1/agents/{agent_id}/sessions/{session_id}/events
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
│   ├── everruns-api/     # HTTP API (axum), API DTOs
│   ├── everruns-worker/  # Temporal worker
│   ├── everruns-core/    # Core abstractions, domain entities, tools
│   ├── everruns-storage/ # Database layer
│   ├── everruns-openai/  # OpenAI provider
│   └── everruns-anthropic/  # Anthropic provider
├── harness/              # Docker Compose
├── specs/                # Specifications
└── scripts/              # Dev scripts
```

## Configuration

Copy `.env.example` to `.env` and configure as needed:

```bash
cp .env.example .env
```

## License

MIT
