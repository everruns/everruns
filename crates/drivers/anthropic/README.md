# everruns-anthropic

> Anthropic Claude LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-anthropic.svg)](https://crates.io/crates/everruns-anthropic)
[![Documentation](https://docs.rs/everruns-anthropic/badge.svg)](https://docs.rs/everruns-anthropic)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-anthropic` registers an Anthropic driver into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider) so the same Everruns
agent loop can run against Claude models through the provider-neutral driver
trait. It speaks the Claude Messages API with streaming, tool use, and reasoning,
and maps provider errors onto Everruns runtime errors.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) for OpenAI models,
or run with no key through the offline simulator in the application-facing
[`everruns`](https://crates.io/crates/everruns) crate.

## Quick Example

```rust
use everruns_anthropic::{AnthropicChatDriver, register_driver};
use everruns_provider::DriverRegistry;

let driver = AnthropicChatDriver::new("your-api-key");

let mut registry = DriverRegistry::new();
register_driver(&mut registry);

assert!(format!("{driver:?}").contains("AnthropicChatDriver"));
```

Framework applications attach the ready-made provider through the open
`ModelSpec`/`Provider` boundary. Low-level hosts can register the driver directly.

## What It Provides

- Claude Messages API streaming
- Registration into the Everruns `DriverRegistry` via `register_driver`
- Provider-specific error mapping into Everruns runtime errors
- Support for provider-neutral messages, tools, and reasoning metadata

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-anthropic)
- [Anthropic provider guide](https://docs.everruns.com/providers/anthropic/)
- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
