# LLM drivers

This directory groups Everruns model-provider driver packages by repository
concern. It is not a Rust package.

Each child remains a separate crates.io package with its existing `everruns-*`
package name and public API, published at the shared platform version. Drivers implement provider-specific
wire protocols over the neutral contracts in
[`everruns-provider`](../provider/README.md); product and Framework composition
remain outside this directory.

The production-safe `everruns-llmsim` driver lives here as the deterministic,
offline implementation of the same provider contract.
