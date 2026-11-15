---
title: Framework shares trace projection and evaluation
description: Project session events into portable traces and run shared scoring rules, explicit model graders, bounded cases and online observation in Framework.
date: 2026-11-15
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/evaluation-and-traces/]
---

Framework applications can project canonical events into turns and steps using the same fold as Server traces. The optional `evaluation` feature adds shared deterministic scoring rules, explicitly selected judge/decision models, bounded case concurrency and an online scoring listener.

```rust
use everruns::evaluation::{Evaluator, Scorer};

let evaluator = Evaluator::new(vec![
    Scorer::Contains { text: "42".into(), weight: 1.0 },
])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

[Evaluation and traces](/framework/evaluation-and-traces/) includes a runnable real-model example. Results retain evidence and session identities for debugging. Online grading reports incomplete evidence and bounded-retention failures instead of silently grading partial data; applications supply their result sink.

## Update 2026-11-15

File-content scoring now uses the same predicate and score explanation in Server and Framework. Each host still loads its own evidence and decides how unavailable files are reported.
