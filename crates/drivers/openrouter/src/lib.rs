//! **Moved.** `everruns-openrouter` is now the [`openrouter`](everruns_drivers::openrouter) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `openrouter` feature instead,
//! and replace `everruns_openrouter::` with `everruns_drivers::openrouter::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::openrouter::register_driver(&mut registry);
//! ```

pub use everruns_drivers::openrouter::*;

/// Moved to [`everruns_drivers::openrouter::OpenRouterChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::openrouter::OpenRouterChatDriver; everruns-openrouter is no longer updated"
)]
pub type OpenRouterChatDriver = everruns_drivers::openrouter::OpenRouterChatDriver;

/// Moved to [`everruns_drivers::openrouter::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::openrouter::register_driver; everruns-openrouter is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_provider::driver_registry::DriverRegistry) {
    everruns_drivers::openrouter::register_driver(registry);
}

/// Moved to [`everruns_drivers::openrouter::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::openrouter::descriptor; everruns-openrouter is no longer updated"
)]
pub fn descriptor() -> everruns_provider::driver_registry::DriverDescriptor {
    everruns_drivers::openrouter::descriptor()
}

/// Moved to [`everruns_drivers::openrouter::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::openrouter::from_env; everruns-openrouter is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> Result<everruns_provider::Provider, everruns_provider::credential_provider::EnvCredentialError>
{
    everruns_drivers::openrouter::from_env(id)
}
