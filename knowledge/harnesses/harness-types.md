---
type: Specification
title: "Harness Types Specification"
description: "Built-in harness types."
tags:
  - everruns
  - harnesses
---
# Harness Types Specification

## Abstract

Harnesses define the base environment and capabilities for sessions. Everruns ships built-in harness types that cover common use cases. Users can also create custom harnesses via the API.

Harnesses may also define starter files. Starter files are copied into each new session created from the harness and can be marked editable or read-only per file.

Harnesses support single-parent inheritance via `parent_harness_id`. Inheritance is live: the effective harness is resolved from parent to child at runtime and in preview.

The base system prompt is **optional**. A harness may contribute no base prompt of its own, common for a child harness whose only job is to add capabilities or MCP servers, in which case the effective prompt is composed entirely from the parent harness (if any), the agent, the session, and capability contributions. Empty or whitespace-only prompts are treated as "no contribution" by the prompt-composition layer, so the column is nullable and the create/preview APIs accept an absent `system_prompt`.

## Harness Naming

Every harness has two name fields:

| Field | Purpose | Constraints | Example |
|-------|---------|-------------|---------|
| `name` | URL/CLI-friendly addressable identifier | `[a-z0-9]+(-[a-z0-9]+)*`, max 64 chars, unique per org | `deep-research` |
| `display_name` | Human-readable label shown in UI | Free-form string, max 2 KB | `Deep Research` |

The `name` field works like a GitHub repository name: lowercase alphanumeric with hyphens, no consecutive hyphens, no leading/trailing hyphens. It is unique per organization (among non-deleted harnesses) and can be used for API lookups, CLI references, and URL routing.

### Name-based access

- **GET /v1/harnesses/{id_or_name}**: Accepts either a prefixed ID (`harness_...`) or a stable name (`generic`). The server tries parsing as ID first, then falls back to name lookup.
- **POST /v1/agents / PATCH /v1/agents/{id}**: Accepts `harness_name` as an alternative to `harness_id`. The two fields are mutually exclusive. Agent create inherits the organization's current default (then the platform built-in fallback) when both are omitted; an explicit selection remains pinned. Update leaves the existing selection unchanged when both are omitted.
- **POST /v1/sessions**: Accepts `harness_name` as an alternative to `harness_id` (mutually exclusive), and `agent_name` as an alternative to `agent_id` (mutually exclusive). When an agent is supplied and no harness is, the session runs on the agent's harness. Harness precedence: explicit request harness → agent's harness → org default → built-in fallback.
- **GET /v1/agents**: Includes the effective harness for each agent and whether it is explicit or inherited, so collection views reflect the harness a new agent session will use.
- **CLI**: `everruns sessions create --harness generic` accepts both IDs and names. `--agent` accepts an id or a name; a bare name resolves server-side, and omitting `--harness` runs the session on the agent's harness. Non-ID harness values are resolved via `GET /v1/harnesses/{name}` before session creation.

## Default Built-in Harnesses vs Harness Examples

Two categories of code-defined harnesses exist:

| Category | What it is | Where it lives | Adoption |
|----------|-----------|----------------|----------|
| **Default built-ins** | Platform-essential harnesses provisioned automatically into every org with `is_built_in = true`. | `crates/server/src/harnesses/{base,generic,platform_chat}.rs`, collected by `built_in_harnesses()`. | Created on org initialization. Read-only by default. |
| **Harness examples** | Adoptable templates the user opts into when needed. Shown in the UI gallery. | `crates/server/src/harnesses/examples.rs` (catalogue) plus the per-harness module file (e.g. `data_analyst.rs`). | Listed via `GET /v1/harness-examples`; adopted via `POST /v1/harnesses/import?from-example={name}`, which creates a normal `is_built_in = false` row that the user can edit. |

**Default built-ins are the minimum required for the platform to function:**

- `base`, required as a fallback parent for sessions without an explicit harness.
- `generic`, required as the default parent harness referenced by examples and most user harnesses.
- `platform-chat`, required by the global chat path (singleton per-user session pattern).

**Harness examples** today: `coding-daytona`, `coding-container`, `data-analyst`. Examples whose required capabilities are not registered for the deployment (for example, `coding-container` when the `container_sandbox` plugin is disabled) are filtered out of `/v1/harness-examples` automatically, mirroring the agent examples behaviour.

**Why the split:** specialised harnesses like `data-analyst` were previously installed into every org by default, polluting fresh installs and creating reconciliation churn for orgs that never used them. Moving them to examples keeps fresh orgs lean while making the same templates discoverable from the UI gallery on demand.

### Migration of legacy built-in rows

Existing orgs that already had `data-analyst`, `coding-daytona`, or `coding-container` provisioned as `is_built_in = true` keep those rows during reconciliation. The reconciliation step demotes them to regular org-owned harnesses (`is_built_in = false`) so existing sessions and agents that reference them keep working, while users gain the ability to edit, archive, or delete them like any custom harness. The demotion is idempotent and only flips the `is_built_in` flag, no data is rewritten and no UUIDs change.

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

Conversational harness for the global chat interface. Inherits Generic capabilities, adds `platform`, and is tagged separately to support the per-user singleton session pattern.

| Property | Value |
|----------|-------|
| Name | `platform-chat` |
| Display Name | Platform Chat |
| Parent | `generic` |
| System Prompt | See `crates/server/src/harnesses/platform_chat.rs` for full prompt |
| Tags | `chat`, `built-in` |

**Effective capabilities:** Inherits Generic harness capabilities and adds local `platform`.

**Authorization rule:** Do not remove `platform` from Platform Chat to paper over authorization bugs. Platform tools must reload the session owner and enforce that caller's permissions via the normal command/policy path.

**System prompt guidance includes:**
- "Run agent" workflow: create session → send message → wait for idle → get results
- Catalog workflow: `discover` unknown commands → `query` state → `execute`
  requested changes → `query` final state
- Recurring autonomous workflow: create an Agent Trigger; never schedule the
  Platform Chat session itself
- Prefer built-in Generic harness over creating new ones
- Confirm before creating harnesses or agents; use common sense for sessions

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
| Why include `infinity_context` in Generic? | General-purpose sessions often grow long. Including long-context support by default keeps the prompt bounded without permanently hiding earlier conversation state. |
| Why support harness inheritance? | It lets users build on Generic or other shared harnesses without duplicating long capability lists, prompts, model defaults, or starter files. |
| How does harness inheritance merge? | System prompt appends parent then child (empty/absent layers contribute nothing; if no layer contributes, the harness has no base prompt). Default model falls back parent then child override. Capabilities merge by capability ID with child config overriding parent. Starter files merge by normalized path with child overriding parent. |
| Why is the system prompt optional? | A harness bundles capabilities, MCP servers, model defaults, network access, and starter files, not just a prompt. Forcing a base prompt made capability-only or MCP-only child harnesses invent filler or duplicate the parent prompt. The composition layer already treats each layer's prompt as optional, so storage and the create/preview APIs allow it to be absent. |
| Can users create additional harnesses? | Yes, via `POST /v1/harnesses`. Built-in harnesses are readonly; users can copy them for editable versions. |
| Why are built-in harnesses readonly? | Prevents accidental modification of system-managed definitions. Copy-to-edit pattern gives users full control while keeping built-ins stable and upgradeable. |
| How are built-in harnesses upgraded? | Reconciliation runs at startup, iterates all orgs and upserts built-in harness definitions. Changes to `org_init::BUILT_IN_HARNESSES` propagate automatically. |
| How do starter files interact with capabilities? | Starter files are first-class harness or agent data, not capability config. If starter files exist, `session_file_system` is automatically retained so the session has a visible workspace and file tools. |

## Built-in Harness Identity

**Built-in harnesses are identified by `name`, not by UUID.** The `harness_id` row is per-org and is generated when the harness is provisioned. Reconciliation, lookups, tests, examples, and migrations must address built-in harnesses (`is_built_in = true`) by `name`.

**Hardcoded UUID literals must not be used to address built-in harnesses.** This rule applies in production code, tests, examples, fixtures, and new migrations. It does not affect literals used for non-built-in harness fixtures (e.g. a test that creates an org-owned custom harness with a fixed ID for setup), those are unaffected because they do not depend on the built-in identity.

The single exception is the historical default-org seed range (see "UUID Allocation" below). Those literals exist purely to keep already-provisioned default-org rows stable and must not grow.

## Built-in Harness Lifecycle

Built-in harnesses are managed by `crates/server/src/org_init.rs`:

1. **Org initialization**: When a new org is created (API or seed), `initialize_org_harnesses()` provisions all built-in harnesses with `is_built_in = true`.
2. **Reconciliation**: On server startup, `reconcile_built_in_harnesses()` ensures all orgs have up-to-date built-in harnesses. New definitions are created; changed definitions are updated.
3. **Readonly protection**: API rejects update/delete on harnesses with `is_built_in = true`. Users can copy built-in harnesses to get editable versions.
4. **Default org**: Uses fixed seed UUIDs (backward compat). Other orgs get fresh UUIDs.

### UUID Allocation (Default Org Only)

Harness seed UUIDs occupy the `0x600-0x6FF` range. These fixed UUIDs exist solely so historical default-org rows stay stable across upgrades; they are an implementation detail of org seeding and are not addressable identity. New code, tests, and migrations must always use name-based lookups. See `crates/server/src/seed.rs` for seed harness definitions and capability assignments.

## Harness Examples (adoptable templates)

These templates are not auto-installed. They appear in the UI gallery (`/harnesses`, `/harnesses/examples`) and are adopted via `POST /v1/harnesses/import?from-example={name}`. Adoption creates a normal `is_built_in = false` row in the caller's org, inheriting from the org's `generic` harness by name (no hardcoded UUIDs). Required capabilities must be registered for the deployment, otherwise the example is filtered out of the listing and import returns 400.

### Data Analyst

Data analysis harness with SQL databases, persistent memory, interactive charts via OpenUI, and a structured analysis pipeline inspired by OpenAI's Kepler data agent and the open-source [Dash](https://github.com/agno-agi/dash) project. Inherits from Generic.

| Property | Value |
|----------|-------|
| Name | `data-analyst` |
| Display Name | Data Analyst |
| Parent | `generic` |
| System Prompt | Structured 6-step analysis pipeline (recall → inspect → plan → execute → visualize → learn) |
| Tags | `data`, `sql`, `analytics`, `built-in` |

**Effective capabilities:** Generic capabilities plus:

| Capability | What it provides |
|------------|-----------------|
| Session SQL Database | `sql_execute`, `sql_query`, `sql_schema`, session-scoped SQLite databases |
| Memory | `remember`, `recall`, `forget`, cross-session persistent learning (passive recall: 8) |
| OpenUI | Rich charts, tables, dashboards via OpenUI Lang |
| Todo List | `write_todos`, multi-step analysis task tracking |
| Data Knowledge | Mounts `/knowledge/{tables,business,queries}/` scaffold for curated context |

**Use cases:**
- Natural-language data analysis (NL-to-SQL)
- Interactive data exploration with visualization
- Agents that learn from corrections across sessions
- Analytics workflows with curated knowledge bases

### Coding (Daytona)

Coding harness with Daytona cloud sandboxes (real filesystem, full process execution, git integration) plus GitHub Scout subagents for repository exploration. Inherits from Generic. Visible only when the `daytona` capability plugin is registered (the OSS deployment registers it whenever the integration is built in). See `crates/server/src/harnesses/coding_daytona.rs`.

### Coding (Container)

Coding harness backed by self-hosted Docker container sandboxes plus GitHub Scout subagents for repository exploration. Inherits from Generic. Visible only when the `container_sandbox` capability plugin is registered (gated by the `FEATURE_CONTAINER_SANDBOX` flag). See `crates/server/src/harnesses/coding_container.rs` for the harness definition and [`crates/container-sandbox/SPEC.md`](../../crates/container-sandbox/SPEC.md) for the underlying capability spec.

## Future Harness Types

The harness type system is designed for extension. Planned additions:
- **Research**: Web fetch, todo list, file system for research workflows
- **Code**: Docker/sandbox execution with file system for coding tasks
