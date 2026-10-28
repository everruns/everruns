---
title: Submit Framework completions as offline batches
description: Submit independent completions through Anthropic Message Batches, persist credential-free tickets, and collect results in another process.
date: 2026-10-27
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/batch-model-requests/]
---

Framework applications can submit independent model completions through
Anthropic Message Batches, then poll, collect or cancel them using a serializable
ticket. Results map to the original request IDs even when returned out of order.
Enable the `anthropic` feature and run the classification example:

```bash
cargo run -p everruns --features anthropic --example batch_classification -- submit /tmp/reviews-001
```

The alpha API leaves persistence and scheduling to your application. It does not
stream tokens or execute tools. Ambiguous submissions require provider
reconciliation before retrying. The [batch guide](/framework/batch-model-requests/)
covers resumption, cancellation, tenant/account scoping, supported providers and
hosting limitations, with a second example for offline evals.
