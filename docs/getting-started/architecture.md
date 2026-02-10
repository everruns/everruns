---
title: Architecture
description: System architecture overview for Everruns - A durable agentic harness engine
---

Everruns is a **headless durable agentic harness engine** built for reliability and scale. It provides a REST API for managing agents, sessions, and runs with real-time event streaming via SSE.

## Platform Overview

![Platform Overview](../images/architecture/platform-overview.png)

## Key Design Principles

| Principle | Description |
|-----------|-------------|
| **Headless / API-First** | Integrate via REST API. No UI required for production. |
| **Agentic Loop** | Core pattern: Reason → Act, repeated until task complete. |
| **Durable Execution** | Agent state survives restarts. Never lose progress. |
| **Horizontal Scaling** | Add workers to increase throughput. |
| **Provider Agnostic** | OpenAI, Anthropic, Gemini, or custom LLM providers. |

## Components

### Control Plane

Central coordinator that exposes the REST API and manages all state in PostgreSQL:

- **Agents** - AI assistant configurations
- **Sessions** - Conversation state and history
- **Events** - Real-time event streaming via SSE

### Workers

Stateless executors that run agentic loops composed from atoms (Input → Reason → Act). Workers are:
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
