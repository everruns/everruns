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

| Capability ID | Name | Purpose | Config |
|---------------|------|---------|--------|
| `session_file_system` | File System | Read, write, list, grep, delete files in `/workspace` | |
| `virtual_bash` | Virtual Bash | Sandboxed bash shell for code execution and scripting | |
| `web_fetch` | Web Fetch | Fetch web content with file download support | `{"enable_file_download": true}` |
| `session_storage` | Storage | Key/value store and encrypted secret storage | |
| `session` | Session | Session info access and title management | |
| `agent_instructions` | AGENTS.md | Reads AGENTS.md from workspace and injects into system prompt | |
| `skills` | Agent Skills | Discover and activate skills from `/.agents/skills/` in session filesystem | |
| `infinity_context` | Infinity Context | Trims older messages from the live prompt while exposing `query_history` for long sessions | |
| `openai_tool_search` | OpenAI Tool Search | Defers tool schema loading on supported OpenAI models | |

**Use cases:**
- Default harness for most agents
- Agents that need file manipulation and code execution
- Agents that store API keys or credentials in session secrets
- General-purpose assistant workflows

### Platform Chat

Conversational harness for the global chat interface. Extends Generic capabilities with platform management tools, tagged separately to support the per-user singleton session pattern.

| Property | Value |
|----------|-------|
| Name | Platform Chat |
| Seed ID | `harness_01933b5a000070008000000000000603` |
| System Prompt | See `crates/server/src/seed.rs` (CHAT_HARNESS) for full prompt |
| Tags | `chat`, `seed` |

**Capabilities:** Generic capabilities plus `platform_management`:

| Capability ID | Name | Purpose | Config |
|---------------|------|---------|--------|
| `session_file_system` | File System | Read, write, list, grep, delete files in `/workspace` | |
| `virtual_bash` | Virtual Bash | Sandboxed bash shell for code execution and scripting | |
| `web_fetch` | Web Fetch | Fetch web content with file download support | `{"enable_file_download": true}` |
| `session_storage` | Storage | Key/value store and encrypted secret storage | |
| `session` | Session | Session info access and title management | |
| `agent_instructions` | AGENTS.md | Reads AGENTS.md from workspace and injects into system prompt | |
| `skills` | Agent Skills | Discover and activate skills from `/.agents/skills/` in session filesystem | |
| `infinity_context` | Infinity Context | Trims older prompt history while keeping it queryable | |
| `openai_tool_search` | OpenAI Tool Search | Defers tool schema loading on supported OpenAI models | |
| `platform_management` | Platform Management | Manage harnesses, agents, and sessions via tools | |

**System prompt guidance includes:**
- "Run agent" workflow: create session → send message → wait for idle → get results
- Prefer built-in Generic harness over creating new ones
- Confirm before creating harnesses or agents; use common sense for sessions

**Use cases:**
- Global chat interface (web UI at `/chat`)
- Per-user singleton sessions via tag-based lookup
- Managing Everruns entities (harnesses, agents, sessions) directly from chat

## Design Decisions

| Question | Decision |
|----------|----------|
| Why separate Base and Generic? | Base provides a zero-capability starting point; Generic provides batteries-included defaults. Separating them lets users choose their starting point. |
| Why is Generic the recommended default? | Most agents need file system and bash access. Bundling these in a harness avoids repetitive per-agent capability setup. |
| Why include `session_storage` in Generic? | Secret storage is needed for agents that interact with external APIs (API keys, tokens). KV storage is useful for persisting state. |
| Why include `session` in Generic? | Session metadata (title, info) is commonly needed and has minimal overhead. |
| Why include `agent_instructions` in Generic? | AGENTS.md is the standard way to provide project-level instructions. Including it by default means users get this functionality without extra configuration. |
| Why include `skills` in Generic? | Skills extend agent abilities via portable instruction packages. Including discovery by default means agents can use skills uploaded to the session filesystem without extra capability setup. |
<<<<<<< HEAD
| Why include `infinity_context` in Generic? | General-purpose sessions often grow long. Including long-context support by default keeps the prompt bounded without permanently hiding earlier conversation state. |
| Can users create additional harnesses? | Yes, via `POST /v1/harnesses`. Built-in harnesses are readonly; users can copy them for editable versions. |
| Why are built-in harnesses readonly? | Prevents accidental modification of system-managed definitions. Copy-to-edit pattern gives users full control while keeping built-ins stable and upgradeable. |
| How are built-in harnesses upgraded? | Reconciliation runs at startup — iterates all orgs and upserts built-in harness definitions. Changes to `org_init::BUILT_IN_HARNESSES` propagate automatically. |

## Built-in Harness Lifecycle

Built-in harnesses are managed by `crates/server/src/org_init.rs`:

1. **Org initialization**: When a new org is created (API or seed), `initialize_org_harnesses()` provisions all built-in harnesses with `is_built_in = true`.
2. **Reconciliation**: On server startup, `reconcile_built_in_harnesses()` ensures all orgs have up-to-date built-in harnesses. New definitions are created; changed definitions are updated.
3. **Readonly protection**: API rejects update/delete on harnesses with `is_built_in = true`. Users can copy built-in harnesses to get editable versions.
4. **Default org**: Uses fixed seed UUIDs (backward compat). Other orgs get fresh UUIDs.

### UUID Allocation (Default Org Only)

Harness seed UUIDs occupy the `0x600-0x6FF` range. These fixed UUIDs are only used for the default org; other orgs get auto-generated UUIDs.

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
