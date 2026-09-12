// Environment domain — where a session's commands run, and what they may touch.
//
// See knowledge/harnesses/execution-environments.md for the model.

pub mod commands;
pub mod queries;
pub mod resolve;

pub use commands::*;
