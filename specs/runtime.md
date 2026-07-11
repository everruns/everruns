# Runtime Specification

## Abstract

`everruns-runtime` is the public execution crate for Everruns.

It lets embedders run sessions inside their own process without the durable
execution engine, gRPC worker boundary, or control-plane server. It also owns
the reusable host-phase execution contract that durable/server-backed workers
use for `input -> reason -> act`.

The crate uses the same core atoms and capability resolution path as the worker
so embedded execution stays behaviorally aligned with the main runtime.

## Goals

1. Provide a supported public crate for in-process execution.
2. Let embedders supply their own `PlatformDefinition` with custom capabilities,
   LLM drivers, and built-in harness templates.
3. Run without PostgreSQL, NATS, or the durable engine.
4. Ship batteries-included in-memory stores so common harnesses work without
   server internals.
5. Preserve the core turn contract (`input -> reason -> act`) and event/message
   shapes used elsewhere in the system.
6. Provide a supported host adapter contract so durable/server-backed execution
   can reuse runtime-owned phase orchestration.

## Position in the Stack

`everruns-runtime` sits above `everruns-core` and below any host application.

- `everruns-core` owns atoms, traits, capabilities, event types, and shared
  domain/runtime types.
- `everruns-runtime` owns embedder-facing orchestration, in-memory stores, turn
  execution, runtime seeding helpers, reusable host-phase execution, and shared
  turn-strategy planning.
- `everruns-server` and `everruns-worker` remain control-plane and durable
  execution hosts. They use runtime-owned host execution and turn-strategy
  planning, while still owning the durable engine implementation, worker
  polling, retries, and process boundaries.

## Public Contract

The public entrypoint is `InProcessRuntimeBuilder` in
`crates/runtime/src/runtime.rs`.

### Builder responsibilities

The builder must allow an embedder to:

- Replace the `PlatformDefinition`
- Register extra capabilities
- Register or replace LLM drivers
- Seed harnesses, agents, and sessions
- Seed workspace files
- Configure a default model
- Optionally register `llmsim` for deterministic local examples and tests
- Swap the default in-memory stores for custom backends via `RuntimeBackends`

`everruns-runtime` also exposes `HarnessBuilder`, `AgentBuilder`, and
`SessionBuilder` as the supported embedder construction path for those seed
models. The core structs remain public data models and may still be constructed
directly, but embedders should prefer the builders so new optional core fields
can be defaulted inside the runtime crate instead of forcing every embedder to
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

### Host execution responsibilities

`everruns-runtime` must also expose a reusable host contract for durable or
server-backed execution:

- `RuntimeHostAdapter`
- `RuntimeSessionLifecycle`
- `execute_input_activity(...)`
- `execute_reason_activity(...)`
- `execute_act_activity(...)`

These APIs own phase-local orchestration, atom wiring, dependency blocker
handling, lifecycle event emission, and generic turn-strategy planning for
server-backed hosts.

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

Host planning APIs:

- `RuntimeTurnState`
- `RuntimeActPlan`
- `RuntimeTurnPlan`
- `plan_next_host_turn(...)`

The planner contract is intentionally durable-agnostic. Runtime decides the
next semantic step; the host owns queueing, retries, scheduling, persistence,
and workflow resumption.

## Execution Semantics

`everruns-runtime` must execute the shared `TurnStateMachine` from `everruns-core`.

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
7. Durable/server-backed workers must execute their per-phase host logic
   through `everruns-runtime` instead of maintaining a separate copy of atom
   wiring in the worker crate.
8. Durable/server-backed workers must use runtime-owned turn-strategy planning
   for `process_input -> reason -> act`, steering continuation,
   dependency-blocked completion, and `waiting_for_tool_results` pause/resume.

## Shared Context Assembly

Context assembly is a shared core concern, not a server-only concern.

`everruns-core` owns the canonical context assembly helper for reason-phase
hosts:

- merged harness/agent/session overlay
- capability dependency resolution
- capability message filtering
- model resolution
- locale resolution
- `RuntimeAgent` construction

Public API:

- `everruns_core::assemble_turn_context(...)`
- `everruns_core::inspect_turn_context(...)`
- `everruns_core::AssembledTurnContext`

`ReasonAtom` and `everruns-runtime` must use this shared path so embedded hosts
and worker-backed hosts stay aligned.

## In-Memory Stores

`everruns-runtime` ships reference in-memory stores for public embedding:

- Session store
- Session virtual filesystem
- Session key/value + secret store
- Message retriever
- Harness store
- Agent store
- Provider store
- Memory store
- Event emitter

These stores are intended to make embedded execution usable out of the box.

## Custom Backends

Embedders that need persistence across process restarts may supply their own
backend bundle through `RuntimeBackends`.

`everruns-runtime` owns a small set of extension traits for mutable runtime
domain stores:

- `RuntimeHarnessStore`
- `RuntimeAgentStore`
- `RuntimeSessionStore`
- `RuntimeMessageStore`
- `RuntimeProviderStore`
- `EventBus` (extends `EventEmitter`; default `collected_events` returns empty)

These extend the corresponding core traits with the minimal write operations the
embedded runtime needs for:

- seeding harnesses, agents, sessions, and initial files
- storing input messages
- persisting assistant and tool-result messages from emitted events
- configuring the runtime default model

Use `RuntimeBackends::in_memory()` for the all-in-memory default, then chain
`.with_message_store(...)`, `.with_storage_store(...)`, etc. to override
individual non-filesystem stores.

`RuntimeBackends` also carries an optional
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

Session files are a platform service: `PlatformDefinition` carries a
`SessionFileSystemFactory`, and the runtime always resolves the concrete
filesystem from that factory before seeding files or executing turns. Embedders
can choose `InMemorySessionFileSystemFactory` (default),
`RealDiskSessionFileSystemFactory` (rooted at a host directory; honours
`.gitignore` for grep), or a custom future factory such as S3. See
`specs/file-store.md` for the trait contract, path namespace, and the rule that
capabilities should consume the seam rather than touching `std::fs` directly.

Factories that depend on host values receive them through
`SessionFileSystemFactoryContext`; the in-process builder accepts this context
and passes it to the selected platform factory before runtime seeding.

This keeps `everruns-core` storage traits read-oriented where they already were,
while making custom embedded backends a supported public path.

`everruns-runtime` owns the public extension seam for embedder-supplied
backends. `everruns-worker` ships the first-party durable/server-backed host
adapter (`WorkerRuntimeHost`) that bridges worker storage/adapters into the
runtime host contract.

### Optional host-backend slots

`RuntimeBackends` carries a uniform set of optional, additive backend slots that
the host forwards into `ActAtom` when present (see
`crates/runtime/src/runtime.rs` and `crates/runtime/src/host.rs`):

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
`everruns-runtime` so external embedders can depend on it directly.

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
rows across SQLite connections, heartbeats live claims, recovers stale claims,
advances recurring cron occurrences in their IANA timezone, and disables
successful one-shots. It delivers the stored prompt through the host's existing
`LocalSessionRunner::send_message` implementation; storage never owns session
execution. Failed deliveries release the occurrence for retry and retain a
diagnostic error. Shutdown stops new claims and waits for an active delivery;
forced abort leaves the lease for stale recovery. See `crates/local/` for the
public lifecycle API and tests.

This boundary provides at-least-once delivery across process failure. Atomic
claims prevent duplicate delivery by concurrent healthy runners, but a crash
after `send_message` accepts the prompt and before SQLite commits completion
can retry that occurrence. Hosts should keep scheduled turns idempotent across
that unavoidable external-delivery window.

## Context and Capabilities

Capabilities continue to live in `everruns-core`.

`InProcessRuntimeBuilder::new()` starts from
`CapabilityRegistry::runtime_builtins()`, a curated registry containing only
capabilities usable with the runtime's default in-process host services. The
default registry intentionally excludes hosted Everruns product capabilities,
demos/tests, and capabilities whose tools require optional host backends such as
`platform_store`, `session_task_registry`, `schedule_store`, SQL databases,
provider credentials, or knowledge stores. Embedders that provide those
services can still build an explicit `PlatformDefinition` and register the
larger platform capability set or any selected capability manually.

`everruns-runtime` must provide the supporting stores those capabilities expect
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

The runtime requires a default model. It may come from:

- explicit `InProcessRuntimeBuilder::default_model(...)`
- helper configuration like `InProcessRuntimeBuilder::llm_sim(...)`

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
test binaries in `crates/runtime/tests/`.

- `crates/runtime/tests/in_process_runtime_test.rs` proves embedded runtimes
  can execute turns, persist message history, seed files, and emit the shared
  event shapes without PostgreSQL or worker infrastructure.
- `crates/runtime/tests/runtime_host_test.rs` proves the reusable host adapter
  contract drives `input -> reason -> act` planning and lifecycle state changes
  for server-backed or durable hosts.

## Source Index

- `crates/runtime/src/lib.rs`
- `crates/runtime/src/builders.rs`
- `crates/runtime/src/runtime.rs`
- `crates/runtime/src/host.rs`
- `crates/runtime/src/in_memory.rs`
- `crates/runtime/src/backends.rs`
- `crates/local/` (`everruns-local`: SQLite-backed local host backends)
- `crates/runtime/examples/in_process_runtime.rs`
- `crates/runtime/examples/inspect_context.rs`
- `examples/weekend-concierge-host/src/lib.rs`
- `examples/weekend-concierge-host/src/main.rs`
- `crates/runtime/tests/in_process_runtime_test.rs`
- `crates/runtime/tests/runtime_host_test.rs`
- `crates/worker/src/runtime_host.rs`
- `crates/core/src/runtime_context.rs`
- `crates/core/src/turn.rs`
- `crates/core/src/platform_definition.rs`
