// OpenAI Driver Implementation
//
// This crate provides an OpenAI-compatible LLM driver implementation.
// It implements the LlmDriver trait from everruns-core, enabling
// the agent loop to communicate with OpenAI's chat completion API.
//
// The OpenAI protocol is used as the base for LLM drivers in the system,
// meaning other providers can adapt their APIs to this format.

mod driver;
mod types;

#[cfg(test)]
mod tests;

pub use driver::OpenAILlmDriver;
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_core::llm_drivers::LlmDriver;
