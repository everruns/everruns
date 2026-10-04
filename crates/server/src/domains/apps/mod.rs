// Frozen App compatibility domain — read-only archival queries.
//
// The live endpoint invocation runtime lives in `domains::agent_channels`.
// See knowledge/foundations/domains.md for the pattern.
mod archival;

pub mod queries;
pub mod types;
pub use archival::*;
