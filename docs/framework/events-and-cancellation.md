---
title: Events and Cancellation
description: Subscribe to live Framework session events and cancel a turn cooperatively.
---

Subscribe before running a turn to observe its live event projection:

```rust
use everruns::{Agent, Model};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Be concise.")
    .model(Model::simulated("Done."))
    .build()?;
let mut session = agent.session();
let mut events = session.events();

let turn = session.run("Start.").await?;
while let Some(event) = events.try_recv() {
    println!("{}", event.event_type());
}
assert!(turn.success);
# Ok(())
# }
```

Known events have typed `SessionEventKind` values. Unknown runtime event types
are preserved as `Other` with their payload, so the projection does not silently
drop information. The feed is live and non-blocking: a slow or dropped consumer
does not stop the turn, and it is not a durable replay API.

Use [lifecycle hooks](/framework/lifecycle-hooks/) instead when application work
must be awaited at an execution boundary or its failure must affect the run.

## Cancel a turn

```rust
use everruns::{CancellationToken, RunOptions};

let cancel = CancellationToken::new();
let options = RunOptions::new().cancel_token(cancel.clone());
cancel.cancel();

let turn = session.run_with("Stop before starting.", options).await?;
assert!(!turn.success);
# Ok::<(), everruns::RunError>(())
```

Cancellation is cooperative. Cancelling drops the in-flight turn future and
tears down tool work through the same runtime path; it does not kill the host
process or provide an independent transaction boundary.
