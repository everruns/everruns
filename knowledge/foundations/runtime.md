---
type: Specification
title: "Runtime Specification"
description: "The supported low-level execution host contract."
tags:
  - everruns
  - foundations
---
# Runtime Specification

## Abstract

`everruns-host` is the low-level execution host and the only host boundary
below the Framework. The application-facing entrypoint is the `everruns` crate
and the Everruns Framework; its purpose and terminology are owned by
[knowledge/framework/](../framework/).

It lets embedders run sessions inside their own process without the durable
execution engine, gRPC worker boundary, or control-plane server. It also owns
the reusable host-phase execution contract that durable/server-backed workers
use for `input -> reason -> act`.

The crate composes the same `everruns-engine` phase algorithms and capability
resolution path as the worker, so embedded execution stays behaviorally aligned
with durable execution.

## Goals

1. Preserve a supported low-level public crate for in-process execution.
2. Let embedders supply their own `HostComposition` with custom capabilities,
   LLM drivers, and built-in harness templates.
3. Run without PostgreSQL, NATS, or the durable engine.
4. Ship batteries-included in-memory stores so common harnesses work without
   server internals.
5. Preserve the core turn contract (`input -> reason -> act`) and event/message
   shapes used elsewhere in the system.
6. Provide a supported host adapter contract so durable/server-backed execution
   can reuse runtime-owned phase orchestration.

## Position in the Stack

`everruns-host` sits above `everruns-core` and `everruns-engine`, and below
Framework adaptation or any host application.

- `everruns-core` owns neutral per-turn contracts, capabilities, event
  types, and shared domain/runtime types. Contracts are grouped by execution
  concern (`tool_context`, `execution_loading`, `provider_resolution`,
  `session_files`, `durability`, and sibling modules), never in a catch-all
  service bag.
- `everruns-engine` owns the abstract execution contract, serializable turn
  machine, Input/Reason/Act algorithms, phase values, portable execution hooks,
  tool scheduler, and pure turn planning.
- `everruns-host` owns the immediate `InProcessExecution` driver,
  embedder-facing orchestration, in-memory stores, turn
  execution, store-backed snapshot/context loading, lifecycle and dependency
  probing, provider/driver resolution, command completion, runtime seeding
  helpers, reusable host-phase composition, and lifecycle-effect application.
- `everruns-durable` owns the checkpointed `DurableExecution` driver and
  persistence/retry machinery.
- `everruns-server` and `everruns-worker` remain control-plane and durable
  execution hosts. They adapt host effects and durable scheduling while owning worker
  polling, retries, and process boundaries.

## Public Contract

The public entrypoint is `InProcessRuntimeBuilder` in
`crates/host/src/runtime.rs`.

### Builder responsibilities

The builder must allow an embedder to:

- Replace the `HostComposition`
- Register extra capabilities
- Register or replace runtime providers over protocol drivers
- Seed harnesses, agents, and sessions
- Seed workspace files
- Configure a credential-free default `ModelSpec`
- Optionally register `llmsim` for deterministic local examples and tests
- Swap the default in-memory stores for custom backends via `HostBackends`

`everruns-host` also exposes `HarnessBuilder`, `AgentBuilder`, and
`SessionBuilder` as the supported embedder construction path for those seed
models. The core structs remain public data models and may still be constructed
directly, but embedders should prefer the builders so new optional core fields
can be defaulted inside the host crate instead of forcing every embedder to
update struct literals. `InProcessRuntimeBuilder::single_session(...)` is the
recommended compact path for the common one-harness, one-agent, one-session
runtime shape and records the generated session id for
`InProcessRuntime::default_session_id()`.

### Runtime responsibilities

`InProcessRuntime` must expose:

- `run_turn(session_id, input)` for one turn of execution
- `run_text_turn(session_id, text)` convenience helper
- `default_session_id()` for runtimes seeded with `single_session(...)`
- `load_context(session_id)` to inspect the assembled turn context without
  executing a new turn, including empty sessions before the first message
- `messages(session_id)` to inspect conversation history
- `read_file(session_id, path)` to inspect the in-memory workspace
- `events()` to inspect emitted runtime events

Each completed turn exposes a stable structured stop reason in addition to the
existing success/error fields, so embedders can distinguish normal completion,
provider token limits, runtime request caps, refusal, failure, and cancellation
without parsing provider strings.

### Host execution responsibilities

`everruns-host` must also expose a reusable host contract for durable or
server-backed execution:

- `RuntimeHostAdapter`
- `RuntimeSessionLifecycle`
- `execute_input_activity(...)`
- `execute_reason_activity(...)`
- `execute_act_activity(...)`

These APIs own phase-local orchestration, atom wiring, dependency blocker
handling, lifecycle event emission, and generic turn-strategy planning for
server-backed hosts.

The host execution contract is value-first (EVE-872): turn execution consumes
`everruns_core::ResolvedExecutionSnapshot` — a neutral, secret-free projection
of the effective harness → agent → session configuration — never stored
`Agent`/`Harness`/`Session` aggregates. `RuntimeHostAdapter::load_resolved_turn`
returns that snapshot plus the turn's message and MCP tool inputs; adapters
perform the platform projection (`ResolvedExecutionSnapshot::project`) so
missing, mismatched, or inactive records fail before host execution begins.
Session status mutation stays a separate host effect that exposes no session
record. A source guard (`crates/host/tests/execution_contract_guard.rs`)
prevents the contract module from naming the record types again; the
`InProcessRuntimeBuilder` seeding APIs still accept records as host
configuration until they are separately replaced.

`RuntimeHostAdapter` also exposes an optional, per-session
`reasoning_effort_handle(session_id)` seam (default `None`). When a host returns
a stable handle for a session, `ReasonAtom` re-reads it on every LLM step and
lets its value override the message-derived `controls.reasoning.effort` (still
gated by the model profile). This lets a tool change reasoning effort
mid-`run_turn` and have subsequent steps in the same turn observe it, instead of
only on the next turn. Hosts that do not override the seam are unaffected.

`InProcessRuntime` implements `RuntimeHostAdapter` and drives its own turn
loop by calling these activity functions directly. Atom construction lives
in one place — host-side — so any improvement (tool-registry caching,
error semantics, hook ordering) flows to both embedded and durable hosts
automatically.

The host also owns one cloneable tool-context service snapshot per act
assembly. Every production `ToolContext` is derived from that complete
snapshot, then decorated only with per-call metadata. Active built-in tools
declare hard service requirements, and host assembly validates the active
registry before reason exposes its definitions to the model. Missing services
are configuration errors that name the tool and service; deliberately
service-free tests may continue to use `ToolContext::new`.

Deployment-selected construction is a host concern. In particular,
`SessionFileSystemFactory` and its type-erased factory context live in
`everruns-host`; core retains only the `SessionFileSystem` effect contract and
the workspace-scoping adapter used by turn execution.

Host planning APIs:

- `Execution` / `TurnExecution`
- `ExecutionTransition`
- `RuntimeTurnState`
- `RuntimeActPlan`
- `RuntimeTurnPlan`
- `advance_host_execution(...)`

`plan_next_host_turn(...)` remains only as a compatibility wrapper for callers
that have not yet adopted an explicit execution driver.

Terminal host plans carry the same structured stop reason. Durable hosts must
preserve it in the turn value returned to their downstream caller.

The planner contract is intentionally durable-agnostic. Runtime decides the
next semantic step; the host owns queueing, retries, scheduling, persistence,
and workflow resumption.

## Execution Semantics

`everruns-host` must drive `everruns-engine::TurnState` and compose the
engine-owned phase executors.

Required behavior:

1. Store the input message in runtime message history before `InputAtom` runs.
2. Emit `input.message` when a turn starts.
3. Resolve effective configuration by folding harness chain, agent, and session
   overlays via `AgentConfigOverlay`.
4. Resolve active capabilities from the effective overlay, not from the full
   platform registry.
5. Execute the same `ReasonAtom` and `ActAtom` implementations used by other
   runtime hosts.
6. Persist assistant messages and tool-result messages from emitted events so
   subsequent turns see the same history shape as the durable/server-backed
   runtime.
7. Durable/server-backed workers must execute their per-phase host composition
   through `everruns-host` instead of maintaining a separate execution loop in
   the worker crate.
8. Durable/server-backed workers must use engine-owned turn planning
   for `process_input -> reason -> act`, steering continuation,
   dependency-blocked completion, and `waiting_for_tool_results` pause/resume.
9. Provider recovery must reuse the assembled reason input in place. It must
   not persist a second user message, schedule another act for already-completed
   tools, or discard provider continuation state that remains valid. Terminal
   transient exhaustion is reported as safe to resume from persisted history;
   permanent provider failures retain their precise classification.

## Shared Context Assembly

Context assembly is split by effect (EVE-905). `everruns-host` owns every
store-backed step: multi-store loading, lifecycle/topology validation, message
queries and filters, model/provider lookup, credential-bearing driver creation,
and context inspection. Its public entrypoints are
`assemble_turn_context`, `assemble_turn_context_from_snapshot`,
`inspect_turn_context`, and `StoreTurnContextResolver`.

`everruns-core` owns only the deterministic projection and transformation
steps: `ResolvedExecutionSnapshot::project`, capability/overlay resolution,
locale projection, `RuntimeAgent` construction, and
`assemble_resolved_turn_context`. `ResolvedTurnContextInput` contains the
secret-free snapshot, filtered messages, safe model/provider identity, and an
opaque ready driver. It never contains persisted Agent, Harness, or Session
records, provider configuration, API keys, or base URLs.

`everruns_engine::ReasonAtom` accepts a preassembled context on normal host
paths. Its narrow `TurnContextResolver` fallback lets direct engine callers delegate preparation to
a host without importing stores into the kernel. Framework in-process and
durable worker paths must use this same split so hooks, retries, cancellation,
events, compaction, and continuation behavior remain aligned.

## In-Memory Stores

`everruns-host` ships reference in-memory stores for public embedding:

- Session store
- Session virtual filesystem
- Session key/value + secret store
- Harness store
- Agent store
- Provider store
- Canonical event log and read-only message history projection

These stores are intended to make embedded execution usable out of the box.

## Custom Backends

Embedders that need persistence across process restarts may supply their own
backend bundle through `HostBackends`.

`everruns-host` owns a small set of extension traits for mutable runtime
domain stores:

- `RuntimeHarnessStore`
- `RuntimeAgentStore`
- `RuntimeSessionStore`
- `RuntimeProviderStore`
- `EventLog` plus its `EventReader` half

These extend the corresponding core traits with the minimal write operations the
embedded runtime needs for:

- seeding harnesses, agents, sessions, and initial files
- configuring the runtime default model

Conversation persistence is intentionally different: accepted inputs and
assistant/tool outputs append once to the canonical `EventLog` through
`HostEventEmitter`. `EventHistory` rebuilds the read-only `MessageRetriever`
projection. There is no writable message-store backend and no dual-write path.
The optional `EventSink` observes already-finalized envelopes after commit; it
is never persistence.

`RuntimeSessionStore` also inherits the live capability mutation methods from
`SessionMutator`. A custom backend that wants to support
`InProcessRuntime::{activate_capability, deactivate_capability}` implements
atomic session-capability upsert/removal; backends that do not implement the
methods return the default unsupported error.

Live capability changes do not replace the `RuntimeAgent` in place. The session
overlay is the source of truth, and the existing per-boundary assembly path
rebuilds the prompt, definitions, executable registry, and hooks before the
next reason/act step. This preserves session identity and conversation history
and avoids two independently mutable copies of the runtime surface.

Use `HostBackends::in_memory()` for the all-in-memory default, then chain
`.with_event_log(...)`, `.with_storage_store(...)`, etc. to override individual
non-filesystem stores.

`HostBackends` also carries an optional
`connection_resolver: Option<Arc<dyn UserConnectionResolver>>`
(set via `.with_connection_resolver(...)`). When supplied, the runtime exposes
it through `ToolContext.connection_resolver` so connection-aware capabilities
(for example the Daytona integration) resolve user connection tokens lazily at
tool execution time. There is no in-memory default: a resolver implies a real
credential source the embedder owns. When unset, `ToolContext.connection_resolver`
stays `None` and connection-aware tools fall back to their own guidance
(session secret, then "connect the provider"). See
`crates/server/specs/user-connections.md` for the connection model the resolver
serves.

Session files are a platform service: `HostComposition` carries a
`SessionFileSystemFactory`, and the runtime always resolves the concrete
filesystem from that factory before seeding files or executing turns. Embedders
can choose `InMemorySessionFileSystemFactory` (default),
`RealDiskSessionFileSystemFactory` (rooted at a host directory; honours
`.gitignore` for grep), or a custom future factory such as S3. See
`knowledge/runtime-resources/file-store.md` for the trait contract, path namespace, and the rule that
capabilities should consume the seam rather than touching `std::fs` directly.

Factories that depend on host values receive them through
`SessionFileSystemFactoryContext`; the in-process builder accepts this context
and passes it to the selected platform factory before runtime seeding.

This keeps `everruns-core` storage traits read-oriented where they already were,
while making custom embedded backends a supported public path.

`everruns-host` owns the public extension seam for embedder-supplied
backends. `everruns-worker` ships the first-party durable/server-backed host
adapter (`WorkerRuntimeHost`) that bridges worker storage/adapters into the
runtime host contract.

### Canonical event log

Conversation persistence is the one backend with a single write path: the
canonical event log is the durable truth and message history is a rebuildable
projection of it. Embedders replace it by implementing the `EventReader` and
`EventLog` traits in `crates/host/src/events.rs` and supplying the result
through the event-log backend slot.

That pair is a supported public SPI, not an in-crate detail. A detached crate
outside this workspace must be able to implement both — including stable
snapshot pagination — from published paths alone, so the reader must be able to
inspect a read request's cursor, build a snapshot-pinned continuation, and
construct a result page. Construction is validated in one place so in-crate and
external implementations expose the same observable invariants: per-session
binding, monotonically increasing sequences that may contain gaps,
continuations pinned to the snapshot their first page captured, and polling
cursors that deliberately start a new snapshot. The log stays append-only;
there is no truncate, rewind, or mutation contract.

`tests/fixtures/external-consumer/event-log` is an out-of-workspace
implementation exercised by CI, so the SPI cannot silently stop being
externally implementable.

### Optional host-backend slots

`HostBackends` carries a uniform set of optional, additive backend slots that
the host forwards into `ActAtom` when present (see
`crates/host/src/runtime.rs` and `crates/host/src/host.rs`):

- `session_task_registry` — persists background-tool / subagent / monitor task
  lifecycle (`everruns_core::session_task::SessionTaskRegistry`).
- `schedule_store_factory` — per-org `SessionScheduleStore` for local schedules
  and monitors.
- `platform_store_factory` — per-(org, session) `PlatformStore` for local
  session management and subagent spawning.

Each slot defaults to `None`; leaving it unset preserves the prior in-memory
behavior (the `RuntimeHostAdapter` method returns `None`). The slots are a
single, repeatable "optional backend slot" pattern so additional local backends
can be added without further runtime changes.

## Local Backends (`everruns-local`)

`everruns-local` is the first-party crate that populates the optional
host-backend slots with local, file-backed implementations for embedded,
single-process hosts (where Yolop and miy would otherwise each reinvent them).
The runtime stays generic and owns only the seams; durable local storage choices
live in this separate opt-in crate. It ships on crates.io alongside
`everruns-host` so external embedders can depend on it directly.

It provides SQLite-backed, restart-survivable stores —
`LocalSessionTaskRegistry`, `LocalScheduleStore`, `LocalPlatformStore` — plus a
`LocalProfile` (data dir / workspace / identity defaults), a composable
`LocalBackends` (which preserves a caller-supplied event bus and
`SessionFileSystemFactory`), and an optional `LocalRuntimeBuilder` convenience
wrapper. Task and message state persists to a SQLite file so a freshly-spawned
process can read, continue, and inspect tasks an earlier process started.

Schedules keep an extensible per-record metadata bag (name/color/kind/etc.) in a
local `metadata` JSON column rather than by widening the shared core
`SessionSchedule` primitive; the trait-level store surface is unchanged. See
`crates/local/` for the implementation and `crates/local/tests/` for the
task-lifecycle, restart-survivability, schedule round-trip, composability, and
embedded-turn coverage.

Persisting a session schedule does not execute it by itself. Embedded hosts
that expose scheduled monitors explicitly start and retain
`LocalScheduleRunner` for the host lifetime. The runner atomically claims due
rows across SQLite connections, but only for the session ids the host reports
through `LocalSessionRunner::routable_session_ids`. Filtering happens before
due-ordering and the batch limit so inactive sessions cannot consume a batch or
starve deliverable work. The runner heartbeats live claims, recovers stale
claims, advances recurring cron occurrences in their IANA timezone, and
disables successful one-shots. It delivers the stored prompt through the host's
existing `LocalSessionRunner::send_message` implementation; storage never owns
session execution. Failed deliveries retain the occurrence and diagnostic
error, then wait `claim_timeout` before retry so a disappearing route cannot
cause poll-rate claim churn. Shutdown stops new claims and waits for an active
delivery; forced abort leaves the lease for stale recovery. See `crates/local/`
for the public lifecycle API and tests.

This boundary provides at-least-once delivery across process failure. Atomic
claims prevent duplicate delivery by concurrent healthy runners, but a crash
after `send_message` accepts the prompt and before SQLite commits completion
can retry that occurrence. Hosts should keep scheduled turns idempotent across
that unavoidable external-delivery window.

## Context and Capabilities

Effect-neutral capability contracts and built-ins live in `everruns-core`.
Environment-backed implementations live in focused integration crates and are
selected by the host's Cargo features.

`InProcessRuntimeBuilder::new()` starts from
`everruns_host::runtime_capability_registry()`: core runtime built-ins plus
only the filesystem, Bashkit, web-fetch, and Lua integrations compiled into
the host. The
default registry intentionally excludes hosted Everruns product capabilities,
demos/tests, and capabilities whose tools require optional host backends such as
`platform_store`, `session_task_registry`, `schedule_store`, SQL databases,
provider credentials, or knowledge stores. Embedders that provide those
services can still build an explicit `HostComposition` and register the
larger platform capability set or any selected capability manually.

`everruns-host` must provide the supporting stores those capabilities expect
through `ToolContext`, including:

- filesystem access
- session storage
- session metadata access/mutation
- memory store access
- message retrieval for context-management capabilities

This means capabilities such as filesystem tools, persistent memory, and
infinity context can run in-process as long as the runtime wires the necessary
backends.

## Model Resolution

The runtime requires a default model and resolves it through the canonical
`ModelSpec` + runtime `Provider` path. New embedders use:

- `InProcessRuntimeBuilder::provider(...)`
- `InProcessRuntimeBuilder::model_spec(...)`

Provider identity is an open string. The provider owns endpoint, headers,
async authentication, and its protocol driver; the model remains
credential-free.

These host entry points configure the default model:

- explicit `InProcessRuntimeBuilder::default_model(...)`
- helper configuration like `InProcessRuntimeBuilder::llm_sim(...)`

They convert into the same direct provider registry and execution path before a
turn runs. They do not retain a separate provider resolution algorithm or
driver semantics.

If no default model is available at build time, `build()` must fail with a
configuration error.

## Non-goals

This spec does not require runtime to own:

- durable workflow persistence schemas
- durable runners or queue/store contracts
- worker registration, polling, and heartbeat transport
- scheduler daemons for cron/due-time task discovery
- server route composition

Those remain separate concerns outside the runtime host orchestration contract.

## Validation

The in-process runtime contract is regression-tested in CI with the pure Rust
test binaries in `crates/host/tests/`.

- `crates/host/tests/in_process_runtime_test.rs` proves embedded runtimes
  can execute turns, persist message history, seed files, and emit the shared
  event shapes without PostgreSQL or worker infrastructure.
- `crates/host/tests/runtime_host_test.rs` proves the reusable host adapter
  contract drives `input -> reason -> act` planning and lifecycle state changes
  for server-backed or durable hosts.

## Source Index

- `crates/host/src/lib.rs`
- `crates/host/src/builders.rs`
- `crates/host/src/runtime.rs`
- `crates/host/src/host.rs`
- `crates/host/src/in_memory.rs`
- `crates/host/src/backends.rs`
- `crates/local/` (`everruns-local`: SQLite-backed local host backends)
- `crates/host/examples/in_process_runtime.rs`
- `crates/host/examples/inspect_context.rs`
- `examples/weekend-concierge-host/src/lib.rs`
- `examples/weekend-concierge-host/src/main.rs`
- `crates/host/tests/in_process_runtime_test.rs`
- `crates/host/tests/runtime_host_test.rs`
- `crates/worker/src/runtime_host.rs`
- `crates/core/src/runtime_context.rs`
- `crates/core/src/turn.rs`
- `crates/host/src/composition.rs`
