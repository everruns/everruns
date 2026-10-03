//! **Moved.** `everruns-meta` is now the [`meta`](everruns_drivers::meta) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `meta` feature instead,
//! and replace `everruns_meta::` with `everruns_drivers::meta::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::meta::register_driver(&mut registry);
//! ```

pub use everruns_drivers::meta::*;

/// Moved to [`everruns_drivers::meta::MetaChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::meta::MetaChatDriver; everruns-meta is no longer updated"
)]
pub type MetaChatDriver = everruns_drivers::meta::MetaChatDriver;

/// Moved to [`everruns_drivers::meta::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::meta::register_driver; everruns-meta is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_provider::driver_registry::DriverRegistry) {
    everruns_drivers::meta::register_driver(registry);
}

/// Moved to [`everruns_drivers::meta::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::meta::descriptor; everruns-meta is no longer updated"
)]
pub fn descriptor() -> everruns_provider::driver_registry::DriverDescriptor {
    everruns_drivers::meta::descriptor()
}

/// Moved to [`everruns_drivers::meta::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::meta::from_env; everruns-meta is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> Result<everruns_provider::Provider, everruns_provider::credential_provider::EnvCredentialError>
{
    everruns_drivers::meta::from_env(id)
}
