# everruns-gemini

> Google Gemini LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-gemini.svg)](https://crates.io/crates/everruns-gemini)
[![Documentation](https://docs.rs/everruns-gemini/badge.svg)](https://docs.rs/everruns-gemini)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-gemini` registers a Google Gemini driver with
[`everruns-core`](https://crates.io/crates/everruns-core) so
the same Everruns agent loop can run against Gemini models. It implements the
provider-neutral `ChatDriver` contract and maps Everruns' messages, tools, and
reasoning onto the Gemini API. Core has no knowledge of specific providers; hosts
register whichever drivers they want available.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for other
backends, or run with no key using the built-in LLM simulator in
[`everruns-runtime`](https://crates.io/crates/everruns-runtime).

## Quick Example

```rust
use everruns_gemini::{GeminiChatDriver, register_driver};
use everruns_core::DriverRegistry;

let mut registry = DriverRegistry::new();
register_driver(&mut registry);
```

Register the driver into a platform and drive a full turn with `everruns-runtime`.

## What It Provides

- A Google Gemini LLM driver
- Registration into the Everruns `DriverRegistry` via `register_driver`
- Streaming, tool calls, and reasoning mapped to provider-neutral Everruns types

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-gemini)
- [Google Gemini provider guide](https://docs.everruns.com/providers/gemini/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
