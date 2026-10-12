// Messages domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

pub mod commands;
pub mod feedback;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;
