# everruns-engine

> Deprecated compatibility shim for `everruns-core::engine`.

[![Crates.io](https://img.shields.io/crates/v/everruns-engine.svg)](https://crates.io/crates/everruns-engine)
[![Documentation](https://docs.rs/everruns-engine/badge.svg)](https://docs.rs/everruns-engine)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This final shim release forwards the existing API to `everruns-core`. The next
platform release removes this package. No capability is removed; enable the
`engine` feature and import `everruns_core::engine` instead.

Part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_core::engine::{TurnPlan, TurnState};
fn accepts_plan(_: &TurnState, _: &TurnPlan) {}
let _ = accepts_plan;
```

## Features

Forwards the existing public API and feature names to `everruns-core::engine`
for this final deprecated release. New consumers use the canonical core module.

## Documentation

See the [public documentation](https://docs.everruns.com/framework/), the
[core API reference](https://docs.rs/everruns-core), and the
[compatibility API reference](https://docs.rs/everruns-engine).

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE).
