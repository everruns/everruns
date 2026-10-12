---
type: Specification
title: "Harness Types Specification"
description: "Canonical harness tree by use case, execution rules, inheritance and legacy deprecation."
tags:
  - everruns
  - harnesses
---
# Harness Types

Harnesses define reusable capability foundations. An agent selects a harness; a session may override it. Choose the smallest foundation that meets the task.

## Canonical tree

Built-ins form a tree by use case, not one chain. Each layer answers one question, so a child never carries something its siblings must not have; single-parent inheritance has no capability subtraction.

- **Base**: what does every agent need to run reliably? Compaction, error disclosure, tool-call repair, loop detection, parallel tool calls and soft approval. Soft approval sits here because the ask_user prompt references request_approval.
- **Conversation** (Base): does the agent only talk? Adds structured questions and message timestamps. No workspace, no shell. The default.
- **Worker** (Base): does the agent need a workspace? Adds files, project instructions, durable and distilled tool output, skills, long context, budgeting, todo, subagents and session tasks. No compute: it works through tools and MCP. Worker does not inherit Conversation, so dialogue extras stay opt-in.
- **Bashkit Worker** (Worker): where does it run? In the Bashkit virtual shell, fixed to the managed Bashkit Virtual Workspace template.
- **Sandbox Worker** (Worker): in a full sandbox. No local capabilities; the Agent sandbox policy supplies the compute capability.

Capability additions and configurations are owned by [the shared preset definition](../../crates/contracts/src/capability/presets.rs); do not duplicate its inventory here. Hosted labels, icons and execution rules are owned by [levels](../../crates/server/src/harnesses/levels.rs) and the [sandbox worker definition](../../crates/server/src/harnesses/sandbox_worker.rs). The public diagram is `docs/images/features/harness-tree.svg`.

## Execution rules

Where an agent runs is the last split, so it is a property of the harness, not a capability check. Built-in definitions declare an execution rule; a custom harness inherits the rule of its nearest built-in ancestor ([queries](../../crates/server/src/domains/harnesses/queries.rs) `execution_rule`).

- Unbound (Base, Conversation, Worker): the Agent sandbox policy is free; without one, sessions get no compute.
- Fixed Bashkit: the Agent sandbox policy is rejected and sessions always use the managed Bashkit template.
- Full sandbox: a policy that only allows Bashkit is rejected, and a session with no container or managed template fails with a clear message. It never falls back to Bashkit silently, because an agent that expects real processes would misbehave.

Agent writes are validated in [command validation](../../crates/server/src/domains/agents/command_validation.rs); session creation resolves the target in [session create](../../crates/server/src/domains/sessions/service/create.rs).

## Legacy Worker split

Before the tree, Worker sat on Worker Base and included bash. Reconciliation detects that shape (Worker's parent is `worker-base`) and, in one transaction, moves agents with a container or managed policy to Sandbox Worker and everything else (other agents, the org default, custom children, triggers, sessions) to Bashkit Worker, clearing Bashkit-only agent policies. Then it rewrites Worker in place. Detection by shape keeps the step idempotent and safe to retry. Existing sessions keep their session-level compute capability. `worker-base` remains a deprecated managed row with its legacy capabilities, parented on Conversation, so custom children keep their behavior.

Web access, secrets/KV tools, arbitrary organization memory mounts, retrieval/citations and session scheduling are opt-in. Recurring agent execution belongs to Agent Triggers. Enforced budgets, permissions, cancellation and durable execution remain infrastructure guarantees at every level.

Filesystem access includes the server-managed agent and owner memory paths described by [memory](../runtime-resources/memory.md). Omitting the organization-memory capability does not disable those paths. Sandbox policies choose the execution target within the rule the harness sets.

Subagents are bundled in Worker and remain independently composable on lower foundations. Ordinary children inherit their parent's harness and agent; blueprints supply specialist behavior. Existing depth and root-tree task limits apply. The task registry supplies monitoring, messaging, cancellation and waiting.

## Shared framework definition

The hosted platform and application framework consume the same capability data. Hosted rows retain live parent relationships; framework constructors flatten that same chain. [Framework Harness](../../crates/everruns/src/harness.rs) exposes `base`, `conversation`, `worker` and `bashkit_worker`; `worker_base` is a deprecated alias of `bashkit_worker`. Sandbox Worker is hosted-only because the framework binds compute in the host and keeps the deprecated Generic constructor's behavior. Optional integrations must still be compiled into the host. Creating a framework session without binding a harness retains the existing empty-harness behavior.

## Default and explicit selection

An explicit session harness wins, then an explicitly selected agent harness, then the organization default, then the configured built-in fallback. Agents that inherit the organization default resolve it when a new session starts. Explicit bindings stay pinned. Custom organization defaults and embedder-provided defaults are honored.

Examples declare their intended harness at import; Dad Jokes uses Conversation plus its time capability. New Coding examples parent on Sandbox Worker and Data Analyst on Bashkit Worker, and both add their specialist capabilities. Previously adopted custom examples keep their existing definitions.

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

Only the canonical tree and the deprecated Worker Base and Generic rows are automatically provisioned. Platform Chat is a separately managed Agent bound to Bashkit Worker. Specialized Coding and Data Analyst harnesses are adopted from the examples catalogue when their required capabilities are available. Former provider-specific coding built-ins and Data Analyst rows are demoted to editable org-owned harnesses without changing their IDs. [Reconciliation](../../crates/server/src/setup/org_init/mod.rs) owns these transitions.

Platform Chat is a managed operator Agent over platform CLI operations, documentation and durable memory. Its Bashkit Worker binding is independent of the organization default. Its owner-based authorization contract remains unchanged.

## Sources

- [Portable preset data](../../crates/contracts/src/capability/presets.rs)
- [Hosted harnesses](../../crates/server/src/harnesses/mod.rs)
- [Framework constructors](../../crates/everruns/src/harness.rs)
- [Organization reconciliation](../../crates/server/src/setup/org_init/mod.rs)
- [Agent example import](../../crates/server/src/api/agents/mod.rs)
- [Session harness resolution](../../crates/server/src/domains/sessions/queries.rs)
