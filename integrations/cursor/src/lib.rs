#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations with the `cursor` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations::cursor::*;
