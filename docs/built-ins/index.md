---
title: Built-ins Overview
description: Built-in harness types and capabilities that ship with Everruns. Harnesses define reusable behavior; Agent Environment profiles select compute.
---

Everruns ships with built-in **harness types** and **capabilities** that provide the foundation for agent sessions.

## Harnesses

A harness defines reusable session behavior: system prompt, default model, starter files, network
policy, and bundled capabilities. Every session is assigned a harness. An Agent
[Environment profile](/features/environments/) independently selects the filesystem and compute
target for new sessions.

| Harness | Description | Capabilities |
|---------|-------------|-------------|
| [Base](/built-ins/harnesses/base/) | Empty harness, full control | None |
| [Conversation](/built-ins/harnesses/conversation/) | Default for dialogue | Context management |
| [Worker Base](/built-ins/harnesses/worker-base/) | Files, bash, project instructions | Specialized workers |
| [Worker](/built-ins/harnesses/worker/) | Skills, long context and delegation | Task coordination |
| [Generic (deprecated)](/built-ins/harnesses/generic/) | Deprecated legacy bundle | 25 configured |
| [Data Analyst](/built-ins/harnesses/data-analyst/) | SQL databases, charts, persistent memory | Worker Base + data capabilities; available as a built-in example |

[Platform Chat](/built-ins/harnesses/platform-chat/) is a managed Agent with an explicit legacy Generic binding.

See the [Harnesses feature guide](/features/harnesses/) for harness selection, API management, and the prompt stack model.

## Harness Examples

Harness examples are adoptable templates. Import them when you want a preconfigured starting point, then customize the resulting org-owned harness.

| Example | Import Name | Description |
|---------|-------------|-------------|
| Coding | `coding` | Provider-neutral coding behavior + GitHub Scout; the Agent Environment profile selects Bashkit, Daytona, or another target |
| Data Analyst | `data-analyst` | Worker Base + SQL databases, charts, persistent memory, and curated data knowledge |

## Capabilities

Capabilities are modular units that extend what an agent can do. Each can contribute tools, system prompt additions, and UI features.

Browse the full [Capabilities reference](/capabilities/) for the complete list organized by category.
