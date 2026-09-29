# everruns-serve-macros

> Attribute macros for the experimental `everruns-serve` agent app framework.

[![Crates.io](https://img.shields.io/crates/v/everruns-serve-macros.svg)](https://crates.io/crates/everruns-serve-macros)
[![Documentation](https://docs.rs/everruns-serve-macros/badge.svg)](https://docs.rs/everruns-serve-macros)
[![License](https://img.shields.io/crates/l/everruns-serve-macros.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-serve-macros` implements the attributes that
[`everruns-serve`](https://crates.io/crates/everruns-serve) re-exports. It is an
implementation crate in the [Everruns](https://everruns.com) ecosystem:
applications depend on `everruns-serve` and use `serve::prelude::*`, never this
crate directly. Experimental, with no compatibility promise.

## Quick Example

```rust
use serve::prelude::*;

/// Roll one die with the given number of sides.
#[tool]
async fn roll_dice(sides: u32) -> Result<u32> {
    Ok(sides)
}
```

## What It Provides

- `#[agent]`, `#[agent(default)]` and `#[agent(sub)]` for agents and subagents
- `#[tool]`, with optional `needs_approval` predicates over a generated arguments struct
- `#[channel]`, `#[schedule]`, `#[connection]` and `#[eval]`
- Link-time registration that `App::builder().discover()` collects

## Documentation

- [Serve overview](https://docs.everruns.com/framework/serve/)
- [`everruns-serve` API reference](https://docs.rs/everruns-serve)
- [Implementation API reference](https://docs.rs/everruns-serve-macros)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
