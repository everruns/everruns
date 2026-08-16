# everruns-integrations-cursor

> Cursor Cloud Agents integration for Everruns.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-cursor.svg)](https://crates.io/crates/everruns-integrations-cursor)
[![Documentation](https://docs.rs/everruns-integrations-cursor/badge.svg)](https://docs.rs/everruns-integrations-cursor)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-cursor` lets Everruns agents launch and manage
[Cursor](https://cursor.com) Background / Cloud Agents through Cursor's public
REST API. Agents can delegate coding work to Cursor's cloud agents and track
their progress, authenticated with a user-supplied Cursor API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_cursor::CursorCapability;

let capability = CursorCapability;

assert_eq!(capability.id(), "cursor");
```

## What It Provides

- Launch and manage Cursor Background / Cloud Agents over the public REST API
- Bring-your-own Cursor API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-cursor)
- [Cursor integration](https://docs.everruns.com/integrations/cursor/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
