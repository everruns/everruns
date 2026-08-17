# LLM drivers

This directory groups Everruns model-provider driver packages by repository
concern. It is not a Rust package or a shared release boundary.

Each child remains an independently versioned crate with its existing
`everruns-*` package name and public API. Drivers implement provider-specific
wire protocols over the neutral contracts in
[`everruns-provider`](../provider/README.md); product and Framework composition
remain outside this directory.

The production-safe `everruns-llmsim` driver lives here as the deterministic,
offline implementation of the same provider contract.
