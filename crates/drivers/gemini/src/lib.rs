//! **Moved.** `everruns-gemini` is now the [`gemini`](everruns_drivers::gemini) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `gemini` feature instead,
//! and replace `everruns_gemini::` with `everruns_drivers::gemini::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::gemini::register_driver(&mut registry);
//! ```

pub use everruns_drivers::gemini::*;

/// Moved to [`everruns_drivers::gemini::GeminiChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::gemini::GeminiChatDriver; everruns-gemini is no longer updated"
)]
pub type GeminiChatDriver = everruns_drivers::gemini::GeminiChatDriver;

/// Moved to [`everruns_drivers::gemini::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::gemini::register_driver; everruns-gemini is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_contracts::driver_registry::DriverRegistry) {
    everruns_drivers::gemini::register_driver(registry);
}

/// Moved to [`everruns_drivers::gemini::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::gemini::descriptor; everruns-gemini is no longer updated"
)]
pub fn descriptor() -> everruns_contracts::driver_registry::DriverDescriptor {
    everruns_drivers::gemini::descriptor()
}

/// Moved to [`everruns_drivers::gemini::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::gemini::from_env; everruns-gemini is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_contracts::ProviderKey>,
) -> Result<everruns_contracts::Provider, everruns_contracts::credential_provider::EnvCredentialError>
{
    everruns_drivers::gemini::from_env(id)
}
