#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! OpenAI provider drivers for Everruns.
//!
//! The `openai` module of `everruns-drivers` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-contracts` and
//! registers OpenAI providers into a [`DriverRegistry`].
//!
//! The crate exposes two drivers:
//!
//! - [`OpenAIChatDriver`], the recommended Responses API driver.
//! - [`OpenAICompletionsChatDriver`], a Chat Completions compatibility driver.
//!
//! OpenRouter lives in the sibling `openrouter` module.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_contracts::DriverRegistry;
//! use everruns_drivers::openai::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```
//!
//!
//! Application authors normally configure OpenAI through the
//! application-facing `everruns::OpenAI` value. See the
//! [Framework model guide](https://docs.everruns.com/framework/models-and-providers/).

mod driver;
pub(crate) mod embeddings;
mod types;

#[cfg(test)]
mod tests;

pub use driver::{
    OpenAIChatDriver, OpenAICompletionsChatDriver, azure_descriptor, azure_provider,
    completions_descriptor, completions_provider, descriptor, from_env, provider, register_driver,
};
pub use embeddings::OpenAIEmbeddingsDriver;
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};

pub mod async_tools;

pub mod decisions;
