---
type: Specification
title: "Framework Application API Boundaries"
description: "Application-facing composition, promotion decisions, and low-level host boundaries."
tags:
  - everruns
  - framework
  - rust
---

# Framework Application API Boundaries

## Intent

The Everruns Framework is the application-facing Rust library exposed by the
`everruns` crate. An application should be able to describe an agent, connect
providers and tools, configure its workspace and integrations, run sessions,
and inspect effective context without importing execution-host or storage
implementation crates. Session history and resume are application concerns,
but their durable source is the canonical event log rather than a writable
message store.

`everruns-host` is the neutral, non-application-facing implementation boundary
for shared effectful orchestration, and the only low-level host boundary. There
is no separate runtime compatibility crate.

## Promoted application concerns

The Framework owns value-first configuration for:

- agent instructions, model values that may bundle their one provider (or plain
  model ids with a separately configured provider), function tools,
  and one open capability-configuration entrypoint for typed built-ins,
  code-defined implementations, and dynamic references;
- editable and read-only initial files plus an optional real-disk workspace;
- scoped HTTP or local-process MCP servers;
- local plugin directory loading and non-fatal compile warnings;
- live message ingress with automatic active-turn steering, optional turn
  waiting, events, cancellation, context inspection, and
  credential-free model identity;
- canonical, lossless session events and lifecycle hooks through curated
  application values rather than engine or worker phase records;
- high-level context-compaction and model-adaptive tool-search behavior without
  checkpoint-store or provider-specific plumbing;
- high-level task, background-message, wake, and workspace-policy behavior
  without route-claiming or filesystem-backend plumbing;
- an open, non-sealed capability conversion contract plus a curated
  capability-author SPI with typed schemas, narrow call context, progress,
  cancellation, and structured errors;
- an opt-in local profile that combines real workspace files with local
  task/schedule state;
- event-derived session history and resume, without promoting writable message
  stores or their file format into the application API.

The application execution boundary is the concrete `everruns::Engine`.
`InMemoryEngine` remains a source-compatible alias, not a second
implementation. `Agent` is immutable behavior; an Engine owns Agent snapshots,
the session catalog, and the backend bundle needed to execute and resume those
sessions. `Session` is engine-bound and does not expose its private execution
binding, concrete in-process runtime, stores, or platform DTOs. The separate
`everruns-engine` crate owns shared Input/Reason/Act execution and turn
planning; it is the lower-level host kernel, not an application-pluggable
Engine implementation.

There is no Agent-owned compatibility engine and no `Agent::session` or
`Agent::resume` path in 0.18. Embedded applications retain an Engine and call
`engine.create(agent)` or `engine.resume(session_id)`. Because Agent behavior
may contain process-local provider drivers, function handlers, and hook
closures, local persistence never serializes it. After a process restart, an
application reconstructs that trusted behavior and explicitly attaches it to a
new `Engine` before resuming the persisted session. Attachment verifies
the ID against the Agent-configured local session catalog; canonical events do
not confer session identity.

These APIs adapt into the same in-process host, provider registry, model
selection, plugin compiler, MCP client, and engine execution that an advanced
host composes directly. The implementation and downstream acceptance fixtures
are linked from the [source index](#source-index).

## Supported dependency paths

An ordinary application targets `everruns` alone for agent configuration and
execution. The facade-only acceptance fixture enforces that boundary and is
intentionally stricter than the requirement for a complete execution host.
The default facade and `everruns-host --no-default-features` graphs do not
contain `everruns-platform`, Reqwest, Rustls, or Hyper. `everruns-host` owns
the session mutation/storage capability boundary and platform re-exports it;
hosted product services remain opt-in through
platform-enabled product composition.

An advanced system integrator may combine `everruns` with `everruns-host` and
focused engine, MCP, provider, and integration crates. `everruns-engine` is the
portable executor/planner boundary; `everruns-host` supplies deployment
composition and lifecycle I/O. That modular composition is
healthy: success means the host composes focused crates deliberately, not that
every transport, backend, or integration is re-exported by one facade.

## Audited application and host surfaces

The inventory covers the public [repository README](../../README.md),
[host README](../../crates/host/README.md),
[Everruns skill](../../skills/everruns/SKILL.md), and
[embedding guide](../../docs/advanced/embedding-everruns.md). It also includes
the in-process, inspection, real-disk, plugin, mount, and Lua examples
under [the host examples](../../crates/host/examples/in_process_runtime.rs)
and the provider-facing [OpenAI README](../../crates/drivers/openai/README.md).

Repository consumers were audited separately because they exercise topologies
that examples do not: the offline
[weekend concierge](../../examples/weekend-concierge-host/src/lib.rs), the
[generic evaluation runner](../../evals/generic/src/subject.rs), the
[Lua-versus-bash research harness](../../research/lua-vs-bash/src/main.rs), the
[worker host](../../crates/worker/src/unified_worker.rs), the
[local host builder](../../crates/everruns/src/local/runtime_builder.rs), and the
[live subagent host test](../../crates/llm-tests/tests/subagent_live_test.rs).
The classification below is the durable decision for each family; individual
implementation tests inside `crates/host` remain host coverage, not additional
application entrypoints.

## Deliberately low-level host concerns

The following stay on the host side rather than being mirrored into ordinary
application configuration:

- replacing individual harness, agent, session, provider, event,
  checkpoint, storage, task, schedule, or platform stores;
- constructing stored harness/agent/session records with stable internal ids;
- replacing the complete platform definition, capability registry, filesystem
  factory, connector registry, egress service, or utility services;
- direct mount/filesystem primitives and mutable worktree routing;
- raw assembled context records and live capability overlays;
- host phase adapters, durable planning state, lifecycle effects, and worker
  orchestration;
- attaching a custom local platform runner or starting a schedule-delivery
  daemon whose routing and lifecycle are owned by the embedding host.

These are valid extension points for server, worker, evaluation, research, or
specialized embedding hosts. Re-exporting their backend-oriented entities from
the Framework would recreate the coupling the application API is intended to
remove. Advanced integrations depend on `everruns-host` directly.

## Classification of existing public use cases

| Audited use case | Classification | Why / Framework mapping |
|---|---|---|
| Host README, skill, and documentation quickstarts | Promote | Ordinary agent/model/session execution is the Framework's primary path. |
| Built-in simulation and real or custom model providers | Promote | Applications select a model value that may carry its provider, or pair a plain credential-free model id with one provider configuration, without constructing a `ModelSpec` or platform registry. |
| Application-defined function tools and initial files | Promote | These are agent behavior and workspace inputs, not host entities. |
| In-process and OpenAI host examples | Promote | They map to normal agent construction and one or more Framework sessions. |
| Live message send, steering, and optional waiting | Promote | A Framework session is a live conversation: message acceptance is independent of turn completion, and routing to the active or next turn is decided atomically by the session. |
| Context-inspection example and evaluation assertions | Promote | Applications receive a curated next-turn context rather than stored records or backend assembly types. |
| Real-disk filesystem and agent-instruction examples | Promote | A single owned workspace root plus editable/read-only seed files is an application concern and retains the runtime filesystem boundary. |
| Plugin-directory example | Promote | Local plugin compilation configures an agent; non-fatal warnings stay inspectable. |
| Scoped HTTP and stdio MCP configuration | Promote | MCP servers are application integrations; stdio remains an opt-in single-tenant feature. |
| Weekend-concierge offline host | Promote | Its seeded harness/agent/session shape is ordinary Framework agent, tool, file, and session composition. |
| Generic evaluation runner | Promote | A custom provider/model, fresh session per sample, workspace seeding/reads, events, and context assertions all have Framework equivalents. |
| Event-derived local history and resume | Promote | Conversation identity and restart continuation are application behavior; canonical events are the durable truth and history is a rebuildable projection. |
| Context compaction policy | Promote | Applications choose high-level strategy and proactive budget; durable checkpoint storage remains host-owned. |
| Capability configuration and dynamic references | Promote | Every capability enters through one open conversion contract. Typed values expose stable built-in schemas; database/plugin catalogs use an ID plus JSON without a closed enum or host dependency. |
| Local task/schedule state used by an agent | Promote | The local profile supplies the state behind activated Framework capabilities. |
| Canonical events, post-turn sinks, and typed lifecycle hooks | Promote | Applications observe lossless values and register behavior; engine buses, worker phases, and durable event-store topology remain host-owned. |
| High-level tasks, background messages, wake, and workspace policy | Promote | Applications configure behavior and resume work without taking ownership of route claims, delivery daemons, or filesystem implementations. |
| Advanced capability authoring | Promote under a curated module | Capability authors need typed schemas, narrow context, progress, cancellation, and structured errors; backend registries and platform records stay hidden. |
| Wake routing and schedule delivery lifecycle | Host-only | Route ownership, claiming, delivery retries, and daemon shutdown require a host runner. |
| Raw custom backend/store examples | Host-only | Store topology and identity scope are host implementation choices. |
| Standalone mount/multi-root/worktree example | Host-only | It demonstrates filesystem implementation and routing rather than application agent composition. |
| Lua code-mode and Lua-versus-bash research harness | Host-only | These intentionally replace the platform/capability topology to compare execution hosts. |
| Worker, durable recovery, and phase-adapter tests | Host-only | They verify execution-host contracts below the application boundary. |
| Local subagent tests with a custom platform store/task registry/session runner | Host-only | The public local profile does not claim to own that specialized topology or its runner lifecycle. |

The classification names the owning entrypoint for each use case; host-only
rows stay reachable through `everruns-host` and its focused siblings.

## Session work boundary

Framework sessions scope application-defined background tasks and direct wakes
through the high-level work queue. Its default provider is offline and
process-local; a host can replace it behind the same application values when it
needs durable recovery. Local claim/settlement calls make leased, at-least-once
delivery observable without exposing runtime task records, store registries, or
platform constants. Distributed route ownership, recurring schedule runners,
and multi-host lifecycle management remain host concerns.

## Boundary constraints

- Application and host setup paths must converge before provider resolution and
  engine execution; no parallel provider or turn semantics are allowed.
- Canonical events are the sole durable conversation record. Message/context
  history is a read projection; new Framework profiles must not select or own a
  writable message store.
- A Framework session has at most one active turn. Sending while that turn still
  accepts input steers it; sending after its terminal boundary starts the next
  turn. The acceptance receipt is authoritative for that timing-dependent
  decision, and blocking request/response remains convenience over send plus
  wait rather than a separate execution mode.
- The host log owns coherent append and bounded snapshot replay. In-memory
  durability is process-lifetime only; JSONL acknowledges only a flushed and
  synchronized canonical envelope. Any projection index is rebuildable.
- Promoting an application concern must not expose credentials, tenant records,
  backend stores, or host lifecycle entities.
- Resume authority is engine-scoped. An in-memory engine must reject an id from
  another engine, retain the exact Agent snapshot chosen at creation, and reopen
  the exact persisted WorkspaceHead and typed Environment extensions. Scale
  portability and Agent serialization are intentionally outside this slice.
- Capability values converge through one builder entrypoint. New built-ins do
  not add builder methods, and third-party values must not require a core or
  host dependency. Dynamic references validate their open ID and JSON object
  boundary at agent build time; known implementations additionally own schema
  validation. Typed activation and configuration values live with their
  capability implementation packages; the Framework facade may re-export
  feature-enabled values but does not own a closed capability catalog.
  Duplicate references and implementation collisions are errors, never
  registry overwrites.
- Local-process MCP stays opt-in and must not enter hosted builds by default.
- Real workspaces keep the runtime filesystem's traversal and symlink-escape
  protections.
- Local task/schedule state is opt-in. Schedule delivery remains an explicitly
  managed host lifecycle with at-least-once semantics.

## Success bar

Every application surface promoted in an atomic unit has a downstream-style
compile/run fixture that imports only `everruns`, uses offline simulation and
temporary files, and does not require credentials or network access. The same
bar applies when the event/history, hooks, task/wake, workspace-policy, and
capability-SPI units land. Compatibility tests continue to exercise the old
runtime path. Product worker/server and local host behavior must remain
unchanged. Facade-only dependency guards apply to ordinary application
fixtures; advanced host fixtures instead forbid the transitional runtime
dependency while allowing focused host and sibling crates.

## Source index

- `crates/everruns/src/agent.rs`
- `crates/capability/src/lib.rs`
- `crates/everruns/src/capability_config.rs`
- `crates/everruns/src/tool_search.rs`
- `crates/everruns/src/hooks.rs`
- `crates/everruns/src/compaction.rs`
- `crates/everruns/src/session.rs`
- `crates/everruns/src/context.rs`
- `crates/everruns/src/mcp.rs`
- `crates/everruns/src/plugin.rs`
- `crates/everruns/src/local.rs`
- `crates/everruns/src/work.rs`
- `crates/everruns/tests/application_parity.rs`
- `crates/everruns/tests/capability_configuration.rs`
- `crates/everruns/tests/session_work.rs`
- `crates/everruns/tests/lifecycle_hooks.rs`
- `examples/coding-cli/tests/application_parity.rs`
- `crates/host/src/runtime.rs`
- `crates/host/src/events.rs`
- `crates/host/src/lib.rs`
- `crates/everruns/src/local/`
