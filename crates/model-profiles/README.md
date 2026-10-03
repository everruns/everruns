# everruns-model-profiles

> Moved: its shared contracts now live in `everruns-contracts`.

Part of the [Everruns](https://everruns.com) ecosystem. **Deprecated:** this crate
moved to [`everruns-contracts`](https://crates.io/crates/everruns-contracts).
It publishes one compatibility release, then leaves the workspace and publish set.

## Migrate

Replace the dependency with `everruns-contracts` and imports from
`everruns_model_profiles::` with `everruns_contracts::model_profile_data::`.

```rust
use everruns_contracts::model_profile_data::get_model_profile;
let profile = get_model_profile("anthropic", "claude-sonnet-5").expect("known model");
assert_eq!(profile.family, "claude-sonnet-5");
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
