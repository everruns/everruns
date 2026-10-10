---
title: Framework Architecture
description: Understand Agent, Engine, Session, the TurnBackend entry point, and how immediate and durable execution share one kernel.
---

Everruns has one turn model and two ways to execute it. A turn runs either in
process, as a session actor holding the session's lease, or durably, as queued
and checkpointed steps. A library application uses the concrete
`everruns::Engine`, whose sessions run in process; what survives a restart is
the session's event log. The Everruns Platform's server and workers run turns
durably. Every path converges on the same `everruns-core` (`engine`
feature) Input/Reason/Act state machine.

![Framework execution architecture: the Framework app and the Platform server start turns through the TurnBackend entry point in everruns-core; the Framework's ActorRunner (the in-process backend plus a session lease) drives the core::engine kernel directly, while the Platform's everruns-durable-engine (TurnStore, DurableRunner, TurnTaskDriver) runs the same kernel as queued, checkpointed steps on the generic everruns-durable engine over an in-memory or PostgreSQL store; Platform workers claim those steps over gRPC.](./architecture.svg)

## Public Framework objects

| Object | Responsibility |
| --- | --- |
| `Agent` | Immutable behavior: instructions, model and provider, tools, capabilities, files, and lifecycle hooks |
| `Engine` | Concrete process-local owner of Agent snapshots, session identity, backends, history, and resume authority |
| `Session` | First-class, engine-bound conversation used for turns, steering, events, cancellation, inspection, and history |
| `Environment` | Session resources, including one exact backend-owned workspace head and typed extensions |

New Framework code creates and resumes sessions through an Engine:

```rust
use everruns::{Agent, Engine, OpenAI};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer concisely.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;

let engine = Engine::new();
let session = engine.create(agent);
let session_id = session.session_id();
let turn = session.send_and_wait("Begin.").await?;
assert!(turn.success);

drop(session);
let resumed = engine.resume(session_id).await?;
assert_eq!(resumed.session_id(), session_id);
# Ok(())
# }
```

## Two execution paths, one kernel

A host hands each turn to a `TurnBackend`, the turn entry point in
`everruns-core` (`host` feature) that starts, cancels, and observes a session's turns. The backend
decides only where the planned steps run and what survives a crash; it never
plans the turn itself.

- **In process, as an actor.** `InProcessBackend` runs the turn on the task
  that awaits it, through `InProcessRuntime`. Every `everruns::Engine` session
  runs on `ActorRunner`, which wraps it and holds the session's lease while a
  turn runs, so two processes sharing a local data directory never run one
  session at once (see [One process runs a session at a
  time](/framework/sessions/#one-process-runs-a-session-at-a-time)). Dropping
  the turn's future stops it; a turn a restart cut off resumes from the event
  log.
- **Durable (Platform).** `everruns-durable-engine` runs each turn step
  (input, reason, act) as a task on an `everruns-durable` queue and checkpoints
  the turn's state after every step, so a step a crashed process held is
  reclaimed and the turn continues from its last checkpoint. The Platform
  server persists each input and then starts the turn from it through the same
  trait, on its `DurableRunner`, and Platform workers claim the steps over gRPC
  and run them with the `TurnTaskDriver`.

A caller either hands the backend input to record (`TurnInput::Message`,
`TurnInput::ToolResults`) or names input it already recorded through its own
store (`TurnInput::StoredMessage`, `TurnInput::RecordedToolResults`). Every
backend serves the stored forms, so a host that persists input itself starts
and continues turns the way the Platform server does.

`everruns-durable` underneath is a generic engine: a task queue, an event log,
signals, timers, child workflows, a worker registry, a dead letter queue,
circuit breakers, and schedules, with an in-memory store and a PostgreSQL
store. It knows nothing about agents or turns.

Neither path owns a private copy of the turn algorithm. `everruns-core` (`engine` feature) owns
the `Execution` contract, `TurnExecution` state, Input/Reason/Act atoms, phase
ordering, and effect production. The in-process `InProcessExecution` and the
checkpointed `TurnExecution` state in `everruns-durable-engine` only select where
state lives and how work is scheduled.

## Choose a recovery boundary

- **Volatile Framework:** `Engine::new()` is offline and database-free. The
  creating Engine can resume a dropped Session, but process exit loses it.
- **Local crash-durable Framework:** `LocalConfig` stores canonical events and
  session identity locally. Rebuild the trusted Agent configuration, attach it
  to a new Engine, and resume by typed `SessionId`.
- **Distributed durable Platform:** server and workers checkpoint workflow state
  in PostgreSQL and recover across process or worker loss. Applications call it
  through the remote API or SDKs rather than configuring the facade Engine.

See [Persistence](/framework/sessions/#persistence) and [Session History and
Resume](/framework/sessions/#history-and-resume) for the exact application lifecycle.

## Extension boundaries

Normal applications depend on `everruns`. `everruns::Engine` is concrete and is
not implemented by applications. Provider integrations implement the open
`ChatDriver` boundary, while canonical storage hosts can implement
`EventLog`/`EventReader` through `everruns-core` (`host` feature).

An application that is itself an execution host may compose
`everruns_core::engine::Execution` with `everruns-core` (`host` feature) or
`everruns-durable-engine`, or implement `TurnBackend` for its own backend. Both
interfaces are experimental. That is
an advanced deployment boundary: preserve event ordering, workspace isolation,
credential separation, cancellation, and committed effect semantics. Start
with [Custom Backends](/framework/custom-backends/) before crossing it.
