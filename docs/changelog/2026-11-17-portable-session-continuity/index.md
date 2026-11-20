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

[`compaction`](/advanced/compaction/) now manages portable continuity, recent
complete exchanges, verbatim current input, scoped reasoning replay and permitted history retrieval. Built-in harnesses inherit
it; Framework sessions without an explicit harness enable it with `builtins`.
New configurations need no separate `infinity_context` entry.

Portable summaries bridge model changes and recover after restarts on persistent
backends. Native checkpoints retain their provider/model scope. Original history
stays queryable. Summary calls are billed separately; fidelity depends on the model.

**In the UI:** Chat and Playground have a **Context** inspector. Capability
settings expose working budgets, proactive mode and summary options. Transcript
notices distinguish saved checkpoints from request-local reductions and show
available token measurements.

**In code:** `session.inspect().await?.compaction` and the context-report API
expose safe policy and checkpoint metadata. API measurements describe the last
main-model request, excluding summary calls. Impossible reductions end visibly;
storage failures retain candidate reuse and recovery.

```rust
let context = session.inspect().await?;
let checkpoint = context.compaction.checkpoint;
```
