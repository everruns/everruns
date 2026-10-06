#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations with the `duckduckgo` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations::duckduckgo::*;
