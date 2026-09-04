# everruns-model-profiles

> Model profile data and types for the Everruns agentic framework.

[![Crates.io](https://img.shields.io/crates/v/everruns-model-profiles.svg)](https://crates.io/crates/everruns-model-profiles)
[![Documentation](https://docs.rs/everruns-model-profiles/badge.svg)](https://docs.rs/everruns-model-profiles)
[![License](https://img.shields.io/crates/l/everruns-model-profiles.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-model-profiles` owns model identity/capability metadata
(`ModelProfile` and its cost/limits/modality/reasoning/speed/verbosity
components), model vendor branding (`ModelVendor`), the model-service
taxonomy (`ServiceKind`), and the hardcoded profile registry sourced from
[models.dev](https://github.com/sst/models.dev), matched by provider wire id
and model id.

It is a focused, dependency-light leaf crate in the
[Everruns](https://everruns.com) ecosystem: it does not depend on
`everruns-provider`, so driver/provider identity is a plain wire-id string
(e.g. `"openai"`, `"anthropic"`) rather than `everruns_provider::DriverId`.
`everruns-provider` depends on this crate and re-exports its types, so
existing callers of `everruns_provider::{ModelProfile, ServiceKind,
model_profiles::*}` are unaffected.

## Quick Example

```rust
use everruns_model_profiles::get_model_profile;

let profile = get_model_profile("anthropic", "claude-sonnet-5").expect("known model");
assert_eq!(profile.family, "claude-sonnet-5");
```

## What It Provides

- `ModelProfile` and its cost/limits/modality/reasoning-effort/speed/verbosity components
- `ModelVendor` model branding and `ServiceKind` service taxonomy
- The built-in profile registry, matched by provider wire id and model id
- Profile-key lookup and cost estimation helpers

## Documentation

- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [API reference](https://docs.rs/everruns-model-profiles)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
