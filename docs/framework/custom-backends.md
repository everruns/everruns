---
title: Custom Backends
description: Decide when a Framework application should cross into low-level execution-host composition.
---

Most applications should use `everruns::Agent`, `Model`, and `Session`. That
surface deliberately hides stored harness records, platform registries, backend
stores, worker phases, and durable scheduling topology.

For durable execution under Framework, implement
[`SessionHosting` through the public hosting SPI](/framework/hosting/). Engine
owns the runtime and recovery; the host supplies transactional session authority,
epochs and independent activation. This path needs only an `everruns` dependency.

Cross into separate low-level composition only when your application is itself an
execution host, for example, a server, evaluation harness, research runtime, or
specialized embedder that must replace storage or orchestration components.

## Host-level choices

The low-level crates expose focused contracts for:

- core agent, event, capability, and provider values;
- the shared Input/Reason/Act kernel and sans-I/O turn planner;
- runtime host phases, canonical event history, and in-memory reference stores;
- local SQLite-backed task and schedule state;
- portable session and agent definitions, plus explicitly selected durable deployment components.

A separately composed low-level host depends on `everruns`, `everruns-core` with the `host` feature,
and `everruns-contracts` for neutral service contracts. The low-level boundary
is `everruns_core::host`; concrete integrations are selected on the facade. It is healthy for such a host to use
low-level extension traits; the goal is not to re-export every backend through
one facade.

## Two engine boundaries

`everruns::Engine` is a concrete application object that owns the behavior
registry, sessions, lifecycle, queries and activation through its hosting profile. It is the normal Framework entrypoint,
not an extension trait. Applications do not implement it.

`everruns_core::engine` is the shared execution kernel, enabled by the `engine`
feature. Advanced hosts
compose its `Execution` contract and serializable `TurnExecution` state machine,
`InputAtom`/`ReasonAtom`/`ActAtom`, and phase values. The implementation lives
in `everruns_core::host`, and the Platform's workers drive the same turn loop
under a session lease. Both use narrow contracts from `everruns-core`. The
engine feature has no dependency on host effects, platform, server, worker,
or durable crates. Do not
copy state advancement or the phase loop into a custom backend; implement the
execution boundary and keep deployment-specific service selection in the host.

Where a turn runs is a separate interface: `everruns_core::host::TurnBackend`
starts, cancels, and observes a session's turns. `InProcessBackend` runs a
turn in process. The legacy local/custom-backend facade lane uses `ActorRunner`
under the session's lease, and the Platform's server marks the session
runnable for a worker that leases it and runs the whole turn. Coherent Framework
hosting instead enters Engine's shared session actor path. `TurnBackend` is not
the durable Framework extension point. A low-level host that records input
through its own event store starts turns from it with
`TurnInput::StoredMessage` and continues a parked turn with
`TurnInput::RecordedToolResults`, which every backend serves. The trait is
public and unsealed but experimental. The facade's [backend conformance suite](https://github.com/everruns/everruns/tree/main/crates/everruns/tests/backend_conformance)
defines the behavior every backend must match.

See [Framework Architecture](/framework/architecture/) for the complete layer
map and the distinction between immediate and durable execution.

Conversation persistence is the one backend with a single write path. Replace it
by implementing the canonical `EventLog`/`EventReader` SPI and passing it to
`HostBackends::with_event_log`; the required snapshot, continuation, and polling
behavior is specified in
[Implementing a custom event log](/framework/events-and-cancellation/#implementing-a-custom-event-log).

## Security boundary

Backend replacement does not relax tenant, credential, filesystem, or tool
execution boundaries. Preserve event ordering, credential redaction, workspace
containment, and cancellation behavior when adapting the host. A custom backend
must fail explicitly when it cannot satisfy a required contract.
