//! Anthropic Claude provider driver for Everruns.
//!
//! `everruns-anthropic` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`LlmDriver`] contract from `everruns-core` and
//! registers Anthropic's Messages API driver into a [`DriverRegistry`].
//!
//! Provider crates depend on `everruns-core`; `everruns-core` does not depend on
//! provider implementations. Hosts register whichever drivers they want to make
//! available.
//!
//! # Example
//!
//! ```
//! use everruns_anthropic::{AnthropicLlmDriver, register_driver};
//! use everruns_core::DriverRegistry;
//!
//! let driver = AnthropicLlmDriver::new("your-api-key");
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//!
//! assert!(format!("{driver:?}").contains("AnthropicLlmDriver"));
//! ```

mod driver;

#[cfg(test)]
mod tests;

pub use driver::{AnthropicLlmDriver, register_driver};

// Re-export core types for convenience
pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
