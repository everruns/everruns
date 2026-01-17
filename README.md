# Everruns

[![Website](https://img.shields.io/badge/Website-everruns.com-blue)](https://everruns.com)
[![Docs](https://img.shields.io/badge/Docs-docs.everruns.com-green)](https://docs.everruns.com)
[![CI](https://github.com/everruns/everruns/actions/workflows/ci.yml/badge.svg)](https://github.com/everruns/everruns/actions/workflows/ci.yml)
[![Status: Vibecoding PoC](https://img.shields.io/badge/Status-Vibecoding%20PoC-orange)](https://github.com/everruns/everruns)
[![Repo: Agent Friendly](https://img.shields.io/badge/Repo-Agent%20Friendly-blue)](AGENTS.md)

> **Note:** This repository is in **Heavy Vibecoding PoC Mode**. Expect rapid changes, experimental features, and unconventional approaches as we explore ideas quickly.

Headless durable AI agent execution platform. Run long-running LLM agents reliably and scalably.

## Overview

Everruns is a service that runs AI agents in the most reliable way possible. Each step and tool call in an agent run is persisted using a PostgreSQL-backed durable execution engine.

### Key Features

- **Durable execution**: Agent sessions survive restarts via PostgreSQL-backed workflows
- **Streaming events**: Real-time SSE streaming of agent responses and tool calls
- **Management UI**: Dashboard for agents, sessions, and chat
- **Extensible capabilities**: Add tools and behaviors to agents via modular capabilities
- **Multi-provider support**: OpenAI, Anthropic, and more

## Quick Start

Deploy Everruns with Docker Compose:

```bash
# Download docker-compose file
mkdir everruns && cd everruns
curl -o docker-compose.yaml https://raw.githubusercontent.com/everruns/everruns/main/examples/docker-compose-full.yaml

# Generate encryption key for secrets
python3 -c "import os, base64; print('kek-v1:' + base64.b64encode(os.urandom(32)).decode())"

# Create .env with your key
echo "SECRETS_ENCRYPTION_KEY=kek-v1:<your-key>" > .env

# Start services
docker compose up -d
```

Access the platform:
- **Web UI**: http://localhost:8080
- **API Docs**: http://localhost:8080/swagger-ui/

For detailed setup instructions, see the [Docker Compose Quickstart](https://docs.everruns.com/getting-started/docker-compose/).

## Documentation

- [Getting Started](https://docs.everruns.com/getting-started/introduction/) - Introduction and key concepts
- [Docker Compose Quickstart](https://docs.everruns.com/getting-started/docker-compose/) - Full deployment guide
- [API Reference](https://docs.everruns.com/api/) - Complete API documentation
- [Capabilities](https://docs.everruns.com/features/capabilities/) - Extend agent functionality

## API Example

```bash
# Create an agent
curl -X POST http://localhost:8080/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "Assistant", "system_prompt": "You are a helpful assistant."}'

# Create a session
curl -X POST http://localhost:8080/api/v1/agents/{agent_id}/sessions

# Send a message
curl -X POST http://localhost:8080/api/v1/sessions/{session_id}/messages \
  -H "Content-Type: application/json" \
  -d '{"content": [{"type": "text", "text": "Hello!"}]}'
```

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for local development setup and guidelines.

## License

MIT
