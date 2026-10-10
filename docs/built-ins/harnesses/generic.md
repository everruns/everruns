---
title: Generic Harness (deprecated)
description: The default harness, bundling core capabilities for general-purpose agent sessions.
---

The **Generic** harness is deprecated. Existing agents, sessions and child harnesses retain its legacy behavior. Choose [Conversation](/built-ins/harnesses/conversation/) for dialogue, [Worker](/built-ins/harnesses/worker/) for files, skills and delegation, or [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) or [Sandbox Worker](/built-ins/harnesses/sandbox-worker/) for a shell. Generic is not an alias for Worker. It configures 25 capabilities. Together they cover file operations, command execution, web access, memory, budgeting, context management, durable tool output, citations, and runtime safeguards.

## Legacy use

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
| [File System](/capabilities/file-system/) | Read, write, edit, list, grep, stat, and delete files in the session workspace (`/workspace`) |
| [Bashkit Shell](/capabilities/bashkit-shell/) | Sandboxed bash shell for running commands, scripts, and text processing |
| [Web Fetch](/capabilities/web-fetch/) | Fetch web content with file download support |
| [Storage](/capabilities/session-storage/) | Key/value store for general data and encrypted secret storage |
| [Session](/capabilities/session/) | Access session metadata and manage session title |
| [Session Schedules](/capabilities/session-schedules/) | Create and manage cron-style schedules that wake the session |
| [AGENTS.md](/capabilities/agent-instructions/) | Reads AGENTS.md from workspace and injects project-level instructions |
| [Agent Skills](/capabilities/agent-skills/) | Discover and activate skills from `/.agents/skills/` |
| [Infinity Context](/capabilities/infinity-context/) | Trims older messages from the live prompt while exposing earlier history via `query_history` |
| [Auto Tool Search](/capabilities/tool-search/#auto-tool-search) | Defers tool schema loading to reduce prompt size, using the provider's native tool search where available |
| [Context Compaction](/advanced/compaction/) | Auto-compacts context at 85% budget via cascading strategies |
| [Budgeting](/capabilities/budgeting/) | Budget awareness in the system prompt and a `check_budget` tool, which currently returns a placeholder; use the REST budget-check endpoint for detailed status |
| [Self-Budget](/capabilities/self-budget/) | Prompt-only guidance for reasoning about a user-requested indicative budget using session usage data |
| [Ask User](/capabilities/ask-user/) | Ask the user 1–4 structured questions, or collect a credential, and wait for the answer |
| Soft Approval | Prompt-level gate asking permission before a destructive, irreversible, or outward-facing action |
| Tool Output Persistence | Persists full tool output to `/.outputs/` before truncation for lossless retrieval |
| Tool Output Distillation | Distills large tool output before it enters the context |
| [Retrieval Citations](/capabilities/citation-retrieval/) | Claim-level citations for answers drawn from retrieval feeds |
| [Citation Verification](/capabilities/citation-verification/) | Stamps faithfulness verdicts on citations (deterministic `heuristic` mode by default) |
| [Parallel Tool Calls](/capabilities/parallel-tool-calls/) | Prefers parallel tool calls where the provider supports them |
| [Message Metadata](/capabilities/message-metadata/) | Annotates messages with timestamps |
| Human Intent | Adds model-authored intent narration to each tool call for UI rendering |
| BTW | Ephemeral side-question command for the current session |
| Tool Loop Detection | Detects repeated tool loops and injects a warning to break them |
| Error Disclosure | Shows full provider error detail (`detailed` mode) |

Infinity Context and Context Compaction work together to bound the active context while retaining older history. See [Context Compaction](/advanced/compaction/#worker-harness-defaults) for details.

## See Also

- [Base Harness](/built-ins/harnesses/base/), empty harness for full control
- [Platform Chat Harness](/built-ins/harnesses/platform-chat/), focused operator chat built on Base
- [Harnesses feature guide](/features/harnesses/), harness selection and API management
