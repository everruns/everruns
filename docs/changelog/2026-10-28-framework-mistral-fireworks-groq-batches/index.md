---
title: Framework batches add Mistral, Fireworks and Groq
description: Submit native Mistral, Fireworks and Groq batch jobs through the shared Framework API, with scoped tickets, typed request lists and runnable examples.
date: 2026-10-28
kind: improvement
significance: notable
products: [framework]
release: "0.50.1"
surfaces: [code]
links:
  docs: [/framework/batch-model-requests/, /providers/groq/]
---

![Everruns plus Mistral, Fireworks and Groq](./integration@2x.png)

Framework batches now support Mistral inline jobs, Fireworks dataset jobs and
Groq Files/Batches through the same optional driver capability. The new
[Groq provider](/providers/groq/) also supports interactive Chat Completions.
Enable `groq` for this example; use the Git dependency in
[Batch Model Requests](/framework/batch-model-requests/) until published:

```rust
use everruns::{Model, batch::Batch, providers::groq::Groq};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ticket = Batch::new(Groq::from_env()?, "reviews/groq-account")
    .request("review_001", Model::from("llama-3.1-8b-instant").completion()
        .user("Classify: delivery arrived early.").max_tokens(64))?
    .submit().await?;
let saved = serde_json::to_vec(&ticket)?;
# let _ = saved;
# Ok(())
# }
```

Persist the ticket and reconstruct the same scoped provider to collect later.
Fireworks requires explicit account and trusted artifact-origin configuration;
its cancellation operation is unsupported and never deletes a job. Examples
cover submission, resumption, collection and cancellation where supported.
