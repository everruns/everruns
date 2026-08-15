---
title: Sessions
description: Keep conversation history across turns while isolating independent Framework sessions.
---

An `Agent` is immutable reusable behavior. An `Engine` owns session identity,
history, and runtime state. A `Session` is an engine-bound live conversation.

```rust
use everruns::{Agent, InMemoryEngine, Model};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .model(Model::simulated("Acknowledged."))
    .build()?;

let engine = InMemoryEngine::new();
let session = engine.create(agent);
let first = session.send_and_wait("My project is Atlas.").await?;
let second = session.send_and_wait("Continue with that project.").await?;

assert!(first.success);
assert!(second.success);
# Ok(())
# }
```

`Session` is always a live conversation. `send` accepts a message without
waiting for a response. If a turn is active, the message steers that turn; if
the previous turn has already finished, it starts a follow-up turn. The receipt
reports which case occurred, so applications do not need to race on session
state themselves:

```rust
# use everruns::{Agent, InMemoryEngine, Model, SendDisposition};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder().instructions("Remember input.").model(Model::simulated("Done.")).build()?;
# let engine = InMemoryEngine::new();
let session = engine.create(agent);
let initial = session.send("Plan my trip.").await?;
let latest = session.send("Prefer trains.").await?;

match latest.disposition {
    SendDisposition::Steered => {
        assert_eq!(latest.turn_id, initial.turn_id);
    }
    SendDisposition::Started => {
        // The first turn completed before the second message was accepted.
    }
    _ => {}
}

let result = latest.wait().await?;
# let _ = result;
# Ok(())
# }
```

Waiting on the latest receipt works in both cases. `send_and_wait` (also
available as the shorter `run` alias) is request/response convenience over the
same live session, not a separate mode.

The first asynchronous operation materializes the in-process host. Later turns
reuse it and send accumulated history through the same context-assembly path.
Two sessions opened on one engine have different opaque IDs and isolated
histories. The engine retains each immutable Agent snapshot, so a session keeps
working after the original Agent handle is dropped. Resume is engine-scoped:
another `InMemoryEngine` rejects the id rather than guessing its configuration.

Conversation isolation does not imply filesystem isolation. The concise
`engine.create(agent)` path permanently selects the Agent's default head before its
first inspection or turn; call `session.start().await` to make that selection
observable earlier. To fix a session to an isolated project view, bind an
[`Environment`](/framework/workspaces-and-environments/) before execution. A
session can never switch heads after it starts.

`Session::inspect` returns the context assembled for the next model call. Use it
for application assertions and debugging rather than reaching into runtime
records or backend stores.

Keep `Session::session_id()` when the application may need to reopen a
conversation. [Session History and Resume](/framework/session-history/) covers
typed resume, bounded transcript pages, and cursor snapshots. See
[Workspaces and Environments](/framework/workspaces-and-environments/) for
exact-head resume, isolation, sharing, and lifecycle. See
[Persistence](/framework/persistence/) to choose engine-lifetime memory or a
crash-durable local profile, and [Events and
cancellation](/framework/events-and-cancellation/) to observe a turn in flight.
