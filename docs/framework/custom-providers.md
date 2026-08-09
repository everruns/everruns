---
title: Custom Providers
description: Implement and attach a custom model provider through the open Framework driver boundary.
---

Use a custom provider when an application talks to a model service that the
Framework does not configure for you. The extension boundary is the public
`ChatDriver` trait plus a credential-free `ModelSpec` and a `Provider` value.

At a high level:

```rust
use everruns::{Agent, BuildError, ChatDriver, Model, ModelSpec, Provider};

fn agent_for(driver: impl ChatDriver) -> Result<Agent, BuildError> {
    let provider = Provider::new("company-gateway", driver);
    let model = Model::with_provider(
        ModelSpec::on("company-gateway", "assistant-v2"),
        provider,
    );

    Agent::builder()
        .instructions("Use the company model gateway.")
        .model(model)
        .build()
}
```

A driver implements the streaming chat-completion contract. It receives the
resolved endpoint, model-facing messages, and call configuration, and returns
an `LlmResponseStream`. Exact trait methods and event shapes live in the
[`everruns::ChatDriver` API reference](https://docs.rs/everruns/latest/everruns/trait.ChatDriver.html).

Keep credential lookup and refresh in trusted host/provider configuration.
`ModelSpec` must remain safe to log, compare, store, and pass across application
boundaries. Provider errors should preserve useful classifications without
including secrets.

Use focused provider crates when they already implement the protocol you need.
Custom backends and provider registry topology belong to [low-level host
composition](/framework/custom-backends/), not ordinary model selection.
