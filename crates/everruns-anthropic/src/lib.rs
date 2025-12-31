// Anthropic Driver Implementation
//
// This crate provides an Anthropic Claude LLM driver implementation.
// It implements the LlmDriver trait from everruns-core, enabling
// the agent loop to communicate with Anthropic's Messages API.

mod driver;

#[cfg(test)]
mod tests;

pub use driver::AnthropicLlmDriver;

// Re-export core types for convenience
pub use everruns_core::llm_drivers::LlmDriver;
