# everruns-fireworks

> Fireworks AI LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-fireworks.svg)](https://crates.io/crates/everruns-fireworks)
[![Documentation](https://docs.rs/everruns-fireworks/badge.svg)](https://docs.rs/everruns-fireworks)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-fireworks` registers the Fireworks AI driver into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider) so the same Everruns
agent loop can run against [Fireworks AI](https://fireworks.ai/)'s open-model
catalog (Llama, Qwen, DeepSeek, Kimi, GLM, gpt-oss, ...). Fireworks exposes an
OpenAI-compatible Chat Completions API, so the driver wraps the core Chat
Completions protocol driver and parses Fireworks' richer `/models` metadata
(chat, tools, image input, context window) into capability profiles.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) for OpenAI models,
or [`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for Claude
models.

## Driver-Only Example

```rust
use everruns_fireworks::FireworksChatDriver;

let driver = FireworksChatDriver::new("your-api-key");
assert_eq!(
    driver.api_url(),
    "https://api.fireworks.ai/inference/v1/chat/completions",
);
```

## What It Provides

- A Fireworks AI Chat Completions driver wrapping the Everruns core protocol driver
- Registration into the Everruns `DriverRegistry` via `register_driver`
- `base_url` override for Fireworks-compatible / proxy endpoints
- Capability profiling derived from Fireworks' `/models` metadata, gated to the
  Fireworks host

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-fireworks)
- [Fireworks AI provider guide](https://docs.everruns.com/providers/fireworks/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
