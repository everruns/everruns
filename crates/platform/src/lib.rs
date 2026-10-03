//! **Moved.** Hosted capabilities now live in [`everruns-capabilities`](https://docs.rs/everruns-capabilities).
//! Control-plane records belong to the server. This deprecated shim ships once.
//!
//! ```
//! use everruns_capabilities::capabilities::hosted_capability_registry;
//! let _registry = hosted_capability_registry();
//! ```
pub use everruns_capabilities::*;
/// Moved to `everruns_capabilities::PlatformStoreExt`.
#[deprecated(note = "moved to everruns_capabilities; everruns-platform is no longer updated")]
pub type PlatformStoreExt = everruns_capabilities::PlatformStoreExt;
