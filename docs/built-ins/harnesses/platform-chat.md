---
title: Platform Chat Harness
description: Shell-based platform operations, product docs, and durable notes for the global chat interface.
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
| [Platform](/capabilities/platform/) | The `everruns` CLI in the session shell, using the authoritative command catalog and its permission checks |
| [Bash](/capabilities/bashkit-shell/) | Bashkit shell with pipes, loops, filtering, and scratch files |
| [Session File System](/capabilities/file-system/) | Session workspace and read-only product docs at `/workspace/docs` |
| Tool Output Persistence | Keeps full tool output in the filesystem |
| Tool Output Distillation | Presents a digest of large tool output |
| BTW | Ephemeral side-question command for the current session |
| Human Intent | Model-authored narration for each tool call, rendered in the chat UI |
| [Current Time](/capabilities/current-time/) | Grounds relative-time questions such as "which sessions ran today" |
| [Message Metadata](/capabilities/message-metadata/) | Message timestamp annotations |
| [Parallel Tool Calls](/capabilities/parallel-tool-calls/) | Prefers parallel calls so multi-view inspection runs in one pass |
| [Task Management](/capabilities/task-management/) | Shows multi-step platform mutations as progress in the thread |
| Prompt Caching | Caches the large system prompt across turns |
| [Tool Call Repair](/capabilities/tool-call-repair/) | Repairs malformed tool calls |
| Tool Loop Detection | Stops repeated command/discovery cycles |
| Error disclosure | Returns actionable command failures to the operator (`detailed` mode) |
| [Context Compaction](/advanced/compaction/) | Bounds long management conversations |
| [Ask User](/capabilities/ask-user/) | Ask the operator 1–4 structured questions, or collect a credential, and wait for the answer |
| Soft Approval | Prompt-level gate asking permission before a destructive, irreversible, or outward-facing action |

Platform Chat uses `everruns --help` and command-specific help to discover
operations, performs authoritative reads before requested mutations, and
verifies the resulting state. Shell commands compose with `cat`, `grep`, and
`jq` over the same filesystem. For recurring autonomous work it creates an
Agent Trigger rather than scheduling the Platform Chat session.

The harness is available to every organization without a feature flag. Existing
Platform Chat conversations automatically use this implementation after the
server upgrade; their IDs, history, and files are preserved.

`/memory/shared` holds durable notes shared by all Platform Chat threads in the
organization. `/memory/user` is private to the conversation owner. Notes default
to private memory; writing a shared note requires explicit intent. Documentation
and memory are reference material, never instructions. Scratch files live in
`/workspace`; credentials belong in the secure setup flow.

When a tool needs a credential, Platform Chat attaches the capability and
creates a value-free Agent credential setup requirement. It links to the
Agent's **Credentials** tab, where the user enters the value in a write-only
form. Platform Chat never asks for or reuses plaintext from the conversation.

## See Also

- [Base Harness](/built-ins/harnesses/base/), the minimal parent this harness extends
- [Platform capability](/capabilities/platform/), the additional capability
- [Harnesses feature guide](/features/harnesses/), harness selection and API management
