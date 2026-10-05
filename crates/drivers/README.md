# LLM drivers

This directory groups Everruns model-provider driver code. It is not a Rust
package.

- [`drivers/`](drivers/README.md) (`everruns-drivers`) holds every vendor driver as a feature-gated
  module over the neutral contracts in
  [`everruns-contracts`](../contracts/README.md). A new vendor is a new module and
  feature there, not a new crate.
- `llmsim/` (`everruns-llmsim`) is the production-safe deterministic, offline
  implementation of the same provider contract. It stays its own crate because
  its `host` feature depends on `everruns-host`, which depends on
  `everruns-drivers`.

Product and Framework composition remain outside this directory.

For driver usage, see the package's [quick start](drivers/README.md#quick-start-get-a-reply),
[runnable examples](drivers/README.md#runnable-examples), and
[Rust API reference](https://docs.rs/everruns-drivers).
