//! **Moved.** `everruns-model-profiles` is now part of
//! [`everruns-contracts`](https://docs.rs/everruns-contracts), in the
//! [Everruns](https://everruns.com) ecosystem. This deprecated shim receives
//! no further updates and leaves the workspace after one migration release.
//!
//! Replace `everruns_model_profiles::` with `everruns_contracts::model_profile_data::`.
//!
//! ```rust
//! use everruns_contracts::model_profile_data::get_model_profile;
//! let profile = get_model_profile("anthropic", "claude-sonnet-5").expect("known model");
//! assert_eq!(profile.family, "claude-sonnet-5");
//! ```

pub use everruns_contracts::model_profile_data::*;

/// Moved to [`everruns_contracts::model_profile_data::ModelProfile`].
#[deprecated(
    note = "moved to everruns_contracts::model_profile_data::ModelProfile; everruns-model-profiles is no longer updated"
)]
pub type ModelProfile = everruns_contracts::model_profile_data::ModelProfile;

#[cfg(test)]
mod tests {
    #[test]
    #[allow(deprecated)]
    fn legacy_profile_registry_preserves_wire_id_lookup() {
        let profile: Option<crate::ModelProfile> =
            crate::get_model_profile("openai", "gpt-6-astra");
        assert_eq!(
            profile,
            everruns_contracts::model_profile_data::get_model_profile("openai", "gpt-6-astra")
        );
    }
}
