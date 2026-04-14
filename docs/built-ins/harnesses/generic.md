---
title: Generic Harness
description: The recommended default harness bundling core capabilities for general-purpose agent sessions.
---

The **Generic** harness is the recommended default for most use cases. It bundles 11 core capabilities that cover file operations, command execution, web access, memory, and context management.

## When to Use

- General-purpose assistants
- Coding and scripting tasks
- Research workflows
- Any session where you want a solid set of defaults

## Configuration

| Property | Value |
|----------|-------|
| **Type** | `generic` |
| **System Prompt** | "You are a helpful assistant." |
| **Default Model** | None (inherits from agent or organization) |

## Bundled Capabilities

| Capability | What it provides |
|------------|-----------------|
| [File System](/capabilities/file-system/) | Read, write, list, grep, and delete files in the session workspace (`/workspace`) |
| [Virtual Bash](/capabilities/virtual-bash/) | Sandboxed bash shell for running commands, scripts, and text processing |
| [Web Fetch](/capabilities/web-fetch/) | Fetch web content with file download support |
| [Storage](/capabilities/session-storage/) | Key/value store for general data and encrypted secret storage |
| [Session](/capabilities/session/) | Access session metadata and manage session title |
| [AGENTS.md](/capabilities/agent-instructions/) | Reads AGENTS.md from workspace and injects project-level instructions |
| [Agent Skills](/capabilities/agent-skills/) | Discover and activate skills from `/.agents/skills/` |
| [Infinity Context](/capabilities/infinity-context/) | Trims older messages from the live prompt while exposing earlier history via `query_history` |
| [OpenAI Tool Search](/capabilities/openai-tool-search/) | Defers tool schema loading on supported models to reduce prompt size |
| [Context Compaction](/advanced/compaction/) | Auto-compacts context at 85% budget via cascading strategies |
| Tool Output Persistence | Persists full tool output to `/.outputs/` before truncation for lossless retrieval |

Infinity Context and Context Compaction work together to keep long sessions unbounded. See [Context Compaction](/advanced/compaction/#generic-harness-defaults) for details.

## See Also

- [Base Harness](/built-ins/harnesses/base/) — empty harness for full control
- [Platform Chat Harness](/built-ins/harnesses/platform-chat/) — extends Generic with platform management
- [Harnesses feature guide](/features/harnesses/) — harness selection and API management
