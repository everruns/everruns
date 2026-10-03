# everruns-fireworks

> Moved: Fireworks AI support is now the `fireworks` module of `everruns-drivers`.

Part of the [Everruns](https://everruns.com) ecosystem. `everruns-fireworks` is a
deprecated shim that re-exports
[`everruns_drivers::fireworks`](https://docs.rs/everruns-drivers/latest/everruns_drivers/fireworks/)
and receives no further updates. This is its last release.

## Migrate

Replace the dependency with `everruns-drivers` and its `fireworks` feature, which
ships it from 0.35:

```toml
# before
everruns-fireworks = "0.34"
# after
everruns-drivers = { version = "0.35", features = ["fireworks"] }
```

Then replace `everruns_fireworks::` with `everruns_drivers::fireworks::` in your code:

```rust
let mut registry = everruns_drivers::DriverRegistry::new();
everruns_drivers::fireworks::register_driver(&mut registry);
```


## What It Provides

- A glob re-export of `everruns_drivers::fireworks`, so existing code keeps building
- `#[deprecated]` wrappers for the driver type, `register_driver`,
  `descriptor`, and `from_env`, so the compiler points at every call site to
  change

## Documentation

- [`everruns-drivers` API reference (docs.rs)](https://docs.rs/everruns-drivers)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
