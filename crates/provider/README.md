# everruns-provider

> Lean provider and LLM abstractions shared by the Everruns Framework and provider crates.

[![Crates.io](https://img.shields.io/crates/v/everruns-provider.svg)](https://crates.io/crates/everruns-provider)
[![Documentation](https://docs.rs/everruns-provider/badge.svg)](https://docs.rs/everruns-provider)
[![License](https://img.shields.io/crates/l/everruns-provider.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-provider` owns credential-free model identity, the `ChatDriver`
boundary, provider assembly, protocol drivers, stream/retry helpers, model
profiles, typed IDs, credential schemas, and the LLM error taxonomy.

It is a focused implementation crate in the [Everruns](https://everruns.com)
ecosystem. Framework application authors normally use the curated surface
through `everruns`; provider implementers and low-level hosts depend here
directly to avoid pulling in the agent-loop kernel.

The default `http` feature provides the shared protocol implementations and
installs the Rustls crypto provider before constructing their clients. Contract-
only consumers can use `default-features = false` to avoid HTTP and TLS
dependencies entirely. The `tls-aws-lc-rs` feature exposes the idempotent startup
initializer independently for binaries that assemble multiple TLS stacks.

## Quick Example

```rust
use everruns_provider::ModelSpec;

let model = ModelSpec::on("company-gateway", "assistant-v2");
assert_eq!(model.provider.as_str(), "company-gateway");
assert_eq!(model.model, "assistant-v2");
```

## What It Provides

- Open `ChatDriver` and provider registry contracts
- Credential-free `ModelSpec` and redacting endpoint/auth values
- Shared OpenAI and Open Responses protocol implementations
- Streaming, retry, model discovery/profile, and error helpers
- Provider-oriented tool and credential schema values

## Documentation

- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Custom providers](https://docs.everruns.com/framework/custom-providers/)
- [API reference](https://docs.rs/everruns-provider)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
