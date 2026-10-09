---
type: Specification
title: "Harness Types Specification"
description: "Canonical harness levels, inheritance and legacy deprecation."
tags:
  - everruns
  - harnesses
---
# Harness Types

Harnesses define reusable capability foundations. An agent selects a harness; a session may override it. Choose the smallest foundation that meets the task.

## Canonical levels

- **Base** is the zero-capability blank canvas.
- **Conversation** is the default for dialogue, with context management and tool-call robustness.
- **Worker Base** adds working files, bash, project instructions and durable tool output. Specialized workers build on it.
- **Worker** adds skills, long-context support, budgeting and delegated task coordination.

The parent chain is Base → Conversation → Worker Base → Worker. Capability additions and configurations are owned by [the shared preset definition](../../crates/contracts/src/capability/presets.rs); do not duplicate its inventory here. Hosted labels and icons are owned by [levels](../../crates/server/src/harnesses/levels.rs).

Web access, secrets/KV tools, arbitrary organization memory mounts, retrieval/citations and session scheduling are opt-in. Recurring agent execution belongs to Agent Triggers. Enforced budgets, permissions, cancellation and durable execution remain infrastructure guarantees at every level.

Filesystem access includes the server-managed agent and owner memory paths described by [memory](../runtime-resources/memory.md). Omitting the organization-memory capability does not disable those paths. Sandbox policies choose the execution target independently of the harness level.

Subagents are bundled in Worker and remain independently composable on lower foundations. Ordinary children inherit their parent's harness and agent; blueprints supply specialist behavior. Existing depth and root-tree task limits apply. The task registry supplies monitoring, messaging, cancellation and waiting.

## Shared framework definition

The hosted platform and application framework consume the same capability data. Hosted rows retain live parent relationships; framework constructors flatten that same chain. [Framework Harness](../../crates/everruns/src/harness.rs) exposes the four constructors and keeps the deprecated Generic constructor's behavior. Optional integrations must still be compiled into the host. Creating a framework session without binding a harness retains the existing empty-harness behavior.

## Default and explicit selection

An explicit session harness wins, then an explicitly selected agent harness, then the organization default, then the configured built-in fallback. Agents that inherit the organization default resolve it when a new session starts. Explicit bindings stay pinned. Custom organization defaults and embedder-provided defaults are honored.

Examples declare their intended harness at import; Dad Jokes uses Conversation plus its time capability. New Coding and Data Analyst harness examples parent on Worker Base and add their specialist capabilities. Previously adopted custom examples keep their existing definitions.

## Generic deprecation

Generic is an active, managed legacy harness. It retains its stable name, UUID, capabilities and configuration, and receives no new feature additions. It is not an alias for Worker: aliasing would add delegation and remove other legacy capabilities.

The managed `deprecated` tag is its API deprecation metadata. The UI hides deprecated built-ins from ordinary selection and catalogue views, offers Show deprecated, and preserves a currently selected deprecated value. Custom tags are not platform deprecation policy. Explicit API/CLI references continue to resolve.

During startup reconciliation, organizations whose default references the built-in Generic have existing inherited agents pinned explicitly to that same Generic row before their default changes to Conversation. The old managed default tag marks the pending upgrade and is consumed atomically with the pointer change. This per-org operation is atomic and idempotent; a later explicit choice of Generic as the org default is respected. Custom defaults, explicit bindings, existing sessions, versions, triggers, child harnesses, files and memory data remain intact. Run reconciliation before accepting creation traffic. [Storage migration](../../crates/server/src/storage/repositories/harnesses/mod.rs) owns the PostgreSQL transaction; the in-memory backend implements equivalent behavior.

If a new managed name already belongs to a custom harness, reconciliation moves that custom slug to `<name>-custom` (with a numeric suffix when occupied). Preserve its ID, display name, definition, capabilities and bindings; never adopt it as a built-in or block startup on the name collision. Name-based callers must use the preserved custom slug afterward. [Reconciliation](../../crates/server/src/setup/org_init/mod.rs) owns this retryable transition.

Deprecation is independent of lifecycle. Archiving Generic would block existing execution. Removal requires a later audit of stored references and external callers; deprecation alone does not authorize deletion.

## Inheritance

A harness has one parent. Resolution folds root to leaf at runtime and preview. Prompt fragments append; capability configurations override by ID; starter files override by normalized path; model defaults fall back through the chain. Execution environment selection replaces inherited compute attachments. There is no general capability subtraction, so parents must stay small.

System prompts are optional. Empty layers contribute no prose. Agent starter files retain the filesystem capability. Harness presentation fields are deprecated and backfilled onto Agents by the Platform Chat migration. Scoped MCP servers and network access compose through the existing overlay contract. [Config composition](../../crates/contracts/src/runtime/config_layer.rs) owns exact merge behavior.

## Built-in identity and lifecycle

Built-ins are addressed by their stable per-org name, never a hardcoded UUID. Database IDs are generated per organization. Reconciliation preserves IDs and synchronizes managed definitions at startup. Built-ins remain read-only; copying creates an editable custom harness. Deprecated Generic remains managed to protect existing inherited behavior.

Only the canonical levels and deprecated Generic are automatically provisioned. Platform Chat is a separately managed Agent with an explicit Generic binding. Specialized Coding and Data Analyst harnesses are adopted from the examples catalogue when their required capabilities are available. Former provider-specific coding built-ins and Data Analyst rows are demoted to editable org-owned harnesses without changing their IDs. [Reconciliation](../../crates/server/src/setup/org_init/mod.rs) owns these transitions.

Platform Chat is a managed operator Agent over platform CLI operations, documentation and durable memory. Its explicit legacy Generic binding is preserved independently of the organization default. Its owner-based authorization contract remains unchanged.

## Sources

- [Portable preset data](../../crates/contracts/src/capability/presets.rs)
- [Hosted harnesses](../../crates/server/src/harnesses/mod.rs)
- [Framework constructors](../../crates/everruns/src/harness.rs)
- [Organization reconciliation](../../crates/server/src/setup/org_init/mod.rs)
- [Agent example import](../../crates/server/src/api/agents/mod.rs)
- [Session harness resolution](../../crates/server/src/domains/sessions/queries.rs)
