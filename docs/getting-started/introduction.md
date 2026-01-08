---
title: Introduction
description: Get started with Everruns - A durable AI agent execution platform
---

Everruns is a durable AI agent execution platform built on Rust with a PostgreSQL-backed durable execution engine. It provides APIs for managing agents, sessions, and runs with streaming event output via SSE.

## Overview

Everruns enables you to build reliable AI agents that can:

- Execute long-running tasks with durability guarantees
- Stream real-time events to clients
- Manage conversations through sessions
- Extend agent capabilities with modular tools

## Key Concepts

### Agents

Agents are AI assistants with configurable system prompts and capabilities. Each agent can be customized with:

- A system prompt that defines its behavior
- A set of capabilities that provide tools
- Model configuration for the underlying LLM

### Sessions

Sessions represent conversations with an agent. Each session maintains:

- Conversation history
- Current execution state
- Configuration overrides

### Capabilities

Capabilities are modular functionality units that extend agent behavior. They can:

- Add instructions to the system prompt
- Provide tools for the agent to use
- Modify execution behavior

See [Capabilities](/features/capabilities) for more details.

## Getting Started

### Quick Start

1. Deploy Everruns using the provided Docker images
2. Configure your LLM providers via the Settings UI
3. Create an agent with your desired configuration
4. Start sessions and interact through the API or UI

### API Access

The API is available at your deployment URL with full OpenAPI documentation:

- **API Base**: `https://your-domain.com/v1/`
- **Swagger UI**: `https://your-domain.com/swagger-ui/`
- **OpenAPI Spec**: `https://your-domain.com/api-doc/openapi.json`

## Architecture

Everruns uses a layered architecture:

- **API Layer**: HTTP endpoints (axum), SSE streaming, Swagger UI
- **Core Layer**: Agent abstractions, capabilities, tools
- **Worker Layer**: Durable workflows for reliable execution
- **Storage Layer**: PostgreSQL with encrypted secrets and durable execution state

For detailed architecture information, see the [GitHub repository](https://github.com/everruns/everruns).
