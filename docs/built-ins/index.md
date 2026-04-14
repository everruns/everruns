---
title: Built-ins Overview
description: Built-in harness types and capabilities that ship with Everruns. Harnesses define session environments; capabilities add tools and behaviors.
---

Everruns ships with built-in **harness types** and **capabilities** that provide the foundation for agent sessions.

## Harnesses

A harness defines the base environment for sessions — system prompt, default model, and bundled capabilities. Every session is assigned a harness.

| Harness | Description | Capabilities |
|---------|-------------|-------------|
| [Base](/built-ins/harnesses/base/) | Empty harness, full control | None |
| [Generic](/built-ins/harnesses/generic/) | Recommended default with core tools | 11 bundled |
| [Platform Chat](/built-ins/harnesses/platform-chat/) | Extends Generic for global chat | Generic + Platform Management |

See the [Harnesses feature guide](/features/harnesses/) for harness selection, API management, and the prompt stack model.

## Capabilities

Capabilities are modular units that extend what an agent can do. Each can contribute tools, system prompt additions, and UI features.

Browse the full [Capabilities reference](/capabilities/) for the complete list organized by category.
