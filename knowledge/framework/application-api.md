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

`everruns-core` (`host` feature) is the neutral, non-application-facing implementation boundary
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
- alpha Engine-level push listeners over the same curated session events, with
  bounded non-blocking delivery, explicit drop statistics, and deadline-bounded
  shutdown flushing;
- opt-in OpenTelemetry and Braintrust Engine observers behind independent
  offline-by-default features, with opaque facade values and no implicit global
  telemetry installation;
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
- direct, agentless model calls over the same provider values an agent uses,
  for work that is one prompt and one answer;
- direct, agentless decision over the same `DecisionsService` the platform's
  guardrails use, for work whose answer is a number rather than prose;
- provider model catalogs and curated model metadata, so an application can
  offer a model choice instead of hard-coding ids.

The application execution boundary is the concrete `everruns::Engine`.
`InMemoryEngine` remains a source-compatible alias, not a second
implementation. `Agent` is immutable behavior; Engine retains executable builds
and delegates session identity, commands, canonical state and activation to one
coherent session hosting profile. Memory is the default; SQLite and Durable
Object profiles share the same Framework lifecycle and core actor driver. See
[Engine-owned durable sessions](engine-durable-sessions.md). `Session` is engine-bound and does not expose its private execution
binding, concrete in-process runtime, stores, or platform DTOs. The `engine` feature of
`everruns-core` owns shared Input/Reason/Act execution and turn
planning; it is the lower-level host kernel, not an application-pluggable
Engine implementation.

Accepted hosted work outlives frontend handles. Interruption inspection reads
canonical state and authority-clock ownership through the query runtime; a live
epoch or durable client wait is not an interrupted executor. Explicit resumption
reuses the original admission and existing activation obligation, without another
turn loop or a forced ownership change. See the
[session interruption adapter](../../crates/everruns/src/session/interrupted.rs).

Neutral persisted turn outcomes and approval records live in `everruns-contracts`.
Core algorithms and hosts import those owners directly; the Framework preserves
its application-facing exports without adding compatibility owners to the kernel.

There is no Agent-owned compatibility engine and no `Agent::session` or
`Agent::resume` path in 0.18. Embedded applications retain an Engine and call
`engine.create(agent)` or `engine.resume(session_id)`. Because Agent behavior
may contain process-local provider drivers, function handlers, and hook
closures, local persistence never serializes it. After a process restart, an
application reconstructs that trusted behavior. Coherent profiles register the
exact retained Agent/Harness/Environment references before activation; missing
code leaves history readable and suspends execution. Explicit legacy local
profiles attach behavior to a new Engine before resume. Legacy attachment verifies
the ID against the Agent-configured local session catalog; canonical events do
not confer session identity. A session created with an explicit Harness also
requires the application to deserialize its portable Harness definition and
attach both reconstructed values through `Engine::attach_with_harness`.
`Engine::attach` remains the no-Harness restart path.

These APIs adapt into the same in-process host, provider registry, model
selection, plugin compiler, MCP client, and engine execution that an advanced
host composes directly. The implementation and downstream acceptance fixtures
are linked from the [source index](#source-index).

## Integration execution across hosts

An external-service integration should expose an application capability through
the existing `IntoCapability` contract when its dependencies can be supplied by
the application. Its Framework and hosted adapters share protocol schemas and
the vendor operation, while each binds credentials from its owning context.
Application clients retain secrets outside serialized capability configuration;
hosted adapters retain lazy, session-scoped connection resolution. Connector UI
and inventory discovery must be separable from the application's dependency
graph. Registration alone does not supply platform persistence or orchestration.

[Brave Search](../../crates/integrations/src/brave_search/framework.rs) is the reference
implementation. Its [acceptance test](../../crates/everruns/tests/brave_search_framework.rs)
checks schema parity, execution through the public Engine, and credential
separation. Other integrations adopt this boundary as they gain Framework
support; this does not imply the entire hosted catalog is executable by default.

## Supported dependency paths

An ordinary application targets `everruns` alone for agent configuration and
execution. The facade-only acceptance fixture enforces that boundary and is
intentionally stricter than the requirement for a complete execution host.
The default facade and core host graphs do not contain `everruns-capabilities`,
Reqwest, Rustls, or Hyper. Core’s `host` feature owns the session
mutation/storage capability boundary; hosted product services remain opt-in
through product composition.

Advanced hosts use `everruns-core` with selected `engine`, `host`, `builtins`,
`mcp`, and AG-UI features, plus `everruns-contracts` for neutral provider and
extension types. Core’s host composition accepts injected services;
[`everruns::batteries`](../../crates/everruns/src/batteries.rs) selects concrete
integrations and provider wiring. These hosts never depend on control-plane
records, which belong to `everruns-server`.

## Audited application and host surfaces

The inventory covers the public [repository README](../../README.md),
[core README](../../crates/core/README.md),
[Everruns Framework skill](../../skills/everruns-framework/SKILL.md), and
[custom backends guide](../../docs/framework/custom-backends.md). It also includes
the in-process, inspection, real-disk, plugin, mount, and Lua examples
under [the host examples](../../crates/everruns/examples/advanced/in_process_runtime.rs)
and the provider-facing [OpenAI README](../../crates/drivers/drivers/README.md).

Repository consumers were audited separately because they exercise topologies
that examples do not: the offline
[weekend concierge](../../examples/weekend-concierge-host/src/lib.rs), the
[generic evaluation runner](../../evals/generic/src/subject.rs), the
[Lua-versus-bash research harness](../../research/lua-vs-bash/src/main.rs), the
[worker host](../../crates/worker/src/unified_worker.rs), the
[local host builder](../../crates/everruns/src/local/runtime_builder.rs), and the
[live subagent host test](../../crates/llm-tests/tests/subagent_live_test.rs).
The decision below is the durable decision for each family; individual
implementation tests inside `crates/core/tests/consolidated/host` remain host coverage, not additional
application entrypoints.

## Host services in Engine applications

An embedding host with its own curated capability implementations, provider
resolution, recovery budgets, MCP credentials and observers must be able to
adopt `Engine` without replacing that behavior by the default Framework
catalog (EVE-1296). Yolop's foreground and child execution is the first such
consumer. Exposing the runtime builder would also expose session authority,
stores and the filesystem factory, and a callback or a separately driven
runtime would bypass Engine's leases, admission and turn runner.

The decision is a curated, value-first SPI,
[`hosting::services`](../../crates/everruns/src/hosting/services.rs), attached
to the immutable Agent. Agent build validates it; every runtime Engine
constructs for that Agent applies it after the Framework has selected the
Environment-backed composition, so frontend sessions, coherent activations,
context queries and resumption all see the same services. Engine-level host
observers use the existing listener dispatcher.

- Host capabilities use the Core `Capability` contract directly, so pre-tool
  hooks, model views, commands and instructions behave as they do in a
  directly composed runtime. Their ids, aliases and tool names are checked
  against the Framework registry and other inputs; collisions fail the build
  rather than overriding a registration. This is what keeps the
  Environment-selected filesystem and containment out of reach.
- Host provider resolution replaces the Agent-owned provider, never
  supplements it. A model names its provider key; an Agent provider beside
  host providers, or a key that disagrees with the Agent provider, is a build
  error. Credentials resolve per call under the session identity, and an
  unknown key fails before execution instead of letting the runtime guess an
  external driver of the same name.
- Retry, stream-stall and MCP credential services are injected values, not a
  second recovery or transport path. MCP credentials are requested per call.
- Host observers receive the same committed envelopes as application
  listeners, on independent bounded queues; they cannot fail a turn or write
  history.
- Host request metadata and OpenRouter attribution (EVE-1302) are a validated,
  credential-free value on the services, not a harness or builder hook. Engine
  seeds it into the execution configuration of every runtime it builds, the
  same assembled turn context both turn model calls and session completions
  read, so there is one place metadata enters execution and no lane or rebuild
  can drop it. Runtime correlation ids (`session_id`, `turn_id` and the rest of
  the contracts' system key list) are reserved: the build rejects them and the
  runtime stamps them last on every request. `SessionContext` exposes the
  effective entries, and the external-consumer fixture records them on the
  actual provider request across first run, reconstruction and resume.
- Services do not replace egress, stores, session authority or leases. Those
  remain profile or low-level host concerns below.

The [external host-services fixture](../../crates/everruns/tests/fixtures/external-consumer/host-services/src/lib.rs)
proves a downstream crate can author a guarded capability with a model view
using only `everruns`, and the
[parity test](../../crates/everruns/tests/host/host_services_parity.rs) checks
the Engine path against direct Core composition.

## Session commands and live controls

A host that drives a Session (Yolop's terminal, print and ACP frontends are
the first) also dispatches capability slash commands, hot-installs
extensions and changes model, reasoning effort and MCP servers while a
session is live (EVE-1297). Handing it the session runtime would let it drive
a second turn loop beside Engine's, so these are curated
[Session operations](../../crates/everruns/src/session/controls.rs) instead.

- Commands, context queries and out-of-band review completions run on the
  runtime the next model call uses (the legacy session's own runtime, or a
  coherent session's query runtime), so they see the same provider, filters,
  filesystem and capability chain. They append no canonical events and take
  no execution ownership. The legacy actor answers them during an active
  turn, so a tool of that turn can call them without deadlocking its mailbox.
- Capability code is trusted and registered as an in-process value (Agent,
  host services or `Engine::register_capability`), never deserialized.
  Activation is separate and persists only the session overlay of id plus
  validated configuration.
- A live model selection is a credential-free provider key and model id the
  Agent already serves; it never brings provider code, and unavailable
  selections fail without fallback or state change.
- Where state lives follows the hosting profile. Coherent hosting commits the
  capability overlay, model and effort through its `configure` authority
  operation (extending `SessionConfiguration`). The explicit legacy lane
  states weaker semantics: its overlay persists in the session record, while
  model and effort are Engine-held and re-applied by the host after a
  restart. Live MCP definitions can carry credentials, so they are
  Engine-held in both.

The [facade tests](../../crates/everruns/tests/facade/session_controls.rs)
cover dispatch from a running turn, hot install through deactivation,
ordering against sends, cancellation and teardown, and coherent reconstruction;
the external host-services fixture drives the same surface with only
`everruns`.

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
remove. Advanced integrations select core’s `host` feature and declare neutral
`everruns-contracts` types directly.

## Decision of existing public use cases

| Audited use case | Decision | Why / Framework mapping |
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

The decision names the owning entrypoint for each use case; host-only
rows stay reachable through core’s opt-in modules and facade batteries.

## Session work boundary

Framework sessions scope application-defined background tasks and direct wakes
through the high-level work queue. Its default provider is offline and
process-local; a host can replace it behind the same application values when it
needs durable recovery. Local claim/settlement calls make leased, at-least-once
delivery observable without exposing runtime task records, store registries, or
platform constants. Distributed route ownership, recurring schedule runners,
and multi-host lifecycle management remain host concerns.

## Direct model call boundary

Not every application need is an agent. Decision, extraction, summary,
and similar one-shot work wants the provider edge — driver, endpoint,
credentials, retries, error classification — without the agent loop. The
Framework previously offered no promoted path for it: the pieces were public
but scattered across `everruns` and `everruns-provider`, so the simplest
possible use required two crates and provider-owned types.

`Model::complete` and `Model::completion` close that. They are a thin
value-first layer over `Provider`'s existing completion methods, reusing the
model and provider values an `Agent` already takes, so a direct call and an
agent call reach a model the same way. Deliberately excluded: conversation
state, tool execution, workspaces, durability, events, and hooks. A completion
holds no history of its own — context is whatever the call passes — and work
that needs any of the excluded concerns uses an Agent instead. The layer adds
no execution semantics, so it cannot drift from the agent path.

The low-level path stays open and is now self-sufficient from the facade: the
`Llm*` request/response types a `Provider` or `ChatDriver` call needs are
re-exported from `everruns`, so calling the driver boundary directly no longer
forces a second crate dependency.

## Direct decision boundary

The counterpart to the direct model call, for work whose answer is a number
rather than prose. *Is this claim supported? How severe is this? Which queue?*
A chat model answers those in text a call site must parse and trust, and every
such site grows the same three things: a prompt asking for JSON, a parser, and
a fallback for when the parse fails. In anything enforcing a policy that
fallback is a silent bypass.

`Decisions::probability` and `Decisions::about` close that the same way `Model` closed
the direct completion: a thin value-first layer over the `DecisionsService`
contract core already owns, reaching a decisions the way `Model` reaches a
chat model. The contract differs because the work differs — state plus typed
questions in, calibrated answers out, and the threshold that decides an outcome
stays in the caller's code. There is nothing to stream, because a decision is
one round trip.

The concrete service is supplied, never assumed: `Decisions::new` takes a model
id and any `DecisionsService`, exactly as `Model::new` takes an id and any
`Provider`, so the facade depends on no vendor and the model is a caller's
decision rather than an inherited default. `Decisions::simulated` keeps tests and examples offline, the
role `Model::simulated` plays for completions. `everruns` re-exports the `TypeSafeAI`
provider behind its `typesafe` feature, the way it re-exports `OpenAI`, so one
import reaches both halves without the vendor entering the default build.

`Decisions::from_registry` is the multi-vendor form: a
`ProviderRegistry` plus an explicit `ModelSpec`, the same account/model pair
used for chat. Model names never select providers. Routing and
the calibrated/uncalibrated distinction are recorded in
[Decisions Service](../operations/decisions-service.md#decision-drivers);
`Answers::is_calibrated` tells a caller when the numbers are one-hot labels
rather than a measured distribution.

The model is named, not fixed. `Model::new` takes a model id because a provider
is transport and serves many; a decisions service has a default of its own, so
`Decisions::model` and `Decision::model` are overrides — per decisions
and per call. That asymmetry is the whole of it: there will be other
classifiers, and pinning a version rather than tracking a vendor default is the
caller's decision. A deployment that must pin one does so by not exposing the
knob in the config it accepts, not by the type being unable to carry one.

Deliberately excluded, and for the same reasons as the completion layer:
history, tools, workspaces, durability, events, hooks. Also excluded: retry and
threshold policy. A decision returns the distribution; what counts as a block,
a routing decision, or an escalation is the application's, and burying it in
the layer would recreate the parse-and-trust problem one level down.

- Direct decision stays a thin layer over `DecisionsService` with no
  policy of its own. The number is the layer's output; the decision is the
  caller's.
- Two paths, split by who writes the questions: `Decisions` when the
  application asks and decides, the `jev` capability when the agent asks as part
  of its own work. Both reach the same tool contract, so behavior matches
  whether the Framework is embedded or the platform runs it.
- Question ids are caller-side labels and never reach the model, so each
  question must carry its whole meaning in its instructions.
## Model catalog boundary

Selecting a model requires an exact, provider-visible id, and the Framework
deliberately keeps that id credential-free and unvalidated. That left
applications with no promoted way to find out which ids a provider actually
serves: discovery, profile lookup, and driver-kind identity were public but
lived in `everruns-provider`, so a model picker forced a second crate
dependency and provider-owned types.

`everruns::models` closes that with the same shape as the direct model call
layer: `list` asks a provider for its catalog, `ModelInfo` carries the id plus
the display and capability metadata a picker renders, and a selection converts
back into the `Model` value an `Agent` or a `Completion` already takes. Catalog
answers are three-valued at the driver boundary, and stay three-valued here: a
provider with no catalog is a typed variant, not an error, so callers keep
curated suggestions instead of reporting a failure.

Capability and limit answers come from the curated profile registry, which is
static data. They are display hints, not guarantees, and an unknown id simply
has no profile. The registry is also readable without any provider call, since
"what is this model" is an offline question.

A provider's runtime key is application-chosen and independent of the driver
kind it speaks. `Provider` now carries that kind explicitly, defaulting to the
key, so profile lookups and catalog enrichment resolve against the vendor even
when the provider is keyed `"my-gateway"`. Credential-resolved providers record
it from the driver descriptor they were built from.

## Boundary constraints

- Application and host setup paths must converge before provider resolution and
  engine execution; no parallel provider or turn semantics are allowed.
- Canonical events are the sole durable conversation record. Message/context
  history is a read projection; new Framework profiles must not select or own a
  writable message store.
- A Framework session has at most one active turn. Sending while that turn still
  accepts input steers it; sending after its terminal boundary starts the next
  turn. Admission precedes that timing-dependent assignment, so its receipt may
  report pending; an authoritative assignment records the eventual decision.
  Blocking request/response remains convenience over send plus wait.
- The host log owns coherent append and bounded snapshot replay. In-memory
  durability is process-lifetime only; JSONL acknowledges only a flushed and
  synchronized canonical envelope. Any projection index is rebuildable.
- Engine listeners observe new events only. Resume does not replay persisted
  history. Listener backpressure, overflow, or failure cannot change turn
  success, event persistence, or per-session event-stream isolation.
- Promoting an application concern must not expose credentials, tenant records,
  backend stores, or host lifecycle entities.
- Resume authority is engine-scoped. An in-memory engine must reject an id from
  another engine, retain the exact Agent snapshot chosen at creation, and reopen
  the exact persisted WorkspaceHead and typed Environment extensions. Agent
  serialization is not part of the Framework contract; distributed Platform
  composition reconstructs trusted behavior at its host boundary.
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
- Direct model calls stay a thin layer over `Provider` with no execution
  semantics of their own. Anything stateful — history, tools, workspaces,
  durability — belongs to an Agent, never to a completion.

## Success bar

Every application surface promoted in an atomic unit has a downstream-style
compile/run fixture that imports only `everruns`, uses offline simulation and
temporary files, and does not require credentials or network access.

This governs **fixtures**, not public documentation. It says a promoted surface
must be *provable* without credentials; it does not say the simulator is how the
surface should be *introduced*. Read as a docs rule it inverts the product:
every page opens on a canned response, and a reader meets a test double before
they meet a model. Public pages lead with a real provider doing real work, and
mention the simulator afterwards, as a testing tool. Snippets on interior
reference pages may still use `Model::simulated` when the model is incidental to
the API being shown. The same
bar applies when the event/history, hooks, task/wake, workspace-policy, and
capability-SPI units land. Product worker/server and local host behavior must
remain aligned through the shared engine kernel. Facade-only dependency guards
apply to ordinary application fixtures; advanced host fixtures may depend on
focused host and sibling crates without turning those crates into Framework
entrypoints.

## Source index

- `crates/everruns/src/agent.rs`
- `crates/contracts/src/capability/lib.rs`
- `crates/everruns/src/capability_config.rs`
- `crates/everruns/src/tool_search.rs`
- `crates/everruns/src/hooks.rs`
- `crates/everruns/src/observers.rs`
- `crates/everruns/src/compaction.rs`
- `crates/everruns/src/session.rs`
- `crates/everruns/src/context.rs`
- `crates/everruns/src/mcp.rs`
- `crates/everruns/src/plugin.rs`
- `crates/everruns/src/llm.rs`
- `crates/everruns/src/decisions.rs`
- `crates/everruns/src/local.rs`
- `crates/everruns/src/work.rs`
- `crates/everruns/tests/facade/application_parity.rs`
- `crates/everruns/tests/facade/capability_configuration.rs`
- `crates/everruns/tests/facade/session_work.rs`
- `crates/everruns/tests/facade/lifecycle_hooks.rs`
- `examples/coding-cli/tests/application_parity.rs`
- `crates/core/src/host/runtime/mod.rs`
- `crates/core/src/host/events.rs`
- `crates/core/src/host/mod.rs`
- `crates/everruns/src/local/`
- `crates/everruns/src/hosting/services.rs`
- `crates/everruns/tests/facade/host_services.rs`
- `crates/everruns/src/session/controls.rs`
- `crates/everruns/tests/facade/session_controls.rs`
