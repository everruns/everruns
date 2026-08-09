# everruns-runtime

> Low-level in-process execution hosting and 0.17.x compatibility for Everruns embedders.

[![Crates.io](https://img.shields.io/crates/v/everruns-runtime.svg)](https://crates.io/crates/everruns-runtime)
[![Documentation](https://docs.rs/everruns-runtime/badge.svg)](https://docs.rs/everruns-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-runtime` exposes the low-level in-process host, reference stores, and
reusable host-phase execution used by embedded and durable systems. It runs the
same `input → reason → act` loop without requiring the durable engine, gRPC
worker boundary, or control-plane server.

This crate remains supported for existing 0.17.x applications and advanced
hosts that own backend or orchestration topology. New ordinary applications
should start with the application-facing [`everruns`](https://crates.io/crates/everruns)
crate and the [Everruns Framework](https://docs.everruns.com/framework/).

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. The runtime builds on the
contracts in [`everruns-core`](https://crates.io/crates/everruns-core) and pairs
with provider crates such as
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic).

## Quick Example

```rust
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let backends = RuntimeBackends::in_memory();
let builder = InProcessRuntimeBuilder::new().backends(backends);
# let _ = builder;
```

The builder is the compatibility path for hosts that intentionally own runtime
backends. To run an ordinary offline agent, use the smaller Framework
[quickstart](https://docs.everruns.com/framework/quickstart/).

`InProcessRuntimeBuilder::new()` starts with a runtime-safe built-in capability
registry. If you call `.platform_definition(...)`, that platform becomes
authoritative; start from `CapabilityRegistry::runtime_builtins()` when you want
the default runtime catalog plus your own additions.

Runnable examples ship with the crate:

```text
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
cargo run -p everruns-runtime --example real_disk_file_system_tools
```

## What It Provides

- `InProcessRuntimeBuilder` for declaring platforms, harnesses, agents, and sessions in code
- In-memory stores for local development and tests, with hooks to plug in custom backends
- Real-disk workspace wiring so file-backed agents can read and write an actual directory
- Turn-context inspection before and after a turn executes
- Host-phase helpers reused by durable and server-backed execution

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-runtime)
- [Runtime compatibility](https://docs.everruns.com/framework/runtime-compatibility/)
- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Everruns Framework](https://docs.everruns.com/framework/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
