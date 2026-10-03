//! **Moved.** `everruns-bedrock` is now the [`bedrock`](everruns_drivers::bedrock) module of
//! [`everruns-drivers`](https://docs.rs/everruns-drivers), part of the
//! [Everruns](https://everruns.com) ecosystem. This crate is a deprecated shim
//! that re-exports that module and receives no further updates.
//!
//! To migrate, depend on `everruns-drivers` with its `bedrock` feature instead,
//! and replace `everruns_bedrock::` with `everruns_drivers::bedrock::`:
//!
//! ```rust
//! let mut registry = everruns_drivers::DriverRegistry::new();
//! everruns_drivers::bedrock::register_driver(&mut registry);
//! ```

pub use everruns_drivers::bedrock::*;

/// Moved to [`everruns_drivers::bedrock::BedrockChatDriver`].
#[deprecated(
    note = "moved to everruns_drivers::bedrock::BedrockChatDriver; everruns-bedrock is no longer updated"
)]
pub type BedrockChatDriver = everruns_drivers::bedrock::BedrockChatDriver;

/// Moved to [`everruns_drivers::bedrock::register_driver`].
#[deprecated(
    note = "moved to everruns_drivers::bedrock::register_driver; everruns-bedrock is no longer updated"
)]
pub fn register_driver(registry: &mut everruns_contracts::driver_registry::DriverRegistry) {
    everruns_drivers::bedrock::register_driver(registry);
}

/// Moved to [`everruns_drivers::bedrock::descriptor`].
#[deprecated(
    note = "moved to everruns_drivers::bedrock::descriptor; everruns-bedrock is no longer updated"
)]
pub fn descriptor() -> everruns_contracts::driver_registry::DriverDescriptor {
    everruns_drivers::bedrock::descriptor()
}

/// Moved to [`everruns_drivers::bedrock::from_env`].
#[deprecated(
    note = "moved to everruns_drivers::bedrock::from_env; everruns-bedrock is no longer updated"
)]
pub fn from_env(
    id: impl Into<everruns_contracts::ProviderKey>,
) -> Result<everruns_contracts::Provider, everruns_contracts::credential_provider::EnvCredentialError>
{
    everruns_drivers::bedrock::from_env(id)
}
