# everruns-local

> SQLite-backed, restart-survivable runtime stores for embedded, single-process Everruns hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-local.svg)](https://crates.io/crates/everruns-local)
[![Documentation](https://docs.rs/everruns-local/badge.svg)](https://docs.rs/everruns-local)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-local` supplies local filesystem configuration and SQLite-backed
session-catalog, task, and schedule state. Framework applications access the high-level profile
through the `everruns` crate's `local` feature; advanced hosts can compose the
focused backends directly with `everruns-host`.

The runtime stays generic and owns only the seams — durable local storage
choices live here, behind an opt-in crate, so embedders (terminal coding agents,
personal agents, …) don't each reinvent them.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## What It Provides

- **`LocalSessionTaskRegistry`** — a `SessionTaskRegistry` over SQLite,
  persisting session tasks and their message channel.
- **`LocalSessionStore`** — a durable session identity and metadata catalog;
  conversation messages remain an event-derived projection.
- **`LocalScheduleStore`** — a `SessionScheduleStore` over SQLite, with an
  additive JSON `metadata` bag (name/color/kind/…) kept local rather than
  widening the shared core primitive.
- **`LocalScheduleRunner`** — an explicitly started/stopped in-process runner
  for due one-shot and recurring schedules. It scopes atomic SQLite claims to
  the sessions reported by `LocalSessionRunner::routable_session_ids`, recovers
  interrupted claims, and delivers prompts through
  `LocalSessionRunner::send_message`.
- **`LocalPlatformStore`** — a `PlatformStore` that implements the
  subagent-critical core honestly and returns explicit unsupported errors for
  platform-management-only operations.
- **`LocalProfile`** — named local environment config (data dir, workspace, base
  URL, org/principal identity defaults).
- **`LocalBackends`** — composable construction of `HostBackends` plus the
  local stores, preserving a caller-supplied event bus and session file-system
  factory.
- **`LocalRuntimeBuilder`** — optional sugar over `InProcessRuntimeBuilder`;
  its default registry composes neutral core capabilities, the full portable
  `everruns-builtins` policy catalog, and host integrations.

Session identity, task, and schedule state persist to SQLite. Advanced hosts
still select their event log independently; the Framework's `LocalConfig`
combines this catalog with its crash-durable canonical event log.

## Install

Requires Rust 1.94+ (edition 2024).

```bash
cargo add everruns --features local
```

## Quick Example

```rust
use everruns_local::{LocalBackends, LocalProfile};
use everruns_host::{HostBackends, InProcessRuntimeBuilder};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
// LocalProfile stores plain filesystem paths and does not expand a leading
// `~`, so pass an absolute path your application owns. Its SQLite file is
// derived from the data dir.
let profile = LocalProfile::new("/var/lib/everruns")
    .with_workspace_root("/var/lib/everruns/workspace");

// Layer the local, SQLite-backed stores over a set of host backends. The event
// bus (and any other store) you pass on `HostBackends` is preserved.
let local = LocalBackends::new(profile, HostBackends::in_memory())?;

// Plug the assembled backends into the in-process runtime builder.
let _builder = InProcessRuntimeBuilder::new().backends(local.runtime_backends.clone());
# Ok(())
# }
```

Hosts that enable `create_schedule` or scheduled `spawn_background` calls must
also run the executor for their lifetime. Use the same `LocalSessionRunner`
implementation that backs `LocalPlatformStore`. Hosts that cannot route every
session in the organization must implement `routable_session_ids`; return
`Some(vec![])` when no route is active and update the returned snapshot as
sessions activate or stop. Then retain the handle:

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
scheduled turns tolerant of that standard crash window. A failed delivery stays
durable and waits the configured `claim_timeout` before retrying.

See the integration tests under [`tests/`](./tests) for end-to-end coverage of
task lifecycle, restart survivability, schedule round-trips, composability, and
embedded turns.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-local)
- [Framework persistence](https://docs.everruns.com/framework/persistence/)
- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
