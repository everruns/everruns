---
title: Testing and Simulation
description: Test Framework applications deterministically without network access or provider credentials.
---

`Model::simulated` is the default testing tool. It uses the normal provider
resolution and execution path while returning a fixed response locally.

```rust
use everruns::{Agent, InMemoryEngine, Model};

# #[tokio::test]
# async fn agent_follows_the_application_flow() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Return the configured result.")
    .model(Model::simulated("approved"))
    .build()?;

let session = InMemoryEngine::new().create(agent);
let turn = session.send_and_wait("Review this.").await?;
assert!(turn.success);
assert_eq!(turn.response, "approved");
# Ok(())
# }
```

Useful test layers are:

1. Build-time validation tests for agent, tool, model, MCP, and compaction configuration.
2. Offline session tests with `Model::simulated`.
3. Context assertions through `Session::inspect`.
4. Event/cancellation tests through the public session API.
5. A small opt-in live-provider suite for protocol integration.

Keep normal tests credential-free and deterministic. Do not make a live model's
wording or tool choice a unit-test oracle. Temporary directories should own
workspace and local-state tests so they do not read or modify developer data.

The runnable programs in [Framework examples](/framework/examples/) are also compiled
in CI using only the public `everruns` facade.
