# everruns-openai

> OpenAI LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-openai.svg)](https://crates.io/crates/everruns-openai)
[![Documentation](https://docs.rs/everruns-openai/badge.svg)](https://docs.rs/everruns-openai)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-openai` registers OpenAI drivers into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider) so the same Everruns
agent loop can run against OpenAI models. It ships the recommended Responses API
driver plus a Chat Completions compatibility driver for OpenAI-compatible
endpoints, mapping Everruns' provider-neutral messages, tools, and reasoning
onto the OpenAI wire format.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for Claude
models. Framework applications use the application-facing
[`everruns`](https://crates.io/crates/everruns) crate; its simulator runs
offline without a key.

## Quick Example: Agent With OpenAI

```rust,no_run
use everruns::{Agent, InMemoryEngine, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Answer clearly and concisely.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .build()?;
    let result = InMemoryEngine::new()
        .create(agent)
        .run("Write one sentence about reliable agents.")
        .await?;

    println!("{}", result.response);
    Ok(())
}
```

## Driver-Only Example

```rust
use everruns_openai::OpenAIChatDriver;

let driver = OpenAIChatDriver::new("your-api-key");
assert!(!driver.uses_custom_url());
```

## What It Provides

- A Responses API driver (recommended) and a Chat Completions compatibility driver
- Registration into the Everruns `DriverRegistry` via `register_driver`
- `base_url` override for OpenAI-compatible endpoints
- Streaming, tool calls, and reasoning mapped to provider-neutral Everruns types

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-openai)
- [OpenAI provider guide](https://docs.everruns.com/providers/openai/)
- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
