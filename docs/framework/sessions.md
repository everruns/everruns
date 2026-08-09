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

The first turn materializes the in-process host. Later turns reuse it and send
the accumulated history through the same context-assembly path. Two sessions
opened from the same agent have different opaque IDs and do not share history.

`Session::inspect` returns the context assembled for the next model call. Use it
for application assertions and debugging rather than reaching into runtime
records or backend stores.

Dropping an ordinary session drops its in-memory conversation. See
[Persistence](/framework/persistence/) for the current boundary and [Events and
cancellation](/framework/events-and-cancellation/) for observing a turn in flight.
