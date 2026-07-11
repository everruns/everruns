# everruns-local

> SQLite-backed, restart-survivable runtime stores for embedded, single-process Everruns hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-local.svg)](https://crates.io/crates/everruns-local)
[![Documentation](https://docs.rs/everruns-local/badge.svg)](https://docs.rs/everruns-local)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-local` populates the optional host-backend slots of
[`everruns-runtime`](https://crates.io/crates/everruns-runtime) with local,
file-backed implementations. The runtime ships in-memory by default; this crate
swaps in durable, SQLite-backed stores so a freshly-spawned process can read,
continue, and inspect work an earlier process started.

The runtime stays generic and owns only the seams — durable local storage
choices live here, behind an opt-in crate, so embedders (terminal coding agents,
personal agents, …) don't each reinvent them.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## What It Provides

- **`LocalSessionTaskRegistry`** — a `SessionTaskRegistry` over SQLite,
  persisting session tasks and their message channel.
- **`LocalScheduleStore`** — a `SessionScheduleStore` over SQLite, with an
  additive JSON `metadata` bag (name/color/kind/…) kept local rather than
  widening the shared core primitive.
- **`LocalScheduleRunner`** — an explicitly started/stopped in-process runner
  for due one-shot and recurring schedules. It uses atomic SQLite claims,
  recovers interrupted claims, and delivers prompts through
  `LocalSessionRunner::send_message`.
- **`LocalPlatformStore`** — a `PlatformStore` that implements the
  subagent-critical core honestly and returns explicit unsupported errors for
  platform-management-only operations.
- **`LocalProfile`** — named local environment config (data dir, workspace, base
  URL, org/principal identity defaults).
- **`LocalBackends`** — composable construction of `RuntimeBackends` plus the
  local stores, preserving a caller-supplied event bus and session file-system
  factory.
- **`LocalRuntimeBuilder`** — optional sugar over `InProcessRuntimeBuilder`.

Task and message state persists to a SQLite file, so process restarts survive:
the next process picks up exactly where the last one left off.

## Install

Requires Rust 1.94+ (edition 2024).

```bash
cargo add everruns-runtime everruns-local
```

## Quick Example

```rust
use everruns_local::{LocalBackends, LocalProfile};
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
// LocalProfile stores plain filesystem paths and does not expand a leading
// `~`, so pass an absolute path your application owns. Its SQLite file is
// derived from the data dir.
let profile = LocalProfile::new("/var/lib/everruns")
    .with_workspace_root("/var/lib/everruns/workspace");

// Layer the local, SQLite-backed stores over a set of runtime backends. The
// event bus (and any other store) you pass on `RuntimeBackends` is preserved.
let local = LocalBackends::new(profile, RuntimeBackends::in_memory())?;

// Plug the assembled backends into the in-process runtime builder.
let _builder = InProcessRuntimeBuilder::new().backends(local.runtime_backends.clone());
# Ok(())
# }
```

Hosts that enable `create_schedule` or scheduled `spawn_background` calls must
also run the executor for their lifetime. Use the same `LocalSessionRunner`
implementation that backs `LocalPlatformStore`, then retain the handle:

```rust,ignore
let schedule_runner = local.start_schedule_runner(session_runner.clone())?;
let local = local.with_platform_runner(session_runner);

// Keep `schedule_runner` alive with the host, then stop cleanly.
schedule_runner.shutdown().await?;
```

Delivery is at-least-once across a process crash: concurrent live runners do
not deliver the same occurrence, and claims are heartbeated while
`send_message` runs, but a crash after the host accepts a message and before
SQLite records completion can cause a retry. Embedded hosts should make
scheduled turns tolerant of that standard crash window.

See the integration tests under [`tests/`](./tests) for end-to-end coverage of
task lifecycle, restart survivability, schedule round-trips, composability, and
embedded turns.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-local)
- [Runtime — embed Everruns in your process](https://docs.everruns.com/features/runtime/#local-backends)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
