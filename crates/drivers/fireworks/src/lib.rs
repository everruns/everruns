//! Fireworks AI provider driver for Everruns.
//!
//! `everruns-fireworks` is part of the [Everruns](https://everruns.com)
//! ecosystem. It implements the [`ChatDriver`] contract from `everruns-provider`
//! and registers the Fireworks AI provider into a [`DriverRegistry`].
//!
//! [Fireworks AI](https://fireworks.ai) serves open models (Llama, Qwen,
//! DeepSeek, GLM, Kimi, gpt-oss, ...) behind an OpenAI-compatible Chat
//! Completions API, so [`FireworksChatDriver`] wraps
//! `everruns_provider::OpenAIProtocolChatDriver` tagged with `DriverId::Fireworks`.
//! Its `/models` endpoint advertises richer metadata (`supports_chat`,
//! `supports_tools`, `supports_image_input`, `context_length`) that this crate
//! parses into capability profiles at discovery time.
//!
//! # Authentication
//!
//! Fireworks authenticates with a single API key, sent as a bearer token by the
//! underlying protocol driver's default (non-Azure) auth path.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_provider::DriverRegistry;
//! use everruns_fireworks::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! assert!(registry.has_driver(&everruns_provider::DriverId::Fireworks));
//! ```

mod driver;

pub use driver::{
    FIREWORKS_DEFAULT_API_URL, FireworksChatDriver, is_fireworks_api_url, provider, register_driver,
};

// Re-export core types for convenience.
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
