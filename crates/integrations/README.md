# everruns-integrations

> Vendor integrations for Everruns agents, one Cargo feature per vendor.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations.svg)](https://crates.io/crates/everruns-integrations)
[![Documentation](https://docs.rs/everruns-integrations/badge.svg)](https://docs.rs/everruns-integrations)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` folds vendor integrations into one crate the way
`everruns-drivers` folds LLM drivers. A default build compiles nothing; enable
the vendors you need. The crate depends on
[`everruns-contracts`](https://crates.io/crates/everruns-contracts) only.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents.

## What It Provides

| Feature | Module | Provides |
|---|---|---|
| `modal` | `modal` | [Modal](https://modal.com) sandboxes: full Linux VMs or gVisor containers, files, snapshots, tunnels. Experimental |

Each module exports `CAPABILITY_PLUGINS` and `CONNECTOR_PLUGINS` for a hosted
catalog, plus its capability and connector types for direct use.

## Quick Example

```toml
[dependencies]
everruns-integrations = { version = "0.41", features = ["modal"] }
```

```rust
# #[cfg(feature = "modal")] {
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::modal::ModalCapability;

// Register it on a capability registry, then name "modal" on a harness.
// Tools resolve the user's Modal connection (`ak-...:as-...`) at call time.
let capability = ModalCapability;
assert_eq!(capability.id(), "modal");
assert_eq!(capability.tools().len(), 8);
# }
```

## Documentation

- API reference: [docs.rs/everruns-integrations](https://docs.rs/everruns-integrations)
- Modal guide: [docs.everruns.com/integrations/modal](https://docs.everruns.com/integrations/modal/)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
