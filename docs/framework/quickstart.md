---
title: Quickstart
description: Install everruns and run a deterministic agent entirely offline.
---

## Install

Add the application-facing crate:

```bash
cargo add everruns
```

The default features include typed tool macros and the offline simulator. They
do not select a network provider.

## Run one turn

```rust
use everruns::{Agent, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You are a concise assistant.")
        .model(Model::simulated("Everruns is ready."))
        .build()?;

    let mut session = agent.session();
    let turn = session.run("Are you ready?").await?;
    println!("{}", turn.response);
    Ok(())
}
```

`Model::simulated` follows the same model/provider path as a live provider, but
returns the configured response deterministically. This makes the smallest
Framework program useful in tests, examples, and disconnected development.

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
    .model(OpenAI::from_env("gpt-5.6-terra")?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The model identity stays credential-free. `OpenAI::from_env` attaches the
provider configuration at the application boundary and redacts the key from
debug output.

Continue with [Agents](/framework/agents/), [Tools and macros](/framework/tools-and-macros/),
and [Sessions](/framework/sessions/).
