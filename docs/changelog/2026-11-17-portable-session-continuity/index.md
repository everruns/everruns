---
title: "Long sessions keep portable context checkpoints"
description: "The compaction capability keeps durable portable continuity, retrieves older evidence, and preserves native provider checkpoints across long sessions."
date: 2026-11-17
kind: improvement
significance: major
products: [framework, platform, cloud]
surfaces: [agent]
links:
  docs: [/advanced/compaction/]
---

Enable [`compaction`](/advanced/compaction/) to manage long-session context with
portable continuity, recent complete exchanges, and permitted history retrieval.
New configurations no longer need a separate `infinity_context` entry.

In Rust, with the default `builtins` feature:

```rust
use everruns::{Agent, CompactionConfig};

let builder = Agent::builder()
    .name("worker")
    .instructions("Complete the current task and verify older evidence when needed.")
    .capability(CompactionConfig::new().context_budget_tokens(64_000));
```

Portable summaries survive restarts and can bridge provider/model changes. Native
checkpoints retain their provider-specific compatibility. Raw history stays
queryable. Summary calls are billed and traced separately; summary fidelity still
depends on the model. The compaction divider shows available token measurements
instead of treating unchanged message counts as a failure.
