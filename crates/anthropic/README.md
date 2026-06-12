# everruns-anthropic

> Anthropic Claude LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-anthropic.svg)](https://crates.io/crates/everruns-anthropic)
[![Documentation](https://docs.rs/everruns-anthropic/badge.svg)](https://docs.rs/everruns-anthropic)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-anthropic` registers an Anthropic driver with
[`everruns-core`](https://crates.io/crates/everruns-core) so the same Everruns
agent loop can run against Claude models through the provider-neutral driver
trait. It speaks the Claude Messages API with streaming, tool use, and reasoning,
and maps provider errors onto Everruns runtime errors.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) for OpenAI models,
or run with no key at all using the built-in LLM simulator in
[`everruns-runtime`](https://crates.io/crates/everruns-runtime).

## Quick Example

```rust
use everruns_anthropic::{AnthropicLlmDriver, register_driver};
use everruns_core::DriverRegistry;

let driver = AnthropicLlmDriver::new("your-api-key");

let mut registry = DriverRegistry::new();
register_driver(&mut registry);

assert!(format!("{driver:?}").contains("AnthropicLlmDriver"));
```

Register the driver into a platform and drive a full turn with
[`everruns-runtime`](https://crates.io/crates/everruns-runtime); see the
[`everruns-openai`](https://crates.io/crates/everruns-openai) README for the same
end-to-end shape with a different provider.

## What It Provides

- Claude Messages API streaming
- Registration into the Everruns `DriverRegistry` via `register_driver`
- Provider-specific error mapping into Everruns runtime errors
- Support for provider-neutral messages, tools, and reasoning metadata

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-anthropic)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
