//! **Moved.** Hosted capabilities now live in [`everruns-capabilities`](https://docs.rs/everruns-capabilities).
//! Control-plane records belong to the server. This deprecated shim ships once.
//!
//! ```
//! use everruns_capabilities::capabilities::hosted_capability_registry;
//! let _registry = hosted_capability_registry();
//! ```
/// Moved to `everruns_capabilities::PlatformStoreExt`.
#[deprecated(note = "moved to everruns_capabilities; everruns-platform is no longer updated")]
pub use everruns_capabilities::PlatformStoreExt;
pub use everruns_capabilities::*;

#[cfg(test)]
mod tests {
    #[test]
    #[allow(deprecated)]
    fn legacy_extension_keeps_the_canonical_tuple_constructor() {
        let constructor: fn(
            std::sync::Arc<dyn everruns_capabilities::PlatformStore>,
        ) -> everruns_capabilities::PlatformStoreExt = crate::PlatformStoreExt;
        let _: fn(
            std::sync::Arc<dyn everruns_capabilities::PlatformStore>,
        ) -> crate::PlatformStoreExt = constructor;
    }
}
