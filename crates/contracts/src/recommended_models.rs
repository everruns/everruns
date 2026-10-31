//! Curated per-provider model recommendations.
//!
//! Adopters need an opinionated starting point: a short recommended list per provider
//! plus a default for each role (coding, research, chat, small, decision). The live
//! model catalog (`model_profile_data`) is descriptive — enumeration order is
//! explicitly not a recommendation ranking, and several drivers discover models live —
//! so this module provides the generic contract for that opinionated layer.
//!
//! Ownership is per provider: each driver file declares its own
//! `*_RECOMMENDED: ProviderRecommendedModels` const next to its `descriptor()` and
//! publishes it via [`crate::DriverDescriptor::recommended_models`]. There is no
//! central table. To refresh a provider's picks (e.g. a vendor rotates flagships),
//! edit that driver's const only.
//!
//! [`crate::DriverRegistry::recommended_models`] resolves the entry for a
//! [`crate::DriverId`]; drivers without a curated entry (external/custom gateways)
//! report `None` and callers should fall back to live discovery
//! (`rank_discovered_models`).
//!
//! Maintenance cadence: review quarterly, or whenever a provider launches a new
//! flagship family. The drivers-crate `recommended_models` integration test keeps
//! every builtin covered and every role default inside its recommended list.

/// Opinionated defaults for one provider.
///
/// All model IDs are canonical catalog IDs (see `model_profile_data::profiles`). Gateway
/// and live-discovery drivers (openrouter, vercel, bedrock, fireworks, cloudflare) map
/// these canonical IDs to their own wire IDs; drivers that host first-party models use
/// them directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRecommendedModels {
    /// Driver ID this entry covers (e.g. `"openai"`, `"anthropic"`).
    pub provider: &'static str,
    /// Short recommended list, strongest first. Adopters can present this as-is.
    pub recommended_models: &'static [&'static str],
    /// Default for code generation / agentic coding.
    pub coding_model: &'static str,
    /// Default for deep research / long-reasoning work.
    pub research_model: &'static str,
    /// Default for everyday chat.
    pub chat_model: &'static str,
    /// Default for cheap/fast small work.
    pub small_model: &'static str,
    /// Default for fast gate/decision checks.
    pub decision_model: &'static str,
}
