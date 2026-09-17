---
title: Quickstart
description: Install everruns and run your first agent against the test simulator, then point it at a real provider.
---

## Install

Add the application-facing crate:

```bash
cargo add everruns
```

The default features include typed tool macros and the test simulator. They do
not select a model provider — see
[Supported providers](/framework/supported-providers/) for the feature or crate
to add.

## Run one turn

```rust
use everruns::{Agent, Engine, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You are a concise assistant.")
        .model(Model::simulated("Everruns is ready."))
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    let turn = session.send_and_wait("Are you ready?").await?;
    println!("{}", turn.response);
    Ok(())
}
```

`Model::simulated` follows the same model/provider path as a live provider, but
replays the configured response instead of calling one. It is a test double, not
a local model: it runs no inference, so this first program proves the wiring
rather than the answer. Keep it for tests and examples, and use a real provider
for everything else.

`send_and_wait` is the request/response convenience; use `send` when the
application needs to stream output or add steering input while a turn runs.

## Use OpenAI

Enable the provider feature and set the credential in the host environment:

```bash
cargo add everruns --features openai
export OPENAI_API_KEY=sk-...
```

Replace the simulated model:

```rust
use everruns::{Agent, OpenAI};

let agent = Agent::builder()
    .instructions("You are a concise assistant.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The model id stays credential-free. `OpenAI::from_env` configures the separate
provider at the application boundary and redacts the key from debug output.

Continue with [Framework Architecture](/framework/architecture/),
[Agents](/framework/agents/), [Tools and macros](/framework/tools-and-macros/),
and [Sessions](/framework/sessions/).
