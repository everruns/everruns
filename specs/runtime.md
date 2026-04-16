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
  execution, runtime seeding helpers, and reusable host-phase execution.
- `everruns-server` and `everruns-worker` remain control-plane and durable
  execution hosts. They use runtime-owned host execution but still own queueing,
  retries, workflows, and process boundaries.

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

### Runtime responsibilities

`InProcessRuntime` must expose:

- `run_turn(session_id, input)` for one turn of execution
- `run_text_turn(session_id, text)` convenience helper
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
handling, and lifecycle event emission for the host runtime. Durable scheduling
and workflow management remain outside this contract.

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
- Session virtual filesystem store
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

`everruns-runtime` owns a small set of extension traits for this:

- `RuntimeHarnessStore`
- `RuntimeAgentStore`
- `RuntimeSessionStore`
- `RuntimeMessageStore`
- `RuntimeFileStore`
- `RuntimeProviderStore`
- optional `RuntimeEventCollector`

These extend the corresponding core traits with the minimal write operations the
embedded runtime needs for:

- seeding harnesses, agents, sessions, and initial files
- storing input messages
- persisting assistant and tool-result messages from emitted events
- configuring the runtime default model

This keeps `everruns-core` storage traits read-oriented where they already were,
while making custom embedded backends a supported public path.

`everruns-runtime` owns the public extension seam for embedder-supplied
backends. `everruns-worker` ships the first-party durable/server-backed host
adapter (`WorkerRuntimeHost`) that bridges worker storage/adapters into the
runtime host contract.

## Context and Capabilities

Capabilities continue to live in `everruns-core`.

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

- durable workflows
- cross-process task recovery
- task queues or workflow schedulers
- server route composition

Those remain separate concerns outside the runtime host-phase contract.

## Source Index

- `crates/runtime/src/lib.rs`
- `crates/runtime/src/runtime.rs`
- `crates/runtime/src/host.rs`
- `crates/runtime/src/in_memory.rs`
- `crates/runtime/examples/in_process_runtime.rs`
- `crates/runtime/examples/inspect_context.rs`
- `crates/runtime/tests/in_process_runtime_test.rs`
- `crates/runtime/tests/runtime_host_test.rs`
- `crates/worker/src/runtime_host.rs`
- `crates/core/src/runtime_context.rs`
- `crates/core/src/turn.rs`
- `crates/core/src/platform_definition.rs`
