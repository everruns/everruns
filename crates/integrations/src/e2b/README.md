# everruns-integrations

This integration is shipped as the `e2b` module of `everruns-integrations` Enable it with `features = ["e2b"]` in Cargo.

> E2B cloud sandboxes for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` gives agents cloud sandboxes backed by
[E2B](https://e2b.dev), so they can create isolated environments, run processes,
and read or write files without touching the host. Sandboxes are managed per
session with leased-resource cleanup, and authenticate with a user-supplied E2B
API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-contracts`](https://crates.io/crates/everruns-contracts) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::e2b::E2BCapability;

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

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
