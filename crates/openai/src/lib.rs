//! OpenAI provider drivers for Everruns.
//!
//! `everruns-openai` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`LlmDriver`] contract from `everruns-core` and
//! registers OpenAI providers into a [`DriverRegistry`].
//!
//! The crate exposes two drivers:
//!
//! - [`OpenAILlmDriver`], the recommended Responses API driver.
//! - [`OpenAICompletionsLlmDriver`], a Chat Completions compatibility driver.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_core::DriverRegistry;
//! use everruns_openai::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```
//!
//! # Embedded Agent Example
//!
//! ```ignore
//! use everruns_core::{
//!     CapabilityRegistry, DriverRegistry, LlmProviderType, ModelWithProvider, PlatformDefinition,
//! };
//! use everruns_runtime::InProcessRuntimeBuilder;
//!
//! let mut drivers = DriverRegistry::new();
//! everruns_openai::register_driver(&mut drivers);
//!
//! let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);
//! let runtime = InProcessRuntimeBuilder::new()
//!     .platform_definition(platform)
//!     .default_model(ModelWithProvider {
//!         model: "gpt-5.4-mini".into(),
//!         provider_type: LlmProviderType::Openai,
//!         api_key: Some(std::env::var("OPENAI_API_KEY")?),
//!         base_url: None,
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
mod image_capability;
mod images;
mod types;

#[cfg(test)]
mod tests;

pub use driver::{OpenAICompletionsLlmDriver, OpenAILlmDriver, register_driver};
pub use image_capability::{EditImageTool, GenerateImageTool, GptImageGenCapability};
pub use types::{
    ChatMessage, ChatRequest, CompletionMetadata, LlmConfig, LlmStreamEvent, MessageRole,
};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
