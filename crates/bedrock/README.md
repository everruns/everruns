# everruns-bedrock

> AWS Bedrock LLM provider for Everruns agents.

`everruns-bedrock` registers an AWS Bedrock driver with
[`everruns-core`](https://github.com/everruns/everruns/tree/main/crates/core) so
the same Everruns agent loop can run against models hosted on Amazon Bedrock. It
implements the provider-neutral `LlmDriver` contract using the Bedrock Runtime
`ConverseStream` API, mapping Everruns' messages, tools, and reasoning onto the
Bedrock wire format.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
`everruns-openai` and `everruns-anthropic` for other backends, or run with no key
using the built-in LLM simulator in `everruns-runtime`.

## Quick Example

```rust
use everruns_bedrock::{BedrockLlmDriver, register_driver};
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

- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
