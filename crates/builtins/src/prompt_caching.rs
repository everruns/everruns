// Prompt Caching Capability
//
// When added to an agent, enables provider-specific prompt caching behavior for
// drivers that support it. This capability does not add tools or prompt text;
// it only configures the outbound LLM request.

use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext,
};
use everruns_core::driver_registry::{PromptCacheConfig, PromptCacheStrategy};

/// Capability ID for provider prompt caching.
pub const PROMPT_CACHING_CAPABILITY_ID: &str = "prompt_caching";

/// Prompt caching capability.
///
/// Drivers translate this generic config into provider-specific request
/// controls when possible. Unsupported providers or models ignore it.
pub struct PromptCachingCapability {
    strategy: PromptCacheStrategy,
    gemini_cached_content: Option<String>,
}

impl PromptCachingCapability {
    pub fn new() -> Self {
        Self {
            strategy: PromptCacheStrategy::Auto,
            gemini_cached_content: None,
        }
    }

    pub fn with_strategy(strategy: PromptCacheStrategy) -> Self {
        Self {
            strategy,
            gemini_cached_content: None,
        }
    }

    pub fn with_gemini_cached_content(
        strategy: PromptCacheStrategy,
        gemini_cached_content: impl Into<String>,
    ) -> Self {
        Self {
            strategy,
            gemini_cached_content: Some(gemini_cached_content.into()),
        }
    }

    /// Returns the PromptCacheConfig for this capability.
    pub fn prompt_cache_config(&self) -> PromptCacheConfig {
        PromptCacheConfig {
            enabled: true,
            strategy: self.strategy,
            gemini_cached_content: self.gemini_cached_content.clone(),
        }
    }
}

impl Default for PromptCachingCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Capability for PromptCachingCapability {
    fn id(&self) -> &str {
        PROMPT_CACHING_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Prompt Caching"
    }

    fn description(&self) -> &str {
        "Enables provider-specific prompt caching where supported and records \
         that request intent in llm.generation metadata."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Кешування промптів",
            "Вмикає кешування промптів, специфічне для провайдера, там, де воно підтримується, і фіксує цей намір запиту в метаданих llm.generation.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_default_strategy() {
        let cap = PromptCachingCapability::new();
        let config = cap.prompt_cache_config();
        assert!(config.enabled);
        assert_eq!(config.strategy, PromptCacheStrategy::Auto);
        assert!(config.gemini_cached_content.is_none());
    }

    #[test]
    fn test_capability_with_gemini_cached_content() {
        let cap = PromptCachingCapability::with_gemini_cached_content(
            PromptCacheStrategy::Auto,
            "cachedContents/example",
        );
        let config = cap.prompt_cache_config();
        assert_eq!(
            config.gemini_cached_content.as_deref(),
            Some("cachedContents/example")
        );
    }
}
