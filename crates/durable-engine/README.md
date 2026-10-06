# everruns-durable-engine

> Durable turn backend for Everruns: agent turns as queued, checkpointed steps on
> `everruns-durable`, behind core's `TurnBackend` seam.

[![Crates.io](https://img.shields.io/crates/v/everruns-durable-engine.svg)](https://crates.io/crates/everruns-durable-engine)
[![Documentation](https://docs.rs/everruns-durable-engine/badge.svg)](https://docs.rs/everruns-durable-engine)
[![License](https://img.shields.io/crates/l/everruns-durable-engine.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

**Status: experimental.** The `TurnBackend` seam this crate implements is
outside the Everruns API stability promises until the backend conformance suite
passes on every backend. Expect breaking changes between releases.

Part of the [Everruns](https://everruns.com) agent framework. An
in-process agent turn lives and dies with the task that runs it. This crate
runs the same turn durably: each step (`process_input`, `reason`, `act`) is a
task on [`everruns-durable`](https://crates.io/crates/everruns-durable)'s queue,
and the turn's state is checkpointed after every step, so a crashed worker's
turn is reclaimed and continues from its last step instead of starting over.

```sh
cargo add everruns-durable-engine
```

## What It Provides

- `DurableRunner`, which implements `everruns_core::host::TurnBackend`: start a
  turn, cancel it, and ask whether a session runs one. Starting a turn creates
  or claims the session's workflow and enqueues its first step before it
  returns; the `TurnTicket` it hands back resolves when the workflow ends.
- `DurableBackend`, which runs a framework application's turns with workers in
  its own process, each step on the session's `InProcessRuntime`:
  `DurableBackend::memory` over an in-memory store, `DurableBackend::postgres`
  over a PostgreSQL store several processes may share, each claiming only its
  own sessions' steps. The `everruns` facade selects it with its `durable`
  feature.
- `TurnTaskDriver`, which runs one claimed turn task against any `TurnStore`:
  cancellation check, heartbeat, the step, completion or failure, then the next
  step's enqueue or the workflow's completion. A `TurnTaskHost` supplies the
  runtime host each step runs on.
- `TurnStore`, the one store contract the driver and the runner share. Any
  `everruns-durable` `WorkflowEventStore` is a `TurnStore`, so the in-memory
  store (tests and development) and the PostgreSQL store (on
  `everruns-durable`'s own schema) work out of the box.

The crate carries no transport. The Everruns platform worker reaches the store
over gRPC: it implements `TurnStore` for its own
client and builds its runner with `DurableRunner::from_store`. Another process
boundary plugs in the same way.

## Quick start

A turn started from a message the host already persisted, then cancelled,
against the in-memory store. With nothing driving the queue the turn waits at
its first step, so cancelling it ends the ticket with `Cancelled`.

```rust
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_durable_engine::DurableRunner;
use everruns_durable_engine::host::{PersistedTurn, TurnBackend, TurnInput, TurnRequest};

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> everruns_contracts::error::Result<()> {
let runner = DurableRunner::new_in_memory();
let session_id = SessionId::new();

let ticket = runner
    .start_turn(TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::Persisted(Box::new(PersistedTurn::Message {
            org_id: 1,
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            request_id: None,
        })),
    ))
    .await?;
assert!(runner.is_running(session_id).await);

assert!(runner.cancel(session_id).await?);
assert!(matches!(ticket.await, Err(AgentLoopError::Cancelled)));
# Ok(())
# }
```

On PostgreSQL, apply `everruns-durable`'s schema and build the runner over the
pool:

```rust,no_run
use everruns_durable_engine::DurableRunner;
use everruns_durable_engine::durable::{PostgresPool, PostgresWorkflowEventStore};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let pool = PostgresPool::connect("postgres://localhost/my_app").await?;
PostgresWorkflowEventStore::migrate(&pool).await?;
let runner = DurableRunner::new_with_pool(pool);
# let _ = runner;
# Ok(())
# }
```

Turns advance only while something claims and runs their tasks: a worker loop
that claims `process_input`, `reason` and `act` tasks from the store and hands
each to a `TurnTaskDriver`.

## Documentation

- [Durable execution](https://docs.everruns.com/explanation/durable-execution/)
- [API reference](https://docs.rs/everruns-durable-engine)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
