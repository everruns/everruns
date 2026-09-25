# everruns-serve-build

`build.rs` helper for [`everruns-serve`](https://crates.io/crates/everruns-serve)
apps. It embeds `agent/**/*.md` and `serve.toml` into the binary.

```rust
// build.rs
fn main() {
    serve_build::embed();
}
```

> **Experimental.** No compatibility promise.
