# everruns-serve-build

> Build-script helper that embeds an `everruns-serve` app's prompts and config.

[![Crates.io](https://img.shields.io/crates/v/everruns-serve-build.svg)](https://crates.io/crates/everruns-serve-build)
[![Documentation](https://docs.rs/everruns-serve-build/badge.svg)](https://docs.rs/everruns-serve-build)
[![License](https://img.shields.io/crates/l/everruns-serve-build.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-serve-build` is the `build.rs` companion to
[`everruns-serve`](https://crates.io/crates/everruns-serve) in the
[Everruns](https://everruns.com) ecosystem. It has no dependencies, so an app's
build script does not compile the runtime twice. Experimental, with no
compatibility promise.

## Quick Example

```rust
// build.rs
fn main() {
    serve_build::embed();
}
```

Then call `serve::assets!();` once in `main.rs`.

## What It Provides

- Embeds every file under `agent/` (instructions, skills) and `serve.toml`
- Emits `cargo:rerun-if-changed`, so adding a skill re-runs the build script
- Skips hidden and binary files

## Documentation

- [Serve overview](https://docs.everruns.com/framework/serve/)
- [API reference](https://docs.rs/everruns-serve-build)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
