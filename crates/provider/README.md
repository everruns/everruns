# everruns-provider

> Moved: its shared contracts now live in `everruns-contracts`.

Part of the [Everruns](https://everruns.com) ecosystem. **Deprecated:** this crate
moved to [`everruns-contracts`](https://crates.io/crates/everruns-contracts).
It publishes one compatibility release, then leaves the workspace and publish set.

## Migrate

Replace the dependency with `everruns-contracts` and imports from
`everruns_provider::` with `everruns_contracts::`. Enable its `http` feature to
retain the protocol transport enabled by this shim's default feature.

```rust
use everruns_contracts::ModelSpec;
let model = ModelSpec::on("company-gateway", "assistant-v2");
assert_eq!(model.provider.as_str(), "company-gateway");
```

## What It Provides

- Existing APIs and feature names forward to the new implementation.
- Deprecated representative type aliases point to the canonical contract types.

## Documentation

- [`everruns-contracts` API reference (docs.rs)](https://docs.rs/everruns-contracts)
- [Everruns Framework](https://docs.everruns.com/framework/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
