//! Anthropic Claude provider driver for Everruns.
//!
//! `everruns-anthropic` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-provider` and
//! registers Anthropic's Messages API driver into a [`DriverRegistry`].
//!
//! Provider crates depend on `everruns-provider`; the provider SPI does not depend on
//! provider implementations. Hosts register whichever drivers they want to make
//! available.
//!
//! # Example
//!
//! ```
//! use everruns_anthropic::{AnthropicChatDriver, provider, register_driver};
//! use everruns_provider::DriverRegistry;
//!
//! let driver = AnthropicChatDriver::new();
//! let service = provider("anthropic", "your-api-key");
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//!
//! assert!(format!("{driver:?}").contains("AnthropicChatDriver"));
//! assert_eq!(service.id().as_str(), "anthropic");
//! ```

mod driver;

#[cfg(test)]
mod tests;

pub use driver::{AnthropicChatDriver, provider, register_driver};

// Re-export core types for convenience
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
