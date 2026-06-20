# everruns-openai

> OpenAI LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-openai.svg)](https://crates.io/crates/everruns-openai)
[![Documentation](https://docs.rs/everruns-openai/badge.svg)](https://docs.rs/everruns-openai)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-openai` registers OpenAI drivers with
[`everruns-core`](https://crates.io/crates/everruns-core) so the same Everruns
agent loop can run against OpenAI models. It ships the recommended Responses API
driver plus a Chat Completions compatibility driver for OpenAI-compatible
endpoints, mapping Everruns' provider-neutral messages, tools, and reasoning
onto the OpenAI wire format.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for Claude
models, or run with no key at all using the built-in LLM simulator in
[`everruns-runtime`](https://crates.io/crates/everruns-runtime).

## Quick Example: Agent With OpenAI

```rust,no_run
use everruns_core::{
    CapabilityRegistry, DriverId, DriverRegistry, PlatformDefinition, ResolvedModel,
};
use everruns_runtime::InProcessRuntimeBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut drivers = DriverRegistry::new();
    everruns_openai::register_driver(&mut drivers);

    let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .default_model(ResolvedModel {
            model: "gpt-5.4-mini".into(),
            provider_type: DriverId::OpenAI,
            api_key: Some(std::env::var("OPENAI_API_KEY")?),
            base_url: None,
            provider_metadata: None,
        })
        .single_session(|s| {
            s.harness("assistant", "You are a helpful assistant.")
                .agent("openai-agent", "Answer clearly and concisely.")
                .session_title("OpenAI example")
        })
        .build()
        .await?;

    let session_id = runtime.default_session_id().expect("single_session id");
    let result = runtime
        .run_text_turn(session_id, "Write one sentence about reliable agents.")
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
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
