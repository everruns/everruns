// OpenAI Provider Implementation
//
// This crate provides an OpenAI-compatible LLM provider implementation.
// It implements the LlmProvider trait from everruns-core, enabling
// the agent loop to communicate with OpenAI's chat completion API.
//
// The OpenAI protocol is used as the base for LLM providers in the system,
// meaning other providers can adapt their APIs to this format.

mod provider;
mod types;

#[cfg(test)]
mod tests;

pub use provider::OpenAiProvider;
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_core::traits::LlmProvider;
