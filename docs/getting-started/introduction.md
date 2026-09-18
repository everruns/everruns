---
title: Introduction
description: What Everruns is, what it provides, and where to go next.
---

Everruns is a durable agentic harness engine built on Rust. It provides APIs for
managing agents, sessions, and runs, streams events over SSE, and persists
execution state in PostgreSQL so a long-running task survives a worker restart.

## Key Concepts

### Agents

An agent is a configuration the runtime executes. Each one carries:

- A system prompt that defines its behavior
- A set of capabilities that provide tools
- Model configuration for the underlying LLM

### Sessions

Sessions represent conversations with an agent. Each session maintains:

- Conversation history
- Current execution state
- Configuration overrides

### Capabilities

A capability is a unit of agent behavior. Each one can:

- Add instructions to the system prompt
- Provide tools for the agent to use
- Modify execution behavior

See [Capabilities](/features/capabilities/) for more details.

## Getting Started

### Ways to run Everruns

- **[Everruns Cloud](https://app.everruns.com)**: the hosted Platform, open in
  early access. We run the server, database, and workers. Free for now, and you
  bring your own model provider keys.
- **[Docker Compose](/getting-started/docker-compose/)**: run the full Platform
  on infrastructure you control.
- **[Framework](/framework/)**: embed durable agents in a Rust process, with no
  separate Platform to operate.

### Quick Start

1. Deploy Everruns using the provided Docker images, or create an account on
   [Everruns Cloud](https://app.everruns.com) and skip this step
2. Configure your LLM providers via the Settings UI
3. Create an agent
4. Start sessions and interact through the API or UI

### API Access

The API is available at your deployment URL:

- **API Base**: `https://your-domain.com/api/v1/`
- **OpenAPI Spec**: `https://your-domain.com/api-doc/openapi.json`

## Architecture

Everruns uses a layered architecture:

- **API Layer**: HTTP endpoints (axum), SSE streaming
- **Core Layer**: Agent abstractions, capabilities, tools
- **Worker Layer**: Durable workflows for reliable execution
- **Storage Layer**: PostgreSQL with encrypted secrets and durable execution state

See [Architecture](/getting-started/architecture/) for how these layers interact.
