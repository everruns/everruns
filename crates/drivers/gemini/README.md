# everruns-gemini

> Moved: Google Gemini support is now the `gemini` module of `everruns-drivers`.

Part of the [Everruns](https://everruns.com) ecosystem. `everruns-gemini` is a
deprecated shim that re-exports
[`everruns_drivers::gemini`](https://docs.rs/everruns-drivers/latest/everruns_drivers/gemini/)
and receives no further updates. This is its last release.

## Migrate

Replace the dependency with `everruns-drivers` and its `gemini` feature, which
ships it from 0.35:

```toml
# before
everruns-gemini = "0.34"
# after
everruns-drivers = { version = "0.35", features = ["gemini"] }
```

Then replace `everruns_gemini::` with `everruns_drivers::gemini::` in your code:

```rust
let mut registry = everruns_drivers::DriverRegistry::new();
everruns_drivers::gemini::register_driver(&mut registry);
```

Using the [`everruns`](https://crates.io/crates/everruns) facade? Nothing to
change: its `gemini` feature turns on the new module, re-exported as
`everruns::drivers::gemini`.

## What It Provides

- A glob re-export of `everruns_drivers::gemini`, so existing code keeps building
- `#[deprecated]` wrappers for the driver type, `register_driver`,
  `descriptor`, and `from_env`, so the compiler points at every call site to
  change

## Documentation

- [`everruns-drivers` API reference (docs.rs)](https://docs.rs/everruns-drivers)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
