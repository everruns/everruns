// Google Gemini Driver Implementation
//
// This crate provides a Google Gemini LLM driver implementation.
// It implements the LlmDriver trait from everruns-core, enabling
// the agent loop to communicate with Google's Gemini API.
//
// Design: This crate depends on everruns-core and registers its driver
// at application startup via register_driver(). This enables dependency
// inversion - core has no knowledge of specific provider implementations.

mod driver;

pub use driver::{GeminiLlmDriver, register_driver};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
