# everruns-bedrock

> AWS Bedrock LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-bedrock.svg)](https://crates.io/crates/everruns-bedrock)
[![Documentation](https://docs.rs/everruns-bedrock/badge.svg)](https://docs.rs/everruns-bedrock)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-bedrock` registers an AWS Bedrock driver with
[`everruns-core`](https://crates.io/crates/everruns-core) so
the same Everruns agent loop can run against models hosted on Amazon Bedrock. It
implements the provider-neutral `ChatDriver` contract using the Bedrock Runtime
`ConverseStream` API, mapping Everruns' messages, tools, and reasoning onto the
Bedrock wire format.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for other
backends, or run with no key using the built-in LLM simulator in
[`everruns-runtime`](https://crates.io/crates/everruns-runtime).

## Quick Example

```rust
use everruns_bedrock::{BedrockChatDriver, register_driver};
use everruns_core::DriverRegistry;

let mut registry = DriverRegistry::new();
register_driver(&mut registry);
```

Register the driver into a platform and drive a full turn with `everruns-runtime`.

## What It Provides

- A Bedrock Runtime `ConverseStream` driver
- Registration into the Everruns `DriverRegistry` via `register_driver`
- AWS credential and region resolution via the standard AWS SDK chain
- Streaming, tool calls, and reasoning mapped to provider-neutral Everruns types

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-bedrock)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
