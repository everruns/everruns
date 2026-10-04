//! Model identity, capability metadata, and the offline profile registry.
//!
//! Provider identity is a wire-id string such as `"openai"` or `"anthropic"`.
//! The [`crate::model_profiles`] adapters accept typed [`crate::DriverId`] values.
//!
//! ```
//! use everruns_contracts::model_profile_data::get_model_profile;
//! let profile = get_model_profile("anthropic", "claude-sonnet-5").expect("known model");
//! assert_eq!(profile.family, "claude-sonnet-5");
//! ```

pub mod profiles;
mod types;

pub use profiles::{
    ModelProfileEntry, all_profile_entries, all_profiles, estimate_cost_usd,
    estimate_cost_usd_for_speed, get_model_profile, get_model_profile_by_key,
    get_model_profile_key, get_model_service_kind, get_model_vendor, profile_entries_for_provider,
    profiles_for_provider, selected_profiles, selected_profiles_for_provider,
};
pub use types::{
    CLEAR_AT_PARAMETER, CostTier, DecisionModelProfile, MID_CONVERSATION_SYSTEM_PARAMETER,
    Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor, ReasoningEffort,
    ReasoningEffortConfig, ReasoningEffortValue, ServiceKind, Speed, SpeedConfig, SpeedValue,
    Verbosity, VerbosityConfig, VerbosityValue,
};
