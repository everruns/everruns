---
type: Specification
title: "Framework Application API Boundaries"
description: "Application-facing composition, low-level host boundaries, and runtime compatibility policy."
tags:
  - everruns
  - framework
  - rust
  - compatibility
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

`everruns-runtime` remains a supported `0.17.x` compatibility API. It is also
the temporary owner of reusable host implementation, but that ownership does
not make its backend entities part of the Framework application model.

## Promoted application concerns

The Framework owns value-first configuration for:

- agent instructions, models, providers, tools, and capability references;
- editable and read-only initial files plus an optional real-disk workspace;
- scoped HTTP or local-process MCP servers;
- local plugin directory loading and non-fatal compile warnings;
- multi-turn execution, events, cancellation, context inspection, and
  credential-free model identity;
- canonical, lossless session events and lifecycle hooks through curated
  application values rather than engine or worker phase records;
- high-level context-compaction behavior without checkpoint-store plumbing;
- high-level task, background-message, wake, and workspace-policy behavior
  without route-claiming or filesystem-backend plumbing;
- an advanced capability-author SPI with typed schemas, narrow call context,
  progress, cancellation, and structured errors;
- an opt-in local profile that combines real workspace files with local
  task/schedule state;
- event-derived session history and resume, without promoting writable message
  stores or their file format into the application API.

These APIs adapt into the same in-process host, provider registry, model
selection, plugin compiler, MCP client, and engine execution used by the
compatibility crate. The implementation and downstream acceptance fixtures are
linked from the [source index](#source-index).

## Supported dependency paths

An ordinary application targets `everruns` alone for agent configuration and
execution. The facade-only acceptance fixture enforces that boundary and is
intentionally stricter than the requirement for a complete execution host.

An advanced system integrator may combine `everruns` with focused host, MCP,
provider, and integration crates. That modular composition is healthy: success
means the host no longer imports `everruns-runtime`, not that every transport,
backend, or integration is re-exported by one facade. During the `0.17.x`
transition, host-only contracts remain available through the compatibility
crate until their neutral host owner lands atomically.

## Audited application and host surfaces

The inventory covers the public [repository README](../../README.md),
[runtime README](../../crates/runtime/README.md),
[runtime skill](../../skills/everruns-runtime/SKILL.md),
[runtime guide](../../docs/features/runtime.mdx), and
[embedding guide](../../docs/advanced/embedding-everruns.md). It also includes
the in-process, provider, inspection, real-disk, plugin, mount, and Lua examples
under [the runtime examples](../../crates/runtime/examples/in_process_runtime.rs)
and the provider-facing [OpenAI README](../../crates/openai/README.md).

Repository consumers were audited separately because they exercise topologies
that examples do not: the offline
[weekend concierge](../../examples/weekend-concierge-host/src/lib.rs), the
[generic evaluation runner](../../evals/generic/src/subject.rs), the
[Lua-versus-bash research harness](../../research/lua-vs-bash/src/main.rs), the
[worker host](../../crates/worker/src/unified_worker.rs), the
[local host builder](../../crates/local/src/runtime_builder.rs), and the
[live subagent host test](../../crates/llm-tests/tests/subagent_live_test.rs).
The classification below is the durable decision for each family; individual
implementation tests inside `crates/runtime` remain compatibility coverage, not
additional application entrypoints.

## Deliberately low-level host concerns

The following stay on the host side rather than being mirrored into ordinary
application configuration:

- replacing individual harness, agent, session, message, provider, event,
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
remove. Host implementations may continue using the compatibility crate in
`0.17.x`; migration of internal consumers and implementation ownership is a
separate atomic step.

## Classification of existing public use cases

| Audited use case | Classification | Why / Framework mapping |
|---|---|---|
| Runtime README, skill, and documentation quickstarts | Promote | Ordinary agent/model/session execution is the Framework's primary path. |
| Built-in simulation and real or custom model providers | Promote | Applications select a credential-free model and attach provider configuration or a custom protocol driver without constructing a platform registry. |
| Application-defined function tools and initial files | Promote | These are agent behavior and workspace inputs, not host entities. |
| In-process and OpenAI runtime examples | Promote | They map to normal agent construction and one or more Framework sessions. |
| Context-inspection example and evaluation assertions | Promote | Applications receive a curated next-turn context rather than stored records or backend assembly types. |
| Real-disk filesystem and agent-instruction examples | Promote | A single owned workspace root plus editable/read-only seed files is an application concern and retains the runtime filesystem boundary. |
| Plugin-directory example | Promote | Local plugin compilation configures an agent; non-fatal warnings stay inspectable. |
| Scoped HTTP and stdio MCP configuration | Promote | MCP servers are application integrations; stdio remains an opt-in single-tenant feature. |
| Weekend-concierge offline host | Promote | Its seeded harness/agent/session shape is ordinary Framework agent, tool, file, and session composition. |
| Generic evaluation runner | Promote | A custom provider/model, fresh session per sample, workspace seeding/reads, events, and context assertions all have Framework equivalents. |
| Event-derived local history and resume | Promote | Conversation identity and restart continuation are application behavior; canonical events are the durable truth and history is a rebuildable projection. |
| Explicit JSONL message-store APIs | Keep for `0.17.x` compatibility | They remain source-compatible for existing callers but are legacy dual-write machinery, not the durable Framework ownership model. |
| Context compaction policy | Promote | Applications choose high-level strategy and proactive budget; durable checkpoint storage remains host-owned. |
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

No audited use case is removed as obsolete in this compatibility unit. The
classification changes the recommended entrypoint, not the availability of the
runtime API.

## Compatibility constraints

- The runtime compatibility surface remains published and usable throughout
  `0.17.x`.
- Old and new setup paths must converge before provider resolution and engine
  execution; no parallel provider or turn semantics are allowed.
- Canonical events are the sole durable conversation record. Message/context
  history is a read projection; new Framework profiles must not select or own a
  writable message store.
- Promoting an application concern must not expose credentials, tenant records,
  backend stores, or host lifecycle entities.
- Local-process MCP stays opt-in and must not enter hosted builds by default.
- Real workspaces keep the runtime filesystem's traversal and symlink-escape
  protections.
- Local task/schedule state is opt-in. Schedule delivery remains an explicitly
  managed host lifecycle with at-least-once semantics.
- Removal or compiler deprecation of `everruns-runtime`, and any post-`0.18`
  cutover work, are outside this contract.

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
- `crates/everruns/src/compaction.rs`
- `crates/everruns/src/session.rs`
- `crates/everruns/src/context.rs`
- `crates/everruns/src/mcp.rs`
- `crates/everruns/src/plugin.rs`
- `crates/everruns/src/local.rs`
- `crates/everruns/tests/application_parity.rs`
- `examples/coding-cli/tests/application_parity.rs`
- `crates/runtime/src/runtime.rs`
- `crates/local/`
