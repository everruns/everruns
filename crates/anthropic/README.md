# everruns-anthropic

Anthropic Claude LLM provider implementation for Everruns.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It
registers an Anthropic driver with `everruns-core` so the same agent loop can
run against Claude models through the provider-neutral Everruns driver trait.

## Quick Example

```rust
use everruns_anthropic::{AnthropicLlmDriver, register_driver};
use everruns_core::DriverRegistry;

let driver = AnthropicLlmDriver::new("your-api-key");

let mut registry = DriverRegistry::new();
register_driver(&mut registry);

assert!(format!("{driver:?}").contains("AnthropicLlmDriver"));
```

## What It Provides

- Claude Messages API streaming
- Registration into the Everruns `DriverRegistry`
- Provider-specific error mapping into Everruns runtime errors
- Support for provider-neutral messages, tools, and reasoning metadata

## License

MIT. See the repository-level `LICENSE` file.
