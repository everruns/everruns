# everruns-integrations-catalog

> The catalog of integrations composed into the hosted Everruns product.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-catalog.svg)](https://crates.io/crates/everruns-integrations-catalog)
[![Documentation](https://docs.rs/everruns-integrations-catalog/badge.svg)](https://docs.rs/everruns-integrations-catalog)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-catalog` is the one place that names every integration
crate composed into the hosted product. Each integration crate publishes its
contributions as plain consts — `CAPABILITY_PLUGINS` and `CONNECTOR_PLUGINS` —
and this crate lists one `CatalogEntry` per crate, so the compiler checks the
composition instead of the linker deciding it.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It composes capabilities from
[`everruns-core`](https://crates.io/crates/everruns-core) and connectors from
[`everruns-platform`](https://crates.io/crates/everruns-platform).

## Quick Example

```rust
use everruns_integrations_catalog::CATALOG;

// Every integration is named exactly once, and each contributes something.
assert!(CATALOG.iter().all(|entry| {
    !entry.capabilities.is_empty() || !entry.connectors.is_empty()
}));
```

Build the default registry, or filter the catalog and register your own set:

```rust
use everruns_core::DeploymentGrade;
use everruns_integrations_catalog::{CATALOG, capability_is_enabled};

let registry =
    everruns_integrations_catalog::oss_capability_registry_for_grade(DeploymentGrade::Prod);
assert!(registry.get("docker_container").is_none());

let without_docker = CATALOG
    .iter()
    .filter(|entry| entry.crate_name != "everruns-integrations-docker");
for entry in without_docker {
    for plugin in entry.capabilities {
        let _enabled = capability_is_enabled(plugin, DeploymentGrade::Prod);
    }
}
```

## What It Provides

- `CATALOG`, the named list of every integration in the hosted product
- `oss_capability_registry_for_grade`, composing builtins, integrations, then
  hosted capabilities so a hosted capability wins an id collision
- `register_capabilities` / `register_connectors` for a registry you own
- `capability_is_enabled`, the deployment-grade and feature-flag gate
- A starting point embedders filter or extend rather than inherit wholesale

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-catalog)
- [Integrations](https://docs.everruns.com/integrations/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
