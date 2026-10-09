# everruns-integrations-catalog

> Deprecated compatibility shim for the hosted Everruns integration catalog.

[Everruns](https://everruns.com) is an agentic runtime and control plane.

Use `everruns-capabilities::integrations_catalog` with the
`hosted-integration-catalog` feature instead. This crate forwards the old API for
one release and will be removed in the following platform release.

```rust
use everruns_capabilities::integrations_catalog::CATALOG;

assert!(!CATALOG.is_empty());
```

## Features

This crate preserves the previous catalog API while you move imports to
`everruns-capabilities`. No integration or capability is removed.

## Documentation

- [Integration guides](https://docs.everruns.com/integrations/)
- [Deprecated API reference](https://docs.rs/everruns-integrations-catalog)
- [Current API reference](https://docs.rs/everruns-capabilities/latest/everruns_capabilities/integrations_catalog/)

## License

MIT. See the [repository license](https://github.com/everruns/everruns/blob/main/LICENSE).
