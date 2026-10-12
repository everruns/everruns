// Prompt Caching Capability
//
// When added to an agent, enables provider-specific prompt caching behavior for
// drivers that support it. This capability does not add tools or prompt text;
// it only configures the outbound LLM request.

use super::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use crate::builtins::driver_registry::{
    PromptCacheConfig, PromptCacheRetention, PromptCacheStrategy,
};
use async_trait::async_trait;

/// Capability ID for provider prompt caching.
pub const PROMPT_CACHING_CAPABILITY_ID: &str = "prompt_caching";

/// Prompt caching capability.
///
/// Drivers translate this generic config into provider-specific request
/// controls when possible. Unsupported providers or models ignore it.
pub struct PromptCachingCapability {
    strategy: PromptCacheStrategy,
    retention: PromptCacheRetention,
    gemini_cached_content: Option<String>,
}

impl PromptCachingCapability {
    pub fn new() -> Self {
        Self {
            strategy: PromptCacheStrategy::Auto,
            retention: PromptCacheRetention::Default,
            gemini_cached_content: None,
        }
    }

    pub fn with_strategy(strategy: PromptCacheStrategy) -> Self {
        Self {
            strategy,
            retention: PromptCacheRetention::Default,
            gemini_cached_content: None,
        }
    }

    /// Request a cache retention; drivers snap it to what the provider supports.
    pub fn with_retention(mut self, retention: PromptCacheRetention) -> Self {
        self.retention = retention;
        self
    }

    pub fn with_gemini_cached_content(
        strategy: PromptCacheStrategy,
        gemini_cached_content: impl Into<String>,
    ) -> Self {
        Self {
            strategy,
            retention: PromptCacheRetention::Default,
            gemini_cached_content: Some(gemini_cached_content.into()),
        }
    }

    /// Returns the PromptCacheConfig for this capability.
    pub fn prompt_cache_config(&self) -> PromptCacheConfig {
        PromptCacheConfig {
            retention: self.retention,
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

    fn prompt_cache_config(&self, config: &serde_json::Value) -> Option<PromptCacheConfig> {
        let gemini_cached_content = config
            .get("gemini_cached_content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| self.gemini_cached_content.clone());
        fn parse<T: serde::de::DeserializeOwned>(
            config: &serde_json::Value,
            key: &str,
        ) -> Option<T> {
            config
                .get(key)
                .and_then(|value| serde_json::from_value(value.clone()).ok())
        }
        Some(PromptCacheConfig {
            retention: parse(config, "retention").unwrap_or(self.retention),
            enabled: true,
            strategy: parse(config, "strategy").unwrap_or(self.strategy),
            gemini_cached_content,
        })
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_cache_configuration_overrides_constructor_and_preserves_fallback() {
        for fallback in [None, Some("cachedContents/constructor")] {
            let cap = match fallback {
                Some(value) => PromptCachingCapability::with_gemini_cached_content(
                    PromptCacheStrategy::Auto,
                    value,
                ),
                None => PromptCachingCapability::new(),
            };
            for (config, expected) in [
                (json!(null), fallback),
                (json!({}), fallback),
                (
                    json!({"gemini_cached_content":"cachedContents/runtime"}),
                    Some("cachedContents/runtime"),
                ),
                (json!({"gemini_cached_content":false}), fallback),
                (json!({"gemini_cached_content":null}), fallback),
            ] {
                assert_eq!(
                    Capability::prompt_cache_config(&cap, &config),
                    Some(PromptCacheConfig {
                        retention: Default::default(),
                        enabled: true,
                        strategy: PromptCacheStrategy::Auto,
                        gemini_cached_content: expected.map(str::to_owned)
                    }),
                    "config={config}, fallback={fallback:?}"
                );
            }
        }
    }

    #[test]
    fn explicit_strategy_can_be_selected_in_capability_config() {
        let capability = PromptCachingCapability::new();
        let config = Capability::prompt_cache_config(
            &capability,
            &serde_json::json!({"strategy":"explicit"}),
        )
        .unwrap();
        assert_eq!(config.strategy, PromptCacheStrategy::Explicit);
    }

    #[test]
    fn retention_is_read_from_config_and_unknown_values_keep_the_constructor_value() {
        let capability =
            PromptCachingCapability::new().with_retention(PromptCacheRetention::FiveMinutes);
        let retention = |config: serde_json::Value| {
            Capability::prompt_cache_config(&capability, &config)
                .unwrap()
                .retention
        };
        assert_eq!(
            retention(json!({"retention":"1h"})),
            PromptCacheRetention::OneHour
        );
        assert_eq!(
            retention(json!({"retention":"30m"})),
            PromptCacheRetention::ThirtyMinutes
        );
        assert_eq!(
            retention(json!({"retention":"2h"})),
            PromptCacheRetention::FiveMinutes
        );
        assert_eq!(retention(json!({})), PromptCacheRetention::FiveMinutes);
    }
}
