//! OpenRouter provider driver for Everruns.
//!
//! `everruns-openrouter` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-core`
//! and registers the OpenRouter provider into a [`DriverRegistry`].
//!
//! OpenRouter exposes an OpenAI-compatible Responses API, so [`OpenRouterChatDriver`]
//! wraps `everruns_core::OpenResponsesProtocolChatDriver` tagged with
//! `DriverId::OpenRouter`. Its `/models` endpoint advertises richer metadata
//! (a `supported_parameters` array) that the crate parses into capability
//! profiles at discovery time.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_core::DriverRegistry;
//! use everruns_openrouter::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

mod driver;
mod request_ext;
mod types;

pub use driver::{OpenRouterChatDriver, register_driver};
pub use request_ext::OpenRouterRequestExtension;
pub use types::{
    OpenRouterArchitecture, OpenRouterModelInfo, OpenRouterModelsResponse, OpenRouterPricing,
    OpenRouterTopProvider,
};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{ChatDriver, DriverRegistry};
