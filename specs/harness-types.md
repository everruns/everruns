# Harness Types Specification

## Abstract

Harnesses define the base environment and capabilities for sessions. Everruns ships built-in harness types that cover common use cases. Users can also create custom harnesses via the API.

Harnesses may also define starter files. Starter files are copied into each new session created from the harness and can be marked editable or read-only per file.

Harnesses support single-parent inheritance via `parent_harness_id`. Inheritance is live: the effective harness is resolved from parent to child at runtime and in preview.

## Harness Naming

Every harness has two name fields:

| Field | Purpose | Constraints | Example |
|-------|---------|-------------|---------|
| `name` | URL/CLI-friendly addressable identifier | `[a-z0-9]+(-[a-z0-9]+)*`, max 64 chars, unique per org | `deep-research` |
| `display_name` | Human-readable label shown in UI | Free-form string, max 2 KB | `Deep Research` |

The `name` field works like a GitHub repository name: lowercase alphanumeric with hyphens, no consecutive hyphens, no leading/trailing hyphens. It is unique per organization (among non-deleted harnesses) and can be used for API lookups, CLI references, and URL routing.

### Name-based access

- **GET /v1/harnesses/{id_or_name}** — Accepts either a prefixed ID (`harness_...`) or a stable name (`generic`). The server tries parsing as ID first, then falls back to name lookup.
- **POST /v1/sessions** — Accepts `harness_name` as an alternative to `harness_id`. The two fields are mutually exclusive.
- **CLI** — `everruns sessions create --harness generic` accepts both IDs and names. Non-ID values are resolved via `GET /v1/harnesses/{name}` before session creation.

## Built-in Harness Types

### Base

The empty harness. No capabilities, no opinions. Intended as a blank canvas for fully custom configurations where users attach capabilities manually.

| Property | Value |
|----------|-------|
| Name | `base` |
| Display Name | Base |
| System Prompt | "You are a helpful assistant." |
| Capabilities | _(none)_ |
| Tags | `base`, `built-in` |

**Use cases:**
- Custom agent configurations where only specific capabilities are needed
- Testing capability behavior in isolation
- Minimal-overhead sessions with no tools

### Generic

The recommended default harness. Bundles the core capabilities needed for general-purpose agent work: file system access, bash execution, secret management, session metadata, and project instructions.

| Property | Value |
|----------|-------|
| Name | `generic` |
| Display Name | Generic |
| System Prompt | "You are a helpful assistant." |
| Tags | `generic`, `default`, `built-in` |

**Capabilities:** See `crates/server/src/seed.rs` for the full Generic harness capability list and configuration.

**Use cases:**
- Default harness for most agents
- Agents that need file manipulation and code execution
- Agents that store API keys or credentials in session secrets
- General-purpose assistant workflows

### Platform Chat

Conversational harness for the global chat interface. Extends Generic capabilities with platform management tools, tagged separately to support the per-user singleton session pattern.

| Property | Value |
|----------|-------|
| Name | `platform-chat` |
| Display Name | Platform Chat |
| Parent | `generic` |
| System Prompt | See `crates/server/src/seed.rs` (CHAT_HARNESS) for full prompt |
| Tags | `chat`, `built-in` |

**Effective capabilities:** Generic capabilities plus `platform_management`. See `crates/server/src/seed.rs` for details.

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
| Why include `infinity_context` in Generic? | General-purpose sessions often grow long. Including long-context support by default keeps the prompt bounded without permanently hiding earlier conversation state. |
| Why support harness inheritance? | It lets users build on Generic or other shared harnesses without duplicating long capability lists, prompts, model defaults, or starter files. |
| How does harness inheritance merge? | System prompt appends parent then child. Default model falls back parent then child override. Capabilities merge by capability ID with child config overriding parent. Starter files merge by normalized path with child overriding parent. |
| Can users create additional harnesses? | Yes, via `POST /v1/harnesses`. Built-in harnesses are readonly; users can copy them for editable versions. |
| Why are built-in harnesses readonly? | Prevents accidental modification of system-managed definitions. Copy-to-edit pattern gives users full control while keeping built-ins stable and upgradeable. |
| How are built-in harnesses upgraded? | Reconciliation runs at startup — iterates all orgs and upserts built-in harness definitions. Changes to `org_init::BUILT_IN_HARNESSES` propagate automatically. |
| How do starter files interact with capabilities? | Starter files are first-class harness or agent data, not capability config. If starter files exist, `session_file_system` is automatically retained so the session has a visible workspace and file tools. |

## Built-in Harness Lifecycle

Built-in harnesses are managed by `crates/server/src/org_init.rs`:

1. **Org initialization**: When a new org is created (API or seed), `initialize_org_harnesses()` provisions all built-in harnesses with `is_built_in = true`.
2. **Reconciliation**: On server startup, `reconcile_built_in_harnesses()` ensures all orgs have up-to-date built-in harnesses. New definitions are created; changed definitions are updated.
3. **Readonly protection**: API rejects update/delete on harnesses with `is_built_in = true`. Users can copy built-in harnesses to get editable versions.
4. **Default org**: Uses fixed seed UUIDs (backward compat). Other orgs get fresh UUIDs.

### UUID Allocation (Default Org Only)

Harness seed UUIDs occupy the `0x600-0x6FF` range. These fixed UUIDs are only used for the default org; other orgs get auto-generated UUIDs. See `crates/server/src/seed.rs` for seed harness definitions and capability assignments.

## Future Harness Types

The harness type system is designed for extension. Planned additions:
- **Research** — Web fetch, todo list, file system for research workflows
- **Data** — SQL database, file system, sample data for analytics
- **Code** — Docker/sandbox execution with file system for coding tasks
