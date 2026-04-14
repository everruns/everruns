---
title: Platform Chat Harness
description: Extends the Generic harness with platform management capabilities for the global chat interface.
---

The **Platform Chat** harness extends the [Generic harness](/built-ins/harnesses/generic/) with platform management capabilities. It powers the global chat interface where users interact with agents that can manage the Everruns platform itself.

## When to Use

- Global chat interface sessions
- Agents that need to manage platform resources (agents, harnesses, providers)
- Administrative assistants that interact with the Everruns API

## Configuration

| Property | Value |
|----------|-------|
| **Type** | `platform-chat` |
| **System Prompt** | Extended prompt with platform management instructions |
| **Default Model** | None (inherits from agent or organization) |

## Bundled Capabilities

All [Generic harness capabilities](/built-ins/harnesses/generic/#bundled-capabilities) plus:

| Capability | What it provides |
|------------|-----------------|
| [Platform Management](/capabilities/platform-management/) | Tools to list and manage agents, harnesses, providers, and other platform resources |

## See Also

- [Generic Harness](/built-ins/harnesses/generic/) — the base this harness extends
- [Platform Management capability](/capabilities/platform-management/) — the additional capability
- [Harnesses feature guide](/features/harnesses/) — harness selection and API management
