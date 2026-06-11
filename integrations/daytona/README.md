# everruns-integrations-daytona

Daytona cloud sandbox integration for Everruns agents.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It adds
cloud-based sandboxed code execution backed by the
[Daytona](https://www.daytona.io) REST API, letting agents create sandboxes,
run commands, and read or write files inside an isolated environment.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_daytona::DaytonaCapability;

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

## License

MIT. See the repository-level `LICENSE` file.
