// Capability domain — commands, queries, types.
//
// Read-only registry. No create/update/delete operations.
// See specs/domains.md for the pattern.

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;
