# everruns-macros

> Procedural-macro implementation for typed Everruns Framework tools.

[![Crates.io](https://img.shields.io/crates/v/everruns-macros.svg)](https://crates.io/crates/everruns-macros)
[![Documentation](https://docs.rs/everruns-macros/badge.svg)](https://docs.rs/everruns-macros)
[![License](https://img.shields.io/crates/l/everruns-macros.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-macros` implements the tool attribute that generates argument JSON
Schema and an async function adapter.

It is an implementation crate in the [Everruns](https://everruns.com)
ecosystem. Its repository directory is `crates/macros`, while its published
package name remains `everruns-macros`. Applications depend on `everruns` and
use the default-enabled `everruns::tool` re-export.

## Quick Example

```rust
use everruns::{Agent, Model};

#[everruns::tool]
/// Add two integers.
async fn add(left: i64, right: i64) -> Result<i64, String> {
    Ok(left + right)
}

let agent = Agent::builder()
    .instructions("Use the tool for arithmetic.")
    .model(Model::simulated("Ready."))
    .tool(add())
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

## What It Provides

- The implementation behind `#[everruns::tool]`
- Typed argument schema generation
- Async function and result adaptation
- Compile-time validation for unsupported signatures

## Documentation

- [Framework tools and macros](https://docs.everruns.com/framework/tools-and-macros/)
- [`everruns` API reference](https://docs.rs/everruns)
- [Implementation API reference](https://docs.rs/everruns-macros)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
