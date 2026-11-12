#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Google Gemini LLM provider driver for Everruns.
//!
//! The `gemini` module of `everruns-drivers` is part of the [Everruns](https://everruns.com) ecosystem.
//! It implements the `ChatDriver` contract from `everruns-contracts`, so the agent
//! loop can run against Google's Gemini API, and registers itself through
//! [`register_driver`]. The same provider answers calibrated decisions on
//! Gemini Decision 1 through `GeminiDecisionDriver` (native targets).
//!
//! Core has no knowledge of specific providers; hosts register whichever drivers
//! they want available via `register_driver` at startup.
//!
//! # Example
//!
//! ```
//! use everruns_drivers::gemini::register_driver;
//! use everruns_contracts::DriverRegistry;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

// Decisions share the System One transport, which is native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod decisions;
mod driver;
mod grounding;

#[cfg(not(target_arch = "wasm32"))]
pub use decisions::GeminiDecisionDriver;
pub use driver::{GeminiChatDriver, descriptor, from_env, provider, register_driver};

// Re-export core types for convenience
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};
