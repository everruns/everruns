---
title: Custom Providers
description: Implement and attach a custom model provider through the open Framework driver boundary.
---

Use a custom provider when an application talks to a model service that the
Framework does not configure for you. The extension boundary is the public
`ChatDriver` trait plus a `Provider` value. The agent selects that provider's
model with a plain credential-free string id.

At a high level:

```rust
use everruns::{Agent, BuildError, ChatDriver, Provider};

fn agent_for(driver: impl ChatDriver) -> Result<Agent, BuildError> {
    Agent::builder()
        .instructions("Use the company model gateway.")
        .provider(Provider::new("company-gateway", driver))
        .model("assistant-v2")
        .build()
}
```

A driver implements the streaming chat-completion contract. It receives the
resolved endpoint, model-facing messages, and call configuration, and returns
an `LlmResponseStream`. Exact trait methods and event shapes live in the
[`everruns::ChatDriver` API reference](https://docs.rs/everruns/latest/everruns/trait.ChatDriver.html).

Keep credential lookup and refresh in trusted host/provider configuration.
Model ids must remain safe to log, compare, store, and pass across application
boundaries. Provider errors should preserve useful classifications without
including secrets.

Use focused provider crates when they already implement the protocol you need.
Custom backends and provider registry topology belong to [low-level host
composition](/framework/custom-backends/), not ordinary model selection.
