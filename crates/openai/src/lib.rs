// OpenAI Driver Implementation
//
// This crate provides OpenAI LLM driver implementations:
//
// - OpenAILlmDriver: Uses Open Responses API (https://www.openresponses.org/)
//   Recommended for new projects with better performance.
//
// - OpenAICompletionsLlmDriver: Uses Chat Completions API
//   For backward compatibility with /v1/chat/completions endpoint.
//
// Design: This crate depends on everruns-core and registers its drivers
// at application startup via register_driver(). This enables dependency
// inversion - core has no knowledge of specific provider implementations.

mod driver;
mod types;

#[cfg(test)]
mod tests;

pub use driver::{OpenAICompletionsLlmDriver, OpenAILlmDriver, register_driver};
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
