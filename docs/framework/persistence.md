---
title: Persistence
description: Understand the current Framework persistence boundary and local application state.
---

Framework sessions keep conversation history in memory by default. Reusing the
same `Session` preserves context across turns; dropping it ends that live
conversation.

For local applications, the feature-gated `LocalConfig` supplies a trusted
real-disk workspace plus SQLite-backed task and schedule state:

```rust
use everruns::{Agent, LocalConfig, Model};

let local = LocalConfig::new(".everruns-data").workspace("./workspace");
let agent = Agent::builder()
    .instructions("Work inside the configured workspace.")
    .model(Model::simulated("Ready."))
    .local(local)
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Enable it with `cargo add everruns --features local`. Select both directories
from trusted application configuration. `LocalConfig` does not claim to make
conversation history durable.

## Compatibility persistence

The `jsonl` feature and its writable message-store APIs remain available only
for source compatibility with existing 0.17.x applications. They are not the
recommended persistence model for new Framework applications. Durable
conversation truth belongs to canonical events; history and context are
projections of that record.

For existing JSONL code and low-level storage hosts, see [Runtime
compatibility](/framework/runtime-compatibility/). Do not design new application
persistence around a writable message store or its file format.
