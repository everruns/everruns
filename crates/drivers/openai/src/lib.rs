//! **Moved.** `everruns-openai` is now the [`openai`](everruns_drivers::openai) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `openai` feature instead,
//! and replace `everruns_openai::` with `everruns_drivers::openai::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::openai::register_driver(&mut registry);
//! ```

pub use everruns_drivers::openai::*;

/// Moved to [`everruns_drivers::openai::OpenAIChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::openai::OpenAIChatDriver; everruns-openai is no longer updated"
)]
pub type OpenAIChatDriver = everruns_drivers::openai::OpenAIChatDriver;

/// Moved to [`everruns_drivers::openai::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::openai::register_driver; everruns-openai is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_provider::driver_registry::DriverRegistry) {
    everruns_drivers::openai::register_driver(registry);
}

/// Moved to [`everruns_drivers::openai::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::openai::descriptor; everruns-openai is no longer updated"
)]
pub fn descriptor() -> everruns_provider::driver_registry::DriverDescriptor {
    everruns_drivers::openai::descriptor()
}

/// Moved to [`everruns_drivers::openai::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::openai::from_env; everruns-openai is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> Result<everruns_provider::Provider, everruns_provider::credential_provider::EnvCredentialError>
{
    everruns_drivers::openai::from_env(id)
}
