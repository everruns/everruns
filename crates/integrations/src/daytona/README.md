# everruns-integrations

This integration is shipped as the `daytona` module of `everruns-integrations` Enable it with `features = ["daytona"]` in Cargo.

> Daytona cloud sandboxes for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` adds cloud-based sandboxed code execution backed
by the [Daytona](https://www.daytona.io) REST API. Agents can create sandboxes,
run commands with streamed output, and read or write files inside an isolated
environment, managed per session and authenticated with a user-supplied Daytona
API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-contracts`](https://crates.io/crates/everruns-contracts) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::daytona::DaytonaCapability;

let capability = DaytonaCapability;

assert_eq!(capability.id(), "daytona");
```

## What It Provides

- Per-session Daytona sandbox lifecycle (create, reuse, dispose)
- In-sandbox command execution with streamed output
- File read/write tools inside the sandbox
- Bring-your-own Daytona API key via the user connection provider
- Inventory-based Everruns integration registration

## Configuration

The Daytona API key is resolved from the user's `daytona` connection; there is
no platform-owned or environment-variable fallback. Configure the connection
before invoking sandbox tools.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [Daytona integration](https://docs.everruns.com/capabilities/daytona/)
- [Daytona sandboxes capability](https://docs.everruns.com/capabilities/daytona/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
