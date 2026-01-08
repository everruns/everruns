# Everruns

[![Website](https://img.shields.io/badge/Website-everruns.com-blue)](https://everruns.com)

> **Note:** This repository is in **Heavy Vibecoding PoC Mode**. Expect rapid changes, experimental features, and unconventional approaches as we explore ideas quickly.

Headless durable AI agent execution platform. Run long-running LLM agents reliably and scalably.

## Overview

Everruns is a service that runs AI agents in the most reliable way possible. Each step and tool call in an agent run is persisted using a PostgreSQL-backed durable execution engine.

### Key Features

- **Durable execution**: Agent sessions survive restarts via PostgreSQL-backed workflows
- **Management UI**: Optional dashboard for agents, sessions, and chat
- **Indefinite sessions**: Sessions can receive messages continuously without terminating (not yet)

### Data Model

- **Agent**: Configuration for an agentic loop (system prompt, model, etc.)
- **Session**: An instance of conversation with an agent
- **Message**: User messages and assistant responses within a session
- **LLM Provider**: External LLM service configuration (OpenAI, Anthropic, etc.)
- **Capability**: Modular functionality that can be enabled on agents (tools, behaviors)

### Architecture

- **API Service**: HTTP API with SSE streaming
- **Worker Service**: Durable worker with gRPC client
- **Management UI**: Next.js dashboard
- **PostgreSQL**: Metadata, events, and durable execution state

## Quick Start Locally

### Prerequisites

- Docker & Docker Compose (optional, for Postgres)
- Rust stable toolchain
- Node.js 18+ (for UI)
- OpenAI API key (for LLM calls)

### Configuration

Copy `.env.example` to `.env` and configure as needed:

```bash
cp .env.example .env
```

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


### Stop Services

```bash
./scripts/dev.sh stop-all
```

## License

MIT
