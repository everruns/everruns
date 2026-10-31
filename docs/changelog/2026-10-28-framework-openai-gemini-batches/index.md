---
title: Run Framework batches on OpenAI and Gemini
description: Use the same scoped batch API with OpenAI Responses, Chat Completions and Gemini inline jobs, including resumable tickets and explicit cancellation.
date: 2026-10-28
kind: improvement
significance: notable
products: [framework]
release: "0.50.0"
surfaces: [code]
links:
  docs: [/framework/batch-model-requests/]
---

Framework batches now support OpenAI Responses, OpenAI Chat Completions and
Gemini inline jobs through the existing driver capability. Typed request lists
and completion builders use the same submit, poll, collect and cancel API.
Enable `openai` for this example; this unreleased API requires the Git dependency
in [Batch Model Requests](/framework/batch-model-requests/):

```rust
use everruns::{Model, batch::Batch, providers::openai::OpenAI};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ticket = Batch::new(OpenAI::from_env()?, "reviews/openai-account")
    .request("review_001", Model::from("gpt-6.1-sol").completion()
        .user("Classify: delivery arrived early.").max_tokens(64))?
    .submit().await?;
let saved = serde_json::to_vec(&ticket)?;
# let _ = saved;
# Ok(())
# }
```

Persist the ticket in application storage and reconstruct the same scoped
provider to collect later. Gemini uses its `gemini` feature and `Gemini` provider.
Provider limits and cancellation outcomes remain explicit; ambiguous uploads or
submissions require reconciliation before creating more work.

## Update 2026-10-28

The same API also supports Mistral inline jobs, Groq Files/Batches and Fireworks
dataset jobs. Fireworks requires explicit account/artifact configuration and
does not support cancellation.
