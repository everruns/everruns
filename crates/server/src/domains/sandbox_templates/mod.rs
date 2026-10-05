// Sandbox Template domain — reusable recipes for where a Session's commands
// run and what they may touch.
//
// See knowledge/harnesses/sandbox-templates.md for the model.

pub mod commands;
pub mod queries;
pub mod resolution;
pub mod resolve;

pub use commands::*;
