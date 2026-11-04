---
title: Groq
description: Use Groq Chat Completions and native offline batches from Everruns Framework, with scoped resumable tickets and explicit provider configuration.
sidebar:
  label: Groq
---

Groq is a first-class Framework driver for OpenAI-compatible Chat Completions.
Enable `groq`, set `GROQ_API_KEY`, and select a model your account can use. The
optional `GROQ_BASE_URL` overrides the API base. Model discovery runs only against
`api.groq.com`; a custom endpoint must speak the corresponding protocol.

This addition targets direct Framework applications. It does not add a Groq
provider picker or credential storage integration to the Platform.

The API is on `main`; published `0.49.0` does not include it. Use the Git
dependency from [Batch Model Requests](/framework/batch-model-requests/) with
`features = ["groq"]` until a release includes it.

```rust
use everruns::{Model, providers::groq::Groq};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let reply = Model::new("llama-3.1-8b-instant", Groq::from_env()?)
    .completion().user("What is the capital of France?").max_tokens(64)
    .send().await?;
println!("{}", reply.text);
# Ok(())
# }
```

For offline jobs, the same driver exposes the optional native batch capability.
It uploads JSONL through Groq Files, creates a native batch, polls it, collects
results by request ID, and explicitly cancels whole jobs. It uses a 24-hour
window, one model per job, and a maximum of 50,000 requests / 200 MB including
the upload envelope. Audio batches and Responses batching are outside this API.

Persist tickets in access-controlled application storage. Reconstruct the same
provider and tenant/account scope to collect later; ambiguous file upload or job
creation requires reconciliation. See the [Groq native batch reference](https://console.groq.com/docs/batch)
for current model eligibility, quotas and retention.
