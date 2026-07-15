// Claude (Anthropic) Tool Search Capability
//
// When added to an agent, enables Anthropic's hosted tool_search (deferred tool
// loading) for Claude models with tool_search=true in their profile. Each tool's
// full parameter schema is loaded on-demand by the model via a server-side
// `tool_search_tool_*_20251119` tool instead of being sent upfront.
//
// Like `openai_tool_search`, this capability provides no tools itself — it sets a
// provider-agnostic `ToolSearchConfig`. The Anthropic driver consumes that config
// and renders the hosted format (`defer_loading: true` per tool plus a
// `tool_search_tool_bm25_20251119` entry). See `crates/anthropic/src/driver.rs`.
//
// If the model does not support tool_search (tool_search=false in the Anthropic
// profile), this capability is silently ignored — no error, no crash. Use
// `auto_tool_search` for a model-adaptive default that picks this on native
// Claude models and the generic client-side mechanism elsewhere.

use super::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use crate::driver_registry::ToolSearchConfig;
use async_trait::async_trait;

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for Claude (Anthropic) tool search.
pub const CLAUDE_TOOL_SEARCH_CAPABILITY_ID: &str = "claude_tool_search";

/// Claude Tool Search capability.
///
/// Adding this capability to an agent/harness enables deferred tool loading for
/// Claude models that support it. The `threshold` controls the minimum number of
/// tools before tool_search activates (default: [`DEFAULT_TOOL_SEARCH_THRESHOLD`]).
pub struct ClaudeToolSearchCapability {
    threshold: usize,
}

impl ClaudeToolSearchCapability {
    pub fn new() -> Self {
        Self {
            threshold: DEFAULT_TOOL_SEARCH_THRESHOLD,
        }
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self { threshold }
    }

    /// Returns the `ToolSearchConfig` for this capability.
    ///
    /// The config is provider-agnostic — the same shape `openai_tool_search`
    /// produces. The Anthropic driver renders it into the hosted Messages-API
    /// format; the OpenAI Responses driver renders it into the Responses format.
    /// Whichever driver handles the request decides the wire shape, so this
    /// carries no provider tag.
    pub fn tool_search_config(&self) -> ToolSearchConfig {
        ToolSearchConfig {
            enabled: true,
            threshold: self.threshold,
        }
    }
}

/// Whether `model` natively supports Anthropic's hosted tool_search.
///
/// This is the single source of truth for the native-Claude decision, consulted
/// by `auto_tool_search`'s runtime dispatch (at capability-collection time) and
/// by `RuntimeAgentBuilder::build` (when reconciling a hosted config with the
/// model). Native Claude tool_search is an Anthropic Messages-API feature, so the
/// lookup is against the Anthropic provider profile regardless of how the model
/// is otherwise routed (a Claude model reached via OpenRouter/Bedrock has the
/// flag masked off in `get_model_profile` and falls back to client-side search).
pub fn model_supports_native_tool_search(model: &str) -> bool {
    crate::model_profiles::get_model_profile(&crate::provider::DriverId::Anthropic, model)
        .is_some_and(|profile| profile.tool_search)
}

impl Default for ClaudeToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Capability for ClaudeToolSearchCapability {
    fn id(&self) -> &str {
        CLAUDE_TOOL_SEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Claude Tool Search"
    }

    fn description(&self) -> &str {
        "Enables deferred tool loading for Claude models that support it \
         (Sonnet 4, Opus 4, Haiku 4.5, and Fable 5 and newer). Reduces token \
         usage by loading tool schemas on-demand instead of upfront."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Пошук інструментів Claude",
            "Вмикає відкладене завантаження інструментів для моделей Claude, які його підтримують (Sonnet 4, Opus 4, Haiku 4.5 та Fable 5 і новіші). Зменшує використання токенів, завантажуючи схеми інструментів на вимогу, а не заздалегідь.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None // No system prompt needed — deferral is handled server-side.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_default_threshold() {
        let cap = ClaudeToolSearchCapability::new();
        let config = cap.tool_search_config();
        assert!(config.enabled);
        assert_eq!(config.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
    }

    #[test]
    fn test_custom_threshold() {
        let cap = ClaudeToolSearchCapability::with_threshold(5);
        assert_eq!(cap.tool_search_config().threshold, 5);
    }

    #[test]
    fn test_native_support_lookup() {
        // Claude 4-family models support hosted tool_search; 3.x do not.
        assert!(model_supports_native_tool_search("claude-opus-4-8"));
        assert!(model_supports_native_tool_search("claude-sonnet-5"));
        assert!(model_supports_native_tool_search("claude-sonnet-4-6"));
        assert!(model_supports_native_tool_search("claude-haiku-4-5"));
        assert!(model_supports_native_tool_search("claude-fable-5"));
        assert!(!model_supports_native_tool_search("claude-3-5-haiku"));
        // Unknown / non-Anthropic models are not native.
        assert!(!model_supports_native_tool_search("gpt-5.5"));
    }
}
