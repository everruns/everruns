# everruns-capability

> Neutral capability contract for Everruns, capability identity, configuration, and code-defined capability authoring.

[![Crates.io](https://img.shields.io/crates/v/everruns-capability.svg)](https://crates.io/crates/everruns-capability)
[![Documentation](https://docs.rs/everruns-capability/badge.svg)](https://docs.rs/everruns-capability)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-capability` is the one open capability contract shared by the
[`everruns`](https://crates.io/crates/everruns) Framework, the hosted Everruns
product, and integration crates: validated `CapabilityId`s, the
`CapabilityRef` reference/config representation that round-trips through
application code, persisted attachments, and worker resolution, the non-sealed
`CapabilitySpec`/`IntoCapability` conversion boundary, code-defined capability
authoring (`Definition`, typed tool handlers, schema metadata, structured
errors), and registry identity bookkeeping with duplicate/collision rejection.

It deliberately depends on neither engine nor host crates, no Tokio, HTTP,
SQLx, OpenAPI, or platform records, so third-party capability packages can
depend on this crate alone. Applications normally consume the same types
re-exported from `everruns`.

## Quick Example

```rust
use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};
use serde_json::json;

struct VendorSearch {
    index: String,
}

impl IntoCapability for VendorSearch {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new("vendor.search")
            .config(json!({ "index": self.index }))
            .into()
    }
}

let spec = VendorSearch { index: "prod".into() }.into_capability();
assert_eq!(spec.capability_ref().id(), "vendor.search");
```

## What It Provides

- `CapabilityId`, open, string-based capability identity with one shared
  validation rule set (character grammar, length, reserved namespaces)
- `CapabilityRef`, capability id plus per-agent JSON object configuration;
  serializes as `{"ref", "config"}` everywhere and redacts config from `Debug`
- `CapabilitySpec` and non-sealed `IntoCapability`, the open conversion
  contract for application capability values
- `definition` (default feature), code-defined capability authoring: typed
  `Handler` tools, generated input/output schemas, execution `Context` with
  progress and cancellation boundaries, structured user/internal errors
- `CapabilityIdIndex` / `ActivationSet`, canonical-id and alias bookkeeping
  with duplicate/collision rejection shared by registries

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-capability)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [Everruns documentation](https://docs.everruns.com)

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents.

## License

MIT, see [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
