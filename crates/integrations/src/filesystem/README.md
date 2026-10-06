# everruns-integrations

This integration is shipped as the `filesystem` module of `everruns-integrations` Enable it with `features = ["filesystem"]` in Cargo.

> Session filesystem tools for Everruns agents.

`everruns-integrations` implements the `session_file_system`
capability over Everruns' neutral session-filesystem contract. It provides
read, write, edit, list, grep, delete, and stat tools while retaining path and
mount policy enforcement supplied by the host.

Part of the [Everruns](https://everruns.com) ecosystem. Applications normally
enable this integration through the `everruns` Framework crate; advanced hosts
may register it directly.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::filesystem::FileSystemCapability;

assert_eq!(FileSystemCapability.id(), "session_file_system");
```

## What It Provides

- Session-scoped filesystem tool implementations
- Traversal-safe path and mount adaptation
- Hash-gated text edits and bounded single-file or ordered multi-file reads/searches
- Model-visible path narration and binary image handling

## Documentation

- [Framework workspace security](https://docs.everruns.com/framework/workspaces-and-environments/)
- [API reference](https://docs.rs/everruns-integrations)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
