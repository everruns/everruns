---
title: Sessions
description: Keep conversation history across turns while isolating independent Framework sessions.
---

An `Agent` is reusable configuration. A `Session` is one live conversation with
that agent.

```rust
use everruns::{Agent, Model};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .model(Model::simulated("Acknowledged."))
    .build()?;

let mut session = agent.session();
let first = session.run("My project is Atlas.").await?;
let second = session.run("Continue with that project.").await?;

assert!(first.success);
assert!(second.success);
# Ok(())
# }
```

The first asynchronous operation materializes the in-process host. Later turns
reuse it and send the accumulated history through the same context-assembly
path. Two sessions opened from the same agent have different opaque IDs and
isolated histories even though the Agent owns their shared event backend.

`Session::inspect` returns the context assembled for the next model call. Use it
for application assertions and debugging rather than reaching into runtime
records or backend stores.

Keep `Session::session_id()` when the application may need to reopen a
conversation. [Session History and Resume](/framework/session-history/) covers
typed resume, bounded transcript pages, and cursor snapshots. See
[Persistence](/framework/persistence/) to choose Agent-lifetime memory or a
crash-durable local profile, and [Events and
cancellation](/framework/events-and-cancellation/) to observe a turn in flight.
