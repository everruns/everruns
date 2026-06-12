# everruns-gemini

> Google Gemini LLM provider for Everruns agents.

`everruns-gemini` registers a Google Gemini driver with
[`everruns-core`](https://github.com/everruns/everruns/tree/main/crates/core) so
the same Everruns agent loop can run against Gemini models. It implements the
provider-neutral `LlmDriver` contract and maps Everruns' messages, tools, and
reasoning onto the Gemini API. Core has no knowledge of specific providers; hosts
register whichever drivers they want available.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
`everruns-openai` and `everruns-anthropic` for other backends, or run with no key
using the built-in LLM simulator in `everruns-runtime`.

## Quick Example

```rust
use everruns_gemini::{GeminiLlmDriver, register_driver};
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

- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
