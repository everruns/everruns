#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! OpenRouter provider driver for Everruns.
//!
//! The `openrouter` module of `everruns-drivers` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-provider`
//! and registers the OpenRouter provider into a [`DriverRegistry`].
//!
//! OpenRouter exposes an OpenAI-compatible Responses API, so [`OpenRouterChatDriver`]
//! wraps `everruns_contracts::OpenResponsesProtocolChatDriver` tagged with
//! `DriverId::OpenRouter`. Its `/models` endpoint advertises richer metadata
//! (a `supported_parameters` array) that the crate parses into capability
//! profiles at discovery time.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_contracts::DriverRegistry;
//! use everruns_drivers::openrouter::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

mod driver;
pub mod options;
mod request_ext;
mod types;

pub use driver::{OpenRouterChatDriver, descriptor, from_env, provider, register_driver};
pub use request_ext::OpenRouterRequestExtension;
pub use types::{
    OpenRouterArchitecture, OpenRouterModelInfo, OpenRouterModelsResponse, OpenRouterPricing,
    OpenRouterTopProvider,
};

// Re-export core types for convenience
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};
