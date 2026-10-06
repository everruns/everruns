# everruns-integrations-web-fetch

> Deprecated compatibility package for the Web Fetch integration.

[Everruns](https://everruns.com) is an agentic runtime and control plane.

This crate is a one-release compatibility shim. Migrate to [`everruns-integrations`](https://crates.io/crates/everruns-integrations) with the `web-fetch` feature. This package will be removed in the following platform release.

## Quick Start

```rust
use everruns-integrations::web_fetch as _;
```

## Features

The `web-fetch` feature forwards to the maintained module in `everruns-integrations`. New applications should depend directly on that crate; compatibility imports continue to resolve during this release.

## Documentation

- [Integration guide](https://docs.everruns.com/integrations)
- [API reference](https://docs.rs/everruns-integrations)

## License

MIT. See the [repository license](https://github.com/everruns/everruns/blob/main/LICENSE).
