# Harness Types Specification

## Abstract

Harnesses define the base environment and capabilities for sessions. Everruns ships built-in harness types that cover common use cases. Users can also create custom harnesses via the API.

## Built-in Harness Types

### Base

The empty harness. No capabilities, no opinions. Intended as a blank canvas for fully custom configurations where users attach capabilities manually.

| Property | Value |
|----------|-------|
| Name | Base |
| Seed ID | `harness_01933b5a000070008000000000000601` |
| System Prompt | "You are a helpful assistant." |
| Capabilities | _(none)_ |
| Tags | `base`, `seed` |

**Use cases:**
- Custom agent configurations where only specific capabilities are needed
- Testing capability behavior in isolation
- Minimal-overhead sessions with no tools

### Generic

The recommended default harness. Bundles the core capabilities needed for general-purpose agent work: file system access, bash execution, secret management, session metadata, and project instructions.

| Property | Value |
|----------|-------|
| Name | Generic |
| Seed ID | `harness_01933b5a000070008000000000000602` |
| System Prompt | "You are a helpful assistant." |
| Tags | `generic`, `default`, `seed` |

**Capabilities:**

| Capability ID | Name | Purpose |
|---------------|------|---------|
| `session_file_system` | File System | Read, write, list, grep, delete files in `/workspace` |
| `virtual_bash` | Virtual Bash | Sandboxed bash shell for code execution and scripting |
| `session_storage` | Session Storage | Key/value store and encrypted secret storage |
| `session` | Session | Session info access and title management |
| `agent_instructions` | Agent Instructions | Reads AGENTS.md from workspace and injects into system prompt |
| `skills` | Agent Skills | Discover and activate skills from `/.agents/skills/` in session filesystem |

**Use cases:**
- Default harness for most agents
- Agents that need file manipulation and code execution
- Agents that store API keys or credentials in session secrets
- General-purpose assistant workflows

### Chat

Conversational harness for the global chat interface. Identical capabilities to Generic, but tagged separately to support the per-user singleton session pattern.

| Property | Value |
|----------|-------|
| Name | Chat |
| Seed ID | `harness_01933b5a000070008000000000000603` |
| System Prompt | "You are a helpful assistant." |
| Tags | `chat`, `seed` |

**Capabilities:** Same as Generic (`session_file_system`, `virtual_bash`, `session_storage`, `session`, `agent_instructions`, `skills`).

**Use cases:**
- Global chat interface (web UI at `/chat`)
- Per-user singleton sessions via tag-based lookup

## Design Decisions

| Question | Decision |
|----------|----------|
| Why separate Base and Generic? | Base provides a zero-capability starting point; Generic provides batteries-included defaults. Separating them lets users choose their starting point. |
| Why is Generic the recommended default? | Most agents need file system and bash access. Bundling these in a harness avoids repetitive per-agent capability setup. |
| Why include `session_storage` in Generic? | Secret storage is needed for agents that interact with external APIs (API keys, tokens). KV storage is useful for persisting state. |
| Why include `session` in Generic? | Session metadata (title, info) is commonly needed and has minimal overhead. |
| Why include `agent_instructions` in Generic? | AGENTS.md is the standard way to provide project-level instructions. Including it by default means users get this functionality without extra configuration. |
| Why include `skills` in Generic? | Skills extend agent abilities via portable instruction packages. Including discovery by default means agents can use skills uploaded to the session filesystem without extra capability setup. |
| Can users create additional harnesses? | Yes, via `POST /v1/harnesses`. Built-in harnesses are seed data, not the only options. |

## Seed Data

Built-in harnesses are seeded on server startup using fixed UUIDs for idempotency. They use `ON CONFLICT DO NOTHING` to avoid overwriting user modifications.

### UUID Allocation

Harness UUIDs occupy the `0x600-0x6FF` range in the seed ID schema:

| Harness | UUID (hex) |
|---------|-----------|
| Base | `0x01933b5a_0000_7000_8000_000000000601` |
| Generic | `0x01933b5a_0000_7000_8000_000000000602` |
| Chat | `0x01933b5a_0000_7000_8000_000000000603` |

## Future Harness Types

The harness type system is designed for extension. Planned additions:
- **Research** — Web fetch, todo list, file system for research workflows
- **Data** — SQL database, file system, sample data for analytics
- **Code** — Docker/sandbox execution with file system for coding tasks
