//! Microsoft MAI provider driver for Everruns.
//!
//! `everruns-mai` is part of the [Everruns](https://everruns.com) ecosystem. It
//! implements the [`ChatDriver`] contract from `everruns-provider` and registers a
//! Microsoft MAI provider (e.g. `mai-code-1-flash`) into a [`DriverRegistry`].
//!
//! Microsoft MAI models are served via [Azure AI Foundry](https://ai.azure.com)
//! behind an OpenAI-compatible Chat Completions API, so [`MaiChatDriver`] wraps
//! `everruns_provider::OpenAIProtocolChatDriver`; its runtime provider owns
//! authentication through [`ProviderAuth`].
//!
//! # Authentication
//!
//! Two schemes are supported, both selected from the provider configuration:
//!
//! - **Azure AI Foundry API key** — the resource key, sent as `api-key`.
//! - **Microsoft Entra ID (OAuth)** — a client-credentials service principal
//!   (`tenant_id`, `client_id`, `client_secret`), supplied through provider
//!   metadata. Bearer tokens are minted and cached, refreshed before expiry.
//!
//! Additional schemes (managed identity, workload identity federation, ...) can
//! be added by implementing [`ProviderAuth`] without changing the driver.
//!
//! # Registering the Driver
//!
//! ```
//! use everruns_provider::DriverRegistry;
//! use everruns_mai::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```
//!
//! [`ProviderAuth`]: everruns_provider::ProviderAuth

mod auth;
mod driver;

pub use auth::{
    DEFAULT_ENTRA_AUTHORITY, DEFAULT_ENTRA_SCOPE, EntraOAuthConfig, EntraOAuthProvider, MaiAuth,
};
pub use driver::{MaiChatDriver, provider, register_driver};

// Re-export core types for convenience.
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
