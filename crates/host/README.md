# everruns-host

> Deprecated compatibility shim for `everruns-core::host`.

[![Crates.io](https://img.shields.io/crates/v/everruns-host.svg)](https://crates.io/crates/everruns-host)
[![Documentation](https://docs.rs/everruns-host/badge.svg)](https://docs.rs/everruns-host)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This final shim release forwards the existing API to `everruns-core`. The next
platform release removes this package. No capability is removed; enable the
`host` feature and import `everruns_core::host` instead.

Part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_core::host::HostComposition;
let composition = HostComposition::default();
assert!(composition.driver_registry().registered_providers().is_empty());
```

## Documentation

See the [public documentation](https://everruns.com/docs), the
[core API reference](https://docs.rs/everruns-core), and the
[compatibility API reference](https://docs.rs/everruns-host).

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE).

Default driver and integration wiring now lives in `everruns::batteries`. Core
hosts accept explicit registries, transports, and shell-hook dispatchers.
