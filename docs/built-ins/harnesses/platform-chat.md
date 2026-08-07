---
title: Platform Chat Harness
description: Extends the Generic harness with catalog-backed platform tools for the global chat interface.
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
| [Platform](/capabilities/platform/) | `discover`, read-only `query`, and mutating `execute` over the authoritative Everruns command catalog |

Platform Chat discovers current command names and schemas before acting. It
uses `query` for inspection, `execute` only for requested mutations, and then
queries the final state. For recurring autonomous work it creates an Agent
Trigger rather than scheduling the Platform Chat session.

## See Also

- [Generic Harness](/built-ins/harnesses/generic/) — the base this harness extends
- [Platform capability](/capabilities/platform/) — the additional capability
- [Harnesses feature guide](/features/harnesses/) — harness selection and API management
