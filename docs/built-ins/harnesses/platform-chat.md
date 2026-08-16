---
title: Platform Chat Harness
description: Focused catalog-backed platform tools for the global chat interface.
---

The **Platform Chat** harness is a focused operator environment built on the
empty [Base harness](/built-ins/harnesses/base/). It powers the global chat
interface where users manage Everruns through the authoritative platform
catalog.

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

| Capability | What it provides |
|------------|-----------------|
| [Platform](/capabilities/platform/) | `discover`, read-only `query`, and mutating `execute` over the authoritative Everruns command catalog |
| Loop detection | Stops repeated command/discovery cycles |
| Error disclosure | Returns actionable command failures to the operator |
| Compaction | Bounds long management conversations |

Platform Chat discovers current command names and schemas before acting. It
uses `query` for inspection, `execute` only for requested mutations, and then
queries the final state. For recurring autonomous work it creates an Agent
Trigger rather than scheduling the Platform Chat session.

Generic-purpose tools such as Bash, web fetch, session secrets, and session
schedules are intentionally absent. This keeps command selection focused and
prevents credentials or schedules from being written into the management
session when they belong to the created worker Agent.

When a tool needs a credential, Platform Chat attaches the capability and
creates a value-free Agent credential setup requirement. It links to the
Agent's **Credentials** tab, where the user enters the value in a write-only
form. Platform Chat never asks for or reuses plaintext from the conversation.

## See Also

- [Base Harness](/built-ins/harnesses/base/), the minimal parent this harness extends
- [Platform capability](/capabilities/platform/), the additional capability
- [Harnesses feature guide](/features/harnesses/), harness selection and API management
