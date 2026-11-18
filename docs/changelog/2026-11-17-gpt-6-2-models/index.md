---
title: GPT-6.2 Astra and Sol join the model catalog
description: Use GPT-6.2 Astra and Sol with current pricing, reasoning and speed controls. Latest-model examples move forward while Luna stays the budget default.
date: 2026-11-17
kind: feature
significance: notable
products: [platform, framework]
surfaces: [api]
links:
  docs: [/providers/openai/, /framework/models-and-providers/]
---

GPT-6.2 Astra and Sol are available in the model catalog with pricing, context
limits, reasoning controls, and Standard, Fast and Ultrafast tiers. They replace
older Astra/Sol recommendations on Responses providers; explicit older model
IDs remain supported. Luna remains the budget default.

In Framework code, select the current balanced model (requires the `openai`
feature and `OPENAI_API_KEY`):

```rust
use everruns::{Agent, OpenAI};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _agent = Agent::builder()
        .provider(OpenAI::from_env()?)
        .model("gpt-6.2-sol")
        .instructions("Answer concisely.")
        .build()?;
    Ok(())
}
```

Use Responses for function tools. Neither model accepts `none` reasoning or
Flex. Ultrafast costs 3x Standard through February 28, 2027, then 6x from
March 1 Pacific time. See [OpenAI](/providers/openai/) and
[models and providers](/framework/models-and-providers/) for configuration.
