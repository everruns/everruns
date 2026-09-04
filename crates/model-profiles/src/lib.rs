//! Model profile data and types shared by the
//! [Everruns](https://everruns.com) provider crates.
//!
//! This crate owns model identity/capability metadata (`ModelProfile` and its
//! cost/limits/modality/reasoning/speed/verbosity components), model vendor
//! branding (`ModelVendor`), the model-service taxonomy (`ServiceKind`), and
//! the hardcoded profile registry sourced from
//! [models.dev](https://github.com/sst/models.dev).
//!
//! It is deliberately dependency-light and does not depend on
//! `everruns-provider`: driver/provider identity is passed as a plain wire-id
//! string (e.g. `"openai"`, `"anthropic"`) rather than `everruns_provider::DriverId`,
//! so `everruns-provider` can depend on this crate (and re-export its types
//! from `model.rs`/`driver_registry.rs`/`model_profiles.rs` for source
//! compatibility) without a cycle.
//!
//! # Example
//!
//! ```
//! use everruns_model_profiles::get_model_profile;
//!
//! let profile = get_model_profile("anthropic", "claude-sonnet-5").expect("known model");
//! assert_eq!(profile.family, "claude-sonnet-5");
//! ```

pub mod profiles;
mod types;

pub use profiles::{
    estimate_cost_usd, get_model_profile, get_model_profile_by_key, get_model_profile_key,
    get_model_service_kind, get_model_vendor,
};
pub use types::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue, ServiceKind, Speed, SpeedConfig,
    SpeedValue, Verbosity, VerbosityConfig, VerbosityValue,
};
