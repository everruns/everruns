//! **Moved.** `everruns-mai` is now the [`mai`](everruns_drivers::mai) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `mai` feature instead,
//! and replace `everruns_mai::` with `everruns_drivers::mai::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::mai::register_driver(&mut registry);
//! ```

pub use everruns_drivers::mai::*;

/// Moved to [`everruns_drivers::mai::MaiChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::mai::MaiChatDriver; everruns-mai is no longer updated"
)]
pub type MaiChatDriver = everruns_drivers::mai::MaiChatDriver;

/// Moved to [`everruns_drivers::mai::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::mai::register_driver; everruns-mai is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_contracts::driver_registry::DriverRegistry) {
    everruns_drivers::mai::register_driver(registry);
}

/// Moved to [`everruns_drivers::mai::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::mai::descriptor; everruns-mai is no longer updated"
)]
pub fn descriptor() -> everruns_contracts::driver_registry::DriverDescriptor {
    everruns_drivers::mai::descriptor()
}

/// Moved to [`everruns_drivers::mai::from_env`].
#[deprecated(note = "moved to everruns_drivers::mai::from_env; everruns-mai is no longer updated")]
pub fn from_env(
    id: impl Into<everruns_contracts::ProviderKey>,
) -> Result<everruns_contracts::Provider, everruns_contracts::credential_provider::EnvCredentialError>
{
    everruns_drivers::mai::from_env(id)
}
