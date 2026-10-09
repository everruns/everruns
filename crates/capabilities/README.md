# everruns-capabilities

> Hosted capabilities and sandbox orchestration.

Part of [Everruns](https://everruns.com).

```rust
use everruns_capabilities::capabilities::hosted_capability_registry;
let _registry = hosted_capability_registry();
```

## Features

Hosted execution services and optional container sandbox, A2A, AG-UI and environment capabilities.

Enable `hosted-integration-catalog` for the Platform's shared capability and
connector registration through `everruns_capabilities::integrations_catalog`.
This replaces the deprecated `everruns-integrations-catalog` package. The
feature is off by default so applications can select their own integrations.

## Documentation

[Everruns documentation](https://docs.everruns.com) · [Rust API](https://docs.rs/everruns-capabilities)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
