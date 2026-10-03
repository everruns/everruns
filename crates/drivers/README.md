# LLM drivers

This directory groups Everruns model-provider driver code. It is not a Rust
package.

- `drivers/` (`everruns-drivers`) holds every vendor driver as a feature-gated
  module over the neutral contracts in
  [`everruns-provider`](../provider/README.md). A new vendor is a new module and
  feature there, not a new crate.
- `llmsim/` (`everruns-llmsim`) is the production-safe deterministic, offline
  implementation of the same provider contract. It stays its own crate because
  its `host` feature depends on `everruns-host`, which depends on
  `everruns-drivers`.
- `anthropic/`, `bedrock/`, `fireworks/`, `gemini/`, `mai/`, `meta/`, `openai/`,
  and `openrouter/` are deprecated shim crates that re-export the matching
  `everruns-drivers` module under the old package name. They ship for one
  release so existing dependents keep building and see the move on crates.io
  and docs.rs, then they are deleted.

Product and Framework composition remain outside this directory.
