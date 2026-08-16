# everruns-integrations-e2b

> E2B cloud sandboxes for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-e2b.svg)](https://crates.io/crates/everruns-integrations-e2b)
[![Documentation](https://docs.rs/everruns-integrations-e2b/badge.svg)](https://docs.rs/everruns-integrations-e2b)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-e2b` gives agents cloud sandboxes backed by
[E2B](https://e2b.dev), so they can create isolated environments, run processes,
and read or write files without touching the host. Sandboxes are managed per
session with leased-resource cleanup, and authenticate with a user-supplied E2B
API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-core`](https://crates.io/crates/everruns-core) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_e2b::E2BCapability;

let capability = E2BCapability;

assert_eq!(capability.id(), "e2b");
```

## What It Provides

- Per-session E2B sandbox lifecycle with leased-resource cleanup
- Process execution over the E2B Connect RPC API
- File read/write tools inside the sandbox
- Bring-your-own E2B API key via the user connection provider
- Inventory-based Everruns integration registration

## Configuration

The E2B API key is resolved from the user's `e2b` connection only; there is no
platform-owned or environment-variable fallback. Tools fail with
`ConnectionRequired` until the connection is configured.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-e2b)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
