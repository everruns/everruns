#![doc = include_str!("../README.md")]

#[deprecated(
    note = "use everruns-integrations with the `openai-decisions` feature; this shim is removed in the next platform release"
)]
pub use everruns_integrations::openai_decisions::*;
