---
title: Framework agents start about 3.5 times faster
description: Building an agent and creating a session in the Everruns Framework now takes roughly a third of the time, with no change to steady-state behavior.
date: 2026-09-08
kind: improvement
significance: minor
products: [framework]
surfaces: [code]
release: "0.24.0"
links:
  docs: [/framework/agents/]
  prs: [everruns/everruns#3486]
---

Agent startup to ready in the Everruns Framework is about 3.5 times faster. In
release builds, `main` to ready dropped from 1.9-2.2 ms to 0.50-0.64 ms and
`Agent::build` from 1.0-1.2 ms to 0.14-0.26 ms.

Two costs left the startup path: provider HTTP clients are now resolved per
request instead of in each driver constructor, and a session's event broadcast
channel is allocated when the first subscriber arrives rather than with the
session. A subscriber still never receives events sent before it subscribed.
No code changes are needed: the same engine and session calls start faster.

```rust
# use everruns::{Agent, Engine, Harness, Model};
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder().instructions("Tell dad jokes.").model(Model::simulated("An impasta.")).build()?;
let engine = Engine::new();
let session = engine
    .create(agent)
    .harness(Harness::conversation())
    .start()
    .await?;
let response = session.send_and_wait("Tell me a joke.").await?;
# Ok(())
# }
```

See [Agents](/framework/agents/). [#3486](https://github.com/everruns/everruns/pull/3486)
