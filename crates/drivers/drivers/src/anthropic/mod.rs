//! Anthropic Claude provider driver for Everruns.
//!
//! The `anthropic` module of `everruns-drivers` is part of the [Everruns](https://everruns.com)
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
//! use everruns_drivers::anthropic::{AnthropicChatDriver, provider, register_driver};
//! use everruns_contracts::DriverRegistry;
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

// Lint floor debt carried over from the former everruns-anthropic crate, which
// sat on scripts/lib/lint-floor-allowlist.txt (33 production unwrap/expect
// sites, mostly `Mutex::lock().unwrap()`). Scoped to this module so the rest
// of everruns-drivers stays on the floor; delete it once the sites are paid.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod computer_toolset;
mod driver;
mod effort;
mod prefill;
mod server_compaction;

pub use driver::{AnthropicChatDriver, descriptor, from_env, provider, register_driver};

// Re-export core types for convenience
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};
