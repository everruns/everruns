# everruns-meta

> Meta Model API provider support for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-meta.svg)](https://crates.io/crates/everruns-meta)
[![Documentation](https://docs.rs/everruns-meta/badge.svg)](https://docs.rs/everruns-meta)
[![License](https://img.shields.io/crates/l/everruns-meta.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-meta` implements Meta's OpenAI-compatible Responses API, including
first-party endpoint defaults and Muse model discovery.

It is a provider crate in the [Everruns](https://everruns.com) ecosystem and
builds on `everruns-provider`. Framework applications pair it with `everruns`.

## Quick Example

```rust
use everruns_meta::provider;

let meta = provider("meta", "model-api-key");
assert_eq!(meta.id().as_str(), "meta");
```

## What It Provides

- A ready-to-use Meta provider assembly
- A Meta-compatible Responses API `ChatDriver`
- Muse model discovery and Meta endpoint defaults
- Registration helpers for low-level provider registries

## Documentation

- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Meta provider guide](https://docs.everruns.com/providers/meta/)
- [API reference](https://docs.rs/everruns-meta)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
