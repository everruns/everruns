//! **Moved.** `everruns-capability` is now part of
//! [`everruns-contracts`](https://docs.rs/everruns-contracts), in the
//! [Everruns](https://everruns.com) ecosystem. This deprecated shim receives
//! no further updates and leaves the workspace after one migration release.
//!
//! Replace `everruns_capability::` with `everruns_contracts::capability::`.
//!
//! ```rust
//! use everruns_contracts::CapabilityRef;
//! let capability = CapabilityRef::new("vendor.search");
//! assert_eq!(capability.id(), "vendor.search");
//! ```

pub use everruns_contracts::capability::*;

/// Moved to [`everruns_contracts::capability::CapabilityRef`].
#[deprecated(
    note = "moved to everruns_contracts::capability::CapabilityRef; everruns-capability is no longer updated"
)]
pub type CapabilityRef = everruns_contracts::capability::CapabilityRef;

#[cfg(all(test, feature = "definition"))]
mod tests {
    #[derive(crate::serde::Serialize, crate::serde::Deserialize, crate::schemars::JsonSchema)]
    #[serde(crate = "crate::serde")]
    #[schemars(crate = "crate::schemars")]
    struct LegacyArguments {
        query: String,
    }

    #[test]
    fn legacy_derive_paths_forward_to_contracts() {
        let schema = crate::json_schema_for::<LegacyArguments>();
        assert!(schema["properties"]["query"].is_object());
    }
}
