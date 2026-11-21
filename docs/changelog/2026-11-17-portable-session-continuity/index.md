---
title: "Long sessions keep portable context checkpoints"
description: "The compaction capability keeps durable portable continuity, retrieves older evidence, and preserves native provider checkpoints across long sessions."
date: 2026-11-17
kind: improvement
significance: major
products: [framework, platform, cloud]
surfaces: [ui, api, code, agent]
links:
  docs: [/advanced/compaction/]
---

[`compaction`](/advanced/compaction/) manages durable continuity, recent complete
exchanges, verbatim current input, scoped reasoning replay and history retrieval.
Built-in harnesses inherit it; implicit Framework harnesses enable it with
`builtins`. No separate `infinity_context` entry is needed.

Portable summaries bridge model changes and persistent-backend restarts. Native
checkpoints retain provider/model scope. History search includes tool observations;
excerpts show matching text, original IDs and roles. Internal summaries support
models without temperature overrides. Summary usage is separate; native estimates
stay labelled.

**In the UI:** Chat and Playground have a **Context** inspector, budget and
summary settings, and notices distinguishing saved checkpoints from request-local
reductions with available token measurements.

**In code:** `session.inspect().await?.compaction` and the context-report API expose
policy and checkpoint metadata. Measurements describe the last main-model request,
excluding summaries. Impossible reductions end visibly; storage failures retain
candidate reuse and recovery.

```rust
let context = session.inspect().await?;
let checkpoint = context.compaction.checkpoint;
```
