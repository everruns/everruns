//! Fluent provider/model info facade over the driver registry and model catalog.
//!
//! The registry answers three questions through one handle:
//!
//! ```rust
//! use everruns_contracts::{DriverId, DriverRegistry};
//! use everruns_contracts::provider_info::ProviderInfo;
//!
//! let registry = DriverRegistry::default();
//! let openai = ProviderInfo::for_provider(&registry, &DriverId::OpenAI);
//! let coding = openai.coding_model(); // Option<ModelInfo>
//! ```
//!
//! [`ProviderInfo`] reads curated picks from the owning driver's
//! `DriverDescriptor::recommended_models` (use
//! [`DriverRegistry::recommended_models`] for the raw struct). [`ModelInfo`]
//! resolves the catalog card via `get_model_profile` and reports capability
//! flags such as [`ModelInfo::supports_reasoning`]. Unknown drivers and models
//! yield `None` / `false` so callers can fall back to live discovery.

use crate::{
    DriverId, DriverRegistry,
    model_profile_data::{ModelProfile, get_model_profile},
    recommended_models::ProviderRecommendedModels,
};

/// Borrowed view of one provider registered in a [`DriverRegistry`].
///
/// Build with [`ProviderInfo::for_provider`]; role accessors return `None` when
/// the driver declares no curated entry (external/custom gateways).
///
/// (`Clone`/`Copy` only: [`DriverRegistry`] itself is not `Debug`.)
#[derive(Clone, Copy)]
pub struct ProviderInfo<'a> {
    registry: &'a DriverRegistry,
    provider: &'a DriverId,
}

impl<'a> ProviderInfo<'a> {
    /// Borrow a provider's info from a registry. No key or network access needed.
    pub fn for_provider(registry: &'a DriverRegistry, provider: &'a DriverId) -> Self {
        Self { registry, provider }
    }

    /// The provider this info belongs to.
    pub fn provider(&self) -> &'a DriverId {
        self.provider
    }

    /// Raw curated picks, if the driver declares any.
    pub fn recommended(&self) -> Option<ProviderRecommendedModels> {
        self.registry.recommended_models(self.provider)
    }

    fn role_model(
        &self,
        pick: impl FnOnce(ProviderRecommendedModels) -> &'static str,
    ) -> Option<ModelInfo<'a>> {
        let recommended = self.recommended()?;
        Some(ModelInfo {
            provider: self.provider,
            model: pick(recommended),
        })
    }

    /// Default model for code generation / agentic coding.
    pub fn coding_model(&self) -> Option<ModelInfo<'a>> {
        self.role_model(|r| r.coding_model)
    }

    /// Default model for deep research / long-reasoning work.
    pub fn research_model(&self) -> Option<ModelInfo<'a>> {
        self.role_model(|r| r.research_model)
    }

    /// Default model for everyday chat.
    pub fn chat_model(&self) -> Option<ModelInfo<'a>> {
        self.role_model(|r| r.chat_model)
    }

    /// Default model for cheap/fast small work.
    pub fn small_model(&self) -> Option<ModelInfo<'a>> {
        self.role_model(|r| r.small_model)
    }

    /// Default model for fast gate/decision checks.
    pub fn decision_model(&self) -> Option<ModelInfo<'a>> {
        self.role_model(|r| r.decision_model)
    }

    /// A handle for an arbitrary model id on this provider (e.g. a discovered
    /// model id). [`ModelInfo::card`] resolves it against the catalog when known.
    pub fn model(&self, model: &'static str) -> ModelInfo<'a> {
        ModelInfo {
            provider: self.provider,
            model,
        }
    }
}

/// A provider-scoped model handle: a canonical id plus catalog lookups.
///
/// Build via [`ProviderInfo::coding_model`] (and the other role accessors),
/// [`ProviderInfo::model`], or [`ModelInfo::new`] directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo<'a> {
    provider: &'a DriverId,
    model: &'static str,
}

impl<'a> ModelInfo<'a> {
    /// Build a handle directly (same as [`ProviderInfo::model`]).
    pub fn new(provider: &'a DriverId, model: &'static str) -> Self {
        Self { provider, model }
    }

    /// The canonical catalog id, e.g. `"gpt-6.1-sol"`.
    pub fn id(&self) -> &'static str {
        self.model
    }

    /// The provider this model belongs to.
    pub fn provider(&self) -> &'a DriverId {
        self.provider
    }

    /// The catalog card for this model, if catalogued.
    pub fn card(&self) -> Option<ModelProfile> {
        get_model_profile(self.provider.as_str(), self.model)
    }

    /// Whether the catalog marks this model as reasoning-capable.
    /// Unknown models report `false` so callers can fall back to live discovery.
    pub fn supports_reasoning(&self) -> bool {
        self.card()
            .map(|profile| profile.reasoning)
            .unwrap_or(false)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::driver_registry::DriverDescriptor;

    const TEST_RECOMMENDED: ProviderRecommendedModels = ProviderRecommendedModels {
        provider: "anthropic",
        recommended_models: &["claude-opus-5-5", "claude-haiku-5-5"],
        coding_model: "claude-opus-5-5",
        research_model: "claude-opus-5-5",
        chat_model: "claude-haiku-5-5",
        small_model: "claude-haiku-5-5",
        decision_model: "claude-haiku-5-5",
    };

    fn registry_with_recommendations() -> DriverRegistry {
        let mut registry = DriverRegistry::default();
        registry.register_descriptor(DriverDescriptor {
            recommended_models: Some(TEST_RECOMMENDED),
            ..DriverDescriptor::chat_only(DriverId::Anthropic, |_| {
                panic!("unit tests never build a driver")
            })
        });
        registry
    }

    #[test]
    fn coding_model_resolves_and_card_matches_profile() {
        let registry = registry_with_recommendations();
        let info = ProviderInfo::for_provider(&registry, &DriverId::Anthropic);
        let coding = info.coding_model().expect("coding model");
        assert_eq!(coding.id(), "claude-opus-5-5");
        assert_eq!(coding.provider(), &DriverId::Anthropic);
        let card = coding.card().expect("catalogued");
        assert_eq!(coding.supports_reasoning(), card.reasoning);
    }

    #[test]
    fn all_role_models_come_from_recommended_list() {
        let registry = registry_with_recommendations();
        let info = ProviderInfo::for_provider(&registry, &DriverId::Anthropic);
        for role in [
            info.coding_model(),
            info.research_model(),
            info.chat_model(),
            info.small_model(),
            info.decision_model(),
        ] {
            let model = role.expect("role model");
            assert!(TEST_RECOMMENDED.recommended_models.contains(&model.id()));
        }
    }

    #[test]
    fn unknown_provider_and_model_fall_back_cleanly() {
        let registry = DriverRegistry::default();
        let info = ProviderInfo::for_provider(&registry, &DriverId::OpenAI);
        assert!(info.recommended().is_none());
        assert!(info.coding_model().is_none());
        let unknown = ModelInfo::new(&DriverId::OpenAI, "no-such-model");
        assert!(unknown.card().is_none());
        assert!(!unknown.supports_reasoning());
    }
}
