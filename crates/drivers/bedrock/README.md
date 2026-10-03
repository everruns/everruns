# everruns-bedrock

> Moved: AWS Bedrock support is now the `bedrock` module of `everruns-drivers`.

Part of the [Everruns](https://everruns.com) ecosystem. `everruns-bedrock` is a
deprecated shim that re-exports
[`everruns_drivers::bedrock`](https://docs.rs/everruns-drivers/latest/everruns_drivers/bedrock/)
and receives no further updates. This is its last release.

## Migrate

Replace the dependency with `everruns-drivers` and its `bedrock` feature, which
ships it from 0.35:

```toml
# before
everruns-bedrock = "0.34"
# after
everruns-drivers = { version = "0.35", features = ["bedrock"] }
```

Then replace `everruns_bedrock::` with `everruns_drivers::bedrock::` in your code:

```rust
let mut registry = everruns_drivers::DriverRegistry::new();
everruns_drivers::bedrock::register_driver(&mut registry);
```

Bedrock's `default-credentials` feature is `bedrock-default-credentials` in
`everruns-drivers`.

Using the [`everruns`](https://crates.io/crates/everruns) facade? Nothing to
change: its `bedrock` feature turns on the new module, re-exported as
`everruns::drivers::bedrock`.

## What It Provides

- A glob re-export of `everruns_drivers::bedrock`, so existing code keeps building
- `#[deprecated]` wrappers for the driver type, `register_driver`,
  `descriptor`, and `from_env`, so the compiler points at every call site to
  change

## Documentation

- [`everruns-drivers` API reference (docs.rs)](https://docs.rs/everruns-drivers)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
