// OpenRouter Driver Implementation
//
// OpenRouter (https://openrouter.ai/) is a multi-provider LLM router that provides
// access to models from OpenAI, Anthropic, Google, Meta, and others through a
// unified OpenAI-compatible Chat Completions API.
//
// Design: Wraps OpenAIProtocolLlmDriver from everruns-core since OpenRouter uses
// the same wire protocol. This crate adds OpenRouter-specific model discovery.

mod driver;

pub use driver::{OpenRouterLlmDriver, register_driver};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
