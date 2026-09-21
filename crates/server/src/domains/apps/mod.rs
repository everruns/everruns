// Frozen App compatibility domain — archival queries and invocation runtime.
//
// See knowledge/foundations/domains.md for the pattern.
mod archival;

pub mod invocation;
pub mod queries;
pub mod types;
pub use archival::*;
pub use invocation::*;
