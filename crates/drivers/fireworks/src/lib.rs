//! **Moved.** `everruns-fireworks` is now the [`fireworks`](everruns_drivers::fireworks) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `fireworks` feature instead,
//! and replace `everruns_fireworks::` with `everruns_drivers::fireworks::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::fireworks::register_driver(&mut registry);
//! ```

pub use everruns_drivers::fireworks::*;

/// Moved to [`everruns_drivers::fireworks::FireworksChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::fireworks::FireworksChatDriver; everruns-fireworks is no longer updated"
)]
pub type FireworksChatDriver = everruns_drivers::fireworks::FireworksChatDriver;

/// Moved to [`everruns_drivers::fireworks::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::fireworks::register_driver; everruns-fireworks is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_provider::driver_registry::DriverRegistry) {
    everruns_drivers::fireworks::register_driver(registry);
}

/// Moved to [`everruns_drivers::fireworks::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::fireworks::descriptor; everruns-fireworks is no longer updated"
)]
pub fn descriptor() -> everruns_provider::driver_registry::DriverDescriptor {
    everruns_drivers::fireworks::descriptor()
}

/// Moved to [`everruns_drivers::fireworks::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::fireworks::from_env; everruns-fireworks is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_provider::ProviderKey>,
) -> Result<everruns_provider::Provider, everruns_provider::credential_provider::EnvCredentialError>
{
    everruns_drivers::fireworks::from_env(id)
}
