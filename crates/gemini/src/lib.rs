//! Google Gemini LLM provider driver for Everruns.
//!
//! `everruns-gemini` is part of the [Everruns](https://everruns.com) ecosystem.
//! It implements the `ChatDriver` contract from `everruns-provider`, so the agent
//! loop can run against Google's Gemini API, and registers itself through
//! [`register_driver`].
//!
//! Core has no knowledge of specific providers; hosts register whichever drivers
//! they want available via `register_driver` at startup.
//!
//! # Example
//!
//! ```
//! use everruns_gemini::register_driver;
//! use everruns_provider::DriverRegistry;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

mod driver;

pub use driver::{GeminiChatDriver, provider, register_driver};

// Re-export core types for convenience
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
