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
Mistral's Responses API preview. Framework callers select the protocol on the provider connection:

```rust
use everruns::{Mistral, MistralApi};

let provider = Mistral::from_env()?.api(MistralApi::Responses);
```

The integration streams text, reasoning and function calls, replays full
conversation history, and accepts images and files in user messages and tool
results. JSON Schema output, token usage and prompt-cache routing use the same
call configuration. Chat Completions remains the default.

Responses is stateless: Everruns sends `store: false` and does not chain response
IDs. Non-streaming calls collect the stream; incomplete or interrupted responses
fail the call. The provider guide lists the preview's configuration and limits.

OpenAI uses the same `.api(...)` pattern with `OpenAIApi`. Every production
provider now has a typed Framework configuration, with its authentication
requirements kept explicit. Hosted provider create/update requests accept `api`,
validate it against the driver catalog, and persist it for direct and remote
workers. Shared per-call settings remain provider-neutral.
