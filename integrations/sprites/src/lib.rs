#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations-experimental with the `sprites` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations_experimental::sprites::*;
