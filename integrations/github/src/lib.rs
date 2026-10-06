#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations with the `github` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations::github::*;
