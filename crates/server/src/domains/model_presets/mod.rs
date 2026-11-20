// Model presets: named sets of models, one per job (knowledge/foundations/model-presets.md).

pub mod builtin;
pub mod commands;
pub mod helper;
pub mod service;
pub mod types;

pub use commands::*;
pub use types::*;

#[cfg(test)]
mod tests;
