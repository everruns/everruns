# everruns-platform

> Backend control-plane entities and store contracts for the Everruns Platform.

[![Crates.io](https://img.shields.io/crates/v/everruns-platform.svg)](https://crates.io/crates/everruns-platform)
[![Documentation](https://docs.rs/everruns-platform/badge.svg)](https://docs.rs/everruns-platform)
[![License](https://img.shields.io/crates/l/everruns-platform.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-platform` contains organization, principal, app/channel, trigger,
payment, reporting, audit, and hosted-capability implementations used by server
and platform backends. It also exposes the product registries and narrow store
contracts consumed by those hosts.

It is a backend crate in the [Everruns](https://everruns.com) ecosystem. Normal
Framework applications use `everruns`; control-plane and specialized host code
uses this crate when it owns platform records or storage.

## Quick Example

```rust
use everruns_platform::{Organization, Principal};
use everruns_platform::capabilities::hosted_capability_registry;

fn accepts_platform_values(_organization: &Organization, _principal: &Principal) {}
# let _ = accepts_platform_values;
# let registry = hosted_capability_registry();
# assert!(registry.has("knowledge_base"));
```

## What It Provides

- Organization, membership, and principal domain values
- App/channel and agent-trigger control-plane records
- Hosted knowledge, memory, delegation, task, hook, and management capabilities
- Hosted platform, knowledge-search, and vector-store contracts
- Product registry composition that layers hosted capabilities over core built-ins
- Payment, reporting, audit, and governance records

## Documentation

- [Everruns Platform architecture](https://docs.everruns.com/explanation/architecture/)
- [Framework crate selection](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns-platform)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
