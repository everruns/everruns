---
title: Events and Cancellation
description: Subscribe to live Framework session events and cancel a turn cooperatively.
---

Subscribe before sending a message to observe its live event projection:

```rust
use everruns::{Agent, InMemoryEngine, Model};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Be concise.")
    .model(Model::simulated("Done."))
    .build()?;
let session = InMemoryEngine::new().create(agent);
let mut events = session.events();

let pending = session.send("Start.").await?;
while let Some(event) = events.recv().await? {
    println!("{}", event.event_type());
    if event.turn_id.as_deref() == Some(&pending.turn_id) && event.kind.is_terminal() {
        break;
    }
}
let turn = pending.wait().await?;
assert!(turn.success);
# Ok(())
# }
```

Known events have typed `SessionEventKind` values. Unknown canonical event types
are preserved as `Other` with their payload, so the projection does not silently
drop information. The feed is live and non-blocking: `send` starts execution in
the session's background actor, a slow or dropped consumer does not stop the
turn, and the feed is not a durable replay API.

Use [lifecycle hooks](/framework/lifecycle-hooks/) instead when application work
must be awaited at an execution boundary or its failure must affect the run.

See [Canonical Framework events](/framework/canonical-events/) for lossless
envelopes, explicit lag handling, ordering, and the durability boundary.
Use [Session History and Resume](/framework/session-history/) to rebuild a
bounded persisted transcript after live lag or a process restart.

## Cancel a turn

A message receipt exposes the specific accepting turn, so live applications
can cancel without racing against whichever turn is active later:

```rust
# use everruns::{Agent, InMemoryEngine, LlmSimConfig, Model};
# use std::time::Duration;
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let model = Model::simulated_with_config(LlmSimConfig::fixed("Done.").with_response_delay(Duration::from_millis(100)));
# let agent = Agent::builder().instructions("Be concise.").model(model).build()?;
# let session = InMemoryEngine::new().create(agent);
let pending = session.send("Start.").await?;
pending.turn().cancel().await?;
let cancelled = pending.wait().await?;
assert!(!cancelled.success);
# Ok(())
# }
```

`run_with` retains cancellation-token convenience for request/response calls:

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
