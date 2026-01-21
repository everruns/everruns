//! Scenario loading and generation

mod generator;
mod loader;

pub use generator::generate_synthetic;
pub use loader::load_from_directory;
