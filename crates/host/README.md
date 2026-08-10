# everruns-host

> Shared effectful orchestration for Everruns execution hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-host.svg)](https://crates.io/crates/everruns-host)
[![Documentation](https://docs.rs/everruns-host/badge.svg)](https://docs.rs/everruns-host)
[![License](https://img.shields.io/crates/l/everruns-host.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

Shared effectful host orchestration used by the Everruns application facade,
local adapters, workers, and advanced custom hosts.

This crate is a published implementation boundary so those crates can share a
single execution path. It is not the ordinary application entrypoint; most
applications in the [Everruns](https://everruns.com) ecosystem should use
[`everruns`](https://crates.io/crates/everruns) and the
[Framework guide](https://docs.everruns.com/framework/).

The pure, sans-I/O turn planner remains in
[`everruns-engine`](https://crates.io/crates/everruns-engine).

## Quick Example

```rust
use everruns_host::{RuntimeHostAdapter, RuntimeHostTurnContext};

fn accepts_host<A: RuntimeHostAdapter>() {}
fn accepts_context(_: RuntimeHostTurnContext) {}
# let _ = accepts_context;
```

## What It Provides

- Canonical event append, bounded replay, and read-only history projection
- Backend/store composition and low-level in-process execution
- Shared input, reason, act, lifecycle, MCP, filesystem, and scheduling host work
- Host adapter contracts for worker, local, and advanced integrations

`everruns-engine` remains the sans-I/O planner. Canonical events are the sole
history write path: host execution appends events, while `EventHistory` rebuilds
messages from bounded, sequence-ordered replay. No writable message store is
part of the maintained host API.

## Documentation

- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns-host)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
