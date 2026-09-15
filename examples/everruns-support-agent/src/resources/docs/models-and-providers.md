---
title: Models and Providers
description: Select credential-free model identities and attach provider implementations to Framework agents.
---

The Framework separates **what model to use** from **how to reach it**:

- `.model("id")` selects the provider-visible model with a credential-free string.
- `Provider` supplies the driver, endpoint, and authentication needed by the host.
- An agent currently accepts one provider, configured separately with `.provider(...)`.

This boundary is open: a new provider does not require a new closed enum variant
or provider-specific branch in application code. The Framework constructs its
execution-facing model specification internally when the agent builds.

## Offline simulation

```rust
use everruns::{Agent, Model};

let agent = Agent::builder()
    .instructions("Answer deterministically.")
    .model(Model::simulated("fixed response"))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

`Model::simulated` is backed by the focused `everruns-llmsim` crate. Depend on
that crate directly when building a low-level host or scripting multi-turn
provider behavior; ordinary Framework applications need only `everruns`.

## OpenAI convenience

With the `openai` feature, `OpenAI::from_env` reads `OPENAI_API_KEY` and the
optional `OPENAI_BASE_URL`:

```rust
use everruns::{Agent, OpenAI};

let agent = Agent::builder()
    .instructions("Be concise.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `OpenAI::new(key)` when the host already owns an explicitly resolved
credential. Never put credentials in a model id, log them as model identity, or
select provider behavior with vendor-specific detection.

## Explicit assembly

Applications with their own driver can use the shared boundary directly:

```rust
use everruns::{Agent, BuildError, ChatDriver, Provider};

fn agent_for(driver: impl ChatDriver) -> Result<Agent, BuildError> {
    Agent::builder()
        .instructions("Use the configured provider.")
        .provider(Provider::new("acme", driver))
        .model("assistant-v1")
        .build()
}
```

For a complete driver boundary, see [Custom providers](/framework/custom-providers/).
