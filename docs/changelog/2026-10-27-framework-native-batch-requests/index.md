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
This API is on `main` and is not included in the published `0.49.0` crate; use
the pinned Git dependency in the [batch guide](/framework/batch-model-requests/).
Enable the `anthropic` feature, submit a completion, and save its ticket:

```rust
use everruns::{batch::Batch, providers::anthropic::Anthropic, Model};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let model = Model::from("claude-sonnet-5");
let ticket = Batch::new(Anthropic::from_env()?, "reviews/anthropic-account")
    .request("review_001", model.completion()
        .user("Classify as positive, negative, or neutral: delivery arrived early.")
        .max_tokens(64))?
    .submit().await?;
let saved_ticket = serde_json::to_vec(&ticket)?;
# let _ = saved_ticket;
# Ok(())
# }
```

The alpha API leaves persistence and scheduling to your application. It does not
stream tokens or execute tools. Ambiguous submissions require provider
reconciliation before retrying. The [batch guide](/framework/batch-model-requests/)
covers resumption, cancellation, tenant/account scoping, supported providers and
hosting limitations, with a second example for offline evals.

## Update 2026-10-28

You can also [submit existing message lists](/changelog/2026-10-28-framework-batch-message-lists/)
through the same batch client.
