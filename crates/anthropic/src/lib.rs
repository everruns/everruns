// Anthropic Driver Implementation
//
// This crate provides an Anthropic Claude LLM driver implementation.
// It implements the LlmDriver trait from everruns-core, enabling
// the agent loop to communicate with Anthropic's Messages API.
//
// Design: This crate depends on everruns-core and registers its driver
// at application startup via register_driver(). This enables dependency
// inversion - core has no knowledge of specific provider implementations.

mod driver;

#[cfg(test)]
mod tests;

pub use driver::{register_driver, AnthropicLlmDriver};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
