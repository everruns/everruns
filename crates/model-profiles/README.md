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
- Full-registry and selected-profile enumeration, with provider filtering

## Offline Enumeration

`all_profiles()` and `profiles_for_provider(provider)` return owned profiles in
registry order. `selected_profiles()` and `selected_profiles_for_provider(provider)`
expose the curated selection. The registry is currently that selection, so the
selected and full lists are identical; neither is the entire models.dev catalog.
Order is deterministic but does not rank recommendations or latest flagships.

For menus, use the companion entry APIs: profile families are not always model
ids, and chat menus must exclude other services.

```rust
use everruns_model_profiles::{profile_entries_for_provider, ServiceKind};

let chat_models: Vec<_> = profile_entries_for_provider("openai")
    .into_iter()
    .filter(|entry| entry.service == ServiceKind::Chat)
    .collect();
assert!(chat_models.iter().any(|entry| entry.model_id == "gpt-6-astra"));
```

`all_profile_entries()` returns canonical identities with their base profiles;
`profile_entries_for_provider(provider)` applies the same capability masks as
point lookup. Entries include canonical lookup ids, accepted aliases, stable
keys, vendors, and service kinds. A canonical lookup id is not necessarily a
provider request id: match live discovery against the canonical id or aliases
before sending requests. Unknown providers return empty lists.

All enumerators and `ModelProfileEntry` are also exported by `everruns-provider`
and its `model_profiles` module; provider-filtered wrappers take `&DriverId`.
No network or credentials are required. After a dependency bump, consumers can
guard required menu entries with a subset assertion and use point lookup in
local-override tests to fail when an override becomes available upstream.

## Documentation

- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [API reference](https://docs.rs/everruns-model-profiles)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
