---
title: Quickstart
description: Install everruns and run a real agent against a live model provider in about five minutes.
---

## Install

Add the application-facing crate with a model provider:

```bash
cargo add everruns --features openai
export OPENAI_API_KEY=sk-...
```

`--features openai` bundles the OpenAI driver. Any other provider is its own
crate — see [Supported providers](/framework/supported-providers/).

## Run one turn

```rust
use everruns::{Agent, Engine, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You are a concise assistant.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .build()?;

    let engine = Engine::new();
    let session = engine.create(agent);
    let turn = session.send_and_wait("Explain durable execution in one sentence.").await?;
    println!("{}", turn.response);
    Ok(())
}
```

The model id stays credential-free: `.model("…")` names the model, and
`.provider(…)` supplies the driver, endpoint, and key separately.
`OpenAI::from_env` reads `OPENAI_API_KEY` and redacts it from debug output.

`send_and_wait` is the request/response convenience; use `send` when the
application needs to stream output or steer a turn while it runs.

## Give it a tool

An agent becomes useful when it can act. Annotate a function and hand it over:

```rust
use everruns::{Agent, OpenAI};

#[everruns::tool]
/// Look up the current stock level for a SKU.
async fn stock_level(sku: String) -> Result<u32, String> {
    Ok(inventory_lookup(&sku).await)
}

# async fn inventory_lookup(_sku: &str) -> u32 { 0 }
# fn build() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer inventory questions. Use the tool rather than guessing.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .tool(stock_level())
    .build()?;
# let _ = agent;
# Ok(())
# }
```

The macro derives the JSON schema from the signature, so the model sees typed
arguments and the code stays ordinary Rust. See
[Tools and macros](/framework/tools-and-macros/).

## Where to go next

- [Agents](/framework/agents/) — instructions, files, workspaces, and MCP.
- [Tools and macros](/framework/tools-and-macros/) — typed function tools.
- [Sessions](/framework/sessions/) — multi-turn state and history.
- [Framework architecture](/framework/architecture/) — how the pieces fit.

## Testing without a provider

Once you are building for real, you will want tests that do not call a model.
`Model::simulated` replays a canned response through the same model/provider
path:

```rust
use everruns::{Agent, Engine, Model};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("You are a concise assistant.")
    .model(Model::simulated("Everruns is ready."))
    .build()?;

let turn = Engine::new().create(agent).send_and_wait("Are you ready?").await?;
assert_eq!(turn.response, "Everruns is ready.");
# Ok(())
# }
```

It is a test double, not a local model: it runs no inference, so it proves your
wiring rather than any model behavior. Use it in tests and CI, not as a way to
run Everruns without a provider. See
[Testing and simulation](/framework/testing-and-simulation/).
