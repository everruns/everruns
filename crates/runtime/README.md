# everruns-runtime

> Deprecated compatibility API for existing 0.17.x applications; new applications use the Everruns Framework.

[![Crates.io](https://img.shields.io/crates/v/everruns-runtime.svg)](https://crates.io/crates/everruns-runtime)
[![Documentation](https://docs.rs/everruns-runtime/badge.svg)](https://docs.rs/everruns-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

> **Transition notice:** `everruns-runtime` is compatibility-only in 0.17.x and
> will be removed in Everruns 0.18. It remains published and usable for the
> entire 0.17.x line.

New ordinary applications should depend on
[`everruns`](https://crates.io/crates/everruns), start with
`use everruns::{Agent, Model};`, and follow the
[Everruns Framework](https://docs.everruns.com/framework/). Advanced system
integrators should use `everruns` plus
[`everruns-host`](https://crates.io/crates/everruns-host) and focused provider,
MCP, or integration crates, starting with
`use everruns_host::{HostBackends, InProcessRuntimeBuilder};`.

For existing 0.17.x applications, `everruns-runtime` owns no execution
algorithm or backend composition. It re-exports the single implementation from
[`everruns-host`](https://crates.io/crates/everruns-host), preserving established
public paths and feature behavior for supported 0.17.x consumers.

Existing users can migrate incrementally because the compatibility paths
execute the same canonical host code. See the authoritative
[runtime migration guide](https://docs.everruns.com/framework/runtime-compatibility/).
All crates are part of the [Everruns](https://everruns.com) ecosystem.

## 0.17 Compatibility Example

```rust
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let backends = RuntimeBackends::in_memory();
let builder = InProcessRuntimeBuilder::new().backends(backends);
# let _ = builder;
```

The builder remains usable through its 0.17 path, but its implementation is the
same `everruns-host::InProcessRuntimeBuilder`. This example is compatibility
coverage, not a starting point for new applications. The Framework
[quickstart](https://docs.everruns.com/framework/quickstart/) is offline and
requires no database, server, worker, network, or credentials.

Runnable examples ship with the crate:

```text
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
cargo run -p everruns-runtime --example real_disk_file_system_tools
```

## What It Provides

- Established runtime types and builders are aliases or re-exports of `everruns-host`.
- `lua` and `mcp-stdio` forward to the same host feature implementations.
- Legacy mutable-history and event-bus traits are deprecated, isolated shims;
  canonical execution writes events only and never calls them.
- The crate will be removed in Everruns 0.18; no independent implementation is
  added here during the 0.17 compatibility window.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-runtime)
- [Runtime migration guide](https://docs.everruns.com/framework/runtime-compatibility/)
- [Everruns Framework](https://docs.everruns.com/framework/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
