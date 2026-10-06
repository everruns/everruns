# everruns-integrations-deno

> Deprecated compatibility package for the Deno sandboxes.

[Everruns](https://everruns.com) is an agentic runtime and control plane.

This crate is a one-release compatibility shim. Migrate to [`everruns-integrations-experimental`](https://crates.io/crates/everruns-integrations-experimental) with the `deno` feature. This package will be removed in the following platform release.

## Quick Start

```rust
use everruns-integrations-experimental::deno as _;
```

## Features

The `deno` feature forwards to the maintained module in `everruns-integrations-experimental`. New applications should depend directly on that crate; compatibility imports continue to resolve during this release.

## Documentation

- [Integration guide](https://docs.everruns.com/integrations)
- [API reference](https://docs.rs/everruns-integrations-experimental)

## License

MIT. See the [repository license](https://github.com/everruns/everruns/blob/main/LICENSE).
