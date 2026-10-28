---
title: Submit Framework batches from message lists
description: Submit existing typed messages and request configuration as a batch, reuse the client for collection, and keep the completion builder available.
date: 2026-10-28
kind: improvement
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/batch-model-requests/]
---

Framework batches now accept a list of typed requests through
`Batch::submit_requests`. Reuse existing messages and configuration without
building a `Completion` for each request. This API is on `main` and is not in
the published `0.49.0` crate; use the Git dependency in the
[batch guide](/framework/batch-model-requests/), then enable `anthropic`:

```rust
use everruns::{
    batch::{Batch, BatchRequest}, llm::{Message, MessageRole},
    providers::anthropic::Anthropic, LlmCallConfig,
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut config = LlmCallConfig::new("claude-sonnet-5");
config.max_tokens = Some(64);
let client = Batch::new(Anthropic::from_env()?, "reviews/anthropic-account");
let ticket = client.submit_requests(vec![BatchRequest {
    id: "review_001".into(),
    messages: vec![Message::text(MessageRole::User, "Classify: delivery arrived early.")],
    config,
}]).await?;
# let _ = ticket;
# Ok(())
# }
```

The same client can poll, collect or cancel the returned ticket. The builder
remains available, and both input paths share validation, tenant/account scoping
and uncertain-submission handling. See [Batch Model Requests](/framework/batch-model-requests/)
for both styles and the updated runnable eval example.

## Update 2026-10-28

The same batch API now supports OpenAI Responses, OpenAI Chat Completions and
Gemini inline jobs. See [Batch Model Requests](/framework/batch-model-requests/)
for provider limits and the updated multi-provider examples.
