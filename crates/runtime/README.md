# everruns-runtime

> Source-compatible 0.17.x adapter for the canonical Everruns host implementation.

[![Crates.io](https://img.shields.io/crates/v/everruns-runtime.svg)](https://crates.io/crates/everruns-runtime)
[![Documentation](https://docs.rs/everruns-runtime/badge.svg)](https://docs.rs/everruns-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-runtime` owns no execution algorithm or backend composition. It
re-exports the single implementation from
[`everruns-host`](https://crates.io/crates/everruns-host), preserving established
public paths and feature behavior for supported 0.17.x consumers.

New ordinary applications should use the application-facing
[`everruns`](https://crates.io/crates/everruns) crate and the
[Everruns Framework](https://docs.everruns.com/framework/). Advanced host
integrators may depend directly on `everruns-host` plus focused provider, MCP,
or integration crates. Existing runtime users can migrate incrementally because
the compatibility paths execute the same canonical host code.
Both are part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let backends = RuntimeBackends::in_memory();
let builder = InProcessRuntimeBuilder::new().backends(backends);
# let _ = builder;
```

The builder remains usable through its 0.17 path, but its implementation is the
same `everruns-host::InProcessRuntimeBuilder`. To run an ordinary offline agent,
use the smaller Framework [quickstart](https://docs.everruns.com/framework/quickstart/).

Runnable examples ship with the crate:

```text
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
cargo run -p everruns-runtime --example real_disk_file_system_tools
```

## What It Provides

- Established runtime types and builders are aliases or re-exports of `everruns-host`.
- `lua` and `mcp-stdio` forward to the same host feature implementations.
- Legacy writable message-store and event-bus traits are deprecated, isolated
  shims; canonical execution writes events only and never calls them.
- The crate is scheduled for removal after the 0.17 compatibility window; no
  independent implementation is added here.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-runtime)
- [Runtime compatibility](https://docs.everruns.com/framework/runtime-compatibility/)
- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Everruns Framework](https://docs.everruns.com/framework/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
