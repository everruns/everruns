---
title: Architecture
description: System architecture overview for Everruns - A durable AI agent execution platform
---

Everruns is a **headless AI agent execution platform** built for reliability and scale. It provides a REST API for managing agents, sessions, and runs with real-time event streaming via SSE.

## Platform Overview

![Everruns Platform Architecture](/images/architecture/overview.png)

Your application connects to the Everruns platform via REST API and SSE. The Control Plane orchestrates Workers, which communicate with LLM providers (OpenAI, Anthropic, etc.). An optional Management UI provides admin access.

## Key Design Principles

| Principle | Description |
|-----------|-------------|
| **Headless / API-First** | Integrate via REST API. No UI required for production. |
| **Agentic Loop** | Core pattern: Reason → Act, repeated until task complete. |
| **Durable Execution** | Agent state survives restarts. Never lose progress. |
| **Horizontal Scaling** | Add workers to increase throughput. |
| **Provider Agnostic** | OpenAI, Anthropic, or custom LLM providers. |

## Components

### Control Plane

Central coordinator that exposes the REST API and manages all state in PostgreSQL:

- **Agents** - AI assistant configurations
- **Sessions** - Conversation state and history
- **Events** - Real-time event streaming via SSE

### Workers

Stateless executors that run the agentic loop:

![Agentic Loop](/images/architecture/agentic-loop.png)

Workers are:
- **Scalable** - Add more to handle concurrent sessions
- **Fault-tolerant** - Failed tasks automatically recovered
- **Stateless** - All state lives in PostgreSQL

### Management UI (Optional)

Administrative interface for platform operators. **Not required for production use.**

- Agent configuration
- Session monitoring
- LLM provider settings

See [Management UI](/features/ui) for details.

## Further Reading

- [Introduction](/getting-started/introduction) - Getting started
- [API Reference](/api) - Full API documentation
- [Capabilities](/features/capabilities) - Extend agent functionality
