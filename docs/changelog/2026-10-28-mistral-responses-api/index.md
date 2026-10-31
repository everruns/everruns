---
title: "Mistral Responses API support in Everruns"
description: "Framework callers can opt into Mistral's stateless Responses API with streaming, tools, reasoning replay, structured output, and image or file inputs."
date: 2026-10-28
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/providers/mistral/]
---

![Everruns and Mistral Responses API](./integration@2x.png)

The [Mistral provider](/providers/mistral/) now supports
Mistral's Responses API preview. Framework callers opt in per call:

```rust
use everruns_contracts::LlmCallConfig;
use everruns_drivers::mistral::{MistralApi, MistralConfigExt};

let config = LlmCallConfig::new("mistral-large-4")
    .with_mistral_api(MistralApi::Responses);
```

The integration streams text, reasoning and function calls, replays full
conversation history, and accepts images and files in user messages and tool
results. JSON Schema output, token usage and prompt-cache routing use the same
call configuration. Chat Completions remains the default.

Responses is stateless: Everruns sends `store: false` and does not chain response
IDs. Non-streaming calls collect the stream; incomplete or interrupted responses
fail the call. The provider guide lists the preview's configuration and limits.
