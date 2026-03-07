// OpenAI Tool Search Capability
//
// When added to an agent, enables tool_search (deferred tool loading) for
// models that support it (GPT-5.4+). Tools are grouped into namespaces
// based on capability categories, and their full parameter schemas are
// loaded on-demand by the model instead of sent upfront.
//
// This capability does not provide any tools itself — it configures the
// LLM driver to use tool_search when constructing the API request.
//
// If the model does not support tool_search (tool_search=false in profile),
// this capability is silently ignored — no error, no crash.

use super::{Capability, CapabilityStatus, SystemPromptContext};
use crate::llm_driver_registry::ToolSearchConfig;
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
        "Enables deferred tool loading for models that support it (GPT-5.4+). \
         Reduces token usage by loading tool schemas on-demand instead of upfront."
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
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = OpenAiToolSearchCapability::new();
        assert_eq!(cap.id(), OPENAI_TOOL_SEARCH_CAPABILITY_ID);
        assert_eq!(cap.name(), "OpenAI Tool Search");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert!(cap.tools().is_empty());
    }

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
