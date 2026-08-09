---
title: Models and Providers
description: Select credential-free model identities and attach provider implementations to Framework agents.
---

The Framework separates **what model to use** from **how to reach it**:

- `ModelSpec` is a credential-free provider and model identity.
- `Provider` supplies the driver, endpoint, and authentication needed by the host.
- `Model` pairs those values for an `Agent`.

This boundary is open: a new provider does not require a new closed enum variant
or provider-specific branch in application code.

## Offline simulation

```rust
use everruns::{Agent, Model};

let agent = Agent::builder()
    .instructions("Answer deterministically.")
    .model(Model::simulated("fixed response"))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

## OpenAI convenience

With the `openai` feature, `OpenAI::from_env` reads `OPENAI_API_KEY` and the
optional `OPENAI_BASE_URL`:

```rust
use everruns::{Agent, OpenAI};

let agent = Agent::builder()
    .instructions("Be concise.")
    .model(OpenAI::from_env("gpt-5.6-terra")?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `OpenAI::new(model, key)` when the host already owns an explicitly resolved
credential. Never put credentials in a `ModelSpec`, log them as model identity,
or select provider behavior with vendor-specific detection.

## Explicit assembly

Applications with their own driver can use the shared boundary directly:

```rust
use everruns::{Agent, BuildError, ChatDriver, Model, ModelSpec, Provider};

fn agent_for(driver: impl ChatDriver) -> Result<Agent, BuildError> {
    let provider = Provider::new("acme", driver);
    let model = Model::with_provider(
        ModelSpec::on("acme", "assistant-v1"),
        provider,
    );
    Agent::builder()
        .instructions("Use the configured provider.")
        .model(model)
        .build()
}
```

For a complete driver boundary, see [Custom providers](/framework/custom-providers/).
