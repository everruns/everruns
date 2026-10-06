#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations with the `web-fetch` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations::web_fetch::*;
