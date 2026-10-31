---
title: Serve uses standard OpenAI endpoint settings
description: Configure Serve with OPENAI_BASE_URL and OPENAI_API_KEY; endpoint overrides apply to OpenAI models and receive IDs without the routing prefix.
date: 2026-10-30
kind: breaking
significance: minor
products: [framework]
surfaces: [code, operations]
links:
  docs: [/framework/serve/]
  prs: [everruns/everruns#5087]
---

Serve now uses the standard OpenAI environment settings. Replace the former
Serve-specific settings in deployment configuration:

```sh
# Before
SERVE_GATEWAY_URL=https://models.example/v1
SERVE_GATEWAY_KEY=your-endpoint-key
# After
OPENAI_BASE_URL=https://models.example/v1
OPENAI_API_KEY=your-endpoint-key
```

Endpoint overrides apply to `openai/…` models and remove only the first routing
prefix. For an endpoint expecting `vendor/model`, change the agent's model from
`vendor/model` to `openai/vendor/model`:

```rust
// Cargo.toml: everruns-serve = "0.50"
use serve::Agent;

fn main() {
    let _agent = Agent::builder().model("openai/vendor/model").build();
}
```

The endpoint must support the Responses API. The explicit URL/key pair takes
precedence over OpenRouter for OpenAI models. Other provider routes and the
HTTP security checks are unchanged. See [Serve](/framework/serve/) for routing
and the shorter Bashkit example instructions.
