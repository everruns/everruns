//! OpenAI provider drivers for Everruns.
//!
//! `everruns-openai` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-core` and
//! registers OpenAI providers into a [`DriverRegistry`].
//!
//! The crate exposes two drivers:
//!
//! - [`OpenAIChatDriver`], the recommended Responses API driver.
//! - [`OpenAICompletionsChatDriver`], a Chat Completions compatibility driver.
//!
//! OpenRouter lives in the separate `everruns-openrouter` crate.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_provider::DriverRegistry;
//! use everruns_openai::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```
//!
//! # Embedded Agent Example
//!
//! ```ignore
//! use everruns_provider::{
//!     CapabilityRegistry, CredentialProvider, DriverRegistry, DriverId, EnvCredentialProvider,
//!     ResolvedModel, PlatformDefinition,
//! };
//! use everruns_runtime::InProcessRuntimeBuilder;
//!
//! let mut drivers = DriverRegistry::new();
//! everruns_openai::register_driver(&mut drivers);
//!
//! // Standalone/dev: resolve credentials through the injectable env provider.
//! // Driver code never reads the environment itself (see knowledge/foundations/llm-drivers.md).
//! let creds = EnvCredentialProvider
//!     .resolve(&DriverId::OpenAI)
//!     .expect("OPENAI_API_KEY not set");
//!
//! let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);
//! let runtime = InProcessRuntimeBuilder::new()
//!     .platform_definition(platform)
//!     .default_model(ResolvedModel {
//!         model: "gpt-5.4-mini".into(),
//!         provider_type: DriverId::OpenAI,
//!         api_key: creds.api_key,
//!         base_url: creds.base_url,
//!     })
//!     .single_session(|s| {
//!         s.harness("assistant", "You are a helpful assistant.")
//!             .agent("openai-agent", "Answer clearly and concisely.")
//!     })
//!     .build()
//!     .await?;
//!
//! let session_id = runtime.default_session_id().expect("single_session id");
//! let result = runtime.run_text_turn(session_id, "Write a one-line status update.").await?;
//! println!("{}", result.response);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod driver;
pub(crate) mod embeddings;
mod types;

#[cfg(test)]
mod tests;

pub use driver::{OpenAIChatDriver, OpenAICompletionsChatDriver, register_driver};
pub use embeddings::OpenAIEmbeddingsDriver;
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
