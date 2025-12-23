// Anthropic Provider Implementation
//
// This crate provides an Anthropic Claude LLM provider implementation.
// It implements the LlmProvider trait from everruns-core, enabling
// the agent loop to communicate with Anthropic's Messages API.

mod provider;

#[cfg(test)]
mod tests;

pub use provider::AnthropicProvider;

// Re-export core types for convenience
pub use everruns_core::llm::LlmProvider;
