# everruns-bedrock

> AWS Bedrock LLM provider for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-bedrock.svg)](https://crates.io/crates/everruns-bedrock)
[![Documentation](https://docs.rs/everruns-bedrock/badge.svg)](https://docs.rs/everruns-bedrock)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-bedrock` registers an AWS Bedrock driver into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider) so
the same Everruns agent loop can run against models hosted on Amazon Bedrock. It
implements the provider-neutral `ChatDriver` contract using the Bedrock Runtime
`ConverseStream` API, mapping Everruns' messages, tools, and reasoning onto the
Bedrock wire format.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for other
backends, or run with no key using the offline simulator in the
application-facing [`everruns`](https://crates.io/crates/everruns) crate.

## Quick Example

```rust
use everruns_bedrock::{BedrockChatDriver, register_driver};
use everruns_provider::DriverRegistry;

let mut registry = DriverRegistry::new();
register_driver(&mut registry);
```

Framework applications attach the ready-made provider through the open
`ModelSpec`/`Provider` boundary. Low-level hosts can register the driver directly.

## What It Provides

- A Bedrock Runtime `ConverseStream` driver
- Registration into the Everruns `DriverRegistry` via `register_driver`
- AWS credential and region resolution via the standard AWS SDK chain
- Streaming, tool calls, and reasoning mapped to provider-neutral Everruns types

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-bedrock)
- [AWS Bedrock provider guide](https://docs.everruns.com/providers/bedrock/)
- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
