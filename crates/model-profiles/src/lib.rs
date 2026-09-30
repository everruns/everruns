#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
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
//!
//! Registry enumeration can also build offline menus without confusing model
//! families with request identities:
//!
//! ```
//! use everruns_model_profiles::{profile_entries_for_provider, ServiceKind};
//! let chat_models: Vec<_> = profile_entries_for_provider("openai")
//!     .into_iter()
//!     .filter(|entry| entry.service == ServiceKind::Chat)
//!     .collect();
//! assert!(chat_models.iter().any(|entry| entry.model_id == "gpt-6-astra"));
//! ```

pub mod profiles;
mod types;

pub use profiles::{
    ModelProfileEntry, all_profile_entries, all_profiles, estimate_cost_usd, get_model_profile,
    get_model_profile_by_key, get_model_profile_key, get_model_service_kind, get_model_vendor,
    profile_entries_for_provider, profiles_for_provider, selected_profiles,
    selected_profiles_for_provider,
};
pub use types::{
    CLEAR_AT_PARAMETER, CostTier, MID_CONVERSATION_SYSTEM_PARAMETER, Modality, ModelCost,
    ModelLimits, ModelModalities, ModelProfile, ModelVendor, ReasoningEffort,
    ReasoningEffortConfig, ReasoningEffortValue, ServiceKind, Speed, SpeedConfig, SpeedValue,
    Verbosity, VerbosityConfig, VerbosityValue,
};
