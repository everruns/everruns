#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Microsoft MAI provider driver for Everruns.
//!
//! The `mai` module of `everruns-drivers` is part of the [Everruns](https://everruns.com) ecosystem. It
//! implements the [`ChatDriver`] contract from `everruns-contracts` and registers a
//! Microsoft MAI provider (e.g. `mai-code-1-flash`) into a [`DriverRegistry`].
//! The same provider answers calibrated decisions on Microsoft's decision
//! models (`Microsoft-Decision-1`) through [`MaiDecisionDriver`].
//!
//! Microsoft MAI models are served via [Azure AI Foundry](https://ai.azure.com)
//! behind an OpenAI-compatible Chat Completions API, so [`MaiChatDriver`] wraps
//! `everruns_contracts::OpenAIProtocolChatDriver`; its runtime provider owns
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
//! use everruns_contracts::DriverRegistry;
//! use everruns_drivers::mai::register_driver;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```
//!
//! [`ProviderAuth`]: everruns_contracts::ProviderAuth

mod auth;
mod decisions;
mod driver;

pub use auth::{
    DEFAULT_ENTRA_AUTHORITY, DEFAULT_ENTRA_SCOPE, EntraOAuthConfig, EntraOAuthProvider, MaiAuth,
};
pub use decisions::{MaiDecisionDriver, decisions_url};
pub use driver::{MaiChatDriver, descriptor, from_env, provider, register_driver};

// Re-export core types for convenience.
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};
