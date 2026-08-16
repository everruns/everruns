# everruns-openrouter

> OpenRouter LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-openrouter.svg)](https://crates.io/crates/everruns-openrouter)
[![Documentation](https://docs.rs/everruns-openrouter/badge.svg)](https://docs.rs/everruns-openrouter)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-openrouter` registers the OpenRouter driver into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider) so the same Everruns
agent loop can run against OpenRouter's model catalog. OpenRouter exposes an
OpenAI-compatible Responses API, so the driver wraps the core Open Responses
protocol driver and parses OpenRouter's richer `/models` metadata into capability
profiles (notably reasoning support).

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) for OpenAI models,
or [`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for Claude
models.

## Driver-Only Example

```rust
use everruns_openrouter::OpenRouterChatDriver;

let driver = OpenRouterChatDriver::new("your-api-key");
assert!(!driver.uses_custom_url());
```

## What It Provides

- An OpenRouter Responses API driver wrapping the Everruns Open Responses protocol
- Registration into the Everruns `DriverRegistry` via `register_driver`
- `base_url` override for OpenRouter-compatible endpoints
- Capability profiling derived from OpenRouter's `/models` metadata

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-openrouter)
- [OpenRouter provider guide](https://docs.everruns.com/providers/openrouter/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
