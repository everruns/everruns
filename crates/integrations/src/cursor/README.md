# everruns-integrations

This integration is shipped as the `cursor` module of `everruns-integrations` Enable it with `features = ["cursor"]` in Cargo.

> Cursor Cloud Agents integration for Everruns.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` lets Everruns agents launch and manage
[Cursor](https://cursor.com) Background / Cloud Agents through Cursor's public
REST API. Agents can delegate coding work to Cursor's cloud agents and track
their progress, authenticated with a user-supplied Cursor API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-contracts`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::cursor::CursorCapability;

let capability = CursorCapability;

assert_eq!(capability.id(), "cursor");
```

## What It Provides

- Launch and manage Cursor Background / Cloud Agents over the public REST API
- Bring-your-own Cursor API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [Cursor integration](https://docs.everruns.com/integrations/cursor/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
