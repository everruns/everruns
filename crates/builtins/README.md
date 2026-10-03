# everruns-builtins

> Deprecated compatibility shim for `everruns-core::builtins`.

[![Crates.io](https://img.shields.io/crates/v/everruns-builtins.svg)](https://crates.io/crates/everruns-builtins)
[![Documentation](https://docs.rs/everruns-builtins/badge.svg)](https://docs.rs/everruns-builtins)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This final shim release forwards the existing API to `everruns-core`. The next
platform release removes this package. No capability is removed; enable the
`builtins` feature and import `everruns_core::builtins` instead.

Part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_core::builtins::portable_capability_registry;
let registry = portable_capability_registry().expect("curated catalog");
assert!(registry.has("current_time"));
```

## Documentation

See the [public documentation](https://everruns.com/docs), the
[core API reference](https://docs.rs/everruns-core), and the
[compatibility API reference](https://docs.rs/everruns-builtins).

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE).
