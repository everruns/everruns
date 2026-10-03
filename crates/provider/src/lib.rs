//! **Moved.** `everruns-provider` is now part of
//! [`everruns-contracts`](https://docs.rs/everruns-contracts), in the
//! [Everruns](https://everruns.com) ecosystem. This deprecated shim receives
//! no further updates and leaves the workspace after one migration release.
//!
//! Replace `everruns_provider::` with `everruns_contracts::`.
//!
//! ```rust
//! use everruns_contracts::ModelSpec;
//! let model = ModelSpec::on("company-gateway", "assistant-v2");
//! assert_eq!(model.provider.as_str(), "company-gateway");
//! ```

pub use everruns_contracts::*;

/// Moved to [`everruns_contracts::ModelSpec`].
#[deprecated(
    note = "moved to everruns_contracts::ModelSpec; everruns-provider is no longer updated"
)]
pub type ModelSpec = everruns_contracts::ModelSpec;

#[cfg(test)]
mod tests {
    #[test]
    #[allow(deprecated)]
    fn legacy_model_spec_has_the_canonical_contract_type() {
        let model: crate::ModelSpec = everruns_contracts::ModelSpec::on("gateway", "assistant");
        let model: everruns_contracts::ModelSpec = model;
        assert_eq!(model.provider.as_str(), "gateway");
    }
}
