# everruns-ag-ui

> Deprecated compatibility shim for `everruns-core::ag_ui`.

[![Crates.io](https://img.shields.io/crates/v/everruns-ag-ui.svg)](https://crates.io/crates/everruns-ag-ui)
[![Documentation](https://docs.rs/everruns-ag-ui/badge.svg)](https://docs.rs/everruns-ag-ui)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This final shim release forwards the existing API to `everruns-core`. The next
platform release removes this package. No capability is removed; enable the
`ag-ui` feature and import `everruns_core::ag_ui` instead.

Part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_core::ag_ui::{Event, RunStartedEvent};
let event = Event::RunStarted(RunStartedEvent::new("thread", "run"));
let _ = event;
```

## Documentation

See the [public documentation](https://everruns.com/docs), the
[core API reference](https://docs.rs/everruns-core), and the
[compatibility API reference](https://docs.rs/everruns-ag-ui).

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE).
