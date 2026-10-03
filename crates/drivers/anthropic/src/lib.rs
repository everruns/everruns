//! **Moved.** `everruns-anthropic` is now the [`anthropic`](everruns_drivers::anthropic) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `anthropic` feature instead,
//! and replace `everruns_anthropic::` with `everruns_drivers::anthropic::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::anthropic::register_driver(&mut registry);
//! ```

pub use everruns_drivers::anthropic::*;

/// Moved to [`everruns_drivers::anthropic::AnthropicChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::anthropic::AnthropicChatDriver; everruns-anthropic is no longer updated"
)]
pub type AnthropicChatDriver = everruns_drivers::anthropic::AnthropicChatDriver;

/// Moved to [`everruns_drivers::anthropic::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::anthropic::register_driver; everruns-anthropic is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_contracts::driver_registry::DriverRegistry) {
    everruns_drivers::anthropic::register_driver(registry);
}

/// Moved to [`everruns_drivers::anthropic::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::anthropic::descriptor; everruns-anthropic is no longer updated"
)]
pub fn descriptor() -> everruns_contracts::driver_registry::DriverDescriptor {
    everruns_drivers::anthropic::descriptor()
}

/// Moved to [`everruns_drivers::anthropic::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::anthropic::from_env; everruns-anthropic is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_contracts::ProviderKey>,
) -> Result<everruns_contracts::Provider, everruns_contracts::credential_provider::EnvCredentialError>
{
    everruns_drivers::anthropic::from_env(id)
}
