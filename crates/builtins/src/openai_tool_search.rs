// OpenAI Tool Search Capability
//
// When added to an agent, enables tool_search (deferred tool loading) for
// models with tool_search=true in their profile. Tools are grouped into
// namespaces based on capability categories, and their full parameter schemas
// are loaded on-demand by the model instead of sent upfront.
//
// This capability does not provide any tools itself — it configures the
// LLM driver to use tool_search when constructing the API request.
//
// If the model does not support tool_search (tool_search=false in profile),
// this capability is silently ignored — no error, no crash.

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use everruns_core::driver_registry::ToolSearchConfig;
use async_trait::async_trait;

/// Default minimum tool count to activate tool_search.
/// Below this threshold, full schemas are sent even when capability is enabled.
pub const DEFAULT_TOOL_SEARCH_THRESHOLD: usize = 15;

/// Capability ID for OpenAI tool search
pub const OPENAI_TOOL_SEARCH_CAPABILITY_ID: &str = "openai_tool_search";

/// OpenAI Tool Search capability.
///
/// Adding this capability to an agent/harness enables deferred tool loading
/// for models that support it. The `threshold` controls the minimum number
/// of tools before tool_search activates (default: 15).
pub struct OpenAiToolSearchCapability {
    threshold: usize,
}

impl OpenAiToolSearchCapability {
    pub fn new() -> Self {
        Self {
            threshold: DEFAULT_TOOL_SEARCH_THRESHOLD,
        }
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self { threshold }
    }

    /// Returns the ToolSearchConfig for this capability
    pub fn tool_search_config(&self) -> ToolSearchConfig {
        ToolSearchConfig {
            enabled: true,
            threshold: self.threshold,
        }
    }
}

/// Whether `model` natively supports hosted tool_search (OpenAI GPT-5.4+).
///
/// This is the single source of truth for the native-vs-client-side decision,
/// consulted by `auto_tool_search`'s runtime dispatch (at capability-collection
/// time) and by `RuntimeAgentBuilder::build` (when disabling a hosted config the
/// model can't honor). Native tool_search is an OpenAI hosted feature, so the
/// lookup is against the OpenAI provider profile regardless of how the model is
/// otherwise routed.
pub fn model_supports_native_tool_search(model: &str) -> bool {
    everruns_core::model_profiles::get_model_profile(&everruns_core::provider::DriverId::OpenAI, model)
        .is_some_and(|profile| profile.tool_search)
}

impl Default for OpenAiToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Capability for OpenAiToolSearchCapability {
    fn id(&self) -> &str {
        OPENAI_TOOL_SEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenAI Tool Search"
    }

    fn description(&self) -> &str {
        "Enables deferred tool loading for models that support it (GPT-5.4 and newer). \
         Reduces token usage by loading tool schemas on-demand instead of upfront."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Пошук інструментів OpenAI",
            "Вмикає відкладене завантаження інструментів для моделей, які його підтримують (GPT-5.4 і новіші). Зменшує використання токенів, завантажуючи схеми інструментів на вимогу, а не заздалегідь.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None // No system prompt needed
    }
}

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_default_threshold() {
        let cap = OpenAiToolSearchCapability::new();
        let config = cap.tool_search_config();
        assert!(config.enabled);
        assert_eq!(config.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
    }

    #[test]
    fn test_custom_threshold() {
        let cap = OpenAiToolSearchCapability::with_threshold(5);
        let config = cap.tool_search_config();
        assert_eq!(config.threshold, 5);
    }
}
