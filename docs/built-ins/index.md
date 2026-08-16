---
title: Built-ins Overview
description: Built-in harness types and capabilities that ship with Everruns. Harnesses define session environments; capabilities add tools and behaviors.
---

Everruns ships with built-in **harness types** and **capabilities** that provide the foundation for agent sessions.

## Harnesses

A harness defines the base environment for sessions, system prompt, default model, and bundled capabilities. Every session is assigned a harness.

| Harness | Description | Capabilities |
|---------|-------------|-------------|
| [Base](/built-ins/harnesses/base/) | Empty harness, full control | None |
| [Generic](/built-ins/harnesses/generic/) | Recommended default with core tools | 16 configured, including 14 user-facing defaults |
| [Data Analyst](/built-ins/harnesses/data-analyst/) | SQL databases, charts, persistent memory | Generic + 5 data capabilities; available as a built-in example |
| [Platform Chat](/built-ins/harnesses/platform-chat/) | Focused global operator chat | Platform + runtime safeguards |

See the [Harnesses feature guide](/features/harnesses/) for harness selection, API management, and the prompt stack model.

## Harness Examples

Harness examples are adoptable templates. Import them when you want a preconfigured starting point, then customize the resulting org-owned harness.

| Example | Import Name | Description |
|---------|-------------|-------------|
| Coding (Daytona) | `coding-daytona` | Generic + Daytona sandbox execution + GitHub Scout subagents for repository exploration |
| Coding (Container) | `coding-container` | Generic + self-hosted container sandbox execution + GitHub Scout subagents for repository exploration |
| Data Analyst | `data-analyst` | Generic + SQL databases, charts, persistent memory, and curated data knowledge |

## Capabilities

Capabilities are modular units that extend what an agent can do. Each can contribute tools, system prompt additions, and UI features.

Browse the full [Capabilities reference](/capabilities/) for the complete list organized by category.
