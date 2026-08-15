# everruns-session-services

> Backend-neutral session mutation and storage capabilities for Everruns execution hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-session-services.svg)](https://crates.io/crates/everruns-session-services)
[![Documentation](https://docs.rs/everruns-session-services/badge.svg)](https://docs.rs/everruns-session-services)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-session-services` owns the small session-service seam shared by the
Everruns Framework host and the hosted platform. It deliberately contains no
organization, payment, connector, audit, email, SQL, or HTTP implementation.

Applications normally use these capabilities through `everruns`; custom hosts
can depend on this crate without compiling `everruns-platform`.

## Quick Example

```rust
use everruns_session_services::SESSION_CAPABILITY_ID;

assert_eq!(SESSION_CAPABILITY_ID, "session");
```

## What It Provides

- `SessionMutator` and its runtime extension
- portable `session` capability tools
- portable `session_storage` capability tools

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-session-services)
- [Everruns Framework](https://docs.everruns.com/framework/)
- [Everruns documentation](https://docs.everruns.com)

Part of the [Everruns](https://everruns.com) ecosystem.

## License

MIT — see [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
